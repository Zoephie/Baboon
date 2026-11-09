//! The simulation: runtime particles, emitters and fields, emitting and stepping a fixed step at a time.

use super::super::{frame_steps, resume_point};
use super::emitter::{EmissionShape, Emitter, ParticleTrigger, emit_from_points};
use super::look::ParticleLook;
use super::movement::turbulence_push;
use super::property::{ParticleState, lod_amount};
use super::rules::{
    MOST_PARTICLES, PRE_ROLL, ParticleController, ParticleUpdate, SEEDS, SYSTEM_SEEDS,
    comes_to_rest, particle_game_time, starting_rotation,
};
use super::shader::ParticleCorner;
use super::system::ParticleSystem;
use super::{
    EffectOnsets, FunctionValues, Gen3StripAt, ViewFrame, clamped_unit, cross, dot, high_bits,
    lcg_step, minus,
};
use std::sync::Arc;

/// `real_internal_random_range`'s generator, seeded so a preview repeats.
#[derive(Debug, Clone, Copy)]
pub(in crate::app::model_preview) struct Random(pub(in crate::app::model_preview) u32);

impl Random {
    pub(in crate::app::model_preview) fn next(&mut self) -> f32 {
        self.bits() as f32 / 65_535.0
    }

    /// `_random`: the next state's high 16 bits, which [`Self::next`] scales.
    pub(in crate::app::model_preview) fn bits(&mut self) -> u32 {
        self.0 = lcg_step(self.0);
        high_bits(self.0)
    }

    pub(in crate::app::model_preview) fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }

    pub(super) fn direction(&mut self) -> [f32; 3] {
        let z = self.range(-1.0, 1.0);
        let (sin, cos) = self.range(0.0, std::f32::consts::TAU).sin_cos();
        let flat = (1.0 - z * z).max(0.0).sqrt();
        [flat * cos, flat * sin, z]
    }
}

/// `initialize_particle`: where a shape puts a particle and which way it goes.
/// Reach's volumes (the 360's 0x82EE3C98, MCC sub_1405C9A40) draw from
/// `get_local_random_seed_address`, MCC's the same generator as [`Random`],
/// so the emitter's own stream stands in. With `volume_random` (`volume
/// emitter particle velocities are random`) they head any way, not outward.
pub(super) fn emit(
    shape: EmissionShape,
    radius: f32,
    angle_degrees: f32,
    volume_random: bool,
    random: &mut Random,
) -> ([f32; 3], [f32; 3]) {
    let angle = angle_degrees.to_radians();
    let turn = random.range(0.0, std::f32::consts::TAU);
    let (sin_turn, cos_turn) = turn.sin_cos();
    let spread = |cone: f32| {
        let (sin, cos) = cone.sin_cos();
        [cos, sin_turn * sin, cos_turn * sin]
    };
    match shape {
        EmissionShape::Sprayer => ([0.0; 3], spread(random.range(0.0, angle))),
        EmissionShape::Disc => {
            let out = [0.0, sin_turn, cos_turn];
            (out.map(|axis| axis * radius), out)
        }
        EmissionShape::Globe => {
            let out = random.direction();
            (out.map(|axis| axis * radius), out)
        }
        EmissionShape::Implode => {
            let out = random.direction();
            (out.map(|axis| axis * radius), out.map(|axis| -axis))
        }
        EmissionShape::Tube => ([0.0, sin_turn * radius, cos_turn * radius], spread(angle)),
        EmissionShape::Halo => {
            let velocity = spread(random.range(0.0, angle));
            (random.direction().map(|axis| axis * radius), velocity)
        }
        EmissionShape::Debris => ([0.0, sin_turn * radius, cos_turn * radius], [0.0, 0.0, 1.0]),
        EmissionShape::Line | EmissionShape::UnweightedLine => {
            let cone = random.range(0.0, angle);
            ([0.0, random.range(-radius, radius), 0.0], spread(cone))
        }
        EmissionShape::Cube => {
            // Drawn z, y, x.
            let [z, y, x] = [(); 3].map(|_| random.range(-radius, radius));
            let place = [x, y, z];
            (place, volume_heading(volume_random, place, random))
        }
        EmissionShape::Cylinder => {
            // Two units long whatever the radius; `axis scale` stretches it.
            let along = random.range(-1.0, 1.0);
            let (sin, cos) = random.range(0.0, std::f32::consts::TAU).sin_cos();
            let out = random.range(0.0, 1.0) * radius;
            let place = [along, out * sin, out * cos];
            (place, volume_heading(volume_random, place, random))
        }
        EmissionShape::Plane => (
            [(); 3].map(|_| random.range(-1.0, 1.0) * radius),
            [1.0, 0.0, 0.0],
        ),
        // An attached effect has no impact, whose size is all that spreads these.
        EmissionShape::ImpactContour => {
            let cone = angle;
            let sign = if random.next() < 0.5 { -1.0 } else { 1.0 };
            let side = cone.sin() * sign;
            (
                [0.0; 3],
                if random.next() < 0.5 {
                    [cone.cos(), side, 0.0]
                } else {
                    [cone.cos(), 0.0, side]
                },
            )
        }
        EmissionShape::ImpactArea => {
            let cone = angle;
            let (sin, cos) = random.range(0.0, std::f32::consts::TAU).sin_cos();
            ([0.0; 3], [cone.cos(), sin * cone.sin(), cos * cone.sin()])
        }
        EmissionShape::Breakable | EmissionShape::CustomPoints => ([0.0; 3], [0.0; 3]),
    }
}

