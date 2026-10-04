use super::*;

fn temp_fixture(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "baboon-duplicate-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn test_tag() -> TagFile {
    TagFile::new(crate::app::test_definition_path(
        "halo4_mcc/camera_track.json",
    ))
    .unwrap()
}

fn test_entry(key: &str, display_path: &str, group_tag: u32) -> TagEntry {
    TagEntry {
        key: key.to_owned(),
        display_path: display_path.to_owned(),
        group_tag,
        group_name: Some("camera_track".to_owned()),
        location: TagEntryLocation::Container {
            container: 0,
            rel_path: format!("Meteorite/Content/Tags/{display_path}.ubulk"),
        },
    }
}

fn container_source(entries: Vec<TagEntry>) -> LoadedSourceData {
    let tree = crate::source::build_tree(&entries);
    let group_tree = crate::source::build_group_tree(&entries);
    LoadedSourceData {
        label: "duplicate test containers".to_owned(),
        source: TagSource::IoStoreContainerSet {
            root: PathBuf::from("C:/duplicate-test/Paks"),
            containers: Vec::new(),
            index: Arc::new(crate::source::ContainerTagIndex::default()),
            packages: Arc::new(crate::source::ContainerPackageIndex::default()),
            shipped: Arc::new(crate::source::ShippedTagIndex::default()),
        },
        names: TagNameIndex::default(),
        game: None,
        entries,
        tree,
        group_tree,
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    }
}

#[test]
fn duplicate_name_validation_rejects_invalid_and_reserved_leaves() {
    let existing = vec!["objects/source.model".to_owned()];
    for invalid in [
        "", " ", ".", "..", "foo.bar", r"foo\bar", "foo/bar", "foo<bar", "foo*bar", "foo ",
        "foo\t", "CON", "com1", "LPT9",
    ] {
        assert!(
            validate_duplicate_leaf_name(invalid, "objects/new.model", &existing).is_err(),
            "{invalid:?} should be rejected"
        );
    }
    assert_eq!(
        validate_duplicate_leaf_name(" new", "objects/new.model", &existing).unwrap(),
        "new"
    );
}

#[test]
fn duplicate_name_validation_is_case_insensitive_and_includes_source() {
    let existing = vec![
        "Objects/Source.model".to_owned(),
        "objects/existing.model".to_owned(),
    ];
    assert!(
        validate_duplicate_leaf_name("existing", "OBJECTS/EXISTING.model", &existing).is_err()
    );
    assert!(validate_duplicate_leaf_name("Source", "objects/Source.model", &existing).is_err());
}

#[test]
fn duplicate_dialog_prefills_copy_and_keeps_parent_and_extension_fixed() {
    assert_eq!(
        duplicate_dialog_parts("Objects/Old-Biped.BIPED"),
        DuplicateDialogParts {
            prefill: "Old-Biped_copy".to_owned(),
            fixed_parent: "Objects".to_owned(),
            extension: "BIPED".to_owned(),
        }
    );
}

#[test]
fn duplicate_operation_routes_are_distinct_and_preserve_existing_paths() {
    assert_ne!(TagNameOperation::Duplicate, TagNameOperation::SaveAsOverlay);
    assert_ne!(TagNameOperation::Duplicate, TagNameOperation::Rename);
    assert_eq!(
        name_operation_route(TagNameOperation::Duplicate),
        NameOperationRoute::InPlaceDuplicateConfirmation
    );
    assert_eq!(
        name_operation_route(TagNameOperation::SaveAsOverlay),
        NameOperationRoute::SaveAsOverlay
    );
    assert_eq!(
        name_operation_route(TagNameOperation::Rename),
        NameOperationRoute::Rename
    );
}

