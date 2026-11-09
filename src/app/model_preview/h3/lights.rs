//! Halo 3's lights on objects: `objects\lights.cpp` works one out, and
//! `c_lights_view::submit_visibility_and_render` draws it through each
//! shader's `dynamic_light` entry.

use std::sync::Arc;

use blam_tags::{TagFile, TagStruct};

use super::super::common::fields::{
    elements, function_slot_of, has_flag, has_flag_labelled, object_block,
};
use super::super::common::view::posed_on;
use super::super::effects::{
    EffectTimes, for_each_attached_part, marker_name, part_frame, read_part_placement,
};
use super::super::functions::FunctionValues;
use crate::app::model_preview::common::marker::MarkerFrame;

use super::super::common::image::Image;
use super::super::common::named_function::NamedFunction;
use super::super::gen3::effects::flares::{Gen3EffectScale, Gen3Flare, read_h3_flare};
use super::super::gen3::lights::SHADOWS_AND_GELS;
use super::super::{RenderModelPreviewMarker, TagReader, decode_image};
use crate::app::model_preview::common::attached::attached;
use crate::app::model_preview::common::math::{cross, dot};
use crate::core::h2_pixel_shader::SamplerKind;

/// What `add_simple_light_to_draw_list` falls back on.
const FALLBACK_DISTANCE: f32 = 5.0;
const FALLBACK_CONE: f32 = 0.5;
/// `light_update`'s range until something names one.
const DEFAULT_RANGE: f32 = 0.5;
pub(in crate::app::model_preview) const SMALLEST: f32 = 1e-6;

#[derive(Debug, Clone)]
struct H3Channel(NamedFunction);

impl std::ops::Deref for H3Channel {
    type Target = NamedFunction;

    fn deref(&self) -> &NamedFunction {
        &self.0
    }
}

impl H3Channel {
    fn read(root: TagStruct<'_>, name: &str) -> Option<Self> {
        NamedFunction::read(root, name).map(Self)
    }

    /// `light_update`: seconds alive, unless an object function is named.
    fn sample(&self, values: Option<&FunctionValues>, time: f32) -> (f32, f32) {
        let named = |name: &str| {
            values
                .and_then(|values| values.get(name))
                .copied()
                .unwrap_or(0.0)
        };
        let input = if self.input.is_empty() {
            time
        } else {
            named(&self.input)
        };
        let range = if self.range.is_empty() {
            DEFAULT_RANGE
        } else {
            named(&self.range)
        };
        (input, range)
    }
}

/// A `light` at the marker an object attaches it to.
#[derive(Debug, Clone)]
pub(crate) struct H3Light {
    /// `use parent position but world orientation`: its axes do not turn.
    world_oriented: bool,
    /// When its effect's event runs, for a light an attached effect makes.
    event: Option<(Arc<EffectTimes>, usize)>,
    /// `destroy light after`, seconds from then.
    destroy_after: f32,
    pub(crate) object: u8,
    pub(in crate::app::model_preview) node: usize,
    pub(in crate::app::model_preview) position: [f32; 3],
    pub(in crate::app::model_preview) forward: [f32; 3],
    up: [f32; 3],
    /// The region and permutation its marker belongs to, if only one.
    pub(in crate::app::model_preview) only_under: Option<(String, String)>,
    pub(in crate::app::model_preview) scale_by: String,
    /// Set for a light an attached effect makes.
    by_effect: Option<Gen3EffectScale>,
    /// `light_update` multiplies in the object's change color.
    change_color: Option<usize>,
    frustum: bool,
    maximum_distance: f32,
    near_width: f32,
    height_scale: f32,
    /// Radians.
    field_of_view: f32,
    diffusion: f32,
    smoothness: f32,
    spherical: f32,
    color: H3Channel,
    intensity: H3Channel,
    pub(crate) gel: Option<Arc<Image>>,
    /// Its `Lens Flare`, which `light_submit_lens_flares` draws at it.
    pub(in crate::app::model_preview) flare: Option<Arc<Gen3Flare>>,
}

/// What a frame's pass reads of a light.
pub(crate) struct H3LitLight {
    /// `s_simple_light`, as `simple_lights` holds it.
    pub(crate) rows: [[f32; 4]; 5],
    /// `Shadow_Projection`, a plain column matrix.
    pub(crate) projection: [f32; 16],
}

