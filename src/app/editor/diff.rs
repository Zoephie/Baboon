//! Structural tag comparison.
//! It owns tag-editor presentation and deferred edit construction; source loading and application lifecycle coordination belong elsewhere.

use super::*;

pub(in crate::app) fn diff_tags(
    a: &TagFile,
    b: &TagFile,
    names: &TagNameIndex,
    limit: usize,
) -> (Vec<TagFieldDiff>, bool) {
    let mut out = Vec::new();
    diff_structs(&a.root(), &b.root(), "", names, &mut out, limit);
    let truncated = out.len() > limit;
    out.truncate(limit);
    (out, truncated)
}

/// Pair up two blocks' elements before diffing them.
///
/// Returns `None` when the elements carry no usable identity, leaving the
/// caller to pair them positionally. Positional pairing is only right when
/// nothing was inserted or removed: insert one element at the top of a block
/// and every element after it reads as rewritten, which is the difference
/// between a diff worth reading and a screen of noise.
///
/// Identities have to be present and distinct on *both* sides to be trusted.
/// Half-identified elements would pair some rows by name and the rest by
/// position, which is harder to reason about than either rule alone.
fn align_by_identity(
    a: &[Option<String>],
    b: &[Option<String>],
) -> Option<Vec<(Option<usize>, Option<usize>)>> {
    let usable = |ids: &[Option<String>]| {
        let named: Vec<&String> = ids.iter().flatten().collect();
        named.len() == ids.len() && {
            let mut seen = HashSet::new();
            named.iter().all(|id| seen.insert(*id))
        }
    };
    if !usable(a) || !usable(b) {
        return None;
    }
    let index_in_b: HashMap<&String, usize> = b
        .iter()
        .enumerate()
        .filter_map(|(index, id)| id.as_ref().map(|id| (id, index)))
        .collect();
    let matched: HashMap<usize, usize> = a
        .iter()
        .enumerate()
        .filter_map(|(ai, id)| {
            let id = id.as_ref()?;
            index_in_b.get(id).map(|&bi| (ai, bi))
        })
        .collect();
    let matched_b: HashSet<usize> = matched.values().copied().collect();

    // Walked in the edited tag's order, so the diff reads the way the tag now
    // does, with removals shown at the point they were taken from.
    let mut pairs = Vec::new();
    let mut next_b = 0usize;
    for (ai, _) in a.iter().enumerate() {
        match matched.get(&ai) {
            Some(&bi) => {
                while next_b < bi {
                    if !matched_b.contains(&next_b) {
                        pairs.push((None, Some(next_b)));
                    }
                    next_b += 1;
                }
                pairs.push((Some(ai), Some(bi)));
                next_b = bi + 1;
            }
            None => pairs.push((Some(ai), None)),
        }
    }
    for bi in next_b..b.len() {
        if !matched_b.contains(&bi) {
            pairs.push((None, Some(bi)));
        }
    }
    Some(pairs)
}

/// The matched pairs whose element moved, as indices into `matched`.
///
/// Reordering a block changes nothing inside any element, so a diff that only
/// reports fields says nothing happened -- while the exported tag is genuinely
/// different, and for a palette the difference matters a great deal: block
/// index fields elsewhere name elements by position, so moving one silently
/// repoints every reference to it.
///
/// Only the smallest set that explains the reordering is reported. Everything
/// on the longest increasing run of destination indices stayed in relative
/// order and did not move; inserting an element shifts the ones below it
/// without reordering anything, and must not light them all up.
fn moved_pairs(matched: &[(usize, usize)]) -> Vec<usize> {
    // Longest increasing subsequence over the destination indices, by patience
    // sorting, reconstructed through predecessor links.
    let mut piles: Vec<usize> = Vec::new();
    let mut previous = vec![usize::MAX; matched.len()];
    for (i, &(_, bi)) in matched.iter().enumerate() {
        let at = piles.partition_point(|&p| matched[p].1 < bi);
        if at > 0 {
            previous[i] = piles[at - 1];
        }
        if at == piles.len() {
            piles.push(i);
        } else {
            piles[at] = i;
        }
    }
    let mut kept = vec![false; matched.len()];
    let mut cursor = piles.last().copied();
    while let Some(i) = cursor {
        kept[i] = true;
        cursor = (previous[i] != usize::MAX).then_some(previous[i]);
    }
    (0..matched.len()).filter(|&i| !kept[i]).collect()
}

