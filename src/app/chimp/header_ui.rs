//! Chimp header view: the package header's sections and its name, import, export and identity editors.
//! It owns drawing and collecting header edits; applying them belongs in `header_model`.

use super::*;

/// The package header as structured rows: identity, the name map, the import
/// map, and the export map, each with who references it.
///
/// Read-only for now. The Metadata view beside it stays as it is and remains the
/// exhaustive dump — this one is the part a person can act on, and the counts
/// are what make an edit's blast radius visible before there is anything to
/// edit.
pub(super) fn draw_chimp_header_view(
    ui: &mut Ui,
    document: &mut ChimpDocument,
    world: &World,
    expert_mode: bool,
    scan_referrers: &mut bool,
) -> bool {
    if document.header_usage.is_none() {
        refresh_chimp_header_usage(document);
    }
    // Collected during the draw and applied after it: the rename needs `&mut`
    // access to the very header and exports the rows are reading from.
    let mut edits = ChimpHeaderEdits::default();
    draw_chimp_header_sections(ui, document, world, expert_mode, &mut edits);
    // Passed out rather than started here: the scan needs the application, and
    // this call is holding a mutable borrow of one of its documents.
    *scan_referrers = edits.scan_referrers;

    if edits.start_identity {
        document.header_name_edit = None;
        document.header_import_edit = None;
        document.header_export_edit = None;
        let versioning = &document.header.versioning_info;
        document.header_identity_edit = Some(ChimpIdentityEdit {
            package_flags: format!("{:08X}", document.header.summary.package_flags),
            licensee_version: versioning.licensee_version,
            is_unversioned: document.header.is_unversioned,
            zen_version: versioning.zen_version,
            file_version_ue4: versioning.package_file_version.file_version_ue4,
            file_version_ue5: versioning.package_file_version.file_version_ue5,
        });
        document.header_error = None;
    }
    if edits.commit_identity
        && let Some(edit) = document.header_identity_edit.take()
    {
        match apply_chimp_identity_edit(document, &edit) {
            Ok(()) => {
                document.header_error = None;
                return true;
            }
            Err(error) => {
                document.header_error = Some(error);
                document.header_identity_edit = Some(edit);
            }
        }
    }
    if let Some(index) = edits.start_export {
        document.header_name_edit = None;
        document.header_import_edit = None;
        document.header_identity_edit = None;
        document.header_export_edit =
            document
                .header
                .export_map
                .get(index)
                .map(|entry| ChimpExportEdit {
                    index,
                    object_name: document
                        .header
                        .name_map
                        .try_get(entry.object_name)
                        .map(|name| name.to_string())
                        .unwrap_or_default(),
                    object_flags: entry.object_flags,
                    filter_flags: entry.filter_flags,
                    recompute_hash: false,
                });
        document.header_error = None;
    }
    if edits.commit_export
        && let Some(edit) = document.header_export_edit.take()
    {
        match apply_chimp_export_edit(world, document, &edit) {
            Ok(()) => {
                document.header_error = None;
                return true;
            }
            Err(error) => {
                document.header_error = Some(error);
                document.header_export_edit = Some(edit);
            }
        }
    }
    if let Some((index, text)) = edits.start_name {
        document.header_import_edit = None;
        document.header_name_edit = Some(ChimpNameEdit {
            index,
            text,
            focus: true,
        });
        document.header_error = None;
    }
    if let Some((slot, current)) = edits.start_import {
        document.header_name_edit = None;
        document.header_import_edit = Some(chimp_import_edit_for(slot, &current, world));
        document.header_error = None;
    }
    if edits.cancel {
        document.header_name_edit = None;
        document.header_import_edit = None;
        document.header_export_edit = None;
        document.header_identity_edit = None;
        document.header_error = None;
    }
    if let Some((index, text)) = edits.commit_name {
        match apply_chimp_name_rename(document, index, &text) {
            Ok(()) => {
                document.header_name_edit = None;
                document.header_error = None;
                return true;
            }
            Err(error) => document.header_error = Some(error),
        }
    }
    if let Some((index, slot)) = edits.commit_import {
        match apply_chimp_import_slot(document, index, slot) {
            Ok(()) => {
                document.header_import_edit = None;
                document.header_error = None;
                return true;
            }
            Err(error) => document.header_error = Some(error),
        }
    }
    false
}

