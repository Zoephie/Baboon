//! Resolving a render_model's materials to the textures a shaded preview samples.
//! It owns the shader→bitmap lookup and its decode; GL upload and drawing
//! belong to the renderer, and geometry to the preview conversion beside it.

use super::*;
use blam_tags::render_method::{
    BitmapAddressMode, ParameterSource, RenderMethod, RenderMethodAnimatedParameterType,
    RenderMethodDefinition, RenderMethodOption, RenderMethodParameter, ResolvedRenderMethod,
    ResolvedValue,
};
use std::collections::HashMap;

/// The texture roles the preview understands.
///
/// Deliberately the maps that describe the *surface* — its colour and its shape
/// — and not the ones describing how it answers light. Specular, self
/// illumination, fresnel and environment reflection were all tried: without a
/// scene to light against, each needed invented lighting and a strength control
/// to look like anything, which is a great deal of machinery and guesswork for
/// a preview. These four are read straight off the tag and need none of it.
///
/// The order is the order the fragment shader binds them in, and
/// [`MaterialTextures::slots`] is indexed by `as usize`, so these discriminants
/// are load-bearing: 0-3 reach the shader as one `vec4` of flags, 4 as the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextureSlot {
    Base,
    Detail,
    Bump,
    BumpDetail,
    AlphaTest,
}

pub(crate) const SLOT_COUNT: usize = 5;

/// Slot → the Bungie parameter name that carries it.
///
/// These names were checked against 50 shipped shaders across H3EK and HREK
/// rather than assumed: `base_map` appears in 25/25 of each, and every name
/// here is spelled identically in both games, so one table serves both. A
/// shader that does not declare a slot simply leaves it unbound — Reach's
/// `brute_captain_armor` has no `detail_map` at all, which is normal.
///
/// `alpha_test_map` earns its place on correctness rather than looks: 14 of 25
/// shipped Halo 3 shaders use it, and ignoring it draws solid quads where the
/// cutouts belong.
pub(crate) const SLOT_PARAMETERS: [(TextureSlot, &str); SLOT_COUNT] = [
    (TextureSlot::Base, "base_map"),
    (TextureSlot::Detail, "detail_map"),
    (TextureSlot::Bump, "bump_map"),
    (TextureSlot::BumpDetail, "bump_detail_map"),
    (TextureSlot::AlphaTest, "alpha_test_map"),
];

/// Longest edge a preview texture is decoded to.
///
/// Shipped base maps are commonly 2048², and a model carries one per slot per
/// material. The preview draws at a few hundred points, so the full mip costs
/// GPU memory and decode time for detail no one can see.
pub(crate) const MAX_TEXTURE_EDGE: u32 = 1024;

/// One decoded texture, ready to become a GL texture.
#[derive(Debug, Clone)]
pub(crate) struct TextureImage {
    pub rgba: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// Whether the sampler should repeat rather than clamp, per axis. Halo
    /// authors detail maps to tile many times over a surface, so getting this
    /// wrong smears one texel across the whole model.
    pub repeat_x: bool,
    pub repeat_y: bool,
    /// UV multiplier for this slot.
    pub scale: f32,
}

/// Every texture one material draws with.
#[derive(Debug, Clone, Default)]
pub(crate) struct MaterialTextures {
    pub slots: [Option<TextureImage>; SLOT_COUNT],
    /// Why this material has no textures, when it has none. Kept so the panel
    /// can say what went wrong instead of silently drawing it untextured.
    pub error: Option<String>,
    /// Set when the render-method definition could not be read and the slots
    /// came from the shader's own parameter block alone. Everything the shader
    /// authors is still found; what is missed are values that would have come
    /// from an option default — including the detail maps' tiling, which lives
    /// there and defaults to 16.
    pub used_shader_parameters_only: bool,
}

impl MaterialTextures {
    pub(crate) fn get(&self, slot: TextureSlot) -> Option<&TextureImage> {
        self.slots[slot as usize].as_ref()
    }

    fn failed(error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Default::default()
        }
    }
}

