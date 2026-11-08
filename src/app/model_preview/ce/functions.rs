//! Halo CE's object functions: the `A out` to `D out` a shader's animation,
//! fade and meter sources read, for an object at rest in a preview.
//!
//! `object_compute_function_values`, from inputs as the `*_export_function_values`
//! leave them on an undamaged object nothing is using. What it precomputes
//! comes from no code in reach: `rate`, the bounds' default and the step size
//! are as Xbox maps have them baked, against their fields.

use blam_tags::{TagFieldData, TagFile, TagStruct};
use eframe::egui::{self, Ui};

use super::super::common::fields::has_flag;
use super::super::common::time::TICKS_PER_SECOND;
use super::passes::{option_index, periodic_evaluate};

const INPUTS: [&str; 4] = ["A in", "B in", "C in", "D in"];
/// `scale function by` and its like: nothing, four inputs, then four outputs.
const SOURCES: [&str; 9] =
    ["none", "A in", "B in", "C in", "D in", "A out", "B out", "C out", "D out"];
const MAP_TO: [&str; 6] = ["linear", "early", "very early", "late", "very late", "cosine"];
const BOUNDS_MODES: [&str; 3] = ["clip", "clip and normalize", "scale to fit"];
const ACTIVATING_ABOVE: f32 = 0.0001;
const OUTPUTS: [&str; 4] = ["A out", "B out", "C out", "D out"];

