//! Resolving a render_model's materials to the textures a shaded preview samples.
//! It owns the shader→bitmap lookup and its decode; GL upload and drawing
//! belong to the renderer, and geometry to the preview conversion beside it.

use super::*;
use blam_tags::render_method::{
    BitmapAddressMode, BitmapFilterMode, ParameterSource, RenderMethod, RenderMethodAnimatedParameterType,
    RenderMethodBitmapExternMode,
    RenderMethodDefinition, RenderMethodOption, RenderMethodParameter, RenderMethodParameterType, ResolvedRenderMethod,
    ResolvedValue,
};
use super::ce::passes::{
    CeCamoSettings, CeModelCommon, CeNumeric, ce_camo_pass, ce_model_passes, ce_transparent_passes,
    read_ce_camo_settings,
};
use super::ce::attachments::elements;
use super::h3_shaders::{
    H3Animated, H3Animation, H3Blend, H3Clock, H3Material, H3Pass, H3Shield, H3ShieldSettings, H3Sources,
    H3Dialect, H3Sampler, H3Texels, H3Texture, H3Value,
};
use super::dx9::stage::H2VertexStage;
use super::dx9::passes::{
    H2DrawPass, H2Image, H2Layer, H2ParameterValue, H2_LAYER_COUNT, animation_function,
    h2_pass_needs_frame_texture, h2_shadow_passes, prepare_draw_pass,
    resolve_h2_layer_passes,
};
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::hlsl::parse::TextureKind;
use std::sync::Arc;
use std::collections::HashMap;

mod h4;
pub(super) use h4::{H4EffectMaterial, h4_screen_vertex, resolve_h4_effect_material};

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
    /// Halo 4's material shaders by path, read and parsed once.
    h4_shaders: HashMap<String, Result<h4::H4Shader, Arc<str>>>,
    /// Halo 4's bitmaps by path: whether each exists, and is a cube.
    h4_bitmap_kinds: HashMap<String, Option<bool>>,
    /// `rasterizer_globals`' two hologram screen materials, read once.
    h4_hologram_post: Option<Option<Arc<crate::app::model_preview::h4_submit::H4HologramPost>>>,
    /// `globals`' `camo distortion texture`, read once.
    h4_camo_texture: Option<Option<String>>,
    h3_shield: Option<Option<Arc<H3Shield>>>,
    h3_sources: Option<H3Sources>,
    /// Images decoded for Halo 3 passes, with whether each is sRGB.
    h3_images: HashMap<(String, i16, usize), Option<H3Bitmap>>,
    definitions: HashMap<String, Option<Arc<RenderMethodDefinition>>>,
    options: HashMap<String, Option<Arc<RenderMethodOption>>>,
    /// Reach's reference shaders by path; Spartan armor shares a few.
    references: HashMap<String, Option<Arc<RenderMethod>>>,
    /// Decoded bitmaps by `(path, image index)`. Shared maps are common —
    /// masterchief's visor variants all reach for the same detail map.
    bitmaps: HashMap<(String, i16), Option<TextureImage>>,
    /// Halo 2 shader templates' parameter defaults, by template path. 1,637
    /// shaders share a few dozen templates; `tex_bump` alone serves 296.
    h2_templates: HashMap<String, Option<Arc<H2TemplateDefaults>>>,
    /// The kit's `global shader`, read at most once; see [`prepare_h2_camo_passes`].
    h2_camo_shader: Option<Option<Arc<TagFile>>>,
    /// Pass maps by path and shape; a shader samples each in only one shape.
    /// By path, shape and image.
    h2_images: HashMap<(String, SamplerKind, usize), Option<Arc<H2Image>>>,
    /// Halo CE's camouflage, read once.
    ce_camo: Option<Option<CeCamoSettings>>,
    /// How many images a Halo CE bitmap holds.
    ce_image_counts: HashMap<String, usize>,
    /// The kit's model shader as GLSL, read once. `None` until asked for.
    ce_model_common: Option<Option<CeModelCommon>>,
    /// The kit's own group → extension names. The cross-game table answers
    /// with the H3+ meaning on a collision, and Halo CE's
    /// `shader_transparent_meter` (`smet`) is Halo 4's `structure_meta` there.
    kit_names: Option<TagNameIndex>,
}

