//! Writing a level as raw geometry for Blender to assemble into `.blend` files.
//! It owns the sidecar format and the script emitted beside it; segmentation
//! belongs to [`super::level_segment`], and USD to [`super::level_export`].
//!
//! Baboon cannot write a `.blend`. The format is a dump of Blender's internal
//! structs — pointer-based, described by its own embedded SDNA, and re-laid-out
//! between releases. Reading one is possible; writing one that a given Blender
//! will open is a version-locked commitment no exporter should make. So the
//! `.blend` files are built *by Blender*, from a sidecar this module writes and
//! a script it emits alongside.
//!
//! The sidecar is raw little-endian arrays because that is the entire point: a
//! position costs 12 bytes here against roughly 24 characters as USD text, which
//! is the difference between 3.8 GB and 8.6 GB for C10. Blender's `foreach_set`
//! consumes flat buffers directly, so the script does no per-vertex work in
//! Python.
//!
//! Geometry is stored in the convention the destination wants rather than the
//! one Unreal uses — triangles wound counter-clockwise and V running up the
//! image — so the script stays a reader rather than a converter.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use blam_tags::iostore::static_mesh::StaticMesh;

use super::level::WorldMatrix;

/// `BABOONLV`, then a version that is checked rather than assumed.
const MAGIC: &[u8; 8] = b"BABOONLV";
const VERSION: u32 = 1;
/// Where the segment count sits, so it can be patched once the split is known.
const SEGMENT_COUNT_AT: u64 = 20;
/// A placement: mesh index, segment index, then a 4x4 of doubles.
const PLACEMENT_SIZE: usize = 4 + 4 + 16 * 8;

/// The script that turns the sidecar into `.blend` files, emitted beside it so
/// the two are always the pair that were written together.
const BUILD_SCRIPT: &str = include_root_str!("src/app/chimp/level_blend.py");

/// What a Blender export produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct BlendExportReport {
    pub(in crate::app) meshes: usize,
    pub(in crate::app) placements: usize,
    pub(in crate::app) segments: usize,
    pub(in crate::app) unreadable_meshes: usize,
    pub(in crate::app) dropped_placements: usize,
    pub(in crate::app) data_bytes: u64,
}

/// One mesh, ready to be written.
pub(in crate::app) struct BlendMesh<'a> {
    pub(in crate::app) name: &'a str,
    pub(in crate::app) mesh: &'a StaticMesh,
}

/// One placement: which mesh, which segment, and where.
pub(in crate::app) struct BlendPlacement {
    pub(in crate::app) mesh: u32,
    pub(in crate::app) segment: u32,
    pub(in crate::app) world: WorldMatrix,
}

/// Write the header and mesh table. Meshes stream in one at a time.
pub(in crate::app) struct BlendWriter {
    out: BufWriter<File>,
    meshes: usize,
}

