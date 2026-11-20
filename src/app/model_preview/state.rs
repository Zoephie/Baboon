//! preview application state.
//! It owns state types only; drawing them and acting on them belong to the feature's other modules.

use super::{ModelPreviewData, PreviewAnimationPlayback};
use std::collections::HashMap;

/// The Model Setup sidebar's width, in points, before it is dragged.
pub(in crate::app) const DEFAULT_MODEL_SETUP_WIDTH: f32 = 360.0;
/// The narrowest the sidebar is dragged to, and the widest it is saved at.
pub(in crate::app) const MIN_MODEL_SETUP_WIDTH: f32 = 240.0;
pub(in crate::app) const MAX_MODEL_SETUP_WIDTH: f32 = 1200.0;

/// Cross-frame model selection and camera state.
/// `loaded_key` prevents data and variant choices from being reused for a newly
/// selected document whose preview has not yet been resolved.
pub(in crate::app) struct ModelPreviewState {
    pub(in crate::app) loaded_key: Option<String>,
    /// Identifies the base geometry/variant build currently running. A newer
    /// request replaces it; late worker replies are ignored by id.
    pub(in crate::app) preview_load_id: Option<u64>,
    pub(in crate::app) render_model_path: Option<String>,
    pub(in crate::app) data: Option<Result<ModelPreviewData, String>>,
    /// The model as it was before a settings change (Game Shaders, a weapon)
    /// began loading it again: the page keeps drawing from it, and only the
    /// viewport waits on the load.
    pub(in crate::app) reloading: Option<ReloadingPreview>,
    /// The page as it was before that reload, still drawn beside the new
    /// model's viewport while what follows the load (its materials, its
    /// collision and physics, its weapons, its clip) arrives, so the
    /// sidebar and player change once, not card by card.
    pub(in crate::app) settling: Option<SettlingPreview>,
    pub(in crate::app) active_tab: ModelTagPanelTab,
    pub(in crate::app) new_variant_name: String,
    pub(in crate::app) selected_variant: Option<usize>,
    pub(in crate::app) region_selections: HashMap<String, ModelRegionSelection>,
    pub(in crate::app) show_markers: bool,
    /// Draw import-tool error geometry and expose each report on hover.
    pub(in crate::app) show_errors: bool,
    /// Include reports carrying the import tool's non-critical flag.
    pub(in crate::app) show_non_critical_errors: bool,
    /// Draw the model skeleton over the preview, with hoverable bone names.
    pub(in crate::app) show_armature: bool,
    /// Draw the object's bounding sphere, for a tag that has one.
    pub(in crate::app) show_bounding_sphere: bool,
    /// The object's `bounding radius` about its `bounding offset`, read from
    /// the open tag each pass so an edit shows at once. `None` for a tag
    /// that is no object.
    pub(in crate::app) bounding_sphere: Option<super::viewport::BoundingSphere>,
    /// Case-insensitive substring filter on marker names (empty = show all).
    /// Only applied while `show_markers` is on.
    pub(in crate::app) marker_filter: String,
    /// Campaign Evolved's settings.
    pub(in crate::app) campaign_evolved: super::campaign_evolved::CampaignEvolvedState,
    /// Bipeds: the weapon held, by path. They start with none.
    pub(in crate::app) weapon: Option<String>,
    /// The `weapon` the cached `data` was built with.
    pub(in crate::app) loaded_weapon: Option<String>,
    /// Halo 2 and later bipeds: what the left hand holds.
    pub(in crate::app) secondary_weapon: Option<String>,
    pub(in crate::app) loaded_secondary_weapon: Option<String>,
    /// The `game_shaders` the cached `data` was read for: turned on or off,
    /// the preview reads again (`TagReader::game_shaders`).
    pub(in crate::app) loaded_game_shaders: bool,
    /// Halo CE's and Halo 2's level of detail picked, lowest first; `None`
    /// for the highest. A view setting: never written to the tag.
    pub(in crate::app) detail_level: Option<usize>,
    pub(in crate::app) loaded_detail_level: Option<usize>,
    /// The weapon picker opened before the unit's weapons were listed: the
    /// pane lists them on a worker (`maybe_request_unit_weapons`).
    pub(in crate::app) list_weapons: bool,
    /// Whether that listing is running.
    pub(in crate::app) weapons_pending: bool,
    /// What outlives the reload a weapon change or the Game Shaders toggle
    /// makes: the old geometry's id, for its function inputs, and the region
    /// selections.
    pub(in crate::app) reload_carry: Option<(u64, HashMap<String, ModelRegionSelection>)>,
    /// The clip selected before a reload, picked again by name once listed.
    pub(in crate::app) clip_after_load: Option<String>,
    /// The variant selected before a reload, or one just created, chosen
    /// again by name once loaded.
    pub(in crate::app) variant_after_load: Option<String>,
    /// `.model` tags only: draw the collision overlay. A draw-time filter,
    /// not a rebuild — the overlay geometry is built once on a worker when
    /// the preview loads, so toggling is instant in both directions.
    pub(in crate::app) show_collision: bool,
    /// `.model` tags only: draw the physics overlay. Same contract.
    pub(in crate::app) show_physics: bool,
    /// A worker is building this model's collision/physics overlays.
    pub(in crate::app) overlays_pending: bool,
    /// The overlays are merged into `data` (or were found absent), so no
    /// further request is needed for this load.
    pub(in crate::app) overlays_loaded: bool,
    /// Scenario tags only: which entries of the `structure bsps` block are
    /// loaded into the composite preview. Empty until the user picks some —
    /// loading every BSP of a campaign scenario unasked would stall the pane.
    pub(in crate::app) scenario_bsp_selection: std::collections::BTreeSet<usize>,
    /// The selection the cached `data` was built for.
    pub(in crate::app) loaded_scenario_selection: std::collections::BTreeSet<usize>,
    pub(in crate::app) render_mode: ModelRenderMode,
    pub(in crate::app) show_backfaces: bool,
    /// Draw materials through the game's own passes.
    pub(in crate::app) game_shaders: bool,
    /// Take the game's no-lightmap path, with its two default lights, instead
    /// of its path for an object in a level.
    pub(in crate::app) default_lights: bool,
    /// Keep the direct light of the object path beside the viewer.
    pub(in crate::app) light_follows_camera: bool,
    pub(in crate::app) active_camo: bool,
    pub(in crate::app) bloom: bool,
    pub(in crate::app) self_shadow: bool,
    pub(in crate::app) prt: bool,
    /// What the panel has set and played of an opened Halo CE object.
    pub(in crate::app) ce: super::ce::panel::CePanelState,
    /// What the panel has set of a Halo 2 or later object's functions.
    pub(in crate::app) function_inputs: super::functions::FunctionInputs,
    /// A texture-resolve job is running for the loaded model.
    pub(in crate::app) textures_pending: bool,
    /// `.model` overlays only: draw the render model itself. Off leaves just
    /// the collision/physics layers on screen — a frame-level filter, so
    /// toggling never rebuilds geometry.
    pub(in crate::app) show_render: bool,
    /// Draw the model's stand-in (Halo 3's and ODST's LOD render model).
    pub(in crate::app) show_lod_model: bool,
    /// Draw the model's imposter (Reach, Halo 2 Anniversary).
    pub(in crate::app) show_imposter: bool,
    /// Perspective projection (the default) instead of orthographic. A fixed
    /// 60° lens whose eye dollies toward the orbit point as the zoom grows,
    /// placed so the focus plane matches the orthographic framing exactly —
    /// toggling never jumps, and the zoom slider keeps one meaning in both.
    /// The tag pane keeps it in step with `GuiPrefs::model_preview_perspective`.
    pub(in crate::app) perspective: bool,
    /// Animation playback (selection, clock, decoded pose) for `.model`
    /// previews.
    pub(in crate::app) animation: PreviewAnimationPlayback,
    /// Draw the world-unit ground grid at z = 0 under the geometry.
    pub(in crate::app) show_grid: bool,
    /// What the viewport is cleared to. The tag pane keeps it in step with
    /// `GuiPrefs::model_preview_background`.
    pub(in crate::app) background: ModelPreviewBackground,
    /// The footer's About button was pressed; the tag pane opens the Model
    /// Preview help page and clears it.
    pub(in crate::app) help_requested: bool,
    pub(in crate::app) scale: f32,
    pub(in crate::app) yaw: f32,
    pub(in crate::app) pitch: f32,
    /// World-space offset of the orbit point from the geometry's bounds
    /// center. Panning moves this, so the camera always orbits and zooms
    /// around what the user framed — screen-space panning made orbiting a
    /// BSP's far corner swing it offscreen.
    pub(in crate::app) focus: [f32; 3],
}

