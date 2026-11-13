//! The delete confirmation for a loose tag or a Campaign Evolved container tag.
//! It owns presentation and the choice; the deletion is
//! [`TagOpsCommand::Delete`].

use super::*;
use crate::app::browser::bulk_rename::{
    ITEM_ROW_HEIGHT, RenameIcon, draw_item_row, draw_references_box, missing_references_title,
    references_box_height, referrer_rows,
};

impl Dialog for DeleteConfirm {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let confirm = &*self;
        let source = cx.model.kit_index(confirm.kit).and_then(|index| cx.model.kits[index].source.as_ref());
        let game = source.and_then(|source| source.source.game());
        let group = confirm.display_path.rsplit_once('.')
            .and_then(|(_, extension)| source.and_then(|source| source.names.group_tag_for(extension)));
        let rows = source.map(|source| referrer_rows(&confirm.referrers, &source.names)).unwrap_or_default();
        let notes: Vec<String> = confirm.referrers_unavailable
            .then(|| "Reference check unavailable. Build the reference index to see what points at this tag.".to_owned())
            .into_iter().collect();
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
        let response = egui::Modal::new(egui::Id::unique("delete_tag_confirm"))
            .frame(egui::Frame::window(&ctx.global_style()))
            .show(ctx, |ui| {
                ui.set_width(window_width(ctx, 480.0));
                crate::app::search::draw_icon_window_header_without_close(ui, title, ButtonIcon::Garbage);
                ui.add_space(12.0);
                crate::app::browser::bulk_rename::padded_body(ui, |ui| {
                    draw_item_row(ui, RenameIcon::Tag(group), game, &confirm.display_path, ITEM_ROW_HEIGHT);
                    ui.add_space(10.0);
                    match &container_target {
                        Some(target_label) => {
                            ui.label(
                                RichText::new(format!("Exact target: {target_label}"))
                                    .color(text_dark())
                                    .monospace(),
                            );
                            ui.add_space(4.0);
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
                    if confirm.has_unsaved_edits {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("This tag has unsaved edits. They will be discarded.")
                                .color(text_dark()),
                        );
                    }
                    ui.add_space(14.0);
                    let max = (window_work_area(ctx).height() * 0.4).clamp(120.0, 320.0);
                    let height = references_box_height(rows.len(), notes.len(), max);
                    draw_references_box(ui, &missing_references_title(rows.len()), &rows, &notes, game, height);
                });
                ui.add_space(12.0);
                popup_footer(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        cancel = icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked();
                        delete = accent_icon_text_button(ui, ButtonAccent::Destructive, ButtonIcon::Garbage, "Delete Tag", true).clicked();
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
