//! Turning a [`LevelScene`] into a USD scene.
//! It owns the scene-to-USD conversion; reading cells belongs to
//! [`super::level`], and mesh decoding belongs to `blam-tags`.
//!
//! USD describes a scene, not a mesh, and its instancing is exactly what a
//! level needs: each mesh is written once as a prototype, and every placement is
//! an `instanceable` prim that references it. Counted over all 2,334 of its
//! cells, C10 places 296,399 copies of 745 meshes, so storing geometry per
//! placement is the difference between a file that opens and one that does not.
//!
//! A placement is a `matrix4d`, which matters more than it sounds. Roughly a
//! fifth of Campaign Evolved's placements are non-uniformly scaled or mirrored,
//! and a format carrying only a quaternion and a single scale can say neither —
//! it has to bake per-placement geometry, losing exactly the sharing that made
//! the export affordable. A matrix says all of it exactly, and USD's is
//! row-major with the translation in the last row, which is Unreal's own
//! `FMatrix` layout, so a placement is written through unchanged.
//!
//! Units stay Unreal's centimetres, declared as `metersPerUnit`, and the scene
//! is Z-up as both Unreal and Blender are.

use super::*;

use std::fmt::Write as _;

use super::level::{LevelScene, WorldMatrix};
use super::mesh_weld::weld;

/// How much of a mesh to export.
///
/// For a Nanite asset these are genuinely different meshes rather than two
/// levels of one. `UStaticMesh` keeps only a coarse fallback in its render data
/// — the real geometry lives in the Nanite pages — so the choice is between a
/// proxy built for hardware that cannot run Nanite and the finest cut there is,
/// with nothing in between until the decoder can emit a coarser cluster cut.
///
/// The gap is not marginal: the same 25 cells of C10 came to 50 MB of text
/// through the fallback and 3.0 GB through Nanite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum MeshDetail {
    /// The cooked render LOD. For a Nanite asset, a coarse proxy.
    Fallback,
    /// Full Nanite geometry where an asset has it.
    Nanite,
}

/// Decode a mesh and merge the duplicate vertices out of it.
///
/// Nanite decodes cluster by cluster and every cluster repeats its boundary
/// vertices, so what comes back is a heap of disconnected triangle patches
/// rather than a surface. Welding is unconditional because it costs nothing to
/// be right about: measured over all 745 of C10's meshes it removes 29.6% of the
/// vertices and 22% of the file while leaving the triangle count identical, and
/// the surface arrives connected rather than as loose patches.
fn decode_mesh(
    world: &World,
    document: &ChimpDocument,
    detail: MeshDetail,
) -> Result<StaticMesh, String> {
    decode_mesh_raw(world, document, detail).map(|mesh| weld(&mesh).0)
}

fn decode_mesh_raw(
    world: &World,
    document: &ChimpDocument,
    detail: MeshDetail,
) -> Result<StaticMesh, String> {
    let header_size = document.header.summary.header_size as usize;
    match detail {
        MeshDetail::Fallback => StaticMesh::from_package(&document.original, header_size),
        MeshDetail::Nanite => {
            // Nanite geometry is streamed from the package's bulk data, so the
            // decoder needs it alongside the package itself.
            let archive = &world.archives()[document.provider.container];
            let bulk = archive
                .chunk_index_for(&document.provider.entry_path)
                .ok()
                .and_then(|chunk| archive.read_bulk_for(chunk, 0).ok());
            StaticMesh::from_package_preferring_nanite(
                &document.original,
                header_size,
                bulk.as_deref(),
            )
        }
    }
    .map_err(|error| format!("{error:#}"))
}

/// Print a number at full precision, with negative zero folded into zero.
///
/// Values are written as decoded: the geometry is the product, and rounding it
/// to save bytes trades the thing being exported for the size of the file that
/// carries it.
/// Widening an `f32` before printing it is not free: `0.8423903` becomes
/// `0.8423902988433838`, the double expansion of a number that only ever had
/// single precision. Vertex data stays `f32` so it prints as what it is.
fn number_f32(value: f32) -> String {
    if !value.is_finite() || value == 0.0 {
        return "0".to_owned();
    }
    format!("{value}")
}

fn number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    format!("{value}")
}

/// What an export produced, and what it could not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct LevelExportReport {
    /// Meshes written once and referenced by every placement of them.
    pub(in crate::app) prototypes: usize,
    pub(in crate::app) instances: usize,
    pub(in crate::app) materials: usize,
    /// Meshes whose geometry could not be read; their placements are absent.
    pub(in crate::app) unreadable_meshes: usize,
    pub(in crate::app) dropped_placements: usize,
}

