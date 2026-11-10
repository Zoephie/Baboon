//! Cheap particle emitters: the emitter tag, its spawn maths, the definitions and their markers on an object.

use super::super::super::{RenderModelPreviewMarker, TagReader};
use super::super::rules::particle_game_time;
use super::super::sim::Random;
use super::library::{CheapLibrary, CheapTexels};
use super::{
    ALPHA, CHEAP_PARTICLE_EMITTER, CORRELATE, DEFAULT_POSITION, DEFAULT_VELOCITY, EffectTimes,
    FunctionValues, MarkerFrame, NORMALIZE, PHYSICS, POSITION_LOCAL, ROTATE_UP, VELOCITY_LOCAL,
    attached, clamped_unit, cross, dot, for_each_attached_part, frac, has_flag, object_value,
    part_frame, posed_on, read_part_placement, tag_function_at,
};
use blam_tags::{TagFile, TagFunction, TagStruct};
use std::collections::HashMap;
use std::sync::Arc;

/// A `cheap_particle_emitter` as its tag has it, before `postprocess`.
#[derive(Debug, Clone)]
pub(super) struct CheapEmitterTag {
    pub(super) version: i64,
    pub(super) spawn_rate: f32,
    pub(super) function: Option<Arc<TagFunction>>,
    /// `distance fade start` and `end`.
    pub(super) fade: [f32; 2],
    pub(super) types: [String; 4],
    pub(super) weights: [f32; 4],
    pub(super) lifetime: [f32; 2],
    pub(super) position_scale: f32,
    pub(super) position_flatten: f32,
    pub(super) directionality: [f32; 2],
    pub(super) speed: [f32; 2],
    pub(super) subframe: [f32; 2],
    pub(super) rotation_randomness: f32,
    pub(super) scale_modifier: f32,
    /// `lighting contrast`, `offset`, `min` and `max`.
    pub(super) lighting: [f32; 4],
}

impl CheapEmitterTag {
    fn read(root: TagStruct<'_>) -> Self {
        let real = |name: &str| root.read_real(name).unwrap_or(0.0);
        let bounds = |name: &str| {
            let bounds = root.read_real_bounds(name);
            [bounds.lower, bounds.upper]
        };
        let function = root
            .descend("spawnrate")
            .and_then(|holder| tag_function_at(&holder, "Mapping"))
            .map(Arc::new);
        Self {
            version: root
                .read_int_any("version")
                .map_or(0, |version| version as i64),
            spawn_rate: real("spawn rate"),
            function,
            fade: [real("distance fade start"), real("distance fade end")],
            types: std::array::from_fn(|at| {
                root.read_string_id(&format!("type {at}"))
                    .unwrap_or_default()
            }),
            weights: std::array::from_fn(|at| real(&format!("weight {at}"))),
            lifetime: bounds("lifetime"),
            position_scale: real("position scale"),
            position_flatten: real("position flatten"),
            directionality: bounds("directionality"),
            speed: bounds("speed scale"),
            subframe: bounds("subframe offset"),
            rotation_randomness: real("rotation randomness"),
            scale_modifier: real("particle scale modifier"),
            lighting: [
                "lighting contrast",
                "lighting offset",
                "lighting min",
                "lighting max",
            ]
            .map(real),
        }
    }

    /// `postprocess`'s version upgrade (the 360's 0x832B4430, tool
    /// 0x140DDB990): version 0 fades from 5 to 20 and spawns at a constant
    /// `spawn rate`; up to 1 takes lighting contrast 1, offset 0 and −11 to
    /// 11; up to 2 no rotation randomness and a scale modifier of 1.
    pub(super) fn upgraded(mut self) -> Self {
        if self.version <= 0 {
            self.fade = [5.0, 20.0];
            self.function = constant_function(self.spawn_rate).map(Arc::new);
        }
        if self.version <= 1 {
            self.lighting = [1.0, 0.0, -11.0, 11.0];
        }
        if self.version <= 2 {
            self.rotation_randomness = 0.0;
            self.scale_modifier = 1.0;
        }
        self.version = self.version.max(3);
        self
    }

