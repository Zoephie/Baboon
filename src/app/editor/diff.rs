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
    diff_structs(&a.root(), &b.root(), "", "", names, &mut out, limit);
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

/// How deep a fingerprint follows nested structure before it stops. Deep enough
/// to reach what distinguishes real elements, bounded so a pathological tag
/// cannot make aligning one block cost the whole document.
const FINGERPRINT_DEPTH: usize = 6;

/// A fingerprint of an element's whole contents, for matching elements that
/// have nothing naming them.
///
/// Nested data is included, and that is the point. `raw()` covers only a
/// struct's fixed-size fields, and for a good many blocks everything that
/// tells one element from another lives in child blocks -- a `zone set pvs`
/// element is a version, a mask and some flags, with the checksums and cluster
/// data underneath. Fingerprinting the fixed part alone made those elements
/// indistinguishable, so the aligner matched an arbitrary valid subsequence
/// and every field below a deletion read as changed, each one shifted by
/// exactly one position.
pub(in crate::app) fn element_fingerprint(element: Option<TagStruct<'_>>) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match element {
        Some(element) => hash_struct(&element, &mut hasher, 0),
        None => hasher.write_u8(0),
    }
    hasher.finish()
}

/// An element by its own fixed fields only, ignoring everything nested. Two
/// elements matching here are the same element even if something inside one of
/// them was edited.
pub(in crate::app) fn shallow_fingerprint(element: Option<TagStruct<'_>>) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match element {
        Some(element) => hasher.write(element.raw()),
        None => hasher.write_u8(0),
    }
    hasher.finish()
}

fn hash_struct(
    st: &TagStruct<'_>,
    hasher: &mut std::collections::hash_map::DefaultHasher,
    depth: usize,
) {
    use std::hash::Hasher;
    hasher.write(st.raw());
    if depth >= FINGERPRINT_DEPTH {
        return;
    }
    for field in st.fields_all() {
        if let Some(block) = field.as_block() {
            // The count matters: two otherwise identical elements holding
            // different numbers of children are different elements.
            hasher.write_usize(block.len());
            for index in 0..block.len() {
                if let Some(element) = block.element(index) {
                    hash_struct(&element, hasher, depth + 1);
                }
            }
        } else if let Some(array) = field.as_array() {
            for index in 0..array.len() {
                if let Some(element) = array.element(index) {
                    hash_struct(&element, hasher, depth + 1);
                }
            }
        } else if let Some(inner) = field.as_struct() {
            hash_struct(&inner, hasher, depth + 1);
        } else if let Some(data) = field.as_data() {
            hasher.write(data);
        }
    }
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
fn align_by_content(
    a: &[u64],
    b: &[u64],
    a_shallow: &[u64],
    b_shallow: &[u64],
) -> Option<Vec<(Option<usize>, Option<usize>)>> {
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
        // Editing a value inside an element changes its contents, so it drops
        // out of the common subsequence even though it is still the same
        // element. Pair the leftovers on their fixed fields first, which an
        // edit further down does not disturb -- without this, deleting one
        // element and editing another paired element 3 against element 4 and
        // reported every field below as changed.
        let mut taken = vec![false; added.len()];
        let mut matched: Vec<(usize, usize)> = Vec::new();
        for &from in removed.iter() {
            if let Some(slot) = added
                .iter()
                .enumerate()
                .position(|(slot, &to)| !taken[slot] && a_shallow[from] == b_shallow[to])
            {
                taken[slot] = true;
                matched.push((from, added[slot]));
            }
        }
        let paired_from: Vec<usize> = matched.iter().map(|(from, _)| *from).collect();
        let paired_to: Vec<usize> = matched.iter().map(|(_, to)| *to).collect();
        let mut rest_from: Vec<usize> = removed
            .iter()
            .copied()
            .filter(|index| !paired_from.contains(index))
            .collect();
        let mut rest_to: Vec<usize> = added
            .iter()
            .copied()
            .filter(|index| !paired_to.contains(index))
            .collect();
        for (from, to) in matched {
            pairs.push((Some(from), Some(to)));
        }
        // Whatever is still unaccounted for is paired in order, then reported
        // as gained or lost.
        let zipped = rest_from.len().min(rest_to.len());
        for index in 0..zipped {
            pairs.push((Some(rest_from[index]), Some(rest_to[index])));
        }
        for &index in &rest_from[zipped..] {
            pairs.push((Some(index), None));
        }
        for &index in &rest_to[zipped..] {
            pairs.push((None, Some(index)));
        }
        rest_from.clear();
        rest_to.clear();
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
    base_path: &str,
    names: &TagNameIndex,
    out: &mut Vec<TagFieldDiff>,
    limit: usize,
) {
    for (fa, fb) in a.fields_all().zip(b.fields_all()) {
        if out.len() > limit {
            return;
        }
        let field_path = append_field_path(path, fa.name());
        let base_field_path = append_field_path(base_path, fa.name());
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
                    // The same elements by their fixed fields alone, which
                    // survive an edit to anything nested inside them.
                    let shallow_a: Vec<u64> =
                        (0..ba.len()).map(|i| shallow_fingerprint(ba.element(i))).collect();
                    let shallow_b: Vec<u64> =
                        (0..bb.len()).map(|i| shallow_fingerprint(bb.element(i))).collect();
                    align_by_content(&fps_a, &fps_b, &shallow_a, &shallow_b)
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
                    base_path: Some(format!("{base_field_path}[{ai}]")),
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
                        diff_structs(
                            &ea,
                            &eb,
                            &format!("{field_path}[{bi}]"),
                            &format!("{base_field_path}[{ai}]"),
                            names,
                            out,
                            limit,
                        );
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
                                    base_path: Some(format!("{base_field_path}[{ai}]")),
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
                            base_path: None,
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
                            base_path: Some(format!("{base_field_path}[{ai}]")),
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
                diff_structs(
                    &ea,
                    &eb,
                    &format!("{field_path}[{i}]"),
                    &format!("{base_field_path}[{i}]"),
                    names,
                    out,
                    limit,
                );
            }
        } else if let (Some(sa), Some(sb)) = (fa.as_struct(), fb.as_struct()) {
            diff_structs(&sa, &sb, &field_path, &base_field_path, names, out, limit);
        } else if let (Some(da), Some(db)) = (fa.as_data(), fb.as_data()) {
            // Raw data fields -- function curves, shader source, import info --
            // are edited through their own editors and land in an exported mod
            // like any other change. They have no readable form here, but a
            // preview that omits them entirely is worse than one that says only
            // that they moved.
            if da != db {
                out.push(TagFieldDiff {
                    path: field_path,
                    base_path: Some(base_field_path),
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
                    base_path: Some(base_field_path),
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
                    dump_struct(&element, &format!("{field_path}[{i}]"), names, out, limit, added,);
                }
            }
        } else if let Some(array) = field.as_array() {
            for i in 0..array.len() {
                if let Some(element) = array.element(i) {
                    dump_struct(&element, &format!("{field_path}[{i}]"), names, out, limit, added,);
                }
            }
        } else if let Some(inner) = field.as_struct() {
            dump_struct(&inner, &field_path, names, out, limit, added);
        } else if let Some(value) = field.value() {
            let text = foundation::format_foundation_scalar_value(names, &value);
            out.push(TagFieldDiff {
                path: field_path.clone(),
                base_path: (!added).then_some(field_path),
                a: if added { String::new() } else { text.clone() },
                b: if added { text } else { String::new() },
            });
        }
    }
}

