//! Halo 4's bloom, as MCC's `sub_1408C0540` builds it (X360
//! `GenerateBloomSource` 0x82B0FD30, `postprocess_bloom_buffer` 0x82B06798;
//! `docs/h4-preview-research/lighting.md` §2.4). The pass graph is Reach's,
//! but each pass is Halo 4's own source (`explicit_shaders\postprocess\*.fx`,
//! `copy\add.fx`) and nothing stores light scaled: Reach's ×8 and ÷32 are
//! gone. The final composite adds the top level unscaled.
//!
//! Carried over from Reach, not read for Halo 4: the surfaces' rounding
//! (`floor(side / divisor + 0.5)`, no fixed sizes; O8) and the pixel size a
//! pass steps by, the source's.

use eframe::glow::{self, HasContext};

use super::h3_draw::screen_program;

/// `globals\defaults\default.camera_fx_settings`: `bloom_highlight`,
/// `bloom_inherent`, `bloom_intensity` and the three colours. The tag
/// predates `bloom_self_illum`, which reads 0 [inferred: zero fill].
const HIGHLIGHT: f32 = 0.1;
const INHERENT: f32 = 0.0;
const SELF_ILLUM: f32 = 0.0;
const INTENSITY: f32 = 0.3;
const LARGE_COLOR: [f32; 3] = [0.0; 3];
const MEDIUM_COLOR: [f32; 3] = [0.240_627_5; 3];
const SMALL_COLOR: [f32; 3] = [1.0; 3];

/// One explicit shader of the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// 29 `downsample_2x2`.
    Downsample2x2 = 0,
    /// 159 `downsample_block_bloom_2x2`, technique 1: one colour grading
    /// lookup, highlight, self-illumination and inherent bloom, intensity
    /// out in alpha.
    BloomCurve = 1,
    /// 49 `downsample_4x4_gaussian`.
    Gaussian4x4 = 2,
    /// 11 `blur_11_horizontal`.
    BlurHorizontal = 3,
    /// 34 `kernel_5`.
    Kernel5 = 4,
    /// 66 `bloom_add_alpha1`.
    AddAlpha1 = 5,
    /// 10 `copy\add`.
    Add = 6,
}

/// Every pass at once, picked by `pass`. `source_texel` is the game's
/// `ps_pixel_size`; offsets in texture `v` turn over for GL's upward `t`.
const FRAGMENT: &str = "#version 410 core
uniform sampler2D source;
uniform sampler2D added;
uniform sampler3D grading;
uniform bool graded;
uniform vec3 grading_scale;
uniform vec3 grading_offset;
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
        color = texture(source, uv);
    } else if (pass == 1) {
        vec4 s = texture(source, uv);
        s.rgb = clamp(s.rgb, 0.0, 1.0);
        s.a = min(s.a, 1.0 / 32.0);
        // DARK_COLOR_MULTIPLIER, `ps_exposure.g`.
        s.rgb *= 8.0;
        float intensity = graded
            ? texture(grading, s.rgb * grading_scale + grading_offset).a
            : dot(s.rgb, vec3(0.3086, 0.6094, 0.0820));
        float bloom = scale.x * intensity + scale.z * s.a + scale.y;
        color = vec4(s.rgb * bloom, intensity);
    } else if (pass == 2) {
        vec4 a = vec4(0.00000001);
        for (int y = -3; y <= 3; y += 2) {
            for (int x = -3; x <= 3; x += 2) {
                a += t(vec2(x, -y));
            }
        }
        color = a * (1.0 / 16.0);
    } else if (pass == 3) {
        // `tex2DOffsetLinear(.., -0.5, 0.5)` on each tap.
        color = 10.0 / 512.0 * t(vec2(-4.1 - 0.5, -0.5)) + 10.0 / 512.0 * t(vec2(3.1 - 0.5, -0.5))
            + 120.0 / 512.0 * t(vec2(1.3 - 0.5, -0.5)) + 120.0 / 512.0 * t(vec2(-2.3 - 0.5, -0.5))
            + 252.0 / 512.0 * t(vec2(-0.5, -0.5));
    } else if (pass == 4) {
        // `ps_kernel` (X360 0x822658A8): x 0.5, y -4.1 .. 3.1.
        vec4 a = 0.01953125 * t(vec2(0.5, 4.1)) + 0.234375 * t(vec2(0.5, 2.3)) + 0.4921875 * t(vec2(0.5, 0.5))
            + 0.234375 * t(vec2(0.5, -1.3)) + 0.01953125 * t(vec2(0.5, -3.1));
        color = a * scale;
    } else if (pass == 5) {
        vec4 original = texture(source, uv);
        vec4 add = texture(added, uv);
        color = vec4(scale.rgb, dot(scale.rgb, vec3(0.299, 0.587, 0.114))) * original + add;
    } else {
        vec4 source_color = texture(source, uv);
        vec4 add = texture(added, uv);
        color = vec4(scale.rgb * source_color.rgb * add.a + add.rgb, add.a);
    }
}
";

