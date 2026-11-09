//! Resolving a model's materials into their textures.

use super::*;

/// Resolve every material of one model to its textures.
///
/// Runs on a worker: it reads and decodes a shader tag plus several bitmaps per
/// material, which is far too much to do inside a frame.
pub(crate) fn resolve_model_textures(
    source: &TagSource,
    materials: &[RenderModelPreviewMaterial],
) -> Vec<MaterialTextures> {
    // The materials' own reads, once each for this resolve.
    let reader = &TagReader::new(source);
    let mut caches = ResolveCaches {
        kit_names: match source {
            TagSource::LooseFolder {
                game: Some(game),
                definitions_root,
                ..
            } => TagNameIndex::load_game(definitions_root, *game).ok(),
            _ => None,
        },
        ..Default::default()
    };
    let mut textures: Vec<MaterialTextures> = materials
        .iter()
        .map(|material| {
            // Guarded per material: blam-tags' enum resolver panics on names
            // a custom kit's recompiled shader tags can carry, and a panic
            // here kills the worker before it sends its message — leaving the
            // viewport on "Loading shaders…" forever.
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                resolve_one_material(reader, material, &mut caches)
            }))
            .unwrap_or_else(|_| {
                MaterialTextures::failed(
                    "this shader crashed the reader — its tags likely use names \
                     this build of blam-tags does not know",
                )
            })
        })
        .collect();
    share_object_layers(materials, &mut textures);
    for texture in &mut textures {
        // A pass with no object implementation still has the fallback's.
        if texture.h2_layers[H2Layer::ObjectLightmap as usize].is_empty() {
            texture.h2_layers[H2Layer::ObjectLightmap as usize] =
                texture.h2_layers[H2Layer::Lightmap as usize].clone();
        }
        let lit = [H2Layer::Lightmap, H2Layer::Prt]
            .iter()
            .flat_map(|layer| &texture.h2_layers[*layer as usize])
            .any(|pass| pass.stage != H2VertexStage::CeModel);
        if lit || texture.h3.is_some() {
            for (layer, pass) in h2_shadow_passes() {
                texture.h2_layers[layer as usize] = vec![pass];
            }
        }
    }
    textures
}

/// A model's hologram shader is on no batch: every material of its object
/// takes its passes. So do its damage shaders, which take their turn over the
/// object; a hologram shader's bloom passes go to the frame's bloom with the
/// rest. A held weapon is an object of its own, without them.
pub(super) fn share_object_layers(materials: &[RenderModelPreviewMaterial], textures: &mut [MaterialTextures]) {
    let transparent = H2Layer::Transparent as usize;
    let bloom = H2Layer::Bloom as usize;
    if let Some(overlay) = materials.iter().position(|material| material.overlay) {
        let passes = std::mem::take(&mut textures[overlay].h2_layers[transparent]);
        share_with_object(materials, textures, materials[overlay].object, H2Layer::Hologram as usize, &passes);
    }
    for (layer, from) in
        [(H2Layer::ShieldDamaged, transparent), (H2Layer::Overshield, transparent), (H2Layer::HologramBloom, bloom)]
    {
        if let Some(owner) = materials.iter().position(|material| material.shield == Some(layer)) {
            let passes = std::mem::take(&mut textures[owner].h2_layers[from]);
            share_with_object(materials, textures, materials[owner].object, layer as usize, &passes);
        }
    }
}

/// Give every material of `object` a copy of `passes` as its `layer`.
fn share_with_object(
    materials: &[RenderModelPreviewMaterial],
    textures: &mut [MaterialTextures],
    object: u8,
    layer: usize,
    passes: &[H2DrawPass],
) {
    for (material, texture) in materials.iter().zip(textures) {
        if material.object == object {
            texture.h2_layers[layer] = passes.to_vec();
        }
    }
}

