use super::*;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn loads_group_names_from_meta_json() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("blam_tag_gui_meta_{stamp}"));
    let game = root.join("halo_test");
    fs::create_dir_all(&game).unwrap();
    fs::write(
        game.join("_meta.json"),
        r#"{"tag_index":{"bipd":"biped","hlmt":"model"}}"#,
    )
    .unwrap();

    let index = TagNameIndex::load_from_definitions(&root);
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(index.name_for(u32::from_be_bytes(*b"bipd")), Some("biped"));
    assert_eq!(
        index.group_tag_for("model"),
        Some(u32::from_be_bytes(*b"hlmt"))
    );
    assert_eq!(
        group_label(&index, u32::from_be_bytes(*b"bipd")),
        "bipd (biped)"
    );
}

#[test]
fn empty_tag_references_format_blank() {
    let index = TagNameIndex::default();
    assert_eq!(
        format_value(
            &index,
            &TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: None
            }),
            false
        ),
        ""
    );
    assert_eq!(
        format_value(
            &index,
            &TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: Some((u32::from_be_bytes(*b"bitm"), "\0".to_owned()))
            }),
            false
        ),
        ""
    );
}

#[test]
fn tag_display_path_converts_to_native_separators() {
    let separator = std::path::MAIN_SEPARATOR;
    assert_eq!(
        to_native_path_string("objects/weapons/rifle.weapon"),
        format!("objects{separator}weapons{separator}rifle.weapon")
    );
}
