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
mod tests {
    use super::*;

    /// The synthetic `Thing`, open in a fresh pane.
    fn open(install: &SyntheticInstall) -> (ChimpDocument, ChimpDocumentUi) {
        let document = install.document(THING);
        let pane = install.pane(&document);
        (document, pane)
    }

    /// The property editor's edit of `Count` to `value`, as its draft sends it.
    fn count_edit(document: &ChimpDocument, value: i64) -> ChimpEdit {
        let mut decoded = document.exports[0].decoded.clone().expect("decoded");
        decoded
            .properties_mut()
            .expect("reflected")
            .entries
            .iter_mut()
            .find(|entry| &*entry.name == "Count")
            .expect("Count is present")
            .value = PropValue::Int(value);
        ChimpEdit::Properties {
            export: 0,
            decoded,
            name_map: document.header.name_map.clone(),
        }
    }

    fn count(document: &ChimpDocument) -> i64 {
        match first_value(document, "Count") {
            PropValue::Int(value) => *value,
            other => panic!("Count is {other:?}"),
        }
    }

    /// The identity draft the Header view starts from, with `flags` typed in.
    fn identity_draft(document: &ChimpDocument, flags: &str) -> ChimpIdentityEdit {
        let versioning = &document.header.versioning_info;
        ChimpIdentityEdit {
            package_flags: flags.to_owned(),
            licensee_version: versioning.licensee_version,
            is_unversioned: document.header.is_unversioned,
            zen_version: versioning.zen_version,
            file_version_ue4: versioning.package_file_version.file_version_ue4,
            file_version_ue5: versioning.package_file_version.file_version_ue5,
        }
    }

    /// An undo decodes the package as it was before the edit, and redo the one
    /// after. Both are changes in their own right: the document stays modified,
    /// its edit count moves so a save in flight does not call it saved, and the
    /// pane's texts are stale. What is on disk is not touched.
    #[test]
    fn undo_restores_an_edit_and_redo_reapplies_it() {
        let install = SyntheticInstall::new();
        let world = install.world.clone();
        let (mut document, mut pane) = open(&install);
        let original = document.original.clone();

        let edit = count_edit(&document, 42);
        assert!(apply_chimp_edit(&world, &mut document, &mut pane, edit, 1.0));
        end_chimp_edit_run(&mut document);
        assert_eq!(count(&document), 42);
        assert_eq!(document.edits, 1);

        pane.document_text_dirty = false;
        let undone = step_chimp_journal(&world, &mut document, &mut pane, false, 2.0);
        assert_eq!(undone, Ok(Some("Edit Thing".to_owned())));
        assert_eq!(count(&document), 7);
        assert!(document.dirty);
        assert_eq!(document.edits, 2);
        assert_eq!(document.checkpoint_due, Some(2.0 + CHIMP_CHECKPOINT_DELAY));
        assert!(pane.document_text_dirty);
        assert_eq!(document.original, original, "the on-disk baseline is left alone");

        let redone = step_chimp_journal(&world, &mut document, &mut pane, true, 3.0);
        assert_eq!(redone, Ok(Some("Edit Thing".to_owned())));
        assert_eq!(count(&document), 42);
        assert_eq!(
            step_chimp_journal(&world, &mut document, &mut pane, true, 4.0),
            Ok(None),
            "nothing left to redo"
        );
    }

    /// Consecutive edit frames — a drag — are one undo step; a frame without an
    /// edit ends the run, and the next edit starts another.
    #[test]
    fn a_run_of_edit_frames_is_one_step_until_a_quiet_frame() {
        let install = SyntheticInstall::new();
        let world = install.world.clone();
        let (mut document, mut pane) = open(&install);

        for value in [10, 11, 12] {
            let edit = count_edit(&document, value);
            apply_chimp_edit(&world, &mut document, &mut pane, edit, 0.0);
        }
        end_chimp_edit_run(&mut document);
        let edit = count_edit(&document, 13);
        apply_chimp_edit(&world, &mut document, &mut pane, edit, 0.0);

        step_chimp_journal(&world, &mut document, &mut pane, false, 0.0).unwrap();
        assert_eq!(count(&document), 12);
        step_chimp_journal(&world, &mut document, &mut pane, false, 0.0).unwrap();
        assert_eq!(count(&document), 7);
        assert!(!document.journal.can_undo());
    }

