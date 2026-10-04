//! Outbound tag-reference reports: the whole tree a tag pulls in, as text.
//! It owns the report's traversal and formatting; index construction, path
//! normalization, and file output belong to the export that asks for it.

use super::*;
use crate::core::source::{DependencyRef, ReverseDependencyIndex, dependency_key};
use std::collections::HashSet;
use std::fmt::Write as _;

/// Marks a tag whose own references were already listed further up the file.
///
/// Halo tag graphs contain cycles — a shader reaching a bitmap that reaches a
/// shader — and plenty of shared leaves besides. Expanding a tag once and
/// pointing later occurrences back at it is what keeps the report finite, and
/// keeps a model with forty shaders on one texture from printing that texture's
/// subtree forty times.
const REPEAT_MARKER: &str = "(see above)";

/// A dependency the index recorded but no loaded tag answers to.
const MISSING_MARKER: &str = "(missing)";

const INDENT: &str = "  ";

/// One tag's outbound references, recursively, as an indented tree.
///
/// `by_dependency_key` resolves a [`DependencyRef`] to the entry it names; the
/// caller builds it once over the whole entry set, because doing it per node
/// turns the walk quadratic on a kit with a hundred thousand tags.
pub(in crate::app) fn tag_reference_tree_text(
    index: &ReverseDependencyIndex,
    by_dependency_key: &HashMap<String, TagEntry>,
    root: &TagEntry,
) -> String {
    let mut visited = HashSet::new();
    let mut body = String::new();
    let mut deepest = 0usize;
    visited.insert(root.key.clone());
    write_node(
        &mut body,
        index,
        by_dependency_key,
        &root.key,
        &root.display_path,
        0,
        &mut visited,
        &mut deepest,
    );

    let mut report = String::new();
    let _ = writeln!(report, "Tag references for {}", root.display_path);
    let _ = writeln!(
        report,
        "{} tags, {} levels deep. A tag already listed above is marked \"{REPEAT_MARKER}\" \
         instead of being expanded again.",
        visited.len(),
        deepest,
    );
    report.push('\n');
    report.push_str(&body);
    report
}

#[allow(clippy::too_many_arguments)]
fn write_node(
    out: &mut String,
    index: &ReverseDependencyIndex,
    by_dependency_key: &HashMap<String, TagEntry>,
    key: &str,
    display_path: &str,
    depth: usize,
    visited: &mut HashSet<String>,
    deepest: &mut usize,
) {
    *deepest = (*deepest).max(depth);
    let _ = writeln!(out, "{}{display_path}", INDENT.repeat(depth));
    for dep in index.dependencies_of(key) {
        // A marked or missing line is still a line the reader sees at this
        // depth, so it counts toward the reported depth like any other.
        *deepest = (*deepest).max(depth + 1);
        let Some(entry) = by_dependency_key.get(&dependency_key(dep.group_tag, &dep.rel_path))
        else {
            let _ = writeln!(
                out,
                "{}{} {MISSING_MARKER}",
                INDENT.repeat(depth + 1),
                missing_label(dep),
            );
            continue;
        };
        // Insert returns false when this tag has been expanded already, which is
        // exactly the cycle-and-shared-leaf case the marker exists for.
        if visited.insert(entry.key.clone()) {
            write_node(
                out,
                index,
                by_dependency_key,
                &entry.key,
                &entry.display_path,
                depth + 1,
                visited,
                deepest,
            );
        } else {
            let _ = writeln!(
                out,
                "{}{} {REPEAT_MARKER}",
                INDENT.repeat(depth + 1),
                entry.display_path,
            );
        }
    }
}

/// A dependency with no loaded tag behind it, named the way the index recorded
/// it: the group is all that is left to identify it by.
fn missing_label(dep: &DependencyRef) -> String {
    match group_tag_to_extension(dep.group_tag) {
        Some(extension) => format!("{}.{extension}", dep.rel_path),
        None => dep.rel_path.clone(),
    }
}

#[cfg(test)]
mod tests {
    //! Right-click → Dump Tag References, the recursive outbound report.
    //!
    //! The two things a recursive walk over a Halo tag graph can get wrong are
    //! covered here: it must terminate on a cycle, and it must not re-expand a tag
    //! that many others share. Both fail the same way — an enormous or endless
    //! file — so both fixtures below are graphs where a naive walk would not stop.

    use super::*;

