//! The order blended batches draw in.

use super::super::super::{ModelGpuFrame, RenderModelPreviewBatch};
use super::ce_flare_view;

/// How far along the view a batch's middle lies, as sprites measure theirs.
pub(super) fn batch_distance(frame: &ModelGpuFrame, batch: &RenderModelPreviewBatch) -> f32 {
    let preview = &frame.preview;
    let start = batch.index_start as usize;
    let indices = preview
        .indices
        .get(start..start + batch.index_count as usize)
        .unwrap_or(&[]);
    let places = indices
        .iter()
        .filter_map(|index| preview.vertices.get(*index as usize));
    let (mut sum, mut count) = ([0.0; 3], 0.0);
    for vertex in places {
        sum = std::array::from_fn(|axis| sum[axis] + vertex.position[axis]);
        count += 1.0;
    }
    let view = ce_flare_view(&frame.camera);
    let middle: [f32; 3] = sum.map(|axis| axis / f32::max(count, 1.0));
    (0..3)
        .map(|axis| (middle[axis] - view.eye[axis]) * view.forward[axis])
        .sum()
}
