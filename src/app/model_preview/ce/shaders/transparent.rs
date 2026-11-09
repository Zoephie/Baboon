//! The transparent generic and chicago shaders' passes, with their stages, fades and maps.

use super::animation::{
    CeColorAnimation, CeMapAnimation, CeModelAnimation, PERIODIC_FUNCTIONS, color_channel,
    function_source, identity_map_rows, nonzero, read_map_animation,
};
use super::base::{
    ALPHA_REFERENCE, CHICAGO_FILE, GENERIC_FILE, ShaderFlags, flat, option_index,
    read_shader_source, shader_directory, texture,
};
use super::glass::{decal_bias, glass_passes};
use super::model::LAYER_GROUPS;
use super::special::{meter_pass, plasma_pass, water_passes};
use super::{
    CeNumeric, Dx9Compare, Dx9Cull, Dx9Depth, Dx9DrawPass, Dx9DrawTexture, Dx9VertexStage,
    FADE_MODES, FramebufferBlend, PASS_GLSL_VERSION, Image, SamplerKind, TagReader, argb, has_flag,
    translate_fx,
};
use blam_tags::{TagBlock, TagFieldData, TagStruct};
use std::sync::Arc;

/// `sub_859960` in MCC's tag_test: a stage's color function as the kit's
/// chicago shader reads it.
pub(super) fn chicago_color_table(function: i32) -> [f32; 12] {
    let mut table = [0.0; 12];
    let set = |table: &mut [f32; 12], pairs: &[(usize, f32)]| {
        pairs
            .iter()
            .for_each(|(index, value)| table[*index] = *value);
    };
    set(
        &mut table,
        match function {
            0 => &[(3, 1.0), (10, 1.0)],
            1 => &[(0, 1.0), (3, 1.0), (10, 1.0)],
            2 => &[(0, 1.0), (10, 1.0)],
            3 => &[(0, 1.0), (10, 2.0)],
            4 => &[(0, 1.0), (3, 1.0), (9, 1.0), (8, 1.0), (10, 1.0), (11, 1.0)],
            5 => &[
                (0, 1.0),
                (3, 1.0),
                (5, 1.0),
                (8, 2.0),
                (11, -1.0),
                (10, 1.0),
            ],
            6 => &[
                (3, 1.0),
                (5, 1.0),
                (6, 1.0),
                (8, 2.0),
                (11, -1.0),
                (10, 1.0),
            ],
            7 => &[
                (0, 1.0),
                (3, 1.0),
                (9, 1.0),
                (11, -1.0),
                (8, 1.0),
                (10, 1.0),
            ],
            8 => &[
                (3, 1.0),
                (4, 1.0),
                (9, 1.0),
                (11, -1.0),
                (8, 1.0),
                (10, 1.0),
            ],
            9 => &[(0, 1.0), (7, 1.0), (2, 1.0), (11, 1.0), (8, 1.0), (10, 1.0)],
            10 => &[(4, 1.0), (7, 1.0), (2, 1.0), (11, 1.0), (8, 1.0), (10, 1.0)],
            11 => &[
                (0, 1.0),
                (1, 1.0),
                (6, 1.0),
                (7, 1.0),
                (2, 1.0),
                (11, 1.0),
                (8, 1.0),
                (10, 1.0),
            ],
            12 => &[
                (1, 1.0),
                (6, 1.0),
                (4, 1.0),
                (7, 1.0),
                (2, 1.0),
                (11, 1.0),
                (8, 1.0),
                (10, 1.0),
            ],
            _ => &[],
        },
    );
    table
}