/// `c_lights_view::initialize_simple_light`.
fn simple_light(
    position: [f32; 3],
    direction: [f32; 3],
    color: [f32; 3],
    size: f32,
    max_distance: f32,
    cone: f32,
    smoothness: f32,
    spherical: f32,
) -> [[f32; 4]; 5] {
    let max_distance = if max_distance > 0.0 {
        max_distance
    } else {
        FALLBACK_DISTANCE
    };
    let cone = if cone > 0.0 { cone } else { FALLBACK_CONE };
    let smoothness = if smoothness > 0.0 { smoothness } else { 1.0 };
    let squared = max_distance * max_distance;
    let cos_edge = (cone * 0.5).cos();
    let cone_scale = (1.0 - spherical).powf(1.0 / smoothness) / (1.0 - cos_edge);
    let (cone_scale, cone_offset) = if cone < SMALLEST {
        (0.0, 1.0)
    } else {
        (cone_scale, -cone_scale * cos_edge)
    };
    let color = color.map(|channel| channel / size.max(SMALLEST));
    [
        [position[0], position[1], position[2], size],
        [-direction[0], -direction[1], -direction[2], spherical],
        [color[0], color[1], color[2], smoothness],
        [
            size * (size + squared) / squared,
            cone_scale,
            -size / squared,
            cone_offset,
        ],
        [squared, 0.0, 0.0, 0.0],
    ]
}

impl H3Light {
    pub(crate) fn inputs(&self) -> impl Iterator<Item = &str> {
        [
            &self.color.input,
            &self.color.range,
            &self.intensity.input,
            &self.intensity.range,
        ]
        .into_iter()
        .chain([&self.scale_by])
        .map(String::as_str)
    }

