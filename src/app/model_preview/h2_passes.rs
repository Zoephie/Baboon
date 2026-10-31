//! A Halo 2 shader's transparent passes, as data the renderer can draw.
//!
//! A shader names a template, whose LODs list passes by layer. Each pass reads
//! its bitmaps, values and colors by parameter name, and the game looks a name
//! up in the shader, then the template, then the pass. A parameter the pass
//! gives a `source extern` is filled at runtime instead.
//!
//! Only LOD 0 is read. An object draws transparent passes with implementation
//! 0 and lightmap passes with `H2_OBJECT_LIGHTMAP_IMPLEMENTATION`, as the game
//! chooses them for something with no lightmap of its own.

use std::collections::HashMap;

use blam_tags::{TagFieldData, TagFile, TagStruct};

use super::h2_prt::H2PrtLight;
use super::h2_stage::{H2_TEXTURE_ROWS, H2VertexStage};
use super::materials::TextureImage;
use crate::core::h2_pixel_shader::{
    AlphaCompare, AlphaTest, PixelShaderOptions, SamplerKind, translate,
};

/// A layer the preview draws, in the order the game's `render_scene` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum H2Layer {
    Texaccum,
    /// A late fill of the texaccum target, drawn to it per light in the game,
    /// for shaders whose color changes rather than sitting in a base map.
    LightAlbedo,
    /// The fallback for a scene with no lightmaps, which adds the default
    /// lights on top.
    Lightmap,
    /// The object path in a level with lightmaps; see `H2_OBJECT_LIGHTMAP_IMPLEMENTATION`.
    ObjectLightmap,
    /// Precomputed radiance transfer, for the shaders with no lightmap pass.
    Prt,
    Environment,
    SelfIllumination,
    Overlay,
    LightDiffuse,
    LightSpecular,
    Hologram,
    /// Takes over from every other opaque layer while camo is on.
    ActiveCamo,
    Transparent,
    /// A mask written to the frame's alpha, for the glow drawn from it.
    Bloom,
    /// Not the game's: see `h2_shadow_passes`.
    ShadowCaster,
    ShadowMask,
}

pub(crate) const H2_LAYER_COUNT: usize = 16;

/// The sphere lights `render_lights` falls back on with nothing else lighting
/// the scene: a warm key and a cool fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H2DefaultLight {
    pub(crate) position: [f32; 3],
    pub(crate) color: [f32; 3],
    pub(crate) radius: f32,
}

pub(crate) const H2_DEFAULT_LIGHTS: [H2DefaultLight; 2] = [
    H2DefaultLight { position: [2000.0, 2000.0, 2000.0], color: [0.9, 0.9, 0.8], radius: 40000.0 },
    H2DefaultLight { position: [-2000.0, -2000.0, 500.0], color: [0.2, 0.2, 0.3], radius: 40000.0 },
];

impl H2Layer {
    pub(crate) const ALL: [Self; H2_LAYER_COUNT] = [
        Self::Texaccum,
        Self::LightAlbedo,
        Self::Lightmap,
        Self::ObjectLightmap,
        Self::Prt,
        Self::Environment,
        Self::SelfIllumination,
        Self::Overlay,
        Self::LightDiffuse,
        Self::LightSpecular,
        Self::Hologram,
        Self::ActiveCamo,
        Self::Transparent,
        Self::Bloom,
        Self::ShadowCaster,
        Self::ShadowMask,
    ];

    /// The tag's `layer` option.
    pub(in crate::app) fn tag_name(self) -> &'static str {
        match self {
            Self::Texaccum => TEXACCUM_LAYER,
            Self::LightAlbedo => "light albedo",
            Self::Lightmap | Self::ObjectLightmap => LIGHTMAP_LAYER,
            Self::Prt => "sh prt",
            Self::Environment => "environment map",
            Self::SelfIllumination => "self-illumination",
            Self::Overlay => "overlay",
            Self::LightDiffuse => "diffuse",
            Self::LightSpecular => "specular",
            Self::Hologram => "hologram",
            Self::ActiveCamo => "active camo",
            Self::Bloom => "bloom",
            Self::ShadowCaster | Self::ShadowMask => "",
            Self::Transparent => TRANSPARENT_LAYER,
        }
    }

    /// An object's implementation on this layer. Only the lightmap layer has
    /// a choice to make; see `rasterizer_shader_get_implementation_index`.
    pub(in crate::app) fn implementation(self) -> usize {
        match self {
            Self::Lightmap => H2_FALLBACK_LIGHTMAP_IMPLEMENTATION,
            Self::ObjectLightmap => H2_OBJECT_LIGHTMAP_IMPLEMENTATION,
            Self::LightDiffuse => H2_SPHERE_LIGHT_DIFFUSE_IMPLEMENTATION,
            Self::ActiveCamo => H2_FULL_ACTIVE_CAMO_IMPLEMENTATION,
            _ => 0,
        }
    }
}

/// The passes behind the preview's own key light shadow. Under the default
/// lights the game draws no shadows at all: layers 10 and 11 only run for a
/// light object that casts them.
pub(crate) fn h2_shadow_passes() -> [(H2Layer, H2DrawPass); 2] {
    let pass = |stage, fragment: String, compare, write| H2DrawPass {
        stage,
        fragment,
        blend: None,
        depth: H2Depth { compare: Some(compare), write },
        channels: [false; 4],
        textures: Vec::new(),
        constants: Vec::new(),
        texture_rows: identity_rows(),
        tint_facing: [1.0; 4],
        tint_glancing: [1.0; 4],
        change_colors: Vec::new(),
        light_colors: Vec::new(),
        light_direct: [0.0; 3],
        light_indirect: [0.0; 3],
    };
    [
        (
            H2Layer::ShadowCaster,
            pass(
                H2VertexStage::ShadowCaster,
                format!("{H2_GLSL_VERSION}void main() {{}}\n"),
                H2Compare::LessOrEqual,
                true,
            ),
        ),
        (
            H2Layer::ShadowMask,
            pass(
                H2VertexStage::ShadowMask,
                format!(
                    "{H2_GLSL_VERSION}in vec4 v_tc0;
uniform sampler2D u_shadow_map;
void main() {{
    vec3 place = v_tc0.xyz * 0.5 + 0.5;
    if (place.z - {H2_SHADOW_BIAS:.4} <= texture(u_shadow_map, place.xy).r) {{
        discard;
    }}
}}
"
                ),
                H2Compare::Equal,
                false,
            ),
        ),
    ]
}

/// Enough to clear a surface of its own shadow, as a share of the depth range.
const H2_SHADOW_BIAS: f32 = 0.004;

/// What a model's `sh prt` passes are lit by. The game samples the level's
/// light probes here, and with none it falls back on a stand-in from straight
/// below; a preview uses its own lights instead.
pub(crate) fn h2_prt_lights() -> [H2PrtLight; 2] {
    H2_DEFAULT_LIGHTS.map(|light| {
        let [x, y, z] = light.position;
        let length = (x * x + y * y + z * z).sqrt();
        H2PrtLight { direction: [x / length, y / length, z / length], color: light.color }
    })
}

