//! Halo CE's and Halo 2's shading after the slot's own draw: active camo,
//! or the lit passes and every light by layer; the transparents; the
//! shields; and what the finished frame blooms over. Each game hooks in
//! where its own differs.

use eframe::glow::{self, HasContext as _};
use super::shadow::ShadowStyle;
use super::super::*;
use super::passes::{H2_DEFAULT_LIGHTS, H2_SHADOWED_LIGHT, H2Layer};
use super::sprites::Dx9Sprites;
use super::draw::h2_draw_order;

/// Where Halo CE's and Halo 2's shading differ.
pub(in crate::app::model_preview) trait Dx9Game {
    /// Before the scene target: what moves the model's vertices or makes
    /// lights, once a frame.
    unsafe fn prepare(&mut self, _cx: &mut FrameCx<'_>) {}

    /// The transparent layer, in `order`.
    unsafe fn transparents(&mut self, cx: &mut FrameCx<'_>, order: &[usize]) {
        unsafe { cx.core.draw_h2_passes(cx.gl, cx.frame, cx.slot, order, H2Layer::Transparent, None) }
    }

    /// After the default lights, before the hologram layer: the game's own
    /// lights, in `order`.
    unsafe fn lights(&mut self, _cx: &mut FrameCx<'_>, _order: &[usize]) {}

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
                    && self.draw_h2_shadow(gl, frame, slot, &order, ShadowStyle::h2(frame))
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
                    let shadowed = frame.self_shadow && self.draw_h2_shadow(gl, frame, slot, &order, ShadowStyle::h2(frame));
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
                // `render_lights`: the game's own lights, each drawn over the layers it lights.
                game.lights(&mut FrameCx { gl, frame, core: self, slot }, &order);
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
