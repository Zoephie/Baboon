//! Halo 4's `material`s: baked as the game bakes them at load (blam-tags
//! `material::bake`), then drawn through their own material shader's
//! translated passes. Objects take the path every loaded level gives them,
//! `albedo` then `static_probe` (`select_mesh_entry_point_for_pass`, MCC
//! 0x1408CE100: lightmaps available, no per-object policy flags); with no
//! level at all the game picks `static_per_vertex_color`, which no material
//! shader compiles. What blends draws `single_pass_single_probe`.

use super::*;
use crate::app::model_preview::h4::shaders::{H4_DEFINES, H4ShaderSource, translate_pass};
use crate::core::hlsl::emit::Stage;
use crate::core::hlsl::parse::Program;
use crate::app::model_preview::h4::submit::{H4HologramPost, H4ScreenMaterial, H4Submit};
use blam_tags::material::{Material, MaterialShader, bake, shader_flags};

/// Entry points (`entry_point_enum`) the passes are taken from.
const ENTRY_DEFAULT: usize = 0;
const ENTRY_ALBEDO: usize = 1;
const ENTRY_STATIC_PROBE: usize = 8;
const ENTRY_SINGLE_PASS_SINGLE_PROBE: usize = 23;

/// A material shader read once: its parameters and its parsed effect.
pub(super) struct H4Shader {
    shader: MaterialShader,
    program: Program,
    /// Translated passes by technique, kept for every material of the shader.
    passes: HashMap<&'static str, Option<Arc<H3Pass>>>,
}

/// `alpha_blend_mode_enum` as the preview's blend states. Kit objects use
/// opaque, alpha_blend, additive, opaque_alpha_blend, pre_multiplied_alpha,
/// add_src_times_srcalpha, multiply, inv_alpha_blend, alpha_blend_max and
/// maximum (`materials.md` §2.3).
pub(super) fn h4_blend(mode: u8) -> H3Blend {
    match mode {
        1 | 24 => H3Blend::Additive,
        2 => H3Blend::Multiply,
        // `alpha_blend_max` differs only in its alpha's MAX, which the preview
        // reads as coverage. One kit material uses it.
        3 | 14 | 22 => H3Blend::AlphaBlend,
        4 => H3Blend::DoubleMultiply,
        5 => H3Blend::PreMultiplied,
        6 => H3Blend::Maximum,
        7 => H3Blend::MultiplyAdd,
        8 => H3Blend::AddSrcTimesDstAlpha,
        9 => H3Blend::AddSrcTimesSrcAlpha,
        10 => H3Blend::InvAlphaBlend,
        11 => H3Blend::MotionBlurStatic,
        12 => H3Blend::MotionBlurInhibit,
        // `opaque_alpha_blend` counts as opaque (`GetIsOpaque`, X360
        // 0x82A9EA18); the rest are screen and shadow-mask modes no object uses.
        _ => H3Blend::Opaque,
    }
}

/// `bitmap_filter_mode` as stored, onto the shared enum's meanings.
fn h4_filter(filter: u8) -> BitmapFilterMode {
    match filter {
        1 => BitmapFilterMode::Point,
        2 => BitmapFilterMode::Bilinear,
        3 | 4 => BitmapFilterMode::Anisotropic2Expensive,
        5 => BitmapFilterMode::Anisotropic3Expensive,
        6 => BitmapFilterMode::Anisotropic4Expensive,
        _ => BitmapFilterMode::Trilinear,
    }
}

fn h4_address(address: u8) -> BitmapAddressMode {
    BitmapAddressMode::from_index(i128::from(address)).unwrap_or_default()
}

/// Whether a bitmap tag exists, and whether its first image is a cube.
fn bitmap_kind(source: &TagSource, path: &str, caches: &mut ResolveCaches) -> Option<bool> {
    if let Some(known) = caches.h4_bitmap_kinds.get(path) {
        return *known;
    }
    let found = load_referenced_tag_from_source(source, path, "bitmap", b"bitm").ok().map(|tag| {
        elements(tag.root(), "bitmaps*").first().and_then(|image| image.read_enum_name("type")).as_deref() == Some("cube map")
    });
    caches.h4_bitmap_kinds.insert(path.to_owned(), found);
    found
}

