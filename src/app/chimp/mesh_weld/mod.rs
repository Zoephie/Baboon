//! Merging a decoded mesh's duplicate vertices back into shared ones.
//! It owns the weld and nothing else; decoding belongs to `blam-tags`, and what
//! an exporter does with the result belongs to its own module.
//!
//! Nanite decodes cluster by cluster, and every cluster repeats the vertices
//! along its boundary, so a decoded mesh arrives as a heap of disconnected
//! triangle patches rather than a surface. Measured across C10's 745 meshes:
//! 108.5 million vertices for 109.7 million triangles, where a closed surface
//! would have roughly half as many vertices as triangles. About half of every
//! mesh is therefore a duplicate of a vertex already in it.
//!
//! Two vertices are the same vertex only when their position, normal *and* UV
//! all agree. Welding on position alone would weld away exactly the detail the
//! rest of the pipeline is preserving: a hard edge is two co-located vertices
//! with different normals, and a UV seam is two with different UVs. Both are
//! deliberate, and merging them would smooth creases and tear textures.
//!
//! The comparison is on the bits, not a tolerance. A cluster boundary repeats
//! values that decoded identically, so exact equality finds the duplicates that
//! are actually there; a tolerance would additionally merge vertices that were
//! merely close, which is a different operation with a different failure mode.

use blam_tags::iostore::static_mesh::{StaticMesh, StaticVertex};
use std::collections::HashMap;

/// A vertex reduced to the bits of everything that distinguishes it.
///
/// `-0.0` is folded into `0.0`: they are the same point, and the same point
/// arriving from two clusters with different signs of zero is still one vertex.
type VertexKey = [u32; 8];

fn key_of(vertex: &StaticVertex) -> VertexKey {
    let bits = |value: f32| (if value == 0.0 { 0.0 } else { value }).to_bits();
    [
        bits(vertex.position[0]),
        bits(vertex.position[1]),
        bits(vertex.position[2]),
        bits(vertex.normal[0]),
        bits(vertex.normal[1]),
        bits(vertex.normal[2]),
        bits(vertex.uv[0]),
        bits(vertex.uv[1]),
    ]
}

/// What a weld removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct WeldReport {
    pub(in crate::app) before: usize,
    pub(in crate::app) after: usize,
}

impl WeldReport {
    #[cfg(test)]
    pub(in crate::app) fn merged(&self) -> usize {
        self.before.saturating_sub(self.after)
    }
}

/// Merge vertices that agree on position, normal and UV, rewriting the indices
/// to match.
///
/// Every triangle survives, and every corner still resolves to the vertex it
/// resolved to before — the mesh describes the same surface with the duplicates
/// removed, which is what makes it editable rather than a pile of loose
/// triangles.
pub(in crate::app) fn weld(mesh: &StaticMesh) -> (StaticMesh, WeldReport) {
    let mut vertices: Vec<StaticVertex> = Vec::with_capacity(mesh.vertices.len());
    let mut seen: HashMap<VertexKey, u32> = HashMap::with_capacity(mesh.vertices.len());
    // Built over the old vertex list rather than per corner, so a vertex used a
    // hundred times is looked up once.
    let mut remap: Vec<u32> = Vec::with_capacity(mesh.vertices.len());

    for vertex in &mesh.vertices {
        let next = vertices.len() as u32;
        let index = *seen.entry(key_of(vertex)).or_insert(next);
        if index == next {
            vertices.push(vertex.clone());
        }
        remap.push(index);
    }

    let indices = mesh
        .indices
        .iter()
        // An index past the end is left alone rather than remapped to something
        // plausible: it is already broken, and inventing a target would hide it.
        .map(|&index| remap.get(index as usize).copied().unwrap_or(index))
        .collect();

    let report = WeldReport {
        before: mesh.vertices.len(),
        after: vertices.len(),
    };
    (StaticMesh { indices, vertices }, report)
}

#[cfg(test)]
mod tests;