/// Loaders shared across one model's materials.
///
/// A render_model's materials overwhelmingly share a render-method definition
/// and its options — all seven of masterchief's resolve through
/// `shaders\shader.render_method_definition` — so caching across the batch
/// turns N of those reads into one.
#[derive(Default)]
struct ResolveCaches {
    definitions: HashMap<String, Option<RenderMethodDefinition>>,
    options: HashMap<String, Option<RenderMethodOption>>,
    /// Decoded bitmaps by `(path, image index)`. Shared maps are common —
    /// masterchief's visor variants all reach for the same detail map.
    bitmaps: HashMap<(String, i16), Option<TextureImage>>,
}

/// Resolve every material of one model to its textures.
///
/// Runs on a worker: it reads and decodes a shader tag plus several bitmaps per
/// material, which is far too much to do inside a frame.
pub(crate) fn resolve_model_textures(
    source: &TagSource,
    materials: &[RenderModelPreviewMaterial],
) -> Vec<MaterialTextures> {
    let mut caches = ResolveCaches::default();
    materials
        .iter()
        .map(|material| {
            // Guarded per material: blam-tags' enum resolver panics on names
            // a custom kit's recompiled shader tags can carry, and a panic
            // here kills the worker before it sends its message — leaving the
            // viewport on "Loading shaders…" forever.
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                resolve_one_material(source, material, &mut caches)
            }))
            .unwrap_or_else(|_| {
                MaterialTextures::failed(
                    "this shader crashed the reader — its tags likely use names \
                     this build of blam-tags does not know",
                )
            })
        })
        .collect()
}

fn resolve_one_material(
    source: &TagSource,
    material: &RenderModelPreviewMaterial,
    caches: &mut ResolveCaches,
) -> MaterialTextures {
    if material.shader_path.is_empty() {
        return MaterialTextures::failed("no shader assigned");
    }
    let Some(extension) = blam_tags::paths::group_tag_to_extension(material.shader_group) else {
        return MaterialTextures::failed(format!(
            "unknown shader group {:?}",
            material.shader_group.to_be_bytes()
        ));
    };
    let group = material.shader_group.to_be_bytes();
    let shader =
        match load_referenced_tag_from_source(source, &material.shader_path, extension, &group) {
            Ok(tag) => tag,
            Err(error) => return MaterialTextures::failed(error.to_string()),
        };
    let Ok(render_method) = RenderMethod::from_tag(&shader) else {
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
    let resolved = cached_render_method_definition(
        source,
        &render_method.definition_path,
        &mut caches.definitions,
    )
    .map(|definition| {
        ResolvedRenderMethod::resolve(&render_method, &definition, |option_path| {
            cached_render_method_option(source, option_path, &mut caches.options)
        })
    });

    let values = Values {
        resolved: resolved.as_ref(),
        method: &render_method,
    };
    let mut textures = MaterialTextures {
        used_shader_parameters_only: resolved.is_none(),
        ..Default::default()
    };
    for (slot, parameter) in SLOT_PARAMETERS {
        let Some(binding) = slot_binding(&values, parameter) else {
            continue;
        };
        textures.slots[slot as usize] = cached_bitmap(source, &binding, caches);
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
    textures
}

/// Where a named parameter's value comes from.
///
/// The resolver is preferred because a good deal of this is *option defaults*
/// rather than anything the shader authors — `detail_map_scale_uniform` is 16
/// by default and dervish's shader never mentions it. Reading the shader block
/// alone finds only what it overrides, which is how the detail maps ended up
/// tiling once across a whole model.
struct Values<'a> {
    resolved: Option<&'a ResolvedRenderMethod>,
    method: &'a RenderMethod,
}

impl Values<'_> {
    fn real(&self, name: &str) -> Option<f32> {
        if let Some(resolved) = self.resolved
            && let Some(found) = resolved.find(name)
            && let ParameterSource::Inline(ResolvedValue::Real(value)) = &found.source
        {
            return value.is_finite().then_some(*value);
        }
        real_parameter(self.method, name)
    }
}