/// Resolve every material of one model to its textures.
///
/// Runs on a worker: it reads and decodes a shader tag plus several bitmaps per
/// material, which is far too much to do inside a frame.
pub(crate) fn resolve_model_textures(
    source: &TagSource,
    materials: &[RenderModelPreviewMaterial],
) -> Vec<MaterialTextures> {
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
                resolve_one_material(source, material, &mut caches)
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
    source: &TagSource,
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
        match load_referenced_tag_from_source(source, &material.shader_path, &extension, &group) {
            Ok(tag) => tag,
            Err(error) => return MaterialTextures::failed(error.to_string()),
        };
    if group == *b"mat " {
        return h4::resolve_h4_material(source, &shader, caches);
    }
    match blam_tags::game::Game::of(&shader) {
        blam_tags::game::Game::Halo1 => {
            return resolve_ce_shader(source, &shader, group, material.permutation, caches);
        },
        blam_tags::game::Game::Halo2 if group == *b"shad" => {
            return resolve_h2_shader(source, &shader, caches);
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
    let definition = cached_render_method_definition(source, &render_method.definition_path, &mut caches.definitions);
    if let Some(definition) = &definition
        && source.game() == Some(GameId::HaloReach)
    {
        reach_version_update(&mut render_method, definition);
        let reference = reach_reference(source, &render_method, &material.shader_path, definition, &mut caches.references);
        render_method.apply_reference_shader(definition, reference.as_deref());
    }
    let resolved = definition.map(|definition| {
        ResolvedRenderMethod::resolve(&render_method, &definition, |option_path| {
            // The engine's resolver takes owned options; this runs once per
            // texture resolve on a worker, not per frame.
            cached_render_method_option(source, option_path, &mut caches.options)
                .map(|option| (*option).clone())
        })
    });

    let shield_only = render_method.flags.contains(blam_tags::render_method::GlobalRenderMethodFlags::OnlyRenderForShields);
    let never_shield = render_method.flags.contains(blam_tags::render_method::GlobalRenderMethodFlags::NeverRenderForShields);
    let values = Values {
        resolved: resolved.as_ref(),
        method: &render_method,
        sort_layer: shader.root().descend("render_method").map_or(2, super::h3_particles::h3_sort_layer),
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
    // `Game::of` calls every tag after Halo 2 `Halo3`, Reach's included.
    if blam_tags::game::Game::of(&shader) == blam_tags::game::Game::Halo3 {
        match prepare_h3_material(source, &values, caches) {
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

/// A bitmap for a pass, and whether `curve` has the card decode it.
fn h3_image(
    source: &TagSource,
    path: &str,
    index: i16,
    kind: SamplerKind,
    caches: &mut ResolveCaches,
) -> Option<H3Bitmap> {
    let key = (path.to_owned(), index, kind as usize);
    if let Some(found) = caches.h3_images.get(&key) {
        return found.clone();
    }
    let found = read_h3_bitmap(source, path, index, kind);
    caches.h3_images.insert(key, found.clone());
    found
}

/// One image of a bitmap, what its texels mean, and its stored mips.
pub(super) type H3Bitmap = (Arc<H2Image>, H3Texels, Arc<Vec<H2Image>>);

/// Levels past the top, as the tag stores them. None when `image` was not
/// decoded at the tag's own size, since these would not continue it.
fn h3_stored_mips(tag: &TagFile, index: usize, image: &H2Image) -> Option<Vec<H2Image>> {
    let bitmap = blam_tags::bitmap::Bitmap::new(tag).ok()?;
    let stored = bitmap.image(index)?;
    let (width, height) = (stored.width(), stored.height());
    let side = |size: u32, level: u32| (size >> level).max(1) as usize;
    let levels = 1..stored.mipmap_levels();
    match image {
        H2Image::Flat(top) if top.width == width as usize && top.height == height as usize => levels
            .map(|level| {
                let data = crate::app::editor::build_bitmap_preview(tag, index, level as usize).ok()?;
                let (width, height) = (side(width, level), side(height, level));
                (data.width as usize == width && data.height as usize == height).then_some(H2Image::Flat(
                    TextureImage {
                        rgba: data.rgba,
                        width,
                        height,
                        repeat_x: top.repeat_x,
                        repeat_y: top.repeat_y,
                        scale: top.scale,
                    },
                ))
            })
            .collect(),
        H2Image::Cube { size, .. } if *size == width as usize => {
            let format = stored.format().ok()?;
            levels
                .map(|level| {
                    let edge = side(width, level);
                    let face = |face: u32| {
                        let data = stored.cube_face_level_bytes(level, face).ok()?;
                        let palette = bitmap.p8_palette();
                        blam_tags::bitmap::decode::decode_to_rgba8(format, edge as u32, edge as u32, data, palette).ok()
                    };
                    let faces = [face(0)?, face(1)?, face(2)?, face(3)?, face(4)?, face(5)?];
                    Some(H2Image::Cube { size: edge, faces })
                })
                .collect()
        }
        _ => None,
    }
}

/// What an `a8` texel samples as: MCC's Halo 3 (`halo3.dll` 0x180801D30) and
/// Reach (`haloreach.dll` 0x180A32D70) upload it as `DXGI_FORMAT_A8_UNORM`,
/// which reads black with its alpha. Reach's `color_black` change-color map is
/// one, and two_change_color reads its red and green.
fn alpha_only(image: &mut H2Image) {
    let blacken = |rgba: &mut Vec<u8>| rgba.chunks_exact_mut(4).for_each(|texel| texel[..3].fill(0));
    match image {
        H2Image::Flat(flat) => blacken(&mut flat.rgba),
        H2Image::Cube { faces, .. } => faces.iter_mut().for_each(blacken),
        H2Image::Volume { rgba, .. } => blacken(rgba),
    }
}

/// What an R8_UNORM texel samples as: (r, 0, 0, 1).
fn red_only(image: &mut H2Image) {
    let keep_red = |rgba: &mut Vec<u8>| {
        rgba.chunks_exact_mut(4).for_each(|texel| {
            texel[1] = 0;
            texel[2] = 0;
            texel[3] = 255;
        })
    };
    match image {
        H2Image::Flat(flat) => keep_red(&mut flat.rgba),
        H2Image::Cube { faces, .. } => faces.iter_mut().for_each(keep_red),
        H2Image::Volume { rgba, .. } => keep_red(rgba),
    }
}

pub(super) fn read_h3_bitmap(source: &TagSource, path: &str, index: i16, kind: SamplerKind) -> Option<H3Bitmap> {
    let tag = load_referenced_tag_from_source(source, path, "bitmap", b"bitm").ok();
    // `render_method_submit`: an index past the last image takes the last.
    let images = tag.as_ref().map_or(1, |tag| elements(tag.root(), "bitmaps*").len());
    let index = (index.max(0) as usize).min(images.saturating_sub(1)) as i16;
    let image = decode_h2_image(source, path, kind, index as usize);
    image.map(|mut image| {
        // `c_rasterizer_texture_ref::allocate`: xRGB and sRGB curves.
        let curve = tag.as_ref().and_then(|tag| {
            elements(tag.root(), "bitmaps*").get(index.max(0) as usize)?.read_int_any("curve")
        });
        let format = tag.as_ref().and_then(|tag| {
            blam_tags::bitmap::Bitmap::new(tag).ok()?.image(index.max(0) as usize)?.format().ok()
        });
        let halo4 = source.game() == Some(crate::core::game::GameId::Halo4);
        let texels = format.map_or(H3Texels::Linear, |format| {
            if halo4 {
                return H3Texels::of_halo4(format, curve.unwrap_or(0));
            }
            let srgb = if matches!(curve, None | Some(0)) {
                H3Texels::default_srgb(format)
            } else {
                matches!(curve, Some(1 | 5))
            };
            H3Texels::of(format, srgb)
        });
        let mut mips = tag.as_ref().and_then(|tag| h3_stored_mips(tag, index.max(0) as usize, &image)).unwrap_or_default();
        if format == Some(blam_tags::bitmap::BitmapFormat::A8) {
            std::iter::once(&mut image).chain(&mut mips).for_each(alpha_only);
        }
        // Halo 4 uploads `y8` and `ay8` as R8_UNORM (`halo4_tag_test`
        // 0x141B7E080), and DirectX 11 views have no swizzle.
        if halo4 && matches!(format, Some(blam_tags::bitmap::BitmapFormat::Y8 | blam_tags::bitmap::BitmapFormat::Ay8)) {
            std::iter::once(&mut image).chain(&mut mips).for_each(red_only);
        }
        (Arc::new(image), texels, Arc::new(mips))
    })
}

/// `c_render_method_shader::version_update`, which Reach runs on every
/// `shader` it loads: older tags' options and parameters as the game draws
/// them. Old Spartan helmets keep a `misc` option that would otherwise turn
/// every map on them by a radian.
fn reach_version_update(method: &mut RenderMethod, definition: &RenderMethodDefinition) {
    if method.class == blam_tags::render_method::RenderMethodClass::Shader {
        method.version_update(definition);
    }
}

/// The template a Reach shader names as its `reference`, which Reach's tool
/// (`postprocess_as_child`, 0x141166270) lays under it before the bake. A
/// shader naming itself has none, as tool refuses it.
fn reach_reference(
    source: &TagSource,
    method: &RenderMethod,
    own_path: &str,
    definition: &RenderMethodDefinition,
    cache: &mut HashMap<String, Option<Arc<RenderMethod>>>,
) -> Option<Arc<RenderMethod>> {
    let (group, path) = method.reference.as_ref()?;
    if path.eq_ignore_ascii_case(own_path) {
        return None;
    }
    cache
        .entry(path.to_ascii_lowercase())
        .or_insert_with(|| {
            let extension = blam_tags::paths::group_tag_to_extension(*group)?;
            let tag = load_referenced_tag_from_source(source, path, extension, &group.to_be_bytes()).ok()?;
            let mut reference = RenderMethod::from_tag(&tag).ok()?;
            reach_version_update(&mut reference, definition);
            Some(Arc::new(reference))
        })
        .clone()
}

/// A render method held inside another tag, resolved as the bake would.
pub(super) fn resolve_embedded_method(source: &TagSource, shader: TagStruct<'_>) -> Option<ResolvedRenderMethod> {
    let mut method = RenderMethod::from_struct(&shader).ok()?;
    let definition = cached_render_method_definition(source, &method.definition_path, &mut HashMap::new())?;
    if source.game() == Some(GameId::HaloReach) {
        let reference = reach_reference(source, &method, "", &definition, &mut HashMap::new());
        method.apply_reference_shader(&definition, reference.as_deref());
    }
    let mut options = HashMap::new();
    let resolved = ResolvedRenderMethod::resolve(&method, &definition, |option_path| {
        cached_render_method_option(source, option_path, &mut options).map(|option| (*option).clone())
    });
    Some(resolved)
}

/// A map of a resolved method, with the sampler the bake gave it: the option's
/// default wherever the shader's `bitmap flags` claim no field of it.
pub(super) fn read_resolved_map(
    source: &TagSource,
    resolved: &ResolvedRenderMethod,
    name: &str,
) -> Option<(H3Bitmap, H3Sampler)> {
    let ParameterSource::Inline(ResolvedValue::Bitmap(binding)) = &resolved.find(name)?.source else {
        return None;
    };
    let bitmap = read_h3_bitmap(source, &binding.bitmap_path, binding.bitmap_index, SamplerKind::Texture2D)?;
    let sampler = H3Sampler {
        address: [binding.address_mode_x, binding.address_mode_y],
        filter: binding.filter_mode,
    };
    Some((bitmap, sampler))
}

/// A map's `_xform`: its scale, then its offset. Tool's
/// `render_method_update_real_parameter` starts from identity and takes only
/// the shader's own animators; the option's default bitmap scale is not used.
fn h3_xform(values: &Values<'_>, parameter: &str) -> [f32; 4] {
    // `update_real_parameter`: identity, whatever `default_bitmap_scale` says.
    let mut xform = [1.0, 1.0, 0.0, 0.0];
    let Some(found) = parameter_named(values.method, parameter) else {
        return xform;
    };
    for animated in &found.animated_parameters {
        let (Some(kind), Some(function)) = (animated.parameter_type, animated.function.as_ref()) else {
            continue;
        };
        let value = function.evaluate(0.0, 0.0);
        match kind.get() {
            RenderMethodAnimatedParameterType::ScaleUniform => xform[..2].fill(value),
            RenderMethodAnimatedParameterType::ScaleX => xform[0] = value,
            RenderMethodAnimatedParameterType::ScaleY => xform[1] = value,
            RenderMethodAnimatedParameterType::TranslationX => xform[2] = value,
            RenderMethodAnimatedParameterType::TranslationY => xform[3] = value,
            _ => {}
        }
    }
    xform
}

/// The value a `UserParameter_` uniform holds in this shader.
fn h3_parameter_value(values: &Values<'_>, name: &str) -> Option<H3Value> {
    let resolved = values.resolved?.find(name)?;
    let ParameterSource::Inline(value) = &resolved.source else {
        return None;
    };
    Some(match value {
        ResolvedValue::Real(value) => H3Value::Floats([*value; 4]),
        // A constant slot as `update_constants` fills it, whatever the
        // engine's doc comment says: red in `i`, alpha in `l`.
        ResolvedValue::Color(slot) => H3Value::Floats(*slot),
        ResolvedValue::Int(value) => H3Value::Integer(*value),
        ResolvedValue::Bool(value) => H3Value::Integer(i32::from(*value)),
        ResolvedValue::Bitmap(_) => return None,
    })
}

/// The bitmap the engine binds for an extern texture, with no level around
/// the object: `rasterizer_globals`' material textures, and its default cube.
fn h3_extern_bitmap(name: &str) -> Option<&'static str> {
    match name {
        "cook torrance cc0236" => Some("rasterizer\\cc0236"),
        "cook torrance dd0236" => Some("rasterizer\\dd0236"),
        "cook torrance c78d78" => Some("rasterizer\\c78d78"),
        "dynamic environment map 1" | "dynamic environment map 2" => {
            Some("shaders\\default_bitmaps\\bitmaps\\default_dynamic_cube_map")
        }
        "active camo distortion texture" => Some("rasterizer\\active_camouflage_distortion"),
        _ => None,
    }
}

/// `render_method_submit`'s `get_extern_constant`, for an extern with no
/// frame in it. 
fn h3_extern_change_color(name: &str) -> Option<usize> {
    ["change color primary", "change color secondary", "change color tertiary", "change color quaternary"]
        .iter()
        .position(|known| *known == name)
}

/// The first of these a family has is what lights its objects.
const H3_LIT_ENTRIES: [&str; 3] = ["static_sh", "static_per_vertex", "default"];

fn prepare_h3_material(
    source: &TagSource,
    values: &Values<'_>,
    caches: &mut ResolveCaches,
) -> Result<H3Material, String> {
    let method = values.method;
    let sources = caches.h3_sources.get_or_insert_with(|| H3Sources::new(source));
    let definition = sources
        .definition(&method.definition_path)
        .ok_or_else(|| format!("{} did not read", method.definition_path))?;
    let options: Vec<usize> = method.options.iter().map(|option| (*option).max(0) as usize).collect();
    let glass = method.group_tag == u32::from_be_bytes(*b"rmgl");
    let blend = definition.blend_of(&options, glass);
    // ODST's `black` has only `albedo`, which is all it needs to be black.
    let lit_entry = H3_LIT_ENTRIES
        .iter()
        .chain(&["albedo"])
        .find(|entry| definition.entry_points.iter().any(|known| known == *entry))
        .ok_or("no lit entry point")?;
    let opaque = blend == H3Blend::Opaque;
    let dialect = sources.dialect();
    let has = |entry: &str| definition.entry_points.iter().any(|known| known == entry);
    // Reach's `select_mesh_entry_point_for_pass` (MCC `sub_14084C910`) lights
    // what blends in a single pass, by the mesh's policy: one probe, or the
    // ambient transfer.
    let single_pass_entries = (dialect == H3Dialect::Reach && !opaque && has("single_pass_single_probe"))
        .then_some(("single_pass_single_probe", "single_pass_single_probe_ambient"));
    let lit_entry = single_pass_entries.as_ref().map_or(lit_entry, |(entry, _)| entry);
    // `render_method_update_postprocess`: the single pass flag counts as blended.
    let single_pass = method.postprocess_definition.as_ref().is_some_and(|baked| baked.flags & 1 != 0);
    let own_albedo = !opaque || single_pass;
    // `render_albedo_decals`: into the albedo target, to be lit with what's under.
    let decal = definition.is_decal();
    let lit_entry = if decal { &"default" } else { lit_entry };
    let lit = if definition.name == "screen" {
        sources.screen_pass(&definition, &options)?
    } else {
        sources.pass(&definition, &options, lit_entry, own_albedo)?
    };
    let albedo = (opaque && definition.entry_points.iter().any(|entry| entry == "albedo"))
        .then(|| sources.pass(&definition, &options, "albedo", false))
        .transpose()?;
    let camo = (definition.entry_points.iter().any(|entry| entry == "active_camo"))
        .then(|| sources.pass(&definition, &options, "active_camo", false))
        .transpose()?;
    // Reach's writes back what it samples, which the preview's copy holds
    // encoded.
    let camo = match dialect {
        H3Dialect::Reach => camo.and_then(|pass| super::reach_camo::camo_pass(&pass)).map(Arc::new),
        H3Dialect::Halo3 => camo,
        H3Dialect::Halo4 => unreachable!("Halo 4 draws materials, not render methods"),
    };
    let shadow_generate = (dialect == H3Dialect::Reach && opaque && has("shadow_generate"))
        .then(|| sources.pass(&definition, &options, "shadow_generate", false))
        .transpose()?;
    let dynamic_light = (opaque && definition.entry_points.iter().any(|entry| entry == "dynamic_light"))
        .then(|| sources.pass(&definition, &options, "dynamic_light", false))
        .transpose()?;
    // `render_mesh_part_default`: a mesh's `PRT vertex type` picks one.
    let mut prt = [None, None, None];
    for (slot, entry) in prt.iter_mut().zip(["static_prt_ambient", "static_prt_linear", "static_prt_quadratic"]) {
        if opaque && has(entry) {
            *slot = Some(sources.pass(&definition, &options, entry, own_albedo)?);
        }
    }
    if let Some((_, ambient)) = single_pass_entries.filter(|(_, ambient)| has(ambient)) {
        prt[0] = Some(sources.pass(&definition, &options, ambient, own_albedo)?);
    }
    let screen_maps = method
        .parameters
        .iter()
        .filter(|parameter| parameter.bitmap_extern_mode == Some(RenderMethodBitmapExternMode::Screen))
        .map(|parameter| parameter.parameter_name.clone())
        .collect();
    let mut material = H3Material {
        dialect,
        albedo,
        lit,
        screen_maps,
        decal,
        decal_modulates_specular: decal && definition.decal_modulates_specular(&options),
        after_lighting: definition.after_lighting(&options),
        sort_layer: values.sort_layer,
        prt,
        camo,
        shadow_generate,
        dynamic_light,
        blend,
        values: Default::default(),
        textures: Vec::new(),
        change_colors: Vec::new(),
        animations: Vec::new(),
        shield: None,
        h4: None,
        fringe: None,
    };
    if opaque {
        material.shield = h3_shield(source, caches);
    }
    let passes: Vec<Arc<H3Pass>> = material
        .albedo
        .iter()
        .chain(&material.camo)
        .chain(&material.shadow_generate)
        .chain(&material.dynamic_light)
        .chain(material.prt.iter().flatten())
        .cloned()
        .chain([Arc::clone(&material.lit)])
        .collect();
    let clock = match (source.game(), definition.name == "screen") {
        (Some(GameId::Halo3Odst), true) => H3Clock::OdstScreen,
        (Some(GameId::Halo3Odst), false) => H3Clock::Odst,
        (Some(GameId::HaloReach), _) => H3Clock::Reach,
        _ => H3Clock::Halo3,
    };
    for parameter in &method.parameters {
        let uniform = match parameter.parameter_type.map(|kind| kind.get()) {
            Some(RenderMethodParameterType::Bitmap) => {
                format!("UserParameter_{}_xform", parameter.parameter_name)
            }
            _ => format!("UserParameter_{}", parameter.parameter_name),
        };
        for animated in &parameter.animated_parameters {
            let (Some(kind), Some(function)) = (animated.parameter_type, animated.function.as_ref()) else {
                continue;
            };
            let writes = match kind.get() {
                RenderMethodAnimatedParameterType::Color => H3Animated::Color,
                RenderMethodAnimatedParameterType::ScaleUniform => H3Animated::ScaleUniform,
                RenderMethodAnimatedParameterType::Value | RenderMethodAnimatedParameterType::ScaleX => {
                    H3Animated::First
                }
                RenderMethodAnimatedParameterType::ScaleY => H3Animated::Second,
                RenderMethodAnimatedParameterType::TranslationX => H3Animated::Third,
                RenderMethodAnimatedParameterType::TranslationY
                | RenderMethodAnimatedParameterType::Alpha => H3Animated::Fourth,
                RenderMethodAnimatedParameterType::FrameIndex => H3Animated::Frame,
            };
            if function.as_constant().is_some() && animated.input_name.is_empty() {
                continue;
            }
            let uniform = match writes {
                H3Animated::Frame => format!("UserParameterTexture_{}", parameter.parameter_name),
                _ => uniform.clone(),
            };
            material.animations.push(H3Animation {
                uniform,
                writes,
                function: function.clone(),
                input: animated.input_name.clone(),
                range: animated.range_name.clone(),
                // `sub_140C4F690`: a period of about 0 is 1.
                period: if animated.time_period_in_seconds.abs() < 0.0001 {
                    1.0
                } else {
                    animated.time_period_in_seconds
                },
                clock,
            });
        }
    }
    for pass in &passes {
        for (uniform, _) in &pass.uniforms {
            let Some(name) = uniform.strip_prefix("UserParameter_") else {
                continue;
            };
            if let Some(ParameterSource::Extern(source)) =
                values.resolved.and_then(|resolved| resolved.find(name)).map(|found| &found.source)
            {
                let source: &'static str = source.into();
                if let Some(channel) = h3_extern_change_color(source) {
                    material.change_colors.push((uniform.clone(), channel));
                } else if source == "debug tint" {
                    material.values.insert(uniform.clone(), H3Value::Floats([1.0, 1.0, 1.0, 0.0]));
                }
                continue;
            }
            let value = match name.strip_suffix("_xform") {
                Some(map) => Some(H3Value::Floats(h3_xform(values, map))),
                None => h3_parameter_value(values, name),
            };
            if let Some(value) = value {
                material.values.insert(uniform.clone(), value);
            }
        }
        for (uniform, kind) in &pass.textures {
            let Some(name) = uniform.strip_prefix("UserParameterTexture_") else {
                continue;
            };
            if material.textures.iter().any(|known| known.uniform == *uniform) {
                continue;
            }
            let found = values.resolved.and_then(|resolved| resolved.find(name));
            let (path, index, sampler) = match found.map(|found| &found.source) {
                // With no render target live behind an extern mode, as here,
                // the bitmap binds: `render_method_submit_volatile_per_shader`.
                Some(ParameterSource::Inline(ResolvedValue::Bitmap(binding))) => (
                    binding.bitmap_path.clone(),
                    binding.bitmap_index,
                    Some(H3Sampler {
                        address: [binding.address_mode_x, binding.address_mode_y],
                        filter: binding.filter_mode,
                    }),
                ),
                Some(ParameterSource::Extern(extern_source)) => {
                    match h3_extern_bitmap(extern_source.into()) {
                        Some(path) => (path.to_owned(), 0, None),
                        None => continue,
                    }
                }
                _ => continue,
            };
            let kind = match kind {
                TextureKind::Cube => SamplerKind::Cube,
                TextureKind::Volume => SamplerKind::Volume,
                _ => SamplerKind::Texture2D,
            };
            let Some((image, texels, mips)) = h3_image(source, &path, index, kind, caches) else {
                continue;
            };
            let animated = material.animations.iter().any(|animation| {
                animation.writes == H3Animated::Frame && animation.uniform == *uniform
            });
            let frames = if animated {
                let tag = load_referenced_tag_from_source(source, &path, "bitmap", b"bitm").ok();
                let count = tag.as_ref().map_or(0, |tag| elements(tag.root(), "bitmaps*").len());
                (0..count)
                    .filter_map(|at| h3_image(source, &path, at as i16, kind, caches))
                    .map(|(image, _, _)| image)
                    .collect()
            } else {
                Vec::new()
            };
            material.textures.push(H3Texture { uniform: uniform.clone(), image, texels, mips, sampler, frames });
        }
        if dialect == H3Dialect::Reach {
            reach_environment_maps(source, pass, &mut material, caches);
        }
    }
    Ok(material)
}

/// Reach's cube maps are the rasterizer's own, not a shader's externs. With no
/// level, `render_method_submit_dynamic_cubemap` (MCC `sub_140817F70`) binds
/// `rasterizer_globals`' default bitmap 2 to both, clamped, trilinear.
fn reach_environment_maps(source: &TagSource, pass: &H3Pass, material: &mut H3Material, caches: &mut ResolveCaches) {
    const DEFAULT_CUBE: &str = "shaders\\default_bitmaps\\bitmaps\\default_dynamic_cube_map";
    for (uniform, _) in &pass.textures {
        let wanted = matches!(
            uniform.as_str(),
            "GlobalTexture_dynamic_environment_map_0" | "GlobalTexture_dynamic_environment_map_1"
        );
        if !wanted || material.textures.iter().any(|known| known.uniform == *uniform) {
            continue;
        }
        let Some((image, texels, mips)) = h3_image(source, DEFAULT_CUBE, 0, SamplerKind::Cube, caches) else {
            continue;
        };
        let sampler = H3Sampler { address: [BitmapAddressMode::Clamp; 2], filter: BitmapFilterMode::Trilinear };
        material.textures.push(H3Texture {
            uniform: uniform.clone(),
            image,
            texels,
            mips,
            sampler: Some(sampler),
            frames: Vec::new(),
        });
    }
}

/// A `shield_impact` tag's constants and noise, as `render_object_context`
/// submits them.
pub(super) fn read_h3_shield_settings(source: &TagSource, path: &str) -> Option<Arc<H3ShieldSettings>> {
    let tag = load_referenced_tag_from_source(source, path, "shield_impact", b"shit").ok()?;
    let root = tag.root();
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let tinted = |color: &str, intensity: &str| {
        let (color, intensity) = (root.read_rgb(color), real(intensity));
        H3Value::Floats([color.red * intensity, color.green * intensity, color.blue * intensity, 0.0])
    };
    let mut values = super::h3_shaders::H3Values::new();
    let mut set = |name: &str, value: H3Value| {
        values.insert(name.to_owned(), value);
    };
    set("extrusion_distance", H3Value::Floats([real("Extrusion Distance"); 4]));
    let flags = root.read_flag_names("flags").unwrap_or_default();
    let flagged = |wanted: &str| flags.iter().any(|(_, name)| name.starts_with(wanted));
    // `render_object_shield_impact`: z is ODST's, for the second noise.
    let secondary = real(if flagged("use secondary") { "Secondary texture scale" } else { "Texture Scale" });
    set("texture_quantities", H3Value::Floats([real("Texture Scale"), real("Scroll Speed"), secondary, 0.0]));
    set("shield_impact_edge_fade", H3Value::Floats([real("Edge Fade"); 4]));
    for (uniform, sharpness, scale, threshold) in [
        ("plasma1_settings", "Plasma Sharpness 1", "Plasma Scale 1", "Plasma Threshold 1"),
        ("plasma2_settings", "Plasma Sharpness 2", "Plasma Scale 2", "Plasma Threshold 2"),
    ] {
        set(uniform, H3Value::Floats([real(sharpness), real(scale), real(threshold), 0.0]));
    }
    set("overshield_color1", tinted("Overshield Color 1", "Overshield Intensity 1"));
    set("overshield_color2", tinted("Overshield Color 2", "Overshield Intensity 2"));
    set("overshield_ambient_color", tinted("Overshield Ambient Color", "Overshield Ambient Intensity"));
    set("shield_impact_color1", tinted("Impact Color 1", "Impact Intensity 1"));
    set("shield_impact_color2", tinted("Impact Color 2", "Impact Intensity 2"));
    set("shield_impact_ambient_color", tinted("Impact Ambient Color", "Impact Ambient Intensity 2"));
    let mut textures = Vec::new();
    for (field, uniform) in [
        ("Shield Impact Noise Texture 1", "shield_impact_noise_texture1"),
        ("Shield Impact Noise Texture 2", "shield_impact_noise_texture2"),
    ] {
        let bitmap = root.read_tag_ref_path(field).filter(|bitmap| !bitmap.is_empty())?;
        let image = Arc::new(decode_h2_image(source, &bitmap, SamplerKind::Texture2D, 0)?);
        textures.push(H3Texture {
            uniform: uniform.to_owned(),
            image,
            texels: H3Texels::Linear,
            mips: Arc::default(),
            sampler: None,
            frames: Vec::new(),
        });
    }
    Some(Arc::new(H3ShieldSettings { procedural: flagged("use procedural uv"), values, textures }))
}

/// The pass, on the global settings a model that names none of its own uses.
fn h3_shield(source: &TagSource, caches: &mut ResolveCaches) -> Option<Arc<H3Shield>> {
    if let Some(found) = &caches.h3_shield {
        return found.clone();
    }
    let settings = read_h3_shield_settings(source, "globals\\global_shield_impact_settings");
    let sources = caches.h3_sources.get_or_insert_with(|| H3Sources::new(source));
    let shield = settings.and_then(|settings| {
        let pass = sources.explicit_pass("shield_impact").ok()?;
        let procedural = sources.explicit_pass("shield_impact_procedural_uv").ok();
        Some(Arc::new(H3Shield { pass, procedural, settings }))
    });
    caches.h3_shield = Some(shield.clone());
    shield
}

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
fn resolve_ce_shader(
    source: &TagSource,
    shader: &TagFile,
    group: [u8; 4],
    permutation: usize,
    caches: &mut ResolveCaches,
) -> MaterialTextures {
    let root = shader.root();
    // The root carries several `flags` fields, one per section, whose option
    // names don't repeat, so every set flag is read off all of them.
    let flags: Vec<String> = root
        .fields()
        .filter(|field| field.clean_name() == "flags")
        .filter_map(|field| match field.value()? {
            TagFieldData::ByteFlags { names, .. }
            | TagFieldData::WordFlags { names, .. }
            | TagFieldData::LongFlags { names, .. } => Some(names),
            _ => None,
        })
        .flatten()
        .map(|(_, name)| name)
        .collect();
    let has = |flag: &str| flags.iter().any(|name| name == flag);
    // Both scale fields store 0 for "unset"; a 0 at runtime would collapse
    // every texel to one (the cyborg ships `detail map v-scale` 0), so it can
    // only mean 1.
    let factor = |value: Option<f32>| value.filter(|v| v.is_finite() && *v != 0.0).unwrap_or(1.0);
    let mut bind = |path: Option<String>, scale: [f32; 2]| {
        let path = path.filter(|path| !path.is_empty())?;
        let image_index = ce_image_index(source, &path, permutation, &mut caches.ce_image_counts);
        let binding = SlotBitmap {
            path,
            image_index: image_index as i16,
            repeat_x: true,
            repeat_y: true,
            scale,
        };
        cached_bitmap(source, &binding, caches)
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
                function: detail_function(root.read_enum_name("detail function")),
                mask: CE_DETAIL_MASKS
                    .iter()
                    .position(|name| root.read_enum_name("detail mask").as_deref() == Some(*name))
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
                function: detail_function(root.read_enum_name("detail map function")),
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
        let image = ce_image_index(source, path, permutation, &mut caches.ce_image_counts);
        let key = (path.to_owned(), kind, image);
        if let Some(cached) = caches.h2_images.get(&key) {
            return cached.clone();
        }
        let decoded = decode_h2_image(source, path, kind, image).map(Arc::new);
        caches.h2_images.insert(key, decoded.clone());
        decoded
    };
    textures.h2_layers[H2Layer::Lightmap as usize] =
        ce_model_passes(source, root, &group, &flags, &mut common, &mut bitmap);
    let transparent = ce_transparent_passes(source, root, &group, &mut bitmap);
    if matches!(&group, b"soso" | b"senv") {
        let settings = caches.ce_camo.get_or_insert_with(|| read_ce_camo_settings(source));
        if let Some(settings) = settings {
            let two_sided = flags.iter().any(|name| name == "two-sided");
            textures.h2_layers[H2Layer::ActiveCamo as usize] =
                vec![ce_camo_pass(settings, two_sided, &mut bitmap)];
        }
    }
    textures.h2_layers[H2Layer::Transparent as usize] =
        expand_ce_digits(source, transparent, permutation, caches);
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
    source: &TagSource,
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
            load_referenced_tag_from_source(source, &numeric.map, "bitmap", b"bitm")
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
                    decode_h2_image(source, &numeric.map, SamplerKind::Texture2D, digit).map(Arc::new)
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
pub(super) const CE_DETAIL_MASKS: [&str; 9] = [
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

/// One Halo 2 template parameter's defaults: what a shader that does not
/// override the parameter draws with.
struct H2TemplateParameter {
    /// Empty when the template names none (`lightmap_alphatest_map` in most).
    default_bitmap: String,
    /// The tiling a shader inherits; 16 for `detail_map`, as in Halo 3's
    /// option defaults. 0 is how many templates spell "untiled".
    bitmap_scale: f32,
}

/// A Halo 2 `shader_template`'s parameters by name, plus whether it alpha
/// tests.
struct H2TemplateDefaults {
    parameters: HashMap<String, H2TemplateParameter>,
    alpha_tested: bool,
}

/// Halo 2's `shader` names its bitmaps in a `parameters` block, by the same
/// names Halo 3's render methods use (`base_map` 1,161 of the 1,637 shaders
/// halo2_mcc render_models use, `bump_map` 917, `detail_map` 528). What a
/// shader leaves out, its template's `categories[]/parameters[]` defaults:
/// `gray_50_percent` for `base_map`, `default_detail` at scale 16 for
/// `detail_map`, as the game does.
fn resolve_h2_shader(
    source: &TagSource,
    shader: &TagFile,
    caches: &mut ResolveCaches,
) -> MaterialTextures {
    let root = shader.root();
    let template = root
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| cached_h2_template(source, &path, &mut caches.h2_templates));
    let parameters = h2_shader_parameters(root);

    let mut textures = MaterialTextures {
        // Halo 2 combines on the stored values, as every pre-sRGB engine did.
        detail: DetailComposition {
            function: DetailFunction::BiasedMultiply,
            ..Default::default()
        },
        ..Default::default()
    };
    for (slot, name) in SLOT_PARAMETERS {
        let authored = parameters.get(name);
        let defaults = template.as_ref().and_then(|template| template.parameters.get(name));
        let path = authored
            .and_then(|parameter| parameter.read_tag_ref_path("bitmap"))
            .filter(|path| !path.is_empty())
            .or_else(|| defaults.map(|defaults| defaults.default_bitmap.clone()))
            .filter(|path| !path.is_empty());
        let Some(path) = path else {
            continue;
        };
        let scale = h2_bitmap_scale(authored, defaults);
        let binding = SlotBitmap {
            path,
            image_index: 0,
            // Halo 2 shaders carry no address mode; its samplers wrap.
            repeat_x: true,
            repeat_y: true,
            scale,
        };
        textures.slots[slot as usize] = cached_bitmap(source, &binding, caches);
    }
    // Inferred from the templates rather than read from their passes: the
    // `*alpha_test*` templates that declare no `alpha_test_map`
    // (`tex_alpha_test`, `tex_bump_alpha_test_single_pass`, ...) can only be
    // cutting out on the base map's alpha.
    if template.as_ref().is_some_and(|template| template.alpha_tested)
        && textures.slots[TextureSlot::AlphaTest as usize].is_none()
    {
        textures.slots[TextureSlot::AlphaTest as usize] =
            textures.slots[TextureSlot::Base as usize].clone();
    }
    // Without the template, what the shader authors still resolves; what is
    // lost is the defaults, the same as a render method without its definition.
    textures.used_shader_parameters_only = template.is_none();
    // The game skips the specular pass for a shader with no specular type.
    let has_specular = root.read_int_any("specular type").unwrap_or(0) != 0;
    for layer in H2Layer::ALL {
        if (layer == H2Layer::LightSpecular && !has_specular) || layer.tag_name().is_empty() {
            continue;
        }
        let has_texaccum = [H2Layer::Texaccum, H2Layer::LightAlbedo]
            .iter()
            .any(|layer| !textures.h2_layers[*layer as usize].is_empty());
        textures.h2_layers[layer as usize] = prepare_h2_passes(
            source,
            shader,
            &parameters,
            template.as_deref(),
            caches,
            layer,
            has_texaccum,
            None,
        );
    }
    let opted_out = root
        .read_flag_names("flags")
        .is_some_and(|flags| flags.iter().any(|(_, name)| name == "no active camo"));
    if !opted_out {
        const BUMP: &str = "active_camo_bump_map";
        let bump = parameters
            .get(BUMP)
            .and_then(|parameter| parameter.read_tag_ref_path("bitmap"))
            .filter(|path| !path.is_empty())
            .or_else(|| {
                let defaults = template.as_ref()?.parameters.get(BUMP)?;
                Some(defaults.default_bitmap.clone()).filter(|path| !path.is_empty())
            });
        textures.h2_layers[H2Layer::ActiveCamo as usize] =
            prepare_h2_camo_passes(source, bump, caches);
    }
    // Passes draw it with Game Shaders on: an emblem's overlay does, and is only
    // off for want of a player. Without them, and with no slots, it is bare.
    if textures.slots.iter().all(Option::is_none)
        && textures.h2_layers[H2Layer::Transparent as usize].is_empty()
    {
        let template = root.read_tag_ref_path("template").unwrap_or_default();
        let leaf = tag_path::name(&template);
        let reason = format!("its template ({leaf}) uses none of the maps this preview draws");
        if textures.h2_layers.iter().all(Vec::is_empty) {
            textures.error = Some(reason);
        } else {
            textures.bare_without_passes = Some(format!("{reason} without Game Shaders"));
        }
    }
    textures
}

/// A shader's `parameters` block by name, the first of a name winning.
fn h2_shader_parameters<'a>(root: TagStruct<'a>) -> HashMap<String, TagStruct<'a>> {
    let mut parameters = HashMap::new();
    if let Some(block) = root.field("parameters").and_then(|field| field.as_block()) {
        for parameter in (0..block.len()).filter_map(|index| block.element(index)) {
            if let Some(name) = parameter.read_string_id("name") {
                parameters.entry(name).or_insert(parameter);
            }
        }
    }
    parameters
}

/// The passes a shader draws active camo with. The game swaps every shader
/// but those flagged `no active camo` for one global on this layer (see
/// `rasterizer_visible_section_build_rendered_geometry`): `globals.globals`'
/// `global shader`, which reads the bump of the shader it replaced.
fn prepare_h2_camo_passes(
    source: &TagSource,
    bump: Option<String>,
    caches: &mut ResolveCaches,
) -> Vec<H2DrawPass> {
    let shader = caches
        .h2_camo_shader
        .get_or_insert_with(|| {
            let globals =
                load_referenced_tag_from_source(source, "globals\\globals", "globals", b"matg").ok()?;
            let root = globals.root();
            let path = root.descend("rasterizer data[0]")?.read_tag_ref_path("global shader")?;
            load_referenced_tag_from_source(source, &path, "shader", b"shad").ok().map(Arc::new)
        })
        .clone();
    let Some(shader) = shader else {
        return Vec::new();
    };
    let root = shader.root();
    let template = root
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| cached_h2_template(source, &path, &mut caches.h2_templates));
    prepare_h2_passes(
        source,
        &shader,
        &h2_shader_parameters(root),
        template.as_deref(),
        caches,
        H2Layer::ActiveCamo,
        false,
        bump,
    )
}

/// A parameter's tiling: what the shader authors over the template's default.
fn h2_bitmap_scale(
    authored: Option<&TagStruct<'_>>,
    defaults: Option<&H2TemplateParameter>,
) -> [f32; 2] {
    let positive = |scale: f32| (scale.is_finite() && scale > 0.0).then_some(scale);
    let inherited = defaults.and_then(|defaults| positive(defaults.bitmap_scale)).unwrap_or(1.0);
    authored
        .and_then(|parameter| h2_scale_animation(parameter))
        .map(|[x, y]| [x.and_then(positive).unwrap_or(inherited), y.and_then(positive).unwrap_or(inherited)])
        .unwrap_or([inherited, inherited])
}

/// The image a Halo CE permutation picks of a bitmap: `rasterizer_set_texture`
/// wraps it at the bitmap's count.
fn ce_image_index(
    source: &TagSource,
    path: &str,
    permutation: usize,
    counts: &mut HashMap<String, usize>,
) -> usize {
    if permutation == 0 {
        return 0;
    }
    let count = *counts.entry(path.to_owned()).or_insert_with(|| {
        load_referenced_tag_from_source(source, path, "bitmap", b"bitm")
            .ok()
            .and_then(|tag| blam_tags::bitmap::Bitmap::new(&tag).ok().map(|bitmap| bitmap.len()))
            .unwrap_or(1)
    });
    permutation % count.max(1)
}

/// Decode a map in the shape its sampler asks for.
pub(super) fn decode_h2_image(source: &TagSource, path: &str, kind: SamplerKind, index: usize) -> Option<H2Image> {
    let tag = load_referenced_tag_from_source(source, path, "bitmap", b"bitm").ok()?;
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

/// A shader's transparent passes, ready to draw. A pass that cannot be
/// prepared drops them all, and the material keeps its slot drawing rather
/// than losing a layer: this needs a loose kit's `.psh` files.
fn prepare_h2_passes(
    source: &TagSource,
    shader: &TagFile,
    parameters: &HashMap<String, TagStruct<'_>>,
    template: Option<&H2TemplateDefaults>,
    caches: &mut ResolveCaches,
    layer: H2Layer,
    has_texaccum: bool,
    camo_bump: Option<String>,
) -> Vec<H2DrawPass> {
    let TagSource::LooseFolder { root, .. } = source else {
        return Vec::new();
    };
    let load = |path: &str, extension: &str, group: &[u8; 4]| {
        load_referenced_tag_from_source(source, path, extension, group).ok()
    };
    let mut passes = match resolve_h2_layer_passes(shader, load, layer.tag_name(), layer.implementation()) {
        Ok(passes) => passes,
        Err(error) => {
            eprintln!("model preview: {error}");
            return Vec::new();
        }
    };
    if let Some(path) = camo_bump {
        for pass in &mut passes {
            pass.parameters.retain(|(name, _)| name != "bump_map");
            pass.parameters.push(("bump_map".to_owned(), H2ParameterValue::Bitmap(path.clone())));
        }
    }
    let mut cases = HashMap::from(["specular", "lightmap"].map(|name| {
        let value = shader.root().read_int_any(&format!("{name} type")).unwrap_or(0) as i32;
        (format!("{name}_type"), value)
    }));
    // A default light's definition has specular on; see `rasterizer_get_switch_extern`.
    cases.insert("light_definition_specular_enabled".to_owned(), 1);
    cases.insert("light_diffuse_contrast".to_owned(), layer.diffuse_contrast());
    let directory = root.join("rasterizer").join("pixel_shaders_dx9");
    let specular_brightness =
        shader.root().read_real("lightmap specular brightness").unwrap_or(1.0);
    let mut prepared = Vec::with_capacity(passes.len());
    for pass in &passes {
        // The texaccum target and friends need a frame this preview does not
        // render, so such a material keeps the slots.
        if layer != H2Layer::Transparent && h2_pass_needs_frame_texture(pass, has_texaccum) {
            return Vec::new();
        }
        let file = directory.join(pass.pixel_shader_file());
        let bytes = match std::fs::read(&file) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("model preview: {}: {error}", file.display());
                return Vec::new();
            }
        };
        let psh = String::from_utf8_lossy(&bytes);
        let drawn = prepare_draw_pass(
            pass,
            &psh,
            cases.clone(),
            specular_brightness,
            |path, kind| {
                let key = (path.to_owned(), kind, 0);
                if let Some(cached) = caches.h2_images.get(&key) {
                    return cached.clone();
                }
                let decoded = decode_h2_image(source, path, kind, 0).map(Arc::new);
                caches.h2_images.insert(key, decoded.clone());
                decoded
            },
            |name| {
                h2_bitmap_scale(
                    parameters.get(name),
                    template.and_then(|template| template.parameters.get(name)),
                )
            },
        );
        match drawn {
            Ok(drawn) => prepared.push(drawn),
            Err(error) => {
                if layer == H2Layer::Transparent {
                    eprintln!("model preview: {}: {error}", pass.path);
                }
                return Vec::new();
            }
        }
    }
    prepared
}

/// A Halo 2 parameter's own tiling, per axis, from its animation properties:
/// each function's value at rest. `bitmap scale uniform` sets both axes; the
/// `x`/`y` pair (all `brute_head`'s detail map carries) sets one each. An axis
/// nothing authors is `None`, left to the template's default.
fn h2_scale_animation(parameter: &TagStruct<'_>) -> Option<[Option<f32>; 2]> {
    let block = parameter.field("animation properties")?.as_block()?;
    let value_of = |kind: &str| {
        (0..block.len())
            .filter_map(|index| block.element(index))
            .find(|property| property.read_enum_name("type").as_deref() == Some(kind))
            .and_then(|property| Some(animation_function(&property)?.evaluate(0.0, 0.0)))
            .filter(|value| value.is_finite())
    };
    let uniform = value_of("bitmap scale uniform");
    let axes = [
        value_of("bitmap scale x").or(uniform),
        value_of("bitmap scale y").or(uniform),
    ];
    axes.iter().any(Option::is_some).then_some(axes)
}

fn cached_h2_template(
    source: &TagSource,
    path: &str,
    cache: &mut HashMap<String, Option<Arc<H2TemplateDefaults>>>,
) -> Option<Arc<H2TemplateDefaults>> {
    if let Some(cached) = cache.get(path) {
        return cached.clone();
    }
    let loaded = load_referenced_tag_from_source(source, path, "shader_template", b"stem")
        .ok()
        .map(|template| {
            let mut parameters = HashMap::new();
            if let Some(categories) = template.root().field("categories").and_then(|f| f.as_block()) {
                for category in (0..categories.len()).filter_map(|index| categories.element(index)) {
                    let Some(block) = category.field("parameters").and_then(|f| f.as_block()) else {
                        continue;
                    };
                    for parameter in (0..block.len()).filter_map(|index| block.element(index)) {
                        let Some(name) = parameter.read_string_id("name") else {
                            continue;
                        };
                        parameters.entry(name).or_insert(H2TemplateParameter {
                            default_bitmap: parameter
                                .read_tag_ref_path("default bitmap")
                                .unwrap_or_default(),
                            bitmap_scale: parameter.read_real("bitmap scale").unwrap_or(0.0),
                        });
                    }
                }
            }
            let leaf = tag_path::name(path);
            Arc::new(H2TemplateDefaults {
                parameters,
                alpha_tested: leaf.contains("alpha_test"),
            })
        });
    cache.insert(path.to_owned(), loaded.clone());
    loaded
}

/// Where a named parameter's value comes from.
///
/// The resolver is preferred because a good deal of this is *option defaults*
/// rather than anything the shader authors. Reading the shader block alone
/// finds only what it overrides.
struct Values<'a> {
    resolved: Option<&'a ResolvedRenderMethod>,
    method: &'a RenderMethod,
    /// `render_method`'s `sort layer`, by option name.
    sort_layer: i32,
}

/// What one slot binds to.
struct SlotBitmap {
    path: String,
    image_index: i16,
    repeat_x: bool,
    repeat_y: bool,
    scale: [f32; 2],
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

fn parameter_named<'a>(
    render_method: &'a RenderMethod,
    name: &str,
) -> Option<&'a RenderMethodParameter> {
    render_method
        .parameters
        .iter()
        .find(|candidate| candidate.parameter_name == name)
}

