//! Halo 2's object functions: what its shaders and attachments read by name.
//!
//! An object's `functions` block turns the values its type exports into named
//! outputs. In a preview nothing runs, so the panel sets those inputs, and
//! each starts where an idle object leaves it.

use std::collections::HashMap;
use std::sync::Arc;

use blam_tags::tag_function::FunctionType;
use blam_tags::{TagFile, TagFunction, TagStruct};
use eframe::egui::{self, Ui};

use crate::app::ui_kit::ThemedCheckbox;

use super::super::ce::attachments::{elements, object_block};
use super::super::dx9::passes::{object_input, tag_function_at};

/// What the panel has set, by object geometry and input name.
pub(crate) type H2FunctionInputs = HashMap<(u64, String), f32>;
/// Every name an object answers to this frame.
pub(crate) type H2FunctionValues = HashMap<String, f32>;

/// A function's `flags`, by option name.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct FunctionFlags {
    invert: bool,
    /// `mapping does not controls active`: an inactive input passes through
    /// unmapped, and the curve's value does not decide whether it is on.
    mapping_does_not_control_active: bool,
    always_active: bool,
    /// `random time offset`: a periodic function runs ahead by a fraction of
    /// a second that differs per object.
    random_time_offset: bool,
    /// `always exports value`: off, it still exports its value.
    always_exports_value: bool,
    /// `turn off with uses magnitude`: the `turn off with` function must also
    /// be above 0.
    turn_off_with_uses_magnitude: bool,
}

impl FunctionFlags {
    fn read(entry: TagStruct<'_>) -> Self {
        let names = entry.read_flag_names("flags").unwrap_or_default();
        // Option names carry their explanation after a `#`.
        let set = |wanted: &str| names.iter().any(|(_, name)| name.split('#').next() == Some(wanted));
        Self {
            invert: set("invert"),
            mapping_does_not_control_active: set("mapping does not controls active"),
            always_active: set("always active"),
            random_time_offset: set("random time offset"),
            always_exports_value: set("always exports value"),
            turn_off_with_uses_magnitude: set("turn off with uses magnitude"),
        }
    }
}

/// The time functions run on, and the seconds a `random time offset` one
/// runs ahead for this object: `object_function_evaluate` hashes the object's
/// handle, `((1664525 * object + 1013904223) >> 16) / 65535`. The preview's
/// owner (0 the opened object) stands in for the handle.
#[derive(Debug, Clone, Copy, Default)]
struct Clock {
    time: f32,
    offset: f32,
}

impl Clock {
    fn of(time: f32, owner: u8) -> Self {
        let hashed = u32::from(owner).wrapping_mul(1_664_525).wrapping_add(1_013_904_223) >> 16;
        Self { time, offset: hashed as f32 / 65_535.0 }
    }
}
/// How far `scale by` can chain before it has to be a loop.
const DEEPEST_CHAIN: usize = 8;

#[derive(Debug, Clone)]
struct H2ObjectFunction {
    flags: FunctionFlags,
    import: String,
    export: String,
    turn_off_with: String,
    /// Reach's `ranged interpolation name`: what feeds the curve's range.
    range: String,
    min_value: f32,
    function: Arc<TagFunction>,
    scale_by: String,
}

/// Whose `object_get_function_value` an object answers by. They agree but for
/// which function of a name answers and how `turn off with` is read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum FunctionRules {
    /// Halo 2's `sub_86F2F0` and `sub_86ECD0`: the last function of a name
    /// stands, and `turn off with` reads what the type exports.
    #[default]
    Halo2,
    /// Halo 3 (`halo3.dll` `sub_1803463B0`, `sub_180346118`) and Reach (the
    /// 360's `object_get_function_value` 0x82b4b2c0 and
    /// `object_function_get_function_value` 0x82b4b908): the first function
    /// of a name answers, and `turn off with` is looked up in full. ODST's
    /// are not read; it sits between two that agree.
    Gen3,
}

/// One of Halo 4's `function switches`: an export that answers with whichever
/// of its functions the switch function's value falls between the bounds of.
/// The 360's `ObjectFunctionSwitchDefinition::Evaluate` 0x84af0220.
#[derive(Debug, Clone)]
struct FunctionSwitch {
    switch: String,
    export: String,
    /// `selection bounds`, inclusive at both ends, and the function picked.
    cases: Vec<([f32; 2], String)>,
}

impl FunctionSwitch {
    /// The first case whose bounds hold the switch's value answers; with
    /// none, the export is 0 and off.
    fn evaluate(
        &self,
        owner: &H2ObjectFunctions,
        inputs: &H2FunctionValues,
        scripted: &H2FunctionValues,
        clock: Clock,
        depth: usize,
    ) -> (f32, bool) {
        let (switch, _) = owner.value(&self.switch, inputs, scripted, clock, depth + 1);
        let picked = self.cases.iter().find(|([lower, upper], _)| *lower <= switch && switch <= *upper);
        match picked {
            Some((_, function)) => owner.value(function, inputs, scripted, clock, depth + 1),
            None => (0.0, false),
        }
    }
}

/// An object's `functions` block, and what its type exports to it.
#[derive(Debug, Clone, Default)]
pub(crate) struct H2ObjectFunctions {
    functions: Vec<H2ObjectFunction>,
    /// Halo 4's `function switches`; empty for every earlier game.
    switches: Vec<FunctionSwitch>,
    rules: FunctionRules,
    /// None for an object whose exporters are not known (Halo 3 and ODST),
    /// which reads [`object_input`] instead.
    exports: Option<TypeExports>,
}