#[test]
fn duplicate_bytes_keep_stored_bytes_and_do_not_mutate_document_state() {
    let stored = b"stored bytes with original layout";
    assert_eq!(select_duplicate_bytes(stored, None).unwrap(), stored);

    let clean_tag = test_tag();
    let clean_document = TagDocument::clean(clean_tag);
    assert_eq!(
        select_duplicate_bytes(stored, Some(&clean_document)).unwrap(),
        stored
    );

    let dirty_tag = test_tag();
    let dirty_document = TagDocument::modified(dirty_tag);
    let before = dirty_document.tag.write_to_bytes().unwrap();
    let revision = dirty_document.dirty.revision();
    let copied = select_duplicate_bytes(&[], Some(&dirty_document)).unwrap();
    assert_eq!(copied, before);
    assert!(dirty_document.dirty.is_set());
    assert_eq!(dirty_document.dirty.revision(), revision);
    assert_eq!(dirty_document.tag.write_to_bytes().unwrap(), before);
}

#[test]
fn loose_duplicate_destination_and_create_new_preserve_extension_bytes_and_collision() {
    let root = temp_fixture("loose-create-new");
    let parent = root.join("Objects");
    fs::create_dir_all(&parent).unwrap();
    let source = parent.join("Old.BIPED");
    let destination = loose_duplicate_destination(&source, "New_copy").unwrap();
    assert_eq!(destination, parent.join("New_copy.BIPED"));

    let existing = b"keep this destination";
    fs::write(&destination, existing).unwrap();
    assert!(write_create_new(&destination, b"must not replace").is_err());
    assert_eq!(fs::read(&destination).unwrap(), existing);

    reset_readonly_and_remove(&destination);
    let copied = b"exact duplicate bytes\0\x01";
    write_create_new(&destination, copied).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), copied);

    let source_entry = TagEntry {
        key: "file:source".to_owned(),
        display_path: "Old.BIPED".to_owned(),
        group_tag: test_tag().header.group_tag,
        group_name: Some("camera_track".to_owned()),
        location: TagEntryLocation::LooseFile(source.clone()),
    };
    let duplicate_entry = loose_duplicate_entry(
        &TagSource::SingleFile {
            path: source.clone(),
        },
        &source_entry,
        &TagNameIndex::default(),
        &destination,
        "New_copy",
    )
    .unwrap();
    assert_eq!(duplicate_entry.group_tag, source_entry.group_tag);
    assert!(matches!(
        duplicate_entry.location,
        TagEntryLocation::LooseFile(ref path) if path == &destination
    ));

    reset_readonly_and_remove(&destination);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn container_duplicate_paths_preserve_parent_and_group_suffix() {
    let paths = container_duplicate_paths(
        "Meteorite/Content/Tags/Objects/old-biped.ubulk",
        "objects/old.model",
        "new_copy",
    )
    .unwrap();
    assert_eq!(
        paths.uasset,
        "Meteorite/Content/Tags/Objects/new_copy-biped.uasset"
    );
    assert_eq!(
        paths.ubulk,
        "Meteorite/Content/Tags/Objects/new_copy-biped.ubulk"
    );
    assert_eq!(paths.package, "/Game/Tags/Objects/new_copy-biped");
    assert_eq!(paths.display, "objects/new_copy.model");
}

#[test]
fn container_duplicate_index_strips_group_suffix_and_keeps_original_rel_path() {
    let rel_path = "Meteorite/Content/Tags/Objects/new_copy-biped.ubulk";
    let group_tag = u32::from_be_bytes(*b"bipd");
    let logical = container_logical_path(rel_path).unwrap();
    assert_eq!(logical, "objects/new_copy");
    assert_ne!(logical, "objects/new_copy-biped");

    let mut index = crate::source::ContainerTagIndex::default();
    let key = container_duplicate_index_key(group_tag, rel_path).unwrap();
    index.insert(key, 7, rel_path.to_owned());
    assert_eq!(
        index.lookup(group_tag, "objects/new_copy"),
        Some((7, rel_path))
    );
    assert_eq!(
        index.lookup(group_tag, "OBJECTS\\NEW_COPY"),
        Some((7, rel_path))
    );
    assert_eq!(index.lookup(group_tag, "objects/new_copy-biped"), None);
}

