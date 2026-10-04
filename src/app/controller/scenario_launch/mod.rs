//! Scenario-aware editing-kit launch preparation and startup-file updates.

use super::*;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

const SCENARIO_GROUP_TAG: u32 = u32::from_be_bytes(*b"scnr");

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ScenarioLaunchContext {
    pub(super) kit_root: PathBuf,
    pub(super) scenario_file: PathBuf,
    pub(super) scenario_path: String,
    pub(super) game: String,
    /// `-tags_dir`/`-data_dir` for a Halo CE or Halo 2 kit using folders other
    /// than its root's own; see [`kit_tool_folder_options`].
    pub(super) tool_options: Vec<(&'static str, PathBuf)>,
}

pub(super) fn scenario_launch_context(
    source: &LoadedSourceData,
    entry: &TagEntry,
) -> Result<ScenarioLaunchContext, String> {
    if entry.group_tag != SCENARIO_GROUP_TAG {
        return Err("Only scenario tags can be launched".to_owned());
    }

    let game = source
        .game
        .as_deref()
        .filter(|game| GameId::from_id(game).is_some_and(GameFacts::launches_scenarios))
        .ok_or_else(|| "Scenario launching requires a supported MCC editing kit".to_owned())?;
    let TagSource::LooseFolder { root, .. } = &source.source else {
        return Err("Scenario launching requires a loaded loose editing-kit folder".to_owned());
    };
    let layout = source
        .kit_layout()
        .ok_or_else(|| "Could not determine the editing-kit root".to_owned())?;
    // Sapien and tag_test open `tags\<scenario>` relative to the folder they
    // run in, so only the root's own tags folder can be launched from, unless
    // the tools can be told where it is.
    if !layout.tags_is_root_tags_folder() && !kit_folders_are_choosable(game) {
        return Err("Scenario launching requires the editing kit's tags folder".to_owned());
    }
    let tool_options = kit_tool_folder_options(&layout, Some(game));
    let kit_root = layout.root;
    let TagEntryLocation::LooseFile(path) = &entry.location else {
        return Err("Scenario launching requires a loose scenario tag".to_owned());
    };
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "The scenario tag is outside the loaded tags folder".to_owned())?;
    if relative.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return Err("The scenario path escapes the loaded tags folder".to_owned());
    }
    if !relative
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("scenario"))
    {
        return Err("The selected scenario does not use the .scenario extension".to_owned());
    }

    let without_extension = relative.with_extension("");
    let scenario_path = without_extension
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .ok_or_else(|| "The scenario path is not valid Unicode".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("\\");
    if scenario_path.is_empty()
        || scenario_path.contains('"')
        || scenario_path.chars().any(char::is_control)
    {
        return Err("The scenario path contains unsupported characters".to_owned());
    }

    Ok(ScenarioLaunchContext {
        kit_root,
        scenario_file: path.clone(),
        scenario_path,
        game: game.to_owned(),
        tool_options,
    })
}

/// Whether this game's Sapien can be handed a scenario to open.
///
/// Two kits are out, for different reasons. Halo Combat Evolved's Sapien is the
/// original tool and takes no scenario on its command line at all — there is no
/// terminal route into it, so opening a scenario "in Sapien" is not a thing
/// that kit can do rather than a thing that happens to be unconfigured. Campaign
/// Evolved ships no Sapien whatsoever. Everywhere else the argument works, and
/// only a missing `sapien.exe` can stop a launch.
///
/// This is also what decides whether the button is *shown*: an editing kit that
/// can never do this should not offer a control for it, greyed out or
/// otherwise.
pub(super) fn sapien_supports_scenario_argument(game: &str) -> bool {
    GameId::from_id(game).is_some_and(GameFacts::sapien_takes_scenario_argument)
}

/// What a kit can launch a scenario in, decided without reference to any one
/// tag.
///
/// The browser's row menus need this. Their drawing functions are free
/// functions with no `&Baboon`, so they cannot run the per-entry validation
/// `scenario_launch_context` does; they gate on the kit-wide half here and let
/// the controller report anything that only the entry can rule out.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::app) struct ScenarioLaunchAvailability {
    /// This kit can launch scenarios at all: a supported MCC game, mounted as a
    /// loose `tags` folder. False for cache and container sources.
    pub(in crate::app) supported: bool,
    /// This kit's Sapien accepts a scenario on its command line — see
    /// [`sapien_supports_scenario_argument`]. Decides whether the item is shown
    /// at all, not whether it is enabled.
    pub(in crate::app) offers_sapien: bool,
    pub(in crate::app) sapien_present: bool,
    pub(in crate::app) tag_test_present: bool,
}

pub(in crate::app) fn scenario_launch_availability(
    source: &LoadedSourceData,
) -> ScenarioLaunchAvailability {
    scenario_launch_availability_with(source, Path::is_file)
}

