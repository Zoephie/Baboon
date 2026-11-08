//! Halo 4's packed particle draws, built from the same fields.

use super::*;

/// One Halo 4 emitter's draw: its particles packed as the card holds them,
/// and the matrix from their space to the world.
#[derive(Debug, Clone)]
pub(crate) struct H4EmitterDraw {
    /// `vs_emitter_to_world_matrix`: forward, left, up and position.
    pub(crate) to_world: [[f32; 3]; 4],
    pub(crate) states: Vec<[u32; 20]>,
    /// `near fade range` and its cutoff.
    pub(crate) near: [f32; 2],
    /// `psBlackOrWhitePointEnabled`: set unless the black point is ever 0
    /// and the white point 1 (`c_particle_emitter_definition` postprocess).
    pub(crate) black_or_white: bool,
}

impl H3ParticleField {
    /// Halo 4: each emitter of `system`'s particles as `pack_particle_state`
    /// writes them, for its material to draw.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn h4_draws(
        &self,
        index: usize,
        system: &H3ParticleSystem,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
    ) -> Vec<H4EmitterDraw> {
        let mut draws = Vec::new();
        if system.hidden || system.look.h4.is_none() {
            return draws;
        }
        if system.child {
            for run in self.instances.get(index).into_iter().flatten() {
                system.h4_draws_at(&run.running, (run.origin, run.axes, run.start), values, view, time, &mut draws);
            }
            return draws;
        }
        let Some(running) = self.running.get(index) else {
            return draws;
        };
        let (origin, axes) = location(system, bones);
        system.h4_draws_at(running, (origin, axes, onset), values, view, time, &mut draws);
        draws
    }
}

impl H3ParticleSystem {
    /// `particle_update.fx`'s outputs for each live particle of a location,
    /// packed.
    fn h4_draws_at(
        &self,
        running: &[RunningEmitter],
        (origin, axes, onset): ([f32; 3], [[f32; 3]; 3], f32),
        values: Option<&FunctionValues>,
        view: &ViewFrame,
        time: f32,
        draws: &mut Vec<H4EmitterDraw>,
    ) {
        use super::super::h4::particles::{H4ParticleState, pack_particle_state};
        let system = self;
        let look = &*system.look;
        for (emitter, running) in system.emitters.iter().zip(running) {
            let base = H3ParticleState {
                system_age: system.times.age(system.event, time - onset).unwrap_or(1.0),
                game_time: particle_game_time(look.reach, time),
                effect_a: system_state(system, values, time, time - onset).map_or(0.0, |state| state.effect_a),
                effect_b: system_state(system, values, time, time - onset).map_or(1.0, |state| state.effect_b),
                location_seed: system.location_seed,
                lod: {
                    let away = minus(origin, view.eye);
                    lod_amount(system.lod, dot(away, away).sqrt())
                },
                ..Default::default()
            };
            const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            let (offset, turned) = if system.world { ([0.0; 3], IDENTITY) } else { emitter_frame(emitter, origin, axes, &base) };
            // `random u/v mirror` reads bit 0 of `256·random`.
            let mirror = |flip: bool| if flip { 1.5 / 256.0 } else { 0.5 / 256.0 };
            let mut states = Vec::with_capacity(running.particles.len());
            for particle in &running.particles {
                let state = particle.state(base);
                let tint = emitter.tint.color(&state);
                let color = look.color.color(&state);
                let alpha = (emitter.alpha.scalar(&state) * look.alpha.scalar(&state)).clamp(0.0, 1.0);
                let rgb: [f32; 3] = std::array::from_fn(|axis| (tint[axis] * color[axis]).clamp(0.0, 1.0));
                let [size_factor, aspect_factor] = emitter.h4.as_ref().map_or([1.0; 2], |h4| h4.size_and_aspect(&state));
                let white_point = emitter.h4.as_ref().and_then(|h4| h4.white_point.as_ref()).map_or(1.0, |white| white.scalar(&state));
                let packed = H4ParticleState {
                    position: particle.position,
                    size: emitter.size.scalar(&state) * emitter.scale.scalar(&state) * size_factor,
                    velocity: particle.velocity,
                    aspect: look.aspect.scalar(&state) * aspect_factor,
                    random: [mirror(particle.flip[0]), mirror(particle.flip[1]), particle.seeds[0], particle.seeds[1]],
                    random2: [particle.seeds[2], particle.seeds[3], particle.seeds[4], particle.seeds[5]],
                    physical_rotation: if emitter.rotation_given { 0.0 } else { particle.rotation.rem_euclid(1.0) },
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
                };
                states.push(pack_particle_state(&packed));
            }
            if states.is_empty() {
                continue;
            }
            draws.push(H4EmitterDraw {
                to_world: [turned[0], turned[1], turned[2], offset],
                states,
                near: [system.near_range.recip(), system.near_cutoff],
                black_or_white: emitter.black_point.function.as_constant().is_none_or(|value| value != 0.0)
                    || emitter.h4.as_ref().and_then(|h4| h4.white_point.as_ref()).is_some_and(|white| {
                        white.function.as_constant().is_none_or(|value| value != 1.0)
                    }),
            });
        }
    }
}
