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

use super::gl::chain::ScreenChain;
use super::gl::target::{ColorTarget, Made, Sampling, Texels};
use eframe::glow::{self, HasContext};

use super::h3_draw::screen_program;

/// What a chain runs on: a camera fx's bloom fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct H4BloomSettings {
    highlight: f32,
    inherent: f32,
    self_illum: f32,
    intensity: f32,
    large: [f32; 3],
    medium: [f32; 3],
    small: [f32; 3],
    /// `GenerateBloomSource`'s entry 4, "no color grading, no self-illum,
    /// with alpha preservation (used for Hologram)": intensity by luminance,
    /// the source's alpha kept, and the chain's output read before its last
    /// `kernel_5` (MCC hands the final composite surface 19, which then holds
    /// `blur_11_horizontal` of the result; `holograms.md` §3).
    hologram: bool,
}

/// The preview camera fx's bloom (`h4::lighting::H4_PREVIEW_STOPS` says
/// whose): `bloom_highlight`, `bloom_inherent`, `bloom_self_illum`,
/// `bloom_intensity` and the three colours of `m80_cam_fx_global`.
pub(super) const VIEW_BLOOM: H4BloomSettings = H4BloomSettings {
    highlight: 0.05,
    inherent: 0.0,
    self_illum: 5.0,
    intensity: 3.0,
    large: [1.0; 3],
    medium: [1.0; 3],
    small: [1.0; 3],
    hologram: false,
};

/// `rasterizer_globals`' `Hologram Camera FX`,
/// `globals\defaults\hologram.camera_fx_settings`.
const HOLOGRAM_BLOOM: H4BloomSettings = H4BloomSettings {
    highlight: 0.1,
    inherent: 0.0,
    self_illum: 0.0,
    intensity: 0.7,
    large: [0.0; 3],
    medium: [0.1, 0.2, 0.3],
    small: [0.1, 0.2, 0.3],
    hologram: true,
};

impl H4BloomSettings {
    /// A camera fx tag's bloom fields, to check the constants against.
    #[cfg(test)]
    pub(super) fn read(root: blam_tags::TagStruct<'_>) -> Self {
        let real = |holder: &str, name: &str| root.descend(holder).and_then(|holder| holder.read_real(name)).unwrap_or(0.0);
        let color = |holder: &str, name: &str| {
            let color = root.descend(holder).and_then(|holder| holder.field(name)).and_then(|field| field.value());
            match color {
                Some(blam_tags::TagFieldData::RealRgbColor(color)) => [color.red, color.green, color.blue],
                _ => [0.0; 3],
            }
        };
        Self {
            highlight: real("bloom_highlight", "highlight bloom"),
            inherent: real("bloom_inherent", "inherent bloom"),
            self_illum: real("bloom_self_illum", "self-illum bloom bloom"),
            intensity: real("bloom_intensity", "bloom intensity"),
            large: color("bloom_large_color", "large color"),
            medium: color("bloom_medium_color", "medium color"),
            small: color("bloom_small_color", "small color"),
            hologram: false,
        }
    }

    /// The hologram chain, its highlight and inherent bloom scaled by
    /// `GetHologramBloomIntensityScale` (`RenderHolograms`, X360 0x829B63C8).
    pub(super) fn hologram(distance_scale: f32) -> Self {
        Self {
            highlight: HOLOGRAM_BLOOM.highlight * distance_scale,
            inherent: HOLOGRAM_BLOOM.inherent * distance_scale,
            ..HOLOGRAM_BLOOM
        }
    }
}

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
uniform bool keeps_alpha;
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
            ? textureLod(grading, s.rgb * grading_scale + grading_offset, 0.0).a
            : dot(s.rgb, vec3(0.3086, 0.6094, 0.0820));
        float bloom = scale.x * intensity + scale.z * s.a + scale.y;
        color = vec4(s.rgb * bloom, keeps_alpha ? s.a : intensity);
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
fn scaled(color: [f32; 3], intensity: f32, frame: [i32; 2]) -> [f32; 4] {
    let factor = intensity * resolution_scale(frame);
    [color[0] * factor, color[1] * factor, color[2] * factor, 1.0]
}

/// `accum_LDR_temp` (/2), then `aux_small`, `aux_tiny`, `aux_mini` (/4, /16,
/// /64) and their seconds.
struct Surfaces {
    temp: ColorTarget,
    small: [ColorTarget; 2],
    tiny: [ColorTarget; 2],
    mini: [ColorTarget; 2],
    frame: [i32; 2],
}

