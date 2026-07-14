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

    pub(super) fn draw_rename_tag_window(&mut self, ctx: &egui::Context) {
        if self.rename_tag.is_none() {
            return;
        }
        let mut open = true;
        let mut do_apply = false;
        let mut cancel = false;
        {
            let state = self.rename_tag.as_mut().expect("checked above");
            egui::Window::new("Rename Tag")
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
                    if state.referrers_unavailable {
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
                            .on_hover_text("Move the file on disk and rewrite all references")
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
        let all = self.keywords.all_keywords();
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
        egui::Window::new("New Tag")
            .id(egui::Id::new("new_tag_dialog"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                if self.loaded_tags_root().is_none() {
                    ui.label(
                        RichText::new(
                            "Load a loose editing-kit tags folder before creating a tag.",
                        )
                        .color(subtle_dark()),
                    );
                    ui.add_space(8.0);
                }

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
                    self.new_tag_dialog.rel_path.clear();
                    self.new_tag_dialog.output_path = None;
                    self.new_tag_dialog.error = None;
                }

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Location").color(subtle_dark()));
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
                });

                if let Some(group) = self
                    .new_tag_dialog
                    .groups
                    .get(self.new_tag_dialog.selected_group)
                {
                    ui.label(
                        RichText::new(format!(
                            "Creates a .{} tag relative to the loaded tags folder.",
                            group.extension
                        ))
                        .color(subtle_dark())
                        .small(),
                    );
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
                    let can_create = self.loaded_tags_root().is_some()
                        && !self.new_tag_dialog.groups.is_empty()
                        && self.new_tag_dialog.output_path.is_some();
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
}
