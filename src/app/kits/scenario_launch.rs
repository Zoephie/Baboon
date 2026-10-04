//! Scenario-aware editing-kit launch preparation and startup-file updates.

use super::*;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

const SCENARIO_GROUP_TAG: u32 = u32::from_be_bytes(*b"scnr");

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) struct ScenarioLaunchContext {
    pub(in crate::app) kit_root: PathBuf,
    pub(in crate::app) scenario_file: PathBuf,
    pub(in crate::app) scenario_path: String,
    pub(in crate::app) game: GameId,
    /// `-tags_dir`/`-data_dir` for a Halo CE or Halo 2 kit using folders other
    /// than its root's own; see [`kit_tool_folder_options`].
    pub(in crate::app) tool_options: Vec<(&'static str, PathBuf)>,
}

pub(in crate::app) fn scenario_launch_context(
    source: &LoadedSourceData,
    entry: &TagEntry,
) -> Result<ScenarioLaunchContext, String> {
    if entry.group_tag != SCENARIO_GROUP_TAG {
        return Err("Only scenario tags can be launched".to_owned());
    }

    let game = source
        .game
        .filter(|game| game.launches_scenarios())
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
    if !layout.tags_is_root_tags_folder() && !game.tools_take_folder_arguments() {
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

/// What a kit can launch a scenario in, decided without reference to any one
/// tag.
///
/// The browser's row menus need this. Their drawing functions are free
/// functions with no `&Baboon`, so they cannot run the per-entry validation
/// `scenario_launch_context` does; they gate on the kit-wide half here and let
/// the caller report anything that only the entry can rule out.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::app) struct ScenarioLaunchAvailability {
    /// This kit can launch scenarios at all: a supported MCC game, mounted as a
    /// loose `tags` folder. False for cache and container sources.
    pub(in crate::app) supported: bool,
    /// This kit's Sapien accepts a scenario on its command line — see
    /// [`GameFacts::sapien_takes_scenario_argument`]. Decides whether the item is shown
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
    let Some(game) = source.game.filter(|game| game.launches_scenarios()) else {
        return unsupported;
    };
    let Some(layout) = source
        .kit_layout()
        .filter(|layout| layout.tags_is_root_tags_folder() || game.tools_take_folder_arguments())
    else {
        return unsupported;
    };
    let kit_root = layout.root.as_path();
    ScenarioLaunchAvailability {
        supported: true,
        offers_sapien: game.sapien_takes_scenario_argument(),
        sapien_present: is_file(&kit_root.join("sapien.exe")),
        tag_test_present: is_file(&kit_root.join(tag_test_executable_for_game(Some(game)))),
    }
}

pub(in crate::app) fn scenario_startup_command(game: GameId, scenario_path: &str) -> String {
    let command = game.scenario_startup_command();
    let argument = if scenario_path.chars().any(char::is_whitespace) || scenario_path.contains(';')
    {
        format!("\"{scenario_path}\"")
    } else {
        scenario_path.to_owned()
    };
    format!("{command} {argument}")
}

pub(in crate::app) fn tag_test_executable_for_game(game: Option<GameId>) -> &'static str {
    game.and_then(GameFacts::tag_test_executable)
        .unwrap_or("tag_test.exe")
}

pub(in crate::app) fn update_scenario_startup_file(path: &Path, command: &str) -> Result<(), String> {
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

pub(in crate::app) fn clear_scenario_startup_commands(path: &Path) -> Result<(), String> {
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
                game: GameId::from_id(game),
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: GameId::from_id(game),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
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
        assert_eq!(context.game, GameId::Halo3);
    }

    /// The browser's row menu gates on this rather than on the per-entry
    /// context, so what it refuses is worth pinning separately: a kit that
    /// cannot launch at all must offer neither item, and a kit whose Sapien
    /// takes no scenario must offer only tag_test.
    #[test]
    fn availability_refuses_unsupported_kits_and_hides_halo_ce_sapien() {
        let tags_root = std::env::temp_dir()
            .join("baboon-availability")
            .join("tags");

        let unsupported =
            scenario_launch_availability(&loaded_source(tags_root.clone(), "haloce_evolved"));
        assert!(
            !unsupported.supported,
            "Campaign Evolved launches no scenarios"
        );
        assert!(!unsupported.offers_sapien);

        // Combat Evolved keeps tag_test but can never take a scenario in Sapien.
        let halo_ce = scenario_launch_availability(&loaded_source(tags_root.clone(), "haloce_mcc"));
        assert!(halo_ce.supported);
        assert!(
            !halo_ce.offers_sapien,
            "Halo CE's Sapien takes no scenario argument"
        );

        let halo3 = scenario_launch_availability(&loaded_source(tags_root.clone(), "halo3_mcc"));
        assert!(halo3.supported && halo3.offers_sapien);
        // Nothing is on disk under a temp path, so neither executable is found
        // and both items would draw disabled rather than missing.
        assert!(!halo3.sapien_present && !halo3.tag_test_present);

        // A folder that is not the kit's `tags` root cannot resolve a kit root.
        let not_tags = scenario_launch_availability(&loaded_source(
            tags_root.parent().unwrap().to_path_buf(),
            "halo3_mcc",
        ));
        assert!(!not_tags.supported);
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
        assert!(!GameId::HaloCe.sapien_takes_scenario_argument());
        assert!(!GameId::CampaignEvolved.sapien_takes_scenario_argument());
        for game in [
            "halo2_mcc",
            "halo3_mcc",
            "halo3odst_mcc",
            "haloreach_mcc",
            "halo4_mcc",
            "halo2amp_mcc",
        ] {
            let game = GameId::from_id(game).unwrap();
            assert!(game.sapien_takes_scenario_argument(), "{game}");
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
            .filter(|game| {
                let game = GameId::from_id(game).unwrap_or_else(|| panic!("definitions/{game} names no game"));
                !game.sapien_takes_scenario_argument()
            })
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
            let game = GameId::from_id(game).unwrap();
            assert_eq!(scenario_startup_command(game, "levels\\test\\map"), command);
            assert_eq!(tag_test_executable_for_game(Some(game)), executable);
        }
        assert_eq!(
            scenario_startup_command(GameId::Halo3, "levels\\my map\\my map"),
            "game_start \"levels\\my map\\my map\""
        );
        assert_eq!(
            scenario_startup_command(GameId::Halo3, "levels\\semi;colon"),
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

impl Baboon {




    pub(in crate::app) fn launch_scenario_in_sapien(&mut self, key: &str) {
        let context = {
            let Some(source) = self.model.source() else {
                self.model.status = "Scenario launching requires a loaded editing kit".to_owned();
                return;
            };
            let Some(entry) = self.model.entry_for_key(key) else {
                self.model.status = "The scenario tag is no longer in the source".to_owned();
                return;
            };
            match scenario_launch_context(source, entry) {
                Ok(context) => context,
                Err(error) => {
                    self.model.status = error;
                    return;
                }
            }
        };
        if !context.game.sapien_takes_scenario_argument() {
            self.model.status =
                "Opening a scenario directly in Sapien is not supported for this editing kit"
                    .to_owned();
            return;
        }
        let executable = context.kit_root.join("sapien.exe");
        if !executable.is_file() {
            self.model.status = format!("Sapien executable not found: {}", executable.display());
            return;
        }

        let dirty = self.model.kits[self.model.active]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set());
        if dirty {
            if let Err(error) = self.save_tag_by_key(key) {
                self.model.status = format!("Could not save scenario before launch: {error}");
                return;
            }
        }

        let mut process = Command::new(&executable);
        for (option, folder) in &context.tool_options {
            process.arg(option).arg(folder);
        }
        process
            .arg(&context.scenario_file)
            .current_dir(&context.kit_root);
        match process.spawn() {
            Ok(_) => {
                self.model.status = format!("Launched Sapien for {}", context.scenario_path);
            }
            Err(error) => {
                self.model.status = format!("Could not launch Sapien for this scenario: {error}");
            }
        }
    }



    pub(in crate::app) fn launch_scenario_in_tag_test(&mut self, key: &str) {
        let context = {
            let Some(source) = self.model.source() else {
                self.model.status = "Scenario launching requires a loaded editing kit".to_owned();
                return;
            };
            let Some(entry) = self.model.entry_for_key(key) else {
                self.model.status = "The scenario tag is no longer in the source".to_owned();
                return;
            };
            match scenario_launch_context(source, entry) {
                Ok(context) => context,
                Err(error) => {
                    self.model.status = error;
                    return;
                }
            }
        };
        let executable_name = tag_test_executable_for_game(Some(context.game));
        let executable = context.kit_root.join(executable_name);
        if !executable.is_file() {
            self.model.status = format!("tag_test executable not found: {}", executable.display());
            return;
        }

        let dirty = self.model.kits[self.model.active]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set());
        if dirty {
            if let Err(error) = self.save_tag_by_key(key) {
                self.model.status = format!("Could not save scenario before launch: {error}");
                return;
            }
        }

        let startup_file = context.kit_root.join("init.txt");
        let command = scenario_startup_command(context.game, &context.scenario_path);
        if let Err(error) = update_scenario_startup_file(&startup_file, &command) {
            self.model.status = error;
            return;
        }
        let mut process = Command::new(&executable);
        for (option, folder) in &context.tool_options {
            process.arg(option).arg(folder);
        }
        process.current_dir(&context.kit_root);
        match process.spawn() {
            Ok(_) => {
                self.model.status = format!(
                    "Launched tag_test for {} using {}",
                    context.scenario_path,
                    startup_file.display()
                );
            }
            Err(error) => {
                self.model.status = format!(
                    "Wrote {}, but could not launch tag_test: {error}",
                    startup_file.display()
                );
            }
        }
    }
}

impl Model {
    /// Whether this workspace's editing kit has a Sapien that can open a
    /// scenario at all — the question of whether to *offer* the button, as
    /// opposed to whether it can be pressed right now.
    ///
    /// Answered from the kit's game alone, deliberately. Whether a particular
    /// scenario resolves to a launchable path, and whether `sapien.exe` is
    /// where it should be, are reasons to grey the button out; a kit whose
    /// Sapien has no way to be given a scenario is a reason for there to be no
    /// button.
    pub(in crate::app) fn kit_offers_scenario_sapien(&self, kit: usize) -> bool {
        self.kits
            .get(kit)
            .and_then(|kit| kit.source.as_ref())
            .and_then(|source| source.game)
            .is_some_and(GameFacts::sapien_takes_scenario_argument)
    }

    pub(in crate::app) fn can_launch_scenario_in_sapien(&self, kit: usize, entry: &TagEntry) -> bool {
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return false;
        };
        let Ok(context) = scenario_launch_context(source, entry) else {
            return false;
        };
        context.game.sapien_takes_scenario_argument()
            && context.kit_root.join("sapien.exe").is_file()
    }

    pub(in crate::app) fn can_launch_scenario_in_tag_test(&self, kit: usize, entry: &TagEntry) -> bool {
        let Some(source) = self.kits.get(kit).and_then(|kit| kit.source.as_ref()) else {
            return false;
        };
        let Ok(context) = scenario_launch_context(source, entry) else {
            return false;
        };
        let executable = tag_test_executable_for_game(Some(context.game));
        context.kit_root.join(executable).is_file()
    }
}
