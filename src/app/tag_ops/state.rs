//! The tag operations' dialog state: New Tag, delete and duplicate.

use super::*;

/// Mandatory confirmation for an in-place Campaign Evolved package duplicate.
/// The destination is deliberately only a leaf: its parent, group, extension,
/// and providing container are all captured from the source entry.
pub(in crate::app) struct ContainerDuplicateConfirm {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) key: String,
    pub(in crate::app) destination_leaf: String,
}

/// Which kind of storage a pending delete will act on. The wording and the
/// warnings differ completely: one moves a file, the other rewrites a pak.
pub(in crate::app) enum DeleteKind {
    Loose,
    Container {
        /// The exact pack that will be rewritten, mod or shipped, with its path.
        target_label: String,
    },
}

/// Mandatory confirmation for deleting a tag.
pub(in crate::app) struct DeleteConfirm {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) key: String,
    pub(in crate::app) display_path: String,
    pub(in crate::app) kind: DeleteKind,
    /// Display paths of the tags that point at this one and will be left
    /// dangling.
    pub(in crate::app) referrers: Vec<String>,
    /// True when no reverse-dependency index was available, so the referrer list
    /// says nothing either way.
    pub(in crate::app) referrers_unavailable: bool,
    pub(in crate::app) has_unsaved_edits: bool,
}

/// Paths of the immutable pre-mutation backup kept beside the target UTOC.
#[derive(Clone, Debug)]
pub(in crate::app) struct DuplicateBackupPaths {
    pub(in crate::app) utoc: PathBuf,
    pub(in crate::app) manifest: PathBuf,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct NewTagGroup {
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) name: String,
    pub(in crate::app) schema_path: PathBuf,
    pub(in crate::app) extension: String,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct NewTagDialog {
    /// Workspace the dialog was opened for. `None` before it is first
    /// opened, since this dialog is always resident rather than optional.
    pub(in crate::app) kit: Option<KitId>,
    pub(in crate::app) game: String,
    pub(in crate::app) rel_path: String,
    pub(in crate::app) output_path: Option<PathBuf>,
    pub(in crate::app) groups: Vec<NewTagGroup>,
    pub(in crate::app) selected_group: usize,
    pub(in crate::app) error: Option<String>,
    /// Whether the selected group can actually be created, and why not when it
    /// cannot. `None` for games other than Campaign Evolved, where the question
    /// does not arise: a loose tag is a file, not a package with a native class
    /// behind it.
    ///
    /// Cached rather than computed per frame because answering it parses the
    /// game's whole mapping table; it is refreshed when the group or the game
    /// changes, which is the only time it can move.
    pub(in crate::app) authorability: Option<(bool, String)>,
}

impl Default for NewTagDialog {
    fn default() -> Self {
        Self {
            kit: None,
            game: GameId::Halo3.as_str().to_owned(),
            rel_path: String::new(),
            output_path: None,
            groups: Vec::new(),
            selected_group: 0,
            error: None,
            authorability: None,
        }
    }
}