    /// `postprocess`'s `spawn_params0` to `4`, with the library's type
    /// `names`.
    pub(super) fn spawn_parameters(&self, names: &[String]) -> [[f32; 4]; 5] {
        let modifier = self.scale_modifier.clamp(0.0, 2.0) * 4.0 * 127.0 / 2.0;
        let inverse = |lifetime: f32| (if lifetime <= 0.0001 { 0.0001 } else { lifetime }).recip();
        let [directionality, speed, subframe] = [self.directionality, self.speed, self.subframe];
        let (shortest, longest) = (inverse(self.lifetime[0]), inverse(self.lifetime[1]));
        let indices = self
            .types
            .each_ref()
            .map(|name| names.iter().position(|known| known == name));
        let (thresholds, steps) = type_thresholds(indices, self.weights);
        [
            [
                self.position_scale,
                self.position_flatten,
                self.rotation_randomness.clamp(0.0, 1.0),
                modifier,
            ],
            [
                directionality[0],
                directionality[1] - directionality[0],
                speed[0],
                speed[1] - speed[0],
            ],
            [
                subframe[0],
                subframe[1] - subframe[0],
                shortest,
                longest - shortest,
            ],
            thresholds,
            steps,
        ]
    }
}

/// What tool's `c_function_definition_editor` makes of a constant: function
/// type 1, clamped to `value` at both ends.
pub(super) fn constant_function(value: f32) -> Option<TagFunction> {
    use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};
    let mut function = H2Function::new(FunctionType::Constant);
    function.set_clamp_range(value, value).ok()?;
    TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).ok()
}

/// `postprocess`'s distance fade: the scale and bias `calculate_LOD_scale`
/// (the 360's 0x832B4E68) reads, whole at the nearer bound and nothing at
/// the farther, whichever field holds which.
pub(super) fn distance_fade([start, end]: [f32; 2]) -> [f32; 2] {
    let range = (end - start).abs();
    let scale = -1.0 / if range <= 0.001 { 0.001 } else { range };
    let farther = if start <= end { end } else { start };
    [scale, -farther * scale]
}

pub(super) fn lod_scale([scale, bias]: [f32; 2], distance: f32) -> f32 {
    (distance * scale + bias).clamp(0.0, 1.0)
}

/// `add_emitter`'s distance from the camera (MCC 0x1403E7E66..0x1403E7EB6):
/// a Manhattan one.
pub(super) fn manhattan(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|axis| (a[axis] - b[axis]).abs()).sum()
}

/// `postprocess`'s `spawn_params3` and `4`: each found type's cumulative
/// threshold and its index step from the last found. The weight a found
/// type adds is the one before it, found or not; with no weight at all,
/// type 0 takes all of it.
pub(super) fn type_thresholds(
    mut indices: [Option<usize>; 4],
    mut weights: [f32; 4],
) -> ([f32; 4], [f32; 4]) {
    let mut total = 0.0;
    for (index, weight) in indices.iter().zip(&weights) {
        if index.is_some() {
            total += weight.abs();
        }
    }
    if total == 0.0 {
        indices[0] = Some(0);
        weights[0] = 1.0;
        total = 1.0;
    }
    let first = indices[0].unwrap_or(0);
    let (mut thresholds, mut steps) = ([0.0; 4], [first as f32, 0.0, 0.0, 0.0]);
    let mut previous = first as f32;
    for at in 1..4 {
        match indices[at] {
            None => thresholds[at] = thresholds[at - 1],
            Some(index) => {
                thresholds[at] = thresholds[at - 1] + weights[at - 1].abs() / total;
                steps[at] = (index as f32 - previous) as i32 as f32;
                previous = index as f32;
            }
        }
    }
    (thresholds, steps)
}

/// `setup_emitter_spawn_state` (the 360's 0x82DB3308, MCC sub_1403E8F80):
/// the exposure, raised to `lighting min`, then held at `lighting max`, as
/// `spawn_position.w`.
pub(super) fn packed_lighting([contrast, offset, low, high]: [f32; 4], intensity: f32) -> f32 {
    let mut exposure = intensity.log2() * contrast + offset;
    if exposure < low {
        exposure = low;
    }
    if exposure >= high {
        exposure = high;
    }
    exposure * 256.0 / 22.0 + 128.0
}

/// `PackSByte4` (`packed_vector.hlsl`), as MCC stores a particle's
/// parameters: held to −128..127 and truncated, so a lighting of 128 or more
/// is 127.
pub(super) fn pack_sbyte4(values: [f32; 4]) -> [i8; 4] {
    values.map(|value| value.clamp(-128.0, 127.0) as i32 as i8)
}

/// `cheap_particles_render.hlsl`'s illumination of a packed lighting.
pub(super) fn illumination(lighting: i8) -> f32 {
    (f32::from(lighting) * (22.0 / 256.0) - 11.0).exp2()
}