/// The identity an element is matched on: whatever *names* it.
///
/// Deliberately not the instance selector's label, which falls back to the
/// element's first scalar. A checksum or a bitmask is a value, not a name:
/// matching on one makes an element that merely had that value edited look
/// removed and re-added, and blocks full of repeated values -- checksums, bit
/// vectors -- have no distinct identities at all, so matching is abandoned
/// exactly where it is needed most.
fn element_identity(element: Option<TagStruct<'_>>, names: &TagNameIndex) -> Option<String> {
    let element = element?;
    foundation::first_named_string_label(element)
        .or_else(|| foundation::first_tag_reference_label(element, names))
        .or_else(|| foundation::first_string_label(element))
}

/// A cheap fingerprint of an element's own bytes, for matching elements that
/// have nothing naming them.
///
/// Only the element's fixed-size data, so two elements differing solely inside
/// a nested block fingerprint alike -- which is what we want: they are the same
/// element, and recursing finds what changed within it.
fn element_fingerprint(element: Option<TagStruct<'_>>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match element {
        Some(element) => element.raw().hash(&mut hasher),
        None => 0u8.hash(&mut hasher),
    }
    hasher.finish()
}

/// Pair elements by content when nothing names them.
///
/// A longest-common-subsequence over element fingerprints, so deleting one
/// element from a block of anonymous ones is reported as one deletion rather
/// than as every element below it having been rewritten -- which is what
/// pairing by position does, and what made a single deleted `zone set pvs`
/// element produce a screen of unrelated changes.
///
/// Unmatched runs on both sides at the same point are then zipped together, so
/// an element whose contents were edited reads as modified rather than as a
/// removal followed by an addition.
///
/// Returns `None` for blocks large enough that the quadratic table is not worth
/// it; those fall back to position, which is at least predictable.
fn align_by_content(a: &[u64], b: &[u64]) -> Option<Vec<(Option<usize>, Option<usize>)>> {
    const MAX_ELEMENTS: usize = 512;
    if a.len() > MAX_ELEMENTS || b.len() > MAX_ELEMENTS {
        return None;
    }
    let width = b.len() + 1;
    let mut table = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i * width + j] = if a[i] == b[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let mut pairs = Vec::new();
    let mut removed: Vec<usize> = Vec::new();
    let mut added: Vec<usize> = Vec::new();
    // Elements dropped on one side and gained on the other, at the same point
    // in the sequence, are the same element edited.
    let flush = |pairs: &mut Vec<(Option<usize>, Option<usize>)>,
                 removed: &mut Vec<usize>,
                 added: &mut Vec<usize>| {
        let paired = removed.len().min(added.len());
        for index in 0..paired {
            pairs.push((Some(removed[index]), Some(added[index])));
        }
        for &index in &removed[paired..] {
            pairs.push((Some(index), None));
        }
        for &index in &added[paired..] {
            pairs.push((None, Some(index)));
        }
        removed.clear();
        added.clear();
    };
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            flush(&mut pairs, &mut removed, &mut added);
            pairs.push((Some(i), Some(j)));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            removed.push(i);
            i += 1;
        } else {
            added.push(j);
            j += 1;
        }
    }
    removed.extend(i..a.len());
    added.extend(j..b.len());
    flush(&mut pairs, &mut removed, &mut added);
    Some(pairs)
}

