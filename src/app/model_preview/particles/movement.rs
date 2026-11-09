//! Movement controllers, read and run: physics, the collider, flocks and turbulence.

use super::*;

/// `set_shader_update_state`'s three, each off without a physics controller.
#[derive(Debug)]
pub(in crate::app::model_preview) struct H3ParticleFlock {
    separation_distance: H3ParticleProperty,
    separation_weight: H3ParticleProperty,
    cohesion_distance: H3ParticleProperty,
    cohesion_weight: H3ParticleProperty,
    alignment_weight: H3ParticleProperty,
    minimum_velocity: H3ParticleProperty,
    maximum_velocity: H3ParticleProperty,
    maximum_acceleration: H3ParticleProperty,
}

/// `update_particles_collider`'s two, against the preview's ground.
#[derive(Debug)]
pub(in crate::app::model_preview) struct H3ParticleCollider {
    elasticity: H3ParticleProperty,
    friction: H3ParticleProperty,
}

/// A Halo 3 or ODST emitter's `collider` movement, when it collides with
/// structure, the only thing the preview's ground stands for
/// (`update_particles_collider`): friction is parameter 0 and elasticity 1.
pub(super) fn read_collider(
    reader: &TagReader,
    emitter: TagStruct<'_>,
    flagged: impl Fn(&str) -> bool,
    seeds: &mut SeedAllocator,
) -> Option<H3ParticleCollider> {
    let movement = emitter.descend("particle movement").filter(|_| flagged("collide with structure"))?;
    let friction = movement_parameter(reader, movement, "collider", 0, seeds)?;
    Some(H3ParticleCollider { elasticity: movement_parameter(reader, movement, "collider", 1, seeds)?, friction })
}

/// A Halo 2 `particle_physics`' `collider` movement, its own or its
/// template's, if it collides with structure: friction is parameter 0 and
/// elasticity 1, as `sub_53DF20` reads them.
pub(in crate::app::model_preview) fn read_collider_of(reader: &TagReader, movement: TagStruct<'_>, seeds: &mut SeedAllocator) -> Option<H3ParticleCollider> {
    let template = movement
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| reader.load(&path, "particle_physics", b"pmov").ok());
    let own = elements(movement, "movements");
    let held = template.as_ref().map(|tag| tag.root());
    let collides = |holder: Option<TagStruct<'_>>| {
        holder.is_some_and(|holder| has_flag(holder, "flags", "collide with structure"))
    };
    if !collides(Some(movement)) && !collides(held) {
        return None;
    }
    let movements = if own.is_empty() { held.map_or(Vec::new(), |root| elements(root, "movements")) } else { own };
    let entry = movements.into_iter().find(|entry| entry.read_enum_option("type").as_deref() == Some("collider"))?;
    let mut by_id = |id: i128| {
        let parameters = elements(entry, "parameters");
        let found = parameters.into_iter().find(|parameter| parameter.read_int_any("parameter id") == Some(id))?;
        H3ParticleProperty::named(found, "property", seeds)
    };
    let friction = by_id(0)?;
    Some(H3ParticleCollider { elasticity: by_id(1)?, friction })
}

/// What a particle does to itself where its step crosses the ground, which
/// runs along `up` and lies `height` below its frame's origin: the part of its
/// velocity into the ground bounces, and the part along it drags.
pub(super) fn bounce(
    position: [f32; 3],
    velocity: [f32; 3],
    (up, height): ([f32; 3], f32),
    dt: f32,
    elasticity: f32,
    friction: f32,
) -> Option<([f32; 3], [f32; 3])> {
    let before = dot(up, position) + height;
    let after = before + dot(up, velocity) * dt;
    if before < 0.0 || after >= 0.0 {
        return None;
    }
    let share = before / (before - after) * dt;
    let keep = 1.0 - friction;
    let into = dot(velocity, up);
    let place = std::array::from_fn(|axis| position[axis] + velocity[axis] * share + up[axis] * PUSH_OUT);
    let along: [f32; 3] = std::array::from_fn(|axis| velocity[axis] - up[axis] * into);
    Some((place, std::array::from_fn(|axis| keep * along[axis] - elasticity * up[axis] * into)))
}

