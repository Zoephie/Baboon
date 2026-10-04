use super::*;

fn node(rel_path: &str, label: &str) -> TagTreeNode {
    TagTreeNode {
        label: label.to_owned(),
        rel_path: PathBuf::from(rel_path),
        children: Vec::new(),
        children_loaded: false,
        entries: Vec::new(),
        entries_loaded: false,
        pending: false,
    }
}

/// A header is marked modified from the set's own record of ancestors,
/// without walking its subtree (which also found nothing in a folder whose
/// contents had not been loaded).
#[test]
fn folder_and_group_headers_know_they_hold_an_edit() {
    let mut modified = ModifiedTags::default();
    modified.insert(&TagEntry {
        key: "file:rifle".to_owned(),
        display_path: "objects/Weapons/rifle.weapon".to_owned(),
        group_tag: u32::from_be_bytes(*b"weap"),
        group_name: Some("weapon".to_owned()),
        location: TagEntryLocation::LooseFile(PathBuf::from("rifle.weapon")),
    });

    assert!(modified.subtree_has_modified(&node("objects", "objects")));
    assert!(
        modified.subtree_has_modified(&node("objects/weapons", "weapons")),
        "unloaded, any case"
    );
    assert!(
        modified.subtree_has_modified(&node("weapon weap", "weapon weap")),
        "its Groups node"
    );
    assert!(!modified.subtree_has_modified(&node("levels", "levels")));
    assert!(
        !modified.subtree_has_modified(&node("weapons", "weapons")),
        "not a same-named folder elsewhere"
    );
    assert!(!ModifiedTags::default().subtree_has_modified(&node("objects", "objects")));
}
