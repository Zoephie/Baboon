//! A Halo CE object's lights: their reflections, coronas and colours, read off the object and what it holds.

use super::super::super::{TagFile, TagReader};
use super::super::markers::{CeFound, CeMarker, CeObjectMarkers};
use super::flare::CeFlare;
use super::point::read_light_tag;
use super::{
    FlareSprite, OcclusionOffset, ViewFrame, cross, dot, elements, function_slot_of, lerp,
    object_block, periodic_evaluate, posed_on, rgb_colors_interpolate,
};
use std::sync::Arc;

/// Colors of a reflection that all read zero take the light's instead.
pub(super) const NO_COLOR: [f32; 4] = [0.0; 4];

/// One sprite of a flare, set along the axis through the screen's center.
#[derive(Debug, Clone)]
pub(crate) struct CeReflection {
    /// Its `flags`, as `rasterizer_lens_flares_draw` reads them.
    pub(super) align_with_screen_center: bool,
    pub(super) radius_not_scaled_by_distance: bool,
    pub(super) radius_scaled_by_occlusion: bool,
    /// Which of [`CeFlare::images`] it draws.
    pub(crate) image: usize,
    pub(super) position: f32,
    pub(super) rotation: f32,
    pub(super) radius: [f32; 2],
    pub(super) brightness: [f32; 2],
    pub(super) brightness_scaled_by: usize,
    pub(super) tint: [f32; 4],
    pub(super) lower: [f32; 4],
    pub(super) upper: [f32; 4],
    /// [`rgb_colors_interpolate`]'s flags.
    pub(super) interpolation: i128,
    /// `animation function`'s number, which [`periodic_evaluate`] switches on.
    pub(super) animation_function: usize,
    pub(super) animation_period: f32,
    pub(super) animation_phase: f32,
}

/// A light on an object, at one of the markers of its name.
#[derive(Debug, Clone)]
pub(crate) struct CeLight {
    /// Which object's functions and change colors drive it; 0 is the opened.
    pub(crate) object: u8,
    pub(super) node: usize,
    pub(super) position: [f32; 3],
    pub(super) forward: [f32; 3],
    /// `A out` to `D out`, or none, which reads 1.
    pub(super) primary: Option<usize>,
    pub(super) change_color: Option<usize>,
    pub(super) lower: [f32; 4],
    pub(super) upper: [f32; 4],
    /// `interpolation flags`, for [`rgb_colors_interpolate`].
    pub(super) interpolation: i128,
    /// `lights_preprocess_scene` submits it to light models.
    pub(super) dynamic: bool,
    /// `find_point_lights_for_object_in_cluster` leaves its own object out.
    pub(super) spares_owner: bool,
    pub(super) radius: f32,
    pub(super) radius_bounds: [f32; 2],
    /// Cosines of its `falloff angle` and `cutoff angle`, or none in all ways.
    pub(super) cone: Option<[f32; 2]>,
    pub(crate) flare: Option<Arc<CeFlare>>,
    /// A light an effect made: the value its age gives it, in place of a
    /// function's, and a color of the bounds alone.
    pub(super) fixed: Option<f32>,
}

fn rgb_of(color: [f32; 4]) -> [f32; 3] {
    [color[0], color[1], color[2]]
}

/// `lens_flare_evaluate_corona_rotation_function`: how far round a corona
/// turns as a share of a whole turn, or none for `none`.
pub(super) fn corona_rotation(
    function: i128,
    forward: [f32; 3],
    to_light: [f32; 3],
    view: &ViewFrame,
) -> f32 {
    let back = view.forward.map(|axis| -axis);
    let along = cross(cross(forward, view.right), forward);
    let (sine, cosine) = match function {
        1 => (dot(along, view.forward), -dot(forward, view.forward)),
        2 => (-dot(view.right, forward), dot(back, forward)),
        3 => (dot(along, to_light), -dot(forward, to_light)),
        4 => (dot(view.right, to_light), -dot(back, to_light)),
        _ => return 0.0,
    };
    if sine == 0.0 {
        0.0
    } else {
        sine.atan2(cosine) / std::f32::consts::TAU
    }
}

