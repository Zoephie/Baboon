use super::*;
use crate::app::browser::{BITMAP_LIBRARY_KEY, MODEL_LIBRARY_KEY};

fn unique_temp_dir(name: &str) -> PathBuf {
    crate::test_kits::unique_temp_path(name)
}

#[test]
fn detection_reads_only_the_folders_that_exist() {
    let root = unique_temp_dir("blam-scan");
    std::fs::create_dir_all(root.join("render")).unwrap();
    std::fs::create_dir_all(root.join("physics")).unwrap();
    // A stray *file* named like a source folder is not a source folder.
    std::fs::write(root.join("collision"), b"not a folder").unwrap();

    let scan = detect_blam_folders(&root);
    assert_eq!(
        scan,
        BlamFolderScan {
            render: true,
            collision: false,
            physics: true,
            structure: false,
        }
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_blam_pane_key_cannot_collide_with_a_tag_key() {
    assert!(BLAM_KEY.starts_with("tool:"));
    for prefix in ["cache:", "ublock:"] {
        assert!(!BLAM_KEY.starts_with(prefix));
    }
    assert_ne!(BLAM_KEY, BITMAP_LIBRARY_KEY);
    assert_ne!(BLAM_KEY, MODEL_LIBRARY_KEY);
}

#[test]
fn rescan_seeds_ticks_from_the_scan() {
    let root = unique_temp_dir("blam-rescan");
    std::fs::create_dir_all(root.join("collision")).unwrap();

    let mut state = BlamUiState::default();
    state.import_render = true;
    state.asset_path = "objects\\test".to_owned();
    state.rescan(&root);

    assert!(!state.import_render, "missing render folder must untick");
    assert!(state.import_collision);
    assert!(!state.import_physics);
    assert!(!state.import_structure);
    assert_eq!(state.scanned_path.as_deref(), Some("objects\\test"));
    std::fs::remove_dir_all(&root).unwrap();
}
