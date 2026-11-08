//! Halo 3's and ODST's final composite: the lit scene and its bloom through
//! the tone curve onto the frame.

use eframe::glow::{self, HasContext};

use super::super::gen3::effects::screen::H3ScreenSettings;
use super::super::gen3::render::Gen3Cx;
use super::super::gen3::pipeline::HALF_LENS;
use crate::app::model_preview::gl::screen::screen_program;
use super::render::H3Game;

/// `final_composite`'s `tone_curve_constants` at a white point of one: the
/// numbers in its comment are the curve before this scale.
const TONE_CURVE: [f32; 4] = {
    let scale = 1.493_801_6;
    [1.493_801_6 / scale, 1.004_149_4 * scale, 0.0, -0.15 * scale * scale * scale]
};

const COMPOSITE_FRAGMENT: &str = "#version 410 core
uniform sampler2D accumulation;
uniform sampler2D bloom;
uniform vec4 tone_curve_constants;
uniform mat4 hue_saturation;
uniform float contrast;
uniform float exposure;
uniform float view_scale;
out vec4 color;
void main() {
    vec2 place = gl_FragCoord.xy / vec2(textureSize(accumulation, 0));
    // A wider lens frames the same picture smaller, about the middle.
    place = (place - 0.5) * view_scale + 0.5;
    vec4 accumulated = texture(accumulation, place);
    if (place.x < 0.0 || place.x > 1.0 || place.y < 0.0 || place.y > 1.0) {
        accumulated = vec4(0.0);
    }
    vec3 blend = (accumulated.rgb + texture(bloom, place).rgb) * exposure;
    blend = (vec4(blend, 1.0) * hue_saturation).rgb;
    float luminance = dot(blend, vec3(0.333));
    if (luminance > 0.0) {
        blend *= pow(luminance, contrast) / luminance;
    }
    vec3 clamped = min(blend, tone_curve_constants.xxx);
    vec3 toned = ((clamped * tone_curve_constants.w + tone_curve_constants.z) * clamped
        + tone_curve_constants.y) * clamped;
    // The game's surfaces are sRGB, so this happens as it writes.
    toned = clamp(toned, 0.0, 1.0);
    color.rgb = mix(12.92 * toned, 1.055 * pow(toned, vec3(1.0 / 2.4)) - 0.055,
        step(0.0031308, toned));
    color.a = accumulated.a;
}
";

impl H3Game {
    /// Halo 3's `final_composite`: the lit scene and its bloom through the
    /// tone curve onto the frame.
    pub(super) unsafe fn draw_composite(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        screen: H3ScreenSettings,
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
            gl.use_program(Some(program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(accumulation));
            let location = gl.get_uniform_location(program, "accumulation");
            gl.uniform_1_i32(location.as_ref(), 0);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, bloom.or(core.black_flat));
            let location = gl.get_uniform_location(program, "bloom");
            gl.uniform_1_i32(location.as_ref(), 1);
            gl.active_texture(glow::TEXTURE0);
            let location = gl.get_uniform_location(program, "tone_curve_constants");
            gl.uniform_4_f32_slice(location.as_ref(), &TONE_CURVE);
            let matrix = screen.color_matrix();
            let columns: [f32; 16] = std::array::from_fn(|at| matrix[at % 4][at / 4]);
            let location = gl.get_uniform_location(program, "hue_saturation");
            gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &columns);
            gl.uniform_1_f32(gl.get_uniform_location(program, "contrast").as_ref(), screen.gamma());
            gl.uniform_1_f32(gl.get_uniform_location(program, "exposure").as_ref(), screen.exposure());
            gl.uniform_1_f32(gl.get_uniform_location(program, "view_scale").as_ref(), screen.view_scale(HALF_LENS));
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            // The accumulation is premultiplied: what adds light covers nothing.
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(state.empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.enable(glow::DEPTH_TEST);
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