/// `rgb_colors_interpolate_and_scale`: the bounds' alphas say how much of the
/// color the change color tints.
pub(super) fn light_color(
    flags: i128,
    lower: [f32; 4],
    upper: [f32; 4],
    change: [f32; 3],
    value: f32,
) -> [f32; 3] {
    let color = rgb_colors_interpolate(flags, rgb_of(lower), rgb_of(upper), value);
    if lower[3] <= 0.0001 && upper[3] <= 0.0001 {
        return std::array::from_fn(|channel| color[channel] * change[channel]);
    }
    let own = lerp(lower[3], upper[3], value);
    std::array::from_fn(|channel| color[channel] * own + change[channel] * (1.0 - own))
}

impl CeLight {
    /// What `object_get_function_value` gives it.
    fn value(&self, outputs: &[f32; 4]) -> f32 {
        self.fixed
            .unwrap_or_else(|| self.primary.map_or(1.0, |index| outputs[index]))
    }

    /// The patch tested for how much of the flare the model hides, if it
    /// shows at all: `rasterizer_lens_flares_submit_occlusion_tests`.
    pub(crate) fn occlusion_patch(
        &self,
        outputs: &[f32; 4],
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
    ) -> Option<FlareSprite> {
        let flare = self.flare.as_deref()?;
        if self.color(self.value(outputs), change_colors) == [0.0; 3] {
            return None;
        }
        let (position, forward) = self.posed(bones);
        let radius = flare.occlusion_radius;
        let offset = match flare.occlusion_offset {
            OcclusionOffset::TowardViewer => view.forward.map(|axis| -axis * radius),
            OcclusionOffset::MarkerForward => {
                forward.map(|axis| axis * radius * std::f32::consts::SQRT_2)
            }
            OcclusionOffset::None => [0.0; 3],
        };
        Some(FlareSprite {
            center: std::array::from_fn(|axis| position[axis] + offset[axis]),
            extent: [radius; 2],
            rotation: 0.0,
            color: [0.0; 3],
            alpha: 0.0,
            image: 0,
            highlight: None,
            tint: None,
            depth_tested: false,
        })
    }

    pub(super) fn color(&self, value: f32, change_colors: &[[f32; 3]; 4]) -> [f32; 3] {
        // `rgb_colors_interpolate`, for a light `light_new_unattached` made.
        if self.fixed.is_some() {
            return rgb_colors_interpolate(
                self.interpolation,
                rgb_of(self.lower),
                rgb_of(self.upper),
                value,
            );
        }
        let change = self
            .change_color
            .map_or([1.0; 3], |index| change_colors[index]);
        light_color(self.interpolation, self.lower, self.upper, change, value)
    }

    /// `rasterizer_set_model_lighting_point_light`, in the form
    /// [`CE_POINT_LIGHT_ROWS`] takes.
    pub(super) fn point_rows(
        &self,
        outputs: &[f32; 4],
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
    ) -> Option<([[f32; 4]; 3], f32)> {
        if !self.dynamic {
            return None;
        }
        let value = self.value(outputs);
        let color = self.color(value, change_colors);
        let radius = self.radius * lerp(self.radius_bounds[0], self.radius_bounds[1], value);
        if color == [0.0; 3] || radius <= 0.0 {
            return None;
        }
        let (position, forward) = self.posed(bones);
        let (a, b) = match self.cone {
            Some([falloff, cutoff]) if falloff != cutoff => {
                let a = 1.0 / (falloff - cutoff);
                (a, -cutoff * a)
            }
            _ => (0.0, 1.0),
        };
        let [x, y, z] = position;
        let [i, j, k] = forward;
        let [red, green, blue] = color;
        Some((
            [
                [x, y, z, 1.0 / (radius * radius)],
                [i, j, k, a],
                [red, green, blue, b],
            ],
            radius,
        ))
    }

