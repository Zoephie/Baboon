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
