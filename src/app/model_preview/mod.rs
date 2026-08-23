//! Model loading, variant selection, and depth-tested preview presentation.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;
use blam_tags::math::{RealPoint3d, RealQuaternion, RealVector3d};
use blam_tags::render_model::{Marker, Node, RenderMesh};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub(in crate::app) mod loading;
pub(in crate::app) mod materials;
mod renderer;
mod variants;

use loading::*;
// Re-exported up to `crate::app`: the preview state and the worker message
// both name these, and neither lives under this module.
pub(in crate::app) use materials::*;
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
    /// One entry per `RenderModelPreviewBatch::material_index`, naming the
    /// shader tag that batch draws with. Kept as the raw reference rather than
    /// resolved textures: resolving reads other tags off disk, which belongs on
    /// a worker rather than in the geometry walk.
    pub materials: Vec<RenderModelPreviewMaterial>,
    pub markers: Vec<RenderModelPreviewMarker>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewRegion {
    pub name: String,
    pub permutations: Vec<String>,
}

/// One vertex as the GL program consumes it. `#[repr(C)]` because the whole
/// buffer is uploaded as raw bytes; the attribute offsets in
/// [`renderer::ModelGlRenderer::new`] are hand-written to match this order and
/// a test pins them.
///
/// `tangent` and `binormal` are carried rather than reconstructed: a normal map
/// needs the handedness the tag authored, and `cross(normal, tangent)` alone
/// cannot recover a mirrored UV island's sign — which is most of a Halo
/// character, since they mirror left to right to halve texture space.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RenderModelPreviewVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub texcoord: [f32; 2],
    pub tangent: [f32; 3],
    pub binormal: [f32; 3],
}

/// The shader one material draws with, as named by the render_model.
#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewMaterial {
    /// Tag-relative path, no extension — e.g.
    /// `objects\characters\masterchief\shaders\masterchief`. Empty when
    /// the tag_ref was null.
    pub shader_path: String,
    /// Group FOURCC of the shader reference (`rmsh`, `rmtr`, `shad`, ...).
    /// Decides which extension the path resolves with, and which resolver
    /// understands the tag behind it.
    pub shader_group: u32,
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

static NEXT_MODEL_GEOMETRY_ID: AtomicU64 = AtomicU64::new(1);

fn model_preview_data(
    source_key: String,
    render_model_path: String,
    preview: RenderModelPreview,
    variants: Vec<ModelVariantPreview>,
) -> ModelPreviewData {
    ModelPreviewData {
        source_key,
        render_model_path,
        preview: Arc::new(preview),
        geometry_id: NEXT_MODEL_GEOMETRY_ID.fetch_add(1, Ordering::Relaxed),
        textures: None,
        variants,
    }
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
    let is_model = is_previewable_geometry_group(entry.group_tag, names);
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
            let needs_load = state.loaded_key.as_deref() != Some(entry.key.as_str())
                || state.data.is_none()
                || state.loaded_high_detail != state.high_detail;
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
                ui.checkbox(&mut state.shaded, "Shaded")
                    .on_hover_text(
                        "Sample each part's own shader — diffuse, detail, normal, specular and                          self-illumination. Off draws the flat per-material colours, which stay                          useful for reading silhouette and topology.",
                    );
                ui.checkbox(&mut state.show_backfaces, "Backfaces");
                // Campaign Evolved: static pieces are Nanite. Full detail is
                // the faithful default; users can opt into the coarse fallback
                // for unusually heavy models. Only meaningful for CE `.model`s.
                let is_campaign_evolved = tag.header.group_tag.to_be_bytes() == *b"hlmt"
                    && tag
                        .root()
                        .read_tag_ref_with_group("skeleton model")
                        .map(|(_, r)| !r.trim().is_empty())
                        .unwrap_or(false);
                if is_campaign_evolved {
                    ui.checkbox(&mut state.high_detail, "High detail")
                        .on_hover_text(
                            "Decode full-resolution Nanite geometry instead of Unreal's coarse \
                             fallback. Disable this for a faster, lower-detail preview.",
                        );
                }
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
    // Hold the viewport until the textures land, rather than drawing the model
    // untextured and re-shading it a second later — a model that changes
    // appearance under the cursor reads as a glitch, not as progress.
    if state.shaded && state.textures_pending && data.textures.is_none() {
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        ui.painter()
            .rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.horizontal_centered(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Loading shaders…").color(subtle_dark()));
                });
            });
        });
        ui.ctx().request_repaint();
        return;
    }
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

/// Build the renderer's camera-independent data for a standalone mesh, such as
/// a Chimp StaticMesh/SkeletalMesh document.
pub(in crate::app) fn standalone_mesh_preview(
    source_key: String,
    preview: RenderModelPreview,
) -> ModelPreviewData {
    model_preview_data(source_key.clone(), source_key, preview, Vec::new())
}

/// Draw a standalone mesh with the same camera, shading, wireframe and
/// backface controls as Baboon's tag model viewer.
pub(in crate::app) fn draw_standalone_mesh_preview(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    // Chimp presents raw Unreal sections, including intentionally two-sided
    // surfaces. Keep backfaces visible unconditionally; classic tag viewers
    // continue to use their own independent toggle in draw_model_preview_panel.
    state.show_backfaces = true;
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
        if ui.button("Reset view").clicked() {
            state.yaw = -0.45;
            state.pitch = 0.25;
            state.pan = Vec2::ZERO;
            state.scale = 1.0;
        }
        egui::ComboBox::from_id_salt(("standalone_model_render_mode", &data.source_key))
            .selected_text(state.render_mode.label())
            .show_ui(ui, |ui| {
                for mode in ModelRenderMode::ALL {
                    ui.selectable_value(&mut state.render_mode, mode, mode.label());
                }
            });
        ui.add_enabled(
            false,
            egui::Checkbox::new(&mut state.show_backfaces, "Backfaces"),
        )
        .on_hover_text("Chimp always displays backfaces");
    });
    let size = Vec2::new(
        ui.available_width().max(280.0),
        ui.available_height().max(300.0),
    );
    draw_model_viewport(ui, data, state, size);
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

    #[test]
    fn campaign_evolved_preview_defaults_to_full_detail() {
        assert!(ModelPreviewState::default().high_detail);
    }
}
