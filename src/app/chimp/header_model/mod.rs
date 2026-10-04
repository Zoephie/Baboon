//! Chimp package header editing, with no UI.
//! It owns validating headers and applying name, import, export and identity edits; the editor that collects them belongs in `header_ui`.

use super::*;

/// A draft of the package's flags and versioning.
///
/// The version fields are not metadata: `file_version_ue5` gates whether the
/// bulk-data map is serialized at all, and the Zen version decides the header's
/// own shape. That is why nothing here is applied without first writing the
/// package and reading it back.
pub(super) struct ChimpIdentityEdit {
    /// Free hex, so an unnamed bit can be set without inventing a checkbox for
    /// every flag the engine defines.
    pub(super) package_flags: String,
    pub(super) licensee_version: i32,
    pub(super) is_unversioned: bool,
    pub(super) zen_version: EZenPackageVersion,
    pub(super) file_version_ue4: i32,
    pub(super) file_version_ue5: i32,
}

/// A draft export-map entry.
pub(super) struct ChimpExportEdit {
    pub(super) index: usize,
    pub(super) object_name: String,
    pub(super) object_flags: u32,
    pub(super) filter_flags: EExportFilterFlags,
    /// Recompute `public_export_hash` from the new object name.
    ///
    /// Off by default and expert-only, because it cuts the other way from the
    /// desync it fixes: importers hold the *old* hash, so recomputing makes this
    /// package right and every importer wrong.
    pub(super) recompute_hash: bool,
}

pub(super) struct ChimpNameEdit {
    pub(super) index: usize,
    pub(super) text: String,
    pub(super) focus: bool,
}

