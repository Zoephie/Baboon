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
