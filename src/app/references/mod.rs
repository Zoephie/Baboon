//! References: the reverse-dependency index, the content explorer, reference
//! jumps, and fixing a tag's dependencies.

use super::*;

pub(in crate::app) mod index;
pub(in crate::app) use index::*;
pub(in crate::app) mod dependencies;
pub(in crate::app) use dependencies::*;
pub(in crate::app) mod ref_jump;
pub(in crate::app) mod explorer;

/// References: the content explorer, reference jumps waiting or loading, field
/// navigation, and a referenced tag waiting to open.
pub(in crate::app) struct ReferencesFeature {
    pub(in crate::app) content_explorer: Option<ContentExplorer>,
    /// A reference-jump awaiting its referrer tag to finish loading before we
    /// can walk it to locate the exact referencing field. Set from the
    /// "References to X" popup; drained by `apply_field_nav`.
    pub(in crate::app) pending_ref_jump: Option<PendingRefJump>,
    /// Active reference-jump navigation: force ancestor blocks open and glow the
    /// exact referencing field until its glow window expires.
    pub(in crate::app) field_nav: Option<FieldNav>,
    /// Which referrer rows in the "References to X" popup are expanded to show
    /// their per-occurrence list. Keyed by row index; reset per references query.
    pub(in crate::app) ref_jump_expanded: HashSet<usize>,
    /// Lazily-computed occurrences per expanded referrer row. A present-but-empty
    /// vec means "walked, none found"; absence means "not yet walked (loading)".
    pub(in crate::app) ref_jump_occurrences: HashMap<usize, Vec<RefOccurrence>>,
    /// Referrer rows whose occurrences a worker is computing. The tag is read
    /// and walked off the UI thread and never cached as a document: it is not
    /// open, so there is no tab to keep it for.
    pub(in crate::app) ref_jump_loading: HashSet<usize>,
    /// Pending "open referenced tag in a new tab" request.
    pub(in crate::app) pending_open: Option<OpenTagRequest>,
}
