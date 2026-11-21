//! The banner across a tag pane whose file changed on disk while the tag had
//! unsaved edits: what happened on the left, the two answers on the right.
//! It sits between the tag's header and its scrolling fields, so it moves the
//! fields down without changing how far they are scrolled.

use super::*;

/// Draw `key`'s changed-on-disk banner, if it has one, and send the answer.
pub(in crate::app) fn draw_disk_banner(cx: &Ctx, ui: &mut Ui, kit_index: usize, entry: &TagEntry) {
    let kit = &cx.model.kits[kit_index];
    let Some(DiskNotice::Changed(_)) = kit.disk.notices.get(&entry.key) else {
        return;
    };
    let message = format!("{} changed on disk. You have unsaved edits.", tag_file_name(entry));
    let mut choice = None;
    egui::Frame::new()
        .fill(disk_banner_fill())
        .stroke(egui::Stroke::new(1.0, disk_banner_edge()))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(button_icon_image(ui, ButtonIcon::Warning, disk_banner_icon(), 16.0));
                ui.label(message);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new("Keep My Edits").min_size(egui::vec2(0.0, BUTTON_HEIGHT)))
                        .on_hover_text("Keep editing your version. Saving will replace the file on disk.")
                        .clicked()
                    {
                        choice = Some(DiskChoice::KeepMyEdits);
                    }
                    if icon_text_button(ui, ButtonIcon::Refresh, "Reload from Disk", true)
                        .on_hover_text("Discard your unsaved edits and load the file as it is now.")
                        .clicked()
                    {
                        choice = Some(DiskChoice::ReloadFromDisk);
                    }
                });
            });
        });
    if let Some(choice) = choice {
        cx.send(DocumentsCommand::DiskChoice { kit: kit.id, key: entry.key.clone(), choice });
    }
}