/// What one draw of the Header view asked for, applied once its borrows are
/// gone.
#[derive(Default)]
struct ChimpHeaderEdits {
    start_name: Option<(usize, String)>,
    commit_name: Option<(usize, String)>,
    start_import: Option<(usize, ImportSlot)>,
    commit_import: Option<(usize, ImportSlot)>,
    start_export: Option<usize>,
    commit_export: bool,
    start_identity: bool,
    commit_identity: bool,
    scan_referrers: bool,
    cancel: bool,
}

fn draw_chimp_header_sections(
    ui: &mut Ui,
    document: &mut ChimpDocument,
    world: &World,
    expert_mode: bool,
    edits: &mut ChimpHeaderEdits,
) {
    let ChimpHeaderEdits {
        start_name: start,
        commit_name: commit,
        start_import,
        commit_import,
        start_export,
        commit_export,
        start_identity,
        commit_identity,
        scan_referrers,
        cancel,
    } = edits;
    let usage = document
        .header_usage
        .as_ref()
        .expect("refreshed by the caller");
    let header = &document.header;

    egui::ScrollArea::vertical()
        .id_salt(("chimp_header_view", document.package.clone()))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::CollapsingHeader::new("Identity")
                .default_open(true)
                .show(ui, |ui| {
                    if let Some(mut edit) = document.header_identity_edit.take() {
                        draw_chimp_identity_panel(
                            ui,
                            &mut edit,
                            expert_mode,
                            document.header_error.as_deref(),
                            commit_identity,
                            cancel,
                        );
                        ui.add_space(4.0);
                        document.header_identity_edit = Some(edit);
                    } else if ui.button("Edit flags and versioning...").clicked() {
                        *start_identity = true;
                    }
                    egui::Grid::new("chimp_header_identity")
                        .num_columns(2)
                        .spacing([18.0, 4.0])
                        .show(ui, |ui| {
                            header_row(ui, "Package", &document.package);
                            header_row(
                                ui,
                                "Package flags",
                                &format!("0x{:08X}", header.summary.package_flags),
                            );
                            header_row(ui, "Unversioned", &header.is_unversioned.to_string());
                            header_row(
                                ui,
                                "Zen version",
                                &format!("{:?}", header.versioning_info.zen_version),
                            );
                            header_row(
                                ui,
                                "Engine version",
                                &format!(
                                    "UE4 {} · UE5 {}",
                                    header.versioning_info.package_file_version.file_version_ue4,
                                    header.versioning_info.package_file_version.file_version_ue5
                                ),
                            );
                        });
                });

            let names = header.name_map.names();
            egui::CollapsingHeader::new(format!("Name map ({})", names.len()))
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Filter").color(subtle_dark()).small());
                        ui.add(
                            egui::TextEdit::singleline(&mut document.header_name_filter)
                                .desired_width(240.0)
                                .hint_text(placeholder_text("substring")),
                        );
                    });
                    // The editor sits above the list rather than inside it: the
                    // rows are virtualised, so an in-row editor would scroll out
                    // from under the user mid-edit.
                    // Taken out rather than borrowed in place: the panel reads
                    // the rest of the document (to price the edit) while it
                    // writes the draft, and those cannot be the same borrow.
                    if let Some(mut edit) = document.header_name_edit.take() {
                        let current = names.get(edit.index).cloned().unwrap_or_default();
                        let entry_usage = usage.names.get(edit.index).cloned().unwrap_or_default();
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.label(
                                RichText::new(format!("Editing entry {} · {current}", edit.index))
                                    .strong(),
                            );
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut edit.text)
                                    .id(egui::Id::new(("chimp_name_edit", edit.index)))
                                    .desired_width(320.0)
                                    .font(egui::TextStyle::Monospace),
                            );
                            if edit.focus {
                                response.request_focus();
                                edit.focus = false;
                            }
                            let submitted = response.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter));

                            // The blast radius, before the change rather than
                            // after it. Editing by index is the useful semantic
                            // precisely because it moves everything at once,
                            // which is also what makes it worth seeing first.
                            ui.label(
                                RichText::new(match entry_usage.count {
                                    0 => "Nothing references this entry.".to_owned(),
                                    1 => "1 reference will follow this rename:".to_owned(),
                                    n => format!("{n} references will follow this rename:"),
                                })
                                .color(subtle_dark())
                                .small(),
                            );
                            for site in &entry_usage.sites {
                                ui.label(
                                    RichText::new(format!("    • {site}"))
                                        .color(subtle_dark())
                                        .small(),
                                );
                            }
                            // Renaming the object an export is named after does
                            // not update the hash other packages address it by.
                            // Recoverable, but only if someone knows.
                            let desyncs =
                                chimp_export_hash_desyncs(header, edit.index, edit.text.trim());
                            if !desyncs.is_empty() {
                                ui.label(
                                    RichText::new(format!(
                                        "Export {} is named after this entry. Its public export \
                                         hash will no longer match the new name, so packages that \
                                         import it by hash keep resolving the old one.",
                                        desyncs
                                            .iter()
                                            .map(usize::to_string)
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    ))
                                    .color(Color32::from_rgb(170, 130, 60))
                                    .small(),
                                );
                            }

                            if let Some(error) = document.header_error.as_deref() {
                                ui.label(
                                    RichText::new(error)
                                        .color(Color32::from_rgb(150, 56, 44))
                                        .small(),
                                );
                            }
                            ui.horizontal(|ui| {
                                if ui.button("Apply").clicked() || submitted {
                                    *commit = Some((edit.index, edit.text.clone()));
                                }
                                if ui.button("Cancel").clicked() {
                                    *cancel = true;
                                }
                            });
                        });
                        ui.add_space(4.0);
                        document.header_name_edit = Some(edit);
                    }

                    let filter = document.header_name_filter.trim().to_ascii_lowercase();
                    let rows: Vec<usize> = names
                        .iter()
                        .enumerate()
                        .filter(|(_, name)| {
                            filter.is_empty() || contains_ignore_ascii_case(name, &filter)
                        })
                        .map(|(index, _)| index)
                        .collect();
                    if rows.is_empty() {
                        ui.label(RichText::new("No matching names").color(subtle_dark()));
                        return;
                    }
                    let row_height = ui.spacing().interact_size.y;
                    // Virtualised: a large package's name map runs to thousands
                    // of entries, and this view is drawn every frame.
                    egui::ScrollArea::vertical()
                        .id_salt("chimp_header_names")
                        .max_height(260.0)
                        .show_rows(ui, row_height, rows.len(), |ui, range| {
                            egui::Grid::new("chimp_header_name_rows")
                                .num_columns(3)
                                .spacing([14.0, 2.0])
                                .show(ui, |ui| {
                                    for &index in &rows[range] {
                                        let usage =
                                            usage.names.get(index).cloned().unwrap_or_default();
                                        ui.label(
                                            RichText::new(format!("{index}"))
                                                .color(subtle_dark())
                                                .monospace(),
                                        );
                                        let mut label = RichText::new(&names[index]).monospace();
                                        if usage.is_package_identity {
                                            label = label.color(foundation_blue());
                                        } else if usage.count == 0 {
                                            label = label.color(subtle_dark());
                                        }
                                        // The package's own name is not editable
                                        // here — see `apply_chimp_name_rename`.
                                        // Shown as a plain label so there is no
                                        // affordance to reach for.
                                        if usage.is_package_identity {
                                            ui.label(label)
                                                .on_hover_text(name_usage_tooltip(&usage));
                                        } else if ui
                                            .add(
                                                egui::Label::new(label).sense(egui::Sense::click()),
                                            )
                                            .on_hover_text(name_usage_tooltip(&usage))
                                            .clicked()
                                        {
                                            *start = Some((index, names[index].clone()));
                                        }
                                        ui.label(
                                            RichText::new(match usage.count {
                                                0 => "unreferenced".to_owned(),
                                                1 => "1 reference".to_owned(),
                                                n => format!("{n} references"),
                                            })
                                            .color(subtle_dark())
                                            .small(),
                                        );
                                        ui.end_row();
                                    }
                                });
                        });
                });

            let slots = read_import_slots(header);
            egui::CollapsingHeader::new(format!("Import map ({})", header.import_map.len()))
                .default_open(true)
                .show(ui, |ui| match &slots {
                    Ok(slots) => {
                        if let Some(mut edit) = document.header_import_edit.take() {
                            draw_chimp_import_editor(
                                ui,
                                &mut edit,
                                world,
                                document.header_error.as_deref(),
                                commit_import,
                                cancel,
                            );
                            ui.add_space(4.0);
                            document.header_import_edit = Some(edit);
                        }
                        // Virtualised like the name map: a level package imports
                        // thousands of slots, each resolved against the world.
                        let row_height = ui.spacing().interact_size.y;
                        egui::ScrollArea::vertical()
                            .id_salt("chimp_header_import_rows")
                            .max_height(260.0)
                            .show_rows(ui, row_height, slots.len(), |ui, range| {
                                egui::Grid::new("chimp_header_imports")
                                    .num_columns(4)
                                    .spacing([14.0, 2.0])
                                    .show(ui, |ui| {
                                        for (index, slot) in slots[range.clone()].iter().enumerate()
                                        {
                                            let index = range.start + index;
                                            ui.label(
                                                RichText::new(format!("{index}"))
                                                    .color(subtle_dark())
                                                    .monospace(),
                                            );
                                            let (kind, target) = import_slot_display(slot, world);
                                            ui.label(
                                                RichText::new(kind).color(subtle_dark()).small(),
                                            );
                                            if ui
                                                .add(
                                                    egui::Label::new(
                                                        RichText::new(target).monospace(),
                                                    )
                                                    .sense(egui::Sense::click()),
                                                )
                                                .on_hover_text("Retarget this import slot")
                                                .clicked()
                                            {
                                                *start_import = Some((index, slot.clone()));
                                            }
                                            let references = usage
                                                .import_references
                                                .get(index)
                                                .copied()
                                                .unwrap_or_default();
                                            ui.label(
                                                RichText::new(match references {
                                                    0 => "unreferenced".to_owned(),
                                                    1 => "1 property".to_owned(),
                                                    n => format!("{n} properties"),
                                                })
                                                .color(subtle_dark())
                                                .small(),
                                            );
                                            ui.end_row();
                                        }
                                    });
                            });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button("+ Add import slot").clicked() {
                                *start_import = Some((slots.len(), ImportSlot::Null));
                            }
                            ui.label(
                                RichText::new(
                                    "Slots cannot be removed: every object property names one by \
                                     position, so dropping one would shift every reference above \
                                     it.",
                                )
                                .color(subtle_dark())
                                .small(),
                            );
                        });
                    }
                    Err(error) => {
                        ui.label(
                            RichText::new(format!("Import map is not resolvable: {error:#}"))
                                .color(Color32::from_rgb(150, 56, 44)),
                        );
                    }
                });

            egui::CollapsingHeader::new(format!("Export map ({})", header.export_map.len()))
                .default_open(false)
                .show(ui, |ui| {
                    if let Some(mut edit) = document.header_export_edit.take() {
                        draw_chimp_export_edit_panel(
                            ui,
                            &mut edit,
                            header,
                            expert_mode,
                            document.header_error.as_deref(),
                            commit_export,
                            cancel,
                        );
                        ui.add_space(4.0);
                        document.header_export_edit = Some(edit);
                    }
                    let row_height = ui.spacing().interact_size.y;
                    egui::ScrollArea::vertical()
                        .id_salt("chimp_header_export_rows")
                        .max_height(260.0)
                        .show_rows(ui, row_height, header.export_map.len(), |ui, range| {
                            egui::Grid::new("chimp_header_exports")
                                .num_columns(4)
                                .spacing([14.0, 2.0])
                                .show(ui, |ui| {
                                    for (index, export) in
                                        header.export_map[range.clone()].iter().enumerate()
                                    {
                                        let index = range.start + index;
                                        ui.label(
                                            RichText::new(format!("{index}"))
                                                .color(subtle_dark())
                                                .monospace(),
                                        );
                                        if ui
                                            .add(
                                                egui::Label::new(
                                                    RichText::new(
                                                        header
                                                            .name_map
                                                            .try_get(export.object_name)
                                                            .map(|name| name.to_string())
                                                            .unwrap_or_else(|| {
                                                                "<bad name reference>".to_owned()
                                                            }),
                                                    )
                                                    .monospace(),
                                                )
                                                .sense(egui::Sense::click()),
                                            )
                                            .on_hover_text("Edit this export's name and flags")
                                            .clicked()
                                        {
                                            *start_export = Some(index);
                                        }
                                        ui.label(
                                            RichText::new(
                                                world
                                                    .class_key(header, export.class_index)
                                                    .unwrap_or_else(|| "Unknown class".to_owned()),
                                            )
                                            .color(subtle_dark())
                                            .small(),
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "flags 0x{:08X} · hash 0x{:016X}",
                                                export.object_flags, export.public_export_hash
                                            ))
                                            .color(subtle_dark())
                                            .small(),
                                        );
                                        ui.end_row();
                                    }
                                });
                        });
                });

            // The inverse of the import map, and the only question here that
            // cannot be answered from this package alone.
            egui::CollapsingHeader::new("Referenced by")
                .default_open(false)
                .show(ui, |ui| match &document.referrers {
                    ChimpReferrerState::Idle => {
                        ui.label(
                            RichText::new(
                                "The paks carry no reverse index, so this means reading every \
                                 mounted package header once.",
                            )
                            .color(subtle_dark())
                            .small(),
                        );
                        if ui.button("Find packages that import this").clicked() {
                            *scan_referrers = true;
                        }
                    }
                    ChimpReferrerState::Scanning => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new("Reading package headers...")
                                    .color(subtle_dark())
                                    .small(),
                            );
                        });
                    }
                    ChimpReferrerState::Done(scan) => {
                        ui.label(
                            RichText::new(match scan.referrers.len() {
                                0 => format!("No hard import, of {} packages read", scan.scanned),
                                1 => format!("1 package of {} imports this", scan.scanned),
                                n => format!("{n} packages of {} import this", scan.scanned),
                            })
                            .color(subtle_dark())
                            .small(),
                        );
                        if scan.referrers.is_empty() {
                            // Deliberately not phrased as "nothing references
                            // this". A soft object reference is FName indices
                            // inside an export's serial data, resolved by name
                            // at runtime; it never appears in the import table
                            // this scan reads, and the Zen summary has no
                            // soft-reference section to read instead. So a zero
                            // here rules out one kind of reference, not all of
                            // them, and it is not evidence that moving the
                            // package is safe.
                            ui.label(
                                RichText::new(
                                    "Soft references live in export data and are not counted.",
                                )
                                .color(subtle_dark())
                                .small(),
                            );
                        }
                        if scan.unreadable > 0 {
                            // "Nothing imports this" and "nothing I could read
                            // imports this" are different answers, and only one
                            // of them makes a rename safe.
                            ui.label(
                                RichText::new(format!(
                                    "{} package(s) could not be read and are not ruled out.",
                                    scan.unreadable
                                ))
                                .color(Color32::from_rgb(170, 130, 60))
                                .small(),
                            );
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("chimp_header_referrers")
                            .max_height(180.0)
                            .show(ui, |ui| {
                                for referrer in &scan.referrers {
                                    ui.label(RichText::new(referrer).monospace());
                                }
                            });
                        if ui.button("Scan again").clicked() {
                            *scan_referrers = true;
                        }
                    }
                });

            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Serial offsets, sizes and section offsets are recomputed when the package is \
                     written, so they are not shown here — the Metadata view carries them as they \
                     stand.",
                )
                .color(subtle_dark())
                .small(),
            );
        });
}

