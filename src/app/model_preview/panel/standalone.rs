//! Chimp's standalone mesh viewer, which draws an Unreal mesh with no tag.

use super::super::super::ui_kit::checkbox::ThemedCheckbox;
use super::super::super::ui_kit::style::subtle_dark;
use super::super::{
    MAX_PREVIEW_SCALE, ModelPreviewData, ModelPreviewState, ModelRenderMode, RenderModelPreview,
    draw_model_viewport, min_preview_scale, model_preview_data,
};
use crate::app::ui_kit::checkbox::CheckboxUi as _;
use eframe::egui;
use eframe::egui::{RichText, Ui, Vec2};

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
    let least = min_preview_scale(&data.preview);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Scale").color(subtle_dark()));
        ui.add(
            egui::Slider::new(&mut state.scale, least..=MAX_PREVIEW_SCALE)
                .logarithmic(true)
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        );
        let drag_speed = (state.scale * 0.05) as f64;
        ui.add(
            egui::DragValue::new(&mut state.scale)
                .range(least..=MAX_PREVIEW_SCALE)
                .speed(drag_speed)
                .max_decimals(5)
                .suffix("×"),
        );
        if ui.button("Reset view").clicked() {
            state.yaw = -0.45;
            state.pitch = 0.25;
            state.focus = [0.0; 3];
            state.scale = 1.0;
        }
        crate::app::ui_kit::ValueComboBox::from_id_salt((
            "standalone_model_render_mode",
            &data.source_key,
        ))
        .selected_text(state.render_mode.label())
        .show_ui(ui, |ui| {
            for mode in ModelRenderMode::ALL {
                ui.selectable_value(&mut state.render_mode, mode, mode.label());
            }
        });
        ui.baboon_checkbox(&mut state.perspective, "Perspective")
            .on_hover_text("Perspective projection instead of the flat orthographic view.");
        ui.baboon_checkbox(&mut state.show_grid, "Grid")
            .on_hover_text("Ground-reference grid on the z = 0 plane.");
        ui.add_enabled(
            false,
            ThemedCheckbox::new(&mut state.show_backfaces, "Backfaces"),
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
