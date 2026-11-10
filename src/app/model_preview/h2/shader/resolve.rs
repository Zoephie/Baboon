//! Halo 2's shaders resolved over their templates' defaults to the passes and
//! textures the preview draws.

use super::super::super::common::fields::has_flag;
use super::super::super::dx9::passes::{Dx9DrawPass, Dx9Layer, animation_function};
use super::super::super::materials::{
    DetailComposition, DetailFunction, MaterialTextures, ResolveCaches, SLOT_PARAMETERS,
    SlotBitmap, TextureSlot, cached_bitmap, decode_image,
};
use super::super::super::{TagFile, TagReader, TagSource, TagStruct, tag_path};
use super::super::shader::{
    h2_pass_needs_frame_texture, prepare_draw_pass, resolve_h2_layer_passes,
};
use super::template::Dx9ParameterValue;
use std::collections::HashMap;
use std::sync::Arc;

/// One Halo 2 template parameter's defaults: what a shader that does not
/// override the parameter draws with.
struct H2TemplateParameter {
    /// Empty when the template names none (`lightmap_alphatest_map` in most).
    default_bitmap: String,
    /// The tiling a shader inherits; 16 for `detail_map`, as in Halo 3's
    /// option defaults. 0 is how many templates spell "untiled".
    bitmap_scale: f32,
}

/// What a resolve keeps of Halo 2's across a model's materials.
#[derive(Default)]
pub(in crate::app::model_preview) struct H2ResolveCaches {
    /// Shader templates' parameter defaults, by template path. 1,637 shaders
    /// share a few dozen templates; `tex_bump` alone serves 296.
    pub(in crate::app::model_preview) templates: HashMap<String, Option<Arc<H2TemplateDefaults>>>,
    /// The kit's `global shader`, read at most once; see
    /// [`prepare_h2_camo_passes`].
    pub(in crate::app::model_preview) camo_shader: Option<Option<Arc<TagFile>>>,
}

/// A Halo 2 `shader_template`'s parameters by name, plus whether it alpha
/// tests.
pub(in crate::app::model_preview) struct H2TemplateDefaults {
    parameters: HashMap<String, H2TemplateParameter>,
    alpha_tested: bool,
}

