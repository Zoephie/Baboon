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
const BUILD_SCRIPT: &str = include_str!("level_blend.py");

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
mod tests {
    use super::*;
    use blam_tags::iostore::static_mesh::StaticVertex;

    fn vertex(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> StaticVertex {
        StaticVertex {
            position,
            normal,
            uv,
        }
    }

    fn one_triangle() -> StaticMesh {
        StaticMesh {
            indices: vec![0, 1, 2],
            vertices: vec![
                vertex([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0]),
                vertex([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.25]),
                vertex([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0]),
            ],
        }
    }

    fn write_sample(path: &Path) -> Vec<u8> {
        let mesh = one_triangle();
        let mut writer = BlendWriter::create(path, 1, 1).unwrap();
        writer
            .write_mesh(BlendMesh {
                name: "SM_Rock",
                mesh: &mesh,
            })
            .unwrap();
        writer
            .finish(
                &[BlendPlacement {
                    mesh: 0,
                    segment: 0,
                    world: super::super::level::IDENTITY,
                }],
                1,
            )
            .unwrap();
        std::fs::read(path).unwrap()
    }

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn the_file_announces_what_it_is_and_what_it_holds() {
        let path = temp("baboon-blend-header.baboonlevel");
        let bytes = write_sample(&path);
        assert_eq!(&bytes[0..8], MAGIC);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), VERSION);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn geometry_is_written_in_the_convention_blender_reads() {
        let path = temp("baboon-blend-convention.baboonlevel");
        let bytes = write_sample(&path);
        // header 24, name length 4, name 7, counts 8
        let mut at = 24 + 4 + 7 + 8;
        let f32_at = |bytes: &[u8], at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());

        // Positions come through untouched.
        assert_eq!(f32_at(&bytes, at + 12), 1.0);
        at += 3 * 3 * 4;
        // Normals too.
        assert_eq!(f32_at(&bytes, at + 8), 1.0);
        at += 3 * 3 * 4;
        // V is flipped: Unreal's runs down the image, Blender's runs up.
        assert_eq!(f32_at(&bytes, at + 4), 1.0);
        assert_eq!(f32_at(&bytes, at + 12), 0.75, "0.25 must arrive as 1 - 0.25");
        at += 3 * 2 * 4;
        // Winding is reversed: Unreal is clockwise, Blender wants the other way.
        let index_at =
            |bytes: &[u8], at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(index_at(&bytes, at), 0);
        assert_eq!(index_at(&bytes, at + 4), 2);
        assert_eq!(index_at(&bytes, at + 8), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_placement_carries_its_matrix_and_its_segment() {
        let path = temp("baboon-blend-placement.baboonlevel");
        let mesh = one_triangle();
        let mut writer = BlendWriter::create(&path, 1, 2).unwrap();
        writer
            .write_mesh(BlendMesh {
                name: "M",
                mesh: &mesh,
            })
            .unwrap();
        let mut world = super::super::level::IDENTITY;
        world[12] = -1234.5;
        writer
            .finish(
                &[
                    BlendPlacement {
                        mesh: 0,
                        segment: 2,
                        world,
                    },
                    BlendPlacement {
                        mesh: 0,
                        segment: 0,
                        world: super::super::level::IDENTITY,
                    },
                ],
                3,
            )
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        // The segment count is patched in after the split is known.
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 3);
        // Two placements of 136 bytes each, at the very end.
        let start = bytes.len() - 2 * PLACEMENT_SIZE;
        assert_eq!(u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap()), 0);
        assert_eq!(
            u32::from_le_bytes(bytes[start + 4..start + 8].try_into().unwrap()),
            2,
            "the segment a placement belongs to must survive"
        );
        let translation_at = start + 8 + 12 * 8;
        assert_eq!(
            f64::from_le_bytes(bytes[translation_at..translation_at + 8].try_into().unwrap()),
            -1234.5,
            "the matrix is written row-major, translation last, as Unreal holds it"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_mesh_costs_its_arrays_and_nothing_else() {
        // The reason for a binary sidecar at all: a position is 12 bytes here
        // against roughly 24 characters of text.
        let path = temp("baboon-blend-size.baboonlevel");
        let bytes = write_sample(&path);
        let header = 24 + 4 + 7 + 8;
        let geometry = 3 * (3 * 4 + 3 * 4 + 2 * 4) + 3 * 4;
        assert_eq!(bytes.len(), header + geometry + 136);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_script_is_written_beside_the_data() {
        let directory = temp("baboon-blend-script");
        std::fs::create_dir_all(&directory).unwrap();
        write_build_script(
            &directory,
            "c10",
            &BlendExportReport {
                meshes: 2,
                placements: 5,
                segments: 3,
                ..BlendExportReport::default()
            },
        )
        .unwrap();
        let script = std::fs::read_to_string(directory.join("build_blend.py")).unwrap();
        assert!(script.contains("BABOONLV"), "the script must read this format");
        let readme = std::fs::read_to_string(directory.join("c10_README.txt")).unwrap();
        // The two things a user can get wrong.
        assert!(readme.contains("the meshes folder beside the masters"));
        assert!(readme.contains("split across 3 masters"));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
