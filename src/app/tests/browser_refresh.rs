use super::*;

#[cfg(windows)]
struct LooseShaderTempRoot(PathBuf);

#[cfg(windows)]
impl Drop for LooseShaderTempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(windows)]
fn loose_shader_fixture(
    forward_slash_root: bool,
) -> (Baboon, LooseShaderTempRoot, PathBuf, TagFile) {
    let mut root = crate::test_kits::unique_temp_path("tag-key-refresh");
    if forward_slash_root {
        root = PathBuf::from(root.to_string_lossy().replace('\\', "/"));
    }
    let cleanup = LooseShaderTempRoot(root.clone());
    let path = root
        .join("objects")
        .join("characters")
        .join("brute")
        .join("shaders")
        .join("brute.shader");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let tag = TagFile::new(crate::app::test_definition_path("halo2_mcc/shader.json")).unwrap();
    std::fs::write(&path, tag.write_to_bytes().unwrap()).unwrap();
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
        all_entries: Vec::new(),
        tree: crate::source::build_folder_directory_tree(&root).unwrap(),
        group_tree: TagTree::default(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    });
    (app, cleanup, path, tag)
}

// Only Windows accepts both separators; on Unix this path has one spelling
// and does not reproduce the browser/full-index mismatch.
#[cfg(windows)]
#[test]
fn mixed_separator_lazy_tag_keeps_its_open_document_after_index_refresh() {
    assert_lazy_tag_survives_refresh(false);
}

#[cfg(windows)]
#[test]
fn lazy_tag_preserves_forward_slash_root_spelling_after_index_refresh() {
    assert_lazy_tag_survives_refresh(true);
}

#[cfg(windows)]
fn assert_lazy_tag_survives_refresh(forward_slash_root: bool) {
    let (mut app, cleanup, path, tag) = loose_shader_fixture(forward_slash_root);
    let root = &cleanup.0;
    // A folder browser uses '/' for its relative path, then appends child
    // folders with the platform separator. This reproduces the Windows key.
    let mut node = TagTreeNode {
        rel_path: PathBuf::from("objects/characters/brute").join("shaders"),
        ..Default::default()
    };
    let source = app.kits[0].source.as_mut().unwrap();
    load_folder_node_entries(&root, &mut node, &mut source.entries, &source.names).unwrap();
    let key = source.entries[0].key.clone();
    assert_eq!(key, format!("file:{}", path.display()));
    assert_eq!(
        node.rel_path,
        PathBuf::from("objects")
            .join("characters")
            .join("brute")
            .join("shaders")
    );
    assert_eq!(node.children.len(), 0);
    let bytes = tag.write_to_bytes().unwrap();
    let document = TagDocument::modified(tag);
    let stamp = document.content_stamp();
    app.kits[0].parsed_tags.insert(key.clone(), document);
    let ctx = egui::Context::default();
    app.select_entry(key.clone(), ctx.clone());

    for _ in 0..2 {
        let entries =
            scan_folder_subtree_entries(&root, Path::new(""), &TagNameIndex::default()).unwrap();
        app.apply_entry_index_refresh(
            0,
            EntryIndexRefresh {
                entries,
                changed: true,
                added: 0,
                updated: 0,
                removed: 0,
                touched: Vec::new(),
                removed_keys: Vec::new(),
                touched_dependencies: Vec::new(),
            },
            ctx.clone(),
        );
        assert!(app.kits[0].source.as_ref().unwrap().entries.is_empty());
        assert!(
            app.entry_for_key_in(0, &key).is_some(),
            "the full index still resolves the open tab"
        );
        assert!(app.kits[0].open_tabs.contains(&key));
        let document = &app.kits[0].parsed_tags[&key];
        assert_eq!(
            document.content_stamp(),
            stamp,
            "refresh must not reload or replace unsaved edits"
        );
        assert!(document.dirty.is_set());
        assert_eq!(document.tag.write_to_bytes().unwrap(), bytes);
    }
}

#[test]
fn browser_refresh_discards_lazy_entries_and_relists_folders() {
    let root = std::env::temp_dir().join(format!(
        "baboon-browser-refresh-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("objects/old")).unwrap();
    let mut tree = crate::source::build_folder_directory_tree(&root).unwrap();
    let mut entries = vec![TagEntry {
        key: "stale".to_owned(),
        display_path: "objects/old/stale.weapon".to_owned(),
        group_tag: u32::from_be_bytes(*b"weap"),
        group_name: None,
        location: TagEntryLocation::LooseFile(root.join("objects/old/stale.weapon")),
    }];

    std::fs::create_dir_all(root.join("new_folder")).unwrap();
    reset_lazy_folder_browser(&root, &mut tree, &mut entries).unwrap();

    assert!(entries.is_empty());
    assert!(tree.children.iter().any(|node| node.label == "new_folder"));
    assert!(tree.children.iter().all(|node| !node.entries_loaded));
    let _ = std::fs::remove_dir_all(root);
}
