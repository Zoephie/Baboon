//! Running particle systems: their types' states, emission and particles, a tick at a time.

use super::super::super::{frame_steps, resume_point};
use super::draw::{CePctlQuad, system_quads};
use super::read::{
    ANIMATION, COLOR, CeParticleSystemAt, CePctlDef, CreationPhysics, MINIMUM, ParticleType,
    ParticleVariables, RADIUS, RATE, ROTATION, SCALE, TypeVariables,
};
use super::{
    CeRandom, CeSecondaryMap, CeSpriteSheet, FramebufferBlend, PointPhysics, ViewFrame, cross,
    posed_on,
};
use std::collections::VecDeque;
use std::sync::Arc;

/// How long a system runs before the preview first shows it.
const PRE_ROLL: f32 = 4.0;

/// `particle_system_new_particles` makes at most this many a call.
const MOST_A_CALL: usize = 128;

/// `particle_systems_initialize`'s `particle system particles` array, which
/// every system shares, attached or made by an effect.
pub(crate) const MOST_SYSTEM_PARTICLES: usize = 512;

/// `object_get_marker_by_name` finds at most this many.
pub(super) const MOST_MARKERS: usize = 8;

#[derive(Debug, Clone)]
pub(super) struct TypeState {
    pub(super) duration: [f32; 2],
    pub(super) transition: [f32; 2],
    pub(super) variables: TypeVariables,
    pub(super) creation_physics: CreationPhysics,
}

#[derive(Debug, Clone)]
pub(super) struct CeParticleState {
    pub(super) duration: [f32; 2],
    pub(super) transition: [f32; 2],
    pub(super) sheet: Arc<CeSpriteSheet>,
    /// Which of the system's sheets it is, for the textures.
    pub(super) sheet_index: usize,
    pub(super) sequence: i32,
    pub(super) scale: [f32; 2],
    pub(super) animation_rate: [f32; 2],
    pub(super) rotation_rate: [f32; 2],
    /// Alpha, red, green, blue, lower and upper.
    pub(super) colors: [[f32; 4]; 2],
    pub(super) radius: f32,
    pub(super) physics: Option<PointPhysics>,
    pub(super) blend: FramebufferBlend,
    pub(super) map_flags: i128,
    pub(super) nonlinear_tint: bool,
    /// `shader flags`' `sort bias`: sorted a quarter unit nearer.
    pub(super) sort_bias: bool,
    /// The shader's second `bitmap`.
    pub(super) secondary: Option<Arc<CeSecondaryMap>>,
}

/// The most states one step may pass through. The game has no such bound
/// (`particle_system_update`), but a state whose durations are all zero or
/// less would never let it out; tool rejects such tags, the editor does not.
const MOST_STATE_CHANGES: u32 = 256;

/// Counts a state change; past [`MOST_STATE_CHANGES`] the step stops where it
/// is, as caught up.
fn caught_up(changes: &mut u32, time_left: &mut f32) -> bool {
    *changes += 1;
    if *changes > MOST_STATE_CHANGES {
        *time_left = 0.0;
        return true;
    }
    false
}

/// `ps_particle_datum`.
#[derive(Debug, Clone)]
pub(super) struct PsParticle {
    forward: bool,
    pub(super) valid: bool,
    pub(super) state: Option<usize>,
    pub(super) transition: Option<usize>,
    pub(super) time_left: f32,
    pub(super) length: f32,
    pub(super) position: [f32; 3],
    velocity: [f32; 3],
    pub(super) axis: [f32; 3],
    pub(super) rotation: f32,
    /// -1 until it is first drawn, which picks one at random.
    pub(super) sprite: f32,
    pub(super) variables: ParticleVariables,
    pub(super) transition_variables: ParticleVariables,
}

/// `particle_type`.
#[derive(Debug, Clone, Default)]
pub(super) struct RunningType {
    pub(super) state: Option<usize>,
    transition: Option<usize>,
    time_left: f32,
    length: f32,
    pub(super) variables: TypeVariables,
    fractional: f32,
    forward: bool,
    /// The first made is last: each new one goes in front.
    pub(super) particles: VecDeque<PsParticle>,
}

/// `particle_system_datum`.
#[derive(Debug, Clone)]
pub(crate) struct CeRunningSystem {
    on: bool,
    scale: f32,
    /// Alpha, red, green, blue.
    color: [f32; 4],
    /// What `light_particle` found under it; alpha-blended sprites take it.
    pub(super) lighting: [f32; 3],
    /// On an object, which its type states may loop on; else on its own.
    attached: bool,
    position: [f32; 3],
    velocity: [f32; 3],
    pub(super) types: Vec<RunningType>,
    first: bool,
}

