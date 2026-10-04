//! Chimp edits: what a document pane asks to change, applied as commands and recorded in the document's journal.
//! It owns applying, undoing and redoing edits to an open package; drawing the editors that ask for them belongs in `property_editor` and `header_ui`.
//!
//! A step is the whole package as a save would write it, and undo decodes it
//! back. `original` is left alone: it is what is on disk, the baseline a
//! discard returns to, and undoing an edit does not change what was saved.

use super::*;
use crate::core::document::journal::{EditJournal, JournalDocument};
use blam_tags::iostore::package::name_map::FNameMap;
use std::cell::Cell;

/// One change a pane asks of its document.
pub(in crate::app) enum ChimpEdit {
    /// An export's values as the property editor's draft holds them, with the
    /// name map its names were interned into.
    Properties {
        export: usize,
        decoded: Export,
        name_map: FNameMap,
    },
    /// A header draft being committed.
    Header(ChimpHeaderCommit),
}

/// A header draft being committed, carrying the draft as it stood.
pub(in crate::app) enum ChimpHeaderCommit {
    Identity(ChimpIdentityEdit),
    Export(ChimpExportEdit),
    Name { index: usize, text: String },
    Import { index: usize, slot: ImportSlot },
}

/// What a journal snapshots for a Chimp document: the package as a save would
/// rebuild it. A document that cannot be rebuilt — orphaned, or holding a
/// header that does not validate — records no step.
struct ChimpJournalSource<'a> {
    world: &'a World,
    document: &'a ChimpDocument,
}

impl JournalDocument for ChimpJournalSource<'_> {
    fn snapshot_bytes(&self) -> Option<Vec<u8>> {
        rebuild_chimp_document(self.world, self.document)
            .ok()
            .map(|(bytes, _)| bytes)
    }
}

/// Bytes captured before an edit that may yet be refused, recorded only once
/// it is not.
struct Captured(Cell<Option<Vec<u8>>>);

impl JournalDocument for Captured {
    fn snapshot_bytes(&self) -> Option<Vec<u8>> {
        self.0.take()
    }
}

/// Run `step` with `document`'s journal held apart from it, so the journal can
/// snapshot the document it belongs to.
fn with_journal<R>(
    world: &World,
    document: &mut ChimpDocument,
    step: impl FnOnce(&mut EditJournal, &ChimpJournalSource) -> R,
) -> R {
    let mut journal = std::mem::take(&mut document.journal);
    let result = step(
        &mut journal,
        &ChimpJournalSource {
            world,
            document: &*document,
        },
    );
    document.journal = journal;
    result
}

/// Note that `document` changed at `now`: it is modified, its recovery
/// checkpoint is pushed back, and what the pane derived from it is stale.
fn note_chimp_change(document: &mut ChimpDocument, pane: &mut ChimpDocumentUi, now: f64) {
    document.dirty = true;
    document.edits += 1;
    document.checkpoint_due = Some(now + CHIMP_CHECKPOINT_DELAY);
    pane.document_text_dirty = true;
    pane.metadata_text_dirty = true;
    // Reference counts are derived from the same header the metadata text is,
    // so they go stale at exactly the same moment.
    pane.header_usage = None;
}

/// Close the run of edits coalescing into `document`'s current undo step.
pub(super) fn end_chimp_edit_run(document: &mut ChimpDocument) {
    document.journal.end_edit_window();
}