/// A draft import slot.
///
/// Both halves are kept as text because neither is recoverable from what the
/// file stores: a script import is a one-way hash of its object path, and a
/// package import carries only `public_export_hash`. Whatever the user types is
/// the only name either has, and it is never written back as one.
pub(super) struct ChimpImportEdit {
    /// The slot being edited, or `slots.len()` for one being appended.
    pub(super) slot: usize,
    pub(super) kind: ChimpImportKind,
    /// `/Script/...` object path, for a script import.
    pub(super) script_path: String,
    /// `/Game/...` package path, for a package import.
    pub(super) package_path: String,
    /// Object name, hashed on commit. Never persisted: the file has no field for
    /// it, and claiming otherwise would be inventing provenance.
    pub(super) object_name: String,
    /// Public exports of `package_path`, once someone asked for them. Loading
    /// another package to answer "what is hash X called" is worth doing on
    /// request and not on every draw.
    pub(super) resolved: Option<Result<Vec<(String, u64)>, String>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ChimpImportKind {
    Script,
    Package,
    Null,
}

/// What the Header view reports about references into the package's tables.
///
/// The counts are the point of the view: a name-map entry is addressed by index,
/// so editing one retargets every reference at once, and seeing how many there
/// are — and whether one of them is the package's own identity — is what makes
/// that safe to reason about before anything is editable.
#[derive(Default)]
pub(super) struct ChimpHeaderUsage {
    /// Per name-map index, parallel to the table.
    pub(super) names: Vec<ChimpNameUsage>,
    /// Per import slot, how many object properties name it.
    pub(super) import_references: Vec<usize>,
}

#[derive(Default, Clone)]
pub(super) struct ChimpNameUsage {
    /// Total references, including the ones `sites` summarises.
    pub(super) count: usize,
    /// Where they are, deduplicated and capped — a tooltip, not a report.
    pub(super) sites: Vec<String>,
    /// This entry backs `summary.name`, so it is the package's identity: the
    /// chunk that serves the package is addressed by a hash of this string.
    pub(super) is_package_identity: bool,
}

impl ChimpNameUsage {
    fn record(&mut self, site: impl Into<String>) {
        self.count += 1;
        let site = site.into();
        if self.sites.len() < 8 && !self.sites.iter().any(|existing| *existing == site) {
            self.sites.push(site);
        }
    }
}

/// Structural checks over the package header itself, before it is written.
///
/// The property validator beside this one answers "is this value legal for its
/// schema slot". This answers the questions that only arise once the *header*
/// can be edited: whether every reference still lands inside the table it points
/// at, and whether the package still is what the container says it is.
pub(super) fn validate_chimp_header(document: &ChimpDocument) -> Result<(), String> {
    validate_chimp_header_parts(
        &document.package,
        &document.header,
        document.payloads.len(),
        &document.exports,
    )
}

/// [`validate_chimp_header`] over the pieces it actually reads, so the checks
/// can be exercised without a mounted package behind them.
fn validate_chimp_header_parts(
    package: &str,
    header: &FZenPackageHeader,
    payload_count: usize,
    exports: &[ChimpExport],
) -> Result<(), String> {
    let names = &header.name_map;

    if payload_count != header.export_map.len() {
        return Err(format!(
            "{package} has {payload_count} export payloads for {} export map entries",
            header.export_map.len()
        ));
    }

    // The most important check here. `FNameMap::get` indexes its slice directly,
    // so an out-of-range `FMappedName` is not a validation failure but a panic —
    // it would take the whole application down on the next draw rather than
    // report anything.
    if names.try_get(header.summary.name).is_none() {
        return Err(format!(
            "{package}: the package name points outside the name map"
        ));
    }
    for (index, export) in header.export_map.iter().enumerate() {
        if names.try_get(export.object_name).is_none() {
            return Err(format!(
                "{package}: export {index}'s object name points outside the name map"
            ));
        }
    }
    for (index, cell) in header.cell_export_map.iter().enumerate() {
        if names.try_get(cell.cpp_class_info).is_none() {
            return Err(format!(
                "{package}: cell export {index}'s class name points outside the name map"
            ));
        }
    }

    // The import map has to remain resolvable as slots, because that is the form
    // every object property addresses it in.
    let slots = read_import_slots(header)
        .map_err(|error| format!("{package}: import map is not resolvable: {error:#}"))?;
    for (index, export) in exports.iter().enumerate() {
        let Ok(decoded) = &export.decoded else {
            continue;
        };
        let ExportBlock::Reflected(block) = &decoded.block else {
            continue;
        };
        if let Some(bad) = first_unresolvable_object_reference(block, slots.len()) {
            return Err(format!(
                "{package}: export {index} references import slot {bad}, but the import map has {} slots",
                slots.len()
            ));
        }
    }

    Ok(())
}

/// The first negative `FPackageIndex` in `block` that names a slot past the end
/// of the import map, if any.
fn first_unresolvable_object_reference(block: &PropertyBlock, slots: usize) -> Option<usize> {
    fn check(value: &PropValue, slots: usize) -> Option<usize> {
        match value.unwrapped() {
            PropValue::Object(index) if *index < 0 => {
                let slot = import_slot_of(*index)?;
                (slot >= slots).then_some(slot)
            }
            PropValue::Array(items) | PropValue::Set(items) => {
                items.iter().find_map(|item| check(item, slots))
            }
            PropValue::Map(pairs) => pairs
                .iter()
                .find_map(|(key, value)| check(key, slots).or_else(|| check(value, slots))),
            PropValue::Struct(nested) => nested.iter().find_map(|(_, value)| check(value, slots)),
            _ => None,
        }
    }
    block.iter().find_map(|(_, value)| check(value, slots))
}

/// Rewrite one name-map entry, and everything that displays it.
///
/// An `FName` is stored as an index, so the rename retargets every reference on
/// disk by itself. What it does not do is update the *resolved* text a decoded
/// value carries — and that is not cosmetic: interning is by string, so editing
/// one of those fields afterwards would fork a fresh entry rather than follow the
/// rename. Refreshing them is part of the operation, not a redraw.
pub(super) fn apply_chimp_name_rename(
    document: &mut ChimpDocument,
    index: usize,
    text: &str,
) -> Result<(), String> {
    let text = text.trim();

    // The package's own name is its identity, and the container addresses the
    // chunk that serves it by a hash of that string. Renaming it here would
    // leave the package claiming to be something no chunk id matches — which the
    // in-place-rename work exists to do properly, moving the header, the chunk
    // id and the store entry together.
    if index == document.header.summary.name.index() as usize {
        return Err(
            "This entry is the package's own name. Renaming a package has to move its chunk id \
             and container-header entry with it, so it is a rename of the package rather than an \
             edit of this table."
                .to_owned(),
        );
    }

    document
        .header
        .name_map
        .rename(index, text)
        .map_err(|error| format!("{error:#}"))?;

    // Every export's cached display name, as `decode_chimp_exports` derived it.
    for (export, entry) in document
        .exports
        .iter_mut()
        .zip(document.header.export_map.iter())
    {
        if entry.object_name.index() as usize == index {
            export.object = document
                .header
                .name_map
                .try_get(entry.object_name)
                .map(|name| name.to_string())
                .unwrap_or_default();
        }
    }

    // Then the resolved text inside decoded property values. The composition
    // has to match the reader's: a non-zero number renders as `_{number - 1}`.
    let base = document
        .header
        .name_map
        .names()
        .get(index)
        .cloned()
        .unwrap_or_default();
    for export in &mut document.exports {
        let Ok(decoded) = &mut export.decoded else {
            continue;
        };
        let ExportBlock::Reflected(block) = &mut decoded.block else {
            continue;
        };
        block.visit_names_mut(&mut |name| {
            if name.index as usize != index {
                return;
            }
            let text = if name.number != 0 {
                format!("{base}_{}", name.number - 1)
            } else {
                base.clone()
            };
            *name = FName::new(name.index, name.number, text);
        });
    }

    Ok(())
}

/// Replace, or append, one import slot and rebuild the four arrays it feeds.
///
/// `write_import_slots` rederives `import_map`, `imported_packages`,
/// `imported_package_names` and `imported_public_export_hashes` from the slot
/// list, preserving slot order — which is what keeps every `FPackageIndex` in
/// every export payload pointing where it did. Appending is safe for the same
/// reason: it cannot renumber an existing slot.
pub(super) fn apply_chimp_import_slot(
    document: &mut ChimpDocument,
    index: usize,
    slot: ImportSlot,
) -> Result<(), String> {
    let mut slots = read_import_slots(&document.header)
        .map_err(|error| format!("Import map is not resolvable: {error:#}"))?;
    if index > slots.len() {
        return Err(format!(
            "Import slot {index} is past the end of a {}-slot import map",
            slots.len()
        ));
    }
    if index == slots.len() {
        slots.push(slot);
    } else {
        slots[index] = slot;
    }
    write_import_slots(&mut document.header, &slots)
        .map_err(|error| format!("Could not rewrite the import map: {error:#}"))
}

/// Write a candidate header and read it straight back, reporting anything that
/// did not survive the trip.
///
/// This is the parity policy's "reopen coverage" applied per edit rather than
/// only in tests, and it exists because the version fields are format selectors:
/// lowering `file_version_ue5` stops the bulk-data map being written at all, and
/// nothing about the edit itself would say so. One rebuild is cheap — the
/// recovery checkpoint already performs one on every commit.
fn chimp_header_survives_reopen(
    header: &FZenPackageHeader,
    payloads: &[Vec<u8>],
) -> Result<(), String> {
    let (bytes, _) = write_package(header, payloads, CE_HEADER_VERSION)
        .map_err(|error| format!("The edited header could not be written: {error:#}"))?;
    let reopened = FZenPackageHeader::deserialize(
        &mut Cursor::new(&bytes),
        None,
        CE_TOC_VERSION,
        CE_HEADER_VERSION,
        None,
    )
    .map_err(|error| format!("The edited header did not reopen: {error:#}"))?;

    let mut drift: Vec<String> = Vec::new();
    let mut check = |field: &str, intended: String, got: String| {
        if intended != got {
            drift.push(format!(
                "{field} was written as {intended} and read back as {got}"
            ));
        }
    };
    check(
        "package flags",
        format!("0x{:08X}", header.summary.package_flags),
        format!("0x{:08X}", reopened.summary.package_flags),
    );
    check(
        "unversioned",
        header.is_unversioned.to_string(),
        reopened.is_unversioned.to_string(),
    );
    // An unversioned package carries no versioning block at all — `serialize`
    // omits it and the reader synthesises one heuristically — so comparing those
    // fields would report drift on every package Campaign Evolved ships. They
    // are only real once the block is actually written.
    if !header.is_unversioned {
        check(
            "Zen version",
            format!("{:?}", header.versioning_info.zen_version),
            format!("{:?}", reopened.versioning_info.zen_version),
        );
        check(
            "UE4 file version",
            header
                .versioning_info
                .package_file_version
                .file_version_ue4
                .to_string(),
            reopened
                .versioning_info
                .package_file_version
                .file_version_ue4
                .to_string(),
        );
        check(
            "UE5 file version",
            header
                .versioning_info
                .package_file_version
                .file_version_ue5
                .to_string(),
            reopened
                .versioning_info
                .package_file_version
                .file_version_ue5
                .to_string(),
        );
        check(
            "licensee version",
            header.versioning_info.licensee_version.to_string(),
            reopened.versioning_info.licensee_version.to_string(),
        );
    }
    // Counts rather than contents: these are the tables a version change can
    // quietly stop writing.
    check(
        "bulk data entries",
        header.bulk_data.len().to_string(),
        reopened.bulk_data.len().to_string(),
    );
    check(
        "name map entries",
        header.name_map.len().to_string(),
        reopened.name_map.len().to_string(),
    );
    check(
        "import slots",
        header.import_map.len().to_string(),
        reopened.import_map.len().to_string(),
    );
    check(
        "export map entries",
        header.export_map.len().to_string(),
        reopened.export_map.len().to_string(),
    );

    if drift.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "The package does not read back as it was written, so the change was not applied:\n  {}",
            drift.join("\n  ")
        ))
    }
}

