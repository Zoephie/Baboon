//! Binding a slot to its bitmap: parameters, values and the bitmap cache.

use super::super::{ResolveCaches, TagReader, TextureImage, decode_thumbnail};
use super::gen3_xform;
use super::slots::MAX_TEXTURE_EDGE;
use blam_tags::render_method::{
    BitmapAddressMode, ParameterSource, RenderMethod, RenderMethodParameter, ResolvedRenderMethod,
    ResolvedValue,
};

/// Where a named parameter's value comes from.
///
/// The resolver is preferred because a good deal of this is *option defaults*
/// rather than anything the shader authors. Reading the shader block alone
/// finds only what it overrides.
pub(in crate::app::model_preview) struct Values<'a> {
    pub(in crate::app::model_preview) resolved: Option<&'a ResolvedRenderMethod>,
    pub(in crate::app::model_preview) method: &'a RenderMethod,
    /// `render_method`'s `sort layer`, by option name.
    pub(in crate::app::model_preview) sort_layer: i32,
}

/// What one slot binds to.
pub(in crate::app::model_preview) struct SlotBitmap {
    pub(in crate::app::model_preview) path: String,
    pub(in crate::app::model_preview) image_index: i16,
    pub(in crate::app::model_preview) repeat_x: bool,
    pub(in crate::app::model_preview) repeat_y: bool,
    pub(in crate::app::model_preview) scale: [f32; 2],
}

/// Find a slot's bitmap: through the resolver when the definition loaded, and
/// off the shader's own parameters when it did not.
pub(super) fn slot_binding(values: &Values<'_>, parameter: &str) -> Option<SlotBitmap> {
    let [x, y, ..] = gen3_xform(values, parameter);
    let scale = [x, y];
    if let Some(resolved) = values.resolved {
        let found = resolved.find(parameter)?;
        let ParameterSource::Inline(ResolvedValue::Bitmap(binding)) = &found.source else {
            // An extern texture is a runtime render target — a scope view, a
            // refraction buffer. There is no tag behind it to load.
            return None;
        };
        if binding.bitmap_path == blam_tags::render_method::DEFAULT_BITMAP {
            return None;
        }
        return Some(SlotBitmap {
            path: binding.bitmap_path.clone(),
            image_index: binding.bitmap_index,
            repeat_x: address_mode_repeats(binding.address_mode_x),
            repeat_y: address_mode_repeats(binding.address_mode_y),
            scale,
        });
    }

    let authored = parameter_named(values.method, parameter)?;
    if authored.bitmap_path.is_empty() || authored.bitmap_extern_mode.is_some() {
        return None;
    }
    Some(SlotBitmap {
        path: authored.bitmap_path.clone(),
        // The shader block carries no image index; the walker gets that from the
        // option's default. Image 0 is what nearly every slot uses.
        image_index: 0,
        repeat_x: address_mode_repeats(authored.bitmap_address_mode_x),
        repeat_y: address_mode_repeats(authored.bitmap_address_mode_y),
        scale,
    })
}

pub(in crate::app::model_preview) fn parameter_named<'a>(
    render_method: &'a RenderMethod,
    name: &str,
) -> Option<&'a RenderMethodParameter> {
    render_method
        .parameters
        .iter()
        .find(|candidate| candidate.parameter_name == name)
}

pub(in crate::app::model_preview) fn cached_bitmap(
    reader: &TagReader,
    binding: &SlotBitmap,
    caches: &mut ResolveCaches,
) -> Option<TextureImage> {
    let key = (binding.path.clone(), binding.image_index);
    // Keyed without how it is bound: the decode is the expensive half, and
    // two materials may bind the same bitmap at different tilings and wraps.
    if let Some(cached) = caches.bitmaps.get(&key) {
        return cached.clone().map(|mut image| {
            image.scale = binding.scale;
            image.repeat_x = binding.repeat_x;
            image.repeat_y = binding.repeat_y;
            image
        });
    }
    let decoded = decode_bound_bitmap(reader, binding);
    caches.bitmaps.insert(key, decoded.clone());
    decoded
}

fn decode_bound_bitmap(reader: &TagReader, binding: &SlotBitmap) -> Option<TextureImage> {
    let tag = reader.load(&binding.path, "bitmap", b"bitm").ok()?;
    let image =
        decode_thumbnail(&tag, binding.image_index.max(0) as usize, MAX_TEXTURE_EDGE).ok()?;
    Some(TextureImage {
        rgba: image.rgba,
        width: image.width,
        height: image.height,
        repeat_x: binding.repeat_x,
        repeat_y: binding.repeat_y,
        scale: binding.scale,
    })
}

/// Whether an address mode tiles.
///
/// `Mirror` counts as tiling: the preview samples with a plain `REPEAT`, and a
/// mirrored map drawn tiled is far closer to right than the same map drawn
/// clamped, which would smear one edge texel across everything past UV 1.
pub(super) fn address_mode_repeats(mode: BitmapAddressMode) -> bool {
    matches!(mode, BitmapAddressMode::Wrap | BitmapAddressMode::Mirror)
}
