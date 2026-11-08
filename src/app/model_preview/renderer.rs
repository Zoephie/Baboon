//! Depth-tested Glow rendering, markers, and preview geometry conversion.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::gl::link::link;
use super::gl::program::Linked;
use super::gl::scope::PostScope;
use super::gl::target::{ColorTarget, Made, Sampling, Texels};
use super::*;
use super::dx9::passes::{
    H2BlendEquation, H2BlendFactor, H2Compare, H2Cull, H2DefaultLight, H2DrawPass, H2Layer,
    H2_GLSL_VERSION, H2_LAYER_COUNT, H2_REFLECTION_SCALE,
    h2_change_colors_for, h2_normalization_face, h2_object_light_direction, h2_prt_object_lights,
};
use super::common::image::H2Image;
use super::h2::bloom::H2Bloom;
use super::dx9::passes::{H2Clock, h2_prt_lights};
use super::dx9::stage::H2VertexStage;
use super::gen3::shaders::{H3Blend, H3Sampler};
use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};
use crate::core::h2_pixel_shader::SamplerKind;
use eframe::glow::{self, HasContext as _};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use crate::app::model_preview::common::attached::Attached;

const ARMATURE_COLOR: Color32 = Color32::from_rgb(255, 204, 0);
const MARKER_TOOLTIP_COLOR: Color32 = Color32::from_rgb(120, 235, 255);

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

