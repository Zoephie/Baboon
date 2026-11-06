//! Halo 4's tracers (`tracer_system`), which replace Halo 3's and Reach's
//! beams and contrails. A point-to-point tracer runs between an effect part's
//! two locations and is worked out whole each frame, as
//! `tracer_stateless_setup.fx` does on the card; it is drawn through the
//! tracer's material from each profile's packed state
//! (`h4_tracer_pass`). A trailing tracer leaves profiles behind as its
//! location moves, so one on an object at rest draws nothing (the game's
//! `TracerGpu::Render` skips one profile or fewer); those are not built.
//! See `docs/h4-preview-research/tracers.md`.

use std::sync::Arc;

use blam_tags::{TagFunction, TagStruct};

use super::super::attachment::MarkerFrame;
use super::super::common::fields::{elements, has_flag};
use super::super::common::view::posed_on;
use super::super::functions::FunctionValues;
use super::super::dx9::passes::tag_function_at;
use super::super::gen3::effects::{
    H3EffectOnsets, H3EffectTimes, for_each_attached_part, part_frame,
    read_part_placement,
};
use super::super::particles::{read_ends, slerped};
use super::particles::{pack_argb8, pack_half2, pack_ushort2n};
use super::super::materials::{H4EffectMaterial, resolve_h4_effect_material};
use super::super::*;
use crate::app::export::geometry::load_referenced_tag_from_source;
use crate::core::source::TagSource;
use crate::app::model_preview::common::math::length;
use crate::app::model_preview::common::random::{high16_over_65535, high24_over_2_24, lcg_step};
use crate::app::model_preview::common::attached::attached;

/// `TracerGpu::k_maxProfilesPerInstance`.
const MOST_PROFILES: usize = 128;
/// `get_sort_bias`'s world units a step, as Reach's strips.
const SORT_BIAS_STEP: f32 = 0.05;
/// Degrees to radians, as the game writes it.
const RADIANS: f32 = 0.017_453_292;
/// Runtime appearance bits the postprocess adds (§5 step 7).
const ORIGIN_FADED: u32 = 1 << 6;
const EDGE_FADED: u32 = 1 << 7;
const ANGLE_FADED: u32 = 1 << 9;
/// `tracerAppearanceFlags`: `random u offset`, `random v offset`.
const RANDOM_U_OFFSET: u32 = 1 << 3;
const RANDOM_V_OFFSET: u32 = 1 << 4;

/// `tracerStateInputEnum`, by index; `profile random` and `tracer random`
/// never reach the card, the postprocess swapping each for a seed.
const STATE_COUNT: usize = 19;
const PROFILE_RANDOM: u8 = 3;
const PROFILE_SEEDS: std::ops::RangeInclusive<u8> = 4..=7;
const TRACER_RANDOM: u8 = 9;
const TRACER_SEED_1: u8 = 10;
const TRACER_SEED_2: u8 = 11;
const INVALID: u8 = 18;
/// `m_usedStates &= 0x7FDF5`: not percentile, profile random or tracer random.
const UPLOADED_STATES: u32 = 0x7_FDF5;

/// `tracerStateInputEnum`'s index for an option name.
fn state_named(name: &str) -> u8 {
    const NAMES: [&str; STATE_COUNT - 1] = [
        "profile age",
        "profile position",
        "unused",
        "profile random",
        "profile correlation 1",
        "profile correlation 2",
        "profile correlation 3",
        "profile correlation 4",
        "game time",
        "tracer random",
        "tracer correlation 1",
        "tracer correlation 2",
        "tracer speed",
        "tracer length",
        "system age",
        "system lod",
        "effect a scale",
        "effect b scale",
    ];
    NAMES
        .iter()
        .position(|known| *known == name)
        .map_or(INVALID, |at| at as u8)
}

/// `TracerStates` and a profile's own: the value of each state by index.
#[derive(Debug, Clone, Copy, Default)]
struct TracerState {
    profile_age: f32,
    percentile: f32,
    profile_seeds: [f32; 4],
    game_time: f32,
    tracer_seeds: [f32; 2],
    speed: f32,
    length: f32,
    system_age: f32,
    lod: f32,
    effect_a: f32,
    effect_b: f32,
}

impl TracerState {
    fn of(&self, index: u8) -> f32 {
        match index {
            0 => self.profile_age,
            1 => self.percentile,
            4..=7 => self.profile_seeds[usize::from(index - 4)],
            8 => self.game_time,
            10 | 11 => self.tracer_seeds[usize::from(index - 10)],
            12 => self.speed,
            13 => self.length,
            14 => self.system_age,
            15 => self.lod,
            16 => self.effect_a,
            17 => self.effect_b,
            _ => 0.0,
        }
    }
}

/// `tracerProperty_*`.
#[derive(Debug)]
struct TracerProperty {
    input: u8,
    range: u8,
    /// `Plus`, else `Times`, by `modifier`.
    adds: Option<bool>,
    modifier: u8,
    function: TagFunction,
    ends: [[f32; 3]; 2],
}

impl TracerProperty {
    fn read(holder: TagStruct<'_>, name: &str) -> Option<Self> {
        let property = holder.descend(name)?;
        let input = |field: &str| state_named(&property.read_enum_option(field).unwrap_or_default());
        Some(Self {
            input: input("Input Variable"),
            range: input("Range Variable"),
            adds: match property.read_enum_option("Output Modifier").as_deref() {
                Some("Plus") => Some(true),
                Some("Times") => Some(false),
                _ => None,
            },
            modifier: input("Output Modifier Input"),
            function: tag_function_at(&property, "Mapping")?,
            ends: read_ends(property),
        })
    }

