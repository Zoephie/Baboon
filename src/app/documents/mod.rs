//! Open tags as documents: selecting and loading them, saving, undo and redo,
//! and closing tabs or the app with the save-changes prompt.

use super::*;

pub(in crate::app) mod selection;
pub(in crate::app) mod saving;
pub(in crate::app) use saving::ordered_unique_keys;
pub(in crate::app) mod close;
pub(in crate::app) use close::{SaveChangesPromptAction, draw_save_changes_prompt};
pub(in crate::app) mod undo;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;

/// Documents: the save-changes prompt.
pub(in crate::app) struct DocumentsFeature {
    /// Modal close transaction; the pending action is executed only after every
    /// selected dirty document has been saved or discard is confirmed.
    pub(in crate::app) save_changes_prompt: SaveChangesPrompt,
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
