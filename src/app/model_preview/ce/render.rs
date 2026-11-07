//! Halo CE's hooks into the dx9 pipeline.

use super::super::dx9::passes::H2Layer;
use super::super::dx9::pipeline::Dx9Game;
use super::super::renderer::{FrameCx, advance_ce_effects, ce_sorts_transparents};

/// Halo CE's own part of the dx9 shading: its flags and effects, and its
/// transparents in the order the game sorts them.
#[derive(Default)]
pub(in crate::app::model_preview) struct CeGame;

impl Dx9Game for CeGame {
    unsafe fn prepare(&mut self, cx: &mut FrameCx<'_>) {
        unsafe {
            let (gl, frame, slot) = (cx.gl, cx.frame, cx.slot);
            if !frame.preview.ce_flags.is_empty() {
                cx.core.move_ce_flags(gl, frame, slot);
            }
            // Halo CE's effects advance once a frame, here, before anything
            // draws or lights by them.
            if !frame.preview.ce_effects.is_empty() {
                let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
                let uploaded = &mut cx.core.slots[slot];
                advance_ce_effects(uploaded, frame, bones);
                let made: Vec<super::lights::CeLight> =
                    uploaded.simulations.ce_particles.lights(&frame.preview.ce_effects).into_iter().map(|(_, _, light)| light).collect();
                if !made.is_empty() {
                    cx.core.ce_points_now = Some(frame.ce_point_lights_with(&made));
                }
            }
        }
    }

    /// `render_transparent_geometry`'s sort, when the batches carry it;
    /// else the layer.
    unsafe fn transparents(&mut self, cx: &mut FrameCx<'_>, order: &[usize]) {
        unsafe {
            if ce_sorts_transparents(cx.frame) {
                cx.core.draw_ce_transparents(cx.gl, cx.frame, cx.slot);
            } else {
                cx.core.draw_h2_passes(cx.gl, cx.frame, cx.slot, order, H2Layer::Transparent, None);
            }
        }
    }
}