    /// The states it reads: its input, its range when ranged, its modifier's
    /// when it has one; none for a constant.
    fn inputs(&self) -> impl Iterator<Item = u8> + '_ {
        let constant = self.function.is_constant();
        let range = self.function.is_ranged().then_some(self.range);
        let modifier = self.adds.map(|_| self.modifier);
        [Some(self.input), range, modifier]
            .into_iter()
            .flatten()
            .filter(move |_| !constant)
    }

    fn used_states(&self) -> u32 {
        self.inputs().fold(0, |mask, input| mask | (1 << input))
    }

    /// `TracerStates::AllocateRandomSeed` (X 0x8382B928) for this property's
    /// turn: `profile random` takes the profile correlation above the highest
    /// used, `tracer random` the first tracer correlation free.
    fn allocate_seeds(&mut self, used: &mut u32) {
        if self.function.is_constant() {
            return;
        }
        let ranged = self.function.is_ranged();
        let modified = self.adds.is_some();
        for (input, reads) in [
            (&mut self.input, true),
            (&mut self.range, ranged),
            (&mut self.modifier, modified),
        ] {
            if !reads {
                continue;
            }
            let seed = match *input {
                PROFILE_RANDOM => {
                    let profile = *used & 0xF0;
                    if profile == 0 {
                        4
                    } else {
                        (31 - profile.leading_zeros()) as u8 + 1
                    }
                }
                TRACER_RANDOM => {
                    if *used & (1 << TRACER_SEED_2) != 0 {
                        INVALID
                    } else if *used & (1 << TRACER_SEED_1) != 0 {
                        TRACER_SEED_2
                    } else {
                        TRACER_SEED_1
                    }
                }
                _ => continue,
            };
            // Past the last correlation the game logs and cycles [inferred: no kit tag does].
            let seed = if *input == PROFILE_RANDOM && !PROFILE_SEEDS.contains(&seed) {
                4
            } else {
                seed
            };
            *input = seed;
            *used |= 1 << seed;
        }
    }

    fn scalar(&self, state: &TracerState) -> f32 {
        let value = self
            .function
            .evaluate(state.of(self.input), state.of(self.range));
        match self.adds {
            Some(true) => value + state.of(self.modifier),
            Some(false) => value * state.of(self.modifier),
            None => value,
        }
    }

    fn color(&self, state: &TracerState) -> [f32; 3] {
        let color = self
            .function
            .evaluate_color(state.of(self.input), state.of(self.range));
        [color.red, color.green, color.blue]
    }

    /// `MapToVector2dRange`: a slerp between the two ends.
    fn vector(&self, state: &TracerState) -> [f32; 3] {
        slerped(self.ends, self.scalar(state))
    }

    /// `get_max`: the larger end of the function's range, its low end alone
    /// for a constant.
    fn most(&self) -> f32 {
        let (low, high) = match &self.function {
            TagFunction::Blob(blob) => {
                (blob.header().clamp_range_min, blob.header().clamp_range_max)
            }
            TagFunction::H2(h2) => (h2.clamp_range_min(), h2.clamp_range_max()),
        };
        low.max(if self.function.is_constant() {
            low
        } else {
            high
        })
    }
}

/// The tag's properties, in its order (the seed allocation's).
const PROPERTY_NAMES: [&str; 15] = [
    "profile lifespan",
    "profile self acceleration",
    "length",
    "offset",
    "profile size",
    "profile density",
    "profile offset",
    "profile rotation",
    "profile rotation rate",
    "profile color",
    "profile alpha",
    "profile alpha2",
    "profile black point",
    "profile palette",
    "profile intensity",
];

/// One of a `tracer_system`'s `tracers` as `PostprocessBlockCallback`
/// (X 0x831FC270) leaves it, but for its material.
#[derive(Debug)]
pub(crate) struct TracerDefinition {
    /// `profile shape`: 0 ribbon, 1 cross, 2 n-gon, 3 horizontal, 4 vertical.
    shape: u32,
    ngon_sides: u32,
    /// The tag's appearance flags and the runtime fade bits.
    appearance: u32,
    /// `1/origin range, origin cutoff, 1/edge range, edge cutoff`, the edge
    /// pair in radians.
    fade: [f32; 4],
    /// `origin fade cutoff` and `range`, when it fades from its origin:
    /// trailing profiles are split at both ends (`Tracer::Extend`).
    origin_fade: Option<[f32; 2]>,
    lifespan: TracerProperty,
    acceleration: TracerProperty,
    length: TracerProperty,
    offset: TracerProperty,
    density: TracerProperty,
    size: TracerProperty,
    profile_offset: TracerProperty,
    rotation: TracerProperty,
    color: TracerProperty,
    alpha: TracerProperty,
    alpha2: TracerProperty,
    black_point: TracerProperty,
    palette: TracerProperty,
    intensity: TracerProperty,
    uv_tiling: [f32; 2],
    uv_scroll: [f32; 2],
    sort_bias: f32,
    /// `m_maxProfileCount`: none draws at 0.
    most_profiles: i32,
}

/// A tracer and the material it draws through.
#[derive(Debug)]
pub(crate) struct H4TracerDefinition {
    tracer: TracerDefinition,
    pub(crate) material: H4EffectMaterial,
    sort_layer: i32,
}

impl std::ops::Deref for H4TracerDefinition {
    type Target = TracerDefinition;

    fn deref(&self) -> &TracerDefinition {
        &self.tracer
    }
}

/// `tracerProfileShapeEnum`'s index.
fn shape_named(name: &str) -> u32 {
    match name {
        "cross" => 1,
        "n-gon" => 2,
        "horizontal" => 3,
        "vertical" => 4,
        _ => 0,
    }
}

fn read_tracer(source: &TagSource, element: TagStruct<'_>) -> Option<H4TracerDefinition> {
    if has_flag(element, "flags", "disabled for debugging") {
        return None;
    }
    let material =
        blam_tags::material::Material::from_struct(&element.descend("actual material?")?);
    let resolved = resolve_h4_effect_material(source, &material, "tracer")
        .map_err(|error| eprintln!("h4 tracer material: {error}"))
        .ok()?;
    let mut properties = Vec::with_capacity(PROPERTY_NAMES.len());
    for name in PROPERTY_NAMES {
        properties.push(TracerProperty::read(element, name)?);
    }
    // The used states, then each property's seeds in turn.
    let mut used = properties
        .iter()
        .fold(0, |mask, property| mask | property.used_states())
        & UPLOADED_STATES;
    for property in &mut properties {
        property.allocate_seeds(&mut used);
    }
    let [
        lifespan,
        acceleration,
        length,
        offset,
        size,
        density,
        profile_offset,
        rotation,
        _rotation_rate,
        color,
        alpha,
        alpha2,
        black_point,
        palette,
        intensity,
    ]: [TracerProperty; 15] = properties.try_into().ok()?;
    let real = |name: &str| element.read_real(name).unwrap_or(0.0);
    let (origin_range, origin_cutoff) = (real("origin fade range"), real("origin fade cutoff"));
    let (edge_range, edge_cutoff) = (real("edge fade range"), real("edge fade cutoff"));
    let mut appearance = element.read_int_any("appearance flags").unwrap_or(0) as u32 & 0x3F;
    if origin_range != 0.0 || origin_cutoff != 0.0 {
        appearance |= ORIGIN_FADED;
    }
    if edge_range != 0.0 || edge_cutoff != 0.0 {
        appearance |= EDGE_FADED;
    }
    if real("angle fade range") != 0.0 || real("angle fade begin") != 0.0 {
        appearance |= ANGLE_FADED;
    }
    let vector = |name: &str| {
        let vector = element.read_vec2(name);
        [vector.i, vector.j]
    };
    // `uv tiling` of 0 is 1, each way.
    let uv_tiling = vector("uv tiling").map(|tiling| if tiling == 0.0 { 1.0 } else { tiling });
    let most_profiles = (length.most() * density.most()) as i32;
    let tracer = TracerDefinition {
        shape: shape_named(&element.read_enum_option("profile shape").unwrap_or_default()),
        ngon_sides: element
            .read_int_any("number of n-gon sides")
            .unwrap_or(0)
            .max(0) as u32,
        appearance,
        fade: [
            origin_range.recip(),
            origin_cutoff,
            (edge_range * RADIANS).recip(),
            edge_cutoff * RADIANS,
        ],
        origin_fade: (appearance & ORIGIN_FADED != 0).then_some([origin_cutoff, origin_range]),
        lifespan,
        acceleration,
        length,
        offset,
        density,
        size,
        profile_offset,
        rotation,
        color,
        alpha,
        alpha2,
        black_point,
        palette,
        intensity,
        uv_tiling,
        uv_scroll: vector("uv scrolling"),
        sort_bias: element.read_int_any("sort bias").unwrap_or(0) as f32 * SORT_BIAS_STEP,
        most_profiles,
    };
    Some(H4TracerDefinition {
        tracer,
        sort_layer: i32::from(material.sort_layer).clamp(1, 3),
        material: resolved,
    })
}

