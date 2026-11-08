//! The shared ring of cheap particles: stepping, spawning and building vertices.

use super::*;

/// The pool every emitter writes, as the card holds it.
#[derive(Debug)]
pub(crate) struct ReachCheapField {
    pub(super) time: Option<f32>,
    pub(super) particles: Vec<CheapParticle>,
    /// `spawn_offset`.
    pub(super) cursor: usize,
    /// One generator per emitter, seeded so a held pose repeats; the game's
    /// is the local seed [inferred: preview convention].
    random: Vec<Random>,
    /// Each kind of turbulence's `(u, v)` phase.
    pub(super) phases: Vec<[f32; 2]>,
}

impl Default for ReachCheapField {
    fn default() -> Self {
        Self::fresh(0)
    }
}

/// A cheap particle's quad corner, in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ReachCheapVertex {
    pub(crate) place: [f32; 3],
    /// u, v and the render texture's layer.
    pub(crate) texcoord: [f32; 3],
    pub(crate) color: [f32; 4],
}

impl ReachCheapField {
    pub(super) fn fresh(emitters: usize) -> Self {
        Self {
            time: None,
            particles: vec![EMPTY; MOST_PARTICLES],
            cursor: 0,
            random: (0..emitters).map(|at| Random((at as u32 + 1).wrapping_mul(977))).collect(),
            phases: Vec::new(),
        }
    }

    /// Run to `time` a frame at a time, as `c_player_view::render` (the 360's
    /// 0x82729774) does: the emitters alive submitted (`shown` leaves out
    /// those under hidden regions), then `s_cheap_particle_system::update`'s
    /// `update_particles` and `spawn_particles`. A new field starts a
    /// pre-roll short, so a held pose has particles in flight.
    pub(crate) fn advance(
        &mut self,
        emitters: &[ReachCheapEmitterAt],
        shown: &[bool],
        values: impl Fn(u8) -> Option<FunctionValues>,
        onsets: &mut EffectOnsets,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        time: f32,
    ) {
        if self.random.len() != emitters.len() || self.time.is_none_or(|last| time < last) {
            *self = Self::fresh(emitters.len());
            self.time = Some(time - PRE_ROLL);
        }
        let (Some(now), Some(first)) = (self.time, emitters.first()) else {
            return;
        };
        let library = Arc::clone(first.library());
        let mut now = resume_point(now, time, PRE_ROLL);
        let scopes: Vec<Option<FunctionValues>> = emitters.iter().map(|at| values(at.object)).collect();
        // By the frame's clock, which the steps trail.
        let onset_of: Vec<f32> = emitters
            .iter()
            .zip(&scopes)
            .map(|(at, scope)| onsets.of(at.object, &at.scale_by, scope.as_ref(), time))
            .collect();
        for dt in frame_steps(now, time) {
            now += dt;
            let mut submitted = Vec::new();
            for (index, at) in emitters.iter().enumerate() {
                if !shown.get(index).copied().unwrap_or(true) {
                    continue;
                }
                let Some(mut submission) = at.submission(scopes[index].as_ref(), bones, eye, now, onset_of[index]) else {
                    continue;
                };
                if submitted.len() >= MOST_EMITTERS {
                    continue;
                }
                if at.definition.rotate_up {
                    submission.up = turned_up(submission.forward, &mut self.random[index]);
                }
                submitted.push((index, submission));
            }
            self.update(&library, dt);
            for (index, submission) in submitted {
                self.spawn(&emitters[index], &submission, scopes[index].as_ref(), now, dt, index);
            }
        }
        self.time = Some(time);
    }

    /// `update_particles` (Reach: the 360's 0x82DB3790; Halo 4: the 360's
    /// 0x82F2BCF8, MCC 0x140384020) and the update shader: each kind of
    /// turbulence's phase moves by the frame, and every live particle ages
    /// and, unless at rest, moves by drag, gravity and its type's turbulence
    /// (which moves its place, not its velocity), at the shaders' step.
    pub(super) fn update(&mut self, library: &CheapLibrary, dt: f32) {
        self.phases.resize(library.turbulence.len(), [0.0; 2]);
        let transforms: Vec<[f32; 4]> = library
            .turbulence
            .iter()
            .zip(&mut self.phases)
            .map(|(turbulence, phase)| {
                let [du_dt, dv_dt, du_dp, dv_dp] = turbulence.rates;
                *phase = [frac(phase[0] + du_dt * dt), frac(phase[1] + dv_dt * dt)];
                [du_dp, dv_dp, phase[0], phase[1]]
            })
            .collect();
        let dt = dt.min(MOST_SHADER_STEP);
        for (index, particle) in self.particles.iter_mut().enumerate() {
            if particle.age.abs() < 1.0 {
                update_particle(particle, index, library, &transforms, dt);
            }
        }
    }

    /// `s_emitter::spawn` (the 360's 0x82DB2F68) and `s_particle_buffers::spawn`
    /// (0x82DB35E8): how many this frame, from the spawn rate at the game
    /// time (or what the input names) and a fresh roll for its range, then
    /// each at the next ring slot, over whatever was there.
    pub(super) fn spawn(
        &mut self,
        at: &ReachCheapEmitterAt,
        submission: &CheapSubmission,
        values: Option<&FunctionValues>,
        time: f32,
        dt: f32,
        index: usize,
    ) {
        let definition = &*at.definition;
        let random = &mut self.random[index];
        let input = spawn_input(&definition.input, values, time, submission.scale);
        let rolled = random.next();
        let range = if definition.range.is_empty() { rolled } else { object_value(values, &definition.range) };
        let rate = definition.spawn_rate.evaluate(input, range);
        let count = spawn_count(rate, dt, submission.scale, random.range(0.0, 1.0 - 1e-4));
        if count == 0 {
            return;
        }
        let transforms = spawn_transforms(random, definition.correlate);
        let lighting = packed_lighting(definition.lighting, NO_PROBE_INTENSITY);
        let shader_dt = dt.min(MOST_SHADER_STEP);
        for _ in 0..count {
            let slot = self.cursor;
            self.particles[slot] = definition.spawn_particle(submission, &transforms, lighting, slot, shader_dt);
            self.cursor = (slot + 1) % MOST_PARTICLES;
        }
    }

