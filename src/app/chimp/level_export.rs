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

use std::fs::File;
use std::io::{BufWriter, Write as _};

use super::level::{LevelScene, WorldMatrix};
use super::level_blend::{
    BlendExportReport, BlendMesh, BlendPlacement, BlendWriter, write_build_script,
};
use super::level_segment::{PlacedMesh, Segment, SegmentBudget, segment};
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

/// Errors are deliberately not checked per call. A `BufWriter` that has failed
/// once keeps failing, so a single flush at the end catches a disk filling up
/// mid-write; checking eight million individual `write!`s would not learn
/// anything the flush does not.
fn write_mesh(usd: &mut impl std::io::Write, prototype: &Prototype) {
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
    write_mesh_body(usd, mesh, &format!("</World/Materials/{material}>"));
    let _ = writeln!(usd, "        }}");
}

/// The `def Mesh` block itself, bound to whichever material prim the caller
/// names — a scene-wide one in a single file, or the prototype's own nested
/// copy in a shared library.
fn write_mesh_body(usd: &mut impl std::io::Write, mesh: &StaticMesh, material_path: &str) {
    let _ = writeln!(usd, "            def Mesh \"Mesh\" (");
    let _ = writeln!(usd, "                prepend apiSchemas = [\"MaterialBindingAPI\"]");
    let _ = writeln!(usd, "                )");
    let _ = writeln!(usd, "            {{");
    let _ = writeln!(usd, "                rel material:binding = {material_path}");
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
}

fn write_instance(
    usd: &mut impl std::io::Write,
    index: usize,
    prototype: &str,
    world: &WorldMatrix,
) {
    write_instance_referencing(usd, index, &format!("</World/Prototypes/{prototype}>"), world);
}

/// A placement whose reference target is written out in full, so it can name a
/// prototype in this file or one in a shared library file beside it.
fn write_instance_referencing(
    usd: &mut impl std::io::Write,
    index: usize,
    reference: &str,
    world: &WorldMatrix,
) {
    let _ = writeln!(usd, "    def Xform \"inst_{index}\" (");
    // The mesh is not copied here: this prim references the one prototype and
    // is marked instanceable, so every placement of a mesh shares its geometry.
    let _ = writeln!(usd, "        instanceable = true");
    let _ = writeln!(usd, "        prepend references = {reference}");
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

/// What a segmented export produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct SegmentedExportReport {
    pub(in crate::app) segments: usize,
    /// Segments that broke a budget no further split could fix — a single mesh
    /// bigger than the whole allowance, or placements sharing one position.
    pub(in crate::app) over_budget: usize,
    pub(in crate::app) prototypes: usize,
    pub(in crate::app) instances: usize,
    pub(in crate::app) materials: usize,
    pub(in crate::app) triangles: usize,
    pub(in crate::app) unreadable_meshes: usize,
    pub(in crate::app) dropped_placements: usize,
    pub(in crate::app) library_bytes: u64,
    pub(in crate::app) segment_bytes: u64,
}

