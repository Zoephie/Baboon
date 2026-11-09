//! Reach's load rules.

use super::super::gen3::rules::Gen3Rules;
use super::super::load::GameRules;

pub(in crate::app::model_preview) const RULES: GameRules = GameRules {
    unit_weapons: true,
    none_instance_takes_all: true,
    ik_markers_on_weapons: true,
    gen3: Gen3Rules {
        reference_shaders: true,
        ..Gen3Rules::DEFAULT
    },
    ..GameRules::DEFAULT
};
