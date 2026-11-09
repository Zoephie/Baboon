//! A chain of full-screen passes through one program: what every gen3 bloom
//! does around its own passes.

use eframe::glow::{self, HasContext};

use super::pass_state::PassState;
use super::scope::PostScope;
use super::target::ColorTarget;

/// The program a chain draws every pass with, and what a pass that adds
/// nothing leaves bound (the program samples `added` regardless).
pub(in crate::app::model_preview) struct ScreenChain {
    pub(in crate::app::model_preview) program: glow::NativeProgram,
    pub(in crate::app::model_preview) nothing: Option<glow::NativeTexture>,
}

/// A chain under way: the state it found, and the frame it reads.
pub(in crate::app::model_preview) struct ChainRun {
    scope: PostScope,
    input: glow::NativeTexture,
}

impl ScreenChain {
    /// Begin a run reading `input`: the state found is kept, then no
    /// scissor, depth test or blend, every channel written, the program and
    /// `vertex_array` bound, `source` on unit 0 and `added` on unit 1, and
    /// `input` filtered, as the game's first taps lean on bilinear filtering
    /// where the preview reads it by texel elsewhere.
    ///
    /// # Safety
    ///
    /// The context must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn begin(
        &self,
        gl: &glow::Context,
        vertex_array: glow::NativeVertexArray,
        input: glow::NativeTexture,
    ) -> ChainRun {
        unsafe {
            let scope = PostScope::capture(gl);
            chain_state(self.program, vertex_array).apply(gl);
            gl.uniform_1_i32(self.uniform(gl, "source").as_ref(), 0);
            gl.uniform_1_i32(self.uniform(gl, "added").as_ref(), 1);
            gl.bind_texture(glow::TEXTURE_2D, Some(input));
            for name in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, glow::LINEAR as i32);
            }
            ChainRun { scope, input }
        }
    }

    /// The program's uniform `name`.
    ///
    /// # Safety
    ///
    /// The context must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn uniform(
        &self,
        gl: &glow::Context,
        name: &str,
    ) -> Option<glow::NativeUniformLocation> {
        unsafe { gl.get_uniform_location(self.program, name) }
    }

    /// One pass into `target`, at its size: `source` on unit 0, `added` (or
    /// nothing) on unit 1, the pass's own uniforms set by `uniforms`.
    ///
    /// # Safety
    ///
    /// The context must be current on this thread, inside a run.
    pub(in crate::app::model_preview) unsafe fn pass(
        &self,
        gl: &glow::Context,
        target: &ColorTarget,
        source: glow::NativeTexture,
        added: Option<glow::NativeTexture>,
        uniforms: impl FnOnce(&dyn Fn(&str) -> Option<glow::NativeUniformLocation>),
    ) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            gl.viewport(0, 0, target.size[0], target.size[1]);
            PassState::new()
                .texture_on(glow::TEXTURE1, glow::TEXTURE_2D, added.or(self.nothing))
                .bind_texture(glow::TEXTURE_2D, Some(source))
                .apply(gl);
            uniforms(&|name| self.uniform(gl, name));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    /// End `run`: units 0 and 1 emptied, the input read by texel again, the
    /// state found put back, and `scene` bound (making the chain's targets
    /// may have left one of theirs bound when the run began).
    ///
    /// # Safety
    ///
    /// The context must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn end(
        &self,
        gl: &glow::Context,
        run: ChainRun,
        scene: glow::NativeFramebuffer,
    ) {
        unsafe {
            PassState::new()
                .texture_on(glow::TEXTURE1, glow::TEXTURE_2D, None)
                .bind_texture(glow::TEXTURE_2D, Some(run.input))
                .apply(gl);
            for name in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, glow::NEAREST as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            run.scope.restore(gl);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene));
        }
    }
}

/// What a chain's passes draw under: no scissor, depth test or blend, every
/// channel written, its program and the screen triangle's vertex array.
fn chain_state(program: glow::NativeProgram, vertex_array: glow::NativeVertexArray) -> PassState {
    PassState::new()
        .disable(glow::SCISSOR_TEST)
        .disable(glow::DEPTH_TEST)
        .disable(glow::BLEND)
        .color_mask([true, true, true, true])
        .use_program(program)
        .bind_vertex_array(vertex_array)
}

#[cfg(all(test, target_os = "macos"))]
mod gl_tests {
    use crate::app::model_preview::harness::gl::headless_gl;
    use eframe::glow::{self, HasContext as _};

    /// A [`ScreenChain`](crate::app::model_preview::gl::chain::ScreenChain) run puts back the
    /// scissor and viewport it found, binds the scene, reads its input by texel
    /// again, and leaves no texture on the units its passes use.
    #[test]
    fn a_screen_chain_run_leaves_the_state_it_found() {
        use crate::app::model_preview::gl::chain::ScreenChain;
        use crate::app::model_preview::gl::link::link;
        use crate::app::model_preview::gl::target::{ColorTarget, Sampling, Texels};
        const VERTEX: &str = "#version 410\nvoid main() { gl_Position = vec4(vec2(gl_VertexID & 1, gl_VertexID >> 1) * 4.0 - 1.0, 0.0, 1.0); }\n";
        const FRAGMENT: &str = "#version 410\nuniform sampler2D source;\nuniform sampler2D added;\nout vec4 colour;\nvoid main() { colour = texelFetch(source, ivec2(0), 0) + texelFetch(added, ivec2(0), 0); }\n";
        let gl = headless_gl();
        unsafe {
            let program = link(
                &gl,
                &[
                    (glow::VERTEX_SHADER, VERTEX),
                    (glow::FRAGMENT_SHADER, FRAGMENT),
                ],
                &[],
            )
            .unwrap();
            let chain = ScreenChain {
                program,
                nothing: None,
            };
            let vertex_array = gl.create_vertex_array().unwrap();
            let [scene, input, target] = [(); 3]
                .map(|_| ColorTarget::new(&gl, [4, 4], Texels::RGBA8, Sampling::NEAREST).unwrap());
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            gl.viewport(1, 1, 2, 2);
            gl.enable(glow::SCISSOR_TEST);
            let run = chain.begin(&gl, vertex_array, input.texture);
            assert!(!gl.is_enabled(glow::SCISSOR_TEST));
            chain.pass(&gl, &target, input.texture, Some(scene.texture), |_| {});
            chain.end(&gl, run, scene.framebuffer);
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            assert_eq!(viewport, [1, 1, 2, 2]);
            assert!(gl.is_enabled(glow::SCISSOR_TEST));
            assert_eq!(
                gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING),
                Some(scene.framebuffer)
            );
            for unit in [glow::TEXTURE0, glow::TEXTURE1] {
                gl.active_texture(unit);
                assert_eq!(
                    gl.get_parameter_texture(glow::TEXTURE_BINDING_2D),
                    None,
                    "unit {unit:#x}"
                );
            }
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(input.texture));
            let filter = gl.get_tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER);
            assert_eq!(
                filter,
                glow::NEAREST as i32,
                "the input is read by texel again"
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            for made in [scene, input, target] {
                made.delete(&gl);
            }
            gl.delete_vertex_array(vertex_array);
            gl.delete_program(program);
        }
    }
}