/// The last loaded model and how its player stood, drawn while the same
/// model loads again with new settings.
pub(in crate::app) struct ReloadingPreview {
    pub(in crate::app) data: ModelPreviewData,
    pub(in crate::app) animation: super::PreviewAnimationPlayback,
}

/// A reload's page before it, held until the new model's page is complete.
pub(in crate::app) struct SettlingPreview {
    pub(in crate::app) page: ReloadingPreview,
    /// When the panel first drew it, by the UI clock.
    pub(in crate::app) since: Option<f64>,
}

/// The longest the old page stands in for the new one, in seconds: what
/// follows a load can fail, and the page must not stay still for good.
const MAX_SETTLE_SECONDS: f64 = 3.0;

impl Default for ModelPreviewState {
    fn default() -> Self {
        Self {
            loaded_key: None,
            preview_load_id: None,
            reloading: None,
            settling: None,
            render_model_path: None,
            data: None,
            active_tab: ModelTagPanelTab::Fields,
            new_variant_name: String::new(),
            selected_variant: None,
            region_selections: HashMap::new(),
            show_markers: false,
            show_errors: false,
            show_non_critical_errors: false,
            show_armature: false,
            show_bounding_sphere: false,
            bounding_sphere: None,
            marker_filter: String::new(),
            campaign_evolved: Default::default(),
            weapon: None,
            loaded_weapon: None,
            secondary_weapon: None,
            loaded_secondary_weapon: None,
            loaded_game_shaders: false,
            detail_level: None,
            loaded_detail_level: None,
            list_weapons: false,
            weapons_pending: false,
            reload_carry: None,
            clip_after_load: None,
            variant_after_load: None,
            show_collision: false,
            show_physics: false,
            overlays_pending: false,
            overlays_loaded: false,
            scenario_bsp_selection: std::collections::BTreeSet::new(),
            loaded_scenario_selection: std::collections::BTreeSet::new(),
            render_mode: ModelRenderMode::Textured,
            show_backfaces: false,
            game_shaders: false,
            default_lights: false,
            light_follows_camera: true,
            active_camo: false,
            bloom: true,
            self_shadow: false,
            prt: true,
            ce: Default::default(),
            function_inputs: Default::default(),
            textures_pending: false,
            show_render: true,
            show_lod_model: false,
            show_imposter: false,
            perspective: true,
            animation: PreviewAnimationPlayback::default(),
            show_grid: true,
            background: ModelPreviewBackground::Theme,
            help_requested: false,
            scale: 1.0,
            yaw: -0.45,
            pitch: 0.25,
            focus: [0.0; 3],
        }
    }
}

