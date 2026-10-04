use super::*;
use std::path::PathBuf;

fn test_jms_vertex(uv: [f32; 2]) -> blam_tags::jms::JmsVertex {
    blam_tags::jms::JmsVertex {
        position: blam_tags::math::RealPoint3d::default(),
        normal: blam_tags::math::RealVector3d::default(),
        tangent: None,
        binormal: None,
        node_sets: Vec::new(),
        uvs: vec![blam_tags::math::RealPoint2d { x: uv[0], y: uv[1] }],
        color: None,
    }
}

#[test]
fn campaign_evolved_export_copies_chimp_uv_conventions_at_the_part_offset() {
    let mut target = blam_tags::jms::JmsFile {
        vertices: vec![
            test_jms_vertex([0.0, 0.0]),
            test_jms_vertex([0.0, 0.0]),
            test_jms_vertex([0.0, 0.0]),
        ],
        ..Default::default()
    };
    let source = blam_tags::jms::JmsFile {
        vertices: vec![test_jms_vertex([2.25, -0.5]), test_jms_vertex([3.0, 1.5])],
        ..Default::default()
    };

    copy_chimp_jms_uvs(&mut target, 1, &source).unwrap();

    assert_eq!(target.vertices[0].uvs[0].x, 0.0);
    assert_eq!(target.vertices[1].uvs[0].x, 2.25);
    assert_eq!(target.vertices[1].uvs[0].y, -0.5);
    assert_eq!(target.vertices[2].uvs[0].x, 3.0);
    assert_eq!(target.vertices[2].uvs[0].y, 1.5);
}

/// The index narrows a package lookup to entries with its file name, then
/// keeps the linear scan's rule and order. Checked against that scan over
/// paths built to trip it: case, backslashes, a plugin's copy of the same
/// file, a longer name sharing the tail, and a `.ubulk` beside the asset.
#[test]
fn a_package_lookup_through_the_index_finds_what_a_scan_found() {
    let paths = [
        (0, 0, "Meteorite/Content/A/SK_Foo.ubulk"),
        (0, 1, "Meteorite/Plugins/P/Content/A/SK_Foo.uasset"),
        (0, 2, "Meteorite/Content/A/SK_Foo.uasset"),
        (1, 0, "Meteorite\\Content\\B\\sk_foo.uasset"),
        (1, 1, "Meteorite/Content/B/XSK_Foo.uasset"),
        (1, 2, "Meteorite/Content/MeshSync/DA_Foo.uasset"),
    ];
    let index = index_ce_paths(paths.iter().copied());
    let normalized = |path: &str| path.to_ascii_lowercase().replace('\\', "/");
    for package in [
        "/Game/A/SK_Foo",
        "/Game/B/SK_FOO",
        "/Game/SK_Foo",
        "/Game/Nope",
        "/Game/B/Foo",
    ] {
        let (file_name, suffix) = ce_package_file_name_and_suffix(package);
        let indexed = index
            .by_file_name
            .get(&file_name)
            .into_iter()
            .flatten()
            .find(|&&(c, e)| {
                let path = paths.iter().find(|p| (p.0, p.1) == (c, e)).unwrap().2;
                normalized(path).ends_with(&suffix)
            })
            .copied();
        let scanned = paths
            .iter()
            .find(|p| normalized(p.2).ends_with(&suffix))
            .map(|p| (p.0, p.1));
        assert_eq!(indexed, scanned, "{package}");
    }
    assert_eq!(index.mesh_sync.len(), 1);
}

/// The indexed package lookups find exactly the entry the linear scans
/// they replaced did, sampled across a real install.
#[test]
fn indexed_package_lookups_match_a_linear_scan() {
    let paks = crate::test_kits::ce_paks();
    if !paks.is_dir() {
        eprintln!(
            "skipping: Campaign Evolved not present at {}",
            paks.display()
        );
        return;
    }
    let loaded = crate::core::source::load_iostore_container_set(
        paks,
        &TagNameIndex::default(),
        crate::test_kits::definitions(),
    )
    .expect("mount Campaign Evolved");
    let TagSource::IoStoreContainerSet { containers, .. } = &loaded.source else {
        panic!("not a container set");
    };
    // What `ce_read_uasset_by_package` did before the index.
    let scan = |package: &str| {
        let tail = package.to_ascii_lowercase().replace('\\', "/");
        let tail = tail.strip_prefix("/game/").unwrap_or(&tail).to_owned();
        let suffix = format!("/{tail}.uasset");
        containers.iter().find_map(|c| {
            c.archive
                .entries()
                .iter()
                .find(|e| {
                    e.path
                        .to_ascii_lowercase()
                        .replace('\\', "/")
                        .ends_with(&suffix)
                })
                .map(|e| e.path.clone())
        })
    };
    let assets: Vec<String> = containers
        .iter()
        .flat_map(|c| c.archive.entries().iter().map(|e| e.path.clone()))
        .filter(|path| path.to_ascii_lowercase().ends_with(".uasset"))
        .collect();
    let mut compared = 0;
    for path in assets.iter().step_by(500) {
        let Some((_, rest)) = path.split_once("/Content/") else {
            continue;
        };
        let package = format!("/Game/{}", rest.trim_end_matches(".uasset"));
        let found = ce_package_entries(containers, &package)
            .next()
            .map(|(_, e)| e.path.clone());
        assert_eq!(found, scan(&package), "{package}");
        compared += 1;
    }
    assert!(compared > 50, "compared only {compared} packages");
}

