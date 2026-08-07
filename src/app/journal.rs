//! Per-tag undo/redo journal.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.
//!
//! `TagFile` is not `Clone`, so snapshots are taken by serializing the tag to
//! bytes (`write_to_bytes`) and restored by re-parsing (`read_from_bytes`). A
//! snapshot is captured immediately *before* a mutating edit batch is applied,
//! so undo restores the exact pre-edit bytes regardless of which op kinds were
//! in the batch.
//!
//! Continuous edits (e.g. dragging a slider that commits every frame) are
//! coalesced into a single undo entry via [`EditJournal::begin_edit`] /
//! [`EditJournal::end_edit_window`]: the first frame of a run captures one
//! snapshot, later frames are skipped until a frame with no edits closes the
//! window.

use super::*;

/// One serialized tag state plus a human-readable label for the action.
///
/// The bytes are shared rather than owned: the session's recovery project
/// captures the history on every autosave tick, and a stack holding a couple of
/// Campaign Evolved animation graphs would otherwise be copied wholesale twice
/// a second.
#[derive(Clone)]
pub(super) struct Snapshot {
    pub(super) bytes: Arc<Vec<u8>>,
    pub(super) label: String,
}

pub(super) struct EditJournal {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    limit: usize,
    /// Ceiling on the bytes one stack may hold. A snapshot is a whole
    /// serialized tag, and Campaign Evolved ships a 105 MiB animation graph --
    /// 64 of those is 6.7 GB. Depth still applies; whichever bound is reached
    /// first drops the oldest entry.
    byte_limit: usize,
    /// True while a run of consecutive edit frames is being coalesced into the
    /// single snapshot already pushed for this run.
    coalescing: bool,
    /// Bumped whenever either stack changes.
    ///
    /// The session's recovery project rewrites its history table wholesale — an
    /// undo stack shifts by one on every edit, so nearly every row moves and
    /// there is nothing to reconcile row by row. Knowing cheaply that nothing
    /// changed is therefore the difference between this and rewriting tens of
    /// megabytes twice a second while someone drags a slider. Same trick as
    /// [`Dirty::revision`], and for the same reason: answering it by hashing
    /// the snapshots would only move the cost from the disk to the CPU.
    revision: u64,
}

impl Default for EditJournal {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: 64,
            byte_limit: 256 * 1024 * 1024,
            coalescing: false,
            revision: 0,
        }
    }
}

impl EditJournal {
    /// Capture a pre-edit snapshot before applying a batch. No-op while already
    /// coalescing a run, so a continuous drag yields a single undo entry.
    /// Clears the redo stack (a new edit invalidates any redo history).
    pub(super) fn begin_edit(&mut self, tag: &TagFile, label: &str) {
        if self.coalescing {
            return;
        }
        if let Ok(bytes) = tag.write_to_bytes() {
            self.push_capped(Snapshot {
                bytes: Arc::new(bytes),
                label: label.to_owned(),
            });
            self.redo.clear();
        }
        self.coalescing = true;
    }

    /// The stacks as they stand, for the session's recovery project. Oldest
    /// first, matching the in-memory order.
    pub(super) fn stacks(&self) -> (&[Snapshot], &[Snapshot]) {
        (&self.undo, &self.redo)
    }

