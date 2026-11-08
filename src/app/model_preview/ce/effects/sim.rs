//! The running effects and their particle field: spawning parts and systems, generating particles, a tick at a time.

use super::*;

/// `effect_update` runs at most this many events in a tick.
pub(super) const MOST_EVENTS_A_TICK: usize = 8;

/// The game's pool is shared by the whole level.
const MOST_PARTICLES: usize = 1024;

/// `seed_random` and the draws made from it.
#[derive(Debug, Clone, Copy, Default)]
pub(in crate::app::model_preview) struct Random(u32);

impl Random {
    pub(super) fn next(&mut self) -> f32 {
        self.0 = lcg_step(self.0);
        high24_over_2_24(self.0)
    }

    pub(super) fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }

    /// `seed_random`: the high half of the next seed.
    pub(in crate::app::model_preview) fn bits(&mut self) -> u32 {
        self.0 = lcg_step(self.0);
        high_bits(self.0)
    }

    fn direction(&mut self) -> [f32; 3] {
        let z = self.range(-1.0, 1.0);
        let (sin, cos) = self.range(0.0, std::f32::consts::TAU).sin_cos();
        let flat = (1.0 - z * z).max(0.0).sqrt();
        [flat * cos, flat * sin, z]
    }
}

/// `effect_datum`, the parts the preview keeps.
#[derive(Debug, Clone)]
pub(super) struct RunningEffect {
    pub(super) event: usize,
    pub(super) in_event: bool,
    pub(super) time: f32,
    pub(super) duration: f32,
    pub(super) last_fraction: f32,
    pub(super) counts: Vec<f32>,
    pub(super) finished: bool,
    pub(super) stopping: bool,
    /// Deleted by `effect_update`: finished and never set going again.
    pub(super) deleted: bool,
    pub(super) scale_a: f32,
    pub(super) scale_b: f32,
    pub(super) color: [f32; 3],
}

/// `particle_datum`, the parts the preview keeps.
#[derive(Debug, Clone)]
pub(super) struct CeParticle {
    effect: usize,
    def: usize,
    /// The node it rides, when `stay attached to marker`; place and
    /// direction are then the node's.
    pub(super) node: Option<usize>,
    pub(super) place: [f32; 3],
    pub(super) direction: [f32; 3],
    pub(super) velocity: [f32; 3],
    pub(super) age: f32,
    pub(super) lifespan: f32,
    pub(super) frame_time: f32,
    pub(super) frame_span: f32,
    pub(super) sequence: i32,
    pub(super) frame: i32,
    pub(super) phase: u8,
    pub(super) rotation: f32,
    pub(super) angular_velocity: f32,
    pub(super) radius: f32,
    /// Alpha, then red, green and blue.
    color: [f32; 4],
    pub(super) flags: u8,
    /// Render frames since `render_particles` last drew it.
    undrawn: u32,
}

/// `particles_update` (360 and MCC) deletes a particle not drawn for more
/// than this many render frames; `render_particles` leaves one at or below
/// its `fade end size` on screen undrawn, so small and distant ones die.
const MOST_UNDRAWN_FRAMES: u32 = 15;

/// A light an effect's part made: `light_new_unattached` on its location.
#[derive(Debug, Clone)]
pub(super) struct CeRunningLight {
    effect: usize,
    def: usize,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    /// The tick it was made.
    born: u32,
    scale: f32,
}

/// Every effect's particles, carried from frame to frame.
#[derive(Debug, Default)]
pub(crate) struct CeParticleField {
    time: Option<f32>,
    effects: Vec<RunningEffect>,
    particles: Vec<CeParticle>,
    lights: Vec<CeRunningLight>,
    /// Particle systems parts have made: the effect, which of its systems,
    /// and the system as it runs.
    systems: Vec<(usize, usize, CeRunningSystem)>,
    /// Effects particles and the panel's plays have made, after the
    /// object's in every index.
    spawned: Vec<CeEffectAt>,
    /// How many plays of each event effect have been made.
    played: Vec<u32>,
    /// `game_time_get`.
    pub(super) tick: u32,
    owed: CeTicks,
    random: Option<Random>,
    system_random: Option<SystemRandom>,
}

/// What an attached effect reads of its object.
pub(crate) struct CeObjectInputs {
    pub(crate) outputs: [f32; 4],
    /// Which outputs are on.
    pub(crate) active: [bool; 4],
    pub(crate) change_colors: [[f32; 3]; 4],
}

