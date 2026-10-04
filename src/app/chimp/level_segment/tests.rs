use super::*;

fn placed(mesh: usize, x: f64, y: f64) -> PlacedMesh {
    PlacedMesh {
        mesh,
        position: [x, y, 0.0],
    }
}

fn budget(triangles: usize, placements: usize) -> SegmentBudget {
    SegmentBudget {
        triangles,
        placements,
    }
}

#[test]
fn a_scene_inside_the_budget_is_one_segment() {
    let placements = [placed(0, 0.0, 0.0), placed(1, 10.0, 0.0)];
    let segments = segment(&placements, &[100, 100], budget(1_000, 100));
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].placements.len(), 2);
    assert_eq!(segments[0].triangles, 200);
    assert!(!segments[0].over_budget);
}

#[test]
fn too_many_placements_split_even_when_the_geometry_is_tiny() {
    // The failure that took the whole level down: one mesh, reused, with an
    // object count no geometry budget can see.
    let placements: Vec<PlacedMesh> = (0..100).map(|i| placed(0, i as f64, 0.0)).collect();
    let segments = segment(&placements, &[10], budget(1_000_000, 10));
    assert!(segments.len() >= 10, "{} segments", segments.len());
    for piece in &segments {
        assert!(piece.placements.len() <= 10);
        assert!(!piece.over_budget);
    }
    let total: usize = segments.iter().map(|s| s.placements.len()).sum();
    assert_eq!(total, 100);
}

#[test]
fn too_much_geometry_splits_even_when_the_placements_are_few() {
    let placements: Vec<PlacedMesh> = (0..8).map(|i| placed(i, i as f64 * 10.0, 0.0)).collect();
    let segments = segment(&placements, &[100; 8], budget(250, 1_000));
    assert!(segments.len() >= 4, "{} segments", segments.len());
    for piece in &segments {
        assert!(piece.triangles <= 250, "{} triangles", piece.triangles);
    }
}

#[test]
fn a_segment_counts_a_reused_mesh_once() {
    // The whole reason a shared library is affordable: placing a mesh a
    // thousand times costs one mesh, not a thousand.
    let placements: Vec<PlacedMesh> = (0..1_000).map(|i| placed(0, i as f64, 0.0)).collect();
    let segments = segment(&placements, &[5_000], budget(10_000, 10_000));
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].triangles, 5_000);
    assert_eq!(segments[0].meshes, vec![0]);
}

#[test]
fn every_placement_lands_in_exactly_one_segment() {
    // A split that loses or duplicates placements would silently change the
    // level, so this is the property that matters most.
    let placements: Vec<PlacedMesh> = (0..500)
        .map(|i| placed(i % 7, (i % 23) as f64 * 3.0, (i / 23) as f64 * 5.0))
        .collect();
    let segments = segment(&placements, &[1_000; 7], budget(4_000, 40));
    let mut seen: Vec<usize> = segments
        .iter()
        .flat_map(|piece| piece.placements.iter().copied())
        .collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 500, "placements were lost or duplicated");
}

#[test]
fn segments_are_spatially_contiguous() {
    // Two clusters far apart must not end up sharing a segment while a
    // nearer neighbour goes elsewhere - that is the whole point of cutting
    // on position rather than on the order placements happen to arrive in.
    let mut placements: Vec<PlacedMesh> = (0..20).map(|i| placed(0, i as f64, 0.0)).collect();
    placements.extend((0..20).map(|i| placed(1, 10_000.0 + i as f64, 0.0)));
    let segments = segment(&placements, &[10, 10], budget(15, 1_000));
    assert_eq!(segments.len(), 2);
    for piece in &segments {
        assert_eq!(piece.meshes.len(), 1, "a segment straddled both clusters");
    }
}

#[test]
fn a_mesh_too_big_for_the_budget_is_reported_rather_than_split_forever() {
    // One mesh cannot be divided, so the budget cannot be met. The segment
    // has to come back marked instead of recursing until the stack ends.
    let placements = [placed(0, 0.0, 0.0)];
    let segments = segment(&placements, &[10_000_000], budget(1_000, 1_000));
    assert_eq!(segments.len(), 1);
    assert!(segments[0].over_budget);
}

#[test]
fn placements_stacked_at_one_point_end_the_recursion() {
    let placements: Vec<PlacedMesh> = (0..100).map(|_| placed(0, 5.0, 5.0)).collect();
    let segments = segment(&placements, &[10], budget(1_000, 10));
    assert_eq!(segments.len(), 1);
    assert!(segments[0].over_budget);
    assert_eq!(segments[0].placements.len(), 100);
}

#[test]
fn an_empty_scene_makes_no_segments() {
    assert!(segment(&[], &[], SegmentBudget::default()).is_empty());
}

#[test]
fn the_default_budget_is_the_measured_ceiling() {
    let budget = SegmentBudget::default();
    assert_eq!(budget.triangles, 30_000_000);
    assert_eq!(budget.placements, 50_000);
}
