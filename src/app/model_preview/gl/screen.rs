//! A program over the whole picture: one triangle that covers it, for any
//! fragment stage.

use super::pass_state::PassState;
use eframe::glow;

/// A screen pass laid over the picture: no depth test, blended as
/// premultiplied (what adds light covers nothing), drawn from `array`, the
/// empty vertex array. What the gen3 composites draw with.
pub(in crate::app::model_preview) fn premultiplied_over(
    array: glow::NativeVertexArray,
) -> PassState {
    PassState::new()
        .disable(glow::DEPTH_TEST)
        .enable(glow::BLEND)
        .blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA)
        .bind_vertex_array(array)
}

const COMPOSITE_VERTEX: &str = include_str!("shaders/composite.vert");

/// A full-screen program for `fragment`, or none if it fails to compile or
/// link. A failure is logged once and remembered, so callers that ask again
/// each frame neither rebuild nor log it again.
pub(in crate::app::model_preview) unsafe fn screen_program(
    gl: &glow::Context,
    fragment: &str,
) -> Option<glow::NativeProgram> {
    thread_local! {
        static FAILED: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
    }
    if FAILED.with(|failed| failed.borrow().contains(fragment)) {
        return None;
    }
    let fail = |log: String| {
        eprintln!("model preview: screen shader: {log}");
        FAILED.with(|failed| failed.borrow_mut().insert(fragment.to_owned()));
        None
    };
    unsafe {
        let stages = [
            (glow::VERTEX_SHADER, COMPOSITE_VERTEX),
            (glow::FRAGMENT_SHADER, fragment),
        ];
        match super::link::link(gl, &stages, &[]) {
            Ok(program) => Some(program),
            Err(log) => fail(log),
        }
    }
}