impl CeRunningSystem {
    /// How many particles it holds.
    pub(crate) fn particle_count(&self) -> usize {
        self.types.iter().map(|run| run.particles.len()).sum()
    }
}

/// Every particle system of the object, carried from frame to frame.
#[derive(Debug, Default)]
pub(crate) struct CeParticleSystemField {
    time: Option<f32>,
    systems: Vec<CeRunningSystem>,
    random: Option<CeRandom>,
}

/// `randomize_particle_variables`: scale, rates and alpha at random, the
/// color's channels together.
fn randomize(state: &CeParticleState, random: &mut CeRandom) -> ParticleVariables {
    let together = random.range(0.0, 1.0);
    let animation = random.range(state.animation_rate[0], state.animation_rate[1]);
    let rotation = random.range(state.rotation_rate[0], state.rotation_rate[1]);
    let scale = random.range(state.scale[0], state.scale[1]);
    let [low, high] = state.colors;
    let alpha = random.range(low[0], high[0]);
    [
        scale,
        animation,
        rotation,
        alpha,
        (high[1] - low[1]) * together + low[1],
        (high[2] - low[2]) * together + low[2],
        (high[3] - low[3]) * together + low[3],
    ]
}

/// `particle_system_next_type_state_index` and
/// `particle_system_next_particle_state_index`: the next state, round again
/// or back again if the flags say, else none.
pub(super) fn next_state(
    state: usize,
    forward: &mut bool,
    count: usize,
    looping: bool,
    bounce: bool,
) -> Option<usize> {
    let step: isize = if *forward { 1 } else { -1 };
    let next = state as isize + step;
    if next >= 0 && (next as usize) < count {
        return Some(next as usize);
    }
    if !looping || count == 0 {
        return None;
    }
    if bounce {
        *forward = !*forward;
        let back = state as isize - step;
        return Some(if back >= 0 {
            (back as usize).min(count - 1)
        } else {
            0
        });
    }
    Some(0)
}

impl CeParticleSystemField {
    /// How many particles its systems hold.
    pub(crate) fn particle_count(&self) -> usize {
        self.systems
            .iter()
            .map(CeRunningSystem::particle_count)
            .sum()
    }

    /// Step every system to `time`, a frame at a time as `game_frame` runs
    /// `particle_systems_update`; `others` particles of the shared array are
    /// in effects' systems.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn advance(
        &mut self,
        systems: &[CeParticleSystemAt],
        inputs: impl Fn(u8) -> ([f32; 4], [bool; 4], [[f32; 3]; 4]),
        bones: Option<&[[f32; 4]]>,
        ground: f32,
        time: f32,
        others: usize,
    ) {
        if self.systems.len() != systems.len() || self.time.is_none_or(|last| time < last) {
            let mut random = CeRandom(0x5eed_1234);
            self.systems = systems
                .iter()
                .map(|system| new_system(&system.def, true, &mut random))
                .collect();
            self.random = Some(random);
            self.time = Some(time - PRE_ROLL);
        }
        let Some(now) = self.time else {
            return;
        };
        let now = resume_point(now, time, PRE_ROLL);
        let mut random = self.random.take().unwrap_or(CeRandom(1));
        for step in frame_steps(now, time) {
            let mut room = MOST_SYSTEM_PARTICLES.saturating_sub(self.particle_count() + others);
            for (system, running) in systems.iter().zip(&mut self.systems) {
                let (outputs, active, change_colors) = inputs(system.object);
                let dt = if running.first { 0.001 } else { step };
                let inputs = (&outputs, &active, &change_colors);
                update_system(
                    system,
                    running,
                    inputs,
                    bones,
                    ground,
                    dt,
                    &mut random,
                    &mut room,
                );
                running.first = false;
            }
        }
        self.time = Some(time);
        self.random = Some(random);
    }

    /// `particle_system_render`: each particle a sprite, or two while it
    /// passes between states that draw differently. `pixel` is the world
    /// size of a pixel at a place, for a scale of nothing. Gives the system,
    /// the sheet, the image, the blend, the nonlinear tint, the color and
    /// the corners.
    pub(crate) fn quads(
        &mut self,
        systems: &[CeParticleSystemAt],
        view: &ViewFrame,
        pixel: impl Fn([f32; 3]) -> f32,
    ) -> Vec<CePctlQuad> {
        let mut quads = Vec::new();
        let mut random = self.random.take().unwrap_or(CeRandom(1));
        let time = self.time.unwrap_or(0.0);
        for (index, (system, running)) in systems.iter().zip(&mut self.systems).enumerate() {
            system_quads(
                &system.def,
                running,
                index,
                view,
                &pixel,
                &mut random,
                time,
                &mut quads,
            );
        }
        self.random = Some(random);
        quads
    }
}

