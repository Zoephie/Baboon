use super::*;
use blam_tags::iostore::package::name_map::EMappedNameType;

/// `FNameMap::get` indexes its slice directly, so a reference past the end
/// of the table is a panic rather than an error — it would take the whole
/// application down on the next draw. This is the check that turns that into
/// a refused save.
#[test]
fn a_name_reference_past_the_end_of_the_table_is_refused_not_fatal() {
    let mut header = header_with_names(&["Warthog"]);
    assert!(validate_chimp_header_parts("pkg", &header, 1, &[]).is_ok());

    header.summary.name = FMappedName::create(7, EMappedNameType::Package, 0);
    let error = validate_chimp_header_parts("pkg", &header, 1, &[]).unwrap_err();
    assert!(error.contains("package name points outside"), "{error}");

    let mut header = header_with_names(&["Warthog"]);
    header.export_map[0].object_name = FMappedName::create(7, EMappedNameType::Package, 0);
    let error = validate_chimp_header_parts("pkg", &header, 1, &[]).unwrap_err();
    assert!(error.contains("export 0's object name"), "{error}");
}

/// `write_package` pairs payloads with export map entries positionally and
/// errors on a mismatch; catching it here names the package instead.
#[test]
fn a_payload_count_that_does_not_match_the_export_map_is_refused() {
    let header = header_with_names(&["Warthog"]);
    let error = validate_chimp_header_parts("pkg", &header, 0, &[]).unwrap_err();
    assert!(error.contains("0 export payloads for 1"), "{error}");
}

/// An object property addresses an import by position, so a reference past
/// the end of the import map would resolve to nothing at load.
#[test]
fn an_object_reference_past_the_end_of_the_import_map_is_refused() {
    let block = PropertyBlock {
        entries: vec![blam_tags::iostore::object::value::PropertyEntry {
            name: "Thing".into(),
            // `-1` is slot 0, `-4` is slot 3 — past an empty import map.
            value: PropValue::Object(-4),
            slot: None,
        }],
        layout: blam_tags::iostore::object::value::BlockLayout::Unversioned {
            schema_len: 1,
            leading_empty: 0,
        },
    };
    assert_eq!(first_unresolvable_object_reference(&block, 0), Some(3));
    assert_eq!(first_unresolvable_object_reference(&block, 4), None);
}

/// The package's own name is its identity, and the chunk that serves it is
/// addressed by a hash of that string — so renaming it here would leave the
/// package claiming to be something no chunk id matches.
#[test]
fn renaming_the_packages_own_name_is_refused() {
    let mut document = rename_fixture();
    let error = apply_chimp_name_rename(&mut document, 0, "Scorpion").unwrap_err();
    assert!(error.contains("package's own name"), "{error}");
    assert_eq!(document.header.name_map.names(), ["Warthog", "Material"]);
}

/// The silent-fork case. An `FName` is written as an index, so the rename
/// lands on disk either way — but the resolved text a decoded value carries
/// is what a later edit interns *by string*, so a stale one would fork a new
/// entry instead of following the rename.
#[test]
fn a_rename_refreshes_every_resolved_name_including_numbered_ones() {
    let mut document = rename_fixture();
    apply_chimp_name_rename(&mut document, 1, "Surface").unwrap();

    assert_eq!(document.header.name_map.names(), ["Warthog", "Surface"]);
    // The export's cached display name.
    assert_eq!(document.exports[0].object, "Surface");

    let mut seen = Vec::new();
    let Ok(decoded) = &mut document.exports[0].decoded else {
        panic!("fixture decodes");
    };
    let ExportBlock::Reflected(block) = &mut decoded.block else {
        panic!("fixture is reflected");
    };
    block.visit_names_mut(&mut |name| seen.push(name.to_string()));
    seen.sort();
    // `Surface_2` proves the number suffix was recomposed, not dropped, and
    // `Warthog` proves an unrelated entry was left alone.
    assert_eq!(seen, ["Surface", "Surface_2", "Warthog"]);
}

/// Renaming the entry an export is named after does not update the hash
/// other packages import it by, so the view has to say so first.
#[test]
fn an_export_hash_desync_is_reported_before_the_rename() {
    let document = rename_fixture();
    assert_eq!(
        chimp_export_hash_desyncs(&document.header, 1, "Surface"),
        vec![0]
    );
    // Renaming it back to what the hash was computed from is no desync, and
    // an entry no export is named after never is.
    assert!(chimp_export_hash_desyncs(&document.header, 1, "Material").is_empty());
    assert!(chimp_export_hash_desyncs(&document.header, 0, "Anything").is_empty());
}

