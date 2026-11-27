//! The constants Halo 2's decorator vertex shaders read.

/// The constants Halo 2's decorator vertex shaders read (`float4 c[208]`)
/// for `frame`'s view: `c0` to `c3` the view and projection, row by row as
/// `dp3(position, c[i]) + c[i].w` takes them; `c4` to `c7` the eye's right,
/// up and forward and its place (`EYE_RIGHT`, `EYE_UP`, `EYE_FORWARD`); `c15.y`
/// the least a model's light falls to, 0 (`sub_936850`); `c53` the fade,
/// `SCALE = clamp(c53.y − distance · c53.x, c53.z, c53.w)`, held at 1 as no
/// fade; and `c203` to `c207` a model's own, set per placement.
pub(in crate::app::model_preview) fn halo_2_vertex_constants(
    frame: &super::super::super::ModelGpuFrame,
) -> Vec<[f32; 4]> {
    use super::super::super::gen3::pipeline::view_projection;
    use super::super::super::{eye_position, unrotate_view_vector};
    let mut constants = vec![[0.0; 4]; HALO_2_CONSTANTS];
    let columns = view_projection(&frame.camera);
    for (row, constant) in constants.iter_mut().take(4).enumerate() {
        *constant = std::array::from_fn(|column| columns[column * 4 + row]);
    }
    let (yaw, pitch) = (frame.camera.yaw, frame.camera.pitch);
    let axis = |vector| {
        let [x, y, z] = unrotate_view_vector(yaw, pitch, vector);
        [x, y, z, 0.0]
    };
    constants[4] = axis([1.0, 0.0, 0.0]);
    constants[5] = axis([0.0, 0.0, 1.0]);
    constants[6] = axis([0.0, 1.0, 0.0]);
    let [x, y, z] = eye_position(&frame.camera);
    constants[7] = [x, y, z, 1.0];
    constants[FADE] = [0.0, 1.0, 0.0, 1.0];
    constants
}

/// `float4 c[96*2+16]`.
const HALO_2_CONSTANTS: usize = 208;

/// `c53`, the fade.
const FADE: usize = 53;

/// `c203`, the first of a model's own five.
pub(in crate::app::model_preview) const HALO_2_RIGID_CONSTANTS: usize = 203;
