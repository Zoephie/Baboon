use super::*;

/// An edit changes the decoded export, not the payload it was read from,
/// so extracting a dirty document's export used to write the bytes from
/// before the edit.
#[test]
fn extracting_an_edited_export_writes_the_edit() {
    let mut document = crate::app::chimp::test_support::rename_fixture();
    document.payloads = vec![b"as read".to_vec()];
    document.exports[0].class = Some("/Script/Engine.Material".to_owned());
    let serialize = |_: &str, _: &Export| Ok(b"as edited".to_vec());

    assert_eq!(
        chimp_export_bytes(&document, 0, serialize).unwrap().as_deref(),
        Some(&b"as read"[..]),
        "an unedited export is its payload"
    );
    document.dirty = true;
    assert_eq!(
        chimp_export_bytes(&document, 0, serialize).unwrap().as_deref(),
        Some(&b"as edited"[..])
    );
    assert_eq!(chimp_export_bytes(&document, 1, serialize).unwrap(), None);
}

#[test]
fn a_mesh_name_names_the_model_its_textures_belong_to() {
    // The asset-type prefix is not the subject; the segment after it is.
    assert_eq!(
        chimp_mesh_texture_subject("/Game/Art/SM_AssaultRifle_GunBody_M_Default").as_deref(),
        Some("assaultrifle")
    );
    assert_eq!(
        chimp_mesh_texture_subject("/Game/Art/SK_Brute").as_deref(),
        Some("brute")
    );
    // A name that leads with its subject keeps it.
    assert_eq!(
        chimp_mesh_texture_subject("/Game/Art/AssaultRifle_Body").as_deref(),
        Some("assaultrifle")
    );
    assert_eq!(
        chimp_mesh_texture_subject("/Game/Art/Warthog").as_deref(),
        Some("warthog")
    );
    // A prefix and nothing else still has to answer something, or the
    // filter would match every texture in the game.
    assert_eq!(
        chimp_mesh_texture_subject("/Game/Art/SM_").as_deref(),
        Some("sm")
    );
    assert_eq!(chimp_mesh_texture_subject("").as_deref(), None);
}

#[test]
fn the_subject_matches_a_models_textures_and_not_the_shared_ones() {
    let subject = chimp_mesh_texture_subject("/Game/Art/SM_AssaultRifle_GunBody_M_Default")
        .expect("subject");
    let matches = |leaf: &str| leaf.to_lowercase().contains(&subject);
    assert!(matches("T_AssaultRifle_GunBody_D"));
    assert!(matches("T_assaultrifle_ORM"));
    assert!(matches("T_AssaultRifle_Decal_01"));
    // The shared master inputs a material graph drags in are exactly what
    // this is here to leave out.
    assert!(!matches("T_MasterNoise_01"));
    assert!(!matches("T_Default_Normal"));
    assert!(!matches("T_Brute_D"));
}

#[test]
fn textures_land_beside_the_mesh_they_belong_to() {
    let prompt = ChimpMeshTexturePrompt {
        kit: KitId(1),
        package: "/Game/Art/SK_Brute".to_owned(),
        format: ChimpMeshFormat::Pskx,
        texture_export: ChimpTextureExport::default(),
        path: PathBuf::from("C:/exports/brute.pskx"),
    };
    assert_eq!(
        prompt.texture_directory(),
        PathBuf::from("C:/exports").join(CHIMP_TEXTURE_DIR)
    );
    assert_eq!(prompt.format_label(), "ActorX PSKX");
}

fn preview_normal_alignment(preview: &ModelPreviewData) -> (f32, f32) {
    // Unreal's source winding is left-handed, so the sign relative to this
    // right-handed cross product is expected to be negative. Magnitude is
    // the useful regression signal: broken packed normals are incoherent.
    let mut signed = 0.0;
    let mut absolute = 0.0;
    let mut count = 0usize;
    for triangle in preview.preview.indices.chunks_exact(3) {
        let Some(a) = preview.preview.vertices.get(triangle[0] as usize) else {
            continue;
        };
        let Some(b) = preview.preview.vertices.get(triangle[1] as usize) else {
            continue;
        };
        let Some(c) = preview.preview.vertices.get(triangle[2] as usize) else {
            continue;
        };
        let [a, b, c] = [a.position, b.position, c.position];
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let face = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let length = (face[0] * face[0] + face[1] * face[1] + face[2] * face[2]).sqrt();
        if length <= 1.0e-6 {
            continue;
        }
        let face = [face[0] / length, face[1] / length, face[2] / length];
        for normal in [
            preview.preview.vertices[triangle[0] as usize].normal,
            preview.preview.vertices[triangle[1] as usize].normal,
            preview.preview.vertices[triangle[2] as usize].normal,
        ] {
            let dot = face[0] * normal[0] + face[1] * normal[1] + face[2] * normal[2];
            signed += dot;
            absolute += dot.abs();
            count += 1;
        }
    }
    if count == 0 {
        return (0.0, 0.0);
    }
    (signed / count as f32, absolute / count as f32)
}