impl TracerDefinition {
    /// Ribbons, horizontals and verticals are one strip, a cross two, an
    /// n-gon one a side.
    fn strips(&self) -> u32 {
        match self.shape {
            1 => 2,
            2 => self.ngon_sides,
            _ => 1,
        }
    }
}

/// A system's `far lod` pair and the values at each end, as the postprocess
/// leaves them.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TracerLod {
    start: f32,
    end: f32,
    start_value: f32,
    end_value: f32,
}

impl TracerLod {
    /// `PostprocessCallback`: with neither value set, 1 near and 0 far.
    fn read(root: TagStruct<'_>) -> Self {
        let real = |name: &str| root.read_real(name).unwrap_or(0.0);
        let (mut start_value, mut end_value) =
            (real("far lod start value"), real("far lod end value"));
        if start_value < 1e-4 && end_value < 1e-4 {
            (start_value, end_value) = (1.0, 0.0);
        }
        Self {
            start: real("far lod start"),
            end: real("far lod end"),
            start_value,
            end_value,
        }
    }

    /// `ComputeLod` (X 0x834DA128) at `distance`, the camera's scale taken
    /// as 1 \[open\].
    fn at(&self, distance: f32) -> f32 {
        let span = self.end - self.start;
        let t = if span == 0.0 {
            0.0
        } else {
            ((distance - self.start) / span).clamp(0.0, 1.0)
        };
        self.start_value + t * (self.end_value - self.start_value)
    }
}

/// Where a tracer runs.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TracerPath {
    /// From its location to the part's secondary location.
    PointToPoint { end: (usize, [f32; 3]) },
    /// Behind its location as it moves, its forward the marker's.
    Trailing { forward: [f32; 3] },
}

/// A tracer at a marker of the object it is attached to.
#[derive(Debug, Clone)]
pub(crate) struct H4TracerAt {
    pub(crate) object: u8,
    start: (usize, [f32; 3]),
    path: TracerPath,
    only_under: Option<(String, String)>,
    scale_by: String,
    secondary_by: String,
    /// `real_random_range(0, 1)` twice, the tracer's correlations.
    seeds: [f32; 2],
    times: Arc<H3EffectTimes>,
    event: usize,
    lod: TracerLod,
    pub(crate) definition: Arc<H4TracerDefinition>,
}

/// The tracer's two correlations, a generator's for each tracer in turn.
fn seeds_of(index: usize) -> [f32; 2] {
    let first = lcg_step((index as u32).wrapping_mul(2_654_435_761) | 1);
    [first, lcg_step(first)].map(high24_over_2_24)
}

/// The tracers of an object's attached effects, one for each location (and,
/// point to point, each location pair). A part runs only when the system's
/// `point-to-point` agrees with its having a secondary location
/// (`UpdateLocations` X 0x8296EB50).
pub(in crate::app::model_preview) fn read_h4_tracers(
    source: &TagSource,
    object: &blam_tags::TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H4TracerAt> {
    let mut tracers = Vec::new();
    let tracer_group = u32::from_be_bytes(*b"trac");
    for_each_attached_part(source, object, |part| {
        if part.group != tracer_group {
            return;
        }
        let Ok(tag) = load_referenced_tag_from_source(source, &part.path, "tracer_system", b"trac")
        else {
            return;
        };
        let root = tag.root();
        let point_to_point = has_flag(root, "tracer system flags", "point-to-point");
        if point_to_point != part.secondary_marker.is_some() {
            // "… It won't render."
            return;
        }
        let lod = TracerLod::read(root);
        let definitions: Vec<Arc<H4TracerDefinition>> = elements(root, "tracers")
            .into_iter()
            .filter_map(|element| read_tracer(source, element))
            .map(Arc::new)
            .collect();
        let (offset, yaw, pitch) = read_part_placement(part.part);
        let starts = part.effect.locations(markers, &part.marker, &frame);
        let ends = match &part.secondary_marker {
            Some(secondary) => part
                .effect
                .locations(markers, secondary, &frame)
                .into_iter()
                .map(Some)
                .collect(),
            None => vec![None; starts.len()],
        };
        for (start, end) in starts.iter().zip(ends) {
            let (place, node) = frame(start);
            let (place, node) = part.placed(place, node);
            let place = part_frame(&place, offset, yaw, pitch);
            let path = match end {
                Some(end) => {
                    let (far, far_node) = frame(&end);
                    TracerPath::PointToPoint {
                        end: (far_node, far.position),
                    }
                }
                None => TracerPath::Trailing {
                    forward: place.axes[0],
                },
            };
            for definition in &definitions {
                tracers.push(H4TracerAt {
                    object: owner,
                    start: (node, place.position),
                    path,
                    only_under: start.only_under.clone(),
                    scale_by: part.effect.scale_by.clone(),
                    secondary_by: part.effect.secondary_by.clone(),
                    seeds: seeds_of(tracers.len()),
                    times: Arc::clone(&part.times),
                    event: part.event,
                    lod,
                    definition: Arc::clone(definition),
                });
            }
        }
    });
    tracers
}

/// `TracerProfileState`, as the card holds it between frames.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct ProfileState {
    position: [f32; 3],
    /// 0 to 1 over its lifespan; the vertex shader drops it at 1.
    age: f32,
    velocity: [f32; 3],
    initial_alpha: f32,
    random: [f32; 4],
    size: f32,
    intensity: f32,
    offset: [f32; 2],
    /// Turns.
    rotation: f32,
    black_point: f32,
    palette: f32,
    percentile: f32,
    /// How far along the tracer it is, in world units.
    length: f32,
    lifespan: f32,
    color: [f32; 4],
    /// `a` is an exponent: the colour goes by `2^a`.
    initial_color: [f32; 4],
}

/// `RawTracerProfileState`'s sixteen words (`tracer_parameters.fxh`).
fn pack_profile(profile: &ProfileState) -> [u32; 16] {
    let [x, y, z] = profile.position.map(f32::to_bits);
    let [vx, vy, vz] = profile.velocity;
    let [r0, r1, r2, r3] = profile.random;
    [
        x,
        y,
        z,
        profile.age.to_bits(),
        pack_half2(vx, vy),
        pack_half2(vz, profile.initial_alpha),
        pack_ushort2n(r0, r1),
        pack_ushort2n(r2, r3),
        pack_half2(profile.size, profile.intensity),
        pack_half2(profile.offset[0], profile.offset[1]),
        pack_ushort2n(profile.rotation, profile.black_point),
        pack_ushort2n(profile.palette, profile.percentile),
        pack_half2(profile.length, profile.lifespan),
        pack_argb8(profile.color),
        pack_argb8(profile.initial_color),
        0,
    ]
}