/// One of Halo 2's function exporters: the names it claims, with what an idle
/// object, just placed, gives each. MCC `halo2_tag_test`; the object one is
/// `sub_86A360`, and every type asks it first.
type ExportTable = &'static [(&'static str, f32)];

/// Every object's exporter. `body_vitality`, `shield_vitality`,
/// `shield_depleted` and `variant` depend on the model; see
/// [`TypeExports::idle`]. `compass`, `random_constant`, the clock and the
/// team, medic and holiday names depend on a game the preview has not got,
/// and stay 0.
const OBJECT_EXPORTS: ExportTable = &[
    ("one", 1.0),
    ("zero", 0.0),
    ("morse_code", 1.0),
    ("alive", 1.0),
    ("body_vitality", 1.0),
    ("shield_vitality", 1.0),
    ("current_body_damage", 0.0),
    ("current_shield_damage", 0.0),
    ("object_overshield_amount", 0.0),
    ("shield_depleted", 0.0),
    ("electrical_power", 1.0),
    ("variant", 0.0),
    ("compass", 0.0),
    ("random_constant", 0.0),
    ("time_hours", 0.0),
    ("time_minutes", 0.0),
    ("time_seconds", 0.0),
    ("holiday", 0.0),
    ("all_quiet", 1.0),
    ("flashlights_allowed", 0.0),
    ("medic", 0.0),
    ("team0", 0.0),
    ("team1", 0.0),
    ("team2", 0.0),
    ("team3", 0.0),
    ("team4", 0.0),
    ("team5", 0.0),
    ("team6", 0.0),
    ("team7", 0.0),
    ("scripted_object_function_a", 0.0),
    ("scripted_object_function_b", 0.0),
    ("scripted_object_function_c", 0.0),
    ("scripted_object_function_d", 0.0),
    ("emblem_active", 1.0),
    ("emblem_foreground_index", 0.0),
    ("emblem_background_index", 0.0),
    ("emblem_toggle_foreground", 1.0),
    ("emblem_foreground_map_scale_x", 0.0),
    ("emblem_background_map_scale_x", 0.0),
];

/// A unit's, biped's and vehicle's, `sub_882EB0`. `barrel_spin` and
/// `illumination` it passes to the weapon held, and claims only with one in
/// hand; the held weapon's own functions answer for it here.
const UNIT_EXPORTS: ExportTable = &[
    ("driver_seat_occupied", 0.0),
    ("gunner_seat_occupied", 0.0),
    ("driver_seat_power", 0.0),
    ("gunner_seat_power", 0.0),
    ("active_camouflage", 0.0),
    ("can_blink", 1.0),
    ("crouch", 0.0),
    ("mouth_aperture", 0.0),
    ("vertical_thrust", 0.0),
    ("integrated_light_power", 0.0),
    ("aiming_change", 0.0),
    ("horizontal_aiming_change", 0.0),
    ("boost", 0.0),
    ("unit_open", 1.0),
    ("unit_closed", 0.0),
];

/// A biped's, `sub_8AE0E0`. `stunned` it claims only while stunned, and
/// `flying_speed` only for a biped that flies.
const BIPED_EXPORTS: ExportTable = &[("flight", 0.0)];

/// A vehicle's, `sub_8D0B60`: all at rest.
const VEHICLE_EXPORTS: ExportTable = &[
    ("speed_absolute", 0.0),
    ("speed_forward", 0.0),
    ("speed_backward", 0.0),
    ("speed_slide_maximum", 0.0),
    ("slide_left", 0.0),
    ("slide_right", 0.0),
    ("slide_absolute", 0.0),
    ("slide_back", 0.0),
    ("turn_left", 0.0),
    ("turn_right", 0.0),
    ("turn_absolute", 0.0),
    ("velocity_forward", 0.0),
    ("velocity_left", 0.0),
    ("velocity_up", 0.0),
    ("velocity_air", 0.0),
    ("velocity_ground", 0.0),
    ("velocity_water", 0.0),
    ("wingtip_contrail", 0.0),
    ("wingtip_contrail_new", 0.0),
    ("front_left_tire_position", 0.0),
    ("front_left_tire_velocity", 0.0),
    ("front_right_tire_position", 0.0),
    ("front_right_tire_velocity", 0.0),
    ("back_left_tire_position", 0.0),
    ("back_left_tire_velocity", 0.0),
    ("back_right_tire_position", 0.0),
    ("back_right_tire_velocity", 0.0),
    ("left_tread_position", 0.0),
    ("left_tread_velocity", 0.0),
    ("right_tread_position", 0.0),
    ("right_tread_velocity", 0.0),
    ("tread_grind", 0.0),
    ("engine_rpm", 0.0),
    ("engine_power", 0.0),
    ("engine_hack", 0.0),
    ("jump", 0.0),
    ("emitting", 0.0),
    ("hover", 0.0),
    ("thrust", 0.0),
    ("mean_antigrav", 0.0),
];

/// A weapon's, `sub_8A0000`: new, full and not yet in a hand, so turned on.
/// The preview counts no rounds, so the ones and tens of the count stay 0.
const WEAPON_EXPORTS: ExportTable = &[
    ("age", 0.0),
    ("heat", 0.0),
    ("overheated", 0.0),
    ("barrel_spin", 0.0),
    ("power", 0.0),
    ("ready", 0.0),
    ("turned_on", 1.0),
    ("primary_ammunition", 1.0),
    ("secondary_ammunition", 1.0),
    ("primary_ammunition_ones", 0.0),
    ("primary_ammunition_tens", 0.0),
    ("primary_trigger_down", 0.0),
    ("secondary_trigger_down", 0.0),
    ("primary_firing", 0.0),
    ("secondary_firing", 0.0),
    ("primary_charged", 0.0),
    ("secondary_charged", 0.0),
    ("primary_rate_of_fire", 0.0),
    ("secondary_rate_of_fire", 0.0),
    ("primary_ejection_port", 0.0),
    ("secondary_ejection_port", 0.0),
    ("illumination", 0.0),
    ("warthog_chaingun_illumination", 0.0),
    ("active_camouflage", 0.0),
    ("bomb_arming_amount", 0.0),
    ("bomb_is_armed", 0.0),
    ("bomb_is_arming", 0.0),
    ("bomb_is_unarmed", 0.0),
];

