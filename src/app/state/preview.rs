//! preview application state.
//! It owns passive cross-frame state and operation messages; rendering and workflow execution belong to UI and controller modules.

use super::*;

pub(in crate::app) const DEFAULT_MODEL_PREVIEW_SIZE: f32 = 1.0;
pub(in crate::app) const MIN_MODEL_PREVIEW_SIZE: f32 = 0.8;
pub(in crate::app) const MAX_MODEL_PREVIEW_SIZE: f32 = 2.6;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum BitmapPanelTab {
    Fields,
    Texture,
}

impl Default for BitmapPanelTab {
    fn default() -> Self {
        Self::Fields
    }
}

/// Background fill behind the bitmap preview image. Helps judge alpha edges
/// against light/dark/saturated backdrops.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum BitmapPreviewBg {
    DarkGray,
    Black,
    White,
    Magenta,
}

impl BitmapPreviewBg {
    pub(in crate::app) const ALL: [Self; 4] =
        [Self::DarkGray, Self::Black, Self::White, Self::Magenta];

    pub(in crate::app) fn color(self) -> egui::Color32 {
        match self {
            Self::DarkGray => egui::Color32::from_rgb(64, 64, 64),
            Self::Black => egui::Color32::BLACK,
            Self::White => egui::Color32::WHITE,
            Self::Magenta => egui::Color32::from_rgb(255, 0, 255),
        }
    }

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::DarkGray => "Dark gray",
            Self::Black => "Black",
            Self::White => "White",
            Self::Magenta => "Magenta",
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
    pub(in crate::app) decoded: Option<Result<BitmapPreviewData, String>>,
    pub(in crate::app) texture: Option<egui::TextureHandle>,
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
            decoded: None,
            texture: None,
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

/// One differing leaf field between two compared tags (Tag Diff).

/// Cross-frame model selection and camera state.
/// `loaded_key` prevents data and variant choices from being reused for a newly
/// selected document whose preview has not yet been resolved.
pub(in crate::app) struct ModelPreviewState {
    pub(in crate::app) loaded_key: Option<String>,
    pub(in crate::app) render_model_path: Option<String>,
    pub(in crate::app) data: Option<Result<ModelPreviewData, String>>,
    pub(in crate::app) active_tab: ModelTagPanelTab,
    pub(in crate::app) new_variant_name: String,
    pub(in crate::app) selected_variant: Option<usize>,
    pub(in crate::app) region_selections: HashMap<String, ModelRegionSelection>,
    pub(in crate::app) show_markers: bool,
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
    /// Sample the model's own shader textures rather than the flat per-material
    /// palette. Off falls back to the untextured view, which stays useful for
    /// reading silhouette and topology.
    pub(in crate::app) shaded: bool,
    /// A texture-resolve job is running for the loaded model.
    pub(in crate::app) textures_pending: bool,
    /// `.model` overlays only: draw the render model itself. Off leaves just
    /// the collision/physics layers on screen — a frame-level filter, so
    /// toggling never rebuilds geometry.
    pub(in crate::app) show_render: bool,
    /// Perspective projection instead of the default orthographic one. The
    /// eye sits two (focus-grown) radii out along the view axis, scaled so
    /// the focus plane matches the orthographic framing exactly — toggling
    /// never jumps, and the zoom slider keeps one meaning in both.
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
            render_model_path: None,
            data: None,
            active_tab: ModelTagPanelTab::Fields,
            new_variant_name: String::new(),
            selected_variant: None,
            region_selections: HashMap::new(),
            show_markers: false,
            marker_filter: String::new(),
            high_detail: true,
            loaded_high_detail: false,
            show_collision: false,
            show_physics: false,
            overlays_pending: false,
            overlays_loaded: false,
            scenario_bsp_selection: std::collections::BTreeSet::new(),
            loaded_scenario_selection: std::collections::BTreeSet::new(),
            render_mode: ModelRenderMode::Shaded,
            show_backfaces: false,
            shaded: true,
            textures_pending: false,
            show_render: true,
            perspective: false,
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
    /// Whether the cached preview no longer matches what the panel should
    /// show — a different tag, or a load-affecting setting that changed.
    /// One predicate for the panel's spinner gate and the loader's early
    /// return, so the two can never disagree about what needs a rebuild.
    pub(in crate::app) fn needs_preview_load(&self, entry_key: &str) -> bool {
        self.loaded_key.as_deref() != Some(entry_key)
            || self.data.is_none()
            || self.loaded_high_detail != self.high_detail
            || self.loaded_scenario_selection != self.scenario_bsp_selection
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ModelRenderMode {
    Shaded,
    Wireframe,
    ShadedWireframe,
}

impl ModelRenderMode {
    pub(in crate::app) const ALL: [Self; 3] =
        [Self::Shaded, Self::Wireframe, Self::ShadedWireframe];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Shaded => "Shaded",
            Self::Wireframe => "Wireframe",
            Self::ShadedWireframe => "Shaded + Wireframe",
        }
    }

