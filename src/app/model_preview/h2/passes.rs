//! Halo 2's shader templates read into passes: a shader names a template,
//! whose LODs list passes by layer. Each pass reads its bitmaps, values and
//! colors by parameter name, and the game looks a name up in the shader, then
//! the template, then the pass. A parameter the pass gives a `source extern`
//! is filled at runtime instead. The passes become [`H2DrawPass`]es here too.
//!
//! Only LOD 0 is read. An object draws transparent passes with implementation
//! 0 and lightmap passes with `H2_OBJECT_LIGHTMAP_IMPLEMENTATION`, as the game
//! chooses them for something with no lightmap of its own.

use super::super::functions::object_input;
use std::collections::HashMap;
use std::sync::Arc;
use blam_tags::{TagFieldData, TagFile, TagStruct};
use super::super::dx9::stage::{H2VertexStage, H2_TEXTURE_ROWS};
use super::super::materials::TextureImage;
use crate::app::model_preview::common::fields::has_flag;
use crate::core::h2_pixel_shader::{ConstantBinding, SamplerBinding, 
    AlphaCompare, AlphaTest, PixelShaderOptions, SamplerKind, translate,
};
use super::super::dx9::passes::*;
use super::super::common::image::H2Image;

/// The object function that shows a player's emblem.
const EMBLEM_INPUT: &str = "emblem_active";

const TEXACCUM_TARGET: &str = "GLOBAL target texaccum";

/// What the game copies the frame into, for what looks through a surface.
const FRAME_COPY: &str = "GLOBAL active camo";

const WATER_REFRACTION: &str = "GLOBAL water refraction";

/// What a live camera feeds, which a preview has none of: a dark screen.
const CAMERA_FEEDS: [&str; 2] = ["GLOBAL texture camera", "FIRST-PERSON scope"];

/// The bump of the shader the global one replaced: the shader's own bump map,
/// by another name. It reaches a pass as its `bump_map`, from
/// `prepare_h2_camo_passes`.
const ACTIVE_CAMO_BUMP: &str = "SHADER active camo bump";

const LIGHT_FALLOFF: &str = "LIGHT falloff";

const LIGHT_GEL: &str = "LIGHT gel";

const ACTIVE_CAMO_AMOUNT: &str = "OBJECT active camo amount";

/// The one frame texture a preview can supply itself.
const VECTOR_NORMALIZATION: &str = "GLOBAL vector-normalization";

const NO_EXTERN: &str = "none";

/// The animations of a parameter. The game reads these and never the
/// parameter's constant, so an input a preview has no value for still counts.
fn read_animations(parameter: TagStruct<'_>) -> Vec<H2Animation> {
    elements(parameter, "animation properties")
        .filter_map(|property| {
            let held_input = property
                .read_string_id("input name")
                .filter(|name| !name.is_empty())
                .map(|name| {
                    let rest = object_input(&name);
                    (name, rest)
                });
            let range_input = property
                .read_string_id("range name")
                .filter(|name| !name.is_empty())
                .map(|name| {
                    let rest = object_input(&name);
                    (name, rest)
                });
            let kind = match property.read_enum_option("type")?.as_str() {
                "bitmap scale uniform" => H2AnimationKind::ScaleUniform,
                "bitmap scale x" => H2AnimationKind::ScaleX,
                "bitmap scale y" => H2AnimationKind::ScaleY,
                "bitmap scale z" => H2AnimationKind::ScaleZ,
                "bitmap translation x" => H2AnimationKind::TranslationX,
                "bitmap translation y" => H2AnimationKind::TranslationY,
                "bitmap translation z" => H2AnimationKind::TranslationZ,
                "bitmap rotation angle" => H2AnimationKind::Rotation,
                "value" => H2AnimationKind::Value,
                "color" => H2AnimationKind::Color,
                _ => return None,
            };
            Some(H2Animation {
                kind,
                period: property.read_real("time period").unwrap_or(0.0),
                held_input,
                range_input,
                function: Arc::new(animation_function(&property)?),
            })
        })
        .collect()
}

/// One level's view of a parameter. The three levels name their fields
/// differently, so each is read into this first.
#[derive(Debug, Clone, Default, PartialEq)]
struct ParameterLevel {
    kind: String,
    bitmap: String,
    value: f32,
    color: [f32; 3],
    /// A shader's own: `shader_setup_get_shader_parameters` is all the game reads.
    animations: Vec<H2Animation>,
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
        animations: Vec::new(),
        kind: parameter.read_enum_option("type").unwrap_or_default(),
        bitmap: parameter.read_tag_ref_path(bitmap).unwrap_or_default(),
        value: parameter.read_real(value).unwrap_or(0.0),
        color: rgb(parameter, color),
    });
}

