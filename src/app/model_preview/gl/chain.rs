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
    pub(in crate::app::model_preview) unsafe fn uniform(&self, gl: &glow::Context, name: &str) -> Option<glow::NativeUniformLocation> {
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
    pub(in crate::app::model_preview) unsafe fn end(&self, gl: &glow::Context, run: ChainRun, scene: glow::NativeFramebuffer) {
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
