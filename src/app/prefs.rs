//! Preferences and last-session persistence, including legacy migration.
//! It owns preference/session serialization and migration; interactive settings presentation belongs to the UI layer.

use super::*;

pub(super) fn prefs_path() -> PathBuf {
    crate::storage::data_path("prefs.json")
}

pub(super) fn last_session_path() -> PathBuf {
    crate::storage::data_path("last_session.json")
}

pub(super) fn terminal_logs_dir() -> PathBuf {
    crate::storage::data_path("terminal-logs")
}

fn legacy_prefs_path() -> PathBuf {
    crate::storage::legacy_installed_path("prefs.json")
}

fn read_prefs_text() -> Option<String> {
    fs::read_to_string(prefs_path())
        .or_else(|_| fs::read_to_string(legacy_prefs_path()))
        .ok()
}

pub(super) fn load_first_run_complete() -> bool {
    first_run_complete_from_text(read_prefs_text().as_deref())
}

fn first_run_complete_from_text(text: Option<&str>) -> bool {
    let Some(text) = text else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        // A pre-existing but malformed preference file still means this is not
        // the user's first launch; normal preference loading will use defaults.
        return true;
    };
    value
        .get("first_run_complete")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

#[cfg(test)]
#[path = "tests/prefs_first_run.rs"]
mod first_run_tests;

pub(super) fn load_gui_prefs() -> GuiPrefs {
    let Some(text) = read_prefs_text() else {
        return GuiPrefs::default();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return GuiPrefs::default();
    };
    let browser_mode =
        browser_mode_from_str(value.get("browser_mode").and_then(Value::as_str)).unwrap_or_default();
    let browser_sort =
        browser_sort_from_str(value.get("browser_sort").and_then(Value::as_str)).unwrap_or_default();
    GuiPrefs {
        browser_mode,
        browser_sort,
        nested_default: nested_default_from_str(
            value.get("nested_default").and_then(Value::as_str),
        )
        .unwrap_or_default(),
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
        confirm_container_overwrite: value
            .get("confirm_container_overwrite")
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
        custom_editing_kit_profiles: load_custom_editing_kit_profiles(&value),
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
        editing_kit_favorites: load_editing_kit_favorites(&value),
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

fn load_editing_kit_favorites(value: &Value) -> Vec<EditingKitFavorites> {
    let Some(kits) = value.get("editing_kit_favorites").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut favorites: Vec<EditingKitFavorites> = Vec::new();
    for kit in kits {
        let Some(root) = kit
            .get("tags_root")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|root| !root.is_empty())
        else {
            continue;
        };
        let root = clean_recent_path(PathBuf::from(root));
        let Some(tags) = kit.get("tags").and_then(Value::as_array) else {
            continue;
        };
        let mut relative_paths: Vec<PathBuf> = Vec::new();
        for tag in tags {
            let Some(path) = tag
                .as_str()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .and_then(|path| clean_favorite_relative_path(PathBuf::from(path)))
            else {
                continue;
            };
            if !relative_paths
                .iter()
                .any(|existing| same_recent_path(existing, &path))
            {
                relative_paths.push(path);
            }
        }
        if relative_paths.is_empty() {
            continue;
        }
        if let Some(existing) = favorites
            .iter_mut()
            .find(|existing| same_recent_path(&existing.tags_root, &root))
        {
            for path in relative_paths {
                if !existing
                    .tags
                    .iter()
                    .any(|current| same_recent_path(current, &path))
                {
                    existing.tags.push(path);
                }
            }
        } else {
            favorites.push(EditingKitFavorites {
                tags_root: root,
                tags: relative_paths,
            });
        }
    }
    favorites
}

pub(super) fn clean_favorite_relative_path(path: PathBuf) -> Option<PathBuf> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    Some(path)
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
    let text = if text
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
    {
        format!(r"\\{}", &text[8..])
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    };
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

fn load_custom_editing_kit_profiles(value: &Value) -> Vec<CustomEditingKitProfile> {
    let Some(entries) = value
        .get("custom_editing_kit_profiles")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut profiles = Vec::new();
    let mut ids = HashSet::new();
    for entry in entries {
        let Some(id) = entry
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        else {
            continue;
        };
        if !ids.insert(id.to_owned()) {
            continue;
        }
        let Some(name) = entry
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let Some(game) = entry
            .get("game")
            .and_then(Value::as_str)
            .and_then(supported_ek_game_id)
            .filter(|game| *game != "haloce_evolved")
        else {
            continue;
        };
        let Some(root) = entry
            .get("root")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|root| !root.is_empty())
            .map(PathBuf::from)
            .map(clean_recent_path)
        else {
            continue;
        };
        let icon = entry
            .get("icon")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|icon| !icon.is_empty())
            .map(PathBuf::from)
            .filter(|icon| safe_custom_icon_relative_path(icon));
        profiles.push(CustomEditingKitProfile {
            id: id.to_owned(),
            name: name.to_owned(),
            game: game.to_owned(),
            root,
            icon,
        });
    }
    profiles
}