/// The draft panel for the package's flags and versioning.
fn draw_chimp_identity_panel(
    ui: &mut Ui,
    edit: &mut ChimpIdentityEdit,
    expert_mode: bool,
    error: Option<&str>,
    commit: &mut bool,
    cancel: &mut bool,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(RichText::new("Flags and versioning").strong());

        ui.label(RichText::new("Package flags").color(subtle_dark()).small());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut edit.package_flags)
                    .desired_width(120.0)
                    .font(egui::TextStyle::Monospace),
            );
            // The two values measured across every shipped Campaign Evolved tag
            // package, so the common cases need no hex at all.
            if ui
                .button("0x80002200")
                .on_hover_text("Every shipped tag group except the five cooked per level")
                .clicked()
            {
                edit.package_flags = "80002200".to_owned();
            }
            if ui
                .button("0x88002200")
                .on_hover_text("The _Generated_ level groups, which carry PKG_CookGenerated")
                .clicked()
            {
                edit.package_flags = "88002200".to_owned();
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Licensee version")
                    .color(subtle_dark())
                    .small(),
            );
            ui.add(egui::DragValue::new(&mut edit.licensee_version));
        });

        ui.add_space(4.0);
        if expert_mode {
            ui.label(
                RichText::new(
                    "These decide the header's shape, not just what it says. Lowering the UE5 \
                     file version stops the bulk-data map being written at all.",
                )
                .color(Color32::from_rgb(170, 130, 60))
                .small(),
            );
            ui.checkbox(&mut edit.is_unversioned, "Unversioned");
            if edit.is_unversioned {
                // Worth stating rather than leaving the fields looking live: an
                // unversioned package stores no versioning block, so the reader
                // synthesises these and nothing typed below is kept.
                ui.label(
                    RichText::new(
                        "An unversioned package carries no versioning block, so the fields below \
                         are not stored — the loader infers them. Uncheck this to make them real.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("Zen version").color(subtle_dark()).small());
                for option in [
                    EZenPackageVersion::Initial,
                    EZenPackageVersion::DataResourceTable,
                    EZenPackageVersion::ImportedPackageNames,
                    EZenPackageVersion::ExportDependencies,
                ] {
                    ui.selectable_value(&mut edit.zen_version, option, format!("{option:?}"));
                }
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("File version UE4")
                        .color(subtle_dark())
                        .small(),
                );
                ui.add(egui::DragValue::new(&mut edit.file_version_ue4));
                ui.label(RichText::new("UE5").color(subtle_dark()).small());
                ui.add(egui::DragValue::new(&mut edit.file_version_ue5));
            });
        } else {
            ui.label(
                RichText::new(
                    "Versioning fields are Expert mode only: they select the header's format, so \
                     a wrong one produces a package that writes cleanly and loads as something \
                     else.",
                )
                .color(subtle_dark())
                .small(),
            );
        }

        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Applying writes the package and reads it back first. If anything does not \
                 survive that, the change is refused rather than saved.",
            )
            .color(subtle_dark())
            .small(),
        );
        if let Some(error) = error {
            ui.label(
                RichText::new(error)
                    .color(Color32::from_rgb(150, 56, 44))
                    .small(),
            );
        }
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                *commit = true;
            }
            if ui.button("Cancel").clicked() {
                *cancel = true;
            }
        });
    });
}

