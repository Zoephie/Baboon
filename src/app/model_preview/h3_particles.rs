//! The particle systems on an attached effect, for Halo 2, Halo 3, ODST, Reach
//! and Halo 4. The game updates most of these on the card, but nothing in the
//! update needs it, so they run here, a fixed step at a time, with their
//! movement controllers, and draw as sprites, particle models or distortion.

use std::sync::Arc;

use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode, ParameterSource, ResolvedRenderMethod, ResolvedValue};
use blam_tags::{TagFieldData, TagFile, TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::h3_effects::adopt_event_function;
use super::ce::attachments::elements;
use super::ce::lights::CeFlareView;
use super::h2::functions::H2FunctionValues;
use super::h2_passes::tag_function_at;
use super::h3_effects::H3EffectOnsets;
use super::h3_effects::{H3EffectTimes, H3ObjectEffect, h3_object_effects, part_runs_by};
use super::h3_strips::{H3StripAt, read_carried_strips};
use super::h3_shaders::{H3Blend, H3Sampler, read_h3_definition};
use super::h2_passes::H2Image;
use super::materials::{H3Bitmap, TextureImage, decode_h2_image, read_h3_bitmap, read_resolved_map, resolve_embedded_method};
use super::renderer::{H3_EXPOSURE, reach_alt_exposure, reach_exposure, reach_lightmap_tint, reach_particle_light};
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::game::GameId;
use crate::core::source::TagSource;

/// Seconds run before the first frame, so a held pose has its particles.
pub(super) const PRE_ROLL: f32 = 6.0;
/// `s_sprite_frame_list`'s room.
const MOST_FRAMES: usize = 15;

/// `global_gravity_get`, in world units a second squared.
const GRAVITY: f32 = 4.171_259_4;
/// The most an emitter keeps alive, whatever its tag allows.
const MOST_PARTICLES: usize = 512;
const SEEDS: usize = 8;
const SYSTEM_SEEDS: usize = 4;

/// `game_state_type_enum`, by option name. Those with a `{` carry another
/// name after it, which a tag's own schema may spell the first way only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H3ParticleInput {
    Age,
    SystemAge,
    Random,
    SystemRandom,
    Seed(usize),
    SystemSeed(usize),
    EmissionTime,
    Lod,
    GameTime,
    EffectA,
    EffectB,
    /// The particle's own rotation, as Halo 2 reads it. Halo 3 and ODST's
    /// `update_states_internal` write 0 here instead (MCC halo3.dll
    /// 0x180221B18, ODST 0x1406D46B0), which [`SeedAllocator::halo3`] keeps.
    Rotation,
    LocationSeed,
    /// Reach's `particle velocity`, 20: `get_state_value` reads
    /// `length(m_velocity) * 0.1`, the speed in the particle's own frame. It
    /// is an evolving state only, so what is read at a spawn reads 0.
    Velocity,
    /// What reads 0: an unknown name, the invalid state, or one no game writes
    /// (`distance from emitter`, 17: neither state list sets it, and the card's
    /// upload mask 0xFE117B02 leaves it out).
    Other,
}

impl H3ParticleInput {
    fn named(name: &str) -> Self {
        match name.split('{').next().unwrap_or_default() {
            "particle age" => Self::Age,
            "system age" | "emitter age" => Self::SystemAge,
            "particle random" => Self::Random,
            "system random" | "emitter random" => Self::SystemRandom,
            "particle correlation 1" | "particle random 1" => Self::Seed(0),
            "particle correlation 2" | "particle random 2" => Self::Seed(1),
            "particle correlation 3" | "particle random 3" => Self::Seed(2),
            "particle correlation 4" | "particle random 4" => Self::Seed(3),
            "system correlation 1" | "emitter correlation 1" | "emitter random 1" => Self::SystemSeed(0),
            "system correlation 2" | "emitter correlation 2" | "emitter random 2" => Self::SystemSeed(1),
            "particle emission time" | "emitter time" | "particle emit time" => Self::EmissionTime,
            "location lod" | "system lod" => Self::Lod,
            "game time" => Self::GameTime,
            "effect a scale" => Self::EffectA,
            "effect b scale" => Self::EffectB,
            "particle rotation" | "physics rotation" => Self::Rotation,
            "location random" => Self::LocationSeed,
            "particle velocity" => Self::Velocity,
            // Reach's `update_states_internal` (the 360's 0x82ED14A0, MCC
            // sub_1405CD4D0) never writes 17, and 18 and 19 only for a boat hull
            // or jetwash emitter, which the preview does not run; the card's
            // upload mask 0xFC217B02 leaves all three out, so the game reads
            // whatever the slot held. 0 is the preview's choice.
            "distance from emitter" | "particle simulation a" | "particle simulation b" => Self::Other,
            _ => Self::Other,
        }
    }

    fn of(self, state: &H3ParticleState) -> f32 {
        match self {
            Self::Age => state.age,
            Self::SystemAge => state.system_age,
            Self::Seed(seed) => state.seeds[seed % SEEDS],
            Self::SystemSeed(seed) => state.system_seeds[seed % SYSTEM_SEEDS],
            Self::EmissionTime => state.emission_time,
            Self::Lod => state.lod,
            Self::GameTime => state.game_time,
            Self::EffectA => state.effect_a,
            Self::EffectB => state.effect_b,
            Self::Rotation => state.rotation,
            Self::LocationSeed => state.location_seed,
            Self::Velocity => state.velocity,
            Self::Random | Self::SystemRandom | Self::Other => 0.0,
        }
    }

    /// `1 <<` its index in Reach's `particle_state_list.hlsl`, as tool's masks
    /// of used states hold it; none for what the preview reads as 0.
    fn state_bit(self) -> u32 {
        let index = match self {
            Self::Age => 0,
            Self::SystemAge => 1,
            Self::Random => 2,
            Self::SystemRandom => 3,
            // Seeds 1 to 4 are states 4 to 7, and 5 to 8 are 22 to 25.
            Self::Seed(seed) if seed < 4 => 4 + seed,
            Self::Seed(seed) => 22 + seed - 4,
            // System seeds 1 and 2 are states 8 and 9, and 3 and 4 are 26 and 27.
            Self::SystemSeed(seed) if seed < 2 => 8 + seed,
            Self::SystemSeed(seed) => 26 + seed - 2,
            Self::EmissionTime => 10,
            Self::Lod => 11,
            Self::GameTime => 12,
            Self::EffectA => 13,
            Self::EffectB => 14,
            Self::Rotation => 15,
            Self::LocationSeed => 16,
            Self::Velocity => 20,
            Self::Other => return 0,
        };
        1 << index
    }
}

/// `c_particle_state_list`.
#[derive(Debug, Clone, Copy, Default)]
struct H3ParticleState {
    age: f32,
    system_age: f32,
    seeds: [f32; SEEDS],
    system_seeds: [f32; SYSTEM_SEEDS],
    emission_time: f32,
    game_time: f32,
    effect_a: f32,
    effect_b: f32,
    rotation: f32,
    location_seed: f32,
    lod: f32,
    /// Reach's `particle velocity`.
    velocity: f32,
}

/// `c_particle_system_definition::calc_lod_amount`: distances in, out, and how
/// far each fades over. None for `lod always 1.0`.
fn lod_amount(lod: Option<[f32; 4]>, distance: f32) -> f32 {
    let Some([near, far, near_feather, far_feather]) = lod else {
        return 1.0;
    };
    if distance <= near || distance >= far {
        0.0
    } else if distance < near + near_feather {
        (distance - near) / near_feather
    } else if distance > far - far_feather {
        (far - distance) / far_feather
    } else {
        1.0
    }
}

/// `c_editable_property_base`, with what `interpolate` runs between.
#[derive(Debug, Clone)]
struct H3ParticleProperty {
    input: H3ParticleInput,
    range: H3ParticleInput,
    adds: Option<bool>,
    modifier: H3ParticleInput,
    function: Arc<TagFunction>,
    ends: [[f32; 3]; 2],
    /// What the game multiplies a value by on reading it: Halo 2's rates are
    /// a tick's, and its turns come in degrees.
    scale: f32,
}

/// `allocate_random_seed`'s two counters, handing seeds out in property order,
/// and what the game's state list leaves of the inputs it does not seed.
#[derive(Default)]
struct SeedAllocator {
    particle: usize,
    system: usize,
    /// Halo 3 and ODST: `particle rotation` reads the 0 their state list writes.
    rotation_unread: bool,
    /// Reach's: seeds go by which states are taken, not by counting.
    reach: Option<ReachSeeds>,
}

/// Reach tool's `allocate_random_seed` (0x14106CFA0): the highest seed not
/// yet taken, and once none is left, the next of a table of reuses.
#[derive(Debug, Clone, Copy)]
struct ReachSeeds {
    /// The states taken, by [`H3ParticleInput::state_bit`].
    used: u32,
    /// `dword_1428F5DE4`, shared by both tables. The shipped tool starts it at
    /// 4 and never resets it, so it counts every run-out of every tag a run
    /// builds; the preview starts it at 4 for each allocation instead.
    reuse: usize,
    /// A movement controller's (`fully_index_parameters_and_postprocess`, tool
    /// 0x140DD92D0): each parameter's own states are marked taken just before
    /// its randoms are allocated, rather than all of them first.
    marks_each: bool,
}

/// `particle random`'s seeds, highest state first: 25 to 22, then 7 to 4.
const REACH_PARTICLE_SEEDS: [H3ParticleInput; 8] = {
    use H3ParticleInput::Seed;
    [Seed(7), Seed(6), Seed(5), Seed(4), Seed(3), Seed(2), Seed(1), Seed(0)]
};
/// The particle reuse table (tool 0x141FFF3C0): states 22, 7, 23, 6, 24, 5, 25, 4.
const REACH_PARTICLE_REUSE: [H3ParticleInput; 8] = {
    use H3ParticleInput::Seed;
    [Seed(4), Seed(3), Seed(5), Seed(2), Seed(6), Seed(1), Seed(7), Seed(0)]
};
/// `system random`'s seeds, highest state first, which is also the system
/// reuse table (tool 0x141FFF3E0): states 27, 26, 9, 8.
const REACH_SYSTEM_SEEDS: [H3ParticleInput; 4] = {
    use H3ParticleInput::SystemSeed;
    [SystemSeed(3), SystemSeed(2), SystemSeed(1), SystemSeed(0)]
};

/// What tool's postprocesses keep of a mask of used states before
/// allocating: all but the two pseudo states, `particle random` and `system
/// random`.
const REACH_ALLOCATED_STATES: u32 = 0x0FFF_FFF3;

impl ReachSeeds {
    fn take(&mut self, seeds: &[H3ParticleInput], reuse: &[H3ParticleInput]) -> H3ParticleInput {
        let free = seeds.iter().copied().find(|seed| self.used & seed.state_bit() == 0);
        let seed = free.unwrap_or_else(|| {
            let seed = reuse[self.reuse % reuse.len()];
            self.reuse += 1;
            seed
        });
        self.used |= seed.state_bit();
        seed
    }
}

impl SeedAllocator {
    /// For a Halo 3 or ODST tag's properties.
    fn halo3() -> Self {
        Self { rotation_unread: true, ..Self::default() }
    }

    /// For a Reach tag's properties, with `used` the states its definition
    /// reads already. Reach writes 0 for `particle rotation`, as Halo 3 does.
    fn reach(used: u32) -> Self {
        let seeds = ReachSeeds { used, reuse: 4, marks_each: false };
        Self { rotation_unread: true, reach: Some(seeds), ..Self::default() }
    }

    /// For one of a Reach movement's controllers: a mask of its own.
    fn reach_controller() -> Self {
        let mut seeds = Self::reach(0);
        seeds.reach.iter_mut().for_each(|reach| reach.marks_each = true);
        seeds
    }

    /// The states Reach's allocations have taken so far; none for Halo 3's.
    fn used(&self) -> u32 {
        self.reach.map_or(0, |reach| reach.used)
    }

    fn allocate(&mut self, input: &mut H3ParticleInput) {
        match (*input, self.reach.as_mut()) {
            (H3ParticleInput::Rotation, _) if self.rotation_unread => *input = H3ParticleInput::Other,
            (H3ParticleInput::Random, Some(reach)) => *input = reach.take(&REACH_PARTICLE_SEEDS, &REACH_PARTICLE_REUSE),
            (H3ParticleInput::SystemRandom, Some(reach)) => *input = reach.take(&REACH_SYSTEM_SEEDS, &REACH_SYSTEM_SEEDS),
            (H3ParticleInput::Random, None) => {
                *input = H3ParticleInput::Seed(self.particle % SEEDS);
                self.particle += 1;
            }
            (H3ParticleInput::SystemRandom, None) => {
                *input = H3ParticleInput::SystemSeed(self.system % SYSTEM_SEEDS);
                self.system += 1;
            }
            _ => {}
        }
    }
}

impl H3ParticleProperty {
    /// A property as its tag gives it, its inputs allocated as read: what
    /// Halo 3's and Halo 2's postprocesses do, and Reach's controllers.
    fn read(property: TagStruct<'_>, seeds: &mut SeedAllocator) -> Option<Self> {
        let mut read = Self::unallocated(property)?;
        read.allocate(seeds);
        Some(read)
    }

    /// A property with its randoms not yet given seeds, for Reach, which
    /// gathers what a definition's properties read before allocating any.
    fn unallocated(property: TagStruct<'_>) -> Option<Self> {
        let input = |field: &str| H3ParticleInput::named(&property.read_enum_name(field).unwrap_or_default());
        let adds = match property.read_enum_name("Output Modifier").as_deref() {
            Some("Plus") => Some(true),
            Some("Times") => Some(false),
            _ => None,
        };
        let function = tag_function_at(&property, "Mapping")?;
        Some(Self {
            input: input("Input Variable"),
            range: input("Range Variable"),
            adds,
            modifier: input("Output Modifier Input"),
            function: Arc::new(function),
            ends: read_ends(property),
            scale: 1.0,
        })
    }

    /// `c_editable_property_base`'s seeds: none for a constant; else its
    /// input, its range (Reach's only when the function is ranged), and its
    /// modifier when it has one.
    fn allocate(&mut self, seeds: &mut SeedAllocator) {
        if let Some(reach) = seeds.reach.as_mut().filter(|reach| reach.marks_each) {
            reach.used |= self.used_states();
        }
        if self.function.as_constant().is_some() {
            return;
        }
        seeds.allocate(&mut self.input);
        if seeds.reach.is_none() || self.function.is_ranged() {
            seeds.allocate(&mut self.range);
        }
        if self.adds.is_some() {
            seeds.allocate(&mut self.modifier);
        }
    }

    /// Reach tool's `used_states`: its input's state, its range's when the
    /// function is ranged, and its modifier's when it has one.
    fn used_states(&self) -> u32 {
        let range = if self.function.is_ranged() { self.range.state_bit() } else { 0 };
        let modifier = if self.adds.is_some() { self.modifier.state_bit() } else { 0 };
        self.input.state_bit() | range | modifier
    }

    /// The same thing, for what Halo 2 has no property for.
    fn fixed(value: f32) -> Option<Self> {
        use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};
        let mut function = H2Function::new(FunctionType::Identity);
        function.set_clamp_range(value, value).ok()?;
        Some(Self {
            input: H3ParticleInput::Other,
            range: H3ParticleInput::Other,
            adds: None,
            modifier: H3ParticleInput::Other,
            function: Arc::new(TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).ok()?),
            ends: [[0.0; 3]; 2],
            scale: 1.0,
        })
    }

    fn fixed_vector(vector: [f32; 3]) -> Option<Self> {
        Some(Self { ends: [vector; 2], ..Self::fixed(1.0)? })
    }

    fn scaled(self, scale: f32) -> Self {
        Self { scale, ..self }
    }

    fn named(holder: TagStruct<'_>, name: &str, seeds: &mut SeedAllocator) -> Option<Self> {
        Self::read(holder.descend(name)?, seeds)
    }

    fn scalar(&self, state: &H3ParticleState) -> f32 {
        self.unscaled(state) * self.scale
    }

    fn unscaled(&self, state: &H3ParticleState) -> f32 {
        let value = self.function.evaluate(self.input.of(state), self.range.of(state));
        match self.adds {
            Some(true) => value + self.modifier.of(state),
            Some(false) => value * self.modifier.of(state),
            None => value,
        }
    }

    fn color(&self, state: &H3ParticleState) -> [f32; 3] {
        let color = self.function.evaluate_color(self.input.of(state), self.range.of(state));
        [color.red, color.green, color.blue]
    }

    fn vector(&self, state: &H3ParticleState) -> [f32; 3] {
        let t = self.scalar(state);
        std::array::from_fn(|axis| self.ends[0][axis] + (self.ends[1][axis] - self.ends[0][axis]) * t)
    }

    /// `get_interpolated_value` for what a direction is: around, not across.
    fn around(&self, state: &H3ParticleState) -> [f32; 3] {
        slerped(self.ends, self.scalar(state))
    }

    /// `c_editable_property_base::get_max`, sampled.
    fn most(&self) -> f32 {
        let sample = |at: usize| {
            let t = at as f32 / 16.0;
            [self.function.evaluate(t, 0.0), self.function.evaluate(t, 1.0)]
        };
        (0..=16).flat_map(sample).fold(f32::MIN, f32::max)
    }
}

/// Unnormalized, as the game's is, and with no special case but what keeps it
/// from dividing by nothing.
pub(super) fn slerped([from, to]: [[f32; 3]; 2], t: f32) -> [f32; 3] {
    let lengths = dot(from, from).sqrt() * dot(to, to).sqrt();
    let lengths = if lengths == 0.0 { 1.0 } else { lengths };
    let mut cos = dot(from, to) / lengths;
    if cos >= 0.9999 {
        cos *= 0.9999;
    }
    let sin = (1.0 - cos * cos).sqrt();
    let angle = cos.acos();
    let (near, far) = (((1.0 - t) * angle).sin() / sin, (angle * t).sin() / sin);
    std::array::from_fn(|axis| from[axis] * near + to[axis] * far)
}

/// `c_particle_system_definition::postprocess`' LOD: in, out, feather in and
/// feather out. Only what ignores the camera mode gets the near pair, and that
/// from how fast it emits. Out and its feather default to 20 and 5 in both
/// MCC tools (Halo 3 0x1407E4A30, ODST 0x140828DE0); the tag's "defaults to
/// 30.0 / 10.0" is Halo 2's text, carried over.
fn baked_lod(raw: [f32; 4], any_camera: bool, rates: impl Iterator<Item = f32>) -> [f32; 4] {
    let empty = |value: f32| value.abs() < 0.0001;
    let [mut near, mut far, mut feather_in, mut feather_out] = raw;
    if empty(far) {
        far = 20.0;
    }
    if empty(feather_out) {
        feather_out = 5.0;
    }
    if any_camera {
        let busy = ((rates.map(|rate| rate / 30.0).sum::<f32>() - 2.0) / 3.0).clamp(0.0, 1.0);
        if empty(near) {
            near = busy * -0.1;
        }
        if empty(feather_in) {
            feather_in = (1.0 - busy) + busy * 3.0;
        }
    }
    // Its inverses, which a feather of nothing leaves at 1.
    let feather = |value: f32| if empty(value) { 1.0 } else { value };
    [near, far, feather(feather_in), feather(feather_out)]
}

/// What Reach tool's postprocesses take as nothing.
const POSTPROCESS_EPSILON: f32 = 0.0001;

/// Reach tool's `c_particle_system_definition::postprocess_block_proc`
/// (0x140AE3220): out and its feather default to 20 and 5, and nothing more,
/// whatever the camera mode. The feathers stay as authored, nothing included:
/// `calc_lod_amount` (the 360's 0x82EE59C8) tests `in + feather in` with the
/// raw one, and only multiplies by the inverse it bakes.
fn reach_baked_lod(raw: [f32; 4]) -> [f32; 4] {
    let [near, far, feather_in, feather_out] = raw;
    let or = |value: f32, default: f32| if value.abs() < POSTPROCESS_EPSILON { default } else { value };
    [near, or(far, 20.0), feather_in, or(feather_out, 5.0)]
}

/// `1 / near fade range`, as it comes out on the card. Reach's postprocess
/// takes it of `max(range, ε)`, so a range of nothing fades over a hair.
fn near_fade_range(reach: bool, range: f32) -> f32 {
    if reach { range.max(POSTPROCESS_EPSILON).recip() } else { range.recip() }
}

/// Where MCC's Reach particle clock comes round, in seconds.
const REACH_GAME_TIME_WRAP: f32 = 7200.0;

/// The `game time` state at `time` seconds in. Halo 3's is the clock's
/// fraction of a second. Reach's particles read `g_render_time+0x18`
/// (`s_render_time::update`, the 360's 0x827477A8, MCC sub_14084E170), the
/// frames' seconds summed whole; MCC converts its counter as a signed int, so
/// it runs to 7200 s and comes round from -7200 s.
pub(super) fn particle_game_time(reach: bool, time: f32) -> f32 {
    if reach {
        (time + REACH_GAME_TIME_WRAP).rem_euclid(2.0 * REACH_GAME_TIME_WRAP) - REACH_GAME_TIME_WRAP
    } else {
        time.rem_euclid(1.0)
    }
}

/// Halo 2 tool's particle system postprocess (`sub_80D2A0`): out and its
/// feather default to 30 and 10, and only a system shown in any camera mode
/// gets the near pair, from how fast it emits. A feather left at nothing stays
/// so: `calc_lod_amount` (tag_test `sub_5D7990`) tests `near + feather` with
/// it, so there is no fade, and only multiplies by the inverse it bakes.
fn h2_baked_lod(raw: [f32; 4], any_camera: bool, rates: impl Iterator<Item = f32>) -> [f32; 4] {
    let empty = |value: f32| value.abs() < 0.0001;
    let [mut near, mut far, mut feather_in, mut feather_out] = raw;
    if empty(far) {
        far = 30.0;
    }
    if empty(feather_out) {
        feather_out = 10.0;
    }
    if any_camera {
        let busy = ((rates.map(|rate| rate / 30.0).sum::<f32>() - 2.0) / 3.0).clamp(0.0, 1.0);
        if empty(near) {
            near = busy * -0.1;
        }
        if empty(feather_in) {
            feather_in = (1.0 - busy) + busy * 3.0;
        }
    }
    [near, far, feather_in, feather_out]
}

/// `frame_advance`: what has landed stops once it is this slow.
fn comes_to_rest(landed: bool, velocity: [f32; 3]) -> bool {
    landed && dot(velocity, velocity) <= SPEED_AT_REST * SPEED_AT_REST
}

/// A point, a vector, or a radius and angles, whichever a property has.
pub(super) fn read_ends(property: TagStruct<'_>) -> [[f32; 3]; 2] {
    let end = |name: &str| -> [f32; 3] {
        if let Some(angled) = property.descend(name) {
            let angles = angled.read_euler2d("Euler angles");
            let radius = angled.read_real("Radius").unwrap_or(0.0);
            let radius = if radius == 0.0 { 1.0 } else { radius };
            let (sin_yaw, cos_yaw) = angles.yaw.sin_cos();
            let (sin_pitch, cos_pitch) = angles.pitch.sin_cos();
            return [cos_yaw * cos_pitch, sin_yaw * cos_pitch, sin_pitch].map(|axis| axis * radius);
        }
        match property.field(name).and_then(|field| field.value()) {
            Some(TagFieldData::RealPoint3d(point)) => [point.x, point.y, point.z],
            Some(TagFieldData::RealVector3d(vector)) => [vector.i, vector.j, vector.k],
            Some(TagFieldData::RealPoint2d(point)) => [point.x, point.y, 0.0],
            Some(TagFieldData::RealVector2d(vector)) => [vector.i, vector.j, 0.0],
            _ => [0.0; 3],
        }
    };
    [end("Starting interpolant"), end("Ending interpolant")]
}

/// `frame_advance`: 0.0625 of speed squared stops what has landed.
const SPEED_AT_REST: f32 = 0.25;

/// A `particle_physics` entry, in the order `update_particles` runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H3ParticleController {
    Physics,
    Collider,
    Flock,
}

/// Where an emitter's particles are updated, as tool's `effect_postprocess`
/// decides it (`c_particle_emitter_definition::decide_whether_cpu_or_gpu`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H3ParticleUpdate {
    /// `c_particle_emitter::frame_advance`, with every movement.
    Cpu,
    /// `particle_update.hlsl`: no collision, and physics once per emitter.
    Gpu,
}

/// `decide_whether_cpu_or_gpu`, and whether a particle that comes to rest
/// moves on to the card (`becomes gpu when at rest`). `force cpu updating`
/// wins, then `force gpu updating`, and either rules out the move; otherwise an
/// emitter goes on the card when its particle lists no attachments (H3's
/// particle flag 0x20, ODST's 0x40) and its own movement asks for nothing past
/// `physics` (movement flag 0x100), and its resting particles move there when
/// the particle lists no attachments. The emitter's and the system's own "can
/// be gpu" bits are set by every postprocess, so always hold.
fn particle_update(
    system_flagged: impl Fn(&str) -> bool,
    any_attachments: bool,
    movement_flagged: impl Fn(&str) -> bool,
) -> (H3ParticleUpdate, bool) {
    let plain_movement = !["collide with", "swarm", "wind"].iter().any(|wanted| movement_flagged(wanted));
    if system_flagged("force cpu updating") {
        (H3ParticleUpdate::Cpu, false)
    } else if system_flagged("force gpu updating") {
        (H3ParticleUpdate::Gpu, false)
    } else if !any_attachments && plain_movement {
        (H3ParticleUpdate::Gpu, true)
    } else {
        (H3ParticleUpdate::Cpu, !any_attachments)
    }
}

