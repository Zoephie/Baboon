//! Right-click → Extract tags to folder, on a Campaign Evolved container.
//!
//! The narrow-scope twin of File → Extract All Tags to Folder. It shares that
//! action's worker, progress bar and cancel, so the only thing that is actually
//! new is *which* entries reach the worker — which is what these gates cover.
//!
//! Both halves are tested against a case where the answers differ. A folder
//! scope that quietly resolved to every tag in the workspace would still pass a
//! test whose fixture only holds one folder, so every fixture here holds two.

use super::*;

/// A mounted container tag, as the browser would have indexed it.
fn container_entry(path: &str) -> TagEntry {
    TagEntry {
        key: format!("ublock:pakchunk0:{path}"),
        display_path: format!("{path}.bitmap"),
        group_tag: u32::from_be_bytes(*b"bitm"),
        group_name: Some("bitmap".to_owned()),
        location: TagEntryLocation::Container {
            container: 0,
            rel_path: format!("Tags/{path}-bitmap.ubulk"),
        },
    }
}

/// A tag authored this session: in the tree, but with no shipped payload behind
/// it to read back out of the container.
fn new_container_entry(path: &str) -> TagEntry {
    TagEntry {
        key: format!("ublock:new:{path}"),
        display_path: format!("{path}.bitmap"),
        group_tag: u32::from_be_bytes(*b"bitm"),
        group_name: Some("bitmap".to_owned()),
        location: TagEntryLocation::NewContainer {
            template: NewContainerTemplate::Derived {
                group: "bitmap".to_owned(),
            },
            package: format!("/Game/Tags/{path}"),
            group_tag: u32::from_be_bytes(*b"bitm"),
        },
    }
}

fn keys_of(entries: &[TagEntry]) -> Vec<String> {
    entries.iter().map(|entry| entry.key.clone()).collect()
}

fn extracted_paths(entries: &[TagEntry], scope: &ContainerDumpScope) -> Vec<String> {
    container_dump_entries(entries, scope)
        .iter()
        .map(|entry| entry.display_path.clone())
        .collect()
}

fn folder(label: &str, keys: Vec<String>) -> ContainerDumpScope {
    ContainerDumpScope::Folder {
        label: label.to_owned(),
        keys,
    }
}

/// The point of the feature. The workspace holds two folders, so a scope that
/// ignored its keys and fell through to "everything" would fail here rather
/// than agreeing by accident.
#[test]
fn a_folder_scope_extracts_only_that_folder() {
    let entries = vec![
        container_entry("objects/characters/elite/elite"),
        container_entry("objects/characters/elite/elite_helmet"),
        container_entry("objects/vehicles/ghost/ghost"),
    ];
    let elite = keys_of(&entries[..2]);

    assert_eq!(
        extracted_paths(&entries, &folder("objects/characters/elite", elite)),
        vec![
            "objects/characters/elite/elite.bitmap",
            "objects/characters/elite/elite_helmet.bitmap",
        ],
    );
    // The same fixture, unscoped, reaches the third tag — so the assertion
    // above is a restriction and not a description of the whole workspace.
    assert_eq!(
        extracted_paths(&entries, &ContainerDumpScope::AllShipped).len(),
        3,
    );
}

/// The count in the menu label and the set handed to the worker come from the
/// same filter, so a folder holding an unsaved tag cannot promise a file that
/// never gets written. `dump_shipped_container_tags` skips such an entry
/// silently, which is exactly the kind of gap a count would paper over.
#[test]
fn a_tag_authored_this_session_is_not_extracted() {
    let entries = vec![
        container_entry("objects/characters/elite/elite"),
        new_container_entry("objects/characters/elite/elite_custom"),
    ];
    let both = keys_of(&entries);

    assert_eq!(
        extracted_paths(&entries, &folder("objects/characters/elite", both)),
        vec!["objects/characters/elite/elite.bitmap"],
    );
}

