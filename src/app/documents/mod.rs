//! Open tags as documents: selecting and loading them, saving, undo and redo,
//! and closing tabs or the app with the save-changes prompt.

use super::*;
use crate::app::shell::session::{clear_last_session, save_last_session};
use crate::app::shell::{OperationNotice, WorkerMessage, spawn_worker};
use crate::app::kits::{
    KitId, KitStamp, TerminalLineEntry, document_edits_are_saveable, is_render_method_layout_group,
    tag_tree_id, trim_terminal_lines,
};
use crate::app::tag_ops::NewTagGroup;
use crate::app::browser::tag_file_name;
use crate::app::mods::OverwriteConfirm;
use crate::app::references::collect_tag_dependency_refs;
use crate::app::chimp::ChimpLevelPhase;
use crate::core::document::value::unsaveable_reason;
use crate::app::editor::{ColorPopupWindow, DeferredFileAction, FunctionPopupWindow, format_byte_count};
use crate::app::export::ContainerDumpReport;

pub(in crate::app) mod selection;
pub(in crate::app) mod disk_changes;
pub(in crate::app) mod disk_save_confirm;
pub(in crate::app) use disk_save_confirm::DiskSaveConfirm;
pub(in crate::app) use disk_changes::{DiskChoice, DiskNotice, DiskReload, DiskWatch, OpenTagCheck};
pub(in crate::app) mod saving;
pub(in crate::app) use saving::ordered_unique_keys;
pub(in crate::app) mod close;
pub(in crate::app) use close::SaveChangesPromptAction;
pub(in crate::app) mod undo;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;

/// Documents: the close in progress. Its save-changes prompt is a dialog in
/// the host.
pub(in crate::app) struct DocumentsFeature {
    /// Let the next request to close the app through without the prompt: set
    /// once the prompt has been answered and the app is closing for real. See
    /// [`Baboon::handle_app_close_request`].
    pub(in crate::app) allow_app_close_once: bool,
    /// Whether the window had focus last frame, so regaining it can be told
    /// apart: that is when open tags are checked for changes on disk.
    pub(in crate::app) window_focused: bool,
}

/// What the document windows can be asked to do.
pub(in crate::app) enum DocumentsCommand {
    /// Carry out the save-changes prompt's answer.
    SaveChangesPrompt(SaveChangesPromptAction),
    /// Answer a tag's changed-on-disk banner.
    DiskChoice { kit: KitId, key: String, choice: DiskChoice },
}

impl Baboon {
    pub(in crate::app) fn apply_documents_command(&mut self, command: DocumentsCommand, ctx: &egui::Context) {
        match command {
            DocumentsCommand::SaveChangesPrompt(action) => self.apply_save_changes_prompt_action(action, ctx),
            DocumentsCommand::DiskChoice { kit, key, choice } => self.apply_disk_choice(kit, key, choice, ctx),
        }
    }
}
