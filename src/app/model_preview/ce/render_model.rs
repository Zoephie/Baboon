//! What Halo CE's gbxmodels add to a render model as it is built.

use super::super::RenderModelPreview;
use blam_tags::{RenderModel, TagFile};

/// `matrix4x3_rotation_from_quaternion` builds a CE marker from the conjugate
/// of what the tag stores, as it does a node, and `RenderModel` only turns
/// the nodes.
pub(in crate::app::model_preview) fn turn_markers(render_model: &mut RenderModel) {
    for marker in render_model
        .marker_groups
        .iter_mut()
        .flat_map(|group| &mut group.markers)
    {
        marker.rotation = marker.rotation.conjugate();
    }
}

/// A gbxmodel's `shaders` run parallel to its materials.
pub(in crate::app::model_preview) fn read_shader_permutations(
    render_tag: &TagFile,
    preview: &mut RenderModelPreview,
) {
    let Some(shaders) = render_tag
        .root()
        .field("shaders")
        .and_then(|field| field.as_block())
    else {
        return;
    };
    for (index, material) in preview.materials.iter_mut().enumerate() {
        let permutation = shaders
            .element(index)
            .and_then(|shader| shader.read_int_any("permutation"));
        material.ce.permutation = permutation.unwrap_or(0).max(0) as usize;
    }
}