/// Apply a drafted identity/versioning change, but only if it survives a write
/// and reopen.
pub(super) fn apply_chimp_identity_edit(
    document: &mut ChimpDocument,
    edit: &ChimpIdentityEdit,
) -> Result<(), String> {
    let text = edit.package_flags.trim();
    let text = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    let package_flags = u32::from_str_radix(text, 16)
        .map_err(|_| format!("{:?} is not a 32-bit hex value", edit.package_flags))?;

    let mut candidate = document.header.clone();
    candidate.summary.package_flags = package_flags;
    candidate.is_unversioned = edit.is_unversioned;
    // Kept in step because `serialize` derives it rather than reading it, and a
    // header that disagreed with itself would reopen as the other one.
    candidate.summary.has_versioning_info = u32::from(!edit.is_unversioned);
    candidate.versioning_info.zen_version = edit.zen_version;
    candidate.versioning_info.licensee_version = edit.licensee_version;
    candidate
        .versioning_info
        .package_file_version
        .file_version_ue4 = edit.file_version_ue4;
    candidate
        .versioning_info
        .package_file_version
        .file_version_ue5 = edit.file_version_ue5;

    chimp_header_survives_reopen(&candidate, &document.payloads)?;
    document.header = candidate;
    Ok(())
}

/// Apply a drafted export-map entry.
///
/// `object_flags` is the reason this is not three independent setters. It is not
/// inert metadata: it is fed to `read_export_in`, and `RF_CLASS_DEFAULT_OBJECT`
/// selects a different native tail branch — so changing it changes what the same
/// bytes *mean*. The export is therefore round-tripped through the new flags and
/// the edit is refused if it no longer decodes, rather than left to fail at save
/// time with the old decode still in hand.
pub(super) fn apply_chimp_export_edit(
    world: &World,
    document: &mut ChimpDocument,
    edit: &ChimpExportEdit,
) -> Result<(), String> {
    let index = edit.index;
    if index >= document.header.export_map.len() {
        return Err(format!("Export {index} is no longer in this package"));
    }
    if edit.object_name.trim().is_empty() {
        return Err("An export needs an object name".to_owned());
    }

    // Re-decode first, so a refusal leaves the document untouched.
    if edit.object_flags != document.header.export_map[index].object_flags
        && let Some(redecoded) = chimp_redecode_export(world, document, index, edit.object_flags)?
    {
        document.exports[index].decoded = Ok(redecoded);
    }
    apply_chimp_export_metadata(document, edit);
    Ok(())
}

