//! Scenario-aware editing-kit launch preparation and startup-file updates.

use super::*;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

const SCENARIO_GROUP_TAG: u32 = u32::from_be_bytes(*b"scnr");
const SUPPORTED_SCENARIO_LAUNCH_GAMES: &[&str] = &[
    "haloce_mcc",
    "halo2_mcc",
    "halo3_mcc",
    "halo3odst_mcc",
    "haloreach_mcc",
    "halo4_mcc",
    "halo2amp_mcc",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ScenarioLaunchContext {
    pub(super) kit_root: PathBuf,
    pub(super) scenario_file: PathBuf,
    pub(super) scenario_path: String,
    pub(super) game: String,
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
        .filter(|game| SUPPORTED_SCENARIO_LAUNCH_GAMES.contains(game))
        .ok_or_else(|| "Scenario launching requires a supported MCC editing kit".to_owned())?;
    let TagSource::LooseFolder { root, .. } = &source.source else {
        return Err("Scenario launching requires a loaded loose editing-kit folder".to_owned());
    };
    if !root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("tags"))
    {
        return Err("Scenario launching requires the editing kit's tags folder".to_owned());
    }
    let kit_root = root
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Could not determine the editing-kit root".to_owned())?;
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
    matches!(
        game,
        "halo2_mcc"
            | "halo3_mcc"
            | "halo3odst_mcc"
            | "haloreach_mcc"
            | "halo4_mcc"
            | "halo2amp_mcc"
    )
}

pub(super) fn scenario_startup_command(game: &str, scenario_path: &str) -> String {
    let command = if game == "haloce_mcc" {
        "map_name"
    } else {
        "game_start"
    };
    let argument = if scenario_path.chars().any(char::is_whitespace) || scenario_path.contains(';')
    {
        format!("\"{scenario_path}\"")
    } else {
        scenario_path.to_owned()
    };
    format!("{command} {argument}")
}

pub(super) fn tag_test_executable_for_game(game: Option<&str>) -> &'static str {
    match game {
        Some("haloce_mcc") => "halo_tag_test.exe",
        Some("halo2_mcc") => "halo2_tag_test.exe",
        Some("halo3_mcc") => "halo3_tag_test.exe",
        Some("halo3odst_mcc") => "atlas_tag_test.exe",
        Some("haloreach_mcc") => "reach_tag_test.exe",
        Some("halo4_mcc") => "halo4_tag_test.exe",
        Some("halo2amp_mcc") => "halo2a_tag_test.exe",
        _ => "tag_test.exe",
    }
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
mod tests {
    use super::*;

