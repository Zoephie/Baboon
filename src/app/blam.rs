//! Blam! import-pipeline panel state and asset-folder detection.
//! It owns this focused support concern; the panel's presentation lives in `ui/blam.rs` and the import pipeline itself will live in `blam-tags`.

use super::*;

/// Which of the conventional tool source folders exist under the asset's data
/// folder. Each maps to one importable tag group: `render` (render_model),
/// `collision` (collision_model), `physics` (physics_model), and `structure`
/// (structure_bsp).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct BlamFolderScan {
    pub(super) render: bool,
    pub(super) collision: bool,
    pub(super) physics: bool,
    pub(super) structure: bool,
}

pub(super) fn detect_blam_folders(asset_folder: &Path) -> BlamFolderScan {
    BlamFolderScan {
        render: asset_folder.join("render").is_dir(),
        collision: asset_folder.join("collision").is_dir(),
        physics: asset_folder.join("physics").is_dir(),
        structure: asset_folder.join("structure").is_dir(),
    }
}

/// The pane key the Blam! import panel occupies in a kit's `tag_tree`.
///
/// Like [`BITMAP_LIBRARY_KEY`], a `tool:`-prefixed key no tag can have: the
/// pane rides the tag-tile layout — draggable, splittable, resizable — while
/// resolving to no document, so the close path finds nothing dirty and the
/// session writer skips it.
pub(in crate::app) const BLAM_KEY: &str = "tool:blam";

pub(in crate::app) const BLAM_TITLE: &str = "Blam!";

pub(super) struct BlamUiState {
    /// Asset folder relative to the kit's `data` folder, backslash form, the
    /// same shape tool commands take.
    pub(super) asset_path: String,
    /// The asset path the folder scan last ran against. `None` forces a rescan
    /// on the next frame the panel draws, so ticks follow the typed path
    /// without touching the disk every frame.
    pub(super) scanned_path: Option<String>,
    pub(super) scan: BlamFolderScan,
    pub(super) import_render: bool,
    pub(super) import_collision: bool,
    pub(super) import_physics: bool,
    pub(super) import_structure: bool,
    pub(super) import_prt: bool,
    /// Shown in the panel's status bar. Display-only until the blam-tags
    /// pipeline lands and reports real progress.
    pub(super) status: String,
}

impl Default for BlamUiState {
    fn default() -> Self {
        Self {
            asset_path: String::new(),
            scanned_path: None,
            scan: BlamFolderScan::default(),
            import_render: false,
            import_collision: false,
            import_physics: false,
            import_structure: false,
            // PRT rides along with a render import whenever one runs.
            import_prt: true,
            status: "Ready".to_owned(),
        }
    }
}

impl BlamUiState {
    /// Re-detect the asset's source folders and re-seed the tick boxes from
    /// what is actually on disk: a present folder starts ticked, a missing one
    /// is unticked (and the panel disables it).
    pub(super) fn rescan(&mut self, asset_folder: &Path) {
        self.scan = detect_blam_folders(asset_folder);
        self.import_render = self.scan.render;
        self.import_collision = self.scan.collision;
        self.import_physics = self.scan.physics;
        self.import_structure = self.scan.structure;
        self.scanned_path = Some(self.asset_path.trim().to_owned());
    }

    pub(super) fn anything_selected(&self) -> bool {
        self.import_render || self.import_collision || self.import_physics || self.import_structure
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        std::env::temp_dir().join(format!("baboon-{name}-{}-{nanos}", std::process::id()))
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
}
