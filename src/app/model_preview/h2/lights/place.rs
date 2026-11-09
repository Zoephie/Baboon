//! Placing Halo 2's flares and lights on an object's markers, and the function values that scale them.

use super::*;

/// `sub_946960`'s divisor for the near plane's edge, `dword_EB9D38`.
pub(super) const NEAR_EDGE_DIVISOR: f32 = 40.0;

/// What `sub_946960` leaves between the apex and the near plane's edge.
pub(super) const NEAR_EDGE_BIAS: f32 = 0.0078125;

/// The bounds tool's postprocess holds `field of view` to, in degrees.
pub(super) const FIELD_OF_VIEW: [f32; 2] = [1.0, 120.0];

/// What the brightness of `sub_8ED2A0` comes to, at most.
pub(super) const BRIGHTEST: f32 = 4.0;

/// `occlusion inner radius scale`.
pub(super) const INNER_SCALES: [f32; 7] = [1.0, 0.5, 0.25, 0.125, 0.0625, 0.03125, 0.015625];

/// The second of two fields that share a name, by its place in the struct.
pub(super) fn second_field_int(root: TagStruct<'_>, name: &str) -> Option<i128> {
    let field = root.fields().filter(|field| field.name() == name).nth(1)?;
    root.read_int_any(&format!("{name}#{}", field.ordinal()))
}

/// What a light's colors and size interpolate by: its attachment's function,
/// or for an effect's light, its clock. None while that light does not exist.
pub(super) fn light_value(
    light: &H2Light,
    effect: Option<&H2EffectLight>,
    scale_by: &str,
    values: Option<&FunctionValues>,
    time: f32,
    onset: f32,
) -> Option<f32> {
    let on = attachment_value(scale_by, values);
    match effect {
        Some(effect) => effect.value(light, on, time, onset),
        None => Some(on),
    }
}

/// What an attachment's `scale by` reads: full with none named, and the
/// idle object's input for a name nothing gave a value.
pub(in crate::app::model_preview) fn attachment_value(
    name: &str,
    values: Option<&FunctionValues>,
) -> f32 {
    if name.is_empty() {
        return 1.0;
    }
    let known = values.and_then(|values| values.get(name)).copied();
    known.unwrap_or_else(|| object_input(name))
}

pub(super) fn scale_function(entry: TagStruct<'_>, block: &str) -> Option<Arc<TagFunction>> {
    let element = *elements(entry, block).first()?;
    tag_function_at(&element, "function").map(Arc::new)
}

/// Where one light goes, and what drives it.
struct H2LightPlace {
    place: MarkerFrame,
    node: usize,
    marker_scale: f32,
    only_under: Option<(String, String)>,
    scale_by: String,
    change_color: Option<usize>,
    effect: Option<H2EffectLight>,
}

/// What a light puts on the object: a sphere's light, a frustum's gel pass,
/// and the flare `light_submit_lens_flares` submits for it.
fn place_h2_light(
    reader: &TagReader,
    path: &str,
    owner: u8,
    at: H2LightPlace,
    found: &mut (Vec<H2FlareAt>, Vec<H2PointLight>, Vec<H2GelLight>),
) {
    let Some((light, lights_model, frustum, flare)) = read_h2_light(reader, path) else {
        return;
    };
    let (place, node) = (at.place, at.node);
    if let Some(frustum) = frustum {
        found.2.push(H2GelLight {
            object: owner,
            node,
            position: place.position,
            forward: place.axes[0],
            up: place.axes[2],
            scale_by: at.scale_by.clone(),
            light: light.clone(),
            frustum,
            effect: at.effect.clone(),
            only_under: at.only_under.clone(),
        });
    }
    if lights_model {
        found.1.push(H2PointLight {
            object: owner,
            node,
            position: place.position,
            scale_by: at.scale_by.clone(),
            light: light.clone(),
            effect: at.effect.clone(),
            only_under: at.only_under.clone(),
        });
    }
    // A preview is a third person view.
    let Some(flare) = flare.filter(|flare| !flare.flags.only_first_person) else {
        return;
    };
    found.0.push(H2FlareAt {
        object: owner,
        node,
        position: place.position,
        forward: place.axes[0],
        marker_scale: at.marker_scale,
        only_under: at.only_under,
        scale_by: at.scale_by,
        change_color: at.change_color,
        light: Some(light),
        effect: at.effect.map(H2FlareEffect::Light),
        flare,
    });
}

