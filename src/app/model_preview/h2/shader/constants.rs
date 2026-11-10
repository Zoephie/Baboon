//! Preparing a pass for draw: its externs, masks, and the pixel and vertex constants it uploads.

use super::super::super::dx9::passes::{
    DEFAULT_AMBIENT, DEFAULT_DIRECT_COLOR, DEFAULT_DIRECT_VECTOR, DEFAULT_INDIRECT_COLOR,
    DEFAULT_INDIRECT_VECTOR, Dx9AnimatedRow, Dx9Animation, Dx9Cull, Dx9DrawPass, Dx9DrawTexture,
    Dx9RowForm, PASS_GLSL_VERSION, fill_components, identity_rows,
};
use super::passes::{
    ACTIVE_CAMO_AMOUNT, ACTIVE_CAMO_BUMP, CAMERA_FEEDS, FRAME_COPY, LIGHT_FALLOFF, LIGHT_GEL,
    TEXACCUM_TARGET, VECTOR_NORMALIZATION, WATER_REFRACTION, change_color_channel,
};
use super::template::{Dx9ParameterValue, Dx9Pass};
use super::{
    ConstantBinding, DX9_TEXTURE_ROWS, Dx9VertexStage, Image, PixelShaderOptions, SamplerBinding,
    SamplerKind, TextureImage, has_flag, translate,
};
use blam_tags::TagStruct;
use std::collections::HashMap;

/// Vertex registers holding a lightmap pass's light vectors.
const LIGHT_DIRECT_REGISTER: i32 = 8;

const LIGHT_INDIRECT_REGISTER: i32 = 9;