impl TracerDefinition {
    /// What `tracer_stateless_setup.fx` and `tracer_update.fx` both set from
    /// the properties: rotation, colour, size, offset, intensity, black
    /// point and palette.
    fn apply_properties(&self, state: &TracerState, profile: &mut ProfileState) {
        let [red, green, blue] = self.color.color(state);
        let [x, y, _] = self.profile_offset.vector(state);
        profile.rotation = self.rotation.scalar(state).rem_euclid(1.0);
        profile.color = [
            red,
            green,
            blue,
            self.alpha.scalar(state) * self.alpha2.scalar(state),
        ];
        profile.size = self.size.scalar(state);
        profile.offset = [x, y];
        profile.intensity = self.intensity.scalar(state);
        profile.black_point = self.black_point.scalar(state).rem_euclid(1.0);
        profile.palette = self.palette.scalar(state).rem_euclid(1.0);
    }
}

/// One tracer's frame: `vs_tracerOverallState` and its profiles' packed
/// states, in strip order.
#[derive(Debug, Clone)]
pub(crate) struct H4TracerDraw {
    pub(crate) shape: f32,
    pub(crate) ngon_sides: f32,
    pub(crate) appearance: f32,
    pub(crate) percentile_step: f32,
    pub(crate) capped_length: f32,
    pub(crate) offset: f32,
    /// `localSpaceOffset_gameTime`: no offset, and the seconds since the
    /// system began.
    pub(crate) game_time: f32,
    pub(crate) uv_tiling: [f32; 2],
    pub(crate) uv_scroll: [f32; 2],
    pub(crate) uv_offset: [f32; 2],
    pub(crate) origin: [f32; 3],
    pub(crate) direction: [f32; 3],
    pub(crate) fade: [f32; 4],
    pub(crate) inputs: [f32; STATE_COUNT],
    pub(crate) strips: u32,
    pub(crate) states: Vec<[u32; 16]>,
}


/// `1 / (length · density)`, held to 0..1, an infinite step 1.
fn percentile_step(length: f32, density: f32) -> f32 {
    let step = (length * density).recip();
    if step.is_nan() || step <= 0.0 {
        0.0
    } else {
        step.min(1.0)
    }
}

/// `clamp(ceil(length · density), 2, 128)`.
fn profile_count(length: f32, density: f32) -> usize {
    let wanted = (length * density).ceil();
    if wanted.is_nan() {
        2
    } else {
        (wanted.max(2.0) as usize).min(MOST_PROFILES)
    }
}

/// The per-tracer part of a frame: its states, and the seconds its system
/// has run.
struct TracerFrame {
    state: TracerState,
    since: f32,
    /// Its event is running and its effect on: it lays profiles down.
    live: bool,
}

impl H4TracerAt {
    pub(crate) fn scale_names(&self) -> [&str; 2] {
        [&self.scale_by, &self.secondary_by]
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }


    pub(crate) fn sort_layer(&self) -> i32 {
        self.definition.sort_layer
    }

    /// World units nearer the camera it sorts.
    pub(crate) fn sort_bias(&self) -> f32 {
        self.definition.sort_bias
    }

    /// Its location this frame, and its forward.
    fn location(&self, bones: Option<&[[f32; 4]]>) -> ([f32; 3], [f32; 3]) {
        let forward = match self.path {
            TracerPath::Trailing { forward } => forward,
            TracerPath::PointToPoint { .. } => [1.0, 0.0, 0.0],
        };
        posed_on(self.start.0, self.start.1, forward, bones)
    }

    /// Its sort position: `Tracer::Submit` hands over its location's.
    pub(crate) fn origin(&self, bones: Option<&[[f32; 4]]>) -> [f32; 3] {
        self.location(bones).0
    }

    /// `TracerStates::UpdateStates` (X 0x82991E10) at `time`, from `at`.
    fn frame(
        &self,
        values: Option<&FunctionValues>,
        at: [f32; 3],
        eye: [f32; 3],
        time: f32,
        onset: f32,
    ) -> TracerFrame {
        let read = |name: &str| {
            if name.is_empty() {
                return 1.0;
            }
            values
                .and_then(|values| values.get(name))
                .copied()
                .unwrap_or(0.0)
        };
        let since = time - onset;
        let effect_a = read(&self.scale_by);
        let age = self.times.age(self.event, since);
        let from_eye: [f32; 3] = std::array::from_fn(|axis| at[axis] - eye[axis]);
        TracerFrame {
            state: TracerState {
                game_time: time,
                tracer_seeds: self.seeds,
                system_age: age.unwrap_or(1.0),
                lod: self.lod.at(length(from_eye)),
                effect_a,
                effect_b: read(&self.secondary_by),
                ..Default::default()
            },
            since,
            live: effect_a > 0.0 && age.is_some(),
        }
    }

    /// `SetMaterialState`'s overall state around `states`.
    #[allow(clippy::too_many_arguments)]
    fn draw_of(
        &self,
        state: &TracerState,
        since: f32,
        [step, capped, offset]: [f32; 3],
        origin: [f32; 3],
        direction: [f32; 3],
        states: Vec<[u32; 16]>,
    ) -> H4TracerDraw {
        let tracer = &*self.definition;
        let random = |flag: u32, seed: f32| {
            if tracer.appearance & flag != 0 {
                seed
            } else {
                0.0
            }
        };
        H4TracerDraw {
            shape: tracer.shape as f32,
            ngon_sides: tracer.ngon_sides as f32,
            appearance: tracer.appearance as f32,
            percentile_step: step,
            capped_length: capped,
            offset,
            game_time: since.max(0.0),
            uv_tiling: tracer.uv_tiling,
            uv_scroll: tracer.uv_scroll,
            uv_offset: [
                random(RANDOM_U_OFFSET, self.seeds[0]),
                random(RANDOM_V_OFFSET, self.seeds[1]),
            ],
            origin,
            direction,
            fade: tracer.fade,
            inputs: std::array::from_fn(|index| {
                if index == 1 {
                    0.0
                } else {
                    state.of(index as u8)
                }
            }),
            strips: tracer.strips(),
            states,
        }
    }

    /// Whether it trails its location, and so runs in an [`H4TracerField`].
    pub(crate) fn trails(&self) -> bool {
        matches!(self.path, TracerPath::Trailing { .. })
    }