const TRANSPARENT_LAYER: &str = "transparent";
const LIGHTMAP_LAYER: &str = "lightmap (indirect)";
const TEXACCUM_LAYER: &str = "texaccum";
const TEXACCUM_TARGET: &str = "GLOBAL target texaccum";
/// What the game copies the frame into, for what looks through a surface.
const FRAME_COPY: &str = "GLOBAL active camo";
const WATER_REFRACTION: &str = "GLOBAL water refraction";
/// What a live camera feeds, which a preview has none of: a dark screen.
const CAMERA_FEEDS: [&str; 2] = ["GLOBAL texture camera", "FIRST-PERSON scope"];
/// The shader's own bump map, by another name.
const ACTIVE_CAMO_BUMP: &str = "SHADER active camo bump";
/// What `render_visible_section_get_lightmap_mode` picks when the level has
/// no lightmaps: fixed lighting, with the default lights doing the rest.
const H2_FALLBACK_LIGHTMAP_IMPLEMENTATION: usize = 3;
/// What it picks for a model in a level with lightmaps. This pass reads the
/// object's lighting and takes its reflection from the object's brightness.
const H2_OBJECT_LIGHTMAP_IMPLEMENTATION: usize = 2;
/// "amount=1 (no blending, requires no opaque render)".
const H2_FULL_ACTIVE_CAMO_IMPLEMENTATION: usize = 1;
/// `render_light_get_diffuse_implementation_index` for a light of type sphere.
const H2_SPHERE_LIGHT_DIFFUSE_IMPLEMENTATION: usize = 1;
const LIGHT_FALLOFF: &str = "LIGHT falloff";
/// The one frame texture a preview can supply itself.
const VECTOR_NORMALIZATION: &str = "GLOBAL vector-normalization";
const NO_EXTERN: &str = "none";

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum H2ParameterValue {
    Bitmap(String),
    Value(f32),
    Color([f32; 3]),
    /// Filled at runtime, by this option name of the pass's `source extern`.
    Extern(String),
    /// Nothing in the shader, template or pass supplies it.
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum H2BlendFactor {
    Zero,
    One,
    SrcColor,
    OneMinusSrcColor,
    SrcAlpha,
    OneMinusSrcAlpha,
    DstColor,
    OneMinusDstColor,
    DstAlpha,
    OneMinusDstAlpha,
    SrcAlphaSaturate,
    ConstantColor,
    OneMinusConstantColor,
    ConstantAlpha,
    OneMinusConstantAlpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum H2BlendEquation {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app) struct H2Blend {
    pub(in crate::app) equation: H2BlendEquation,
    pub(in crate::app) source: H2BlendFactor,
    pub(in crate::app) destination: H2BlendFactor,
    /// ARGB as stored, for the `constant` factors.
    pub(in crate::app) constant: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum H2Compare {
    Never,
    Less,
    Equal,
    LessOrEqual,
    Greater,
    NotEqual,
    GreaterOrEqual,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) struct H2Depth {
    /// `None` turns the depth test off.
    pub(in crate::app) compare: Option<H2Compare>,
    pub(in crate::app) write: bool,
}

/// A vertex constant the pass fills from a parameter: for a bitmap, a row of
/// its texture transform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct H2VertexConstant {
    pub(in crate::app) parameter: String,
    pub(in crate::app) bank: String,
    pub(in crate::app) register: i32,
    pub(in crate::app) components: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct H2PassTexture {
    pub(in crate::app) parameter: String,
    pub(in crate::app) mode: String,
    pub(in crate::app) value: H2ParameterValue,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct H2Pass {
    pub(in crate::app) path: String,
    pub(in crate::app) implementation: usize,
    pub(in crate::app) vertex_shader: String,
    /// `None` draws without blending.
    pub(in crate::app) blend: Option<H2Blend>,
    pub(in crate::app) depth: H2Depth,
    pub(in crate::app) channels: String,
    pub(in crate::app) alpha_test: Option<AlphaTest>,
    /// One per texture stage, in stage order.
    pub(in crate::app) textures: Vec<H2PassTexture>,
    pub(in crate::app) parameters: Vec<(String, H2ParameterValue)>,
    pub(in crate::app) vertex_constants: Vec<H2VertexConstant>,
}

impl H2Pass {
    /// The kit's generated source for this pass's implementation.
    pub(in crate::app) fn pixel_shader_file(&self) -> String {
        format!("{}_{}.psh", self.path.replace(['\\', '/'], "_"), self.implementation)
    }
}

/// One level's view of a parameter. The three levels name their fields
/// differently, so each is read into this first.
#[derive(Debug, Clone, Default, PartialEq)]
struct ParameterLevel {
    kind: String,
    bitmap: String,
    value: f32,
    color: [f32; 3],
}

type Parameters = HashMap<String, ParameterLevel>;

/// The game's lookup: the first level holding the name, except that a bitmap
/// with no tag falls through to the next level. A level only answers for its
/// own kind of parameter.
fn resolve_parameter(levels: [&Parameters; 3], name: &str, kind: &str) -> H2ParameterValue {
    for level in levels {
        let Some(found) = level.get(name).filter(|found| found.kind == kind) else {
            continue;
        };
        match kind {
            "bitmap" if found.bitmap.is_empty() => continue,
            "bitmap" => return H2ParameterValue::Bitmap(found.bitmap.clone()),
            "value" => return H2ParameterValue::Value(found.value),
            "color" => return H2ParameterValue::Color(found.color),
            _ => break,
        }
    }
    H2ParameterValue::Missing
}

fn rgb(parameter: TagStruct<'_>, name: &str) -> [f32; 3] {
    let color = parameter.read_rgb(name);
    [color.red, color.green, color.blue]
}

fn elements<'a>(holder: TagStruct<'a>, name: &str) -> impl Iterator<Item = TagStruct<'a>> {
    let block = holder.field_path(name).and_then(|field| field.as_block());
    (0..block.as_ref().map_or(0, |block| block.len()))
        .filter_map(move |index| block.as_ref()?.element(index))
}

fn insert_level(
    into: &mut Parameters,
    parameter: TagStruct<'_>,
    bitmap: &str,
    value: &str,
    color: &str,
) {
    let Some(name) = parameter.read_string_id("name") else {
        return;
    };
    into.entry(name).or_insert_with(|| ParameterLevel {
        kind: parameter.read_enum_name("type").unwrap_or_default(),
        bitmap: parameter.read_tag_ref_path(bitmap).unwrap_or_default(),
        value: parameter.read_real(value).unwrap_or(0.0),
        color: rgb(parameter, color),
    });
}

fn shader_parameters(shader: TagStruct<'_>) -> Parameters {
    let mut parameters = Parameters::new();
    for parameter in elements(shader, "parameters") {
        insert_level(&mut parameters, parameter, "bitmap", "const value", "const color");
    }
    parameters
}

fn template_parameters(template: TagStruct<'_>) -> Parameters {
    let mut parameters = Parameters::new();
    for category in elements(template, "categories") {
        for parameter in elements(category, "parameters") {
            insert_level(
                &mut parameters,
                parameter,
                "default bitmap",
                "default const value",
                "default const color",
            );
        }
    }
    parameters
}

fn read_alpha_test(implementation: TagStruct<'_>) -> Option<AlphaTest> {
    let state = implementation.descend("alpha-test state[0]")?;
    let enabled = state
        .read_flag_names("flags")?
        .iter()
        .any(|(_, name)| name == "alpha-test enabled");
    if !enabled {
        return None;
    }
    let compare = match state.read_enum_name("alpha compare function")?.as_str() {
        "greater" | "greater or equal" => AlphaCompare::Greater,
        "less" | "less or equal" => AlphaCompare::Less,
        "not equal" => AlphaCompare::NotEqual,
        _ => return None,
    };
    let reference = state.read_int_any("alpha-test ref")? as f32 / 255.0;
    Some(AlphaTest { compare, reference })
}

fn named_blend(mode: &str) -> Option<(H2BlendFactor, H2BlendFactor)> {
    use H2BlendFactor::*;
    Some(match mode {
        "add" => (One, One),
        "multiply" => (DstColor, Zero),
        "add src times dstalpha" => (DstAlpha, One),
        "add src times srcalpha" => (SrcAlpha, One),
        "add dst times srcalpha inverse" => (One, OneMinusSrcAlpha),
        "alpha blend" => (SrcAlpha, OneMinusSrcAlpha),
        _ => return None,
    })
}

fn blend_factor(name: &str) -> Option<H2BlendFactor> {
    use H2BlendFactor::*;
    Some(match name {
        "zero" => Zero,
        "one" => One,
        "srccolor" => SrcColor,
        "srccolor inverse" => OneMinusSrcColor,
        "srcalpha" => SrcAlpha,
        "srcalpha inverse" => OneMinusSrcAlpha,
        "dstcolor" => DstColor,
        "dstcolor inverse" => OneMinusDstColor,
        "dstalpha" => DstAlpha,
        "dstalpha inverse" => OneMinusDstAlpha,
        "srcalpha saturate" => SrcAlphaSaturate,
        "constant color" => ConstantColor,
        "constant color inverse" => OneMinusConstantColor,
        "constant alpha" => ConstantAlpha,
        "constant alpha inverse" => OneMinusConstantAlpha,
        _ => return None,
    })
}

/// The game draws a `logic-op` function, or `custom` with no state, unblended.
/// The signed functions are its add and subtract.
fn blend_equation(name: &str) -> Option<H2BlendEquation> {
    use H2BlendEquation::*;
    Some(match name {
        "add" | "add signed" => Add,
        "subtract" | "reverse subtract signed" => Subtract,
        "reverse subtract" => ReverseSubtract,
        "min" => Min,
        "max" => Max,
        _ => return None,
    })
}

fn read_blend(implementation: TagStruct<'_>) -> Option<H2Blend> {
    let mode = implementation.read_enum_name("alpha-blend")?;
    if mode != "custom" {
        let (source, destination) = named_blend(&mode)?;
        return Some(H2Blend {
            equation: H2BlendEquation::Add,
            source,
            destination,
            constant: [0; 4],
        });
    }
    let state = implementation.descend("alpha-blend state[0]")?;
    let color = match state.field("blend color").and_then(|field| field.value()) {
        Some(TagFieldData::ArgbColor(color)) => color.0,
        _ => 0,
    };
    Some(H2Blend {
        equation: blend_equation(&state.read_enum_name("blend function")?)?,
        source: blend_factor(&state.read_enum_name("blend src factor")?)?,
        destination: blend_factor(&state.read_enum_name("blend dst factor")?)?,
        constant: color.to_be_bytes(),
    })
}

fn compare(name: &str) -> Option<H2Compare> {
    use H2Compare::*;
    Some(match name {
        "never" => Never,
        "less" => Less,
        "equal" => Equal,
        "less or equal" => LessOrEqual,
        "greater" => Greater,
        "not equal" => NotEqual,
        "greater or equal" => GreaterOrEqual,
        "always" => Always,
        _ => return None,
    })
}

fn read_depth(implementation: TagStruct<'_>) -> H2Depth {
    const OFF: H2Depth = H2Depth { compare: None, write: false };
    let test = |compare, write| H2Depth { compare: Some(compare), write };
    match implementation.read_enum_name("depth").as_deref() {
        Some("default opaque") => test(H2Compare::Equal, false),
        Some("default opaque write") => test(H2Compare::LessOrEqual, true),
        Some("default transparent") => test(H2Compare::LessOrEqual, false),
        Some("custom") => implementation
            .descend("depth state[0]")
            .and_then(|state| {
                let function = state.read_enum_name("depth compare function")?;
                let write = state
                    .read_flag_names("flags")?
                    .iter()
                    .any(|(_, name)| name == "depth write");
                Some(test(compare(&function)?, write))
            })
            .unwrap_or(OFF),
        _ => OFF,
    }
}

fn read_pass(
    path: &str,
    pass: TagStruct<'_>,
    index: usize,
    shader: &Parameters,
    template: &Parameters,
) -> Result<H2Pass, String> {
    let implementation = pass
        .descend(&format!("implementations[{index}]"))
        .ok_or_else(|| format!("{path} has no implementation {index}"))?;

    let mut own = Parameters::new();
    let mut externs: HashMap<String, String> = HashMap::new();
    for parameter in elements(pass, "parameters") {
        insert_level(
            &mut own,
            parameter,
            "default bitmap",
            "default const value",
            "default const color",
        );
        if let (Some(name), Some(source)) =
            (parameter.read_string_id("name"), parameter.read_enum_name("source extern"))
            && source != NO_EXTERN
        {
            externs.insert(name, source);
        }
    }
    let levels = [shader, template, &own];

    let mut parameters: Vec<_> = own
        .iter()
        .map(|(name, level)| {
            let value = match externs.get(name) {
                Some(source) => H2ParameterValue::Extern(source.clone()),
                None => resolve_parameter(levels, name, &level.kind),
            };
            (name.clone(), value)
        })
        .collect();
    parameters.sort_by(|a, b| a.0.cmp(&b.0));

    let textures = elements(implementation, "textures")
        .map(|texture| {
            let parameter = texture.read_string_id("source parameter").unwrap_or_default();
            let source = texture.read_enum_name("source extern").unwrap_or_default();
            let value = if !source.is_empty() && source != NO_EXTERN {
                H2ParameterValue::Extern(source)
            } else {
                resolve_parameter(levels, &parameter, "bitmap")
            };
            H2PassTexture {
                parameter,
                mode: texture.read_enum_name("mode").unwrap_or_default(),
                value,
            }
        })
        .collect();

    Ok(H2Pass {
        path: path.to_owned(),
        implementation: index,
        vertex_shader: implementation.read_tag_ref_path("vertex shader").unwrap_or_default(),
        blend: read_blend(implementation),
        depth: read_depth(implementation),
        channels: implementation.read_enum_name("channels").unwrap_or_default(),
        alpha_test: read_alpha_test(implementation),
        textures,
        parameters,
        vertex_constants: elements(implementation, "vs constants")
            .map(|constant| H2VertexConstant {
                parameter: constant.read_string_id("source parameter").unwrap_or_default(),
                bank: constant.read_enum_name("register bank").unwrap_or_default(),
                register: constant.read_int_any("register index").unwrap_or(0) as i32,
                components: constant.read_enum_name("component mask").unwrap_or_default(),
            })
            .collect(),
    })
}

/// A decoded map of any shape a pass samples.
#[derive(Debug, Clone)]
pub(crate) enum H2Image {
    Flat(TextureImage),
    /// Six RGBA faces of `size` texels a side, in +X -X +Y -Y +Z -Z order.
    Cube { size: usize, faces: [Vec<u8>; 6] },
    Volume { width: usize, height: usize, depth: usize, rgba: Vec<u8> },
}

/// One texture unit of a pass. A `None` image draws the unit's placeholder.
#[derive(Debug, Clone)]
pub(crate) struct H2DrawTexture {
    pub(crate) slot: u32,
    pub(crate) kind: SamplerKind,
    pub(crate) image: Option<std::sync::Arc<H2Image>>,
    /// Bound to the generated normalization cube instead of an image.
    pub(crate) normalization: bool,
    /// Bound to the preview's texaccum target instead of an image.
    pub(crate) texaccum: bool,
    /// Bound to a copy of the frame so far instead of an image.
    pub(crate) frame_copy: bool,
    /// Bound to a white volume. A default light's radius puts a model at the
    /// middle of its falloff, which reads 1.
    pub(crate) falloff: bool,
}

/// A pass ready for the renderer: resolved, translated and decoded on the
/// worker, with no GL state yet.
#[derive(Debug, Clone)]
pub(crate) struct H2DrawPass {
    pub(crate) stage: H2VertexStage,
    pub(crate) fragment: String,
    pub(crate) blend: Option<H2Blend>,
    pub(crate) depth: H2Depth,
    pub(crate) channels: [bool; 4],
    pub(crate) textures: Vec<H2DrawTexture>,
    /// Pixel constant registers, by register index.
    pub(crate) constants: Vec<(u32, [f32; 4])>,
    pub(crate) texture_rows: [[f32; 4]; H2_TEXTURE_ROWS],
    pub(crate) tint_facing: [f32; 4],
    pub(crate) tint_glancing: [f32; 4],
    /// Constant components filled from the object's change colors at draw
    /// time: register, component, channel.
    pub(crate) change_colors: Vec<(u32, usize, usize)>,
    /// Constant components filled from the current light: register, component.
    pub(crate) light_colors: Vec<(u32, usize)>,
    /// The vectors a lightmap pass lights by. Most passes keep them in the
    /// tag, each as its pixel shader expects: some negate them.
    pub(crate) light_direct: [f32; 3],
    pub(crate) light_indirect: [f32; 3],
}

/// One initial permutation of a change color channel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct H2ChangePermutation {
    pub(crate) weight: f32,
    pub(crate) lower: [f32; 3],
    pub(crate) upper: [f32; 3],
    pub(crate) variant: String,
}

/// An object's four change color channels.
pub(crate) type H2ChangeColors = [Vec<H2ChangePermutation>; 4];

/// The object's change colors. Halo 2 nests each group's parent as a struct,
/// so `object` sits at the end of the chain of leading structs.
pub(in crate::app) fn read_h2_change_colors(object: &TagFile) -> H2ChangeColors {
    let mut holder = object.root();
    let block = loop {
        if holder.field("change colors").is_some() {
            break Some(holder);
        }
        let Some(parent) = holder.fields_all().find_map(|field| field.as_struct()) else {
            break None;
        };
        holder = parent;
    };
    let mut colors = H2ChangeColors::default();
    let Some(block) = block else {
        return colors;
    };
    for (channel, entry) in elements(block, "change colors").take(4).enumerate() {
        colors[channel] = elements(entry, "initial permutations")
            .map(|permutation| H2ChangePermutation {
                weight: permutation.read_real("weight").unwrap_or(0.0),
                lower: rgb(permutation, "color lower bound"),
                upper: rgb(permutation, "color upper bound"),
                variant: permutation.read_string_id("variant name").unwrap_or_default(),
            })
            .collect();
    }
    colors
}

/// One channel's color for a model variant. The game picks by a per-object
/// position; a preview has none, so this takes the permutation at the middle
/// of the weights and the middle of its range. White with nothing allowed, as
/// the game does.
fn change_color_for(permutations: &[H2ChangePermutation], variant: &str) -> [f32; 3] {
    let allowed: Vec<_> = permutations
        .iter()
        .filter(|entry| entry.weight > 0.0 && (entry.variant.is_empty() || entry.variant == variant))
        .collect();
    let total: f32 = allowed.iter().map(|entry| entry.weight).sum();
    let mut remaining = total / 2.0;
    for entry in allowed {
        remaining -= entry.weight;
        if remaining <= 0.0 {
            return std::array::from_fn(|axis| (entry.lower[axis] + entry.upper[axis]) / 2.0);
        }
    }
    [1.0; 3]
}

pub(crate) fn h2_change_colors_for(colors: &H2ChangeColors, variant: &str) -> [[f32; 3]; 4] {
    std::array::from_fn(|channel| change_color_for(&colors[channel], variant))
}

/// Frame textures a pass needs that a preview has no way to produce. The
/// texaccum target counts unless this material's own texaccum pass fills it.
pub(in crate::app) fn h2_pass_needs_frame_texture(pass: &H2Pass, has_texaccum: bool) -> bool {
    pass.textures.iter().any(|texture| {
        matches!(
            &texture.value,
            H2ParameterValue::Extern(name)
                if name != VECTOR_NORMALIZATION
                    && name != LIGHT_FALLOFF
                    && name != FRAME_COPY
                    && name != ACTIVE_CAMO_BUMP
                    && !CAMERA_FEEDS.contains(&name.as_str())
                    && !(has_texaccum && name == TEXACCUM_TARGET)
        )
    })
}

/// One face of the cube that normalizes a vector by sampling it, in the GL
/// face order: each texel holds its own direction, packed to 0..1.
pub(crate) fn h2_normalization_face(face: usize, size: usize) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let direction = match face {
                0 => [1.0, -v, -u],
                1 => [-1.0, -v, u],
                2 => [u, 1.0, v],
                3 => [u, -1.0, -v],
                4 => [u, -v, 1.0],
                _ => [-u, -v, -1.0],
            };
            let length = direction.iter().map(|axis: &f32| axis * axis).sum::<f32>().sqrt();
            for axis in direction {
                rgba.push(((axis / length * 0.5 + 0.5) * 255.0).round() as u8);
            }
            rgba.push(255);
        }
    }
    rgba
}