/// A range's lower bound and delta, each times the scales its `A` and `B`
/// pairs name.
pub(super) fn scaled(value: [f32; 2], effect: &RunningEffect, a: [bool; 2], b: [bool; 2]) -> [f32; 2] {
    let mut lower = value[0];
    let mut delta = value[1] - value[0];
    if a[0] {
        lower *= effect.scale_a;
    }
    if b[0] {
        lower *= effect.scale_b;
    }
    if a[1] {
        delta *= effect.scale_a;
    }
    if b[1] {
        delta *= effect.scale_b;
    }
    [lower, delta]
}

pub(super) fn rotate_about(vector: [f32; 3], axis: [f32; 3], sine: f32, cosine: f32) -> [f32; 3] {
    let along = dot(axis, vector);
    let across = cross(axis, vector);
    std::array::from_fn(|at| vector[at] * cosine + across[at] * sine + axis[at] * along * (1.0 - cosine))
}

fn turn_by_location(location: &CeLocation, local: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|at| (0..3).map(|row| location.axes[row][at] * local[row]).sum::<f32>())
}

impl CeParticleField {
    /// How many particles the systems its effects made hold.
    pub(crate) fn system_particle_count(&self) -> usize {
        self.systems.iter().map(|(_, _, running)| running.particle_count()).sum()
    }

