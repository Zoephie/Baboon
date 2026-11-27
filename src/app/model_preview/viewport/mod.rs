//! The egui widget over the GL picture: input, the camera it moves, and the
//! overlays drawn on top (error reports, markers, the armature, the
//! bounding sphere).

use super::super::ui_kit::style::foundation_input_edge;
use super::{
    MAX_PREVIEW_SCALE, ModelPreviewData, ModelPreviewState, PreviewCamera, armature_node_matrices,
    armature_node_positions, pose_beneath,
    build_frame, min_preview_scale, model_gl_failure, paint_model_gl, preview_background,
    preview_center_radius, preview_clock, unrotate_view_vector, viewport_wheel_zoom,
};
use eframe::egui;
use eframe::egui::{Align2, Color32, FontId, Sense, Stroke, Ui, Vec2};
use std::sync::Arc;

mod bounds;
pub(in crate::app) use bounds::BoundingSphere;
pub(in crate::app::model_preview) use bounds::*;
mod errors;
pub(in crate::app::model_preview) use errors::*;
mod markers;
mod pose_gizmo;
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
    crate::app::ui_kit::autoscroll::reserve_middle_drag(ui, &response);
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
    state.pose.follow(data.geometry_id);
    let wheel = viewport_wheel_zoom(ui, &response);
    let posing = state.pose.on && pose_drag(ui, &response, data, state, rect, wheel);
    if response.dragged_by(egui::PointerButton::Middle) {
        pan_world(state, response.drag_delta());
    } else if response.dragged_by(egui::PointerButton::Primary) && !posing {
        let delta = response.drag_delta();
        if ui.input(|i| i.modifiers.shift) {
            pan_world(state, delta);
        } else {
            state.yaw += delta.x * 0.01;
            state.pitch = (state.pitch + delta.y * 0.01).clamp(-1.45, 1.45);
        }
    }
    if let Some(scroll) = wheel.filter(|_| !posing) {
        let least = min_preview_scale(&data.preview);
        let old_scale = state.scale.clamp(least, MAX_PREVIEW_SCALE);
        let new_scale = (old_scale * (scroll / 450.0).exp()).clamp(least, MAX_PREVIEW_SCALE);
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

    follow_snapshot(ui.ctx(), state);
    let time = preview_clock(ui);
    let frame = build_frame(
        data,
        state,
        rect,
        time,
        state.background.color(preview_background(ui.visuals())),
        &[],
    );
    // The overlays below draw over the same view and pose.
    let camera = PreviewCamera::new(data, state, rect);
    let bones = frame.bones.clone();
    let skinning_rows = bones.as_deref().map(Vec::as_slice);
    // What runs on outside the frame, as a game keeps it.
    let running = super::load::rules(data.preview.game)
        .runs_on
        .is_some_and(|runs_on| runs_on(data, state, time));
    if frame.moves() || running {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }
    // A snapshot asked for: the frame on screen, which the last pass drew,
    // drawn before this pass's frame so nothing has moved on.
    let snapshot = ask_snapshot(ui.ctx(), state);
    let shown = frame.clone();
    painter.add(egui::PaintCallback {
        rect,
        callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, painter| {
            if let Some(request) = &snapshot {
                super::renderer::paint_snapshot(painter, request);
            }
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
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            note,
            FontId::proportional(12.0),
            color,
        );
    }

    // The helpers, from here on: kept with the frame for a snapshot.
    let helpers_from = ui.ctx().graphics(|graphics| graphics.get(painter.layer_id()).map_or(0, |list| list.next_idx().0));
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

    if state.show_bounding_sphere
        && let Some(sphere) = state.bounding_sphere
    {
        let root = armature_node_matrices(data, state).into_iter().next();
        draw_bounding_sphere(&painter, &camera, sphere.about_root(root.as_ref()));
    }

    if state.show_armature || state.pose.on {
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
            // Posing, joints are bigger targets, the ones selected marked,
            // the active one, which carries the gizmo, larger.
            let picked = state.pose.on && state.pose.is_selected(index);
            let active = state.pose.on && state.pose.selected == Some(index);
            if state.pose.on && state.pose.pins.contains_key(&index) {
                painter.circle_stroke(joint, 7.0, Stroke::new(2.0_f32, PIN_COLOR));
            }
            let radius = if active { 5.5 } else if state.pose.on { 4.0 } else { 2.5 };
            painter.circle_filled(joint, radius, if picked { ARMATURE_COLOR } else { Color32::WHITE });
            painter.circle_stroke(joint, radius, Stroke::new(1.0_f32, Color32::BLACK));
            if let Some(distance) = hover_pos.map(|pos| screen_edge_length(pos, joint))
                && distance <= 6.0
                && hovered.is_none_or(|(_, _, closest)| distance < closest)
            {
                hovered = Some((index, joint, distance));
            }
        }
        // The selected joint's move gizmo, but while it twists.
        let twisting = state.pose.dragging.is_some_and(|drag| drag.twist);
        if state.pose.on
            && !twisting
            && let Some(gizmo) = state
                .pose
                .selected
                .and_then(|joint| positions.get(joint))
                .and_then(|&joint| pose_gizmo::PoseGizmo::at(&camera, joint))
        {
            match state.pose.gizmo {
                super::pose::GizmoMode::Move => {
                    let lit = match state.pose.dragging {
                        Some(drag) => drag.lock,
                        None => hover_pos.and_then(|pos| gizmo.hit(pos)),
                    };
                    gizmo.draw(&painter, lit);
                }
                super::pose::GizmoMode::Rotate => {
                    if let Some(rings) = state
                        .pose
                        .selected
                        .and_then(|joint| positions.get(joint))
                        .and_then(|&joint| pose_gizmo::PoseRings::at(&camera, joint))
                    {
                        let lit = match state.pose.dragging {
                            Some(drag) => drag.ring.map(|ring| ring.axis),
                            None => hover_pos.and_then(|pos| rings.hit(pos)).map(|hit| hit.axis),
                        };
                        rings.draw(&painter, lit);
                    }
                }
                super::pose::GizmoMode::Scale => {
                    if let Some(joint) = state.pose.selected
                        && let Some(circle) = positions.get(joint).and_then(|&at| pose_gizmo::PoseScale::at(&camera, at))
                    {
                        let lit = match state.pose.dragging {
                            Some(drag) => drag.scale.is_some(),
                            None => hover_pos.is_some_and(|pos| circle.hit(pos)),
                        };
                        circle.draw(&painter, lit, state.pose.scale_of(joint));
                    }
                }
            }
        }
        // Dragging, the joint names itself and how many nodes above it turn.
        if let Some(drag) = state.pose.dragging
            && let (Some(&Some(position)), Some(node)) =
                (projected.get(drag.joint), data.preview.nodes.get(drag.joint))
        {
            let turning = if state.pose.chain == 1 { "1 node".to_owned() } else { format!("{} nodes", state.pose.chain) };
            draw_preview_hover_label(
                &painter,
                position + Vec2::new(7.0, -7.0),
                &format!("{} · turning {turning} (scroll to change)", node.name),
                ARMATURE_COLOR,
            );
        } else if let Some((index, position, _)) = hovered
            && let Some(node) = data.preview.nodes.get(index)
        {
            draw_preview_hover_label(
                &painter,
                position + Vec2::new(7.0, -7.0),
                &node.name,
                ARMATURE_COLOR,
            );
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
    keep_snapshot_source(ui, state, shown, rect, painter.layer_id(), helpers_from);
}

/// A gizmo handle a press takes.
enum Handle {
    Move(super::pose::Lock),
    /// A ring, and its radius on screen.
    Ring(pose_gizmo::RingHit, f32),
    /// The scale circle, and the joint on screen.
    Scale(egui::Pos2),
}

/// The handle of `mode`'s gizmo about `joint` under `pointer`.
fn gizmo_handle(camera: &PreviewCamera, mode: super::pose::GizmoMode, joint: [f32; 3], pointer: egui::Pos2) -> Option<Handle> {
    match mode {
        super::pose::GizmoMode::Move => pose_gizmo::PoseGizmo::at(camera, joint)?.hit(pointer).map(Handle::Move),
        super::pose::GizmoMode::Rotate => pose_gizmo::PoseRings::at(camera, joint)?
            .hit(pointer)
            .map(|hit| Handle::Ring(hit, pose_gizmo::RING)),
        super::pose::GizmoMode::Scale => {
            let circle = pose_gizmo::PoseScale::at(camera, joint)?;
            circle.hit(pointer).then_some(Handle::Scale(circle.centre))
        }
    }
}

/// A pinned joint's ring.
const PIN_COLOR: Color32 = Color32::from_rgb(255, 140, 40);

/// How near a joint, in points, a press must land to pick it while posing.
const POSE_PICK_DISTANCE: f32 = 8.0;

/// Pose mode's input. A drag started on a joint carries it across the plane
/// through it facing the camera, turning the chain above it (the wheel
/// lengthening or shortening the chain meanwhile); on the root it moves the
/// whole skeleton; with Alt held it twists the joint about its bone. A
/// double-click on a joint pins or unpins it, and every drag ends by holding
/// the pins. Whether a pose drag is under way, which the orbit and zoom then
/// leave alone.
fn pose_drag(
    ui: &egui::Ui,
    response: &egui::Response,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    rect: egui::Rect,
    wheel: Option<f32>,
) -> bool {
    use super::pose::{PoseDrag, RingGrab, ScaleGrab, chain_above, moved_with, hold_pins, reach, reach_held, resolve_limits, rotate, twist};
    let joint_at = |state: &ModelPreviewState, pointer: egui::Pos2| {
        let camera = PreviewCamera::new(data, state, rect);
        armature_node_positions(data, state)
            .iter()
            .enumerate()
            .filter_map(|(index, &position)| {
                let projected = camera.project(position);
                projected.in_front.then(|| (index, screen_edge_length(pointer, projected.pos)))
            })
            .filter(|&(_, distance)| distance <= POSE_PICK_DISTANCE)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    };
    let shift = ui.input(|i| i.modifiers.shift);
    // A click on a joint selects it alone, putting the gizmo on it; with
    // Shift, adds it to the selection or takes it out. A click anywhere
    // else (but the gizmo's own handles) lets the selection go.
    if response.clicked()
        && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
    {
        let on_handle = state.pose.selected.is_some_and(|joint| {
            let camera = PreviewCamera::new(data, state, rect);
            armature_node_positions(data, state)
                .get(joint)
                .is_some_and(|&at| gizmo_handle(&camera, state.pose.gizmo, at, pointer).is_some())
        });
        if !on_handle {
            match (joint_at(state, pointer), shift) {
                (Some(joint), true) => state.pose.toggle_selected(joint),
                (None, true) => {}
                (joint, false) => state.pose.select_only(joint),
            }
        }
    }
    // A double-click on a joint pins or unpins it; with Shift, selects it
    // and everything below it. Taken after the two clicks it is made of.
    if response.double_clicked()
        && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        && let Some(joint) = joint_at(state, pointer)
    {
        state.pose.follow(data.geometry_id);
        if shift {
            state.pose.select_below(&data.preview.nodes, joint);
        } else {
            let at = armature_node_positions(data, state)[joint];
            state.pose.toggle_pin(joint, at);
            state.pose.select_only(Some(joint));
        }
    }
    if response.drag_started_by(egui::PointerButton::Primary) {
        let pointer = ui.input(|i| i.pointer.press_origin());
        let alt = ui.input(|i| i.modifiers.alt);
        let positions = armature_node_positions(data, state);
        // The selected joint's gizmo first: a handle locks the drag.
        let camera = PreviewCamera::new(data, state, rect);
        let handle = state
            .pose
            .selected
            .filter(|_| !alt)
            .and_then(|joint| Some((joint, gizmo_handle(&camera, state.pose.gizmo, *positions.get(joint)?, pointer?)?)));
        state.pose.dragging = match handle {
            Some((joint, Handle::Move(lock))) => Some(PoseDrag {
                joint,
                twist: false,
                lock: Some(lock),
                anchor: positions[joint],
                held: positions[joint],
                ring: None,
                scale: None,
            }),
            Some((joint, Handle::Ring(hit, radius))) => Some(PoseDrag {
                joint,
                twist: false,
                lock: None,
                anchor: positions[joint],
                held: positions[joint],
                ring: pointer.map(|pointer| RingGrab {
                    axis: hit.axis,
                    press: [pointer.x, pointer.y],
                    tangent: [hit.tangent.x, hit.tangent.y],
                    radius,
                    turned: 0.0,
                }),
                scale: None,
            }),
            Some((joint, Handle::Scale(centre))) => Some(PoseDrag {
                joint,
                twist: false,
                lock: None,
                anchor: positions[joint],
                held: positions[joint],
                ring: None,
                scale: pointer.map(|pointer| ScaleGrab {
                    centre: [centre.x, centre.y],
                    press: (pointer - centre).length(),
                    from: state.pose.scale_of(joint),
                }),
            }),
            None => pointer.and_then(|pointer| joint_at(state, pointer)).map(|joint| {
                // A drag on a selected joint takes the selection with it.
                if state.pose.is_selected(joint) {
                    state.pose.activate(joint);
                } else {
                    state.pose.select_only(Some(joint));
                }
                PoseDrag { joint, twist: alt, lock: None, anchor: positions[joint], held: positions[joint], ring: None, scale: None }
            }),
        };
    }
    if !response.dragged_by(egui::PointerButton::Primary) {
        state.pose.dragging = None;
        return false;
    }
    let Some(drag) = state.pose.dragging else {
        return false;
    };
    let nodes = &data.preview.nodes;
    let limits = if state.pose.limits {
        resolve_limits(nodes, &data.preview.joint_limits)
    } else {
        Vec::new()
    };
    let base = pose_beneath(data, state);
    if base.is_empty() {
        return true;
    }
    // The gizmo's scale, turn and twist take every selected joint, each
    // about its own origin, nested ones adding up (a hand's finger joints
    // curl together).
    let joints = state.pose.joints();
    if let Some(grab) = drag.scale {
        if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
            let ratio = grab.scale([pointer.x, pointer.y]) / state.pose.scale_of(drag.joint);
            for &joint in &joints {
                state.pose.set_scale(joint, state.pose.scale_of(joint) * ratio);
            }
        }
    } else if let Some(ring) = drag.ring {
        if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
            let angle = ring.angle([pointer.x, pointer.y]);
            for &joint in &joints {
                rotate(nodes, &base, &mut state.pose, joint, ring.axis, angle - ring.turned, &limits);
            }
            if let Some(grab) = state.pose.dragging.as_mut().and_then(|drag| drag.ring.as_mut()) {
                grab.turned = angle;
            }
        }
    } else if drag.twist {
        // A point across is a hundredth of a radian.
        let angle = response.drag_delta().x * 0.01;
        for &joint in &joints {
            twist(nodes, &base, &mut state.pose, joint, angle, &limits);
        }
    } else {
        // The wheel lengthens or shortens the chain a node at a time.
        if let Some(scrolled) = wheel {
            state.pose.chain = if scrolled > 0.0 {
                state.pose.chain + 1
            } else {
                state.pose.chain.saturating_sub(1).max(1)
            };
        }
        let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) else {
            return true;
        };
        let camera = PreviewCamera::new(data, state, rect);
        let matrices = armature_node_matrices(data, state);
        let Some(joint) = matrices.get(drag.joint).map(|matrix| matrix.translation()) else {
            return true;
        };
        let target = match drag.lock {
            Some(lock) => {
                let (origin, direction) = camera.ray(pointer);
                // Edge on, the line or plane picks nothing: the joint waits.
                let Some(target) = lock.target(drag.anchor, origin, direction) else {
                    return true;
                };
                target
            }
            None => camera.unproject_beside(pointer, joint),
        };
        // The others the move takes along, as they stand before it.
        let others: Vec<(usize, [f32; 3])> = {
            let positions = armature_node_positions(data, state);
            moved_with(nodes, &joints, drag.joint)
                .into_iter()
                .filter_map(|joint| Some((joint, *positions.get(joint)?)))
                .collect()
        };
        let chain = chain_above(nodes, drag.joint, state.pose.chain);
        if chain.is_empty() {
            // The root: the whole skeleton moves with it.
            for axis in 0..3 {
                state.pose.offset[axis] += target[axis] - joint[axis];
            }
        } else {
            // A chain longer than the joint has ancestors stops at the root.
            state.pose.chain = state.pose.chain.min(chain.len());
            if drag.lock.is_some() {
                let held = reach_held(nodes, &base, &mut state.pose, drag.joint, &chain, (drag.held, target), &limits);
                if let Some(dragging) = state.pose.dragging.as_mut() {
                    dragging.held = held;
                }
            } else {
                reach(nodes, &base, &mut state.pose, drag.joint, &chain, target, &limits);
            }
        }
        // The others move as far as the dragged joint did, each through its
        // own chain; moving the root moved them all already.
        if !chain.is_empty() {
            let reached = armature_node_positions(data, state)[drag.joint];
            let delta: [f32; 3] = std::array::from_fn(|axis| reached[axis] - joint[axis]);
            for &(other, from) in &others {
                let chain = chain_above(nodes, other, state.pose.chain);
                if !chain.is_empty() {
                    let target = std::array::from_fn(|axis| from[axis] + delta[axis]);
                    reach(nodes, &base, &mut state.pose, other, &chain, target, &limits);
                }
            }
        }
        // A pinned joint moved is pinned where it is moved to.
        let positions = armature_node_positions(data, state);
        for moved in std::iter::once(drag.joint).chain(others.iter().map(|&(other, _)| other)) {
            if state.pose.pins.contains_key(&moved) {
                state.pose.pins.insert(moved, positions[moved]);
            }
        }
    }
    hold_pins(nodes, &base, &mut state.pose, Some(drag.joint), &limits);
    true
}

