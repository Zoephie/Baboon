//! Reach's object self shadow on the GPU (`render_fullscreen_shadow_buffer`):
//! the object's depth from the light through its materials' `shadow_generate`,
//! then `shadow_apply` into the mask the static pass reads. See
//! `docs/reach-preview-research/shadow.md`.

use eframe::glow::{self, HasContext};

use super::super::gen3::render::Gen3Batch;
use super::super::gl::target::{Made, Sampling, Texels};
use super::super::gen3::shadow::shadow_casters;
use crate::app::model_preview::gl::screen::screen_program;
use super::super::frame::ModelGpuFrame;
use super::super::renderer::RenderCore;
use super::super::gen3::render::Gen3Cx;
use super::super::gen3::state::Gen3State;
use super::super::unrotate_view_vector;
use super::render::ReachGame;
use super::shadow::{ReachShadowCamera, ReachShadowFilter, hull_planes};

/// `shadow_apply.hlsl`'s `default_ps` with no level: `CHANNEL_TRANSFORM` is
/// (1, 1, 0.2, 1), and the pixels it touches are those of the scene inside
/// the hull, which the game finds with two stencil passes over its faces.
pub(in crate::app::model_preview) const APPLY_FRAGMENT: &str = include_str!("shaders/shadow_apply.frag");

/// The map and the mask, kept between frames.
pub(in crate::app::model_preview) struct ReachShadowTargets {
    map: glow::NativeTexture,
    map_framebuffer: glow::NativeFramebuffer,
    map_size: i32,
    mask: glow::NativeTexture,
    mask_framebuffer: glow::NativeFramebuffer,
    mask_size: [i32; 2],
}

impl ReachShadowTargets {
    unsafe fn new(gl: &glow::Context, map_size: i32, mask_size: [i32; 2]) -> Option<Self> {
        let mut made = Made::new(gl);
        unsafe {
            // `sub_140847650`: clamped, point.
            let map = made.depth([map_size; 2], Texels::DEPTH24, Sampling::NEAREST_CLAMPED, glow::DEPTH_ATTACHMENT)?;
            let mask = made.color(mask_size, Texels::RGBA8, Sampling::NEAREST)?;
            if !made.complete(&[map.framebuffer, mask.framebuffer]) {
                return None;
            }
            made.keep();
            Some(Self {
                map: map.texture,
                map_framebuffer: map.framebuffer,
                map_size,
                mask: mask.texture,
                mask_framebuffer: mask.framebuffer,
                mask_size,
            })
        }
    }

    unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.map_framebuffer);
            gl.delete_framebuffer(self.mask_framebuffer);
            gl.delete_texture(self.map);
            gl.delete_texture(self.mask);
        }
    }
}

impl ReachGame {
    /// The camera the object's shadow is drawn from this frame.
    fn shadow_camera(frame: &ModelGpuFrame, viewport: [i32; 2]) -> ReachShadowCamera {
        let (low, high) = (frame.preview.bounds_min, frame.preview.bounds_max);
        let center: [f32; 3] = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
        let radius = (0..3).map(|axis| (high[axis] - center[axis]).powi(2)).sum::<f32>().sqrt();
        // `render_projection_sphere_diameter_in_pixels`: across the picture's
        // height, as the projection scales it at the sphere's depth.
        let camera = &frame.camera;
        let forward = unrotate_view_vector(camera.yaw, camera.pitch, [0.0, 1.0, 0.0]);
        let ahead = (0..3).map(|axis| (center[axis] - camera.center[axis]) * forward[axis]).sum::<f32>() * camera.scale;
        let w = (1.0 + ahead * camera.perspective).max(0.1);
        let diameter = 2.0 * radius * camera.scale * camera.clip_scale[1] * viewport[1] as f32 * 0.5 / w;
        let direction = super::lighting::reach_shadow_direction(frame);
        ReachShadowCamera::new(direction, center, radius, diameter)
    }

    /// The object's shadow into its mask, if it casts one: the mask, or none.
    /// Reads the depth and normals the albedo pass left.
    pub(in crate::app::model_preview) unsafe fn draw_shadow(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        batches: &[Gen3Batch],
    ) -> Option<glow::NativeTexture> {
        let (gl, frame, slot) = (cx.gl, cx.frame, cx.slot);
        let core: &mut RenderCore = cx.core;
        let state: &mut Gen3State = cx.state;
        let caster = frame.preview.reach.shadow_caster?;
        let (size, albedo_framebuffer, normal, depth_copy, depth_copy_framebuffer) = state
            .targets
            .as_ref()
            .map(|targets| (targets.size, targets.albedo_framebuffer, targets.normal, targets.depth_copy, targets.depth_copy_framebuffer))?;
        let camera = Self::shadow_camera(frame, size);
        let casters = shadow_casters(batches);
        if casters.is_empty() {
            return None;
        }
        unsafe {
            let stale = self
                .shadow
                .as_ref()
                .is_some_and(|targets| targets.map_size != camera.pixels || targets.mask_size != size);
            if stale && let Some(old) = self.shadow.take() {
                old.delete(gl);
            }
            if self.shadow.is_none() {
                self.shadow = ReachShadowTargets::new(gl, camera.pixels, size);
            }
            let (map, map_framebuffer, mask, mask_framebuffer) =
                self.shadow.as_ref().map(|targets| (targets.map, targets.map_framebuffer, targets.mask, targets.mask_framebuffer))?;
            let bias = [caster.slope_bias, caster.depth_bias];
            let view_projection = camera.view_projection;
            state.generate_shadow_depth(core, gl, frame, slot, &casters, map_framebuffer, camera.pixels, view_projection, bias);
            Gen3State::copy_scene_depth(gl, size, albedo_framebuffer, depth_copy_framebuffer);
            if self.shadow_program.is_none() {
                self.shadow_program = screen_program(gl, APPLY_FRAGMENT);
            }
            let program = self.shadow_program?;
            let textures = [("scene_depth", depth_copy), ("normals", normal), ("shadow_map", map)];
            Gen3State::begin_shadow_apply(gl, frame, size, mask_framebuffer, [1.0, 0.5, 0.5, 1.0], program, textures, &view_projection)?;
            let at = |name: &str| gl.get_uniform_location(program, name);
            let texel = 1.0 / camera.pixels as f32;
            gl.uniform_2_f32(at("texel_offset").as_ref(), -texel, texel);
            gl.uniform_3_f32_slice(at("hull_center").as_ref(), &camera.center);
            gl.uniform_3_f32_slice(at("hull_left").as_ref(), &camera.left);
            gl.uniform_3_f32_slice(at("hull_up").as_ref(), &camera.up);
            gl.uniform_3_f32_slice(at("shadow_direction").as_ref(), &camera.direction);
            gl.uniform_2_f32(at("hull_scale").as_ref(), camera.half_depth, camera.radius);
            gl.uniform_4_f32_slice(at("hull_planes").as_ref(), hull_planes().as_flattened());
            let filter = match camera.filter() {
                ReachShadowFilter::Point => 0,
                ReachShadowFilter::Bilinear => 1,
                ReachShadowFilter::Fancy => 2,
            };
            gl.uniform_1_i32(at("filter_mode").as_ref(), filter);
            // `compute_lod_and_alpha`'s fade: full short of the shadow's fade
            // distance, which a preview's framing stays inside.
            gl.uniform_1_f32(at("shadow_alpha").as_ref(), 1.0);
            // The red and alpha the apply writes.
            state.draw_shadow_apply(core, gl, slot, [true, false, false, true]);
            Some(mask)
        }
    }
}