fn custom_editing_kit_profiles_value(
    profiles: &[CustomEditingKitProfile],
) -> Vec<Value> {
    profiles
        .iter()
        .map(|profile| {
            json!({
                "id": profile.id,
                "name": profile.name,
                "game": profile.game,
                "root": profile.root.display().to_string(),
                "icon": profile.icon.as_ref().map(|path| path.display().to_string()),
            })
        })
        .collect()
}

pub(super) fn save_gui_prefs(
    prefs: &GuiPrefs,
    terminal_open_games: &HashSet<String>,
    first_run_complete: bool,
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
        "browser_mode": browser_mode_str(prefs.browser_mode),
        "browser_sort": browser_sort_str(prefs.browser_sort),
        "nested_default": nested_default_str(prefs.nested_default),
        "show_browser_prefixes": prefs.show_browser_prefixes,
        "folders_before_tags": prefs.folders_before_tags,
        "double_click_to_open_tags": prefs.double_click_to_open_tags,
        "session_restore": prefs.session_restore.as_str(),
        "show_block_sizes": prefs.show_block_sizes,
        "scroll_to_cycle_dropdowns": prefs.scroll_to_cycle_dropdowns,
        "confirm_container_overwrite": prefs.confirm_container_overwrite,
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
        "custom_editing_kit_profiles": custom_editing_kit_profiles_value(&prefs.custom_editing_kit_profiles),
        "tool_commands_window_pos": prefs.tool_commands_window_pos.map(|pos| vec![pos.x, pos.y]),
        "tool_commands_window_size": prefs.tool_commands_window_size.map(|size| vec![size.x, size.y]),
        "tool_commands_left_width": prefs.tool_commands_left_width,
        "tool_commands_collapsed_categories": collapsed_tool_categories,
        "recent_folders": prefs.recent_folders.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
        "editing_kit_favorites": prefs.editing_kit_favorites.iter().filter(|kit| !kit.tags.is_empty()).map(|kit| {
            json!({
                "tags_root": kit.tags_root.display().to_string(),
                "tags": kit.tags.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "custom_color_swatches": prefs.custom_color_swatches.iter().map(|swatch| {
            swatch.map(|rgba| format!("#{:02X}{:02X}{:02X}{:02X}", rgba[0], rgba[1], rgba[2], rgba[3]))
        }).collect::<Vec<_>>(),
        "palette_last_dir": prefs.palette_last_dir.as_ref().map(|path| path.display().to_string()),
        "storage_mode": crate::storage::active_mode().map(crate::storage::StorageMode::as_str),
        "first_run_complete": first_run_complete,
        "terminal_open_games": games,
    });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Could not encode preferences: {error}"))?;
    write_text_atomic(&path, &text)
}

fn write_text_atomic(path: &Path, text: &str) -> Result<(), String> {
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text).map_err(|error| format!("Could not save preferences: {error}"))?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| format!("Could not replace preferences: {error}"))?;
    }
    fs::rename(&temp, path).map_err(|error| format!("Could not install preferences: {error}"))
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

/// Read `last_session.json`.
///
/// Version 3 stores an array of kits. Versions 1 and 2 stored a single source
/// and are upgraded in place to a one-kit session, so a session file written
/// by any earlier build still restores rather than being silently dropped.
/// The browser view enums travel as strings in both `prefs.json` and
/// `last_session.json`. Mapped in one place so the two files cannot drift.
fn browser_mode_from_str(text: Option<&str>) -> Option<BrowserMode> {
    match text? {
        "folders" => Some(BrowserMode::Folders),
        "groups" => Some(BrowserMode::Groups),
        _ => None,
    }
}

