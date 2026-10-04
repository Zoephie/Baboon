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
mod tests {
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
}
