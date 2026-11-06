//! Reach's bloom, as MCC's `postprocess_player_view` (`sub_1408428C0`) and
//! `sub_140841B00` build it: the lit frame halved, its bright part taken by the
//! bloom curve, that a quarter and a sixteenth again, each level blurred and
//! added into the one above. Every pass stores light down three stops and
//! clamps where the game's shaders do. The final composite adds the top level
//! at eight times. See `docs/reach-preview-research/bloom.md`.

use super::gl::target::{ColorTarget, Made, Sampling, Texels};
use eframe::glow::{self, HasContext};

use super::h3_draw::screen_program;

/// `globals\defaults\default.camera_fx_settings`, which every bloom parameter
/// falls through to with no level: `bloom_highlight`, `bloom_inherent`,
/// `bloom_intensity` and the three colours.
const HIGHLIGHT: f32 = 0.1;
const INHERENT: f32 = 0.0;
const INTENSITY: f32 = 0.3;
const LARGE_COLOR: [f32; 3] = [0.0; 3];
const MEDIUM_COLOR: [f32; 3] = [0.240_627_5; 3];
const SMALL_COLOR: [f32; 3] = [1.0; 3];

/// One explicit shader of the chain, by its `rasterizer_globals` index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// 29 `downsample_2x2`.
    Downsample2x2 = 0,
    /// 159 `downsample_2x2_block_bloom`.
    BloomCurve = 1,
    /// 51 `copy`.
    Copy = 2,
    /// 49 `downsample_4x4_gaussian`.
    Gaussian4x4 = 3,
    /// 11 `blur_11_horizontal`.
    BlurHorizontal = 4,
    /// 34 `kernel_5`.
    Kernel5 = 5,
    /// 66 `bloom_add_alpha1`.
    AddAlpha1 = 6,
    /// 10 `add`.
    Add = 7,
}

/// Every pass at once, picked by `pass`. `ps` is the source's texel, as the
/// game's `pixel_size`; the blur offsets' y is turned over for GL's upward t.
const FRAGMENT: &str = "#version 410 core
uniform sampler2D source;
uniform sampler2D added;
uniform int pass;
uniform vec2 target_size;
uniform vec2 source_texel;
uniform vec4 scale;
out vec4 color;
vec2 uv;
vec4 t(vec2 offset) {
    return texture(source, uv + offset * source_texel);
}
void main() {
    uv = gl_FragCoord.xy / target_size;
    if (pass == 0) {
        color = vec4(texture(source, uv).rgb, 1.0);
    } else if (pass == 1) {
        vec4 s = clamp(texture(source, uv), 0.0, 1.0);
        vec3 c = s.rgb * 8.0;
        float intensity = dot(c, vec3(0.299, 0.587, 0.114));
        vec3 b = c * (scale.x * intensity + scale.y);
        color = min(vec4(b, intensity) * 0.25, vec4(8.0));
    } else if (pass == 2) {
        color = texture(source, uv);
    } else if (pass == 3) {
        vec4 a = vec4(0.0);
        for (int y = -3; y <= 3; y += 2) {
            for (int x = -3; x <= 3; x += 2) {
                a += t(vec2(x, y)) * 8.0;
            }
        }
        a += 1e-8;
        color = min(a * 0.25, vec4(8.0)) * 0.03125;
    } else if (pass == 4) {
        vec4 a = 80.0 * t(vec2(-4.1, -0.5)) + 960.0 * t(vec2(-2.3, -0.5)) + 2016.0 * t(vec2(-0.5, -0.5))
            + 960.0 * t(vec2(1.3, -0.5)) + 80.0 * t(vec2(3.1, -0.5));
        color = min(a * 0.0078125, vec4(8.0)) * 0.03125;
    } else if (pass == 5) {
        vec4 a = 8.0 * (0.01953125 * t(vec2(0.5, 4.1)) + 0.234375 * t(vec2(0.5, 2.3))
            + 0.4921875 * t(vec2(0.5, 0.5)) + 0.234375 * t(vec2(0.5, -1.3))
            + 0.01953125 * t(vec2(0.5, -3.1)));
        color = min(a * scale * 4.0, vec4(8.0)) * 0.03125;
    } else if (pass == 6) {
        vec3 rgb = scale.rgb * 8.0 * texture(source, uv).rgb + 8.0 * texture(added, uv).rgb;
        color = min(vec4(rgb, 1.0) * 4.0, vec4(8.0)) / 32.0;
    } else {
        vec4 b = texture(added, uv);
        vec3 rgb = scale.rgb * 8.0 * texture(source, uv).rgb * (8.0 * b.a) + 8.0 * b.rgb;
        color = min(vec4(rgb * 4.0, b.a * 32.0), vec4(8.0)) * 0.03125;
    }
}
";

