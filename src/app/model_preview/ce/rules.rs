//! Halo CE's load rules.

use super::super::load::{GameRules, ObjectControls};

/// A Halo CE object names its gbxmodel and attaches nothing by the later
/// games' readers. Default Lights, PRT and Self Shadow change nothing a
/// Halo CE model draws, and its renderer has no bloom, so its panel leaves
/// them out.
pub(in crate::app::model_preview) const RULES: GameRules = GameRules {
    textures: true,
    lit_options: false,
    object_controls: Some(ObjectControls {
        above: super::panel::draw_ce_object_controls,
        below: super::panel::draw_ce_function_inputs,
    }),
    ..GameRules::DEFAULT
};
