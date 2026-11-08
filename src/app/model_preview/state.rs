//! preview application state.
//! It owns state types only; drawing them and acting on them belong to the feature's other modules.

use super::*;

pub(in crate::app) const DEFAULT_MODEL_PREVIEW_SIZE: f32 = 1.0;
pub(in crate::app) const MIN_MODEL_PREVIEW_SIZE: f32 = 0.8;
pub(in crate::app) const MAX_MODEL_PREVIEW_SIZE: f32 = 2.6;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::app) enum BitmapPanelTab {
    #[default]
    Fields,
    Texture,
}


/// Background fill behind the bitmap preview image. Helps judge alpha edges
/// against light/dark/saturated backdrops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum BitmapPreviewBg {
    DarkGray,
    Black,
    White,
    Magenta,
}

impl BitmapPreviewBg {
    pub(in crate::app) const ALL: [Self; 4] =
        [Self::White, Self::DarkGray, Self::Black, Self::Magenta];

    pub(in crate::app) fn color(self) -> egui::Color32 {
        match self {
            // Match the dark-mode page background rather than the lighter
            // Foundation section/header surfaces.
            Self::DarkGray => egui::Color32::from_rgb(40, 40, 40),
            Self::Black => egui::Color32::BLACK,
            Self::White => egui::Color32::WHITE,
            Self::Magenta => egui::Color32::from_rgb(255, 0, 255),
        }
    }

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::DarkGray => "Dark Grey",
            Self::Black => "Black",
            Self::White => "White",
            Self::Magenta => "Magenta",
        }
    }

    pub(in crate::app) fn as_str(self) -> &'static str {
        match self {
            Self::DarkGray => "dark_gray",
            Self::Black => "black",
            Self::White => "white",
            Self::Magenta => "magenta",
        }
    }

    pub(in crate::app) fn from_str(value: &str) -> Option<Self> {
        match value {
            "dark_gray" => Some(Self::DarkGray),
            "black" => Some(Self::Black),
            "white" => Some(Self::White),
            "magenta" => Some(Self::Magenta),
            _ => None,
        }
    }
}

/// View choices shared by every bitmap preview and persisted between sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct BitmapPreviewViewSettings {
    pub(in crate::app) bg: BitmapPreviewBg,
    pub(in crate::app) show_checkerboard: bool,
    pub(in crate::app) show_border: bool,
}

impl Default for BitmapPreviewViewSettings {
    fn default() -> Self {
        Self {
            bg: BitmapPreviewBg::DarkGray,
            show_checkerboard: true,
            show_border: true,
        }
    }
}

/// Cross-frame bitmap view controls and lazily uploaded texture state.
/// `texture_dirty` is the synchronization boundary between decoded RGBA data and
/// the GPU texture after channel, image, or mip selection changes.
pub(in crate::app) struct BitmapPreviewState {
    pub(in crate::app) active_tab: BitmapPanelTab,
    pub(in crate::app) show_red: bool,
    pub(in crate::app) show_green: bool,
    pub(in crate::app) show_blue: bool,
    pub(in crate::app) show_alpha: bool,
    /// Draw a subtle fixed-size checker behind the image to expose alpha.
    pub(in crate::app) show_checkerboard: bool,
    /// Draw a high-contrast two-pixel outline just outside the image bounds.
    pub(in crate::app) show_border: bool,
    pub(in crate::app) decoded: Option<Result<BitmapPreviewData, String>>,
    /// A decode running on a worker, to be taken as `decoded` when it lands.
    /// Dropped with `decoded` when the tag changes, so a result decoded from
    /// the old bytes is never shown.
    pub(in crate::app) decoding:
        Option<std::sync::mpsc::Receiver<Result<BitmapPreviewData, String>>>,
    pub(in crate::app) texture: Option<egui::TextureHandle>,
    pub(in crate::app) checker_texture: Option<egui::TextureHandle>,
    pub(in crate::app) texture_dirty: bool,
    pub(in crate::app) zoom: f32,
    /// Pan offset of the image center relative to the canvas center, in
    /// screen pixels. Updated by drag-to-pan and zoom-to-cursor.
    pub(in crate::app) pan: Vec2,
    /// False until zoom is initialized to fit the image on first decode.
    pub(in crate::app) zoom_initialized: bool,
    /// Background fill behind the previewed image.
    pub(in crate::app) bg: BitmapPreviewBg,
    /// Selected image (sequence) index and mipmap level being previewed.
    pub(in crate::app) image_index: usize,
    pub(in crate::app) mip_index: usize,
}

impl Default for BitmapPreviewState {
    fn default() -> Self {
        Self {
            active_tab: BitmapPanelTab::Fields,
            show_red: true,
            show_green: true,
            show_blue: true,
            show_alpha: true,
            show_checkerboard: true,
            show_border: true,
            decoded: None,
            decoding: None,
            texture: None,
            checker_texture: None,
            texture_dirty: true,
            zoom: 1.0,
            pan: Vec2::ZERO,
            zoom_initialized: false,
            bg: BitmapPreviewBg::DarkGray,
            image_index: 0,
            mip_index: 0,
        }
    }
}

impl BitmapPreviewState {
    pub(in crate::app) fn apply_view_settings(&mut self, settings: BitmapPreviewViewSettings) {
        self.bg = settings.bg;
        self.show_checkerboard = settings.show_checkerboard;
        self.show_border = settings.show_border;
    }