fn resolve_one_material(
    reader: &TagReader,
    material: &RenderModelPreviewMaterial,
    caches: &mut ResolveCaches,
) -> MaterialTextures {
    if material.shader_path.is_empty() {
        return MaterialTextures::failed("no shader assigned");
    }
    let extension = caches
        .kit_names
        .as_ref()
        .and_then(|names| names.name_for(material.shader_group))
        .map(str::to_owned)
        .or_else(|| blam_tags::paths::group_tag_to_extension(material.shader_group).map(str::to_owned));
    let Some(extension) = extension else {
        return MaterialTextures::failed(format!(
            "unknown shader group {:?}",
            material.shader_group.to_be_bytes()
        ));
    };
    let group = material.shader_group.to_be_bytes();
    let shader =
        match reader.load(&material.shader_path, &extension, &group) {
            Ok(tag) => tag,
            Err(error) => return MaterialTextures::failed(error.to_string()),
        };
    if group == *b"mat " {
        return super::super::h4::materials::resolve_h4_material(reader, &shader, caches);
    }
    match blam_tags::game::Game::of(&shader) {
        blam_tags::game::Game::Halo1 => {
            return resolve_ce_shader(reader, &shader, group, material.permutation, caches);
        },
        blam_tags::game::Game::Halo2 if group == *b"shad" => {
            return resolve_h2_shader(reader, &shader, caches);
        }
        _ => {}
    }
    let Ok(mut render_method) = RenderMethod::from_tag(&shader) else {
        // Halo CE and Halo 2 shaders are not render methods at all; they carry
        // their bitmaps as fixed schema fields instead. Naming that is more use
        // than "failed to parse".
        return MaterialTextures::failed("not a render-method shader (Halo 3 and Reach only)");
    };

    // The render-method definition is the *better* route — it lets the walker
    // apply option defaults for values the shader does not author itself, and
    // the detail maps' tiling is exactly such a value. It is not required
    // though: a definition that will not parse still leaves every explicitly
    // authored bitmap working, which is nearly all of them.
    let definition = cached_render_method_definition(&reader.source, &render_method.definition_path, &mut caches.definitions);
    if let Some(definition) = &definition
        && reader.source.game() == Some(GameId::HaloReach)
    {
        reach_version_update(&mut render_method, definition);
        let reference = reach_reference(reader, &render_method, &material.shader_path, definition, &mut caches.references);
        render_method.apply_reference_shader(definition, reference.as_deref());
    }
    let resolved = definition.map(|definition| {
        ResolvedRenderMethod::resolve(&render_method, &definition, |option_path| {
            // The engine's resolver takes owned options; this runs once per
            // texture resolve on a worker, not per frame.
            cached_render_method_option(&reader.source, option_path, &mut caches.options)
                .map(|option| (*option).clone())
        })
    });

    let shield_only = render_method.flags.contains(blam_tags::render_method::GlobalRenderMethodFlags::OnlyRenderForShields);
    let never_shield = render_method.flags.contains(blam_tags::render_method::GlobalRenderMethodFlags::NeverRenderForShields);
    let values = Values {
        resolved: resolved.as_ref(),
        method: &render_method,
        sort_layer: shader.root().descend("render_method").map_or(2, super::super::particles::h3_sort_layer),
    };
    let mut textures = MaterialTextures {
        used_shader_parameters_only: resolved.is_none(),
        shield_only,
        never_shield,
        ..Default::default()
    };
    for (slot, parameter) in SLOT_PARAMETERS {
        let Some(binding) = slot_binding(&values, parameter) else {
            continue;
        };
        textures.slots[slot as usize] = cached_bitmap(reader, &binding, caches);
    }
    // Foliage is the one family whose base alpha IS coverage: leaves are
    // authored as cutouts in the base map and the shader often names no
    // `alpha_test_map` at all. Everywhere else a base map's alpha carries a
    // mask (usually specular), which is why this stays scoped to `rmfl`
    // instead of becoming a general fallback.
    if material.shader_group.to_be_bytes() == *b"rmfl"
        && textures.slots[TextureSlot::AlphaTest as usize].is_none()
    {
        textures.slots[TextureSlot::AlphaTest as usize] =
            textures.slots[TextureSlot::Base as usize].clone();
    }
    // `Game::of` calls every tag after Halo 2 `Halo3`, Reach's included.
    if blam_tags::game::Game::of(&shader) == blam_tags::game::Game::Halo3 {
        match prepare_h3_material(reader, &values, caches) {
            Ok(material) => textures.h3 = Some(Arc::new(material)),
            // The general shader still draws it, so this is only worth saying
            // when there is nothing else to show.
            Err(error) if textures.slots.iter().all(Option::is_none) => {
                textures.error = Some(format!("Halo 3 shader not translated: {error}"));
            }
            Err(_) => {}
        }
    }
    textures
}

/// Decode a map in the shape its sampler asks for.
pub(in crate::app::model_preview) fn decode_h2_image(reader: &TagReader, path: &str, kind: SamplerKind, index: usize) -> Option<H2Image> {
    let tag = reader.load(path, "bitmap", b"bitm").ok()?;
    if kind == SamplerKind::Texture2D {
        let image = decode_thumbnail(&tag, index, MAX_TEXTURE_EDGE).ok()?;
        return Some(H2Image::Flat(TextureImage {
            rgba: image.rgba,
            width: image.width,
            height: image.height,
            repeat_x: true,
            repeat_y: true,
            scale: [1.0, 1.0],
        }));
    }
    let bitmap = blam_tags::bitmap::Bitmap::new(&tag).ok()?;
    let image = bitmap.image(index)?;
    let (width, height) = (image.width(), image.height());
    let format = image.format().ok()?;
    let decode = |data: &[u8]| {
        blam_tags::bitmap::decode::decode_to_rgba8(format, width, height, data, bitmap.p8_palette())
            .ok()
    };
    if kind == SamplerKind::Cube {
        if !image.is_cube() {
            return None;
        }
        let face = |index: usize| decode(image.cube_face_bytes(index as u32).ok()?);
        let faces = [face(0)?, face(1)?, face(2)?, face(3)?, face(4)?, face(5)?];
        return Some(H2Image::Cube { size: width as usize, faces });
    }
    let depth = image.depth() as usize;
    let mut rgba = Vec::new();
    for index in 0..depth {
        let slice = if image.is_array() {
            array_layer_bytes(&image, index as u32, 0)?
        } else {
            image.volume_slice_bytes(index as u32).ok()?
        };
        rgba.extend(decode(slice)?);
    }
    Some(H2Image::Volume { width: width as usize, height: height as usize, depth, rgba })
}

/// One layer of a texture array at mip `level`. Each layer keeps its whole
/// chain, one after another (`globals\global_render_texture` is 14 chains
/// of 21844 bytes), where a volume's slices lie together a level at a time.
pub(in crate::app::model_preview) fn array_layer_bytes<'a>(
    image: &blam_tags::bitmap::BitmapImage<'a>,
    layer: u32,
    level: u32,
) -> Option<&'a [u8]> {
    let format = image.format().ok()?;
    let (width, height) = (image.width(), image.height());
    let chain = format.surface_bytes(width, height, image.mipmap_levels());
    let before = format.surface_bytes(width, height, level);
    let size = format.level_bytes((width >> level).max(1), (height >> level).max(1));
    let start = usize::try_from(layer as u64 * chain + before).ok()?;
    image.pixel_bytes().ok()?.get(start..start.checked_add(size as usize)?)
}
