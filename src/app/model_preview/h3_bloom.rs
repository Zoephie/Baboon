//! Halo 3's bloom, as `postprocess_bloom_buffer` builds it: the frame's bright
//! part at a quarter of its size, a quarter of that twice over, then each level
//! blurred and added to the one above. The final composite adds the top level
//! ahead of its tone curve.

use super::gl::target::{ColorTarget, Made, Sampling, Texels};
use eframe::glow::{self, HasContext};

use super::h3_draw::screen_program;

/// `globals\default.camera_fx_settings`.
const BLOOM_POINT: f32 = 1.2;
const BLOOM_INHERENT: f32 = 0.2;
const BLOOM_INTENSITY: f32 = 0.2;

/// What `bloom_point_exposure` comes to at no exposure, the tag's anti-bloom
/// having nothing to raise.
const BLOOM_POINT_EXPOSURE: f32 = 0.66943294;

const DOWNSAMPLE: i32 = 4;

const FRAGMENT: &str = "#version 410 core
uniform sampler2D source;
uniform sampler2D added;
uniform int mode;
uniform vec2 target_texel;
uniform vec2 source_texel;
uniform vec2 curve;
uniform float scale;
out vec4 color;
vec2 place;
vec3 at(vec2 offset) {
    return texture(source, place + offset * source_texel).rgb;
}
void main() {
    place = gl_FragCoord.xy * target_texel;
    vec3 result;
    if (mode == 0) {
        // `downsample_4x4_block_bloom`. Its LDR pass takes the larger of the two,
        // and clamped colors never pass this, so it has nothing to add.
        vec3 mean = (at(vec2(-1.0, -1.0)) + at(vec2(1.0, -1.0))
            + at(vec2(-1.0, 1.0)) + at(vec2(1.0, 1.0))) / 4.0 + 0.00000001;
        float intensity = dot(mean, vec3(0.299, 0.587, 0.114));
        result = mean * max(intensity * curve.y, intensity - curve.x) / intensity;
    } else if (mode == 1) {
        // `downsample_4x4_gaussian`
        result = 0.0625 * (at(vec2(-2.0, -2.0)) + at(vec2(2.0, -2.0))
                + at(vec2(-2.0, 2.0)) + at(vec2(2.0, 2.0)))
            + 0.125 * (at(vec2(0.0, -2.0)) + at(vec2(-2.0, 0.0))
                + at(vec2(2.0, 0.0)) + at(vec2(0.0, 2.0)))
            + 0.25 * at(vec2(0.0));
    } else if (mode == 2) {
        // `blur_11_horizontal`
        result = (10.0 * at(vec2(-4.1, 0.5)) + 120.0 * at(vec2(-2.3, 0.5))
            + 252.0 * at(vec2(-0.5, 0.5)) + 120.0 * at(vec2(1.3, 0.5))
            + 10.0 * at(vec2(3.1, 0.5))) / 512.0;
    } else if (mode == 3) {
        // `kernel_5`, with `gaussian_blur_fixed`'s kernel.
        result = (at(vec2(0.5, -4.1)) * 0.01953125 + at(vec2(0.5, -2.3)) * 0.234375
            + at(vec2(0.5, -0.5)) * 0.4921875 + at(vec2(0.5, 1.3)) * 0.234375
            + at(vec2(0.5, 3.1)) * 0.01953125) * scale;
    } else {
        // `bloom_add_alpha1` and `add`, whose alpha is 1 here.
        result = scale * at(vec2(0.0)) + texture(added, place).rgb;
    }
    color = vec4(result, 1.0);
}
";

/// `aux_small`, `aux_tiny` and `aux_mini`, and the second of each.
struct Levels {
    small: [ColorTarget; 2],
    tiny: [ColorTarget; 2],
    mini: [ColorTarget; 2],
    frame: [i32; 2],
}

impl Levels {
    unsafe fn new(gl: &glow::Context, frame: [i32; 2]) -> Option<Self> {
        let of = |level: i32| frame.map(|side| (side / DOWNSAMPLE.pow(level as u32)).max(1));
        let mut made = Made::new(gl);
        let mut level = |size| unsafe { made.color(size, Texels::RGBA16F, Sampling::LINEAR_CLAMPED) };
        let levels = Self {
            small: [level(of(1))?, level(of(1))?],
            tiny: [level(of(2))?, level(of(2))?],
            mini: [level(of(3))?, level(of(3))?],
            frame,
        };
        made.keep();
        Some(levels)
    }

    unsafe fn delete(&self, gl: &glow::Context) {
        for level in self.small.iter().chain(&self.tiny).chain(&self.mini) {
            unsafe { level.delete(gl) };
        }
    }
}

