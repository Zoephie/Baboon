//! One effect particle's step: its frame sequence and its update.

use super::*;

/// `particle_datum` flags.
pub(super) const BACKWARDS: u8 = 1;

const AT_REST: u8 = 2;

pub(super) const MIRROR_U: u8 = 4;

pub(super) const MIRROR_V: u8 = 8;

/// `particle_next_sequence`: false where the particle dies.
pub(super) fn next_sequence(particle: &mut CeParticle, def: &CeParticleDef, random: &mut CeRandom) -> bool {
    let [initial, looping, last] = def.sequence_counts;
    let mut pick = |offset: i32, count: i32| def.first_sequence + offset + (random.next() * count as f32) as i32;
    particle.sequence = -1;
    if particle.phase == 0 {
        if initial > 0 {
            particle.sequence = pick(0, initial);
        }
        particle.phase += 1;
    }
    if particle.sequence == -1 && particle.phase == 1 {
        particle.phase = 2;
    }
    if particle.phase == 2 {
        if particle.age >= particle.lifespan || looping <= 0 {
            particle.phase += 1;
        } else {
            particle.sequence = pick(initial, looping);
        }
    }
    if particle.sequence == -1 && particle.phase == 3 {
        if last > 0 {
            particle.sequence = pick(initial + looping, last);
        }
        particle.phase += 1;
    }
    if particle.sequence == -1 || def.sequences.is_empty() {
        return false;
    }
    particle.sequence = particle.sequence.clamp(0, def.sequences.len() as i32 - 1);
    true
}

/// `particle_next_frame`: false where the particle dies.
fn next_frame(particle: &mut CeParticle, def: &CeParticleDef, random: &mut CeRandom) -> bool {
    particle.frame_time = 0.0;
    if particle.flags & BACKWARDS != 0 {
        if particle.frame <= 0 {
            if !next_sequence(particle, def, random) {
                return false;
            }
            particle.frame = def.sprites_in(particle.sequence) as i32 - 1;
        } else {
            particle.frame -= 1;
        }
        true
    } else {
        let next = particle.frame + 1;
        if next >= def.sprites_in(particle.sequence) as i32 {
            let alive = next_sequence(particle, def, random);
            particle.frame = 0;
            alive
        } else {
            particle.frame = next;
            true
        }
    }
}

/// `particles_update` for one particle: how it ends, and the scale of any
/// collision effect its hitting structure makes.
pub(super) fn update_particle(particle: &mut CeParticle, def: &CeParticleDef, random: &mut CeRandom, ground: f32, dt: f32) -> (Fate, Option<f32>) {
    let first = particle.age == 0.0;
    particle.age += dt;
    if particle.age >= particle.lifespan && !first && def.sequence_counts[2] == 0 {
        return (Fate::Died, None);
    }
    // `particle_update_frame_time`.
    if !def.flags.animation_stops_at_rest || particle.flags & AT_REST == 0 {
        if def.flags.animate_once_per_frame {
            if !next_frame(particle, def, random) {
                return (Fate::Died, None);
            }
        } else {
            let mut alive = true;
            if particle.frame_time == -1.0 {
                alive = next_frame(particle, def, random);
                particle.frame_time = 0.0;
            }
            let mut left = dt;
            while alive {
                let span = particle.frame_span - particle.frame_time;
                if span > left {
                    particle.frame_time += left;
                    break;
                }
                alive = next_frame(particle, def, random);
                left -= span;
                if left <= 0.0 {
                    break;
                }
            }
            if !alive {
                return (Fate::Died, None);
            }
        }
    }
    // `particle_update_physics`.
    if particle.flags & AT_REST != 0 {
        return (Fate::Alive, None);
    }
    let radius = def.radius_scale(particle.age, particle.lifespan) * particle.radius;
    let mut resting = false;
    let mut collided = None;
    match (particle.node, def.physics) {
        (Some(_), Some(physics)) => {
            let drag = physics.air_friction * radius * radius;
            let mass = physics.mass(radius);
            let keep = if mass == 0.0 {
                if drag == 0.0 { 1.0 } else { 0.0 }
            } else {
                (1.0 - drag / mass * dt).clamp(0.0, 1.0)
            };
            for axis in 0..3 {
                particle.velocity[axis] *= keep;
                particle.place[axis] += particle.velocity[axis] * dt;
            }
            resting = true;
        }
        (Some(_), None) => {
            for axis in 0..3 {
                particle.place[axis] += particle.velocity[axis] * dt;
            }
            resting = true;
        }
        (None, Some(physics)) => {
            if physics.step(&mut particle.place, &mut particle.velocity, radius, ground, dt) {
                // `particle_update_physics`: what it hit sets off its
                // collision effect, as hard as it is going past half a unit.
                let scale = (dot(particle.velocity, particle.velocity).sqrt() - 0.5).clamp(0.0, 1.0);
                collided = Some(scale);
                if def.flags.dies_on_structure {
                    let fate = if def.collision_effect.is_none() { Fate::Died } else { Fate::Deleted };
                    return (fate, collided);
                }
                resting = true;
                particle.frame_span += def.contact_deterioration;
            }
            // Always in air: the preview has no water.
            if def.flags.dies_in_air {
                return (Fate::Died, collided);
            }
        }
        (None, None) => {
            for axis in 0..3 {
                particle.place[axis] += particle.velocity[axis] * dt;
            }
        }
    }
    let speed = dot(particle.velocity, particle.velocity);
    if speed >= 0.0625 || !resting {
        if speed >= 0.0625 {
            particle.direction = particle.velocity;
        }
        particle.rotation += particle.angular_velocity * dt;
        return (Fate::Alive, collided);
    }
    if particle.node.is_none() {
        if def.flags.dies_at_rest {
            return (Fate::Died, collided);
        }
        particle.flags |= AT_REST;
    }
    particle.rotation += particle.angular_velocity * dt;
    (Fate::Alive, collided)
}
