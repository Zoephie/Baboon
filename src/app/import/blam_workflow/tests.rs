use super::*;

/// Root of a real H3EK install, via `BLAM_TEST_H3EK_ROOT`. Skips (like the
/// other kit-dependent tests) when absent.
fn h3ek_root() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var("BLAM_TEST_H3EK_ROOT").ok()?);
    root.join("data").is_dir().then_some(root)
}

fn scratch_dir(name: &str) -> PathBuf {
    crate::test_kits::unique_temp_dir(name)
}

fn run_job(
    job: &BlamImportJob,
) -> (
    Vec<(String, Result<String, String>)>,
    Vec<(TagEntry, TagFile)>,
) {
    let (tx, _rx) = std::sync::mpsc::channel();
    let stamp = KitStamp {
        kit: KitId(0),
        generation: 0,
    };
    run_blam_import(job, &tx, stamp, &egui::Context::default())
}

fn assert_written_and_rereadable(created: &[(TagEntry, TagFile)]) {
    for (entry, _tag) in created {
        let TagEntryLocation::LooseFile(path) = &entry.location else {
            panic!("an imported tag must be a loose file");
        };
        assert!(path.is_file(), "{} was not written", path.display());
        TagFile::read(path)
            .unwrap_or_else(|error| panic!("{} does not re-read: {error}", path.display()));
    }
}

/// The full worker pass over a stock asset: render, collision and physics
/// from the kit's own source files, written into a scratch tags root so
/// the real kit is untouched. PRT stays off here for speed — the solver
/// itself is proven in blam-tags' own corpus tests.
#[test]
fn a_stock_asset_imports_all_three_model_tags() {
    let Some(root) = h3ek_root() else {
        eprintln!("skipping: set BLAM_TEST_H3EK_ROOT to a real H3EK install");
        return;
    };
    let asset = "objects/vehicles/ghost_aa";
    let data_dir = root.join("data").join(asset);
    if !data_dir.join("render").is_dir() {
        eprintln!("skipping: no {asset} sources in this kit's data");
        return;
    }
    let tags_root = scratch_dir("blam-import");
    let job = BlamImportJob {
        data_dir,
        tags_root: tags_root.clone(),
        asset_rel: asset.to_owned(),
        asset_name: "ghost_aa".to_owned(),
        schema_dir: locate_definitions_root().join("halo3_mcc"),
        names: TagNameIndex::default(),
        render: true,
        prt: false,
        collision: true,
        physics: true,
        structure: false,
    };
    let (outcomes, created) = run_job(&job);
    for (label, result) in &outcomes {
        assert!(result.is_ok(), "{label} failed: {result:?}");
    }
    assert_eq!(created.len(), 3, "render, collision and physics tags");
    assert_written_and_rereadable(&created);
    assert!(
        tags_root
            .join(asset)
            .join("ghost_aa.render_model")
            .is_file(),
        "the render_model must be filed where the kit files it",
    );
    std::fs::remove_dir_all(&tags_root).unwrap();
}

/// The structure pipeline routes every `.ass` in the folder to its own
/// BSP, named after the file. One small shipped level file, staged into a
/// scratch data folder so only that one imports.
#[test]
fn a_shipped_level_ass_becomes_a_structure_bsp() {
    let Some(root) = h3ek_root() else {
        eprintln!("skipping: set BLAM_TEST_H3EK_ROOT to a real H3EK install");
        return;
    };
    let source = root.join("data/levels/solo/070_waste/structure/070_bsp_011.ass");
    if !source.is_file() {
        eprintln!("skipping: no 070_bsp_011.ass in this kit's data");
        return;
    }
    let scratch = scratch_dir("blam-sbsp");
    let data_dir = scratch.join("data/levels/test_level");
    std::fs::create_dir_all(data_dir.join("structure")).unwrap();
    std::fs::copy(&source, data_dir.join("structure/070_bsp_011.ass")).unwrap();
    let tags_root = scratch.join("tags");
    let job = BlamImportJob {
        data_dir,
        tags_root: tags_root.clone(),
        asset_rel: "levels/test_level".to_owned(),
        asset_name: "test_level".to_owned(),
        schema_dir: locate_definitions_root().join("halo3_mcc"),
        names: TagNameIndex::default(),
        render: false,
        prt: false,
        collision: false,
        physics: false,
        structure: true,
    };
    let (outcomes, created) = run_job(&job);
    for (label, result) in &outcomes {
        assert!(result.is_ok(), "{label} failed: {result:?}");
    }
    assert_eq!(created.len(), 1);
    assert!(
        tags_root
            .join("levels/test_level/070_bsp_011.scenario_structure_bsp")
            .is_file(),
        "the BSP must be named after its .ass, not the level folder",
    );
    assert_written_and_rereadable(&created);
    std::fs::remove_dir_all(&scratch).unwrap();
}

/// A filed tag's entry must be the one the folder scan would make for the
/// same file. It used to be keyed by its bare display path, which the entry
/// index cannot read back and no browser-opened tab ever matches.
#[test]
fn a_filed_tag_is_keyed_like_the_folder_scan() {
    let scratch = scratch_dir("blam-file-tag");
    let tags_root = scratch.join("tags");
    let schema_dir = locate_definitions_root().join("halo3_mcc");
    let job = BlamImportJob {
        data_dir: scratch.join("data/objects/test"),
        tags_root: tags_root.clone(),
        asset_rel: "objects/test".to_owned(),
        asset_name: "test".to_owned(),
        schema_dir: schema_dir.clone(),
        names: TagNameIndex::default(),
        render: false,
        prt: false,
        collision: false,
        physics: false,
        structure: false,
    };
    let tag = TagFile::new(schema_path(&schema_dir, "render_model").unwrap()).unwrap();

    let (entry, _) = file_tag(&job, tag, "test", "render_model").unwrap();
    let scanned = crate::core::source::scan_folder_subtree_entries(
        &tags_root,
        Path::new(""),
        &TagNameIndex::default(),
    )
    .unwrap();

    std::fs::remove_dir_all(&scratch).unwrap();
    assert_eq!(scanned.len(), 1);
    assert_eq!(entry.key, scanned[0].key);
    assert_eq!(entry.display_path, scanned[0].display_path);
}
