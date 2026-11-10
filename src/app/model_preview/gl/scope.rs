//! The state a pass borrows and gives back.

use eframe::glow::{self, HasContext};

/// The framebuffer state a pass into its own targets borrows and must
/// return: the framebuffer, viewport and scissor bound when it began.
pub(in crate::app::model_preview) struct PostScope {
    pub(in crate::app::model_preview) framebuffer: Option<glow::NativeFramebuffer>,
    pub(in crate::app::model_preview) viewport: [i32; 4],
    pub(in crate::app::model_preview) scissor: [i32; 4],
    pub(in crate::app::model_preview) scissored: bool,
}

impl PostScope {
    pub(in crate::app::model_preview) unsafe fn capture(gl: &glow::Context) -> Self {
        unsafe {
            let (mut viewport, mut scissor) = ([0; 4], [0; 4]);
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut scissor);
            Self {
                framebuffer: gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING),
                viewport,
                scissor,
                scissored: gl.is_enabled(glow::SCISSOR_TEST),
            }
        }
    }

    /// The viewport's size, at least a pixel each way.
    pub(in crate::app::model_preview) fn size(&self) -> [i32; 2] {
        [self.viewport[2].max(1), self.viewport[3].max(1)]
    }

    pub(in crate::app::model_preview) unsafe fn restore(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, self.framebuffer);
            let [x, y, width, height] = self.viewport;
            gl.viewport(x, y, width, height);
            if self.scissored {
                gl.enable(glow::SCISSOR_TEST);
            } else {
                gl.disable(glow::SCISSOR_TEST);
            }
            let [x, y, width, height] = self.scissor;
            gl.scissor(x, y, width, height);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod gl_tests {
    use crate::app::model_preview::harness::gl::headless_gl;
    use eframe::glow::{self, HasContext as _};

    /// [`PostScope`](crate::app::model_preview::gl::scope::PostScope) gives back the
    /// framebuffer, viewport and scissor a pass found, whatever it left.
    #[test]
    fn a_post_scope_restores_what_it_found() {
        use crate::app::model_preview::gl::scope::PostScope;
        use crate::app::model_preview::gl::target::{ColorTarget, Sampling, Texels};
        let gl = headless_gl();
        unsafe {
            let found = ColorTarget::new(&gl, [8, 8], Texels::RGBA8, Sampling::NEAREST).unwrap();
            let other = ColorTarget::new(&gl, [8, 8], Texels::RGBA8, Sampling::NEAREST).unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(found.framebuffer));
            gl.viewport(1, 2, 3, 4);
            gl.scissor(2, 3, 4, 5);
            gl.enable(glow::SCISSOR_TEST);
            let scope = PostScope::capture(&gl);
            assert_eq!(scope.size(), [3, 4]);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(other.framebuffer));
            gl.viewport(0, 0, 8, 8);
            gl.scissor(0, 0, 1, 1);
            gl.disable(glow::SCISSOR_TEST);
            scope.restore(&gl);
            let (mut viewport, mut scissor) = ([0; 4], [0; 4]);
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut scissor);
            assert_eq!(
                gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING),
                Some(found.framebuffer)
            );
            assert_eq!((viewport, scissor), ([1, 2, 3, 4], [2, 3, 4, 5]));
            assert!(gl.is_enabled(glow::SCISSOR_TEST));
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            found.delete(&gl);
            other.delete(&gl);
        }
    }
}