/// `particle_system_new_attached` and `particle_system_initialize`: every
/// type in its first state for a time from its bounds.
fn new_system(def: &CePctlDef, attached: bool, random: &mut CeRandom) -> CeRunningSystem {
    let types = def
        .types
        .iter()
        .map(|kind| {
            let first = &kind.states[0];
            let length = random.range(first.duration[0], first.duration[1]);
            RunningType {
                state: Some(0),
                time_left: length,
                length,
                forward: true,
                ..RunningType::default()
            }
        })
        .collect();
    CeRunningSystem {
        on: true,
        scale: 1.0,
        color: [1.0; 4],
        lighting: [1.0; 3],
        attached,
        position: [0.0; 3],
        velocity: [0.0; 3],
        types,
        first: true,
    }
}

/// `particle_system_new_unattached`, as an effect's part makes one: at a
/// point, going, tinted and scaled as the effect says, lit by what
/// `light_particle` finds below it, which in a preview is nothing: grey.
/// `particle_system_initialize` steps it a thousandth of a second at once.
#[allow(clippy::too_many_arguments)]
pub(crate) fn new_unattached(
    def: &CePctlDef,
    position: [f32; 3],
    velocity: [f32; 3],
    color: [f32; 4],
    scale: f32,
    ground: f32,
    random: &mut CeRandom,
    room: &mut usize,
) -> CeRunningSystem {
    let mut running = new_system(def, false, random);
    running.position = position;
    running.velocity = velocity;
    running.color = color;
    running.scale = scale;
    // `light_particle` with no structure under it: `global_real_rgb_grey`.
    running.lighting = [0.5; 3];
    update_unattached(def, &mut running, ground, 0.001, random, room);
    running.first = false;
    running
}

/// `particle_system_update` for a system on no object: its own point physics
/// moves it, and it makes particles where it is. False once no type is in
/// a state, when `particle_system_delete` takes it. It makes no more than
/// `room` particles, which it takes from.
pub(crate) fn update_unattached(
    def: &CePctlDef,
    running: &mut CeRunningSystem,
    ground: f32,
    dt: f32,
    random: &mut CeRandom,
    room: &mut usize,
) -> bool {
    // `particle_system_update_default`, which explosion uses too.
    if let Some(physics) = def.physics {
        physics.step(
            &mut running.position,
            &mut running.velocity,
            1.0,
            ground,
            dt,
        );
    }
    let markers = [(running.position, [0.0; 3])];
    step_types(def, running, &markers, ground, dt, random, room)
}

/// What an object's functions give its systems: its outputs, which are on,
/// and its change colors.
type SystemInputs<'a> = (&'a [f32; 4], &'a [bool; 4], &'a [[f32; 3]; 4]);

#[allow(clippy::too_many_arguments)]
fn update_system(
    system: &CeParticleSystemAt,
    running: &mut CeRunningSystem,
    (outputs, active, change_colors): SystemInputs<'_>,
    bones: Option<&[[f32; 4]]>,
    ground: f32,
    dt: f32,
    random: &mut CeRandom,
    room: &mut usize,
) {
    // `object_get_function_value`: none is on at one.
    (running.scale, running.on) = match system.primary {
        Some(slot) => (outputs[slot], active[slot]),
        None => (1.0, true),
    };
    running.color = match system.change_color {
        Some(slot) => [
            1.0,
            change_colors[slot][0],
            change_colors[slot][1],
            change_colors[slot][2],
        ],
        None => [1.0; 4],
    };
    let markers: Vec<([f32; 3], [f32; 3])> = system
        .markers
        .iter()
        .map(|&(node, position, forward)| posed_on(node, position, forward, bones))
        .collect();
    step_types(&system.def, running, &markers, ground, dt, random, room);
}