fn diff_structs(
    a: &TagStruct<'_>,
    b: &TagStruct<'_>,
    path: &str,
    names: &TagNameIndex,
    out: &mut Vec<TagFieldDiff>,
    limit: usize,
) {
    for (fa, fb) in a.fields_all().zip(b.fields_all()) {
        if out.len() > limit {
            return;
        }
        let field_path = append_field_path(path, fa.name());
        if let (Some(ba), Some(bb)) = (fa.as_block(), fb.as_block()) {
            let ids_a: Vec<Option<String>> = (0..ba.len())
                .map(|i| element_identity(ba.element(i), names))
                .collect();
            let ids_b: Vec<Option<String>> = (0..bb.len())
                .map(|i| element_identity(bb.element(i), names))
                .collect();
            // Names first, since they survive an element's contents changing.
            // Otherwise match on content, which is the only thing anonymous
            // elements have. Position is the last resort.
            let pairs = align_by_identity(&ids_a, &ids_b)
                .or_else(|| {
                    let fps_a: Vec<u64> =
                        (0..ba.len()).map(|i| element_fingerprint(ba.element(i))).collect();
                    let fps_b: Vec<u64> =
                        (0..bb.len()).map(|i| element_fingerprint(bb.element(i))).collect();
                    align_by_content(&fps_a, &fps_b)
                })
                .unwrap_or_else(|| {
                    (0..ba.len().max(bb.len()))
                        .map(|i| ((i < ba.len()).then_some(i), (i < bb.len()).then_some(i)))
                        .collect()
                });
            // Reported before the per-element diffs so a reorder reads as a
            // property of the block rather than of one element in it.
            let matched: Vec<(usize, usize)> = pairs
                .iter()
                .filter_map(|(x, y)| Some(((*x)?, (*y)?)))
                .collect();
            for index in moved_pairs(&matched) {
                if out.len() > limit {
                    return;
                }
                let (ai, bi) = matched[index];
                let label = ids_b[bi]
                    .clone()
                    .or_else(|| ids_a[ai].clone())
                    .unwrap_or_else(|| format!("element {ai}"));
                out.push(TagFieldDiff {
                    path: format!("{field_path}[{bi}]"),
                    a: format!("position {ai}"),
                    b: format!("moved to {bi} — {label}"),
                });
            }
            for (ai, bi) in pairs {
                if out.len() > limit {
                    return;
                }
                match (ai, bi) {
                    (Some(ai), Some(bi)) => {
                        let (Some(ea), Some(eb)) = (ba.element(ai), bb.element(bi)) else {
                            continue;
                        };
                        let before = out.len();
                        diff_structs(&ea, &eb, &format!("{field_path}[{bi}]"), names, out, limit);
                        // Name the element, but only once something inside it
                        // turned out to differ: an index alone says very little
                        // about which of a hundred palette entries this is.
                        // Both sides carry the name, which is how the renderer
                        // tells "this element, unchanged in itself" from one
                        // that was added or removed.
                        if out.len() > before
                            && let Some(label) = ids_b[bi].clone()
                        {
                            out.insert(
                                before,
                                TagFieldDiff {
                                    path: format!("{field_path}[{bi}]"),
                                    a: label.clone(),
                                    b: label,
                                },
                            );
                        }
                    }
                    // An added or removed element is reported with its
                    // contents: "one more element" says nothing about what is
                    // actually being shipped.
                    (None, Some(bi)) => {
                        let label = ids_b[bi].clone().unwrap_or_else(|| format!("element {bi}"));
                        out.push(TagFieldDiff {
                            path: format!("{field_path}[{bi}]"),
                            a: String::new(),
                            b: format!("added — {label}"),
                        });
                        if let Some(element) = bb.element(bi) {
                            dump_struct(
                                &element,
                                &format!("{field_path}[{bi}]"),
                                names,
                                out,
                                limit,
                                true,
                            );
                        }
                    }
                    (Some(ai), None) => {
                        let label = ids_a[ai].clone().unwrap_or_else(|| format!("element {ai}"));
                        out.push(TagFieldDiff {
                            path: format!("{field_path}[{ai}]"),
                            a: format!("removed — {label}"),
                            b: String::new(),
                        });
                        if let Some(element) = ba.element(ai) {
                            dump_struct(
                                &element,
                                &format!("{field_path}[{ai}]"),
                                names,
                                out,
                                limit,
                                false,
                            );
                        }
                    }
                    (None, None) => {}
                }
            }
        } else if let (Some(aa), Some(ab)) = (fa.as_array(), fb.as_array()) {
            // Arrays are fixed-count, so their elements always correspond.
            for i in 0..aa.len().min(ab.len()) {
                let (Some(ea), Some(eb)) = (aa.element(i), ab.element(i)) else {
                    continue;
                };
                diff_structs(&ea, &eb, &format!("{field_path}[{i}]"), names, out, limit);
            }
        } else if let (Some(sa), Some(sb)) = (fa.as_struct(), fb.as_struct()) {
            diff_structs(&sa, &sb, &field_path, names, out, limit);
        } else if let (Some(da), Some(db)) = (fa.as_data(), fb.as_data()) {
            // Raw data fields -- function curves, shader source, import info --
            // are edited through their own editors and land in an exported mod
            // like any other change. They have no readable form here, but a
            // preview that omits them entirely is worse than one that says only
            // that they moved.
            if da != db {
                out.push(TagFieldDiff {
                    path: field_path,
                    a: format!("{} bytes", da.len()),
                    b: if da.len() == db.len() {
                        format!("{} bytes, contents differ", db.len())
                    } else {
                        format!("{} bytes", db.len())
                    },
                });
            }
        } else if let (Some(va), Some(vb)) = (fa.value(), fb.value()) {
            let ta = foundation::format_foundation_scalar_value(names, &va);
            let tb = foundation::format_foundation_scalar_value(names, &vb);
            if ta != tb {
                out.push(TagFieldDiff {
                    path: field_path,
                    a: ta,
                    b: tb,
                });
            }
        }
    }
}