    /// `rasterizer_lens_flares_draw`, for one light of `lights_preprocess_scene`.
    pub(crate) fn sprites(
        &self,
        outputs: &[f32; 4],
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        occlusion: f32,
    ) -> Vec<FlareSprite> {
        let Some(flare) = self.flare.as_deref() else {
            return Vec::new();
        };
        let value = self.value(outputs);
        let color = self.color(value, change_colors);
        if color == [0.0; 3] {
            return Vec::new();
        }
        let (position, forward) = self.posed(bones);
        let to_light: [f32; 3] = std::array::from_fn(|axis| position[axis] - view.eye[axis]);
        let distance = dot(to_light, view.forward);
        let length = dot(to_light, to_light).sqrt().max(0.0001);
        let toward = to_light.map(|value| value / length);
        // The axis runs through the light and the screen's center.
        let mirror: [f32; 3] =
            std::array::from_fn(|axis| (view.forward[axis] * distance - to_light[axis]) * 2.0);
        let fade = if flare.fade[1] > 0.0 {
            ((distance - flare.fade[1]) / (flare.fade[0] - flare.fade[1])).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let [falloff, cutoff] = flare.cones;
        let slope = if falloff == cutoff {
            0.0
        } else {
            1.0 / (falloff - cutoff)
        };
        let offset = -cutoff * slope;
        let scales = [
            1.0,
            (offset - dot(forward, view.forward) * slope).clamp(0.0, 1.0),
            (offset - dot(forward, toward) * slope).clamp(0.0, 1.0),
            (dot(toward, view.forward) * slope + offset).clamp(0.0, 1.0),
        ];
        let screen_angle = dot(to_light, view.right).atan2(dot(to_light, view.up));
        let corona = corona_rotation(flare.rotation_function, forward, to_light, view);
        let mut brightness = fade * occlusion;
        let mut sprites = Vec::new();
        for (index, reflection) in flare.reflections.iter().enumerate() {
            let scale = scales
                .get(reflection.brightness_scaled_by)
                .copied()
                .unwrap_or(1.0);
            let alpha = lerp(reflection.brightness[0], reflection.brightness[1], value)
                * scale
                * brightness;
            // The first reflection dims the rest.
            if index == 0 {
                brightness = alpha;
            }
            if alpha <= 0.0 {
                continue;
            }
            let mut radius = lerp(reflection.radius[0], reflection.radius[1], value);
            if reflection.radius_scaled_by_occlusion {
                radius *= (occlusion + 1.0) * 0.5;
            }
            if reflection.radius_not_scaled_by_distance {
                radius *= distance;
            }
            let mut rotation = reflection.rotation.to_radians();
            if index == 0 {
                rotation += corona * flare.rotation_scale;
            }
            if reflection.align_with_screen_center {
                rotation += screen_angle;
            }
            let stretch = if index == 0 {
                flare.corona_scale
            } else {
                [1.0; 2]
            };
            sprites.push(FlareSprite {
                center: std::array::from_fn(|axis| {
                    position[axis] + mirror[axis] * reflection.position
                }),
                extent: [radius * stretch[0], radius * stretch[1]],
                rotation,
                color: reflection.tinted(color, time),
                alpha,
                image: reflection.image,
                highlight: None,
                tint: None,
                depth_tested: false,
            });
        }
        sprites
    }

    fn posed(&self, bones: Option<&[[f32; 4]]>) -> ([f32; 3], [f32; 3]) {
        posed_on(self.node, self.position, self.forward, bones)
    }
}

impl CeReflection {
    /// The light's color, unless the reflection sets its own. An animation
    /// function past `one` and `zero` runs it between its bounds.
    fn tinted(&self, light: [f32; 3], time: f32) -> [f32; 3] {
        if self.tint == NO_COLOR && self.lower == NO_COLOR && self.upper == NO_COLOR {
            return light;
        }
        let tint: [f32; 3] = std::array::from_fn(|channel| self.tint[channel]);
        let own = if self.animation_function > 1 {
            let period = if self.animation_period == 0.0 {
                1.0
            } else {
                self.animation_period
            };
            let phase = (self.animation_phase + time) / period;
            let u = periodic_evaluate(self.animation_function, phase);
            rgb_colors_interpolate(
                self.interpolation,
                rgb_of(self.lower),
                rgb_of(self.upper),
                u,
            )
        } else {
            tint
        };
        std::array::from_fn(|channel| lerp(light[channel], own[channel], self.tint[3]))
    }
}

/// Mark the outputs an object's lights take their value from.
pub(in crate::app::model_preview) fn mark_outputs_read(
    lights: &[CeLight],
    object: u8,
    read: &mut [bool; 4],
) {
    let owned = lights.iter().filter(|light| light.object == object);
    for output in owned.filter_map(|light| light.primary) {
        read[output] = true;
    }
}

/// The lights among `object`'s attachments, at its `markers`. `light_new`
/// makes one per attachment: `light_reconnect_to_map` puts its light at the
/// first slot of `object_get_marker_by_name`, read whatever the count (the
/// origin when no marker is found), and `lights_preprocess_scene` draws its
/// lens flare at every marker found, up to 8, none when none is.
pub(in crate::app::model_preview) fn read_ce_lights(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &CeObjectMarkers,
) -> Vec<CeLight> {
    const MOST_FLARES: usize = 8;
    let mut lights = Vec::new();
    let Some(block) = object_block(object, "attachments") else {
        return lights;
    };
    let light_group = u32::from_be_bytes(*b"ligh");
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if group != light_group || path.is_empty() {
            continue;
        }
        let Some(template) = read_light_tag(reader, &path) else {
            continue;
        };
        // `light_new`: it needs one or the other to exist.
        if template.flare.is_none() && !template.dynamic {
            continue;
        }
        let name = attachment.read_string("marker").unwrap_or_default();
        let found = markers.get(&name, MOST_FLARES);
        let placed = light_placements(&found, template.dynamic, template.flare.is_some());
        lights.extend(placed.into_iter().map(|(at, dynamic, flare)| CeLight {
            object: owner,
            node: at.node,
            position: at.position,
            forward: at.forward(),
            primary: function_slot_of(attachment, "primary scale"),
            change_color: function_slot_of(attachment, "change color"),
            dynamic,
            flare: template.flare.clone().filter(|_| flare),
            ..template.clone()
        }));
    }
    lights
}

