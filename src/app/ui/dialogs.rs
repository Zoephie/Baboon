//! Modal dialogs, one module per dialog, and the conversion reports several of them share.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

mod cache_import;
mod chimp_prompts;
mod clear_stash_confirm;
mod container_dump_confirm;
mod container_duplicate_confirm;
mod container_folder;
mod delete_confirm;
mod exported_mod;
mod import_tag;
mod keyword_chooser;
mod mod_export;
mod new_tag;
mod operation_notice;
mod overwrite_confirm;
mod rename_tag;
mod tag_import;
mod tsv_paste;

pub(in crate::app) use mod_export::DiffNode;

/// What a folder import actually wrote, grouped by how much can be claimed for
/// it.
///
/// The three buckets are not severity levels. Built-from-definitions is the
/// ordinary path and the cleaner one: the tag holds what the source gave it and
/// what the schema defaults to, nothing else. Started-from-a-kit-tag is the
/// exception, taken only for the handful of groups whose schema cannot express
/// them, and it is worth naming because that tag's layout revision came with it.
fn draw_folder_import_report(ui: &mut Ui, report: &FolderConversionReport) {
    ui.label(
        RichText::new(format!(
            "Imported {} tag(s): {} from the definitions, {} from a kit tag. {} failed, {} ignored.",
            report.converted_count(),
            report.generated_count(),
            report.native_count(),
            report.failed_count(),
            report.ignored_files.len()
        ))
        .strong(),
    );
    ui.label(
        RichText::new(format!(
            "{} ({}) -> {}",
            report.source_root.display(),
            report.source_game,
            report.destination_root.display()
        ))
        .monospace()
        .small()
        .color(subtle_dark()),
    );
    ui.label(
        RichText::new(format!("Target profile: {}", report.target_game))
            .small()
            .color(subtle_dark()),
    );
    egui::ScrollArea::vertical()
        .id_salt("tag_import_results")
        .max_height(320.0)
        .show(ui, |ui| {
            for wanted in [
                FolderConversionFileStatus::NativeLayout,
                FolderConversionFileStatus::GeneratedLayout,
                FolderConversionFileStatus::Kept,
                FolderConversionFileStatus::Failed,
            ] {
                let (label, color) = match wanted {
                    FolderConversionFileStatus::NativeLayout => (
                        "Started from a kit tag — the definitions cannot build this group",
                        Color32::from_rgb(242, 196, 48),
                    ),
                    FolderConversionFileStatus::GeneratedLayout => {
                        ("Built from the target's own definitions", text_dark())
                    }
                    FolderConversionFileStatus::Kept => {
                        ("Already in the kit — left as it was", subtle_dark())
                    }
                    FolderConversionFileStatus::Failed => {
                        ("Failed / skipped", material_delete_text())
                    }
                };
                let matching = report
                    .files
                    .iter()
                    .filter(|file| file.status == wanted)
                    .collect::<Vec<_>>();
                if matching.is_empty() {
                    continue;
                }
                ui.collapsing(
                    RichText::new(format!("{label} ({})", matching.len())).color(color),
                    |ui| {
                        for file in matching {
                            let replaced = if file.overwritten { " [replaced]" } else { "" };
                            let output = file
                                .output
                                .as_ref()
                                .map(|path| format!(" -> {}", path.display()))
                                .unwrap_or_default();
                            ui.label(
                                RichText::new(format!(
                                    "{}{}{} — {}",
                                    file.source, output, replaced, file.detail
                                ))
                                .small()
                                .color(color),
                            );
                        }
                    },
                );
            }
            if !report.ignored_files.is_empty() {
                ui.collapsing(
                    format!("Ignored non-tag files ({})", report.ignored_files.len()),
                    |ui| {
                        for path in &report.ignored_files {
                            ui.label(RichText::new(path).monospace().small());
                        }
                    },
                );
            }
        });
}

/// The conversion summary and issue list.
///
/// Shared by the single-tag import preview and the Campaign Evolved import
/// dialog: both answer the same question — what will this conversion cost — and
/// two copies would drift.
fn draw_conversion_report(ui: &mut Ui, report: &TagConversionReport, salt: &str) {
    egui::Grid::new(format!("{salt}_summary"))
        .num_columns(2)
        .spacing([20.0, 3.0])
        .show(ui, |ui| {
            for (label, value) in [
                ("Copied exactly", report.copied_exact),
                ("Converted semantically", report.converted_semantic),
                (
                    "Mapped through schema/catalog aliases",
                    report.mapped_aliases,
                ),
                ("Target fields left at defaults", report.defaulted_target),
                ("Unsupported source values", report.unsupported_source),
                ("Truncated elements", report.truncated),
            ] {
                ui.label(label);
                ui.label(value.to_string());
                ui.end_row();
            }
            // Only worth a row when there are any. A resource can be most of
            // what a tag is -- an animation graph's whole payload is one -- so
            // when one crossed, say so.
            if report.transferred_resources > 0 {
                ui.label("Pageable resources carried across");
                ui.label(report.transferred_resources.to_string());
                ui.end_row();
            }
            // The one row that is a to-do list rather than a statistic.
            if report.dropped_references > 0 {
                ui.label("References to reconnect by hand");
                ui.label(report.dropped_references.to_string());
                ui.end_row();
            }
        });

    if report.issues.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new("Conversion details")
            .color(subtle_dark())
            .small(),
    );
    egui::ScrollArea::vertical()
        .id_salt(format!("{salt}_issues"))
        .max_height(230.0)
        .show(ui, |ui| {
            for issue in &report.issues {
                let kind = match issue.kind {
                    ConversionIssueKind::Unsupported => "Unsupported",
                    ConversionIssueKind::Truncated => "Truncated",
                    ConversionIssueKind::Warning => "Warning",
                };
                ui.label(
                    RichText::new(format!("{kind}: {} — {}", issue.path, issue.message))
                        .color(subtle_dark())
                        .small(),
                );
            }
        });
}
