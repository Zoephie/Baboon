use super::*;
use crate::app::kits::loading::persist_entry_index_changes;

/// A plain Save leaves nothing for the periodic refresh to find, and the
/// reference index knows what the saved tag now points at.
#[test]
fn a_saved_tag_updates_its_index_row_and_references() {
    let root = std::env::temp_dir().join(format!(
        "baboon-save-index-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // Rows under a real game are told apart by this test's own folder.
    let game = GameId::Halo3;
    std::fs::create_dir_all(root.join("objects")).unwrap();
    let path = root.join("objects/crate.model");
    let mut tag = TagFile::new(locate_definitions_root().join("halo3_mcc/model.json")).unwrap();
    tag.write_atomic(&path).unwrap();
    let names = TagNameIndex::default();
    let entries =
        crate::core::source::scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    crate::core::source::save_entry_index(game.as_str(), &root, &entries).unwrap();
    let entry = entries[0].clone();

    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: root.clone(),
            game: Some(game),
            definitions_root: PathBuf::new(),
        },
        names: names.clone(),
        game: Some(game),
        entries: entries.clone(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: entries.clone(),
        reverse_dependencies: Some(ReverseDependencyIndex::default()),
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    });
    crate::app::apply_field_edit(&mut tag, "render model", "mode:objects/crate").unwrap();
    app.model.kits[0]
        .parsed_tags
        .insert(entry.key.clone(), TagDocument::modified(tag));

    let saved = app.save_tag_by_key(&entry.key);
    let refresh = crate::core::source::refresh_entry_index(game.as_str(), &root, &names);
    let referrers = app.model.kits[0]
        .source
        .as_ref()
        .and_then(|source| source.reverse_dependencies.as_ref())
        .map(|index| {
            index
                .dependents_for(u32::from_be_bytes(*b"mode"), "objects\\crate")
                .to_vec()
        });

    crate::core::source::remove_test_index_source(game.as_str(), &root);
    std::fs::remove_dir_all(&root).unwrap();
    assert!(saved.is_ok(), "{saved:?}");
    assert!(
        !refresh.unwrap().changed,
        "the refresh finds the save already indexed"
    );
    assert_eq!(referrers, Some(vec![entry.key.clone()]));
}