fn cached_bitmap(
    source: &TagSource,
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

    /// Kits to resolve against: env var first, then the usual Steam location.
    ///
    /// Halo 3 and Reach both, deliberately — they are the two games this targets,
    /// and a slot table that silently only worked for one is exactly the failure
    /// worth catching.
    fn kit_root(env: &str, default: &str) -> Option<PathBuf> {
        let root = std::env::var_os(env)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(default));
        root.is_dir().then_some(root)
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
        env: &'static str,
        default_root: &'static str,
        model: &'static str,
    }

    const CASES: [Case; 3] = [
        Case {
            game: "halo3_mcc",
            env: "BABOON_H3EK_TAGS",
            default_root: r"D:\SteamLibrary\steamapps\common\H3EK\tags",
            model: r"objects\characters\masterchief\masterchief",
        },
        Case {
            game: "halo3_mcc",
            env: "BABOON_H3EK_TAGS",
            default_root: r"D:\SteamLibrary\steamapps\common\H3EK\tags",
            model: r"objects\characters\dervish\dervish",
        },
        Case {
            game: "haloreach_mcc",
            env: "BABOON_HREK_TAGS",
            default_root: r"D:\SteamLibrary\steamapps\common\HREK\tags",
            model: r"objects\characters\brute\brute",
        },
    ];

    /// The whole chain, on a shipped character: render_model → per-part material →
    /// shader tag → resolved parameters → decoded bitmaps.
    #[test]
    fn a_shipped_character_resolves_to_real_textures_in_both_games() {
        let mut ran = 0;
        for case in CASES {
            let Some(root) = kit_root(case.env, case.default_root) else {
                eprintln!(
                    "skipping: {} not present (set {})",
                    case.default_root, case.env
                );
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
        let Some(root) = kit_root(CASES[0].env, CASES[0].default_root) else {
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
        let Some(root) = kit_root(CASES[1].env, CASES[1].default_root) else {
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
        let Some(root) = kit_root(CASES[0].env, CASES[0].default_root) else {
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
        let tags = std::path::PathBuf::from(crate::core::test_kits::tag_path("haloce_mcc", ""));
        if !tags.join("characters/cyborg/shaders/armor.shader_model").is_file() {
            eprintln!("skipping: set BLAM_TEST_HCEEK to a Halo CE kit's tags folder");
            return;
        }
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
        let Ok(root) = std::env::var("BLAM_TEST_HCEEK") else {
            eprintln!("skipping: BLAM_TEST_HCEEK is not set");
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloCe),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
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
        let tags = crate::core::test_kits::h2ek_tags();
        if !tags.join("objects/characters/masterchief/shaders/masterchief.shader").is_file() {
            eprintln!("skipping: set BLAM_TEST_H2EK to a Halo 2 kit's tags folder");
            return;
        }
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
        let Ok(root) = std::env::var("BLAM_TEST_H2EK") else {
            eprintln!("skipping: BLAM_TEST_H2EK is not set");
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::Halo2),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let path = "objects\\bitmaps\\reflection_maps\\vehicle_glass";
        let Some(H2Image::Cube { size, faces }) = decode_h2_image(&source, path, SamplerKind::Cube, 0)
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
        let Ok(root) = std::env::var("BLAM_TEST_H2EK") else {
            eprintln!("skipping: BLAM_TEST_H2EK is not set");
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::Halo2),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
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
        let Ok(root) = std::env::var("BLAM_TEST_H2EK") else {
            eprintln!("skipping: BLAM_TEST_H2EK is not set");
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::Halo2),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let Some(H2Image::Volume { width, height, depth, rgba }) =
            decode_h2_image(&source, "rasterizer\\plasma_shield_noise", SamplerKind::Volume, 0)
        else {
            panic!("the plasma noise should decode as a volume");
        };
        assert!(depth > 1);
        assert_eq!(rgba.len(), width * height * depth * 4);
        let slice = width * height * 4;
        assert_ne!(rgba[..slice], rgba[slice * (depth - 1)..], "slices differ");
    }

    /// With `BLAM_TEST_HREK`: `color_black`, the change-color map Emile's flair
    /// names, is `a8` and samples black, so `two_change_color` leaves it
    /// untinted; `gray_50_percent`, an `a8r8g8b8`, keeps its gray.
    #[test]
    fn kit_an_a8_bitmap_samples_black_with_its_alpha() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            eprintln!("skipping: BLAM_TEST_HREK is not set");
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloReach),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let first_texel = |path: &str| {
            let (image, _, _) = read_h3_bitmap(&source, path, 0, SamplerKind::Texture2D).expect(path);
            let H2Image::Flat(flat) = &*image else {
                panic!("{path} should be flat");
            };
            [flat.rgba[0], flat.rgba[1], flat.rgba[2], flat.rgba[3]]
        };
        assert_eq!(first_texel("shaders\\default_bitmaps\\bitmaps\\color_black"), [0, 0, 0, 255]);
        assert_eq!(first_texel("shaders\\default_bitmaps\\bitmaps\\gray_50_percent")[..3], [128, 128, 128]);
    }

    /// With `BLAM_TEST_HREK`: an array's layers each keep their whole chain,
    /// so `globals\global_render_texture`'s second layer starts 21844 bytes
    /// in, not a level's 16384 in as a volume's would.
    #[test]
    fn kit_an_arrays_layers_are_whole_chains_apart() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            eprintln!("skipping: BLAM_TEST_HREK is not set");
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloReach),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let path = "globals\\global_render_texture";
        let Some(H2Image::Volume { width, height, depth, rgba }) = decode_h2_image(&source, path, SamplerKind::Volume, 0)
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
