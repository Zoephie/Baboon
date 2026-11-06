//! Reach's screen-space lights on the lit frame (`c_lights_view::render`):
//! cheap lights, then each screen-space `light`, added through its
//! `light_apply` explicit shader. The inline lights go to the static pass as
//! `simple_lights` instead. See `docs/reach-preview-research/lights.md`.

use eframe::glow::{self, HasContext};

use super::super::gen3::screen_lights::screen_to_relative_world;
use super::super::gen3::pipeline::{set_uniform_value, view_projection};
use super::super::renderer::{FrameCx, RenderCore, h2_eye_position};
use super::lighting::{REACH_EXPOSURE, REACH_HDR_DIVISOR, reach_frame_value};
use super::lights::{CHEAP_ALBEDO_BLEND, ReachLightMethod, ReachLitLight};
use super::render::ReachGame;

/// `render_screenspace_light`'s near fade: out within `near_fading` of the
/// camera, full at twice it.
fn near_fade(light: &ReachLitLight, eye: [f32; 3]) -> f32 {
    if light.near_fading <= 0.0 {
        return 1.0;
    }
    let distance = (0..3).map(|axis| (light.position[axis] - eye[axis]).powi(2)).sum::<f32>().sqrt();
    (distance / light.near_fading - 1.0).clamp(0.0, 1.0)
}

impl ReachGame {
    /// Every Reach light of the frame, lit: its attached lights by age since
    /// their scale came on, and its cheap lights by render time.
    pub(super) fn frame_lights_of(cx: &mut FrameCx<'_>) -> Vec<ReachLitLight> {
        let (frame, slot) = (cx.frame, cx.slot);
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        let mut lit = Vec::new();
        let lights = frame.preview.reach_lights.len();
        let shown = |index: usize| frame.reach_lights_shown.get(index).copied().unwrap_or(true);
        for (index, light) in frame.preview.reach_cheap_lights.iter().enumerate() {
            if !shown(lights + index) {
                continue;
            }
            let values = frame.function_values_of_object(light.object);
            let colors = frame.change_colors_of_object(light.object);
            lit.extend(light.lit(values, &colors, bones, frame.time));
        }
        for (index, light) in frame.preview.reach_lights.iter().enumerate() {
            if !shown(index) {
                continue;
            }
            let values = frame.function_values_of_object(light.object);
            let colors = frame.change_colors_of_object(light.object);
            let onset = cx.core.effect_onsets(slot).of(light.object, light.scale_name(), values, frame.time);
            lit.extend(light.lit(values, &colors, bones, frame.time - onset));
        }
        lit
    }

    /// `render_cheap_screenspace_lights_from_draw_list`, then the screen-space
    /// `light`s, into `accumulation_target`. Its depth is copied first, for the
    /// pass to read.
    pub(super) unsafe fn draw_screen_lights(
        &mut self,
        cx: &mut FrameCx<'_>,
        lit: &[ReachLitLight],
        accumulation_target: glow::NativeFramebuffer,
    ) {
        let (gl, frame, core) = (cx.gl, cx.frame, &mut *cx.core);
        let screen: Vec<&ReachLitLight> =
            lit.iter().filter(|light| light.method == ReachLightMethod::Screenspace).collect();
        let Some(targets) = core.gen3.targets.as_ref() else {
            return;
        };
        if screen.is_empty() {
            return;
        }
        let (size, albedo, normal, depth_copy) = (targets.size, targets.albedo, targets.normal, targets.depth_copy);
        let eye = h2_eye_position(&frame.camera);
        let view_projection = view_projection(&frame.camera);
        let depth_copy_framebuffer = targets.depth_copy_framebuffer;
        unsafe {
            core.begin_screen_lights(gl, size, accumulation_target, depth_copy_framebuffer);
            for light in screen {
                let combo = usize::from(light.frustum) + 2 * usize::from(light.specular);
                let Some(pass) = frame.preview.reach_light_passes.get(combo).cloned().flatten() else {
                    continue;
                };
                let fade = near_fade(light, eye);
                let Some(matrix) = screen_to_relative_world(&view_projection, size, light.origin) else {
                    continue;
                };
                if fade <= 1e-4 {
                    continue;
                }
                let linked = core.with_h3_program(gl, &pass, |this, program| {
                    gl.use_program(Some(program.program));
                    // Tag lights come down by the HDR target's stops; cheap
                    // lights only by exposure (`render_cheap_screenspace_lights`).
                    let exposure = if light.cheap { REACH_EXPOSURE } else { REACH_EXPOSURE / REACH_HDR_DIVISOR };
                    let [r, g, b] = light.color.map(|channel| channel * exposure * fade);
                    let to_light: [f32; 3] = std::array::from_fn(|axis| light.origin[axis] - eye[axis]);
                    for (name, ty) in &pass.uniforms {
                        let location = program.location(gl, name);
                        if location.is_none() {
                            continue;
                        }
                        match name.as_str() {
                            "screen_to_relative_world" => {
                                gl.uniform_matrix_4_f32_slice(location.as_ref(), false, matrix.as_flattened());
                            }
                            "light_rotation" => {
                                gl.uniform_matrix_3_f32_slice(location.as_ref(), true, light.rotation.as_flattened());
                            }
                            "light_colour_falloff_power" => {
                                gl.uniform_4_f32(location.as_ref(), r, g, b, light.falloff_power)
                            }
                            "light_attenuation" => gl.uniform_4_f32_slice(location.as_ref(), &light.attenuation),
                            "camera_to_light" => {
                                gl.uniform_4_f32(location.as_ref(), to_light[0], to_light[1], to_light[2], 0.0)
                            }
                            "material_coeff" => gl.uniform_4_f32(location.as_ref(), CHEAP_ALBEDO_BLEND, 0.0, 0.0, 0.0),
                            other => {
                                if let Some(value) = reach_frame_value(frame, other, this.gen3.stops) {
                                    set_uniform_value(gl, location.as_ref(), ty, &value);
                                }
                            }
                        }
                    }
                    RenderCore::bind_screen_light_textures(gl, &pass, program, |name| match name {
                        "GlobalTexture_depth_sampler" => Some(Some(depth_copy)),
                        "GlobalTexture_albedo_sampler" => Some(Some(albedo)),
                        "GlobalTexture_normal_sampler" => Some(Some(normal)),
                        _ => None,
                    });
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                });
                if linked.is_none() {
                    continue;
                }
            }
            RenderCore::end_screen_lights(gl);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_near_fading_light_is_out_up_close_and_full_at_twice_its_distance() {
        let mut light = ReachLitLight {
            method: ReachLightMethod::Screenspace,
            specular: false,
            frustum: false,
            origin: [0.0; 3],
            position: [0.0; 3],
            forward: [1.0, 0.0, 0.0],
            left: [0.0, 1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            color: [1.0; 3],
            falloff_power: 5.0,
            attenuation: [1.0; 4],
            rotation: [[0.0; 3]; 3],
            range: [1.0, 0.0],
            cheap: false,
            near_fading: 2.0,
        };
        assert_eq!(near_fade(&light, [1.0, 0.0, 0.0]), 0.0);
        assert_eq!(near_fade(&light, [3.0, 0.0, 0.0]), 0.5);
        assert_eq!(near_fade(&light, [5.0, 0.0, 0.0]), 1.0);
        light.near_fading = 0.0;
        assert_eq!(near_fade(&light, [0.1, 0.0, 0.0]), 1.0);
    }
}