    /// A point-to-point tracer's frame at `time`, or none while its event is
    /// not running, its effect is off or it can make no profiles.
    pub(crate) fn draw(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        time: f32,
        onset: f32,
    ) -> Option<H4TracerDraw> {
        let TracerPath::PointToPoint { end } = self.path else {
            return None;
        };
        let tracer = &*self.definition;
        if tracer.most_profiles <= 0 {
            return None;
        }
        let start = self.origin(bones);
        let TracerFrame {
            mut state,
            since,
            live,
        } = self.frame(values, start, eye, time, onset);
        if !live {
            return None;
        }
        let end = posed_on(end.0, end.1, [1.0, 0.0, 0.0], bones).0;
        // `SetStatelessPositionAndLength`: a beam of no length runs forward.
        let span: [f32; 3] = std::array::from_fn(|axis| end[axis] - start[axis]);
        let cap = length(span);
        let direction = if cap > 0.0 {
            span.map(|axis| axis / cap)
        } else {
            [1.0, 0.0, 0.0]
        };
        // `Tracer::FrameAdvance`: the length held to the gap between the two.
        let length = tracer.length.scalar(&state);
        let capped = if cap == 0.0 || cap > length {
            length
        } else {
            cap
        };
        state.length = capped;
        let offset = tracer.offset.scalar(&state);
        let density = tracer.density.scalar(&state);
        let count = profile_count(capped, density);
        let step = percentile_step(capped, density);
        // `tracer_stateless_setup.fx`: no age, velocity or random, the
        // initial colour white at an exponent of 1, the initial alpha 1.
        let states = (0..count)
            .map(|index| {
                let percentile = if index == count - 1 {
                    1.0
                } else {
                    index as f32 * step
                };
                state.percentile = percentile;
                let mut profile = ProfileState {
                    position: std::array::from_fn(|axis| {
                        start[axis] + direction[axis] * (offset + capped * percentile)
                    }),
                    initial_alpha: 1.0,
                    percentile,
                    length: percentile * capped,
                    initial_color: [1.0; 4],
                    ..Default::default()
                };
                tracer.apply_properties(&state, &mut profile);
                pack_profile(&profile)
            })
            .collect();
        Some(self.draw_of(
            &state,
            since,
            [step, capped, offset],
            start,
            direction,
            states,
        ))
    }
}

/// `TracerGpu::k_profilesPerRow`.
const PROFILES_PER_ROW: usize = 16;
/// `Tracer::k_maxProfilesSpawnedPerFrame`, `…Constrained`, and the count
/// past which the constrained one holds (`k_profileSpawnConstraintThreshold`).
const SPAWNED_PER_FRAME: usize = 10;
const SPAWNED_PER_FRAME_CONSTRAINED: usize = 2;
const SPAWN_CONSTRAINT_THRESHOLD: usize = 25;
/// `tracerAppearanceFlags`: `profile opacity from scale a`.
const OPACITY_FROM_SCALE_A: u32 = 1 << 2;
/// What `Tracer::Extend` and `PlaceProfilesToPoint` take for no distance.
const PLACE_EPSILON: f32 = 1e-4;

/// A row of the card's profiles: they are freed together once the longest
/// lifespan among them has run (`TracerGpu::FrameAdvance`).
#[derive(Debug, Clone, Default)]
struct TrailRow {
    slots: Vec<ProfileState>,
    timer: f32,
}

/// A trailing tracer's `Tracer` and `TracerGpu` between frames.
#[derive(Debug, Clone, Default)]
struct RunningTracer {
    /// Oldest first, all but the newest full.
    rows: Vec<TrailRow>,
    /// The last real profile's place, and whether there is one yet.
    last_placed: [f32; 3],
    placed_real: bool,
    /// The last real profile's length along the trail (`Tracer+60`).
    length: f32,
    /// The newest slot holds the temporary leading profile.
    leading_temporary: bool,
    location: Option<[f32; 3]>,
    speed: f32,
    random: u32,
}

impl RunningTracer {
    fn next_seed(&mut self) -> f32 {
        self.random = lcg_step(self.random);
        high16_over_65535(self.random)
    }

    fn count(&self) -> usize {
        self.rows.iter().map(|row| row.slots.len()).sum()
    }

    /// `QueueProfile` / `AllocateProfile` (X 0x82BA4A78): a temporary
    /// profile last time is written over; else the newest row takes it, or a
    /// new one does. No more than a strip holds.
    fn queue(&mut self, profile: ProfileState, temporary: bool) {
        let timer = (1.0 - profile.age) * profile.lifespan;
        if self.leading_temporary
            && let Some(row) = self.rows.last_mut()
            && let Some(slot) = row.slots.last_mut()
        {
            *slot = profile;
            row.timer = row.timer.max(timer);
            self.leading_temporary = temporary;
            return;
        }
        if self.count() >= MOST_PROFILES {
            return;
        }
        if self
            .rows
            .last()
            .is_none_or(|row| row.slots.len() >= PROFILES_PER_ROW)
        {
            self.rows.push(TrailRow::default());
        }
        let row = self.rows.last_mut().expect("a row");
        row.slots.push(profile);
        row.timer = row.timer.max(timer);
        self.leading_temporary = temporary;
    }

    /// `CreateProfile` and `InitializeProfile` (X 0x829C2A98, 0x82B9B070):
    /// `distance` on from the last real profile.
    fn create(
        &mut self,
        tracer: &TracerDefinition,
        state: &TracerState,
        temporary: bool,
        direction: [f32; 3],
        distance: f32,
    ) {
        // The temporary leading profile is reused, its seeds and all.
        let reused = self
            .leading_temporary
            .then(|| self.rows.last().and_then(|row| row.slots.last()))
            .flatten();
        let random = match reused {
            Some(profile) => profile.random,
            None => std::array::from_fn(|_| self.next_seed()),
        };
        let profile_state = TracerState {
            profile_seeds: random,
            ..*state
        };
        let position =
            std::array::from_fn(|axis| self.last_placed[axis] + direction[axis] * distance);
        let length = if self.placed_real {
            self.length + distance
        } else {
            0.0
        };
        let opacity_from_a = tracer.appearance & OPACITY_FROM_SCALE_A != 0;
        let profile = ProfileState {
            position,
            random,
            length,
            lifespan: tracer.lifespan.scalar(&profile_state),
            initial_alpha: if opacity_from_a { state.effect_a } else { 1.0 },
            // No lightmap: white at an exponent of 0.
            initial_color: [1.0, 1.0, 1.0, 0.0],
            ..Default::default()
        };
        if !temporary {
            self.placed_real = true;
            self.last_placed = position;
            self.length = length;
        }
        self.queue(profile, temporary);
    }

    /// `Tracer::Extend` (X 0x829C2840): an origin fade gets profiles at
    /// both its ends; with none, the first stretch gets a tiny one.
    fn extend(
        &mut self,
        tracer: &TracerDefinition,
        state: &TracerState,
        temporary: bool,
        direction: [f32; 3],
        by: f32,
    ) {
        let mut current = self.length;
        let last = current + by;
        if let Some([cutoff, range]) = tracer.origin_fade {
            for boundary in [cutoff, cutoff + range] {
                if current + PLACE_EPSILON < boundary && boundary < last - PLACE_EPSILON {
                    self.create(tracer, state, false, direction, boundary - current);
                    current = boundary;
                }
            }
        } else if current < PLACE_EPSILON && last > PLACE_EPSILON {
            self.create(tracer, state, false, direction, 2.0 * PLACE_EPSILON);
        }
        self.create(tracer, state, temporary, direction, last - current);
    }