/// `sub_8597B0`: the same for a stage's alpha function.
pub(super) fn chicago_alpha_table(function: i32) -> [f32; 10] {
    let mut table = [0.0; 10];
    let set = |table: &mut [f32; 10], pairs: &[(usize, f32)]| {
        pairs
            .iter()
            .for_each(|(index, value)| table[*index] = *value);
    };
    set(
        &mut table,
        match function {
            0 => &[(2, 1.0), (8, 1.0)],
            1 => &[(0, 1.0), (2, 1.0), (8, 1.0)],
            2 => &[(0, 1.0), (8, 1.0)],
            3 => &[(0, 1.0), (8, 2.0)],
            4 => &[(0, 1.0), (2, 1.0), (7, 1.0), (9, 1.0), (6, 1.0), (8, 1.0)],
            5 => &[(0, 1.0), (2, 1.0), (4, 1.0), (6, 2.0), (9, -1.0), (8, 1.0)],
            6 => &[(2, 1.0), (4, 1.0), (5, 1.0), (6, 2.0), (9, -1.0), (8, 1.0)],
            7 => &[(0, 1.0), (2, 1.0), (7, 1.0), (9, -1.0), (6, 1.0), (8, 1.0)],
            8 => &[(2, 1.0), (3, 1.0), (7, 1.0), (9, -1.0), (6, 1.0), (8, 1.0)],
            9 => &[(0, 1.0), (9, 1.0), (6, 1.0), (8, 1.0)],
            10 => &[(3, 1.0), (9, 1.0), (6, 1.0), (8, 1.0)],
            11 => &[(0, 1.0), (1, 1.0), (5, 1.0), (9, 1.0), (6, 1.0), (8, 1.0)],
            12 => &[(1, 1.0), (5, 1.0), (3, 1.0), (9, 1.0), (6, 1.0), (8, 1.0)],
            _ => &[],
        },
    );
    table
}

/// The stage arguments of `S#_CONFIGURATION`, which a stage's enums fill in
/// the tag's order, as `sub_71B7B0` copies them.
fn generic_stage_macro(
    index: usize,
    color_mux: bool,
    alpha_mux: bool,
    values: &[i32; 26],
    fog: bool,
) -> String {
    let list = |range: std::ops::Range<usize>| {
        values[range]
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "{index},{},{color_mux},{},{},{alpha_mux},{},{fog}",
        list(0..8),
        list(8..14),
        list(14..22),
        list(22..26),
    )
}

/// The stage `sub_71B7B0` adds for a blend function's fade. Vertex inputs by
/// fade mode: its color input, then its alpha one.
pub(super) fn generic_fade_stage(blend: i32, fade: i32, index: usize) -> String {
    const SCRATCH: i32 = 11;
    let (color_input, alpha_input) = match fade {
        1 => (20, 10),
        2 => (10, 20),
        _ => (19, 9),
    };
    let mut values = [0; 26];
    let fade_alpha = |values: &mut [i32; 26]| {
        values[14] = SCRATCH;
        values[16] = alpha_input;
        values[24] = 1;
    };
    let fade_color = |values: &mut [i32; 26]| {
        values[0] = SCRATCH;
        values[2] = color_input;
        values[12] = 1;
    };
    match blend {
        0 => fade_alpha(&mut values),
        1 | 2 | 5 => {
            fade_color(&mut values);
            values[4] = if blend == 2 { 2 } else { 1 };
            values[6] = color_input;
            values[7] = 1;
        }
        7 => {
            fade_alpha(&mut values);
            fade_color(&mut values);
        }
        _ => fade_color(&mut values),
    }
    generic_stage_macro(index, false, false, &values, true)
}

/// A shader's enum values in tag order, with a name to start after.
fn enum_values(stage: TagStruct<'_>, after: &str) -> Vec<i32> {
    let mut started = false;
    stage
        .fields()
        .filter_map(|field| {
            let value = match field.value()? {
                TagFieldData::CharEnum { value, .. } => value as i32,
                TagFieldData::ShortEnum { value, .. } => value as i32,
                TagFieldData::LongEnum { value, .. } => value,
                _ => return None,
            };
            let keep = started;
            started |= field.clean_name() == after;
            keep.then_some(value)
        })
        .collect()
}

/// Which vertex stage draws the first map.
fn transparent_stage(first_map: i32, screenspace: bool) -> Dx9VertexStage {
    match first_map {
        1 => Dx9VertexStage::CeTransparentReflection,
        2 => Dx9VertexStage::CeTransparentObject,
        3 => Dx9VertexStage::CeTransparentViewer,
        _ if screenspace => Dx9VertexStage::CeTransparentScreen,
        _ => Dx9VertexStage::CeTransparent,
    }
}

