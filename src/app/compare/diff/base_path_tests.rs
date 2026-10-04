use super::*;

/// Deleting an element shifts every index below it, so the same field lives
/// at two different paths. A side-by-side view has to know both, or it
/// reads the wrong element out of the shipped tag.
#[test]
fn a_diff_row_knows_both_sides_paths() {
    let names = TagNameIndex::default();
    let a = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
    let mut b = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
    crate::app::add_block_element(&mut b, "sound classes").unwrap();
    let (rows, _) = diff_tags(&a, &b, &names, 5000);
    assert!(!rows.is_empty());
    // An added element exists only on the edited side.
    assert!(
        rows.iter()
            .any(|row| row.base_path.is_none() && row.b.starts_with("added")),
        "an added element has no path in the shipped tag: {rows:?}"
    );
}
