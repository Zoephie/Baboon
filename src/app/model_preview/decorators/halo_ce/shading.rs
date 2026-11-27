//! Halo CE's detail objects with Game Shaders: their colour as Sapien bakes
//! it, and their pass as the game draws it.
//!
//! The pass (`_rasterizer_detail_objects_begin`, 360 0x83712F18) translates
//! the kit's `shaders\vsh\detail_object_type0.vsh` (screen-facing) or
//! `type1.vsh` (viewer-facing) and `shaders\psh\modulate.psh`, blends by
//! alpha, tests depth without writing it, culls nothing, and writes colour
//! alone. The vertex shaders read their constants from tables
//! `detail_object_build_vertices` and the begin fill (`c13` to `c18`, a type's
//! from `c19`, a sprite's from `c29`), which [`halo_ce_vertex_constants`]
//! fills from the collection.

use super::super::super::ModelGpuFrame;
use super::super::super::dx9::passes::{
    Dx9Blend, Dx9BlendEquation, Dx9BlendFactor, Dx9Compare, Dx9Cull, Dx9Depth, Dx9DrawPass,
    Dx9DrawTexture,
};
use super::super::super::dx9::stage::Dx9VertexStage;
use super::super::super::load::TagReader;
use super::super::super::materials::decode_image;
use super::{DetailType, HaloCeCollection};
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::hlsl::emit::{Stage, translate};
use crate::core::hlsl::parse::parse;
use crate::core::hlsl::pre::{Lines, Preprocessor};
use std::path::Path;
use std::sync::Arc;

/// Which way a collection's detail objects turn (`collection type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum DetailFacing {
    /// The eye's right and up (`detail_object_type0.vsh`).
    #[default]
    ScreenFacing,
    /// About the vertical, toward the eye (`detail_object_type1.vsh`).
    ViewerFacing,
}

impl DetailFacing {
    pub(crate) fn from_option_name(name: &str) -> Option<Self> {
        match name {
            "screen-facing" => Some(Self::ScreenFacing),
            "viewer-facing" => Some(Self::ViewerFacing),
            _ => None,
        }
    }

    fn vertex_shader(self) -> &'static str {
        match self {
            Self::ScreenFacing => "detail_object_type0.vsh",
            Self::ViewerFacing => "detail_object_type1.vsh",
        }
    }
}

/// Where a Halo CE editing kit keeps the shader source Game Shaders
/// translate, beside `shaders\fx`.
fn shader_folder(reader: &TagReader, folder: &str) -> Option<std::path::PathBuf> {
    Some(reader.kit_root()?.join("shaders").join(folder))
}

/// The pass a collection of `facing` draws its plate `plate` with.
pub(in crate::app::model_preview) fn detail_object_pass(
    reader: &TagReader,
    facing: DetailFacing,
    plate: &str,
) -> Result<Dx9DrawPass, String> {
    let vertex_folder =
        shader_folder(reader, "vsh").ok_or("no editing kit root for its shader source")?;
    let pixel_folder =
        shader_folder(reader, "psh").ok_or("no editing kit root for its shader source")?;
    let vertex = translate_file(&vertex_folder, facing.vertex_shader(), Stage::Vertex)?;
    let pixel = translate_file(&pixel_folder, "modulate.psh", Stage::Pixel)?;
    let (vertex, pixel) = (clamped_colors(&vertex), pixel_reading(&pixel, &vertex)?);
    let depth = Dx9Depth {
        compare: Some(Dx9Compare::LessOrEqual),
        write: false,
    };
    let mut pass = Dx9DrawPass::new(Dx9VertexStage::DetailObject(facing), pixel, depth);
    pass.vertex_source = Some(Arc::from(vertex));
    pass.blend = Some(Dx9Blend {
        equation: Dx9BlendEquation::Add,
        source: Dx9BlendFactor::SrcAlpha,
        destination: Dx9BlendFactor::OneMinusSrcAlpha,
        constant: [0; 4],
    });
    pass.cull = Dx9Cull::None;
    pass.channels = [true, true, true, false];
    pass.textures = vec![Dx9DrawTexture {
        slot: 0,
        kind: SamplerKind::Texture2D,
        image: decode_image(reader, plate, SamplerKind::Texture2D, 0).map(Arc::new),
        normalization: false,
        texaccum: false,
        frame_copy: false,
        falloff: false,
        gel: false,
    }];
    Ok(pass)
}