    pub(crate) fn animates(&self) -> bool {
        [&self.color, &self.intensity]
            .iter()
            .any(|channel| channel.input.is_empty() && channel.function.as_constant().is_none())
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// `light_update`'s `state.color` and `state.current_scale` this frame,
    /// or none while the light is out.
    pub(crate) fn state(
        &self,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        time: f32,
        onset: f32,
    ) -> Option<([f32; 3], f32)> {
        if let Some((times, event)) = &self.event {
            let elapsed = times.elapsed(*event, time - onset)?;
            if self.destroy_after > 0.0 && elapsed >= self.destroy_after {
                return None;
            }
        }
        let (input, range) = self.color.sample(values, time);
        let color = self.color.function.evaluate_color(input, range);
        let (input, range) = self.intensity.sample(values, time);
        let mut intensity = self.intensity.function.evaluate(input, range);
        let mut scale = 1.0;
        if !self.scale_by.is_empty() {
            let value = values
                .and_then(|values| values.get(&self.scale_by))
                .copied()
                .unwrap_or(0.0);
            scale = self.by_effect.map_or(value, |effect| effect.of(value));
            intensity *= scale;
        }
        let tint = self
            .change_color
            .and_then(|channel| change_colors.get(channel))
            .copied();
        let [red, green, blue] = tint.unwrap_or([1.0; 3]);
        let color = [color.red * red, color.green * green, color.blue * blue]
            .map(|channel| channel * intensity);
        if color.iter().all(|channel| *channel <= 0.0) {
            return None;
        }
        Some((color, scale))
    }

    /// The light this frame, or none while it is out.
    pub(crate) fn lit(
        &self,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        time: f32,
        onset: f32,
    ) -> Option<H3LitLight> {
        let (color, _) = self.state(values, change_colors, time, onset)?;
        let (position, mut forward) = posed_on(self.node, self.position, self.forward, bones);
        let (_, mut up) = posed_on(self.node, self.position, self.up, bones);
        if self.world_oriented {
            (forward, up) = (self.forward, self.up);
        }
        let (cone, near) = if self.frustum {
            (self.field_of_view, self.near_width)
        } else {
            (0.0, 0.0)
        };
        // A sphere has no cone: `cone_angle_radians` reads as the fallback.
        let cone_angle = if self.frustum {
            cone
        } else {
            std::f32::consts::TAU
        };
        let tangent = (cone.max(SMALLEST) * 0.5).tan();
        let behind = if self.frustum {
            near * 0.5 / tangent
        } else {
            0.0
        };
        let origin: [f32; 3] = std::array::from_fn(|axis| position[axis] - forward[axis] * behind);
        let rows = simple_light(
            origin,
            forward,
            color,
            self.diffusion,
            self.maximum_distance,
            cone_angle,
            self.smoothness,
            self.spherical,
        );
        // `setup_shadow_camera_and_projection`'s axes, into a gel's 0 to 1.
        let across = cross(forward, up);
        let along = cross(across, forward);
        let aspect = if self.height_scale > 0.0 {
            self.height_scale
        } else {
            1.0
        };
        let x: [f32; 3] =
            std::array::from_fn(|axis| across[axis] * 0.5 / tangent + forward[axis] * 0.5);
        let y: [f32; 3] = std::array::from_fn(|axis| {
            along[axis] * 0.5 / (tangent * aspect) + forward[axis] * 0.5
        });
        let to_origin = |row: [f32; 3]| -dot(row, origin);
        let mut projection = [0.0; 16];
        for axis in 0..3 {
            projection[axis * 4] = x[axis];
            projection[axis * 4 + 1] = y[axis];
            projection[axis * 4 + 3] = forward[axis];
        }
        projection[12] = to_origin(x);
        projection[13] = to_origin(y);
        projection[15] = to_origin(forward);
        Some(H3LitLight { rows, projection })
    }
}

fn read_h3_light(reader: &TagReader, path: &str) -> Option<H3LightTag> {
    let tag = reader.load(path, "light", b"ligh").ok()?;
    let root = tag.root();
    if has_flag(root, "flags", "render first person only") {
        return None;
    }
    let gel = root
        .read_tag_ref_path("gel bitmap")
        .filter(|gel| !gel.is_empty());
    let gel = gel
        .filter(|_| has_flag_labelled(root, "flags", &SHADOWS_AND_GELS))
        .and_then(|gel| decode_image(reader, &gel, SamplerKind::Texture2D, 0))
        .map(Arc::new);
    Some(H3LightTag {
        frustum: root
            .field("type")
            .is_some_and(|field| field.enum_is("frustum")),
        destroy_after: root.read_real("destroy light after").unwrap_or(0.0),
        maximum_distance: root.read_real("maximum distance").unwrap_or(0.0),
        near_width: root.read_real("frustum near width").unwrap_or(0.0),
        height_scale: root.read_real("frustum height scale").unwrap_or(1.0),
        field_of_view: root
            .read_real("frustum field of view")
            .unwrap_or(0.0)
            .to_radians(),
        diffusion: root.read_real("distance diffusion").unwrap_or(0.0),
        smoothness: root.read_real("angular smoothness").unwrap_or(0.0),
        spherical: root.read_real("percent spherical").unwrap_or(0.0),
        color: H3Channel::read(root, "color")?,
        intensity: H3Channel::read(root, "intensity")?,
        gel,
        flare: root
            .read_tag_ref_path("Lens Flare")
            .filter(|path| !path.is_empty())
            .and_then(|path| read_h3_flare(reader, &path))
            .map(Arc::new),
    })
}

/// A `light`, before a marker places it.
struct H3LightTag {
    destroy_after: f32,
    frustum: bool,
    maximum_distance: f32,
    near_width: f32,
    height_scale: f32,
    field_of_view: f32,
    diffusion: f32,
    smoothness: f32,
    spherical: f32,
    color: H3Channel,
    intensity: H3Channel,
    gel: Option<Arc<Image>>,
    flare: Option<Arc<Gen3Flare>>,
}

/// An object's attached `light`s, at the first marker of each name, and those
/// its attached effects make, `light_new_unattached` at each of a location's.
pub(in crate::app::model_preview) fn read_h3_lights(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H3Light> {
    let mut lights = Vec::new();
    let Some(block) = object_block(object, "attachments") else {
        return lights;
    };
    let light_group = u32::from_be_bytes(*b"ligh");
    let place_light = |tag: H3LightTag,
                       place: MarkerFrame,
                       node: usize,
                       only_under: Option<(String, String)>,
                       scale_by: String,
                       change_color: Option<usize>,
                       by_effect: Option<Gen3EffectScale>,
                       event: Option<(Arc<EffectTimes>, usize)>,
                       world_oriented: bool| H3Light {
        event,
        world_oriented,
        destroy_after: tag.destroy_after,
        object: owner,
        node,
        position: place.position,
        forward: place.axes[0],
        up: place.axes[2],
        only_under,
        scale_by,
        by_effect,
        change_color,
        frustum: tag.frustum,
        maximum_distance: tag.maximum_distance,
        near_width: tag.near_width,
        height_scale: tag.height_scale,
        field_of_view: tag.field_of_view,
        diffusion: tag.diffusion,
        smoothness: tag.smoothness,
        spherical: tag.spherical,
        color: tag.color,
        intensity: tag.intensity,
        gel: tag.gel,
        flare: tag.flare,
    };
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if group != light_group || path.is_empty() {
            continue;
        }
        let Some(tag) = read_h3_light(reader, &path) else {
            continue;
        };
        let name = marker_name(attachment, "marker");
        let Some(marker) = markers
            .iter()
            .find(|marker| marker.name.eq_ignore_ascii_case(&name))
        else {
            continue;
        };
        let (place, node) = frame(marker);
        let scale_by = attachment
            .read_string_id("primary scale")
            .unwrap_or_default();
        let change_color = function_slot_of(attachment, "change color");
        lights.push(place_light(
            tag,
            place,
            node,
            None,
            scale_by,
            change_color,
            None,
            None,
            false,
        ));
    }
    for_each_attached_part(reader, object, |part| {
        if part.group != light_group {
            return;
        }
        let (offset, yaw, pitch) = read_part_placement(part.part);
        for marker in &part.effect.locations(markers, &part.marker, &frame) {
            let Some(tag) = read_h3_light(reader, &part.path) else {
                return;
            };
            let (place, node) = frame(marker);
            let (place, node) = part.placed(place, node);
            let place = part_frame(&place, offset, yaw, pitch);
            let by_effect = Some(Gen3EffectScale {
                scaled: part.scaled,
            });
            let only_under = marker.only_under.clone();
            let event = Some((Arc::clone(&part.times), part.event));
            let (scale_by, change_color) = (part.effect.scale_by.clone(), part.effect.change_color);
            lights.push(place_light(
                tag,
                place,
                node,
                only_under,
                scale_by,
                change_color,
                by_effect,
                event,
                part.world_oriented,
            ));
        }
    });
    lights
}

attached!(H3Light);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_light_reaches_zero_at_its_maximum_distance() {
        let rows = simple_light(
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [1.0; 3],
            2.0,
            10.0,
            1.0,
            1.0,
            0.0,
        );
        let [_, _, _, [scale, _, offset, _], _] = rows;
        let at = |distance: f32| (1.0 / (2.0 + distance * distance)) * scale + offset;
        assert!(at(10.0).abs() < 1e-5);
        assert!((at(0.0) - (0.5 * scale + offset)).abs() < 1e-5);
    }

