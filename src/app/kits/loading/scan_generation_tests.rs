use super::*;

/// A finished scan replaces the lists folder panes index into, so it has
/// to move the generation the panes rebuild on.
#[test]
fn a_finished_scan_moves_the_kit_generation() {
    let root = std::env::temp_dir().join(format!(
        "baboon-scan-generation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: root.clone(),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names: TagNameIndex::default(),
        game: None,
        entries: Vec::new(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    });
    let before = app.model.kits[0].generation;
    let stamp = app.kit_stamp();
    // Not empty: an empty scan leaves the reference build thinking the
    // scan is unfinished, and it starts another scan, which bumps the
    // generation on its own and would hide a missing bump here.
    let scanned = vec![TagEntry {
        key: "file:objects/a.model".to_owned(),
        display_path: "objects/a.model".to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: None,
        location: TagEntryLocation::LooseFile(root.join("objects/a.model")),
    }];

    app.handle_all_entries_scanned(stamp, Ok(scanned), &egui::Context::default());
    assert!(!app.model.kits[0].scanning_entries, "no second scan was started");

    std::fs::remove_dir_all(&root).unwrap();
    assert_ne!(app.model.kits[0].generation, before);
}

/// Loading a source into one kit leaves another kit's index work alone.
/// The flags were app-wide, so any kit finishing a load cleared another
/// kit's running reference build (and its progress bar), which let a
/// second build start over it.
#[test]
fn loading_one_kit_leaves_another_kits_index_build_running() {
    let mut app = Baboon::for_test();
    app.model.kits[0].index_jobs.building_references = true;
    let second = KitId(app.model.kits[0].id.0 + 1);
    app.push_kit(Kit::empty(second, TagNameIndex::default()));

    app.handle_source_loaded(
        second,
        Ok(LoadedSourceData {
            label: "second".to_owned(),
            source: TagSource::SingleFile {
                path: PathBuf::from("second.model"),
            },
            names: TagNameIndex::default(),
            game: None,
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        }),
        None,
        &egui::Context::default(),
    );

    assert!(app.model.kits[0].index_jobs.building_references);
}

/// An empty tags folder scans to nothing, and that is a finished scan.
/// The reference build used to read the empty list as "not scanned yet"
/// and start another scan, which landed empty and started another.
#[test]
fn an_empty_folder_is_scanned_once() {
    let root = std::env::temp_dir().join(format!(
        "baboon-empty-scan-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: root.clone(),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names: TagNameIndex::default(),
        game: None,
        entries: Vec::new(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    });
    let stamp = app.kit_stamp();

    app.handle_all_entries_scanned(stamp, Ok(Vec::new()), &egui::Context::default());

    std::fs::remove_dir_all(&root).unwrap();
    assert!(!app.model.kits[0].scanning_entries, "no second scan was started");
    let index = app.model.kits[0]
        .source
        .as_ref()
        .unwrap()
        .reverse_dependencies
        .as_ref();
    assert!(
        index.is_some(),
        "an empty folder has an empty reference graph"
    );
}
