//! Halo 2's hooks into the dx9 pipeline, and its state for each slot.

use eframe::glow::{self, HasContext as _};

use super::super::common::features::paths;
use super::super::common::slots::GameSlots;
use super::super::common::sprites::draw_sprite_quad;
use super::super::dx9::passes::Dx9Layer;
use super::super::dx9::pipeline::Dx9Game;
use super::super::dx9::sprites::Dx9Sprites;
use super::super::gen3::pipeline::gen3_set_blend;
use super::super::gen3::shaders::Gen3Blend;
use super::super::renderer::FrameCx;
use super::slot::H2Slot;
use crate::app::model_preview::common::sprite_gl::{draw_flare_sprites, sprite_occlusion};

/// Halo 2's own part of the dx9 shading: its cloth, its gel lights' maps,
/// its light volumes, liquids and contrails, and its flares.
#[derive(Default)]
pub(in crate::app::model_preview) struct H2Game {
    slots: GameSlots<H2Slot>,
}

impl Dx9Game for H2Game {
    /// `cloth_update`, before anything draws the cloth's vertices; then the
    /// sprites' maps.
    unsafe fn prepare(&mut self, cx: &mut FrameCx<'_>) {
        unsafe {
            if cx
                .frame
                .preview
                .cloth
                .first()
                .is_some_and(|cloth| cloth.halo2())
            {
                cx.core.move_cloth(cx.gl, cx.frame, cx.slot);
            }
            let h2 = self.slots.of(cx.slot, cx.core.slot_uploads(cx.slot));
            if cx.core.draws_object_sprites(cx.frame) {
                h2.sync_textures(cx.gl, cx.frame);
            }
        }
    }

    /// `sub_6C18F0`'s shield shaders, each while its value is above nothing:
    /// `object_overshield_amount` is the vitality past 1.
    unsafe fn shields(&mut self, cx: &mut FrameCx<'_>, order: &[usize]) {
        let frame = cx.frame;
        let value = |name: &str| {
            frame
                .function_values
                .as_ref()
                .and_then(|values| values.get(name))
                .copied()
                .unwrap_or(0.0)
        };
        for (layer, name) in [
            (Dx9Layer::ShieldDamaged, "current_shield_damage"),
            (Dx9Layer::Overshield, "object_overshield_amount"),
        ] {
            if value(name) > 0.0 {
                unsafe {
                    cx.core
                        .draw_h2_passes(cx.gl, frame, cx.slot, order, layer, None)
                };
            }
        }
    }

    /// `rasterizer_postprocess_scene`'s bloom over the finished frame.
    unsafe fn bloom(&mut self, cx: &mut FrameCx<'_>, order: Vec<usize>) {
        unsafe {
            cx.core
                .draw_dx9_bloom(cx.gl, cx.frame, cx.slot, Some(order))
        }
    }

    /// `render_lights`' sphere lights, then the gel lights with their maps.
    unsafe fn lights(&mut self, cx: &mut FrameCx<'_>, order: &[usize]) {
        unsafe {
            super::lighting::draw_h2_point_lights(cx.core, cx.gl, cx.frame, cx.slot, order);
            let h2 = self.slots.of(cx.slot, cx.core.slot_uploads(cx.slot));
            let gels = h2.gels(cx.gl, cx.frame).to_vec();
            super::lighting::draw_h2_gel_lights(cx.core, cx.gl, cx.frame, cx.slot, order, &gels);
        }
    }

