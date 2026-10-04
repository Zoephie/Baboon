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
