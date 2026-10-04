use super::super::level::read_cell_into;
use super::*;

/// Export a slice of a real level and check geometry is shared rather than
/// repeated. Skips unless `BABOON_PROBE_PAKS` points at an install.
#[test]
fn a_real_level_exports_as_shared_geometry() {
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
    // A slice, not the level: this runs in the ordinary test suite.
    for cell in cells.iter().step_by(97) {
        if let Ok(document) = load_chimp_package(&world, cell) {
            read_cell_into(&document, &mut scene);
        }
    }
    let (usd, report) = scene_to_usd(&world, &scene, MeshDetail::Fallback);

    assert!(report.instances > 0, "nothing was placed");
    assert!(
        report.prototypes < report.instances,
        "{} prototypes for {} instances: geometry was repeated, not shared",
        report.prototypes,
        report.instances
    );
    assert!(report.materials > 0, "no materials were resolved");
    // Every mesh appears once and only once, however many times it is placed.
    assert_eq!(
        usd.matches("def Mesh ").count(),
        report.prototypes,
        "a mesh was written more than once"
    );
    assert_eq!(usd.matches("instanceable = true").count(), report.instances);
    assert!(usd.starts_with("#usda 1.0"));
    assert!(usd.contains("upAxis = \"Z\""));
    assert!(usd.contains("metersPerUnit = 0.01"));

    eprintln!(
        "exported {} cells: {} prototypes, {} instances, {} materials, {} KiB",
        scene.cells,
        report.prototypes,
        report.instances,
        report.materials,
        usd.len() / 1024
    );
}

/// The streaming writer and the in-memory one must produce the same file.
///
/// Streaming exists so a whole level does not have to fit in memory, and the
/// only thing that makes it safe to use for the big exports is that it is
/// not a second, differently-behaved exporter. Splicing the geometry in from
/// a sidecar is exactly the kind of change that would show up as an off-by-
/// one brace or a missing newline, which is why this compares every byte
/// rather than a summary.
#[test]
fn streaming_an_export_writes_the_same_bytes_as_building_it_in_memory() {
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
    for cell in cells.iter().step_by(211) {
        if let Ok(document) = load_chimp_package(&world, cell) {
            read_cell_into(&document, &mut scene);
        }
    }
    let (in_memory, memory_report) = scene_to_usd(&world, &scene, MeshDetail::Fallback);

    let path = std::env::temp_dir().join("baboon-streaming-parity.usda");
    let stream_report = write_scene_usd(&world, &scene, MeshDetail::Fallback, &path)
        .expect("stream the export");
    let streamed = std::fs::read_to_string(&path).expect("read back the export");
    let _ = std::fs::remove_file(&path);

    assert_eq!(stream_report, memory_report);
    assert_eq!(
        streamed.len(),
        in_memory.len(),
        "streamed {} bytes against {} in memory",
        streamed.len(),
        in_memory.len()
    );
    assert!(streamed == in_memory, "the two writers disagree on content");
    // The sidecar the geometry passed through must not be left behind.
    assert!(!path.with_extension("prototypes.tmp").exists());
}