/// A Reach volume's heading, normalized later: out from its centre, or with
/// `volume_random` any way at all.
fn volume_heading(volume_random: bool, place: [f32; 3], random: &mut Random) -> [f32; 3] {
    if volume_random {
        [(); 3].map(|_| random.range(-1.0, 1.0))
    } else {
        place
    }
}

/// Reach's emitter `axis scale`, which tool's postprocess makes ones when it
/// is all nothing.
pub(super) fn reach_axis_scale(raw: [f32; 3]) -> [f32; 3] {
    if raw == [0.0; 3] { [1.0; 3] } else { raw }
}

#[derive(Debug, Clone)]
pub(super) struct Particle {
    /// Unique among its emitter's, for what follows it.
    pub(super) id: u64,
    /// In contact, so that a collision's attachments play once.
    pub(super) colliding: bool,
    /// Touched a surface it could stand on this step.
    pub(super) landed: bool,
    /// Slow enough on one to have stopped for good.
    pub(super) resting: bool,
    /// Not yet through an update: the card's first after a spawn moves nothing.
    pub(super) spawned_this_frame: bool,
    /// Moved from the processor to the card when it came to rest.
    pub(super) on_card: bool,
    pub(super) axis: [f32; 3],
    pub(super) age: f32,
    pub(super) inverse_lifespan: f32,
    pub(super) position: [f32; 3],
    pub(super) velocity: [f32; 3],
    pub(super) rotation: f32,
    pub(super) rotation_rate: f32,
    pub(super) frame: f32,
    pub(super) frame_rate: f32,
    pub(super) seeds: [f32; SEEDS],
    pub(super) emission_time: f32,
    pub(super) flip: [bool; 3],
    /// Which of the look's `sequences` it draws.
    pub(super) sequence: usize,
    pub(super) system_seeds: [f32; SYSTEM_SEEDS],
}

#[derive(Debug, Clone)]
pub(super) struct RunningEmitter {
    pub(super) particles: Vec<Particle>,
    made: u64,
    accumulator: f32,
    started: bool,
    /// Where it was a step ago, for what it spawns on the way.
    previous: Option<[f32; 3]>,
    random: Random,
    /// `c_particle_system::get_random_seed`: picked once as the system is
    /// made and shared by all its emitters.
    system_seeds: [f32; SYSTEM_SEEDS],
}

/// Everything in flight on a preview.
#[derive(Debug, Default)]
pub(crate) struct ParticleField {
    time: Option<f32>,
    /// Per system, per emitter.
    pub(super) running: Vec<Vec<RunningEmitter>>,
    /// Per system, what particles have started of it.
    pub(super) instances: Vec<Vec<ChildInstance>>,
    /// Per system, where the particles something follows are.
    tracks: Vec<Vec<ParticleTrack>>,
}

/// Where a live particle is, for what is attached to it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ParticleTrack {
    pub(crate) id: u64,
    pub(crate) place: [f32; 3],
    velocity: [f32; 3],
}

/// `effect_new_attached_from_particle`: one run of a system, attached to a
/// particle while that lasts. A death's has no particle left to follow.
#[derive(Debug, Clone)]
pub(super) struct ChildInstance {
    follows: Option<u64>,
    parent: usize,
    pub(super) origin: [f32; 3],
    pub(super) axes: [[f32; 3]; 3],
    pub(super) start: f32,
    pub(super) running: Vec<RunningEmitter>,
    /// The effect A scale it was made with.
    effect_a: f32,
    /// Halo 2's carried systems: the lifespan of the particle that made it,
    /// over which its age runs to 1, and then it emits no more.
    lifetime: Option<f32>,
}

/// `particle_block.h`'s limit on systems.
const MOST_INSTANCES: usize = 320;

fn fresh_emitters(system: &ParticleSystem, seed: u32) -> Vec<RunningEmitter> {
    let system_seeds = std::array::from_fn(|at| Random(at as u32 + 1 + seed / 7).next());
    let emitters = system.emitters.iter().enumerate();
    emitters
        .map(|(index, _)| RunningEmitter {
            particles: Vec::new(),
            made: 0,
            accumulator: 0.0,
            started: false,
            previous: None,
            random: Random(seed.wrapping_mul(977) ^ (index as u32).wrapping_mul(131)),
            system_seeds,
        })
        .collect()
}