/// `update_particles_collider`'s `push_out_distance`.
pub(super) const PUSH_OUT: f32 = 0.005;

/// A Halo 3 or ODST emitter's `swarm` movement, when its flags name one.
pub(super) fn read_flock(
    reader: &TagReader,
    emitter: TagStruct<'_>,
    flagged: impl Fn(&str) -> bool,
    seeds: &mut SeedAllocator,
) -> Option<H3ParticleFlock> {
    let movement = emitter.descend("particle movement").filter(|_| flagged("swarm"))?;
    let mut by_id = |id: i128| movement_parameter(reader, movement, "swarm", id, seeds);
    Some(H3ParticleFlock {
        separation_distance: by_id(0)?,
        separation_weight: by_id(1)?,
        cohesion_distance: by_id(2)?,
        cohesion_weight: by_id(3)?,
        alignment_weight: by_id(4)?,
        minimum_velocity: by_id(6)?,
        maximum_velocity: by_id(7)?,
        maximum_acceleration: by_id(8)?,
    })
}

impl H3ParticleCollider {
    /// `c_particle_controller::update_particles_collider`, on the preview's ground.
    pub(super) fn update(
        &self,
        particles: &mut [Particle],
        state: H3ParticleState,
        plane: ([f32; 3], f32),
        dies: bool,
        hits: &mut Vec<(u64, [f32; 3], [f32; 3])>,
        dt: f32,
    ) {
        for particle in particles.iter_mut().filter(|particle| !particle.resting) {
            let state = H3ParticleState { age: particle.age, seeds: particle.seeds, rotation: particle.rotation, ..state };
            let (elasticity, friction) = (self.elasticity.scalar(&state), self.friction.scalar(&state));
            match bounce(particle.position, particle.velocity, plane, dt, elasticity, friction) {
                Some((place, velocity)) => {
                    particle.position = place;
                    particle.velocity = velocity;
                    particle.rotation_rate *= 1.0 - friction;
                    particle.landed = plane.0[2] > 0.8;
                    if !particle.colliding {
                        hits.push((particle.id, place, velocity));
                        particle.colliding = true;
                    }
                    if dies {
                        particle.age = 1.0;
                    }
                }
                None => particle.colliding = false,
            }
        }
    }
}

impl H3ParticlePhysics {
    /// `c_particle_emitter_gpu::set_shader_update_state`: gravity, air
    /// friction and rotational friction, once for the emitter at its system's
    /// states.
    pub(super) fn on_card(&self, state: &H3ParticleState) -> [f32; 3] {
        let read = |property: &Option<H3ParticleProperty>| property.as_ref().map(|property| property.scalar(state));
        [
            read(&self.gravity).unwrap_or(1.0) * GRAVITY,
            read(&self.air_friction).unwrap_or(1.0),
            read(&self.rotational_friction).unwrap_or(0.0),
        ]
    }

    /// `c_particle_controller::update_particles_physics`.
    pub(super) fn update(&self, particles: &mut [Particle], state: H3ParticleState, dt: f32) {
        for particle in particles.iter_mut().filter(|particle| !particle.resting) {
            let state = H3ParticleState {
                age: particle.age,
                seeds: particle.seeds,
                system_seeds: particle.system_seeds,
                emission_time: particle.emission_time,
                rotation: particle.rotation,
                ..state
            };
            let read = |property: &Option<H3ParticleProperty>| property.as_ref().map(|property| property.scalar(&state));
            particle.velocity[2] -= read(&self.gravity).unwrap_or(1.0) * GRAVITY * dt;
            let air = (read(&self.air_friction).unwrap_or(1.0) * dt).clamp(0.0, 1.0);
            particle.velocity = particle.velocity.map(|axis| axis - air * axis);
            let spin = (read(&self.rotational_friction).unwrap_or(0.0) * dt).clamp(0.0, 1.0);
            particle.rotation_rate -= spin * particle.rotation_rate;
        }
    }
}