struct Surface {
    framebuffer: glow::NativeFramebuffer,
    color: glow::NativeTexture,
    size: [i32; 2],
}

impl Surface {
    /// DXGI 10, RGBA16F, as the aux surfaces (MCC table 0x1424A6000).
    unsafe fn new(gl: &glow::Context, size: [i32; 2]) -> Option<Self> {
        unsafe {
            let color = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(color));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA16F as i32,
                size[0],
                size[1],
                0,
                glow::RGBA,
                glow::FLOAT,
                glow::PixelUnpackData::Slice(None),
            );
            for (name, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
            }
            let framebuffer = gl.create_framebuffer().ok()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(color), 0);
            Some(Self { framebuffer, color, size })
        }
    }

    unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_texture(self.color);
        }
    }
}

fn divided(frame: [i32; 2], divisor: f32) -> [i32; 2] {
    frame.map(|side| ((side as f32 / divisor + 0.5).floor() as i32).max(1))
}

/// MCC's high-resolution bloom scale: `min(1, 921600 / (w·h))` once the
/// frame is wider than 1280 or taller than 720 (`use_hdr_bloom_scaling`, off
/// by default, would replace it).
fn resolution_scale([width, height]: [i32; 2]) -> f32 {
    if width > 1280 || height > 720 { (921_600.0 / (width as f32 * height as f32)).min(1.0) } else { 1.0 }
}

/// A colour scaled by the intensity and the resolution, as the chain hands
/// its passes.
fn scaled(color: [f32; 3], frame: [i32; 2]) -> [f32; 4] {
    let factor = INTENSITY * resolution_scale(frame);
    [color[0] * factor, color[1] * factor, color[2] * factor, 1.0]
}

/// `accum_LDR_temp` (/2), then `aux_small`, `aux_tiny`, `aux_mini` (/4, /16,
/// /64) and their seconds.
struct Surfaces {
    temp: Surface,
    small: [Surface; 2],
    tiny: [Surface; 2],
    mini: [Surface; 2],
    frame: [i32; 2],
}

impl Surfaces {
    unsafe fn new(gl: &glow::Context, frame: [i32; 2]) -> Option<Self> {
        let pair = |divisor| unsafe {
            let size = divided(frame, divisor);
            Some([Surface::new(gl, size)?, Surface::new(gl, size)?])
        };
        unsafe {
            Some(Self {
                temp: Surface::new(gl, divided(frame, 2.0))?,
                small: pair(4.0)?,
                tiny: pair(16.0)?,
                mini: pair(64.0)?,
                frame,
            })
        }
    }

    unsafe fn delete(&self, gl: &glow::Context) {
        let all = std::iter::once(&self.temp).chain(&self.small).chain(&self.tiny).chain(&self.mini);
        for surface in all {
            unsafe { surface.delete(gl) };
        }
    }
}

pub(super) struct H4Bloom {
    program: glow::NativeProgram,
    surfaces: Option<Surfaces>,
    nothing: Option<glow::NativeTexture>,
}

/// The colour grading lookup shader 159 reads its intensity from, and how.
pub(super) struct H4BloomGrading {
    pub(super) texture: glow::NativeTexture,
    pub(super) scale: [f32; 3],
    pub(super) offset: [f32; 3],
}