/// A location's axes, looking along what a particle was doing.
pub(super) fn axes_along(forward: [f32; 3]) -> [[f32; 3]; 3] {
    let forward = if dot(forward, forward) > 0.0 {
        clamped_unit(forward)
    } else {
        [1.0, 0.0, 0.0]
    };
    let reference = if forward[2].abs() < 0.99 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let left = clamped_unit(cross(reference, forward));
    [forward, left, cross(forward, left)]
}

/// A particle's birth, death or hit: its id, place and velocity.
type ParticleEvent = (u64, [f32; 3], [f32; 3]);

/// One step of one location, `dt` long, returning what its particles started
/// and ended.
#[allow(clippy::too_many_arguments)]
fn step_location(
    system: &ParticleSystem,
    emitters: &mut [RunningEmitter],
    state: Option<ParticleState>,
    (origin, axes): ([f32; 3], [[f32; 3]; 3]),
    now: f32,
    dt: f32,
    ground: f32,
    (births, deaths, hits): (
        &mut Vec<ParticleEvent>,
        &mut Vec<ParticleEvent>,
        &mut Vec<ParticleEvent>,
    ),
    tracks: &mut Vec<ParticleTrack>,
) {
    tracks.clear();
    for (at, (emitter, running)) in system.emitters.iter().zip(emitters).enumerate() {
        // What was made outlives its event, and goes on falling and landing.
        let rest = ParticleState {
            system_age: 1.0,
            game_time: particle_game_time(system.look.reach, now),
            ..Default::default()
        };
        let frame = emitter_frame(emitter, origin, axes, state.as_ref().unwrap_or(&rest));
        let home = if system.world { origin } else { [0.0; 3] };
        // The ground, in whichever frame the particles live.
        let plane = if system.world {
            ([0.0, 0.0, 1.0], -ground)
        } else {
            (
                std::array::from_fn(|row| frame.1[row][2]),
                frame.0[2] - ground,
            )
        };
        if let Some(mut state) = state {
            state.system_seeds = running.system_seeds;
            emitter.pulse(
                &system.look,
                state,
                running,
                dt,
                frame,
                system.world,
                births,
                at,
            );
            let ground = Some((plane, system.look.dies_on_ground, &mut *hits));
            emitter.step(
                state,
                &mut running.particles,
                dt,
                home,
                deaths,
                ground,
                system.look.dies_at_rest,
            );
        } else {
            // The next run of its event is a new location's, with its burst.
            running.started = false;
            running.previous = None;
            let ground = Some((plane, system.look.dies_on_ground, &mut *hits));
            emitter.step(
                rest,
                &mut running.particles,
                dt,
                home,
                deaths,
                ground,
                system.look.dies_at_rest,
            );
        }
        if !(system.attached.is_empty() && system.carried.is_empty()) {
            let (offset, turned) = frame;
            let to_world = |vector: [f32; 3]| -> [f32; 3] {
                std::array::from_fn(|axis| (0..3).map(|row| turned[row][axis] * vector[row]).sum())
            };
            tracks.extend(running.particles.iter().map(|particle| {
                let (place, velocity) = if system.world {
                    (particle.position, particle.velocity)
                } else {
                    let moved = to_world(particle.position);
                    (
                        std::array::from_fn(|axis| offset[axis] + moved[axis]),
                        to_world(particle.velocity),
                    )
                };
                ParticleTrack {
                    id: particle.id,
                    place,
                    velocity,
                }
            }));
        }
    }
}

/// The states of a system this step, or none while it should not emit.
pub(super) fn system_state(
    system: &ParticleSystem,
    values: Option<&FunctionValues>,
    time: f32,
    since: f32,
) -> Option<ParticleState> {
    let read = |name: &str| {
        if name.is_empty() {
            1.0
        } else {
            values
                .and_then(|values| values.get(name))
                .copied()
                .unwrap_or(0.0)
        }
    };
    let effect_a = read(&system.scale_by);
    if effect_a <= 0.0 {
        return None;
    }
    Some(ParticleState {
        system_age: system.times.age(system.event, since)?,
        game_time: particle_game_time(system.look.reach, time),
        effect_a,
        effect_b: read(&system.secondary_by),
        location_seed: system.location_seed,
        ..Default::default()
    })
}

/// `calc_lod_amount` from the camera. Nothing pulses at none.
fn with_lod(
    system: &ParticleSystem,
    state: Option<ParticleState>,
    origin: [f32; 3],
    eye: [f32; 3],
) -> Option<ParticleState> {
    let away = minus(origin, eye);
    let lod = lod_amount(system.lod, dot(away, away).sqrt());
    state
        .filter(|_| lod > 0.0)
        .map(|state| ParticleState { lod, ..state })
}