    /// Step every effect and particle to `time`, a frame at a time: each
    /// frame first runs the ticks it owes, `effects_update` being in
    /// `game_tick`, then moves particles and systems by its own time, as
    /// `game_frame` does. `others` particles of the systems' shared array are
    /// in the object's attached systems. Each new play in `plays` starts a
    /// copy of its event effect.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn advance(
        &mut self,
        effects: &[CeEffectAt],
        plays: &CeEventPlays,
        inputs: impl Fn(u8) -> CeObjectInputs,
        bones: Option<&[[f32; 4]]>,
        ground: f32,
        time: f32,
        others: usize,
    ) {
        let own = effects.len();
        if self.effects.len() != own + self.spawned.len() || self.time.is_none_or(|last| time < last) {
            self.effects = effects.iter().map(|_| new_running()).collect();
            self.particles.clear();
            self.lights.clear();
            self.systems.clear();
            self.spawned.clear();
            // Plays made before the restart are not made again.
            self.played = plays.counts.clone();
            self.system_random = Some(SystemRandom::seeded(0x5eed_4321));
            self.tick = 0;
            self.owed = CeTicks::default();
            self.random = Some(Random(0x1234_5678));
            let mut random = self.random.take().unwrap();
            for (running, effect) in self.effects.iter_mut().zip(effects) {
                set_event(running, &effect.def, 0, &mut random);
            }
            self.random = Some(random);
            self.time = Some(time - PRE_ROLL);
        }
        let Some(now) = self.time else {
            return;
        };
        let now = resume_point(now, time, PRE_ROLL);
        let mut random = self.random.take().unwrap_or(Random(1));
        self.play_events(effects, plays, &mut random);
        for dt in frame_steps(now, time) {
            let effects: Vec<CeEffectAt> = effects.iter().chain(&self.spawned).cloned().collect();
            let ticks = self.owed.run(dt);
            for _ in 0..ticks {
                self.tick_effects(&effects, &inputs, bones, ground, others, &mut random);
            }
            self.frame_parts(&effects, ground, dt, ticks as f32 * STEP, others, &mut random);
            self.forget_spawned(own);
        }
        self.time = Some(time);
        self.random = Some(random);
    }

    /// How many particles are alive.
    #[cfg(test)]
    pub(crate) fn particle_count(&self) -> usize {
        self.particles.len()
    }

    /// Whether anything the object's own `own` effects did not make is still
    /// running: what plays and particles made, and what those left.
    pub(crate) fn runs_played(&self, own: usize) -> bool {
        !self.spawned.is_empty()
            || self.particles.iter().any(|particle| particle.effect >= own)
            || self.lights.iter().any(|light| light.effect >= own)
            || self.systems.iter().any(|(effect, _, _)| *effect >= own)
    }

    /// `effect_new_from_object` for each play not yet made: a copy of the
    /// event's effect on the object's markers, at the plays' scales, white,
    /// at rest, among those particles have made.
    fn play_events(&mut self, effects: &[CeEffectAt], plays: &CeEventPlays, random: &mut Random) {
        self.played.resize(effects.len(), 0);
        for (index, effect) in effects.iter().enumerate() {
            let wanted = plays.counts.get(index).copied().unwrap_or(0);
            let made = std::mem::replace(&mut self.played[index], wanted);
            if effect.event.is_none() {
                continue;
            }
            for _ in made..wanted {
                if self.spawned.len() >= MOST_SPAWNED {
                    return;
                }
                let spawn = CeSpawn { scales: plays.scales, color: [1.0; 3], velocity: [0.0; 3] };
                let copy = CeEffectAt { spawn: Some(spawn), event: None, ..effect.clone() };
                let mut running = new_running();
                set_event(&mut running, &copy.def, 0, random);
                self.effects.push(running);
                self.spawned.push(copy);
            }
        }
    }

    /// One `game_tick` of `effects_update`: every effect's events, and the
    /// lights past their duration let go.
    fn tick_effects(
        &mut self,
        effects: &[CeEffectAt],
        inputs: &impl Fn(u8) -> CeObjectInputs,
        bones: Option<&[[f32; 4]]>,
        ground: f32,
        others: usize,
        random: &mut Random,
    ) {
        self.tick += 1;
        let mut system_room = MOST_SYSTEM_PARTICLES.saturating_sub(self.system_particle_count() + others);
        for (index, effect) in effects.iter().enumerate() {
            // An event's own effect only stands for its plays.
            if effect.event.is_some() {
                continue;
            }
            let state = inputs(effect.object);
            let running = &mut self.effects[index];
            let system_random = self.system_random.get_or_insert_with(|| SystemRandom::seeded(1));
            let made = Made {
                particles: &mut self.particles,
                lights: &mut self.lights,
                systems: &mut self.systems,
                system_random,
                system_room: &mut system_room,
                ground,
                tick: self.tick,
            };
            update_effect(index, running, effect, &state, made, random, bones, STEP);
        }
        // `lights_preprocess_scene` lets go of those past their duration.
        let tick = self.tick;
        self.lights.retain(|light| {
            let def = effects.get(light.effect).and_then(|effect| effect.def.lights.get(light.def));
            def.is_some_and(|def| def.alive(tick - light.born))
        });
    }

    /// One `game_frame`, `dt` long, after `ticked` of ticks:
    /// `particle_systems_update` by the frame, then, only when ticks ran,
    /// `particles_update` by them, with the effects what they hit and died
    /// of make. A particle that steps a frame per update steps one per tick.
    fn frame_parts(&mut self, effects: &[CeEffectAt], ground: f32, dt: f32, ticked: f32, others: usize, random: &mut Random) {
        let mut system_room = MOST_SYSTEM_PARTICLES.saturating_sub(self.system_particle_count() + others);
        // Those with no type in a state go.
        let system_random = self.system_random.get_or_insert_with(|| SystemRandom::seeded(1));
        self.systems.retain_mut(|(effect, def, running)| {
            effects
                .get(*effect)
                .and_then(|effect| effect.def.systems.get(*def))
                .is_some_and(|def| update_unattached(def, running, ground, dt, system_random, &mut system_room))
        });
        // `game_frame` runs `particles_update` only when ticks have.
        if ticked <= 0.0 {
            return;
        }
        let mut made: Vec<CeEffectAt> = Vec::new();
        self.particles.retain_mut(|particle| {
            if particle.undrawn > MOST_UNDRAWN_FRAMES {
                return false;
            }
            let def = &effects[particle.effect].def.particles[particle.def];
            let (fate, collided) = update_particle(particle, def, random, ground, ticked);
            // `particle_effect_new` for what it hit, then what it died of.
            if let (Some(scale), Some(effect)) = (collided, &def.collision_effect) {
                made.extend(spawn_effect(effect, particle, scale));
            }
            if let (Fate::Died, Some(effect)) = (fate, &def.death_effect) {
                made.extend(spawn_effect(effect, particle, 0.0));
            }
            fate == Fate::Alive
        });
        for effect in made {
            if self.spawned.len() >= MOST_SPAWNED {
                break;
            }
            let mut running = new_running();
            set_event(&mut running, &effect.def, 0, random);
            self.effects.push(running);
            self.spawned.push(effect);
        }
    }

    /// Lets go of the effects particles made that have run their events and
    /// left nothing behind, renumbering what came after them. `first` is
    /// how many are the object's own.
    fn forget_spawned(&mut self, first: usize) {
        let total = self.effects.len();
        let mut used = vec![false; total];
        for particle in &self.particles {
            used[particle.effect] = true;
        }
        for light in &self.lights {
            used[light.effect] = true;
        }
        for (effect, _, _) in &self.systems {
            used[*effect] = true;
        }
        let gone: Vec<bool> = (0..total).map(|at| at >= first && self.effects[at].finished && !used[at]).collect();
        if !gone.contains(&true) {
            return;
        }
        let mut renumbered = Vec::with_capacity(total);
        let mut next = 0;
        for &dropped in &gone {
            renumbered.push(next);
            if !dropped {
                next += 1;
            }
        }
        let effects = std::mem::take(&mut self.effects);
        self.effects = effects.into_iter().zip(&gone).filter(|(_, dropped)| !**dropped).map(|(effect, _)| effect).collect();
        let spawned = std::mem::take(&mut self.spawned);
        self.spawned =
            spawned.into_iter().zip(&gone[first..]).filter(|(_, dropped)| !**dropped).map(|(effect, _)| effect).collect();
        for particle in &mut self.particles {
            particle.effect = renumbered[particle.effect];
        }
        for light in &mut self.lights {
            light.effect = renumbered[light.effect];
        }
        for (effect, _, _) in &mut self.systems {
            *effect = renumbered[*effect];
        }
    }

    /// The sprites of the particle systems effects' parts have made, with
    /// the effect and system each came from.
    pub(crate) fn system_quads(
        &mut self,
        effects: &[CeEffectAt],
        view: &ViewFrame,
        pixel: impl Fn([f32; 3]) -> f32,
    ) -> Vec<(usize, usize, CePctlQuad)> {
        let effects = self.all(effects);
        let mut quads = Vec::new();
        let time = self.tick as f32 * STEP;
        let random = self.system_random.get_or_insert_with(|| SystemRandom::seeded(1));
        for (effect, def, running) in &mut self.systems {
            let Some(at) = effects.get(*effect) else {
                continue;
            };
            let Some(system) = at.def.systems.get(*def) else {
                continue;
            };
            // Each system numbers its groups from nought.
            let base = quads.len();
            for mut quad in unattached_quads(system, running, 0, view, &pixel, random, time) {
                quad.group += base;
                quads.push((def_key(&at.def), *def, quad));
            }
        }
        quads
    }

    /// The object's effects, then those particles have made.
    fn all(&self, effects: &[CeEffectAt]) -> Vec<CeEffectAt> {
        effects.iter().chain(&self.spawned).cloned().collect()
    }

    /// The lights effects' parts have made, as `lights_preprocess_scene`
    /// has them now, with the effect and light each came from.
    pub(crate) fn lights(&self, effects: &[CeEffectAt]) -> Vec<(usize, usize, CeLight)> {
        let effects = self.all(effects);
        self.lights
            .iter()
            .filter_map(|light| {
                let effect = effects.get(light.effect)?;
                let def = effect.def.lights.get(light.def)?;
                let made = def.at(effect.object, light.node, light.position, light.forward, self.tick - light.born, light.scale)?;
                Some((def_key(&effect.def), light.def, made))
            })
            .collect()
    }

    /// `render_particles` and `build_sprite`: each particle's quad, with
    /// what `pixels` says a sphere of a radius at a point covers on screen,
    /// counting the frames each goes undrawn.
    pub(crate) fn quads(
        &mut self,
        effects: &[CeEffectAt],
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        pixels: impl Fn([f32; 3], f32) -> f32,
    ) -> Vec<CeParticleQuad> {
        let effects = self.all(effects);
        let mut quads = Vec::new();
        // `render_particles` gives a particle's shader the average radius of
        // its kind drawn with it: sum and count, by effect and kind.
        let mut radii: std::collections::HashMap<(usize, usize), (f32, f32)> = std::collections::HashMap::new();
        for particle in &mut self.particles {
            let Some(effect) = effects.get(particle.effect) else {
                continue;
            };
            let def = &effect.def.particles[particle.def];
            let (center, direction) = match particle.node {
                Some(node) => posed_on(node, particle.place, particle.direction, bones),
                None => (particle.place, particle.direction),
            };
            let radius = def.radius_scale(particle.age, particle.lifespan) * particle.radius;
            let diameter = pixels(center, radius);
            if diameter <= def.fade_end_size {
                particle.undrawn = particle.undrawn.saturating_add(1);
                continue;
            }
            particle.undrawn = 0;
            let entry = radii.entry((def_key(&effect.def), particle.def)).or_insert((0.0, 0.0));
            entry.0 += radius;
            entry.1 += 1.0;
            let mut scale = radius * 2.0 * def.sprite_scale;
            if diameter < def.minimum_size {
                scale *= def.minimum_size / diameter;
            }
            let mut fade = 1.0;
            if def.fade_in > 0.0 && particle.age < def.fade_in {
                fade = particle.age / def.fade_in;
            }
            if def.fade_out > 0.0 && particle.lifespan - particle.age < def.fade_out {
                fade *= (particle.lifespan - particle.age) / def.fade_out;
            }
            let Some(sprite) = usize::try_from(particle.sequence)
                .ok()
                .and_then(|at| def.sequences.get(at))
                .and_then(|frames| usize::try_from(particle.frame).ok().and_then(|at| frames.get(at)))
            else {
                continue;
            };
            let to_particle = unit_or_unchanged(std::array::from_fn(|axis| center[axis] - view.eye[axis]));
            let (across, up) = sprite_basis(def.orientation, direction, to_particle, view);
            // `build_sprite_compute_vertex_fade`, which only a turned sprite takes.
            if def.fade_mode != 0 && def.orientation != 0 {
                let normal = unit_or_unchanged(cross(across, up));
                let facing = dot(to_particle, normal).abs();
                fade *= if def.fade_mode == 2 { 1.0 - facing } else { facing };
            }
            let [alpha, red, green, blue] = particle.color;
            let shown_alpha =
                if def.blend != FramebufferBlend::AlphaBlend && !def.nonlinear_tint { fade } else { alpha * fade };
            let size = def.image_widths.get(sprite.image).copied().unwrap_or(1.0) * scale;
            let [left, right, top, bottom] = sprite.rect;
            let center_uv = [left + sprite.registration[0], top + sprite.registration[1]];
            let (sine, cosine) = particle.rotation.sin_cos();
            let corner = |u: f32, v: f32| {
                let x0 = u - center_uv[0];
                let y0 = center_uv[1] - v;
                let mut x = x0 * cosine - y0 * sine;
                let mut y = y0 * cosine + x0 * sine;
                if particle.flags & MIRROR_U != 0 {
                    x = -x;
                }
                if particle.flags & MIRROR_V != 0 {
                    y = -y;
                }
                let place: [f32; 3] = std::array::from_fn(|axis| center[axis] + (across[axis] * x + up[axis] * y) * size);
                [place[0], place[1], place[2], u, v]
            };
            quads.push(CeParticleQuad {
                effect: def_key(&effect.def),
                def: particle.def,
                blend: def.blend,
                nonlinear_tint: def.nonlinear_tint,
                sort_bias: def.sort_bias,
                secondary: None,
                image: sprite.image,
                corners: [corner(left, bottom), corner(right, bottom), corner(left, top), corner(right, top)],
                color: [red, green, blue, shown_alpha],
            });
        }
        // Then each kind's second map, with what its kind's sprites average.
        let time = self.tick as f32 * STEP;
        for quad in &mut quads {
            let Some(effect) = effects.iter().find(|effect| def_key(&effect.def) == quad.effect) else {
                continue;
            };
            let Some(map) = effect.def.particles.get(quad.def).and_then(|def| def.secondary.as_ref()) else {
                continue;
            };
            let (sum, count) = radii.get(&(quad.effect, quad.def)).copied().unwrap_or((0.0, 0.0));
            let average = if count > 0.0 { sum / count } else { 0.0 };
            quad.secondary = Some(map.draw(time, average));
        }
        quads
    }
}

