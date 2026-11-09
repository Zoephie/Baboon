//! Reading beams and contrails: their properties, shapes, looks and definitions, and the strips on an object.

use super::super::super::super::{RenderModelPreviewMarker, TagReader};
use super::{
    EffectTimes, MarkerFrame, SpriteShader, elements, for_each_attached_part, has_flag,
    high24_over_2_24, lcg_step, part_frame, read_ends, read_part_placement, read_sprite_shader,
    slerped, tag_function_at,
};
use blam_tags::{TagFunction, TagStruct};
use std::sync::Arc;

/// `beams.inl`'s limit, and what a contrail's rows hold.
pub(super) const MOST_PROFILES: usize = 128;

pub(super) const FEWEST_PROFILES: f32 = 2.0;

/// `k_real_epsilon`.
pub(in crate::app::model_preview) const EPSILON: f32 = 0.0001;

/// Reach's beam uv scroll clock, `g_render_time+0x10`: the frames' time,
/// wrapped at a minute (`c_beam_gpu::set_shader_state<render>`, the 360's
/// 0x826B2F98, MCC sub_14088B5B0).
pub(super) const BEAM_SCROLL_PERIOD: f32 = 60.0;

/// Reach's contrail's, `get_game_time_seconds_repeat_arbitrary(0x830000)`:
/// a count of 1/65536 seconds wrapped at 0x830000, 131 seconds
/// (`c_contrail_gpu::set_shader_state<render>`, MCC sub_14085A3D0).
pub(super) const CONTRAIL_SCROLL_PERIOD: f32 = 0x83_0000 as f32 / 65_536.0;

/// What Reach's `c_contrail::pulse` (the 360's 0x82ECB188) holds a
/// contrail's emission rate to before it accumulates.
pub(super) const MOST_PULSED_EMISSION_RATE: f32 = 100_000.0;

/// `get_sort_bias` (beam 0x82EA2548, contrail 0x82ECB7D8): world units a step
/// of `sort bias`.
const SORT_BIAS_STEP: f32 = 0.05;

pub(super) const OWN_SEEDS: usize = 2;

pub(super) const PROFILE_SEEDS: usize = 4;

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
pub(super) struct StripState {
    pub(super) position: f32,
    pub(super) uncapped_position: f32,
    pub(super) profile_age: f32,
    pub(super) profile_seeds: [f32; PROFILE_SEEDS],
    pub(super) game_time: f32,
    pub(super) age: f32,
    pub(super) seeds: [f32; OWN_SEEDS],
    pub(super) length: f32,
    pub(super) speed: f32,
    pub(super) effect_a: f32,
    pub(super) effect_b: f32,
}

/// `c_editable_property_base`.
#[derive(Debug)]
pub(super) struct StripProperty {
    input: StripInput,
    range: StripInput,
    adds: Option<bool>,
    modifier: StripInput,
    function: TagFunction,
    pub(super) ends: [[f32; 3]; 2],
}

impl StripProperty {
    fn read(holder: TagStruct<'_>, name: &str) -> Option<Self> {
        let property = holder.descend(name)?;
        let input =
            |field: &str| StripInput::named(&property.read_enum_option(field).unwrap_or_default());
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
    pub(super) fn fixed(value: f32) -> Option<Self> {
        use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};
        let mut function = H2Function::new(FunctionType::Identity);
        function.set_clamp_range(value, value).ok()?;
        Some(Self {
            input: StripInput::Invalid,
            range: StripInput::Invalid,
            adds: None,
            modifier: StripInput::Invalid,
            function: TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes())
                .ok()?,
            ends: [[0.0; 3]; 2],
        })
    }

    pub(super) fn scalar(&self, state: &StripState) -> f32 {
        let value = self
            .function
            .evaluate(self.input.of(state), self.range.of(state));
        match self.adds {
            Some(true) => value + self.modifier.of(state),
            Some(false) => value * self.modifier.of(state),
            None => value,
        }
    }

    pub(super) fn color(&self, state: &StripState) -> [f32; 3] {
        let color = self
            .function
            .evaluate_color(self.input.of(state), self.range.of(state));
        [color.red, color.green, color.blue]
    }

    /// `map_to_vector3d_range`: a direction goes around, not across. Opposite
    /// ends have no way around, and it divides by nothing as the shader does.
    pub(super) fn vector(&self, state: &StripState) -> [f32; 3] {
        slerped(self.ends, self.scalar(state))
    }
}

/// `profile shape`, by option name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StripShape {
    Ribbon,
    Cross,
    Ngon(usize),
}

impl StripShape {
    pub(super) fn strips(self) -> usize {
        match self {
            Self::Ribbon => 1,
            Self::Cross => 2,
            Self::Ngon(sides) => sides,
        }
    }

