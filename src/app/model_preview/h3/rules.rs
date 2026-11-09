//! Halo 3's and ODST's load rules.

use super::super::gen3::rules::Gen3Rules;
use super::super::gen3::shaders::Gen3Clock;
use super::super::load::GameRules;

pub(in crate::app::model_preview) const RULES: GameRules = GameRules {
    unit_weapons: true,
    ..GameRules::DEFAULT
};

/// ODST's: Halo 3's, but for its clocks and its screen effects' object
/// falloff.
pub(in crate::app::model_preview) const ODST_RULES: GameRules = GameRules {
    gen3: Gen3Rules {
        clock: Gen3Clock::Odst,
        screen_clock: Gen3Clock::OdstScreen,
        screen_object_falloff: true,
        ..RULES.gen3
    },
    ..RULES
};