/// Which channel a pass parameter's extern reads, if it is a change color.
fn change_color_channel(source: &str) -> Option<usize> {
    match source {
        "OBJECT change color primary" => Some(0),
        "OBJECT change color secondary" => Some(1),
        "OBJECT change color tertiary" => Some(2),
        "OBJECT change color quaternary" => Some(3),
        _ => None,
    }
}

/// The GL version every translated shader and vertex stage declares.
pub(crate) const H2_GLSL_VERSION: &str = "#version 330\n";

/// What a preview has in place of a level's lighting: the colors the
/// no-lightmap passes hold as literals, so both paths light a surface alike.
/// The vectors are the game's `default_object_lighting`.
const DEFAULT_AMBIENT: [f32; 4] = [0.0627, 0.0627, 0.0627, 1.0];
const DEFAULT_DIRECT_COLOR: [f32; 4] = [0.502, 0.490, 0.471, 1.0];
/// Vertex registers holding a lightmap pass's light vectors.
const LIGHT_DIRECT_REGISTER: i32 = 8;
const LIGHT_INDIRECT_REGISTER: i32 = 9;
const DEFAULT_DIRECT_VECTOR: [f32; 4] = [-0.577, -0.577, -0.577, 1.0];
const DEFAULT_INDIRECT_COLOR: [f32; 4] = [0.373, 0.353, 0.333, 1.0];
const DEFAULT_INDIRECT_VECTOR: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
/// Reflection vectors in the game carry a scale of about 2.
pub(crate) const H2_REFLECTION_SCALE: f32 = 2.0;

