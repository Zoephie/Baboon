//! Preferences and last-session persistence, including legacy migration.
//! It owns preference/session serialization and migration; interactive settings presentation belongs to the UI layer.

use crate::app::kits::detect::add_standard_editing_kit_profiles;
use super::*;
use crate::app::browser::{BrowserMode, BrowserSort};

pub(super) fn prefs_path() -> PathBuf {
    crate::core::storage::data_path("prefs.json")
}

pub(super) fn last_session_path() -> PathBuf {
    crate::core::storage::data_path("last_session.json")
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

#[cfg(test)]
mod first_run_tests;

#[cfg(test)]
mod update_channel_tests;

#[cfg(test)]
mod bitmap_preview_tests;

#[cfg(test)]
mod compat_fixtures_tests;

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
    let browser_mode = browser_mode_from_str(value.get("browser_mode").and_then(Value::as_str))
        .unwrap_or_default();
    let browser_sort = browser_sort_from_str(value.get("browser_sort").and_then(Value::as_str))
        .unwrap_or_default();
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
    json!({
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
    })
}

/// Replace `path` with `text` so a crash leaves either the old file or the
/// new one. This used to remove the old file before renaming the new one in,
/// so a crash between the two left no file at all.
fn write_text_atomic(path: &Path, text: &str, what: &str) -> Result<(), String> {
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
        // Versions 3 to 6 are that same per-kit object, once per open kit.
        // Version 4 adds optional Chimp package tabs, version 5 the Bitmap
        // Library flag, and version 6 folder panes. Each field is optional on
        // the way in, so older files still load and only lack what they never
        // recorded.
        3 | 4 | 5 | 6 => value
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
    // Absent in sessions written while `project_path` still meant "the file this
    // workspace autosaves to", which was set for every workspace that had a
    // project at all — so its presence is exactly what this flag now records.
    let has_project = source
        .get("has_project")
        .and_then(Value::as_bool)
        .unwrap_or(project_path.is_some());
    // Absent in every session written before the focused workspace was
    // recorded, which reads back as "no kit was active" and leaves the restore
    // picking whichever kit it used to.
    let was_active = value
        .get("active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // Absent in sessions written before the browser view became per-kit, and
    // in every version-1 and version-2 file. `None` means "use the default".
    let browser_mode = browser_mode_from_str(value.get("browser_mode").and_then(Value::as_str));
    let browser_sort = browser_sort_from_str(value.get("browser_sort").and_then(Value::as_str));
    let mut tags = Vec::new();
    for item in value
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
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
    let folders = value
        .get("folders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let rel_path = item
                .get("path")?
                .as_str()
                .map(str::trim)
                .filter(|path| !path.is_empty())?;
            let label = item
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .or_else(|| rel_path.rsplit(['/', '\\']).next())?;
            Some(LastSessionFolder {
                rel_path: PathBuf::from(rel_path),
                label: label.to_owned(),
            })
        })
        .collect::<Vec<_>>();
    let chimp_packages = value
        .get("chimp_packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|package| !package.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let active_chimp_package = value
        .get("active_chimp_package")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|package| chimp_packages.iter().any(|open| open == package))
        .map(str::to_owned);
    // Absent in every session written before the Bitmap Library existed, which
    // reads back as "it was not open" — the right answer for those files.
    let bitmap_library_open = value
        .get("bitmap_library")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let model_library_open = value
        .get("model_library")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // Keep source-only workspaces. The source path is meaningful session state
    // even when no tag window or project was open in that workspace.
    Some(LastSessionKit {
        source_kind,
        source_path,
        game,
        profile_id,
        project_path,
        has_project,
        browser_mode,
        browser_sort,
        tags,
        folders,
        chimp_packages,
        active_chimp_package,
        bitmap_library_open,
        model_library_open,
        was_active,
    })
}

/// Persist every kit's source and open tag/folder panes for the launch-time restore
/// prompt, along with which of them was focused. Written from the confirmed
/// app-exit path and again as the event loop tears down, so a quit that never
/// asks the window to close — macOS Cmd+Q — still records the session; a crash
/// leaves the previous one intact.
pub(super) fn save_last_session(session: &LastSessionState) -> Result<(), String> {
    let path = last_session_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create session folder: {error}"))?;
    }
    let text = serde_json::to_string_pretty(&session_value(session))
        .map_err(|error| format!("Could not encode session: {error}"))?;
    // Atomic, as the doc above promises: this runs after every autosave, so a
    // plain write left a window for a crash to truncate the session to nothing.
    write_text_atomic(&path, &text, "session")
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
            let folders = kit
                .folders
                .iter()
                .map(|folder| {
                    json!({
                        "path": folder.rel_path.to_string_lossy().replace('\\', "/"),
                        "label": folder.label,
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
                    "has_project": kit.has_project,
                },
                "browser_mode": kit.browser_mode.map(browser_mode_str),
                "browser_sort": kit.browser_sort.map(browser_sort_str),
                "tags": tags,
                "folders": folders,
                "chimp_packages": kit.chimp_packages,
                "active_chimp_package": kit.active_chimp_package,
                "bitmap_library": kit.bitmap_library_open,
                "model_library": kit.model_library_open,
                // Which workspace the user was looking at. Written as a flag on
                // the kit rather than an index beside the list: the restore
                // prompt can drop kits, and an index would then point at
                // whichever one moved into that slot. Absent in sessions
                // written before this, which read back as "no kit was active"
                // and leave the restore picking as it used to.
                "active": kit.was_active,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "version": 6,
        "kits": kits,
    })
}

pub(super) fn clear_last_session() {
    let _ = fs::remove_file(last_session_path());
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod nested_default_tests;

#[cfg(test)]
mod chimp_pref_tests;

#[cfg(test)]
mod session_tests;

#[cfg(test)]
mod prefs_round_trip_tests;

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

#[cfg(test)]
mod prefs_throttle_tests;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;
