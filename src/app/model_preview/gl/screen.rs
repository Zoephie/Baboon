//! A program over the whole picture: one triangle that covers it, for any
//! fragment stage.

use eframe::glow;

const COMPOSITE_VERTEX: &str = "#version 410 core
void main() {
    vec2 corner = vec2(gl_VertexID & 1, gl_VertexID >> 1) * 4.0 - 1.0;
    gl_Position = vec4(corner, 0.0, 1.0);
}
";

/// A full-screen program for `fragment`, or none if it fails to compile or
/// link. A failure is logged once and remembered, so callers that ask again
/// each frame neither rebuild nor log it again.
pub(in crate::app::model_preview) unsafe fn screen_program(gl: &glow::Context, fragment: &str) -> Option<glow::NativeProgram> {
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
        let stages = [(glow::VERTEX_SHADER, COMPOSITE_VERTEX), (glow::FRAGMENT_SHADER, fragment)];
        match super::link::link(gl, &stages, &[]) {
            Ok(program) => Some(program),
            Err(log) => fail(log),
        }
    }
}
