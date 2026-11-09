//! A material prepared for draw: its passes compiled, their values, textures and animations bound.

use super::*;

/// The first of these a family has is what lights its objects.
const H3_LIT_ENTRIES: [&str; 3] = ["static_sh", "static_per_vertex", "default"];

pub(in crate::app::model_preview) fn prepare_h3_material(
    reader: &TagReader,
    values: &Values<'_>,
    caches: &mut ResolveCaches,
) -> Result<Gen3Material, String> {
    let method = values.method;
    let compiled = compile_passes(reader, method, caches)?;
    let mut material = Gen3Material {
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
    if compiled.blend == Gen3Blend::Opaque {
        material.shield = gen3_shield(reader, caches);
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
    blend: Gen3Blend,
    decal: bool,
    decal_modulates_specular: bool,
    after_lighting: bool,
    /// Whether the definition is `screen`'s, which its clock follows.
    screen: bool,
    lit: Arc<Gen3Pass>,
    albedo: Option<Arc<Gen3Pass>>,
    camo: Option<Arc<Gen3Pass>>,
    shadow_generate: Option<Arc<Gen3Pass>>,
    dynamic_light: Option<Arc<Gen3Pass>>,
    prt: [Option<Arc<Gen3Pass>>; 3],
}

/// The method's definition read, and each entry point it draws through
/// compiled for its options.
fn compile_passes(
    reader: &TagReader,
    method: &RenderMethod,
    caches: &mut ResolveCaches,
) -> Result<CompiledPasses, String> {
    let sources = caches
        .h3_sources
        .get_or_insert_with(|| Gen3Sources::new(&reader.source));
    let definition = sources
        .definition(&method.definition_path)
        .ok_or_else(|| format!("{} did not read", method.definition_path))?;
    let options: Vec<usize> = method
        .options
        .iter()
        .map(|option| (*option).max(0) as usize)
        .collect();
    let glass = method.group_tag == u32::from_be_bytes(*b"rmgl");
    let blend = definition.blend_of(&options, glass);
    // ODST's `black` has only `albedo`, which is all it needs to be black.
    let lit_entry = H3_LIT_ENTRIES
        .iter()
        .chain(&["albedo"])
        .find(|entry| definition.entry_points.iter().any(|known| known == *entry))
        .ok_or("no lit entry point")?;
    let opaque = blend == Gen3Blend::Opaque;
    let dialect = sources.dialect();
    let has = |entry: &str| definition.entry_points.iter().any(|known| known == entry);
    // Reach's `select_mesh_entry_point_for_pass` (MCC `sub_14084C910`) lights
    // what blends in a single pass, by the mesh's policy: one probe, or the
    // ambient transfer.
    let single_pass_entries =
        (dialect == RenderMethodDialect::Reach && !opaque && has("single_pass_single_probe"))
            .then_some((
                "single_pass_single_probe",
                "single_pass_single_probe_ambient",
            ));
    let lit_entry = single_pass_entries
        .as_ref()
        .map_or(lit_entry, |(entry, _)| entry);
    // `render_method_update_postprocess`: the single pass flag counts as blended.
    let single_pass = method
        .postprocess_definition
        .as_ref()
        .is_some_and(|baked| baked.flags & 1 != 0);
    let own_albedo = !opaque || single_pass;
    // `render_albedo_decals`: into the albedo target, to be lit with what's under.
    let decal = definition.is_decal();
    let lit_entry = if decal { &"default" } else { lit_entry };
    let lit = if definition.name == "screen" {
        sources.screen_pass(&definition, &options)?
    } else {
        sources.pass(&definition, &options, lit_entry, own_albedo)?
    };
    let albedo = (opaque
        && definition
            .entry_points
            .iter()
            .any(|entry| entry == "albedo"))
    .then(|| sources.pass(&definition, &options, "albedo", false))
    .transpose()?;
    let camo = (definition
        .entry_points
        .iter()
        .any(|entry| entry == "active_camo"))
    .then(|| sources.pass(&definition, &options, "active_camo", false))
    .transpose()?;
    // Reach's writes back what it samples, which the preview's copy holds
    // encoded.
    let camo = match dialect {
        RenderMethodDialect::Reach => camo
            .and_then(|pass| super::super::super::reach::camo::camo_pass(&pass))
            .map(Arc::new),
        RenderMethodDialect::Halo3 => camo,
    };
    let shadow_generate =
        (dialect == RenderMethodDialect::Reach && opaque && has("shadow_generate"))
            .then(|| sources.pass(&definition, &options, "shadow_generate", false))
            .transpose()?;
    let dynamic_light = (opaque
        && definition
            .entry_points
            .iter()
            .any(|entry| entry == "dynamic_light"))
    .then(|| sources.pass(&definition, &options, "dynamic_light", false))
    .transpose()?;
    // `render_mesh_part_default`: a mesh's `PRT vertex type` picks one.
    let mut prt = [None, None, None];
    for (slot, entry) in prt.iter_mut().zip([
        "static_prt_ambient",
        "static_prt_linear",
        "static_prt_quadratic",
    ]) {
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
        .filter(|parameter| {
            parameter.bitmap_extern_mode == Some(RenderMethodBitmapExternMode::Screen)
        })
        .map(|parameter| parameter.parameter_name.clone())
        .collect()
}

/// Every pass the material draws through, the lit pass last.
fn material_passes(material: &Gen3Material) -> Vec<Arc<Gen3Pass>> {
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
fn animation_clock(reader: &TagReader, screen: bool) -> Gen3Clock {
    match (reader.source.game(), screen) {
        (Some(GameId::Halo3Odst), true) => Gen3Clock::OdstScreen,
        (Some(GameId::Halo3Odst), false) => Gen3Clock::Odst,
        (Some(GameId::HaloReach), _) => Gen3Clock::Reach,
        _ => Gen3Clock::Halo3,
    }
}

/// Each animated parameter that moves, as the uniform it writes.
fn parameter_animations(method: &RenderMethod, clock: Gen3Clock) -> Vec<Gen3Animation> {
    let mut animations = Vec::new();
    for parameter in &method.parameters {
        let uniform = match parameter.parameter_type.map(|kind| kind.get()) {
            Some(RenderMethodParameterType::Bitmap) => {
                format!("UserParameter_{}_xform", parameter.parameter_name)
            }
            _ => format!("UserParameter_{}", parameter.parameter_name),
        };
        for animated in &parameter.animated_parameters {
            let (Some(kind), Some(function)) =
                (animated.parameter_type, animated.function.as_ref())
            else {
                continue;
            };
            let writes = match kind.get() {
                RenderMethodAnimatedParameterType::Color => Gen3Animated::Color,
                RenderMethodAnimatedParameterType::ScaleUniform => Gen3Animated::ScaleUniform,
                RenderMethodAnimatedParameterType::Value
                | RenderMethodAnimatedParameterType::ScaleX => Gen3Animated::First,
                RenderMethodAnimatedParameterType::ScaleY => Gen3Animated::Second,
                RenderMethodAnimatedParameterType::TranslationX => Gen3Animated::Third,
                RenderMethodAnimatedParameterType::TranslationY
                | RenderMethodAnimatedParameterType::Alpha => Gen3Animated::Fourth,
                RenderMethodAnimatedParameterType::FrameIndex => Gen3Animated::Frame,
            };
            if function.as_constant().is_some() && animated.input_name.is_empty() {
                continue;
            }
            let uniform = match writes {
                Gen3Animated::Frame => format!("UserParameterTexture_{}", parameter.parameter_name),
                _ => uniform.clone(),
            };
            animations.push(Gen3Animation {
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
fn bind_pass_values(values: &Values<'_>, pass: &Gen3Pass, material: &mut Gen3Material) {
    for (uniform, _) in &pass.uniforms {
        let Some(name) = uniform.strip_prefix("UserParameter_") else {
            continue;
        };
        if let Some(ParameterSource::Extern(source)) = values
            .resolved
            .and_then(|resolved| resolved.find(name))
            .map(|found| &found.source)
        {
            let source: &'static str = source.into();
            if let Some(channel) = gen3_extern_change_color(source) {
                material.change_colors.push((uniform.clone(), channel));
            } else if source == "debug tint" {
                material
                    .values
                    .insert(uniform.clone(), Gen3Value::Floats([1.0, 1.0, 1.0, 0.0]));
            }
            continue;
        }
        let value = match name.strip_suffix("_xform") {
            Some(map) => Some(Gen3Value::Floats(gen3_xform(values, map))),
            None => gen3_parameter_value(values, name),
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
    pass: &Gen3Pass,
    material: &mut Gen3Material,
    caches: &mut ResolveCaches,
) {
    for (uniform, kind) in &pass.textures {
        let Some(name) = uniform.strip_prefix("UserParameterTexture_") else {
            continue;
        };
        if material
            .textures
            .iter()
            .any(|known| known.uniform == *uniform)
        {
            continue;
        }
        let found = values.resolved.and_then(|resolved| resolved.find(name));
        let (path, index, sampler) = match found.map(|found| &found.source) {
            // With no render target live behind an extern mode, as here,
            // the bitmap binds: `render_method_submit_volatile_per_shader`.
            Some(ParameterSource::Inline(ResolvedValue::Bitmap(binding))) => (
                binding.bitmap_path.clone(),
                binding.bitmap_index,
                Some(Sampler {
                    address: [binding.address_mode_x, binding.address_mode_y],
                    filter: binding.filter_mode,
                }),
            ),
            Some(ParameterSource::Extern(extern_source)) => {
                match gen3_extern_bitmap(extern_source.into()) {
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
        let Some((image, texels, mips)) = gen3_image(reader, &path, index, kind, caches) else {
            continue;
        };
        let animated = material.animations.iter().any(|animation| {
            animation.writes == Gen3Animated::Frame && animation.uniform == *uniform
        });
        let frames = if animated {
            let tag = reader.load(&path, "bitmap", b"bitm").ok();
            let count = tag
                .as_ref()
                .map_or(0, |tag| elements(tag.root(), "bitmaps*").len());
            (0..count)
                .filter_map(|at| gen3_image(reader, &path, at as i16, kind, caches))
                .map(|(image, _, _)| image)
                .collect()
        } else {
            Vec::new()
        };
        material.textures.push(Gen3Texture {
            uniform: uniform.clone(),
            image,
            texels,
            mips,
            sampler,
            frames,
        });
    }
}