/// `system_state` for a step `dt` long ending at `time`, and for an event that
/// came and went within it: a frame can hold more than one of the game's
/// ticks, and its one-tick events are many.
fn system_state_stepped(
    system: &ParticleSystem,
    values: Option<&FunctionValues>,
    time: f32,
    dt: f32,
    onset: f32,
) -> Option<ParticleState> {
    let since = time - onset;
    system_state(system, values, time, since).or_else(|| {
        let inside = system
            .times
            .begins_within(system.event, since - dt, since)?;
        system_state(system, values, time, inside)
    })
}

impl Emitter {
    /// `initialize_particle`'s shape, then each axis of the place by `axis
    /// scale` (Reach's, after every shape; ones elsewhere).
    pub(super) fn emitted(
        &self,
        radius: f32,
        angle: f32,
        random: &mut Random,
    ) -> ([f32; 3], [f32; 3]) {
        let (position, velocity) = if self.shape == EmissionShape::CustomPoints {
            emit_from_points(&self.points, radius, angle, random)
        } else {
            emit(self.shape, radius, angle, self.volume_random, random)
        };
        (
            std::array::from_fn(|axis| position[axis] * self.axis_scale[axis]),
            velocity,
        )
    }

    /// `c_particle_emitter::pulse` and `spawn_particle`.
    fn pulse(
        &self,
        look: &ParticleLook,
        state: ParticleState,
        running: &mut RunningEmitter,
        dt: f32,
        frame: ([f32; 3], [[f32; 3]; 3]),
        world: bool,
        started: &mut Vec<(u64, [f32; 3], [f32; 3])>,
        at: usize,
    ) {
        // `c_particle_emitter::pulse`: spread through the step, not bunched at its end.
        running.accumulator = running.accumulator.fract();
        if !running.started {
            running.started = true;
            running.accumulator += self.starting.scalar(&state);
        }
        if !self.one_at_a_time || running.particles.is_empty() {
            running.accumulator += self.rate.scalar(&state) * dt;
        }
        let most = self.most.scalar(&state);
        if most > 0.0 {
            running.accumulator = running
                .accumulator
                .min(most - running.particles.len() as f32);
        }
        let (offset, turned) = frame;
        let previous = running.previous.unwrap_or(offset);
        running.previous = Some(offset);
        let count = (running.accumulator + self.spawn_slack).max(0.0).floor();
        let between = if count >= 1.0 {
            running.accumulator.recip()
        } else {
            0.0
        };
        // Each one spent comes off what is owed, which can leave it a hair
        // under nothing for the next pulse.
        running.accumulator -= count;
        for birth in 0..count as usize {
            if running.particles.len() >= MOST_PARTICLES {
                break;
            }
            let along = if self.bunched {
                1.0
            } else {
                birth as f32 * between
            };
            let random = &mut running.random;
            let seeds: [f32; SEEDS] = std::array::from_fn(|_| random.next());
            let mut particle_state = ParticleState {
                seeds,
                emission_time: state.system_age,
                ..state
            };
            particle_state.system_seeds = state.system_seeds;
            let (radius, angle) = (
                self.radius.scalar(&particle_state),
                self.angle.scalar(&particle_state),
            );
            let (position, mut velocity) = self.emitted(radius, angle, random);
            let length = (velocity[0].powi(2) + velocity[1].powi(2) + velocity[2].powi(2))
                .sqrt()
                .max(1e-6);
            let speed = self.speed.scalar(&particle_state);
            let speed = if speed == 0.0 && look.still_moves {
                0.001
            } else {
                speed
            };
            velocity = velocity.map(|axis| axis / length * speed);
            let to_world = |vector: [f32; 3]| -> [f32; 3] {
                std::array::from_fn(|axis| (0..3).map(|at| turned[at][axis] * vector[at]).sum())
            };
            let placed = {
                let moved = to_world(position);
                std::array::from_fn(|axis| {
                    previous[axis] + (offset[axis] - previous[axis]) * along + moved[axis]
                })
            };
            running.made += 1;
            let id = running.made << 8 | at as u64;
            started.push((id, placed, to_world(velocity)));
            let ahead = (1.0 - along) * dt;
            let (position, velocity) = if world {
                (placed, to_world(velocity))
            } else {
                (
                    std::array::from_fn(|axis| position[axis] + velocity[axis] * ahead),
                    velocity,
                )
            };
            let inverse_lifespan = 1.0 / self.lifespan.scalar(&particle_state).max(0.01);
            let mut flip = [
                random.next() >= 0.5,
                random.next() >= 0.5,
                random.next() >= 0.5,
            ];
            // One random word gives both: `& 1` mirrors v, `% count` the sequence.
            let sequence = if look.sequences.is_empty() {
                0
            } else {
                let word = (random.next() * 65536.0) as usize;
                flip[1] = word & 1 != 0;
                word % look.sequences.len()
            };
            let axis = self.axis.pick(
                self.reference_axis,
                self.axis_angle.scalar(&particle_state),
                random,
            );
            running.particles.push(Particle {
                id,
                colliding: false,
                landed: false,
                resting: false,
                spawned_this_frame: true,
                on_card: false,
                axis,
                age: if self.bunched {
                    0.0
                } else {
                    along * dt * inverse_lifespan
                },
                inverse_lifespan,
                position,
                velocity,
                rotation: starting_rotation(
                    look.random_rotation,
                    look.vertical
                        && look
                            .material
                            .as_ref()
                            .is_some_and(|material| material.turns_vertical_a_quarter()),
                    || random.next(),
                ),
                rotation_rate: self.rotation_rate.scalar(&particle_state),
                frame: 0.0,
                frame_rate: look.rate.scalar(&particle_state),
                seeds,
                emission_time: state.system_age,
                flip,
                sequence,
                system_seeds: state.system_seeds,
            });
        }
    }

