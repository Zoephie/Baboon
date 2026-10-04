//! The Rename Folder window for a loose tags folder.
//! It owns presentation and the name entered; the rename belongs to the controller.

use super::*;

impl Baboon {
    /// Rename Folder for a loose folder: the new name, and what it will change
    /// counted before anything is touched.
    pub(in crate::app::ui) fn draw_loose_folder_rename_window(&mut self, ctx: &egui::Context) {
        if self.loose_folder_rename.is_none() {
            return;
        }
        let mut open = true;
        let mut do_apply = false;
        let mut cancel = false;
        {
            let state = self.loose_folder_rename.as_mut().expect("checked above");
            egui::Window::new("Rename Folder")
                .constrain_to(window_work_area(ctx))
                .id(egui::Id::new("loose_folder_rename"))
                .open(&mut open)
                .default_width(window_width(ctx, 520.0))
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(RichText::new("Parent folder").color(subtle_dark()).small());
                    let parent = if state.parent_display.is_empty() {
                        "(root)".to_owned()
                    } else {
                        state.parent_display.clone()
                    };
                    ui.label(RichText::new(parent).color(text_dark()).monospace());
                    ui.add_space(6.0);

                    ui.label(RichText::new("Folder name").color(subtle_dark()).small());
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut state.name_input)
                            .id(egui::Id::new("loose_folder_rename_name"))
                            .desired_width(480.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    if state.focus_input {
                        response.request_focus();
                        if let Some(mut text_state) = egui::TextEdit::load_state(ctx, response.id) {
                            text_state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(state.name_input.chars().count()),
                                )));
                            text_state.store(ctx, response.id);
                        }
                        state.focus_input = false;
                    }
                    if lost_focus_once(&response) && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        do_apply = true;
                    }
                    if response.changed() {
                        state.error = None;
                    }
                    if let Some(error) = state.error.as_deref() {
                        ui.add_space(4.0);
                        ui.label(RichText::new(error).color(material_delete_text()).small());
                    }

                    ui.add_space(8.0);
                    let new_name = state.name_input.trim();
                    let folder = |name: &str| {
                        if state.parent_display.is_empty() {
                            format!("{name}/")
                        } else {
                            format!("{}/{name}/", state.parent_display)
                        }
                    };
                    ui.label(
                        RichText::new(format!(
                            "{}  →  {}",
                            folder(&state.old_name),
                            if new_name.is_empty() {
                                "(enter a new name)".to_owned()
                            } else {
                                folder(new_name)
                            }
                        ))
                        .color(text_dark())
                        .monospace()
                        .small(),
                    );
                    ui.add_space(8.0);

                    ui.label(
                        RichText::new(format!(
                            "{} tag(s) in this folder and its subfolders will get new paths.",
                            state.tag_count
                        ))
                        .color(text_dark()),
                    );
                    match &state.outside_referrers {
                        None => {
                            ui.label(
                                RichText::new(
                                    "Reference index unavailable: tags elsewhere that reference \
                                     these are still found and updated on apply, but can't be \
                                     counted here.",
                                )
                                .color(subtle_dark()),
                            );
                        }
                        Some(referrers) if referrers.is_empty() => {
                            ui.label(
                                RichText::new("No tags outside this folder reference them.")
                                    .color(subtle_dark()),
                            );
                        }
                        Some(referrers) => {
                            ui.label(
                                RichText::new(format!(
                                    "{} tag(s) outside this folder reference them and will be \
                                     updated:",
                                    referrers.len()
                                ))
                                .color(text_dark()),
                            );
                            egui::ScrollArea::vertical()
                                .id_salt("loose_folder_rename_referrers")
                                .max_height(200.0)
                                .show(ui, |ui| {
                                    for referrer in referrers {
                                        ui.label(
                                            RichText::new(referrer).color(subtle_dark()).small(),
                                        );
                                    }
                                });
                        }
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "References between tags inside the folder are updated too. Baboon \
                             is locked until the rename finishes.",
                        )
                        .color(subtle_dark())
                        .small(),
                    );

                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!new_name.is_empty(), egui::Button::new("Rename"))
                            .on_hover_text("Rename the folder on disk and rewrite all references")
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
            // Only closes once the job has started; a rejected name keeps the
            // dialog up with its reason attached.
            if self.apply_loose_folder_rename() {
                self.loose_folder_rename = None;
            }
        } else if cancel || !open {
            self.loose_folder_rename = None;
        }
    }
}
