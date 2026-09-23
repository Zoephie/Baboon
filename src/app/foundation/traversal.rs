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
    let mut walk = FindFilterWalk {
        names,
        plans: FindPlans::new(docs, query, match_case, whole_word),
        look_in,
        canon: String::new(),
        visible_paths: std::collections::HashSet::new(),
    };
    walk.collect(tag.root(), false);
    FieldFilter {
        visible_paths: walk.visible_paths,
    }
}

/// One filter walk. `canon` is the canonical path of the node being visited,
/// grown and truncated in place; every element of a block shares its
/// canonical paths, so most visits find theirs already recorded and allocate
/// nothing.
struct FindFilterWalk<'a> {
    names: &'a TagNameIndex,
    plans: FindPlans<'a>,
    look_in: FindLookIn,
    canon: String,
    visible_paths: std::collections::HashSet<String>,
}

impl FindFilterWalk<'_> {
    fn mark_visible(&mut self) {
        if !self.visible_paths.contains(self.canon.as_str()) {
            self.visible_paths.insert(self.canon.clone());
        }
    }

    fn collect(&mut self, tag_struct: TagStruct<'_>, under_matched_container: bool) -> bool {
        let plan = self.plans.plan(&tag_struct);
        let mut any = false;
        if self.look_in.includes_blocks() {
            for (index, entry) in plan.entries.iter().enumerate() {
                if !matches!(entry, DefEntry::Explanation { .. }) {
                    continue;
                }
                let matches = plan.doc_title_matches[index] || plan.doc_body_matches[index];
                if matches || under_matched_container {
                    // Same form as `documentation_path`.
                    let len = self.canon.len();
                    if len > 0 {
                        self.canon.push('/');
                    }
                    let _ = std::fmt::Write::write_fmt(
                        &mut self.canon,
                        format_args!("@documentation {index}"),
                    );
                    self.mark_visible();
                    self.canon.truncate(len);
                    any = true;
                }
            }
        }
        for (field, field_plan) in tag_struct.fields().zip(&plan.fields) {
            let parent_len = self.canon.len();
            if parent_len > 0 {
                self.canon.push('/');
            }
            self.canon.push_str(&field_plan.clean);
            let is_block = field_plan.is_block;
            let is_documentation = field_plan.is_documentation;
            let label_enabled = if is_block || is_documentation {
                self.look_in.includes_blocks()
            } else {
                self.look_in.includes_field_names()
            };
            let label_matches = label_enabled && field_plan.label_matches;
            let documentation_body_matches = is_documentation
                && self.look_in.includes_blocks()
                && field_plan.explanation_matches;
            let value_matches = !is_block
                && !is_documentation
                && self.look_in.includes_values()
                && field.value().is_some_and(|value| {
                    self.plans
                        .matches(&format_foundation_scalar_value(self.names, &value))
                });
            let node_matches = label_matches || documentation_body_matches || value_matches;
            let child_under_matched = under_matched_container || (is_block && label_matches);
            let mut child_matches = false;

            if let Some(nested) = field.as_struct() {
                child_matches |= self.collect(nested, child_under_matched);
            } else if let Some(block) = field.as_block() {
                for index in 0..block.len() {
                    if let Some(element) = block.element(index) {
                        child_matches |= self.collect(element, child_under_matched);
                    }
                }
            } else if let Some(array) = field.as_array() {
                for index in 0..array.len() {
                    if let Some(element) = array.element(index) {
                        child_matches |= self.collect(element, child_under_matched);
                    }
                }
            }

            if node_matches || child_matches || under_matched_container {
                self.mark_visible();
            }
            self.canon.truncate(parent_len);
            any |= node_matches || child_matches;
        }
        any
    }
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