    /// `update_particle_state`, a step at a time.
    pub(super) fn step(
        &self,
        state: ParticleState,
        particles: &mut Vec<Particle>,
        dt: f32,
        home: [f32; 3],
        ended: &mut Vec<(u64, [f32; 3], [f32; 3])>,
        ground: Option<(([f32; 3], f32), bool, &mut Vec<(u64, [f32; 3], [f32; 3])>)>,
        dies_at_rest: bool,
    ) {
        if self.update == ParticleUpdate::Gpu {
            return self.step_on_card(state, particles, dt, ended);
        }
        let mut ground = ground;
        for controller in &self.controllers {
            match controller {
                ParticleController::Flock => {
                    if let Some(flock) = &self.flock {
                        flock.update(particles, state, home, dt);
                    }
                }
                ParticleController::Collider => {
                    if let (Some(collider), Some((plane, dies, hits))) =
                        (&self.collider, ground.as_mut())
                    {
                        collider.update(particles, state, *plane, *dies, hits, dt);
                    }
                }
                ParticleController::Physics => {
                    if let Some(physics) = &self.physics {
                        physics.update(particles, state, dt);
                    }
                }
            }
        }
        // `c_particle_emitter::frame_advance`. The processor's turbulence
        // (`update_particles_turbulence`, the 360's 0x82EE0D80) does nothing;
        // what is on the card has the card's.
        let turbulence = self
            .turbulence
            .as_ref()
            .map(|turbulence| turbulence.xform(&state));
        particles.retain_mut(|particle| {
            if particle.on_card {
                // `set_shader_update_state` gives a processor emitter's no physics.
                return self.card_frame(particle, state, dt, [0.0; 3], turbulence, ended);
            }
            particle.age += particle.inverse_lifespan * dt;
            if self.persistent {
                particle.age = particle.age.fract();
            }
            if let Some(terms) = &self.update_terms {
                particle.age = terms.age(particle.age);
            }
            if particle.age > 1.0 || (particle.resting && dies_at_rest) {
                ended.push((particle.id, particle.position, particle.velocity));
                return false;
            }
            let state = particle.state(state);
            let acceleration = self.acceleration.around(&state);
            for axis in 0..3 {
                particle.velocity[axis] += acceleration[axis] * dt;
            }
            if particle.resting {
                return true;
            }
            if comes_to_rest(particle.landed, particle.velocity) {
                particle.resting = true;
                if self.card_at_rest {
                    particle.move_to_card();
                }
                return true;
            }
            particle.landed = false;
            let travel = self
                .update_terms
                .as_ref()
                .map_or(dt, |terms| terms.travel(dt));
            for axis in 0..3 {
                particle.position[axis] += particle.velocity[axis] * travel;
            }
            particle.rotation = (particle.rotation + particle.rotation_rate * dt).rem_euclid(1.0);
            particle.frame = (particle.frame + particle.frame_rate * dt).rem_euclid(1.0);
            true
        });
    }
}

impl Emitter {
    /// `particle_update.hlsl`'s `update_particle_state`, for an emitter on the
    /// card: a particle's first update after its spawn moves nothing; then it
    /// ages, moves by the velocity it had, and only then is accelerated. Its
    /// physics is the emitter's, and none without the `physics` movement.
    fn step_on_card(
        &self,
        state: ParticleState,
        particles: &mut Vec<Particle>,
        dt: f32,
        ended: &mut Vec<ParticleEvent>,
    ) {
        let physical = self.controllers.contains(&ParticleController::Physics);
        let physics = self.physics.as_ref().filter(|_| physical);
        let constants = physics.map_or([0.0; 3], |physics| physics.on_card(&state));
        let turbulence = self
            .turbulence
            .as_ref()
            .map(|turbulence| turbulence.xform(&state));
        particles.retain_mut(|particle| {
            self.card_frame(particle, state, dt, constants, turbulence, ended)
        });
    }

