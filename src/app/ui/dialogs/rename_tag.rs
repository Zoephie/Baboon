//! The Rename Tag window.
//! It owns presentation and the path entered; the rename belongs to the controller.

use super::*;

impl Baboon {
    pub(in crate::app::ui) fn draw_rename_tag_window(&mut self, ctx: &egui::Context) {
        if self.rename_tag.is_none() {
            return;
        }
        let mut open = true;
        let mut do_apply = false;
        let mut cancel = false;
        {
            let state = self.rename_tag.as_mut().expect("checked above");
            let title = match state.operation {
                TagNameOperation::Duplicate => "Duplicate Tag",
                TagNameOperation::Rename if state.is_new_container => "Rename / Move New Tag",
                // Rename and Move are one operation for a tag in a pak, and both
                // menu items land here. Saying only "Rename" made Move look like
                // it had opened the wrong window.
                TagNameOperation::Rename if state.whole_path_editable => "Rename / Move Tag",
                TagNameOperation::SaveAsOverlay if state.is_new_container => "Copy New Tag",
                TagNameOperation::SaveAsOverlay if state.is_container => "Save Tag As (New Copy)",
                _ => "Rename Tag",
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
                    if state.operation == TagNameOperation::Duplicate {
                        ui.label(
                            RichText::new("Destination leaf (parent and extension are fixed)")
                                .color(subtle_dark())
                                .small(),
                        );
                        ui.horizontal(|ui| {
                            let parent = if state.fixed_parent.is_empty() {
                                "(root)/".to_owned()
                            } else {
                                format!("{}/", state.fixed_parent)
                            };
                            ui.label(RichText::new(parent).color(subtle_dark()).monospace());
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut state.new_path_input)
                                    .id(egui::Id::new("duplicate_tag_name"))
                                    .desired_width(330.0)
                                    .font(egui::TextStyle::Monospace),
                            );
                            if state.focus_input {
                                response.request_focus();
                                if let Some(mut text_state) =
                                    egui::TextEdit::load_state(ctx, response.id)
                                {
                                    text_state.cursor.set_char_range(Some(
                                        egui::text::CCursorRange::two(
                                            egui::text::CCursor::new(0),
                                            egui::text::CCursor::new(
                                                state.new_path_input.chars().count(),
                                            ),
                                        ),
                                    ));
                                    text_state.store(ctx, response.id);
                                }
                                state.focus_input = false;
                            }
                            ui.label(
                                RichText::new(format!(".{}", state.extension))
                                    .color(subtle_dark()),
                            );
                        });
                    } else {
                        ui.label(
                            RichText::new(if state.whole_path_editable {
                                "New path (folders allowed; extension is fixed)"
                            } else {
                                "New name (extension is fixed)"
                            })
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
                                RichText::new(format!(".{}", state.extension))
                                    .color(subtle_dark()),
                            );
                        });
                    }
                    // When the whole path is editable it keeps no parent from
                    // the old one — what is typed IS the destination.
                    let preview_parent = if state.operation == TagNameOperation::Duplicate {
                        state.fixed_parent.as_str()
                    } else if state.whole_path_editable {
                        ""
                    } else {
                        state
                            .old_display
                            .rsplit_once('/')
                            .map(|(parent, _)| parent)
                            .unwrap_or("")
                    };
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
                    if state.operation == TagNameOperation::Duplicate {
                        ui.label(
                            RichText::new(
                                "Creates an independent clean duplicate; existing references are \
                                 unchanged.",
                            )
                            .color(text_dark()),
                        );
                        ui.label(
                            RichText::new(if state.is_container {
                                "The exact current Campaign Evolved container will be backed up \
                                 and updated in place after a separate confirmation."
                            } else {
                                "Current unsaved edits are copied when the source is dirty; clean \
                                 sources are copied byte-for-byte."
                            })
                            .color(subtle_dark())
                            .small(),
                        );
                    } else if state.is_new_container {
                        ui.label(
                            RichText::new(if state.operation == TagNameOperation::Rename {
                                "This tag has not been saved yet, so it simply moves to the new \
                                 path — nothing is written."
                            } else {
                                "Creates a second unsaved tag with a copy of this one's contents."
                            })
                            .color(text_dark()),
                        );
                        ui.label(
                            RichText::new(if state.operation == TagNameOperation::Rename {
                                "It is written when you Save it or Export Mod."
                            } else {
                                "Both tags are written only when you Save them or Export Mod."
                            })
                            .color(subtle_dark())
                            .small(),
                        );
                    } else if let Some(pak) = state.in_place_pak.clone() {
                        ui.label(
                            RichText::new(format!(
                                "Moves this tag inside {pak}, the pack that already holds it. No \
                                 new container is written."
                            ))
                            .color(text_dark()),
                        );
                        ui.label(
                            RichText::new(
                                "That pack is backed up first. Only tags Baboon created can be \
                                 moved this way — the game's own are copied into an overlay \
                                 instead, because moving one would break every reference to it.",
                            )
                            .color(subtle_dark())
                            .small(),
                        );
                    } else if state.is_container {
                        if state.operation == TagNameOperation::Rename {
                            ui.label(
                                RichText::new(
                                    "This is one of the game's own tags, so it is copied to the \
                                     new path in an overlay container rather than moved. Moving it \
                                     would break every reference to it, and a pak cannot forward \
                                     them.",
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
                                egui::Button::new(if state.operation == TagNameOperation::Duplicate {
                                    "Duplicate"
                                } else {
                                    "Apply"
                                }),
                            )
                            .on_hover_text(match state.operation {
                                TagNameOperation::Duplicate => {
                                    "Create an independent duplicate without rewriting references"
                                }
                                TagNameOperation::Rename if state.is_new_container => {
                                    "Move this unsaved tag to the new path (nothing is written yet)"
                                }
                                TagNameOperation::SaveAsOverlay if state.is_new_container => {
                                    "Copy this unsaved tag to the new path (nothing is written yet)"
                                }
                                TagNameOperation::SaveAsOverlay if state.is_container => {
                                    "Write a higher-priority overlay container (base game unchanged)"
                                }
                                TagNameOperation::Rename if state.whole_path_editable => {
                                    "Move this tag inside the pak that already holds it"
                                }
                                _ => "Move the file on disk and rewrite all references",
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
            self.begin_rename_tag(ctx);
        }
        if cancel || !open {
            self.rename_tag = None;
        }
    }
}