/// The types of a system through their states, making and moving their
/// particles: true while any type is in a state.
fn step_types(
    def: &CePctlDef,
    running: &mut CeRunningSystem,
    markers: &[([f32; 3], [f32; 3])],
    ground: f32,
    dt: f32,
    random: &mut CeRandom,
    room: &mut usize,
) -> bool {
    let mut active = false;
    let (velocity, attached) = (running.velocity, running.attached);
    for (kind, run) in def.types.iter().zip(&mut running.types) {
        if kind.flags.disabled {
            continue;
        }
        run.time_left -= dt;
        let mut changes = 0;
        while let Some(state) = run.state {
            if run.time_left >= 0.0 {
                break;
            }
            if caught_up(&mut changes, &mut run.time_left) {
                break;
            }
            let bounds = match run.transition {
                None => {
                    run.transition = next_state(
                        state,
                        &mut run.forward,
                        kind.states.len(),
                        // A system on no object never goes round again.
                        kind.flags.type_states_loop && attached,
                        kind.flags.type_states_bounce,
                    );
                    if run.transition.is_none() {
                        run.state = None;
                    }
                    kind.states[state].transition
                }
                Some(next) => {
                    run.state = Some(next);
                    run.transition = None;
                    kind.states[next].duration
                }
            };
            let length = random.range(bounds[0], bounds[1]);
            run.time_left += length;
            run.length = length;
        }
        let Some(state) = run.state else {
            continue;
        };
        run.variables = match run.transition {
            None => kind.states[state].variables,
            Some(next) => {
                let t = if run.length != 0.0 {
                    (run.time_left / run.length).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                std::array::from_fn(|at| {
                    kind.states[state].variables[at] * t
                        + kind.states[next].variables[at] * (1.0 - t)
                })
            }
        };
        if kind.flags.tint_by_effect_color {
            for channel in 0..4 {
                run.variables[COLOR + channel] *= running.color[channel];
            }
        }
        let scale = running.scale;
        let flags = &kind.flags;
        for (scales, at) in [
            (flags.minimum_count_scales, MINIMUM),
            (flags.creation_rate_scales, RATE),
            (flags.scale_scales, SCALE),
            (flags.animation_rate_scales, ANIMATION),
            (flags.rotation_rate_scales, ROTATION),
        ] {
            if scales {
                run.variables[at] *= scale;
            }
        }
        if running.on {
            new_particles(
                kind,
                run,
                running.first,
                scale,
                markers,
                velocity,
                dt,
                random,
                room,
            );
        }
        update_particles(kind, run, ground, dt, random);
        active = true;
    }
    active
}

/// `particle_system_new_particles`: the type's initial count the first time,
/// else what its creation rate makes of the time, at random among the
/// attachment's markers.
#[allow(clippy::too_many_arguments)]
fn new_particles(
    kind: &ParticleType,
    run: &mut RunningType,
    first: bool,
    scale: f32,
    markers: &[([f32; 3], [f32; 3])],
    system_velocity: [f32; 3],
    dt: f32,
    random: &mut CeRandom,
    room: &mut usize,
) {
    let Some(state) = run.state else {
        return;
    };
    let wanted = if first {
        if kind.flags.initial_count_scales {
            (kind.initial_count as f32 * scale + 0.5) as i32
        } else {
            kind.initial_count
        }
    } else {
        let made = run.variables[RATE] * dt;
        let whole = made as i32;
        let mut wanted = run.particles.len() as i32 + whole;
        run.fractional += made - whole as f32;
        if run.fractional > 1.0 {
            run.fractional -= 1.0;
            wanted += 1;
        }
        wanted
    };
    let physics = if first {
        kind.initial_physics
    } else {
        kind.states[state].creation_physics
    };
    let mut made = 0;
    while (run.particles.len() as i32) < wanted
        && !markers.is_empty()
        && made < MOST_A_CALL
        && *room > 0
    {
        made += 1;
        *room -= 1;
        let rotation = random.range(0.0, std::f32::consts::TAU);
        let (position, forward) = markers[random.index(markers.len())];
        let (position, mut velocity, axis) = match physics {
            CreationPhysics::Explosion => explosion(kind, position, random),
            CreationPhysics::Jet => jet(kind, position, forward, random),
            CreationPhysics::Default => (position, [0.0; 3], [0.0; 3]),
        };
        // Each creation adds the system's own velocity.
        velocity = std::array::from_fn(|axis| velocity[axis] + system_velocity[axis]);
        run.particles.push_front(PsParticle {
            forward: true,
            valid: true,
            state: None,
            transition: None,
            time_left: 0.0,
            length: 0.0,
            position,
            velocity,
            axis,
            rotation,
            sprite: -1.0,
            variables: [0.0; 7],
            transition_variables: [0.0; 7],
        });
    }
    // Short of the minimum, the state hurries on.
    if (run.particles.len() as f32) < run.variables[MINIMUM] {
        run.time_left *= 0.3;
    }
}

/// `particle_system_new_particle_explosion`: out from the marker at random,
/// spread across by the first constant and up by the second, going at the
/// third times that; the axis lies flat, square to the way out.
fn explosion(
    kind: &ParticleType,
    marker: [f32; 3],
    random: &mut CeRandom,
) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let [across, up, speed] = kind.constants;
    let direction = random.direction();
    let out = [
        direction[0] * across,
        direction[1] * across,
        direction[2] * up,
    ];
    let position = std::array::from_fn(|axis| marker[axis] + out[axis]);
    let velocity = out.map(|axis| axis * speed);
    // `rotate_vector_about_axis` by a quarter turn about up.
    let axis = [-out[1], out[0], 0.0];
    (position, velocity, axis)
}

