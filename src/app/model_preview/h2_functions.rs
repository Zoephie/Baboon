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

const INVERT: i128 = 1;
const MAPPING_CONTROLS_ACTIVE: i128 = 2;
const ALWAYS_ACTIVE: i128 = 4;
/// How far `scale by` can chain before it has to be a loop.
const DEEPEST_CHAIN: usize = 8;

#[derive(Debug, Clone)]
struct H2ObjectFunction {
    flags: i128,
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
        time: f32,
        depth: usize,
    ) -> (f32, bool) {
        let (input, mut active) = exported(&self.import, inputs);
        let mapping_controls = self.flags & MAPPING_CONTROLS_ACTIVE != 0;
        let mut value;
        if self.is_periodic() {
            value = self.function.evaluate(time, 1.0) * input;
        } else if !active && mapping_controls {
            value = input;
        } else {
            let inverted = if self.flags & INVERT != 0 { 1.0 - input } else { input };
            value = self.function.evaluate(inverted, 1.0);
            if !mapping_controls {
                active = value > 0.0;
            }
        }
        if !self.scale_by.is_empty() {
            let (scale, scaled) = owner.value(&self.scale_by, inputs, time, depth + 1);
            value *= scale;
            active = active && scaled;
        }
        if active && self.min_value > 0.0 {
            active = value > self.min_value;
        }
        if self.flags & ALWAYS_ACTIVE != 0 {
            active = true;
        }
        if active && !self.turn_off_with.is_empty() {
            active = exported(&self.turn_off_with, inputs).1;
        }
        if !active { (0.0, false) } else { (value.clamp(0.0, 1.0), true) }
    }
}

impl H2ObjectFunctions {
    /// `sub_86F2F0`, `object_get_function_value`: nothing named reads full,
    /// then what the object's type exports, then its functions, the last of
    /// a name standing.
    fn value(
        &self,
        name: &str,
        inputs: &H2FunctionValues,
        time: f32,
        depth: usize,
    ) -> (f32, bool) {
        if name.is_empty() || name == "one" {
            return (1.0, true);
        }
        if name == "zero" || depth > DEEPEST_CHAIN {
            return (0.0, false);
        }
        let last = self.functions.iter().rev().find(|function| function.export == name);
        match last {
            Some(function) => function.evaluate(self, inputs, time, depth),
            None => exported(name, inputs),
        }
    }

    /// The names the panel can set.
    fn imports(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for function in &self.functions {
            if !function.import.is_empty() && !names.contains(&function.import.as_str()) {
                names.push(&function.import);
            }
        }
        names
    }

    /// Whether anything here moves with no one touching it.
    pub(crate) fn move_at_rest(&self, inputs: &H2FunctionValues) -> bool {
        self.functions.iter().any(|function| {
            function.is_periodic() && exported(&function.import, inputs).0 > 0.0
        })
    }

    /// Every name this frame, for what reads them.
    pub(crate) fn values(
        &self,
        inputs: &H2FunctionInputs,
        geometry_id: u64,
        extra: &[String],
        time: f32,
    ) -> H2FunctionValues {
        let mut set = H2FunctionValues::new();
        let names = self.imports().into_iter().map(str::to_owned).chain(extra.iter().cloned());
        for name in names {
            let value = inputs
                .get(&(geometry_id, name.clone()))
                .copied()
                .unwrap_or_else(|| object_input(&name));
            set.insert(name, value);
        }
        let mut values = set.clone();
        for function in &self.functions {
            let (value, _) = self.value(&function.export, &set, time, 0);
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
                flags: entry.read_int_any("flags").unwrap_or(0),
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
    for name in names {
        let at_rest = object_input(name);
        let mut value = set.get(&(key, name.to_owned())).copied().unwrap_or(at_rest);
        if ui.add(egui::Slider::new(&mut value, 0.0..=1.0).text(name)).changed() {
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
        ui.label(format!("{export}: {:.2}", values.get(export).copied().unwrap_or(0.0)));
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

    fn function(import: &str, export: &str) -> H2ObjectFunction {
        H2ObjectFunction {
            flags: 0,
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
        H2ObjectFunctions { functions }.value(name, &inputs, 0.0, 0).0
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
        let inverted = H2ObjectFunction { flags: INVERT, ..function("speed", "glow") };
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
