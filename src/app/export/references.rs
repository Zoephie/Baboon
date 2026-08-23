//! Outbound tag-reference reports: the whole tree a tag pulls in, as text.
//! It owns the report's traversal and formatting; index construction, path
//! normalization, and file output belong to the controller.

use super::*;
use crate::source::{DependencyRef, ReverseDependencyIndex, dependency_key};
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
#[path = "../tests/reference_dump.rs"]
mod tests;
