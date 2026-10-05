//! An open tag as a document: the tag, whether and how often it changed
//! since its last save, and its undo history; the edits that change it and
//! how they apply; and how typed text becomes a field value.

pub(crate) mod apply;
pub(crate) mod journal;
pub(crate) mod ops;
pub(crate) mod value;

use blam_tags::TagFile;

use journal::EditJournal;

/// Whether a document diverges from its last save, and how many times it has
/// been changed.
///
/// The count exists so anything that caches a document's serialized bytes can
/// tell "still the same tag" from "edited again" without serializing it. The
/// Campaign Evolved autosave used to re-serialize every dirty document twice a
/// second purely to discover nothing had changed — 100 ms a tick with a 105 MiB
/// animation graph open.
#[derive(Default)]
pub(crate) struct Dirty {
    set: bool,
    revision: u64,
}

impl Dirty {
    pub(crate) fn is_set(&self) -> bool {
        self.set
    }

    /// Monotonic across saves: clearing the flag must not let a later edit
    /// reuse a revision some cache has already seen.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn touch(&mut self) {
        self.set = true;
        self.revision = self.revision.wrapping_add(1);
    }

    pub(crate) fn clear(&mut self) {
        self.set = false;
    }
}

/// Parsed tag plus its unsaved-state and byte-snapshot edit history.
/// `dirty` reflects divergence from the last successful save, while journal
/// entries may still exist after saving to support later undo operations.
pub(crate) struct TagDocument {
    /// Process-unique identity. `dirty`'s revision restarts at zero when a tag
    /// is reloaded into a fresh document, so a cache keyed on the revision
    /// alone would take the reload for the tag it replaced.
    pub(crate) id: u64,
    pub(crate) tag: TagFile,
    pub(crate) dirty: Dirty,
    pub(crate) journal: EditJournal,
}

fn next_document_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl TagDocument {
    pub(crate) fn clean(tag: TagFile) -> Self {
        Self {
            id: next_document_id(),
            tag,
            dirty: Dirty::default(),
            journal: EditJournal::default(),
        }
    }

    /// A document that starts life already modified — used for tags that exist
    /// only in memory (a newly-created or imported container tag with no backing
    /// payload on disk or in a pak), so closing it prompts to save and it feeds
    /// Export Mod as a modified tag.
    pub(crate) fn modified(tag: TagFile) -> Self {
        let mut dirty = Dirty::default();
        dirty.touch();
        Self {
            id: next_document_id(),
            tag,
            dirty,
            journal: EditJournal::default(),
        }
    }

    /// Changes whenever this document's contents may have: any edit, undo or
    /// redo, or the document being replaced by a reload.
    pub(crate) fn content_stamp(&self) -> (u64, u64) {
        (self.id, self.dirty.revision())
    }
}