/// Describe a whole tag as additions, for a tag the game has no counterpart
/// for.
///
/// A new tag has nothing to diff against, but "what is in it" is the same
/// question the modified rows answer, so it is answered in the same shape and
/// rendered by the same code rather than through a second presentation.
pub(in crate::app) fn describe_tag(
    tag: &TagFile,
    names: &TagNameIndex,
    limit: usize,
) -> (Vec<TagFieldDiff>, bool) {
    let mut out = Vec::new();
    dump_struct(&tag.root(), "", names, &mut out, limit, true);
    let truncated = out.len() > limit;
    out.truncate(limit);
    (out, truncated)
}

/// Emit every scalar in a struct that has no counterpart, so an added or
/// removed element shows what it actually contains.
fn dump_struct(
    st: &TagStruct<'_>,
    path: &str,
    names: &TagNameIndex,
    out: &mut Vec<TagFieldDiff>,
    limit: usize,
    added: bool,
) {
    for field in st.fields_all() {
        if out.len() > limit {
            return;
        }
        let field_path = append_field_path(path, field.name());
        if let Some(block) = field.as_block() {
            for i in 0..block.len() {
                if let Some(element) = block.element(i) {
                    dump_struct(&element, &format!("{field_path}[{i}]"), names, out, limit, added);
                }
            }
        } else if let Some(array) = field.as_array() {
            for i in 0..array.len() {
                if let Some(element) = array.element(i) {
                    dump_struct(&element, &format!("{field_path}[{i}]"), names, out, limit, added);
                }
            }
        } else if let Some(inner) = field.as_struct() {
            dump_struct(&inner, &field_path, names, out, limit, added);
        } else if let Some(value) = field.value() {
            let text = foundation::format_foundation_scalar_value(names, &value);
            out.push(TagFieldDiff {
                path: field_path,
                a: if added { String::new() } else { text.clone() },
                b: if added { text } else { String::new() },
            });
        }
    }
}

#[cfg(test)]
mod alignment_tests {
    use super::align_by_identity;

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
        assert!(align_by_identity(&[], &[]).is_some(), "two empty blocks align trivially");
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
        let pairs = super::align_by_content(&a, &b).expect("aligns");
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
        let pairs = super::align_by_content(&a, &b).expect("aligns");
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
        let pairs = super::align_by_content(&a, &b).expect("aligns");
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
        assert!(super::align_by_content(&big, &big).is_none());
    }

    #[test]
    fn everything_added_or_everything_removed() {
        let pairs = align_by_identity(&[], &ids(&["a", "b"])).expect("aligns");
        assert_eq!(pairs, vec![(None, Some(0)), (None, Some(1))]);
        let pairs = align_by_identity(&ids(&["a", "b"]), &[]).expect("aligns");
        assert_eq!(pairs, vec![(Some(0), None), (Some(1), None)]);
    }
}
