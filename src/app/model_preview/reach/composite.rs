//! Reach's final composite: the lit scene, its bloom and the screen noise
//! onto the frame, with no tone curve.

use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};
use eframe::glow::{self, HasContext};

use super::super::gen3::screen_effects::H3ScreenSettings;
use crate::app::model_preview::common::image::H3Sampler;
use super::super::gen3::render::Gen3Cx;
use super::super::gen3::pipeline::HALF_LENS;
use crate::app::model_preview::gl::screen::screen_program;
use crate::app::model_preview::gl::texture::upload_h2_image_linear;
use super::lighting::{
    REACH_COMPOSITE_FRAGMENT, REACH_DARK_MULTIPLIER, reach_composite_gamma, reach_noise_offset, reach_noise_params,
};
use super::render::ReachGame;

impl ReachGame {
    /// Reach's `final_composite`: see [`REACH_COMPOSITE_FRAGMENT`].
    pub(super) unsafe fn draw_composite(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        depth: Option<glow::NativeTexture>,
        screen: H3ScreenSettings,
    ) {
        let (gl, core) = (cx.gl, &mut *cx.core);
        let state = &mut *cx.state;
        let noise = cx.frame.preview.reach.screen_noise.as_ref();
        unsafe {
            if self.noise.is_none()
                && let Some((image, texels, mips)) = noise
            {
                // Sampler 7: wrapped, point.
                let sampler = H3Sampler { address: [BitmapAddressMode::Wrap; 2], filter: BitmapFilterMode::Point };
                self.noise = upload_h2_image_linear(gl, image, *texels, mips, Some(sampler));
            }
            if self.composite_program.is_none() {
                self.composite_program = screen_program(gl, REACH_COMPOSITE_FRAGMENT);
            }
            let Some(program) = self.composite_program else {
                return;
            };
            gl.use_program(Some(program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(accumulation));
            gl.uniform_1_i32(gl.get_uniform_location(program, "accumulation").as_ref(), 0);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, bloom.or(core.black_flat));
            gl.uniform_1_i32(gl.get_uniform_location(program, "bloom").as_ref(), 1);
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, depth);
            gl.uniform_1_i32(gl.get_uniform_location(program, "scene_depth").as_ref(), 2);
            gl.active_texture(glow::TEXTURE3);
            gl.bind_texture(glow::TEXTURE_2D, self.noise.or(core.black_flat));
            gl.uniform_1_i32(gl.get_uniform_location(program, "noise").as_ref(), 3);
            gl.active_texture(glow::TEXTURE0);
            let matrix = screen.color_matrix();
            let columns: [f32; 16] = std::array::from_fn(|at| matrix[at % 4][at / 4]);
            gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(program, "hue_saturation").as_ref(), false, &columns);
            let gamma = reach_composite_gamma(screen.gamma());
            gl.uniform_1_f32(gl.get_uniform_location(program, "gamma").as_ref(), gamma);
            // With no map to read, no noise.
            let [bright, dark] = if self.noise.is_some() { screen.noise() } else { [0.0; 2] };
            let params = reach_noise_params(bright, dark);
            gl.uniform_4_f32_slice(gl.get_uniform_location(program, "noise_params").as_ref(), &params);
            // `real_random`'s two draws, new each frame.
            self.noise_frame = self.noise_frame.wrapping_add(1);
            let frame_number = self.noise_frame as f32;
            let random = [0, 1].map(|salt| super::super::gen3::shield::real_random(frame_number, salt));
            let [x, y] = reach_noise_offset(self.noise_frame, random);
            gl.uniform_2_f32(gl.get_uniform_location(program, "noise_offset").as_ref(), x, y);
            gl.uniform_1_f32(gl.get_uniform_location(program, "dark_multiplier").as_ref(), REACH_DARK_MULTIPLIER);
            gl.uniform_1_f32(gl.get_uniform_location(program, "view_scale").as_ref(), screen.view_scale(HALF_LENS));
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(state.empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.enable(glow::DEPTH_TEST);
        }
    }
}
