//! The Model Setup sidebar: a card per part of the object's setup, each
//! collapsible with a Reset in its header, and each shown only when the
//! object has something for it.

use super::super::super::editor::fields::{BoxPart, draw_resettable_box};
use super::super::{
    ModelPreviewData, ModelPreviewState, ModelRegionSelection, compute_variant_selection,
    default_permutation, frame, is_model_physics_overlay_region, reset_model_preview_selection,
};
use super::rows::CardRows;
use super::setup::draw_scenario_setup;
use super::variants::{draw_variant_controls, draw_variant_selector};
use super::weapons::{carry_through_reload, draw_unit_weapon_pickers};
use eframe::egui::Ui;

/// The sidebar's cards: a scenario's BSPs, the permutations (with the
/// variant picker and `variant_actions` in its header), the weapons, and,
/// with Game Shaders on, the effects and the object's functions.
pub(in crate::app::model_preview) fn draw_setup_body(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    // The game's materials are drawn: the Materials card shows.
    materials: bool,
    variant_actions: &mut dyn FnMut(&mut Ui, &mut ModelPreviewState),
) {
    let own = super::super::load::rules(data.preview.game).object_controls;
    if !data.scenario_bsps.is_empty() {
        let changed = !state.scenario_bsp_selection.is_empty();
        if setup_section(ui, "Structure BSPs", changed, |ui, part| {
            if part == BoxPart::Body {
                draw_scenario_setup(ui, data, state);
            }
        }) {
            state.scenario_bsp_selection.clear();
        }
    }
    if data.preview.decorators.is_some() {
        super::decorators::draw_decorator_cards(ui, data, state);
    } else if has_permutations(data) {
        let changed = permutations_changed(data, state);
        if setup_section(ui, "Permutations", changed, |ui, part| {
            if part == BoxPart::Body {
                let mut rows = CardRows::default();
                draw_variant_selector(ui, data, state, &mut rows, variant_actions);
                draw_variant_controls(ui, data, state, &mut rows);
            }
        }) {
            reset_model_preview_selection(state, data, state.selected_variant);
        }
    }
    draw_materials_card(ui, data, state, materials);
    let shared_weapons = data
        .weapons
        .as_ref()
        .is_some_and(|weapons| weapons.can_hold());
    if shared_weapons || (own.weapons.has)(data) {
        let changed = weapons_changed(state) || (own.weapons.changed)(data, state);
        if setup_section(ui, "Weapons", changed, |ui, part| {
            if part == BoxPart::Body {
                draw_unit_weapon_pickers(ui, data, state);
                if (own.weapons.has)(data) {
                    (own.weapons.draw)(ui, data, state);
                }
            }
        }) {
            reset_weapons(data, state);
            (own.weapons.reset)(data, state);
        }
    }
    // Effects and functions are Game Shaders' alone: off, the load reads none.
    if !state.game_shaders {
        return;
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
        if setup_section(ui, "Effects", changed, |ui, part| {
            if part == BoxPart::Body {
                let mut rows = CardRows::default();
                super::super::functions::draw_event_effects(
                    ui,
                    &mut rows,
                    &events,
                    key,
                    &mut state.function_inputs,
                );
                if (own.effects.has)(data) {
                    (own.effects.draw)(ui, data, state);
                }
            }
        }) {
            release_inputs(state, key, &events, true);
            (own.effects.reset)(data, state);
        }
    }
    if !sliders.is_empty() || (own.functions.has)(data) {
        let changed =
            held_inputs(state, key, &events, false) || (own.functions.changed)(data, state);
        if setup_section(ui, "Object Functions", changed, |ui, part| {
            if part != BoxPart::Body {
                return;
            }
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

/// A card titled `title`, boxed like a tag block and open until collapsed,
/// with a Reset at the right of its header that is enabled when `changed`;
/// `contents` draws its other header controls and its body. Whether Reset
/// was pressed.
/// The Materials card: room for the Material Editor, which the tag pane
/// draws there once its own drawing is done. Open, the editor is; closed,
/// it lets go of the shaders it holds that have no unsaved edits. Only for a
/// game whose materials the preview draws, and a model that names some.
fn draw_materials_card(ui: &mut Ui, data: &ModelPreviewData, state: &mut ModelPreviewState, materials: bool) {
    let live = ui.is_enabled();
    let names_some = data.preview.materials.iter().any(|material| !material.shader_path.trim().is_empty());
    if !materials || !names_some {
        if live {
            state.material_editor = None;
        }
        return;
    }
    let mut slot = None;
    let open = super::super::super::editor::fields::draw_foundation_box_opened(
        ui,
        "Materials".to_owned(),
        ("model_setup_section", "Materials"),
        Some(state.material_editor.is_some()),
        |ui, part| {
            if part == BoxPart::Body {
                // As tall as the editor drew last pass; its dropdown's height
                // before it has drawn.
                let height = state
                    .material_content_height
                    .unwrap_or(super::super::material_editor::MATERIAL_SLOT_PICKER_HEIGHT);
                let size = eframe::egui::vec2(ui.available_width(), height);
                let (rect, _) = ui.allocate_exact_size(size, eframe::egui::Sense::hover());
                slot = Some(super::super::material_editor::MaterialSlot {
                    rect,
                    clip: ui.clip_rect(),
                    layer: ui.layer_id(),
                    enabled: ui.is_enabled(),
                });
            }
        },
    );
    state.material_slot = slot;
    // Still, while the model loads again: shown as it was, kept as it is.
    // Closed, it is kept as it was, to show again when opened.
    if live {
        match (open, state.material_editor.is_some()) {
            (true, false) => {
                state.material_editor = Some(state.material_editor_closed.take().unwrap_or_default());
            }
            (false, true) => {
                let mut kept = state.material_editor.take();
                if let Some(kept) = kept.as_mut() {
                    kept.hovered = None;
                    kept.confirm_reset = None;
                }
                state.material_editor_closed = kept;
            }
            _ => {}
        }
    }
}

pub(in crate::app::model_preview) fn setup_section(
    ui: &mut Ui,
    title: &str,
    changed: bool,
    contents: impl FnMut(&mut Ui, BoxPart),
) -> bool {
    draw_resettable_box(ui, title, ("model_setup_section", title), changed, contents)
}

/// Whether any region shows in the Permutations section.
fn has_permutations(data: &ModelPreviewData) -> bool {
    data.preview
        .regions
        .iter()
        .any(|region| !is_model_physics_overlay_region(data, region))
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
        .filter(|region| !is_model_physics_overlay_region(data, region))
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
        carry_through_reload(data, state);
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