/// A folder whose every tag was authored this session resolves to nothing, and
/// the caller turns that into a status message instead of starting a run that
/// writes no files.
#[test]
fn a_folder_of_only_unsaved_tags_resolves_to_nothing() {
    let entries = vec![new_container_entry("objects/mine/thing")];
    let keys = keys_of(&entries);

    assert!(container_dump_entries(&entries, &folder("objects/mine", keys)).is_empty());
}

/// Keys are matched against the entries actually present, so a key for a tag
/// that has since been deleted drops out rather than aborting the run. The
/// captured scope outlives the frame that built it, and the workspace can move
/// under it.
#[test]
fn a_key_with_no_surviving_entry_is_dropped() {
    let entries = vec![container_entry("objects/characters/elite/elite")];
    let mut keys = keys_of(&entries);
    keys.push("ublock:pakchunk0:objects/characters/elite/deleted".to_owned());

    assert_eq!(
        extracted_paths(&entries, &folder("objects/characters/elite", keys)),
        vec!["objects/characters/elite/elite.bitmap"],
    );
}

/// A browser folder node holding `entry_indices`, with `children` beneath it.
fn node(label: &str, entry_indices: &[usize], children: Vec<TagTreeNode>) -> TagTreeNode {
    TagTreeNode {
        label: label.to_owned(),
        rel_path: std::path::PathBuf::from(label),
        children,
        children_loaded: true,
        entries: entry_indices.to_vec(),
        entries_loaded: true,
        pending: false,
    }
}

/// The other half of the honest count. `collect_container_tag_keys` is what the
/// menu label counts; `container_dump_entries` is what the worker writes. If the
/// collector let an authored tag through, the label would promise a file that
/// the run then silently skips — the count would be wrong even though every
/// controller-side gate above still passed.
#[test]
fn the_menu_count_excludes_a_tag_authored_this_session() {
    let entries = vec![
        container_entry("objects/characters/elite/elite"),
        new_container_entry("objects/characters/elite/elite_custom"),
    ];
    let folder = node("objects/characters/elite", &[0, 1], Vec::new());

    let collected = crate::app::browser::collect_container_tag_keys(&folder, &entries);
    assert_eq!(collected, vec![entries[0].key.clone()]);
    // The count the menu shows and the set the worker writes agree, which is the
    // property the two functions exist to keep.
    assert_eq!(
        collected.len(),
        container_dump_entries(&entries, &folder_scope_of(&collected)).len(),
    );
}

/// Subfolders are included, so right-clicking `objects` covers everything under
/// it rather than only the tags sitting directly in it.
#[test]
fn the_menu_count_descends_into_subfolders() {
    let entries = vec![
        container_entry("objects/loose"),
        container_entry("objects/characters/elite/elite"),
        container_entry("objects/characters/elite/elite_helmet"),
    ];
    let tree = node(
        "objects",
        &[0],
        vec![node("objects/characters", &[], vec![node("objects/characters/elite", &[1, 2], Vec::new())])],
    );

    assert_eq!(
        crate::app::browser::collect_container_tag_keys(&tree, &entries).len(),
        3,
    );
}

fn folder_scope_of(keys: &[String]) -> ContainerDumpScope {
    folder("objects/characters/elite", keys.to_vec())
}

/// The all-shipped scope is unchanged by the folder work: it still covers every
/// container entry and still excludes authored ones.
#[test]
fn the_all_shipped_scope_still_covers_every_container_tag() {
    let entries = vec![
        container_entry("objects/characters/elite/elite"),
        container_entry("objects/vehicles/ghost/ghost"),
        new_container_entry("objects/mine/thing"),
    ];

    assert_eq!(
        extracted_paths(&entries, &ContainerDumpScope::AllShipped),
        vec![
            "objects/characters/elite/elite.bitmap",
            "objects/vehicles/ghost/ghost.bitmap",
        ],
    );
}