#[test]
fn lower_provider_search_is_nearest_first() {
    assert_eq!(
        lower_priority_container_indices(4, 8).collect::<Vec<_>>(),
        vec![3, 2, 1, 0]
    );
    assert_eq!(
        lower_priority_container_indices(0, 8).collect::<Vec<_>>(),
        Vec::<usize>::new()
    );
}

#[test]
fn duplicate_completion_applies_whenever_the_workspace_is_still_open() {
    // The write is already on disk when this runs, and a pak rewrite takes
    // seconds — long enough for routine work to bump the kit's generation.
    // Only a closed workspace may drop the result.
    assert_eq!(
        classify_container_duplicate_completion(true, true),
        ContainerDuplicateCompletion::Apply
    );
    assert_eq!(
        classify_container_duplicate_completion(true, false),
        ContainerDuplicateCompletion::KitClosed
    );
    assert_eq!(
        classify_container_duplicate_completion(false, true),
        ContainerDuplicateCompletion::Failed
    );
}

#[test]
fn a_container_that_is_no_longer_mounted_has_no_index() {
    let source = container_source(vec![]);
    let utoc = Path::new("C:/Game/Paks/pakchunk0-Windows.utoc");
    // A workspace that no longer holds the container a copy was written into
    // is the one case that genuinely cannot register it.
    assert_eq!(container_index_for_utoc(Some(&source), 0, utoc), None);
    assert_eq!(container_index_for_utoc(None, 0, utoc), None);
}

#[test]
fn duplicate_completion_failure_and_closed_kit_paths_clear_guard_without_phantom_state() {
    let group_tag = test_tag().header.group_tag;
    let source_entry = test_entry("source", "objects/source.model", group_tag);
    let destination_entry = test_entry("destination", "objects/new_copy.model", group_tag);

    for (succeeded, current) in [(false, true), (true, false)] {
        let mut running = HashSet::from([KitId(41)]);
        let mut source = container_source(vec![source_entry.clone()]);
        let mut kit = Kit::empty(KitId(41), TagNameIndex::default());
        let destination_tag = test_tag();
        let outcome = classify_container_duplicate_completion(succeeded, current);
        clear_container_duplicate_running(&mut running, KitId(41));

        if outcome == ContainerDuplicateCompletion::Apply {
            apply_container_duplicate_source_state(
                &mut source,
                0,
                group_tag,
                "/Game/Tags/Objects/new_copy-model",
                "Meteorite/Content/Tags/Objects/new_copy-model.uasset",
                "Meteorite/Content/Tags/Objects/new_copy-model.ubulk",
                false,
                &destination_entry,
                &destination_tag,
                &[],
            )
            .unwrap();
            register_clean_duplicate_document(
                &mut kit,
                destination_entry.clone(),
                destination_tag,
            );
        }

        assert!(running.is_empty());
        assert_eq!(source.entries.len(), 1);
        assert_eq!(source.entries[0].key, source_entry.key);
        let TagSource::IoStoreContainerSet { index, .. } = &source.source else {
            panic!("test source must be a container source");
        };
        assert!(index.lookup(group_tag, "objects/new_copy").is_none());
        assert!(!kit.parsed_tags.contains_key(&destination_entry.key));
        assert!(!kit.open_tabs.contains(&destination_entry.key));
    }
}

