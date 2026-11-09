//! Halo 4's load rules.

use super::super::common::image::ImageTexels;
use super::super::gen3::rules::Gen3Rules;
use super::super::load::GameRules;

pub(in crate::app::model_preview) const RULES: GameRules = GameRules {
    unit_weapons: true,
    legacy_weapon_ik: false,
    none_instance_takes_all: true,
    weapon_hand_marker: Some(super::weapons::weapon_specific_marker),
    ik_markers_on_weapons: true,
    gen3: Gen3Rules {
        texels: Some(ImageTexels::of_halo4),
        luminance_as_red: true,
        ..Gen3Rules::DEFAULT
    },
    ..GameRules::DEFAULT
};