fn browser_mode_str(mode: BrowserMode) -> &'static str {
    match mode {
        BrowserMode::Folders => "folders",
        BrowserMode::Groups => "groups",
    }
}

fn browser_sort_from_str(text: Option<&str>) -> Option<BrowserSort> {
    match text? {
        "natural" => Some(BrowserSort::Natural),
        "name" => Some(BrowserSort::Name),
        "type" => Some(BrowserSort::Type),
        _ => None,
    }
}

fn nested_default_from_str(text: Option<&str>) -> Option<NestedDefault> {
    match text? {
        "schema" => Some(NestedDefault::Schema),
        "collapsed" => Some(NestedDefault::Collapsed),
        "expanded" => Some(NestedDefault::Expanded),
        _ => None,
    }
}

fn nested_default_str(nested: NestedDefault) -> &'static str {
    match nested {
        NestedDefault::Schema => "schema",
        NestedDefault::Collapsed => "collapsed",
        NestedDefault::Expanded => "expanded",
    }
}

fn browser_sort_str(sort: BrowserSort) -> &'static str {
    match sort {
        BrowserSort::Natural => "natural",
        BrowserSort::Name => "name",
        BrowserSort::Type => "type",
    }
}

pub(super) fn load_last_session() -> Option<LastSessionState> {
    let text = fs::read_to_string(last_session_path()).ok()?;
    let value = serde_json::from_str::<Value>(&text).ok()?;
    parse_last_session(&value)
}

/// Pure parse of a session document, split out from the file read so every
/// format version is covered by tests.
fn parse_last_session(value: &Value) -> Option<LastSessionState> {
    let kits = match value.get("version").and_then(Value::as_u64)? {
        // Versions 1 and 2 each describe a single source, so they load as one
        // kit. They are both accepted because they were both written: v1 by
        // released Baboon, v2 by the build that added `.baboon` projects.
        1 | 2 => vec![parse_session_kit(value)?],
        // Version 3 is that same per-kit object, once per open kit.
        3 => value
            .get("kits")?
            .as_array()?
            .iter()
            .filter_map(parse_session_kit)
            .collect(),
        _ => return None,
    };
    if kits.is_empty() {
        return None;
    }
    Some(LastSessionState { kits })
}