fn extern_value(name: &str, specular_brightness: f32) -> [f32; 4] {
    let [r, g, b, a] = DEFAULT_DIRECT_COLOR;
    match name {
        // The luminance of ambient plus direct; see `rasterizer_get_value_extern`.
        "LIGHTMAP object env brightness" => {
            let [ambient, ..] = DEFAULT_AMBIENT;
            let luminance = 0.299 * (r + ambient) + 0.587 * (g + ambient) + 0.114 * (b + ambient);
            [luminance.clamp(0.0, 1.0); 4]
        }
        "LIGHTMAP object specular color" => {
            [r * specular_brightness, g * specular_brightness, b * specular_brightness, a]
        }
        "LIGHTMAP object specular color times one half" => {
            let half = specular_brightness * 0.5;
            [r * half, g * half, b * half, a]
        }
        "LIGHTMAP object ambient color" => DEFAULT_AMBIENT,
        "LIGHTMAP object direct color" => DEFAULT_DIRECT_COLOR,
        "LIGHTMAP object direct vector" => DEFAULT_DIRECT_VECTOR,
        "LIGHTMAP object indirect color" => DEFAULT_INDIRECT_COLOR,
        "LIGHTMAP object indirect vector" => DEFAULT_INDIRECT_VECTOR,
        // White is the game's own answer when an object allows no colors.
        _ if name.starts_with("OBJECT change color") => [1.0; 4],
        // Only drawn at all with camo fully on.
        "OBJECT active camo amount" => [1.0; 4],
        _ => [0.0, 0.0, 0.0, 1.0],
    }
}