impl ModelPreviewState {
    /// Forget the loaded preview and any load in flight, so the next frame
    /// rebuilds from the document as it is now. Dropping the pending load
    /// matters after an edit: its worker is parsing the bytes from before it.
    pub(in crate::app) fn invalidate_load(&mut self) {
        self.remember_selection();
        self.loaded_key = None;
        self.data = None;
        self.reloading = None;
        self.settling = None;
        self.preview_load_id = None;
    }

    /// Whether the new model's page, `data`, has what the old one showed:
    /// its materials (which the Object Functions card reads), its
    /// collision and physics, its weapons and its clip. At `now`, a page
    /// held past [`MAX_SETTLE_SECONDS`] counts as settled.
    pub(in crate::app) fn page_settled(
        &self,
        settling: &SettlingPreview,
        data: &ModelPreviewData,
        now: f64,
    ) -> bool {
        if settling.since.is_some_and(|since| now - since > MAX_SETTLE_SECONDS) {
            return true;
        }
        let had_overlays = settling.page.data.preview.batches.iter().any(|batch| {
            matches!(batch.layer, super::ModelPreviewLayer::Collision | super::ModelPreviewLayer::Physics)
        });
        let materials = !self.render_mode.uses_textures()
            || data.preview.materials.is_empty()
            || data.textures_for(self.game_shaders).is_some();
        let overlays = !had_overlays || self.overlays_loaded;
        let weapons = !self.list_weapons && !self.weapons_pending;
        let clip = !self.animation.resume_on_decode;
        materials && overlays && weapons && clip
    }

