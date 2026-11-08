//! The bitmap preview's state: the tab it shows, its background and view
//! settings, and the decode it holds.

use super::*;

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