    /// `strip_and_offset_to_cross_sectional_offset`.
    pub(super) fn across(self, strip: usize, offset: usize) -> [f32; 2] {
        match self {
            Self::Ribbon | Self::Cross => {
                let side = offset as f32 - 0.5;
                if strip == 0 { [side, 0.0] } else { [0.0, side] }
            }
            Self::Ngon(sides) => {
                let (sin, cos) =
                    (std::f32::consts::TAU * (strip + offset) as f32 / sides as f32).sin_cos();
                [0.5 * cos, -0.5 * sin]
            }
        }
    }

    /// `profile_offset_to_texcoord`'s `v_shift`.
    pub(super) fn v_shift(self, strip: usize, offset: usize) -> f32 {
        match self {
            Self::Ribbon | Self::Cross => offset as f32 - 0.5,
            Self::Ngon(sides) => (strip + offset) as f32 / sides as f32 - 0.5,
        }
    }
}

/// What a beam and a contrail share of their looks.
#[derive(Debug)]
pub(super) struct StripLook {
    pub(super) shape: StripShape,
    pub(super) double_sided: bool,
    pub(super) tiling: [f32; 2],
    pub(super) scroll: [f32; 2],
    /// `1 / range` and the cutoff, world units.
    pub(super) origin_fade: Option<[f32; 2]>,
    /// The same in radians.
    pub(super) edge_fade: Option<[f32; 2]>,
    /// Reach's beams' `m_angle_range` and `m_angle_cutoff`: the range and the
    /// begin angle in radians, the range not inverted.
    pub(super) angle_fade: Option<[f32; 2]>,
    /// `get_sort_bias`, in world units nearer the camera; Reach's.
    pub(super) sort_bias: f32,
    /// Reach's: its own exposure, scroll clocks, emission limit and fades.
    pub(super) own_terms: bool,
    pub(super) shader: Arc<SpriteShader>,
}

/// HLSL's `saturate`, which takes a NaN to 0 where Rust's `clamp` passes it
/// through [inferred: the DirectX 10+ rule].
pub(super) fn saturate(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

impl StripLook {
    /// A fade's `saturate`. Reach's step fades (a range of 0) meet a NaN at
    /// the cutoff, which comes out 0.
    pub(super) fn fade(&self, value: f32) -> f32 {
        if self.own_terms {
            saturate(value)
        } else {
            value.clamp(0.0, 1.0)
        }
    }

    /// The uv scroll's clock: Reach's wraps at `period`, Halo 3's runs on.
    pub(super) fn scroll_time(&self, period: f32, time: f32) -> f32 {
        if self.own_terms {
            time.rem_euclid(period)
        } else {
            time
        }
    }
}

/// An origin or edge fade's `1 / range` and cutoff, `scale` to the shader's
/// units, if its postprocess turns it on: with a cutoff, or with a range too
/// where `by_range` (Reach's beams, tool sub_140DD25D0, the 360's
/// 0x833B3460). Reach's contrails' (tool 0x140DCFCE0) go by the cutoff, as
/// Halo 3's.
pub(super) fn strip_fade(range: f32, cutoff: f32, scale: f32, by_range: bool) -> Option<[f32; 2]> {
    (cutoff != 0.0 || (by_range && range != 0.0)).then(|| [(range * scale).recip(), cutoff * scale])
}

/// `beam` is a `beam_system`'s, else a `contrail_system`'s.
pub(super) fn read_strip_look(
    reader: &TagReader,
    element: TagStruct<'_>,
    beam: bool,
) -> Option<StripLook> {
    let own_terms = reader.rules().gen3.strip_terms;
    let mut shader = read_sprite_shader(reader, element.descend("actual shader?")?)?;
    if own_terms && beam {
        // `effects__beam.hlsl`'s `sample_diffuse` (l.373–458): the strip's one
        // texcoord, a `particle_alpha` of 1, and no palette shift.
        shader.palette_shift = None;
        shader.plasma_second_sprite = false;
        shader.unit_particle_alpha = true;
    }
    let shape = match element.read_enum_option("profile shape").as_deref() {
        Some("cross") => StripShape::Cross,
        Some("n-gon") => {
            let sides = element
                .read_int_any("number of n-gon sides")
                .unwrap_or(0)
                .max(0) as usize;
            StripShape::Ngon(sides.max(3))
        }
        _ => StripShape::Ribbon,
    };
    let tiling = element.read_vec2("uv tiling");
    let scroll = element.read_vec2("uv scrolling");
    let real = |name: &str| element.read_real(name).unwrap_or(0.0);
    let by_range = own_terms && beam;
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
        sort_bias: if own_terms {
            element.read_int_any("sort bias").unwrap_or(0) as f32 * SORT_BIAS_STEP
        } else {
            0.0
        },
        own_terms,
        shader: Arc::new(shader),
    })
}

