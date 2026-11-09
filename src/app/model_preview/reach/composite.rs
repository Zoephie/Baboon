//! Reach's final composite: the lit scene, its bloom and the screen noise
//! onto the frame, with no tone curve.

use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};
use eframe::glow::{self, HasContext};

use super::super::gen3::effects::screen::Gen3ScreenSettings;
use crate::app::model_preview::common::image::Sampler;
use super::super::gen3::render::Gen3Cx;
use super::super::gen3::pipeline::HALF_LENS;
use crate::app::model_preview::gl::pass_state::PassState;
use crate::app::model_preview::gl::screen::{premultiplied_over, screen_program};
use crate::app::model_preview::gl::texture::upload_image_linear;
use super::render::ReachGame;

impl ReachGame {
    /// Reach's `final_composite`: see [`REACH_COMPOSITE_FRAGMENT`].
    pub(super) unsafe fn draw_composite(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        depth: Option<glow::NativeTexture>,
        screen: Gen3ScreenSettings,
    ) {
        let (gl, core) = (cx.gl, &mut *cx.core);
        let state = &mut *cx.state;
        let noise = cx.frame.preview.reach.screen_noise.as_ref();
        unsafe {
            if self.noise.is_none()
                && let Some((image, texels, mips)) = noise
            {
                // Sampler 7: wrapped, point.
                let sampler = Sampler { address: [BitmapAddressMode::Wrap; 2], filter: BitmapFilterMode::Point };
                self.noise = upload_image_linear(gl, image, *texels, mips, Some(sampler));
            }
            if self.composite_program.is_none() {
                self.composite_program = screen_program(gl, REACH_COMPOSITE_FRAGMENT);
            }
            let Some(program) = self.composite_program else {
                return;
            };
            let at = |name: &str| gl.get_uniform_location(program, name);
            PassState::new()
                .use_program(program)
                .sampler(0, glow::TEXTURE_2D, Some(accumulation), at("accumulation"))
                .sampler(1, glow::TEXTURE_2D, bloom.or(core.black_flat), at("bloom"))
                .sampler(2, glow::TEXTURE_2D, depth, at("scene_depth"))
                .sampler(3, glow::TEXTURE_2D, self.noise.or(core.black_flat), at("noise"))
                .active_texture(glow::TEXTURE0)
                .apply(gl);
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
            premultiplied_over(state.empty_array(gl)).apply(gl);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            PassState::new().enable(glow::DEPTH_TEST).apply(gl);
        }
    }
}

/// `final_composite_base`'s factor on the lit target and on the bloom.
pub(crate) const REACH_DARK_MULTIPLIER: f32 = 8.0;

/// `rasterizer_globals`' `default bitmaps` element the final composite's
/// noise reads: `monochrome noise for screen effect` (its element label).
pub(crate) const REACH_SCREEN_NOISE_BITMAP: usize = 15;

/// `postprocess_player_view`'s `gamma` constant's `.z` (MCC
/// `sub_1408428C0`): half the screen effects' gamma, which the composite
/// raises the light to (`convert_to_gamma2_and_apply_color_adjustments`).
pub(in crate::app::model_preview) fn reach_composite_gamma(gamma: f32) -> f32 {
    gamma * 0.5
}

/// `noise_params` (MCC `sub_1408428C0` into `sub_140841DE0`): from the area
/// screen effects' bright and dark noise, `(2·bright, dark, 1 − bright, 0)`.
/// No noise leaves the picture as it was.
pub(in crate::app::model_preview) fn reach_noise_params(bright: f32, dark: f32) -> [f32; 4] {
    [bright + bright, dark, 1.0 - bright, 0.0]
}

/// The noise's offset for a frame (MCC `sub_1408428C0`): a half texel step
/// that turns over every other frame, plus `real_random`'s two draws
/// (`random`, each in [0, 1)), halved.
pub(in crate::app::model_preview) fn reach_noise_offset(frame: u32, random: [f32; 2]) -> [f32; 2] {
    let step = |frame: u32| (frame & 2) as f32 * 0.5;
    [(step(frame) + random[0]) * 0.5, (step(frame.wrapping_add(1)) + random[1]) * 0.5]
}

/// `final_composite`, as MCC runs it: the lit target and bloom brought back
/// up, `pow(x, gamma.z)` (0.5 at the default gamma, `convert_output_gamma2`),
/// the colour matrix, then `apply_noise`. The tone curve's call is commented
/// out in the source.
pub(in crate::app::model_preview) const REACH_COMPOSITE_FRAGMENT: &str = include_str!("shaders/composite.frag");

#[cfg(test)]
mod tests {
    use super::*;

    /// `gamma.z` is half the screen effects' gamma: 0.5, the square root, at rest.
    #[test]
    fn reachs_composite_raises_to_half_the_gamma() {
        assert_eq!(reach_composite_gamma(1.0), 0.5);
        assert_eq!(reach_composite_gamma(3.0), 1.5);
    }

    /// `apply_noise`'s `noise_params`: none leaves the colour alone; bright
    /// scales about 1, dark only adds.
    #[test]
    fn reachs_noise_scales_by_bright_and_adds_dark() {
        assert_eq!(reach_noise_params(0.0, 0.0), [0.0, 0.0, 1.0, 0.0]);
        let [scale, add, base, _] = reach_noise_params(0.25, 0.1);
        let apply = |noise: f32| (noise * scale + base, noise * add);
        assert_eq!(apply(0.0), (0.75, 0.0));
        assert_eq!(apply(1.0), (1.25, 0.1));
    }

    /// The offset steps half a texel every other frame, each axis a frame
    /// apart, plus half of each random draw.
    #[test]
    fn reachs_noise_offset_steps_by_frame_and_random() {
        assert_eq!(reach_noise_offset(0, [0.0, 0.0]), [0.0, 0.0]);
        assert_eq!(reach_noise_offset(1, [0.0, 0.0]), [0.0, 0.5]);
        assert_eq!(reach_noise_offset(2, [0.0, 0.0]), [0.5, 0.5]);
        assert_eq!(reach_noise_offset(3, [0.0, 0.0]), [0.5, 0.0]);
        assert_eq!(reach_noise_offset(0, [0.5, 0.25]), [0.25, 0.125]);
    }
}
