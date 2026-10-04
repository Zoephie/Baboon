//! Open tags as documents: selecting and loading them, saving, undo and redo,
//! and closing tabs or the app with the save-changes prompt.

use super::*;
use crate::core::document::value::unsaveable_reason;
use crate::app::editor::{ColorPopupWindow, DeferredFileAction, FunctionPopupWindow, format_byte_count};
use crate::app::export::ContainerDumpReport;
use crate::app::browser::is_folder_pane_key;

pub(in crate::app) mod selection;
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
}

/// What the document windows can be asked to do.
pub(in crate::app) enum DocumentsCommand {
    /// Carry out the save-changes prompt's answer.
    SaveChangesPrompt(SaveChangesPromptAction),
}

impl Baboon {
    pub(in crate::app) fn apply_documents_command(&mut self, command: DocumentsCommand, ctx: &egui::Context) {
        match command {
            DocumentsCommand::SaveChangesPrompt(action) => self.apply_save_changes_prompt_action(action, ctx),
        }
    }
}
