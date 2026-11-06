//! Halo 3's and ODST's place in the gen3 pipeline: their self shadow, from
//! a map generated through dx9 shadow passes and applied after the opaque
//! pass.

use eframe::glow::{self, HasContext};

use super::super::gen3::render::{Gen3Batch, Gen3Game};
use super::super::renderer::FrameCx;

/// Halo 3's and ODST's hooks: the pipeline's defaults, and the self shadow.
#[derive(Default)]
pub(in crate::app::model_preview) struct H3Game;

impl Gen3Game for H3Game {
    unsafe fn self_shadow(
        &mut self,
        cx: &mut FrameCx<'_>,
        _batches: &[Gen3Batch],
        order: &[usize],
        scene: glow::NativeFramebuffer,
    ) -> bool {
        if !cx.frame.self_shadow {
            return false;
        }
        // Generated from the light, for the apply pass after the opaque one.
        unsafe {
            cx.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene));
            let drawn = cx.core.draw_h2_shadow(cx.gl, cx.frame, cx.slot, order);
            cx.core.bind_slot_vertices(cx.gl, cx.slot);
            drawn
        }
    }
}