    pub(in crate::app) fn view_settings(&self) -> BitmapPreviewViewSettings {
        BitmapPreviewViewSettings {
            bg: self.bg,
            show_checkerboard: self.show_checkerboard,
            show_border: self.show_border,
        }
    }
}

/// Decoded pixels and metadata for the currently selected bitmap image/mip.
/// `rgba` is always tightly packed RGBA8 for `width * height` pixels.
pub(in crate::app) struct BitmapPreviewData {
    pub(in crate::app) width: u32,
    pub(in crate::app) height: u32,
    pub(in crate::app) image_count: usize,
    /// Mipmap level count of the currently-decoded image (≥ 1).
    pub(in crate::app) mip_count: usize,
    pub(in crate::app) format_name: String,
    pub(in crate::app) type_name: String,
    pub(in crate::app) rgba: Vec<u8>,
}

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
    /// Case-insensitive substring filter on marker names (empty = show all).
    /// Only applied while `show_markers` is on.
    pub(in crate::app) marker_filter: String,
    /// Campaign Evolved only: decode the full-resolution **Nanite** geometry for
    /// static mesh pieces instead of the coarse LOD fallback. On by default for
    /// faithful previews; users can disable it for unusually heavy models.
    pub(in crate::app) high_detail: bool,
    /// The `high_detail` value the cached `data` was built with, so toggling it
    /// invalidates the cache and reloads.
    pub(in crate::app) loaded_high_detail: bool,
    /// Bipeds: the weapon held, by path. They start with none.
    pub(in crate::app) weapon: Option<String>,
    /// The `weapon` the cached `data` was built with.
    pub(in crate::app) loaded_weapon: Option<String>,
    /// Halo 2 bipeds: what the left hand holds.
    pub(in crate::app) secondary_weapon: Option<String>,
    pub(in crate::app) loaded_secondary_weapon: Option<String>,
    /// What outlives the reload a weapon change makes: the old geometry's id,
    /// for its function inputs, and the region selections.
    pub(in crate::app) ce_weapon_carry: Option<(u64, HashMap<String, ModelRegionSelection>)>,
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
    /// Draw Halo 2 opaque materials through the game's own passes.
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
    /// What the panel set an opened Halo CE object's function inputs to.
    pub(in crate::app) ce_function_inputs: super::ce::functions::CeFunctionInputs,
    /// How many times the panel has played each Halo CE event effect, by
    /// model and index among the preview's effects.
    pub(in crate::app) ce_event_counts: super::ce::functions::CeEventCounts,
    /// Scale A and scale B the panel plays Halo CE event effects at.
    pub(in crate::app) ce_event_scales: [f32; 2],
    /// The shots the panel has fired from Halo CE weapons' triggers.
    pub(in crate::app) ce_shots: super::ce::firing::CeShots,
    /// What the panel has set of a Halo 2 object's functions.
    pub(in crate::app) function_inputs: super::functions::FunctionInputs,
    /// A texture-resolve job is running for the loaded model.
    pub(in crate::app) textures_pending: bool,
    /// `.model` overlays only: draw the render model itself. Off leaves just
    /// the collision/physics layers on screen — a frame-level filter, so
    /// toggling never rebuilds geometry.
    pub(in crate::app) show_render: bool,
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
    pub(in crate::app) scale: f32,
    pub(in crate::app) yaw: f32,
    pub(in crate::app) pitch: f32,
    /// World-space offset of the orbit point from the geometry's bounds
    /// center. Panning moves this, so the camera always orbits and zooms
    /// around what the user framed — screen-space panning made orbiting a
    /// BSP's far corner swing it offscreen.
    pub(in crate::app) focus: [f32; 3],
}

impl Default for ModelPreviewState {
    fn default() -> Self {
        Self {
            loaded_key: None,
            preview_load_id: None,
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
            marker_filter: String::new(),
            high_detail: true,
            loaded_high_detail: false,
            weapon: None,
            loaded_weapon: None,
            secondary_weapon: None,
            loaded_secondary_weapon: None,
            ce_weapon_carry: None,
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
            ce_function_inputs: Default::default(),
            ce_event_counts: Default::default(),
            ce_event_scales: super::ce::functions::CE_EVENT_SCALES,
            ce_shots: Default::default(),
            function_inputs: Default::default(),
            textures_pending: false,
            show_render: true,
            perspective: true,
            animation: PreviewAnimationPlayback::default(),
            show_grid: true,
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
        self.preview_load_id = None;
    }

    /// Note the clip and variant selected, by name, for the reload to choose
    /// again; what is noted already (a weapon change's clip, a variant just
    /// created) stays.
    pub(in crate::app) fn remember_selection(&mut self) {
        let Some(Ok(data)) = &self.data else {
            return;
        };
        let animations = data.animations.as_deref().map_or(&[][..], Vec::as_slice);
        let clip = self.animation.selected.and_then(|index| Some(animations.get(index)?.name.clone()));
        let variant = self.selected_variant.and_then(|index| Some(data.variants.get(index)?.name.clone()));
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
        self.loaded_key.as_deref() != Some(entry_key)
            || self.data.is_none()
            || self.loaded_high_detail != self.high_detail
            || self.loaded_weapon != self.weapon
            || self.loaded_secondary_weapon != self.secondary_weapon
            || self.loaded_scenario_selection != self.scenario_bsp_selection
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

#[cfg(test)]
mod tests {
    use super::*;

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