/// Export a level as one shared prototype library plus a segment per region.
///
/// A whole level does not import — 109.7 million triangles across 296,399
/// placements is past what Blender opens — so the placements are divided
/// spatially until each piece fits a budget, and each piece becomes its own
/// file. Geometry is written once into a library the segments reference, so
/// splitting the level does not multiply its geometry: a rock used across the
/// map is stored once however many segments place it.
///
/// The library is not optional and not a cache. A segment holds only
/// placements, so a segment without its library beside it imports as nothing at
/// all — which is why [`write_segment_readme`] says so in the export folder.
pub(in crate::app) fn write_segmented_usd(
    world: &World,
    scene: &LevelScene,
    detail: MeshDetail,
    directory: &Path,
    name: &str,
    budget: SegmentBudget,
) -> std::io::Result<SegmentedExportReport> {
    std::fs::create_dir_all(directory)?;
    let mut report = SegmentedExportReport::default();
    let library_name = format!("{name}_prototypes.usda");

    // One pass over the meshes: decode, write, drop, and keep only the triangle
    // count the split needs. Nothing holds more than a mesh at a time.
    let mut taken = HashSet::new();
    let mut prototypes: Vec<Option<String>> = Vec::with_capacity(scene.meshes.len());
    let mut mesh_triangles: Vec<usize> = Vec::with_capacity(scene.meshes.len());
    let mut materials = HashSet::new();
    {
        let library_path = directory.join(&library_name);
        let mut library = BufWriter::new(File::create(&library_path)?);
        write_stage_header(&mut library);
        write_prototypes_open(&mut library);
        for package in &scene.meshes {
            match load_prototype(world, package, detail, &mut taken) {
                Some(prototype) => {
                    let triangles = prototype.mesh.indices.len() / 3;
                    materials.insert(prototype.material.clone());
                    write_library_prototype(&mut library, &prototype);
                    prototypes.push(Some(prototype.prim));
                    mesh_triangles.push(triangles);
                    report.triangles += triangles;
                }
                None => {
                    report.unreadable_meshes += 1;
                    prototypes.push(None);
                    mesh_triangles.push(0);
                }
            }
        }
        let _ = writeln!(library, "    }}");
        let _ = writeln!(library, "}}");
        library.flush()?;
        drop(library);
        report.library_bytes = std::fs::metadata(&library_path)?.len();
    }
    report.prototypes = prototypes.iter().flatten().count();
    report.materials = materials.len();

    // A placement whose mesh could not be decoded has nothing to reference, so
    // it is dropped before the split rather than skewing a segment's budget with
    // geometry that will not be there.
    let placed: Vec<(usize, PlacedMesh)> = scene
        .placements
        .iter()
        .enumerate()
        .filter(|(_, placement)| {
            matches!(prototypes.get(placement.mesh), Some(Some(_)))
        })
        .map(|(index, placement)| {
            (
                index,
                PlacedMesh {
                    mesh: placement.mesh,
                    position: [
                        placement.world[12],
                        placement.world[13],
                        placement.world[14],
                    ],
                },
            )
        })
        .collect();
    report.dropped_placements = scene.placements.len() - placed.len();
    let positions: Vec<PlacedMesh> = placed.iter().map(|(_, placed)| *placed).collect();

    let segments = segment(&positions, &mesh_triangles, budget);
    report.segments = segments.len();
    for (number, piece) in segments.iter().enumerate() {
        if piece.over_budget {
            report.over_budget += 1;
        }
        let path = directory.join(format!("{name}_seg_{number:02}.usda"));
        let mut usd = BufWriter::new(File::create(&path)?);
        write_stage_header(&mut usd);
        for (slot, &index) in piece.placements.iter().enumerate() {
            let (placement_index, _) = placed[index];
            let placement = &scene.placements[placement_index];
            let Some(Some(prim)) = prototypes.get(placement.mesh) else {
                continue;
            };
            write_instance_referencing(
                &mut usd,
                slot,
                &format!("@./{library_name}@</World/Prototypes/{prim}>"),
                &placement.world,
            );
            report.instances += 1;
        }
        let _ = writeln!(usd, "}}");
        usd.flush()?;
        drop(usd);
        report.segment_bytes += std::fs::metadata(&path)?.len();
    }

    write_segment_readme(directory, name, &library_name, &segments, budget, &report)?;
    Ok(report)
}

