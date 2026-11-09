//! Halo 4's load rules.

use super::super::load::GameRules;

pub(in crate::app::model_preview) const RULES: GameRules = GameRules {
    unit_weapons: true,
    legacy_weapon_ik: false,
    none_instance_takes_all: true,
    weapon_hand_marker: Some(super::weapons::weapon_specific_marker),
    ik_markers_on_weapons: true,
    ..GameRules::DEFAULT
};
