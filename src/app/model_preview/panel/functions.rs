//! The Model Setup sidebar's body: its sections, each collapsible with a
//! Reset in its header, and each shown only when the object has something
//! for it.

use super::super::super::ui_kit::button_icons::{ButtonIcon, icon_button};
use super::super::super::ui_kit::style::text_dark;
use super::super::{
    ModelPreviewData, ModelPreviewState, ModelRegionSelection, compute_variant_selection,
    default_permutation, frame, is_model_physics_overlay_region, reset_model_preview_selection,
};
use super::setup::draw_scenario_setup;
use super::variants::draw_variant_controls;
use super::weapons::{carry_through_weapon_change, draw_unit_weapon_pickers};
use eframe::egui;
use eframe::egui::{RichText, Ui};

/// The sidebar's body: a scenario's BSPs, the permutations, the weapons,
/// the effects and the object's functions.
pub(in crate::app::model_preview) fn draw_setup_body(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    let own = super::super::load::rules(data.preview.game).object_controls;
    if !data.scenario_bsps.is_empty() {
        let changed = !state.scenario_bsp_selection.is_empty();
        if setup_section(ui, "Structure BSPs", changed, |ui| {
            draw_scenario_setup(ui, data, state)
        }) {
            state.scenario_bsp_selection.clear();
        }
    }
    if has_permutations(data, state) {
        let changed = permutations_changed(data, state);
        if setup_section(ui, "Permutations", changed, |ui| {
            draw_variant_controls(ui, data, state)
        }) {
            reset_model_preview_selection(state, data, state.selected_variant);
        }
    }
    let shared_weapons = data
        .weapons
        .as_ref()
        .is_some_and(|weapons| !weapons.equippable.is_empty());
    if shared_weapons || (own.weapons.has)(data) {
        let changed = weapons_changed(state) || (own.weapons.changed)(data, state);
        if setup_section(ui, "Weapons", changed, |ui| {
            draw_unit_weapon_pickers(ui, data, state);
            if (own.weapons.has)(data) {
                (own.weapons.draw)(ui, data, state);
            }
        }) {
            reset_weapons(data, state);
            (own.weapons.reset)(data, state);
        }
    }
    let shader_inputs = frame::shader_input_names(data, 0);
    let (events, sliders) = data
        .object_functions
        .as_ref()
        .map(|functions| super::super::functions::function_names(functions, &shader_inputs))
        .unwrap_or_default();
    let key = data.geometry_id;
    super::super::functions::settle_event_effects(&events, key, &mut state.function_inputs);
    if !events.is_empty() || (own.effects.has)(data) {
        let changed = held_inputs(state, key, &events, true) || (own.effects.changed)(data, state);
        if setup_section(ui, "Effects", changed, |ui| {
            super::super::functions::draw_event_effects(
                ui,
                &events,
                key,
                &mut state.function_inputs,
            );
            if (own.effects.has)(data) {
                (own.effects.draw)(ui, data, state);
            }
        }) {
            release_inputs(state, key, &events, true);
            (own.effects.reset)(data, state);
        }
    }
    if !sliders.is_empty() || (own.functions.has)(data) {
        let changed =
            held_inputs(state, key, &events, false) || (own.functions.changed)(data, state);
        if setup_section(ui, "Object Functions", changed, |ui| {
            if let Some(functions) = data
                .object_functions
                .as_ref()
                .filter(|_| !sliders.is_empty())
            {
                let time = ui.input(|input| input.time) as f32;
                let values = frame::object_function_values(data, &state.function_inputs, time);
                super::super::functions::draw_function_controls(
                    ui,
                    functions,
                    &sliders,
                    key,
                    &mut state.function_inputs,
                    &values,
                    &shader_inputs,
                );
            }
            if (own.functions.has)(data) {
                (own.functions.draw)(ui, data, state);
            }
        }) {
            release_inputs(state, key, &events, false);
            (own.functions.reset)(data, state);
        }
    }
}

/// A section titled `title`, open until collapsed, with a Reset in its
/// header that is enabled when `changed`. Whether Reset was pressed.
pub(in crate::app::model_preview) fn setup_section(
    ui: &mut Ui,
    title: &str,
    changed: bool,
    body: impl FnOnce(&mut Ui),
) -> bool {
    let id = ui.make_persistent_id(("model_setup_section", title));
    let mut reset = false;
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            ui.label(RichText::new(title).strong().color(text_dark()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let tooltip = format!("Reset {title}");
                reset = icon_button(ui, ButtonIcon::Undo, &tooltip, changed, text_dark()).clicked();
            });
        })
        .body(body);
    reset
}

/// Whether any region shows in the Permutations section.
fn has_permutations(data: &ModelPreviewData, state: &ModelPreviewState) -> bool {
    data.preview
        .regions
        .iter()
        .any(|region| !is_model_physics_overlay_region(data, state, region))
}

/// Whether any region is away from what the variant shown gives it.
pub(in crate::app::model_preview) fn permutations_changed(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> bool {
    let wanted = compute_variant_selection(data, state.selected_variant);
    data.preview
        .regions
        .iter()
        .filter(|region| !is_model_physics_overlay_region(data, state, region))
        .any(|region| {
            // A region the panel has not drawn yet starts as it will be drawn.
            let live = state
                .region_selections
                .get(&region.name)
                .cloned()
                .unwrap_or_else(|| ModelRegionSelection {
                    enabled: !region.permutations.is_empty(),
                    permutation: default_permutation(region),
                });
            wanted.get(&region.name) != Some(&live)
        })
}

/// Whether the unit holds a weapon the panel gave it.
fn weapons_changed(state: &ModelPreviewState) -> bool {
    state.weapon.is_some() || state.secondary_weapon.is_some()
}

/// Empties both hands, carrying the selection through the reload.
fn reset_weapons(data: &ModelPreviewData, state: &mut ModelPreviewState) {
    if weapons_changed(state) {
        carry_through_weapon_change(data, state);
        state.weapon = None;
        state.secondary_weapon = None;
    }
}

/// Whether the panel holds any of the object's inputs: its event effects'
/// (`events_only`) or the rest.
pub(in crate::app::model_preview) fn held_inputs(
    state: &ModelPreviewState,
    key: u64,
    events: &[&str],
    events_only: bool,
) -> bool {
    state
        .function_inputs
        .keys()
        .any(|(at, name)| *at == key && events.contains(&name.as_str()) == events_only)
}

/// Lets go of the inputs [`held_inputs`] finds.
pub(in crate::app::model_preview) fn release_inputs(
    state: &mut ModelPreviewState,
    key: u64,
    events: &[&str],
    events_only: bool,
) {
    state
        .function_inputs
        .retain(|(at, name), _| *at != key || events.contains(&name.as_str()) != events_only);
}