    /// A refused header commit changes nothing and records nothing: no step that
    /// would undo to where the document already is. The reason lands on the pane
    /// with the draft kept. An accepted one is a step of its own and clears it.
    #[test]
    fn a_refused_header_commit_records_no_step() {
        let install = SyntheticInstall::new();
        let world = install.world.clone();
        let (mut document, mut pane) = open(&install);

        let bad = identity_draft(&document, "zz");
        pane.header_identity_edit = Some(bad.clone());
        let commit = ChimpEdit::Header(ChimpHeaderCommit::Identity(bad));
        assert!(!apply_chimp_edit(&world, &mut document, &mut pane, commit, 0.0));
        assert!(!document.journal.can_undo());
        assert!(!document.dirty);
        assert_eq!(document.edits, 0);
        assert_eq!(pane.header_error.as_deref(), Some("\"zz\" is not a 32-bit hex value"));
        assert!(pane.header_identity_edit.is_some(), "the draft is kept");

        let good = identity_draft(&document, "80002200");
        let commit = ChimpEdit::Header(ChimpHeaderCommit::Identity(good));
        assert!(apply_chimp_edit(&world, &mut document, &mut pane, commit, 0.0));
        assert!(pane.header_identity_edit.is_none());
        assert!(pane.header_error.is_none());
        assert_eq!(document.header.summary.package_flags, 0x8000_2200);

        let undone = step_chimp_journal(&world, &mut document, &mut pane, false, 0.0);
        assert_eq!(undone, Ok(Some("Edit package identity".to_owned())));
        assert_eq!(document.header.summary.package_flags, 0);
        assert!(!document.journal.can_undo());
    }

    /// A header commit straight after a property edit is not folded into that
    /// edit's run, and undoing a rename puts the old name back everywhere the
    /// rename reached.
    #[test]
    fn undoing_a_rename_restores_the_name_and_the_values_showing_it() {
        let install = SyntheticInstall::new();
        let world = install.world.clone();
        let (mut document, mut pane) = open(&install);

        let edit = count_edit(&document, 42);
        apply_chimp_edit(&world, &mut document, &mut pane, edit, 0.0);
        let rename = ChimpEdit::Header(ChimpHeaderCommit::Name {
            index: 2,
            text: "Comet".to_owned(),
        });
        pane.header_name_edit = Some(ChimpNameEdit {
            index: 2,
            text: "Comet".to_owned(),
            focus: false,
        });
        assert!(apply_chimp_edit(&world, &mut document, &mut pane, rename, 0.0));
        assert!(pane.header_name_edit.is_none());
        assert_eq!(document.header.name_map.names()[2], "Comet");

        step_chimp_journal(&world, &mut document, &mut pane, false, 0.0).unwrap();
        assert_eq!(document.header.name_map.names()[2], "Rocket");
        assert!(matches!(
            first_value(&document, "Tag"),
            PropValue::Name(name) if name.as_str() == "Rocket"
        ));
        assert_eq!(count(&document), 42, "the property edit is a step of its own");
        step_chimp_journal(&world, &mut document, &mut pane, false, 0.0).unwrap();
        assert_eq!(count(&document), 7);
    }

    /// On the Chimp surface the Edit menu's Undo and Redo, and their keys, act on
    /// the selected package: enabled once it has history, and stepping it.
    #[test]
    fn undo_on_the_chimp_surface_steps_the_selected_package() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING]);
        app.model.prefs.enable_chimp = true;
        let kit = app.model.kits[0].id;
        app.views[kit].surface = KitSurface::Chimp;
        assert!(!app.can_undo_current());

        let edit = count_edit(&app.model.kits[0].chimp.documents[THING], 42);
        app.commands.send(ChimpCommand::PaneDrawn {
            kit,
            package: THING.to_owned(),
            edit: Some(edit),
        });
        let ctx = egui::Context::default();
        app.apply_commands(&ctx);
        assert_eq!(count(&app.model.kits[0].chimp.documents[THING]), 42);
        assert!(app.can_undo_current());
        assert!(!app.can_redo_current());

        app.undo_current_tag();
        assert_eq!(count(&app.model.kits[0].chimp.documents[THING]), 7);
        assert_eq!(app.model.status, "Undo: Edit Thing");
        assert!(app.can_redo_current());
        app.redo_current_tag();
        assert_eq!(count(&app.model.kits[0].chimp.documents[THING]), 42);
        assert_eq!(app.model.status, "Redo: Edit Thing");
    }
}