/// What one slot binds to.
struct SlotBitmap {
    path: String,
    image_index: i16,
    repeat_x: bool,
    repeat_y: bool,
    scale: f32,
}

/// Find a slot's bitmap: through the resolver when the definition loaded, and
/// off the shader's own parameters when it did not.
fn slot_binding(values: &Values<'_>, parameter: &str) -> Option<SlotBitmap> {
    let scale = slot_scale(values, parameter);
    if let Some(resolved) = values.resolved {
        let found = resolved.find(parameter)?;
        let ParameterSource::Inline(ResolvedValue::Bitmap(binding)) = &found.source else {
            // An extern texture is a runtime render target — a scope view, a
            // refraction buffer. There is no tag behind it to load.
            return None;
        };
        if binding.bitmap_path.is_empty() || binding.extern_texture_mode.is_some() {
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

/// A slot's UV multiplier.
///
/// Two places carry this and the option's is the one that usually holds the
/// real number: `detail_map_scale_uniform` defaults to **16** while dervish's
/// shader never mentions it, so reading only the shader's own `scale uniform`
/// animator left every detail map tiling once across the whole model.
fn slot_scale(values: &Values<'_>, parameter: &str) -> f32 {
    let from_option = values.real(&format!("{parameter}_scale_uniform"));
    let from_animator = parameter_named(values.method, parameter)
        .and_then(|found| {
            found.animated_parameters.iter().find(|animated| {
                animated.parameter_type.map(|kind| kind.get())
                    == Some(RenderMethodAnimatedParameterType::ScaleUniform)
            })
        })
        .and_then(|animated| animated.function.as_ref())
        .map(|function| function.evaluate(0.0, 0.0));

    // The animator wins when the shader authored one — that is an explicit
    // override of the option's default.
    from_animator
        .filter(|scale| scale.is_finite() && *scale > 0.0 && (*scale - 1.0).abs() > f32::EPSILON)
        .or(from_option)
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or(1.0)
}

fn parameter_named<'a>(
    render_method: &'a RenderMethod,
    name: &str,
) -> Option<&'a RenderMethodParameter> {
    render_method
        .parameters
        .iter()
        .find(|candidate| candidate.parameter_name == name)
}

/// A real parameter's value — the plain field when it is authored, otherwise its
/// `value` animator, which is where the shipped tags actually put it.
fn real_parameter(render_method: &RenderMethod, name: &str) -> Option<f32> {
    let found = parameter_named(render_method, name)?;
    if found.real_parameter != 0.0 && found.real_parameter.is_finite() {
        return Some(found.real_parameter);
    }
    let animated = found.animated_parameters.iter().find(|animated| {
        animated.parameter_type.map(|kind| kind.get())
            == Some(RenderMethodAnimatedParameterType::Value)
    })?;
    let value = animated.function.as_ref()?.evaluate(0.0, 0.0);
    value.is_finite().then_some(value)
}

fn cached_bitmap(
    source: &TagSource,
    binding: &SlotBitmap,
    caches: &mut ResolveCaches,
) -> Option<TextureImage> {
    let key = (binding.path.clone(), binding.image_index);
    // Keyed without the scale: the decode is the expensive half, and two
    // materials may bind the same bitmap at different tilings.
    if let Some(cached) = caches.bitmaps.get(&key) {
        return cached.clone().map(|mut image| {
            image.scale = binding.scale;
            image
        });
    }
    let decoded = decode_bound_bitmap(source, binding);
    caches.bitmaps.insert(key, decoded.clone());
    decoded
}

fn decode_bound_bitmap(source: &TagSource, binding: &SlotBitmap) -> Option<TextureImage> {
    let tag = load_referenced_tag_from_source(source, &binding.path, "bitmap", b"bitm").ok()?;
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
fn address_mode_repeats(mode: BitmapAddressMode) -> bool {
    matches!(mode, BitmapAddressMode::Wrap | BitmapAddressMode::Mirror)
}

#[cfg(test)]
#[path = "../tests/model_materials.rs"]
mod tests;