/// Read a transparent shader's maps into textures and animations.
fn read_transparent_maps(
    maps: Option<&TagBlock<'_>>,
    cube_first: bool,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<Image>>,
) -> (Vec<Dx9DrawTexture>, Vec<CeMapAnimation>) {
    let mut textures = Vec::new();
    let mut animations = Vec::new();
    for slot in 0..4usize {
        let cube = slot == 0 && cube_first;
        let element = maps.and_then(|maps| maps.element(slot));
        let kind = if cube {
            SamplerKind::Cube
        } else {
            SamplerKind::Texture2D
        };
        let Some(element) = element else {
            textures.push(texture(slot as u32, kind, (!cube).then(|| flat(0))));
            continue;
        };
        let image = element
            .read_tag_ref_path("map")
            .filter(|path| !path.is_empty())
            .and_then(|path| bitmap(&path, kind));
        let has = |flag: &str| has_flag(element, "flags", flag);
        let image = match image.as_deref() {
            Some(Image::Flat(flat)) if has("u-clamped") || has("v-clamped") => {
                let mut clamped = flat.clone();
                clamped.repeat_x = !has("u-clamped");
                clamped.repeat_y = !has("v-clamped");
                Some(Arc::new(Image::Flat(clamped)))
            }
            _ => image,
        };
        textures.push(texture(slot as u32, kind, image));
        if !cube {
            let scale = [
                nonzero(element.read_real("map u-scale"), 1.0),
                nonzero(element.read_real("map v-scale"), 1.0),
            ];
            let offset = [
                element.read_real("map u-offset").unwrap_or(0.0),
                element.read_real("map v-offset").unwrap_or(0.0),
            ];
            let rotation = element.read_real("map rotation").unwrap_or(0.0);
            animations.push(read_map_animation(
                element,
                slot * 2,
                scale,
                offset,
                rotation,
            ));
        }
    }
    (textures, animations)
}

/// A `numeric` shader's digits, before they are split across passes.
fn first_map_digits(root: TagStruct<'_>) -> Option<CeNumeric> {
    let block = ["maps", "4 stage maps"]
        .into_iter()
        .find_map(|name| root.field(name).and_then(|field| field.as_block()))?;
    Some(CeNumeric {
        map: block
            .element(0)?
            .read_tag_ref_path("map")
            .filter(|path| !path.is_empty())?,
        count: 0,
        limit: root.read_int_any("numeric counter limit").unwrap_or(0) as i32,
        digit: 0,
        place: 0,
        timer: has_flag(root, "extra flags", "numeric countdown timer"),
    })
}

/// `cf_DecalZBiasValue`, as `rasterizer_model_draw_model_shader_pp` sets it.
pub(super) const MODEL_DECAL_DEPTH_BIAS: f32 = -5.5e-5;

/// `cf_TransparentDecalZBiasValue`, as `rasterizer_transparent_geometry_group_draw`
/// sets it. Neither has a slope term.
pub(super) const TRANSPARENT_DECAL_DEPTH_BIAS: f32 = -5e-6;

/// A transparent pass's state, whichever group fills its constants.
fn transparent_pass(
    root: TagStruct<'_>,
    first_map: i32,
    flags: &ShaderFlags<'_>,
    fragment: String,
    textures: Vec<Dx9DrawTexture>,
    constants: Vec<(u32, [f32; 4])>,
    animation: CeModelAnimation,
) -> Dx9DrawPass {
    let has = |flag: &str| flags.has(flag);
    let depth = Dx9Depth {
        compare: Some(Dx9Compare::LessOrEqual),
        write: false,
    };
    let stage = transparent_stage(first_map, has("first map is in screenspace"));
    let mut pass = Dx9DrawPass::new(stage, fragment, depth);
    pass.channels = [true, true, true, false];
    pass.blend = Some(FramebufferBlend::read(root).gl());
    pass.cull = if has("two-sided") {
        Dx9Cull::None
    } else {
        Dx9Cull::Back
    };
    pass.hidden_by_camo = !has("don't fade active-camouflage");
    pass.depth_bias = decal_bias(flags.has("decal"));
    pass.numeric = has("numeric").then(|| first_map_digits(root)).flatten();
    pass.textures = textures;
    pass.constants = constants;
    pass.texture_rows = identity_map_rows();
    let fade_source = function_source(root, "framebuffer fade source");
    pass.ce_animation = Some(Arc::new(CeModelAnimation {
        fade_source,
        ..animation
    }));
    pass
}

const FIRST_MAP_TYPES: [&str; 4] = [
    "2D map",
    "first map is reflection cube map",
    "first map is object-centered cube map",
    "first map is viewer-centered cube map",
];

const GENERIC_FIRST_MAP_TYPES: [&str; 4] = [
    "2D map",
    "reflection cube map",
    "object-centered cube map",
    "viewer-centered cube map",
];