fn job_at(done: usize, total: usize, elapsed: Duration) -> ChimpLevelJob {
    ChimpLevelJob {
        id: 0,
        kit: KitId(0),
        name: "C10".to_owned(),
        phase: ChimpLevelPhase::ReadingCells,
        done,
        total,
        phase_started: Instant::now() - elapsed,
    }
}

#[test]
fn a_progress_estimate_waits_until_it_has_something_to_go_on() {
    // A rate from the first few items swings by minutes and teaches the
    // user to ignore the number, so there is no number until then.
    assert!(
        job_at(0, 2334, Duration::from_secs(10))
            .remaining()
            .is_none()
    );
    assert!(
        job_at(3, 2334, Duration::from_millis(200))
            .remaining()
            .is_none()
    );
    // Finished is not "0s left", it is nothing to say.
    assert!(
        job_at(2334, 2334, Duration::from_secs(60))
            .remaining()
            .is_none()
    );
}

#[test]
fn a_progress_estimate_extrapolates_the_rate_so_far() {
    // Half done after 60s means about 60s left.
    let remaining = job_at(1_000, 2_000, Duration::from_secs(60))
        .remaining()
        .expect("half way through is enough to estimate from");
    assert!(
        (55..=65).contains(&remaining.as_secs()),
        "estimated {}s",
        remaining.as_secs()
    );
}

#[test]
fn a_fraction_stays_inside_the_bar() {
    assert_eq!(job_at(0, 0, Duration::from_secs(1)).fraction(), 0.0);
    assert_eq!(job_at(1_167, 2_334, Duration::from_secs(1)).fraction(), 0.5);
    // A count past the total would draw outside the bar.
    assert_eq!(job_at(9_999, 2_334, Duration::from_secs(1)).fraction(), 1.0);
}

#[test]
fn time_left_is_said_at_the_precision_it_is_known_to() {
    assert_eq!(format_remaining(Duration::from_secs(3)), "a few seconds");
    assert_eq!(format_remaining(Duration::from_secs(42)), "about 42s");
    assert_eq!(format_remaining(Duration::from_secs(260)), "about 4m 20s");
    // Past ten minutes the seconds are noise.
    assert_eq!(format_remaining(Duration::from_secs(1_500)), "about 25m");
}

#[test]
fn a_persistent_level_is_recognised_by_its_folder() {
    // Unreal names a persistent level after the folder holding it, which is
    // what the menu test keys on before the cells are searched for.
    assert!(chimp_looks_like_level("/Game/Levels/Halo1/Solo/C10/C10"));
    assert!(chimp_looks_like_level("/Game/Levels/Halo1/Solo/c10/C10"));
    // A cell, a mesh, and anything else is one package.
    assert!(!chimp_looks_like_level(
        "/Game/Levels/Halo1/Solo/C10/_Generated_/043ATWPYEEJ"
    ));
    assert!(!chimp_looks_like_level("/Game/Meshes/Rocks/SM_Rock_A"));
    assert!(!chimp_looks_like_level("/Game"));
    assert!(!chimp_looks_like_level(""));
}

#[test]
fn an_export_names_its_files_after_the_level() {
    let prompt = ChimpLevelExportPrompt {
        kit: KitId(0),
        package: "/Game/Levels/Halo1/Solo/C10/C10".to_owned(),
        cells: Vec::new(),
        format: ChimpLevelFormat::SegmentedUsd,
        nanite: true,
        split: true,
        triangles: 30_000_000,
        placements: 50_000,
    };
    assert_eq!(prompt.name(), "C10");
    assert_eq!(prompt.budget().triangles, 30_000_000);
    assert_eq!(prompt.budget().placements, 50_000);
}

#[test]
fn not_splitting_is_one_segment_rather_than_another_exporter() {
    // Turning the split off has to go down the same path, or there are two
    // ways to write a level and only one of them stays tested.
    let prompt = ChimpLevelExportPrompt {
        kit: KitId(0),
        package: "/Game/Levels/X/Small/Small".to_owned(),
        cells: Vec::new(),
        format: ChimpLevelFormat::Blender,
        nanite: false,
        split: false,
        triangles: 30_000_000,
        placements: 50_000,
    };
    assert_eq!(prompt.budget().triangles, usize::MAX);
    assert_eq!(prompt.budget().placements, usize::MAX);
}