/// Retargeting a slot must leave every other slot where it was: an object
/// property names an import by position, so a reordered list would silently
/// repoint properties nobody edited.
#[test]
fn retargeting_one_import_slot_leaves_the_others_in_place() {
    let mut document = rename_fixture();
    let slots = vec![
        ImportSlot::Script(FPackageObjectIndex::create_script_import(
            "/Script/Engine.Actor",
        )),
        ImportSlot::Package(ImportTarget {
            package: "/Game/One".to_owned(),
            object_hash: public_export_hash("One"),
        }),
        ImportSlot::Null,
    ];
    write_import_slots(&mut document.header, &slots).unwrap();

    apply_chimp_import_slot(
        &mut document,
        1,
        ImportSlot::Package(ImportTarget {
            package: "/Game/Two".to_owned(),
            object_hash: public_export_hash("Two"),
        }),
    )
    .unwrap();

    let after = read_import_slots(&document.header).unwrap();
    assert_eq!(after.len(), 3);
    assert_eq!(after[0], slots[0], "slot 0 moved");
    assert_eq!(after[2], ImportSlot::Null, "slot 2 moved");
    let ImportSlot::Package(target) = &after[1] else {
        panic!("slot 1 is a package import");
    };
    assert_eq!(target.package, "/Game/Two");
    assert_eq!(target.object_hash, public_export_hash("Two"));
}

/// Appending cannot renumber anything, which is why it is offered while
/// removal is not.
#[test]
fn an_appended_slot_lands_at_the_end_and_is_refused_past_it() {
    let mut document = rename_fixture();
    write_import_slots(&mut document.header, &[ImportSlot::Null]).unwrap();

    apply_chimp_import_slot(&mut document, 1, ImportSlot::Null).unwrap();
    assert_eq!(read_import_slots(&document.header).unwrap().len(), 2);

    let error = apply_chimp_import_slot(&mut document, 5, ImportSlot::Null).unwrap_err();
    assert!(error.contains("past the end"), "{error}");
    assert_eq!(read_import_slots(&document.header).unwrap().len(), 2);
}

/// The stored hash is case-folded, so the object-name box cannot be used to
/// tell two spellings apart — worth pinning, because the editor shows the
/// hash it computed and a user could reasonably expect casing to matter.
#[test]
fn an_import_object_hash_ignores_the_case_it_was_typed_in() {
    let hash = public_export_hash("Crate");
    assert_eq!(public_export_hash("crate"), hash);
    assert_eq!(public_export_hash("CRATE"), hash);
    assert_ne!(public_export_hash("Crates"), hash);
}

/// Interning rather than renaming is the difference between "this export is
/// now called X" and "everything called Y is now called X".
fn export_edit(document: &ChimpDocument, object_name: &str) -> ChimpExportEdit {
    ChimpExportEdit {
        index: 0,
        object_name: object_name.to_owned(),
        object_flags: document.header.export_map[0].object_flags,
        filter_flags: document.header.export_map[0].filter_flags,
        recompute_hash: false,
    }
}

/// Interning rather than renaming is the difference between "this export is
/// now called X" and "everything called Y is now called X" — the second is
/// what the name-map row does, and confusing the two would silently move
/// every other reference to that entry.
#[test]
fn renaming_an_export_interns_a_name_instead_of_rewriting_the_table() {
    let mut document = rename_fixture();
    let before = document.header.name_map.names().to_vec();

    let edit = export_edit(&document, "Surface");
    apply_chimp_export_metadata(&mut document, &edit);

    // `Material` is still entry 1, untouched, and `Surface` was appended.
    assert_eq!(
        &document.header.name_map.names()[..before.len()],
        &before[..]
    );
    assert_eq!(document.header.name_map.names().last().unwrap(), "Surface");
    assert_eq!(document.exports[0].object, "Surface");
}

/// Off by default, because it cuts both ways: recomputing makes this package
/// self-consistent and every importer holding the old hash wrong.
#[test]
fn an_export_hash_moves_only_when_the_recompute_is_asked_for() {
    let mut document = rename_fixture();
    let original = document.header.export_map[0].public_export_hash;
    assert_eq!(original, public_export_hash("Material"));

    let edit = export_edit(&document, "Surface");
    apply_chimp_export_metadata(&mut document, &edit);
    assert_eq!(
        document.header.export_map[0].public_export_hash, original,
        "the hash must not follow the name on its own"
    );

    let mut edit = export_edit(&document, "Surface");
    edit.recompute_hash = true;
    apply_chimp_export_metadata(&mut document, &edit);
    assert_eq!(
        document.header.export_map[0].public_export_hash,
        public_export_hash("Surface")
    );
}

/// A trailing `_N` belongs in the reference's number field, and `store` is
/// what puts it there — so the round trip has to give the name back whole.
#[test]
fn an_export_name_with_a_number_suffix_round_trips_through_the_reference() {
    let mut document = rename_fixture();
    let edit = export_edit(&document, "Surface_3");
    apply_chimp_export_metadata(&mut document, &edit);

    assert_eq!(document.exports[0].object, "Surface_3");
    // Stored as base + number, not as a literal entry.
    assert_eq!(document.header.name_map.names().last().unwrap(), "Surface");
    assert_eq!(document.header.export_map[0].object_name.number, 4);
}

