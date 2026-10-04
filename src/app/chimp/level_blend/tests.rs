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
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        VERSION
    );
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
    let f32_at =
        |bytes: &[u8], at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());

    // Positions come through untouched.
    assert_eq!(f32_at(&bytes, at + 12), 1.0);
    at += 3 * 3 * 4;
    // Normals too.
    assert_eq!(f32_at(&bytes, at + 8), 1.0);
    at += 3 * 3 * 4;
    // V is flipped: Unreal's runs down the image, Blender's runs up.
    assert_eq!(f32_at(&bytes, at + 4), 1.0);
    assert_eq!(
        f32_at(&bytes, at + 12),
        0.75,
        "0.25 must arrive as 1 - 0.25"
    );
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
    assert_eq!(
        u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap()),
        0
    );
    assert_eq!(
        u32::from_le_bytes(bytes[start + 4..start + 8].try_into().unwrap()),
        2,
        "the segment a placement belongs to must survive"
    );
    let translation_at = start + 8 + 12 * 8;
    assert_eq!(
        f64::from_le_bytes(
            bytes[translation_at..translation_at + 8]
                .try_into()
                .unwrap()
        ),
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
    assert!(
        script.contains("BABOONLV"),
        "the script must read this format"
    );
    let readme = std::fs::read_to_string(directory.join("c10_README.txt")).unwrap();
    // The two things a user can get wrong.
    assert!(readme.contains("the meshes folder beside the masters"));
    assert!(readme.contains("split across 3 masters"));
    let _ = std::fs::remove_dir_all(&directory);
}
