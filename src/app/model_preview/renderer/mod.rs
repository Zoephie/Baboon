//! Depth-tested Glow rendering, markers, and preview geometry conversion.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::ModelGpuFrame;
use super::gl::scope::PostScope;
use eframe::egui;
use eframe::glow::{self, HasContext as _};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

mod game;
pub(in crate::app::model_preview) use game::*;
mod core;
pub(in crate::app::model_preview) use core::*;
mod slot;
pub(in crate::app::model_preview) use slot::*;
mod program;
pub(in crate::app::model_preview) use program::*;
mod grid;
pub(in crate::app::model_preview) use grid::*;

thread_local! {
    static MODEL_GL_RENDERER: RefCell<Option<CachedModelGlRenderer>> = const { RefCell::new(None) };
}

struct CachedModelGlRenderer {
    context_id: usize,
    renderer: Result<ModelGlRenderer, String>,
    /// Geometry whose drawing panicked, with what the panic said: it is not
    /// drawn again, and the panel says why.
    failed: HashMap<u64, String>,
}

/// Why drawing `geometry_id` stopped, if it panicked.
pub(super) fn model_gl_failure(geometry_id: u64) -> Option<String> {
    MODEL_GL_RENDERER.with(|cell| cell.borrow().as_ref()?.failed.get(&geometry_id).cloned())
}

pub(in crate::app::model_preview) fn paint_model_gl(
    info: egui::PaintCallbackInfo,
    painter: &eframe::egui_glow::Painter,
    frame: &ModelGpuFrame,
) {
    let gl = painter.gl();
    let viewport = info.viewport_in_pixels();
    let clip = info.clip_rect_in_pixels();
    let left = viewport.left_px.max(clip.left_px);
    let bottom = viewport.from_bottom_px.max(clip.from_bottom_px);
    let right = (viewport.left_px + viewport.width_px).min(clip.left_px + clip.width_px);
    let top =
        (viewport.from_bottom_px + viewport.height_px).min(clip.from_bottom_px + clip.height_px);
    if right <= left || top <= bottom {
        return;
    }

    unsafe {
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(left, bottom, right - left, top - bottom);
        gl.color_mask(true, true, true, true);
        gl.depth_mask(true);
        let [r, g, b] = frame.background;
        gl.clear_color(r, g, b, 1.0);
        gl.clear_depth_f32(1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
    }

    MODEL_GL_RENDERER.with(|cell| {
        let context_id = Arc::as_ptr(gl) as usize;
        let mut cached = cell.borrow_mut();
        if cached
            .as_ref()
            .is_none_or(|cached| cached.context_id != context_id)
        {
            let created = ModelGlRenderer::new(gl);
            if let Err(error) = &created {
                eprintln!("{error}");
            }
            *cached = Some(CachedModelGlRenderer {
                context_id,
                renderer: created,
                failed: HashMap::new(),
            });
        }
        let Some(CachedModelGlRenderer {
            renderer: Ok(renderer),
            failed,
            ..
        }) = cached.as_mut()
        else {
            return;
        };
        if failed.contains_key(&frame.geometry_id) {
            return;
        }
        // A panic part way through leaves the scene target bound; what the
        // frame drew into goes back, so the rest of the window still draws.
        let state = unsafe { PostScope::capture(gl) };
        let painted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            renderer.paint(gl, frame)
        }));
        if let Err(panic) = painted {
            unsafe {
                state.restore(gl);
                gl.use_program(None);
                gl.bind_vertex_array(None);
            }
            let message = crate::app::shell::worker::panic_detail(panic.as_ref()).to_owned();
            eprintln!("model preview: drawing crashed: {message}");
            failed.insert(frame.geometry_id, message);
        }
    });
}

/// The preview's renderer: what every game draws with ([`RenderCore`]),
/// and each game's own drawing ([`GameRender`]), which it hands the frame to.
pub(super) struct ModelGlRenderer {
    core: RenderCore,
    games: GameRenders,
}

impl ModelGlRenderer {
    pub(super) fn new(gl: &glow::Context) -> Result<Self, String> {
        Ok(Self {
            core: RenderCore::new(gl)?,
            games: GameRenders::default(),
        })
    }

    /// The shared gen3 targets' normal texture, between frames.
    #[cfg(test)]
    pub(in crate::app::model_preview) fn gen3_targets_normal(&self) -> Option<glow::NativeTexture> {
        self.core
            .gen3_targets
            .as_ref()
            .map(|targets| targets.normal)
    }

    /// Draw `frame`: its slot, the game's preparation, the scene target, the
    /// shared program and grid, the game's shading, the wireframe, what the
    /// game draws over it, and the scene put back.
    pub(super) unsafe fn paint(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        self.games.of(frame).begin(frame);
        unsafe {
            let Some(slot) = self.core.acquire_frame_slot(gl, frame) else {
                return;
            };
            let game = self.games.of(frame);
            game.prepare(&mut FrameCx {
                gl,
                frame,
                core: &mut self.core,
                slot,
            });
            let scene = self.core.begin_scene(gl, frame);
            // The game's passes need the scene target's stencil and copies;
            // without one, the frame draws as with them off.
            if scene.is_none() && frame.game_shaders && self.core.shaders_supported {
                let plain = ModelGpuFrame {
                    game_shaders: false,
                    ..frame.clone()
                };
                self.paint(gl, &plain);
                return;
            }
            self.core.begin_drawing(gl, frame, slot);
            let game = self.games.of(frame);
            if frame.render_mode.draws_shading() {
                gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
                gl.uniform_1_f32(Some(&self.core.unlit), 0.0);
                game.shade(&mut FrameCx {
                    gl,
                    frame,
                    core: &mut self.core,
                    slot,
                });
            }
            if frame.render_mode.draws_wireframe() {
                self.core.draw_wireframe(gl, frame, slot);
            }
            game.finish(&mut FrameCx {
                gl,
                frame,
                core: &mut self.core,
                slot,
            });
            self.core.end_frame(gl, scene);
        }
    }

    #[cfg(test)]
    pub(super) fn for_each_simulation(
        &self,
        geometry_id: u64,
        mut visit: impl FnMut(&str, &dyn std::fmt::Debug),
    ) {
        self.core.for_each_simulation(geometry_id, &mut visit);
        if let Some(slot) = self.core.slot_of(geometry_id) {
            self.games
                .each_simulation(slot, self.core.slot_uploads(slot), &mut visit);
        }
    }
}
