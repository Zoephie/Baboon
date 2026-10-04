//! The application shell: the frame loop and the worker messages it applies,
//! the session saved and restored, update checks, and the windows and bars
//! around the features.

use super::*;

pub(in crate::app) mod updates;
pub(in crate::app) mod actions;
pub(in crate::app) use actions::AppAction;
pub(in crate::app) mod menus;
pub(in crate::app) use menus::draw_menu_bar;
pub(in crate::app) mod jobs;
pub(in crate::app) mod session;
pub(in crate::app) use session::state::*;
pub(in crate::app) mod frame;
pub(in crate::app) use frame::{draw_keyword_bar, draw_scenario_launcher_buttons};
pub(in crate::app) mod workspace;
pub(in crate::app) mod welcome;
pub(in crate::app) mod first_run;
pub(in crate::app) mod settings;
pub(in crate::app) mod kit_tiles;
pub(in crate::app) mod tag_tiles;
pub(in crate::app) mod loading;
pub(in crate::app) use loading::centered_loading_state;
pub(in crate::app) mod recents;
pub(in crate::app) mod operation_notice;
pub(in crate::app) use operation_notice::OperationNotice;
pub(in crate::app) mod worker;
pub(in crate::app) use worker::*;
pub(in crate::app) mod launch;
pub(in crate::app) use launch::{CommandLineLaunch, resolve_launch_tag_entries};
pub(crate) use launch::{StartupArguments, parse_startup_arguments};

/// The shell: settings and first run, update checks, the session being
/// restored, the operation notice, toolbar icons and game artwork, and when
/// prefs are next checked.
pub(in crate::app) struct ShellFeature {
    pub(in crate::app) settings_open: bool,
    pub(in crate::app) settings_tab: SettingsTab,
    pub(in crate::app) pending_ui_scale: f32,
    pub(in crate::app) first_run_wizard: Option<FirstRunWizardState>,
    /// The most recent check's result, kept only while it is actually an
    /// update. The status line expires on a timer, so this is what keeps the
    /// news reachable after a silent startup check.
    pub(in crate::app) available_update: Option<UpdateCheckResult>,
    /// The most recent successful check, update or not, so Settings can report
    /// the outcome after the status line has expired.
    pub(in crate::app) last_update_check: Option<UpdateCheckResult>,
    /// Startup-only prompt reconstructed from the prior session file.
    pub(in crate::app) last_opened_windows: Option<LastOpenedWindowsPrompt>,
    /// Kits whose session-restore load has not landed yet, and the one the
    /// session named as focused. Every load ends by making its own kit active,
    /// so the focus can only be honoured once none are outstanding.
    pub(in crate::app) restoring_kits: HashSet<KitId>,
    pub(in crate::app) restored_active_kit: Option<KitId>,
    /// Result of the last container write, shown until dismissed.
    pub(in crate::app) operation_notice: Option<OperationNotice>,
    /// Toolbar launcher icons (decoded from embedded .ico at startup).
    pub(in crate::app) blender_icon: Option<egui::TextureHandle>,
    pub(in crate::app) sapien_icon: Option<egui::TextureHandle>,
    pub(in crate::app) tag_test_icon: Option<egui::TextureHandle>,
    pub(in crate::app) game_banner_textures: HashMap<Option<GameId>, egui::TextureHandle>,
    pub(in crate::app) game_emblem_textures: HashMap<GameId, egui::TextureHandle>,
    pub(in crate::app) custom_editing_kit_textures: HashMap<String, egui::TextureHandle>,
    pub(in crate::app) custom_editing_kit_texture_failures: HashSet<String>,
    pub(in crate::app) last_pixels_per_point: f32,
    /// When the per-frame prefs check next runs (egui time).
    pub(in crate::app) prefs_next_check_at: f64,
}