/// `effect_new` leaves an effect before its first event.
fn new_running() -> RunningEffect {
    RunningEffect {
        event: 0,
        in_event: false,
        time: 0.0,
        duration: 0.0,
        last_fraction: -1.0,
        counts: Vec::new(),
        finished: false,
        stopping: false,
        deleted: false,
        scale_a: 1.0,
        scale_b: 1.0,
        color: [1.0; 3],
    }
}

/// `global_real_rgb_grey`, which `light_particle` gives where no structure is.
const PARTICLE_GREY: f32 = 0.5;

/// The most effects particles may have made at once. The game's effect
/// array is shared with every other effect; the preview keeps its own.
const MOST_SPAWNED: usize = 128;

/// Where none of an effect's own markers stand: [`posed_on`] leaves a place
/// on no node of the skeleton where it is.
const WORLD_NODE: usize = usize::MAX / 8;

/// How a particle's update ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fate {
    Alive,
    /// `datum_delete`: gone, and nothing more.
    Deleted,
    /// `particle_die`: gone, making its death effect.
    Died,
}

/// `particle_effect_new` for an effect: `effect_new_unattached_from_markers`
/// with two markers where the particle is, `velocity` along its way and
/// `gravity` straight down; a location naming neither takes the first.
fn spawn_effect(def: &Arc<CeEffectDef>, particle: &CeParticle, scale: f32) -> Option<CeEffectAt> {
    let along = unit_or_unchanged(particle.direction);
    let frame = |forward: [f32; 3]| {
        let up = unit_or_unchanged(perpendicular(forward));
        CeLocation { node: WORLD_NODE, position: particle.place, axes: [forward, cross(up, forward), up] }
    };
    let locations = def
        .locations
        .iter()
        .map(|name| vec![if name == "gravity" { frame([0.0, 0.0, -1.0]) } else { frame(along) }])
        .collect();
    Some(CeEffectAt {
        object: u8::MAX,
        def: def.clone(),
        spawn: Some(CeSpawn {
            scales: [scale, 0.0],
            color: [1.0; 3],
            velocity: particle.velocity.map(|axis| axis * 0.033_333_335),
        }),
        event: None,
        primary: None,
        secondary: None,
        change_color: None,
        locations,
    })
}

