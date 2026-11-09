//! Halo CE's shaders resolved to the textures and passes the preview draws.

use super::super::super::*;
use super::super::super::ce::shaders::{
    ce_camo_pass, ce_model_passes, ce_transparent_passes,
    read_ce_camo_settings,
};
use super::super::super::dx9::passes::{CeNumeric, 
    H2DrawPass, H2Layer,
};
use crate::core::h2_pixel_shader::SamplerKind;
use std::sync::Arc;
use std::collections::HashMap;
use super::super::super::materials::*;

/// Halo CE model shading, as `ShaderModel()` in the kit's `fx/model_common.h`
/// does it and `rasterizer_model_draw_{model,environment}_shader_pp` (CE
/// Anniversary X360) feed it.
///
/// - `shader_model` (1,633 of the 2,419 parts haloce_mcc's gbxmodels draw):
///   base, multipurpose and detail maps; the detail map tiles at
///   `(detail map scale, detail map scale · detail map v-scale)` over the base
///   coordinates, which `map u/v-scale` scales; its `detail function` and
///   `detail mask` combine it; alpha test on the base map's alpha unless the
///   shader is `not alpha-tested` or an `alpha-blended decal`.
/// - `shader_environment` on a model (148 parts) draws through the same
///   function with no mask: base map, primary detail map at its scale by the
///   `detail map function`, and — only when `alpha-tested` — the bump map bound
///   in the multipurpose slot, whose alpha is the test. Its bump is never
///   shaded on a model: bump is lightmap-only in CE.
///
/// Both clip at alpha 0x7F, as the preview's alpha slot already does. The
/// transparent families are effects drawn blended; the preview draws opaque,
/// so they stay untextured rather than turning into solid quads.
pub(in crate::app::model_preview) fn resolve_ce_shader(
    reader: &TagReader,
    shader: &TagFile,
    group: [u8; 4],
    permutation: usize,
    caches: &mut ResolveCaches,
) -> MaterialTextures {
    let root = shader.root();
    let flags = super::super::super::ce::shaders::ShaderFlags::of(root);
    let has = |flag: &str| flags.has(flag);
    // Both scale fields store 0 for "unset"; a 0 at runtime would collapse
    // every texel to one (the cyborg ships `detail map v-scale` 0), so it can
    // only mean 1.
    let factor = |value: Option<f32>| value.filter(|v| v.is_finite() && *v != 0.0).unwrap_or(1.0);
    let mut bind = |path: Option<String>, scale: [f32; 2]| {
        let path = path.filter(|path| !path.is_empty())?;
        let image_index = ce_image_index(reader, &path, permutation, &mut caches.ce_image_counts);
        let binding = SlotBitmap {
            path,
            image_index: image_index as i16,
            repeat_x: true,
            repeat_y: true,
            scale,
        };
        cached_bitmap(reader, &binding, caches)
    };
    let detail_function = |name: Option<String>| match name.as_deref() {
        Some("multiply") => DetailFunction::Multiply,
        Some("double/biased add") => DetailFunction::BiasedAdd,
        _ => DetailFunction::BiasedMultiply,
    };

    let mut textures = MaterialTextures::default();
    match &group {
        b"soso" => {
            let map = [factor(root.read_real("map u-scale")), factor(root.read_real("map v-scale"))];
            let detail = factor(root.read_real("detail map scale"));
            let detail_v = factor(root.read_real("detail map v-scale"));
            textures.slots[TextureSlot::Base as usize] = bind(root.read_tag_ref_path("base map"), map);
            textures.slots[TextureSlot::Multipurpose as usize] =
                bind(root.read_tag_ref_path("multipurpose map"), map);
            textures.slots[TextureSlot::Detail as usize] = bind(
                root.read_tag_ref_path("detail map"),
                [map[0] * detail, map[1] * detail * detail_v],
            );
            textures.detail = DetailComposition {
                function: detail_function(root.read_enum_option("detail function")),
                mask: CE_DETAIL_MASKS
                    .iter()
                    .position(|name| root.read_enum_option("detail mask").as_deref() == Some(*name))
                    .unwrap_or(0) as u8,
                xbox_channel_order: has("multipurpose map uses OG Xbox channel order"),
            };
            if !has("not alpha-tested") && !has("alpha-blended decal") {
                textures.slots[TextureSlot::AlphaTest as usize] =
                    textures.slots[TextureSlot::Base as usize].clone();
            }
        }
        b"senv" => {
            textures.slots[TextureSlot::Base as usize] = bind(root.read_tag_ref_path("base map"), [1.0; 2]);
            textures.slots[TextureSlot::Detail as usize] = bind(
                root.read_tag_ref_path("primary detail map"),
                [factor(root.read_real("primary detail map scale")); 2],
            );
            textures.detail = DetailComposition {
                function: detail_function(root.read_enum_option("detail map function")),
                ..Default::default()
            };
            if has("alpha-tested") {
                textures.slots[TextureSlot::AlphaTest as usize] =
                    bind(root.read_tag_ref_path("bump map"), [1.0; 2]);
            }
        }
        _ => {}
    }
    let mut common = caches.ce_model_common.take();
    let mut bitmap = |path: &str, kind: SamplerKind| {
        let image = ce_image_index(reader, path, permutation, &mut caches.ce_image_counts);
        let key = (path.to_owned(), kind, image);
        if let Some(cached) = caches.h2_images.get(&key) {
            return cached.clone();
        }
        let decoded = decode_image(reader, path, kind, image).map(Arc::new);
        caches.h2_images.insert(key, decoded.clone());
        decoded
    };
    textures.h2_layers[H2Layer::Lightmap as usize] =
        ce_model_passes(reader, root, &group, &flags, &mut common, &mut bitmap);
    let transparent = ce_transparent_passes(reader, root, &group, &mut bitmap);
    if matches!(&group, b"soso" | b"senv") {
        let settings = caches.ce_camo.get_or_insert_with(|| read_ce_camo_settings(reader));
        if let Some(settings) = settings {
            let two_sided = flags.has("two-sided");
            textures.h2_layers[H2Layer::ActiveCamo as usize] =
                vec![ce_camo_pass(settings, two_sided, &mut bitmap)];
        }
    }
    textures.h2_layers[H2Layer::Transparent as usize] =
        expand_ce_digits(reader, transparent, permutation, caches);
    caches.ce_model_common = common;
    if !textures.h2_layers[H2Layer::Transparent as usize].is_empty() {
        return textures;
    }
    if !matches!(&group, b"soso" | b"senv") {
        return MaterialTextures::failed(
            "a Halo CE transparent shader the preview does not draw yet",
        );
    }
    if textures.slots.iter().all(Option::is_none) {
        textures.error = Some("the shader names no base or detail map".to_owned());
    }
    textures
}

