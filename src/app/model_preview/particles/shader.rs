//! The sprite shader that particles, strips, leaves and tracers share, read from its render method, and the sprite vertex.

use super::super::TagReader;
#[cfg(test)]
use super::super::TagSource;
#[cfg(test)]
use super::GameId;
use super::look::{AlbedoConstants, ParticleAlbedo, ParticleLighting};
use super::rules::ParticleRules;
use super::{
    Gen3Bitmap, Gen3Blend, H3_EXPOSURE, Sampler, SamplerKind, dot, elements, reach_alt_exposure,
    reach_exposure, read_h3_bitmap, read_h3_definition, read_resolved_map, resolve_embedded_method,
    tag_function_at,
};
use blam_tags::render_method::{
    BitmapAddressMode, BitmapFilterMode, ParameterSource, ResolvedRenderMethod, ResolvedValue,
};
use blam_tags::{TagFieldData, TagStruct};

/// Unnormalized, as the game's is, and with no special case but what keeps it
/// from dividing by nothing.
pub(in crate::app::model_preview) fn slerped([from, to]: [[f32; 3]; 2], t: f32) -> [f32; 3] {
    let lengths = dot(from, from).sqrt() * dot(to, to).sqrt();
    let lengths = if lengths == 0.0 { 1.0 } else { lengths };
    let mut cos = dot(from, to) / lengths;
    if cos >= 0.9999 {
        cos *= 0.9999;
    }
    let sin = (1.0 - cos * cos).sqrt();
    let angle = cos.acos();
    let (near, far) = (((1.0 - t) * angle).sin() / sin, (angle * t).sin() / sin);
    std::array::from_fn(|axis| from[axis] * near + to[axis] * far)
}

/// A point, a vector, or a radius and angles, whichever a property has.
pub(in crate::app::model_preview) fn read_ends(property: TagStruct<'_>) -> [[f32; 3]; 2] {
    let end = |name: &str| -> [f32; 3] {
        if let Some(angled) = property.descend(name) {
            let angles = angled.read_euler2d("Euler angles");
            let radius = angled.read_real("Radius").unwrap_or(0.0);
            let radius = if radius == 0.0 { 1.0 } else { radius };
            let (sin_yaw, cos_yaw) = angles.yaw.sin_cos();
            let (sin_pitch, cos_pitch) = angles.pitch.sin_cos();
            return [cos_yaw * cos_pitch, sin_yaw * cos_pitch, sin_pitch].map(|axis| axis * radius);
        }
        match property.field(name).and_then(|field| field.value()) {
            Some(TagFieldData::RealPoint3d(point)) => [point.x, point.y, point.z],
            Some(TagFieldData::RealVector3d(vector)) => [vector.i, vector.j, vector.k],
            Some(TagFieldData::RealPoint2d(point)) => [point.x, point.y, 0.0],
            Some(TagFieldData::RealVector2d(vector)) => [vector.i, vector.j, 0.0],
            _ => [0.0; 3],
        }
    };
    [end("Starting interpolant"), end("Ending interpolant")]
}

