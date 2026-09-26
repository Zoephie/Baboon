//! Shared illustrated loading state for content-sized waits.

use super::*;
use std::sync::OnceLock;

const LOADING_SPINNER_SIZE: f32 = 256.0;
const LOADING_TEXT_SPACE: f32 = 64.0;
const LOADING_REVOLUTION_SECONDS: f32 = 4.0;
/// Rotation resamples an already-rasterized SVG texture. Rendering above the
/// display resolution first keeps curves and one-pixel lines clean while they
/// move, especially on fractional Windows display scales such as 150%.
const LOADING_RASTER_SCALE: f32 = 2.0;

struct LoadingPalette {
    primary: &'static str,
    secondary: &'static str,
    background_opacity: &'static str,
    lines: &'static str,
}

const DARK_LOADING_PALETTE: LoadingPalette = LoadingPalette {
    primary: "#BFBFBF",
    secondary: "#808080",
    background_opacity: "0.4",
    lines: "#404040",
};

const LIGHT_LOADING_PALETTE: LoadingPalette = LoadingPalette {
    primary: "#333333",
    secondary: "#4D4D4D",
    background_opacity: "0.25",
    lines: "#999999",
};

const LOADING_BACKGROUND_SVG: &str = include_str!("../../../assets/loading/loading-background.svg");
const LOADING_INNER_SVG: &str = include_str!("../../../assets/loading/loading-inner.svg");
const LOADING_OUTER_SVG: &str = include_str!("../../../assets/loading/loading-outer.svg");

static LIGHT_BACKGROUND_SVG: OnceLock<String> = OnceLock::new();
static LIGHT_INNER_SVG: OnceLock<String> = OnceLock::new();
static LIGHT_OUTER_SVG: OnceLock<String> = OnceLock::new();

/// Replaces the dark palette stored in the source artwork with its light-theme
/// counterparts. The semantic classes in the SVGs document which artwork uses
/// each token; the literal values keep the files previewable outside Baboon.
fn light_loading_svg(source: &str) -> String {
    source
        .replace(DARK_LOADING_PALETTE.primary, LIGHT_LOADING_PALETTE.primary)
        .replace(
            DARK_LOADING_PALETTE.secondary,
            LIGHT_LOADING_PALETTE.secondary,
        )
        .replace(
            &format!(
                "fill-opacity=\"{}\"",
                DARK_LOADING_PALETTE.background_opacity
            ),
            &format!(
                "fill-opacity=\"{}\"",
                LIGHT_LOADING_PALETTE.background_opacity
            ),
        )
        .replace(DARK_LOADING_PALETTE.lines, LIGHT_LOADING_PALETTE.lines)
}

fn themed_svg(
    dark_mode: bool,
    source: &'static str,
    light_source: &'static OnceLock<String>,
) -> &'static [u8] {
    if dark_mode {
        source.as_bytes()
    } else {
        light_source
            .get_or_init(|| light_loading_svg(source))
            .as_bytes()
    }
}

/// The pixel size to rasterize artwork drawn `size` points wide at.
///
/// Rounded up to a quarter-octave step rather than used exactly. egui keeps
/// every (uri, size) rasterization for the life of the process, and makes it
/// on the UI thread, so an exact size gave every thumbnail-slider position and
/// every resize of a small pane its own pair of SVG renders and textures —
/// never freed. The ladder bounds that to a handful per part, and oversamples
/// by between 2x and 2.4x instead of exactly 2x.
fn loading_raster_pixels(size: f32, pixels_per_point: f32) -> u32 {
    let needed = (size * pixels_per_point * LOADING_RASTER_SCALE).max(1.0);
    let quarter_octaves = (needed.log2() * 4.0).ceil();
    2.0_f32.powf(quarter_octaves / 4.0).ceil() as u32
}

fn loading_image_uri(part: &str, theme: &str, raster_pixels: u32, pixels_per_point: f32) -> String {
    let dpi = (pixels_per_point * 100.0).round().max(1.0) as u32;
    format!("bytes://baboon_loading/{part}-{theme}-dpi{dpi}-{raster_pixels}px.svg")
}

