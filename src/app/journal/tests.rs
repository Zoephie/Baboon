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
            Snapshot::new(Arc::new(vec![0; 400]), format!("edit {i}")),
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
        Snapshot::new(Arc::new(vec![0; budget * 4]), "huge".to_owned()),
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

/// A step keeps its id through undo and a restart, and a step made after
/// a restore cannot take an id the restored file already holds.
#[test]
fn steps_keep_their_ids_and_new_ones_never_collide() {
    let restored = Snapshot::restored(1_000_000, Arc::new(vec![1]), "old".to_owned());
    assert_eq!(restored.id, 1_000_000);
    let fresh = Snapshot::new(Arc::new(vec![2]), "new".to_owned());
    assert!(fresh.id > restored.id);

    let mut tag = fresh_model();
    let mut journal = EditJournal::default();
    journal.begin_edit(&tag, "edit");
    let pushed = journal.stacks().0[0].id;
    add_variant(&mut tag);
    journal.end_edit_window();
    journal.undo(&tag);
    assert_ne!(
        journal.stacks().1[0].id,
        pushed,
        "the redo step is a new step"
    );
}
