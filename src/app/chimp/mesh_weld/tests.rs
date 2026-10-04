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