/// What particles, beams and contrails ask of a shader alike.
#[derive(Debug)]
pub(crate) struct SpriteShader {
    pub(crate) albedo: ParticleAlbedo,
    pub(crate) blend: Gen3Blend,
    pub(crate) black_point: bool,
    pub(crate) frame_blend: bool,
    /// `depth_fade_range`, with the `depth_fade` option on.
    pub(crate) depth_fade: Option<f32>,
    pub(in crate::app::model_preview) add: [f32; 3],
    /// Reach's: `add` is at `V_ILLUM_EXPOSURE` (`default_vs`).
    pub(in crate::app::model_preview) add_at_illum: bool,
    /// Reach's beams: `sample_diffuse` takes a `particle_alpha` of 1
    /// (`effects__beam.hlsl` l.373–458), not the vertex's alpha.
    pub(crate) unit_particle_alpha: bool,
    /// `alpha_test.hlsl`: under a half of alpha it is gone, and otherwise
    /// solid, writing depth like anything opaque.
    pub(crate) alpha_test: bool,
    /// Whether the pixel is lit, as `per_pixel_ravi_order_3` is.
    pub(crate) lit_pixels: bool,
    /// `sort layer`: `transparent_layer_and_z_sort_proc` goes by this first.
    pub(crate) sort_layer: i32,
    /// `distortion_scale`, for what bends the scene instead of covering it.
    pub(crate) distortion: Option<f32>,
    pub(in crate::app::model_preview) lighting: ParticleLighting,
    /// `starting_uv_scale` and `ending_uv_scale`.
    pub(in crate::app::model_preview) uv_scale: [f32; 2],
    pub(crate) base_map: Option<Gen3Bitmap>,
    pub(crate) alpha_map: Option<Gen3Bitmap>,
    pub(crate) palette_map: Option<Gen3Bitmap>,
    /// `base_map2`, which only `palettized_plasma` reads.
    pub(crate) second_map: Option<Gen3Bitmap>,
    /// `base_map`'s, `alpha_map`'s, `palette`'s and `base_map2`'s, as baked.
    pub(crate) samplers: [Option<Sampler>; 4],
    pub(crate) constants: AlbedoConstants,
    /// Reach's `per_pixel_smooth`: `contrast_scale` and `contrast_offset`.
    pub(in crate::app::model_preview) contrast: [f32; 2],
    /// Reach's `warp` `sphere`: `sphere_warp_scale`.
    pub(crate) sphere_warp: Option<f32>,
    /// Reach's `depth_fade` `palette_shift`: `palette_shift_amount`.
    pub(crate) palette_shift: Option<f32>,
    /// Reach's plasmas read `base_map2` at the second sprite
    /// (`particle_render.hlsl`'s `texcoord_sprite1`), not the first.
    pub(crate) plasma_second_sprite: bool,
}

impl SpriteShader {
    /// The constant self-illumination added, at the area screen effects'
    /// exposure `stops` where it is Reach's.
    pub(crate) fn add(&self, stops: f32) -> [f32; 3] {
        let exposure = if self.add_at_illum {
            reach_alt_exposure(stops)[1]
        } else {
            1.0
        };
        self.add.map(|channel| channel * exposure)
    }

    /// Reach's `per_pixel_smooth` contrast, when its pixels are lit so.
    pub(crate) fn smooth_contrast(&self) -> Option<[f32; 2]> {
        (self.lighting == ParticleLighting::ReachSmooth).then_some(self.contrast)
    }

    /// What a leaf draws with: only a map, cut out at half its alpha.
    pub(in crate::app::model_preview) fn plain(blend: Gen3Blend) -> Self {
        Self {
            albedo: ParticleAlbedo::Diffuse,
            blend,
            black_point: false,
            frame_blend: false,
            depth_fade: None,
            add: [0.0; 3],
            add_at_illum: false,
            unit_particle_alpha: false,
            alpha_test: false,
            lit_pixels: false,
            sort_layer: 2,
            distortion: None,
            lighting: ParticleLighting::None,
            uv_scale: [1.0; 2],
            base_map: None,
            alpha_map: None,
            palette_map: None,
            second_map: None,
            samplers: [None; 4],
            constants: AlbedoConstants::default(),
            contrast: [0.0; 2],
            sphere_warp: None,
            palette_shift: None,
            plasma_second_sprite: false,
        }
    }

    pub(crate) fn alpha_tested(blend: Gen3Blend, base_map: Gen3Bitmap) -> Self {
        // `leaf_system_render_opaque` binds its plate clamped and bilinear.
        let plate = Sampler {
            address: [BitmapAddressMode::Clamp; 2],
            filter: BitmapFilterMode::Bilinear,
        };
        Self {
            alpha_test: true,
            base_map: Some(base_map),
            samplers: [Some(plate), None, None, None],
            ..Self::plain(blend)
        }
    }

    #[cfg(test)]
    pub(crate) fn blank() -> Self {
        Self::plain(Gen3Blend::Additive)
    }
}

