//! The gen3 games' drawing, as the renderer hands each frame over: Halo 3,
//! ODST, Reach and Halo 4.

use std::sync::Arc;

use eframe::glow;

use super::super::RenderModelPreviewBatch;
use super::super::common::plain::PlainRender;
use super::super::frame::ModelGpuFrame;
use super::super::renderer::{FrameCx, GameRender, RenderCore};
use super::effects::screen::H3ScreenSettings;
use super::pipeline::{FrameValues, PictureCopies, halo3_frame_values};
use super::shaders::H3Material;
use super::sprites::{Gen3Sprite, Gen3Sprites};
use super::state::Gen3State;

/// A batch the gen3 pipeline draws, with its material.
pub(in crate::app::model_preview) type Gen3Batch = (RenderModelPreviewBatch, Arc<H3Material>);

/// What a gen3 hook draws with: the frame's `FrameCx` and the gen3
/// pipeline's own state.
pub(in crate::app::model_preview) struct Gen3Cx<'a> {
    pub(in crate::app::model_preview) gl: &'a glow::Context,
    pub(in crate::app::model_preview) frame: &'a ModelGpuFrame,
    pub(in crate::app::model_preview) core: &'a mut RenderCore,
    pub(in crate::app::model_preview) slot: usize,
    pub(in crate::app::model_preview) state: &'a mut Gen3State,
}

/// Where the gen3 games differ inside the shared pipeline
/// ([`Gen3State::draw`]). Each default is what a game without the stage
/// does: Halo 3's and ODST's, whose own differences are the self shadow.
pub(in crate::app::model_preview) trait Gen3Game {
    /// Take out of `batches` what draws apart from the scene.
    fn split(&mut self, _frame: &ModelGpuFrame, _batches: &mut Vec<Gen3Batch>) {}

    /// What the engine's globals hold for the game's passes.
    fn frame_values(&self) -> FrameValues {
        halo3_frame_values
    }

    /// The exposure stops the scene draws at, from the screen effects.
    fn stops(&self, _screen: &H3ScreenSettings) -> f32 {
        0.0
    }

