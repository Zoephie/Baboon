//! Modal dialogs for rename, paste, keyword selection, and new-tag workflows.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

impl Baboon {
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