#[test]
fn duplicate_completion_success_adds_clean_destination_and_preserves_dirty_source() {
    let group_tag = test_tag().header.group_tag;
    let source_entry = test_entry("source", "objects/source.model", group_tag);
    let destination_entry = test_entry("destination", "objects/new_copy.model", group_tag);
    let mut source = container_source(vec![source_entry.clone()]);
    let mut kit = Kit::empty(KitId(41), TagNameIndex::default());
    let source_tag = test_tag();
    let source_document = TagDocument::modified(source_tag);
    let source_before = source_document.tag.write_to_bytes().unwrap();
    let source_revision = source_document.dirty.revision();
    kit.parsed_tags
        .insert(source_entry.key.clone(), source_document);
    let destination_tag = test_tag();

    apply_container_duplicate_source_state(
        &mut source,
        0,
        group_tag,
        "/Game/Tags/Objects/new_copy-model",
        "Meteorite/Content/Tags/Objects/new_copy-model.uasset",
        "Meteorite/Content/Tags/Objects/new_copy-model.ubulk",
        false,
        &destination_entry,
        &destination_tag,
        &[],
    )
    .unwrap();
    register_clean_duplicate_document(&mut kit, destination_entry.clone(), destination_tag);

    assert_eq!(source.entries.len(), 2);
    let TagSource::IoStoreContainerSet { index, .. } = &source.source else {
        panic!("test source must be a container source");
    };
    assert_eq!(
        index.lookup(group_tag, "objects/new_copy"),
        Some((0, "Meteorite/Content/Tags/Objects/new_copy-model.ubulk"))
    );
    let source_document = kit.parsed_tags.get(&source_entry.key).unwrap();
    assert!(source_document.dirty.is_set());
    assert_eq!(source_document.dirty.revision(), source_revision);
    assert_eq!(source_document.tag.write_to_bytes().unwrap(), source_before);
    let destination_document = kit.parsed_tags.get(&destination_entry.key).unwrap();
    assert!(!destination_document.dirty.is_set());
    assert!(kit.open_tabs.contains(&destination_entry.key));
    assert_eq!(
        kit.selected_key.as_deref(),
        Some(destination_entry.key.as_str())
    );
}

#[test]
fn duplicate_lands_beside_its_source_rather_than_at_the_end_of_the_folder() {
    // Folders are drawn in entry-vector order under the default Natural
    // sort, so a pushed copy would appear at the bottom of the folder —
    // which is what made a successful duplicate look like it never landed.
    let group_tag = test_tag().header.group_tag;
    let mut source = container_source(vec![
        test_entry("alpha", "objects/alpha.model", group_tag),
        test_entry("source", "objects/source.model", group_tag),
        test_entry("zulu", "objects/zulu.model", group_tag),
    ]);
    let destination_entry = test_entry("destination", "objects/source_copy.model", group_tag);

    apply_container_duplicate_source_state(
        &mut source,
        0,
        group_tag,
        "/Game/Tags/Objects/source_copy-model",
        "Meteorite/Content/Tags/Objects/source_copy-model.uasset",
        "Meteorite/Content/Tags/Objects/source_copy-model.ubulk",
        false,
        &destination_entry,
        &test_tag(),
        &[],
    )
    .unwrap();

    let order: Vec<&str> = source
        .entries
        .iter()
        .map(|entry| entry.display_path.as_str())
        .collect();
    assert_eq!(
        order,
        [
            "objects/alpha.model",
            "objects/source.model",
            "objects/source_copy.model",
            "objects/zulu.model",
        ]
    );
}

#[test]
fn single_file_registration_updates_browser_source_and_preserves_group() {
    let old_entry = TagEntry {
        key: "file:old".to_owned(),
        display_path: "old.model".to_owned(),
        group_tag: test_tag().header.group_tag,
        group_name: Some("camera_track".to_owned()),
        location: TagEntryLocation::LooseFile(PathBuf::from("old.model")),
    };
    let new_entry = TagEntry {
        key: "file:new".to_owned(),
        display_path: "new.model".to_owned(),
        group_tag: old_entry.group_tag,
        group_name: old_entry.group_name.clone(),
        location: TagEntryLocation::LooseFile(PathBuf::from("new.model")),
    };
    let entries = vec![old_entry];
    let mut source = LoadedSourceData {
        label: "single file".to_owned(),
        source: TagSource::SingleFile {
            path: PathBuf::from("old.model"),
        },
        names: TagNameIndex::default(),
        game: None,
        tree: crate::source::build_tree(&entries),
        group_tree: crate::source::build_group_tree(&entries),
        entries,
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    };

    crate::app::controller::register_created_tag_in_source(&mut source, new_entry.clone(), &[]);

    assert_eq!(source.entries.len(), 2);
    assert!(
        source.entries.iter().any(|entry| {
            entry.key == new_entry.key && entry.group_tag == new_entry.group_tag
        })
    );
    assert!(source.all_entries.is_empty());
    assert!(source.tree.entries.contains(&1));
    assert!(
        source
            .group_tree
            .children
            .iter()
            .flat_map(|node| node.entries.iter())
            .any(|&index| source.entries[index].key == new_entry.key)
    );
}