    /// `cheap_particles_render.hlsl`'s quads in ring-slot order, as two
    /// triangles each: facing along the velocity, stretched by its speed, or
    /// the screen; coloured by the type's colour, its fade and the packed
    /// lighting. There is no fog to scatter in.
    pub(crate) fn vertices(&self, library: &CheapLibrary, view: &ViewFrame) -> Vec<ReachCheapVertex> {
        let mut vertices = Vec::new();
        for (index, particle) in self.particles.iter().enumerate() {
            if particle.age.abs() > 1.0 {
                continue;
            }
            let [kind, lighting, dx, dy] = particle.parameters;
            let Some(found) = usize::try_from(kind).ok().and_then(|kind| library.types.get(kind)) else {
                continue;
            };
            let [_, size, stretch_factor, y_scale] = found.rows[RENDER];
            // Halo 4's lies between `size`'s bounds, by a cheap hash of the
            // slot (`frac(index · 0.863182394)`).
            let size = if library.halo4 {
                let [lower, upper, _, _] = found.rows[FADE];
                lower + (upper - lower) * frac(index as f32 * 0.863_182_4)
            } else {
                size
            };
            let to_particle = clamped_unit(minus(particle.position, view.eye));
            // Pulled 1 cm toward the camera.
            let place: [f32; 3] = std::array::from_fn(|axis| particle.position[axis] - to_particle[axis] * 0.01);
            let local = [f32::from(dx) / 63.5 * size, f32::from(dy) / 63.5 * size];
            let velocity = particle.velocity;
            let speed = dot(velocity, velocity).sqrt();
            let basis = if y_scale > 0.0 {
                // Unguarded in the shader: at rest the quad is NaN.
                if speed == 0.0 {
                    continue;
                }
                let stretch = (stretch_factor * speed).max(1.0) / speed;
                let along = velocity.map(|axis| axis * stretch);
                [along, clamped_unit(cross(along, to_particle))]
            } else {
                [view.right, view.up]
            };
            let fade = found.rows[FADE][3];
            let blend = (fade - (fade * particle.age).abs()).clamp(0.0, 1.0);
            let [red, green, blue, alpha] = found.rows[COLOR0];
            let lit = blend * illumination(lighting);
            let color = [red * lit, green * lit, blue * lit, (alpha * blend).clamp(0.0, 1.0)];
            // `generate_quad_point_2d`'s Direct3D 11 order, as a strip.
            let corner = |[x, y]: [f32; 2]| {
                let (across, down) = (x - 0.5, y - 0.5);
                let offset = [across * local[0] - down * local[1], across * local[1] + down * local[0]];
                ReachCheapVertex {
                    place: std::array::from_fn(|axis| place[axis] + offset[0] * basis[0][axis] + offset[1] * basis[1][axis]),
                    texcoord: [x, (y - 0.5) * y_scale + 0.5, found.texture as f32],
                    color,
                }
            };
            let corners = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]].map(corner);
            vertices.extend([0, 1, 2, 2, 1, 3].map(|at| corners[at]));
        }
        vertices
    }
}

/// `update.hlsl`'s `update_particle` for a live particle at ring slot
/// `index`, with each kind of turbulence's `(du_dp, dv_dp, phase)` and the
/// shaders' `dt`.
pub(super) fn update_particle(particle: &mut CheapParticle, index: usize, library: &CheapLibrary, transforms: &[[f32; 4]], dt: f32) {
    let kind = usize::try_from(particle.parameters[0]).unwrap_or(usize::MAX);
    let [drag, gravity, strength, place] = library.row(kind, PHYSICS);
    let (layer, transform) = if library.halo4 { turbulence_of(place) } else { (0, 0) };
    let gravity = -gravity;
    particle.age += sign(particle.age) * particle.delta_age * dt;
    if particle.age >= 0.0 {
        // The library's `turbulence` on the sampler the shader calls
        // `velocity_texture`, point sampled [inferred: MCC's filter mode 2
        // was not mapped].
        let push = match (library.turbulence.get(layer), transforms.get(transform)) {
            (Some(turbulence), Some(transform)) => turbulence.push(index, *transform),
            _ => [0.0; 4],
        };
        for axis in 0..3 {
            particle.position[axis] += (particle.velocity[axis] + push[axis] * strength) * dt;
        }
        particle.position[2] += gravity * dt * dt;
        let pull = [0.0, 0.0, gravity];
        for axis in 0..3 {
            particle.velocity[axis] += (pull[axis] - particle.velocity[axis] * drag) * dt;
        }
    }
    // Here the game projects the particle into the screen and collides it
    // with the depth and normal buffers (`_surface_depth_stencil`,
    // `_surface_normal_biased`): a bounce by `elasticity`, rest, a kill on
    // leaving the surface and the type's `change type` all happen only
    // there. The preview has no such buffers to read back and leaves it out
    // (cheap particle spec §4.1–4.2, open item 1).
}
