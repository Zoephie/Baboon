use super::*;

const KEY: &str = "file:test.render_model";
const FIELD: &str = "node list checksum";

fn app_with_open_tag() -> Baboon {
    let mut app = Baboon::for_test();
    let schema = locate_definitions_root().join("halo3_mcc/render_model.json");
    app.kits[0].parsed_tags.insert(
        KEY.to_owned(),
        TagDocument::clean(TagFile::new(schema).unwrap()),
    );
    app
}

fn set(value: &str) -> DeferredOps {
    DeferredOps {
        pending: vec![PendingFieldEdit {
            path: FIELD.to_owned(),
            input: value.to_owned(),
        }],
        ..DeferredOps::default()
    }
}

fn value(app: &Baboon) -> String {
    let doc = &app.kits[0].parsed_tags[KEY];
    doc.tag
        .root()
        .field_path(FIELD)
        .and_then(|field| field.value())
        .map(|value| match value {
            blam_tags::TagFieldData::LongInteger(value) => value.to_string(),
            other => panic!("{FIELD} is a long integer, got {other:?}"),
        })
        .unwrap_or_default()
}

fn undo_steps(app: &mut Baboon) -> usize {
    let doc = app.kits[0].parsed_tags.get_mut(KEY).unwrap();
    let mut steps = 0;
    while doc.journal.undo(&doc.tag).is_some() {
        steps += 1;
    }
    steps
}

/// A popup's confirmed edit is its own undo step; a pane's per-frame
/// edits join the one still open.
#[test]
fn own_edits_are_separate_steps_and_coalesced_edits_merge() {
    let mut app = app_with_open_tag();
    app.apply_doc_ops(0, KEY, "Edit color", set("7"), UndoStep::Own);
    app.apply_doc_ops(0, KEY, "Edit color", set("8"), UndoStep::Own);
    assert_eq!(value(&app), "8");
    assert_eq!(undo_steps(&mut app), 2, "two confirmed popups, two steps");

    let mut app = app_with_open_tag();
    app.apply_doc_ops(0, KEY, "Edit", set("7"), UndoStep::Coalesce);
    app.apply_doc_ops(0, KEY, "Edit", set("8"), UndoStep::Coalesce);
    assert_eq!(undo_steps(&mut app), 1, "one typing session, one step");
}

/// A read-only kit refuses the edit wherever it came from. The popups and
/// the reference picker applied theirs regardless, because only the pane
/// checked.
#[test]
fn a_read_only_kit_refuses_every_ui_edit() {
    let mut app = app_with_open_tag();
    let profile = CustomEditingKitProfile {
        read_only: true,
        git_tracked: false,
        id: "protected".to_owned(),
        name: "Protected".to_owned(),
        game: "halo3_mcc".to_owned(),
        root: PathBuf::from("/nowhere"),
        icon: None,
        tags_folder: None,
        data_folder: None,
    };
    app.kits[0].profile = Some(EditingKitProfileIdentity {
        id: profile.id.clone(),
        name: profile.name.clone(),
    });
    app.prefs.custom_editing_kit_profiles = vec![profile];
    let before = value(&app);

    let applied = app.apply_doc_ops(0, KEY, "Edit color", set("7"), UndoStep::Own);

    assert!(applied.is_none());
    assert_eq!(value(&app), before, "the tag is unchanged");
    assert!(app.status.contains("read-only"), "status: {}", app.status);
    assert_eq!(undo_steps(&mut app), 0);
}

/// A draft whose value was applied is marked clean, whichever path
/// applied it. Typed as `07`, which the field shows as `7`: only the
/// accept step can tell that draft was applied rather than abandoned.
#[test]
fn an_applied_edit_marks_its_draft_clean() {
    let mut app = app_with_open_tag();
    let draft_key = format!("{KEY}|{FIELD}");
    let shown = value(&app);
    let draft = app.kits[0]
        .edit_buffers
        .draft_mut(draft_key.clone(), &shown);
    draft.text = "07".to_owned();
    draft.changed = true;

    app.apply_doc_ops(0, KEY, "Paste TSV", set("07"), UndoStep::Own);

    let shown = value(&app);
    assert_eq!(shown, "7");
    let draft = app.kits[0].edit_buffers.take(&draft_key, &shown);
    assert!(!draft.changed, "the applied draft still reads as unsaved");
    assert_eq!(draft.text, "7");
}