    /// One frame of `update_particle_state` for one particle, with the
    /// emitter's gravity, air friction and rotational friction, and Reach's
    /// turbulence at this frame's `xform`; false once it dies.
    fn card_frame(
        &self,
        particle: &mut Particle,
        state: ParticleState,
        dt: f32,
        [gravity, air_friction, rotational_friction]: [f32; 3],
        turbulence: Option<[f32; 4]>,
        ended: &mut Vec<ParticleEvent>,
    ) -> bool {
        let dt = if std::mem::take(&mut particle.spawned_this_frame) {
            0.0
        } else {
            dt
        };
        particle.age += particle.inverse_lifespan * dt;
        if let Some(terms) = &self.update_terms {
            particle.age = terms.age(particle.age);
        }
        if particle.age >= 1.0 {
            ended.push((particle.id, particle.position, particle.velocity));
            return false;
        }
        let (position, velocity) = (particle.position, particle.velocity);
        let travel = self
            .update_terms
            .as_ref()
            .map_or(dt, |terms| terms.travel(dt));
        particle.position = std::array::from_fn(|axis| position[axis] + velocity[axis] * travel);
        let state = particle.state(state);
        if let (Some(turbulence), Some(xform)) = (&self.turbulence, turbulence) {
            let strength = turbulence.strength.scalar(&state);
            let push = turbulence_push(
                &turbulence.image,
                xform,
                [particle.emission_time, particle.seeds[4]],
                strength * dt,
            );
            particle.position = std::array::from_fn(|axis| particle.position[axis] + push[axis]);
        }
        let acceleration = if self.card_lerps {
            self.acceleration.vector(&state)
        } else {
            self.acceleration.around(&state)
        };
        particle.velocity = std::array::from_fn(|axis| velocity[axis] + acceleration[axis] * dt);
        particle.velocity[2] -= gravity * dt;
        let air = (air_friction * dt).clamp(0.0, 1.0);
        particle.velocity = particle.velocity.map(|axis| axis - air * axis);
        let spin = (rotational_friction * dt).clamp(0.0, 1.0);
        particle.rotation_rate -= spin * particle.rotation_rate;
        particle.rotation = (particle.rotation + particle.rotation_rate * dt).rem_euclid(1.0);
        particle.frame = (particle.frame + particle.frame_rate * dt).rem_euclid(1.0);
        true
    }
}

impl Particle {
    /// What its properties read of it, over its system's states. Its speed
    /// is Reach's `particle velocity`, which no Halo 3 input reads.
    pub(super) fn state(&self, base: ParticleState) -> ParticleState {
        ParticleState {
            age: self.age,
            seeds: self.seeds,
            system_seeds: self.system_seeds,
            emission_time: self.emission_time,
            rotation: self.rotation,
            velocity: 0.1 * dot(self.velocity, self.velocity).sqrt(),
            ..base
        }
    }

    /// `frame_advance`'s move to the card for a particle at rest: it keeps a
    /// thousandth of a unit a second along its way (or straight down), stops
    /// turning, and is queued afresh, so the card's next update moves nothing.
    fn move_to_card(&mut self) {
        let speed = dot(self.velocity, self.velocity).sqrt();
        let way = if speed > 0.0 {
            self.velocity.map(|axis| axis / speed)
        } else {
            [0.0, 0.0, -1.0]
        };
        self.velocity = way.map(|axis| axis * 0.001);
        self.rotation_rate = 0.0;
        self.on_card = true;
        self.spawned_this_frame = true;
    }
}