fn parameter_vector(value: &H2ParameterValue, specular_brightness: f32) -> [f32; 4] {
    match value {
        H2ParameterValue::Value(value) => [*value; 4],
        H2ParameterValue::Color([r, g, b]) => [*r, *g, *b, 1.0],
        H2ParameterValue::Extern(name) => extern_value(name, specular_brightness),
        H2ParameterValue::Bitmap(_) | H2ParameterValue::Missing => [0.0, 0.0, 0.0, 1.0],
    }
}

fn channel_mask(channels: &str) -> [bool; 4] {
    match channels {
        "color only" => [true, true, true, false],
        "alpha only" => [false, false, false, true],
        _ => [true; 4],
    }
}

/// Rows of an identity 2D transform: `x` in even registers, `y` in odd ones.
fn identity_rows() -> [[f32; 4]; H2_TEXTURE_ROWS] {
    std::array::from_fn(|row| if row % 2 == 0 { [1.0, 0.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0, 0.0] })
}

/// Turn a resolved pass into what the renderer draws. `bitmap` decodes a map
/// by path in the shape its sampler wants; `scale` gives a parameter's tiling.
pub(in crate::app) fn prepare_draw_pass(
    pass: &H2Pass,
    psh: &str,
    cases: HashMap<String, i32>,
    specular_brightness: f32,
    mut bitmap: impl FnMut(&str, SamplerKind) -> Option<std::sync::Arc<H2Image>>,
    scale: impl Fn(&str) -> [f32; 2],
) -> Result<H2DrawPass, String> {
    let stage = H2VertexStage::from_path(&pass.vertex_shader)
        .ok_or_else(|| format!("No vertex stage for {}", pass.vertex_shader))?;
    let options = PixelShaderOptions { cases, alpha_test: pass.alpha_test };
    let translated = translate(psh, H2_GLSL_VERSION, &options)?;
    let lookup = |name: &str| {
        pass.parameters.iter().find(|(parameter, _)| parameter == name).map(|(_, value)| value)
    };

    let mut constants: Vec<(u32, [f32; 4])> = Vec::new();
    let mut change_colors = Vec::new();
    let mut light_colors = Vec::new();
    for binding in &translated.constants {
        let value = lookup(&binding.source)
            .map_or([0.0, 0.0, 0.0, 1.0], |value| parameter_vector(value, specular_brightness));
        let at = constants.iter().position(|(register, _)| *register == binding.register);
        let at = at.unwrap_or_else(|| {
            constants.push((binding.register, [0.0; 4]));
            constants.len() - 1
        });
        for component in binding.components.chars() {
            let index = match component {
                'r' | 'x' => 0,
                'g' | 'y' => 1,
                'b' | 'z' => 2,
                _ => 3,
            };
            constants[at].1[index] = value[index];
            if let Some(H2ParameterValue::Extern(source)) = lookup(&binding.source)
                && let Some(channel) = change_color_channel(source)
                && index < 3
            {
                change_colors.push((binding.register, index, channel));
            }
            if let Some(H2ParameterValue::Extern(source)) = lookup(&binding.source)
                && matches!(source.as_str(), "LIGHT diffuse color" | "LIGHT specular color")
                && index < 3
            {
                light_colors.push((binding.register, index));
            }
        }
    }

    let textures = translated
        .samplers
        .iter()
        .map(|sampler| {
            let value = pass.textures.get(sampler.slot as usize).map(|texture| &texture.value);
            let image = match value {
                Some(H2ParameterValue::Bitmap(path)) => bitmap(path, sampler.kind),
                Some(H2ParameterValue::Extern(name)) if CAMERA_FEEDS.contains(&name.as_str()) => {
                    Some(std::sync::Arc::new(H2Image::Flat(TextureImage {
                        rgba: vec![0, 0, 0, 255],
                        width: 1,
                        height: 1,
                        repeat_x: false,
                        repeat_y: false,
                        scale: [1.0, 1.0],
                    })))
                }
                Some(H2ParameterValue::Extern(name)) if name == ACTIVE_CAMO_BUMP => {
                    match lookup("bump_map") {
                        Some(H2ParameterValue::Bitmap(path)) => bitmap(path, sampler.kind),
                        _ => None,
                    }
                }
                _ => None,
            };
            let normalization = matches!(
                value,
                Some(H2ParameterValue::Extern(name)) if name == VECTOR_NORMALIZATION
            );
            let texaccum = matches!(
                value,
                Some(H2ParameterValue::Extern(name)) if name == TEXACCUM_TARGET
            );
            let falloff = matches!(
                value,
                Some(H2ParameterValue::Extern(name)) if name == LIGHT_FALLOFF
            );
            H2DrawTexture {
                slot: sampler.slot,
                kind: sampler.kind,
                image,
                normalization,
                texaccum,
                frame_copy: matches!(
                    value,
                    Some(H2ParameterValue::Extern(name))
                        if name == FRAME_COPY || name == WATER_REFRACTION
                ),
                falloff,
            }
        })
        .collect();

    let mut texture_rows = if stage.uses_scale_rows() {
        std::array::from_fn(|row| {
            if (row == 3 && stage.is_lightmap()) || (row == 2 && stage == H2VertexStage::Texaccum) {
                [1.0; 4]
            } else {
                [1.0, 1.0, 0.0, 0.0]
            }
        })
    } else {
        identity_rows()
    };
    let (mut tint_facing, mut tint_glancing) = ([1.0; 4], [1.0; 4]);
    let [x, y, z, _] = DEFAULT_DIRECT_VECTOR;
    let mut light_direct = [x, y, z];
    let [x, y, z, _] = DEFAULT_INDIRECT_VECTOR;
    let mut light_indirect = [x, y, z];
    for constant in &pass.vertex_constants {
        if stage.is_lightmap()
            && matches!(constant.register, LIGHT_DIRECT_REGISTER | LIGHT_INDIRECT_REGISTER)
        {
            let vector = if constant.register == LIGHT_DIRECT_REGISTER {
                &mut light_direct
            } else {
                &mut light_indirect
            };
            let value = lookup(&constant.parameter)
                .map_or([0.0; 4], |value| parameter_vector(value, specular_brightness));
            match constant.components.as_str() {
                "x (value)" => vector[0] = value[0],
                "y (value)" => vector[1] = value[1],
                "z (value)" => vector[2] = value[2],
                _ => *vector = [value[0], value[1], value[2]],
            }
            continue;
        }
        if constant.bank.starts_with("Vn") {
            let value = lookup(&constant.parameter)
                .map_or([1.0; 4], |value| parameter_vector(value, specular_brightness));
            match constant.register {
                10 => tint_facing = value,
                11 => tint_glancing = value,
                _ => {}
            }
            continue;
        }
        let Some(row) = texture_rows.get_mut(constant.register.max(0) as usize) else {
            continue;
        };
        if stage.takes_row_values() {
            let channel = ["x (value)", "y (value)", "z (value)", "w (value)"]
                .iter()
                .position(|name| constant.components == *name);
            if let Some(channel) = channel {
                let value = lookup(&constant.parameter)
                    .map_or(0.0, |value| parameter_vector(value, specular_brightness)[channel]);
                row[channel] = value;
                continue;
            }
        }
        let [x, y] = scale(&constant.parameter);
        if constant.components.contains("row1") {
            *row = [x, 0.0, 0.0, 0.0];
        } else if constant.components.contains("row2") {
            *row = [0.0, y, 0.0, 0.0];
        } else if constant.components.contains("simple xform") {
            *row = [x, y, 0.0, 0.0];
        } else if constant.components.starts_with("xy (2D scale") {
            row[0] = x;
            row[1] = y;
        } else if constant.components.starts_with("zw (2D scale") {
            row[2] = x;
            row[3] = y;
        }
    }

    Ok(H2DrawPass {
        stage,
        fragment: translated.glsl,
        blend: pass.blend,
        depth: pass.depth,
        channels: channel_mask(&pass.channels),
        textures,
        constants,
        texture_rows,
        tint_facing,
        tint_glancing,
        change_colors,
        light_colors,
        light_direct,
        light_indirect,
    })
}

