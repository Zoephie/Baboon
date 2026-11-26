//! Resolving a model's materials into their textures.

use super::super::super::{TagNameIndex, TagSource};
use super::super::common::fields::elements;
use super::super::{
    MaterialTextures, RenderModelPreviewMaterial, ResolveCaches, TagReader, TextureImage,
    TextureSlot, Values, cached_render_method_definition, cached_render_method_option,
    decode_thumbnail,
};
use super::binding::{cached_bitmap, slot_binding};
use super::slots::{MAX_TEXTURE_EDGE, SLOT_PARAMETERS};
use super::{Image, SamplerKind, prepare_gen3_material, reference_shader, shader_version_update};
use blam_tags::render_method::{RenderMethod, ResolvedRenderMethod};
use std::sync::Arc;

/// Resolve every material of one model to its textures.
///
/// Runs on a worker: it reads and decodes a shader tag plus several bitmaps per
/// material, which is far too much to do inside a frame.
///
/// Without `game_shaders` ([`TagReader::game_shaders`]), the Halo 3, ODST,
/// Reach and Halo 4 materials keep their slot maps only: translating their
/// passes and decoding the maps those read at full size was most of a
/// resolve's time, for a look only Game Shaders draws. Halo CE's and Halo 2's
/// DX9 passes, which Game Shaders alone draws too, still resolve: they cost a
/// few milliseconds, and whether a material has any decides what the footer
/// says of it drawn without them.
pub(crate) fn resolve_model_textures(
    source: &TagSource,
    // The editing kit's root folder, as the kit names it ([`TagReader::kit_root`]).
    kit_root: Option<std::path::PathBuf>,
    materials: &[RenderModelPreviewMaterial],
    game_shaders: bool,
) -> Vec<MaterialTextures> {
    // The materials' own reads, once each for this resolve.
    let reader = &TagReader::with_game_shaders(source, game_shaders).with_kit_root(kit_root);
    let mut caches = resolve_caches(source);
    let mut textures: Vec<MaterialTextures> = materials
        .iter()
        .map(|material| guarded_resolve(reader, material, &mut caches))
        .collect();
    finish_materials(materials, &mut textures);
    textures
}

/// The caches one model's resolve shares across its materials, with the
/// kit's own group names.
pub(in crate::app::model_preview) fn resolve_caches(source: &TagSource) -> ResolveCaches {
    ResolveCaches {
        kit_names: match source {
            TagSource::LooseFolder {
                game: Some(game),
                definitions_root,
                ..
            } => TagNameIndex::load_game(definitions_root, *game).ok(),
            _ => None,
        },
        ..Default::default()
    }
}

/// [`resolve_one_material`], guarded: blam-tags' enum resolver panics on
/// names a custom kit's recompiled shader tags can carry, and a panic here
/// kills the worker before it sends its message, leaving the viewport on
/// "Loading shaders…" forever.
fn guarded_resolve(reader: &TagReader, material: &RenderModelPreviewMaterial, caches: &mut ResolveCaches) -> MaterialTextures {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| resolve_one_material(reader, material, caches)))
        .unwrap_or_else(|_| {
            MaterialTextures::failed(
                "this shader crashed the reader — its tags likely use names \
                 this build of blam-tags does not know",
            )
        })
}

/// What follows every material's resolve: each keeps its own DX9 passes,
/// then an object's overlay and damage passes are shared among its
/// materials, with the preview's shadow passes.
fn finish_materials(materials: &[RenderModelPreviewMaterial], textures: &mut [MaterialTextures]) {
    for texture in textures.iter_mut() {
        texture.own_dx9_layers = texture.dx9_layers.clone();
    }
    super::super::dx9::resolved::finish_dx9_layers(
        &materials[..textures.len().min(materials.len())],
        textures,
        &super::super::load::shadow_passes(),
    );
}

/// The materials at `indices` resolved again, the rest as `previous` has
/// them: the Material Editor's edit of one shader, which every material
/// naming it draws. `shader`, when given, is that shader's document, read in
/// place of its file. An object's shared passes are shared again from every
/// material's own, so an overlay shader edited hands its passes out anew,
/// and a plain one keeps those it was given.
///
/// `caches` live across the editor's resolves of one load: the shader's
/// own entries are dropped first, the rest (decoded maps, translated
/// includes, definitions) are what makes one material quick.
pub(in crate::app::model_preview) fn resolve_model_materials(
    source: &TagSource,
    // As [`resolve_model_textures`]'s.
    kit_root: Option<std::path::PathBuf>,
    materials: &[RenderModelPreviewMaterial],
    previous: &[MaterialTextures],
    indices: &[u16],
    shader: Option<Arc<blam_tags::TagFile>>,
    game_shaders: bool,
    caches: &mut ResolveCaches,
) -> Vec<MaterialTextures> {
    let reader = &TagReader::with_game_shaders(source, game_shaders).with_kit_root(kit_root);
    let mut textures = previous.to_vec();
    for &index in indices {
        let index = usize::from(index);
        let (Some(material), true) = (materials.get(index), index < textures.len()) else {
            continue;
        };
        caches.references.remove(&material.shader_path.to_ascii_lowercase());
        if let (Some(shader), Some(extension)) = (&shader, material_extension(material, caches)) {
            reader.seed(&material.shader_path, &extension, &material.shader_group.to_be_bytes(), Arc::clone(shader));
        }
        let mut resolved = guarded_resolve(reader, material, caches);
        resolved.own_dx9_layers = resolved.dx9_layers.clone();
        textures[index] = resolved;
    }
    // Shared again from every material's own passes, the new ones' too.
    for texture in textures.iter_mut() {
        texture.dx9_layers = texture.own_dx9_layers.clone();
    }
    finish_materials(materials, &mut textures);
    textures
}

