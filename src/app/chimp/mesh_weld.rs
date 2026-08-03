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
mod tests {
    use super::*;

    fn vertex(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> StaticVertex {
        StaticVertex {
            position,
            normal,
            uv,
        }
    }

    /// A plain vertex, distinguished only by its position.
    fn at(x: f32) -> StaticVertex {
        vertex([x, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0])
    }

    fn mesh_of(vertices: Vec<StaticVertex>, indices: Vec<u32>) -> StaticMesh {
        StaticMesh { indices, vertices }
    }

    /// Every corner of every triangle, as the attributes it actually resolves
    /// to. This is what a weld must leave untouched: the surface is the corners,
    /// not the vertex list they happen to be stored in.
    fn corners(mesh: &StaticMesh) -> Vec<([f32; 3], [f32; 3], [f32; 2])> {
        mesh.indices
            .iter()
            .map(|&index| {
                let vertex = &mesh.vertices[index as usize];
                (vertex.position, vertex.normal, vertex.uv)
            })
            .collect()
    }

    #[test]
    fn duplicates_merge_and_the_surface_is_unchanged() {
        // Two triangles sharing an edge, written as two separate clusters would
        // write them: the shared vertices appear twice.
        let mesh = mesh_of(
            vec![at(0.0), at(1.0), at(2.0), at(1.0), at(2.0), at(3.0)],
            vec![0, 1, 2, 3, 4, 5],
        );
        let (welded, report) = weld(&mesh);
        assert_eq!(report.before, 6);
        assert_eq!(report.after, 4);
        assert_eq!(report.merged(), 2);
        // The triangles are still the same triangles.
        assert_eq!(welded.indices.len(), mesh.indices.len());
        assert_eq!(corners(&welded), corners(&mesh));
    }

    #[test]
    fn a_hard_edge_is_not_welded_away() {
        // Same position, different normal: a crease. Welding on position alone
        // would merge these and smooth the edge.
        let sharp = vertex([1.0, 2.0, 3.0], [1.0, 0.0, 0.0], [0.25, 0.5]);
        let other_face = vertex([1.0, 2.0, 3.0], [0.0, 1.0, 0.0], [0.25, 0.5]);
        let mesh = mesh_of(vec![sharp, other_face], vec![0, 1, 0]);
        let (welded, report) = weld(&mesh);
        assert_eq!(report.merged(), 0);
        assert_eq!(welded.vertices.len(), 2);
        assert_eq!(corners(&welded), corners(&mesh));
    }

    #[test]
    fn a_uv_seam_is_not_welded_away() {
        // Same position and normal, different UV: where the texture wraps.
        // Merging these tears the texture across the seam.
        let left = vertex([1.0, 2.0, 3.0], [0.0, 0.0, 1.0], [0.0, 0.5]);
        let right = vertex([1.0, 2.0, 3.0], [0.0, 0.0, 1.0], [1.0, 0.5]);
        let mesh = mesh_of(vec![left, right], vec![0, 1, 0]);
        let (welded, report) = weld(&mesh);
        assert_eq!(report.merged(), 0);
        assert_eq!(welded.vertices.len(), 2);
        assert_eq!(corners(&welded), corners(&mesh));
    }

    #[test]
    fn normals_and_uvs_survive_a_merge_exactly() {
        // The values carried through must be the ones that went in, not a
        // rounded or averaged version of them.
        let a = vertex([0.5, -1.5, 2.25], [0.0, 0.6, 0.8], [0.8423903, 0.125]);
        let mesh = mesh_of(vec![a.clone(), a.clone(), a], vec![0, 1, 2]);
        let (welded, _) = weld(&mesh);
        assert_eq!(welded.vertices.len(), 1);
        assert_eq!(welded.vertices[0].normal, [0.0, 0.6, 0.8]);
        assert_eq!(welded.vertices[0].uv, [0.8423903, 0.125]);
        assert_eq!(welded.vertices[0].position, [0.5, -1.5, 2.25]);
    }

    #[test]
    fn signed_zeroes_are_the_same_point() {
        // Two clusters can disagree on the sign of a zero and still mean the
        // same vertex; the bits differ, the point does not.
        let positive = vertex([0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0]);
        let negative = vertex([-0.0, 0.0, 1.0], [0.0, -0.0, 1.0], [-0.0, 0.0]);
        let mesh = mesh_of(vec![positive, negative], vec![0, 1, 0]);
        let (_, report) = weld(&mesh);
        assert_eq!(report.merged(), 1);
    }

    #[test]
    fn indices_are_remapped_rather_than_left_pointing_at_the_old_list() {
        let mesh = mesh_of(vec![at(0.0), at(0.0), at(1.0)], vec![2, 1, 0]);
        let (welded, _) = weld(&mesh);
        assert_eq!(welded.vertices.len(), 2);
        // The old index 2 must now find the vertex that moved down to 1.
        assert_eq!(welded.indices, vec![1, 0, 0]);
        assert_eq!(corners(&welded), corners(&mesh));
    }

    #[test]
    fn an_empty_mesh_welds_to_nothing() {
        let (welded, report) = weld(&mesh_of(Vec::new(), Vec::new()));
        assert!(welded.vertices.is_empty());
        assert!(welded.indices.is_empty());
        assert_eq!(report.merged(), 0);
    }

    #[test]
    fn an_out_of_range_index_is_left_as_it_was() {
        // A broken index stays broken and visible rather than being quietly
        // pointed at whatever vertex happens to be there after the weld.
        let mesh = mesh_of(vec![at(0.0), at(0.0)], vec![0, 1, 9]);
        let (welded, _) = weld(&mesh);
        assert_eq!(welded.indices, vec![0, 0, 9]);
    }
}
