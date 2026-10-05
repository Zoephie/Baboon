//! The "Overwrite game files?" confirmation before a Campaign Evolved container tag is saved in place.
//! It owns presentation and the choice; the save is
//! [`ModsCommand::Overwrite`] or [`ModsCommand::ExportInstead`].

use super::*;

impl Dialog for OverwriteConfirm {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let (kit, key) = (self.kit, self.key.clone());
        let mut open = true;
        let mut do_overwrite = false;
        let mut do_export = false;
        let mut cancel = false;
        let mut dont_ask = !cx.model.prefs.confirm_container_overwrite;
        // Which container this would actually be written into. With a mod mounted
        // over the tag, that is the mod — not the game's shipped pak, which is
        // what this dialog used to promise in every case.
        let target = cx
            .model
            .resolve_kit(kit)
            .and_then(|index| cx.model.container_label_for_tag(index, &key));
        egui::Window::new("Overwrite game files?")
            .id(egui::Id::new("overwrite_confirm"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(window_width(ctx, 520.0))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(match target.as_ref() {
                        Some((label, true)) => format!(
                            "Save will overwrite this tag inside the mounted mod {label}, in place:"
                        ),
                        Some((label, false)) => format!(
                            "Save will overwrite this tag inside the game's shipped pak {label}, \
                             in place:"
                        ),
                        None => "Save will overwrite this tag inside the game's shipped pak files, \
                                 in place:"
                            .to_owned(),
                    })
                    .color(text_dark()),
                );
                ui.add_space(5.0);
                ui.label(RichText::new(&key).color(text_dark()).monospace());
                ui.add_space(9.0);
                ui.label(
                    RichText::new(
                        "This modifies the original game content and cannot be undone without a backup of the pak files.",
                    )
                    .color(egui::Color32::from_rgb(210, 120, 90)),
                );
                ui.add_space(5.0);
                ui.label(
                    RichText::new(
                        "To keep the base game untouched, cancel and use File \u{2192} Export Mod\u{2026} instead — it bundles your changes into a separate mod overlay.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(8.0);
                ui.checkbox(
                    &mut dont_ask,
                    "Don't ask again (changeable in Settings)",
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Overwrite Game Files").clicked() {
                        do_overwrite = true;
                    }
                    if ui.button("Export Mod Instead\u{2026}").clicked() {
                        do_export = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if do_overwrite {
            cx.send(ModsCommand::Overwrite {
                kit,
                key,
                stop_asking: dont_ask,
            });
        } else if do_export {
            cx.send(ModsCommand::ExportInstead { kit });
        }
        open && !cancel && !do_overwrite && !do_export
    }
}
