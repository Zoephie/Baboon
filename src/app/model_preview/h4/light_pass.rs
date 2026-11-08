//! Halo 4's screen-space lights on the lit frame (`MidnightLightParfait::Render`,
//! X360 0x82A0A3F0): each attached light and cheap light, added through
//! explicit shader 187 or 204. See `docs/h4-preview-research/lights.md`.

use eframe::glow::{self, HasContext};

use super::super::gen3::shaders::{H3Dialect, H3Value};
use super::super::gen3::screen_lights::screen_to_relative_world;
use super::super::gen3::pipeline::{frame_value, h4_matrix, set_value, view_projection};
use super::super::frame::ModelGpuFrame;
use crate::app::model_preview::common::camera::h2_eye_position;
use super::super::gen3::render::Gen3Cx;
use super::super::gen3::state::Gen3State;
use super::lights::{H4LightBlend, H4LitLight};
use super::render::H4Game;

impl H4Game {
    /// Every Halo 4 light of the frame, on the preview's clock as game time.
    fn frame_lights_of(frame: &ModelGpuFrame) -> Vec<H4LitLight> {
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        let eye = h2_eye_position(&frame.camera);
        let mut lit = Vec::new();
        for (index, light) in frame.preview.h4.lights.iter().enumerate() {
            if !frame.shown.is(super::super::common::features::paths::H4_LIGHTS, index) {
                continue;
            }
            let values = frame.function_values_of_object(light.object);
            let colors = frame.change_colors_of_object(light.object);
            lit.extend(light.lit(values, &colors, bones, frame.time, eye));
        }
        for (index, light) in frame.preview.h4.cheap_lights.iter().enumerate() {
            if !frame.shown.is(super::super::common::features::paths::H4_CHEAP_LIGHTS, index) {
                continue;
            }
            let values = frame.function_values_of_object(light.object);
            let colors = frame.change_colors_of_object(light.object);
            lit.extend(light.lit(values, &colors, bones, frame.time));
        }
        lit
    }

    /// `MidnightLightParfait::Render` (X360 0x82A0A3F0) for the object's
    /// lights, into `accumulation_target` after the static lighting: each
    /// light's pass over the whole picture, reading the albedo, normal and a
    /// copy of the depth by pixel. The game bounds each with a stencilled
    /// volume (explicit 52, an icosahedron at 1.1 times the light's end);
    /// the falloff reaches zero within it, so the picture's whole is the same
    /// \[inferred\], save a physically correct light whose radius from its
    /// colour outreaches its end, which the volume would cut.
    pub(super) unsafe fn draw_lights(&mut self, cx: &mut Gen3Cx<'_>, accumulation_target: glow::NativeFramebuffer) {
        let (gl, frame, core) = (cx.gl, cx.frame, &mut *cx.core);
        let state = &mut *cx.state;
        let mut lit = Self::frame_lights_of(frame);
        let Some(targets) = state.targets.as_ref() else {
            return;
        };
        if lit.is_empty() {
            return;
        }
        // Lights that take away draw last.
        lit.sort_by_key(|light| light.blend() == H4LightBlend::Subtract);
        let (size, albedo, normal, depth_copy) = (
            targets.size,
            targets.albedo,
            targets.normal,
            targets.depth_copy,
        );
        let eye = h2_eye_position(&frame.camera);
        let view_projection = view_projection(&frame.camera);
        let exposure = match super::lighting::h4_frame_value("ps_view_exposure") {
            Some(H3Value::Floats(exposure)) => exposure[0],
            _ => return,
        };
        let depth_copy_framebuffer = targets.depth_copy_framebuffer;
        unsafe {
            state.begin_screen_lights(gl, size, accumulation_target, depth_copy_framebuffer);
            for light in &lit {
                let Some(pass) = frame
                    .preview
                    .h4.light_passes
                    .get(&(light.physical, light.technique()))
                    .cloned()
                else {
                    continue;
                };
                let Some(matrix) = screen_to_relative_world(
                    &view_projection,
                    size,
                    light.position,
                ) else {
                    continue;
                };
                let linked = state.with_program(core, gl, &pass, |_, this, program| {
                    gl.blend_equation(match light.blend() {
                        H4LightBlend::Add => glow::FUNC_ADD,
                        H4LightBlend::Subtract => glow::FUNC_REVERSE_SUBTRACT,
                    });
                    gl.use_program(Some(program.program));
                    let constants = light.constants(&matrix, eye, exposure);
                    for (name, ty) in &pass.uniforms {
                        let location = program.location(gl, name);
                        if location.is_none() {
                            continue;
                        }
                        if name == "ps_screen_space_light_constants" {
                            gl.uniform_4_f32_slice(location.as_ref(), constants.as_flattened());
                            continue;
                        }
                        if let Some(matrix) = h4_matrix(frame, name) {
                            match matrix.len() {
                                9 => gl.uniform_matrix_3_f32_slice(location.as_ref(), false, &matrix),
                                _ => gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &matrix),
                            }
                            continue;
                        }
                        // Its blend state is additive (1).
                        let value = if name == "ps_material_blend_constant" {
                            Some(H3Value::Floats(
                                super::submit::blend_constant(1),
                            ))
                        } else {
                            frame_value(frame, H3Dialect::Halo4, name, size, 0.0)
                        };
                        if let Some(value) = value {
                            set_value(gl, location.as_ref(), ty, &value);
                        }
                    }
                    let placeholder = this.placeholder;
                    Gen3State::bind_screen_light_textures(gl, &pass, program, |name| match name {
                        "depth_samplerTexture" => Some(Some(depth_copy)),
                        "albedo_samplerTexture" => Some(Some(albedo)),
                        "normal_samplerTexture" => Some(Some(normal)),
                        _ => Some(placeholder),
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
