use super::super::level::read_cell_into;
use super::*;

/// Read a slice of a real level, split it small, and check the pieces are a
/// level rather than a pile of files.
///
/// Skips unless `BABOON_PROBE_PAKS` points at an install.
#[test]
fn a_segmented_export_splits_into_referencing_pieces() {
    let Ok(root) = std::env::var("BABOON_PROBE_PAKS") else {
        eprintln!("skipping: set BABOON_PROBE_PAKS to a Campaign Evolved Paks folder");
        return;
    };
    let usmap = load_chimp_usmap(None).expect("bundled usmap");
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
    for cell in cells.iter().step_by(97) {
        if let Ok(document) = load_chimp_package(&world, cell) {
            read_cell_into(&document, &mut scene);
        }
    }
    let directory = std::env::temp_dir().join("baboon-segment-test");
    let _ = std::fs::remove_dir_all(&directory);
    // Small enough that this really does split, whatever the slice holds.
    let budget = SegmentBudget {
        triangles: 400_000,
        placements: 200,
    };
    let report = write_segmented_usd(
        &world,
        &scene,
        MeshDetail::Fallback,
        &directory,
        "c10",
        budget,
        &|_, _, _| {},
    )
    .expect("write the segmented export");

    assert!(report.segments > 1, "the budget did not force a split");
    assert!(report.prototypes > 0);
    let library = std::fs::read_to_string(directory.join("c10_prototypes.usda")).unwrap();
    // All the geometry, once, and nothing placed.
    assert_eq!(library.matches("def Mesh ").count(), report.prototypes);
    assert!(!library.contains("instanceable = true"));
    // A material a segment can actually reach: nested inside the prototype,
    // so the reference carries it.
    assert!(library.contains("</World/Prototypes/"));
    assert!(!library.contains("rel material:binding = </World/Materials/"));

    let mut instances = 0;
    for number in 0..report.segments {
        let path = directory.join(format!("c10_seg_{number:02}.usda"));
        let text = std::fs::read_to_string(&path).expect("a segment per count");
        // Placements only: geometry belongs to the library.
        assert!(
            !text.contains("def Mesh "),
            "segment {number} carries geometry"
        );
        assert!(text.starts_with("#usda 1.0"));
        assert!(text.contains("metersPerUnit = 0.01"));
        let referencing = text
            .matches("@./c10_prototypes.usda@</World/Prototypes/")
            .count();
        let placed = text.matches("instanceable = true").count();
        assert_eq!(referencing, placed, "a placement referenced nothing");
        instances += placed;
    }
    assert_eq!(instances, report.instances);
    assert_eq!(
        instances + report.dropped_placements,
        scene.placements.len(),
        "placements were lost between the scene and the segments"
    );

    let readme = std::fs::read_to_string(directory.join("c10_README.txt")).unwrap();
    assert!(readme.contains("400000 triangles"));
    assert!(readme.contains("200 placements"));
    // The one thing a user can get wrong, said plainly.
    assert!(readme.contains("Keep the library file beside the"));
    let _ = std::fs::remove_dir_all(&directory);
}