fn paint_loading_art(ui: &Ui, rect: egui::Rect, include_background: bool) {
    let dark_mode = ui.visuals().dark_mode;
    let theme = if dark_mode { "dark" } else { "light" };
    let size = rect.width().min(rect.height());
    let pixels_per_point = ui.ctx().pixels_per_point();
    let raster_pixels = loading_raster_pixels(size, pixels_per_point);
    let raster_size = Vec2::splat(raster_pixels as f32 / pixels_per_point);

    if include_background {
        egui::Image::from_bytes(
            loading_image_uri("background", theme, raster_pixels, pixels_per_point),
            themed_svg(dark_mode, LOADING_BACKGROUND_SVG, &LIGHT_BACKGROUND_SVG),
        )
        .fit_to_exact_size(raster_size)
        .paint_at(ui, rect);
    }

    let seconds = ui.input(|input| input.time) as f32;
    let angle = (seconds * std::f32::consts::TAU / LOADING_REVOLUTION_SECONDS)
        .rem_euclid(std::f32::consts::TAU);
    let center = Vec2::splat(0.5);

    egui::Image::from_bytes(
        loading_image_uri("outer", theme, raster_pixels, pixels_per_point),
        themed_svg(dark_mode, LOADING_OUTER_SVG, &LIGHT_OUTER_SVG),
    )
    .fit_to_exact_size(raster_size)
    .rotate(angle, center)
    .paint_at(ui, rect);

    egui::Image::from_bytes(
        loading_image_uri("inner", theme, raster_pixels, pixels_per_point),
        themed_svg(dark_mode, LOADING_INNER_SVG, &LIGHT_INNER_SVG),
    )
    .fit_to_exact_size(raster_size)
    .rotate(-angle, center)
    .paint_at(ui, rect);

    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(16));
}

fn paint_loading_spinner(ui: &mut Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_loading_art(ui, rect, true);
}

/// Paint only the counter-rotating rings, centered inside an existing preview
/// frame. The transparent 256-point artwork scales down for smaller thumbnails.
pub(in crate::app) fn paint_loading_rings(ui: &Ui, container: egui::Rect) {
    paint_loading_rings_sized(ui, container, LOADING_SPINNER_SIZE);
}

/// Paint the rings-only indicator with an explicit maximum size. Sidebars use
/// a smaller ceiling than previews so the activity reads as secondary to the
/// full illustrated loading state in the main canvas.
pub(in crate::app) fn paint_loading_rings_sized(
    ui: &Ui,
    container: egui::Rect,
    max_size: f32,
) {
    let size = max_size.min(container.width()).min(container.height());
    if size <= 0.0 {
        return;
    }

    let rect = egui::Rect::from_center_size(container.center(), Vec2::splat(size));
    paint_loading_art(ui, rect, false);
}

/// Draw a loading illustration and its copy as one centered content group.
/// This is intended for a pane or viewport, not compact toolbar activity.
pub(in crate::app) fn centered_loading_state(ui: &mut Ui, title: &str, detail: &str) {
    let available = ui.available_size();
    let spinner_size = LOADING_SPINNER_SIZE
        .min(available.x)
        .min((available.y - LOADING_TEXT_SPACE).max(1.0));
    let content_height = spinner_size + LOADING_TEXT_SPACE;

    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.add_space(((available.y - content_height) * 0.5).max(0.0));
        paint_loading_spinner(ui, spinner_size);
        ui.heading(RichText::new(title).color(text_dark()).strong().italics());
        ui.label(RichText::new(detail).color(subtle_dark()));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_palette_replaces_each_dark_loading_token() {
        let dark = format!("{LOADING_BACKGROUND_SVG}{LOADING_INNER_SVG}{LOADING_OUTER_SVG}");
        for class in [
            "loading-primary",
            "loading-secondary",
            "loading-background",
            "loading-lines",
        ] {
            assert!(dark.contains(class), "missing semantic class {class}");
        }

        let combined = light_loading_svg(&dark);

        for expected in ["#333333", "#4D4D4D", "#999999", "fill-opacity=\"0.25\""] {
            assert!(combined.contains(expected), "missing {expected}");
        }
        for dark in ["#BFBFBF", "#808080", "#404040", "fill-opacity=\"0.4\""] {
            assert!(!combined.contains(dark), "left dark token {dark}");
        }
    }

    #[test]
    fn loading_texture_key_tracks_fractional_display_scale() {
        let pixels = loading_raster_pixels(256.0, 1.5);
        assert_eq!(
            loading_image_uri("outer", "dark", pixels, 1.5),
            "bytes://baboon_loading/outer-dark-dpi150-862px.svg"
        );
    }

    /// Every size the artwork is drawn at — thumbnail cells from the slider's
    /// whole range, panes of any height — lands on a few rasterizations, each
    /// at least twice the display resolution.
    #[test]
    fn loading_rasters_are_bounded_and_oversampled() {
        for pixels_per_point in [1.0, 1.25, 1.5, 2.0, 3.0] {
            let mut rasters = std::collections::BTreeSet::new();
            for quarter_points in 4..=(LOADING_SPINNER_SIZE as u32 * 4) {
                let size = quarter_points as f32 / 4.0;
                let pixels = loading_raster_pixels(size, pixels_per_point);
                let needed = size * pixels_per_point * LOADING_RASTER_SCALE;
                assert!(pixels as f32 >= needed, "{size}pt @{pixels_per_point}x undersampled");
                assert!(pixels as f32 <= needed * 1.2 + 1.0, "{size}pt oversampled to {pixels}px");
                rasters.insert(pixels);
            }
            assert!(
                rasters.len() <= 36,
                "{} rasterizations at {pixels_per_point}x",
                rasters.len()
            );
        }
    }
}
