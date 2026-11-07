//! The gen3 games' drawing, as the renderer hands each frame over: Halo 3,
//! ODST, Reach and Halo 4.

use std::sync::Arc;

use eframe::glow;

use super::super::RenderModelPreviewBatch;
use super::super::h3::render::H3Game;
use super::super::h4::render::H4Game;
use super::super::reach::lights::ReachLitLight;
use super::super::reach::render::ReachGame;
use super::super::common::plain::PlainRender;
use super::super::renderer::{FrameCx, GameRender, ModelGpuFrame, RenderCore};
use super::screen_effects::H3ScreenSettings;
use super::shaders::H3Material;
use super::sprites::{Gen3Sprite, Gen3Sprites};
use crate::core::game::GameId;

/// A batch the gen3 pipeline draws, with its material.
pub(in crate::app::model_preview) type Gen3Batch = (RenderModelPreviewBatch, Arc<H3Material>);

/// Where the gen3 games differ inside the shared pipeline
/// ([`RenderCore::draw_h3`]). Each default is what a game without the stage
/// does: Halo 3's and ODST's, whose own differences are the self shadow.
pub(in crate::app::model_preview) trait Gen3Game {
    /// Take out of `batches` what draws apart from the scene.
    fn split(&mut self, _frame: &ModelGpuFrame, _batches: &mut Vec<Gen3Batch>) {}

    /// The exposure stops the scene draws at, from the screen effects.
    fn stops(&self, _screen: &H3ScreenSettings) -> f32 {
        0.0
    }

    /// After the albedo and its decals, before the lights.
    unsafe fn after_albedo(&mut self, _cx: &mut FrameCx<'_>, _albedo: glow::NativeFramebuffer, _size: [i32; 2]) {}

    /// The self shadow ahead of the lit pass: whether a shadow map was drawn
    /// for the apply after the opaque pass.
    unsafe fn self_shadow(
        &mut self,
        _cx: &mut FrameCx<'_>,
        _batches: &[Gen3Batch],
        _order: &[usize],
        _scene: glow::NativeFramebuffer,
    ) -> bool {
        false
    }

    /// The lights the lit pass reads inline, which the screen lights draw too.
    fn frame_lights(&mut self, _cx: &mut FrameCx<'_>) -> Vec<ReachLitLight> {
        Vec::new()
    }

    /// What the opaque lit pass reads of the self shadow.
    fn lit_values(&mut self, _core: &mut RenderCore) {}

    /// Lights drawn over the screen after the static lighting.
    unsafe fn screen_lights(&mut self, _cx: &mut FrameCx<'_>, _lit: &[ReachLitLight], _accumulation: glow::NativeFramebuffer) {}

    /// What composites among the blended parts, where it lies farther than
    /// `nearer_than`.
    unsafe fn blended(&mut self, _cx: &mut FrameCx<'_>, _nearer_than: f32, _size: [i32; 2]) {}

    /// Whether the camo pass draws, and if so whether opaque.
    fn camo(&mut self, _core: &mut RenderCore, frame: &ModelGpuFrame) -> Option<bool> {
        frame.active_camo.then_some(false)
    }

    /// Shield passes after the shared shield impact.
    unsafe fn shield(&mut self, _cx: &mut FrameCx<'_>) {}

    /// Sprites after the rest, before the bloom.
    unsafe fn late_sprites(&mut self, _cx: &mut FrameCx<'_>) {}

    /// The bloom of what `accumulation` holds, drawn into `framebuffer`.
    unsafe fn bloom(
        &mut self,
        cx: &mut FrameCx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        size: [i32; 2],
    ) -> Option<glow::NativeTexture>;

    /// The lit scene and its bloom onto `framebuffer`.
    unsafe fn composite(
        &mut self,
        cx: &mut FrameCx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        screen: H3ScreenSettings,
        size: [i32; 2],
    );

    /// In the sprite pass, once a frame after the shared simulations: the
    /// game's own.
    unsafe fn advance_own_sprites(&mut self, _cx: &mut Gen3Sprites<'_>) {}

    /// In the sprite pass, in its far-to-near order: `sprite`, if it is the
    /// game's own to draw. Whether it was.
    unsafe fn draw_own_sprite(&mut self, _cx: &mut Gen3Sprites<'_>, _sprite: Gen3Sprite) -> bool {
        false
    }

    /// How long VISR has been on as of `time`, for `slot`'s `uploads`th
    /// model. Only Halo 3 and ODST read a vision.
    fn vision_on_for(&mut self, _slot: usize, _uploads: u64, _time: f32) -> f32 {
        0.0
    }

    /// Each simulation the game keeps for `slot` while it holds its
    /// `uploads`th model, by name.
    #[cfg(test)]
    fn each_simulation(&self, _slot: usize, _uploads: u64, _visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {}
}

/// Halo 3's, ODST's, Reach's and Halo 4's drawing: the slot's own draw, then
/// the gen3 pipeline with the game's hooks.
#[derive(Default)]
pub(in crate::app::model_preview) struct Gen3Render {
    h3: H3Game,
    reach: ReachGame,
    h4: H4Game,
}

impl Gen3Render {
    fn game(&mut self, game: Option<GameId>) -> &mut dyn Gen3Game {
        match game {
            Some(GameId::HaloReach) => &mut self.reach,
            Some(GameId::Halo4) => &mut self.h4,
            _ => &mut self.h3,
        }
    }
}

impl GameRender for Gen3Render {
    unsafe fn prepare(&mut self, _cx: &mut FrameCx<'_>) {}

    /// With no gen3 shading to draw (game shaders off, a mode without maps, a
    /// context short of GLSL 4.10), the plain draw.
    unsafe fn shade(&mut self, cx: &mut FrameCx<'_>) {
        if !cx.core.h3_draws(cx.frame) {
            return unsafe { PlainRender.shade(cx) };
        }
        unsafe {
            // Halo 3's and ODST's self shadow draws through dx9 shadow passes.
            cx.core.prepare_h2_programs(cx.gl, cx.frame);
            cx.core.draw_slot_batches(cx.gl, cx.frame, cx.slot);
            let order = cx.frame.visible_batches.clone();
            let game = self.game(cx.frame.preview.game);
            cx.core.draw_h3(game, cx.gl, cx.frame, cx.slot, &order);
        }
    }

    unsafe fn finish(&mut self, cx: &mut FrameCx<'_>) {
        if !cx.core.h3_draws(cx.frame) {
            unsafe {
                if cx.core.draws_object_sprites(cx.frame) {
                    use super::sprites::SpriteStage;
                    let game = self.game(cx.frame.preview.game);
                    cx.core.draw_gen3_sprites(game, cx.gl, cx.frame, cx.slot, SpriteStage::FartherThan(f32::NEG_INFINITY));
                    cx.core.draw_gen3_sprites(game, cx.gl, cx.frame, cx.slot, SpriteStage::Rest);
                }
            }
        }
    }

    #[cfg(test)]
    fn each_simulation(&self, slot: usize, uploads: u64, visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {
        let Self { h3, reach, h4 } = self;
        for game in [h3 as &dyn Gen3Game, reach, h4] {
            game.each_simulation(slot, uploads, visit);
        }
    }
}
