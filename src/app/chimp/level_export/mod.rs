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
    document: &ChimpPackage,
    detail: MeshDetail,
) -> Result<StaticMesh, String> {
    decode_mesh_raw(world, document, detail).map(|mesh| weld(&mesh).0)
}

fn decode_mesh_raw(
    world: &World,
    document: &ChimpPackage,
    detail: MeshDetail,
) -> Result<StaticMesh, String> {
    let header_size = document.header.summary.header_size as usize;
    match detail {
        MeshDetail::Fallback => StaticMesh::from_package(&document.bytes, header_size),
        MeshDetail::Nanite => {
            // Nanite geometry is streamed from the package's bulk data, so the
            // decoder needs it alongside the package itself.
            let archive = &world.archives()[document.provider.container];
            let bulk = archive
                .chunk_index_for(&document.provider.entry_path)
                .ok()
                .and_then(|chunk| archive.read_bulk_for(chunk, 0).ok());
            StaticMesh::from_package_preferring_nanite(
                &document.bytes,
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

// The single-file USD writer, superseded by the segmented and Blender
// exports. Nothing in the app calls it; its tests still use it as a reference,
// so it is compiled for them only.
#[cfg(test)]
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

// The single-file USD writer, superseded by the segmented and Blender
// exports. Nothing in the app calls it; its tests still use it as a reference,
// so it is compiled for them only.
#[cfg(test)]
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
    let _ = writeln!(
        usd,
        "                prepend apiSchemas = [\"MaterialBindingAPI\"]"
    );
    let _ = writeln!(usd, "                )");
    let _ = writeln!(usd, "            {{");
    let _ = writeln!(
        usd,
        "                rel material:binding = {material_path}"
    );
    let _ = writeln!(
        usd,
        "                uniform token subdivisionScheme = \"none\""
    );
    // Unreal winds its triangles clockwise; USD reads counter-clockwise as
    // front-facing unless told otherwise, which imports every surface
    // back-facing and reads as broken normals. The single-mesh exporters
    // sidestep this by negating Y to mirror into right-handed space, but a
    // level cannot: mirroring the geometry would mean mirroring every world
    // placement with it. Declaring the convention says the same thing and
    // leaves the world coordinates alone.
    let _ = writeln!(
        usd,
        "                uniform token orientation = \"leftHanded\""
    );

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
        let _ = write!(usd, "({}, {})", number_f32(u), number_f32(1.0 - v));
    }
    let _ = writeln!(usd, "] (");
    let _ = writeln!(usd, "                    interpolation = \"vertex\"");
    let _ = writeln!(usd, "                )");
    let _ = writeln!(usd, "            }}");
}

// The single-file USD writer, superseded by the segmented and Blender
// exports. Nothing in the app calls it; its tests still use it as a reference,
// so it is compiled for them only.
#[cfg(test)]
fn write_instance(
    usd: &mut impl std::io::Write,
    index: usize,
    prototype: &str,
    world: &WorldMatrix,
) {
    write_instance_referencing(
        usd,
        index,
        &format!("</World/Prototypes/{prototype}>"),
        world,
    );
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

/// Which part of a write is running, for callers that show progress.
///
/// Named here rather than taking the UI's phase type, so the exporter does not
/// depend on the thing displaying it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ExportStage {
    Meshes,
    Segments,
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
    progress: &dyn Fn(ExportStage, usize, usize),
) -> std::io::Result<SegmentedExportReport> {
    std::fs::create_dir_all(directory)?;
    let mut report = SegmentedExportReport::default();
    let library_name = format!("{name}_prototypes.usda");

    let mut materials = HashSet::new();
    let library_path = directory.join(&library_name);
    let mut library = BufWriter::new(File::create(&library_path)?);
    write_stage_header(&mut library);
    write_prototypes_open(&mut library);
    let mut taken = HashSet::new();
    let pass = level_pass(
        scene,
        budget,
        progress,
        |package| load_prototype(world, package, detail, &mut taken),
        |prototype| {
            materials.insert(prototype.material.clone());
            report.triangles += prototype.mesh.indices.len() / 3;
            write_library_prototype(&mut library, &prototype);
            Ok(prototype.prim)
        },
    )?;
    let _ = writeln!(library, "    }}");
    let _ = writeln!(library, "}}");
    library.flush()?;
    drop(library);
    report.library_bytes = std::fs::metadata(&library_path)?.len();
    let LevelPass {
        meshes: prototypes,
        unreadable,
        placed,
        segments,
    } = pass;
    report.unreadable_meshes = unreadable;
    report.prototypes = prototypes.iter().flatten().count();
    report.materials = materials.len();
    report.dropped_placements = scene.placements.len() - placed.len();
    report.segments = segments.len();
    for (number, piece) in segments.iter().enumerate() {
        progress(ExportStage::Segments, number, segments.len());
        if piece.over_budget {
            report.over_budget += 1;
        }
        let path = directory.join(format!("{name}_seg_{number:02}.usda"));
        let mut usd = BufWriter::new(File::create(&path)?);
        write_stage_header(&mut usd);
        for (slot, &index) in piece.placements.iter().enumerate() {
            let placement = &scene.placements[placed[index]];
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
    progress: &dyn Fn(ExportStage, usize, usize),
) -> std::io::Result<BlendExportReport> {
    std::fs::create_dir_all(directory)?;
    let mut report = BlendExportReport::default();

    let data_path = directory.join(format!("{name}.baboonlevel"));
    let mut writer = BlendWriter::create(&data_path, scene.meshes.len(), scene.placements.len())?;
    let mut taken = HashSet::new();
    let LevelPass {
        meshes: written,
        unreadable,
        placed,
        segments,
    } = level_pass(
        scene,
        budget,
        progress,
        |package| load_prototype(world, package, detail, &mut taken),
        |prototype| {
            writer.write_mesh(BlendMesh {
                name: &prototype.prim,
                mesh: &prototype.mesh,
            })?;
            report.meshes += 1;
            Ok(report.meshes as u32 - 1)
        },
    )?;
    report.unreadable_meshes = unreadable;
    report.dropped_placements = scene.placements.len() - placed.len();
    report.segments = segments.len();
    let mut placements = Vec::with_capacity(placed.len());
    for (number, piece) in segments.iter().enumerate() {
        for &index in &piece.placements {
            let placement = &scene.placements[placed[index]];
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

/// What both level exports share: every mesh decoded once and handed to
/// `write`, then the placements whose meshes decoded, split into segments.
struct LevelPass<T> {
    /// What `write` returned for each mesh, `None` where it did not decode.
    meshes: Vec<Option<T>>,
    unreadable: usize,
    /// The placements kept, as indices into `LevelScene::placements`. A
    /// segment's `placements` index into this.
    placed: Vec<usize>,
    segments: Vec<Segment>,
}

/// Decode, write and drop the meshes one at a time, keeping only what the
/// split needs; then drop the placements of meshes that did not decode, which
/// have nothing to reference and would skew a segment's budget with geometry
/// that is not there, and split the rest on `budget`.
fn level_pass<T>(
    scene: &LevelScene,
    budget: SegmentBudget,
    progress: &dyn Fn(ExportStage, usize, usize),
    mut load: impl FnMut(&str) -> Option<Prototype>,
    mut write: impl FnMut(Prototype) -> std::io::Result<T>,
) -> std::io::Result<LevelPass<T>> {
    let mut meshes = Vec::with_capacity(scene.meshes.len());
    let mut mesh_triangles = Vec::with_capacity(scene.meshes.len());
    let mut unreadable = 0;
    for (index, package) in scene.meshes.iter().enumerate() {
        progress(ExportStage::Meshes, index, scene.meshes.len());
        match load(package) {
            Some(prototype) => {
                mesh_triangles.push(prototype.mesh.indices.len() / 3);
                meshes.push(Some(write(prototype)?));
            }
            None => {
                unreadable += 1;
                mesh_triangles.push(0);
                meshes.push(None);
            }
        }
    }
    let placed: Vec<usize> = scene
        .placements
        .iter()
        .enumerate()
        .filter(|(_, placement)| matches!(meshes.get(placement.mesh), Some(Some(_))))
        .map(|(index, _)| index)
        .collect();
    let positions: Vec<PlacedMesh> = placed
        .iter()
        .map(|&index| {
            let placement = &scene.placements[index];
            PlacedMesh {
                mesh: placement.mesh,
                position: [
                    placement.world[12],
                    placement.world[13],
                    placement.world[14],
                ],
            }
        })
        .collect();
    let segments = segment(&positions, &mesh_triangles, budget);
    Ok(LevelPass {
        meshes,
        unreadable,
        placed,
        segments,
    })
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
    write_mesh_body(usd, mesh, &format!("</World/Prototypes/{prim}/{material}>"));
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

// The single-file USD writer, superseded by the segmented and Blender
// exports. Nothing in the app calls it; its tests still use it as a reference,
// so it is compiled for them only.
#[cfg(test)]
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
    // The package, not an editor document: a document decodes its own mesh
    // preview (Nanite for a static mesh) and renders text panes, and the
    // mesh is decoded again just below.
    let (document, mesh) = load_chimp_package(world, package)
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

// The single-file USD writer, superseded by the segmented and Blender
// exports. Nothing in the app calls it; its tests still use it as a reference,
// so it is compiled for them only.
#[cfg(test)]
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

// The single-file USD writer, superseded by the segmented and Blender
// exports. Nothing in the app calls it; its tests still use it as a reference,
// so it is compiled for them only.
#[cfg(test)]
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
        write_instance(
            &mut usd,
            report.instances,
            &prototype.prim,
            &placement.world,
        );
        report.instances += 1;
    }
    let _ = writeln!(usd, "}}");

    report.prototypes = prototypes.iter().flatten().count();
    report.materials = materials.len();
    // Every byte written above is ASCII, so this cannot fail.
    (
        String::from_utf8(usd).expect("the writer emits ASCII"),
        report,
    )
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod real_data_tests;

#[cfg(test)]
mod segmented_tests;

#[cfg(test)]
mod census_tests;

#[cfg(test)]
mod sample_export_tests;

#[cfg(test)]
mod level_pass_tests;
