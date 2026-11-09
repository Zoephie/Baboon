//! The DX9 self shadow: the shadow map, its light matrix and the shadow pass, which
//! Halo 2 draws and Halo 3 and ODST borrow.

use super::super::*;
use super::super::gl::scope::PostScope;
use super::super::gl::target::{Made, Sampling, Texels};
use super::super::dx9::passes::H2Layer;
use eframe::glow::{self, HasContext as _};

pub(in crate::app::model_preview) const H2_SHADOW_MAP_SIZE: i32 = 2048;

/// Past the units a pass's own samplers take.
pub(in crate::app::model_preview) const H2_SHADOW_MAP_UNIT: u32 = 8;

pub(in crate::app::model_preview) const H3_NORMAL_TARGET_UNIT: u32 = 9;

/// `c_rasterizer::render_globals.k_shadow_resolution`.
pub(in crate::app::model_preview) const H3_SHADOW_MAP_SIZE: i32 = 512;

/// `set_shadow_generate_mode`'s bias, the second in depth units.
pub(in crate::app::model_preview) const H3_SHADOW_SLOPE_BIAS: f32 = 1.414;

pub(in crate::app::model_preview) const H3_SHADOW_DEPTH_BIAS: f32 = 0.003;

/// A depth texture and the framebuffer to fill it through.
pub(in crate::app::model_preview) unsafe fn h2_shadow_map(gl: &glow::Context) -> Option<(glow::NativeFramebuffer, glow::NativeTexture)> {
    unsafe {
        let draw = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
        let read = gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING);
        let mut made = Made::new(gl);
        let size = [H2_SHADOW_MAP_SIZE; 2];
        let map = made.depth(size, Texels::DEPTH24, Sampling::NEAREST_CLAMPED, glow::DEPTH_ATTACHMENT);
        let complete = map.as_ref().is_some_and(|map| made.complete(&[map.framebuffer]));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
        let map = map.filter(|_| complete)?;
        made.keep();
        Some((map.framebuffer, map.texture))
    }
}

/// The view from the light, an orthographic box around the whole model.
pub(in crate::app::model_preview) fn h2_shadow_matrix(preview: &RenderModelPreview, toward: [f32; 3]) -> [f32; 16] {
    let center: [f32; 3] =
        std::array::from_fn(|axis| (preview.bounds_min[axis] + preview.bounds_max[axis]) * 0.5);
    let radius = (0..3)
        .map(|axis| (preview.bounds_max[axis] - preview.bounds_min[axis]).powi(2))
        .sum::<f32>()
        .sqrt()
        .max(1e-4)
        * 0.55;
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    };
    let unit = |v: [f32; 3]| {
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
        [v[0] / length, v[1] / length, v[2] / length]
    };
    let forward = toward.map(|value| -value);
    let right = unit(cross(forward, [0.0, 0.0, 1.0]));
    let up = cross(right, forward);
    let dot = |a: [f32; 3]| a[0] * center[0] + a[1] * center[1] + a[2] * center[2];
    let row = |axis: [f32; 3]| {
        [axis[0] / radius, axis[1] / radius, axis[2] / radius, -dot(axis) / radius]
    };
    let [x, y, z] = [row(right), row(up), row(forward)];
    [
        x[0], y[0], z[0], 0.0,
        x[1], y[1], z[1], 0.0,
        x[2], y[2], z[2], 0.0,
        x[3], y[3], z[3], 1.0,
    ]
}

impl RenderCore {
    /// Mark in the stencil what the key light cannot reach: the shadow map
    /// from the light's side, then the model in view against it.
    pub(in crate::app::model_preview) unsafe fn draw_h2_shadow(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        order: &[usize],
    ) -> bool {
        unsafe {
            let Some((framebuffer, _)) = *self.dx9.h2_shadow_map.get_or_insert_with(|| h2_shadow_map(gl)) else {
                return false;
            };
            let h3 = self.h3_draws(frame);
            let size = if h3 { H3_SHADOW_MAP_SIZE } else { H2_SHADOW_MAP_SIZE };
            let state = PostScope::capture(gl);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.disable(glow::SCISSOR_TEST);
            gl.depth_mask(true);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.viewport(0, 0, size, size);
            if h3 {
                gl.enable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(H3_SHADOW_SLOPE_BIAS, H3_SHADOW_DEPTH_BIAS * (1 << 24) as f32);
            }
            self.draw_h2_passes(gl, frame, slot, order, H2Layer::ShadowCaster, None);
            gl.disable(glow::POLYGON_OFFSET_FILL);
            state.restore(gl);
            if h3 {
                return true;
            }
            gl.clear_stencil(0);
            gl.clear(glow::STENCIL_BUFFER_BIT);
            gl.enable(glow::STENCIL_TEST);
            gl.stencil_func(glow::ALWAYS, 1, 0xFF);
            gl.stencil_op(glow::KEEP, glow::KEEP, glow::REPLACE);
            self.draw_h2_passes(gl, frame, slot, order, H2Layer::ShadowMask, None);
            gl.disable(glow::STENCIL_TEST);
            true
        }
    }
}
