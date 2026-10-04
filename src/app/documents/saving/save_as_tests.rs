use std::path::{Path, PathBuf};

use super::*;

fn write_classic_ce_tag(path: &Path, group: &[u8; 4]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut bytes = [0u8; 64];
    bytes[36..40].copy_from_slice(group);
    bytes[60..64].copy_from_slice(b"blam");
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn save_as_registers_classic_ce_copy_in_loaded_folder() {
    let root = crate::test_kits::unique_temp_path("save-as-register-ce");
    let old_path = root.join("objects").join("old").join("old.gbxmodel");
    write_classic_ce_tag(&old_path, b"mod2");
    std::fs::create_dir_all(root.join("objects")).unwrap();

    let names = TagNameIndex::default();
    let old_entry = loose_file_entry(&root, &old_path, &names)
        .unwrap()
        .expect("old CE tag should probe");
    let entries = vec![old_entry.clone()];
    let mut source = LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: root.clone(),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names,
        game: None,
        entries: entries.clone(),
        tree: crate::core::source::build_folder_directory_tree(&root).unwrap(),
        group_tree: crate::core::source::build_group_tree(&entries),
        all_entries: entries,
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    };

    let saved_path = root.join("saved").join("cyborg.gbxmodel");
    write_classic_ce_tag(&saved_path, b"mod2");

    let registered = register_saved_copy_in_loaded_source(&mut source, &saved_path).unwrap();
    // The key the folder scan gives the copy. The root is a temp folder,
    // which is not canonical on macOS (/var is /private/var), and keying the
    // copy off canonical paths gave it a key the scan never makes.
    let scanned_key = loose_file_entry(&root, &saved_path, &TagNameIndex::default())
        .unwrap()
        .unwrap()
        .key;

    let _ = std::fs::remove_dir_all(&root);
    assert!(registered);
    assert!(
        source.entries.iter().any(|entry| entry.key == scanned_key),
        "the copy is keyed like the folder scan"
    );
    assert!(
        source
            .tree
            .children
            .iter()
            .any(|node| node.label == "saved")
    );
    assert!(source.entries.iter().any(|entry| {
        entry.display_path == "saved/cyborg.gbxmodel"
            && entry.group_tag == u32::from_be_bytes(*b"mod2")
    }));
    assert!(source.all_entries.iter().any(|entry| {
        entry.display_path == "saved/cyborg.gbxmodel"
            && entry.group_tag == u32::from_be_bytes(*b"mod2")
    }));
    assert!(source.group_tree.children.iter().any(|node| {
        node.entries
            .iter()
            .any(|&index| source.all_entries[index].display_path == "saved/cyborg.gbxmodel")
    }));
}
