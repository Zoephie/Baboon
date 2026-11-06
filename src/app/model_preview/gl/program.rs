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
