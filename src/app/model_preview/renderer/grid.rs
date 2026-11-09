//! The ground grid and the theme colours behind it.

use super::*;

/// Ground-reference grid on the z = 0 plane (the world ground in Halo's
/// Z-up space), Blender-style: minor lines at a spacing picked from the
/// model's size, plus the world X and Y axes across the same extent. Returns
/// the line-list vertices and the index where the axis vertices begin —
/// minor lines first, then two vertices of the X axis, then two of the Y
/// axis — so the axes can draw in their own colors.
pub(super) fn grid_line_vertices(
    preview: &RenderModelPreview,
) -> (Vec<RenderModelPreviewVertex>, usize) {
    let (bounds_center, radius) = preview_center_radius(preview);
    // One Halo world unit is ten feet; a biped is a third of one. Fine
    // spacing for hand-held things, world units for vehicles, and coarser
    // tiers so a BSP is not buried under thousands of lines.
    let spacing = if radius <= 1.5 {
        0.1
    } else if radius <= 15.0 {
        1.0
    } else if radius <= 150.0 {
        10.0
    } else {
        100.0
    };
    const HALF_CELLS: i32 = 16;
    let snap = |value: f32| (value / spacing).round() * spacing;
    let center = [snap(bounds_center[0]), snap(bounds_center[1])];
    let extent = HALF_CELLS as f32 * spacing;
    let (min_x, max_x) = (center[0] - extent, center[0] + extent);
    let (min_y, max_y) = (center[1] - extent, center[1] + extent);
    let line = |from: [f32; 2], to: [f32; 2]| {
        [from, to].map(|[x, y]| RenderModelPreviewVertex {
            position: [x, y, 0.0],
            normal: [0.0, 0.0, 1.0],
            ..Default::default()
        })
    };
    let mut vertices = Vec::with_capacity((HALF_CELLS as usize * 2 + 1) * 4 + 4);
    for step in -HALF_CELLS..=HALF_CELLS {
        let offset = step as f32 * spacing;
        vertices.extend(line(
            [center[0] + offset, min_y],
            [center[0] + offset, max_y],
        ));
        vertices.extend(line(
            [min_x, center[1] + offset],
            [max_x, center[1] + offset],
        ));
    }
    let axis_start = vertices.len();
    vertices.extend(line([min_x, 0.0], [max_x, 0.0]));
    vertices.extend(line([0.0, min_y], [0.0, max_y]));
    (vertices, axis_start)
}

/// The clear color of the preview, shared with the scene target.
/// The theme's panel color, so a preview sits in the UI around it.
pub(in crate::app::model_preview) fn preview_background(visuals: &egui::Visuals) -> [f32; 3] {
    let [r, g, b, _] = visuals.panel_fill.to_array();
    [r, g, b].map(|channel| channel as f32 / 255.0)
}

pub(super) fn background_is_dark(background: [f32; 3]) -> bool {
    background.iter().sum::<f32>() / 3.0 < 0.5
}

/// Minor grid lines: a step off the background, toward the contrast.
pub(super) fn grid_line_color(background: [f32; 3]) -> [f32; 3] {
    const STEP: f32 = 0.11;
    let step = if background_is_dark(background) {
        STEP
    } else {
        -STEP
    };
    background.map(|channel| (channel + step).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_background_is_the_theme_s_and_grid_lines_stay_visible_on_it() {
        for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
            let background = preview_background(&visuals);
            assert_eq!(background_is_dark(background), visuals.dark_mode);
            let line = grid_line_color(background);
            let (line, background) = (line[0], background[0]);
            assert!((line - background).abs() > 0.05);
            assert_eq!(line > background, visuals.dark_mode);
        }
    }

    /// The grid is a flat z = 0 line list with no skinning influences (a
    /// stray weight would let an animated draw warp it), snapped to world
    /// spacing, with the two axis lines appended last for their own colors.
    #[test]
    fn ground_grid_lies_flat_unskinned_and_ends_with_the_axes() {
        let preview = RenderModelPreview {
            bounds_min: [-0.3, -0.25, 0.0],
            bounds_max: [0.31, 0.25, 0.65],
            ..Default::default()
        };
        let (vertices, axis_start) = grid_line_vertices(&preview);
        assert_eq!(vertices.len(), axis_start + 4, "two axis lines at the end");
        assert_eq!(vertices.len() % 2, 0, "a line list pairs up");
        assert!(axis_start > 0, "no minor lines");
        for vertex in &vertices {
            assert_eq!(vertex.position[2], 0.0, "grid left the ground plane");
            assert_eq!(vertex.normal, [0.0, 0.0, 1.0]);
            assert_eq!(vertex.node_weights, [0.0; 4], "grid must not skin");
        }
        // Biped-sized bounds pick the 0.1-unit tier; every line then sits on
        // a multiple of it.
        for vertex in &vertices[..axis_start] {
            for value in [vertex.position[0], vertex.position[1]] {
                let snapped = (value / 0.1).round() * 0.1;
                assert!(
                    (value - snapped).abs() < 1e-4,
                    "line off the 0.1 spacing: {value}"
                );
            }
        }
        // The axis lines really are the world axes.
        assert_eq!(vertices[axis_start].position[1], 0.0);
        assert_eq!(vertices[axis_start + 1].position[1], 0.0);
        assert_eq!(vertices[axis_start + 2].position[0], 0.0);
        assert_eq!(vertices[axis_start + 3].position[0], 0.0);
    }
}
