//! Reach's cheap particles (`cheap_particle_emitter`, `cpem`): one pool of
//! 8192 the card spawns, moves and draws every frame, from emitters that an
//! effect's parts submit while their event runs. Here they run on the
//! processor a preview frame at a time, and draw as one batch after every
//! other transparent. They collide only with what is on screen, which the
//! preview leaves out (see [`ReachCheapField::update`]).

use std::collections::HashMap;
use std::sync::Arc;

use blam_tags::{TagFieldData, TagFile, TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::ce::attachments::elements;
use super::ce::lights::{CeFlareView, posed_on};
use super::h2_functions::H2FunctionValues;
use super::h2_passes::tag_function_at;
use super::h3_effects::{
    CHEAP_PARTICLE_EMITTER, H3EffectOnsets, H3EffectTimes, adopt_event_function, for_each_attached_part, part_frame,
    read_part_placement,
};
use super::h3_particles::{PRE_ROLL, Random, particle_game_time};
use super::reach_lights::object_value;
use super::*;
use crate::core::source::TagSource;

/// `s_cheap_particle_system::k_max_particles`: the ring every emitter writes.
const MOST_PARTICLES: usize = 0x2000;
/// `add_emitter`'s room (the 360's 0x82DB1D68, MCC 0x1403E7E10): past it,
/// "too many cheap particle emitters".
const MOST_EMITTERS: usize = 200;
/// `shader_setup_global_state` (the 360's 0x82DB2368): the shaders'
/// `delta_time.x` is the frame's, at most this.
const MOST_SHADER_STEP: f32 = 0.5;
/// `s_emitter::spawn`'s string id 440: the spawn rate reads the emitter's
/// scale.
const ALPHA: &str = "alpha";
/// What `effects_submit_cheap_attachments` (the 360's 0x82EA9570) passes for
/// an effect with no lightprobe; the preview has none [inferred: the game's
/// sample is a level's lightmap].
const NO_PROBE_INTENSITY: f32 = 1.0;
/// `postprocess`'s textures for an emitter that names none.
const DEFAULT_POSITION: &str = "shaders\\default_bitmaps\\bitmaps\\random_in_sphere";
const DEFAULT_VELOCITY: &str = "shaders\\default_bitmaps\\bitmaps\\random_in_sphere_center_biased";

/// `cheap_particle_emitter_flags`, by option name.
const CORRELATE: &str = "correlate position and velocity";
const POSITION_LOCAL: &str = "position texture in local space";
const VELOCITY_LOCAL: &str = "velocity texture in local space";
const NORMALIZE: &str = "normalize velocity before scaling";
const ROTATE_UP: &str = "randomly rotate emitter about up vector";

/// The type texture's rows (`cheap_particles_common.hlsl`'s `TYPE_DATA_*`).
const PHYSICS: usize = 0;
const COLLISION: usize = 1;
const COLOR0: usize = 2;
const FADE: usize = 3;
const RENDER: usize = 4;

/// `cheap_particles_initialize.hlsl`: an empty slot, `(0, 0, 0, 2)` and
/// `(0, 1, 0, 0)`, dead by its age.
const EMPTY: CheapParticle =
    CheapParticle { position: [0.0; 3], age: 2.0, velocity: [0.0, 1.0, 0.0], delta_age: 0.0, parameters: [0; 4] };

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn minus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt().max(1e-6);
    a.map(|axis| axis / length)
}

/// HLSL's `frac`.
fn frac(x: f32) -> f32 {
    x - x.floor()
}

/// HLSL's `sign`: 0 at 0, which Rust's `signum` is not.
fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// A map the cheap particle shaders fetch texels of: -1 to 1 for a signed
/// format (`q8w8v8u8`), 0 to 1 otherwise, in the decoder's r, g, b, a
/// [inferred: the standard D3D swizzle].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheapTexels {
    width: usize,
    height: usize,
    texels: Vec<[f32; 4]>,
}

impl CheapTexels {
    /// The top level of a bitmap's first image.
    fn read(source: &TagSource, path: &str) -> Option<Self> {
        let tag = load_referenced_tag_from_source(source, path, "bitmap", b"bitm").ok()?;
        let bitmap = blam_tags::bitmap::Bitmap::new(&tag).ok()?;
        let image = bitmap.image(0)?;
        let format = image.format().ok()?;
        let (width, height) = (image.width(), image.height());
        let top = image.pixel_bytes().ok()?.get(..format.level_bytes(width, height) as usize)?;
        let rgba = blam_tags::bitmap::decode::decode_to_rgba8(format, width, height, top, bitmap.p8_palette()).ok()?;
        Some(Self::of_bytes(width as usize, height as usize, &rgba, format.is_signed()))
    }

    /// From the decoder's bytes, which bias a signed byte by 128.
    fn of_bytes(width: usize, height: usize, rgba: &[u8], signed: bool) -> Self {
        let value = |byte: u8| if signed { ((f32::from(byte) - 128.0) / 127.0).max(-1.0) } else { f32::from(byte) / 255.0 };
        let texels = rgba.chunks_exact(4).map(|texel| std::array::from_fn(|channel| value(texel[channel]))).collect();
        Self { width, height, texels }
    }

    /// `generate_random`: a point sample at `index · transform.xy +
    /// transform.zw`, wrapped (sampler address mode 0).
    fn generate(&self, index: f32, transform: [f32; 4]) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [0.0; 4];
        }
        let (u, v) = (index * transform[0] + transform[2], index * transform[1] + transform[3]);
        let column = ((u * self.width as f32).floor() as i64).rem_euclid(self.width as i64) as usize;
        let row = ((v * self.height as f32).floor() as i64).rem_euclid(self.height as i64) as usize;
        self.texels.get(row * self.width + column).copied().unwrap_or([0.0; 4])
    }
}

/// `globals\global_render_texture`, as the array the pixel shader samples:
/// each level's layers one after another, RGBA.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheapRenderTexture {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) layers: usize,
    pub(crate) levels: Vec<Vec<u8>>,
}

impl CheapRenderTexture {
    fn read(source: &TagSource, path: &str) -> Option<Self> {
        let tag = load_referenced_tag_from_source(source, path, "bitmap", b"bitm").ok()?;
        let bitmap = blam_tags::bitmap::Bitmap::new(&tag).ok()?;
        let image = bitmap.image(0).filter(|image| image.is_array())?;
        let format = image.format().ok()?;
        let (width, height, layers) = (image.width(), image.height(), image.depth());
        let levels = (0..image.mipmap_levels())
            .map(|level| {
                let (across, down) = ((width >> level).max(1), (height >> level).max(1));
                let mut rgba = Vec::new();
                for layer in 0..layers {
                    let bytes = super::materials::array_layer_bytes(&image, layer, level)?;
                    let decoded = blam_tags::bitmap::decode::decode_to_rgba8(format, across, down, bytes, bitmap.p8_palette());
                    rgba.extend(decoded.ok()?);
                }
                Some(rgba)
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { width: width as usize, height: height as usize, layers: layers as usize, levels })
    }
}

/// A `cheap_particle_type_library` type, as its tag has it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct CheapTypeTag {
    drag: f32,
    gravity: f32,
    turbulence: f32,
    depth_range: f32,
    elasticity: f32,
    death: f32,
    /// `change type`, none below 0.
    change: i64,
    /// `orientation` `velocity`, else `screen facing`.
    velocity_facing: bool,
    /// `color0`'s red, green, blue and alpha.
    color: [f32; 4],
    intensity: f32,
    fade_start: f32,
    /// `size`'s upper bound; Reach never reads the lower.
    size: f32,
    /// Halo 4's: `size`'s lower bound, and the `turbulence type` it takes.
    size_lower: f32,
    turbulence_type: i64,
    stretch: f32,
    texture: i64,
    y_scale: f32,
}

impl CheapTypeTag {
    fn read(element: TagStruct<'_>) -> Self {
        let real = |name: &str| element.read_real(name).unwrap_or(0.0);
        let color = match element.field("color0").and_then(|field| field.value()) {
            Some(TagFieldData::RealArgbColor(color)) => [color.red, color.green, color.blue, color.alpha],
            _ => [0.0; 4],
        };
        let index = |name: &str| element.read_int_any(name).map_or(-1, |index| index as i64);
        Self {
            drag: real("drag"),
            gravity: real("gravity"),
            turbulence: real("turbulence"),
            depth_range: real("depth_range"),
            elasticity: real("elasticity"),
            death: real("death"),
            change: index("change type"),
            velocity_facing: element.read_enum_name("orientation").as_deref() == Some("velocity"),
            color,
            intensity: real("intensity0"),
            fade_start: real("fade start"),
            size: element.read_real_bounds("size").upper,
            size_lower: element.read_real_bounds("size").lower,
            turbulence_type: index("turbulence type"),
            stretch: real("motion blur stretch"),
            texture: index("texture"),
            y_scale: real("texture y scale"),
        }
    }

    /// `type-library-compile` (tool 0x1403C60C0): the type's five rows of
    /// the type texture, as type `own` of a library with `textures` maps.
    /// No change of type is a change to itself, a texture out of range is
    /// the first, and a screen-facing type's y scale is negated.
    ///
    /// Halo 4's (`halo4`) are laid out as its shaders read them
    /// (`cheap_particles_update.fx`, `cheap_particles_render.fx`): the
    /// turbulence type's place in an array of `k_maxTypes` in `PHYSICS.w`,
    /// the depth range in `COLLISION.x`, and `size`'s bounds and the fade in
    /// row 3. No Halo 4 tool was at hand; the kit's shipped type texture
    /// agrees (`kit_halo_4s_type_table_matches_the_shipped_type_texture`).
    fn compiled(&self, own: usize, textures: usize, halo4: bool) -> CheapType {
        let texture = usize::try_from(self.texture).ok().filter(|texture| *texture < textures).unwrap_or(0);
        let change = usize::try_from(self.change).unwrap_or(own) as f32;
        let fade = (1.0 - self.fade_start).clamp(1e-4, 1.0).recip();
        let facing = if self.velocity_facing { 1.0 } else { -1.0 };
        let [red, green, blue, alpha] = self.color;
        let intensity = self.intensity;
        let mut rows = [[0.0; 4]; 5];
        rows[COLOR0] = [red * intensity, green * intensity, blue * intensity, alpha];
        let place = (texture as f32 + 0.5) / textures.max(1) as f32;
        rows[RENDER] = [place, self.size, self.stretch, facing * self.y_scale];
        if !halo4 {
            rows[PHYSICS] = [self.drag, self.gravity, self.turbulence, self.depth_range];
            rows[COLLISION] = [0.0, self.elasticity, self.death, change];
            rows[FADE] = [0.0, 0.0, 0.0, fade];
            return CheapType { rows, texture };
        }
        let turbulence = usize::try_from(self.turbulence_type).unwrap_or(0).min(MOST_TURBULENCE_TYPES - 1);
        let array = (turbulence as f32 + 0.5) / MOST_TURBULENCE_TYPES as f32;
        rows[PHYSICS] = [self.drag, self.gravity, self.turbulence, array];
        rows[COLLISION] = [self.depth_range, self.elasticity, self.death, change];
        rows[FADE] = [self.size_lower, self.size, 0.0, fade];
        CheapType { rows, texture }
    }
}