/// Split each numeric pass into one per digit its bitmap holds.
fn expand_ce_digits(
    reader: &TagReader,
    passes: Vec<H2DrawPass>,
    place: usize,
    caches: &mut ResolveCaches,
) -> Vec<H2DrawPass> {
    let mut expanded = Vec::with_capacity(passes.len());
    for pass in passes {
        let Some(numeric) = pass.numeric.clone() else {
            expanded.push(pass);
            continue;
        };
        // `ce_image_index` wraps at the count, so this is the count itself.
        let count = caches.ce_image_counts.get(&numeric.map).copied().unwrap_or_else(|| {
            reader.load(&numeric.map, "bitmap", b"bitm")
                .ok()
                .and_then(|tag| blam_tags::bitmap::Bitmap::new(&tag).ok().map(|bitmap| bitmap.len()))
                .unwrap_or(1)
        });
        for digit in 0..count {
            let key = (numeric.map.clone(), SamplerKind::Texture2D, digit);
            let image = caches
                .h2_images
                .entry(key)
                .or_insert_with(|| {
                    decode_image(reader, &numeric.map, SamplerKind::Texture2D, digit).map(Arc::new)
                })
                .clone();
            let mut pass = pass.clone();
            if let Some(first) = pass.textures.first_mut() {
                first.image = image;
            }
            pass.numeric = Some(CeNumeric { count, digit, place, ..numeric.clone() });
            expanded.push(pass);
        }
    }
    expanded
}

/// Halo CE `detail mask` options in order; the index is what the fragment
/// shader decodes (see [`DetailComposition::mask`]).
pub(in crate::app::model_preview) const CE_DETAIL_MASKS: [&str; 9] = [
    "none",
    "reflection mask inverse",
    "reflection mask",
    "self-illumination mask inverse",
    "self-illumination mask",
    "change-color mask inverse",
    "change-color mask",
    "multipurpose map alpha inverse",
    "multipurpose map alpha",
];

/// The image a Halo CE permutation picks of a bitmap: `rasterizer_set_texture`
/// wraps it at the bitmap's count.
fn ce_image_index(
    reader: &TagReader,
    path: &str,
    permutation: usize,
    counts: &mut HashMap<String, usize>,
) -> usize {
    if permutation == 0 {
        return 0;
    }
    let count = *counts.entry(path.to_owned()).or_insert_with(|| {
        reader.load(path, "bitmap", b"bitm")
            .ok()
            .and_then(|tag| blam_tags::bitmap::Bitmap::new(&tag).ok().map(|bitmap| bitmap.len()))
            .unwrap_or(1)
    });
    permutation % count.max(1)
}