pub(super) struct H3Bloom {
    program: glow::NativeProgram,
    levels: Option<Levels>,
    /// What a pass adding nothing leaves bound, as `added` is sampled regardless.
    nothing: Option<glow::NativeTexture>,
}

impl H3Bloom {
    pub(super) unsafe fn new(gl: &glow::Context, nothing: Option<glow::NativeTexture>) -> Option<Self> {
        unsafe { Some(Self { program: screen_program(gl, FRAGMENT)?, levels: None, nothing }) }
    }

    /// One step of the chain, drawing `source` through `mode` into `target`.
    unsafe fn step(
        &self,
        gl: &glow::Context,
        target: &ColorTarget,
        mode: i32,
        source: (glow::NativeTexture, [i32; 2]),
        added: Option<glow::NativeTexture>,
        scale: f32,
    ) {
        let uniform = |name: &str| unsafe { gl.get_uniform_location(self.program, name) };
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            gl.viewport(0, 0, target.size[0], target.size[1]);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, added.or(self.nothing));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(source.0));
            gl.uniform_1_i32(uniform("mode").as_ref(), mode);
            gl.uniform_1_f32(uniform("scale").as_ref(), scale);
            let [width, height] = target.size.map(|side| 1.0 / side as f32);
            gl.uniform_2_f32(uniform("target_texel").as_ref(), width, height);
            let [width, height] = source.1.map(|side| 1.0 / side as f32);
            gl.uniform_2_f32(uniform("source_texel").as_ref(), width, height);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    /// `gaussian_blur_fixed`, `pair[1]` being its scratch.
    unsafe fn blur(&self, gl: &glow::Context, pair: &[ColorTarget; 2], scale: f32) {
        unsafe {
            self.step(gl, &pair[1], 2, (pair[0].texture, pair[0].size), None, 1.0);
            self.step(gl, &pair[0], 3, (pair[1].texture, pair[1].size), None, scale);
        }
    }

    /// What the final composite adds. Leaves `scene` bound, and the viewport and
    /// scissor as they were.
    pub(super) unsafe fn build(
        &mut self,
        gl: &glow::Context,
        vertex_array: glow::NativeVertexArray,
        scene: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        frame: [i32; 2],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            if self.levels.as_ref().is_none_or(|levels| levels.frame != frame) {
                if let Some(old) = self.levels.take() {
                    old.delete(gl);
                }
                self.levels = Levels::new(gl, frame);
            }
            let levels = self.levels.as_ref()?;
            let scissored = gl.is_enabled(glow::SCISSOR_TEST);
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
            gl.color_mask(true, true, true, true);
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(vertex_array));
            let uniform = |name: &str| gl.get_uniform_location(self.program, name);
            gl.uniform_1_i32(uniform("source").as_ref(), 0);
            gl.uniform_1_i32(uniform("added").as_ref(), 1);
            let point = BLOOM_POINT_EXPOSURE * BLOOM_POINT;
            gl.uniform_2_f32(uniform("curve").as_ref(), point, BLOOM_INHERENT);

            // The target reads by texel, but the downsample's taps lean on the
            // game's bilinear filter.
            gl.bind_texture(glow::TEXTURE_2D, Some(accumulation));
            for name in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, glow::LINEAR as i32);
            }

            let [small, _] = &levels.small;
            let ([tiny, _], [mini, _]) = (&levels.tiny, &levels.mini);
            self.step(gl, small, 0, (accumulation, frame), None, 1.0);
            self.step(gl, tiny, 1, (small.texture, small.size), None, 1.0);
            self.step(gl, mini, 1, (tiny.texture, tiny.size), None, 1.0);
            self.blur(gl, &levels.mini, BLOOM_INTENSITY);
            // Into the scratch of each: the level below still reads its own.
            let tiny_sum = &levels.tiny[1];
            self.step(gl, tiny_sum, 4, (tiny.texture, tiny.size), Some(mini.texture), BLOOM_INTENSITY);
            self.step(gl, tiny, 2, (tiny_sum.texture, tiny_sum.size), None, 1.0);
            self.step(gl, tiny_sum, 3, (tiny.texture, tiny.size), None, 1.0);
            let small_sum = &levels.small[1];
            self.step(gl, small_sum, 4, (small.texture, small.size), Some(tiny_sum.texture), BLOOM_INTENSITY);
            self.step(gl, small, 2, (small_sum.texture, small_sum.size), None, 1.0);
            self.step(gl, small_sum, 3, (small.texture, small.size), None, 1.0);

            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(accumulation));
            for name in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, glow::NEAREST as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene));
            gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
            if scissored {
                gl.enable(glow::SCISSOR_TEST);
            }
            Some(small_sum.texture)
        }
    }
}
