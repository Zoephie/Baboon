//! Modal dialogs for rename, paste, keyword selection, and new-tag workflows.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

impl Baboon {
    pub(super) fn draw_tag_conversion_window(&mut self, ctx: &egui::Context) {
        if !self.expert_mode {
            self.tag_conversion_dialog = None;
            return;
        }
        if self.tag_conversion_dialog.is_none() {
            return;
        }

        let mut open = true;
        let mut analyze = false;
        let mut choose_and_save = false;
        let mut confirm_inside_source = false;
        let mut cancel_inside_source = false;
        {
            let dialog = self.tag_conversion_dialog.as_mut().expect("checked above");
            egui::Window::new("Save Tag for Another Game")
                .id(egui::Id::new("tag_conversion"))
                .open(&mut open)
                .default_width(680.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new("Source tag").color(subtle_dark()).small());
                    ui.label(
                        RichText::new(&dialog.source_label)
                            .color(text_dark())
                            .monospace(),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Source profile").color(subtle_dark()));
                        ui.label(RichText::new(&dialog.source_game).color(text_dark()));
                    });

                    let previous_target = dialog.target_game.clone();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Target profile").color(subtle_dark()));
                        egui::ComboBox::from_id_salt("tag_conversion_target")
                            .selected_text(&dialog.target_game)
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for game in CONVERSION_GAMES {
                                    if *game != dialog.source_game {
                                        ui.selectable_value(
                                            &mut dialog.target_game,
                                            (*game).to_owned(),
                                            *game,
                                        );
                                    }
                                }
                            });
                    });
                    if dialog.target_game != previous_target {
                        dialog.draft = None;
                        dialog.error = None;
                        dialog.pending_source_destination = None;
                    }

                    ui.add_space(8.0);
                    if ui.button("Analyze Conversion").clicked() {
                        analyze = true;
                    }

                    if let Some(draft) = dialog.draft.as_ref() {
                        let report = &draft.report;
                        ui.add_space(8.0);
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "Target: {} (.{}), group {}",
                                dialog.target_game,
                                draft.target_extension,
                                draft.target_group_name
                            ))
                            .color(text_dark())
                            .strong(),
                        );
                        egui::Grid::new("tag_conversion_summary")
                            .num_columns(2)
                            .spacing([20.0, 3.0])
                            .show(ui, |ui| {
                                ui.label("Copied exactly");
                                ui.label(report.copied_exact.to_string());
                                ui.end_row();
                                ui.label("Converted semantically");
                                ui.label(report.converted_semantic.to_string());
                                ui.end_row();
                                ui.label("Mapped through schema/catalog aliases");
                                ui.label(report.mapped_aliases.to_string());
                                ui.end_row();
                                ui.label("Target fields left at defaults");
                                ui.label(report.defaulted_target.to_string());
                                ui.end_row();
                                ui.label("Unsupported source values");
                                ui.label(report.unsupported_source.to_string());
                                ui.end_row();
                                ui.label("Truncated elements");
                                ui.label(report.truncated.to_string());
                                ui.end_row();
                            });

                        if !report.issues.is_empty() {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("Conversion details")
                                    .color(subtle_dark())
                                    .small(),
                            );
                            egui::ScrollArea::vertical()
                                .id_salt("tag_conversion_issues")
                                .max_height(230.0)
                                .show(ui, |ui| {
                                    for issue in &report.issues {
                                        let kind = match issue.kind {
                                            ConversionIssueKind::Unsupported => "Unsupported",
                                            ConversionIssueKind::Truncated => "Truncated",
                                            ConversionIssueKind::Warning => "Warning",
                                        };
                                        ui.label(
                                            RichText::new(format!(
                                                "{kind}: {} — {}",
                                                issue.path, issue.message
                                            ))
                                            .color(subtle_dark())
                                            .small(),
                                        );
                                    }
                                });
                        }
                    }

                    if let Some(error) = dialog.error.as_ref() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(error).color(material_delete_text()));
                    }

                    if let Some(output) = dialog.pending_source_destination.as_ref() {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.label(
                            RichText::new(
                                "This destination is inside the currently loaded source tags folder. The converted tag uses a different profile and will not be added to the current browser.",
                            )
                            .color(material_delete_text()),
                        );
                        ui.label(
                            RichText::new(output.display().to_string())
                                .monospace()
                                .small()
                                .color(subtle_dark()),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Save There Anyway").clicked() {
                                confirm_inside_source = true;
                            }
                            if ui.button("Choose Another Location").clicked() {
                                cancel_inside_source = true;
                                choose_and_save = true;
                            }
                        });
                    } else {
                        ui.add_space(10.0);
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui
                                    .add_enabled(
                                        dialog.draft.is_some(),
                                        egui::Button::new("Choose Location and Save..."),
                                    )
                                    .clicked()
                                {
                                    choose_and_save = true;
                                }
                                ui.label(
                                    RichText::new(
                                        "Saving creates a new copy; the source tag is not modified.",
                                    )
                                    .color(subtle_dark())
                                    .small(),
                                );
                            },
                        );
                    }
                });
        }

        if analyze {
            self.analyze_tag_conversion();
        }
        if cancel_inside_source {
            if let Some(dialog) = self.tag_conversion_dialog.as_mut() {
                dialog.pending_source_destination = None;
            }
        }
        if confirm_inside_source {
            self.confirm_tag_conversion_inside_source();
        } else if choose_and_save {
            self.choose_tag_conversion_destination();
        }
        if !open {
            self.tag_conversion_dialog = None;
        }
    }

    pub(super) fn draw_folder_conversion_window(&mut self, ctx: &egui::Context) {
        if !self.expert_mode
            && self
                .folder_conversion_dialog
                .as_ref()
                .is_none_or(|dialog| !dialog.running)
        {
            self.folder_conversion_dialog = None;
            return;
        }
        if self.folder_conversion_dialog.is_none() {
            return;
        }
        let mut open = true;
        let mut choose_destination = false;
        let mut start = false;
        let running;
        {
            let dialog = self
                .folder_conversion_dialog
                .as_mut()
                .expect("checked above");
            running = dialog.running;
            egui::Window::new("Save Folder for Another Game")
                .id(egui::Id::new("folder_conversion"))
                .open(&mut open)
                .default_width(760.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new("Source folder").color(subtle_dark()).small());
                    ui.label(RichText::new(&dialog.source_label).monospace().color(text_dark()));
                    ui.horizontal(|ui| {
                        ui.label(format!("Source profile: {}", dialog.source_game));
                        ui.label("Target profile:");
                        let previous = dialog.target_game.clone();
                        ui.add_enabled_ui(!dialog.running, |ui| {
                            egui::ComboBox::from_id_salt("folder_conversion_target")
                                .selected_text(&dialog.target_game)
                                .show_ui(ui, |ui| {
                                    for game in CONVERSION_GAMES {
                                        if *game != dialog.source_game {
                                            ui.selectable_value(
                                                &mut dialog.target_game,
                                                (*game).to_owned(),
                                                *game,
                                            );
                                        }
                                    }
                                });
                        });
                        if dialog.target_game != previous {
                            dialog.destination_parent = None;
                            dialog.report = None;
                            dialog.error = None;
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!dialog.running, egui::Button::new("Choose Destination..."))
                            .clicked()
                        {
                            choose_destination = true;
                        }
                        if let Some(destination) = dialog.destination_parent.as_ref() {
                            ui.label(
                                RichText::new(format!(
                                    "{}\\{}",
                                    destination.display(),
                                    dialog.source_label
                                ))
                                .monospace()
                                .small()
                                .color(subtle_dark()),
                            );
                        }
                    });
                    ui.label(
                        RichText::new(
                            "Existing destination tags are replaced atomically. Reference paths are not relocated.",
                        )
                        .small()
                        .color(subtle_dark()),
                    );

                    if let Some(progress) = dialog.progress.as_ref() {
                        ui.add_space(8.0);
                        let fraction = if progress.total == 0 {
                            0.0
                        } else {
                            progress.processed as f32 / progress.total as f32
                        };
                        ui.label(RichText::new(&progress.phase).strong());
                        ui.add(
                            egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                                .animate(progress.total == 0)
                                .text(format!(
                                    "{} / {} — {} converted, {} failed",
                                    progress.processed,
                                    progress.total,
                                    progress.converted,
                                    progress.failed
                                )),
                        );
                        if !progress.current.is_empty() {
                            ui.label(
                                RichText::new(&progress.current)
                                    .monospace()
                                    .small()
                                    .color(subtle_dark()),
                            );
                        }
                        ctx.request_repaint();
                    }

                    if let Some(report) = dialog.report.as_ref() {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "Completed: {} native-layout, {} generated-layout, {} failed, {} ignored",
                                report.native_count(),
                                report.generated_count(),
                                report.failed_count(),
                                report.ignored_files.len()
                            ))
                            .strong(),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} -> {} | Destination: {}",
                                report.source_label,
                                report.target_game,
                                report.destination_root.display()
                            ))
                            .monospace()
                            .small(),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("folder_conversion_results")
                            .max_height(320.0)
                            .show(ui, |ui| {
                                for wanted in [
                                    FolderConversionFileStatus::NativeLayout,
                                    FolderConversionFileStatus::GeneratedLayout,
                                    FolderConversionFileStatus::Failed,
                                ] {
                                    let (label, color) = match wanted {
                                        FolderConversionFileStatus::NativeLayout => {
                                            ("Native-layout verified", text_dark())
                                        }
                                        FolderConversionFileStatus::GeneratedLayout => (
                                            "Generated layout — native compatibility unverified",
                                            material_delete_text(),
                                        ),
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
                                        RichText::new(format!("{label} ({})", matching.len()))
                                            .color(color),
                                        |ui| {
                                            for file in matching {
                                                let replaced = if file.overwritten {
                                                    " [replaced]"
                                                } else {
                                                    ""
                                                };
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
                    if let Some(error) = dialog.error.as_ref() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(error).color(material_delete_text()));
                    }
                    ui.add_space(8.0);
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui
                                .add_enabled(
                                    !dialog.running && dialog.destination_parent.is_some(),
                                    egui::Button::new("Convert Folder"),
                                )
                                .clicked()
                            {
                                start = true;
                            }
                        },
                    );
                });
        }
        if choose_destination {
            self.choose_folder_conversion_destination();
        }
        if start {
            self.begin_folder_conversion();
        }
        if !open && !running {
            self.folder_conversion_dialog = None;
        }
    }

    /// Destructive-save confirmation for Campaign Evolved container tags. Save
    /// overwrites the shipped pak files in place, so we always confirm and point
    /// the user at Export Mod as the non-destructive alternative.
    /// Strip anything from a mod name that cannot be part of a file name.
    ///
    /// The name becomes three files in a folder the user does not type, so a
    /// separator or a reserved character here is a write failure later rather
    /// than a different path.
    fn sanitize_mod_name(name: &str) -> String {
        name.chars()
            .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
            .collect::<String>()
            .trim()
            .to_owned()
    }

    /// Review what Export Mod is about to write, and where.
    ///
    /// The save dialog this replaces asked for one file name when the output is
    /// three, which invited renaming -- and renaming is how a mod loses the
    /// `_P` that gives it priority over the game's own containers. It also
    /// guarded only the container, silently overwriting the `.ucas` and `.pak`
    /// beside it.
    pub(super) fn draw_mod_export_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.mod_export.as_ref() else {
            return;
        };
        let kit = dialog.kit;
        let new_count = dialog
            .rows
            .iter()
            .filter(|row| row.kind == ModExportChange::New)
            .count();
        let modified_count = dialog
            .rows
            .iter()
            .filter(|row| row.kind == ModExportChange::Modified)
            .count();
        let unresolved_count = dialog
            .rows
            .iter()
            .filter(|row| row.kind == ModExportChange::Unresolved)
            .count();
        let stem = dialog.stem();
        let existing = dialog.existing_files();
        let in_game_folder = self
            .kits
            .iter()
            .find(|k| k.id == kit)
            .and_then(|k| k.source.as_ref())
            .map(|source| source.source.root_path() == dialog.folder)
            .unwrap_or(false);
        let included = dialog.included().count();
        let name_ok = !dialog.name.trim().is_empty();

        let mut open = true;
        let mut cancel = false;
        let mut export = false;
        let mut browse = false;
        let mut set_all: Option<bool> = None;
        let mut toggled: Option<usize> = None;
        let mut name_edit = dialog.name.clone();

        egui::Window::new("Export Mod")
            .id(egui::Id::new("mod_export"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(680.0)
            .default_height(460.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let Some(dialog) = self.mod_export.as_ref() else {
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{new_count} new · {modified_count} modified"
                        ))
                        .color(text_dark()),
                    );
                    if unresolved_count > 0 {
                        ui.label(
                            RichText::new(format!("· {unresolved_count} excluded"))
                                .color(egui::Color32::from_rgb(210, 120, 90)),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Include none").clicked() {
                            set_all = Some(false);
                        }
                        if ui.button("Include all").clicked() {
                            set_all = Some(true);
                        }
                    });
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (index, row) in dialog.rows.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let mut include = row.include;
                                let enabled = row.kind != ModExportChange::Unresolved;
                                if ui
                                    .add_enabled(enabled, egui::Checkbox::new(&mut include, ""))
                                    .changed()
                                {
                                    toggled = Some(index);
                                }
                                let (marker, color) = match row.kind {
                                    ModExportChange::New => ("+", added_text()),
                                    ModExportChange::Modified => ("~", modified_text()),
                                    ModExportChange::Unresolved => {
                                        ("!", egui::Color32::from_rgb(210, 120, 90))
                                    }
                                };
                                // A marker as well as a colour: this is a
                                // confirmation before writing files, and colour
                                // alone excludes a good number of readers.
                                ui.label(RichText::new(marker).color(color).monospace());
                                ui.label(RichText::new(&row.display_path).color(color));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!("{} KB", row.bytes / 1024))
                                                .color(subtle_dark())
                                                .small(),
                                        );
                                        if let Some(reason) = row.reason.as_deref() {
                                            ui.label(
                                                RichText::new(reason).color(subtle_dark()).small(),
                                            );
                                        }
                                    },
                                );
                            });
                        }
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Mod name").color(text_dark()));
                    ui.add(egui::TextEdit::singleline(&mut name_edit).desired_width(220.0));
                    ui.label(
                        RichText::new(format!("{stem}.utoc / .ucas / .pak"))
                            .color(subtle_dark())
                            .monospace()
                            .small(),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Folder").color(text_dark()));
                    ui.label(
                        RichText::new(dialog.folder.display().to_string())
                            .color(subtle_dark())
                            .monospace()
                            .small(),
                    );
                    if ui.button("Browse...").clicked() {
                        browse = true;
                    }
                });
                if in_game_folder {
                    ui.label(
                        RichText::new("This is the game's own Paks folder — nothing to copy afterwards.")
                            .color(subtle_dark())
                            .small(),
                    );
                }
                if !existing.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("Overwrites: {}", existing.join(", ")))
                            .color(egui::Color32::from_rgb(210, 120, 90)),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let ready = name_ok && included > 0;
                    if ui
                        .add_enabled(ready, egui::Button::new("Export"))
                        .on_disabled_hover_text(if name_ok {
                            "Nothing is selected to export"
                        } else {
                            "Enter a name for the mod"
                        })
                        .clicked()
                    {
                        export = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        // Applied after the window closes its borrow of `self`.
        if let Some(dialog) = self.mod_export.as_mut() {
            if dialog.name != name_edit {
                dialog.name = Self::sanitize_mod_name(&name_edit);
                dialog.overwrite_acknowledged = false;
            }
            if let Some(value) = set_all {
                for row in dialog.rows.iter_mut() {
                    if row.kind != ModExportChange::Unresolved {
                        row.include = value;
                    }
                }
            }
            if let Some(index) = toggled
                && let Some(row) = dialog.rows.get_mut(index)
            {
                row.include = !row.include;
            }
        }
        if browse
            && let Some(folder) = rfd::FileDialog::new()
                .set_title("Export mod into folder")
                .pick_folder()
            && let Some(dialog) = self.mod_export.as_mut()
        {
            dialog.folder = folder;
            dialog.overwrite_acknowledged = false;
        }
        if !open || cancel {
            self.mod_export = None;
            return;
        }
        if export {
            let Some(dialog) = self.mod_export.as_ref() else {
                return;
            };
            let included: HashSet<String> = dialog
                .included()
                .map(|row| row.identity.clone())
                .collect();
            let output = dialog.folder.join(format!("{}.utoc", dialog.stem()));
            let snapshot = dialog.snapshot.clone();
            // The workspace may have been closed while this was open.
            if self.focus_navigation_kit(kit) {
                self.write_reviewed_mod(&snapshot, &included, output);
            }
            self.mod_export = None;
        }
    }

    /// What to do with a mod that was just exported.
    ///
    /// A mod is three files and only the `.pak` looks like one, so copying that
    /// alone -- the obvious thing to do -- produces a mod the game finds and
    /// then has nothing to load. The instruction used to live in the status
    /// line, was lost when exports moved onto projects, and the status line now
    /// clears itself after a few seconds besides.
    pub(super) fn draw_exported_mod_window(&mut self, ctx: &egui::Context) {
        let Some(exported) = self.exported_mod.as_ref() else {
            return;
        };
        let stem = exported.stem.clone();
        let directory = exported.directory.clone();
        let count = exported.count;
        let skipped = exported.skipped;
        let mut open = true;
        let mut close = false;
        let mut reveal = false;
        egui::Window::new("Mod exported")
            .id(egui::Id::new("exported_mod"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("{count} tag(s) exported. The base game is unchanged."))
                        .color(text_dark()),
                );
                if skipped > 0 {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "{skipped} tag(s) in this workspace's project could not be resolved \
                             and are NOT in this mod."
                        ))
                        .color(egui::Color32::from_rgb(210, 120, 90)),
                    );
                }
                ui.add_space(10.0);
                ui.label(RichText::new("Copy all three files into the game:").color(text_dark()));
                ui.add_space(4.0);
                for extension in ["utoc", "ucas", "pak"] {
                    ui.label(
                        RichText::new(format!("    {stem}.{extension}"))
                            .color(text_dark())
                            .monospace(),
                    );
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new("    Meteorite/Content/Paks/")
                        .color(text_dark())
                        .monospace(),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(
                        "All three are needed. The .pak is the only one the game scans for, but \
                         the tag data is in the .ucas -- copying it alone loads nothing.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "If you rename them, keep the _P suffix. It is what gives the mod \
                         priority over the game's own files; without it the mod is ignored.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if !directory.as_os_str().is_empty()
                        && ui.button("Show in file browser").clicked()
                    {
                        reveal = true;
                    }
                    if ui.button("Done").clicked() {
                        close = true;
                    }
                });
            });
        if reveal {
            self.open_folder_in_explorer(directory, "mod");
        }
        if !open || close {
            self.exported_mod = None;
        }
    }

    /// Confirmation for the Campaign Evolved "clear modifications" action.
    ///
    /// It is irreversible and can drop work stashed in earlier sessions, so it
    /// lists exactly what is about to go rather than asking in the abstract.
    pub(super) fn draw_clear_stash_confirm_window(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.clear_stash_confirm.as_ref() else {
            return;
        };
        let kit = confirm.kit;
        let stashed = confirm.stashed.clone();
        let unsaved = confirm.unsaved;
        let mut open = true;
        let mut do_clear = false;
        let mut cancel = false;
        egui::Window::new("Clear unsaved modifications?")
            .id(egui::Id::new("clear_stash_confirm"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Every tag in this workspace goes back to the way the game ships it.",
                    )
                    .color(text_dark()),
                );
                ui.add_space(6.0);
                if unsaved > 0 {
                    let noun = if unsaved == 1 { "tag" } else { "tags" };
                    ui.label(
                        RichText::new(format!("{unsaved} open {noun} with unsaved edits"))
                            .color(text_dark()),
                    );
                }
                if !stashed.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "{} tag(s) stashed in this workspace's project, including any kept \
                             from earlier sessions:",
                            stashed.len()
                        ))
                        .color(text_dark()),
                    );
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for path in &stashed {
                                ui.label(RichText::new(path).color(text_dark()).monospace().small());
                            }
                        });
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "This cannot be undone. Tags already saved into the game's pak files, \
                         and mods you have already exported, are not affected.",
                    )
                    .color(egui::Color32::from_rgb(210, 120, 90)),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Clear Modifications").clicked() {
                        do_clear = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if !open || cancel {
            self.clear_stash_confirm = None;
        } else if do_clear {
            self.clear_stash_confirm = None;
            // Resolved rather than assumed: the workspace may have been closed
            // while the confirmation was up.
            if let Some(index) = self.resolve_kit(kit) {
                self.clear_campaign_stash(index, ctx);
            }
        }
    }

    pub(super) fn draw_overwrite_confirm_window(&mut self, ctx: &egui::Context) {
        let Some((kit, key)) = self
            .overwrite_confirm
            .as_ref()
            .map(|confirm| (confirm.kit, confirm.key.clone()))
        else {
            return;
        };
        let mut open = true;
        let mut do_overwrite = false;
        let mut do_export = false;
        let mut cancel = false;
        let mut dont_ask = !self.confirm_container_overwrite;
        egui::Window::new("Overwrite game files?")
            .id(egui::Id::new("overwrite_confirm"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Save will overwrite this tag inside the game's shipped pak files, in place:")
                        .color(text_dark()),
                );
                ui.add_space(5.0);
                ui.label(RichText::new(&key).color(text_dark()).monospace());
                ui.add_space(9.0);
                ui.label(
                    RichText::new(
                        "This modifies the original game content and cannot be undone without a backup of the pak files.",
                    )
                    .color(egui::Color32::from_rgb(210, 120, 90)),
                );
                ui.add_space(5.0);
                ui.label(
                    RichText::new(
                        "To keep the base game untouched, cancel and use File \u{2192} Export Mod\u{2026} instead — it bundles your changes into a separate mod overlay.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(8.0);
                ui.checkbox(
                    &mut dont_ask,
                    "Don't ask again (changeable in Settings)",
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Overwrite Game Files").clicked() {
                        do_overwrite = true;
                    }
                    if ui.button("Export Mod Instead\u{2026}").clicked() {
                        do_export = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if !open || cancel {
            self.overwrite_confirm = None;
        } else if do_overwrite {
            self.overwrite_confirm = None;
            // Apply the opt-out only when the user commits to the overwrite.
            if dont_ask && self.confirm_container_overwrite {
                self.confirm_container_overwrite = false;
                self.persist_prefs_if_changed();
            }
            // Both actions write through the active kit's source. Return to the
            // workspace this was raised from, and drop it if that workspace has
            // been closed — overwriting the game's paks in place is the last
            // thing that should land on whichever game is focused by now.
            if self.focus_navigation_kit(kit) {
                self.overwrite_current_tag_in_place(&key);
            }
        } else if do_export {
            self.overwrite_confirm = None;
            if self.focus_navigation_kit(kit) {
                self.export_mod();
            }
        }
    }

    pub(super) fn draw_rename_tag_window(&mut self, ctx: &egui::Context) {
        if self.rename_tag.is_none() {
            return;
        }
        let mut open = true;
        let mut do_apply = false;
        let mut cancel = false;
        {
            let state = self.rename_tag.as_mut().expect("checked above");
            let title = if state.is_container && !state.redirect {
                "Save Tag As (New Copy)"
            } else {
                "Rename Tag"
            };
            egui::Window::new(title)
                .id(egui::Id::new("rename_tag"))
                .open(&mut open)
                .default_width(560.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new("Current path").color(subtle_dark()).small());
                    ui.label(
                        RichText::new(&state.old_display)
                            .color(text_dark())
                            .monospace(),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("New name (extension is fixed)")
                            .color(subtle_dark())
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut state.new_path_input)
                                .desired_width(430.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        ui.label(
                            RichText::new(format!(".{}", state.extension)).color(subtle_dark()),
                        );
                    });
                    let preview_parent = state
                        .old_display
                        .rsplit_once('/')
                        .map(|(parent, _)| parent)
                        .unwrap_or("");
                    let preview_name = state.new_path_input.trim();
                    let preview = if preview_name.is_empty() {
                        "(enter a new name)".to_owned()
                    } else if preview_parent.is_empty() {
                        format!("{preview_name}.{}", state.extension)
                    } else {
                        format!("{preview_parent}/{preview_name}.{}", state.extension)
                    };
                    ui.add_space(3.0);
                    ui.label(RichText::new("Preview").color(subtle_dark()).small());
                    ui.label(
                        RichText::new(preview)
                            .color(text_dark())
                            .monospace()
                            .small(),
                    );
                    ui.add_space(8.0);
                    if state.is_container {
                        if state.redirect {
                            ui.label(
                                RichText::new(
                                    "Existing references will be redirected to the new tag via the \
                                     overlay container.",
                                )
                                .color(text_dark()),
                            );
                        } else {
                            ui.label(
                                RichText::new(
                                    "Writes an independent new tag; existing references are \
                                     unchanged.",
                                )
                                .color(text_dark()),
                            );
                        }
                        ui.label(
                            RichText::new(
                                "A higher-priority overlay container is written; base game files \
                                 are never modified.",
                            )
                            .color(subtle_dark())
                            .small(),
                        );
                    } else if state.referrers_unavailable {
                        ui.label(
                            RichText::new(
                                "Reference index unavailable — references are still rewritten on \
                                 apply, but can't be previewed here.",
                            )
                            .color(subtle_dark()),
                        );
                    } else if state.referrers.is_empty() {
                        ui.label(
                            RichText::new("No other tags reference this tag.").color(subtle_dark()),
                        );
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "{} referring tag(s) will be updated:",
                                state.referrers.len()
                            ))
                            .color(text_dark()),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("rename_referrers")
                            .max_height(220.0)
                            .show(ui, |ui| {
                                for referrer in &state.referrers {
                                    ui.label(RichText::new(referrer).color(subtle_dark()).small());
                                }
                            });
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !state.new_path_input.trim().is_empty(),
                                egui::Button::new("Apply"),
                            )
                            .on_hover_text(if state.is_container {
                                "Write a higher-priority overlay container (base game unchanged)"
                            } else {
                                "Move the file on disk and rewrite all references"
                            })
                            .clicked()
                        {
                            do_apply = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
        }
        if do_apply {
            // begin_rename_tag clears `rename_tag` on success; on a validation
            // error it leaves the dialog open with a status message.
            self.begin_rename_tag();
        }
        if cancel || !open {
            self.rename_tag = None;
        }
    }

    /// TSV import window: the user pastes tab-separated rows (header = field
    /// names) and applies them onto the target block's existing elements.
    pub(super) fn draw_tsv_paste_window(&mut self, ctx: &egui::Context) {
        if self.tsv_paste.is_none() {
            return;
        }
        let mut open = true;
        let mut do_apply = false;
        {
            let paste = self.tsv_paste.as_mut().expect("checked above");
            egui::Window::new(format!("Paste TSV → {}", paste.block_label))
                .id(egui::Id::new("tsv_paste"))
                .open(&mut open)
                .default_width(560.0)
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "Paste tab-separated rows (first row = field names) to overwrite \
                             this block's {} element(s), cell by cell. Extra rows are ignored — \
                             add elements first if you need more.",
                            paste.element_count
                        ))
                        .color(subtle_dark()),
                    );
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut paste.text)
                                    .desired_rows(12)
                                    .desired_width(f32::INFINITY)
                                    .font(egui::TextStyle::Monospace)
                                    .hint_text("paste TSV here (Ctrl+V)"),
                            );
                        });
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!paste.text.trim().is_empty(), egui::Button::new("Apply"))
                            .clicked()
                        {
                            do_apply = true;
                        }
                        if let Some(status) = &paste.status {
                            ui.label(RichText::new(status).color(subtle_dark()));
                        }
                    });
                });
        }
        if do_apply {
            self.apply_tsv_paste();
        }
        if !open {
            self.tsv_paste = None;
        }
    }

    pub(super) fn draw_keyword_chooser_window(&mut self, ctx: &egui::Context) {
        if !self.keyword_chooser_open {
            return;
        }
        let mut open = true;
        let mut chosen: Option<String> = None;
        let all = self.kits[self.active].keywords.all_keywords();
        egui::Window::new("Keywords")
            .id(egui::Id::new("keyword_chooser"))
            .open(&mut open)
            .default_width(280.0)
            .show(ctx, |ui| {
                if all.is_empty() {
                    ui.label(
                        RichText::new("No keywords yet — add them on a tag's Keywords bar.")
                            .color(subtle_dark()),
                    );
                }
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (keyword, count) in &all {
                            if ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(format!("{keyword}  ({count})"))
                                            .color(text_dark()),
                                    )
                                    .sense(Sense::click()),
                                )
                                .on_hover_text("Show tags with this keyword")
                                .clicked()
                            {
                                chosen = Some(keyword.clone());
                            }
                        }
                    });
            });
        if let Some(keyword) = chosen {
            self.show_tags_with_keyword(&keyword);
        }
        self.keyword_chooser_open = open;
    }

    /// Reference-graph navigator: parents (referenced by) on the left, children
    /// (references) on the right, with the focused tag and back/forward history.
    pub(super) fn draw_new_tag_window(&mut self, ctx: &egui::Context) {
        if !self.new_tag_open {
            return;
        }

        let mut open = self.new_tag_open;
        let mut refresh_groups = false;
        let mut create = false;
        let mut close_requested = false;
        // Campaign Evolved container sources create the tag in memory (no loose
        // tags folder, no filesystem picker) at a container-relative path.
        let is_container = self.current_source_is_container();
        egui::Window::new("New Tag")
            .id(egui::Id::new("new_tag_dialog"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                if !is_container && self.loaded_tags_root().is_none() {
                    ui.label(
                        RichText::new(
                            "Load a loose editing-kit tags folder before creating a tag.",
                        )
                        .color(subtle_dark()),
                    );
                    ui.add_space(8.0);
                }

                if is_container {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Game").color(subtle_dark()));
                        ui.label("Halo: Campaign Evolved");
                    });
                } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Game").color(subtle_dark()));
                    let before = self.new_tag_dialog.game.clone();
                    let games = crate::app::controller::available_definition_games();
                    let (_, wheel_delta) = combo_box_with_scroll(
                        ui,
                        egui::ComboBox::from_id_salt("new_tag_game")
                            .selected_text(&self.new_tag_dialog.game)
                            .width(220.0),
                        |ui| {
                            for game in &games {
                                ui.selectable_value(
                                    &mut self.new_tag_dialog.game,
                                    game.clone(),
                                    game,
                                );
                            }
                        },
                    );
                    if let Some(delta) = wheel_delta {
                        let current = games
                            .iter()
                            .position(|game| game == &self.new_tag_dialog.game)
                            .unwrap_or(0);
                        if let Some(next) = combo_scroll_next_index(current, games.len(), delta) {
                            self.new_tag_dialog.game = games[next].clone();
                        }
                    }
                    if self.new_tag_dialog.game != before {
                        refresh_groups = true;
                    }
                });
                }

                let selected_group_before = self.new_tag_dialog.selected_group;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Group").color(subtle_dark()));
                    let selected = self
                        .new_tag_dialog
                        .groups
                        .get(self.new_tag_dialog.selected_group)
                        .map(|group| {
                            format!("{} ({})", group.name, format_group_tag(group.group_tag))
                        })
                        .unwrap_or_else(|| "No schemas".to_owned());
                    let (_, wheel_delta) = combo_box_with_scroll(
                        ui,
                        egui::ComboBox::from_id_salt("new_tag_group")
                            .selected_text(selected)
                            .width(320.0),
                        |ui| {
                            for (index, group) in self.new_tag_dialog.groups.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.new_tag_dialog.selected_group,
                                    index,
                                    format!(
                                        "{} ({})",
                                        group.name,
                                        format_group_tag(group.group_tag)
                                    ),
                                );
                            }
                        },
                    );
                    if let Some(delta) = wheel_delta {
                        let current = self.new_tag_dialog.selected_group;
                        if let Some(next) = combo_scroll_next_index(
                            current,
                            self.new_tag_dialog.groups.len(),
                            delta,
                        ) {
                            self.new_tag_dialog.selected_group = next;
                        }
                    }
                });
                if self.new_tag_dialog.selected_group != selected_group_before {
                    // The container path is group-independent (the user types it);
                    // only the loose filesystem output path is tied to the group.
                    if !is_container {
                        self.new_tag_dialog.rel_path.clear();
                        self.new_tag_dialog.output_path = None;
                    }
                    self.new_tag_dialog.error = None;
                }

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Path").color(subtle_dark()));
                    if is_container {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_tag_dialog.rel_path)
                                .hint_text("objects/characters/foo/foo")
                                .desired_width(440.0),
                        );
                    } else {
                        let location = if self.new_tag_dialog.rel_path.is_empty() {
                            "No tag selected".to_owned()
                        } else {
                            self.new_tag_dialog.rel_path.clone()
                        };
                        let mut location_text = location;
                        ui.add_enabled(
                            false,
                            egui::TextEdit::singleline(&mut location_text).desired_width(360.0),
                        );
                        if ui
                            .add_enabled(
                                self.loaded_tags_root().is_some()
                                    && !self.new_tag_dialog.groups.is_empty(),
                                egui::Button::new("Choose..."),
                            )
                            .clicked()
                        {
                            self.choose_new_tag_output_path();
                        }
                    }
                });

                if let Some(group) = self
                    .new_tag_dialog
                    .groups
                    .get(self.new_tag_dialog.selected_group)
                {
                    let hint = if is_container {
                        format!(
                            "Creates a .{} tag in memory. Save writes a new override \
                             container; Export Mod bundles it. The base game is untouched.",
                            group.extension
                        )
                    } else {
                        format!(
                            "Creates a .{} tag relative to the loaded tags folder.",
                            group.extension
                        )
                    };
                    ui.label(RichText::new(hint).color(subtle_dark()).small());
                }

                if let Some(error) = &self.new_tag_dialog.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(error).color(material_delete_text()));
                }

                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        close_requested = true;
                    }
                    let can_create = !self.new_tag_dialog.groups.is_empty()
                        && if is_container {
                            !self.new_tag_dialog.rel_path.trim().is_empty()
                        } else {
                            self.loaded_tags_root().is_some()
                                && self.new_tag_dialog.output_path.is_some()
                        };
                    if ui
                        .add_enabled(can_create, egui::Button::new("Create"))
                        .clicked()
                    {
                        create = true;
                    }
                });
            });

        if refresh_groups {
            self.refresh_new_tag_groups();
        }
        if close_requested {
            open = false;
        }
        self.new_tag_open = open;
        if create {
            self.create_new_tag();
        }
    }

    pub(super) fn draw_import_tag_window(&mut self, ctx: &egui::Context) {
        if self.import_tag_dialog.is_none() {
            return;
        }
        // Snapshot fields for the immutable overwrite lookup before borrowing the
        // dialog mutably for rendering (the banner lags edits by one frame).
        let (folder_snapshot, name_snapshot, group_tag) = {
            let dialog = self.import_tag_dialog.as_ref().unwrap();
            (dialog.folder_rel.clone(), dialog.name.clone(), dialog.group_tag)
        };
        let overwrite_logical =
            self.import_overwrite_target(&folder_snapshot, &name_snapshot, group_tag);

        let mut open = true;
        let mut do_import = false;
        let mut do_cancel = false;
        egui::Window::new("Import Tag")
            .id(egui::Id::new("import_tag_dialog"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                let dialog = self.import_tag_dialog.as_mut().unwrap();

                ui.horizontal(|ui| {
                    ui.label(RichText::new("File").color(subtle_dark()));
                    ui.label(
                        dialog
                            .source_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("(unknown)"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Group").color(subtle_dark()));
                    ui.label(format!(
                        "{} ({})",
                        dialog.group_name,
                        format_group_tag(dialog.group_tag)
                    ));
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Folder").color(subtle_dark()));
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.folder_rel)
                            .hint_text("objects/characters/foo (blank = root)")
                            .desired_width(440.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Name").color(subtle_dark()));
                    ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(440.0));
                });

                match &overwrite_logical {
                    Some(logical) => {
                        ui.label(
                            RichText::new(format!(
                                "⟳ Overwrites existing tag  {logical}.{}",
                                dialog.extension
                            ))
                            .color(Color32::from_rgb(242, 196, 48))
                            .small(),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new("✦ New tag (no base-game counterpart)")
                                .color(subtle_dark())
                                .small(),
                        );
                    }
                }

                match &dialog.comparison {
                    Some(cmp) => match cmp.severity {
                        blam_tags::LayoutSeverity::Match => {
                            ui.label(
                                RichText::new(format!("✔ Schema matches {}", dialog.group_name))
                                    .color(disclosure_triangle_green())
                                    .small(),
                            );
                        }
                        blam_tags::LayoutSeverity::Drift => {
                            ui.label(
                                RichText::new(
                                    "⚠ Schema differs in field metadata only (wire layout \
                                     matches). Safe to import.",
                                )
                                .color(Color32::from_rgb(242, 196, 48))
                                .small(),
                            );
                            ui.checkbox(&mut dialog.import_anyway, "Import anyway");
                        }
                        blam_tags::LayoutSeverity::Incompatible => {
                            ui.label(
                                RichText::new(
                                    "✖ Schema is incompatible (group, version, or size differs) — \
                                     this tag does not match the base game.",
                                )
                                .color(material_delete_text())
                                .small(),
                            );
                        }
                    },
                    None => {
                        ui.label(
                            RichText::new("No shipped definition for this group — not validated.")
                                .color(subtle_dark())
                                .small(),
                        );
                    }
                }

                if let Some(error) = &dialog.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(error).color(material_delete_text()));
                }

                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        do_cancel = true;
                    }
                    let blocked = matches!(
                        dialog.comparison.as_ref().map(|cmp| cmp.severity),
                        Some(blam_tags::LayoutSeverity::Incompatible)
                    ) || dialog.name.trim().is_empty();
                    if ui
                        .add_enabled(!blocked, egui::Button::new("Import"))
                        .clicked()
                    {
                        do_import = true;
                    }
                });
            });

        if !open {
            do_cancel = true;
        }
        if do_cancel {
            self.import_tag_dialog = None;
        } else if do_import {
            self.confirm_import_tag();
        }
    }

    pub(super) fn draw_import_discard_confirm(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.import_discard_confirm.as_ref() else {
            return;
        };
        let label = self.tag_path_label(&pending.target_key);
        let mut discard = false;
        let mut cancel = false;
        egui::Window::new("Discard unsaved changes?")
            .id(egui::Id::new("import_discard_confirm"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "{label} has unsaved edits. Replace it with the imported tag?"
                ));
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Discard & Replace").clicked() {
                        discard = true;
                    }
                });
            });
        if discard {
            self.apply_import_discard();
        } else if cancel {
            self.import_discard_confirm = None;
        }
    }
}
#[cfg(test)]
mod mod_export_tests {
    use super::*;
    use std::path::PathBuf;

