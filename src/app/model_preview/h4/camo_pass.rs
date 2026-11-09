//! Halo 4's full camo, drawn after the composite.

use super::super::gen3::render::Gen3Cx;
use super::super::gen3::shaders::H3Material;
use super::super::RenderModelPreviewBatch;
use super::super::gen3::pipeline::h3_set_cull;
use eframe::glow::{self, HasContext};
use std::sync::Arc;
use super::render::H4Game;
use super::super::renderer::RenderCore;

impl H4Game {
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
        cx: &mut Gen3Cx<'_>,
        batches: &[(RenderModelPreviewBatch, Arc<H3Material>)],
        draw: super::camo::H4CamoDraw,
        size: [i32; 2],
    ) {
        let (gl, frame, slot, core) = (cx.gl, cx.frame, cx.slot, &mut *cx.core);
        let state = &mut *cx.state;
        /// `camo depth bias function`: the kit's constant, in a 24-bit
        /// buffer's units.
        const DEPTH_BIAS: f32 = -2e-5 * 16_777_216.0;
        let [width, height] = size.map(|side| side as f32);
        state.pass_values = Some(
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
            let hooks = super::pass_hooks::h4_pass_hooks(state);
            if hooks.camo_scene.is_none() {
                hooks.camo_scene = gl.create_texture().ok();
            }
            gl.bind_texture(glow::TEXTURE_2D, hooks.camo_scene);
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
            let mut camo = |this: &mut RenderCore| {
                for (batch, material) in batches {
                    if let Some(pass) = material.camo.as_ref() {
                        state.draw_pass(this, gl, frame, slot, batch, material, pass, None);
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
        state.pass_values = None;
    }
}