/// The five `EObjectFlags` this crate names, as `(bit, label)`.
const CHIMP_OBJECT_FLAG_BITS: [(u32, &str); 5] = [
    (0x0000_0001, "Public"),
    (0x0000_0002, "Standalone"),
    (0x0000_0008, "Transactional"),
    (0x0000_0010, "ClassDefaultObject"),
    (0x0000_0020, "ArchetypeObject"),
];

/// The draft panel for one export-map entry.
fn draw_chimp_export_edit_panel(
    ui: &mut Ui,
    edit: &mut ChimpExportEdit,
    header: &FZenPackageHeader,
    expert_mode: bool,
    error: Option<&str>,
    commit: &mut bool,
    cancel: &mut bool,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(RichText::new(format!("Export {}", edit.index)).strong());

        ui.label(RichText::new("Object name").color(subtle_dark()).small());
        ui.add(
            egui::TextEdit::singleline(&mut edit.object_name)
                .desired_width(320.0)
                .font(egui::TextStyle::Monospace),
        );

        // The engine resolves an asset as `Package.Object`, so an export named
        // after the package leaf is the one that makes the package loadable
        // under its own path.
        let leaf = header
            .package_name()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        let was_leaf = header
            .export_map
            .get(edit.index)
            .and_then(|entry| header.name_map.try_get(entry.object_name))
            .is_some_and(|name| *name == leaf);
        if was_leaf && edit.object_name.trim() != leaf {
            ui.label(
                RichText::new(format!(
                    "This export is named after the package ({leaf}). The engine resolves the \
                     asset as {}.{leaf} — renaming it here leaves nothing at that path.",
                    header.package_name()
                ))
                .color(Color32::from_rgb(170, 130, 60))
                .small(),
            );
        }

        let stored_hash = header
            .export_map
            .get(edit.index)
            .map(|entry| entry.public_export_hash)
            .unwrap_or_default();
        let new_hash = public_export_hash(edit.object_name.trim());
        if stored_hash != new_hash {
            ui.label(
                RichText::new(format!(
                    "Public export hash 0x{stored_hash:016X} was computed from the old name. \
                     Other packages import this export by that hash and are not updated either \
                     way."
                ))
                .color(subtle_dark())
                .small(),
            );
            if expert_mode {
                ui.checkbox(
                    &mut edit.recompute_hash,
                    format!("Recompute it as 0x{new_hash:016X} (importers keep the old one)"),
                );
            }
        }

        ui.add_space(4.0);
        ui.label(RichText::new("Object flags").color(subtle_dark()).small());
        ui.horizontal_wrapped(|ui| {
            for (bit, label) in CHIMP_OBJECT_FLAG_BITS {
                let mut set = edit.object_flags & bit != 0;
                if ui.checkbox(&mut set, label).changed() {
                    if set {
                        edit.object_flags |= bit;
                    } else {
                        edit.object_flags &= !bit;
                    }
                }
            }
        });
        let named: u32 = CHIMP_OBJECT_FLAG_BITS.iter().map(|(bit, _)| bit).sum();
        let remainder = edit.object_flags & !named;
        ui.label(
            RichText::new(format!(
                "0x{:08X}{}",
                edit.object_flags,
                if remainder != 0 {
                    format!(" · 0x{remainder:08X} unnamed, preserved")
                } else {
                    String::new()
                }
            ))
            .color(subtle_dark())
            .small(),
        );
        ui.label(
            RichText::new(
                "Object flags decide how the payload is read, not just how it is labelled. \
                 Applying re-reads this export and refuses the change if it no longer decodes.",
            )
            .color(subtle_dark())
            .small(),
        );

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Filter flags").color(subtle_dark()).small());
            for option in [
                EExportFilterFlags::None,
                EExportFilterFlags::NotForClient,
                EExportFilterFlags::NotForServer,
            ] {
                ui.selectable_value(&mut edit.filter_flags, option, format!("{option:?}"));
            }
        });

        if let Some(error) = error {
            ui.label(
                RichText::new(error)
                    .color(Color32::from_rgb(150, 56, 44))
                    .small(),
            );
        }
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                *commit = true;
            }
            if ui.button("Cancel").clicked() {
                *cancel = true;
            }
        });
    });
}

