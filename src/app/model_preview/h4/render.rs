//! Halo 4's place in the gen3 pipeline: its holograms, full camo, self
//! shadow mask, screen lights, shield, flares, bloom and composite.

use eframe::glow::{self, HasContext};

use super::super::gen3::render::{Gen3Batch, Gen3Game};
use super::super::gen3::screen_effects::H3ScreenSettings;
use super::super::renderer::{FrameCx, ModelGpuFrame, RenderCore};
use super::super::renderer::h3_draw::{h3_layered_distance, h3_set_cull};
use super::super::renderer::h4_holograms::{HologramPicture, draws_as_hologram};
use super::camo::H4CamoDraw;

/// Halo 4's hooks, and what it carries through a frame: the holograms and
/// their picture, the camo and what it draws over, and the shadow mask.
#[derive(Default)]
pub(in crate::app::model_preview) struct H4Game {
    holograms: Vec<Gen3Batch>,
    hologram: Option<HologramPicture>,
    camo: Option<H4CamoDraw>,
    camo_batches: Vec<Gen3Batch>,
    mask: Option<glow::NativeTexture>,
}

impl Gen3Game for H4Game {
    /// Holograms draw apart from the scene (`RenderHolograms`). Full camo
    /// (`render_object_mesh_parts`): a camo object's parts draw only into the
    /// shadow and as camo, so nothing of it lands in the light; below full
    /// transparency its lit base does.
    fn split(&mut self, frame: &ModelGpuFrame, batches: &mut Vec<Gen3Batch>) {
        let (holograms, rest): (Vec<_>, Vec<_>) =
            batches.drain(..).partition(|(_, material)| draws_as_hologram(material));
        *batches = rest;
        self.holograms = holograms;
        self.camo = frame.preview.h4_camo.as_ref().filter(|_| frame.active_camo).map(|camo| camo.draw(1.0, frame.time));
        self.camo_batches = match self.camo {
            Some(draw) if !draw.has_base => batches.drain(..).chain(self.holograms.drain(..)).collect(),
            Some(_) => batches.clone(),
            None => Vec::new(),
        };
    }

    /// `RenderHolograms`' generate pass: after the albedo and its decals,
    /// before the lights.
    unsafe fn after_albedo(&mut self, cx: &mut FrameCx<'_>, albedo: glow::NativeFramebuffer, size: [i32; 2]) {
        self.hologram = if self.holograms.is_empty() {
            None
        } else {
            unsafe { cx.core.h4_generate_holograms(cx.gl, cx.frame, cx.slot, &self.holograms, albedo, size) }
        };
    }

    /// `render_fullscreen_shadow_buffer`: the mask, between the albedo and
    /// the static lighting that reads it.
    unsafe fn self_shadow(
        &mut self,
        cx: &mut FrameCx<'_>,
        batches: &[Gen3Batch],
        _order: &[usize],
        _scene: glow::NativeFramebuffer,
    ) -> bool {
        self.mask = if cx.frame.self_shadow && self.camo.is_none_or(|draw| draw.has_base) {
            unsafe { cx.core.draw_h4_shadow(cx.gl, cx.frame, cx.slot, batches) }
        } else {
            None
        };
        false
    }

    fn lit_values(&mut self, core: &mut RenderCore) {
        if let Some(mask) = self.mask.take() {
            core.h4_shadow_mask = Some(mask);
            let floor = [0.0, super::shadow::DIRECT_FLOOR, 0.0, 0.0];
            core.reach_pass_values = Some([("ps_bsp_lightmap_scale_constants", floor)].into_iter().collect());
        }
    }

    /// `MidnightLightParfait::Render`: after the static lighting, before
    /// what blends.
    unsafe fn screen_lights(&mut self, cx: &mut FrameCx<'_>, _lit: &[super::super::reach::lights::ReachLitLight], accumulation: glow::NativeFramebuffer) {
        unsafe {
            cx.core.draw_h4_lights(cx.gl, cx.frame, accumulation);
            cx.core.bind_slot_vertices(cx.gl, cx.slot);
            h3_set_cull(cx.gl, cx.frame);
            cx.gl.depth_func(glow::LEQUAL);
            cx.gl.depth_mask(false);
            cx.gl.blend_func(glow::ONE, glow::ONE);
        }
    }

    /// `QueueHologramApply`: the holograms' composite is one more
    /// transparent, at their sphere.
    unsafe fn blended(&mut self, cx: &mut FrameCx<'_>, nearer_than: f32, size: [i32; 2]) {
        if let Some(picture) = self.hologram.take_if(|picture| h3_layered_distance(2, picture.distance) > nearer_than) {
            unsafe { cx.core.h4_composite_holograms(cx.gl, cx.frame, cx.slot, &picture, size) }
        }
    }

    /// Halo 4's camo draws after the composite, not here.
    fn camo(&mut self, _core: &mut RenderCore, _frame: &ModelGpuFrame) -> Option<bool> {
        None
    }

    unsafe fn shield(&mut self, cx: &mut FrameCx<'_>) {
        unsafe { cx.core.draw_h4_shield(cx.gl, cx.frame, cx.slot) }
    }

    unsafe fn late_sprites(&mut self, cx: &mut FrameCx<'_>) {
        unsafe { cx.core.draw_h4_flares(cx.gl, cx.frame, cx.slot) }
    }

    unsafe fn bloom(
        &mut self,
        cx: &mut FrameCx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        size: [i32; 2],
    ) -> Option<glow::NativeTexture> {
        unsafe { cx.core.h3_built_bloom(cx.gl, framebuffer, accumulation, size, Some(cx.frame)) }
    }

    unsafe fn composite(
        &mut self,
        cx: &mut FrameCx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        screen: H3ScreenSettings,
        size: [i32; 2],
    ) {
        unsafe {
            let depth = cx.core.object_depth(cx.gl, framebuffer);
            cx.core.h4_composite(cx.gl, cx.frame, accumulation, bloom, depth, screen);
            if let Some(draw) = self.camo {
                cx.core.draw_h4_camo(cx.gl, cx.frame, cx.slot, &self.camo_batches, draw, size);
            }
        }
    }
}
