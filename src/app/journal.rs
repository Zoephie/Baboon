//! Per-tag undo/redo journal.
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
pub(super) struct Snapshot {
    pub(super) bytes: Vec<u8>,
    pub(super) label: String,
}

pub(super) struct EditJournal {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    limit: usize,
    /// True while a run of consecutive edit frames is being coalesced into the
    /// single snapshot already pushed for this run.
    coalescing: bool,
}

impl Default for EditJournal {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: 64,
            coalescing: false,
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
                bytes,
                label: label.to_owned(),
            });
            self.redo.clear();
        }
        self.coalescing = true;
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
    pub(super) fn undo(&mut self, current: &TagFile) -> Option<(Vec<u8>, String)> {
        let snapshot = self.undo.pop()?;
        if let Ok(bytes) = current.write_to_bytes() {
            push_capped_into(
                &mut self.redo,
                self.limit,
                Snapshot {
                    bytes,
                    label: snapshot.label.clone(),
                },
            );
        }
        self.coalescing = false;
        Some((snapshot.bytes, snapshot.label))
    }

    /// Pop the most recent redo snapshot, recording `current` on the undo stack.
    pub(super) fn redo(&mut self, current: &TagFile) -> Option<(Vec<u8>, String)> {
        let snapshot = self.redo.pop()?;
        if let Ok(bytes) = current.write_to_bytes() {
            push_capped_into(
                &mut self.undo,
                self.limit,
                Snapshot {
                    bytes,
                    label: snapshot.label.clone(),
                },
            );
        }
        self.coalescing = false;
        Some((snapshot.bytes, snapshot.label))
    }

    fn push_capped(&mut self, snapshot: Snapshot) {
        push_capped_into(&mut self.undo, self.limit, snapshot);
    }
}

fn push_capped_into(stack: &mut Vec<Snapshot>, limit: usize, snapshot: Snapshot) {
    stack.push(snapshot);
    if stack.len() > limit {
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
        let mut dirty = false;
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
        assert_eq!(bytes, original);
        tag = TagFile::read_from_bytes(&bytes).unwrap();
        assert_eq!(tag.write_to_bytes().unwrap(), original);
        assert!(!journal.can_undo());
        assert!(journal.can_redo());

        // Redo restores the post-edit bytes.
        let (bytes, _) = journal.redo(&tag).unwrap();
        assert_eq!(bytes, edited);
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