pub(super) fn draw_model_viewport(
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

/// Leave out what only a shield draws, once the materials are known.
pub(super) fn without_shield_parts(visible: &mut Vec<usize>, preview: &RenderModelPreview, textures: Option<&[MaterialTextures]>) {
    let Some(textures) = textures else {
        return;
    };
    visible.retain(|&batch| {
        let material = preview.batches.get(batch).map_or(0, |batch| batch.material_index as usize);
        !textures.get(material).is_some_and(|texture| texture.shield_only)
    });
}

/// The batches the current toggles and region selections draw.
///
/// The overlay toggles are draw-time filters: the collision and physics
/// layers sit merged in the geometry from the moment the worker delivers
/// them, and showing or hiding them costs one batch-list rebuild — not the
/// tag re-reads that froze the frame when the toggles re-merged the preview.
/// Gated on `overlays_loaded` so a standalone collision/physics tag — whose
/// MAIN content uses these region names — is never filtered.
///
/// An overlay region shares its selection with the render region of the same
/// name, so it follows the variant when its permutations are named like the
/// render model's. When they aren't — a Halo CE gbxmodel names its monitor
/// permutations `monitor`/`lightning-100` while the collision model's only
/// one is `__base` — the selected name matches no overlay batch, and the
/// overlay draws its region's first permutation instead of vanishing.
/// The frame `state` shows of `data` in `rect` at `time` over `background`,
/// with `forced` exports set over what the functions make. The app and the
/// headless harness both build their frames here.
fn build_frame(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
    rect: egui::Rect,
    time: f32,
    background: [f32; 3],
    forced: &[(String, f32)],
) -> ModelGpuFrame {
    let camera = PreviewCamera::new(data, state, rect);
    let preview = Arc::clone(&data.preview);
    let mut visible_batches = visible_batch_indices(&preview, state);
    let shield_batches = visible_batches.clone();
    without_shield_parts(&mut visible_batches, &preview, data.textures.as_deref().map(Vec::as_slice));
    let skinning_rows = animation_skinning_rows(data, state);
    let mut function_values = function_values_for(data, &state.function_inputs, time);
    // What a function exports, which its inputs cannot reach: a scripted
    // value, say. Only the harness forces any.
    if let Some(values) = function_values.as_mut() {
        for (name, value) in forced {
            values.insert(name.clone(), *value);
        }
    }
    let mut frame = ModelGpuFrame {
        preview,
        geometry_id: data.geometry_id,
        mesh_id: data.mesh_id,
        textures_id: data.textures_id,
        visible_batches,
        shield_batches,
        camera: camera.gpu_uniforms(),
        render_mode: state.render_mode,
        background,
        show_backfaces: state.show_backfaces,
        show_grid: state.show_grid,
        textures: state
            .render_mode
            .uses_textures()
            .then(|| data.textures.clone())
            .flatten(),
        bones: skinning_rows.clone().map(Arc::new),
        game_shaders: state.game_shaders,
        default_lights: state.default_lights,
        active_camo: state.active_camo,
        // Halo CE's renderer has no bloom: only the remaster's does.
        bloom: state.bloom && data.preview.game != Some(GameId::HaloCe),
        self_shadow: state.self_shadow,
        prt: state.prt,
        light_follows_camera: state.light_follows_camera,
        time,
        h2_prt_clusters: None,
        object_states: object_states(data, &state.ce_function_inputs, &state.ce_shots, time),
        object_moves: object_moves(data, &state.function_inputs, time, function_values.as_ref()),
        function_values,
        attached_values: attached_values(data, time),
        attached_colors: attached_colors(data),
        h3_flares_shown: h3_flares_shown(&data.preview, state),
        h3_lights_shown: h3_lights_shown(&data.preview, state),
        reach_lights_shown: reach_lights_shown(&data.preview, state),
        h3_volumes_shown: h3_volumes_shown(&data.preview, state),
        h4_tracers_shown: h4_tracers_shown(&data.preview, state),
        h3_systems_shown: h3_systems_shown(&data.preview, state),
        h3_strips_shown: h3_strips_shown(&data.preview, state),
        reach_cheap_shown: reach_cheap_shown(&data.preview, state),
        h2_flares_shown: h2_flares_shown(&data.preview, state),
        h2_shown: H2Shown::of(&data.preview, state),
        ce_points: Vec::new(),
        ce_plays: ce_event_plays(data, &state.ce_event_counts, state.ce_event_scales),
        change_colors: h2_change_colors_for(
            &data.change_colors,
            state
                .selected_variant
                .and_then(|index| data.variants.get(index))
                .map_or("", |variant| variant.name.as_str()),
        ),
    };
    frame.h2_prt_clusters = frame.h2_object_prt_clusters();
    frame.ce_points = frame.ce_point_lights();
    frame
}

pub(super) fn visible_batch_indices(
    preview: &RenderModelPreview,
    state: &ModelPreviewState,
) -> Vec<usize> {
    let overlay_has_selected = |layer: ModelPreviewLayer, region: &str, permutation: &str| {
        preview.batches.iter().any(|batch| {
            batch.layer == layer
                && batch.region_name == region
                && batch.permutation_name == permutation
        })
    };
    let overlay_first_permutation = |layer: ModelPreviewLayer, region: &str| {
        preview
            .batches
            .iter()
            .find(|batch| batch.layer == layer && batch.region_name == region)
            .map(|batch| batch.permutation_name.as_str())
    };
    let instances = visible_instances(preview, state);
    preview
        .batches
        .iter()
        .enumerate()
        .filter_map(|(index, batch)| {
            if !model_layer_visible(state, batch.layer) {
                return None;
            }
            if let Some(instance) = preview.instances.batches.get(&index) {
                // A mask holds 128 placements; one past them is never chosen.
                let chosen = u32::try_from(*instance).ok().and_then(|shift| instances.checked_shr(shift));
                return (chosen.unwrap_or(0) & 1 != 0).then_some(index);
            }
            let selection = state.region_selections.get(&batch.region_name)?;
            if !selection.enabled {
                return None;
            }
            if selection.permutation == batch.permutation_name {
                return Some(index);
            }
            let is_overlay = state.overlays_loaded && batch.layer != ModelPreviewLayer::Render;
            (is_overlay
                && !overlay_has_selected(batch.layer, &batch.region_name, &selection.permutation)
                && overlay_first_permutation(batch.layer, &batch.region_name)
                    == Some(batch.permutation_name.as_str()))
            .then_some(index)
        })
        .collect()
}

/// `model_compute_instance_bitfield_visible`: what the selected permutations
/// allow of what the variant's instance group chose.
fn visible_instances(preview: &RenderModelPreview, state: &ModelPreviewState) -> u128 {
    let instances = &preview.instances;
    let chosen = state.selected_variant.and_then(|variant| instances.variants.get(variant));
    let allowed = state.region_selections.iter().filter(|(_, selection)| selection.enabled);
    let allowed = allowed.filter_map(|(region, selection)| {
        instances.masks.get(&(region.clone(), selection.permutation.clone()))
    });
    chosen.copied().unwrap_or(0) & allowed.fold(0, |mask, permutation| mask | permutation)
}

/// Which of the preview's Halo 3 flares have a marker under what is selected.
fn h3_flares_shown(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
    let shown = |only| marker_active(only, &state.region_selections);
    let h4 = preview.h4.flares.iter().map(|at| at.only_under().is_none_or(shown));
    preview.gen3.flares.iter().map(|at| at.only_under().is_none_or(shown)).chain(h4).collect()
}

/// The same of Reach's lights and then its cheap lights, then Halo 4's.
pub(super) fn reach_lights_shown(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
    let shown = |only| marker_active(only, &state.region_selections);
    let lights = preview.reach.lights.iter().map(|light| light.only_under().is_none_or(shown));
    let cheap = preview.reach.cheap_lights.iter().map(|light| light.only_under().is_none_or(shown));
    let h4 = preview.h4.lights.iter().map(|light| light.only_under().is_none_or(shown));
    let h4_cheap = preview.h4.cheap_lights.iter().map(|light| light.only_under().is_none_or(shown));
    lights.chain(cheap).chain(h4).chain(h4_cheap).collect()
}

/// The same of Halo 3's lights.
fn h3_lights_shown(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
    let shown = |only| marker_active(only, &state.region_selections);
    preview.h3.lights.iter().map(|light| light.only_under().is_none_or(shown)).collect()
}

/// The same of Halo 3's light volumes.
fn h3_volumes_shown(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
    let shown = |only| marker_active(only, &state.region_selections);
    preview.gen3.light_volumes.iter().map(|at| at.only_under().is_none_or(shown)).collect()
}

/// The same of Halo 4's tracers.
fn h4_tracers_shown(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
    let shown = |only| marker_active(only, &state.region_selections);
    preview.h4.tracers.iter().map(|at| at.only_under().is_none_or(shown)).collect()
}

/// The same of Reach's cheap particle emitters.
fn reach_cheap_shown(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
    let shown = |only| marker_active(only, &state.region_selections);
    preview.gen3.cheap_emitters.iter().map(|at| at.only_under().is_none_or(shown)).collect()
}

/// The same of Halo 3's beams and contrails.
fn h3_strips_shown(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
    let shown = |only| marker_active(only, &state.region_selections);
    preview.gen3.strips.iter().map(|at| at.only_under().is_none_or(shown)).collect()
}

/// The same of Halo 3's particle systems.
fn h3_systems_shown(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
    let shown = |only| marker_active(only, &state.region_selections);
    preview.particles.iter().map(|at| at.only_under().is_none_or(shown)).collect()
}

/// The same of Halo 2's.
fn h2_flares_shown(preview: &RenderModelPreview, state: &ModelPreviewState) -> Vec<bool> {
    let shown = |only| marker_active(only, &state.region_selections);
    preview.h2.flares.iter().map(|at| at.only_under().is_none_or(shown)).collect()
}

/// Which of Halo 2's other lights and effects have a marker under what is
/// selected: `object_get_markers_by_string_id` finds only those of the
/// permutations the object shows, so what hangs on the others is not made.
#[derive(Debug, Clone, Default)]
pub(super) struct H2Shown {
    pub(in crate::app::model_preview) points: Vec<bool>,
    pub(in crate::app::model_preview) gels: Vec<bool>,
    pub(in crate::app::model_preview) volumes: Vec<bool>,
    pub(in crate::app::model_preview) contrails: Vec<bool>,
    pub(in crate::app::model_preview) liquids: Vec<bool>,
    antennas: Vec<bool>,
}

impl H2Shown {
    pub(super) fn of(preview: &RenderModelPreview, state: &ModelPreviewState) -> Self {
        let shown = |only: Option<&(String, String)>| only.is_none_or(|only| marker_active(only, &state.region_selections));
        Self {
            points: preview.h2.point_lights.iter().map(|at| shown(at.only_under())).collect(),
            gels: preview.h2.gel_lights.iter().map(|at| shown(at.only_under())).collect(),
            volumes: preview.h2.light_volumes.iter().map(|at| shown(at.only_under())).collect(),
            contrails: preview.h2.contrails.iter().map(|at| shown(at.only_under())).collect(),
            liquids: preview.h2.liquids.iter().map(|at| shown(at.only_under())).collect(),
            antennas: preview.antennas.iter().map(|at| shown(at.only_under())).collect(),
        }
    }
}

pub(in crate::app::model_preview) fn is_shown(shown: &[bool], index: usize) -> bool {
    shown.get(index).copied().unwrap_or(true)
}

pub(in crate::app::model_preview) fn marker_active(only: &(String, String), selections: &HashMap<String, ModelRegionSelection>) -> bool {
    let selection = selections.get(&only.0);
    selection.is_some_and(|selection| selection.enabled && selection.permutation == only.1)
}

fn model_layer_visible(state: &ModelPreviewState, layer: ModelPreviewLayer) -> bool {
    if !state.overlays_loaded {
        // A bare render/collision/physics tag is the primary preview, not an
        // optional overlay. `show_render` remains true for that path.
        return state.show_render;
    }
    match layer {
        ModelPreviewLayer::Render => state.show_render,
        ModelPreviewLayer::Collision => state.show_collision,
        ModelPreviewLayer::Physics => state.show_physics,
    }
}

fn draw_model_errors(
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

fn polygon_edges(polygon: &[egui::Pos2]) -> impl Iterator<Item = (&egui::Pos2, &egui::Pos2)> {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
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

fn point_in_polygon(point: egui::Pos2, polygon: &[egui::Pos2]) -> bool {
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

fn polygon_area_twice(polygon: &[egui::Pos2]) -> f32 {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum()
}

fn animated_marker_transform(
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

fn draw_preview_hover_label(
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

#[derive(Clone, Copy)]
pub(super) struct ModelGpuCamera {
    pub(super) center: [f32; 3],
    pub(super) scale: f32,
    pub(super) yaw: f32,
    pub(super) pitch: f32,
    pub(super) clip_scale: [f32; 2],
    pub(super) depth_scale: f32,
    /// Linear remap `z = t · x + y` (with `t = view.y · depth_scale`) that
    /// spreads the extended render distance across NDC depth; see
    /// `gpu_uniforms`.
    pub(super) depth_map: [f32; 2],
    /// `1 / eye` in scaled view units: `w = 1 + view.y · this`. 0.0 is the
    /// orthographic view.
    pub(super) perspective: f32,
}

/// Perspective's vertical field of view (across the viewport's shorter side),
/// fixed so zooming moves the eye rather than narrowing the lens.
///
/// The eye sits on the view axis at the distance where this lens frames the
/// focus plane exactly as the orthographic view does at the same zoom, so
/// `w = 1 + view.y / eye` is 1 at the orbit point: toggling the projection
/// never jumps, and pan and zoom-to-cursor keep their orthographic math
/// there. The fit spans `1.1 · radius` each side of the focus in *scaled*
/// view units at every zoom, so in those units `eye = 1.1 · radius /
/// tan(fov / 2)` is constant — and in world units it shrinks as `1 / scale`.
/// That is the dolly: zooming in walks the eye toward the orbit point and the
/// perspective deepens, where the old fixed eye at two radii only magnified
/// the picture and flattened toward orthographic the closer one looked.
const PERSPECTIVE_FOV_Y: f32 = 60.0 * std::f32::consts::PI / 180.0;

/// The perspective near plane, as a fraction of the eye's distance to the
/// orbit point. Small, because a dolly brings the eye right up to (and
/// inside) geometry, and anything nearer than this is clipped away — but not
/// smaller: depth precision falls with the near plane, and at the deepest
/// zoom on a level 1% left distant walls fighting.
const PERSPECTIVE_NEAR_FRACTION: f32 = 0.05;

/// How many depth windows of render distance the clip range covers, beyond
/// the one the framing math is built around. Depth clipping is remapped —
/// not the eye — so a larger margin costs only depth-buffer precision and
/// never flattens the perspective. See `depth_map` in `gpu_uniforms`.
const PREVIEW_RENDER_DISTANCE: f32 = 8.0;

/// GPU skinning bone budget: three vec4 uniform rows per bone. Halo node
/// indices are bytes, so 256 covers every skeleton any graph can address
/// (Reach's halsey alone has 163 nodes; the old budget of 96 silently
/// refused her animations). 768 vec4 rows sit well inside desktop uniform
/// limits — the practical floor is 1024 vec4.
pub(in crate::app) const MAX_PREVIEW_BONES: usize = 256;

#[derive(Clone)]
pub(super) struct ModelGpuFrame {
    pub(super) preview: Arc<RenderModelPreview>,
    pub(super) geometry_id: u64,
    /// See `ModelPreviewData::mesh_id`.
    pub(super) mesh_id: u64,
    /// What `textures` belongs to; see `ModelPreviewData::textures_id`.
    pub(super) textures_id: u64,
    pub(super) visible_batches: Vec<usize>,
    /// `visible_batches` with the parts only a shield draws.
    pub(super) shield_batches: Vec<usize>,
    pub(super) camera: ModelGpuCamera,
    pub(super) render_mode: ModelRenderMode,
    pub(super) show_backfaces: bool,
    /// What the frame clears to.
    pub(super) background: [f32; 3],
    /// Draw the world-unit ground grid on the z = 0 plane.
    pub(super) show_grid: bool,
    /// `None` until the worker has resolved them, or when shading is off.
    pub(super) textures: Option<Arc<Vec<MaterialTextures>>>,
    /// Skinning matrices for this frame — three vec4 rows per bone, in
    /// `RenderModelPreview::nodes` order. `None` draws the bind pose.
    pub(super) bones: Option<Arc<Vec<[f32; 4]>>>,
    /// The selected variant's change colors for Halo 2 passes.
    pub(super) change_colors: [[f32; 3]; 4],
    /// Halo CE: each object's `A out` to `D out` and change colors this frame,
    /// the opened one first. Empty with none.
    pub(super) object_states: Vec<CeObjectState>,
    /// Whether those change by themselves.
    pub(super) object_moves: bool,
    /// Halo 2: what its shaders can read of the object's functions.
    pub(super) function_values: Option<functions::FunctionValues>,
    /// What a held weapon's own functions read, by owner.
    pub(super) attached_values: Vec<(u8, functions::FunctionValues)>,
    /// Each attached Halo 2 or Halo 3 object's own change colors, by owner.
    pub(super) attached_colors: Vec<(u8, [[f32; 3]; 4])>,
    pub(super) h3_flares_shown: Vec<bool>,
    pub(super) h3_lights_shown: Vec<bool>,
    /// Reach's lights and then its cheap lights, as `h3_lights_shown`.
    pub(super) reach_lights_shown: Vec<bool>,
    pub(super) h3_volumes_shown: Vec<bool>,
    pub(super) h4_tracers_shown: Vec<bool>,
    pub(super) h3_systems_shown: Vec<bool>,
    pub(super) h3_strips_shown: Vec<bool>,
    pub(super) reach_cheap_shown: Vec<bool>,
    pub(super) h2_flares_shown: Vec<bool>,
    /// The rest of Halo 2's lights and effects, by whether their marker is
    /// under what is selected.
    pub(super) h2_shown: H2Shown,
    /// Each object's point lights this frame, in step with `object_states`.
    pub(super) ce_points: Vec<[[f32; 4]; ce::lights::CE_POINT_LIGHT_ROWS]>,
    /// The panel's plays of Halo CE's event effects.
    pub(super) ce_plays: ce::particles::CeEventPlays,
    /// Draw opaque Halo 2 materials through their passes.
    pub(super) game_shaders: bool,
    pub(super) default_lights: bool,
    pub(super) active_camo: bool,
    pub(super) bloom: bool,
    pub(super) self_shadow: bool,
    pub(super) prt: bool,
    pub(super) light_follows_camera: bool,
    /// Seconds, for what Halo 2 shaders animate.
    pub(super) time: f32,
    /// A PRT model's constants on the object path: lit, then in shadow.
    pub(super) h2_prt_clusters: Option<[Vec<[f32; 4]>; 2]>,
}

/// Wrapped to keep a frame time precise in the `f32` the shaders take.
const H2_TIME_WRAP: f64 = 3600.0;

/// The time the preview's simulations and functions run by.
pub(super) fn preview_clock(ui: &Ui) -> f32 {
    ui.input(|input| input.time % H2_TIME_WRAP) as f32
}

/// What the parts an object owns draw with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CeObjectState {
    pub(in crate::app::model_preview) outputs: [f32; 4],
    /// Which outputs are on, as `object_get_function_value` answers.
    active: [bool; 4],
    pub(in crate::app::model_preview) change_colors: [[f32; 3]; 4],
}

/// The names the shaders, lights and effects of `object` read, besides what
/// its functions import: 0 for the opened object, or an attached one's owner.
pub(super) fn h2_shader_inputs(data: &ModelPreviewData, object: u8) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let textures = data.textures.iter().flat_map(|textures| textures.iter());
    let owned: Vec<&MaterialTextures> = data
        .preview
        .materials
        .iter()
        .zip(textures)
        .filter(|(material, _)| material.object == object)
        .map(|(_, texture)| texture)
        .collect();
    let passes = owned.iter().flat_map(|texture| texture.h2_layers.iter().flatten());
    let flares = data.preview.h2.flares.iter().filter(|at| at.object == object).map(|at| at.scale_name());
    let lights = data.preview.h2.point_lights.iter().filter(|light| light.object == object);
    let h3_lights = data.preview.h3.lights.iter().filter(|light| light.object == object);
    let reach_lights = data.preview.reach.lights.iter().filter(|light| light.object == object);
    let reach_cheap = data.preview.reach.cheap_lights.iter().filter(|light| light.object == object);
    let h4_lights = data.preview.h4.lights.iter().filter(|light| light.object == object);
    let h4_cheap = data.preview.h4.cheap_lights.iter().filter(|light| light.object == object);
    let h4_flares = data.preview.h4.flares.iter().filter(|at| at.object == object);
    let h3_flares = data.preview.gen3.flares.iter().filter(|at| at.object == object);
    let h3_volumes = data.preview.gen3.light_volumes.iter().filter(|at| at.object == object);
    let h4_tracers = data.preview.h4.tracers.iter().filter(|at| at.object == object);
    let h3_systems = data.preview.particles.iter().filter(|at| at.object == object);
    let h3_strips = data.preview.gen3.strips.iter().filter(|at| at.object == object);
    let reach_emitters = data.preview.gen3.cheap_emitters.iter().filter(|at| at.object == object);
    let h3_screens = data.preview.gen3.screen_effects.iter().filter(|at| at.object == object);
    let gels = data.preview.h2.gel_lights.iter().filter(|light| light.object == object);
    let h2_volumes = data.preview.h2.light_volumes.iter().filter(|at| at.object == object).map(|at| at.scale_name());
    let h2_contrails = data.preview.h2.contrails.iter().filter(|at| at.object == object).map(|at| at.scale_name());
    let h2_liquids = data.preview.h2.liquids.iter().filter(|at| at.object == object).map(|at| at.scale_name());
    let h3 = owned.iter().filter_map(|texture| texture.h3.as_ref()).flat_map(|material| &material.animations);
    let h3 = h3.flat_map(|animation| [animation.input.as_str(), animation.range.as_str()]);
    let h4 = owned.iter().filter_map(|texture| texture.h3.as_ref()).filter_map(|material| material.h4());
    // A hologram's screen materials read its object's functions too.
    let post = h4.clone().filter_map(|submit| submit.post());
    let post = post.flat_map(|post| post.combine.submit.inputs().chain(post.composite.submit.inputs()));
    let h3 = h3.chain(h4.flat_map(|submit| submit.inputs())).chain(post);
    let flares = flares
        .chain(h3)
        .chain(h3_lights.flat_map(|light| light.inputs()))
        .chain(reach_lights.flat_map(|light| light.inputs()))
        .chain(reach_cheap.flat_map(|light| light.inputs()))
        .chain(h4_lights.flat_map(|light| light.inputs()))
        .chain(h4_cheap.flat_map(|light| light.inputs()))
        .chain(h4_flares.flat_map(|at| at.inputs()))
        .chain(h3_flares.map(|at| at.scale_name()))
        .chain(h3_volumes.flat_map(|at| at.scale_names()))
        .chain(h4_tracers.flat_map(|at| at.scale_names()))
        .chain(h3_systems.flat_map(|at| at.scale_names()))
        .chain(h3_strips.flat_map(|at| at.scale_names()))
        .chain(reach_emitters.flat_map(|at| at.scale_names()))
        .chain(h3_screens.clone().map(|at| at.scale_name()))
        .chain(h3_screens.flat_map(|at| at.object_inputs()))
        .chain(lights.map(|light| light.scale_name()))
        .chain(gels.map(|light| light.scale_name()))
        .chain(h2_volumes)
        .chain(h2_contrails)
        .chain(h2_liquids)
        .filter(|name| !name.is_empty());
    // `sub_6C18F0` gates the shield overlay on this itself.
    let h3_shield = owned.iter().filter_map(|texture| texture.h3.as_ref()).any(|h3| h3.shield.is_some());
    let shield = data.preview.materials.iter().any(|m| m.object == object && m.shield == Some(H2Layer::ShieldDamaged));
    let reach_shield = object == 0 && (data.preview.reach.shield.is_some() || data.preview.h4.shield.is_some());
    let gate = (shield || h3_shield || reach_shield).then_some("current_shield_damage");
    let gate = gate.into_iter().chain(h3_shield.then_some(gen3::pipeline::H3_OVERSHIELD));
    for name in passes.flat_map(|pass| pass.function_inputs()).chain(flares).chain(gate) {
        if !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }
    names
}

/// Every name an opened Halo 2 object answers to.
pub(super) fn h2_function_values(
    data: &ModelPreviewData,
    set: &functions::FunctionInputs,
    time: f32,
) -> functions::FunctionValues {
    let inputs = h2_shader_inputs(data, 0);
    data.object_functions.as_ref().map_or_else(Default::default, |functions| {
        functions.values(set, data.geometry_id, &inputs, time, 0)
    })
}

/// Weapons in hand run on their own functions, at rest: the sliders are the
/// opened object's.
pub(super) fn attached_values(data: &ModelPreviewData, time: f32) -> Vec<(u8, functions::FunctionValues)> {
    if data.attached.is_empty() {
        return Vec::new();
    }
    let rest = functions::FunctionInputs::default();
    let values = data.attached.iter().map(|attached| {
        let inputs = h2_shader_inputs(data, attached.owner);
        (attached.owner, attached.functions.values(&rest, data.geometry_id, &inputs, time, attached.owner))
    });
    values.collect()
}

fn attached_colors(data: &ModelPreviewData) -> Vec<(u8, [[f32; 3]; 4])> {
    data.attached.iter().map(|attached| (attached.owner, attached.change_colors)).collect()
}

fn function_values_for(
    data: &ModelPreviewData,
    set: &functions::FunctionInputs,
    time: f32,
) -> Option<functions::FunctionValues> {
    let flares = data.preview.h2.flares.iter().any(|at| !at.scale_name().is_empty())
        || data.preview.h2.point_lights.iter().any(|light| !light.scale_name().is_empty())
        || data.preview.h2.gel_lights.iter().any(|light| !light.scale_name().is_empty());
    let h3 = data.textures.iter().flat_map(|textures| textures.iter()).any(|texture| texture.h3.is_some());
    (data.object_functions.is_some() || flares || h3).then(|| h2_function_values(data, set, time))
}

/// Whether anything on the object changes with time alone.
pub(super) fn object_moves(
    data: &ModelPreviewData,
    set: &functions::FunctionInputs,
    time: f32,
    function_values: Option<&functions::FunctionValues>,
) -> bool {
    object_functions(data).any(|functions| functions.move_at_rest())
        || h2_functions_move(data, set, time)
        || data.preview.h2.flares.iter().any(|at| at.animates())
        || data.preview.h2.point_lights.iter().any(|light| light.animates())
        || data.preview.h2.gel_lights.iter().any(|light| light.animates())
        || data.preview.h3.lights.iter().any(|light| light.animates())
        || data.preview.reach.lights.iter().any(|light| light.animates())
        || data.preview.reach.cheap_lights.iter().any(|light| light.animates())
        || data.preview.h4.lights.iter().any(|light| light.animates())
        || data.preview.h4.cheap_lights.iter().any(|light| light.animates())
        || data.preview.h4.flares.iter().any(|at| at.animates())
        || data.preview.gen3.flares.iter().any(|at| at.animates())
        || data.preview.gen3.light_volumes.iter().any(|at| at.animates())
        || !data.preview.h4.tracers.is_empty()
        || !data.preview.particles.is_empty()
        || !data.preview.gen3.cheap_emitters.is_empty()
        || !data.preview.gen3.strips.is_empty()
        || !data.preview.gen3.screen_effects.is_empty()
        || !data.preview.cloth.is_empty()
        || !data.preview.h2.liquids.is_empty()
        || !data.preview.h2.contrails.is_empty()
        // An event's effect runs only when played, and asks for frames then.
        || data.preview.ce.effects.iter().any(|effect| effect.event.is_none())
        || !data.preview.ce.contrails.is_empty()
        || !data.preview.ce.lightning.is_empty()
        || !data.preview.ce.glows.is_empty()
        || !data.preview.ce.particle_systems.is_empty()
        || !data.preview.ce.flags.is_empty()
        || !data.preview.h3.leaves.is_empty()
        || data.preview.reach.shield.as_ref().is_some_and(|shield| {
            let damage = function_values.and_then(|values| values.get(gen3::pipeline::H3_SHIELD_DAMAGE)).copied().unwrap_or(0.0);
            shield.settings.showing(damage)
        })
        || data.textures.iter().flat_map(|textures| textures.iter()).any(|texture| {
            texture.h3.as_ref().is_some_and(|material| {
                material.animations.iter().any(|animation| animation.moves())
                    || material.h4().is_some_and(|submit| submit.moves())
                    || (material.shield.is_some() && gen3::pipeline::h3_shield_showing(function_values))
            })
        })
}

/// Whether anything draws, lights or simulates outside the materials' passes:
/// any of the preview's features.
pub(super) fn has_effects(preview: &RenderModelPreview) -> bool {
    preview.any_feature(|list| !list.is_empty())
}

fn h2_functions_move(
    data: &ModelPreviewData,
    set: &functions::FunctionInputs,
    time: f32,
) -> bool {
    let held = attached_values(data, time);
    let held_move = data.attached.iter().zip(&held).any(|(attached, (_, values))| {
        attached.functions.move_at_rest(values)
    });
    held_move
        || data
            .object_functions
            .as_ref()
            .is_some_and(|functions| functions.move_at_rest(&h2_function_values(data, set, time)))
}

/// The opened object's functions, then those of what is attached to it.
fn object_functions(
    data: &ModelPreviewData,
) -> impl Iterator<Item = &ce::functions::CeObjectFunctions> {
    data.ce_object_functions
        .iter()
        .chain(data.ce_attached.iter().filter_map(|object| object.functions.as_ref()))
}

fn object_states(
    data: &ModelPreviewData,
    set: &ce::functions::CeFunctionInputs,
    shots: &ce::firing::CeShots,
    time: f32,
) -> Vec<CeObjectState> {
    let state = |functions: Option<&ce::functions::CeObjectFunctions>, object, colors| {
        let (outputs, active) = functions.map_or(([0.0; 4], [false; 4]), |functions| {
            let fired = ce::firing::fired_input(&data.preview.ce.firing, shots, data.geometry_id, object, time);
            let inputs = functions.with_fired(functions.inputs_for(set, (data.geometry_id, object)), fired);
            functions.outputs_active(&inputs, ce::functions::object_time(time, object))
        });
        CeObjectState { outputs, active, change_colors: dx9::passes::ce_change_colors(colors) }
    };
    let mut states = vec![state(data.ce_object_functions.as_ref(), 0, &data.change_colors)];
    for (index, object) in data.ce_attached.iter().enumerate() {
        states.push(state(object.functions.as_ref(), index as u8 + 1, &object.change_colors));
    }
    states
}

impl ModelGpuFrame {
    /// The outputs of the object that owns `material`.
    pub(super) fn outputs_of(&self, material: usize) -> [f32; 4] {
        let object = self.preview.materials.get(material).map_or(0, |material| material.object);
        self.object_states.get(object as usize).map_or([0.0; 4], |state| state.outputs)
    }

    /// The function values of the object that owns `material`.
    pub(super) fn function_values_of(&self, material: usize) -> Option<&functions::FunctionValues> {
        let object = self.preview.materials.get(material).map_or(0, |material| material.object);
        self.function_values_of_object(object)
    }

    pub(super) fn function_values_of_object(&self, object: u8) -> Option<&functions::FunctionValues> {
        if object == 0 {
            return self.function_values.as_ref();
        }
        self.attached_values.iter().find(|(owner, _)| *owner == object).map(|(_, values)| values)
    }

    /// The change colors of the object that owns `material`.
    pub(super) fn change_colors_of(&self, material: usize) -> [[f32; 3]; 4] {
        let object = self.preview.materials.get(material).map_or(0, |material| material.object);
        self.change_colors_of_object(object)
    }

    /// The opened object's change colors by the selected variant; each
    /// attached object has its own, as `object_set_initial_change_colors`
    /// gives every object.
    pub(super) fn change_colors_of_object(&self, object: u8) -> [[f32; 3]; 4] {
        if object == 0 {
            return self.change_colors;
        }
        if let Some((_, colors)) = self.attached_colors.iter().find(|(owner, _)| *owner == object) {
            return *colors;
        }
        self.object_states.get(object as usize).map_or([[1.0; 3]; 4], |state| state.change_colors)
    }

    /// Transparent passes draw whether or not game shaders are on.
    pub(super) fn h2_draws(&self, layer: H2Layer) -> bool {
        match layer {
            H2Layer::Transparent => true,
            H2Layer::ActiveCamo => self.active_camo,
            _ => self.game_shaders,
        }
    }

    pub(super) fn h2_object_prt_clusters(&self) -> Option<[Vec<[f32; 4]>; 2]> {
        let prt = self.preview.h2.prt.as_ref().filter(|_| self.prt && self.game_shaders && !self.default_lights)?;
        let (lights, ambient) = h2_prt_object_lights(self.h2_object_light());
        Some([prt.clusters_lit_by(&lights, ambient), prt.clusters_lit_by(&lights[..1], ambient)])
    }

    /// `lights_prepare_for_object_dynamic`, for the opened object and what it
    /// holds, as one sphere around the model.
    pub(super) fn ce_point_lights(&self) -> Vec<[[f32; 4]; ce::lights::CE_POINT_LIGHT_ROWS]> {
        self.ce_point_lights_with(&[])
    }

    /// The same, with lights the object's effects have made besides.
    pub(super) fn ce_point_lights_with(&self, made: &[ce::lights::CeLight]) -> Vec<[[f32; 4]; ce::lights::CE_POINT_LIGHT_ROWS]> {
        if self.preview.ce.lights.is_empty() && made.is_empty() {
            return Vec::new();
        }
        let lights: std::borrow::Cow<'_, [ce::lights::CeLight]> = if made.is_empty() {
            std::borrow::Cow::Borrowed(&self.preview.ce.lights)
        } else {
            std::borrow::Cow::Owned(self.preview.ce.lights.iter().chain(made).cloned().collect())
        };
        let (low, high) = (self.preview.bounds_min, self.preview.bounds_max);
        let center: [f32; 3] = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
        let reach = (0..3).map(|axis| (high[axis] - center[axis]).powi(2)).sum::<f32>().sqrt();
        let bones = self.bones.as_ref().map(|bones| bones.as_slice());
        let state = |object: u8| {
            let state = self.object_states.get(usize::from(object));
            (state.map_or([0.0; 4], |state| state.outputs),
             state.map_or([[1.0; 3]; 4], |state| state.change_colors))
        };
        (0..self.object_states.len().max(1) as u8)
            .map(|object| {
                ce::lights::point_light_rows(&lights, object, center, reach, state, bones)
            })
            .collect()
    }

    /// Whether anything in the frame changes over time, so the view keeps
    /// redrawing: an effect or simulation that runs, or a pass that animates.
    pub(super) fn moves(&self) -> bool {
        // What draws, lights or simulates outside the passes runs whatever
        // the passes do.
        if self.object_moves && has_effects(&self.preview) {
            return true;
        }
        let Some(textures) = self.textures.as_ref() else {
            return false;
        };
        // Halo 3 has no layers to ask: what moves it is all in `object_moves`.
        if self.object_moves && textures.iter().any(|texture| texture.h3.is_some()) {
            return true;
        }
        self.visible_batches.iter().any(|&index| {
            let Some(batch) = self.preview.batches.get(index) else {
                return false;
            };
            textures
                .get(batch.material_index as usize)
                .is_some_and(|material| {
                    H2Layer::ALL
                        .iter()
                        .filter(|layer| self.h2_draws(**layer))
                        .flat_map(|layer| &material.h2_layers[*layer as usize])
                        .any(|pass| {
                            let outputs = self.outputs_of(batch.material_index as usize);
                            pass.moves()
                                || pass.moves_with(&outputs)
                                || (self.object_moves && pass.reads_outputs())
                        })
                })
        })
    }

    pub(super) fn h2_lightmap_layer(&self) -> H2Layer {
        if self.default_lights { H2Layer::Lightmap } else { H2Layer::ObjectLightmap }
    }

    /// Which way the object path's direct light lies from a surface.
    pub(super) fn h2_object_light(&self) -> [f32; 3] {
        if self.light_follows_camera {
            h2_camera_light(self.camera.yaw, self.camera.pitch)
        } else {
            h2_object_light_direction()
        }
    }
}

/// Over the viewer's shoulder, so orbiting never leaves the near side unlit.
/// The preview's own, in view axes: right, forward and up.
pub(super) fn h2_camera_light(yaw: f32, pitch: f32) -> [f32; 3] {
    const LIGHT: [f32; 3] = [-0.4, -0.7, 0.6];
    let length = LIGHT.iter().map(|value| value * value).sum::<f32>().sqrt();
    unrotate_view_vector(yaw, pitch, LIGHT.map(|value| value / length))
}

thread_local! {
    static MODEL_GL_RENDERER: RefCell<Option<CachedModelGlRenderer>> = const { RefCell::new(None) };
}

struct CachedModelGlRenderer {
    context_id: usize,
    renderer: Result<ModelGlRenderer, String>,
    /// Geometry whose drawing panicked, with what the panic said: it is not
    /// drawn again, and the panel says why.
    failed: HashMap<u64, String>,
}

thread_local! {
    /// Models whose played Halo CE event effects still have something
    /// running, as the last frame drawn left them.
    static CE_EVENTS_RUNNING: RefCell<std::collections::HashSet<u64>> = RefCell::new(Default::default());
}

/// Whether a played event effect on `geometry_id` still needs frames.
fn ce_events_running(geometry_id: u64) -> bool {
    CE_EVENTS_RUNNING.with(|running| running.borrow().contains(&geometry_id))
}

/// The panel's plays of `data`'s event effects, by index among its effects.
pub(super) fn ce_event_plays(
    data: &ModelPreviewData,
    counts: &ce::functions::CeEventCounts,
    scales: [f32; 2],
) -> ce::particles::CeEventPlays {
    let effects = &data.preview.ce.effects;
    let count = |index| counts.get(&(data.geometry_id, index)).copied().unwrap_or(0);
    ce::particles::CeEventPlays { counts: (0..effects.len()).map(count).collect(), scales }
}

/// Why drawing `geometry_id` stopped, if it panicked.
pub(super) fn model_gl_failure(geometry_id: u64) -> Option<String> {
    MODEL_GL_RENDERER.with(|cell| cell.borrow().as_ref()?.failed.get(&geometry_id).cloned())
}

fn paint_model_gl(
    info: egui::PaintCallbackInfo,
    painter: &eframe::egui_glow::Painter,
    frame: &ModelGpuFrame,
) {
    let gl = painter.gl();
    let viewport = info.viewport_in_pixels();
    let clip = info.clip_rect_in_pixels();
    let left = viewport.left_px.max(clip.left_px);
    let bottom = viewport.from_bottom_px.max(clip.from_bottom_px);
    let right = (viewport.left_px + viewport.width_px).min(clip.left_px + clip.width_px);
    let top =
        (viewport.from_bottom_px + viewport.height_px).min(clip.from_bottom_px + clip.height_px);
    if right <= left || top <= bottom {
        return;
    }

    unsafe {
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(left, bottom, right - left, top - bottom);
        gl.color_mask(true, true, true, true);
        gl.depth_mask(true);
        let [r, g, b] = frame.background;
        gl.clear_color(r, g, b, 1.0);
        gl.clear_depth_f32(1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
    }

    MODEL_GL_RENDERER.with(|cell| {
        let context_id = Arc::as_ptr(gl) as usize;
        let mut cached = cell.borrow_mut();
        if cached
            .as_ref()
            .is_none_or(|cached| cached.context_id != context_id)
        {
            let created = ModelGlRenderer::new(gl);
            if let Err(error) = &created {
                eprintln!("{error}");
            }
            *cached = Some(CachedModelGlRenderer {
                context_id,
                renderer: created,
                failed: HashMap::new(),
            });
        }
        let Some(CachedModelGlRenderer { renderer: Ok(renderer), failed, .. }) = cached.as_mut() else {
            return;
        };
        if failed.contains_key(&frame.geometry_id) {
            return;
        }
        // A panic part way through leaves the scene target bound; what the
        // frame drew into goes back, so the rest of the window still draws.
        let state = unsafe { PostScope::capture(gl) };
        let painted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe { renderer.paint(gl, frame) }));
        if let Err(panic) = painted {
            unsafe {
                state.restore(gl);
                gl.use_program(None);
                gl.bind_vertex_array(None);
            }
            let message = crate::app::shell::worker::panic_detail(panic.as_ref()).to_owned();
            eprintln!("model preview: drawing crashed: {message}");
            failed.insert(frame.geometry_id, message);
        }
    });
}

#[cfg(all(test, target_os = "macos"))]
#[path = "headless_shot.rs"]
pub(super) mod headless_shot;
pub(super) use gen3::pipeline::EXPOSURE as H3_EXPOSURE;
pub(super) use super::reach::lighting::{
    REACH_DARK_MULTIPLIER, REACH_SCREEN_NOISE_BITMAP, reach_alt_exposure, reach_exposure, reach_lightmap_tint,
    reach_particle_light,
};

/// Sampler uniform per slot, in `TextureSlot` order — the fragment shader binds
/// texture unit `i` to `SAMPLER_UNIFORMS[i]`.
const SAMPLER_UNIFORMS: [&str; SLOT_COUNT] = [
    "u_tex_base",
    "u_tex_detail",
    "u_tex_bump",
    "u_tex_bump_detail",
    "u_tex_alpha",
    "u_tex_multipurpose",
];

/// The vertex declarations and helpers every model stage shares: all of the
/// modern source before its `main`.
pub(super) fn model_vertex_prelude(version_declaration: &str) -> String {
    let (vertex, _) = model_shader_sources(version_declaration, true, "");
    vertex.split("void main()").next().unwrap_or_default().to_owned()
}

/// Build the two GLSL sources.
///
/// Split out of `RenderCore::new` so the shaders can be checked
/// without a GL context: a compile failure here disables the whole preview
/// and the only signal is a line on stderr, which no test would see.
fn model_shader_sources(
    version_declaration: &str,
    modern: bool,
    precision: &str,
) -> (String, String) {
    let (attribute, varying_out, varying_in, fragment_output, output_name, sample) = if modern {
        (
            "in",
            "out",
            "in",
            "out vec4 out_color;",
            "out_color",
            "texture",
        )
    } else {
        (
            "attribute",
            "varying",
            "varying",
            "",
            "gl_FragColor",
            "texture2D",
        )
    };
    // Position and normal are required; the rest are read only on the
    // shaded path and a driver may drop them from an unused program.
    let bone_rows = MAX_PREVIEW_BONES * 3;
    let vertex_source = format!(
        "{}{precision}\
             {attribute} vec3 a_position;\n\
             {attribute} vec3 a_normal;\n\
             {attribute} vec2 a_texcoord;\n\
             {attribute} vec3 a_tangent;\n\
             {attribute} vec3 a_binormal;\n\
             {attribute} vec4 a_node_indices;\n\
             {attribute} vec4 a_node_weights;\n\
             uniform vec3 u_center;\n\
             uniform float u_scale;\n\
             uniform vec2 u_angles;\n\
             uniform vec2 u_clip_scale;\n\
             uniform float u_depth_scale;\n\
             // Depth remap `z = t * x + y` across the near and far planes.\n\
             // See PREVIEW_RENDER_DISTANCE.\n\
             uniform vec2 u_depth_map;\n\
             // 1 / eye distance (scaled view units); 0 is orthographic.\n\
             uniform float u_perspective;\n\
             // Skinning: three vec4 rows of an affine matrix per bone,\n\
             // bind-relative (world * inverse_bind), so zero-weight vertices\n\
             // and unanimated draws stay exactly where the tag put them.\n\
             uniform float u_animated;\n\
             uniform vec4 u_bones[{bone_rows}];\n\
             {varying_out} vec2 v_uv;\n\
             {varying_out} vec3 v_normal;\n\
             {varying_out} vec3 v_tangent;\n\
             {varying_out} vec3 v_binormal;\n\
             vec3 rotate_view(vec3 value) {{\n\
                 float sy = sin(u_angles.x);\n\
                 float cy = cos(u_angles.x);\n\
                 float sp = sin(u_angles.y);\n\
                 float cp = cos(u_angles.y);\n\
                 vec3 yawed = vec3(value.x * cy - value.y * sy, value.x * sy + value.y * cy, value.z);\n\
                 return vec3(yawed.x, yawed.y * cp - yawed.z * sp, yawed.y * sp + yawed.z * cp);\n\
             }}\n\
             void skin_influence(float index_f, float weight, vec4 point_h,\n\
                                 inout vec3 skinned_pos, inout vec3 skinned_nrm,\n\
                                 inout vec3 skinned_tan, inout vec3 skinned_bin) {{\n\
                 if (weight <= 0.0) {{ return; }}\n\
                 int base = int(index_f + 0.5) * 3;\n\
                 vec4 r0 = u_bones[base];\n\
                 vec4 r1 = u_bones[base + 1];\n\
                 vec4 r2 = u_bones[base + 2];\n\
                 skinned_pos += weight * vec3(dot(r0, point_h), dot(r1, point_h), dot(r2, point_h));\n\
                 skinned_nrm += weight * vec3(dot(r0.xyz, a_normal), dot(r1.xyz, a_normal), dot(r2.xyz, a_normal));\n\
                 skinned_tan += weight * vec3(dot(r0.xyz, a_tangent), dot(r1.xyz, a_tangent), dot(r2.xyz, a_tangent));\n\
                 skinned_bin += weight * vec3(dot(r0.xyz, a_binormal), dot(r1.xyz, a_binormal), dot(r2.xyz, a_binormal));\n\
             }}\n\
             void skinned_pose(out vec3 position, out vec3 normal_in, out vec3 tangent_in, out vec3 binormal_in) {{\n\
                 position = a_position;\n\
                 normal_in = a_normal;\n\
                 tangent_in = a_tangent;\n\
                 binormal_in = a_binormal;\n\
                 float weight_total = a_node_weights.x + a_node_weights.y + a_node_weights.z + a_node_weights.w;\n\
                 if (u_animated > 0.5 && weight_total > 0.0001) {{\n\
                     vec4 point_h = vec4(a_position, 1.0);\n\
                     vec3 sp = vec3(0.0);\n\
                     vec3 sn = vec3(0.0);\n\
                     vec3 st = vec3(0.0);\n\
                     vec3 sb = vec3(0.0);\n\
                     skin_influence(a_node_indices.x, a_node_weights.x, point_h, sp, sn, st, sb);\n\
                     skin_influence(a_node_indices.y, a_node_weights.y, point_h, sp, sn, st, sb);\n\
                     skin_influence(a_node_indices.z, a_node_weights.z, point_h, sp, sn, st, sb);\n\
                     skin_influence(a_node_indices.w, a_node_weights.w, point_h, sp, sn, st, sb);\n\
                     position = sp / weight_total;\n\
                     normal_in = sn;\n\
                     tangent_in = st;\n\
                     binormal_in = sb;\n\
                 }}\n\
             }}\n\
             vec4 clip_position(vec3 position) {{\n\
                 vec3 view = rotate_view((position - u_center) * u_scale);\n\
                 // Perspective: the eye sits along -Y so w grows with depth;\n\
                 // at the focus plane (view.y = 0) w is exactly 1 and the\n\
                 // framing matches the orthographic view. See the derivation\n\
                 // on PERSPECTIVE_FOV_Y.\n\
                 float w = 1.0 + view.y * u_perspective;\n\
                 float z_clip = view.y * u_depth_scale * u_depth_map.x + u_depth_map.y;\n\
                 return vec4(view.x * u_clip_scale.x, view.z * u_clip_scale.y, z_clip, w);\n\
             }}\n\
             void main() {{\n\
                 vec3 position;\n\
                 vec3 normal_in;\n\
                 vec3 tangent_in;\n\
                 vec3 binormal_in;\n\
                 skinned_pose(position, normal_in, tangent_in, binormal_in);\n\
                 gl_Position = clip_position(position);\n\
                 vec3 source_normal = length(normal_in) > 0.0001 ? normalize(normal_in) : vec3(0.0, 0.0, 1.0);\n\
                 v_normal = rotate_view(source_normal);\n\
                 v_tangent = rotate_view(tangent_in);\n\
                 v_binormal = rotate_view(binormal_in);\n\
                 v_uv = a_texcoord;\n\
             }}\n",
        version_declaration
    );
    // The lighting rig is the one this preview has always used — a key, a
    // fill, a rim and an overhead term, all in view space so they follow the
    // camera. It moved from the vertex shader to here so a normal map has
    // something to perturb: per-vertex lighting would sample the map and
    // then throw the result away between vertices.
    let fragment_source = format!(
        "{}{precision}\
             {varying_in} vec2 v_uv;\n\
             {varying_in} vec3 v_normal;\n\
             {varying_in} vec3 v_tangent;\n\
             {varying_in} vec3 v_binormal;\n\
             uniform sampler2D u_tex_base;\n\
             uniform sampler2D u_tex_detail;\n\
             uniform sampler2D u_tex_bump;\n\
             uniform sampler2D u_tex_bump_detail;\n\
             uniform sampler2D u_tex_alpha;\n\
             uniform sampler2D u_tex_multipurpose;\n\
             // Which slots this material bound: 0-3 in `u_have_a`, 4-5 in\n\
             // `u_have_b.xy`. Each slot's UV multiplier is a per-axis pair:\n\
             // base and detail in `_a`, bump and bump detail in `_b`, alpha\n\
             // and multipurpose in `_c`.\n\
             uniform vec4 u_have_a;\n\
             uniform vec4 u_have_b;\n\
             uniform vec4 u_uv_scale_a;\n\
             uniform vec4 u_uv_scale_b;\n\
             uniform vec4 u_uv_scale_c;\n\
             // Detail combine: x the function (0 the Halo 3 linear-space\n\
             // biased multiply; 1-3 Halo CE's biased multiply, multiply and\n\
             // biased add, on stored values), y the CE detail mask (0 none),\n\
             // z the Xbox multipurpose channel order.\n\
             uniform vec4 u_detail;\n\
             uniform vec3 u_base_color;\n\
             uniform float u_unlit;\n\
             uniform float u_shaded;\n\
             {fragment_output}\n\
             vec3 to_linear(vec3 c) {{ return pow(c, vec3(2.2)); }}\n\
             vec3 to_srgb(vec3 c) {{ return pow(clamp(c, 0.0, 1.0), vec3(1.0 / 2.2)); }}\n\
             void main() {{\n\
                 vec3 albedo = u_base_color;\n\
                 bool shaded = u_shaded > 0.5;\n\
                 if (shaded) {{\n\
                     // Alpha test first: a discarded fragment costs nothing else.\n\
                     // This is the ONLY thing that discards. A base map's alpha\n\
                     // is not opacity in Halo — it carries a mask, most often\n\
                     // specular — so treating it as coverage punched holes\n\
                     // through every character whose diffuse had a dark mask.\n\
                     if (u_have_b.x > 0.5 && {sample}(u_tex_alpha, v_uv * u_uv_scale_c.xy).a < 0.5) discard;\n\
                     if (u_have_a.x > 0.5) {{\n\
                         albedo = to_linear({sample}(u_tex_base, v_uv * u_uv_scale_a.xy).rgb);\n\
                     }}\n\
                     // Halo detail maps are grey-centred and modulate: mid-grey\n\
                     // leaves the base untouched, darker and lighter push it.\n\
                     // They tile — commonly sixteen times — so the scale matters.\n\
                     if (u_have_a.y > 0.5) {{\n\
                         vec3 detail = {sample}(u_tex_detail, v_uv * u_uv_scale_a.zw).rgb;\n\
                         if (u_detail.x < 0.5) {{\n\
                             albedo = clamp(albedo * to_linear(detail) * 2.0, 0.0, 1.0);\n\
                         }} else {{\n\
                             // Halo CE's ShaderModel() (fx/model_common.h): the\n\
                             // mask lerps the detail toward its function's\n\
                             // neutral value, then combines with the base.\n\
                             float mask = 1.0;\n\
                             if (u_detail.y > 0.5 && u_have_b.y > 0.5) {{\n\
                                 vec4 mp = {sample}(u_tex_multipurpose, v_uv * u_uv_scale_c.zw);\n\
                                 if (u_detail.z > 0.5) mp = mp.agrb;\n\
                                 float k = u_detail.y;\n\
                                 float channel = k < 2.5 ? mp.b : (k < 4.5 ? mp.g : (k < 6.5 ? mp.a : mp.r));\n\
                                 mask = mod(k, 2.0) > 0.5 ? 1.0 - channel : channel;\n\
                             }}\n\
                             vec3 neutral = u_detail.x > 1.5 && u_detail.x < 2.5 ? vec3(1.0) : vec3(0.5);\n\
                             vec3 d = mix(neutral, detail, mask);\n\
                             vec3 base = to_srgb(albedo);\n\
                             vec3 combined = u_detail.x < 1.5 ? base * d * 2.0\n\
                                 : (u_detail.x < 2.5 ? base * d : base + 2.0 * d - 1.0);\n\
                             albedo = to_linear(clamp(combined, 0.0, 1.0));\n\
                         }}\n\
                     }}\n\
                 }}\n\
                 vec3 normal = normalize(v_normal);\n\
                 if (shaded && (u_have_a.z > 0.5 || u_have_a.w > 0.5)) {{\n\
                     // Tangent space, using the frame the tag authored rather\n\
                     // than a reconstructed one — mirrored UV islands need the\n\
                     // binormal's own sign to come out the right way round.\n\
                     vec3 t = normalize(v_tangent);\n\
                     vec3 b = normalize(v_binormal);\n\
                     if (length(v_tangent) > 0.0001 && length(v_binormal) > 0.0001) {{\n\
                         // The engine's own unpack, verbatim from the kit's\n\
                         // source (rasterizer/hlsl/bump_mapping.fx): X and Y\n\
                         // come off the texture through BUMP_CONVERT — byte\n\
                         // 128 is exactly flat — Z is ALWAYS reconstructed\n\
                         // rather than sampled (the compressed formats carry\n\
                         // no trustworthy blue), and the components go into\n\
                         // the authored tangent frame with no negation.\n\
                         vec3 tn = vec3(0.0, 0.0, 1.0);\n\
                         if (u_have_a.z > 0.5) {{\n\
                             vec2 sampled = {sample}(u_tex_bump, v_uv * u_uv_scale_b.xy).xy * (255.0 / 127.0) - (128.0 / 127.0);\n\
                             tn = vec3(sampled, sqrt(1.0 - min(dot(sampled, sampled), 1.0)));\n\
                         }}\n\
                         // A detail normal tiles far finer than the base one\n\
                         // and adds its tangent-plane components over it —\n\
                         // the engine's stock combine — staying stable when\n\
                         // either map is flat.\n\
                         if (u_have_a.w > 0.5) {{\n\
                             vec2 dn = {sample}(u_tex_bump_detail, v_uv * u_uv_scale_b.zw).xy * (255.0 / 127.0) - (128.0 / 127.0);\n\
                             tn = vec3(tn.xy + dn, tn.z);\n\
                         }}\n\
                         tn = normalize(tn);\n\
                         normal = normalize(t * tn.x + b * tn.y + normal * tn.z);\n\
                     }}\n\
                 }}\n\
                 // The rig this preview has always used — a key, a fill, a rim\n\
                 // and an overhead term, in view space so they follow the camera.\n\
                 // Neutral on purpose: the maps supply the colour, and anything\n\
                 // that tried to supply more needed a scene this preview has not\n\
                 // got.\n\
                 float key = max(dot(normal, normalize(vec3(-0.35, -0.55, 0.76))), 0.0);\n\
                 float fill = max(dot(normal, normalize(vec3(0.72, 0.22, 0.36))), 0.0);\n\
                 float rim = pow(clamp(1.0 - abs(normal.y), 0.0, 1.0), 2.0);\n\
                 float overhead = clamp(normal.z * 0.5 + 0.5, 0.0, 1.0);\n\
                 float shade = clamp(0.42 + key * 0.46 + fill * 0.16 + rim * 0.10 + overhead * 0.08, 0.32, 1.22);\n\
                 vec3 lit = albedo * shade + vec3(key * key * (22.0 / 255.0));\n\
                 vec3 flat_color = shaded ? to_srgb(albedo) : u_base_color;\n\
                 vec3 result = mix(to_srgb(lit), flat_color, u_unlit);\n\
                 {output_name} = vec4(result, 1.0);\n\
             }}\n",
        version_declaration
    );

    (vertex_source, fragment_source)
}

/// Ground-reference grid on the z = 0 plane (the world ground in Halo's
/// Z-up space), Blender-style: minor lines at a spacing picked from the
/// model's size, plus the world X and Y axes across the same extent. Returns
/// the line-list vertices and the index where the axis vertices begin —
/// minor lines first, then two vertices of the X axis, then two of the Y
/// axis — so the axes can draw in their own colors.
fn grid_line_vertices(preview: &RenderModelPreview) -> (Vec<RenderModelPreviewVertex>, usize) {
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

/// Wire the full `RenderModelPreviewVertex` layout into the currently bound
/// vertex array. Shared by the model and grid buffers so no attribute is
/// ever left disabled and reading a stale constant (a disabled array reads
/// `(0, 0, 0, 1)` — a phantom skinning weight on bone 1).
unsafe fn bind_preview_vertex_layout(
    gl: &glow::Context,
    program: glow::NativeProgram,
) -> Result<(), String> {
    let stride = std::mem::size_of::<RenderModelPreviewVertex>() as i32;
    unsafe {
        let position = gl
            .get_attrib_location(program, "a_position")
            .ok_or_else(|| "model preview shader has no position attribute".to_owned())?;
        let normal = gl
            .get_attrib_location(program, "a_normal")
            .ok_or_else(|| "model preview shader has no normal attribute".to_owned())?;
        gl.enable_vertex_attrib_array(position);
        gl.vertex_attrib_pointer_f32(position, 3, glow::FLOAT, false, stride, 0);
        gl.enable_vertex_attrib_array(normal);
        gl.vertex_attrib_pointer_f32(normal, 3, glow::FLOAT, false, stride, 12);
        // Optional, unlike the two above: the untextured paths (particle
        // models, Chimp's UE meshes) leave these at zero, and a driver is
        // free to optimise an unread attribute out of the program entirely.
        // A missing location is therefore not an error — offsets are pinned
        // by `gpu_vertex_layout_matches_the_hand_written_attribute_offsets`.
        for (name, size, offset) in [
            ("a_texcoord", 2, 24),
            ("a_tangent", 3, 32),
            ("a_binormal", 3, 44),
            // Skinning influences ride as floats: GLSL 100 has no integer
            // vertex attributes, and the shader casts the indices.
            ("a_node_indices", 4, 56),
            ("a_node_weights", 4, 72),
        ] {
            if let Some(location) = gl.get_attrib_location(program, name) {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(location, size, glow::FLOAT, false, stride, offset);
            }
        }
    }
    Ok(())
}

const H2_SHADOW_MAP_SIZE: i32 = 2048;
/// Past the units a pass's own samplers take.
const H2_SHADOW_MAP_UNIT: u32 = 8;
const H3_NORMAL_TARGET_UNIT: u32 = 9;
const H2_GEL_MASK_UNIT: u32 = 10;
/// `c_rasterizer::render_globals.k_shadow_resolution`.
pub(super) const H3_SHADOW_MAP_SIZE: i32 = 512;
/// `set_shadow_generate_mode`'s bias, the second in depth units.
const H3_SHADOW_SLOPE_BIAS: f32 = 1.414;
const H3_SHADOW_DEPTH_BIAS: f32 = 0.003;

/// A depth texture and the framebuffer to fill it through.
unsafe fn h2_shadow_map(gl: &glow::Context) -> Option<(glow::NativeFramebuffer, glow::NativeTexture)> {
    unsafe {
        let draw = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
        let read = gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING);
        let mut made = Made::new(gl);
        let size = [H2_SHADOW_MAP_SIZE; 2];
        let map = made.depth(size, Texels::DEPTH24, Sampling::NEAREST_CLAMPED, glow::DEPTH_ATTACHMENT);
        let complete = map.as_ref().is_some_and(|map| made.complete(&[map.framebuffer]));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
        let map = map.filter(|_| complete)?;
        made.keep();
        Some((map.framebuffer, map.texture))
    }
}

/// The view from the light, an orthographic box around the whole model.
fn h2_shadow_matrix(preview: &RenderModelPreview, toward: [f32; 3]) -> [f32; 16] {
    let center: [f32; 3] =
        std::array::from_fn(|axis| (preview.bounds_min[axis] + preview.bounds_max[axis]) * 0.5);
    let radius = (0..3)
        .map(|axis| (preview.bounds_max[axis] - preview.bounds_min[axis]).powi(2))
        .sum::<f32>()
        .sqrt()
        .max(1e-4)
        * 0.55;
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    };
    let unit = |v: [f32; 3]| {
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
        [v[0] / length, v[1] / length, v[2] / length]
    };
    let forward = toward.map(|value| -value);
    let right = unit(cross(forward, [0.0, 0.0, 1.0]));
    let up = cross(right, forward);
    let dot = |a: [f32; 3]| a[0] * center[0] + a[1] * center[1] + a[2] * center[2];
    let row = |axis: [f32; 3]| {
        [axis[0] / radius, axis[1] / radius, axis[2] / radius, -dot(axis) / radius]
    };
    let [x, y, z] = [row(right), row(up), row(forward)];
    [
        x[0], y[0], z[0], 0.0,
        x[1], y[1], z[1], 0.0,
        x[2], y[2], z[2], 0.0,
        x[3], y[3], z[3], 1.0,
    ]
}

/// Above the attributes the main program takes, which it numbers itself.
const PRT_CLUSTER_ATTRIBUTE: u32 = 14;
const PRT_WEIGHTS_ATTRIBUTE: u32 = 15;

/// How many models stay on the GPU at once: geometry, ground grid and
/// textures. Past this, the one drawn longest ago gives up its slot.
const MODEL_GPU_SLOTS: usize = 4;

/// Which slot each model draws from, least recently drawn out first.
///
/// A slot per model keeps two previews on screen from replacing each other's
/// upload every frame, which would re-send both models' vertices, indices
/// and textures to the GPU each time.
#[derive(Default)]
struct ModelSlotLru {
    /// `(geometry id, last drawn)`, one per slot in use.
    slots: Vec<(u64, u64)>,
    clock: u64,
}

impl ModelSlotLru {
    /// The slot for `geometry_id`, and whether it must be uploaded: a new
    /// slot, or one taken from the model drawn longest ago.
    fn acquire(&mut self, geometry_id: u64, capacity: usize) -> (usize, bool) {
        self.clock += 1;
        if let Some(index) = self.slots.iter().position(|(id, _)| *id == geometry_id) {
            self.slots[index].1 = self.clock;
            return (index, false);
        }
        if self.slots.len() < capacity.max(1) {
            self.slots.push((geometry_id, self.clock));
            return (self.slots.len() - 1, true);
        }
        let index = self
            .slots
            .iter()
            .enumerate()
            .min_by_key(|(_, (_, used))| *used)
            .map(|(index, _)| index)
            .unwrap_or(0);
        self.slots[index] = (geometry_id, self.clock);
        (index, true)
    }

    /// Forget a slot that could not be created.
    fn abandon(&mut self, index: usize) {
        if index + 1 == self.slots.len() {
            self.slots.pop();
        }
    }
}

/// A slot's simulations: what each carries from frame to frame, restarted
/// together when the slot takes another model.
#[derive(Default)]
pub(in crate::app::model_preview) struct SlotSimulations {
    /// Halo 2's and the gen3 games' particle systems.
    pub(in crate::app::model_preview) particles: particles::H3ParticleField,
    /// Reach's cheap particles.
    pub(in crate::app::model_preview) reach_cheap: particles::cheap::ReachCheapField,
    pub(in crate::app::model_preview) h3_strips: gen3::strips::H3StripField,
    /// When each effect began, for Halo 2's and the gen3 games' effect parts.
    pub(in crate::app::model_preview) effect_onsets: effects::EffectOnsets,
    /// Halo 2's and the gen3 games' cloth.
    pub(in crate::app::model_preview) cloth: cloth::H3ClothField,
    /// Every antenna's spring.
    pub(in crate::app::model_preview) antennas: antennas::spring::ReachAntennaField,
}

impl SlotSimulations {
    /// Each simulation by name, for the G-sim gate. Taken apart whole, so one
    /// added without a name here does not compile.
    #[cfg(test)]
    fn each(&self, mut visit: impl FnMut(&str, &dyn std::fmt::Debug)) {
        let Self {
            particles,
            reach_cheap,
            h3_strips,
            effect_onsets,
            cloth,
            antennas,
        } = self;
        visit("particles", particles);
        visit("reach_cheap", reach_cheap);
        visit("h3_strips", h3_strips);
        visit("effect_onsets", effect_onsets);
        visit("cloth", cloth);
        visit("antennas", antennas);
    }
}

/// One model's buffers, ground grid and textures on the GPU.
pub(super) struct ModelGpuSlot {
    /// What runs from frame to frame, restarted together.
    pub(in crate::app::model_preview) simulations: SlotSimulations,
    pub(in crate::app::model_preview) vertex_array: glow::NativeVertexArray,
    vertex_buffer: glow::NativeBuffer,
    /// Halo 2 per-vertex PRT, unused by every other model.
    prt_buffer: glow::NativeBuffer,
    index_buffer: glow::NativeBuffer,
    /// The ground grid's own vertex array and buffer, built per geometry
    /// (its spacing and extent follow the model's bounds).
    grid_vertex_array: glow::NativeVertexArray,
    grid_vertex_buffer: glow::NativeBuffer,
    grid_vertex_count: i32,
    /// Vertex index where the world-axis lines begin; see `grid_line_vertices`.
    grid_axis_start: i32,
    grid_built: bool,
    /// The `mesh_id` the buffers hold.
    mesh_id: Option<u64>,
    /// How many models the slot has taken, which each game's own state for
    /// the slot follows.
    uploads: u64,
    /// Per batch, where its transparent passes sort from.
    pub(in crate::app::model_preview) h2_sort_points: Vec<[f32; 3]>,
    /// GL textures per material, in `RenderModelPreview::materials` order.
    materials: Vec<MaterialGlTextures>,
    /// The `textures_id` the uploaded `materials` belong to, so a reloaded
    /// model, or another model given this slot, re-uploads rather than drawing
    /// the wrong textures.
    uploaded_textures: Option<u64>,
    /// Halo 3's and Halo 4's lens flares: per flare, then per image, of
    /// `flare_geometry`.
    pub(in crate::app::model_preview) flare_textures: Vec<Vec<Option<glow::NativeTexture>>>,
    pub(in crate::app::model_preview) h3_textures: gen3::pipeline::H3SlotTextures,
    flare_geometry: Option<u64>,
    pub(in crate::app::model_preview) antenna_textures: Vec<Option<glow::NativeTexture>>,
    pub(in crate::app::model_preview) volume_textures: Vec<Option<glow::NativeTexture>>,
    /// The cheap particles' library's render texture array.
    pub(in crate::app::model_preview) reach_cheap_texture: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) strip_textures: Vec<[Option<glow::NativeTexture>; 4]>,
    pub(in crate::app::model_preview) particle_textures: Vec<[Option<glow::NativeTexture>; 4]>,
}