/// [`scenario_launch_availability`], asking `is_file` whether each tool is
/// there. The browser draws this every frame and passes a cached answer.
pub(in crate::app) fn scenario_launch_availability_with(
    source: &LoadedSourceData,
    is_file: impl Fn(&Path) -> bool,
) -> ScenarioLaunchAvailability {
    let unsupported = ScenarioLaunchAvailability::default();
    let Some(game) = source
        .game
        .as_deref()
        .filter(|game| GameId::from_id(game).is_some_and(GameFacts::launches_scenarios))
    else {
        return unsupported;
    };
    let Some(layout) = source
        .kit_layout()
        .filter(|layout| layout.tags_is_root_tags_folder() || kit_folders_are_choosable(game))
    else {
        return unsupported;
    };
    let kit_root = layout.root.as_path();
    ScenarioLaunchAvailability {
        supported: true,
        offers_sapien: sapien_supports_scenario_argument(game),
        sapien_present: is_file(&kit_root.join("sapien.exe")),
        tag_test_present: is_file(&kit_root.join(tag_test_executable_for_game(Some(game)))),
    }
}

pub(super) fn scenario_startup_command(game: &str, scenario_path: &str) -> String {
    let command = GameId::from_id(game).map_or("game_start", GameFacts::scenario_startup_command);
    let argument = if scenario_path.chars().any(char::is_whitespace) || scenario_path.contains(';')
    {
        format!("\"{scenario_path}\"")
    } else {
        scenario_path.to_owned()
    };
    format!("{command} {argument}")
}

pub(super) fn tag_test_executable_for_game(game: Option<&str>) -> &'static str {
    game.and_then(GameId::from_id)
        .and_then(GameFacts::tag_test_executable)
        .unwrap_or("tag_test.exe")
}

pub(super) fn update_scenario_startup_file(path: &Path, command: &str) -> Result<(), String> {
    let existing = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(format!("Could not read {}: {error}", path.display()));
        }
    };
    let updated = update_startup_file_bytes(&existing, command)?;
    write_bytes_atomic(path, &updated)
        .map_err(|error| format!("Could not update {}: {error}", path.display()))
}

pub(super) fn clear_scenario_startup_commands(path: &Path) -> Result<(), String> {
    let existing = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!("Could not read {}: {error}", path.display()));
        }
    };
    let updated = clear_startup_file_bytes(&existing)?;
    if updated == existing {
        return Ok(());
    }
    write_bytes_atomic(path, &updated)
        .map_err(|error| format!("Could not update {}: {error}", path.display()))
}

fn update_startup_file_bytes(existing: &[u8], command: &str) -> Result<Vec<u8>, String> {
    edit_startup_file_bytes(existing, Some(command))
}

fn clear_startup_file_bytes(existing: &[u8]) -> Result<Vec<u8>, String> {
    edit_startup_file_bytes(existing, None)
}

fn edit_startup_file_bytes(existing: &[u8], command: Option<&str>) -> Result<Vec<u8>, String> {
    const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
    let (bom, text_bytes) = if existing.starts_with(UTF8_BOM) {
        (UTF8_BOM, &existing[UTF8_BOM.len()..])
    } else {
        (&[][..], existing)
    };
    let text = std::str::from_utf8(text_bytes)
        .map_err(|_| "The startup file is not valid UTF-8".to_owned())?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let preserve_trailing_newline = text.is_empty() || text.ends_with('\n') || text.ends_with('\r');

    let mut lines = Vec::new();
    let mut replaced = false;
    for line in text.lines() {
        if is_active_scenario_command(line) {
            let comment = inline_comment(line);
            if !replaced && command.is_some() {
                let command = command.expect("command checked above");
                let replacement = comment
                    .map(|comment| format!("{command} {}", comment.trim_start()))
                    .unwrap_or_else(|| command.to_owned());
                lines.push(replacement);
                replaced = true;
            } else if let Some(comment) = comment {
                lines.push(comment.trim_start().to_owned());
            }
        } else {
            lines.push(line.strip_suffix('\r').unwrap_or(line).to_owned());
        }
    }
    if !replaced && let Some(command) = command {
        lines.push(command.to_owned());
    }

    let mut updated = lines.join(newline);
    if preserve_trailing_newline {
        updated.push_str(newline);
    }
    let mut bytes = Vec::with_capacity(bom.len() + updated.len());
    bytes.extend_from_slice(bom);
    bytes.extend_from_slice(updated.as_bytes());
    Ok(bytes)
}

fn is_active_scenario_command(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with(';') {
        return false;
    }
    trimmed.split_whitespace().next().is_some_and(|command| {
        command.eq_ignore_ascii_case("game_start") || command.eq_ignore_ascii_case("map_name")
    })
}

fn inline_comment(line: &str) -> Option<&str> {
    let mut quoted = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ';' if !quoted => return Some(&line[index..]),
            _ => {}
        }
    }
    None
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "startup file has no parent"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("startup.txt");
    let temp = parent.join(format!(
        ".{file_name}.baboon-{}-{nonce}.tmp",
        std::process::id()
    ));
    fs::write(&temp, bytes)?;
    if let Err(error) = replace_file(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both pointers reference NUL-terminated UTF-16 buffers that remain
    // alive for the duration of the Win32 call.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests;
