//! The banner across a tag pane whose file changed outside Baboon: what
//! happened on the left, the answers on the right. Amber when the file
//! changed under unsaved edits, red when it was deleted. It sits between the
//! tag's header and its scrolling fields, so it moves the fields down without
//! changing how far they are scrolled.

use super::*;

/// The colours and words of one kind of banner.
struct BannerLook {
    fill: Color32,
    edge: Color32,
    icon: Color32,
    message: String,
}

/// Draw `entry`'s banner, if it has one, and send the answer.
pub(in crate::app) fn draw_disk_banner(cx: &Ctx, ui: &mut Ui, kit_index: usize, entry: &TagEntry) {
    let kit = &cx.model.kits[kit_index];
    let Some(notice) = kit.disk.notices.get(&entry.key) else {
        return;
    };
    let name = tag_file_name(entry);
    let look = match notice {
        DiskNotice::Changed(_) => BannerLook {
            fill: disk_banner_fill(),
            edge: disk_banner_edge(),
            icon: disk_banner_icon(),
            message: format!("{name} changed on disk. You have unsaved edits."),
        },
        DiskNotice::Missing(_) => {
            let edited = kit.parsed_tags.get(&entry.key).is_some_and(|doc| doc.dirty.is_set());
            let save = if edited { "Save to keep your edits." } else { "Save to restore it." };
            BannerLook {
                fill: disk_missing_fill(),
                edge: disk_missing_edge(),
                icon: disk_missing_icon(),
                message: format!("{name} was deleted from disk. {save}"),
            }
        }
    };
    let missing = matches!(notice, DiskNotice::Missing(_));
    let mut choice = None;
    egui::Frame::new()
        .fill(look.fill)
        .stroke(egui::Stroke::new(1.0, look.edge))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(button_icon_image(ui, ButtonIcon::Warning, look.icon, 16.0));
                ui.label(look.message);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if missing {
                        if icon_text_button(ui, ButtonIcon::CloseTab, "Close Tab", true)
                            .on_hover_text("Close this tab. Unsaved edits are asked about first.")
                            .clicked()
                        {
                            choice = Some(DiskChoice::CloseTab);
                        }
                        return;
                    }
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