/// A type as the shaders read it: its rows of `global_type_texture`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CheapType {
    rows: [[f32; 4]; 5],
    /// The render texture's layer. The shaders take `RENDER.x · count − 0.5`
    /// through `vs_array_texture_parameters`, whose values were not read
    /// [inferred: the layer `RENDER.x` was baked from].
    texture: usize,
}

/// `CheapParticleTurbulenceType::k_maxTypes`, the layers of Halo 4's
/// turbulence array.
const MOST_TURBULENCE_TYPES: usize = 8;

/// Halo 4's `cheap_particles_update.fx`: the turbulence type's array layer,
/// `PHYSICS.w · layers − 0.5`, and its transform, `(int)(4 · PHYSICS.w)`.
/// The two agree only for type 0; past it the transform is another type's
/// (type 1 and 3 take 0 and 1), as the game has it.
fn turbulence_of(place: f32) -> (usize, usize) {
    let layer = (place * MOST_TURBULENCE_TYPES as f32 - 0.5).round().max(0.0) as usize;
    (layer, (4.0 * place).max(0.0) as usize)
}

/// A kind of turbulence: the map its push is read from, and `turb du dt`,
/// `turb dv dt`, `turb du dp` and `turb dv dp`.
#[derive(Debug)]
struct CheapTurbulence {
    texels: CheapTexels,
    rates: [f32; 4],
    /// Halo 4's push spans -1 to 1 of what the map holds
    /// (`GenerateRandomInRange(…, -1, 1)`); Reach's is the map's own.
    centered: bool,
}

impl CheapTurbulence {
    fn read(source: &TagSource, holder: TagStruct<'_>, centered: bool) -> Option<Self> {
        let path = holder.read_tag_ref_path("turbulence").filter(|path| !path.is_empty())?;
        Some(Self {
            texels: CheapTexels::read(source, &path)?,
            rates: ["turb du dt", "turb dv dt", "turb du dp", "turb dv dp"].map(|name| holder.read_real(name).unwrap_or(0.0)),
            centered,
        })
    }

    /// The push at ring slot `index`, by `transform`.
    fn push(&self, index: usize, transform: [f32; 4]) -> [f32; 4] {
        let sample = self.texels.generate(index as f32, transform);
        if self.centered { sample.map(|value| value * 2.0 - 1.0) } else { sample }
    }
}

/// A `cheap_particle_type_library`, with the maps the shaders bind from it.
/// The game binds the one `rasterizer_globals` names (`shader_setup_type_library`,
/// the 360's 0x82DB2B50); every kit emitter names the same
/// `globals\global.cheap_particle_type_library`, so the emitter's stands in.
#[derive(Debug)]
pub(crate) struct CheapLibrary {
    names: Vec<String>,
    types: Vec<CheapType>,
    random: CheapTexels,
    /// Reach's one turbulence; Halo 4's `turbulence types`.
    turbulence: Vec<CheapTurbulence>,
    render: Option<CheapRenderTexture>,
    /// Halo 4's: per-type turbulence, its own rows, sizes and exposure.
    halo4: bool,
}

impl CheapLibrary {
    fn read(source: &TagSource, path: &str) -> Option<Self> {
        let tag = load_referenced_tag_from_source(source, path, "cheap_particle_type_library", b"cptl").ok()?;
        let root = tag.root();
        let textures = elements(root, "textures").len();
        let types = elements(root, "types");
        let map = |name: &str| root.read_tag_ref_path(name).filter(|path| !path.is_empty());
        let halo4 = source.game() == Some(GameId::Halo4);
        // Halo 4 binds `turbulenceTexture`, an array the tool builds from
        // these; the kit's is stale (8 layers for 4 types, none matching),
        // so each type's own map stands in [inferred].
        let turbulence: Vec<CheapTurbulence> = if halo4 {
            let kinds = elements(root, "turbulence types");
            kinds.iter().map(|kind| CheapTurbulence::read(source, *kind, true)).collect::<Option<_>>()?
        } else {
            vec![CheapTurbulence::read(source, root, false)?]
        };
        Some(Self {
            names: types.iter().map(|element| element.read_string_id("name").unwrap_or_default()).collect(),
            types: types
                .iter()
                .enumerate()
                .map(|(at, element)| CheapTypeTag::read(*element).compiled(at, textures, halo4))
                .collect(),
            random: CheapTexels::read(source, &map("random")?)?,
            turbulence,
            render: map("render_texture").and_then(|path| CheapRenderTexture::read(source, &path)),
            halo4,
        })
    }

    /// Row `row` of type `kind`. The texture is 256 types wide, and a type
    /// past the library's reads nothing.
    fn row(&self, kind: usize, row: usize) -> [f32; 4] {
        self.types.get(kind).map_or([0.0; 4], |found| found.rows[row])
    }

    pub(crate) fn render_texture(&self) -> Option<&CheapRenderTexture> {
        self.render.as_ref()
    }

    /// Whether it is Halo 4's, which lights by its own exposures.
    pub(crate) fn halo4(&self) -> bool {
        self.halo4
    }
}

/// A `cheap_particle_emitter` as its tag has it, before `postprocess`.
#[derive(Debug, Clone)]
struct CheapEmitterTag {
    version: i64,
    spawn_rate: f32,
    function: Option<Arc<TagFunction>>,
    /// `distance fade start` and `end`.
    fade: [f32; 2],
    types: [String; 4],
    weights: [f32; 4],
    lifetime: [f32; 2],
    position_scale: f32,
    position_flatten: f32,
    directionality: [f32; 2],
    speed: [f32; 2],
    subframe: [f32; 2],
    rotation_randomness: f32,
    scale_modifier: f32,
    /// `lighting contrast`, `offset`, `min` and `max`.
    lighting: [f32; 4],
}

impl CheapEmitterTag {
    fn read(root: TagStruct<'_>) -> Self {
        let real = |name: &str| root.read_real(name).unwrap_or(0.0);
        let bounds = |name: &str| {
            let bounds = root.read_real_bounds(name);
            [bounds.lower, bounds.upper]
        };
        let function = root.descend("spawnrate").and_then(|holder| tag_function_at(&holder, "Mapping")).map(Arc::new);
        Self {
            version: root.read_int_any("version").map_or(0, |version| version as i64),
            spawn_rate: real("spawn rate"),
            function,
            fade: [real("distance fade start"), real("distance fade end")],
            types: std::array::from_fn(|at| root.read_string_id(&format!("type {at}")).unwrap_or_default()),
            weights: std::array::from_fn(|at| real(&format!("weight {at}"))),
            lifetime: bounds("lifetime"),
            position_scale: real("position scale"),
            position_flatten: real("position flatten"),
            directionality: bounds("directionality"),
            speed: bounds("speed scale"),
            subframe: bounds("subframe offset"),
            rotation_randomness: real("rotation randomness"),
            scale_modifier: real("particle scale modifier"),
            lighting: ["lighting contrast", "lighting offset", "lighting min", "lighting max"].map(real),
        }
    }

    /// `postprocess`'s version upgrade (the 360's 0x832B4430, tool
    /// 0x140DDB990): version 0 fades from 5 to 20 and spawns at a constant
    /// `spawn rate`; up to 1 takes lighting contrast 1, offset 0 and −11 to
    /// 11; up to 2 no rotation randomness and a scale modifier of 1.
    fn upgraded(mut self) -> Self {
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
    fn spawn_parameters(&self, names: &[String]) -> [[f32; 4]; 5] {
        let modifier = self.scale_modifier.clamp(0.0, 2.0) * 4.0 * 127.0 / 2.0;
        let inverse = |lifetime: f32| (if lifetime <= 0.0001 { 0.0001 } else { lifetime }).recip();
        let [directionality, speed, subframe] = [self.directionality, self.speed, self.subframe];
        let (shortest, longest) = (inverse(self.lifetime[0]), inverse(self.lifetime[1]));
        let indices = self.types.each_ref().map(|name| names.iter().position(|known| known == name));
        let (thresholds, steps) = type_thresholds(indices, self.weights);
        [
            [self.position_scale, self.position_flatten, self.rotation_randomness.clamp(0.0, 1.0), modifier],
            [directionality[0], directionality[1] - directionality[0], speed[0], speed[1] - speed[0]],
            [subframe[0], subframe[1] - subframe[0], shortest, longest - shortest],
            thresholds,
            steps,
        ]
    }
}

/// What tool's `c_function_definition_editor` makes of a constant: function
/// type 1, clamped to `value` at both ends.
fn constant_function(value: f32) -> Option<TagFunction> {
    use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};
    let mut function = H2Function::new(FunctionType::Constant);
    function.set_clamp_range(value, value).ok()?;
    TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).ok()
}

/// `postprocess`'s distance fade: the scale and bias `calculate_LOD_scale`
/// (the 360's 0x832B4E68) reads, whole at the nearer bound and nothing at
/// the farther, whichever field holds which.
fn distance_fade([start, end]: [f32; 2]) -> [f32; 2] {
    let range = (end - start).abs();
    let scale = -1.0 / if range <= 0.001 { 0.001 } else { range };
    let farther = if start <= end { end } else { start };
    [scale, -farther * scale]
}

fn lod_scale([scale, bias]: [f32; 2], distance: f32) -> f32 {
    (distance * scale + bias).clamp(0.0, 1.0)
}