    /// Note the clip and variant selected, by name, for the reload to choose
    /// again; what is noted already (a weapon change's clip, a variant just
    /// created) stays.
    pub(in crate::app) fn remember_selection(&mut self) {
        let Some(Ok(data)) = &self.data else {
            return;
        };
        let animations = data.animations.as_deref().map_or(&[][..], Vec::as_slice);
        let clip = self
            .animation
            .selected
            .and_then(|index| Some(animations.get(index)?.name.clone()));
        let variant = self
            .selected_variant
            .and_then(|index| Some(data.variants.get(index)?.name.clone()));
        if self.clip_after_load.is_none() {
            self.clip_after_load = clip;
        }
        if self.variant_after_load.is_none() {
            self.variant_after_load = variant;
        }
    }

    /// Whether the cached preview no longer matches what the panel should
    /// show — a different tag, or a load-affecting setting that changed.
    /// One predicate for the panel's spinner gate and the loader's early
    /// return, so the two can never disagree about what needs a rebuild.
    pub(in crate::app) fn needs_preview_load(&self, entry_key: &str) -> bool {
        self.data.is_none() || !self.load_settings_match(entry_key)
    }

    /// Whether the last load asked for was of `entry_key` with every
    /// setting as it is now. The loader asks this before starting one, so
    /// a setting that marks the page out of date always starts its load.
    pub(in crate::app) fn load_settings_match(&self, entry_key: &str) -> bool {
        self.loaded_key.as_deref() == Some(entry_key)
            && self.campaign_evolved.loaded_as_set()
            && self.loaded_weapon == self.weapon
            && self.loaded_secondary_weapon == self.secondary_weapon
            && self.loaded_game_shaders == self.game_shaders
            && self.loaded_detail_level == self.detail_level
            && self.loaded_scenario_selection == self.scenario_bsp_selection
    }
}

#[cfg(test)]
impl ModelPreviewState {
    /// A state showing an empty preview of `entry_key` as loaded, for a test
    /// that draws the panel around it.
    pub(in crate::app) fn loaded_empty_for_test(entry_key: &str) -> Self {
        // One region, so the sidebar has its Permutations card.
        let preview = super::RenderModelPreview {
            regions: vec![super::RenderModelPreviewRegion {
                name: "body".into(),
                permutations: vec!["base".into()],
            }],
            ..Default::default()
        };
        let data = super::model_preview_data(
            entry_key.to_owned(),
            entry_key.to_owned(),
            preview,
            Vec::new(),
        );
        let mut state = Self {
            active_tab: ModelTagPanelTab::ModelPreview,
            loaded_key: Some(entry_key.to_owned()),
            data: Some(Ok(data)),
            ..Self::default()
        };
        state.campaign_evolved.loaded_high_detail = state.campaign_evolved.high_detail;
        state
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ModelRenderMode {
    Solid,
    Wireframe,
    SolidWireframe,
    Textured,
    TexturedWireframe,
}

impl ModelRenderMode {
    pub(in crate::app) const ALL: [Self; 5] = [
        Self::Solid,
        Self::Wireframe,
        Self::SolidWireframe,
        Self::Textured,
        Self::TexturedWireframe,
    ];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Wireframe => "Wireframe",
            Self::SolidWireframe => "Solid + Wireframe",
            Self::Textured => "Textured",
            Self::TexturedWireframe => "Textured + Wireframe",
        }
    }

    pub(in crate::app) fn draws_shading(self) -> bool {
        !matches!(self, Self::Wireframe)
    }