impl ModelGpuSlot {
    unsafe fn new(gl: &glow::Context, program: glow::NativeProgram) -> Result<Self, String> {
        unsafe {
            let vertex_array = gl
                .create_vertex_array()
                .map_err(|error| error.to_string())?;
            let vertex_buffer = gl.create_buffer().map_err(|error| error.to_string())?;
            let prt_buffer = gl.create_buffer().map_err(|error| error.to_string())?;
            let index_buffer = gl.create_buffer().map_err(|error| error.to_string())?;
            gl.bind_vertex_array(Some(vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertex_buffer));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(index_buffer));
            bind_preview_vertex_layout(gl, program)?;
            gl.bind_vertex_array(None);

            let grid_vertex_array = gl
                .create_vertex_array()
                .map_err(|error| error.to_string())?;
            let grid_vertex_buffer = gl.create_buffer().map_err(|error| error.to_string())?;
            gl.bind_vertex_array(Some(grid_vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(grid_vertex_buffer));
            bind_preview_vertex_layout(gl, program)?;
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            Ok(Self {
                mesh_id: None,
                uploads: 0,
                h2_sort_points: Vec::new(),
                vertex_array,
                vertex_buffer,
                prt_buffer,
                index_buffer,
                grid_vertex_array,
                grid_vertex_buffer,
                grid_vertex_count: 0,
                grid_axis_start: 0,
                grid_built: false,
                materials: Vec::new(),
                uploaded_textures: None,
                flare_textures: Vec::new(),
                h3_textures: Default::default(),
                flare_geometry: None,
                simulations: SlotSimulations::default(),
                antenna_textures: Vec::new(),
                volume_textures: Vec::new(),
                reach_cheap_texture: None,
                strip_textures: Vec::new(),
                particle_textures: Vec::new(),
            })
        }
    }

    /// Write `moved` (places, normals and, where given, binormals) over the
    /// uploaded vertices from `first` on, the rest of each vertex as loaded.
    /// A range the preview does not have (a simulation left from another
    /// model) writes nothing.
    pub(in crate::app::model_preview) unsafe fn rewrite_vertices(
        &self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        first: usize,
        moved: impl IntoIterator<Item = cloth::MovedClothVertex>,
    ) {
        let moved: Vec<_> = moved.into_iter().collect();
        let Some(loaded) = frame.preview.vertices.get(first..first + moved.len()) else {
            return;
        };
        let mut vertices = loaded.to_vec();
        for (vertex, (position, normal, binormal)) in vertices.iter_mut().zip(moved) {
            vertex.position = position;
            vertex.normal = normal;
            if let Some(binormal) = binormal {
                vertex.binormal = binormal;
            }
        }
        let offset = first * size_of::<RenderModelPreviewVertex>();
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertex_buffer));
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, offset as i32, slice_bytes(&vertices));
        }
    }

    unsafe fn upload_geometry(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        self.mesh_id = Some(frame.mesh_id);
        self.h2_sort_points = h2_sort_points(&frame.preview);
        unsafe {
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertex_buffer));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                slice_bytes(&frame.preview.vertices),
                glow::STATIC_DRAW,
            );
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.index_buffer));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                slice_bytes(&frame.preview.indices),
                glow::STATIC_DRAW,
            );
            match &frame.preview.h2.prt {
                Some(prt) => {
                    gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.prt_buffer));
                    gl.buffer_data_u8_slice(
                        glow::ARRAY_BUFFER,
                        slice_bytes(&prt.vertices),
                        glow::STATIC_DRAW,
                    );
                    let stride = std::mem::size_of::<[f32; 5]>() as i32;
                    gl.enable_vertex_attrib_array(PRT_CLUSTER_ATTRIBUTE);
                    gl.vertex_attrib_pointer_f32(
                        PRT_CLUSTER_ATTRIBUTE,
                        1,
                        glow::FLOAT,
                        false,
                        stride,
                        0,
                    );
                    gl.enable_vertex_attrib_array(PRT_WEIGHTS_ATTRIBUTE);
                    gl.vertex_attrib_pointer_f32(
                        PRT_WEIGHTS_ATTRIBUTE,
                        4,
                        glow::FLOAT,
                        false,
                        stride,
                        4,
                    );
                }
                None => {
                    gl.disable_vertex_attrib_array(PRT_CLUSTER_ATTRIBUTE);
                    gl.disable_vertex_attrib_array(PRT_WEIGHTS_ATTRIBUTE);
                }
            }
            match &frame.preview.gen3.prt {
                Some(prt) => {
                    gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.prt_buffer));
                    gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(&prt.vertices), glow::STATIC_DRAW);
                    let stride = std::mem::size_of::<[f32; gen3::prt::COEFFICIENTS]>() as i32;
                    for (index, attribute) in gen3::pipeline::PRT_ATTRIBUTES.into_iter().enumerate() {
                        gl.enable_vertex_attrib_array(attribute);
                        // The first holds a fourth for the linear shader, in the
                        // second's first place.
                        let size = if index == 0 { 4 } else { 3 };
                        gl.vertex_attrib_pointer_f32(attribute, size, glow::FLOAT, false, stride, index as i32 * 12);
                    }
                }
                None => {
                    for attribute in gen3::pipeline::PRT_ATTRIBUTES {
                        gl.disable_vertex_attrib_array(attribute);
                    }
                }
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertex_buffer));
        }
        self.grid_built = false;
    }

    /// Upload the maps of the sprites, flares, antennas and effects the
    /// slot's geometry draws, once per geometry.
    pub(in crate::app::model_preview) unsafe fn sync_sprite_textures(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        if self.flare_geometry == Some(frame.geometry_id) {
            return;
        }
        unsafe {
            let drained = self
                .particle_textures
                .drain(..)
                .chain(self.strip_textures.drain(..));
            for texture in drained.flatten().flatten() {
                gl.delete_texture(texture);
            }
            let maps = |look: &particles::H3SpriteShader| sprite_maps(gl, look);
            self.particle_textures =
                frame.preview.particles.iter().map(|system| maps(&system.look.shader)).collect();
            self.strip_textures = frame.preview.gen3.strips.iter().map(|strip| maps(strip.shader())).collect();
            if let Some(texture) = self.reach_cheap_texture.take() {
                gl.delete_texture(texture);
            }
            let render = frame.preview.gen3.cheap_emitters.first().and_then(|at| at.library().render_texture());
            self.reach_cheap_texture = render.and_then(|render| upload_cheap_render_texture(gl, render));
            for texture in self.flare_textures.drain(..).flatten().flatten() {
                gl.delete_texture(texture);
            }
            self.flare_textures = frame
                .preview
                .gen3.flares
                .iter()
                .map(|at| {
                    let (image, texels, mips) = at.bitmap();
                    vec![gen3::pipeline::upload_h2_image_linear(gl, image, *texels, mips, None)]
                })
                .chain(h4::flare_pass::h4_flare_images(gl, frame))
                .collect();
            for texture in self.antenna_textures.drain(..).flatten() {
                gl.delete_texture(texture);
            }
            self.antenna_textures = frame
                .preview
                .antennas
                .iter()
                .map(|antenna| match &antenna.h3_bitmap {
                    // `build_sprite`. Reach's `draw_textured_transparent_quad`
                    // sets address mode 1 and filter mode 2, taken to be
                    // clamped and trilinear. [inferred]
                    Some((image, texels, mips)) => {
                        let filter =
                            if antenna.is_reach() { BitmapFilterMode::Trilinear } else { BitmapFilterMode::Bilinear };
                        let sampler = H3Sampler { address: [BitmapAddressMode::Clamp; 2], filter };
                        gen3::pipeline::upload_h2_image_linear(gl, image, *texels, mips, Some(sampler))
                    }
                    None => upload_h2_image(gl, &antenna.image),
                })
                .collect();
            for texture in self.volume_textures.drain(..).flatten() {
                gl.delete_texture(texture);
            }
            self.volume_textures = frame
                .preview
                .gen3.light_volumes
                .iter()
                .map(|at| {
                    let (image, texels, mips) = at.bitmap()?;
                    gen3::pipeline::upload_h2_image_linear(gl, image, *texels, mips, at.sampler())
                })
                .collect();
            self.flare_geometry = Some(frame.geometry_id);
        }
    }

    /// Start every simulation over, for a model new to this slot.
    ///
    /// The fields restart on their own only when their count changes or time
    /// goes back, so a slot taken over by another model would step the last
    /// model's particles, flags and cloth against the new model's definitions.
    /// How many models the slot has taken.
    pub(in crate::app::model_preview) fn uploads(&self) -> u64 {
        self.uploads
    }

    fn restart_simulations(&mut self) {
        self.simulations = SlotSimulations::default();
    }

    /// Build the ground grid for this slot's model, once.
    unsafe fn sync_grid(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        if self.grid_built {
            return;
        }
        let (vertices, axis_start) = grid_line_vertices(&frame.preview);
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.grid_vertex_buffer));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                slice_bytes(&vertices),
                glow::STATIC_DRAW,
            );
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
        self.grid_axis_start = axis_start as i32;
        self.grid_vertex_count = vertices.len() as i32;
        self.grid_built = true;
    }

    /// Bring the slot's textures in line with the frame's, uploading only
    /// when they changed or arrived.
    unsafe fn sync_textures(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        // Untextured modes keep what is uploaded: switching back must not
        // upload every map again in the paint callback. They draw without it
        // (see `bound_materials`).
        if frame.textures.is_none() {
            return;
        }
        let wanted = Some(frame.textures_id);
        if self.uploaded_textures == wanted {
            return;
        }
        unsafe { self.release_textures(gl) };
        self.uploaded_textures = wanted;
        let Some(textures) = frame.textures.as_ref() else {
            return;
        };
        self.materials = textures
            .iter()
            .map(|material| {
                let mut uploaded = MaterialGlTextures::default();
                for slot in ALL_SLOTS {
                    let index = slot as usize;
                    let Some(image) = material.get(slot) else {
                        continue;
                    };
                    uploaded.uv_scales[index] = image.scale;
                    uploaded.textures[index] = unsafe { upload_texture(gl, image) };
                }
                uploaded.detail = material.detail;
                let upload = |passes: &[H2DrawPass]| -> Vec<Vec<Option<glow::NativeTexture>>> {
                    passes
                        .iter()
                        .map(|pass| {
                            pass.textures
                                .iter()
                                .map(|texture| {
                                    let image = texture.image.as_ref()?;
                                    unsafe { upload_h2_image(gl, image) }
                                })
                                .collect()
                        })
                        .collect()
                };
                uploaded.h2_textures = std::array::from_fn(|layer| upload(&material.h2_layers[layer]));
                uploaded
            })
            .collect();
    }

    unsafe fn release_textures(&mut self, gl: &glow::Context) {
        for material in self.materials.drain(..) {
            let passes = material.h2_textures.into_iter().flatten().flatten().flatten();
            for texture in material.textures.into_iter().flatten().chain(passes) {
                unsafe { gl.delete_texture(texture) };
            }
        }
    }
}

/// A color and depth target at a pane's size. One holds the whole scene while
/// game shaders are on, because the environment layer blends by destination
/// alpha, which the window's framebuffer need not keep. The other is texaccum,
/// for lightmap passes to read back at each pixel's screen position.
pub(in crate::app::model_preview) struct H2Target {
    pub(in crate::app::model_preview) framebuffer: glow::NativeFramebuffer,
    color: glow::NativeTexture,
    pub(in crate::app::model_preview) depth: glow::NativeRenderbuffer,
    pub(in crate::app::model_preview) size: [i32; 2],
}

impl H2Target {
    unsafe fn new(gl: &glow::Context, width: i32, height: i32) -> Option<Self> {
        let size = [width, height];
        let mut made = Made::new(gl);
        unsafe {
            let color = made.texture(size, Texels::RGBA8, Sampling::NEAREST_CLAMPED)?;
            let depth = made.renderbuffer(size, glow::DEPTH24_STENCIL8)?;
            let framebuffer = made.framebuffer(&[(glow::COLOR_ATTACHMENT0, color)])?;
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_STENCIL_ATTACHMENT, glow::RENDERBUFFER, Some(depth));
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                eprintln!("model preview: render target incomplete: {status:#x}");
                return None;
            }
            made.keep();
            Some(Self { framebuffer, color, depth, size })
        }
    }

    unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_texture(self.color);
            gl.delete_renderbuffer(self.depth);
        }
    }
}

/// The clear color of the preview, shared with the scene target.
/// The theme's panel color, so a preview sits in the UI around it.
fn preview_background(visuals: &egui::Visuals) -> [f32; 3] {
    let [r, g, b, _] = visuals.panel_fill.to_array();
    [r, g, b].map(|channel| channel as f32 / 255.0)
}

fn background_is_dark(background: [f32; 3]) -> bool {
    background.iter().sum::<f32>() / 3.0 < 0.5
}

/// Minor grid lines: a step off the background, toward the contrast.
fn grid_line_color(background: [f32; 3]) -> [f32; 3] {
    const STEP: f32 = 0.11;
    let step = if background_is_dark(background) { STEP } else { -STEP };
    background.map(|channel| (channel + step).clamp(0.0, 1.0))
}

/// Link a pass with the main program's attribute locations, because the
/// model's vertex array is bound to those.
unsafe fn link_h2_program(
    gl: &glow::Context,
    main: glow::NativeProgram,
    vertex_source: &str,
    fragment_source: &str,
) -> Result<glow::NativeProgram, String> {
    unsafe {
        let mut attributes = Vec::new();
        for name in [
            "a_position",
            "a_normal",
            "a_texcoord",
            "a_tangent",
            "a_binormal",
            "a_node_indices",
            "a_node_weights",
        ] {
            if let Some(location) = gl.get_attrib_location(main, name) {
                attributes.push((location, name));
            }
        }
        attributes.push((PRT_CLUSTER_ATTRIBUTE, "a_prt_cluster"));
        attributes.push((PRT_WEIGHTS_ATTRIBUTE, "a_prt_weights"));
        let stages = [(glow::VERTEX_SHADER, vertex_source), (glow::FRAGMENT_SHADER, fragment_source)];
        link(gl, &stages, &attributes).map_err(|error| format!("Halo 2 pass: {error}"))
    }
}

fn gl_blend_factor(factor: H2BlendFactor) -> u32 {
    match factor {
        H2BlendFactor::Zero => glow::ZERO,
        H2BlendFactor::One => glow::ONE,
        H2BlendFactor::SrcColor => glow::SRC_COLOR,
        H2BlendFactor::OneMinusSrcColor => glow::ONE_MINUS_SRC_COLOR,
        H2BlendFactor::SrcAlpha => glow::SRC_ALPHA,
        H2BlendFactor::OneMinusSrcAlpha => glow::ONE_MINUS_SRC_ALPHA,
        H2BlendFactor::DstColor => glow::DST_COLOR,
        H2BlendFactor::OneMinusDstColor => glow::ONE_MINUS_DST_COLOR,
        H2BlendFactor::DstAlpha => glow::DST_ALPHA,
        H2BlendFactor::OneMinusDstAlpha => glow::ONE_MINUS_DST_ALPHA,
        H2BlendFactor::SrcAlphaSaturate => glow::SRC_ALPHA_SATURATE,
        H2BlendFactor::ConstantColor => glow::CONSTANT_COLOR,
        H2BlendFactor::OneMinusConstantColor => glow::ONE_MINUS_CONSTANT_COLOR,
        H2BlendFactor::ConstantAlpha => glow::CONSTANT_ALPHA,
        H2BlendFactor::OneMinusConstantAlpha => glow::ONE_MINUS_CONSTANT_ALPHA,
    }
}

fn gl_blend_equation(equation: H2BlendEquation) -> u32 {
    match equation {
        H2BlendEquation::Add => glow::FUNC_ADD,
        H2BlendEquation::Subtract => glow::FUNC_SUBTRACT,
        H2BlendEquation::ReverseSubtract => glow::FUNC_REVERSE_SUBTRACT,
        H2BlendEquation::Min => glow::MIN,
        H2BlendEquation::Max => glow::MAX,
    }
}

fn gl_compare(compare: H2Compare) -> u32 {
    match compare {
        H2Compare::Never => glow::NEVER,
        H2Compare::Less => glow::LESS,
        H2Compare::Equal => glow::EQUAL,
        H2Compare::LessOrEqual => glow::LEQUAL,
        H2Compare::Greater => glow::GREATER,
        H2Compare::NotEqual => glow::NOTEQUAL,
        H2Compare::GreaterOrEqual => glow::GEQUAL,
        H2Compare::Always => glow::ALWAYS,
    }
}

/// Where a batch sorts from. The game keys a part by the depth of its rigid
/// node, and a skinned part by node 0, so skinned parts keep their order among
/// themselves: they all share one point here, and the sort is stable.
fn h2_sort_points(preview: &RenderModelPreview) -> Vec<[f32; 3]> {
    let center = preview.vertices.iter().fold([0.0f32; 3], |sum, vertex| {
        std::array::from_fn(|axis| sum[axis] + vertex.position[axis])
    });
    let count = preview.vertices.len().max(1) as f32;
    let center = center.map(|sum| sum / count);
    preview
        .batches
        .iter()
        .map(|batch| {
            let start = batch.index_start as usize;
            let end = start + batch.index_count as usize;
            let indices = preview.indices.get(start..end).unwrap_or(&[]);
            let vertex = |index: &u32| preview.vertices.get(*index as usize);
            let rigid = indices.first().and_then(vertex).is_some_and(|first| {
                indices.iter().filter_map(vertex).all(|other| {
                    other.node_indices[0] == first.node_indices[0]
                        && other.node_weights[1..].iter().all(|weight| *weight <= 0.0)
                })
            });
            if !rigid || indices.is_empty() {
                return center;
            }
            let sum = indices.iter().filter_map(vertex).fold([0.0f32; 3], |sum, vertex| {
                std::array::from_fn(|axis| sum[axis] + vertex.position[axis])
            });
            sum.map(|sum| sum / indices.len() as f32)
        })
        .collect()
}

/// Visible batches farthest first; the eye looks along +Y in view space.
pub(in crate::app::model_preview) fn h2_draw_order(frame: &ModelGpuFrame, points: &[[f32; 3]]) -> Vec<usize> {
    let camera = &frame.camera;
    let depth = |index: usize| {
        let point = points.get(index).copied().unwrap_or(camera.center);
        let offset = std::array::from_fn(|axis| point[axis] - camera.center[axis]);
        rotate_view_vector(camera.yaw, camera.pitch, offset)[1]
    };
    let mut order = frame.visible_batches.clone();
    order.sort_by(|a, b| depth(*b).total_cmp(&depth(*a)));
    order
}

/// Whether a context can compile `H2_GLSL_VERSION`. This asks the context, not
/// egui's `ShaderVersion`: that only sorts contexts into the dialects egui
/// itself uses, and calls everything from GL 3.1 up `#version 140`.
fn h2_context_supported(major: u32, minor: u32, embedded: bool) -> bool {
    !embedded && (major, minor) >= (3, 3)
}

/// The eye in model space, which the pass stages measure Fresnel from. An
/// orthographic view has no eye, so one far away along the view axis stands in.
pub(super) fn h2_eye_position(camera: &ModelGpuCamera) -> [f32; 3] {
    let distance = if camera.perspective > 0.0 { 1.0 / camera.perspective } else { 1.0e5 };
    let eye = unrotate_view_vector(camera.yaw, camera.pitch, [0.0, -distance, 0.0]);
    std::array::from_fn(|axis| camera.center[axis] + eye[axis] / camera.scale.max(1e-6))
}

/// Polygon offset units in one whole of window depth, on a 24 bit buffer.
pub(in crate::app::model_preview) const DEPTH_UNITS: f32 = (1 << 24) as f32;

/// The vertex stream of Halo CE's lens flares: clip position, then texcoord.
const CE_FLARE_STRIDE: i32 = 6 * 4;

/// `view_depth`: a window depth back to the distance along the view, by
/// `u_depth`'s projection terms (see [`scene_depth_terms`]), for the sprites
/// that fade against the scene's depth.
macro_rules! glsl_view_depth {
    () => {
        "float view_depth(float window) {
    float ndc = window * 2.0 - 1.0;
    return (ndc - u_depth.y) / (u_depth.x - ndc * u_depth.z);
}
"
    };
}

/// `u_depth` for [`glsl_view_depth`]: the projection's depth terms, then
/// the preview's scale, which brings its units back to the world's.
pub(in crate::app::model_preview) fn scene_depth_terms(camera: &ModelGpuCamera) -> [f32; 4] {
    [camera.depth_scale * camera.depth_map[0], camera.depth_map[1], camera.perspective, camera.scale]
}

const CE_FLARE_VERTEX: &str = "#version 330
layout(location = 0) in vec4 a_clip;
layout(location = 1) in vec2 a_uv;
out vec2 v_uv;
void main() {
    gl_Position = a_clip;
    v_uv = a_uv;
}
";

const CE_FLARE_FRAGMENT: &str = concat!("#version 330
in vec2 v_uv;
uniform sampler2D u_map;
uniform vec4 u_color;
uniform float u_highlight;
uniform vec2 u_tint;
uniform vec2 u_circular;
uniform float u_fade;
uniform vec4 u_depth;
uniform sampler2D u_scene_depth;
out vec4 out_color;
", glsl_view_depth!(), "void main() {
    vec4 map = texture(u_map, v_uv);
    if (u_circular.x > 0.0) {
        // Reach's light volume `circular` albedo: a disc, and no map.
        vec2 delta = 2.0 * v_uv - 1.0;
        float disc = pow(clamp(u_circular.x - u_circular.x * dot(delta, delta), 0.0, 1.0), u_circular.y);
        map = vec4(vec3(disc), 1.0);
    }
    if (u_fade > 0.0) {
        // Reach's light volume depth fade, as the particles'.
        float scene = texelFetch(u_scene_depth, ivec2(gl_FragCoord.xy), 0).r;
        float behind = (view_depth(scene) - view_depth(gl_FragCoord.z)) / u_depth.w;
        map.a *= clamp(behind / u_fade, 0.0, 1.0);
    }
    if (u_tint.y > 0.0) {
        vec4 lifted = pow(map, vec4(u_tint.y)) * map;
        out_color = vec4((lifted.rgb * u_tint.x + map.rgb * u_color.rgb) * u_color.a, 1.0);
    } else if (u_highlight < 0.0) {
        out_color = vec4(u_color.rgb * map.rgb, u_color.a * map.a);
    } else {
        // The pixel shader of `sub_92E4A0`, read from its bytecode.
        vec3 square = map.rgb * map.rgb;
        vec3 sixth = square * square * square;
        out_color = vec4((1.0 - u_color.rgb) * u_highlight * sixth + map.rgb * u_color.rgb, u_color.a);
    }
}
");

/// What every game's camera-facing quads draw with (flares, antennas, effect
/// sprites), made on first use.
pub(super) struct SpriteGl {
    pub(in crate::app::model_preview) program: glow::NativeProgram,
    pub(in crate::app::model_preview) vertex_array: glow::NativeVertexArray,
    pub(in crate::app::model_preview) buffer: glow::NativeBuffer,
    pub(in crate::app::model_preview) color: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) highlight: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) tint: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) map: Option<glow::NativeUniformLocation>,
    /// Reach's light volumes': the `circular` albedo and the depth fade.
    pub(in crate::app::model_preview) circular: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) fade: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) depth: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) scene_depth: Option<glow::NativeUniformLocation>,
    /// A patch's samples past the depth buffer, and in all.
    pub(in crate::app::model_preview) queries: [glow::NativeQuery; 2],
}

/// `effect_update` for Halo CE's attached effects, up to the frame's time.
/// Stepping to a time it has reached does nothing, so the model's lights
/// and the sprites can both ask for it.
pub(in crate::app::model_preview) fn advance_ce_effects(ce: &mut ce::slot::CeSlot, frame: &ModelGpuFrame, bones: Option<&[[f32; 4]]>) {
    let states = &frame.object_states;
    let inputs = |object: u8| {
        let state = states.get(object as usize);
        ce::particles::CeObjectInputs {
                active: state.map_or([false; 4], |state| state.active),
            outputs: state.map_or([0.0; 4], |state| state.outputs),
            change_colors: state.map_or([[1.0; 3]; 4], |state| state.change_colors),
        }
    };
    let lowest = frame.preview.vertices.iter().map(|vertex| vertex.position[2]);
    let ground = lowest.fold(0.0, f32::min);
    let others = ce.simulations.pctl.particle_count();
    let effects = &frame.preview.ce.effects;
    ce.simulations.particles.advance(effects, &frame.ce_plays, inputs, bones, ground, frame.time, others);
    let running = ce.simulations.particles.runs_played(effects.len());
    CE_EVENTS_RUNNING.with(|set| {
        let mut set = set.borrow_mut();
        if running {
            set.insert(frame.geometry_id);
        } else {
            set.remove(&frame.geometry_id);
        }
    });
}

/// A Halo CE sprite through the `effect` shader: its map, framebuffer blend
/// function and nonlinear tint, each corner's color, its corners, and the
/// shader's second map.
type CeSprite = (glow::NativeTexture, ce::passes::FramebufferBlend, bool, [[f32; 4]; 4], [[f32; 5]; 4], Option<ce::particles::CeSecondaryDraw>);

/// What one of Halo CE's transparent groups draws.
enum CeGroupDraw {
    /// Through the `effect` shader.
    Sprites(Vec<CeSprite>),
    /// Through the flares' plain color-times-map program: an antenna, or with
    /// `additive` a light volume, which `rasterizer_widget_begin(5, 1)` adds.
    Plain { quads: Vec<antennas::AntennaQuad>, texture: glow::NativeTexture, additive: bool },
}

/// One of Halo CE's transparent groups, which `rasterizer_transparent_geometry_draw`
/// sorts with the model's transparent parts by `z_sort`, far to near.
pub(in crate::app::model_preview) struct CeGroup {
    z_sort: f32,
    draw: CeGroupDraw,
}

/// How much nearer `sort bias` sorts a group (`_rasterizer_dynamic_unlit_geometry_draw`).
const CE_SORT_BIAS: f32 = 0.25;

/// `z_sort`: how far in front of the eye the point lies along the view,
/// negated, so the farthest sorts first; `sort bias` brings it nearer.
fn ce_z_sort(view: &common::view::ViewFrame, point: [f32; 3], sort_bias: bool) -> f32 {
    let along: f32 = (0..3).map(|axis| (point[axis] - view.eye[axis]) * view.forward[axis]).sum();
    if sort_bias { CE_SORT_BIAS - along } else { -along }
}

/// The mean of the quads' corners: `build_sprites_end`'s centroid, the mean
/// of its sprites' centers.
fn mean_corner<'a>(quads: impl Iterator<Item = &'a [[f32; 5]; 4]>) -> [f32; 3] {
    let (mut sum, mut count) = ([0.0f32; 3], 0.0f32);
    for corner in quads.flatten() {
        for axis in 0..3 {
            sum[axis] += corner[axis];
        }
        count += 1.0;
    }
    sum.map(|axis| axis / count.max(1.0))
}

/// The middle of the quads' bounds, as `lightning_submit` takes a bolt's.
fn bounds_middle<'a>(quads: impl Iterator<Item = &'a [[f32; 5]; 4]>) -> [f32; 3] {
    let (mut low, mut high) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for corner in quads.flatten() {
        for axis in 0..3 {
            low[axis] = low[axis].min(corner[axis]);
            high[axis] = high[axis].max(corner[axis]);
        }
    }
    std::array::from_fn(|axis| if low[axis] <= high[axis] { (low[axis] + high[axis]) * 0.5 } else { 0.0 })
}

/// Which source a sprite group comes from, in [`CeSpriteGroups`]' keys.
const CE_LIGHTNING: u8 = 0;
const CE_EFFECT_SYSTEM: u8 = 1;
const CE_SYSTEM: u8 = 2;
const CE_GLOW: u8 = 3;
const CE_CONTRAIL: u8 = 4;
const CE_PARTICLE: u8 = 5;

/// Sprites gathered into the groups the game submits them in, in the order
/// each group first appears.
#[derive(Default)]
struct CeSpriteGroups {
    /// Each group's `sort bias`, whether its centroid is the middle of its
    /// bounds rather than the mean, and its sprites.
    groups: Vec<(bool, bool, Vec<CeSprite>)>,
    by_key: HashMap<(u8, usize, usize), usize>,
}

impl CeSpriteGroups {
    fn push(&mut self, key: (u8, usize, usize), sort_bias: bool, by_bounds: bool, sprite: CeSprite) {
        let at = *self.by_key.entry(key).or_insert_with(|| {
            self.groups.push((sort_bias, by_bounds, Vec::new()));
            self.groups.len() - 1
        });
        self.groups[at].2.push(sprite);
    }

    fn sorted_by(self, view: &common::view::ViewFrame) -> impl Iterator<Item = CeGroup> {
        self.groups.into_iter().map(move |(sort_bias, by_bounds, sprites)| {
            let corners = sprites.iter().map(|sprite| &sprite.4);
            let centroid = if by_bounds { bounds_middle(corners) } else { mean_corner(corners) };
            CeGroup { z_sort: ce_z_sort(view, centroid, sort_bias), draw: CeGroupDraw::Sprites(sprites) }
        })
    }
}