/// What identifies an effect's tag among those the preview has read.
pub(crate) fn def_key(def: &Arc<CeEffectDef>) -> usize {
    Arc::as_ptr(def) as usize
}

/// Every effect tag the object's effects can come to run: their own, and
/// those their particles make when they hit something or die.
pub(crate) fn reachable_defs(effects: &[CeEffectAt]) -> Vec<Arc<CeEffectDef>> {
    let mut found: Vec<Arc<CeEffectDef>> = Vec::new();
    let mut pending: Vec<Arc<CeEffectDef>> = effects.iter().map(|effect| effect.def.clone()).collect();
    while let Some(def) = pending.pop() {
        if found.iter().any(|known| Arc::ptr_eq(known, &def)) {
            continue;
        }
        for particle in &def.particles {
            pending.extend(particle.collision_effect.iter().chain(&particle.death_effect).cloned());
        }
        found.push(def);
    }
    found
}

/// What an effect's update makes: particles and lights, and the tick.
pub(super) struct Made<'a> {
    pub(super) particles: &'a mut Vec<CeParticle>,
    pub(super) lights: &'a mut Vec<CeRunningLight>,
    pub(super) systems: &'a mut Vec<(usize, usize, CeRunningSystem)>,
    pub(super) system_random: &'a mut SystemRandom,
    /// What is left of the particle systems' shared particles.
    pub(super) system_room: &'a mut usize,
    pub(super) ground: f32,
    pub(super) tick: u32,
}