/// The movements a Halo 3 or ODST emitter runs: tool's `postprocess_internal`
/// keeps one for each kind its own flags name (adding any the tag lacks) and
/// sorts them by `sub_140A846A0`'s table: physics, wind, swarm, collider. The
/// preview has no wind.
fn flagged_controllers(flagged: impl Fn(&str) -> bool) -> Vec<H3ParticleController> {
    [
        (flagged("physics"), H3ParticleController::Physics),
        (flagged("swarm"), H3ParticleController::Flock),
        (flagged("collide with"), H3ParticleController::Collider),
    ]
    .into_iter()
    .filter_map(|(on, controller)| on.then_some(controller))
    .collect()
}

/// One parameter of a flagged movement kind as tool leaves it
/// (`sub_140A84700`): the emitter's own, else its template's and on up the
/// template chain (`sub_140A848F0`), else a constant 1.
fn movement_parameter(
    source: &TagSource,
    movement: TagStruct<'_>,
    kind: &str,
    id: i128,
    seeds: &mut SeedAllocator,
) -> Option<H3ParticleProperty> {
    movement_parameter_or(source, movement, kind, id, 1.0, seeds)
}

/// [`movement_parameter`], with the constant tool fills in where it differs.
fn movement_parameter_or(
    source: &TagSource,
    movement: TagStruct<'_>,
    kind: &str,
    id: i128,
    default: f32,
    seeds: &mut SeedAllocator,
) -> Option<H3ParticleProperty> {
    chained_parameter(source, movement, kind, id, seeds, MOST_TEMPLATES).or_else(|| H3ParticleProperty::fixed(default))
}

/// How deep a template chain is followed, against one that names itself.
const MOST_TEMPLATES: usize = 8;

fn chained_parameter(
    source: &TagSource,
    holder: TagStruct<'_>,
    kind: &str,
    id: i128,
    seeds: &mut SeedAllocator,
    depth: usize,
) -> Option<H3ParticleProperty> {
    let of_kind = |entry: &TagStruct<'_>| entry.read_enum_name("type").as_deref() == Some(kind);
    let entry = elements(holder, "movements").into_iter().find(of_kind);
    let parameters = entry.map(|entry| elements(entry, "parameters")).unwrap_or_default();
    if let Some(found) = parameters.into_iter().find(|parameter| parameter.read_int_any("parameter id") == Some(id)) {
        return H3ParticleProperty::named(found, "property", seeds);
    }
    let path = holder.read_tag_ref_path("template").filter(|path| !path.is_empty() && depth > 0)?;
    let template = load_referenced_tag_from_source(source, &path, "particle_physics", b"pmov").ok()?;
    chained_parameter(source, template.root(), kind, id, seeds, depth - 1)
}

/// A Halo 2 `particle_physics`' movements, its own or its template's.
fn h2_controllers(source: &TagSource, movement: TagStruct<'_>) -> Vec<H3ParticleController> {
    let template = movement
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| load_referenced_tag_from_source(source, &path, "particle_physics", b"pmov").ok());
    let own = elements(movement, "movements");
    let movements = if own.is_empty() { template.as_ref().map_or(Vec::new(), |tag| elements(tag.root(), "movements")) } else { own };
    movements
        .into_iter()
        .filter_map(|entry| match entry.read_enum_name("type").as_deref() {
            Some("physics") => Some(H3ParticleController::Physics),
            Some("collider") => Some(H3ParticleController::Collider),
            _ => None,
        })
        .collect()
}

/// A particle's physical rotation at birth, in turns: random with `random
/// starting rotation`, else Halo 4's three-quarter turn for `bitmap authored
/// vertically` (`c_particle_definition::initialize_particle`, the 360's
/// 0x82AFA548; `particles.md` §1.3). Halo 4's shaders never read that flag,
/// so this is its only quarter turn; Halo 3 and Reach turn at the draw instead.
fn starting_rotation(random_rotation: bool, halo4_vertical: bool, random: impl FnOnce() -> f32) -> f32 {
    if random_rotation {
        random()
    } else if halo4_vertical {
        0.75
    } else {
        0.0
    }
}

/// `initialize_particle`'s nudge to a speed of none, which what faces its
/// velocity needs one to have. Halo 3 gives it to everything, ODST and Reach
/// (the 360's 0x82EE3C98) only to those.
fn still_particles_move(game: Option<GameId>, billboard: H3Billboard) -> bool {
    use H3Billboard::{ScreenParallel, ScreenPerpendicular, VelocityHorizontal};
    let only_facing = matches!(game, Some(GameId::Halo3Odst | GameId::HaloReach));
    !only_facing || matches!(billboard, ScreenParallel | ScreenPerpendicular | VelocityHorizontal)
}

/// `c_particle_definition`'s billboard styles, by option name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H3Billboard {
    ScreenFacing,
    CameraFacing,
    ScreenParallel,
    ScreenPerpendicular,
    ScreenVertical,
    ScreenHorizontal,
    LocalVertical,
    LocalHorizontal,
    World,
    VelocityHorizontal,
}

impl H3Billboard {
    fn named(name: &str) -> Self {
        match name {
            "camera facing" => Self::CameraFacing,
            "parallel to direction" => Self::ScreenParallel,
            "perpendicular to direction" => Self::ScreenPerpendicular,
            "vertical" => Self::ScreenVertical,
            "horizontal" => Self::ScreenHorizontal,
            "local vertical" => Self::LocalVertical,
            "local horizontal" => Self::LocalHorizontal,
            "world (particle models)" => Self::World,
            "velocity horizontal (particle models)" => Self::VelocityHorizontal,
            _ => Self::ScreenFacing,
        }
    }
}

/// How `sample_diffuse` reads the maps, by the `albedo` option's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum H3ParticleAlbedo {
    Diffuse = 0,
    DiffuseBillboardAlpha = 1,
    DiffuseSpriteAlpha = 2,
    Palette = 3,
    PaletteBillboardAlpha = 4,
    PaletteSpriteAlpha = 5,
    /// ODST's: tinted, less so the brighter the texel.
    DiffuseModulated = 6,
    /// ODST's: a ramp lit by the particle's palette value.
    PaletteGlow = 7,
    /// ODST's: two noises' difference, looked up in the palette.
    PalettePlasma = 8,
    /// Halo 2's `effect_shader_*_tint`: the texel's fourth power toward itself
    /// by the tint, so that bright stays white.
    H2Tint = 9,
    /// Halo 2's `effect_shader_alpha_multiply_add`, whose colour and alpha part.
    H2MultiplyAdd = 10,
    /// Halo 2's `effect_shader_alphablend_distortion`: the frame as it stands,
    /// read where its `v8u8` bump map pushes, times the particle's colour.
    H2Distortion = 11,
    /// Reach's `palettized_2d_plasma`: the plasma's noises with no alpha
    /// term, looked up at the depth fade rather than the palette value.
    Palette2dPlasma = 12,
}

/// What ODST's three new albedo options read.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct H3AlbedoConstants {
    pub(crate) tint: [f32; 3],
    pub(crate) modulation: f32,
    pub(crate) alpha_modulation: f32,
    pub(crate) base_xform: [f32; 4],
    pub(crate) second_xform: [f32; 4],
}

impl H3ParticleAlbedo {
    fn named(name: &str) -> Self {
        match name {
            "diffuse_plus_billboard_alpha" => Self::DiffuseBillboardAlpha,
            "diffuse_plus_sprite_alpha" => Self::DiffuseSpriteAlpha,
            "palettized" => Self::Palette,
            "palettized_plus_billboard_alpha" => Self::PaletteBillboardAlpha,
            "palettized_plus_sprite_alpha" | "palettized_plus_alpha" => Self::PaletteSpriteAlpha,
            "diffuse_modulated" => Self::DiffuseModulated,
            "palettized_glow" => Self::PaletteGlow,
            "palettized_plasma" => Self::PalettePlasma,
            "palettized_2d_plasma" => Self::Palette2dPlasma,
            _ => Self::Diffuse,
        }
    }
}

/// A `.particle`.
#[derive(Debug)]
pub(crate) struct H3ParticleLook {
    /// `dies at rest`.
    dies_at_rest: bool,
    /// Whether a speed of none is nudged to a little.
    still_moves: bool,
    billboard: H3Billboard,
    flip_u: bool,
    flip_v: bool,
    random_rotation: bool,
    intensity_fades: bool,
    one_shot: bool,
    backwards: bool,
    /// `attachments`: effects its particles start, by what starts them.
    attachments: Vec<(H3ParticleTrigger, String)>,
    /// Whether the tag lists any attachment at all, which is what keeps its
    /// emitters off the card (`c_particle_definition::postprocess`).
    any_attachments: bool,
    /// A Halo 2 particle with `attached particle systems`: its own tag.
    carried_systems: Option<String>,
    /// Halo 2: `sub_61AE40` scales a death or collision effect by `speed - 0.5`.
    death_scaled_by_speed: bool,
    /// `s_mesh_variant_list`: what `Model` draws in place of a quad.
    model: Option<Vec<Vec<blam_tags::ParticleVertex>>>,
    /// `bitmap authored vertically`: a quarter turn, for what is not a mesh.
    vertical: bool,
    /// `dies on structure collision`.
    dies_on_ground: bool,
    /// `tint from lightmap`.
    tint_from_lightmap: bool,
    double_sided: bool,
    /// `fade when viewed edge-on`: `1 / range` and the cutoff, in radians.
    edge_fade: Option<[f32; 2]>,
    /// `motion blur`'s stretch.
    blur_aspect: Option<f32>,
    /// The postprocess's: at 1, a hemisphere.
    curvature: f32,
    /// `m_sprite`: the quad's corner and size in its own units.
    corner: [f32; 4],
    /// `m_frames`: left, top, width and height.
    frames: Vec<[f32; 4]>,
    /// Halo 2's `select random sequence`: each sequence's frames and corner,
    /// one of which a particle takes at birth. Empty for one sequence.
    sequences: Vec<(Vec<[f32; 4]>, [f32; 4])>,
    aspect: H3ParticleProperty,
    color: H3ParticleProperty,
    intensity: H3ParticleProperty,
    alpha: H3ParticleProperty,
    frame: H3ParticleProperty,
    rate: H3ParticleProperty,
    palette: H3ParticleProperty,
    pub(crate) shader: Arc<H3SpriteShader>,
    /// Halo 4's: drawn through its material from packed state, not as quads.
    pub(crate) h4: Option<Arc<super::h4_particles::H4ParticleRender>>,
    /// Halo 3's colours are light, and Halo 2's are what shows.
    exposed: bool,
    /// Reach's: its own game time, lighting, exposure and shapes.
    reach: bool,
    /// Reach's `uses cheap shader`: `particle_render_fast.hlsl`.
    cheap: bool,
    /// Reach's `runtime m_used_particle_states` (prt3+460): the states its
    /// properties read, seeds allocated included, which its emitters' seeds
    /// must go around. None for Halo 3's.
    used_states: u32,
}

/// The `lighting` option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H3ParticleLighting {
    None,
    Ambient,
    Directional,
    /// Reach's `per_vertex_ambient`: the probe's ambient, on the vertex.
    ReachAmbient,
    /// Reach's `per_pixel_smooth`: between the probe's dark and bright, by
    /// how the bent normal faces its bright direction.
    ReachSmooth,
}

/// `ravi_order_0` and `ravi_order_3`, of `calculate_and_set_ravi_constants`'
/// ten: the ambient, or what comes from `normal`.
fn ravi(lighting: H3ParticleLighting, normal: [f32; 3], constants: &[[f32; 4]; 10]) -> [f32; 3] {
    const C1: f32 = 0.429_043;
    const C2: f32 = 0.511_664;
    const C4: f32 = 0.886_227;
    let product = |a: [f32; 3], b: [f32; 4]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let [x, y, z] = normal;
    std::array::from_fn(|channel| {
        let ambient = C4 * constants[0][channel];
        let value = if lighting == H3ParticleLighting::Ambient {
            ambient
        } else {
            let linear = product(normal, constants[1 + channel]);
            let first = product([x * y, y * z, z * x], constants[4 + channel]);
            let [a, b, c, d] = constants[7 + channel];
            let second = x * x * a + y * y * b + z * z * c + d / 3.0;
            ambient - 2.0 * C2 * linear - 2.0 * C1 * first - C1 * second
        };
        value / std::f32::consts::PI
    })
}

/// What particles, beams and contrails ask of a shader alike.
#[derive(Debug)]
pub(crate) struct H3SpriteShader {
    pub(crate) albedo: H3ParticleAlbedo,
    pub(crate) blend: H3Blend,
    pub(crate) black_point: bool,
    pub(crate) frame_blend: bool,
    /// `depth_fade_range`, with the `depth_fade` option on.
    pub(crate) depth_fade: Option<f32>,
    add: [f32; 3],
    /// Reach's: `add` is at `V_ILLUM_EXPOSURE` (`default_vs`).
    add_at_illum: bool,
    /// Reach's beams: `sample_diffuse` takes a `particle_alpha` of 1
    /// (`effects__beam.hlsl` l.373–458), not the vertex's alpha.
    pub(crate) unit_particle_alpha: bool,
    /// `alpha_test.hlsl`: under a half of alpha it is gone, and otherwise
    /// solid, writing depth like anything opaque.
    pub(crate) alpha_test: bool,
    /// Whether the pixel is lit, as `per_pixel_ravi_order_3` is.
    pub(crate) lit_pixels: bool,
    /// `sort layer`: `transparent_layer_and_z_sort_proc` goes by this first.
    pub(crate) sort_layer: i32,
    /// `distortion_scale`, for what bends the scene instead of covering it.
    pub(crate) distortion: Option<f32>,
    lighting: H3ParticleLighting,
    /// `starting_uv_scale` and `ending_uv_scale`.
    uv_scale: [f32; 2],
    pub(crate) base_map: Option<H3Bitmap>,
    pub(crate) alpha_map: Option<H3Bitmap>,
    pub(crate) palette_map: Option<H3Bitmap>,
    /// `base_map2`, which only `palettized_plasma` reads.
    pub(crate) second_map: Option<H3Bitmap>,
    /// `base_map`'s, `alpha_map`'s, `palette`'s and `base_map2`'s, as baked.
    pub(crate) samplers: [Option<H3Sampler>; 4],
    pub(crate) constants: H3AlbedoConstants,
    /// Reach's `per_pixel_smooth`: `contrast_scale` and `contrast_offset`.
    contrast: [f32; 2],
    /// Reach's `warp` `sphere`: `sphere_warp_scale`.
    pub(crate) sphere_warp: Option<f32>,
    /// Reach's `depth_fade` `palette_shift`: `palette_shift_amount`.
    pub(crate) palette_shift: Option<f32>,
    /// Reach's plasmas read `base_map2` at the second sprite
    /// (`particle_render.hlsl`'s `texcoord_sprite1`), not the first.
    pub(crate) plasma_second_sprite: bool,
}

impl H3SpriteShader {
    /// The constant self-illumination added, at the area screen effects'
    /// exposure `stops` where it is Reach's.
    pub(crate) fn add(&self, stops: f32) -> [f32; 3] {
        let exposure = if self.add_at_illum { reach_alt_exposure(stops)[1] } else { 1.0 };
        self.add.map(|channel| channel * exposure)
    }

    /// Reach's `per_pixel_smooth` contrast, when its pixels are lit so.
    pub(crate) fn smooth_contrast(&self) -> Option<[f32; 2]> {
        (self.lighting == H3ParticleLighting::ReachSmooth).then_some(self.contrast)
    }

    /// What a leaf draws with: only a map, cut out at half its alpha.
    fn plain(blend: H3Blend) -> Self {
        Self {
            albedo: H3ParticleAlbedo::Diffuse,
            blend,
            black_point: false,
            frame_blend: false,
            depth_fade: None,
            add: [0.0; 3],
            add_at_illum: false,
            unit_particle_alpha: false,
            alpha_test: false,
            lit_pixels: false,
            sort_layer: 2,
            distortion: None,
            lighting: H3ParticleLighting::None,
            uv_scale: [1.0; 2],
            base_map: None,
            alpha_map: None,
            palette_map: None,
            second_map: None,
            samplers: [None; 4],
            constants: H3AlbedoConstants::default(),
            contrast: [0.0; 2],
            sphere_warp: None,
            palette_shift: None,
            plasma_second_sprite: false,
        }
    }

    pub(crate) fn alpha_tested(blend: H3Blend, base_map: H3Bitmap) -> Self {
        // `leaf_system_render_opaque` binds its plate clamped and bilinear.
        let plate = H3Sampler { address: [BitmapAddressMode::Clamp; 2], filter: BitmapFilterMode::Bilinear };
        Self { alpha_test: true, base_map: Some(base_map), samplers: [Some(plate), None, None, None], ..Self::plain(blend) }
    }

    #[cfg(test)]
    pub(crate) fn blank() -> Self {
        Self::plain(H3Blend::Additive)
    }
}

/// A kit's `tags` folder as the effects tests read it, against the bundled
/// definitions.
#[cfg(test)]
pub(super) fn loose_kit(root: &str, game: GameId) -> TagSource {
    TagSource::LooseFolder {
        root: root.into(),
        game: Some(game),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    }
}

/// `sort layer`, by option name: what is unset is `normal`.
pub(super) fn h3_sort_layer(shader: TagStruct<'_>) -> i32 {
    match shader.read_enum_name("sort layer").as_deref() {
        Some("pre-pass") => 1,
        Some("post-pass") => 3,
        _ => 2,
    }
}

/// A sprite system's `actual shader?`, by its options' names. Reach's options
/// (`shaders\particle.render_method_definition`) read their parameters
/// through the resolver, falling to what their option tags default them to.
pub(super) fn read_sprite_shader(source: &TagSource, shader: TagStruct<'_>) -> Option<H3SpriteShader> {
    let definition = shader.read_tag_ref_path("definition").filter(|path| !path.is_empty())?;
    let definition = read_h3_definition(source, &definition)?;
    let options: Vec<usize> = elements(shader, "options")
        .iter()
        .map(|option| option.read_int_any("short").unwrap_or(0).max(0) as usize)
        .collect();
    let chosen = |category: &str| definition.chosen(category, &options).unwrap_or_default().to_owned();
    let reach = source.game() == Some(GameId::HaloReach);
    let resolved = resolve_embedded_method(source, shader);
    let resolved_real = |name: &str, default: f32| inline_real(resolved.as_ref(), name).unwrap_or(default);
    let add = parameter(shader, "self_illum_color")
        .filter(|_| chosen("self_illumination") == "constant_color")
        .and_then(|parameter| elements(parameter, "animated parameters").first().copied())
        .and_then(|animated| tag_function_at(&animated, "function"))
        .map(|function| function.evaluate_color(0.0, 0.0))
        .map_or([0.0; 3], |color| [color.red, color.green, color.blue]);
    let ([base_map, alpha_map, palette_map, second_map], samplers) = read_maps(source, shader);
    let depth_fade = chosen("depth_fade");
    // `compute_depth_fade`: Reach's `low_res` is `on` on DirectX 11, its
    // four taps being the 360's alone, and `palette_shift` fades as `on` does.
    let fades = if reach { matches!(depth_fade.as_str(), "on" | "low_res" | "palette_shift") } else { depth_fade == "on" };
    let albedo = H3ParticleAlbedo::named(&chosen("albedo"));
    Some(H3SpriteShader {
        albedo,
        blend: definition.blend(&options),
        black_point: chosen("black_point") == "on",
        frame_blend: chosen("frame_blend") == "on",
        depth_fade: (fades && chosen("blend_mode") != "opaque").then(|| {
            let range = if reach {
                resolved_real("depth_fade_range", 0.1)
            } else {
                parameter(shader, "depth_fade_range")
                    .and_then(|parameter| elements(parameter, "animated parameters").first().copied())
                    .and_then(|animated| tag_function_at(&animated, "function"))
                    .map_or(1.0, |function| function.evaluate(0.0, 0.0))
            };
            range.max(1e-4)
        }),
        add,
        add_at_illum: false,
        unit_particle_alpha: false,
        alpha_test: false,
        lit_pixels: chosen("lighting") == "per_pixel_ravi_order_3",
        sort_layer: h3_sort_layer(shader),
        distortion: chosen("specialized_rendering").starts_with("distortion").then(|| {
            parameter(shader, "distortion_scale")
                .and_then(|parameter| elements(parameter, "animated parameters").first().copied())
                .and_then(|animated| tag_function_at(&animated, "function"))
                .map_or(1.0, |function| function.evaluate(0.0, 0.0))
        }),
        lighting: match chosen("lighting").as_str() {
            "per_vertex_ravi_order_0" => H3ParticleLighting::Ambient,
            "per_pixel_ravi_order_3" => H3ParticleLighting::Directional,
            "per_vertex_ambient" => H3ParticleLighting::ReachAmbient,
            "per_pixel_smooth" => H3ParticleLighting::ReachSmooth,
            // `smoke_lighting` draws normals into the low-res transparency
            // buffer, and `lit_particle_overdraw_apply.hlsl` lights that in a
            // pass of its own, whose compositing was not read: it draws unlit.
            _ => H3ParticleLighting::None,
        },
        uv_scale: ["starting_uv_scale", "ending_uv_scale"].map(|name| {
            parameter(shader, name)
                .and_then(|parameter| elements(parameter, "animated parameters").first().copied())
                .and_then(|animated| tag_function_at(&animated, "function"))
                .map_or(1.0, |function| function.evaluate(0.0, 0.0))
        }),
        base_map,
        alpha_map,
        palette_map,
        second_map,
        samplers,
        constants: read_albedo_constants(shader, resolved.as_ref(), reach),
        // `smooth_lighting.render_method_option`'s defaults.
        contrast: [resolved_real("contrast_scale", 0.5), resolved_real("contrast_offset", 0.5)],
        // `warp_sphere.render_method_option`'s default.
        sphere_warp: (reach && chosen("warp") == "sphere").then(|| resolved_real("sphere_warp_scale", 0.41)),
        // `depth_fade_palette_shift.render_method_option`'s default.
        palette_shift: (reach && depth_fade == "palette_shift").then(|| resolved_real("palette_shift_amount", 0.5)),
        plasma_second_sprite: reach && matches!(albedo, H3ParticleAlbedo::PalettePlasma | H3ParticleAlbedo::Palette2dPlasma),
    })
}

/// A resolved parameter's value, when the resolver bakes one in.
fn inline_value(resolved: Option<&ResolvedRenderMethod>, name: &str) -> Option<ResolvedValue> {
    let ParameterSource::Inline(value) = &resolved?.find(name)?.source else {
        return None;
    };
    Some(value.clone())
}

pub(super) fn inline_real(resolved: Option<&ResolvedRenderMethod>, name: &str) -> Option<f32> {
    match inline_value(resolved, name)? {
        ResolvedValue::Real(real) => Some(real),
        _ => None,
    }
}

fn parameter<'a>(shader: TagStruct<'a>, name: &str) -> Option<TagStruct<'a>> {
    let parameters = elements(shader, "parameters");
    parameters.into_iter().find(|parameter| parameter.read_string_id("parameter name").as_deref() == Some(name))
}

fn read_map(source: &TagSource, shader: TagStruct<'_>, name: &str) -> Option<H3Bitmap> {
    let path = parameter(shader, name)?.read_tag_ref_path("bitmap").filter(|path| !path.is_empty())?;
    read_h3_bitmap(source, &path, 0, SamplerKind::Texture2D)
}

/// A sprite shader's maps, and the sampler each is baked with. Failing to
/// resolve leaves what it authored, on the preview's default sampler.
fn read_maps(source: &TagSource, shader: TagStruct<'_>) -> ([Option<H3Bitmap>; 4], [Option<H3Sampler>; 4]) {
    const MAPS: [&str; 4] = ["base_map", "alpha_map", "palette", "base_map2"];
    let Some(resolved) = resolve_embedded_method(source, shader) else {
        return (MAPS.map(|name| read_map(source, shader, name)), [None; 4]);
    };
    let mut maps = MAPS.map(|name| read_resolved_map(source, &resolved, name)).into_iter();
    let (bitmaps, samplers): (Vec<_>, Vec<_>) = std::array::from_fn::<_, 4, _>(|_| maps.next().flatten())
        .map(|map| map.map_or((None, None), |(bitmap, sampler)| (Some(bitmap), Some(sampler))))
        .into_iter()
        .unzip();
    let mut bitmaps = bitmaps.into_iter();
    let mut samplers = samplers.into_iter();
    (std::array::from_fn(|_| bitmaps.next().flatten()), std::array::from_fn(|_| samplers.next().flatten()))
}

/// A map's `_xform`, its scale then its offset, from its animators at rest.
fn sprite_xform(shader: TagStruct<'_>, name: &str) -> [f32; 4] {
    let mut xform = [1.0, 1.0, 0.0, 0.0];
    let Some(found) = parameter(shader, name) else {
        return xform;
    };
    for animated in elements(found, "animated parameters") {
        let Some(function) = tag_function_at(&animated, "function") else {
            continue;
        };
        let value = function.evaluate(0.0, 0.0);
        match animated.read_enum_name("type").as_deref() {
            Some("scale uniform") => xform[..2].fill(value),
            Some("scale x") => xform[0] = value,
            Some("scale y") => xform[1] = value,
            Some("translation x") => xform[2] = value,
            Some("translation y") => xform[3] = value,
            _ => {}
        }
    }
    xform
}

