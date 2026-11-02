//! Halo 3's leaf systems: a few leaves let go near a point, fall, flutter and
//! tumble on the wind that moves cloth. Nothing here touches a level.

use std::sync::Arc;

use blam_tags::TagFile;

use super::ce_attachments::{elements, object_block};
use super::h3_cloth::{H3Wind, away_from_pole, turned_about};
use super::h3_effects::marker_name;
use super::h3_particles::{H3ParticleVertex, H3SpriteShader};
use super::h3_shaders::H3Blend;
use super::materials::read_h3_bitmap;
use super::renderer::H3_EXPOSURE;
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;

/// `k_maximum_leaves_per_system`.
const MOST_LEAVES: usize = 64;
const TICK: f32 = 1.0 / 30.0;
/// `leaf_system_update_physics`' constants.
const MOST_WIND: f32 = 0.0087;
const WIND_EXCLUSION: f32 = 150.0 * std::f32::consts::PI / 180.0;
const WIND_FRICTION: f32 = 5.0;
const PENDULUM_SLOWDOWN: f32 = 0.2;
/// `k_real_epsilon`, and `_valid_real_epsilon` for a turn worth making.
const EPSILON: f32 = 0.0001;
const SMALLEST_TURN: f32 = 0.001;
const TURNS_PER_SECOND: f32 = 0.003;
/// `C0_constant_scale`: from a light probe's first term to a color.
const PROBE_CONSTANT: f32 = 0.282_095;
/// `leaf_system_render_opaque`'s `width_to_depth_multiplier`.
const DEPTH_OF_WIDTH: f32 = 0.15;
/// How long a new system runs before it shows, at most: leaves come slowly.
const MOST_SETTLE: f32 = 60.0;

const STOPS: [f32; 4] = [-1.0, -0.34, 0.32, 1.0];
/// `k_verts`: a rough disc in y and z.
const VERTICES: [[f32; 2]; 12] = [
    [STOPS[1], STOPS[0]],
    [STOPS[2], STOPS[0]],
    [STOPS[0], STOPS[1]],
    [STOPS[1], STOPS[1]],
    [STOPS[2], STOPS[1]],
    [STOPS[3], STOPS[1]],
    [STOPS[0], STOPS[2]],
    [STOPS[1], STOPS[2]],
    [STOPS[2], STOPS[2]],
    [STOPS[3], STOPS[2]],
    [STOPS[1], STOPS[3]],
    [STOPS[2], STOPS[3]],
];
/// `k_indices`.
const TRIANGLES: [[usize; 3]; 14] = [
    [0, 2, 3],
    [3, 1, 0],
    [3, 4, 1],
    [1, 4, 5],
    [2, 6, 3],
    [3, 6, 7],
    [7, 8, 3],
    [3, 8, 4],
    [4, 8, 5],
    [5, 8, 9],
    [6, 10, 7],
    [7, 10, 8],
    [8, 10, 11],
    [11, 9, 8],
];

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt();
    if length > 0.0 { a.map(|axis| axis / length) } else { a }
}

/// `normals_interpolate`.
fn normals_between(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let angle = dot(a, b).clamp(-1.0, 1.0).acos();
    let (from, axis, by) = if t <= 0.5 { (a, cross(a, b), angle * t) } else { (b, cross(b, a), angle * (1.0 - t)) };
    let axis = unit(axis);
    if axis == [0.0; 3] { from } else { unit(turned_about(from, axis, by)) }
}

/// `matrix4x3_multiply(orientation, rotation)`, for a rotation about a world
/// axis: its turn is taken in the orientation's own terms, as the game's is.
fn multiplied(axes: [[f32; 3]; 3], axis: [f32; 3], angle: f32) -> [[f32; 3]; 3] {
    const BASIS: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    BASIS.map(|column| {
        let turned = turned_about(column, axis, angle);
        std::array::from_fn(|at| (0..3).map(|row| axes[row][at] * turned[row]).sum())
    })
}

#[derive(Debug, Clone)]
struct LeafType {
    frequency: f32,
    mass: f32,
    scale: [f32; 2],
    flitteriness: f32,
    swing: f32,
    flutter_scale: f32,
    flutter_speed: f32,
    flutter_phase: f32,
    tumble: f32,
    rotation: f32,
    starting_velocity: f32,
    friction: [f32; 2],
    /// `left`, `right`, `top` and `bottom` of its sprite.
    bounds: [f32; 4],
}

