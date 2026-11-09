//! A Halo 4 material resolved for draw, with the screen and hologram materials.

use super::*;

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
fn screen_material(reader: &TagReader, path: &str, caches: &mut ResolveCaches) -> Result<H4ScreenMaterial, String> {
    let tag = reader.load(path, "material", b"mat ").map_err(|error| error.to_string())?;
    let material = Material::from_tag(&tag);
    let shader = match caches.h4_shaders.remove(&material.shader) {
        Some(known) => known.map_err(|error| error.to_string()),
        None => read_shader(reader, &material.shader),
    }?;
    let pixel = translate_pass(&shader.program, "", "screen", Stage::Pixel);
    let baked = {
        let lookup = std::cell::RefCell::new(|path: &str| bitmap_kind(reader, path, caches));
        bake(&material, &shader.shader, &|path: &str| (lookup.borrow_mut())(path))
    };
    caches.h4_shaders.insert(material.shader.clone(), Ok(shader));
    let pixel = pixel?;
    let pass = Arc::new(Gen3Pass {
        fullscreen: true,
        vertex: SCREEN_VERTEX.to_owned(),
        pixel: pixel.source,
        uniforms: pixel.uniforms,
        textures: pixel.textures,
    });
    let mut textures: Vec<Gen3Texture> = Vec::new();
    for texture in &baked.textures {
        let Some((uniform, _)) = texture_uniform(&pass.textures, &texture.parameter) else { continue };
        if textures.iter().any(|known| known.uniform == uniform) {
            continue;
        }
        let Some((image, texels, mips)) = gen3_image(reader, &texture.bitmap, 0, SamplerKind::Texture2D, caches) else { continue };
        let sampler = Sampler { address: [h4_address(texture.address_u), h4_address(texture.address_v)], filter: h4_filter(texture.filter) };
        textures.push(Gen3Texture { uniform: uniform.to_owned(), image, texels, mips, sampler: Some(sampler), frames: Vec::new() });
    }
    // `set_explicit_shaders` (X360 0x82AA4FC8) submits for every phase.
    let submit = H4Submit::new(baked, 0, 0, None);
    Ok(H4ScreenMaterial { values: submit.baked_values(), pass, submit, textures })
}

/// `rasterizer_globals`' explicit shaders 22 ("postprocess hologram") and 24
/// ("hologram composite"), whose pixel shaders are `material`s, read once.
fn hologram_post(reader: &TagReader, caches: &mut ResolveCaches) -> Option<Arc<H4HologramPost>> {
    const POSTPROCESS_HOLOGRAM: usize = 22;
    const HOLOGRAM_COMPOSITE: usize = 24;
    if let Some(known) = &caches.h4_hologram_post {
        return known.clone();
    }
    let post = (|| {
        let globals =
            reader.load("globals\\rasterizer_globals", "rasterizer_globals", b"rasg").ok()?;
        let explicit = elements(globals.root(), "explicit shaders");
        let path = |index: usize| explicit.get(index)?.read_tag_ref_path("explicit pixel shader").filter(|path| !path.is_empty());
        let (combine, composite) = (path(POSTPROCESS_HOLOGRAM)?, path(HOLOGRAM_COMPOSITE)?);
        Some(Arc::new(H4HologramPost {
            combine: screen_material(reader, &combine, caches).ok()?,
            composite: screen_material(reader, &composite, caches).ok()?,
        }))
    })();
    caches.h4_hologram_post = Some(post.clone());
    post
}

/// The pass's sampler for a parameter. Parameter names are string ids, which
/// hold lowercase; the source spells the sampler as it likes
/// (`control_map_SpGlRf`) and GLSL names are exact, so the pass's spelling
/// is the one to bind.
pub(super) fn texture_uniform<'a>(wanted: &'a [(String, TextureKind)], parameter: &str) -> Option<(&'a str, &'a TextureKind)> {
    let lowercase = format!("{parameter}Texture");
    wanted.iter().find(|(name, _)| name.eq_ignore_ascii_case(&lowercase)).map(|(name, kind)| (name.as_str(), kind))
}

