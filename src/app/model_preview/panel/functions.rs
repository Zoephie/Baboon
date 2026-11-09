//! The object function controls: the game's own controls, the unit's
//! weapons and the function sliders.

use super::super::{ModelPreviewData, ModelPreviewState, frame};
use super::weapons::draw_unit_weapon_pickers;
use eframe::egui::Ui;

pub(in crate::app::model_preview) fn draw_object_functions(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    let controls = super::super::load::rules(data.preview.game).object_controls;
    if let Some(controls) = controls {
        (controls.above)(ui, data, state);
    }
    draw_unit_weapon_pickers(ui, data, state);
    if let Some(functions) = &data.object_functions {
        let time = ui.input(|input| input.time) as f32;
        let values = frame::object_function_values(data, &state.function_inputs, time);
        super::super::functions::draw_function_controls(
            ui,
            functions,
            data.geometry_id,
            &mut state.function_inputs,
            &values,
            &frame::shader_input_names(data, 0),
        );
    }
    if let Some(controls) = controls {
        (controls.below)(ui, data, state);
    }
}