/// A `leaf_system`, at its object.
#[derive(Debug)]
pub(crate) struct H3LeavesAt {
    origin: [f32; 3],
    emission_rate: [f32; 2],
    lifetime: f32,
    wind_range: [f32; 2],
    wind_scale: f32,
    time_scale: f32,
    sphere: f32,
    cylinder: f32,
    types: Vec<LeafType>,
    shader: H3SpriteShader,
}

impl H3LeavesAt {
    pub(crate) fn shader(&self) -> &H3SpriteShader {
        &self.shader
    }

    pub(crate) fn origin(&self) -> [f32; 3] {
        self.origin
    }
}

/// What an object's `leaf_system`s come to: at their marker, or the middle of
/// the object as `object_get_bounding_sphere` has it.
pub(super) fn read_h3_leaves(
    source: &TagSource,
    object: &TagFile,
    markers: &[RenderModelPreviewMarker],
    middle: [f32; 3],
) -> Vec<H3LeavesAt> {
    let group = u32::from_be_bytes(*b"lswd");
    let widgets = object_block(object, "widgets").map_or(Vec::new(), |holder| elements(holder, "widgets"));
    let mut systems = Vec::new();
    for widget in widgets {
        let Some((found, path)) = widget.read_tag_ref_with_group("type") else {
            continue;
        };
        if found != group || path.is_empty() {
            continue;
        }
        let Ok(tag) = load_referenced_tag_from_source(source, &path, "leaf_system", b"lswd") else {
            continue;
        };
        let root = tag.root();
        let Some(plate) = root.read_tag_ref_path("bitmap sprite plate").filter(|path| !path.is_empty()) else {
            continue;
        };
        let bitmap = load_referenced_tag_from_source(source, &plate, "bitmap", b"bitm").ok();
        let sprites = bitmap.as_ref().map_or(Vec::new(), |bitmap| {
            elements(bitmap.root(), "sequences").first().map_or(Vec::new(), |sequence| elements(*sequence, "sprites"))
        });
        let mut bitmap_index = 0;
        let types: Vec<LeafType> = elements(root, "leaf types")
            .iter()
            .map(|leaf| {
                let real = |name: &str| leaf.read_real(name).unwrap_or(0.0);
                let sprite = sprites.get(leaf.read_int_any("bitmap sprite index").unwrap_or(0).max(0) as usize);
                if let Some(sprite) = sprite {
                    bitmap_index = sprite.read_int_any("bitmap index").unwrap_or(0) as i16;
                }
                let bounds = sprite.map_or([0.0, 1.0, 0.0, 1.0], |sprite| {
                    ["left", "right", "top", "bottom"].map(|name| sprite.read_real(name).unwrap_or(0.0))
                });
                let scale = leaf.read_real_bounds("geometry scale");
                let friction = match leaf.field("air friction XY and Z").and_then(|field| field.value()) {
                    Some(blam_tags::TagFieldData::FractionBounds(bounds)) => [bounds.lower, bounds.upper],
                    _ => [0.0; 2],
                };
                LeafType {
                    frequency: real("frequency"),
                    mass: real("mass"),
                    scale: [scale.lower, scale.upper],
                    flitteriness: real("flitteriness"),
                    swing: real("flitteriness swing arm length"),
                    flutter_scale: real("flitteriness scale"),
                    flutter_speed: real("flitteriness speed"),
                    flutter_phase: real("flitteriness leaves phase"),
                    tumble: real("tumble scale"),
                    rotation: real("rotation scale"),
                    starting_velocity: real("starting velocity"),
                    friction,
                    bounds,
                }
            })
            .collect();
        let Some(map) = read_h3_bitmap(source, &plate, bitmap_index, SamplerKind::Texture2D) else {
            continue;
        };
        let name = marker_name(root, "marker attach name");
        let marker = markers.iter().find(|marker| !name.is_empty() && marker.name.eq_ignore_ascii_case(&name));
        let real = |name: &str| root.read_real(name).unwrap_or(0.0);
        let bounds = |name: &str| {
            let bounds = root.read_real_bounds(name);
            [bounds.lower, bounds.upper]
        };
        systems.push(H3LeavesAt {
            origin: marker.map_or(middle, |marker| marker.position),
            emission_rate: bounds("emission rate"),
            lifetime: real("lifetime"),
            wind_range: bounds("wind min/max"),
            wind_scale: real("wind scale"),
            time_scale: real("time scale"),
            sphere: real("emissions sphere radius"),
            cylinder: real("movement cylinder radius"),
            types,
            shader: H3SpriteShader::alpha_tested(H3Blend::AlphaBlend, map),
        });
    }
    systems
}

