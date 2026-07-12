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
mod tests {
    use super::*;
    use crate::source::{TagEntry, TagEntryLocation};
    use std::path::PathBuf;

    fn entry(display_path: &str, group: &[u8; 4]) -> TagEntry {
        TagEntry {
            key: display_path.to_owned(),
            display_path: display_path.to_owned(),
            group_tag: u32::from_be_bytes(*group),
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
        }
    }

    #[test]
    fn matches_filename_not_parent_folders() {
        let entries = vec![
            entry("floodcombat_elite/garbage/hg_arm/hg_arm.model", b"mode"),
            entry("characters/elite/elite.model", b"mode"),
        ];
        // "elite" should match only the tag whose *filename* contains it.
        let matches = compute_filter_matches(&entries, "elite");
        assert_eq!(matches, vec![1]);
    }

    #[test]
    fn matches_group_tag_and_is_case_insensitive() {
        let entries = vec![
            entry("fx/spark.effect", b"effe"),
            entry("weapons/rifle.weapon", b"weap"),
        ];
        // Group four-CC match, regardless of query case.
        assert_eq!(compute_filter_matches(&entries, "WEAP"), vec![1]);
    }

    #[test]
    fn reference_input_uses_fourcc_and_backslash_path_without_extension() {
        let e = entry("objects/weapons/rifle/rifle.weapon", b"weap");
        // "weap" four-CC, backslash path, extension stripped.
        assert_eq!(
            entry_reference_input(&e),
            "weap:objects\\weapons\\rifle\\rifle"
        );
    }

    #[test]
    fn malformed_filter_warnings() {
        // Well-formed filters: no warning.
        assert!(browser_filter_warning("").is_none());
        assert!(browser_filter_warning("elite").is_none());
        assert!(browser_filter_warning("arm | rifle").is_none());
        assert!(browser_filter_warning("^elite.model$").is_none());
        assert!(browser_filter_warning("weapon$").is_none());
        // Empty OR operand.
        assert!(browser_filter_warning("foo |").is_some());
        assert!(browser_filter_warning("a || b").is_some());
        // Anchor-only term.
        assert!(browser_filter_warning("^").is_some());
        assert!(browser_filter_warning("foo ^").is_some());
    }

    #[test]
    fn boolean_and_or_and_anchors() {
        let entries = vec![
            entry("characters/elite/elite_arm.model", b"mode"),
            entry("characters/elite/elite.model", b"mode"),
            entry("weapons/rifle.weapon", b"weap"),
        ];
        // AND: both terms must match the same entry.
        assert_eq!(compute_filter_matches(&entries, "elite arm"), vec![0]);
        // OR: either side matches.
        assert_eq!(compute_filter_matches(&entries, "arm | rifle"), vec![0, 2]);
        // Prefix anchor on filename.
        assert_eq!(compute_filter_matches(&entries, "^elite_"), vec![0]);
        // Suffix anchor on filename.
        assert_eq!(compute_filter_matches(&entries, "weapon$"), vec![2]);
        // Exact filename anchor.
        assert_eq!(compute_filter_matches(&entries, "^elite.model$"), vec![1]);
    }

    #[test]
    fn folder_hlsl_include_collector_finds_nested_include_entries() {
        let entries = vec![
            entry("rasterizer/hlsl/ssao.hlsl_include", b"hlsl"),
            entry("rasterizer/hlsl/post/tonemap.hlsl_include", b"hlsl"),
            entry("rasterizer/bitmaps/noise.bitmap", b"bitm"),
        ];
        let tree = crate::source::build_tree(&entries);
        let rasterizer = tree
            .children
            .iter()
            .find(|node| node.label == "rasterizer")
            .expect("rasterizer folder");

        assert_eq!(
            collect_hlsl_include_keys(rasterizer, &entries),
            vec![
                "rasterizer/hlsl/ssao.hlsl_include".to_owned(),
                "rasterizer/hlsl/post/tonemap.hlsl_include".to_owned(),
            ]
        );
    }

    #[test]
    fn folder_material_shader_collector_finds_nested_material_shader_entries() {
        let entries = vec![
            entry(
                "shaders/material_shaders/decals/base.material_shader",
                b"mats",
            ),
            entry(
                "shaders/material_shaders/decals/palette/palette.material_shader",
                b"mats",
            ),
            entry("shaders/material_shaders/decals/noise.bitmap", b"bitm"),
        ];
        let tree = crate::source::build_tree(&entries);
        let shaders = tree
            .children
            .iter()
            .find(|node| node.label == "shaders")
            .expect("shaders folder");

        assert_eq!(
            collect_material_shader_keys(shaders, &entries),
            vec![
                "shaders/material_shaders/decals/base.material_shader".to_owned(),
                "shaders/material_shaders/decals/palette/palette.material_shader".to_owned(),
            ]
        );
    }

    #[test]
    fn tag_extract_menu_is_limited_to_model_and_animation_groups() {
        assert!(supports_tag_extract_menu(u32::from_be_bytes(*b"hlmt")));
        assert!(supports_tag_extract_menu(u32::from_be_bytes(*b"mode")));
        assert!(supports_tag_extract_menu(u32::from_be_bytes(*b"mod2")));
        assert!(supports_tag_extract_menu(u32::from_be_bytes(*b"coll")));
        assert!(supports_tag_extract_menu(u32::from_be_bytes(*b"phmo")));
        assert!(supports_tag_extract_menu(u32::from_be_bytes(*b"jmad")));
        assert!(supports_tag_extract_menu(u32::from_be_bytes(*b"antr")));
        assert!(!supports_tag_extract_menu(u32::from_be_bytes(*b"scnr")));
        assert!(!supports_tag_extract_menu(u32::from_be_bytes(*b"weap")));
    }
}