/// The draft panel for one import slot.
fn draw_chimp_import_editor(
    ui: &mut Ui,
    edit: &mut ChimpImportEdit,
    world: &World,
    error: Option<&str>,
    commit: &mut Option<(usize, ImportSlot)>,
    cancel: &mut bool,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(RichText::new(format!("Import slot {}", edit.slot)).strong());
        ui.horizontal(|ui| {
            ui.selectable_value(&mut edit.kind, ChimpImportKind::Script, "Script");
            ui.selectable_value(&mut edit.kind, ChimpImportKind::Package, "Package");
            ui.selectable_value(&mut edit.kind, ChimpImportKind::Null, "Null");
        });

        match edit.kind {
            ChimpImportKind::Script => {
                ui.label(
                    RichText::new("Object path, e.g. /Script/Engine.StaticMesh")
                        .color(subtle_dark())
                        .small(),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut edit.script_path)
                        .desired_width(420.0)
                        .font(egui::TextStyle::Monospace),
                );
                ui.label(
                    RichText::new(
                        "Stored as a one-way hash of this path. If the module is not mounted, \
                         Baboon cannot name it back — it will read as unknown.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
            }
            ChimpImportKind::Package => {
                ui.label(
                    RichText::new("Package path, e.g. /Game/Art/Meshes/SM_Crate")
                        .color(subtle_dark())
                        .small(),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut edit.package_path)
                        .desired_width(420.0)
                        .font(egui::TextStyle::Monospace),
                );
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Object name").color(subtle_dark()).small());
                    ui.add(
                        egui::TextEdit::singleline(&mut edit.object_name)
                            .desired_width(260.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    if ui
                        .button("List exports")
                        .on_hover_text("Read the target package and list what it exports publicly")
                        .clicked()
                    {
                        edit.resolved = Some(chimp_public_exports_of(world, &edit.package_path));
                    }
                });
                ui.label(
                    RichText::new(format!(
                        "Stored as hash 0x{:016X}",
                        public_export_hash(edit.object_name.trim())
                    ))
                    .color(subtle_dark())
                    .small(),
                );
                match &edit.resolved {
                    Some(Ok(exports)) if exports.is_empty() => {
                        ui.label(
                            RichText::new("That package exports nothing publicly.")
                                .color(subtle_dark())
                                .small(),
                        );
                    }
                    Some(Ok(exports)) => {
                        let mut pick = None;
                        egui::ScrollArea::vertical()
                            .id_salt("chimp_import_exports")
                            .max_height(140.0)
                            .show(ui, |ui| {
                                for (name, hash) in exports {
                                    if ui
                                        .selectable_label(
                                            public_export_hash(edit.object_name.trim()) == *hash,
                                            RichText::new(name).monospace(),
                                        )
                                        .clicked()
                                    {
                                        pick = Some(name.clone());
                                    }
                                }
                            });
                        if let Some(name) = pick {
                            edit.object_name = name;
                        }
                    }
                    Some(Err(error)) => {
                        ui.label(
                            RichText::new(error)
                                .color(Color32::from_rgb(150, 56, 44))
                                .small(),
                        );
                    }
                    None => {}
                }
            }
            ChimpImportKind::Null => {
                ui.label(
                    RichText::new(
                        "The slot resolves to nothing. Properties naming it read as None.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
            }
        }

        ui.label(
            RichText::new(
                "Applying rebuilds the imported-package list, which is sorted by package id — the \
                 Metadata view will show it reordered. Dependency arcs are not rebuilt.",
            )
            .color(subtle_dark())
            .small(),
        );
        if let Some(error) = error {
            ui.label(
                RichText::new(error)
                    .color(Color32::from_rgb(150, 56, 44))
                    .small(),
            );
        }
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                let slot = match edit.kind {
                    ChimpImportKind::Script => Some(ImportSlot::Script(
                        FPackageObjectIndex::create_script_import(edit.script_path.trim()),
                    )),
                    ChimpImportKind::Package => Some(ImportSlot::Package(ImportTarget {
                        package: edit.package_path.trim().to_owned(),
                        object_hash: public_export_hash(edit.object_name.trim()),
                    })),
                    ChimpImportKind::Null => Some(ImportSlot::Null),
                };
                if let Some(slot) = slot {
                    *commit = Some((edit.slot, slot));
                }
            }
            if ui.button("Cancel").clicked() {
                *cancel = true;
            }
        });
    });
}

