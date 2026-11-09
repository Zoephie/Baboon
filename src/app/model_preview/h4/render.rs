//! Halo 4's place in the gen3 pipeline: its holograms, full camo, self
//! shadow mask, screen lights, shield, flares, bloom and composite.

use super::super::gen3::pipeline::FrameValues;
use super::lighting::h4_frame_values;
use eframe::glow::{self, HasContext};

use super::super::common::slots::GameSlots;
use super::effects::gl::H4EffectCx;
use super::super::gen3::render::{Gen3Batch, Gen3Game};
use super::super::gen3::sprites::{Gen3Sprite, Gen3Sprites};
use super::super::gen3::effects::screen::H3ScreenSettings;
use super::super::frame::ModelGpuFrame;
use super::super::gen3::render::Gen3Cx;
use super::super::gen3::pipeline::h3_set_cull;
use crate::app::model_preview::common::sprites::h3_layered_distance;
use super::holograms::{HologramPicture, draws_as_hologram};
use super::camo::H4CamoDraw;

/// Halo 4's hooks, and what it carries through a frame: the holograms and
/// their picture, the camo and what it draws over, and the shadow mask.
#[derive(Default)]
pub(in crate::app::model_preview) struct H4Game {
    /// The shadow's depth, blurs and mask, kept between frames, and its
    /// horizontal, vertical and apply programs.
    pub(super) shadow: Option<super::shadow_pass::H4ShadowTargets>,
    pub(super) shadow_programs: Option<[Option<glow::NativeProgram>; 3]>,
    /// The hologram surfaces and the screen materials' bitmaps.
    pub(super) surfaces: Option<super::holograms::H4Holograms>,
    /// The bloom chain, the view's and the holograms'.
    pub(super) bloom: Option<super::bloom::H4Bloom>,
    /// The colour grading lookup and the image it was uploaded from. Holding
    /// the image keeps its address from going to a later preview's, so a new
    /// load always uploads its own.
    pub(super) grading: Option<(std::sync::Arc<super::super::common::image::H2Image>, glow::NativeTexture)>,
    /// The final composite's program.
    pub(super) composite_program: Option<glow::NativeProgram>,
    /// The corners the flare reflections draw from.
    pub(super) flare_geometry: Option<(glow::NativeVertexArray, glow::NativeBuffer)>,
    holograms: Vec<Gen3Batch>,
    hologram: Option<HologramPicture>,
    camo: Option<H4CamoDraw>,
    camo_batches: Vec<Gen3Batch>,
    mask: Option<glow::NativeTexture>,
    /// Each of the renderer's slots' state.
    slots: GameSlots<super::slot::H4Slot>,
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
        self.camo = frame.preview.h4.camo.as_ref().filter(|_| frame.active_camo).map(|camo| camo.draw(1.0, frame.time));
        self.camo_batches = match self.camo {
            Some(draw) if !draw.has_base => batches.drain(..).chain(self.holograms.drain(..)).collect(),
            Some(_) => batches.clone(),
            None => Vec::new(),
        };
    }

    /// `RenderHolograms`' generate pass: after the albedo and its decals,
    /// before the lights.
    unsafe fn after_albedo(&mut self, cx: &mut Gen3Cx<'_>, albedo: glow::NativeFramebuffer, size: [i32; 2]) {
        self.hologram = if self.holograms.is_empty() {
            None
        } else {
            unsafe {
                let holograms = std::mem::take(&mut self.holograms);
                let picture = self.generate_holograms(cx.core, cx.state, cx.gl, cx.frame, cx.slot, &holograms, albedo, size);
                self.holograms = holograms;
                picture
            }
        };
    }

    fn frame_values(&self) -> FrameValues {
        h4_frame_values
    }

    /// `render_fullscreen_shadow_buffer`: the mask, between the albedo and
    /// the static lighting that reads it.
    unsafe fn self_shadow(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        batches: &[Gen3Batch],
        _order: &[usize],
        _scene: glow::NativeFramebuffer,
    ) {
        self.mask = if cx.frame.self_shadow && self.camo.is_none_or(|draw| draw.has_base) {
            unsafe { self.draw_shadow(cx, batches) }
        } else {
            None
        };
    }

    fn lit_values(&mut self, cx: &mut Gen3Cx<'_>) {
        let state = &mut *cx.state;
        if let Some(mask) = self.mask.take() {
            state.shadow_mask = Some(mask);
            let floor = [0.0, super::shadow::DIRECT_FLOOR, 0.0, 0.0];
            state.pass_values = Some([("ps_bsp_lightmap_scale_constants", floor)].into_iter().collect());
        }
    }

    /// `MidnightLightParfait::Render`: after the static lighting, before
    /// what blends.
    unsafe fn screen_lights(&mut self, cx: &mut Gen3Cx<'_>, accumulation: glow::NativeFramebuffer) {
        unsafe {
            self.draw_lights(cx, accumulation);
            cx.core.bind_slot_vertices(cx.gl, cx.slot);
            h3_set_cull(cx.gl, cx.frame);
            cx.gl.depth_func(glow::LEQUAL);
            cx.gl.depth_mask(false);
            cx.gl.blend_func(glow::ONE, glow::ONE);
        }
    }

    /// `QueueHologramApply`: the holograms' composite is one more
    /// transparent, at their sphere.
    unsafe fn blended(&mut self, cx: &mut Gen3Cx<'_>, nearer_than: f32, size: [i32; 2]) {
        if let Some(picture) = self.hologram.take_if(|picture| h3_layered_distance(2, picture.distance) > nearer_than) {
            unsafe { self.composite_holograms(cx.core, cx.state, cx.gl, cx.frame, cx.slot, &picture, size) }
        }
    }

    /// Halo 4's camo draws after the composite, not here.
    fn camo(&mut self, _cx: &mut Gen3Cx<'_>) -> Option<bool> {
        None
    }

    unsafe fn shield(&mut self, cx: &mut Gen3Cx<'_>) {
        unsafe { self.draw_shield(cx.core, cx.state, cx.gl, cx.frame, cx.slot) }
    }

    unsafe fn late_sprites(&mut self, cx: &mut Gen3Cx<'_>) {
        unsafe { self.draw_flares(cx.core, cx.state, cx.gl, cx.frame, cx.slot) }
    }

    unsafe fn bloom(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        size: [i32; 2],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            let (gl, frame, core) = (cx.gl, cx.frame, &mut *cx.core);
            let state = &mut *cx.state;
            let vertices = state.empty_array(gl);
            let grading = self.grading_texture(gl, frame);
            if self.bloom.is_none() {
                self.bloom = super::bloom::H4Bloom::new(gl, core.black_flat, core.white_volume);
            }
            let settings = &super::bloom::VIEW_BLOOM;
            self.bloom.as_mut()?.build(gl, vertices, framebuffer, accumulation, size, grading.as_ref(), settings)
        }
    }

    unsafe fn composite(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        screen: H3ScreenSettings,
        size: [i32; 2],
    ) {
        unsafe {
            let depth = cx.state.object_depth(cx.gl, framebuffer);
            self.draw_composite(cx, accumulation, bloom, depth, screen);
            if let Some(draw) = self.camo {
                let batches = std::mem::take(&mut self.camo_batches);
                self.draw_camo(cx, &batches, draw, size);
                self.camo_batches = batches;
            }
        }
    }

    /// The trailing tracers step on, and the effects' GL state follows the
    /// slot's geometry.
    unsafe fn advance_own_sprites(&mut self, cx: &mut Gen3Sprites<'_>) {
        let frame = cx.frame;
        let h4 = self.slots.of(cx.slot, cx.uploads);
        unsafe { h4.sync_effects(cx.gl, frame) };
        if frame.preview.h4.tracers.iter().any(|tracer| tracer.trails()) {
            let values = |object: u8| frame.function_values_of_object(object).cloned();
            let tracers = &frame.preview.h4.tracers;
            h4.trails.advance(tracers, values, cx.onsets, cx.bones, cx.view.eye, frame.time);
        }
    }

    /// The tracers, and the light volumes and particle systems that draw
    /// through their material, from packed state.
    unsafe fn draw_own_sprite(&mut self, cx: &mut Gen3Sprites<'_>, sprite: Gen3Sprite) -> bool {
        let (gl, frame, bones, view) = (cx.gl, cx.frame, cx.bones, cx.view);
        let h4 = self.slots.of(cx.slot, cx.uploads);
        let (depth, size, placeholder) = (cx.depth_copy, cx.size, cx.placeholder);
        let drew = unsafe {
            match sprite {
                Gen3Sprite::Tracer(index) => {
                    let at = &frame.preview.h4.tracers[index];
                    let values = frame.function_values_of_object(at.object);
                    let onset = cx.onsets.of(at.object, at.scale_name(), values, frame.time);
                    let draw = if at.trails() {
                        h4.trails.draw(index, at, values, bones, view.eye, frame.time, onset)
                    } else {
                        at.draw(values, bones, view.eye, frame.time, onset)
                    };
                    if let Some(draw) = draw {
                        let material = &at.definition.material;
                        let effects = &mut h4.effects;
                        let mut fx = H4EffectCx { gl, state: effects, frame, depth, size, placeholder, object: at.object };
                        super::effects::tracer_pass::draw_h4_tracer(&mut fx, material, &draw);
                        Some(true)
                    } else {
                        Some(false)
                    }
                }
                Gen3Sprite::Leaves(_) => return false,
                Gen3Sprite::Volume(index) => {
                    let at = &frame.preview.gen3.light_volumes[index];
                    let Some(material) = at.h4_material() else {
                        return false;
                    };
                    let values = frame.function_values_of_object(at.object);
                    let onset = cx.onsets.of(at.object, at.scale_name(), values, frame.time);
                    match at.h4_draw(values, bones, view, frame.time, onset) {
                        Some(draw) => {
                            let effects = &mut h4.effects;
                            let mut fx = H4EffectCx { gl, state: effects, frame, depth, size, placeholder, object: at.object };
                            super::effects::light_volume_pass::draw_h4_light_volume(&mut fx, material, &draw);
                            Some(true)
                        }
                        None => Some(false),
                    }
                }
                Gen3Sprite::System(index) => {
                    let system = &frame.preview.particles[index];
                    let Some(render) = system.look.h4.as_ref() else {
                        return false;
                    };
                    let values = frame.function_values_of_object(system.object);
                    let onset = cx.onsets.of(system.object, system.scale_name(), values, frame.time);
                    let draws = cx.particles.h4_draws(index, system, values, bones, view, frame.time, onset);
                    if draws.is_empty() {
                        Some(false)
                    } else {
                        let effects = &mut h4.effects;
                        let mut fx = H4EffectCx { gl, state: effects, frame, depth, size, placeholder, object: system.object };
                        super::effects::particle_pass::draw_h4_particles(&mut fx, render, &draws);
                        Some(true)
                    }
                }
            }
        };
        // What drew through its own program hands the sprite program back.
        if drew == Some(true) {
            let flares = cx.flares;
            unsafe {
                gl.use_program(Some(flares.program));
                gl.bind_vertex_array(Some(flares.vertex_array));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(flares.buffer));
            }
        }
        drew.is_some()
    }

    #[cfg(test)]
    fn each_simulation(&self, slot: usize, uploads: u64, visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {
        if let Some(h4) = self.slots.current(slot, uploads) {
            visit("h4_trails", &h4.trails);
        }
    }
}
