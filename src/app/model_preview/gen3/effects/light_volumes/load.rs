//! Reading a light volume definition, and the volumes on an object's markers.

use super::*;

/// What a volume's row can hold, whatever its tag says; also the most
/// Reach's postprocess bakes.
pub(super) const MOST_PROFILES: i32 = 128;

/// Reach's postprocess (tool 0x140DD0FA0): an `LOD out distance` of none
/// is this far.
pub(super) const DEFAULT_LOD_OUT: f32 = 20.0;

/// One of a `light_volume_system`'s `light_volumes`.
#[derive(Debug)]
pub(crate) struct H3VolumeDefinition {
    pub(super) length: H3VolumeProperty,
    pub(super) offset: H3VolumeProperty,
    pub(super) density: H3VolumeProperty,
    pub(super) profile_length: H3VolumeProperty,
    pub(in crate::app::model_preview) thickness: H3VolumeProperty,
    pub(in crate::app::model_preview) color: H3VolumeProperty,
    pub(in crate::app::model_preview) alpha: H3VolumeProperty,
    pub(in crate::app::model_preview) intensity: H3VolumeProperty,
    pub(in crate::app::model_preview) brightness_ratio: f32,
    pub(super) blend: H3Blend,
    /// None for Reach's `circular`, which samples no map.
    pub(super) bitmap: Option<H3Bitmap>,
    pub(super) sampler: Option<Sampler>,
    pub(super) sort_layer: i32,
    /// Reach's: its own colour, exposure and level of detail.
    pub(super) reach: bool,
    /// Reach's `albedo` `circular`: `center_offset` and `falloff`.
    pub(super) circular: Option<[f32; 2]>,
    /// Reach's `depth_fade` `on`: `depth_fade_range`.
    pub(super) depth_fade: Option<f32>,
    pub(super) lod: Option<ReachVolumeLod>,
    /// Reach's precompiled rows (`s_light_volume_pre_vertex`), at
    /// `max profile count`: each profile's colour times its intensity
    /// squared times its alpha, and its thickness.
    pub(super) precompiled: Option<Vec<([f32; 3], f32)>>,
    /// Halo 4's: the `actual material` it draws through, and its
    /// `appearance flags`.
    pub(in crate::app::model_preview) h4: Option<(H4EffectMaterial, u32)>,
}

impl H3VolumeDefinition {
    /// The baked rows: profile `i` at `i / (max − 0.5)` with every other
    /// state 0 (a fresh `c_light_volume_states`), unclamped.
    pub(super) fn bake(&self, most: usize) -> Vec<([f32; 3], f32)> {
        let place = |index: usize| index as f32 / ((most as f32 + 0.5) - 1.0);
        (0..most)
            .map(|index| {
                let state = H3VolumeState { percentile: place(index), ..Default::default() };
                let intensity = self.intensity.scalar(&state);
                let factor = intensity * intensity * self.alpha.scalar(&state);
                (self.color.color(&state).map(|channel| channel * factor), self.thickness.scalar(&state))
            })
            .collect()
    }