/// Export a level as raw geometry for Blender to assemble into `.blend` files.
///
/// Segmented on the same budgets as the USD path and for the same reason: the
/// master scene has to place the level's objects, and 296,399 of them is past
/// what Blender takes. One master per region, each linking the per-mesh files
/// rather than copying them.
pub(in crate::app) fn write_blend_export(
    world: &World,
    scene: &LevelScene,
    detail: MeshDetail,
    directory: &Path,
    name: &str,
    budget: SegmentBudget,
) -> std::io::Result<BlendExportReport> {
    std::fs::create_dir_all(directory)?;
    let mut report = BlendExportReport::default();

    // Meshes are decoded, written and dropped one at a time; only the triangle
    // counts the split needs are kept.
    let mut taken = HashSet::new();
    let mut written: Vec<Option<u32>> = Vec::with_capacity(scene.meshes.len());
    let mut mesh_triangles: Vec<usize> = Vec::with_capacity(scene.meshes.len());
    let data_path = directory.join(format!("{name}.baboonlevel"));
    let mut writer = BlendWriter::create(&data_path, scene.meshes.len(), scene.placements.len())?;
    for package in &scene.meshes {
        match load_prototype(world, package, detail, &mut taken) {
            Some(prototype) => {
                writer.write_mesh(BlendMesh {
                    name: &prototype.prim,
                    mesh: &prototype.mesh,
                })?;
                written.push(Some(report.meshes as u32));
                mesh_triangles.push(prototype.mesh.indices.len() / 3);
                report.meshes += 1;
            }
            None => {
                report.unreadable_meshes += 1;
                written.push(None);
                mesh_triangles.push(0);
            }
        }
    }

    // A placement whose mesh could not be decoded has nothing to place.
    let placed: Vec<(usize, PlacedMesh)> = scene
        .placements
        .iter()
        .enumerate()
        .filter(|(_, placement)| matches!(written.get(placement.mesh), Some(Some(_))))
        .map(|(index, placement)| {
            (
                index,
                PlacedMesh {
                    mesh: placement.mesh,
                    position: [
                        placement.world[12],
                        placement.world[13],
                        placement.world[14],
                    ],
                },
            )
        })
        .collect();
    report.dropped_placements = scene.placements.len() - placed.len();
    let positions: Vec<PlacedMesh> = placed.iter().map(|(_, placed)| *placed).collect();

    let segments = segment(&positions, &mesh_triangles, budget);
    report.segments = segments.len();
    let mut placements = Vec::with_capacity(placed.len());
    for (number, piece) in segments.iter().enumerate() {
        for &index in &piece.placements {
            let (placement_index, _) = placed[index];
            let placement = &scene.placements[placement_index];
            let Some(Some(mesh)) = written.get(placement.mesh) else {
                continue;
            };
            placements.push(BlendPlacement {
                mesh: *mesh,
                segment: number as u32,
                world: placement.world,
            });
        }
    }
    report.placements = placements.len();
    report.data_bytes = writer.finish(&placements, segments.len())?;

    write_build_script(directory, name, &report)?;
    Ok(report)
}

/// One prototype in the shared library, with its material nested inside it.
///
/// The material lives *within* the prototype rather than in a scene-wide scope
/// because a reference only remaps paths inside the subtree it pulls in. A
/// binding pointing at `/World/Materials/X` would still say that after being
/// referenced into a segment, where no such prim exists, and every surface would
/// import unbound. Nesting costs a duplicated stub per prototype and makes the
/// reference carry everything it needs.
fn write_library_prototype(usd: &mut impl std::io::Write, prototype: &Prototype) {
    let Prototype {
        prim,
        mesh,
        material,
    } = prototype;
    let _ = writeln!(usd, "        def Xform \"{prim}\"");
    let _ = writeln!(usd, "        {{");
    let _ = writeln!(usd, "            def Material \"{material}\"");
    let _ = writeln!(usd, "            {{");
    let _ = writeln!(
        usd,
        "                token outputs:surface.connect = </World/Prototypes/{prim}/{material}/Surface.outputs:surface>"
    );
    let _ = writeln!(usd, "                def Shader \"Surface\"");
    let _ = writeln!(usd, "                {{");
    let _ = writeln!(
        usd,
        "                    uniform token info:id = \"UsdPreviewSurface\""
    );
    let _ = writeln!(usd, "                    token outputs:surface");
    let _ = writeln!(usd, "                }}");
    let _ = writeln!(usd, "            }}");
    write_mesh_body(
        usd,
        mesh,
        &format!("</World/Prototypes/{prim}/{material}>"),
    );
    let _ = writeln!(usd, "        }}");
}