/// Halo CE's effects, particle systems, glows, contrails, lightning, antennas
/// and light volumes brought up to the frame, as the groups the game submits
/// them in: a bolt, a particle state's draw, a glow, a contrail, the
/// particles of each definition, an antenna, a light volume. The slot's maps
/// are uploaded already.
pub(in crate::app::model_preview) fn ce_groups(
    gl: &glow::Context,
    ce: &mut ce::slot::CeSlot,
    frame: &ModelGpuFrame,
    bones: Option<&[[f32; 4]]>,
    view: &common::view::ViewFrame,
) -> Vec<CeGroup> {
    let camera = &frame.camera;
    let effects = &frame.preview.ce.effects;
    let states = &frame.object_states;
    let lowest = frame.preview.vertices.iter().map(|vertex| vertex.position[2]);
    let ground = lowest.fold(0.0, f32::min);
    let mut viewport = [0; 4];
    unsafe { gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport) };
    let half_width = viewport[2].max(1) as f32 * 0.5;
    // `render_frustum_sphere_diameter_in_pixels`.
    let pixels = |center: [f32; 3], radius: f32| {
        let middle = ce_flare_clip(camera, center);
        let side = ce_flare_clip(camera, std::array::from_fn(|axis| center[axis] + view.right[axis] * radius));
        ((side[0] / side[3] - middle[0] / middle[3]).abs() * half_width * 2.0).max(0.0)
    };
    let contrails = &frame.preview.ce.contrails;
    let switched = |object: u8| {
        states.get(object as usize).map_or(([0.0; 4], [false; 4]), |state| (state.outputs, state.active))
    };
    ce.simulations.contrails.advance(contrails, switched, bones, ground, frame.time);
    let change_colors = |object: u8| states.get(object as usize).map_or([[1.0; 3]; 4], |state| state.change_colors);
    let mut sprites = CeSpriteGroups::default();
    // Made afresh every frame, as the game draws it.
    let seed = (frame.time * 1000.0) as u32;
    for (index, lightning) in frame.preview.ce.lightning.iter().enumerate() {
        let Some(texture) = ce.textures.lightning.get(index).copied().flatten() else {
            continue;
        };
        let seed = seed.wrapping_add(index as u32 * 7919);
        let state = states.get(usize::from(lightning.object));
        let outputs = state.map_or([0.0; 4], |state| state.outputs);
        let colors = state.map_or([[1.0; 3]; 4], |state| state.change_colors);
        for quad in lightning.quads(&outputs, &colors, bones, view, seed) {
            let sprite = (texture, lightning.blend, lightning.nonlinear_tint, quad.colors, quad.corners, None);
            sprites.push((CE_LIGHTNING, index, 0), lightning.sort_bias, true, sprite);
        }
    }
    // `particle_systems_render`, for those effects' parts made.
    let pixel_of = |center: [f32; 3]| 2.0 / pixels(center, 1.0).max(0.0001);
    for (effect, system, quad) in ce.simulations.particles.system_quads(effects, view, pixel_of) {
        let texture = ce
            .textures.effect_pctl
            .get(&effect)
            .and_then(|systems| systems.get(system))
            .and_then(|sheets| sheets.get(quad.sheet))
            .and_then(|images| images.get(quad.image))
            .copied()
            .flatten();
        if let Some(texture) = texture {
            let sprite = (texture, quad.blend, quad.nonlinear_tint, [quad.color; 4], quad.corners, quad.secondary);
            sprites.push((CE_EFFECT_SYSTEM, quad.group, 0), quad.sort_bias, false, sprite);
        }
    }
    // And those on the object's attachments.
    let systems = &frame.preview.ce.particle_systems;
    if !systems.is_empty() {
        let inputs = |object: u8| {
            let state = states.get(usize::from(object));
            (
                state.map_or([0.0; 4], |state| state.outputs),
                state.map_or([false; 4], |state| state.active),
                state.map_or([[1.0; 3]; 4], |state| state.change_colors),
            )
        };
        let others = ce.simulations.particles.system_particle_count();
        ce.simulations.pctl.advance(systems, inputs, bones, ground, frame.time, others);
        let pixel = |center: [f32; 3]| 2.0 / pixels(center, 1.0).max(0.0001);
        for quad in ce.simulations.pctl.quads(systems, view, pixel) {
            let texture = ce.textures.pctl.get(quad.system).and_then(|sheets| sheets.get(quad.sheet)).and_then(|images| images.get(quad.image)).copied().flatten();
            if let Some(texture) = texture {
                let sprite = (texture, quad.blend, quad.nonlinear_tint, [quad.color; 4], quad.corners, quad.secondary);
                sprites.push((CE_SYSTEM, quad.group, 0), quad.sort_bias, false, sprite);
            }
        }
    }
    // `glow_render`, through `global_shader_effect_additive`.
    let glows = &frame.preview.ce.glows;
    // `glow_update` reads an output only while it is on, else nothing.
    let read_if_on = |object: u8| {
        let (outputs, active) = switched(object);
        std::array::from_fn(|slot| if active[slot] { outputs[slot] } else { 0.0 })
    };
    ce.simulations.glows.advance(glows, read_if_on, bones, frame.time);
    let pixel = |center: [f32; 3]| 2.0 / pixels(center, 1.0).max(0.0001);
    for (index, quad) in ce.simulations.glows.quads(glows, view, pixel) {
        if let Some(texture) = ce.textures.glows.get(index).copied().flatten() {
            sprites.push((CE_GLOW, index, 0), false, false, (texture, ce::passes::FramebufferBlend::Add, false, quad.colors, quad.corners, None));
        }
    }
    for quad in ce.simulations.contrails.quads(contrails, change_colors, view) {
        let Some(contrail) = contrails.get(quad.contrail) else {
            continue;
        };
        let texture = ce.textures.contrails.get(quad.contrail).and_then(|images| images.get(quad.image)).copied().flatten();
        if let Some(texture) = texture {
            let sprite = (texture, contrail.def.blend, contrail.def.nonlinear_tint, quad.colors, quad.corners, None);
            sprites.push((CE_CONTRAIL, quad.contrail, 0), contrail.def.sort_bias, false, sprite);
        }
    }
    for quad in ce.simulations.particles.quads(effects, bones, view, pixels) {
        let texture = ce
            .textures.particles
            .get(&quad.effect)
            .and_then(|defs| defs.get(quad.def))
            .and_then(|images| images.get(quad.image))
            .copied()
            .flatten();
        if let Some(texture) = texture {
            let sprite = (texture, quad.blend, quad.nonlinear_tint, [quad.color; 4], quad.corners, quad.secondary);
            sprites.push((CE_PARTICLE, quad.effect, quad.def), quad.sort_bias, false, sprite);
        }
    }
    let mut groups: Vec<CeGroup> = sprites.sorted_by(view).collect();
    // `antenna_render_proper`: one group each, at the mean of its sprites.
    for (antenna, texture) in frame.preview.ce.antennas.iter().zip(&ce.textures.antennas) {
        let Some(texture) = *texture else {
            continue;
        };
        let quads = antenna.quads(bones, view);
        let centroid = mean_corner(quads.iter().map(|quad| &quad.corners));
        let draw = CeGroupDraw::Plain { quads, texture, additive: false };
        groups.push(CeGroup { z_sort: ce_z_sort(view, centroid, false), draw });
    }
    // `light_volume_submit`: one group each, at its marker.
    for (volume, texture) in frame.preview.ce.light_volumes.iter().zip(&ce.textures.light_volumes) {
        let Some(texture) = *texture else {
            continue;
        };
        let (outputs, active) = switched(volume.object);
        let quads = volume.quads(&outputs, &active, bones, view);
        let draw = CeGroupDraw::Plain { quads, texture, additive: true };
        groups.push(CeGroup { z_sort: ce_z_sort(view, volume.marker(bones), false), draw });
    }
    groups
}

/// `rasterizer_transparent_geometry_group_draw` for one of Halo CE's groups,
/// tested against the model's depth and not written.
#[allow(clippy::too_many_arguments)]
pub(in crate::app::model_preview) unsafe fn draw_ce_group(
    gl: &glow::Context,
    effect_gl: &mut Option<Option<CeEffectGl>>,
    main: glow::NativeProgram,
    flares: &SpriteGl,
    ce: &mut ce::slot::CeSlot,
    camera: &ModelGpuCamera,
    group: CeGroup,
) {
    unsafe {
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LEQUAL);
        gl.depth_mask(false);
        gl.disable(glow::CULL_FACE);
        gl.color_mask(true, true, true, false);
        gl.enable(glow::BLEND);
        gl.blend_equation(glow::FUNC_ADD);
        gl.active_texture(glow::TEXTURE0);
        match group.draw {
            CeGroupDraw::Sprites(sprites) => {
                if let Some(effect_gl) = effect_gl.get_or_insert_with(|| ce_effect_gl(gl, main)).as_ref() {
                    draw_ce_sprites(gl, effect_gl, ce, camera, sprites);
                }
            }
            CeGroupDraw::Plain { quads, texture, additive } => {
                gl.use_program(Some(flares.program));
                gl.bind_vertex_array(Some(flares.vertex_array));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(flares.buffer));
                gl.uniform_1_i32(flares.map.as_ref(), 0);
                // Plain color times map.
                gl.uniform_1_f32(flares.highlight.as_ref(), -1.0);
                gl.uniform_2_f32(flares.tint.as_ref(), 0.0, 0.0);
                gl.blend_func(glow::SRC_ALPHA, if additive { glow::ONE } else { glow::ONE_MINUS_SRC_ALPHA });
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                for quad in quads {
                    let vertices = quad.corners.map(|[x, y, z, u, v]| {
                        let [a, b, c, d] = ce_flare_clip(camera, [x, y, z]);
                        [a, b, c, d, u, v]
                    });
                    gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(&vertices), glow::STREAM_DRAW);
                    let [red, green, blue, alpha] = quad.color;
                    gl.uniform_4_f32(flares.color.as_ref(), red, green, blue, alpha);
                    gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                }
            }
        }
    }
}

/// `render_particles`' draw: each sprite through the `effect` shader.
unsafe fn draw_ce_sprites(
    gl: &glow::Context,
    effect_gl: &CeEffectGl,
    ce: &mut ce::slot::CeSlot,
    camera: &ModelGpuCamera,
    sprites: Vec<CeSprite>,
) {
    unsafe {
        gl.use_program(Some(effect_gl.program));
        gl.bind_vertex_array(Some(effect_gl.vertex_array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(effect_gl.buffer));
        gl.active_texture(glow::TEXTURE0);
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LEQUAL);
        gl.depth_mask(false);
        gl.enable(glow::BLEND);
        // `C_ZSPRITE_PARAMS`' radius moves a pixel toward the eye; these are
        // how far clip z and w go for each unit of that.
        let zsprite_z = camera.scale * camera.depth_scale * camera.depth_map[0];
        let zsprite_w = camera.scale * camera.perspective;
        for (texture, function, nonlinear, colors, corners, secondary) in sprites {
            // A second map that would not upload draws as none, not as black.
            let secondary = secondary.as_ref().and_then(|draw| {
                let key = Arc::as_ptr(&draw.map) as usize;
                if !ce.textures.secondary.contains_key(&key)
                    && let Some(made) = upload_h2_image(gl, &draw.map.image)
                {
                    ce.textures.secondary.insert(key, made);
                }
                ce.textures.secondary.get(&key).map(|map| (draw, *map))
            });
            match secondary {
                Some((draw, map)) => {
                    gl.active_texture(glow::TEXTURE1);
                    gl.bind_texture(glow::TEXTURE_2D, Some(map));
                    gl.active_texture(glow::TEXTURE0);
                    gl.uniform_1_i32(effect_gl.mode.as_ref(), draw.mode);
                    gl.uniform_4_f32_slice(effect_gl.rows.as_ref(), draw.rows.as_flattened());
                    gl.uniform_3_f32(effect_gl.zsprite.as_ref(), draw.radius, zsprite_z, zsprite_w);
                }
                None => gl.uniform_1_i32(effect_gl.mode.as_ref(), 0),
            }
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            let blend = function.gl();
            gl.blend_equation(gl_blend_equation(blend.equation));
            gl.blend_func(gl_blend_factor(blend.source), gl_blend_factor(blend.destination));
            gl.uniform_1_i32(effect_gl.blend.as_ref(), function.shader_number());
            gl.uniform_1_i32(effect_gl.nonlinear.as_ref(), i32::from(nonlinear));
            // The zsprite stream's corners, in `build_sprite`'s vertex order
            // (left bottom, right bottom, right top, left top): 00, 01, 11, 10.
            let units = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
            let mut vertices = [[0.0f32; CE_EFFECT_FLOATS]; 4];
            for ((vertex, ([x, y, z, u, v], unit)), [red, green, blue, alpha]) in
                vertices.iter_mut().zip(corners.into_iter().zip(units)).zip(colors)
            {
                let [a, b, c, d] = ce_flare_clip(camera, [x, y, z]);
                *vertex = [a, b, c, d, u, v, unit[0], unit[1], red, green, blue, alpha];
            }
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(&vertices), glow::STREAM_DRAW);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }
}

/// `fx/eff_common.h`: what Halo CE's particles, contrails and light volumes
/// draw with. The fog it folds into the vertex alpha is 1 with no level.
/// `effect.vsh`, `effect_multitexture.vsh`, `effect_multitexture_screenspace.vsh`
/// and `effect_zsprite.vsh` as far as a sprite needs: its place, its map's
/// coordinates, and the corner of the sprite the zsprite stream gives it.
const CE_EFFECT_VERTEX: &str = "#version 330
layout(location = 0) in vec4 a_clip;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec2 a_unit;
layout(location = 3) in vec4 a_color;
out vec2 v_uv;
out vec2 v_unit;
out vec4 v_clip;
out vec4 v_color;
void main() {
    gl_Position = a_clip;
    v_uv = a_uv;
    v_unit = a_unit;
    v_clip = a_clip;
    v_color = a_color;
}
";

const CE_EFFECT_FRAGMENT: &str = "#version 330
in vec2 v_uv;
in vec2 v_unit;
in vec4 v_clip;
// The vertex's color, as the game's effect vertices carry it.
in vec4 v_color;
uniform sampler2D u_map;
uniform sampler2D u_second;
uniform int u_blend;
uniform int u_nonlinear;
// 0 none, 1 a second color map over the sprite, 2 over the screen, 3 zsprite.
uniform int u_mode;
uniform vec4 u_rows[2];
// The zsprite's radius, and how clip z and w move per unit toward the eye.
uniform vec3 u_zsprite;
out vec4 out_color;
void main() {
    vec4 t0 = texture(u_map, v_uv);
    vec4 r0 = vec4(u_nonlinear != 0 ? mix(pow(t0.rgb, vec3(4.0)), t0.rgb, v_color.rgb) : t0.rgb * v_color.rgb, t0.a);
    if (u_mode == 1 || u_mode == 2) {
        vec4 at = u_mode == 1 ? vec4(v_uv, 0.0, 1.0) : vec4(v_clip.xy / v_clip.w, 1.0, 1.0);
        vec4 t1 = texture(u_second, vec2(dot(at, u_rows[0]), dot(at, u_rows[1])));
        r0 *= t1;
    }
    float depth = gl_FragCoord.z;
    if (u_mode == 3) {
        // The map's green says how far toward the eye the pixel stands, never
        // through the near plane.
        float toward = u_zsprite.x * texture(u_second, v_unit).g;
        if (u_zsprite.z > 0.0) {
            toward = min(toward, max((v_clip.w - 0.001) / u_zsprite.z, 0.0));
        }
        float z = v_clip.z - toward * u_zsprite.y;
        float w = v_clip.w - toward * u_zsprite.z;
        depth = (z / w) * 0.5 + 0.5;
    }
    gl_FragDepth = depth;
    float a = v_color.a;
    // `sub_85F3B0` draws min with the multiply shader, and subtract and max
    // with the add one.
    if (u_blend == 0) {
        r0.a *= a;
    } else if (u_blend == 1 || u_blend == 5) {
        r0.rgb = mix(vec3(1.0), r0.rgb, a);
    } else if (u_blend == 2) {
        r0.rgb = mix(vec3(0.5), r0.rgb, a);
    } else if (u_blend == 3 || u_blend == 4 || u_blend == 6) {
        r0.rgb *= a;
    } else if (u_blend == 7) {
        r0 *= a;
    }
    out_color = r0;
}
";

pub(in crate::app::model_preview) struct CeEffectGl {
    program: glow::NativeProgram,
    vertex_array: glow::NativeVertexArray,
    buffer: glow::NativeBuffer,
    blend: Option<glow::NativeUniformLocation>,
    nonlinear: Option<glow::NativeUniformLocation>,
    mode: Option<glow::NativeUniformLocation>,
    rows: Option<glow::NativeUniformLocation>,
    zsprite: Option<glow::NativeUniformLocation>,
}

/// A CE effect sprite's vertex: clip position, map coordinates, the
/// sprite's corner for a zsprite's map, and color.
const CE_EFFECT_FLOATS: usize = 12;
const CE_EFFECT_STRIDE: i32 = CE_EFFECT_FLOATS as i32 * 4;

unsafe fn ce_effect_gl(gl: &glow::Context, main: glow::NativeProgram) -> Option<CeEffectGl> {
    unsafe {
        let program = link_h2_program(gl, main, CE_EFFECT_VERTEX, CE_EFFECT_FRAGMENT)
            .map_err(|error| eprintln!("model preview: {error}"))
            .ok()?;
        let vertex_array = gl.create_vertex_array().ok()?;
        let buffer = gl.create_buffer().ok()?;
        gl.bind_vertex_array(Some(vertex_array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 4, glow::FLOAT, false, CE_EFFECT_STRIDE, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, CE_EFFECT_STRIDE, 16);
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, CE_EFFECT_STRIDE, 24);
        gl.enable_vertex_attrib_array(3);
        gl.vertex_attrib_pointer_f32(3, 4, glow::FLOAT, false, CE_EFFECT_STRIDE, 32);
        gl.use_program(Some(program));
        gl.uniform_1_i32(gl.get_uniform_location(program, "u_map").as_ref(), 0);
        gl.uniform_1_i32(gl.get_uniform_location(program, "u_second").as_ref(), 1);
        Some(CeEffectGl {
            program,
            vertex_array,
            buffer,
            blend: gl.get_uniform_location(program, "u_blend"),
            nonlinear: gl.get_uniform_location(program, "u_nonlinear"),
            mode: gl.get_uniform_location(program, "u_mode"),
            rows: gl.get_uniform_location(program, "u_rows"),
            zsprite: gl.get_uniform_location(program, "u_zsprite"),
        })
    }
}

const PARTICLE_VERTEX: &str = "#version 330 core
layout(location = 0) in vec4 a_clip;
layout(location = 1) in vec2 a_sprite;
layout(location = 2) in vec2 a_next_sprite;
layout(location = 3) in vec2 a_billboard;
layout(location = 4) in vec4 a_color;
layout(location = 5) in vec3 a_extra;
layout(location = 6) in vec4 a_lean;
layout(location = 7) in vec3 a_normal;
out vec4 v_lean;
out vec3 v_normal;
out vec2 v_sprite;
out vec2 v_next_sprite;
out vec2 v_billboard;
out vec4 v_color;
out vec3 v_extra;
void main() {
    gl_Position = a_clip;
    v_sprite = a_sprite;
    v_next_sprite = a_next_sprite;
    v_billboard = a_billboard;
    v_color = a_color;
    v_extra = a_extra;
    v_lean = a_lean;
    v_normal = a_normal;
}
";

const PARTICLE_FRAGMENT: &str = concat!("#version 330 core
in vec2 v_sprite;
in vec2 v_next_sprite;
in vec2 v_billboard;
in vec4 v_color;
in vec3 v_extra;
in vec4 v_lean;
in vec3 v_normal;
uniform bool u_lit;
uniform bool u_alpha_test;
uniform bool u_premultiplied;
uniform vec4 u_lighting[10];
uniform float u_distortion;
uniform bool u_unsigned;
uniform sampler2D u_base;
uniform sampler2D u_alpha;
uniform sampler2D u_palette;
uniform sampler2D u_base2;
uniform vec4 u_base_xform;
uniform vec4 u_base2_xform;
uniform vec3 u_tint;
uniform vec2 u_modulation;
uniform int u_albedo;
uniform bool u_black_point;
uniform bool u_multiply;
uniform vec3 u_add;
uniform float u_fade;
uniform vec4 u_depth;
uniform sampler2D u_scene_depth;
uniform vec4 u_screen;
uniform bool u_smooth;
uniform vec3 u_reach_light[3];
uniform vec2 u_contrast;
uniform float u_sphere_warp;
uniform float u_palette_shift;
uniform bool u_plasma_second_sprite;
uniform bool u_unit_particle_alpha;
out vec4 out_color;
", glsl_view_depth!(), "// `other` is the second sprite texcoord, which only Reach's plasmas read.
vec4 albedo(vec2 sprite, vec2 other, float particle_alpha, float depth_fade) {
    float external_alpha = particle_alpha * depth_fade;
    if (u_albedo == 6) {
        vec4 diffuse = texture(u_base, sprite);
        float brightness = dot(diffuse.rgb, diffuse.rgb) / sqrt(3.0);
        diffuse.rgb *= mix(u_tint, vec3(1.0), u_modulation.x * brightness);
        return diffuse;
    }
    if (u_albedo == 7) {
        float ramp = texture(u_base, sprite).g;
        float glow = v_extra.y * ramp;
        return vec4(clamp(u_tint * glow * pow(2.0, glow * 6.0), 0.0, 1.0), ramp);
    }
    if (u_albedo == 8 || u_albedo == 12) {
        vec2 second = u_plasma_second_sprite ? other : sprite;
        float noise_a = texture(u_base, sprite * u_base_xform.xy + u_base_xform.zw).r;
        float noise_b = texture(u_base2, second * u_base2_xform.xy + u_base2_xform.zw).r;
        float alpha = texture(u_alpha, v_billboard).a;
        if (u_albedo == 12) {
            // Reach's `palettized_2d_plasma`: no alpha term but the palette
            // shift's, and the depth fade picks the palette's row.
            float index = clamp(abs(noise_a - noise_b), 0.0, 1.0);
            index = clamp(index + (1.0 - alpha * particle_alpha) * u_palette_shift, 0.0, 1.0);
            return vec4(texture(u_palette, vec2(index, depth_fade)).rgb, alpha);
        }
        float index = clamp(abs(noise_a - noise_b) + (1.0 - alpha * external_alpha) * u_modulation.y, 0.0, 1.0);
        return vec4(texture(u_palette, vec2(index, v_extra.y)).rgb, alpha);
    }
    vec4 base = texture(u_base, sprite);
    vec4 tone = base;
    float row = v_extra.y;
    // The palette options; Halo 2's tints read the base map alone.
    if (u_albedo >= 3 && u_albedo <= 5) {
        // Reach's `palette_shift` moves the index by what has faded.
        float shifted = clamp(base.r + (1.0 - depth_fade * particle_alpha) * u_palette_shift, 0.0, 1.0);
        float index = u_palette_shift == 0.0 ? base.r : shifted;
        tone = texture(u_palette, vec2(index, row));
    }
    if (u_albedo == 1 || u_albedo == 4) {
        tone.a = texture(u_alpha, v_billboard).a;
    } else if (u_albedo == 2 || u_albedo == 5) {
        tone.a = texture(u_alpha, sprite).a;
    }
    return tone;
}
void main() {
    if (u_albedo == 11) {
        // `effect_shader_alphablend_distortion_0.psh` (U is the decode's green,
        // V its red). MCC makes `v8u8` an `R8G8_SNORM` texture (`dword_EBC5B8`),
        // so the shader's unsigned-to-signed `bump - round(bump)` runs on bytes
        // already signed, and folds the strong ones back towards nothing.
        vec2 signed_byte = round(texture(u_base, v_sprite).gr * 127.0);
        vec2 snorm = max(signed_byte / 127.0, -1.0);
        vec2 bump = snorm - round(snorm);
        vec2 screen = (gl_FragCoord.xy - u_screen.xy) / u_screen.zw;
        // Its screen v runs down the frame, this one's up.
        vec2 read = screen + u_modulation.x / 64.0 * vec2(bump.x, -bump.y);
        if (v_color.a <= 0.0) {
            discard;
        }
        out_color = vec4(texture(u_palette, read).rgb * v_color.rgb, v_color.a);
        return;
    }
    float depth_fade = 1.0;
    if (u_fade > 0.0) {
        float scene = texelFetch(u_scene_depth, ivec2(gl_FragCoord.xy), 0).r;
        float behind = (view_depth(scene) - view_depth(gl_FragCoord.z)) / u_depth.w;
        depth_fade = clamp(behind / u_fade, 0.0, 1.0);
    }
    // Reach's `calc_warp` `sphere`: both sprites pushed out toward the rim.
    vec2 rim = v_billboard * 2.0 - 1.0;
    vec2 warp = u_sphere_warp == 0.0 ? vec2(0.0) : rim * dot(rim, rim) * dot(rim, rim) * u_sphere_warp;
    vec2 sprite = v_sprite + warp;
    vec2 next_sprite = v_next_sprite + warp;
    // Reach's beams sample with a `particle_alpha` of 1.
    float particle_alpha = u_unit_particle_alpha ? 1.0 : v_color.a;
    vec4 color = mix(
        albedo(sprite, next_sprite, particle_alpha, depth_fade),
        albedo(next_sprite, sprite, particle_alpha, depth_fade),
        v_extra.x
    );
    if (u_distortion != 0.0) {
        // `compute_normalized_distortion`, in the frame's own widths and heights.
        vec2 map = u_unsigned ? (color.xy * 255.0 - 128.0) / 127.0 : color.xy;
        vec2 push = vec2(map.x, -map.y) * v_color.a * depth_fade;
        out_color = vec4(vec2(dot(v_lean.xy, push), dot(v_lean.zw, push)) * u_distortion / 32.0, 0.0, 1.0);
        return;
    }
    color.a *= depth_fade;
    if (u_black_point) {
        float black = v_extra.z;
        float middle = (black + 1.0) * 0.5;
        color.a = middle * clamp((color.a - black) / (middle - black), 0.0, 1.0)
            + clamp(color.a - middle, 0.0, 1.0);
    }
    if (u_alpha_test) {
        if (color.a < 0.5) {
            discard;
        }
        color.a = 1.0;
    }
    if (u_albedo == 9) {
        vec3 fourth = color.rgb * color.rgb;
        fourth *= fourth;
        vec3 toward = mix(fourth, color.rgb, v_color.rgb);
        color.rgb = mix(toward, color.rgb * v_color.rgb, u_modulation.x);
        color.a *= v_color.a;
    } else if (u_albedo == 10) {
        color.rgb *= v_color.rgb * v_color.a;
        color.a *= v_color.a;
    } else {
        color *= v_color;
    }
    if (u_lit) {
        vec3 ambient = 0.886227 * u_lighting[0].rgb;
        vec3 normal = normalize(v_normal);
        vec3 a = normal * normal.yzx;
        vec4 b = vec4(normal * normal, 1.0 / 3.0);
        vec3 first = vec3(dot(normal, u_lighting[1].rgb), dot(normal, u_lighting[2].rgb), dot(normal, u_lighting[3].rgb));
        vec3 second = vec3(dot(a, u_lighting[4].rgb), dot(a, u_lighting[5].rgb), dot(a, u_lighting[6].rgb));
        vec3 third = vec3(dot(b, u_lighting[7]), dot(b, u_lighting[8]), dot(b, u_lighting[9]));
        vec3 full = ambient - 2.0 * 0.511664 * first - 2.0 * 0.429043 * second - 0.429043 * third;
        color.rgb *= mix(ambient, full, color.a) / 3.1415926535;
    }
    if (u_smooth) {
        // Reach's `per_pixel_smooth`: from the probe's dark to its bright.
        float cosine = dot(normalize(v_normal), u_reach_light[0]);
        float toward = pow(clamp(cosine * u_contrast.x + u_contrast.y, 0.0, 1.0), 3.0);
        color.rgb *= mix(u_reach_light[2], u_reach_light[1], toward);
    }
    if (u_multiply) {
        color.rgb = mix(vec3(1.0), color.rgb, color.a);
    } else {
        color.rgb += u_add;
    }
    // `convert_to_render_target_premultiplied_alpha`.
    if (u_premultiplied && u_albedo != 10) {
        color.rgb *= color.a;
    }
    out_color = color;
}
");

const PARTICLE_FLOATS: usize = 24;

/// A run of vertices as the particle program takes them.
/// A gen3 sprite shader's base, alpha, palette and second maps, each with
/// its own sampler.
pub(in crate::app::model_preview) unsafe fn sprite_maps(
    gl: &glow::Context,
    look: &particles::H3SpriteShader,
) -> [Option<glow::NativeTexture>; 4] {
    let [base, alpha, palette, second] = look.samplers;
    [(&look.base_map, base), (&look.alpha_map, alpha), (&look.palette_map, palette), (&look.second_map, second)].map(
        |(map, sampler)| {
            map.as_ref().and_then(|(image, texels, mips)| unsafe {
                gen3::pipeline::upload_h2_image_linear(gl, image, *texels, mips, sampler)
            })
        },
    )
}

pub(in crate::app::model_preview) fn particle_floats(camera: &ModelGpuCamera, drawn: &[particles::H3ParticleVertex]) -> Vec<[f32; PARTICLE_FLOATS]> {
    drawn
        .iter()
        .map(|vertex| {
            let [a, b, c, d] = ce_flare_clip(camera, vertex.place);
            let ([e, f], [g, h], [i, j]) = (vertex.sprite, vertex.next_sprite, vertex.billboard);
            let [k, l, m, n] = vertex.color;
            let [o, p, q] = vertex.extra;
            let lean = vertex.lean.map(|side| {
                let reach: [f32; 3] = std::array::from_fn(|axis| vertex.place[axis] + side[axis]);
                let [x, y, _, w] = ce_flare_clip(camera, reach);
                [(x / w - a / d) * 0.5, (y / w - b / d) * 0.5]
            });
            let [[r, s], [t, u]] = lean;
            let [v, w, x] = vertex.normal;
            [a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r, s, t, u, v, w, x]
        })
        .collect()
}

/// What Halo 3's particles draw with, made on first use.
pub(in crate::app::model_preview) struct H3ParticleGl {
    program: glow::NativeProgram,
    vertex_array: glow::NativeVertexArray,
    buffer: glow::NativeBuffer,
    albedo: Option<glow::NativeUniformLocation>,
    black_point: Option<glow::NativeUniformLocation>,
    multiply: Option<glow::NativeUniformLocation>,
    add: Option<glow::NativeUniformLocation>,
    lit: Option<glow::NativeUniformLocation>,
    premultiplied: Option<glow::NativeUniformLocation>,
    alpha_test: Option<glow::NativeUniformLocation>,
    tint: Option<glow::NativeUniformLocation>,
    modulation: Option<glow::NativeUniformLocation>,
    base_xform: Option<glow::NativeUniformLocation>,
    second_xform: Option<glow::NativeUniformLocation>,
    lighting: Option<glow::NativeUniformLocation>,
    distortion: Option<glow::NativeUniformLocation>,
    unsigned: Option<glow::NativeUniformLocation>,
    fade: Option<glow::NativeUniformLocation>,
    depth: Option<glow::NativeUniformLocation>,
    screen: Option<glow::NativeUniformLocation>,
    smooth: Option<glow::NativeUniformLocation>,
    reach_light: Option<glow::NativeUniformLocation>,
    contrast: Option<glow::NativeUniformLocation>,
    sphere_warp: Option<glow::NativeUniformLocation>,
    palette_shift: Option<glow::NativeUniformLocation>,
    plasma_second_sprite: Option<glow::NativeUniformLocation>,
    unit_particle_alpha: Option<glow::NativeUniformLocation>,
}

impl H3ParticleGl {
    pub(in crate::app::model_preview) unsafe fn draw(
        &self,
        gl: &glow::Context,
        look: &particles::H3SpriteShader,
        vertices: &[[f32; PARTICLE_FLOATS]],
        textures: [Option<glow::NativeTexture>; 4],
        no_map: Option<glow::NativeTexture>,
        depth: Option<glow::NativeTexture>,
        camera: &ModelGpuCamera,
        lighting: &[[f32; 4]; 10],
        stops: f32,
    ) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(vertices), glow::STREAM_DRAW);
            for (unit, texture) in [0, 1, 2, 4].into_iter().zip(textures) {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, texture.or(no_map));
            }
            gl.active_texture(glow::TEXTURE3);
            // Halo 2 has no depth copy, and no fade to read one; an empty unit
            // still makes Apple's driver complain.
            gl.bind_texture(glow::TEXTURE_2D, depth.or(no_map));
            gl.active_texture(glow::TEXTURE0);
            let range = if depth.is_some() { look.depth_fade.unwrap_or(0.0) } else { 0.0 };
            gl.uniform_1_f32(self.fade.as_ref(), range);
            gl.uniform_4_f32_slice(self.depth.as_ref(), &scene_depth_terms(camera));
            gl.uniform_1_i32(self.albedo.as_ref(), look.albedo as i32);
            if look.albedo == particles::H3ParticleAlbedo::H2Distortion {
                let mut viewport = [0; 4];
                gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
                let [x, y, width, height] = viewport.map(|value| value as f32);
                gl.uniform_4_f32(self.screen.as_ref(), x, y, width.max(1.0), height.max(1.0));
            }
            let constants = look.constants;
            gl.uniform_3_f32_slice(self.tint.as_ref(), &constants.tint);
            gl.uniform_2_f32(self.modulation.as_ref(), constants.modulation, constants.alpha_modulation);
            gl.uniform_4_f32_slice(self.base_xform.as_ref(), &constants.base_xform);
            gl.uniform_4_f32_slice(self.second_xform.as_ref(), &constants.second_xform);
            gl.uniform_1_i32(self.black_point.as_ref(), i32::from(look.black_point));
            let multiply = look.blend == H3Blend::Multiply;
            gl.uniform_1_i32(self.multiply.as_ref(), i32::from(multiply));
            gl.uniform_3_f32_slice(self.add.as_ref(), &look.add(stops));
            gl.uniform_1_f32(self.distortion.as_ref(), look.distortion.unwrap_or(0.0));
            gl.uniform_1_i32(self.lit.as_ref(), i32::from(look.lit_pixels));
            let contrast = look.smooth_contrast();
            gl.uniform_1_i32(self.smooth.as_ref(), i32::from(contrast.is_some()));
            if let Some([scale, offset]) = contrast {
                let light = reach_particle_light();
                let rows = [light.direction, light.bright, light.dark];
                gl.uniform_3_f32_slice(self.reach_light.as_ref(), rows.as_flattened());
                gl.uniform_2_f32(self.contrast.as_ref(), scale, offset);
            }
            gl.uniform_1_f32(self.sphere_warp.as_ref(), look.sphere_warp.unwrap_or(0.0));
            gl.uniform_1_f32(self.palette_shift.as_ref(), look.palette_shift.unwrap_or(0.0));
            gl.uniform_1_i32(self.plasma_second_sprite.as_ref(), i32::from(look.plasma_second_sprite));
            gl.uniform_1_i32(self.unit_particle_alpha.as_ref(), i32::from(look.unit_particle_alpha));
            let premultiplied = look.blend == H3Blend::PreMultiplied;
            gl.uniform_1_i32(self.premultiplied.as_ref(), i32::from(premultiplied));
            gl.uniform_1_i32(self.alpha_test.as_ref(), i32::from(look.alpha_test));
            gl.depth_mask(look.alpha_test);
            gl.uniform_4_f32_slice(self.lighting.as_ref(), lighting.as_flattened());
            let unsigned = look.base_map.as_ref().is_some_and(|(_, texels, _)| *texels != gen3::shaders::H3Texels::Signed);
            gl.uniform_1_i32(self.unsigned.as_ref(), i32::from(unsigned));
            if look.distortion.is_some() {
                gl.blend_equation(glow::FUNC_ADD);
                gl.blend_func(glow::ONE, glow::ONE);
            } else {
                gen3::pipeline::h3_set_blend(gl, look.blend);
            }
            // What covers the background has to say so, as an antenna does.
            let covers = matches!(look.blend, H3Blend::AlphaBlend | H3Blend::PreMultiplied);
            gl.color_mask(true, true, true, covers);
            gl.draw_arrays(glow::TRIANGLES, 0, vertices.len() as i32);
            gl.depth_mask(false);
            gl.color_mask(true, true, true, false);
        }
    }
}