impl Surfaces {
    unsafe fn new(gl: &glow::Context, frame: [i32; 2]) -> Option<Self> {
        // DXGI 10, RGBA16F, as the aux surfaces (MCC table 0x1424A6000).
        let mut made = Made::new(gl);
        let mut surface = |divisor| unsafe { made.color(divided(frame, divisor), Texels::RGBA16F, Sampling::LINEAR_CLAMPED) };
        let surfaces = Self {
            temp: surface(2.0)?,
            small: [surface(4.0)?, surface(4.0)?],
            tiny: [surface(16.0)?, surface(16.0)?],
            mini: [surface(64.0)?, surface(64.0)?],
            frame,
        };
        made.keep();
        Some(surfaces)
    }

    unsafe fn delete(&self, gl: &glow::Context) {
        let all = std::iter::once(&self.temp).chain(&self.small).chain(&self.tiny).chain(&self.mini);
        for surface in all {
            unsafe { surface.delete(gl) };
        }
    }
}

pub(super) struct H4Bloom {
    chain: ScreenChain,
    surfaces: Option<Surfaces>,
    /// Bound for the grading lookup when there is none: an empty 3D unit is
    /// "unloadable" to a sampler the program declares even where it never
    /// reads it (Apple's GL warns once).
    no_volume: Option<glow::NativeTexture>,
}

/// The colour grading lookup shader 159 reads its intensity from, and how.
pub(super) struct H4BloomGrading {
    pub(super) texture: glow::NativeTexture,
    pub(super) scale: [f32; 3],
    pub(super) offset: [f32; 3],
}

impl H4Bloom {
    pub(super) unsafe fn new(
        gl: &glow::Context,
        nothing: Option<glow::NativeTexture>,
        no_volume: Option<glow::NativeTexture>,
    ) -> Option<Self> {
        unsafe { Some(Self { chain: ScreenChain { program: screen_program(gl, FRAGMENT)?, nothing }, surfaces: None, no_volume }) }
    }

    unsafe fn step(
        &self,
        gl: &glow::Context,
        target: &ColorTarget,
        pass: Pass,
        source: (glow::NativeTexture, [i32; 2]),
        added: Option<glow::NativeTexture>,
        scale: [f32; 4],
    ) {
        unsafe {
            self.chain.pass(gl, target, source.0, added, |uniform| {
                gl.uniform_1_i32(uniform("pass").as_ref(), pass as i32);
                gl.uniform_4_f32_slice(uniform("scale").as_ref(), &scale);
                gl.uniform_2_f32(uniform("target_size").as_ref(), target.size[0] as f32, target.size[1] as f32);
                let [width, height] = source.1.map(|side| 1.0 / side as f32);
                gl.uniform_2_f32(uniform("source_texel").as_ref(), width, height);
            });
        }
    }

    /// `gaussian_blur_fixed`: across into `scratch`, then back down scaled.
    unsafe fn blur(&self, gl: &glow::Context, surface: &ColorTarget, scratch: &ColorTarget, scale: [f32; 4]) {
        unsafe {
            self.step(gl, scratch, Pass::BlurHorizontal, (surface.texture, surface.size), None, [1.0; 4]);
            self.step(gl, surface, Pass::Kernel5, (scratch.texture, scratch.size), None, scale);
        }
    }