/// The extension `material`'s shader reference resolves with: the kit's own
/// name for its group, else the cross-game table's.
fn material_extension(material: &RenderModelPreviewMaterial, caches: &ResolveCaches) -> Option<String> {
    caches
        .kit_names
        .as_ref()
        .and_then(|names| names.name_for(material.shader_group))
        .map(str::to_owned)
        .or_else(|| blam_tags::paths::group_tag_to_extension(material.shader_group).map(str::to_owned))
}

fn resolve_one_material(
    reader: &TagReader,
    material: &RenderModelPreviewMaterial,
    caches: &mut ResolveCaches,
) -> MaterialTextures {
    if material.shader_path.is_empty() {
        return MaterialTextures::failed("no shader assigned");
    }
    let Some(extension) = material_extension(material, caches) else {
        return MaterialTextures::failed(format!(
            "unknown shader group {:?}",
            material.shader_group.to_be_bytes()
        ));
    };
    let group = material.shader_group.to_be_bytes();
    let shader = match reader.load(&material.shader_path, &extension, &group) {
        Ok(tag) => tag,
        Err(error) => return MaterialTextures::failed(error.to_string()),
    };
    if let Some(textures) =
        super::super::load::resolve_own_shader(reader, &shader, group, material, caches)
    {
        return textures;
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
    let definition = cached_render_method_definition(
        &reader.source,
        &render_method.definition_path,
        &mut caches.definitions,
    );
    if let Some(definition) = &definition
        && reader.rules().gen3.reference_shaders
    {
        shader_version_update(&mut render_method, definition);
        let reference = reference_shader(
            reader,
            &render_method,
            &material.shader_path,
            definition,
            &mut caches.references,
        );
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

    let shield_only = render_method
        .flags
        .contains(blam_tags::render_method::GlobalRenderMethodFlags::OnlyRenderForShields);
    let never_shield = render_method
        .flags
        .contains(blam_tags::render_method::GlobalRenderMethodFlags::NeverRenderForShields);
    let values = Values {
        resolved: resolved.as_ref(),
        method: &render_method,
        sort_layer: shader
            .root()
            .descend("render_method")
            .map_or(2, super::super::particles::gen3_sort_layer),
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
    if reader.game_shaders() && blam_tags::game::Game::of(&shader) == blam_tags::game::Game::Halo3
    {
        match prepare_gen3_material(reader, &values, caches) {
            Ok(material) => textures.gen3 = Some(Arc::new(material)),
            Err(error) => {
                // The general shader still draws it, so it only goes untextured
                // when there is nothing else to show.
                if textures.slots.iter().all(Option::is_none) {
                    textures.error = Some(format!("Halo 3 shader not translated: {error}"));
                }
                textures.game_shaders_error = Some(error);
            }
        }
    }
    textures
}

/// `globals`' `rasterizer data`, whose `glow` stands in for a Halo CE or
/// Halo 2 light volume's bitmap.
pub(in crate::app::model_preview) fn read_glow(reader: &TagReader) -> Option<Arc<Image>> {
    let globals = reader.load("globals\\globals", "globals", b"matg").ok()?;
    let data = elements(globals.root(), "rasterizer data");
    let path = data
        .first()?
        .read_tag_ref_path("glow")
        .filter(|path| !path.is_empty())?;
    decode_image(reader, &path, SamplerKind::Texture2D, 0).map(Arc::new)
}

/// Decode a map in the shape its sampler asks for.
pub(in crate::app::model_preview) fn decode_image(
    reader: &TagReader,
    path: &str,
    kind: SamplerKind,
    index: usize,
) -> Option<Image> {
    let tag = reader.load(path, "bitmap", b"bitm").ok()?;
    if kind == SamplerKind::Texture2D {
        let image = decode_thumbnail(&tag, index, MAX_TEXTURE_EDGE).ok()?;
        return Some(Image::Flat(TextureImage {
            rgba: image.rgba.into(),
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
        return Some(Image::Cube {
            size: width as usize,
            faces,
        });
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
    Some(Image::Volume {
        width: width as usize,
        height: height as usize,
        depth,
        rgba,
    })
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
    image
        .pixel_bytes()
        .ok()?
        .get(start..start.checked_add(size as usize)?)
}
