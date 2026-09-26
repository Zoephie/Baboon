//! Shared fixtures for Chimp's unit tests.
//! It owns the synthetic headers and documents several test modules build on; the tests themselves sit beside their code.

use super::*;
use blam_tags::iostore::package::name_map::{EMappedNameType, FNameMap};
use blam_tags::iostore::package::zen::{
    EExportCommandType, FDependencyBundleHeader, FExportBundleEntry, FExportMapEntry,
};

/// A header with `names` interned and one export named after the first.
pub(super) fn header_with_names(names: &[&str]) -> FZenPackageHeader {
    let mut header = FZenPackageHeader {
        container_header_version: CE_HEADER_VERSION,
        is_unversioned: true,
        ..Default::default()
    };
    header.name_map = FNameMap::create(EMappedNameType::Package);
    for name in names {
        header.name_map.store(name);
    }
    header.summary.name = FMappedName::create(0, EMappedNameType::Package, 0);
    header.export_map = vec![FExportMapEntry {
        cooked_serial_offset: 0,
        cooked_serial_size: 0,
        object_name: FMappedName::create(0, EMappedNameType::Package, 0),
        outer_index: FPackageObjectIndex::default(),
        class_index: FPackageObjectIndex::default(),
        super_index: FPackageObjectIndex::default(),
        template_index: FPackageObjectIndex::default(),
        public_export_hash: 0,
        object_flags: 0,
        filter_flags: EExportFilterFlags::None,
        padding: [0; 3],
    }];
    // The reader enforces a Create and a Serialize command per export, plus
    // a dependency bundle header. A header without them is not a package —
    // which the reopen gate rightly refuses, so the fixture has to be one.
    for command_type in [EExportCommandType::Create, EExportCommandType::Serialize] {
        header.export_bundle_entries.push(FExportBundleEntry {
            local_export_index: 0,
            command_type,
        });
    }
    header
        .dependency_bundle_headers
        .push(FDependencyBundleHeader::default());
    header
}

/// A document with two names, an export named after the second, and one
/// reflected property holding `FName`s that point at it.
pub(super) fn rename_fixture() -> ChimpDocument {
    use blam_tags::iostore::object::value::{BlockLayout, PropertyEntry};

    let mut header = header_with_names(&["Warthog", "Material"]);
    // The export is named after entry 1, leaving entry 0 as the package's
    // own identity — the one the guard refuses.
    header.export_map[0].object_name = FMappedName::create(1, EMappedNameType::Package, 0);
    header.export_map[0].public_export_hash = public_export_hash("Material");

    let block = PropertyBlock {
        entries: vec![
            PropertyEntry {
                name: "Plain".into(),
                value: PropValue::Name(FName::new(1, 0, "Material")),
                slot: None,
            },
            PropertyEntry {
                name: "Numbered".into(),
                // Number 3 renders as `_2`, which is how the reader composes
                // it — the refresh has to reproduce that, not just the base.
                value: PropValue::Array(vec![PropValue::Name(FName::new(1, 3, "Material_2"))]),
                slot: None,
            },
            PropertyEntry {
                name: "Other".into(),
                value: PropValue::Name(FName::new(0, 0, "Warthog")),
                slot: None,
            },
        ],
        layout: BlockLayout::Unversioned {
            schema_len: 3,
            leading_empty: 0,
        },
    };

    ChimpDocument {
        package: "/Game/Test/Thing".to_owned(),
        provider: PackageProvider {
            container: 0,
            entry_path: "Content/Test/Thing.uasset".to_owned(),
            read_order: 0,
        },
        original: Vec::new(),
        header,
        payloads: vec![Vec::new()],
        exports: vec![ChimpExport {
            object: "Material".to_owned(),
            class: None,
            decoded: Ok(Export {
                block: ExportBlock::Reflected(block),
                trailer: blam_tags::iostore::object::export::Trailer::NoGuid,
                tail: Vec::new(),
            }),
        }],
        texture_previews: Vec::new(),
        mesh_kind: None,
        mesh_preview: None,
        mesh_preview_state: Default::default(),
        selected_export: 0,
        dirty: false,
        view: ChimpDocumentView::Header,
        document_text: String::new(),
        document_lines: ChimpJsonLines::default(),
        document_text_dirty: false,
        metadata_text: String::new(),
        metadata_lines: ChimpJsonLines::default(),
        metadata_text_dirty: false,
        header_usage: None,
        header_name_filter: String::new(),
        header_name_edit: None,
        header_import_edit: None,
        header_export_edit: None,
        header_identity_edit: None,
        header_error: None,
        referrers: ChimpReferrerState::Idle,
        orphaned: false,
        checkpoint_due: None,
        edits: 0,
    }
}
