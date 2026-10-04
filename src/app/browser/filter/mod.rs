//! Browser filter parsing, warnings, and match computation.
//! It owns tag-browser filtering and presentation; source discovery, document loading, and edit application belong elsewhere.

use super::*;

pub(in crate::app) fn node_matches(node: &TagTreeNode, entries: &[TagEntry], filter: &str) -> bool {
    node.entries
        .iter()
        .any(|&index| entry_matches(&entries[index], filter))
        || node
            .children
            .iter()
            .any(|child| node_matches(child, entries, filter))
}

pub(in crate::app) fn lazy_node_matches(
    node: &TagTreeNode,
    entries: &[TagEntry],
    filter: &str,
) -> bool {
    // Only show a folder node if it contains files whose NAME matches —
    // don't keep a folder open just because its own path contains the term.
    node.entries
        .iter()
        .any(|&index| entry_matches(&entries[index], filter))
        || node
            .children
            .iter()
            .any(|child| lazy_node_matches(child, entries, filter))
}

/// Whether `haystack` contains `needle`, ignoring ASCII case, without
/// lowercasing a copy of either. For filters that test every row, every frame.
pub(in crate::app) fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    needle.is_empty()
        || haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

pub(in crate::app) fn entry_matches(entry: &TagEntry, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    entry_matches_lower(entry, &filter.to_ascii_lowercase())
}

/// Like [`entry_matches`] but takes an already-lowercased filter, so callers
/// that test many entries against one query don't re-lowercase it each time.
///
/// Query syntax (all case-insensitive):
/// - whitespace = AND (`elite arm` → both terms must match),
/// - `|` = OR (`elite | rifle`),
/// - `^foo` anchors to the start of the filename, `foo$` to the end,
///   `^foo$` is an exact filename match.
///
/// A plain (un-anchored) term matches the filename, the group four-CC, or the
/// group name; anchored terms match the filename only.
fn entry_matches_lower(entry: &TagEntry, filter_lower: &str) -> bool {
    // Match only the filename (last path segment), not parent folder names.
    // A tag at "floodcombat_elite/garbage/hg_arm/hg_arm.model" should NOT
    // appear when searching "elite" — only "elite.model" etc. should match.
    let filename = entry
        .display_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&entry.display_path)
        .to_ascii_lowercase();
    let fourcc = format_group_tag(entry.group_tag).to_ascii_lowercase();
    let group = entry
        .group_name
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut had_term = false;
    for or_group in filter_lower.split('|') {
        let mut group_ok = true;
        let mut group_had_term = false;
        for term in or_group.split_whitespace() {
            group_had_term = true;
            had_term = true;
            if !filter_term_matches(term, &filename, &fourcc, &group) {
                group_ok = false;
                break;
            }
        }
        if group_had_term && group_ok {
            return true;
        }
    }
    // A filter with no real terms (e.g. just "|" or whitespace) matches all.
    !had_term
}

fn filter_term_matches(term: &str, filename: &str, fourcc: &str, group: &str) -> bool {
    let anchored_start = term.starts_with('^');
    let anchored_end = term.ends_with('$') && term.len() > 1;
    let inner = term.trim_start_matches('^');
    let inner = if anchored_end {
        &inner[..inner.len().saturating_sub(1)]
    } else {
        inner
    };
    if inner.is_empty() {
        return true; // a lone anchor matches anything
    }
    match (anchored_start, anchored_end) {
        (true, true) => filename == inner,
        (true, false) => filename.starts_with(inner),
        (false, true) => filename.ends_with(inner),
        (false, false) => {
            filename.contains(inner) || fourcc.contains(inner) || group.contains(inner)
        }
    }
}

/// A human-readable warning for a degenerate browser filter, or `None` when it's
/// well-formed. The boolean grammar (space = AND, `|` = OR, `^`/`$` anchors) has
/// no hard syntax errors, so we flag the cases that silently misbehave: an empty
/// operand around `|`, and a term that is only an anchor.
pub(in crate::app) fn browser_filter_warning(filter: &str) -> Option<String> {
    let trimmed = filter.trim();
    if trimmed.is_empty() {
        return None;
    }
    let operands: Vec<&str> = trimmed.split('|').collect();
    if operands.len() > 1 && operands.iter().any(|operand| operand.trim().is_empty()) {
        return Some("empty term around '|' — that side matches nothing".to_owned());
    }
    for operand in &operands {
        for term in operand.split_whitespace() {
            let inner = term.trim_start_matches('^');
            let inner = inner.strip_suffix('$').unwrap_or(inner);
            if inner.is_empty() {
                return Some(format!("'{term}' is only an anchor — matches everything"));
            }
        }
    }
    None
}

/// Collect the indices of all entries matching `filter`, in display order.
/// Called only when the cached query changes (see [`FilterCache`]), not per
/// frame, so the O(N) lowercase scan happens at most once per keystroke.
pub(in crate::app) fn compute_filter_matches(entries: &[TagEntry], filter: &str) -> Vec<usize> {
    let filter_lower = filter.to_ascii_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry_matches_lower(entry, &filter_lower))
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests;
