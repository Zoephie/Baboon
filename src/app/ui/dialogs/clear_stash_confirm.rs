//! The Campaign Evolved "clear modifications" confirmation.
//! It owns presentation and the choice; clearing the stash belongs to the controller.

use super::*;

impl Baboon {
    /// Confirmation for the Campaign Evolved "clear modifications" action.
    ///
    /// It is irreversible and can drop work stashed in earlier sessions, so it
    /// lists exactly what is about to go rather than asking in the abstract.
    pub(in crate::app::ui) fn draw_clear_stash_confirm_window(&mut self, ctx: &egui::Context) {
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
                                ui.label(
                                    RichText::new(path).color(text_dark()).monospace().small(),
                                );
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
}
