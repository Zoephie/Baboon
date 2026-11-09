//! Active camouflage: its settings and its pass.

use super::base::{
    CAMO_FILE, SimplePass, plain_blend, read_shader_source, shader_directory, texture,
};
use super::{
    Dx9BlendFactor, Dx9DrawPass, Dx9VertexStage, H2_GLSL_VERSION, H2_TEXTURE_ROWS, Image,
    SamplerKind, TagReader, has_flag, translate_fx,
};
use std::sync::Arc;

/// What `globals` gives every object's active camouflage.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app::model_preview) struct CeCamoSettings {
    fragment: String,
    effect_register: u32,
    screen_register: u32,
    pub(super) refraction: f32,
    falloff: f32,
    tint: [f32; 3],
    pub(super) distortion: Option<String>,
}

/// `globals\globals`'s rasterizer data, and the kit's shader for it.
pub(in crate::app::model_preview) fn read_ce_camo_settings(
    reader: &TagReader,
) -> Option<CeCamoSettings> {
    let directory = shader_directory(reader)?;
    let globals = reader.load("globals\\globals", "globals", b"matg").ok()?;
    let root = globals.root();
    let data = root.descend("rasterizer data[0]")?;
    let tint = data.read_rgb("tint color");
    let read = |name: &str| read_shader_source(&directory, name);
    let shader = translate_fx(CAMO_FILE, H2_GLSL_VERSION, &[], read).ok()?;
    let entry = if has_flag(data, "flags", "tint edge density") {
        "main_T0_P1(i)"
    } else {
        "main_T0_P0(i)"
    };
    Some(CeCamoSettings {
        fragment: shader.entry(entry),
        effect_register: shader.register("c_effect_intensity")?,
        screen_register: shader.register("c_split_screen_offset_and_scale")?,
        refraction: data.read_real("refraction amount").unwrap_or(0.0),
        falloff: data.read_real("distance falloff").unwrap_or(0.0),
        tint: [tint.red, tint.green, tint.blue],
        distortion: data
            .read_tag_ref_path("active camouflage distortion")
            .filter(|path| !path.is_empty()),
    })
}

/// `rasterizer_active_camouflage_draw` at full intensity. A cheap copy of the
/// frame behind stands in for its own primary target, as Halo 2's camo does.
pub(in crate::app::model_preview) fn ce_camo_pass(
    settings: &CeCamoSettings,
    two_sided: bool,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<Image>>,
) -> Dx9DrawPass {
    /// `refraction amount` is in pixels of the half-width the game draws at.
    const HALF_WIDTH: f32 = 320.0;
    let distortion = settings
        .distortion
        .as_deref()
        .and_then(|path| bitmap(path, SamplerKind::Cube));
    let mut frame = texture(2, SamplerKind::Texture2D, None);
    frame.frame_copy = true;
    let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
    rows[0] = [settings.refraction / HALF_WIDTH, settings.falloff, 0.0, 0.0];
    let [red, green, blue] = settings.tint;
    rows[1] = [red, green, blue, 0.0];
    SimplePass {
        stage: Dx9VertexStage::CeActiveCamo,
        fragment: settings.fragment.clone(),
        blend: plain_blend(Dx9BlendFactor::One, Dx9BlendFactor::Zero),
        two_sided,
        write_depth: true,
        channels: [true, true, true, false],
        textures: vec![texture(0, SamplerKind::Cube, distortion), frame],
        constants: vec![
            (settings.effect_register, [1.0; 4]),
            (settings.screen_register, [0.0, 1.0, 0.0, 1.0]),
        ],
        rows,
    }
    .build()
}