/// The kit's `file` in `folder`, its includes beside it, translated.
fn translate_file(folder: &Path, file: &str, stage: Stage) -> Result<String, String> {
    let path = folder.join(file);
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut include = |name: &str, _: &str| {
        let text = std::fs::read_to_string(folder.join(name)).ok()?;
        Some((Arc::<str>::from(name), Arc::new(Lines::of(&text))))
    };
    let tokens = Preprocessor::new(&[], &mut include).run(&text)?;
    Ok(translate(&parse(&tokens)?, "main", stage)?.source)
}

/// A translated vertex stage with its colours held to 0 to 1, as Direct3D
/// 9 holds a vertex shader's `oD0` and `oD1`: a detail object's fade runs
/// past either end of its range.
fn clamped_colors(vertex: &str) -> String {
    let Some(end) = vertex.rfind('}') else {
        return vertex.to_owned();
    };
    let clamps: String = ["v_COLOR0", "v_COLOR1"]
        .iter()
        .filter(|name| vertex.contains(&format!("out vec4 {name};")))
        .map(|name| format!("    {name} = clamp({name}, 0.0, 1.0);\n"))
        .collect();
    format!("{}{clamps}{}", &vertex[..end], &vertex[end..])
}

/// A translated pixel stage reading what `vertex` writes: an input narrower
/// than the vertex stage's output (`modulate.psh` takes `TEXCOORD0` as two)
/// takes it whole and reads its first components; its sampler is the dx9
/// pass's first, `s0`.
fn pixel_reading(pixel: &str, vertex: &str) -> Result<String, String> {
    let mut pixel = pixel.to_owned();
    for line in pixel.clone().lines() {
        let Some((width, name)) = line
            .strip_prefix("in vec")
            .and_then(|rest| rest.split_once(' '))
        else {
            continue;
        };
        let name = name.trim_end_matches(';');
        let (Ok(width), true) = (
            width.parse::<usize>(),
            vertex.contains(&format!("out vec4 {name};")),
        ) else {
            continue;
        };
        if width < 4 {
            let swizzle = &"xyzw"[..width];
            pixel = pixel.replace(line, &format!("in vec4 {name};"));
            pixel = pixel.replace(&format!(" = {name};"), &format!(" = {name}.{swizzle};"));
        }
    }
    if !pixel.contains("S0Tex") {
        return Err("modulate.psh samples no S0Tex".to_owned());
    }
    Ok(pixel.replace("S0Tex", "s0"))
}

/// `c13`: the colour bytes' scale to an index (`255.01`), the least a
/// screen-facing object's lift is, and the vertical (`.yyzy`).
const DETAIL_OBJECT_OFFSET: [f32; 4] = [255.01, 0.0, 1.0, 1.0];

/// `c14`: a cell's size and how far a screen-facing object is lifted toward
/// the eye looking down (`rasterizer_debug_options`' default, 0.4).
const DETAIL_OBJECT_CELL: [f32; 4] = [8.0, 8.0, 8.0, 0.4];

/// `c15` to `c18`: each corner's texcoord across its sprite and its place,
/// across from −½ to ½ and up from 0 to 1, standing on its bottom edge.
const CORNERS: [[f32; 4]; 4] = [
    [0.0, 1.0, -0.5, 0.0],
    [1.0, 1.0, 0.5, 0.0],
    [1.0, 0.0, 0.5, 1.0],
    [0.0, 0.0, -0.5, 1.0],
];

const TYPE_DATA: usize = 19;
const FRAME_DATA: usize = 29;
const EYE_XFORM: usize = 27;

/// The detail object vertex shaders' constants (`float4 c[...]`) for
/// `frame`'s view of `collection`.
pub(in crate::app::model_preview) fn halo_ce_vertex_constants(
    frame: &ModelGpuFrame,
    collection: &HaloCeCollection,
    // Whether the Distance card's fade is on.
    fade: bool,
) -> Vec<[f32; 4]> {
    use super::super::super::gen3::pipeline::view_projection;
    use super::super::super::{eye_position, unrotate_view_vector};
    let sprites: Vec<_> = collection.plate.sequences.iter().flatten().collect();
    let mut constants = vec![[0.0; 4]; FRAME_DATA + sprites.len()];
    let columns = view_projection(&frame.camera);
    for (row, constant) in constants.iter_mut().take(4).enumerate() {
        *constant = std::array::from_fn(|column| columns[column * 4 + row]);
    }
    let (yaw, pitch) = (frame.camera.yaw, frame.camera.pitch);
    let axis = |vector| {
        let [x, y, z] = unrotate_view_vector(yaw, pitch, vector);
        [x, y, z, 0.0]
    };
    let [x, y, z] = eye_position(&frame.camera);
    // `.w` is `C_TWO`.
    constants[4] = [x, y, z, 2.0];
    constants[5] = axis([0.0, 1.0, 0.0]);
    constants[13] = DETAIL_OBJECT_OFFSET;
    constants[14] = DETAIL_OBJECT_CELL;
    constants[15..19].copy_from_slice(&CORNERS);
    for (index, kind) in collection
        .types
        .iter()
        .enumerate()
        .take(EYE_XFORM - TYPE_DATA)
    {
        constants[TYPE_DATA + index] = type_data(collection, kind, fade);
    }
    constants[EYE_XFORM] = axis([1.0, 0.0, 0.0]);
    constants[EYE_XFORM + 1] = axis([0.0, 0.0, 1.0]);
    for (index, sprite) in sprites.iter().enumerate() {
        let [left, right, top, bottom] = sprite.bounds;
        constants[FRAME_DATA + index] = [left, top, right - left, bottom - top];
    }
    constants
}