fn shader_parameters(shader: TagStruct<'_>) -> Parameters {
    let mut parameters = Parameters::new();
    let mut hidden = false;
    for parameter in elements(shader, "parameters") {
        insert_level(&mut parameters, parameter, "bitmap", "const value", "const color");
        // Only a player has an emblem, which a preview's object is not.
        if elements(parameter, "animation properties")
            .any(|animation| animation.read_string_id("input name").as_deref() == Some(EMBLEM_INPUT))
            && let Some(level) =
                parameter.read_string_id("name").and_then(|name| parameters.get_mut(&name))
        {
            level.value = 0.0;
            hidden = true;
        }
        if let Some(level) =
            parameter.read_string_id("name").and_then(|name| parameters.get_mut(&name))
            && level.animations.is_empty()
        {
            level.animations = read_animations(parameter);
        }
    }
    if hidden {
        parameters.values_mut().for_each(|level| level.animations.clear());
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
    let enabled = has_flag(state, "flags", "alpha-test enabled");
    if !enabled {
        return None;
    }
    let compare = match state.read_enum_option("alpha compare function")?.as_str() {
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
/// The signed functions are its add and subtract: MCC's shader compiler maps
/// them so (`sub_6DB880`, table 0xE9DB6C), where the Xbox had signed ops.
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
    let mode = implementation.read_enum_option("alpha-blend")?;
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
        equation: blend_equation(&state.read_enum_option("blend function")?)?,
        source: blend_factor(&state.read_enum_option("blend src factor")?)?,
        destination: blend_factor(&state.read_enum_option("blend dst factor")?)?,
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
    match implementation.read_enum_option("depth").as_deref() {
        Some("default opaque") => test(H2Compare::Equal, false),
        Some("default opaque write") => test(H2Compare::LessOrEqual, true),
        Some("default transparent") => test(H2Compare::LessOrEqual, false),
        Some("custom") => implementation
            .descend("depth state[0]")
            .and_then(|state| {
                let function = state.read_enum_option("depth compare function")?;
                let write = has_flag(state, "flags", "depth write");
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
            (parameter.read_string_id("name"), parameter.read_enum_option("source extern"))
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
    let animations = own
        .keys()
        .filter(|name| !externs.contains_key(*name))
        .filter_map(|name| {
            let level = shader.get(name).filter(|level| !level.animations.is_empty())?;
            Some((name.clone(), level.animations.clone()))
        })
        .collect();

    let textures = elements(implementation, "textures")
        .map(|texture| {
            let parameter = texture.read_string_id("source parameter").unwrap_or_default();
            let source = texture.read_enum_option("source extern").unwrap_or_default();
            let value = if !source.is_empty() && source != NO_EXTERN {
                H2ParameterValue::Extern(source)
            } else {
                resolve_parameter(levels, &parameter, "bitmap")
            };
            H2PassTexture {
                parameter,
                mode: texture.read_enum_option("mode").unwrap_or_default(),
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
        channels: implementation.read_enum_option("channels").unwrap_or_default(),
        custom_channels: read_custom_channels(implementation),
        alpha_test: read_alpha_test(implementation),
        textures,
        parameters,
        animations,
        vertex_constants: elements(implementation, "vs constants")
            .map(|constant| H2VertexConstant {
                parameter: constant.read_string_id("source parameter").unwrap_or_default(),
                bank: constant.read_enum_option("register bank").unwrap_or_default(),
                register: constant.read_int_any("register index").unwrap_or(0) as i32,
                components: constant.read_enum_option("component mask").unwrap_or_default(),
            })
            .collect(),
    })
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
                    && name != LIGHT_GEL
                    && name != FRAME_COPY
                    && name != ACTIVE_CAMO_BUMP
                    && !CAMERA_FEEDS.contains(&name.as_str())
                    && !(has_texaccum && name == TEXACCUM_TARGET)
        )
    })
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

/// Vertex registers holding a lightmap pass's light vectors.
const LIGHT_DIRECT_REGISTER: i32 = 8;

const LIGHT_INDIRECT_REGISTER: i32 = 9;

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
        // A pass's own value for it comes from the frame; see `camo_amounts`.
        ACTIVE_CAMO_AMOUNT => [1.0; 4],
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

/// The components a vertex constant's `component mask` writes. Color and
/// brightness are separate parameters sharing a register, as rgb and alpha.
fn component_mask(components: &str) -> [bool; 4] {
    match components {
        "x (value)" => [true, false, false, false],
        "y (value)" => [false, true, false, false],
        "z (value)" => [false, false, true, false],
        "w (value)" => [false, false, false, true],
        "xyz/rgb (color)" => [true, true, true, false],
        _ => [true; 4],
    }
}

fn channel_mask(channels: &str) -> [bool; 4] {
    match channels {
        "color only" => [true, true, true, false],
        "alpha only" => [false, false, false, true],
        _ => [true; 4],
    }
}

/// A `custom` pass's own mask. None set, as on `z_prepass`, writes depth alone.
fn read_custom_channels(implementation: TagStruct<'_>) -> Option<[bool; 4]> {
    if implementation.read_enum_option("channels").as_deref() != Some("custom") {
        return None;
    }
    let state = implementation.descend("channels state[0]");
    Some(["R", "G", "B", "A"].map(|channel| state.is_some_and(|state| has_flag(state, "flags", channel))))
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
    let pixel = pixel_constants(pass, &translated.constants, specular_brightness);
    let textures = draw_textures(pass, &translated.samplers, &mut bitmap);
    let vertex = vertex_constants(pass, stage, specular_brightness, &scale);
    Ok(H2DrawPass {
        stage,
        fragment: translated.glsl,
        blend: pass.blend,
        depth: pass.depth,
        depth_bias: 0.0,
        cull: H2Cull::Frame,
        hidden_by_camo: false,
        numeric: None,
        channels: pass.custom_channels.unwrap_or_else(|| channel_mask(&pass.channels)),
        textures,
        constants: pixel.constants,
        texture_rows: vertex.texture_rows,
        tint_facing: vertex.tint_facing,
        tint_glancing: vertex.tint_glancing,
        change_colors: pixel.change_colors,
        light_colors: pixel.light_colors,
        camo_amounts: pixel.camo_amounts,
        light_direct: vertex.light_direct,
        light_indirect: vertex.light_indirect,
        animated_constants: pixel.animated_constants,
        animated_rows: vertex.animated_rows,
        animated_row_values: vertex.animated_row_values,
        animated_tint_facing: vertex.animated_tint_facing,
        animated_tint_glancing: vertex.animated_tint_glancing,
        ce_animation: None,
    })
}

/// `pass`'s value for the parameter `name`, if it has one.
fn parameter_of<'a>(pass: &'a H2Pass, name: &str) -> Option<&'a H2ParameterValue> {
    pass.parameters.iter().find(|(parameter, _)| parameter == name).map(|(_, value)| value)
}

/// The animations `pass` runs on the parameter `name`.
fn animations_of(pass: &H2Pass, name: &str) -> Vec<H2Animation> {
    pass.animations.get(name).cloned().unwrap_or_default()
}

/// The pixel shader's constants, and which of their components animate, take
/// a change color, a light's color or the active camo amount.
struct PixelConstants {
    constants: Vec<(u32, [f32; 4])>,
    animated_constants: Vec<(u32, usize, Vec<H2Animation>)>,
    change_colors: Vec<(u32, usize, usize)>,
    light_colors: Vec<(u32, usize)>,
    camo_amounts: Vec<(u32, usize)>,
}

/// The translated pixel shader's constant bindings, filled from `pass`'s
/// parameters component by component.
fn pixel_constants(pass: &H2Pass, bindings: &[ConstantBinding], specular_brightness: f32) -> PixelConstants {
    let lookup = |name: &str| parameter_of(pass, name);
    let mut constants: Vec<(u32, [f32; 4])> = Vec::new();
    let mut animated_constants = Vec::new();
    let mut change_colors = Vec::new();
    let mut light_colors = Vec::new();
    let mut camo_amounts = Vec::new();
    for binding in bindings {
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
            let animations = animations_of(pass, &binding.source);
            if animations.iter().any(|animation| !animation.is_transform()) {
                animated_constants.push((binding.register, index, animations));
            }
            if let Some(H2ParameterValue::Extern(source)) = lookup(&binding.source)
                && let Some(channel) = change_color_channel(source)
                && index < 3
            {
                change_colors.push((binding.register, index, channel));
            }
            if let Some(H2ParameterValue::Extern(source)) = lookup(&binding.source)
                && matches!(
                    source.as_str(),
                    "LIGHT diffuse color"
                        | "LIGHT specular color"
                        | "LIGHTMAP object direct color"
                        | "LIGHTMAP object specular color"
                        | "LIGHTMAP object specular color times one half"
                )
                && index < 3
            {
                light_colors.push((binding.register, index));
            }
            if matches!(lookup(&binding.source), Some(H2ParameterValue::Extern(name)) if name == ACTIVE_CAMO_AMOUNT)
                && index < 4
            {
                camo_amounts.push((binding.register, index));
            }
        }
    }
    PixelConstants { constants, animated_constants, change_colors, light_colors, camo_amounts }
}

/// Each sampler's map: a bitmap decoded by `bitmap`, a black feed for the
/// camera externs, the bump map for active camo's, or an extern the renderer
/// binds (normalization, texaccum, falloff, gel, the frame copy).
fn draw_textures(
    pass: &H2Pass,
    samplers: &[SamplerBinding],
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<std::sync::Arc<H2Image>>,
) -> Vec<H2DrawTexture> {
    samplers
        .iter()
        .map(|sampler| {
            let value = pass.textures.get(sampler.slot as usize).map(|texture| &texture.value);
            let image = match value {
                Some(H2ParameterValue::Bitmap(path)) => bitmap(path.as_str(), sampler.kind),
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
                    match parameter_of(pass, "bump_map") {
                        Some(H2ParameterValue::Bitmap(path)) => bitmap(path.as_str(), sampler.kind),
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
            let gel = matches!(value, Some(H2ParameterValue::Extern(name)) if name == LIGHT_GEL);
            H2DrawTexture {
                slot: sampler.slot,
                kind: sampler.kind,
                image,
                normalization,
                texaccum,
                gel,
                frame_copy: matches!(
                    value,
                    Some(H2ParameterValue::Extern(name))
                        if name == FRAME_COPY || name == WATER_REFRACTION
                ),
                falloff,
            }
        })
        .collect()
}

/// The vertex shader's constants: the texture rows (scales and transforms),
/// the facing and glancing tints, the lightmap's two light vectors, and
/// which of them animate.
struct VertexConstants {
    texture_rows: [[f32; 4]; H2_TEXTURE_ROWS],
    tint_facing: [f32; 4],
    tint_glancing: [f32; 4],
    animated_tint_facing: Vec<([bool; 4], Vec<H2Animation>)>,
    animated_tint_glancing: Vec<([bool; 4], Vec<H2Animation>)>,
    animated_rows: Vec<H2AnimatedRow>,
    animated_row_values: Vec<(usize, usize, Vec<H2Animation>)>,
    light_direct: [f32; 3],
    light_indirect: [f32; 3],
}

/// The texture rows a `stage` starts from before its constants fill them.
fn default_texture_rows(stage: H2VertexStage) -> [[f32; 4]; H2_TEXTURE_ROWS] {
    if stage.uses_scale_rows() {
        std::array::from_fn(|row| {
            if (row == 3 && stage.is_lightmap()) || (row == 2 && stage == H2VertexStage::Texaccum) {
                [1.0; 4]
            } else {
                [1.0, 1.0, 0.0, 0.0]
            }
        })
    } else {
        identity_rows()
    }
}

/// `pass`'s vertex constants, in order: a lightmap stage's light vectors, the
/// `Vn` tints, then the texture rows, by value or by `scale`'s tiling.
fn vertex_constants(
    pass: &H2Pass,
    stage: H2VertexStage,
    specular_brightness: f32,
    scale: &impl Fn(&str) -> [f32; 2],
) -> VertexConstants {
    let lookup = |name: &str| parameter_of(pass, name);
    let mut texture_rows = default_texture_rows(stage);
    let (mut tint_facing, mut tint_glancing) = ([1.0; 4], [1.0; 4]);
    let (mut animated_tint_facing, mut animated_tint_glancing) = (Vec::new(), Vec::new());
    let mut animated_rows = Vec::new();
    let mut animated_row_values = Vec::new();
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
            let mask = component_mask(&constant.components);
            let (tint, animated) = match constant.register {
                10 => (&mut tint_facing, &mut animated_tint_facing),
                11 => (&mut tint_glancing, &mut animated_tint_glancing),
                _ => continue,
            };
            fill_components(tint, value, mask);
            animated.push((mask, animations_of(pass, &constant.parameter)));
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
                let animations = animations_of(pass, &constant.parameter);
                if animations.iter().any(|animation| !animation.is_transform()) {
                    animated_row_values.push((constant.register.max(0) as usize, channel, animations));
                }
                continue;
            }
        }
        let [x, y] = scale(&constant.parameter);
        let Some(form) = scale_row(row, &constant.components, [x, y]) else {
            continue;
        };
        let animations: Vec<_> = animations_of(pass, &constant.parameter)
            .into_iter()
            .filter(H2Animation::is_transform)
            .collect();
        if !animations.is_empty() {
            animated_rows.push(H2AnimatedRow {
                register: constant.register.max(0) as usize,
                form,
                scale: [x, y, x],
                animations,
            });
        }
    }
    VertexConstants {
        texture_rows,
        tint_facing,
        tint_glancing,
        animated_tint_facing,
        animated_tint_glancing,
        animated_rows,
        animated_row_values,
        light_direct,
        light_indirect,
    }
}