impl ParticleField {
    /// Run everything up to `time`, a frame at a time as `effects_frame_advance`
    /// runs the particle systems. A new field starts a pre-roll short, so a
    /// held pose has particles in flight from its first frame.
    pub(crate) fn advance(
        &mut self,
        systems: &[Arc<ParticleSystem>],
        values: impl Fn(u8) -> Option<FunctionValues>,
        onsets: &mut EffectOnsets,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        ground: f32,
        time: f32,
    ) {
        if self.running.len() != systems.len() || self.time.is_none_or(|last| time < last) {
            self.running = systems
                .iter()
                .enumerate()
                .map(|(at, system)| fresh_emitters(system, at as u32))
                .collect();
            self.instances = vec![Vec::new(); systems.len()];
            self.tracks = vec![Vec::new(); systems.len()];
            self.time = Some(time - PRE_ROLL);
        }
        let Some(now) = self.time else {
            return;
        };
        let mut now = resume_point(now, time, PRE_ROLL);
        // By the frame's clock, which the steps trail: a function that has
        // just come on must not read as having come on in their future.
        let onset_of: Vec<f32> = systems
            .iter()
            .map(|system| {
                let scope = values(system.object);
                onsets.of(system.object, &system.scale_by, scope.as_ref(), time)
            })
            .collect();
        for dt in frame_steps(now, time) {
            now += dt;
            let mut spawned = Vec::new();
            for (index, system) in systems.iter().enumerate() {
                let (mut births, mut deaths, mut hits) = (Vec::new(), Vec::new(), Vec::new());
                if system.child {
                    let mut runs = std::mem::take(&mut self.instances[index]);
                    let mut no_tracks = Vec::new();
                    let tracks = &self.tracks;
                    runs.retain_mut(|run| {
                        let age = now - run.start;
                        if let Some(followed) = run.follows {
                            let found =
                                tracks[run.parent].iter().find(|track| track.id == followed);
                            match found {
                                Some(track) => {
                                    run.origin = track.place;
                                    run.axes = axes_along(track.velocity);
                                }
                                None => run.follows = None,
                            }
                        }
                        // Its first step runs whatever its length: an event of a
                        // tick can be over before a frame ends.
                        let first = age <= dt * 1.5;
                        let alive = first
                            || run
                                .lifetime
                                .map_or(age < system.times.once(), |lifetime| age < lifetime);
                        if system.die_with_effect && !alive {
                            run.running
                                .iter_mut()
                                .for_each(|running| running.particles.clear());
                        }
                        let state = alive
                            .then(|| system_state_stepped(system, None, now, dt, run.start))
                            .flatten();
                        let state = state.map(|state| ParticleState {
                            effect_a: run.effect_a,
                            system_age: run
                                .lifetime
                                .map_or(state.system_age, |lifetime| age / lifetime),
                            ..state
                        });
                        let state = with_lod(system, state, run.origin, eye);
                        let (place, events) = (
                            (run.origin, run.axes),
                            (&mut births, &mut deaths, &mut hits),
                        );
                        step_location(
                            system,
                            &mut run.running,
                            state,
                            place,
                            now,
                            dt,
                            ground,
                            events,
                            &mut no_tracks,
                        );
                        alive
                            || run
                                .running
                                .iter()
                                .any(|running| !running.particles.is_empty())
                    });
                    self.instances[index] = runs;
                    continue;
                }
                let scope = values(system.object);
                let onset = onset_of[index];
                let state = system_state_stepped(system, scope.as_ref(), now, dt, onset);
                let (origin, axes) = location(system, bones);
                let state = with_lod(system, state, origin, eye);
                let (running, tracks) = (&mut self.running[index], &mut self.tracks[index]);
                let events = (&mut births, &mut deaths, &mut hits);
                step_location(
                    system,
                    running,
                    state,
                    (origin, axes),
                    now,
                    dt,
                    ground,
                    events,
                    tracks,
                );
                let off = !system.scale_by.is_empty()
                    && !scope
                        .as_ref()
                        .and_then(|values| values.get(&system.scale_by))
                        .is_some_and(|value| *value > 0.0);
                if system.die_with_effect && (off || system.times.is_over(now - onset)) {
                    self.running[index]
                        .iter_mut()
                        .for_each(|running| running.particles.clear());
                }
                let effect_a = state.map_or(1.0, |state| state.effect_a);
                for (trigger, offset) in &system.attached {
                    let events = match trigger {
                        ParticleTrigger::Birth | ParticleTrigger::Carried => &births,
                        ParticleTrigger::Death => &deaths,
                        ParticleTrigger::Collision => &hits,
                    };
                    let follow =
                        matches!(trigger, ParticleTrigger::Birth | ParticleTrigger::Carried);
                    spawned.extend(events.iter().map(|&(id, place, velocity)| {
                        let speed = dot(velocity, velocity).sqrt();
                        let (effect_a, lifetime) = match trigger {
                            // The game makes them under the same effect.
                            ParticleTrigger::Carried => (effect_a, self.lifespan_of(index, id)),
                            // A hit reports the velocity it bounced away with.
                            ParticleTrigger::Death | ParticleTrigger::Collision
                                if system.look.death_scaled_by_speed =>
                            {
                                ((speed - 0.5).clamp(0.0, 1.0), None)
                            }
                            _ => (1.0, None),
                        };
                        (
                            (index + offset, index),
                            follow.then_some(id),
                            (place, velocity),
                            system.world,
                            origin,
                            axes,
                            effect_a,
                            lifetime,
                        )
                    }));
                }
            }
            for (
                (index, parent),
                follows,
                (place, velocity),
                world,
                origin,
                axes,
                effect_a,
                lifetime,
            ) in spawned
            {
                let Some(system) = systems.get(index) else {
                    continue;
                };
                let runs = &mut self.instances[index];
                if runs.len() >= MOST_INSTANCES {
                    continue;
                }
                // What a death reports lies wherever its particle lived.
                let turn = |vector: [f32; 3]| -> [f32; 3] {
                    std::array::from_fn(|axis| (0..3).map(|at| axes[at][axis] * vector[at]).sum())
                };
                let (place, velocity) = if world {
                    (place, velocity)
                } else {
                    (
                        std::array::from_fn(|axis| origin[axis] + turn(place)[axis]),
                        turn(velocity),
                    )
                };
                let seed = (runs.len() as u32 + 1)
                    .wrapping_mul(index as u32 + 1)
                    .wrapping_add((now * 30.0) as u32);
                runs.push(ChildInstance {
                    follows,
                    parent,
                    origin: place,
                    axes: axes_along(velocity),
                    start: now,
                    running: fresh_emitters(system, seed),
                    effect_a,
                    lifetime,
                });
            }
        }
        self.time = Some(time);
    }

