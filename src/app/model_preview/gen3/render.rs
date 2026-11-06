//! The gen3 games' drawing, as the renderer hands each frame over: Halo 3,
//! ODST, Reach and Halo 4.

use super::super::renderer::{FrameCx, GameRender, LegacyRender};

/// Halo 3's, ODST's, Reach's and Halo 4's drawing: the slot's own draw, then
/// the gen3 pipeline.
#[derive(Default)]
pub(in crate::app::model_preview) struct Gen3Render {
    /// A context that cannot draw the gen3 shaders draws as before.
    fallback: LegacyRender,
}

impl GameRender for Gen3Render {
    unsafe fn prepare(&mut self, cx: &mut FrameCx<'_>) {
        if !cx.core.h3_draws(cx.frame) {
            unsafe { self.fallback.prepare(cx) }
        }
    }

    unsafe fn shade(&mut self, cx: &mut FrameCx<'_>) {
        if !cx.core.h3_draws(cx.frame) {
            return unsafe { self.fallback.shade(cx) };
        }
        unsafe {
            // Halo 3's and ODST's self shadow draws through dx9 shadow passes.
            cx.core.prepare_h2_programs(cx.gl, cx.frame);
            cx.core.draw_slot_batches(cx.gl, cx.frame, cx.slot);
            let order = cx.frame.visible_batches.clone();
            cx.core.draw_h3(cx.gl, cx.frame, cx.slot, &order);
        }
    }

    unsafe fn finish(&mut self, cx: &mut FrameCx<'_>) {
        if !cx.core.h3_draws(cx.frame) {
            unsafe { self.fallback.finish(cx) }
        }
    }
}
