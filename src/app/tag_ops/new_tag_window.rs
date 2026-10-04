//! The New Tag window.
//! It owns presentation and the group and path chosen; creating the
//! tag is [`TagOpsCommand::CreateNewTag`].

use super::*;

/// The New Tag dialog: a game, a group and where the tag goes. Create sends
/// [`TagOpsCommand::CreateNewTag`], which takes the dialog back from the host
/// and returns it with the reason if the tag cannot be made.
impl Dialog for NewTagDialog {
    fn show(&mut self, cx: &Ctx) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        let mut refresh_groups = false;
        let mut create = false;
        let mut close_requested = false;
        // Campaign Evolved container sources create the tag in memory (no loose
        // tags folder, no filesystem picker) at a container-relative path.
        let is_container = cx.model.current_source_is_container();
        egui::Window::new("New Tag")
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::new("new_tag_dialog"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .default_width(window_width(ctx, 560.0))
            .show(ctx, |ui| {
                if !is_container && cx.model.loaded_tags_root().is_none() {
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
                        let before = self.game.clone();
                        let games = crate::app::documents::saving::available_definition_games();
                        let (_, wheel_delta) = combo_box_with_scroll(
                            ui,
                            egui::ComboBox::from_id_salt("new_tag_game")
                                .selected_text(&self.game)
                                .width(220.0),
                            |ui| {
                                for game in &games {
                                    ui.selectable_value(&mut self.game, game.clone(), game);
                                }
                            },
                        );
                        if let Some(delta) = wheel_delta {
                            let current = games
                                .iter()
                                .position(|game| game == &self.game)
                                .unwrap_or(0);
                            if let Some(next) = combo_scroll_next_index(current, games.len(), delta)
                            {
                                self.game = games[next].clone();
                            }
                        }
                        if self.game != before {
                            refresh_groups = true;
                        }
                    });
                }

                let selected_group_before = self.selected_group;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Group").color(subtle_dark()));
                    let selected = self
                        .groups
                        .get(self.selected_group)
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
                            for (index, group) in self.groups.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.selected_group,
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
                        let current = self.selected_group;
                        if let Some(next) =
                            combo_scroll_next_index(current, self.groups.len(), delta)
                        {
                            self.selected_group = next;
                        }
                    }
                });
                if let Some((authorable, note)) = self.authorability.clone() {
                    ui.label(
                        RichText::new(note)
                            .color(if authorable {
                                subtle_dark()
                            } else {
                                Color32::from_rgb(170, 130, 60)
                            })
                            .small(),
                    );
                }
                if self.selected_group != selected_group_before {
                    self.refresh_authorability(cx.model);
                    // The container path is group-independent (the user types it);
                    // only the loose filesystem output path is tied to the group.
                    if !is_container {
                        self.rel_path.clear();
                        self.output_path = None;
                    }
                    self.error = None;
                }

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Path").color(subtle_dark()));
                    if is_container {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.rel_path)
                                // Prefixed, because a bare path here reads as a
                                // filled field: the placeholder looked exactly
                                // like a value someone had already typed, and
                                // Create sat disabled with no explanation.
                                .hint_text(placeholder_text("e.g. objects/characters/foo/foo"))
                                .desired_width(440.0),
                        );
                    } else {
                        let location = if self.rel_path.is_empty() {
                            "No tag selected".to_owned()
                        } else {
                            self.rel_path.clone()
                        };
                        let mut location_text = location;
                        ui.add_enabled(
                            false,
                            egui::TextEdit::singleline(&mut location_text).desired_width(360.0),
                        );
                        if ui
                            .add_enabled(
                                cx.model.loaded_tags_root().is_some() && !self.groups.is_empty(),
                                egui::Button::new("Choose..."),
                            )
                            .clicked()
                        {
                            self.choose_output_path(cx.model);
                        }
                    }
                });

                if let Some(group) = self.groups.get(self.selected_group) {
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

                if let Some(error) = &self.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(error).color(material_delete_text()));
                }

                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        close_requested = true;
                    }
                    let can_create = !self.groups.is_empty()
                        && if is_container {
                            !self.rel_path.trim().is_empty()
                        } else {
                            cx.model.loaded_tags_root().is_some() && self.output_path.is_some()
                        };
                    if ui
                        .add_enabled(can_create, egui::Button::new("Create"))
                        .on_disabled_hover_text(if self.groups.is_empty() {
                            "No tag groups are available for this game"
                        } else if is_container {
                            "Enter a path for the new tag"
                        } else if cx.model.loaded_tags_root().is_none() {
                            "Load a loose editing-kit tags folder first"
                        } else {
                            "Choose where to save the new tag"
                        })
                        .clicked()
                    {
                        create = true;
                    }
                });
            });

        if refresh_groups {
            self.refresh_groups(cx.model);
        }
        if create {
            cx.send(TagOpsCommand::CreateNewTag);
        }
        open && !close_requested
    }
}
