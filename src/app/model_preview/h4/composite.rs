//! Halo 4's end of the frame: its colour grading, final composite, full camo
//! drawn after it, and shield.

use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};

use super::super::dx9::passes::H2Image;
use super::super::gen3::shaders::{H3Material, H3Sampler, H3Texels};
use super::super::RenderModelPreviewBatch;
use super::super::renderer::h3_draw::{H3_SHIELD_DAMAGE, HALF_LENS, h3_set_cull, screen_program, upload_h2_image_linear};
use eframe::glow::{self, HasContext};
use std::sync::Arc;
use super::render::H4Game;
use super::super::renderer::*;

impl H4Game {
    /// Halo 4's `c_render_shield::render_object_internal`: every part that
    /// takes the shield through explicit 61, added (`ONE, ONE`) over what
    /// lies behind, no depth written, no face culled. The pass applies its
    /// own exposure.
    pub(super) unsafe fn draw_shield(&mut self, core: &mut RenderCore, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        let Some(shield) = frame.preview.h4_shield.clone() else {
            return;
        };
        let damage = frame.function_values.as_ref().and_then(|values| values.get(H3_SHIELD_DAMAGE)).copied().unwrap_or(0.0);
        if !shield.settings.showing(damage) {
            return;
        }
        let parts: Vec<_> = frame
            .shield_batches
            .iter()
            .filter_map(|&index| frame.preview.batches.get(index))
            .filter_map(|batch| {
                let index = batch.material_index as usize;
                let material = core.h3_material(frame, index)?;
                let textures = frame.textures.as_ref()?.get(index)?;
                super::super::reach::shield::draws_shield(textures.shield_only, textures.never_shield, material.decal)
                    .then(|| (batch.clone(), Arc::clone(material)))
            })
            .collect();
        if parts.is_empty() {
            return;
        }
        let (low, high) = (frame.preview.bounds_min, frame.preview.bounds_max);
        let center = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
        let mut values = shield.values(damage, frame.time, frame.function_values.as_ref(), center);
        let camera = &frame.camera;
        let eye = if camera.perspective > 0.0 { 1.0 / camera.perspective } else { 1.0e5 };
        values.insert(
            super::shield::DEPTH_DECODE,
            [camera.depth_scale * camera.depth_map[0], camera.depth_map[1], camera.perspective, eye],
        );
        values.insert(super::shield::WORLD_SCALE, [camera.scale; 4]);
        unsafe {
            if let Some(targets) = core.gen3.targets.as_ref() {
                let [width, height] = targets.size;
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(targets.depth_copy_framebuffer));
                let mask = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
                gl.blit_framebuffer(0, 0, width, height, 0, 0, width, height, mask, glow::NEAREST);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                core.gen3.depth_copy = Some(targets.depth_copy);
            }
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.color_mask(true, true, true, true);
            core.gen3.pass_values = Some(values);
            for (batch, material) in &parts {
                core.h3_draw_pass(gl, frame, slot, batch, material, &shield.pass, None);
            }
            core.gen3.pass_values = None;
            h3_set_cull(gl, frame);
        }
    }

    /// Halo 4's colour grading lookup, uploaded once per preview, and how a
    /// pass reads its texel centres (`SetColorGradingParameters`).
    pub(super) unsafe fn grading_texture(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) -> Option<super::bloom::H4BloomGrading> {
        unsafe {
            let grading = frame.preview.h4_color_grading.as_ref();
            let key = grading.map(|(image, ..)| Arc::as_ptr(image) as usize);
            if self.grading.map(|(known, _)| Some(known)) != Some(key) {
                if let Some((_, texture)) = self.grading.take() {
                    gl.delete_texture(texture);
                }
                // Sampler 8: bilinear, clamped.
                let sampler = H3Sampler { address: [BitmapAddressMode::Clamp; 2], filter: BitmapFilterMode::Bilinear };
                let uploaded = grading.and_then(|(image, _, _)| upload_h2_image_linear(gl, image, H3Texels::Linear, &[], Some(sampler)));
                self.grading = key.zip(uploaded);
            }
            let size = match grading.map(|(image, ..)| &**image) {
                Some(H2Image::Volume { width, height, depth, .. }) => [*width, *height, *depth],
                _ => return None,
            };
            let (scale, offset) = super::lighting::h4_grading_scale_offset(size);
            self.grading.map(|(_, texture)| super::bloom::H4BloomGrading { texture, scale, offset })
        }
    }

    /// Halo 4's `final_composite`: see `h4::lighting::H4_COMPOSITE_FRAGMENT`.
    pub(super) unsafe fn draw_composite(
        &mut self,
        cx: &mut FrameCx<'_>,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        depth: Option<glow::NativeTexture>,
        screen: super::super::gen3::screen_effects::H3ScreenSettings,
    ) {
        use super::lighting::{H4_COMPOSITE_FRAGMENT, H4_DARK_MULTIPLIER};
        let (gl, frame, core) = (cx.gl, cx.frame, &mut *cx.core);
        unsafe {
            let grading = self.grading_texture(gl, frame);
            if self.composite_program.is_none() {
                self.composite_program = screen_program(gl, H4_COMPOSITE_FRAGMENT);
            }
            let Some(program) = self.composite_program else {
                return;
            };
            gl.use_program(Some(program));
            let bind = |unit: u32, target: u32, texture: Option<glow::NativeTexture>, name: &str| {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(target, texture);
                gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), unit as i32);
            };
            bind(0, glow::TEXTURE_2D, Some(accumulation), "accumulation");
            bind(1, glow::TEXTURE_2D, bloom.or(core.gen3.no_bloom), "bloom");
            bind(2, glow::TEXTURE_2D, depth, "scene_depth");
            bind(3, glow::TEXTURE_3D, grading.as_ref().map(|grading| grading.texture).or(core.white_volume), "grading");
            gl.active_texture(glow::TEXTURE0);
            let (scale, offset) = grading.as_ref().map_or(([1.0; 3], [0.0; 3]), |grading| (grading.scale, grading.offset));
            gl.uniform_3_f32_slice(gl.get_uniform_location(program, "grading_scale").as_ref(), &scale);
            gl.uniform_3_f32_slice(gl.get_uniform_location(program, "grading_offset").as_ref(), &offset);
            gl.uniform_1_f32(gl.get_uniform_location(program, "dark_multiplier").as_ref(), H4_DARK_MULTIPLIER);
            let filmic = super::lighting::h4_filmic_params(super::lighting::H4_FILMIC_CURVE, false);
            gl.uniform_1_f32_slice(gl.get_uniform_location(program, "filmic").as_ref(), &filmic);
            gl.uniform_1_f32(gl.get_uniform_location(program, "view_scale").as_ref(), screen.view_scale(HALF_LENS));
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(core.h3_empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.enable(glow::DEPTH_TEST);
        }
    }

    /// `c_render_active_camo::render_object_callback` for a Halo 4 object,
    /// over the finished picture: each part's `active_camo` entry point reads
    /// a copy of the picture so far (`resolve_framebuffer_for_active_camo`),
    /// `alpha_blend`. The game reads and writes its light ahead of the tone
    /// curve; the preview's backdrop is the picture, and at full camo nothing
    /// of the object is in it, so reading and writing the picture's bytes is
    /// the same. Fully transparent, nothing of the object is under it,
    /// and the nearest part alone shows, as its depth decides. Otherwise its
    /// lit base wrote depth, which the camo meets nudged toward the eye by
    /// `camo depth bias function`'s constant.
    pub(super) unsafe fn draw_camo(
        &mut self,
        cx: &mut FrameCx<'_>,
        batches: &[(RenderModelPreviewBatch, Arc<H3Material>)],
        draw: super::camo::H4CamoDraw,
        size: [i32; 2],
    ) {
        let (gl, frame, slot, core) = (cx.gl, cx.frame, cx.slot, &mut *cx.core);
        /// `camo depth bias function`: the kit's constant, in a 24-bit
        /// buffer's units.
        const DEPTH_BIAS: f32 = -2e-5 * 16_777_216.0;
        let [width, height] = size.map(|side| side as f32);
        core.gen3.pass_values = Some(
            [
                ("ps_activeCamoFactor", draw.factor),
                ("ps_distortionTextureTransform", draw.transform),
                ("distortionTextureEnabled", [f32::from(u8::from(draw.textured)); 4]),
                ("ps_textureSize", [width, height, 1.0 / width, 1.0 / height]),
            ]
            .into_iter()
            .collect(),
        );
        unsafe {
            if core.gen3.camo_scene.is_none() {
                core.gen3.camo_scene = gl.create_texture().ok();
            }
            gl.bind_texture(glow::TEXTURE_2D, core.gen3.camo_scene);
            gl.copy_tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8, 0, 0, size[0], size[1], 0);
            for (name, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            h3_set_cull(gl, frame);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            let camo = |this: &mut RenderCore| {
                for (batch, material) in batches {
                    if let Some(pass) = material.camo.as_ref() {
                        this.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
                    }
                }
            };
            if draw.has_base {
                gl.enable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(0.0, DEPTH_BIAS);
            } else {
                gl.depth_mask(true);
                gl.clear(glow::DEPTH_BUFFER_BIT);
                gl.disable(glow::BLEND);
                gl.color_mask(false, false, false, false);
                camo(core);
                gl.color_mask(true, true, true, true);
            }
            gl.depth_mask(false);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            camo(core);
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.depth_mask(true);
        }
        core.gen3.pass_values = None;
    }
}
