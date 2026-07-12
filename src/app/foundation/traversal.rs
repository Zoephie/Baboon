//! Field-path traversal and search-filter preparation.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

pub(in crate::app) fn strip_node_indices(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut skipping = false;
    for ch in path.chars() {
        match ch {
            '/' => {
                skipping = false;
                out.push('/');
            }
            '#' | '[' => skipping = true,
            _ if skipping => {}
            _ => out.push(ch),
        }
    }
    out
}

/// Whether a tag offers the "Search fields" box. Shader/material tags are
/// excluded because they use the dedicated grid surface rather than the block
/// tree; every other tag (including sound tags, which have a full field tree
/// below their audition surface) supports it.
pub(in crate::app) fn supports_field_search(entry: &TagEntry) -> bool {
    !(is_material_tag(entry) || is_material_shader_tag(entry) || is_shader_tag(entry))
}

/// Resolve the field-filter action to apply *this* frame. Returns `Some` only
/// on the frame the (trimmed, lowercased) query changes, so the collapse is a
/// one-shot the user can then adjust by hand. Clearing a previously-applied
/// query yields one `RestoreDefaults` pass that re-expands the editor.
pub(in crate::app) fn compute_pending_field_filter(
    tag: &TagFile,
    supports: bool,
    tag_key: &str,
    field_search: &HashMap<String, String>,
    field_search_applied: &mut HashMap<String, String>,
) -> Option<FieldFilterAction> {
    if !supports {
        return None;
    }
    let query = field_search
        .get(tag_key)
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if query.is_empty() {
        // Re-expand to defaults once, but only if a search was actually active.
        return field_search_applied
            .remove(tag_key)
            .map(|_| FieldFilterAction::RestoreDefaults);
    }
    // Apply every frame while a query is present. Hiding non-matches is a
    // per-frame render decision (not a one-shot collapse), so the filter must
    // stay live.
    field_search_applied.insert(tag_key.to_owned(), query.clone());
    Some(FieldFilterAction::Apply(compute_field_filter(tag, &query)))
}

/// Build the set of collapsible nodes to open for a "Search fields" query:
/// every struct / block / array whose (display) name contains `query`, plus
/// all of their ancestor nodes, plus the ancestors of any matching leaf field.
/// `query` must already be lowercased and non-empty.
pub(in crate::app) fn compute_field_filter(tag: &TagFile, query: &str) -> FieldFilter {
    let mut visible_paths = std::collections::HashSet::new();
    collect_visible_paths(tag.root(), "", query, false, &mut visible_paths);
    FieldFilter { visible_paths }
}

/// Records, in `visible_paths`, every field that should render while searching:
/// a name match, an ancestor container of a match, or anything inside a
/// name-matched container. Anything else is omitted, so a container with no
/// match beneath it is hidden entirely. Returns whether this subtree had a
/// match, so the caller can mark itself an ancestor-of-match.
fn collect_visible_paths(
    tag_struct: TagStruct<'_>,
    canon_prefix: &str,
    query: &str,
    // True when an ancestor container's *own name* matched — its whole subtree is
    // part of that match, so everything under it stays visible.
    under_matched: bool,
    visible_paths: &mut std::collections::HashSet<String>,
) -> bool {
    let mut any = false;
    for field in tag_struct.fields() {
        let name_matches = clean_field_name(field.name())
            .to_ascii_lowercase()
            .contains(query);
        // Canonical path = raw field names joined by '/', no element indices.
        let canon = if canon_prefix.is_empty() {
            field.name().to_owned()
        } else {
            format!("{canon_prefix}/{}", field.name())
        };

        let child_under = under_matched || name_matches;
        let child_matched = if let Some(nested) = field.as_struct() {
            collect_visible_paths(nested, &canon, query, child_under, visible_paths)
        } else if let Some(block) = field.as_block() {
            block
                .element(0)
                .map(|el| collect_visible_paths(el, &canon, query, child_under, visible_paths))
                .unwrap_or(false)
        } else if let Some(array) = field.as_array() {
            array
                .element(0)
                .map(|el| collect_visible_paths(el, &canon, query, child_under, visible_paths))
                .unwrap_or(false)
        } else {
            false
        };

        // A field renders only if it matched, is an ancestor of a match, or lives
        // inside a name-matched container. A container with no match anywhere
        // beneath it never enters the set, so it is hidden entirely.
        if name_matches || child_matched || under_matched {
            visible_paths.insert(canon);
        }
        any |= name_matches || child_matched;
    }
    any
}
