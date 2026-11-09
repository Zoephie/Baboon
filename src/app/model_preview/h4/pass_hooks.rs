//! What Halo 4's passes read that the shared binder does not: the per-draw
//! functions and hologram rig, its own matrices, and the camo's copy of the
//! picture.

use std::sync::Arc;

use eframe::glow::{self, HasContext as _};

use super::super::RenderModelPreviewBatch;
use super::super::frame::ModelGpuFrame;
use super::super::gen3::pipeline::{Gen3PassHooks, h4_hologram_node, h4_matrix};
use super::super::gen3::state::Gen3State;
use super::super::gen3::shaders::{H3Dialect, H3Material, H3Pass, H3Value, H3Values};
use super::materials::submit::{DrawnObject, H4Draw, object_parameters};

/// Halo 4's pass hooks, and what its stages set for them.
#[derive(Default)]
pub(in crate::app::model_preview) struct H4PassHooks {
    /// The picture as Halo 4's camo reads it, bytes as they are.
    pub(in crate::app::model_preview) camo_scene: Option<glow::NativeTexture>,
    /// Which draw of a Halo 4 material is under way, when not its albedo or
    /// own pass (a hologram's or `opaque_alpha_blend`'s fringe).
    pub(in crate::app::model_preview) submit_draw: Option<H4Draw>,
}

/// Halo 4's pass hooks on its pipeline's state.
pub(in crate::app::model_preview) fn h4_pass_hooks(state: &mut Gen3State) -> &mut H4PassHooks {
    state.hooks.as_any_mut().downcast_mut().expect("Halo 4's pipeline keeps Halo 4's pass hooks")
}

impl Gen3PassHooks for H4PassHooks {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn draw_values(
        &self,
        frame: &ModelGpuFrame,
        batch: &RenderModelPreviewBatch,
        material: &H3Material,
        pass: &Arc<H3Pass>,
    ) -> Option<H3Values> {
        material.h4().map(|submit| {
            let object = batch.material_index as usize;
            let owner = frame.preview.materials.get(object).map_or(0, |material| material.object);
            let drawn = DrawnObject { values: frame.function_values_of(object), time: frame.time, owner };
            let albedo = material.albedo.as_ref().is_some_and(|albedo| Arc::ptr_eq(albedo, pass));
            let fringe = material.fringe.as_ref().is_some_and(|fringe| Arc::ptr_eq(fringe, pass));
            let draw = match self.submit_draw {
                Some(draw) => draw,
                None if albedo => H4Draw::Albedo,
                None if fringe => H4Draw::Fringe,
                None => H4Draw::Lit,
            };
            let mut values = submit.draw_values(draw, &drawn, h4_hologram_node(frame));
            // Only the opened object's count is read; attached Halo 4
            // objects bring geometry alone.
            let count = if owner == 0 { frame.preview.h4.change_color_count } else { 0 };
            let colors = object_parameters(frame.change_colors_of(object), count);
            values.insert("ps_material_object_parameters".to_owned(), H3Value::FloatArray(colors));
            values
        })
    }

    unsafe fn set_matrix(
        &self,
        gl: &glow::Context,
        location: Option<&glow::NativeUniformLocation>,
        frame: &ModelGpuFrame,
        material: &H3Material,
        name: &str,
        shadow_view_projection: Option<[f32; 16]>,
    ) -> bool {
        unsafe {
            // The shadow's generate draws from the light.
            if material.dialect() == H3Dialect::Halo4
                && name == "vs_view_view_projection_matrix"
                && let Some(matrix) = shadow_view_projection
            {
                gl.uniform_matrix_4_f32_slice(location, false, &matrix);
                return true;
            }
            if material.dialect() == H3Dialect::Halo4
                && let Some(matrix) = h4_matrix(frame, name)
            {
                match matrix.len() {
                    9 => gl.uniform_matrix_3_f32_slice(location, false, &matrix),
                    _ => gl.uniform_matrix_4_f32_slice(location, false, &matrix),
                }
                return true;
            }
        }
        false
    }

    fn camo_scene(&self) -> Option<glow::NativeTexture> {
        self.camo_scene
    }
}