/// A projectile's, `sub_8F72F0`, as it is made.
const PROJECTILE_EXPORTS: ExportTable = &[
    ("tracer", 1.0),
    ("projectile_attach", 0.0),
    ("acceleration_range", 1.0),
    ("range_remaining", 0.0),
    ("time_remaining", 0.0),
];

/// A device's (machine, control, light fixture), `sub_8A83F0`, as one placed
/// with neither `initially off` nor `initially open` would be.
const DEVICE_EXPORTS: ExportTable = &[
    ("power", 1.0),
    ("position", 0.0),
    ("change_in_power", 0.0),
    ("change_in_position", 0.0),
    ("loop", 1.0),
    ("delay", 0.0),
    ("locked", 0.0),
];

/// What a Halo 2 object's type exports to its functions: its exporters, in
/// the order `sub_8D8EC0` asks them, and what of its model decides the
/// vitality and variant names.
#[derive(Debug, Clone, Copy)]
struct TypeExports {
    tables: &'static [ExportTable],
    has_body: bool,
    has_shield: bool,
    variants: usize,
}

impl TypeExports {
    /// For a Halo 2 `object` previewing `model`; none for another game.
    fn of(object: &TagFile, model: Option<&TagFile>) -> Option<Self> {
        if blam_tags::game::Game::of(object) != blam_tags::game::Game::Halo2 {
            return None;
        }
        let tables: &'static [ExportTable] = match &object.header.group_tag.to_be_bytes() {
            b"bipd" => &[OBJECT_EXPORTS, UNIT_EXPORTS, BIPED_EXPORTS],
            b"vehi" => &[OBJECT_EXPORTS, UNIT_EXPORTS, VEHICLE_EXPORTS],
            b"weap" => &[OBJECT_EXPORTS, WEAPON_EXPORTS],
            b"proj" => &[OBJECT_EXPORTS, PROJECTILE_EXPORTS],
            b"mach" | b"ctrl" | b"lifi" => &[OBJECT_EXPORTS, DEVICE_EXPORTS],
            _ => &[OBJECT_EXPORTS],
        };
        let damage = model.and_then(|model| elements(model.root(), "new damage info").first().copied());
        let vitality = |field: &str| damage.and_then(|damage| damage.read_real(field)).unwrap_or(0.0);
        Some(Self {
            tables,
            has_body: vitality("maximum vitality") > 0.0,
            has_shield: vitality("maximum shield vitality") > 0.0,
            variants: model.map_or(0, |model| elements(model.root(), "variants").len()),
        })
    }

    /// What an idle object of this type gives `name`, if an exporter claims it.
    /// The vitalities are full where the model has them; the variant is its
    /// first of `variants`, `(index + 1) / count`.
    fn idle(&self, name: &str) -> Option<f32> {
        let model_says = match name {
            "body_vitality" => Some(f32::from(u8::from(self.has_body))),
            "shield_vitality" => Some(f32::from(u8::from(self.has_shield))),
            "shield_depleted" => Some(f32::from(u8::from(!self.has_shield))),
            "variant" => Some(if self.variants > 0 { 1.0 / self.variants as f32 } else { 0.0 }),
            _ => None,
        };
        let claimed = self.tables.iter().flat_map(|table| table.iter()).find(|(export, _)| *export == name);
        claimed.map(|(_, idle)| model_says.unwrap_or(*idle))
    }
}

impl H2ObjectFunction {
    fn is_periodic(&self) -> bool {
        self.function.function_type() == FunctionType::Periodic
    }

    /// `sub_86ECD0`: the value and whether it is active.
    fn evaluate(
        &self,
        owner: &H2ObjectFunctions,
        inputs: &H2FunctionValues,
        scripted: &H2FunctionValues,
        clock: Clock,
        depth: usize,
    ) -> (f32, bool) {
        // A script's value for the import is the function's, unmapped.
        if let Some(value) = scripted.get(&self.import) {
            return (*value, true);
        }
        let (input, mut active) = owner.exported(&self.import, inputs);
        // No name reads full, which is what an object with no such field gets.
        let range = if self.range.is_empty() { 1.0 } else { owner.value(&self.range, inputs, scripted, clock, depth + 1).0 };
        let mapping_passes = self.flags.mapping_does_not_control_active;
        let mut value;
        if self.is_periodic() {
            let time = if self.flags.random_time_offset { clock.time + clock.offset } else { clock.time };
            value = self.function.evaluate(time, range) * input;
        } else if !active && mapping_passes {
            value = input;
        } else {
            let inverted = if self.flags.invert { 1.0 - input } else { input };
            value = self.function.evaluate(inverted, range);
            if !mapping_passes {
                active = value > 0.0;
            }
        }
        if !self.scale_by.is_empty() {
            let (scale, scaled) = owner.value(&self.scale_by, inputs, scripted, clock, depth + 1);
            value *= scale;
            active = active && scaled;
        }
        if active && self.min_value > 0.0 {
            active = value > self.min_value;
        }
        if self.flags.always_active {
            active = true;
        }
        if active && !self.turn_off_with.is_empty() {
            active = match owner.rules {
                FunctionRules::Halo2 => owner.exported(&self.turn_off_with, inputs).1,
                FunctionRules::Gen3 => {
                    let (magnitude, on) = owner.value(&self.turn_off_with, inputs, scripted, clock, depth + 1);
                    on && (!self.flags.turn_off_with_uses_magnitude || magnitude != 0.0)
                }
            };
        }
        match (active, self.flags.always_exports_value) {
            (true, _) => (value.clamp(0.0, 1.0), true),
            (false, true) => (value.clamp(0.0, 1.0), false),
            (false, false) => (0.0, false),
        }
    }
}