impl BlendWriter {
    /// Begin a sidecar. `meshes`, `placements` and `segments` are declared up
    /// front so the reader can allocate once and check as it goes rather than
    /// discovering the shape of the file while parsing it.
    /// The segment count is only known once every mesh has been decoded, since
    /// the split is budgeted on geometry — so it is written as zero here and
    /// patched by [`BlendWriter::finish`].
    pub(in crate::app) fn create(
        path: &Path,
        meshes: usize,
        placements: usize,
    ) -> std::io::Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(MAGIC)?;
        out.write_all(&VERSION.to_le_bytes())?;
        out.write_all(&(meshes as u32).to_le_bytes())?;
        out.write_all(&(placements as u32).to_le_bytes())?;
        out.write_all(&0u32.to_le_bytes())?;
        Ok(Self { out, meshes: 0 })
    }

    /// Append one mesh: its name, then its arrays back to back.
    ///
    /// Triangles are wound counter-clockwise here even though Unreal winds them
    /// clockwise. Blender treats counter-clockwise as front-facing, and a mesh
    /// handed over the other way imports with every surface inside out — which
    /// reads as broken normals rather than as a winding problem.
    pub(in crate::app) fn write_mesh(&mut self, mesh: BlendMesh<'_>) -> std::io::Result<()> {
        let BlendMesh { name, mesh } = mesh;
        let vertices = mesh.vertices.len();
        let triangles = mesh.indices.len() / 3;
        self.out.write_all(&(name.len() as u32).to_le_bytes())?;
        self.out.write_all(name.as_bytes())?;
        self.out.write_all(&(vertices as u32).to_le_bytes())?;
        self.out.write_all(&(triangles as u32).to_le_bytes())?;

        let mut buffer: Vec<u8> = Vec::with_capacity(vertices * 12);
        for vertex in &mesh.vertices {
            for value in vertex.position {
                buffer.extend_from_slice(&value.to_le_bytes());
            }
        }
        self.out.write_all(&buffer)?;

        buffer.clear();
        for vertex in &mesh.vertices {
            for value in vertex.normal {
                buffer.extend_from_slice(&value.to_le_bytes());
            }
        }
        self.out.write_all(&buffer)?;

        buffer.clear();
        for vertex in &mesh.vertices {
            // Unreal's V runs down the image; Blender's runs up, as USD's does.
            buffer.extend_from_slice(&vertex.uv[0].to_le_bytes());
            buffer.extend_from_slice(&(1.0 - vertex.uv[1]).to_le_bytes());
        }
        self.out.write_all(&buffer)?;

        buffer.clear();
        for triangle in mesh.indices.chunks_exact(3) {
            for index in [triangle[0], triangle[2], triangle[1]] {
                buffer.extend_from_slice(&index.to_le_bytes());
            }
        }
        self.out.write_all(&buffer)?;
        self.meshes += 1;
        Ok(())
    }

    /// Append every placement, then finish the file.
    ///
    /// The matrix is written exactly as Unreal holds it — row-major with the
    /// translation last — and transposed on the Blender side, which takes
    /// column vectors. Doing it there keeps one convention in the file and one
    /// conversion in the reader.
    pub(in crate::app) fn finish(
        mut self,
        placements: &[BlendPlacement],
        segments: usize,
    ) -> std::io::Result<u64> {
        let mut buffer: Vec<u8> = Vec::with_capacity(placements.len() * PLACEMENT_SIZE);
        for placement in placements {
            buffer.extend_from_slice(&placement.mesh.to_le_bytes());
            buffer.extend_from_slice(&placement.segment.to_le_bytes());
            for value in placement.world {
                buffer.extend_from_slice(&value.to_le_bytes());
            }
        }
        self.out.write_all(&buffer)?;
        // A `BufWriter` that failed once keeps failing, so this catches a disk
        // filling up part-way through several gigabytes of geometry.
        self.out.flush()?;

        let mut file = self
            .out
            .into_inner()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        file.seek(SeekFrom::Start(SEGMENT_COUNT_AT))?;
        file.write_all(&(segments as u32).to_le_bytes())?;
        file.sync_all()?;
        file.seek(SeekFrom::End(0))
    }
}

/// Write the build script and a note beside the data.
///
/// Emitted rather than installed: the script and the file it reads are written
/// together and stay a matched pair, where an addon would have to keep working
/// against every version of the format anyone still has on disk.
pub(in crate::app) fn write_build_script(
    directory: &Path,
    name: &str,
    report: &BlendExportReport,
) -> std::io::Result<()> {
    std::fs::write(directory.join("build_blend.py"), BUILD_SCRIPT)?;
    let mut readme = BufWriter::new(File::create(directory.join(format!("{name}_README.txt")))?);
    let _ = writeln!(readme, "{name} - exported from Baboon for Blender");
    let _ = writeln!(readme);
    let _ = writeln!(
        readme,
        "  {name}.baboonlevel   {} meshes and {} placements, as raw geometry\n\
         \x20 build_blend.py     the script that turns it into .blend files",
        report.meshes, report.placements
    );
    let _ = writeln!(readme);
    let _ = writeln!(readme, "WHY THERE IS A SCRIPT");
    let _ = writeln!(
        readme,
        "  Baboon cannot write .blend files - the format is a dump of Blender's\n\
         \x20 own internal structures and changes between releases. So Blender\n\
         \x20 builds them, from the geometry in the .baboonlevel file."
    );
    let _ = writeln!(readme);
    let _ = writeln!(readme, "HOW TO RUN IT");
    let _ = writeln!(
        readme,
        "  Open Blender, go to the Scripting tab, open build_blend.py and press\n\
         \x20 Run. It reads the .baboonlevel file sitting next to it.\n\
         \x20 From a terminal instead:\n\
         \x20     blender --background --python build_blend.py"
    );
    let _ = writeln!(readme);
    let _ = writeln!(readme, "WHAT YOU GET");
    let _ = writeln!(
        readme,
        "  meshes/<name>.blend   one file per mesh, holding only that geometry\n\
         \x20 {name}_master_NN.blend  one per region, linking the meshes above and\n\
         \x20                     placing them in world space\n\
         \x20\n\
         \x20 The masters link rather than copy, so a mesh placed a thousand times\n\
         \x20 is stored once and every placement of it is a linked duplicate. Keep\n\
         \x20 the meshes folder beside the masters - a master without it opens\n\
         \x20 with the placements present and the geometry missing."
    );
    if report.segments > 1 {
        let _ = writeln!(readme);
        let _ = writeln!(
            readme,
            "  This level is split across {} masters. The whole of it does not\n\
             \x20 open at once: {} placements is past what Blender takes in one\n\
             \x20 scene. The masters share a coordinate system, so opening two\n\
             \x20 lines them up exactly.",
            report.segments, report.placements
        );
    }
    readme.flush()
}

#[cfg(test)]
mod tests;