    fn loaded_source(root: PathBuf, game: &str) -> LoadedSourceData {
        LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root,
                game: Some(game.to_owned()),
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: Some(game.to_owned()),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
        }
    }

    fn scenario_entry(path: PathBuf) -> TagEntry {
        TagEntry {
            key: path.display().to_string(),
            display_path: "levels/test/my map/my map.scenario".to_owned(),
            group_tag: SCENARIO_GROUP_TAG,
            group_name: Some("scenario".to_owned()),
            location: TagEntryLocation::LooseFile(path),
        }
    }

    #[test]
    fn launch_context_builds_an_extensionless_engine_path() {
        let kit_root = std::env::temp_dir().join("baboon-scenario-context");
        let tags_root = kit_root.join("tags");
        let scenario_file = tags_root
            .join("levels")
            .join("test")
            .join("my map")
            .join("my map.scenario");
        let entry = scenario_entry(scenario_file.clone());
        let context =
            scenario_launch_context(&loaded_source(tags_root, "halo3_mcc"), &entry).unwrap();
        assert_eq!(context.kit_root, kit_root);
        assert_eq!(context.scenario_file, scenario_file);
        assert_eq!(context.scenario_path, "levels\\test\\my map\\my map");
        assert_eq!(context.game, "halo3_mcc");
    }

    #[test]
    fn launch_context_rejects_unsupported_sources_and_escaping_paths() {
        let kit_root = std::env::temp_dir().join("baboon-scenario-context-errors");
        let tags_root = kit_root.join("tags");
        let outside = scenario_entry(kit_root.join("outside.scenario"));
        assert!(
            scenario_launch_context(&loaded_source(tags_root.clone(), "halo3_mcc"), &outside)
                .is_err()
        );

        let valid = scenario_entry(tags_root.join("levels").join("test.scenario"));
        assert!(
            scenario_launch_context(&loaded_source(tags_root, "haloce_evolved"), &valid).is_err()
        );
    }

    /// The same answer decides whether the scenario's Sapien button is drawn at
    /// all, so this covers every game Baboon ships definitions for rather than
    /// only the ones that support it — a game added without a decision here
    /// would silently get no button.
    #[test]
    fn sapien_scenario_arguments_exclude_halo_ce() {
        // Combat Evolved's Sapien takes no scenario argument, and Campaign
        // Evolved has no Sapien. Both mean "no button", not "greyed out".
        assert!(!sapien_supports_scenario_argument("haloce_mcc"));
        assert!(!sapien_supports_scenario_argument("haloce_evolved"));
        for game in [
            "halo2_mcc",
            "halo3_mcc",
            "halo3odst_mcc",
            "haloreach_mcc",
            "halo4_mcc",
            "halo2amp_mcc",
        ] {
            assert!(sapien_supports_scenario_argument(game), "{game}");
        }
    }

    /// Every game with definitions is either offered the button or explicitly
    /// not, so adding a game cannot leave this unanswered by accident.
    #[test]
    fn every_shipped_game_has_a_decision_about_the_sapien_button() {
        let definitions = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let games: Vec<String> = std::fs::read_dir(&definitions)
            .expect("the definitions submodule is required to build")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.starts_with('.'))
            .collect();
        assert!(!games.is_empty(), "definitions/ has per-game folders");
        let without_sapien: Vec<&String> = games
            .iter()
            .filter(|game| !sapien_supports_scenario_argument(game))
            .collect();
        assert_eq!(
            without_sapien.len(),
            2,
            "exactly two shipped games have no scenario-capable Sapien, got {without_sapien:?}"
        );
    }

    #[test]
    fn startup_commands_and_executables_match_supported_games() {
        let cases = [
            (
                "haloce_mcc",
                "map_name levels\\test\\map",
                "halo_tag_test.exe",
            ),
            (
                "halo2_mcc",
                "game_start levels\\test\\map",
                "halo2_tag_test.exe",
            ),
            (
                "halo3_mcc",
                "game_start levels\\test\\map",
                "halo3_tag_test.exe",
            ),
            (
                "halo3odst_mcc",
                "game_start levels\\test\\map",
                "atlas_tag_test.exe",
            ),
            (
                "haloreach_mcc",
                "game_start levels\\test\\map",
                "reach_tag_test.exe",
            ),
            (
                "halo4_mcc",
                "game_start levels\\test\\map",
                "halo4_tag_test.exe",
            ),
            (
                "halo2amp_mcc",
                "game_start levels\\test\\map",
                "halo2a_tag_test.exe",
            ),
        ];
        for (game, command, executable) in cases {
            assert_eq!(scenario_startup_command(game, "levels\\test\\map"), command);
            assert_eq!(tag_test_executable_for_game(Some(game)), executable);
        }
        assert_eq!(
            scenario_startup_command("halo3_mcc", "levels\\my map\\my map"),
            "game_start \"levels\\my map\\my map\""
        );
        assert_eq!(
            scenario_startup_command("halo3_mcc", "levels\\semi;colon"),
            "game_start \"levels\\semi;colon\""
        );
    }

    #[test]
    fn startup_update_preserves_settings_comments_bom_and_crlf() {
        let existing =
            b"\xEF\xBB\xBF; game_start old\\comment\r\ndebug_objects 1\r\ngame_start old\\map ; primary note\r\nmap_name duplicate ; duplicate note\r\n";
        let updated = update_startup_file_bytes(existing, "game_start levels\\new\\map").unwrap();
        assert_eq!(
            updated,
            b"\xEF\xBB\xBF; game_start old\\comment\r\ndebug_objects 1\r\ngame_start levels\\new\\map ; primary note\r\n; duplicate note\r\n"
        );
    }

    #[test]
    fn startup_update_appends_and_preserves_missing_trailing_newline() {
        let updated =
            update_startup_file_bytes(b"debug_objects 1", "game_start levels\\new\\map").unwrap();
        assert_eq!(updated, b"debug_objects 1\ngame_start levels\\new\\map");

        let created = update_startup_file_bytes(b"", "map_name levels\\test\\map").unwrap();
        assert_eq!(created, b"map_name levels\\test\\map\n");
    }

    #[test]
    fn startup_update_rejects_non_utf8_files() {
        assert!(update_startup_file_bytes(&[0xff], "game_start map").is_err());
        assert!(clear_startup_file_bytes(&[0xff]).is_err());
    }

    #[test]
    fn startup_clear_removes_active_launches_and_preserves_other_content() {
        let existing =
            b"\xEF\xBB\xBF; game_start commented\r\ndebug_objects 1\r\ngame_start old\\map ; first note\r\nmap_name duplicate ; second note\r\n";
        let cleared = clear_startup_file_bytes(existing).unwrap();
        assert_eq!(
            cleared,
            b"\xEF\xBB\xBF; game_start commented\r\ndebug_objects 1\r\n; first note\r\n; second note\r\n"
        );
    }

    #[test]
    fn startup_file_is_created_and_atomically_replaced() {
        let root = std::env::temp_dir().join(format!(
            "baboon-scenario-init-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("init.txt");
        update_scenario_startup_file(&path, "game_start levels\\first").unwrap();
        update_scenario_startup_file(&path, "game_start levels\\second").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "game_start levels\\second\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
