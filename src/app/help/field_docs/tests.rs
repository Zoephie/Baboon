use super::*;
use crate::core::document::apply::add_block_element;

#[test]
fn parses_fields_and_explanations_keyed_by_guid() {
    let json = r#"{
            "structs": {
                "s": {
                    "guid": "4015cede9c496f80bcd3fc8804062596",
                    "fields": [
                        {"type":"explanation","name":"attenuation distances","definition":"how it attenuates"},
                        {"type":"real","name":"minimum distance:world units#start attenuating at this distance"},
                        {"type":"string_id","name":"material name^"}
                    ]
                }
            }
        }"#;
    let docs = parse_def_docs(json);
    let guid = parse_guid_hex("4015cede9c496f80bcd3fc8804062596").unwrap();
    let entries = docs.entries_for(guid, "s");
    assert_eq!(entries.len(), 3);
    match &entries[0] {
        DefEntry::Explanation { title, body } => {
            assert_eq!(title, "attenuation distances");
            assert_eq!(body, "how it attenuates");
        }
        _ => panic!("expected explanation first"),
    }
    match &entries[1] {
        DefEntry::Field {
            clean_name,
            help,
            unit,
            ..
        } => {
            assert_eq!(clean_name, "minimum distance");
            assert_eq!(unit.as_deref(), Some("world units"));
            assert_eq!(help.as_deref(), Some("start attenuating at this distance"));
        }
        _ => panic!("expected field"),
    }
    // `material name^` cleans to `material name`.
    match &entries[2] {
        DefEntry::Field { clean_name, .. } => assert_eq!(clean_name, "material name"),
        _ => panic!("expected field"),
    }
}

#[test]
fn parses_tag_reference_allowed_groups() {
    let json = r#"{
            "structs": {
                "s": {
                    "guid": "4015cede9c496f80bcd3fc8804062596",
                    "fields": [
                        {
                            "type":"tag_reference",
                            "name":"render model",
                            "definition":{"flags":0,"allowed":["mode"]}
                        },
                        {
                            "type":"tag_reference",
                            "name":"object",
                            "definition":{"flags":0,"allowed":["bipd","vehi"]}
                        }
                    ]
                }
            }
        }"#;
    let docs = parse_def_docs(json);
    let guid = parse_guid_hex("4015cede9c496f80bcd3fc8804062596").unwrap();
    let entries = docs.entries_for(guid, "s");
    let render_model = parse_group_tag("mode").unwrap();
    let biped = parse_group_tag("bipd").unwrap();
    let vehicle = parse_group_tag("vehi").unwrap();

    assert!(entries.iter().any(|entry| matches!(
        entry,
        DefEntry::Field {
            clean_name,
            tag_reference_allowed,
            ..
        } if clean_name == "render model"
            && tag_reference_allowed.as_slice() == [render_model]
    )));
    assert!(entries.iter().any(|entry| matches!(
        entry,
        DefEntry::Field {
            clean_name,
            tag_reference_allowed,
            ..
        } if clean_name == "object"
            && tag_reference_allowed.as_slice() == [biped, vehicle]
    )));
}

#[test]
fn inheritance_chain_resolves_parent_struct_docs() {
    // biped inherits acceleration scale (unit) + collision damage (object);
    // their struct lives in object.json, reached via the parent_tag chain
    // (biped → unit → obje). build_def_docs must merge it in.
    let docs = build_def_docs(Path::new("definitions"), GameId::Halo3, "biped");
    // The object base struct GUID (where the inherited fields live).
    let guid = parse_guid_hex("6c5aa9947a45fcf55742a488f0943380").unwrap();
    let entries = docs.entries_for(guid, "s");
    assert!(
        !entries.is_empty(),
        "inherited object struct must resolve via the parent chain"
    );
    let has = |target: &str| {
        entries
            .iter()
            .any(|e| matches!(e, DefEntry::Field { clean_name, .. } if clean_name == target))
    };
    assert!(
        has("acceleration scale"),
        "inherited unit field should resolve"
    );
    assert!(
        has("collision damage"),
        "inherited object field should resolve"
    );
    assert!(
        entries
            .iter()
            .any(|e| matches!(e, DefEntry::Explanation { .. })),
        "object struct should carry explanations"
    );
}

#[test]
fn overlay_aligns_with_stripped_tag_by_guid_and_clean_name() {
    // End-to-end: a stripped tag's struct GUID + clean field names line up
    // with the parsed docs, so help/units + explanations can be overlaid.
    let json = std::fs::read_to_string("definitions/haloreach_mcc/sound_classes.json").unwrap();
    let docs = parse_def_docs(&json);
    let mut tag = TagFile::new("definitions/haloreach_mcc/sound_classes.json").unwrap();
    add_block_element(&mut tag, "sound classes").unwrap();
    let classes = tag
        .root()
        .field("sound classes")
        .and_then(|f| f.as_block())
        .unwrap();
    let element = classes.element(0).unwrap();
    let params = element.descend("distance parameters").unwrap();

    // GUID keying matches between the stripped layout and the JSON docs.
    let entries = docs.entries_for_struct(&params);
    assert!(
        !entries.is_empty(),
        "distance-parameters struct must resolve docs by GUID"
    );
    // A stripped tag field name matches a doc entry carrying help + unit.
    assert!(params.field_names().any(|n| n == "minimum distance"));
    assert!(
        entries.iter().any(|e| matches!(
            e,
            DefEntry::Field { clean_name, help, unit, .. }
                if clean_name == "minimum distance"
                    && help.is_some()
                    && unit.as_deref() == Some("world units")
        )),
        "`minimum distance` should overlay help + unit"
    );
    // The sound-class struct supplies explanation rows to inject.
    assert!(
        docs.entries_for_struct(&element)
            .iter()
            .any(|e| matches!(e, DefEntry::Explanation { .. })),
        "sound-class struct should supply explanations"
    );
}

