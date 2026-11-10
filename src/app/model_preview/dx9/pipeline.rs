//! Halo CE's and Halo 2's shading after the slot's own draw: active camo,
//! or the lit passes and every light by layer; the transparents; the
//! shields; and what the finished frame blooms over. Each game hooks in
//! where its own differs.

use super::super::{FrameCx, ModelGpuFrame, RenderCore};
use super::binds::{Dx9Binds, NoOwnBinds};
use super::draw::draw_order;
use super::passes::{DEFAULT_LIGHTS, Dx9Layer, SHADOWED_LIGHT};
use super::shadow::ShadowStyle;
use super::sprites::Dx9Sprites;
use eframe::glow::{self, HasContext as _};

/// Where Halo CE's and Halo 2's shading differ.
pub(in crate::app::model_preview) trait Dx9Game {
    /// First, before the frame finds its slot (see `GameRender::begin`).
    fn begin(&mut self, _frame: &ModelGpuFrame) {}

    /// Before the scene target: what moves the model's vertices or makes
    /// lights, once a frame.
    unsafe fn prepare(&mut self, _cx: &mut FrameCx<'_>) {}

    /// What the game binds on its passes beside the shared uniforms.
    fn binds(&self) -> &dyn Dx9Binds {
        &NoOwnBinds
    }

    /// The transparent layer, in `order`.
    unsafe fn transparents(&mut self, cx: &mut FrameCx<'_>, order: &[usize]) {
        unsafe {
            cx.core.draw_dx9_layer(
                cx.gl,
                cx.frame,
                cx.slot,
                order,
                Dx9Layer::Transparent,
                None,
                self.binds(),
            )
        }
    }

    /// After the transparents: the game's shield layers, in `order`.
    unsafe fn shields(&mut self, _cx: &mut FrameCx<'_>, _order: &[usize]) {}

    /// Last, over the finished frame: the game's bloom of what `order`
    /// draws, when the frame blooms.
    unsafe fn bloom(&mut self, _cx: &mut FrameCx<'_>, _order: Vec<usize>) {}

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
    fn each_simulation(
        &self,
        _slot: usize,
        _uploads: u64,
        _visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug),
    ) {
    }
}

impl RenderCore {
    /// The dx9 passes over the slot's own draw, with `game`'s hooks: the
    /// game's shaders, so none without Game Shaders. What the finished frame
    /// blooms over, if it does.
    pub(in crate::app::model_preview) unsafe fn draw_dx9_passes(
        &mut self,
        game: &mut dyn Dx9Game,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
    ) -> Option<Vec<usize>> {
        if !frame.game_shaders {
            return None;
        }
        unsafe {
            if self.dx9_camo_on(frame) {
                let order = frame.visible_batches.clone();
                self.copy_dx9_frame(gl, frame, &order);
                self.draw_dx9_layer(
                    gl,
                    frame,
                    slot,
                    &order,
                    Dx9Layer::ActiveCamo,
                    None,
                    game.binds(),
                );
            } else {
                // The lit layers, the self shadow, then every light.
                let order = frame.visible_batches.clone();
                self.copy_dx9_frame(gl, frame, &order);
                self.fill_dx9_texaccum(gl, frame, slot, &order, game.binds());
                let lightmap = frame.dx9_lightmap_layer();
                self.draw_dx9_layer(gl, frame, slot, &order, lightmap, None, game.binds());
                self.draw_dx9_layer(gl, frame, slot, &order, Dx9Layer::Prt, None, game.binds());
                if frame.self_shadow
                    && !frame.default_lights
                    && self.draw_dx9_shadow(
                        gl,
                        frame,
                        slot,
                        &order,
                        ShadowStyle::dx9(frame),
                        game.binds(),
                    )
                {
                    gl.enable(glow::STENCIL_TEST);
                    gl.stencil_func(glow::EQUAL, 1, 0xFF);
                    gl.stencil_op(glow::KEEP, glow::KEEP, glow::KEEP);
                    let shadowed = Some(&SHADOWED_LIGHT);
                    self.draw_dx9_layer(gl, frame, slot, &order, lightmap, shadowed, game.binds());
                    self.draw_dx9_layer(
                        gl,
                        frame,
                        slot,
                        &order,
                        Dx9Layer::Prt,
                        shadowed,
                        game.binds(),
                    );
                    gl.disable(glow::STENCIL_TEST);
                }
                for layer in [
                    Dx9Layer::Environment,
                    Dx9Layer::SelfIllumination,
                    Dx9Layer::Overlay,
                ] {
                    self.draw_dx9_layer(gl, frame, slot, &order, layer, None, game.binds());
                }
                if frame.default_lights {
                    let shadowed = frame.self_shadow
                        && self.draw_dx9_shadow(
                            gl,
                            frame,
                            slot,
                            &order,
                            ShadowStyle::dx9(frame),
                            game.binds(),
                        );
                    for (index, light) in DEFAULT_LIGHTS.iter().enumerate() {
                        if shadowed && index == 0 {
                            gl.enable(glow::STENCIL_TEST);
                            gl.stencil_func(glow::EQUAL, 0, 0xFF);
                            gl.stencil_op(glow::KEEP, glow::KEEP, glow::KEEP);
                        }
                        for layer in [Dx9Layer::LightDiffuse, Dx9Layer::LightSpecular] {
                            self.draw_dx9_layer(
                                gl,
                                frame,
                                slot,
                                &order,
                                layer,
                                Some(light),
                                game.binds(),
                            );
                        }
                        gl.disable(glow::STENCIL_TEST);
                    }
                }
                // `render_lights`: the game's own lights, each drawn over the layers it lights.
                game.lights(
                    &mut FrameCx {
                        gl,
                        frame,
                        core: self,
                        slot,
                    },
                    &order,
                );
                self.draw_dx9_layer(
                    gl,
                    frame,
                    slot,
                    &order,
                    Dx9Layer::Hologram,
                    None,
                    game.binds(),
                );
            }
            let order = draw_order(frame, &self.slots[slot].sort_points);
            game.transparents(
                &mut FrameCx {
                    gl,
                    frame,
                    core: self,
                    slot,
                },
                &order,
            );
            game.shields(
                &mut FrameCx {
                    gl,
                    frame,
                    core: self,
                    slot,
                },
                &order,
            );
            // `rasterizer_postprocess_scene` blooms the finished frame,
            // after every transparent and effect.
            if frame.bloom {
                return Some(order.clone());
            }
            None
        }
    }
}