fn read_albedo_constants(shader: TagStruct<'_>, resolved: Option<&ResolvedRenderMethod>, reach: bool) -> H3AlbedoConstants {
    let real = |name: &str| inline_real(resolved, name).unwrap_or(0.0);
    let tint = match inline_value(resolved, "tint_color") {
        Some(ResolvedValue::Color([_, red, green, blue])) => [red, green, blue],
        _ => [1.0; 3],
    };
    // `albedo_palettized_plasma.render_method_option`'s, which Reach's 2d
    // plasma shares.
    let alpha_modulation = if reach {
        inline_real(resolved, "alpha_modulation_factor").unwrap_or(0.1)
    } else {
        real("alpha_modulation_factor")
    };
    H3AlbedoConstants {
        tint,
        modulation: real("modulation_factor"),
        alpha_modulation,
        base_xform: sprite_xform(shader, "base_map"),
        second_xform: sprite_xform(shader, "base_map2"),
    }
}

/// `postprocess_frame_animation`'s `m_sprite`, from a sprite's left, right, top
/// and bottom, its registration point, and the particle's `center offset`.
fn sprite_corner(rect: [f32; 4], center: [f32; 2], offset: [f32; 2], plate_aspect: f32) -> [f32; 4] {
    let [left, right, top, bottom] = rect;
    let (width, height) = (right - left, bottom - top);
    let (mut scale_x, mut scale_y) = (1.0, 1.0);
    if width <= height {
        scale_x = width / height;
    } else {
        scale_y = height / width;
    }
    scale_x *= plate_aspect;
    let shift_x = 2.0 * (center[0] + offset[0]) / width - 1.0;
    let shift_y = 2.0 * (center[1] - offset[1]) / height - 1.0;
    let (x, y) = ((-1.0 - shift_x) * scale_x, (-1.0 - shift_y) * scale_y);
    [x, y, (1.0 - shift_x) * scale_x - x, (1.0 - shift_y) * scale_y - y]
}

/// `postprocess_frame_animation`: the quad from the first sequence's sprite at
/// the starting frame, and the frames from `first + 1`'s, manual ones first.
fn read_frames(
    source: &TagSource,
    shader: TagStruct<'_>,
    first: usize,
    offset: [f32; 2],
    start: f32,
    one_shot: bool,
) -> (Vec<[f32; 4]>, [f32; 4]) {
    let whole = vec![[0.0, 0.0, 1.0, 1.0]];
    let tag = parameter(shader, "base_map")
        .and_then(|base| base.read_tag_ref_path("bitmap"))
        .and_then(|path| load_referenced_tag_from_source(source, &path, "bitmap", b"bitm").ok());
    let Some(tag) = tag else {
        return (whole, sprite_corner([0.0, 1.0, 0.0, 1.0], [0.5, 0.5], offset, 1.0));
    };
    let plate_aspect = elements(tag.root(), "bitmaps").first().map_or(1.0, |image| {
        let size = |name: &str| image.read_int_any(name).unwrap_or(1) as f32;
        size("width") / size("height")
    });
    let read = |sprite: &TagStruct<'_>, name: &str| sprite.read_real(name).unwrap_or(0.0);
    let sequences = elements(tag.root(), "sequences");
    let (mut rect, mut center) = ([0.0, 1.0, 0.0, 1.0], [0.5, 0.5]);
    if let Some(sequence) = sequences.get(first % sequences.len().max(1)) {
        let sprites = elements(*sequence, "sprites");
        let count = if one_shot { sprites.len().saturating_sub(1) } else { sprites.len() };
        if count > 0 {
            let sprite = sprites[(count as f32 * start).floor() as i32 as usize % sprites.len()];
            rect = ["left", "right", "top", "bottom"].map(|name| read(&sprite, name));
            let registration = sprite.read_point2d("registration point");
            center = [registration.x, registration.y];
        }
    }
    let manual = elements(tag.root(), "manual_sequences");
    let listed = if manual.is_empty() { sequences } else { manual };
    let frames: Vec<[f32; 4]> = listed
        .get((first + 1) % listed.len().max(1))
        .map(|sequence| elements(*sequence, "sprites"))
        .unwrap_or_default()
        .iter()
        .take(MOST_FRAMES)
        .map(|sprite| {
            let [left, right, top, bottom] = ["left", "right", "top", "bottom"].map(|name| read(sprite, name));
            [left, top, right - left, bottom - top]
        })
        .collect();
    let frames = if frames.is_empty() {
        vec![[rect[0], rect[2], rect[1] - rect[0], rect[3] - rect[2]]]
    } else {
        frames
    };
    (frames, sprite_corner(rect, center, offset, plate_aspect))
}

/// A `particle_model`'s meshes, each a triangle list.
fn read_particle_mesh(source: &TagSource, path: &str) -> Option<Vec<Vec<blam_tags::ParticleVertex>>> {
    let tag = load_referenced_tag_from_source(source, path, "particle_model", b"pmdf").ok()?;
    let meshes = blam_tags::particle_model_meshes(&tag, "particle_model").ok()?;
    let triangles = |mesh: &blam_tags::ParticleObjectMesh| {
        mesh.indices.iter().filter_map(|index| mesh.vertices.get(*index as usize).copied()).collect::<Vec<_>>()
    };
    let meshes: Vec<_> = meshes.iter().map(triangles).filter(|mesh: &Vec<_>| !mesh.is_empty()).collect();
    (!meshes.is_empty()).then_some(meshes)
}

fn read_look(source: &TagSource, path: &str) -> Option<H3ParticleLook> {
    let tag = load_referenced_tag_from_source(source, path, "particle", b"prt3").ok()?;
    let root = tag.root();
    let model = match root.read_tag_ref_path("Model").filter(|model| !model.is_empty()) {
        Some(model) => Some(read_particle_mesh(source, &model)?),
        None => None,
    };
    // Halo 4 draws through the particle's own `actual material`; its
    // `actual shader` is the deprecated render method's remains.
    let halo4 = source.game() == Some(GameId::Halo4);
    let h4 = match halo4 {
        true => {
            // The GPU draws a model's strip whole, variant by variant.
            let strip = match root.read_tag_ref_path("Model").filter(|model| !model.is_empty()) {
                Some(path) => {
                    let tag = load_referenced_tag_from_source(source, &path, "particle_model", b"pmdf").ok()?;
                    Some(blam_tags::particle_model_strip(&tag).ok()?)
                }
                None => None,
            };
            Some(Arc::new(super::h4_particles::read_h4_particle_render(source, root, strip.as_ref()).ok()?))
        }
        false => None,
    };
    let shader = root.descend("actual shader?");
    let flags = root.read_flag_names("appearance flags").unwrap_or_default();
    let flagged = |wanted: &str| flags.iter().any(|(_, name)| name.starts_with(wanted));
    let animation = root.read_flag_names("animation flags").unwrap_or_default();
    let animated = |wanted: &str| animation.iter().any(|(_, name)| name.starts_with(wanted));
    let first = root.read_int_any("first sequence index").unwrap_or(0).max(0) as usize;
    let offset = root.read_point2d("center offset");
    let start = tag_function_at(&root.descend("frame index")?, "Mapping").map_or(0.0, |function| function.evaluate(0.0, 0.0));
    let one_shot = animated("frame animation one shot");
    let (frames, corner) = match shader.filter(|_| !halo4) {
        Some(shader) => read_frames(source, shader, first, [offset.x, offset.y], start, one_shot),
        None => (vec![[0.0, 0.0, 1.0, 1.0]], [0.0, 0.0, 1.0, 1.0]),
    };
    let billboard = H3Billboard::named(&root.read_enum_name("particle billboard style").unwrap_or_default());
    // Halo 4's simulation is Reach's, with terms of its own.
    let reach = matches!(source.game(), Some(GameId::HaloReach | GameId::Halo4));
    let main_flags = root.read_flag_names("main flags").unwrap_or_default();
    let cheap = reach && main_flags.iter().any(|(_, name)| name.starts_with("uses cheap shader"));
    const PROPERTIES: [&str; 7] =
        ["aspect ratio", "color", "intensity", "alpha", "frame index", "animation rate", "palette animation"];
    let mut properties = unallocated_properties(root, PROPERTIES)?;
    // Reach's `c_particle_definition::postprocess` (tool 0x140DD6FA0) takes
    // the mask of all seven before allocating them in this order. Its
    // attachments' scales come between, which the preview does not read.
    let mut seeds = if reach {
        SeedAllocator::reach(used_states_of(&properties) & REACH_ALLOCATED_STATES)
    } else {
        SeedAllocator::halo3()
    };
    properties.iter_mut().for_each(|property| property.allocate(&mut seeds));
    let [aspect, color, intensity, alpha, frame, rate, palette] = properties;
    let mut sprite_shader = match shader.filter(|_| !halo4) {
        Some(shader) => read_sprite_shader(source, shader)?,
        None => H3SpriteShader::plain(H3Blend::Additive),
    };
    // `default_vs`: Reach's constant self-illumination is at `V_ILLUM_EXPOSURE`.
    sprite_shader.add_at_illum = reach;
    if cheap {
        // `particle_render_fast.hlsl` warps nothing and lights no pixel.
        sprite_shader.sphere_warp = None;
        if sprite_shader.lighting == H3ParticleLighting::ReachSmooth {
            sprite_shader.lighting = H3ParticleLighting::None;
        }
    }
    Some(H3ParticleLook {
        billboard,
        still_moves: still_particles_move(source.game(), billboard),
        flip_u: flagged("random u mirror"),
        flip_v: flagged("random v mirror"),
        random_rotation: flagged("random starting rotation"),
        intensity_fades: flagged("intensity affects alpha"),
        model,
        vertical: flagged("bitmap authored vertically"),
        dies_on_ground: main_flags.iter().any(|(_, name)| name.starts_with("dies on structure collision")),
        dies_at_rest: main_flags.iter().any(|(_, name)| name.starts_with("dies at rest")),
        tint_from_lightmap: flagged("tint from lightmap"),
        double_sided: flagged("double-sided"),
        attachments: elements(root, "attachments")
            .iter()
            .filter_map(|attachment| {
                let trigger = match attachment.read_enum_name("trigger").as_deref() {
                    Some("birth") => H3ParticleTrigger::Birth,
                    Some("death") => H3ParticleTrigger::Death,
                    Some("collision") => H3ParticleTrigger::Collision,
                    _ => return None,
                };
                let (group, path) = attachment.read_tag_ref_with_group("type")?;
                (group == u32::from_be_bytes(*b"effe") && !path.is_empty()).then_some((trigger, path))
            })
            .collect(),
        any_attachments: !elements(root, "attachments").is_empty(),
        carried_systems: None,
        death_scaled_by_speed: false,
        edge_fade: flagged("fade when viewed edge-on").then(|| {
            let real = |name: &str| root.read_real(name).unwrap_or(0.0);
            let range = real("angle fade range");
            let range = if range < 0.0001 { 30.0 } else { range };
            let radians = std::f32::consts::PI / 180.0;
            [(range * radians).recip(), real("angle fade cutoff") * radians]
        }),
        blur_aspect: flagged("motion blur").then(|| root.read_real("motion blur aspect scale").unwrap_or(0.0)),
        curvature: root.read_real("curvature").unwrap_or(0.0).clamp(0.0, 1.0) * 0.9999,
        one_shot,
        backwards: animated("can animate backwards"),
        corner,
        frames,
        sequences: Vec::new(),
        aspect,
        color,
        intensity,
        alpha,
        frame,
        rate,
        palette,
        shader: Arc::new(sprite_shader),
        h4,
        exposed: true,
        reach,
        cheap,
        used_states: seeds.used(),
    })
}

/// `holder`'s properties of these names, none yet seeded; none if any is missing.
fn unallocated_properties<const N: usize>(holder: TagStruct<'_>, names: [&str; N]) -> Option<[H3ParticleProperty; N]> {
    let read = names.map(|name| H3ParticleProperty::unallocated(holder.descend(name)?));
    read.into_iter().collect::<Option<Vec<_>>>()?.try_into().ok()
}

/// What a definition's properties read, by Reach's state bits.
fn used_states_of(properties: &[H3ParticleProperty]) -> u32 {
    properties.iter().fold(0, |used, property| used | property.used_states())
}

/// Halo 2's `sequences`: the sprites of `first`, on a plate of its own shape.
fn read_h2_frames(tag: &TagFile, first: usize) -> (Vec<[f32; 4]>, [f32; 4]) {
    let plate_aspect = elements(tag.root(), "bitmaps").first().map_or(1.0, |image| {
        let size = |name: &str| image.read_int_any(name).unwrap_or(1) as f32;
        size("width") / size("height")
    });
    let sequences = elements(tag.root(), "sequences");
    let sprites = sequences.get(first % sequences.len().max(1)).map_or(Vec::new(), |sequence| elements(*sequence, "sprites"));
    let rect_of = |sprite: &TagStruct<'_>| ["left", "right", "top", "bottom"].map(|name| sprite.read_real(name).unwrap_or(0.0));
    let frames: Vec<[f32; 4]> = sprites
        .iter()
        .take(MOST_FRAMES)
        .map(|sprite| {
            let [left, right, top, bottom] = rect_of(sprite);
            [left, top, right - left, bottom - top]
        })
        .collect();
    let (rect, center) = sprites.first().map_or(([0.0, 1.0, 0.0, 1.0], [0.5, 0.5]), |sprite| {
        let registration = sprite.read_point2d("registration point");
        (rect_of(sprite), [registration.x, registration.y])
    });
    let frames = if frames.is_empty() { vec![[0.0, 0.0, 1.0, 1.0]] } else { frames };
    (frames, sprite_corner(rect, center, [0.0; 2], plate_aspect))
}

/// `effect_shader_*`'s pixel stage and blend, by the template that names the
/// pass, by name alone: the distortion one lives under `particle\` too.
fn h2_particle_pass(template: &str) -> Option<(H3ParticleAlbedo, H3Blend, bool)> {
    use H3ParticleAlbedo::{Diffuse, H2Distortion, H2MultiplyAdd, H2Tint};
    Some(match template.rsplit(['\\', '/']).next().unwrap_or_default() {
        "particle_additive" => (Diffuse, H3Blend::AddSrcTimesSrcAlpha, false),
        "particle_additive_tint" => (H2Tint, H3Blend::AddSrcTimesSrcAlpha, false),
        "particle_alpha_blend" => (Diffuse, H3Blend::AlphaBlend, false),
        "particle_alpha_blend_tint" => (H2Tint, H3Blend::AlphaBlend, false),
        "particle_alpha_multiply_add" => (H2MultiplyAdd, H3Blend::PreMultiplied, false),
        // Its alpha test passes anything above nothing, which the albedo does.
        "particle_alpha_blend_distortion" => (H2Distortion, H3Blend::AlphaBlend, false),
        "particle_alpha_test_no_lighting_fixed_for_glass" => (Diffuse, H3Blend::AlphaBlend, true),
        _ => return None,
    })
}

/// A Halo 2 `.particle`, and the two properties of a quad that Halo 3 moved to
/// its emitters.
fn read_h2_look(source: &TagSource, path: &str) -> Option<(H3ParticleLook, H3ParticleProperty, H3ParticleProperty)> {
    let tag = load_referenced_tag_from_source(source, path, "particle", b"prt3").ok()?;
    let root = tag.root();
    let template = root.read_tag_ref_path("shader template")?;
    let (albedo, blend, alpha_test) = h2_particle_pass(template.trim_end_matches(".shader_template"))?;
    let parameters = elements(root, "shader parameters");
    let parameter = |name: &str| parameters.iter().find(|entry| entry.read_string_id("name").as_deref() == Some(name));
    let distorts = albedo == H3ParticleAlbedo::H2Distortion;
    let map_name = if distorts { "bump_map" } else { "alpha_map_texture" };
    let map = parameter(map_name)?.read_tag_ref_path("bitmap").filter(|path| !path.is_empty())?;
    let plate = load_referenced_tag_from_source(source, &map, "bitmap", b"bitm").ok()?;
    let flags = root.read_flag_names("flags").unwrap_or_default();
    let flagged = |wanted: &str| flags.iter().any(|(_, name)| name.starts_with(wanted));
    let first = root.read_int_any("first sequence index").unwrap_or(0).max(0) as usize;
    let (frames, corner) = read_h2_frames(&plate, first);
    // `sub_53CEA0`: `first + random % count`, held inside the bitmap's.
    let sequences = if flagged("select random sequence") {
        let count = root.read_int_any("sequence count").unwrap_or(1).max(1) as usize;
        let last = elements(plate.root(), "sequences").len().saturating_sub(1);
        (0..count).map(|at| read_h2_frames(&plate, (first + at).min(last))).collect()
    } else {
        Vec::new()
    };
    // The distortion's pixel shader reads `distortion_scale` where the others
    // read `modulation_factor`; its template's default is 1.
    let modulation = if distorts {
        parameter("distortion_scale").and_then(|entry| entry.read_real("const value")).unwrap_or(1.0)
    } else {
        parameter("modulation_factor").and_then(|entry| entry.read_real("const value")).unwrap_or(0.0)
    };
    let shader = H3SpriteShader {
        albedo,
        alpha_test,
        frame_blend: !flagged("disable frame blending"),
        base_map: Some(read_h3_bitmap(source, &map, 0, SamplerKind::Texture2D)?),
        constants: H3AlbedoConstants { modulation, ..Default::default() },
        ..H3SpriteShader::plain(blend)
    };
    let mut seeds = SeedAllocator::default();
    let billboard = H3Billboard::named(&root.read_enum_name("particle billboard style").unwrap_or_default());
    let mut property = |name: &str| H3ParticleProperty::named(root, name, &mut seeds);
    let (color, alpha, scale, rotation, frame) =
        (property("color")?, property("alpha")?, property("scale")?, property("rotation")?, property("frame index")?);
    let look = H3ParticleLook {
        billboard,
        still_moves: false,
        flip_u: flagged("random u mirror"),
        flip_v: flagged("random v mirror"),
        random_rotation: false,
        intensity_fades: false,
        model: None,
        vertical: flagged("bitmap authored vertically"),
        dies_on_ground: flagged("dies on structure collision"),
        dies_at_rest: flagged("dies at rest"),
        // The doubled tint of what is lit comes from a level.
        tint_from_lightmap: false,
        double_sided: true,
        // `sub_61AE40`: a death's `effect` is made where the particle died, and
        // a collision's where it first struck (`sub_53DF20`). A material effect
        // needs the surface's material, which the preview's ground has none
        // of, and a sound draws nothing.
        attachments: [("death effect", H3ParticleTrigger::Death), ("collision effect", H3ParticleTrigger::Collision)]
            .into_iter()
            .filter_map(|(field, trigger)| {
                let (group, path) = root.read_tag_ref_with_group(field)?;
                (group == u32::from_be_bytes(*b"effe") && !path.is_empty()).then_some((trigger, path))
            })
            .collect(),
        // Read only for Halo 3 and ODST: Halo 2's emitters are all on the processor.
        any_attachments: false,
        carried_systems: (!elements(root, "attached particle systems").is_empty()).then(|| path.to_owned()),
        death_scaled_by_speed: true,
        edge_fade: None,
        blur_aspect: None,
        curvature: 0.0,
        one_shot: flagged("frame animation one shot"),
        backwards: flagged("can animate backwards"),
        corner,
        frames,
        sequences,
        aspect: H3ParticleProperty::fixed(1.0)?,
        color,
        intensity: H3ParticleProperty::fixed(1.0)?,
        alpha,
        frame,
        rate: H3ParticleProperty::fixed(0.0)?,
        palette: H3ParticleProperty::fixed(0.0)?,
        shader: Arc::new(shader),
        h4: None,
        exposed: false,
        reach: false,
        cheap: false,
        used_states: 0,
    };
    Some((look, scale, rotation))
}

/// `c_particle_emitter_definition`'s emission shapes, by option name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H3EmissionShape {
    Sprayer,
    Disc,
    Globe,
    Implode,
    Tube,
    Halo,
    Debris,
    Line,
    ImpactContour,
    ImpactArea,
    Breakable,
    /// ODST's: from the points of the emitter's `custom shape`.
    CustomPoints,
    /// Reach's: anywhere in a box `emission radius` out each way.
    Cube,
    /// Reach's: along x from -1 to 1, out to `emission radius` around it.
    Cylinder,
    /// Reach's: as `Line`, drawn from the local random seed.
    UnweightedLine,
    /// Reach's: a box of `emission radius` that `axis scale` flattens.
    Plane,
}

/// ODST's `s_particle_emitter_custom_points::s_point`, decompressed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct H3CustomPoint {
    position: [f32; 3],
    normal: [f32; 3],
}

/// A `particle_emitter_custom_points`.
fn read_custom_points(source: &TagSource, emitter: TagStruct<'_>) -> Vec<H3CustomPoint> {
    let path = emitter.read_tag_ref_path("custom shape").filter(|path| !path.is_empty());
    let tag = path.and_then(|path| {
        load_referenced_tag_from_source(source, &path, "particle_emitter_custom_points", b"pecp").ok()
    });
    let Some(tag) = tag else {
        return Vec::new();
    };
    let root = tag.root();
    let (scale, offset) = (root.read_vec3("compression scale"), root.read_vec3("compression offset"));
    let word = |point: TagStruct<'_>, name: &str| point.read_int_any(name).unwrap_or(0) as f32;
    let points = elements(root, "points").into_iter().map(|point| H3CustomPoint {
        position: [
            word(point, "position_x") * scale.i + offset.i,
            word(point, "position_y") * scale.j + offset.j,
            word(point, "position_z") * scale.k + offset.k,
        ],
        normal: ["normal_x", "normal_y", "normal_z"].map(|name| word(point, name) / 127.0),
    });
    points.collect()
}

/// `initialize_particle`'s custom shape: `emission radius` chooses the point
/// and `emission angle` how far its normal leans to a random direction, in a
/// straight line rather than around the sphere.
fn emit_from_points(
    points: &[H3CustomPoint],
    radius: f32,
    angle_degrees: f32,
    random: &mut Random,
) -> ([f32; 3], [f32; 3]) {
    if points.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    let at = ((points.len() as f32 * radius) as i32).clamp(0, points.len() as i32 - 1) as usize;
    let point = points[at];
    let stray = (angle_degrees / 180.0).clamp(0.0, 1.0);
    let toward = random.direction();
    (point.position, std::array::from_fn(|axis| (1.0 - stray) * point.normal[axis] + stray * toward[axis]))
}

impl H3EmissionShape {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "sprayer" => Self::Sprayer,
            "disc" => Self::Disc,
            "globe" => Self::Globe,
            "implode" => Self::Implode,
            "tube" => Self::Tube,
            "halo" => Self::Halo,
            "debris" => Self::Debris,
            "line" => Self::Line,
            "custom points" => Self::CustomPoints,
            "impact contour" => Self::ImpactContour,
            "impact area" => Self::ImpactArea,
            "breakable surface" => Self::Breakable,
            "cube" => Self::Cube,
            "cylinder" => Self::Cylinder,
            "unweighted line" => Self::UnweightedLine,
            "plane" => Self::Plane,
            // Reach's `particle_simulation_initialize_particle`, which needs
            // a boat or a jetwash simulation the preview does not run.
            "boat hull surface" | "jetwash" => return None,
            _ => return None,
        })
    }
}

/// `set_shader_update_state`'s three, each off without a physics controller.
#[derive(Debug)]
struct H3ParticleFlock {
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
struct H3ParticleCollider {
    elasticity: H3ParticleProperty,
    friction: H3ParticleProperty,
}

/// A Halo 3 or ODST emitter's `collider` movement, when it collides with
/// structure, the only thing the preview's ground stands for
/// (`update_particles_collider`): friction is parameter 0 and elasticity 1.
fn read_collider(
    source: &TagSource,
    emitter: TagStruct<'_>,
    flagged: impl Fn(&str) -> bool,
    seeds: &mut SeedAllocator,
) -> Option<H3ParticleCollider> {
    let movement = emitter.descend("particle movement").filter(|_| flagged("collide with structure"))?;
    let friction = movement_parameter(source, movement, "collider", 0, seeds)?;
    Some(H3ParticleCollider { elasticity: movement_parameter(source, movement, "collider", 1, seeds)?, friction })
}

/// A Halo 2 `particle_physics`' `collider` movement, its own or its
/// template's, if it collides with structure: friction is parameter 0 and
/// elasticity 1, as `sub_53DF20` reads them.
fn read_collider_of(source: &TagSource, movement: TagStruct<'_>, seeds: &mut SeedAllocator) -> Option<H3ParticleCollider> {
    let template = movement
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| load_referenced_tag_from_source(source, &path, "particle_physics", b"pmov").ok());
    let own = elements(movement, "movements");
    let held = template.as_ref().map(|tag| tag.root());
    let collides = |holder: Option<TagStruct<'_>>| {
        let flags = holder.and_then(|holder| holder.read_flag_names("flags")).unwrap_or_default();
        flags.iter().any(|(_, name)| name.starts_with("collide with structure"))
    };
    if !collides(Some(movement)) && !collides(held) {
        return None;
    }
    let movements = if own.is_empty() { held.map_or(Vec::new(), |root| elements(root, "movements")) } else { own };
    let entry = movements.into_iter().find(|entry| entry.read_enum_name("type").as_deref() == Some("collider"))?;
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
fn bounce(
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
const PUSH_OUT: f32 = 0.005;

/// A Halo 3 or ODST emitter's `swarm` movement, when its flags name one.
fn read_flock(
    source: &TagSource,
    emitter: TagStruct<'_>,
    flagged: impl Fn(&str) -> bool,
    seeds: &mut SeedAllocator,
) -> Option<H3ParticleFlock> {
    let movement = emitter.descend("particle movement").filter(|_| flagged("swarm"))?;
    let mut by_id = |id: i128| movement_parameter(source, movement, "swarm", id, seeds);
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
    fn update(
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
    fn on_card(&self, state: &H3ParticleState) -> [f32; 3] {
        let read = |property: &Option<H3ParticleProperty>| property.as_ref().map(|property| property.scalar(state));
        [
            read(&self.gravity).unwrap_or(1.0) * GRAVITY,
            read(&self.air_friction).unwrap_or(1.0),
            read(&self.rotational_friction).unwrap_or(0.0),
        ]
    }

    /// `c_particle_controller::update_particles_physics`.
    fn update(&self, particles: &mut [Particle], state: H3ParticleState, dt: f32) {
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
    fn update(&self, particles: &mut [Particle], base: H3ParticleState, home: [f32; 3], dt: f32) {
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
                    add(unit(toward).map(|axis| axis * scale));
                }
                let (mine, theirs) = (unit(particle.velocity), unit(*velocity));
                let weight = self.alignment_weight.scalar(&state);
                add(minus(theirs, mine).map(|axis| axis * weight));
            }
            let reach = self.cohesion_distance.scalar(&state);
            if reach > 0.0 {
                let toward = minus(home, particle.position);
                let weight = self.cohesion_weight.scalar(&state) * dot(toward, toward).sqrt() / reach;
                add(unit(toward).map(|axis| axis * weight));
            }
            let mut change = force.map(|axis| axis * dt);
            let most = self.maximum_acceleration.scalar(&state);
            if dot(change, change).sqrt() > most {
                change = unit(change).map(|axis| axis * most);
            }
            let eased: [f32; 3] = std::array::from_fn(|axis| {
                (particle.velocity[axis] + change[axis]) * 0.25 + particle.velocity[axis] * 0.75
            });
            let (low, high) = (self.minimum_velocity.scalar(&state), self.maximum_velocity.scalar(&state));
            let speed = dot(eased, eased).sqrt();
            particle.velocity = if speed > high {
                unit(eased).map(|axis| axis * high)
            } else if speed < low {
                unit(eased).map(|axis| axis * low)
            } else {
                eased
            };
        }
    }
}

