//! Halo 3's particle systems on an attached effect. The game updates most of
//! these on the card, but nothing in the update needs it, so they run here, a
//! fixed step at a time, and draw as quads. Models, distortion and collision
//! are left out: a preview has no level to hit.

use std::sync::Arc;

use blam_tags::{TagFieldData, TagFile, TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::h3_effects::adopt_event_function;
use super::ce_attachments::elements;
use super::ce_lights::CeFlareView;
use super::h2_functions::H2FunctionValues;
use super::h2_passes::tag_function_at;
use super::h3_effects::H3EffectOnsets;
use super::h3_effects::{H3EffectTimes, H3ObjectEffect, h3_object_effects, part_markers, part_runs_by};
use super::h3_strips::{H3StripAt, read_carried_strips};
use super::h3_shaders::{H3Blend, read_h3_definition};
use super::materials::{H3Bitmap, read_h3_bitmap};
use super::renderer::H3_EXPOSURE;
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;

/// The game's tick, which everything here steps by.
const STEP: f32 = 1.0 / 30.0;
/// Seconds run before the first frame, so a held pose has its particles.
const PRE_ROLL: f32 = 6.0;
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
    Rotation,
    LocationSeed,
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
            Self::Random | Self::SystemRandom | Self::Other => 0.0,
        }
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
}

/// `allocate_random_seed`'s two counters, handing seeds out in property order.
#[derive(Default)]
struct SeedAllocator {
    particle: usize,
    system: usize,
}

impl SeedAllocator {
    fn allocate(&mut self, input: &mut H3ParticleInput) {
        match *input {
            H3ParticleInput::Random => {
                *input = H3ParticleInput::Seed(self.particle % SEEDS);
                self.particle += 1;
            }
            H3ParticleInput::SystemRandom => {
                *input = H3ParticleInput::SystemSeed(self.system % SYSTEM_SEEDS);
                self.system += 1;
            }
            _ => {}
        }
    }
}

impl H3ParticleProperty {
    fn read(property: TagStruct<'_>, seeds: &mut SeedAllocator) -> Option<Self> {
        let input = |field: &str| H3ParticleInput::named(&property.read_enum_name(field).unwrap_or_default());
        let adds = match property.read_enum_name("Output Modifier").as_deref() {
            Some("Plus") => Some(true),
            Some("Times") => Some(false),
            _ => None,
        };
        let function = tag_function_at(&property, "Mapping")?;
        let (mut input_at, mut range, mut modifier) =
            (input("Input Variable"), input("Range Variable"), input("Output Modifier Input"));
        if function.as_constant().is_none() {
            seeds.allocate(&mut input_at);
            seeds.allocate(&mut range);
            if adds.is_some() {
                seeds.allocate(&mut modifier);
            }
        }
        Some(Self {
            input: input_at,
            range,
            adds,
            modifier,
            function: Arc::new(function),
            ends: read_ends(property),
        })
    }

    fn named(holder: TagStruct<'_>, name: &str, seeds: &mut SeedAllocator) -> Option<Self> {
        Self::read(holder.descend(name)?, seeds)
    }

    fn scalar(&self, state: &H3ParticleState) -> f32 {
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
            _ => [0.0; 3],
        }
    };
    [end("Starting interpolant"), end("Ending interpolant")]
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
}

impl H3ParticleAlbedo {
    fn named(name: &str) -> Self {
        match name {
            "diffuse_plus_billboard_alpha" => Self::DiffuseBillboardAlpha,
            "diffuse_plus_sprite_alpha" => Self::DiffuseSpriteAlpha,
            "palettized" => Self::Palette,
            "palettized_plus_billboard_alpha" => Self::PaletteBillboardAlpha,
            "palettized_plus_sprite_alpha" | "palettized_plus_alpha" => Self::PaletteSpriteAlpha,
            _ => Self::Diffuse,
        }
    }
}

/// A `.particle`.
#[derive(Debug)]
pub(crate) struct H3ParticleLook {
    billboard: H3Billboard,
    flip_u: bool,
    flip_v: bool,
    random_rotation: bool,
    intensity_fades: bool,
    one_shot: bool,
    backwards: bool,
    /// `attachments`: effects its particles start, by what starts them.
    attachments: Vec<(H3ParticleTrigger, String)>,
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
    aspect: H3ParticleProperty,
    color: H3ParticleProperty,
    intensity: H3ParticleProperty,
    alpha: H3ParticleProperty,
    frame: H3ParticleProperty,
    rate: H3ParticleProperty,
    palette: H3ParticleProperty,
    pub(crate) shader: Arc<H3SpriteShader>,
}