pub(super) fn extern_value(name: &str, specular_brightness: f32) -> [f32; 4] {
    let [r, g, b, a] = DEFAULT_DIRECT_COLOR;
    match name {
        // The luminance of ambient plus direct; see `rasterizer_get_value_extern`.
        "LIGHTMAP object env brightness" => {
            let [ambient, ..] = DEFAULT_AMBIENT;
            let luminance = 0.299 * (r + ambient) + 0.587 * (g + ambient) + 0.114 * (b + ambient);
            [luminance.clamp(0.0, 1.0); 4]
        }
        "LIGHTMAP object specular color" => [
            r * specular_brightness,
            g * specular_brightness,
            b * specular_brightness,
            a,
        ],
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

fn parameter_vector(value: &Dx9ParameterValue, specular_brightness: f32) -> [f32; 4] {
    match value {
        Dx9ParameterValue::Value(value) => [*value; 4],
        Dx9ParameterValue::Color([r, g, b]) => [*r, *g, *b, 1.0],
        Dx9ParameterValue::Extern(name) => extern_value(name, specular_brightness),
        Dx9ParameterValue::Bitmap(_) | Dx9ParameterValue::Missing => [0.0, 0.0, 0.0, 1.0],
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
pub(super) fn read_custom_channels(implementation: TagStruct<'_>) -> Option<[bool; 4]> {
    if implementation.read_enum_option("channels").as_deref() != Some("custom") {
        return None;
    }
    let state = implementation.descend("channels state[0]");
    Some(
        ["R", "G", "B", "A"]
            .map(|channel| state.is_some_and(|state| has_flag(state, "flags", channel))),
    )
}

/// Turn a resolved pass into what the renderer draws. `bitmap` decodes a map
/// by path in the shape its sampler wants; `scale` gives a parameter's tiling.
pub(in crate::app) fn prepare_draw_pass(
    pass: &Dx9Pass,
    psh: &str,
    cases: HashMap<String, i32>,
    specular_brightness: f32,
    mut bitmap: impl FnMut(&str, SamplerKind) -> Option<std::sync::Arc<Image>>,
    scale: impl Fn(&str) -> [f32; 2],
) -> Result<Dx9DrawPass, String> {
    let stage = Dx9VertexStage::from_path(&pass.vertex_shader)
        .ok_or_else(|| format!("No vertex stage for {}", pass.vertex_shader))?;
    let options = PixelShaderOptions {
        cases,
        alpha_test: pass.alpha_test,
    };
    let translated = translate(psh, PASS_GLSL_VERSION, &options)?;
    let pixel = pixel_constants(pass, &translated.constants, specular_brightness);
    let textures = draw_textures(pass, &translated.samplers, &mut bitmap);
    let vertex = vertex_constants(pass, stage, specular_brightness, &scale);
    Ok(Dx9DrawPass {
        stage,
        fragment: translated.glsl,
        blend: pass.blend,
        depth: pass.depth,
        depth_bias: 0.0,
        cull: Dx9Cull::Frame,
        hidden_by_camo: false,
        numeric: None,
        channels: pass
            .custom_channels
            .unwrap_or_else(|| channel_mask(&pass.channels)),
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
        own_animation: None,
    })
}

/// `pass`'s value for the parameter `name`, if it has one.
fn parameter_of<'a>(pass: &'a Dx9Pass, name: &str) -> Option<&'a Dx9ParameterValue> {
    pass.parameters
        .iter()
        .find(|(parameter, _)| parameter == name)
        .map(|(_, value)| value)
}

/// The animations `pass` runs on the parameter `name`.
fn animations_of(pass: &Dx9Pass, name: &str) -> Vec<Dx9Animation> {
    pass.animations.get(name).cloned().unwrap_or_default()
}

/// The pixel shader's constants, and which of their components animate, take
/// a change color, a light's color or the active camo amount.
struct PixelConstants {
    constants: Vec<(u32, [f32; 4])>,
    animated_constants: Vec<(u32, usize, Vec<Dx9Animation>)>,
    change_colors: Vec<(u32, usize, usize)>,
    light_colors: Vec<(u32, usize)>,
    camo_amounts: Vec<(u32, usize)>,
}

/// The translated pixel shader's constant bindings, filled from `pass`'s
/// parameters component by component.
fn pixel_constants(
    pass: &Dx9Pass,
    bindings: &[ConstantBinding],
    specular_brightness: f32,
) -> PixelConstants {
    let lookup = |name: &str| parameter_of(pass, name);
    let mut constants: Vec<(u32, [f32; 4])> = Vec::new();
    let mut animated_constants = Vec::new();
    let mut change_colors = Vec::new();
    let mut light_colors = Vec::new();
    let mut camo_amounts = Vec::new();
    for binding in bindings {
        let value = lookup(&binding.source).map_or([0.0, 0.0, 0.0, 1.0], |value| {
            parameter_vector(value, specular_brightness)
        });
        let at = constants
            .iter()
            .position(|(register, _)| *register == binding.register);
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
            if let Some(Dx9ParameterValue::Extern(source)) = lookup(&binding.source)
                && let Some(channel) = change_color_channel(source)
                && index < 3
            {
                change_colors.push((binding.register, index, channel));
            }
            if let Some(Dx9ParameterValue::Extern(source)) = lookup(&binding.source)
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
            if matches!(lookup(&binding.source), Some(Dx9ParameterValue::Extern(name)) if name == ACTIVE_CAMO_AMOUNT)
                && index < 4
            {
                camo_amounts.push((binding.register, index));
            }
        }
    }
    PixelConstants {
        constants,
        animated_constants,
        change_colors,
        light_colors,
        camo_amounts,
    }
}

/// Each sampler's map: a bitmap decoded by `bitmap`, a black feed for the
/// camera externs, the bump map for active camo's, or an extern the renderer
/// binds (normalization, texaccum, falloff, gel, the frame copy).
fn draw_textures(
    pass: &Dx9Pass,
    samplers: &[SamplerBinding],
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<std::sync::Arc<Image>>,
) -> Vec<Dx9DrawTexture> {
    samplers
        .iter()
        .map(|sampler| {
            let value = pass
                .textures
                .get(sampler.slot as usize)
                .map(|texture| &texture.value);
            let image = match value {
                Some(Dx9ParameterValue::Bitmap(path)) => bitmap(path.as_str(), sampler.kind),
                Some(Dx9ParameterValue::Extern(name)) if CAMERA_FEEDS.contains(&name.as_str()) => {
                    Some(std::sync::Arc::new(Image::Flat(TextureImage {
                        rgba: vec![0, 0, 0, 255],
                        width: 1,
                        height: 1,
                        repeat_x: false,
                        repeat_y: false,
                        scale: [1.0, 1.0],
                    })))
                }
                Some(Dx9ParameterValue::Extern(name)) if name == ACTIVE_CAMO_BUMP => {
                    match parameter_of(pass, "bump_map") {
                        Some(Dx9ParameterValue::Bitmap(path)) => {
                            bitmap(path.as_str(), sampler.kind)
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            let normalization = matches!(
                value,
                Some(Dx9ParameterValue::Extern(name)) if name == VECTOR_NORMALIZATION
            );
            let texaccum = matches!(
                value,
                Some(Dx9ParameterValue::Extern(name)) if name == TEXACCUM_TARGET
            );
            let falloff = matches!(
                value,
                Some(Dx9ParameterValue::Extern(name)) if name == LIGHT_FALLOFF
            );
            let gel = matches!(value, Some(Dx9ParameterValue::Extern(name)) if name == LIGHT_GEL);
            Dx9DrawTexture {
                slot: sampler.slot,
                kind: sampler.kind,
                image,
                normalization,
                texaccum,
                gel,
                frame_copy: matches!(
                    value,
                    Some(Dx9ParameterValue::Extern(name))
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
    texture_rows: [[f32; 4]; DX9_TEXTURE_ROWS],
    tint_facing: [f32; 4],
    tint_glancing: [f32; 4],
    animated_tint_facing: Vec<([bool; 4], Vec<Dx9Animation>)>,
    animated_tint_glancing: Vec<([bool; 4], Vec<Dx9Animation>)>,
    animated_rows: Vec<Dx9AnimatedRow>,
    animated_row_values: Vec<(usize, usize, Vec<Dx9Animation>)>,
    light_direct: [f32; 3],
    light_indirect: [f32; 3],
}

/// The texture rows a `stage` starts from before its constants fill them.
fn default_texture_rows(stage: Dx9VertexStage) -> [[f32; 4]; DX9_TEXTURE_ROWS] {
    if stage.uses_scale_rows() {
        std::array::from_fn(|row| {
            if (row == 3 && stage.is_lightmap()) || (row == 2 && stage == Dx9VertexStage::Texaccum)
            {
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
    pass: &Dx9Pass,
    stage: Dx9VertexStage,
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
            && matches!(
                constant.register,
                LIGHT_DIRECT_REGISTER | LIGHT_INDIRECT_REGISTER
            )
        {
            let vector = if constant.register == LIGHT_DIRECT_REGISTER {
                &mut light_direct
            } else {
                &mut light_indirect
            };
            let value = lookup(&constant.parameter).map_or([0.0; 4], |value| {
                parameter_vector(value, specular_brightness)
            });
            match constant.components.as_str() {
                "x (value)" => vector[0] = value[0],
                "y (value)" => vector[1] = value[1],
                "z (value)" => vector[2] = value[2],
                _ => *vector = [value[0], value[1], value[2]],
            }
            continue;
        }
        if constant.bank.starts_with("Vn") {
            let value = lookup(&constant.parameter).map_or([1.0; 4], |value| {
                parameter_vector(value, specular_brightness)
            });
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
                let value = lookup(&constant.parameter).map_or(0.0, |value| {
                    parameter_vector(value, specular_brightness)[channel]
                });
                row[channel] = value;
                let animations = animations_of(pass, &constant.parameter);
                if animations.iter().any(|animation| !animation.is_transform()) {
                    animated_row_values.push((
                        constant.register.max(0) as usize,
                        channel,
                        animations,
                    ));
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
            .filter(Dx9Animation::is_transform)
            .collect();
        if !animations.is_empty() {
            animated_rows.push(Dx9AnimatedRow {
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
fn scale_row(row: &mut [f32; 4], components: &str, [x, y]: [f32; 2]) -> Option<Dx9RowForm> {
    let form = if components.contains("row1") {
        *row = [x, 0.0, 0.0, 0.0];
        Dx9RowForm::Row1
    } else if components.contains("row2") {
        *row = [0.0, y, 0.0, 0.0];
        Dx9RowForm::Row2
    } else if components.contains("row3") {
        *row = [0.0, 0.0, x, 0.0];
        Dx9RowForm::Row3
    } else if components.contains("simple xform") {
        *row = [x, y, 0.0, 0.0];
        Dx9RowForm::SimpleXform
    } else if components.starts_with("xy (2D scale") {
        row[0] = x;
        row[1] = y;
        Dx9RowForm::ScaleXy
    } else if components.starts_with("zw (2D scale") {
        row[2] = x;
        row[3] = y;
        Dx9RowForm::ScaleZw
    } else {
        return None;
    };
    Some(form)
}