    fn dialog(name: &str) -> ModExportDialog {
        ModExportDialog {
            kit: KitId(0),
            snapshot: CampaignProjectSnapshot {
                game: "haloce_evolved".to_owned(),
                source_path: PathBuf::new(),
                selected_identity: None,
                tabs: Vec::new(),
                overlays: Default::default(),
            },
            rows: Vec::new(),
            name: name.to_owned(),
            folder: PathBuf::from("/tmp"),
            overwrite_acknowledged: false,
        }
    }

    /// `_P` is what gives a mod priority over the game's own containers, so it
    /// is part of the name rather than something a rename can drop -- which is
    /// exactly how a reported mod came to build correctly and do nothing.
    #[test]
    fn the_stem_always_carries_the_priority_suffix() {
        assert_eq!(dialog("h2a_magnum").stem(), "h2a_magnum_P");
        assert_eq!(dialog("h2a_magnum_P").stem(), "h2a_magnum_P");
        assert_eq!(dialog("  spaced  ").stem(), "spaced_P");
    }

    /// The name becomes three files in a folder the user never types, so a
    /// separator would be a write failure rather than a different path.
    #[test]
    fn a_mod_name_cannot_contain_path_syntax() {
        assert_eq!(Baboon::sanitize_mod_name("../../etc/passwd"), "....etcpasswd");
        assert_eq!(Baboon::sanitize_mod_name("my:mod?"), "mymod");
        assert_eq!(Baboon::sanitize_mod_name("  trimmed  "), "trimmed");
    }
}
