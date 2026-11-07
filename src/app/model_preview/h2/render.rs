//! Halo 2's hooks into the dx9 pipeline.

use super::super::dx9::pipeline::Dx9Game;
use super::super::renderer::FrameCx;

/// Halo 2's own part of the dx9 shading: its cloth.
#[derive(Default)]
pub(in crate::app::model_preview) struct H2Game;

impl Dx9Game for H2Game {
    /// `cloth_update`, before anything draws the cloth's vertices.
    unsafe fn prepare(&mut self, cx: &mut FrameCx<'_>) {
        if cx.frame.preview.h3_cloth.first().is_some_and(|cloth| cloth.halo2()) {
            unsafe { cx.core.move_cloth(cx.gl, cx.frame, cx.slot) }
        }
    }
}
