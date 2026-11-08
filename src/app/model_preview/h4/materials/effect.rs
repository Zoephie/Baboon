//! An effect's material: its shader resolved for the effect passes.

use super::*;

/// An effect's `material` (a particle's, tracer's or light volume's embedded
/// one), baked as any material is, its vertex shader the system's own pass
/// of the shader's `_default` technique (`particle_techniques.fxh`).
#[derive(Debug)]
pub(crate) struct H4EffectMaterial {
    pub(crate) pass: Arc<H3Pass>,
    pub(crate) submit: H4Submit,
    pub(crate) values: crate::app::model_preview::gen3::shaders::H3Values,
    /// Each sampler's bitmap; a texture array's carries every image in
    /// `frames`, its layers in order.
    pub(crate) textures: Vec<H3Texture>,
    /// `alpha_blend_mode_enum`.
    pub(crate) blend_mode: u8,
    /// The first bitmap in postprocess order, which the particle reads its
    /// sprites and tight bounds from (`postprocess_frame_animation`).
    pub(crate) first_bitmap: Option<String>,
    /// The material's own parameters over the shader's defaults, by name.
    reals: std::collections::BTreeMap<String, f32>,
    bools: std::collections::BTreeMap<String, bool>,
}

impl H4EffectMaterial {
    pub(crate) fn real(&self, name: &str) -> Option<f32> {
        self.reals.get(&name.to_ascii_lowercase()).copied()
    }

    pub(crate) fn flag(&self, name: &str) -> bool {
        self.bools.get(&name.to_ascii_lowercase()).copied().unwrap_or(false)
    }
}

/// Bake `material` and translate its `_default` technique's `vertex_pass`
/// with its `_default` pixel shader.
pub(in crate::app::model_preview) fn resolve_h4_effect_material(
    reader: &TagReader,
    material: &Material,
    vertex_pass: &str,
) -> Result<H4EffectMaterial, String> {
    let mut caches = ResolveCaches::default();
    let caches = &mut caches;
    if material.shader.is_empty() {
        return Err("the material names no material shader".to_owned());
    }
    let shader = read_shader(reader, &material.shader)?;
    let vertex = translate_pass(&shader.program, "_default", vertex_pass, Stage::Vertex)?;
    let pixel = translate_pass(&shader.program, "_default", "_default", Stage::Pixel)?;
    let mut pass = super::super::super::gen3::shaders::pass_of(vertex, pixel);
    // The depth fade reads the preview's depth, where the pixel shader has one.
    if pass.pixel.contains("psDepthConstants") {
        super::shaders::rewire_depth_fade(&mut pass)?;
    }
    let pass = Arc::new(pass);
    let baked = {
        let lookup = std::cell::RefCell::new(|path: &str| bitmap_kind(reader, path, caches));
        bake(material, &shader.shader, &|path: &str| (lookup.borrow_mut())(path))
    };
    // A parameter's value: the material's, else the shader's default.
    let mut reals = std::collections::BTreeMap::new();
    let mut bools = std::collections::BTreeMap::new();
    for parameter in shader.shader.parameters.iter().chain(&material.parameters) {
        let name = parameter.name.to_ascii_lowercase();
        reals.insert(name.clone(), parameter.real);
        bools.insert(name, parameter.int_bool != 0);
    }
    let mut textures: Vec<H3Texture> = Vec::new();
    for texture in &baked.textures {
        let Some((uniform, kind)) = texture_uniform(&pass.textures, &texture.parameter) else { continue };
        if textures.iter().any(|known| known.uniform == uniform) {
            continue;
        }
        let sampler_kind = match kind {
            TextureKind::Cube => SamplerKind::Cube,
            TextureKind::Volume => SamplerKind::Volume,
            _ => SamplerKind::Texture2D,
        };
        let Some((image, texels, mips)) = h3_image(reader, &texture.bitmap, 0, sampler_kind, caches) else { continue };
        // A texture array holds the bitmap's images as its layers.
        let frames = if *kind == TextureKind::FlatArray {
            let tag = reader.load(&texture.bitmap, "bitmap", b"bitm").ok();
            let count = tag.as_ref().map_or(1, |tag| elements(tag.root(), "bitmaps*").len().max(1));
            (0..count)
                .filter_map(|at| h3_image(reader, &texture.bitmap, at as i16, SamplerKind::Texture2D, caches))
                .map(|(image, ..)| image)
                .collect()
        } else {
            Vec::new()
        };
        let sampler = H3Sampler { address: [h4_address(texture.address_u), h4_address(texture.address_v)], filter: h4_filter(texture.filter) };
        textures.push(H3Texture { uniform: uniform.to_owned(), image, texels, mips, sampler: Some(sampler), frames });
    }
    let first_bitmap = baked.textures.first().map(|texture| texture.bitmap.clone()).filter(|path| !path.is_empty());
    let blend_mode = baked.alpha_blend_mode;
    let submit = H4Submit::new(baked, ENTRY_DEFAULT, ENTRY_DEFAULT, None);
    Ok(H4EffectMaterial { pass, values: submit.baked_values(), submit, textures, blend_mode, first_bitmap, reals, bools })
}
