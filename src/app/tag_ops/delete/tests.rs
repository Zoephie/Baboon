use super::*;
use crate::app::kits::{KitMut, KitView};

fn record_at(utoc: &str, ubulk: &str) -> CreatedTagRecord {
    CreatedTagRecord {
        utoc_path: utoc.to_owned(),
        chunk_label: "pakchunk240-WinGDK".to_owned(),
        package_path: "/Game/Tags/objects/copy-biped".to_owned(),
        package_id: 7,
        uasset_path: "Meteorite/Content/Tags/objects/copy-biped.uasset".to_owned(),
        ubulk_path: ubulk.to_owned(),
        display_path: "objects/copy.biped".to_owned(),
        group_tag: 0,
        source_display: "objects/original.biped".to_owned(),
        container_entry_count_before: 12,
        origin: CreatedTagOrigin::Authored,
        created_unix_secs: 1,
    }
}

fn ledger_with(utoc: &str, ubulk: &str) -> CreatedTagLedger {
    let mut ledger = CreatedTagLedger::default();
    ledger.record(record_at(utoc, ubulk));
    ledger
}

fn container_entry(container: usize, rel_path: &str) -> TagEntry {
    TagEntry {
        key: format!("ublock:pakchunk240-WinGDK:{rel_path}"),
        display_path: "objects/copy.biped".to_owned(),
        group_tag: 0,
        group_name: None,
        location: TagEntryLocation::Container {
            container,
            rel_path: rel_path.to_owned(),
        },
    }
}

#[test]
fn only_recorded_container_copies_are_deletable() {
    let ubulk = "Meteorite/Content/Tags/objects/copy-biped.ubulk";
    let ledger = ledger_with("C:/Game/Paks/pakchunk240-WinGDK.utoc", ubulk);
    let empty = CreatedTagLedger::default();

    // Without a live container list there is nothing to check a record
    // against, so nothing is deletable.
    assert!(delete_eligibility(&container_entry(0, ubulk), &[], &[], &ledger).is_err());
    assert!(delete_eligibility(&container_entry(0, ubulk), &[], &[], &empty).is_err());
}

/// The hole this closes. A renamed tag's chunks are appended like any
/// other, so the appended-chunk threshold reads them as Baboon's and would
/// authorise the delete — of a tag the game shipped, which has no other
/// copy. The ledger has to answer *no*, not merely fail to answer.
#[test]
fn a_renamed_shipped_tag_is_refused_even_though_its_chunks_were_appended() {
    let utoc = Path::new("C:/Game/Paks/pakchunk0-WinGDK.utoc");
    let ubulk = "Meteorite/Content/Tags/objects/renamed-biped.ubulk";
    let mut ledger = CreatedTagLedger::default();
    ledger.record_rename(
        "Meteorite/Content/Tags/objects/shipped-biped.ubulk",
        CreatedTagRecord {
            utoc_path: utoc.display().to_string(),
            ubulk_path: ubulk.to_owned(),
            ..record_at(utoc.to_str().unwrap(), ubulk)
        },
    );

    let verdict = ledger_delete_verdict(&ledger, utoc, ubulk)
        .expect("the rename is recorded, so the ledger has an answer");
    let error = verdict.expect_err("and the answer is no");
    assert!(error.contains("the game ships"), "{error}");
}

/// An origin a newer build wrote is not `Authored`, so it is not deletable.
#[test]
fn an_unrecognized_origin_is_refused() {
    let utoc = Path::new("C:/Game/Paks/pakchunk240-WinGDK.utoc");
    let ubulk = "Meteorite/Content/Tags/objects/copy-biped.ubulk";
    let mut ledger = CreatedTagLedger::default();
    ledger.record(CreatedTagRecord {
        origin: CreatedTagOrigin::Unrecognized("ImportedFromMod".to_owned()),
        ..record_at(utoc.to_str().unwrap(), ubulk)
    });
    let verdict = ledger_delete_verdict(&ledger, utoc, ubulk).expect("recorded");
    let error = verdict.expect_err("not Baboon's to delete");
    assert!(error.contains("ImportedFromMod"), "{error}");
    // And the converse, so the check can disagree.
    let authored = ledger_with(utoc.to_str().unwrap(), ubulk);
    assert!(matches!(ledger_delete_verdict(&authored, utoc, ubulk), Some(Ok(_))));
}

/// The other half of the same call: a copy Baboon made stays deletable
/// after being renamed, because the origin follows the tag rather than
/// being re-derived from where its chunks now sit.
#[test]
fn a_renamed_copy_is_still_deletable() {
    let utoc = Path::new("C:/Game/Paks/pakchunk240-WinGDK.utoc");
    let old = "Meteorite/Content/Tags/objects/copy-biped.ubulk";
    let new = "Meteorite/Content/Tags/objects/moved-biped.ubulk";
    let mut ledger = ledger_with(utoc.to_str().unwrap(), old);
    ledger.record_rename(
        old,
        CreatedTagRecord {
            ubulk_path: new.to_owned(),
            ..record_at(utoc.to_str().unwrap(), new)
        },
    );

    assert!(
        ledger_delete_verdict(&ledger, utoc, old).is_none(),
        "nothing is at the old path any more"
    );
    let target = ledger_delete_verdict(&ledger, utoc, new)
        .expect("the copy is recorded at its new path")
        .expect("and it is still Baboon's to delete");
    assert_eq!(target.ubulk_path, new);
    assert_eq!(
        target.minimum_appended_index,
        Some(12),
        "the original provenance line is carried, not re-guessed"
    );
}