/// Where one light attachment draws, and what at each: its dynamic light at
/// the first slot (`light_reconnect_to_map`), its lens flare at each marker
/// found (`lights_preprocess_scene`). Each is a place, whether its light is
/// there, and whether a flare is; a place with neither is left out.
fn light_placements(found: &CeFound, dynamic: bool, flare: bool) -> Vec<(CeMarker, bool, bool)> {
    let flared = found.found();
    let first = (found.first(), dynamic, flare && !flared.is_empty());
    std::iter::once(first)
        .chain(flared.iter().skip(1).map(|at| (*at, false, flare)))
        .filter(|(_, dynamic, flare)| *dynamic || *flare)
        .collect()
}
#[cfg(test)]
mod placement_tests {
    use super::super::super::super::{ModelRegionSelection, RenderModelPreviewMarker};
    use super::*;
    use std::collections::HashMap;

    fn markers(names: &[(&str, f32)]) -> CeObjectMarkers {
        let markers: Vec<_> = names
            .iter()
            .map(|(name, x)| RenderModelPreviewMarker {
                name: (*name).to_owned(),
                position: [*x, 0.0, 0.0],
                axes: CeMarker::ORIGIN.axes,
                ..Default::default()
            })
            .collect();
        let picks: HashMap<String, ModelRegionSelection> = HashMap::new();
        CeObjectMarkers::of_markers(&markers, &[], &picks, CeMarker::ORIGIN)
    }

    fn placed(found: &CeFound, dynamic: bool, flare: bool) -> Vec<(f32, bool, bool)> {
        light_placements(found, dynamic, flare)
            .into_iter()
            .map(|(at, dynamic, flare)| (at.position[0], dynamic, flare))
            .collect()
    }

    /// One light per attachment, at the first marker; a flare at each.
    #[test]
    fn a_light_on_a_repeated_marker_lights_once_and_flares_at_each() {
        let found = markers(&[("lamp", 1.0), ("lamp", 2.0)]).get("lamp", 8);
        assert_eq!(
            placed(&found, true, true),
            [(1.0, true, true), (2.0, false, true)]
        );
    }

    /// A marker not found: the light at the origin, no flare at all.
    #[test]
    fn a_light_whose_marker_is_missing_lights_the_origin_without_a_flare() {
        let found = markers(&[("lamp", 1.0)]).get("gone", 8);
        assert_eq!(placed(&found, true, true), [(0.0, true, false)]);
        assert!(
            placed(&found, false, true).is_empty(),
            "a flare alone draws nothing"
        );
        let unnamed = markers(&[]).get("", 8);
        assert_eq!(
            placed(&unnamed, false, true),
            [(0.0, false, true)],
            "no name is the origin, found"
        );
    }
}