/// Explain the split in the folder it produced, because a directory of segments
/// and a library is not self-evident and the library is easy to leave behind.
fn write_segment_readme(
    directory: &Path,
    name: &str,
    library_name: &str,
    segments: &[Segment],
    budget: SegmentBudget,
    report: &SegmentedExportReport,
) -> std::io::Result<()> {
    let mut readme = BufWriter::new(File::create(directory.join(format!("{name}_README.txt")))?);
    let _ = writeln!(readme, "{name} - exported from Baboon as segmented USD");
    let _ = writeln!(readme);
    let _ = writeln!(
        readme,
        "This level is {} triangles across {} placements, which is more than\n\
         Blender opens in one file. It has been split into {} segments.",
        report.triangles, report.instances, report.segments
    );
    let _ = writeln!(readme);
    let _ = writeln!(readme, "WHAT IS HERE");
    let _ = writeln!(
        readme,
        "  {library_name}\n    Every mesh in the level, stored once. This file contains all the\n\
         \x20   geometry; it places nothing.\n\
         \x20 {name}_seg_NN.usda\n    One region of the level. Holds only placements, each referencing a\n\
         \x20   mesh in the library above."
    );
    let _ = writeln!(readme);
    let _ = writeln!(readme, "HOW TO IMPORT");
    let _ = writeln!(
        readme,
        "  Import any {name}_seg_NN.usda. Keep the library file beside the\n\
         \x20 segments - a segment on its own references geometry that is not there\n\
         \x20 and imports as nothing. Import as many segments as your machine will\n\
         \x20 take; they share a coordinate system, so they line up exactly."
    );
    let _ = writeln!(readme);
    let _ = writeln!(readme, "HOW THE SPLIT WAS CHOSEN");
    let _ = writeln!(
        readme,
        "  Segments are regions of the map, not arbitrary slices. The level is\n\
         \x20 cut in half at the middle of its widest axis, and each half again,\n\
         \x20 until every piece fits within:\n\
         \x20     {} triangles across the distinct meshes it uses\n\
         \x20     {} placements\n\
         \x20 Both limits matter. Geometry and object count run out separately: a\n\
         \x20 stand of foliage can place tens of thousands of copies of a handful\n\
         \x20 of meshes, and a budget counting only triangles would not see it.\n\
         \x20 Dense areas therefore produce more, smaller segments than open ones.",
        budget.triangles, budget.placements
    );
    if report.over_budget > 0 {
        let _ = writeln!(readme);
        let _ = writeln!(
            readme,
            "  {} segment(s) exceed the budget and could not be split further -\n\
             \x20 a single mesh larger than the whole allowance, or placements\n\
             \x20 stacked at one point. They are listed below and may be slow or\n\
             \x20 impossible to open.",
            report.over_budget
        );
    }
    let _ = writeln!(readme);
    let _ = writeln!(readme, "SEGMENTS");
    for (number, piece) in segments.iter().enumerate() {
        let _ = writeln!(
            readme,
            "  {name}_seg_{number:02}.usda  {:>12} triangles  {:>8} placements  {:>4} meshes{}",
            piece.triangles,
            piece.placements.len(),
            piece.meshes.len(),
            if piece.over_budget {
                "  [OVER BUDGET]"
            } else {
                ""
            }
        );
    }
    readme.flush()
}