fn read_shader(source: &TagSource, path: &str) -> Result<H4Shader, String> {
    let tag = load_referenced_tag_from_source(source, path, "material_shader", b"mats").map_err(|error| error.to_string())?;
    let program = H4ShaderSource::read(source, path)?.program(&H4_DEFINES)?;
    Ok(H4Shader { shader: MaterialShader::from_tag(&tag), program, passes: HashMap::new() })
}

impl H4Shader {
    /// A technique drawn on the preview's skinned vertices.
    fn pass(&mut self, technique: &'static str) -> Option<Arc<H3Pass>> {
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
                Some(Arc::new(super::super::gen3::shaders::pass_of(vertex, pixel)))
            })
            .clone()
    }
}

/// A full-screen triangle for an explicit shader's pass, its texcoord running
/// up the picture as GL's targets do: v is GL's t. The hologram's screen
/// rectangles are handed over in the same terms (see `h4_holograms.rs`).
const SCREEN_VERTEX: &str = "#version 410 core
out vec2 v_TEXCOORD0;
void main() {
    vec2 corner = vec2(gl_VertexID & 1, gl_VertexID >> 1) * 4.0 - 1.0;
    gl_Position = vec4(corner, 0.0, 1.0);
    v_TEXCOORD0 = corner * 0.5 + 0.5;
}
";

/// [`SCREEN_VERTEX`], for the light passes.
pub(in crate::app::model_preview) fn h4_screen_vertex() -> &'static str {
    SCREEN_VERTEX
}

/// One of `rasterizer_globals`' explicit shaders that is a `material`, baked
/// as any material is, its one pass (`screen`) over the picture.
fn screen_material(source: &TagSource, path: &str, caches: &mut ResolveCaches) -> Result<H4ScreenMaterial, String> {
    let tag = load_referenced_tag_from_source(source, path, "material", b"mat ").map_err(|error| error.to_string())?;
    let material = Material::from_tag(&tag);
    let shader = match caches.h4_shaders.remove(&material.shader) {
        Some(known) => known.map_err(|error| error.to_string()),
        None => read_shader(source, &material.shader),
    }?;
    let pixel = translate_pass(&shader.program, "", "screen", Stage::Pixel);
    let baked = {
        let lookup = std::cell::RefCell::new(|path: &str| bitmap_kind(source, path, caches));
        bake(&material, &shader.shader, &|path: &str| (lookup.borrow_mut())(path))
    };
    caches.h4_shaders.insert(material.shader.clone(), Ok(shader));
    let pixel = pixel?;
    let pass = Arc::new(H3Pass {
        fullscreen: true,
        vertex: SCREEN_VERTEX.to_owned(),
        pixel: pixel.source,
        uniforms: pixel.uniforms,
        textures: pixel.textures,
    });
    let mut textures: Vec<H3Texture> = Vec::new();
    for texture in &baked.textures {
        let Some((uniform, _)) = texture_uniform(&pass.textures, &texture.parameter) else { continue };
        if textures.iter().any(|known| known.uniform == uniform) {
            continue;
        }
        let Some((image, texels, mips)) = h3_image(source, &texture.bitmap, 0, SamplerKind::Texture2D, caches) else { continue };
        let sampler = H3Sampler { address: [h4_address(texture.address_u), h4_address(texture.address_v)], filter: h4_filter(texture.filter) };
        textures.push(H3Texture { uniform: uniform.to_owned(), image, texels, mips, sampler: Some(sampler), frames: Vec::new() });
    }
    // `set_explicit_shaders` (X360 0x82AA4FC8) submits for every phase.
    let submit = H4Submit::new(baked, 0, 0, None);
    Ok(H4ScreenMaterial { values: submit.baked_values(), pass, submit, textures })
}

/// `rasterizer_globals`' explicit shaders 22 ("postprocess hologram") and 24
/// ("hologram composite"), whose pixel shaders are `material`s, read once.
fn hologram_post(source: &TagSource, caches: &mut ResolveCaches) -> Option<Arc<H4HologramPost>> {
    const POSTPROCESS_HOLOGRAM: usize = 22;
    const HOLOGRAM_COMPOSITE: usize = 24;
    if let Some(known) = &caches.h4_hologram_post {
        return known.clone();
    }
    let post = (|| {
        let globals =
            load_referenced_tag_from_source(source, "globals\\rasterizer_globals", "rasterizer_globals", b"rasg").ok()?;
        let explicit = elements(globals.root(), "explicit shaders");
        let path = |index: usize| explicit.get(index)?.read_tag_ref_path("explicit pixel shader").filter(|path| !path.is_empty());
        let (combine, composite) = (path(POSTPROCESS_HOLOGRAM)?, path(HOLOGRAM_COMPOSITE)?);
        Some(Arc::new(H4HologramPost {
            combine: screen_material(source, &combine, caches).ok()?,
            composite: screen_material(source, &composite, caches).ok()?,
        }))
    })();
    caches.h4_hologram_post = Some(post.clone());
    post
}

