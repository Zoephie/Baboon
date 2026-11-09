//! What Halo 4's passes read that the shared binder does not: the per-draw
//! functions and hologram rig, its own matrices, and the camo's copy of the
//! picture.

use std::sync::Arc;

use eframe::glow::{self, HasContext as _};

use super::super::RenderModelPreviewBatch;
use super::super::frame::ModelGpuFrame;
use super::super::gen3::pipeline::{Gen3PassHooks, camera_to_world, view_projection};
use super::super::gen3::shaders::{Gen3Dialect, Gen3Material, Gen3Pass, Gen3Value, Gen3Values};
use super::super::gen3::state::Gen3State;
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
    state
        .hooks
        .as_any_mut()
        .downcast_mut()
        .expect("Halo 4's pipeline keeps Halo 4's pass hooks")
}

impl Gen3PassHooks for H4PassHooks {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn draw_values(
        &self,
        frame: &ModelGpuFrame,
        batch: &RenderModelPreviewBatch,
        material: &Gen3Material,
        pass: &Arc<Gen3Pass>,
    ) -> Option<Gen3Values> {
        material.h4().map(|submit| {
            let object = batch.material_index as usize;
            let owner = frame
                .preview
                .materials
                .get(object)
                .map_or(0, |material| material.object);
            let drawn = DrawnObject {
                values: frame.function_values_of(object),
                time: frame.time,
                owner,
            };
            let albedo = material
                .albedo
                .as_ref()
                .is_some_and(|albedo| Arc::ptr_eq(albedo, pass));
            let fringe = material
                .fringe
                .as_ref()
                .is_some_and(|fringe| Arc::ptr_eq(fringe, pass));
            let draw = match self.submit_draw {
                Some(draw) => draw,
                None if albedo => H4Draw::Albedo,
                None if fringe => H4Draw::Fringe,
                None => H4Draw::Lit,
            };
            let mut values = submit.draw_values(draw, &drawn, h4_hologram_node(frame));
            // Only the opened object's count is read; attached Halo 4
            // objects bring geometry alone.
            let count = if owner == 0 {
                frame.preview.h4.change_color_count
            } else {
                0
            };
            let colors = object_parameters(frame.change_colors_of(object), count);
            values.insert(
                "ps_material_object_parameters".to_owned(),
                Gen3Value::FloatArray(colors),
            );
            values
        })
    }

    unsafe fn set_matrix(
        &self,
        gl: &glow::Context,
        location: Option<&glow::NativeUniformLocation>,
        frame: &ModelGpuFrame,
        material: &Gen3Material,
        name: &str,
        shadow_view_projection: Option<[f32; 16]>,
    ) -> bool {
        unsafe {
            // The shadow's generate draws from the light.
            if material.dialect() == Gen3Dialect::Halo4
                && name == "vs_view_view_projection_matrix"
                && let Some(matrix) = shadow_view_projection
            {
                gl.uniform_matrix_4_f32_slice(location, false, &matrix);
                return true;
            }
            if material.dialect() == Gen3Dialect::Halo4
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

/// The matrix a Halo 4 hologram's light rig turns by: the skinning matrix of
/// the object's node 1 (`min(1, count - 1)`, `handle_object_part`), which is
/// `world × inverse_bind` and so none at all in the bind pose. With the light
/// following the camera the rig also turns about the vertical by the eye's
/// heading, so the key light keeps the side it was authored on toward the
/// viewer; this is the preview's own.
pub(in crate::app::model_preview) fn h4_hologram_node(frame: &ModelGpuFrame) -> [[f32; 4]; 3] {
    let node = 1.min(frame.preview.nodes.len().saturating_sub(1));
    let mut rows = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    if let Some(posed) = frame
        .bones
        .as_ref()
        .and_then(|bones| bones.get(node * 3..node * 3 + 3))
    {
        rows.copy_from_slice(posed);
    }
    if !frame.light_follows_camera {
        return rows;
    }
    let camera = camera_to_world(frame);
    let heading = camera[6].atan2(camera[2]);
    let (sin, cos) = heading.sin_cos();
    let turn = [[cos, -sin, 0.0], [sin, cos, 0.0], [0.0, 0.0, 1.0]];
    std::array::from_fn(|row| {
        let mut turned: [f32; 4] =
            std::array::from_fn(|column| (0..3).map(|k| turn[row][k] * rows[k][column]).sum());
        turned[3] = rows[row][3];
        turned
    })
}

/// The rotation Halo 4's albedo pass encodes normals in and its lit pass
/// decodes them from (`EncodeWorldspaceNormal`, `DecodeWorldspaceNormal`).
/// The two only need to agree, but the encoding fails for a normal at -z, so
/// it is the camera's: whatever faces the camera has z toward it. Rowed for
/// HLSL, whose `mul(normal, axis)` turns world into camera space.
fn h4_normal_axis(frame: &ModelGpuFrame) -> [f32; 9] {
    let camera = camera_to_world(frame);
    [
        camera[0], camera[1], camera[2], camera[4], camera[5], camera[6], camera[8], camera[9],
        camera[10],
    ]
}

/// The matrices Halo 4's passes read, by name: `mul(position, matrix)` as
/// Halo 3's `View_Projection`, the camera's axes and eye in
/// `camera_to_world`'s columns (`vs_view_camera_right` is `._m00_m10_m20`).
pub(in crate::app::model_preview) fn h4_matrix(
    frame: &ModelGpuFrame,
    name: &str,
) -> Option<Vec<f32>> {
    match name {
        "vs_view_view_projection_matrix" => Some(view_projection(&frame.camera).to_vec()),
        "vs_view_camera_to_world_matrix" | "ps_camera_to_world_matrix" => {
            Some(camera_to_world(frame).to_vec())
        }
        "ps_worldspace_normal_axis" => Some(h4_normal_axis(frame).to_vec()),
        _ => None,
    }
}
