//! The window Save opens when a tag's file changed on disk since it was
//! opened: the user must choose between their edits and the file before
//! anything is written. It owns presentation and the choice; acting on it is
//! [`DocumentsCommand::DiskChoice`]. Cancel leaves the tag's banner up, so
//! the choice is still waiting there.

use super::*;
use crate::app::browser::bulk_rename::{ITEM_ROW_HEIGHT, RenameIcon, draw_item_row, padded_body};
use crate::app::search::draw_icon_window_header_without_close;

/// Save stopped because `key`'s file changed under unsaved edits.
pub(in crate::app) struct DiskSaveConfirm {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) key: String,
    /// What the tag's row shows, taken when Save opened the window.
    pub(in crate::app) display_path: String,
    pub(in crate::app) group_tag: u32,
}

impl Dialog for DiskSaveConfirm {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let Some(index) = cx.model.resolve_kit(self.kit) else {
            return false;
        };
        let kit = &cx.model.kits[index];
        // Answered some other way (the banner, a reload, the tab closed):
        // nothing left to ask.
        if !kit.disk.notices.contains_key(&self.key) {
            return false;
        }
        let game = kit.source.as_ref().and_then(|source| source.game);
        let mut choice = None;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::unique("disk_save_confirm"))
            .frame(egui::Frame::window(&ctx.global_style()))
            .show(ctx, |ui| {
                ui.set_width(window_width(ctx, 480.0));
                draw_icon_window_header_without_close(ui, "Tag Changed on Disk", ButtonIcon::Warning);
                ui.add_space(12.0);
                padded_body(ui, |ui| {
                    draw_item_row(ui, RenameIcon::Tag(Some(self.group_tag)), game, &self.display_path, ITEM_ROW_HEIGHT);
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(
                            "This tag was changed outside Baboon after you opened it, and you have \
                             unsaved edits. Choose which version to keep before saving.",
                        )
                        .color(text_dark()),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "Keep My Edits and Save replaces the file on disk with your version. \
                             Reload from Disk discards your edits and loads the file as it is now.",
                        )
                        .color(subtle_dark()),
                    );
                });
                ui.add_space(12.0);
                popup_footer(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        cancel = icon_text_button(ui, ButtonIcon::Clear, "Cancel", true)
                            .on_hover_text("Don't save yet. The banner on the tag keeps the choice waiting.")
                            .clicked();
                        if icon_text_button(ui, ButtonIcon::Refresh, "Reload from Disk", true).clicked() {
                            choice = Some(DiskChoice::ReloadFromDisk);
                        }
                        if icon_text_button(ui, ButtonIcon::Save, "Keep My Edits and Save", true).clicked() {
                            choice = Some(DiskChoice::KeepAndSave);
                        }
                    });
                });
            });
        if let Some(choice) = choice {
            cx.send(DocumentsCommand::DiskChoice { kit: self.kit, key: self.key.clone(), choice });
        }
        choice.is_none() && !cancel && !response.should_close()
    }
}
