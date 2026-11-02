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

use super::ce_attachments::{elements, object_block};
use super::h2_passes::{object_input, tag_function_at};

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
    min_value: f32,
    function: Arc<TagFunction>,
    scale_by: String,
}

/// An object's `functions` block.
#[derive(Debug, Clone, Default)]
pub(crate) struct H2ObjectFunctions {
    functions: Vec<H2ObjectFunction>,
}

/// What `object_type_export_function_values` leaves in `inputs`: its value,
/// and whether anything exports it.
fn exported(name: &str, inputs: &H2FunctionValues) -> (f32, bool) {
    let value = inputs.get(name).copied().unwrap_or_else(|| object_input(name));
    (value, value > 0.0)
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
        let (input, mut active) = exported(&self.import, inputs);
        let mapping_passes = self.flags.mapping_does_not_control_active;
        let mut value;
        if self.is_periodic() {
            let time = if self.flags.random_time_offset { clock.time + clock.offset } else { clock.time };
            value = self.function.evaluate(time, 1.0) * input;
        } else if !active && mapping_passes {
            value = input;
        } else {
            let inverted = if self.flags.invert { 1.0 - input } else { input };
            value = self.function.evaluate(inverted, 1.0);
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
            active = exported(&self.turn_off_with, inputs).1;
        }
        if !active { (0.0, false) } else { (value.clamp(0.0, 1.0), true) }
    }
}

impl H2ObjectFunctions {
    /// `sub_86F2F0`, `object_get_function_value`: a script's value first
    /// (`object_set_function_variable`, read by `sub_8E7890`), then nothing
    /// named reads full, then what the object's type exports, then its
    /// functions, the last of a name standing.
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
        let last = self.functions.iter().rev().find(|function| function.export == name);
        match last {
            Some(function) => function.evaluate(self, inputs, scripted, clock, depth),
            None => exported(name, inputs),
        }
    }

    /// The names the panel can set: what a function reads, and what scales or
    /// turns it off, unless another function here makes that.
    fn imports(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        let made_here = |name: &str| self.functions.iter().any(|function| function.export == name);
        for function in &self.functions {
            let read = [&function.import, &function.scale_by, &function.turn_off_with];
            for name in read.map(String::as_str) {
                if !name.is_empty() && !made_here(name) && !names.contains(&name) {
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
            for function in self.functions.iter().filter(|function| function.export == name) {
                for source in [&function.import, &function.scale_by, &function.turn_off_with] {
                    if !source.is_empty() && !reached.contains(source) {
                        reached.push(source.clone());
                    }
                }
            }
        }
        reached
    }

    /// Whether anything here moves with no one touching it.
    pub(crate) fn move_at_rest(&self, inputs: &H2FunctionValues) -> bool {
        self.functions.iter().any(|function| {
            function.is_periodic() && exported(&function.import, inputs).0 > 0.0
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
            let value = inputs
                .get(&(geometry_id, name.clone()))
                .copied()
                .unwrap_or_else(|| object_input(&name));
            set.insert(name, value);
        }
        // What the panel sets of a name a function makes is a script's value.
        let scripted: H2FunctionValues = self
            .functions
            .iter()
            .filter_map(|function| {
                let value = inputs.get(&(geometry_id, function.export.clone()))?;
                Some((function.export.clone(), *value))
            })
            .collect();
        let mut values = set.clone();
        for function in &self.functions {
            let (value, _) = self.value(&function.export, &set, &scripted, clock, 0);
            values.insert(function.export.clone(), value);
        }
        values
    }
}

/// An object's functions, or none on a tag with no such block. An empty block
/// still counts: its shaders can read what the object's type exports.
pub(crate) fn read_h2_object_functions(object: &TagFile) -> Option<H2ObjectFunctions> {
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
                min_value: entry.read_real("min value").unwrap_or(0.0),
                function: Arc::new(tag_function_at(&entry, "default function")?),
                scale_by: name(entry, "scale by"),
            })
        })
        .collect();
    Some(H2ObjectFunctions { functions })
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
pub(super) fn draw_h2_function_controls(
    ui: &mut Ui,
    functions: &H2ObjectFunctions,
    key: u64,
    set: &mut H2FunctionInputs,
    values: &H2FunctionValues,
    shader_inputs: &[String],
) {
    let exports = |name: &str| functions.functions.iter().any(|function| function.export == name);
    let mut names = functions.imports();
    for name in shader_inputs {
        if !exports(name) && !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    let (events, names): (Vec<&str>, Vec<&str>) =
        names.into_iter().partition(|name| super::h3_effects::is_event_function(name));
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
        let at_rest = object_input(name);
        let mut value = set.get(&(key, name.to_owned())).copied().unwrap_or(at_rest);
        let slider = egui::Slider::new(&mut value, 0.0..=1.0).text(name);
        if ui.add_enabled(drawn(name), slider).on_disabled_hover_text(UNREAD).changed() {
            set.insert((key, name.to_owned()), value);
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for function in &functions.functions {
        let export = function.export.as_str();
        if export.is_empty() || seen.contains(&export) {
            continue;
        }
        seen.push(export);
        let computed = values.get(export).copied().unwrap_or(0.0);
        let held = (key, export.to_owned());
        ui.add_enabled_ui(drawn(export), |ui| {
            ui.horizontal(|ui| {
                let mut scripted = set.contains_key(&held);
                let toggle = ui.checkbox(&mut scripted, "").on_hover_text(SCRIPTED);
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
        use super::super::h3_effects::is_event_function;
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
        let functions = H2ObjectFunctions { functions: vec![shield, down, chained] };
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
            min_value: 0.0,
            function: identity(),
            scale_by: String::new(),
        }
    }

    fn value(functions: Vec<H2ObjectFunction>, name: &str, inputs: &[(&str, f32)]) -> f32 {
        let inputs = inputs.iter().map(|(name, value)| ((*name).to_owned(), *value)).collect();
        H2ObjectFunctions { functions }.value(name, &inputs, &H2FunctionValues::new(), Clock::default(), 0).0
    }

    #[test]
    fn a_script_sets_what_a_function_makes_and_what_it_reads() {
        let functions = H2ObjectFunctions { functions: vec![function("", "invincibility"), function("zero", "effects")] };
        let none = H2FunctionValues::new();
        let scripted: H2FunctionValues = [("invincibility".to_owned(), 1.0)].into_iter().collect();
        assert_eq!(functions.value("invincibility", &none, &none, Clock::default(), 0).0, 0.0, "an empty import reads nothing");
        assert_eq!(functions.value("invincibility", &none, &scripted, Clock::default(), 0).0, 1.0);
        let by_import: H2FunctionValues = [("zero".to_owned(), 0.75)].into_iter().collect();
        assert_eq!(functions.value("effects", &none, &by_import, Clock::default(), 0).0, 0.75, "the import's script value, unmapped");
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

    #[test]
    fn the_last_function_of_a_name_stands_and_a_loop_ends() {
        let list = vec![function("speed", "glow"), function("power", "glow")];
        assert_eq!(value(list, "glow", &[("speed", 0.25), ("power", 0.75)]), 0.75);
        let looped = H2ObjectFunction { scale_by: "glow".to_owned(), ..function("speed", "glow") };
        assert_eq!(value(vec![looped], "glow", &[("speed", 1.0)]), 0.0);
    }
}