/// A USD prim name: alphanumerics and underscores, never leading with a digit.
fn prim_name(raw: &str) -> String {
    let mut name = String::with_capacity(raw.len());
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            name.push(character);
        } else {
            name.push('_');
        }
    }
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name
}

/// A prototype's prim name, kept unique across meshes whose leaves collide.
fn unique_prim_name(raw: &str, taken: &mut HashSet<String>) -> String {
    let base = prim_name(raw);
    if taken.insert(base.clone()) {
        return base;
    }
    for suffix in 1.. {
        let candidate = format!("{base}_{suffix}");
        if taken.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("an unbounded search cannot fail")
}

/// One mesh, written once and referenced by every placement of it.
struct Prototype {
    prim: String,
    mesh: StaticMesh,
    material: String,
}

fn write_mesh(usd: &mut String, prototype: &Prototype) {
    let Prototype {
        prim,
        mesh,
        material,
    } = prototype;
    // An Xform *containing* a Mesh, not a bare Mesh. A reference composes the
    // target's content into the referencing prim, and the referencing prim's
    // own type wins — so an Xform referencing a Mesh receives the geometry
    // attributes while staying an Xform, and imports as a transform with no
    // mesh at all. Wrapping the mesh means the reference brings a child Mesh
    // with it, whatever the placement prim is.
    let _ = writeln!(usd, "        def Xform \"{prim}\"");
    let _ = writeln!(usd, "        {{");
    let _ = writeln!(usd, "            def Mesh \"Mesh\" (");
    let _ = writeln!(usd, "                prepend apiSchemas = [\"MaterialBindingAPI\"]");
    let _ = writeln!(usd, "                )");
    let _ = writeln!(usd, "            {{");
    let _ = writeln!(
        usd,
        "                rel material:binding = </World/Materials/{material}>"
    );
    let _ = writeln!(usd, "                uniform token subdivisionScheme = \"none\"");
    // Unreal winds its triangles clockwise; USD reads counter-clockwise as
    // front-facing unless told otherwise, which imports every surface
    // back-facing and reads as broken normals. The single-mesh exporters
    // sidestep this by negating Y to mirror into right-handed space, but a
    // level cannot: mirroring the geometry would mean mirroring every world
    // placement with it. Declaring the convention says the same thing and
    // leaves the world coordinates alone.
    let _ = writeln!(usd, "                uniform token orientation = \"leftHanded\"");

    let _ = write!(usd, "                point3f[] points = [");
    for (index, vertex) in mesh.vertices.iter().enumerate() {
        if index > 0 {
            let _ = write!(usd, ", ");
        }
        let [x, y, z] = vertex.position;
        let _ = write!(
            usd,
            "({}, {}, {})",
            number_f32(x),
            number_f32(y),
            number_f32(z)
        );
    }
    let _ = writeln!(usd, "]");

    let triangles = mesh.indices.len() / 3;
    let _ = write!(usd, "                int[] faceVertexCounts = [");
    for index in 0..triangles {
        if index > 0 {
            let _ = write!(usd, ", ");
        }
        let _ = write!(usd, "3");
    }
    let _ = writeln!(usd, "]");

    let _ = write!(usd, "                int[] faceVertexIndices = [");
    for (index, vertex) in mesh.indices.iter().take(triangles * 3).enumerate() {
        if index > 0 {
            let _ = write!(usd, ", ");
        }
        let _ = write!(usd, "{vertex}");
    }
    let _ = writeln!(usd, "]");

    let _ = write!(usd, "                normal3f[] primvars:normals = [");
    for (index, vertex) in mesh.vertices.iter().enumerate() {
        if index > 0 {
            let _ = write!(usd, ", ");
        }
        let [i, j, k] = vertex.normal;
        let _ = write!(
            usd,
            "({}, {}, {})",
            number_f32(i),
            number_f32(j),
            number_f32(k)
        );
    }
    let _ = writeln!(usd, "] (");
    let _ = writeln!(usd, "                    interpolation = \"vertex\"");
    let _ = writeln!(usd, "                )");

    let _ = write!(usd, "                texCoord2f[] primvars:st = [");
    for (index, vertex) in mesh.vertices.iter().enumerate() {
        if index > 0 {
            let _ = write!(usd, ", ");
        }
        // Unreal's V runs down the image and USD's runs up.
        let [u, v] = vertex.uv;
        let _ = write!(
            usd,
            "({}, {})",
            number_f32(u),
            number_f32(1.0 - v)
        );
    }
    let _ = writeln!(usd, "] (");
    let _ = writeln!(usd, "                    interpolation = \"vertex\"");
    let _ = writeln!(usd, "                )");
    let _ = writeln!(usd, "            }}");
    let _ = writeln!(usd, "        }}");
}

fn write_instance(usd: &mut String, index: usize, prototype: &str, world: &WorldMatrix) {
    let _ = writeln!(usd, "    def Xform \"inst_{index}\" (");
    // The mesh is not copied here: this prim references the one prototype and
    // is marked instanceable, so every placement of a mesh shares its geometry.
    let _ = writeln!(usd, "        instanceable = true");
    let _ = writeln!(
        usd,
        "        prepend references = </World/Prototypes/{prototype}>"
    );
    let _ = writeln!(usd, "    )");
    let _ = writeln!(usd, "    {{");
    let _ = write!(usd, "        matrix4d xformOp:transform = ( ");
    for row in 0..4 {
        if row > 0 {
            let _ = write!(usd, ", ");
        }
        let _ = write!(
            usd,
            "({}, {}, {}, {})",
            number(world[row * 4]),
            number(world[row * 4 + 1]),
            number(world[row * 4 + 2]),
            number(world[row * 4 + 3])
        );
    }
    let _ = writeln!(usd, " )");
    let _ = writeln!(
        usd,
        "        uniform token[] xformOpOrder = [\"xformOp:transform\"]"
    );
    let _ = writeln!(usd, "    }}");
}

/// Convert a level scene into a USD (`.usda`) document.
pub(in crate::app) fn scene_to_usd(
    world: &World,
    scene: &LevelScene,
    detail: MeshDetail,
) -> (String, LevelExportReport) {
    let mut report = LevelExportReport::default();
    let mut taken = HashSet::new();
    let mut materials: Vec<String> = Vec::new();
    let mut prototypes: Vec<Option<Prototype>> = Vec::with_capacity(scene.meshes.len());

    for package in &scene.meshes {
        let leaf = package.rsplit('/').next().unwrap_or("mesh");
        let loaded = load_chimp_document(world, package)
            .and_then(|document| {
                decode_mesh(world, &document, detail).map(|mesh| (document, mesh))
            });
        match loaded {
            Ok((document, mesh)) => {
                // The mesh's own material, resolved the same way the material
                // list written into a single-mesh export is.
                let material = prim_name(
                    &chimp_material_names(&document.header)
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| leaf.to_owned()),
                );
                if !materials.contains(&material) {
                    materials.push(material.clone());
                }
                prototypes.push(Some(Prototype {
                    prim: unique_prim_name(leaf, &mut taken),
                    mesh,
                    material,
                }));
            }
            Err(_) => {
                report.unreadable_meshes += 1;
                prototypes.push(None);
            }
        }
    }

    let mut usd = String::new();
    let _ = writeln!(usd, "#usda 1.0");
    let _ = writeln!(usd, "(");
    let _ = writeln!(usd, "    defaultPrim = \"World\"");
    let _ = writeln!(usd, "    metersPerUnit = 0.01");
    let _ = writeln!(usd, "    upAxis = \"Z\"");
    let _ = writeln!(usd, "    doc = \"Exported by Baboon\"");
    let _ = writeln!(usd, ")");
    let _ = writeln!(usd);
    let _ = writeln!(usd, "def Xform \"World\"");
    let _ = writeln!(usd, "{{");

    let _ = writeln!(usd, "    def Scope \"Materials\"");
    let _ = writeln!(usd, "    {{");
    for material in &materials {
        let _ = writeln!(usd, "        def Material \"{material}\"");
        let _ = writeln!(usd, "        {{");
        let _ = writeln!(
            usd,
            "            token outputs:surface.connect = </World/Materials/{material}/Surface.outputs:surface>"
        );
        let _ = writeln!(usd, "            def Shader \"Surface\"");
        let _ = writeln!(usd, "            {{");
        let _ = writeln!(
            usd,
            "                uniform token info:id = \"UsdPreviewSurface\""
        );
        let _ = writeln!(usd, "                token outputs:surface");
        let _ = writeln!(usd, "            }}");
        let _ = writeln!(usd, "        }}");
    }
    let _ = writeln!(usd, "    }}");

    // Geometry lives here once. Nothing draws it directly; the placements below
    // reference it, which is what keeps a quarter of a million copies down to
    // the size of the meshes themselves.
    let _ = writeln!(usd, "    def Scope \"Prototypes\"");
    let _ = writeln!(usd, "    {{");
    // Referenced, not shown: hiding the sources keeps every mesh from also
    // being drawn in a heap at the origin.
    let _ = writeln!(usd, "        uniform token visibility = \"invisible\"");
    for prototype in prototypes.iter().flatten() {
        write_mesh(&mut usd, prototype);
    }
    let _ = writeln!(usd, "    }}");

    for placement in &scene.placements {
        let Some(Some(prototype)) = prototypes.get(placement.mesh) else {
            report.dropped_placements += 1;
            continue;
        };
        write_instance(&mut usd, report.instances, &prototype.prim, &placement.world);
        report.instances += 1;
    }
    let _ = writeln!(usd, "}}");

    report.prototypes = prototypes.iter().flatten().count();
    report.materials = materials.len();
    (usd, report)
}