/// Every halo2_mcc struct has the all-zero GUID (Halo 2's definitions carry
/// none, in tool.exe or HABT's XML). Keyed by that, one struct's
/// explanations went to every struct in the tag: masterchief.biped showed the
/// object's collision-damage explanations, empty, inside each `functions`
/// element. Keyed by name, each struct gets only its own.
#[test]
fn halo2_structs_get_only_their_own_explanations() {
    let tag_path = crate::test_kits::tag_path(
        "halo2_mcc",
        "objects/characters/masterchief/masterchief.biped",
    );
    let def = crate::app::test_definition_path("halo2_mcc/biped.json");
    if !std::path::Path::new(tag_path).exists() || !def.exists() {
        eprintln!("skipping: H2 biped/definition not present");
        return;
    }
    let docs = build_def_docs(&crate::core::bundled::locate_definitions_root(), GameId::Halo2, "biped");
    let bytes = std::fs::read(tag_path).unwrap();
    let tag = blam_tags::classic::read_classic_tag_file(
        &bytes,
        blam_tags::layout::TagLayout::from_json(&def).unwrap(),
    )
    .unwrap();

    fn find<'a>(s: TagStruct<'a>, name: &str) -> Option<TagStruct<'a>> {
        if s.definition().name() == name {
            return Some(s);
        }
        s.fields_all().find_map(|f| {
            f.as_struct()
                .and_then(|nested| find(nested, name))
                .or_else(|| {
                    f.as_block()
                        .and_then(|b| b.iter().find_map(|e| find(e, name)))
                })
        })
    }
    let explanations = |s: &TagStruct<'_>| -> Vec<String> {
        docs.entries_for_struct(s)
            .iter()
            .filter_map(|e| match e {
                DefEntry::Explanation { title, .. } => Some(title.clone()),
                DefEntry::Field { .. } => None,
            })
            .collect()
    };

    let object = find(tag.root(), "object_block_struct").expect("the object struct");
    assert_eq!(
        object.definition().guid(),
        [0; 16],
        "the premise: H2 structs have no GUID"
    );
    let titles = explanations(&object);
    for expected in [
        "Applying collision damage",
        "Game collision damage parameters",
        "Absolute collision damage parameters",
    ] {
        assert!(
            titles.iter().any(|t| t == expected),
            "object struct lost {expected:?}: {titles:?}"
        );
    }
    let function =
        find(tag.root(), "object_function_block_struct").expect("a functions element");
    assert!(
        explanations(&function).is_empty(),
        "the functions element borrowed another struct's explanations"
    );

    // The object's explanations still precede the fields they introduce.
    let order: Vec<&str> = docs
        .entries_for_struct(&object)
        .iter()
        .filter_map(|e| match e {
            DefEntry::Explanation { title, .. } => Some(title.as_str()),
            DefEntry::Field { clean_name, .. } => Some(clean_name.as_str()),
        })
        .collect();
    let at = |name: &str| order.iter().position(|n| *n == name).unwrap();
    assert!(at("Applying collision damage") < at("Apply collision damage scale"));
    assert!(at("Game collision damage parameters") < at("min game acc (default)"));
}

/// Reach's object family and shader types, read from its definitions:
/// an `object` reference takes every object type, a `unit` one only units.
#[test]
fn the_group_hierarchy_expands_a_parent_to_its_descendants() {
    let hierarchy =
        GroupHierarchy::load(&crate::core::bundled::locate_definitions_root(), GameId::HaloReach);
    let tag = |s: &str| blam_tags::parse_group_tag(s).unwrap();
    assert!(hierarchy.is_a(tag("bipd"), tag("unit")));
    assert!(hierarchy.is_a(tag("bipd"), tag("obje")), "two levels up");
    assert!(!hierarchy.is_a(tag("weap"), tag("unit")));

    let objects = hierarchy.expand(&[tag("obje")]);
    assert_eq!(
        objects[0],
        tag("obje"),
        "the allowed group itself comes first"
    );
    for group in [
        "bipd", "vehi", "weap", "eqip", "scen", "mach", "ctrl", "proj", "crea",
    ] {
        assert!(
            objects.contains(&tag(group)),
            "object does not take {group}"
        );
    }
    assert!(!objects.contains(&tag("bitm")));

    let units = hierarchy.expand(&[tag("unit")]);
    assert!(units.contains(&tag("bipd")) && units.contains(&tag("vehi")));
    assert!(!units.contains(&tag("weap")));

    let shaders = hierarchy.expand(&[tag("rm  ")]);
    assert!(shaders.contains(&tag("rmsh")) && shaders.contains(&tag("rmtr")));

    // A leaf group stands alone.
    assert_eq!(hierarchy.expand(&[tag("bitm")]), [tag("bitm")]);
}