#[cfg(test)]
mod alignment_tests {
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
        let pairs = super::align_by_content(&a, &b, &distinct(a.len()), &distinct_from(a.len(), b.len())).expect("aligns");
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
        let pairs = super::align_by_content(&a, &b, &distinct(a.len()), &distinct_from(a.len(), b.len())).expect("aligns");
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
        let pairs = super::align_by_content(&a, &b, &distinct(a.len()), &distinct_from(a.len(), b.len())).expect("aligns");
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
}
#[cfg(test)]
mod base_path_tests {
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
}
#[cfg(test)]
mod deletion_repro_tests {
    use super::*;

    const PAKS: &str = "/Users/camden/Halo/halo-campaign-evolved_pc/Meteorite/Content/Paks";

    fn read_a15() -> Option<TagFile> {
        if !std::path::Path::new(PAKS).exists() {
            return None;
        }
        let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let names = crate::format::TagNameIndex::load_from_definitions(&defs);
        let loaded = crate::source::load_iostore_container_set(
            std::path::PathBuf::from(PAKS),
            &names,
            &defs,
        )
        .ok()?;
        let entry = loaded
            .entries
            .iter()
            .find(|entry| entry.display_path.ends_with("a15.scenario"))?
            .clone();
        crate::source::read_entry(&loaded.source, &entry).ok()
    }

