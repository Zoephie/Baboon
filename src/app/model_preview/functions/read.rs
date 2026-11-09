//! Reading an object's functions: their flags, clocks, switches and inputs.

use super::{elements, has_flag, high16_over_65535, lcg_step, object_block, tag_function_at};
use blam_tags::{TagFile, TagFunction, TagStruct};
use std::collections::HashMap;
use std::sync::Arc;

/// What the panel has set, by object geometry and input name.
pub(crate) type FunctionInputs = HashMap<(u64, String), f32>;

/// Every name an object answers to this frame.
pub(crate) type FunctionValues = HashMap<String, f32>;

/// What an object's type exports to its functions, where its game says
/// (only Halo 2's are known).
pub(in crate::app::model_preview) trait TypeExporter:
    std::fmt::Debug + Send + Sync
{
    /// What an idle object of this type gives `name`, if an exporter claims it.
    fn idle(&self, name: &str) -> Option<f32>;
}

/// `object_get_function_value_simple`: no name reads full, and a name the
/// object does not answer, nothing.
pub(in crate::app::model_preview) fn object_value(
    values: Option<&FunctionValues>,
    name: &str,
) -> f32 {
    if name.is_empty() || name == "one" {
        return 1.0;
    }
    values
        .and_then(|values| values.get(name))
        .copied()
        .unwrap_or(0.0)
}

/// A function's `flags`, by option name.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct FunctionFlags {
    pub(super) invert: bool,
    /// `mapping does not controls active`: an inactive input passes through
    /// unmapped, and the curve's value does not decide whether it is on.
    pub(super) mapping_does_not_control_active: bool,
    pub(super) always_active: bool,
    /// `random time offset`: a periodic function runs ahead by a fraction of
    /// a second that differs per object.
    pub(super) random_time_offset: bool,
    /// `always exports value`: off, it still exports its value.
    pub(super) always_exports_value: bool,
    /// `turn off with uses magnitude`: the `turn off with` function must also
    /// be above 0.
    pub(super) turn_off_with_uses_magnitude: bool,
}

impl FunctionFlags {
    fn read(entry: TagStruct<'_>) -> Self {
        let set = |wanted: &str| has_flag(entry, "flags", wanted);
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
pub(super) struct Clock {
    pub(super) time: f32,
    pub(super) offset: f32,
}

impl Clock {
    pub(super) fn of(time: f32, owner: u8) -> Self {
        Self {
            time,
            offset: high16_over_65535(lcg_step(u32::from(owner))),
        }
    }
}

/// How far `scale by` can chain before it has to be a loop.
pub(super) const DEEPEST_CHAIN: usize = 8;

#[derive(Debug, Clone)]
pub(super) struct ObjectFunction {
    pub(super) flags: FunctionFlags,
    pub(super) import: String,
    pub(super) export: String,
    pub(super) turn_off_with: String,
    /// Reach's `ranged interpolation name`: what feeds the curve's range.
    pub(super) range: String,
    pub(super) min_value: f32,
    pub(super) function: Arc<TagFunction>,
    pub(super) scale_by: String,
}

/// Whose `object_get_function_value` an object answers by. They agree but for
/// which function of a name answers and how `turn off with` is read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum FunctionRules {
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
pub(super) struct FunctionSwitch {
    pub(super) switch: String,
    pub(super) export: String,
    /// `selection bounds`, inclusive at both ends, and the function picked.
    pub(super) cases: Vec<([f32; 2], String)>,
}

impl FunctionSwitch {
    /// The first case whose bounds hold the switch's value answers; with
    /// none, the export is 0 and off.
    pub(super) fn evaluate(
        &self,
        owner: &ObjectFunctions,
        inputs: &FunctionValues,
        scripted: &FunctionValues,
        clock: Clock,
        depth: usize,
    ) -> (f32, bool) {
        let (switch, _) = owner.value(&self.switch, inputs, scripted, clock, depth + 1);
        let picked = self
            .cases
            .iter()
            .find(|([lower, upper], _)| *lower <= switch && switch <= *upper);
        match picked {
            Some((_, function)) => owner.value(function, inputs, scripted, clock, depth + 1),
            None => (0.0, false),
        }
    }
}

/// An object's `functions` block, and what its type exports to it.
#[derive(Debug, Clone, Default)]
pub(crate) struct ObjectFunctions {
    pub(super) functions: Vec<ObjectFunction>,
    /// Halo 4's `function switches`; empty for every earlier game.
    pub(super) switches: Vec<FunctionSwitch>,
    pub(super) rules: FunctionRules,
    /// None for an object whose exporters are not known (Halo 3 and ODST),
    /// which reads [`object_input`] instead.
    pub(super) exports: Option<Arc<dyn TypeExporter>>,
}

/// An object's functions, or none on a tag with no such block. An empty block
/// still counts: its shaders can read what the object's type exports. `model`
/// is the object's, whose damage info and variants some exports report.
pub(crate) fn read_object_functions(
    object: &TagFile,
    model: Option<&TagFile>,
) -> Option<ObjectFunctions> {
    let block = object_block(object, "functions")?;
    let name = |entry: TagStruct<'_>, field: &str| entry.read_string_id(field).unwrap_or_default();
    let functions: Vec<_> = elements(block, "functions")
        .iter()
        .filter_map(|entry| {
            let entry = *entry;
            Some(ObjectFunction {
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
    Some(ObjectFunctions {
        functions,
        switches,
        rules,
        exports: super::super::load::type_exports(object, model),
    })
}

/// Halo 4's `function switches`. A case that picks its own switch's export is
/// a loop the game turns to `zero` as it loads (the 360's
/// `ObjectFunctionSwitchDefinition::Postprocess` 0x84af07a8).
fn read_function_switches(
    block: TagStruct<'_>,
    name: &impl Fn(TagStruct<'_>, &str) -> String,
) -> Vec<FunctionSwitch> {
    elements(block, "function switches")
        .iter()
        .map(|entry| {
            let export = name(*entry, "export name");
            let cases = elements(*entry, "switched functions")
                .iter()
                .map(|case| {
                    let bounds = match case
                        .field("selection bounds")
                        .and_then(|field| field.value())
                    {
                        Some(blam_tags::TagFieldData::RealBounds(bounds)) => {
                            [bounds.lower, bounds.upper]
                        }
                        _ => [0.0; 2],
                    };
                    let function = name(*case, "function name");
                    (
                        bounds,
                        if function == export {
                            "zero".to_owned()
                        } else {
                            function
                        },
                    )
                })
                .collect();
            FunctionSwitch {
                switch: name(*entry, "switch function name"),
                export,
                cases,
            }
        })
        .collect()
}

/// What a whole, idle object reads for an input: full for what measures its
/// condition, and otherwise the zero the game falls back on when nothing
/// exports the name, which leaves boosts, hits and open doors off. A guess,
/// for what has no object to ask: a Halo 2 object's functions answer from its
/// type's exporters (`functions`), which Halo 3 and ODST's are not yet.
pub(in crate::app::model_preview) fn object_input(name: &str) -> f32 {
    match name {
        "alive" | "body_vitality" | "device_power" | "electrical_power" | "health" | "one"
        | "power" | "shield" | "shield_strength" | "shield_vitality" => 1.0,
        _ => 0.0,
    }
}
