//! Changing which tags exist and where: New Tag, renaming and moving tags and
//! folders with their references rewritten, duplicating, deleting, and folders
//! inside a Campaign Evolved container.

use super::*;

pub(in crate::app) mod delete;
pub(in crate::app) mod duplicate;
pub(in crate::app) mod rename_in_place;
pub(in crate::app) mod folder_rename;
pub(in crate::app) use folder_rename::sibling_differing_in_case;
pub(in crate::app) mod container_folders;
pub(in crate::app) mod new_tag_window;
pub(in crate::app) mod rename_tag_window;
pub(in crate::app) mod delete_confirm;
pub(in crate::app) mod container_duplicate_confirm;
pub(in crate::app) mod container_folder_window;
pub(in crate::app) mod loose_folder_rename_window;
pub(in crate::app) mod new_tag;
pub(in crate::app) use new_tag::*;
pub(in crate::app) mod refactor;
pub(in crate::app) use refactor::*;
pub(in crate::app) mod group_report;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;

/// Tag operations: New Tag, rename, folder rename and refactor, container
/// folders, delete and duplicate, the operations running per workspace, and the
/// ledger of tags Baboon created.
pub(in crate::app) struct TagOpsFeature {
    pub(in crate::app) new_tag_open: bool,
    pub(in crate::app) new_tag_dialog: NewTagDialog,
    /// Mandatory confirmation for an in-place Campaign Evolved duplicate.
    pub(in crate::app) container_duplicate_confirm: Option<ContainerDuplicateConfirm>,
    /// Kit ids with an in-place duplicate worker currently running.
    pub(in crate::app) container_duplicate_running: HashSet<KitId>,
    /// Kit ids with an in-place rename worker currently running. Mutually
    /// exclusive with the duplicate and delete sets for the same reason they are
    /// with each other: two writers on one `.utoc` race, and each invalidates
    /// the archive handle the other validates against.
    pub(in crate::app) container_rename_running: HashSet<KitId>,
    /// Mandatory confirmation for deleting a tag.
    pub(in crate::app) delete_confirm: Option<DeleteConfirm>,
    /// Kit ids with an in-place delete worker currently running.
    pub(in crate::app) container_delete_running: HashSet<KitId>,
    /// Every Campaign Evolved tag this installation created by duplicating
    /// another. Deletion is limited to what is recorded here, because a copy is
    /// otherwise indistinguishable from a tag the game shipped.
    pub(in crate::app) created_tags: CreatedTagLedger,
    pub(in crate::app) rename_tag: Option<RenameTagState>,
    /// Rename Folder dialog for a loose tags folder, if one is open.
    pub(in crate::app) loose_folder_rename: Option<LooseFolderRenameState>,
    /// New/Rename Folder dialog for a container source, if one is open.
    pub(in crate::app) container_folder_dialog: Option<ContainerFolderDialog>,
    pub(in crate::app) folder_refactor: Option<FolderRefactorUiState>,
}
