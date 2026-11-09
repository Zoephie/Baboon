//! Halo 3's and ODST's load rules.

use super::super::load::GameRules;

pub(in crate::app::model_preview) const RULES: GameRules = GameRules {
    unit_weapons: true,
    ..GameRules::DEFAULT
};

/// ODST's: Halo 3's, where they agree.
pub(in crate::app::model_preview) const ODST_RULES: GameRules = GameRules { ..RULES };
