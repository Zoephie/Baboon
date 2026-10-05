//! Preferences and last-session persistence, including legacy migration.
//! It owns preference/session serialization and migration; interactive settings presentation belongs to the UI layer.

use crate::app::kits::detect::add_standard_editing_kit_profiles;
use super::*;
use crate::app::kits::safe_custom_icon_relative_path;
use crate::app::editor::default_color_swatches;
use crate::app::browser::{BrowserMode, BrowserSearchScope, BrowserSort};

pub(super) fn prefs_path() -> PathBuf {
    crate::core::storage::data_path("prefs.json")
}

pub(super) fn terminal_logs_dir() -> PathBuf {
    crate::core::storage::data_path("terminal-logs")
}

fn legacy_prefs_path() -> PathBuf {
    crate::core::storage::legacy_installed_path("prefs.json")
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





pub(super) fn load_gui_prefs() -> GuiPrefs {
    let Some(text) = read_prefs_text() else {
        return GuiPrefs::default();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return GuiPrefs::default();
    };
    let prefs = prefs_from_value(&value);
    if value.get("editing_kit_profiles").is_none()
        && (value.get("editing_kit_paths").is_some()
            || value.get("custom_editing_kit_profiles").is_some())
    {
        // Preserve unrelated preferences and onboarding state during the one-time upgrade.
        let mut migrated = value.clone();
        if let Some(object) = migrated.as_object_mut() {
            object.remove("editing_kit_paths");
            object.remove("custom_editing_kit_profiles");
            let mut profiles = custom_editing_kit_profiles_value(&prefs.custom_editing_kit_profiles);
            profiles.extend(prefs.unusable_kit_entries.profiles.iter().cloned());
            object.insert("editing_kit_profiles".to_owned(), Value::Array(profiles));
            if let Ok(text) = serde_json::to_string_pretty(&migrated)
                && let Err(error) = write_text_atomic(&prefs_path(), &text, "preferences")
            {
                eprintln!("Could not save editing-kit migration: {error}");
            }
        }
    }
    prefs
}

/// Decodes stored preferences, falling back to the default for anything the
/// file does not carry — which is how a file written before a preference
/// existed keeps working.
fn prefs_from_value(value: &Value) -> GuiPrefs {
    let saved_scope = value.get("browser_search_scope");
    let mut browser_search_scope = BrowserSearchScope {
        tags: saved_scope
            .and_then(|v| v.get("tags"))
            .and_then(Value::as_bool)
            .unwrap_or(true),
        folders: saved_scope
            .and_then(|v| v.get("folders"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        keywords: saved_scope
            .and_then(|v| v.get("keywords"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    if !browser_search_scope.tags && !browser_search_scope.folders && !browser_search_scope.keywords
    {
        browser_search_scope = BrowserSearchScope::default();
    }
    let browser_mode = browser_mode_from_str(value.get("browser_mode").and_then(Value::as_str))
        .unwrap_or_default();
    let browser_sort = browser_sort_from_str(value.get("browser_sort").and_then(Value::as_str))
        .unwrap_or_default();
    GuiPrefs {
        browser_mode,
        browser_sort,
        browser_search_scope,
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
        update_channel: value
            .get("update_channel")
            .and_then(Value::as_str)
            .and_then(UpdateChannel::from_str)
            .unwrap_or_default(),
        check_updates_on_startup: value
            .get("check_updates_on_startup")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        show_block_sizes: value
            .get("show_block_sizes")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        angles_in_degrees: value
            .get("angles_in_degrees")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        scroll_to_cycle_dropdowns: value
            .get("scroll_to_cycle_dropdowns")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        confirm_container_overwrite: value
            .get("confirm_container_overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        confirm_runtime_poke: value
            .get("confirm_runtime_poke")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        enable_chimp: value
            .get("enable_chimp")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        chimp_output_dir: value
            .get("chimp_output_dir")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from),
        chimp_usmap_path: value
            .get("chimp_usmap_path")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from),
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
        scroll_speed: value
            .get("scroll_speed")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(DEFAULT_SCROLL_SPEED)
            .clamp(MIN_SCROLL_SPEED, MAX_SCROLL_SPEED),
        zoom_speed: value
            .get("zoom_speed")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(DEFAULT_ZOOM_SPEED)
            .clamp(MIN_ZOOM_SPEED, MAX_ZOOM_SPEED),
        model_preview_size: value
            .get("model_preview_size")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(DEFAULT_MODEL_PREVIEW_SIZE)
            .clamp(MIN_MODEL_PREVIEW_SIZE, MAX_MODEL_PREVIEW_SIZE),
        model_preview_perspective: value
            .get("model_preview_perspective")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        bitmap_preview_view: BitmapPreviewViewSettings {
            bg: value
                .get("bitmap_preview_background")
                .and_then(Value::as_str)
                .and_then(BitmapPreviewBg::from_str)
                .unwrap_or(BitmapPreviewBg::DarkGray),
            show_checkerboard: value
                .get("bitmap_preview_checkerboard")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            show_border: value
                .get("bitmap_preview_border")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        },
        blender_path: value
            .get("blender_path")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from),
        editing_kit_paths: HashMap::new(),
        ek_folder_aliases: load_ek_folder_aliases(&value),
        custom_editing_kit_profiles: load_unified_editing_kit_profiles(&value),
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
        unusable_kit_entries: UnusableKitEntries {
            profiles: entries_for_unsupported_games(
                value
                    .get("editing_kit_profiles")
                    .or_else(|| value.get("custom_editing_kit_profiles")),
            ),
            aliases: entries_for_unsupported_games(value.get("ek_folder_aliases")),
        },
    }
}

/// The entries of a profile or alias array whose `game` this build does not
/// support — missing, empty or unknown, such as one a newer Baboon added.
///
/// They used to be dropped while loading, so the next save deleted them for
/// good. They are kept as written instead, never offered as kits, and written
/// back after the usable ones. An entry naming a supported game is the
/// loader's to accept or reject, never kept here, so no entry is both.
fn entries_for_unsupported_games(entries: Option<&Value>) -> Vec<Value> {
    entries
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter(|entry| {
                    entry
                        .get("game")
                        .and_then(Value::as_str)
                        .and_then(|game| game_for_saved_id(game.trim()))
                        .is_none()
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
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
            .get(shortcut.game.as_str())
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        paths.insert(shortcut.game.as_str().to_owned(), PathBuf::from(path));
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
        let mut relative_paths: Vec<PathBuf> = Vec::new();
        for tag in kit
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
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
        let mut folder_paths: Vec<PathBuf> = Vec::new();
        for folder in kit
            .get("folders")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(path) = folder
                .as_str()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .and_then(|path| clean_favorite_relative_path(PathBuf::from(path)))
            else {
                continue;
            };
            if !folder_paths
                .iter()
                .any(|existing| same_recent_path(existing, &path))
            {
                folder_paths.push(path);
            }
        }
        if relative_paths.is_empty() && folder_paths.is_empty() {
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
            for path in folder_paths {
                if !existing
                    .folders
                    .iter()
                    .any(|current| same_recent_path(current, &path))
                {
                    existing.folders.push(path);
                }
            }
        } else {
            favorites.push(EditingKitFavorites {
                tags_root: root,
                tags: relative_paths,
                folders: folder_paths,
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

fn load_custom_color_swatches(value: &Value) -> Vec<Option<ColorPaletteSwatch>> {
    let Some(items) = value.get("custom_color_swatches").and_then(Value::as_array) else {
        return default_color_swatches();
    };
    let legacy = items.len() <= LEGACY_CUSTOM_COLOR_SWATCH_COUNT;
    let mut swatches = if legacy {
        default_color_swatches()
    } else {
        vec![None; CUSTOM_COLOR_SWATCH_COUNT]
    };
    let start = if legacy {
        CUSTOM_COLOR_SWATCH_COUNT - LEGACY_CUSTOM_COLOR_SWATCH_COUNT
    } else {
        0
    };
    for (index, item) in items.iter().take(CUSTOM_COLOR_SWATCH_COUNT).enumerate() {
        swatches[start + index] = if let Some(text) = item.as_str() {
            parse_pref_rgba(text).map(ColorPaletteSwatch::unnamed)
        } else {
            let rgba = item
                .get("rgba")
                .and_then(Value::as_str)
                .and_then(parse_pref_rgba);
            rgba.map(|rgba| {
                let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                ColorPaletteSwatch::named(rgba, name)
            })
        };
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
                        game: game_for_saved_id(game)?.as_str().to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn load_custom_editing_kit_profiles(value: &Value) -> Vec<CustomEditingKitProfile> {
    let Some(entries) = value
        .get("editing_kit_profiles")
        .or_else(|| value.get("custom_editing_kit_profiles"))
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
            .and_then(game_for_saved_id)
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
        // Read only for the kits that can use them, so a value left behind by
        // a changed engine never redirects a kit whose tools can't follow it.
        let folder = |key: &str| {
            entry
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|folder| !folder.is_empty() && game.tools_take_folder_arguments())
                .map(PathBuf::from)
                .map(clean_recent_path)
        };
        let tags_folder = folder("tags_folder");
        let data_folder = folder("data_folder");
        profiles.push(CustomEditingKitProfile {
            read_only: !game.is_campaign_evolved()
                && entry
                    .get("read_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            git_tracked: entry
                .get("git_tracked")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            id: id.to_owned(),
            name: name.to_owned(),
            game: game.as_str().to_owned(),
            root,
            icon,
            tags_folder,
            data_folder,
        });
    }
    profiles
}

fn custom_editing_kit_profiles_value(profiles: &[CustomEditingKitProfile]) -> Vec<Value> {
    profiles
        .iter()
        .map(|profile| {
            let mut value = json!({
                "id": profile.id,
                "read_only": profile.read_only,
                "git_tracked": profile.git_tracked,
                "name": profile.name,
                "game": profile.game,
                "root": profile.root.display().to_string(),
                "icon": profile.icon.as_ref().map(|path| path.display().to_string()),
            });
            // Written only when set, so a kit using its root's own folders
            // saves exactly as it did before these existed.
            for (key, folder) in [
                ("tags_folder", &profile.tags_folder),
                ("data_folder", &profile.data_folder),
            ] {
                if let Some(folder) = folder {
                    value[key] = Value::String(folder.display().to_string());
                }
            }
            value
        })
        .collect()
}

fn load_unified_editing_kit_profiles(value: &Value) -> Vec<CustomEditingKitProfile> {
    let mut profiles = load_custom_editing_kit_profiles(value);
    // The new field is authoritative: never resurrect removed legacy entries.
    if value.get("editing_kit_profiles").is_none() {
        add_standard_editing_kit_profiles(&mut profiles, &load_editing_kit_paths(value));
    }
    profiles
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
    let value = prefs_to_value(prefs, terminal_open_games, first_run_complete);
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Could not encode preferences: {error}"))?;
    write_text_atomic(&path, &text, "preferences")
}

/// Encodes preferences as the JSON stored in `prefs.json`.
/// Split out from [`save_gui_prefs`] so the encoding can be exercised against
/// [`prefs_from_value`] without touching the filesystem.
fn prefs_to_value(
    prefs: &GuiPrefs,
    terminal_open_games: &HashSet<String>,
    first_run_complete: bool,
) -> Value {
    let mut games: Vec<&String> = terminal_open_games.iter().collect();
    games.sort();
    let mut collapsed_tool_categories: Vec<&String> =
        prefs.tool_commands_collapsed_categories.iter().collect();
    collapsed_tool_categories.sort();
    let mut profiles = prefs.custom_editing_kit_profiles.clone();
    add_standard_editing_kit_profiles(&mut profiles, &prefs.editing_kit_paths);
    let mut value = json!({
        "browser_mode": browser_mode_str(prefs.browser_mode),
        "browser_sort": browser_sort_str(prefs.browser_sort),
        "nested_default": nested_default_str(prefs.nested_default),
        "show_browser_prefixes": prefs.show_browser_prefixes,
        "folders_before_tags": prefs.folders_before_tags,
        "double_click_to_open_tags": prefs.double_click_to_open_tags,
        "session_restore": prefs.session_restore.as_str(),
        "update_channel": prefs.update_channel.as_str(),
        "check_updates_on_startup": prefs.check_updates_on_startup,
        "show_block_sizes": prefs.show_block_sizes,
        "angles_in_degrees": prefs.angles_in_degrees,
        "scroll_to_cycle_dropdowns": prefs.scroll_to_cycle_dropdowns,
        "confirm_container_overwrite": prefs.confirm_container_overwrite,
        "confirm_runtime_poke": prefs.confirm_runtime_poke,
        "enable_chimp": prefs.enable_chimp,
        "chimp_output_dir": prefs.chimp_output_dir.as_ref().map(|path| path.display().to_string()),
        "chimp_usmap_path": prefs.chimp_usmap_path.as_ref().map(|path| path.display().to_string()),
        "expert_mode": prefs.expert_mode,
        "dark_mode": prefs.dark_mode,
        "ui_scale": prefs.ui_scale,
        "scroll_speed": prefs.scroll_speed,
        "zoom_speed": prefs.zoom_speed,
        "model_preview_size": prefs.model_preview_size,
        "model_preview_perspective": prefs.model_preview_perspective,
        "bitmap_preview_background": prefs.bitmap_preview_view.bg.as_str(),
        "bitmap_preview_checkerboard": prefs.bitmap_preview_view.show_checkerboard,
        "bitmap_preview_border": prefs.bitmap_preview_view.show_border,
        "blender_path": prefs.blender_path.as_ref().map(|path| path.display().to_string()),
        "ek_folder_aliases": prefs.ek_folder_aliases.iter().map(|alias| {
            json!({
                "folder_name": alias.folder_name,
                "game": alias.game,
            })
        }).chain(prefs.unusable_kit_entries.aliases.iter().cloned()).collect::<Vec<_>>(),
        "editing_kit_profiles": custom_editing_kit_profiles_value(&profiles)
            .into_iter()
            .chain(prefs.unusable_kit_entries.profiles.iter().cloned())
            .collect::<Vec<_>>(),
        "tool_commands_window_pos": prefs.tool_commands_window_pos.map(|pos| vec![pos.x, pos.y]),
        "tool_commands_window_size": prefs.tool_commands_window_size.map(|size| vec![size.x, size.y]),
        "tool_commands_left_width": prefs.tool_commands_left_width,
        "tool_commands_collapsed_categories": collapsed_tool_categories,
        "recent_folders": prefs.recent_folders.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
        "editing_kit_favorites": prefs.editing_kit_favorites.iter().filter(|kit| !kit.tags.is_empty() || !kit.folders.is_empty()).map(|kit| {
            json!({
                "tags_root": kit.tags_root.display().to_string(),
                "tags": kit.tags.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
                "folders": kit.folders.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "custom_color_swatches": prefs.custom_color_swatches.iter().map(|swatch| {
            swatch.as_ref().map(|swatch| {
                let rgba = swatch.rgba;
                let encoded = format!("#{:02X}{:02X}{:02X}{:02X}", rgba[0], rgba[1], rgba[2], rgba[3]);
                match swatch.name.as_deref() {
                    Some(name) => json!({ "rgba": encoded, "name": name }),
                    None => Value::String(encoded),
                }
            })
        }).collect::<Vec<_>>(),
        "palette_last_dir": prefs.palette_last_dir.as_ref().map(|path| path.display().to_string()),
        "storage_mode": crate::core::storage::active_mode().map(crate::core::storage::StorageMode::as_str),
        "first_run_complete": first_run_complete,
        "terminal_open_games": games,
    });
    value["browser_search_scope"] = json!({
        "tags": prefs.browser_search_scope.tags,
        "folders": prefs.browser_search_scope.folders,
        "keywords": prefs.browser_search_scope.keywords,
    });
    value
}

/// Replace `path` with `text` so a crash leaves either the old file or the
/// new one. This used to remove the old file before renaming the new one in,
/// so a crash between the two left no file at all.
pub(in crate::app) fn write_text_atomic(path: &Path, text: &str, what: &str) -> Result<(), String> {
    use std::io::Write as _;
    let mut file = atomic_write_file::AtomicWriteFile::open(path)
        .map_err(|error| format!("Could not save {what}: {error}"))?;
    file.write_all(text.as_bytes())
        .map_err(|error| format!("Could not save {what}: {error}"))?;
    file.commit()
        .map_err(|error| format!("Could not install {what}: {error}"))
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
pub(in crate::app) fn browser_mode_from_str(text: Option<&str>) -> Option<BrowserMode> {
    match text? {
        "folders" => Some(BrowserMode::Folders),
        "groups" => Some(BrowserMode::Groups),
        _ => None,
    }
}

pub(in crate::app) fn browser_mode_str(mode: BrowserMode) -> &'static str {
    match mode {
        BrowserMode::Folders => "folders",
        BrowserMode::Groups => "groups",
    }
}

pub(in crate::app) fn browser_sort_from_str(text: Option<&str>) -> Option<BrowserSort> {
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

pub(in crate::app) fn browser_sort_str(sort: BrowserSort) -> &'static str {
    match sort {
        BrowserSort::Natural => "natural",
        BrowserSort::Name => "name",
        BrowserSort::Type => "type",
    }
}






impl Baboon {
    pub(in crate::app) fn current_prefs(&self) -> GuiPrefs {
        GuiPrefs {
            // The focused workspace's view is what a new one is seeded with,
            // so a single-workspace session remembers its choice as before.
            browser_mode: self.views[self.model.kits[self.model.active].id].browser.mode,
            browser_sort: self.views[self.model.kits[self.model.active].id].browser.sort,
            ..self.model.prefs.clone()
        }
    }

    pub(in crate::app) fn persist_prefs_if_changed(&mut self) {
        let _ = self.try_persist_prefs();
    }

    /// Write prefs if they changed; whether a write failed.
    pub(in crate::app) fn try_persist_prefs(&mut self) -> bool {
        let prefs = self.current_prefs();
        if prefs == self.saved_prefs && self.kit_tools.terminal_open_games == self.kit_tools.saved_terminal_open_games {
            return false;
        }
        match save_gui_prefs(&prefs, &self.kit_tools.terminal_open_games, true) {
            Ok(()) => {
                self.saved_prefs = prefs;
                self.kit_tools.saved_terminal_open_games = self.kit_tools.terminal_open_games.clone();
                false
            }
            Err(error) => {
                self.model.status = error;
                true
            }
        }
    }

    /// The per-frame prefs check, at most once a second.
    ///
    /// It ran every frame: a full GuiPrefs rebuilt (recents, favorites, kit
    /// profiles and swatches cloned) just to compare, and while a window, the
    /// UI-scale slider or a splitter was being dragged the value changed every
    /// frame, so prefs.json was rewritten at the frame rate. A failed write was
    /// retried, and reported, every frame too. Explicit calls (settings, runtime
    /// poke) still write at once, and exit flushes whatever is pending.
    pub(in crate::app) fn persist_prefs_throttled(&mut self, now: f64) {
        const CHECK_INTERVAL: f64 = 1.0;
        const RETRY_AFTER_FAILURE: f64 = 10.0;
        if now < self.shell.prefs_next_check_at {
            return;
        }
        let failed = self.try_persist_prefs();
        self.shell.prefs_next_check_at = now
            + if failed {
                RETRY_AFTER_FAILURE
            } else {
                CHECK_INTERVAL
            };
    }
}

pub(in crate::app) mod state;
pub(in crate::app) use state::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::browser::BrowserAction;
    use crate::core::test_kits::{compat_json, compat_samples};
    use std::path::PathBuf;

    #[test]
    fn legacy_custom_color_swatches_migrate_to_last_row() {
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
        assert_eq!(
            swatches[0],
            Some(ColorPaletteSwatch::unnamed([255, 0, 0, 255]))
        );
        assert_eq!(
            swatches[48],
            Some(ColorPaletteSwatch::unnamed([255, 0, 0, 255]))
        );
        assert_eq!(swatches[49], None);
        assert_eq!(
            swatches[50],
            Some(ColorPaletteSwatch::unnamed([51, 102, 153, 128]))
        );
        assert_eq!(swatches[51], None);
    }

    #[test]
    fn named_color_swatches_load_from_preferences() {
        let value = serde_json::json!({
            "custom_color_swatches": [
                { "rgba": "#FF0000FF", "name": "Red" }
            ]
        });

        let swatches = load_custom_color_swatches(&value);
        assert_eq!(
            swatches[48],
            Some(ColorPaletteSwatch::named([255, 0, 0, 255], "Red"))
        );
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
                    "read_only": true,
                    "git_tracked": true,
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
        assert!(profiles[0].read_only);
        assert!(profiles[0].git_tracked);
        assert!(!profiles[1].read_only, "old entries must remain writable");
        assert!(!profiles[1].git_tracked, "old entries must not enable Git");
        assert_eq!(
            profiles
                .iter()
                .map(|profile| profile.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Reach Project", "Second Project"]
        );
        assert_eq!(
            profiles[0].icon.as_deref(),
            Some(Path::new("editing kit icons/reach-11111111/icon-a.png"))
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
                    ],
                    "folders": [
                        "objects/characters/brute",
                        "objects/characters/brute",
                        "../outside"
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
        assert_eq!(
            favorites[0].folders,
            vec![PathBuf::from("objects/characters/brute")]
        );
        assert_eq!(favorites[1].tags_root, PathBuf::from("C:/Games/H3EK/tags"));
        assert_eq!(
            favorites[1].tags,
            vec![PathBuf::from("objects/brute.model")]
        );
        assert!(favorites[1].folders.is_empty());
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

    #[test]
    fn browser_search_scope_defaults_to_tags_and_round_trips_every_selection() {
        let tags_only = BrowserSearchScope::default();
        assert!(tags_only.tags && !tags_only.folders && !tags_only.keywords);
        assert_eq!(prefs_from_value(&json!({})).browser_search_scope, tags_only);
        for selection in 1..8 {
            let scope = BrowserSearchScope {
                tags: selection & 1 != 0,
                folders: selection & 2 != 0,
                keywords: selection & 4 != 0,
            };
            let prefs = GuiPrefs {
                browser_search_scope: scope,
                ..GuiPrefs::default()
            };
            let saved = prefs_to_value(&prefs, &HashSet::new(), false);
            assert_eq!(prefs_from_value(&saved).browser_search_scope, scope);
        }
        assert_eq!(
            prefs_from_value(&json!({"browser_search_scope": {
                "tags": false, "folders": false, "keywords": false
            }}))
            .browser_search_scope,
            tags_only
        );
    }

    // Every saved format Baboon reads, fed through the real readers from the
    // synthetic samples in `testdata/compat` (see its README; regenerate with
    // `gen_samples.py`). Old files must keep loading, files a newer build wrote
    // must not be destroyed by this one, and the cases a reader refuses are
    // pinned beside the ones it accepts, so a reader that accepted everything
    // would fail here too.

    #[test]
    fn compat_prefs() {
        let prefs = prefs_from_value(&compat_json("prefs/prefs.current.json"));
        assert_eq!(prefs.custom_editing_kit_profiles.len(), 10);
        let games: Vec<&str> = prefs
            .custom_editing_kit_profiles
            .iter()
            .map(|profile| profile.game.as_str())
            .collect();
        for game in [
            "haloce_mcc",
            "halo2_mcc",
            "halo2amp_mcc",
            "halo3_mcc",
            "halo3odst_mcc",
            "haloreach_mcc",
            "halo4_mcc",
            "haloce_evolved",
        ] {
            assert!(games.contains(&game), "{game}");
        }
        let moda = prefs
            .custom_editing_kit_profiles
            .iter()
            .find(|profile| profile.name == "H2 (moda tags)")
            .unwrap();
        assert!(moda.read_only && moda.git_tracked);
        // A backslash icon path is one component on Unix, so it is not under the
        // icon folder there and is dropped; Windows keeps it.
        #[cfg(not(windows))]
        assert!(moda.icon.is_none());
        assert!(moda.tags_folder.is_some());
        let reach = prefs
            .custom_editing_kit_profiles
            .iter()
            .find(|profile| profile.name == "Reach ignored folder")
            .unwrap();
        assert!(
            reach.tags_folder.is_none(),
            "tags_folder is ignored for a game whose folders are not choosable"
        );
        assert_eq!(prefs.ek_folder_aliases.len(), 2);
        assert_eq!(prefs.editing_kit_favorites.len(), 2);

        let encoded = prefs_to_value(&prefs, &HashSet::from(["halo3_mcc".to_owned()]), true);
        let decoded = prefs_from_value(&encoded);
        let ids = |prefs: &GuiPrefs| {
            prefs
                .custom_editing_kit_profiles
                .iter()
                .map(|profile| (profile.id.clone(), profile.game.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&prefs), ids(&decoded));

        // Kits for games this build does not support are not offered, but they
        // are written back as they were read (63b257d). Before that, the next
        // save of any preference deleted them.
        let stored = compat_json("prefs/prefs.unknown_game.json");
        let unknown = prefs_from_value(&stored);
        assert_eq!(
            ids(&unknown),
            [(
                "bab00000-0000-4000-8000-000000000002".to_owned(),
                "halo3_mcc".to_owned()
            )]
        );
        assert!(unknown.ek_folder_aliases.is_empty());
        let resaved = prefs_to_value(&unknown, &HashSet::new(), true);
        let profiles = resaved["editing_kit_profiles"].as_array().unwrap();
        assert_eq!(profiles.len(), 3, "{profiles:#?}");
        for kept in [
            &stored["editing_kit_profiles"][0],
            &stored["editing_kit_profiles"][2],
        ] {
            assert!(profiles.contains(kept), "{kept} kept field for field");
        }
        assert_eq!(
            resaved["ek_folder_aliases"],
            stored["ek_folder_aliases"],
            "the alias for an unknown game is kept"
        );
        assert!(prefs_from_value(&resaved) == unknown, "stable once written");

        // Before unified profiles: `editing_kit_paths`, one standard kit per game.
        let legacy = prefs_from_value(&compat_json("prefs/prefs.legacy_editing_kit_paths.json"));
        assert_eq!(legacy.custom_editing_kit_profiles.len(), 8);
        assert!(
            legacy
                .custom_editing_kit_profiles
                .iter()
                .all(|profile| profile.id.starts_with("bab00000-0000-4000-8000-00000000000"))
        );
        assert_eq!(legacy.session_restore, SessionRestore::Always);
        assert_eq!(
            legacy.custom_color_swatches.len(),
            CUSTOM_COLOR_SWATCH_COUNT
        );

        // The older `custom_editing_kit_profiles` key, a mixed-case game id.
        let old = prefs_from_value(&compat_json("prefs/prefs.legacy_custom_profiles.json"));
        assert!(
            old.custom_editing_kit_profiles
                .iter()
                .any(|profile| profile.game == "halo3_mcc" && profile.name == "Old custom")
        );
        assert!(
            old.custom_editing_kit_profiles
                .iter()
                .any(|profile| profile.game == "haloreach_mcc")
        );

        // A file cut short is not a first run.
        let text = std::fs::read_to_string(compat_samples().join("prefs/prefs.malformed.json")).unwrap();
        assert!(first_run_complete_from_text(Some(&text)));
        assert!(!first_run_complete_from_text(None));
    }

    #[test]
    fn no_preferences_starts_first_run() {
        assert!(!first_run_complete_from_text(None));
    }

    #[test]
    fn existing_preferences_without_marker_are_complete() {
        assert!(first_run_complete_from_text(Some("{}")));
    }

    #[test]
    fn explicit_false_resumes_and_true_finishes_setup() {
        assert!(!first_run_complete_from_text(Some(
            r#"{"first_run_complete":false}"#
        )));
        assert!(first_run_complete_from_text(Some(
            r#"{"first_run_complete":true}"#
        )));
    }

    #[test]
    fn malformed_existing_preferences_still_skip_first_run() {
        assert!(first_run_complete_from_text(Some("not json")));
    }

    fn stored(prefs: &GuiPrefs) -> GuiPrefs {
        prefs_from_value(&prefs_to_value(prefs, &HashSet::new(), true))
    }

    #[test]
    fn the_stable_channel_is_the_default() {
        let prefs = prefs_from_value(&json!({}));
        assert_eq!(prefs.update_channel, UpdateChannel::Stable);
        assert!(prefs.check_updates_on_startup);
    }

    #[test]
    fn preferences_written_before_this_setting_existed_load_as_stable() {
        // No `update_channel` key, but plenty of other settings: an existing user's
        // file must not silently move them onto development builds.
        let prefs = prefs_from_value(&json!({
            "session_restore": "always",
            "dark_mode": true,
        }));
        assert_eq!(prefs.update_channel, UpdateChannel::Stable);
        assert!(prefs.check_updates_on_startup);
        assert_eq!(prefs.session_restore, SessionRestore::Always);
    }

    #[test]
    fn the_chosen_channel_survives_a_save_and_load() {
        let mut prefs = GuiPrefs::default();
        prefs.update_channel = UpdateChannel::Development;
        prefs.check_updates_on_startup = false;

        let reloaded = stored(&prefs);
        assert_eq!(reloaded.update_channel, UpdateChannel::Development);
        assert!(!reloaded.check_updates_on_startup);

        prefs.update_channel = UpdateChannel::Stable;
        prefs.check_updates_on_startup = true;
        let reloaded = stored(&prefs);
        assert_eq!(reloaded.update_channel, UpdateChannel::Stable);
        assert!(reloaded.check_updates_on_startup);
    }

    #[test]
    fn an_unrecognised_channel_falls_back_to_stable() {
        let prefs = prefs_from_value(&json!({"update_channel": "nightly"}));
        assert_eq!(prefs.update_channel, UpdateChannel::Stable);
    }

    #[test]
    fn old_preferences_keep_the_bitmap_view_defaults() {
        let prefs = prefs_from_value(&json!({}));
        assert_eq!(
            prefs.bitmap_preview_view,
            BitmapPreviewViewSettings::default()
        );
    }

    #[test]
    fn bitmap_view_choices_survive_a_save_and_load() {
        let mut prefs = GuiPrefs::default();
        prefs.bitmap_preview_view = BitmapPreviewViewSettings {
            bg: BitmapPreviewBg::Magenta,
            show_checkerboard: false,
            show_border: false,
        };

        assert_eq!(
            stored(&prefs).bitmap_preview_view,
            prefs.bitmap_preview_view
        );
    }

    #[test]
    fn an_unknown_bitmap_background_uses_the_default() {
        let prefs = prefs_from_value(&json!({
            "bitmap_preview_background": "chartreuse",
            "bitmap_preview_checkerboard": false,
            "bitmap_preview_border": false,
        }));

        assert_eq!(prefs.bitmap_preview_view.bg, BitmapPreviewBg::DarkGray);
        assert!(!prefs.bitmap_preview_view.show_checkerboard);
        assert!(!prefs.bitmap_preview_view.show_border);
    }

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

    #[test]
    fn editing_kit_migration_is_stable_and_saves_only_unified_profiles() {
        let legacy = json!({
            "editing_kit_paths": {
                "halo2_mcc": "Z:/Unavailable/H2EK",
                "haloce_evolved": "Z:/Unavailable/Halo Campaign Evolved"
            },
            "custom_editing_kit_profiles": [{
                "id": "11111111-1111-4111-8111-111111111111",
                "name": "My mod", "game": "halo3_mcc", "root": "Z:/Unavailable/Mod",
                "icon": "editing kit icons/mod/icon.png"
            }]
        });
        let prefs = prefs_from_value(&legacy);
        assert!(prefs.editing_kit_paths.is_empty());
        assert_eq!(prefs.custom_editing_kit_profiles.len(), 3);
        assert_eq!(
            prefs.custom_editing_kit_profiles,
            prefs_from_value(&legacy).custom_editing_kit_profiles
        );
        assert_eq!(prefs.custom_editing_kit_profiles[0].name, "My mod");
        assert!(prefs.custom_editing_kit_profiles[0].icon.is_some());
        let saved = prefs_to_value(&prefs, &HashSet::new(), true);
        assert!(saved.get("editing_kit_paths").is_none());
        assert!(saved.get("custom_editing_kit_profiles").is_none());
        assert_eq!(
            prefs.custom_editing_kit_profiles,
            prefs_from_value(&saved).custom_editing_kit_profiles
        );
        let mut removed = saved;
        removed["editing_kit_profiles"] = json!([]);
        removed["editing_kit_paths"] = legacy["editing_kit_paths"].clone();
        assert!(
            prefs_from_value(&removed)
                .custom_editing_kit_profiles
                .is_empty()
        );
    }

    #[test]
    fn editing_kit_detection_profiles_deduplicate_roots_without_changing_edits() {
        let paths = HashMap::from([("halo2_mcc".to_owned(), PathBuf::from("Z:/Unavailable/H2EK"))]);
        let mut profiles = Vec::new();
        assert_eq!(add_standard_editing_kit_profiles(&mut profiles, &paths), 1);
        profiles[0].name = "Renamed kit".to_owned();
        profiles[0].icon = Some(PathBuf::from("editing kit icons/mod/icon.png"));
        let previous = profiles.clone();
        assert_eq!(add_standard_editing_kit_profiles(&mut profiles, &paths), 0);
        assert_eq!(profiles, previous);
        let legacy = json!({
            "editing_kit_paths": { "halo2_mcc": "Z:/Unavailable/H2EK" },
            "custom_editing_kit_profiles": custom_editing_kit_profiles_value(&profiles)
        });
        assert_eq!(
            prefs_from_value(&legacy).custom_editing_kit_profiles,
            previous
        );
    }

    /// A profile or folder alias naming a game this build does not know (one
    /// a newer Baboon supports, or none) used to be dropped on load, and so
    /// deleted from the file by the next save of any preference.
    #[test]
    fn kits_for_unsupported_games_survive_a_save_but_are_not_offered() {
        let usable = json!({
            "id": "6f1c3f9e-1d1b-4c2a-9a55-0d6f1d1b2c3a",
            "name": "Halo 3",
            "game": "halo3_mcc",
            "root": "/kits/h3",
        });
        let future = json!({
            "id": "0b5e2a7c-3a43-4f37-8f2e-6a9d2c1b4e5f",
            "name": "Halo Infinite",
            "game": "haloinfinite",
            "root": "/kits/hi",
            "some_newer_setting": [1, 2, 3],
        });
        let gameless = json!({
            "id": "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f",
            "name": "No game",
            "game": "",
            "root": "/kits/none",
        });
        let future_alias = json!({ "folder_name": "HIEK", "game": "haloinfinite" });
        let usable_alias = json!({ "folder_name": "H3EK", "game": "halo3_mcc" });
        let stored = json!({
            "editing_kit_profiles": [usable, future, gameless],
            "ek_folder_aliases": [usable_alias, future_alias],
        });

        let prefs = prefs_from_value(&stored);
        let offered: Vec<&str> = prefs
            .custom_editing_kit_profiles
            .iter()
            .map(|profile| profile.game.as_str())
            .collect();
        assert_eq!(offered, ["halo3_mcc"], "only a supported game is a kit");
        assert_eq!(prefs.ek_folder_aliases.len(), 1);

        let written = prefs_to_value(&prefs, &HashSet::new(), true);
        let profiles = written["editing_kit_profiles"].as_array().unwrap();
        assert!(profiles.contains(&future), "{profiles:#?}");
        assert!(profiles.contains(&gameless), "{profiles:#?}");
        assert_eq!(profiles.len(), 3);
        let aliases = written["ek_folder_aliases"].as_array().unwrap();
        assert!(aliases.contains(&future_alias), "{aliases:#?}");
        assert_eq!(aliases.len(), 2);

        // Written back and read again, nothing changes.
        assert!(prefs_from_value(&written) == prefs);
    }

    #[test]
    fn chimp_is_enabled_for_preferences_written_before_it_existed() {
        let prefs = prefs_from_value(&json!({}));
        assert!(prefs.enable_chimp);
        assert_eq!(prefs.chimp_output_dir, None);
        assert_eq!(prefs.chimp_usmap_path, None);
    }

    #[test]
    fn chimp_visibility_and_output_directory_round_trip() {
        let prefs = GuiPrefs {
            enable_chimp: false,
            chimp_output_dir: Some(PathBuf::from("D:/Mods/Chimp")),
            chimp_usmap_path: Some(PathBuf::from("D:/Mappings/Meteorite.usmap")),
            ..GuiPrefs::default()
        };
        let value = prefs_to_value(&prefs, &HashSet::new(), true);
        let restored = prefs_from_value(&value);
        assert!(!restored.enable_chimp);
        assert_eq!(
            restored.chimp_output_dir,
            Some(PathBuf::from("D:/Mods/Chimp"))
        );
        assert_eq!(
            restored.chimp_usmap_path,
            Some(PathBuf::from("D:/Mappings/Meteorite.usmap"))
        );
    }

    // Preferences are held once, as the `GuiPrefs` that was loaded, so what is
    // written back is what was read plus what the user changed — with no
    // mirrored field for a new preference to be forgotten in.

    fn app_with(prefs: GuiPrefs) -> Baboon {
        Baboon::assemble(
            &egui::Context::default(),
            crate::app::shell::window_state::WindowStateTracker::for_test(),
            prefs,
            HashSet::new(),
            None,
            TagNameIndex::default(),
            None,
        )
    }

    /// Loaded, then written back untouched: every field survives. Most are set
    /// away from their defaults, so a writer that rebuilt the struct from
    /// defaults, or dropped a field on the way through, would differ.
    #[test]
    fn loaded_prefs_are_written_back_unchanged() {
        let prefs = GuiPrefs {
            browser_mode: BrowserMode::Groups,
            browser_search_scope: BrowserSearchScope {
                tags: false,
                folders: true,
                keywords: true,
            },
            show_browser_prefixes: true,
            folders_before_tags: true,
            double_click_to_open_tags: true,
            check_updates_on_startup: false,
            show_block_sizes: true,
            angles_in_degrees: false,
            scroll_to_cycle_dropdowns: false,
            confirm_container_overwrite: false,
            confirm_runtime_poke: false,
            enable_chimp: false,
            chimp_output_dir: Some(PathBuf::from("/out")),
            chimp_usmap_path: Some(PathBuf::from("/mappings.usmap")),
            expert_mode: true,
            dark_mode: true,
            ui_scale: 1.25,
            scroll_speed: 2.5,
            zoom_speed: 0.75,
            model_preview_size: 333.0,
            model_preview_perspective: false,
            blender_path: Some(PathBuf::from("/blender")),
            tool_commands_window_pos: Some(egui::pos2(10.0, 20.0)),
            tool_commands_window_size: Some(egui::vec2(700.0, 500.0)),
            tool_commands_left_width: MIN_TOOL_COMMANDS_LEFT_WIDTH + 40.0,
            tool_commands_collapsed_categories: HashSet::from(["build".to_owned()]),
            recent_folders: vec![PathBuf::from("/recent")],
            custom_color_swatches: vec![Some(ColorPaletteSwatch::named([1, 2, 3, 4], "Sample"))],
            palette_last_dir: Some(PathBuf::from("/palettes")),
            ..GuiPrefs::default()
        };
        let app = app_with(prefs.clone());
        assert!(
            app.current_prefs() == prefs,
            "a loaded preference did not survive"
        );
    }

    /// A value out of range in the file is brought into range once, and that is
    /// what is written back.
    #[test]
    fn out_of_range_prefs_are_corrected_when_loaded() {
        let app = app_with(GuiPrefs {
            tool_commands_window_size: None,
            tool_commands_left_width: 0.0,
            ..GuiPrefs::default()
        });
        let written = app.current_prefs();
        assert_eq!(
            written.tool_commands_window_size,
            Some(DEFAULT_TOOL_COMMANDS_WINDOW_SIZE)
        );
        assert_eq!(
            written.tool_commands_left_width,
            MIN_TOOL_COMMANDS_LEFT_WIDTH
        );
        assert!(
            written != app.saved_prefs,
            "the correction is written back once"
        );
    }

    /// A change made through the live prefs is what gets written.
    #[test]
    fn a_changed_pref_is_what_gets_written() {
        let mut app = app_with(GuiPrefs::default());
        app.model.prefs.expert_mode = true;
        app.model.prefs.browser_search_scope = BrowserSearchScope {
            tags: false,
            folders: false,
            keywords: true,
        };
        app.views[app.model.kits[0].id].browser.mode = BrowserMode::Groups;
        let written = app.current_prefs();
        assert!(written.expert_mode);
        assert_eq!(written.browser_search_scope, app.model.prefs.browser_search_scope);
        assert_eq!(
            written.browser_mode,
            BrowserMode::Groups,
            "the focused kit's view"
        );
    }

    /// The saved search scope seeds the first workspace, a new one, and a folder
    /// browser opened in it.
    #[test]
    fn saved_search_scope_seeds_startup_and_new_workspaces() {
        let scope = BrowserSearchScope {
            tags: true,
            folders: false,
            keywords: true,
        };
        let mut app = app_with(GuiPrefs {
            browser_search_scope: scope,
            ..GuiPrefs::default()
        });
        assert_eq!(app.views[app.model.kits[0].id].browser.search_scope, scope);
        let kit = app.add_kit();
        assert_eq!(app.views[kit].browser.search_scope, scope);
        app.model.active = app.model.kit_index(kit).expect("the new kit");
        app.handle_browser_action(
            BrowserAction::OpenFolderBrowser {
                rel_path: PathBuf::from("objects"),
                label: "objects".into(),
                open_in_new_tab: true,
            },
            egui::Context::default(),
        );
        let pane = app.views[kit].browser.folder_browsers.values().next().expect("a folder browser");
        assert_eq!(pane.search_scope, scope);
    }

    /// The per-frame prefs check runs at most once a second. It ran every
    /// frame, and wrote prefs.json every frame while a slider was dragged.
    #[test]
    fn the_per_frame_prefs_check_runs_once_a_second() {
        // Unchanged prefs, so nothing is written: this only watches the clock.
        let mut app = Baboon::for_test();
        app.persist_prefs_throttled(10.0);
        assert_eq!(app.shell.prefs_next_check_at, 11.0);
        app.persist_prefs_throttled(10.5);
        assert_eq!(app.shell.prefs_next_check_at, 11.0, "inside the second: skipped");
        app.persist_prefs_throttled(11.2);
        assert_eq!(app.shell.prefs_next_check_at, 12.2);
    }

    /// A kit's chosen folders survive a save and load. A kit on its root's own
    /// folders saves no folder keys at all, exactly as before they existed,
    /// and an engine whose tools can't use them reads them as unset.
    #[test]
    fn chosen_kit_folders_round_trip_and_stay_out_of_other_kits() {
        let profile = |id: &str, game: &str, tags: Option<&str>, data: Option<&str>| {
            CustomEditingKitProfile {
                read_only: false,
                git_tracked: false,
                id: id.to_owned(),
                name: id.to_owned(),
                game: game.to_owned(),
                root: PathBuf::from("/kits/H2EK"),
                icon: None,
                tags_folder: tags.map(PathBuf::from),
                data_folder: data.map(PathBuf::from),
            }
        };
        let moda = profile(
            "00000000-0000-4000-8000-00000000000a",
            "halo2_mcc",
            Some("tags_moda"),
            Some("/elsewhere/data_moda"),
        );
        let stock = profile(
            "00000000-0000-4000-8000-00000000000b",
            "halo2_mcc",
            None,
            None,
        );
        let saved = custom_editing_kit_profiles_value(&[moda.clone(), stock.clone()]);
        assert!(saved[1].get("tags_folder").is_none() && saved[1].get("data_folder").is_none());
        let loaded = load_custom_editing_kit_profiles(&json!({ "editing_kit_profiles": saved }));
        assert_eq!(loaded, vec![moda, stock]);

        let mut halo3 = custom_editing_kit_profiles_value(&[profile(
            "00000000-0000-4000-8000-00000000000c",
            "halo3_mcc",
            None,
            None,
        )]);
        halo3[0]["tags_folder"] = json!("tags_moda");
        let loaded = load_custom_editing_kit_profiles(&json!({ "editing_kit_profiles": halo3 }));
        assert_eq!(loaded[0].tags_folder, None);
    }

}
