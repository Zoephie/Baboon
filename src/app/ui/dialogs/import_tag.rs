//! The Import Tag window and its discard confirmation.
//! It owns presentation and request collection; analysing and writing the conversion belong to the controller.

use super::*;

impl Baboon {
    pub(in crate::app::ui) fn draw_import_tag_window(&mut self, ctx: &egui::Context) {
        if self.import_tag_dialog.is_none() {
            return;
        }
        // Snapshot fields for the immutable overwrite lookup before borrowing the
        // dialog mutably for rendering (the banner lags edits by one frame).
        let (folder_snapshot, name_snapshot, group_tag) = {
            let dialog = self.import_tag_dialog.as_ref().unwrap();
            (
                dialog.folder_rel.clone(),
                dialog.name.clone(),
                dialog.group_tag,
            )
        };
        let overwrite_logical =
            self.import_overwrite_target(&folder_snapshot, &name_snapshot, group_tag);

        let mut open = true;
        let mut do_import = false;
        let mut do_cancel = false;
        let mut do_analyze = false;
        // Set to the source profile when the user asks what this conversion
        // will cost; opens the compatibility sheet pre-aimed at the answer.
        let mut show_compat: Option<String> = None;
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
                            .hint_text(placeholder_text("objects/characters/foo (blank = root)"))
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

                let group_name = dialog.group_name.clone();
                let identical_profiles: Vec<String> = dialog
                    .profile_verdicts
                    .iter()
                    .filter(|(_, fit)| fit.is_identical())
                    .map(|(game, _)| game.clone())
                    .collect();
                match &mut dialog.mode {
                    ImportMode::Convert { source_game, draft } => {
                        ui.label(
                            RichText::new(format!("⟳ This is a {source_game} tag"))
                                .color(Color32::from_rgb(242, 196, 48))
                                .small(),
                        );
                        ui.label(
                            RichText::new(
                                "Its root struct matches Campaign Evolved's, but nested structs \
                                 do not. Copying the bytes would land a tag the game reads at \
                                 the wrong offsets, so it is converted instead.",
                            )
                            .color(subtle_dark())
                            .small(),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Source profile").color(subtle_dark()));
                            egui::ComboBox::from_id_salt("import_source_profile")
                                .selected_text(source_game.as_str())
                                .show_ui(ui, |ui| {
                                    for game in &identical_profiles {
                                        if ui.selectable_label(game == source_game, game).clicked()
                                        {
                                            source_game.clone_from(game);
                                            // The draft belongs to the profile
                                            // it was made from.
                                            *draft = None;
                                        }
                                    }
                                });
                            if ui
                                .button(if draft.is_some() {
                                    "Re-analyze"
                                } else {
                                    "Analyze conversion"
                                })
                                .clicked()
                            {
                                do_analyze = true;
                            }
                            if ui
                                .link("What transfers?")
                                .on_hover_text(
                                    "Open the compatibility sheet for this group, in this \
                                     direction",
                                )
                                .clicked()
                            {
                                show_compat = Some(source_game.clone());
                            }
                        });
                        if let Some(draft) = draft.as_ref() {
                            ui.add_space(6.0);
                            draw_conversion_report(ui, &draft.report, "import_conversion");
                        }
                    }
                    ImportMode::Native {
                        comparison,
                        import_anyway,
                    } => match comparison {
                        Some(cmp) => match cmp.severity {
                            blam_tags::LayoutSeverity::Match => {
                                ui.label(
                                    RichText::new(format!("✔ Schema matches {group_name}"))
                                        .color(disclosure_triangle_green())
                                        .small(),
                                );
                            }
                            blam_tags::LayoutSeverity::Drift => {
                                ui.label(
                                    RichText::new(
                                        "⚠ Schema differs in field metadata only (wire layout \
                                         matches), and no other game claims this tag. Safe to \
                                         import.",
                                    )
                                    .color(Color32::from_rgb(242, 196, 48))
                                    .small(),
                                );
                                ui.checkbox(import_anyway, "Import anyway");
                            }
                            blam_tags::LayoutSeverity::Incompatible => {
                                ui.label(
                                    RichText::new(
                                        "✖ Schema is incompatible (group, version, or size \
                                         differs) — this tag does not match the base game.",
                                    )
                                    .color(material_delete_text())
                                    .small(),
                                );
                            }
                        },
                        None => {
                            ui.label(
                                RichText::new(
                                    "No shipped definition for this group — not validated.",
                                )
                                .color(subtle_dark())
                                .small(),
                            );
                        }
                    },
                }

                if dialog.profile_verdicts.len() > 1 {
                    egui::CollapsingHeader::new("Compared against each game")
                        .id_salt("import_profile_verdicts")
                        .show(ui, |ui| {
                            for (game, fit) in &dialog.profile_verdicts {
                                let (mark, color) = match fit {
                                    ProfileFit::Identical => {
                                        ("✔ identical", disclosure_triangle_green())
                                    }
                                    ProfileFit::Diverges(_) => {
                                        ("⚠ differs", Color32::from_rgb(242, 196, 48))
                                    }
                                    ProfileFit::WrongGroup => {
                                        ("✖ different group", material_delete_text())
                                    }
                                };
                                ui.label(
                                    RichText::new(format!("{game}  {mark}"))
                                        .color(color)
                                        .small(),
                                );
                                if let ProfileFit::Diverges(where_) = fit {
                                    ui.label(
                                        RichText::new(format!("      {where_}"))
                                            .color(subtle_dark())
                                            .small(),
                                    );
                                }
                            }
                        });
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
                    let blocked = match &dialog.mode {
                        // Nothing to confirm until a conversion exists.
                        ImportMode::Convert { draft, .. } => draft.is_none(),
                        ImportMode::Native { comparison, .. } => matches!(
                            comparison.as_ref().map(|cmp| cmp.severity),
                            Some(blam_tags::LayoutSeverity::Incompatible)
                        ),
                    } || dialog.name.trim().is_empty();
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
        if let Some(source_game) = show_compat {
            let group = self
                .import_tag_dialog
                .as_ref()
                .map(|dialog| dialog.group_name.clone())
                .unwrap_or_default();
            self.tag_compat.ensure_loaded(&locate_help_docs_root());
            self.tag_compat
                .focus(&source_game, CAMPAIGN_EVOLVED_GAME, &group);
            self.help_panel_tab = HelpPanelTab::TagCompat;
            self.about_open = true;
        }
        if do_cancel {
            self.import_tag_dialog = None;
        } else if do_analyze {
            self.analyze_import_conversion();
        } else if do_import {
            self.confirm_import_tag();
        }
    }

    pub(in crate::app::ui) fn draw_import_discard_confirm(&mut self, ctx: &egui::Context) {
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
