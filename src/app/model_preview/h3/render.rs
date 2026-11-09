//! Halo 3's and ODST's place in the gen3 pipeline: their self shadow, from
//! a map generated through dx9 shadow passes and applied after the opaque
//! pass; their object lights; VISR and the motion suck over the finished
//! picture; their leaves; and their bloom and composite.

use eframe::glow::{self, HasContext};

use super::super::common::slots::GameSlots;
use super::super::gen3::pipeline::h3_lighting_constants;
use super::super::gen3::render::{Gen3Batch, Gen3Game};
use super::super::gen3::sprites::{Gen3Sprite, Gen3Sprites};
use super::super::particles::particle_floats;
use super::slot::H3Slot;
use super::super::gen3::effects::screen::H3ScreenSettings;
use super::super::gen3::render::Gen3Cx;
use super::super::gen3::pipeline::PictureCopies;
use super::super::frame::ModelGpuFrame;
use super::bloom::H3Bloom;
use super::super::gen3::shadow::h3_shadow_style;
use super::super::gen3::pipeline::h3_set_cull;
use super::super::dx9::passes::Dx9Layer;
use super::light_pass::draw_dynamic_lights;

/// Halo 3's and ODST's hooks: the pipeline's defaults, the self shadow, and
/// their bloom and composite.
#[derive(Default)]
pub(in crate::app::model_preview) struct H3Game {
    bloom: Option<H3Bloom>,
    /// The final composite's program.
    pub(super) composite_program: Option<glow::NativeProgram>,
    /// Each of the renderer's slots' state.
    pub(super) slots: GameSlots<H3Slot>,
    /// Whether this frame's self shadow drew its map, for the apply.
    shadowed: bool,
    /// VISR's and the motion suck's programs, and VISR's mask.
    pub(super) vision_program: Option<glow::NativeProgram>,
    pub(super) suck_program: Option<glow::NativeProgram>,
    pub(super) vision_mask: Option<glow::NativeTexture>,
}

impl Gen3Game for H3Game {
    /// The leaves step on, and their maps follow the slot's geometry.
    unsafe fn advance_own_sprites(&mut self, cx: &mut Gen3Sprites<'_>) {
        let h3 = self.slots.of(cx.slot, cx.uploads);
        unsafe { h3.sync_textures(cx.gl, cx.frame) };
        h3.leaves.advance(&cx.frame.preview.h3.leaves, cx.frame.time);
    }

    /// The leaves, through the particle program.
    unsafe fn draw_own_sprite(&mut self, cx: &mut Gen3Sprites<'_>, sprite: Gen3Sprite) -> bool {
        let Gen3Sprite::Leaves(index) = sprite else {
            return false;
        };
        let frame = cx.frame;
        let leaves = &frame.preview.h3.leaves[index];
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

    fn picture_effects(&self, frame: &ModelGpuFrame, screen: &H3ScreenSettings) -> bool {
        Self::draws_picture_effects(frame, screen)
    }

    unsafe fn before_screen_shaders(&mut self, cx: &mut Gen3Cx<'_>, screen: &H3ScreenSettings, copies: &PictureCopies) {
        unsafe { self.draw_picture_suck(cx, screen, copies) }
    }

    unsafe fn after_blur(&mut self, cx: &mut Gen3Cx<'_>, screen: &H3ScreenSettings, copies: &PictureCopies) {
        unsafe { self.draw_picture_vision(cx, screen, copies) }
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
        cx: &mut Gen3Cx<'_>,
        _batches: &[Gen3Batch],
        order: &[usize],
        scene: glow::NativeFramebuffer,
    ) {
        self.shadowed = cx.frame.self_shadow && unsafe {
            // Generated from the light, for the apply pass after the opaque one.
            cx.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene));
            let drawn = cx.core.draw_h2_shadow(cx.gl, cx.frame, cx.slot, order, h3_shadow_style(cx.frame));
            cx.core.bind_slot_vertices(cx.gl, cx.slot);
            drawn
        };
    }

    /// The self shadow's apply, through the dx9 shadow passes, if its map
    /// was drawn.
    unsafe fn after_opaque(&mut self, cx: &mut Gen3Cx<'_>, order: &[usize]) {
        if !self.shadowed {
            return;
        }
        let (gl, frame, slot) = (cx.gl, cx.frame, cx.slot);
        unsafe {
            cx.core.draw_h2_passes(gl, frame, slot, order, Dx9Layer::ShadowApply, None);
            gl.bind_vertex_array(Some(cx.core.slots[slot].vertex_array));
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            h3_set_cull(gl, frame);
        }
    }

    unsafe fn dynamic_lights(&mut self, cx: &mut Gen3Cx<'_>, batches: &[Gen3Batch]) {
        unsafe { draw_dynamic_lights(cx, batches) }
    }

    unsafe fn bloom(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        size: [i32; 2],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            let vertices = cx.state.empty_array(cx.gl);
            if self.bloom.is_none() {
                self.bloom = H3Bloom::new(cx.gl, cx.core.black_flat);
            }
            self.bloom.as_mut()?.build(cx.gl, vertices, framebuffer, accumulation, size)
        }
    }

    unsafe fn composite(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        _framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        screen: H3ScreenSettings,
        _size: [i32; 2],
    ) {
        unsafe { self.draw_composite(cx, accumulation, bloom, screen) }
    }
}
