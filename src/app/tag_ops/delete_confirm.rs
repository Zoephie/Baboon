//! The delete confirmation for a loose tag or a Campaign Evolved container tag.
//! It owns presentation and the choice; the deletion is
//! [`TagOpsCommand::Delete`].

use super::*;

impl Dialog for DeleteConfirm {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let confirm = &*self;
        let display_path = confirm.display_path.clone();
        let has_unsaved_edits = confirm.has_unsaved_edits;
        let referrers = confirm.referrers.clone();
        let referrers_unavailable = confirm.referrers_unavailable;
        let container_target = match &confirm.kind {
            DeleteKind::Container { target_label } => Some(target_label.clone()),
            DeleteKind::Loose => None,
        };
        let title = match container_target {
            Some(_) => "Delete from Campaign Evolved Container?",
            None => "Delete Tag?",
        };
        let mut delete = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("delete_tag_confirm"))
            .frame(egui::Frame::window(&ctx.global_style()))
            .show(ctx, |ui| {
                ui.set_width(window_width(ctx, 560.0));
                crate::app::search::draw_icon_window_header_without_close(ui, title, ButtonIcon::Garbage);
                ui.add_space(12.0);
                ui.label(RichText::new(&display_path).monospace());
                ui.add_space(8.0);
                match &container_target {
                    Some(target_label) => {
                        ui.label(
                            RichText::new(format!("Exact target: {target_label}"))
                                .color(text_dark())
                                .monospace(),
                        );
                        ui.add_space(8.0);
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
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("This tag has unsaved edits. They will be discarded.")
                            .color(text_dark()),
                    );
                }
                ui.add_space(8.0);
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
                    crate::app::ScrollArea::vertical()
                        .max_height(120.0)
                        .show(ui, |ui| {
                            for referrer in referrers.iter().take(100) {
                                ui.label(RichText::new(referrer).color(subtle_dark()).monospace());
                            }
                        });
                }
                ui.add_space(12.0);
                popup_footer(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        cancel = icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked();
                        delete = icon_text_button(ui, ButtonIcon::Garbage, "Delete Tag", true).clicked();
                    });
                });
            });
        // Left open for the delete, which takes it back from the host.
        if delete {
            cx.send(TagOpsCommand::Delete);
        }
        !cancel && !response.should_close()
    }
}