/// Export a level scene to a `.usda` file without ever holding it in memory.
///
/// A whole level is 8.5 GiB of text over 745 meshes; building that as a `String`
/// needs the document, every decoded mesh, and the spare copy a growing buffer
/// reallocates through, which is more memory than the machines this runs on
/// have. Streaming caps the cost at one mesh at a time.
///
/// The geometry goes to a sidecar file first because a `.usda` names its
/// materials before the meshes that bind them, and the material list is only
/// known once every mesh has been loaded. Writing the meshes aside and splicing
/// them in keeps the document byte-for-byte what the in-memory writer produces,
/// rather than reordering a file that importers have already been tested
/// against.
pub(in crate::app) fn write_scene_usd(
    world: &World,
    scene: &LevelScene,
    detail: MeshDetail,
    path: &Path,
) -> std::io::Result<LevelExportReport> {
    let mut report = LevelExportReport::default();
    let mut taken = HashSet::new();
    let mut materials: Vec<String> = Vec::new();
    // Only the names survive the loop; each mesh is written and dropped.
    let mut prototypes: Vec<Option<(String, String)>> = Vec::with_capacity(scene.meshes.len());

    let geometry_path = path.with_extension("prototypes.tmp");
    {
        let mut geometry = BufWriter::new(File::create(&geometry_path)?);
        for package in &scene.meshes {
            match load_prototype(world, package, detail, &mut taken) {
                Some(prototype) => {
                    if !materials.contains(&prototype.material) {
                        materials.push(prototype.material.clone());
                    }
                    write_mesh(&mut geometry, &prototype);
                    prototypes.push(Some((prototype.prim, prototype.material)));
                }
                None => {
                    report.unreadable_meshes += 1;
                    prototypes.push(None);
                }
            }
        }
        geometry.flush()?;
    }

    let mut usd = BufWriter::new(File::create(path)?);
    write_stage_header(&mut usd);
    write_materials(&mut usd, &materials);
    write_prototypes_open(&mut usd);
    std::io::copy(&mut File::open(&geometry_path)?, &mut usd)?;
    let _ = writeln!(usd, "    }}");
    for placement in &scene.placements {
        let Some(Some((prim, _))) = prototypes.get(placement.mesh) else {
            report.dropped_placements += 1;
            continue;
        };
        write_instance(&mut usd, report.instances, prim, &placement.world);
        report.instances += 1;
    }
    let _ = writeln!(usd, "}}");
    // A `BufWriter` that failed once keeps failing, so this is where a disk
    // filling up two hours into an export is caught.
    usd.flush()?;
    drop(usd);
    let _ = std::fs::remove_file(&geometry_path);

    report.prototypes = prototypes.iter().flatten().count();
    report.materials = materials.len();
    Ok(report)
}

/// One mesh, decoded and named, or `None` if it could not be read.
fn load_prototype(
    world: &World,
    package: &str,
    detail: MeshDetail,
    taken: &mut HashSet<String>,
) -> Option<Prototype> {
    let leaf = package.rsplit('/').next().unwrap_or("mesh");
    let (document, mesh) = load_chimp_document(world, package)
        .and_then(|document| decode_mesh(world, &document, detail).map(|mesh| (document, mesh)))
        .ok()?;
    // The mesh's own material, resolved the same way the material list written
    // into a single-mesh export is.
    let material = prim_name(
        &chimp_material_names(&document.header)
            .into_iter()
            .next()
            .unwrap_or_else(|| leaf.to_owned()),
    );
    Some(Prototype {
        prim: unique_prim_name(leaf, taken),
        mesh,
        material,
    })
}

fn write_stage_header(usd: &mut impl std::io::Write) {
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
}

