//! Reach's screen-space lights on the lit frame (`c_lights_view::render`):
//! cheap lights, then each screen-space `light`, added through its
//! `light_apply` explicit shader. The inline lights go to the static pass as
//! `simple_lights` instead. See `docs/reach-preview-research/lights.md`.

use eframe::glow::{self, HasContext};

use super::super::gen3::screen_lights::screen_to_relative_world;
use super::super::gen3::pipeline::{set_value, view_projection};
use crate::app::model_preview::common::camera::eye_position;
use super::super::gen3::render::Gen3Cx;
use super::super::gen3::state::Gen3State;
use super::lighting::{REACH_EXPOSURE, REACH_HDR_DIVISOR, reach_frame_value};
use super::lights::{CHEAP_ALBEDO_BLEND, ReachLightMethod, ReachLitLight};
use super::render::ReachGame;

use std::sync::Arc;
use super::super::{RenderModelPreview, TagReader};

use super::super::gen3::shaders::{H3Pass, H3Sources, SCREEN_VERTEX, VERTEX_SKINNED, base_defines, pass_of};
use crate::core::hlsl::emit::{Stage, Translated};

impl H3Sources {
    /// One of Reach's `light_apply` explicit shaders (`lights\\<file>`): its
    /// pixel stage only, over a triangle that covers the picture. The game
    /// draws it over the light's volume behind a stencil; outside a light's
    /// reach its falloff is 0, so the whole picture serves.
    pub(crate) fn reach_light_pass(&mut self, file: &str) -> Result<Arc<H3Pass>, String> {
        let main = format!("#include \"lights\\{file}.hlsl\"\n");
        let defines = base_defines(self.dialect(), VERTEX_SKINNED, Stage::Pixel);
        let pixel = self.compile(&defines, &main, "default_ps", Stage::Pixel)?;
        let vertex =
            Translated { source: SCREEN_VERTEX.to_owned(), uniforms: Vec::new(), textures: Vec::new(), attributes: Vec::new() };
        let mut pass = pass_of(vertex, pixel);
        pass.vertex = SCREEN_VERTEX.to_owned();
        pass.fullscreen = true;
        Ok(Arc::new(pass))
    }
}

/// `render_screenspace_light_callback`'s `99 + combo`, gels aside: the
/// `light_apply` passes the object's lights draw with, compiled once.
pub(in crate::app::model_preview) fn read_reach_light_passes(reader: &TagReader, preview: &mut RenderModelPreview) {
    let lit = !(preview.reach.lights.is_empty() && preview.reach.cheap_lights.is_empty());
    if lit && preview.reach.light_passes.is_empty() {
        let mut sources = H3Sources::new(&reader.source);
        let files = ["light_apply", "light_apply_frustum", "light_apply_specular", "light_apply_frustum_specular"];
        preview.reach.light_passes = files.iter().map(|file| sources.reach_light_pass(file).ok()).collect();
    }
}

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
    pub(super) fn frame_lights_of(cx: &mut Gen3Cx<'_>) -> Vec<ReachLitLight> {
        let (frame, slot) = (cx.frame, cx.slot);
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        let mut lit = Vec::new();
        for (index, light) in frame.preview.reach.cheap_lights.iter().enumerate() {
            if !frame.shown.is(super::super::common::features::paths::REACH_CHEAP_LIGHTS, index) {
                continue;
            }
            let values = frame.function_values_of_object(light.object);
            let colors = frame.change_colors_of_object(light.object);
            lit.extend(light.lit(values, &colors, bones, frame.time));
        }
        for (index, light) in frame.preview.reach.lights.iter().enumerate() {
            if !frame.shown.is(super::super::common::features::paths::REACH_LIGHTS, index) {
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
        cx: &mut Gen3Cx<'_>,
        lit: &[ReachLitLight],
        accumulation_target: glow::NativeFramebuffer,
    ) {
        let (gl, frame, core) = (cx.gl, cx.frame, &mut *cx.core);
        let state = &mut *cx.state;
        let screen: Vec<&ReachLitLight> =
            lit.iter().filter(|light| light.method == ReachLightMethod::Screenspace).collect();
        let Some(targets) = state.targets.as_ref() else {
            return;
        };
        if screen.is_empty() {
            return;
        }
        let (size, albedo, normal, depth_copy) = (targets.size, targets.albedo, targets.normal, targets.depth_copy);
        let eye = eye_position(&frame.camera);
        let view_projection = view_projection(&frame.camera);
        let depth_copy_framebuffer = targets.depth_copy_framebuffer;
        unsafe {
            state.begin_screen_lights(gl, size, accumulation_target, depth_copy_framebuffer);
            for light in screen {
                let combo = usize::from(light.frustum) + 2 * usize::from(light.specular);
                let Some(pass) = frame.preview.reach.light_passes.get(combo).cloned().flatten() else {
                    continue;
                };
                let fade = near_fade(light, eye);
                let Some(matrix) = screen_to_relative_world(&view_projection, size, light.origin) else {
                    continue;
                };
                if fade <= 1e-4 {
                    continue;
                }
                let linked = state.with_program(core, gl, &pass, |state, _, program| {
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
                                if let Some(value) = reach_frame_value(frame, other, state.stops) {
                                    set_value(gl, location.as_ref(), ty, &value);
                                }
                            }
                        }
                    }
                    Gen3State::bind_screen_light_textures(gl, &pass, program, |name| match name {
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
            Gen3State::end_screen_lights(gl);
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
