//! Halo 3's bloom, as `postprocess_bloom_buffer` builds it: the frame's bright
//! part at a quarter of its size, a quarter of that twice over, then each level
//! blurred and added to the one above. The final composite adds the top level
//! ahead of its tone curve.

use super::super::gl::chain::ScreenChain;
use super::super::gl::target::{ColorTarget, Made, Sampling, Texels};
use eframe::glow::{self, HasContext};

use crate::app::model_preview::gl::screen::screen_program;

/// `globals\default.camera_fx_settings`.
const BLOOM_POINT: f32 = 1.2;
const BLOOM_INHERENT: f32 = 0.2;
const BLOOM_INTENSITY: f32 = 0.2;

/// What `bloom_point_exposure` comes to at no exposure, the tag's anti-bloom
/// having nothing to raise.
const BLOOM_POINT_EXPOSURE: f32 = 0.66943294;

const DOWNSAMPLE: i32 = 4;

const FRAGMENT: &str = include_str!("shaders/bloom.frag");

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
        let mut level =
            |size| unsafe { made.color(size, Texels::RGBA16F, Sampling::LINEAR_CLAMPED) };
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

pub(in crate::app::model_preview) struct H3Bloom {
    chain: ScreenChain,
    levels: Option<Levels>,
}

impl H3Bloom {
    pub(in crate::app::model_preview) unsafe fn new(
        gl: &glow::Context,
        nothing: Option<glow::NativeTexture>,
    ) -> Option<Self> {
        unsafe {
            Some(Self {
                chain: ScreenChain {
                    program: screen_program(gl, FRAGMENT)?,
                    nothing,
                },
                levels: None,
            })
        }
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
        unsafe {
            self.chain.pass(gl, target, source.0, added, |uniform| {
                gl.uniform_1_i32(uniform("mode").as_ref(), mode);
                gl.uniform_1_f32(uniform("scale").as_ref(), scale);
                let [width, height] = target.size.map(|side| 1.0 / side as f32);
                gl.uniform_2_f32(uniform("target_texel").as_ref(), width, height);
                let [width, height] = source.1.map(|side| 1.0 / side as f32);
                gl.uniform_2_f32(uniform("source_texel").as_ref(), width, height);
            });
        }
    }

    /// `gaussian_blur_fixed`, `pair[1]` being its scratch.
    unsafe fn blur(&self, gl: &glow::Context, pair: &[ColorTarget; 2], scale: f32) {
        unsafe {
            self.step(gl, &pair[1], 2, (pair[0].texture, pair[0].size), None, 1.0);
            self.step(
                gl,
                &pair[0],
                3,
                (pair[1].texture, pair[1].size),
                None,
                scale,
            );
        }
    }

    /// What the final composite adds. Leaves `scene` bound, and the viewport and
    /// scissor as they were.
    pub(in crate::app::model_preview) unsafe fn build(
        &mut self,
        gl: &glow::Context,
        vertex_array: glow::NativeVertexArray,
        scene: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        frame: [i32; 2],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            if self
                .levels
                .as_ref()
                .is_none_or(|levels| levels.frame != frame)
            {
                if let Some(old) = self.levels.take() {
                    old.delete(gl);
                }
                self.levels = Levels::new(gl, frame);
            }
            let levels = self.levels.as_ref()?;
            let run = self.chain.begin(gl, vertex_array, accumulation);
            let point = BLOOM_POINT_EXPOSURE * BLOOM_POINT;
            gl.uniform_2_f32(
                self.chain.uniform(gl, "curve").as_ref(),
                point,
                BLOOM_INHERENT,
            );

            let [small, _] = &levels.small;
            let ([tiny, _], [mini, _]) = (&levels.tiny, &levels.mini);
            self.step(gl, small, 0, (accumulation, frame), None, 1.0);
            self.step(gl, tiny, 1, (small.texture, small.size), None, 1.0);
            self.step(gl, mini, 1, (tiny.texture, tiny.size), None, 1.0);
            self.blur(gl, &levels.mini, BLOOM_INTENSITY);
            // Into the scratch of each: the level below still reads its own.
            let tiny_sum = &levels.tiny[1];
            self.step(
                gl,
                tiny_sum,
                4,
                (tiny.texture, tiny.size),
                Some(mini.texture),
                BLOOM_INTENSITY,
            );
            self.step(gl, tiny, 2, (tiny_sum.texture, tiny_sum.size), None, 1.0);
            self.step(gl, tiny_sum, 3, (tiny.texture, tiny.size), None, 1.0);
            let small_sum = &levels.small[1];
            self.step(
                gl,
                small_sum,
                4,
                (small.texture, small.size),
                Some(tiny_sum.texture),
                BLOOM_INTENSITY,
            );
            self.step(gl, small, 2, (small_sum.texture, small_sum.size), None, 1.0);
            self.step(gl, small_sum, 3, (small.texture, small.size), None, 1.0);

            self.chain.end(gl, run, scene);
            Some(small_sum.texture)
        }
    }
}