/// A texture row filled from a parameter's tiling, in the form its
/// `components` name; `None` when they name none.
fn scale_row(row: &mut [f32; 4], components: &str, [x, y]: [f32; 2]) -> Option<H2RowForm> {
    let form = if components.contains("row1") {
        *row = [x, 0.0, 0.0, 0.0];
        H2RowForm::Row1
    } else if components.contains("row2") {
        *row = [0.0, y, 0.0, 0.0];
        H2RowForm::Row2
    } else if components.contains("row3") {
        *row = [0.0, 0.0, x, 0.0];
        H2RowForm::Row3
    } else if components.contains("simple xform") {
        *row = [x, y, 0.0, 0.0];
        H2RowForm::SimpleXform
    } else if components.starts_with("xy (2D scale") {
        row[0] = x;
        row[1] = y;
        H2RowForm::ScaleXy
    } else if components.starts_with("zw (2D scale") {
        row[2] = x;
        row[3] = y;
        H2RowForm::ScaleZw
    } else {
        return None;
    };
    Some(form)
}

/// The passes of one layer of `shader`, in template order. `load` reads a tag
/// by path, extension and group; a shader with no template has no passes.
pub(in crate::app) fn resolve_h2_layer_passes<T: std::borrow::Borrow<TagFile>>(
    shader: &TagFile,
    mut load: impl FnMut(&str, &str, &[u8; 4]) -> Option<T>,
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
    let template_root = template.borrow().root();
    let shader_levels = shader_parameters(root);
    let template_levels = template_parameters(template_root);

    let mut passes = Vec::new();
    for entry in elements(template_root, "LODs[0]/pass") {
        if entry.read_enum_option("layer").as_deref() != Some(layer) {
            continue;
        }
        let path = entry.read_tag_ref_path("pass").unwrap_or_default();
        let pass = load(&path, "shader_pass", b"spas")
            .ok_or_else(|| format!("Cannot read {path}"))?;
        passes.push(read_pass(
            &path,
            pass.borrow().root(),
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
    use crate::app::model_preview::harness::kit::kit_root;
    use crate::core::game::GameId;

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
                    animations: Vec::new(),
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
    fn rows_and_constants_come_from_the_pass() {
        let pass = H2Pass {
            path: String::new(),
            implementation: 0,
            vertex_shader: "rasterizer\\vertex_shaders_dx9\\transparent_generic_default".to_owned(),
            blend: None,
            depth: H2Depth { compare: None, write: false },
            channels: "color only".to_owned(),
            custom_channels: None,
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
            animations: Default::default(),
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
    fn tint_color_and_brightness_share_a_register() {
        let pass = H2Pass {
            path: String::new(),
            implementation: 0,
            vertex_shader: "rasterizer\\vertex_shaders_dx9\\transparent_generic_default".to_owned(),
            blend: None,
            depth: H2Depth { compare: None, write: false },
            channels: "color only".to_owned(),
            custom_channels: None,
            alpha_test: None,
            textures: Vec::new(),
            parameters: vec![
                ("color".to_owned(), H2ParameterValue::Color([0.5, 0.0, 0.25])),
                ("bright".to_owned(), H2ParameterValue::Value(0.8)),
            ],
            vertex_constants: vec![
                H2VertexConstant { parameter: "color".to_owned(), bank: "Vn [0,15]".to_owned(), register: 10, components: "xyz/rgb (color)".to_owned() },
                H2VertexConstant { parameter: "bright".to_owned(), bank: "Vn [0,15]".to_owned(), register: 10, components: "w (value)".to_owned() },
            ],
            animations: Default::default(),
        };
        let drawn = prepare_draw_pass(&pass, "float4 main(\n)\n{\n    return 0;\n}\n", HashMap::new(), 1.0, |_, _| None, |_| [1.0, 1.0]).unwrap();
        assert_eq!(drawn.tint_facing, [0.5, 0.0, 0.25, 0.8]);
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
            custom_channels: None,
            alpha_test: None,
            textures: vec![H2PassTexture {
                parameter: String::new(),
                mode: String::new(),
                value: H2ParameterValue::Extern(name.to_owned()),
            }],
            parameters: Vec::new(),
            vertex_constants: Vec::new(),
            animations: Default::default(),
        };
        assert!(h2_pass_needs_frame_texture(&pass(TEXACCUM_TARGET), false));
        assert!(!h2_pass_needs_frame_texture(&pass(TEXACCUM_TARGET), true));
        assert!(h2_pass_needs_frame_texture(&pass("GLOBAL something unsupplied"), true));
        // A camera feed draws as a dark screen rather than costing the material.
        assert!(!h2_pass_needs_frame_texture(&pass("GLOBAL texture camera"), false));
        assert!(!h2_pass_needs_frame_texture(&pass("FIRST-PERSON scope"), false));
        assert!(!h2_pass_needs_frame_texture(&pass(VECTOR_NORMALIZATION), false));
    }

    /// With `BLAM_TEST_H2EK`: a change-color shader fills texaccum on its
    /// light albedo layer, the only thing its diffuse pass has to read.
    #[test]
    fn jackal_legs_diffuse_reads_its_light_albedo() {
        let Some(root) = kit_root(GameId::Halo2) else { return };
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

    /// With `BLAM_TEST_H2EK`: the tag's constant is 1, so a resolve that
    /// ignores the shader's input would leave the emblem on.
    #[test]
    fn a_player_emblem_is_off_without_a_player() {
        let Some(root) = kit_root(GameId::Halo2) else { return };
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
        for path in [
            "objects\\characters\\masterchief\\shaders\\mc_emblem",
            "objects\\characters\\elite\\shaders\\elite_emblem",
        ] {
            let shader = read(path, "shader", b"shad").unwrap();
            let passes = resolve_h2_layer_passes(&shader, read, H2Layer::Overlay.tag_name(), 0).unwrap();
            let value = passes[0].parameters.iter().find(|(name, _)| name == EMBLEM_INPUT);
            assert_eq!(value.map(|(_, value)| value), Some(&H2ParameterValue::Value(0.0)), "{path}");
        }
    }

    /// With `BLAM_TEST_H2EK`: this pass's pixel shader is solid white, which
    /// shows through whatever is translucent over it unless the mask stops it.
    #[test]
    fn a_depth_prepass_writes_no_color() {
        let Some(root) = kit_root(GameId::Halo2) else { return };
        let definitions = crate::core::bundled::locate_definitions_root();
        let file = root.join("shaders/shader_passes/z_prepass/z_prepass.shader_pass");
        let pass = crate::core::source::read_tag_at_path(
            &file,
            Some(crate::core::game::GameId::Halo2),
            Some(&definitions),
            u32::from_be_bytes(*b"spas"),
        )
        .unwrap();
        let root = pass.root();
        let implementation = elements(root, "implementations").next().unwrap();
        assert_eq!(read_custom_channels(implementation), Some([false; 4]));
    }

    /// With `BLAM_TEST_H2EK`: the constant is 1, and reading it instead of
    /// the function leaves a parked ghost boosting.
    #[test]
    fn an_idle_ghost_is_not_boosting() {
        let Some(root) = kit_root(GameId::Halo2) else { return };
        let definitions = crate::core::bundled::locate_definitions_root();
        let file = root.join("objects/vehicles/ghost/shaders/plasma_vent.shader");
        let shader = crate::core::source::read_tag_at_path(
            &file,
            Some(crate::core::game::GameId::Halo2),
            Some(&definitions),
            u32::from_be_bytes(*b"shad"),
        )
        .unwrap();
        let root = shader.root();
        let levels = shader_parameters(root);
        let animations = &levels["framebuffer_fade"].animations;
        assert_eq!(animated_vector(animations, 3.0.into()).map(|value| value[0]), Some(0.0));
    }

    /// With `BLAM_TEST_H2EK`: the brute hair that drew untextured.
    #[test]
    fn brute_hair_resolves_from_the_kit() {
        let Some(root) = kit_root(GameId::Halo2) else { return };
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