/// The viewport of the framebuffer being drawn, copied into a texture of its
/// size. A blit, as what is drawn to may be multisampled.
pub(in crate::app::model_preview) unsafe fn copy_frame(gl: &glow::Context, copy: &mut Option<ColorTarget>) -> Option<glow::NativeTexture> {
    unsafe {
        let mut viewport = [0; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
        let [x, y, width, height] = viewport;
        if width <= 0 || height <= 0 {
            return None;
        }
        if copy.as_ref().is_some_and(|copy| copy.size != [width, height])
            && let Some(old) = copy.take()
        {
            old.delete(gl);
        }
        let drawn_to = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
        if copy.is_none() {
            *copy = Some(ColorTarget::new(gl, [width, height], Texels::RGBA8, Sampling::LINEAR_CLAMPED)?);
        }
        let made = copy.as_ref()?;
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, drawn_to);
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(made.framebuffer));
        gl.blit_framebuffer(x, y, x + width, y + height, 0, 0, width, height, glow::COLOR_BUFFER_BIT, glow::NEAREST);
        gl.bind_framebuffer(glow::FRAMEBUFFER, drawn_to);
        Some(made.texture)
    }
}

/// A cheap particle vertex: clip position, texcoord and layer, colour.
pub(in crate::app::model_preview) const CHEAP_FLOATS: usize = 11;

/// Reach's cheap particle program and its stream.
pub(in crate::app::model_preview) struct ReachCheapGl {
    program: glow::NativeProgram,
    vertex_array: glow::NativeVertexArray,
    buffer: glow::NativeBuffer,
    exposure: Option<glow::NativeUniformLocation>,
    self_illum: Option<glow::NativeUniformLocation>,
}

impl ReachCheapGl {
    /// `render_particles` (the 360's 0x82DB3B58): `set_alpha_blend_mode(5)`,
    /// ONE and INVSRCALPHA, tested and not written against depth, at
    /// `g_exposure.x` for the area screen effects' `stops`. The game writes
    /// colour alone (`set_color_write_enable(0, 7)`); here alpha is what
    /// covers the preview's background, so it is written as the other
    /// premultiplied sprites write it.
    /// `exposures` are the exposure and the self-illumination exposure the
    /// colour runs between by alpha: Halo 4's two, or Reach's one twice.
    pub(in crate::app::model_preview) unsafe fn draw(&self, gl: &glow::Context, vertices: &[[f32; CHEAP_FLOATS]], texture: glow::NativeTexture, [exposure, self_illum]: [f32; 2]) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(vertices), glow::STREAM_DRAW);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
            gl.uniform_1_f32(self.exposure.as_ref(), exposure);
            gl.uniform_1_f32(self.self_illum.as_ref(), self_illum);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            gen3::pipeline::h3_set_blend(gl, H3Blend::PreMultiplied);
            gl.draw_arrays(glow::TRIANGLES, 0, vertices.len() as i32);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
            gl.color_mask(true, true, true, false);
        }
    }
}

pub(in crate::app::model_preview) unsafe fn reach_cheap_gl(gl: &glow::Context, main: glow::NativeProgram) -> Option<ReachCheapGl> {
    use particles::cheap::{CHEAP_PARTICLE_FRAGMENT, CHEAP_PARTICLE_VERTEX};
    unsafe {
        let program = link_h2_program(gl, main, CHEAP_PARTICLE_VERTEX, CHEAP_PARTICLE_FRAGMENT)
            .map_err(|error| eprintln!("model preview: {error}"))
            .ok()?;
        let vertex_array = gl.create_vertex_array().ok()?;
        let buffer = gl.create_buffer().ok()?;
        gl.bind_vertex_array(Some(vertex_array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        let stride = (CHEAP_FLOATS * size_of::<f32>()) as i32;
        let mut offset = 0;
        for (attribute, size) in [4, 3, 4].into_iter().enumerate() {
            gl.enable_vertex_attrib_array(attribute as u32);
            gl.vertex_attrib_pointer_f32(attribute as u32, size, glow::FLOAT, false, stride, offset);
            offset += size * size_of::<f32>() as i32;
        }
        gl.use_program(Some(program));
        gl.uniform_1_i32(gl.get_uniform_location(program, "u_render").as_ref(), 0);
        Some(ReachCheapGl { program, vertex_array, buffer, exposure: gl.get_uniform_location(program, "u_exposure"), self_illum: gl.get_uniform_location(program, "u_self_illum") })
    }
}

/// The render texture array, every level: sampler 0 at address mode 1,
/// clamped (`shader_setup_type_library`, the 360's 0x82DB2B50) [inferred: by
/// the tag enum's order, as Baboon reads sampler modes elsewhere], linear
/// within a layer and between levels, and GL never filters across layers.
unsafe fn upload_cheap_render_texture(
    gl: &glow::Context,
    render: &particles::cheap::CheapRenderTexture,
) -> Option<glow::NativeTexture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
        for (level, rgba) in render.levels.iter().enumerate() {
            let (width, height) = ((render.width >> level).max(1), (render.height >> level).max(1));
            if rgba.len() != width * height * render.layers * 4 {
                gl.delete_texture(texture);
                return None;
            }
            let data = glow::PixelUnpackData::Slice(Some(rgba));
            let [width, height, layers] = [width, height, render.layers].map(|size| size as i32);
            gl.tex_image_3d(glow::TEXTURE_2D_ARRAY, level as i32, glow::RGBA8 as i32, width, height, layers, 0, glow::RGBA, glow::UNSIGNED_BYTE, data);
        }
        let last = render.levels.len().saturating_sub(1) as i32;
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32),
            (glow::TEXTURE_MAX_LEVEL, last),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D_ARRAY, name, value);
        }
        gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
        Some(texture)
    }
}

pub(in crate::app::model_preview) unsafe fn h3_particle_gl(gl: &glow::Context, main: glow::NativeProgram) -> Option<H3ParticleGl> {
    unsafe {
        let program = link_h2_program(gl, main, PARTICLE_VERTEX, PARTICLE_FRAGMENT)
            .map_err(|error| eprintln!("model preview: {error}"))
            .ok()?;
        let vertex_array = gl.create_vertex_array().ok()?;
        let buffer = gl.create_buffer().ok()?;
        gl.bind_vertex_array(Some(vertex_array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        let stride = (PARTICLE_FLOATS * size_of::<f32>()) as i32;
        let mut offset = 0;
        for (attribute, size) in [4, 2, 2, 2, 4, 3, 4, 3].into_iter().enumerate() {
            gl.enable_vertex_attrib_array(attribute as u32);
            gl.vertex_attrib_pointer_f32(attribute as u32, size, glow::FLOAT, false, stride, offset);
            offset += size * size_of::<f32>() as i32;
        }
        gl.use_program(Some(program));
        for (unit, name) in ["u_base", "u_alpha", "u_palette", "u_scene_depth", "u_base2"].into_iter().enumerate() {
            gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), unit as i32);
        }
        Some(H3ParticleGl {
            program,
            vertex_array,
            buffer,
            albedo: gl.get_uniform_location(program, "u_albedo"),
            black_point: gl.get_uniform_location(program, "u_black_point"),
            multiply: gl.get_uniform_location(program, "u_multiply"),
            add: gl.get_uniform_location(program, "u_add"),
            lit: gl.get_uniform_location(program, "u_lit"),
            premultiplied: gl.get_uniform_location(program, "u_premultiplied"),
            alpha_test: gl.get_uniform_location(program, "u_alpha_test"),
            tint: gl.get_uniform_location(program, "u_tint"),
            modulation: gl.get_uniform_location(program, "u_modulation"),
            base_xform: gl.get_uniform_location(program, "u_base_xform"),
            second_xform: gl.get_uniform_location(program, "u_base2_xform"),
            lighting: gl.get_uniform_location(program, "u_lighting"),
            distortion: gl.get_uniform_location(program, "u_distortion"),
            unsigned: gl.get_uniform_location(program, "u_unsigned"),
            fade: gl.get_uniform_location(program, "u_fade"),
            depth: gl.get_uniform_location(program, "u_depth"),
            screen: gl.get_uniform_location(program, "u_screen"),
            smooth: gl.get_uniform_location(program, "u_smooth"),
            reach_light: gl.get_uniform_location(program, "u_reach_light"),
            contrast: gl.get_uniform_location(program, "u_contrast"),
            sphere_warp: gl.get_uniform_location(program, "u_sphere_warp"),
            palette_shift: gl.get_uniform_location(program, "u_palette_shift"),
            plasma_second_sprite: gl.get_uniform_location(program, "u_plasma_second_sprite"),
            unit_particle_alpha: gl.get_uniform_location(program, "u_unit_particle_alpha"),
        })
    }
}

/// The preview's renderer: what every game draws with ([`RenderCore`]),
/// and each game's own drawing ([`GameRender`]), which it hands the frame to.
struct ModelGlRenderer {
    core: RenderCore,
    games: GameRenders,
}

/// What a game's drawing is handed each frame.
pub(super) struct FrameCx<'a> {
    pub(super) gl: &'a glow::Context,
    pub(super) frame: &'a ModelGpuFrame,
    pub(super) core: &'a mut RenderCore,
    /// The frame's slot in `core`.
    pub(super) slot: usize,
}

/// A game's drawing: the renderer runs the frame and hands these steps to
/// the game it is from.
pub(super) trait GameRender {
    /// Before the scene target: what moves the geometry or makes lights.
    unsafe fn prepare(&mut self, cx: &mut FrameCx<'_>);
    /// The shading, when the mode draws any.
    unsafe fn shade(&mut self, cx: &mut FrameCx<'_>);
    /// After the wireframe: what draws over it.
    unsafe fn finish(&mut self, cx: &mut FrameCx<'_>);

    /// Each simulation the game keeps for `slot` while it holds its `uploads`th
    /// model, by name: what the G-sim gate reads.
    #[cfg(test)]
    fn each_simulation(&self, _slot: usize, _uploads: u64, _visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {}
}

/// Each game's drawing, by the game a preview is from.
#[derive(Default)]
struct GameRenders {
    plain: common::plain::PlainRender,
    dx9: dx9::render::Dx9Render,
    gen3: gen3::render::Gen3Render,
}

impl GameRenders {
    /// Each game's own simulations for `slot`.
    #[cfg(test)]
    fn each_simulation(&self, slot: usize, uploads: u64, visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {
        let Self { plain, dx9, gen3 } = self;
        for game in [plain as &dyn GameRender, dx9, gen3] {
            game.each_simulation(slot, uploads, visit);
        }
    }

    /// The drawing for `frame`, by [`Route::of`] its preview.
    fn of(&mut self, frame: &ModelGpuFrame) -> &mut dyn GameRender {
        match Route::of(&frame.preview) {
            Route::Gen3 => &mut self.gen3,
            Route::Dx9 => &mut self.dx9,
            Route::Plain => &mut self.plain,
        }
    }
}

/// Which drawing a preview goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Gen3,
    Dx9,
    Plain,
}

impl Route {
    /// By the game the preview is from. One of no game this draws draws as
    /// Halo 3's when its tag is gen3 (from a source that names no game), and
    /// plainly when no tag gave it (Chimp's meshes).
    fn of(preview: &RenderModelPreview) -> Self {
        match preview.game {
            Some(GameId::Halo3 | GameId::Halo3Odst | GameId::HaloReach | GameId::Halo4) => Self::Gen3,
            Some(GameId::HaloCe | GameId::Halo2) => Self::Dx9,
            _ if preview.generation == Some(blam_tags::game::Game::Halo3) => Self::Gen3,
            _ => Self::Plain,
        }
    }
}

impl ModelGlRenderer {
    fn new(gl: &glow::Context) -> Result<Self, String> {
        Ok(Self { core: RenderCore::new(gl)?, games: GameRenders::default() })
    }

    /// Draw `frame`: its slot, the game's preparation, the scene target, the
    /// shared program and grid, the game's shading, the wireframe, what the
    /// game draws over it, and the scene put back.
    unsafe fn paint(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        // Set again if this frame steps the effects and something runs on.
        CE_EVENTS_RUNNING.with(|running| running.borrow_mut().remove(&frame.geometry_id));
        unsafe {
            let Some(slot) = self.core.acquire_frame_slot(gl, frame) else {
                return;
            };
            let game = self.games.of(frame);
            game.prepare(&mut FrameCx { gl, frame, core: &mut self.core, slot });
            let scene = self.core.begin_scene(gl, frame);
            // The game's passes need the scene target's stencil and copies;
            // without one, the frame draws as with them off.
            if scene.is_none() && frame.game_shaders && self.core.shaders_supported {
                let plain = ModelGpuFrame { game_shaders: false, ..frame.clone() };
                self.paint(gl, &plain);
                return;
            }
            self.core.begin_drawing(gl, frame, slot);
            let game = self.games.of(frame);
            if frame.render_mode.draws_shading() {
                gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
                gl.uniform_1_f32(Some(&self.core.unlit), 0.0);
                game.shade(&mut FrameCx { gl, frame, core: &mut self.core, slot });
            }
            if frame.render_mode.draws_wireframe() {
                self.core.draw_wireframe(gl, frame, slot);
            }
            game.finish(&mut FrameCx { gl, frame, core: &mut self.core, slot });
            self.core.end_frame(gl, scene);
        }
    }

    #[cfg(test)]
    pub(super) fn for_each_simulation(&self, geometry_id: u64, mut visit: impl FnMut(&str, &dyn std::fmt::Debug)) {
        self.core.for_each_simulation(geometry_id, &mut visit);
        if let Some(slot) = self.core.slot_of(geometry_id) {
            self.games.each_simulation(slot, self.core.slot_uploads(slot), &mut visit);
        }
    }
}

/// The GL state the preview draws with: the shared program, slots and
/// targets, and, until each game's drawing moves to its own folder, every
/// game's own state too.
pub(super) struct RenderCore {
    /// The normals the gen3 albedo pass laid down, which Halo 3's dx9
    /// shadow apply reads: the gen3 targets' own, kept as they are remade.
    pub(in crate::app::model_preview) h3_normal_target: Option<glow::NativeTexture>,
    /// What a gen3 array sampler reads with nothing of its own.
    pub(in crate::app::model_preview) black_array: Option<glow::NativeTexture>,
    /// As `h2_bloom`.
    pub(in crate::app::model_preview) sprite_gl: Option<Option<SpriteGl>>,
    /// The frame's CE point lights with those its effects have made, which
    /// only the drawing knows; none while effects make no lights.
    pub(in crate::app::model_preview) ce_points_now: Option<Vec<[[f32; 4]; ce::lights::CE_POINT_LIGHT_ROWS]>>,
    /// As `h2_bloom`.
    pub(in crate::app::model_preview) particle_gl: Option<Option<H3ParticleGl>>,
    /// As `h2_bloom`.
    pub(in crate::app::model_preview) reach_cheap_gl: Option<Option<ReachCheapGl>>,
    /// Particles step once a frame, however many calls draw them.
    pub(in crate::app::model_preview) sprites_advanced: bool,
    /// Halo 2's `target_particle_distortion`, copied from the frame.
    pub(in crate::app::model_preview) h2_distortion_copy: Option<ColorTarget>,
    /// Whether this context's GLSL can draw the games' shaders, which every
    /// pipeline's scene needs; see `H2_GLSL_VERSION`.
    shaders_supported: bool,
    /// Linked pass programs. `None` records a failure, so it is not retried
    /// every frame and the material keeps drawing as it did.
    h2_programs: HashMap<(H2VertexStage, String), Option<Rc<Linked>>>,
    /// Whether this context can draw the gen3 games' shaders: the translated
    /// shaders want GLSL 4.10.
    pub(in crate::app::model_preview) gen3_supported: bool,
    /// What a map nothing fills reads: an unmapped particle stage, a
    /// composite with no bloom built.
    pub(in crate::app::model_preview) black_flat: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) black_cube: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) black_volume: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) white_volume: Option<glow::NativeTexture>,
    h2_normalization_cube: Option<glow::NativeTexture>,
    h2_target: Option<H2Target>,
    pub(in crate::app::model_preview) scene: Option<H2Target>,
    h2_frame_copy: Option<H2Target>,
    /// The gel and `H2GelRows` of the frustum light drawing now.
    pub(in crate::app::model_preview) h2_gel: Option<(Option<glow::NativeTexture>, [[f32; 4]; 4])>,
    /// Made on first use; `Some(None)` once making it failed, so a driver
    /// that cannot is not asked again (and nothing leaks) every frame.
    h2_bloom: Option<Option<H2Bloom>>,
    /// As `h2_bloom`.
    h2_shadow_map: Option<Option<(glow::NativeFramebuffer, glow::NativeTexture)>>,
    pub(in crate::app::model_preview) program: glow::NativeProgram,
    pub(in crate::app::model_preview) slots: Vec<ModelGpuSlot>,
    lru: ModelSlotLru,
    center: glow::NativeUniformLocation,
    scale: glow::NativeUniformLocation,
    angles: glow::NativeUniformLocation,
    clip_scale: glow::NativeUniformLocation,
    depth_scale: glow::NativeUniformLocation,
    depth_map: glow::NativeUniformLocation,
    perspective: glow::NativeUniformLocation,
    /// Optional like the texture uniforms: a driver may strip the skinning
    /// path from a program whose draws never enable it.
    animated: Option<glow::NativeUniformLocation>,
    bones: Option<glow::NativeUniformLocation>,
    base_color: glow::NativeUniformLocation,
    unlit: glow::NativeUniformLocation,
    shaded: Option<glow::NativeUniformLocation>,
    have_a: Option<glow::NativeUniformLocation>,
    have_b: Option<glow::NativeUniformLocation>,
    uv_scale_a: Option<glow::NativeUniformLocation>,
    uv_scale_b: Option<glow::NativeUniformLocation>,
    uv_scale_c: Option<glow::NativeUniformLocation>,
    detail: Option<glow::NativeUniformLocation>,
    /// One sampler location per slot, in `TextureSlot` order.
    samplers: [Option<glow::NativeUniformLocation>; SLOT_COUNT],
    /// Bound to every slot a material lacks; see [`Self::bind_material`].
    pub(in crate::app::model_preview) placeholder: Option<glow::NativeTexture>,
}

/// One material's textures on the GPU, in `TextureSlot` order.
struct MaterialGlTextures {
    textures: [Option<glow::NativeTexture>; SLOT_COUNT],
    /// By `H2Layer` and pass, one texture per entry of its `textures`.
    h2_textures: [Vec<Vec<Option<glow::NativeTexture>>>; H2_LAYER_COUNT],
    uv_scales: [[f32; 2]; SLOT_COUNT],
    detail: DetailComposition,
}

impl Default for MaterialGlTextures {
    fn default() -> Self {
        Self {
            textures: Default::default(),
            h2_textures: Default::default(),
            uv_scales: [[1.0; 2]; SLOT_COUNT],
            detail: DetailComposition::default(),
        }
    }
}

impl RenderCore {
    /// Each simulation `geometry_id`'s slot keeps between frames, by name,
    /// for the refactor's G-sim gate.
    #[cfg(test)]
    pub(super) fn for_each_simulation(&self, geometry_id: u64, visit: impl FnMut(&str, &dyn std::fmt::Debug)) {
        if let Some(slot) = self.slot_of(geometry_id) {
            self.slots[slot].simulations.each(visit);
        }
    }

    /// How many models `slot` has taken: a game's own state for the slot
    /// starts afresh when this moves on.
    pub(in crate::app::model_preview) fn slot_uploads(&self, slot: usize) -> u64 {
        self.slots[slot].uploads
    }

    /// The slot holding `geometry_id`, if one does.
    #[cfg(test)]
    fn slot_of(&self, geometry_id: u64) -> Option<usize> {
        let index = self.lru.slots.iter().position(|(id, _)| *id == geometry_id)?;
        (index < self.slots.len()).then_some(index)
    }

    /// `flag_render_proper` builds its mesh from where the sheet is now.
    pub(in crate::app::model_preview) unsafe fn move_ce_flags(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        ce: &mut ce::slot::CeSlot,
    ) {
        let uploaded = &mut self.slots[slot];
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        let lowest = frame.preview.vertices.iter().map(|vertex| vertex.position[2]);
        let ground = lowest.fold(0.0, f32::min);
        let written = ce.simulations.flags.advance(&frame.preview.ce.flags, bones, ground, frame.time);
        for (first, moved) in written {
            let moved = moved.into_iter().map(|(position, normal)| (position, normal, None));
            unsafe { uploaded.rewrite_vertices(gl, frame, first, moved) };
        }
    }

    fn new(gl: &glow::Context) -> Result<Self, String> {
        let shader_version = eframe::egui_glow::ShaderVersion::get(gl);
        let modern = shader_version.is_new_shader_interface();
        let precision = if shader_version.is_embedded() { "precision mediump float;\n" } else { "" };
        let (vertex_source, fragment_source) =
            model_shader_sources(shader_version.version_declaration(), modern, precision);

        unsafe {
            let stages = [(glow::VERTEX_SHADER, vertex_source.as_str()), (glow::FRAGMENT_SHADER, fragment_source.as_str())];
            let program = link(gl, &stages, &[]).map_err(|error| format!("model preview shader: {error}"))?;

            let uniform = |name| {
                gl.get_uniform_location(program, name)
                    .ok_or_else(|| format!("model preview shader has no {name} uniform"))
            };
            let version = gl.version();
            // Made in this order so every texture keeps the name it had.
            let black_array = upload_solid_volume(gl, glow::TEXTURE_2D_ARRAY, [0, 0, 0, 255]);
            let black_flat = upload_black_flat(gl);
            Ok(Self {
                gen3_supported: !version.is_embedded && (version.major, version.minor) >= (4, 1),
                black_flat,
                black_array,
                h3_normal_target: None,
                sprite_gl: None,
                shaders_supported: h2_context_supported(version.major, version.minor, version.is_embedded),
                h2_programs: HashMap::new(),
                black_cube: upload_black_cube(gl),
                particle_gl: None,
                reach_cheap_gl: None,
                ce_points_now: None,
                sprites_advanced: false,
                h2_distortion_copy: None,
                black_volume: upload_solid_volume(gl, glow::TEXTURE_3D, [0, 0, 0, 255]),
                white_volume: upload_solid_volume(gl, glow::TEXTURE_3D, [255; 4]),
                h2_normalization_cube: upload_normalization_cube(gl),
                h2_target: None,
                scene: None,
                h2_frame_copy: None,
                h2_gel: None,
                h2_bloom: None,
                h2_shadow_map: None,
                program,
                slots: Vec::new(),
                lru: ModelSlotLru::default(),
                center: uniform("u_center")?,
                scale: uniform("u_scale")?,
                angles: uniform("u_angles")?,
                clip_scale: uniform("u_clip_scale")?,
                depth_scale: uniform("u_depth_scale")?,
                depth_map: uniform("u_depth_map")?,
                perspective: uniform("u_perspective")?,
                animated: gl.get_uniform_location(program, "u_animated"),
                bones: gl.get_uniform_location(program, "u_bones"),
                base_color: uniform("u_base_color")?,
                unlit: uniform("u_unlit")?,
                // Optional: a driver is free to drop a uniform the program does
                // not end up reading, and the untextured path reads none of
                // these. Missing is not an error.
                shaded: gl.get_uniform_location(program, "u_shaded"),
                have_a: gl.get_uniform_location(program, "u_have_a"),
                have_b: gl.get_uniform_location(program, "u_have_b"),
                uv_scale_a: gl.get_uniform_location(program, "u_uv_scale_a"),
                uv_scale_b: gl.get_uniform_location(program, "u_uv_scale_b"),
                uv_scale_c: gl.get_uniform_location(program, "u_uv_scale_c"),
                detail: gl.get_uniform_location(program, "u_detail"),
                samplers: SAMPLER_UNIFORMS.map(|name| gl.get_uniform_location(program, name)),
                placeholder: upload_placeholder_texture(gl),
            })
        }
    }

    /// The frame's slot, its geometry and textures uploaded and its
    /// simulations restarted for a new model; none if no slot can be made.
    unsafe fn acquire_frame_slot(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) -> Option<usize> {
        unsafe {
            let (slot, upload) = self.lru.acquire(frame.geometry_id, MODEL_GPU_SLOTS);
            if slot == self.slots.len() {
                match ModelGpuSlot::new(gl, self.program) {
                    Ok(created) => self.slots.push(created),
                    Err(error) => {
                        eprintln!("model preview: {error}");
                        self.lru.abandon(slot);
                        return None;
                    }
                }
            }
            if upload {
                self.slots[slot].restart_simulations();
                self.slots[slot].uploads += 1;
            }
            // Overlays merging in keep the slot, and its running effects,
            // and change only the buffers.
            if upload || self.slots[slot].mesh_id != Some(frame.mesh_id) {
                self.slots[slot].upload_geometry(gl, frame);
            }
            self.slots[slot].sync_textures(gl, frame);
            self.ce_points_now = None;
            Some(slot)
        }
    }

    /// The shared program's state, camera and pose, then the ground grid.
    unsafe fn begin_drawing(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
            // Sampler i reads texture unit i, fixed for the life of the program.
            for (unit, location) in self.samplers.iter().enumerate() {
                if let Some(location) = location {
                    gl.uniform_1_i32(Some(location), unit as i32);
                }
            }
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            gl.front_face(glow::CCW);
            if frame.show_backfaces {
                gl.disable(glow::CULL_FACE);
            } else {
                gl.enable(glow::CULL_FACE);
                gl.cull_face(glow::BACK);
            }
            gl.uniform_3_f32(
                Some(&self.center),
                frame.camera.center[0],
                frame.camera.center[1],
                frame.camera.center[2],
            );
            gl.uniform_1_f32(Some(&self.scale), frame.camera.scale);
            gl.uniform_2_f32(Some(&self.angles), frame.camera.yaw, frame.camera.pitch);
            gl.uniform_2_f32(
                Some(&self.clip_scale),
                frame.camera.clip_scale[0],
                frame.camera.clip_scale[1],
            );
            gl.uniform_1_f32(Some(&self.depth_scale), frame.camera.depth_scale);
            gl.uniform_2_f32(
                Some(&self.depth_map),
                frame.camera.depth_map[0],
                frame.camera.depth_map[1],
            );
            gl.uniform_1_f32(Some(&self.perspective), frame.camera.perspective);
            match (frame.bones.as_ref(), &self.animated) {
                (Some(rows), Some(animated)) => {
                    gl.uniform_1_f32(Some(animated), 1.0);
                    if let Some(bones) = &self.bones {
                        gl.uniform_4_f32_slice(Some(bones), rows.as_flattened());
                    }
                }
                (None, Some(animated)) => gl.uniform_1_f32(Some(animated), 0.0),
                _ => {}
            }

            // The ground grid draws first, depth-tested and depth-written, so
            // the model correctly occludes the lines behind it and stands on
            // the ones in front.
            if frame.show_grid {
                self.slots[slot].sync_grid(gl, frame);
                let ModelGpuSlot {
                    grid_vertex_array,
                    grid_vertex_count,
                    grid_axis_start,
                    vertex_array,
                    ..
                } = self.slots[slot];
                if grid_vertex_count > 0 {
                    gl.bind_vertex_array(Some(grid_vertex_array));
                    if let Some(animated) = &self.animated {
                        gl.uniform_1_f32(Some(animated), 0.0);
                    }
                    gl.uniform_1_f32(Some(&self.unlit), 1.0);
                    self.bind_material(gl, None);
                    gl.line_width(1.0);
                    let flat = |gl: &glow::Context, location, [r, g, b]: [f32; 3]| {
                        gl.uniform_3_f32(Some(location), r, g, b);
                    };
                    // Minor lines a step off the clear color; the world X
                    // and Y axes in Blender's muted red and green.
                    flat(gl, &self.base_color, grid_line_color(frame.background));
                    gl.draw_arrays(glow::LINES, 0, grid_axis_start);
                    let axis_verts = grid_vertex_count - grid_axis_start;
                    if axis_verts >= 2 {
                        flat(gl, &self.base_color, [0.72, 0.42, 0.42]);
                        gl.draw_arrays(glow::LINES, grid_axis_start, 2);
                    }
                    if axis_verts >= 4 {
                        flat(gl, &self.base_color, [0.42, 0.62, 0.42]);
                        gl.draw_arrays(glow::LINES, grid_axis_start + 2, 2);
                    }
                    gl.bind_vertex_array(Some(vertex_array));
                    if frame.bones.is_some()
                        && let Some(animated) = &self.animated
                    {
                        gl.uniform_1_f32(Some(animated), 1.0);
                    }
                }
            }

        }
    }


    /// The slot's effect onsets: when each effect's scale came on.
    pub(in crate::app::model_preview) fn effect_onsets(&mut self, slot: usize) -> &mut effects::EffectOnsets {
        &mut self.slots[slot].simulations.effect_onsets
    }

    /// Bind the slot's model vertex array again.
    pub(in crate::app::model_preview) unsafe fn bind_slot_vertices(&self, gl: &glow::Context, slot: usize) {
        unsafe { gl.bind_vertex_array(Some(self.slots[slot].vertex_array)) }
    }

    /// The slot's own draw of the visible batches, in the shared program:
    /// what a game's passes do not replace.
    pub(super) unsafe fn draw_slot_batches(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        unsafe { self.draw_batches(gl, frame, false, Self::bound_materials(frame, &self.slots[slot].materials)) }
    }