    /// How many times either stack has changed. See [`EditJournal::revision`].
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    /// Seed a journal from a restored session.
    ///
    /// Replaces rather than merges: a freshly opened document has no history of
    /// its own, and restoring into one that somehow did would interleave two
    /// unrelated edit trails.
    pub(super) fn restore(&mut self, undo: Vec<Snapshot>, redo: Vec<Snapshot>) {
        self.undo = undo;
        self.redo = redo;
        self.coalescing = false;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Close the current coalescing window (call on a frame with no edits), so
    /// the next edit starts a fresh undo entry.
    pub(super) fn end_edit_window(&mut self) {
        self.coalescing = false;
    }

    pub(super) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(super) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Pop the most recent undo snapshot, recording `current` on the redo stack.
    /// Returns the bytes to restore and the action label.
    pub(super) fn undo(&mut self, current: &TagFile) -> Option<(Arc<Vec<u8>>, String)> {
        let snapshot = self.undo.pop()?;
        if let Ok(bytes) = current.write_to_bytes() {
            push_capped_into(
                &mut self.redo,
                self.limit,
                self.byte_limit,
                Snapshot {
                    bytes: Arc::new(bytes),
                    label: snapshot.label.clone(),
                },
            );
        }
        self.coalescing = false;
        self.revision = self.revision.wrapping_add(1);
        Some((snapshot.bytes, snapshot.label))
    }

    /// Pop the most recent redo snapshot, recording `current` on the undo stack.
    pub(super) fn redo(&mut self, current: &TagFile) -> Option<(Arc<Vec<u8>>, String)> {
        let snapshot = self.redo.pop()?;
        if let Ok(bytes) = current.write_to_bytes() {
            push_capped_into(
                &mut self.undo,
                self.limit,
                self.byte_limit,
                Snapshot {
                    bytes: Arc::new(bytes),
                    label: snapshot.label.clone(),
                },
            );
        }
        self.coalescing = false;
        self.revision = self.revision.wrapping_add(1);
        Some((snapshot.bytes, snapshot.label))
    }

    fn push_capped(&mut self, snapshot: Snapshot) {
        push_capped_into(&mut self.undo, self.limit, self.byte_limit, snapshot);
        self.revision = self.revision.wrapping_add(1);
    }
}

fn push_capped_into(stack: &mut Vec<Snapshot>, limit: usize, byte_limit: usize, snapshot: Snapshot,) {
    stack.push(snapshot);
    // Always keep the newest entry, even on its own over budget: dropping it
    // would mean an edit that cannot be undone at all.
    while stack.len() > 1
        && (stack.len() > limit || stack.iter().map(|s| s.bytes.len()).sum::<usize>() > byte_limit)
    {
        stack.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_model() -> TagFile {
        TagFile::new("definitions/halo2_mcc/model.json").unwrap()
    }

    fn add_variant(tag: &mut TagFile) {
        let mut dirty = Dirty::default();
        apply_model_variant_ops(
            tag,
            vec![ModelVariantOp::Create {
                name: "test".to_owned(),
                regions: vec![ModelVariantRegionChoice {
                    region_name: "body".to_owned(),
                    permutation_name: "default".to_owned(),
                }],
            }],
            &mut dirty,
        );
    }

    /// A snapshot is a whole serialized tag, and Campaign Evolved ships a
    /// 105 MiB animation graph. Depth alone let the journal reach gigabytes, so
    /// the byte budget evicts first -- but never the newest entry, or the edit
    /// just made could not be undone.
    #[test]
    fn the_journal_evicts_on_bytes_before_depth() {
        let mut stack = Vec::new();
        let budget = 1000;
        for i in 0..5 {
            push_capped_into(
                &mut stack,
                64,
                budget,
                Snapshot { bytes: Arc::new(vec![0; 400]), label: format!("edit {i}"), },
            );
        }
        assert_eq!(stack.len(), 2, "400 x 2 fits in 1000, 400 x 3 does not");
        assert_eq!(stack.last().unwrap().label, "edit 4", "the newest survives");

        // One entry over budget on its own is still kept: losing it would mean
        // an edit with no way back.
        let mut lone = Vec::new();
        push_capped_into(
            &mut lone,
            64,
            budget,
            Snapshot { bytes: Arc::new(vec![0; budget * 4]), label: "huge".to_owned(), },
        );
        assert_eq!(lone.len(), 1);
    }

    #[test]
    fn undo_then_redo_round_trips_exact_bytes() {
        let mut tag = fresh_model();
        let original = tag.write_to_bytes().unwrap();
        let mut journal = EditJournal::default();
        assert!(!journal.can_undo());

        journal.begin_edit(&tag, "Add variant");
        add_variant(&mut tag);
        let edited = tag.write_to_bytes().unwrap();
        assert_ne!(original, edited);
        assert!(journal.can_undo());

        // Undo restores the pre-edit bytes and arms redo.
        let (bytes, label) = journal.undo(&tag).unwrap();
        assert_eq!(label, "Add variant");
        assert_eq!(*bytes, original);
        tag = TagFile::read_from_bytes(&bytes).unwrap();
        assert_eq!(tag.write_to_bytes().unwrap(), original);
        assert!(!journal.can_undo());
        assert!(journal.can_redo());

        // Redo restores the post-edit bytes.
        let (bytes, _) = journal.redo(&tag).unwrap();
        assert_eq!(*bytes, edited);
        assert!(journal.can_undo());
    }

    #[test]
    fn consecutive_edits_coalesce_into_one_entry() {
        let tag = fresh_model();
        let mut journal = EditJournal::default();
        journal.begin_edit(&tag, "first");
        journal.begin_edit(&tag, "second"); // same window → no new snapshot
        assert!(journal.undo(&tag).is_some());
        assert!(!journal.can_undo());
    }

    #[test]
    fn end_edit_window_starts_a_new_entry() {
        let tag = fresh_model();
        let mut journal = EditJournal::default();
        journal.begin_edit(&tag, "first");
        journal.end_edit_window();
        journal.begin_edit(&tag, "second");
        // Two distinct entries now exist.
        assert!(journal.undo(&tag).is_some());
        assert!(journal.can_undo());
    }
}
