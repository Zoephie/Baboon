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
