//! Resolving a render method: Reach's references and version update, embedded methods and their maps.

use super::*;

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
) -> Option<(H3Bitmap, Sampler)> {
    let ParameterSource::Inline(ResolvedValue::Bitmap(binding)) = &resolved.find(name)?.source else {
        return None;
    };
    let bitmap = read_h3_bitmap(reader, &binding.bitmap_path, binding.bitmap_index, SamplerKind::Texture2D)?;
    let sampler = Sampler {
        address: [binding.address_mode_x, binding.address_mode_y],
        filter: binding.filter_mode,
    };
    Some((bitmap, sampler))
}

/// Reach's cube maps are the rasterizer's own, not a shader's externs. With no
/// level, `render_method_submit_dynamic_cubemap` (MCC `sub_140817F70`) binds
/// `rasterizer_globals`' default bitmap 2 to both, clamped, trilinear.
pub(super) fn reach_environment_maps(reader: &TagReader, pass: &H3Pass, material: &mut H3Material, caches: &mut ResolveCaches) {
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
        let sampler = Sampler { address: [BitmapAddressMode::Clamp; 2], filter: BitmapFilterMode::Trilinear };
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