    /// The wireframe over what shaded, in the shared program.
    unsafe fn draw_wireframe(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        unsafe {
            if frame.render_mode.draws_wireframe() {
                // Halo 3's passes end on their own programs; the camera and
                // material uniforms below are the preview's.
                gl.use_program(Some(self.program));
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE);
                gl.line_width(1.0);
                gl.depth_func(glow::LEQUAL);
                gl.uniform_1_f32(Some(&self.unlit), 1.0);
                self.draw_batches(gl, frame, true, Self::bound_materials(frame, &self.slots[slot].materials));
            }
        }
    }

    /// Halo 2's bloom over `bloom_order`, after the wireframe and the sprites.
    pub(in crate::app::model_preview) unsafe fn draw_dx9_bloom(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize, bloom_order: Option<Vec<usize>>) {
        unsafe {
            if let Some(order) = bloom_order {
                gl.use_program(Some(self.program));
                gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
                self.draw_h2_bloom(gl, frame, slot, &order);
            }

        }
    }

    /// The frame's end: fill polygons again, `scene` copied to where the
    /// frame belonged, and nothing left bound.
    unsafe fn end_frame(&mut self, gl: &glow::Context, scene: Option<PostScope>) {
        unsafe {
            // Polygon mode is not reset by egui_glow's generic callback-state
            // restoration and would otherwise turn subsequent UI meshes into wireframes.
            gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
            if let Some(scene) = scene {
                self.finish_scene(gl, scene);
            }
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.use_program(None);
        }
    }

    /// Whether a frame has flares or antennas to draw.
    pub(in crate::app::model_preview) fn draws_object_sprites(&self, frame: &ModelGpuFrame) -> bool {
        let preview = &frame.preview;
        frame.game_shaders
            && self.shaders_supported
            && !(preview.ce.lights.is_empty()
                && preview.ce.antennas.is_empty()
                && preview.ce.effects.is_empty()
                && preview.ce.contrails.is_empty()
                && preview.ce.light_volumes.is_empty()
                && preview.ce.lightning.is_empty()
                && preview.ce.glows.is_empty()
                && preview.ce.particle_systems.is_empty()
                && preview.antennas.is_empty()
                && preview.h2.flares.is_empty()
                && preview.gen3.flares.is_empty()
                && preview.h4.flares.is_empty()
                && preview.gen3.light_volumes.is_empty()
                && preview.h4.tracers.is_empty()
                && preview.h2.light_volumes.is_empty()
                && preview.h2.contrails.is_empty()
                && preview.h2.liquids.is_empty()
                && preview.particles.is_empty()
                && preview.gen3.cheap_emitters.is_empty()
                && preview.gen3.strips.is_empty()
                && preview.h3.leaves.is_empty())
    }

    /// Halo CE's transparents as `rasterizer_transparent_geometry_draw` draws
    /// them: the model's transparent parts and the effects' groups in one sort
    /// by `z_sort`, far to near, each part from its centroid on its node.
    pub(in crate::app::model_preview) unsafe fn draw_ce_transparents(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        ce: &mut ce::slot::CeSlot,
        effect_gl: &mut Option<Option<CeEffectGl>>,
    ) {
        enum Transparent {
            Part(usize),
            Group(CeGroup),
        }
        unsafe {
            let view = ce_flare_view(&frame.camera);
            let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
            let mut groups = Vec::new();
            if self.draws_object_sprites(frame) && ce_has_groups(&frame.preview) {
                self.sprite_gl.get_or_insert_with(|| link_sprite_gl(gl, self.program));
                self.slots[slot].sync_sprite_textures(gl, frame);
                groups = ce_groups(gl, ce, frame, bones, &view);
            }
            let points = &self.slots[slot].h2_sort_points;
            let parts = frame.visible_batches.iter().map(|&index| {
                let place = frame.preview.batches.get(index).and_then(|batch| batch.sort_position);
                let point = match place {
                    Some((point, node)) => common::view::posed_on(node, point, [0.0; 3], bones).0,
                    None => points.get(index).copied().unwrap_or(frame.camera.center),
                };
                (ce_z_sort(&view, point, false), Transparent::Part(index))
            });
            let groups = groups.into_iter().map(|group| (group.z_sort, Transparent::Group(group)));
            let mut sorted: Vec<_> = parts.chain(groups).collect();
            sorted.sort_by(|(a, _), (b, _)| a.total_cmp(b));
            let vertex_array = self.slots[slot].vertex_array;
            for (_, transparent) in sorted {
                match transparent {
                    Transparent::Part(index) => {
                        // What the passes take as theirs: the model's vertices
                        // and the frame's culling.
                        gl.bind_vertex_array(Some(vertex_array));
                        if frame.show_backfaces {
                            gl.disable(glow::CULL_FACE);
                        } else {
                            gl.enable(glow::CULL_FACE);
                        }
                        self.draw_h2_passes(gl, frame, slot, &[index], H2Layer::Transparent, None);
                    }
                    Transparent::Group(group) => {
                        let Some(flares) = self.sprite_gl.as_ref().and_then(Option::as_ref) else {
                            continue;
                        };
                        draw_ce_group(gl, effect_gl, self.program, flares, ce, &frame.camera, group);
                    }
                }
            }
            // As the passes leave it.
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(vertex_array));
            gl.disable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            gl.depth_mask(true);
            gl.color_mask(true, true, true, true);
            gl.active_texture(glow::TEXTURE0);
            if frame.show_backfaces {
                gl.disable(glow::CULL_FACE);
            } else {
                gl.enable(glow::CULL_FACE);
            }
        }
    }

    /// The slot's uploaded maps, or none in a mode that draws without them.
    fn bound_materials<'a>(frame: &ModelGpuFrame, uploaded: &'a [MaterialGlTextures]) -> &'a [MaterialGlTextures] {
        if frame.textures.is_some() { uploaded } else { &[] }
    }

    fn h2_passes(frame: &ModelGpuFrame, material: usize, layer: H2Layer) -> &[H2DrawPass] {
        frame
            .textures
            .as_ref()
            .and_then(|textures| textures.get(material))
            .map_or(&[], |material| material.h2_layers[layer as usize].as_slice())
    }

    /// Whether the model is camouflaged: camo needs the model's shader for it.
    pub(in crate::app::model_preview) fn h2_camo_on(&self, frame: &ModelGpuFrame) -> bool {
        frame.active_camo
            && (0..frame.preview.materials.len())
                .any(|material| self.h2_material_ready(frame, material, H2Layer::ActiveCamo))
    }

    /// Whether a material's passes stand in for the slots. Transparent ones
    /// always do. Opaque ones need game shaders on, and a template with a
    /// lightmap layer needs that layer to have prepared.
    fn h2_replaces_slots(&self, frame: &ModelGpuFrame, material: usize) -> bool {
        if frame.game_shaders && self.h3_material(frame, material).is_some() {
            return true;
        }
        if self.h2_material_ready(frame, material, H2Layer::Transparent) {
            return true;
        }
        if self.h2_camo_on(frame) {
            return self.h2_material_ready(frame, material, H2Layer::ActiveCamo);
        }
        if !frame.game_shaders {
            return false;
        }
        let lightmap = frame.h2_lightmap_layer();
        self.h2_material_ready(frame, material, lightmap)
            || self.h2_material_ready(frame, material, H2Layer::Prt)
            || (Self::h2_passes(frame, material, lightmap).is_empty()
                && (self.h2_material_ready(frame, material, H2Layer::Overlay)
                    || (frame
                        .textures
                        .as_ref()
                        .and_then(|textures| textures.get(material))
                        .is_none_or(|material| material.get(TextureSlot::Base).is_none())
                        && self.h2_material_ready(frame, material, H2Layer::SelfIllumination))))
    }

    /// Whether a material draws through its passes rather than the slots: it
    /// has some, and every one linked.
    fn h2_material_ready(&self, frame: &ModelGpuFrame, material: usize, layer: H2Layer) -> bool {
        let passes = Self::h2_passes(frame, material, layer);
        self.shaders_supported
            && !passes.is_empty()
            && (layer != H2Layer::Prt || (frame.prt && frame.preview.h2.prt.is_some()))
            && passes.iter().all(|pass| {
                self.h2_programs
                    .get(&(pass.stage, pass.fragment.clone()))
                    .is_some_and(Option::is_some)
            })
    }

    /// Link the dx9 pass programs the frame's materials name: Halo 2's and
    /// Halo CE's passes, and the shadow passes Halo 3 and ODST borrow.
    pub(super) unsafe fn prepare_h2_programs(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        if !self.shaders_supported {
            return;
        }
        let Some(textures) = frame.textures.as_ref() else {
            return;
        };
        let passes = textures.iter().flat_map(|material| material.h2_layers.iter().flatten());
        for pass in passes {
            let key = (pass.stage, pass.fragment.clone());
            if self.h2_programs.contains_key(&key) {
                continue;
            }
            let vertex = pass.stage.source(H2_GLSL_VERSION);
            let linked = unsafe { link_h2_program(gl, self.program, &vertex, &pass.fragment) };
            let program = match linked {
                Ok(program) => Some(Rc::new(Linked::new(program))),
                Err(error) => {
                    eprintln!("model preview: {error}");
                    None
                }
            };
            self.h2_programs.insert(key, program);
        }
    }

    /// With game shaders on, send the whole frame to the scene target. What it
    /// replaces is handed back for `finish_scene`.
    unsafe fn begin_scene(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
    ) -> Option<PostScope> {
        if !frame.game_shaders || !self.shaders_supported {
            return None;
        }
        unsafe {
            let state = PostScope::capture(gl);
            let size = state.size();
            if self.scene.as_ref().is_none_or(|scene| scene.size != size) {
                if let Some(old) = self.scene.take() {
                    old.delete(gl);
                }
                self.scene = H2Target::new(gl, size[0], size[1]);
            }
            let Some(scene) = &self.scene else {
                state.restore(gl);
                return None;
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene.framebuffer));
            gl.viewport(0, 0, size[0], size[1]);
            gl.disable(glow::SCISSOR_TEST);
            let [r, g, b] = frame.background;
            gl.clear_color(r, g, b, 1.0);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            Some(state)
        }
    }

    /// Copy the scene target to where the frame belonged, and put everything
    /// back.
    unsafe fn finish_scene(&self, gl: &glow::Context, state: PostScope) {
        let Some(scene) = &self.scene else {
            return;
        };
        unsafe {
            let [x, y, width, height] = state.viewport;
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(scene.framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, state.framebuffer);
            if state.scissored {
                gl.enable(glow::SCISSOR_TEST);
            }
            gl.scissor(state.scissor[0], state.scissor[1], state.scissor[2], state.scissor[3]);
            gl.blit_framebuffer(
                0,
                0,
                scene.size[0],
                scene.size[1],
                x,
                y,
                x + width,
                y + height,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            state.restore(gl);
        }
    }

    /// Draw every texaccum pass into the target, sized to the live viewport,
    /// and put the framebuffer, viewport and scissor back exactly as found.
    pub(in crate::app::model_preview) unsafe fn fill_h2_texaccum(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
    ) {
        let needed = order.iter().any(|&index| {
            frame.preview.batches.get(index).is_some_and(|batch| {
                let material = batch.material_index as usize;
                self.h2_material_ready(frame, material, H2Layer::Texaccum)
                    || self.h2_material_ready(frame, material, H2Layer::LightAlbedo)
            })
        });
        if !needed {
            return;
        }
        unsafe {
            let state = PostScope::capture(gl);
            let size = state.size();
            if self.h2_target.as_ref().is_none_or(|target| target.size != size) {
                if let Some(old) = self.h2_target.take() {
                    old.delete(gl);
                }
                self.h2_target = H2Target::new(gl, size[0], size[1]);
            }
            if let Some(target) = &self.h2_target {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
                gl.viewport(0, 0, size[0], size[1]);
                gl.disable(glow::SCISSOR_TEST);
                // The game draws texaccum against the scene's depth: a shader
                // may lay its depth down here alone, with its later passes
                // testing for equal.
                let scene_depth = self.scene.as_ref().map(|scene| scene.depth);
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_STENCIL_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(scene_depth.unwrap_or(target.depth)),
                );
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear_depth_f32(1.0);
                gl.clear(if scene_depth.is_some() {
                    glow::COLOR_BUFFER_BIT
                } else {
                    glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT
                });
                self.draw_h2_passes(gl, frame, slot, order, H2Layer::Texaccum, None);
                self.draw_h2_passes(gl, frame, slot, order, H2Layer::LightAlbedo, None);
            }
            state.restore(gl);
        }
    }

    /// Mark in the stencil what the key light cannot reach: the shadow map
    /// from the light's side, then the model in view against it.
    pub(in crate::app::model_preview) unsafe fn draw_h2_shadow(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
    ) -> bool {
        unsafe {
            let Some((framebuffer, _)) = *self.h2_shadow_map.get_or_insert_with(|| h2_shadow_map(gl)) else {
                return false;
            };
            let h3 = self.h3_draws(frame);
            let size = if h3 { H3_SHADOW_MAP_SIZE } else { H2_SHADOW_MAP_SIZE };
            let state = PostScope::capture(gl);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.disable(glow::SCISSOR_TEST);
            gl.depth_mask(true);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.viewport(0, 0, size, size);
            if h3 {
                gl.enable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(H3_SHADOW_SLOPE_BIAS, H3_SHADOW_DEPTH_BIAS * (1 << 24) as f32);
            }
            self.draw_h2_passes(gl, frame, slot, order, H2Layer::ShadowCaster, None);
            gl.disable(glow::POLYGON_OFFSET_FILL);
            state.restore(gl);
            if h3 {
                return true;
            }
            gl.clear_stencil(0);
            gl.clear(glow::STENCIL_BUFFER_BIT);
            gl.enable(glow::STENCIL_TEST);
            gl.stencil_func(glow::ALWAYS, 1, 0xFF);
            gl.stencil_op(glow::KEEP, glow::KEEP, glow::REPLACE);
            self.draw_h2_passes(gl, frame, slot, order, H2Layer::ShadowMask, None);
            gl.disable(glow::STENCIL_TEST);
            true
        }
    }

    /// Write the bloom layer to the frame's alpha, and add the glow from it.
    unsafe fn draw_h2_bloom(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
    ) {
        let Some((framebuffer, color, size)) =
            self.scene.as_ref().map(|scene| (scene.framebuffer, scene.color, scene.size))
        else {
            return;
        };
        unsafe {
            gl.color_mask(false, false, false, true);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.color_mask(true, true, true, true);
            self.draw_h2_passes(gl, frame, slot, order, H2Layer::Bloom, None);
            // `sub_6C18F0`: the hologram shader draws while its control is up.
            let hologram = frame.preview.h2.hologram_control.as_deref().is_some_and(|name| {
                frame.function_values.as_ref().and_then(|values| values.get(name)).is_some_and(|value| *value > 0.0)
            });
            if hologram {
                self.draw_h2_passes(gl, frame, slot, order, H2Layer::HologramBloom, None);
            }
            if let Some(bloom) = self.h2_bloom.get_or_insert_with(|| H2Bloom::new(gl, H2_GLSL_VERSION)) {
                bloom.apply(gl, framebuffer, color, size, frame.show_backfaces);
            }
        }
    }

    /// Keep what lies behind the model, for what shows through it: as
    /// `sub_6BD480` does, before anything of the model has drawn.
    pub(in crate::app::model_preview) unsafe fn copy_h2_frame(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, order: &[usize]) {
        let needed = order.iter().any(|&index| {
            frame.preview.batches.get(index).is_some_and(|batch| {
                let material = batch.material_index as usize;
                H2Layer::ALL.iter().any(|layer| {
                    Self::h2_passes(frame, material, *layer)
                        .iter()
                        .any(|pass| pass.textures.iter().any(|texture| texture.frame_copy))
                })
            })
        });
        let Some(scene) = self.scene.as_ref().filter(|_| needed) else {
            return;
        };
        let (framebuffer, [width, height]) = (scene.framebuffer, scene.size);
        unsafe {
            if self.h2_frame_copy.as_ref().is_none_or(|copy| copy.size != [width, height]) {
                if let Some(old) = self.h2_frame_copy.take() {
                    old.delete(gl);
                }
                self.h2_frame_copy = H2Target::new(gl, width, height);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            }
            if let Some(copy) = &self.h2_frame_copy {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(copy.framebuffer));
                gl.blit_framebuffer(
                    0,
                    0,
                    width,
                    height,
                    0,
                    0,
                    width,
                    height,
                    glow::COLOR_BUFFER_BIT,
                    glow::NEAREST,
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            }
        }
    }

    /// Draw Halo 2 materials through their passes, in `order`. Leaves the GL
    /// state the wireframe pass and the UI expect.
    pub(in crate::app::model_preview) unsafe fn draw_h2_passes(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
        layer: H2Layer,
        light: Option<&H2DefaultLight>,
    ) {
        let eye = h2_eye_position(&frame.camera);
        let mut drew = false;
        let camouflaged = self.h2_camo_on(frame);
        for &batch_index in order {
            let Some(batch) = frame.preview.batches.get(batch_index) else {
                continue;
            };
            let material = batch.material_index as usize;
            if !self.h2_material_ready(frame, material, layer)
                || (layer != H2Layer::Transparent && !self.h2_replaces_slots(frame, material))
            {
                continue;
            }
            let Some((byte_offset, index_count)) = model_draw_range(
                batch.index_start,
                batch.index_count,
                frame.preview.indices.len(),
            ) else {
                continue;
            };
            for (pass_index, pass) in Self::h2_passes(frame, material, layer).iter().enumerate() {
                if camouflaged && pass.hidden_by_camo {
                    continue;
                }
                if pass.numeric.as_ref().is_some_and(|numeric| !numeric.shows(&frame.outputs_of(material))) {
                    continue;
                }
                let key = (pass.stage, pass.fragment.clone());
                let Some(Some(program)) = self.h2_programs.get(&key).cloned() else {
                    continue;
                };
                drew = true;
                unsafe {
                    gl.use_program(Some(program.program));
                    self.bind_h2_uniforms(gl, frame, material, &program, pass, eye, light);
                    let textures = self.slots[slot]
                        .materials
                        .get(material)
                        .and_then(|material| material.h2_textures[layer as usize].get(pass_index));
                    for (index, texture) in pass.textures.iter().enumerate() {
                        let uploaded = textures.and_then(|list| list.get(index)).copied().flatten();
                        gl.active_texture(glow::TEXTURE0 + texture.slot);
                        match texture.kind {
                            SamplerKind::Texture2D => {
                                let target = self.h2_target.as_ref().map(|target| target.color);
                                let gel = self.h2_gel.and_then(|(texture, _)| texture);
                                let bound = if texture.gel {
                                    gel.or(self.placeholder)
                                } else if texture.texaccum {
                                    target.or(self.placeholder)
                                } else if texture.frame_copy {
                                    let copy = self.h2_frame_copy.as_ref();
                                    copy.map(|copy| copy.color).or(self.placeholder)
                                } else {
                                    uploaded.or(self.placeholder)
                                };
                                gl.bind_texture(glow::TEXTURE_2D, bound);
                            }
                            SamplerKind::Cube => gl.bind_texture(
                                glow::TEXTURE_CUBE_MAP,
                                if texture.normalization {
                                    self.h2_normalization_cube
                                } else {
                                    uploaded.or(self.black_cube)
                                },
                            ),
                            SamplerKind::Volume => {
                                let fallback = if texture.falloff {
                                    self.white_volume
                                } else {
                                    self.black_volume
                                };
                                gl.bind_texture(glow::TEXTURE_3D, uploaded.or(fallback));
                            }
                        }
                        let location = program.location(gl, &format!("s{}", texture.slot));
                        if location.is_some() {
                            gl.uniform_1_i32(location.as_ref(), texture.slot as i32);
                        }
                    }
                    match pass.blend {
                        Some(blend) => {
                            gl.enable(glow::BLEND);
                            gl.blend_equation(gl_blend_equation(blend.equation));
                            gl.blend_func(
                                gl_blend_factor(blend.source),
                                gl_blend_factor(blend.destination),
                            );
                            let [a, r, g, b] = blend.constant.map(|value| value as f32 / 255.0);
                            gl.blend_color(r, g, b, a);
                        }
                        None => gl.disable(glow::BLEND),
                    }
                    match pass.depth.compare {
                        Some(compare) => {
                            gl.enable(glow::DEPTH_TEST);
                            gl.depth_func(gl_compare(compare));
                        }
                        None => gl.disable(glow::DEPTH_TEST),
                    }
                    gl.depth_mask(pass.depth.write);
                    if pass.depth_bias != 0.0 {
                        gl.enable(glow::POLYGON_OFFSET_FILL);
                        gl.polygon_offset(0.0, pass.depth_bias * DEPTH_UNITS);
                    } else {
                        gl.disable(glow::POLYGON_OFFSET_FILL);
                    }
                    let [r, g, b, a] = pass.channels;
                    gl.color_mask(r, g, b, a);
                    match pass.cull {
                        H2Cull::Frame => {}
                        H2Cull::Back | H2Cull::Front => {
                            let front = pass.cull == H2Cull::Front;
                            gl.enable(glow::CULL_FACE);
                            gl.cull_face(if front { glow::FRONT } else { glow::BACK });
                        }
                        H2Cull::None => gl.disable(glow::CULL_FACE),
                    }
                    gl.draw_elements(
                        glow::TRIANGLES,
                        index_count,
                        glow::UNSIGNED_INT,
                        byte_offset,
                    );
                    if pass.cull != H2Cull::Frame {
                        gl.cull_face(glow::BACK);
                        if frame.show_backfaces {
                            gl.disable(glow::CULL_FACE);
                        } else {
                            gl.enable(glow::CULL_FACE);
                        }
                    }
                }
            }
        }
        if drew {
            unsafe {
                gl.use_program(Some(self.program));
                gl.disable(glow::BLEND);
                gl.disable(glow::POLYGON_OFFSET_FILL);
                gl.blend_equation(glow::FUNC_ADD);
                gl.enable(glow::DEPTH_TEST);
                gl.depth_func(glow::LESS);
                gl.depth_mask(true);
                gl.color_mask(true, true, true, true);
                gl.active_texture(glow::TEXTURE0);
            }
        }
    }

    unsafe fn bind_h2_uniforms(
        &self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        material: usize,
        program: &Linked,
        pass: &H2DrawPass,
        eye: [f32; 3],
        light: Option<&H2DefaultLight>,
    ) {
        unsafe {
            if matches!(pass.stage, H2VertexStage::CeModel | H2VertexStage::CeGlassLit) {
                let object = frame.preview.materials.get(material).map_or(0, |material| material.object);
                let location = program.location(gl, "u_ce_point");
                let points = self.ce_points_now.as_ref().unwrap_or(&frame.ce_points);
                let rows = points.get(usize::from(object)).copied().unwrap_or_default();
                gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            }
            if matches!(pass.stage, H2VertexStage::ShadowCaster | H2VertexStage::ShadowMask) {
                let location = program.location(gl, "u_shadow_matrix");
                let (toward, shadow_alpha) = if self.h3_draws(frame) {
                    gen3::pipeline::h3_shadow_light(frame)
                } else if frame.default_lights {
                    (h2_prt_lights()[0].direction, 1.0)
                } else {
                    (frame.h2_object_light(), 1.0)
                };
                let matrix = h2_shadow_matrix(&frame.preview, toward);
                gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &matrix);
                // A caster draws into the map: bound then, it would be read
                // and written at once.
                if pass.stage == H2VertexStage::ShadowMask
                    && let Some(Some((_, map))) = self.h2_shadow_map
                {
                    gl.active_texture(glow::TEXTURE0 + H2_SHADOW_MAP_UNIT);
                    gl.bind_texture(glow::TEXTURE_2D, Some(map));
                    let location = program.location(gl, "u_shadow_map");
                    gl.uniform_1_i32(location.as_ref(), H2_SHADOW_MAP_UNIT as i32);
                }
                if let Some(normal) = self.h3_normal_target {
                    gl.active_texture(glow::TEXTURE0 + H3_NORMAL_TARGET_UNIT);
                    gl.bind_texture(glow::TEXTURE_2D, Some(normal));
                    let location = program.location(gl, "u_normal_target");
                    gl.uniform_1_i32(location.as_ref(), H3_NORMAL_TARGET_UNIT as i32);
                    let location = program.location(gl, "u_shadow_toward");
                    gl.uniform_3_f32_slice(location.as_ref(), &toward);
                    let location = program.location(gl, "u_shadow_size");
                    gl.uniform_1_f32(location.as_ref(), H3_SHADOW_MAP_SIZE as f32);
                    let location = program.location(gl, "u_shadow_alpha");
                    gl.uniform_1_f32(location.as_ref(), shadow_alpha);
                }
            }
            for (name, axis) in [
                ("u_view_right", [1.0, 0.0, 0.0]),
                ("u_view_up", [0.0, 0.0, 1.0]),
                ("u_view_back", [0.0, -1.0, 0.0]),
            ] {
                let location = program.location(gl, name);
                if location.is_some() {
                    let vector = unrotate_view_vector(frame.camera.yaw, frame.camera.pitch, axis);
                    gl.uniform_3_f32_slice(location.as_ref(), &vector);
                }
            }
            if let Some(prt) = &frame.preview.h2.prt {
                let location = program.location(gl, "u_prt");
                if location.is_some() {
                    let clusters = match &frame.h2_prt_clusters {
                        Some([lit, shadowed]) => {
                            if light.is_some() { shadowed } else { lit }
                        }
                        None => &prt.clusters,
                    };
                    gl.uniform_4_f32_slice(location.as_ref(), clusters.as_flattened());
                    let direction = if frame.h2_prt_clusters.is_some() {
                        frame.h2_object_light()
                    } else {
                        prt.light
                    };
                    let location = program.location(gl, "u_prt_light");
                    gl.uniform_3_f32_slice(location.as_ref(), &direction);
                }
            }
            let camera = &frame.camera;
            let set_1 = |gl: &glow::Context, program: &Linked, name: &str, value: f32| {
                let location = program.location(gl, name);
                gl.uniform_1_f32(location.as_ref(), value);
            };
            set_1(gl, program, "u_scale", camera.scale);
            set_1(gl, program, "u_depth_scale", camera.depth_scale);
            set_1(gl, program, "u_perspective", camera.perspective);
            set_1(gl, program, "u_reflection_scale", H2_REFLECTION_SCALE);
            set_1(gl, program, "u_animated", if frame.bones.is_some() { 1.0 } else { 0.0 });
            let location = program.location(gl, "u_light_direct");
            let follows = matches!(pass.stage, H2VertexStage::CeModel | H2VertexStage::CeGlassLit)
                && frame.light_follows_camera;
            let direct = if !follows && (frame.default_lights || !pass.stage.is_lightmap()) {
                pass.light_direct
            } else {
                frame.h2_object_light().map(|value| -value)
            };
            gl.uniform_3_f32_slice(location.as_ref(), &direct);
            let location = program.location(gl, "u_light_indirect");
            gl.uniform_3_f32_slice(location.as_ref(), &pass.light_indirect);
            let location = program.location(gl, "u_center");
            gl.uniform_3_f32_slice(location.as_ref(), &camera.center);
            let location = program.location(gl, "u_eye");
            gl.uniform_3_f32_slice(location.as_ref(), &eye);
            if let Some(copy) = &self.h2_frame_copy {
                let location = program.location(gl, "u_view_size");
                gl.uniform_2_f32(location.as_ref(), copy.size[0] as f32, copy.size[1] as f32);
            }
            let location = program.location(gl, "u_angles");
            gl.uniform_2_f32(location.as_ref(), camera.yaw, camera.pitch);
            let location = program.location(gl, "u_clip_scale");
            gl.uniform_2_f32_slice(location.as_ref(), &camera.clip_scale);
            let location = program.location(gl, "u_depth_map");
            gl.uniform_2_f32_slice(location.as_ref(), &camera.depth_map);
            if let Some(rows) = frame.bones.as_ref() {
                let location = program.location(gl, "u_bones");
                gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            }
            let location = program.location(gl, "u_texture_rows");
            let (outputs, change_colors) = (frame.outputs_of(material), frame.change_colors_of(material));
            let clock = H2Clock { time: frame.time, values: frame.function_values_of(material) };
            let rows = pass.texture_rows_at(clock, &outputs);
            gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            let (tint_facing, tint_glancing) = pass.tints_at(clock);
            let location = program.location(gl, "u_tint_facing");
            gl.uniform_4_f32_slice(location.as_ref(), &tint_facing);
            let location = program.location(gl, "u_tint_glancing");
            gl.uniform_4_f32_slice(location.as_ref(), &tint_glancing);
            for (register, value) in &pass.constants {
                let mut value = *value;
                pass.animate_constant(*register, &mut value, clock);
                if let Some(animation) = &pass.ce_animation {
                    animation.constant(
                        *register,
                        &mut value,
                        frame.time,
                        &change_colors,
                        &outputs,
                    );
                    if animation.eye_register == Some(*register) {
                        let [x, y, z] =
                            unrotate_view_vector(frame.camera.yaw, frame.camera.pitch, [0.0, -1.0, 0.0]);
                        value = [x, y, z, 1.0];
                    }
                }
                for (changed, component, channel) in &pass.change_colors {
                    if changed == register {
                        value[*component] = change_colors[*channel][*component];
                    }
                }
                for (changed, component) in &pass.camo_amounts {
                    if changed == register {
                        value[*component] = if frame.active_camo { 1.0 } else { 0.0 };
                    }
                }
                if let Some(light) = light {
                    for (changed, component) in &pass.light_colors {
                        if changed == register {
                            value[*component] = light.color[*component];
                        }
                    }
                }
                let location = program.location(gl, &format!("c{register}"));
                let location = location.or_else(|| program.location(gl, &format!("ce_registers[{register}]")));
                gl.uniform_4_f32_slice(location.as_ref(), &value);
            }
            if let Some(light) = light {
                let location = program.location(gl, "u_light_position");
                gl.uniform_3_f32_slice(location.as_ref(), &light.position);
                let location = program.location(gl, "u_light_falloff");
                gl.uniform_1_f32(location.as_ref(), 0.5 / light.radius);
            }
            if let Some((_, rows)) = &self.h2_gel {
                let location = program.location(gl, "u_gel_rows");
                gl.uniform_4_f32_slice(location.as_ref(), rows.as_flattened());
            }
            if let (H2VertexStage::LightGel, Some(location), Some((gel, _))) =
                (pass.stage, program.location(gl, "u_gel_map"), &self.h2_gel)
            {
                gl.active_texture(glow::TEXTURE0 + H2_GEL_MASK_UNIT);
                gl.bind_texture(glow::TEXTURE_2D, gel.or(self.placeholder));
                gl.uniform_1_i32(Some(&location), H2_GEL_MASK_UNIT as i32);
            }
            let location = program.location(gl, "zwriteout");
            gl.uniform_4_f32(location.as_ref(), 1.0, 1.0, 1.0, 1.0);
        }
    }

    /// Bind one material's textures, and tell the shader which slots are live.
    ///
    /// A slot with no texture is flagged absent, which makes the shader skip
    /// it, and gets the 1x1 placeholder bound rather than nothing. Leaving
    /// the slot on texture 0 is legal GL, but macOS's driver checks every
    /// active sampler at each draw, whether or not the shader branch reads it,
    /// and logs "unit 0 GLD_TEXTURE_INDEX_2D is unloadable" for any batch
    /// without a base map. That includes every batch while the textures are
    /// still loading.
    unsafe fn bind_material(&self, gl: &glow::Context, material: Option<&MaterialGlTextures>) {
        let mut have = [0.0f32; SLOT_COUNT];
        let mut scales = [[1.0f32; 2]; SLOT_COUNT];
        for slot in ALL_SLOTS {
            let index = slot as usize;
            let texture = material.and_then(|material| material.textures[index]);
            have[index] = if texture.is_some() { 1.0 } else { 0.0 };
            if let Some(material) = material {
                scales[index] = material.uv_scales[index];
            }
            unsafe {
                gl.active_texture(glow::TEXTURE0 + index as u32);
                gl.bind_texture(glow::TEXTURE_2D, texture.or(self.placeholder));
            }
        }
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            if let Some(location) = &self.shaded {
                gl.uniform_1_f32(Some(location), if material.is_some() { 1.0 } else { 0.0 });
            }
            if let Some(location) = &self.have_a {
                gl.uniform_4_f32(Some(location), have[0], have[1], have[2], have[3]);
            }
            if let Some(location) = &self.have_b {
                gl.uniform_4_f32(Some(location), have[4], have[5], 0.0, 0.0);
            }
            for (location, [first, second]) in [
                (&self.uv_scale_a, [0, 1]),
                (&self.uv_scale_b, [2, 3]),
                (&self.uv_scale_c, [4, 5]),
            ] {
                if let Some(location) = location {
                    let ([a, b], [c, d]) = (scales[first], scales[second]);
                    gl.uniform_4_f32(Some(location), a, b, c, d);
                }
            }
            if let Some(location) = &self.detail {
                let detail = material.map(|material| material.detail).unwrap_or_default();
                let function = match detail.function {
                    DetailFunction::LinearBiasedMultiply => 0.0,
                    DetailFunction::BiasedMultiply => 1.0,
                    DetailFunction::Multiply => 2.0,
                    DetailFunction::BiasedAdd => 3.0,
                };
                let xbox = if detail.xbox_channel_order { 1.0 } else { 0.0 };
                gl.uniform_4_f32(Some(location), function, detail.mask as f32, xbox, 0.0);
            }
        }
    }

    unsafe fn draw_batches(
        &self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        wire: bool,
        materials: &[MaterialGlTextures],
    ) {
        for &batch_index in &frame.visible_batches {
            let Some(batch) = frame.preview.batches.get(batch_index) else {
                continue;
            };
            let Some((byte_offset, index_count)) = model_draw_range(
                batch.index_start,
                batch.index_count,
                frame.preview.indices.len(),
            ) else {
                continue;
            };
            let color = if wire && background_is_dark(frame.background) {
                Color32::from_rgb(190, 205, 215)
            } else if wire {
                Color32::from_rgb(38, 55, 65)
            } else {
                batch
                    .flat_color
                    .map(|[r, g, b]| Color32::from_rgb(r, g, b))
                    .unwrap_or_else(|| material_color(batch.material_index))
            };
            // Passes draw after every opaque batch; see `draw_h2_passes`.
            let index = batch.material_index as usize;
            if !wire && self.h2_replaces_slots(frame, index) {
                continue;
            }
            // The wireframe pass draws flat on purpose, so it never binds a
            // texture — and neither does a batch whose material did not resolve.
            let material = (!wire)
                .then(|| materials.get(batch.material_index as usize))
                .flatten();
            unsafe {
                self.bind_material(gl, material);
                gl.uniform_3_f32(
                    Some(&self.base_color),
                    color.r() as f32 / 255.0,
                    color.g() as f32 / 255.0,
                    color.b() as f32 / 255.0,
                );
                gl.draw_elements(
                    glow::TRIANGLES,
                    index_count,
                    glow::UNSIGNED_INT,
                    byte_offset,
                );
            }
        }
    }
}

pub(in crate::app::model_preview) fn model_draw_range(
    index_start: u32,
    index_count: u32,
    total_indices: usize,
) -> Option<(i32, i32)> {
    let start = index_start as usize;
    let available = total_indices.checked_sub(start)?;
    let count = (index_count as usize).min(available);
    let count = count - count % 3;
    if count == 0 || count > i32::MAX as usize {
        return None;
    }
    let byte_offset = start.checked_mul(std::mem::size_of::<u32>())?;
    Some((i32::try_from(byte_offset).ok()?, count as i32))
}

