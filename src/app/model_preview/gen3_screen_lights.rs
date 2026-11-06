//! What Reach's and Halo 4's screen-space lights share around each light's
//! own constants: the scene's depth to read, the additive state they draw
//! under, and the scene textures each pass names.

use super::*;
use gen3::shaders::H3Pass;
use h3_draw::H3Program;

impl RenderCore {
    /// Ready the screen lights over `accumulation`, `size` of it: its depth
    /// copied for the passes to read, then no depth test or culling, adding
    /// one to one into its colour alone, through the empty vertex array.
    pub(super) unsafe fn begin_screen_lights(
        &mut self,
        gl: &glow::Context,
        size: [i32; 2],
        accumulation: glow::NativeFramebuffer,
        depth_copy: glow::NativeFramebuffer,
    ) {
        unsafe {
            Self::copy_scene_depth(gl, size, accumulation, depth_copy);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(accumulation));
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE);
            // `RenderSSLightInternal`'s colour write mask 7.
            gl.color_mask(true, true, true, false);
            let empty = self.h3_empty_array(gl);
            gl.bind_vertex_array(Some(empty));
        }
    }

    /// Bind each of `pass`'s textures that `texture` answers for to its own
    /// unit, in the pass's order; one it does not answer for keeps its unit
    /// and is left unset.
    pub(super) unsafe fn bind_screen_light_textures(
        gl: &glow::Context,
        pass: &H3Pass,
        program: &H3Program,
        texture: impl Fn(&str) -> Option<Option<glow::NativeTexture>>,
    ) {
        for (unit, (name, _)) in pass.textures.iter().enumerate() {
            unsafe {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                let Some(bound) = texture(name) else {
                    continue;
                };
                gl.bind_texture(glow::TEXTURE_2D, bound);
                gl.uniform_1_i32(program.location(gl, name).as_ref(), unit as i32);
            }
        }
    }

    /// The scene's state again once the lights are drawn.
    pub(super) unsafe fn end_screen_lights(gl: &glow::Context) {
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            gl.enable(glow::DEPTH_TEST);
        }
    }
}
