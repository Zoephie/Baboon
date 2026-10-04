use super::*;

/// A refresh patches the reference index with what changed. It used to
/// drop it, so "References to" said the index was unavailable until a
/// manual rebuild after any change the refresh noticed.
#[test]
fn a_refresh_patches_the_reference_index_instead_of_dropping_it() {
    let target = DependencyRef {
        group_tag: u32::from_be_bytes(*b"bitm"),
        rel_path: "shared\\texture".to_owned(),
    };
    let mut index = ReverseDependencyIndex::default();
    index.set_tag_dependencies("file:kept".to_owned(), vec![target.clone()]);
    index.set_tag_dependencies("file:gone".to_owned(), vec![target.clone()]);
    let root = std::env::temp_dir();
    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::SingleFile {
            path: root.join("x"),
        },
        names: TagNameIndex::default(),
        game: None,
        entries: Vec::new(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: Some(index),
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    });

    app.apply_entry_index_refresh(
        0,
        EntryIndexRefresh {
            entries: Vec::new(),
            changed: true,
            added: 1,
            updated: 0,
            removed: 1,
            touched: Vec::new(),
            removed_keys: vec!["file:gone".to_owned()],
            touched_dependencies: vec![("file:new".to_owned(), vec![target.clone()])],
            errors: Vec::new(),
        },
        egui::Context::default(),
    );

    let index = app.kits[0]
        .source
        .as_ref()
        .and_then(|source| source.reverse_dependencies.as_ref())
        .expect("the reference index survives a refresh");
    let mut referrers = index
        .dependents_for(target.group_tag, &target.rel_path)
        .to_vec();
    referrers.sort();
    assert_eq!(referrers, ["file:kept", "file:new"]);
}
