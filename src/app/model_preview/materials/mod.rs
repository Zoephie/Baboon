//! Resolving a render_model's materials to the textures a shaded preview samples.
//! This is the shared part: the dispatcher, the texture slots, the caches and
//! the bitmap binding and decode every game uses. Each game's own shader
//! resolution is its folder's `materials.rs`. GL upload and drawing belong to
//! the renderer, and geometry to the preview conversion beside it.

use super::*;
use blam_tags::render_method::{
    BitmapAddressMode, ParameterSource, RenderMethod,
    RenderMethodDefinition, RenderMethodOption, RenderMethodParameter, ResolvedRenderMethod,
    ResolvedValue,
};
use super::ce::shaders::{
    CeCamoSettings, CeModelCommon,
};
use super::gen3::shaders::{
    H3Material, H3Shield, H3Sources,
};
use super::dx9::stage::H2VertexStage;
use super::dx9::passes::{
    H2DrawPass, H2Layer, H2_LAYER_COUNT, h2_shadow_passes,
};
use super::common::image::H2Image;
use crate::core::h2_pixel_shader::SamplerKind;
use std::sync::Arc;
use std::collections::HashMap;

use super::ce::shaders::resolve::*;
use super::gen3::materials::{H3Bitmap, h3_xform, prepare_h3_material, reach_reference, reach_version_update};
use super::h2::shader::resolve::*;

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
/// are load-bearing: 0-3 reach the shader as one `vec4` of flags, 4-5 as the
/// next.
///
/// `Multipurpose` is Halo CE's alone: its model shaders mask the detail map by
/// one of that map's channels (`fx/model_common.h`). No render method or Halo 2
/// shader names it, so it is absent from [`SLOT_PARAMETERS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextureSlot {
    Base,
    Detail,
    Bump,
    BumpDetail,
    AlphaTest,
    Multipurpose,
}

pub(crate) const SLOT_COUNT: usize = 6;

/// Every slot, in binding order.
pub(crate) const ALL_SLOTS: [TextureSlot; SLOT_COUNT] = [
    TextureSlot::Base,
    TextureSlot::Detail,
    TextureSlot::Bump,
    TextureSlot::BumpDetail,
    TextureSlot::AlphaTest,
    TextureSlot::Multipurpose,
];

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
pub(crate) const SLOT_PARAMETERS: [(TextureSlot, &str); 5] = [
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
    /// UV multiplier for this slot, per axis: 329 of the 2,468 Halo 2
    /// parameters that author an x/y scale pair tile the two differently, and
    /// Halo CE's `map u/v scale` and `detail map v scale` are per axis too.
    pub scale: [f32; 2],
}

impl TextureImage {
    /// Whether its pixels fill its size, which is how much GL reads.
    pub(crate) fn whole(&self) -> bool {
        rgba_fills(&self.rgba, &[self.width, self.height])
    }
}

/// Whether `rgba` holds four bytes for every texel of `dimensions`.
pub(crate) fn rgba_fills(rgba: &[u8], dimensions: &[usize]) -> bool {
    let texels = dimensions.iter().try_fold(4usize, |bytes, side| bytes.checked_mul(*side));
    texels.is_some_and(|bytes| rgba.len() >= bytes)
}

/// How a material's detail map combines with its base map.
///
/// `LinearBiasedMultiply` is the preview's long-standing Halo 3 / Reach
/// combine and stays theirs. The other three are Halo CE's `detail function`s
/// (`fx/model_common.h`), computed as the engine did, on the stored (gamma)
/// values; Halo 2 uses the first of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DetailFunction {
    #[default]
    LinearBiasedMultiply,
    /// `base · detail · 2`; mid-grey leaves the base untouched.
    BiasedMultiply,
    /// `base · detail`.
    Multiply,
    /// `base + 2 · detail − 1`.
    BiasedAdd,
}

/// What masks a material's detail map, as Halo CE's `detail mask` numbers it:
/// 0 is none; then reflection, self-illumination, change-colour and the
/// multipurpose alpha, each as inverse (odd) then direct (even). Each reads one
/// multipurpose-map channel (`b`, `g`, `a`, `r`) after the optional Xbox
/// channel reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct DetailComposition {
    pub function: DetailFunction,
    pub mask: u8,
    /// `use xbox multipurpose channel order`: sample the map as `agrb`.
    pub xbox_channel_order: bool,
}