#[test]
fn duplicate_backup_is_exact_manifested_and_read_only() {
    let root = temp_fixture("backup");
    fs::create_dir_all(&root).unwrap();
    let utoc = root.join("pakchunk7-WinGDK.utoc");
    let ucas = root.join("pakchunk7-WinGDK.ucas");
    let original = b"exact toc bytes\0\x01".to_vec();
    fs::write(&utoc, &original).unwrap();
    fs::write(&ucas, vec![4u8; 37]).unwrap();
    let backup = create_duplicate_backup(&utoc).unwrap();
    assert_eq!(fs::read(&backup.utoc).unwrap(), original);
    assert_eq!(
        backup.manifest.file_name().unwrap().to_string_lossy(),
        "pakchunk7-WinGDK.utoc.baboon-duplicate-backup.manifest.json"
    );
    assert!(fs::metadata(&backup.utoc).unwrap().permissions().readonly());
    assert!(
        fs::metadata(&backup.manifest)
            .unwrap()
            .permissions()
            .readonly()
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&backup.manifest).unwrap()).unwrap();
    assert_eq!(manifest["version"], DUPLICATE_BACKUP_VERSION);
    assert_eq!(manifest["original_ucas_length"], 37);

    // A container can legitimately be written to more than once — duplicate,
    // delete, duplicate again — so a second backup takes the next slot
    // instead of failing, and never disturbs the first.
    fs::write(&utoc, b"second generation").unwrap();
    let second = create_duplicate_backup(&utoc).unwrap();
    assert_ne!(second.utoc, backup.utoc);
    assert_eq!(fs::read(&backup.utoc).unwrap(), original);
    assert_eq!(fs::read(&second.utoc).unwrap(), b"second generation");
    reset_readonly_and_remove(&second.manifest);
    reset_readonly_and_remove(&second.utoc);
    reset_readonly_and_remove(&backup.manifest);
    reset_readonly_and_remove(&backup.utoc);
    let _ = fs::remove_file(utoc);
    let _ = fs::remove_file(ucas);
    let _ = fs::remove_dir(&root);
}

#[test]
fn duplicate_backup_never_writes_over_an_occupied_slot() {
    // A backup is the only record of a state the container can still be
    // walked back to, so an occupied slot is stepped over, never reused —
    // even when what occupies it is not something Baboon wrote.
    let root = temp_fixture("backup-occupied-slot");
    fs::create_dir_all(&root).unwrap();
    let utoc = root.join("pakchunk8-WinGDK.utoc");
    let ucas = root.join("pakchunk8-WinGDK.ucas");
    fs::write(&utoc, b"original toc").unwrap();
    fs::write(&ucas, b"ucas").unwrap();
    let manifest = backup_sibling_path(&utoc, DUPLICATE_BACKUP_MANIFEST_SUFFIX).unwrap();
    fs::write(&manifest, b"keep").unwrap();

    let backup = create_duplicate_backup(&utoc).unwrap();
    assert_ne!(backup.manifest, manifest);
    assert_eq!(fs::read(&manifest).unwrap(), b"keep");
    assert_eq!(fs::read(&backup.utoc).unwrap(), b"original toc");

    reset_readonly_and_remove(&backup.manifest);
    reset_readonly_and_remove(&backup.utoc);
    let _ = fs::remove_file(manifest);
    let _ = fs::remove_file(utoc);
    let _ = fs::remove_file(ucas);
    let _ = fs::remove_dir(&root);
}
