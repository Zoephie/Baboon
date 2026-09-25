//! The delete confirmation for a loose tag or a Campaign Evolved container tag.
//! It owns presentation and the choice; the deletion belongs to the controller.

use super::*;

impl Baboon {
    pub(in crate::app::ui) fn draw_delete_confirm_window(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.delete_confirm.as_ref() else {
            return;
        };
        let display_path = confirm.display_path.clone();
        let has_unsaved_edits = confirm.has_unsaved_edits;
        let referrers = confirm.referrers.clone();
        let referrers_unavailable = confirm.referrers_unavailable;
        let container_target = match &confirm.kind {
            DeleteKind::Container { target_label } => Some(target_label.clone()),
            DeleteKind::Loose => None,
        };
        let title = match container_target {
            Some(_) => "Delete from Campaign Evolved container?",
            None => "Delete tag?",
        };
        let mut open = true;
        let mut delete = false;
        let mut cancel = false;
        egui::Window::new(title)
            .id(egui::Id::new("delete_tag_confirm"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("Delete {display_path}")).color(text_dark()));
                ui.add_space(7.0);
                match &container_target {
                    Some(target_label) => {
                        ui.label(
                            RichText::new(format!("Exact target: {target_label}"))
                                .color(text_dark())
                                .monospace(),
                        );
                        ui.add_space(7.0);
                        ui.label(
                            RichText::new(
                                "This changes the target UTOC. The retired bytes stay in the \
                                 UCAS as dead space, and the sibling PAK will not be changed. An \
                                 immutable UTOC backup and manifest are created immediately \
                                 before the delete.",
                            )
                            .color(subtle_dark()),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new(
                                "The file is moved into Baboon's deleted-tags folder, not erased, \
                                 so it can be recovered by hand.",
                            )
                            .color(subtle_dark()),
                        );
                    }
                }
                if has_unsaved_edits {
                    ui.add_space(7.0);
                    ui.label(
                        RichText::new("This tag has unsaved edits. They will be discarded.")
                            .color(text_dark()),
                    );
                }
                ui.add_space(7.0);
                if referrers_unavailable {
                    ui.label(
                        RichText::new(
                            "Reference check unavailable — build the reference index to see what \
                             points at this tag.",
                        )
                        .color(subtle_dark()),
                    );
                } else if referrers.is_empty() {
                    ui.label(RichText::new("Nothing references this tag.").color(subtle_dark()));
                } else {
                    ui.label(
                        RichText::new(format!(
                            "{} tag(s) reference this one and will be left pointing at nothing:",
                            referrers.len()
                        ))
                        .color(text_dark()),
                    );
                    egui::ScrollArea::vertical()
                        .max_height(120.0)
                        .show(ui, |ui| {
                            for referrer in referrers.iter().take(100) {
                                ui.label(RichText::new(referrer).color(subtle_dark()).monospace());
                            }
                        });
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        delete = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if delete {
            self.begin_delete_tag(ctx.clone());
        } else if cancel || !open {
            self.delete_confirm = None;
        }
    }
}
