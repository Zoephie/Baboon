//! What Halo 4 keeps for each of the renderer's slots: its tracers' trails,
//! and the programs, bitmaps and state buffer its effects draw with.

use eframe::glow;

use super::super::common::slots::GameSlot;
use super::super::frame::ModelGpuFrame;

/// Halo 4's state for one of the renderer's slots.
#[derive(Default)]
pub(in crate::app::model_preview) struct H4Slot {
    /// The trailing tracers between frames.
    pub(in crate::app::model_preview) trails: super::tracers::H4TracerField,
    /// The effects' programs, bitmaps and state buffer, of `effects_for`.
    pub(in crate::app::model_preview) effects: super::effects::gl::H4EffectGl,
    effects_for: Option<u64>,
}

impl GameSlot for H4Slot {
    fn restart(&mut self) {
        self.trails = Default::default();
    }
}

impl H4Slot {
    /// The effects' GL state let go when the slot's geometry is another.
    pub(in crate::app::model_preview) unsafe fn sync_effects(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        if self.effects_for != Some(frame.geometry_id) {
            unsafe { self.effects.clear(gl) };
            self.effects_for = Some(frame.geometry_id);
        }
    }
}
