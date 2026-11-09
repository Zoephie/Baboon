//! A frame no game shades: the slot's own draw of its batches, and nothing
//! of any game's over it.

use super::super::renderer::{FrameCx, GameRender};

/// The shared program's draw alone, for a frame of no game the preview
/// shades, or one whose game's shading cannot draw.
#[derive(Default)]
pub(in crate::app::model_preview) struct PlainRender;

impl GameRender for PlainRender {
    unsafe fn prepare(&mut self, _cx: &mut FrameCx<'_>) {}

    unsafe fn shade(&mut self, cx: &mut FrameCx<'_>) {
        // A preview no game is named for still resolves its shaders by their
        // own game, so it keeps every game's rule for what passes replace.
        let frame = cx.frame;
        unsafe {
            cx.core
                .draw_slot_batches(cx.gl, frame, cx.slot, &|core, material| {
                    (frame.game_shaders && core.gen3_material(frame, material).is_some())
                        || core.h2_replaces_slots(frame, material)
                })
        }
    }

    unsafe fn finish(&mut self, _cx: &mut FrameCx<'_>) {}
}