impl H2ObjectFunctions {
    /// What an idle object gives `name`: its type's exporter's value, or,
    /// where those are not known, [`object_input`]'s.
    fn idle(&self, name: &str) -> f32 {
        match &self.exports {
            Some(exports) => exports.idle(name).unwrap_or(0.0),
            None => object_input(name),
        }
    }

    /// Whether the object's type exports `name`, so its exporter answers for
    /// it before any function of the same name.
    fn type_exports(&self, name: &str) -> bool {
        self.exports.as_ref().is_some_and(|exports| exports.idle(name).is_some())
    }

    /// What `object_type_export_function_values` leaves for `name`: the
    /// panel's value, else the idle one, and whether that is on.
    fn exported(&self, name: &str, inputs: &H2FunctionValues) -> (f32, bool) {
        let value = inputs.get(name).copied().unwrap_or_else(|| self.idle(name));
        (value, value > 0.0)
    }

    /// Whether a function here makes `name` for what reads it: one of that
    /// name, and none of the type's exporters claiming it first.
    fn makes(&self, name: &str) -> bool {
        !self.type_exports(name) && self.exports_named().any(|export| export == name)
    }

    /// Every name a function or switch here exports.
    fn exports_named(&self) -> impl Iterator<Item = &str> {
        let functions = self.functions.iter().map(|function| function.export.as_str());
        functions.chain(self.switches.iter().map(|switch| switch.export.as_str()))
    }

    /// What `name` is made from here: a function's import, scale and
    /// `turn off with`, or a switch's switch function and cases.
    fn sources_of<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        let functions = self.functions.iter().filter(move |function| function.export == name);
        let functions = functions.flat_map(|function| [&function.import, &function.scale_by, &function.turn_off_with]);
        let switches = self.switches.iter().filter(move |switch| switch.export == name);
        let switches = switches.flat_map(|switch| {
            std::iter::once(&switch.switch).chain(switch.cases.iter().map(|(_, function)| function))
        });
        functions.chain(switches).map(String::as_str).filter(|source| !source.is_empty())
    }

    /// `object_get_function_value`: a script's value first
    /// (`object_set_function_variable`, read by Halo 2's `sub_8E7890`), then
    /// nothing named reads full, then what the object's type exports, then
    /// its functions: by [`FunctionRules`], the last or the first of a name.
    /// Halo 4 (the 360's 0x82a92870) asks its switches only when no function
    /// has the name, and every switch of the name runs, so the last stands.
    fn value(
        &self,
        name: &str,
        inputs: &H2FunctionValues,
        scripted: &H2FunctionValues,
        clock: Clock,
        depth: usize,
    ) -> (f32, bool) {
        if let Some(value) = scripted.get(name) {
            return (*value, true);
        }
        if name.is_empty() || name == "one" {
            return (1.0, true);
        }
        if name == "zero" || depth > DEEPEST_CHAIN {
            return (0.0, false);
        }
        if self.type_exports(name) {
            return self.exported(name, inputs);
        }
        let named = |function: &&H2ObjectFunction| function.export == name;
        let answers = match self.rules {
            FunctionRules::Halo2 => self.functions.iter().rev().find(named),
            FunctionRules::Gen3 => self.functions.iter().find(named),
        };
        if let Some(function) = answers {
            return function.evaluate(self, inputs, scripted, clock, depth);
        }
        match self.switches.iter().rev().find(|switch| switch.export == name) {
            Some(switch) => switch.evaluate(self, inputs, scripted, clock, depth),
            None => self.exported(name, inputs),
        }
    }

    /// The names the panel can set: what a function reads, and what scales or
    /// turns it off, unless another function here makes that.
    fn imports(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for export in self.exports_named() {
            for name in self.sources_of(export) {
                if !self.makes(name) && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        names
    }

    /// Everything `read` depends on, down through the functions making it.
    fn reaching(&self, read: &[String]) -> Vec<String> {
        let mut reached = read.to_vec();
        let mut next = 0;
        while let Some(name) = reached.get(next).cloned() {
            next += 1;
            for source in self.sources_of(&name) {
                if !reached.iter().any(|reached| reached == source) {
                    reached.push(source.to_owned());
                }
            }
        }
        reached
    }

    /// Whether anything here moves with no one touching it.
    pub(crate) fn move_at_rest(&self, inputs: &H2FunctionValues) -> bool {
        self.functions.iter().any(|function| {
            function.is_periodic() && self.exported(&function.import, inputs).0 > 0.0
        })
    }

    /// Every name this frame, for what reads them.
    /// `owner` is 0 for the opened object, else the attached one's number.
    pub(crate) fn values(
        &self,
        inputs: &H2FunctionInputs,
        geometry_id: u64,
        extra: &[String],
        time: f32,
        owner: u8,
    ) -> H2FunctionValues {
        let clock = Clock::of(time, owner);
        let mut set = H2FunctionValues::new();
        let names = self.imports().into_iter().map(str::to_owned).chain(extra.iter().cloned());
        for name in names {
            let value = inputs.get(&(geometry_id, name.clone())).copied().unwrap_or_else(|| self.idle(&name));
            set.insert(name, value);
        }
        // What the panel sets of a name a function makes is a script's value.
        let scripted: H2FunctionValues = self
            .exports_named()
            .filter_map(|export| {
                let value = inputs.get(&(geometry_id, export.to_owned()))?;
                Some((export.to_owned(), *value))
            })
            .collect();
        let mut values = set.clone();
        for export in self.exports_named() {
            let (value, _) = self.value(export, &set, &scripted, clock, 0);
            values.insert(export.to_owned(), value);
        }
        values
    }
}

/// An object's functions, or none on a tag with no such block. An empty block
/// still counts: its shaders can read what the object's type exports. `model`
/// is the object's, whose damage info and variants some exports report.
pub(crate) fn read_h2_object_functions(object: &TagFile, model: Option<&TagFile>) -> Option<H2ObjectFunctions> {
    let block = object_block(object, "functions")?;
    let name = |entry: TagStruct<'_>, field: &str| entry.read_string_id(field).unwrap_or_default();
    let functions: Vec<_> = elements(block, "functions")
        .iter()
        .filter_map(|entry| {
            let entry = *entry;
            Some(H2ObjectFunction {
                flags: FunctionFlags::read(entry),
                import: name(entry, "import name"),
                export: name(entry, "export name"),
                turn_off_with: name(entry, "turn off with"),
                range: name(entry, "ranged interpolation name"),
                min_value: entry.read_real("min value").unwrap_or(0.0),
                function: Arc::new(tag_function_at(&entry, "default function")?),
                scale_by: name(entry, "scale by"),
            })
        })
        .collect();
    let rules = match blam_tags::game::Game::of(object) {
        blam_tags::game::Game::Halo2 => FunctionRules::Halo2,
        _ => FunctionRules::Gen3,
    };
    let switches = object_block(object, "function switches")
        .map(|block| read_function_switches(block, &name))
        .unwrap_or_default();
    Some(H2ObjectFunctions { functions, switches, rules, exports: TypeExports::of(object, model) })
}

/// Halo 4's `function switches`. A case that picks its own switch's export is
/// a loop the game turns to `zero` as it loads (the 360's
/// `ObjectFunctionSwitchDefinition::Postprocess` 0x84af07a8).
fn read_function_switches(block: TagStruct<'_>, name: &impl Fn(TagStruct<'_>, &str) -> String) -> Vec<FunctionSwitch> {
    elements(block, "function switches")
        .iter()
        .map(|entry| {
            let export = name(*entry, "export name");
            let cases = elements(*entry, "switched functions")
                .iter()
                .map(|case| {
                    let bounds = match case.field("selection bounds").and_then(|field| field.value()) {
                        Some(blam_tags::TagFieldData::RealBounds(bounds)) => [bounds.lower, bounds.upper],
                        _ => [0.0; 2],
                    };
                    let function = name(*case, "function name");
                    (bounds, if function == export { "zero".to_owned() } else { function })
                })
                .collect();
            FunctionSwitch { switch: name(*entry, "switch function name"), export, cases }
        })
        .collect()
}