/// `sub_1407C4D90 || sub_1407C4E10`: a frame large enough that the small
/// surfaces take fixed sizes.
fn high_resolution([width, height]: [i32; 2]) -> bool {
    if width > height { height > 720 } else { width > 1152 }
}

/// `sub_140849480`'s `floor(side / divisor + 0.5)`, at least a texel.
fn divided(frame: [i32; 2], divisor: f32) -> [i32; 2] {
    frame.map(|side| ((side as f32 / divisor + 0.5).floor() as i32).max(1))
}

/// The surfaces' sizes, `sub_140848A60` fixing the small three when the
/// frame is high resolution: `accum_LDR_temp`, `aux_bloom_tmp`, then
/// `aux_small`, `aux_tiny` and `aux_mini` (and each one's second).
fn surface_sizes(frame: [i32; 2]) -> [[i32; 2]; 5] {
    let fixed = high_resolution(frame);
    let pick = |size: [i32; 2], divisor: f32| if fixed { size } else { divided(frame, divisor) };
    [divided(frame, 2.0), divided(frame, 4.0), pick([288, 180], 4.0), pick([72, 45], 16.0), pick([18, 11], 64.0)]
}

/// The surfaces of the chain, numbered as the game's.
struct Surfaces {
    temp: ColorTarget,
    bloom_temp: ColorTarget,
    small: [ColorTarget; 2],
    tiny: [ColorTarget; 2],
    mini: [ColorTarget; 2],
    frame: [i32; 2],
}

impl Surfaces {
    unsafe fn new(gl: &glow::Context, frame: [i32; 2]) -> Option<Self> {
        let [temp, bloom_temp, small, tiny, mini] = surface_sizes(frame);
        let mut made = Made::new(gl);
        let mut surface = |size| unsafe { made.color(size, Texels::RGBA16F, Sampling::LINEAR_CLAMPED) };
        let surfaces = Self {
            temp: surface(temp)?,
            bloom_temp: surface(bloom_temp)?,
            small: [surface(small)?, surface(small)?],
            tiny: [surface(tiny)?, surface(tiny)?],
            mini: [surface(mini)?, surface(mini)?],
            frame,
        };
        made.keep();
        Some(surfaces)
    }

    unsafe fn delete(&self, gl: &glow::Context) {
        let pairs = self.small.iter().chain(&self.tiny).chain(&self.mini);
        for surface in [&self.temp, &self.bloom_temp].into_iter().chain(pairs) {
            unsafe { surface.delete(gl) };
        }
    }
}

pub(super) struct ReachBloom {
    program: glow::NativeProgram,
    surfaces: Option<Surfaces>,
    /// What a pass adding nothing leaves bound, as `added` is sampled regardless.
    nothing: Option<glow::NativeTexture>,
}

/// A colour scaled by the intensity, as `sub_140841B00` hands its passes.
fn scaled(color: [f32; 3]) -> [f32; 4] {
    [color[0] * INTENSITY, color[1] * INTENSITY, color[2] * INTENSITY, 1.0]
}

impl ReachBloom {
    pub(super) unsafe fn new(gl: &glow::Context, nothing: Option<glow::NativeTexture>) -> Option<Self> {
        unsafe { Some(Self { program: screen_program(gl, FRAGMENT)?, surfaces: None, nothing }) }
    }

