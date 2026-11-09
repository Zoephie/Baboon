//! What a game binds on a dx9 pass beside the shared uniforms: its own
//! lights, whether its light follows the camera, and its own vertex
//! lighting. The shared binder calls each where the game's own used to sit,
//! so the uniforms go in the order they did.

use super::super::ModelGpuFrame;
use super::super::gl::program::Linked;
use super::passes::{Dx9DrawPass, Dx9Light};
use eframe::glow;

/// A game's own part of a dx9 pass's uniforms. Each does nothing unless the
/// game has its own.
pub(in crate::app::model_preview) trait Dx9Binds {
    /// First of a pass's uniforms: the game's own lights on `material`.
    ///
    /// # Safety
    ///
    /// `program` must be in use, in the current context.
    unsafe fn bind_own_lights(
        &self,
        _gl: &glow::Context,
        _frame: &ModelGpuFrame,
        _material: usize,
        _program: &Linked,
        _pass: &Dx9DrawPass,
    ) {
    }

    /// Whether `pass`'s direct light is the camera's rather than its own.
    fn light_follows_camera(&self, _frame: &ModelGpuFrame, _pass: &Dx9DrawPass) -> bool {
        false
    }

    /// After the view's axes: the game's own vertex lighting, lit by
    /// `light` when one is drawing.
    ///
    /// # Safety
    ///
    /// `program` must be in use, in the current context.
    unsafe fn bind_own_vertex_lighting(
        &self,
        _gl: &glow::Context,
        _frame: &ModelGpuFrame,
        _program: &Linked,
        _light: Option<&Dx9Light>,
    ) {
    }
}

/// A drawing with nothing of its own to bind, such as the shadow passes a
/// game outside dx9 borrows.
pub(in crate::app::model_preview) struct NoOwnBinds;

impl Dx9Binds for NoOwnBinds {}