pub(super) fn slice_bytes<T>(slice: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(slice.as_ptr().cast::<u8>(), std::mem::size_of_val(slice)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each game's preview goes to its family's drawing; one of no game goes
    /// to gen3 if its tag is gen3, and is drawn plainly if no tag gave it.
    #[test]
    fn a_preview_routes_by_its_game_then_its_generation() {
        use blam_tags::game::Game;
        let route = |game: Option<GameId>, generation: Option<Game>| {
            Route::of(&RenderModelPreview { game, generation, ..Default::default() })
        };
        for game in [GameId::Halo3, GameId::Halo3Odst, GameId::HaloReach, GameId::Halo4] {
            assert_eq!(route(Some(game), Some(Game::Halo3)), Route::Gen3, "{game:?}");
        }
        assert_eq!(route(Some(GameId::HaloCe), Some(Game::Halo1)), Route::Dx9);
        assert_eq!(route(Some(GameId::Halo2), Some(Game::Halo2)), Route::Dx9);
        assert_eq!(route(None, Some(Game::Halo3)), Route::Gen3, "a gen3 tag of no game");
        assert_eq!(route(None, None), Route::Plain, "geometry no tag gave");
    }

    #[test]
    fn a_shields_parts_leave_the_ordinary_passes_once_materials_are_known() {
        let batch = |material_index| RenderModelPreviewBatch { material_index, ..Default::default() };
        let preview = RenderModelPreview { batches: vec![batch(0), batch(1), batch(0)], ..Default::default() };
        let textures = [MaterialTextures::default(), MaterialTextures { shield_only: true, ..Default::default() }];
        let mut visible = vec![0, 1, 2];
        without_shield_parts(&mut visible, &preview, None);
        assert_eq!(visible, [0, 1, 2], "nothing is known yet");
        without_shield_parts(&mut visible, &preview, Some(&textures));
        assert_eq!(visible, [0, 2]);
    }

    #[test]
    fn a_permutation_of_mesh_none_draws_nothing_whatever_its_count() {
        let permutation = |mesh_index, mesh_count| blam_tags::Permutation {
            name: String::new(),
            mesh_index,
            mesh_count,
            instance_mask: [0; 4],
        };
        let region = |permutations| blam_tags::Region { name: String::new(), permutations };
        let model = RenderModel {
            regions: vec![
                region(vec![permutation(0, 1)]),
                region(vec![permutation(-1, 1)]),
                region(vec![permutation(1, 2)]),
            ],
            ..Default::default()
        };
        assert_eq!(preview_mesh_order(&model, 3), [(0, 0, 0), (2, 0, 1), (2, 0, 2)]);
    }

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

    #[test]
    fn the_camera_light_keeps_its_place_as_the_view_orbits() {
        let toward_eye = |yaw, pitch| unrotate_view_vector(yaw, pitch, [0.0, -1.0, 0.0]);
        let share = |yaw, pitch| {
            let (light, eye): ([f32; 3], [f32; 3]) = (h2_camera_light(yaw, pitch), toward_eye(yaw, pitch));
            (0..3).map(|axis| light[axis] * eye[axis]).sum::<f32>()
        };
        for (yaw, pitch) in [(0.0, 0.0), (1.5, 0.4), (3.1, -0.6), (-2.0, 1.0)] {
            assert!((share(yaw, pitch) - share(0.0, 0.0)).abs() < 1e-5);
        }
        assert!(share(0.0, 0.0) > 0.5);
    }
    use blam_tags::math::{RealPoint2d, RealPoint3d, RealVector3d};
    use blam_tags::render_model::{GeometryPartType, RenderMeshPart, RenderVertex};

    /// An eye at the origin looking along +X.
    fn looking_along_x() -> common::view::ViewFrame {
        common::view::ViewFrame { eye: [0.0; 3], forward: [1.0, 0.0, 0.0], right: [0.0, 1.0, 0.0], up: [0.0, 0.0, 1.0] }
    }

    /// `z_sort` puts the farther first, and `sort bias` a quarter unit nearer.
    #[test]
    fn ce_z_sort_runs_far_to_near_and_sort_bias_draws_later() {
        let view = looking_along_x();
        let far = ce_z_sort(&view, [10.0, 3.0, 0.0], false);
        let near = ce_z_sort(&view, [2.0, -5.0, 1.0], false);
        assert!(far < near, "farther sorts first");
        assert_eq!(ce_z_sort(&view, [2.0, 0.0, 0.0], true), near + CE_SORT_BIAS);
    }

    /// Sprites of one key make one group, at the mean of their corners, or
    /// for a bolt at the middle of their bounds; the groups keep the order
    /// they first appeared in.
    #[test]
    fn ce_sprites_group_by_key_at_their_centroid() {
        let view = looking_along_x();
        let texture = glow::NativeTexture(std::num::NonZeroU32::new(1).unwrap());
        let at = |x: f32| (texture, ce::passes::FramebufferBlend::AlphaBlend, false, [[1.0; 4]; 4], [[x, 0.0, 0.0, 0.0, 0.0]; 4], None);
        let mut sprites = CeSpriteGroups::default();
        sprites.push((CE_PARTICLE, 7, 0), false, false, at(2.0));
        sprites.push((CE_LIGHTNING, 0, 0), false, true, at(1.0));
        sprites.push((CE_PARTICLE, 7, 0), false, false, at(6.0));
        sprites.push((CE_LIGHTNING, 0, 0), false, true, at(9.0));
        sprites.push((CE_LIGHTNING, 0, 0), false, true, at(9.0));
        let groups: Vec<CeGroup> = sprites.sorted_by(&view).collect();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].z_sort, -4.0, "the particles' mean");
        assert_eq!(groups[1].z_sort, -5.0, "the bolt's middle, not its mean");
    }

    /// Two previews on screen draw alternately. Each model is uploaded once
    /// and then stays; with one slot, every draw replaced the other model.
    #[test]
    fn two_visible_previews_each_upload_once() {
        let mut lru = ModelSlotLru::default();
        let mut uploads = 0;
        let mut slots = std::collections::HashSet::new();
        for _ in 0..100 {
            for geometry in [10, 20] {
                let (slot, upload) = lru.acquire(geometry, MODEL_GPU_SLOTS);
                uploads += usize::from(upload);
                slots.insert((geometry, slot));
            }
        }
        assert_eq!(uploads, 2);
        assert_eq!(slots.len(), 2, "each model keeps its own slot");
    }

    /// Past capacity, the model drawn longest ago gives up its slot, not one
    /// still being drawn.
    #[test]
    fn the_least_recently_drawn_model_is_evicted() {
        let mut lru = ModelSlotLru::default();
        for geometry in 1..=4 {
            lru.acquire(geometry, 4);
        }
        // 1 is drawn again, so 2 is now the oldest.
        assert_eq!(lru.acquire(1, 4), (0, false));
        let (slot, upload) = lru.acquire(5, 4);
        assert!(upload);
        assert_eq!(slot, 1, "model 2's slot");
        assert_eq!(lru.acquire(1, 4), (0, false), "still resident");
        assert!(lru.acquire(2, 4).1, "evicted, so uploaded again");
    }

    #[test]
    fn dense_indices_use_full_u32_offsets_and_counts() {
        let start = 70_002_u32;
        let count = 120_003_u32;
        assert_eq!(
            model_draw_range(start, count, 250_000),
            Some(((start * 4) as i32, count as i32))
        );
    }

    #[test]
    fn a_marker_of_one_permutation_goes_with_that_permutation() {
        let selections = HashMap::from([(
            "f_hull".to_owned(),
            ModelRegionSelection { enabled: true, permutation: "base".to_owned() },
        )]);
        let of = |region: &str, permutation: &str| (region.to_owned(), permutation.to_owned());
        assert!(marker_active(&of("f_hull", "base"), &selections));
        assert!(!marker_active(&of("f_hull", "minor"), &selections));
        assert!(!marker_active(&of("hull", "base"), &selections));
        let mut off = selections.clone();
        off.get_mut("f_hull").unwrap().enabled = false;
        assert!(!marker_active(&of("f_hull", "base"), &off));
    }

    /// With `BLAM_TEST_HREK`: the marine's radio antenna is drawn only while
    /// its `trooper_radio` backpack is.
    #[test]
    fn kit_reach_antenna_shows_only_with_its_permutation() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            eprintln!("skipping: BLAM_TEST_HREK is not set");
            return;
        };
        let path = "objects\\characters\\marine\\marine";
        let data = super::super::load::kit_object_tests::kit_preview(
            &root,
            crate::core::game::GameId::HaloReach,
            path,
            "biped",
            b"bipd",
        );
        let shown = |permutation: &str| {
            let mut state = ModelPreviewState::default();
            let selection = ModelRegionSelection { enabled: true, permutation: permutation.to_owned() };
            state.region_selections.insert("flair_backpack".to_owned(), selection);
            H2Shown::of(&data.preview, &state).antennas
        };
        assert_eq!(shown("trooper_radio"), [true]);
        assert_eq!(shown("trooper"), [false]);
    }

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

    /// The depth window must cover an animation's travel: the reach bound is
    /// at least the largest per-frame chain length, and an empty pose adds
    /// nothing.
    #[test]
    fn pose_reach_covers_the_travelling_frame() {
        let frame = |x: f32| {
            vec![
                PreviewNodeTransform {
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    translation: [x, 0.0, 0.9],
                    scale: 1.0,
                },
                PreviewNodeTransform {
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    translation: [0.0, 0.0, 0.25],
                    scale: 1.0,
                },
            ]
        };
        let pose = PreviewAnimationPose::new(0, vec![frame(0.0), frame(3.0)]);
        let reach = pose.reach;
        let travelled = (3.0f32 * 3.0 + 0.9 * 0.9).sqrt() + 0.25;
        assert!(
            reach >= travelled - 1e-4,
            "reach {reach} misses the travelled frame {travelled}"
        );
        let empty = PreviewAnimationPose::new(0, Vec::new());
        assert_eq!(empty.reach, 0.0);
    }

    /// The extended render distance must clip exactly at its declared bounds
    /// in both projections, wherever the dolly has put the eye: near plane
    /// just in front of the eye, far plane at t = K, and post-divide depth
    /// monotone between them so the depth test orders correctly.
    #[test]
    fn depth_remap_clips_at_the_near_and_far_planes_for_any_eye() {
        let k = PREVIEW_RENDER_DISTANCE;

        // Orthographic: w = 1, so z = t·x + y must span NDC over t ∈ [-K, K].
        let [x, y] = depth_map_coefficients(None);
        assert!(((-k) * x + y - -1.0).abs() < 1e-5);
        assert!((k * x + y - 1.0).abs() < 1e-5);

        // Perspective, from the default framing's eye down to a dolly deep
        // inside a level.
        for eye in [1.9, 0.5, 0.02, 0.0005] {
            let [x, y] = depth_map_coefficients(Some(eye));
            let z = |t: f32| t * x + y;
            let w = |t: f32| 1.0 + t / eye;
            let near = -eye * (1.0 - PERSPECTIVE_NEAR_FRACTION);
            let tolerance = 1e-4 * w(k);
            assert!(
                (z(near) + w(near)).abs() < tolerance,
                "eye {eye}: near plane moved"
            );
            assert!(
                (z(k) - w(k)).abs() < tolerance,
                "eye {eye}: far plane short of K"
            );
            let samples = (0..=64)
                .map(|step| near + (k - near) * step as f32 / 64.0)
                .map(|t| z(t) / w(t))
                .collect::<Vec<_>>();
            // Strictly increasing wherever f32 can tell the samples apart;
            // at the deepest dolly the far samples round together, so ties
            // are allowed there but never a reversal.
            assert!(
                samples.windows(2).all(|pair| if eye >= 0.02 {
                    pair[1] > pair[0]
                } else {
                    pair[1] >= pair[0]
                }),
                "eye {eye}: post-divide depth not monotone"
            );
        }
    }

    #[test]
    fn a_models_size_does_not_change_how_far_the_view_pulls_back() {
        let widest = |half: f32| {
            let preview = RenderModelPreview {
                bounds_min: [-half; 3],
                bounds_max: [half; 3],
                ..Default::default()
            };
            let (_, radius) = preview_center_radius(&preview);
            (radius * 1.1 / min_preview_scale(&preview), radius * 1.1 / 0.02)
        };
        let (tiny, tiny_at_a_fixed_scale) = widest(0.01);
        let (level, level_at_a_fixed_scale) = widest(500.0);
        assert!((tiny - level).abs() < 1.0, "{tiny} {level}");
        assert!(tiny_at_a_fixed_scale * 1000.0 < level_at_a_fixed_scale);
    }

    fn camera(scale: f32, perspective: bool) -> PreviewCamera {
        PreviewCamera {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0)),
            center: [0.0; 3],
            radius: 1.0,
            depth_radius: 1.0,
            // Unrotated: view space is world space, the eye looks along +Y.
            yaw: 0.0,
            pitch: 0.0,
            scale,
            perspective,
        }
    }

    /// How far from the viewport center a point lands, in pixels.
    fn screen_offset(camera: &PreviewCamera, point: [f32; 3]) -> f32 {
        (camera.project(point).pos - camera.rect.center()).length()
    }

    /// On the orbit point's plane, perspective frames exactly like the
    /// orthographic view at every zoom, so toggling never jumps and the
    /// pan / zoom-to-cursor math holds in both.
    #[test]
    fn perspective_matches_orthographic_at_the_focus_plane() {
        for scale in [0.05, 1.0, 20.0, 400.0] {
            let point = [0.3 / scale, 0.0, 0.2 / scale];
            let flat = camera(scale, false).project(point).pos;
            let deep = camera(scale, true).project(point).pos;
            assert!(
                (flat - deep).length() < 1e-3,
                "scale {scale}: {flat:?} vs {deep:?}"
            );
        }
    }

    /// The reported defect: zooming in only magnified the picture, so the
    /// perspective stayed the same strength and up close looked flat. With
    /// the eye dollying in, a point the same world distance nearer the
    /// camera than the orbit point grows ever larger relative to one on the
    /// focus plane.
    #[test]
    fn zooming_in_dollies_the_eye_and_deepens_the_perspective() {
        let convergence = |scale: f32| {
            let camera = camera(scale, true);
            let offset = 0.1 / scale;
            let nearer = screen_offset(&camera, [offset, -0.2, 0.0]);
            let on_plane = screen_offset(&camera, [offset, 0.0, 0.0]);
            nearer / on_plane
        };
        let (wide, close) = (convergence(1.0), convergence(8.0));
        assert!(wide > 1.05, "no perspective at the default zoom: {wide}");
        assert!(
            close > wide * 1.5,
            "zoom did not deepen the perspective: {wide} → {close}"
        );

        // The lens itself does not change: at every zoom the eye sits where
        // a 60° field of view frames the focus plane, `1.1 · radius /
        // tan(30°)` out in scaled units — `1 / scale` of that in world units.
        for scale in [1.0, 8.0, 100.0] {
            let eye_world = camera(scale, true).eye_distance() / scale;
            let expected = 1.1 / (30.0f32.to_radians().tan() * scale);
            assert!((eye_world - expected).abs() < 1e-5 * expected.max(1.0));
        }
    }

    /// Overlays behind the eye are dropped rather than drawn stretched across
    /// the viewport; the orthographic view has no eye and keeps everything.
    #[test]
    fn overlay_points_behind_the_eye_are_not_drawn() {
        let scale = 10.0;
        let perspective = camera(scale, true);
        let eye_world = perspective.eye_distance() / scale;
        let in_front = [0.0, -eye_world * 0.5, 0.0];
        let behind = [0.0, -eye_world * 1.5, 0.0];
        assert!(perspective.project(in_front).in_front);
        assert!(!perspective.project(behind).in_front);
        assert!(camera(scale, false).project(behind).in_front);
    }

    #[test]
    fn draw_range_clamps_to_complete_valid_triangles() {
        assert_eq!(model_draw_range(6, 10, 14), Some((24, 6)));
        assert_eq!(model_draw_range(20, 3, 14), None);
    }

    /// The attribute offsets in `RenderCore::new` are hand-written against
    /// this layout, and nothing else checks them — a reordered or resized field
    /// would silently feed the shader the wrong bytes and show as a model that
    /// renders but looks wrong.
    /// A malformed shader disables the whole preview and reports it only on
    /// stderr, so these check the structure no GPU is here to check.
    /// The pan and zoom-to-cursor math turns screen deltas back into world
    /// moves through `unrotate_view_vector`; if it drifts from the forward
    /// rotation, panning smears diagonally and zoom-to-cursor orbits away
    /// from the pointer instead of onto it.
    #[test]
    fn unrotate_inverts_rotate_for_any_view_angles() {
        for (yaw, pitch) in [(0.0, 0.0), (-0.45, 0.25), (1.2, -1.4), (3.0, 0.9)] {
            for vector in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, -2.0, 5.0]] {
                let there = rotate_view_vector(yaw, pitch, vector);
                let back = unrotate_view_vector(yaw, pitch, there);
                for axis in 0..3 {
                    assert!(
                        (back[axis] - vector[axis]).abs() < 1e-5,
                        "round trip failed at yaw {yaw} pitch {pitch}: {vector:?} -> {back:?}"
                    );
                }
            }
        }
    }

    /// `both_shader_dialects_declare_what_the_renderer_binds` reads the GLSL
    /// as text; a compile error would still
    /// only surface at runtime, as a preview that draws nothing. With
    /// `glslangValidator` on PATH (`brew install glslang`), compile every
    /// dialect egui can hand the renderer; without it, skip by name.
    #[test]
    fn pass_support_reads_the_context_version() {
        assert!(h2_context_supported(4, 1, false), "the macOS core profile");
        assert!(h2_context_supported(3, 3, false));
        assert!(!h2_context_supported(3, 2, false));
        assert!(!h2_context_supported(3, 3, true), "OpenGL ES");
    }

    #[test]
    fn every_shader_dialect_compiles_under_glslang() {
        if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
            eprintln!("skipping: glslangValidator is not on PATH (brew install glslang)");
            return;
        }
        let dir = crate::core::test_kits::unique_temp_dir("model_preview_glsl");
        for (index, (declaration, modern, precision)) in [
            ("#version 330\n", true, ""),
            ("#version 140\n", true, ""),
            ("#version 300 es\n", true, "precision mediump float;\n"),
            ("#version 120\n", false, ""),
            ("#version 100\n", false, "precision mediump float;\n"),
        ]
        .into_iter()
        .enumerate()
        {
            let (vertex, fragment) = model_shader_sources(declaration, modern, precision);
            for (stage, source) in [("vert", vertex), ("frag", fragment)] {
                let path = dir.join(format!("model_{index}.{stage}"));
                std::fs::write(&path, source).expect("write shader");
                let output = std::process::Command::new("glslangValidator")
                    .arg(&path)
                    .output()
                    .expect("run glslangValidator");
                assert!(
                    output.status.success(),
                    "{} {stage} does not compile:\n{}",
                    declaration.trim(),
                    String::from_utf8_lossy(&output.stdout)
                );
            }
        }
    }

    /// The particle program is linked on first use, so a compile error would
    /// only show as effects that draw nothing. Without `glslangValidator`,
    /// skip by name.
    #[test]
    fn the_particle_program_compiles_under_glslang() {
        assert_compiles_under_glslang("particle", &[("vert", PARTICLE_VERTEX), ("frag", PARTICLE_FRAGMENT)]);
    }

    /// Reach's cheap particle program, sampling its layers, as the particle
    /// program is checked.
    #[test]
    fn the_cheap_particle_program_compiles_under_glslang() {
        use super::particles::cheap::{CHEAP_PARTICLE_FRAGMENT, CHEAP_PARTICLE_VERTEX};
        assert_compiles_under_glslang("cheap_particle", &[("vert", CHEAP_PARTICLE_VERTEX), ("frag", CHEAP_PARTICLE_FRAGMENT)]);
    }

    /// The flare program, which Reach's light volumes' disc and depth fade
    /// share, and Reach's composite with its gamma and noise, as the
    /// particle program is checked.
    #[test]
    fn the_flare_and_reach_composite_programs_compile_under_glslang() {
        assert_compiles_under_glslang("flare", &[("vert", CE_FLARE_VERTEX), ("frag", CE_FLARE_FRAGMENT)]);
        assert_compiles_under_glslang("reach_composite", &[("frag", super::reach::lighting::REACH_COMPOSITE_FRAGMENT)]);
    }

    /// Each `(stage, source)` of `name` through `glslangValidator`; without
    /// it, skip by name.
    fn assert_compiles_under_glslang(name: &str, stages: &[(&str, &str)]) {
        if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
            eprintln!("skipping: glslangValidator is not on PATH (brew install glslang)");
            return;
        }
        let dir = crate::core::test_kits::unique_temp_dir(&format!("model_preview_{name}_glsl"));
        for (stage, source) in stages {
            let path = dir.join(format!("{name}.{stage}"));
            std::fs::write(&path, source).expect("write shader");
            let output = std::process::Command::new("glslangValidator").arg(&path).output().expect("run glslangValidator");
            let printed = String::from_utf8_lossy(&output.stdout);
            assert!(output.status.success(), "{name} {stage} does not compile:\n{printed}");
        }
    }

    #[test]
    fn both_shader_dialects_declare_what_the_renderer_binds() {
        for (declaration, modern, precision) in [
            ("#version 330\n", true, ""),
            ("#version 100\n", false, "precision mediump float;\n"),
        ] {
            let (vertex, fragment) = model_shader_sources(declaration, modern, precision);

            // Every attribute the vertex array points at, and every uniform the
            // renderer looks up, has to actually be declared.
            for name in [
                "a_position",
                "a_normal",
                "a_texcoord",
                "a_tangent",
                "a_binormal",
                "a_node_indices",
                "a_node_weights",
            ] {
                assert!(vertex.contains(name), "{name} missing from vertex shader");
            }
            for name in ["u_animated", "u_bones"] {
                assert!(vertex.contains(name), "{name} missing from vertex shader");
            }
            for name in SAMPLER_UNIFORMS {
                assert!(
                    fragment.contains(name),
                    "{name} missing from fragment shader"
                );
            }
            for name in [
                "u_have_a",
                "u_have_b",
                "u_uv_scale_a",
                "u_uv_scale_b",
                "u_shaded",
            ] {
                assert!(
                    fragment.contains(name),
                    "{name} missing from fragment shader"
                );
            }
            assert!(fragment.contains("uniform vec4 u_have_b;"));
            assert!(fragment.contains("uniform vec4 u_uv_scale_b;"));
            // The environment term must stay behind the ambient-strength gate: a
            // shader that asks for none of it has to light exactly as it did
            // before the term existed.

            // A surface with no specular mask must reflect LESS, not more.
            // Having that backwards buried dervish's bare skin — which carries
            // no mask — under a flat wash of environment tint.

            // The detail normal adds to the base one rather than replacing it.
            assert!(
                fragment.contains("tn = vec3(tn.xy + dn, tn.z);"),
                "bump_detail_map should blend into the base normal"
            );
            assert!(
                fragment.contains("u_have_a.z > 0.5 || u_have_a.w > 0.5"),
                "a detail normal with no base bump map should still perturb"
            );
            // The unpack is the engine's own, verbatim from the kit's
            // rasterizer/hlsl/bump_mapping.fx: BUMP_CONVERT on X and Y (byte
            // 128 = exactly flat) for BOTH maps, Z always reconstructed, and
            // NO channel negation — the authored tangent frame carries the
            // convention. `*2-1` unpacks and green flips have both been tried
            // and both bend the lighting; the engine source is the authority.
            assert_eq!(
                fragment
                    .matches("* (255.0 / 127.0) - (128.0 / 127.0)")
                    .count(),
                2,
                "base and detail bumps must both unpack through BUMP_CONVERT"
            );
            assert!(
                fragment.contains("sqrt(1.0 - min(dot(sampled, sampled), 1.0))"),
                "Z must be reconstructed from X/Y, never sampled"
            );
            assert!(
                !fragment.contains("tn.y = -tn.y"),
                "no green flip: the engine feeds sampled Y straight into the tangent frame"
            );
            for name in [
                "u_center",
                "u_scale",
                "u_angles",
                "u_clip_scale",
                "u_depth_scale",
                "u_depth_map",
                "u_perspective",
            ] {
                assert!(vertex.contains(name), "{name} missing from vertex shader");
            }
            for name in ["u_base_color", "u_unlit"] {
                assert!(
                    fragment.contains(name),
                    "{name} missing from fragment shader"
                );
            }

            // The varyings must be declared on both sides or the link fails.
            for name in ["v_uv", "v_normal", "v_tangent", "v_binormal"] {
                assert!(
                    vertex.contains(name) && fragment.contains(name),
                    "{name} not on both sides"
                );
            }

            // Dialect: `texture` vs `texture2D`, and the output keyword pair.
            if modern {
                assert!(fragment.contains("out vec4 out_color;"));
                assert!(fragment.contains("texture(u_tex_base"));
                assert!(vertex.contains("in vec3 a_position"));
            } else {
                assert!(fragment.contains("gl_FragColor"));
                assert!(fragment.contains("texture2D(u_tex_base"));
                assert!(vertex.contains("attribute vec3 a_position"));
            }

            // Only the alpha-test map may discard.
            //
            // A base map's alpha channel carries a mask in Halo — usually
            // specular — not coverage. Discarding on it made dervish, and every
            // other character with a dark diffuse mask, render mostly
            // see-through, while masterchief happened to look fine because his
            // mask is bright. One `discard`, in the alpha-test branch.
            assert_eq!(
                fragment.matches("discard;").count(),
                1,
                "the fragment shader should discard only on the alpha-test map"
            );
            assert!(
                fragment.contains("u_tex_alpha, v_uv * u_uv_scale_c.xy).a < 0.5) discard"),
                "the one discard should be the alpha-test map's"
            );

            for (label, source) in [("vertex", &vertex), ("fragment", &fragment)] {
                assert_eq!(
                    source.matches('{').count(),
                    source.matches('}').count(),
                    "unbalanced braces in the {label} shader — a `format!` escape slipped"
                );
                assert!(
                    source.starts_with(declaration),
                    "the {label} shader must open with its version declaration"
                );
                assert!(
                    !source.contains("{{") && !source.contains("}}"),
                    "a `format!` brace escape survived into the {label} source"
                );
            }
        }
    }

    #[test]
    fn gpu_vertex_layout_matches_the_hand_written_attribute_offsets() {
        assert_eq!(std::mem::size_of::<RenderModelPreviewVertex>(), 88);
        let vertex = RenderModelPreviewVertex::default();
        let base = std::ptr::addr_of!(vertex) as usize;
        let offset = |field: usize| field - base;
        assert_eq!(offset(std::ptr::addr_of!(vertex.normal) as usize), 12);
        assert_eq!(offset(std::ptr::addr_of!(vertex.texcoord) as usize), 24);
        assert_eq!(offset(std::ptr::addr_of!(vertex.tangent) as usize), 32);
        assert_eq!(offset(std::ptr::addr_of!(vertex.binormal) as usize), 44);
        assert_eq!(offset(std::ptr::addr_of!(vertex.node_indices) as usize), 56);
        assert_eq!(offset(std::ptr::addr_of!(vertex.node_weights) as usize), 72);
    }

    #[test]
    fn indexed_preview_keeps_shared_vertices_and_part_batches() {
        let vertex = |x, y| RenderVertex {
            position: RealPoint3d { x, y, z: 0.0 },
            texcoord: RealPoint2d { x, y },
            normal: RealVector3d {
                i: 0.0,
                j: 0.0,
                k: 1.0,
            },
            tangent: RealVector3d::ZERO,
            binormal: RealVector3d::ZERO,
            node_indices: [0; 4],
            node_weights: [0.0; 4],
            lightmap_texcoord: RealPoint2d { x: 0.0, y: 0.0 },
            vert_color: RealVector3d::ZERO,
        };
        let part = |index_start| RenderMeshPart {
            material_index: index_start as u16 / 3,
            index_start,
            index_count: 3,
            part_type: GeometryPartType::OpaqueNonShadowing,
            transparent_sorting_index: -1,
            sort_position: None,
        };
        let mesh = RenderMesh {
            vertices: vec![
                vertex(0.0, 0.0),
                vertex(1.0, 0.0),
                vertex(1.0, 1.0),
                vertex(0.0, 1.0),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            parts: vec![part(0), part(3)],
            subparts: Vec::new(),
            rigid_node_index: None,
            water_data: None,
            prt_vertex_type: Default::default(),
            has_prt_vertex_stream: false,
            prt_ambient_stream: Vec::new(),
            has_vertex_color: false,
            use_region_index_for_sorting: false,
            has_lightmap_uvs: false,
        };
        let mut preview = RenderModelPreview {
            bounds_min: [f32::INFINITY; 3],
            bounds_max: [f32::NEG_INFINITY; 3],
            ..Default::default()
        };

        append_render_mesh_to_preview(&mut preview, &mesh, "body", "default");

        assert_eq!(preview.vertices.len(), 4);
        assert_eq!(preview.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(preview.batches.len(), 2);
        assert_eq!(
            (
                preview.batches[0].index_start,
                preview.batches[0].index_count
            ),
            (0, 3)
        );
        assert_eq!(
            (
                preview.batches[1].index_start,
                preview.batches[1].index_count
            ),
            (3, 3)
        );
    }

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
    fn error_visibility_follows_its_owning_model_layer() {
        let mut state = ModelPreviewState {
            overlays_loaded: true,
            show_render: true,
            show_collision: false,
            show_physics: true,
            ..Default::default()
        };
        assert!(model_layer_visible(&state, ModelPreviewLayer::Render));
        assert!(!model_layer_visible(&state, ModelPreviewLayer::Collision));
        assert!(model_layer_visible(&state, ModelPreviewLayer::Physics));

        state.show_render = false;
        assert!(!model_layer_visible(&state, ModelPreviewLayer::Render));
        assert!(model_layer_visible(&state, ModelPreviewLayer::Physics));
    }

    #[test]
    fn non_critical_errors_are_opt_in() {
        let mut state = ModelPreviewState::default();
        assert!(!state.show_non_critical_errors);
        state.show_non_critical_errors = true;
        assert!(state.show_non_critical_errors);
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

pub(super) const MARKER_AXIS_SCREEN_LENGTH: f32 = 15.0;

fn marker_axis_screen_deltas(camera: &PreviewCamera, axes: [[f32; 3]; 3]) -> [Vec2; 3] {
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

pub(super) fn draw_marker_axes(
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

pub(super) fn marker_axes_hovered(
    pos: egui::Pos2,
    origin: egui::Pos2,
    axis_deltas: [Vec2; 3],
) -> bool {
    screen_edge_length(pos, origin) <= 7.0
        || axis_deltas
            .into_iter()
            .any(|delta| point_segment_distance(pos, origin, origin + delta) <= 5.0)
}

pub(super) fn point_segment_distance(point: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let ap = point - a;
    let denom = ab.dot(ab);
    if denom <= f32::EPSILON {
        return screen_edge_length(point, a);
    }
    let t = (ap.dot(ab) / denom).clamp(0.0, 1.0);
    screen_edge_length(point, a + ab * t)
}

pub(super) fn screen_edge_length(a: egui::Pos2, b: egui::Pos2) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

pub(in crate::app) fn material_color(index: u16) -> Color32 {
    const COLORS: &[(u8, u8, u8)] = &[
        (132, 168, 188),
        (176, 166, 128),
        (142, 182, 150),
        (180, 136, 134),
        (150, 145, 190),
        (186, 154, 104),
        (126, 174, 176),
    ];
    let (r, g, b) = COLORS[index as usize % COLORS.len()];
    Color32::from_rgb(r, g, b)
}

struct ProjectedPoint {
    pos: egui::Pos2,
    /// In front of the perspective near plane; always true orthographically.
    in_front: bool,
}

/// The camera's zoom bounds. Wide on purpose: 5× was plenty for a vehicle but
/// nothing on a structure BSP, where inspecting a doorway on a whole level
/// needs a couple hundred times the fitted view.
/// How far the view pulls back: half its shorter side, in world units. The
/// same for every model, whose own size only sets where zoom 1× is.
const WIDEST_PREVIEW_VIEW: f32 = 10_000.0;
pub(in crate::app) const MAX_PREVIEW_SCALE: f32 = 500.0;

/// A preview's bounds center and bounding-sphere radius — the fit every
/// camera computation derives from.
pub(in crate::app) fn min_preview_scale(preview: &RenderModelPreview) -> f32 {
    let (_, radius) = preview_center_radius(preview);
    (radius * 1.1 / WIDEST_PREVIEW_VIEW).min(1.0)
}

fn preview_center_radius(preview: &RenderModelPreview) -> ([f32; 3], f32) {
    let (min, max) = (preview.bounds_min, preview.bounds_max);
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let extent = [
        (max[0] - min[0]).abs(),
        (max[1] - min[1]).abs(),
        (max[2] - min[2]).abs(),
    ];
    let radius = ((extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt()
        * 0.5)
        .max(0.001);
    (center, radius)
}

/// World → view: yaw about Z, then pitch about X — the one rotation this
/// preview uses, mirrored in the vertex shader's `rotate_view`.
pub(in crate::app::model_preview) fn rotate_view_vector(yaw: f32, pitch: f32, vector: [f32; 3]) -> [f32; 3] {
    let (sy, cy) = yaw.sin_cos();
    let x = vector[0] * cy - vector[1] * sy;
    let y = vector[0] * sy + vector[1] * cy;
    let (sp, cp) = pitch.sin_cos();
    [x, y * cp - vector[2] * sp, y * sp + vector[2] * cp]
}

/// View → world: the exact inverse of [`rotate_view_vector`] (un-pitch, then
/// un-yaw). What turns a screen-space pan or zoom-to-cursor offset back into
/// the world-space focus move it stands for.
pub(super) fn unrotate_view_vector(yaw: f32, pitch: f32, vector: [f32; 3]) -> [f32; 3] {
    let (sp, cp) = pitch.sin_cos();
    let y = vector[1] * cp + vector[2] * sp;
    let z = -vector[1] * sp + vector[2] * cp;
    let (sy, cy) = yaw.sin_cos();
    [vector[0] * cy + y * sy, -vector[0] * sy + y * cy, z]
}

struct PreviewCamera {
    rect: egui::Rect,
    /// Orbit point: bounds center plus the user's world-space focus offset.
    center: [f32; 3],
    /// Fit radius — deliberately the *bounds* radius, unaffected by focus, so
    /// the zoom slider keeps one meaning wherever the user pans.
    radius: f32,
    /// Depth-normalization radius: the bounds radius grown by the focus
    /// offset, so panned-to geometry can never fall outside the depth range.
    depth_radius: f32,
    yaw: f32,
    pitch: f32,
    scale: f32,
    perspective: bool,
}

impl PreviewCamera {
    fn new(data: &ModelPreviewData, state: &ModelPreviewState, rect: egui::Rect) -> Self {
        let (bounds_center, radius) = preview_center_radius(&data.preview);
        let center = [
            bounds_center[0] + state.focus[0],
            bounds_center[1] + state.focus[1],
            bounds_center[2] + state.focus[2],
        ];
        let focus_length = (state.focus[0] * state.focus[0]
            + state.focus[1] * state.focus[1]
            + state.focus[2] * state.focus[2])
            .sqrt();
        // An animation can carry the model well outside its bind-pose bounds
        // (boarding and assassination clips travel several world units), and
        // geometry past the depth window vanishes wholesale — sliced away
        // plane-first as the pose travels. Grow the window by the pose's own
        // reach; the cost is depth precision this preview has to spare.
        let reach = state
            .animation
            .pose
            .as_deref()
            .map(|pose| pose.reach)
            .unwrap_or(0.0);
        let scale = state.scale.clamp(min_preview_scale(&data.preview), MAX_PREVIEW_SCALE);
        Self {
            rect,
            center,
            radius,
            // What the view spans must draw however small the model is.
            depth_radius: (radius + focus_length + reach).max(radius * 1.1 / scale),
            yaw: state.yaw,
            pitch: state.pitch,
            scale,
            perspective: state.perspective,
        }
    }

    fn project(&self, point: [f32; 3]) -> ProjectedPoint {
        let x = (point[0] - self.center[0]) * self.scale;
        let y = (point[1] - self.center[1]) * self.scale;
        let z = (point[2] - self.center[2]) * self.scale;
        let rotated = self.rotate_vector([x, y, z]);
        let fit = self.rect.width().min(self.rect.height()) / (self.radius * 2.2).max(0.001);
        // The same divide the vertex shader applies, so markers stay glued to
        // their geometry in either projection.
        let w = 1.0 + rotated[1] * self.inverse_eye();
        // The GPU clips at the near plane; an overlay point past it is behind
        // (or level with) the eye and has no place on screen.
        let in_front = w >= PERSPECTIVE_NEAR_FRACTION;
        let w = w.max(PERSPECTIVE_NEAR_FRACTION);
        let screen = self.rect.center() + Vec2::new(rotated[0] * fit / w, -rotated[2] * fit / w);
        ProjectedPoint {
            pos: screen,
            in_front,
        }
    }

    /// Eye distance from the orbit point in scaled view units; see
    /// [`PERSPECTIVE_FOV_Y`].
    fn eye_distance(&self) -> f32 {
        (self.radius * 1.1).max(0.001) / (PERSPECTIVE_FOV_Y * 0.5).tan()
    }

    /// `1 / eye` for perspective, 0 for the orthographic view.
    fn inverse_eye(&self) -> f32 {
        if self.perspective {
            1.0 / self.eye_distance()
        } else {
            0.0
        }
    }

    fn gpu_uniforms(&self) -> ModelGpuCamera {
        let width = self.rect.width().max(1.0);
        let height = self.rect.height().max(1.0);
        let fit = width.min(height) / (self.radius * 2.2).max(0.001);
        ModelGpuCamera {
            center: self.center,
            scale: self.scale,
            yaw: self.yaw,
            pitch: self.pitch,
            clip_scale: [2.0 * fit / width, 2.0 * fit / height],
            // Orthographic projection: smaller view-space Y is nearer. The
            // focus-grown radius safely maps all depths inside NDC even when
            // the orbit point sits far from the bounds center.
            depth_scale: 1.0 / (self.depth_radius * self.scale * 1.05).max(0.001),
            // Depth remap `z = t·x + y` with `t = view.y · depth_scale`;
            // see `depth_map_coefficients`.
            depth_map: depth_map_coefficients(
                self.perspective.then(|| {
                    self.eye_distance() / (self.depth_radius * self.scale * 1.05).max(0.001)
                }),
            ),
            perspective: self.inverse_eye(),
        }
    }

    fn rotate_vector(&self, vector: [f32; 3]) -> [f32; 3] {
        rotate_view_vector(self.yaw, self.pitch, vector)
    }
}

/// The `[x, y]` of the shader's depth remap `z = t·x + y`, with `t = view.y ·
/// depth_scale` (documented at the `depth_map` field of `ModelGpuCamera`).
///
/// `t` is depth in units of the depth window, and the far plane sits at
/// `t = K` (`PREVIEW_RENDER_DISTANCE`) in both projections. Orthographic maps
/// `[-K, K]` straight onto NDC. Perspective takes `eye` — the eye's distance,
/// in the same units — and solves the line through `z(n) = -w(n)` and
/// `z(K) = w(K)`, with `w(t) = 1 + t / eye` and the near plane `n` just in
/// front of the eye. `z / w` is monotone in `t` between them, so the depth
/// test orders correctly however close the dolly brings the eye.
fn depth_map_coefficients(eye: Option<f32>) -> [f32; 2] {
    let k = PREVIEW_RENDER_DISTANCE;
    let Some(eye) = eye.filter(|eye| eye.is_finite() && *eye > 0.0) else {
        return [1.0 / k, 0.0];
    };
    let near = -eye * (1.0 - PERSPECTIVE_NEAR_FRACTION);
    let w = |t: f32| 1.0 + t / eye;
    let x = (w(k) + w(near)) / (k - near);
    [x, -w(near) - x * near]
}

/// Build flat preview geometry with draw batches grouped by region and
/// permutation. Ported from blam-tags so the GUI owns its preview type; the
/// render meshes are derived separately via `RenderModel::derive_render_meshes`.
pub(super) fn render_model_to_preview(
    model: &RenderModel,
    render_meshes: &[RenderMesh],
) -> RenderModelPreview {
    let node_world = preview_node_world_transforms(&model.nodes);
    let mut preview = RenderModelPreview {
        regions: model
            .regions
            .iter()
            .map(|region| RenderModelPreviewRegion {
                name: region.name.clone(),
                permutations: region
                    .permutations
                    .iter()
                    .map(|permutation| permutation.name.clone())
                    .collect(),
            })
            .collect(),
        // Index-aligned with `RenderMeshPart::material_index`, which is what
        // the batches carry, so a batch's shader is `materials[index]`.
        materials: model
            .materials
            .iter()
            .map(|material| RenderModelPreviewMaterial {
                shader_path: material.render_method.clone(),
                shader_group: material.render_method_group,
                overlay: false, shield: None,
                permutation: 0,
                object: 0,
            })
            .collect(),
        nodes: preview_skeleton_nodes(&model.nodes),
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };

    for (region, permutation, mesh_index) in preview_mesh_order(model, render_meshes.len()) {
        let (region, permutation) = (&model.regions[region], &model.regions[region].permutations[permutation]);
        append_render_mesh_to_preview(&mut preview, &render_meshes[mesh_index], &region.name, &permutation.name);
    }

    append_instances_to_preview(&mut preview, model, render_meshes);

    if preview.vertices.is_empty() {
        preview.bounds_min = [0.0; 3];
        preview.bounds_max = [0.0; 3];
    }

    for group in &model.marker_groups {
        for marker in &group.markers {
            preview.markers.push(RenderModelPreviewMarker {
                name: group.name.clone(),
                node_index: marker.node_index as i16,
                position: transform_preview_marker_position(marker, &node_world),
                scale: marker.scale,
                axes: transform_preview_marker_axes(marker, &node_world),
                only_under: marker_permutation(model, marker),
            });
        }
    }

    preview
}

fn marker_permutation(model: &RenderModel, marker: &blam_tags::render_model::Marker) -> Option<(String, String)> {
    let region = model.regions.get(usize::try_from(marker.region_index).ok()?)?;
    let permutation = region.permutations.get(usize::try_from(marker.permutation_index).ok()?)?;
    Some((region.name.clone(), permutation.name.clone()))
}

/// The meshes a preview holds, in the order its vertices lie: by region and
/// permutation, which is not the tag's. Anything parallel to the vertices has
/// to walk this too. Each is a region, permutation and mesh index.
pub(super) fn preview_mesh_order(model: &RenderModel, mesh_count: usize) -> Vec<(usize, usize, usize)> {
    let mut order = Vec::new();
    for (region_index, region) in model.regions.iter().enumerate() {
        for (permutation_index, permutation) in region.permutations.iter().enumerate() {
            // NONE, whatever its count says: a permutation with no geometry.
            let Ok(first_mesh) = usize::try_from(permutation.mesh_index) else {
                continue;
            };
            let count = permutation.mesh_count.max(0) as usize;
            for mesh_index in first_mesh..first_mesh.saturating_add(count) {
                if mesh_index < mesh_count {
                    order.push((region_index, permutation_index, mesh_index));
                }
            }
        }
    }
    order
}

/// `render_object_context_instances`: each placement draws its subpart of the
/// instance mesh, carried from there into the model by its own transform and
/// riding its node from then on. A mirrored one winds backward.
fn append_instances_to_preview(
    preview: &mut RenderModelPreview,
    model: &RenderModel,
    render_meshes: &[RenderMesh],
) {
    let instances = &mut preview.instances;
    instances.empty_regions = model
        .regions
        .iter()
        .filter(|region| region.permutations.iter().all(|permutation| permutation.mesh_index < 0))
        .map(|region| region.name.clone())
        .collect();
    for region in &model.regions {
        for permutation in &region.permutations {
            let mask = permutation.instance_mask.iter().rev().fold(0u128, |mask, word| mask << 32 | *word as u128);
            instances.masks.insert((region.name.clone(), permutation.name.clone()), mask);
        }
    }
    instances.names = model.instance_placements.iter().map(|placement| placement.name.clone()).collect();
    let Some(mesh) = model.instance_mesh_index.and_then(|mesh| render_meshes.get(mesh)) else {
        return;
    };
    let first_vertex = preview.vertices.len();
    for (instance, (placement, subpart)) in model.instance_placements.iter().zip(&mesh.subparts).enumerate() {
        let (Some(part), Ok(node)) = (mesh.parts.get(subpart.part_index as usize), usize::try_from(placement.node_index)) else {
            continue;
        };
        let start = subpart.index_start as usize;
        let Some(source) = mesh.indices.get(start..start + subpart.index_count as usize) else {
            continue;
        };
        let scale = placement.scale;
        let axes = [placement.forward, placement.left, placement.up].map(vector3_to_array);
        let turned = |[x, y, z]: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|axis| axes[0][axis] * x + axes[1][axis] * y + axes[2][axis] * z)
        };
        let direction = |vector: [f32; 3]| turned(vector).map(|value| value * scale.signum());
        let origin = point3_to_array(placement.position);
        // A triangle naming a vertex the mesh lacks is dropped whole, so the
        // triangles after it keep their corners.
        let triangles: Vec<&[u32]> = source
            .chunks_exact(3)
            .filter(|corners| corners.iter().all(|&index| (index as usize) < mesh.vertices.len()))
            .collect();
        // Each vertex the triangles name is added once.
        let adding = triangles.iter().copied().flatten().collect::<std::collections::HashSet<_>>().len();
        let Some(vertex_base) =
            common::geometry::VertexBase::of(preview.vertices.len(), adding, common::geometry::Overflow::RefusesAppend)
        else {
            continue;
        };
        let mut remapped = HashMap::new();
        let mut indices = Vec::with_capacity(source.len());
        for &index in triangles.into_iter().flatten() {
            let next = vertex_base.offset(remapped.len() as u32);
            let at = *remapped.entry(index).or_insert(next);
            if at == next {
                let vertex = &mesh.vertices[index as usize];
                let placed = turned(point3_to_array(vertex.position));
                let position = std::array::from_fn(|axis| placed[axis] * scale + origin[axis]);
                expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
                preview.vertices.push(RenderModelPreviewVertex {
                    position,
                    normal: direction(vector3_to_array(vertex.normal)),
                    texcoord: [vertex.texcoord.x, vertex.texcoord.y],
                    tangent: direction(vector3_to_array(vertex.tangent)),
                    binormal: direction(vector3_to_array(vertex.binormal)),
                    node_indices: [node as f32, 0.0, 0.0, 0.0],
                    node_weights: [1.0, 0.0, 0.0, 0.0],
                });
            }
            indices.push(at);
        }
        if scale < 0.0 {
            indices.chunks_exact_mut(3).for_each(|triangle| triangle.swap(1, 2));
        }
        let index_start = preview.indices.len() as u32;
        preview.indices.extend_from_slice(&indices);
        instances_batch(preview, instance, part.material_index, index_start, indices.len() as u32);
    }
    preview.instances.vertex_count = preview.vertices.len() - first_vertex;
}

fn instances_batch(
    preview: &mut RenderModelPreview,
    instance: usize,
    material_index: u16,
    index_start: u32,
    index_count: u32,
) {
    preview.instances.batches.insert(preview.batches.len(), instance);
    preview.batches.push(RenderModelPreviewBatch {
        region_name: String::new(),
        permutation_name: String::new(),
        material_index,
        index_start,
        index_count,
        flat_color: None,
        layer: ModelPreviewLayer::Render,
        sort_position: None,
    });
}

/// Append one render mesh without flattening every triangle into three unique
/// vertices. The GPU renderer consumes indexed `u32` geometry directly, so
/// retaining the source topology is both faithful and dramatically smaller for
/// dense Campaign Evolved Nanite previews.
fn append_render_mesh_to_preview(
    preview: &mut RenderModelPreview,
    mesh: &RenderMesh,
    region_name: &str,
    permutation_name: &str,
) {
    let Some(vertex_base) =
        common::geometry::VertexBase::of(preview.vertices.len(), mesh.vertices.len(), common::geometry::Overflow::DropsTriangle)
    else {
        return;
    };

    preview.vertices.reserve(mesh.vertices.len());
    for vertex in &mesh.vertices {
        let position = point3_to_array(vertex.position);
        expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
        preview.vertices.push(RenderModelPreviewVertex {
            position,
            normal: vector3_to_array(vertex.normal),
            // Already decompressed: `derive_render_meshes` applies the mesh's
            // compression bounds, so these are real UVs rather than the packed
            // [0,1] the buffer stores for the UShort2N formats.
            texcoord: [vertex.texcoord.x, vertex.texcoord.y],
            tangent: vector3_to_array(vertex.tangent),
            binormal: vector3_to_array(vertex.binormal),
            // Global skeleton indices — blam-tags resolves the per-mesh
            // palettes, and rigid meshes arrive as a single full-weight
            // influence, so animation needs no rigid special case.
            node_indices: vertex.node_indices.map(|index| index as f32),
            node_weights: vertex.node_weights,
        });
    }

    for part in &mesh.parts {
        let source_start = part.index_start as usize;
        let source_end = source_start
            .saturating_add(part.index_count as usize)
            .min(mesh.indices.len());
        let Some(source_indices) = mesh.indices.get(source_start..source_end) else {
            continue;
        };
        let Ok(index_start) = u32::try_from(preview.indices.len()) else {
            continue;
        };
        for triangle in source_indices.chunks_exact(3) {
            let convert = |index: u32| {
                ((index as usize) < mesh.vertices.len())
                    .then(|| vertex_base.checked_offset(index))
                    .flatten()
            };
            if let (Some(a), Some(b), Some(c)) = (
                convert(triangle[0]),
                convert(triangle[1]),
                convert(triangle[2]),
            ) {
                preview.indices.extend_from_slice(&[a, b, c]);
            }
        }
        let Ok(index_end) = u32::try_from(preview.indices.len()) else {
            continue;
        };
        let index_count = index_end - index_start;
        if index_count > 0 {
            let sort_position =
                part.sort_position.map(|at| (point3_to_array(at.position), usize::from(at.node_indices[0])));
            preview.batches.push(RenderModelPreviewBatch {
                region_name: region_name.to_owned(),
                permutation_name: permutation_name.to_owned(),
                material_index: part.material_index,
                index_start,
                index_count,
                flat_color: None,
                layer: ModelPreviewLayer::Render,
                sort_position,
            });
        }
    }
}

/// The preview's copy of the skeleton: parent-local bind pose plus the
/// inverse of the accumulated bind world transform, ready for skinning.
pub(super) fn preview_skeleton_nodes(nodes: &[Node]) -> Vec<RenderModelPreviewNode> {
    let world = preview_node_world_transforms(nodes);
    nodes
        .iter()
        .zip(&world)
        .map(|(node, (world_rot, world_trans))| {
            let bind_world =
                blam_tags::math::Matrix4::from_loc_rot_scale(*world_trans, *world_rot, 1.0);
            let inverse = bind_world.inverse();
            RenderModelPreviewNode {
                name: node.name.clone(),
                parent: node.parent_node,
                bind_rotation: node.default_rotation.normalized().to_array(),
                bind_translation: point3_to_array(node.default_translation),
                inverse_bind: [inverse.m[0], inverse.m[1], inverse.m[2]],
            }
        })
        .collect()
}

pub(super) fn preview_node_world_transforms(nodes: &[Node]) -> Vec<(RealQuaternion, RealPoint3d)> {
    let mut world: Vec<(RealQuaternion, RealPoint3d)> = Vec::with_capacity(nodes.len());
    for node in nodes {
        let local_rot = node.default_rotation.normalized();
        let local_trans = node.default_translation;
        if node.parent_node >= 0
            && let Some((parent_rot, parent_trans)) = world.get(node.parent_node as usize).copied()
        {
            let rot = (parent_rot * local_rot).normalized();
            let trans = parent_trans + (parent_rot * local_trans.as_vector());
            world.push((rot, trans));
            continue;
        }
        world.push((local_rot, local_trans));
    }
    world
}

pub(super) fn transform_preview_marker_position(
    marker: &Marker,
    node_world: &[(RealQuaternion, RealPoint3d)],
) -> [f32; 3] {
    let local = marker.translation;
    let world = if marker.node_index >= 0 {
        node_world
            .get(marker.node_index as usize)
            .map(|(rot, trans)| *trans + (*rot * local.as_vector()))
            .unwrap_or(local)
    } else {
        local
    };
    point3_to_array(world)
}

pub(super) fn transform_preview_marker_axes(
    marker: &Marker,
    node_world: &[(RealQuaternion, RealPoint3d)],
) -> [[f32; 3]; 3] {
    let local_rot = marker.rotation.normalized();
    let world_rot = if marker.node_index >= 0 {
        node_world
            .get(marker.node_index as usize)
            .map(|(rot, _)| (*rot * local_rot).normalized())
            .unwrap_or(local_rot)
    } else {
        local_rot
    };
    [
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 1.0,
                    j: 0.0,
                    k: 0.0,
                },
        ),
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 0.0,
                    j: 1.0,
                    k: 0.0,
                },
        ),
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 0.0,
                    j: 0.0,
                    k: 1.0,
                },
        ),
    ]
}