/// Parse one kit's `{source, tags}` object. Both format versions use the same
/// shape for this part, which is what makes the v1 upgrade a one-liner.
fn parse_session_kit(value: &Value) -> Option<LastSessionKit> {
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
    let profile_id = source
        .get("profile_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    let project_path = source
        .get("project_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    // Absent in sessions written before the browser view became per-kit, and
    // in every version-1 and version-2 file. `None` means "use the default".
    let browser_mode = browser_mode_from_str(value.get("browser_mode").and_then(Value::as_str));
    let browser_sort = browser_sort_from_str(value.get("browser_sort").and_then(Value::as_str));
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
    if tags.is_empty() && project_path.is_none() {
        return None;
    }
    Some(LastSessionKit {
        source_kind,
        source_path,
        game,
        profile_id,
        project_path,
        browser_mode,
        browser_sort,
        tags,
    })
}

/// Persist every kit's source and open tag keys for the launch-time restore
/// prompt. Written only from the confirmed app-exit path, so a crash or a
/// canceled close leaves the previous successfully closed session intact.
pub(super) fn save_last_session(session: &LastSessionState) -> Result<(), String> {
    let path = last_session_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create session folder: {error}"))?;
    }
    let text = serde_json::to_string_pretty(&session_value(session))
        .map_err(|error| format!("Could not encode session: {error}"))?;
    fs::write(path, text).map_err(|error| format!("Could not save session: {error}"))
}

/// Pure encode of a session document, split out from the file write so the
/// round trip through [`parse_last_session`] is covered by tests.
fn session_value(session: &LastSessionState) -> Value {
    let kits = session
        .kits
        .iter()
        .map(|kit| {
            let tags = kit
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
            json!({
                "source": {
                    "kind": kit.source_kind.as_str(),
                    "path": kit.source_path.display().to_string(),
                    "game": kit.game,
                    "profile_id": kit.profile_id,
                    "project_path": kit.project_path.as_ref().map(|path| path.display().to_string()),
                },
                "browser_mode": kit.browser_mode.map(browser_mode_str),
                "browser_sort": kit.browser_sort.map(browser_sort_str),
                "tags": tags,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "version": 3,
        "kits": kits,
    })
}

pub(super) fn clear_last_session() {
    let _ = fs::remove_file(last_session_path());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_session_v1_remains_compatible() {
        let session = parse_last_session(
            &serde_json::from_str::<Value>(
            r#"{
                "version": 1,
                "source": {
                    "kind": "loose_folder",
                    "path": "C:/tags",
                    "game": "haloreach"
                },
                "tags": [{
                    "key": "objects/test.weapon",
                    "label": "test",
                    "group_tag": 2003132784
                }]
            }"#,
            )
            .expect("valid json"),
        )
        .expect("version 1 session");
        // A single-source file loads as one kit.
        assert_eq!(session.kits.len(), 1);
        assert_eq!(session.kits[0].source_kind, LastSessionSourceKind::LooseFolder);
        assert_eq!(session.kits[0].project_path, None);
        assert_eq!(session.kits[0].tags.len(), 1);
    }

    #[test]
    fn project_pointer_survives_with_no_open_tabs() {
        let session = parse_last_session(
            &serde_json::from_str::<Value>(
            r#"{
                "version": 2,
                "source": {
                    "kind": "iostore_container_set",
                    "path": "C:/CampaignEvolved/Paks",
                    "game": "haloce_evolved",
                    "project_path": "C:/mods/recovery.baboon"
                },
                "tags": []
            }"#,
            )
            .expect("valid json"),
        )
        .expect("project-only session");
        assert_eq!(session.kits.len(), 1);
        assert_eq!(
            session.kits[0].source_kind,
            LastSessionSourceKind::IoStoreContainerSet
        );
        assert_eq!(
            session.kits[0].project_path,
            Some(PathBuf::from("C:/mods/recovery.baboon"))
        );
        assert!(session.kits[0].tags.is_empty());
    }

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
                "haloce_evolved": "D:/Games/Halo Campaign Evolved",
                "halo4_mcc": "",
                "unknown": "C:/Games/Unknown"
            }
        });

        let paths = load_editing_kit_paths(&value);

        assert_eq!(paths.len(), 2);
        assert_eq!(
            paths.get("halo3_mcc"),
            Some(&PathBuf::from("C:/Games/H3EK"))
        );
        assert_eq!(
            paths.get("haloce_evolved"),
            Some(&PathBuf::from("D:/Games/Halo Campaign Evolved"))
        );
        assert!(!paths.contains_key("halo4_mcc"));
        assert!(!paths.contains_key("unknown"));
    }

    #[cfg(windows)]
    #[test]
    fn clean_recent_path_hides_windows_verbatim_prefixes() {
        assert_eq!(
            clean_recent_path(PathBuf::from(r"\\?\D:\Games\H2EK")),
            PathBuf::from(r"D:\Games\H2EK")
        );
        assert_eq!(
            clean_recent_path(PathBuf::from(r"\\?\UNC\server\share\H3EK")),
            PathBuf::from(r"\\server\share\H3EK")
        );
        assert_eq!(
            clean_recent_path(PathBuf::from(r"D:\Games\H4EK")),
            PathBuf::from(r"D:\Games\H4EK")
        );
    }

    #[test]
    fn custom_editing_kit_profiles_round_trip_in_creation_order() {
        assert!(load_custom_editing_kit_profiles(&json!({})).is_empty());
        let value = json!({
            "custom_editing_kit_profiles": [
                {
                    "id": "11111111-1111-4111-8111-111111111111",
                    "name": "Reach Project",
                    "game": "haloreach_mcc",
                    "root": "\\\\?\\D:\\Kits\\ReachProject",
                    "icon": "editing kit icons/reach-11111111/icon-a.png"
                },
                {
                    "id": "22222222-2222-4222-8222-222222222222",
                    "name": "Second Project",
                    "game": "halo3_mcc",
                    "root": "D:/Kits/H3Project",
                    "icon": "../../unsafe.png"
                }
            ]
        });
        let profiles = load_custom_editing_kit_profiles(&value);
        assert_eq!(
            profiles
                .iter()
                .map(|profile| profile.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Reach Project", "Second Project"]
        );
        assert_eq!(
            profiles[0].icon.as_deref(),
            Some(Path::new(
                "editing kit icons/reach-11111111/icon-a.png"
            ))
        );
        assert_eq!(profiles[1].icon, None);
        #[cfg(windows)]
        assert_eq!(profiles[0].root, PathBuf::from(r"D:\Kits\ReachProject"));

        let serialized = json!({
            "custom_editing_kit_profiles": custom_editing_kit_profiles_value(&profiles)
        });
        assert_eq!(load_custom_editing_kit_profiles(&serialized), profiles);
    }

    #[test]
    fn editing_kit_favorites_are_scoped_by_tags_root() {
        let value = json!({
            "editing_kit_favorites": [
                {
                    "tags_root": "C:/Games/H2EK/tags",
                    "tags": [
                        "objects/brute.model",
                        "objects/brute.model",
                        "../outside.model"
                    ]
                },
                {
                    "tags_root": "C:/Games/H3EK/tags",
                    "tags": ["objects/brute.model"]
                }
            ]
        });

        let favorites = load_editing_kit_favorites(&value);

        assert_eq!(favorites.len(), 2);
        assert_eq!(favorites[0].tags_root, PathBuf::from("C:/Games/H2EK/tags"));
        assert_eq!(
            favorites[0].tags,
            vec![PathBuf::from("objects/brute.model")]
        );
        assert_eq!(favorites[1].tags_root, PathBuf::from("C:/Games/H3EK/tags"));
        assert_eq!(
            favorites[1].tags,
            vec![PathBuf::from("objects/brute.model")]
        );
    }

    #[test]
    fn favorite_paths_must_be_relative_and_normalized() {
        assert_eq!(
            clean_favorite_relative_path(PathBuf::from("objects/brute.model")),
            Some(PathBuf::from("objects/brute.model"))
        );
        assert!(clean_favorite_relative_path(PathBuf::from("../brute.model")).is_none());
        assert!(clean_favorite_relative_path(PathBuf::from("./brute.model")).is_none());
        assert!(clean_favorite_relative_path(PathBuf::new()).is_none());
    }
}

