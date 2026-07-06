use super::*;

pub(super) fn prefs_path() -> PathBuf {
    app_prefs_path("Baboon", "baboon", ".baboon-prefs.json")
}

pub(super) fn last_session_path() -> PathBuf {
    app_data_path(
        "Baboon",
        "baboon",
        "last_session.json",
        ".baboon-last-session.json",
    )
}

pub(super) fn terminal_logs_dir() -> PathBuf {
    if let Some(dir) = app_data_dir("Baboon", "baboon") {
        return dir.join("terminal-logs");
    }
    PathBuf::from("terminal-logs")
}

fn legacy_prefs_path() -> PathBuf {
    app_prefs_path("Genesis", "genesis", ".genesis-prefs.json")
}

fn app_prefs_path(windows_folder: &str, unix_folder: &str, fallback_name: &str) -> PathBuf {
    app_data_path(windows_folder, unix_folder, "prefs.json", fallback_name)
}

fn app_data_path(
    windows_folder: &str,
    unix_folder: &str,
    filename: &str,
    fallback_name: &str,
) -> PathBuf {
    if let Some(dir) = app_data_dir(windows_folder, unix_folder) {
        return dir.join(filename);
    }
    PathBuf::from(fallback_name)
}

fn app_data_dir(windows_folder: &str, unix_folder: &str) -> Option<PathBuf> {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return Some(PathBuf::from(appdata).join(windows_folder));
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return Some(PathBuf::from(home).join(".config").join(unix_folder));
    }
    None
}

fn read_prefs_text() -> Option<String> {
    fs::read_to_string(prefs_path())
        .or_else(|_| fs::read_to_string(legacy_prefs_path()))
        .ok()
}

pub(super) fn load_gui_prefs() -> GuiPrefs {
    let Some(text) = read_prefs_text() else {
        return GuiPrefs::default();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return GuiPrefs::default();
    };
    let browser_mode = match value.get("browser_mode").and_then(Value::as_str) {
        Some("groups") => BrowserMode::Groups,
        _ => BrowserMode::Folders,
    };
    let browser_sort = match value.get("browser_sort").and_then(Value::as_str) {
        Some("name") => BrowserSort::Name,
        Some("type") => BrowserSort::Type,
        _ => BrowserSort::Natural,
    };
    GuiPrefs {
        browser_mode,
        browser_sort,
        show_browser_prefixes: value
            .get("show_browser_prefixes")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        folders_before_tags: value
            .get("folders_before_tags")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        double_click_to_open_tags: value
            .get("double_click_to_open_tags")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        session_restore: value
            .get("session_restore")
            .and_then(Value::as_str)
            .and_then(SessionRestore::from_str)
            .unwrap_or_else(|| {
                // Migrate the old boolean: true → Always, absent/false → Ask.
                match value
                    .get("auto_restore_last_session")
                    .and_then(Value::as_bool)
                {
                    Some(true) => SessionRestore::Always,
                    _ => SessionRestore::Ask,
                }
            }),
        show_block_sizes: value
            .get("show_block_sizes")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        scroll_to_cycle_dropdowns: value
            .get("scroll_to_cycle_dropdowns")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        expert_mode: value
            .get("expert_mode")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        dark_mode: value
            .get("dark_mode")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        ui_scale: value
            .get("ui_scale")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(DEFAULT_UI_SCALE)
            .clamp(MIN_UI_SCALE, MAX_UI_SCALE),
        model_preview_size: value
            .get("model_preview_size")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(DEFAULT_MODEL_PREVIEW_SIZE)
            .clamp(MIN_MODEL_PREVIEW_SIZE, MAX_MODEL_PREVIEW_SIZE),
        blender_path: value
            .get("blender_path")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from),
        editing_kit_paths: load_editing_kit_paths(&value),
        ek_folder_aliases: load_ek_folder_aliases(&value),
        tool_commands_window_pos: load_pos2(&value, "tool_commands_window_pos"),
        tool_commands_window_size: load_vec2(&value, "tool_commands_window_size"),
        tool_commands_left_width: value
            .get("tool_commands_left_width")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(DEFAULT_TOOL_COMMANDS_LEFT_WIDTH)
            .max(MIN_TOOL_COMMANDS_LEFT_WIDTH),
        tool_commands_collapsed_categories: load_string_set(
            &value,
            "tool_commands_collapsed_categories",
        ),
        recent_folders: load_path_list(&value, "recent_folders"),
        custom_color_swatches: load_custom_color_swatches(&value),
        palette_last_dir: value
            .get("palette_last_dir")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from),
    }
}

fn load_pos2(value: &Value, key: &str) -> Option<egui::Pos2> {
    let arr = value.get(key)?.as_array()?;
    let x = arr.first()?.as_f64()? as f32;
    let y = arr.get(1)?.as_f64()? as f32;
    Some(egui::pos2(x, y))
}