    fn entry(path: &str, group: &[u8; 4]) -> TagEntry {
        let group_tag = u32::from_be_bytes(*group);
        TagEntry {
            key: format!("file:{path}"),
            display_path: format!("{path}.{}", std::str::from_utf8(group).unwrap()),
            group_tag,
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(path)),
        }
    }

    fn dep(path: &str, group: &[u8; 4]) -> DependencyRef {
        DependencyRef {
            group_tag: u32::from_be_bytes(*group),
            rel_path: path.to_owned(),
        }
    }

    /// The lookup the export builds once over the whole entry set. Keyed the
    /// same way, so the fixtures exercise the real resolution path.
    fn lookup(entries: &[TagEntry]) -> HashMap<String, TagEntry> {
        entries
            .iter()
            .map(|entry| {
                (
                    dependency_key(entry.group_tag, trim_group(&entry.display_path)),
                    entry.clone(),
                )
            })
            .collect()
    }

    fn trim_group(display_path: &str) -> &str {
        display_path
            .rsplit_once('.')
            .map_or(display_path, |(p, _)| p)
    }

    /// The tree alone, without the two header lines.
    ///
    /// The header explains what the markers mean and so contains their text; a
    /// count taken over the whole report would find every marker twice.
    fn body(report: &str) -> &str {
        report.split_once("\n\n").map_or(report, |(_, body)| body)
    }

    #[test]
    fn a_reference_cycle_terminates_and_marks_the_repeat() {
        // shader → bitmap → shader. A walk with no visited set never returns.
        let entries = vec![
            entry("shaders/wall", b"shad"),
            entry("bitmaps/wall", b"bitm"),
        ];
        let mut index = ReverseDependencyIndex::default();
        index.set_tag_dependencies(
            "file:shaders/wall".to_owned(),
            [dep("bitmaps/wall", b"bitm")],
        );
        index.set_tag_dependencies(
            "file:bitmaps/wall".to_owned(),
            [dep("shaders/wall", b"shad")],
        );

        let report = tag_reference_tree_text(&index, &lookup(&entries), &entries[0]);

        let body = body(&report);
        assert!(
            body.contains(&format!("shaders/wall.shad {REPEAT_MARKER}")),
            "the cycle's second visit must be marked, not expanded: {report}"
        );
        // Once expanded as the root, once marked on the way back. Without the
        // visited set the second would expand and the walk would never stop.
        assert_eq!(body.matches("shaders/wall.shad").count(), 2, "{report}");
        assert_eq!(body.matches("bitmaps/wall.bitm").count(), 1, "{report}");
    }

    #[test]
    fn a_shared_leaf_is_expanded_once_not_once_per_parent() {
        // Two shaders on one bitmap: the second must point back, not re-expand.
        let entries = vec![
            entry("objects/crate", b"mode"),
            entry("shaders/a", b"shad"),
            entry("shaders/b", b"shad"),
            entry("bitmaps/shared", b"bitm"),
        ];
        let mut index = ReverseDependencyIndex::default();
        index.set_tag_dependencies(
            "file:objects/crate".to_owned(),
            [dep("shaders/a", b"shad"), dep("shaders/b", b"shad")],
        );
        index.set_tag_dependencies(
            "file:shaders/a".to_owned(),
            [dep("bitmaps/shared", b"bitm")],
        );
        index.set_tag_dependencies(
            "file:shaders/b".to_owned(),
            [dep("bitmaps/shared", b"bitm")],
        );

        let report = tag_reference_tree_text(&index, &lookup(&entries), &entries[0]);

        assert_eq!(
            body(&report).matches(REPEAT_MARKER).count(),
            1,
            "only the second parent's copy should be marked: {report}"
        );
        assert!(
            report.contains("4 tags"),
            "the count is unique tags, not visits: {report}"
        );
        assert!(report.contains("2 levels deep"), "{report}");
    }

    #[test]
    fn a_dependency_with_no_loaded_tag_is_named_and_marked_missing() {
        let entries = vec![entry("objects/crate", b"mode")];
        let mut index = ReverseDependencyIndex::default();
        index.set_tag_dependencies(
            "file:objects/crate".to_owned(),
            [dep("shaders/gone", b"shad")],
        );

        let report = tag_reference_tree_text(&index, &lookup(&entries), &entries[0]);

        assert!(
            body(&report).contains(&format!("shaders/gone.shader {MISSING_MARKER}")),
            "a missing dependency is still worth naming: {report}"
        );
    }

    #[test]
    fn a_tag_that_references_nothing_still_reports_itself() {
        let entries = vec![entry("bitmaps/lone", b"bitm")];
        let index = ReverseDependencyIndex::default();

        let report = tag_reference_tree_text(&index, &lookup(&entries), &entries[0]);

        assert!(report.contains("bitmaps/lone.bitm"), "{report}");
        assert!(report.contains("1 tags"), "{report}");
        assert!(report.contains("0 levels deep"), "{report}");
    }

    /// Indentation is what makes the file readable as a tree, so it is worth
    /// pinning rather than leaving to whatever `write_node` happens to do.
    #[test]
    fn depth_is_two_spaces_per_level() {
        let entries = vec![
            entry("objects/crate", b"mode"),
            entry("shaders/a", b"shad"),
            entry("bitmaps/b", b"bitm"),
        ];
        let mut index = ReverseDependencyIndex::default();
        index.set_tag_dependencies("file:objects/crate".to_owned(), [dep("shaders/a", b"shad")]);
        index.set_tag_dependencies("file:shaders/a".to_owned(), [dep("bitmaps/b", b"bitm")]);

        let report = tag_reference_tree_text(&index, &lookup(&entries), &entries[0]);

        assert!(report.contains("\nobjects/crate.mode\n"), "{report}");
        assert!(report.contains("\n  shaders/a.shad\n"), "{report}");
        assert!(report.contains("\n    bitmaps/b.bitm\n"), "{report}");
    }
}
