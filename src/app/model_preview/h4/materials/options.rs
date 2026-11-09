//! A Halo 4 shader's entry points, blend and sampler options, as read off the tag.

use super::*;

/// Entry points (`entry_point_enum`) the passes are taken from.
pub(super) const ENTRY_DEFAULT: usize = 0;

pub(super) const ENTRY_ALBEDO: usize = 1;

pub(super) const ENTRY_STATIC_PROBE: usize = 8;

pub(super) const ENTRY_SINGLE_PASS_SINGLE_PROBE: usize = 23;

/// A material shader read once: its parameters and its parsed effect.
pub(in crate::app::model_preview) struct H4Shader {
    pub(super) shader: MaterialShader,
    pub(super) program: Program,
    /// Translated passes by technique, kept for every material of the shader.
    passes: HashMap<&'static str, Option<Arc<Gen3Pass>>>,
}

/// `alpha_blend_mode_enum` as the preview's blend states. Kit objects use
/// opaque, alpha_blend, additive, opaque_alpha_blend, pre_multiplied_alpha,
/// add_src_times_srcalpha, multiply, inv_alpha_blend, alpha_blend_max and
/// maximum (`materials.md` §2.3).
pub(in crate::app::model_preview) fn h4_blend(mode: u8) -> Gen3Blend {
    match mode {
        1 | 24 => Gen3Blend::Additive,
        2 => Gen3Blend::Multiply,
        // `alpha_blend_max` differs only in its alpha's MAX, which the preview
        // reads as coverage. One kit material uses it.
        3 | 14 | 22 => Gen3Blend::AlphaBlend,
        4 => Gen3Blend::DoubleMultiply,
        5 => Gen3Blend::PreMultiplied,
        6 => Gen3Blend::Maximum,
        7 => Gen3Blend::MultiplyAdd,
        8 => Gen3Blend::AddSrcTimesDstAlpha,
        9 => Gen3Blend::AddSrcTimesSrcAlpha,
        10 => Gen3Blend::InvAlphaBlend,
        11 => Gen3Blend::MotionBlurStatic,
        12 => Gen3Blend::MotionBlurInhibit,
        // `opaque_alpha_blend` counts as opaque (`GetIsOpaque`, X360
        // 0x82A9EA18); the rest are screen and shadow-mask modes no object uses.
        _ => Gen3Blend::Opaque,
    }
}

/// `bitmap_filter_mode` as stored, onto the shared enum's meanings.
pub(super) fn h4_filter(filter: u8) -> BitmapFilterMode {
    match filter {
        1 => BitmapFilterMode::Point,
        2 => BitmapFilterMode::Bilinear,
        3 | 4 => BitmapFilterMode::Anisotropic2Expensive,
        5 => BitmapFilterMode::Anisotropic3Expensive,
        6 => BitmapFilterMode::Anisotropic4Expensive,
        _ => BitmapFilterMode::Trilinear,
    }
}

pub(super) fn h4_address(address: u8) -> BitmapAddressMode {
    BitmapAddressMode::from_index(i128::from(address)).unwrap_or_default()
}

/// Whether a bitmap tag exists, and whether its first image is a cube.
pub(super) fn bitmap_kind(reader: &TagReader, path: &str, caches: &mut ResolveCaches) -> Option<bool> {
    if let Some(known) = caches.h4_bitmap_kinds.get(path) {
        return *known;
    }
    let found = reader.load(path, "bitmap", b"bitm").ok().map(|tag| {
        elements(tag.root(), "bitmaps*").first().and_then(|image| image.field("type")).is_some_and(|kind| kind.enum_is("cube map"))
    });
    caches.h4_bitmap_kinds.insert(path.to_owned(), found);
    found
}

pub(super) fn read_shader(reader: &TagReader, path: &str) -> Result<H4Shader, String> {
    let tag = reader.load(path, "material_shader", b"mats").map_err(|error| error.to_string())?;
    let program = H4ShaderSource::read(reader, path)?.program(&H4_DEFINES)?;
    Ok(H4Shader { shader: MaterialShader::from_tag(&tag), program, passes: HashMap::new() })
}

impl H4Shader {
    /// A technique drawn on the preview's skinned vertices.
    pub(super) fn pass(&mut self, technique: &'static str) -> Option<Arc<Gen3Pass>> {
        let program = &self.program;
        self.passes
            .entry(technique)
            .or_insert_with(|| {
                if !program.techniques.iter().any(|candidate| &*candidate.name == technique) {
                    return None;
                }
                let vertex = translate_pass(program, technique, "skinned", Stage::Vertex).ok()?;
                // `MAKE_TECHNIQUE_VS_ONLY`: the plain `shadow_generate` writes
                // depth alone; the textured override clips in its own.
                let pixel = match translate_pass(program, technique, "_default", Stage::Pixel) {
                    Ok(pixel) => pixel,
                    Err(_) if technique == "shadow_generate" => crate::core::hlsl::emit::Translated {
                        source: "#version 410 core\nvoid main() {}\n".to_owned(),
                        uniforms: Vec::new(),
                        textures: Vec::new(),
                        attributes: Vec::new(),
                    },
                    Err(_) => return None,
                };
                Some(Arc::new(super::super::super::gen3::shaders::pass_of(vertex, pixel)))
            })
            .clone()
    }
}
