//! Halo 4's load rules.

use super::super::common::image::ImageTexels;
use super::super::gen3::rules::Gen3Rules;
use super::super::load::GameRules;
use super::super::particles::ParticleRules;
use super::super::reach::rules::SIMULATION;

pub(in crate::app::model_preview) const RULES: GameRules = GameRules {
    unit_weapons: true,
    legacy_weapon_ik: false,
    none_instance_takes_all: true,
    weapon_hand_marker: Some(super::weapons::weapon_specific_marker),
    ik_markers_on_weapons: true,
    // Halo 4's simulation is Reach's, with terms of its own.
    particles: ParticleRules {
        read_material: Some(super::effects::particles::read_h4_particle_material),
        read_emitter_terms: Some(super::effects::emitter_terms::read_h4_emitter_terms),
        cheap_turbulence_types: true,
        ..SIMULATION
    },
    gen3: Gen3Rules {
        texels: Some(ImageTexels::of_halo4),
        luminance_as_red: true,
        light_volume_reader: Some(super::effects::light_volumes::read_h4_definition),
        ..Gen3Rules::DEFAULT
    },
    textures: true,
    ..GameRules::DEFAULT
};
