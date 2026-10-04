use super::align_by_identity;

/// Fixed-field keys that match nothing, so a test exercises content
/// matching alone rather than the fallback that pairs on them.
fn distinct(len: usize) -> Vec<u64> {
    (0..len as u64).collect()
}

fn distinct_from(offset: usize, len: usize) -> Vec<u64> {
    (0..len as u64).map(|i| i + 1000 + offset as u64).collect()
}

fn ids(names: &[&str]) -> Vec<Option<String>> {
    names.iter().map(|n| Some((*n).to_owned())).collect()
}

/// The case positional pairing gets wrong: one element inserted at the top.
/// Everything below it is the same element as before and must pair with
/// itself, or the diff claims the whole block was rewritten.
#[test]
fn an_insertion_does_not_shift_everything_below_it() {
    let a = ids(&["warthog", "ghost", "banshee"]);
    let b = ids(&["scorpion", "warthog", "ghost", "banshee"]);
    let pairs = align_by_identity(&a, &b).expect("named elements align by identity");
    assert_eq!(
        pairs,
        vec![
            (None, Some(0)),
            (Some(0), Some(1)),
            (Some(1), Some(2)),
            (Some(2), Some(3)),
        ]
    );
}

#[test]
fn a_removal_is_reported_where_the_element_was() {
    let a = ids(&["warthog", "ghost", "banshee"]);
    let b = ids(&["warthog", "banshee"]);
    let pairs = align_by_identity(&a, &b).expect("aligns");
    assert_eq!(
        pairs,
        vec![(Some(0), Some(0)), (Some(1), None), (Some(2), Some(1))]
    );
}

/// Reordering is not a change to any element, so every one still pairs.
#[test]
fn reordering_pairs_every_element() {
    let a = ids(&["warthog", "ghost", "banshee"]);
    let b = ids(&["banshee", "warthog", "ghost"]);
    let pairs = align_by_identity(&a, &b).expect("aligns");
    let mut matched: Vec<(usize, usize)> = pairs
        .iter()
        .filter_map(|(x, y)| Some(((*x)?, (*y)?)))
        .collect();
    matched.sort();
    assert_eq!(matched, vec![(0, 1), (1, 2), (2, 0)]);
    assert!(
        pairs.iter().all(|(x, y)| x.is_some() && y.is_some()),
        "a pure reorder adds and removes nothing: {pairs:?}"
    );
}

/// Identities have to be trustworthy on both sides. Anonymous or repeated
/// ones fall back to position, which is at least predictable.
#[test]
fn unusable_identities_fall_back_to_position() {
    assert!(align_by_identity(&ids(&["a", "a"]), &ids(&["a", "b"])).is_none());
    assert!(align_by_identity(&ids(&["a", "b"]), &ids(&["a", "a"])).is_none());
    assert!(align_by_identity(&[None, Some("a".into())], &ids(&["a", "b"])).is_none());
    assert!(
        align_by_identity(&[], &[]).is_some(),
        "two empty blocks align trivially"
    );
}

/// An insertion shifts everything below it without reordering anything.
/// Reporting those as moved would undo the point of matching by identity.
#[test]
fn an_insertion_moves_nothing() {
    // a: warthog ghost banshee -> b: scorpion warthog ghost banshee
    let matched = [(0, 1), (1, 2), (2, 3)];
    assert!(super::moved_pairs(&matched).is_empty());
}

/// One element pulled to the front is one move, not three.
#[test]
fn a_reorder_reports_only_what_actually_moved() {
    // a: warthog ghost banshee -> b: banshee warthog ghost
    let matched = [(0, 1), (1, 2), (2, 0)];
    let moved = super::moved_pairs(&matched);
    assert_eq!(moved.len(), 1, "only banshee moved: {moved:?}");
    assert_eq!(matched[moved[0]], (2, 0));
}

#[test]
fn an_unchanged_block_reports_no_moves() {
    assert!(super::moved_pairs(&[(0, 0), (1, 1), (2, 2)]).is_empty());
    assert!(super::moved_pairs(&[]).is_empty());
}

/// A full reversal cannot be explained by fewer moves than this.
#[test]
fn a_reversal_keeps_one_element_still() {
    let matched = [(0, 3), (1, 2), (2, 1), (3, 0)];
    assert_eq!(super::moved_pairs(&matched).len(), 3);
}

/// The reported case: one element deleted from a block whose elements have
/// nothing naming them. Pairing by position reports every element below the
/// deletion as rewritten; pairing by content reports one deletion.
#[test]
fn deleting_one_anonymous_element_is_one_deletion() {
    let a = [10, 20, 30, 40, 50];
    let b = [10, 20, 40, 50];
    let pairs =
        super::align_by_content(&a, &b, &distinct(a.len()), &distinct_from(a.len(), b.len()))
            .expect("aligns");
    assert_eq!(
        pairs,
        vec![
            (Some(0), Some(0)),
            (Some(1), Some(1)),
            (Some(2), None),
            (Some(3), Some(2)),
            (Some(4), Some(3)),
        ]
    );
}

/// Editing an element changes its fingerprint, so it drops out of the
/// common subsequence on both sides at once. Zipping those together is
/// what makes it read as modified rather than removed and re-added.
#[test]
fn editing_an_anonymous_element_reads_as_a_modification() {
    let a = [10, 20, 30];
    let b = [10, 99, 30];
    let pairs =
        super::align_by_content(&a, &b, &distinct(a.len()), &distinct_from(a.len(), b.len()))
            .expect("aligns");
    assert_eq!(
        pairs,
        vec![(Some(0), Some(0)), (Some(1), Some(1)), (Some(2), Some(2))]
    );
}

/// Repeated values are ordinary in these blocks -- checksums, bit vectors --
/// and must not defeat matching the way duplicate identities do.
#[test]
fn repeated_content_still_aligns() {
    let a = [0, 0, 0, 7];
    let b = [0, 0, 0, 0, 7];
    let pairs =
        super::align_by_content(&a, &b, &distinct(a.len()), &distinct_from(a.len(), b.len()))
            .expect("aligns");
    let added: Vec<_> = pairs.iter().filter(|(x, _)| x.is_none()).collect();
    assert_eq!(added.len(), 1, "one element gained: {pairs:?}");
    assert!(
        pairs.iter().filter(|(_, y)| y.is_none()).count() == 0,
        "nothing was lost: {pairs:?}"
    );
}

/// A block big enough that the quadratic table is not worth building falls
/// back to position rather than stalling the review.
#[test]
fn very_large_blocks_fall_back_to_position() {
    let big: Vec<u64> = (0..600).collect();
    assert!(super::align_by_content(&big, &big, &big, &big).is_none());
}

#[test]
fn everything_added_or_everything_removed() {
    let pairs = align_by_identity(&[], &ids(&["a", "b"])).expect("aligns");
    assert_eq!(pairs, vec![(None, Some(0)), (None, Some(1))]);
    let pairs = align_by_identity(&ids(&["a", "b"]), &[]).expect("aligns");
    assert_eq!(pairs, vec![(Some(0), None), (Some(1), None)]);
}