#[cfg(test)]
mod tests {
    use super::super::level::{IDENTITY, compose};
    use super::*;

    #[test]
    fn a_prim_name_is_an_identifier() {
        assert_eq!(prim_name("SM_Tree_Mangrove_A"), "SM_Tree_Mangrove_A");
        assert_eq!(prim_name("SM-Tree.01"), "SM_Tree_01");
        // USD will not accept a name that leads with a digit, and Campaign
        // Evolved's generated cells are named exactly that way.
        assert_eq!(prim_name("043ATWPYEEJ"), "_043ATWPYEEJ");
        assert_eq!(prim_name(""), "_");
    }

    #[test]
    fn colliding_leaves_get_distinct_prims() {
        // Two packages can end in the same name, and a prototype that silently
        // replaced another would place the wrong mesh everywhere it is used.
        let mut taken = HashSet::new();
        assert_eq!(unique_prim_name("SM_Rock", &mut taken), "SM_Rock");
        assert_eq!(unique_prim_name("SM_Rock", &mut taken), "SM_Rock_1");
        assert_eq!(unique_prim_name("SM_Rock", &mut taken), "SM_Rock_2");
        assert_eq!(unique_prim_name("SM-Rock", &mut taken), "SM_Rock_3");
    }

    #[test]
    fn a_placement_references_the_prototype_rather_than_copying_it() {
        let mut usd = String::new();
        write_instance(&mut usd, 7, "SM_Rock", &IDENTITY);
        assert!(usd.contains("def Xform \"inst_7\""));
        assert!(usd.contains("instanceable = true"));
        assert!(usd.contains("references = </World/Prototypes/SM_Rock>"));
        // Geometry must never appear beside a placement.
        assert!(!usd.contains("points"));
    }

