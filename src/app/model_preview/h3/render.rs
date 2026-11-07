//! Halo 3's and ODST's place in the gen3 pipeline: their self shadow, from
//! a map generated through dx9 shadow passes and applied after the opaque
//! pass.

use eframe::glow::{self, HasContext};

use super::super::common::slots::GameSlots;
use super::super::gen3::pipeline::h3_lighting_constants;
use super::super::gen3::render::{Gen3Batch, Gen3Game};
use super::super::gen3::sprites::{Gen3Sprite, Gen3Sprites};
use super::super::renderer::particle_floats;
use super::slot::H3Slot;
use super::super::gen3::screen_effects::H3ScreenSettings;
use super::super::renderer::FrameCx;
use super::bloom::H3Bloom;

/// Halo 3's and ODST's hooks: the pipeline's defaults, the self shadow, and
/// their bloom and composite.
#[derive(Default)]
pub(in crate::app::model_preview) struct H3Game {
    bloom: Option<H3Bloom>,
    /// The final composite's program.
    pub(super) composite_program: Option<glow::NativeProgram>,
    /// Each of the renderer's slots' state.
    slots: GameSlots<H3Slot>,
}

impl Gen3Game for H3Game {
    /// The leaves step on, and their maps follow the slot's geometry.
    unsafe fn advance_own_sprites(&mut self, cx: &mut Gen3Sprites<'_>) {
        let h3 = self.slots.of(cx.slot, cx.uploads);
        unsafe { h3.sync_textures(cx.gl, cx.frame) };
        h3.leaves.advance(&cx.frame.preview.h3_leaves, cx.frame.time);
    }

    /// The leaves, through the particle program.
    unsafe fn draw_own_sprite(&mut self, cx: &mut Gen3Sprites<'_>, sprite: Gen3Sprite) -> bool {
        let Gen3Sprite::Leaves(index) = sprite else {
            return false;
        };
        let frame = cx.frame;
        let leaves = &frame.preview.h3_leaves[index];
        let Some(particles) = cx.particle_gl else {
            return true;
        };
        let h3 = self.slots.of(cx.slot, cx.uploads);
        let lighting = h3_lighting_constants(frame);
        let drawn = h3.leaves.vertices(index, leaves, &lighting);
        if drawn.is_empty() {
            return true;
        }
        let camera = &frame.camera;
        let vertices = particle_floats(camera, &drawn);
        let textures = h3.leaf_textures.get(index).copied().unwrap_or_default();
        let (no_map, depth, stops) = (cx.no_map, cx.depth_copy, cx.stops);
        unsafe { particles.draw(cx.gl, leaves.shader(), &vertices, textures, no_map, depth, camera, &lighting, stops) };
        true
    }

    fn vision_on_for(&mut self, slot: usize, uploads: u64, time: f32) -> f32 {
        self.slots.of(slot, uploads).vision_on_for(time)
    }

    #[cfg(test)]
    fn each_simulation(&self, slot: usize, uploads: u64, visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {
        if let Some(h3) = self.slots.current(slot, uploads) {
            visit("h3_leaves", &h3.leaves);
            visit("h3_vision_clock", &h3.vision_clock);
        }
    }

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

    unsafe fn bloom(
        &mut self,
        cx: &mut FrameCx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        size: [i32; 2],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            let vertices = cx.core.h3_empty_array(cx.gl);
            if self.bloom.is_none() {
                self.bloom = H3Bloom::new(cx.gl, cx.core.black_flat);
            }
            self.bloom.as_mut()?.build(cx.gl, vertices, framebuffer, accumulation, size)
        }
    }

    unsafe fn composite(
        &mut self,
        cx: &mut FrameCx<'_>,
        _framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        screen: H3ScreenSettings,
        _size: [i32; 2],
    ) {
        unsafe { self.draw_composite(cx, accumulation, bloom, screen) }
    }
}