fn load_vec2(value: &Value, key: &str) -> Option<Vec2> {
    let arr = value.get(key)?.as_array()?;
    let x = arr.first()?.as_f64()? as f32;
    let y = arr.get(1)?.as_f64()? as f32;
    Some(Vec2::new(
        x.max(MIN_TOOL_COMMANDS_WINDOW_SIZE.x),
        y.max(MIN_TOOL_COMMANDS_WINDOW_SIZE.y),
    ))
}

fn load_string_set(value: &Value, key: &str) -> HashSet<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn load_path_list(value: &Value, key: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(items) = value.get(key).and_then(Value::as_array) {
        for item in items {
            let Some(path) = item.as_str().map(str::trim).filter(|path| !path.is_empty()) else {
                continue;
            };
            let path = clean_recent_path(PathBuf::from(path));
            if !paths
                .iter()
                .any(|existing| same_recent_path(existing, &path))
            {
                paths.push(path);
            }
            if paths.len() >= MAX_RECENT_FOLDERS {
                break;
            }
        }
    }
    paths
}

fn load_editing_kit_paths(value: &Value) -> HashMap<String, PathBuf> {
    let mut paths = HashMap::new();
    let Some(entries) = value.get("editing_kit_paths").and_then(Value::as_object) else {
        return paths;
    };
    for shortcut in EDITING_KIT_SHORTCUTS {
        let Some(path) = entries
            .get(shortcut.game)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        paths.insert(shortcut.game.to_owned(), PathBuf::from(path));
    }
    paths
}

fn load_custom_color_swatches(value: &Value) -> Vec<Option<[u8; 4]>> {
    let mut swatches = vec![None; CUSTOM_COLOR_SWATCH_COUNT];
    if let Some(items) = value.get("custom_color_swatches").and_then(Value::as_array) {
        for (index, item) in items.iter().take(CUSTOM_COLOR_SWATCH_COUNT).enumerate() {
            let Some(text) = item.as_str() else {
                continue;
            };
            swatches[index] = parse_pref_rgba(text);
        }
    }
    swatches
}

fn parse_pref_rgba(text: &str) -> Option<[u8; 4]> {
    let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if hex.len() != 8 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
        u8::from_str_radix(&hex[6..8], 16).ok()?,
    ])
}

pub(super) fn clean_recent_path(path: PathBuf) -> PathBuf {
    let text = path.display().to_string();
    #[cfg(windows)]
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned();
    #[cfg(not(windows))]
    let text = text;
    PathBuf::from(text)
}

