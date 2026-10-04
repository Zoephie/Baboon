use super::super::level::read_cell_into;
use super::*;

/// Bytes one vertex costs in a binary sidecar: position, normal and UV as
/// the `f32`s they already are.
const BINARY_BYTES_PER_VERTEX: usize = 3 * 4 + 3 * 4 + 2 * 4;
/// Bytes one triangle costs: three `u32` indices.
const BINARY_BYTES_PER_TRIANGLE: usize = 3 * 4;

/// Count what a whole level actually contains, rather than extrapolating a
/// slice of it linearly.
///
/// The distinction matters because the two quantities scale differently: a
/// level's *placements* grow with its cells, but its *unique meshes*
/// saturate — the same rock is scattered everywhere — and mesh bytes are
/// what a file is made of. Estimating a whole level from one area therefore
/// overstates it by however much the meshes repeat, which is the question
/// this answers with a count instead of a guess.
///
/// `BABOON_CENSUS_SAMPLE` caps how many prototypes are decoded for the size
/// measurement; omit it to measure every one. `BABOON_CENSUS_MESHES` caches
/// the mesh list, because walking 2,334 cells takes thirteen minutes and the
/// answer does not change between runs.
#[test]
#[ignore]
fn probe_full_level_census() {
    let root = std::env::var("BABOON_PROBE_PAKS").expect("BABOON_PROBE_PAKS");
    let sample: usize = std::env::var("BABOON_CENSUS_SAMPLE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(usize::MAX);
    let usmap = load_chimp_usmap(None).expect("usmap");
    let world = World::open(std::path::Path::new(&root), usmap).expect("mount");
    let meshes = level_mesh_list(&world);

    // What those meshes actually cost, measured rather than extrapolated.
    // The USDA figure is the exact text this exporter writes and the binary
    // figure is the same geometry as the raw arrays a sidecar would carry,
    // each measured before and after welding so the two questions — which
    // container, and whether to weld — are answered separately.
    let mut measured = 0usize;
    let mut unreadable = 0usize;
    let mut raw = GeometrySize::default();
    let mut welded_total = GeometrySize::default();
    for package in meshes.iter().take(sample) {
        let leaf = package.rsplit('/').next().unwrap_or("mesh");
        // The undecorated decode, so the weld's effect is what is being
        // measured rather than something already applied.
        let decoded = load_chimp_package(&world, package)
            .and_then(|document| decode_mesh_raw(&world, &document, MeshDetail::Nanite));
        let Ok(mesh) = decoded else {
            unreadable += 1;
            continue;
        };
        let (welded, report) = weld(&mesh);
        assert_eq!(
            welded.indices.len(),
            mesh.indices.len(),
            "{leaf}: welding dropped triangles"
        );
        assert_eq!(report.after, welded.vertices.len());
        raw.absorb(&mesh, prim_name(leaf).as_str());
        welded_total.absorb(&welded, prim_name(leaf).as_str());
        measured += 1;
        if measured % 100 == 0 {
            eprintln!(
                "  measured {measured} prototypes: {} MiB USDA raw, {} MiB welded",
                raw.usda_bytes >> 20,
                welded_total.usda_bytes >> 20
            );
        }
    }

    eprintln!(
        "\nmeasured {measured} of {} prototypes at Nanite detail ({unreadable} unreadable)\n\
             \x20             {:>16} {:>16}   change\n\
             \x20 vertices    {:>16} {:>16}   {:+.1}%\n\
             \x20 triangles   {:>16} {:>16}   {:+.1}%\n\
             \x20 USDA        {:>13.2} GiB {:>13.2} GiB   {:+.1}%\n\
             \x20 binary      {:>13.2} GiB {:>13.2} GiB   {:+.1}%",
        meshes.len(),
        "raw",
        "welded",
        raw.vertices,
        welded_total.vertices,
        change(raw.vertices as f64, welded_total.vertices as f64),
        raw.triangles,
        welded_total.triangles,
        change(raw.triangles as f64, welded_total.triangles as f64),
        gib(raw.usda_bytes),
        gib(welded_total.usda_bytes),
        change(raw.usda_bytes as f64, welded_total.usda_bytes as f64),
        gib(raw.binary_bytes),
        gib(welded_total.binary_bytes),
        change(raw.binary_bytes as f64, welded_total.binary_bytes as f64),
    );
}

fn gib(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

fn change(from: f64, to: f64) -> f64 {
    if from == 0.0 {
        0.0
    } else {
        (to - from) / from * 100.0
    }
}

/// What one form of the geometry costs, accumulated over every mesh.
#[derive(Default)]
struct GeometrySize {
    vertices: usize,
    triangles: usize,
    usda_bytes: usize,
    binary_bytes: usize,
}

impl GeometrySize {
    fn absorb(&mut self, mesh: &StaticMesh, prim: &str) {
        let vertices = mesh.vertices.len();
        let triangles = mesh.indices.len() / 3;
        // Measured by writing the text, not by estimating a per-number
        // width: the digits a float prints to are the file.
        let mut usd: Vec<u8> = Vec::new();
        write_mesh(
            &mut usd,
            &Prototype {
                prim: prim.to_owned(),
                mesh: StaticMesh {
                    indices: mesh.indices.clone(),
                    vertices: mesh.vertices.clone(),
                },
                material: "M".to_owned(),
            },
        );
        self.vertices += vertices;
        self.triangles += triangles;
        self.usda_bytes += usd.len();
        self.binary_bytes +=
            vertices * BINARY_BYTES_PER_VERTEX + triangles * BINARY_BYTES_PER_TRIANGLE;
    }
}

/// Every unique mesh C10 places, read from the cache when there is one.
///
/// The walk itself is the slow part and its answer is fixed for an install,
/// so a cached list is the difference between iterating on the measurement
/// in seconds and in quarter-hours.
fn level_mesh_list(world: &World) -> Vec<String> {
    let cache = std::env::var("BABOON_CENSUS_MESHES").ok();
    if let Some(path) = &cache
        && let Ok(text) = std::fs::read_to_string(path)
    {
        let meshes: Vec<String> = text.lines().map(str::to_owned).collect();
        eprintln!("{} unique meshes, from the cache at {path}", meshes.len());
        return meshes;
    }

    let cells: Vec<String> = world
        .packages()
        .iter()
        .filter(|record| record.name.to_lowercase().contains("/c10/_generated_/"))
        .map(|record| record.name.clone())
        .collect();
    eprintln!("C10 has {} generated cells", cells.len());

    // The saturation curve, sampled as the read progresses: if unique meshes
    // flatten while placements keep climbing, a per-area measurement cannot
    // be scaled to the level by cell count.
    let mut scene = LevelScene::default();
    for (read, cell) in cells.iter().enumerate() {
        if let Ok(document) = load_chimp_package(world, cell) {
            read_cell_into(&document, &mut scene);
        }
        let at = read + 1;
        if at % 200 == 0 || at == cells.len() {
            eprintln!(
                "  {at} cells: {} unique meshes, {} placements",
                scene.meshes.len(),
                scene.placements.len()
            );
        }
    }
    // A placement is a matrix: ~200 bytes of USDA text against 128 bytes of
    // binary, so the placement table is noise beside the geometry either way.
    eprintln!(
        "\nwhole level: {} cells read, {} unique meshes, {} placements, skipped {:?}\n\
             \x20 placements cost {:.1} MiB USDA / {:.1} MiB binary",
        scene.cells,
        scene.meshes.len(),
        scene.placements.len(),
        scene.skipped,
        (scene.placements.len() * 200) as f64 / (1024.0 * 1024.0),
        (scene.placements.len() * 128) as f64 / (1024.0 * 1024.0),
    );
    if let Some(path) = &cache {
        let _ = std::fs::write(path, scene.meshes.join("\n"));
    }
    scene.meshes
}
