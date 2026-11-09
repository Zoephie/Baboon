//! Reading a Halo 2 shader template's passes: their externs, parameter levels, blend, depth and alpha test.

use super::super::super::common::fields::{path_elements, rgb};
use super::super::super::dx9::passes::{
    Dx9Blend, Dx9BlendEquation, Dx9BlendFactor, Dx9Compare, Dx9Depth, H2Animation, H2AnimationKind,
    animation_function,
};
use super::constants::read_custom_channels;
use super::template::{Dx9ParameterValue, Dx9Pass, Dx9PassTexture, Dx9VertexConstant};
use super::{AlphaCompare, AlphaTest, has_flag, object_input};
use blam_tags::{TagFieldData, TagFile, TagStruct};
use std::collections::HashMap;
use std::sync::Arc;

/// The object function that shows a player's emblem.
pub(super) const EMBLEM_INPUT: &str = "emblem_active";

pub(super) const TEXACCUM_TARGET: &str = "GLOBAL target texaccum";

/// What the game copies the frame into, for what looks through a surface.
pub(super) const FRAME_COPY: &str = "GLOBAL active camo";

pub(super) const WATER_REFRACTION: &str = "GLOBAL water refraction";

/// What a live camera feeds, which a preview has none of: a dark screen.
pub(super) const CAMERA_FEEDS: [&str; 2] = ["GLOBAL texture camera", "FIRST-PERSON scope"];

/// The bump of the shader the global one replaced: the shader's own bump map,
/// by another name. It reaches a pass as its `bump_map`, from
/// `prepare_h2_camo_passes`.
pub(super) const ACTIVE_CAMO_BUMP: &str = "SHADER active camo bump";

pub(super) const LIGHT_FALLOFF: &str = "LIGHT falloff";

pub(super) const LIGHT_GEL: &str = "LIGHT gel";

pub(super) const ACTIVE_CAMO_AMOUNT: &str = "OBJECT active camo amount";

/// The one frame texture a preview can supply itself.
pub(super) const VECTOR_NORMALIZATION: &str = "GLOBAL vector-normalization";

const NO_EXTERN: &str = "none";