    /// `light_volume.hlsl`'s profile colour, before the map: Halo 3's colour
    /// at its exposure with alpha unscaled by intensity; Reach's
    /// (`lights__light_volume.hlsl`) with alpha by intensity too and its
    /// colour at `V_ILLUM_EXPOSURE` unless it multiplies, or a baked row's,
    /// which carries both and an alpha and intensity of 1. Alpha goes by the
    /// brightness `ratio` over the `overdraw`.
    pub(super) fn profile_color(
        &self,
        state: &H3VolumeState,
        baked: Option<[f32; 3]>,
        [ratio, overdraw]: [f32; 2],
        stops: f32,
    ) -> [f32; 4] {
        if !self.reach {
            // `light_volume_fx.hlsl`'s `STATE.m_color * STATE.m_intensity` is a
            // float4, so intensity scales alpha too (Halo 3 and ODST alike).
            let color = self.color.color(state).map(|channel| channel.clamp(0.0, 1.0));
            let intensity = self.intensity.scalar(state);
            let [red, green, blue] = color.map(|channel| channel * intensity * H3_EXPOSURE);
            let alpha = self.alpha.scalar(state).clamp(0.0, 1.0) * intensity;
            return [red, green, blue, alpha * ratio / overdraw];
        }
        let exposure = if self.blend == H3Blend::Multiply { 1.0 } else { reach_alt_exposure(stops)[1] };
        // TODO(reach-effects spec §3.4): Reach premultiplies and writes an
        // alpha of 1, which the flare program's `(rgb, a)` under SRC_ALPHA,
        // ONE matches; an alpha-blended volume (none in the kit) would not.
        let ([red, green, blue], alpha) = match baked {
            Some(baked) => (baked, 1.0),
            None => {
                let intensity = self.intensity.scalar(state);
                let color = self.color.color(state).map(|channel| channel.clamp(0.0, 1.0) * intensity);
                (color, self.alpha.scalar(state).clamp(0.0, 1.0) * intensity)
            }
        };
        [red * exposure, green * exposure, blue * exposure, alpha * ratio / overdraw]
    }
}

/// A volume at a marker of the object it is attached to.
#[derive(Debug, Clone)]
pub(crate) struct H3LightVolumeAt {
    pub(crate) object: u8,
    pub(super) node: usize,
    pub(super) position: [f32; 3],
    pub(super) forward: [f32; 3],
    /// `use parent position but world orientation`: its forward does not turn.
    pub(super) world_oriented: bool,
    pub(super) only_under: Option<(String, String)>,
    pub(super) scale_by: String,
    /// `secondary scale`, the effect's `b`.
    pub(super) secondary_by: String,
    pub(super) seeds: [f32; SEEDS],
    pub(super) times: Arc<EffectTimes>,
    pub(super) event: usize,
    pub(crate) definition: Arc<H3VolumeDefinition>,
}

pub(super) fn read_definition(reader: &TagReader, element: TagStruct<'_>) -> Option<H3VolumeDefinition> {
    if reader.source.game() == Some(GameId::Halo4) {
        return read_h4_definition(reader, element);
    }
    let reach = reader.source.game() == Some(GameId::HaloReach);
    let shader = element.descend("actual shader?")?;
    let definition = shader.read_tag_ref_path("definition").filter(|path| !path.is_empty())?;
    let definition = read_h3_definition(reader, &definition)?;
    let options: Vec<usize> = elements(shader, "options")
        .iter()
        .map(|option| option.read_int_any("short").unwrap_or(0).max(0) as usize)
        .collect();
    let chosen = |category: &str| definition.chosen(category, &options).unwrap_or_default().to_owned();
    let resolved = resolve_embedded_method(reader, shader);
    let resolved_real = |name: &str, default: f32| inline_real(resolved.as_ref(), name).unwrap_or(default);
    // `lights__light_volume.hlsl` l.288–296; `albedo_circular.render_method_option`'s defaults.
    let circular = (reach && chosen("albedo") == "circular")
        .then(|| [resolved_real("center_offset", 1.1), resolved_real("falloff", 2.0)]);
    let (bitmap, sampler) = if circular.is_some() {
        (None, None)
    } else {
        let parameters = elements(shader, "parameters");
        let base = parameters.iter().find(|parameter| {
            parameter.read_string_id("parameter name").as_deref() == Some("base_map")
        })?;
        let map = base.read_tag_ref_path("bitmap").filter(|path| !path.is_empty())?;
        let baked = resolved
            .as_ref()
            .and_then(|resolved| super::super::super::super::gen3::materials::read_resolved_map(reader, resolved, "base_map"));
        match baked {
            Some((bitmap, sampler)) => (Some(bitmap), Some(sampler)),
            None => (Some(read_h3_bitmap(reader, &map, 0, SamplerKind::Texture2D)?), None),
        }
    };
    let blend = definition.blend(&options);
    // `depth_fade_on.render_method_option`'s range. `biased`'s option tag is
    // not in the kit, and no volume uses it.
    let depth_fade = (reach && chosen("depth_fade") == "on" && blend != H3Blend::Opaque)
        .then(|| resolved_real("depth_fade_range", 0.1).max(1e-4));
    let flagged = |wanted: &str| has_flag(element, "flags", wanted);
    let lod = if reach { ReachVolumeLod::read(element, flagged) } else { None };
    let properties = VolumeProperties::read(element)?;
    // The postprocess tests what the properties read before it narrows the
    // per-volume four's inputs.
    let most = max_profile_count(&properties.density, &properties.length);
    let precompiled = reach && precompiles(flagged, &properties.all(), most);
    let mut volume = properties.postprocessed(element, |properties| H3VolumeDefinition {
        blend,
        sampler,
        bitmap,
        sort_layer: super::super::super::super::particles::h3_sort_layer(shader),
        reach,
        circular,
        depth_fade,
        lod,
        precompiled: None,
        h4: None,
        ..properties
    });
    if precompiled {
        volume.precompiled = Some(volume.bake(most));
    }
    Some(volume)
}