    /// The light volumes, liquids and contrails, under the widgets' depth
    /// state.
    unsafe fn draw_before_antennas(&mut self, cx: &mut Dx9Sprites<'_>) {
        let Dx9Sprites {
            gl,
            frame,
            flares,
            view,
            bones,
            slot,
            uploads,
            ..
        } = *cx;
        let camera = &frame.camera;
        let h2 = self.slots.of(slot, uploads);
        let values = |object: u8| frame.function_values_of_object(object).cloned();
        h2.contrails
            .advance(&frame.preview.h2.contrails, values, bones, frame.time);
        unsafe {
            let volumes = frame
                .preview
                .h2
                .light_volumes
                .iter()
                .zip(&h2.textures.light_volumes);
            for (index, (at, textures)) in volumes.enumerate() {
                if !frame.shown.is(paths::H2_LIGHT_VOLUMES, index) {
                    continue;
                }
                let values = frame.function_values_of_object(at.object);
                let onset = cx.onsets.of(at.object, at.scale_name(), values, frame.time);
                for (volume, texture) in at.volumes().iter().zip(textures) {
                    let Some(texture) = texture else {
                        continue;
                    };
                    gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
                    gen3_set_blend(
                        gl,
                        if volume.fuzzy() {
                            Gen3Blend::Additive
                        } else {
                            Gen3Blend::PreMultiplied
                        },
                    );
                    let colors = frame.change_colors_of_object(at.object);
                    for quad in at.quads(volume, values, &colors, bones, view, frame.time, onset) {
                        draw_sprite_quad(gl, flares, camera, quad.corners, quad.color);
                    }
                }
            }
            let liquids = frame.preview.h2.liquids.iter().zip(&h2.textures.liquids);
            for (index, (at, textures)) in liquids.enumerate() {
                if !frame.shown.is(paths::H2_LIQUIDS, index) {
                    continue;
                }
                let values = frame.function_values_of_object(at.object);
                for (arc, texture) in at.arcs().iter().zip(textures) {
                    let Some(texture) = texture else {
                        continue;
                    };
                    gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
                    gen3_set_blend(gl, Gen3Blend::PreMultiplied);
                    for quad in at.quads(arc, values, bones, view, frame.time) {
                        draw_sprite_quad(gl, flares, camera, quad.corners, quad.color);
                    }
                }
            }
            let contrails = frame
                .preview
                .h2
                .contrails
                .iter()
                .zip(&h2.textures.contrails);
            for (index, (at, texture)) in contrails.enumerate() {
                if !frame.shown.is(paths::H2_CONTRAILS, index) {
                    continue;
                }
                let Some(texture) = texture else {
                    continue;
                };
                gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
                gen3_set_blend(gl, at.blend());
                for quad in h2.contrails.quads(index, at, view) {
                    draw_sprite_quad(gl, flares, camera, quad.corners, quad.color);
                }
            }
        }
    }

    unsafe fn draw_flares(&mut self, cx: &mut Dx9Sprites<'_>) {
        let Dx9Sprites {
            gl,
            frame,
            flares,
            view,
            bones,
            slot,
            uploads,
            ..
        } = *cx;
        let camera = &frame.camera;
        let h2 = self.slots.of(slot, uploads);
        unsafe {
            for (index, at) in frame.preview.h2.flares.iter().enumerate() {
                if !frame.shown.is(paths::H2_FLARES, index) {
                    continue;
                }
                let occlusion = match at.occlusion_patches(bones, view) {
                    None => [1.0; 2],
                    Some((outer, inner)) => {
                        let Some(outer) = sprite_occlusion(gl, flares, camera, view, &outer) else {
                            continue;
                        };
                        let inner = inner.map_or(Some(1.0), |inner| {
                            sprite_occlusion(gl, flares, camera, view, &inner)
                        });
                        [outer * inner.unwrap_or(0.0), outer]
                    }
                };
                let values = frame.function_values_of_object(at.object);
                let colors = frame.change_colors_of_object(at.object);
                let onset = cx.onsets.of(at.object, at.scale_name(), values, frame.time);
                let sprites =
                    at.sprites(values, &colors, bones, view, frame.time, onset, occlusion);
                let Some(textures) = h2.textures.flares.get(index) else {
                    continue;
                };
                draw_flare_sprites(gl, flares, camera, view, &sprites, textures);
            }
        }
    }

    #[cfg(test)]
    fn each_simulation(
        &self,
        slot: usize,
        uploads: u64,
        visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug),
    ) {
        if let Some(h2) = self.slots.current(slot, uploads) {
            visit("h2_contrails", &h2.contrails);
        }
    }
}
