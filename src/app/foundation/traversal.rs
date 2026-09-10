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

/// Whether a tag supports filtering its field tree from Find. Shader/material
/// tags are excluded because they use the dedicated grid surface rather than
/// the block tree; every other tag (including sound tags, which have a full
/// field tree below their audition surface) supports it.
pub(in crate::app) fn supports_field_search(entry: &TagEntry) -> bool {
    !(is_material_tag(entry) || is_material_shader_tag(entry) || is_shader_tag(entry))
}

/// Build the visible field set for Find's optional filter mode using the same
/// targets and matching options as the occurrence index.
pub(in crate::app) fn compute_find_field_filter(
    tag: &TagFile,
    names: &TagNameIndex,
    docs: Option<&DefDocs>,
    query: &str,
    look_in: FindLookIn,
    match_case: bool,
    whole_word: bool,
) -> FieldFilter {
    let mut visible_paths = std::collections::HashSet::new();
    collect_find_visible_paths(
        tag.root(),
        names,
        docs,
        "",
        query,
        look_in,
        match_case,
        whole_word,
        false,
        &mut visible_paths,
    );
    FieldFilter { visible_paths }
}

#[allow(clippy::too_many_arguments)]
fn collect_find_visible_paths(
    tag_struct: TagStruct<'_>,
    names: &TagNameIndex,
    docs: Option<&DefDocs>,
    canon_prefix: &str,
    query: &str,
    look_in: FindLookIn,
    match_case: bool,
    whole_word: bool,
    under_matched_container: bool,
    visible_paths: &mut std::collections::HashSet<String>,
) -> bool {
    let mut any = false;
    if look_in.includes_blocks() {
        let entries = docs
            .map(|docs| docs.entries_for(&tag_struct.definition().guid()))
            .unwrap_or(&[]);
        for (index, entry) in entries.iter().enumerate() {
            let DefEntry::Explanation { title, body } = entry else {
                continue;
            };
            let matches =
                !find_text_ranges(&clean_field_name(title), query, match_case, whole_word)
                    .is_empty()
                    || !find_text_ranges(body.trim_end(), query, match_case, whole_word).is_empty();
            if matches || under_matched_container {
                visible_paths.insert(documentation_path(canon_prefix, index));
                any = true;
            }
        }
    }
    for field in tag_struct.fields() {
        let clean = clean_field_name(field.name());
        let canon = if canon_prefix.is_empty() {
            clean.clone()
        } else {
            format!("{canon_prefix}/{clean}")
        };
        let is_block = field.as_block().is_some() || field.as_array().is_some();
        let is_documentation = field.field_type() == TagFieldType::Explanation;
        let display_label = if is_block {
            foundation_block_title(field.name())
        } else {
            clean.clone()
        };
        let label_enabled = if is_block || is_documentation {
            look_in.includes_blocks()
        } else {
            look_in.includes_field_names()
        };
        let label_matches = label_enabled
            && !find_text_ranges(&display_label, query, match_case, whole_word).is_empty();
        let documentation_body_matches = is_documentation
            && look_in.includes_blocks()
            && field.explanation().is_some_and(|body| {
                !find_text_ranges(body.trim_end(), query, match_case, whole_word).is_empty()
            });
        let value_matches = !is_block
            && !is_documentation
            && look_in.includes_values()
            && field.value().is_some_and(|value| {
                !find_text_ranges(
                    &format_foundation_scalar_value(names, &value),
                    query,
                    match_case,
                    whole_word,
                )
                .is_empty()
            });
        let node_matches = label_matches || documentation_body_matches || value_matches;
        let child_under_matched = under_matched_container || (is_block && label_matches);
        let mut child_matches = false;

        if let Some(nested) = field.as_struct() {
            child_matches |= collect_find_visible_paths(
                nested,
                names,
                docs,
                &canon,
                query,
                look_in,
                match_case,
                whole_word,
                child_under_matched,
                visible_paths,
            );
        } else if let Some(block) = field.as_block() {
            for index in 0..block.len() {
                if let Some(element) = block.element(index) {
                    child_matches |= collect_find_visible_paths(
                        element,
                        names,
                        docs,
                        &canon,
                        query,
                        look_in,
                        match_case,
                        whole_word,
                        child_under_matched,
                        visible_paths,
                    );
                }
            }
        } else if let Some(array) = field.as_array() {
            for index in 0..array.len() {
                if let Some(element) = array.element(index) {
                    child_matches |= collect_find_visible_paths(
                        element,
                        names,
                        docs,
                        &canon,
                        query,
                        look_in,
                        match_case,
                        whole_word,
                        child_under_matched,
                        visible_paths,
                    );
                }
            }
        }

        if node_matches || child_matches || under_matched_container {
            visible_paths.insert(canon);
        }
        any |= node_matches || child_matches;
    }
    any
}

/// Per-pane temporary request emitted by a filtered block header. Kept under
/// the field-edit scope so split views of the same tag cannot consume each
/// other's jump.
pub(in crate::app) fn find_filter_block_jump_id(view_scope: &str, tag_key: &str) -> egui::Id {
    egui::Id::new(("field_edit", view_scope, tag_key, "find_filter_block_jump"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_with_one_ai_properties_element() -> TagFile {
        let mut tag = TagFile::new(crate::app::test_definition_path("halo2_mcc/object.json"))
            .expect("object test definition");
        let field_index = tag
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_name(field.name()) == "ai properties")
            .expect("object schema has ai properties")
            .0;
        tag.root_mut()
            .field_at_mut(field_index)
            .unwrap()
            .as_block_mut()
            .unwrap()
            .add_element();
        tag
    }

    #[test]
    fn matching_a_field_keeps_its_block_ancestor_visible() {
        let filter = compute_find_field_filter(
            &object_with_one_ai_properties_element(),
            &TagNameIndex::default(),
            None,
            "ai type name",
            FindLookIn {
                field_names: true,
                field_values: false,
                blocks: false,
            },
            false,
            false,
        );
        assert!(filter.visible_paths.contains("ai properties"));
        assert!(filter.visible_paths.contains("ai properties/ai type name"));
    }

    #[test]
    fn matching_a_block_keeps_its_contents_visible() {
        let filter = compute_find_field_filter(
            &object_with_one_ai_properties_element(),
            &TagNameIndex::default(),
            None,
            "ai properties",
            FindLookIn {
                field_names: false,
                field_values: false,
                blocks: true,
            },
            false,
            false,
        );
        assert!(filter.visible_paths.contains("ai properties"));
        assert!(filter.visible_paths.contains("ai properties/ai type name"));
    }

    #[test]
    fn documentation_body_match_is_visible_with_blocks_enabled() {
        let tag = TagFile::new(crate::app::test_definition_path("halo3_mcc/model.json"))
            .expect("model test definition");
        let docs = build_def_docs(std::path::Path::new("definitions"), "halo3_mcc", "model");
        let filter = compute_find_field_filter(
            &tag,
            &TagNameIndex::default(),
            Some(&docs),
            "descending order",
            FindLookIn {
                field_names: false,
                field_values: false,
                blocks: true,
            },
            false,
            false,
        );
        assert!(
            filter
                .visible_paths
                .iter()
                .any(|path| path.starts_with("@documentation "))
        );
    }
}