#[derive(Debug, Clone)]
struct Leaf {
    place: [f32; 3],
    /// `orientation`'s forward, left and up.
    axes: [[f32; 3]; 3],
    kind: usize,
    normal: [f32; 3],
    pendulum: [f32; 3],
    velocity: [f32; 3],
    age: f32,
    rotation: u16,
}

#[derive(Debug, Clone, Default)]
struct RunningLeaves {
    leaves: Vec<Option<Leaf>>,
    next_emission: f32,
    alive: f32,
    random: u32,
}

impl RunningLeaves {
    fn next(&mut self) -> f32 {
        self.random = self.random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.random >> 16) as f32 / 65_536.0
    }

    fn direction(&mut self) -> [f32; 3] {
        let z = self.next() * 2.0 - 1.0;
        let (sin, cos) = (self.next() * std::f32::consts::TAU).sin_cos();
        let flat = (1.0 - z * z).max(0.0).sqrt();
        [flat * cos, flat * sin, z]
    }

    /// `leaf_system_emit`.
    fn emit(&mut self, system: &H3LeavesAt, dt: f32) {
        self.next_emission -= dt;
        let total: f32 = system.types.iter().map(|kind| kind.frequency).sum();
        if self.next_emission > 0.0 || total <= 0.0 {
            return;
        }
        let Some(slot) = self.leaves.iter().position(Option::is_none) else {
            return;
        };
        let offset = self.direction();
        let reach = system.sphere * self.next();
        let place = std::array::from_fn(|axis| system.origin[axis] + offset[axis] * reach);
        let normal = self.direction();
        let pendulum = unit(cross(normal, [0.0, 0.0, 1.0]));
        let mut pick = self.next() * total;
        let mut kind = 0;
        for (at, leaf) in system.types.iter().enumerate() {
            if pick < leaf.frequency {
                kind = at;
                break;
            }
            pick -= leaf.frequency;
        }
        let leaf_type = &system.types[kind];
        let mut velocity = self.direction();
        velocity[2] -= leaf_type.mass.max(0.0);
        velocity[2] = -velocity[2].abs();
        let velocity = unit(velocity).map(|axis| axis * leaf_type.starting_velocity);
        self.leaves[slot] = Some(Leaf {
            place,
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            kind,
            normal,
            pendulum,
            velocity,
            age: 0.0,
            rotation: 0,
        });
        let [low, high] = system.emission_rate;
        self.next_emission = self.next() * (high - low) + low;
    }

    /// `leaf_system_update_physics`.
    fn update(&mut self, system: &H3LeavesAt, wind: [f32; 3], tick: f32) {
        let dt = tick * system.time_scale * tick * 30.0;
        self.alive += dt;
        if dt <= 0.0 {
            return;
        }
        let strength = dot(wind, wind).sqrt() * system.wind_scale;
        let mut force = unit(wind);
        force[2] = -force[2].abs();
        let direction = force;
        let [low, high] = system.wind_range;
        let strength = if low != 0.0 || high != 0.0 { strength.clamp(low, high) } else { strength };
        let force = force.map(|axis| axis * dt * strength * 30.0);
        let force = away_from_pole([0.0, 0.0, 1.0], WIND_EXCLUSION, force);
        let share = (strength / (system.wind_scale * MOST_WIND).max(EPSILON)).clamp(0.0, 1.0);
        let alive = self.alive;
        for (index, slot) in self.leaves.iter_mut().enumerate() {
            let Some(leaf) = slot else {
                continue;
            };
            let kind = &system.types[leaf.kind];
            let facing = unit(leaf.velocity);
            // `velocity_scale` is still last tick's when the tumble reads it.
            let speed = dot(leaf.velocity, leaf.velocity).sqrt();
            leaf.normal = normals_between(leaf.normal, direction, dt);
            let [flat, fall] = kind.friction;
            for axis in 0..2 {
                leaf.velocity[axis] = WIND_FRICTION * force[axis] * flat / dt + (1.0 - flat) * leaf.velocity[axis];
            }
            leaf.velocity[2] = WIND_FRICTION * force[2] * fall / dt + (1.0 - fall) * leaf.velocity[2];
            let flutter = (alive + index as f32 * kind.flutter_phase) * kind.flutter_speed;
            let age_in = (leaf.age / 2.0).clamp(0.0, 1.0);
            let mut pendulum = flutter.sin() * (kind.flutter_scale * 10.0) * ((1.0 - share) * kind.flitteriness) * dt * age_in;
            let tumbling = ((share - 0.01).clamp(0.0, 0.99) / 0.99).max(0.02);
            let tumble = 5.0 * kind.tumble * tumbling * dt * speed.clamp(0.0, 1.0);
            let mut drift = [0.0; 3];
            if pendulum.abs() > SMALLEST_TURN {
                let swing_axis = leaf.pendulum;
                if (dot(swing_axis, leaf.normal) - 1.0).abs() < EPSILON {
                    let left = [0.0, 1.0, 0.0];
                    let at_left = (dot(swing_axis, left) - 1.0).abs() < EPSILON;
                    leaf.pendulum = if at_left { [1.0, 0.0, 0.0] } else { left };
                }
                let upward = leaf.normal[2] >= 0.0;
                let mut arm = leaf.normal;
                arm[2] = arm[2].abs();
                arm = arm.map(|axis| axis * kind.swing);
                if arm[2] < PENDULUM_SLOWDOWN {
                    let slowed = arm[2] / PENDULUM_SLOWDOWN;
                    pendulum *= slowed * slowed;
                }
                let pivot: [f32; 3] = std::array::from_fn(|axis| leaf.place[axis] + arm[axis]);
                let mut swung = turned_about(arm, swing_axis, pendulum);
                if (swung[2] - 1.0).abs() < EPSILON {
                    swung = turned_about(swung, swing_axis, pendulum);
                }
                let hung: [f32; 3] = std::array::from_fn(|axis| pivot[axis] - swung[axis]);
                drift = std::array::from_fn(|axis| hung[axis] - leaf.place[axis]);
                let mut normal = unit(std::array::from_fn(|axis| pivot[axis] - hung[axis]));
                normal[2] *= if upward { 1.0 } else { -1.0 };
                leaf.normal = normal;
                let desired = normals_between(leaf.axes[0], normal, (1.0 - share) * dt);
                let axis = unit(cross(leaf.axes[0], desired));
                if axis != [0.0; 3] {
                    let angle = dot(leaf.axes[0], desired).clamp(-1.0, 1.0).acos();
                    leaf.axes = multiplied(leaf.axes, axis, angle);
                }
            }
            if tumble > 0.0 {
                let across = unit(cross([0.0, 0.0, 1.0], facing));
                let across = if across == [0.0; 3] { [0.0, 1.0, 0.0] } else { across };
                leaf.axes = multiplied(leaf.axes, across, tumble);
            }
            if kind.rotation > 0.0 {
                let step = dt * share * TURNS_PER_SECOND * f32::from(u16::MAX) * kind.rotation;
                leaf.rotation = leaf.rotation.wrapping_add(step as u16);
                let turn = f32::from(leaf.rotation) / f32::from(u16::MAX) * std::f32::consts::TAU;
                leaf.axes[1] = turned_about(leaf.axes[1], leaf.axes[0], turn);
                leaf.axes[2] = turned_about(leaf.axes[2], leaf.axes[0], turn);
            }
            leaf.velocity[2] -= kind.mass;
            for axis in 0..3 {
                leaf.velocity[axis] += force[axis];
                let moved = leaf.place[axis] + leaf.velocity[axis] * dt + drift[axis];
                // `c_word_quantized_position` holds it to the movement bounds.
                leaf.place[axis] = moved.clamp(system.origin[axis] - system.cylinder, system.origin[axis] + system.cylinder);
            }
        }
    }

    /// `leaf_system_cleanup`.
    fn clean_up(&mut self, system: &H3LeavesAt, dt: f32) {
        for slot in &mut self.leaves {
            if let Some(leaf) = slot {
                leaf.age += dt;
                if leaf.age >= system.lifetime {
                    *slot = None;
                }
            }
        }
    }
}

