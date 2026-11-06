//! What Reach's and Halo 4's object self shadows share around their own maps
//! and applies: the casters, `render_shadow_generate`'s biased depth, the
//! scene's depth, and an apply blended by its least into the mask.

use eframe::glow::{self, HasContext as _};
use super::super::renderer::*;
use super::super::*;
use super::shaders::{H3Material, H3Pass};

/// A batch that casts, its material, and that material's `shadow_generate`.
pub(in crate::app::model_preview) type ShadowCaster<'a> = (&'a RenderModelPreviewBatch, &'a Arc<H3Material>, Arc<H3Pass>);

/// The batches that cast: opaque, no decal, and with a `shadow_generate`.
pub(in crate::app::model_preview) fn shadow_casters(batches: &[(RenderModelPreviewBatch, Arc<H3Material>)]) -> Vec<ShadowCaster<'_>> {
    batches
        .iter()
        .filter(|(_, material)| material.blend == gen3::shaders::H3Blend::Opaque && !material.decal)
        .filter_map(|(batch, material)| Some((batch, material, Arc::clone(material.shadow_generate.as_ref()?))))
        .collect()
}

impl RenderCore {
    /// `render_shadow_generate`: `casters`' depth alone into `framebuffer`, a
    /// square of `pixels`, seen through `view_projection`, biased by `slope`
    /// and by `depth` in depth units, culled as ever. Leaves depth testing on
    /// and every channel writable.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) unsafe fn generate_shadow_depth(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        casters: &[ShadowCaster<'_>],
        framebuffer: glow::NativeFramebuffer,
        pixels: i32,
        view_projection: [f32; 16],
        [slope, depth]: [f32; 2],
    ) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.viewport(0, 0, pixels, pixels);
            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.color_mask(false, false, false, false);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(slope, depth * (1 << 24) as f32);
            super::pipeline::h3_set_cull(gl, frame);
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
            self.gen3.shadow_view_projection = Some(view_projection);
            for (batch, material, pass) in casters {
                self.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
            }
            self.gen3.shadow_view_projection = None;
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.color_mask(true, true, true, true);
        }
    }

    /// The scene's depth and stencil, `size` of them, from `scene` (a pass's
    /// framebuffer that holds them) into `depth_copy`, to find each pixel's
    /// place in the world. Leaves the viewport at `size`.
    pub(in crate::app::model_preview) unsafe fn copy_scene_depth(
        gl: &glow::Context,
        size: [i32; 2],
        scene: glow::NativeFramebuffer,
        depth_copy: glow::NativeFramebuffer,
    ) {
        unsafe {
            gl.viewport(0, 0, size[0], size[1]);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(scene));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(depth_copy));
            let bits = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
            gl.blit_framebuffer(0, 0, size[0], size[1], 0, 0, size[0], size[1], bits, glow::NEAREST);
        }
    }

    /// The apply's start: `mask` cleared to `clear` (`setup_targets_shadow_mask`),
    /// `program` bound with `textures` on units 0 up, and the frame's
    /// `screen_to_world` and the light's `shadow_view_projection` set.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) unsafe fn begin_shadow_apply(
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        size: [i32; 2],
        mask: glow::NativeFramebuffer,
        clear: [f32; 4],
        program: glow::NativeProgram,
        textures: [(&str, glow::NativeTexture); 3],
        view_projection: &[f32; 16],
    ) -> Option<()> {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(mask));
            gl.clear_color(clear[0], clear[1], clear[2], clear[3]);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(program));
            let at = |name: &str| gl.get_uniform_location(program, name);
            for (unit, (name, texture)) in textures.into_iter().enumerate() {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.uniform_1_i32(at(name).as_ref(), unit as i32);
            }
            gl.active_texture(glow::TEXTURE0);
            let screen_to_world =
                super::screen_lights::screen_to_relative_world(&super::pipeline::view_projection(&frame.camera), size, [0.0; 3])?;
            gl.uniform_matrix_4_f32_slice(at("screen_to_world").as_ref(), true, screen_to_world.as_flattened());
            gl.uniform_matrix_4_f32_slice(at("shadow_view_projection").as_ref(), false, view_projection);
            Some(())
        }
    }

    /// The apply's triangle, kept to `channels` and the least of each, then
    /// the state the scene draws with again and the slot's vertex array.
    pub(in crate::app::model_preview) unsafe fn draw_shadow_apply(&mut self, gl: &glow::Context, slot: usize, [red, green, blue, alpha]: [bool; 4]) {
        unsafe {
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::MIN);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.color_mask(red, green, blue, alpha);
            gl.bind_vertex_array(Some(self.h3_empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
        }
    }
}