/// Every texture one material draws with.
#[derive(Debug, Clone, Default)]
pub(crate) struct MaterialTextures {
    pub slots: [Option<TextureImage>; SLOT_COUNT],
    pub detail: DetailComposition,
    /// Why this material has no textures, when it has none. Kept so the panel
    /// can say what went wrong instead of silently drawing it untextured.
    pub error: Option<String>,
    /// Set when the render-method definition could not be read and the slots
    /// came from the shader's own parameter block alone. Everything the shader
    /// authors is still found; what is missed are values that would have come
    /// from an option default — including the detail maps' tiling, which lives
    /// there and defaults to 16.
    pub used_shader_parameters_only: bool,
    /// Halo 2 passes by `H2Layer`, drawn in place of the slots above; empty
    /// for other games and outside a loose kit, whose `.psh` sources they need.
    pub h2_layers: [Vec<H2DrawPass>; H2_LAYER_COUNT],
    /// Halo 3's own shader, translated from the kit's source.
    pub h3: Option<Arc<H3Material>>,
    /// Reach's `only render for shields`: `render_model_postprocess` (the
    /// 360's) makes its parts a shield's, which no ordinary pass draws.
    pub shield_only: bool,
    /// Reach's `never render for shields`: an ordinary part the shield skips.
    pub never_shield: bool,
    /// Why it draws bare unless its passes do.
    pub bare_without_passes: Option<String>,
}

impl MaterialTextures {
    pub(crate) fn get(&self, slot: TextureSlot) -> Option<&TextureImage> {
        self.slots[slot as usize].as_ref()
    }