    pub(in crate::app) fn draws_shading(self) -> bool {
        matches!(self, Self::Shaded | Self::ShadedWireframe)
    }

    pub(in crate::app) fn draws_wireframe(self) -> bool {
        matches!(self, Self::Wireframe | Self::ShadedWireframe)
    }
}

#[cfg(test)]
mod model_render_mode_tests {
    use super::*;

    #[test]
    fn model_render_modes_select_expected_passes() {
        assert!(ModelRenderMode::Shaded.draws_shading());
        assert!(!ModelRenderMode::Shaded.draws_wireframe());

        assert!(!ModelRenderMode::Wireframe.draws_shading());
        assert!(ModelRenderMode::Wireframe.draws_wireframe());

        assert!(ModelRenderMode::ShadedWireframe.draws_shading());
        assert!(ModelRenderMode::ShadedWireframe.draws_wireframe());
        assert_eq!(
            ModelPreviewState::default().render_mode,
            ModelRenderMode::Shaded
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ModelTagPanelTab {
    Fields,
    RenderModel,
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct ModelRegionSelection {
    pub(in crate::app) enabled: bool,
    pub(in crate::app) permutation: String,
}

#[derive(Clone)]
/// Source geometry and resolved variants used by the shared GPU renderer.
pub(in crate::app) struct ModelPreviewData {
    pub(in crate::app) source_key: String,
    pub(in crate::app) render_model_path: String,
    /// Shared so the deferred Glow callback can retain the immutable source
    /// geometry without cloning dense vertex/index buffers every frame.
    pub(in crate::app) preview: std::sync::Arc<RenderModelPreview>,
    /// Monotonic identity used by the shared GL upload cache. Unlike a pointer,
    /// it cannot be accidentally reused after a preview reload.
    pub(in crate::app) geometry_id: u64,
    /// One entry per `RenderModelPreview::materials`, once the worker has
    /// resolved them. `None` while that job is still running — the panel shows
    /// its spinner rather than drawing the model untextured, so a model never
    /// appears half-shaded and then changes under the cursor.
    pub(in crate::app) textures: Option<std::sync::Arc<Vec<MaterialTextures>>>,
    pub(in crate::app) variants: Vec<ModelVariantPreview>,
    /// Scenario tags only: the `structure bsps` block's references (one per
    /// element, `None` where the reference is empty), driving the per-BSP
    /// checkbox list. Empty for every other tag group.
    pub(in crate::app) scenario_bsps: Vec<Option<String>>,
    /// `.model` tags only: the linked animation graph's animation list, once
    /// the worker has read it. `None` elsewhere and before it lands.
    pub(in crate::app) animations: Option<std::sync::Arc<Vec<PreviewAnimationEntry>>>,
}

#[derive(Clone)]
/// Resolved model variant with an explicit removed-vs-inherited region boundary.
pub(in crate::app) struct ModelVariantPreview {
    pub(in crate::app) name: String,
    /// Region name → resolved permutation (own perm or parent-inherited).
    pub(in crate::app) regions: HashMap<String, String>,
    /// Region names the variant's block LISTS at all — including ones listed with
    /// an empty permutation (which means "explicitly removed", e.g. spec-ops elite
    /// has no helmet). A region NOT in this set is simply uncustomised and falls
    /// back to its base permutation (e.g. major elite helmet → base), rather than
    /// being hidden. Distinguishes "removed" from "not customised".
    pub(in crate::app) listed_regions: std::collections::HashSet<String>,
}
