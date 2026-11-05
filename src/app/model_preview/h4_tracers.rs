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

use super::attachment::MarkerFrame;
use super::ce_attachments::elements;
use super::ce_lights::posed_on;
use super::h2_functions::H2FunctionValues;
use super::h2_passes::tag_function_at;
use super::h3_effects::{
    H3EffectTimes, adopt_event_function, for_each_attached_part, part_frame, read_part_placement,
};
use super::h3_particles::{read_ends, slerped};
use super::h4_particles::{pack_argb8, pack_half2, pack_ushort2n};
use super::materials::{H4EffectMaterial, resolve_h4_effect_material};
use super::*;
use crate::app::export::geometry::load_referenced_tag_from_source;
use crate::core::source::TagSource;

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
        let input = |field: &str| state_named(&property.read_enum_name(field).unwrap_or_default());
        Some(Self {
            input: input("Input Variable"),
            range: input("Range Variable"),
            adds: match property.read_enum_name("Output Modifier").as_deref() {
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

/// One of a `tracer_system`'s `tracers`, as `PostprocessBlockCallback`
/// (X 0x831FC270) leaves it.
#[derive(Debug)]
pub(crate) struct H4TracerDefinition {
    /// `profile shape`: 0 ribbon, 1 cross, 2 n-gon, 3 horizontal, 4 vertical.
    shape: u32,
    ngon_sides: u32,
    /// The tag's appearance flags and the runtime fade bits.
    appearance: u32,
    /// `1/origin range, origin cutoff, 1/edge range, edge cutoff`, the edge
    /// pair in radians.
    fade: [f32; 4],
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
    pub(crate) material: H4EffectMaterial,
    sort_layer: i32,
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
    let flags = element.read_flag_names("flags").unwrap_or_default();
    if flags
        .iter()
        .any(|(_, name)| name.starts_with("disabled for debugging"))
    {
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
        _lifespan,
        _acceleration,
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
    Some(H4TracerDefinition {
        shape: shape_named(&element.read_enum_name("profile shape").unwrap_or_default()),
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
        sort_layer: i32::from(material.sort_layer).clamp(1, 3),
        material: resolved,
    })
}

impl H4TracerDefinition {
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
    /// as 1 [open].
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

/// A point-to-point tracer between two markers of the object it is
/// attached to.
#[derive(Debug, Clone)]
pub(crate) struct H4TracerAt {
    pub(crate) object: u8,
    start: (usize, [f32; 3]),
    end: (usize, [f32; 3]),
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
    let step = |state: u32| state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    let first = step((index as u32).wrapping_mul(2_654_435_761) | 1);
    [first, step(first)].map(|state| (state >> 8) as f32 / (1u32 << 24) as f32)
}

/// The point-to-point tracers of an object's attached effects, one for each
/// of a part's location pairs. A part draws only when the system's
/// `point-to-point` agrees with its having a secondary location
/// (`UpdateLocations` X 0x8296EB50); trailing ones draw nothing at rest.
pub(super) fn read_h4_tracers(
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
        let Some(secondary) = part.secondary_marker.as_ref() else {
            return;
        };
        let Ok(tag) = load_referenced_tag_from_source(source, &part.path, "tracer_system", b"trac")
        else {
            return;
        };
        let root = tag.root();
        let system_flags = root
            .read_flag_names("tracer system flags")
            .unwrap_or_default();
        if !system_flags
            .iter()
            .any(|(_, name)| name.starts_with("point-to-point"))
        {
            return;
        }
        let lod = TracerLod::read(root);
        let definitions: Vec<Arc<H4TracerDefinition>> = elements(root, "tracers")
            .into_iter()
            .filter_map(|element| read_tracer(source, element))
            .map(Arc::new)
            .collect();
        let (offset, yaw, pitch) = read_part_placement(part.part);
        let ends = part.effect.locations(markers, secondary, &frame);
        for (start, end) in part
            .effect
            .locations(markers, &part.marker, &frame)
            .iter()
            .zip(&ends)
        {
            let (place, node) = frame(start);
            let (place, node) = part.placed(place, node);
            let place = part_frame(&place, offset, yaw, pitch);
            let (far, far_node) = frame(end);
            for definition in &definitions {
                tracers.push(H4TracerAt {
                    object: owner,
                    start: (node, place.position),
                    end: (far_node, far.position),
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

/// `RawTracerProfileState`'s sixteen words for a point-to-point profile
/// (`tracer_stateless_setup.fx`): no age, velocity or random, the initial
/// colour white at an exponent of 1 and the initial alpha 1.
#[derive(Debug, Clone, Copy, PartialEq)]
struct StatelessProfile {
    position: [f32; 3],
    size: f32,
    intensity: f32,
    offset: [f32; 2],
    rotation: f32,
    black_point: f32,
    palette: f32,
    percentile: f32,
    length: f32,
    color: [f32; 4],
}

fn pack_stateless(profile: &StatelessProfile) -> [u32; 16] {
    let [x, y, z] = profile.position.map(f32::to_bits);
    [
        x,
        y,
        z,
        0.0f32.to_bits(),
        pack_half2(0.0, 0.0),
        pack_half2(0.0, 1.0),
        0,
        0,
        pack_half2(profile.size, profile.intensity),
        pack_half2(profile.offset[0], profile.offset[1]),
        pack_ushort2n(profile.rotation, profile.black_point),
        pack_ushort2n(profile.palette, profile.percentile),
        pack_half2(profile.length, 0.0),
        pack_argb8(profile.color),
        pack_argb8([1.0; 4]),
        0,
    ]
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

fn length_of(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
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

impl H4TracerAt {
    pub(crate) fn only_under(&self) -> Option<&(String, String)> {
        self.only_under.as_ref()
    }

    pub(crate) fn scale_names(&self) -> [&str; 2] {
        [&self.scale_by, &self.secondary_by]
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    pub(crate) fn on_attached(mut self, name: &str) -> Self {
        adopt_event_function(name, &mut self.scale_by, &mut self.object);
        if let Some((region, _)) = &mut self.only_under {
            *region = format!("{name}: {region}");
        }
        self
    }

    pub(crate) fn sort_layer(&self) -> i32 {
        self.definition.sort_layer
    }

    /// World units nearer the camera it sorts.
    pub(crate) fn sort_bias(&self) -> f32 {
        self.definition.sort_bias
    }

    /// Its sort position: `Tracer::Submit` hands over its location's.
    pub(crate) fn origin(&self, bones: Option<&[[f32; 4]]>) -> [f32; 3] {
        let (node, position) = self.start;
        posed_on(node, position, [1.0, 0.0, 0.0], bones).0
    }

    /// The frame at `time`, or none while its event is not running, its
    /// effect is off or it can make no profiles.
    pub(crate) fn draw(
        &self,
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        time: f32,
        onset: f32,
    ) -> Option<H4TracerDraw> {
        let tracer = &*self.definition;
        if tracer.most_profiles <= 0 {
            return None;
        }
        let read = |name: &str| {
            if name.is_empty() {
                return 1.0;
            }
            values
                .and_then(|values| values.get(name))
                .copied()
                .unwrap_or(0.0)
        };
        let effect_a = read(&self.scale_by);
        if effect_a <= 0.0 {
            return None;
        }
        let since = time - onset;
        let system_age = self.times.age(self.event, since)?;
        let start = posed_on(self.start.0, self.start.1, [1.0, 0.0, 0.0], bones).0;
        let end = posed_on(self.end.0, self.end.1, [1.0, 0.0, 0.0], bones).0;
        // `SetStatelessPositionAndLength`: a beam of no length runs forward.
        let span: [f32; 3] = std::array::from_fn(|axis| end[axis] - start[axis]);
        let cap = length_of(span);
        let direction = if cap > 0.0 {
            span.map(|axis| axis / cap)
        } else {
            [1.0, 0.0, 0.0]
        };
        let from_eye: [f32; 3] = std::array::from_fn(|axis| start[axis] - eye[axis]);
        let mut state = TracerState {
            game_time: time,
            tracer_seeds: self.seeds,
            system_age,
            lod: self.lod.at(length_of(from_eye)),
            effect_a,
            effect_b: read(&self.secondary_by),
            ..Default::default()
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
        let states = (0..count)
            .map(|index| {
                let percentile = if index == count - 1 {
                    1.0
                } else {
                    index as f32 * step
                };
                state.percentile = percentile;
                let along = offset + capped * percentile;
                let [red, green, blue] = tracer.color.color(&state);
                let [x, y, _] = tracer.profile_offset.vector(&state);
                pack_stateless(&StatelessProfile {
                    position: std::array::from_fn(|axis| start[axis] + direction[axis] * along),
                    size: tracer.size.scalar(&state),
                    intensity: tracer.intensity.scalar(&state),
                    offset: [x, y],
                    rotation: tracer.rotation.scalar(&state).rem_euclid(1.0),
                    black_point: tracer.black_point.scalar(&state).rem_euclid(1.0),
                    palette: tracer.palette.scalar(&state).rem_euclid(1.0),
                    percentile,
                    length: percentile * capped,
                    color: [
                        red,
                        green,
                        blue,
                        tracer.alpha.scalar(&state) * tracer.alpha2.scalar(&state),
                    ],
                })
            })
            .collect();
        state.percentile = 0.0;
        let inputs = std::array::from_fn(|index| state.of(index as u8));
        let random = |flag: u32, seed: f32| {
            if tracer.appearance & flag != 0 {
                seed
            } else {
                0.0
            }
        };
        Some(H4TracerDraw {
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
            origin: start,
            direction,
            fade: tracer.fade,
            inputs,
            strips: tracer.strips(),
            states,
        })
    }
}

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

    /// A point-to-point profile has no age, velocity or random, an initial
    /// alpha of 1 and a white initial colour at an exponent of 1, which
    /// doubles its colour.
    #[test]
    fn a_stateless_profile_packs_as_the_setup_writes_it() {
        let profile = StatelessProfile {
            position: [1.0, 2.0, 3.0],
            size: 0.5,
            intensity: 2.0,
            offset: [0.0, 0.0],
            rotation: 0.25,
            black_point: 0.0,
            palette: 0.5,
            percentile: 1.0,
            length: 3.0,
            color: [1.0, 0.0, 0.0, 0.5],
        };
        let words = pack_stateless(&profile);
        assert_eq!(words[..4], [1.0f32, 2.0, 3.0, 0.0].map(f32::to_bits));
        assert_eq!(words[5] >> 16, 0x3C00, "initial alpha 1");
        assert_eq!(words[8], 0x3800 | (0x4000 << 16), "size 0.5, intensity 2");
        assert_eq!(words[10] & 0xFFFF, 16383, "a quarter turn");
        assert_eq!(words[11] >> 16, 65535, "the last profile's place");
        assert_eq!(words[12] & 0xFFFF, 0x4200, "length 3");
        assert_eq!(words[13], (127 << 24) | (255 << 16));
        assert_eq!(words[14], u32::MAX);
        let other = pack_stateless(&StatelessProfile {
            rotation: 0.5,
            ..profile
        });
        assert_ne!(other[10], words[10]);
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
        let source = super::super::h3_particles::loose_kit(&root, crate::core::game::GameId::Halo4);
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
            end: (usize::MAX, [0.0, 0.0, 4.0]),
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
                let flags = tag
                    .root()
                    .read_flag_names("tracer system flags")
                    .unwrap_or_default();
                if !flags
                    .iter()
                    .any(|(_, name)| name.starts_with("point-to-point"))
                {
                    continue;
                }
                for element in elements(tag.root(), "tracers") {
                    // The postprocess deletes these.
                    let flags = element.read_flag_names("flags").unwrap_or_default();
                    if flags
                        .iter()
                        .any(|(_, name)| name.starts_with("disabled for debugging"))
                    {
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