impl H3ParticleFlock {
    /// `update_particles_flocking`, over the processor's particles: those
    /// moved to the card are off its list. Avoidance is of a world the
    /// preview lacks.
    pub(super) fn update(&self, particles: &mut [Particle], base: H3ParticleState, home: [f32; 3], dt: f32) {
        let on_processor = |particle: &Particle| (!particle.on_card).then_some((particle.position, particle.velocity));
        let places: Vec<Option<([f32; 3], [f32; 3])>> = particles.iter().map(on_processor).collect();
        for (at, particle) in particles.iter_mut().enumerate().filter(|(_, particle)| !particle.on_card) {
            let state = H3ParticleState { age: particle.age, seeds: particle.seeds, rotation: particle.rotation, ..base };
            let distance_to = |other: &([f32; 3], [f32; 3])| {
                let delta = minus(other.0, particle.position);
                dot(delta, delta)
            };
            let others = places.iter().enumerate().filter(|(index, _)| *index != at);
            let neighbors = others.filter_map(|(_, neighbor)| neighbor.as_ref());
            let neighbor = neighbors.min_by(|a, b| distance_to(a).total_cmp(&distance_to(b)));
            let mut force = [0.0; 3];
            let mut add = |amount: [f32; 3]| force = std::array::from_fn(|axis| force[axis] + amount[axis]);
            if let Some((place, velocity)) = neighbor {
                let weight = self.separation_weight.scalar(&state);
                if weight != 0.0 {
                    let toward = minus(*place, particle.position);
                    let ratio = dot(toward, toward).sqrt() / self.separation_distance.scalar(&state).max(0.1);
                    let scale = if ratio < 1.0 { -ratio * weight } else { weight };
                    add(clamped_unit(toward).map(|axis| axis * scale));
                }
                let (mine, theirs) = (clamped_unit(particle.velocity), clamped_unit(*velocity));
                let weight = self.alignment_weight.scalar(&state);
                add(minus(theirs, mine).map(|axis| axis * weight));
            }
            let reach = self.cohesion_distance.scalar(&state);
            if reach > 0.0 {
                let toward = minus(home, particle.position);
                let weight = self.cohesion_weight.scalar(&state) * dot(toward, toward).sqrt() / reach;
                add(clamped_unit(toward).map(|axis| axis * weight));
            }
            let mut change = force.map(|axis| axis * dt);
            let most = self.maximum_acceleration.scalar(&state);
            if dot(change, change).sqrt() > most {
                change = clamped_unit(change).map(|axis| axis * most);
            }
            let eased: [f32; 3] = std::array::from_fn(|axis| {
                (particle.velocity[axis] + change[axis]) * 0.25 + particle.velocity[axis] * 0.75
            });
            let (low, high) = (self.minimum_velocity.scalar(&state), self.maximum_velocity.scalar(&state));
            let speed = dot(eased, eased).sqrt();
            particle.velocity = if speed > high {
                clamped_unit(eased).map(|axis| axis * high)
            } else if speed < low {
                clamped_unit(eased).map(|axis| axis * low)
            } else {
                eased
            };
        }
    }
}

#[derive(Debug)]
pub(in crate::app::model_preview) struct H3ParticlePhysics {
    pub(super) gravity: Option<H3ParticleProperty>,
    pub(super) air_friction: Option<H3ParticleProperty>,
    pub(super) rotational_friction: Option<H3ParticleProperty>,
}

/// Reach's `turbulence` movement, which `particle_update.hlsl` runs on the
/// card: after a particle moves and before it accelerates, it is pushed by a
/// texel of the movement's texture, picked by its birth time along u and its
/// fifth seed along v.
#[derive(Debug)]
pub(in crate::app::model_preview) struct H3ParticleTurbulence {
    /// Parameter 0, read for each particle: `_index_emitter_movement_turbulence`,
    /// from `get_parameter_by_id_indexed(turbulence, 0)` in `populate_gpu_data`
    /// (the 360's 0x82EE8350).
    pub(super) strength: H3ParticleProperty,
    /// `c_particle_emitter_gpu::frame_advance<update>` (the 360's 0x826CAF88):
    /// parameters 3, 4, 1 and 2, read once a frame at the system's states, as
    /// u's and v's scale and then their offsets.
    pub(super) xform: [H3ParticleProperty; 4],
    /// The movement's `turbulence texture`, decoded once.
    pub(super) image: TextureImage,
}