    /// `PlaceProfilesToPoint` (X 0x82995628): profiles a density apart up to
    /// `at`, no more than the frame's allowance, and a temporary one at `at`
    /// itself while the effect runs.
    fn place_to(
        &mut self,
        tracer: &TracerDefinition,
        state: &TracerState,
        at: [f32; 3],
        spawned: &mut usize,
    ) {
        let moved: [f32; 3] = std::array::from_fn(|axis| at[axis] - self.last_placed[axis]);
        let distance = length(moved);
        if self.placed_real && distance <= PLACE_EPSILON {
            return;
        }
        let direction = if distance > 0.0 {
            moved.map(|axis| axis / distance)
        } else {
            [0.0; 3]
        };
        let wanted = distance * tracer.density.scalar(state);
        let allowance = if *spawned < SPAWN_CONSTRAINT_THRESHOLD {
            SPAWNED_PER_FRAME
        } else {
            SPAWNED_PER_FRAME_CONSTRAINED
        };
        let whole = if wanted.is_finite() {
            (wanted.max(0.0) as usize).min(allowance)
        } else {
            allowance
        };
        *spawned += whole;
        let steps = wanted.min(whole as f32);
        let mut remaining = distance;
        let mut left = steps;
        while left >= 1.0 {
            let step = distance / steps;
            self.extend(tracer, state, false, direction, step);
            remaining -= step;
            left -= 1.0;
        }
        if !self.placed_real || remaining > PLACE_EPSILON {
            self.extend(tracer, state, true, direction, remaining);
        }
    }

    /// `TracerGpu::FrameAdvance`'s spawned branch: rows past their time go;
    /// the rest age, move and take their properties (`tracer_update.fx`),
    /// each by its buffer slot's place.
    fn update(&mut self, tracer: &TracerDefinition, state: &TracerState, dt: f32) {
        self.rows.retain(|row| row.timer >= 0.0);
        if self.rows.is_empty() {
            self.leading_temporary = false;
        }
        let count = self.count();
        let step = percentile_step(self.length, tracer.density.scalar(state));
        for (at, row) in self.rows.iter_mut().enumerate() {
            row.timer -= dt;
            for (within, profile) in row.slots.iter_mut().enumerate() {
                if profile.age >= 1.0 {
                    continue;
                }
                let before = TracerState {
                    profile_age: profile.age,
                    percentile: profile.percentile,
                    profile_seeds: profile.random,
                    ..*state
                };
                profile.age += dt / profile.lifespan;
                let acceleration = tracer.acceleration.vector(&before);
                let moving = profile.position.iter_mut().zip(&mut profile.velocity);
                for ((place, velocity), push) in moving.zip(acceleration) {
                    *place += *velocity * dt;
                    *velocity += push * dt;
                }
                tracer.apply_properties(&before, profile);
                let index = at * PROFILES_PER_ROW + within;
                profile.percentile = if index + 1 == count {
                    1.0
                } else {
                    index as f32 * step
                };
            }
        }
    }
}

/// What a slot keeps of its trailing tracers from frame to frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct H4TracerField {
    time: Option<f32>,
    running: Vec<RunningTracer>,
}

impl H4TracerField {
    /// Run the trailing tracers up to `time`, their locations where `bones`
    /// has them: `PulseAll`, then each one's update.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn advance(
        &mut self,
        tracers: &[H4TracerAt],
        values: impl Fn(u8) -> Option<FunctionValues>,
        onsets: &mut H3EffectOnsets,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        time: f32,
    ) {
        if self.running.len() != tracers.len() || self.time.is_none_or(|last| time < last) {
            self.running = (0..tracers.len())
                .map(|at| RunningTracer {
                    random: (at as u32).wrapping_mul(977) | 1,
                    ..Default::default()
                })
                .collect();
            self.time = Some(time);
        }
        let dt = time - self.time.unwrap_or(time);
        self.time = Some(time);
        if dt <= 0.0 {
            return;
        }
        // `PulseAll` sets the frame's allowance going once for every tracer.
        let mut spawned = 0;
        for (tracer, running) in tracers.iter().zip(&mut self.running) {
            if !tracer.trails() {
                continue;
            }
            let (at, _) = tracer.location(bones);
            let last = running.location.replace(at);
            if last.is_none() {
                running.last_placed = at;
            }
            running.speed = last.map_or(0.0, |last| {
                length(std::array::from_fn(|axis| last[axis] - at[axis])) / dt
            });
            let values = values(tracer.object);
            let onset = onsets.of(tracer.object, &tracer.scale_by, values.as_ref(), time);
            let TracerFrame {
                mut state, live, ..
            } = tracer.frame(values.as_ref(), at, eye, time, onset);
            state.speed = running.speed;
            state.length = running.length;
            if live {
                running.place_to(&tracer.definition, &state, at, &mut spawned);
            }
            running.update(&tracer.definition, &state, dt);
        }
    }

    /// The trailing tracer at `index`'s frame, or none while it has one
    /// profile or fewer (`TracerGpu::Render`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &self,
        index: usize,
        tracer: &H4TracerAt,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        time: f32,
        onset: f32,
    ) -> Option<H4TracerDraw> {
        let running = self.running.get(index)?;
        if running.count() <= 1 {
            return None;
        }
        let (origin, forward) = tracer.location(bones);
        let TracerFrame {
            mut state, since, ..
        } = tracer.frame(values, origin, eye, time, onset);
        state.speed = running.speed;
        state.length = running.length;
        let definition = &*tracer.definition;
        let step = percentile_step(running.length, definition.density.scalar(&state));
        let offset = definition.offset.scalar(&state);
        let states = running
            .rows
            .iter()
            .flat_map(|row| &row.slots)
            .map(pack_profile)
            .collect();
        let direction = {
            let length = length(forward).max(1e-6);
            forward.map(|axis| axis / length)
        };
        Some(tracer.draw_of(
            &state,
            since,
            [step, running.length, offset],
            origin,
            direction,
            states,
        ))
    }
}