/// `particle_system_new_particle_jet`: along the marker's forward at the
/// first constant per tick, a share (the second) of it at random; the axis
/// is square to the way it goes and the forward, or to up when the third
/// constant is set.
fn jet(
    kind: &ParticleType,
    marker: [f32; 3],
    forward: [f32; 3],
    random: &mut CeRandom,
) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let [speed, spread, upright] = kind.constants;
    let per_tick = speed * 0.033_333_335;
    let scattered = spread * per_tick;
    let straight = (1.0 - spread) * per_tick;
    let direction = random.direction();
    let velocity: [f32; 3] =
        std::array::from_fn(|axis| direction[axis] * scattered + forward[axis] * straight);
    let axis = if upright == 0.0 {
        cross(forward, velocity)
    } else {
        cross(velocity, [0.0, 0.0, 1.0])
    };
    (marker, velocity, axis)
}

/// The particles of one type through their states, turned, animated and
/// moved; those out of states, or killed where the flags say, go.
fn update_particles(
    kind: &ParticleType,
    run: &mut RunningType,
    ground: f32,
    dt: f32,
    random: &mut CeRandom,
) {
    let multipliers = run.variables;
    let states = &kind.particle_states;
    run.particles.retain_mut(|particle| {
        particle.time_left -= dt;
        if particle.state.is_none() && !states.is_empty() {
            particle.state = Some(0);
            let length = random.range(states[0].duration[0], states[0].duration[1]);
            particle.time_left = length;
            particle.length = length;
            particle.variables = randomize(&states[0], random);
        }
        if !particle.valid {
            particle.state = None;
        }
        let mut changes = 0;
        while let Some(state) = particle.state {
            if particle.time_left >= 0.0 {
                break;
            }
            if caught_up(&mut changes, &mut particle.time_left) {
                break;
            }
            let bounds = match particle.transition {
                None => {
                    particle.transition = next_state(
                        state,
                        &mut particle.forward,
                        states.len(),
                        kind.flags.particle_states_loop,
                        kind.flags.particle_states_bounce,
                    );
                    if particle.transition.is_none() {
                        particle.state = None;
                    }
                    states[state].transition
                }
                Some(next) => {
                    particle.state = Some(next);
                    particle.transition = None;
                    states[next].duration
                }
            };
            let length = random.range(bounds[0], bounds[1]);
            particle.time_left += length;
            particle.length = length;
            match particle.transition {
                None => particle.variables = particle.transition_variables,
                Some(next) => particle.transition_variables = randomize(&states[next], random),
            }
        }
        let Some(state) = particle.state else {
            return false;
        };
        let t = if particle.length != 0.0 {
            (particle.time_left / particle.length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let rate = |at: usize| match particle.transition {
            None => particle.variables[at],
            Some(_) => particle.transition_variables[at] * (1.0 - t) + particle.variables[at] * t,
        };
        particle.rotation += rate(ROTATION) * multipliers[ROTATION] * dt;
        particle.sprite += rate(ANIMATION) * multipliers[ANIMATION] * dt;
        // `particle_system_update_particle_default`.
        let current = &states[state];
        let (physics, radius) = match particle.transition.map(|next| &states[next]) {
            None => (
                current.physics,
                current.radius * multipliers[RADIUS] * kind.radius,
            ),
            Some(next) => {
                let physics = match (current.physics, next.physics) {
                    (Some(a), Some(b)) => Some(a.lerp(&b, t)),
                    (a, b) => a.or(b),
                };
                (
                    physics,
                    (current.radius * t + next.radius * (1.0 - t))
                        * multipliers[RADIUS]
                        * kind.radius,
                )
            }
        };
        if let Some(physics) = physics {
            // Always in air: the preview has no water. Touching the floor is
            // touching structure.
            let grounded = physics.step(
                &mut particle.position,
                &mut particle.velocity,
                radius,
                ground,
                dt,
            );
            if kind.flags.die_in_air || (grounded && kind.flags.die_on_ground) {
                particle.valid = false;
            }
        }
        true
    });
}