#[derive(Debug)]
pub(super) struct BeamDefinition {
    pub(super) length: StripProperty,
    pub(super) offset: StripProperty,
    pub(super) density: StripProperty,
    pub(super) rotation: StripProperty,
    pub(super) thickness: StripProperty,
    pub(super) color: StripProperty,
    pub(super) alpha: StripProperty,
    pub(super) black_point: StripProperty,
    pub(super) palette: StripProperty,
    pub(super) intensity: StripProperty,
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
        &mut beam.length,
        &mut beam.offset,
        &mut beam.density,
        &mut beam.rotation,
        &mut beam.thickness,
        &mut beam.color,
        &mut beam.alpha,
        &mut beam.black_point,
        &mut beam.palette,
        &mut beam.intensity,
    ];
    properties
        .into_iter()
        .for_each(|property| property.allocate(&mut own, &mut profile));
    Some(beam)
}

#[derive(Debug)]
pub(super) struct ContrailDefinition {
    pub(super) emission_rate: StripProperty,
    pub(super) lifespan: StripProperty,
    pub(super) acceleration: StripProperty,
    pub(super) size: StripProperty,
    pub(super) offset: StripProperty,
    pub(super) rotation: StripProperty,
    pub(super) color: StripProperty,
    pub(super) alpha: StripProperty,
    pub(super) secondary_alpha: StripProperty,
    pub(super) black_point: StripProperty,
    pub(super) palette: StripProperty,
    pub(super) intensity: StripProperty,
    pub(super) tint_from_lightmap: bool,
    pub(super) opacity_from_a: bool,
    pub(super) random_u: bool,
    pub(super) random_v: bool,
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
        &mut contrail.emission_rate,
        &mut contrail.lifespan,
        &mut contrail.acceleration,
        &mut contrail.size,
        &mut contrail.offset,
        &mut contrail.rotation,
        &mut contrail.color,
        &mut contrail.alpha,
        &mut contrail.secondary_alpha,
        &mut contrail.black_point,
        &mut contrail.palette,
        &mut contrail.intensity,
    ];
    properties
        .into_iter()
        .for_each(|property| property.allocate(&mut own, &mut profile));
    Some(contrail)
}

#[derive(Debug)]
pub(super) enum StripKind {
    Beam(BeamDefinition),
    Contrail(ContrailDefinition),
}

/// A beam or a contrail at a marker of the object it is attached to.
#[derive(Debug)]
pub(crate) struct Gen3StripAt {
    pub(crate) object: u8,
    pub(super) node: usize,
    pub(super) position: [f32; 3],
    pub(super) forward: [f32; 3],
    /// `use parent position but world orientation`: its forward does not turn.
    pub(super) world_oriented: bool,
    /// Started by a particle: it follows them, not a marker.
    pub(super) carried: bool,
    /// `secondary location (beams)`: where a beam runs to, instead of its length.
    pub(super) end: Option<(usize, [f32; 3])>,
    pub(super) only_under: Option<(String, String)>,
    pub(super) scale_by: String,
    pub(super) secondary_by: String,
    pub(super) seeds: [f32; OWN_SEEDS],
    pub(super) times: Arc<EffectTimes>,
    pub(super) event: usize,
    pub(super) look: Arc<StripLook>,
    pub(super) kind: Arc<StripKind>,
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
) -> Vec<Gen3StripAt> {
    let mut strips = Vec::new();
    let beam_group = u32::from_be_bytes(*b"beam");
    let contrail_group = u32::from_be_bytes(*b"cntl");
    for_each_attached_part(reader, object, |part| {
        type Read = fn(TagStruct<'_>) -> Option<StripKind>;
        let beam = part.group == beam_group;
        let (blocks, group_name, code, read): (&str, &str, &[u8; 4], Read) = if beam {
            ("beams", "beam_system", b"beam", |element| {
                read_beam(element).map(StripKind::Beam)
            })
        } else if part.group == contrail_group {
            ("contrails", "contrail_system", b"cntl", |element| {
                read_contrail(element).map(StripKind::Contrail)
            })
        } else {
            return;
        };
        let Ok(tag) = reader.load(&part.path, group_name, code) else {
            return;
        };
        let (offset, yaw, pitch) = read_part_placement(part.part);
        for element in elements(tag.root(), blocks) {
            let (Some(look), Some(kind)) = (read_strip_look(reader, element, beam), read(element))
            else {
                continue;
            };
            let (look, kind) = (Arc::new(look), Arc::new(kind));
            for marker in &part.effect.locations(markers, &part.marker, &frame) {
                let (place, node) = frame(marker);
                let (place, node) = part.placed(place, node);
                let place = part_frame(&place, offset, yaw, pitch);
                let end = part.secondary_marker.as_ref().and_then(|name| {
                    let far = markers
                        .iter()
                        .find(|marker| marker.name.eq_ignore_ascii_case(name))?;
                    let (far, node) = frame(far);
                    Some((node, far.position))
                });
                strips.push(Gen3StripAt {
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
pub(in crate::app::model_preview) fn read_carried_strips(
    reader: &TagReader,
    path: &str,
) -> Vec<Gen3StripAt> {
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
                let (Some(look), Some(contrail)) = (
                    read_strip_look(reader, element, false),
                    read_contrail(element),
                ) else {
                    continue;
                };
                strips.push(Gen3StripAt {
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