/// The animations of a parameter. The game reads these and never the
/// parameter's constant, so an input a preview has no value for still counts.
fn read_animations(parameter: TagStruct<'_>) -> Vec<H2Animation> {
    path_elements(parameter, "animation properties")
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
pub(super) struct ParameterLevel {
    pub(super) kind: String,
    pub(super) bitmap: String,
    pub(super) value: f32,
    pub(super) color: [f32; 3],
    /// A shader's own: `shader_setup_get_shader_parameters` is all the game reads.
    pub(super) animations: Vec<H2Animation>,
}

pub(super) type Parameters = HashMap<String, ParameterLevel>;

/// The game's lookup: the first level holding the name, except that a bitmap
/// with no tag falls through to the next level. A level only answers for its
/// own kind of parameter.
pub(super) fn resolve_parameter(
    levels: [&Parameters; 3],
    name: &str,
    kind: &str,
) -> Dx9ParameterValue {
    for level in levels {
        let Some(found) = level.get(name).filter(|found| found.kind == kind) else {
            continue;
        };
        match kind {
            "bitmap" if found.bitmap.is_empty() => continue,
            "bitmap" => return Dx9ParameterValue::Bitmap(found.bitmap.clone()),
            "value" => return Dx9ParameterValue::Value(found.value),
            "color" => return Dx9ParameterValue::Color(found.color),
            _ => break,
        }
    }
    Dx9ParameterValue::Missing
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

pub(super) fn shader_parameters(shader: TagStruct<'_>) -> Parameters {
    let mut parameters = Parameters::new();
    let mut hidden = false;
    for parameter in path_elements(shader, "parameters") {
        insert_level(
            &mut parameters,
            parameter,
            "bitmap",
            "const value",
            "const color",
        );
        // Only a player has an emblem, which a preview's object is not.
        if path_elements(parameter, "animation properties").any(|animation| {
            animation.read_string_id("input name").as_deref() == Some(EMBLEM_INPUT)
        }) && let Some(level) = parameter
            .read_string_id("name")
            .and_then(|name| parameters.get_mut(&name))
        {
            level.value = 0.0;
            hidden = true;
        }
        if let Some(level) = parameter
            .read_string_id("name")
            .and_then(|name| parameters.get_mut(&name))
            && level.animations.is_empty()
        {
            level.animations = read_animations(parameter);
        }
    }
    if hidden {
        parameters
            .values_mut()
            .for_each(|level| level.animations.clear());
    }
    parameters
}

fn template_parameters(template: TagStruct<'_>) -> Parameters {
    let mut parameters = Parameters::new();
    for category in path_elements(template, "categories") {
        for parameter in path_elements(category, "parameters") {
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

pub(super) fn named_blend(mode: &str) -> Option<(Dx9BlendFactor, Dx9BlendFactor)> {
    use Dx9BlendFactor::*;
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

fn blend_factor(name: &str) -> Option<Dx9BlendFactor> {
    use Dx9BlendFactor::*;
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
pub(super) fn blend_equation(name: &str) -> Option<Dx9BlendEquation> {
    use Dx9BlendEquation::*;
    Some(match name {
        "add" | "add signed" => Add,
        "subtract" | "reverse subtract signed" => Subtract,
        "reverse subtract" => ReverseSubtract,
        "min" => Min,
        "max" => Max,
        _ => return None,
    })
}

fn read_blend(implementation: TagStruct<'_>) -> Option<Dx9Blend> {
    let mode = implementation.read_enum_option("alpha-blend")?;
    if mode != "custom" {
        let (source, destination) = named_blend(&mode)?;
        return Some(Dx9Blend {
            equation: Dx9BlendEquation::Add,
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
    Some(Dx9Blend {
        equation: blend_equation(&state.read_enum_option("blend function")?)?,
        source: blend_factor(&state.read_enum_option("blend src factor")?)?,
        destination: blend_factor(&state.read_enum_option("blend dst factor")?)?,
        constant: color.to_be_bytes(),
    })
}

fn compare(name: &str) -> Option<Dx9Compare> {
    use Dx9Compare::*;
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

fn read_depth(implementation: TagStruct<'_>) -> Dx9Depth {
    const OFF: Dx9Depth = Dx9Depth {
        compare: None,
        write: false,
    };
    let test = |compare, write| Dx9Depth {
        compare: Some(compare),
        write,
    };
    match implementation.read_enum_option("depth").as_deref() {
        Some("default opaque") => test(Dx9Compare::Equal, false),
        Some("default opaque write") => test(Dx9Compare::LessOrEqual, true),
        Some("default transparent") => test(Dx9Compare::LessOrEqual, false),
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

pub(super) fn read_pass(
    path: &str,
    pass: TagStruct<'_>,
    index: usize,
    shader: &Parameters,
    template: &Parameters,
) -> Result<Dx9Pass, String> {
    let implementation = pass
        .descend(&format!("implementations[{index}]"))
        .ok_or_else(|| format!("{path} has no implementation {index}"))?;

    let mut own = Parameters::new();
    let mut externs: HashMap<String, String> = HashMap::new();
    for parameter in path_elements(pass, "parameters") {
        insert_level(
            &mut own,
            parameter,
            "default bitmap",
            "default const value",
            "default const color",
        );
        if let (Some(name), Some(source)) = (
            parameter.read_string_id("name"),
            parameter.read_enum_option("source extern"),
        ) && source != NO_EXTERN
        {
            externs.insert(name, source);
        }
    }
    let levels = [shader, template, &own];

    let mut parameters: Vec<_> = own
        .iter()
        .map(|(name, level)| {
            let value = match externs.get(name) {
                Some(source) => Dx9ParameterValue::Extern(source.clone()),
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
            let level = shader
                .get(name)
                .filter(|level| !level.animations.is_empty())?;
            Some((name.clone(), level.animations.clone()))
        })
        .collect();

    let textures = path_elements(implementation, "textures")
        .map(|texture| {
            let parameter = texture
                .read_string_id("source parameter")
                .unwrap_or_default();
            let source = texture
                .read_enum_option("source extern")
                .unwrap_or_default();
            let value = if !source.is_empty() && source != NO_EXTERN {
                Dx9ParameterValue::Extern(source)
            } else {
                resolve_parameter(levels, &parameter, "bitmap")
            };
            Dx9PassTexture {
                parameter,
                mode: texture.read_enum_option("mode").unwrap_or_default(),
                value,
            }
        })
        .collect();

    Ok(Dx9Pass {
        path: path.to_owned(),
        implementation: index,
        vertex_shader: implementation
            .read_tag_ref_path("vertex shader")
            .unwrap_or_default(),
        blend: read_blend(implementation),
        depth: read_depth(implementation),
        channels: implementation
            .read_enum_option("channels")
            .unwrap_or_default(),
        custom_channels: read_custom_channels(implementation),
        alpha_test: read_alpha_test(implementation),
        textures,
        parameters,
        animations,
        vertex_constants: path_elements(implementation, "vs constants")
            .map(|constant| Dx9VertexConstant {
                parameter: constant
                    .read_string_id("source parameter")
                    .unwrap_or_default(),
                bank: constant
                    .read_enum_option("register bank")
                    .unwrap_or_default(),
                register: constant.read_int_any("register index").unwrap_or(0) as i32,
                components: constant
                    .read_enum_option("component mask")
                    .unwrap_or_default(),
            })
            .collect(),
    })
}

/// Frame textures a pass needs that a preview has no way to produce. The
/// texaccum target counts unless this material's own texaccum pass fills it.
pub(in crate::app) fn h2_pass_needs_frame_texture(pass: &Dx9Pass, has_texaccum: bool) -> bool {
    pass.textures.iter().any(|texture| {
        matches!(
            &texture.value,
            Dx9ParameterValue::Extern(name)
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
pub(super) fn change_color_channel(source: &str) -> Option<usize> {
    match source {
        "OBJECT change color primary" => Some(0),
        "OBJECT change color secondary" => Some(1),
        "OBJECT change color tertiary" => Some(2),
        "OBJECT change color quaternary" => Some(3),
        _ => None,
    }
}

/// The passes of one layer of `shader`, in template order. `load` reads a tag
/// by path, extension and group; a shader with no template has no passes.
pub(in crate::app) fn resolve_h2_layer_passes<T: std::borrow::Borrow<TagFile>>(
    shader: &TagFile,
    mut load: impl FnMut(&str, &str, &[u8; 4]) -> Option<T>,
    layer: &str,
    implementation: usize,
) -> Result<Vec<Dx9Pass>, String> {
    let root = shader.root();
    let Some(template_path) = root
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
    else {
        return Ok(Vec::new());
    };
    let template = load(&template_path, "shader_template", b"stem")
        .ok_or_else(|| format!("Cannot read {template_path}"))?;
    let template_root = template.borrow().root();
    let shader_levels = shader_parameters(root);
    let template_levels = template_parameters(template_root);

    let mut passes = Vec::new();
    for entry in path_elements(template_root, "LODs[0]/pass") {
        if entry.read_enum_option("layer").as_deref() != Some(layer) {
            continue;
        }
        let path = entry.read_tag_ref_path("pass").unwrap_or_default();
        let pass =
            load(&path, "shader_pass", b"spas").ok_or_else(|| format!("Cannot read {path}"))?;
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