/// `c19 + type`: its fade, `(far − depth) / (far − near)` as `depth · y + x`
/// (none at all where far is no further than near, and whole with the fade
/// off), and its sequence's first image's size in texels times its `size`.
fn type_data(collection: &HaloCeCollection, kind: &DetailType, fade: bool) -> [f32; 4] {
    let image = collection
        .plate
        .sequences
        .get(kind.sequence)
        .and_then(|sprites| sprites.first())
        .and_then(|sprite| collection.plate.images.get(sprite.image))
        .copied()
        .unwrap_or([0.0; 2]);
    let span = kind.far_fade - kind.near_fade;
    let (scale, offset) = match (fade, span > 0.0) {
        (false, _) => (0.0, 1.0),
        (true, true) => (-1.0 / span, kind.far_fade / span),
        (true, false) => (0.0, 0.0),
    };
    [offset, scale, image[0] * kind.size, image[1] * kind.size]
}

/// The light Sapien's colour reads off the lightmap under a detail object
/// (`sub_427EC0`): 0.5 grey where it finds none, plus the 0.1 it adds.
const GROUND_LIGHT: f32 = 0.6;

/// The colour it reads off the ground's base map: 0.5 grey where none.
const GROUND_COLOR: f32 = 0.5;

/// A detail object's colour as Sapien bakes it (`sub_423590`): between the
/// type's least and most colour at `along`, in HSV where it says so, lit by
/// the ground's light and its ambient colour, tinted by the ground's colour
/// but for its override, and kept in RGB565.
pub(in crate::app::model_preview) fn detail_object_color(
    kind: &DetailType,
    along: f32,
    // The Lighting card's level, of the ground light's stand-in.
    light: f32,
) -> [f32; 3] {
    let mixed = if kind.hsv {
        hsv_mix(kind.colors, along, kind.more_colors)
    } else {
        std::array::from_fn(|channel| {
            kind.colors[0][channel] + (kind.colors[1][channel] - kind.colors[0][channel]) * along
        })
    };
    let [ambient_alpha, ambient @ ..] = kind.ambient;
    let color: [f32; 3] = std::array::from_fn(|channel| {
        let base = ambient[channel] * ambient_alpha + (1.0 - ambient_alpha) * GROUND_LIGHT * light;
        let tint = (mixed[channel]
            * ((1.0 - kind.color_override) * GROUND_COLOR + kind.color_override))
            .min(1.0);
        (base * tint).clamp(0.0, 1.0)
    });
    rgb565(color)
}

/// `sub_578670` in HSV: the hue the short way round the wheel, or with
/// `...more colors` the long way.
fn hsv_mix(colors: [[f32; 3]; 2], along: f32, more_colors: bool) -> [f32; 3] {
    let [mut first, mut second] = colors.map(to_hsv);
    if ((first[0] - second[0]).abs() > 0.5) != more_colors {
        if first[0] < second[0] {
            first[0] += 1.0;
        } else {
            second[0] += 1.0;
        }
    }
    let mut mixed: [f32; 3] =
        std::array::from_fn(|at| first[at] + (second[at] - first[at]) * along);
    if mixed[0] > 1.0 {
        mixed[0] -= 1.0;
    }
    from_hsv(mixed)
}

fn to_hsv([r, g, b]: [f32; 3]) -> [f32; 3] {
    let most = r.max(g).max(b);
    let least = r.min(g).min(b);
    let span = most - least;
    let hue = if span <= 0.0 {
        0.0
    } else if most == r {
        ((g - b) / span).rem_euclid(6.0) / 6.0
    } else if most == g {
        ((b - r) / span + 2.0) / 6.0
    } else {
        ((r - g) / span + 4.0) / 6.0
    };
    [hue, if most > 0.0 { span / most } else { 0.0 }, most]
}

