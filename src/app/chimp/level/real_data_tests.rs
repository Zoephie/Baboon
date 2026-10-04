use super::*;

/// Read part of a real Campaign Evolved level and check what comes out is a
/// coherent world rather than a plausible-looking pile of numbers.
///
/// Skips unless `BABOON_PROBE_PAKS` points at an install, in the same way
/// the editing-kit fixture tests skip: the layout this reads was measured
/// from shipped data, so shipped data is the only thing that can tell us it
/// still holds.
#[test]
fn a_real_level_reads_as_a_coherent_world() {
    let Ok(root) = std::env::var("BABOON_PROBE_PAKS") else {
        eprintln!("skipping: set BABOON_PROBE_PAKS to a Campaign Evolved Paks folder");
        return;
    };
    let usmap = crate::app::chimp::load_chimp_usmap(None).expect("bundled usmap");
    let world = World::open(std::path::Path::new(&root), usmap).expect("mount the install");
    let cells: Vec<String> = world
        .packages()
        .iter()
        .filter(|record| record.name.to_lowercase().contains("/c10/_generated_/"))
        .map(|record| record.name.clone())
        .collect();
    if cells.is_empty() {
        eprintln!("skipping: this install has no C10 cells");
        return;
    }

    let mut scene = LevelScene::default();
    for cell in cells.iter().step_by(11) {
        if let Ok(document) = crate::app::chimp::load_chimp_package(&world, cell) {
            read_cell_into(&document, &mut scene);
        }
    }

    assert!(scene.cells > 0, "no cell could be read");
    assert!(!scene.placements.is_empty(), "the level placed nothing");
    assert!(
        scene.meshes.len() < scene.placements.len(),
        "a level reuses its meshes; {} meshes for {} placements means instancing was lost",
        scene.meshes.len(),
        scene.placements.len()
    );
    // Skipping some is expected - Blueprint-inherited meshes are not in the
    // cell - but losing a large share of the level would mean the reading is
    // wrong rather than the data being awkward.
    assert!(
        scene.skipped.total() * 20 < scene.placements.len(),
        "skipped {} of {} placements",
        scene.skipped.total(),
        scene.placements.len()
    );
    for placement in &scene.placements {
        assert!(
            placement.world.iter().all(|value| value.is_finite()),
            "a placement is not a finite transform"
        );
        assert!(
            (placement.world[15] - 1.0).abs() < 1e-6,
            "a placement is not affine"
        );
    }
    let extent = scene.placements.iter().fold(
        ([f64::MAX; 3], [f64::MIN; 3]),
        |(mut min, mut max), placement| {
            for axis in 0..3 {
                min[axis] = min[axis].min(placement.world[12 + axis]);
                max[axis] = max[axis].max(placement.world[12 + axis]);
            }
            (min, max)
        },
    );
    // Centimetres: a Halo level is hundreds of metres across, not microns
    // and not light years.
    for axis in 0..3 {
        let span = extent.1[axis] - extent.0[axis];
        assert!(
            (100.0..5_000_000.0).contains(&span),
            "axis {axis} spans {span} cm, which is not a level"
        );
    }
    eprintln!(
        "read {} cells: {} meshes, {} placements, {} skipped",
        scene.cells,
        scene.meshes.len(),
        scene.placements.len(),
        scene.skipped.total()
    );
}
