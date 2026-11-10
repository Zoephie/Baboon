//! A flare's parameters for a frame: its view, occlusion and reflections.

use super::read::{
    FEWEST_SAMPLES, H4FlareAt, H4FlareDraw, H4FlareView, H4OcclusionPatch, H4Reflection,
    H4ReflectionColor, MINIMUM_VISIBLE, OCCLUSION_PIXELS, SMALLEST, corona_rotation,
};
use super::{
    FunctionValues, OcclusionOffset, dot, object_value, posed_on, unit_unless_under_micro,
};
use blam_tags::TagFunction;
use blam_tags::tag_function::transition_function_evaluate;
use std::f32::consts::PI;
use std::sync::Arc;

/// `LensFlareEvaluateParameters`' result (`s_lens_flare_render_parameters`).
#[derive(Debug, Clone, PartialEq)]
struct H4FlareParameters {
    brightness: f32,
    rotation: f32,
    radius: f32,
    scale: [f32; 2],
    position: [f32; 3],
    color: [f32; 3],
}

impl H4FlareAt {
    pub(crate) fn inputs(&self) -> impl Iterator<Item = &str> {
        let flare = &self.flare;
        let curves = [&flare.time_color, &flare.age_color]
            .into_iter()
            .flatten()
            .map(|curve| curve.input.as_str());
        [self.scale_by.as_str(), self.secondary_by.as_str()]
            .into_iter()
            .chain(curves)
    }

    pub(crate) fn animates(&self) -> bool {
        let flare = &self.flare;
        let moving = |function: &Option<Arc<TagFunction>>| {
            function
                .as_ref()
                .is_some_and(|function| function.as_constant().is_none())
        };
        moving(&flare.time_brightness)
            || moving(&flare.time_rotation)
            || flare.time_color.as_ref().is_some_and(|curve| {
                curve.input.is_empty() && curve.function.as_constant().is_none()
            })
    }

