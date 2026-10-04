use super::*;

#[test]
fn searchable_text_separator_only_appears_between_values() {
    let mut blob = String::new();

    append_searchable_text(&mut blob, "First");
    assert_eq!(blob, "first");

    append_searchable_text(&mut blob, "Second");
    assert_eq!(blob, "first · second");

    append_searchable_text(&mut blob, "Third");
    assert_eq!(blob, "first · second · third");
    assert!(!blob.starts_with(" · "));
    assert!(!blob.contains(" ·  · "));
}

#[test]
fn searchable_text_extracts_text_kinds_only() {
    assert_eq!(
        field_searchable_text(Some(TagFieldData::String("Hello".to_owned()))).as_deref(),
        Some("Hello")
    );
    assert_eq!(
        field_searchable_text(Some(TagFieldData::CharEnum {
            value: 1,
            name: Some("alert".to_owned()),
        }))
        .as_deref(),
        Some("alert")
    );
    // Numbers / padding carry no searchable text.
    assert_eq!(
        field_searchable_text(Some(TagFieldData::LongInteger(42))),
        None
    );
    assert_eq!(field_searchable_text(None), None);
}

#[test]
fn first_match_finds_a_string_id_value_and_path() {
    let mut tag = TagFile::new("definitions/halo2_mcc/model.json").unwrap();
    let mut dirty = Dirty::default();
    apply_model_variant_ops(
        &mut tag,
        vec![ModelVariantOp::Create {
            name: "myhero".to_owned(),
            regions: Vec::new(),
        }],
        &mut dirty,
    );
    let hit = first_field_value_match(&tag.root(), "hero", "");
    let (path, value) = hit.expect("variant name should match 'hero'");
    assert!(value.to_ascii_lowercase().contains("hero"));
    assert!(path.to_ascii_lowercase().contains("variant"));
    assert!(
        first_field_value_match(&tag.root(), "zzz-not-present", "").is_none(),
        "absent text should not match"
    );
}
