//! The panel's controls for a Halo CE object's functions and effects.

use super::super::load::{ObjectControls, SectionControls};
use super::super::panel::{
    CardRows, carry_through_reload, draw_held_regions, name_cell, name_column_width, play_row,
    value_slider, weapon_dropdown,
};
use crate::app::ui_kit::button_icons::ButtonIcon;
use super::super::{ModelPreviewData, ModelPreviewState, frame};
use crate::app::model_preview::ce::functions::{
    CeEventCounts, CeFunctionInputs, CeObjectFunctions, CeObjectKey, OUTPUTS, object_time,
};
use eframe::egui::{RichText, Ui};

/// What the panel has set and played of an opened Halo CE object.
pub(in crate::app) struct CePanelState {
    /// What the panel set the object's function inputs to.
    pub(in crate::app) function_inputs: CeFunctionInputs,
    /// How many times the panel has played each event effect, by model and
    /// index among the preview's effects.
    pub(in crate::app) event_counts: CeEventCounts,
    /// Scale A and scale B the panel plays event effects at.
    pub(in crate::app) event_scales: [f32; 2],
    /// The shots the panel has fired from weapons' triggers.
    pub(in crate::app) shots: super::firing::CeShots,
}

impl CePanelState {
    /// A weapon change carries the opened object's function inputs from the
    /// unit as it was, `from`, to the unit as it loads, `to`.
    pub(in crate::app::model_preview) fn carry_function_inputs(&mut self, from: u64, to: u64) {
        if let Some(inputs) = self.function_inputs.get(&(from, 0)).copied() {
            self.function_inputs.insert((to, 0), inputs);
        }
    }
}

impl Default for CePanelState {
    fn default() -> Self {
        Self {
            function_inputs: Default::default(),
            event_counts: Default::default(),
            event_scales: super::functions::CE_EVENT_SCALES,
            shots: Default::default(),
        }
    }
}

/// What the Effects section plays and fires, and where it keeps track.
pub(in crate::app::model_preview) struct CeEffectControls<'a> {
    pub(in crate::app::model_preview) effects: &'a [super::effects::CeEffectAt],
    pub(in crate::app::model_preview) firing: &'a [super::firing::CeWeaponFiring],
    pub(in crate::app::model_preview) geometry: u64,
    pub(in crate::app::model_preview) counts: &'a mut CeEventCounts,
    pub(in crate::app::model_preview) shots: &'a mut super::firing::CeShots,
    pub(in crate::app::model_preview) scales: &'a mut [f32; 2],
    /// The preview's clock, which shots are fired at.
    pub(in crate::app::model_preview) time: f32,
}

/// A Fire button for each trigger of a weapon, and a Play button for each
/// effect the objects name for their events, each under its object's name
/// when it is not the opened one's, with the scales they go at.
pub(in crate::app::model_preview) fn draw_ce_event_effects(
    ui: &mut Ui,
    controls: CeEffectControls<'_>,
    object_name: impl Fn(u8) -> Option<String>,
) {
    let CeEffectControls {
        effects,
        firing,
        geometry,
        counts,
        shots,
        scales,
        time,
    } = controls;
    let named = |object: u8, name: &str| {
        object_name(object).map_or_else(|| name.to_owned(), |object| format!("{object}: {name}"))
    };
    let events: Vec<(usize, String)> = effects
        .iter()
        .enumerate()
        .filter_map(|(index, effect)| {
            Some((index, named(effect.object, &effect.event.as_ref()?.name)))
        })
        .collect();
    if events.is_empty() && firing.is_empty() {
        return;
    }
    const SCALES: &str = "What effect_new_from_object is given. A firing effect gets its trigger's \
                          rate and error; reloading, readying, damage and the rest get 0 and 0.";
    const FIRE: &str = "Fire the trigger once, as weapon_trigger_fire does: it plays the next \
                        of its firing effects and lights the weapon's inputs, at scale A's \
                        rate of fire. The weapon is held as an actor holds one: it never \
                        overheats.";
    let mut rows = CardRows::default();
    let name_width = name_column_width(ui, ["Scale A", "Scale B"], ui.available_width());
    for (scale, name) in scales.iter_mut().zip(["Scale A", "Scale B"]) {
        rows.row(ui, |ui| {
            name_cell(ui, name, name_width);
            value_slider(ui, scale, true).on_hover_text(SCALES);
        });
    }
    for weapon in firing {
        for (trigger, label) in
            (0..weapon.triggers.len()).zip(["primary trigger", "secondary trigger"])
        {
            let name = format!("Fire {}", named(weapon.object, label));
            if play_row(ui, &mut rows, ButtonIcon::Play, &name, FIRE) {
                let fired = shots.entry((geometry, weapon.object)).or_default();
                if let Some(effect) = fired.fire(weapon, trigger, scales[0], time) {
                    *counts.entry((geometry, effect)).or_default() += 1;
                }
                ui.ctx().request_repaint();
            }
        }
    }
    for (index, name) in events {
        if play_row(ui, &mut rows, ButtonIcon::Play, &name, "Play this effect") {
            *counts.entry((geometry, index)).or_default() += 1;
            ui.ctx().request_repaint();
        }
    }
}

