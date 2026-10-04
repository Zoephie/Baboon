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
pub(in crate::app) use session::draw_last_opened_windows_prompt;
pub(in crate::app) mod frame;
pub(in crate::app) use frame::{draw_keyword_bar, draw_scenario_launcher_buttons};
pub(in crate::app) mod workspace;
pub(in crate::app) use workspace::IndexingNotice;
pub(in crate::app) mod welcome;
pub(in crate::app) use welcome::draw_welcome_screen;
pub(in crate::app) mod first_run;
pub(in crate::app) use first_run::{FirstRunCommand, draw_first_run_wizard};
pub(in crate::app) mod settings;
pub(in crate::app) use settings::{
    SettingsCommand, draw_nested_default_picker, draw_settings_window, draw_update_channel_picker,
};
pub(in crate::app) mod kit_tiles;
pub(in crate::app) use kit_tiles::TileParts;
pub(in crate::app) mod tag_tiles;
pub(in crate::app) use tag_tiles::{TileInputs, draw_tag_tiles};
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

/// The shell's caches of game and editing-kit artwork, each loaded once.
impl ShellFeature {
    pub(in crate::app) fn game_banner_texture(
        &mut self,
        ctx: &egui::Context,
        game: Option<GameId>,
    ) -> Option<&egui::TextureHandle> {
        if !self.game_banner_textures.contains_key(&game) {
            let name = game.map_or("unknown", GameId::as_str);
            let texture = load_png_texture(
                ctx,
                &format!("game_banner_{name}"),
                get_game_banner_bytes(game),
            )?;
            self.game_banner_textures.insert(game, texture);
        }
        self.game_banner_textures.get(&game)
    }

    pub(in crate::app) fn game_emblem_texture(
        &mut self,
        ctx: &egui::Context,
        game: GameId,
    ) -> Option<&egui::TextureHandle> {
        if !self.game_emblem_textures.contains_key(&game) {
            let bytes = get_game_emblem_bytes(game);
            let texture = load_png_texture(ctx, &format!("game_emblem_{game}"), bytes)?;
            self.game_emblem_textures.insert(game, texture);
        }
        self.game_emblem_textures.get(&game)
    }

    pub(in crate::app) fn custom_editing_kit_texture(
        &mut self,
        ctx: &egui::Context,
        profile: &CustomEditingKitProfile,
    ) -> Option<&egui::TextureHandle> {
        let relative = profile.icon.as_deref()?;
        if self
            .custom_editing_kit_texture_failures
            .contains(&profile.id)
        {
            return None;
        }
        if !self.custom_editing_kit_textures.contains_key(&profile.id) {
            let texture = resolve_custom_icon_path(relative)
                .ok()
                .and_then(|absolute| fs::read(absolute).ok())
                .and_then(|bytes| {
                    load_png_texture(ctx, &format!("custom_editing_kit_{}", profile.id), &bytes)
                });
            let Some(texture) = texture else {
                self.custom_editing_kit_texture_failures
                    .insert(profile.id.clone());
                return None;
            };
            self.custom_editing_kit_textures
                .insert(profile.id.clone(), texture);
        }
        self.custom_editing_kit_textures.get(&profile.id)
    }

    /// Resolve the image shown in a loaded workspace's browser header.
    ///
    /// A custom profile's selected image takes precedence over the built-in
    /// engine artwork. Looking the profile up by its stable ID keeps restored
    /// workspaces connected to later name/icon edits without copying a
    /// potentially stale icon path into session state.
    pub(in crate::app) fn workspace_banner_texture(
        &mut self,
        ctx: &egui::Context,
        profiles: &[CustomEditingKitProfile],
        game: Option<GameId>,
        profile_id: Option<&str>,
    ) -> Option<egui::TextureHandle> {
        let profile = profile_id.and_then(|profile_id| {
            profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .cloned()
        });
        if let Some(profile) = profile
            && let Some(texture) = self.custom_editing_kit_texture(ctx, &profile).cloned()
        {
            return Some(texture);
        }
        self.game_banner_texture(ctx, game).cloned()
    }
}
