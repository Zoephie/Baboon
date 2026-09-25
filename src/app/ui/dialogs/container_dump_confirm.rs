//! The "this will take a while" confirmation for a container extraction.
//! It owns presentation and the choice; the extraction belongs to the controller.

use super::*;

impl Baboon {
    /// "This will take a while" for a container extraction, whether it covers
    /// the whole workspace or one right-clicked folder.
    ///
    /// Nothing here is destructive, so the wording is about cost rather than
    /// danger: how many files, roughly how long, and — the part that surprises
    /// people — that a mod mounted over the game is ignored, because this
    /// extracts what the game *ships*, not what it currently loads. That last
    /// point holds for a folder too, so it is said unconditionally.
    pub(in crate::app::ui) fn draw_container_dump_confirm_window(&mut self, ctx: &egui::Context) {
        let Some((kit, output, total, folder)) =
            self.container_dump_confirm.as_ref().map(|confirm| {
                let folder = match &confirm.scope {
                    ContainerDumpScope::AllShipped => None,
                    ContainerDumpScope::Folder { label, .. } => Some(label.clone()),
                };
                (confirm.kit, confirm.output.clone(), confirm.total, folder)
            })
        else {
            return;
        };
        let mut open = true;
        let mut do_extract = false;
        let mut cancel = false;
        let title = match &folder {
            Some(_) => "Extract this folder's tags?",
            None => "Extract every shipped tag?",
        };
        egui::Window::new(title)
            .id(egui::Id::new("container_dump_confirm"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(match &folder {
                        Some(label) => {
                            format!("This writes the {total} shipped tag(s) in {label} into:")
                        }
                        None => {
                            format!(
                                "This writes all {total} tags from the mounted containers into:"
                            )
                        }
                    })
                    .color(text_dark()),
                );
                ui.add_space(5.0);
                ui.label(
                    RichText::new(output.display().to_string())
                        .color(text_dark())
                        .monospace(),
                );
                ui.add_space(9.0);
                // Measured on the shipping install: 12,292 tags, 5.4 GB, about a
                // minute on an SSD. Stated as a range because a slow disk is
                // several times that, and a promise of "a minute" that turns
                // into ten is worse than no estimate at all. A folder is a
                // fraction of that and gets no figure at all rather than one
                // scaled from a total it has no fixed relation to.
                ui.label(
                    RichText::new(match &folder {
                        Some(_) => {
                            "Every container payload has to be read and decompressed, so this \
                             takes longer than the tag count suggests. Baboon stays usable while \
                             it runs, and you can cancel it from the status bar."
                        }
                        None => {
                            "Every container payload has to be read and decompressed. Expect a \
                             few gigabytes of files and anywhere from a minute to considerably \
                             longer, depending on the disk. Baboon stays usable while it runs, \
                             and you can cancel it from the status bar."
                        }
                    })
                    .color(egui::Color32::from_rgb(210, 120, 90)),
                );
                ui.add_space(5.0);
                ui.label(
                    RichText::new(match &folder {
                        // The folder's full path is recreated rather than
                        // flattened, so several folder extractions into one
                        // destination merge into a single loadable kit.
                        Some(label) => format!(
                            "Tags keep their full paths, so this lands under {label}/ inside the \
                             folder you pick and can be reopened with File \u{2192} Load Folder.",
                        ),
                        None => "Tags are laid out like an editing kit — levels/, objects/, \
                                 shaders/ — so the result can be reopened with File \
                                 \u{2192} Load Folder."
                            .to_owned(),
                    })
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(3.0);
                ui.label(
                    RichText::new(
                        "Mods mounted over the game are ignored: this extracts the copy the game \
                         ships, and tags that only a mod provides are skipped.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let accept = match &folder {
                        Some(_) => "Extract Tags",
                        None => "Extract All Tags",
                    };
                    if ui.button(accept).clicked() {
                        do_extract = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if !open || cancel {
            self.container_dump_confirm = None;
        } else if do_extract {
            // Taken rather than cleared: the scope captured at right-click is
            // what the run covers, and it moves into the job here.
            let scope = self
                .container_dump_confirm
                .take()
                .map(|confirm| confirm.scope);
            // The extraction reads the active kit's source, so return to the
            // workspace this was raised from and drop it if that workspace has
            // since closed.
            if let Some(scope) = scope {
                if self.focus_navigation_kit(kit) {
                    self.start_container_dump(kit, output, scope, ctx.clone());
                }
            }
        }
    }
}
