//! An open tag's pane hands its edits on every frame, an empty set included,
//! because applying none is what ends the undo step that typing coalesces
//! into. Were the empty frames skipped, every edit after the first would fold
//! into one step that undo could only take back whole.

use super::perf_baseline_tests::Harness;
use super::perf_baseline_tests::fixture;

fn steps(h: &mut Harness, key: &str) -> usize {
    let kit = h.app.model.active;
    let doc = h.app.model.kits[kit].parsed_tags.get_mut(key).expect("the tag is open");
    let mut count = 0;
    while doc.journal.undo(&doc.tag).is_some() {
        count += 1;
    }
    count
}

#[test]
fn a_drawn_frame_with_no_edits_ends_the_undo_step() {
    let mut h = Harness::new();
    let tag = fixture::synthetic_shader(1);
    let path = "shaders/undo.shader";
    let mut entries = fixture::synthetic_entries(1, 1, 1);
    entries.push(fixture::document_entry(path, &tag));
    fixture::install_kit(&mut h.app, entries);
    let key = fixture::open_document(&mut h.app, path, tag);
    h.frame(Vec::new());

    // Two edits a frame apart, as typing makes them.
    let kit = h.app.model.active;
    let doc = h.app.model.kits[kit].parsed_tags.get_mut(&key).unwrap();
    doc.journal.begin_edit(&doc.tag, "first");
    h.frame(Vec::new());
    let doc = h.app.model.kits[kit].parsed_tags.get_mut(&key).unwrap();
    doc.journal.begin_edit(&doc.tag, "second");
    assert_eq!(steps(&mut h, &key), 2, "the frame between them ended the first");
}

/// The check can fail: with no frame drawn between them, the two edits are
/// one step.
#[test]
fn edits_with_no_frame_between_them_are_one_step() {
    let mut h = Harness::new();
    let tag = fixture::synthetic_shader(1);
    let path = "shaders/undo.shader";
    let mut entries = fixture::synthetic_entries(1, 1, 1);
    entries.push(fixture::document_entry(path, &tag));
    fixture::install_kit(&mut h.app, entries);
    let key = fixture::open_document(&mut h.app, path, tag);
    h.frame(Vec::new());

    let kit = h.app.model.active;
    let doc = h.app.model.kits[kit].parsed_tags.get_mut(&key).unwrap();
    doc.journal.begin_edit(&doc.tag, "first");
    doc.journal.begin_edit(&doc.tag, "second");
    assert_eq!(steps(&mut h, &key), 1);
}
