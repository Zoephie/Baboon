//! Halo 3's beams and contrails: ribbons, crosses and tubes of profiles. A beam
//! is worked out whole each frame; a contrail leaves profiles behind as its
//! location moves, so one on a still object draws nothing, as in the game.

use std::collections::BTreeMap;
use std::sync::Arc;

use blam_tags::{TagFunction, TagStruct};

use crate::app::model_preview::common::marker::MarkerFrame;
use super::super::super::common::fields::{elements, has_flag};
use super::super::super::common::view::{ViewFrame, posed_on};
use super::super::super::functions::FunctionValues;
use super::super::super::common::named_function::tag_function_at;
use super::super::super::effects::{EffectOnsets, EffectTimes, for_each_attached_part, part_frame, read_part_placement};
use super::super::super::particles::{
    H3ParticleVertex, H3SpriteShader, lightmap_tint, particle_exposure, read_ends, read_sprite_shader, slerped,
};
use super::super::super::*;
use crate::app::model_preview::common::math::{clamped_unit, cross, dot, minus};
use crate::app::model_preview::common::random::{high24_over_2_24, lcg_step};
use crate::app::model_preview::common::attached::attached;

/// `beams.inl`'s limit, and what a contrail's rows hold.
const MOST_PROFILES: usize = 128;
const FEWEST_PROFILES: f32 = 2.0;
/// `k_real_epsilon`.
pub(in crate::app::model_preview) const EPSILON: f32 = 0.0001;
/// Reach's beam uv scroll clock, `g_render_time+0x10`: the frames' time,
/// wrapped at a minute (`c_beam_gpu::set_shader_state<render>`, the 360's
/// 0x826B2F98, MCC sub_14088B5B0).
const REACH_BEAM_SCROLL_PERIOD: f32 = 60.0;
/// Reach's contrail's, `get_game_time_seconds_repeat_arbitrary(0x830000)`:
/// a count of 1/65536 seconds wrapped at 0x830000, 131 seconds
/// (`c_contrail_gpu::set_shader_state<render>`, MCC sub_14085A3D0).
const REACH_CONTRAIL_SCROLL_PERIOD: f32 = 0x83_0000 as f32 / 65_536.0;
/// What Reach's `c_contrail::pulse` (the 360's 0x82ECB188) holds a
/// contrail's emission rate to before it accumulates.
const REACH_MOST_EMISSION_RATE: f32 = 100_000.0;
/// `get_sort_bias` (beam 0x82EA2548, contrail 0x82ECB7D8): world units a step
/// of `sort bias`.
const SORT_BIAS_STEP: f32 = 0.05;
const OWN_SEEDS: usize = 2;
const PROFILE_SEEDS: usize = 4;

/// The two systems' state inputs, by option name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StripInput {
    Position,
    UncappedPosition,
    ProfileAge,
    ProfileRandom,
    ProfileSeed(usize),
    GameTime,
    Age,
    Random,
    Seed(usize),
    Length,
    Speed,
    Lod,
    EffectA,
    EffectB,
    Invalid,
}

impl StripInput {
    fn named(name: &str) -> Self {
        match name {
            "profile position" => Self::Position,
            "profile uncapped position" => Self::UncappedPosition,
            "profile age" => Self::ProfileAge,
            "profile random" => Self::ProfileRandom,
            "profile correlation 1" => Self::ProfileSeed(0),
            "profile correlation 2" => Self::ProfileSeed(1),
            "profile correlation 3" => Self::ProfileSeed(2),
            "profile correlation 4" => Self::ProfileSeed(3),
            "game time" => Self::GameTime,
            "beam age" | "contrail age" => Self::Age,
            "beam random" | "contrail random" => Self::Random,
            "beam correlation 1" | "contrail correlation 1" => Self::Seed(0),
            "beam correlation 2" | "contrail correlation 2" => Self::Seed(1),
            "beam length" => Self::Length,
            "contrail speed" => Self::Speed,
            "beam lod" | "contrail lod" => Self::Lod,
            "effect a scale" => Self::EffectA,
            "effect b scale" => Self::EffectB,
            _ => Self::Invalid,
        }
    }

    fn of(self, state: &StripState) -> f32 {
        match self {
            Self::Position => state.position,
            Self::UncappedPosition => state.uncapped_position,
            Self::ProfileAge => state.profile_age,
            Self::ProfileSeed(at) => state.profile_seeds[at],
            Self::GameTime => state.game_time,
            Self::Age => state.age,
            Self::Seed(at) => state.seeds[at],
            Self::Length => state.length,
            Self::Speed => state.speed,
            Self::EffectA => state.effect_a,
            Self::EffectB => state.effect_b,
            Self::ProfileRandom | Self::Random | Self::Lod | Self::Invalid => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct StripState {
    position: f32,
    uncapped_position: f32,
    profile_age: f32,
    profile_seeds: [f32; PROFILE_SEEDS],
    game_time: f32,
    age: f32,
    seeds: [f32; OWN_SEEDS],
    length: f32,
    speed: f32,
    effect_a: f32,
    effect_b: f32,
}

/// `c_editable_property_base`.
#[derive(Debug)]
struct StripProperty {
    input: StripInput,
    range: StripInput,
    adds: Option<bool>,
    modifier: StripInput,
    function: TagFunction,
    ends: [[f32; 3]; 2],
}

impl StripProperty {
    fn read(holder: TagStruct<'_>, name: &str) -> Option<Self> {
        let property = holder.descend(name)?;
        let input = |field: &str| StripInput::named(&property.read_enum_option(field).unwrap_or_default());
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

    /// `allocate_random_seed`: a virtual random takes the next free correlation.
    fn allocate(&mut self, own: &mut usize, profile: &mut usize) {
        for input in [&mut self.input, &mut self.range, &mut self.modifier] {
            match *input {
                StripInput::Random => {
                    *input = StripInput::Seed(*own % OWN_SEEDS);
                    *own += 1;
                }
                StripInput::ProfileRandom => {
                    *input = StripInput::ProfileSeed(*profile % PROFILE_SEEDS);
                    *profile += 1;
                }
                _ => {}
            }
        }
    }

    /// `c_editable_property_base::set_to_default`, for what a tag older than
    /// the property has none of.
    fn fixed(value: f32) -> Option<Self> {
        use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};
        let mut function = H2Function::new(FunctionType::Identity);
        function.set_clamp_range(value, value).ok()?;
        Some(Self {
            input: StripInput::Invalid,
            range: StripInput::Invalid,
            adds: None,
            modifier: StripInput::Invalid,
            function: TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).ok()?,
            ends: [[0.0; 3]; 2],
        })
    }

    fn scalar(&self, state: &StripState) -> f32 {
        let value = self.function.evaluate(self.input.of(state), self.range.of(state));
        match self.adds {
            Some(true) => value + self.modifier.of(state),
            Some(false) => value * self.modifier.of(state),
            None => value,
        }
    }

    fn color(&self, state: &StripState) -> [f32; 3] {
        let color = self.function.evaluate_color(self.input.of(state), self.range.of(state));
        [color.red, color.green, color.blue]
    }

    /// `map_to_vector3d_range`: a direction goes around, not across. Opposite
    /// ends have no way around, and it divides by nothing as the shader does.
    fn vector(&self, state: &StripState) -> [f32; 3] {
        slerped(self.ends, self.scalar(state))
    }
}

/// `profile shape`, by option name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StripShape {
    Ribbon,
    Cross,
    Ngon(usize),
}

impl StripShape {
    fn strips(self) -> usize {
        match self {
            Self::Ribbon => 1,
            Self::Cross => 2,
            Self::Ngon(sides) => sides,
        }
    }

