use super::*;
use crate::core::document::value::append_field_path_for;

fn field_names_only() -> FindLookIn {
    FindLookIn {
        field_names: true,
        field_values: false,
        blocks: false,
    }
}

fn tag_with_one_ai_properties_element() -> TagFile {
    let mut tag = TagFile::new(test_definition_path("halo2_mcc/object.json")).unwrap();
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

fn ai_type_name_render_path(tag: &TagFile) -> String {
    let root = tag.root();
    let ai_properties = root
        .fields()
        .find(|field| clean_field_name(field.name()) == "ai properties")
        .expect("object schema has ai properties");
    let block_path = append_field_path_for("", &ai_properties);
    let element = ai_properties
        .as_block()
        .unwrap()
        .element(0)
        .expect("test block has one element");
    let ai_type_name = element
        .fields()
        .find(|field| clean_field_name(field.name()) == "ai type name")
        .expect("ai properties has ai type name");
    append_field_path_for(&format!("{block_path}[0]"), &ai_type_name)
}

fn biped_with_one_inherited_ai_properties_element() -> TagFile {
    let mut tag = TagFile::new(test_definition_path("halo3_mcc/biped.json")).unwrap();
    tag.root_mut()
        .field_path_mut("unit/object/ai properties")
        .expect("biped inherits the object ai properties block")
        .as_block_mut()
        .unwrap()
        .add_element();
    tag
}

fn inherited_ai_type_name_render_path(tag: &TagFile) -> String {
    let unit = tag
        .root()
        .fields()
        .find(|field| is_inherited_parent_name(field.name()))
        .expect("biped has an inherited unit wrapper");
    let unit_struct = unit.as_struct().unwrap();
    let object = unit_struct
        .fields()
        .find(|field| is_inherited_parent_name(field.name()))
        .expect("unit has an inherited object wrapper");
    let object_struct = object.as_struct().unwrap();
    let ai_properties = object_struct
        .fields()
        .find(|field| clean_field_name(field.name()) == "ai properties")
        .expect("object has ai properties");
    let block_path = append_field_path_for("unit/object", &ai_properties);
    let element = ai_properties
        .as_block()
        .unwrap()
        .element(0)
        .expect("test block has one element");
    let ai_type_name = element
        .fields()
        .find(|field| clean_field_name(field.name()) == "ai type name")
        .expect("ai properties has ai type name");
    append_field_path_for(&format!("{block_path}[0]"), &ai_type_name)
}

#[test]
fn collected_nested_path_matches_renderer_ordinal_path() {
    let tag = tag_with_one_ai_properties_element();
    let expected = ai_type_name_render_path(&tag);
    let occurrences = collect_find_occurrences(
        &tag,
        "test.object",
        &TagNameIndex::default(),
        None,
        "ai type name",
        field_names_only(),
        false,
        false,
    );
    let hit = occurrences
        .iter()
        .find(|hit| hit.text == "ai type name")
        .expect("nested label should be collected");
    assert_eq!(hit.field_path, expected);
}

#[test]
fn collected_match_identity_is_accepted_by_widget_lookup() {
    let tag = tag_with_one_ai_properties_element();
    let rendered_path = ai_type_name_render_path(&tag);
    let occurrences = collect_find_occurrences(
        &tag,
        "test.object",
        &TagNameIndex::default(),
        None,
        "ai type name",
        field_names_only(),
        false,
        false,
    );
    let matching_cells = occurrences
        .iter()
        .map(|hit| (hit.tag_key.clone(), hit.field_path.clone(), hit.kind))
        .collect::<HashSet<_>>();
    assert!(matching_cells.contains(&(
        "test.object".to_owned(),
        rendered_path,
        FindTargetKind::Label,
    )));
}

#[test]
fn block_targets_are_independent_from_field_names() {
    let tag = tag_with_one_ai_properties_element();
    let blocks_only = FindLookIn {
        field_names: false,
        field_values: false,
        blocks: true,
    };
    let block_hits = collect_find_occurrences(
        &tag,
        "test.object",
        &TagNameIndex::default(),
        None,
        "ai properties",
        blocks_only,
        false,
        false,
    );
    assert!(
        block_hits
            .iter()
            .any(|hit| hit.kind == FindTargetKind::Block)
    );

    let field_hits = collect_find_occurrences(
        &tag,
        "test.object",
        &TagNameIndex::default(),
        None,
        "ai properties",
        field_names_only(),
        false,
        false,
    );
    assert!(
        field_hits
            .iter()
            .all(|hit| hit.kind != FindTargetKind::Block)
    );
}

#[test]
fn block_search_includes_injected_documentation_titles_and_bodies() {
    let tag = TagFile::new(test_definition_path("halo3_mcc/model.json"))
        .expect("model test definition");
    let docs = build_def_docs(std::path::Path::new("definitions"), GameId::Halo3, "model");
    let blocks_only = FindLookIn {
        field_names: false,
        field_values: false,
        blocks: true,
    };

    let title_hits = collect_find_occurrences(
        &tag,
        "test.model",
        &TagNameIndex::default(),
        Some(&docs),
        "level of detail",
        blocks_only,
        false,
        false,
    );
    assert!(
        title_hits
            .iter()
            .any(|hit| hit.kind == FindTargetKind::Documentation)
    );

    let body_hits = collect_find_occurrences(
        &tag,
        "test.model",
        &TagNameIndex::default(),
        Some(&docs),
        "descending order",
        blocks_only,
        false,
        false,
    );
    assert!(
        body_hits
            .iter()
            .any(|hit| hit.kind == FindTargetKind::Documentation)
    );
}

/// Inheritance wrappers are presentation-only path segments: unlike ordinary
/// fields, `unit/object` intentionally carry no `#ordinal` in Foundation.
#[test]
fn collected_biped_inherited_path_matches_plain_wrapper_renderer_path() {
    let tag = biped_with_one_inherited_ai_properties_element();
    let expected = inherited_ai_type_name_render_path(&tag);
    let occurrences = collect_find_occurrences(
        &tag,
        "test.biped",
        &TagNameIndex::default(),
        None,
        "ai type name",
        field_names_only(),
        false,
        false,
    );
    let hit = occurrences
        .iter()
        .find(|hit| hit.text == "ai type name")
        .expect("inherited nested label should be collected");
    assert_eq!(hit.field_path, expected);
    assert!(expected.starts_with("unit/object/ai properties#"));
}

#[test]
fn ranges_honor_case_and_word_boundaries() {
    assert_eq!(
        find_text_ranges("Brute brute", "brute", false, false),
        vec![0..5, 6..11]
    );
    assert_eq!(
        find_text_ranges("Brute brute", "brute", true, false),
        vec![6..11]
    );
    assert_eq!(
        find_text_ranges("brute brute_captain", "brute", false, true),
        vec![0..5]
    );
}

#[test]
fn ranges_are_non_overlapping_and_empty_query_is_safe() {
    assert_eq!(
        find_text_ranges("aaaa", "aa", true, false),
        vec![0..2, 2..4]
    );
    assert!(find_text_ranges("abc", "", false, false).is_empty());
}

#[test]
fn appending_keeps_label_then_value_occurrence_order() {
    let mut out = Vec::new();
    let names = TagNameIndex::default();
    let mut walk = FindWalk {
        tag_key: "tag",
        names: &names,
        plans: FindPlans::new(None, "needle", false, false),
        query: "needle",
        look_in: FindLookIn::default(),
        match_case: false,
        whole_word: false,
        path: "field".to_owned(),
        out: &mut out,
    };
    walk.append(FindTargetKind::Label, "needle label");
    walk.append(FindTargetKind::Value, "needle value");
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].kind, FindTargetKind::Label);
    assert_eq!(out[0].field_path, "field");
    assert_eq!(out[1].kind, FindTargetKind::Value);
}

