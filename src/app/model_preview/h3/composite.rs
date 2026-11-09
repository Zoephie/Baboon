//! Halo 3's and ODST's final composite: the lit scene and its bloom through
//! the tone curve onto the frame.

use eframe::glow::{self, HasContext};

use super::super::gen3::effects::screen::Gen3ScreenSettings;
use super::super::gen3::render::Gen3Cx;
use super::super::gen3::pipeline::HALF_LENS;
use crate::app::model_preview::gl::pass_state::PassState;
use crate::app::model_preview::gl::screen::{premultiplied_over, screen_program};
use super::render::H3Game;

/// `final_composite`'s `tone_curve_constants` at a white point of one: the
/// numbers in its comment are the curve before this scale.
const TONE_CURVE: [f32; 4] = {
    let scale = 1.493_801_6;
    [1.493_801_6 / scale, 1.004_149_4 * scale, 0.0, -0.15 * scale * scale * scale]
};

const COMPOSITE_FRAGMENT: &str = include_str!("shaders/composite.frag");

impl H3Game {
    /// Halo 3's `final_composite`: the lit scene and its bloom through the
    /// tone curve onto the frame.
    pub(super) unsafe fn draw_composite(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        screen: Gen3ScreenSettings,
    ) {
        let (gl, core) = (cx.gl, &mut *cx.core);
        let state = &mut *cx.state;
        unsafe {
            if self.composite_program.is_none() {
                self.composite_program = screen_program(gl, COMPOSITE_FRAGMENT);
            }
            let Some(program) = self.composite_program else {
                return;
            };
            let at = |name: &str| gl.get_uniform_location(program, name);
            PassState::new()
                .use_program(program)
                .sampler(0, glow::TEXTURE_2D, Some(accumulation), at("accumulation"))
                .sampler(1, glow::TEXTURE_2D, bloom.or(core.black_flat), at("bloom"))
                .active_texture(glow::TEXTURE0)
                .apply(gl);
            let location = gl.get_uniform_location(program, "tone_curve_constants");
            gl.uniform_4_f32_slice(location.as_ref(), &TONE_CURVE);
            let matrix = screen.color_matrix();
            let columns: [f32; 16] = std::array::from_fn(|at| matrix[at % 4][at / 4]);
            let location = gl.get_uniform_location(program, "hue_saturation");
            gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &columns);
            gl.uniform_1_f32(gl.get_uniform_location(program, "contrast").as_ref(), screen.gamma());
            gl.uniform_1_f32(gl.get_uniform_location(program, "exposure").as_ref(), screen.exposure());
            gl.uniform_1_f32(gl.get_uniform_location(program, "view_scale").as_ref(), screen.view_scale(HALF_LENS));
            premultiplied_over(state.empty_array(gl)).apply(gl);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            PassState::new().enable(glow::DEPTH_TEST).apply(gl);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tone_curve_takes_white_to_white() {
        let [limit, first, second, third] = TONE_CURVE;
        let toned = |value: f32| {
            let value = value.min(limit);
            ((value * third + second) * value + first) * value
        };
        assert!((limit - 1.0).abs() < 1e-5);
        assert!((toned(1.0) - 1.0).abs() < 1e-4, "{}", toned(1.0));
        assert!((toned(0.5) - 0.6875).abs() < 1e-3, "{}", toned(0.5));
        assert!((toned(3.0) - 1.0).abs() < 1e-4, "{}", toned(3.0));
    }
}
