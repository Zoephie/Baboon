//! The container-dump confirmation's state.

use super::*;

/// Mandatory confirmation for extracting shipped tags out of a container set.
///
/// Not destructive, but expensive enough to be worth a deliberate answer: it is
/// thousands of decompressed reads and file writes, and the user has to know
/// that before it starts rather than three minutes into a frozen-looking
/// window. Carries its workspace like the other modeless confirms.
pub(in crate::app) struct ContainerDumpConfirm {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) output: PathBuf,
    pub(in crate::app) total: usize,
    pub(in crate::app) scope: ContainerDumpScope,
}

/// Which shipped tags an extraction covers.
///
/// The scope is captured when the confirmation is raised rather than re-derived
/// when it is accepted. A modeless confirm outlives the frame that opened it, and
/// a folder's membership is a snapshot of the tree the user right-clicked: making
/// the run re-walk the workspace on accept would let an import or a delete in
/// between silently change what gets written.
#[derive(Clone)]
pub(in crate::app) enum ContainerDumpScope {
    /// Every `Container` entry in the workspace — the File menu action.
    AllShipped,
    /// Just the tags beneath one browser folder, captured at right-click.
    /// `label` is the folder's display path, for the confirmation wording.
    Folder { label: String, keys: Vec<String> },
}
