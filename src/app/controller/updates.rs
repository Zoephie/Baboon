//! Release lookup and version-comparison helpers for the controller.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::UpdateCheckFinished` to the application status.
    pub(super) fn handle_update_check_finished(
        &mut self,
        result: Result<UpdateCheckResult, String>,
    ) -> bool {
        self.status = match result {
            Ok(result) => update_check_status(&result),
            Err(error) if error == NO_PUBLIC_RELEASE_MESSAGE => error,
            Err(error) => format!("Update check failed: {error}"),
        };
        false
    }
}

pub(super) fn fetch_latest_release() -> Result<UpdateCheckResult, String> {
    #[cfg(target_os = "windows")]
    {
        fetch_latest_release_powershell()
    }
    #[cfg(not(target_os = "windows"))]
    {
        fetch_latest_release_curl()
    }
}

pub(super) const NO_PUBLIC_RELEASE_MESSAGE: &str = "No public Baboon releases found yet";

#[cfg(target_os = "windows")]
fn fetch_latest_release_powershell() -> Result<UpdateCheckResult, String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         $headers = @{{ 'User-Agent' = 'Baboon' }}; \
         try {{ \
             $release = Invoke-RestMethod -UseBasicParsing -Headers $headers -Uri '{}'; \
             [Console]::Out.WriteLine($release.tag_name); \
             [Console]::Out.WriteLine($release.html_url); \
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
         }}",
        BABOON_LATEST_RELEASE_API
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
    parse_latest_release_lines(&output.stdout, &output.stderr, output.status.success())
}

#[cfg(not(target_os = "windows"))]
fn fetch_latest_release_curl() -> Result<UpdateCheckResult, String> {
    let output = Command::new("curl")
        .args([
            "-sSL",
            "-w",
            "\n%{http_code}",
            "-H",
            "User-Agent: Baboon",
            BABOON_LATEST_RELEASE_API,
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
    Ok(UpdateCheckResult {
        latest_tag,
        release_url,
    })
}

#[cfg(target_os = "windows")]
fn parse_latest_release_lines(
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
    Ok(UpdateCheckResult {
        latest_tag,
        release_url,
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

pub(super) fn update_check_status(result: &UpdateCheckResult) -> String {
    let current = env!("CARGO_PKG_VERSION");
    if is_newer_release(&result.latest_tag, current) {
        format!(
            "Update available: {} (current {}). {}",
            result.latest_tag, current, result.release_url
        )
    } else {
        format!("Baboon is up to date ({current})")
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