#[cfg(test)]
mod nested_default_tests {
    use super::*;

    /// The stored spelling has to round trip, or the setting silently reverts
    /// to Default on the next launch.
    #[test]
    fn every_nested_default_round_trips_through_its_stored_name() {
        for option in NestedDefault::ALL {
            assert_eq!(
                nested_default_from_str(Some(nested_default_str(option))),
                Some(option),
                "{} did not round trip",
                option.label()
            );
        }
        // An absent or unrecognised value falls back rather than failing the
        // whole preferences load.
        assert_eq!(nested_default_from_str(None), None);
        assert_eq!(nested_default_from_str(Some("nonsense")), None);
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    /// A version-1 file predates multiple kits. It must still restore, as one
    /// kit — silently dropping it would lose the user's open tags on upgrade.
    #[test]
    fn version_1_sessions_upgrade_to_a_single_kit() {
        let value = serde_json::json!({
            "version": 1,
            "source": { "kind": "loose_folder", "path": "/tags", "game": "halo3_mcc" },
            "tags": [{ "key": "file:/tags/a.weapon", "label": "a", "group_tag": 1, "path": null }],
        });
        let session = parse_last_session(&value).expect("v1 session parses");
        assert_eq!(session.kits.len(), 1);
        assert_eq!(session.kits[0].game.as_deref(), Some("halo3_mcc"));
        assert_eq!(session.kits[0].tags.len(), 1);
    }

    #[test]
    fn version_3_sessions_restore_every_kit() {
        let value = serde_json::json!({
            "version": 3,
            "kits": [
                {
                    "source": { "kind": "loose_folder", "path": "/h3", "game": "halo3_mcc" },
                    "tags": [{ "key": "file:/h3/a.weapon", "label": "a", "group_tag": 1 }],
                },
                {
                    "source": { "kind": "loose_folder", "path": "/reach", "game": "haloreach_mcc" },
                    "tags": [{ "key": "file:/reach/b.weapon", "label": "b", "group_tag": 1 }],
                },
            ],
        });
        let session = parse_last_session(&value).expect("v3 session parses");
        assert_eq!(session.kits.len(), 2);
        assert_eq!(session.kits[1].game.as_deref(), Some("haloreach_mcc"));
        assert_eq!(session.kits[1].tags[0].key, "file:/reach/b.weapon");
    }

    #[test]
    fn custom_profile_identity_survives_session_round_trip() {
        let mut custom = kit("/custom-reach", Some(BrowserMode::Folders));
        custom.game = Some("haloreach_mcc".to_owned());
        custom.profile_id = Some("11111111-1111-4111-8111-111111111111".to_owned());
        let restored = parse_last_session(&session_value(&LastSessionState {
            kits: vec![custom],
        }))
        .expect("custom profile session parses");
        assert_eq!(
            restored.kits[0].profile_id.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(restored.kits[0].game.as_deref(), Some("haloreach_mcc"));
    }

    /// A kit whose tags all failed to parse contributes nothing, and a session
    /// left with no kits at all is no session.
    #[test]
    fn kits_without_usable_tags_are_dropped() {
        let value = serde_json::json!({
            "version": 3,
            "kits": [
                { "source": { "kind": "loose_folder", "path": "/h3" }, "tags": [] },
                {
                    "source": { "kind": "loose_folder", "path": "/reach" },
                    "tags": [{ "key": "file:/reach/b.weapon", "label": "b", "group_tag": 1 }],
                },
            ],
        });
        let session = parse_last_session(&value).expect("session parses");
        assert_eq!(session.kits.len(), 1);
        assert_eq!(session.kits[0].source_path, PathBuf::from("/reach"));

        let empty = serde_json::json!({ "version": 3, "kits": [] });
        assert!(parse_last_session(&empty).is_none());
    }

    #[test]
    fn unknown_versions_are_ignored() {
        let value = serde_json::json!({ "version": 99, "kits": [] });
        assert!(parse_last_session(&value).is_none());
    }

    fn kit(path: &str, mode: Option<BrowserMode>) -> LastSessionKit {
        LastSessionKit {
            source_kind: LastSessionSourceKind::LooseFolder,
            source_path: PathBuf::from(path),
            game: None,
            profile_id: None,
            project_path: None,
            browser_mode: mode,
            browser_sort: Some(BrowserSort::Name),
            tags: vec![LastSessionTag {
                key: format!("file:{path}/a.weapon"),
                label: "a".to_owned(),
                group_tag: 1,
                path: None,
            }],
        }
    }

    /// Each workspace keeps its own browser view, so a session holding a kit
    /// in Folders and one in Groups must bring both back as they were — not
    /// collapse them onto one setting.
    #[test]
    fn each_kit_restores_its_own_browser_view() {
        let session = LastSessionState {
            kits: vec![
                kit("/evolved", Some(BrowserMode::Folders)),
                kit("/reach", Some(BrowserMode::Groups)),
            ],
        };
        let restored = parse_last_session(&session_value(&session)).expect("round trip");
        assert_eq!(restored.kits[0].browser_mode, Some(BrowserMode::Folders));
        assert_eq!(restored.kits[1].browser_mode, Some(BrowserMode::Groups));
        assert_eq!(restored.kits[0].browser_sort, Some(BrowserSort::Name));
    }

    /// Sessions written before the view was saved carry none, which has to
    /// stay distinguishable from a saved Folders so the restore can fall back
    /// to the user's default instead of overriding it.
    #[test]
    fn sessions_without_a_saved_view_restore_none() {
        let value = serde_json::json!({
            "version": 3,
            "kits": [{
                "source": { "kind": "loose_folder", "path": "/h3" },
                "tags": [{ "key": "file:/h3/a.weapon", "label": "a", "group_tag": 1 }],
            }],
        });
        let session = parse_last_session(&value).expect("session parses");
        assert_eq!(session.kits[0].browser_mode, None);
        assert_eq!(session.kits[0].browser_sort, None);
    }

    /// A kit whose session is its project has no tags to save, so dropping the
    /// project path on write left nothing to restore it from.
    #[test]
    fn a_projects_path_survives_the_round_trip() {
        let mut project_kit = kit("/evolved", None);
        project_kit.project_path = Some(PathBuf::from("/evolved/work.baboon"));
        project_kit.tags.clear();
        let session = LastSessionState {
            kits: vec![project_kit],
        };
        let restored = parse_last_session(&session_value(&session)).expect("round trip");
        assert_eq!(
            restored.kits[0].project_path,
            Some(PathBuf::from("/evolved/work.baboon"))
        );
    }
}