/// Whether an event's particle or part is made for the preview's object:
/// violent (`effect_part_allowed_by_disposition` leaves out what is for
/// nonviolent mode only) and in air with no level about it
/// (`effect_allowed_by_environment`: water needs water under the point, and
/// space only is never made).
pub(super) fn created_here(entry: TagStruct<'_>) -> bool {
    let environment = entry.read_enum_option("create in");
    let violence = entry.read_enum_option("violence mode");
    matches!(environment.as_deref(), None | Some("any environment" | "air only"))
        && violence.as_deref() != Some("nonviolent mode only")
}

/// `effect_generate_parts`, for the parts that make lights: one at every
/// instance of the location, of the parts [`created_here`].
pub(super) fn generate_parts(index: usize, running: &RunningEffect, effect: &CeEffectAt, lights: &mut Vec<CeRunningLight>, tick: u32) {
    for part in &effect.def.events[running.event].lights {
        if !part.created {
            continue;
        }
        let Some(instances) = effect.locations.get(part.location) else {
            continue;
        };
        // The type-specific scale.
        let mut scale = 1.0;
        if part.scales_a.type_specific {
            scale = running.scale_a;
        }
        if part.scales_b.type_specific {
            scale *= running.scale_b;
        }
        for instance in instances {
            lights.push(CeRunningLight {
                effect: index,
                def: part.light,
                node: instance.node,
                position: instance.position,
                forward: instance.axes[0],
                born: tick,
                scale,
            });
        }
    }
}

