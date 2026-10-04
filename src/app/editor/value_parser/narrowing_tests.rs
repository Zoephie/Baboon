use super::*;

/// `crate` is `bloc` in every game that has it; there is no `crat` group.
#[test]
fn the_fallback_table_files_a_crate_under_bloc() {
    assert_eq!(
        fallback_extension_to_group_tag("crate"),
        Some(u32::from_be_bytes(*b"bloc"))
    );
    assert_eq!(
        fallback_extension_to_group_tag("device_control"),
        Some(u32::from_be_bytes(*b"ctrl"))
    );
}

/// Run `check` on the first field of `wanted` type found in a fresh tag of
/// any Halo 3 group, following inline structs (a fresh tag's blocks are
/// empty). Fails rather than skips when there is none, so the test can
/// never pass by finding nothing.
fn with_field(wanted: TagFieldType, check: impl Fn(&TagField<'_>)) {
    fn find<'a>(root: TagStruct<'a>, wanted: TagFieldType) -> Option<TagField<'a>> {
        root.fields_all().find_map(|field| {
            if field.field_type() == wanted {
                return Some(field);
            }
            field.as_struct().and_then(|inner| find(inner, wanted))
        })
    }
    let schemas = locate_definitions_root().join("halo3_mcc");
    let mut paths: Vec<_> = std::fs::read_dir(&schemas)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "json")
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('_'))
        })
        .collect();
    paths.sort();
    for path in paths {
        let Ok(tag) = TagFile::new(&path) else {
            continue;
        };
        if let Some(field) = find(tag.root(), wanted) {
            check(&field);
            return;
        }
    }
    panic!("no fresh Halo 3 tag has a {wanted:?} field outside a block");
}

/// A typed value that does not fit its field is an error. These were `as`
/// casts, which wrap: 200 in a char block index became -56 and 0x1FF in
/// byte flags became 0xFF, both written without a word.
#[test]
fn a_typed_value_that_does_not_fit_its_field_is_refused() {
    with_field(TagFieldType::CharBlockIndex, |field| {
        assert!(
            parse_gui_field_value(field, "200").is_err(),
            "200 in a char block index"
        );
        assert!(matches!(
            parse_gui_field_value(field, "none"),
            Ok(TagFieldData::CharBlockIndex(-1))
        ));
        assert!(matches!(
            parse_gui_field_value(field, "127"),
            Ok(TagFieldData::CharBlockIndex(127))
        ));
    });
    with_field(TagFieldType::ByteFlags, |field| {
        assert!(
            parse_gui_field_value(field, "0x1FF").is_err(),
            "0x1FF in byte flags"
        );
        assert!(matches!(
            parse_gui_field_value(field, "0xFF"),
            Ok(TagFieldData::ByteFlags { value: 0xFF, .. })
        ));
    });
    with_field(TagFieldType::LongFlags, |field| {
        // A mask is a bit pattern: every bit of the field is fair game.
        assert!(matches!(
            parse_gui_field_value(field, "0xFFFFFFFF"),
            Ok(TagFieldData::LongFlags { value: -1, .. })
        ));
        assert!(parse_gui_field_value(field, "0x100000000").is_err());
    });
    with_field(TagFieldType::CharEnum, |field| {
        let Some(blam_tags::TagOptions::Enum { names, .. }) = field.options() else {
            panic!("a char enum without options");
        };
        let past_the_end = names.len().to_string();
        assert!(
            parse_gui_field_value(field, &past_the_end).is_err(),
            "index {past_the_end} of {} options",
            names.len()
        );
        assert!(
            parse_gui_field_value(field, "300").is_err(),
            "300 in a char enum"
        );
        assert!(parse_gui_field_value(field, "0").is_ok());
    });
}