/// Sliders for an opened object's inputs, and what its functions make of them.
/// `read` is which outputs the model's shaders read, once they have resolved:
/// the rest cannot change the picture, whatever the game uses them for.
/// `fired` is what the panel's shots give the inputs, by name.
pub(in crate::app::model_preview) fn draw_ce_function_controls(
    ui: &mut Ui,
    functions: &CeObjectFunctions,
    name: Option<&str>,
    key: CeObjectKey,
    set: &mut CeFunctionInputs,
    read: Option<[bool; 4]>,
    fired: impl Fn(&str) -> Option<f32>,
) {
    const UNREAD: &str = "No shader or light on this model reads this.";
    let reaching = read.map_or([true; 4], |read| functions.inputs_reaching(read));
    let read = read.unwrap_or([true; 4]);
    let mut values = functions.inputs_for(set, key);
    ui.push_id(key.1, |ui| {
        let mut rows = CardRows::default();
        // The opened object's go under the section's own title.
        if let Some(name) = name {
            rows.row(ui, |ui| {
                ui.label(RichText::new(format!("{name} Functions")).strong());
            });
        }
        // Each output by its letter, and by what it is used for when the
        // tag says.
        let outputs_named: Vec<String> = functions
            .functions
            .iter()
            .take(OUTPUTS.len())
            .enumerate()
            .map(|(index, function)| match function.usage.trim() {
                "" => OUTPUTS[index].to_owned(),
                usage => format!("{}: {usage}", OUTPUTS[index]),
            })
            .collect();
        let names = functions
            .input_names
            .iter()
            .flatten()
            .map(String::as_str)
            .chain(outputs_named.iter().map(String::as_str));
        let row_width = ui.available_width().max(1.0);
        let name_width = name_column_width(ui, names, f32::MAX).min((row_width - 130.0).max(40.0));
        let before = values;
        for ((value, name), active) in values.iter_mut().zip(&functions.input_names).zip(reaching) {
            let Some(name) = name else {
                continue;
            };
            rows.row(ui, |ui| {
                ui.add_enabled_ui(active, |ui| {
                    name_cell(ui, name, name_width);
                    value_slider(ui, value, true);
                })
                .response
                .on_disabled_hover_text(UNREAD);
            });
        }
        if values != before {
            set.insert(key, values);
        }
        let time = object_time(ui.input(|input| input.time) as f32, key.1);
        let outputs = functions.outputs(&functions.with_fired(values, fired), time);
        for (index, name) in outputs_named.iter().enumerate() {
            // What the function makes: shown, not set.
            let mut shown = outputs[index];
            rows.row(ui, |ui| {
                ui.add_enabled_ui(read[index], |ui| {
                    name_cell(ui, name, name_width);
                    value_slider(ui, &mut shown, false);
                })
                .response
                .on_disabled_hover_text(UNREAD);
            });
        }
    });
}

/// Halo CE's own controls, by sidebar section: the weapon its biped holds,
/// the effects its objects play and fire, and its functions' inputs.
pub(in crate::app::model_preview) const CONTROLS: ObjectControls = ObjectControls {
    weapons: SectionControls {
        has: |data| data.ce.weapons.is_some(),
        draw: draw_ce_weapon_picker,
        // The weapon it holds is the shared section's to reset.
        changed: |_, _| false,
        reset: |_, _| {},
    },
    effects: SectionControls {
        has: |data| {
            !data.preview.ce.firing.is_empty()
                || data
                    .preview
                    .ce
                    .effects
                    .iter()
                    .any(|effect| effect.event.is_some())
        },
        draw: draw_ce_effects,
        changed: |data, state| {
            let geometry = data.geometry_id;
            state.ce.event_scales != super::functions::CE_EVENT_SCALES
                || state.ce.event_counts.keys().any(|(at, _)| *at == geometry)
                || state.ce.shots.keys().any(|(at, _)| *at == geometry)
        },
        reset: |data, state| {
            let geometry = data.geometry_id;
            state.ce.event_scales = super::functions::CE_EVENT_SCALES;
            state.ce.event_counts.retain(|(at, _), _| *at != geometry);
            state.ce.shots.retain(|(at, _), _| *at != geometry);
        },
    },
    functions: SectionControls {
        has: |data| {
            data.ce.functions.is_some()
                || data.ce.attached.iter().any(|held| held.functions.is_some())
        },
        draw: draw_ce_function_inputs,
        changed: |data, state| {
            let geometry = data.geometry_id;
            state
                .ce
                .function_inputs
                .keys()
                .any(|(at, _)| *at == geometry)
        },
        reset: |data, state| {
            let geometry = data.geometry_id;
            state
                .ce
                .function_inputs
                .retain(|(at, _), _| *at != geometry);
        },
    },
};

