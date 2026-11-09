//! Reach's load rules.

use super::super::load::GameRules;

pub(in crate::app::model_preview) const RULES: GameRules = GameRules {
    unit_weapons: true,
    none_instance_takes_all: true,
    ..GameRules::DEFAULT
};
