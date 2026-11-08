//! The import-tool error overlay: each primitive drawn in its colour, and the hover that names it.

use super::*;

fn model_error_color(error: &ModelErrorPrimitive) -> Color32 {
    Color32::from_rgba_unmultiplied(
        error.color[0],
        error.color[1],
        error.color[2],
        error.color[3],
    )
}

fn model_error_fill(error: &ModelErrorPrimitive) -> Color32 {
    Color32::from_rgba_unmultiplied(
        error.color[0],
        error.color[1],
        error.color[2],
        ((error.color[3] as u16 * 72) / 255) as u8,
    )
}

pub(in crate::app::model_preview) fn draw_model_errors(
    ui: &Ui,
    painter: &egui::Painter,
    response: &egui::Response,
    camera: &PreviewCamera,
    errors: &[ModelErrorPrimitive],
    state: &ModelPreviewState,
    skinning_rows: Option<&[[f32; 4]]>,
) {
    let pointer = response
        .hovered()
        .then(|| ui.input(|input| input.pointer.hover_pos()))
        .flatten();
    let mut hover = ErrorHoverState::new(pointer);

    for error in errors {
        if !model_layer_visible(state, error.layer)
            || (error.non_critical && !state.show_non_critical_errors)
        {
            continue;
        }
        let color = model_error_color(error);
        let label_color = Color32::from_rgb(error.color[0], error.color[1], error.color[2]);
        draw_model_error_shape(
            painter,
            camera,
            error,
            color,
            label_color,
            skinning_rows,
            &mut hover,
        );
    }

    if let Some(hovered) = hover.closest {
        draw_preview_hover_label(
            painter,
            hovered.position + Vec2::new(8.0, -20.0),
            hovered.label,
            hovered.color,
        );
    }
}

#[derive(Clone, Copy)]
struct ErrorHover<'a> {
    distance: f32,
    position: egui::Pos2,
    label: &'a str,
    color: Color32,
}

struct ErrorHoverState<'a> {
    pointer: Option<egui::Pos2>,
    closest: Option<ErrorHover<'a>>,
}

impl<'a> ErrorHoverState<'a> {
    fn new(pointer: Option<egui::Pos2>) -> Self {
        Self {
            pointer,
            closest: None,
        }
    }

    fn consider(&mut self, distance: f32, position: egui::Pos2, label: &'a str, color: Color32) {
        if self
            .closest
            .is_none_or(|closest| distance < closest.distance)
        {
            self.closest = Some(ErrorHover {
                distance,
                position,
                label,
                color,
            });
        }
    }

    fn consider_point(
        &mut self,
        position: egui::Pos2,
        radius: f32,
        label: &'a str,
        color: Color32,
    ) {
        let Some(pointer) = self.pointer else { return };
        let distance = screen_edge_length(pointer, position);
        if distance <= radius {
            self.consider(distance, position, label, color);
        }
    }

    fn consider_segment(
        &mut self,
        start: egui::Pos2,
        end: egui::Pos2,
        label: &'a str,
        color: Color32,
    ) {
        let Some(pointer) = self.pointer else { return };
        let distance = point_segment_distance(pointer, start, end);
        if distance <= 7.0 {
            self.consider(distance, start, label, color);
        }
    }
}

fn draw_model_error_shape<'a>(
    painter: &egui::Painter,
    camera: &PreviewCamera,
    error: &'a ModelErrorPrimitive,
    color: Color32,
    label_color: Color32,
    skinning_rows: Option<&[[f32; 4]]>,
    hover: &mut ErrorHoverState<'a>,
) {
    match &error.shape {
        ModelErrorShape::Point(point) => {
            let Some(position) = project_error_point(camera, point, skinning_rows) else {
                return;
            };
            painter.circle_filled(position, 4.5, color);
            painter.circle_stroke(position, 5.5, Stroke::new(1.25_f32, Color32::WHITE));
            hover.consider_point(position, 8.0, &error.label, label_color);
        }
        ModelErrorShape::Vector {
            point,
            normal,
            length,
        } => {
            let start_world = animated_error_point(point, skinning_rows);
            let end_world = [
                start_world[0] + normal[0] * length,
                start_world[1] + normal[1] * length,
                start_world[2] + normal[2] * length,
            ];
            let (start, end) = (camera.project(start_world), camera.project(end_world));
            if !start.in_front || !end.in_front {
                return;
            }
            let (start, end) = (start.pos, end.pos);
            painter.line_segment([start, end], Stroke::new(3.0_f32, color));
            hover.consider_segment(start, end, &error.label, label_color);
        }
        ModelErrorShape::Polyline(points) => {
            let Some(projected) = project_error_points(camera, points, skinning_rows) else {
                return;
            };
            for pair in projected.windows(2) {
                painter.line_segment([pair[0], pair[1]], Stroke::new(3.0_f32, color));
                hover.consider_segment(pair[0], pair[1], &error.label, label_color);
            }
        }
        ModelErrorShape::Face(points) => {
            draw_model_error_face(
                painter,
                camera,
                error,
                points,
                color,
                label_color,
                skinning_rows,
                hover,
            );
        }
    }
}

