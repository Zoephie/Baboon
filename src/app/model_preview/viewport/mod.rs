//! The egui widget over the GL picture: input, the camera it moves, and the
//! overlays drawn on top (error reports, markers, the armature).

use super::*;

mod errors;
pub(in crate::app::model_preview) use errors::*;
mod markers;
pub(in crate::app::model_preview) use markers::*;
mod screen;
pub(in crate::app::model_preview) use screen::*;

pub(in crate::app::model_preview) fn draw_model_viewport(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    desired_size: Vec2,
) {
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    // Pixels → world units at the current zoom, for the pan and zoom-to-cursor
    // math below. Uses the same bounds-derived fit as `PreviewCamera`.
    let (_, radius) = preview_center_radius(&data.preview);
    let fit = rect.width().min(rect.height()) / (radius * 2.2).max(0.001);
    let world_per_pixel = 1.0 / (fit * state.scale).max(0.0001);

    // Panning moves the world-space orbit point, not a screen offset: the
    // camera then orbits and zooms around whatever the user framed, which is
    // what makes a corner of a BSP inspectable. Screen-space panning kept the
    // orbit pivot at the model's center, so orbiting a panned view swung the
    // framed geometry away.
    let pan_world = |state: &mut ModelPreviewState, delta: Vec2| {
        let moved = unrotate_view_vector(
            state.yaw,
            state.pitch,
            [-delta.x * world_per_pixel, 0.0, delta.y * world_per_pixel],
        );
        state.focus[0] += moved[0];
        state.focus[1] += moved[1];
        state.focus[2] += moved[2];
    };
    if response.dragged_by(egui::PointerButton::Middle) {
        pan_world(state, response.drag_delta());
    } else if response.dragged_by(egui::PointerButton::Primary) {
        let delta = response.drag_delta();
        if ui.input(|i| i.modifiers.shift) {
            pan_world(state, delta);
        } else {
            state.yaw += delta.x * 0.01;
            state.pitch = (state.pitch + delta.y * 0.01).clamp(-1.45, 1.45);
        }
    }
    if let Some(scroll) = viewport_wheel_zoom(ui, &response) {
        let least = min_preview_scale(&data.preview);
        let old_scale = state.scale.clamp(least, MAX_PREVIEW_SCALE);
        let new_scale =
            (old_scale * (scroll / 450.0).exp()).clamp(least, MAX_PREVIEW_SCALE);
        // Zoom toward the cursor: shift the orbit point so the geometry
        // under the pointer stays put. Without this, zooming into a BSP
        // always dives at its center and the doorway drifts offscreen.
        if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
            let towards = pointer - rect.center();
            let factor = 1.0 / (fit * old_scale) - 1.0 / (fit * new_scale);
            let moved = unrotate_view_vector(
                state.yaw,
                state.pitch,
                [towards.x * factor, 0.0, -towards.y * factor],
            );
            state.focus[0] += moved[0];
            state.focus[1] += moved[1];
            state.focus[2] += moved[2];
        }
        state.scale = new_scale;
    }

    let time = preview_clock(ui);
    let frame = build_frame(data, state, rect, time, preview_background(ui.visuals()), &[]);
    // The overlays below draw over the same view and pose.
    let camera = PreviewCamera::new(data, state, rect);
    let bones = frame.bones.clone();
    let skinning_rows = bones.as_deref().map(Vec::as_slice);
    let firing = ce::firing::shots_moving(&data.preview.ce.firing, &state.ce_shots, data.geometry_id, time);
    if frame.moves() || ce_events_running(data.geometry_id) || firing {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
    painter.add(egui::PaintCallback {
        rect,
        callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, painter| {
            paint_model_gl(info, painter, &frame);
        })),
    });
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0_f32, foundation_input_edge()),
        egui::StrokeKind::Middle,
    );
    if let Some(message) = model_gl_failure(data.geometry_id) {
        let note = format!("Drawing this crashed: {message}");
        let color = ui.visuals().error_fg_color;
        painter.text(rect.center(), Align2::CENTER_CENTER, note, FontId::proportional(12.0), color);
    }

    if state.show_errors {
        draw_model_errors(
            ui,
            &painter,
            &response,
            &camera,
            &data.preview.errors,
            state,
            skinning_rows,
        );
    }

    if state.show_armature {
        let positions = armature_node_positions(data, state);
        let projected = positions
            .iter()
            .map(|&position| {
                let projected = camera.project(position);
                projected.in_front.then_some(projected.pos)
            })
            .collect::<Vec<_>>();
        let hover_pos = response
            .hovered()
            .then(|| ui.input(|i| i.pointer.hover_pos()))
            .flatten();
        let mut hovered: Option<(usize, egui::Pos2, f32)> = None;
        for (index, node) in data.preview.nodes.iter().enumerate() {
            let Some(&Some(joint)) = projected.get(index) else {
                continue;
            };
            if node.parent >= 0 {
                let Some(&Some(parent)) = projected.get(node.parent as usize) else {
                    continue;
                };
                painter.line_segment(
                    [parent, joint],
                    Stroke::new(3.5_f32, Color32::from_rgba_unmultiplied(0, 0, 0, 180)),
                );
                painter.line_segment([parent, joint], Stroke::new(1.5_f32, ARMATURE_COLOR));
            }
            painter.circle_filled(joint, 2.5, Color32::WHITE);
            painter.circle_stroke(joint, 2.5, Stroke::new(1.0_f32, Color32::BLACK));
            if let Some(distance) = hover_pos.map(|pos| screen_edge_length(pos, joint))
                && distance <= 6.0
                && hovered.is_none_or(|(_, _, closest)| distance < closest)
            {
                hovered = Some((index, joint, distance));
            }
        }
        if let Some((index, position, _)) = hovered
            && let Some(node) = data.preview.nodes.get(index)
        {
            draw_preview_hover_label(&painter, position + Vec2::new(7.0, -7.0), &node.name, ARMATURE_COLOR);
        }
    }

    if state.show_markers {
        let hover_pos = if response.hovered() {
            ui.input(|i| i.pointer.hover_pos())
        } else {
            None
        };
        let marker_filter = state.marker_filter.trim().to_ascii_lowercase();
        for marker in &data.preview.markers {
            // Name filter (case-insensitive substring; empty = show all).
            if !marker_filter.is_empty()
                && !marker.name.to_ascii_lowercase().contains(&marker_filter)
            {
                continue;
            }
            let (position, axes) = animated_marker_transform(marker, skinning_rows);
            let projected = camera.project(position);
            if !projected.in_front {
                continue;
            }
            let axis_deltas = marker_axis_screen_deltas(&camera, axes);
            draw_marker_axes(&painter, projected.pos, axis_deltas);
            if hover_pos.is_some_and(|pos| marker_axes_hovered(pos, projected.pos, axis_deltas)) {
                draw_preview_hover_label(
                    &painter,
                    projected.pos + Vec2::new(7.0, -7.0),
                    &marker.name,
                    MARKER_TOOLTIP_COLOR,
                );
            }
        }
    }
}
