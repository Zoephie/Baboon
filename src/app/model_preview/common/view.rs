//! The view and the pose every game's sprites and attachments are placed by:
//! where the camera is, and where a posed node puts a point.

use super::camera::{ModelGpuCamera, eye_position, unrotate_view_vector};

/// Where the preview's camera sits and faces: what sprites, flares and
/// screen-facing quads are turned toward.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewFrame {
    pub(crate) eye: [f32; 3],
    pub(crate) forward: [f32; 3],
    pub(crate) right: [f32; 3],
    pub(crate) up: [f32; 3],
}

/// A point and direction on `node`, where the pose puts them.
pub(in crate::app::model_preview) fn posed_on(
    node: usize,
    position: [f32; 3],
    direction: [f32; 3],
    bones: Option<&[[f32; 4]]>,
) -> ([f32; 3], [f32; 3]) {
    let Some(rows) = bones.and_then(|bones| bones.get(node * 3..node * 3 + 3)) else {
        return (position, direction);
    };
    let [x, y, z] = position;
    let [i, j, k] = direction;
    let place = |row: &[f32; 4]| row[0] * x + row[1] * y + row[2] * z + row[3];
    let turn = |row: &[f32; 4]| row[0] * i + row[1] * j + row[2] * k;
    (
        [place(&rows[0]), place(&rows[1]), place(&rows[2])],
        [turn(&rows[0]), turn(&rows[1]), turn(&rows[2])],
    )
}

pub(in crate::app::model_preview) fn ce_flare_view(camera: &ModelGpuCamera) -> ViewFrame {
    let along = |vector| unrotate_view_vector(camera.yaw, camera.pitch, vector);
    ViewFrame {
        eye: eye_position(camera),
        forward: along([0.0, 1.0, 0.0]),
        right: along([1.0, 0.0, 0.0]),
        up: along([0.0, 0.0, 1.0]),
    }
}