/// Halo 4's `c_light_volume_definition::postprocess_block_proc` (MCC
/// 0x1403ABF50): Reach's, through the `actual material`, and never
/// precompiled, which MCC clears every time and could not draw ("not
/// implemented for D3D11").
fn read_h4_definition(reader: &TagReader, element: TagStruct<'_>) -> Option<H3VolumeDefinition> {
    let material = blam_tags::material::Material::from_struct(&element.descend("actual material?")?);
    let resolved = resolve_h4_effect_material(reader, &material, "light_volume")
        .map_err(|error| eprintln!("h4 light volume material: {error}"))
        .ok()?;
    let flagged = |wanted: &str| has_flag(element, "flags", wanted);
    let lod = ReachVolumeLod::read(element, flagged);
    let appearance = element.read_int_any("appearance flags").unwrap_or(0) as u32;
    let properties = VolumeProperties::read(element)?;
    Some(properties.postprocessed(element, |properties| H3VolumeDefinition {
        // The material blends; this only sorts.
        blend: H3Blend::Additive,
        sampler: None,
        bitmap: None,
        sort_layer: i32::from(material.sort_layer).clamp(1, 3),
        reach: true,
        circular: None,
        depth_fade: None,
        lod,
        precompiled: None,
        h4: Some((resolved, appearance)),
        ..properties
    }))
}

/// The volumes an object's attached effects run, one at each marker of a
/// location.
pub(in crate::app::model_preview) fn read_h3_light_volumes(
    reader: &TagReader,
    object: &blam_tags::TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H3LightVolumeAt> {
    let mut volumes = Vec::new();
    let volume_group = u32::from_be_bytes(*b"ltvl");
    for_each_attached_part(reader, object, |part| {
        if part.group != volume_group {
            return;
        }
        let Ok(tag) = reader.load(&part.path, "light_volume_system", b"ltvl") else {
            return;
        };
        let (offset, yaw, pitch) = read_part_placement(part.part);
        for element in elements(tag.root(), "light_volumes") {
            let Some(definition) = read_definition(reader, element).map(Arc::new) else {
                continue;
            };
            for marker in &part.effect.locations(markers, &part.marker, &frame) {
                let (place, node) = frame(marker);
                let (place, node) = part.placed(place, node);
                let place = part_frame(&place, offset, yaw, pitch);
                volumes.push(H3LightVolumeAt {
                    object: owner,
                    node,
                    position: place.position,
                    forward: place.axes[0],
                    world_oriented: part.world_oriented,
                    only_under: marker.only_under.clone(),
                    scale_by: part.effect.scale_by.clone(),
                    secondary_by: part.effect.secondary_by.clone(),
                    seeds: volume_seeds(volumes.len()),
                    times: Arc::clone(&part.times),
                    event: part.event,
                    definition: Arc::clone(&definition),
                });
            }
        }
    });
    volumes
}