/// What tool's `postprocess_internal` gives a turbulence movement that, nor
/// its template, names a texture.
const DEFAULT_TURBULENCE: &str = "fx\\bitmaps\\turbulence\\default_turbulence";

/// A Reach emitter's `turbulence` movement, when its flags name one. Tool's
/// table (0x1427E4208) fills in a missing parameter 0 with 1, 1 and 2 with 0,
/// 3 with 1 and 4 with 0.3.
pub(super) fn read_turbulence(
    reader: &TagReader,
    emitter: TagStruct<'_>,
    flagged: impl Fn(&str) -> bool,
    seeds: &mut SeedAllocator,
) -> Option<H3ParticleTurbulence> {
    let movement = emitter.descend("particle movement").filter(|_| flagged("turbulence"))?;
    let mut parameter = |id: i128, default: f32| movement_parameter_or(reader, movement, "turbulence", id, default, seeds);
    let strength = parameter(0, 1.0)?;
    let defaults = [(1, 0.0), (2, 0.0), (3, 1.0), (4, 0.3)];
    let [u_offset, v_offset, u_scale, v_scale] = defaults.map(|(id, default)| parameter(id, default));
    let named = |holder: TagStruct<'_>| holder.read_tag_ref_path("turbulence texture").filter(|path| !path.is_empty());
    let template = movement
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| reader.load(&path, "particle_physics", b"pmov").ok());
    let path = named(movement)
        .or_else(|| template.as_ref().and_then(|tag| named(tag.root())))
        .unwrap_or_else(|| DEFAULT_TURBULENCE.to_owned());
    let image = match decode_image(reader, &path, SamplerKind::Texture2D, 0)? {
        Image::Flat(image) if image.whole() && image.width > 0 && image.height > 0 => image,
        _ => return None,
    };
    Some(H3ParticleTurbulence { strength, xform: [u_scale?, v_scale?, u_offset?, v_offset?], image })
}

impl H3ParticleTurbulence {
    pub(super) fn xform(&self, state: &H3ParticleState) -> [f32; 4] {
        self.xform.each_ref().map(|property| property.scalar(state))
    }
}

/// `particle_update.hlsl`'s push: the texel at `(birth time, seed 5) · xform.xy
/// + xform.zw`, less a half, times `scale` (the strength over the frame).
pub(super) fn turbulence_push(image: &TextureImage, xform: [f32; 4], [birth, seed]: [f32; 2], scale: f32) -> [f32; 3] {
    let uv = [birth * xform[0] + xform[2], seed * xform[1] + xform[3]];
    sample_wrapped(image, uv).map(|channel| (channel - 0.5) * scale)
}

/// `tex2Dlod` at level 0 on the sampler the turbulence is bound with
/// (vertex sampler 22): address mode 0, wrap, and filter 2, bilinear
/// [inferred: Baboon reads those values so elsewhere].
pub(super) fn sample_wrapped(image: &TextureImage, [u, v]: [f32; 2]) -> [f32; 3] {
    let (width, height) = (image.width as i64, image.height as i64);
    let (x, y) = (u * width as f32 - 0.5, v * height as f32 - 0.5);
    let (left, top) = (x.floor(), y.floor());
    let (across, down) = (x - left, y - top);
    let texel = |column: i64, row: i64| -> [f32; 3] {
        let at = (row.rem_euclid(height) * width + column.rem_euclid(width)) as usize * 4;
        std::array::from_fn(|channel| image.rgba[at + channel] as f32 / 255.0)
    };
    let (left, top) = (left as i64, top as i64);
    let [a, b, c, d] = [texel(left, top), texel(left + 1, top), texel(left, top + 1), texel(left + 1, top + 1)];
    std::array::from_fn(|channel| {
        let upper = a[channel] + (b[channel] - a[channel]) * across;
        let lower = c[channel] + (d[channel] - c[channel]) * across;
        upper + (lower - upper) * down
    })
}
