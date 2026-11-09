//! Reach's load rules.

use super::super::gen3::rules::Gen3Rules;
use super::super::gen3::shaders::{Gen3Clock, RenderMethodDialect};
use super::super::load::GameRules;

pub(in crate::app::model_preview) const RULES: GameRules = GameRules {
    unit_weapons: true,
    none_instance_takes_all: true,
    ik_markers_on_weapons: true,
    gen3: Gen3Rules {
        dialect: RenderMethodDialect::Reach,
        reference_shaders: true,
        clock: Gen3Clock::Reach,
        screen_clock: Gen3Clock::Reach,
        single_pass_entries: true,
        shadow_generate: true,
        camo_pass: Some(super::camo::camo_pass),
        environment_maps: true,
        screen_effect_terms: true,
        strip_terms: true,
        light_volume_terms: true,
        ..Gen3Rules::DEFAULT
    },
    ..GameRules::DEFAULT
};
