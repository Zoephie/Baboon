//! Tag Compare's and Git Review's state: the diff, its filters, and Git
//! history.

use super::*;

#[derive(Debug, Clone)]
pub(in crate::app) struct TagFieldDiff {
    /// Path in the edited tag.
    pub(in crate::app) path: String,
    /// Path in the tag as shipped, when it differs -- deleting an element
    /// shifts every index below it, so the same field lives at two paths and a
    /// side-by-side view needs both to find it.
    pub(in crate::app) base_path: Option<String>,
    pub(in crate::app) a: String,
    pub(in crate::app) b: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum TagCompareSource {
    OpenTag,
    File,
    EditingKit,
    GitHead,
    GitHistory,
}

pub(in crate::app) struct GitHistoryCommit {
    pub(in crate::app) hash: String,
    pub(in crate::app) short_hash: String,
    pub(in crate::app) date: String,
    pub(in crate::app) author: String,
    pub(in crate::app) subject: String,
}

#[derive(Default)]
pub(in crate::app) struct GitHistoryState {
    pub(in crate::app) commits: Vec<GitHistoryCommit>,
    pub(in crate::app) selected: Option<String>,
    pub(in crate::app) has_more: bool,
    pub(in crate::app) loaded: bool,
    pub(in crate::app) error: Option<String>,
}

#[derive(Clone, Copy)]
pub(in crate::app) struct TagDiffFilters {
    pub(in crate::app) both: bool,
    pub(in crate::app) current_only: bool,
    pub(in crate::app) comparison_only: bool,
}

impl Default for TagDiffFilters {
    fn default() -> Self {
        Self {
            both: true,
            current_only: true,
            comparison_only: true,
        }
    }
}

/// The launch tag stays fixed while the comparison source and tag are chosen.
pub(in crate::app) struct TagDiffState {
    /// Kit containing the current tag.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) a_key: String,
    pub(in crate::app) source: TagCompareSource,
    /// Loaded kit owning the selected open comparison tag.
    pub(in crate::app) b_kit: Option<KitId>,
    /// Open-tab key of the comparison tag.
    pub(in crate::app) b_key: Option<String>,
    /// File selected with Browse.
    pub(in crate::app) b_path: Option<PathBuf>,
    /// Tags root of the selected configured editing kit.
    pub(in crate::app) comparison_kit_root: Option<PathBuf>,
    pub(in crate::app) git_history: GitHistoryState,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) filters: TagDiffFilters,
    /// Reverses which tag is on the left in the results table.
    pub(in crate::app) swapped: bool,
    pub(in crate::app) results: Option<TagDiffResults>,
    /// The Git read this window is waiting on, if any.
    pub(in crate::app) git_pending: Option<u64>,
}

#[derive(Clone)]
pub(in crate::app) struct TagDiffResults {
    pub(in crate::app) diffs: Vec<TagFieldDiff>,
    /// True when the diff hit the cap and more differences exist.
    pub(in crate::app) truncated: bool,
    pub(in crate::app) reverse_diffs: Vec<TagFieldDiff>,
    pub(in crate::app) reverse_truncated: bool,
}