/// An input's value on an object at rest, as the `*_export_function_values`
/// give it: vitality and `alive` full, damage and motion nothing; a device as
/// a scenario places one by default (`device_add_scenario_information`:
/// powered, at position 0); a weapon `ready` with its magazines full; `compass`
/// facing north. `random constant` is drawn once per object from the game's
/// seed, which a preview has none of, so a middle.
fn resting_input(name: &str, weapon: Option<TagStruct<'_>>) -> f32 {
    let magazine = |index: usize| {
        let magazines = weapon?.field("magazines")?.as_block()?;
        let rounds = magazines.element(index)?.read_int_any("rounds loaded maximum")?;
        Some(if rounds != 0 { 1.0 } else { 0.0 })
    };
    match name {
        "body vitality" | "shield vitality" | "alive" | "can blink" | "ready" | "power" => 1.0,
        "random constant" | "compass" => 0.5,
        "primary ammunition" => magazine(0).unwrap_or(0.0),
        "secondary ammunition" => magazine(1).unwrap_or(0.0),
        _ => 0.0,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CeFunction {
    invert: bool,
    /// Adds to its last value each tick, wrapping at 1.
    additive: bool,
    /// One over the period, which a 0 leaves at 1.
    rate: f32,
    scale_period_by: usize,
    function: usize,
    scale_function_by: usize,
    wobble_function: usize,
    wobble_rate: f32,
    wobble_magnitude: f32,
    square_threshold: f32,
    steps: i32,
    sawtooth: i32,
    add: usize,
    scale_result_by: usize,
    map_to: usize,
    bounds_mode: usize,
    bounds: [f32; 2],
    scale_by: f32,
    usage: String,
    /// `always active`: it stays on at its lower bound.
    always_active: bool,
    /// `turn off with`: the function whose being off turns this one off.
    turn_off_with: Option<usize>,
}

fn inverse(value: f32) -> f32 {
    if value > 0.0 { 1.0 / value } else { 0.0 }
}

/// `object_compute_function_values`' clock for the object `owner` (0 the
/// opened one, 1 the weapon it holds): the game's time plus 57 ticks per
/// object index, so like objects run out of step. The game uses the object's
/// datum index; the owner stands in for it.
pub(in crate::app::model_preview) fn object_time(time: f32, owner: u8) -> f32 {
    time + 57.0 * f32::from(owner) / TICKS_PER_SECOND
}

impl CeFunction {
    fn read(block: TagStruct<'_>) -> Self {
        let source = |field: &str| option_index(block, field, &SOURCES) as usize;
        let bounds = match block.field("bounds").and_then(|field| field.value()) {
            Some(TagFieldData::FractionBounds(bounds)) => [bounds.lower, bounds.upper],
            _ => [0.0; 2],
        };
        let flagged = |option: &str| has_flag(block, "flags", option);
        let real = |field: &str| block.read_real(field).unwrap_or(0.0);
        Self {
            invert: flagged("invert"),
            additive: flagged("additive"),
            always_active: flagged("always active"),
            turn_off_with: usize::try_from(block.read_int_any("turn off with").unwrap_or(-1)).ok(),
            rate: Some(real("period")).filter(|period| *period > 0.0).map_or(1.0, |period| 1.0 / period),
            scale_period_by: source("scale period by"),
            function: option_index(block, "function", &super::passes::PERIODIC_FUNCTIONS) as usize,
            scale_function_by: source("scale function by"),
            wobble_function: option_index(block, "wobble function", &super::passes::PERIODIC_FUNCTIONS)
                as usize,
            // Tool's postprocess (`sub_4A09A0`) leaves it be, and the game
            // multiplies the time by it.
            wobble_rate: real("wobble period"),
            wobble_magnitude: real("wobble magnitude"),
            square_threshold: real("square wave threshold"),
            steps: block.read_int_any("step count").unwrap_or(0) as i32,
            sawtooth: block.read_int_any("sawtooth count").unwrap_or(0) as i32,
            add: source("add"),
            scale_result_by: source("scale result by"),
            map_to: option_index(block, "map to", &MAP_TO) as usize,
            bounds_mode: option_index(block, "bounds mode", &BOUNDS_MODES) as usize,
            // Tool's postprocess: bounds that meet become nothing to one.
            bounds: if bounds[0] == bounds[1] { [0.0, 1.0] } else { bounds },
            scale_by: real("scale by"),
            usage: block.read_string("usage").unwrap_or_default(),
        }
    }

    #[cfg(test)]
    fn evaluate(&self, values: &[f32; 9], time: f32) -> f32 {
        self.evaluate_active(values, time).0
    }

    /// `object_compute_function_values` for one function: its value, and
    /// whether it is on, which it stops being at its lower bound unless
    /// `always active`.
    fn evaluate_active(&self, values: &[f32; 9], time: f32) -> (f32, bool) {
        let [lower, upper] = self.bounds;
        let mut rate = self.rate;
        if self.scale_period_by != 0 && values[self.scale_period_by] > 0.0 {
            rate /= values[self.scale_period_by];
        }
        let mut value = periodic_evaluate(self.function, rate * time);
        if self.scale_function_by != 0 {
            value *= values[self.scale_function_by];
        }
        if self.invert {
            value = 1.0 - value;
        }
        if self.wobble_magnitude != 0.0 {
            let wobble = periodic_evaluate(self.wobble_function, self.wobble_rate * time);
            value += (wobble - 0.5) * self.wobble_magnitude * 2.0;
        }
        if self.square_threshold != 0.0 {
            value = if value <= self.square_threshold { 0.0 } else { 1.0 };
        }
        if self.steps > 1 {
            value = (self.steps as f32 * value).floor() * inverse((self.steps - 1) as f32);
        }
        // Tool bakes one over one less than the count, nothing for one or less;
        // the game wraps by it where it is something.
        if self.sawtooth > 1 {
            value %= 1.0 / (self.sawtooth - 1) as f32;
        }
        if self.add != 0 {
            value = (value + values[self.add]).min(1.0);
        }
        if self.scale_result_by != 0 {
            value *= values[self.scale_result_by];
        }
        let mut value = super::lights::transition_evaluate(self.map_to as i32, value);
        if self.scale_by > 0.0 {
            value *= self.scale_by;
        }
        let mut active = true;
        if self.bounds_mode == 2 {
            let value = (upper - lower) * value + lower;
            if value <= lower + ACTIVATING_ABOVE {
                active = self.always_active;
            }
            return (value, active);
        }
        if value <= lower + ACTIVATING_ABOVE {
            value = lower;
            active = self.always_active;
        }
        value = value.min(upper);
        if self.bounds_mode == 1 {
            value = (value - lower) * inverse(upper - lower);
        }
        (value, active)
    }

    /// `evaluate`, and for an additive function what its ticks have added up
    /// to, taking the value as steady across them.
    fn output(&self, values: &[f32; 9], time: f32) -> (f32, bool) {
        let (value, active) = self.evaluate_active(values, time);
        (if self.additive { (value * time * TICKS_PER_SECOND).rem_euclid(1.0) } else { value }, active)
    }

    /// The values it reads, as `SOURCES` numbers them.
    fn sources(&self) -> [usize; 4] {
        [self.scale_period_by, self.scale_function_by, self.add, self.scale_result_by]
    }

    fn moves(&self) -> bool {
        const STILL: [usize; 2] = [0, 1];
        self.additive || !STILL.contains(&self.function) || self.wobble_magnitude != 0.0
    }
}

/// What an opened object's functions need.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CeObjectFunctions {
    inputs: [f32; 4],
    /// What each input reads, for those the object uses.
    input_names: [Option<String>; 4],
    functions: Vec<CeFunction>,
}

impl CeObjectFunctions {
    /// The object's `A out` to `D out`, its inputs at `inputs`.
    pub(crate) fn outputs(&self, inputs: &[f32; 4], time: f32) -> [f32; 4] {
        self.outputs_active(inputs, time).0
    }

    /// The outputs, and which are on: the bit `object_get_function_value`
    /// answers with. One a `turn off with` names that is off turns it off;
    /// a later one is read as it stood the tick before, which a still preview
    /// gets by working them out twice. A function the object lacks is off.
    pub(crate) fn outputs_active(&self, inputs: &[f32; 4], time: f32) -> ([f32; 4], [bool; 4]) {
        let mut values = [0.0; 9];
        values[1..5].copy_from_slice(inputs);
        let mut active = [false; 4];
        for _ in 0..2 {
            for (index, function) in self.functions.iter().take(4).enumerate() {
                let (value, mut on) = function.output(&values, time);
                if let Some(other) = function.turn_off_with
                    && !active.get(other).copied().unwrap_or(false)
                {
                    on = false;
                }
                values[5 + index] = value;
                active[index] = on;
            }
        }
        ([values[5], values[6], values[7], values[8]], active)
    }

    pub(crate) fn move_at_rest(&self) -> bool {
        self.functions.iter().take(4).any(CeFunction::moves)
    }

    /// Which inputs can change the outputs marked in `read`, following one
    /// function's reads of another's output.
    fn inputs_reaching(&self, mut read: [bool; 4]) -> [bool; 4] {
        let mut inputs = [false; 4];
        loop {
            let before = read;
            for (index, function) in self.functions.iter().take(4).enumerate() {
                if !read[index] {
                    continue;
                }
                for source in function.sources() {
                    match source {
                        1..=4 => inputs[source - 1] = true,
                        5..=8 => read[source - 5] = true,
                        _ => {}
                    }
                }
            }
            if read == before {
                return inputs;
            }
        }
    }

    /// The inputs to draw with: what the panel set, or rest.
    pub(crate) fn inputs_for(&self, set: &CeFunctionInputs, key: CeObjectKey) -> [f32; 4] {
        set.get(&key).copied().unwrap_or(self.inputs)
    }

    /// Which output a function whose `usage` is `name` makes.
    #[cfg(test)]
    pub(crate) fn output_named(&self, name: &str) -> Option<usize> {
        self.functions.iter().take(OUTPUTS.len()).position(|function| function.usage == name)
    }

    /// `inputs`, each raised to what `fired` gives its name: what the
    /// panel's shots export, over what is set by hand or at rest.
    pub(crate) fn with_fired(&self, mut inputs: [f32; 4], fired: impl Fn(&str) -> Option<f32>) -> [f32; 4] {
        for (value, name) in inputs.iter_mut().zip(&self.input_names) {
            if let Some(shot) = name.as_deref().and_then(&fired) {
                *value = value.max(shot);
            }
        }
        inputs
    }
}

/// A Halo CE feature that reads object outputs (`A out`..`D out`), so its
/// object's panel knows which inputs do something.
pub(in crate::app::model_preview) trait ReadsOutputs {
    /// Whose outputs: 0 the opened object, 1 the weapon it holds.
    fn object(&self) -> u8;
    /// The output slots it reads, 0 to 3.
    fn outputs_read(&self) -> Vec<usize>;
}

/// Sets `read[slot]` for each output `items` of `object` read.
pub(in crate::app::model_preview) fn mark_outputs_read<T: ReadsOutputs>(items: &[T], object: u8, read: &mut [bool; 4]) {
    for item in items.iter().filter(|item| item.object() == object) {
        for slot in item.outputs_read() {
            if let Some(read) = read.get_mut(slot) {
                *read = true;
            }
        }
    }
}

/// A model, and which of its objects.
pub(crate) type CeObjectKey = (u64, u8);
/// Inputs set by hand, each for the object it was set on.
pub(crate) type CeFunctionInputs = std::collections::HashMap<CeObjectKey, [f32; 4]>;

/// Plays of the event effects, by model and index among its effects.
pub(crate) type CeEventCounts = std::collections::HashMap<(u64, usize), u32>;

/// What a held trigger gives `weapon_effect_new` for its firing effect: its
/// rate, which climbs to 1 while held, and its error, none on a cool weapon.
/// Reloading, readying, damage and the rest give 0 and 0.
pub(crate) const CE_EVENT_SCALES: [f32; 2] = [1.0, 0.0];

/// What the Effects section plays and fires, and where it keeps track.
pub(in crate::app::model_preview) struct CeEffectControls<'a> {
    pub(in crate::app::model_preview) effects: &'a [super::particles::CeEffectAt],
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
pub(in crate::app::model_preview) fn draw_ce_event_effects(ui: &mut Ui, controls: CeEffectControls<'_>, object_name: impl Fn(u8) -> Option<String>) {
    let CeEffectControls { effects, firing, geometry, counts, shots, scales, time } = controls;
    let named = |object: u8, name: &str| object_name(object).map_or_else(|| name.to_owned(), |object| format!("{object}: {name}"));
    let events: Vec<(usize, String)> = effects
        .iter()
        .enumerate()
        .filter_map(|(index, effect)| Some((index, named(effect.object, &effect.event.as_ref()?.name))))
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
        ui.strong(name.map_or("Object Functions".to_owned(), |name| format!("{name} Functions")));
        let changed = values != functions.inputs;
        if ui.add_enabled(changed, egui::Button::new("Reset")).clicked() {
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
        let text = format!("{}: {:.2}  {}", OUTPUTS[index], outputs[index], function.usage);
        ui.add_enabled(read[index], egui::Label::new(text)).on_disabled_hover_text(UNREAD);
    }
    });
}