/// Apply `edit` to `document`, recording the step in its journal, and settle
/// the drafts in `pane` that asked for it. Returns whether the document
/// changed; a refused header commit leaves it as it was, its draft in place
/// and the reason on the pane.
pub(super) fn apply_chimp_edit(
    world: &World,
    document: &mut ChimpDocument,
    pane: &mut ChimpDocumentUi,
    edit: ChimpEdit,
    now: f64,
) -> bool {
    match edit {
        ChimpEdit::Properties {
            export,
            decoded,
            name_map,
        } => {
            let Some(object) = document.exports.get(export).map(|export| export.object.clone())
            else {
                return false;
            };
            with_journal(world, document, |journal, source| {
                journal.begin_edit(source, &format!("Edit {object}"));
            });
            document.exports[export].decoded = Ok(decoded);
            document.header.name_map = name_map;
        }
        ChimpEdit::Header(commit) => {
            // Captured before and recorded after: a commit can be refused, and
            // a refused one must not leave a step that undoes nothing.
            let before = Captured(Cell::new(
                rebuild_chimp_document(world, document)
                    .ok()
                    .map(|(bytes, _)| bytes),
            ));
            let label = match &commit {
                ChimpHeaderCommit::Identity(_) => "Edit package identity",
                ChimpHeaderCommit::Export(_) => "Edit export entry",
                ChimpHeaderCommit::Name { .. } => "Rename name entry",
                ChimpHeaderCommit::Import { .. } => "Change import slot",
            };
            let applied = match &commit {
                ChimpHeaderCommit::Identity(edit) => apply_chimp_identity_edit(document, edit),
                ChimpHeaderCommit::Export(edit) => apply_chimp_export_edit(world, document, edit),
                ChimpHeaderCommit::Name { index, text } => {
                    apply_chimp_name_rename(document, *index, text)
                }
                ChimpHeaderCommit::Import { index, slot } => {
                    apply_chimp_import_slot(document, *index, slot.clone())
                }
            };
            if let Err(error) = applied {
                pane.header_error = Some(error);
                return false;
            }
            // A step of its own, not folded into a run of property edits.
            document.journal.end_edit_window();
            document.journal.begin_edit(&before, label);
            document.journal.end_edit_window();
            match commit {
                ChimpHeaderCommit::Identity(_) => pane.header_identity_edit = None,
                ChimpHeaderCommit::Export(_) => pane.header_export_edit = None,
                ChimpHeaderCommit::Name { .. } => pane.header_name_edit = None,
                ChimpHeaderCommit::Import { .. } => pane.header_import_edit = None,
            }
            pane.header_error = None;
        }
    }
    note_chimp_change(document, pane, now);
    refresh_chimp_header_usage(document, pane);
    true
}

/// Step `document` back (or, with `redo`, forward) through its journal.
/// Returns the step's label, `None` when there was nothing to step to, or
/// why the step could not be decoded.
pub(super) fn step_chimp_journal(
    world: &World,
    document: &mut ChimpDocument,
    pane: &mut ChimpDocumentUi,
    redo: bool,
    now: f64,
) -> Result<Option<String>, String> {
    let restored = with_journal(world, document, |journal, source| {
        if redo {
            journal.redo(source)
        } else {
            journal.undo(source)
        }
    });
    let Some((bytes, label)) = restored else {
        return Ok(None);
    };
    let (header, payloads, exports) = decode_chimp_exports(world, &document.provider, &bytes)?;
    document.mesh_kind = chimp_mesh_kind(&exports);
    document.header = header;
    document.payloads = payloads;
    document.exports = exports;
    // Drafts name rows of the header that was just replaced.
    pane.header_name_edit = None;
    pane.header_import_edit = None;
    pane.header_export_edit = None;
    pane.header_identity_edit = None;
    pane.header_error = None;
    note_chimp_change(document, pane, now);
    refresh_chimp_header_usage(document, pane);
    Ok(Some(label))
}

impl Baboon {
    /// A mounted kit's open document and its pane, with the world it was
    /// decoded against.
    pub(super) fn chimp_document_and_pane(
        &mut self,
        kit_index: usize,
        package: &str,
    ) -> Option<(Arc<World>, &mut ChimpDocument, &mut ChimpDocumentUi)> {
        let kit = &mut self.model.kits[kit_index];
        let ChimpMount::Ready(world) = &kit.chimp.mount else {
            return None;
        };
        let world = world.clone();
        let document = kit.chimp.documents.get_mut(package)?;
        let pane = self.views[kit.id].chimp.documents.get_mut(package)?;
        Some((world, document, pane))
    }

    /// The package undo and redo act on while the Chimp surface is in front:
    /// the active kit's selected document.
    fn current_chimp_document(&self) -> Option<&ChimpDocument> {
        let chimp = &self.model.kits[self.model.active].chimp;
        chimp.documents.get(chimp.selected_package.as_deref()?)
    }

    pub(in crate::app) fn can_step_chimp_journal(&self, redo: bool) -> bool {
        self.current_chimp_document().is_some_and(|document| {
            if redo {
                document.journal.can_redo()
            } else {
                document.journal.can_undo()
            }
        })
    }

    /// Undo (or redo) the selected Chimp document's last edit.
    pub(in crate::app) fn step_current_chimp_journal(&mut self, redo: bool) {
        let verb = if redo { "Redo" } else { "Undo" };
        let kit_index = self.model.active;
        let now = self.egui_ctx.input(|input| input.time);
        let stepped = self.model.kits[kit_index]
            .chimp
            .selected_package
            .clone()
            .and_then(|package| self.chimp_document_and_pane(kit_index, &package))
            .map(|(world, document, pane)| step_chimp_journal(&world, document, pane, redo, now));
        self.model.status = match stepped {
            Some(Ok(Some(label))) => format!("{verb}: {label}"),
            Some(Err(error)) => format!("{verb} failed: {error}"),
            Some(Ok(None)) | None => format!("Nothing to {}", verb.to_ascii_lowercase()),
        };
    }
}

#[cfg(test)]
mod tests;
