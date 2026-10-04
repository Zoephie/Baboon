//! Changing which tags exist and where: New Tag, renaming and moving tags and
//! folders with their references rewritten, duplicating, deleting, and folders
//! inside a Campaign Evolved container.

use super::*;
use crate::app::editor::{
    H2TemplateCache, combo_box_with_scroll, combo_scroll_next_index, lost_focus_once,
};
use crate::app::browser::{
    ContainerFolderDialog, LooseFolderRenameState, RenameTagState, TagNameOperation,
};

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
    /// Kit ids with an in-place duplicate worker currently running.
    pub(in crate::app) container_duplicate_running: HashSet<KitId>,
    /// Kit ids with an in-place rename worker currently running. Mutually
    /// exclusive with the duplicate and delete sets for the same reason they are
    /// with each other: two writers on one `.utoc` race, and each invalidates
    /// the archive handle the other validates against.
    pub(in crate::app) container_rename_running: HashSet<KitId>,
    /// Kit ids with an in-place delete worker currently running.
    pub(in crate::app) container_delete_running: HashSet<KitId>,
    /// Every Campaign Evolved tag this installation created by duplicating
    /// another. Deletion is limited to what is recorded here, because a copy is
    /// otherwise indistinguishable from a tag the game shipped.
    pub(in crate::app) created_tags: CreatedTagLedger,
    pub(in crate::app) folder_refactor: Option<FolderRefactorUiState>,
}

/// What the tag operation windows commit to. Each acts on the state its
/// window holds, which stays put until the handler has used it.
pub(in crate::app) enum TagOpsCommand {
    /// Delete the tag the delete confirmation names.
    Delete,
    /// Make or rename the folder in the container folder dialog. The dialog
    /// closes only when the name is accepted; a rejection keeps it up with
    /// its reason attached.
    ApplyContainerFolder,
    /// Rename or move the loose folder in its dialog, which closes once the
    /// job has started and otherwise stays up with the reason.
    ApplyLooseFolderRename,
    /// Rename the tag in the rename dialog, which closes on success and
    /// otherwise stays up with a status message.
    Rename,
    /// Create the tag the New Tag window describes.
    CreateNewTag,
    /// Duplicate the container tag at `key` in `kit` as `destination_leaf`.
    DuplicateContainerTag {
        kit: KitId,
        key: String,
        destination_leaf: String,
    },
}

impl Baboon {
    pub(in crate::app) fn apply_tag_ops_command(&mut self, command: TagOpsCommand, ctx: &egui::Context) {
        match command {
            TagOpsCommand::Delete => self.begin_delete_tag(ctx.clone()),
            TagOpsCommand::ApplyContainerFolder => {
                if self.apply_container_folder_dialog() {
                    self.dialogs.close::<ContainerFolderDialog>();
                }
            }
            TagOpsCommand::ApplyLooseFolderRename => {
                if self.apply_loose_folder_rename() {
                    self.dialogs.close::<LooseFolderRenameState>();
                }
            }
            TagOpsCommand::Rename => self.begin_rename_tag(&ctx),
            TagOpsCommand::CreateNewTag => self.create_new_tag(),
            TagOpsCommand::DuplicateContainerTag {
                kit,
                key,
                destination_leaf,
            } => self.start_container_duplicate(kit, key, destination_leaf, ctx.clone()),
        }
    }
}