/// An object's chain of parent structs, ending at `object`.
fn struct_chain(object: &TagFile) -> Option<Vec<TagStruct<'_>>> {
    let mut chain = vec![object.root()];
    loop {
        let last = *chain.last()?;
        if last.field("functions").is_some() {
            return Some(chain);
        }
        chain.push(last.fields_all().find_map(|field| field.as_struct())?);
    }
}

/// The shader `render_object_list` lays over the object: group and path.
pub(in crate::app) fn read_ce_modifier_shader(object: &TagFile) -> Option<(u32, String)> {
    let (group, path) = struct_chain(object)?.last()?.read_tag_ref_with_group("modifier shader")?;
    (!path.is_empty()).then_some((group, path))
}

/// The bitmap image the object makes every shader draw, 0 for each its own.
pub(in crate::app) fn read_ce_forced_permutation(object: &TagFile) -> usize {
    struct_chain(object)
        .and_then(|chain| chain.last()?.read_int_any("forced shader permutation index"))
        .unwrap_or(0)
        .max(0) as usize
}

/// An object's functions, reading its chain of parent structs from the object
/// up: a type's inputs replace those of the groups beneath it.
pub(in crate::app) fn read_ce_object_functions(object: &TagFile) -> Option<CeObjectFunctions> {
    let chain = struct_chain(object)?;
    let mut inputs = [0.0; 4];
    let mut input_names: [Option<String>; 4] = Default::default();
    let weapon = chain.iter().copied().find(|level| level.field("magazines").is_some());
    for level in chain.iter().rev() {
        for (index, name) in INPUTS.into_iter().enumerate() {
            let Some(option) = level.read_enum_name(name).filter(|option| option != "none") else {
                continue;
            };
            inputs[index] = resting_input(&option, weapon);
            input_names[index] = Some(option);
        }
    }
    let block = chain.last()?.field("functions")?.as_block()?;
    let functions = block.iter().map(CeFunction::read);
    Some(CeObjectFunctions { inputs, input_names, functions: functions.collect() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::harness::kit::kit_source;
    use crate::core::game::GameId;
    use crate::app::model_preview::load_referenced_tag_from_source;

    /// With `BLAM_TEST_HCEEK`: a function's `invert` is read by the option's
    /// name as shown, though the tag stores it as `invert#…`. Matched raw, it
    /// never read, and 17 of the kit's functions lost their inversion.
    #[test]
    fn kit_functions_read_their_invert_flag() {
        let Some(source) = kit_source(GameId::HaloCe) else { return };
        let tag = load_referenced_tag_from_source(&source, "weapons\\plasma rifle\\plasma rifle", "weapon", b"weap")
            .unwrap();
        let functions = read_ce_object_functions(&tag).unwrap();
        assert!(functions.functions.iter().any(|function| function.invert), "the plasma rifle inverts a function");
        assert!(functions.functions.iter().any(|function| !function.invert));
    }

    /// With `BLAM_TEST_HCEEK`: real objects at rest, by what their tags say.
    #[test]
    fn kit_objects_rest_where_their_inputs_put_them() {
        let Some(source) = kit_source(GameId::HaloCe) else { return };
        let biped = |path: &str| {
            let tag = load_referenced_tag_from_source(&source, path, "biped", b"bipd").unwrap();
            read_ce_object_functions(&tag).unwrap()
        };
        let rest = |objects: &CeObjectFunctions, time| objects.outputs(&objects.inputs, time);
        let tag = load_referenced_tag_from_source(&source, "characters\\cyborg\\cyborg", "biped", b"bipd")
            .unwrap();
        // Only the shield hit reads an output, the second, off one input.
        let chief = read_ce_object_functions(&tag).unwrap();
        assert_eq!(chief.inputs_reaching([false, true, false, false]), [true, false, false, false]);
        let (group, path) = read_ce_modifier_shader(&tag).unwrap();
        assert_eq!(group, u32::from_be_bytes(*b"spla"));
        assert!(path.ends_with("shield hit"), "{path}");
        // `recent shield damage` drives the glow a shield hit's plasma reads.
        let cyborg = biped("characters\\cyborg\\cyborg");
        assert_eq!(rest(&cyborg, 1.0)[1], 0.0);
        assert_eq!(cyborg.input_names[0].as_deref(), Some("recent shield damage"));
        assert_eq!(cyborg.functions[1].usage, "shield glow source");
        // `transition_function_evaluate` reads 1.0 from its table's entry
        // 1022, a byte short of full.
        assert!((cyborg.outputs(&[1.0, 1.0, 0.0, 0.0], 1.0)[1] - 254.0 / 255.0).abs() < 1e-6);
        assert_eq!(rest(&biped("characters\\elite\\elite"), 1.0)[0], 0.0);
        // The sentinel's last function runs off `alive`.
        assert_eq!(rest(&biped("characters\\sentinel\\sentinel"), 1.0)[3], 1.0);
        // The monitor's idle animation has no input to hold it still.
        let monitor = biped("characters\\monitor\\monitor");
        assert!(monitor.move_at_rest());
        assert_ne!(rest(&monitor, 1.0)[2], rest(&monitor, 2.0)[2]);
    }

    fn function(function: usize) -> CeFunction {
        CeFunction {
            invert: false,
            additive: false,
            rate: 1.0,
            scale_period_by: 0,
            function,
            scale_function_by: 0,
            wobble_function: 0,
            wobble_rate: 0.0,
            wobble_magnitude: 0.0,
            square_threshold: 0.0,
            steps: 0,
            sawtooth: 0,
            add: 0,
            scale_result_by: 0,
            map_to: 0,
            bounds_mode: 0,
            bounds: [0.0, 1.0],
            scale_by: 0.0,
            usage: String::new(),
            always_active: false,
            turn_off_with: None,
        }
    }

    #[test]
    fn a_shield_glow_rests_at_nothing_and_rises_with_its_input() {
        let glow = CeFunction { scale_function_by: 1, map_to: 2, ..function(0) };
        let objects = CeObjectFunctions { functions: vec![glow], ..Default::default() };
        assert_eq!(objects.outputs(&[0.0; 4], 3.0)[0], 0.0);
        // `transition_function_evaluate` reads a byte table: within a step of it.
        assert!((objects.outputs(&[0.5, 0.0, 0.0, 0.0], 3.0)[0] - 0.5f32.powf(0.25)).abs() < 1.0 / 255.0);
    }

    #[test]
    fn a_function_goes_off_at_its_lower_bound_or_with_the_one_it_names() {
        let input = CeFunction { scale_function_by: 1, ..function(0) };
        let lasting = CeFunction { always_active: true, ..input.clone() };
        let follower = CeFunction { turn_off_with: Some(0), ..function(0) };
        let objects = CeObjectFunctions { functions: vec![input, lasting, follower], ..Default::default() };
        let (_, rested) = objects.outputs_active(&[0.0; 4], 0.0);
        assert_eq!(rested, [false, true, false, false]);
        let (_, raised) = objects.outputs_active(&[0.5, 0.0, 0.0, 0.0], 0.0);
        assert_eq!(raised, [true, true, true, false]);
    }

    #[test]
    fn a_sawtooth_wraps_at_one_over_one_less_than_its_count() {
        let values = [0.0, 0.7, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let input = CeFunction { scale_function_by: 1, ..function(0) };
        let three = CeFunction { sawtooth: 3, ..input.clone() };
        assert!((three.evaluate(&values, 0.0) - 0.2).abs() < 1e-5);
        // One tooth is no sawtooth at all.
        let one = CeFunction { sawtooth: 1, ..input };
        assert!((one.evaluate(&values, 0.0) - 0.7).abs() < 1e-5);
    }

    #[test]
    fn a_later_function_can_read_an_earlier_output() {
        let first = CeFunction { scale_function_by: 1, ..function(0) };
        let second = CeFunction { scale_function_by: 5, invert: true, ..function(0) };
        let objects = CeObjectFunctions { functions: vec![first, second], ..Default::default() };
        assert_eq!(objects.outputs(&[1.0, 0.0, 0.0, 0.0], 0.0)[..2], [1.0, 0.0]);
    }

    #[test]
    fn square_steps_and_normalized_bounds_shape_the_value() {
        let values = [0.0, 0.7, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let input = CeFunction { scale_function_by: 1, ..function(0) };
        let square = CeFunction { square_threshold: 0.5, ..input.clone() };
        assert_eq!(square.evaluate(&values, 0.0), 1.0);
        let steps = CeFunction { steps: 3, ..input.clone() };
        assert_eq!(steps.evaluate(&values, 0.0), 1.0);
        let normalized = CeFunction { bounds_mode: 1, bounds: [0.5, 1.0], ..input };
        assert!((normalized.evaluate(&values, 0.0) - 0.4).abs() < 1e-5);
    }

    /// The crewman's `blinking`, as an Xbox map bakes it: bounds 0.8 to 1 that
    /// normalize, so its spark only shows at the top of each cycle.
    #[test]
    fn a_blink_rests_shut_off_and_peaks_at_one() {
        let blink = CeFunction {
            scale_function_by: 1,
            bounds_mode: 1,
            bounds: [0.8, 1.0],
            ..function(11)
        };
        let values = [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(blink.evaluate(&values, 0.0), 0.0);
        let top = (0..4000).map(|step| blink.evaluate(&values, step as f32 * 0.01)).fold(0.0, f32::max);
        assert!(top > 0.9, "{top}");
    }

    #[test]
    fn an_additive_function_wraps_as_its_ticks_add_up() {
        let values = [0.0, 0.25, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let adding = CeFunction { additive: true, scale_function_by: 1, ..function(0) };
        assert!((adding.output(&values, 1.0 / 30.0).0 - 0.25).abs() < 1e-4);
        assert!((adding.output(&values, 3.0 / 30.0).0 - 0.75).abs() < 1e-4);
        assert!(adding.output(&values, 5.0 / 30.0).0 < 0.26);
        assert!(CeFunction { additive: true, ..function(0) }.moves());
    }

    #[test]
    fn inputs_set_on_one_object_do_not_carry_to_another() {
        let objects = CeObjectFunctions { inputs: [1.0, 0.0, 0.0, 0.0], ..Default::default() };
        let set = CeFunctionInputs::from([((7, 0), [0.25; 4])]);
        assert_eq!(objects.inputs_for(&set, (7, 0)), [0.25; 4]);
        assert_eq!(objects.inputs_for(&set, (8, 0)), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(objects.inputs_for(&set, (7, 1)), [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn an_input_reaches_a_read_output_through_another_output() {
        let first = CeFunction { scale_function_by: 2, ..function(0) };
        let second = CeFunction { add: 5, ..function(0) };
        let third = CeFunction { scale_result_by: 3, ..function(0) };
        let objects =
            CeObjectFunctions { functions: vec![first, second, third], ..Default::default() };
        assert_eq!(objects.inputs_reaching([false, true, false, false]), [false, true, false, false]);
        assert_eq!(objects.inputs_reaching([false, false, true, false]), [false, false, true, false]);
        assert_eq!(objects.inputs_reaching([false; 4]), [false; 4]);
    }

    #[test]
    fn only_a_running_function_moves() {
        let objects = CeObjectFunctions { functions: vec![function(0)], ..Default::default() };
        assert!(!objects.move_at_rest());
        let objects = CeObjectFunctions { functions: vec![function(6)], ..Default::default() };
        assert!(objects.move_at_rest());
    }
}

