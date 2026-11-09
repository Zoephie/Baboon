//! Halo CE's hooks into the dx9 pipeline, and its state for each slot.

use super::super::common::slots::GameSlots;
use super::super::common::view::ViewFrame;
use super::super::dx9::passes::H2Layer;
use super::super::dx9::pipeline::Dx9Game;
use super::super::dx9::sprites::Dx9Sprites;
use super::super::renderer::FrameCx;
use super::draw::{CeEffectGl, advance_ce_effects, ce_groups, ce_has_groups, ce_sorts_transparents, draw_ce_group};
use crate::app::model_preview::common::sprite_gl::{ce_flare_occlusion, draw_flare_sprites};
use super::slot::CeSlot;
use eframe::glow::{self, HasContext as _};

/// Halo CE's own part of the dx9 shading: its flags and effects, its
/// transparents in the order the game sorts them, its effects' groups and
/// its lights' flares.
#[derive(Default)]
pub(in crate::app::model_preview) struct CeGame {
    slots: GameSlots<CeSlot>,
    /// The `effect` shader's program, made on first use.
    effect_gl: Option<Option<CeEffectGl>>,
}

impl Dx9Game for CeGame {
    unsafe fn prepare(&mut self, cx: &mut FrameCx<'_>) {
        unsafe {
            let (gl, frame, slot) = (cx.gl, cx.frame, cx.slot);
            let ce = self.slots.of(slot, cx.core.slot_uploads(slot));
            if !frame.preview.ce.flags.is_empty() {
                cx.core.move_ce_flags(gl, frame, slot, ce);
            }
            // Halo CE's effects advance once a frame, here, before anything
            // draws or lights by them.
            if !frame.preview.ce.effects.is_empty() {
                let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
                advance_ce_effects(ce, frame, bones);
                let made: Vec<super::lights::CeLight> =
                    ce.simulations.particles.lights(&frame.preview.ce.effects).into_iter().map(|(_, _, light)| light).collect();
                if !made.is_empty() {
                    cx.core.dx9.ce_points_now = Some(frame.ce_point_lights_with(&made));
                }
            }
            // Before anything draws: the transparents and the sprites both
            // draw the effects' groups.
            if cx.core.draws_object_sprites(frame) {
                ce.sync_textures(gl, frame);
            }
        }
    }

    /// Halo CE's renderer has no bloom: only the remaster's does.
    unsafe fn bloom(&mut self, _cx: &mut FrameCx<'_>, _order: Vec<usize>) {}

    /// `render_transparent_geometry`'s sort, when the batches carry it;
    /// else the layer.
    unsafe fn transparents(&mut self, cx: &mut FrameCx<'_>, order: &[usize]) {
        unsafe {
            if ce_sorts_transparents(cx.frame) {
                let ce = self.slots.of(cx.slot, cx.core.slot_uploads(cx.slot));
                cx.core.draw_ce_transparents(cx.gl, cx.frame, cx.slot, ce, &mut self.effect_gl);
            } else {
                cx.core.draw_h2_passes(cx.gl, cx.frame, cx.slot, order, H2Layer::Transparent, None);
            }
        }
    }

    /// The effects' groups, when the transparents did not sort them in.
    unsafe fn draw_own_sprites(&mut self, cx: &mut Dx9Sprites<'_>) {
        let Dx9Sprites { gl, frame, flares, view, bones, program, slot, uploads, .. } = *cx;
        if !ce_has_groups(&frame.preview) || ce_sorts_transparents(frame) {
            return;
        }
        let ce = self.slots.of(slot, uploads);
        unsafe {
            for group in ce_groups(gl, ce, frame, bones, view) {
                draw_ce_group(gl, &mut self.effect_gl, program, flares, ce, &frame.camera, group);
            }
            gl.use_program(Some(flares.program));
            gl.bind_vertex_array(Some(flares.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(flares.buffer));
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        }
    }

    /// The lights' flares, and those of the lights the effects make.
    unsafe fn draw_flares(&mut self, cx: &mut Dx9Sprites<'_>) {
        let Dx9Sprites { gl, frame, flares, view, bones, slot, uploads, .. } = *cx;
        let view: &ViewFrame = view;
        let camera = &frame.camera;
        let ce = self.slots.of(slot, uploads);
        unsafe {
            for (index, light) in frame.preview.ce.lights.iter().enumerate() {
                let state = frame.object_states.get(usize::from(light.object));
                let outputs = state.map_or([0.0; 4], |state| state.outputs);
                let colors = state.map_or([[1.0; 3]; 4], |state| state.change_colors);
                let Some(patch) = light.occlusion_patch(&outputs, &colors, bones, view) else {
                    continue;
                };
                let Some(occlusion) = ce_flare_occlusion(gl, flares, camera, view, &patch) else {
                    continue;
                };
                let sprites = light.sprites(&outputs, &colors, bones, view, frame.time, occlusion);
                let Some(textures) = ce.textures.light_flares.get(index) else {
                    continue;
                };
                draw_flare_sprites(gl, flares, camera, view, &sprites, textures);
            }
            for (effect, def, light) in ce.simulations.particles.lights(&frame.preview.ce.effects) {
                let state = frame.object_states.get(usize::from(light.object));
                let outputs = state.map_or([0.0; 4], |state| state.outputs);
                let colors = state.map_or([[1.0; 3]; 4], |state| state.change_colors);
                let Some(patch) = light.occlusion_patch(&outputs, &colors, bones, view) else {
                    continue;
                };
                let Some(occlusion) = ce_flare_occlusion(gl, flares, camera, view, &patch) else {
                    continue;
                };
                let sprites = light.sprites(&outputs, &colors, bones, view, frame.time, occlusion);
                let Some(textures) = ce.textures.effect_flares.get(&effect).and_then(|lights| lights.get(def)) else {
                    continue;
                };
                draw_flare_sprites(gl, flares, camera, view, &sprites, textures);
            }
        }
    }

    #[cfg(test)]
    fn each_simulation(&self, slot: usize, uploads: u64, visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {
        if let Some(ce) = self.slots.current(slot, uploads) {
            ce.simulations.each(visit);
        }
    }
}
