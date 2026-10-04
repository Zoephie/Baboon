use super::EditingKitProfileIdentity;
use super::{Kit, KitId, TagDocument, active_after_removal, kit_has_dirty_documents};
use crate::app::test_definition_path;
use crate::core::source::{LoadedSourceData, TagEntry, TagEntryLocation, TagSource, build_tree};
use blam_tags::TagFile;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

fn kit_holding(location: TagEntryLocation, endian: blam_tags::Endian) -> Kit {
    let mut tag = TagFile::new(test_definition_path("halo4_mcc/camera_track.json")).unwrap();
    tag.endian = endian;
    let entry = TagEntry {
        key: "tag".to_owned(),
        display_path: "test/example.camera_track".to_owned(),
        group_tag: tag.header.group_tag,
        group_name: Some("camera_track".to_owned()),
        location,
    };
    let entries = vec![entry];
    let mut kit = Kit::empty(KitId(0), Default::default());
    kit.source = Some(LoadedSourceData {
        label: "test".to_owned(),
        // Irrelevant to the question asked: what decides an edit's fate is
        // the entry's location, not how the browser was opened.
        source: TagSource::SingleFile {
            path: PathBuf::from("example.camera_track"),
        },
        names: Default::default(),
        game: None,
        tree: build_tree(&entries),
        group_tree: build_tree(&entries),
        entries,
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    });
    kit.parsed_tags
        .insert("tag".to_owned(), TagDocument::modified(tag));
    kit
}

/// Edits to a tag with nowhere to be written are not unsaved *work*, and
/// must not raise the close prompt.
///
/// The prompt's only outcomes are Save and Discard. Save on a monolithic
/// build fails by construction, and `CloseApp` re-checks for dirty
/// documents after the prompt closes — so counting these would put a quit
/// behind a dialog whose Save button can never clear it.
#[test]
fn a_dirty_tag_that_can_never_be_saved_is_not_unsaved_work() {
    let monolithic = kit_holding(
        TagEntryLocation::Monolithic {
            name: "test\\example".to_owned(),
            group_tag: u32::from_be_bytes(*b"trak"),
        },
        blam_tags::Endian::Be,
    );
    assert!(
        !kit_has_dirty_documents(&monolithic),
        "a monolithic build's edits are session-scratch, not unsaved work"
    );

    // The same document from somewhere it can be written back to still
    // stops a close, which is the whole point of the flag.
    let loose = kit_holding(
        TagEntryLocation::LooseFile(PathBuf::from("example.camera_track")),
        blam_tags::Endian::Le,
    );
    assert!(kit_has_dirty_documents(&loose));
}

/// A folder move rewrites tag keys underneath the open tabs. The tree is
/// what has to be rewritten: `open_tabs` is re-derived from it every frame,
/// so a remap that touched only the list was overwritten immediately and
/// left the panes pointing at keys the source no longer had.
#[test]
fn remapping_tag_keys_rewrites_the_layout_tree_itself() {
    let mut kit = Kit::empty(KitId(0), Default::default());
    kit.open_tag_pane("file:/tags/objects/a.weapon");
    kit.open_tag_pane("file:/tags/objects/b.weapon");
    kit.selected_key = Some("file:/tags/objects/a.weapon".to_owned());

    let mut map = HashMap::new();
    map.insert(
        "file:/tags/objects/a.weapon".to_owned(),
        "file:/tags/moved/a.weapon".to_owned(),
    );
    kit.remap_tag_keys(&map);

    // Read back through the tree, not the cached list, so the assertion
    // fails if only the list was rewritten.
    let panes = kit.tabs_from_tree();
    assert!(panes.contains(&"file:/tags/moved/a.weapon".to_owned()));
    assert!(!panes.contains(&"file:/tags/objects/a.weapon".to_owned()));
    assert!(panes.contains(&"file:/tags/objects/b.weapon".to_owned()));
    assert_eq!(kit.open_tabs, panes);
    assert_eq!(
        kit.selected_key.as_deref(),
        Some("file:/tags/moved/a.weapon")
    );
}

#[test]
fn removing_a_kit_before_the_active_one_shifts_the_selection_down() {
    // [a b *c] -> remove a -> [b *c]: the active kit moved from 2 to 1.
    assert_eq!(active_after_removal(2, 0, 2), 1);
    assert_eq!(active_after_removal(1, 0, 2), 0);
}

#[test]
fn removing_a_kit_after_the_active_one_leaves_the_selection_alone() {
    // [*a b c] -> remove c -> [*a b]: still index 0.
    assert_eq!(active_after_removal(0, 2, 2), 0);
    assert_eq!(active_after_removal(1, 2, 2), 1);
}

#[test]
fn removing_the_active_kit_selects_the_one_that_took_its_place() {
    // [a *b c] -> remove b -> [a c]: index 1 is now the former c.
    assert_eq!(active_after_removal(1, 1, 2), 1);
    // Removing the last kit clamps back onto the new last kit.
    assert_eq!(active_after_removal(2, 2, 2), 1);
}

#[test]
fn the_selection_never_points_past_the_end() {
    // Closing the only kit leaves one fresh empty workspace behind it.
    assert_eq!(active_after_removal(0, 0, 1), 0);
    assert_eq!(active_after_removal(5, 0, 1), 0);
}

#[test]
fn an_inflight_source_reserves_an_empty_workspace() {
    let mut kit = Kit::empty(KitId(0), Default::default());
    assert!(kit.is_empty_workspace());
    assert!(kit.can_accept_source_load());

    kit.requested_path = Some(PathBuf::from("reach"));

    assert!(kit.is_empty_workspace());
    assert!(!kit.can_accept_source_load());
}

#[test]
fn releasing_a_failed_source_load_makes_the_workspace_reusable() {
    let mut kit = Kit::empty(KitId(0), Default::default());
    kit.requested_path = Some(PathBuf::from("reach"));
    kit.profile = Some(EditingKitProfileIdentity {
        id: "reach-profile".to_owned(),
        name: "Reach".to_owned(),
    });
    kit.restore.pending_launch_tags = Some(vec![PathBuf::from("objects/example.weapon")]);

    kit.release_source_load();

    assert!(kit.can_accept_source_load());
    assert!(kit.profile.is_none());
    assert!(kit.restore.pending_launch_tags.is_none());
}

#[test]
fn a_duplicate_pending_source_matches_its_reserved_workspace() {
    let mut kit = Kit::empty(KitId(0), Default::default());
    kit.requested_path = Some(PathBuf::from("reach"));

    assert!(super::requested_path_matches(&kit, Path::new("reach")));
    assert!(!super::requested_path_matches(
        &kit,
        Path::new("campaign-evolved")
    ));
}