fn header_row(ui: &mut Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).color(subtle_dark()).small());
    ui.label(RichText::new(value).monospace());
    ui.end_row();
}

fn name_usage_tooltip(usage: &ChimpNameUsage) -> String {
    let mut lines = Vec::new();
    if usage.is_package_identity {
        lines.push(
            "This entry is the package's own name. The container addresses the package by a hash \
             of it, so changing it here would not move the chunk that serves it."
                .to_owned(),
        );
    }
    match usage.count {
        0 => lines.push("Nothing references this entry.".to_owned()),
        _ => {
            lines.push(format!("Referenced {} time(s) by:", usage.count));
            lines.extend(usage.sites.iter().map(|site| format!("  • {site}")));
            if usage.sites.len() < usage.count && usage.sites.len() == 8 {
                lines.push("  • …".to_owned());
            }
        }
    }
    lines.join("\n")
}

fn import_slot_display(slot: &ImportSlot, world: &World) -> (String, String) {
    match slot {
        ImportSlot::Script(index) => (
            "script".to_owned(),
            world
                .class_path(index.raw_index())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("unknown to this mount (0x{:016X})", index.raw_index())),
        ),
        ImportSlot::Package(target) => (
            "package".to_owned(),
            format!("{}#{:016X}", target.package, target.object_hash),
        ),
        ImportSlot::Null => ("null".to_owned(), "None".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name-map filter matches exactly what lowercasing each name did,
    /// without the per-row, per-frame copy.
    #[test]
    fn the_name_filter_matches_as_lowercasing_did() {
        let names = [
            "SM_Warthog_Chassis",
            "/Game/Vehicles/Warthog",
            "ÉCLAIR_Mesh",
            "éclair_mesh",
            "",
            "hog",
        ];
        for filter in ["", "hog", "warthog_c", "/game/", "éclair", "clair_m", "zzz"] {
            for name in names {
                assert_eq!(
                    contains_ignore_ascii_case(name, filter),
                    name.to_ascii_lowercase().contains(filter),
                    "{name:?} / {filter:?}"
                );
            }
        }
    }

    /// Object flags decide how a payload is *read*, so a wrong one is not a
    /// mislabel — it is a different interpretation of the same bytes. The named
    /// bits have to be the ones the engine uses.
    #[test]
    fn the_named_object_flag_bits_match_the_engine_values() {
        let named: Vec<(u32, &str)> = CHIMP_OBJECT_FLAG_BITS.to_vec();
        assert_eq!(named[0], (0x0000_0001, "Public"));
        assert_eq!(named[1], (0x0000_0002, "Standalone"));
        assert_eq!(named[2], (0x0000_0008, "Transactional"));
        // The one that actually changes decoding, via the native tail branch.
        assert_eq!(named[3], (0x0000_0010, "ClassDefaultObject"));
        assert_eq!(named[4], (0x0000_0020, "ArchetypeObject"));

        // Campaign Evolved's measured tag flags decompose into these, with
        // nothing unnamed left over: 0xb is Public + Standalone + Transactional.
        let all: u32 = CHIMP_OBJECT_FLAG_BITS.iter().map(|(bit, _)| bit).sum();
        assert_eq!(0xb_u32 & !all, 0, "0xb is fully named");
        assert_eq!(
            0x1_u32 & !all,
            0,
            "the generated-group value is fully named"
        );
    }
}