#[derive(Debug)]
struct H3ParticlePhysics {
    gravity: Option<H3ParticleProperty>,
    air_friction: Option<H3ParticleProperty>,
    rotational_friction: Option<H3ParticleProperty>,
}

/// `e_particle_attachment_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H3ParticleTrigger {
    Birth,
    Death,
    Collision,
    /// Halo 2's `attached particle systems`: made with the particle, they
    /// follow it, and their age runs over its lifespan (`sub_5E0850`).
    Carried,
}

/// `emission_axis_enum`, for what turns about an axis of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H3ParticleAxis {
    Constant,
    Cone,
    Disc,
    Globe,
}

impl H3ParticleAxis {
    fn named(name: &str) -> Self {
        match name {
            "cone" => Self::Cone,
            "disc" => Self::Disc,
            "globe" => Self::Globe,
            _ => Self::Constant,
        }
    }

    /// `initialize_particle`'s, about `reference` of x, y and z.
    fn pick(self, reference: usize, angle: f32, random: &mut Random) -> [f32; 3] {
        let [i, j, k] = match self {
            Self::Constant => [1.0, 0.0, 0.0],
            Self::Cone => {
                let cone = random.range(0.0, angle.to_radians());
                let turn = random.range(0.0, std::f32::consts::TAU);
                [cone.cos(), turn.sin() * cone.sin(), turn.cos() * cone.sin()]
            }
            Self::Disc => {
                let turn = random.range(0.0, std::f32::consts::TAU);
                [0.0, turn.sin(), turn.cos()]
            }
            Self::Globe => random.direction(),
        };
        match reference {
            1 => [k, i, j],
            2 => [j, k, i],
            _ => [i, j, k],
        }
    }
}

#[derive(Debug)]
struct H3Emitter {
    update: H3ParticleUpdate,
    /// On the card, Reach's and Halo 4's self-acceleration is a straight lerp
    /// between its ends (Reach's `map_to_vector3d_range` has its slerp
    /// commented out; Halo 4's `ParticleMapToVector3dLerp`); Halo 3's and
    /// ODST's turn from one to the other.
    card_lerps: bool,
    /// `becomes gpu when at rest`: a particle that stops moves on to the card.
    card_at_rest: bool,
    /// What a pulse's count may fall short of a whole particle by and still
    /// make one: Halo 2's `c_particle_emitter::pulse` (0x68d160) spawns while
    /// `count + 0.0001 >= 1`; Halo 3's floors it.
    spawn_slack: f32,
    controllers: Vec<H3ParticleController>,
    shape: H3EmissionShape,
    points: Vec<H3CustomPoint>,
    scroll: [f32; 2],
    axis: H3ParticleAxis,
    reference_axis: usize,
    axis_angle: H3ParticleProperty,
    offset: H3ParticleProperty,
    direction: H3ParticleProperty,
    radius: H3ParticleProperty,
    angle: H3ParticleProperty,
    starting: H3ParticleProperty,
    most: H3ParticleProperty,
    rate: H3ParticleProperty,
    lifespan: H3ParticleProperty,
    acceleration: H3ParticleProperty,
    speed: H3ParticleProperty,
    rotation: H3ParticleProperty,
    rotation_rate: H3ParticleProperty,
    size: H3ParticleProperty,
    scale: H3ParticleProperty,
    tint: H3ParticleProperty,
    alpha: H3ParticleProperty,
    black_point: H3ParticleProperty,
    physics: Option<H3ParticlePhysics>,
    flock: Option<H3ParticleFlock>,
    collider: Option<H3ParticleCollider>,
    /// Halo 2's `looping particle`: another only when none are left.
    one_at_a_time: bool,
    /// Halo 2's `persistent particle`: age comes round, and nothing dies of it.
    persistent: bool,
    /// Without Halo 2's `spread between ticks`, a tick's all start at its end.
    bunched: bool,
    /// Halo 2's rotation is all in `rotation`, and what spins it reads the
    /// `particle rotation` input.
    rotation_given: bool,
    /// Reach's `volume emitter particle velocities are random`.
    volume_random: bool,
    /// Reach's `axis scale`; ones elsewhere.
    axis_scale: [f32; 3],
    /// Reach's `turbulence` movement, which only the card runs.
    turbulence: Option<H3ParticleTurbulence>,
    /// Halo 4's own update terms.
    h4: Option<H4EmitterTerms>,
}

/// What Halo 4's `particle_update.fx` adds over Reach's: its system's
/// `particles live forever` and `disable velocity`, its `size scale`
/// (`m_scaleMultiplier`), and the emitter's scale x and y and alpha white
/// point.
#[derive(Debug)]
struct H4EmitterTerms {
    live_forever: bool,
    disable_velocity: bool,
    size_scale: f32,
    scale: Option<[H3ParticleProperty; 2]>,
    white_point: Option<H3ParticleProperty>,
}

impl H4EmitterTerms {
    /// `liveForever`: the age comes round, and from just short of 1 back to 0.
    fn age(&self, age: f32) -> f32 {
        if !self.live_forever {
            return age;
        }
        let age = age.fract();
        if age > 0.99 { 0.0 } else { age }
    }

    /// The step a frame moves a particle along its velocity.
    fn travel(&self, dt: f32) -> f32 {
        if self.disable_velocity { 0.0 } else { dt * self.size_scale }
    }

    /// `m_size`'s and `m_aspect`'s factors: scale y and the size scale, and
    /// scale x over scale y.
    fn size_and_aspect(&self, state: &H3ParticleState) -> [f32; 2] {
        let [x, y] = self.scale.as_ref().map_or([1.0; 2], |[x, y]| [x.scalar(state), y.scalar(state)]);
        let aspect = if y != 0.0 { x / y } else { 1.0 };
        [y * self.size_scale, aspect]
    }
}

/// A Halo 3 or ODST emitter's `physics` movement, when its flags name one:
/// gravity is parameter 0, air friction 1 and rotational friction 2.
fn read_physics(
    source: &TagSource,
    emitter: TagStruct<'_>,
    flagged: impl Fn(&str) -> bool,
    seeds: &mut SeedAllocator,
) -> Option<H3ParticlePhysics> {
    let movement = emitter.descend("particle movement").filter(|_| flagged("physics"))?;
    let mut parameter = |id: i128| movement_parameter(source, movement, "physics", id, seeds);
    Some(H3ParticlePhysics { gravity: parameter(0), air_friction: parameter(1), rotational_friction: parameter(2) })
}

/// A Halo 2 `particle_physics`' physics movement, its own or its template's.
fn read_movement(source: &TagSource, movement: TagStruct<'_>, seeds: &mut SeedAllocator) -> Option<H3ParticlePhysics> {
    let template = movement
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| load_referenced_tag_from_source(source, &path, "particle_physics", b"pmov").ok());
    let own = elements(movement, "movements");
    let movements = if own.is_empty() { template.as_ref().map_or(Vec::new(), |tag| elements(tag.root(), "movements")) } else { own };
    let physics = movements.into_iter().find(|entry| entry.read_enum_name("type").as_deref() == Some("physics"))?;
    let by_id = |id: i128, seeds: &mut SeedAllocator| {
        let parameters = elements(physics, "parameters");
        let found = parameters.into_iter().find(|parameter| parameter.read_int_any("parameter id") == Some(id))?;
        H3ParticleProperty::named(found, "property", seeds)
    };
    Some(H3ParticlePhysics {
        gravity: by_id(0, seeds),
        air_friction: by_id(1, seeds),
        rotational_friction: by_id(2, seeds),
    })
}

/// An emitter's properties, in tag order.
const EMITTER_PROPERTIES: [&str; 18] = [
    "translational offset",
    "relative direction",
    "emission radius",
    "emission angle",
    "emission axis angle",
    "particle starting count",
    "particle max count",
    "particle emission rate",
    "particle lifespan",
    "particle self-acceleration",
    "particle initial velocity",
    "particle rotation",
    "particle initial rotation rate",
    "particle size",
    "particle scale",
    "particle tint",
    "particle alpha",
    "particle alpha black point",
];

/// The order an emitter's properties take their seeds in, by
/// [`EMITTER_PROPERTIES`]: Reach's `effect_postprocess` (tool 0x140DD5070)
/// goes in tag order, and Halo 3's and ODST's allocate `emission axis angle`
/// last.
fn emitter_seed_order(reach: bool) -> [usize; 18] {
    std::array::from_fn(|at| match at {
        _ if reach => at,
        0..4 => at,
        4..17 => at + 1,
        _ => 4,
    })
}

/// A Halo 3, ODST or Reach emitter of `look`'s particles; `system_flagged`
/// reads its system's flags.
fn read_emitter(
    source: &TagSource,
    emitter: TagStruct<'_>,
    system_flagged: impl Fn(&str) -> bool,
    look: &H3ParticleLook,
    size_scale: f32,
) -> Option<H3Emitter> {
    // Halo 4 seeds and moves as Reach does, with terms of its own.
    let halo4 = source.game() == Some(GameId::Halo4);
    let reach = source.game() == Some(GameId::HaloReach) || halo4;
    let shape = H3EmissionShape::named(&emitter.read_enum_name("emission shape").unwrap_or_default())?;
    let mut properties = unallocated_properties(emitter, EMITTER_PROPERTIES)?;
    let movement_flags = emitter.descend("particle movement").and_then(|movement| movement.read_flag_names("flags"));
    let movement_flagged = |wanted: &str| movement_flags.iter().flatten().any(|(_, name)| name.starts_with(wanted));
    let (live_forever, disable_velocity) = (system_flagged("particles live forever"), system_flagged("disable velocity"));
    let (update, card_at_rest) = particle_update(system_flagged, look.any_attachments, movement_flagged);
    let controllers = flagged_controllers(movement_flagged);
    let (physics, flock, collider, turbulence);
    if reach {
        // `c_particle_controller::fully_index_parameters_and_postprocess`
        // (tool 0x140DD92D0): each controller first, with a mask of its own.
        // The preview reads no wind, nor swarm's parameter 5, so what those
        // take is not counted.
        let mut seeds = [(); 4].map(|_| SeedAllocator::reach_controller());
        physics = read_physics(source, emitter, movement_flagged, &mut seeds[0]);
        flock = read_flock(source, emitter, movement_flagged, &mut seeds[1]);
        collider = read_collider(source, emitter, movement_flagged, &mut seeds[2]);
        turbulence = read_turbulence(source, emitter, movement_flagged, &mut seeds[3]);
        // `get_used_particle_states` (0x140DD9B40) of the movement, the
        // emitter's own postprocess (0x140AE36E0), then its particle's.
        let movement_used = seeds.iter().fold(0, |used, seeds| used | seeds.used());
        let own = (movement_used | used_states_of(&properties)) & REACH_ALLOCATED_STATES;
        let mut seeds = SeedAllocator::reach(own | look.used_states);
        for at in emitter_seed_order(true) {
            properties[at].allocate(&mut seeds);
        }
    } else {
        let mut seeds = SeedAllocator::halo3();
        for at in emitter_seed_order(false) {
            properties[at].allocate(&mut seeds);
        }
        physics = read_physics(source, emitter, movement_flagged, &mut seeds);
        flock = read_flock(source, emitter, movement_flagged, &mut seeds);
        collider = read_collider(source, emitter, movement_flagged, &mut seeds);
        turbulence = None;
    }
    let [
        offset,
        direction,
        radius,
        angle,
        axis_angle,
        starting,
        most,
        rate,
        lifespan,
        acceleration,
        speed,
        rotation,
        rotation_rate,
        size,
        scale,
        tint,
        alpha,
        black_point,
    ] = properties;
    let scroll = emitter.read_vec2("uv scrolling");
    let emitter_flags = emitter.read_flag_names("emitter flags").unwrap_or_default();
    let raw_scale = emitter.read_point3d("axis scale");
    Some(H3Emitter {
        update,
        card_lerps: reach,
        card_at_rest,
        spawn_slack: 0.0,
        axis: H3ParticleAxis::named(&emitter.read_enum_name("particle axis (for models)").unwrap_or_default()),
        reference_axis: match emitter.read_enum_name("particle reference axis").as_deref() {
            Some("y") => 1,
            Some("z") => 2,
            _ => 0,
        },
        axis_angle,
        scroll: [scroll.i, scroll.j],
        controllers,
        points: read_custom_points(source, emitter),
        shape, offset, direction, radius, angle, starting, most, rate, lifespan, acceleration, speed,
        rotation, rotation_rate, size, scale, tint, alpha, black_point, physics, flock, collider,
        one_at_a_time: false,
        persistent: false,
        bunched: false,
        rotation_given: false,
        volume_random: reach
            && emitter_flags.iter().any(|(_, name)| name.starts_with("volume emitter particle velocities are random")),
        axis_scale: if reach { reach_axis_scale([raw_scale.x, raw_scale.y, raw_scale.z]) } else { [1.0; 3] },
        turbulence,
        h4: halo4.then(|| {
            // Read past Reach's allocation [inferred: their seeds follow].
            let property = |name: &str| emitter.descend(name).and_then(|holder| H3ParticleProperty::unallocated(holder));
            let scale = property("particle scale x").zip(property("particle scale y")).map(|(x, y)| [x, y]);
            H4EmitterTerms {
                live_forever,
                disable_velocity,
                // `c_particle_system_definition::postprocess_block_proc`.
                size_scale: if size_scale < 1e-4 { 1.0 } else { size_scale },
                scale,
                white_point: property("particle alpha white point"),
            }
        }),
    })
}

/// Reach's `turbulence` movement, which `particle_update.hlsl` runs on the
/// card: after a particle moves and before it accelerates, it is pushed by a
/// texel of the movement's texture, picked by its birth time along u and its
/// fifth seed along v.
#[derive(Debug)]
struct H3ParticleTurbulence {
    /// Parameter 0, read for each particle: `_index_emitter_movement_turbulence`,
    /// from `get_parameter_by_id_indexed(turbulence, 0)` in `populate_gpu_data`
    /// (the 360's 0x82EE8350).
    strength: H3ParticleProperty,
    /// `c_particle_emitter_gpu::frame_advance<update>` (the 360's 0x826CAF88):
    /// parameters 3, 4, 1 and 2, read once a frame at the system's states, as
    /// u's and v's scale and then their offsets.
    xform: [H3ParticleProperty; 4],
    /// The movement's `turbulence texture`, decoded once.
    image: TextureImage,
}

/// What tool's `postprocess_internal` gives a turbulence movement that, nor
/// its template, names a texture.
const DEFAULT_TURBULENCE: &str = "fx\\bitmaps\\turbulence\\default_turbulence";

/// A Reach emitter's `turbulence` movement, when its flags name one. Tool's
/// table (0x1427E4208) fills in a missing parameter 0 with 1, 1 and 2 with 0,
/// 3 with 1 and 4 with 0.3.
fn read_turbulence(
    source: &TagSource,
    emitter: TagStruct<'_>,
    flagged: impl Fn(&str) -> bool,
    seeds: &mut SeedAllocator,
) -> Option<H3ParticleTurbulence> {
    let movement = emitter.descend("particle movement").filter(|_| flagged("turbulence"))?;
    let mut parameter = |id: i128, default: f32| movement_parameter_or(source, movement, "turbulence", id, default, seeds);
    let strength = parameter(0, 1.0)?;
    let defaults = [(1, 0.0), (2, 0.0), (3, 1.0), (4, 0.3)];
    let [u_offset, v_offset, u_scale, v_scale] = defaults.map(|(id, default)| parameter(id, default));
    let named = |holder: TagStruct<'_>| holder.read_tag_ref_path("turbulence texture").filter(|path| !path.is_empty());
    let template = movement
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| load_referenced_tag_from_source(source, &path, "particle_physics", b"pmov").ok());
    let path = named(movement)
        .or_else(|| template.as_ref().and_then(|tag| named(tag.root())))
        .unwrap_or_else(|| DEFAULT_TURBULENCE.to_owned());
    let image = match decode_h2_image(source, &path, SamplerKind::Texture2D, 0)? {
        H2Image::Flat(image) if image.whole() && image.width > 0 && image.height > 0 => image,
        _ => return None,
    };
    Some(H3ParticleTurbulence { strength, xform: [u_scale?, v_scale?, u_offset?, v_offset?], image })
}

impl H3ParticleTurbulence {
    fn xform(&self, state: &H3ParticleState) -> [f32; 4] {
        self.xform.each_ref().map(|property| property.scalar(state))
    }
}

/// `particle_update.hlsl`'s push: the texel at `(birth time, seed 5) · xform.xy
/// + xform.zw`, less a half, times `scale` (the strength over the frame).
fn turbulence_push(image: &TextureImage, xform: [f32; 4], [birth, seed]: [f32; 2], scale: f32) -> [f32; 3] {
    let uv = [birth * xform[0] + xform[2], seed * xform[1] + xform[3]];
    sample_wrapped(image, uv).map(|channel| (channel - 0.5) * scale)
}

