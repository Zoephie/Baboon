//! Halo 2's load rules.

use super::super::load::GameRules;
use super::super::particles::{OwnParticleSystem, ParticleRules};

pub(in crate::app::model_preview) const RULES: GameRules = GameRules {
    unit_weapons: true,
    particles: ParticleRules {
        own_system: Some(OwnParticleSystem {
            read: super::particles::read_h2_system_parts,
            baked_lod: super::particles::h2_baked_lod_of,
        }),
        ..ParticleRules::DEFAULT
    },
    ..GameRules::DEFAULT
};