    /// Deleting one `zone set pvs` element reported a screen of changes through
    /// everything below it, each value shifted by exactly one position -- the
    /// signature of comparing element n against element n+1.
    ///
    /// These elements are a version, a mask and some flags, with everything
    /// that tells them apart in nested blocks, so a fingerprint of the fixed
    /// data alone could not distinguish them and the aligner paired the wrong
    /// ones.
    #[test]
    fn deleting_a_zone_set_reports_only_that_deletion() {
        let (Some(base), Some(mut edited)) = (read_a15(), read_a15()) else {
            eprintln!("skipping: Campaign Evolved not present");
            return;
        };
        let names = crate::format::TagNameIndex::default();
        let before = base
            .root()
            .fields_all()
            .find_map(|field| {
                (field.name() == "zone set pvs")
                    .then(|| field.as_block())
                    .flatten()
                    .map(|block| block.len())
            })
            .expect("a zone set pvs block");
        assert!(before > 4, "need several elements to shift, got {before}");

        let mut dirty = Dirty::default();
        crate::app::apply_block_ops(
            &mut edited,
            vec![BlockOp {
                path: "zone set pvs".to_owned(),
                kind: BlockOpKind::Delete(3),
            }],
            &mut dirty,
        );

        let (rows, _) = diff_tags(&base, &edited, &names, 5000);
        let removals: Vec<&TagFieldDiff> = rows
            .iter()
            .filter(|row| row.b.is_empty() && row.a.starts_with("removed"))
            .collect();
        assert_eq!(
            removals.len(),
            1,
            "one element was deleted, so one removal: {:?}",
            removals.iter().map(|row| &row.path).collect::<Vec<_>>()
        );

        // Everything else in the tag is untouched by a deletion further up, so
        // nothing outside the removed element may be reported as changed.
        let removed = &removals[0].path;
        let strays: Vec<&String> = rows
            .iter()
            .filter(|row| !row.path.starts_with(removed.as_str()))
            .map(|row| &row.path)
            .collect();
        assert!(
            strays.is_empty(),
            "{} field(s) outside the deleted element reported as changed, e.g. {:?}",
            strays.len(),
            strays.iter().take(5).collect::<Vec<_>>()
        );
    }

    /// The reported case, in full: an element deleted from `zone set pvs` and
    /// a value edited inside one that shifts up to take its place.
    ///
    /// The edit changes that element's contents, so it drops out of the common
    /// subsequence and used to be paired positionally against its neighbour --
    /// reporting every field below as changed, each shifted by one position.
    #[test]
    fn editing_an_element_that_also_shifts_is_still_the_same_element() {
        let (Some(base), Some(mut edited)) = (read_a15(), read_a15()) else {
            eprintln!("skipping: Campaign Evolved not present");
            return;
        };
        let names = crate::format::TagNameIndex::default();
        let mut dirty = Dirty::default();
        crate::app::apply_block_ops(
            &mut edited,
            vec![BlockOp {
                path: "zone set pvs".to_owned(),
                kind: BlockOpKind::Delete(3),
            }],
            &mut dirty,
        );
        // Element 4 is now element 3. Editing inside it is what defeated the
        // alignment.
        let applied = crate::app::apply_pending_edits(
            &mut edited,
            vec![PendingFieldEdit {
                path: "zone set pvs[3]/bsp checksums[0]/bsp checksum".to_owned(),
                input: "12345".to_owned(),
            }],
            &mut dirty,
        );
        assert!(
            applied.status.is_some(),
            "the edit has to land for this to test anything"
        );

        let (rows, _) = diff_tags(&base, &edited, &names, 5000);
        let modifications: Vec<&TagFieldDiff> = rows
            .iter()
            .filter(|row| !row.a.is_empty() && !row.b.is_empty())
            .collect();
        assert_eq!(
            modifications.len(),
            1,
            "one value was edited, so one modification: {:?}",
            modifications
                .iter()
                .map(|row| (&row.path, &row.a, &row.b))
                .collect::<Vec<_>>()
        );
        assert_eq!(modifications[0].b, "12345");
    }

    /// Deleting one element and adding another, as reported.
    #[test]
    fn a_deletion_and_an_addition_change_no_values() {
        let (Some(base), Some(mut edited)) = (read_a15(), read_a15()) else {
            eprintln!("skipping: Campaign Evolved not present");
            return;
        };
        let names = crate::format::TagNameIndex::default();
        let mut dirty = Dirty::default();
        crate::app::apply_block_ops(
            &mut edited,
            vec![
                BlockOp {
                    path: "zone set pvs".to_owned(),
                    kind: BlockOpKind::Delete(3),
                },
                BlockOp {
                    path: "zone sets".to_owned(),
                    kind: BlockOpKind::Add,
                },
            ],
            &mut dirty,
        );
        // The dialog compares the shipped tag against the project overlay,
        // which is the edited tag serialized and read back -- not the
        // in-memory one it was edited in.
        let bytes = edited.write_to_bytes().expect("serialize the edited tag");
        let edited = TagFile::read_from_bytes(&bytes).expect("read the overlay back");
        let (rows, _) = diff_tags(&base, &edited, &names, 5000);
        // Removing one element and adding another changes no value anywhere,
        // so nothing may be reported as modified: renumbering is not an edit.
        let modifications: Vec<&TagFieldDiff> = rows
            .iter()
            .filter(|row| !row.a.is_empty() && !row.b.is_empty())
            .collect();
        assert!(
            modifications.is_empty(),
            "{} field(s) reported as changed by a deletion and an addition: {:?}",
            modifications.len(),
            modifications
                .iter()
                .take(5)
                .map(|row| (&row.path, &row.a, &row.b))
                .collect::<Vec<_>>()
        );
        assert!(
            rows.iter().any(|row| row.b.starts_with("added")),
            "the new element should be reported"
        );
        assert!(
            rows.iter().any(|row| row.a.starts_with("removed")),
            "the deleted element should be reported"
        );
    }
}