/// `s_emitter::spawn`'s count: by the frame and the scale, rounded down
/// after `roll`, `real_random_range(0, 1 − 1e-4)`.
pub(super) fn spawn_count(rate: f32, dt: f32, scale: f32, roll: f32) -> usize {
    (rate * dt * scale + roll).floor().max(0.0) as usize
}

/// `generate_random_texture_transform` (the 360's 0x82DB3190, MCC
/// sub_1403E8810): a scale in ±[1, 2)/64 for each of u and v, and an
/// offset, from the generator's draws in the game's order.
pub(super) fn texture_transform(random: &mut Random) -> [f32; 4] {
    let scale = |random: &mut Random| {
        let flip = (1 - 3 * (random.bits() & 1) as i32) as f32;
        (flip + random.next()) * 0.015625
    };
    let (u, v) = (scale(random), scale(random));
    let (w, z) = (random.next(), random.next());
    [u, v, z, w]
}

/// `setup_emitter_spawn_state`'s `c114`, `c115` and `c116`: the random,
/// position and velocity transforms, each rolled anew, but with `correlate
/// position and velocity` the velocity's is the position's, so both read
/// the same texel.
pub(super) fn spawn_transforms(random: &mut Random, correlate: bool) -> [[f32; 4]; 3] {
    let random_transform = texture_transform(random);
    let position = texture_transform(random);
    let velocity = if correlate {
        position
    } else {
        texture_transform(random)
    };
    [random_transform, position, velocity]
}

/// `s_emitter::spawn`'s input to the spawn rate: the game time, or with an
/// input variable the object's function of that name, `alpha` (440) the
/// emitter's scale.
pub(super) fn spawn_input(
    variable: &str,
    values: Option<&FunctionValues>,
    time: f32,
    scale: f32,
) -> f32 {
    match variable {
        "" => particle_game_time(true, time),
        ALPHA => scale,
        name => object_value(values, name),
    }
}

/// `add_emitter`'s `randomly rotate emitter about up vector`: a new up of
/// `forward` across a fresh random direction, every frame. Sic: not
/// normalized after, in either build.
pub(super) fn turned_up(forward: [f32; 3], random: &mut Random) -> [f32; 3] {
    let turn = clamped_unit(std::array::from_fn(|_| random.range(-1.0, 1.0)));
    cross(forward, turn)
}

/// One slot of the ring: `position_and_age`, `velocity_and_delta_age` and
/// the packed `particle_parameters` (type, lighting, rotation and scale).
/// MCC keeps velocity and the age's rate as halves, which changes little.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CheapParticle {
    pub(super) position: [f32; 3],
    /// From 0 to 1 over its life; below 0 at rest, the sign the flag.
    pub(super) age: f32,
    pub(super) velocity: [f32; 3],
    pub(super) delta_age: f32,
    pub(super) parameters: [i8; 4],
}

/// A `cheap_particle_emitter` as `postprocess` leaves it.
#[derive(Debug)]
pub(crate) struct CheapEmitterDefinition {
    pub(super) correlate: bool,
    pub(super) position_local: bool,
    pub(super) velocity_local: bool,
    pub(super) normalize: bool,
    pub(super) rotate_up: bool,
    pub(super) spawn_rate: Arc<TagFunction>,
    /// `spawnrate`'s `Input Variable` and `Range Variable`.
    pub(super) input: String,
    pub(super) range: String,
    /// `distance fade start` and `end`, as scale and bias.
    pub(super) lod: [f32; 2],
    pub(super) parameters: [[f32; 4]; 5],
    pub(super) lighting: [f32; 4],
    pub(super) position: Arc<CheapTexels>,
    pub(super) velocity: Arc<CheapTexels>,
    pub(super) library: Arc<CheapLibrary>,
}

