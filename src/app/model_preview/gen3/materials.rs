//! The gen3 games' render methods resolved to the materials and bitmaps the
//! gen3 pipeline draws.

use super::super::*;
use blam_tags::render_method::{
    BitmapAddressMode, BitmapFilterMode, ParameterSource, RenderMethod, RenderMethodAnimatedParameterType,
    RenderMethodBitmapExternMode,
    RenderMethodDefinition, RenderMethodParameterType, ResolvedRenderMethod,
    ResolvedValue,
};
use super::super::common::fields::{elements, has_flag};
use super::super::gen3::shaders::{H3Animated, H3Animation, H3Blend, H3Clock, H3Material, H3Pass, H3Shield, H3ShieldSettings, H3Sources, H3Texture, H3Value, RenderMethodDialect};
use crate::app::model_preview::common::image::{H3Sampler, H3Texels};
use super::super::common::image::H2Image;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::hlsl::parse::TextureKind;
use std::sync::Arc;
use std::collections::HashMap;

/// A bitmap for a pass, and whether `curve` has the card decode it.
pub(in crate::app::model_preview) fn h3_image(
    reader: &TagReader,
    path: &str,
    index: i16,
    kind: SamplerKind,
    caches: &mut ResolveCaches,
) -> Option<H3Bitmap> {
    let key = (path.to_owned(), index, kind as usize);
    if let Some(found) = caches.h3_images.get(&key) {
        return found.clone();
    }
    let found = read_h3_bitmap(reader, path, index, kind);
    caches.h3_images.insert(key, found.clone());
    found
}

/// One image of a bitmap, what its texels mean, and its stored mips.
pub(in crate::app::model_preview) type H3Bitmap = (Arc<H2Image>, H3Texels, Arc<Vec<H2Image>>);

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

