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

/// The Sequences sidebar's width, in points, before it is dragged.
pub(in crate::app) const DEFAULT_BITMAP_SIDEBAR_WIDTH: f32 = 300.0;
/// The narrowest the sidebar is dragged to, and the widest it is saved at.
pub(in crate::app) const MIN_BITMAP_SIDEBAR_WIDTH: f32 = 220.0;
pub(in crate::app) const MAX_BITMAP_SIDEBAR_WIDTH: f32 = 800.0;
/// The sequence playback rate before it is changed, and its range, in frames
/// a second. The bitmap holds no rate of its own: whatever draws it sets one.
pub(in crate::app) const DEFAULT_SPRITE_PLAYBACK_FPS: u32 = 15;
pub(in crate::app) const MAX_SPRITE_PLAYBACK_FPS: u32 = 60;

/// View choices shared by every bitmap preview and persisted between sessions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) struct BitmapPreviewViewSettings {
    pub(in crate::app) bg: BitmapPreviewBg,
    pub(in crate::app) show_checkerboard: bool,
    pub(in crate::app) show_border: bool,
    /// The Sequences sidebar's width.
    pub(in crate::app) sidebar_width: f32,
    pub(in crate::app) sprites: SpriteOverlaySettings,
}

impl Default for BitmapPreviewViewSettings {
    fn default() -> Self {
        Self {
            bg: BitmapPreviewBg::DarkGray,
            show_checkerboard: true,
            show_border: true,
            sidebar_width: DEFAULT_BITMAP_SIDEBAR_WIDTH,
            sprites: SpriteOverlaySettings::default(),
        }
    }
}

/// What the canvas draws over a bitmap's sprites, and how a sequence plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct SpriteOverlaySettings {
    /// A faint outline round every sprite on the shown image.
    pub(in crate::app) show_all: bool,
    /// The selected sprite's rect less the tag's `sprite spacing`, as the
    /// game's HUD and antennas read it.
    pub(in crate::app) show_content: bool,
    /// A cross at the selected sprite's registration point.
    pub(in crate::app) show_registration: bool,
    /// A mark at the shown image's own `registration point`.
    pub(in crate::app) show_image_registration: bool,
    /// While a sequence plays, keep its registration point still on screen.
    pub(in crate::app) pin_registration: bool,
    pub(in crate::app) playback_fps: u32,
}

impl Default for SpriteOverlaySettings {
    fn default() -> Self {
        Self {
            show_all: true,
            show_content: false,
            show_registration: true,
            show_image_registration: false,
            pin_registration: true,
            playback_fps: DEFAULT_SPRITE_PLAYBACK_FPS,
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
    /// The image and mip `decoded` is of. When it is not the one selected,
    /// the caller decodes that one and shows this meanwhile.
    pub(in crate::app) decoded_for: Option<(usize, usize)>,
    /// The image and mip `decoding` is of.
    pub(in crate::app) decoding_for: Option<(usize, usize)>,
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
    /// The Sequences sidebar's width.
    pub(in crate::app) sidebar_width: f32,
    pub(in crate::app) sprite_view: SpriteOverlaySettings,
    /// The sequence or sprite picked in the sidebar or on the canvas.
    pub(in crate::app) sprite_selection: Option<SpriteSelection>,
    /// When the selected sequence started playing, if it is playing.
    pub(in crate::app) sprite_playback: Option<SpritePlayback>,
    /// The Sequences card's filter.
    pub(in crate::app) sequence_filter: String,
    /// The tag's sequences, read once for each revision of the document
    /// (its id and revision).
    pub(in crate::app) sequences: Option<((u64, u64), std::sync::Arc<blam_tags::bitmap::BitmapDefinition>)>,
    /// Asked from the Camera menu: zoom to the selection on the next frame.
    pub(in crate::app) frame_selection: bool,
    /// A sprite was picked on the canvas: the sidebar scrolls to its row.
    pub(in crate::app) scroll_to_selection: bool,
}

/// Which block a selection is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum SequenceBlock {
    /// `sequences[]`, what tool built and the game reads.
    Processed,
    /// `manual_sequences[]`, authored windows in repeat space.
    Manual,
}

/// A sequence, or one frame of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct SpriteSelection {
    pub(in crate::app) block: SequenceBlock,
    pub(in crate::app) sequence: usize,
    /// The sprite, or the frame of a sequence without sprites; none for the
    /// whole sequence.
    pub(in crate::app) frame: Option<usize>,
}

/// A sequence playing from `frame` at `started`, in seconds of UI time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) struct SpritePlayback {
    pub(in crate::app) started: f64,
    pub(in crate::app) frame: usize,
}

impl Default for BitmapPreviewState {
    fn default() -> Self {
        Self {
            active_tab: BitmapPanelTab::Fields,
            show_red: true,
            show_green: true,
            show_blue: true,
            show_alpha: false,
            show_checkerboard: true,
            show_border: true,
            decoded: None,
            decoding: None,
            decoded_for: None,
            decoding_for: None,
            texture: None,
            checker_texture: None,
            texture_dirty: true,
            zoom: 1.0,
            pan: Vec2::ZERO,
            zoom_initialized: false,
            bg: BitmapPreviewBg::DarkGray,
            image_index: 0,
            mip_index: 0,
            sidebar_width: DEFAULT_BITMAP_SIDEBAR_WIDTH,
            sprite_view: SpriteOverlaySettings::default(),
            sprite_selection: None,
            sprite_playback: None,
            sequence_filter: String::new(),
            sequences: None,
            frame_selection: false,
            scroll_to_selection: false,
        }
    }
}

impl BitmapPreviewState {
    pub(in crate::app) fn apply_view_settings(&mut self, settings: BitmapPreviewViewSettings) {
        self.bg = settings.bg;
        self.show_checkerboard = settings.show_checkerboard;
        self.show_border = settings.show_border;
        self.sidebar_width = settings.sidebar_width;
        self.sprite_view = settings.sprites;
    }

    pub(in crate::app) fn view_settings(&self) -> BitmapPreviewViewSettings {
        BitmapPreviewViewSettings {
            bg: self.bg,
            show_checkerboard: self.show_checkerboard,
            show_border: self.show_border,
            sidebar_width: self.sidebar_width,
            sprites: self.sprite_view,
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