impl CheapEmitterDefinition {
    fn read(
        reader: &TagReader,
        path: &str,
        libraries: &mut HashMap<String, Option<Arc<CheapLibrary>>>,
        maps: &mut HashMap<String, Option<Arc<CheapTexels>>>,
    ) -> Option<Self> {
        let tag = reader.load(path, "cheap_particle_emitter", b"cpem").ok()?;
        let root = tag.root();
        let emitter = CheapEmitterTag::read(root).upgraded();
        let library_path = root
            .read_tag_ref_path("global type library")
            .filter(|path| !path.is_empty())?;
        let library = libraries
            .entry(library_path.to_ascii_lowercase())
            .or_insert_with(|| CheapLibrary::read(reader, &library_path).map(Arc::new))
            .clone()?;
        let mut map = |field: &str, default: &str| {
            let path = root
                .read_tag_ref_path(field)
                .filter(|path| !path.is_empty())
                .unwrap_or_else(|| default.to_owned());
            maps.entry(path.to_ascii_lowercase())
                .or_insert_with(|| CheapTexels::read(reader, &path).map(Arc::new))
                .clone()
        };
        let (position, velocity) = (
            map("position texture", DEFAULT_POSITION)?,
            map("velocity texture", DEFAULT_VELOCITY)?,
        );
        let flagged = |wanted: &str| has_flag(root, "flags", wanted);
        let spawnrate = root.descend("spawnrate");
        let variable = |name: &str| {
            spawnrate
                .and_then(|holder| holder.read_string_id(name))
                .unwrap_or_default()
        };
        Some(Self {
            correlate: flagged(CORRELATE),
            position_local: flagged(POSITION_LOCAL),
            velocity_local: flagged(VELOCITY_LOCAL),
            normalize: flagged(NORMALIZE),
            rotate_up: flagged(ROTATE_UP),
            spawn_rate: emitter.function.clone()?,
            input: variable("Input Variable"),
            range: variable("Range Variable"),
            lod: distance_fade(emitter.fade),
            parameters: emitter.spawn_parameters(&library.names),
            lighting: emitter.lighting,
            position,
            velocity,
            library,
        })
    }

    /// `cheap_particles_spawn.hlsl`'s `spawn_particle` at ring slot `index`,
    /// for an emitter submitted as `at`, with the frame's random, position
    /// and velocity transforms, `lighting` packed, and the shaders' `dt`.
    pub(super) fn spawn_particle(
        &self,
        at: &CheapSubmission,
        transforms: &[[f32; 4]; 3],
        lighting: f32,
        index: usize,
        dt: f32,
    ) -> CheapParticle {
        let [placement, speeds, times, thresholds, steps] = self.parameters;
        let library = &self.library;
        let (up, forward) = (at.up, at.forward);
        let left = cross(up, forward);
        // `mul(random, transform)`: rows up, left, forward and nothing.
        let local = |random: [f32; 4]| -> [f32; 3] {
            std::array::from_fn(|axis| {
                random[0] * up[axis] + random[1] * left[axis] + random[2] * forward[axis]
            })
        };
        let index_f = index as f32;
        let mut position = self.position.generate(index_f, transforms[1]);
        if self.position_local {
            let [x, y, z] = local(position);
            position = [x, y, z, 0.0];
        }
        let position: [f32; 3] = std::array::from_fn(|axis| position[axis] * placement[0]);
        let along = dot(position, forward) * placement[1];
        let mut position: [f32; 3] =
            std::array::from_fn(|axis| position[axis] - forward[axis] * along + at.position[axis]);
        let random = library.random.generate(index_f, transforms[0]);
        let directionality = speeds[0] + speeds[1] * random[0];
        let speed = speeds[2] + speeds[3] * random[1];
        let mut velocity = self.velocity.generate(index_f, transforms[2]);
        if self.velocity_local {
            let [x, y, z] = local(velocity);
            velocity = [x, y, z, 0.0];
        }
        let mut direction: [f32; 3] = std::array::from_fn(|axis| {
            velocity[axis] + (forward[axis] - velocity[axis]) * directionality
        });
        if self.normalize {
            let length = dot(direction, direction).sqrt();
            direction = direction.map(|axis| axis / length);
        }
        let mut velocity = direction.map(|axis| axis * speed);
        let delta_age = random[2] * times[3] + times[2];
        let kind: f32 = (0..4)
            .filter(|&at| random[3] >= thresholds[at])
            .map(|at| steps[at])
            .sum();
        let physics = library.row(kind as usize, PHYSICS);
        let (drag, gravity) = (physics[0], -physics[1]);
        // The subframe offset: an update without collision.
        let subframe = (times[0] + times[1] * random[2]) * dt;
        for axis in 0..3 {
            position[axis] += velocity[axis] * subframe;
        }
        position[2] += gravity * subframe * subframe;
        velocity[2] += gravity * subframe;
        // Sic: a tenth of the subframe, not its share of the life.
        let age = (0.1 * subframe).max(0.0001);
        velocity = velocity.map(|axis| axis - axis * drag * subframe);
        let angle = frac(index_f * 0.2736 + random[3]) * placement[2];
        // `(4 − 4|x|)x`'s sine and cosine, at the modifier's scale.
        let local_dx = [frac(angle + 0.25) * 2.0 - 1.0, frac(angle) * 2.0 - 1.0]
            .map(|x| (placement[3] - placement[3] * x.abs()) * x);
        let parameters = pack_sbyte4([kind, lighting, local_dx[0], local_dx[1]]);
        CheapParticle {
            position,
            age,
            velocity,
            delta_age,
            parameters,
        }
    }
}

