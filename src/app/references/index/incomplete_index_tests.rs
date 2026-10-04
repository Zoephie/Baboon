use super::*;

/// A build that lost tags to a crashed reader says so, rather than
/// reporting a complete index.
#[test]
fn an_incomplete_reference_index_is_reported_as_such() {
    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::SingleFile {
            path: PathBuf::from("a.model"),
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

    app.handle_reverse_dependencies_built(stamp, ReverseDependencyIndex::default(), 3);

    assert!(app.model.status.contains("without 3 tag"), "{}", app.model.status);
}