/// The pass's sampler for a parameter. Parameter names are string ids, which
/// hold lowercase; the source spells the sampler as it likes
/// (`control_map_SpGlRf`) and GLSL names are exact, so the pass's spelling
/// is the one to bind.
fn texture_uniform<'a>(wanted: &'a [(String, TextureKind)], parameter: &str) -> Option<(&'a str, &'a TextureKind)> {
    let lowercase = format!("{parameter}Texture");
    wanted.iter().find(|(name, _)| name.eq_ignore_ascii_case(&lowercase)).map(|(name, kind)| (name.as_str(), kind))
}

pub(super) fn resolve_h4_material(source: &TagSource, tag: &TagFile, caches: &mut ResolveCaches) -> MaterialTextures {
    let material = Material::from_tag(tag);
    if material.shader.is_empty() {
        return MaterialTextures::failed("the material names no material shader");
    }
    let shader = match caches.h4_shaders.remove(&material.shader) {
        Some(known) => known,
        None => read_shader(source, &material.shader).map_err(Arc::<str>::from),
    };
    let mut shader = match shader {
        Ok(shader) => shader,
        Err(error) => {
            caches.h4_shaders.insert(material.shader.clone(), Err(Arc::clone(&error)));
            return MaterialTextures::failed(format!("Halo 4 material shader not read: {error}"));
        }
    };
    let baked = {
        let lookup = |path: &str| bitmap_kind(source, path, caches);
        // The lookup needs the caches mutably; the bake only reads.
        let lookup = std::cell::RefCell::new(lookup);
        bake(&material, &shader.shader, &|path: &str| (lookup.borrow_mut())(path))
    };
    let blend = h4_blend(baked.alpha_blend_mode);
    // A decal shader's one technique (`techniques_decals.fxh`) writes the
    // albedo target, ahead of the lights unless the material puts it after
    // (`materialFlags` bit 1, `decal post lighting`).
    let decal = shader.shader.flags & blam_tags::material::shader_flags::IS_DECAL != 0;
    let hologram = shader.shader.flags & shader_flags::IS_HOLOGRAM != 0;
    let blended_hologram = shader.shader.flags & shader_flags::IS_BLENDED_HOLOGRAM != 0;
    // `c_render_hologram::render_object_callback` (X360 0x83B2CCD8): outside a
    // cinematic the transparent mode is 0, and `handle_object_part`
    // (0x83B2EB80) draws every hologram, blended ones too, in pass 0 alone:
    // opaque, depth written, back faces culled, through pass 16's entry,
    // `single_pass_single_probe`.
    let blend = if hologram || blended_hologram { H3Blend::Opaque } else { blend };
    let opaque = blend == H3Blend::Opaque;
    let single_pass = |shader: &mut H4Shader| shader.pass("single_pass_single_probe").map(|pass| (pass, ENTRY_SINGLE_PASS_SINGLE_PROBE));
    let (albedo, lit) = if hologram || blended_hologram {
        (None, single_pass(&mut shader))
    } else if decal {
        (None, shader.pass("_default").map(|pass| (pass, ENTRY_DEFAULT)))
    } else if opaque && let (Some(albedo), Some(lit)) = (shader.pass("albedo"), shader.pass("static_probe")) {
        (Some(albedo), Some((lit, ENTRY_STATIC_PROBE)))
    } else if opaque {
        // `opaque_alpha_blend` counts as transparent too (`GetIsTransparent`,
        // X360 0x82A9EAB0), and its blend state is off: a shader with no
        // deferred techniques (Cortana's hair compiles only `single_pass_*`)
        // shows through that pass, drawn as opaque.
        (None, single_pass(&mut shader))
    } else {
        (None, single_pass(&mut shader).or_else(|| shader.pass("albedo").map(|pass| (pass, ENTRY_ALBEDO))))
    };
    // `opaque_alpha_blend` (23) also draws in the transparent pass, through a
    // single-pass entry under `alpha_blend` (`material_submit_volatile_per_shader`,
    // X360 0x83B08008), for the edges its opaque pass clipped away.
    const OPAQUE_ALPHA_BLEND: u8 = 23;
    let fringe = (baked.alpha_blend_mode == OPAQUE_ALPHA_BLEND && albedo.is_some() && !hologram && !blended_hologram)
        .then(|| shader.pass("single_pass_single_probe"))
        .flatten();
    let mut textures = MaterialTextures::default();
    let result = match lit {
        Some((lit, lit_entry)) => {
            // `handle_object_part` clips a blended hologram below full alpha.
            let clip = (hologram || blended_hologram).then_some(if blended_hologram { 1.0 } else { 0.0 });
            let post = clip.and_then(|_| hologram_post(source, caches));
            let submit = H4Submit::new(baked.clone(), ENTRY_ALBEDO, lit_entry, clip).with_post(post);
            let mut h3 = H3Material {
                dialect: H3Dialect::Halo4,
                albedo: albedo.filter(|_| opaque),
                lit: Arc::clone(&lit),
                fringe,
                screen_maps: Vec::new(),
                decal,
                decal_modulates_specular: false,
                after_lighting: decal && material.flags & 0x2 != 0,
                sort_layer: i32::from(baked.sort_layer),
                prt: [None, None, None],
                camo: shader.pass("active_camo"),
                shadow_generate: shader.pass("shadow_generate").filter(|_| opaque),
                dynamic_light: None,
                blend,
                values: submit.baked_values(),
                textures: Vec::new(),
                change_colors: Vec::new(),
                animations: Vec::new(),
                shield: None,
                h4: Some(Arc::new(submit)),
            };
            let mut submit_frames = Vec::new();
            let passes = [h3.albedo.as_ref(), Some(&lit), h3.fringe.as_ref()];
            let wanted: Vec<(String, TextureKind)> =
                passes.into_iter().flatten().flat_map(|pass| pass.textures.iter().cloned()).collect();
            for texture in &baked.textures {
                let Some((uniform, kind)) = texture_uniform(&wanted, &texture.parameter) else { continue };
                let uniform = uniform.to_owned();
                if h3.textures.iter().any(|known| known.uniform == uniform) {
                    continue;
                }
                let kind = match kind {
                    TextureKind::Cube => SamplerKind::Cube,
                    TextureKind::Volume => SamplerKind::Volume,
                    _ => SamplerKind::Texture2D,
                };
                let Some((image, texels, mips)) = h3_image(source, &texture.bitmap, 0, kind, caches) else { continue };
                // A frame-index function picks among the bitmap's images per draw.
                let frames: Vec<_> = match texture.frame_index_function {
                    Some(function) => {
                        let tag = load_referenced_tag_from_source(source, &texture.bitmap, "bitmap", b"bitm").ok();
                        let count = tag.as_ref().map_or(0, |tag| elements(tag.root(), "bitmaps*").len());
                        if count > 1 {
                            submit_frames.push((uniform.clone(), function, count));
                        }
                        (0..count).filter_map(|at| h3_image(source, &texture.bitmap, at as i16, kind, caches)).map(|(image, ..)| image).collect()
                    }
                    None => Vec::new(),
                };
                h3.textures.push(H3Texture {
                    uniform,
                    image,
                    texels,
                    mips,
                    sampler: Some(H3Sampler {
                        address: [h4_address(texture.address_u), h4_address(texture.address_v)],
                        filter: h4_filter(texture.filter),
                    }),
                    frames,
                });
            }
            // `c_render_active_camo` binds `globals`' distortion texture at s13.
            let camo_texture = caches
                .h4_camo_texture
                .get_or_insert_with(|| super::super::h4::camo::H4Camo::read(source).and_then(|camo| camo.texture))
                .clone();
            let camo_uniform = h3.camo.as_ref().and_then(|pass| {
                pass.textures.iter().find(|(name, _)| name == "distortionTexture_texture").map(|(name, _)| name.clone())
            });
            if let (Some(path), Some(uniform)) = (camo_texture, camo_uniform)
                && let Some((image, texels, mips)) = h3_image(source, &path, 0, SamplerKind::Texture2D, caches)
            {
                // Address 0 (wrap), filter 2 (bilinear).
                let sampler = H3Sampler { address: [BitmapAddressMode::Wrap; 2], filter: BitmapFilterMode::Bilinear };
                h3.textures.push(H3Texture { uniform, image, texels, mips, sampler: Some(sampler), frames: Vec::new() });
            }
            // Still this material's alone, so the submit takes them in place.
            if let Some(submit) = h3.h4.as_mut().and_then(Arc::get_mut) {
                for (uniform, function, count) in submit_frames {
                    submit.add_frames(uniform, function, count);
                }
            }
            Ok(h3)
        }
        None => Err("no object pass compiled".to_owned()),
    };
    // The plain shaded preview still shows the colour map.
    if let Some(color) = baked.textures.iter().find(|texture| texture.parameter == "color_map") {
        let binding = SlotBitmap {
            path: color.bitmap.clone(),
            image_index: 0,
            repeat_x: true,
            repeat_y: true,
            scale: baked.texture_constants.get(usize::from(color.sampler)).map_or([1.0, 1.0], |xform| [xform[0], xform[1]]),
        };
        textures.slots[TextureSlot::Base as usize] = cached_bitmap(source, &binding, caches);
    }
    match result {
        Ok(h3) => textures.h3 = Some(Arc::new(h3)),
        Err(error) => textures.error = Some(format!("Halo 4 material not drawn: {error}")),
    }
    caches.h4_shaders.insert(material.shader, Ok(shader));
    textures
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Didact's whole suit glowed and the Chief turned to chrome while
    /// `control_map_spglrf` missed `control_map_SpGlRfTexture` and the white
    /// placeholder stood in for it.
    #[test]
    fn a_parameter_finds_its_sampler_whatever_the_source_spells_it() {
        let wanted = vec![
            ("color_mapTexture".to_owned(), TextureKind::Flat),
            ("control_map_SpGlRfTexture".to_owned(), TextureKind::Flat),
            ("reflection_mapTexture".to_owned(), TextureKind::Cube),
        ];
        assert_eq!(texture_uniform(&wanted, "control_map_spglrf").map(|(name, _)| name), Some("control_map_SpGlRfTexture"));
        assert_eq!(texture_uniform(&wanted, "reflection_map").map(|(_, kind)| *kind), Some(TextureKind::Cube));
        assert_eq!(texture_uniform(&wanted, "control_map"), None, "a prefix is not a match");
    }

    #[test]
    fn blend_modes_take_the_preview_states_their_names_describe() {
        assert_eq!(h4_blend(0), H3Blend::Opaque);
        assert_eq!(h4_blend(23), H3Blend::Opaque, "opaque_alpha_blend counts as opaque");
        assert_eq!(h4_blend(3), H3Blend::AlphaBlend);
        assert_eq!(h4_blend(24), H3Blend::Additive, "alpha_blend_additive_transparent");
        assert_eq!(h4_blend(9), H3Blend::AddSrcTimesSrcAlpha);
        assert_eq!(h4_blend(10), H3Blend::InvAlphaBlend);
    }
}

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
    source: &TagSource,
    material: &Material,
    vertex_pass: &str,
) -> Result<H4EffectMaterial, String> {
    let mut caches = ResolveCaches::default();
    let caches = &mut caches;
    if material.shader.is_empty() {
        return Err("the material names no material shader".to_owned());
    }
    let shader = read_shader(source, &material.shader)?;
    let vertex = translate_pass(&shader.program, "_default", vertex_pass, Stage::Vertex)?;
    let pixel = translate_pass(&shader.program, "_default", "_default", Stage::Pixel)?;
    let mut pass = super::super::gen3::shaders::pass_of(vertex, pixel);
    // The depth fade reads the preview's depth, where the pixel shader has one.
    if pass.pixel.contains("psDepthConstants") {
        super::super::h4::shield::rewire_depth_fade(&mut pass)?;
    }
    let pass = Arc::new(pass);
    let baked = {
        let lookup = std::cell::RefCell::new(|path: &str| bitmap_kind(source, path, caches));
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
        let Some((image, texels, mips)) = h3_image(source, &texture.bitmap, 0, sampler_kind, caches) else { continue };
        // A texture array holds the bitmap's images as its layers.
        let frames = if *kind == TextureKind::FlatArray {
            let tag = load_referenced_tag_from_source(source, &texture.bitmap, "bitmap", b"bitm").ok();
            let count = tag.as_ref().map_or(1, |tag| elements(tag.root(), "bitmaps*").len().max(1));
            (0..count)
                .filter_map(|at| h3_image(source, &texture.bitmap, at as i16, SamplerKind::Texture2D, caches))
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
