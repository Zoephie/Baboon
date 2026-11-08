//! Halo CE's and Halo 2's shading after the slot's own draw: active camo,
//! or the lit passes and every light by layer; the transparents; the
//! shields; and what the finished frame blooms over. Each game hooks in
//! where its own differs.

use eframe::glow::{self, HasContext as _};

use super::super::renderer::*;
use super::super::*;
use super::passes::{H2_DEFAULT_LIGHTS, H2_SHADOWED_LIGHT, H2DefaultLight, H2Layer};
use super::sprites::Dx9Sprites;

/// Where Halo CE's and Halo 2's shading differ.
pub(in crate::app::model_preview) trait Dx9Game {
    /// Before the scene target: what moves the model's vertices or makes
    /// lights, once a frame.
    unsafe fn prepare(&mut self, _cx: &mut FrameCx<'_>) {}

    /// The transparent layer, in `order`.
    unsafe fn transparents(&mut self, cx: &mut FrameCx<'_>, order: &[usize]) {
        unsafe { cx.core.draw_h2_passes(cx.gl, cx.frame, cx.slot, order, H2Layer::Transparent, None) }
    }

    /// The gel lights' maps, by `h2.gel_lights`, while the lights draw.
    unsafe fn gel_maps(&mut self, _cx: &mut FrameCx<'_>) -> Vec<Option<glow::NativeTexture>> {
        Vec::new()
    }

    /// In the sprite pass, under the widgets' depth state, before the
    /// antennas: the game's own sprites.
    unsafe fn draw_before_antennas(&mut self, _cx: &mut Dx9Sprites<'_>) {}

    /// In the sprite pass, after the antennas: the game's own sprites, with
    /// the sprite program bound again after.
    unsafe fn draw_own_sprites(&mut self, _cx: &mut Dx9Sprites<'_>) {}

    /// In the sprite pass, under the flares' added blend: the game's lens
    /// flares.
    unsafe fn draw_flares(&mut self, _cx: &mut Dx9Sprites<'_>) {}

    /// Each simulation the game keeps for `slot` while it holds its
    /// `uploads`th model, by name.
    #[cfg(test)]
    fn each_simulation(&self, _slot: usize, _uploads: u64, _visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {}
}

impl RenderCore {
    /// The dx9 passes over the slot's own draw, with `game`'s hooks. What the
    /// finished frame blooms over, if it does.
    pub(in crate::app::model_preview) unsafe fn draw_dx9_passes(
        &mut self,
        game: &mut dyn Dx9Game,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
    ) -> Option<Vec<usize>> {
        unsafe {
            if frame.game_shaders && self.h2_camo_on(frame) {
                let order = frame.visible_batches.clone();
                self.copy_h2_frame(gl, frame, &order);
                self.draw_h2_passes(gl, frame, slot, &order, H2Layer::ActiveCamo, None);
            } else if frame.game_shaders {
                // The lit layers, the self shadow, then every light.
                let order = frame.visible_batches.clone();
                self.copy_h2_frame(gl, frame, &order);
                self.fill_h2_texaccum(gl, frame, slot, &order);
                let lightmap = frame.h2_lightmap_layer();
                self.draw_h2_passes(gl, frame, slot, &order, lightmap, None);
                self.draw_h2_passes(gl, frame, slot, &order, H2Layer::Prt, None);
                if frame.self_shadow
                    && !frame.default_lights
                    && self.draw_h2_shadow(gl, frame, slot, &order)
                {
                    gl.enable(glow::STENCIL_TEST);
                    gl.stencil_func(glow::EQUAL, 1, 0xFF);
                    gl.stencil_op(glow::KEEP, glow::KEEP, glow::KEEP);
                    let shadowed = Some(&H2_SHADOWED_LIGHT);
                    self.draw_h2_passes(gl, frame, slot, &order, lightmap, shadowed);
                    self.draw_h2_passes(gl, frame, slot, &order, H2Layer::Prt, shadowed);
                    gl.disable(glow::STENCIL_TEST);
                }
                for layer in [
                    H2Layer::Environment,
                    H2Layer::SelfIllumination,
                    H2Layer::Overlay,
                ] {
                    self.draw_h2_passes(gl, frame, slot, &order, layer, None);
                }
                if frame.default_lights {
                    let shadowed = frame.self_shadow && self.draw_h2_shadow(gl, frame, slot, &order);
                    for (index, light) in H2_DEFAULT_LIGHTS.iter().enumerate() {
                        if shadowed && index == 0 {
                            gl.enable(glow::STENCIL_TEST);
                            gl.stencil_func(glow::EQUAL, 0, 0xFF);
                            gl.stencil_op(glow::KEEP, glow::KEEP, glow::KEEP);
                        }
                        for layer in [H2Layer::LightDiffuse, H2Layer::LightSpecular] {
                            self.draw_h2_passes(gl, frame, slot, &order, layer, Some(light));
                        }
                        gl.disable(glow::STENCIL_TEST);
                    }
                }
                // `render_lights`: real lights draw whichever rig stands behind them.
                let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
                for (index, point) in frame.preview.h2.point_lights.iter().enumerate() {
                    if !frame.shown.is(super::super::common::features::paths::H2_POINT_LIGHTS, index) {
                        continue;
                    }
                    let values = frame.function_values_of_object(point.object);
                    let onsets = &mut self.slots[slot].simulations.effect_onsets;
                    let onset = onsets.of(point.object, point.scale_name(), values, frame.time);
                    let Some(light) = point.lit(values, bones, frame.time, onset) else {
                        continue;
                    };
                    let (light, draws) = h2::lights::overbrighten(&light);
                    let specular = H2DefaultLight { color: light.specular, ..light };
                    for _ in 0..draws {
                        for (layer, light) in
                            [(point.diffuse_layer(), &light), (H2Layer::LightSpecular, &specular)]
                        {
                            self.draw_h2_passes(gl, frame, slot, &order, layer, Some(light));
                        }
                    }
                }
                let gels = game.gel_maps(&mut FrameCx { gl, frame, core: self, slot });
                let mut masked = false;
                for (index, gel) in frame.preview.h2.gel_lights.iter().enumerate() {
                    if !frame.shown.is(super::super::common::features::paths::H2_GEL_LIGHTS, index) {
                        continue;
                    }
                    let values = frame.function_values_of_object(gel.object);
                    let onsets = &mut self.slots[slot].simulations.effect_onsets;
                    let onset = onsets.of(gel.object, gel.scale_name(), values, frame.time);
                    let Some((light, rows)) = gel.lit(values, bones, frame.time, onset) else {
                        continue;
                    };
                    let specular = h2::lights::has_specular(&light);
                    let (light, draws) = h2::lights::overbrighten(&light);
                    self.h2_gel = Some((gels.get(index).copied().flatten(), rows.rows));
                    if specular {
                        self.draw_h2_passes(gl, frame, slot, &order, H2Layer::GelMask, Some(&light));
                        masked = true;
                    }
                    let layer = if specular { H2Layer::LightFrustum } else { H2Layer::LightGel };
                    for _ in 0..draws {
                        self.draw_h2_passes(gl, frame, slot, &order, layer, Some(&light));
                    }
                    self.h2_gel = None;
                }
                if masked {
                    gl.color_mask(false, false, false, true);
                    gl.clear_color(0.0, 0.0, 0.0, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                    gl.color_mask(true, true, true, true);
                }
                self.draw_h2_passes(gl, frame, slot, &order, H2Layer::Hologram, None);
            }
            let order = h2_draw_order(frame, &self.slots[slot].h2_sort_points);
            game.transparents(&mut FrameCx { gl, frame, core: self, slot }, &order);
            // `sub_6C18F0`: `object_overshield_amount` is the vitality past 1.
            let value = |name: &str| {
                frame.function_values.as_ref().and_then(|values| values.get(name)).copied().unwrap_or(0.0)
            };
            for (layer, name) in [
                (H2Layer::ShieldDamaged, "current_shield_damage"),
                (H2Layer::Overshield, "object_overshield_amount"),
            ] {
                if value(name) > 0.0 {
                    self.draw_h2_passes(gl, frame, slot, &order, layer, None);
                }
            }
            // `rasterizer_postprocess_scene` blooms the finished frame,
            // after every transparent and effect.
            if frame.game_shaders && frame.bloom {
                return Some(order.clone());
            }
            None
        }
    }
}
