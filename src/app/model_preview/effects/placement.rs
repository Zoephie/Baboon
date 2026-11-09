//! Where a part sits: its marker, rotation and frame.

use super::*;

pub(in crate::app::model_preview) fn marker_name(holder: TagStruct<'_>, field: &str) -> String {
    holder.read_string_id(field).or_else(|| holder.read_string(field)).unwrap_or_default()
}

/// `matrix4x3_rotation_from_angles` at no roll: forward, left and up.
pub(super) fn part_rotation(yaw: f32, pitch: f32) -> [[f32; 3]; 3] {
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    [
        [cos_pitch * cos_yaw, sin_yaw, sin_pitch * cos_yaw],
        [-cos_pitch * sin_yaw, cos_yaw, -sin_pitch * sin_yaw],
        [-sin_pitch, 0.0, cos_pitch],
    ]
}

/// `effect_get_part_to_node`: a part's own frame within its location's.
pub(in crate::app::model_preview) fn part_frame(location: &MarkerFrame, offset: [f32; 3], yaw: f32, pitch: f32) -> MarkerFrame {
    let of_location = |vector: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|axis| (0..3).map(|at| location.axes[at][axis] * vector[at]).sum())
    };
    let moved = of_location(offset);
    MarkerFrame {
        axes: part_rotation(yaw, pitch).map(of_location),
        position: std::array::from_fn(|axis| location.position[axis] + moved[axis]),
        node: location.node,
    }
}

/// The offset and turn `part_frame` wants, from the tag.
pub(in crate::app::model_preview) fn read_part_placement(part: TagStruct<'_>) -> ([f32; 3], f32, f32) {
    let offset = part.read_point3d("relative offset");
    let angles = part.read_euler2d("relative orientation (yaw, pitch)");
    ([offset.x, offset.y, offset.z], angles.yaw, angles.pitch)
}
