//! The confirmation for duplicating a Campaign Evolved container tag.
//! It owns presentation and the choice; the duplicate belongs to the controller.

use super::*;

impl Baboon {
    pub(in crate::app::ui) fn draw_container_duplicate_confirm_window(
        &mut self,
        ctx: &egui::Context,
    ) {
        let Some((kit, key, destination_leaf)) =
            self.container_duplicate_confirm.as_ref().map(|confirm| {
                (
                    confirm.kit,
                    confirm.key.clone(),
                    confirm.destination_leaf.clone(),
                )
            })
        else {
            return;
        };
        let mut open = true;
        let mut duplicate = false;
        let mut cancel = false;
        let (source_display, destination_display, target_label, target_kind, target_utoc) = self
            .resolve_kit(kit)
            .and_then(|index| {
                let entry = self.entry_for_key_in(index, &key)?;
                let (stem, extension) = entry
                    .display_path
                    .rsplit_once('.')
                    .map(|(stem, extension)| (stem, extension))
                    .unwrap_or((&entry.display_path, ""));
                let parent = stem
                    .rsplit_once('/')
                    .map(|(parent, _)| parent)
                    .unwrap_or("");
                let destination_display = if extension.is_empty() {
                    destination_leaf.clone()
                } else {
                    format!("{destination_leaf}.{extension}")
                };
                let destination_display = if parent.is_empty() {
                    destination_display
                } else {
                    format!("{parent}/{destination_display}")
                };
                let (label, is_mod) = self.container_label_for_tag(index, &key)?;
                let utoc = match &entry.location {
                    TagEntryLocation::Container { container, .. } => self.kits[index]
                        .source
                        .as_ref()
                        .and_then(|source| match &source.source {
                            TagSource::IoStoreContainerSet { containers, .. } => {
                                containers.get(*container)
                            }
                            _ => None,
                        })
                        .map(|container| container.utoc_path.display().to_string())?,
                    _ => return None,
                };
                Some((
                    entry.display_path.clone(),
                    destination_display,
                    label,
                    if is_mod { "mounted mod" } else { "shipped pak" },
                    utoc,
                ))
            })
            .unwrap_or_else(|| {
                (
                    key.clone(),
                    destination_leaf.clone(),
                    "unknown container".to_owned(),
                    "container",
                    "unknown UTOC".to_owned(),
                )
            });
        egui::Window::new("Duplicate in Campaign Evolved container?")
            .id(egui::Id::new("container_duplicate_confirm"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "Duplicate {source_display} as {destination_display}"
                    ))
                    .color(text_dark()),
                );
                ui.add_space(7.0);
                ui.label(
                    RichText::new(format!(
                        "Exact target: {target_kind} {target_label} ({target_utoc})"
                    ))
                    .color(text_dark())
                    .monospace(),
                );
                ui.add_space(7.0);
                ui.label(
                    RichText::new(
                        "This changes the target UTOC and UCAS. The sibling PAK will not be \
                         changed. An immutable UTOC backup and manifest are created immediately \
                         before the duplicate.",
                    )
                    .color(subtle_dark()),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Duplicate").clicked() {
                        duplicate = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if duplicate {
            self.container_duplicate_confirm = None;
            self.start_container_duplicate(kit, key, destination_leaf, ctx.clone());
        } else if cancel || !open {
            self.container_duplicate_confirm = None;
        }
    }
}