/// A multi-row UDIM set numbers its rows downward from the top of the
/// reassembled image, because that is the block coordinate Unreal recovers
/// from the number. Flipping is the mistake, so pin the direction against a
/// set whose rows are told apart by their authored resolution.
#[test]
#[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
fn udim_rows_are_numbered_downward_from_the_top() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
    let package = world
        .packages()
        .iter()
        .map(|package| package.name.clone())
        .find(|name| name.to_ascii_lowercase().ends_with("t_elite_minor_armor_n"))
        .expect("elite minor armour normal");
    let (_, pane) = load_chimp_document_with_pane(&world, &package).unwrap();
    let surfaces = chimp_selected_surfaces(&pane.texture_previews, &package, None).unwrap();
    assert_eq!(
        (surfaces.width_in_blocks, surfaces.height_in_blocks),
        (3, 2)
    );

    // In the middle column this set is full size on the top row and half
    // size on the bottom, so the pair (1002, 1012) fixes the direction.
    let layer = &surfaces.layers[0];
    let top_middle = block_base_level(layer, 3, 2, 1, 0).unwrap();
    let bottom_middle = block_base_level(layer, 3, 2, 1, 1).unwrap();
    assert_ne!(
        top_middle, bottom_middle,
        "this fixture only pins the direction if the two rows differ"
    );

    let directory = std::env::temp_dir().join(format!("baboon-udim-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    write_chimp_texture(
        &world,
        &package,
        &directory.join("t.png"),
        ChimpTextureExport {
            format: ChimpTextureFormat::Png,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    let side = |name: &str| {
        let bytes = std::fs::read(directory.join(name)).unwrap();
        u32::from_be_bytes(bytes[16..20].try_into().unwrap())
    };
    // Top row is 100x, the row below it 101x.
    assert_eq!(
        side("t.1002.png"),
        layer.mips[top_middle].width / 3,
        "1002 should be the top-middle block"
    );
    assert_eq!(
        side("t.1012.png"),
        layer.mips[bottom_middle].width / 3,
        "1012 should be the block below 1002"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

/// Exports a real UDIM virtual texture and checks the DDS files it produces.
///
/// This is the end-to-end check for the whole texture path: the VT tiles are
/// reassembled while still compressed, split back into UDIM blocks, and
/// written with their full mip chain.
#[test]
#[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
fn real_udim_virtual_texture_extracts_to_numbered_dds_files() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
    let target = std::env::var("CE_TEXTURE_PACKAGE")
        .unwrap_or_else(|_| "T_MI_FloodTank_Default_D".to_owned())
        .to_ascii_lowercase();
    let package = world
        .packages()
        .iter()
        .map(|package| package.name.clone())
        .find(|name| name.to_ascii_lowercase().ends_with(&target))
        .unwrap_or_else(|| panic!("no Texture2D package ending in {target:?}"));

    let (_, pane) = load_chimp_document_with_pane(&world, &package).unwrap();
    let surfaces = chimp_selected_surfaces(&pane.texture_previews, &package, None).unwrap();
    assert!(surfaces.is_virtual, "{package} should be a virtual texture");
    assert!(surfaces.is_udim(), "{package} should be a UDIM set");
    // Tiles were cropped in block space, so the surface is still compressed.
    assert_eq!(surfaces.layers[0].mips[0].pixel_format, "PF_DXT1");

    let directory = std::env::temp_dir().join(format!("baboon-dds-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("texture.dds");
    write_chimp_texture(
        &world,
        &package,
        &output,
        ChimpTextureExport::default(),
        None,
    )
    .unwrap();

    let mut written: Vec<String> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    written.sort();
    assert_eq!(
        written.len(),
        (surfaces.width_in_blocks * surfaces.height_in_blocks) as usize,
        "one DDS per UDIM block: {written:?}"
    );
    assert!(
        written.contains(&"texture.1001.dds".to_owned()),
        "{written:?}"
    );

    let bytes = std::fs::read(directory.join("texture.1001.dds")).unwrap();
    assert_eq!(&bytes[0..4], b"DDS ");
    let read_u32 = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let (height, width) = (read_u32(12), read_u32(16));
    let mip_count = read_u32(28);
    // One block of a 7x1 grid over 7168x1024 is a square 1024 page.
    assert_eq!(
        (width, height),
        (
            surfaces.width / surfaces.width_in_blocks,
            surfaces.height / surfaces.height_in_blocks
        )
    );
    assert!(mip_count > 1, "expected a mip chain, found {mip_count}");
    // fourcc "DX10" at the pixel-format block, then the DXGI format.
    assert_eq!(&bytes[84..88], b"DX10");
    assert_eq!(read_u32(128), 71, "PF_DXT1 should write DXGI BC1_UNORM");
    std::fs::remove_dir_all(&directory).unwrap();

    // PNG and TIFF split identically, at the same per-block resolution, so a
    // set exported one way lines up with the same set exported another.
    for (format, extension, magic) in [
        (ChimpTextureFormat::Png, "png", &b"\x89PNG"[..]),
        (ChimpTextureFormat::Tiff, "tif", &b"II*"[..]),
    ] {
        std::fs::create_dir_all(&directory).unwrap();
        write_chimp_texture(
            &world,
            &package,
            &directory.join(format!("texture.{extension}")),
            ChimpTextureExport {
                format,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let mut flat: Vec<String> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        flat.sort();
        assert_eq!(flat.len(), written.len(), "{extension}: {flat:?}");
        assert!(
            flat.contains(&format!("texture.1001.{extension}")),
            "{flat:?}"
        );
        let first = std::fs::read(directory.join(format!("texture.1001.{extension}"))).unwrap();
        assert!(first.starts_with(magic), "{extension} magic");
        std::fs::remove_dir_all(&directory).unwrap();
    }
}

/// Turning the split off writes one stitched image covering every UDIM
/// block, for an engine that cannot import a numbered set.
#[test]
#[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
fn unsplit_udim_exports_one_stitched_image() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
    let package = world
        .packages()
        .iter()
        .map(|package| package.name.clone())
        .find(|name| name.to_ascii_lowercase().ends_with("t_elite_minor_armor_n"))
        .expect("elite minor armour normal");
    let (_, pane) = load_chimp_document_with_pane(&world, &package).unwrap();
    let surfaces = chimp_selected_surfaces(&pane.texture_previews, &package, None).unwrap();
    assert!(surfaces.is_udim());

    let directory =
        std::env::temp_dir().join(format!("baboon-stitched-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    write_chimp_texture(
        &world,
        &package,
        &directory.join("t.png"),
        ChimpTextureExport {
            format: ChimpTextureFormat::Png,
            split_udim: false,
        },
        None,
    )
    .unwrap();
    let written: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(written, vec!["t.png".to_owned()], "one file, not a set");

    let bytes = std::fs::read(directory.join("t.png")).unwrap();
    let side = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    // The whole atlas, at the full reassembled size.
    assert_eq!((side(16), side(20)), (surfaces.width, surfaces.height));
    std::fs::remove_dir_all(directory).unwrap();
}

/// A mesh's textures follow the format chosen in the prompt, not a fixed
/// TIFF, and land beside the mesh in `textures2d`.
#[test]
#[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
fn mesh_textures_use_the_chosen_image_format() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
    let index = index_chimp_package_types(&world);
    let mesh = world
        .packages()
        .iter()
        .zip(&index.package_types)
        .find(|(package, kind)| {
            kind.as_deref() == Some("SkeletalMesh")
                && !chimp_mesh_texture_packages(
                    &world,
                    &load_chimp_document(&world, &package.name).unwrap().header,
                )
                .is_empty()
        })
        .map(|(package, _)| package.name.clone())
        .expect("a skeletal mesh whose materials reference textures");

    for (format, extension) in [
        (ChimpTextureFormat::Png, "png"),
        (ChimpTextureFormat::Dds, "dds"),
    ] {
        let directory =
            std::env::temp_dir().join(format!("baboon-meshtex-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        write_chimp_mesh(
            &world,
            &mesh,
            &directory.join("mesh.pskx"),
            ChimpMeshFormat::Pskx,
            ChimpTextureScope::All,
            ChimpTextureExport {
                format,
                ..Default::default()
            },
        )
        .unwrap();
        let textures: Vec<_> = std::fs::read_dir(directory.join(CHIMP_TEXTURE_DIR))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!textures.is_empty(), "{mesh} wrote no textures");
        assert!(
            textures.iter().all(|name| name.ends_with(extension)),
            "expected only .{extension}: {textures:?}"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}

/// Scratch helper: export a texture to DDS and report each file's header.
/// `CE_TEXTURE_PACKAGE` picks it, `CE_TEXTURE_DDS_DIR` says where.
#[test]
#[ignore = "manual check; set CE_PAKS and CE_TEXTURE_DDS_DIR"]
fn real_texture_to_dds_report() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let directory =
        std::path::PathBuf::from(std::env::var("CE_TEXTURE_DDS_DIR").expect("set dir"));
    std::fs::create_dir_all(&directory).unwrap();
    let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
    let target = std::env::var("CE_TEXTURE_PACKAGE")
        .unwrap_or_else(|_| "T_Elite_Minor_Armor_N".to_owned())
        .to_ascii_lowercase();
    let package = world
        .packages()
        .iter()
        .map(|package| package.name.clone())
        .find(|name| name.to_ascii_lowercase().ends_with(&target))
        .unwrap_or_else(|| panic!("no package ending in {target:?}"));
    let fmt = match std::env::var("CE_TEXTURE_FORMAT")
        .unwrap_or_default()
        .as_str()
    {
        "png" => ChimpTextureFormat::Png,
        "tif" => ChimpTextureFormat::Tiff,
        _ => ChimpTextureFormat::Dds,
    };
    let split = std::env::var("CE_TEXTURE_SPLIT").unwrap_or_default() != "0";
    println!(
        "{}",
        write_chimp_texture(
            &world,
            &package,
            &directory.join(format!("t.{}", fmt.extension())),
            ChimpTextureExport {
                format: fmt,
                split_udim: split
            },
            None
        )
        .unwrap()
    );
    let mut names: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    names.sort();
    for path in names {
        let bytes = std::fs::read(&path).unwrap();
        let at =
            |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        println!(
            "  {}: {}x{} mips={} dxgi={} bytes={}",
            path.file_name().unwrap().to_string_lossy(),
            at(16),
            at(12),
            at(28),
            at(128),
            bytes.len()
        );
    }
}

/// Scratch helper: dump a decoded mip to PNG so it can be eyeballed.
/// `CE_TEXTURE_PACKAGE` picks the texture, `CE_TEXTURE_MIP` the level and
/// `CE_TEXTURE_PNG` the output path.
#[test]
#[ignore = "manual visual check; set CE_PAKS and CE_TEXTURE_PNG"]
fn real_texture_mip_to_png() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let out = std::env::var("CE_TEXTURE_PNG").expect("set CE_TEXTURE_PNG");
    let level: usize = std::env::var("CE_TEXTURE_MIP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
    let target = std::env::var("CE_TEXTURE_PACKAGE")
        .unwrap_or_else(|_| "T_MI_FloodTank_Default_D".to_owned())
        .to_ascii_lowercase();
    let package = world
        .packages()
        .iter()
        .map(|package| package.name.clone())
        .find(|name| name.to_ascii_lowercase().ends_with(&target))
        .unwrap_or_else(|| panic!("no package ending in {target:?}"));
    let (_, pane) = load_chimp_document_with_pane(&world, &package).unwrap();
    let surfaces = chimp_selected_surfaces(&pane.texture_previews, &package, None).unwrap();
    let data = chimp_texture_mip_data(surfaces, 0, level).unwrap();
    println!(
        "{package}: {}x{} {} ({})",
        data.width, data.height, data.format_name, data.type_name
    );
    image::RgbaImage::from_raw(data.width, data.height, data.rgba)
        .unwrap()
        .save(&out)
        .unwrap();
}

#[test]
#[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
fn real_meshes_preview_and_extract_to_jms_and_actorx() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
    let type_index = index_chimp_package_types(&world);

    for (type_name, expected_kind) in [
        ("SkeletalMesh", ChimpMeshKind::Skeletal),
        ("StaticMesh", ChimpMeshKind::Static),
    ] {
        let preferred_suffix = match expected_kind {
            ChimpMeshKind::Skeletal => "/SK_Elite_Common_Body",
            ChimpMeshKind::Static => "",
        };
        let mut candidates = world
            .packages()
            .iter()
            .enumerate()
            .filter(|(package_index, _)| {
                type_index
                    .package_types
                    .get(*package_index)
                    .and_then(Option::as_deref)
                    == Some(type_name)
            })
            .map(|(_, package)| package)
            .collect::<Vec<_>>();
        candidates.sort_by_key(|package| {
            !package
                .name
                .to_ascii_lowercase()
                .ends_with(&preferred_suffix.to_ascii_lowercase())
        });
        let package = candidates
            .into_iter()
            .find_map(|package| {
                let (_, pane) = load_chimp_document_with_pane(&world, &package.name).ok()?;
                pane.mesh_preview.as_ref()?.as_ref().ok()?;
                Some(package.name.clone())
            })
            .unwrap_or_else(|| panic!("no decodable {type_name} package"));
        let (document, pane) = load_chimp_document_with_pane(&world, &package).unwrap();
        assert_eq!(pane.view, ChimpDocumentView::Mesh);
        assert_eq!(document.mesh_kind, Some(expected_kind));
        let preview = pane.mesh_preview.as_ref().unwrap().as_ref().unwrap();
        assert!(!preview.preview.vertices.is_empty());
        assert!(!preview.preview.indices.is_empty());
        assert!(!preview.preview.batches.is_empty());
        let (signed_alignment, absolute_alignment) = preview_normal_alignment(preview);
        eprintln!(
            "{package}: winding-signed normal alignment {signed_alignment:.3}, magnitude {absolute_alignment:.3}"
        );
        assert!(
            absolute_alignment > 0.45,
            "{package} normals do not follow the decoded surface ({absolute_alignment:.3})"
        );

        let formats = if expected_kind == ChimpMeshKind::Skeletal
            && preview.preview.vertices.len() <= 65_536
        {
            vec![
                ChimpMeshFormat::Jms,
                ChimpMeshFormat::Psk,
                ChimpMeshFormat::Pskx,
            ]
        } else {
            vec![ChimpMeshFormat::Jms, ChimpMeshFormat::Pskx]
        };
        for format in formats {
            let output = std::env::temp_dir().join(format!(
                "baboon-chimp-mesh-{}.{}",
                uuid::Uuid::new_v4(),
                format.extension()
            ));
            write_chimp_mesh(
                &world,
                &package,
                &output,
                format,
                ChimpTextureScope::None,
                ChimpTextureExport::default(),
            )
            .unwrap();
            let bytes = std::fs::read(&output).unwrap();
            match format {
                ChimpMeshFormat::Jms => assert!(bytes.starts_with(b";### VERSION ###")),
                ChimpMeshFormat::Psk => {
                    assert!(bytes.windows(8).any(|window| window == b"FACE0000"))
                }
                ChimpMeshFormat::Pskx => {
                    assert!(bytes.windows(8).any(|window| window == b"FACE3200"))
                }
            }
            std::fs::remove_file(output).unwrap();
        }
    }
}

#[test]
#[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
fn real_spiritdropship_nanite_export_is_complete() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
    let package = world
        .packages()
        .iter()
        .find(|package| {
            package
                .name
                .to_ascii_lowercase()
                .contains("sm_spiritdropship_body")
        })
        .unwrap_or_else(|| panic!("SM_SpiritDropShip_Body was not found"));
    let (document, pane) = load_chimp_document_with_pane(&world, &package.name).unwrap();
    assert_eq!(document.mesh_kind, Some(ChimpMeshKind::Static));

    let archive = &world.archives()[document.provider.container];
    let chunk = archive
        .chunk_index_for(&document.provider.entry_path)
        .expect("static mesh package has an IoStore chunk");
    let bulk = archive
        .read_bulk_for(chunk, 0)
        .expect("Nanite static mesh has readable bulk data");
    let resources = blam_tags::iostore::nanite::NaniteResources::parse(
        &document.original,
        document.header.summary.header_size as usize,
    )
    .expect("static mesh has Nanite resources");
    let nanite =
        blam_tags::iostore::nanite::decode_nanite(&document.original, &bulk, &resources);
    let mut miswound_triangles = 0usize;
    let mut duplicate_index_triangles = 0usize;
    let mut zero_area_triangles = 0usize;
    for triangle in &nanite.triangles {
        if triangle[0] == triangle[1]
            || triangle[1] == triangle[2]
            || triangle[0] == triangle[2]
        {
            duplicate_index_triangles += 1;
        }
        let [a, b, c] = triangle.map(|index| nanite.positions[index as usize]);
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let face = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        if face[0] * face[0] + face[1] * face[1] + face[2] * face[2] <= 1.0e-12 {
            zero_area_triangles += 1;
        }
        let normal = triangle.iter().fold([0.0; 3], |mut total, index| {
            let normal = nanite.normals[*index as usize];
            total[0] += normal[0];
            total[1] += normal[1];
            total[2] += normal[2];
            total
        });
        if face[0] * normal[0] + face[1] * normal[1] + face[2] * normal[2] > 0.0 {
            miswound_triangles += 1;
        }
    }
    let converted = StaticMesh::from_nanite(&nanite);
    let preview = pane
        .mesh_preview
        .as_ref()
        .and_then(|preview| preview.as_ref().ok())
        .expect("Nanite static mesh has a Chimp preview");
    assert_eq!(preview.preview.vertices.len(), converted.vertices.len());
    assert_eq!(preview.preview.indices.len(), converted.indices.len());
    let mut converted_miswound_triangles = 0usize;
    let mut severe_uv_stretch_triangles = 0usize;
    let mut maximum_uv_per_cm = 0.0f32;
    let mut long_uv_edge_triangles = 0usize;
    let mut negative_wrap_span_triangles = 0usize;
    let mut maximum_uv_edge = 0.0f32;
    for triangle in converted.indices.chunks_exact(3) {
        let a = converted.vertices[triangle[0] as usize].position;
        let b = converted.vertices[triangle[1] as usize].position;
        let c = converted.vertices[triangle[2] as usize].position;
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let face = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let normal = triangle.iter().fold([0.0; 3], |mut total, index| {
            let normal = converted.vertices[*index as usize].normal;
            total[0] += normal[0];
            total[1] += normal[1];
            total[2] += normal[2];
            total
        });
        if face[0] * normal[0] + face[1] * normal[1] + face[2] * normal[2] > 0.0 {
            converted_miswound_triangles += 1;
        }
        let mut severe_uv_stretch = false;
        let mut long_uv_edge = false;
        for [left, right] in [[0usize, 1usize], [1, 2], [2, 0]] {
            let left = &converted.vertices[triangle[left] as usize];
            let right = &converted.vertices[triangle[right] as usize];
            let dx = left.position[0] - right.position[0];
            let dy = left.position[1] - right.position[1];
            let dz = left.position[2] - right.position[2];
            let position_distance_squared = dx * dx + dy * dy + dz * dz;
            if position_distance_squared <= 1.0e-12 {
                continue;
            }
            let du = left.uv[0] - right.uv[0];
            let dv = left.uv[1] - right.uv[1];
            let uv_edge = (du * du + dv * dv).sqrt();
            maximum_uv_edge = maximum_uv_edge.max(uv_edge);
            long_uv_edge |= uv_edge > 2.0;
            let uv_per_cm = ((du * du + dv * dv) / position_distance_squared).sqrt();
            maximum_uv_per_cm = maximum_uv_per_cm.max(uv_per_cm);
            severe_uv_stretch |= uv_per_cm > 10.0;
        }
        severe_uv_stretch_triangles += usize::from(severe_uv_stretch);
        long_uv_edge_triangles += usize::from(long_uv_edge);
        for axis in 0..2 {
            let coordinates = [triangle[0], triangle[1], triangle[2]]
                .map(|index| converted.vertices[index as usize].uv[axis]);
            let minimum = coordinates.iter().copied().fold(f32::INFINITY, f32::min);
            let maximum = coordinates
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max);
            if minimum < -1.0 && maximum < 0.0 && maximum - minimum > 1.0 {
                negative_wrap_span_triangles += 1;
                break;
            }
        }
    }
    eprintln!(
        "UV wrap diagnostics: long_edges={long_uv_edge_triangles}, negative_wrap_spans={negative_wrap_span_triangles}, maximum_edge={maximum_uv_edge}"
    );
    let mut position_ids = std::collections::HashMap::<[u32; 3], u32>::new();
    let mut canonical_vertices = Vec::with_capacity(converted.vertices.len());
    for vertex in &converted.vertices {
        let key = vertex
            .position
            .map(|value| if value == 0.0 { 0 } else { value.to_bits() });
        let next = position_ids.len() as u32;
        canonical_vertices.push(*position_ids.entry(key).or_insert(next));
    }
    let mut edges = Vec::with_capacity(converted.indices.len());
    for triangle in converted.indices.chunks_exact(3) {
        let ids = [
            canonical_vertices[triangle[0] as usize],
            canonical_vertices[triangle[1] as usize],
            canonical_vertices[triangle[2] as usize],
        ];
        if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2] {
            continue;
        }
        for [a, b] in [[ids[0], ids[1]], [ids[1], ids[2]], [ids[2], ids[0]]] {
            let [lo, hi] = if a < b { [a, b] } else { [b, a] };
            edges.push((u64::from(lo) << 32) | u64::from(hi));
        }
    }
    edges.sort_unstable();
    let mut boundary_edges = Vec::new();
    let mut cursor = 0usize;
    while cursor < edges.len() {
        let edge = edges[cursor];
        let mut end = cursor + 1;
        while end < edges.len() && edges[end] == edge {
            end += 1;
        }
        if end - cursor == 1 {
            boundary_edges.push(edge);
        }
        cursor = end;
    }
    let mut boundary_adjacency = std::collections::HashMap::<u32, Vec<u32>>::new();
    for edge in &boundary_edges {
        let a = (edge >> 32) as u32;
        let b = *edge as u32;
        boundary_adjacency.entry(a).or_default().push(b);
        boundary_adjacency.entry(b).or_default().push(a);
    }
    let mut visited = std::collections::HashSet::new();
    let mut triangular_boundary_loops = 0usize;
    let mut triangular_hole_edges = std::collections::HashMap::<u64, u32>::new();
    for &start in boundary_adjacency.keys() {
        if !visited.insert(start) {
            continue;
        }
        let mut stack = vec![start];
        let mut vertices = 0usize;
        let mut degree_sum = 0usize;
        let mut component = Vec::new();
        while let Some(vertex) = stack.pop() {
            vertices += 1;
            component.push(vertex);
            let neighbours = &boundary_adjacency[&vertex];
            degree_sum += neighbours.len();
            for &neighbour in neighbours {
                if visited.insert(neighbour) {
                    stack.push(neighbour);
                }
            }
        }
        if vertices == 3 && degree_sum == 6 {
            triangular_boundary_loops += 1;
            for index in 0..3 {
                let a = component[index];
                let b = component[(index + 1) % 3];
                let third = component[(index + 2) % 3];
                let [lo, hi] = if a < b { [a, b] } else { [b, a] };
                triangular_hole_edges.insert((u64::from(lo) << 32) | u64::from(hi), third);
            }
        }
    }
    let mut paired_degenerate_triangles = 0usize;
    let mut paired_zero_area_holes = std::collections::HashSet::<[u32; 3]>::new();
    for triangle in converted.indices.chunks_exact(3) {
        let ids = [
            canonical_vertices[triangle[0] as usize],
            canonical_vertices[triangle[1] as usize],
            canonical_vertices[triangle[2] as usize],
        ];
        let mut distinct = ids;
        distinct.sort_unstable();
        let distinct_len = if distinct[0] == distinct[2] {
            1
        } else if distinct[0] == distinct[1] || distinct[1] == distinct[2] {
            2
        } else {
            3
        };
        if distinct_len == 2 {
            let a = distinct[0];
            let b = distinct[2];
            let edge = (u64::from(a) << 32) | u64::from(b);
            paired_degenerate_triangles +=
                usize::from(triangular_hole_edges.contains_key(&edge));
        }

        let positions = [
            converted.vertices[triangle[0] as usize].position,
            converted.vertices[triangle[1] as usize].position,
            converted.vertices[triangle[2] as usize].position,
        ];
        let ab = [
            positions[1][0] - positions[0][0],
            positions[1][1] - positions[0][1],
            positions[1][2] - positions[0][2],
        ];
        let ac = [
            positions[2][0] - positions[0][0],
            positions[2][1] - positions[0][1],
            positions[2][2] - positions[0][2],
        ];
        let cross = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        if cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2] > 1.0e-12 {
            continue;
        }
        for [x, y] in [[ids[0], ids[1]], [ids[1], ids[2]], [ids[2], ids[0]]] {
            if x == y {
                continue;
            }
            let [lo, hi] = if x < y { [x, y] } else { [y, x] };
            let edge = (u64::from(lo) << 32) | u64::from(hi);
            if let Some(&third) = triangular_hole_edges.get(&edge) {
                let mut hole = [lo, hi, third];
                hole.sort_unstable();
                paired_zero_area_holes.insert(hole);
            }
        }
    }
    eprintln!(
        "{}: input_triangles={}, decoded_triangles={}, duplicate_index_triangles={}, zero_area_triangles={}, miswound_before={}, miswound_after={}, severe_uv_stretch_triangles={}, maximum_uv_per_cm={}, boundary_edges={}, triangular_boundary_loops={}, paired_degenerate_triangles={}, paired_zero_area_holes={}, unresolved_vertices={}",
        package.name,
        resources.num_input_triangles,
        nanite.triangles.len(),
        duplicate_index_triangles,
        zero_area_triangles,
        miswound_triangles,
        converted_miswound_triangles,
        severe_uv_stretch_triangles,
        maximum_uv_per_cm,
        boundary_edges.len(),
        triangular_boundary_loops,
        paired_degenerate_triangles,
        paired_zero_area_holes.len(),
        nanite.unresolved_vertices,
    );
    assert_eq!(nanite.unresolved_vertices, 0);
    assert_eq!(
        nanite.triangles.len(),
        resources.num_input_triangles as usize
    );
    assert!(
        miswound_triangles > 0,
        "fixture should exercise the regression"
    );
    assert_eq!(converted_miswound_triangles, 0);
    assert_eq!(
        severe_uv_stretch_triangles, 0,
        "repaired Nanite faces must remain on their local UV seams"
    );
    assert!(maximum_uv_per_cm < 5.0);
    assert_eq!(
        negative_wrap_span_triangles, 0,
        "negative repeating UV faces should be split at wrap boundaries"
    );
    assert_eq!(long_uv_edge_triangles, 0);
    assert!(
        triangular_boundary_loops <= 1,
        "the Nanite repair should remove the mass triangular-hole pattern"
    );

    let expected_faces = converted
        .indices
        .chunks_exact(3)
        .filter(|triangle| {
            triangle[0] != triangle[1]
                && triangle[1] != triangle[2]
                && triangle[0] != triangle[2]
        })
        .count();
    let jms = blam_tags::iostore::actorx::static_mesh_to_jms(&converted, &[]);
    assert_eq!(jms.triangles.len(), expected_faces);
    let mut jms_bytes = Vec::new();
    jms.write(&mut jms_bytes, 8213).unwrap();
    assert!(jms_bytes.starts_with(b";### VERSION ###"));
    let output = std::env::temp_dir().join(format!(
        "baboon-spiritdropship-{}.pskx",
        uuid::Uuid::new_v4()
    ));
    write_chimp_mesh(
        &world,
        &package.name,
        &output,
        ChimpMeshFormat::Pskx,
        ChimpTextureScope::None,
        ChimpTextureExport::default(),
    )
    .unwrap();
    let bytes = std::fs::read(&output).unwrap();
    let face_chunk = bytes
        .windows(8)
        .position(|window| window == b"FACE3200")
        .expect("PSKX contains its 32-bit face chunk");
    let face_count =
        i32::from_le_bytes(bytes[face_chunk + 28..face_chunk + 32].try_into().unwrap())
            as usize;
    assert_eq!(face_count, expected_faces);
    std::fs::remove_file(output).unwrap();
}

fn preview(export_index: usize) -> ChimpTexturePreview {
    ChimpTexturePreview {
        export_index,
        preview: BitmapPreviewState::default(),
        surfaces: Err(format!("export {export_index}")),
    }
}

/// The export the user picked is the one that gets written. A package with
/// two Texture2D exports used to export the first whatever was selected,
/// because the selection was read off a freshly loaded document.
#[test]
fn texture_export_writes_the_selected_texture() {
    let previews = [preview(2), preview(5)];
    let pick = |index| selected_texture_preview(&previews, index).map(|p| p.export_index);

    assert_eq!(pick(Some(5)), Some(5), "the selected texture");
    assert_eq!(pick(Some(2)), Some(2));
    assert_eq!(pick(None), Some(2), "nothing selected: the first texture");
    assert_eq!(
        pick(Some(0)),
        Some(2),
        "a non-texture export selected: the first texture"
    );
    assert_eq!(
        selected_texture_preview(&[], Some(5)).map(|p| p.export_index),
        None
    );
}
