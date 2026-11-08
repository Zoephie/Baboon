//! GLSL pieces spliced into more than one program's source.

/// `view_depth`: a window depth back to the distance along the view, by
/// `u_depth`'s projection terms (see [`scene_depth_terms`](crate::app::model_preview::common::camera::scene_depth_terms)), for the sprites
/// that fade against the scene's depth.
macro_rules! glsl_view_depth {
    () => {
        "float view_depth(float window) {
    float ndc = window * 2.0 - 1.0;
    return (ndc - u_depth.y) / (u_depth.x - ndc * u_depth.z);
}
"
    };
}

pub(in crate::app::model_preview) use glsl_view_depth;