    /// `c_screen_postprocess::copy` and `apply_binary_op_ex`: `source` (and
    /// `added`) through `pass` into `target`, `pixel_size` the source's.
    unsafe fn step(
        &self,
        gl: &glow::Context,
        target: &ColorTarget,
        pass: Pass,
        source: (glow::NativeTexture, [i32; 2]),
        added: Option<glow::NativeTexture>,
        scale: [f32; 4],
    ) {
        let uniform = |name: &str| unsafe { gl.get_uniform_location(self.program, name) };
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            gl.viewport(0, 0, target.size[0], target.size[1]);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, added.or(self.nothing));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(source.0));
            gl.uniform_1_i32(uniform("pass").as_ref(), pass as i32);
            gl.uniform_4_f32_slice(uniform("scale").as_ref(), &scale);
            gl.uniform_2_f32(uniform("target_size").as_ref(), target.size[0] as f32, target.size[1] as f32);
            let [width, height] = source.1.map(|side| 1.0 / side as f32);
            gl.uniform_2_f32(uniform("source_texel").as_ref(), width, height);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    /// `gaussian_blur_fixed`: `surface` blurred across into `scratch` and
    /// back down, scaled.
    unsafe fn blur(&self, gl: &glow::Context, surface: &ColorTarget, scratch: &ColorTarget, scale: [f32; 4]) {
        unsafe {
            self.step(gl, scratch, Pass::BlurHorizontal, (surface.texture, surface.size), None, [1.0; 4]);
            self.step(gl, surface, Pass::Kernel5, (scratch.texture, scratch.size), None, scale);
        }
    }

    /// What the final composite adds: `aux_small2`. Leaves `scene` bound, and
    /// the viewport and scissor as they were.
    pub(super) unsafe fn build(
        &mut self,
        gl: &glow::Context,
        vertex_array: glow::NativeVertexArray,
        scene: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        frame: [i32; 2],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            if self.surfaces.as_ref().is_none_or(|surfaces| surfaces.frame != frame) {
                if let Some(old) = self.surfaces.take() {
                    old.delete(gl);
                }
                self.surfaces = Surfaces::new(gl, frame);
            }
            let surfaces = self.surfaces.as_ref()?;
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
            // The lit target is read by texel elsewhere; the first pass's tap
            // leans on bilinear filtering.
            gl.bind_texture(glow::TEXTURE_2D, Some(accumulation));
            for name in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, glow::LINEAR as i32);
            }

            let ones = [1.0; 4];
            let curve = [HIGHLIGHT, INHERENT, 1.0, 1.0];
            let [small, small2] = &surfaces.small;
            let [tiny, tiny2] = &surfaces.tiny;
            let [mini, mini2] = &surfaces.mini;
            self.step(gl, &surfaces.temp, Pass::Downsample2x2, (accumulation, frame), None, [1.0, 1.0, 0.0, 0.0]);
            let temp = (surfaces.temp.texture, surfaces.temp.size);
            if high_resolution(frame) {
                self.step(gl, &surfaces.bloom_temp, Pass::BloomCurve, temp, None, curve);
                let bloom_temp = (surfaces.bloom_temp.texture, surfaces.bloom_temp.size);
                self.step(gl, small, Pass::Copy, bloom_temp, None, ones);
            } else {
                self.step(gl, small, Pass::BloomCurve, temp, None, curve);
            }
            self.step(gl, tiny, Pass::Gaussian4x4, (small.texture, small.size), None, ones);
            self.step(gl, mini, Pass::Gaussian4x4, (tiny.texture, tiny.size), None, ones);
            self.blur(gl, mini, mini2, scaled(LARGE_COLOR));
            self.step(gl, tiny2, Pass::AddAlpha1, (tiny.texture, tiny.size), Some(mini.texture), scaled(MEDIUM_COLOR));
            self.blur(gl, tiny2, tiny, ones);
            self.step(gl, small2, Pass::Add, (small.texture, small.size), Some(tiny2.texture), scaled(SMALL_COLOR));
            self.blur(gl, small2, small, ones);

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
            Some(small2.texture)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_above_720_lines_takes_the_fixed_small_surfaces() {
        assert!(high_resolution([1920, 1080]));
        assert!(!high_resolution([1280, 720]), "720 itself is not above");
        assert!(high_resolution([1200, 1300]), "portrait goes by width");
        assert!(!high_resolution([1100, 1300]));
        assert_eq!(surface_sizes([1920, 1080]), [[960, 540], [480, 270], [288, 180], [72, 45], [18, 11]]);
        assert_eq!(surface_sizes([1000, 700]), [[500, 350], [250, 175], [250, 175], [63, 44], [16, 11]]);
    }

    #[test]
    fn the_default_colours_scale_by_the_intensity() {
        assert_eq!(scaled(LARGE_COLOR), [0.0, 0.0, 0.0, 1.0]);
        assert!((scaled(MEDIUM_COLOR)[0] - 0.072_188_25).abs() < 1e-7);
        assert_eq!(scaled(SMALL_COLOR), [0.3, 0.3, 0.3, 1.0]);
    }
}
