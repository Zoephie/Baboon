//! Copies between framebuffers by blit.

use eframe::glow::{self, HasContext as _};

/// The depth and stencil, `size` of them, from `from` into `to`, which are
/// left bound for reading and drawing.
pub(in crate::app::model_preview) unsafe fn blit_depth(
    gl: &glow::Context,
    from: glow::NativeFramebuffer,
    to: glow::NativeFramebuffer,
    size: [i32; 2],
) {
    let [width, height] = size;
    unsafe {
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(from));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(to));
        let mask = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
        gl.blit_framebuffer(0, 0, width, height, 0, 0, width, height, mask, glow::NEAREST);
    }
}

/// The vertex array a full-screen triangle draws from, which has none: made
/// into `held` on first use.
pub(in crate::app::model_preview) unsafe fn empty_vertex_array(
    gl: &glow::Context,
    held: &mut Option<glow::NativeVertexArray>,
) -> glow::NativeVertexArray {
    unsafe { *held.get_or_insert_with(|| gl.create_vertex_array().expect("a vertex array")) }
}