/// A kit's `tags` folder as the effects tests read it, against the bundled
/// definitions.
#[cfg(test)]
pub(in crate::app::model_preview) fn loose_kit(
    root: impl AsRef<std::path::Path>,
    game: GameId,
) -> TagSource {
    TagSource::LooseFolder {
        root: root.as_ref().into(),
        game: Some(game),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    }
}

/// `sort layer`, by option name: what is unset is `normal`.
pub(in crate::app::model_preview) fn gen3_sort_layer(shader: TagStruct<'_>) -> i32 {
    match shader.read_enum_option("sort layer").as_deref() {
        Some("pre-pass") => 1,
        Some("post-pass") => 3,
        _ => 2,
    }
}

/// A sprite system's `actual shader?`, by its options' names. Reach's options
/// (`shaders\particle.render_method_definition`) read their parameters
/// through the resolver, falling to what their option tags default them to.
pub(in crate::app::model_preview) fn read_sprite_shader(
    reader: &TagReader,
    shader: TagStruct<'_>,
) -> Option<SpriteShader> {
    let definition = shader
        .read_tag_ref_path("definition")
        .filter(|path| !path.is_empty())?;
    let definition = read_h3_definition(reader, &definition)?;
    let options: Vec<usize> = elements(shader, "options")
        .iter()
        .map(|option| option.read_int_any("short").unwrap_or(0).max(0) as usize)
        .collect();
    let chosen = |category: &str| {
        definition
            .chosen(category, &options)
            .unwrap_or_default()
            .to_owned()
    };
    let reach_method = ParticleRules::of(reader.source.game()).reach_render_method;
    let resolved = resolve_embedded_method(reader, shader);
    let resolved_real =
        |name: &str, default: f32| inline_real(resolved.as_ref(), name).unwrap_or(default);
    let add = parameter(shader, "self_illum_color")
        .filter(|_| chosen("self_illumination") == "constant_color")
        .and_then(|parameter| elements(parameter, "animated parameters").first().copied())
        .and_then(|animated| tag_function_at(&animated, "function"))
        .map(|function| function.evaluate_color(0.0, 0.0))
        .map_or([0.0; 3], |color| [color.red, color.green, color.blue]);
    let ([base_map, alpha_map, palette_map, second_map], samplers) = read_maps(reader, shader);
    let depth_fade = chosen("depth_fade");
    // `compute_depth_fade`: Reach's `low_res` is `on` on DirectX 11, its
    // four taps being the 360's alone, and `palette_shift` fades as `on` does.
    let fades = if reach_method {
        matches!(depth_fade.as_str(), "on" | "low_res" | "palette_shift")
    } else {
        depth_fade == "on"
    };
    let albedo = ParticleAlbedo::named(&chosen("albedo"));
    Some(SpriteShader {
        albedo,
        blend: definition.blend(&options),
        black_point: chosen("black_point") == "on",
        frame_blend: chosen("frame_blend") == "on",
        depth_fade: (fades && chosen("blend_mode") != "opaque").then(|| {
            let range = if reach_method {
                resolved_real("depth_fade_range", 0.1)
            } else {
                parameter(shader, "depth_fade_range")
                    .and_then(|parameter| {
                        elements(parameter, "animated parameters").first().copied()
                    })
                    .and_then(|animated| tag_function_at(&animated, "function"))
                    .map_or(1.0, |function| function.evaluate(0.0, 0.0))
            };
            range.max(1e-4)
        }),
        add,
        add_at_illum: false,
        unit_particle_alpha: false,
        alpha_test: false,
        lit_pixels: chosen("lighting") == "per_pixel_ravi_order_3",
        sort_layer: gen3_sort_layer(shader),
        distortion: chosen("specialized_rendering")
            .starts_with("distortion")
            .then(|| {
                parameter(shader, "distortion_scale")
                    .and_then(|parameter| {
                        elements(parameter, "animated parameters").first().copied()
                    })
                    .and_then(|animated| tag_function_at(&animated, "function"))
                    .map_or(1.0, |function| function.evaluate(0.0, 0.0))
            }),
        lighting: match chosen("lighting").as_str() {
            "per_vertex_ravi_order_0" => ParticleLighting::Ambient,
            "per_pixel_ravi_order_3" => ParticleLighting::Directional,
            "per_vertex_ambient" => ParticleLighting::ReachAmbient,
            "per_pixel_smooth" => ParticleLighting::ReachSmooth,
            // `smoke_lighting` draws normals into the low-res transparency
            // buffer, and `lit_particle_overdraw_apply.hlsl` lights that in a
            // pass of its own, whose compositing was not read: it draws unlit.
            _ => ParticleLighting::None,
        },
        uv_scale: ["starting_uv_scale", "ending_uv_scale"].map(|name| {
            parameter(shader, name)
                .and_then(|parameter| elements(parameter, "animated parameters").first().copied())
                .and_then(|animated| tag_function_at(&animated, "function"))
                .map_or(1.0, |function| function.evaluate(0.0, 0.0))
        }),
        base_map,
        alpha_map,
        palette_map,
        second_map,
        samplers,
        constants: read_albedo_constants(shader, resolved.as_ref(), reach_method),
        // `smooth_lighting.render_method_option`'s defaults.
        contrast: [
            resolved_real("contrast_scale", 0.5),
            resolved_real("contrast_offset", 0.5),
        ],
        // `warp_sphere.render_method_option`'s default.
        sphere_warp: (reach_method && chosen("warp") == "sphere")
            .then(|| resolved_real("sphere_warp_scale", 0.41)),
        // `depth_fade_palette_shift.render_method_option`'s default.
        palette_shift: (reach_method && depth_fade == "palette_shift")
            .then(|| resolved_real("palette_shift_amount", 0.5)),
        plasma_second_sprite: reach_method
            && matches!(
                albedo,
                ParticleAlbedo::PalettePlasma | ParticleAlbedo::Palette2dPlasma
            ),
    })
}