    /// `strip_and_offset_to_cross_sectional_offset`.
    fn across(self, strip: usize, offset: usize) -> [f32; 2] {
        match self {
            Self::Ribbon | Self::Cross => {
                let side = offset as f32 - 0.5;
                if strip == 0 { [side, 0.0] } else { [0.0, side] }
            }
            Self::Ngon(sides) => {
                let (sin, cos) = (std::f32::consts::TAU * (strip + offset) as f32 / sides as f32).sin_cos();
                [0.5 * cos, -0.5 * sin]
            }
        }
    }

    /// `profile_offset_to_texcoord`'s `v_shift`.
    fn v_shift(self, strip: usize, offset: usize) -> f32 {
        match self {
            Self::Ribbon | Self::Cross => offset as f32 - 0.5,
            Self::Ngon(sides) => (strip + offset) as f32 / sides as f32 - 0.5,
        }
    }
}

/// What a beam and a contrail share of their looks.
#[derive(Debug)]
struct StripLook {
    shape: StripShape,
    double_sided: bool,
    tiling: [f32; 2],
    scroll: [f32; 2],
    /// `1 / range` and the cutoff, world units.
    origin_fade: Option<[f32; 2]>,
    /// The same in radians.
    edge_fade: Option<[f32; 2]>,
    /// Reach's beams' `m_angle_range` and `m_angle_cutoff`: the range and the
    /// begin angle in radians, the range not inverted.
    angle_fade: Option<[f32; 2]>,
    /// `get_sort_bias`, in world units nearer the camera; Reach's.
    sort_bias: f32,
    /// Reach's: its own exposure, scroll clocks, emission limit and fades.
    reach: bool,
    shader: Arc<H3SpriteShader>,
}

/// HLSL's `saturate`, which takes a NaN to 0 where Rust's `clamp` passes it
/// through [inferred: the DirectX 10+ rule].
fn saturate(value: f32) -> f32 {
    if value.is_nan() { 0.0 } else { value.clamp(0.0, 1.0) }
}

impl StripLook {
    /// A fade's `saturate`. Reach's step fades (a range of 0) meet a NaN at
    /// the cutoff, which comes out 0.
    fn fade(&self, value: f32) -> f32 {
        if self.reach { saturate(value) } else { value.clamp(0.0, 1.0) }
    }

