//! Linking a pass's program, with the vertex attributes it binds, and the program cache.

use super::super::super::{Rc, RenderCore};
use super::{Gen3Pass, Gen3State, Linked};
use eframe::glow;
use eframe::glow::HasContext as _;
use std::sync::Arc;

/// Which attributes of the preview's vertex each semantic reads.
const ATTRIBUTES: [(&str, &str); 7] = [
    ("a_POSITION", "a_position"),
    ("a_TEXCOORD0", "a_texcoord"),
    ("a_NORMAL", "a_normal"),
    ("a_TANGENT", "a_tangent"),
    ("a_BINORMAL", "a_binormal"),
    ("a_BLENDINDICES", "a_node_indices"),
    ("a_BLENDWEIGHT", "a_node_weights"),
];

/// Where `static_prt_*`'s coefficients arrive, past the preview's own.
pub(in crate::app::model_preview) const PRT_ATTRIBUTES: [u32; 3] = [11, 12, 13];

const PRT_NAMES: [&str; 3] = ["a_BLENDWEIGHT1", "a_BLENDWEIGHT2", "a_BLENDWEIGHT3"];

/// A linked pass and where its uniforms landed.
pub(in crate::app::model_preview) struct Gen3Program {
    linked: Linked,
}

impl std::ops::Deref for Gen3Program {
    type Target = Linked;

    fn deref(&self) -> &Linked {
        &self.linked
    }
}

impl Gen3State {
    /// Run `draw` with `pass`'s program, linking it the first time, and keep
    /// it for the next frame whatever `draw` returns. `None` when the pass
    /// does not link. The program leaves the cache while `draw` runs, so
    /// `draw` can use the renderer too.
    pub(in crate::app::model_preview) unsafe fn with_program<R>(
        &mut self,
        core: &mut RenderCore,
        gl: &glow::Context,
        pass: &Arc<Gen3Pass>,
        draw: impl FnOnce(&mut Self, &mut RenderCore, &Gen3Program) -> R,
    ) -> Option<R> {
        let program = unsafe { self.program(core, gl, pass) }?;
        Some(draw(self, core, &program))
    }

    /// `pass`'s program, linked the first time its source is asked for. It
    /// stays in the cache while lent, so a pass drawn inside another's draw
    /// finds it.
    unsafe fn program(
        &mut self,
        core: &mut RenderCore,
        gl: &glow::Context,
        pass: &Arc<Gen3Pass>,
    ) -> Option<Rc<Gen3Program>> {
        let key = Arc::as_ptr(pass) as usize;
        let known = match self.programs.get(&key) {
            Some((_, known)) => known.clone(),
            None => {
                // A new pass: the old ones only these entries still hold go.
                self.programs
                    .retain(|_, (held, _)| Arc::strong_count(held) > 1);
                let source = (pass.vertex.clone(), pass.pixel.clone());
                let known = match self.linked.get(&source) {
                    Some(known) => known.clone(),
                    None => {
                        let linked = unsafe { link_pass(core, gl, pass) };
                        self.linked.insert(source, linked.clone());
                        linked
                    }
                };
                self.programs.insert(key, (Arc::clone(pass), known.clone()));
                known
            }
        };
        match known {
            Ok(program) => Some(program),
            Err(error) => {
                core.note(format!("a pass did not link: {error}"));
                None
            }
        }
    }
}

/// `pass`'s stages linked, with the attributes the preview's vertices bind.
unsafe fn link_pass(
    core: &RenderCore,
    gl: &glow::Context,
    pass: &Gen3Pass,
) -> super::super::state::LinkedPass {
    unsafe {
        let stages = [
            (glow::VERTEX_SHADER, pass.vertex.as_str()),
            (glow::FRAGMENT_SHADER, pass.pixel.as_str()),
        ];
        let mut attributes = Vec::new();
        for (name, preview) in ATTRIBUTES {
            if let Some(location) = gl.get_attrib_location(core.program, preview) {
                attributes.push((location, name));
            }
        }
        attributes.extend(PRT_ATTRIBUTES.into_iter().zip(PRT_NAMES));
        match super::super::super::gl::link::link(gl, &stages, &attributes) {
            Ok(program) => Ok(Rc::new(Gen3Program {
                linked: Linked::new(program),
            })),
            Err(error) => {
                eprintln!("h3 program: {error}");
                Err(error)
            }
        }
    }
}