    #[test]
    fn a_cone_closes_at_half_its_angle() {
        let rows = simple_light([0.0; 3], [1.0, 0.0, 0.0], [1.0; 3], 1.0, 5.0, 1.0, 2.0, 0.0);
        let [_, _, _, [_, scale, _, offset], _] = rows;
        let at = |angle: f32| angle.cos() * scale + offset;
        assert!(at(0.5).abs() < 1e-5, "the edge is dark");
        assert!((at(0.0) - 1.0).abs() < 1e-4, "the axis is full");
    }

    #[test]
    fn a_light_with_no_distance_takes_the_game_s_fallback() {
        let rows = simple_light([0.0; 3], [1.0, 0.0, 0.0], [1.0; 3], 1.0, 0.0, 1.0, 1.0, 0.0);
        assert_eq!(rows[4][0], FALLBACK_DISTANCE * FALLBACK_DISTANCE);
    }

    #[test]
    fn the_stored_direction_points_back_at_the_light() {
        let rows = simple_light(
            [0.0; 3],
            [0.0, 0.0, 1.0],
            [1.0; 3],
            1.0,
            5.0,
            1.0,
            1.0,
            0.25,
        );
        assert_eq!(rows[1], [0.0, 0.0, -1.0, 0.25]);
    }
}
