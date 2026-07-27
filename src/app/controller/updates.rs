//! Release lookup and version-comparison helpers for the controller.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::UpdateCheckFinished` to the application status.
    ///
    /// A found update reports itself through the status bar's own notification,
    /// so the status line never repeats it — but a check the user asked for
    /// still has to clear the "Checking for updates..." it put there. Only news
    /// that has nowhere else to go, like "up to date" or a failure, is written
    /// here, and a `silent` startup check does not write even that.
    pub(super) fn handle_update_check_finished(
        &mut self,
        silent: bool,
        result: Result<UpdateCheckResult, String>,
    ) -> bool {
        match result {
            Ok(result) => {
                let outdated = is_update_available(&result);
                if !silent {
                    self.status = if outdated {
                        String::new()
                    } else {
                        update_check_status(&result)
                    };
                }
                self.available_update = outdated.then(|| result.clone());
                self.last_update_check = Some(result);
            }
            Err(error) => {
                self.available_update = None;
                self.last_update_check = None;
                if !silent {
                    self.status = update_check_error_status(self.update_channel, &error);
                }
            }
        }
        false
    }

    /// One sentence on where this build stands: the last check's verdict when
    /// there has been one, otherwise a description of what is running.
    pub(in crate::app) fn update_check_summary(&self) -> String {
        match self.last_update_check.as_ref() {
            Some(result) => update_check_status(result),
            None => format!("This build: {}", running_build_description()),
        }
    }
}

/// Names the running build the way the active channel identifies builds — by
/// version, and by commit too once one is baked in.
fn running_build_description() -> String {
    let version = env!("CARGO_PKG_VERSION");
    if BABOON_BUILD_COMMIT.is_empty() {
        format!("Baboon {version}")
    } else {
        format!("Baboon {version} ({})", short_commit(BABOON_BUILD_COMMIT))
    }
}

pub(super) fn fetch_latest_release(channel: UpdateChannel) -> Result<UpdateCheckResult, String> {
    let api_url = match channel {
        UpdateChannel::Stable => BABOON_STABLE_RELEASE_API,
        UpdateChannel::Development => BABOON_DEV_RELEASE_API,
    };
    #[cfg(target_os = "windows")]
    {
        fetch_latest_release_powershell(channel, api_url)
    }
    #[cfg(not(target_os = "windows"))]
    {
        fetch_latest_release_curl(channel, api_url)
    }
}

/// Sentinel returned by the fetchers when the channel's release does not exist
/// (HTTP 404). The handler turns it into channel-specific wording.
pub(super) const NO_PUBLIC_RELEASE_MESSAGE: &str = "__baboon_no_release__";

#[cfg(target_os = "windows")]
fn fetch_latest_release_powershell(
    channel: UpdateChannel,
    api_url: &str,
) -> Result<UpdateCheckResult, String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         $headers = @{{ 'User-Agent' = 'Baboon' }}; \
         try {{ \
             $release = Invoke-RestMethod -UseBasicParsing -Headers $headers -Uri '{api_url}'; \
             [Console]::Out.WriteLine($release.tag_name); \
             [Console]::Out.WriteLine($release.html_url); \
             [Console]::Out.WriteLine($release.target_commitish); \
         }} catch {{ \
             $statusCode = $null; \
             if ($_.Exception.Response -ne $null) {{ \
                 $statusCode = [int]$_.Exception.Response.StatusCode; \
             }} \
             if ($statusCode -eq 404) {{ \
                 [Console]::Out.WriteLine('__BABOON_NO_PUBLIC_RELEASE__'); \
                 exit 0; \
             }} \
             [Console]::Error.WriteLine($_.Exception.Message); \
             exit 1; \
         }}"
    );
    let output = Command::new("powershell.exe")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .output()
        .map_err(|error| format!("Could not run PowerShell: {error}"))?;
    parse_latest_release_lines(
        channel,
        &output.stdout,
        &output.stderr,
        output.status.success(),
    )
}

#[cfg(not(target_os = "windows"))]
fn fetch_latest_release_curl(
    channel: UpdateChannel,
    api_url: &str,
) -> Result<UpdateCheckResult, String> {
    let output = Command::new("curl")
        .args([
            "-sSL",
            "-w",
            "\n%{http_code}",
            "-H",
            "User-Agent: Baboon",
            api_url,
        ])
        .output()
        .map_err(|error| format!("Could not run curl: {error}"))?;
    if !output.status.success() {
        return Err(command_error(&output.stderr));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some((body, status_code)) = text.rsplit_once('\n') else {
        return Err("GitHub response did not include an HTTP status".to_owned());
    };
    if status_code.trim() == "404" {
        return Err(NO_PUBLIC_RELEASE_MESSAGE.to_owned());
    }
    if status_code.trim() != "200" {
        return Err(format!("GitHub returned HTTP {}", status_code.trim()));
    }
    let value: Value = serde_json::from_str(body)
        .map_err(|error| format!("GitHub returned invalid JSON: {error}"))?;
    let latest_tag = value
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    if latest_tag.is_empty() {
        return Err("GitHub response did not include a release tag".to_owned());
    }
    let release_url = value
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty())
        .unwrap_or(BABOON_RELEASES_URL)
        .to_owned();
    let commit = value
        .get("target_commitish")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    Ok(UpdateCheckResult {
        channel,
        latest_tag,
        release_url,
        commit,
    })
}