fn draw_model_error_face<'a>(
    painter: &egui::Painter,
    camera: &PreviewCamera,
    error: &'a ModelErrorPrimitive,
    points: &[ModelErrorPoint],
    color: Color32,
    label_color: Color32,
    skinning_rows: Option<&[[f32; 4]]>,
    hover: &mut ErrorHoverState<'a>,
) {
    let Some(projected) = project_error_points(camera, points, skinning_rows) else {
        return;
    };
    if projected.len() < 3 {
        return;
    }

    // egui's stroked convex-polygon tessellator forms mitered joins. When a
    // face is viewed exactly edge-on, its screen-space area approaches zero
    // and those joins can shoot far beyond the viewport. Fill only
    // non-degenerate projections, then draw every bounded edge separately.
    if polygon_area_twice(&projected).abs() > 0.5 {
        painter.add(egui::Shape::convex_polygon(
            projected.clone(),
            model_error_fill(error),
            Stroke::NONE,
        ));
    }
    for (&start, &end) in polygon_edges(&projected) {
        painter.line_segment([start, end], Stroke::new(2.0_f32, color));
    }

    let Some(pointer) = hover.pointer else { return };
    let edge_distance = polygon_edges(&projected)
        .map(|(&start, &end)| point_segment_distance(pointer, start, end))
        .fold(f32::INFINITY, f32::min);
    let distance = if point_in_polygon(pointer, &projected) {
        0.0
    } else {
        edge_distance
    };
    if distance <= 7.0 {
        hover.consider(distance, pointer, &error.label, label_color);
    }
}

/// `None` behind the perspective eye, where an overlay cannot be drawn.
fn project_error_point(
    camera: &PreviewCamera,
    point: &ModelErrorPoint,
    skinning_rows: Option<&[[f32; 4]]>,
) -> Option<egui::Pos2> {
    let projected = camera.project(animated_error_point(point, skinning_rows));
    projected.in_front.then_some(projected.pos)
}

/// `None` when any point is behind the perspective eye: a shape that crosses
/// the near plane has no faithful 2D outline.
fn project_error_points(
    camera: &PreviewCamera,
    points: &[ModelErrorPoint],
    skinning_rows: Option<&[[f32; 4]]>,
) -> Option<Vec<egui::Pos2>> {
    points
        .iter()
        .map(|point| project_error_point(camera, point, skinning_rows))
        .collect()
}

fn animated_error_point(point: &ModelErrorPoint, skinning_rows: Option<&[[f32; 4]]>) -> [f32; 3] {
    let Some(rows) = skinning_rows else {
        return point.position;
    };
    // As the vertex shader does: by the weights of the nodes that exist,
    // over their sum.
    let mut result = [0.0; 3];
    let mut applied = 0.0;
    for (&index, &weight) in point.node_indices.iter().zip(&point.node_weights) {
        if index < 0 || weight <= 0.0 {
            continue;
        }
        let start = index as usize * 3;
        let Some(matrix) = rows.get(start..start + 3) else {
            continue;
        };
        applied += weight;
        for axis in 0..3 {
            result[axis] += weight
                * (matrix[axis][0] * point.position[0]
                    + matrix[axis][1] * point.position[1]
                    + matrix[axis][2] * point.position[2]
                    + matrix[axis][3]);
        }
    }
    if applied <= f32::EPSILON {
        return point.position;
    }
    result.map(|axis| axis / applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_face_hover_detects_inside_and_outside_points() {
        let face = [
            egui::pos2(10.0, 10.0),
            egui::pos2(50.0, 10.0),
            egui::pos2(50.0, 40.0),
            egui::pos2(10.0, 40.0),
        ];
        assert!(point_in_polygon(egui::pos2(30.0, 25.0), &face));
        assert!(!point_in_polygon(egui::pos2(60.0, 25.0), &face));
        assert!(polygon_area_twice(&face).abs() > 0.5);

        let edge_on = [
            egui::pos2(10.0, 20.0),
            egui::pos2(30.0, 20.0),
            egui::pos2(50.0, 20.0),
        ];
        assert_eq!(polygon_area_twice(&edge_on), 0.0);
    }

    #[test]
    fn error_overlay_uses_the_primitives_authored_color() {
        let error = ModelErrorPrimitive {
            label: "open edge".to_owned(),
            non_critical: false,
            color: [12, 34, 56, 78],
            layer: ModelPreviewLayer::Collision,
            shape: ModelErrorShape::Point(ModelErrorPoint {
                position: [0.0; 3],
                node_indices: [-1; 4],
                node_weights: [0.0; 4],
            }),
        };
        assert_eq!(
            model_error_color(&error),
            Color32::from_rgba_unmultiplied(12, 34, 56, 78)
        );
        assert_eq!(
            model_error_fill(&error),
            Color32::from_rgba_unmultiplied(12, 34, 56, 22)
        );
    }
}
