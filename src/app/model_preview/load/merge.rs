//! Merging one preview into another: overlays into a model, BSPs into a scenario, and attached objects into their parent.

use super::*;

/// Append every region of `src` into `dst`, offsetting vertex, index, and
/// material references. Regions with a name `dst` already lists are merged
/// into the existing entry.
pub(in crate::app::model_preview) fn merge_preview_append(dst: &mut RenderModelPreview, src: &RenderModelPreview) {
    let Some(vertex_base) = VertexBase::of(dst.vertices.len(), src.vertices.len(), Overflow::RefusesAppend) else {
        return;
    };
    let material_base = dst.materials.len();
    dst.materials.extend(src.materials.iter().cloned());
    dst.errors.extend(src.errors.iter().cloned());
    for vertex in &src.vertices {
        expand_preview_bounds_local(&mut dst.bounds_min, &mut dst.bounds_max, vertex.position);
        dst.vertices.push(*vertex);
    }
    // The PRT attributes stay parallel to the vertices: overlays have none.
    if let Some(prt) = dst.h2.prt.as_mut() {
        prt.pad(src.vertices.len());
    }
    if let Some(prt) = dst.gen3.prt.as_mut() {
        prt.pad(src.vertices.len());
    }
    for batch in &src.batches {
        let start = batch.index_start as usize;
        let end = start
            .saturating_add(batch.index_count as usize)
            .min(src.indices.len());
        if start >= end {
            continue;
        }
        let index_start = dst.indices.len() as u32;
        dst.indices.extend(
            src.indices[start..end]
                .iter()
                .map(|&index| vertex_base.offset(index)),
        );
        dst.batches.push(RenderModelPreviewBatch {
            region_name: batch.region_name.clone(),
            permutation_name: batch.permutation_name.clone(),
            material_index: (batch.material_index as usize + material_base).min(u16::MAX as usize)
                as u16,
            index_start,
            index_count: (end - start) as u32,
            flat_color: batch.flat_color,
            layer: batch.layer,
            sort_position: batch.sort_position,
        });
    }
    // A region `dst` has keeps its own permutations: an overlay's names would
    // show as choices that hide the render geometry, and its batches fall back
    // to their first permutation when the chosen one is not theirs.
    for region in &src.regions {
        if !dst.regions.iter().any(|existing| existing.name == region.name) {
            dst.regions.push(region.clone());
        }
    }
}