    /// The lifespan of a live particle of system `index`, by its id.
    fn lifespan_of(&self, index: usize, id: u64) -> Option<f32> {
        let particles = self
            .running
            .get(index)?
            .iter()
            .flat_map(|running| &running.particles);
        particles
            .filter(|particle| particle.id == id)
            .map(|particle| particle.inverse_lifespan.recip())
            .next()
    }

    /// The quads of a system, in a world-space triangle list, at the area
    /// screen effects' exposure `stops`.
    pub(crate) fn vertices(
        &self,
        index: usize,
        system: &ParticleSystem,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        lighting: &[[f32; 4]; 10],
        time: f32,
        onset: f32,
        stops: f32,
    ) -> Vec<ParticleCorner> {
        let mut vertices = Vec::new();
        if system.hidden {
            return vertices;
        }
        if system.child {
            for run in self.instances.get(index).into_iter().flatten() {
                let at = (run.origin, run.axes, run.start);
                system.vertices_at(
                    &run.running,
                    at,
                    values,
                    view,
                    lighting,
                    time,
                    stops,
                    &mut vertices,
                );
            }
            return vertices;
        }
        let Some(running) = self.running.get(index) else {
            return vertices;
        };
        let (origin, axes) = location(system, bones);
        system.vertices_at(
            running,
            (origin, axes, onset),
            values,
            view,
            lighting,
            time,
            stops,
            &mut vertices,
        );
        vertices
    }

    /// Where the particles of `system` are, for a contrail that follows them.
    pub(crate) fn tracks_of<'a>(
        &'a self,
        systems: &[Arc<ParticleSystem>],
        strip: &Arc<Gen3StripAt>,
    ) -> Vec<&'a ParticleTrack> {
        let carriers = systems.iter().zip(&self.tracks);
        let carrying = carriers
            .filter(|(system, _)| system.carried.iter().any(|known| Arc::ptr_eq(known, strip)));
        carrying.flat_map(|(_, tracks)| tracks).collect()
    }

    /// Where a system's sort goes, for one whose place a particle chose.
    pub(crate) fn distance(
        &self,
        index: usize,
        system: &ParticleSystem,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
    ) -> f32 {
        let run = self.instances.get(index).and_then(|runs| runs.first());
        match run.filter(|_| system.child) {
            Some(run) => dot(view.forward, minus(run.origin, view.eye)) - system.sort_bias * 0.05,
            None => system.distance(bones, view),
        }
    }
}

/// Where a system stands: its marker, on its node's pose.
pub(super) fn location(
    system: &ParticleSystem,
    bones: Option<&[[f32; 4]]>,
) -> ([f32; 3], [[f32; 3]; 3]) {
    let Some(rows) = bones.and_then(|bones| bones.get(system.node * 3..system.node * 3 + 3)) else {
        return (system.position, system.axes);
    };
    let [x, y, z] = system.position;
    let place = |row: &[f32; 4]| row[0] * x + row[1] * y + row[2] * z + row[3];
    let turn = |[i, j, k]: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|row| rows[row][0] * i + rows[row][1] * j + rows[row][2] * k)
    };
    let axes = if system.world_oriented {
        system.axes
    } else {
        system.axes.map(turn)
    };
    ([place(&rows[0]), place(&rows[1]), place(&rows[2])], axes)
}

/// `c_particle_emitter::calc_matrix`: its direction turns the location, and its
/// offset moves along that.
pub(super) fn emitter_frame(
    emitter: &Emitter,
    origin: [f32; 3],
    axes: [[f32; 3]; 3],
    state: &ParticleState,
) -> ([f32; 3], [[f32; 3]; 3]) {
    let direction = clamped_unit(emitter.direction.around(state));
    let yaw = direction[1].atan2(direction[0]);
    let pitch = direction[2].asin();
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    let rotation = [
        [cos_pitch * cos_yaw, sin_yaw, sin_pitch * cos_yaw],
        [-cos_pitch * sin_yaw, cos_yaw, -sin_pitch * sin_yaw],
        [-sin_pitch, 0.0, cos_pitch],
    ];
    let turned: [[f32; 3]; 3] = rotation
        .map(|row| std::array::from_fn(|axis| (0..3).map(|at| axes[at][axis] * row[at]).sum()));
    let offset = emitter.offset.vector(state);
    let moved: [f32; 3] = std::array::from_fn(|axis| {
        origin[axis] + (0..3).map(|at| turned[at][axis] * offset[at]).sum::<f32>()
    });
    (moved, turned)
}
