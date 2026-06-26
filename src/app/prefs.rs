use super::*;

pub(super) fn prefs_path() -> PathBuf {
    app_prefs_path("Baboon", "baboon", ".baboon-prefs.json")
}

fn legacy_prefs_path() -> PathBuf {
    app_prefs_path("Genesis", "genesis", ".genesis-prefs.json")
}

fn app_prefs_path(windows_folder: &str, unix_folder: &str, fallback_name: &str) -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata)
            .join(windows_folder)
            .join("prefs.json");
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(home)
            .join(".config")
            .join(unix_folder)
            .join("prefs.json");
    }
    PathBuf::from(fallback_name)
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
    GuiPrefs {
        browser_mode,
        show_browser_prefixes: value
            .get("show_browser_prefixes")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        double_click_to_open_tags: value
            .get("double_click_to_open_tags")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        show_block_sizes: value
            .get("show_block_sizes")
            .and_then(Value::as_bool)
            .unwrap_or(false),
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
        ek_folder_aliases: load_ek_folder_aliases(&value),
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
    let value = json!({
        "browser_mode": match prefs.browser_mode {
            BrowserMode::Folders => "folders",
            BrowserMode::Groups => "groups",
        },
        "show_browser_prefixes": prefs.show_browser_prefixes,
        "double_click_to_open_tags": prefs.double_click_to_open_tags,
        "show_block_sizes": prefs.show_block_sizes,
        "expert_mode": prefs.expert_mode,
        "dark_mode": prefs.dark_mode,
        "ui_scale": prefs.ui_scale,
        "model_preview_size": prefs.model_preview_size,
        "blender_path": prefs.blender_path.as_ref().map(|path| path.display().to_string()),
        "ek_folder_aliases": prefs.ek_folder_aliases.iter().map(|alias| {
            json!({
                "folder_name": alias.folder_name,
                "game": alias.game,
            })
        }).collect::<Vec<_>>(),
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
