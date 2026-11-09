//! The game table: what each game's renderer is handed per frame, the trait it implements, and the route from a preview to its game.

use super::*;
use eframe::glow;

/// What a game's drawing is handed each frame.
pub(in crate::app::model_preview) struct FrameCx<'a> {
    pub(in crate::app::model_preview) gl: &'a glow::Context,
    pub(in crate::app::model_preview) frame: &'a ModelGpuFrame,
    pub(in crate::app::model_preview) core: &'a mut RenderCore,
    /// The frame's slot in `core`.
    pub(in crate::app::model_preview) slot: usize,
}

/// A game's drawing: the renderer runs the frame and hands these steps to
/// the game it is from.
pub(in crate::app::model_preview) trait GameRender {
    /// First, before the frame finds its slot: what the last frame left
    /// for this model and must not outlive a frame that draws nothing.
    fn begin(&mut self, _frame: &ModelGpuFrame) {}
    /// Before the scene target: what moves the geometry or makes lights.
    unsafe fn prepare(&mut self, cx: &mut FrameCx<'_>);
    /// The shading, when the mode draws any.
    unsafe fn shade(&mut self, cx: &mut FrameCx<'_>);
    /// After the wireframe: what draws over it.
    unsafe fn finish(&mut self, cx: &mut FrameCx<'_>);

    /// Each simulation the game keeps for `slot` while it holds its `uploads`th
    /// model, by name: what the G-sim gate reads.
    #[cfg(test)]
    fn each_simulation(&self, _slot: usize, _uploads: u64, _visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {}
}

/// Each game's drawing, by the game a preview is from.
#[derive(Default)]
pub(super) struct GameRenders {
    pub(super) plain: common::plain::PlainRender,
    pub(super) dx9: dx9::render::Dx9Render,
    pub(super) gen3: gen3::render::Gen3Render,
}

impl GameRenders {
    /// Each game's own simulations for `slot`.
    #[cfg(test)]
    pub(super) fn each_simulation(&self, slot: usize, uploads: u64, visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {
        let Self { plain, dx9, gen3 } = self;
        for game in [plain as &dyn GameRender, dx9, gen3] {
            game.each_simulation(slot, uploads, visit);
        }
    }

    /// The drawing for `frame`, by [`Route::of`] its preview.
    pub(super) fn of(&mut self, frame: &ModelGpuFrame) -> &mut dyn GameRender {
        match Route::of(&frame.preview) {
            Route::Gen3 => &mut self.gen3,
            Route::Dx9 => &mut self.dx9,
            Route::Plain => &mut self.plain,
        }
    }
}

/// Which drawing a preview goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Route {
    Gen3,
    Dx9,
    Plain,
}

impl Route {
    /// By the game the preview is from. One of no game this draws draws as
    /// Halo 3's when its tag is gen3 (from a source that names no game), and
    /// plainly when no tag gave it (Chimp's meshes).
    pub(super) fn of(preview: &RenderModelPreview) -> Self {
        match preview.game {
            Some(GameId::Halo3 | GameId::Halo3Odst | GameId::HaloReach | GameId::Halo4) => Self::Gen3,
            Some(GameId::HaloCe | GameId::Halo2) => Self::Dx9,
            _ if preview.generation == Some(blam_tags::game::Game::Halo3) => Self::Gen3,
            _ => Self::Plain,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each game's preview goes to its family's drawing; one of no game goes
    /// to gen3 if its tag is gen3, and is drawn plainly if no tag gave it.
    #[test]
    fn a_preview_routes_by_its_game_then_its_generation() {
        use blam_tags::game::Game;
        let route = |game: Option<GameId>, generation: Option<Game>| {
            Route::of(&RenderModelPreview { game, generation, ..Default::default() })
        };
        for game in [GameId::Halo3, GameId::Halo3Odst, GameId::HaloReach, GameId::Halo4] {
            assert_eq!(route(Some(game), Some(Game::Halo3)), Route::Gen3, "{game:?}");
        }
        assert_eq!(route(Some(GameId::HaloCe), Some(Game::Halo1)), Route::Dx9);
        assert_eq!(route(Some(GameId::Halo2), Some(Game::Halo2)), Route::Dx9);
        assert_eq!(route(None, Some(Game::Halo3)), Route::Gen3, "a gen3 tag of no game");
        assert_eq!(route(None, None), Route::Plain, "geometry no tag gave");
    }

    /// A frame begins by forgetting that the model's Halo CE event effects
    /// were running, before it looks for a slot: a frame that finds none
    /// draws nothing, so it must not leave the viewport asking for frames.
    #[test]
    fn a_halo_ce_frame_forgets_its_running_events_before_it_finds_a_slot() {
        use super::super::super::ce::draw::{CE_EVENTS_RUNNING, ce_events_running};
        let preview = RenderModelPreview { game: Some(GameId::HaloCe), ..Default::default() };
        let data = model_preview_data(String::new(), String::new(), preview, Vec::new());
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64.0, 64.0));
        let frame = build_frame(&data, &ModelPreviewState::default(), rect, 0.0, [0.0; 3], &[]);
        CE_EVENTS_RUNNING.with(|running| running.borrow_mut().insert(frame.geometry_id));
        assert!(ce_events_running(frame.geometry_id));
        GameRenders::default().of(&frame).begin(&frame);
        assert!(!ce_events_running(frame.geometry_id));
    }
}