/// `tex2Dlod` at level 0 on the sampler the turbulence is bound with
/// (vertex sampler 22): address mode 0, wrap, and filter 2, bilinear
/// [inferred: Baboon reads those values so elsewhere].
fn sample_wrapped(image: &TextureImage, [u, v]: [f32; 2]) -> [f32; 3] {
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

/// A Halo 2 emitter: Halo 3's without what it added, its rates in ticks and
/// degrees, and `size` and `rotation` scaled by the `.particle`'s.
fn read_h2_emitter(
    source: &TagSource,
    emitter: TagStruct<'_>,
    scale: H3ParticleProperty,
    rotation: H3ParticleProperty,
    flagged: &impl Fn(&str) -> bool,
) -> Option<H3Emitter> {
    let mut seeds = SeedAllocator::default();
    let shape = H3EmissionShape::named(&emitter.read_enum_name("emission shape").unwrap_or_default())?;
    let mut property = |name: &str| H3ParticleProperty::named(emitter, name, &mut seeds);
    let (radius, angle, rate, lifespan) = (
        property("emission radius")?,
        property("emission angle")?,
        property("particle emission rate")?.scaled(30.0),
        property("particle lifespan")?,
    );
    let (speed, rotation_rate, size, tint, alpha) = (
        property("particle velocity")?,
        property("particle angular velocity")?.scaled(1.0 / 360.0),
        property("particle size")?,
        property("particle tint")?,
        property("particle alpha")?,
    );
    let offset = emitter.read_point3d("translational offset");
    let turn = emitter.read_euler2d("relative direction");
    let (sin_yaw, cos_yaw) = turn.yaw.sin_cos();
    let (sin_pitch, cos_pitch) = turn.pitch.sin_cos();
    let movement = emitter
        .read_tag_ref_path("particle physics")
        .filter(|path| !path.is_empty())
        .and_then(|path| load_referenced_tag_from_source(source, &path, "particle_physics", b"pmov").ok());
    let physics = movement.as_ref().and_then(|tag| read_movement(source, tag.root(), &mut seeds));
    let collider = movement.as_ref().and_then(|tag| read_collider_of(source, tag.root(), &mut seeds));
    // In the order the tag lists them, as the game runs its movements.
    let controllers = movement.as_ref().map_or_else(Vec::new, |tag| h2_controllers(source, tag.root()));
    let controllers = controllers
        .into_iter()
        .filter(|controller| match controller {
            H3ParticleController::Physics => physics.is_some(),
            H3ParticleController::Collider => collider.is_some(),
            H3ParticleController::Flock => false,
        })
        .collect();
    Some(H3Emitter {
        h4: None,
        update: H3ParticleUpdate::Cpu,
        card_lerps: false,
        card_at_rest: false,
        spawn_slack: 0.0001,
        controllers,
        shape,
        points: Vec::new(),
        scroll: [0.0; 2],
        axis: H3ParticleAxis::named(""),
        reference_axis: 0,
        axis_angle: H3ParticleProperty::fixed(0.0)?,
        offset: H3ParticleProperty::fixed_vector([offset.x, offset.y, offset.z])?,
        direction: H3ParticleProperty::fixed_vector([cos_yaw * cos_pitch, sin_yaw * cos_pitch, sin_pitch])?,
        radius,
        angle,
        starting: H3ParticleProperty::fixed(0.0)?,
        most: H3ParticleProperty::fixed(0.0)?,
        rate,
        lifespan,
        acceleration: H3ParticleProperty::fixed_vector([0.0; 3])?,
        speed,
        rotation,
        rotation_rate,
        size,
        scale,
        tint,
        alpha,
        black_point: H3ParticleProperty::fixed(0.0)?,
        physics,
        flock: None,
        collider,
        one_at_a_time: flagged("looping particle"),
        persistent: flagged("persistent particle"),
        bunched: !flagged("spread between ticks"),
        rotation_given: true,
        volume_random: false,
        axis_scale: [1.0; 3],
        turbulence: None,
    })
}

/// A system in an event of an attached effect, at a marker.
#[derive(Debug)]
pub(crate) struct H3ParticleSystem {
    node: usize,
    position: [f32; 3],
    axes: [[f32; 3]; 3],
    only_under: Option<(String, String)>,
    pub(crate) object: u8,
    scale_by: String,
    secondary_by: String,
    sort_bias: f32,
    /// `coordinate system`: particles left where they were made.
    world: bool,
    location_seed: f32,
    near_range: f32,
    near_cutoff: f32,
    times: Arc<H3EffectTimes>,
    event: usize,
    pub(crate) look: Arc<H3ParticleLook>,
    emitters: Vec<Arc<H3Emitter>>,
    /// What its particles start: how far along the list each system lies.
    attached: Vec<(H3ParticleTrigger, usize)>,
    /// Started by a particle, at the particle: nothing of its own runs it.
    child: bool,
    /// `use parent position but world orientation`: its axes do not turn.
    world_oriented: bool,
    /// Contrails its particles start, each following the particle.
    carried: Vec<Arc<H3StripAt>>,
    hidden: bool,
    die_with_effect: bool,
    lod: Option<[f32; 4]>,
}

/// What a tag's `particle system` holds, wherever it ends up.
#[derive(Clone)]
struct H3SystemParts {
    /// `don't render system`: it still runs, for what its particles start.
    hidden: bool,
    /// `particles die when effect ends`.
    die_with_effect: bool,
    lod: Option<[f32; 4]>,
    sort_bias: f32,
    world: bool,
    near_range: f32,
    near_cutoff: f32,
    look: Arc<H3ParticleLook>,
    emitters: Vec<Arc<H3Emitter>>,
}

fn read_system_parts(source: &TagSource, system: TagStruct<'_>) -> Option<H3SystemParts> {
    let flags = system.read_flag_names("flags").unwrap_or_default();
    let flagged = |wanted: &str| flags.iter().any(|(_, name)| name.starts_with(wanted));
    if !part_runs_by(system, "environment", "disposition") {
        return None;
    }
    let path = system.read_tag_ref_path("particle").filter(|path| !path.is_empty())?;
    let h2 = source.game() == Some(GameId::Halo2);
    let (look, emitters): (Arc<H3ParticleLook>, Vec<_>) = if h2 {
        let (look, scale, rotation) = read_h2_look(source, &path)?;
        let read = |emitter: &TagStruct<'_>| read_h2_emitter(source, *emitter, scale.clone(), rotation.clone(), &flagged);
        (Arc::new(look), elements(system, "emitters").iter().filter_map(read).map(Arc::new).collect())
    } else {
        let look = read_look(source, &path).map(Arc::new)?;
        let size_scale = system.read_real("size scale").unwrap_or(1.0);
        let read = |emitter: &TagStruct<'_>| read_emitter(source, *emitter, flagged, &look, size_scale).map(Arc::new);
        let emitters = elements(system, "emitters").iter().filter_map(read).collect();
        (look, emitters)
    };
    if emitters.is_empty() {
        return None;
    }
    let real = |field: &str| system.read_real(field).unwrap_or(0.0);
    let cutoff = if flagged("override near fade") { real("near fade override") } else { real("near fade cutoff") };
    Some(H3SystemParts {
        hidden: flagged("don't render system"),
        die_with_effect: flagged("particles die when effect ends"),
        lod: {
            let raw = [
                real("LOD in distance"),
                real("LOD out distance"),
                real("LOD feather in delta"),
                real("LOD feather out delta"),
            ];
            let any_camera = system.read_int_any("camera mode") == Some(0);
            let rates = emitters.iter().map(|emitter| emitter.rate.most());
            if h2 {
                Some(h2_baked_lod(raw, any_camera, rates))
            } else if !(flagged("lod always 1.0") || flagged("ignore lod")) {
                Some(if look.reach { reach_baked_lod(raw) } else { baked_lod(raw, any_camera, rates) })
            } else {
                None
            }
        },
        sort_bias: system.read_int_any("sort bias").unwrap_or(0) as f32,
        world: system.read_enum_name("coordinate system").as_deref() == Some("world"),
        near_range: near_fade_range(look.reach, real("near fade range")),
        near_cutoff: cutoff,
        look,
        emitters,
    })
}

/// The systems of the effects a look's particles start, and their events.
fn read_attached_parts(
    source: &TagSource,
    look: &H3ParticleLook,
) -> Vec<(H3ParticleTrigger, H3SystemParts, Arc<H3EffectTimes>, usize)> {
    let mut parts = Vec::new();
    for (trigger, path) in &look.attachments {
        let Ok(effect) = load_referenced_tag_from_source(source, path, "effect", b"effe") else {
            continue;
        };
        let times = Arc::new(H3EffectTimes::read(effect.root()));
        for (at, event) in elements(effect.root(), "events").into_iter().enumerate() {
            for system in elements(event, "particle systems") {
                if let Some(read) = read_system_parts(source, system) {
                    parts.push((*trigger, read, Arc::clone(&times), at));
                }
            }
        }
    }
    let carried = look.carried_systems.as_ref();
    if let Some(tag) = carried.and_then(|path| load_referenced_tag_from_source(source, path, "particle", b"prt3").ok()) {
        let lasting = Arc::new(H3EffectTimes::lasting());
        for system in elements(tag.root(), "attached particle systems") {
            if let Some(read) = read_system_parts(source, system) {
                parts.push((H3ParticleTrigger::Carried, read, Arc::clone(&lasting), 0));
            }
        }
    }
    parts
}

/// The contrails these systems' particles start, each once: a system at many
/// markers shares its own.
pub(super) fn carried_strips(systems: &[Arc<H3ParticleSystem>]) -> Vec<Arc<H3StripAt>> {
    let mut strips: Vec<Arc<H3StripAt>> = Vec::new();
    for strip in systems.iter().flat_map(|system| &system.carried) {
        if !strips.iter().any(|known| Arc::ptr_eq(known, strip)) {
            strips.push(Arc::clone(strip));
        }
    }
    strips
}

impl H3ParticleSystem {
    pub(crate) fn only_under(&self) -> Option<&(String, String)> {
        self.only_under.as_ref()
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    pub(crate) fn scale_names(&self) -> [&str; 2] {
        [&self.scale_by, &self.secondary_by]
    }

    pub(crate) fn on_attached(mut self, name: &str) -> Self {
        adopt_event_function(name, &mut self.scale_by, &mut self.object);
        if let Some((region, _)) = &mut self.only_under {
            *region = format!("{name}: {region}");
        }
        self
    }
}

/// The systems of an object's attached effects, one at each marker of a location.
pub(super) fn read_h3_particles(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H3ParticleSystem> {
    let mut systems = Vec::new();
    for object_effect in h3_object_effects(source, object) {
        let Ok(effect) = load_referenced_tag_from_source(source, &object_effect.path, "effect", b"effe") else {
            continue;
        };
        let locations = elements(effect.root(), "locations");
        let times = Arc::new(object_effect.times_of(effect.root()));
        let world_oriented = H3ObjectEffect::world_oriented(effect.root());
        for (at, event) in elements(effect.root(), "events").into_iter().enumerate() {
            for system in elements(event, "particle systems") {
                let location = usize::try_from(system.read_block_index("location")).ok();
                let Some(location) = location.and_then(|location| locations.get(location)) else {
                    continue;
                };
                let name = object_effect.marker_of(*location);
                let Some(parts) = read_system_parts(source, system) else {
                    continue;
                };
                let children = read_attached_parts(source, &parts.look);
                let carried: Vec<_> = parts
                    .look
                    .attachments
                    .iter()
                    .filter(|(trigger, _)| *trigger == H3ParticleTrigger::Birth)
                    .flat_map(|(_, path)| read_carried_strips(source, path))
                    .map(Arc::new)
                    .collect();
                for marker in &object_effect.locations(markers, &name, &frame) {
                    let (place, node) = frame(marker);
                    let attached = children.iter().enumerate().map(|(at, child)| (child.0, at + 1)).collect();
                    let parts = parts.clone();
                    systems.push(H3ParticleSystem {
                        node,
                        position: place.position,
                        axes: place.axes,
                        only_under: marker.only_under.clone(),
                        object: owner,
                        scale_by: object_effect.scale_by.clone(),
                        secondary_by: object_effect.secondary_by.clone(),
                        sort_bias: parts.sort_bias,
                        world: parts.world,
                        location_seed: (systems.len() as f32 * 0.618_034).fract(),
                        near_range: parts.near_range,
                        near_cutoff: parts.near_cutoff,
                        times: Arc::clone(&times),
                        event: at,
                        look: parts.look,
                        emitters: parts.emitters,
                        attached,
                        child: false,
                        world_oriented,
                        carried: carried.clone(),
                        hidden: parts.hidden,
                        die_with_effect: parts.die_with_effect,
                        lod: parts.lod,
                    });
                    for (_, parts, times, event) in &children {
                        let parts = parts.clone();
                        systems.push(H3ParticleSystem {
                            node,
                            position: place.position,
                            axes: place.axes,
                            only_under: marker.only_under.clone(),
                            object: owner,
                            scale_by: String::new(),
                            secondary_by: String::new(),
                            sort_bias: parts.sort_bias,
                            world: parts.world,
                            location_seed: (systems.len() as f32 * 0.618_034).fract(),
                            near_range: parts.near_range,
                            near_cutoff: parts.near_cutoff,
                            times: Arc::clone(times),
                            event: *event,
                            look: parts.look,
                            emitters: parts.emitters,
                            attached: Vec::new(),
                            child: true,
                            world_oriented: false,
                            carried: Vec::new(),
                            hidden: parts.hidden,
                            die_with_effect: parts.die_with_effect,
                            lod: parts.lod,
                        });
                    }
                }
            }
        }
    }
    systems
}

/// `real_internal_random_range`'s generator, seeded so a preview repeats.
#[derive(Debug, Clone, Copy)]
pub(super) struct Random(pub(super) u32);

impl Random {
    pub(super) fn next(&mut self) -> f32 {
        self.bits() as f32 / 65_535.0
    }

    /// `_random`: the next state's high 16 bits, which [`Self::next`] scales.
    pub(super) fn bits(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0 >> 16
    }

    pub(super) fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }

    fn direction(&mut self) -> [f32; 3] {
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
fn emit(
    shape: H3EmissionShape,
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
        H3EmissionShape::Sprayer => ([0.0; 3], spread(random.range(0.0, angle))),
        H3EmissionShape::Disc => {
            let out = [0.0, sin_turn, cos_turn];
            (out.map(|axis| axis * radius), out)
        }
        H3EmissionShape::Globe => {
            let out = random.direction();
            (out.map(|axis| axis * radius), out)
        }
        H3EmissionShape::Implode => {
            let out = random.direction();
            (out.map(|axis| axis * radius), out.map(|axis| -axis))
        }
        H3EmissionShape::Tube => ([0.0, sin_turn * radius, cos_turn * radius], spread(angle)),
        H3EmissionShape::Halo => {
            let velocity = spread(random.range(0.0, angle));
            (random.direction().map(|axis| axis * radius), velocity)
        }
        H3EmissionShape::Debris => ([0.0, sin_turn * radius, cos_turn * radius], [0.0, 0.0, 1.0]),
        H3EmissionShape::Line | H3EmissionShape::UnweightedLine => {
            let cone = random.range(0.0, angle);
            ([0.0, random.range(-radius, radius), 0.0], spread(cone))
        }
        H3EmissionShape::Cube => {
            // Drawn z, y, x.
            let [z, y, x] = [(); 3].map(|_| random.range(-radius, radius));
            let place = [x, y, z];
            (place, volume_heading(volume_random, place, random))
        }
        H3EmissionShape::Cylinder => {
            // Two units long whatever the radius; `axis scale` stretches it.
            let along = random.range(-1.0, 1.0);
            let (sin, cos) = random.range(0.0, std::f32::consts::TAU).sin_cos();
            let out = random.range(0.0, 1.0) * radius;
            let place = [along, out * sin, out * cos];
            (place, volume_heading(volume_random, place, random))
        }
        H3EmissionShape::Plane => ([(); 3].map(|_| random.range(-1.0, 1.0) * radius), [1.0, 0.0, 0.0]),
        // An attached effect has no impact, whose size is all that spreads these.
        H3EmissionShape::ImpactContour => {
            let cone = angle;
            let sign = if random.next() < 0.5 { -1.0 } else { 1.0 };
            let side = cone.sin() * sign;
            ([0.0; 3], if random.next() < 0.5 { [cone.cos(), side, 0.0] } else { [cone.cos(), 0.0, side] })
        }
        H3EmissionShape::ImpactArea => {
            let cone = angle;
            let (sin, cos) = random.range(0.0, std::f32::consts::TAU).sin_cos();
            ([0.0; 3], [cone.cos(), sin * cone.sin(), cos * cone.sin()])
        }
        H3EmissionShape::Breakable | H3EmissionShape::CustomPoints => ([0.0; 3], [0.0; 3]),
    }
}

/// A Reach volume's heading, normalized later: out from its centre, or with
/// `volume_random` any way at all.
fn volume_heading(volume_random: bool, place: [f32; 3], random: &mut Random) -> [f32; 3] {
    if volume_random { [(); 3].map(|_| random.range(-1.0, 1.0)) } else { place }
}

/// Reach's emitter `axis scale`, which tool's postprocess makes ones when it
/// is all nothing.
fn reach_axis_scale(raw: [f32; 3]) -> [f32; 3] {
    if raw == [0.0; 3] { [1.0; 3] } else { raw }
}

#[derive(Debug, Clone)]
struct Particle {
    /// Unique among its emitter's, for what follows it.
    id: u64,
    /// In contact, so that a collision's attachments play once.
    colliding: bool,
    /// Touched a surface it could stand on this step.
    landed: bool,
    /// Slow enough on one to have stopped for good.
    resting: bool,
    /// Not yet through an update: the card's first after a spawn moves nothing.
    spawned_this_frame: bool,
    /// Moved from the processor to the card when it came to rest.
    on_card: bool,
    axis: [f32; 3],
    age: f32,
    inverse_lifespan: f32,
    position: [f32; 3],
    velocity: [f32; 3],
    rotation: f32,
    rotation_rate: f32,
    frame: f32,
    frame_rate: f32,
    seeds: [f32; SEEDS],
    emission_time: f32,
    flip: [bool; 3],
    /// Which of the look's `sequences` it draws.
    sequence: usize,
    system_seeds: [f32; SYSTEM_SEEDS],
}

#[derive(Debug, Clone)]
struct RunningEmitter {
    particles: Vec<Particle>,
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
pub(crate) struct H3ParticleField {
    time: Option<f32>,
    /// Per system, per emitter.
    running: Vec<Vec<RunningEmitter>>,
    /// Per system, what particles have started of it.
    instances: Vec<Vec<H3ChildInstance>>,
    /// Per system, where the particles something follows are.
    tracks: Vec<Vec<H3ParticleTrack>>,
}

/// Where a live particle is, for what is attached to it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct H3ParticleTrack {
    pub(crate) id: u64,
    pub(crate) place: [f32; 3],
    velocity: [f32; 3],
}

/// `effect_new_attached_from_particle`: one run of a system, attached to a
/// particle while that lasts. A death's has no particle left to follow.
#[derive(Debug, Clone)]
struct H3ChildInstance {
    follows: Option<u64>,
    parent: usize,
    origin: [f32; 3],
    axes: [[f32; 3]; 3],
    start: f32,
    running: Vec<RunningEmitter>,
    /// The effect A scale it was made with.
    effect_a: f32,
    /// Halo 2's carried systems: the lifespan of the particle that made it,
    /// over which its age runs to 1, and then it emits no more.
    lifetime: Option<f32>,
}

/// `particle_block.h`'s limit on systems.
const MOST_INSTANCES: usize = 320;

fn fresh_emitters(system: &H3ParticleSystem, seed: u32) -> Vec<RunningEmitter> {
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
fn axes_along(forward: [f32; 3]) -> [[f32; 3]; 3] {
    let forward = if dot(forward, forward) > 0.0 { unit(forward) } else { [1.0, 0.0, 0.0] };
    let reference = if forward[2].abs() < 0.99 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let left = unit(cross(reference, forward));
    [forward, left, cross(forward, left)]
}

/// A particle's birth, death or hit: its id, place and velocity.
type ParticleEvent = (u64, [f32; 3], [f32; 3]);

/// One step of one location, `dt` long, returning what its particles started
/// and ended.
#[allow(clippy::too_many_arguments)]
fn step_location(
    system: &H3ParticleSystem,
    emitters: &mut [RunningEmitter],
    state: Option<H3ParticleState>,
    (origin, axes): ([f32; 3], [[f32; 3]; 3]),
    now: f32,
    dt: f32,
    ground: f32,
    (births, deaths, hits): (&mut Vec<ParticleEvent>, &mut Vec<ParticleEvent>, &mut Vec<ParticleEvent>),
    tracks: &mut Vec<H3ParticleTrack>,
) {
    tracks.clear();
    for (at, (emitter, running)) in system.emitters.iter().zip(emitters).enumerate() {
        // What was made outlives its event, and goes on falling and landing.
        let rest = H3ParticleState {
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
            (std::array::from_fn(|row| frame.1[row][2]), frame.0[2] - ground)
        };
        if let Some(mut state) = state {
            state.system_seeds = running.system_seeds;
            emitter.pulse(&system.look, state, running, dt, frame, system.world, births, at);
            let ground = Some((plane, system.look.dies_on_ground, &mut *hits));
            emitter.step(state, &mut running.particles, dt, home, deaths, ground, system.look.dies_at_rest);
        } else {
            // The next run of its event is a new location's, with its burst.
            running.started = false;
            running.previous = None;
            let ground = Some((plane, system.look.dies_on_ground, &mut *hits));
            emitter.step(rest, &mut running.particles, dt, home, deaths, ground, system.look.dies_at_rest);
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
                    (std::array::from_fn(|axis| offset[axis] + moved[axis]), to_world(particle.velocity))
                };
                H3ParticleTrack { id: particle.id, place, velocity }
            }));
        }
    }
}

