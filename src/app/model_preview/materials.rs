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
    definitions: HashMap<String, Option<Arc<RenderMethodDefinition>>>,
    options: HashMap<String, Option<Arc<RenderMethodOption>>>,
    /// Decoded bitmaps by `(path, image index)`. Shared maps are common —
    /// masterchief's visor variants all reach for the same detail map.
    bitmaps: HashMap<(String, i16), Option<TextureImage>>,
    /// Halo 2 shader templates' parameter defaults, by template path. 1,637
    /// shaders share a few dozen templates; `tex_bump` alone serves 296.
    h2_templates: HashMap<String, Option<Arc<H2TemplateDefaults>>>,
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
    match blam_tags::game::Game::of(&shader) {
        blam_tags::game::Game::Halo1 => return resolve_ce_shader(source, &shader, group, caches),
        blam_tags::game::Game::Halo2 if group == *b"shad" => {
            return resolve_h2_shader(source, &shader, caches);
        }
        _ => {}
    }
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
            // The engine's resolver takes owned options; this runs once per
            // texture resolve on a worker, not per frame.
            cached_render_method_option(source, option_path, &mut caches.options)
                .map(|option| (*option).clone())
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
        let binding = SlotBitmap {
            path,
            image_index: 0,
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
        _ => {
            return MaterialTextures::failed(
                "a Halo CE transparent shader — drawn blended in game, so not textured \
                 in this opaque preview",
            );
        }
    }
    if textures.slots.iter().all(Option::is_none) {
        textures.error = Some("the shader names no base or detail map".to_owned());
    }
    textures
}

/// Halo CE `detail mask` options in order; the index is what the fragment
/// shader decodes (see [`DetailComposition::mask`]).
const CE_DETAIL_MASKS: [&str; 9] = [
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
    let mut parameters: HashMap<String, TagStruct<'_>> = HashMap::new();
    if let Some(block) = root.field("parameters").and_then(|field| field.as_block()) {
        for parameter in (0..block.len()).filter_map(|index| block.element(index)) {
            if let Some(name) = parameter.read_string_id("name") {
                parameters.entry(name).or_insert(parameter);
            }
        }
    }

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
        let positive = |scale: f32| (scale.is_finite() && scale > 0.0).then_some(scale);
        let inherited = defaults.and_then(|defaults| positive(defaults.bitmap_scale)).unwrap_or(1.0);
        let scale = authored
            .and_then(h2_scale_animation)
            .map(|[x, y]| [x.and_then(positive).unwrap_or(inherited), y.and_then(positive).unwrap_or(inherited)])
            .unwrap_or([inherited, inherited]);
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
    // 633 of halo2_mcc's 3,147 model materials land here, nearly all on
    // `transparent\*` templates (skies, plasma, alpha-blended env) and the
    // self-illumination-only `illum*` ones.
    if textures.slots.iter().all(Option::is_none) {
        let template = root.read_tag_ref_path("template").unwrap_or_default();
        let leaf = template.rsplit(['\\', '/']).next().unwrap_or(&template);
        textures.error = Some(format!(
            "its template ({leaf}) uses none of the maps this preview draws"
        ));
    }
    textures
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
            .and_then(|property| {
                let data = property.field_path("function/data")?.as_block()?;
                let bytes: Vec<u8> = (0..data.len())
                    .filter_map(|index| data.element(index))
                    .filter_map(|byte| byte.read_int_any("Value").map(|value| value as i8 as u8))
                    .collect();
                let function = crate::app::editor::function_editor::h2_tag_function(&bytes)?;
                Some(function.evaluate(0.0, 0.0))
            })
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
            let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
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
    scale: [f32; 2],
}

/// Find a slot's bitmap: through the resolver when the definition loaded, and
/// off the shader's own parameters when it did not.
fn slot_binding(values: &Values<'_>, parameter: &str) -> Option<SlotBitmap> {
    let scale = [slot_scale(values, parameter); 2];
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

    /// The detail maps' tiling comes from the resolver's option defaults.
    ///
    /// `detail_map_scale_uniform` is **16** and dervish's shader never mentions it,
    /// so reading only the shader's own block found nothing and every detail map
    /// tiled once across the whole model — which is most of what "detail" is for.
    #[test]
    fn detail_tiling_comes_from_the_option_defaults() {
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
            }],
        );
        let material = &resolved[0];
        assert!(
            !material.used_shader_parameters_only,
            "the render-method definition should load; without it the option          default below is unreachable"
        );

        for slot in [TextureSlot::Detail, TextureSlot::BumpDetail] {
            let image = material
                .get(slot)
                .expect("dervish carries both detail maps");
            assert!(
                image.scale.iter().all(|scale| (scale - 16.0).abs() < 0.01),
                "detail tiling should come from the option default, got {:?}",
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

        // Transparent effects stay untextured, and say why.
        let shield = resolve_classic(
            &source,
            r"characters\cyborg\shaders\light shield",
            b"schi",
        );
        assert!(shield.error.as_deref().is_some_and(|error| error.contains("transparent")));
        assert!(shield.slots.iter().all(Option::is_none));
        // `smet` is Halo 4's `structure_meta` in the cross-game extension table;
        // the kit's own names make it `shader_transparent_meter`, so the shader
        // loads and is reported as what it is rather than as a missing file.
        let meter = resolve_classic(&source, r"vehicles\warthog\shaders\meter engine", b"smet");
        assert!(
            meter.error.as_deref().is_some_and(|error| error.contains("transparent")),
            "{:?}",
            meter.error
        );
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
        let (summary, detail) = texture_resolve_note(&textures, &materials).unwrap();
        assert_eq!(
            summary,
            "1 of 3 materials untextured; 1 read without their definition or template defaults \
             — hover for why"
        );
        assert!(detail.contains("shaders/a: shader tag not found"));
        assert!(detail.contains("shaders/b"));
        assert!(!detail.contains("shaders/c"));
        assert_eq!(texture_resolve_note(&textures[2..], &materials[2..]), None);
    }
}