pub(super) fn same_recent_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn load_ek_folder_aliases(value: &Value) -> Vec<EkFolderAlias> {
    value
        .get("ek_folder_aliases")
        .and_then(Value::as_array)
        .map(|aliases| {
            aliases
                .iter()
                .filter_map(|alias| {
                    let folder_name = alias.get("folder_name")?.as_str()?.trim();
                    let game = alias.get("game")?.as_str()?.trim();
                    if folder_name.is_empty() {
                        return None;
                    }
                    Some(EkFolderAlias {
                        folder_name: folder_name.to_owned(),
                        game: supported_ek_game_id(game)?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn save_gui_prefs(
    prefs: &GuiPrefs,
    terminal_open_games: &HashSet<String>,
) -> Result<(), String> {
    let path = prefs_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create preferences folder: {error}"))?;
    }
    let mut games: Vec<&String> = terminal_open_games.iter().collect();
    games.sort();
    let mut collapsed_tool_categories: Vec<&String> =
        prefs.tool_commands_collapsed_categories.iter().collect();
    collapsed_tool_categories.sort();
    let mut editing_kit_paths = serde_json::Map::new();
    for shortcut in EDITING_KIT_SHORTCUTS {
        if let Some(path) = prefs.editing_kit_paths.get(shortcut.game) {
            let text = path.display().to_string();
            if !text.trim().is_empty() {
                editing_kit_paths.insert(shortcut.game.to_owned(), json!(text));
            }
        }
    }
    let value = json!({
        "browser_mode": match prefs.browser_mode {
            BrowserMode::Folders => "folders",
            BrowserMode::Groups => "groups",
        },
        "browser_sort": match prefs.browser_sort {
            BrowserSort::Natural => "natural",
            BrowserSort::Name => "name",
            BrowserSort::Type => "type",
        },
        "show_browser_prefixes": prefs.show_browser_prefixes,
        "folders_before_tags": prefs.folders_before_tags,
        "double_click_to_open_tags": prefs.double_click_to_open_tags,
        "session_restore": prefs.session_restore.as_str(),
        "show_block_sizes": prefs.show_block_sizes,
        "scroll_to_cycle_dropdowns": prefs.scroll_to_cycle_dropdowns,
        "expert_mode": prefs.expert_mode,
        "dark_mode": prefs.dark_mode,
        "ui_scale": prefs.ui_scale,
        "model_preview_size": prefs.model_preview_size,
        "blender_path": prefs.blender_path.as_ref().map(|path| path.display().to_string()),
        "editing_kit_paths": editing_kit_paths,
        "ek_folder_aliases": prefs.ek_folder_aliases.iter().map(|alias| {
            json!({
                "folder_name": alias.folder_name,
                "game": alias.game,
            })
        }).collect::<Vec<_>>(),
        "tool_commands_window_pos": prefs.tool_commands_window_pos.map(|pos| vec![pos.x, pos.y]),
        "tool_commands_window_size": prefs.tool_commands_window_size.map(|size| vec![size.x, size.y]),
        "tool_commands_left_width": prefs.tool_commands_left_width,
        "tool_commands_collapsed_categories": collapsed_tool_categories,
        "recent_folders": prefs.recent_folders.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
        "custom_color_swatches": prefs.custom_color_swatches.iter().map(|swatch| {
            swatch.map(|rgba| format!("#{:02X}{:02X}{:02X}{:02X}", rgba[0], rgba[1], rgba[2], rgba[3]))
        }).collect::<Vec<_>>(),
        "palette_last_dir": prefs.palette_last_dir.as_ref().map(|path| path.display().to_string()),
        "terminal_open_games": games,
    });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Could not encode preferences: {error}"))?;
    fs::write(path, text).map_err(|error| format!("Could not save preferences: {error}"))
}

/// Load the set of game identifiers for which the terminal should auto-open.
/// Reads the same prefs.json as `load_gui_prefs`.
pub(super) fn load_terminal_open_games() -> HashSet<String> {
    let Some(text) = read_prefs_text() else {
        return HashSet::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return HashSet::new();
    };
    value
        .get("terminal_open_games")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn load_last_session() -> Option<LastSessionState> {
    let text = fs::read_to_string(last_session_path()).ok()?;
    let value = serde_json::from_str::<Value>(&text).ok()?;
    if value.get("version").and_then(Value::as_u64)? != 1 {
        return None;
    }
    let source = value.get("source")?;
    let source_kind = LastSessionSourceKind::from_str(source.get("kind")?.as_str()?.trim())?;
    let source_path = source
        .get("path")?
        .as_str()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)?;
    let game = source
        .get("game")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|game| !game.is_empty())
        .map(str::to_owned);
    let mut tags = Vec::new();
    for item in value.get("tags")?.as_array()? {
        let Some(key) = item
            .get("key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|key| !key.is_empty())
        else {
            continue;
        };
        let label = item
            .get("label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .unwrap_or(key)
            .to_owned();
        let group_tag = item.get("group_tag").and_then(Value::as_u64).unwrap_or(0) as u32;
        let path = item
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        tags.push(LastSessionTag {
            key: key.to_owned(),
            label,
            group_tag,
            path,
        });
    }
    if tags.is_empty() {
        return None;
    }
    Some(LastSessionState {
        source_kind,
        source_path,
        game,
        tags,
    })
}

/// Persist the source and open tag keys used by the launch-time restore
/// prompt. This is written only from the confirmed app-exit path, so a crash or
/// canceled close leaves the previous successfully closed session intact.
pub(super) fn save_last_session(session: &LastSessionState) -> Result<(), String> {
    let path = last_session_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create session folder: {error}"))?;
    }
    let tags = session
        .tags
        .iter()
        .map(|tag| {
            json!({
                "key": tag.key,
                "label": tag.label,
                "group_tag": tag.group_tag,
                "path": tag.path.as_ref().map(|path| path.display().to_string()),
            })
        })
        .collect::<Vec<_>>();
    let value = json!({
        "version": 1,
        "source": {
            "kind": session.source_kind.as_str(),
            "path": session.source_path.display().to_string(),
            "game": session.game,
        },
        "tags": tags,
    });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Could not encode session: {error}"))?;
    fs::write(path, text).map_err(|error| format!("Could not save session: {error}"))
}

pub(super) fn clear_last_session() {
    let _ = fs::remove_file(last_session_path());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_color_swatches_load_as_fixed_global_slots() {
        let value = serde_json::json!({
            "custom_color_swatches": [
                "#FF0000FF",
                null,
                "#33669980",
                "not-a-color"
            ]
        });

        let swatches = load_custom_color_swatches(&value);
        assert_eq!(swatches.len(), CUSTOM_COLOR_SWATCH_COUNT);
        assert_eq!(swatches[0], Some([255, 0, 0, 255]));
        assert_eq!(swatches[1], None);
        assert_eq!(swatches[2], Some([51, 102, 153, 128]));
        assert_eq!(swatches[3], None);
    }

    #[test]
    fn load_editing_kit_paths_ignores_empty_and_unknown_entries() {
        let value = json!({
            "editing_kit_paths": {
                "halo3_mcc": "C:/Games/H3EK",
                "halo4_mcc": "",
                "unknown": "C:/Games/Unknown"
            }
        });

        let paths = load_editing_kit_paths(&value);

        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths.get("halo3_mcc"),
            Some(&PathBuf::from("C:/Games/H3EK"))
        );
        assert!(!paths.contains_key("halo4_mcc"));
        assert!(!paths.contains_key("unknown"));
    }
}