/// `render_submit_object_attachments`' `lens_flare`s, at every marker of their
/// names, and `light_submit_lens_flares`' lights, which take the first. Then
/// the lights its effects' events make, `effect_generate_part`'s only light,
/// and their `lens_flare` parts, which `sub_515280` submits at every marker
/// of their location: scale 1, tinted by the effect's color (its object's
/// change color, or white).
pub(in crate::app::model_preview) fn read_h2_flares(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> (Vec<H2FlareAt>, Vec<H2PointLight>, Vec<H2GelLight>) {
    let mut found = (Vec::new(), Vec::new(), Vec::new());
    let lens_group = u32::from_be_bytes(*b"lens");
    let light_group = u32::from_be_bytes(*b"ligh");
    let attachments =
        object_block(object, "attachments").map(|block| elements(block, "attachments"));
    for attachment in attachments.unwrap_or_default() {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let name = attachment.read_string_id("marker").unwrap_or_default();
        let scale_by = attachment
            .read_string_id("primary scale")
            .unwrap_or_default();
        let change_color = function_slot_of(attachment, "change color");
        let mut named = markers
            .iter()
            .filter(|marker| marker.name.eq_ignore_ascii_case(&name));
        if group == light_group {
            let Some(marker) = named.next() else {
                continue;
            };
            let (place, node) = frame(marker);
            let at = H2LightPlace {
                place,
                node,
                marker_scale: marker.scale,
                only_under: marker.only_under.clone(),
                scale_by,
                change_color,
                effect: None,
            };
            place_h2_light(reader, &path, owner, at, &mut found);
            continue;
        }
        if group != lens_group {
            continue;
        }
        let Some(flare) = read_h2_flare(reader, &path).map(Arc::new) else {
            continue;
        };
        if flare.flags.only_first_person {
            continue;
        }
        for marker in named {
            let (place, node) = frame(marker);
            found.0.push(H2FlareAt {
                object: owner,
                node,
                position: place.position,
                forward: place.axes[0],
                marker_scale: marker.scale,
                only_under: marker.only_under.clone(),
                scale_by: scale_by.clone(),
                change_color,
                light: None,
                effect: None,
                flare: Arc::clone(&flare),
            });
        }
    }
    for_each_attached_part(reader, object, |part| {
        if part.group == lens_group {
            let third_person = |flare: &H2Flare| !flare.flags.only_first_person;
            let Some(flare) = read_h2_flare(reader, &part.path).filter(third_person) else {
                return;
            };
            let flare = Arc::new(flare);
            let change_color = part.effect.change_color;
            for marker in part_markers(markers, &part.marker, part.effect.once) {
                let (place, node) = frame(&marker);
                let (place, node) = part.placed(place, node);
                found.0.push(H2FlareAt {
                    object: owner,
                    node,
                    position: place.position,
                    forward: place.axes[0],
                    marker_scale: 1.0,
                    only_under: marker.only_under.clone(),
                    scale_by: part.effect.scale_by.clone(),
                    change_color,
                    light: None,
                    effect: Some(H2FlareEffect::Part(H2EffectPart::new(&part))),
                    flare: Arc::clone(&flare),
                });
            }
            return;
        }
        if part.group != light_group {
            return;
        }
        // `sub_8EBD10` takes the location's node, or none if not attached.
        let Some(marker) = part_markers(markers, &part.marker, part.effect.once)
            .into_iter()
            .next()
        else {
            return;
        };
        let (place, node) = frame(&marker);
        let (place, node) = part.placed(place, node);
        let at = H2LightPlace {
            place,
            node,
            marker_scale: 1.0,
            only_under: marker.only_under.clone(),
            scale_by: part.effect.scale_by.clone(),
            change_color: None,
            effect: Some(H2EffectLight::new(
                Arc::clone(&part.times),
                part.event,
                part.scaled,
            )),
        };
        place_h2_light(reader, &part.path, owner, at, &mut found);
    });
    found
}
