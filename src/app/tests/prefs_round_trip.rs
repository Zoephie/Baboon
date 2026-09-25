//! Preferences are held once, as the `GuiPrefs` that was loaded, so what is
//! written back is what was read plus what the user changed — with no
//! mirrored field for a new preference to be forgotten in.

use super::*;

fn app_with(prefs: GuiPrefs) -> Baboon {
    Baboon::assemble(
        &egui::Context::default(),
        crate::window_state::WindowStateTracker::for_test(),
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
        model_preview_size: 333.0,
        blender_path: Some(PathBuf::from("/blender")),
        tool_commands_window_pos: Some(egui::pos2(10.0, 20.0)),
        tool_commands_window_size: Some(egui::vec2(700.0, 500.0)),
        tool_commands_left_width: MIN_TOOL_COMMANDS_LEFT_WIDTH + 40.0,
        tool_commands_collapsed_categories: HashSet::from(["build".to_owned()]),
        recent_folders: vec![PathBuf::from("/recent")],
        custom_color_swatches: vec![Some([1, 2, 3, 4])],
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
    app.prefs.expert_mode = true;
    app.kits[0].browser_mode = BrowserMode::Groups;
    let written = app.current_prefs();
    assert!(written.expert_mode);
    assert_eq!(
        written.browser_mode,
        BrowserMode::Groups,
        "the focused kit's view"
    );
}