    pub(in crate::app) fn draws_wireframe(self) -> bool {
        matches!(
            self,
            Self::Wireframe | Self::SolidWireframe | Self::TexturedWireframe
        )
    }

    pub(in crate::app) fn uses_textures(self) -> bool {
        matches!(self, Self::Textured | Self::TexturedWireframe)
    }

    pub(in crate::app) fn without_textures(self) -> Self {
        match self {
            Self::Textured => Self::Solid,
            Self::TexturedWireframe => Self::SolidWireframe,
            other => other,
        }
    }
}

/// The Shading menu's choice: how the model's surfaces are drawn. Each is a
/// render mode, and Game Shaders is Textured through the game's own passes;
/// the wireframe drawn over a shaded choice is a toggle of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ModelShading {
    Wireframe,
    Solid,
    Textured,
    GameShaders,
}

impl ModelShading {
    pub(in crate::app) const ALL: [Self; 4] =
        [Self::Wireframe, Self::Solid, Self::Textured, Self::GameShaders];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Wireframe => "Wireframe",
            Self::Solid => "Solid",
            Self::Textured => "Textured",
            Self::GameShaders => "Game Shaders",
        }
    }

    pub(in crate::app) fn uses_textures(self) -> bool {
        matches!(self, Self::Textured | Self::GameShaders)
    }
}

/// What the model viewport is cleared to behind the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ModelPreviewBackground {
    /// The page's own background, dark or light with the theme.
    Theme,
    White,
    Black,
    GreenScreen,
}

impl ModelPreviewBackground {
    pub(in crate::app) const ALL: [Self; 4] =
        [Self::Theme, Self::White, Self::Black, Self::GreenScreen];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::White => "White",
            Self::Black => "Black",
            Self::GreenScreen => "Green Screen",
        }
    }

    /// The clear colour, `theme` being the page's own.
    pub(in crate::app) fn color(self, theme: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Theme => theme,
            Self::White => [1.0; 3],
            Self::Black => [0.0; 3],
            // Chroma-key green, #00B140.
            Self::GreenScreen => [0.0, 177.0 / 255.0, 64.0 / 255.0],
        }
    }

    pub(in crate::app) fn as_str(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::White => "white",
            Self::Black => "black",
            Self::GreenScreen => "green_screen",
        }
    }

    pub(in crate::app) fn from_str(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|background| background.as_str() == value)
    }
}

impl ModelPreviewState {
    /// The Shading menu's choice the render mode and Game Shaders make.
    pub(in crate::app) fn shading(&self) -> ModelShading {
        match self.render_mode {
            ModelRenderMode::Wireframe => ModelShading::Wireframe,
            ModelRenderMode::Solid | ModelRenderMode::SolidWireframe => ModelShading::Solid,
            ModelRenderMode::Textured | ModelRenderMode::TexturedWireframe
                if self.game_shaders =>
            {
                ModelShading::GameShaders
            }
            ModelRenderMode::Textured | ModelRenderMode::TexturedWireframe => {
                ModelShading::Textured
            }
        }
    }

    /// Pick `shading`, keeping the wireframe drawn over it as it was.
    /// Returns whether Game Shaders turned on or off, which reads the
    /// preview again.
    pub(in crate::app) fn set_shading(&mut self, shading: ModelShading) -> bool {
        let overlay = self.wireframe_overlay();
        self.render_mode = match shading {
            ModelShading::Wireframe => ModelRenderMode::Wireframe,
            ModelShading::Solid if overlay => ModelRenderMode::SolidWireframe,
            ModelShading::Solid => ModelRenderMode::Solid,
            _ if overlay => ModelRenderMode::TexturedWireframe,
            _ => ModelRenderMode::Textured,
        };
        let game_shaders = shading == ModelShading::GameShaders;
        let changed = self.game_shaders != game_shaders;
        self.game_shaders = game_shaders;
        changed
    }

    /// Whether a wireframe is drawn over the shaded surfaces.
    pub(in crate::app) fn wireframe_overlay(&self) -> bool {
        matches!(
            self.render_mode,
            ModelRenderMode::SolidWireframe | ModelRenderMode::TexturedWireframe
        )
    }