/// The `lighting` option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H3ParticleLighting {
    None,
    Ambient,
    Directional,
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
    pub(crate) add: [f32; 3],
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
}

impl H3SpriteShader {
    #[cfg(test)]
    pub(crate) fn blank() -> Self {
        Self {
            albedo: H3ParticleAlbedo::Diffuse,
            blend: H3Blend::Additive,
            black_point: false,
            frame_blend: false,
            depth_fade: None,
            add: [0.0; 3],
            lit_pixels: false,
            sort_layer: 2,
            distortion: None,
            lighting: H3ParticleLighting::None,
            uv_scale: [1.0; 2],
            base_map: None,
            alpha_map: None,
            palette_map: None,
        }
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

/// A sprite system's `actual shader?`, by its options' names.
pub(super) fn read_sprite_shader(source: &TagSource, shader: TagStruct<'_>) -> Option<H3SpriteShader> {
    let definition = shader.read_tag_ref_path("definition").filter(|path| !path.is_empty())?;
    let definition = read_h3_definition(source, &definition)?;
    let options: Vec<usize> = elements(shader, "options")
        .iter()
        .map(|option| option.read_int_any("short").unwrap_or(0).max(0) as usize)
        .collect();
    let chosen = |category: &str| definition.chosen(category, &options).unwrap_or_default().to_owned();
    let add = parameter(shader, "self_illum_color")
        .filter(|_| chosen("self_illumination") == "constant_color")
        .and_then(|parameter| elements(parameter, "animated parameters").first().copied())
        .and_then(|animated| tag_function_at(&animated, "function"))
        .map(|function| function.evaluate_color(0.0, 0.0))
        .map_or([0.0; 3], |color| [color.red, color.green, color.blue]);
    Some(H3SpriteShader {
        albedo: H3ParticleAlbedo::named(&chosen("albedo")),
        blend: definition.blend(&options),
        black_point: chosen("black_point") == "on",
        frame_blend: chosen("frame_blend") == "on",
        depth_fade: (chosen("depth_fade") == "on" && chosen("blend_mode") != "opaque").then(|| {
            let range = parameter(shader, "depth_fade_range")
                .and_then(|parameter| elements(parameter, "animated parameters").first().copied())
                .and_then(|animated| tag_function_at(&animated, "function"))
                .map_or(1.0, |function| function.evaluate(0.0, 0.0));
            range.max(1e-4)
        }),
        add,
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
            _ => H3ParticleLighting::None,
        },
        uv_scale: ["starting_uv_scale", "ending_uv_scale"].map(|name| {
            parameter(shader, name)
                .and_then(|parameter| elements(parameter, "animated parameters").first().copied())
                .and_then(|animated| tag_function_at(&animated, "function"))
                .map_or(1.0, |function| function.evaluate(0.0, 0.0))
        }),
        base_map: read_map(source, shader, "base_map"),
        alpha_map: read_map(source, shader, "alpha_map"),
        palette_map: read_map(source, shader, "palette"),
    })
}

fn parameter<'a>(shader: TagStruct<'a>, name: &str) -> Option<TagStruct<'a>> {
    let parameters = elements(shader, "parameters");
    parameters.into_iter().find(|parameter| parameter.read_string_id("parameter name").as_deref() == Some(name))
}

