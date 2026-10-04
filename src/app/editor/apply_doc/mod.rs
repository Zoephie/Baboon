//! Applying the edits the UI collected to an open tag: the read-only
//! refusal, the undo step, draft bookkeeping and the status line around
//! `core::document::apply`.

use super::*;

impl Baboon {
    /// Apply edits the UI collected to one open tag: the one entry every
    /// UI-originated edit goes through, so each gets the same undo step, the
    /// same read-only refusal, the same draft bookkeeping and the same
    /// status line.
    ///
    /// `None` when nothing was applied: the tag is not open in `kit_index`,
    /// or the kit is read-only (which says so on the status line when there
    /// was something to refuse).
    pub(in crate::app) fn apply_doc_ops(
        &mut self,
        kit_index: usize,
        tag_key: &str,
        label: &str,
        ops: DeferredOps,
        step: UndoStep,
    ) -> Option<AppliedDeferredOps> {
        if self.editing_kit_is_read_only(kit_index) {
            if !ops.is_empty() {
                self.refuse_read_only_edit(kit_index);
            }
            if let Some(doc) = self.kits[kit_index].parsed_tags.get_mut(tag_key) {
                doc.journal.end_edit_window();
            }
            return None;
        }
        let kit = &mut self.kits[kit_index];
        let doc = kit.parsed_tags.get_mut(tag_key)?;
        let applied = apply_deferred_ops(doc, ops, label);
        if step == UndoStep::Own {
            doc.journal.end_edit_window();
        }
        // Per-edit outcomes: a draft whose value applied cleanly is marked
        // clean, while one the parser rejected keeps the text the user typed
        // instead of snapping back to the old value.
        kit.edit_buffers
            .accept_successful_edits(tag_key, &applied.outcomes);
        if applied.model_variants_changed
            && let Some(preview) = kit.model_previews.get_mut(tag_key)
        {
            preview.invalidate_load();
        }
        if let Some(status) = &applied.status {
            self.status = status.clone();
        }
        Some(applied)
    }
}

#[cfg(test)]
mod apply_doc_ops_tests;