/// A resolved parameter's value, when the resolver bakes one in.
fn inline_value(resolved: Option<&ResolvedRenderMethod>, name: &str) -> Option<ResolvedValue> {
    let ParameterSource::Inline(value) = &resolved?.find(name)?.source else {
        return None;
    };
    Some(value.clone())
}

pub(in crate::app::model_preview) fn inline_real(
    resolved: Option<&ResolvedRenderMethod>,
    name: &str,
) -> Option<f32> {
    match inline_value(resolved, name)? {
        ResolvedValue::Real(real) => Some(real),
        _ => None,
    }
}

pub(super) fn parameter<'a>(shader: TagStruct<'a>, name: &str) -> Option<TagStruct<'a>> {
    let parameters = elements(shader, "parameters");
    parameters
        .into_iter()
        .find(|parameter| parameter.read_string_id("parameter name").as_deref() == Some(name))
}

fn read_map(reader: &TagReader, shader: TagStruct<'_>, name: &str) -> Option<Gen3Bitmap> {
    let path = parameter(shader, name)?
        .read_tag_ref_path("bitmap")
        .filter(|path| !path.is_empty())?;
    read_h3_bitmap(reader, &path, 0, SamplerKind::Texture2D)
}

/// A sprite shader's maps, and the sampler each is baked with. Failing to
/// resolve leaves what it authored, on the preview's default sampler.
fn read_maps(
    reader: &TagReader,
    shader: TagStruct<'_>,
) -> ([Option<Gen3Bitmap>; 4], [Option<Sampler>; 4]) {
    const MAPS: [&str; 4] = ["base_map", "alpha_map", "palette", "base_map2"];
    let Some(resolved) = resolve_embedded_method(reader, shader) else {
        return (MAPS.map(|name| read_map(reader, shader, name)), [None; 4]);
    };
    let mut maps = MAPS
        .map(|name| read_resolved_map(reader, &resolved, name))
        .into_iter();
    let (bitmaps, samplers): (Vec<_>, Vec<_>) =
        std::array::from_fn::<_, 4, _>(|_| maps.next().flatten())
            .map(|map| {
                map.map_or((None, None), |(bitmap, sampler)| {
                    (Some(bitmap), Some(sampler))
                })
            })
            .into_iter()
            .unzip();
    let mut bitmaps = bitmaps.into_iter();
    let mut samplers = samplers.into_iter();
    (
        std::array::from_fn(|_| bitmaps.next().flatten()),
        std::array::from_fn(|_| samplers.next().flatten()),
    )
}

