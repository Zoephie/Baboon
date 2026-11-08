//! A linked program and where its uniforms landed, each looked up once.

use std::cell::RefCell;
use std::collections::HashMap;

use eframe::glow::{self, HasContext};

/// A linked program. Uniforms are looked up by name the first time each is
/// asked for: a translated shader declares a different handful each.
pub(in crate::app::model_preview) struct Linked {
    pub(in crate::app::model_preview) program: glow::NativeProgram,
    locations: RefCell<HashMap<String, Option<glow::NativeUniformLocation>>>,
}

impl Linked {
    pub(in crate::app::model_preview) fn new(program: glow::NativeProgram) -> Self {
        Self { program, locations: RefCell::default() }
    }

    /// Where uniform `name` landed, or none where the program has none.
    ///
    /// # Safety
    ///
    /// The context the program was linked in must be current on this thread.
    pub(in crate::app::model_preview) unsafe fn location(&self, gl: &glow::Context, name: &str) -> Option<glow::NativeUniformLocation> {
        if let Some(found) = self.locations.borrow().get(name) {
            return *found;
        }
        let found = unsafe { gl.get_uniform_location(self.program, name) };
        self.locations.borrow_mut().insert(name.to_owned(), found);
        found
    }
}

#[cfg(all(test, target_os = "macos"))]
mod gl_tests {
    use crate::app::model_preview::harness::gl::headless_gl;
    use eframe::glow::{self, HasContext as _};

    /// [`Linked`](crate::app::model_preview::gl::program::Linked) finds a uniform the program
    /// has, answers none for one it lacks, and asks GL once for each.
    #[test]
    fn a_linked_program_looks_each_uniform_up_once() {
        use crate::app::model_preview::gl::link::link;
        use crate::app::model_preview::gl::program::Linked;
        const VERTEX: &str = "#version 410\nvoid main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }\n";
        const FRAGMENT: &str = "#version 410\nuniform vec4 tint;\nout vec4 colour;\nvoid main() { colour = tint; }\n";
        let gl = headless_gl();
        unsafe {
            let linked = Linked::new(link(&gl, &[(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)], &[]).unwrap());
            let tint = linked.location(&gl, "tint");
            assert!(tint.is_some());
            assert_eq!(linked.location(&gl, "missing"), None);
            // Deleted, the program has no uniforms left to find: only the cache answers.
            gl.delete_program(linked.program);
            assert_eq!(linked.location(&gl, "tint"), tint, "the first answer is kept");
            assert_eq!(linked.location(&gl, "missing"), None);
        }
    }
}
