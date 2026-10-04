use super::*;

/// A library asks for its own kit's scan, not the focused kit's.
#[test]
fn a_library_scans_its_own_kit_not_the_focused_one() {
    let root = std::env::temp_dir().join(format!(
        "baboon-library-scan-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut app = Baboon::for_test();
    let second = KitId(app.model.kits[0].id.0 + 1);
    app.model.kits.push(Kit::empty(second, TagNameIndex::default()));
    app.model.active = 1;
    app.install_loaded_source(LoadedSourceData {
        label: "library kit".to_owned(),
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
    app.model.active = 0;

    app.refresh_thumbnail_library::<Models>(1, &egui::Context::default());

    std::fs::remove_dir_all(&root).unwrap();
    assert!(app.model.kits[1].scanning_entries, "the library's kit is scanned");
    assert!(!app.model.kits[0].scanning_entries, "the focused kit is not");
}