fn identity_edit(document: &ChimpDocument) -> ChimpIdentityEdit {
    let versioning = &document.header.versioning_info;
    ChimpIdentityEdit {
        package_flags: format!("{:08X}", document.header.summary.package_flags),
        licensee_version: versioning.licensee_version,
        is_unversioned: document.header.is_unversioned,
        zen_version: versioning.zen_version,
        file_version_ue4: versioning.package_file_version.file_version_ue4,
        file_version_ue5: versioning.package_file_version.file_version_ue5,
    }
}

/// The gate has to pass the thing it is guarding, or it is just a refusal.
#[test]
fn a_flag_change_that_reopens_as_itself_is_applied() {
    let mut document = rename_fixture();
    let mut edit = identity_edit(&document);
    edit.package_flags = "80002200".to_owned();

    apply_chimp_identity_edit(&mut document, &edit).unwrap();
    assert_eq!(document.header.summary.package_flags, 0x8000_2200);

    // `0x` is accepted as well as bare hex, since the view shows it both ways.
    let mut edit = identity_edit(&document);
    edit.package_flags = "0x88002200".to_owned();
    apply_chimp_identity_edit(&mut document, &edit).unwrap();
    assert_eq!(document.header.summary.package_flags, 0x8800_2200);
}

/// Campaign Evolved's packages are unversioned, and an unversioned package
/// stores no versioning block — the loader infers one. So a version edit
/// there cannot persist, and the gate must not pretend it did.
#[test]
fn versioning_fields_are_not_stored_while_a_package_is_unversioned() {
    let document = rename_fixture();
    assert!(document.header.is_unversioned, "the fixture is CE-shaped");

    let mut candidate = document.header.clone();
    candidate.versioning_info.licensee_version = 7;
    candidate
        .versioning_info
        .package_file_version
        .file_version_ue5 = 1;

    // The gate passes, because those fields are simply not written...
    chimp_header_survives_reopen(&candidate, &document.payloads).unwrap();

    // ...and this is what actually comes back.
    let (bytes, _) = write_package(&candidate, &document.payloads, CE_HEADER_VERSION).unwrap();
    let reopened = FZenPackageHeader::deserialize(
        &mut Cursor::new(&bytes),
        None,
        CE_TOC_VERSION,
        CE_HEADER_VERSION,
        None,
    )
    .unwrap();
    assert_eq!(reopened.versioning_info.licensee_version, 0);
    assert_ne!(
        reopened
            .versioning_info
            .package_file_version
            .file_version_ue5,
        1
    );
}

/// A refused edit must leave the document exactly as it was — the candidate
/// is written and reopened before anything is assigned.
#[test]
fn an_unparseable_flag_value_changes_nothing() {
    let mut document = rename_fixture();
    let before = document.header.summary.package_flags;
    let mut edit = identity_edit(&document);
    edit.package_flags = "not hex".to_owned();

    let error = apply_chimp_identity_edit(&mut document, &edit).unwrap_err();
    assert!(error.contains("hex value"), "{error}");
    assert_eq!(document.header.summary.package_flags, before);
}

/// Turning versioning on makes the block real, and `has_versioning_info` is
/// derived on write — a draft that moved one without the other would reopen
/// as the opposite thing.
#[test]
fn turning_versioning_on_makes_the_fields_persist() {
    let mut document = rename_fixture();
    let mut edit = identity_edit(&document);
    edit.is_unversioned = false;
    edit.licensee_version = 7;
    edit.file_version_ue5 = 1013;

    apply_chimp_identity_edit(&mut document, &edit).unwrap();
    assert!(!document.header.is_unversioned);
    assert_eq!(document.header.summary.has_versioning_info, 1);
    assert_eq!(document.header.versioning_info.licensee_version, 7);

    // Now that the block is written, the gate is comparing real fields — so
    // the same edit round-trips instead of being silently dropped.
    chimp_header_survives_reopen(&document.header, &document.payloads).unwrap();

    let mut edit = identity_edit(&document);
    edit.is_unversioned = true;
    apply_chimp_identity_edit(&mut document, &edit).unwrap();
    assert!(document.header.is_unversioned);
    assert_eq!(document.header.summary.has_versioning_info, 0);
}

/// The gate's first job is catching a header that cannot be read back at
/// all — which is exactly what an export with no bundle commands is.
#[test]
fn the_reopen_gate_refuses_a_header_that_cannot_be_read_back() {
    let document = rename_fixture();
    let mut candidate = document.header.clone();
    candidate.export_bundle_entries.clear();

    let error = chimp_header_survives_reopen(&candidate, &document.payloads).unwrap_err();
    assert!(error.contains("did not reopen"), "{error}");
}