attached!(H4TracerAt);

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};

    fn function(kind: FunctionType, low: f32, high: f32) -> TagFunction {
        let mut function = H2Function::new(kind);
        function.set_clamp_range(low, high).unwrap();
        TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).unwrap()
    }

    fn property(kind: FunctionType, input: u8, range: u8) -> TracerProperty {
        TracerProperty {
            input,
            range,
            adds: None,
            modifier: 0,
            function: function(kind, 0.0, 1.0),
            ends: [[0.0; 3]; 2],
        }
    }

    #[test]
    fn inputs_read_by_their_names() {
        assert_eq!(state_named("profile position"), 1);
        assert_eq!(state_named("tracer random"), TRACER_RANDOM);
        assert_eq!(state_named("effect b scale"), 17);
        assert_eq!(state_named("invalid state --- please set again"), INVALID);
    }

    /// `profile random` takes the correlation above the highest used, and
    /// `tracer random` the first free of two; a constant takes none.
    #[test]
    fn randoms_take_seeds_as_the_postprocess_hands_them_out() {
        let mut used = 0;
        let mut first = property(FunctionType::Identity, PROFILE_RANDOM, 0);
        first.allocate_seeds(&mut used);
        assert_eq!(first.input, 4);
        let mut second = property(FunctionType::Identity, PROFILE_RANDOM, 0);
        second.allocate_seeds(&mut used);
        assert_eq!(second.input, 5);
        let mut tracer = property(FunctionType::Identity, TRACER_RANDOM, 0);
        tracer.allocate_seeds(&mut used);
        assert_eq!(tracer.input, TRACER_SEED_1);
        let mut again = property(FunctionType::Identity, TRACER_RANDOM, 0);
        again.allocate_seeds(&mut used);
        assert_eq!(again.input, TRACER_SEED_2);
        let mut constant = property(FunctionType::Constant, PROFILE_RANDOM, 0);
        constant.allocate_seeds(&mut used);
        assert_eq!(constant.input, PROFILE_RANDOM, "a constant reads nothing");
        // A tag that names correlation 3 itself pushes the next past it.
        let mut named = 1 << 6;
        let mut after = property(FunctionType::Identity, PROFILE_RANDOM, 0);
        after.allocate_seeds(&mut named);
        assert_eq!(after.input, 7);
    }

    /// `numProfiles`: at least two, at most a strip's 128; the step between
    /// them the inverse of the product, held to 1.
    #[test]
    fn a_beam_has_two_profiles_at_least_and_a_strip_at_most() {
        assert_eq!(profile_count(0.1, 1.0), 2);
        assert_eq!(profile_count(2.5, 2.0), 5);
        assert_eq!(profile_count(1000.0, 1.0), MOST_PROFILES);
        assert_eq!(percentile_step(2.5, 2.0), 0.2);
        assert_eq!(percentile_step(0.1, 1.0), 1.0);
        assert_eq!(percentile_step(0.0, 1.0), 1.0, "an infinite step is 1");
        assert_eq!(percentile_step(-1.0, 1.0), 0.0);
    }

    /// The far LOD runs from its start value to its end value between its
    /// distances; with neither value set it is 1 near and 0 far.
    #[test]
    fn the_far_lod_ramps_between_its_distances() {
        let lod = TracerLod {
            start: 10.0,
            end: 20.0,
            start_value: 1.0,
            end_value: 0.0,
        };
        assert_eq!(lod.at(5.0), 1.0);
        assert_eq!(lod.at(15.0), 0.5);
        assert_eq!(lod.at(25.0), 0.0);
    }

    /// `RawTracerProfileState`: position and age as they are, velocity and
    /// initial alpha, size and intensity, offset, length and lifespan as
    /// halves, random, rotation, black point, palette and place as
    /// truncated ushorts, the colours as ARGB8.
    #[test]
    fn a_profile_packs_as_the_card_reads_it() {
        let profile = ProfileState {
            position: [1.0, 2.0, 3.0],
            age: 0.5,
            velocity: [0.0, 0.0, 2.0],
            initial_alpha: 1.0,
            random: [1.0, 0.0, 0.5, 0.0],
            size: 0.5,
            intensity: 2.0,
            rotation: 0.25,
            palette: 0.5,
            percentile: 1.0,
            length: 3.0,
            lifespan: 2.0,
            color: [1.0, 0.0, 0.0, 0.5],
            initial_color: [1.0, 1.0, 1.0, 0.0],
            ..Default::default()
        };
        let words = pack_profile(&profile);
        assert_eq!(words[..4], [1.0f32, 2.0, 3.0, 0.5].map(f32::to_bits));
        assert_eq!(
            words[5],
            0x4000 | (0x3C00 << 16),
            "velocity z 2, initial alpha 1"
        );
        assert_eq!(words[6], 65535);
        assert_eq!(words[7], 32767);
        assert_eq!(words[8], 0x3800 | (0x4000 << 16), "size 0.5, intensity 2");
        assert_eq!(words[10] & 0xFFFF, 16383, "a quarter turn");
        assert_eq!(words[11] >> 16, 65535, "the last profile's place");
        assert_eq!(words[12], 0x4200 | (0x4000 << 16), "length 3, lifespan 2");
        assert_eq!(words[13], (127 << 24) | (255 << 16));
        assert_eq!(words[14], 0x00FF_FFFF, "white at an exponent of 0");
        let other = pack_profile(&ProfileState {
            rotation: 0.5,
            ..profile
        });
        assert_ne!(other[10], words[10]);
    }

    fn fixed(value: f32) -> TracerProperty {
        TracerProperty {
            function: function(FunctionType::Constant, value, value),
            ..property(FunctionType::Constant, INVALID, INVALID)
        }
    }

    /// A ribbon two profiles a unit, each living `lifespan`.
    fn trail(lifespan: f32, origin_fade: Option<[f32; 2]>) -> TracerDefinition {
        TracerDefinition {
            shape: 0,
            ngon_sides: 0,
            appearance: 0,
            fade: [0.0; 4],
            origin_fade,
            lifespan: fixed(lifespan),
            acceleration: fixed(0.0),
            length: fixed(1.0),
            offset: fixed(0.0),
            density: fixed(2.0),
            size: fixed(1.0),
            profile_offset: fixed(0.0),
            rotation: fixed(0.0),
            color: fixed(1.0),
            alpha: fixed(1.0),
            alpha2: fixed(1.0),
            black_point: fixed(0.0),
            palette: fixed(0.0),
            intensity: fixed(1.0),
            uv_tiling: [1.0; 2],
            uv_scroll: [0.0; 2],
            sort_bias: 0.0,
            most_profiles: 2,
        }
    }

    /// Run `running` along `path`, a frame of `dt` at each point.
    fn run(tracer: &TracerDefinition, running: &mut RunningTracer, path: &[[f32; 3]], dt: f32) {
        let state = TracerState {
            effect_a: 1.0,
            ..Default::default()
        };
        for &at in path {
            if running.location.replace(at).is_none() {
                running.last_placed = at;
            }
            let mut spawned = 0;
            running.place_to(tracer, &state, at, &mut spawned);
            running.update(tracer, &state, dt);
        }
    }

    /// At rest only the temporary leading profile sits at the origin, so
    /// `Render` has one profile and draws nothing.
    #[test]
    fn a_trail_at_rest_keeps_one_profile() {
        let tracer = trail(1.0, None);
        let mut running = RunningTracer {
            random: 1,
            ..Default::default()
        };
        run(&tracer, &mut running, &[[0.0; 3]; 10], 0.1);
        assert_eq!(running.count(), 1);
        assert!(running.leading_temporary);
    }

    /// Moving lays profiles a density apart behind the location, oldest
    /// first along the trail, with the leading one written over each frame.
    #[test]
    fn a_moving_trail_lays_profiles_behind_it() {
        let tracer = trail(10.0, None);
        let mut running = RunningTracer {
            random: 1,
            ..Default::default()
        };
        let path: Vec<[f32; 3]> = (0..4).map(|step| [step as f32, 0.0, 0.0]).collect();
        run(&tracer, &mut running, &path, 0.1);
        // The tiny first stretch over the leading profile, two in the first
        // unit, then one (1.9996 a unit, floored) and two: the floor drops
        // what a frame's distance falls short of.
        assert_eq!(running.count(), 6);
        assert!((running.length - 3.0).abs() < 1e-3, "{}", running.length);
        let lengths: Vec<f32> = running
            .rows
            .iter()
            .flat_map(|row| &row.slots)
            .map(|profile| profile.length)
            .collect();
        assert!(
            lengths.windows(2).all(|pair| pair[0] <= pair[1]),
            "{lengths:?}"
        );
        let x = running.rows.last().unwrap().slots.last().unwrap().position[0];
        assert!((x - 3.0).abs() < 1e-3, "the newest is at the location: {x}");
    }

    /// An origin fade gets real profiles at both its ends, so the fade runs
    /// straight between them. Before the first real one a profile's length
    /// stays 0 (`InitializeProfile`), so the first lands at the cutoff with
    /// none.
    #[test]
    fn an_origin_fade_gets_profiles_at_its_ends() {
        let tracer = TracerDefinition {
            density: fixed(0.1),
            ..trail(10.0, Some([1.0, 0.5]))
        };
        let mut running = RunningTracer {
            random: 1,
            ..Default::default()
        };
        run(&tracer, &mut running, &[[0.0; 3], [3.0, 0.0, 0.0]], 0.1);
        let profiles: Vec<&ProfileState> = running.rows.iter().flat_map(|row| &row.slots).collect();
        let places: Vec<f32> = profiles.iter().map(|profile| profile.position[0]).collect();
        assert_eq!(places.len(), 3, "{places:?}");
        assert!(
            (places[0] - 1.0).abs() < 1e-4 && (places[1] - 1.5).abs() < 1e-4,
            "{places:?}"
        );
        let lengths: Vec<f32> = profiles.iter().map(|profile| profile.length).collect();
        assert_eq!(lengths, [0.0, 0.5, 2.0]);
        assert!(
            running.leading_temporary,
            "the last reaches the location for now"
        );
    }

    /// No more than ten profiles a frame, two once the frame has had 25.
    #[test]
    fn a_frame_lays_down_no_more_than_its_allowance() {
        let tracer = TracerDefinition {
            density: fixed(100.0),
            ..trail(10.0, None)
        };
        let state = TracerState {
            effect_a: 1.0,
            ..Default::default()
        };
        let mut running = RunningTracer {
            random: 1,
            placed_real: true,
            ..Default::default()
        };
        let mut spawned = 0;
        running.place_to(&tracer, &state, [5.0, 0.0, 0.0], &mut spawned);
        assert_eq!(spawned, 10);
        let mut late = RunningTracer {
            random: 1,
            placed_real: true,
            ..Default::default()
        };
        let mut spawned = 30;
        late.place_to(&tracer, &state, [5.0, 0.0, 0.0], &mut spawned);
        assert_eq!(spawned, 32);
    }

    /// A row goes as a whole once its longest lifespan has run; until then a
    /// dead profile stays, aged past 1, for the vertex shader to drop.
    #[test]
    fn profiles_die_a_row_at_a_time() {
        let tracer = trail(0.5, None);
        let mut running = RunningTracer {
            random: 1,
            ..Default::default()
        };
        run(&tracer, &mut running, &[[0.0; 3], [2.0, 0.0, 0.0]], 0.1);
        let laid = running.count();
        assert!(laid > 1);
        let state = TracerState::default();
        for _ in 0..5 {
            running.update(&tracer, &state, 0.1);
        }
        assert_eq!(running.count(), laid, "still within the row's time");
        assert!(running.rows[0].slots[0].age >= 1.0);
        for _ in 0..3 {
            running.update(&tracer, &state, 0.1);
        }
        assert_eq!(running.count(), 0);
    }

    #[test]
    fn two_tracers_get_different_seeds_inside_the_unit_range() {
        let (first, second) = (seeds_of(0), seeds_of(1));
        assert_ne!(first, second);
        assert!(
            first
                .iter()
                .chain(&second)
                .all(|seed| (0.0..1.0).contains(seed))
        );
    }

    /// With `BLAM_TEST_H4EK`: the beam hole's tracer reads, draws its
    /// profiles from one marker to the other, and the kit's point-to-point
    /// systems all read onto their materials.
    #[test]
    fn kit_halo_4_tracers_read_and_run_between_their_markers() {
        let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
            eprintln!("skipping: BLAM_TEST_H4EK is not set");
            return;
        };
        let source = super::super::super::particles::loose_kit(&root, crate::core::game::GameId::Halo4);
        let path = "environments\\solo\\m90_sacrifice\\fx\\beams\\parts\\beam_beamhole";
        let tag =
            load_referenced_tag_from_source(&source, path, "tracer_system", b"trac").expect(path);
        let element = elements(tag.root(), "tracers")[0];
        let tracer = read_tracer(&source, element).expect("its tracer");
        assert!(
            tracer
                .material
                .pass
                .vertex
                .contains("vs_tracer_profile_state_buffer")
        );
        assert!(
            tracer.appearance & ORIGIN_FADED != 0,
            "the beam hole fades from its origin"
        );
        let at = H4TracerAt {
            object: 0,
            start: (usize::MAX, [0.0; 3]),
            path: TracerPath::PointToPoint {
                end: (usize::MAX, [0.0, 0.0, 4.0]),
            },
            only_under: None,
            scale_by: String::new(),
            secondary_by: String::new(),
            seeds: seeds_of(0),
            times: Arc::new(H3EffectTimes::read_nothing()),
            event: 0,
            lod: TracerLod::read(tag.root()),
            definition: Arc::new(tracer),
        };
        let draw = at
            .draw(None, None, [10.0, 0.0, 0.0], 1.0, 0.0)
            .expect("it draws");
        assert!(draw.states.len() >= 2);
        assert_eq!(draw.direction, [0.0, 0.0, 1.0]);
        let z = |words: &[u32; 16]| f32::from_bits(words[2]);
        let last = draw.states.last().unwrap();
        assert!(
            z(last) > z(&draw.states[0]),
            "the profiles run toward the end"
        );
        assert!(
            draw.capped_length <= 4.0 + 1e-4,
            "held to the gap: {}",
            draw.capped_length
        );
        // Every point-to-point tracer in the kit reads onto its material.
        let (mut read, mut disabled, mut failures) = (0, 0, Vec::new());
        let mut stack = vec![std::path::PathBuf::from(&root)];
        while let Some(folder) = stack.pop() {
            for entry in std::fs::read_dir(&folder).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path
                    .extension()
                    .is_none_or(|extension| extension != "tracer_system")
                {
                    continue;
                }
                let relative = path.strip_prefix(&root).unwrap().with_extension("");
                let relative = relative.to_string_lossy().replace('/', "\\");
                let tag =
                    load_referenced_tag_from_source(&source, &relative, "tracer_system", b"trac")
                        .unwrap();
                if !has_flag(tag.root(), "tracer system flags", "point-to-point") {
                    continue;
                }
                for element in elements(tag.root(), "tracers") {
                    // The postprocess deletes these.
                    if has_flag(element, "flags", "disabled for debugging") {
                        disabled += 1;
                        continue;
                    }
                    match read_tracer(&source, element) {
                        Some(_) => read += 1,
                        None => failures.push(relative.clone()),
                    }
                }
            }
        }
        eprintln!(
            "NOTE KIT {read} point-to-point tracers read, {disabled} disabled, {} failed: {failures:?}",
            failures.len()
        );
        assert!(failures.is_empty());
        assert!(read > 100, "tracers read: {read}");
    }
}