pub(super) fn point3_to_array(p: RealPoint3d) -> [f32; 3] {
    [p.x, p.y, p.z]
}

pub(super) fn vector3_to_array(v: RealVector3d) -> [f32; 3] {
    [v.i, v.j, v.k]
}

/// The texture bound to a slot whose material has none: one opaque white
/// texel, never sampled for its value (the slot's `have` flag is zero), only
/// there so the unit holds a complete texture. Complete means a non-mipmap
/// minification filter. GL's default filter expects mipmaps, and with one
/// level it would leave this texture as incomplete as texture 0.
unsafe fn upload_placeholder_texture(gl: &glow::Context) -> Option<glow::NativeTexture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            1,
            1,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&[255, 255, 255, 255])),
        );
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(texture)
    }
}

/// Halo 2's, 3's and Reach's antennas, whose maps follow Halo CE's in the
/// slot's, each with whether it is Reach's; Reach's far ends are at `tips`.
/// Those on a permutation not shown are left out.
pub(in crate::app::model_preview) fn h2_antennas(
    frame: &ModelGpuFrame,
    textures: &[Option<glow::NativeTexture>],
    bones: Option<&[[f32; 4]]>,
    view: &common::view::ViewFrame,
    tips: &[Option<[f32; 3]>],
) -> Vec<(Vec<antennas::AntennaQuad>, glow::NativeTexture, bool)> {
    let preview = &frame.preview;
    preview
        .antennas
        .iter()
        .zip(textures)
        .enumerate()
        .filter(|(index, _)| is_shown(&frame.h2_shown.antennas, *index))
        .filter_map(|(index, (antenna, texture))| {
            let tip = tips.get(index).copied().flatten();
            Some((antenna.quads(bones, view, tip), (*texture)?, antenna.is_reach()))
        })
        .collect()
}

/// Whether the preview has any of Halo CE's effects, particle systems, glows,
/// contrails, lightning, antennas or light volumes.
pub(in crate::app::model_preview) fn ce_has_groups(preview: &RenderModelPreview) -> bool {
    !(preview.ce.effects.is_empty()
        && preview.ce.particle_systems.is_empty()
        && preview.ce.glows.is_empty()
        && preview.ce.contrails.is_empty()
        && preview.ce.lightning.is_empty()
        && preview.ce.antennas.is_empty()
        && preview.ce.light_volumes.is_empty())
}

/// Whether Halo CE's effects sort in among the model's transparent parts, as
/// the game draws them, rather than after: a CE model's batches know where
/// they sort, and the transparents draw only when shading does.
pub(in crate::app::model_preview) fn ce_sorts_transparents(frame: &ModelGpuFrame) -> bool {
    frame.render_mode.draws_shading() && frame.preview.batches.iter().any(|batch| batch.sort_position.is_some())
}

pub(super) fn ce_flare_view(camera: &ModelGpuCamera) -> common::view::ViewFrame {
    let along = |vector| unrotate_view_vector(camera.yaw, camera.pitch, vector);
    common::view::ViewFrame {
        eye: h2_eye_position(camera),
        forward: along([0.0, 1.0, 0.0]),
        right: along([1.0, 0.0, 0.0]),
        up: along([0.0, 0.0, 1.0]),
    }
}

/// The vertex shader's `clip_position`.
pub(super) fn ce_flare_clip(camera: &ModelGpuCamera, point: [f32; 3]) -> [f32; 4] {
    let scaled = std::array::from_fn(|axis| (point[axis] - camera.center[axis]) * camera.scale);
    let view = rotate_view_vector(camera.yaw, camera.pitch, scaled);
    [
        view[0] * camera.clip_scale[0],
        view[2] * camera.clip_scale[1],
        view[1] * camera.depth_scale * camera.depth_map[0] + camera.depth_map[1],
        1.0 + view[1] * camera.perspective,
    ]
}

/// How many pixels wide plus high a screen-facing sprite of `half` extents
/// is at `center`: `lens_flare_project_billboard`'s half extents across the
/// viewport, as Reach's `lens_flare_render_internal` measures them.
pub(in crate::app::model_preview) fn ce_flare_pixels(camera: &ModelGpuCamera, viewport: [i32; 2], center: [f32; 3], half: [f32; 2]) -> f32 {
    let w = ce_flare_clip(camera, center)[3];
    if w <= 0.0 {
        return 0.0;
    }
    let across = half[0] * camera.scale * camera.clip_scale[0] / w * viewport[0] as f32;
    let down = half[1] * camera.scale * camera.clip_scale[1] / w * viewport[1] as f32;
    across + down
}

/// A sprite's four corners, all at the depth of its center.
fn ce_flare_quad(
    camera: &ModelGpuCamera,
    view: &common::view::ViewFrame,
    sprite: &ce::lights::FlareSprite,
) -> [[f32; 6]; 4] {
    let (sine, cosine) = sprite.rotation.sin_cos();
    let center = sprite.center;
    let depth = ce_flare_clip(camera, center);
    [[0.0, 0.0, -1.0, -1.0], [1.0, 0.0, 1.0, -1.0], [0.0, 1.0, -1.0, 1.0], [1.0, 1.0, 1.0, 1.0]].map(
        |[u, v, x, y]| {
            let across = x * sprite.extent[0];
            let upward = y * sprite.extent[1];
            let (right, up) = (across * cosine - upward * sine, across * sine + upward * cosine);
            let corner = std::array::from_fn(|axis| {
                center[axis] + view.right[axis] * right + view.up[axis] * up
            });
            let clip = ce_flare_clip(camera, corner);
            let scale = depth[3] / clip[3];
            [clip[0] * scale, clip[1] * scale, depth[2], depth[3], u, 1.0 - v]
        },
    )
}

/// How much of `patch` the model leaves showing, if any of it is on screen.
pub(in crate::app::model_preview) unsafe fn ce_flare_occlusion(
    gl: &glow::Context,
    flares: &SpriteGl,
    camera: &ModelGpuCamera,
    view: &common::view::ViewFrame,
    patch: &ce::lights::FlareSprite,
) -> Option<f32> {
    unsafe {
        let vertices = ce_flare_quad(camera, view, patch);
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(&vertices), glow::STREAM_DRAW);
        gl.enable(glow::DEPTH_TEST);
        gl.color_mask(false, false, false, false);
        let mut samples = [0.0; 2];
        for (sample, (query, compare)) in samples.iter_mut().zip([
            (flares.queries[0], glow::LEQUAL),
            (flares.queries[1], glow::ALWAYS),
        ]) {
            gl.depth_func(compare);
            gl.begin_query(glow::SAMPLES_PASSED, query);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.end_query(glow::SAMPLES_PASSED);
            // glow's plain reads take the EXT entry point below GL 4.5,
            // which panics on a Mac. This one checks the core entry
            // point, and with no query buffer bound its offset is
            // where the result goes.
            let mut passed = 0_u64;
            let result = &raw mut passed as usize;
            gl.get_query_parameter_u64_with_offset(query, glow::QUERY_RESULT, result);
            *sample = passed as f32;
        }
        gl.disable(glow::DEPTH_TEST);
        gl.color_mask(true, true, true, false);
        (samples[1] > 0.0).then(|| samples[0] / samples[1])
    }
}

pub(in crate::app::model_preview) unsafe fn draw_flare_sprites(
    gl: &glow::Context,
    flares: &SpriteGl,
    camera: &ModelGpuCamera,
    view: &common::view::ViewFrame,
    sprites: &[ce::lights::FlareSprite],
    textures: &[Option<glow::NativeTexture>],
) {
    unsafe {
        for sprite in sprites {
            let Some(texture) = textures.get(sprite.image).copied().flatten() else {
                continue;
            };
            let vertices = ce_flare_quad(camera, view, sprite);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(&vertices), glow::STREAM_DRAW);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            let [red, green, blue] = sprite.color;
            gl.uniform_4_f32(flares.color.as_ref(), red, green, blue, sprite.alpha);
            gl.uniform_1_f32(flares.highlight.as_ref(), sprite.highlight.unwrap_or(-1.0));
            let [modulation, power] = sprite.tint.unwrap_or([0.0; 2]);
            gl.uniform_2_f32(flares.tint.as_ref(), modulation, power);
            if sprite.depth_tested {
                gl.enable(glow::DEPTH_TEST);
                gl.depth_func(glow::LEQUAL);
            }
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.disable(glow::DEPTH_TEST);
        }
    }
}

pub(super) unsafe fn link_sprite_gl(gl: &glow::Context, main: glow::NativeProgram) -> Option<SpriteGl> {
    unsafe {
        let program = link_h2_program(gl, main, CE_FLARE_VERTEX, CE_FLARE_FRAGMENT)
            .map_err(|error| eprintln!("model preview: {error}"))
            .ok()?;
        let vertex_array = gl.create_vertex_array().ok()?;
        let buffer = gl.create_buffer().ok()?;
        gl.bind_vertex_array(Some(vertex_array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 4, glow::FLOAT, false, CE_FLARE_STRIDE, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, CE_FLARE_STRIDE, 16);
        Some(SpriteGl {
            program,
            vertex_array,
            buffer,
            color: gl.get_uniform_location(program, "u_color"),
            highlight: gl.get_uniform_location(program, "u_highlight"),
            tint: gl.get_uniform_location(program, "u_tint"),
            map: gl.get_uniform_location(program, "u_map"),
            circular: gl.get_uniform_location(program, "u_circular"),
            fade: gl.get_uniform_location(program, "u_fade"),
            depth: gl.get_uniform_location(program, "u_depth"),
            scene_depth: gl.get_uniform_location(program, "u_scene_depth"),
            queries: [gl.create_query().ok()?, gl.create_query().ok()?],
        })
    }
}

pub(in crate::app::model_preview) unsafe fn upload_h2_image(gl: &glow::Context, image: &H2Image) -> Option<glow::NativeTexture> {
    if !image.whole() {
        return None;
    }
    let texture_parameters = |target| unsafe {
        gl.generate_mipmap(target);
        gl.tex_parameter_i32(target, glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32);
        gl.tex_parameter_i32(target, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
    };
    unsafe {
        match image {
            H2Image::Flat(image) => upload_texture(gl, image),
            H2Image::Cube { size, faces } => {
                let texture = gl.create_texture().ok()?;
                gl.bind_texture(glow::TEXTURE_CUBE_MAP, Some(texture));
                for (face, rgba) in faces.iter().enumerate() {
                    gl.tex_image_2d(
                        glow::TEXTURE_CUBE_MAP_POSITIVE_X + face as u32,
                        0,
                        glow::RGBA8 as i32,
                        *size as i32,
                        *size as i32,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(Some(rgba)),
                    );
                }
                texture_parameters(glow::TEXTURE_CUBE_MAP);
                for wrap in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T, glow::TEXTURE_WRAP_R] {
                    gl.tex_parameter_i32(glow::TEXTURE_CUBE_MAP, wrap, glow::CLAMP_TO_EDGE as i32);
                }
                gl.bind_texture(glow::TEXTURE_CUBE_MAP, None);
                Some(texture)
            }
            H2Image::Volume { width, height, depth, rgba } => {
                let texture = gl.create_texture().ok()?;
                gl.bind_texture(glow::TEXTURE_3D, Some(texture));
                gl.tex_image_3d(
                    glow::TEXTURE_3D,
                    0,
                    glow::RGBA8 as i32,
                    *width as i32,
                    *height as i32,
                    *depth as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(rgba)),
                );
                texture_parameters(glow::TEXTURE_3D);
                gl.bind_texture(glow::TEXTURE_3D, None);
                Some(texture)
            }
        }
    }
}

/// The cube `GLOBAL vector-normalization` names. Bilinear, as a direction
/// lookup wants, and small: only its directions matter.
unsafe fn upload_normalization_cube(gl: &glow::Context) -> Option<glow::NativeTexture> {
    const SIZE: usize = 64;
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_CUBE_MAP, Some(texture));
        for face in 0..6 {
            gl.tex_image_2d(
                glow::TEXTURE_CUBE_MAP_POSITIVE_X + face as u32,
                0,
                glow::RGBA8 as i32,
                SIZE as i32,
                SIZE as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&h2_normalization_face(face, SIZE))),
            );
        }
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_R, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_CUBE_MAP, name, value as i32);
        }
        gl.bind_texture(glow::TEXTURE_CUBE_MAP, None);
        Some(texture)
    }
}

/// A 1x1 black 2D texture: the bloom a pass reads when none was drawn.
unsafe fn upload_black_flat(gl: &glow::Context) -> Option<glow::NativeTexture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        let black = glow::PixelUnpackData::Slice(Some(&[0, 0, 0, 255]));
        gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, 1, 1, 0, glow::RGBA, glow::UNSIGNED_BYTE, black);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(texture)
    }
}

/// A 1x1 black cube, for passes whose cube map is not read: a reflection
/// that adds nothing.
unsafe fn upload_black_cube(gl: &glow::Context) -> Option<glow::NativeTexture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_CUBE_MAP, Some(texture));
        for face in 0..6 {
            gl.tex_image_2d(
                glow::TEXTURE_CUBE_MAP_POSITIVE_X + face,
                0,
                glow::RGBA8 as i32,
                1,
                1,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&[0, 0, 0, 255])),
            );
        }
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
            (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_CUBE_MAP, name, value as i32);
        }
        gl.bind_texture(glow::TEXTURE_CUBE_MAP, None);
        Some(texture)
    }
}

/// A 1x1x1 texture of one color for `target`, a volume or an array of 2D
/// layers: a stand-in for a map that is not read.
unsafe fn upload_solid_volume(gl: &glow::Context, target: u32, color: [u8; 4]) -> Option<glow::NativeTexture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(target, Some(texture));
        gl.tex_image_3d(
            target,
            0,
            glow::RGBA8 as i32,
            1,
            1,
            1,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&color)),
        );
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
            (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
        ] {
            gl.tex_parameter_i32(target, name, value as i32);
        }
        gl.bind_texture(target, None);
        Some(texture)
    }
}

/// Upload one decoded texture, with mipmaps and the wrap modes the shader
/// authored.
///
/// Mipmaps matter here beyond quality: a detail map tiling twenty times across
/// a model aliases into noise without them, which is exactly where a preview
/// looks worst.
unsafe fn upload_texture(gl: &glow::Context, image: &TextureImage) -> Option<glow::NativeTexture> {
    if image.width == 0 || image.height == 0 || !image.whole() {
        return None;
    }
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            image.width as i32,
            image.height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&image.rgba)),
        );
        gl.generate_mipmap(glow::TEXTURE_2D);
        let wrap = |repeat: bool| {
            if repeat {
                glow::REPEAT as i32
            } else {
                glow::CLAMP_TO_EDGE as i32
            }
        };
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap(image.repeat_x));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap(image.repeat_y));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR_MIPMAP_LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(texture)
    }
}