/// The half of an export edit that needs nothing but the document: the name,
/// the filter flags, and the optional hash recompute.
///
/// Split out because `object_flags` is the only field whose change has to be
/// proven against a decoder first, and everything else would otherwise be
/// untestable without a mounted world behind it.
fn apply_chimp_export_metadata(document: &mut ChimpDocument, edit: &ChimpExportEdit) {
    let index = edit.index;
    let object_name = edit.object_name.trim();
    let entry = &mut document.header.export_map[index];
    entry.object_flags = edit.object_flags;
    entry.filter_flags = edit.filter_flags;
    // `store` interns rather than renames: an existing entry is reused, a new
    // one appended, and the trailing `_N` lands in the reference's number field
    // where it belongs.
    entry.object_name = document.header.name_map.store(object_name);
    if edit.recompute_hash {
        let resolved = document
            .header
            .name_map
            .try_get(entry.object_name)
            .map(|name| name.to_string())
            .unwrap_or_else(|| object_name.to_owned());
        document.header.export_map[index].public_export_hash = public_export_hash(&resolved);
    }

    document.exports[index].object = document
        .header
        .name_map
        .try_get(document.header.export_map[index].object_name)
        .map(|name| name.to_string())
        .unwrap_or_default();
}

/// Round-trip one export through `object_flags` to see whether it still decodes.
///
/// Serializing the *current* decoded value first is what keeps unsaved property
/// edits: re-reading `payloads[index]` would decode the bytes as they were on
/// disk and silently discard them. `Ok(None)` means there was nothing decoded to
/// re-interpret, so the flag is the only thing changing.
fn chimp_redecode_export(
    world: &World,
    document: &ChimpDocument,
    index: usize,
    object_flags: u32,
) -> Result<Option<Export>, String> {
    let export = &document.exports[index];
    let (Some(class), Ok(decoded)) = (export.class.as_deref(), &export.decoded) else {
        return Ok(None);
    };
    let names = document.header.name_map.copy_raw_names();
    let resolver = world.resolver(&document.header, &document.original, &names);
    let payload = write_export_in(class, decoded, world.usmap(), Some(&resolver))
        .map_err(|error| format!("Could not re-serialize export {index}: {error:#}"))?;
    let bulk: Vec<(i64, i64)> = document
        .header
        .bulk_data
        .iter()
        .map(|entry| (entry.serial_offset, entry.serial_size))
        .collect();
    let context = ExportContext {
        bulk_data: &bulk,
        resolver: Some(&resolver),
    };
    read_export_in(
        &payload,
        &names,
        world.usmap(),
        class,
        object_flags,
        &context,
    )
    .map(Some)
    .map_err(|error| {
        format!(
            "Export {index} no longer decodes with object flags 0x{object_flags:08X}: {error:#}"
        )
    })
}