    /// The uv scroll's clock: Reach's wraps at `period`, Halo 3's runs on.
    fn scroll_time(&self, period: f32, time: f32) -> f32 {
        if self.reach { time.rem_euclid(period) } else { time }
    }
}

/// An origin or edge fade's `1 / range` and cutoff, `scale` to the shader's
/// units, if its postprocess turns it on: with a cutoff, or with a range too
/// where `by_range` (Reach's beams, tool sub_140DD25D0, the 360's
/// 0x833B3460). Reach's contrails' (tool 0x140DCFCE0) go by the cutoff, as
/// Halo 3's.
fn strip_fade(range: f32, cutoff: f32, scale: f32, by_range: bool) -> Option<[f32; 2]> {
    (cutoff != 0.0 || (by_range && range != 0.0)).then(|| [(range * scale).recip(), cutoff * scale])
}

/// `beam` is a `beam_system`'s, else a `contrail_system`'s.
fn read_strip_look(reader: &TagReader, element: TagStruct<'_>, beam: bool) -> Option<StripLook> {
    let reach = reader.source.game() == Some(GameId::HaloReach);
    let mut shader = read_sprite_shader(reader, element.descend("actual shader?")?)?;
    if reach && beam {
        // `effects__beam.hlsl`'s `sample_diffuse` (l.373–458): the strip's one
        // texcoord, a `particle_alpha` of 1, and no palette shift.
        shader.palette_shift = None;
        shader.plasma_second_sprite = false;
        shader.unit_particle_alpha = true;
    }
    let shape = match element.read_enum_option("profile shape").as_deref() {
        Some("cross") => StripShape::Cross,
        Some("n-gon") => {
            let sides = element.read_int_any("number of n-gon sides").unwrap_or(0).max(0) as usize;
            StripShape::Ngon(sides.max(3))
        }
        _ => StripShape::Ribbon,
    };
    let tiling = element.read_vec2("uv tiling");
    let scroll = element.read_vec2("uv scrolling");
    let real = |name: &str| element.read_real(name).unwrap_or(0.0);
    let by_range = reach && beam;
    let fade = |range: f32, cutoff: f32, scale: f32| strip_fade(range, cutoff, scale, by_range);
    let degrees = std::f32::consts::PI / 180.0;
    let [angle_range, angle_begin] = [real("angle fade range"), real("angle fade begin")];
    Some(StripLook {
        shape,
        double_sided: has_flag(element, "appearance flags", "double-sided"),
        // The postprocess's: no tiling is one tile.
        tiling: [tiling.i, tiling.j].map(|rate| if rate == 0.0 { 1.0 } else { rate }),
        scroll: [scroll.i, scroll.j],
        origin_fade: fade(real("origin fade range"), real("origin fade cutoff"), 1.0),
        edge_fade: fade(real("edge fade range"), real("edge fade cutoff"), degrees),
        angle_fade: (by_range && (angle_range != 0.0 || angle_begin != 0.0))
            .then_some([angle_range * degrees, angle_begin * degrees]),
        sort_bias: if reach { element.read_int_any("sort bias").unwrap_or(0) as f32 * SORT_BIAS_STEP } else { 0.0 },
        reach,
        shader: Arc::new(shader),
    })
}

#[derive(Debug)]
struct BeamDefinition {
    length: StripProperty,
    offset: StripProperty,
    density: StripProperty,
    rotation: StripProperty,
    thickness: StripProperty,
    color: StripProperty,
    alpha: StripProperty,
    black_point: StripProperty,
    palette: StripProperty,
    intensity: StripProperty,
}

fn read_beam(element: TagStruct<'_>) -> Option<BeamDefinition> {
    let read = |name: &str| StripProperty::read(element, name);
    let mut beam = BeamDefinition {
        length: read("length")?,
        offset: read("offset")?,
        density: read("profile_density")?,
        rotation: read("profile_rotation")?,
        thickness: read("profile_thickness")?,
        color: read("profile_color")?,
        alpha: read("profile_alpha")?,
        black_point: read("profile_black_point")?,
        palette: read("profile_palette")?,
        intensity: read("profile_intensity")?,
    };
    let (mut own, mut profile) = (0, 0);
    let properties = [
        &mut beam.length, &mut beam.offset, &mut beam.density, &mut beam.rotation, &mut beam.thickness,
        &mut beam.color, &mut beam.alpha, &mut beam.black_point, &mut beam.palette, &mut beam.intensity,
    ];
    properties.into_iter().for_each(|property| property.allocate(&mut own, &mut profile));
    Some(beam)
}

#[derive(Debug)]
struct ContrailDefinition {
    emission_rate: StripProperty,
    lifespan: StripProperty,
    acceleration: StripProperty,
    size: StripProperty,
    offset: StripProperty,
    rotation: StripProperty,
    color: StripProperty,
    alpha: StripProperty,
    secondary_alpha: StripProperty,
    black_point: StripProperty,
    palette: StripProperty,
    intensity: StripProperty,
    tint_from_lightmap: bool,
    opacity_from_a: bool,
    random_u: bool,
    random_v: bool,
}

fn read_contrail(element: TagStruct<'_>) -> Option<ContrailDefinition> {
    let read = |name: &str| StripProperty::read(element, name);
    let flagged = |wanted: &str| has_flag(element, "appearance flags", wanted);
    let mut contrail = ContrailDefinition {
        emission_rate: read("emission rate")?,
        lifespan: read("profile lifespan")?,
        acceleration: read("profile self acceleration")?,
        size: read("profile size")?,
        offset: read("profile offset")?,
        rotation: read("profile rotation")?,
        color: read("profile color")?,
        alpha: read("profile alpha")?,
        secondary_alpha: read("profile secondary alpha").or_else(|| StripProperty::fixed(1.0))?,
        black_point: read("profile black point")?,
        palette: read("profile palette")?,
        intensity: read("profile intensity")?,
        tint_from_lightmap: flagged("tint from lightmap"),
        opacity_from_a: flagged("profile opacity from scale a"),
        random_u: flagged("random u offset"),
        random_v: flagged("random v offset"),
    };
    let (mut own, mut profile) = (0, 0);
    let properties = [
        &mut contrail.emission_rate, &mut contrail.lifespan, &mut contrail.acceleration, &mut contrail.size,
        &mut contrail.offset, &mut contrail.rotation, &mut contrail.color, &mut contrail.alpha,
        &mut contrail.secondary_alpha, &mut contrail.black_point, &mut contrail.palette, &mut contrail.intensity,
    ];
    properties.into_iter().for_each(|property| property.allocate(&mut own, &mut profile));
    Some(contrail)
}

#[derive(Debug)]
enum StripKind {
    Beam(BeamDefinition),
    Contrail(ContrailDefinition),
}

/// A beam or a contrail at a marker of the object it is attached to.
#[derive(Debug)]
pub(crate) struct H3StripAt {
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    /// `use parent position but world orientation`: its forward does not turn.
    world_oriented: bool,
    /// Started by a particle: it follows them, not a marker.
    carried: bool,
    /// `secondary location (beams)`: where a beam runs to, instead of its length.
    end: Option<(usize, [f32; 3])>,
    only_under: Option<(String, String)>,
    scale_by: String,
    secondary_by: String,
    seeds: [f32; OWN_SEEDS],
    times: Arc<EffectTimes>,
    event: usize,
    look: Arc<StripLook>,
    kind: Arc<StripKind>,
}

fn seeds_of(index: usize) -> [f32; OWN_SEEDS] {
    let mut random = (index as u32).wrapping_mul(2654435761) | 1;
    std::array::from_fn(|_| {
        random = lcg_step(random);
        high24_over_2_24(random)
    })
}

/// The beams and contrails of an object's attached effects, one at each marker
/// of a location.
pub(in crate::app::model_preview) fn read_h3_strips(
    reader: &TagReader,
    object: &blam_tags::TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H3StripAt> {
    let mut strips = Vec::new();
    let beam_group = u32::from_be_bytes(*b"beam");
    let contrail_group = u32::from_be_bytes(*b"cntl");
    for_each_attached_part(reader, object, |part| {
        type Read = fn(TagStruct<'_>) -> Option<StripKind>;
        let beam = part.group == beam_group;
        let (blocks, group_name, code, read): (&str, &str, &[u8; 4], Read) = if beam {
                ("beams", "beam_system", b"beam", |element| read_beam(element).map(StripKind::Beam))
            } else if part.group == contrail_group {
                ("contrails", "contrail_system", b"cntl", |element| read_contrail(element).map(StripKind::Contrail))
            } else {
                return;
            };
        let Ok(tag) = reader.load(&part.path, group_name, code) else {
            return;
        };
        let (offset, yaw, pitch) = read_part_placement(part.part);
        for element in elements(tag.root(), blocks) {
            let (Some(look), Some(kind)) = (read_strip_look(reader, element, beam), read(element)) else {
                continue;
            };
            let (look, kind) = (Arc::new(look), Arc::new(kind));
            for marker in &part.effect.locations(markers, &part.marker, &frame) {
                let (place, node) = frame(marker);
                let (place, node) = part.placed(place, node);
                let place = part_frame(&place, offset, yaw, pitch);
                let end = part.secondary_marker.as_ref().and_then(|name| {
                    let far = markers.iter().find(|marker| marker.name.eq_ignore_ascii_case(name))?;
                    let (far, node) = frame(far);
                    Some((node, far.position))
                });
                strips.push(H3StripAt {
                    object: owner,
                    node,
                    position: place.position,
                    forward: place.axes[0],
                    world_oriented: part.world_oriented,
                    carried: false,
                    end,
                    only_under: marker.only_under.clone(),
                    scale_by: part.effect.scale_by.clone(),
                    secondary_by: part.effect.secondary_by.clone(),
                    seeds: seeds_of(strips.len()),
                    times: Arc::clone(&part.times),
                    event: part.event,
                    look: Arc::clone(&look),
                    kind: Arc::clone(&kind),
                });
            }
        }
    });
    strips
}

/// The contrails an effect holds, as a particle starts them.
pub(in crate::app::model_preview) fn read_carried_strips(reader: &TagReader, path: &str) -> Vec<H3StripAt> {
    let mut strips = Vec::new();
    let Ok(effect) = reader.load(path, "effect", b"effe") else {
        return strips;
    };
    let times = Arc::new(EffectTimes::read(effect.root()));
    let contrail_group = u32::from_be_bytes(*b"cntl");
    for (at, event) in elements(effect.root(), "events").into_iter().enumerate() {
        for part in elements(event, "parts") {
            let Some((group, path)) = part.read_tag_ref_with_group("type") else {
                continue;
            };
            if group != contrail_group || path.is_empty() {
                continue;
            }
            let Ok(tag) = reader.load(&path, "contrail_system", b"cntl") else {
                continue;
            };
            for element in elements(tag.root(), "contrails") {
                let (Some(look), Some(contrail)) = (read_strip_look(reader, element, false), read_contrail(element)) else {
                    continue;
                };
                strips.push(H3StripAt {
                    object: 0,
                    node: 0,
                    position: [0.0; 3],
                    forward: [1.0, 0.0, 0.0],
                    world_oriented: false,
                    carried: true,
                    end: None,
                    only_under: None,
                    scale_by: String::new(),
                    secondary_by: String::new(),
                    seeds: seeds_of(strips.len()),
                    times: Arc::clone(&times),
                    event: at,
                    look: Arc::new(look),
                    kind: Arc::new(StripKind::Contrail(contrail)),
                });
            }
        }
    }
    strips
}





/// `cross_section_world_basis`. Reach's has no guard for straight up, where
/// `normalize(0)` takes every corner to NaN and the strip does not draw; the
/// preview keeps Halo 3's basis for it.
fn world_basis(direction: [f32; 3]) -> [[f32; 3]; 2] {
    let product = cross(direction, [0.0, 0.0, 1.0]);
    if product == [0.0; 3] {
        return [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
    }
    let first = clamped_unit(product);
    [first, clamped_unit(cross(first, direction))]
}

/// One profile, as both systems' vertex shaders read it.
#[derive(Debug, Clone, Copy, Default)]
struct Profile {
    place: [f32; 3],
    direction: [f32; 3],
    offset: [f32; 2],
    size: f32,
    rotation: f32,
    /// Lengthwise in world units, for the texture.
    length: f32,
    color: [f32; 4],
    palette: f32,
    black_point: f32,
}

/// `default_vs`, for a run of profiles.
fn strip_vertices(
    look: &StripLook,
    profiles: &[Profile],
    view: &ViewFrame,
    time: f32,
    uv_offset: [f32; 2],
    origin_fade: impl Fn(&Profile, [f32; 3]) -> f32,
    edge_view: impl Fn([f32; 3]) -> [f32; 3],
) -> Vec<H3ParticleVertex> {
    let shape = look.shape;
    let vertex = |profile: &Profile, strip: usize, offset: usize| {
        let world = world_basis(profile.direction);
        let basis = if shape == StripShape::Ribbon {
            let first = clamped_unit(cross(minus(profile.place, view.eye), profile.direction));
            [first, clamped_unit(cross(first, profile.direction))]
        } else {
            world
        };
        let (sin, cos) = (std::f32::consts::TAU * profile.rotation).sin_cos();
        let basis: [[f32; 3]; 2] = [
            std::array::from_fn(|axis| cos * basis[0][axis] + sin * basis[1][axis]),
            std::array::from_fn(|axis| -sin * basis[0][axis] + cos * basis[1][axis]),
        ];
        let [x, y] = shape.across(strip, offset).map(|side| side * profile.size);
        let place: [f32; 3] = std::array::from_fn(|axis| {
            profile.place[axis]
                + basis[0][axis] * x
                + basis[1][axis] * y
                + world[0][axis] * profile.offset[0]
                + world[1][axis] * profile.offset[1]
        });
        let shifted = [profile.length, shape.v_shift(strip, offset)];
        let uv: [f32; 2] = std::array::from_fn(|axis| {
            shifted[axis] * look.tiling[axis]
                + [0.0, 0.5][axis]
                + time * look.scroll[axis]
                + uv_offset[axis]
        });
        let mut color = profile.color;
        color[3] *= origin_fade(profile, place);
        if let Some([range, cutoff]) = look.edge_fade {
            let normal = clamped_unit(cross(basis[0], basis[1]));
            let angle = dot(edge_view(place), normal).abs().min(1.0).acos();
            color[3] *= look.fade(range * (angle - cutoff));
        }
        if let Some([range, cutoff]) = look.angle_fade {
            color[3] *= angle_fade(place, profile, view.eye, range, cutoff);
        }
        H3ParticleVertex {
            place,
            sprite: uv,
            next_sprite: uv,
            billboard: uv,
            color,
            extra: [0.0, profile.palette, profile.black_point],
            lean: [[0.0; 3]; 2],
            normal: [0.0, 0.0, 1.0],
        }
    };
    let mut vertices = Vec::with_capacity(profiles.len() * shape.strips() * 6);
    for strip in 0..shape.strips() {
        for pair in profiles.windows(2) {
            // The n-gon's sides wind so the inward ones cull.
            if let (StripShape::Ngon(_), false) = (shape, look.double_sided) {
                let middle: [f32; 3] = {
                    let side = vertex(&pair[0], strip, 0).place;
                    let other = vertex(&pair[0], strip, 1).place;
                    std::array::from_fn(|axis| (side[axis] + other[axis]) * 0.5)
                };
                if dot(minus(middle, pair[0].place), minus(view.eye, middle)) < 0.0 {
                    continue;
                }
            }
            let quad = [
                vertex(&pair[0], strip, 0),
                vertex(&pair[0], strip, 1),
                vertex(&pair[1], strip, 1),
                vertex(&pair[1], strip, 0),
            ];
            vertices.extend([quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
        }
    }
    vertices
}

/// Reach's beam angle fade (`effects__beam.hlsl` l.296–305), at a corner
/// `place` of `profile`: by how far from face on the corner's side is seen,
/// across the beam's direction. Angle 0 faces the camera.
fn angle_fade(place: [f32; 3], profile: &Profile, eye: [f32; 3], range: f32, cutoff: f32) -> f32 {
    let profile_normal = clamped_unit(minus(place, profile.place));
    let camera_to_vertex = clamped_unit(minus(place, eye));
    let profile_ortho = clamped_unit(cross(camera_to_vertex, profile.direction));
    let profile_camera_ortho = clamped_unit(cross(profile_ortho, profile.direction));
    let angle = dot(profile_camera_ortho, profile_normal).abs().min(1.0).acos();
    // A range of 0 divides by nothing, as the game does: a step at the begin
    // angle, and a NaN on it.
    saturate(1.0 - (angle - cutoff) / range)
}

/// A profile's colour as `default_vs` exposes it, by `particle_exposure`'s
/// rule, at the area screen effects' `stops`.
fn exposed(look: &StripLook, color: [f32; 3], alpha: f32, intensity: f32, stops: f32) -> [f32; 4] {
    let light = particle_exposure(look.shader.blend, true, look.reach, stops);
    let [red, green, blue] = color.map(|channel| channel.clamp(0.0, 1.0) * intensity * light);
    [red, green, blue, alpha.clamp(0.0, 1.0) * intensity]
}

/// `pulse`'s split of a beam: how far its `count` profiles reach.
fn beam_profiles(capped: f32, density: f32) -> usize {
    (capped * density).ceil().clamp(FEWEST_PROFILES, MOST_PROFILES as f32) as usize
}

/// `c_contrail_profile`, and what its update keeps.
#[derive(Debug, Clone, Copy)]
struct ContrailProfile {
    place: [f32; 3],
    velocity: [f32; 3],
    age: f32,
    lifespan: f32,
    length: f32,
    seeds: [f32; PROFILE_SEEDS],
    alpha: f32,
    temporary: bool,
}

/// `c_contrail`.
#[derive(Debug, Clone, Default)]
struct RunningContrail {
    /// Oldest first.
    profiles: Vec<ContrailProfile>,
    accumulator: f32,
    previous: [f32; 3],
    length: f32,
    initialized: bool,
    location: Option<[f32; 3]>,
    speed: f32,
    random: u32,
}

impl RunningContrail {
    fn next_seed(&mut self) -> f32 {
        self.random = lcg_step(self.random);
        high24_over_2_24(self.random)
    }

    /// `create_profile`.
    fn create(
        &mut self,
        contrail: &ContrailDefinition,
        state: &StripState,
        temporary: bool,
        direction: [f32; 3],
        distance: f32,
    ) {
        if self.profiles.last().is_some_and(|profile| profile.temporary) {
            self.profiles.pop();
        }
        if self.profiles.len() >= MOST_PROFILES {
            return;
        }
        let seeds: [f32; PROFILE_SEEDS] = std::array::from_fn(|_| self.next_seed());
        let place = std::array::from_fn(|axis| self.previous[axis] + direction[axis] * distance);
        let length = if self.initialized { distance + self.length } else { 0.0 };
        let lifespan = contrail.lifespan.scalar(&StripState { profile_seeds: seeds, ..*state });
        self.profiles.push(ContrailProfile {
            place,
            velocity: [0.0; 3],
            age: 0.0,
            lifespan,
            length,
            seeds,
            alpha: if contrail.opacity_from_a { state.effect_a } else { 1.0 },
            temporary,
        });
        if !temporary {
            self.initialized = true;
            self.previous = place;
            self.length = length;
        }
    }

    /// `extend`: origin fades need profiles at their ends.
    fn extend(
        &mut self,
        contrail: &ContrailDefinition,
        look: &StripLook,
        state: &StripState,
        temporary: bool,
        direction: [f32; 3],
        by: f32,
    ) {
        let mut current = self.length;
        let last = self.length + by;
        if let (true, Some([inverse, cutoff])) = (self.initialized, look.origin_fade) {
            for boundary in [cutoff, cutoff + inverse.recip()] {
                if boundary > current + EPSILON && boundary < last - EPSILON {
                    self.create(contrail, state, false, direction, boundary - current);
                    current = boundary;
                }
            }
        }
        self.create(contrail, state, temporary, direction, last - current);
    }

    /// `c_contrail::pulse`.
    fn pulse(&mut self, contrail: &ContrailDefinition, look: &StripLook, state: &StripState, at: [f32; 3], dt: f32) {
        let moved = minus(at, self.previous);
        let total = dot(moved, moved).sqrt();
        // `normalize3d`: what is too short to have a direction has no length.
        let (total, direction) =
            if total < EPSILON { (0.0, [0.0; 3]) } else { (total, moved.map(|axis| axis / total)) };
        // `event_initialize_contrail_system` (Halo 3 X360 0x829F9BA0, Reach's
        // by name) pulses a new system once with no time and its flag set,
        // which `c_contrail::pulse` turns into a permanent profile where it
        // starts; `pulse_all` then pulses with the flag clear.
        if !self.initialized {
            self.extend(contrail, look, state, false, direction, total);
            return;
        }
        if total <= EPSILON {
            return;
        }
        let mut remaining = total;
        let rate = contrail.emission_rate.scalar(state);
        let rate = if look.reach { rate.min(REACH_MOST_EMISSION_RATE) } else { rate };
        self.accumulator += rate * dt;
        if self.accumulator >= 1.0 {
            let each = total / self.accumulator;
            while self.accumulator >= 1.0 {
                self.extend(contrail, look, state, false, direction, each);
                remaining -= each;
                self.accumulator -= 1.0;
            }
        }
        if remaining > EPSILON {
            self.extend(contrail, look, state, true, direction, remaining);
        }
    }

    /// `contrail_main`.
    fn update(&mut self, contrail: &ContrailDefinition, state: &StripState, dt: f32) {
        self.profiles.retain_mut(|profile| {
            let state = StripState { profile_age: profile.age, profile_seeds: profile.seeds, ..*state };
            profile.age += dt / profile.lifespan.max(1e-4);
            if profile.age >= 1.0 {
                return false;
            }
            let acceleration = contrail.acceleration.vector(&state);
            for axis in 0..3 {
                profile.place[axis] += profile.velocity[axis] * dt;
                profile.velocity[axis] += acceleration[axis] * dt;
            }
            true
        });
    }
}

/// What a slot keeps of its contrails from frame to frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct H3StripField {
    time: Option<f32>,
    running: Vec<RunningContrail>,
    /// Per strip, one for each particle it follows, with when that began.
    /// Ordered by the particle's id, so they draw in the same order every run.
    carried: Vec<BTreeMap<u64, (f32, RunningContrail)>>,
}

impl H3StripAt {
    pub(crate) fn scale_names(&self) -> [&str; 2] {
        [&self.scale_by, &self.secondary_by]
    }


    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    pub(crate) fn shader(&self) -> &H3SpriteShader {
        &self.look.shader
    }

    /// How much nearer than its location it sorts: `add_element` takes
    /// `get_sort_bias` off its view depth.
    pub(crate) fn sort_bias(&self) -> f32 {
        self.look.sort_bias
    }

    /// Its sort position, as `submit` hands it over.
    pub(crate) fn origin(&self, bones: Option<&[[f32; 4]]>) -> [f32; 3] {
        posed_on(self.node, self.position, self.forward, bones).0
    }

    /// The states this frame, or none while its event is not running.
    fn state(&self, values: Option<&FunctionValues>, time: f32, onset: f32) -> Option<StripState> {
        let read = |name: &str, none: f32| {
            if name.is_empty() {
                return none;
            }
            values.and_then(|values| values.get(name)).copied().unwrap_or(0.0)
        };
        let effect_a = read(&self.scale_by, 1.0);
        if effect_a <= 0.0 {
            return None;
        }
        Some(StripState {
            game_time: time.rem_euclid(1.0),
            age: self.times.age(self.event, time - onset)?,
            seeds: self.seeds,
            effect_a,
            effect_b: read(&self.secondary_by, 1.0),
            ..Default::default()
        })
    }

    fn beam_vertices(
        &self,
        beam: &BeamDefinition,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
        stops: f32,
    ) -> Vec<H3ParticleVertex> {
        let Some(mut state) = self.state(values, time, onset) else {
            return Vec::new();
        };
        let (origin, turned) = posed_on(self.node, self.position, self.forward, bones);
        let forward = if self.world_oriented { self.forward } else { turned };
        let uncapped = beam.length.scalar(&state);
        let reach = self.end.map(|(node, end)| minus(posed_on(node, end, [1.0, 0.0, 0.0], bones).0, origin));
        let (direction, cap) = match reach {
            Some(reach) => (clamped_unit(reach), dot(reach, reach).sqrt()),
            None => (clamped_unit(forward), 0.0),
        };
        let capped = if cap != 0.0 && cap <= uncapped { cap } else { uncapped };
        state.length = capped;
        let density = beam.density.scalar(&state);
        let count = beam_profiles(capped, density);
        let step = (density * capped).max(1e-6).recip();
        let cap_percentage = if uncapped != 0.0 { capped / uncapped } else { 1.0 };
        let offset = beam.offset.scalar(&state);
        let look = &*self.look;
        let profiles: Vec<Profile> = (0..count)
            .map(|index| {
                let percentile = if index == count - 1 { 1.0 } else { index as f32 * step };
                state.position = percentile;
                state.uncapped_position = percentile * cap_percentage;
                let along = offset + capped * percentile;
                let alpha = beam.alpha.scalar(&state);
                let intensity = beam.intensity.scalar(&state);
                Profile {
                    place: std::array::from_fn(|axis| origin[axis] + direction[axis] * along),
                    direction,
                    size: beam.thickness.scalar(&state),
                    rotation: beam.rotation.scalar(&state),
                    length: (percentile * capped).abs(),
                    color: exposed(look, beam.color.color(&state), alpha, intensity, stops),
                    palette: beam.palette.scalar(&state).clamp(0.0, 0.9999),
                    black_point: beam.black_point.scalar(&state).clamp(0.0, 0.9999),
                    ..Default::default()
                }
            })
            .collect();
        let fade = look.origin_fade;
        let scroll_time = look.scroll_time(REACH_BEAM_SCROLL_PERIOD, time);
        strip_vertices(
            look,
            &profiles,
            view,
            scroll_time,
            [0.0; 2],
            |_, place| {
                fade.map_or(1.0, |[range, cutoff]| {
                    let from = minus(place, origin);
                    look.fade(range * (dot(from, from).sqrt() - cutoff))
                })
            },
            |place| clamped_unit(minus(place, view.eye)),
        )
    }

    fn contrail_vertices(
        &self,
        contrail: &ContrailDefinition,
        running: &RunningContrail,
        values: Option<&FunctionValues>,
        view: &ViewFrame,
        lighting: &[[f32; 4]; 10],
        time: f32,
        onset: f32,
        stops: f32,
    ) -> Vec<H3ParticleVertex> {
        if running.profiles.len() < 2 {
            return Vec::new();
        }
        let base = StripState {
            speed: running.speed,
            ..self.state(values, time, onset).unwrap_or(StripState { game_time: time.rem_euclid(1.0), age: 1.0, seeds: self.seeds, ..Default::default() })
        };
        let look = &*self.look;
        let initial = if contrail.tint_from_lightmap { lightmap_tint(lighting) } else { [1.0; 3] };
        let live = &running.profiles;
        let profiles: Vec<Profile> = live
            .iter()
            .enumerate()
            .map(|(index, profile)| {
                let state = StripState { profile_age: profile.age, profile_seeds: profile.seeds, ..base };
                let direction = match live.get(index + 1) {
                    Some(next) => minus(next.place, profile.place),
                    None => minus(profile.place, live[index - 1].place),
                };
                let alpha = contrail.alpha.scalar(&state).clamp(0.0, 1.0)
                    * contrail.secondary_alpha.scalar(&state).clamp(0.0, 1.0)
                    * profile.alpha;
                let [x, y, _] = contrail.offset.vector(&state);
                let intensity = contrail.intensity.scalar(&state);
                let mut color = exposed(look, contrail.color.color(&state), alpha, intensity, stops);
                (0..3).for_each(|channel| color[channel] *= initial[channel]);
                Profile {
                    place: profile.place,
                    direction,
                    offset: [x, y],
                    size: contrail.size.scalar(&state),
                    rotation: contrail.rotation.scalar(&state).rem_euclid(1.0),
                    length: profile.length,
                    color,
                    palette: contrail.palette.scalar(&state).rem_euclid(1.0),
                    black_point: contrail.black_point.scalar(&state).rem_euclid(1.0),
                }
            })
            .collect();
        let fade = look.origin_fade;
        let uv_offset = [
            if contrail.random_u { self.seeds[0] } else { 0.0 },
            if contrail.random_v { self.seeds[1] } else { 0.0 },
        ];
        let scroll_time = look.scroll_time(REACH_CONTRAIL_SCROLL_PERIOD, time);
        strip_vertices(
            look,
            &profiles,
            view,
            scroll_time,
            uv_offset,
            |profile, _| fade.map_or(1.0, |[range, cutoff]| look.fade(range * (profile.length - cutoff))),
            |_| view.forward,
        )
    }
}

impl H3StripField {
    /// Run contrails up to `time`, with their locations where `bones` has them.
    pub(crate) fn advance(
        &mut self,
        strips: &[Arc<H3StripAt>],
        values: impl Fn(u8) -> Option<FunctionValues>,
        onsets: &mut EffectOnsets,
        tracks: impl Fn(&Arc<H3StripAt>) -> Vec<(u64, [f32; 3])>,
        bones: Option<&[[f32; 4]]>,
        time: f32,
    ) {
        if self.running.len() != strips.len() || self.time.is_none_or(|last| time < last) {
            self.running = (0..strips.len())
                .map(|at| RunningContrail { random: at as u32 * 977 + 1, ..Default::default() })
                .collect();
            self.carried = vec![BTreeMap::new(); strips.len()];
            self.time = Some(time);
        }
        let dt = time - self.time.unwrap_or(time);
        self.time = Some(time);
        if dt <= 0.0 {
            return;
        }
        for (index, (strip, running)) in strips.iter().zip(&mut self.running).enumerate() {
            let StripKind::Contrail(contrail) = &*strip.kind else {
                continue;
            };
            if strip.carried {
                let live = tracks(strip);
                let carried = &mut self.carried[index];
                for &(id, place) in &live {
                    let (born, follower) = carried.entry(id).or_insert_with(|| {
                        (time, RunningContrail { random: (id as u32).wrapping_mul(977) | 1, previous: place, location: Some(place), ..Default::default() })
                    });
                    if let Some(state) = strip.state(None, time, *born) {
                        follower.location = Some(place);
                        follower.pulse(contrail, &strip.look, &state, place, dt);
                    }
                }
                for (id, (born, follower)) in carried.iter_mut() {
                    let state = strip.state(None, time, *born).unwrap_or_default();
                    follower.update(contrail, &state, dt);
                    if !live.iter().any(|(known, _)| known == id) {
                        follower.location = None;
                    }
                }
                carried.retain(|_, (_, follower)| follower.location.is_some() || !follower.profiles.is_empty());
                continue;
            }
            let at = strip.origin(bones);
            let last = running.location.replace(at);
            if last.is_none() {
                running.previous = at;
            }
            running.speed = last.map_or(0.0, |last| {
                let moved = minus(at, last);
                dot(moved, moved).sqrt() / dt
            });
            let values = values(strip.object);
            let onset = onsets.of(strip.object, &strip.scale_by, values.as_ref(), time);
            let state = strip.state(values.as_ref(), time, onset);
            if let Some(state) = state {
                let state = StripState { speed: running.speed, ..state };
                running.pulse(contrail, &strip.look, &state, at, dt);
                running.update(contrail, &state, dt);
            } else {
                let rest = StripState { game_time: time.rem_euclid(1.0), age: 1.0, seeds: strip.seeds, ..Default::default() };
                running.update(contrail, &rest, dt);
            }
        }
    }

    /// The triangles of a beam or contrail this frame, at the area screen
    /// effects' exposure `stops`.
    pub(crate) fn vertices(
        &self,
        index: usize,
        strip: &H3StripAt,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        lighting: &[[f32; 4]; 10],
        time: f32,
        onset: f32,
        stops: f32,
    ) -> Vec<H3ParticleVertex> {
        match &*strip.kind {
            StripKind::Contrail(contrail) if strip.carried => {
                let followers = self.carried.get(index).into_iter().flatten();
                followers
                    .flat_map(|(_, (born, follower))| {
                        strip.contrail_vertices(contrail, follower, None, view, lighting, time, *born, stops)
                    })
                    .collect()
            }
            StripKind::Beam(beam) => strip.beam_vertices(beam, values, bones, view, time, onset, stops),
            StripKind::Contrail(contrail) => self
                .running
                .get(index)
                .map_or_else(Vec::new, |running| {
                    strip.contrail_vertices(contrail, running, values, view, lighting, time, onset, stops)
                }),
        }
    }
}

attached!(H3StripAt);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::harness::kit::{kit_root, kit_source};
    use crate::core::game::GameId;
    use super::super::super::shaders::H3Blend;
    use super::super::super::pipeline::EXPOSURE as H3_EXPOSURE;
    use super::super::super::super::reach::lighting::{reach_alt_exposure, reach_exposure};

    #[test]
    fn a_beam_always_has_two_profiles_and_never_more_than_its_rows() {
        assert_eq!(beam_profiles(0.1, 0.1), 2);
        assert_eq!(beam_profiles(3.0, 2.2), 7);
        assert_eq!(beam_profiles(100.0, 100.0), MOST_PROFILES);
    }

    #[test]
    fn a_ribbon_runs_across_its_first_axis_and_a_cross_adds_the_second() {
        assert_eq!(StripShape::Ribbon.strips(), 1);
        assert_eq!(StripShape::Cross.across(0, 1), [0.5, 0.0]);
        assert_eq!(StripShape::Cross.across(1, 0), [0.0, -0.5]);
        assert_eq!(StripShape::Ngon(6).strips(), 6);
    }

    #[test]
    fn straight_up_has_a_basis_of_its_own() {
        assert_eq!(world_basis([0.0, 0.0, 1.0]), [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0]]);
        assert_eq!(world_basis([1.0, 0.0, 0.0])[0], [0.0, -1.0, 0.0]);
    }

    fn constant(value: f32) -> StripProperty {
        StripProperty::fixed(value).unwrap()
    }

    fn contrail(rate: f32, lifespan: f32) -> ContrailDefinition {
        ContrailDefinition {
            emission_rate: constant(rate),
            lifespan: constant(lifespan),
            acceleration: constant(0.0),
            size: constant(1.0),
            offset: constant(0.0),
            rotation: constant(0.0),
            color: constant(1.0),
            alpha: constant(1.0),
            secondary_alpha: constant(1.0),
            black_point: constant(0.0),
            palette: constant(0.0),
            intensity: constant(1.0),
            tint_from_lightmap: false,
            opacity_from_a: false,
            random_u: false,
            random_v: false,
        }
    }

    fn look(origin_fade: Option<[f32; 2]>) -> StripLook {
        StripLook {
            shape: StripShape::Ribbon,
            double_sided: true,
            tiling: [1.0; 2],
            scroll: [0.0; 2],
            origin_fade,
            edge_fade: None,
            angle_fade: None,
            sort_bias: 0.0,
            reach: false,
            shader: Arc::new(H3SpriteShader::blank()),
        }
    }

    #[test]
    fn a_property_a_tag_is_too_old_for_stays_at_its_default() {
        let state = StripState { profile_age: 0.5, length: 3.0, ..Default::default() };
        assert_eq!(StripProperty::fixed(1.0).unwrap().scalar(&state), 1.0);
        assert_eq!(StripProperty::fixed(0.0).unwrap().scalar(&state), 0.0);
    }

    #[test]
    fn a_vector_goes_around_and_opposite_ends_have_no_way_to() {
        let state = StripState::default();
        let mut property = constant(0.5);
        property.ends = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let turned = property.vector(&state);
        assert!((dot(turned, turned) - 1.0).abs() < 1e-3, "{turned:?} is not across the corner");
        property.ends = [[0.0, 0.0, 0.1], [0.0, 0.0, -0.1]];
        assert!(!property.vector(&state)[2].is_finite());
        property.ends = [[0.0, 0.0, 0.2], [0.0; 3]];
        assert!((property.vector(&state)[2] - 0.2 * std::f32::consts::FRAC_PI_4.sin()).abs() < 1e-5);
    }

    /// A new system's first pulse lays a permanent profile where it starts,
    /// so moving on extends from there instead of replacing it.
    #[test]
    fn a_new_contrail_is_anchored_where_it_starts() {
        let (contrail, look) = (contrail(20.0, 10.0), look(None));
        let state = StripState::default();
        let mut running = RunningContrail { previous: [2.0, 0.0, 0.0], ..Default::default() };
        running.pulse(&contrail, &look, &state, [2.0, 0.0, 0.0], 0.1);
        assert_eq!(running.profiles.len(), 1);
        assert!(!running.profiles[0].temporary, "the start is permanent");
        assert!(running.initialized);
        running.pulse(&contrail, &look, &state, [2.01, 0.0, 0.0], 0.001);
        assert_eq!(running.profiles[0].place, [2.0, 0.0, 0.0], "the start stays put");
        assert!(running.profiles.last().unwrap().temporary, "the far end is the tip");
    }

    #[test]
    fn a_contrail_left_where_it_stands_lays_down_nothing_after_its_start() {
        let (contrail, look) = (contrail(30.0, 1.0), look(None));
        let mut running = RunningContrail::default();
        let state = StripState::default();
        running.pulse(&contrail, &look, &state, [0.0; 3], 0.1);
        let first = running.profiles.len();
        assert!(first > 0);
        for _ in 0..10 {
            running.pulse(&contrail, &look, &state, [0.0; 3], 0.1);
        }
        assert_eq!(running.profiles.len(), first);
    }

    #[test]
    fn a_move_under_the_epsilon_is_no_move() {
        let (contrail, look) = (contrail(100.0, 10.0), look(None));
        let state = StripState::default();
        let mut running = RunningContrail::default();
        running.pulse(&contrail, &look, &state, [0.0; 3], 0.1);
        let started = running.profiles.len();
        running.pulse(&contrail, &look, &state, [0.00005, 0.0, 0.0], 0.1);
        assert_eq!(running.profiles.len(), started, "too little to have a direction");
        running.pulse(&contrail, &look, &state, [0.0002, 0.0, 0.0], 0.1);
        assert!(running.profiles.len() > started, "and this is enough");
    }

    #[test]
    fn moving_lays_profiles_along_the_way_and_the_tip_is_replaced() {
        let (contrail, look) = (contrail(20.0, 10.0), look(None));
        let mut running = RunningContrail { initialized: true, ..Default::default() };
        let state = StripState::default();
        running.pulse(&contrail, &look, &state, [1.0, 0.0, 0.0], 0.1);
        assert_eq!(running.profiles.len(), 2, "two laid, none left over for a tip");
        assert!((running.profiles[1].place[0] - 1.0).abs() < 1e-5);
        assert!((running.length - 1.0).abs() < 1e-5);
        running.pulse(&contrail, &look, &state, [1.5, 0.0, 0.0], 0.025);
        assert_eq!(running.profiles.len(), 3);
        assert!(running.profiles[2].temporary);
        running.pulse(&contrail, &look, &state, [1.75, 0.0, 0.0], 0.0125);
        assert_eq!(running.profiles.len(), 3, "the tip moved rather than grew");
        assert!((running.profiles[2].place[0] - 1.75).abs() < 1e-5);
    }

    #[test]
    fn a_fade_gets_profiles_at_both_its_ends() {
        let (contrail, look) = (contrail(0.0, 10.0), look(Some([2.0, 0.25])));
        let mut running = RunningContrail { initialized: true, ..Default::default() };
        running.pulse(&contrail, &look, &StripState::default(), [1.0, 0.0, 0.0], 0.1);
        let lengths: Vec<f32> = running.profiles.iter().map(|profile| profile.length).collect();
        assert_eq!(lengths, [0.25, 0.75, 1.0]);
    }

    fn carried_strip() -> Arc<H3StripAt> {
        Arc::new(H3StripAt {
            object: 0,
            node: 0,
            position: [0.0; 3],
            forward: [1.0, 0.0, 0.0],
            world_oriented: false,
            carried: true,
            end: None,
            only_under: None,
            scale_by: String::new(),
            secondary_by: String::new(),
            seeds: [0.0; OWN_SEEDS],
            times: Arc::new(EffectTimes::read_nothing()),
            event: 0,
            look: Arc::new(look(None)),
            kind: Arc::new(StripKind::Contrail(contrail(60.0, 0.5))),
        })
    }

    #[test]
    fn a_carried_contrail_trails_its_particle_and_fades_after_it_dies() {
        let strip = carried_strip();
        let strips = [Arc::clone(&strip)];
        let mut field = H3StripField::default();
        let mut onsets = EffectOnsets::default();
        let view = ViewFrame { eye: [0.0, -5.0, 0.0], forward: [0.0, 1.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0] };
        let lighting = [[0.0; 4]; 10];
        let mut advance = |field: &mut H3StripField, place: Option<[f32; 3]>, time: f32| {
            let tracks = |_: &Arc<H3StripAt>| place.map(|place| (7, place)).into_iter().collect();
            field.advance(&strips, |_| None, &mut onsets, tracks, None, time);
        };
        advance(&mut field, Some([0.0; 3]), 0.0);
        for step in 1..=10 {
            let time = step as f32 / 30.0;
            advance(&mut field, Some([time, 0.0, 0.0]), time);
        }
        let drawn = field.vertices(0, &strip, None, None, &view, &lighting, 0.33, 0.0, 0.0);
        assert!(!drawn.is_empty(), "a moving particle leaves a trail");
        let reach = drawn.iter().map(|vertex| vertex.place[0]).fold(0.0, f32::max);
        assert!(reach > 0.2, "along where it went: {reach}");
        advance(&mut field, None, 0.4);
        assert!(!field.vertices(0, &strip, None, None, &view, &lighting, 0.4, 0.0, 0.0).is_empty(), "it outlives the particle");
        for step in 0..30 {
            advance(&mut field, None, 0.45 + step as f32 * 0.1);
        }
        assert!(field.vertices(0, &strip, None, None, &view, &lighting, 3.5, 0.0, 0.0).is_empty());
        assert!(field.carried[0].is_empty(), "nothing is left to run");
    }

    fn reach_look(origin_fade: Option<[f32; 2]>) -> StripLook {
        StripLook { reach: true, ..look(origin_fade) }
    }

    /// Reach's beams fade with a range alone, as `laser_emitter`'s origin
    /// does; its contrails and Halo 3's want a cutoff.
    #[test]
    fn reach_beams_fade_with_a_range_or_a_cutoff_and_contrails_with_a_cutoff() {
        assert_eq!(strip_fade(20.0, 0.0, 1.0, true), Some([0.05, 0.0]));
        assert_eq!(strip_fade(20.0, 0.0, 1.0, false), None, "a contrail's, or Halo 3's");
        assert_eq!(strip_fade(0.0, 0.0, 1.0, true), None, "neither set");
        assert_eq!(strip_fade(0.0, 2.0, 1.0, false), Some([f32::INFINITY, 2.0]), "a cutoff alone is a step");
    }

    /// A range of 0 makes the fade a step, with a NaN on it that Reach's
    /// `saturate` takes to 0 and Halo 3's clamp passes on.
    #[test]
    fn a_reach_step_fade_is_0_at_its_cutoff() {
        let step = f32::INFINITY * 0.0;
        assert_eq!(reach_look(None).fade(step), 0.0);
        assert!(look(None).fade(step).is_nan(), "Halo 3's is as it was");
        assert_eq!(reach_look(None).fade(f32::INFINITY), 1.0);
        assert_eq!(reach_look(None).fade(0.25), look(None).fade(0.25));
    }

    /// `effects__beam.hlsl` l.296–305: 0 where the side faces the camera,
    /// fading over the range past the begin angle.
    #[test]
    fn a_reach_beam_fades_its_sides_seen_edge_on() {
        let profile = Profile { place: [0.0; 3], direction: [1.0, 0.0, 0.0], ..Default::default() };
        // Far down -y, so every corner is seen along +y.
        let eye = [0.0, -1.0e4, 0.0];
        let (range, cutoff) = (20f32.to_radians(), 45f32.to_radians());
        // Toward the eye from the axis, its side faces it; above, it is edge on.
        let facing = angle_fade([0.0, -0.5, 0.0], &profile, eye, range, cutoff);
        let edge_on = angle_fade([0.0, 0.0, 0.5], &profile, eye, range, cutoff);
        assert_eq!(facing, 1.0);
        assert_eq!(edge_on, 0.0);
        // 55 degrees round from facing: half way through 45 to 65.
        let turned = 55f32.to_radians();
        let between = [0.0, -turned.cos(), turned.sin()].map(|axis| axis * 0.5);
        let half = angle_fade(between, &profile, eye, range, cutoff);
        assert!((half - 0.5).abs() < 1e-3, "{half}");
        assert_eq!(angle_fade(between, &profile, eye, 0.0, cutoff), 0.0, "past the begin, a step");
        assert_eq!(angle_fade([0.0, -0.5, 0.0], &profile, eye, 0.0, cutoff), 1.0, "before it");
    }

    /// `particle_exposure`'s rule, at the stops: Halo 3's at its own.
    #[test]
    fn a_reach_strip_is_exposed_by_its_blend_and_the_stops() {
        let blended = |blend: H3Blend, reach: bool| {
            let mut shader = H3SpriteShader::blank();
            shader.blend = blend;
            StripLook { reach, shader: Arc::new(shader), ..look(None) }
        };
        let red = |look: &StripLook, stops: f32| exposed(look, [1.0, 0.0, 0.0], 1.0, 1.0, stops)[0];
        let additive = blended(H3Blend::Additive, true);
        assert_eq!(red(&additive, 0.0), reach_alt_exposure(0.0)[1]);
        assert_eq!(red(&additive, 1.0), reach_alt_exposure(1.0)[1]);
        assert_ne!(red(&additive, 1.0), red(&additive, 0.0));
        assert_eq!(red(&blended(H3Blend::AlphaBlend, true), 1.0), reach_exposure(1.0)[0]);
        assert_eq!(red(&blended(H3Blend::Multiply, true), 1.0), 1.0);
        assert_eq!(red(&blended(H3Blend::Additive, false), 1.0), H3_EXPOSURE, "Halo 3's ignore the stops");
    }

    /// Reach's beams scroll on a minute's clock and its contrails on 131
    /// seconds'; Halo 3's run on.
    #[test]
    fn reach_strips_scroll_on_a_clock_that_wraps() {
        assert_eq!(REACH_CONTRAIL_SCROLL_PERIOD, 131.0);
        assert_eq!(reach_look(None).scroll_time(REACH_BEAM_SCROLL_PERIOD, 61.5), 1.5);
        assert_eq!(reach_look(None).scroll_time(REACH_CONTRAIL_SCROLL_PERIOD, 132.0), 1.0);
        assert_eq!(look(None).scroll_time(REACH_BEAM_SCROLL_PERIOD, 61.5), 61.5);
    }

    /// `c_contrail::pulse` holds Reach's emission rate to 100000.
    #[test]
    fn reach_holds_a_contrails_emission_rate_down() {
        let contrail = contrail(1.0e6, 10.0);
        let state = StripState::default();
        let pulsed = |look: &StripLook| {
            let mut running = RunningContrail { initialized: true, ..Default::default() };
            running.pulse(&contrail, look, &state, [1.0, 0.0, 0.0], 5.0e-6);
            running.accumulator
        };
        assert!((pulsed(&reach_look(None)) - 0.5).abs() < 1e-4, "{}", pulsed(&reach_look(None)));
        assert!(pulsed(&look(None)) < 1.0, "Halo 3's laid five, and has what is left");
        let laid = |look: &StripLook| {
            let mut running = RunningContrail { initialized: true, ..Default::default() };
            running.pulse(&contrail, look, &state, [1.0, 0.0, 0.0], 5.0e-6);
            running.profiles.len()
        };
        assert_eq!(laid(&reach_look(None)), 1, "only the tip");
        assert_eq!(laid(&look(None)), 5);
    }

    fn reach_kit() -> Option<TagSource> {
        let root = kit_root(GameId::HaloReach)?;
        Some(super::super::super::super::particles::loose_kit(&root, GameId::HaloReach))
    }

    fn looks_of(source: &TagSource, path: &str, group: &str, code: &[u8; 4], blocks: &str) -> Vec<StripLook> {
        let tag = load_referenced_tag_from_source(source, path, group, code).expect(path);
        let beam = blocks == "beams";
        elements(tag.root(), blocks).into_iter().map(|element| read_strip_look(&crate::app::model_preview::load::TagReader::new(source), element, beam).expect(path)).collect()
    }

    /// With `BLAM_TEST_HREK`: the focus rifle's beams read Reach's plasma
    /// without its shift or second sprite, at a particle alpha of 1, and its
    /// pulse's angle fade in radians.
    #[test]
    fn kit_reach_focus_rifle_beams_take_one_texcoord_and_an_angle_fade() {
        let Some(source) = reach_kit() else {
            return;
        };
        let beams = looks_of(&source, "objects\\weapons\\rifle\\focus_rifle\\fx\\projectile_3p", "beam_system", b"beam", "beams");
        use super::super::super::super::particles::H3ParticleAlbedo;
        assert_eq!(beams[1].shader.albedo, H3ParticleAlbedo::Palette2dPlasma);
        assert_eq!(beams[2].shader.albedo, H3ParticleAlbedo::PalettePlasma);
        for plasma in &beams[1..3] {
            assert!(plasma.reach && plasma.shader.unit_particle_alpha && !plasma.shader.plasma_second_sprite);
            assert!(plasma.shader.palette_shift.is_none());
        }
        let [range, begin] = beams[3].angle_fade.expect("the pulse's angle fade");
        assert!((range - 20f32.to_radians()).abs() < 1e-6 && (begin - 45f32.to_radians()).abs() < 1e-6);
        assert!(beams[0].angle_fade.is_none());
        let laser = looks_of(&source, "levels\\dlc\\cex_beaver_creek\\fx\\laser_emitter", "beam_system", b"beam", "beams");
        assert_eq!(laser[0].origin_fade, Some([0.05, 0.0]), "a range and no cutoff");
        let trail = looks_of(&source, "cinematics\\050lb_reunited\\fx\\missile_trail", "contrail_system", b"cntl", "contrails");
        assert!((trail[0].sort_bias - 0.15).abs() < 1e-6, "3 steps of 0.05: {}", trail[0].sort_bias);
        assert!(!trail[0].shader.unit_particle_alpha, "a contrail's alpha is its own");
    }

    /// With `BLAM_TEST_H3EK`: Halo 3's beams keep their particle alpha, sort
    /// bias and fades as they were.
    #[test]
    fn kit_halo_3_beams_keep_their_own_look() {
        let Some(source) = kit_source(GameId::Halo3) else { return };
        let beams = looks_of(&source, "objects\\equipment\\autoturret\\fx\\projectile", "beam_system", b"beam", "beams");
        assert!(!beams.is_empty());
        for beam in &beams {
            assert!(!beam.reach && !beam.shader.unit_particle_alpha && beam.angle_fade.is_none());
            assert_eq!(beam.sort_bias, 0.0);
        }
    }

    #[test]
    fn profiles_age_out() {
        let (contrail, look) = (contrail(10.0, 0.5), look(None));
        let mut running = RunningContrail { initialized: true, ..Default::default() };
        let state = StripState::default();
        running.pulse(&contrail, &look, &state, [1.0, 0.0, 0.0], 0.1);
        assert!(!running.profiles.is_empty());
        running.update(&contrail, &state, 0.25);
        assert!(!running.profiles.is_empty());
        running.update(&contrail, &state, 0.25);
        assert!(running.profiles.is_empty());
    }
}