/// A match rejected as a partial word still consumes its bytes, as it did
/// when the search ran over lowercased copies.
#[test]
fn rejected_word_match_resumes_after_itself() {
    assert_eq!(find_text_ranges("aaa a", "aa", false, true), Vec::<std::ops::Range<usize>>::new());
    assert_eq!(find_text_ranges("xaa aa", "aa", false, true), vec![4..6]);
    assert_eq!(find_text_ranges("Ünit UNIT", "unit", false, false), vec![6..10]);
    assert!(find_text_has_match("Needle", "needle", false, false));
    assert!(!find_text_has_match("Needle", "needle", true, false));
}

#[test]
fn all_tag_merge_uses_source_order() {
    let occurrence = |key: &str| FindOccurrence {
        tag_key: key.to_owned(),
        field_path: "field".to_owned(),
        kind: FindTargetKind::Value,
        text: "hit".to_owned(),
        range: 0..3,
    };
    let by_key = HashMap::from([
        ("b".to_owned(), vec![occurrence("b")]),
        ("a".to_owned(), vec![occurrence("a")]),
    ]);
    let ordered = order_find_occurrences(&["a".to_owned(), "b".to_owned()], by_key);
    assert_eq!(ordered[0].tag_key, "a");
    assert_eq!(ordered[1].tag_key, "b");
}
