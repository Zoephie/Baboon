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

/// Cross-frame model selection, camera, and projected-geometry cache.
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
    pub(in crate::app) projected_triangles: Vec<ModelProjectedTriangle>,
    pub(in crate::app) show_markers: bool,
    /// Case-insensitive substring filter on marker names (empty = show all).
    /// Only applied while `show_markers` is on.
    pub(in crate::app) marker_filter: String,
    pub(in crate::app) render_mode: ModelRenderMode,
    pub(in crate::app) show_backfaces: bool,
    pub(in crate::app) scale: f32,
    pub(in crate::app) yaw: f32,
    pub(in crate::app) pitch: f32,
    pub(in crate::app) pan: Vec2,
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
            projected_triangles: Vec::new(),
            show_markers: false,
            marker_filter: String::new(),
            render_mode: ModelRenderMode::Shaded,
            show_backfaces: false,
            scale: 1.0,
            yaw: -0.45,
            pitch: 0.25,
            pan: Vec2::ZERO,
        }
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
/// Source geometry and resolved variants used to rebuild projected triangles.
/// It remains independent of camera state so view changes do not reload tags.
pub(in crate::app) struct ModelPreviewData {
    pub(in crate::app) source_key: String,
    pub(in crate::app) render_model_path: String,
    pub(in crate::app) preview: RenderModelPreview,
    pub(in crate::app) draw_triangles: Vec<ModelSourceTriangle>,
    pub(in crate::app) variants: Vec<ModelVariantPreview>,
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

#[derive(Clone, Copy)]
pub(in crate::app) struct ModelSourceTriangle {
    pub(in crate::app) batch_index: usize,
    pub(in crate::app) positions: [[f32; 3]; 3],
    pub(in crate::app) normals: [[f32; 3]; 3],
    pub(in crate::app) fill: Color32,
}

pub(in crate::app) struct ModelProjectedTriangle {
    pub(in crate::app) points: [egui::Pos2; 3],
    pub(in crate::app) depth: f32,
    pub(in crate::app) fills: [Color32; 3],
}