    #[test]
    fn a_placement_is_written_as_unreals_own_matrix() {
        // USD and Unreal agree on layout — row-major, translation last — so a
        // transposed write would be silently wrong rather than rejected.
        let mut usd = String::new();
        write_instance(
            &mut usd,
            0,
            "SM_Rock",
            &compose([100.0, -200.0, 50.0], [0.0; 3], [1.0; 3]),
        );
        assert!(
            usd.contains("(100, -200, 50, 1)"),
            "the translation must be the last row: {usd}"
        );
    }

    #[test]
    fn a_mirrored_placement_needs_no_special_case() {
        // The reason for USD over a quaternion-and-one-scale format: a
        // reflection is just a matrix, so no placement needs baked geometry and
        // the instancing survives.
        let mut usd = String::new();
        write_instance(
            &mut usd,
            0,
            "SM_Rock",
            &compose([0.0; 3], [0.0; 3], [-1.3, 1.3, 1.3]),
        );
        assert!(usd.contains("(-1.3, 0, 0, 0)"), "{usd}");
        assert!(usd.contains("instanceable = true"));
    }

    #[test]
    fn an_identity_placement_writes_the_identity() {
        let mut usd = String::new();
        write_instance(&mut usd, 0, "SM_Rock", &IDENTITY);
        assert!(usd.contains("( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )"));
    }
}

#[cfg(test)]
mod real_data_tests {
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
            if let Ok(document) = load_chimp_document(&world, cell) {
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
}

#[cfg(test)]
mod census {
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
            let decoded = load_chimp_document(&world, package)
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
            let mut usd = String::new();
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
            if let Ok(document) = load_chimp_document(world, cell) {
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
}

#[cfg(test)]
mod sample_export {
    use super::super::level::read_cell_into;
    use super::*;

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
            if let Ok(document) = load_chimp_document(&world, cell) {
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
}