/// The effects a Halo CE object's events play and its weapon's triggers
/// fire, with the scales they go at.
pub(in crate::app::model_preview) fn draw_ce_effects(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    let held_name = |object: u8| {
        let held = usize::from(object).checked_sub(1)?;
        data.ce.attached.get(held).map(|held| held.name.clone())
    };
    let controls = CeEffectControls {
        effects: &data.preview.ce.effects,
        firing: &data.preview.ce.firing,
        geometry: data.geometry_id,
        counts: &mut state.ce.event_counts,
        shots: &mut state.ce.shots,
        scales: &mut state.ce.event_scales,
        time: frame::preview_clock(ui),
    };
    draw_ce_event_effects(ui, controls, held_name);
}

/// The inputs of the object's functions and of what is attached to it, each marked for the outputs something reads.
pub(in crate::app::model_preview) fn draw_ce_function_inputs(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    let time = frame::preview_clock(ui);
    let read = |object: u8| {
        data.textures.as_ref().map(|textures| {
            let mut read = [false; 4];
            let owned = data.preview.materials.iter().zip(textures.iter());
            let passes = owned
                .filter(|(material, _)| material.object == object)
                .flat_map(|(_, texture)| texture.dx9_layers.iter().flatten());
            for animation in passes.filter_map(|pass| pass.own_animation.as_ref()) {
                for (read, output) in read.iter_mut().zip(animation.outputs_read()) {
                    *read |= output;
                }
            }
            let preview = &data.preview;
            super::lights::mark_outputs_read(&preview.ce.lights, object, &mut read);
            super::functions::mark_outputs_read(&preview.ce.effects, object, &mut read);
            super::functions::mark_outputs_read(&preview.ce.contrails, object, &mut read);
            super::functions::mark_outputs_read(&preview.ce.particle_systems, object, &mut read);
            super::functions::mark_outputs_read(&preview.ce.light_volumes, object, &mut read);
            super::functions::mark_outputs_read(&preview.ce.lightning, object, &mut read);
            super::functions::mark_outputs_read(&preview.ce.glows, object, &mut read);
            read
        })
    };
    let attached = data
        .ce
        .attached
        .iter()
        .enumerate()
        .filter_map(|(index, object)| {
            let functions = object.functions.as_ref()?;
            Some((index as u8 + 1, Some(object.name.as_str()), functions))
        });
    let own = data
        .ce
        .functions
        .iter()
        .map(|functions| (0, None, functions));
    for (object, name, functions) in own.chain(attached) {
        draw_ce_function_controls(
            ui,
            functions,
            name,
            (data.geometry_id, object),
            &mut state.ce.function_inputs,
            read(object),
            super::firing::fired_input(
                &data.preview.ce.firing,
                &state.ce.shots,
                data.geometry_id,
                object,
                time,
            ),
        );
    }
}

/// Which weapon a Halo CE biped holds. Changing it reloads the preview, so
/// what should outlive that goes along with the state.
pub(in crate::app::model_preview) fn draw_ce_weapon_picker(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    let Some(weapons) = data.ce.weapons.as_ref() else {
        return;
    };
    let options: Option<Vec<_>> = weapons.equippable.as_ref().map(|listed| {
        listed
            .iter()
            .map(|weapon| super::super::weapons::WeaponOption {
                path: weapon.path.clone(),
                name: weapon.name.clone(),
                class: weapon.class.clone(),
            })
            .collect()
    });
    let mut rows = CardRows::default();
    let label_width = name_column_width(ui, ["Weapon"], ui.available_width());
    let salt = ("ce_weapon_picker", data.render_model_path.as_str());
    let held = state.weapon.clone();
    let pick = weapon_dropdown(
        ui,
        &mut rows,
        "Weapon",
        label_width,
        salt,
        options.as_deref(),
        held.as_deref(),
    );
    if pick.opened && options.is_none() {
        state.list_weapons = true;
    }
    if let Some(choice) = pick.chosen.filter(|choice| *choice != state.weapon) {
        carry_through_reload(data, state);
        state.weapon = choice;
    }
    let held = state.loaded_weapon.clone();
    draw_held_regions(ui, data, state, &mut rows, held.as_deref());
}
