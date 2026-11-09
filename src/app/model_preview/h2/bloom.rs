//! An optional glow for Halo 2 materials. Bloom passes (the `selfibloomination`
//! layer) write a mask to the frame's alpha. `rasterizer_bloom_postprocess`
//! downsamples it with its third built-in shader (`off_EBC81C[2]`), which keeps
//! `0.25 * mask` (`rasterizer_selfibloomination_scale` 0.25) and none of the
//! colour under it (`..._to_bloom_amount` 0), blurs that, and its fourth adds
//! twice the blurred mask back to every channel: a white glow. The same passes
//! bloom what is brighter than a threshold, `saturate(4 * (color - threshold))`,
//! added back at twice a brightness. A level's sky or fog may override both
//! (08b's control room: 0.3 and 0.2); a lone model takes what
//! `rasterizer_postprocess_scene` uses with no override.

use eframe::glow::{self, HasContext};

use super::super::gl::pass_state::PassState;
use super::super::gl::target::{ColorTarget, Made, Sampling, Texels};

/// The blur amount `rasterizer_postprocess_scene` passes, in downsampled texels.
const BLUR_RADIUS: f32 = 3.992;
/// `rasterizer_bloom_default_threshold`.
const THRESHOLD: f32 = 0.9;
/// `sub_90C570`'s default 0.5 for both, as `rasterizer_postprocess_scene`
/// mixes them: (0.5 + 1) * 0.5 * 0.5.
const BRIGHTNESS: f32 = 0.375;
/// `rasterizer_selfibloomination_scale`, halved and doubled by the downsample.
const MASK_SCALE: f32 = 0.25;
const DOWNSAMPLE: i32 = 4;
const BLUR_PASSES: usize = 2;

const VERTEX: &str = include_str!("shaders/bloom.vert");

const FRAGMENT: &str = include_str!("shaders/bloom.frag");

pub(in crate::app::model_preview) struct H2Bloom {
    program: glow::NativeProgram,
    vertex_array: glow::NativeVertexArray,
    surfaces: Option<([ColorTarget; 2], [i32; 2])>,
}

impl H2Bloom {
    pub(in crate::app::model_preview) unsafe fn new(gl: &glow::Context, version: &str) -> Option<Self> {
        unsafe {
            let (vertex, fragment) = (format!("{version}{VERTEX}"), format!("{version}{FRAGMENT}"));
            let stages = [(glow::VERTEX_SHADER, vertex.as_str()), (glow::FRAGMENT_SHADER, fragment.as_str())];
            let program = super::super::gl::link::link(gl, &stages, &[])
                .map_err(|error| eprintln!("model preview: bloom: {error}"))
                .ok()?;
            let vertex_array = gl.create_vertex_array().ok()?;
            Some(Self { program, vertex_array, surfaces: None })
        }
    }

    /// Add the glow of `scene`, whose alpha holds the mask, to the framebuffer
    /// bound for it. Leaves that framebuffer bound.
    pub(in crate::app::model_preview) unsafe fn apply(
        &mut self,
        gl: &glow::Context,
        scene_framebuffer: glow::NativeFramebuffer,
        scene_color: glow::NativeTexture,
        size: [i32; 2],
        show_backfaces: bool,
    ) {
        let small = [(size[0] / DOWNSAMPLE).max(1), (size[1] / DOWNSAMPLE).max(1)];
        unsafe {
            if self.surfaces.as_ref().is_none_or(|(_, current)| *current != small) {
                if let Some((old, _)) = self.surfaces.take() {
                    old.iter().for_each(|surface| surface.delete(gl));
                }
                let mut made = Made::new(gl);
                let both = [(); 2].map(|_| made.color(small, Texels::RGBA8, Sampling::LINEAR_CLAMPED));
                let [Some(first), Some(second)] = both else {
                    drop(made);
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene_framebuffer));
                    return;
                };
                made.keep();
                self.surfaces = Some(([first, second], small));
            }
            let Some(([first, second], _)) = &self.surfaces else {
                return;
            };
            bloom_state(self.program, self.vertex_array).apply(gl);
            let uniform = |name: &str| gl.get_uniform_location(self.program, name);
            gl.uniform_1_i32(uniform("u_source").as_ref(), 0);
            let draw = |target: glow::NativeFramebuffer, source, mode: i32| {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target));
                gl.bind_texture(glow::TEXTURE_2D, Some(source));
                gl.uniform_1_i32(uniform("u_mode").as_ref(), mode);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            };

            gl.viewport(0, 0, small[0], small[1]);
            gl.uniform_1_f32(uniform("u_scale").as_ref(), MASK_SCALE);
            gl.uniform_1_f32(uniform("u_threshold").as_ref(), THRESHOLD);
            gl.uniform_1_f32(uniform("u_brightness").as_ref(), BRIGHTNESS);
            draw(first.framebuffer, scene_color, 0);
            // The nine taps reach four texels, as far as the game's blur.
            let reach = BLUR_RADIUS / 4.0;
            for _ in 0..BLUR_PASSES {
                gl.uniform_2_f32(uniform("u_step").as_ref(), reach / small[0] as f32, 0.0);
                draw(second.framebuffer, first.texture, 1);
                gl.uniform_2_f32(uniform("u_step").as_ref(), 0.0, reach / small[1] as f32);
                draw(first.framebuffer, second.texture, 1);
            }

            gl.viewport(0, 0, size[0], size[1]);
            add_state().apply(gl);
            draw(scene_framebuffer, first.texture, 2);
            after_bloom_state(show_backfaces).apply(gl);
        }
    }
}

/// What the mask and blurs draw under: the bloom program and its triangle, no
/// depth test, culling or blend, every channel written, unit 0 active.
fn bloom_state(program: glow::NativeProgram, vertex_array: glow::NativeVertexArray) -> PassState {
    PassState::new()
        .use_program(program)
        .bind_vertex_array(vertex_array)
        .disable(glow::DEPTH_TEST)
        .disable(glow::CULL_FACE)
        .disable(glow::BLEND)
        .depth_mask(false)
        .color_mask([true, true, true, true])
        .active_texture(glow::TEXTURE0)
}

/// How the blurred mask goes onto the scene: `sub_9206D0(19, 10)`,
/// `(20, 2)`, inverse destination colour and one, alpha kept.
fn add_state() -> PassState {
    PassState::new()
        .enable(glow::BLEND)
        .blend_equation(glow::FUNC_ADD)
        .blend_func(glow::ONE_MINUS_DST_COLOR, glow::ONE)
        .color_mask([true, true, true, false])
}

/// What the bloom switched off for its full-screen passes, back for whatever
/// draws the model after.
fn after_bloom_state(show_backfaces: bool) -> PassState {
    PassState::new()
        .disable(glow::BLEND)
        .color_mask([true, true, true, true])
        .depth_mask(true)
        .enable(glow::DEPTH_TEST)
        .culling_unless(show_backfaces)
        .bind_texture(glow::TEXTURE_2D, None)
}
