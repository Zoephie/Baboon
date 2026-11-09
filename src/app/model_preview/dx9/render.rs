//! The dx9 games' drawing, as the renderer hands each frame over: Halo CE
//! and Halo 2.

use super::super::ce::render::CeGame;
use super::super::h2::render::H2Game;
use super::super::renderer::{FrameCx, GameRender};
use super::pipeline::Dx9Game;
use crate::core::game::GameId;

/// Halo CE's and Halo 2's drawing: the slot's own draw, the dx9 passes with the game's
/// hooks, then its sprites and bloom.
#[derive(Default)]
pub(in crate::app::model_preview) struct Dx9Render {
    ce: CeGame,
    h2: H2Game,
    /// What the bloom covers, from the shading to the finish.
    bloom_order: Option<Vec<usize>>,
}

impl Dx9Render {
    fn game(&mut self, game: Option<GameId>) -> &mut dyn Dx9Game {
        match game {
            Some(GameId::HaloCe) => &mut self.ce,
            _ => &mut self.h2,
        }
    }
}

impl GameRender for Dx9Render {
    unsafe fn prepare(&mut self, cx: &mut FrameCx<'_>) {
        unsafe { self.game(cx.frame.preview.game).prepare(cx) }
    }

    unsafe fn shade(&mut self, cx: &mut FrameCx<'_>) {
        unsafe {
            cx.core.prepare_h2_programs(cx.gl, cx.frame);
            cx.core.draw_slot_batches(cx.gl, cx.frame, cx.slot, &|core, material| core.h2_replaces_slots(cx.frame, material));
            let game = self.game(cx.frame.preview.game);
            self.bloom_order = cx.core.draw_dx9_passes(game, cx.gl, cx.frame, cx.slot);
        }
    }

    unsafe fn finish(&mut self, cx: &mut FrameCx<'_>) {
        unsafe {
            if cx.core.draws_object_sprites(cx.frame) {
                let game = self.game(cx.frame.preview.game);
                cx.core.draw_dx9_sprites(game, cx.gl, cx.frame, cx.slot);
            }
            let order = self.bloom_order.take();
            cx.core.draw_dx9_bloom(cx.gl, cx.frame, cx.slot, order);
        }
    }

    #[cfg(test)]
    fn each_simulation(&self, slot: usize, uploads: u64, visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {
        let Self { ce, h2, .. } = self;
        for game in [ce as &dyn Dx9Game, h2] {
            game.each_simulation(slot, uploads, visit);
        }
    }
}
