//! Reach's load rules.

use super::super::gen3::rules::Gen3Rules;
use super::super::gen3::shaders::{Gen3Clock, RenderMethodDialect};
use super::super::load::GameRules;
use super::super::particles::ParticleRules;

pub(in crate::app::model_preview) const RULES: GameRules = GameRules {
    unit_weapons: true,
    none_instance_takes_all: true,
    ik_markers_on_weapons: true,
    particles: REACH_PARTICLES,
    effect_vectors: true,
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

/// Reach's particles, whose simulation Halo 4's takes on with terms of its
/// own.
pub(in crate::app::model_preview) const REACH_PARTICLES: ParticleRules = ParticleRules {
    reach_render_method: true,
    only_facing_still_particles_move: true,
    ..SIMULATION
};

/// What Reach's particles and Halo 4's share.
pub(in crate::app::model_preview) const SIMULATION: ParticleRules = ParticleRules {
    reach_postprocess_seeds: true,
    cheap_shader: true,
    adds_at_illum: true,
    reach_look: true,
    card_lerps: true,
    random_volume_velocities: true,
    scales_axes: true,
    ..ParticleRules::DEFAULT
};