/// The public exports of a mounted package, as `(object name, hash)`.
///
/// `public_export_hash` is one-way, so a hash cannot be turned back into the
/// name it came from. This reads the package that actually holds the export and
/// matches on the hash — the difference between a usable picker and a raw 64-bit
/// field.
pub(super) fn chimp_public_exports_of(
    world: &World,
    package: &str,
) -> Result<Vec<(String, u64)>, String> {
    let record = world
        .package(package)
        .ok_or_else(|| format!("{package} is not mounted"))?;
    let provider = record
        .active_provider()
        .cloned()
        .ok_or_else(|| format!("{package} has no active provider"))?;
    let bytes = world
        .read_provider(&provider)
        .map_err(|error| error.to_string())?;
    let header = FZenPackageHeader::deserialize(
        &mut Cursor::new(&bytes),
        None,
        CE_TOC_VERSION,
        CE_HEADER_VERSION,
        None,
    )
    .map_err(|error| format!("{package} did not parse: {error:#}"))?;
    let mut exports: Vec<(String, u64)> = header
        .export_map
        .iter()
        .filter(|export| export.public_export_hash != 0)
        .filter_map(|export| {
            header
                .name_map
                .try_get(export.object_name)
                .map(|name| (name.to_string(), export.public_export_hash))
        })
        .collect();
    exports.sort();
    exports.dedup();
    Ok(exports)
}

/// Export object names this entry backs whose `public_export_hash` would no
/// longer match, so the view can say so before the rename happens.
///
/// The hash is how *other* packages address an export. Renaming the object it
/// names does not update theirs, so the two drift apart — recoverable, but only
/// if someone knows it happened.
pub(super) fn chimp_export_hash_desyncs(
    header: &FZenPackageHeader,
    index: usize,
    text: &str,
) -> Vec<usize> {
    header
        .export_map
        .iter()
        .enumerate()
        .filter(|(_, export)| export.object_name.index() as usize == index)
        .filter(|(_, export)| {
            let resolved = if export.object_name.number != 0 {
                format!("{text}_{}", export.object_name.number - 1)
            } else {
                text.to_owned()
            };
            export.public_export_hash != public_export_hash(&resolved)
        })
        .map(|(index, _)| index)
        .collect()
}

