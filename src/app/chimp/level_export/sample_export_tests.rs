use super::super::level::read_cell_into;
use super::*;

/// Write as much of C10 as fits in a triangle budget, to find out how much
/// geometry an importer will actually take.
///
/// Budgeted on triangles rather than cells because triangles are what an
/// importer runs out of memory on, and a cell's cost varies enormously —
/// C10's cells range from near-empty to thousands of placements. The budget
/// counts *prototype* triangles, the geometry the file contains: instancing
/// means a mesh placed a thousand times is still written once, so that is
/// what the file weighs and what Blender allocates for it.
///
/// `BABOON_SAMPLE_TRIANGLES` sets the budget, `BABOON_SAMPLE_OUT` the file,
/// and `BABOON_SAMPLE_NANITE` selects full detail over the fallback.
#[test]
#[ignore]
fn write_triangle_budgeted_usd() {
    let root = std::env::var("BABOON_PROBE_PAKS").expect("BABOON_PROBE_PAKS");
    let out = std::env::var("BABOON_SAMPLE_OUT").expect("BABOON_SAMPLE_OUT");
    let budget: usize = std::env::var("BABOON_SAMPLE_TRIANGLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10_000_000);
    let detail = if std::env::var("BABOON_SAMPLE_NANITE").is_ok() {
        MeshDetail::Nanite
    } else {
        MeshDetail::Fallback
    };
    let usmap = load_chimp_usmap(None).expect("usmap");
    let world = World::open(std::path::Path::new(&root), usmap).expect("mount");
    let cells: Vec<String> = world
        .packages()
        .iter()
        .filter(|record| record.name.to_lowercase().contains("/c10/_generated_/"))
        .map(|record| record.name.clone())
        .collect();

    // Cells are taken whole: half a cell is not a place, and a mesh dropped
    // mid-cell would leave its neighbours standing around a hole.
    let mut scene = LevelScene::default();
    let mut triangles = 0usize;
    let mut counted = 0usize;
    for cell in &cells {
        let Ok(document) = load_chimp_package(&world, cell) else {
            continue;
        };
        read_cell_into(&document, &mut scene);
        // `meshes` only ever grows, and in first-seen order, so everything
        // past the high-water mark is new to this cell.
        while counted < scene.meshes.len() {
            triangles += load_chimp_package(&world, &scene.meshes[counted])
                .and_then(|mesh_document| decode_mesh(&world, &mesh_document, detail))
                .map(|mesh| mesh.indices.len() / 3)
                .unwrap_or(0);
            counted += 1;
        }
        if triangles >= budget {
            break;
        }
    }

    let out_path = std::path::PathBuf::from(&out);
    let report = write_scene_usd(&world, &scene, detail, &out_path).expect("write the sample");
    let written = std::fs::metadata(&out_path)
        .map(|meta| meta.len())
        .unwrap_or(0);

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
    eprintln!(
        "wrote {out} ({detail:?})\n\
             \x20 {} of {} cells: {} prototypes, {} instances, {} materials\n\
             \x20 {triangles} triangles of geometry, {:.2} MiB of text\n\
             \x20 spans {:.0} x {:.0} x {:.0} m\n\
             \x20 {} meshes unreadable, {} placements dropped, skipped reading {:?}",
        scene.cells,
        cells.len(),
        report.prototypes,
        report.instances,
        report.materials,
        written as f64 / (1024.0 * 1024.0),
        (extent.1[0] - extent.0[0]) / 100.0,
        (extent.1[1] - extent.0[1]) / 100.0,
        (extent.1[2] - extent.0[2]) / 100.0,
        report.unreadable_meshes,
        report.dropped_placements,
        scene.skipped,
    );
}

/// Write a segmented export of C10 for trying in Blender.
///
/// `BABOON_SEGMENT_DIR` is the folder, `BABOON_SEGMENT_STEP` reads every Nth
/// cell, and `BABOON_SEGMENT_TRIANGLES` / `BABOON_SEGMENT_PLACEMENTS`
/// override the budgets so a small area can still be made to split.
#[test]
#[ignore]
fn write_segmented_sample() {
    let root = std::env::var("BABOON_PROBE_PAKS").expect("BABOON_PROBE_PAKS");
    let directory = std::path::PathBuf::from(
        std::env::var("BABOON_SEGMENT_DIR").expect("BABOON_SEGMENT_DIR"),
    );
    let step: usize = std::env::var("BABOON_SEGMENT_STEP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let default = SegmentBudget::default();
    let budget = SegmentBudget {
        triangles: std::env::var("BABOON_SEGMENT_TRIANGLES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default.triangles),
        placements: std::env::var("BABOON_SEGMENT_PLACEMENTS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default.placements),
    };
    let detail = if std::env::var("BABOON_SAMPLE_NANITE").is_ok() {
        MeshDetail::Nanite
    } else {
        MeshDetail::Fallback
    };
    let usmap = load_chimp_usmap(None).expect("usmap");
    let world = World::open(std::path::Path::new(&root), usmap).expect("mount");
    let cells: Vec<String> = world
        .packages()
        .iter()
        .filter(|record| record.name.to_lowercase().contains("/c10/_generated_/"))
        .map(|record| record.name.clone())
        .collect();

    let mut scene = LevelScene::default();
    for cell in cells.iter().step_by(step) {
        if let Ok(document) = load_chimp_package(&world, cell) {
            read_cell_into(&document, &mut scene);
        }
    }
    let report = write_segmented_usd(
        &world,
        &scene,
        detail,
        &directory,
        "c10",
        budget,
        &|_, _, _| {},
    )
    .expect("write");
    eprintln!(
        "wrote {} ({detail:?})\n\
             \x20 {} cells -> {} segments ({} over budget)\n\
             \x20 {} prototypes, {} instances, {} materials, {} triangles\n\
             \x20 library {:.1} MiB, segments {:.1} MiB total\n\
             \x20 {} meshes unreadable, {} placements dropped",
        directory.display(),
        scene.cells,
        report.segments,
        report.over_budget,
        report.prototypes,
        report.instances,
        report.materials,
        report.triangles,
        report.library_bytes as f64 / (1024.0 * 1024.0),
        report.segment_bytes as f64 / (1024.0 * 1024.0),
        report.unreadable_meshes,
        report.dropped_placements,
    );
}

/// Write a Blender export of C10 for trying the emitted script against.
///
/// Shares `BABOON_SEGMENT_*` with [`write_segmented_sample`], since it is
/// the same level split the same way.
#[test]
#[ignore]
fn write_blend_sample() {
    let root = std::env::var("BABOON_PROBE_PAKS").expect("BABOON_PROBE_PAKS");
    let directory = std::path::PathBuf::from(
        std::env::var("BABOON_SEGMENT_DIR").expect("BABOON_SEGMENT_DIR"),
    );
    let step: usize = std::env::var("BABOON_SEGMENT_STEP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let default = SegmentBudget::default();
    let budget = SegmentBudget {
        triangles: std::env::var("BABOON_SEGMENT_TRIANGLES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default.triangles),
        placements: std::env::var("BABOON_SEGMENT_PLACEMENTS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default.placements),
    };
    let detail = if std::env::var("BABOON_SAMPLE_NANITE").is_ok() {
        MeshDetail::Nanite
    } else {
        MeshDetail::Fallback
    };
    let usmap = load_chimp_usmap(None).expect("usmap");
    let world = World::open(std::path::Path::new(&root), usmap).expect("mount");
    let cells: Vec<String> = world
        .packages()
        .iter()
        .filter(|record| record.name.to_lowercase().contains("/c10/_generated_/"))
        .map(|record| record.name.clone())
        .collect();

    let selected: Vec<String> = cells.iter().step_by(step).cloned().collect();
    let threads = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4);
    let started = std::time::Instant::now();
    let scene = super::super::level::read_cells(&world, &selected, threads, &|_| {});
    eprintln!(
        "read {} cells on {threads} threads in {:.1}s",
        scene.cells,
        started.elapsed().as_secs_f64()
    );
    let started = std::time::Instant::now();
    let report = write_blend_export(
        &world,
        &scene,
        detail,
        &directory,
        "c10",
        budget,
        &|_, _, _| {},
    )
    .expect("write");
    eprintln!("wrote geometry in {:.1}s", started.elapsed().as_secs_f64());
    eprintln!(
        "wrote {} ({detail:?})\n\
             \x20 {} cells -> {} meshes, {} placements, {} segments\n\
             \x20 {:.1} MiB of geometry\n\
             \x20 {} meshes unreadable, {} placements dropped",
        directory.display(),
        scene.cells,
        report.meshes,
        report.placements,
        report.segments,
        report.data_bytes as f64 / (1024.0 * 1024.0),
        report.unreadable_meshes,
        report.dropped_placements,
    );
}

/// Write a slice of C10 to a `.usda` for eyeballing in Blender.
///
/// `BABOON_PROBE_PAKS` selects the install, `BABOON_SAMPLE_OUT` the file,
/// and `BABOON_SAMPLE_STEP` how many cells to skip between reads.
#[test]
#[ignore]
fn write_sample_usd() {
    let root = std::env::var("BABOON_PROBE_PAKS").expect("BABOON_PROBE_PAKS");
    let out = std::env::var("BABOON_SAMPLE_OUT").expect("BABOON_SAMPLE_OUT");
    let step: usize = std::env::var("BABOON_SAMPLE_STEP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(12);
    let usmap = load_chimp_usmap(None).expect("usmap");
    let world = World::open(std::path::Path::new(&root), usmap).expect("mount");
    let cells: Vec<String> = world
        .packages()
        .iter()
        .filter(|record| record.name.to_lowercase().contains("/c10/_generated_/"))
        .map(|record| record.name.clone())
        .collect();

    let mut scene = LevelScene::default();
    for cell in cells.iter().step_by(step) {
        if let Ok(document) = load_chimp_package(&world, cell) {
            read_cell_into(&document, &mut scene);
        }
    }
    let detail = if std::env::var("BABOON_SAMPLE_NANITE").is_ok() {
        MeshDetail::Nanite
    } else {
        MeshDetail::Fallback
    };
    let (usd, report) = scene_to_usd(&world, &scene, detail);
    std::fs::write(&out, &usd).expect("write the sample");
    eprintln!(
        "wrote {out} ({detail:?})
  {} cells, {} prototypes, {} instances, {} materials,              {:.1} MiB
  skipped reading: {:?}
               dropped placements: {}",
        scene.cells,
        report.prototypes,
        report.instances,
        report.materials,
        usd.len() as f64 / (1024.0 * 1024.0),
        scene.skipped,
        report.dropped_placements
    );
}
