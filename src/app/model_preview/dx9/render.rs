//! A dx9 game's drawing, as the renderer hands each frame over: the slot's
//! own draw, the dx9 passes with the game's hooks, then its sprites and
//! bloom. The game table holds one for each dx9 game.

use super::super::frame::ModelGpuFrame;
use super::super::renderer::{FrameCx, GameRender};
use super::pipeline::Dx9Game;

/// A dx9 game's drawing, through `G`'s hooks.
#[derive(Default)]
pub(in crate::app::model_preview) struct Dx9Pipeline<G> {
    game: G,
    /// What the bloom covers, from the shading to the finish.
    bloom_order: Option<Vec<usize>>,
}

impl<G: Dx9Game> GameRender for Dx9Pipeline<G> {
    fn begin(&mut self, frame: &ModelGpuFrame) {
        self.game.begin(frame);
    }

    unsafe fn prepare(&mut self, cx: &mut FrameCx<'_>) {
        unsafe { self.game.prepare(cx) }
    }

    unsafe fn shade(&mut self, cx: &mut FrameCx<'_>) {
        unsafe {
            cx.core.prepare_dx9_programs(cx.gl, cx.frame);
            cx.core
                .draw_slot_batches(cx.gl, cx.frame, cx.slot, &|core, material| {
                    core.dx9_replaces_slots(cx.frame, material)
                });
            self.bloom_order = cx
                .core
                .draw_dx9_passes(&mut self.game, cx.gl, cx.frame, cx.slot);
        }
    }

    unsafe fn finish(&mut self, cx: &mut FrameCx<'_>) {
        unsafe {
            if cx.core.draws_object_sprites(cx.frame) {
                cx.core
                    .draw_dx9_sprites(&mut self.game, cx.gl, cx.frame, cx.slot);
            }
            if let Some(order) = self.bloom_order.take() {
                self.game.bloom(cx, order);
            }
        }
    }

    #[cfg(test)]
    fn each_simulation(
        &self,
        slot: usize,
        uploads: u64,
        visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug),
    ) {
        self.game.each_simulation(slot, uploads, visit);
    }
}