    pub(in crate::app::model_preview) fn failed(error: impl Into<String>) -> Self {
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
pub(in crate::app::model_preview) struct ResolveCaches {
    /// Halo 4's material shaders by path, read and parsed once.
    pub(in crate::app::model_preview) h4_shaders: HashMap<String, Result<super::h4::materials::H4Shader, Arc<str>>>,
    /// Halo 4's bitmaps by path: whether each exists, and is a cube.
    pub(in crate::app::model_preview) h4_bitmap_kinds: HashMap<String, Option<bool>>,
    /// `rasterizer_globals`' two hologram screen materials, read once.
    pub(in crate::app::model_preview) h4_hologram_post: Option<Option<Arc<crate::app::model_preview::h4::submit::H4HologramPost>>>,
    /// `globals`' `camo distortion texture`, read once.
    pub(in crate::app::model_preview) h4_camo_texture: Option<Option<String>>,
    pub(in crate::app::model_preview) h3_shield: Option<Option<Arc<H3Shield>>>,
    pub(in crate::app::model_preview) h3_sources: Option<H3Sources>,
    /// Images decoded for Halo 3 passes, with whether each is sRGB.
    pub(in crate::app::model_preview) h3_images: HashMap<(String, i16, usize), Option<H3Bitmap>>,
    pub(in crate::app::model_preview) definitions: HashMap<String, Option<Arc<RenderMethodDefinition>>>,
    pub(in crate::app::model_preview) options: HashMap<String, Option<Arc<RenderMethodOption>>>,
    /// Reach's reference shaders by path; Spartan armor shares a few.
    pub(in crate::app::model_preview) references: HashMap<String, Option<Arc<RenderMethod>>>,
    /// Decoded bitmaps by `(path, image index)`. Shared maps are common —
    /// masterchief's visor variants all reach for the same detail map.
    pub(in crate::app::model_preview) bitmaps: HashMap<(String, i16), Option<TextureImage>>,
    /// Halo 2 shader templates' parameter defaults, by template path. 1,637
    /// shaders share a few dozen templates; `tex_bump` alone serves 296.
    pub(in crate::app::model_preview) h2_templates: HashMap<String, Option<Arc<H2TemplateDefaults>>>,
    /// The kit's `global shader`, read at most once; see [`prepare_h2_camo_passes`].
    pub(in crate::app::model_preview) h2_camo_shader: Option<Option<Arc<TagFile>>>,
    /// Pass maps by path and shape; a shader samples each in only one shape.
    /// By path, shape and image.
    pub(in crate::app::model_preview) h2_images: HashMap<(String, SamplerKind, usize), Option<Arc<H2Image>>>,
    /// Halo CE's camouflage, read once.
    pub(in crate::app::model_preview) ce_camo: Option<Option<CeCamoSettings>>,
    /// How many images a Halo CE bitmap holds.
    pub(in crate::app::model_preview) ce_image_counts: HashMap<String, usize>,
    /// The kit's model shader as GLSL, read once. `None` until asked for.
    pub(in crate::app::model_preview) ce_model_common: Option<Option<CeModelCommon>>,
    /// The kit's own group → extension names. The cross-game table answers
    /// with the H3+ meaning on a collision, and Halo CE's
    /// `shader_transparent_meter` (`smet`) is Halo 4's `structure_meta` there.
    pub(in crate::app::model_preview) kit_names: Option<TagNameIndex>,
}

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
fn share_object_layers(materials: &[RenderModelPreviewMaterial], textures: &mut [MaterialTextures]) {
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
        return super::h4::materials::resolve_h4_material(reader, &shader, caches);
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
        sort_layer: shader.root().descend("render_method").map_or(2, super::particles::h3_sort_layer),
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
pub(super) fn decode_h2_image(reader: &TagReader, path: &str, kind: SamplerKind, index: usize) -> Option<H2Image> {
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
pub(super) fn array_layer_bytes<'a>(
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
fn slot_binding(values: &Values<'_>, parameter: &str) -> Option<SlotBitmap> {
    let [x, y, ..] = h3_xform(values, parameter);
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
fn address_mode_repeats(mode: BitmapAddressMode) -> bool {
    matches!(mode, BitmapAddressMode::Wrap | BitmapAddressMode::Mirror)
}

#[cfg(test)]
mod tests {
    //! Resolving a render_model's materials to textures, against real editing kits.
    //!
    //! Every link in this chain is a lookup that can fail quietly — a shader group
    //! that resolves to the wrong extension, a parameter name spelled differently
    //! per game, an rmop default that never gets consulted. A unit test with a
    //! synthetic tag would pass through all of it and prove nothing, because the
    //! thing being tested *is* whether shipped tags match the assumptions. So these
    //! run against installed kits and self-skip without them.

    use super::*;
    use crate::app::model_preview::harness::kit::{kit_root, kit_source};
    use crate::core::game::GameId;

    /// The model's hologram and damage passes go to its own materials, never
    /// to a held weapon's.
    #[test]
    fn a_models_overlay_passes_stay_on_its_own_object() {
        let material = |object, overlay, shield| {
            RenderModelPreviewMaterial { object, overlay, shield, ..Default::default() }
        };
        let materials = [
            material(0, false, None),
            material(0, true, None),
            material(0, false, Some(H2Layer::ShieldDamaged)),
            material(1, false, None),
        ];
        let mut textures = vec![MaterialTextures::default(); materials.len()];
        let [(_, pass), ..] = h2_shadow_passes();
        textures[1].h2_layers[H2Layer::Transparent as usize] = vec![pass.clone()];
        textures[2].h2_layers[H2Layer::Transparent as usize] = vec![pass];
        share_object_layers(&materials, &mut textures);
        let has = |index: usize, layer: H2Layer| !textures[index].h2_layers[layer as usize].is_empty();
        for layer in [H2Layer::Hologram, H2Layer::ShieldDamaged] {
            assert!(has(0, layer) && has(1, layer) && has(2, layer), "{layer:?} on the model");
            assert!(!has(3, layer), "{layer:?} on the weapon");
        }
    }

    /// The kit a case resolves against, through the shared harness helper.
    ///
    /// Halo 3 and Reach both, deliberately — they are the two games this targets,
    /// and a slot table that silently only worked for one is exactly the failure
    /// worth catching.
    fn case_kit_root(case: &Case) -> Option<PathBuf> {
        crate::app::model_preview::harness::kit::kit_root(case.kit)
    }

    fn loose_source(tags_root: PathBuf, game: &str) -> TagSource {
        TagSource::LooseFolder {
            root: tags_root,
            game: GameId::from_id(game),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        }
    }

    /// Load a render_model and turn it into the preview's material list, the same
    /// way the preview loader does.
    fn model_materials(source: &TagSource, rel: &str) -> Option<Vec<RenderModelPreviewMaterial>> {
        let tag = load_referenced_tag_from_source(source, rel, "render_model", b"mode").ok()?;
        let model = blam_tags::render_model::RenderModel::from_tag(&tag).ok()?;
        Some(
            model
                .materials
                .iter()
                .map(|material| RenderModelPreviewMaterial {
                    shader_path: material.render_method.clone(),
                    shader_group: material.render_method_group,
                    overlay: false, shield: None,
                    permutation: 0,
                    object: 0,
                })
                .collect(),
        )
    }

    struct Case {
        game: &'static str,
        kit: GameId,
        model: &'static str,
    }

    const CASES: [Case; 3] = [
        Case {
            game: "halo3_mcc",
            kit: GameId::Halo3,
            model: r"objects\characters\masterchief\masterchief",
        },
        Case {
            game: "halo3_mcc",
            kit: GameId::Halo3,
            model: r"objects\characters\dervish\dervish",
        },
        Case {
            game: "haloreach_mcc",
            kit: GameId::HaloReach,
            model: r"objects\characters\brute\brute",
        },
    ];

    /// The whole chain, on a shipped character: render_model → per-part material →
    /// shader tag → resolved parameters → decoded bitmaps.
    #[test]
    fn a_shipped_character_resolves_to_real_textures_in_both_games() {
        let mut ran = 0;
        for case in CASES {
            let Some(root) = case_kit_root(&case) else {
                continue;
            };
            let source = loose_source(root, case.game);
            let Some(materials) = model_materials(&source, case.model) else {
                eprintln!("skipping: could not read {}", case.model);
                continue;
            };
            assert!(
                !materials.is_empty(),
                "{} has no materials at all",
                case.model
            );

            let resolved = resolve_model_textures(&source, &materials);
            assert_eq!(resolved.len(), materials.len(), "one result per material");

            // Every material must resolve to *something*. A character whose parts
            // all came back bare would mean the chain broke, not that the artist
            // shipped it untextured.
            let with_base = resolved
                .iter()
                .filter(|textures| textures.get(TextureSlot::Base).is_some())
                .count();
            assert!(
                with_base > 0,
                "{}: no material resolved a base_map. errors: {:?}",
                case.model,
                resolved
                    .iter()
                    .filter_map(|t| t.error.clone())
                    .collect::<Vec<_>>()
            );

            // The decoded image has to be usable as a texture, not just non-empty.
            for textures in &resolved {
                for (slot, name) in SLOT_PARAMETERS {
                    let Some(image) = textures.get(slot) else {
                        continue;
                    };
                    assert!(
                        image.width > 0 && image.height > 0,
                        "{}: {name} decoded to an empty image",
                        case.model
                    );
                    assert_eq!(
                        image.rgba.len(),
                        image.width * image.height * 4,
                        "{}: {name} is not tightly packed RGBA8",
                        case.model
                    );
                    assert!(
                        image.width as u32 <= MAX_TEXTURE_EDGE
                            && image.height as u32 <= MAX_TEXTURE_EDGE,
                        "{}: {name} came back at {}x{}, above the {MAX_TEXTURE_EDGE} cap",
                        case.model,
                        image.width,
                        image.height
                    );
                }
            }

            let slots_found: Vec<&str> = SLOT_PARAMETERS
                .iter()
                .filter(|(slot, _)| resolved.iter().any(|t| t.get(*slot).is_some()))
                .map(|(_, name)| *name)
                .collect();
            eprintln!(
                "{}: {} materials, {with_base} with a base map, slots seen: {}",
                case.game,
                materials.len(),
                slots_found.join(", ")
            );
            ran += 1;
        }
        if ran == 0 {
            eprintln!("skipping: no editing kit was available for either game");
        }
    }

    /// Masterchief specifically, because his shader is the one whose slots were
    /// read off disk while designing the table — diffuse, detail and normal all
    /// present on one material is what the feature was asked for.
    #[test]
    fn masterchiefs_body_shader_carries_diffuse_detail_and_normal() {
        let Some(root) = case_kit_root(&CASES[0]) else {
            eprintln!("skipping: H3EK not present");
            return;
        };
        let source = loose_source(root, "halo3_mcc");
        let Some(materials) = model_materials(&source, CASES[0].model) else {
            eprintln!("skipping: could not read masterchief.render_model");
            return;
        };
        let resolved = resolve_model_textures(&source, &materials);

        // The body material is the one that has all three; the visor and light
        // shaders legitimately do not.
        let has_all_three = resolved.iter().any(|textures| {
            textures.get(TextureSlot::Base).is_some()
                && textures.get(TextureSlot::Detail).is_some()
                && textures.get(TextureSlot::Bump).is_some()
        });
        assert!(
            has_all_three,
            "no masterchief material carried base + detail + bump together; errors: {:?}",
            resolved
                .iter()
                .filter_map(|t| t.error.clone())
                .collect::<Vec<_>>()
        );
    }

    /// A material with no shader must come back explained rather than silently
    /// bare, so the panel can say why a part is untextured.
    #[test]
    fn a_material_with_no_shader_reports_why() {
        let source = loose_source(PathBuf::from("C:/nonexistent/tags"), "halo3_mcc");
        let resolved = resolve_model_textures(
            &source,
            &[RenderModelPreviewMaterial {
                shader_path: String::new(),
                shader_group: 0,
                overlay: false, shield: None,
                permutation: 0,
                object: 0,
            }],
        );
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].error.is_some());
        assert!(resolved[0].slots.iter().all(Option::is_none));
    }

    /// Address modes decide whether a detail map tiles. Halo authors them to repeat
    /// many times over a surface, so a clamp here would smear one edge texel.
    #[test]
    fn wrap_modes_map_to_repeating_and_clamping() {
        use blam_tags::render_method::BitmapAddressMode;
        assert!(address_mode_repeats(BitmapAddressMode::Wrap));
        assert!(address_mode_repeats(BitmapAddressMode::Mirror));
        assert!(!address_mode_repeats(BitmapAddressMode::Clamp));
        assert!(!address_mode_repeats(BitmapAddressMode::BlackBorder));
    }

    /// Dervish carries a `bump_detail_map` on top of his `bump_map`, which is what
    /// prompted adding that slot — and he is also the model the base-map alpha
    /// discard used to punch holes through, so he is worth keeping as a fixture.
    #[test]
    fn dervish_resolves_a_detail_normal_on_top_of_his_bump_map() {
        let Some(root) = case_kit_root(&CASES[1]) else {
            eprintln!("skipping: H3EK not present");
            return;
        };
        let source = loose_source(root, "halo3_mcc");
        let Some(materials) = model_materials(&source, CASES[1].model) else {
            eprintln!("skipping: could not read dervish.render_model");
            return;
        };
        let resolved = resolve_model_textures(&source, &materials);

        assert!(
            resolved
                .iter()
                .any(|textures| textures.get(TextureSlot::Bump).is_some()
                    && textures.get(TextureSlot::BumpDetail).is_some()),
            "no dervish material carried both a bump map and a detail normal; errors: {:?}",
            resolved
                .iter()
                .filter_map(|t| t.error.clone())
                .collect::<Vec<_>>()
        );
    }

    /// The detail maps tile by the shader's own scale animator, as tool's
    /// `render_method_update_real_parameter` bakes them: dervish's armor
    /// authors 16.
    #[test]
    fn detail_tiling_comes_from_the_shaders_animator() {
        let Some(root) = case_kit_root(&CASES[0]) else {
            eprintln!("skipping: H3EK not present");
            return;
        };
        let source = loose_source(root, "halo3_mcc");
        let resolved = resolve_model_textures(
            &source,
            &[RenderModelPreviewMaterial {
                shader_path: r"objects\characters\dervish\shaders\dervish_armor".to_owned(),
                shader_group: u32::from_be_bytes(*b"rmsh"),
                overlay: false, shield: None,
                permutation: 0,
                object: 0,
            }],
        );
        let material = &resolved[0];
        assert!(
            !material.used_shader_parameters_only,
            "the render-method definition should load"
        );

        for slot in [TextureSlot::Detail, TextureSlot::BumpDetail] {
            let image = material
                .get(slot)
                .expect("dervish carries both detail maps");
            assert!(
                image.scale.iter().all(|scale| (scale - 16.0).abs() < 0.01),
                "detail tiling should come from the shader's animator, got {:?}",
                image.scale
            );
        }
        // The base map is not a detail map and must not inherit its tiling.
        let base = material.get(TextureSlot::Base).expect("a base map");
        assert!(base.scale.iter().all(|scale| (scale - 1.0).abs() < 0.01), "base scale {:?}", base.scale);
    }


    /// One classic shader, resolved the way the preview resolves a material.
    fn resolve_classic(source: &TagSource, path: &str, group: &[u8; 4]) -> MaterialTextures {
        resolve_model_textures(
            source,
            &[RenderModelPreviewMaterial {
                shader_path: path.to_owned(),
                shader_group: u32::from_be_bytes(*group),
                overlay: false, shield: None,
                permutation: 0,
                object: 0,
            }],
        )
        .remove(0)
    }

    /// Halo CE model shading, against `BLAM_TEST_HCEEK` (the kit's `tags`).
    /// Each case pairs with one that sets the opposite flag, so a resolver that
    /// ignored the flag fails one of them.
    #[test]
    fn halo_ce_shaders_resolve_as_the_engine_composes_them() {
        let Some(tags) = kit_root(GameId::HaloCe) else { return };
        let source = loose_source(tags, "haloce_mcc");

        // The cyborg's armor: every map, its detail masked by the reflection
        // channel in Xbox order, tiled ten times, alpha tested on the base.
        let armor = resolve_classic(&source, r"characters\cyborg\shaders\armor", b"soso");
        assert_eq!(armor.error, None);
        for slot in [TextureSlot::Base, TextureSlot::Detail, TextureSlot::Multipurpose] {
            assert!(armor.get(slot).is_some(), "armor lacks {slot:?}");
        }
        assert_eq!(
            armor.detail,
            DetailComposition {
                function: DetailFunction::BiasedMultiply,
                mask: 2,
                xbox_channel_order: true,
            }
        );
        // `detail map v scale` ships 0, which can only mean 1.
        assert_eq!(armor.get(TextureSlot::Detail).unwrap().scale, [10.0, 10.0]);
        assert!(armor.get(TextureSlot::AlphaTest).is_some(), "armor is alpha tested");
        assert!(armor.get(TextureSlot::Bump).is_none());

        // `not alpha tested`: the same resolver must leave the alpha slot empty.
        let metal = resolve_classic(&source, r"scenery\c_metalwide\shaders\c_metal", b"soso");
        assert!(metal.get(TextureSlot::Base).is_some());
        assert!(metal.get(TextureSlot::AlphaTest).is_none(), "c_metal is not alpha tested");

        // An alpha-tested environment shader tests on its bump map's alpha, and
        // never shades the bump: on a model, CE bump is lightmap-only.
        let teleporter = resolve_classic(
            &source,
            r"scenery\teleporter_base\shaders\teleporter_base",
            b"senv",
        );
        assert!(teleporter.get(TextureSlot::Base).is_some());
        assert!(teleporter.get(TextureSlot::AlphaTest).is_some());
        assert!(teleporter.get(TextureSlot::Bump).is_none());
        assert_eq!(teleporter.detail.mask, 0);

        // A chicago shader draws through its pass, not through the slots.
        let shield = resolve_classic(
            &source,
            r"characters\cyborg\shaders\light shield",
            b"schi",
        );
        assert!(shield.error.is_none());
        assert!(shield.slots.iter().all(Option::is_none));
        assert_eq!(shield.h2_layers[H2Layer::Transparent as usize].len(), 1);
        // `smet` is Halo 4's `structure_meta` in the cross-game extension table;
        // the kit's own names make it `shader_transparent_meter`, so the shader
        // loads rather than being reported as a missing file.
        let meter = resolve_classic(&source, r"vehicles\warthog\shaders\meter engine", b"smet");
        assert!(meter.error.is_none(), "{:?}", meter.error);
        assert_eq!(meter.h2_layers[H2Layer::Transparent as usize].len(), 1);
    }

    /// With `BLAM_TEST_HCEEK`: a permutation draws its own image of a bitmap,
    /// in both drawing paths, and past the last one wraps.
    #[test]
    fn a_halo_ce_permutation_picks_the_image_and_wraps() {
        let Some(source) = kit_source(GameId::HaloCe) else { return };
        let path = r"characters\grunt\shaders\grunt torso";
        let resolve = |permutation| {
            let material = RenderModelPreviewMaterial {
                shader_path: path.to_owned(),
                shader_group: u32::from_be_bytes(*b"soso"),
                overlay: false, shield: None,
                permutation,
                object: 0,
            };
            resolve_model_textures(&source, &[material]).remove(0)
        };
        let shader = load_referenced_tag_from_source(&source, path, "shader_model", b"soso").unwrap();
        let base = shader.root().read_tag_ref_path("base map").unwrap();
        let tag = load_referenced_tag_from_source(&source, &base, "bitmap", b"bitm").unwrap();
        let count = blam_tags::bitmap::Bitmap::new(&tag).unwrap().len();
        assert!(count > 1, "{base} holds {count}");
        let slot = |textures: &MaterialTextures| {
            textures.get(TextureSlot::Base).map(|image| image.rgba.clone()).unwrap()
        };
        let pass = |textures: &MaterialTextures| match textures.h2_layers
            [H2Layer::Lightmap as usize][0]
            .textures[0]
            .image
            .as_deref()
        {
            Some(H2Image::Flat(image)) => image.rgba.clone(),
            _ => panic!("no base map"),
        };
        let (first, second, wrapped) = (resolve(0), resolve(1), resolve(count));
        assert_ne!(slot(&first), slot(&second));
        assert_ne!(pass(&first), pass(&second));
        assert_eq!(slot(&first), slot(&wrapped));
        assert_eq!(pass(&first), pass(&wrapped));
    }

    /// Halo 2, against `BLAM_TEST_H2EK` (the kit's `tags`).
    #[test]
    fn halo_2_shaders_resolve_parameters_over_template_defaults() {
        let Some(tags) = kit_root(GameId::Halo2) else { return };
        let source = loose_source(tags, "halo2_mcc");

        // The chief authors his maps, and his detail tiling as a `bitmap scale
        // x`/`y` pair of 9 with no uniform; reading only the uniform fell back to
        // the template's 16.
        let chief = resolve_classic(&source, r"objects\characters\masterchief\shaders\masterchief", b"shad");
        assert_eq!(chief.error, None);
        assert!(!chief.used_shader_parameters_only, "the template should load");
        for slot in [TextureSlot::Base, TextureSlot::Detail, TextureSlot::Bump] {
            assert!(chief.get(slot).is_some(), "masterchief lacks {slot:?}");
        }
        assert_eq!(chief.get(TextureSlot::Detail).unwrap().scale, [9.0, 9.0]);
        assert_eq!(chief.get(TextureSlot::Base).unwrap().scale, [1.0, 1.0]);
        assert_eq!(chief.detail.function, DetailFunction::BiasedMultiply);
        assert!(chief.get(TextureSlot::AlphaTest).is_none(), "tex_bump does not alpha test");

        // brute_shoulder_armor names no detail_map at all: its template supplies
        // both the bitmap (`default_detail`) and the tiling (16).
        let shoulder = resolve_classic(
            &source,
            r"objects\characters\brute\shaders\brute_shoulder_armor",
            b"shad",
        );
        assert_eq!(shoulder.get(TextureSlot::Detail).expect("template default detail").scale, [16.0, 16.0]);

        // A base-alpha alpha-test template with no alpha_test_map cuts out on the
        // base map.
        let shotgun = resolve_classic(&source, r"objects\weapons\rifle\shotgun\shaders\shotgun_primary", b"shad");
        assert!(shotgun.get(TextureSlot::AlphaTest).is_some(), "shotgun_primary alpha tests");
    }

    /// Passes draw a material with no slots only while Game Shaders are on.
    #[test]
    fn a_material_of_passes_alone_is_bare_only_without_game_shaders() {
        let materials = [RenderModelPreviewMaterial {
            shader_path: "shaders/emblem".to_owned(),
            ..Default::default()
        }];
        let textures = [MaterialTextures {
            bare_without_passes: Some("no maps".to_owned()),
            ..Default::default()
        }];
        assert_eq!(texture_resolve_note(&textures, &materials, true), None);
        let (summary, detail) = texture_resolve_note(&textures, &materials, false).unwrap();
        assert!(summary.starts_with("1 of 1 materials untextured"), "{summary}");
        assert!(detail.contains("shaders/emblem: no maps"), "{detail}");
    }

    /// A material that drew untextured says why, instead of just looking flat.
    #[test]
    fn untextured_materials_say_why() {
        let material = |path: &str| RenderModelPreviewMaterial {
            shader_path: path.to_owned(),
            ..Default::default()
        };
        let materials = [
            material("shaders/a"),
            material("shaders/b"),
            material("shaders/c"),
        ];
        let textures = [
            MaterialTextures {
                error: Some("shader tag not found".to_owned()),
                ..Default::default()
            },
            MaterialTextures {
                used_shader_parameters_only: true,
                ..Default::default()
            },
            MaterialTextures::default(),
        ];
        let (summary, detail) = texture_resolve_note(&textures, &materials, true).unwrap();
        assert_eq!(
            summary,
            "1 of 3 materials untextured; 1 read without their definition or template defaults \
             — hover for why"
        );
        assert!(detail.contains("shaders/a: shader tag not found"));
        assert!(detail.contains("shaders/b"));
        assert!(!detail.contains("shaders/c"));
        assert_eq!(texture_resolve_note(&textures[2..], &materials[2..], true), None);
    }

    /// With `BLAM_TEST_H2EK`: `+X`'s top row is `+Y`'s right column reversed,
    /// so a cube in the right order joins there. In D3D order this pair is
    /// `+X` and `-X`, which share no edge at all.
    #[test]
    fn h2_cube_faces_join_at_their_edges() {
        let Some(source) = kit_source(GameId::Halo2) else { return };
        let path = "objects\\bitmaps\\reflection_maps\\vehicle_glass";
        let Some(H2Image::Cube { size, faces }) = decode_h2_image(&crate::app::model_preview::load::TagReader::new(&source), path, SamplerKind::Cube, 0)
        else {
            panic!("{path} should decode as a cube");
        };
        let texel = |face: usize, x: usize, y: usize| -> [u8; 3] {
            let at = (y * size + x) * 4;
            [faces[face][at], faces[face][at + 1], faces[face][at + 2]]
        };
        let seam = |other: usize| -> f32 {
            let total: u32 = (0..size)
                .map(|x| {
                    let (a, b) = (texel(0, x, 0), texel(other, size - 1, size - 1 - x));
                    (0..3).map(|c| a[c].abs_diff(b[c]) as u32).sum::<u32>()
                })
                .sum();
            total as f32 / size as f32
        };
        assert!(seam(2) < 30.0, "+X and +Y should join: {}", seam(2));
        assert!(seam(1) > seam(2) * 2.0, "{} against {}", seam(1), seam(2));
        let mean = |face: usize| faces[face].iter().map(|byte| *byte as u64).sum::<u64>();
        assert!(mean(4) > mean(5), "the sky is on +Z");
    }

    /// With `BLAM_TEST_H2EK`: a brute names no camo shader, and still takes
    /// the global one, with its own shader's bump.
    #[test]
    fn every_h2_shader_takes_the_global_active_camo() {
        let Some(source) = kit_source(GameId::Halo2) else { return };
        let material = |path: &str| RenderModelPreviewMaterial {
            shader_path: path.to_owned(),
            shader_group: u32::from_be_bytes(*b"shad"),
            overlay: false, shield: None,
            permutation: 0,
            object: 0,
        };
        let camo = |path: &str| {
            let textures = resolve_model_textures(&source, &[material(path)]);
            textures[0].h2_layers[H2Layer::ActiveCamo as usize].clone()
        };
        let chief = camo("objects\\characters\\masterchief\\shaders\\masterchief");
        let brute = camo("objects\\characters\\brute\\shaders\\brute");
        assert_eq!(chief.len(), 1);
        assert_eq!(brute.len(), 1);
        let bump = |passes: &[H2DrawPass]| match passes[0].textures[0].image.as_deref() {
            Some(H2Image::Flat(image)) => image.rgba.clone(),
            other => panic!("the bump should be a flat map: {other:?}"),
        };
        assert_ne!(bump(&chief), bump(&brute), "each shader brings its own bump");
    }

    /// With `BLAM_TEST_H2EK`: a volume's slices all decode.
    #[test]
    fn h2_volumes_decode_every_slice() {
        let Some(source) = kit_source(GameId::Halo2) else { return };
        let Some(H2Image::Volume { width, height, depth, rgba }) =
            decode_h2_image(&crate::app::model_preview::load::TagReader::new(&source), "rasterizer\\plasma_shield_noise", SamplerKind::Volume, 0)
        else {
            panic!("the plasma noise should decode as a volume");
        };
        assert!(depth > 1);
        assert_eq!(rgba.len(), width * height * depth * 4);
        let slice = width * height * 4;
        assert_ne!(rgba[..slice], rgba[slice * (depth - 1)..], "slices differ");
    }

    /// With `BLAM_TEST_HREK`: an array's layers each keep their whole chain,
    /// so `globals\global_render_texture`'s second layer starts 21844 bytes
    /// in, not a level's 16384 in as a volume's would.
    #[test]
    fn kit_an_arrays_layers_are_whole_chains_apart() {
        let Some(source) = kit_source(GameId::HaloReach) else { return };
        let path = "globals\\global_render_texture";
        let Some(H2Image::Volume { width, height, depth, rgba }) = decode_h2_image(&crate::app::model_preview::load::TagReader::new(&source), path, SamplerKind::Volume, 0)
        else {
            panic!("the render texture should decode as layers");
        };
        assert_eq!((width, height, depth), (64, 64, 14));
        let tag = load_referenced_tag_from_source(&source, path, "bitmap", b"bitm").unwrap();
        let bitmap = blam_tags::bitmap::Bitmap::new(&tag).unwrap();
        let image = bitmap.image(0).unwrap();
        let pixels = image.pixel_bytes().unwrap();
        assert_eq!(pixels.len(), 14 * 21844);
        let decode = |bytes: &[u8]| {
            let format = image.format().unwrap();
            blam_tags::bitmap::decode::decode_to_rgba8(format, 64, 64, bytes, bitmap.p8_palette()).unwrap()
        };
        let second = &rgba[64 * 64 * 4..2 * 64 * 64 * 4];
        assert_eq!(second, decode(&pixels[21844..21844 + 16384]).as_slice());
        assert_ne!(second, decode(&pixels[16384..2 * 16384]).as_slice(), "a volume's layout reads the first layer's mips");
        assert_eq!(array_layer_bytes(&image, 1, 1).map(<[u8]>::len), Some(32 * 32 * 4));
        assert_eq!(array_layer_bytes(&image, 1, 1).map(<[u8]>::as_ptr), Some(pixels[21844 + 16384..].as_ptr()));
    }
}
