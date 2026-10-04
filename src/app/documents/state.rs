//! The close and save prompts' state: what is waiting to close and what is
//! dirty.

use super::*;

#[derive(Clone, Debug)]
/// Close transaction retained while the save/discard prompt spans UI frames.
/// Key-bearing variants use the same stable keys as `parsed_tags` and tabs.
pub(in crate::app) enum PendingCloseAction {
    CloseApp,
    CloseTab(String),
    CloseAllTabs,
    CloseAllButThis(String),
    /// Close a whole kit, discarding its documents and caches.
    CloseKit(KitId),
}

pub(in crate::app) struct DirtyTagEntry {
    pub(in crate::app) path: String,
    pub(in crate::app) tag_id: String,
    pub(in crate::app) checked: bool,
}

/// Confirmation shared by the Chimp toolbar discard action and the close
/// transaction. `pending_action` is set only when discarding is part of an
/// app/kit close; a toolbar discard has no continuation.
pub(in crate::app) struct ChimpDiscardPrompt {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) packages: Vec<String>,
    pub(in crate::app) pending_action: Option<PendingCloseAction>,
    pub(in crate::app) error: Option<String>,
}

/// Foundation-style confirmation shown when a close action would discard
/// edited tags. `allow_app_close_once` is set only after the user confirms an
/// app exit; the next native close request is then allowed through instead of
/// being vetoed and prompting again.
pub(in crate::app) struct SaveChangesPrompt {
    pub(in crate::app) visible: bool,
    /// Whether this workspace can hold edits in a Baboon project rather than
    /// writing them into the game. Container sources can; a loose kit has
    /// nowhere to stash to, so it is offered Save or nothing.
    pub(in crate::app) can_stash: bool,
    pub(in crate::app) dirty_tags: Vec<DirtyTagEntry>,
    pub(in crate::app) pending_action: PendingCloseAction,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) allow_app_close_once: bool,
    /// Where discarding would delete from, and how many of the listed tags have
    /// a stashed copy there. Discarding on a stashing workspace is not "close
    /// without writing anything" — it deletes rows out of a file that persists
    /// across sessions — so the prompt has to be able to name both.
    pub(in crate::app) stash_file: Option<PathBuf>,
    pub(in crate::app) stashed: usize,
    /// Set by the first click on Discard. The second click is the one that acts,
    /// which is what keeps a one-click "no thanks" on a quit dialog from
    /// deleting work the user believed was already exported.
    pub(in crate::app) confirm_discard: bool,
}

impl Default for SaveChangesPrompt {
    fn default() -> Self {
        Self {
            visible: false,
            can_stash: false,
            dirty_tags: Vec::new(),
            pending_action: PendingCloseAction::CloseApp,
            error: None,
            allow_app_close_once: false,
            stash_file: None,
            stashed: 0,
            confirm_discard: false,
        }
    }
}
