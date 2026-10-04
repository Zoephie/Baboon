//! The application shell: the frame loop and the worker messages it applies,
//! the session saved and restored, update checks, and the windows and bars
//! around the features.

use super::*;
use crate::app::runtime_poke::{LastPoke, PokePlan, PokeReport};
use crate::core::document::value::extension_to_group_tag;
use crate::app::editor::{
    DeferredFileAction, EditorCommand, EditorFeature, PaneInputs, apply_scroll_speed,
    begin_wheel_gesture, draw_tag_pane, end_wheel_gesture, lost_focus_once,
    set_combo_scroll_cycle_enabled, set_zoom_speed, truncate_for_cell, view_text_tab_button,
};
use crate::app::export::ContainerDumpReport;
use crate::app::browser::{
    BITMAP_LIBRARY_KEY, BITMAP_LIBRARY_TITLE, Bitmaps, BrowserAction, BrowserCommand,
    BrowserFeature, BrowserMode, BrowserSort, FilterCache, FolderBrowserState, KeywordChooser,
    MODEL_LIBRARY_KEY, MODEL_LIBRARY_TITLE, Models, PaletteTable, RefOccurrence, TagQueryResults,
    ThumbnailImage, draw_folder_browser_pane, draw_kit_browser, draw_thumbnail_library,
    is_folder_pane_key, style_list_menu, tag_tab_label,
};
use std::cell::RefCell;

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
pub(in crate::app) use workspace::IndexingNotice;
pub(in crate::app) mod welcome;
pub(in crate::app) use welcome::draw_welcome_screen;
pub(in crate::app) mod first_run;
pub(in crate::app) use first_run::FirstRunCommand;
pub(in crate::app) mod settings;
pub(in crate::app) use settings::{
    SettingsCommand, SettingsWindow, draw_nested_default_picker, draw_update_channel_picker,
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
    /// The most recent check's result, kept only while it is actually an
    /// update. The status line expires on a timer, so this is what keeps the
    /// news reachable after a silent startup check.
    pub(in crate::app) available_update: Option<UpdateCheckResult>,
    /// The most recent successful check, update or not, so Settings can report
    /// the outcome after the status line has expired.
    pub(in crate::app) last_update_check: Option<UpdateCheckResult>,
    /// Kits whose session-restore load has not landed yet, and the one the
    /// session named as focused. Every load ends by making its own kit active,
    /// so the focus can only be honoured once none are outstanding.
    pub(in crate::app) restoring_kits: HashSet<KitId>,
    pub(in crate::app) restored_active_kit: Option<KitId>,
    /// Toolbar launcher icons (decoded from embedded .ico at startup).
    pub(in crate::app) blender_icon: Option<egui::TextureHandle>,
    pub(in crate::app) sapien_icon: Option<egui::TextureHandle>,
    pub(in crate::app) tag_test_icon: Option<egui::TextureHandle>,
    pub(in crate::app) artwork: ArtworkCache,
    pub(in crate::app) last_pixels_per_point: f32,
    /// When the per-frame prefs check next runs (egui time).
    pub(in crate::app) prefs_next_check_at: f64,
}

/// Game and editing-kit artwork, each loaded the first time it is drawn.
///
/// Filled behind `&self`, so a draw that may only read the shell — a dialog,
/// through `AppReads` — can still show artwork. A texture handle is a shared
/// reference, so handing out clones costs nothing.
#[derive(Default)]
pub(in crate::app) struct ArtworkCache {
    game_banners: RefCell<HashMap<Option<GameId>, egui::TextureHandle>>,
    game_emblems: RefCell<HashMap<GameId, egui::TextureHandle>>,
    custom_editing_kits: RefCell<HashMap<String, egui::TextureHandle>>,
    /// Custom editing kits whose icon would not load, so it is not retried
    /// every frame.
    custom_editing_kit_failures: RefCell<HashSet<String>>,
}

impl ArtworkCache {
    pub(in crate::app) fn game_banner(
        &self,
        ctx: &egui::Context,
        game: Option<GameId>,
    ) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.game_banners.borrow().get(&game) {
            return Some(texture.clone());
        }
        let name = game.map_or("unknown", GameId::as_str);
        let texture = load_png_texture(
            ctx,
            &format!("game_banner_{name}"),
            get_game_banner_bytes(game),
        )?;
        self.game_banners.borrow_mut().insert(game, texture.clone());
        Some(texture)
    }

    pub(in crate::app) fn game_emblem(
        &self,
        ctx: &egui::Context,
        game: GameId,
    ) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.game_emblems.borrow().get(&game) {
            return Some(texture.clone());
        }
        let bytes = get_game_emblem_bytes(game);
        let texture = load_png_texture(ctx, &format!("game_emblem_{game}"), bytes)?;
        self.game_emblems.borrow_mut().insert(game, texture.clone());
        Some(texture)
    }

    pub(in crate::app) fn custom_editing_kit(
        &self,
        ctx: &egui::Context,
        profile: &CustomEditingKitProfile,
    ) -> Option<egui::TextureHandle> {
        let relative = profile.icon.as_deref()?;
        if self
            .custom_editing_kit_failures
            .borrow()
            .contains(&profile.id)
        {
            return None;
        }
        if let Some(texture) = self.custom_editing_kits.borrow().get(&profile.id) {
            return Some(texture.clone());
        }
        let texture = resolve_custom_icon_path(relative)
            .ok()
            .and_then(|absolute| fs::read(absolute).ok())
            .and_then(|bytes| {
                load_png_texture(ctx, &format!("custom_editing_kit_{}", profile.id), &bytes)
            });
        let Some(texture) = texture else {
            self.custom_editing_kit_failures
                .borrow_mut()
                .insert(profile.id.clone());
            return None;
        };
        self.custom_editing_kits
            .borrow_mut()
            .insert(profile.id.clone(), texture.clone());
        Some(texture)
    }

    /// Resolve the image shown in a loaded workspace's browser header.
    ///
    /// A custom profile's selected image takes precedence over the built-in
    /// engine artwork. Looking the profile up by its stable ID keeps restored
    /// workspaces connected to later name/icon edits without copying a
    /// potentially stale icon path into session state.
    pub(in crate::app) fn workspace_banner(
        &self,
        ctx: &egui::Context,
        profiles: &[CustomEditingKitProfile],
        game: Option<GameId>,
        profile_id: Option<&str>,
    ) -> Option<egui::TextureHandle> {
        let profile = profile_id
            .and_then(|profile_id| profiles.iter().find(|profile| profile.id == profile_id));
        if let Some(profile) = profile
            && let Some(texture) = self.custom_editing_kit(ctx, profile)
        {
            return Some(texture);
        }
        self.game_banner(ctx, game)
    }

    /// Drop every texture, to load again at a new scale.
    pub(in crate::app) fn clear(&self) {
        self.game_banners.borrow_mut().clear();
        self.game_emblems.borrow_mut().clear();
        self.custom_editing_kits.borrow_mut().clear();
        self.custom_editing_kit_failures.borrow_mut().clear();
    }

    /// Forget one custom editing kit's icon, edited or removed.
    pub(in crate::app) fn forget_custom_editing_kit(&self, id: &str) {
        self.custom_editing_kits.borrow_mut().remove(id);
        self.custom_editing_kit_failures.borrow_mut().remove(id);
    }

    /// Try every custom editing kit icon again, as after a path changed.
    pub(in crate::app) fn retry_custom_editing_kits(&self) {
        self.custom_editing_kit_failures.borrow_mut().clear();
    }
}
