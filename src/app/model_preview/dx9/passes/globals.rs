//! The default lighting globals a pass's constants fall back to.

/// What a preview has in place of a level's lighting: the colors the
/// no-lightmap passes hold as literals, so both paths light a surface alike.
/// The vectors are the game's `default_object_lighting`.
pub(in crate::app::model_preview) const DEFAULT_AMBIENT: [f32; 4] = [0.0627, 0.0627, 0.0627, 1.0];

pub(in crate::app::model_preview) const DEFAULT_DIRECT_COLOR: [f32; 4] = [0.502, 0.490, 0.471, 1.0];

pub(in crate::app::model_preview) const DEFAULT_DIRECT_VECTOR: [f32; 4] =
    [-0.577, -0.577, -0.577, 1.0];

pub(in crate::app::model_preview) const DEFAULT_INDIRECT_COLOR: [f32; 4] =
    [0.373, 0.353, 0.333, 1.0];

pub(in crate::app::model_preview) const DEFAULT_INDIRECT_VECTOR: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

pub(in crate::app::model_preview) fn fill_components(
    target: &mut [f32; 4],
    value: [f32; 4],
    mask: [bool; 4],
) {
    for (index, write) in mask.into_iter().enumerate() {
        if write {
            target[index] = value[index];
        }
    }
}