    /// What the final composite adds: the bloom of `accumulation` by
    /// `settings`. Leaves `scene` bound, and the viewport and scissor as they
    /// were. A hologram's chain reads no grading.
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn build(
        &mut self,
        gl: &glow::Context,
        vertex_array: glow::NativeVertexArray,
        scene: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        frame: [i32; 2],
        grading: Option<&H4BloomGrading>,
        settings: &H4BloomSettings,
    ) -> Option<glow::NativeTexture> {
        let grading = grading.filter(|_| !settings.hologram);
        unsafe {
            if self.surfaces.as_ref().is_none_or(|surfaces| surfaces.frame != frame) {
                if let Some(old) = self.surfaces.take() {
                    old.delete(gl);
                }
                self.surfaces = Surfaces::new(gl, frame);
            }
            let surfaces = self.surfaces.as_ref()?;
            let run = self.chain.begin(gl, vertex_array, accumulation);
            let uniform = |name: &str| self.chain.uniform(gl, name);
            gl.uniform_1_i32(uniform("grading").as_ref(), 2);
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_3D, grading.map(|grading| grading.texture).or(self.no_volume));
            gl.active_texture(glow::TEXTURE0);
            gl.uniform_1_i32(uniform("graded").as_ref(), i32::from(grading.is_some()));
            gl.uniform_1_i32(uniform("keeps_alpha").as_ref(), i32::from(settings.hologram));
            if let Some(grading) = grading {
                gl.uniform_3_f32_slice(uniform("grading_scale").as_ref(), &grading.scale);
                gl.uniform_3_f32_slice(uniform("grading_offset").as_ref(), &grading.offset);
            }

            let ones = [1.0; 4];
            let [small, small2] = &surfaces.small;
            let [tiny, tiny2] = &surfaces.tiny;
            let [mini, mini2] = &surfaces.mini;
            self.step(gl, &surfaces.temp, Pass::Downsample2x2, (accumulation, frame), None, [1.0, 1.0, 0.0, 0.0]);
            let temp = (surfaces.temp.texture, surfaces.temp.size);
            let curve = [settings.highlight, settings.inherent, settings.self_illum, 1.0];
            self.step(gl, small, Pass::BloomCurve, temp, None, curve);
            self.step(gl, tiny, Pass::Gaussian4x4, (small.texture, small.size), None, ones);
            self.step(gl, mini, Pass::Gaussian4x4, (tiny.texture, tiny.size), None, ones);
            let intensity = settings.intensity;
            self.blur(gl, mini, mini2, scaled(settings.large, intensity, frame));
            let medium = scaled(settings.medium, intensity, frame);
            self.step(gl, tiny2, Pass::AddAlpha1, (tiny.texture, tiny.size), Some(mini.texture), medium);
            self.blur(gl, tiny2, tiny, ones);
            let small_color = scaled(settings.small, intensity, frame);
            self.step(gl, small2, Pass::Add, (small.texture, small.size), Some(tiny2.texture), small_color);
            self.blur(gl, small2, small, ones);

            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_3D, None);
            gl.active_texture(glow::TEXTURE0);
            self.chain.end(gl, run, scene);
            // `small` holds `blur_11_horizontal` of the result.
            Some(if settings.hologram { small.texture } else { small2.texture })
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
        assert_eq!(scaled([0.0; 3], 3.0, [1000, 700]), [0.0, 0.0, 0.0, 1.0]);
        assert!((scaled([0.25; 3], 0.3, [1000, 700])[0] - 0.075).abs() < 1e-7);
        assert!((scaled([1.0; 3], 0.3, [1920, 1080])[0] - 0.133_333).abs() < 1e-5);
    }

    #[test]
    fn the_surfaces_are_the_frame_halved_then_quartered() {
        assert_eq!(divided([1000, 700], 2.0), [500, 350]);
        assert_eq!(divided([1000, 700], 64.0), [16, 11]);
        assert_eq!(divided([10, 10], 64.0), [1, 1], "never empty");
    }

    /// The hologram's chain scales only highlight and inherent bloom by its
    /// distance, and keeps its own camera fx's intensity and colours.
    #[test]
    fn a_hologram_scales_highlight_and_inherent_bloom_by_distance() {
        let far = H4BloomSettings::hologram(3.0);
        assert!((far.highlight - 0.3).abs() < 1e-6);
        assert_eq!(far.inherent, 0.0);
        assert_eq!((far.intensity, far.small), (0.7, [0.1, 0.2, 0.3]));
        assert!(far.hologram && !VIEW_BLOOM.hologram);
    }

    /// With `BLAM_TEST_H4EK`: the view's bloom is `m80_cam_fx_global`'s and the
    /// hologram's is the hologram camera fx's.
    #[test]
    fn the_bloom_settings_are_their_tags() {
        let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
            return;
        };
        let read = |path: &str| blam_tags::TagFile::read(std::path::Path::new(&root).join(path)).unwrap();
        let m80 = read("environments/solo/m80_delta/m80_cam_fx_global.camera_fx_settings");
        assert_eq!(H4BloomSettings::read(m80.root()), VIEW_BLOOM);
        let hologram = read("globals/defaults/hologram.camera_fx_settings");
        assert_eq!(H4BloomSettings { hologram: true, ..H4BloomSettings::read(hologram.root()) }, HOLOGRAM_BLOOM);
    }
}
