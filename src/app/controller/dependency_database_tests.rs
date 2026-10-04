use super::*;

fn loose(root: &Path, all_entries: Vec<TagEntry>) -> LoadedSourceData {
    LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: root.to_path_buf(),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names: TagNameIndex::default(),
        game: None,
        entries: Vec::new(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries,
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    }
}

/// Fix Tag Dependencies uses the completed scan it already has. It used to
/// rescan the whole folder on the UI thread every time.
#[test]
fn fix_dependencies_uses_the_completed_scan_without_rescanning() {
    // An empty folder on disk: a rescan would find nothing.
    let root = std::env::temp_dir().join(format!(
        "baboon-fix-deps-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let known = TagEntry {
        key: "file:objects/a.model".to_owned(),
        display_path: "objects/a.model".to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: None,
        location: TagEntryLocation::LooseFile(root.join("objects/a.model")),
    };
    let mut app = Baboon::for_test();
    app.install_loaded_source(loose(&root, vec![known]));
    let scanned = app
        .dependency_database_entries()
        .map(|entries| entries.len());

    let mut unscanned = Baboon::for_test();
    unscanned.install_loaded_source(loose(&root, Vec::new()));
    let waiting = unscanned.dependency_database_entries().is_err();

    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(
        scanned,
        Ok(1),
        "the in-memory scan, not a rescan of the empty folder"
    );
    assert!(
        waiting,
        "no scan yet: say so rather than scan on the UI thread"
    );
}