/// Runs the exact app CE-preview path against the optional `CE_PAKS`
/// installation. `CE_MODEL` selects the tag path fragment and `CE_HD`
/// enables Nanite detail. Skips when `CE_PAKS` is not configured.
#[test]
fn ce_model_real_path() {
    let Some(paks) = std::env::var_os("CE_PAKS").map(PathBuf::from) else {
        eprintln!("skip: set CE_PAKS to a Campaign Evolved Paks directory");
        return;
    };
    if !paks.exists() {
        eprintln!("skip: CE_PAKS does not exist: {}", paks.display());
        return;
    }
    let defs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let loaded =
        crate::core::source::load_iostore_container_set(paks, &TagNameIndex::default(), &defs)
            .expect("mount CE container set");
    let source = &loaded.source;
    let hlmt = u32::from_be_bytes(*b"hlmt");
    let entry = loaded
        .entries
        .iter()
        .chain(loaded.all_entries.iter())
        .find(|e| {
            e.group_tag == hlmt
                && e.display_path.to_ascii_lowercase().contains(
                    &std::env::var("CE_MODEL").unwrap_or_else(|_| "pelican/pelican".into()),
                )
        })
        .expect("Campaign Evolved model entry")
        .clone();
    eprintln!(
        "[TEST] entry: {} loc={:?}",
        entry.display_path,
        std::mem::discriminant(&entry.location)
    );
    let tag = read_entry(source, &entry).expect("read Campaign Evolved model");
    unsafe {
        std::env::set_var("CE_DEBUG", "1");
    }
    let data = load_campaign_evolved_preview(
        &tag,
        &entry,
        Some(source),
        std::env::var("CE_HD").is_ok(),
    )
    .expect("recognized as CE model")
    .expect("CE preview built");
    eprintln!(
        "[TEST] preview bounds min{:?} max{:?}  ({} draw tris)",
        data.preview.bounds_min,
        data.preview.bounds_max,
        data.preview.indices.len() / 3
    );
    if std::env::var("CE_JMS").is_ok() {
        let skeleton = tag_ref_path(&tag.root(), "skeleton model")
            .expect("Campaign Evolved model has a skeleton model");
        let jms = campaign_evolved_render_jms(&tag, &entry, source, &skeleton)
            .expect("Campaign Evolved Chimp JMS export");
        assert!(!jms.vertices.is_empty());
        assert!(!jms.triangles.is_empty());
        assert!(jms.vertices.iter().all(|vertex| {
            vertex
                .uvs
                .iter()
                .all(|uv| uv.x.is_finite() && uv.y.is_finite())
        }));
        eprintln!(
            "[TEST] Chimp JMS export: {} vertices, {} triangles",
            jms.vertices.len(),
            jms.triangles.len()
        );
    }
    // Optional OBJ export of the exact preview geometry (set CE_OBJ).
    if std::env::var("CE_OBJ").is_ok() {
        use std::io::Write;
        let out = std::env::temp_dir().join(format!(
            "baboon-ce-preview-{}.obj",
            std::env::var("CE_MODEL")
                .unwrap_or_else(|_| "pelican".into())
                .replace(['/', '\\'], "_")
        ));
        let mut f = std::fs::File::create(&out).unwrap();
        for vertex in &data.preview.vertices {
            let pos = vertex.position;
            writeln!(f, "v {} {} {}", pos[0], pos[1], pos[2]).unwrap();
        }
        for triangle in data.preview.indices.chunks_exact(3) {
            writeln!(
                f,
                "f {} {} {}",
                triangle[0] + 1,
                triangle[1] + 1,
                triangle[2] + 1
            )
            .unwrap();
        }
        eprintln!(
            "[TEST] wrote {} ({} tris)",
            out.display(),
            data.preview.indices.len() / 3
        );
    }
}
