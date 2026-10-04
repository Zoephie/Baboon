use super::*;

/// The exporters read the mesh kind straight off the exports now, not off
/// a document's preview. A skeletal mesh package also carries static
/// pieces, and is still skeletal.
#[test]
fn a_packages_mesh_kind_is_read_from_its_export_classes() {
    let export = |class: &str| ChimpExport {
        object: class.to_owned(),
        class: Some(class.to_owned()),
        decoded: Err(String::new()),
    };
    assert_eq!(chimp_mesh_kind(&[export("Texture2D")]), None);
    assert_eq!(
        chimp_mesh_kind(&[export("Material"), export("StaticMesh")]),
        Some(ChimpMeshKind::Static)
    );
    assert_eq!(
        chimp_mesh_kind(&[export("StaticMesh"), export("SkeletalMesh")]),
        Some(ChimpMeshKind::Skeletal)
    );
}

/// "Nothing imports this" and "nothing I could read imports this" are
/// different answers, and a rename is only safe under the first. A scan
/// that swallowed unreadable packages would report the safe one.
#[test]
fn a_referrer_scan_keeps_what_it_could_not_rule_out_separate_from_what_it_cleared() {
    let clean = ChimpReferrerScan {
        referrers: Vec::new(),
        scanned: 100,
        unreadable: 0,
    };
    let partial = ChimpReferrerScan {
        referrers: Vec::new(),
        scanned: 100,
        unreadable: 3,
    };
    // Both found nothing; only one of them looked everywhere.
    assert!(clean.referrers.is_empty() && partial.referrers.is_empty());
    assert_eq!(clean.unreadable, 0);
    assert_ne!(partial.unreadable, 0);
}

#[test]
fn a_mesh_import_is_a_material_by_the_prefix_the_game_uses() {
    assert!(is_chimp_material_package(
        "/Game/Art/Materials/MI_Brute_Body"
    ));
    assert!(is_chimp_material_package("/Game/Art/Materials/M_Master"));
    // Everything else a mesh imports - skeletons, physics, engine content,
    // and the textures themselves - is not a material.
    assert!(!is_chimp_material_package("/Game/Art/Textures/T_Brute_D"));
    assert!(!is_chimp_material_package("/Game/Art/SK_Brute"));
    assert!(!is_chimp_material_package("/Script/Engine"));
    // The prefix is on the leaf, not anywhere in the path.
    assert!(!is_chimp_material_package("/Game/M_Things/SK_Brute"));
}

#[test]
fn bundled_chimp_usmap_loads_and_invalid_custom_file_is_rejected() {
    assert!(load_chimp_usmap(None).is_ok());
    let path =
        std::env::temp_dir().join(format!("baboon-invalid-{}.usmap", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"not a usmap").unwrap();
    let error = load_chimp_usmap(Some(&path)).err().expect("invalid USMAP");
    assert!(error.contains("Could not parse USMAP"));
    std::fs::remove_file(path).unwrap();
}

#[test]
#[ignore = "requires a Campaign Evolved install plus CE_PAKS and CE_USMAP"]
fn real_custom_usmap_mounts_and_decodes_a_package() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let path = PathBuf::from(std::env::var_os("CE_USMAP").expect("set CE_USMAP"));
    let world = World::open(root, load_chimp_usmap(Some(&path)).unwrap()).unwrap();
    let package = world
        .packages()
        .iter()
        .find(|package| {
            package
                .name
                .to_ascii_lowercase()
                .contains("sm_spiritdropship_body")
        })
        .unwrap_or_else(|| panic!("SM_SpiritDropShip_Body was not found"));
    let document = load_chimp_document(&world, &package.name).unwrap();
    assert!(!document.exports.is_empty());
    assert_eq!(document.mesh_kind, Some(ChimpMeshKind::Static));
}

#[test]
fn raw_values_are_identified_without_embedding_binary_in_json() {
    assert_eq!(
        chimp_value_json(&PropValue::Raw(vec![1, 2, 3])),
        json!({"unknown_bytes": 3})
    );
}

fn reopen(install: &SyntheticInstall, document: &ChimpDocument) -> ChimpDocument {
    let (bytes, _) = rebuild_chimp_document(&install.world, document).unwrap();
    decode_chimp_document(&install.world, document.provider.clone(), bytes).unwrap()
}

/// An unedited package rebuilds to exactly the bytes it was read from.
#[test]
fn an_unedited_package_rebuilds_to_its_own_bytes() {
    let install = SyntheticInstall::new();
    for package in [THING, OTHER] {
        let document = install.document(package);
        let (bytes, _) = rebuild_chimp_document(&install.world, &document).unwrap();
        assert_eq!(bytes, document.original, "{package}");
    }
}