#[cfg(target_os = "windows")]
fn parse_latest_release_lines(
    channel: UpdateChannel,
    stdout: &[u8],
    stderr: &[u8],
    success: bool,
) -> Result<UpdateCheckResult, String> {
    if !success {
        return Err(command_error(stderr));
    }
    let text = String::from_utf8_lossy(stdout);
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let latest_tag = lines.next().unwrap_or_default().to_owned();
    if latest_tag == "__BABOON_NO_PUBLIC_RELEASE__" {
        return Err(NO_PUBLIC_RELEASE_MESSAGE.to_owned());
    }
    if latest_tag.is_empty() {
        return Err("GitHub response did not include a release tag".to_owned());
    }
    let release_url = lines
        .next()
        .filter(|url| !url.is_empty())
        .unwrap_or(BABOON_RELEASES_URL)
        .to_owned();
    let commit = lines.next().unwrap_or_default().to_owned();
    Ok(UpdateCheckResult {
        channel,
        latest_tag,
        release_url,
        commit,
    })
}

fn command_error(stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        "command exited without an error message".to_owned()
    } else {
        message
    }
}

/// Whether `result` describes a build newer than the running one.
///
/// The two channels answer this from different evidence. A stable release
/// carries a version to compare against `CARGO_PKG_VERSION`. A development
/// release is republished under the same `dev` tag every time, so its only
/// identity is its commit — anything other than an exact match with the commit
/// this binary was built from means a different build is on offer, including
/// the case where our own commit is unknown.
pub(super) fn is_update_available(result: &UpdateCheckResult) -> bool {
    match result.channel {
        UpdateChannel::Stable => is_newer_release(&result.latest_tag, env!("CARGO_PKG_VERSION")),
        UpdateChannel::Development => !is_same_commit(BABOON_BUILD_COMMIT, &result.commit),
    }
}

/// Whether two commit strings name the same build.
///
/// The `-dirty` marker is ignored: a working-tree build sitting on the same
/// commit as the published one has nothing to download. Comparing it literally
/// would instead announce an update on every single launch of every local
/// build. An empty value is unknown, and unknown never counts as a match.
fn is_same_commit(left: &str, right: &str) -> bool {
    let left = base_commit(left);
    let right = base_commit(right);
    !left.is_empty() && !right.is_empty() && left.eq_ignore_ascii_case(right)
}

/// A commit without its working-tree marker.
fn base_commit(commit: &str) -> &str {
    commit.strip_suffix("-dirty").unwrap_or(commit)
}

/// Describes what a check found, in prose.
///
/// No release URL: wherever an available update is shown, it is shown as a
/// link, and a bare URL in a sentence would only duplicate it.
pub(super) fn update_check_status(result: &UpdateCheckResult) -> String {
    let current = env!("CARGO_PKG_VERSION");
    match result.channel {
        UpdateChannel::Stable => {
            if is_update_available(result) {
                format!(
                    "Update available: {} (current {current})",
                    result.latest_tag
                )
            } else {
                format!("Baboon is up to date ({current})")
            }
        }
        UpdateChannel::Development => {
            let offered = short_commit(&result.commit);
            if !is_update_available(result) {
                return format!("Running the latest development build ({offered})");
            }
            let running = if BABOON_BUILD_COMMIT.is_empty() {
                "this build's commit is unknown".to_owned()
            } else {
                format!("built from {}", short_commit(BABOON_BUILD_COMMIT))
            };
            format!("Development build available: {offered} (current {current}, {running})")
        }
    }
}

/// Turns a fetch failure into a sentence, giving the "no such release" sentinel
/// wording that names the channel it came from.
pub(super) fn update_check_error_status(channel: UpdateChannel, error: &str) -> String {
    if error != NO_PUBLIC_RELEASE_MESSAGE {
        return format!("Update check failed: {error}");
    }
    match channel {
        UpdateChannel::Stable => "No public Baboon releases found yet".to_owned(),
        UpdateChannel::Development => "No development builds have been published yet".to_owned(),
    }
}

fn is_newer_release(latest: &str, current: &str) -> bool {
    let latest = version_numbers(latest);
    let current = version_numbers(current);
    let max_len = latest.len().max(current.len());
    for index in 0..max_len {
        let latest_part = latest.get(index).copied().unwrap_or(0);
        let current_part = current.get(index).copied().unwrap_or(0);
        if latest_part != current_part {
            return latest_part > current_part;
        }
    }
    false
}

fn version_numbers(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

#[cfg(test)]
#[path = "../tests/updates.rs"]
mod tests;