/// A reference-index build reads every tag before it reports. A tag saved
/// while it ran had its new references recorded, and then the finished
/// build replaced the index with what it had read before the save. The
/// saved tag's fingerprint was current, so no refresh ever fixed it.
#[test]
fn a_tag_saved_during_a_reference_build_keeps_its_new_references() {
    let root = std::env::temp_dir().join(format!(
        "baboon-save-during-build-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // Rows under a real game are told apart by this test's own folder.
    let game = GameId::Halo3;
    std::fs::create_dir_all(root.join("objects")).unwrap();
    let path = root.join("objects/crate.model");
    let mut tag = TagFile::new(locate_definitions_root().join("halo3_mcc/model.json")).unwrap();
    tag.write_atomic(&path).unwrap();
    let names = TagNameIndex::default();
    let entries =
        crate::core::source::scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    crate::core::source::save_entry_index(game.as_str(), &root, &entries).unwrap();
    let entry = entries[0].clone();

    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: root.clone(),
            game: Some(game),
            definitions_root: PathBuf::new(),
        },
        names: names.clone(),
        game: None,
        entries: entries.clone(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: entries.clone(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    });
    // A build starts, and reads the tag as it is: pointing at nothing.
    let stamp = app.kit_stamp();
    app.model.kits[0].index_jobs.building_references = true;
    let mut read_before_the_save = ReverseDependencyIndex::default();
    read_before_the_save.set_tag_dependencies(entry.key.clone(), Vec::new());

    // Then the tag is edited and saved while the build is still running.
    crate::app::apply_field_edit(&mut tag, "render model", "mode:objects/crate").unwrap();
    app.model.kits[0]
        .parsed_tags
        .insert(entry.key.clone(), TagDocument::modified(tag));
    let saved = app.save_tag_by_key(&entry.key);
    app.handle_reverse_dependencies_built(stamp, read_before_the_save, 0);

    let referrers = app.model.kits[0]
        .source
        .as_ref()
        .and_then(|source| source.reverse_dependencies.as_ref())
        .map(|index| {
            index
                .dependents_for(u32::from_be_bytes(*b"mode"), "objects\\crate")
                .to_vec()
        });
    crate::core::source::remove_test_index_source(game.as_str(), &root);
    std::fs::remove_dir_all(&root).unwrap();
    assert!(saved.is_ok(), "{saved:?}");
    assert_eq!(referrers, Some(vec![entry.key.clone()]));
    assert!(
        app.model.kits[0]
            .index_jobs
            .references_changed_during_build
            .is_empty(),
        "the changes are spent once the build lands"
    );
}

/// The refresh used to drop every write error and every unreadable tag
/// without a word; they now reach the status line.
#[test]
fn a_refresh_reports_a_tag_whose_references_cannot_be_read() {
    let root = crate::test_kits::unique_temp_dir("refresh-errors");
    std::fs::create_dir_all(root.join("objects")).unwrap();
    let good = root.join("objects/good.model");
    TagFile::new(locate_definitions_root().join("halo3_mcc/model.json"))
        .unwrap()
        .write_atomic(&good)
        .unwrap();
    let bad = root.join("objects/bad.model");
    std::fs::write(&bad, b"not a tag").unwrap();
    let entry = |path: &Path| TagEntry {
        key: file_entry_key(&path),
        display_path: path
            .strip_prefix(&root)
            .unwrap()
            .display()
            .to_string(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: None,
        location: TagEntryLocation::LooseFile(path.to_path_buf()),
    };
    let refresh = EntryIndexRefresh {
        entries: Vec::new(),
        changed: true,
        added: 2,
        updated: 0,
        removed: 0,
        touched: vec![entry(&good), entry(&bad)],
        removed_keys: Vec::new(),
        touched_dependencies: Vec::new(),
        errors: Vec::new(),
    };
    let source = TagSource::LooseFolder {
        root: root.clone(),
        game: None,
        definitions_root: PathBuf::new(),
    };
    let game = format!("refresh_errors_{}", std::process::id());
    let refresh = persist_entry_index_changes(&game, &root, &source, refresh);
    crate::core::source::remove_test_index_rows(&game);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(refresh.touched_dependencies.len(), 1, "the good tag is read");
    assert_eq!(refresh.errors.len(), 1, "{:?}", refresh.errors);
    assert!(refresh.errors[0].contains("bad.model"), "{:?}", refresh.errors);
}

/// The shader grid reads definitions and options through per-kit caches
/// that never looked at the file again, so saving one left the grid
/// showing the old parameters until the source was reloaded.
#[test]
fn saving_a_render_method_option_drops_the_cached_ones() {
    let root = std::env::temp_dir().join(format!(
        "baboon-save-rmop-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // Rows under a real game are told apart by this test's own folder.
    let game = GameId::Halo3;
    std::fs::create_dir_all(root.join("shaders")).unwrap();
    for (file, group) in [
        ("shaders/bump.render_method_option", "render_method_option"),
        ("shaders/crate.model", "model"),
    ] {
        TagFile::new(locate_definitions_root().join(format!("halo3_mcc/{group}.json")))
            .unwrap()
            .write_atomic(root.join(file))
            .unwrap();
    }
    let names = TagNameIndex::default();
    let entries =
        crate::core::source::scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: root.clone(),
            game: Some(game),
            definitions_root: PathBuf::new(),
        },
        names: names.clone(),
        game: Some(game),
        entries: entries.clone(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    });
    let key_of = |group: &[u8; 4]| {
        entries
            .iter()
            .find(|entry| entry.group_tag == u32::from_be_bytes(*group))
            .map(|entry| entry.key.clone())
            .unwrap()
    };
    let save = |app: &mut Baboon, key: &str, group: &str| {
        let tag =
            TagFile::new(locate_definitions_root().join(format!("halo3_mcc/{group}.json")))
                .unwrap();
        app.model.kits[0]
            .parsed_tags
            .insert(key.to_owned(), TagDocument::modified(tag));
        app.views[app.model.kits[0].id]
            .caches.rmop_cache
            .insert("rmop:shaders\\bump".to_owned(), None);
        let epoch = app.views[app.model.kits[0].id].caches.render_method_epoch;
        let saved = app.save_tag_by_key(key);
        assert!(saved.is_ok(), "{saved:?}");
        (
            app.views[app.model.kits[0].id].caches.rmop_cache.is_empty(),
            app.views[app.model.kits[0].id].caches.render_method_epoch != epoch,
        )
    };

    let model = save(&mut app, &key_of(b"hlmt"), "model");
    let option = save(&mut app, &key_of(b"rmop"), "render_method_option");
    crate::core::source::remove_test_index_source(game.as_str(), &root);
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(model, (false, false), "saving another group leaves them");
    assert_eq!(option, (true, true), "saving an option drops them");
}