/// An edited package rebuilds into bytes that decode to the edited model:
/// every property, the grown name map and the header alike.
#[test]
fn an_edited_package_rebuilds_into_the_same_model() {
    let install = SyntheticInstall::new();
    let mut document = install.document(THING);
    set_first_value(&mut document, "Count", PropValue::Int(-12));
    set_first_value(
        &mut document,
        "Values",
        PropValue::Array(vec![PropValue::Int(1)]),
    );
    set_first_value(&mut document, "Later", PropValue::Int(5));
    let comet =
        blam_tags::iostore::object::edit::intern_name(&mut document.header.name_map, "Comet");
    set_first_value(&mut document, "Tag", PropValue::Name(comet));

    let reopened = reopen(&install, &document);
    assert!(first_block(&reopened).semantic_eq(first_block(&document)));
    assert_eq!(
        reopened.header.name_map.names(),
        document.header.name_map.names()
    );
    assert_eq!(
        read_import_slots(&reopened.header).unwrap(),
        read_import_slots(&document.header).unwrap()
    );
    assert!(matches!(first_value(&reopened, "Tag"), PropValue::Name(name) if name.as_str() == "Comet"));
}

/// The rebuild refuses what it cannot write back: an orphaned document, a
/// reference past the import map, and a value of the wrong type for its
/// slot. Each says which package and why.
#[test]
fn a_rebuild_refuses_orphans_bad_references_and_mistyped_values() {
    let install = SyntheticInstall::new();
    let mut document = install.document(THING);
    document.orphaned = true;
    let error = rebuild_chimp_document(&install.world, &document).unwrap_err();
    assert!(
        error.starts_with(
            "/Game/Test/Thing is no longer in the mounted containers, so it cannot be written back."
        ),
        "{error}"
    );

    let mut document = install.document(THING);
    set_first_value(&mut document, "Target", PropValue::Object(-9));
    assert_eq!(
        rebuild_chimp_document(&install.world, &document).unwrap_err(),
        "/Game/Test/Thing: export 0 references import slot 8, but the import map has 2 slots"
    );

    let mut document = install.document(THING);
    set_first_value(&mut document, "Count", PropValue::Bool(true));
    let error = rebuild_chimp_document(&install.world, &document).unwrap_err();
    assert!(
        error.starts_with("Could not validate /Game/Test/Thing export Thing: Count:"),
        "{error}"
    );
}

/// The two text views a document opens with: the decoded package and its
/// header metadata, including where it is served from.
#[test]
fn a_documents_text_views_describe_the_package_and_where_it_lives() {
    let install = SyntheticInstall::new();
    let document = install.document(THING);
    let text: Value = serde_json::from_str(&document.document_text).unwrap();
    assert_eq!(text["Package"], THING);
    assert_eq!(text["Source"], "Meteorite/Content/Test/Thing.uasset");
    assert_eq!(text["Summary"]["Exports"], 1);
    assert_eq!(text["Summary"]["Imports"], 2);
    // Imported packages are ordered by package id, not by slot.
    assert_eq!(text["Imports"], json!([SYNTHETIC_CLASS_PACKAGE, OTHER]));
    let properties = &text["Exports"][0]["Properties"];
    assert_eq!(text["Exports"][0]["Name"], "Thing");
    assert_eq!(properties["Count"], 7);
    assert_eq!(properties["Label"], "Warthog");
    assert_eq!(properties["Tag"], "Rocket");
    assert_eq!(properties["Values"], json!([10, 20, 30]));
    assert_eq!(properties["Lookup"], json!([{"key": 1, "value": 100}]));
    assert_eq!(properties["Target"], json!({"object_index": -1}));
    assert_eq!(properties["Inner"]["Depth"], 3);
    assert!(text["Exports"][0]["DecodeError"].is_null());

    let metadata: Value = serde_json::from_str(&document.metadata_text).unwrap();
    assert_eq!(metadata["Summary"]["Package"], THING);
    assert_eq!(metadata["Summary"]["IsUnversioned"], true);
    assert_eq!(metadata["NameMap"], json!([THING, "Thing", "Rocket"]));
    assert_eq!(metadata["ExportMap"][0]["ObjectName"], "Thing");
    assert_eq!(metadata["ExportMap"][0]["ObjectFlags"], "0x0000000B");
    assert_eq!(metadata["ExportMap"][0]["Class"], synthetic_class_key());
    let provider = &metadata["PhysicalProviders"][0];
    assert_eq!(provider["Active"], true);
    assert_eq!(provider["EntryPath"], "Meteorite/Content/Test/Thing.uasset");
    assert_eq!(
        provider["Container"],
        install
            .root
            .join("Paks")
            .join("pakchunk0-Windows.utoc")
            .display()
            .to_string()
    );
    assert_eq!(provider["RecoveredDirectoryIndex"], false);
}

#[test]
fn readable_documents_are_the_default_view() {
    assert_eq!(ChimpDocumentView::default(), ChimpDocumentView::Document);
    assert_eq!(
        chimp_class_display_name(Some("/Script/Engine.BlueprintGeneratedClass")),
        "BlueprintGeneratedClass"
    );
    assert_eq!(chimp_class_display_name(None), "Unknown");
}

#[test]
#[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
fn real_fast_type_index_matches_legacy_window() {
    let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
    let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
    let fast = index_chimp_package_types_with_prefixes(&world, &[64 * 1024, 1024 * 1024]);
    let legacy = index_chimp_package_types_with_prefixes(&world, &[1024 * 1024]);
    assert_eq!(fast.package_types, legacy.package_types);
    assert_eq!(fast.type_counts, legacy.type_counts);
    assert_eq!(fast.failures, legacy.failures);
}