/// The states of a system this step, or none while it should not emit.
fn system_state(
    system: &H3ParticleSystem,
    values: Option<&H2FunctionValues>,
    time: f32,
    since: f32,
) -> Option<H3ParticleState> {
    let read = |name: &str| {
        if name.is_empty() { 1.0 } else { values.and_then(|values| values.get(name)).copied().unwrap_or(0.0) }
    };
    let effect_a = read(&system.scale_by);
    if effect_a <= 0.0 {
        return None;
    }
    Some(H3ParticleState {
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
    system: &H3ParticleSystem,
    state: Option<H3ParticleState>,
    origin: [f32; 3],
    eye: [f32; 3],
) -> Option<H3ParticleState> {
    let away = minus(origin, eye);
    let lod = lod_amount(system.lod, dot(away, away).sqrt());
    state.filter(|_| lod > 0.0).map(|state| H3ParticleState { lod, ..state })
}

/// `system_state` for a step `dt` long ending at `time`, and for an event that
/// came and went within it: a frame can hold more than one of the game's
/// ticks, and its one-tick events are many.
fn system_state_stepped(
    system: &H3ParticleSystem,
    values: Option<&H2FunctionValues>,
    time: f32,
    dt: f32,
    onset: f32,
) -> Option<H3ParticleState> {
    let since = time - onset;
    system_state(system, values, time, since).or_else(|| {
        let inside = system.times.begins_within(system.event, since - dt, since)?;
        system_state(system, values, time, inside)
    })
}

impl H3Emitter {
    /// `initialize_particle`'s shape, then each axis of the place by `axis
    /// scale` (Reach's, after every shape; ones elsewhere).
    fn emitted(&self, radius: f32, angle: f32, random: &mut Random) -> ([f32; 3], [f32; 3]) {
        let (position, velocity) = if self.shape == H3EmissionShape::CustomPoints {
            emit_from_points(&self.points, radius, angle, random)
        } else {
            emit(self.shape, radius, angle, self.volume_random, random)
        };
        (std::array::from_fn(|axis| position[axis] * self.axis_scale[axis]), velocity)
    }

    /// `c_particle_emitter::pulse` and `spawn_particle`.
    fn pulse(
        &self,
        look: &H3ParticleLook,
        state: H3ParticleState,
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
            running.accumulator = running.accumulator.min(most - running.particles.len() as f32);
        }
        let (offset, turned) = frame;
        let previous = running.previous.unwrap_or(offset);
        running.previous = Some(offset);
        let count = (running.accumulator + self.spawn_slack).max(0.0).floor();
        let between = if count >= 1.0 { running.accumulator.recip() } else { 0.0 };
        // Each one spent comes off what is owed, which can leave it a hair
        // under nothing for the next pulse.
        running.accumulator -= count;
        for birth in 0..count as usize {
            if running.particles.len() >= MOST_PARTICLES {
                break;
            }
            let along = if self.bunched { 1.0 } else { birth as f32 * between };
            let random = &mut running.random;
            let seeds: [f32; SEEDS] = std::array::from_fn(|_| random.next());
            let mut particle_state = H3ParticleState { seeds, emission_time: state.system_age, ..state };
            particle_state.system_seeds = state.system_seeds;
            let (radius, angle) = (self.radius.scalar(&particle_state), self.angle.scalar(&particle_state));
            let (position, mut velocity) = self.emitted(radius, angle, random);
            let length = (velocity[0].powi(2) + velocity[1].powi(2) + velocity[2].powi(2)).sqrt().max(1e-6);
            let speed = self.speed.scalar(&particle_state);
            let speed = if speed == 0.0 && look.still_moves { 0.001 } else { speed };
            velocity = velocity.map(|axis| axis / length * speed);
            let to_world = |vector: [f32; 3]| -> [f32; 3] {
                std::array::from_fn(|axis| (0..3).map(|at| turned[at][axis] * vector[at]).sum())
            };
            let placed = {
                let moved = to_world(position);
                std::array::from_fn(|axis| previous[axis] + (offset[axis] - previous[axis]) * along + moved[axis])
            };
            running.made += 1;
            let id = running.made << 8 | at as u64;
            started.push((id, placed, to_world(velocity)));
            let ahead = (1.0 - along) * dt;
            let (position, velocity) = if world {
                (placed, to_world(velocity))
            } else {
                (std::array::from_fn(|axis| position[axis] + velocity[axis] * ahead), velocity)
            };
            let inverse_lifespan = 1.0 / self.lifespan.scalar(&particle_state).max(0.01);
            let mut flip = [random.next() >= 0.5, random.next() >= 0.5, random.next() >= 0.5];
            // One random word gives both: `& 1` mirrors v, `% count` the sequence.
            let sequence = if look.sequences.is_empty() {
                0
            } else {
                let word = (random.next() * 65536.0) as usize;
                flip[1] = word & 1 != 0;
                word % look.sequences.len()
            };
            let axis = self.axis.pick(self.reference_axis, self.axis_angle.scalar(&particle_state), random);
            running.particles.push(Particle {
                id,
                colliding: false,
                landed: false,
                resting: false,
                spawned_this_frame: true,
                on_card: false,
                axis,
                age: if self.bunched { 0.0 } else { along * dt * inverse_lifespan },
                inverse_lifespan,
                position,
                velocity,
                rotation: starting_rotation(look.random_rotation, look.h4.is_some() && look.vertical, || random.next()),
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
    fn step(
        &self,
        state: H3ParticleState,
        particles: &mut Vec<Particle>,
        dt: f32,
        home: [f32; 3],
        ended: &mut Vec<(u64, [f32; 3], [f32; 3])>,
        ground: Option<(([f32; 3], f32), bool, &mut Vec<(u64, [f32; 3], [f32; 3])>)>,
        dies_at_rest: bool,
    ) {
        if self.update == H3ParticleUpdate::Gpu {
            return self.step_on_card(state, particles, dt, ended);
        }
        let mut ground = ground;
        for controller in &self.controllers {
            match controller {
                H3ParticleController::Flock => {
                    if let Some(flock) = &self.flock {
                        flock.update(particles, state, home, dt);
                    }
                }
                H3ParticleController::Collider => {
                    if let (Some(collider), Some((plane, dies, hits))) = (&self.collider, ground.as_mut()) {
                        collider.update(particles, state, *plane, *dies, hits, dt);
                    }
                }
                H3ParticleController::Physics => {
                    if let Some(physics) = &self.physics {
                        physics.update(particles, state, dt);
                    }
                }
            }
        }
        // `c_particle_emitter::frame_advance`. The processor's turbulence
        // (`update_particles_turbulence`, the 360's 0x82EE0D80) does nothing;
        // what is on the card has the card's.
        let turbulence = self.turbulence.as_ref().map(|turbulence| turbulence.xform(&state));
        particles.retain_mut(|particle| {
            if particle.on_card {
                // `set_shader_update_state` gives a processor emitter's no physics.
                return self.card_frame(particle, state, dt, [0.0; 3], turbulence, ended);
            }
            particle.age += particle.inverse_lifespan * dt;
            if self.persistent {
                particle.age = particle.age.fract();
            }
            if let Some(h4) = &self.h4 {
                particle.age = h4.age(particle.age);
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
            let travel = self.h4.as_ref().map_or(dt, |h4| h4.travel(dt));
            for axis in 0..3 {
                particle.position[axis] += particle.velocity[axis] * travel;
            }
            particle.rotation = (particle.rotation + particle.rotation_rate * dt).rem_euclid(1.0);
            particle.frame = (particle.frame + particle.frame_rate * dt).rem_euclid(1.0);
            true
        });
    }
}

impl H3Emitter {
    /// `particle_update.hlsl`'s `update_particle_state`, for an emitter on the
    /// card: a particle's first update after its spawn moves nothing; then it
    /// ages, moves by the velocity it had, and only then is accelerated. Its
    /// physics is the emitter's, and none without the `physics` movement.
    fn step_on_card(
        &self,
        state: H3ParticleState,
        particles: &mut Vec<Particle>,
        dt: f32,
        ended: &mut Vec<ParticleEvent>,
    ) {
        let physical = self.controllers.contains(&H3ParticleController::Physics);
        let physics = self.physics.as_ref().filter(|_| physical);
        let constants = physics.map_or([0.0; 3], |physics| physics.on_card(&state));
        let turbulence = self.turbulence.as_ref().map(|turbulence| turbulence.xform(&state));
        particles.retain_mut(|particle| self.card_frame(particle, state, dt, constants, turbulence, ended));
    }

    /// One frame of `update_particle_state` for one particle, with the
    /// emitter's gravity, air friction and rotational friction, and Reach's
    /// turbulence at this frame's `xform`; false once it dies.
    fn card_frame(
        &self,
        particle: &mut Particle,
        state: H3ParticleState,
        dt: f32,
        [gravity, air_friction, rotational_friction]: [f32; 3],
        turbulence: Option<[f32; 4]>,
        ended: &mut Vec<ParticleEvent>,
    ) -> bool {
        let dt = if std::mem::take(&mut particle.spawned_this_frame) { 0.0 } else { dt };
        particle.age += particle.inverse_lifespan * dt;
        if let Some(h4) = &self.h4 {
            particle.age = h4.age(particle.age);
        }
        if particle.age >= 1.0 {
            ended.push((particle.id, particle.position, particle.velocity));
            return false;
        }
        let (position, velocity) = (particle.position, particle.velocity);
        let travel = self.h4.as_ref().map_or(dt, |h4| h4.travel(dt));
        particle.position = std::array::from_fn(|axis| position[axis] + velocity[axis] * travel);
        let state = particle.state(state);
        if let (Some(turbulence), Some(xform)) = (&self.turbulence, turbulence) {
            let strength = turbulence.strength.scalar(&state);
            let push = turbulence_push(&turbulence.image, xform, [particle.emission_time, particle.seeds[4]], strength * dt);
            particle.position = std::array::from_fn(|axis| particle.position[axis] + push[axis]);
        }
        let acceleration =
            if self.card_lerps { self.acceleration.vector(&state) } else { self.acceleration.around(&state) };
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
    fn state(&self, base: H3ParticleState) -> H3ParticleState {
        H3ParticleState {
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
        let way = if speed > 0.0 { self.velocity.map(|axis| axis / speed) } else { [0.0, 0.0, -1.0] };
        self.velocity = way.map(|axis| axis * 0.001);
        self.rotation_rate = 0.0;
        self.on_card = true;
        self.spawned_this_frame = true;
    }
}

/// One corner of a quad, as the particle program reads it.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct H3ParticleVertex {
    pub(crate) place: [f32; 3],
    pub(crate) sprite: [f32; 2],
    pub(crate) next_sprite: [f32; 2],
    pub(crate) billboard: [f32; 2],
    pub(crate) color: [f32; 4],
    /// Frame blend, palette row and black point.
    pub(crate) extra: [f32; 3],
    /// What a distortion's map leans along: its tangent and binormal, at scale.
    pub(crate) lean: [[f32; 3]; 2],
    /// `per_pixel_ravi_order_3`'s normal, bent as `surface_basis` bends it.
    pub(crate) normal: [f32; 3],
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn minus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt().max(1e-6);
    a.map(|axis| axis / length)
}

/// `billboard_basis`.
fn billboard(
    style: H3Billboard,
    place: [f32; 3],
    velocity: [f32; 3],
    local: [[f32; 3]; 3],
    view: &CeFlareView,
) -> [[f32; 3]; 2] {
    let eye: [f32; 3] = std::array::from_fn(|axis| place[axis] - view.eye[axis]);
    match style {
        H3Billboard::ScreenFacing => [view.right, view.up],
        H3Billboard::CameraFacing => {
            let eye = unit(eye);
            let other = if eye[2].abs() < 0.99 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
            let across = unit(cross(eye, other));
            [across, unit(cross(across, eye))]
        }
        H3Billboard::ScreenParallel => {
            let along = unit(velocity);
            [along, unit(cross(along, eye))]
        }
        H3Billboard::ScreenPerpendicular => {
            let motion = unit(velocity);
            let other = if motion[2].abs() < 0.99 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
            let across = unit(cross(motion, other));
            [across, cross(motion, across)]
        }
        H3Billboard::ScreenVertical => {
            let up = [0.0, 0.0, 1.0];
            [up, unit(cross(up, eye))]
        }
        H3Billboard::LocalVertical => [local[2], unit(cross(local[2], eye))],
        H3Billboard::LocalHorizontal => [local[0], local[1]],
        H3Billboard::ScreenHorizontal | H3Billboard::World => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        H3Billboard::VelocityHorizontal => {
            let along = unit(velocity);
            let other = if along[0].abs() < 0.99 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
            [along, unit(cross(other, along))]
        }
    }
}

/// `default_vs`' exposure: none for a multiply, nor for Halo 2, whose colours
/// are what shows. Halo 3's at its own; Reach's additives at
/// `V_ILLUM_EXPOSURE` (`v_alt_exposure.y`) and the rest at `v_exposure.x`,
/// at the area screen effects' `stops`. Beams and contrails take the same
/// (`effects__beam.hlsl` and `effects__contrail_render.hlsl`, l.258–269).
pub(super) fn particle_exposure(blend: H3Blend, exposed: bool, reach: bool, stops: f32) -> f32 {
    if blend == H3Blend::Multiply || !exposed {
        1.0
    } else if !reach {
        H3_EXPOSURE
    } else if matches!(blend, H3Blend::Additive | H3Blend::AddSrcTimesSrcAlpha) {
        reach_alt_exposure(stops)[1]
    } else {
        reach_exposure(stops)[0]
    }
}

/// `particle_render_fast.hlsl`'s `billboard_basis(…, limited = true)`: only
/// screen and camera facing, and every other style lies flat in the world.
fn cheap_billboard(style: H3Billboard) -> H3Billboard {
    match style {
        H3Billboard::ScreenFacing | H3Billboard::CameraFacing => style,
        _ => H3Billboard::World,
    }
}

/// The frame of the second sprite texcoord (`particle_render.hlsl`'s
/// `DOUBLE_SPRITE_TEXCOORD`): the next when frames blend; for Reach's
/// plasmas without, `floor(seed 1 · 16)` frames on.
fn second_frame(frame: f32, frame_blend: bool, plasma_second_sprite: bool, seed: f32) -> f32 {
    if plasma_second_sprite && !frame_blend { frame.floor() + (seed * 16.0).floor() } else { frame + 1.0 }
}

/// `compute_variant`.
fn frame_variant(input: f32, count: usize, one_shot: bool, backwards: bool) -> f32 {
    if count <= 1 {
        return 0.0;
    }
    let count = (if one_shot { count - 1 } else { count }) as f32;
    let variant = count * input;
    if backwards { count - variant } else { variant }
}

/// `effect_lightprobe_get_lightmap_tint`, as `initial_color.rgb * exp2(a)`
/// comes out: the probe's constant term, with a brightness of at most 2.
pub(super) fn lightmap_tint(lighting: &[[f32; 4]; 10]) -> [f32; 3] {
    let scale = (4.0 * std::f32::consts::PI).sqrt().recip();
    let tint: [f32; 3] = std::array::from_fn(|channel| lighting[0][channel] * scale);
    let brightest = tint[0].max(tint[1]).max(tint[2]).max(f32::EPSILON);
    let exponent = brightest.log2().clamp(0.0, 1.0);
    tint.map(|channel| (channel * 2f32.powf(-exponent)).clamp(0.0, 1.0) * 2f32.powf(exponent))
}

/// `surface_basis`, for a distortion: its corners lean away as a dome's would.
fn curved_lean(lean: [[f32; 3]; 2], plane: [[f32; 3]; 2], offset: [f32; 2], curvature: f32) -> [[f32; 3]; 2] {
    let norm = (offset[0] * offset[0] + offset[1] * offset[1]).sqrt() + 0.0001;
    let [x, y] = offset.map(|side| side / norm);
    let axis = std::array::from_fn(|axis| -y * plane[0][axis] + x * plane[1][axis]);
    let rotation = axis_rotation(axis, curvature * std::f32::consts::FRAC_PI_2);
    lean.map(|side| std::array::from_fn(|column| (0..3).map(|row| side[row] * rotation[row][column]).sum()))
}

/// `matrix3x3_rotation_from_axis_and_angle`.
fn axis_rotation(axis: [f32; 3], angle: f32) -> [[f32; 3]; 3] {
    let (sin, cos) = angle.sin_cos();
    let [x, y, z] = axis;
    let rest = axis.map(|value| value * (1.0 - cos));
    [
        [rest[0] * x + cos, rest[1] * x + sin * z, rest[2] * x - sin * y],
        [rest[0] * y - sin * z, rest[1] * y + cos, rest[2] * y + sin * x],
        [rest[0] * z + sin * y, rest[1] * z - sin * x, rest[2] * z + cos],
    ]
}

/// `frame_texcoord`.
fn sprite_uv(frame: [f32; 4], uv: [f32; 2], scroll: [f32; 2], scale: f32) -> [f32; 2] {
    let [left, top, width, height] = frame;
    let shrink = 0.5 * (1.0 - scale);
    [
        uv[0] * width * scale + scroll[0] * width.abs() + left + shrink * width,
        uv[1] * height * scale + scroll[1] * height.abs() + top + shrink * height,
    ]
}

impl H3ParticleField {
    /// Run everything up to `time`, a frame at a time as `effects_frame_advance`
    /// runs the particle systems. A new field starts a pre-roll short, so a
    /// held pose has particles in flight from its first frame.
    pub(crate) fn advance(
        &mut self,
        systems: &[Arc<H3ParticleSystem>],
        values: impl Fn(u8) -> Option<H2FunctionValues>,
        onsets: &mut H3EffectOnsets,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        ground: f32,
        time: f32,
    ) {
        if self.running.len() != systems.len() || self.time.is_none_or(|last| time < last) {
            self.running = systems.iter().enumerate().map(|(at, system)| fresh_emitters(system, at as u32)).collect();
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
                            let found = tracks[run.parent].iter().find(|track| track.id == followed);
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
                        let alive = first || run.lifetime.map_or(age < system.times.once(), |lifetime| age < lifetime);
                        if system.die_with_effect && !alive {
                            run.running.iter_mut().for_each(|running| running.particles.clear());
                        }
                        let state = alive.then(|| system_state_stepped(system, None, now, dt, run.start)).flatten();
                        let state = state.map(|state| H3ParticleState {
                            effect_a: run.effect_a,
                            system_age: run.lifetime.map_or(state.system_age, |lifetime| age / lifetime),
                            ..state
                        });
                        let state = with_lod(system, state, run.origin, eye);
                        let (place, events) = ((run.origin, run.axes), (&mut births, &mut deaths, &mut hits));
                        step_location(system, &mut run.running, state, place, now, dt, ground, events, &mut no_tracks);
                        alive || run.running.iter().any(|running| !running.particles.is_empty())
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
                step_location(system, running, state, (origin, axes), now, dt, ground, events, tracks);
                let off = !system.scale_by.is_empty()
                    && !scope.as_ref().and_then(|values| values.get(&system.scale_by)).is_some_and(|value| *value > 0.0);
                if system.die_with_effect && (off || system.times.is_over(now - onset)) {
                    self.running[index].iter_mut().for_each(|running| running.particles.clear());
                }
                let effect_a = state.map_or(1.0, |state| state.effect_a);
                for (trigger, offset) in &system.attached {
                    let events = match trigger {
                        H3ParticleTrigger::Birth | H3ParticleTrigger::Carried => &births,
                        H3ParticleTrigger::Death => &deaths,
                        H3ParticleTrigger::Collision => &hits,
                    };
                    let follow = matches!(trigger, H3ParticleTrigger::Birth | H3ParticleTrigger::Carried);
                    spawned.extend(events.iter().map(|&(id, place, velocity)| {
                        let speed = dot(velocity, velocity).sqrt();
                        let (effect_a, lifetime) = match trigger {
                            // The game makes them under the same effect.
                            H3ParticleTrigger::Carried => (effect_a, self.lifespan_of(index, id)),
                            // A hit reports the velocity it bounced away with.
                            H3ParticleTrigger::Death | H3ParticleTrigger::Collision if system.look.death_scaled_by_speed => {
                                ((speed - 0.5).clamp(0.0, 1.0), None)
                            }
                            _ => (1.0, None),
                        };
                        ((index + offset, index), follow.then_some(id), (place, velocity), system.world, origin, axes, effect_a, lifetime)
                    }));
                }
            }
            for ((index, parent), follows, (place, velocity), world, origin, axes, effect_a, lifetime) in spawned {
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
                    (std::array::from_fn(|axis| origin[axis] + turn(place)[axis]), turn(velocity))
                };
                let seed = (runs.len() as u32 + 1).wrapping_mul(index as u32 + 1).wrapping_add((now * 30.0) as u32);
                runs.push(H3ChildInstance {
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
        let particles = self.running.get(index)?.iter().flat_map(|running| &running.particles);
        particles.filter(|particle| particle.id == id).map(|particle| particle.inverse_lifespan.recip()).next()
    }

    /// The quads of a system, in a world-space triangle list, at the area
    /// screen effects' exposure `stops`.
    pub(crate) fn vertices(
        &self,
        index: usize,
        system: &H3ParticleSystem,
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        lighting: &[[f32; 4]; 10],
        time: f32,
        onset: f32,
        stops: f32,
    ) -> Vec<H3ParticleVertex> {
        let mut vertices = Vec::new();
        if system.hidden {
            return vertices;
        }
        if system.child {
            for run in self.instances.get(index).into_iter().flatten() {
                let at = (run.origin, run.axes, run.start);
                system.vertices_at(&run.running, at, values, view, lighting, time, stops, &mut vertices);
            }
            return vertices;
        }
        let Some(running) = self.running.get(index) else {
            return vertices;
        };
        let (origin, axes) = location(system, bones);
        system.vertices_at(running, (origin, axes, onset), values, view, lighting, time, stops, &mut vertices);
        vertices
    }

    /// Where the particles of `system` are, for a contrail that follows them.
    pub(crate) fn tracks_of<'a>(&'a self, systems: &[Arc<H3ParticleSystem>], strip: &Arc<H3StripAt>) -> Vec<&'a H3ParticleTrack> {
        let carriers = systems.iter().zip(&self.tracks);
        let carrying = carriers.filter(|(system, _)| system.carried.iter().any(|known| Arc::ptr_eq(known, strip)));
        carrying.flat_map(|(_, tracks)| tracks).collect()
    }

    /// Where a system's sort goes, for one whose place a particle chose.
    pub(crate) fn distance(
        &self,
        index: usize,
        system: &H3ParticleSystem,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
    ) -> f32 {
        let run = self.instances.get(index).and_then(|runs| runs.first());
        match run.filter(|_| system.child) {
            Some(run) => dot(view.forward, minus(run.origin, view.eye)) - system.sort_bias * 0.05,
            None => system.distance(bones, view),
        }
    }
}

impl H3ParticleSystem {
    /// A location's quads, at the end of `vertices`.
    #[allow(clippy::too_many_arguments)]
    fn vertices_at(
        &self,
        running: &[RunningEmitter],
        (origin, axes, onset): ([f32; 3], [[f32; 3]; 3], f32),
        values: Option<&H2FunctionValues>,
        view: &CeFlareView,
        lighting: &[[f32; 4]; 10],
        time: f32,
        stops: f32,
        vertices: &mut Vec<H3ParticleVertex>,
    ) {
        let system = self;
        let look = &*system.look;
        // Halo 4's draw through their material, from packed state.
        if look.h4.is_some() {
            return;
        }
        let light = particle_exposure(look.shader.blend, look.exposed, look.reach, stops);
        let initial = match (look.tint_from_lightmap, look.reach) {
            (false, _) => [1.0; 3],
            (true, false) => lightmap_tint(lighting),
            (true, true) => reach_lightmap_tint(),
        };
        let ambient = reach_particle_light().ambient;
        // `particle_render_fast.hlsl`: no aspect, no blur, no quarter turn.
        let style = if look.cheap { cheap_billboard(look.billboard) } else { look.billboard };
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
            // `calc_matrix`, or none for what already lies in the world.
            const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            let (offset, turned) = if system.world { ([0.0; 3], IDENTITY) } else { emitter_frame(emitter, origin, axes, &base) };
            for particle in &running.particles {
                let state = particle.state(base);
                let to_world = |vector: [f32; 3]| -> [f32; 3] {
                    std::array::from_fn(|axis| (0..3).map(|at| turned[at][axis] * vector[at]).sum())
                };
                let place: [f32; 3] = {
                    let moved = to_world(particle.position);
                    std::array::from_fn(|axis| offset[axis] + moved[axis])
                };
                let velocity = to_world(particle.velocity);
                let size = emitter.size.scalar(&state) * emitter.scale.scalar(&state);
                let mut aspect = if look.cheap { 1.0 } else { look.aspect.scalar(&state) };
                if let Some(stretch) = look.blur_aspect.filter(|_| !look.cheap) {
                    // A baseline of 30, not the frame rate. The preview's observer is still.
                    aspect += dot(velocity, velocity).sqrt() * stretch / (30.0 * size);
                }
                let basis = billboard(style, place, velocity, turned, view);
                let manual = emitter.rotation.scalar(&state).rem_euclid(1.0);
                let turned_up = if look.vertical && !look.cheap { 0.25 } else { 0.0 };
                let spun = if emitter.rotation_given { 0.0 } else { particle.rotation };
                let (sin, cos) = (std::f32::consts::TAU * (spun + manual + turned_up)).sin_cos();
                let tint = emitter.tint.color(&state);
                let color = look.color.color(&state);
                let intensity = look.intensity.scalar(&state);
                let mut alpha = (emitter.alpha.scalar(&state) * look.alpha.scalar(&state)).clamp(0.0, 1.0);
                // `default_vs`: wholly transparent is dead, whatever the blend.
                if alpha == 0.0 {
                    continue;
                }
                if look.intensity_fades {
                    alpha *= intensity;
                }
                let eye: [f32; 3] = std::array::from_fn(|axis| place[axis] - view.eye[axis]);
                let near = system.near_range * (dot(view.forward, eye) - system.near_cutoff);
                alpha *= if near.is_nan() { 0.0 } else { near.clamp(0.0, 1.0) };
                if alpha == 0.0 && look.model.is_none() {
                    continue;
                }
                let rgb: [f32; 3] = std::array::from_fn(|axis| {
                    (tint[axis] * color[axis]).clamp(0.0, 1.0) * initial[axis] * intensity * light
                });
                let shading = look.shader.lighting;
                let normal_at = |planar: [f32; 2], spun: [[f32; 3]; 2]| -> [f32; 3] {
                    let normal = unit(cross(spun[0], spun[1]));
                    if shading == H3ParticleLighting::None {
                        return normal;
                    }
                    let tangent = (look.curvature * std::f32::consts::FRAC_PI_2).tan();
                    let bent: [f32; 3] = if tangent > 0.0 {
                        let plane: [f32; 3] = std::array::from_fn(|axis| spun[0][axis] * planar[0] + spun[1][axis] * planar[1]);
                        let plane = unit(plane);
                        std::array::from_fn(|axis| normal[axis] + plane[axis])
                    } else {
                        normal
                    };
                    unit(bent)
                };
                // `per_vertex_ravi_order_0` on the vertex; order 3 on the pixel.
                // Reach's `per_vertex_ambient` is the probe's ambient, whatever
                // the normal.
                let lit_by = |normal: [f32; 3]| match shading {
                    H3ParticleLighting::Ambient => ravi(shading, normal, lighting),
                    H3ParticleLighting::ReachAmbient => ambient,
                    _ => [1.0; 3],
                };
                let (frames, corner) = match look.sequences.get(particle.sequence) {
                    Some((frames, corner)) => (frames, *corner),
                    None => (&look.frames, look.corner),
                };
                let count = frames.len();
                let frame_at = particle.frame + look.frame.scalar(&state).rem_euclid(1.0);
                let frame = frame_variant(frame_at, count, look.one_shot, look.backwards && particle.flip[2]);
                let first = frames[frame.floor().rem_euclid(count as f32) as usize % count];
                let shader = &look.shader;
                let next = second_frame(frame, shader.frame_blend, shader.plasma_second_sprite, particle.seeds[0]);
                let second = frames[next.floor().rem_euclid(count as f32) as usize % count];
                let blend = if shader.frame_blend && !look.cheap { frame.fract() } else { 0.0 };
                let scroll = emitter.scroll.map(|rate| rate * time);
                let [from, to] = look.shader.uv_scale;
                let uv_scale = |at: f32| (from + (to - from) * at / 2.0).recip();
                let (first_scale, second_scale) =
                    if look.shader.frame_blend { (uv_scale(1.0 + blend), uv_scale(blend)) } else { (1.0, 1.0) };
                let black = emitter.black_point.scalar(&state).clamp(0.0, 0.9999);
                // The cheap shader's `palette_v` is a literal half.
                let palette = if look.cheap { 0.5 } else { look.palette.scalar(&state).clamp(0.0, 0.9999) };
                let [x, y, width, height] = corner;
                let at_spot = |spot: [f32; 3], mut uv: [f32; 2], normal: [f32; 3], lit: [f32; 3], lean: [[f32; 3]; 2]| {
                    let mut faded = alpha;
                    if let Some([range, cutoff]) = look.edge_fade {
                        let toward = unit(minus(spot, view.eye));
                        let angle = std::f32::consts::FRAC_PI_2 - dot(toward, normal).abs().min(1.0).acos();
                        faded *= (range * (angle - cutoff)).clamp(0.0, 1.0);
                    }
                    if look.flip_u && particle.flip[0] && !look.cheap {
                        uv[0] = 1.0 - uv[0];
                    }
                    if look.flip_v && particle.flip[1] && !look.cheap {
                        uv[1] = 1.0 - uv[1];
                    }
                    // The cheap shader samples every map at the quad's own uv.
                    let (sprite, next_sprite) = if look.cheap {
                        (uv, uv)
                    } else {
                        (sprite_uv(first, uv, scroll, first_scale), sprite_uv(second, uv, scroll, second_scale))
                    };
                    H3ParticleVertex {
                        place: spot,
                        sprite,
                        next_sprite,
                        billboard: uv,
                        color: [rgb[0] * lit[0], rgb[1] * lit[1], rgb[2] * lit[2], faded],
                        extra: [blend, palette, black],
                        lean,
                        normal,
                    }
                };
                if let Some(meshes) = &look.model {
                    let angle = std::f32::consts::TAU * (particle.rotation + manual);
                    let variant = frame_variant(frame_at, meshes.len(), look.one_shot, look.backwards && particle.flip[2]);
                    let mesh = &meshes[variant.floor().rem_euclid(meshes.len() as f32) as usize % meshes.len()];
                    let plane = [basis[0], basis[1], cross(basis[0], basis[1])];
                    let plane = axis_rotation(particle.axis, angle).map(|row| -> [f32; 3] {
                        std::array::from_fn(|axis| (0..3).map(|at| row[at] * plane[at][axis]).sum())
                    });
                    let along = |vector: [f32; 3]| -> [f32; 3] {
                        std::array::from_fn(|axis| (0..3).map(|at| vector[at] * plane[at][axis]).sum())
                    };
                    for triangle in mesh.chunks_exact(3) {
                        // Backfaces cull among the transparents.
                        let turned_away = triangle.iter().all(|vertex| {
                            let normal = along([vertex.normal.i, vertex.normal.j, vertex.normal.k]);
                            dot(normal, minus(view.eye, place)) < 0.0
                        });
                        if turned_away && !look.double_sided {
                            continue;
                        }
                        for vertex in triangle {
                        let at = [vertex.position.x * aspect, vertex.position.y, vertex.position.z];
                        let moved = along(at);
                        let spot = std::array::from_fn(|axis| place[axis] + moved[axis] * size);
                        let normal = unit(along([vertex.normal.i, vertex.normal.j, vertex.normal.k]));
                        let lit = lit_by(normal);
                        let lean = [plane[0], plane[1]].map(|side| side.map(|axis| axis * size * width));
                        vertices.push(at_spot(spot, [vertex.texcoord.x, vertex.texcoord.y], normal, lit, lean));
                        }
                    }
                    continue;
                }
                let corner = |shift: [f32; 2]| {
                    let planar = [(shift[0] * width + x) * aspect, shift[1] * height + y];
                    let spun: [[f32; 3]; 2] = [
                        std::array::from_fn(|axis| cos * basis[0][axis] + sin * basis[1][axis]),
                        std::array::from_fn(|axis| -sin * basis[0][axis] + cos * basis[1][axis]),
                    ];
                    let spot: [f32; 3] = std::array::from_fn(|axis| {
                        place[axis] + (spun[0][axis] * planar[0] + spun[1][axis] * planar[1]) * size
                    });
                    let lean = [spun[0].map(|axis| axis * size * width), spun[1].map(|axis| axis * size * height)];
                    let lean = if look.shader.distortion.is_some() { curved_lean(lean, spun, planar, look.curvature) } else { lean };
                    let normal = normal_at(planar, spun);
                    at_spot(spot, shift, normal, lit_by(normal), lean)
                };
                let quad = [corner([0.0, 0.0]), corner([1.0, 0.0]), corner([1.0, 1.0]), corner([0.0, 1.0])];
                vertices.extend([quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
            }
        }
    }
}

/// Where a system stands: its marker, on its node's pose.
fn location(system: &H3ParticleSystem, bones: Option<&[[f32; 4]]>) -> ([f32; 3], [[f32; 3]; 3]) {
    let Some(rows) = bones.and_then(|bones| bones.get(system.node * 3..system.node * 3 + 3)) else {
        return (system.position, system.axes);
    };
    let [x, y, z] = system.position;
    let place = |row: &[f32; 4]| row[0] * x + row[1] * y + row[2] * z + row[3];
    let turn = |[i, j, k]: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|row| rows[row][0] * i + rows[row][1] * j + rows[row][2] * k)
    };
    let axes = if system.world_oriented { system.axes } else { system.axes.map(turn) };
    ([place(&rows[0]), place(&rows[1]), place(&rows[2])], axes)
}

/// `c_particle_emitter::calc_matrix`: its direction turns the location, and its
/// offset moves along that.
fn emitter_frame(
    emitter: &H3Emitter,
    origin: [f32; 3],
    axes: [[f32; 3]; 3],
    state: &H3ParticleState,
) -> ([f32; 3], [[f32; 3]; 3]) {
    let direction = unit(emitter.direction.around(state));
    let yaw = direction[1].atan2(direction[0]);
    let pitch = direction[2].asin();
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    let rotation = [
        [cos_pitch * cos_yaw, sin_yaw, sin_pitch * cos_yaw],
        [-cos_pitch * sin_yaw, cos_yaw, -sin_pitch * sin_yaw],
        [-sin_pitch, 0.0, cos_pitch],
    ];
    let turned: [[f32; 3]; 3] = rotation.map(|row| {
        std::array::from_fn(|axis| (0..3).map(|at| axes[at][axis] * row[at]).sum())
    });
    let offset = emitter.offset.vector(state);
    let moved: [f32; 3] = std::array::from_fn(|axis| {
        origin[axis] + (0..3).map(|at| turned[at][axis] * offset[at]).sum::<f32>()
    });
    (moved, turned)
}

impl H3ParticleSystem {
    /// Its sort position, nudged by its tag: `add_element`'s `sort_bias`.
    pub(crate) fn distance(&self, bones: Option<&[[f32; 4]]>, view: &CeFlareView) -> f32 {
        let (origin, _) = location(self, bones);
        let eye: [f32; 3] = std::array::from_fn(|axis| origin[axis] - view.eye[axis]);
        dot(view.forward, eye) - self.sort_bias * 0.05
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_custom_shape_picks_its_point_by_radius_and_strays_by_angle() {
        let mut random = Random(1);
        let point = |x: f32| H3CustomPoint { position: [x, 0.0, 0.0], normal: [0.0, 0.0, 1.0] };
        let points = [point(1.0), point(2.0), point(3.0), point(4.0)];
        assert_eq!(emit_from_points(&points, 0.0, 0.0, &mut random), (point(1.0).position, [0.0, 0.0, 1.0]));
        assert_eq!(emit_from_points(&points, 0.5, 0.0, &mut random).0, point(3.0).position);
        assert_eq!(emit_from_points(&points, 1.0, 0.0, &mut random).0, point(4.0).position, "the last, not past it");
        assert_eq!(emit_from_points(&points, -1.0, 0.0, &mut random).0, point(1.0).position);
        let (_, strayed) = emit_from_points(&points, 0.0, 90.0, &mut random);
        assert!(strayed != [0.0, 0.0, 1.0]);
        assert_eq!(emit_from_points(&[], 0.5, 0.0, &mut random), ([0.0; 3], [0.0; 3]));
    }

    #[test]
    fn only_odst_and_reach_let_a_particle_that_faces_nothing_stay_still() {
        let (halo3, odst, reach) = (Some(GameId::Halo3), Some(GameId::Halo3Odst), Some(GameId::HaloReach));
        assert!(still_particles_move(halo3, H3Billboard::ScreenFacing));
        assert!(!still_particles_move(odst, H3Billboard::ScreenFacing));
        assert!(still_particles_move(odst, H3Billboard::ScreenParallel));
        assert!(still_particles_move(odst, H3Billboard::VelocityHorizontal));
        assert!(!still_particles_move(reach, H3Billboard::ScreenFacing));
        assert!(still_particles_move(reach, H3Billboard::ScreenPerpendicular));
    }

    #[test]
    fn a_lod_left_empty_takes_the_postprocess_defaults() {
        assert_eq!(baked_lod([0.0; 4], true, [].into_iter()), [0.0, 20.0, 1.0, 5.0]);
        assert_eq!(baked_lod([0.0; 4], true, [150.0].into_iter()), [-0.1, 20.0, 3.0, 5.0]);
        assert_eq!(baked_lod([0.0; 4], false, [150.0].into_iter()), [0.0, 20.0, 1.0, 5.0], "not for one camera");
        assert_eq!(baked_lod([1.0, 8.0, 2.0, 3.0], true, [150.0].into_iter()), [1.0, 8.0, 2.0, 3.0]);
    }

    #[test]
    fn a_direction_turns_around_rather_than_across() {
        let ends = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let middle = slerped(ends, 0.5);
        assert!((dot(middle, middle).sqrt() - 1.0).abs() < 1e-3, "{middle:?}");
        assert!((slerped(ends, 0.0)[0] - 1.0).abs() < 1e-3);
        assert!((slerped(ends, 1.0)[1] - 1.0).abs() < 1e-3);
        let same = slerped([[0.0, 0.0, 2.0], [0.0, 0.0, 2.0]], 0.5);
        assert!(same.iter().all(|axis| axis.is_finite()));
    }

    #[test]
    fn only_what_landed_slowly_rests() {
        assert!(comes_to_rest(true, [0.2, 0.0, 0.0]));
        assert!(!comes_to_rest(true, [0.3, 0.0, 0.0]));
        assert!(!comes_to_rest(false, [0.0; 3]));
    }

    #[test]
    fn a_line_runs_along_y_and_a_breakable_has_nothing_of_its_own() {
        let mut random = Random(3);
        let (place, _) = emit(H3EmissionShape::Line, 2.0, 0.0, false, &mut random);
        assert_eq!(place[0], 0.0);
        assert!(place[1].abs() <= 2.0 && place[1] != 0.0);
        assert_eq!(emit(H3EmissionShape::Breakable, 2.0, 30.0, false, &mut random), ([0.0; 3], [0.0; 3]));
    }

    #[test]
    fn an_impact_with_no_size_sprays_from_one_point_at_its_angle() {
        let mut random = Random(5);
        for shape in [H3EmissionShape::ImpactContour, H3EmissionShape::ImpactArea] {
            let (place, velocity) = emit(shape, 9.0, 60.0, false, &mut random);
            assert_eq!(place, [0.0; 3]);
            assert!((velocity[0] - 0.5).abs() < 1e-5);
            assert!((dot(velocity, velocity) - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn a_flat_distortion_keeps_its_lean_and_a_curved_one_tilts_it() {
        let plane = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        assert_eq!(curved_lean(plane, plane, [1.0, 0.0], 0.0)[0], plane[0]);
        let curved = curved_lean(plane, plane, [1.0, 0.0], 0.5);
        assert!(curved[0][2].abs() > 0.5, "{curved:?}");
        assert!((curved[1][1] - 1.0).abs() < 1e-3, "the axis itself stays: {curved:?}");
    }

    #[test]
    fn a_particles_axes_look_along_it_and_a_still_one_gets_a_default() {
        let axes = axes_along([0.0, 2.0, 0.0]);
        assert_eq!(axes[0], [0.0, 1.0, 0.0]);
        assert!(dot(axes[1], axes[0]).abs() < 1e-6 && dot(axes[2], axes[0]).abs() < 1e-6);
        assert_eq!(axes_along([0.0; 3])[0], [1.0, 0.0, 0.0]);
    }

    #[test]
    fn an_older_tags_names_read_as_the_same_inputs() {
        assert_eq!(H3ParticleInput::named("emitter age"), H3ParticleInput::SystemAge);
        assert_eq!(H3ParticleInput::named("emitter random"), H3ParticleInput::SystemRandom);
        assert_eq!(H3ParticleInput::named("emitter correlation 2"), H3ParticleInput::SystemSeed(1));
        assert_eq!(H3ParticleInput::named("system lod"), H3ParticleInput::Lod);
        assert_eq!(H3ParticleInput::named("no such input"), H3ParticleInput::Other);
    }

    #[test]
    fn halo_2_passes_come_from_their_templates_and_distortion_has_none() {
        let pass = |name: &str| h2_particle_pass(&format!("shaders\\shader_templates\\transparent\\{name}"));
        assert_eq!(pass("particle_additive"), Some((H3ParticleAlbedo::Diffuse, H3Blend::AddSrcTimesSrcAlpha, false)));
        assert_eq!(pass("particle_alpha_blend_tint"), Some((H3ParticleAlbedo::H2Tint, H3Blend::AlphaBlend, false)));
        assert_eq!(pass("particle_alpha_multiply_add"), Some((H3ParticleAlbedo::H2MultiplyAdd, H3Blend::PreMultiplied, false)));
        assert_eq!(pass("particle_alpha_blend_distortion"), Some((H3ParticleAlbedo::H2Distortion, H3Blend::AlphaBlend, false)));
    }

    #[test]
    fn a_halo_2_lod_fades_in_near_the_camera_only_when_shown_in_any_camera_mode() {
        assert_eq!(h2_baked_lod([0.0; 4], true, [].into_iter()), [0.0, 30.0, 1.0, 10.0]);
        assert_eq!(h2_baked_lod([0.0; 4], true, [150.0].into_iter()), [-0.1, 30.0, 3.0, 10.0]);
        assert_eq!(h2_baked_lod([0.0; 4], false, [150.0].into_iter()), [0.0, 30.0, 0.0, 10.0], "not for one camera");
        assert_eq!(h2_baked_lod([1.0, 5.0, 2.0, 3.0], true, [150.0].into_iter()), [1.0, 5.0, 2.0, 3.0]);
        // The SMG's muzzle burst, a hand's length from the eye.
        let one_camera = Some(h2_baked_lod([0.0; 4], false, [30.0].into_iter()));
        assert_eq!(lod_amount(one_camera, 0.8), 1.0);
        let any_camera = Some(h2_baked_lod([0.0; 4], true, [30.0].into_iter()));
        assert!((lod_amount(any_camera, 0.8) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn a_scale_multiplies_what_a_property_reads() {
        let state = H3ParticleState::default();
        let rate = H3ParticleProperty::fixed(2.0).unwrap();
        assert_eq!(rate.scalar(&state), 2.0);
        assert_eq!(rate.scaled(30.0).scalar(&state), 60.0);
    }

    #[test]
    fn a_lightmap_tint_is_the_probes_constant_and_keeps_only_two_stops() {
        let scale = (4.0 * std::f32::consts::PI).sqrt();
        let mut lighting = [[0.0; 4]; 10];
        lighting[0] = [0.5 * scale, 0.25 * scale, 0.0, 0.0];
        let tint = lightmap_tint(&lighting);
        assert!((tint[0] - 0.5).abs() < 1e-5 && (tint[1] - 0.25).abs() < 1e-5);
        lighting[0] = [16.0 * scale, 8.0 * scale, 0.0, 0.0];
        let tint = lightmap_tint(&lighting);
        assert!((tint[0] - 2.0).abs() < 1e-5, "{tint:?}");
    }

    #[test]
    fn detail_fades_in_and_out_across_its_feathers() {
        let lod = Some([10.0, 100.0, 10.0, 20.0]);
        assert_eq!(lod_amount(None, 1000.0), 1.0, "always");
        assert_eq!(lod_amount(lod, 5.0), 0.0);
        assert_eq!(lod_amount(lod, 15.0), 0.5);
        assert_eq!(lod_amount(lod, 50.0), 1.0);
        assert_eq!(lod_amount(lod, 90.0), 0.5);
        assert_eq!(lod_amount(lod, 150.0), 0.0);
    }

    #[test]
    fn a_particle_falling_through_the_ground_bounces_off_it() {
        let ground = ([0.0, 0.0, 1.0], 0.0);
        let (place, velocity) = bounce([0.0, 0.0, 0.1], [1.0, 0.0, -2.0], ground, 0.1, 0.5, 0.25).unwrap();
        assert!((place[2] - PUSH_OUT).abs() < 1e-6, "{place:?}");
        assert!((place[0] - 0.05).abs() < 1e-6, "stopped where it met: {place:?}");
        assert_eq!(velocity, [0.75, 0.0, 1.0]);
    }

    #[test]
    fn a_step_that_stays_above_or_below_the_ground_meets_nothing() {
        let ground = ([0.0, 0.0, 1.0], 0.0);
        assert_eq!(bounce([0.0, 0.0, 1.0], [0.0, 0.0, -1.0], ground, 0.1, 0.5, 0.5), None);
        assert_eq!(bounce([0.0, 0.0, -1.0], [0.0, 0.0, -1.0], ground, 0.1, 0.5, 0.5), None);
        assert_eq!(bounce([0.0, 0.0, 0.1], [0.0, 0.0, 5.0], ground, 0.1, 0.5, 0.5), None);
    }

    #[test]
    fn the_ground_is_found_from_a_lifted_frame_too() {
        // A frame 2 above the ground: its origin is 2 up from the plane.
        let hit = bounce([0.0, 0.0, -1.9], [0.0, 0.0, -2.0], ([0.0, 0.0, 1.0], 2.0), 0.1, 0.0, 0.0);
        assert!(hit.is_some());
    }

    #[test]
    fn a_turn_about_x_carries_y_to_z_and_leaves_x() {
        let turned = axis_rotation([1.0, 0.0, 0.0], std::f32::consts::FRAC_PI_2);
        assert!((turned[0][0] - 1.0).abs() < 1e-6);
        assert!((turned[1][2] - 1.0).abs() < 1e-6, "{turned:?}");
        assert!((turned[2][1] + 1.0).abs() < 1e-6, "{turned:?}");
    }

    #[test]
    fn a_reference_axis_rotates_the_picked_axis_round() {
        let mut random = Random(1);
        assert_eq!(H3ParticleAxis::Constant.pick(0, 0.0, &mut random), [1.0, 0.0, 0.0]);
        assert_eq!(H3ParticleAxis::Constant.pick(1, 0.0, &mut random), [0.0, 1.0, 0.0]);
        assert_eq!(H3ParticleAxis::Constant.pick(2, 0.0, &mut random), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_whole_sprite_sits_around_its_place_and_an_offset_pins_its_edge() {
        let whole = [0.0, 1.0, 0.0, 1.0];
        assert_eq!(sprite_corner(whole, [0.5, 0.5], [0.0, 0.0], 1.0), [-1.0, -1.0, 2.0, 2.0]);
        // `blast_wave_half`: the left edge at the origin.
        assert_eq!(sprite_corner(whole, [0.5, 0.5], [-0.5, 0.0], 1.0), [0.0, -1.0, 2.0, 2.0]);
        // The sign of y is the other way.
        assert_eq!(sprite_corner(whole, [0.5, 0.5], [0.0, -0.5], 1.0), [-1.0, -2.0, 2.0, 2.0]);
    }

    #[test]
    fn a_narrow_sprite_narrows_and_a_wide_plate_widens() {
        let narrow = sprite_corner([0.0, 0.5, 0.0, 1.0], [0.25, 0.5], [0.0, 0.0], 1.0);
        assert_eq!(narrow, [-0.5, -1.0, 1.0, 2.0]);
        let wide = sprite_corner([0.0, 1.0, 0.0, 1.0], [0.5, 0.5], [0.0, 0.0], 2.0);
        assert_eq!(wide, [-2.0, -1.0, 4.0, 2.0]);
    }

    #[test]
    fn inputs_read_by_their_names_with_or_without_the_second_spelling() {
        assert_eq!(H3ParticleInput::named("system age{emitter age}"), H3ParticleInput::SystemAge);
        assert_eq!(H3ParticleInput::named("system age"), H3ParticleInput::SystemAge);
        assert_eq!(H3ParticleInput::named("particle correlation 3"), H3ParticleInput::Seed(2));
        assert_eq!(H3ParticleInput::named("no such state"), H3ParticleInput::Other);
    }

    #[test]
    fn randoms_take_seeds_in_turn_and_wrap() {
        let mut seeds = SeedAllocator::default();
        let mut take = |mut input| {
            seeds.allocate(&mut input);
            input
        };
        assert_eq!(take(H3ParticleInput::Random), H3ParticleInput::Seed(0));
        assert_eq!(take(H3ParticleInput::Random), H3ParticleInput::Seed(1));
        assert_eq!(take(H3ParticleInput::SystemRandom), H3ParticleInput::SystemSeed(0));
        assert_eq!(take(H3ParticleInput::Age), H3ParticleInput::Age);
    }

    #[test]
    fn a_sprayer_starts_at_the_origin_inside_its_cone() {
        let mut random = Random(7);
        for _ in 0..50 {
            let (place, direction) = emit(H3EmissionShape::Sprayer, 2.0, 10.0, false, &mut random);
            assert_eq!(place, [0.0; 3]);
            assert!(direction[0] >= 10f32.to_radians().cos() - 1e-5, "{direction:?}");
        }
    }

    #[test]
    fn a_globe_starts_on_its_radius_heading_out_and_an_implode_in() {
        let mut random = Random(9);
        let (place, direction) = emit(H3EmissionShape::Globe, 2.0, 0.0, false, &mut random);
        assert!((dot(place, place).sqrt() - 2.0).abs() < 1e-4);
        assert!(dot(place, direction) > 0.0);
        let (place, direction) = emit(H3EmissionShape::Implode, 2.0, 0.0, false, &mut random);
        assert!(dot(place, direction) < 0.0);
    }

    #[test]
    fn a_one_shot_stops_short_of_wrapping_and_a_single_frame_never_moves() {
        assert_eq!(frame_variant(0.5, 4, false, false), 2.0);
        assert_eq!(frame_variant(0.5, 4, true, false), 1.5);
        assert_eq!(frame_variant(0.25, 4, false, true), 3.0);
        assert_eq!(frame_variant(0.9, 1, false, false), 0.0);
    }

    #[test]
    fn the_same_seed_runs_the_same() {
        let (mut a, mut b) = (Random(3), Random(3));
        assert_eq!(a.next(), b.next());
        assert_ne!(a.next(), Random(4).next());
    }

    /// `decide_whether_cpu_or_gpu`: forcing the processor wins, then forcing
    /// the card; otherwise the card takes a particle with no attachments
    /// whose movement asks for nothing past `physics`, and a particle with no
    /// attachments moves to the card when it rests unless something was forced.
    #[test]
    fn tool_puts_an_emitter_on_the_card_only_when_nothing_needs_the_processor() {
        use H3ParticleUpdate::{Cpu, Gpu};
        let named =
            |names: &'static [&'static str]| move |wanted: &str| names.iter().any(|name| name.starts_with(wanted));
        let decide = |system: &'static [&'static str], attachments, movement: &'static [&'static str]| {
            particle_update(named(system), attachments, named(movement))
        };
        assert_eq!(decide(&[], false, &[]), (Gpu, true));
        assert_eq!(decide(&[], false, &["physics"]), (Gpu, true));
        assert_eq!(decide(&[], true, &["physics"]), (Cpu, false), "attachments play on the processor");
        assert_eq!(decide(&[], false, &["physics", "collide with structure"]), (Cpu, true), "on to the card at rest");
        assert_eq!(decide(&[], false, &["swarm"]), (Cpu, true));
        assert_eq!(decide(&["force gpu updating{force gpu rendering}"], true, &["wind"]), (Gpu, false));
        let force_cpu = &["force cpu updating{force cpu rendering}"];
        assert_eq!(decide(force_cpu, false, &["collide with structure"]), (Cpu, false), "forced stays put");
        let both = &["force cpu updating{force cpu rendering}", "force gpu updating{force gpu rendering}"];
        assert_eq!(decide(both, false, &[]), (Cpu, false), "forcing the processor wins");
    }

    /// Tool keeps one movement for each flagged kind, in its table's order
    /// whatever order the tag lists them in.
    #[test]
    fn movements_follow_the_flags_in_tools_order() {
        use H3ParticleController::{Collider, Flock, Physics};
        let named =
            |names: &'static [&'static str]| move |wanted: &str| names.iter().any(|name| name.starts_with(wanted));
        assert_eq!(flagged_controllers(named(&["collide with bipeds", "swarm", "physics"])), [Physics, Flock, Collider]);
        assert_eq!(flagged_controllers(named(&[])), []);
    }

    /// The first emitter of an effect's first system, from the Halo 3 kit.
    fn kit_emitter(root: &str, path: &str) -> Arc<H3Emitter> {
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(GameId::Halo3),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let effect = load_referenced_tag_from_source(&source, path, "effect", b"effe").unwrap();
        let event = elements(effect.root(), "events").into_iter().next().unwrap();
        let system = elements(event, "particle systems").into_iter().next().unwrap();
        Arc::clone(&read_system_parts(&source, system).unwrap().emitters[0])
    }

    /// With `BLAM_TEST_H3EK`: plain physics goes on the card, colliding stays
    /// on the processor, and a template's movements need the emitter's own
    /// flags, as tool's `postprocess_internal` keeps them.
    #[test]
    fn kit_emitters_update_where_tool_puts_them_and_move_as_their_flags_say() {
        let Ok(root) = std::env::var("BLAM_TEST_H3EK") else {
            eprintln!("skipping: BLAM_TEST_H3EK is not set");
            return;
        };
        let plain = kit_emitter(&root, "fx\\cinematics\\120lc_activation\\02\\shot_03\\halochunk_impact");
        assert_eq!(plain.update, H3ParticleUpdate::Gpu);
        assert_eq!(plain.controllers, [H3ParticleController::Physics]);
        let state = H3ParticleState { system_age: 0.5, effect_a: 1.0, effect_b: 1.0, lod: 1.0, ..Default::default() };
        let physics = plain.physics.as_ref().map(|physics| physics.on_card(&state));
        assert_eq!(physics, Some([0.0, 2.0, 1.0]), "gravity, air and rotational friction");
        let colliding = kit_emitter(&root, "fx\\cinematics\\100ld_confront_truth\\07\\shot_04\\tentacle_erupt");
        assert_eq!(colliding.update, H3ParticleUpdate::Cpu);
        assert!(colliding.controllers.contains(&H3ParticleController::Collider));
        // `_default`'s physics, with none of the emitter's own flags set.
        let unflagged = kit_emitter(&root, "fx\\cinematics\\040lb_cov_flee\\08\\shot_4\\flood_spew");
        assert!(unflagged.controllers.is_empty(), "{:?}", unflagged.controllers);
        assert!(unflagged.physics.is_none(), "the template's physics is not the emitter's");
    }

    /// On the card a particle's spawn frame moves nothing; after that it moves
    /// by the velocity it had before its emitter's air friction (2 here, with
    /// no gravity) slows it, where the processor slows it first.
    #[test]
    fn a_particle_on_the_card_waits_a_frame_then_moves_before_it_accelerates() {
        let Ok(root) = std::env::var("BLAM_TEST_H3EK") else {
            eprintln!("skipping: BLAM_TEST_H3EK is not set");
            return;
        };
        let emitter = kit_emitter(&root, "fx\\cinematics\\120lc_activation\\02\\shot_03\\halochunk_impact");
        let particle = Particle {
            id: 1,
            colliding: false,
            landed: false,
            resting: false,
            spawned_this_frame: true,
            on_card: false,
            axis: [0.0, 0.0, 1.0],
            age: 0.0,
            inverse_lifespan: 0.1,
            position: [0.0; 3],
            velocity: [1.0, 0.0, 0.0],
            rotation: 0.0,
            rotation_rate: 0.0,
            frame: 0.0,
            frame_rate: 0.0,
            seeds: [0.0; SEEDS],
            emission_time: 0.0,
            flip: [false; 3],
            sequence: 0,
            system_seeds: [0.0; SYSTEM_SEEDS],
        };
        let (mut particles, mut ended, dt) = (vec![particle], Vec::new(), 1.0 / 60.0);
        let state = H3ParticleState { system_age: 0.5, effect_a: 1.0, effect_b: 1.0, lod: 1.0, ..Default::default() };
        emitter.step(state, &mut particles, dt, [0.0; 3], &mut ended, None, false);
        assert_eq!((particles[0].position, particles[0].age), ([0.0; 3], 0.0), "the spawn frame");
        emitter.step(state, &mut particles, dt, [0.0; 3], &mut ended, None, false);
        assert_eq!(particles[0].position[0], dt, "it moved by the velocity it had");
        assert_eq!(particles[0].velocity[0], 1.0 - 2.0 * dt, "then friction took some off");
    }

    /// With `BLAM_TEST_H3EK`: a colliding emitter's particle that stops on the
    /// ground moves to the card, with a thousandth of a unit a second left and
    /// no physics; its first frame there moves nothing.
    #[test]
    fn a_particle_at_rest_moves_to_the_card_and_drifts() {
        let Ok(root) = std::env::var("BLAM_TEST_H3EK") else {
            eprintln!("skipping: BLAM_TEST_H3EK is not set");
            return;
        };
        let emitter = kit_emitter(&root, "fx\\cinematics\\100ld_confront_truth\\07\\shot_04\\tentacle_erupt");
        assert_eq!((emitter.update, emitter.card_at_rest), (H3ParticleUpdate::Cpu, true));
        let particle = Particle {
            id: 1,
            colliding: false,
            landed: true,
            resting: false,
            spawned_this_frame: false,
            on_card: false,
            axis: [0.0, 0.0, 1.0],
            age: 0.0,
            inverse_lifespan: 0.1,
            position: [0.0; 3],
            velocity: [0.1, 0.0, 0.0],
            rotation: 0.0,
            rotation_rate: 0.5,
            frame: 0.0,
            frame_rate: 0.0,
            seeds: [0.0; SEEDS],
            emission_time: 0.0,
            flip: [false; 3],
            sequence: 0,
            system_seeds: [0.0; SYSTEM_SEEDS],
        };
        let (mut particles, mut ended, dt) = (vec![particle], Vec::new(), 1.0 / 60.0);
        let state = H3ParticleState { system_age: 0.5, effect_a: 1.0, effect_b: 1.0, lod: 1.0, ..Default::default() };
        emitter.step(state, &mut particles, dt, [0.0; 3], &mut ended, None, true);
        let rested = particles[0].clone();
        assert!(rested.on_card && rested.rotation_rate == 0.0, "{rested:?}");
        assert!((dot(rested.velocity, rested.velocity).sqrt() - 0.001).abs() < 1e-6);
        emitter.step(state, &mut particles, dt, [0.0; 3], &mut ended, None, true);
        assert_eq!(particles.len(), 1, "on the card it no longer dies at rest");
        assert_eq!(particles[0].position, rested.position, "its first frame there");
        emitter.step(state, &mut particles, dt, [0.0; 3], &mut ended, None, true);
        let moved: [f32; 3] = std::array::from_fn(|axis| rested.position[axis] + rested.velocity[axis] * dt);
        assert_eq!(particles[0].position, moved, "it drifts by what it kept");
    }

    /// Halo 3 and ODST's state list writes `particle rotation` as 0 and
    /// never writes `distance from emitter`; Halo 2 reads the particle's own
    /// rotation.
    #[test]
    fn only_halo_2_reads_a_particles_rotation() {
        let resolved = |mut seeds: SeedAllocator, name: &str| {
            let mut input = H3ParticleInput::named(name);
            seeds.allocate(&mut input);
            input.of(&H3ParticleState { rotation: 0.25, ..Default::default() })
        };
        assert_eq!(resolved(SeedAllocator::default(), "particle rotation"), 0.25, "Halo 2");
        assert_eq!(resolved(SeedAllocator::halo3(), "particle rotation{physics rotation}"), 0.0);
        assert_eq!(resolved(SeedAllocator::halo3(), "distance from emitter"), 0.0);
    }

    /// A property whose function is not constant, reading `input` (and
    /// `range`, ranged or not).
    fn varying(input: H3ParticleInput, range: H3ParticleInput, ranged: bool) -> H3ParticleProperty {
        use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};
        let mut function = H2Function::new(FunctionType::Identity);
        function.set_clamp_range(0.0, 1.0).unwrap();
        function.set_ranged(ranged);
        let function = TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).unwrap();
        assert!(function.as_constant().is_none() && function.is_ranged() == ranged);
        H3ParticleProperty { input, range, function: Arc::new(function), ..H3ParticleProperty::fixed(0.0).unwrap() }
    }

    fn seeded(seeds: &mut SeedAllocator, mut input: H3ParticleInput) -> H3ParticleInput {
        seeds.allocate(&mut input);
        input
    }

    /// `particle_state_list.hlsl`'s state 20 is a tenth of the speed, read
    /// by name only in Reach's schema; 17 to 19 have no writer.
    #[test]
    fn reach_reads_a_tenth_of_the_speed_as_particle_velocity() {
        assert_eq!(H3ParticleInput::named("particle velocity"), H3ParticleInput::Velocity);
        for unwritten in ["distance from emitter", "particle simulation a", "particle simulation b"] {
            assert_eq!(H3ParticleInput::named(unwritten), H3ParticleInput::Other, "{unwritten}");
        }
        let particle = Particle { velocity: [3.0, 0.0, 4.0], ..test_particle() };
        let state = particle.state(H3ParticleState::default());
        assert_eq!(H3ParticleInput::Velocity.of(&state), 0.5);
        assert_eq!(H3ParticleInput::Velocity.of(&H3ParticleState::default()), 0.0, "nothing at a spawn");
    }

    /// `g_render_time+0x18`: seconds summed, not Halo 3's fraction of one,
    /// coming round at MCC's signed limit.
    #[test]
    fn reach_game_time_runs_in_seconds_and_comes_round_at_two_hours() {
        assert_eq!(particle_game_time(false, 2.5), 0.5);
        assert_eq!(particle_game_time(true, 2.5), 2.5);
        assert_eq!(particle_game_time(true, 7199.0), 7199.0);
        assert_eq!(particle_game_time(true, 7201.0), -7199.0);
    }

    /// Tool's `allocate_random_seed`: seed 8 first and down, and when all
    /// eight are taken, the reuse table from its fifth entry on.
    #[test]
    fn reach_hands_out_the_highest_free_seed_then_reuses_from_the_fifth() {
        use H3ParticleInput::{Random, Seed};
        let mut seeds = SeedAllocator::reach(0);
        let taken: Vec<_> = (0..10).map(|_| seeded(&mut seeds, Random)).collect();
        let expected = [Seed(7), Seed(6), Seed(5), Seed(4), Seed(3), Seed(2), Seed(1), Seed(0), Seed(6), Seed(1)];
        assert_eq!(taken, expected, "states 25 to 22, 7 to 4, then the table's 24 and 5");
        let mut halo3 = SeedAllocator::halo3();
        assert_eq!(seeded(&mut halo3, Random), Seed(0), "Halo 3 counts up");
    }

    /// What a property reads by name is taken already, so a random goes
    /// around it rather than reading the same stream.
    #[test]
    fn reach_steps_around_a_seed_read_by_name() {
        use H3ParticleInput::{Random, Seed, SystemRandom, SystemSeed};
        let explicit = H3ParticleInput::named("particle correlation 4").state_bit();
        let mut seeds = SeedAllocator::reach(explicit);
        let taken: Vec<_> = (0..8).map(|_| seeded(&mut seeds, Random)).collect();
        assert_eq!(taken[3..], [Seed(4), Seed(2), Seed(1), Seed(0), Seed(6)], "seed 4 skipped");
        let mut seeds = SeedAllocator::reach(0);
        let systems: Vec<_> = (0..5).map(|_| seeded(&mut seeds, SystemRandom)).collect();
        let expected = [SystemSeed(3), SystemSeed(2), SystemSeed(1), SystemSeed(0), SystemSeed(3)];
        assert_eq!(systems, expected, "27, 26, 9, 8, then the four-long table's fifth, 27");
        assert_eq!(seeded(&mut seeds, H3ParticleInput::Rotation), H3ParticleInput::Other, "Reach writes 0 there");
    }

    /// Reach's `used_states` and allocation read a range only when the
    /// function is ranged; Halo 3's allocate it whatever.
    #[test]
    fn reach_seeds_a_range_only_when_the_function_is_ranged() {
        use H3ParticleInput::{Random, Seed};
        let flat = varying(Random, Random, false);
        assert_eq!(flat.used_states(), Random.state_bit());
        let mut reach = flat.clone();
        reach.allocate(&mut SeedAllocator::reach(0));
        assert_eq!((reach.input, reach.range), (Seed(7), Random));
        let mut halo3 = flat;
        halo3.allocate(&mut SeedAllocator::halo3());
        assert_eq!((halo3.input, halo3.range), (Seed(0), Seed(1)));
        let mut ranged = varying(Random, Random, true);
        ranged.allocate(&mut SeedAllocator::reach(0));
        assert_eq!((ranged.input, ranged.range), (Seed(7), Seed(6)));
    }

    /// A Reach controller marks each parameter's own states just before its
    /// randoms take seeds: here the third parameter reads system seed 2 by
    /// name, so its random modifier cannot take it too.
    #[test]
    fn a_reach_controller_marks_each_parameter_before_seeding_it() {
        use H3ParticleInput::{SystemRandom, SystemSeed};
        let named = H3ParticleInput::named("system correlation 2");
        let parameters = [
            varying(SystemRandom, SystemRandom, false),
            varying(SystemRandom, SystemRandom, false),
            H3ParticleProperty { adds: Some(true), modifier: SystemRandom, ..varying(named, SystemRandom, false) },
        ];
        let run = |mut seeds: SeedAllocator| {
            let mut parameters = parameters.clone();
            parameters.iter_mut().for_each(|parameter| parameter.allocate(&mut seeds));
            (parameters[2].modifier, seeds.used())
        };
        let (modifier, used) = run(SeedAllocator::reach_controller());
        assert_eq!(modifier, SystemSeed(0), "state 8, since 9 is the parameter's own");
        let seeds = [27, 26, 9, 8].iter().fold(0, |used, state| used | 1 << state);
        assert_eq!(used & REACH_ALLOCATED_STATES, seeds, "and `system random` itself, which the emitter drops");
        assert_eq!(run(SeedAllocator::reach(0)).0, SystemSeed(1), "unmarked, it takes 9");
    }

    /// Reach seeds an emitter's properties in tag order, the axis angle
    /// fifth; Halo 3 and ODST seed the axis angle last.
    #[test]
    fn reach_seeds_the_emission_axis_angle_fifth_and_halo_3_last() {
        let seeded = |reach: bool| {
            let random = || varying(H3ParticleInput::Random, H3ParticleInput::Other, false);
            let mut properties: [_; 18] = std::array::from_fn(|_| random());
            let mut seeds = if reach { SeedAllocator::reach(0) } else { SeedAllocator::halo3() };
            for at in emitter_seed_order(reach) {
                properties[at].allocate(&mut seeds);
            }
            properties.map(|property| property.input)
        };
        assert_eq!(EMITTER_PROPERTIES[4], "emission axis angle");
        assert_eq!(seeded(true)[4], H3ParticleInput::Seed(3), "the fifth: state 7");
        assert_eq!(seeded(false)[4], H3ParticleInput::Seed(17 % SEEDS), "the eighteenth, counted");
        assert_eq!(seeded(false)[5], H3ParticleInput::Seed(4));
        assert_eq!(emitter_seed_order(false)[17], 4);
    }

    /// Reach's postprocess derives no near pair and keeps a feather of
    /// nothing, so it does not fade in at all.
    #[test]
    fn a_reach_lod_keeps_its_feathers_and_takes_no_near_pair() {
        assert_eq!(reach_baked_lod([0.0; 4]), [0.0, 20.0, 0.0, 5.0]);
        assert_eq!(reach_baked_lod([1.0, 8.0, 2.0, 3.0]), [1.0, 8.0, 2.0, 3.0]);
        let reach = Some(reach_baked_lod([0.0; 4]));
        assert_eq!(lod_amount(reach, 0.5), 1.0, "no fade in");
        let halo3 = Some(baked_lod([0.0; 4], true, [150.0].into_iter()));
        assert!(lod_amount(halo3, 0.5) < 1.0, "Halo 3's busy any-camera system fades in");
    }

    #[test]
    fn a_reach_near_fade_of_nothing_fades_over_a_hair() {
        assert_eq!(near_fade_range(true, 0.0), 1.0 / POSTPROCESS_EPSILON);
        assert_eq!(near_fade_range(true, 2.0), 0.5);
        assert!(near_fade_range(false, 0.0).is_infinite(), "Halo 3's as it was");
    }

    /// Reach's four new shapes, and the two it emits nothing from here.
    #[test]
    fn reachs_volumes_fill_their_boxes_and_a_plane_heads_along_x() {
        assert_eq!(H3EmissionShape::named("cube"), Some(H3EmissionShape::Cube));
        assert_eq!(H3EmissionShape::named("unweighted line"), Some(H3EmissionShape::UnweightedLine));
        assert_eq!(H3EmissionShape::named("boat hull surface"), None);
        assert_eq!(H3EmissionShape::named("jetwash"), None);
        let mut random = Random(11);
        for _ in 0..50 {
            let (place, heading) = emit(H3EmissionShape::Cube, 2.0, 0.0, false, &mut random);
            assert!(place.iter().all(|axis| axis.abs() <= 2.0) && heading == place, "out from the centre");
            let (place, heading) = emit(H3EmissionShape::Cylinder, 3.0, 0.0, false, &mut random);
            assert!(place[0].abs() <= 1.0, "two units long whatever the radius: {place:?}");
            assert!((place[1] * place[1] + place[2] * place[2]).sqrt() <= 3.0 && heading == place);
            let (place, heading) = emit(H3EmissionShape::Plane, 2.0, 0.0, false, &mut random);
            assert!(place.iter().all(|axis| axis.abs() <= 2.0) && heading == [1.0, 0.0, 0.0]);
            let (place, _) = emit(H3EmissionShape::UnweightedLine, 2.0, 0.0, false, &mut random);
            assert!(place[0] == 0.0 && place[2] == 0.0 && place[1].abs() <= 2.0);
        }
        let (place, heading) = emit(H3EmissionShape::Cube, 2.0, 0.0, true, &mut random);
        assert_ne!(heading, place, "volume velocities are random");
    }

    /// An emitter with nothing but `shape`, for what reads its fields alone.
    fn h4_terms(live_forever: bool, disable_velocity: bool, size_scale: f32) -> H4EmitterTerms {
        H4EmitterTerms { live_forever, disable_velocity, size_scale, scale: None, white_point: None }
    }

    /// `liveForever` brings the age round, and from past 0.99 back to 0;
    /// without it the age runs on to death.
    #[test]
    fn a_particle_that_lives_forever_comes_round() {
        let forever = h4_terms(true, false, 1.0);
        assert!((forever.age(1.25) - 0.25).abs() < 1e-6);
        assert_eq!(forever.age(0.995), 0.0);
        assert_eq!(forever.age(0.5), 0.5);
        assert_eq!(h4_terms(false, false, 1.0).age(1.25), 1.25);
    }

    /// `disableVelocity` holds it in place; otherwise the size scale
    /// multiplies the step.
    #[test]
    fn velocity_moves_by_the_size_scale_unless_disabled() {
        assert_eq!(h4_terms(false, true, 2.0).travel(0.5), 0.0);
        assert_eq!(h4_terms(false, false, 2.0).travel(0.5), 1.0);
    }

    /// Scale y and the size scale go into the size, scale x over scale y
    /// into the aspect; with neither the size scale alone.
    #[test]
    fn scale_x_and_y_bake_into_size_and_aspect() {
        let base = H3ParticleState::default();
        let none = h4_terms(false, false, 3.0);
        assert_eq!(none.size_and_aspect(&base), [3.0, 1.0]);
        let constant = |value: f32| H3ParticleProperty::fixed(value).unwrap();
        let scaled = H4EmitterTerms { scale: Some([constant(2.0), constant(0.5)]), ..h4_terms(false, false, 1.0) };
        assert_eq!(scaled.size_and_aspect(&base), [0.5, 4.0]);
    }

    fn test_emitter(shape: H3EmissionShape) -> H3Emitter {
        let fixed = || H3ParticleProperty::fixed(0.0).unwrap();
        H3Emitter {
            h4: None,
            update: H3ParticleUpdate::Gpu,
            card_lerps: false,
            card_at_rest: false,
            spawn_slack: 0.0,
            controllers: Vec::new(),
            shape,
            points: Vec::new(),
            scroll: [0.0; 2],
            axis: H3ParticleAxis::Constant,
            reference_axis: 0,
            axis_angle: fixed(),
            offset: fixed(),
            direction: fixed(),
            radius: fixed(),
            angle: fixed(),
            starting: fixed(),
            most: fixed(),
            rate: fixed(),
            lifespan: fixed(),
            acceleration: H3ParticleProperty::fixed_vector([0.0; 3]).unwrap(),
            speed: fixed(),
            rotation: fixed(),
            rotation_rate: fixed(),
            size: fixed(),
            scale: fixed(),
            tint: fixed(),
            alpha: fixed(),
            black_point: fixed(),
            physics: None,
            flock: None,
            collider: None,
            one_at_a_time: false,
            persistent: false,
            bunched: false,
            rotation_given: false,
            volume_random: false,
            axis_scale: [1.0; 3],
            turbulence: None,
        }
    }

    fn test_particle() -> Particle {
        Particle {
            id: 1,
            colliding: false,
            landed: false,
            resting: false,
            spawned_this_frame: false,
            on_card: false,
            axis: [0.0, 0.0, 1.0],
            age: 0.0,
            inverse_lifespan: 0.1,
            position: [0.0; 3],
            velocity: [0.0; 3],
            rotation: 0.0,
            rotation_rate: 0.0,
            frame: 0.0,
            frame_rate: 0.0,
            seeds: [0.0; SEEDS],
            emission_time: 0.0,
            flip: [false; 3],
            sequence: 0,
            system_seeds: [0.0; SYSTEM_SEEDS],
        }
    }

    /// `axis scale` scales the place after the shape, and all nothing is ones.
    #[test]
    fn reachs_axis_scale_flattens_what_its_shape_emits() {
        assert_eq!(reach_axis_scale([0.0; 3]), [1.0; 3]);
        assert_eq!(reach_axis_scale([0.0, 2.0, 2.0]), [0.0, 2.0, 2.0]);
        let emitter = H3Emitter { axis_scale: [2.0, 1.0, 0.0], ..test_emitter(H3EmissionShape::Plane) };
        let (place, _) = emitter.emitted(1.0, 0.0, &mut Random(5));
        let (unscaled, _) = emit(H3EmissionShape::Plane, 1.0, 0.0, false, &mut Random(5));
        assert_eq!(place, [unscaled[0] * 2.0, unscaled[1], 0.0]);
    }

    /// A 2 by 1 texture: red on the left, green on the right, half blue.
    fn turbulence_image() -> TextureImage {
        TextureImage {
            rgba: vec![255, 0, 128, 255, 0, 255, 128, 255],
            width: 2,
            height: 1,
            repeat_x: true,
            repeat_y: true,
            scale: [1.0; 2],
        }
    }

    /// The texel at `(birth, seed 5) · scale + offset`, wrapped and blended,
    /// less a half and times the strength over the frame.
    #[test]
    fn turbulence_pushes_by_a_wrapped_blended_texel_less_a_half() {
        let image = turbulence_image();
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1e-3);
        assert!(close(sample_wrapped(&image, [0.25, 0.5]), [1.0, 0.0, 0.502]));
        assert!(close(sample_wrapped(&image, [1.25, 0.5]), [1.0, 0.0, 0.502]), "wrapped");
        assert!(close(sample_wrapped(&image, [0.5, 0.5]), [0.5, 0.5, 0.502]), "blended");
        assert!(close(sample_wrapped(&image, [0.0, 0.5]), [0.5, 0.5, 0.502]), "blended across the wrap");
        // u = birth · 1 + 0.25, the left texel.
        let push = turbulence_push(&image, [1.0, 0.3, 0.25, 0.0], [0.0, 0.7], 2.0);
        assert!(close(push, [1.0, -1.0, 0.004]), "{push:?}");
    }

    /// On the card, turbulence moves a particle after its velocity does and
    /// before it accelerates, and not on its spawn frame.
    #[test]
    fn a_turbulent_particle_on_the_card_is_pushed_after_it_moves() {
        let turbulence = H3ParticleTurbulence {
            strength: H3ParticleProperty::fixed(2.0).unwrap(),
            xform: [1.0, 0.3, 0.25, 0.0].map(|value| H3ParticleProperty::fixed(value).unwrap()),
            image: turbulence_image(),
        };
        let emitter = H3Emitter { turbulence: Some(turbulence), ..test_emitter(H3EmissionShape::Sprayer) };
        let particle = Particle { spawned_this_frame: true, velocity: [1.0, 0.0, 0.0], ..test_particle() };
        let (mut particles, mut ended, dt) = (vec![particle], Vec::new(), 0.5);
        emitter.step(H3ParticleState::default(), &mut particles, dt, [0.0; 3], &mut ended, None, false);
        assert_eq!(particles[0].position, [0.0; 3], "the spawn frame");
        emitter.step(H3ParticleState::default(), &mut particles, dt, [0.0; 3], &mut ended, None, false);
        let [x, y, z] = particles[0].position;
        // Moved a half along x, then pushed by the left texel less a half,
        // (0.5, -0.5, 0.002), at a strength of 2 over half a second.
        let near = (x - (0.5 + 0.5)).abs() < 1e-4 && (y + 0.5).abs() < 1e-4 && (z - 0.00196).abs() < 1e-4;
        assert!(near, "{:?}", particles[0].position);
        let calm = test_emitter(H3EmissionShape::Sprayer);
        let mut particles = vec![Particle { velocity: [1.0, 0.0, 0.0], ..test_particle() }];
        calm.step(H3ParticleState::default(), &mut particles, dt, [0.0; 3], &mut ended, None, false);
        assert_eq!(particles[0].position, [0.5, 0.0, 0.0], "none without the movement");
    }

    /// Halo 4's `muzzle_smoke` pushes from (-0.1, 0.1, 0) to the opposite
    /// way. On Reach's and Halo 4's card that is a lerp, through nothing, so
    /// a particle keeps a sound velocity; Halo 3's turn has no way round and
    /// is not asked to find one here.
    #[test]
    fn reach_and_halo_4_lerp_opposite_accelerations_on_the_card() {
        let ends = [[-0.1, 0.1, 0.0], [0.1, -0.1, 0.0]];
        let acceleration = H3ParticleProperty { ends, ..H3ParticleProperty::fixed(0.25).unwrap() };
        assert!(acceleration.around(&H3ParticleState::default()).iter().any(|axis| axis.is_nan()), "the turn fails");
        let emitter = H3Emitter { card_lerps: true, acceleration, ..test_emitter(H3EmissionShape::Sprayer) };
        let mut particles = vec![Particle { velocity: [1.0, 0.0, 0.0], ..test_particle() }];
        let mut ended = Vec::new();
        emitter.step(H3ParticleState::default(), &mut particles, 1.0, [0.0; 3], &mut ended, None, false);
        let [x, y, z] = particles[0].velocity;
        let near = (x - 0.95).abs() < 1e-6 && (y - 0.05).abs() < 1e-6 && z == 0.0;
        assert!(near, "a quarter of the way from one end to the other: {:?}", particles[0].velocity);
    }

    /// Reach's additive blends take `v_alt_exposure.y`, the rest
    /// `v_exposure.x`, and multiply none; Halo 3's keep its own.
    #[test]
    fn reach_exposes_a_particle_by_its_blend() {
        assert_eq!(particle_exposure(H3Blend::Additive, true, true, 0.0), reach_alt_exposure(0.0)[1]);
        assert_eq!(particle_exposure(H3Blend::AddSrcTimesSrcAlpha, true, true, 0.0), reach_alt_exposure(0.0)[1]);
        assert_eq!(particle_exposure(H3Blend::AlphaBlend, true, true, 0.0), reach_exposure(0.0)[0]);
        assert_eq!(particle_exposure(H3Blend::Multiply, true, true, 0.0), 1.0);
        assert_eq!(particle_exposure(H3Blend::Additive, true, false, 0.0), H3_EXPOSURE);
        assert_eq!(particle_exposure(H3Blend::Additive, false, false, 0.0), 1.0, "Halo 2's show as they are");
    }

    /// The area screen effects' stops reach Reach's exposure and its
    /// constant self-illumination, and not Halo 3's.
    #[test]
    fn the_stops_move_reachs_particle_exposure_alone() {
        assert_eq!(particle_exposure(H3Blend::Additive, true, true, 1.0), reach_alt_exposure(1.0)[1]);
        assert_ne!(reach_alt_exposure(1.0)[1], reach_alt_exposure(0.0)[1]);
        assert_eq!(particle_exposure(H3Blend::AlphaBlend, true, true, -1.0), reach_exposure(-1.0)[0]);
        assert_eq!(particle_exposure(H3Blend::Additive, true, false, 1.0), H3_EXPOSURE);
        let mut shader = H3SpriteShader::blank();
        shader.add = [1.0; 3];
        assert_eq!(shader.add(1.0), [1.0; 3], "Halo 3's add as authored");
        shader.add_at_illum = true;
        assert_eq!(shader.add(0.0), [reach_alt_exposure(0.0)[1]; 3]);
        assert_eq!(shader.add(1.0), [reach_alt_exposure(1.0)[1]; 3]);
    }

    #[test]
    fn the_cheap_shader_faces_the_screen_or_camera_or_lies_flat() {
        assert_eq!(cheap_billboard(H3Billboard::ScreenFacing), H3Billboard::ScreenFacing);
        assert_eq!(cheap_billboard(H3Billboard::CameraFacing), H3Billboard::CameraFacing);
        assert_eq!(cheap_billboard(H3Billboard::ScreenParallel), H3Billboard::World);
        assert_eq!(cheap_billboard(H3Billboard::LocalVertical), H3Billboard::World);
    }

    /// Halo 4 births a vertically authored sprite a three-quarter turn round,
    /// so `parallel to direction` flames lie along their velocity, not across
    /// the view; a random start wins, and nothing else turns.
    #[test]
    fn halo_4_vertical_sprites_start_three_quarters_round() {
        assert_eq!(starting_rotation(false, true, || 0.3), 0.75);
        assert_eq!(starting_rotation(true, true, || 0.3), 0.3);
        assert_eq!(starting_rotation(false, false, || 0.3), 0.0);
    }

    /// `DOUBLE_SPRITE_TEXCOORD`: the next frame when frames blend, else a
    /// Reach plasma's sixteenth of seed 1 on.
    #[test]
    fn a_reach_plasma_reads_its_second_noise_a_seeded_number_of_frames_on() {
        assert_eq!(second_frame(2.5, false, true, 0.2), 5.0, "floor(2.5) + floor(3.2)");
        assert_eq!(second_frame(2.5, true, true, 0.2), 3.5, "blending, the next");
        assert_eq!(second_frame(2.5, false, false, 0.2), 3.5, "not Reach's plasma");
    }

    fn reach_source(root: &str) -> TagSource {
        loose_kit(root, GameId::HaloReach)
    }

    /// With `BLAM_TEST_HREK`: Reach's particle options read by name, and its
    /// seeds handed out from the top.
    #[test]
    fn kit_reach_particles_read_their_own_options_and_seeds() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            eprintln!("skipping: BLAM_TEST_HREK is not set");
            return;
        };
        let source = reach_source(&root);
        let plasma = read_look(&source, "fx\\particles\\plasma\\plasma_blue_medium").unwrap();
        assert!(plasma.reach && !plasma.cheap);
        assert_eq!(plasma.shader.albedo, H3ParticleAlbedo::Palette2dPlasma);
        assert!(plasma.shader.plasma_second_sprite);
        assert_eq!(plasma.shader.sphere_warp, Some(0.2));
        assert_eq!(plasma.shader.depth_fade, Some(0.25));
        assert_ne!(plasma.used_states & 1 << 25, 0, "its first random took seed 8: {:#x}", plasma.used_states);
        let paper = read_look(&source, "fx\\particles\\models\\debris\\blowing_paper\\blowing_paper").unwrap();
        assert!(paper.shader.smooth_contrast().is_some());
        let shards = read_look(&source, "fx\\particles\\models\\weapons\\needler_shards\\needler_shards").unwrap();
        assert!(shards.cheap && shards.shader.sphere_warp.is_none() && shards.shader.smooth_contrast().is_none());
    }

    /// With `BLAM_TEST_HREK`: `fly_swarm` emits from a plane flattened by its
    /// axis scale, with turbulence, on the card.
    #[test]
    fn kit_reach_swarm_emits_from_a_flattened_plane_with_turbulence() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            eprintln!("skipping: BLAM_TEST_HREK is not set");
            return;
        };
        let source = reach_source(&root);
        let path = "fx\\fx_library\\ambient\\nature\\bug_swarm\\fly_swarm";
        let effect = load_referenced_tag_from_source(&source, path, "effect", b"effe").unwrap();
        let systems = elements(effect.root(), "events").into_iter().flat_map(|event| elements(event, "particle systems"));
        let parts = systems.filter_map(|system| read_system_parts(&source, system));
        let emitters: Vec<_> = parts.flat_map(|parts| parts.emitters).collect();
        let plane = emitters.iter().find(|emitter| emitter.shape == H3EmissionShape::Plane).expect("a plane emitter");
        assert_eq!(plane.axis_scale, [0.0, 2.0, 2.0]);
        assert_eq!(plane.update, H3ParticleUpdate::Gpu);
        let turbulence = plane.turbulence.as_ref().expect("its turbulence");
        assert!(turbulence.image.width > 0 && turbulence.image.whole());
    }
}

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
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
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
        values: Option<&H2FunctionValues>,
        view: &CeFlareView,
        time: f32,
        draws: &mut Vec<H4EmitterDraw>,
    ) {
        use super::h4_particles::{H4ParticleState, pack_particle_state};
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