fn generic_pass(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    flags: &ShaderFlags<'_>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<Image>>,
) -> Result<Dx9DrawPass, String> {
    let first_map = option_index(root, "first map type", &GENERIC_FIRST_MAP_TYPES);
    let maps = root.field("maps").and_then(|field| field.as_block());
    let (textures, maps) = read_transparent_maps(maps.as_ref(), first_map != 0, bitmap);

    let mut macros: Vec<(String, String)> = Vec::new();
    let mut constants: Vec<(u32, [f32; 4])> =
        (0..16).map(|register| (register, [0.0; 4])).collect();
    let mut colors = Vec::new();
    let stages = root.field("stages").and_then(|field| field.as_block());
    let count = stages.as_ref().map_or(0, |stages| stages.len());
    if count == 0 {
        let mut values = [0; 26];
        (values[0], values[2], values[8]) = (5, 1, 1);
        (values[14], values[16], values[22]) = (5, 1, 1);
        macros.push((
            "S0_CONFIGURATION".to_owned(),
            generic_stage_macro(0, false, false, &values, false),
        ));
    }
    for index in 0..count {
        let Some(stage) = stages.as_ref().and_then(|stages| stages.element(index)) else {
            continue;
        };
        let enums = enum_values(stage, "color0 animation function");
        let values: [i32; 26] = enums
            .try_into()
            .map_err(|_| "A stage has the wrong number of options.".to_owned())?;
        let has = |flag: &str| has_flag(stage, "flags", flag);
        macros.push((
            format!("S{index}_CONFIGURATION"),
            generic_stage_macro(index, has("color mux"), has("alpha mux"), &values, false),
        ));
        let lower = argb(stage, "color0 animation lower bound");
        let upper = argb(stage, "color0 animation upper bound");
        let period = nonzero(stage.read_real("color0 animation period"), 1.0);
        let function =
            option_index(stage, "color0 animation function", &PERIODIC_FUNCTIONS) as usize;
        let channel = color_channel(stage, "color0 source");
        constants[index].1 = lower;
        constants[index + 8].1 = argb(stage, "color1");
        colors.push(CeColorAnimation {
            register: index as u32,
            function,
            period,
            lower,
            upper,
            animates_alpha: true,
            channel,
            follows_output: has("A-out controls color0 animation"),
        });
    }
    let blend = FramebufferBlend::read(root).shader_number();
    let fade = option_index(root, "framebuffer fade mode", &FADE_MODES);
    let last = count.max(1);
    macros.push((
        format!("S{last}_CONFIGURATION"),
        generic_fade_stage(blend, fade, last),
    ));
    // Fog on and not planar: the last stage fades rather than fogs.
    constants.push((16, [1.0, 0.0, 0.0, 0.0]));

    let mut defines: Vec<(&str, &str)> = macros
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    if first_map != 0 {
        defines.push(("FIRST_MAP_IS_CUBE", "1"));
    }
    let read = |name: &str| read_shader_source(directory, name);
    let shader = translate_fx(GENERIC_FILE, PASS_GLSL_VERSION, &defines, read)?;
    let reference = flags.has("alpha-tested").then_some(ALPHA_REFERENCE);
    let fragment = shader.entry_tested("pixel_main(i)", reference);
    let animation = CeModelAnimation {
        maps,
        colors,
        ..Default::default()
    };
    Ok(transparent_pass(
        root, first_map, flags, fragment, textures, constants, animation,
    ))
}