fn reopen(bytes: &[u8]) -> FZenPackageHeader {
    FZenPackageHeader::deserialize(
        &mut Cursor::new(bytes),
        None,
        CE_TOC_VERSION,
        CE_HEADER_VERSION,
        None,
    )
    .expect("the package reopens")
}

/// The fixture, put through one write and reopen so its summary holds real
/// section offsets.
///
/// A hand-built header starts with every offset and `header_size` at zero —
/// a state no file is ever in, because the writer fills them. Round-trip
/// properties are about well-formed headers, and reopening is how one is
/// obtained; comparing against the unwritten form would only measure the
/// fixture.
fn normalized_fixture() -> ChimpDocument {
    let mut document = rename_fixture();
    let (bytes, _) =
        write_package(&document.header, &document.payloads, CE_HEADER_VERSION).unwrap();
    document.header = reopen(&bytes);
    document
}

/// The control every other round-trip rests on.
///
/// Without it, "the edit read back" would be equally true of a writer that
/// quietly rearranged half the header — the edited field would survive and
/// everything around it would have moved. Writing, reopening and writing
/// again has to reach the same bytes, which is what separates rebuilt from
/// rearranged.
#[test]
fn writing_a_package_is_a_fixpoint_over_reopening_it() {
    let document = normalized_fixture();
    let (first, _) =
        write_package(&document.header, &document.payloads, CE_HEADER_VERSION).unwrap();
    let (second, _) =
        write_package(&reopen(&first), &document.payloads, CE_HEADER_VERSION).unwrap();
    assert_eq!(
        first, second,
        "an unedited package must rebuild identically"
    );
}

/// A rename has to survive the writer, not just the in-memory table.
#[test]
fn a_renamed_entry_survives_a_write_and_reopen() {
    let mut document = rename_fixture();
    let before = document.header.name_map.names().to_vec();
    apply_chimp_name_rename(&mut document, 1, "Surface").unwrap();

    let (bytes, _) =
        write_package(&document.header, &document.payloads, CE_HEADER_VERSION).unwrap();
    let reopened = reopen(&bytes);

    let mut expected = before;
    expected[1] = "Surface".to_owned();
    assert_eq!(reopened.name_map.names(), expected.as_slice());
    // And the export that referenced it now resolves to the new text.
    assert_eq!(
        reopened
            .name_map
            .try_get(reopened.export_map[0].object_name)
            .as_deref(),
        Some("Surface")
    );
}

/// Retargeting rebuilds four parallel arrays from the slot list, so the
/// proof it worked is that the slots come back as they were set — not that
/// the arrays look plausible.
#[test]
fn a_retargeted_import_survives_a_write_and_reopen() {
    let mut document = normalized_fixture();
    let slots = vec![
        ImportSlot::Script(FPackageObjectIndex::create_script_import(
            "/Script/Engine.Actor",
        )),
        ImportSlot::Package(ImportTarget {
            package: "/Game/One".to_owned(),
            object_hash: public_export_hash("One"),
        }),
    ];
    write_import_slots(&mut document.header, &slots).unwrap();
    apply_chimp_import_slot(
        &mut document,
        1,
        ImportSlot::Package(ImportTarget {
            package: "/Game/Two".to_owned(),
            object_hash: public_export_hash("Two"),
        }),
    )
    .unwrap();

    let (bytes, _) =
        write_package(&document.header, &document.payloads, CE_HEADER_VERSION).unwrap();
    let after = read_import_slots(&reopen(&bytes)).unwrap();

    assert_eq!(after[0], slots[0]);
    assert_eq!(
        after[1],
        ImportSlot::Package(ImportTarget {
            package: "/Game/Two".to_owned(),
            object_hash: public_export_hash("Two"),
        })
    );
}

/// Export metadata is written verbatim, so this is the check that it is not
/// being recomputed out from under the edit.
#[test]
fn edited_export_metadata_survives_a_write_and_reopen() {
    let mut document = rename_fixture();
    let mut edit = export_edit(&document, "Surface");
    edit.filter_flags = EExportFilterFlags::NotForServer;
    edit.recompute_hash = true;
    apply_chimp_export_metadata(&mut document, &edit);

    let (bytes, _) =
        write_package(&document.header, &document.payloads, CE_HEADER_VERSION).unwrap();
    let reopened = reopen(&bytes);
    let entry = &reopened.export_map[0];

    assert_eq!(
        reopened.name_map.try_get(entry.object_name).as_deref(),
        Some("Surface")
    );
    assert_eq!(entry.filter_flags, EExportFilterFlags::NotForServer);
    assert_eq!(entry.public_export_hash, public_export_hash("Surface"));
}