fn from_hsv([hue, saturation, value]: [f32; 3]) -> [f32; 3] {
    let sixth = (hue * 6.0).rem_euclid(6.0);
    let chroma = value * saturation;
    let x = chroma * (1.0 - ((sixth % 2.0) - 1.0).abs());
    let [r, g, b] = match sixth as u32 {
        0 => [chroma, x, 0.0],
        1 => [x, chroma, 0.0],
        2 => [0.0, chroma, x],
        3 => [0.0, x, chroma],
        4 => [x, 0.0, chroma],
        _ => [chroma, 0.0, x],
    };
    let least = value - chroma;
    [r + least, g + least, b + least]
}

/// A colour kept in RGB565 as Sapien packs it (each channel cut to its
/// bits, `sub_423590`) and read back to eight bits a channel by repeating
/// its high bits [inferred: `detail_object_build_vertices`' widening].
fn rgb565(color: [f32; 3]) -> [f32; 3] {
    let byte = |channel: f32| (channel * 255.0) as u32;
    let widen = |bits: u32, count: u32| {
        let wide = bits << (8 - count) | bits >> (2 * count - 8);
        wide as f32 / 255.0
    };
    let [r, g, b] = color.map(byte);
    [widen(r >> 3, 5), widen(g >> 2, 6), widen(b >> 3, 5)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without `...more colors` the hue takes the short way round: red to
    /// magenta through the pinks, not the greens; with it, the long way.
    #[test]
    fn hsv_takes_the_way_round_the_flag_names() {
        let colors = [[1.0, 0.0, 0.0], [1.0, 0.0, 1.0]];
        let short = hsv_mix(colors, 0.5, false);
        assert!(
            short[1] < 0.01 && short[0] > 0.9 && short[2] > 0.4,
            "{short:?}"
        );
        let long = hsv_mix(colors, 0.5, true);
        assert!(long[1] > 0.9, "the long way crosses green: {long:?}");
    }

    /// RGB565 keeps five, six and five bits: white stays white, and a
    /// channel between steps falls to the one below.
    #[test]
    fn rgb565_keeps_its_bits() {
        assert_eq!(rgb565([1.0; 3]), [1.0; 3]);
        let [r, g, _] = rgb565([0.5, 0.5, 0.0]);
        assert!(r < 0.5 && (r - 0.5).abs() < 8.0 / 255.0, "{r}");
        assert!(g <= 0.5 && (g - 0.5).abs() < 4.0 / 255.0, "{g}");
    }

    /// A type fades between its near and far fade distance with the fade on,
    /// and stays whole with it off.
    #[test]
    fn a_types_fade_follows_the_distance_card() {
        let kind = DetailType {
            near_fade: 3.0,
            far_fade: 8.0,
            ..Default::default()
        };
        let collection = HaloCeCollection {
            types: vec![kind.clone()],
            ..Default::default()
        };
        let [offset, scale, ..] = type_data(&collection, &kind, true);
        assert!((3.0 * scale + offset - 1.0).abs() < 1e-5 && (8.0 * scale + offset).abs() < 1e-5);
        let [offset, scale, ..] = type_data(&collection, &kind, false);
        assert_eq!((offset, scale), (1.0, 0.0));
    }

    /// A translated pixel stage takes what the vertex stage writes whole and
    /// reads its first components, and samples the pass's first unit.
    #[test]
    fn the_pixel_stage_reads_what_the_vertex_stage_writes() {
        let vertex = "out vec4 v_TEXCOORD0;\nvoid main() {\n    v_TEXCOORD0 = vec4(1.0);\n}\n";
        let pixel = "in vec2 v_TEXCOORD0;\nuniform sampler2D S0Tex;\nvoid main() {\n    vec2 hlsl_p0;\n    hlsl_p0 = v_TEXCOORD0;\n}\n";
        let read = pixel_reading(pixel, vertex).unwrap();
        assert!(
            read.contains("in vec4 v_TEXCOORD0;") && read.contains("hlsl_p0 = v_TEXCOORD0.xy;"),
            "{read}"
        );
        assert!(read.contains("uniform sampler2D s0;"), "{read}");
        let clamped =
            clamped_colors("out vec4 v_COLOR0;\nvoid main() {\n    v_COLOR0 = vec4(2.0);\n}\n");
        assert!(
            clamped.ends_with("    v_COLOR0 = clamp(v_COLOR0, 0.0, 1.0);\n}\n"),
            "{clamped}"
        );
    }
}