    /// `render_object_attachments`' `lens` and `lens_flares_create_internal`
    /// (X360 0x83AE7FC8): the scale both attachment functions give it, and
    /// the tint, brought down to at most 1 with the excess taken into the
    /// scale. None while the scale is off.
    fn created(
        &self,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
    ) -> Option<(f32, [f32; 3])> {
        let mut scale =
            object_value(values, &self.scale_by) * object_value(values, &self.secondary_by);
        if scale <= 0.0 {
            return None;
        }
        let mut tint = self
            .change_color
            .and_then(|channel| change_colors.get(channel))
            .copied()
            .unwrap_or([1.0; 3]);
        let most = tint.iter().copied().fold(0.0, f32::max);
        if most > 1.0 {
            tint = tint.map(|channel| channel / most);
            scale *= most;
        }
        // `pixel32`: the tint is stored as bytes.
        Some((
            scale,
            tint.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() / 255.0),
        ))
    }

    /// `LensFlareEvaluateParameters` (X360 0x83AE9730) for `reflection`, at
    /// visibility `opacity`, the flare's centre at `center` on screen.
    #[allow(clippy::too_many_arguments)]
    fn parameters(
        &self,
        reflection: &H4Reflection,
        scale: f32,
        tint: [f32; 3],
        position: [f32; 3],
        forward: [f32; 3],
        center: [f32; 2],
        opacity: f32,
        values: Option<&FunctionValues>,
        view: &H4FlareView<'_>,
        time: f32,
    ) -> Option<H4FlareParameters> {
        let flare = &*self.flare;
        let to_flare: [f32; 3] = std::array::from_fn(|axis| position[axis] - view.eye[axis]);
        let depth = dot(view.forward, to_flare);
        let to_center: [f32; 3] =
            std::array::from_fn(|axis| 2.0 * (view.forward[axis] * depth - to_flare[axis]));
        let to_eye = unit_unless_under_micro(to_flare.map(|axis| -axis));
        let mut brightness = 1.0;
        let [near, far] = flare.far_fade;
        if near != far {
            brightness *= ((depth - far) / (near - far)).clamp(0.0, 1.0);
        }
        let [begin, end] = flare.near_fade;
        if begin != end && depth < end {
            brightness *= ((depth - begin) / (end - begin)).clamp(0.0, 1.0);
        }
        if flare.cutoff > SMALLEST {
            let angle = dot(forward, to_eye).clamp(-1.0, 1.0).acos();
            if angle > flare.cutoff {
                brightness = 0.0;
            } else if flare.cutoff > flare.falloff {
                brightness *= 1.0
                    - ((angle - flare.falloff) / (flare.cutoff - flare.falloff)).clamp(0.0, 1.0);
            }
        }
        brightness *= opacity;
        // `CurrentDissolveAmount`: game time. An unsynchronized flare's own
        // offset (a checksum of its slot) is not reproduced.
        let clock = time;
        let age = 0.0;
        if let Some(function) = &flare.time_brightness {
            brightness *= function.evaluate(clock, 0.0);
        }
        if let Some(function) = &flare.age_brightness {
            brightness *= function.evaluate(age, 0.0);
        }
        if brightness <= 0.0 {
            return None;
        }
        let mut corona = corona_rotation(
            flare.rotation_function,
            flare.rotation_scale,
            position,
            forward,
            view,
        );
        let left = view.right.map(|axis| -axis);
        // [inferred, as Halo 3's: the projection's +56, +68 and +80 are its
        // forward, left and up.]
        let screen_angle = dot(to_flare, view.forward).atan2(dot(to_flare, left));
        let mut tint3 = [1.0; 3];
        for (curve, input) in [(&flare.time_color, clock), (&flare.age_color, age)] {
            if let Some(curve) = curve {
                let mut at = curve.function.evaluate(input, 0.0);
                if !curve.input.is_empty() {
                    at = object_value(values, &curve.input);
                }
                let color = curve.function.evaluate_color(at, 1.0);
                tint3 = [
                    tint3[0] * color.red,
                    tint3[1] * color.green,
                    tint3[2] * color.blue,
                ];
            }
        }
        if let Some(function) = &flare.time_rotation {
            corona += PI * function.evaluate(clock, 0.0);
        }
        if let Some(function) = &flare.age_rotation {
            corona += PI * function.evaluate(age, 0.0);
        }
        let mut input = scale;
        if reflection.flags.proximity_input {
            let from_center = (center[0] * center[0] + center[1] * center[1]).sqrt();
            input *= ((std::f32::consts::SQRT_2 - from_center) / std::f32::consts::SQRT_2)
                .clamp(0.0, 1.0);
        }
        let reflected = reflection.brightness.evaluate(input, 0.0) * brightness;
        // The marker matrix's scale: the object's, 1.
        let mut radius = reflection.radius.evaluate(input, 0.0);
        let mut color = match &reflection.color {
            H4ReflectionColor::Curve(curve) => {
                let color = curve.evaluate_color(input, 0.0);
                [color.red, color.green, color.blue]
            }
            H4ReflectionColor::Constant(color) => *color,
        };
        let scale = [
            reflection.scale[0].evaluate(input, 0.0),
            reflection.scale[1].evaluate(input, 0.0),
        ];
        if reflected <= 0.0 || radius <= 0.0 {
            return None;
        }
        let mut rotation = reflection.rotation_offset.to_radians() + corona;
        color = std::array::from_fn(|channel| color[channel] * tint3[channel]);
        if !reflection.flags.ignore_external_color {
            color = std::array::from_fn(|channel| color[channel] * tint[channel]);
        }
        if reflection.flags.rotate_from_center {
            rotation += screen_angle;
        }
        if reflection.flags.radius_by_occlusion {
            radius *= (opacity * 0.5 + 0.5).clamp(0.0, 1.0);
        }
        if reflection.flags.radius_by_distance {
            radius *= depth;
        }
        Some(H4FlareParameters {
            brightness: reflected,
            rotation,
            radius,
            scale,
            position: std::array::from_fn(|axis| {
                position[axis] + to_center[axis] * reflection.axis_offset
            }),
            color,
        })
    }

    fn placed(&self, bones: Option<&[[f32; 4]]>) -> ([f32; 3], [f32; 3]) {
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        (position, unit_unless_under_micro(forward))
    }

    /// `lens_flare_render_occlusion` (X360 0x83AE82E0): the occlusion
    /// reflection's quad facing the eye, sized by the inner share, pushed
    /// along the offset direction; none for a flare that tests nothing or
    /// shows nothing to test.
    pub(crate) fn occlusion_patch(
        &self,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &H4FlareView<'_>,
        time: f32,
    ) -> Option<H4OcclusionPatch> {
        let flare = &*self.flare;
        if flare.flags.no_occlusion_test || flare.flags.first_person_only {
            return None;
        }
        let (scale, tint) = self.created(values, change_colors)?;
        let (position, forward) = self.placed(bones);
        let (center, _) = (view.project)(position)?;
        let depth = dot(
            view.forward,
            std::array::from_fn(|axis| position[axis] - view.eye[axis]),
        );
        let opacity = transition_function_evaluate(flare.falloff_function, 1.0);
        // Past the last, the last.
        let reflection = flare
            .reflections
            .get(flare.occlusion_reflection)
            .or(flare.reflections.last())?;
        let parameters = self.parameters(
            reflection, scale, tint, position, forward, center, opacity, values, view, time,
        )?;
        // `LensFlareProjectBillboard` at radius 1: the half size in pixels,
        // before the projection's own scale.
        let half = [
            0.5 * view.viewport[0] / depth,
            0.5 * view.viewport[1] / depth,
        ];
        let across = ((2.0 * half[0] + 2.0 * half[1]) / OCCLUSION_PIXELS).clamp(0.0, 1.0);
        let share = if flare.flags.simple_occlusion_box {
            flare.inner_scale
        } else {
            flare.inner_scale + (1.0 - flare.inner_scale) * (1.0 - across)
        };
        let radius = parameters.radius * share;
        let half = [
            (half[0] * radius).min(view.viewport[0] / 2.0),
            (half[1] * radius).min(view.viewport[1] / 2.0),
        ];
        let expected = (2.5 * 2.0 * half[0] * 2.0 * half[1]).max(1.0);
        // The quad's own frame: facing the eye, upright on the screen, turned
        // by the flare's rotation when it asks.
        let (sine, cosine) = if flare.flags.rotate_occlusion_box {
            parameters.rotation.sin_cos()
        } else {
            (0.0, 1.0)
        };
        let left = view.right.map(|axis| -axis);
        let toward_viewer =
            unit_unless_under_micro(std::array::from_fn(|axis| view.eye[axis] - position[axis]));
        let offset = match flare.offset_direction {
            OcclusionOffset::TowardViewer => toward_viewer,
            OcclusionOffset::MarkerForward => forward,
            OcclusionOffset::None => [0.0; 3],
        };
        let corners =
            [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]].map(|[x, y]: [f32; 2]| {
                let (x, y) = (x * radius, y * radius);
                let (x, y) = (x * cosine - y * sine, x * sine + y * cosine);
                std::array::from_fn(|axis| {
                    position[axis]
                        + left[axis] * x
                        + view.up[axis] * y
                        + offset[axis] * flare.occlusion_offset
                })
            });
        Some(H4OcclusionPatch { corners, expected })
    }

    /// `lens_flares_render`' share of the patch seen, from the samples that
    /// passed; 1 for a flare that tests nothing.
    pub(crate) fn visibility(&self, passed: Option<(f32, f32)>) -> f32 {
        if self.flare.flags.no_occlusion_test {
            return 1.0;
        }
        let Some((passed, expected)) = passed else {
            return 0.0;
        };
        let passed = if passed > 0.0 && passed < FEWEST_SAMPLES {
            FEWEST_SAMPLES
        } else {
            passed
        };
        passed / expected.max(FEWEST_SAMPLES)
    }

    /// `lens_flare_render_internal`'s reflections at `visibility`, none below
    /// the least visible share.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draws(
        &self,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &H4FlareView<'_>,
        time: f32,
        visibility: f32,
    ) -> Vec<H4FlareDraw> {
        let flare = &*self.flare;
        if visibility <= MINIMUM_VISIBLE || flare.flags.first_person_only {
            return Vec::new();
        }
        let Some((scale, tint)) = self.created(values, change_colors) else {
            return Vec::new();
        };
        let (position, forward) = self.placed(bones);
        let mut draws = Vec::new();
        let mut opacity = visibility;
        for reflection in &flare.reflections {
            // Each reflection eases the last one's opacity, unless the flare
            // turns that feedback off.
            let from = if flare.flags.no_opacity_feedback {
                visibility
            } else {
                opacity
            };
            opacity = transition_function_evaluate(flare.falloff_function, from);
            if opacity <= SMALLEST {
                continue;
            }
            let Some((center, _)) = (view.project)(position) else {
                continue;
            };
            let Some(parameters) = self.parameters(
                reflection, scale, tint, position, forward, center, opacity, values, view, time,
            ) else {
                continue;
            };
            let Some((at, focal)) = (view.project)(parameters.position) else {
                continue;
            };
            let depth = dot(
                view.forward,
                std::array::from_fn(|axis| parameters.position[axis] - view.eye[axis]),
            );
            let size = [
                reflection.aspect[0] * parameters.scale[0] * parameters.radius / depth,
                reflection.aspect[1] * parameters.scale[1] * parameters.radius / depth,
            ];
            let x = if reflection.flags.lock_x {
                center[0]
            } else {
                at[0]
            };
            let y = if reflection.flags.lock_y {
                center[1]
            } else {
                at[1]
            };
            let [lower, upper] = reflection.offset_bounds;
            let [r, g, b] = parameters.color;
            draws.push(H4FlareDraw {
                image: reflection.image,
                center_rotation: [x, y, parameters.rotation, 0.0],
                scale: [size[0], size[1], focal[0], focal[1]],
                origin_bounds: [center[0], center[1], lower, upper],
                // World-space axes (flag 0x800) are not read.
                axes: [1.0, 0.0, 0.0, 1.0],
                mirror: reflection.flags.mirror,
                modulation: [
                    1.0 - reflection.modulation,
                    reflection.tint_power,
                    parameters.brightness,
                    1.0 - reflection.modulation,
                ],
                tint: [r, g, b, 1.0],
                flip: [reflection.flags.flip_u, reflection.flags.flip_v],
            });
        }
        draws
    }
}