fn read_map(source: &TagSource, shader: TagStruct<'_>, name: &str) -> Option<H3Bitmap> {
    let path = parameter(shader, name)?.read_tag_ref_path("bitmap").filter(|path| !path.is_empty())?;
    read_h3_bitmap(source, &path, 0, SamplerKind::Texture2D)
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
    let shader = root.descend("actual shader?")?;
    let flags = root.read_flag_names("appearance flags").unwrap_or_default();
    let flagged = |wanted: &str| flags.iter().any(|(_, name)| name.starts_with(wanted));
    let animation = root.read_flag_names("animation flags").unwrap_or_default();
    let animated = |wanted: &str| animation.iter().any(|(_, name)| name.starts_with(wanted));
    let first = root.read_int_any("first sequence index").unwrap_or(0).max(0) as usize;
    let offset = root.read_point2d("center offset");
    let start = tag_function_at(&root.descend("frame index")?, "Mapping").map_or(0.0, |function| function.evaluate(0.0, 0.0));
    let (frames, corner) = read_frames(source, shader, first, [offset.x, offset.y], start, animated("one shot"));
    let mut seeds = SeedAllocator::default();
    Some(H3ParticleLook {
        billboard: H3Billboard::named(&root.read_enum_name("particle billboard style").unwrap_or_default()),
        flip_u: flagged("random u mirror"),
        flip_v: flagged("random v mirror"),
        random_rotation: flagged("random starting rotation"),
        intensity_fades: flagged("intensity affects alpha"),
        model,
        vertical: flagged("bitmap authored vertically"),
        dies_on_ground: root
            .read_flag_names("main flags")
            .unwrap_or_default()
            .iter()
            .any(|(_, name)| name.starts_with("dies on structure collision")),
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
        edge_fade: flagged("fade when viewed edge-on").then(|| {
            let real = |name: &str| root.read_real(name).unwrap_or(0.0);
            let range = real("angle fade range");
            let range = if range < 0.0001 { 30.0 } else { range };
            let radians = std::f32::consts::PI / 180.0;
            [(range * radians).recip(), real("angle fade cutoff") * radians]
        }),
        blur_aspect: flagged("motion blur").then(|| root.read_real("motion blur aspect scale").unwrap_or(0.0)),
        curvature: root.read_real("curvature").unwrap_or(0.0).clamp(0.0, 1.0) * 0.9999,
        one_shot: animated("one shot"),
        backwards: animated("can animate backwards"),
        corner,
        frames,
        aspect: H3ParticleProperty::named(root, "aspect ratio", &mut seeds)?,
        color: H3ParticleProperty::named(root, "color", &mut seeds)?,
        intensity: H3ParticleProperty::named(root, "intensity", &mut seeds)?,
        alpha: H3ParticleProperty::named(root, "alpha", &mut seeds)?,
        frame: H3ParticleProperty::named(root, "frame index", &mut seeds)?,
        rate: H3ParticleProperty::named(root, "animation rate", &mut seeds)?,
        palette: H3ParticleProperty::named(root, "palette animation", &mut seeds)?,
        shader: Arc::new(read_sprite_shader(source, shader)?),
    })
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
            "impact contour" => Self::ImpactContour,
            "impact area" => Self::ImpactArea,
            "breakable surface" => Self::Breakable,
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

fn read_collider(source: &TagSource, emitter: TagStruct<'_>, seeds: &mut SeedAllocator) -> Option<H3ParticleCollider> {
    let movement = emitter.descend("particle movement")?;
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

fn read_flock(source: &TagSource, emitter: TagStruct<'_>, seeds: &mut SeedAllocator) -> Option<H3ParticleFlock> {
    let movement = emitter.descend("particle movement")?;
    let template = movement
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| load_referenced_tag_from_source(source, &path, "particle_physics", b"pmov").ok());
    let own = elements(movement, "movements");
    let movements = if own.is_empty() { template.as_ref().map_or(Vec::new(), |tag| elements(tag.root(), "movements")) } else { own };
    let swarm = movements.into_iter().find(|entry| entry.read_enum_name("type").as_deref() == Some("swarm"))?;
    let mut by_id = |id: i128| {
        let parameters = elements(swarm, "parameters");
        let found = parameters.into_iter().find(|parameter| parameter.read_int_any("parameter id") == Some(id))?;
        H3ParticleProperty::named(found, "property", seeds)
    };
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

impl H3ParticleFlock {
    /// `update_particles_flocking`. Avoidance is of a world the preview lacks.
    fn update(&self, particles: &mut [Particle], base: H3ParticleState, home: [f32; 3], dt: f32) {
        let places: Vec<([f32; 3], [f32; 3])> = particles.iter().map(|particle| (particle.position, particle.velocity)).collect();
        for (at, particle) in particles.iter_mut().enumerate() {
            let state = H3ParticleState { age: particle.age, seeds: particle.seeds, rotation: particle.rotation, ..base };
            let distance_to = |other: &([f32; 3], [f32; 3])| {
                let delta = minus(other.0, particle.position);
                dot(delta, delta)
            };
            let neighbors = places.iter().enumerate().filter(|(index, _)| *index != at).map(|(_, neighbor)| neighbor);
            let neighbor = neighbors.min_by(|a, b| distance_to(a).total_cmp(&distance_to(b)));
            let mut force = [0.0; 3];
            let mut add = |amount: [f32; 3]| force = std::array::from_fn(|axis| force[axis] + amount[axis]);
            if let Some((place, velocity)) = neighbor {
                let weight = self.separation_weight.scalar(&state);
                if weight != 0.0 {
                    let away = minus(particle.position, *place);
                    let ratio = dot(away, away).sqrt() / self.separation_distance.scalar(&state).max(0.1);
                    let scale = if ratio < 1.0 { -ratio * weight } else { weight };
                    add(unit(away).map(|axis| axis * scale));
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
    shape: H3EmissionShape,
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
}

fn read_physics(source: &TagSource, emitter: TagStruct<'_>, seeds: &mut SeedAllocator) -> Option<H3ParticlePhysics> {
    let movement = emitter.descend("particle movement")?;
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

fn read_emitter(source: &TagSource, emitter: TagStruct<'_>) -> Option<H3Emitter> {
    let mut seeds = SeedAllocator::default();
    let shape = H3EmissionShape::named(&emitter.read_enum_name("emission shape").unwrap_or_default())?;
    let mut property = |name: &str| H3ParticleProperty::named(emitter, name, &mut seeds);
    let (offset, direction, radius, angle) = (
        property("translational offset")?,
        property("relative direction")?,
        property("emission radius")?,
        property("emission angle")?,
    );
    let (starting, most, rate, lifespan) = (
        property("particle starting count")?,
        property("particle max count")?,
        property("particle emission rate")?,
        property("particle lifespan")?,
    );
    let (acceleration, speed, rotation, rotation_rate) = (
        property("particle self-acceleration")?,
        property("particle initial velocity")?,
        property("particle rotation")?,
        property("particle initial rotation rate")?,
    );
    let (size, scale, tint, alpha, black_point) = (
        property("particle size")?,
        property("particle scale")?,
        property("particle tint")?,
        property("particle alpha")?,
        property("particle alpha black point")?,
    );
    let axis_angle = property("emission axis angle")?;
    let physics = read_physics(source, emitter, &mut seeds);
    let flock = read_flock(source, emitter, &mut seeds);
    let collider = read_collider(source, emitter, &mut seeds);
    let scroll = emitter.read_vec2("uv scrolling");
    Some(H3Emitter {
        axis: H3ParticleAxis::named(&emitter.read_enum_name("particle axis (for models)").unwrap_or_default()),
        reference_axis: match emitter.read_enum_name("particle reference axis").as_deref() {
            Some("y") => 1,
            Some("z") => 2,
            _ => 0,
        },
        axis_angle,
        scroll: [scroll.i, scroll.j],
        shape, offset, direction, radius, angle, starting, most, rate, lifespan, acceleration, speed,
        rotation, rotation_rate, size, scale, tint, alpha, black_point, physics, flock, collider,
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
    let look = system.read_tag_ref_path("particle").filter(|path| !path.is_empty())?;
    let look = read_look(source, &look).map(Arc::new)?;
    let emitters: Vec<_> = elements(system, "emitters")
        .iter()
        .filter_map(|emitter| read_emitter(source, *emitter).map(Arc::new))
        .collect();
    if emitters.is_empty() {
        return None;
    }
    let real = |field: &str| system.read_real(field).unwrap_or(0.0);
    let cutoff = if flagged("override near fade") { real("near fade override") } else { real("near fade cutoff") };
    Some(H3SystemParts {
        hidden: flagged("don't render system"),
        die_with_effect: flagged("particles die when effect ends"),
        lod: {
            let out = real("LOD out distance");
            (!flagged("lod always 1.0") && !flagged("ignore lod") && out > 0.0).then(|| {
                [real("LOD in distance"), out, real("LOD feather in delta"), real("LOD feather out delta")]
            })
        },
        sort_bias: system.read_int_any("sort bias").unwrap_or(0) as f32,
        world: system.read_enum_name("coordinate system").as_deref() == Some("world"),
        // As `1 / range` comes out on the card.
        near_range: real("near fade range").recip(),
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
                for marker in part_markers(markers, &name, object_effect.once) {
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
struct Random(u32);

impl Random {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 16) as f32 / 65_535.0
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
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
fn emit(shape: H3EmissionShape, radius: f32, angle_degrees: f32, random: &mut Random) -> ([f32; 3], [f32; 3]) {
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
        H3EmissionShape::Line => ([0.0, random.range(-radius, radius), 0.0], spread(angle)),
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
        H3EmissionShape::Breakable => ([0.0; 3], [0.0; 3]),
    }
}

#[derive(Debug, Clone)]
struct Particle {
    /// Unique among its emitter's, for what follows it.
    id: u64,
    /// In contact, so that a collision's attachments play once.
    colliding: bool,
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
    system_seeds: [f32; SYSTEM_SEEDS],
}

#[derive(Debug, Clone)]
struct RunningEmitter {
    particles: Vec<Particle>,
    made: u64,
    accumulator: f32,
    started: bool,
    random: Random,
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
}

/// `particle_block.h`'s limit on systems.
const MOST_INSTANCES: usize = 320;

fn fresh_emitters(system: &H3ParticleSystem, seed: u32) -> Vec<RunningEmitter> {
    let emitters = system.emitters.iter().enumerate();
    emitters
        .map(|(index, _)| RunningEmitter {
            particles: Vec::new(),
            made: 0,
            accumulator: 0.0,
            started: false,
            random: Random(seed.wrapping_mul(977) ^ (index as u32).wrapping_mul(131)),
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

/// One step of one location, returning what its particles started and ended.
#[allow(clippy::too_many_arguments)]
fn step_location(
    system: &H3ParticleSystem,
    emitters: &mut [RunningEmitter],
    state: Option<H3ParticleState>,
    origin: [f32; 3],
    axes: [[f32; 3]; 3],
    now: f32,
    ground: f32,
    births: &mut Vec<(u64, [f32; 3], [f32; 3])>,
    deaths: &mut Vec<(u64, [f32; 3], [f32; 3])>,
    hits: &mut Vec<(u64, [f32; 3], [f32; 3])>,
    tracks: &mut Vec<H3ParticleTrack>,
) {
    tracks.clear();
    for (at, (emitter, running)) in system.emitters.iter().zip(emitters).enumerate() {
        // What was made outlives its event, and goes on falling and landing.
        let rest = H3ParticleState { system_age: 1.0, game_time: now.rem_euclid(1.0), ..Default::default() };
        let frame = emitter_frame(emitter, origin, axes, state.as_ref().unwrap_or(&rest));
        let home = if system.world { origin } else { [0.0; 3] };
        // The ground, in whichever frame the particles live.
        let plane = if system.world {
            ([0.0, 0.0, 1.0], -ground)
        } else {
            (std::array::from_fn(|row| frame.1[row][2]), frame.0[2] - ground)
        };
        if let Some(mut state) = state {
            let random = &mut running.random;
            state.system_seeds = std::array::from_fn(|at| {
                let mut seeded = Random(at as u32 + 1 + random.0 / 7);
                seeded.next()
            });
            emitter.pulse(&system.look, state, running, STEP, frame, system.world, births, at);
            let ground = Some((plane, system.look.dies_on_ground, &mut *hits));
            emitter.step(state, &mut running.particles, STEP, home, deaths, ground);
        } else {
            // The next run of its event is a new location's, with its burst.
            running.started = false;
            let ground = Some((plane, system.look.dies_on_ground, &mut *hits));
            emitter.step(rest, &mut running.particles, STEP, home, deaths, ground);
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
    onset: f32,
) -> Option<H3ParticleState> {
    let read = |name: &str| {
        if name.is_empty() { 1.0 } else { values.and_then(|values| values.get(name)).copied().unwrap_or(0.0) }
    };
    let effect_a = read(&system.scale_by);
    if effect_a <= 0.0 {
        return None;
    }
    Some(H3ParticleState {
        system_age: system.times.age(system.event, time - onset)?,
        game_time: time.rem_euclid(1.0),
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

/// `system_state` for a step ending at `time`, and for an event that came and
/// went within it: the game's tick is half a step, and its one-tick events are many.
fn system_state_stepped(
    system: &H3ParticleSystem,
    values: Option<&H2FunctionValues>,
    time: f32,
    onset: f32,
) -> Option<H3ParticleState> {
    system_state(system, values, time, onset).or_else(|| {
        let begin = system.times.begins_within(system.event, time - onset - STEP, time - onset)?;
        system_state(system, values, begin + onset, onset)
    })
}

impl H3Emitter {
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
        running.accumulator = running.accumulator.fract();
        if !running.started {
            running.started = true;
            running.accumulator += self.starting.scalar(&state);
        }
        running.accumulator += self.rate.scalar(&state) * dt;
        let most = self.most.scalar(&state);
        if most > 0.0 {
            running.accumulator = running.accumulator.min(most - running.particles.len() as f32).max(0.0);
        }
        for _ in 0..running.accumulator.floor() as usize {
            if running.particles.len() >= MOST_PARTICLES {
                break;
            }
            let random = &mut running.random;
            let seeds: [f32; SEEDS] = std::array::from_fn(|_| random.next());
            let mut particle_state = H3ParticleState { seeds, emission_time: state.system_age, ..state };
            particle_state.system_seeds = state.system_seeds;
            let (position, mut velocity) = emit(
                self.shape,
                self.radius.scalar(&particle_state),
                self.angle.scalar(&particle_state),
                random,
            );
            let length = (velocity[0].powi(2) + velocity[1].powi(2) + velocity[2].powi(2)).sqrt().max(1e-6);
            let speed = self.speed.scalar(&particle_state);
            let speed = if speed == 0.0 { 0.001 } else { speed };
            velocity = velocity.map(|axis| axis / length * speed);
            let (offset, turned) = frame;
            let to_world = |vector: [f32; 3]| -> [f32; 3] {
                std::array::from_fn(|axis| (0..3).map(|at| turned[at][axis] * vector[at]).sum())
            };
            let placed = {
                let moved = to_world(position);
                std::array::from_fn(|axis| offset[axis] + moved[axis])
            };
            running.made += 1;
            let id = running.made << 8 | at as u64;
            started.push((id, placed, to_world(velocity)));
            let (position, velocity) = if world { (placed, to_world(velocity)) } else { (position, velocity) };
            let axis = self.axis.pick(self.reference_axis, self.axis_angle.scalar(&particle_state), random);
            running.particles.push(Particle {
                id,
                colliding: false,
                axis,
                age: 0.0,
                inverse_lifespan: 1.0 / self.lifespan.scalar(&particle_state).max(0.01),
                position,
                velocity,
                rotation: if look.random_rotation { random.next() } else { 0.0 },
                rotation_rate: self.rotation_rate.scalar(&particle_state),
                frame: 0.0,
                frame_rate: look.rate.scalar(&particle_state),
                seeds,
                emission_time: state.system_age,
                flip: [random.next() >= 0.5, random.next() >= 0.5, random.next() >= 0.5],
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
    ) {
        if let Some(flock) = &self.flock {
            flock.update(particles, state, home, dt);
        }
        if let (Some(collider), Some((plane, dies, hits))) = (&self.collider, ground) {
            for particle in particles.iter_mut() {
                let state = H3ParticleState { age: particle.age, seeds: particle.seeds, rotation: particle.rotation, ..state };
                let (elasticity, friction) = (collider.elasticity.scalar(&state), collider.friction.scalar(&state));
                match bounce(particle.position, particle.velocity, plane, dt, elasticity, friction) {
                    Some((place, velocity)) => {
                        particle.position = place;
                        particle.velocity = velocity;
                        particle.rotation_rate *= 1.0 - friction;
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
        particles.retain_mut(|particle| {
            particle.age += particle.inverse_lifespan * dt;
            if particle.age >= 1.0 {
                ended.push((particle.id, particle.position, particle.velocity));
                return false;
            }
            let state = H3ParticleState {
                age: particle.age,
                seeds: particle.seeds,
                system_seeds: particle.system_seeds,
                emission_time: particle.emission_time,
                rotation: particle.rotation,
                ..state
            };
            for axis in 0..3 {
                particle.position[axis] += particle.velocity[axis] * dt;
            }
            let acceleration = self.acceleration.vector(&state);
            for axis in 0..3 {
                particle.velocity[axis] += acceleration[axis] * dt;
            }
            if let Some(physics) = &self.physics {
                let read = |property: &Option<H3ParticleProperty>| property.as_ref().map(|property| property.scalar(&state));
                particle.velocity[2] -= read(&physics.gravity).unwrap_or(1.0) * GRAVITY * dt;
                let air = (read(&physics.air_friction).unwrap_or(1.0) * dt).clamp(0.0, 1.0);
                particle.velocity = particle.velocity.map(|axis| axis - air * axis);
                let spin = (read(&physics.rotational_friction).unwrap_or(0.0) * dt).clamp(0.0, 1.0);
                particle.rotation_rate -= spin * particle.rotation_rate;
            }
            particle.rotation = (particle.rotation + particle.rotation_rate * dt).rem_euclid(1.0);
            particle.frame = (particle.frame + particle.frame_rate * dt).rem_euclid(1.0);
            true
        });
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
    /// Run everything up to `time`. A new field starts a pre-roll short, so a
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
        let Some(mut now) = self.time else {
            return;
        };
        while now + STEP <= time {
            now += STEP;
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
                        if system.die_with_effect && age >= system.times.once() {
                            run.running.iter_mut().for_each(|running| running.particles.clear());
                        }
                        let state = (age < system.times.once()).then(|| system_state_stepped(system, None, now, run.start)).flatten();
                        let state = with_lod(system, state, run.origin, eye);
                        step_location(system, &mut run.running, state, run.origin, run.axes, now, ground, &mut births, &mut deaths, &mut hits, &mut no_tracks);
                        age < system.times.once() || run.running.iter().any(|running| !running.particles.is_empty())
                    });
                    self.instances[index] = runs;
                    continue;
                }
                let scope = values(system.object);
                let onset = onsets.of(system.object, &system.scale_by, scope.as_ref(), now);
                let state = system_state_stepped(system, scope.as_ref(), now, onset);
                let (origin, axes) = location(system, bones);
                let state = with_lod(system, state, origin, eye);
                step_location(system, &mut self.running[index], state, origin, axes, now, ground, &mut births, &mut deaths, &mut hits, &mut self.tracks[index]);
                let off = !system.scale_by.is_empty()
                    && !scope.as_ref().and_then(|values| values.get(&system.scale_by)).is_some_and(|value| *value > 0.0);
                if system.die_with_effect && (off || system.times.is_over(now - onset)) {
                    self.running[index].iter_mut().for_each(|running| running.particles.clear());
                }
                for (trigger, offset) in &system.attached {
                    let events = match trigger {
                        H3ParticleTrigger::Birth => &births,
                        H3ParticleTrigger::Death => &deaths,
                        H3ParticleTrigger::Collision => &hits,
                    };
                    let follow = *trigger == H3ParticleTrigger::Birth;
                    spawned.extend(events.iter().map(|&(id, place, velocity)| {
                        ((index + offset, index), follow.then_some(id), (place, velocity), system.world, origin, axes)
                    }));
                }
            }
            for ((index, parent), follows, (place, velocity), world, origin, axes) in spawned {
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
                });
            }
        }
        self.time = Some(now);
    }

    /// The quads of a system, in a world-space triangle list.
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
    ) -> Vec<H3ParticleVertex> {
        let mut vertices = Vec::new();
        if system.hidden {
            return vertices;
        }
        if system.child {
            for run in self.instances.get(index).into_iter().flatten() {
                let at = (run.origin, run.axes, run.start);
                system.vertices_at(&run.running, at, values, view, lighting, time, &mut vertices);
            }
            return vertices;
        }
        let Some(running) = self.running.get(index) else {
            return vertices;
        };
        let (origin, axes) = location(system, bones);
        system.vertices_at(running, (origin, axes, onset), values, view, lighting, time, &mut vertices);
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
        vertices: &mut Vec<H3ParticleVertex>,
    ) {
        let system = self;
        let look = &*system.look;
        for (emitter, running) in system.emitters.iter().zip(running) {
            let base = H3ParticleState {
                system_age: system.times.age(system.event, time - onset).unwrap_or(1.0),
                game_time: time.rem_euclid(1.0),
                effect_a: system_state(system, values, time, onset).map_or(0.0, |state| state.effect_a),
                effect_b: system_state(system, values, time, onset).map_or(1.0, |state| state.effect_b),
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
                let state = H3ParticleState {
                    age: particle.age,
                    seeds: particle.seeds,
                    system_seeds: particle.system_seeds,
                    emission_time: particle.emission_time,
                    rotation: particle.rotation,
                    ..base
                };
                let to_world = |vector: [f32; 3]| -> [f32; 3] {
                    std::array::from_fn(|axis| (0..3).map(|at| turned[at][axis] * vector[at]).sum())
                };
                let place: [f32; 3] = {
                    let moved = to_world(particle.position);
                    std::array::from_fn(|axis| offset[axis] + moved[axis])
                };
                let velocity = to_world(particle.velocity);
                let size = emitter.size.scalar(&state) * emitter.scale.scalar(&state);
                let mut aspect = look.aspect.scalar(&state);
                if let Some(stretch) = look.blur_aspect {
                    // A baseline of 30, not the frame rate. The preview's observer is still.
                    aspect += dot(velocity, velocity).sqrt() * stretch / (30.0 * size);
                }
                let basis = billboard(look.billboard, place, velocity, turned, view);
                let manual = emitter.rotation.scalar(&state).rem_euclid(1.0);
                let turned_up = if look.vertical { 0.25 } else { 0.0 };
                let (sin, cos) = (std::f32::consts::TAU * (particle.rotation + manual + turned_up)).sin_cos();
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
                let light = if look.shader.blend == H3Blend::Multiply { 1.0 } else { H3_EXPOSURE };
                let initial = if look.tint_from_lightmap { lightmap_tint(lighting) } else { [1.0; 3] };
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
                let lit_by = |normal: [f32; 3]| {
                    if shading == H3ParticleLighting::Ambient { ravi(shading, normal, lighting) } else { [1.0; 3] }
                };
                let count = look.frames.len();
                let frame_at = particle.frame + look.frame.scalar(&state).rem_euclid(1.0);
                let frame = frame_variant(frame_at, count, look.one_shot, look.backwards && particle.flip[2]);
                let first = look.frames[frame.floor().rem_euclid(count as f32) as usize % count];
                let second = look.frames[(frame + 1.0).floor().rem_euclid(count as f32) as usize % count];
                let blend = if look.shader.frame_blend { frame.fract() } else { 0.0 };
                let scroll = emitter.scroll.map(|rate| rate * time);
                let [from, to] = look.shader.uv_scale;
                let uv_scale = |at: f32| (from + (to - from) * at / 2.0).recip();
                let (first_scale, second_scale) =
                    if look.shader.frame_blend { (uv_scale(1.0 + blend), uv_scale(blend)) } else { (1.0, 1.0) };
                let black = emitter.black_point.scalar(&state).clamp(0.0, 0.9999);
                let palette = look.palette.scalar(&state).clamp(0.0, 0.9999);
                let [x, y, width, height] = look.corner;
                let at_spot = |spot: [f32; 3], mut uv: [f32; 2], normal: [f32; 3], lit: [f32; 3], lean: [[f32; 3]; 2]| {
                    let mut faded = alpha;
                    if let Some([range, cutoff]) = look.edge_fade {
                        let toward = unit(minus(spot, view.eye));
                        let angle = std::f32::consts::FRAC_PI_2 - dot(toward, normal).abs().min(1.0).acos();
                        faded *= (range * (angle - cutoff)).clamp(0.0, 1.0);
                    }
                    if look.flip_u && particle.flip[0] {
                        uv[0] = 1.0 - uv[0];
                    }
                    if look.flip_v && particle.flip[1] {
                        uv[1] = 1.0 - uv[1];
                    }
                    H3ParticleVertex {
                        place: spot,
                        sprite: sprite_uv(first, uv, scroll, first_scale),
                        next_sprite: sprite_uv(second, uv, scroll, second_scale),
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
    let direction = unit(emitter.direction.vector(state));
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
    fn a_line_runs_along_y_and_a_breakable_has_nothing_of_its_own() {
        let mut random = Random(3);
        let (place, _) = emit(H3EmissionShape::Line, 2.0, 0.0, &mut random);
        assert_eq!(place[0], 0.0);
        assert!(place[1].abs() <= 2.0 && place[1] != 0.0);
        assert_eq!(emit(H3EmissionShape::Breakable, 2.0, 30.0, &mut random), ([0.0; 3], [0.0; 3]));
    }

    #[test]
    fn an_impact_with_no_size_sprays_from_one_point_at_its_angle() {
        let mut random = Random(5);
        for shape in [H3EmissionShape::ImpactContour, H3EmissionShape::ImpactArea] {
            let (place, velocity) = emit(shape, 9.0, 60.0, &mut random);
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
            let (place, direction) = emit(H3EmissionShape::Sprayer, 2.0, 10.0, &mut random);
            assert_eq!(place, [0.0; 3]);
            assert!(direction[0] >= 10f32.to_radians().cos() - 1e-5, "{direction:?}");
        }
    }

    #[test]
    fn a_globe_starts_on_its_radius_heading_out_and_an_implode_in() {
        let mut random = Random(9);
        let (place, direction) = emit(H3EmissionShape::Globe, 2.0, 0.0, &mut random);
        assert!((dot(place, place).sqrt() - 2.0).abs() < 1e-4);
        assert!(dot(place, direction) > 0.0);
        let (place, direction) = emit(H3EmissionShape::Implode, 2.0, 0.0, &mut random);
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
}