pub(in crate::app::model_preview) fn read_h3_bitmap(reader: &TagReader, path: &str, index: i16, kind: SamplerKind) -> Option<H3Bitmap> {
    let tag = reader.load(path, "bitmap", b"bitm").ok();
    // `render_method_submit`: an index past the last image takes the last.
    let images = tag.as_ref().map_or(1, |tag| elements(tag.root(), "bitmaps*").len());
    let index = (index.max(0) as usize).min(images.saturating_sub(1)) as i16;
    let image = decode_h2_image(reader, path, kind, index as usize);
    image.map(|mut image| {
        // `c_rasterizer_texture_ref::allocate`: xRGB and sRGB curves.
        let curve = tag.as_ref().and_then(|tag| {
            elements(tag.root(), "bitmaps*").get(index.max(0) as usize)?.read_int_any("curve")
        });
        let format = tag.as_ref().and_then(|tag| {
            blam_tags::bitmap::Bitmap::new(tag).ok()?.image(index.max(0) as usize)?.format().ok()
        });
        let halo4 = reader.source.game() == Some(crate::core::game::GameId::Halo4);
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
pub(in crate::app::model_preview) fn reach_version_update(method: &mut RenderMethod, definition: &RenderMethodDefinition) {
    if method.class == blam_tags::render_method::RenderMethodClass::Shader {
        method.version_update(definition);
    }
}

/// The template a Reach shader names as its `reference`, which Reach's tool
/// (`postprocess_as_child`, 0x141166270) lays under it before the bake. A
/// shader naming itself has none, as tool refuses it.
pub(in crate::app::model_preview) fn reach_reference(
    reader: &TagReader,
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
            let tag = reader.load(path, extension, &group.to_be_bytes()).ok()?;
            let mut reference = RenderMethod::from_tag(&tag).ok()?;
            reach_version_update(&mut reference, definition);
            Some(Arc::new(reference))
        })
        .clone()
}

/// A render method held inside another tag, resolved as the bake would.
pub(in crate::app::model_preview) fn resolve_embedded_method(reader: &TagReader, shader: TagStruct<'_>) -> Option<ResolvedRenderMethod> {
    let mut method = RenderMethod::from_struct(&shader).ok()?;
    let definition = reader.render_method_definition(&method.definition_path)?;
    if reader.source.game() == Some(GameId::HaloReach) {
        let reference = reach_reference(reader, &method, "", &definition, &mut HashMap::new());
        method.apply_reference_shader(&definition, reference.as_deref());
    }
    let resolved = ResolvedRenderMethod::resolve(&method, &definition, |option_path| {
        reader.render_method_option(option_path).map(|option| (*option).clone())
    });
    Some(resolved)
}

/// A map of a resolved method, with the sampler the bake gave it: the option's
/// default wherever the shader's `bitmap flags` claim no field of it.
pub(in crate::app::model_preview) fn read_resolved_map(
    reader: &TagReader,
    resolved: &ResolvedRenderMethod,
    name: &str,
) -> Option<(H3Bitmap, H3Sampler)> {
    let ParameterSource::Inline(ResolvedValue::Bitmap(binding)) = &resolved.find(name)?.source else {
        return None;
    };
    let bitmap = read_h3_bitmap(reader, &binding.bitmap_path, binding.bitmap_index, SamplerKind::Texture2D)?;
    let sampler = H3Sampler {
        address: [binding.address_mode_x, binding.address_mode_y],
        filter: binding.filter_mode,
    };
    Some((bitmap, sampler))
}

/// A map's `_xform`: its scale, then its offset. Tool's
/// `render_method_update_real_parameter` starts from identity and takes only
/// the shader's own animators; the option's default bitmap scale is not used.
pub(in crate::app::model_preview) fn h3_xform(values: &Values<'_>, parameter: &str) -> [f32; 4] {
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

pub(in crate::app::model_preview) fn prepare_h3_material(
    reader: &TagReader,
    values: &Values<'_>,
    caches: &mut ResolveCaches,
) -> Result<H3Material, String> {
    let method = values.method;
    let compiled = compile_passes(reader, method, caches)?;
    let mut material = H3Material {
        dialect: compiled.dialect.into(),
        albedo: compiled.albedo,
        lit: compiled.lit,
        screen_maps: screen_maps(method),
        decal: compiled.decal,
        decal_modulates_specular: compiled.decal_modulates_specular,
        after_lighting: compiled.after_lighting,
        sort_layer: values.sort_layer,
        prt: compiled.prt,
        camo: compiled.camo,
        shadow_generate: compiled.shadow_generate,
        dynamic_light: compiled.dynamic_light,
        blend: compiled.blend,
        values: Default::default(),
        textures: Vec::new(),
        change_colors: Vec::new(),
        animations: Vec::new(),
        shield: None,
        fringe: None,
    };
    if compiled.blend == H3Blend::Opaque {
        material.shield = h3_shield(reader, caches);
    }
    let passes = material_passes(&material);
    let clock = animation_clock(reader, compiled.screen);
    material.animations = parameter_animations(method, clock);
    for pass in &passes {
        bind_pass_values(values, pass, &mut material);
        bind_pass_textures(reader, values, pass, &mut material, caches);
        if compiled.dialect == RenderMethodDialect::Reach {
            reach_environment_maps(reader, pass, &mut material, caches);
        }
    }
    Ok(material)
}

/// What a render method's definition compiles to for its options: its
/// passes, and the flags its blend and family set.
struct CompiledPasses {
    dialect: RenderMethodDialect,
    blend: H3Blend,
    decal: bool,
    decal_modulates_specular: bool,
    after_lighting: bool,
    /// Whether the definition is `screen`'s, which its clock follows.
    screen: bool,
    lit: Arc<H3Pass>,
    albedo: Option<Arc<H3Pass>>,
    camo: Option<Arc<H3Pass>>,
    shadow_generate: Option<Arc<H3Pass>>,
    dynamic_light: Option<Arc<H3Pass>>,
    prt: [Option<Arc<H3Pass>>; 3],
}

/// The method's definition read, and each entry point it draws through
/// compiled for its options.
fn compile_passes(
    reader: &TagReader,
    method: &RenderMethod,
    caches: &mut ResolveCaches,
) -> Result<CompiledPasses, String> {
    let sources = caches.h3_sources.get_or_insert_with(|| H3Sources::new(&reader.source));
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
    let single_pass_entries = (dialect == RenderMethodDialect::Reach && !opaque && has("single_pass_single_probe"))
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
        RenderMethodDialect::Reach => camo.and_then(|pass| super::super::reach::camo::camo_pass(&pass)).map(Arc::new),
        RenderMethodDialect::Halo3 => camo,
    };
    let shadow_generate = (dialect == RenderMethodDialect::Reach && opaque && has("shadow_generate"))
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
    Ok(CompiledPasses {
        dialect,
        blend,
        decal,
        decal_modulates_specular: decal && definition.decal_modulates_specular(&options),
        after_lighting: definition.after_lighting(&options),
        screen: definition.name == "screen",
        lit,
        albedo,
        camo,
        shadow_generate,
        dynamic_light,
        prt,
    })
}

/// The parameters whose bitmaps the screen supplies.
fn screen_maps(method: &RenderMethod) -> Vec<String> {
    method
        .parameters
        .iter()
        .filter(|parameter| parameter.bitmap_extern_mode == Some(RenderMethodBitmapExternMode::Screen))
        .map(|parameter| parameter.parameter_name.clone())
        .collect()
}

/// Every pass the material draws through, the lit pass last.
fn material_passes(material: &H3Material) -> Vec<Arc<H3Pass>> {
    material
        .albedo
        .iter()
        .chain(&material.camo)
        .chain(&material.shadow_generate)
        .chain(&material.dynamic_light)
        .chain(material.prt.iter().flatten())
        .cloned()
        .chain([Arc::clone(&material.lit)])
        .collect()
}

/// Which clock the material's animated parameters run by.
fn animation_clock(reader: &TagReader, screen: bool) -> H3Clock {
    match (reader.source.game(), screen) {
        (Some(GameId::Halo3Odst), true) => H3Clock::OdstScreen,
        (Some(GameId::Halo3Odst), false) => H3Clock::Odst,
        (Some(GameId::HaloReach), _) => H3Clock::Reach,
        _ => H3Clock::Halo3,
    }
}

/// Each animated parameter that moves, as the uniform it writes.
fn parameter_animations(method: &RenderMethod, clock: H3Clock) -> Vec<H3Animation> {
    let mut animations = Vec::new();
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
            animations.push(H3Animation {
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
    animations
}

/// The values `pass`'s user parameters take: an extern's change color or
/// debug tint, a bitmap's transform, or the parameter's own value.
fn bind_pass_values(values: &Values<'_>, pass: &H3Pass, material: &mut H3Material) {
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
}

/// The bitmaps `pass`'s user textures bind, each once, with an animated one's
/// frames.
fn bind_pass_textures(
    reader: &TagReader,
    values: &Values<'_>,
    pass: &H3Pass,
    material: &mut H3Material,
    caches: &mut ResolveCaches,
) {
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
        let Some((image, texels, mips)) = h3_image(reader, &path, index, kind, caches) else {
            continue;
        };
        let animated = material.animations.iter().any(|animation| {
            animation.writes == H3Animated::Frame && animation.uniform == *uniform
        });
        let frames = if animated {
            let tag = reader.load(&path, "bitmap", b"bitm").ok();
            let count = tag.as_ref().map_or(0, |tag| elements(tag.root(), "bitmaps*").len());
            (0..count)
                .filter_map(|at| h3_image(reader, &path, at as i16, kind, caches))
                .map(|(image, _, _)| image)
                .collect()
        } else {
            Vec::new()
        };
        material.textures.push(H3Texture { uniform: uniform.clone(), image, texels, mips, sampler, frames });
    }
}

/// Reach's cube maps are the rasterizer's own, not a shader's externs. With no
/// level, `render_method_submit_dynamic_cubemap` (MCC `sub_140817F70`) binds
/// `rasterizer_globals`' default bitmap 2 to both, clamped, trilinear.
fn reach_environment_maps(reader: &TagReader, pass: &H3Pass, material: &mut H3Material, caches: &mut ResolveCaches) {
    const DEFAULT_CUBE: &str = "shaders\\default_bitmaps\\bitmaps\\default_dynamic_cube_map";
    for (uniform, _) in &pass.textures {
        let wanted = matches!(
            uniform.as_str(),
            "GlobalTexture_dynamic_environment_map_0" | "GlobalTexture_dynamic_environment_map_1"
        );
        if !wanted || material.textures.iter().any(|known| known.uniform == *uniform) {
            continue;
        }
        let Some((image, texels, mips)) = h3_image(reader, DEFAULT_CUBE, 0, SamplerKind::Cube, caches) else {
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
pub(in crate::app::model_preview) fn read_h3_shield_settings(reader: &TagReader, path: &str) -> Option<Arc<H3ShieldSettings>> {
    let tag = reader.load(path, "shield_impact", b"shit").ok()?;
    let root = tag.root();
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let tinted = |color: &str, intensity: &str| {
        let (color, intensity) = (root.read_rgb(color), real(intensity));
        H3Value::Floats([color.red * intensity, color.green * intensity, color.blue * intensity, 0.0])
    };
    let mut values = super::super::gen3::shaders::H3Values::new();
    let mut set = |name: &str, value: H3Value| {
        values.insert(name.to_owned(), value);
    };
    set("extrusion_distance", H3Value::Floats([real("Extrusion Distance"); 4]));
    // `render_object_shield_impact`: z is ODST's, for the second noise.
    let secondary = real(if has_flag(root, "flags", "use secondary (non-uniform) texture scale") { "Secondary texture scale" } else { "Texture Scale" });
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
        let image = Arc::new(decode_h2_image(reader, &bitmap, SamplerKind::Texture2D, 0)?);
        textures.push(H3Texture {
            uniform: uniform.to_owned(),
            image,
            texels: H3Texels::Linear,
            mips: Arc::default(),
            sampler: None,
            frames: Vec::new(),
        });
    }
    Some(Arc::new(H3ShieldSettings { procedural: has_flag(root, "flags", "use procedural uv generation"), values, textures }))
}

/// The pass, on the global settings a model that names none of its own uses.
fn h3_shield(reader: &TagReader, caches: &mut ResolveCaches) -> Option<Arc<H3Shield>> {
    if let Some(found) = &caches.h3_shield {
        return found.clone();
    }
    let settings = read_h3_shield_settings(reader, "globals\\global_shield_impact_settings");
    let sources = caches.h3_sources.get_or_insert_with(|| H3Sources::new(&reader.source));
    let shield = settings.and_then(|settings| {
        let pass = sources.explicit_pass("shield_impact").ok()?;
        let procedural = sources.explicit_pass("shield_impact_procedural_uv").ok();
        Some(Arc::new(H3Shield { pass, procedural, settings }))
    });
    caches.h3_shield = Some(shield.clone());
    shield
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::harness::kit::kit_source;
    use crate::core::game::GameId;

    /// With `BLAM_TEST_HREK`: `color_black`, the change-color map Emile's flair
    /// names, is `a8` and samples black, so `two_change_color` leaves it
    /// untinted; `gray_50_percent`, an `a8r8g8b8`, keeps its gray.
    #[test]
    fn kit_an_a8_bitmap_samples_black_with_its_alpha() {
        let Some(source) = kit_source(GameId::HaloReach) else { return };
        let first_texel = |path: &str| {
            let (image, _, _) = read_h3_bitmap(&crate::app::model_preview::load::TagReader::new(&source), path, 0, SamplerKind::Texture2D).expect(path);
            let H2Image::Flat(flat) = &*image else {
                panic!("{path} should be flat");
            };
            [flat.rgba[0], flat.rgba[1], flat.rgba[2], flat.rgba[3]]
        };
        assert_eq!(first_texel("shaders\\default_bitmaps\\bitmaps\\color_black"), [0, 0, 0, 255]);
        assert_eq!(first_texel("shaders\\default_bitmaps\\bitmaps\\gray_50_percent")[..3], [128, 128, 128]);
    }
}