/// Halo 2's `shader` names its bitmaps in a `parameters` block, by the same
/// names Halo 3's render methods use (`base_map` 1,161 of the 1,637 shaders
/// halo2_mcc render_models use, `bump_map` 917, `detail_map` 528). What a
/// shader leaves out, its template's `categories[]/parameters[]` defaults:
/// `gray_50_percent` for `base_map`, `default_detail` at scale 16 for
/// `detail_map`, as the game does.
pub(in crate::app::model_preview) fn resolve_h2_shader(
    reader: &TagReader,
    shader: &TagFile,
    caches: &mut ResolveCaches,
) -> MaterialTextures {
    let root = shader.root();
    let template = root
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| cached_h2_template(reader, &path, &mut caches.h2.templates));
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
        let defaults = template
            .as_ref()
            .and_then(|template| template.parameters.get(name));
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
        textures.slots[slot as usize] = cached_bitmap(reader, &binding, caches);
    }
    // Inferred from the templates rather than read from their passes: the
    // `*alpha_test*` templates that declare no `alpha_test_map`
    // (`tex_alpha_test`, `tex_bump_alpha_test_single_pass`, ...) can only be
    // cutting out on the base map's alpha.
    if template
        .as_ref()
        .is_some_and(|template| template.alpha_tested)
        && textures.slots[TextureSlot::AlphaTest as usize].is_none()
    {
        textures.slots[TextureSlot::AlphaTest as usize] =
            textures.slots[TextureSlot::Base as usize].clone();
    }
    // Without the template, what the shader authors still resolves; what is
    // lost is the defaults, the same as a render method without its definition.
    textures.used_shader_parameters_only = template.is_none();
    // The game skips the specular pass for a shader with no specular type.
    let has_specular = !root
        .field("specular type")
        .is_some_and(|field| field.enum_is("none"));
    for layer in Dx9Layer::ALL {
        if (layer == Dx9Layer::LightSpecular && !has_specular) || layer.tag_name().is_empty() {
            continue;
        }
        let has_texaccum = [Dx9Layer::Texaccum, Dx9Layer::LightAlbedo]
            .iter()
            .any(|layer| !textures.dx9_layers[*layer as usize].is_empty());
        textures.dx9_layers[layer as usize] = prepare_h2_passes(
            reader,
            shader,
            &parameters,
            template.as_deref(),
            caches,
            layer,
            has_texaccum,
            None,
        );
    }
    if !has_flag(root, "flags", "no active camo") {
        const BUMP: &str = "active_camo_bump_map";
        let bump = parameters
            .get(BUMP)
            .and_then(|parameter| parameter.read_tag_ref_path("bitmap"))
            .filter(|path| !path.is_empty())
            .or_else(|| {
                let defaults = template.as_ref()?.parameters.get(BUMP)?;
                Some(defaults.default_bitmap.clone()).filter(|path| !path.is_empty())
            });
        textures.dx9_layers[Dx9Layer::ActiveCamo as usize] =
            prepare_h2_camo_passes(reader, bump, caches);
    }
    // Passes draw it with Game Shaders on, transparent ones too: an emblem's
    // overlay does, and is only off for want of a player. Without them, and
    // with no slots, it is bare.
    if textures.slots.iter().all(Option::is_none) {
        let template = root.read_tag_ref_path("template").unwrap_or_default();
        let leaf = tag_path::name(&template);
        let reason = format!("its template ({leaf}) uses none of the maps this preview draws");
        if textures.dx9_layers.iter().all(Vec::is_empty) {
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
        for parameter in block.iter() {
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
    reader: &TagReader,
    bump: Option<String>,
    caches: &mut ResolveCaches,
) -> Vec<Dx9DrawPass> {
    let shader = caches
        .h2
        .camo_shader
        .get_or_insert_with(|| {
            let globals = reader.load("globals\\globals", "globals", b"matg").ok()?;
            let root = globals.root();
            let path = root
                .descend("rasterizer data[0]")?
                .read_tag_ref_path("global shader")?;
            reader.load(&path, "shader", b"shad").ok()
        })
        .clone();
    let Some(shader) = shader else {
        return Vec::new();
    };
    let root = shader.root();
    let template = root
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| cached_h2_template(reader, &path, &mut caches.h2.templates));
    prepare_h2_passes(
        reader,
        &shader,
        &h2_shader_parameters(root),
        template.as_deref(),
        caches,
        Dx9Layer::ActiveCamo,
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
    let inherited = defaults
        .and_then(|defaults| positive(defaults.bitmap_scale))
        .unwrap_or(1.0);
    authored
        .and_then(|parameter| h2_scale_animation(parameter))
        .map(|[x, y]| {
            [
                x.and_then(positive).unwrap_or(inherited),
                y.and_then(positive).unwrap_or(inherited),
            ]
        })
        .unwrap_or([inherited, inherited])
}

/// A shader's transparent passes, ready to draw. A pass that cannot be
/// prepared drops them all, and the material keeps its slot drawing rather
/// than losing a layer: this needs a loose kit's `.psh` files.
fn prepare_h2_passes(
    reader: &TagReader,
    shader: &TagFile,
    parameters: &HashMap<String, TagStruct<'_>>,
    template: Option<&H2TemplateDefaults>,
    caches: &mut ResolveCaches,
    layer: Dx9Layer,
    has_texaccum: bool,
    camo_bump: Option<String>,
) -> Vec<Dx9DrawPass> {
    let TagSource::LooseFolder { root, .. } = &reader.source else {
        return Vec::new();
    };
    let load =
        |path: &str, extension: &str, group: &[u8; 4]| reader.load(path, extension, group).ok();
    let mut passes =
        match resolve_h2_layer_passes(shader, load, layer.tag_name(), layer.implementation()) {
            Ok(passes) => passes,
            Err(error) => {
                eprintln!("model preview: {error}");
                return Vec::new();
            }
        };
    if let Some(path) = camo_bump {
        for pass in &mut passes {
            pass.parameters.retain(|(name, _)| name != "bump_map");
            pass.parameters.push((
                "bump_map".to_owned(),
                Dx9ParameterValue::Bitmap(path.clone()),
            ));
        }
    }
    let mut cases = HashMap::from(["specular", "lightmap"].map(|name| {
        let value = shader
            .root()
            .read_int_any(&format!("{name} type"))
            .unwrap_or(0) as i32;
        (format!("{name}_type"), value)
    }));
    // A default light's definition has specular on; see `rasterizer_get_switch_extern`.
    cases.insert("light_definition_specular_enabled".to_owned(), 1);
    cases.insert(
        "light_diffuse_contrast".to_owned(),
        layer.diffuse_contrast(),
    );
    let directory = root.join("rasterizer").join("pixel_shaders_dx9");
    let specular_brightness = shader
        .root()
        .read_real("lightmap specular brightness")
        .unwrap_or(1.0);
    let mut prepared = Vec::with_capacity(passes.len());
    for pass in &passes {
        // The texaccum target and friends need a frame this preview does not
        // render, so such a material keeps the slots.
        if layer != Dx9Layer::Transparent && h2_pass_needs_frame_texture(pass, has_texaccum) {
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
                if let Some(cached) = caches.dx9_images.get(&key) {
                    return cached.clone();
                }
                let decoded = decode_image(reader, path, kind, 0).map(Arc::new);
                caches.dx9_images.insert(key, decoded.clone());
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
                if layer == Dx9Layer::Transparent {
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
            .find(|property| property.read_enum_option("type").as_deref() == Some(kind))
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
    reader: &TagReader,
    path: &str,
    cache: &mut HashMap<String, Option<Arc<H2TemplateDefaults>>>,
) -> Option<Arc<H2TemplateDefaults>> {
    if let Some(cached) = cache.get(path) {
        return cached.clone();
    }
    let loaded = reader
        .load(path, "shader_template", b"stem")
        .ok()
        .map(|template| {
            let mut parameters = HashMap::new();
            if let Some(categories) = template
                .root()
                .field("categories")
                .and_then(|f| f.as_block())
            {
                for category in categories.iter() {
                    let Some(block) = category.field("parameters").and_then(|f| f.as_block())
                    else {
                        continue;
                    };
                    for parameter in block.iter() {
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
