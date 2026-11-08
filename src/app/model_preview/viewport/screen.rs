//! Screen-space helpers for the overlays: polygons, segment distance and the hover label.

use super::*;

pub(in crate::app::model_preview) fn polygon_edges(polygon: &[egui::Pos2]) -> impl Iterator<Item = (&egui::Pos2, &egui::Pos2)> {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
}

pub(in crate::app::model_preview) fn point_in_polygon(point: egui::Pos2, polygon: &[egui::Pos2]) -> bool {
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for &current in polygon {
        if (current.y > point.y) != (previous.y > point.y)
            && point.x
                < (previous.x - current.x) * (point.y - current.y) / (previous.y - current.y)
                    + current.x
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

pub(in crate::app::model_preview) fn polygon_area_twice(polygon: &[egui::Pos2]) -> f32 {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum()
}

pub(in crate::app::model_preview) fn draw_preview_hover_label(
    painter: &egui::Painter,
    text_pos: egui::Pos2,
    text: &str,
    color: Color32,
) {
    let label_rect =
        egui::Rect::from_min_size(text_pos, Vec2::new(text.len() as f32 * 6.0 + 8.0, 16.0));
    painter.rect_filled(
        label_rect,
        2.0,
        Color32::from_rgba_unmultiplied(0, 0, 0, 180),
    );
    painter.text(
        text_pos + Vec2::new(4.0, 1.0),
        Align2::LEFT_TOP,
        text,
        FontId::proportional(10.0),
        color,
    );
}

pub(in crate::app::model_preview) fn point_segment_distance(point: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let ap = point - a;
    let denom = ab.dot(ab);
    if denom <= f32::EPSILON {
        return screen_edge_length(point, a);
    }
    let t = (ap.dot(ab) / denom).clamp(0.0, 1.0);
    screen_edge_length(point, a + ab * t)
}

pub(in crate::app::model_preview) fn screen_edge_length(a: egui::Pos2, b: egui::Pos2) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}