    /// Draw a wireframe over the shaded surfaces, or stop. Wireframe shading
    /// is nothing but the wireframe, so it is left as it is.
    pub(in crate::app) fn set_wireframe_overlay(&mut self, on: bool) {
        use ModelRenderMode::*;
        self.render_mode = match (self.render_mode, on) {
            (Solid | SolidWireframe, true) => SolidWireframe,
            (Solid | SolidWireframe, false) => Solid,
            (Textured | TexturedWireframe, true) => TexturedWireframe,
            (Textured | TexturedWireframe, false) => Textured,
            (Wireframe, _) => Wireframe,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shading_choices_keep_the_wireframe_overlay() {
        let mut state = ModelPreviewState::default();
        assert_eq!(state.shading(), ModelShading::Textured);
        state.set_wireframe_overlay(true);
        assert_eq!(state.render_mode, ModelRenderMode::TexturedWireframe);
        assert!(state.set_shading(ModelShading::GameShaders), "Game Shaders turned on");
        assert_eq!(state.shading(), ModelShading::GameShaders);
        assert!(state.wireframe_overlay());
        assert!(state.set_shading(ModelShading::Solid), "Game Shaders turned off");
        assert_eq!(state.render_mode, ModelRenderMode::SolidWireframe);
        assert!(!state.game_shaders);
        state.set_shading(ModelShading::Wireframe);
        state.set_wireframe_overlay(false);
        assert_eq!(state.render_mode, ModelRenderMode::Wireframe);
        assert!(!state.set_shading(ModelShading::Textured));
        assert_eq!(state.render_mode, ModelRenderMode::Textured);
    }

    #[test]
    fn preview_backgrounds_round_trip_their_names() {
        for background in ModelPreviewBackground::ALL {
            assert_eq!(
                ModelPreviewBackground::from_str(background.as_str()),
                Some(background)
            );
        }
        let theme = [0.1, 0.2, 0.3];
        assert_eq!(ModelPreviewBackground::Theme.color(theme), theme);
    }

    #[test]
    fn model_render_modes_select_expected_passes() {
        assert!(ModelRenderMode::Solid.draws_shading());
        assert!(!ModelRenderMode::Solid.draws_wireframe());
        assert!(!ModelRenderMode::Solid.uses_textures());

        assert!(!ModelRenderMode::Wireframe.draws_shading());
        assert!(ModelRenderMode::Wireframe.draws_wireframe());

        assert!(ModelRenderMode::SolidWireframe.draws_shading());
        assert!(ModelRenderMode::SolidWireframe.draws_wireframe());
        assert!(ModelRenderMode::Textured.draws_shading());
        assert!(!ModelRenderMode::Textured.draws_wireframe());
        assert!(ModelRenderMode::Textured.uses_textures());
        assert!(ModelRenderMode::TexturedWireframe.draws_shading());
        assert!(ModelRenderMode::TexturedWireframe.draws_wireframe());
        assert!(ModelRenderMode::TexturedWireframe.uses_textures());
        assert_eq!(
            ModelPreviewState::default().render_mode,
            ModelRenderMode::Textured
        );
    }

    #[test]
    fn unavailable_textures_keep_the_wireframe_choice() {
        assert_eq!(
            ModelRenderMode::Textured.without_textures(),
            ModelRenderMode::Solid
        );
        assert_eq!(
            ModelRenderMode::TexturedWireframe.without_textures(),
            ModelRenderMode::SolidWireframe
        );
        assert_eq!(
            ModelRenderMode::Wireframe.without_textures(),
            ModelRenderMode::Wireframe
        );
    }

    #[test]
    fn non_critical_errors_are_opt_in() {
        let mut state = ModelPreviewState::default();
        assert!(!state.show_non_critical_errors);
        state.show_non_critical_errors = true;
        assert!(state.show_non_critical_errors);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ModelTagPanelTab {
    Fields,
    ModelPreview,
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct ModelRegionSelection {
    pub(in crate::app) enabled: bool,
    pub(in crate::app) permutation: String,
}