/// The passes of one layer of `shader`, in template order. `load` reads a tag
/// by path, extension and group; a shader with no template has no passes.
pub(in crate::app) fn resolve_h2_layer_passes(
    shader: &TagFile,
    mut load: impl FnMut(&str, &str, &[u8; 4]) -> Option<TagFile>,
    layer: &str,
    implementation: usize,
) -> Result<Vec<H2Pass>, String> {
    let root = shader.root();
    let Some(template_path) =
        root.read_tag_ref_path("template").filter(|path| !path.is_empty())
    else {
        return Ok(Vec::new());
    };
    let template = load(&template_path, "shader_template", b"stem")
        .ok_or_else(|| format!("Cannot read {template_path}"))?;
    let template_root = template.root();
    let shader_levels = shader_parameters(root);
    let template_levels = template_parameters(template_root);

    let mut passes = Vec::new();
    for entry in elements(template_root, "LODs[0]/pass") {
        if entry.read_enum_name("layer").as_deref() != Some(layer) {
            continue;
        }
        let path = entry.read_tag_ref_path("pass").unwrap_or_default();
        let pass = load(&path, "shader_pass", b"spas")
            .ok_or_else(|| format!("Cannot read {path}"))?;
        passes.push(read_pass(
            &path,
            pass.root(),
            implementation,
            &shader_levels,
            &template_levels,
        )?);
    }
    Ok(passes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lights_choose_between_the_game_s_two_lightmap_paths() {
        assert_eq!(H2Layer::Lightmap.implementation(), 3);
        assert_eq!(H2Layer::ObjectLightmap.implementation(), 2);
        assert_eq!(H2Layer::ObjectLightmap.tag_name(), H2Layer::Lightmap.tag_name());
    }

    #[test]
    fn object_env_brightness_is_the_luminance_of_the_lighting() {
        let [brightness, ..] = extern_value("LIGHTMAP object env brightness", 1.0);
        assert!((brightness - 0.554).abs() < 0.001, "{brightness}");
    }

    fn level(entries: &[(&str, &str, &str, f32)]) -> Parameters {
        entries
            .iter()
            .map(|(name, kind, bitmap, value)| {
                ((*name).to_owned(), ParameterLevel {
                    kind: (*kind).to_owned(),
                    bitmap: (*bitmap).to_owned(),
                    value: *value,
                    color: [*value; 3],
                })
            })
            .collect()
    }

    #[test]
    fn the_shader_wins_then_the_template_then_the_pass() {
        let shader = level(&[("a", "value", "", 1.0)]);
        let template = level(&[("a", "value", "", 2.0), ("b", "value", "", 2.0)]);
        let pass = level(&[("a", "value", "", 3.0), ("b", "value", "", 3.0), ("c", "value", "", 3.0)]);
        let levels = [&shader, &template, &pass];
        for (name, expected) in [("a", 1.0), ("b", 2.0), ("c", 3.0)] {
            assert_eq!(resolve_parameter(levels, name, "value"), H2ParameterValue::Value(expected));
        }
        assert_eq!(resolve_parameter(levels, "d", "value"), H2ParameterValue::Missing);
    }

    #[test]
    fn a_bitmap_with_no_tag_falls_through_and_kinds_do_not_cross() {
        let shader = level(&[("map", "bitmap", "", 0.0)]);
        let template = level(&[("map", "value", "", 5.0)]);
        let pass = level(&[("map", "bitmap", "default", 0.0)]);
        let levels = [&shader, &template, &pass];
        assert_eq!(
            resolve_parameter(levels, "map", "bitmap"),
            H2ParameterValue::Bitmap("default".to_owned())
        );
    }

    #[test]
    fn named_blends_match_the_games_tables() {
        use H2BlendFactor::*;
        assert_eq!(named_blend("add"), Some((One, One)));
        assert_eq!(named_blend("multiply"), Some((DstColor, Zero)));
        assert_eq!(named_blend("add src times dstalpha"), Some((DstAlpha, One)));
        assert_eq!(named_blend("add src times srcalpha"), Some((SrcAlpha, One)));
        assert_eq!(named_blend("add dst times srcalpha inverse"), Some((One, OneMinusSrcAlpha)));
        assert_eq!(named_blend("alpha blend"), Some((SrcAlpha, OneMinusSrcAlpha)));
        assert_eq!(named_blend("disabled"), None);
        assert_eq!(blend_equation("logic-op"), None);
    }

    #[test]
    fn change_colors_filter_by_variant_and_fall_back_to_white() {
        let entry = |weight, value: f32, variant: &str| H2ChangePermutation {
            weight,
            lower: [value; 3],
            upper: [value + 0.25; 3],
            variant: variant.to_owned(),
        };
        let list = [entry(1.0, 0.25, "minor"), entry(1.0, 0.5, "major"), entry(0.0, 0.75, "")];
        assert_eq!(change_color_for(&list, "minor"), [0.375; 3]);
        assert_eq!(change_color_for(&list, "major"), [0.625; 3]);
        assert_eq!(change_color_for(&list, "ultra"), [1.0; 3], "nothing allowed");
        assert_eq!(change_color_for(&[], "minor"), [1.0; 3]);
        let shared = [entry(1.0, 0.0, ""), entry(2.0, 0.5, ""), entry(1.0, 0.75, "")];
        assert_eq!(change_color_for(&shared, "any"), [0.625; 3], "the middle of the weights");
    }

    #[test]
    fn rows_and_constants_come_from_the_pass() {
        let pass = H2Pass {
            path: String::new(),
            implementation: 0,
            vertex_shader: "rasterizer\\vertex_shaders_dx9\\transparent_generic_default".to_owned(),
            blend: None,
            depth: H2Depth { compare: None, write: false },
            channels: "color only".to_owned(),
            alpha_test: None,
            textures: Vec::new(),
            parameters: vec![
                ("tint".to_owned(), H2ParameterValue::Color([0.25, 0.5, 0.75])),
                ("glow".to_owned(), H2ParameterValue::Value(0.5)),
                ("primary".to_owned(), H2ParameterValue::Extern("OBJECT change color primary".to_owned())),
            ],
            vertex_constants: vec![
                H2VertexConstant { parameter: "map".to_owned(), bank: "Cn [0,12]".to_owned(), register: 2, components: "xyw row1 (2D affine xform)".to_owned() },
                H2VertexConstant { parameter: "map".to_owned(), bank: "Cn [0,12]".to_owned(), register: 3, components: "xyw row2 (2D affine xform)".to_owned() },
                H2VertexConstant { parameter: "tint".to_owned(), bank: "Vn [0,15]".to_owned(), register: 10, components: "xyzw".to_owned() },
            ],
        };
        let psh = "#const C1.rgb= $tint\n#const C1.a= $glow\n#const C2.rgb= $primary\n\
                   float4 c1 : register(C1);\nfloat4 c2 : register(C2);\n\
                   float4 main(\n)\n{\n    return c1 * c2;\n}\n";
        let drawn = prepare_draw_pass(&pass, psh, HashMap::new(), 1.0, |_, _| None, |_| [4.0, 8.0]).unwrap();
        assert_eq!(drawn.stage, H2VertexStage::Default);
        assert_eq!(drawn.channels, [true, true, true, false]);
        assert_eq!(drawn.constants, [(1, [0.25, 0.5, 0.75, 0.5]), (2, [1.0, 1.0, 1.0, 0.0])]);
        assert_eq!(drawn.texture_rows[2], [4.0, 0.0, 0.0, 0.0]);
        assert_eq!(drawn.texture_rows[3], [0.0, 8.0, 0.0, 0.0]);
        assert_eq!(drawn.texture_rows[0], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(drawn.tint_facing, [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(drawn.tint_glancing, [1.0; 4]);
        assert_eq!(drawn.change_colors, [(2, 0, 0), (2, 1, 0), (2, 2, 0)]);
    }

    #[test]
    fn normalization_faces_hold_their_own_direction() {
        let center = |face: usize, size: usize| -> [u8; 3] {
            let rgba = h2_normalization_face(face, size);
            let at = (size / 2 * size + size / 2) * 4;
            [rgba[at], rgba[at + 1], rgba[at + 2]]
        };
        assert!(center(0, 33)[0] > 250, "+X");
        assert!(center(3, 33)[1] < 5, "-Y");
        assert!(center(4, 33)[2] > 250, "+Z");
        assert!(center(5, 33)[2] < 5, "-Z");
    }

    #[test]
    fn frame_textures_other_than_normalization_are_refused() {
        let pass = |name: &str| H2Pass {
            path: String::new(),
            implementation: 3,
            vertex_shader: String::new(),
            blend: None,
            depth: H2Depth { compare: None, write: false },
            channels: String::new(),
            alpha_test: None,
            textures: vec![H2PassTexture {
                parameter: String::new(),
                mode: String::new(),
                value: H2ParameterValue::Extern(name.to_owned()),
            }],
            parameters: Vec::new(),
            vertex_constants: Vec::new(),
        };
        assert!(h2_pass_needs_frame_texture(&pass(TEXACCUM_TARGET), false));
        assert!(!h2_pass_needs_frame_texture(&pass(TEXACCUM_TARGET), true));
        assert!(h2_pass_needs_frame_texture(&pass("GLOBAL something unsupplied"), true));
        // A camera feed draws as a dark screen rather than costing the material.
        assert!(!h2_pass_needs_frame_texture(&pass("GLOBAL texture camera"), false));
        assert!(!h2_pass_needs_frame_texture(&pass("FIRST-PERSON scope"), false));
        assert!(!h2_pass_needs_frame_texture(&pass(VECTOR_NORMALIZATION), false));
    }

    #[test]
    fn the_source_file_follows_the_pass_path() {
        let pass = H2Pass {
            path: "shaders\\shader_passes\\transparent\\lit".to_owned(),
            implementation: 0,
            vertex_shader: String::new(),
            blend: None,
            depth: H2Depth { compare: None, write: false },
            channels: String::new(),
            alpha_test: None,
            textures: Vec::new(),
            parameters: Vec::new(),
            vertex_constants: Vec::new(),
        };
        assert_eq!(pass.pixel_shader_file(), "shaders_shader_passes_transparent_lit_0.psh");
    }

    /// With `BLAM_TEST_H2EK`: a change-color shader fills texaccum on its
    /// light albedo layer, the only thing its diffuse pass has to read.
    #[test]
    fn jackal_legs_diffuse_reads_its_light_albedo() {
        let Ok(root) = std::env::var("BLAM_TEST_H2EK") else {
            eprintln!("skipping: BLAM_TEST_H2EK is not set");
            return;
        };
        let root = std::path::PathBuf::from(root);
        let definitions = crate::core::bundled::locate_definitions_root();
        let read = |path: &str, extension: &str, group: &[u8; 4]| {
            let file = root.join(format!("{}.{extension}", path.replace('\\', "/")));
            crate::core::source::read_tag_at_path(
                &file,
                Some(crate::core::game::GameId::Halo2),
                Some(&definitions),
                u32::from_be_bytes(*group),
            )
            .ok()
        };
        let shader = read("objects\\characters\\jackal\\shaders\\jackal_legs", "shader", b"shad").unwrap();
        let layer = |layer: H2Layer| {
            resolve_h2_layer_passes(&shader, read, layer.tag_name(), layer.implementation()).unwrap()
        };
        assert!(layer(H2Layer::Texaccum).is_empty());
        let albedo = layer(H2Layer::LightAlbedo);
        assert_eq!(albedo.len(), 1);
        assert_eq!(H2VertexStage::from_path(&albedo[0].vertex_shader), Some(H2VertexStage::Lightmap));
        let diffuse = layer(H2Layer::LightDiffuse);
        assert_eq!(diffuse.len(), 1);
        assert!(h2_pass_needs_frame_texture(&diffuse[0], false));
        assert!(!h2_pass_needs_frame_texture(&diffuse[0], true));
    }

    /// With `BLAM_TEST_H2EK`: the brute hair that drew untextured.
    #[test]
    fn brute_hair_resolves_from_the_kit() {
        let Ok(root) = std::env::var("BLAM_TEST_H2EK") else {
            eprintln!("skipping: BLAM_TEST_H2EK is not set");
            return;
        };
        let root = std::path::PathBuf::from(root);
        let definitions = crate::core::bundled::locate_definitions_root();
        let read = |path: &str, extension: &str, group: &[u8; 4]| {
            let file = root.join(format!("{}.{extension}", path.replace('\\', "/")));
            crate::core::source::read_tag_at_path(
                &file,
                Some(crate::core::game::GameId::Halo2),
                Some(&definitions),
                u32::from_be_bytes(*group),
            )
            .ok()
        };
        let shader = read("objects\\characters\\brute\\shaders\\brute_hair", "shader", b"shad").unwrap();
        let passes = resolve_h2_layer_passes(&shader, read, H2Layer::Transparent.tag_name(), 0).unwrap();
        assert_eq!(passes.len(), 1);
        let pass = &passes[0];
        let blend = pass.blend.unwrap();
        assert_eq!(blend.source, H2BlendFactor::One);
        assert_eq!(blend.destination, H2BlendFactor::OneMinusSrcAlpha);
        assert_eq!(pass.depth, H2Depth { compare: Some(H2Compare::LessOrEqual), write: false });
        assert_eq!(pass.vertex_constants.len(), 4);
        assert_eq!(pass.vertex_constants[2].parameter, "change_color_map");
        assert_eq!(pass.vertex_constants[2].register, 2);
        assert_eq!(pass.channels, "color only");
        assert_eq!(pass.alpha_test, None);
        assert_eq!(pass.textures.len(), 2);
        assert_eq!(pass.textures[0].parameter, "alpha_blend_map");
        for texture in &pass.textures {
            assert!(
                matches!(&texture.value, H2ParameterValue::Bitmap(path) if path.contains("brute")),
                "{texture:?}"
            );
        }
        let extern_of = |name: &str| {
            pass.parameters.iter().find(|(parameter, _)| parameter == name).map(|(_, value)| value.clone())
        };
        assert_eq!(
            extern_of("change_color_primary"),
            Some(H2ParameterValue::Extern("OBJECT change color primary".to_owned()))
        );
        assert_eq!(
            extern_of("global_light_hack"),
            Some(H2ParameterValue::Extern("LIGHTMAP object direct color".to_owned()))
        );

        let custom = "shaders\\shader_passes\\transparent\\alpha_detail_masked";
        let tag = read(custom, "shader_pass", b"spas").unwrap();
        let blend = read_pass(custom, tag.root(), 0, &Parameters::new(), &Parameters::new())
            .unwrap()
            .blend
            .unwrap();
        assert_eq!(blend.source, H2BlendFactor::ConstantColor);
        assert_eq!(blend.constant, [0, 255, 255, 255], "custom blends carry their constant");

        let file = root.join("rasterizer/pixel_shaders_dx9").join(pass.pixel_shader_file());
        let source = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).into_owned();
        let translated = crate::core::h2_pixel_shader::translate(
            &source,
            "#version 330\n",
            &crate::core::h2_pixel_shader::PixelShaderOptions::default(),
        )
        .unwrap();
        assert_eq!(translated.samplers.len(), 2);

        let drawn = prepare_draw_pass(
            pass,
            &source,
            HashMap::new(),
            1.0,
            |path, _| {
                read(path, "bitmap", b"bitm")?;
                Some(std::sync::Arc::new(H2Image::Flat(TextureImage {
                    rgba: vec![0; 4],
                    width: 1,
                    height: 1,
                    repeat_x: true,
                    repeat_y: true,
                    scale: [1.0, 1.0],
                })))
            },
            |_| [1.0, 1.0],
        )
        .unwrap();
        assert_eq!(drawn.stage, H2VertexStage::from_path(&pass.vertex_shader).unwrap());
        assert!(!drawn.change_colors.is_empty());
        let object = read("objects\\characters\\brute\\brute", "biped", b"bipd").unwrap();
        let colors = read_h2_change_colors(&object);
        assert!(colors.iter().any(|channel| !channel.is_empty()), "the brute has change colors");
        assert_eq!(drawn.textures.len(), 2);
        assert!(drawn.textures.iter().all(|texture| texture.image.is_some()), "both maps decode");
        assert!(!drawn.constants.is_empty());
        for (register, value) in &drawn.constants {
            assert!(value.iter().any(|component| *component != 0.0), "c{register} is unset: {drawn:?}");
        }
    }
}