/// A map's `_xform`, its scale then its offset, from its animators at rest.
fn sprite_xform(shader: TagStruct<'_>, name: &str) -> [f32; 4] {
    let mut xform = [1.0, 1.0, 0.0, 0.0];
    let Some(found) = parameter(shader, name) else {
        return xform;
    };
    for animated in elements(found, "animated parameters") {
        let Some(function) = tag_function_at(&animated, "function") else {
            continue;
        };
        let value = function.evaluate(0.0, 0.0);
        match animated.read_enum_option("type").as_deref() {
            Some("scale uniform") => xform[..2].fill(value),
            Some("scale x") => xform[0] = value,
            Some("scale y") => xform[1] = value,
            Some("translation x") => xform[2] = value,
            Some("translation y") => xform[3] = value,
            _ => {}
        }
    }
    xform
}

fn read_albedo_constants(
    shader: TagStruct<'_>,
    resolved: Option<&ResolvedRenderMethod>,
    reach_render_method: bool,
) -> AlbedoConstants {
    let real = |name: &str| inline_real(resolved, name).unwrap_or(0.0);
    let tint = match inline_value(resolved, "tint_color") {
        Some(ResolvedValue::Color([_, red, green, blue])) => [red, green, blue],
        _ => [1.0; 3],
    };
    // `albedo_palettized_plasma.render_method_option`'s, which Reach's 2d
    // plasma shares.
    let alpha_modulation = if reach_render_method {
        inline_real(resolved, "alpha_modulation_factor").unwrap_or(0.1)
    } else {
        real("alpha_modulation_factor")
    };
    AlbedoConstants {
        tint,
        modulation: real("modulation_factor"),
        alpha_modulation,
        base_xform: sprite_xform(shader, "base_map"),
        second_xform: sprite_xform(shader, "base_map2"),
    }
}

/// One corner of a quad, as the particle program reads it.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ParticleCorner {
    pub(crate) place: [f32; 3],
    pub(crate) sprite: [f32; 2],
    pub(crate) next_sprite: [f32; 2],
    pub(crate) billboard: [f32; 2],
    pub(crate) color: [f32; 4],
    /// Frame blend, palette row and black point.
    pub(crate) extra: [f32; 3],
    /// What a distortion's map leans along: its tangent and binormal, at scale.
    pub(crate) lean: [[f32; 3]; 2],
    /// `per_pixel_ravi_order_3`'s normal, bent as `surface_basis` bends it.
    pub(crate) normal: [f32; 3],
}

/// `default_vs`' exposure: none for a multiply, nor for Halo 2, whose colours
/// are what shows. Halo 3's at its own; Reach's additives at
/// `V_ILLUM_EXPOSURE` (`v_alt_exposure.y`) and the rest at `v_exposure.x`,
/// at the area screen effects' `stops`. Beams and contrails take the same
/// (`effects__beam.hlsl` and `effects__contrail_render.hlsl`, l.258–269).
pub(in crate::app::model_preview) fn particle_exposure(
    blend: Gen3Blend,
    exposed: bool,
    reach_exposure_stops: bool,
    stops: f32,
) -> f32 {
    if blend == Gen3Blend::Multiply || !exposed {
        1.0
    } else if !reach_exposure_stops {
        H3_EXPOSURE
    } else if matches!(blend, Gen3Blend::Additive | Gen3Blend::AddSrcTimesSrcAlpha) {
        reach_alt_exposure(stops)[1]
    } else {
        reach_exposure(stops)[0]
    }
}

/// `compute_variant`.
pub(super) fn frame_variant(input: f32, count: usize, one_shot: bool, backwards: bool) -> f32 {
    if count <= 1 {
        return 0.0;
    }
    let count = (if one_shot { count - 1 } else { count }) as f32;
    let variant = count * input;
    if backwards { count - variant } else { variant }
}
