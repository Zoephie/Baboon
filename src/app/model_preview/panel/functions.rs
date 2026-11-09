//! The object function controls: CE event effects, the function sliders and CE's function inputs.

use super::super::{ModelPreviewData, ModelPreviewState, ce, frame};
use super::weapons::{draw_ce_weapon_picker, draw_h2_weapon_pickers};
use eframe::egui::Ui;

pub(in crate::app::model_preview) fn draw_object_functions(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    draw_ce_weapon_picker(ui, data, state);
    let held_name = |object: u8| {
        let held = usize::from(object).checked_sub(1)?;
        data.ce_attached.get(held).map(|held| held.name.clone())
    };
    let time = frame::preview_clock(ui);
    let controls = ce::panel::CeEffectControls {
        effects: &data.preview.ce.effects,
        firing: &data.preview.ce.firing,
        geometry: data.geometry_id,
        counts: &mut state.ce_event_counts,
        shots: &mut state.ce_shots,
        scales: &mut state.ce_event_scales,
        time,
    };
    ce::panel::draw_ce_event_effects(ui, controls, held_name);
    draw_h2_weapon_pickers(ui, data, state);
    if let Some(functions) = &data.object_functions {
        let time = ui.input(|input| input.time) as f32;
        let values = frame::h2_function_values(data, &state.function_inputs, time);
        super::super::functions::draw_function_controls(
            ui,
            functions,
            data.geometry_id,
            &mut state.function_inputs,
            &values,
            &frame::h2_shader_inputs(data, 0),
        );
    }
    let read = |object: u8| {
        data.textures.as_ref().map(|textures| {
            let mut read = [false; 4];
            let owned = data.preview.materials.iter().zip(textures.iter());
            let passes = owned
                .filter(|(material, _)| material.object == object)
                .flat_map(|(_, texture)| texture.h2_layers.iter().flatten());
            for animation in passes.filter_map(|pass| pass.ce_animation.as_ref()) {
                for (read, output) in read.iter_mut().zip(animation.outputs_read()) {
                    *read |= output;
                }
            }
            let preview = &data.preview;
            ce::lights::mark_outputs_read(&preview.ce.lights, object, &mut read);
            ce::functions::mark_outputs_read(&preview.ce.effects, object, &mut read);
            ce::functions::mark_outputs_read(&preview.ce.contrails, object, &mut read);
            ce::functions::mark_outputs_read(&preview.ce.particle_systems, object, &mut read);
            ce::functions::mark_outputs_read(&preview.ce.light_volumes, object, &mut read);
            ce::functions::mark_outputs_read(&preview.ce.lightning, object, &mut read);
            ce::functions::mark_outputs_read(&preview.ce.glows, object, &mut read);
            read
        })
    };
    let attached = data
        .ce_attached
        .iter()
        .enumerate()
        .filter_map(|(index, object)| {
            let functions = object.functions.as_ref()?;
            Some((index as u8 + 1, Some(object.name.as_str()), functions))
        });
    let own = data
        .ce_object_functions
        .iter()
        .map(|functions| (0, None, functions));
    for (object, name, functions) in own.chain(attached) {
        ce::panel::draw_ce_function_controls(
            ui,
            functions,
            name,
            (data.geometry_id, object),
            &mut state.ce_function_inputs,
            read(object),
            ce::firing::fired_input(
                &data.preview.ce.firing,
                &state.ce_shots,
                data.geometry_id,
                object,
                time,
            ),
        );
    }
}