/// Recompute who references each name-map entry and each import slot.
pub(super) fn refresh_chimp_header_usage(document: &mut ChimpDocument) {
    let mut names = vec![ChimpNameUsage::default(); document.header.name_map.len()];
    let mut record = |mapped: FMappedName, site: &str| {
        if let Some(usage) = names.get_mut(mapped.index() as usize) {
            usage.record(site);
        }
    };

    record(document.header.summary.name, "package name");
    for (index, export) in document.header.export_map.iter().enumerate() {
        record(export.object_name, &format!("export {index} object name"));
    }
    for (index, cell) in document.header.cell_export_map.iter().enumerate() {
        record(cell.cpp_class_info, &format!("cell export {index} class"));
    }
    if let Some(usage) = names.get_mut(document.header.summary.name.index() as usize) {
        usage.is_package_identity = true;
    }

    // Property values. `visit_names_mut` is the crate's only traversal over
    // every shape a name can hide behind, and nothing is mutated here — taking
    // it by `&mut` costs nothing and avoids a second copy of a match whose whole
    // value is that it is exhaustive.
    for index in 0..document.exports.len() {
        let Ok(decoded) = &mut document.exports[index].decoded else {
            continue;
        };
        let ExportBlock::Reflected(block) = &mut decoded.block else {
            continue;
        };
        let site = format!("export {index} properties");
        block.visit_names_mut(&mut |name| {
            if let Some(usage) = names.get_mut(name.index as usize) {
                usage.record(&site);
            }
        });
    }

    // Import slots, by the `FPackageIndex` an object property would carry.
    let slot_count = document.header.import_map.len();
    let mut import_references = vec![0usize; slot_count];
    for slot in 0..slot_count {
        let package_index = import_package_index(slot);
        for export in &document.exports {
            let Ok(decoded) = &export.decoded else {
                continue;
            };
            let ExportBlock::Reflected(block) = &decoded.block else {
                continue;
            };
            import_references[slot] += count_object_references(block, package_index);
        }
    }

    document.header_usage = Some(ChimpHeaderUsage {
        names,
        import_references,
    });
}

pub(super) fn validate_chimp_property_block(
    class: &str,
    block: &PropertyBlock,
    usmap: &Usmap,
) -> Result<(), String> {
    for entry in &block.entries {
        let Some(slot) = entry.slot else {
            continue;
        };
        let ty = property_type_for_slot(class, slot, usmap).map_err(|error| error.to_string())?;
        validate_value_for_type(&ty, &entry.value)
            .map_err(|error| format!("{}: {error}", entry.name))?;
        if let (PropertyType::Struct(nested), PropValue::Struct(value)) = (&ty, &entry.value) {
            validate_chimp_property_block(nested, value, usmap)?;
        }
    }
    Ok(())
}

/// Seed a draft from the slot as it stands, so opening the editor shows what is
/// there rather than an empty form.
pub(super) fn chimp_import_edit_for(
    slot: usize,
    current: &ImportSlot,
    world: &World,
) -> ChimpImportEdit {
    let mut edit = ChimpImportEdit {
        slot,
        kind: ChimpImportKind::Null,
        script_path: String::new(),
        package_path: String::new(),
        object_name: String::new(),
        resolved: None,
    };
    match current {
        ImportSlot::Script(index) => {
            edit.kind = ChimpImportKind::Script;
            // Only the mount can name a script hash; an unknown one is left
            // blank rather than filled with something invented.
            edit.script_path = world
                .class_path(index.raw_index())
                .unwrap_or_default()
                .to_owned();
        }
        ImportSlot::Package(target) => {
            edit.kind = ChimpImportKind::Package;
            edit.package_path = target.package.clone();
            // The object name is not in the file — only its hash is. Recover it
            // from the package that holds the export, and leave it blank when
            // that is not possible rather than guessing.
            if let Ok(exports) = chimp_public_exports_of(world, &target.package) {
                edit.object_name = exports
                    .iter()
                    .find(|(_, hash)| *hash == target.object_hash)
                    .map(|(name, _)| name.clone())
                    .unwrap_or_default();
                edit.resolved = Some(Ok(exports));
            }
        }
        ImportSlot::Null => {}
    }
    edit
}

#[cfg(test)]
mod tests;
