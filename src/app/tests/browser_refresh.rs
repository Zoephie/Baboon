use super::*;

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