    /// After the albedo and its decals, before the lights.
    unsafe fn after_albedo(&mut self, _cx: &mut Gen3Cx<'_>, _albedo: glow::NativeFramebuffer, _size: [i32; 2]) {}

    /// The self shadow ahead of the lit pass.
    unsafe fn self_shadow(
        &mut self,
        _cx: &mut Gen3Cx<'_>,
        _batches: &[Gen3Batch],
        _order: &[usize],
        _scene: glow::NativeFramebuffer,
    ) {
    }

    /// After the opaque parts' lit pass, the batches in `order`: what reads
    /// it, as Halo 3's self shadow's apply does.
    unsafe fn after_opaque(&mut self, _cx: &mut Gen3Cx<'_>, _order: &[usize]) {}

    /// After the static lighting, its blend set additive: the object's own
    /// lights, each on its own over `batches`.
    unsafe fn dynamic_lights(&mut self, _cx: &mut Gen3Cx<'_>, _batches: &[Gen3Batch]) {}

    /// The lights the lit pass reads inline, as `simple_lights` rows; the
    /// game draws the rest over the screen.
    fn inline_lights(&mut self, _cx: &mut Gen3Cx<'_>) -> Vec<[[f32; 4]; 5]> {
        Vec::new()
    }

    /// What the opaque lit pass reads of the self shadow.
    fn lit_values(&mut self, _cx: &mut Gen3Cx<'_>) {}

    /// Lights drawn over the screen after the static lighting.
    unsafe fn screen_lights(&mut self, _cx: &mut Gen3Cx<'_>, _accumulation: glow::NativeFramebuffer) {}

    /// What composites among the blended parts, where it lies farther than
    /// `nearer_than`.
    unsafe fn blended(&mut self, _cx: &mut Gen3Cx<'_>, _nearer_than: f32, _size: [i32; 2]) {}

    /// Whether the camo pass draws, and if so whether opaque.
    fn camo(&mut self, cx: &mut Gen3Cx<'_>) -> Option<bool> {
        cx.frame.active_camo.then_some(false)
    }

    /// Shield passes after the shared shield impact.
    unsafe fn shield(&mut self, _cx: &mut Gen3Cx<'_>) {}

    /// Sprites after the rest, before the bloom.
    unsafe fn late_sprites(&mut self, _cx: &mut Gen3Cx<'_>) {}

    /// The bloom of what `accumulation` holds, drawn into `framebuffer`.
    unsafe fn bloom(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        size: [i32; 2],
    ) -> Option<glow::NativeTexture>;

    /// The lit scene and its bloom onto `framebuffer`.
    unsafe fn composite(
        &mut self,
        cx: &mut Gen3Cx<'_>,
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

    /// Whether the game draws over the finished picture, around its screen
    /// shaders and blur (Halo 3's and ODST's VISR and motion suck).
    fn picture_effects(&self, _frame: &ModelGpuFrame, _screen: &H3ScreenSettings) -> bool {
        false
    }

    /// Over the finished picture, before its screen shaders.
    unsafe fn before_screen_shaders(&mut self, _cx: &mut Gen3Cx<'_>, _screen: &H3ScreenSettings, _copies: &PictureCopies) {}

    /// Over the finished picture, after its blur.
    unsafe fn after_blur(&mut self, _cx: &mut Gen3Cx<'_>, _screen: &H3ScreenSettings, _copies: &PictureCopies) {}

    /// Each simulation the game keeps for `slot` while it holds its
    /// `uploads`th model, by name.
    #[cfg(test)]
    fn each_simulation(&self, _slot: usize, _uploads: u64, _visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {}
}

/// A gen3 game's drawing: the slot's own draw, then the gen3 pipeline with
/// `G`'s hooks. Each gen3 game has its own, which the renderer's game table
/// picks.
#[derive(Default)]
pub(in crate::app::model_preview) struct Gen3Pipeline<G> {
    /// The pipeline's own state, between and within frames.
    state: Gen3State,
    game: G,
}

impl<G: Gen3Game> Gen3Pipeline<G> {
    /// The pipeline's state, and the game's hooks.
    fn parts(&mut self) -> (&mut Gen3State, &mut dyn Gen3Game) {
        (&mut self.state, &mut self.game)
    }
}

impl<G: Gen3Game> GameRender for Gen3Pipeline<G> {
    /// The scene's depth copy is the frame's own, made while its picture draws.
    unsafe fn prepare(&mut self, _cx: &mut FrameCx<'_>) {
        self.state.depth_copy = None;
        // A frame that stopped before its last sprites leaves no queue for
        // this one to draw from.
        self.state.sprite_queue = None;
    }

    /// With no gen3 shading to draw (game shaders off, a mode without maps, a
    /// context short of GLSL 4.10), the plain draw.
    unsafe fn shade(&mut self, cx: &mut FrameCx<'_>) {
        if !cx.core.h3_draws(cx.frame) {
            return unsafe { PlainRender.shade(cx) };
        }
        self.state.frame_values = self.game.frame_values();
        unsafe {
            // Halo 3's and ODST's self shadow draws through dx9 shadow passes.
            cx.core.prepare_h2_programs(cx.gl, cx.frame);
            // A material with translated passes draws through them; the rest
            // carry only the shadow passes, which replace nothing.
            let frame = cx.frame;
            cx.core.draw_slot_batches(cx.gl, frame, cx.slot, &|core, material| core.h3_material(frame, material).is_some());
            let order = cx.frame.visible_batches.clone();
            let (state, game) = self.parts();
            state.draw(cx.core, game, cx.gl, cx.frame, cx.slot, &order);
        }
    }

    unsafe fn finish(&mut self, cx: &mut FrameCx<'_>) {
        if !cx.core.h3_draws(cx.frame) {
            unsafe {
                if cx.core.draws_object_sprites(cx.frame) {
                    use super::sprites::SpriteStage;
                    let (state, game) = self.parts();
                    state.draw_sprites(cx.core, game, cx.gl, cx.frame, cx.slot, SpriteStage::FartherThan(f32::NEG_INFINITY));
                    state.draw_sprites(cx.core, game, cx.gl, cx.frame, cx.slot, SpriteStage::Rest);
                }
            }
        }
    }

    #[cfg(test)]
    fn each_simulation(&self, slot: usize, uploads: u64, visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {
        self.game.each_simulation(slot, uploads, visit);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use crate::app::model_preview::harness::kit::kit_source;
    use crate::app::model_preview::harness::shot::{Shot, ShotOptions, shoot};
    use crate::core::game::GameId;

    /// A gen3 tag from a source that names no game draws through Halo 3's
    /// hooks on the legacy route, and so draws as a Halo 3 frame does: its
    /// sprites once, from inside the gen3 pipeline, and not again after it.
    #[test]
    fn a_gen3_frame_with_no_game_draws_as_halo_3s_does() {
        let Some(source) = kit_source(GameId::Halo3) else {
            return;
        };
        let shot = Shot {
            model: "objects\\vehicles\\warthog\\warthog".to_owned(),
            shaders: true,
            lights: true,
            yaw: 0.6,
            pitch: 0.25,
            kind: Some("vehicle".to_owned()),
        };
        let options = ShotOptions { time: 3.0, ..ShotOptions::new(GameId::Halo3) };
        let without = ShotOptions { no_game: true, ..options.clone() };
        let shown = shoot(&source, &[(shot.clone(), options), (shot, without)]);
        assert!(shown[0].pixels.iter().any(|byte| *byte != 0), "the warthog drew");
        assert!(shown[0].pixels == shown[1].pixels, "the frame with no game drew differently");
    }
}
