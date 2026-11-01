//! Halo 3's particle systems on an attached effect. The game updates most of
//! these on the card, but nothing in the update needs it, so they run here, a
//! fixed step at a time, and draw as quads. Models, distortion and collision
//! are left out: a preview has no level to hit.

use std::sync::Arc;

use blam_tags::{TagFieldData, TagFile, TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::ce_attachments::{elements, object_block};
use super::ce_lights::CeFlareView;
use super::h2_functions::H2FunctionValues;
use super::h2_passes::tag_function_at;
use super::h3_effects::{H3EffectTimes, MOST_MARKERS, marker_name, part_runs_by};
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
    Other,
}

impl H3ParticleInput {
    fn named(name: &str) -> Self {
        match name.split('{').next().unwrap_or_default() {
            "particle age" => Self::Age,
            "system age" => Self::SystemAge,
            "particle random" => Self::Random,
            "system random" => Self::SystemRandom,
            "particle correlation 1" => Self::Seed(0),
            "particle correlation 2" => Self::Seed(1),
            "particle correlation 3" => Self::Seed(2),
            "particle correlation 4" => Self::Seed(3),
            "system correlation 1" => Self::SystemSeed(0),
            "system correlation 2" => Self::SystemSeed(1),
            "particle emission time" => Self::EmissionTime,
            "location lod" => Self::Lod,
            "game time" => Self::GameTime,
            "effect a scale" => Self::EffectA,
            "effect b scale" => Self::EffectB,
            "particle rotation" => Self::Rotation,
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
            // A preview is always as near as a system's detail goes.
            Self::Lod => 1.0,
            Self::GameTime => state.game_time,
            Self::EffectA => state.effect_a,
            Self::EffectB => state.effect_b,
            Self::Rotation => state.rotation,
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
fn read_ends(property: TagStruct<'_>) -> [[f32; 3]; 2] {
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
    World,
    VelocityHorizontal,
}

impl H3Billboard {
    fn named(name: &str) -> Self {
        match name {
            "camera facing" => Self::CameraFacing,
            "parallel to direction" | "screen parallel" => Self::ScreenParallel,
            "perpendicular to direction" | "screen perpendicular" => Self::ScreenPerpendicular,
            "vertical" | "screen vertical" | "local vertical" => Self::ScreenVertical,
            "horizontal" | "screen horizontal" | "local horizontal" => Self::ScreenHorizontal,
            "world" => Self::World,
            "velocity horizontal" => Self::VelocityHorizontal,
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
            "palettized_plus_sprite_alpha" => Self::PaletteSpriteAlpha,
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
    pub(crate) albedo: H3ParticleAlbedo,
    pub(crate) blend: H3Blend,
    pub(crate) black_point: bool,
    pub(crate) frame_blend: bool,
    pub(crate) add: [f32; 3],
    pub(crate) base_map: Option<H3Bitmap>,
    pub(crate) alpha_map: Option<H3Bitmap>,
    pub(crate) palette_map: Option<H3Bitmap>,
}

fn parameter<'a>(shader: TagStruct<'a>, name: &str) -> Option<TagStruct<'a>> {
    let parameters = elements(shader, "parameters");
    parameters.into_iter().find(|parameter| parameter.read_string_id("parameter name").as_deref() == Some(name))
}

fn read_map(source: &TagSource, shader: TagStruct<'_>, name: &str) -> Option<H3Bitmap> {
    let path = parameter(shader, name)?.read_tag_ref_path("bitmap").filter(|path| !path.is_empty())?;
    read_h3_bitmap(source, &path, 0, SamplerKind::Texture2D)
}

/// `postprocess_frame_animation`: `first + 1`'s sprites, in the bitmap's terms.
fn read_frames(source: &TagSource, shader: TagStruct<'_>, first: usize) -> (Vec<[f32; 4]>, [f32; 4]) {
    let whole = (vec![[0.0, 0.0, 1.0, 1.0]], [-1.0, -1.0, 2.0, 2.0]);
    let tag = parameter(shader, "base_map")
        .and_then(|base| base.read_tag_ref_path("bitmap"))
        .and_then(|path| load_referenced_tag_from_source(source, &path, "bitmap", b"bitm").ok());
    let Some(tag) = tag else {
        return whole;
    };
    let sequences = elements(tag.root(), "sequences");
    if sequences.is_empty() {
        return whole;
    }
    let rect = |sprite: &TagStruct<'_>| {
        let read = |name: &str| sprite.read_real(name).unwrap_or(0.0);
        let (left, right, top, bottom) = (read("left"), read("right"), read("top"), read("bottom"));
        [left, top, right - left, bottom - top]
    };
    let frames: Vec<_> = elements(sequences[(first + 1) % sequences.len()], "sprites").iter().map(rect).collect();
    let frames = if frames.is_empty() { whole.0 } else { frames };
    let sized = |[_, _, width, height]: [f32; 4]| {
        if width <= height { [-1.0 * width / height, -1.0, 2.0 * width / height, 2.0] } else { [-1.0, -1.0 * height / width, 2.0, 2.0 * height / width] }
    };
    let corner = sized(frames[0]);
    (frames, corner)
}

fn read_look(source: &TagSource, path: &str) -> Option<H3ParticleLook> {
    let tag = load_referenced_tag_from_source(source, path, "particle", b"prt3").ok()?;
    let root = tag.root();
    // Those drawn from a mesh wait on the rest of the model path.
    if root.read_tag_ref_path("Model").is_some_and(|model| !model.is_empty()) {
        return None;
    }
    let shader = root.descend("actual shader?")?;
    let definition = shader.read_tag_ref_path("definition").filter(|path| !path.is_empty())?;
    let definition = read_h3_definition(source, &definition)?;
    let options: Vec<usize> = elements(shader, "options")
        .iter()
        .map(|option| option.read_int_any("short").unwrap_or(0).max(0) as usize)
        .collect();
    let chosen = |category: &str| definition.chosen(category, &options).unwrap_or_default().to_owned();
    if chosen("specialized_rendering").starts_with("distortion") {
        return None;
    }
    let flags = root.read_flag_names("appearance flags").unwrap_or_default();
    let flagged = |wanted: &str| flags.iter().any(|(_, name)| name.starts_with(wanted));
    let animation = root.read_flag_names("animation flags").unwrap_or_default();
    let animated = |wanted: &str| animation.iter().any(|(_, name)| name.starts_with(wanted));
    let first = root.read_int_any("first sequence index").unwrap_or(0).max(0) as usize;
    let (frames, corner) = read_frames(source, shader, first);
    let mut seeds = SeedAllocator::default();
    let add = parameter(shader, "self_illum_color")
        .filter(|_| chosen("self_illumination") == "constant_color")
        .and_then(|parameter| elements(parameter, "animated parameters").first().copied())
        .and_then(|animated| tag_function_at(&animated, "function"))
        .map(|function| function.evaluate_color(0.0, 0.0))
        .map_or([0.0; 3], |color| [color.red, color.green, color.blue]);
    Some(H3ParticleLook {
        billboard: H3Billboard::named(&root.read_enum_name("particle billboard style").unwrap_or_default()),
        flip_u: flagged("random u mirror"),
        flip_v: flagged("random v mirror"),
        random_rotation: flagged("random starting rotation"),
        intensity_fades: flagged("intensity affects alpha"),
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
        albedo: H3ParticleAlbedo::named(&chosen("albedo")),
        blend: definition.blend(&options),
        black_point: chosen("black_point") == "on",
        frame_blend: chosen("frame_blend") == "on",
        add,
        base_map: read_map(source, shader, "base_map"),
        alpha_map: read_map(source, shader, "alpha_map"),
        palette_map: read_map(source, shader, "palette"),
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
            // Rectangles want an impact to size them, and breakables a surface.
            _ => return None,
        })
    }
}

/// `set_shader_update_state`'s three, each off without a physics controller.
#[derive(Debug)]
struct H3ParticlePhysics {
    gravity: Option<H3ParticleProperty>,
    air_friction: Option<H3ParticleProperty>,
    rotational_friction: Option<H3ParticleProperty>,
}

#[derive(Debug)]
struct H3Emitter {
    shape: H3EmissionShape,
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
    let physics = read_physics(source, emitter, &mut seeds);
    Some(H3Emitter {
        shape, offset, direction, radius, angle, starting, most, rate, lifespan, acceleration, speed,
        rotation, rotation_rate, size, scale, tint, alpha, black_point, physics,
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
    near_range: f32,
    near_cutoff: f32,
    times: Arc<H3EffectTimes>,
    event: usize,
    pub(crate) look: Arc<H3ParticleLook>,
    emitters: Vec<Arc<H3Emitter>>,
}

impl H3ParticleSystem {
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
    let Some(block) = object_block(object, "attachments") else {
        return systems;
    };
    let effect_group = u32::from_be_bytes(*b"effe");
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if group != effect_group || path.is_empty() {
            continue;
        }
        let Ok(effect) = load_referenced_tag_from_source(source, &path, "effect", b"effe") else {
            continue;
        };
        let locations = elements(effect.root(), "locations");
        let times = Arc::new(H3EffectTimes::read(effect.root()));
        for (at, event) in elements(effect.root(), "events").into_iter().enumerate() {
            for system in elements(event, "particle systems") {
                let flags = system.read_flag_names("flags").unwrap_or_default();
                let flagged = |wanted: &str| flags.iter().any(|(_, name)| name.starts_with(wanted));
                if flagged("don't render system") || !part_runs_by(system, "environment", "disposition") {
                    continue;
                }
                let location = usize::try_from(system.read_block_index("location")).ok();
                let Some(location) = location.and_then(|location| locations.get(location)) else {
                    continue;
                };
                let mut name = marker_name(*location, "marker name");
                if name == "replace" {
                    name = marker_name(attachment, "marker");
                }
                let Some(look) = system.read_tag_ref_path("particle").filter(|path| !path.is_empty()) else {
                    continue;
                };
                let Some(look) = read_look(source, &look).map(Arc::new) else {
                    continue;
                };
                let emitters: Vec<_> = elements(system, "emitters")
                    .iter()
                    .filter_map(|emitter| read_emitter(source, *emitter).map(Arc::new))
                    .collect();
                if emitters.is_empty() {
                    continue;
                }
                let real = |field: &str| system.read_real(field).unwrap_or(0.0);
                let cutoff = if flagged("override near fade") { real("near fade override") } else { real("near fade cutoff") };
                let range = real("near fade range");
                let named = markers.iter().filter(|marker| marker.name.eq_ignore_ascii_case(&name));
                for marker in named.take(MOST_MARKERS) {
                    let (place, node) = frame(marker);
                    systems.push(H3ParticleSystem {
                        node,
                        position: place.position,
                        axes: place.axes,
                        only_under: marker.only_under.clone(),
                        object: owner,
                        scale_by: attachment.read_string_id("primary scale").unwrap_or_default(),
                        secondary_by: attachment.read_string_id("secondary scale").unwrap_or_default(),
                        sort_bias: system.read_int_any("sort bias").unwrap_or(0) as f32,
                        near_range: if range > 0.0 { 1.0 / range } else { 0.0 },
                        near_cutoff: cutoff,
                        times: Arc::clone(&times),
                        event: at,
                        look: Arc::clone(&look),
                        emitters: emitters.clone(),
                    });
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
        H3EmissionShape::Line => ([random.range(-radius, radius), 0.0, 0.0], spread(angle)),
    }
}

#[derive(Debug, Clone)]
struct Particle {
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
}

/// The states of a system this step, or none while it should not emit.
fn system_state(
    system: &H3ParticleSystem,
    values: Option<&H2FunctionValues>,
    time: f32,
) -> Option<H3ParticleState> {
    let read = |name: &str| {
        if name.is_empty() { 1.0 } else { values.and_then(|values| values.get(name)).copied().unwrap_or(0.0) }
    };
    let effect_a = read(&system.scale_by);
    if effect_a <= 0.0 {
        return None;
    }
    Some(H3ParticleState {
        system_age: system.times.age(system.event, time)?,
        game_time: time.rem_euclid(1.0),
        effect_a,
        effect_b: read(&system.secondary_by),
        ..Default::default()
    })
}

impl H3Emitter {
    /// `c_particle_emitter::pulse` and `spawn_particle`.
    fn pulse(&self, look: &H3ParticleLook, state: H3ParticleState, running: &mut RunningEmitter, dt: f32) {
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
            running.particles.push(Particle {
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
    fn step(&self, state: H3ParticleState, particles: &mut Vec<Particle>, dt: f32) {
        particles.retain_mut(|particle| {
            particle.age += particle.inverse_lifespan * dt;
            if particle.age >= 1.0 {
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
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt().max(1e-6);
    a.map(|axis| axis / length)
}

/// `billboard_basis`.
fn billboard(style: H3Billboard, place: [f32; 3], velocity: [f32; 3], view: &CeFlareView) -> [[f32; 3]; 2] {
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

/// `frame_texcoord`, with no scroll.
fn sprite_uv(frame: [f32; 4], uv: [f32; 2]) -> [f32; 2] {
    [uv[0] * frame[2] + frame[0], uv[1] * frame[3] + frame[1]]
}

impl H3ParticleField {
    /// Run everything up to `time`. A new field starts a pre-roll short, so a
    /// held pose has particles in flight from its first frame.
    pub(crate) fn advance(
        &mut self,
        systems: &[Arc<H3ParticleSystem>],
        values: impl Fn(u8) -> Option<H2FunctionValues>,
        time: f32,
    ) {
        if self.running.len() != systems.len() || self.time.is_none_or(|last| time < last) {
            self.running = systems
                .iter()
                .enumerate()
                .map(|(at, system)| {
                    let emitters = system.emitters.iter().enumerate();
                    emitters
                        .map(|(index, _)| RunningEmitter {
                            particles: Vec::new(),
                            accumulator: 0.0,
                            started: false,
                            random: Random((at as u32).wrapping_mul(977) ^ (index as u32).wrapping_mul(131)),
                        })
                        .collect()
                })
                .collect();
            self.time = Some(time - PRE_ROLL);
        }
        let Some(mut now) = self.time else {
            return;
        };
        while now + STEP <= time {
            now += STEP;
            for (system, emitters) in systems.iter().zip(&mut self.running) {
                let values = values(system.object);
                let state = system_state(system, values.as_ref(), now);
                for (emitter, running) in system.emitters.iter().zip(emitters) {
                    if let Some(mut state) = state {
                        let random = &mut running.random;
                        state.system_seeds = std::array::from_fn(|at| {
                            let mut seeded = Random(at as u32 + 1 + random.0 / 7);
                            seeded.next()
                        });
                        emitter.pulse(&system.look, state, running, STEP);
                        emitter.step(state, &mut running.particles, STEP);
                    } else {
                        let rest = H3ParticleState {
                            system_age: 1.0,
                            game_time: now.rem_euclid(1.0),
                            ..Default::default()
                        };
                        emitter.step(rest, &mut running.particles, STEP);
                    }
                }
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
        time: f32,
    ) -> Vec<H3ParticleVertex> {
        let mut vertices = Vec::new();
        let Some(running) = self.running.get(index) else {
            return vertices;
        };
        let (origin, axes) = location(system, bones);
        let look = &*system.look;
        for (emitter, running) in system.emitters.iter().zip(running) {
            let base = H3ParticleState {
                system_age: system.times.age(system.event, time).unwrap_or(1.0),
                game_time: time.rem_euclid(1.0),
                effect_a: system_state(system, values, time).map_or(0.0, |state| state.effect_a),
                effect_b: system_state(system, values, time).map_or(1.0, |state| state.effect_b),
                ..Default::default()
            };
            let (offset, turned) = emitter_frame(emitter, origin, axes, &base);
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
                let aspect = look.aspect.scalar(&state);
                let basis = billboard(look.billboard, place, velocity, view);
                let manual = emitter.rotation.scalar(&state).rem_euclid(1.0);
                let (sin, cos) = (std::f32::consts::TAU * (particle.rotation + manual)).sin_cos();
                let tint = emitter.tint.color(&state);
                let color = look.color.color(&state);
                let intensity = look.intensity.scalar(&state);
                let mut alpha = (emitter.alpha.scalar(&state) * look.alpha.scalar(&state)).clamp(0.0, 1.0);
                if look.intensity_fades {
                    alpha *= intensity;
                }
                let eye: [f32; 3] = std::array::from_fn(|axis| place[axis] - view.eye[axis]);
                alpha *= (system.near_range * (dot(view.forward, eye) - system.near_cutoff)).clamp(0.0, 1.0);
                let light = if look.blend == H3Blend::Multiply { 1.0 } else { H3_EXPOSURE };
                let rgb: [f32; 3] = std::array::from_fn(|axis| {
                    (tint[axis] * color[axis]).clamp(0.0, 1.0) * intensity * light
                });
                let count = look.frames.len();
                let frame_at = particle.frame + look.frame.scalar(&state).rem_euclid(1.0);
                let frame = frame_variant(frame_at, count, look.one_shot, look.backwards && particle.flip[2]);
                let first = look.frames[frame.floor().rem_euclid(count as f32) as usize % count];
                let second = look.frames[(frame + 1.0).floor().rem_euclid(count as f32) as usize % count];
                let blend = if look.frame_blend { frame.fract() } else { 0.0 };
                let black = emitter.black_point.scalar(&state).clamp(0.0, 0.9999);
                let palette = look.palette.scalar(&state).clamp(0.0, 0.9999);
                let [x, y, width, height] = look.corner;
                let corner = |shift: [f32; 2]| {
                    let planar = [(shift[0] * width + x) * aspect, shift[1] * height + y];
                    let turned_x = planar[0] * cos - planar[1] * sin;
                    let turned_y = planar[0] * sin + planar[1] * cos;
                    let spot: [f32; 3] = std::array::from_fn(|axis| {
                        place[axis] + (basis[0][axis] * turned_x + basis[1][axis] * turned_y) * size
                    });
                    let mut uv = [shift[0], 1.0 - shift[1]];
                    if look.flip_u && particle.flip[0] {
                        uv[0] = 1.0 - uv[0];
                    }
                    if look.flip_v && particle.flip[1] {
                        uv[1] = 1.0 - uv[1];
                    }
                    H3ParticleVertex {
                        place: spot,
                        sprite: sprite_uv(first, uv),
                        next_sprite: sprite_uv(second, uv),
                        billboard: uv,
                        color: [rgb[0], rgb[1], rgb[2], alpha],
                        extra: [blend, palette, black],
                    }
                };
                let quad = [corner([0.0, 0.0]), corner([1.0, 0.0]), corner([1.0, 1.0]), corner([0.0, 1.0])];
                vertices.extend([quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
            }
        }
        vertices
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
    ([place(&rows[0]), place(&rows[1]), place(&rows[2])], system.axes.map(turn))
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
