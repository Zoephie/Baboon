//! The live particles of a system, each as the card's update shader leaves
//! it, for a game that draws its particles through a material of its own.

use super::property::{ParticleState, lod_amount};
use super::rules::particle_game_time;
use super::sim::{ParticleField, RunningEmitter, emitter_frame, location, system_state};
use super::system::ParticleSystem;
use super::{FunctionValues, ViewFrame, dot, minus};

/// One live particle as the card's update shader leaves it
/// (`s_particle_state`), for a game that draws its particles from their
/// states to pack.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(in crate::app::model_preview) struct LiveParticle {
    /// In emitter space.
    pub(in crate::app::model_preview) position: [f32; 3],
    /// Half the quad's extent.
    pub(in crate::app::model_preview) size: f32,
    pub(in crate::app::model_preview) velocity: [f32; 3],
    pub(in crate::app::model_preview) aspect: f32,
    pub(in crate::app::model_preview) random: [f32; 4],
    pub(in crate::app::model_preview) random2: [f32; 4],
    pub(in crate::app::model_preview) physical_rotation: f32,
    pub(in crate::app::model_preview) manual_rotation: f32,
    pub(in crate::app::model_preview) animated_frame: f32,
    pub(in crate::app::model_preview) manual_frame: f32,
    pub(in crate::app::model_preview) birth_time: f32,
    pub(in crate::app::model_preview) inverse_lifespan: f32,
    /// Of its life, 0 to 1.
    pub(in crate::app::model_preview) age: f32,
    pub(in crate::app::model_preview) intensity: f32,
    pub(in crate::app::model_preview) rotation_rate: f32,
    pub(in crate::app::model_preview) frame_rate: f32,
    pub(in crate::app::model_preview) simulation: [f32; 2],
    pub(in crate::app::model_preview) black_point: f32,
    pub(in crate::app::model_preview) white_point: f32,
    pub(in crate::app::model_preview) palette: f32,
    pub(in crate::app::model_preview) axis: [f32; 3],
    /// Tint times colour, and alpha.
    pub(in crate::app::model_preview) color: [f32; 4],
    /// The spawn colour, and the exposure exponent in `.3`.
    pub(in crate::app::model_preview) initial_color: [f32; 4],
}

/// One emitter's live particles at one location, and the matrix from their
/// space to the world.
#[derive(Debug, Clone)]
pub(in crate::app::model_preview) struct LiveEmitter {
    /// `vs_emitter_to_world_matrix`: forward, left, up and position.
    pub(in crate::app::model_preview) to_world: [[f32; 3]; 4],
    pub(in crate::app::model_preview) particles: Vec<LiveParticle>,
    /// `near fade range` and its cutoff.
    pub(in crate::app::model_preview) near: [f32; 2],
    /// `psBlackOrWhitePointEnabled`: set unless the black point is ever 0
    /// and the white point 1 (`c_particle_emitter_definition` postprocess).
    pub(in crate::app::model_preview) black_or_white: bool,
}

impl ParticleField {
    /// Each emitter of `system`'s live particles, at each of its locations,
    /// for its material to draw.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) fn live_emitters(
        &self,
        index: usize,
        system: &ParticleSystem,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
    ) -> Vec<LiveEmitter> {
        let mut draws = Vec::new();
        if system.hidden || system.look.material.is_none() {
            return draws;
        }
        if system.child {
            for run in self.instances.get(index).into_iter().flatten() {
                system.live_emitters_at(
                    &run.running,
                    (run.origin, run.axes, run.start),
                    values,
                    view,
                    time,
                    &mut draws,
                );
            }
            return draws;
        }
        let Some(running) = self.running.get(index) else {
            return draws;
        };
        let (origin, axes) = location(system, bones);
        system.live_emitters_at(
            running,
            (origin, axes, onset),
            values,
            view,
            time,
            &mut draws,
        );
        draws
    }
}