/// Where an emitter is submitted this frame, as `add_emitter` keeps it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CheapSubmission {
    pub(super) position: [f32; 3],
    pub(super) forward: [f32; 3],
    pub(super) up: [f32; 3],
    /// The effect's scale by the distance fade.
    pub(super) scale: f32,
}

/// A cheap particle emitter at a marker of the object an effect is attached
/// to.
#[derive(Debug, Clone)]
pub(crate) struct CheapEmitterAt {
    pub(crate) object: u8,
    pub(super) node: usize,
    pub(super) position: [f32; 3],
    pub(super) forward: [f32; 3],
    pub(super) up: [f32; 3],
    pub(super) only_under: Option<(String, String)>,
    pub(super) scale_by: String,
    pub(super) secondary_by: String,
    /// `A scales values` and `B scales values` take `type-specific scale`.
    pub(super) scaled: bool,
    pub(super) b_scaled: bool,
    pub(super) times: Arc<EffectTimes>,
    pub(super) event: usize,
    pub(super) definition: Arc<CheapEmitterDefinition>,
}

/// The cheap particle emitters an object's attached effects run, one at each
/// marker of a part's location.
pub(in crate::app::model_preview) fn read_reach_cheap_emitters(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<CheapEmitterAt> {
    let mut emitters = Vec::new();
    let (mut definitions, mut libraries, mut maps) =
        (HashMap::new(), HashMap::new(), HashMap::new());
    for_each_attached_part(reader, object, |part| {
        if part.group != CHEAP_PARTICLE_EMITTER {
            return;
        }
        let definition = definitions
            .entry(part.path.to_ascii_lowercase())
            .or_insert_with(|| {
                CheapEmitterDefinition::read(reader, &part.path, &mut libraries, &mut maps)
                    .map(Arc::new)
            })
            .clone();
        let Some(definition) = definition else {
            return;
        };
        let (offset, yaw, pitch) = read_part_placement(part.part);
        for marker in &part.effect.locations(markers, &part.marker, &frame) {
            let (place, node) = frame(marker);
            let (place, node) = part.placed(place, node);
            let place = part_frame(&place, offset, yaw, pitch);
            emitters.push(CheapEmitterAt {
                object: owner,
                node,
                position: place.position,
                forward: place.axes[0],
                up: place.axes[2],
                only_under: marker.only_under.clone(),
                scale_by: part.effect.scale_by.clone(),
                secondary_by: part.effect.secondary_by.clone(),
                scaled: part.scaled,
                b_scaled: part.b_scaled,
                times: Arc::clone(&part.times),
                event: part.event,
                definition: Arc::clone(&definition),
            });
        }
    });
    emitters
}

impl CheapEmitterAt {
    /// The functions it reads off its object: its effect's scales and its
    /// spawn rate's inputs.
    pub(crate) fn scale_names(&self) -> [&str; 4] {
        [
            &self.scale_by,
            &self.secondary_by,
            &self.definition.input,
            &self.definition.range,
        ]
    }

    pub(crate) fn library(&self) -> &Arc<CheapLibrary> {
        &self.definition.library
    }

    /// `effects_submit_cheap_attachments` and `add_emitter` at `time`: none
    /// unless its event is in its duration (one of no length never is) and
    /// its effect is on, at `effect_scale` of `type-specific scale` by the
    /// distance fade from `eye`, and none at no scale. Its up is turned in
    /// `add_emitter` once it has room.
    pub(super) fn submission(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        time: f32,
        onset: f32,
    ) -> Option<CheapSubmission> {
        self.times.age(self.event, time - onset)?;
        if !self
            .times
            .duration(self.event)
            .is_some_and(|duration| duration > 0.0)
        {
            return None;
        }
        // `attachments_update` runs the effect while its function is on.
        let effect_a = object_value(values, &self.scale_by);
        if effect_a <= 0.0 {
            return None;
        }
        let effect_b = object_value(values, &self.secondary_by);
        let scale = (if self.scaled { effect_a } else { 1.0 })
            * (if self.b_scaled { effect_b } else { 1.0 });
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        let (_, up) = posed_on(self.node, self.position, self.up, bones);
        let scale = scale * lod_scale(self.definition.lod, manhattan(position, eye));
        (scale > 0.0).then_some(CheapSubmission {
            position,
            forward,
            up,
            scale,
        })
    }
}

attached!(CheapEmitterAt);
