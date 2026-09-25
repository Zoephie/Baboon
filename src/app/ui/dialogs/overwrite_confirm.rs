//! The "Overwrite game files?" confirmation before a Campaign Evolved container tag is saved in place.
//! It owns presentation and the choice; the save belongs to the controller.

use super::*;

impl Baboon {
    pub(in crate::app::ui) fn draw_overwrite_confirm_window(&mut self, ctx: &egui::Context) {
        let Some((kit, key)) = self
            .overwrite_confirm
            .as_ref()
            .map(|confirm| (confirm.kit, confirm.key.clone()))
        else {
            return;
        };
        let mut open = true;
        let mut do_overwrite = false;
        let mut do_export = false;
        let mut cancel = false;
        let mut dont_ask = !self.confirm_container_overwrite;
        // Which container this would actually be written into. With a mod mounted
        // over the tag, that is the mod — not the game's shipped pak, which is
        // what this dialog used to promise in every case.
        let target = self
            .resolve_kit(kit)
            .and_then(|index| self.container_label_for_tag(index, &key));
        egui::Window::new("Overwrite game files?")
            .id(egui::Id::new("overwrite_confirm"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
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
        if !open || cancel {
            self.overwrite_confirm = None;
        } else if do_overwrite {
            self.overwrite_confirm = None;
            // Apply the opt-out only when the user commits to the overwrite.
            if dont_ask && self.confirm_container_overwrite {
                self.confirm_container_overwrite = false;
                self.persist_prefs_if_changed();
            }
            // Both actions write through the active kit's source. Return to the
            // workspace this was raised from, and drop it if that workspace has
            // been closed — overwriting the game's paks in place is the last
            // thing that should land on whichever game is focused by now.
            if self.focus_navigation_kit(kit) {
                self.begin_overwrite_current_tag_in_place(&key, ctx);
            }
        } else if do_export {
            self.overwrite_confirm = None;
            if self.focus_navigation_kit(kit) {
                self.export_mod();
            }
        }
    }
}