pub(in crate::app::model_preview) fn resolve_h4_material(reader: &TagReader, tag: &TagFile, caches: &mut ResolveCaches) -> MaterialTextures {
    let material = Material::from_tag(tag);
    if material.shader.is_empty() {
        return MaterialTextures::failed("the material names no material shader");
    }
    let shader = match caches.h4_shaders.remove(&material.shader) {
        Some(known) => known,
        None => read_shader(reader, &material.shader).map_err(Arc::<str>::from),
    };
    let mut shader = match shader {
        Ok(shader) => shader,
        Err(error) => {
            caches.h4_shaders.insert(material.shader.clone(), Err(Arc::clone(&error)));
            return MaterialTextures::failed(format!("Halo 4 material shader not read: {error}"));
        }
    };
    let baked = {
        let lookup = |path: &str| bitmap_kind(reader, path, caches);
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
    let blend = if hologram || blended_hologram { Gen3Blend::Opaque } else { blend };
    let opaque = blend == Gen3Blend::Opaque;
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
            let post = clip.and_then(|_| hologram_post(reader, caches));
            let submit = H4Submit::new(baked.clone(), ENTRY_ALBEDO, lit_entry, clip).with_post(post);
            let values = submit.baked_values();
            let mut h3 = Gen3Material {
                dialect: Gen3MaterialDialect::Halo4(Arc::new(submit)),
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
                values,
                textures: Vec::new(),
                change_colors: Vec::new(),
                animations: Vec::new(),
                shield: None,
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
                let Some((image, texels, mips)) = gen3_image(reader, &texture.bitmap, 0, kind, caches) else { continue };
                // A frame-index function picks among the bitmap's images per draw.
                let frames: Vec<_> = match texture.frame_index_function {
                    Some(function) => {
                        let tag = reader.load(&texture.bitmap, "bitmap", b"bitm").ok();
                        let count = tag.as_ref().map_or(0, |tag| elements(tag.root(), "bitmaps*").len());
                        if count > 1 {
                            submit_frames.push((uniform.clone(), function, count));
                        }
                        (0..count).filter_map(|at| gen3_image(reader, &texture.bitmap, at as i16, kind, caches)).map(|(image, ..)| image).collect()
                    }
                    None => Vec::new(),
                };
                h3.textures.push(Gen3Texture {
                    uniform,
                    image,
                    texels,
                    mips,
                    sampler: Some(Sampler {
                        address: [h4_address(texture.address_u), h4_address(texture.address_v)],
                        filter: h4_filter(texture.filter),
                    }),
                    frames,
                });
            }
            // `c_render_active_camo` binds `globals`' distortion texture at s13.
            let camo_texture = caches
                .h4_camo_texture
                .get_or_insert_with(|| super::super::super::h4::camo::H4Camo::read(reader).and_then(|camo| camo.texture))
                .clone();
            let camo_uniform = h3.camo.as_ref().and_then(|pass| {
                pass.textures.iter().find(|(name, _)| name == "distortionTexture_texture").map(|(name, _)| name.clone())
            });
            if let (Some(path), Some(uniform)) = (camo_texture, camo_uniform)
                && let Some((image, texels, mips)) = gen3_image(reader, &path, 0, SamplerKind::Texture2D, caches)
            {
                // Address 0 (wrap), filter 2 (bilinear).
                let sampler = Sampler { address: [BitmapAddressMode::Wrap; 2], filter: BitmapFilterMode::Bilinear };
                h3.textures.push(Gen3Texture { uniform, image, texels, mips, sampler: Some(sampler), frames: Vec::new() });
            }
            // Still this material's alone, so the submit takes them in place.
            if let Some(submit) = h3.h4_mut().and_then(Arc::get_mut) {
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
        textures.slots[TextureSlot::Base as usize] = cached_bitmap(reader, &binding, caches);
    }
    match result {
        Ok(h3) => textures.h3 = Some(Arc::new(h3)),
        Err(error) => textures.error = Some(format!("Halo 4 material not drawn: {error}")),
    }
    caches.h4_shaders.insert(material.shader, Ok(shader));
    textures
}
