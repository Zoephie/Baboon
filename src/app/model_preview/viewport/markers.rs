//! Marker axes and the armature, drawn where the animated pose puts them.

use super::super::{PreviewCamera, RenderModelPreviewMarker};
use super::screen::{point_segment_distance, screen_edge_length};
use eframe::egui;
use eframe::egui::{Color32, Stroke, Vec2};

pub(in crate::app::model_preview) const ARMATURE_COLOR: Color32 = Color32::from_rgb(255, 204, 0);

pub(in crate::app::model_preview) const MARKER_TOOLTIP_COLOR: Color32 =
    Color32::from_rgb(120, 235, 255);

pub(in crate::app::model_preview) fn animated_marker_transform(
    marker: &RenderModelPreviewMarker,
    skinning_rows: Option<&[[f32; 4]]>,
) -> ([f32; 3], [[f32; 3]; 3]) {
    let Some(rows) = skinning_rows else {
        return (marker.position, marker.axes);
    };
    if marker.node_index < 0 {
        return (marker.position, marker.axes);
    }
    let start = marker.node_index as usize * 3;
    let Some(matrix) = rows.get(start..start + 3) else {
        return (marker.position, marker.axes);
    };
    let transform_point = |point: [f32; 3]| {
        matrix
            .iter()
            .map(|row| row[0] * point[0] + row[1] * point[1] + row[2] * point[2] + row[3])
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or(point)
    };
    let transform_axis = |axis: [f32; 3]| {
        let transformed: [f32; 3] = matrix
            .iter()
            .map(|row| row[0] * axis[0] + row[1] * axis[1] + row[2] * axis[2])
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or(axis);
        let length = transformed.iter().map(|v| v * v).sum::<f32>().sqrt();
        if length > 0.0001 {
            transformed.map(|v| v / length)
        } else {
            axis
        }
    };
    (
        transform_point(marker.position),
        marker.axes.map(transform_axis),
    )
}

pub(in crate::app::model_preview) const MARKER_AXIS_SCREEN_LENGTH: f32 = 15.0;

pub(in crate::app::model_preview) fn marker_axis_screen_deltas(
    camera: &PreviewCamera,
    axes: [[f32; 3]; 3],
) -> [Vec2; 3] {
    axes.map(|axis| {
        let view_axis = camera.rotate_vector(axis);
        let screen = Vec2::new(view_axis[0], -view_axis[2]);
        let len = screen.length();
        if len <= 0.001 {
            Vec2::new(0.0, -MARKER_AXIS_SCREEN_LENGTH * 0.45)
        } else {
            screen / len * MARKER_AXIS_SCREEN_LENGTH
        }
    })
}

pub(in crate::app::model_preview) fn draw_marker_axes(
    painter: &egui::Painter,
    origin: egui::Pos2,
    axis_deltas: [Vec2; 3],
) {
    let colors = [
        Color32::from_rgb(220, 35, 28),
        Color32::from_rgb(20, 180, 45),
        Color32::from_rgb(40, 85, 235),
    ];
    for (delta, color) in axis_deltas.into_iter().zip(colors) {
        let end = origin + delta;
        painter.line_segment(
            [origin, end],
            Stroke::new(2.5_f32, Color32::from_rgba_unmultiplied(0, 0, 0, 150)),
        );
        painter.line_segment([origin, end], Stroke::new(1.35_f32, color));
    }
}

pub(in crate::app::model_preview) fn marker_axes_hovered(
    pos: egui::Pos2,
    origin: egui::Pos2,
    axis_deltas: [Vec2; 3],
) -> bool {
    screen_edge_length(pos, origin) <= 7.0
        || axis_deltas
            .into_iter()
            .any(|delta| point_segment_distance(pos, origin, origin + delta) <= 5.0)
}

#[cfg(test)]
mod tests {
    use super::super::super::RenderModelPreviewMarker;
    use super::*;

    #[test]
    fn marker_transform_uses_its_animated_bone() {
        let marker = RenderModelPreviewMarker {
            name: "weapon".to_owned(),
            node_index: 1,
            position: [1.0, 2.0, 3.0],
            scale: 1.0,
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            only_under: None,
        };
        let rows = vec![
            [1.0, 0.0, 0.0, 99.0],
            [0.0, 1.0, 0.0, 99.0],
            [0.0, 0.0, 1.0, 99.0],
            [1.0, 0.0, 0.0, 0.5],
            [0.0, 1.0, 0.0, -0.25],
            [0.0, 0.0, 1.0, 1.0],
        ];
        let (position, axes) = animated_marker_transform(&marker, Some(&rows));
        assert_eq!(position, [1.5, 1.75, 4.0]);
        assert_eq!(axes, marker.axes);
    }
}
