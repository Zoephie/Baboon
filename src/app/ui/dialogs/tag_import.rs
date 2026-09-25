//! The Import Tags window: a tag, or a folder of them, from another game's kit.
//! It owns presentation and action collection; measuring, converting, and writing belong to the controller and its workers.

use super::*;

/// What the Import Tags window asked for this frame.
///
/// Collected rather than applied inline: every one of these needs `&mut self`,
/// and the window is drawn while the dialog is already borrowed.
enum ImportDialogAction {
    Resolve,
    BrowseFile,
    BrowseFolder,
    /// The source profile changed, so the preview no longer describes the
    /// conversion that would run.
    InvalidateAnalysis,
    Import,
    /// Write the tag whose data loss the user has just been shown and accepted.
    AcceptLosses,
    /// Write the tags a folder run held back, same bargain.
    AcceptHeldBack,
    /// Throw away a lossy conversion rather than write it.
    DiscardLossy,
}

impl Baboon {
    /// Import Tags: bring a tag, or a whole folder of them, in from another
    /// game's editing kit.
    ///
    /// One window covers both, because the difference is a property of the path
    /// the user gave rather than a decision they should have to make first. The
    /// slow parts — measuring the source and building the preview — run on
    /// workers, so a path naming a whole kit's tag tree does not stall a frame.
    pub(in crate::app::ui) fn draw_tag_import_window(&mut self, ctx: &egui::Context) {
        if self.tag_import_dialog.is_none() {
            return;
        }
        // Resolved before the dialog is borrowed mutably, so the banner lags an
        // edit by one frame. That is the same bargain the Campaign Evolved
        // import dialog makes, and it beats re-statting the disk mid-render.
        let (single_output, folder_output, existing) = {
            let dialog = self.tag_import_dialog.as_ref().expect("checked above");
            let single = dialog.single_output();
            let folder = dialog.folder_output_root();
            let existing = if dialog.source_is_folder() {
                folder
                    .as_ref()
                    .is_some_and(|path| recheck_cached(ctx, ("is_dir", path), || path.is_dir()))
            } else {
                single
                    .as_ref()
                    .is_some_and(|path| is_file_cached(ctx, path))
            };
            (single, folder, existing)
        };

        let mut open = true;
        let mut action = None;
        let running;
        {
            let dialog = self.tag_import_dialog.as_mut().expect("checked above");
            running = dialog.running;
            let busy = dialog.running || dialog.analyzing;
            egui::Window::new("Import Tags")
                .id(egui::Id::new("tag_import"))
                .open(&mut open)
                .default_width(720.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Into").color(subtle_dark()));
                        ui.label(RichText::new(&dialog.target_game).color(text_dark()).strong());
                        ui.label(
                            RichText::new(dialog.target_tags_root.display().to_string())
                                .monospace()
                                .small()
                                .color(subtle_dark()),
                        );
                    });
                    ui.add_space(8.0);

                    // ── Source ────────────────────────────────────────────────
                    ui.label(RichText::new("Source").color(subtle_dark()).small());
                    ui.horizontal(|ui| {
                        let response = ui.add_enabled(
                            !busy,
                            egui::TextEdit::singleline(&mut dialog.source_input)
                                .hint_text(placeholder_text("Paste a path to a tag or a folder"))
                                .desired_width(400.0),
                        );
                        // On leaving the box — which Enter also does — never on
                        // a keystroke: resolving walks the path, and a path can
                        // name a folder holding tens of thousands of files.
                        if response.lost_focus() {
                            action = Some(ImportDialogAction::Resolve);
                        }
                        if ui.add_enabled(!busy, egui::Button::new("Choose tag...")).clicked() {
                            action = Some(ImportDialogAction::BrowseFile);
                        }
                        if ui
                            .add_enabled(!busy, egui::Button::new("Choose folder..."))
                            .on_hover_text("Every tag in the folder and all its subfolders")
                            .clicked()
                        {
                            action = Some(ImportDialogAction::BrowseFolder);
                        }
                    });

                    let typed = normalize_import_input(&dialog.source_input);
                    if dialog.resolving {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new("Checking what is in there...")
                                    .color(subtle_dark())
                                    .small(),
                            );
                        });
                        ctx.request_repaint();
                    } else if typed.is_empty() {
                        ui.label(
                            RichText::new(
                                "Pick a tag to convert one, or a folder to convert everything \
                                 under it.",
                            )
                            .color(subtle_dark())
                            .small(),
                        );
                    } else if !dialog.facts_are_current() {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("This path has not been checked yet.")
                                    .color(Color32::from_rgb(242, 196, 48))
                                    .small(),
                            );
                            if ui.small_button("Check").clicked() {
                                action = Some(ImportDialogAction::Resolve);
                            }
                        });
                    } else if let Some(facts) = dialog.facts.as_ref() {
                        if facts.is_folder {
                            let mut summary = format!("{} tag(s) found", facts.tag_files);
                            if facts.skipped_files > 0 {
                                summary.push_str(&format!(
                                    ", {} other file(s) will be skipped",
                                    facts.skipped_files
                                ));
                            }
                            ui.label(
                                RichText::new(summary)
                                    .color(if facts.tag_files == 0 {
                                        material_delete_text()
                                    } else {
                                        text_dark()
                                    })
                                    .small(),
                            );
                        } else {
                            let group = facts
                                .group_tag
                                .map(format_group_tag)
                                .unwrap_or_else(|| "unknown".to_owned());
                            ui.label(
                                RichText::new(format!("One tag, group {group}"))
                                    .color(text_dark())
                                    .small(),
                            );
                        }
                    }

                    // ── Which game it came from ───────────────────────────────
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("From").color(subtle_dark()));
                        let previous = dialog.source_game.clone();
                        ui.add_enabled_ui(!busy, |ui| {
                            egui::ComboBox::from_id_salt("tag_import_source_game")
                                .selected_text(&dialog.source_game)
                                .width(200.0)
                                .show_ui(ui, |ui| {
                                    for game in import_sources_for(&dialog.target_game) {
                                        ui.selectable_value(
                                            &mut dialog.source_game,
                                            game.to_owned(),
                                            game,
                                        );
                                    }
                                });
                        });
                        if dialog.source_game != previous {
                            dialog.source_game_note = "Chosen by hand".to_owned();
                            action = Some(ImportDialogAction::InvalidateAnalysis);
                        }
                    });
                    ui.label(
                        RichText::new(&dialog.source_game_note)
                            .color(subtle_dark())
                            .small(),
                    );

                    // ── Where it lands ────────────────────────────────────────
                    ui.add_space(8.0);
                    ui.label(RichText::new("Destination").color(subtle_dark()).small());
                    ui.horizontal(|ui| {
                        let response = ui.add_enabled(
                            !busy,
                            egui::TextEdit::singleline(&mut dialog.destination_rel)
                                .hint_text(placeholder_text("objects/characters/masterchief"))
                                .desired_width(440.0),
                        );
                        if response.changed() {
                            dialog.destination_touched = true;
                        }
                    });
                    let resolved = if dialog.source_is_folder() {
                        folder_output.clone()
                    } else {
                        single_output.clone()
                    };
                    match resolved.as_ref() {
                        Some(path) => {
                            ui.label(
                                RichText::new(path.display().to_string())
                                    .monospace()
                                    .small()
                                    .color(subtle_dark()),
                            );
                        }
                        None if !dialog.source_is_folder() && dialog.draft.is_none() => {
                            // The extension belongs to the *target* group, so
                            // there is no full path to show until the conversion
                            // has said what the tag becomes.
                            ui.label(
                                RichText::new(
                                    "The file extension follows from the target group, and is \
                                     filled in once the conversion is analyzed.",
                                )
                                .color(subtle_dark())
                                .small(),
                            );
                        }
                        None => {}
                    }
                    if existing {
                        ui.label(
                            RichText::new(if dialog.source_is_folder() {
                                "\u{27f3} This folder already exists — tags with matching names \
                                 are replaced"
                            } else {
                                "\u{27f3} Replaces the tag already at this path"
                            })
                            .color(Color32::from_rgb(242, 196, 48))
                            .small(),
                        );
                    }

                    // ── Preview and run ───────────────────────────────────────
                    ui.add_space(10.0);
                    let ready = dialog.facts_are_current()
                        && dialog
                            .facts
                            .as_ref()
                            .is_some_and(|facts| facts.tag_files > 0);
                    ui.horizontal(|ui| {
                        // One button. Converting and writing are one intention,
                        // and the report below appears once it has run rather
                        // than behind a preview step nothing depends on.
                        if ui
                            .add_enabled(ready && !busy, egui::Button::new("Import"))
                            .on_disabled_hover_text(
                                "Choose a tag or a folder that holds tags first",
                            )
                            .clicked()
                        {
                            action = Some(ImportDialogAction::Import);
                        }
                        if dialog.analyzing {
                            ui.spinner();
                            ui.label(RichText::new("Converting...").color(subtle_dark()).small());
                            ctx.request_repaint();
                        }
                    });

                    // The question. Nothing has been written at this point --
                    // the tag is converted and sitting here, and the user is
                    // being shown exactly what it costs before it lands.
                    if !dialog.pending_losses.is_empty() {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "\u{26a0} This tag converts, but loses {} field(s) {} has no                                  counterpart for:",
                                dialog.pending_losses.len(),
                                dialog.target_game
                            ))
                            .color(Color32::from_rgb(242, 196, 48)),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("tag_import_losses")
                            .max_height(110.0)
                            .show(ui, |ui| {
                                for loss in &dialog.pending_losses {
                                    ui.label(
                                        RichText::new(format!("    {loss}"))
                                            .monospace()
                                            .small()
                                            .color(subtle_dark()),
                                    );
                                }
                            });
                        ui.label(
                            RichText::new(
                                "Everything else came across. Import it anyway, or leave it out                                  and nothing is written.",
                            )
                            .color(subtle_dark())
                            .small(),
                        );
                        ui.horizontal(|ui| {
                            if ui
                                .button("Import anyway, losing those fields")
                                .clicked()
                            {
                                action = Some(ImportDialogAction::AcceptLosses);
                            }
                            if ui
                                .button("Leave it out")
                                .on_hover_text("Discards the conversion; nothing is written")
                                .clicked()
                            {
                                action = Some(ImportDialogAction::DiscardLossy);
                            }
                        });
                    }

                    if let Some(written) = dialog.written.as_ref() {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!("\u{2714} {written}"))
                                .color(disclosure_triangle_green()),
                        );
                    }

                    if let Some(draft) = dialog.draft.as_ref() {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "Becomes a {} {} tag (.{})",
                                dialog.target_game, draft.target_group_name, draft.target_extension
                            ))
                            .color(text_dark())
                            .strong(),
                        );
                        // A routed conversion has been through two or more
                        // engines' worth of loss. Saying so above the numbers is
                        // the difference between a reader trusting them and
                        // knowing what they are.
                        if !draft.route.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "\u{21b3} Routed via {} — {} does not convert to {} \
                                     directly for this tag, so it was carried through in stages. \
                                     Nothing was written along the way.",
                                    draft.route.join(" \u{2192} "),
                                    dialog.source_game,
                                    dialog.target_game,
                                ))
                                .color(Color32::from_rgb(242, 196, 48))
                                .small(),
                            );
                        }
                        match draft.native_layout_template.as_ref() {
                            None => {
                                ui.label(
                                    RichText::new(format!(
                                        "Built from {}'s own definitions — no tag in your kit \
                                         was used or needed",
                                        dialog.target_game
                                    ))
                                    .color(subtle_dark())
                                    .small(),
                                );
                            }
                            // Named, not just counted. A kit ships one group at
                            // several layout revisions, so this is the single
                            // fact that explains why the same import can behave
                            // differently on someone else's copy of the kit —
                            // and it is a line two people can compare.
                            Some(template) => {
                                let shown = template
                                    .strip_prefix(&dialog.target_tags_root)
                                    .unwrap_or(template);
                                ui.label(
                                    RichText::new(format!(
                                        "Started from the kit's {} — {} cannot be built from the \
                                         definitions alone, so its layout came from that tag",
                                        shown.display(),
                                        draft.target_group_name
                                    ))
                                    .color(Color32::from_rgb(242, 196, 48))
                                    .small(),
                                );
                            }
                        }
                        draw_conversion_report(ui, &draft.report, "tag_import");
                    }

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
                                    "{} / {} — {} imported, {} failed",
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
                        draw_folder_import_report(ui, report);
                        if !report.held_back.is_empty() {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(format!(
                                    "\u{26a0} {} tag(s) converted but were not written, because they                                      lose fields {} has no counterpart for.",
                                    report.held_back.len(),
                                    report.target_game
                                ))
                                .color(Color32::from_rgb(242, 196, 48)),
                            );
                            egui::CollapsingHeader::new("What each of them gives up")
                                .id_salt("tag_import_held_back")
                                .show(ui, |ui| {
                                    for entry in &report.held_back {
                                        ui.label(
                                            RichText::new(format!(
                                                "{} — {}",
                                                entry.source,
                                                entry.losses.join(", ")
                                            ))
                                            .small()
                                            .color(subtle_dark()),
                                        );
                                    }
                                });
                            if ui
                                .add_enabled(
                                    !busy,
                                    egui::Button::new(format!(
                                        "Import those {} too, losing those fields",
                                        report.held_back.len()
                                    )),
                                )
                                .on_hover_text(
                                    "Converts and writes only the held-back tags; the rest are                                      already in",
                                )
                                .clicked()
                            {
                                action = Some(ImportDialogAction::AcceptHeldBack);
                            }
                        }
                    }

                    if let Some(error) = dialog.error.as_ref() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(error).color(material_delete_text()));
                    }
                });
        }

        match action {
            Some(ImportDialogAction::Resolve) => self.resolve_import_source(),
            Some(ImportDialogAction::BrowseFile) => self.choose_import_source_file(),
            Some(ImportDialogAction::BrowseFolder) => self.choose_import_source_folder(),
            Some(ImportDialogAction::InvalidateAnalysis) => {
                if let Some(dialog) = self.tag_import_dialog.as_mut() {
                    dialog.draft = None;
                    dialog.draft_stamp = None;
                    dialog.written = None;
                    dialog.error = None;
                }
            }
            Some(ImportDialogAction::Import) => self.begin_tag_import(),
            Some(ImportDialogAction::AcceptLosses) => self.accept_import_losses(ctx),
            Some(ImportDialogAction::AcceptHeldBack) => self.accept_held_back_imports(),
            Some(ImportDialogAction::DiscardLossy) => {
                if let Some(dialog) = self.tag_import_dialog.as_mut() {
                    dialog.draft = None;
                    dialog.draft_stamp = None;
                    dialog.pending_losses.clear();
                    dialog.error = dialog.pending_refusal.take();
                }
            }
            None => {}
        }
        // A running import owns the dialog: closing it would orphan the progress
        // and the report of a job that is still writing files.
        if !open && !running {
            self.tag_import_dialog = None;
        }
    }
}