/// What a slot keeps of its leaves from frame to frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct H3LeafField {
    wind: H3Wind,
    time: Option<f32>,
    ticks: f32,
    running: Vec<RunningLeaves>,
}

impl H3LeafField {
    pub(crate) fn advance(&mut self, systems: &[Arc<H3LeavesAt>], time: f32) {
        if self.running.len() != systems.len() || self.time.is_none_or(|last| time < last) {
            self.running = (0..systems.len())
                .map(|at| RunningLeaves {
                    leaves: vec![None; MOST_LEAVES],
                    random: at as u32 * 977 + 1,
                    ..Default::default()
                })
                .collect();
            let settle = systems.iter().map(|system| system.lifetime).fold(0.0, f32::max).min(MOST_SETTLE);
            self.time = Some(time - settle);
        }
        let mut now = self.time.unwrap_or(time);
        while now + TICK <= time {
            now += TICK;
            self.ticks += 1.0;
            let wind = self.wind.step(self.ticks);
            for (system, running) in systems.iter().zip(&mut self.running) {
                running.emit(system, TICK);
                running.update(system, wind, TICK);
                running.clean_up(system, TICK);
            }
        }
        self.time = Some(now);
    }

    /// `leaf_system_render_opaque`'s triangles.
    pub(crate) fn vertices(
        &self,
        index: usize,
        system: &H3LeavesAt,
        lighting: &[[f32; 4]; 10],
    ) -> Vec<H3ParticleVertex> {
        let mut vertices = Vec::new();
        let Some(running) = self.running.get(index) else {
            return vertices;
        };
        let [red, green, blue] = [0, 1, 2].map(|channel| lighting[0][channel] * PROBE_CONSTANT * H3_EXPOSURE);
        for (at, leaf) in running.leaves.iter().enumerate() {
            let Some(leaf) = leaf else {
                continue;
            };
            let kind = &system.types[leaf.kind];
            let mut random = (at as u32).wrapping_mul(2_362_187);
            let mut next = || {
                random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (random >> 16) as f32 / 65_536.0
            };
            let [low, high] = kind.scale;
            let size = if low != 0.0 && high != 0.0 { low + next() * (high - low) } else { 0.02 };
            let places: Vec<[f32; 3]> = VERTICES
                .iter()
                .map(|[y, z]| {
                    let depth = (next() * 2.0 - 1.0) * size * DEPTH_OF_WIDTH;
                    std::array::from_fn(|axis| {
                        leaf.place[axis]
                            + leaf.axes[0][axis] * depth
                            + leaf.axes[1][axis] * y * size
                            + leaf.axes[2][axis] * z * size
                    })
                })
                .collect();
            let [left, right, top, bottom] = kind.bounds;
            for corner in TRIANGLES.iter().flatten() {
                let [y, z] = VERTICES[*corner];
                let uv = [(right - left) * (y + 1.0) * 0.5 + left, (bottom - top) * (z + 1.0) * 0.5 + top];
                vertices.push(H3ParticleVertex {
                    place: places[*corner],
                    sprite: uv,
                    next_sprite: uv,
                    billboard: uv,
                    color: [red, green, blue, 1.0],
                    normal: leaf.axes[0],
                    ..Default::default()
                });
            }
        }
        vertices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system() -> H3LeavesAt {
        H3LeavesAt {
            origin: [0.0, 0.0, 10.0],
            emission_rate: [0.1, 0.1],
            lifetime: 4.0,
            wind_range: [0.0, 0.0],
            wind_scale: 0.75,
            time_scale: 1.0,
            sphere: 1.0,
            cylinder: 20.0,
            types: vec![LeafType {
                frequency: 1.0,
                mass: 0.22,
                scale: [0.03, 0.04],
                flitteriness: 1.0,
                swing: 0.4,
                flutter_scale: 1.0,
                flutter_speed: 0.3,
                flutter_phase: 30.0,
                tumble: 1.5,
                rotation: 1.0,
                starting_velocity: 0.1,
                friction: [0.2, 0.0],
                bounds: [0.0, 1.0, 0.0, 1.0],
            }],
            shader: H3SpriteShader::blank(),
        }
    }

    fn run(system: &Arc<H3LeavesAt>, seconds: f32) -> H3LeafField {
        let mut field = H3LeafField::default();
        field.advance(std::slice::from_ref(system), 0.0);
        field.advance(std::slice::from_ref(system), seconds);
        field
    }

    #[test]
    fn leaves_let_go_near_their_point_and_fall() {
        let system = Arc::new(system());
        let field = run(&system, 0.0);
        let leaves: Vec<_> = field.running[0].leaves.iter().flatten().collect();
        assert!(!leaves.is_empty());
        assert!(leaves.iter().any(|leaf| leaf.place[2] < 10.0), "they have been falling a while");
        assert!(leaves.iter().all(|leaf| leaf.place.iter().all(|axis| axis.is_finite())));
    }

    #[test]
    fn a_leaf_goes_when_its_lifetime_is_up() {
        let mut system = system();
        system.emission_rate = [1000.0, 1000.0];
        let mut running = RunningLeaves { leaves: vec![None; MOST_LEAVES], random: 1, ..Default::default() };
        running.emit(&system, TICK);
        assert_eq!(running.leaves.iter().flatten().count(), 1);
        running.emit(&system, TICK);
        assert_eq!(running.leaves.iter().flatten().count(), 1, "the next waits its turn");
        running.clean_up(&system, 3.9);
        assert_eq!(running.leaves.iter().flatten().count(), 1);
        running.clean_up(&system, 0.2);
        assert_eq!(running.leaves.iter().flatten().count(), 0);
    }

    #[test]
    fn leaves_stay_within_their_cylinder() {
        let mut system = system();
        system.cylinder = 0.5;
        let system = Arc::new(system);
        let field = run(&system, 0.0);
        for leaf in field.running[0].leaves.iter().flatten() {
            assert!((0..3).all(|axis| (leaf.place[axis] - system.origin[axis]).abs() <= 0.5 + 1e-4));
        }
    }

    fn fall_after(friction: f32, ticks: usize) -> f32 {
        let mut system = system();
        system.cylinder = f32::MAX / 2.0;
        system.types[0].friction[1] = friction;
        let mut running = RunningLeaves { leaves: vec![None; MOST_LEAVES], random: 1, ..Default::default() };
        running.emit(&system, TICK);
        (0..ticks).for_each(|_| running.update(&system, [0.0; 3], TICK));
        running.leaves[0].as_ref().unwrap().velocity[2]
    }

    #[test]
    fn only_z_friction_gives_a_leaf_a_terminal_speed() {
        // Both shipped systems have none, so theirs keep gaining to the box's floor.
        assert!(fall_after(0.0, 300) < 2.0 * fall_after(0.0, 150) * 0.99);
        assert!((fall_after(0.5, 300) - fall_after(0.5, 150)).abs() < 1e-3);
    }

    #[test]
    fn a_leaf_is_fourteen_triangles_of_its_sprite() {
        let system = Arc::new(system());
        let field = run(&system, 0.0);
        let count = field.running[0].leaves.iter().flatten().count();
        let lighting = [[1.0; 4]; 10];
        let drawn = field.vertices(0, &system, &lighting);
        assert_eq!(drawn.len(), count * 14 * 3);
        assert!(drawn.iter().all(|vertex| vertex.sprite.iter().all(|uv| (0.0..=1.0).contains(uv))));
    }

    #[test]
    fn halfway_between_normals_lies_between_them() {
        let middle = normals_between([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.5);
        assert!((middle[0] - middle[1]).abs() < 1e-5 && (dot(middle, middle) - 1.0).abs() < 1e-5);
        assert_eq!(normals_between([1.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.25), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn a_turned_orientation_stays_square() {
        let axes = multiplied([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], unit([1.0, 2.0, 3.0]), 0.7);
        assert!((dot(axes[0], axes[0]) - 1.0).abs() < 1e-5);
        assert!(dot(axes[0], axes[1]).abs() < 1e-5);
    }
}