/// `effect_real_random_range`, with a range's pair from each scale.
fn effect_random_range(value: [f32; 2], running: &RunningEffect, a: [bool; 2], b: [bool; 2], random: &mut Random) -> f32 {
    let [lower, delta] = scaled(value, running, a, b);
    random.range(0.0, delta) + lower
}

/// `effect_generate_parts` for the parts that make particle systems:
/// `particle_system_new_unattached` at every instance of the location,
/// going by `effect_random_translational_velocity` along the location's
/// forward (or straight down), tinted by the effect's color.
#[allow(clippy::too_many_arguments)]
pub(super) fn generate_systems(
    index: usize,
    running: &RunningEffect,
    effect: &CeEffectAt,
    systems: &mut Vec<(usize, usize, CeRunningSystem)>,
    random: &mut Random,
    system_random: &mut SystemRandom,
    room: &mut usize,
    bones: Option<&[[f32; 4]]>,
    ground: f32,
) {
    for part in &effect.def.events[running.event].systems {
        if !part.created {
            continue;
        }
        let (Some(instances), Some(def)) = (effect.locations.get(part.location), effect.def.systems.get(part.system)) else {
            continue;
        };
        let mut scale = 1.0;
        if part.scales_a.type_specific {
            scale = running.scale_a;
        }
        if part.scales_b.type_specific {
            scale *= running.scale_b;
        }
        for instance in instances {
            let (point, mut forward) = posed_on(instance.node, instance.position, instance.axes[0], bones);
            if part.face_down {
                forward = [0.0, 0.0, -1.0];
            }
            let speed = effect_random_range(part.velocity, running, part.scales_a.velocity, part.scales_b.velocity, random);
            let mut cone = part.cone;
            if part.scales_a.cone {
                cone *= running.scale_a;
            }
            if part.scales_b.cone {
                cone *= running.scale_b;
            }
            let angle = random.next() * cone;
            let mut direction = forward;
            if angle != 0.0 {
                let (sine, cosine) = angle.sin_cos();
                direction = rotate_about(direction, random.direction(), sine, cosine);
            }
            let inherited = effect.spawn.map_or([0.0; 3], |spawn| spawn.velocity);
            let velocity: [f32; 3] = std::array::from_fn(|axis| direction[axis] * speed + inherited[axis]);
            let [red, green, blue] = running.color;
            let made = new_unattached(def, point, velocity, [1.0, red, green, blue], scale, ground, system_random, room);
            systems.push((index, part.system, made));
        }
    }
}