impl ParticleSystem {
    /// `particle_update.fx`'s outputs for each live particle of a location.
    fn live_emitters_at(
        &self,
        running: &[RunningEmitter],
        (origin, axes, onset): ([f32; 3], [[f32; 3]; 3], f32),
        values: Option<&FunctionValues>,
        view: &ViewFrame,
        time: f32,
        draws: &mut Vec<LiveEmitter>,
    ) {
        let system = self;
        let look = &*system.look;
        for (emitter, running) in system.emitters.iter().zip(running) {
            let base = ParticleState {
                system_age: system.times.age(system.event, time - onset).unwrap_or(1.0),
                game_time: particle_game_time(look.later_look, time),
                effect_a: system_state(system, values, time, time - onset)
                    .map_or(0.0, |state| state.effect_a),
                effect_b: system_state(system, values, time, time - onset)
                    .map_or(1.0, |state| state.effect_b),
                location_seed: system.location_seed,
                lod: {
                    let away = minus(origin, view.eye);
                    lod_amount(system.lod, dot(away, away).sqrt())
                },
                ..Default::default()
            };
            const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            let (offset, turned) = if system.world {
                ([0.0; 3], IDENTITY)
            } else {
                emitter_frame(emitter, origin, axes, &base)
            };
            // `random u/v mirror` reads bit 0 of `256·random`.
            let mirror = |flip: bool| if flip { 1.5 / 256.0 } else { 0.5 / 256.0 };
            let mut live = Vec::with_capacity(running.particles.len());
            for particle in &running.particles {
                let state = particle.state(base);
                let tint = emitter.tint.color(&state);
                let color = look.color.color(&state);
                let alpha =
                    (emitter.alpha.scalar(&state) * look.alpha.scalar(&state)).clamp(0.0, 1.0);
                let rgb: [f32; 3] =
                    std::array::from_fn(|axis| (tint[axis] * color[axis]).clamp(0.0, 1.0));
                let [size_factor, aspect_factor] = emitter
                    .update_terms
                    .as_ref()
                    .map_or([1.0; 2], |terms| terms.size_and_aspect(&state));
                let white_point = emitter
                    .update_terms
                    .as_ref()
                    .and_then(|terms| terms.white_point())
                    .map_or(1.0, |white| white.scalar(&state));
                live.push(LiveParticle {
                    position: particle.position,
                    size: emitter.size.scalar(&state) * emitter.scale.scalar(&state) * size_factor,
                    velocity: particle.velocity,
                    aspect: look.aspect.scalar(&state) * aspect_factor,
                    random: [
                        mirror(particle.flip[0]),
                        mirror(particle.flip[1]),
                        particle.seeds[0],
                        particle.seeds[1],
                    ],
                    random2: [
                        particle.seeds[2],
                        particle.seeds[3],
                        particle.seeds[4],
                        particle.seeds[5],
                    ],
                    physical_rotation: if emitter.rotation_given {
                        0.0
                    } else {
                        particle.rotation.rem_euclid(1.0)
                    },
                    manual_rotation: emitter.rotation.scalar(&state).rem_euclid(1.0),
                    animated_frame: particle.frame.rem_euclid(1.0),
                    manual_frame: look.frame.scalar(&state).rem_euclid(1.0),
                    birth_time: particle.emission_time,
                    inverse_lifespan: particle.inverse_lifespan,
                    age: particle.age,
                    intensity: look.intensity.scalar(&state),
                    rotation_rate: particle.rotation_rate,
                    frame_rate: particle.frame_rate,
                    simulation: [0.0; 2],
                    // A black point of 1 would pick a sixth corner set, past
                    // the five; GLSL reads nothing sound there.
                    black_point: emitter.black_point.scalar(&state).clamp(0.0, 0.998),
                    white_point: white_point.clamp(0.0, 1.0),
                    palette: look.palette.scalar(&state).clamp(0.0, 1.0),
                    axis: particle.axis,
                    color: [rgb[0], rgb[1], rgb[2], alpha],
                    initial_color: [1.0, 1.0, 1.0, 0.0],
                });
            }
            if live.is_empty() {
                continue;
            }
            draws.push(LiveEmitter {
                to_world: [turned[0], turned[1], turned[2], offset],
                particles: live,
                near: [system.near_range.recip(), system.near_cutoff],
                black_or_white: emitter
                    .black_point
                    .function
                    .as_constant()
                    .is_none_or(|value| value != 0.0)
                    || emitter
                        .update_terms
                        .as_ref()
                        .and_then(|terms| terms.white_point())
                        .is_some_and(|white| {
                            white
                                .function
                                .as_constant()
                                .is_none_or(|value| value != 1.0)
                        }),
            });
        }
    }
}
