//! The panel's controls for a Halo CE object's functions and effects.

use crate::app::model_preview::ce::functions::{
    CeEventCounts, CeFunctionInputs, CeObjectFunctions, CeObjectKey, OUTPUTS, object_time,
};
use eframe::egui::{self, Ui};

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
    ui.add_space(6.0);
    egui::CollapsingHeader::new("Effects").id_salt("ce_event_effects").show(ui, |ui| {
        const SCALES: &str = "What effect_new_from_object is given. A firing effect gets its trigger's \
                              rate and error; reloading, readying, damage and the rest get 0 and 0.";
        const FIRE: &str = "Fire the trigger once, as weapon_trigger_fire does: it plays the next \
                            of its firing effects and lights the weapon's inputs, at scale A's \
                            rate of fire. The weapon is held as an actor holds one: it never \
                            overheats.";
        for (scale, name) in scales.iter_mut().zip(["scale A", "scale B"]) {
            ui.add(egui::Slider::new(scale, 0.0..=1.0).text(name)).on_hover_text(SCALES);
        }
        for weapon in firing {
            for (trigger, label) in (0..weapon.triggers.len()).zip(["primary trigger", "secondary trigger"]) {
                if ui.small_button(format!("Fire {}", named(weapon.object, label))).on_hover_text(FIRE).clicked() {
                    let fired = shots.entry((geometry, weapon.object)).or_default();
                    if let Some(effect) = fired.fire(weapon, trigger, scales[0], time) {
                        *counts.entry((geometry, effect)).or_default() += 1;
                    }
                    ui.ctx().request_repaint();
                }
            }
        }
        for (index, name) in events {
            if ui.small_button(format!("Play {name}")).clicked() {
                *counts.entry((geometry, index)).or_default() += 1;
                ui.ctx().request_repaint();
            }
        }
    });
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
    ui.add_space(6.0);
    ui.push_id(key.1, |ui| {
        ui.horizontal(|ui| {
            ui.strong(name.map_or("Object Functions".to_owned(), |name| {
                format!("{name} Functions")
            }));
            let changed = values != functions.inputs;
            if ui
                .add_enabled(changed, egui::Button::new("Reset"))
                .clicked()
            {
                values = functions.inputs;
                set.remove(&key);
            }
        });
        let before = values;
        for ((value, name), active) in values.iter_mut().zip(&functions.input_names).zip(reaching) {
            let Some(name) = name else {
                continue;
            };
            ui.add_enabled(active, egui::Slider::new(value, 0.0..=1.0).text(name))
                .on_disabled_hover_text(UNREAD);
        }
        if values != before {
            set.insert(key, values);
        }
        let time = object_time(ui.input(|input| input.time) as f32, key.1);
        let outputs = functions.outputs(&functions.with_fired(values, fired), time);
        for (index, function) in functions.functions.iter().take(OUTPUTS.len()).enumerate() {
            let text = format!(
                "{}: {:.2}  {}",
                OUTPUTS[index], outputs[index], function.usage
            );
            ui.add_enabled(read[index], egui::Label::new(text))
                .on_disabled_hover_text(UNREAD);
        }
    });
}