#[test]
fn unsaved_and_read_only_entries_are_never_deletable() {
    let ledger = CreatedTagLedger::default();
    let new_container = TagEntry {
        key: "new:1".to_owned(),
        display_path: "objects/fresh.biped".to_owned(),
        group_tag: 0,
        group_name: None,
        location: TagEntryLocation::NewContainer {
            template: NewContainerTemplate::Donor {
                container: 0,
                rel_path: "Tags/template-biped.uasset".to_owned(),
            },
            package: "/Game/Tags/objects/fresh-biped".to_owned(),
            group_tag: 0,
        },
    };
    let monolithic = TagEntry {
        key: "cache:bipd:objects/cached".to_owned(),
        display_path: "objects/cached.biped".to_owned(),
        group_tag: 0,
        group_name: None,
        location: TagEntryLocation::Monolithic {
            name: "objects/cached".to_owned(),
            group_tag: 0,
        },
    };
    assert!(delete_eligibility(&new_container, &[], &[], &ledger).is_err());
    assert!(delete_eligibility(&monolithic, &[], &[], &ledger).is_err());
}

#[test]
fn a_missing_loose_file_is_not_deletable() {
    let ledger = CreatedTagLedger::default();
    let entry = TagEntry {
        key: "file:missing".to_owned(),
        display_path: "objects/missing.biped".to_owned(),
        group_tag: 0,
        group_name: None,
        location: TagEntryLocation::LooseFile(PathBuf::from(
            "C:/definitely/not/here/missing.biped",
        )),
    };
    assert!(delete_eligibility(&entry, &[], &[], &ledger).is_err());
}

#[test]
fn forgetting_a_tag_clears_its_kit_state_and_bumps_the_generation() {
    let mut kit = Kit::empty(KitId(7), TagNameIndex::default());
    let entry = container_entry(0, "Meteorite/Content/Tags/objects/copy-biped.ubulk");
    let key = entry.key.clone();
    let other = TagEntry {
        key: "keep".to_owned(),
        display_path: "objects/keep.biped".to_owned(),
        ..container_entry(0, "Meteorite/Content/Tags/objects/keep-biped.ubulk")
    };
    let entries = vec![entry.clone(), other.clone()];
    kit.source = Some(LoadedSourceData {
        label: "delete test".to_owned(),
        source: TagSource::IoStoreContainerSet {
            root: PathBuf::from("C:/Game/Paks"),
            containers: Vec::new(),
            index: Arc::new(crate::core::source::ContainerTagIndex::default()),
            packages: Arc::new(crate::core::source::ContainerPackageIndex::default()),
            shipped: Arc::new(crate::core::source::ShippedTagIndex::default()),
        },
        names: TagNameIndex::default(),
        game: None,
        tree: crate::core::source::build_tree(&entries),
        group_tree: crate::core::source::build_group_tree(&entries),
        entries,
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    });
    kit.selected_key = Some(key.clone());
    let generation_before = kit.generation;

    let mut view = KitView::for_test(&kit);
    forget_tag_in_kit(KitMut::new(&mut kit, &mut view), &key);

    let source = kit.source.as_ref().unwrap();
    assert_eq!(source.entries.len(), 1);
    assert_eq!(source.entries[0].key, other.key);
    assert!(!kit.parsed_tags.contains_key(&key));
    assert_eq!(kit.selected_key, None);
    assert_ne!(
        kit.generation, generation_before,
        "the browser filter and field index only rebuild on a new generation"
    );
    assert!(
        !source
            .tree
            .children
            .iter()
            .flat_map(|node| node.entries.iter())
            .any(|&index| source.entries.get(index).is_none()),
        "the rebuilt tree must not address entries that no longer exist"
    );
}

#[test]
fn moving_a_deleted_tag_never_overwrites_what_is_already_there() {
    let root = std::env::temp_dir().join(format!(
        "baboon-delete-trash-{}-{}",
        std::process::id(),
        now_unix_secs()
    ));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("tag.biped");
    let destination = root.join("moved.biped");
    fs::write(&source, b"tag bytes").unwrap();

    move_to_trash(&source, &destination).unwrap();
    assert!(!source.exists(), "the original is gone once it has moved");
    assert_eq!(fs::read(&destination).unwrap(), b"tag bytes");

    fs::write(&source, b"second tag").unwrap();
    assert!(move_to_trash(&source, &destination).is_err());
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"tag bytes",
        "an occupied destination keeps its own bytes"
    );
    assert!(source.exists(), "a refused move leaves the tag in place");

    let _ = fs::remove_file(&source);
    let _ = fs::remove_file(&destination);
    let _ = fs::remove_dir(&root);
}

#[test]
fn loose_trash_keeps_the_tag_path_and_refuses_to_escape_it() {
    let destination =
        loose_trash_destination(Some("haloce_evolved"), "objects\\weapons\\rifle.weapon", 99)
            .unwrap();
    let tail: Vec<String> = destination
        .components()
        .rev()
        .take(5)
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        tail,
        vec![
            "rifle.weapon".to_owned(),
            "weapons".to_owned(),
            "objects".to_owned(),
            "99".to_owned(),
            "haloce_evolved".to_owned(),
        ]
    );
    assert!(loose_trash_destination(None, "../../escape.weapon", 1).is_err());
    assert!(loose_trash_destination(None, "", 1).is_err());
}