fn write_materials(usd: &mut impl std::io::Write, materials: &[String]) {
    let _ = writeln!(usd, "    def Scope \"Materials\"");
    let _ = writeln!(usd, "    {{");
    for material in materials {
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
}

/// Geometry lives here once. Nothing draws it directly; the placements
/// reference it, which is what keeps a quarter of a million copies down to the
/// size of the meshes themselves.
fn write_prototypes_open(usd: &mut impl std::io::Write) {
    let _ = writeln!(usd, "    def Scope \"Prototypes\"");
    let _ = writeln!(usd, "    {{");
    // Referenced, not shown: hiding the sources keeps every mesh from also being
    // drawn in a heap at the origin.
    let _ = writeln!(usd, "        uniform token visibility = \"invisible\"");
}

/// Convert a level scene into a USD (`.usda`) document held in memory.
///
/// Only safe for a slice of a level — see [`write_scene_usd`] for anything whose
/// size is not known to be modest.
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
        match load_prototype(world, package, detail, &mut taken) {
            Some(prototype) => {
                if !materials.contains(&prototype.material) {
                    materials.push(prototype.material.clone());
                }
                prototypes.push(Some(prototype));
            }
            None => {
                report.unreadable_meshes += 1;
                prototypes.push(None);
            }
        }
    }

    let mut usd: Vec<u8> = Vec::new();
    write_stage_header(&mut usd);
    write_materials(&mut usd, &materials);
    write_prototypes_open(&mut usd);
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
    // Every byte written above is ASCII, so this cannot fail.
    (String::from_utf8(usd).expect("the writer emits ASCII"), report)
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

    /// One placement, as the text the exporter writes for it.
    fn instance_text(index: usize, prototype: &str, world: &WorldMatrix) -> String {
        let mut usd: Vec<u8> = Vec::new();
        write_instance(&mut usd, index, prototype, world);
        String::from_utf8(usd).expect("the writer emits ASCII")
    }

    #[test]
    fn a_placement_references_the_prototype_rather_than_copying_it() {
        let usd = instance_text(7, "SM_Rock", &IDENTITY);
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
        let usd = instance_text(
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
        let usd = instance_text(
            0,
            "SM_Rock",
            &compose([0.0; 3], [0.0; 3], [-1.3, 1.3, 1.3]),
        );
        assert!(usd.contains("(-1.3, 0, 0, 0)"), "{usd}");
        assert!(usd.contains("instanceable = true"));
    }

    #[test]
    fn an_identity_placement_writes_the_identity() {
        let usd = instance_text(0, "SM_Rock", &IDENTITY);
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
            if let Ok(document) = load_chimp_document(&world, cell) {
                read_cell_into(&document, &mut scene);
            }
        }
        let (in_memory, memory_report) = scene_to_usd(&world, &scene, MeshDetail::Fallback);

        let path = std::env::temp_dir().join("baboon-streaming-parity.usda");
        let stream_report =
            write_scene_usd(&world, &scene, MeshDetail::Fallback, &path).expect("stream the export");
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
}

#[cfg(test)]
mod segmented_tests {
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
            if let Ok(document) = load_chimp_document(&world, cell) {
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
            assert!(!text.contains("def Mesh "), "segment {number} carries geometry");
            assert!(text.starts_with("#usda 1.0"));
            assert!(text.contains("metersPerUnit = 0.01"));
            let referencing = text.matches("@./c10_prototypes.usda@</World/Prototypes/").count();
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
            let Ok(document) = load_chimp_document(&world, cell) else {
                continue;
            };
            read_cell_into(&document, &mut scene);
            // `meshes` only ever grows, and in first-seen order, so everything
            // past the high-water mark is new to this cell.
            while counted < scene.meshes.len() {
                triangles += load_chimp_document(&world, &scene.meshes[counted])
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
        let report =
            write_scene_usd(&world, &scene, detail, &out_path).expect("write the sample");
        let written = std::fs::metadata(&out_path).map(|meta| meta.len()).unwrap_or(0);

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
            if let Ok(document) = load_chimp_document(&world, cell) {
                read_cell_into(&document, &mut scene);
            }
        }
        let report =
            write_segmented_usd(&world, &scene, detail, &directory, "c10", budget).expect("write");
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

        let mut scene = LevelScene::default();
        for cell in cells.iter().step_by(step) {
            if let Ok(document) = load_chimp_document(&world, cell) {
                read_cell_into(&document, &mut scene);
            }
        }
        let report =
            write_blend_export(&world, &scene, detail, &directory, "c10", budget).expect("write");
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
