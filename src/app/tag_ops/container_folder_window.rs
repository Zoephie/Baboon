//! The New/Rename Folder window for a container source.
//! It owns presentation and the name entered; applying it is
//! [`TagOpsCommand::ApplyContainerFolder`].

use super::*;

/// New/Rename Folder for a container source.
///
/// Deliberately says so: nothing here touches a pak. A folder becomes real
/// in the container's directory index only once a tag is created, imported
/// or moved into it, and until then it lives in the workspace.
pub(in crate::app) fn draw_container_folder_window(cx: &Ctx, tag_ops: &mut TagOpsFeature) {
    let ctx = cx.egui;
    if tag_ops.container_folder_dialog.is_none() {
        return;
    }
    let mut open = true;
    let mut do_apply = false;
    let mut cancel = false;
    {
        let state = tag_ops
            .container_folder_dialog
            .as_mut()
            .expect("checked above");
        let renaming = state.renaming.is_some();
        let title = if renaming {
            "Rename Folder"
        } else {
            "New Folder"
        };
        egui::Window::new(title)
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::new("container_folder"))
            .open(&mut open)
            .default_width(window_width(ctx, 440.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new("Parent folder").color(subtle_dark()).small());
                let parent = match state.parent_rel.as_deref() {
                    Some(parent) if !parent.is_empty() => parent.to_owned(),
                    _ => "(root)".to_owned(),
                };
                ui.label(RichText::new(parent).color(text_dark()).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("Folder name").color(subtle_dark()).small());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut state.name_input)
                        .id(egui::Id::new("container_folder_name"))
                        .desired_width(400.0)
                        .font(egui::TextStyle::Monospace),
                );
                if state.focus_input {
                    response.request_focus();
                    state.focus_input = false;
                }
                if lost_focus_once(&response) && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    do_apply = true;
                }
                // Typing is the user's answer to a rejection, so the stale
                // message goes away rather than sitting under a fixed field.
                if response.changed() {
                    state.error = None;
                }

                if let Some(error) = state.error.as_deref() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(error).color(material_delete_text()).small());
                }

                ui.add_space(8.0);
                ui.label(
                    RichText::new(if renaming {
                        "This folder holds no tags, so renaming it changes nothing in the \
                         game's paks."
                    } else {
                        "Nothing is written to any pak. The folder appears in the container \
                         once a tag is created, imported or moved into it."
                    })
                    .color(subtle_dark())
                    .small(),
                );

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(if renaming { "Rename" } else { "Create Folder" })
                        .clicked()
                    {
                        do_apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
    }
    if do_apply {
        cx.send(TagOpsCommand::ApplyContainerFolder);
    } else if cancel || !open {
        tag_ops.container_folder_dialog = None;
    }
}
