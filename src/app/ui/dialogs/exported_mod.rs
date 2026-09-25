//! The window shown after Export Mod: where the mod went and what to copy.
//! It owns presentation only; the export itself belongs to the controller.

use super::*;

impl Baboon {
    /// What to do with a mod that was just exported.
    ///
    /// A mod is three files and only the `.pak` looks like one, so copying that
    /// alone -- the obvious thing to do -- produces a mod the game finds and
    /// then has nothing to load. The instruction used to live in the status
    /// line, was lost when exports moved onto projects, and the status line now
    /// clears itself after a few seconds besides.
    pub(in crate::app::ui) fn draw_exported_mod_window(&mut self, ctx: &egui::Context) {
        let Some(exported) = self.exported_mod.as_ref() else {
            return;
        };
        let stem = exported.stem.clone();
        let directory = exported.directory.clone();
        // The mod's own folder under `~mods`, which is what has to be copied.
        let mod_folder = directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| stem.clone());
        let count = exported.count;
        let skipped = exported.skipped;
        let mut open = true;
        let mut close = false;
        let mut reveal = false;
        egui::Window::new("Mod exported")
            .id(egui::Id::new("exported_mod"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{count} tag(s) exported. The base game is unchanged."
                    ))
                    .color(text_dark()),
                );
                if skipped > 0 {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "{skipped} tag(s) in this workspace's project could not be resolved \
                             and are NOT in this mod."
                        ))
                        .color(egui::Color32::from_rgb(210, 120, 90)),
                    );
                }
                ui.add_space(10.0);
                // The mod is a folder now, so the instruction is to copy the
                // folder — moving the three files loose still works, but it
                // puts them back among the game's own containers.
                ui.label(
                    RichText::new("Copy this folder, with all three files in it, into the game:")
                        .color(text_dark()),
                );
                ui.add_space(4.0);
                for extension in ["utoc", "ucas", "pak"] {
                    ui.label(
                        RichText::new(format!("    {mod_folder}/{stem}.{extension}"))
                            .color(text_dark())
                            .monospace(),
                    );
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("    Meteorite/Content/Paks/{MODS_DIR}/"))
                        .color(text_dark())
                        .monospace(),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(
                        "All three are needed. The .pak is the only one the game scans for, but \
                         the tag data is in the .ucas -- copying it alone loads nothing.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "If you rename them, keep the _P suffix. It is what gives the mod \
                         priority over the game's own files; without it the mod is ignored.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if !directory.as_os_str().is_empty()
                        && ui.button("Show in file browser").clicked()
                    {
                        reveal = true;
                    }
                    if ui.button("Done").clicked() {
                        close = true;
                    }
                });
            });
        if reveal {
            self.open_folder_in_explorer(directory, "mod");
        }
        if !open || close {
            self.exported_mod = None;
        }
    }
}