/// `effect_generate_particles`.
pub(super) fn generate_particles(
    index: usize,
    running: &mut RunningEffect,
    effect: &CeEffectAt,
    particles: &mut Vec<CeParticle>,
    random: &mut Random,
    bones: Option<&[[f32; 4]]>,
) {
    let def = &effect.def;
    let fraction = if running.duration <= 0.0 { 1.0 } else { running.time / running.duration };
    let event = &def.events[running.event];
    for (at, entry) in event.particles.iter().enumerate() {
        let count = running.counts.get(at).copied().unwrap_or(0.0);
        let due = (integral(entry.distribution, fraction) * count) as i32
            - (integral(entry.distribution, running.last_fraction) * count) as i32;
        if due <= 0 || entry.first_person_only || !entry.created {
            continue;
        }
        let Some(instances) = effect.locations.get(entry.location) else {
            continue;
        };
        let particle_def = &def.particles[entry.def];
        for location in instances {
            for _ in 0..due {
                if particles.len() >= MOST_PARTICLES {
                    return;
                }
                let [lower, delta] = scaled(entry.distribution_radius, running, entry.scales_a.distribution_radius, entry.scales_b.distribution_radius);
                let distance = random.range(0.0, delta) + lower;
                let spread = random.direction();
                let offset = turn_by_location(location, entry.offset);
                let local: [f32; 3] =
                    std::array::from_fn(|axis| location.position[axis] + spread[axis] * distance + offset[axis]);
                // `effect_random_translational_velocity`.
                let [lower, delta] = scaled(entry.velocity, running, entry.scales_a.velocity, entry.scales_b.velocity);
                let speed = random.range(0.0, delta) + lower;
                let mut cone = entry.cone;
                if entry.scales_a.cone {
                    cone *= running.scale_a;
                }
                if entry.scales_b.cone {
                    cone *= running.scale_b;
                }
                let mut direction = entry.direction;
                let angle = random.next() * cone;
                if angle != 0.0 {
                    let axis = random.direction();
                    let (sine, cosine) = angle.sin_cos();
                    direction = rotate_about(direction, axis, sine, cosine);
                }
                let direction = turn_by_location(location, direction);
                // Each particle goes with the effect too, which moves per tick.
                let inherited = effect.spawn.map_or([0.0; 3], |spawn| spawn.velocity.map(|axis| axis * 30.0));
                let velocity: [f32; 3] = std::array::from_fn(|axis| direction[axis] * speed + inherited[axis]);
                let attached = entry.flags.stay_attached;
                let (place, direction, velocity) = if attached {
                    (local, direction, velocity)
                } else {
                    let (place, direction) = posed_on(location.node, local, direction, bones);
                    let (_, velocity) = posed_on(location.node, [0.0; 3], velocity, bones);
                    (place, direction, velocity)
                };
                let [lower, delta] = scaled(entry.radius, running, entry.scales_a.radius, entry.scales_b.radius);
                let radius = random.range(0.0, delta) + lower;
                let [lower, delta] = scaled(entry.angular_velocity, running, entry.scales_a.angular_velocity, entry.scales_b.angular_velocity);
                let angular_velocity = random.range(0.0, delta) + lower;
                let rotation =
                    if entry.flags.random_initial_angle { random.range(0.0, std::f32::consts::TAU) } else { 0.0 };
                let mix = if entry.scales_a.tint || entry.scales_b.tint {
                    let mut mix = 1.0;
                    if entry.scales_a.tint {
                        mix = running.scale_a;
                    }
                    if entry.scales_b.tint {
                        mix *= running.scale_b;
                    }
                    mix
                } else {
                    random.next()
                };
                let [low, high] = entry.tint;
                let rgb = ce_color_interpolate(
                    entry.flags.tint_interpolation,
                    [low[0], low[1], low[2]],
                    [high[0], high[1], high[2]],
                    mix,
                );
                let tinted = if entry.flags.tint_from_object {
                    std::array::from_fn(|channel| rgb[channel] * running.color[channel])
                } else {
                    rgb
                };
                // `particle_new`: `light_particle` finds no structure under a
                // preview's particle, so both its light and the diffuse color
                // below it are grey.
                let mut tinted = tinted;
                if !particle_def.flags.self_illuminated {
                    tinted = tinted.map(|channel| channel * PARTICLE_GREY);
                }
                if particle_def.flags.tint_from_diffuse {
                    tinted = tinted.map(|channel| channel * PARTICLE_GREY);
                }
                let alpha = (1.0 - mix) * low[3] + high[3] * mix;
                let mut lifespan = random.range(particle_def.lifespan[0], particle_def.lifespan[1]);
                if lifespan <= 0.0 {
                    lifespan = 0.0001;
                }
                let frame_span = if particle_def.animation_rate[1] == 0.0 {
                    f32::MAX
                } else {
                    1.0 / random.range(particle_def.animation_rate[0], particle_def.animation_rate[1])
                };
                let mut flags = 0;
                if particle_def.flags.can_animate_backwards && random.bits() & 1 != 0 {
                    flags |= BACKWARDS;
                }
                if particle_def.flags.random_horizontal_mirror && random.bits() & 4 != 0 {
                    flags |= MIRROR_U;
                }
                if particle_def.flags.random_vertical_mirror && random.bits() & 8 != 0 {
                    flags |= MIRROR_V;
                }
                let mut particle = CeParticle {
                    effect: index,
                    def: entry.def,
                    node: attached.then_some(location.node),
                    place,
                    direction,
                    velocity,
                    age: 0.0,
                    lifespan,
                    frame_time: -1.0,
                    frame_span,
                    sequence: -1,
                    frame: -1,
                    phase: 0,
                    rotation,
                    angular_velocity,
                    radius,
                    color: [alpha, tinted[0], tinted[1], tinted[2]],
                    flags,
                    undrawn: 0,
                };
                if next_sequence(&mut particle, particle_def, random) {
                    let count = particle_def.sprites_in(particle.sequence) as i32;
                    particle.frame = if particle_def.flags.starts_on_random_frame {
                        let step = if particle.flags & BACKWARDS == 0 { -1 } else { 1 };
                        step + (random.next() * count as f32) as i32
                    } else if particle.flags & BACKWARDS != 0 {
                        count
                    } else {
                        -1
                    };
                    particles.push(particle);
                }
            }
        }
    }
    running.last_fraction = fraction;
}
