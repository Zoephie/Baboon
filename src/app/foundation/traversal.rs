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

/// Strip only element subscripts (`[N]`) from a field path, preserving field
/// ordinals (`#N`). Two paths that differ only in which parent block element
/// was selected normalize to the same string, so the block clipboard can gate
/// paste on the block's *schema* position rather than the concrete instance
/// (e.g. `damage sections#3[0]/instant responses#5` and `…[1]/…#5` both become
/// `damage sections#3/instant responses#5`). Keeping the `#N` ordinal still
/// distinguishes genuinely different same-named sibling blocks.
pub(in crate::app) fn strip_element_indices(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut skipping = false;
    for ch in path.chars() {
        match ch {
            '[' => skipping = true,
            ']' => skipping = false,
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

/// Per-pane temporary request emitted by a filtered block header. Kept under
/// the field-edit scope so split views of the same tag cannot consume each
/// other's jump.
pub(in crate::app) fn field_search_block_jump_id(
    view_scope: &str,
    tag_key: &str,
) -> egui::Id {
    egui::Id::new((
        "field_edit",
        view_scope,
        tag_key,
        "field_search_block_jump",
    ))
}

/// Return the exact renderer path of the first field/container label matching
/// `query`, in the same document order and with the same case-insensitive
/// substring semantics as "Search fields". Unlike the filter's canonical path
/// set, this retains ordinals and block element indices so field navigation can
/// select the right element and land on the exact rendered row.
pub(in crate::app) fn first_field_label_match(tag: &TagFile, query: &str) -> Option<String> {
    field_label_matches(tag, query).into_iter().next()
}

/// Return every matching field/container renderer path in document order.
/// One path is emitted per matching row even when the query appears more than
/// once in its label, so repeated Jump/Enter presses advance predictably.
pub(in crate::app) fn field_label_matches(tag: &TagFile, query: &str) -> Vec<String> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    collect_label_matches(tag.root(), &query, true, "", &mut matches);
    matches
}

fn collect_label_matches(
    tag_struct: TagStruct<'_>,
    query: &str,
    following_inherited_chain: bool,
    prefix: &str,
    matches: &mut Vec<String>,
) {
    for field in tag_struct.fields() {
        let label = clean_field_name(field.name());
        let inherited_wrapper = following_inherited_chain
            && field.as_struct().is_some()
            && is_inherited_parent_name(field.name());
        let path = if inherited_wrapper {
            append_field_path(prefix, field.clean_name().as_ref())
        } else {
            append_field_path_for(prefix, &field)
        };

        if label.to_ascii_lowercase().contains(query) {
            matches.push(path.clone());
        }

        if let Some(block) = field.as_block() {
            for index in 0..block.len() {
                if let Some(element) = block.element(index) {
                    collect_label_matches(
                        element,
                        query,
                        false,
                        &format!("{path}[{index}]"),
                        matches,
                    );
                }
            }
        } else if let Some(array) = field.as_array() {
            for index in 0..array.len() {
                if let Some(element) = array.element(index) {
                    collect_label_matches(
                        element,
                        query,
                        false,
                        &format!("{path}[{index}]"),
                        matches,
                    );
                }
            }
        } else if let Some(nested) = field.as_struct() {
            collect_label_matches(nested, query, inherited_wrapper, &path, matches);
        }
    }
}

/// Per-pane field-search cycling state, kept in egui temporary memory. The
/// normalized query travels with the next index so editing the search resets
/// the sequence without adding persistent project/session state.
#[derive(Clone, Default)]
pub(in crate::app) struct FieldSearchCursor {
    query: String,
    next: usize,
}

impl FieldSearchCursor {
    pub(in crate::app) fn advance(&mut self, query: &str, match_count: usize) -> Option<usize> {
        let query = query.trim().to_ascii_lowercase();
        if self.query != query {
            self.query = query;
            self.next = 0;
        }
        if match_count == 0 {
            return None;
        }
        let current = self.next.min(match_count - 1);
        self.next = (current + 1) % match_count;
        Some(current)
    }
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
        let clean = clean_field_name(field.name());
        let name_matches = clean.to_ascii_lowercase().contains(query);
        // Canonical path = CLEAN field names joined by '/', no element indices or
        // ordinals. Must match `strip_node_indices` of the render-walk paths,
        // which are also built from clean names (see `append_field_path_for`).
        let canon = if canon_prefix.is_empty() {
            clean.clone()
        } else {
            format!("{canon_prefix}/{clean}")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn object_with_two_ai_property_elements() -> TagFile {
        let mut tag = TagFile::new(crate::app::test_definition_path("halo2_mcc/object.json"))
            .expect("object test definition");
        let field_index = tag
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_name(field.name()) == "ai properties")
            .expect("object schema has ai properties")
            .0;
        let mut root = tag.root_mut();
        let mut field = root.field_at_mut(field_index).expect("ai properties field");
        let mut block = field.as_block_mut().expect("ai properties block");
        block.add_element();
        block.add_element();
        tag
    }

    #[test]
    fn first_label_match_is_trimmed_case_insensitive_and_exactly_indexed() {
        let tag = object_with_two_ai_property_elements();
        let hit = first_field_label_match(&tag, "  AI TYPE NAME  ")
            .expect("nested field label should match");
        let ai_properties = tag
            .root()
            .fields()
            .find(|field| clean_field_name(field.name()) == "ai properties")
            .unwrap();
        let block_path = append_field_path_for("", &ai_properties);
        let element = ai_properties.as_block().unwrap().element(0).unwrap();
        let ai_type_name = element
            .fields()
            .find(|field| clean_field_name(field.name()) == "ai type name")
            .unwrap();
        let expected = append_field_path_for(&format!("{block_path}[0]"), &ai_type_name);
        assert_eq!(hit, expected);
    }

    #[test]
    fn container_label_match_lands_on_the_container_before_its_children() {
        let tag = object_with_two_ai_property_elements();
        let hit =
            first_field_label_match(&tag, "ai properties").expect("container label should match");
        assert!(
            !hit.contains('['),
            "container path must target its header: {hit}"
        );
        assert!(hit.starts_with("ai properties#"), "renderer path: {hit}");
    }

    #[test]
    fn blank_and_missing_label_queries_have_no_jump_target() {
        let tag = object_with_two_ai_property_elements();
        assert_eq!(first_field_label_match(&tag, "   "), None);
        assert_eq!(
            first_field_label_match(&tag, "definitely not a field"),
            None
        );
    }

    #[test]
    fn all_label_matches_follow_document_and_element_order() {
        let tag = object_with_two_ai_property_elements();
        let matches = field_label_matches(&tag, "ai type name");
        assert_eq!(matches.len(), 2);
        assert!(matches[0].contains("[0]"), "first element: {}", matches[0]);
        assert!(matches[1].contains("[1]"), "second element: {}", matches[1]);
    }

    #[test]
    fn search_cursor_cycles_wraps_and_resets_when_the_query_changes() {
        let mut cursor = FieldSearchCursor::default();
        assert_eq!(cursor.advance("barrels", 3), Some(0));
        assert_eq!(cursor.advance("barrels", 3), Some(1));
        assert_eq!(cursor.advance("barrels", 3), Some(2));
        assert_eq!(cursor.advance("barrels", 3), Some(0));
        assert_eq!(cursor.advance("firing", 2), Some(0));
        assert_eq!(cursor.advance("missing", 0), None);
        assert_eq!(cursor.advance("barrels", 3), Some(0));
    }

    #[test]
    fn clearing_after_a_search_retains_the_restore_defaults_pass() {
        let tag = object_with_two_ai_property_elements();
        let key = "test.object";
        let mut searches = HashMap::from([(key.to_owned(), "ai type".to_owned())]);
        let mut applied = HashMap::new();
        assert!(matches!(
            compute_pending_field_filter(&tag, true, key, &searches, &mut applied),
            Some(FieldFilterAction::Apply(_))
        ));

        searches.get_mut(key).unwrap().clear();
        assert!(matches!(
            compute_pending_field_filter(&tag, true, key, &searches, &mut applied),
            Some(FieldFilterAction::RestoreDefaults)
        ));
        assert!(!applied.contains_key(key));
    }
}