/// `add_emitter`'s distance from the camera (MCC 0x1403E7E66..0x1403E7EB6):
/// a Manhattan one.
fn manhattan(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|axis| (a[axis] - b[axis]).abs()).sum()
}

/// `postprocess`'s `spawn_params3` and `4`: each found type's cumulative
/// threshold and its index step from the last found. The weight a found
/// type adds is the one before it, found or not; with no weight at all,
/// type 0 takes all of it.
fn type_thresholds(mut indices: [Option<usize>; 4], mut weights: [f32; 4]) -> ([f32; 4], [f32; 4]) {
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
fn packed_lighting([contrast, offset, low, high]: [f32; 4], intensity: f32) -> f32 {
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
fn pack_sbyte4(values: [f32; 4]) -> [i8; 4] {
    values.map(|value| value.clamp(-128.0, 127.0) as i32 as i8)
}

/// `cheap_particles_render.hlsl`'s illumination of a packed lighting.
fn illumination(lighting: i8) -> f32 {
    (f32::from(lighting) * (22.0 / 256.0) - 11.0).exp2()
}

/// `s_emitter::spawn`'s count: by the frame and the scale, rounded down
/// after `roll`, `real_random_range(0, 1 − 1e-4)`.
fn spawn_count(rate: f32, dt: f32, scale: f32, roll: f32) -> usize {
    (rate * dt * scale + roll).floor().max(0.0) as usize
}

/// `generate_random_texture_transform` (the 360's 0x82DB3190, MCC
/// sub_1403E8810): a scale in ±[1, 2)/64 for each of u and v, and an
/// offset, from the generator's draws in the game's order.
fn texture_transform(random: &mut Random) -> [f32; 4] {
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
fn spawn_transforms(random: &mut Random, correlate: bool) -> [[f32; 4]; 3] {
    let random_transform = texture_transform(random);
    let position = texture_transform(random);
    let velocity = if correlate { position } else { texture_transform(random) };
    [random_transform, position, velocity]
}

/// `s_emitter::spawn`'s input to the spawn rate: the game time, or with an
/// input variable the object's function of that name, `alpha` (440) the
/// emitter's scale.
fn spawn_input(variable: &str, values: Option<&H2FunctionValues>, time: f32, scale: f32) -> f32 {
    match variable {
        "" => particle_game_time(true, time),
        ALPHA => scale,
        name => object_value(values, name),
    }
}

/// `add_emitter`'s `randomly rotate emitter about up vector`: a new up of
/// `forward` across a fresh random direction, every frame. Sic: not
/// normalized after, in either build.
fn turned_up(forward: [f32; 3], random: &mut Random) -> [f32; 3] {
    let turn = unit(std::array::from_fn(|_| random.range(-1.0, 1.0)));
    cross(forward, turn)
}

/// One slot of the ring: `position_and_age`, `velocity_and_delta_age` and
/// the packed `particle_parameters` (type, lighting, rotation and scale).
/// MCC keeps velocity and the age's rate as halves, which changes little.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CheapParticle {
    position: [f32; 3],
    /// From 0 to 1 over its life; below 0 at rest, the sign the flag.
    age: f32,
    velocity: [f32; 3],
    delta_age: f32,
    parameters: [i8; 4],
}

/// A `cheap_particle_emitter` as `postprocess` leaves it.
#[derive(Debug)]
pub(crate) struct CheapEmitterDefinition {
    correlate: bool,
    position_local: bool,
    velocity_local: bool,
    normalize: bool,
    rotate_up: bool,
    spawn_rate: Arc<TagFunction>,
    /// `spawnrate`'s `Input Variable` and `Range Variable`.
    input: String,
    range: String,
    /// `distance fade start` and `end`, as scale and bias.
    lod: [f32; 2],
    parameters: [[f32; 4]; 5],
    lighting: [f32; 4],
    position: Arc<CheapTexels>,
    velocity: Arc<CheapTexels>,
    library: Arc<CheapLibrary>,
}

impl CheapEmitterDefinition {
    fn read(
        source: &TagSource,
        path: &str,
        libraries: &mut HashMap<String, Option<Arc<CheapLibrary>>>,
        maps: &mut HashMap<String, Option<Arc<CheapTexels>>>,
    ) -> Option<Self> {
        let tag = load_referenced_tag_from_source(source, path, "cheap_particle_emitter", b"cpem").ok()?;
        let root = tag.root();
        let emitter = CheapEmitterTag::read(root).upgraded();
        let library_path = root.read_tag_ref_path("global type library").filter(|path| !path.is_empty())?;
        let library = libraries
            .entry(library_path.to_ascii_lowercase())
            .or_insert_with(|| CheapLibrary::read(source, &library_path).map(Arc::new))
            .clone()?;
        let mut map = |field: &str, default: &str| {
            let path = root.read_tag_ref_path(field).filter(|path| !path.is_empty()).unwrap_or_else(|| default.to_owned());
            maps.entry(path.to_ascii_lowercase()).or_insert_with(|| CheapTexels::read(source, &path).map(Arc::new)).clone()
        };
        let (position, velocity) = (map("position texture", DEFAULT_POSITION)?, map("velocity texture", DEFAULT_VELOCITY)?);
        let flags = root.read_flag_names("flags").unwrap_or_default();
        let flagged = |wanted: &str| flags.iter().any(|(_, name)| name == wanted);
        let spawnrate = root.descend("spawnrate");
        let variable = |name: &str| spawnrate.and_then(|holder| holder.read_string_id(name)).unwrap_or_default();
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
    fn spawn_particle(&self, at: &CheapSubmission, transforms: &[[f32; 4]; 3], lighting: f32, index: usize, dt: f32) -> CheapParticle {
        let [placement, speeds, times, thresholds, steps] = self.parameters;
        let library = &self.library;
        let (up, forward) = (at.up, at.forward);
        let left = cross(up, forward);
        // `mul(random, transform)`: rows up, left, forward and nothing.
        let local = |random: [f32; 4]| -> [f32; 3] {
            std::array::from_fn(|axis| random[0] * up[axis] + random[1] * left[axis] + random[2] * forward[axis])
        };
        let index_f = index as f32;
        let mut position = self.position.generate(index_f, transforms[1]);
        if self.position_local {
            let [x, y, z] = local(position);
            position = [x, y, z, 0.0];
        }
        let position: [f32; 3] = std::array::from_fn(|axis| position[axis] * placement[0]);
        let along = dot(position, forward) * placement[1];
        let mut position: [f32; 3] = std::array::from_fn(|axis| position[axis] - forward[axis] * along + at.position[axis]);
        let random = library.random.generate(index_f, transforms[0]);
        let directionality = speeds[0] + speeds[1] * random[0];
        let speed = speeds[2] + speeds[3] * random[1];
        let mut velocity = self.velocity.generate(index_f, transforms[2]);
        if self.velocity_local {
            let [x, y, z] = local(velocity);
            velocity = [x, y, z, 0.0];
        }
        let mut direction: [f32; 3] =
            std::array::from_fn(|axis| velocity[axis] + (forward[axis] - velocity[axis]) * directionality);
        if self.normalize {
            let length = dot(direction, direction).sqrt();
            direction = direction.map(|axis| axis / length);
        }
        let mut velocity = direction.map(|axis| axis * speed);
        let delta_age = random[2] * times[3] + times[2];
        let kind: f32 = (0..4).filter(|&at| random[3] >= thresholds[at]).map(|at| steps[at]).sum();
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
        CheapParticle { position, age, velocity, delta_age, parameters }
    }
}

/// Where an emitter is submitted this frame, as `add_emitter` keeps it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CheapSubmission {
    position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
    /// The effect's scale by the distance fade.
    scale: f32,
}

/// A cheap particle emitter at a marker of the object an effect is attached
/// to.
#[derive(Debug, Clone)]
pub(crate) struct ReachCheapEmitterAt {
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
    only_under: Option<(String, String)>,
    scale_by: String,
    secondary_by: String,
    /// `A scales values` and `B scales values` take `type-specific scale`.
    scaled: bool,
    b_scaled: bool,
    times: Arc<H3EffectTimes>,
    event: usize,
    definition: Arc<CheapEmitterDefinition>,
}

/// The cheap particle emitters an object's attached effects run, one at each
/// marker of a part's location.
pub(super) fn read_reach_cheap_emitters(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<ReachCheapEmitterAt> {
    let mut emitters = Vec::new();
    let (mut definitions, mut libraries, mut maps) = (HashMap::new(), HashMap::new(), HashMap::new());
    for_each_attached_part(source, object, |part| {
        if part.group != CHEAP_PARTICLE_EMITTER {
            return;
        }
        let definition = definitions
            .entry(part.path.to_ascii_lowercase())
            .or_insert_with(|| CheapEmitterDefinition::read(source, &part.path, &mut libraries, &mut maps).map(Arc::new))
            .clone();
        let Some(definition) = definition else {
            return;
        };
        let (offset, yaw, pitch) = read_part_placement(part.part);
        for marker in &part.effect.locations(markers, &part.marker, &frame) {
            let (place, node) = frame(marker);
            let (place, node) = part.placed(place, node);
            let place = part_frame(&place, offset, yaw, pitch);
            emitters.push(ReachCheapEmitterAt {
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

impl ReachCheapEmitterAt {
    pub(crate) fn only_under(&self) -> Option<&(String, String)> {
        self.only_under.as_ref()
    }

    /// Follow the regions `merge_attached` renames for what `name` brings.
    pub(crate) fn on_attached(mut self, name: &str) -> Self {
        adopt_event_function(name, &mut self.scale_by, &mut self.object);
        if let Some((region, _)) = &mut self.only_under {
            *region = format!("{name}: {region}");
        }
        self
    }

    /// The functions it reads off its object: its effect's scales and its
    /// spawn rate's inputs.
    pub(crate) fn scale_names(&self) -> [&str; 4] {
        [&self.scale_by, &self.secondary_by, &self.definition.input, &self.definition.range]
    }

    pub(crate) fn library(&self) -> &Arc<CheapLibrary> {
        &self.definition.library
    }

    /// `effects_submit_cheap_attachments` and `add_emitter` at `time`: none
    /// unless its event is in its duration (one of no length never is) and
    /// its effect is on, at `effect_scale` of `type-specific scale` by the
    /// distance fade from `eye`, and none at no scale. Its up is turned in
    /// `add_emitter` once it has room.
    fn submission(
        &self,
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        time: f32,
        onset: f32,
    ) -> Option<CheapSubmission> {
        self.times.age(self.event, time - onset)?;
        if !self.times.duration(self.event).is_some_and(|duration| duration > 0.0) {
            return None;
        }
        // `attachments_update` runs the effect while its function is on.
        let effect_a = object_value(values, &self.scale_by);
        if effect_a <= 0.0 {
            return None;
        }
        let effect_b = object_value(values, &self.secondary_by);
        let scale = (if self.scaled { effect_a } else { 1.0 }) * (if self.b_scaled { effect_b } else { 1.0 });
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        let (_, up) = posed_on(self.node, self.position, self.up, bones);
        let scale = scale * lod_scale(self.definition.lod, manhattan(position, eye));
        (scale > 0.0).then_some(CheapSubmission { position, forward, up, scale })
    }
}

/// The pool every emitter writes, as the card holds it.
#[derive(Debug)]
pub(crate) struct ReachCheapField {
    time: Option<f32>,
    particles: Vec<CheapParticle>,
    /// `spawn_offset`.
    cursor: usize,
    /// One generator per emitter, seeded so a held pose repeats; the game's
    /// is the local seed [inferred: preview convention].
    random: Vec<Random>,
    /// Each kind of turbulence's `(u, v)` phase.
    phases: Vec<[f32; 2]>,
}

impl Default for ReachCheapField {
    fn default() -> Self {
        Self::fresh(0)
    }
}

/// A cheap particle's quad corner, in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ReachCheapVertex {
    pub(crate) place: [f32; 3],
    /// u, v and the render texture's layer.
    pub(crate) texcoord: [f32; 3],
    pub(crate) color: [f32; 4],
}

impl ReachCheapField {
    fn fresh(emitters: usize) -> Self {
        Self {
            time: None,
            particles: vec![EMPTY; MOST_PARTICLES],
            cursor: 0,
            random: (0..emitters).map(|at| Random((at as u32 + 1).wrapping_mul(977))).collect(),
            phases: Vec::new(),
        }
    }

    /// Run to `time` a frame at a time, as `c_player_view::render` (the 360's
    /// 0x82729774) does: the emitters alive submitted (`shown` leaves out
    /// those under hidden regions), then `s_cheap_particle_system::update`'s
    /// `update_particles` and `spawn_particles`. A new field starts a
    /// pre-roll short, so a held pose has particles in flight.
    pub(crate) fn advance(
        &mut self,
        emitters: &[ReachCheapEmitterAt],
        shown: &[bool],
        values: impl Fn(u8) -> Option<H2FunctionValues>,
        onsets: &mut H3EffectOnsets,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        time: f32,
    ) {
        if self.random.len() != emitters.len() || self.time.is_none_or(|last| time < last) {
            *self = Self::fresh(emitters.len());
            self.time = Some(time - PRE_ROLL);
        }
        let (Some(now), Some(first)) = (self.time, emitters.first()) else {
            return;
        };
        let library = Arc::clone(first.library());
        let mut now = resume_point(now, time, PRE_ROLL);
        let scopes: Vec<Option<H2FunctionValues>> = emitters.iter().map(|at| values(at.object)).collect();
        // By the frame's clock, which the steps trail.
        let onset_of: Vec<f32> = emitters
            .iter()
            .zip(&scopes)
            .map(|(at, scope)| onsets.of(at.object, &at.scale_by, scope.as_ref(), time))
            .collect();
        for dt in frame_steps(now, time) {
            now += dt;
            let mut submitted = Vec::new();
            for (index, at) in emitters.iter().enumerate() {
                if !shown.get(index).copied().unwrap_or(true) {
                    continue;
                }
                let Some(mut submission) = at.submission(scopes[index].as_ref(), bones, eye, now, onset_of[index]) else {
                    continue;
                };
                if submitted.len() >= MOST_EMITTERS {
                    continue;
                }
                if at.definition.rotate_up {
                    submission.up = turned_up(submission.forward, &mut self.random[index]);
                }
                submitted.push((index, submission));
            }
            self.update(&library, dt);
            for (index, submission) in submitted {
                self.spawn(&emitters[index], &submission, scopes[index].as_ref(), now, dt, index);
            }
        }
        self.time = Some(time);
    }

    /// `update_particles` (Reach: the 360's 0x82DB3790; Halo 4: the 360's
    /// 0x82F2BCF8, MCC 0x140384020) and the update shader: each kind of
    /// turbulence's phase moves by the frame, and every live particle ages
    /// and, unless at rest, moves by drag, gravity and its type's turbulence
    /// (which moves its place, not its velocity), at the shaders' step.
    fn update(&mut self, library: &CheapLibrary, dt: f32) {
        self.phases.resize(library.turbulence.len(), [0.0; 2]);
        let transforms: Vec<[f32; 4]> = library
            .turbulence
            .iter()
            .zip(&mut self.phases)
            .map(|(turbulence, phase)| {
                let [du_dt, dv_dt, du_dp, dv_dp] = turbulence.rates;
                *phase = [frac(phase[0] + du_dt * dt), frac(phase[1] + dv_dt * dt)];
                [du_dp, dv_dp, phase[0], phase[1]]
            })
            .collect();
        let dt = dt.min(MOST_SHADER_STEP);
        for (index, particle) in self.particles.iter_mut().enumerate() {
            if particle.age.abs() < 1.0 {
                update_particle(particle, index, library, &transforms, dt);
            }
        }
    }

    /// `s_emitter::spawn` (the 360's 0x82DB2F68) and `s_particle_buffers::spawn`
    /// (0x82DB35E8): how many this frame, from the spawn rate at the game
    /// time (or what the input names) and a fresh roll for its range, then
    /// each at the next ring slot, over whatever was there.
    fn spawn(
        &mut self,
        at: &ReachCheapEmitterAt,
        submission: &CheapSubmission,
        values: Option<&H2FunctionValues>,
        time: f32,
        dt: f32,
        index: usize,
    ) {
        let definition = &*at.definition;
        let random = &mut self.random[index];
        let input = spawn_input(&definition.input, values, time, submission.scale);
        let rolled = random.next();
        let range = if definition.range.is_empty() { rolled } else { object_value(values, &definition.range) };
        let rate = definition.spawn_rate.evaluate(input, range);
        let count = spawn_count(rate, dt, submission.scale, random.range(0.0, 1.0 - 1e-4));
        if count == 0 {
            return;
        }
        let transforms = spawn_transforms(random, definition.correlate);
        let lighting = packed_lighting(definition.lighting, NO_PROBE_INTENSITY);
        let shader_dt = dt.min(MOST_SHADER_STEP);
        for _ in 0..count {
            let slot = self.cursor;
            self.particles[slot] = definition.spawn_particle(submission, &transforms, lighting, slot, shader_dt);
            self.cursor = (slot + 1) % MOST_PARTICLES;
        }
    }

    /// `cheap_particles_render.hlsl`'s quads in ring-slot order, as two
    /// triangles each: facing along the velocity, stretched by its speed, or
    /// the screen; coloured by the type's colour, its fade and the packed
    /// lighting. There is no fog to scatter in.
    pub(crate) fn vertices(&self, library: &CheapLibrary, view: &CeFlareView) -> Vec<ReachCheapVertex> {
        let mut vertices = Vec::new();
        for (index, particle) in self.particles.iter().enumerate() {
            if particle.age.abs() > 1.0 {
                continue;
            }
            let [kind, lighting, dx, dy] = particle.parameters;
            let Some(found) = usize::try_from(kind).ok().and_then(|kind| library.types.get(kind)) else {
                continue;
            };
            let [_, size, stretch_factor, y_scale] = found.rows[RENDER];
            // Halo 4's lies between `size`'s bounds, by a cheap hash of the
            // slot (`frac(index · 0.863182394)`).
            let size = if library.halo4 {
                let [lower, upper, _, _] = found.rows[FADE];
                lower + (upper - lower) * frac(index as f32 * 0.863_182_4)
            } else {
                size
            };
            let to_particle = unit(minus(particle.position, view.eye));
            // Pulled 1 cm toward the camera.
            let place: [f32; 3] = std::array::from_fn(|axis| particle.position[axis] - to_particle[axis] * 0.01);
            let local = [f32::from(dx) / 63.5 * size, f32::from(dy) / 63.5 * size];
            let velocity = particle.velocity;
            let speed = dot(velocity, velocity).sqrt();
            let basis = if y_scale > 0.0 {
                // Unguarded in the shader: at rest the quad is NaN.
                if speed == 0.0 {
                    continue;
                }
                let stretch = (stretch_factor * speed).max(1.0) / speed;
                let along = velocity.map(|axis| axis * stretch);
                [along, unit(cross(along, to_particle))]
            } else {
                [view.right, view.up]
            };
            let fade = found.rows[FADE][3];
            let blend = (fade - (fade * particle.age).abs()).clamp(0.0, 1.0);
            let [red, green, blue, alpha] = found.rows[COLOR0];
            let lit = blend * illumination(lighting);
            let color = [red * lit, green * lit, blue * lit, (alpha * blend).clamp(0.0, 1.0)];
            // `generate_quad_point_2d`'s Direct3D 11 order, as a strip.
            let corner = |[x, y]: [f32; 2]| {
                let (across, down) = (x - 0.5, y - 0.5);
                let offset = [across * local[0] - down * local[1], across * local[1] + down * local[0]];
                ReachCheapVertex {
                    place: std::array::from_fn(|axis| place[axis] + offset[0] * basis[0][axis] + offset[1] * basis[1][axis]),
                    texcoord: [x, (y - 0.5) * y_scale + 0.5, found.texture as f32],
                    color,
                }
            };
            let corners = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]].map(corner);
            vertices.extend([0, 1, 2, 2, 1, 3].map(|at| corners[at]));
        }
        vertices
    }
}

/// `update.hlsl`'s `update_particle` for a live particle at ring slot
/// `index`, with each kind of turbulence's `(du_dp, dv_dp, phase)` and the
/// shaders' `dt`.
fn update_particle(particle: &mut CheapParticle, index: usize, library: &CheapLibrary, transforms: &[[f32; 4]], dt: f32) {
    let kind = usize::try_from(particle.parameters[0]).unwrap_or(usize::MAX);
    let [drag, gravity, strength, place] = library.row(kind, PHYSICS);
    let (layer, transform) = if library.halo4 { turbulence_of(place) } else { (0, 0) };
    let gravity = -gravity;
    particle.age += sign(particle.age) * particle.delta_age * dt;
    if particle.age >= 0.0 {
        // The library's `turbulence` on the sampler the shader calls
        // `velocity_texture`, point sampled [inferred: MCC's filter mode 2
        // was not mapped].
        let push = match (library.turbulence.get(layer), transforms.get(transform)) {
            (Some(turbulence), Some(transform)) => turbulence.push(index, *transform),
            _ => [0.0; 4],
        };
        for axis in 0..3 {
            particle.position[axis] += (particle.velocity[axis] + push[axis] * strength) * dt;
        }
        particle.position[2] += gravity * dt * dt;
        let pull = [0.0, 0.0, gravity];
        for axis in 0..3 {
            particle.velocity[axis] += (pull[axis] - particle.velocity[axis] * drag) * dt;
        }
    }
    // Here the game projects the particle into the screen and collides it
    // with the depth and normal buffers (`_surface_depth_stencil`,
    // `_surface_normal_biased`): a bounce by `elasticity`, rest, a kill on
    // leaving the surface and the type's `change type` all happen only
    // there. The preview has no such buffers to read back and leaves it out
    // (cheap particle spec §4.1–4.2, open item 1).
}

/// The cheap particle program: the quads as built, the render texture's
/// layer sampled a half level sharp, by the vertex colour, at `g_exposure.x`.
pub(super) const CHEAP_PARTICLE_VERTEX: &str = "#version 330 core
layout(location = 0) in vec4 a_clip;
layout(location = 1) in vec3 a_texcoord;
layout(location = 2) in vec4 a_color;
out vec3 v_texcoord;
out vec4 v_color;
void main() {
    gl_Position = a_clip;
    v_texcoord = a_texcoord;
    v_color = a_color;
}
";

/// `cheap_particles_render.hlsl`'s pixel shader with no fog to add. Its
/// alpha goes out whole, where the game's is a 32nd: the target's bias is
/// taken to undo that, as the tool's \"alpha of zero gives you additive\"
/// needs [inferred: not traced to the target's exponent bias].
pub(super) const CHEAP_PARTICLE_FRAGMENT: &str = "#version 330 core
uniform sampler2DArray u_render;
uniform float u_exposure;
// Halo 4 runs from its exposure to its self-illumination one by alpha;
// Reach's two are the same.
uniform float u_self_illum;
in vec3 v_texcoord;
in vec4 v_color;
out vec4 out_color;
void main() {
    vec4 texel = texture(u_render, v_texcoord, -0.5) * v_color;
    out_color = vec4(texel.rgb * mix(u_exposure, u_self_illum, texel.a), texel.a);
}
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::game::GameId;

    fn texels(width: usize, height: usize, texels: Vec<[f32; 4]>) -> CheapTexels {
        CheapTexels { width, height, texels }
    }

    /// A type of drag, gravity and turbulence, white, a unit at most.
    fn kind(drag: f32, gravity: f32, turbulence: f32) -> CheapType {
        CheapTypeTag {
            drag,
            gravity,
            turbulence,
            velocity_facing: false,
            color: [1.0; 4],
            intensity: 2.0,
            size: 1.0,
            y_scale: 1.0,
            texture: 3,
            change: -1,
            ..Default::default()
        }
        .compiled(0, 14, false)
    }

    fn library(types: Vec<CheapType>, turbulence: [f32; 4]) -> CheapLibrary {
        CheapLibrary {
            names: (0..types.len()).map(|at| format!("type{at}")).collect(),
            types,
            random: texels(1, 1, vec![[0.5, 0.5, 0.5, 0.5]]),
            turbulence: vec![CheapTurbulence {
                texels: texels(1, 1, vec![turbulence]),
                rates: [0.1, 0.05, 0.1, 0.05],
                centered: false,
            }],
            render: None,
            halo4: false,
        }
    }

    fn definition(library: CheapLibrary) -> CheapEmitterDefinition {
        CheapEmitterDefinition {
            correlate: true,
            position_local: false,
            velocity_local: false,
            normalize: false,
            rotate_up: false,
            spawn_rate: Arc::new(constant_function(60.0).unwrap()),
            input: String::new(),
            range: String::new(),
            lod: distance_fade([100.0, 150.0]),
            parameters: [[0.0, 0.0, 0.0, 254.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0; 4], [0.0; 4]],
            lighting: [1.0, 0.0, -11.0, 11.0],
            position: Arc::new(texels(1, 1, vec![[0.0; 4]])),
            velocity: Arc::new(texels(1, 1, vec![[0.0, 0.0, 1.0, 0.0]])),
            library: Arc::new(library),
        }
    }

    fn times(delay: f32, duration: f32) -> Arc<H3EffectTimes> {
        Arc::new(H3EffectTimes::of_events(&[([delay; 2], [duration; 2])], true))
    }

    fn emitter(definition: CheapEmitterDefinition, times: Arc<H3EffectTimes>) -> ReachCheapEmitterAt {
        ReachCheapEmitterAt {
            object: 0,
            node: 0,
            position: [0.0; 3],
            forward: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            only_under: None,
            scale_by: String::new(),
            secondary_by: String::new(),
            scaled: false,
            b_scaled: false,
            times,
            event: 0,
            definition: Arc::new(definition),
        }
    }

    fn view() -> CeFlareView {
        CeFlareView { eye: [0.0, -10.0, 0.0], forward: [0.0, 1.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0] }
    }

    fn live(field: &ReachCheapField) -> usize {
        field.particles.iter().filter(|particle| particle.age.abs() < 1.0).count()
    }

    /// `postprocess`'s upgrade runs from the version it finds: 0 fades from
    /// 5 to 20 and spawns at its old constant rate, 1 takes the lighting
    /// defaults too, 2 only the rotation and scale; 3 is left alone.
    #[test]
    fn an_old_emitter_is_upgraded_from_its_version_on() {
        let tag = |version: i64| CheapEmitterTag {
            version,
            spawn_rate: 12.0,
            function: None,
            fade: [1.0, 2.0],
            types: Default::default(),
            weights: [0.0; 4],
            lifetime: [1.0; 2],
            position_scale: 1.0,
            position_flatten: 0.0,
            directionality: [0.0; 2],
            speed: [0.0; 2],
            subframe: [0.0; 2],
            rotation_randomness: 0.5,
            scale_modifier: 0.5,
            lighting: [2.0, 3.0, -1.0, 1.0],
        }
        .upgraded();
        let zero = tag(0);
        assert_eq!(zero.fade, [5.0, 20.0]);
        assert_eq!(zero.function.as_ref().map(|function| function.evaluate(0.3, 0.7)), Some(12.0));
        assert_eq!((zero.lighting, zero.rotation_randomness, zero.scale_modifier), ([1.0, 0.0, -11.0, 11.0], 0.0, 1.0));
        let one = tag(1);
        assert_eq!((one.fade, one.lighting), ([1.0, 2.0], [1.0, 0.0, -11.0, 11.0]));
        assert!(one.function.is_none(), "only version 0's rate becomes a function");
        let two = tag(2);
        assert_eq!((two.lighting, two.rotation_randomness, two.scale_modifier), ([2.0, 3.0, -1.0, 1.0], 0.0, 1.0));
        let three = tag(3);
        assert_eq!((three.rotation_randomness, three.scale_modifier), (0.5, 0.5));
    }

    /// The fade is whole at the nearer bound and gone at the farther,
    /// whichever field holds which, and by the Manhattan distance.
    #[test]
    fn the_distance_fade_runs_from_the_nearer_bound_by_manhattan_distance() {
        let fade = distance_fade([100.0, 150.0]);
        assert_eq!(lod_scale(fade, 100.0), 1.0);
        assert_eq!(lod_scale(fade, 125.0), 0.5);
        assert_eq!(lod_scale(fade, 150.0), 0.0);
        let backwards = distance_fade([30.0, 25.0]);
        assert_eq!(lod_scale(backwards, 25.0), 1.0);
        assert_eq!(lod_scale(backwards, 27.5), 0.5);
        assert_eq!(manhattan([3.0, -4.0, 0.0], [0.0; 3]), 7.0, "not the 5 of a straight line");
        let point = distance_fade([10.0, 10.0]);
        assert!((point[0] + 1000.0).abs() < 1e-3, "a range under 0.001 is 0.001: {point:?}");
    }

    /// `spawn_params3`/`4`: thresholds from the weight before each found
    /// type, steps from the last found; a type not found holds the last
    /// threshold, and no weight at all is type 0's whole.
    #[test]
    fn types_pick_by_cumulative_weights_and_step_between_indices() {
        let (thresholds, steps) = type_thresholds([Some(5), Some(9), None, None], [0.5, 1.0, 0.0, 0.0]);
        assert_eq!(thresholds, [0.0, 0.5 / 1.5, 0.5 / 1.5, 0.5 / 1.5]);
        assert_eq!(steps, [5.0, 4.0, 0.0, 0.0]);
        // Sic: type 2 adds type 1's weight, though type 1 was not found.
        let (thresholds, steps) = type_thresholds([Some(1), None, Some(3), None], [1.0, 3.0, 1.0, 0.0]);
        assert_eq!(thresholds, [0.0, 0.0, 1.5, 1.5]);
        assert_eq!(steps, [1.0, 0.0, 2.0, 0.0]);
        let (thresholds, steps) = type_thresholds([Some(7), None, None, None], [0.0; 4]);
        assert_eq!((thresholds, steps), ([0.0; 4], [0.0; 4]), "no weight: type 0");
        let (_, steps) = type_thresholds([None, Some(4), None, None], [1.0, 1.0, 0.0, 0.0]);
        assert_eq!(steps, [0.0, 4.0, 0.0, 0.0], "a first type not found is 0");
    }

    /// `spawn_params0`–`2`: rotation held to 0..1, the modifier to 0..2 and
    /// packed by 254, lifetimes inverted over at least 1e-4.
    #[test]
    fn spawn_parameters_clamp_and_invert_as_postprocess_does() {
        let tag = CheapEmitterTag {
            version: 3,
            spawn_rate: 0.0,
            function: None,
            fade: [0.0; 2],
            types: ["a".to_owned(), String::new(), String::new(), String::new()],
            weights: [1.0, 0.0, 0.0, 0.0],
            lifetime: [0.0, 4.0],
            position_scale: 50.0,
            position_flatten: 0.25,
            directionality: [0.2, 0.6],
            speed: [2.0, 9.0],
            subframe: [0.0, 1.0],
            rotation_randomness: 3.0,
            scale_modifier: 7.5,
            lighting: [0.0; 4],
        };
        let [p0, p1, p2, ..] = tag.spawn_parameters(&["a".to_owned()]);
        assert_eq!(p0, [50.0, 0.25, 1.0, 508.0], "a modifier of 7.5 packs as 2's");
        assert!((p1[1] - 0.4).abs() < 1e-6 && p1[2] == 2.0 && p1[3] == 7.0, "{p1:?}");
        assert!((p2[2] - 10_000.0).abs() < 1.0, "a lifetime of none is 1e-4: {p2:?}");
        assert!((p2[3] - (0.25 - p2[2])).abs() < 1e-3);
    }

    /// The compile's rows: gravity as authored, change none as its own
    /// index, the colour by intensity, the fade over what is left, and the
    /// layer's place, the largest size, the stretch, and a screen-facing
    /// type's y scale negated.
    #[test]
    fn a_type_compiles_to_its_five_rows() {
        let tag = CheapTypeTag {
            drag: 1.5,
            gravity: 4.0,
            turbulence: 0.5,
            depth_range: 0.1,
            elasticity: 0.5,
            death: 0.7,
            change: -1,
            velocity_facing: false,
            color: [1.0, 0.5, 0.25, 0.0],
            intensity: 30.0,
            fade_start: 0.75,
            size: 0.2,
            stretch: 2.5,
            texture: 2,
            y_scale: 0.5,
            ..Default::default()
        };
        let rows = tag.compiled(9, 14, false).rows;
        assert_eq!(rows[PHYSICS], [1.5, 4.0, 0.5, 0.1]);
        assert_eq!(rows[COLLISION], [0.0, 0.5, 0.7, 9.0], "none changes to itself");
        assert_eq!(rows[COLOR0], [30.0, 15.0, 7.5, 0.0]);
        assert_eq!(rows[FADE], [0.0, 0.0, 0.0, 4.0]);
        assert_eq!(rows[RENDER], [2.5 / 14.0, 0.2, 2.5, -0.5]);
        let other = CheapTypeTag { change: 3, velocity_facing: true, texture: 20, fade_start: 1.0, ..tag };
        let rows = other.compiled(9, 14, false).rows;
        assert_eq!(rows[COLLISION][3], 3.0);
        assert_eq!(rows[RENDER][0], 0.5 / 14.0, "a texture out of range is the first");
        assert_eq!(rows[RENDER][3], 0.5, "velocity facing keeps its sign");
        assert_eq!(rows[FADE][3], 10_000.0, "held to 1e-4 left");
    }

    /// On MCC the lighting is a signed byte: an exposure above −0.086 packs
    /// to 127 and draws at 0.942, never brighter; below it, as computed.
    #[test]
    fn mcc_holds_the_lighting_byte_at_127() {
        let packed = |lighting: [f32; 4]| pack_sbyte4([0.0, packed_lighting(lighting, 1.0), 0.0, 0.0])[1];
        assert_eq!(packed([1.0, 0.0, 0.0, 0.0]), 127, "exposure 0 is 128 before the clamp");
        assert!((illumination(127) - 0.942).abs() < 1e-3, "{}", illumination(127));
        assert_eq!(packed([1.0, -0.25, -11.0, 11.0]), 125);
        assert_eq!(packed([1.0, 5.0, -11.0, 11.0]), 127);
        assert_eq!(packed([1.0, -20.0, -11.0, 11.0]), 0, "held at the minimum");
        assert_eq!(pack_sbyte4([3.9, -2.7, 200.0, -300.0]), [3, -2, 127, -128], "truncated and held");
    }

    /// A rate by the frame and scale, rounded by the roll.
    #[test]
    fn the_count_rounds_stochastically() {
        assert_eq!(spawn_count(60.0, 1.0 / 60.0, 1.0, 0.0), 1);
        assert_eq!(spawn_count(30.0, 1.0 / 60.0, 1.0, 0.4), 0);
        assert_eq!(spawn_count(30.0, 1.0 / 60.0, 1.0, 0.6), 1);
        assert_eq!(spawn_count(30.0, 1.0 / 60.0, 0.5, 0.6), 0, "the scale counts");
    }

    /// Each scale is ±[1, 2)/64 and each offset in [0, 1).
    #[test]
    fn a_texture_transform_scales_by_a_64th_either_way() {
        let mut random = Random(7);
        let (mut negative, mut positive) = (false, false);
        for _ in 0..64 {
            let [u, v, z, w] = texture_transform(&mut random);
            for scale in [u, v] {
                assert!((1.0..=2.0).contains(&(scale.abs() * 64.0)), "{scale}");
                negative |= scale < 0.0;
                positive |= scale > 0.0;
            }
            assert!((0.0..=1.0).contains(&z) && (0.0..=1.0).contains(&w));
        }
        assert!(negative && positive);
    }

    /// Correlated, the velocity reads the position's texel; otherwise its
    /// own roll.
    #[test]
    fn a_correlated_emitter_reads_velocity_where_it_reads_position() {
        let [_, position, velocity] = spawn_transforms(&mut Random(3), true);
        assert_eq!(position, velocity);
        let [_, position, velocity] = spawn_transforms(&mut Random(3), false);
        assert_ne!(position, velocity);
    }

    /// The rate reads the game time, the emitter's scale for `alpha`, and
    /// another name off the object.
    #[test]
    fn the_spawn_rate_reads_game_time_alpha_or_an_object_function() {
        let values = H2FunctionValues::from([("power".to_owned(), 0.25)]);
        assert_eq!(spawn_input("", Some(&values), 12.5, 0.5), 12.5, "Reach's game time, in seconds");
        assert_eq!(spawn_input("", None, 7300.0, 0.5), -7100.0, "which comes round at two hours");
        assert_eq!(spawn_input(ALPHA, Some(&values), 12.5, 0.5), 0.5);
        assert_eq!(spawn_input("power", Some(&values), 12.5, 0.5), 0.25);
        assert_eq!(spawn_input("heat", Some(&values), 12.5, 0.5), 0.0, "not answered");
    }

    /// The turned up lies across forward and is not made a unit.
    #[test]
    fn a_rotated_emitters_up_crosses_its_forward_unnormalized() {
        let mut random = Random(11);
        let mut lengths = Vec::new();
        for _ in 0..16 {
            let up = turned_up([1.0, 0.0, 0.0], &mut random);
            assert_eq!(up[0], 0.0, "across forward");
            lengths.push(dot(up, up).sqrt());
        }
        assert!(lengths.iter().any(|length| (length - 1.0).abs() > 0.05), "{lengths:?}");
    }

    /// Spawned with no speed and no flatten, a particle sits at the emitter,
    /// takes a tenth of the subframe as its age, and its type from where the
    /// random's w falls among the thresholds.
    #[test]
    fn a_spawned_particle_takes_its_subframe_and_type() {
        let mut emitter = definition(library(vec![kind(0.0, 0.0, 0.0), kind(0.0, 40.0, 0.0)], [0.0; 4]));
        let at = CheapSubmission { position: [1.0, 2.0, 3.0], forward: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0], scale: 1.0 };
        let transforms = [[0.0; 4]; 3];
        let particle = emitter.spawn_particle(&at, &transforms, 128.0, 0, 0.5);
        assert_eq!(particle.position, [1.0, 2.0, 3.0]);
        assert_eq!(particle.age, 0.0001, "no subframe: the least age");
        assert_eq!(particle.parameters, [0, 127, -63, 0], "type 0, the lighting held, no rotation");
        // A subframe of half the step: the age is a tenth of it, not by the life.
        emitter.parameters[2] = [0.5, 0.0, 1.0, 0.0];
        emitter.parameters[3] = [0.0, 0.25, 1.0, 1.0];
        emitter.parameters[4] = [0.0, 1.0, 0.0, 0.0];
        let particle = emitter.spawn_particle(&at, &transforms, 128.0, 0, 0.5);
        assert_eq!(particle.age, 0.1 * 0.25);
        assert_eq!(particle.parameters[0], 1, "w 0.5 passes 0.25, not 1");
        assert!(particle.position[2] < 3.0 && particle.velocity[2] < 0.0, "type 1 falls over its subframe");
    }

    /// In local space a texture's x runs along up, y along left and z along
    /// forward; flatten takes forward's share away.
    #[test]
    fn local_space_and_flatten_follow_the_emitters_axes() {
        let mut emitter = definition(library(vec![kind(0.0, 0.0, 0.0)], [0.0; 4]));
        emitter.position = Arc::new(texels(1, 1, vec![[1.0, 0.0, 0.0, 0.0]]));
        emitter.parameters[0] = [2.0, 0.0, 0.0, 0.0];
        let at = CheapSubmission { position: [0.0; 3], forward: [0.0, 1.0, 0.0], up: [0.0, 0.0, 1.0], scale: 1.0 };
        let world = emitter.spawn_particle(&at, &[[0.0; 4]; 3], 0.0, 0, 0.0).position;
        assert_eq!(world, [2.0, 0.0, 0.0], "world space: x is x");
        emitter.position_local = true;
        let local = emitter.spawn_particle(&at, &[[0.0; 4]; 3], 0.0, 0, 0.0).position;
        assert_eq!(local, [0.0, 0.0, 2.0], "local space: x is up");
        emitter.position = Arc::new(texels(1, 1, vec![[0.0, 0.0, 1.0, 0.0]]));
        emitter.parameters[0] = [2.0, 1.0, 0.0, 0.0];
        let flat = emitter.spawn_particle(&at, &[[0.0; 4]; 3], 0.0, 0, 0.0).position;
        assert_eq!(flat, [0.0, 0.0, 0.0], "forward flattened away");
    }

    /// Drag slows, a positive gravity pulls down, turbulence moves the place
    /// and not the velocity, and at rest nothing moves but the age.
    #[test]
    fn the_update_drags_falls_and_turbulates_the_place_only() {
        let library = library(vec![kind(1.0, 4.0, 2.0)], [0.5, 0.0, 0.0, 0.0]);
        let mut particle = CheapParticle { position: [0.0; 3], age: 0.5, velocity: [1.0, 0.0, 0.0], delta_age: 0.5, parameters: [0; 4] };
        update_particle(&mut particle, 0, &library, &[[0.0; 4]], 0.1);
        assert!((particle.age - 0.55).abs() < 1e-6);
        assert!((particle.position[0] - (1.0 + 0.5 * 2.0) * 0.1).abs() < 1e-6, "{:?}", particle.position);
        assert!((particle.position[2] + 4.0 * 0.01).abs() < 1e-6, "gravity pulls down");
        assert!((particle.velocity[0] - 0.9).abs() < 1e-6, "dragged, and the turbulence is not in it");
        assert!((particle.velocity[2] + 0.4).abs() < 1e-6);
        let mut resting = CheapParticle { age: -0.5, ..particle };
        update_particle(&mut resting, 0, &library, &[[0.0; 4]], 0.1);
        assert_eq!(resting.position, particle.position, "at rest it stays");
        assert!((resting.age + 0.55).abs() < 1e-6, "and ages away from 0");
    }

    /// The shaders' step is held to half a second; the turbulence's phase
    /// moves by the whole frame.
    #[test]
    fn a_long_frame_moves_particles_half_a_second() {
        let library = library(vec![kind(0.0, 0.0, 0.0)], [0.0; 4]);
        let mut field = ReachCheapField::fresh(1);
        field.particles[0] = CheapParticle { position: [0.0; 3], age: 0.1, velocity: [1.0, 0.0, 0.0], delta_age: 0.0, parameters: [0; 4] };
        field.update(&library, 2.0);
        assert_eq!(field.particles[0].position[0], 0.5, "not 2");
        assert!((field.phases[0][0] - 0.2).abs() < 1e-6, "{:?}", field.phases);
        field.update(&library, 0.25);
        assert_eq!(field.particles[0].position[0], 0.75);
    }

    /// Spawning past the end of the ring comes round and overwrites the
    /// oldest slot, alive or not.
    #[test]
    fn the_ring_overwrites_from_the_start() {
        let definition = definition(library(vec![kind(0.0, 0.0, 0.0)], [0.0; 4]));
        let at = emitter(definition, times(0.0, 100.0));
        let mut field = ReachCheapField::fresh(1);
        field.cursor = MOST_PARTICLES - 1;
        field.particles[0].age = 0.5;
        let submission = CheapSubmission { position: [9.0; 3], forward: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0], scale: 1.0 };
        // 60 a second over two 60ths, at most one more from the roll.
        field.spawn(&at, &submission, None, 0.0, 2.0 / 60.0, 0);
        assert!(field.cursor == 1 || field.cursor == 2, "{}", field.cursor);
        assert_eq!(field.particles[MOST_PARTICLES - 1].position, [9.0; 3]);
        assert_eq!(field.particles[0].position, [9.0; 3], "slot 0 written over");
    }

    /// An event of no length never emits; one with a duration emits while
    /// in it, at the type-specific scales it asks for.
    #[test]
    fn only_an_event_with_a_duration_submits_and_scales_as_asked() {
        let make = |times| emitter(definition(library(vec![kind(0.0, 0.0, 0.0)], [0.0; 4])), times);
        let eye = [0.0; 3];
        assert!(make(times(0.0, 0.0)).submission(None, None, eye, 0.0, 0.0).is_none(), "an instant");
        assert!(make(times(0.0, 1.0)).submission(None, None, eye, 0.5, 0.0).is_some());
        assert!(make(times(1.0, 1.0)).submission(None, None, eye, 0.5, 0.0).is_none(), "in its delay");
        let values = H2FunctionValues::from([("power".to_owned(), 0.5), ("heat".to_owned(), 0.25)]);
        let mut scaled = make(times(0.0, 1.0));
        scaled.scale_by = "power".to_owned();
        scaled.secondary_by = "heat".to_owned();
        let scale = |at: &ReachCheapEmitterAt| at.submission(Some(&values), None, eye, 0.5, 0.0).map(|at| at.scale);
        assert_eq!(scale(&scaled), Some(1.0), "the effect runs, but the part takes no scale");
        scaled.scaled = true;
        assert_eq!(scale(&scaled), Some(0.5));
        scaled.b_scaled = true;
        assert_eq!(scale(&scaled), Some(0.125));
        let off = H2FunctionValues::from([("power".to_owned(), 0.0)]);
        assert!(scaled.submission(Some(&off), None, eye, 0.5, 0.0).is_none(), "its effect is off");
        let far = make(times(0.0, 1.0)).submission(None, None, [100.0, 50.0, 0.0], 0.5, 0.0);
        assert!(far.is_none(), "150 away by Manhattan distance, past the fade");
    }

    /// At most 200 emitters a frame spawn; the rest wait.
    #[test]
    fn no_more_than_200_emitters_spawn_a_frame() {
        let one = emitter(definition(library(vec![kind(0.0, 0.0, 0.0)], [0.0; 4])), times(0.0, 100.0));
        let count = |emitters: usize| {
            let all = vec![one.clone(); emitters];
            let mut field = ReachCheapField::fresh(emitters);
            field.time = Some(0.0);
            field.advance(&all, &[], |_| None, &mut H3EffectOnsets::default(), None, [0.0; 3], 1.0 / 60.0);
            live(&field)
        };
        assert_eq!(count(200), 200, "a particle each at 60 a second");
        assert_eq!(count(250), 200);
    }

    /// Facing the velocity, a quad stretches along it by its speed; facing
    /// the screen it lies in the camera's right and up and flips v; a quad
    /// at rest facing its velocity draws nothing.
    #[test]
    fn a_quad_faces_its_velocity_or_the_screen() {
        let mut moving = kind(0.0, 0.0, 0.0);
        moving.rows[RENDER] = [0.0, 1.0, 2.0, 1.0];
        let screen = kind(0.0, 0.0, 0.0);
        let library = library(vec![moving, screen], [0.0; 4]);
        let mut field = ReachCheapField::fresh(0);
        // Rotation (−63.5, 0): a unit along the basis's first axis.
        let particle = CheapParticle { position: [0.0; 3], age: 0.0, velocity: [0.0, 0.0, 3.0], delta_age: 0.0, parameters: [0, 127, -64, 0] };
        field.particles[0] = particle;
        let quad = field.vertices(&library, &view());
        assert_eq!(quad.len(), 6);
        let span = |quad: &[ReachCheapVertex], axis: usize| {
            let values = quad.iter().map(|vertex| vertex.place[axis]);
            values.clone().fold(f32::MIN, f32::max) - values.fold(f32::MAX, f32::min)
        };
        let length = 64.0 / 63.5 * 6.0;
        assert!((span(&quad, 2) - length).abs() < 1e-4, "stretched by 2 · 3: {}", span(&quad, 2));
        field.particles[0].velocity = [0.0; 3];
        assert!(field.vertices(&library, &view()).is_empty(), "NaN at rest");
        field.particles[0] = CheapParticle { parameters: [1, 127, -64, 0], ..particle };
        let quad = field.vertices(&library, &view());
        let side = 64.0 / 63.5;
        assert!((span(&quad, 0) - side).abs() < 1e-4 && (span(&quad, 2) - side).abs() < 1e-4, "the screen's right and up, unstretched");
        assert!(span(&quad, 1) < 1e-4, "square to the view");
        let flipped = quad.iter().find(|vertex| vertex.texcoord[0] == 0.0 && vertex.place[0] > 0.0).unwrap();
        assert_eq!(flipped.texcoord[2], 3.0, "the type's layer");
        assert!(quad.iter().any(|vertex| vertex.texcoord[1] == 1.0), "{quad:?}");
    }

    /// Colour is the type's by its intensity, the packed lighting and the
    /// fade from `fade start`; alpha by the fade alone.
    #[test]
    fn a_quads_colour_fades_from_its_fade_start() {
        let mut tag = CheapTypeTag { color: [1.0, 1.0, 1.0, 0.5], intensity: 2.0, size: 1.0, y_scale: 1.0, fade_start: 0.5, ..Default::default() };
        tag.change = -1;
        let library = library(vec![tag.compiled(0, 14, false)], [0.0; 4]);
        let mut field = ReachCheapField::fresh(0);
        let at = |age: f32| CheapParticle { position: [0.0; 3], age, velocity: [0.0; 3], delta_age: 0.0, parameters: [0, 127, -64, 0] };
        field.particles[0] = at(0.25);
        let full = field.vertices(&library, &view())[0].color;
        assert!((full[0] - 2.0 * illumination(127)).abs() < 1e-5 && full[3] == 0.5, "{full:?}");
        field.particles[0] = at(0.75);
        let half = field.vertices(&library, &view())[0].color;
        assert!((half[0] - full[0] * 0.5).abs() < 1e-5 && (half[3] - 0.25).abs() < 1e-6, "{half:?}");
    }

    /// Halo 4's turbulence type is a place in an array of eight: the layer
    /// is the type, the transform `(int)(4 · place)` only type 0's.
    #[test]
    fn halo_4_takes_its_turbulence_layer_and_transform_as_the_shader_does() {
        let place = |kind: usize| (kind as f32 + 0.5) / MOST_TURBULENCE_TYPES as f32;
        assert_eq!(turbulence_of(place(0)), (0, 0));
        assert_eq!(turbulence_of(place(1)), (1, 0), "type 1 borrows type 0's transform");
        assert_eq!(turbulence_of(place(2)), (2, 1));
        assert_eq!(turbulence_of(place(3)), (3, 1));
    }

    /// Halo 4's push is its type's own turbulence, spanning -1 to 1 of the
    /// map; Reach's one map pushes as it is.
    #[test]
    fn halo_4_pushes_by_its_types_turbulence_centered() {
        let tag = CheapTypeTag { turbulence: 1.0, turbulence_type: 2, change: -1, size: 1.0, ..Default::default() };
        let mut library = library(vec![tag.compiled(0, 14, true)], [0.0; 4]);
        let kind = |value: f32| CheapTurbulence { texels: texels(1, 1, vec![[value, value, value, 1.0]]), rates: [0.0; 4], centered: true };
        library.turbulence = vec![kind(0.5), kind(0.5), kind(1.0)];
        library.halo4 = true;
        let still = CheapParticle { position: [0.0; 3], age: 0.5, velocity: [0.0; 3], delta_age: 0.0, parameters: [0; 4] };
        let mut particle = still;
        update_particle(&mut particle, 0, &library, &[[0.0; 4]; 2], 0.1);
        assert!((particle.position[0] - 0.1).abs() < 1e-6, "layer 2, 1 centered to 1: {:?}", particle.position);
        library.turbulence[2] = kind(0.5);
        let mut centered = still;
        update_particle(&mut centered, 0, &library, &[[0.0; 4]; 2], 0.1);
        assert_eq!(centered.position, [0.0; 3], "the map's middle pushes nothing");
        library.halo4 = false;
        library.turbulence = vec![CheapTurbulence { centered: false, ..kind(0.5) }];
        let mut reach = still;
        update_particle(&mut reach, 0, &library, &[[0.0; 4]], 0.1);
        assert!((reach.position[0] - 0.05).abs() < 1e-6, "Reach's map as it is: {:?}", reach.position);
    }

    /// Halo 4's quad is sized between `size`'s bounds by its slot; Reach's
    /// is the upper bound.
    #[test]
    fn halo_4_sizes_each_quad_between_its_bounds() {
        let tag = CheapTypeTag { size_lower: 1.0, size: 3.0, y_scale: 1.0, change: -1, color: [1.0; 4], ..Default::default() };
        let mut library = library(vec![tag.compiled(0, 14, true)], [0.0; 4]);
        library.halo4 = true;
        let mut field = ReachCheapField::fresh(0);
        let particle = CheapParticle { position: [0.0; 3], age: 0.0, velocity: [0.0; 3], delta_age: 0.0, parameters: [0, 127, -64, 0] };
        field.particles[0] = particle;
        field.particles[1] = particle;
        let width = |quad: &[ReachCheapVertex]| {
            let values = quad.iter().map(|vertex| vertex.place[0]);
            (values.clone().fold(f32::MIN, f32::max) - values.fold(f32::MAX, f32::min)) * 63.5 / 64.0
        };
        let quads = field.vertices(&library, &view());
        let (first, second) = (width(&quads[..6]), width(&quads[6..]));
        // Slot 0 hashes to 0, slot 1 to frac(0.863…).
        assert!((first - 1.0).abs() < 1e-4, "{first}");
        assert!((second - (1.0 + 2.0 * 0.863_182_4)).abs() < 1e-3, "{second}");
        library.halo4 = false;
        let reach = field.vertices(&library, &view());
        assert!((width(&reach[..6]) - 3.0).abs() < 1e-4);
    }

    /// With `BLAM_TEST_H4EK`: Halo 4's shipped type texture is the compile of
    /// its library's types, all five rows of all 152, and its four kinds of
    /// turbulence read.
    #[test]
    fn kit_halo_4s_type_table_matches_the_shipped_type_texture() {
        let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
            eprintln!("skipping: BLAM_TEST_H4EK is not set");
            return;
        };
        let source = super::super::h3_particles::loose_kit(&root, GameId::Halo4);
        let library = CheapLibrary::read(&source, "fx\\cheap_particles\\global").expect("the library");
        assert!(library.halo4());
        assert_eq!((library.types.len(), library.turbulence.len()), (152, 4));
        let tag = load_referenced_tag_from_source(&source, "fx\\cheap_particles\\global_type_texture", "bitmap", b"bitm").unwrap();
        let bitmap = blam_tags::bitmap::Bitmap::new(&tag).unwrap();
        let image = bitmap.image(0).unwrap();
        let pixels = image.pixel_bytes().unwrap();
        let half = |at: usize| half_to_f32(u16::from_le_bytes([pixels[at], pixels[at + 1]]));
        let mut checked = 0;
        for (index, found) in library.types.iter().enumerate() {
            for row in 0..5 {
                let texel: [f32; 4] = std::array::from_fn(|channel| half(((row * 256 + index) * 4 + channel) * 2));
                for (got, want) in texel.iter().zip(found.rows[row]) {
                    assert!((got - want).abs() <= (0.01 * want.abs()).max(2e-3), "type {index} row {row}: {texel:?} against {:?}", found.rows[row]);
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 760);
        assert!(library.render_texture().is_some(), "the render texture array reads");
    }

    /// The type texture the kit ships is the compile of the library's types:
    /// all five rows of every type agree with `globals\global_type_texture`.
    #[test]
    fn kit_the_type_table_matches_the_shipped_type_texture() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            eprintln!("skipping: BLAM_TEST_HREK is not set");
            return;
        };
        let source = super::super::h3_particles::loose_kit(&root, GameId::HaloReach);
        let library = CheapLibrary::read(&source, "globals\\global").expect("the library");
        assert_eq!(library.types.len(), 60);
        let tag = load_referenced_tag_from_source(&source, "globals\\global_type_texture", "bitmap", b"bitm").unwrap();
        let bitmap = blam_tags::bitmap::Bitmap::new(&tag).unwrap();
        let image = bitmap.image(0).unwrap();
        assert_eq!((image.width(), image.height()), (256, 8));
        let pixels = image.pixel_bytes().unwrap();
        let half = |at: usize| half_to_f32(u16::from_le_bytes([pixels[at], pixels[at + 1]]));
        let mut checked = 0;
        for (index, found) in library.types.iter().enumerate() {
            for row in 0..5 {
                let texel: [f32; 4] = std::array::from_fn(|channel| half(((row * 256 + index) * 4 + channel) * 2));
                for (got, want) in texel.iter().zip(found.rows[row]) {
                    assert!((got - want).abs() <= (0.01 * want.abs()).max(2e-3), "type {index} row {row}: {texel:?} against {:?}", found.rows[row]);
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 300);
        assert!(library.render_texture().is_some_and(|render| render.layers == 14 && render.levels.len() == 7));
    }

    /// With `BLAM_TEST_HREK`: m52's electrical fire reads its sparks as
    /// `postprocess` would leave them, at the effect scenery's marker, and
    /// they are submitted only in the first tenth of a second of its cycle.
    #[test]
    fn kit_an_electrical_fire_reads_its_postprocessed_sparks() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            eprintln!("skipping: BLAM_TEST_HREK is not set");
            return;
        };
        let source = super::super::h3_particles::loose_kit(&root, GameId::HaloReach);
        let path = "levels\\solo\\m52\\fx\\city_damage_fx\\building_electrical_fire";
        let object = load_referenced_tag_from_source(&source, path, "effect_scenery", b"efsc").unwrap();
        let marker = RenderModelPreviewMarker {
            name: "marker".to_owned(),
            node_index: 0,
            position: [0.0; 3],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale: 1.0,
            only_under: None,
        };
        let frame = |marker: &RenderModelPreviewMarker| {
            (MarkerFrame { axes: marker.axes, position: marker.position, node: marker.node_index }, 0)
        };
        let emitters = read_reach_cheap_emitters(&source, &object, 0, &[marker], frame);
        assert_eq!(emitters.len(), 1, "one cpem part");
        let definition = &*emitters[0].definition;
        assert!(definition.correlate && definition.position_local && definition.velocity_local && definition.rotate_up);
        assert!(!definition.normalize);
        assert_eq!(definition.lod, distance_fade([180.0, 200.0]));
        let [p0, p1, p2, p3, p4] = definition.parameters;
        assert_eq!(p0, [0.0, 1.0, 0.0, 381.0]);
        assert_eq!(p1, [0.0, 0.0, 20.0, 0.0]);
        assert!((p2[1] - 0.8).abs() < 1e-6 && (p2[2] - 1.0 / 1.5).abs() < 1e-6 && (p2[3] - (0.25 - 1.0 / 1.5)).abs() < 1e-6, "{p2:?}");
        let names = &definition.library.names;
        let first = names.iter().position(|known| known == "spark_electrical01").unwrap() as f32;
        assert_eq!((p3, p4), ([0.0; 4], [first, 0.0, 0.0, 0.0]));
        assert_eq!(definition.spawn_rate.evaluate(0.0, 0.5), 5000.0, "the mapping's, not version 0's `spawn rate`");
        assert_eq!(pack_sbyte4([0.0, packed_lighting(definition.lighting, 1.0), 0.0, 0.0])[1], 127);
        let on = |time: f32| emitters[0].submission(None, None, [0.0; 3], time, 0.0).is_some();
        assert!(on(0.05), "in its event of a tenth of a second");
        assert!(!on(0.5), "after it");
        assert!(on(15.05), "the next cycle, 15 seconds on");
    }

    /// With `BLAM_TEST_HREK`: m45's huge space junk attaches its effect on
    /// `marker`, and its cheap particle part sits at `up`, a vector
    /// `build_gravity_effect_vectors` makes there, not a marker of the model.
    #[test]
    fn kit_space_junk_emits_from_its_up_vector() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            eprintln!("skipping: BLAM_TEST_HREK is not set");
            return;
        };
        let source = super::super::h3_particles::loose_kit(&root, GameId::HaloReach);
        let path = "levels\\solo\\m45\\fx\\space_junk_huge";
        let object = load_referenced_tag_from_source(&source, path, "effect_scenery", b"efsc").unwrap();
        let marker = RenderModelPreviewMarker {
            name: "marker".to_owned(),
            node_index: 0,
            position: [1.0, 2.0, 3.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale: 1.0,
            only_under: None,
        };
        let frame = |marker: &RenderModelPreviewMarker| {
            (MarkerFrame { axes: marker.axes, position: marker.position, node: marker.node_index }, 0)
        };
        let emitters = read_reach_cheap_emitters(&source, &object, 0, &[marker], frame);
        assert_eq!(emitters.len(), 1, "one cpem part, though the model has no `up` marker");
        let emitter = &emitters[0];
        assert_eq!(emitter.position, [1.0, 2.0, 3.0], "at the attachment's marker");
        assert_eq!((emitter.forward, emitter.up), ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]), "facing up");
    }

    /// IEEE half to single, for the kit's `abgrfp16` type texture.
    fn half_to_f32(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exponent = i32::from((bits >> 10) & 0x1f);
        let mantissa = f32::from(bits & 0x3ff);
        sign * match exponent {
            0 => mantissa / 1024.0 * 2f32.powi(-14),
            31 => f32::INFINITY,
            _ => (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
        }
    }
}
