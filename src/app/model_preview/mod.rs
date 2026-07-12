//! Model loading, variant selection, and software-rendered preview presentation.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;
use blam_tags::math::{RealPoint3d, RealQuaternion, RealVector3d};
use blam_tags::render_model::{Marker, Node, RenderMesh};

mod loading;
mod renderer;
mod variants;

use loading::*;
use renderer::*;
use variants::*;

/// Renderer-facing preview geometry derived from a [`RenderModel`]. Lives in
/// Baboon (not blam-tags) since it is purely a GUI concern.
#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreview {
    pub regions: Vec<RenderModelPreviewRegion>,
    pub vertices: Vec<RenderModelPreviewVertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<RenderModelPreviewBatch>,
    pub markers: Vec<RenderModelPreviewMarker>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewRegion {
    pub name: String,
    pub permutations: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RenderModelPreviewVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewBatch {
    pub region_name: String,
    pub permutation_name: String,
    pub material_index: u16,
    pub index_start: u32,
    pub index_count: u32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewMarker {
    pub name: String,
    pub position: [f32; 3],
    pub axes: [[f32; 3]; 3],
}

pub(super) fn draw_model_preview_panel(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    state: &mut ModelPreviewState,
    model_preview_size: &mut f32,
    edit: &mut FieldEditContext<'_>,
) {
    let is_model = is_model_group(entry.group_tag, names);
    if !is_model {
        return;
    }

    egui::CollapsingHeader::new(RichText::new("Render model").strong().color(text_dark()))
        .id_salt(("model_preview", &entry.key))
        .default_open(true)
        .show(ui, |ui| {
            // The parse is synchronous; on the first frame for a tag, show a
            // spinner, kick the (blocking) parse, and repaint so the decoded
            // model appears next frame instead of a blank panel. (A future
            // change can move the parse to a worker thread — see plan 1.9.)
            let needs_load =
                state.loaded_key.as_deref() != Some(entry.key.as_str()) || state.data.is_none();
            if needs_load {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Loading model…").color(subtle_dark()));
                });
                ensure_model_preview_loaded(tag, entry, names, source, state);
                ui.ctx().request_repaint();
                return;
            }

            ui.horizontal(|ui| {
                ui.label(RichText::new("Scale").color(subtle_dark()));
                ui.add(
                    egui::Slider::new(&mut state.scale, 0.05..=5.0)
                        .show_value(false)
                        .clamping(egui::SliderClamping::Always),
                );
                ui.add(
                    egui::DragValue::new(&mut state.scale)
                        .range(0.05..=5.0)
                        .speed(0.01)
                        .max_decimals(2)
                        .suffix("×"),
                );
                if ui.button("Reset").clicked() {
                    state.yaw = -0.45;
                    state.pitch = 0.25;
                    state.pan = Vec2::ZERO;
                    state.scale = 1.0;
                }
                ui.checkbox(&mut state.show_markers, "Markers");
                if state.show_markers {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.marker_filter)
                            .hint_text("filter markers…")
                            .desired_width(110.0),
                    );
                }
                egui::ComboBox::from_id_salt(("model_render_mode", &entry.key))
                    .selected_text(state.render_mode.label())
                    .show_ui(ui, |ui| {
                        for mode in ModelRenderMode::ALL {
                            ui.selectable_value(&mut state.render_mode, mode, mode.label());
                        }
                    });
                ui.checkbox(&mut state.show_backfaces, "Backfaces");
                ui.label(RichText::new("Viewport").color(subtle_dark()));
                ui.add(
                    egui::Slider::new(
                        model_preview_size,
                        MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
                    )
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
                );
                draw_model_viewport_size_input(ui, model_preview_size);
                if ui.button("Refresh model").clicked() {
                    state.loaded_key = None;
                    state.data = None;
                    ensure_model_preview_loaded(tag, entry, names, source, state);
                }
            });

            let Some(data_result) = state.data.take() else {
                ui.label(RichText::new("No preview loaded").color(subtle_dark()));
                return;
            };
            let mut restore_data = Some(data_result);
            let data = match restore_data.as_ref().expect("preview data just set") {
                Ok(data) => data,
                Err(error) => {
                    ui.colored_label(Color32::from_rgb(150, 56, 44), error);
                    state.data = restore_data.take();
                    return;
                }
            };

            let mut mutation_requested = false;
            let desired_viewport = model_viewport_size(ui.available_width(), *model_preview_size);
            let can_place_controls_beside = ui.available_width() >= desired_viewport.x + 360.0;
            if can_place_controls_beside {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        draw_model_viewport_with_stats(ui, data, state, desired_viewport)
                    });
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        if draw_variant_controls(ui, data, state, edit) {
                            mutation_requested = true;
                        }
                    });
                });
            } else {
                draw_model_viewport_with_stats(ui, data, state, desired_viewport);
                ui.add_space(8.0);
                if draw_variant_controls(ui, data, state, edit) {
                    mutation_requested = true;
                }
            }
            if mutation_requested {
                state.loaded_key = None;
                state.data = None;
            } else {
                state.data = restore_data.take();
            }
        });
    ui.add_space(8.0);
}

pub(in crate::app) fn draw_model_viewport_size_input(ui: &mut Ui, model_preview_size: &mut f32) {
    let mut percent = model_preview_size_percent(*model_preview_size);
    let response = ui.add(
        egui::DragValue::new(&mut percent)
            .range(
                model_preview_size_percent(MIN_MODEL_PREVIEW_SIZE)
                    ..=model_preview_size_percent(MAX_MODEL_PREVIEW_SIZE),
            )
            .speed(1.0)
            .max_decimals(0)
            .suffix("%"),
    );
    if response.changed() {
        *model_preview_size = model_preview_size_from_percent(percent);
    }
}

fn model_preview_size_percent(model_preview_size: f32) -> f32 {
    model_preview_size * 100.0
}

fn model_preview_size_from_percent(percent: f32) -> f32 {
    (percent / 100.0).clamp(MIN_MODEL_PREVIEW_SIZE, MAX_MODEL_PREVIEW_SIZE)
}

fn model_viewport_size(available_width: f32, model_preview_size: f32) -> Vec2 {
    let scale = model_preview_size.clamp(MIN_MODEL_PREVIEW_SIZE, MAX_MODEL_PREVIEW_SIZE);
    let desired = Vec2::new(470.0 * scale, 300.0 * scale);
    let width = desired.x.min(available_width.max(280.0)).max(280.0);
    Vec2::new(width, desired.y * (width / desired.x))
}

fn draw_model_viewport_with_stats(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    desired_size: Vec2,
) {
    draw_model_viewport(ui, data, state, desired_size);
    ui.small(
        RichText::new(format!(
            "{} vertices, {} triangles",
            data.preview.vertices.len(),
            data.preview.indices.len() / 3
        ))
        .color(subtle_dark()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_percentage_conversion_clamps_to_persisted_range() {
        assert_eq!(model_preview_size_percent(1.25), 125.0);
        assert_eq!(model_preview_size_from_percent(125.0), 1.25);
        assert_eq!(
            model_preview_size_from_percent(20.0),
            MIN_MODEL_PREVIEW_SIZE
        );
        assert_eq!(
            model_preview_size_from_percent(400.0),
            MAX_MODEL_PREVIEW_SIZE
        );
    }
}