fn chicago_pass(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    flags: &ShaderFlags<'_>,
    field: &str,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<Image>>,
) -> Result<Dx9DrawPass, String> {
    let first_map = option_index(root, "first map type", &FIRST_MAP_TYPES);
    let maps = root.field(field).and_then(|field| field.as_block());
    let (textures, animations) = read_transparent_maps(maps.as_ref(), first_map != 0, bitmap);

    let mut replicate = [0.0; 3];
    let mut functions = [(0, 0); 3];
    let count = maps.as_ref().map_or(0, |maps| maps.len());
    for (index, slot) in functions
        .iter_mut()
        .enumerate()
        .take(count.saturating_sub(1))
    {
        let Some(map) = maps.as_ref().and_then(|maps| maps.element(index)) else {
            continue;
        };
        *slot = (
            map.read_int_any("color function").unwrap_or(0) as i32,
            map.read_int_any("alpha function").unwrap_or(0) as i32,
        );
        replicate[index] = f32::from(u8::from(has_flag(map, "flags", "alpha replicate")));
    }
    let blend = match FramebufferBlend::read(root) {
        FramebufferBlend::AlphaBlend => 0,
        FramebufferBlend::Multiply | FramebufferBlend::ComponentMin => 1,
        FramebufferBlend::DoubleMultiply => 2,
        FramebufferBlend::AlphaMultiplyAdd => 4,
        FramebufferBlend::Add | FramebufferBlend::Subtract | FramebufferBlend::ComponentMax => 3,
    };
    let fade = option_index(root, "framebuffer fade mode", &FADE_MODES);
    let custom = flags.has("custom edition blending");
    let truth = |on: bool| f32::from(u8::from(on));

    let mut constants = vec![(0, [0.0; 4])];
    for (stage, (color, alpha)) in functions.into_iter().enumerate() {
        let (color, alpha) = (chicago_color_table(color), chicago_alpha_table(alpha));
        let first = 1 + 6 * stage as u32;
        for row in 0..3 {
            let at = row as usize * 4;
            constants.push((first + row, std::array::from_fn(|index| color[at + index])));
            constants.push((
                first + 3 + row,
                std::array::from_fn(|index| alpha.get(at + index).copied().unwrap_or(0.0)),
            ));
        }
    }
    constants.push((
        19,
        [replicate[0], replicate[1], replicate[2], truth(custom)],
    ));
    constants.push((
        20,
        [truth(fade == 0), truth(fade == 1), truth(fade == 2), 0.0],
    ));
    constants.push((
        21,
        [
            truth(blend == 0),
            truth(blend == 1),
            truth(blend == 2),
            truth(blend == 3),
        ],
    ));

    let cube = if first_map != 0 { "1" } else { "0" };
    let read = |name: &str| read_shader_source(directory, name);
    let shader = translate_fx(CHICAGO_FILE, PASS_GLSL_VERSION, &[("USE_CUBE", cube)], read)?;
    let reference = flags.has("alpha-tested").then_some(ALPHA_REFERENCE);
    let fragment = shader.entry_tested("pixel_main(i)", reference);
    let animation = CeModelAnimation {
        maps: animations,
        ..Default::default()
    };
    Ok(transparent_pass(
        root, first_map, flags, fragment, textures, constants, animation,
    ))
}

/// A transparent shader's passes: its extra layers, then itself. Empty for
/// any other group, or where the kit has no shaders.
pub(in crate::app::model_preview) fn ce_transparent_passes(
    reader: &TagReader,
    root: TagStruct<'_>,
    group: &[u8; 4],
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<Image>>,
) -> Vec<Dx9DrawPass> {
    let Some(directory) = shader_directory(reader) else {
        return Vec::new();
    };
    let single = match group {
        b"sgla" => Some(glass_passes(
            &directory,
            root,
            &ShaderFlags::of(root),
            bitmap,
        )),
        b"smet" => {
            let flags = ShaderFlags::of(root);
            Some(
                meter_pass(&directory, root, &flags, bitmap).map(|mut pass| {
                    pass.depth_bias = decal_bias(flags.has("decal"));
                    vec![pass]
                }),
            )
        }
        b"swat" => Some(water_passes(
            &directory,
            root,
            &ShaderFlags::of(root),
            bitmap,
        )),
        b"spla" => Some(plasma_pass(&directory, root, bitmap).map(|pass| vec![pass])),
        _ => None,
    };
    if let Some(passes) = single {
        return passes.unwrap_or_else(|error| {
            eprintln!("model preview: transparent shader: {error}");
            Vec::new()
        });
    }
    if !matches!(group, b"sotr" | b"schi" | b"scex") {
        return Vec::new();
    }
    let mut passes = Vec::new();
    let layers = root
        .field("extra layers")
        .and_then(|field| field.as_block());
    for index in 0..layers.as_ref().map_or(0, |layers| layers.len()) {
        let Some(path) = layers
            .as_ref()
            .and_then(|layers| layers.element(index))
            .and_then(|layer| layer.read_tag_ref_path("shader"))
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        for (extension, layer_group) in LAYER_GROUPS {
            if let Ok(tag) = reader.load(&path, extension, layer_group) {
                passes.extend(ce_transparent_passes(
                    reader,
                    tag.root(),
                    layer_group,
                    bitmap,
                ));
                break;
            }
        }
    }
    let flags = ShaderFlags::of(root);
    let pass = match group {
        b"sotr" => generic_pass(&directory, root, &flags, bitmap),
        b"schi" => chicago_pass(&directory, root, &flags, "maps", bitmap),
        _ => chicago_pass(&directory, root, &flags, "4 stage maps", bitmap),
    };
    match pass {
        Ok(pass) => passes.push(pass),
        Err(error) => {
            eprintln!("model preview: transparent shader: {error}");
            return Vec::new();
        }
    }
    passes
}
