//! The Rename Folder window for a loose tags folder.
//! It owns presentation and the name entered; the rename is
//! [`TagOpsCommand::ApplyLooseFolderRename`].

use super::*;
use super::single_rename::{SingleRename, draw_single_rename};
use crate::app::browser::bulk_rename::{RenameIcon, referrer_rows};

/// Rename Folder for a loose folder: the new name, and what it will change
/// counted before anything is touched.
impl Dialog for LooseFolderRenameState {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        let mut response = Default::default();
        let source = cx.model.kit_index(self.kit).and_then(|index| cx.model.kits[index].source.as_ref());
        let game = source.and_then(|source| source.source.game());
        // The folder's own tags get new paths: say so under the references.
        let mut notes = Vec::new();
        if self.tag_count > 0 {
            notes.push(crate::app::browser::bulk_rename::folder_tags_note(self.tag_count, "the renamed folder"));
        }
        let rows = match &self.outside_referrers {
            Some(paths) => source.map(|source| referrer_rows(paths, &source.names)).unwrap_or_default(),
            None => {
                notes.push("Reference index unavailable. Tags elsewhere that reference this folder's tags are still found and fixed when renaming.".to_owned());
                Vec::new()
            }
        };
        let state = &mut *self;
        egui::Window::new("Rename Folder")
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::unique("loose_folder_rename"))
            .title_bar(false)
            .collapsible(false)
            .frame(egui::Frame::window(&ctx.global_style()))
            .default_width(window_width(ctx, 480.0))
            .resizable(false)
            .show(ctx, |ui| {
                let before = state.name_input.clone();
                response = draw_single_rename(cx, ui, &mut open, SingleRename {
                    input: &mut state.name_input,
                    focus: &mut state.focus_input,
                    field_id: egui::Id::unique("loose_folder_rename_name"),
                    current: &state.old_name,
                    location: tag_path::join(&state.parent_display, &state.old_name),
                    extension: None,
                    icon: RenameIcon::Folder,
                    game,
                    noun: "Folder names",
                    error: state.error.as_deref(),
                    notes: None,
                    references: Some((rows, notes)),
                    apply_hover: "Rename the folder on disk and fix every reference",
                });
                if state.name_input != before {
                    state.error = None;
                }
            });
        // Left open for the apply, which closes it once it succeeds.
        if response.apply {
            cx.send(TagOpsCommand::ApplyLooseFolderRename);
        }
        open && !response.cancel
    }
}