impl H4Bloom {
    pub(super) unsafe fn new(gl: &glow::Context, nothing: Option<glow::NativeTexture>) -> Option<Self> {
        unsafe { Some(Self { program: screen_program(gl, FRAGMENT)?, surfaces: None, nothing }) }
    }

    unsafe fn step(
        &self,
        gl: &glow::Context,
        target: &Surface,
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

    /// `gaussian_blur_fixed`: across into `scratch`, then back down scaled.
    unsafe fn blur(&self, gl: &glow::Context, surface: &Surface, scratch: &Surface, scale: [f32; 4]) {
        unsafe {
            self.step(gl, scratch, Pass::BlurHorizontal, (surface.color, surface.size), None, [1.0; 4]);
            self.step(gl, surface, Pass::Kernel5, (scratch.color, scratch.size), None, scale);
        }
    }

    /// What the final composite adds. Leaves `scene` bound, and the viewport
    /// and scissor as they were.
    pub(super) unsafe fn build(
        &mut self,
        gl: &glow::Context,
        vertex_array: glow::NativeVertexArray,
        scene: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        frame: [i32; 2],
        grading: Option<&H4BloomGrading>,
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
            gl.uniform_1_i32(uniform("grading").as_ref(), 2);
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_3D, grading.map(|grading| grading.texture));
            gl.active_texture(glow::TEXTURE0);
            gl.uniform_1_i32(uniform("graded").as_ref(), i32::from(grading.is_some()));
            if let Some(grading) = grading {
                gl.uniform_3_f32_slice(uniform("grading_scale").as_ref(), &grading.scale);
                gl.uniform_3_f32_slice(uniform("grading_offset").as_ref(), &grading.offset);
            }
            // The lit target is read by texel elsewhere; this first tap leans
            // on bilinear filtering.
            gl.bind_texture(glow::TEXTURE_2D, Some(accumulation));
            for name in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, glow::LINEAR as i32);
            }

            let ones = [1.0; 4];
            let [small, small2] = &surfaces.small;
            let [tiny, tiny2] = &surfaces.tiny;
            let [mini, mini2] = &surfaces.mini;
            self.step(gl, &surfaces.temp, Pass::Downsample2x2, (accumulation, frame), None, [1.0, 1.0, 0.0, 0.0]);
            let temp = (surfaces.temp.color, surfaces.temp.size);
            self.step(gl, small, Pass::BloomCurve, temp, None, [HIGHLIGHT, INHERENT, SELF_ILLUM, 1.0]);
            self.step(gl, tiny, Pass::Gaussian4x4, (small.color, small.size), None, ones);
            self.step(gl, mini, Pass::Gaussian4x4, (tiny.color, tiny.size), None, ones);
            self.blur(gl, mini, mini2, scaled(LARGE_COLOR, frame));
            self.step(gl, tiny2, Pass::AddAlpha1, (tiny.color, tiny.size), Some(mini.color), scaled(MEDIUM_COLOR, frame));
            self.blur(gl, tiny2, tiny, ones);
            self.step(gl, small2, Pass::Add, (small.color, small.size), Some(tiny2.color), scaled(SMALL_COLOR, frame));
            self.blur(gl, small2, small, ones);

            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_3D, None);
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
            Some(small2.color)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_frames_scale_the_bloom_down_by_their_area() {
        assert_eq!(resolution_scale([1280, 720]), 1.0);
        assert!((resolution_scale([1920, 1080]) - 0.444_444).abs() < 1e-5);
        assert_eq!(resolution_scale([1300, 400]), 1.0, "past 1280 wide but under 921,600 texels");
        assert_eq!(scaled(LARGE_COLOR, [1000, 700]), [0.0, 0.0, 0.0, 1.0]);
        assert!((scaled(MEDIUM_COLOR, [1000, 700])[0] - 0.072_188_25).abs() < 1e-7);
        assert!((scaled(SMALL_COLOR, [1920, 1080])[0] - 0.133_333).abs() < 1e-5);
    }

    #[test]
    fn the_surfaces_are_the_frame_halved_then_quartered() {
        assert_eq!(divided([1000, 700], 2.0), [500, 350]);
        assert_eq!(divided([1000, 700], 64.0), [16, 11]);
        assert_eq!(divided([10, 10], 64.0), [1, 1], "never empty");
    }
}