/// A button for each effect an event would play. An effect starts when its
/// function comes on, so a press turns it off for a frame and then on.
fn draw_event_effects(ui: &mut Ui, events: &[&str], key: u64, set: &mut H2FunctionInputs) {
    const STARTING: f32 = -1.0;
    for name in events {
        if let Some(value) = set.get_mut(&(key, (*name).to_owned()))
            && *value == STARTING
        {
            *value = 1.0;
        }
    }
    if events.is_empty() {
        return;
    }
    ui.add_space(6.0);
    egui::CollapsingHeader::new("Effects").id_salt("object_event_effects").show(ui, |ui| {
        for name in events {
            if ui.small_button(format!("Play {name}")).clicked() {
                set.insert((key, (*name).to_owned()), STARTING);
                // It has to be seen off for a frame before it can come on.
                ui.ctx().request_repaint();
            }
        }
    });
}

/// Sliders for what an object imports, and what its functions come to.
pub(in crate::app::model_preview) fn draw_h2_function_controls(
    ui: &mut Ui,
    functions: &H2ObjectFunctions,
    key: u64,
    set: &mut H2FunctionInputs,
    values: &H2FunctionValues,
    shader_inputs: &[String],
) {
    let mut names = functions.imports();
    for name in shader_inputs {
        if !functions.makes(name) && !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    let (events, names): (Vec<&str>, Vec<&str>) =
        names.into_iter().partition(|name| super::super::gen3::effects::is_event_function(name));
    draw_event_effects(ui, &events, key, set);
    if names.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.strong("Object Functions");
        let changed = set.keys().any(|(geometry, _)| *geometry == key);
        if ui.add_enabled(changed, egui::Button::new("Reset")).clicked() {
            set.retain(|(geometry, _), _| *geometry != key);
        }
    });
    const SCRIPTED: &str = "Set it as a script would, with object_set_function_variable: \
                            the game reads that before what the function makes.";
    const UNREAD: &str = "Nothing this preview draws reads this. It may drive something \
                          the preview does not have, such as an effect's particles.";
    let reached = functions.reaching(shader_inputs);
    let drawn = |name: &str| reached.iter().any(|known| known == name);
    for name in names {
        let at_rest = functions.idle(name);
        let mut value = set.get(&(key, name.to_owned())).copied().unwrap_or(at_rest);
        let slider = egui::Slider::new(&mut value, 0.0..=1.0).text(name);
        if ui.add_enabled(drawn(name), slider).on_disabled_hover_text(UNREAD).changed() {
            set.insert((key, name.to_owned()), value);
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for function in &functions.functions {
        let export = function.export.as_str();
        // One the type exports never answers for its name; the slider above
        // does.
        if export.is_empty() || seen.contains(&export) || functions.type_exports(export) {
            continue;
        }
        seen.push(export);
        let computed = values.get(export).copied().unwrap_or(0.0);
        let held = (key, export.to_owned());
        ui.add_enabled_ui(drawn(export), |ui| {
            ui.horizontal(|ui| {
                let mut scripted = set.contains_key(&held);
                let toggle = ui.add(ThemedCheckbox::without_text(&mut scripted)).on_hover_text(SCRIPTED);
                if toggle.changed() {
                    if scripted {
                        set.insert(held.clone(), computed);
                    } else {
                        set.remove(&held);
                    }
                }
                match set.get_mut(&held) {
                    Some(value) => {
                        ui.add(egui::Slider::new(value, 0.0..=1.0).text(export));
                    }
                    None => {
                        ui.label(format!("{export}: {computed:.2}"));
                    }
                }
            });
        })
        .response
        .on_disabled_hover_text(UNREAD);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Arc<TagFunction> {
        let mut function = blam_tags::tag_function::H2Function::new(FunctionType::Identity);
        function.set_clamp_range(0.0, 1.0).unwrap();
        let encoding = blam_tags::tag_function::FunctionEncoding::H2;
        Arc::new(TagFunction::parse_encoded(encoding, &function.to_bytes()).unwrap())
    }

    #[test]
    fn an_events_function_goes_by_its_field_and_a_region_name_does_not() {
        use super::super::super::gen3::effects::is_event_function;
        assert!(is_event_function("firing effect: firing"));
        assert!(is_event_function("transition effect (generic): human_explosion_large"));
        assert!(!is_event_function("headlights"));
        assert!(!is_event_function("turret: power"));
    }

    #[test]
    fn only_what_reaches_something_drawn_counts() {
        let mut shield = function("current_shield_damage", "shield_hit");
        shield.scale_by = "scale".to_owned();
        let mut down = function("shield_depleted", "shield_down");
        down.turn_off_with = "body_vitality".to_owned();
        let chained = function("shield_hit", "glow");
        let functions = H2ObjectFunctions { functions: vec![shield, down, chained], ..Default::default() };
        let reached = functions.reaching(&["glow".to_owned()]);
        for name in ["glow", "shield_hit", "current_shield_damage", "scale"] {
            assert!(reached.iter().any(|known| known == name), "{name}");
        }
        for name in ["shield_down", "shield_depleted", "body_vitality"] {
            assert!(!reached.iter().any(|known| known == name), "{name}");
        }
    }

    fn function(import: &str, export: &str) -> H2ObjectFunction {
        H2ObjectFunction {
            flags: FunctionFlags::default(),
            import: import.to_owned(),
            export: export.to_owned(),
            turn_off_with: String::new(),
            range: String::new(),
            min_value: 0.0,
            function: identity(),
            scale_by: String::new(),
        }
    }

    fn value(functions: Vec<H2ObjectFunction>, name: &str, inputs: &[(&str, f32)]) -> f32 {
        let inputs = inputs.iter().map(|(name, value)| ((*name).to_owned(), *value)).collect();
        let functions = H2ObjectFunctions { functions, ..Default::default() };
        functions.value(name, &inputs, &H2FunctionValues::new(), Clock::default(), 0).0
    }

    #[test]
    fn a_script_sets_what_a_function_makes_and_what_it_reads() {
        let functions = vec![function("", "invincibility"), function("zero", "effects")];
        let functions = H2ObjectFunctions { functions, ..Default::default() };
        let none = H2FunctionValues::new();
        let scripted: H2FunctionValues = [("invincibility".to_owned(), 1.0)].into_iter().collect();
        assert_eq!(functions.value("invincibility", &none, &none, Clock::default(), 0).0, 0.0, "an empty import reads nothing");
        assert_eq!(functions.value("invincibility", &none, &scripted, Clock::default(), 0).0, 1.0);
        let by_import: H2FunctionValues = [("zero".to_owned(), 0.75)].into_iter().collect();
        assert_eq!(functions.value("effects", &none, &by_import, Clock::default(), 0).0, 0.75, "the import's script value, unmapped");
    }

    fn switch(switch: &str, export: &str, cases: &[([f32; 2], &str)]) -> FunctionSwitch {
        let cases = cases.iter().map(|(bounds, function)| (*bounds, (*function).to_owned())).collect();
        FunctionSwitch { switch: switch.to_owned(), export: export.to_owned(), cases }
    }

    fn switched(functions: Vec<H2ObjectFunction>, switches: Vec<FunctionSwitch>, inputs: &[(&str, f32)]) -> H2FunctionValues {
        let functions = H2ObjectFunctions { functions, switches, rules: FunctionRules::Gen3, exports: None };
        let inputs: H2FunctionInputs = inputs.iter().map(|(name, value)| ((0, (*name).to_owned()), *value)).collect();
        functions.values(&inputs, 0, &[], 0.0, 0)
    }

    /// `ObjectFunctionSwitchDefinition::Evaluate`: the first case holding the
    /// switch's value, bounds included, else 0.
    #[test]
    fn a_switch_answers_with_the_first_case_its_value_falls_in() {
        let functions = vec![function("red", "red_out"), function("blue", "blue_out")];
        let cases = [([0.0, 0.5], "red_out"), ([0.5, 1.0], "blue_out")];
        let switches = vec![switch("pick", "color", &cases)];
        let at = |pick: f32| switched(functions.clone(), switches.clone(), &[("pick", pick), ("red", 0.25), ("blue", 0.75)])["color"];
        assert_eq!(at(0.0), 0.25);
        assert_eq!(at(0.5), 0.25, "both ends are in, and the first case wins");
        assert_eq!(at(0.8), 0.75);
        let gap = vec![switch("pick", "color", &[([0.0, 0.25], "red_out")])];
        assert_eq!(switched(functions.clone(), gap, &[("pick", 0.5), ("red", 0.25)])["color"], 0.0);
    }

    /// Halo 4's `object_get_function_value` asks a switch only when no
    /// function has the name, and the last switch of a name stands.
    #[test]
    fn a_function_beats_a_switch_and_the_last_switch_stands() {
        let functions = vec![function("red", "color"), function("blue", "blue_out"), function("red", "red_out")];
        let switches = vec![switch("", "color", &[([0.0, 1.0], "blue_out")])];
        assert_eq!(switched(functions.clone(), switches, &[("red", 0.25), ("blue", 0.75)])["color"], 0.25);
        let switches = vec![
            switch("", "shade", &[([0.0, 1.0], "red_out")]),
            switch("", "shade", &[([0.0, 1.0], "blue_out")]),
        ];
        assert_eq!(switched(functions, switches, &[("red", 0.25), ("blue", 0.75)])["shade"], 0.75);
    }

    /// The panel offers what a switch reads, and not what it makes.
    #[test]
    fn a_switch_reads_its_switch_function_and_makes_its_export() {
        let functions = H2ObjectFunctions {
            functions: vec![function("red", "red_out")],
            switches: vec![switch("pick", "color", &[([0.0, 1.0], "red_out")])],
            ..Default::default()
        };
        let imports = functions.imports();
        assert!(imports.contains(&"pick") && imports.contains(&"red"), "{imports:?}");
        assert!(functions.makes("color"));
        let reached = functions.reaching(&["color".to_owned()]);
        for name in ["pick", "red_out", "red"] {
            assert!(reached.iter().any(|known| known == name), "{name}");
        }
    }

    /// With `BLAM_TEST_H4EK`: Cortana's hologram colors come from switches.
    #[test]
    fn halo_4_cortana_reads_her_function_switches() {
        let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
            return;
        };
        let path = std::path::Path::new(&root).join("objects/characters/storm_cortana/storm_cortana.biped");
        let tag = TagFile::read(&path).unwrap();
        let functions = read_h2_object_functions(&tag, None).unwrap();
        assert!(!functions.switches.is_empty());
        for switch in &functions.switches {
            assert!(!switch.export.is_empty() && !switch.switch.is_empty(), "{switch:?}");
            assert!(switch.cases.iter().all(|(_, function)| !function.is_empty()), "{switch:?}");
            assert!(functions.makes(&switch.export), "{}", switch.export);
        }
        let values = functions.values(&H2FunctionInputs::new(), 0, &[], 0.0, 0);
        assert!(functions.switches.iter().all(|switch| values.contains_key(&switch.export)));
    }

    #[test]
    fn an_output_follows_what_it_imports() {
        assert_eq!(value(vec![function("speed", "glow")], "glow", &[("speed", 0.5)]), 0.5);
        assert_eq!(value(vec![function("speed", "glow")], "glow", &[("speed", 0.0)]), 0.0);
    }

    #[test]
    fn nothing_named_reads_full_and_an_unknown_name_reads_nothing() {
        assert_eq!(value(Vec::new(), "", &[]), 1.0);
        assert_eq!(value(Vec::new(), "one", &[]), 1.0);
        assert_eq!(value(Vec::new(), "no_such_function", &[]), 0.0);
    }

    #[test]
    fn invert_and_scale_by_shape_the_value() {
        let inverted = H2ObjectFunction { flags: FunctionFlags { invert: true, ..Default::default() }, ..function("speed", "glow") };
        assert_eq!(value(vec![inverted], "glow", &[("speed", 0.25)]), 0.75);
        let scaled = H2ObjectFunction { scale_by: "power".to_owned(), ..function("speed", "glow") };
        assert_eq!(value(vec![scaled], "glow", &[("speed", 0.5), ("power", 0.5)]), 0.25);
    }

    fn device(functions: Vec<H2ObjectFunction>) -> H2ObjectFunctions {
        let tables = &[OBJECT_EXPORTS, DEVICE_EXPORTS];
        let exports = TypeExports { tables, has_body: true, has_shield: false, variants: 4 };
        H2ObjectFunctions { functions, exports: Some(exports), ..Default::default() }
    }

    #[test]
    fn what_the_type_exports_answers_before_a_function_of_its_name() {
        // A control with its own `position` function, as several kit
        // switches have: the device's exporter wins, idle at 0.
        let functions = device(vec![function("one", "position")]);
        let none = H2FunctionValues::new();
        assert_eq!(functions.value("position", &none, &none, Clock::default(), 0), (0.0, false));
        assert!(!functions.makes("position"), "it is an input, the panel's to set");
        let set = H2FunctionValues::from([("position".to_owned(), 0.5)]);
        assert_eq!(functions.value("position", &set, &none, Clock::default(), 0).0, 0.5);
    }

    #[test]
    fn an_idle_object_reads_what_its_type_and_model_give() {
        let functions = device(Vec::new());
        assert_eq!(functions.idle("power"), 1.0, "a device starts powered");
        assert_eq!(functions.idle("loop"), 1.0);
        assert_eq!(functions.idle("body_vitality"), 1.0);
        assert_eq!(functions.idle("shield_vitality"), 0.0, "this model has no shield");
        assert_eq!(functions.idle("shield_depleted"), 1.0);
        assert_eq!(functions.idle("variant"), 0.25, "the first of four");
        assert_eq!(functions.idle("turned_on"), 0.0, "a weapon's, which a device does not export");
    }

    #[test]
    fn without_known_exporters_the_functions_answer_first() {
        // Halo 3 and ODST, whose exporters this does not have.
        let functions = H2ObjectFunctions { functions: vec![function("one", "position")], ..Default::default() };
        let none = H2FunctionValues::new();
        assert_eq!(functions.value("position", &none, &none, Clock::default(), 0), (1.0, true));
    }

    #[test]
    fn a_min_value_keeps_a_function_off_below_it() {
        let held = H2ObjectFunction { min_value: 0.5, ..function("speed", "glow") };
        assert_eq!(value(vec![held.clone()], "glow", &[("speed", 0.25)]), 0.0);
        assert_eq!(value(vec![held], "glow", &[("speed", 0.75)]), 0.75);
    }

    #[test]
    fn turn_off_with_needs_its_export_on() {
        let tied = H2ObjectFunction { turn_off_with: "alive".to_owned(), ..function("speed", "glow") };
        assert_eq!(value(vec![tied.clone()], "glow", &[("speed", 1.0), ("alive", 0.0)]), 0.0);
        assert_eq!(value(vec![tied], "glow", &[("speed", 1.0), ("alive", 1.0)]), 1.0);
    }

    fn gen3(functions: Vec<H2ObjectFunction>, name: &str, inputs: &[(&str, f32)]) -> (f32, bool) {
        let inputs = inputs.iter().map(|(name, value)| ((*name).to_owned(), *value)).collect();
        let functions = H2ObjectFunctions { functions, rules: FunctionRules::Gen3, ..Default::default() };
        functions.value(name, &inputs, &H2FunctionValues::new(), Clock::default(), 0)
    }

    #[test]
    fn halo_3_and_reach_take_the_first_function_of_a_name() {
        let list = vec![function("speed", "glow"), function("power", "glow")];
        assert_eq!(gen3(list.clone(), "glow", &[("speed", 0.25), ("power", 0.75)]).0, 0.25);
        assert_eq!(value(list, "glow", &[("speed", 0.25), ("power", 0.75)]), 0.75, "Halo 2 the last");
    }

    #[test]
    fn halo_3_and_reach_look_turn_off_with_up_in_full_and_can_ask_its_magnitude() {
        // `alive` is made by a function here, which Halo 2's exported read misses.
        let made = function("power", "alive");
        let tied = H2ObjectFunction { turn_off_with: "alive".to_owned(), ..function("speed", "glow") };
        let inputs = [("speed", 1.0), ("power", 0.0)];
        assert_eq!(gen3(vec![tied.clone(), made.clone()], "glow", &inputs), (0.0, false));
        assert_eq!(value(vec![tied.clone(), made.clone()], "glow", &inputs), 1.0, "Halo 2 reads the idle export");
        let inputs = [("speed", 1.0), ("power", 0.5)];
        assert_eq!(gen3(vec![tied.clone(), made.clone()], "glow", &inputs), (1.0, true));
        // On, but by magnitude: a mapping that holds it active at 0.
        let held = H2ObjectFunction { flags: FunctionFlags { always_active: true, ..Default::default() }, ..made };
        let magnitude = H2ObjectFunction {
            flags: FunctionFlags { turn_off_with_uses_magnitude: true, ..Default::default() },
            ..tied.clone()
        };
        let inputs = [("speed", 1.0), ("power", 0.0)];
        assert_eq!(gen3(vec![tied, held.clone()], "glow", &inputs), (1.0, true), "on is enough");
        assert_eq!(gen3(vec![magnitude, held], "glow", &inputs), (0.0, false), "0 turns it off");
    }

    #[test]
    fn always_exports_value_keeps_its_value_while_off() {
        let held = H2ObjectFunction { min_value: 0.5, ..function("speed", "glow") };
        let exporting = H2ObjectFunction {
            flags: FunctionFlags { always_exports_value: true, ..Default::default() },
            ..held.clone()
        };
        assert_eq!(gen3(vec![held], "glow", &[("speed", 0.25)]), (0.0, false));
        assert_eq!(gen3(vec![exporting], "glow", &[("speed", 0.25)]), (0.25, false));
    }

    #[test]
    fn a_range_name_feeds_the_curves_range() {
        // A curve whose output is its range: a constant of 0 below, 1 above.
        let mut ranged = blam_tags::tag_function::H2Function::new(FunctionType::Constant);
        ranged.set_clamp_range(0.0, 1.0).unwrap();
        let encoding = blam_tags::tag_function::FunctionEncoding::H2;
        let curve = TagFunction::parse_encoded(encoding, &ranged.to_bytes()).unwrap();
        let low = curve.evaluate(0.5, 0.0);
        let high = curve.evaluate(0.5, 1.0);
        let function = H2ObjectFunction { range: "heat".to_owned(), function: Arc::new(curve), ..function("speed", "glow") };
        assert_eq!(gen3(vec![function.clone()], "glow", &[("speed", 0.5), ("heat", 0.0)]).0, low.clamp(0.0, 1.0));
        assert_eq!(gen3(vec![function], "glow", &[("speed", 0.5), ("heat", 1.0)]).0, high.clamp(0.0, 1.0));
    }

    #[test]
    fn the_last_function_of_a_name_stands_and_a_loop_ends() {
        let list = vec![function("speed", "glow"), function("power", "glow")];
        assert_eq!(value(list, "glow", &[("speed", 0.25), ("power", 0.75)]), 0.75);
        let looped = H2ObjectFunction { scale_by: "glow".to_owned(), ..function("speed", "glow") };
        assert_eq!(value(vec![looped], "glow", &[("speed", 1.0)]), 0.0);
    }
}
