//! The New Folder window for loose kits and containers, and pending-folder rename.
//! It owns presentation and the name entered; applying it is
//! [`TagOpsCommand::ApplyContainerFolder`].

use super::*;
use crate::app::editor::{picker_search_field, picker_results, COMBO_POPUP_MAX_HEIGHT};

impl ContainerFolderDialog {
    pub(in crate::app) fn select_parent_directory(&mut self, root: &Path, picked: &Path) -> Result<(), String> {
        let root = fs::canonicalize(root).map_err(|error| error.to_string())?;
        let picked = fs::canonicalize(picked).map_err(|error| error.to_string())?;
        if !picked.is_dir() { return Err("Choose a parent folder".to_owned()); }
        let relative = picked.strip_prefix(&root)
            .map_err(|_| "Choose a parent inside this editing kit's tags directory".to_owned())?;
        let parent = tag_path::relative_tag_path(relative);
        self.parent_rel = (!parent.is_empty()).then_some(parent);
        self.error = None;
        Ok(())
    }
}

fn parent_folder_picker(ui: &mut Ui, state: &mut ContainerFolderDialog, model: &Model) {
    let kit_index = model.kit_index(state.kit);
    let root = kit_index.and_then(|index| model.loaded_tags_root_for(index));
    let popup_id = ui.make_persistent_id("new_folder_parent");
    let response = icon_text_button(ui, ButtonIcon::Browse, "Browse", state.renaming.is_none());
    let just_opened = response.clicked() && !egui::Popup::is_id_open(ui.ctx(), popup_id);
    if response.clicked() {
        if let Some(root) = root {
            let directory = root.join(tag_path::to_relative(state.parent_rel.as_deref().unwrap_or_default()));
            if let Some(picked) = rfd::FileDialog::new().set_title("Select Parent Folder")
                .set_directory(directory).pick_folder()
                && let Err(error) = state.select_parent_directory(&root, &picked)
            {
                state.error = Some(error);
            }
        } else {
            egui::Popup::toggle_id(ui.ctx(), popup_id);
        }
    }
    egui::Popup::from_response(&response).id(popup_id).open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_width(340.0);
        let id = popup_id.with("search");
        let mut query = ui.data(|data| data.get_temp::<String>(id)).unwrap_or_default();
        let changed = picker_search_field(ui, &mut query, "Search folders…", just_opened);
        ui.data_mut(|data| data.insert_temp(id, query.clone()));
        let mut folders = std::collections::BTreeSet::new();
        folders.insert(String::new());
        if let Some(kit) = kit_index.map(|index| &model.kits[index]) {
            if let Some(source) = &kit.source {
                for entry in source.full_entry_set() {
                    let mut parent = tag_path::parent(&entry.display_path);
                    while let Some(path) = parent {
                        folders.insert(path.to_owned());
                        parent = tag_path::parent(path);
                    }
                }
            }
            for folder in &kit.pending_container_folders { folders.insert(folder.clone()); }
        }
        picker_results(ui, COMBO_POPUP_MAX_HEIGHT, changed, |ui| {
            for folder in folders {
                let label = if folder.is_empty() { "(root)" } else { &folder };
                if !label.to_lowercase().contains(&query.trim().to_lowercase()) { continue; }
                if ui.selectable_label(state.parent_rel.as_deref().unwrap_or_default() == folder, label).clicked() {
                    state.parent_rel = (!folder.is_empty()).then_some(folder);
                    state.error = None;
                    egui::Popup::close_id(ui.ctx(), popup_id);
                }
            }
        });
    });
}

/// New Folder creates a directory for loose kits or a pending workspace folder
/// for containers. Rename applies only to pending container folders.
impl Dialog for ContainerFolderDialog {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        let mut do_apply = false;
        let mut cancel = false;
        {
            let state = &mut *self;
            let renaming = state.renaming.is_some();
            let title = if renaming {
                "Rename Folder"
            } else {
                "New Folder"
            };
            egui::Window::new(title)
                .constrain_to(window_work_area(ctx))
                .id(egui::Id::unique("container_folder"))
                .collapsible(false)
                .title_bar(false)
                .frame(egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin {
                    left: 20, right: 20, top: 0, bottom: 0,
                }))
                .default_width(window_width(ctx, 440.0))
                .resizable(false)
                .show(ctx, |ui| {
                    ui.spacing_mut().window_margin = egui::Margin { left: 20, right: 20, top: 0, bottom: 0 };
                    crate::app::search::draw_icon_window_header(ui, title,
                        if renaming { ButtonIcon::Rename } else { ButtonIcon::Add }, &mut open);
                    ui.add_space(10.0);
                    ui.label(RichText::new("Parent Folder").strong());
                    let parent = match state.parent_rel.as_deref() {
                        Some(parent) if !parent.is_empty() => parent.to_owned(),
                        _ => "(root)".to_owned(),
                    };
                    ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), BUTTON_HEIGHT),
                        egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        parent_folder_picker(ui, state, cx.model);
                        let mut parent = parent;
                        ui.add_sized(Vec2::new(ui.available_width(), BUTTON_HEIGHT),
                            crate::app::ui_kit::singleline_text_edit(&mut parent).interactive(false)
                                .vertical_align(egui::Align::Center));
                    });
                    ui.add_space(6.0);

                    ui.label(RichText::new("Folder Name").strong());
                    let response = ui.add(
                        crate::app::ui_kit::singleline_text_edit(&mut state.name_input)
                            .id(egui::Id::unique("container_folder_name"))
                            .desired_width(ui.available_width())
                            .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
                            .vertical_align(egui::Align::Center)
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
                        RichText::new(if cx.model.loaded_tags_root_for(cx.model.kit_index(state.kit).unwrap_or(cx.model.active)).is_some() {
                            "Creates a folder inside this editing kit's tags directory."
                        } else if renaming {
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
                    ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 40.0),
                        egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let background = ui.max_rect().expand2(Vec2::new(20.0, 0.0));
                        let mut painter = ui.painter().clone();
                        painter.set_clip_rect(background.intersect(ctx.content_rect()));
                        let mut rounding = ui.visuals().window_corner_radius;
                        rounding.nw = 0;
                        rounding.ne = 0;
                        painter.rect_filled(background, rounding, foundation_block_bar());
                        if icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked() { cancel = true; }
                        if icon_text_button(ui, if renaming { ButtonIcon::Rename } else { ButtonIcon::Add },
                            if renaming { "Rename" } else { "Create Folder" }, !state.name_input.trim().is_empty()).clicked()
                        {
                            do_apply = true;
                        }
                    });
                });
        }
        // Left open for the apply, which closes it once it succeeds.
        if do_apply {
            cx.send(TagOpsCommand::ApplyContainerFolder);
        }
        open && !cancel
    }
}