/// Keep what this pass drew, the frame and the helpers painted over it from
/// `helpers_from` on, moved to the viewport's corner: what a snapshot asked
/// for next pass draws, as it is on screen then.
fn keep_snapshot_source(
    ui: &Ui,
    state: &mut ModelPreviewState,
    frame: super::frame::ModelGpuFrame,
    rect: egui::Rect,
    layer: egui::LayerId,
    helpers_from: usize,
) {
    let offset = -rect.min.to_vec2();
    let helpers = ui.ctx().graphics(|graphics| {
        graphics.get(layer).map_or_else(Vec::new, |list| {
            list.all_entries()
                .skip(helpers_from)
                .map(|clipped| {
                    let mut clipped = clipped.clone();
                    clipped.shape.translate(offset);
                    clipped.clip_rect = clipped.clip_rect.translate(offset);
                    clipped
                })
                .collect()
        })
    });
    state.snapshot_source = Some(super::snapshot::SnapshotSource {
        frame,
        points: rect.size(),
        pixels_per_point: ui.ctx().pixels_per_point(),
        helpers,
    });
}

/// A snapshot asked for this pass, made into its request from the frame the
/// last pass drew, at the snapshot's scale.
fn ask_snapshot(ctx: &egui::Context, state: &mut ModelPreviewState) -> Option<Arc<super::snapshot::SnapshotRequest>> {
    use super::snapshot::SnapshotPhase;
    if !matches!(state.snapshot, SnapshotPhase::Asked) {
        return None;
    }
    let Some(source) = &state.snapshot_source else {
        state.snapshot = SnapshotPhase::Idle;
        state.snapshot_message = Some("The preview has drawn nothing to copy yet".to_owned());
        return None;
    };
    let request = source.request(ctx, state.snapshot_scale);
    state.snapshot = SnapshotPhase::Drawing(Arc::clone(&request));
    ctx.request_repaint();
    Some(request)
}

/// Move a snapshot on: once drawn, its image to the clipboard; once copied,
/// what happened for the status bar.
fn follow_snapshot(ctx: &egui::Context, state: &mut ModelPreviewState) {
    use super::snapshot::SnapshotPhase;
    match &state.snapshot {
        SnapshotPhase::Drawing(request) => match request.take_result() {
            Some(Ok(image)) => state.snapshot = SnapshotPhase::Copying(super::snapshot::copy_to_clipboard(image, ctx)),
            Some(Err(error)) => {
                state.snapshot_message = Some(format!("Could not snapshot the preview: {error}"));
                state.snapshot = SnapshotPhase::Idle;
            }
            None => ctx.request_repaint(),
        },
        SnapshotPhase::Copying(done) => {
            let message = done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
            if let Some(message) = message {
                state.snapshot_message = Some(message);
                state.snapshot = SnapshotPhase::Idle;
            }
        }
        SnapshotPhase::Idle | SnapshotPhase::Asked | SnapshotPhase::Confirming(_) => {}
    }
}
