//! Halo 3's beams and contrails: ribbons, crosses and tubes of profiles. A beam
//! is worked out whole each frame; a contrail leaves profiles behind as its
//! location moves, so one on a still object draws nothing, as in the game.

use std::sync::Arc;

use blam_tags::{TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::ce_attachments::elements;
use super::ce_lights::{CeFlareView, posed_on};
use super::h2_functions::H2FunctionValues;
use super::h2_passes::tag_function_at;
use super::h3_effects::{H3EffectOnsets, H3EffectTimes, for_each_attached_part, part_frame, part_markers, read_part_placement};
use super::h3_particles::{H3ParticleVertex, H3SpriteShader, lightmap_tint, read_ends, read_sprite_shader};
use super::h3_shaders::H3Blend;
use super::renderer::H3_EXPOSURE;
use super::*;
use crate::core::source::TagSource;

/// `beams.inl`'s limit, and what a contrail's rows hold.
const MOST_PROFILES: usize = 128;
const FEWEST_PROFILES: f32 = 2.0;
/// `k_real_epsilon`.
const EPSILON: f32 = 1.1920929e-7;
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
        let input = |field: &str| StripInput::named(&property.read_enum_name(field).unwrap_or_default());
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

    fn vector(&self, state: &StripState) -> [f32; 3] {
        let t = self.scalar(state);
        std::array::from_fn(|axis| self.ends[0][axis] + (self.ends[1][axis] - self.ends[0][axis]) * t)
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
    shader: Arc<H3SpriteShader>,
}

fn read_strip_look(source: &TagSource, element: TagStruct<'_>) -> Option<StripLook> {
    let shader = read_sprite_shader(source, element.descend("actual shader?")?)?;
    let shape = match element.read_enum_name("profile shape").as_deref() {
        Some("cross") => StripShape::Cross,
        Some("n-gon") => {
            let sides = element.read_int_any("number of n-gon sides").unwrap_or(0).max(0) as usize;
            StripShape::Ngon(sides.max(3))
        }
        _ => StripShape::Ribbon,
    };
    let flags = element.read_flag_names("appearance flags").unwrap_or_default();
    let tiling = element.read_vec2("uv tiling");
    let scroll = element.read_vec2("uv scrolling");
    let real = |name: &str| element.read_real(name).unwrap_or(0.0);
    let fade = |range: f32, cutoff: f32, scale: f32| {
        (cutoff != 0.0).then(|| [(range * scale).recip(), cutoff * scale])
    };
    let degrees = std::f32::consts::PI / 180.0;
    Some(StripLook {
        shape,
        double_sided: flags.iter().any(|(_, name)| name.starts_with("double-sided")),
        // The postprocess's: no tiling is one tile.
        tiling: [tiling.i, tiling.j].map(|rate| if rate == 0.0 { 1.0 } else { rate }),
        scroll: [scroll.i, scroll.j],
        origin_fade: fade(real("origin fade range"), real("origin fade cutoff"), 1.0),
        edge_fade: fade(real("edge fade range"), real("edge fade cutoff"), degrees),
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
    let flags = element.read_flag_names("appearance flags").unwrap_or_default();
    let flagged = |wanted: &str| flags.iter().any(|(_, name)| name.starts_with(wanted));
    let mut contrail = ContrailDefinition {
        emission_rate: read("emission rate")?,
        lifespan: read("profile lifespan")?,
        acceleration: read("profile self acceleration")?,
        size: read("profile size")?,
        offset: read("profile offset")?,
        rotation: read("profile rotation")?,
        color: read("profile color")?,
        alpha: read("profile alpha")?,
        secondary_alpha: read("profile secondary alpha")?,
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
    /// `secondary location (beams)`: where a beam runs to, instead of its length.
    end: Option<(usize, [f32; 3])>,
    only_under: Option<(String, String)>,
    scale_by: String,
    secondary_by: String,
    seeds: [f32; OWN_SEEDS],
    times: Arc<H3EffectTimes>,
    event: usize,
    look: Arc<StripLook>,
    kind: Arc<StripKind>,
}

fn seeds_of(index: usize) -> [f32; OWN_SEEDS] {
    let mut random = (index as u32).wrapping_mul(2654435761) | 1;
    std::array::from_fn(|_| {
        random = random.wrapping_mul(1664525).wrapping_add(1013904223);
        (random >> 8) as f32 / (1u32 << 24) as f32
    })
}

/// The beams and contrails of an object's attached effects, one at each marker
/// of a location.
pub(super) fn read_h3_strips(
    source: &TagSource,
    object: &blam_tags::TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H3StripAt> {
    let mut strips = Vec::new();
    let beam_group = u32::from_be_bytes(*b"beam");
    let contrail_group = u32::from_be_bytes(*b"cntl");
    for_each_attached_part(source, object, |part| {
        type Read = fn(TagStruct<'_>) -> Option<StripKind>;
        let (blocks, group_name, code, read): (&str, &str, &[u8; 4], Read) = if part.group == beam_group {
                ("beams", "beam_system", b"beam", |element| read_beam(element).map(StripKind::Beam))
            } else if part.group == contrail_group {
                ("contrails", "contrail_system", b"cntl", |element| read_contrail(element).map(StripKind::Contrail))
            } else {
                return;
            };
        let Ok(tag) = load_referenced_tag_from_source(source, &part.path, group_name, code) else {
            return;
        };
        let (offset, yaw, pitch) = read_part_placement(part.part);
        for element in elements(tag.root(), blocks) {
            let (Some(look), Some(kind)) = (read_strip_look(source, element), read(element)) else {
                continue;
            };
            let (look, kind) = (Arc::new(look), Arc::new(kind));
            for marker in part_markers(markers, &part.marker, part.effect.once) {
                let (place, node) = frame(marker);
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

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn minus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt().max(1e-6);
    a.map(|axis| axis / length)
}

/// `cross_section_world_basis`.
fn world_basis(direction: [f32; 3]) -> [[f32; 3]; 2] {
    let product = cross(direction, [0.0, 0.0, 1.0]);
    if product == [0.0; 3] {
        return [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
    }
    let first = unit(product);
    [first, unit(cross(first, direction))]
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
    view: &CeFlareView,
    time: f32,
    uv_offset: [f32; 2],
    origin_fade: impl Fn(&Profile, [f32; 3]) -> f32,
    edge_view: impl Fn([f32; 3]) -> [f32; 3],
) -> Vec<H3ParticleVertex> {
    let shape = look.shape;
    let vertex = |profile: &Profile, strip: usize, offset: usize| {
        let world = world_basis(profile.direction);
        let basis = if shape == StripShape::Ribbon {
            let first = unit(cross(minus(profile.place, view.eye), profile.direction));
            [first, unit(cross(first, profile.direction))]
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
            let normal = unit(cross(basis[0], basis[1]));
            let angle = dot(edge_view(place), normal).abs().min(1.0).acos();
            color[3] *= (range * (angle - cutoff)).clamp(0.0, 1.0);
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

fn exposed(blend: H3Blend, color: [f32; 3], alpha: f32, intensity: f32) -> [f32; 4] {
    let light = if blend == H3Blend::Multiply { 1.0 } else { H3_EXPOSURE };
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
        self.random = self.random.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.random >> 8) as f32 / (1u32 << 24) as f32
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
        if self.initialized && total <= EPSILON {
            return;
        }
        let direction = if total > 0.0 { moved.map(|axis| axis / total) } else { [0.0; 3] };
        let mut remaining = total;
        self.accumulator += contrail.emission_rate.scalar(state) * dt;
        if self.accumulator >= 1.0 {
            let each = total / self.accumulator;
            while self.accumulator >= 1.0 {
                self.extend(contrail, look, state, false, direction, each);
                remaining -= each;
                self.accumulator -= 1.0;
            }
        }
        if !self.initialized || remaining > EPSILON {
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
}

impl H3StripAt {
    pub(crate) fn only_under(&self) -> Option<&(String, String)> {
        self.only_under.as_ref()
    }

    pub(crate) fn scale_names(&self) -> [&str; 2] {
        [&self.scale_by, &self.secondary_by]
    }

    pub(crate) fn on_attached(mut self, name: &str) -> Self {
        if let Some((region, _)) = &mut self.only_under {
            *region = format!("{name}: {region}");
        }
        self
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    pub(crate) fn shader(&self) -> &H3SpriteShader {
        &self.look.shader
    }

    /// Its sort position, as `submit` hands it over.
    pub(crate) fn origin(&self, bones: Option<&[[f32; 4]]>) -> [f32; 3] {
        posed_on(self.node, self.position, self.forward, bones).0
    }

    /// The states this frame, or none while its event is not running.
    fn state(&self, values: Option<&H2FunctionValues>, time: f32, onset: f32) -> Option<StripState> {
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
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        time: f32,
        onset: f32,
    ) -> Vec<H3ParticleVertex> {
        let Some(mut state) = self.state(values, time, onset) else {
            return Vec::new();
        };
        let (origin, forward) = posed_on(self.node, self.position, self.forward, bones);
        let uncapped = beam.length.scalar(&state);
        let reach = self.end.map(|(node, end)| minus(posed_on(node, end, [1.0, 0.0, 0.0], bones).0, origin));
        let (direction, cap) = match reach {
            Some(reach) => (unit(reach), dot(reach, reach).sqrt()),
            None => (unit(forward), 0.0),
        };
        let capped = if cap != 0.0 && cap <= uncapped { cap } else { uncapped };
        state.length = capped;
        let density = beam.density.scalar(&state);
        let count = beam_profiles(capped, density);
        let step = (density * capped).max(1e-6).recip();
        let cap_percentage = if uncapped != 0.0 { capped / uncapped } else { 1.0 };
        let offset = beam.offset.scalar(&state);
        let blend = self.look.shader.blend;
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
                    color: exposed(blend, beam.color.color(&state), alpha, intensity),
                    palette: beam.palette.scalar(&state).clamp(0.0, 0.9999),
                    black_point: beam.black_point.scalar(&state).clamp(0.0, 0.9999),
                    ..Default::default()
                }
            })
            .collect();
        let fade = self.look.origin_fade;
        strip_vertices(
            &self.look,
            &profiles,
            view,
            time,
            [0.0; 2],
            |_, place| {
                fade.map_or(1.0, |[range, cutoff]| {
                    let from = minus(place, origin);
                    (range * (dot(from, from).sqrt() - cutoff)).clamp(0.0, 1.0)
                })
            },
            |place| unit(minus(place, view.eye)),
        )
    }

    fn contrail_vertices(
        &self,
        contrail: &ContrailDefinition,
        running: &RunningContrail,
        values: Option<&H2FunctionValues>,
        view: &CeFlareView,
        lighting: &[[f32; 4]; 10],
        time: f32,
        onset: f32,
    ) -> Vec<H3ParticleVertex> {
        if running.profiles.len() < 2 {
            return Vec::new();
        }
        let base = StripState {
            speed: running.speed,
            ..self.state(values, time, onset).unwrap_or(StripState { game_time: time.rem_euclid(1.0), age: 1.0, seeds: self.seeds, ..Default::default() })
        };
        let blend = self.look.shader.blend;
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
                let mut color = exposed(blend, contrail.color.color(&state), alpha, contrail.intensity.scalar(&state));
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
        let fade = self.look.origin_fade;
        let uv_offset = [
            if contrail.random_u { self.seeds[0] } else { 0.0 },
            if contrail.random_v { self.seeds[1] } else { 0.0 },
        ];
        strip_vertices(
            &self.look,
            &profiles,
            view,
            time,
            uv_offset,
            |profile, _| fade.map_or(1.0, |[range, cutoff]| (range * (profile.length - cutoff)).clamp(0.0, 1.0)),
            |_| view.forward,
        )
    }
}

impl H3StripField {
    /// Run contrails up to `time`, with their locations where `bones` has them.
    pub(crate) fn advance(
        &mut self,
        strips: &[Arc<H3StripAt>],
        values: impl Fn(u8) -> Option<H2FunctionValues>,
        onsets: &mut H3EffectOnsets,
        bones: Option<&[[f32; 4]]>,
        time: f32,
    ) {
        if self.running.len() != strips.len() || self.time.is_none_or(|last| time < last) {
            self.running = (0..strips.len())
                .map(|at| RunningContrail { random: at as u32 * 977 + 1, ..Default::default() })
                .collect();
            self.time = Some(time);
        }
        let dt = time - self.time.unwrap_or(time);
        self.time = Some(time);
        if dt <= 0.0 {
            return;
        }
        for (strip, running) in strips.iter().zip(&mut self.running) {
            let StripKind::Contrail(contrail) = &*strip.kind else {
                continue;
            };
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

    /// The triangles of a beam or contrail this frame.
    pub(crate) fn vertices(
        &self,
        index: usize,
        strip: &H3StripAt,
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        lighting: &[[f32; 4]; 10],
        time: f32,
        onset: f32,
    ) -> Vec<H3ParticleVertex> {
        match &*strip.kind {
            StripKind::Beam(beam) => strip.beam_vertices(beam, values, bones, view, time, onset),
            StripKind::Contrail(contrail) => self
                .running
                .get(index)
                .map_or_else(Vec::new, |running| strip.contrail_vertices(contrail, running, values, view, lighting, time, onset)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn constant_function(value: f32) -> TagFunction {
        let mut function = blam_tags::tag_function::H2Function::new(blam_tags::tag_function::FunctionType::Identity);
        function.set_clamp_range(value, value).unwrap();
        let encoding = blam_tags::tag_function::FunctionEncoding::H2;
        TagFunction::parse_encoded(encoding, &function.to_bytes()).unwrap()
    }

    fn constant(value: f32) -> StripProperty {
        StripProperty {
            input: StripInput::Invalid,
            range: StripInput::Invalid,
            adds: None,
            modifier: StripInput::Invalid,
            function: constant_function(value),
            ends: [[0.0; 3]; 2],
        }
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
            shader: Arc::new(H3SpriteShader::blank()),
        }
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
