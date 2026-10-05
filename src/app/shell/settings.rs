//! Preferences window and its settings tabs.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;
use crate::app::shell::frame::draw_kit_banner_tile;
use crate::app::shell::frame::recheck_cached;
use crate::app::shell::frame::editing_kit_title_text;

#[derive(Clone)]
struct EditingKitDrag(String);

#[derive(Clone)]
pub(in crate::app) struct EditingKitReorderRequest {
    source: String,
    target: String,
    after: bool,
}

fn reorder_editing_kit_profiles(
    profiles: &mut Vec<CustomEditingKitProfile>,
    request: &EditingKitReorderRequest,
) -> bool {
    if request.source == request.target {
        return false;
    }
    let Some(from) = profiles
        .iter()
        .position(|profile| profile.id == request.source)
    else {
        return false;
    };
    let Some(target) = profiles
        .iter()
        .position(|profile| profile.id == request.target)
    else {
        return false;
    };
    let insertion = target + usize::from(request.after);
    let destination = if from < insertion {
        insertion - 1
    } else {
        insertion
    };
    if from == destination {
        return false;
    }
    let profile = profiles.remove(from);
    profiles.insert(destination, profile);
    true
}

/// Shared, compact presentation for built-in and user-created editing kits.
#[cfg(test)]
fn editing_kit_card(
    ui: &mut Ui,
    name: &str,
    path: &Path,
    texture: Option<&egui::TextureHandle>,
    error: Option<&str>,
    icon_warning: Option<&str>,
    reorder_id: Option<&str>,
) -> (bool, bool, bool) {
    editing_kit_card_with_read_only(
        ui,
        name,
        path,
        texture,
        error,
        icon_warning,
        reorder_id,
        false,
    )
}

fn editing_kit_card_with_read_only(
    ui: &mut Ui,
    name: &str,
    path: &Path,
    texture: Option<&egui::TextureHandle>,
    error: Option<&str>,
    icon_warning: Option<&str>,
    reorder_id: Option<&str>,
    read_only: bool,
) -> (bool, bool, bool) {
    let invalid = error.is_some();
    let fill = if invalid {
        if is_dark_mode() {
            Color32::from_rgb(98, 40, 36)
        } else {
            Color32::from_rgb(255, 222, 216)
        }
    } else {
        foundation_group_bg()
    };
    let mut load = false;
    let mut edit = false;
    let mut remove = false;
    let card = Frame::NONE
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Center within a fixed row, not the remaining scroll area's height.
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 40.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    if let Some(id) = reorder_id {
                        let (rect, grabber) =
                            ui.allocate_exact_size(Vec2::new(14.0, 40.0), Sense::drag());
                        let grabber = grabber.on_hover_text("Drag to reorder editing kit");
                        grabber.dnd_set_drag_payload(EditingKitDrag(id.to_owned()));
                        if grabber.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        }
                        for x in [-2.5, 2.5] {
                            for y in [-6.0, 0.0, 6.0] {
                                ui.painter().circle_filled(
                                    rect.center() + Vec2::new(x, y),
                                    1.2,
                                    subtle_dark(),
                                );
                            }
                        }
                    }
                    let image_size = Vec2::splat(40.0);
                    if let Some(texture) = texture {
                        ui.add(
                            egui::Image::new(texture)
                                .fit_to_exact_size(image_size)
                                .corner_radius(egui::CornerRadius::same(4)),
                        );
                    } else {
                        ui.add(
                            button_icon_image(ui, ButtonIcon::FolderOpen, text_dark(), 40.0)
                                .fit_to_exact_size(image_size),
                        );
                    }
                    ui.add_space(5.0);
                    let action_width = editing_kit_action_width(ui, "Open")
                        + editing_kit_action_width(ui, "Edit")
                        + ICON_BUTTON_SIZE.x
                        + ui.spacing().item_spacing.x * 3.0;
                    let text_width = (ui.available_width() - action_width).max(0.0);
                    // Reserve the whole text column even when its labels are short.
                    let (text_rect, _) =
                        ui.allocate_exact_size(Vec2::new(text_width, 40.0), Sense::hover());
                    {
                        let mut text_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(text_rect)
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        let ui = &mut text_ui;
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add_space(2.0);
                        ui.add(
                            egui::Label::new(editing_kit_title_text(
                                ui, name, read_only, 14.0, true,
                            ))
                            .truncate(),
                        )
                        .on_hover_text(name);
                        ui.add(
                            egui::Label::new(
                                RichText::new(path.display().to_string()).size(11.0).color(
                                    if invalid {
                                        material_delete_text()
                                    } else {
                                        subtle_dark()
                                    },
                                ),
                            )
                            .truncate(),
                        )
                        .on_hover_text(path.display().to_string());
                    }
                    let open = icon_text_button(ui, ButtonIcon::Open, "Open", !invalid);
                    load = if let Some(error) = error {
                        open.on_disabled_hover_text(error)
                    } else {
                        open.on_hover_text("Open editing kit")
                    }
                    .clicked();
                    edit = icon_text_button(ui, ButtonIcon::Edit, "Edit", true)
                        .on_hover_text("Edit editing kit")
                        .clicked();
                    remove = icon_button(
                        ui,
                        ButtonIcon::Clear,
                        "Remove editing kit",
                        true,
                        text_dark(),
                    )
                    .clicked();
                },
            );
        });
    if let Some(target) = reorder_id {
        if let Some(drag) = card.response.dnd_hover_payload::<EditingKitDrag>() {
            if drag.0 != target {
                let after = ui
                    .input(|input| input.pointer.hover_pos())
                    .is_some_and(|pos| pos.y > card.response.rect.center().y);
                let y = if after {
                    card.response.rect.bottom()
                } else {
                    card.response.rect.top()
                };
                ui.painter().hline(
                    card.response.rect.x_range(),
                    y,
                    Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
                );
            }
        }
        if let Some(drag) = card.response.dnd_release_payload::<EditingKitDrag>() {
            let after = ui
                .input(|input| input.pointer.hover_pos())
                .is_some_and(|pos| pos.y > card.response.rect.center().y);
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("editing_kit_reorder_request"),
                    EditingKitReorderRequest {
                        source: drag.0.clone(),
                        target: target.to_owned(),
                        after,
                    },
                )
            });
        }
    }
    if let Some(error) = error {
        card.response.on_hover_text(error);
    } else if let Some(warning) = icon_warning {
        card.response.on_hover_text(warning);
    }
    ui.add_space(4.0);
    (load, edit, remove)
}

/// Reserve only the width the shared icon-and-text button actually needs.
fn editing_kit_action_width(ui: &Ui, label: &str) -> f32 {
    let font = TextStyle::Button.resolve(ui.style());
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, text_dark());
    text.size().x
        + BUTTON_ICON_SIZE
        + ui.spacing().icon_spacing
        + 2.0 * ui.spacing().button_padding.x
}

#[derive(Default)]
struct EditingKitFormActions {
    save: bool,
    cancel: bool,
    remove: bool,
}

#[derive(Clone)]
struct DraftIconTexture {
    path: PathBuf,
    texture: Option<egui::TextureHandle>,
}

fn draft_icon_path(ctx: &egui::Context, icon: &CustomEditingKitIconDraft) -> Option<PathBuf> {
    match icon {
        CustomEditingKitIconDraft::Default => None,
        // Resolving looks for the file in two places; the form asks every frame.
        CustomEditingKitIconDraft::Existing(path) => {
            recheck_cached(ctx, ("kit_icon", path), || {
                resolve_custom_icon_path(path).ok()
            })
        }
        CustomEditingKitIconDraft::Selected(path) => Some(path.clone()),
    }
}

fn draft_editing_kit_icon_texture(
    ctx: &egui::Context,
    icon: &CustomEditingKitIconDraft,
) -> Option<egui::TextureHandle> {
    let key = egui::Id::new("editing_kit_draft_icon_texture");
    let Some(path) = draft_icon_path(ctx, icon) else {
        ctx.data_mut(|data| data.remove::<DraftIconTexture>(key));
        return None;
    };
    if let Some(cached) = ctx.data(|data| data.get_temp::<DraftIconTexture>(key))
        && cached.path == path
    {
        return cached.texture;
    }
    let texture = fs::read(&path)
        .ok()
        .and_then(|bytes| load_png_texture(ctx, "editing_kit_draft_icon", &bytes));
    ctx.data_mut(|data| {
        data.insert_temp(
            key,
            DraftIconTexture {
                path,
                texture: texture.clone(),
            },
        )
    });
    texture
}

fn editing_kit_field_label(ui: &mut Ui, label: &str) {
    ui.label(RichText::new(label).strong().color(text_dark()));
}

fn editing_kit_text_input(value: &mut String, width: f32) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(value)
        .desired_width(width)
        .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
        .vertical_align(egui::Align::Center)
}

fn draw_editing_kit_form(
    ui: &mut Ui,
    draft: &mut CustomEditingKitDraft,
    texture: Option<&egui::TextureHandle>,
) -> EditingKitFormActions {
    let mut actions = EditingKitFormActions::default();
    let title = if draft.name.trim().is_empty() {
        "Editing Kit"
    } else {
        draft.name.trim()
    };
    draw_kit_banner_tile(
        ui,
        title,
        &draft.root_input,
        texture,
        draft.read_only && !draft.is_campaign_evolved(),
    );
    ui.add_space(12.0);
    ui.columns(2, |columns| {
        editing_kit_field_label(&mut columns[0], "Name");
        let width = columns[0].available_width();
        if columns[0]
            .add(
                editing_kit_text_input(&mut draft.name, width)
                    .hint_text(placeholder_text("Editing kit name")),
            )
            .changed()
        {
            ui_repaint_for_kit_draft(&columns[0]);
        }
        editing_kit_field_label(&mut columns[1], "Engine");
        egui::ComboBox::from_id_salt("editing_kit_form_engine")
            .selected_text(game_display_name(&draft.game))
            .width(columns[1].available_width())
            .show_ui(&mut columns[1], |ui| {
                for game in GameId::ALL {
                    if ui
                        .selectable_value(&mut draft.game, game.as_str().to_owned(), game.display_name())
                        .changed()
                    {
                        refill_kit_folders(draft);
                        ui_repaint_for_kit_draft(ui);
                    }
                }
            });
    });
    ui.add_space(8.0);
    editing_kit_field_label(ui, "Editing Kit Root Folder");
    ui.horizontal(|ui| {
        let width = (ui.available_width()
            - editing_kit_action_width(ui, "Browse")
            - editing_kit_action_width(ui, "Clear")
            - ui.spacing().item_spacing.x * 2.0
            - 8.0)
            .max(40.0);
        if ui
            .add(
                editing_kit_text_input(&mut draft.root_input, width)
                    .hint_text(placeholder_text("Kit root, tags folder, or game install")),
            )
            .changed()
        {
            draft.error = None;
            refill_kit_folders(draft);
            ui_repaint_for_kit_draft(ui);
        }
        if icon_text_button(ui, ButtonIcon::Browse, "Browse", true).clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Select Editing Kit Root")
                .pick_folder()
        {
            draft.root_input = path.display().to_string();
            draft.error = None;
            refill_kit_folders(draft);
            ui_repaint_for_kit_draft(ui);
        }
        if icon_text_button(ui, ButtonIcon::Clear, "Clear", true).clicked() {
            draft.root_input.clear();
            draft.error = None;
            refill_kit_folders(draft);
            ui_repaint_for_kit_draft(ui);
        }
    });
    if kit_folders_are_choosable(&draft.game) {
        draw_kit_folder_input(ui, draft, KitFolder::Tags);
        draw_kit_folder_input(ui, draft, KitFolder::Data);
        ui.label(
            RichText::new(
                "Leave a folder empty to use the root's own. Its tools are pointed at \
                 other folders with -tags_dir and -data_dir.",
            )
            .small()
            .color(subtle_dark()),
        );
    }
    ui.add_space(8.0);
    editing_kit_field_label(ui, "Custom Icon (.png)");
    ui.horizontal(|ui| {
        let mut display = draft_icon_path(ui.ctx(), &draft.icon)
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let width = (ui.available_width()
            - editing_kit_action_width(ui, "Browse")
            - editing_kit_action_width(ui, "Clear")
            - ui.spacing().item_spacing.x * 2.0
            - 8.0)
            .max(40.0);
        ui.add(
            editing_kit_text_input(&mut display, width)
                .interactive(false)
                .hint_text(placeholder_text("Default engine artwork")),
        );
        if icon_text_button(ui, ButtonIcon::Browse, "Browse", true).clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Select Editing Kit Icon")
                .add_filter("PNG image", &["png"])
                .pick_file()
        {
            match validate_custom_icon_source(&path) {
                Ok((width, height)) => {
                    draft.icon = CustomEditingKitIconDraft::Selected(path);
                    draft.icon_warning = (width != RECOMMENDED_CUSTOM_ICON_SIZE
                        || height != RECOMMENDED_CUSTOM_ICON_SIZE)
                        .then(|| {
                            format!("This image is {width} × {height}; 200 × 200 is recommended.")
                        });
                    draft.error = None;
                    ui.ctx().data_mut(|data| {
                        data.remove::<DraftIconTexture>(egui::Id::new(
                            "editing_kit_draft_icon_texture",
                        ))
                    });
                    ui_repaint_for_kit_draft(ui);
                }
                Err(error) => draft.error = Some(error),
            }
        }
        if icon_text_button(ui, ButtonIcon::Clear, "Clear", true).clicked() {
            draft.icon = CustomEditingKitIconDraft::Default;
            draft.icon_warning = None;
            draft.error = None;
            ui_repaint_for_kit_draft(ui);
        }
    });
    ui.label(
        RichText::new("Optional. Clear to use engine artwork. A 200 × 200 PNG is recommended.")
            .small()
            .color(subtle_dark()),
    );
    if !draft.is_campaign_evolved() {
        ui.add_space(8.0);
        editing_kit_field_label(ui, "Options");
        ui.checkbox(&mut draft.read_only, "Read-Only");
        ui.indent("editing_kit_read_only_help", |ui| {
            ui.label(
                RichText::new("Make this editing kit read-only within Baboon.")
                    .small()
                    .color(subtle_dark()),
            );
            ui.label(
                RichText::new("This doesn’t prevent this kit from being edited in other tools.")
                    .small()
                    .italics()
                    .color(subtle_dark()),
            );
        });
        ui.add_space(8.0);
        ui.checkbox(&mut draft.git_tracked, "Tracked in Git");
        ui.indent("editing_kit_git_help", |ui| {
            ui.label(
                RichText::new(
                    "Compare tags with their version in this kit’s current Git commit (HEAD).",
                )
                .small()
                .color(subtle_dark()),
            );
        });
    }
    if let Some(warning) = &draft.icon_warning {
        ui.label(
            RichText::new(warning)
                .small()
                .color(Color32::from_rgb(220, 170, 70)),
        );
    }
    if let Some(error) = &draft.error {
        ui.label(RichText::new(error).color(material_delete_text()));
    }
    ui.add_space(8.0);
    ui.separator();
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), BUTTON_HEIGHT),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            if draft.editing_id.is_some() {
                actions.remove = ui.button("Remove Editing Kit").clicked();
            }
            actions.cancel = ui.button("Cancel").clicked();
            actions.save = ui.button("Save").clicked();
        },
    );
    actions
}

fn ui_repaint_for_kit_draft(ui: &Ui) {
    ui.ctx().request_repaint();
}

#[derive(Clone, Copy)]
enum KitFolder {
    Tags,
    Data,
}

impl KitFolder {
    fn name(self) -> &'static str {
        match self {
            Self::Tags => "tags",
            Self::Data => "data",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Tags => "Tags Folder",
            Self::Data => "Data Folder",
        }
    }

    fn input(self, draft: &mut CustomEditingKitDraft) -> (&mut String, &mut bool) {
        match self {
            Self::Tags => (&mut draft.tags_folder_input, &mut draft.tags_folder_auto),
            Self::Data => (&mut draft.data_folder_input, &mut draft.data_folder_auto),
        }
    }
}

/// Fill the folder inputs the user hasn't chosen with the root's own `tags`
/// and `data` folders, after the root or engine changes.
fn refill_kit_folders(draft: &mut CustomEditingKitDraft) {
    if !kit_folders_are_choosable(&draft.game) {
        return;
    }
    let root = PathBuf::from(draft.root_input.trim());
    for folder in [KitFolder::Tags, KitFolder::Data] {
        let (input, auto) = folder.input(draft);
        if *auto {
            *input = default_kit_folder_name(&root, folder.name()).unwrap_or_default();
        }
    }
}

/// One folder row: the path (relative to the root, or absolute), Browse,
/// Clear, and the root's folders with matching names as quick picks.
fn draw_kit_folder_input(ui: &mut Ui, draft: &mut CustomEditingKitDraft, folder: KitFolder) {
    let root = PathBuf::from(draft.root_input.trim());
    ui.add_space(8.0);
    editing_kit_field_label(ui, folder.label());
    ui.horizontal(|ui| {
        let width = (ui.available_width()
            - editing_kit_action_width(ui, "Browse")
            - editing_kit_action_width(ui, "Clear")
            - ui.spacing().item_spacing.x * 2.0
            - 8.0)
            .max(40.0);
        let hint = format!("{} (the root's own)", folder.name());
        let (input, auto) = folder.input(draft);
        if ui
            .add(editing_kit_text_input(input, width).hint_text(placeholder_text(&hint)))
            .changed()
        {
            *auto = input.trim().is_empty();
            ui_repaint_for_kit_draft(ui);
        }
        if icon_text_button(ui, ButtonIcon::Browse, "Browse", true).clicked() {
            let mut dialog = rfd::FileDialog::new().set_title(format!("Select {}", folder.label()));
            if root.is_dir() {
                dialog = dialog.set_directory(&root);
            }
            if let Some(path) = dialog.pick_folder() {
                *input = path
                    .strip_prefix(&root)
                    .map(Path::to_path_buf)
                    .unwrap_or(path)
                    .display()
                    .to_string();
                *auto = false;
                ui_repaint_for_kit_draft(ui);
            }
        }
        if icon_text_button(ui, ButtonIcon::Clear, "Clear", true).clicked() {
            input.clear();
            *auto = true;
            ui_repaint_for_kit_draft(ui);
        }
    });
    // Listing the root on every frame would read the disk 60 times a second,
    // so the choices are kept per root and folder kind.
    let cache_id = egui::Id::new(("editing_kit_folder_candidates", folder.name(), &root));
    let candidates = ui
        .ctx()
        .data(|data| data.get_temp::<Arc<Vec<String>>>(cache_id))
        .unwrap_or_else(|| {
            let candidates = Arc::new(kit_folder_candidates(&root, folder.name()));
            ui.ctx()
                .data_mut(|data| data.insert_temp(cache_id, Arc::clone(&candidates)));
            candidates
        });
    if candidates.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            let (input, auto) = folder.input(draft);
            for name in candidates.iter() {
                let selected = input.trim().eq_ignore_ascii_case(name);
                if ui.selectable_label(selected, name).clicked() {
                    *input = name.clone();
                    *auto = false;
                    ui_repaint_for_kit_draft(ui);
                }
            }
        });
    }
}

fn settings_window_body(
    ui: &mut Ui,
    open: &mut bool,
    selected: &mut SettingsTab,
    content: impl FnOnce(&mut Ui, SettingsTab),
) {
    // Establish the requested height before drawing. Calling set_min_height
    // after drawing adds that height at the current cursor, doubling the body.
    ui.set_min_height(ui.available_height());
    crate::app::search::draw_icon_window_header(ui, "Settings", ButtonIcon::Settings, open);
    ui.separator();
    ScrollArea::horizontal()
        .id_salt("settings_tabs_scroll")
        .max_height(BUTTON_ICON_SIZE + 20.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (tab, label) in [
                    (SettingsTab::Startup, "Startup"),
                    (SettingsTab::Browser, "Browser"),
                    (SettingsTab::EditingKits, "Editing Kits"),
                    (SettingsTab::Appearance, "Appearance"),
                    (SettingsTab::Tools, "Tools"),
                ] {
                    if view_text_tab_button(ui, label, *selected == tab).clicked() {
                        *selected = tab;
                    }
                }
            });
        });
    ui.add_space(8.0);
    // Leave room for the layout's trailing item spacing. Otherwise a
    // height-filling ScrollArea feeds a slightly larger content size back to
    // egui's Resize each frame, including during horizontal edge drags.
    let height = (ui.available_height() - ui.spacing().item_spacing.y).max(0.0);
    ScrollArea::vertical()
        .id_salt("settings_content_scroll")
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .max_height(height)
        .show(ui, |ui| {
            Frame::NONE
                .inner_margin(ui.spacing().window_margin)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    content(ui, *selected);
                });
        });
}

impl Baboon {
    pub(in crate::app) fn set_editing_kit_path_input(
        &mut self,
        shortcut: EditingKitShortcut,
        input: String,
    ) {
        let trimmed = input.trim().to_owned();
        if trimmed.is_empty() {
            self.model.prefs.editing_kit_paths.remove(shortcut.game.as_str());
        } else {
            self.model.prefs
                .editing_kit_paths
                .insert(shortcut.game.as_str().to_owned(), PathBuf::from(&trimmed));
        }
        self.kit_tools.editing_kit_path_inputs
            .insert(shortcut.game.as_str().to_owned(), input);
        if self.kit_tools.editing_kit_path_attention.as_deref() == Some(shortcut.game.as_str()) && !trimmed.is_empty()
        {
            self.kit_tools.editing_kit_path_attention = None;
        }
        self.refresh_builtin_editing_kit_validation(shortcut);
    }

    fn commit_custom_editing_kit_draft(&mut self, draft: &mut CustomEditingKitDraft) -> bool {
        let name = draft.name.trim().to_owned();
        if name.is_empty() {
            draft.error = Some("Enter an editing kit name".to_owned());
            return false;
        }
        let Some(game) = game_for_saved_id(&draft.game) else {
            draft.error = Some("Choose a supported editing-kit engine".to_owned());
            return false;
        };
        let root_input = PathBuf::from(draft.root_input.trim());
        let choosable = game.tools_take_folder_arguments();
        let folder_input = |input: &str| {
            Some(input.trim())
                .filter(|input| choosable && !input.is_empty())
                .map(PathBuf::from)
        };
        let tags_input = folder_input(&draft.tags_folder_input);
        let data_input = folder_input(&draft.data_folder_input);
        let layout = match validate_kit_layout(
            &root_input,
            game.as_str(),
            tags_input.as_deref(),
            data_input.as_deref(),
        ) {
            Ok(layout) => layout,
            Err(error) => {
                draft.error = Some(error);
                return false;
            }
        };
        if custom_profile_tags_conflicts(
            &self.model.prefs.custom_editing_kit_profiles,
            draft.editing_id.as_deref(),
            &layout.tags,
        ) {
            draft.error = Some("Another editing kit already uses this tags folder".to_owned());
            return false;
        }
        let tags_folder = choosable
            .then(|| folder_to_store(&layout.root, Some(&layout.tags), "tags"))
            .flatten();
        let data_folder = choosable
            .then(|| folder_to_store(&layout.root, layout.data.as_deref(), "data"))
            .flatten();

        let id = draft
            .editing_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let existing_profile = self
            .model.prefs
            .custom_editing_kit_profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned();
        let storage_name = existing_profile
            .as_ref()
            .map(|profile| profile.name.as_str())
            .unwrap_or(&name);
        let icon = match &draft.icon {
            CustomEditingKitIconDraft::Default => None,
            CustomEditingKitIconDraft::Existing(path) => Some(path.clone()),
            CustomEditingKitIconDraft::Selected(path) => {
                match copy_custom_icon(
                    path,
                    storage_name,
                    &id,
                    existing_profile
                        .as_ref()
                        .and_then(|profile| profile.icon.as_deref()),
                ) {
                    Ok(relative) => Some(relative),
                    Err(error) => {
                        draft.error = Some(error);
                        return false;
                    }
                }
            }
        };
        let profile = CustomEditingKitProfile {
            read_only: draft.read_only && !game.is_campaign_evolved(),
            git_tracked: draft.git_tracked && !game.is_campaign_evolved(),
            id: id.clone(),
            name: name.clone(),
            game: game.as_str().to_owned(),
            root: layout.root,
            icon,
            tags_folder,
            data_folder,
        };
        let previous_profiles = self.model.prefs.custom_editing_kit_profiles.clone();
        let previous = existing_profile;
        if let Some(index) = self
            .model.prefs
            .custom_editing_kit_profiles
            .iter()
            .position(|existing| existing.id == id)
        {
            self.model.prefs.custom_editing_kit_profiles[index] = profile.clone();
        } else {
            self.model.prefs.custom_editing_kit_profiles.push(profile.clone());
        }
        let prefs = self.current_prefs();
        if let Err(error) = save_gui_prefs(&prefs, &self.kit_tools.terminal_open_games, true) {
            self.model.prefs.custom_editing_kit_profiles = previous_profiles;
            if previous.as_ref().and_then(|profile| profile.icon.as_ref()) != profile.icon.as_ref()
                && let Some(icon) = &profile.icon
            {
                let _ =
                    remove_unreferenced_custom_icon(icon, &self.model.prefs.custom_editing_kit_profiles);
            }
            draft.error = Some(error);
            return false;
        }
        self.saved_prefs = prefs;
        self.kit_tools.saved_terminal_open_games = self.kit_tools.terminal_open_games.clone();
        self.shell.artwork.forget_custom_editing_kit(&id);
        self.refresh_editing_kit_validation();

        if let Some(previous) = previous {
            let source_changed = previous.game != profile.game
                || !same_recent_path(&previous.root, &profile.root)
                || previous.tags_folder != profile.tags_folder
                || previous.data_folder != profile.data_folder;
            for kit in &mut self.model.kits {
                if kit
                    .profile
                    .as_ref()
                    .is_some_and(|identity| identity.id == id)
                {
                    if source_changed {
                        kit.profile = None;
                    } else if let Some(identity) = &mut kit.profile {
                        identity.name = profile.name.clone();
                    }
                }
            }
            if previous.icon != profile.icon
                && let Some(old_icon) = previous.icon
                && let Err(error) = remove_unreferenced_custom_icon(
                    &old_icon,
                    &self.model.prefs.custom_editing_kit_profiles,
                )
            {
                self.model.status = error;
                return true;
            }
        }
        self.model.status = format!("Saved editing kit {}", profile.name);
        true
    }

    fn remove_custom_editing_kit_profile(&mut self, removal: &CustomEditingKitRemoval) {
        let previous_profiles = self.model.prefs.custom_editing_kit_profiles.clone();
        let removed = self
            .model.prefs
            .custom_editing_kit_profiles
            .iter()
            .find(|profile| profile.id == removal.id)
            .cloned();
        self.model.prefs
            .custom_editing_kit_profiles
            .retain(|profile| profile.id != removal.id);
        let prefs = self.current_prefs();
        if let Err(error) = save_gui_prefs(&prefs, &self.kit_tools.terminal_open_games, true) {
            self.model.prefs.custom_editing_kit_profiles = previous_profiles;
            self.model.status = error;
            return;
        }
        self.saved_prefs = prefs;
        self.kit_tools.saved_terminal_open_games = self.kit_tools.terminal_open_games.clone();
        self.shell.artwork.forget_custom_editing_kit(&removal.id);
        self.refresh_editing_kit_validation();
        for kit in &mut self.model.kits {
            if kit
                .profile
                .as_ref()
                .is_some_and(|profile| profile.id == removal.id)
            {
                kit.profile = None;
            }
        }
        if let Some(icon) = removed.and_then(|profile| profile.icon)
            && let Err(error) =
                remove_unreferenced_custom_icon(&icon, &self.model.prefs.custom_editing_kit_profiles)
        {
            self.model.status = error;
            return;
        }
        self.model.status = format!("Removed editing kit {}", removal.name);
    }
}

/// What Settings draws on: a draft of the preferences, the state it owns
/// (the shell's, the kits feature's and Chimp's usmap path being typed), and
/// what it asked for, held until the draft has been sent.
struct SettingsDraw<'a> {
    prefs: GuiPrefs,
    window: &'a mut SettingsWindow,
    app: &'a AppReads<'a>,
    /// Dialogs Settings opens over itself: the editing-kit editor and its
    /// removal confirmation.
    opened: Vec<Box<dyn Dialog>>,
    effects: Vec<SettingsCommand>,
}

/// What Settings asks for beyond the preferences it edits.
pub(in crate::app) enum SettingsCommand {
    /// Chimp was turned on: mount it in every Campaign Evolved workspace.
    MountChimpEverywhere,
    /// Chimp was turned off: every workspace back to its tags, Chimp's state
    /// dropped.
    DropChimpEverywhere,
    CheckForUpdates,
    /// The update channel changed: forget the last check's verdict.
    ForgetUpdateCheck,
    AutoDetectEditingKits,
    RefreshEditingKitStatus,
    LoadEditingKit(CustomEditingKitProfile),
    ReorderEditingKits(EditingKitReorderRequest),
    /// Save the editing kit the dialog describes; one the commit refuses
    /// goes back to the dialog with its reason.
    CommitEditingKitDraft(CustomEditingKitDraft),
    RemoveEditingKit(CustomEditingKitRemoval),
    ChooseBlenderPath,
    /// Use the USMAP at this typed path, or the bundled one when it is empty.
    CommitChimpUsmapInput(String),
    ChooseChimpUsmap,
    UseBundledUsmap,
}

impl Baboon {
    pub(in crate::app) fn apply_settings_command(&mut self, command: SettingsCommand, ctx: &egui::Context) {
        match command {
            SettingsCommand::MountChimpEverywhere => {
                let indices: Vec<usize> = self
                    .model
                    .kits
                    .iter()
                    .enumerate()
                    .filter(|(_, kit)| {
                        kit.source.as_ref().is_some_and(|source| {
                            matches!(&source.source, TagSource::IoStoreContainerSet { .. })
                        })
                    })
                    .map(|(index, _)| index)
                    .collect();
                for index in indices {
                    self.begin_chimp_mount(index, ctx.clone());
                }
            }
            SettingsCommand::DropChimpEverywhere => {
                for index in 0..self.model.kits.len() {
                    self.views[self.model.kits[index].id].surface = KitSurface::Tags;
                    self.reset_chimp(index);
                }
            }
            SettingsCommand::CheckForUpdates => self.begin_check_for_updates(ctx.clone(), false),
            SettingsCommand::ForgetUpdateCheck => self.forget_update_check(),
            SettingsCommand::AutoDetectEditingKits => self.auto_detect_editing_kit_paths(),
            SettingsCommand::RefreshEditingKitStatus => {
                self.refresh_editing_kit_validation();
                self.model.status = "Editing-kit status refreshed".to_owned();
            }
            SettingsCommand::LoadEditingKit(profile) => {
                self.load_custom_editing_kit_profile(profile, ctx.clone());
            }
            SettingsCommand::ReorderEditingKits(request) => self.reorder_editing_kits(&request),
            SettingsCommand::CommitEditingKitDraft(mut draft) => {
                if !self.commit_custom_editing_kit_draft(&mut draft) {
                    self.dialogs.open(draft);
                }
            }
            SettingsCommand::RemoveEditingKit(removal) => self.remove_custom_editing_kit_profile(&removal),
            SettingsCommand::ChooseBlenderPath => self.choose_blender_path(),
            SettingsCommand::CommitChimpUsmapInput(input) => {
                self.commit_chimp_usmap_path_input(&input, ctx.clone())
            }
            SettingsCommand::ChooseChimpUsmap => self.choose_chimp_usmap_path(ctx.clone()),
            SettingsCommand::UseBundledUsmap => self.apply_chimp_usmap_path(None, ctx.clone()),
        }
    }

    /// Open Settings on `tab`, or turn the open one to it.
    pub(in crate::app) fn open_settings(&mut self, tab: Option<SettingsTab>) {
        match self.dialogs.get_mut::<SettingsWindow>() {
            Some(settings) => {
                if let Some(tab) = tab {
                    settings.tab = tab;
                }
            }
            None => self.dialogs.open(SettingsWindow::new(
                &self.model.prefs,
                tab.unwrap_or(SettingsTab::Startup),
            )),
        }
    }

    /// Forget what the last update check found, as when the channel it
    /// checked changes.
    pub(in crate::app) fn forget_update_check(&mut self) {
        self.shell.available_update = None;
        self.shell.last_update_check = None;
    }

    /// Move an editing kit in the list as dragged, and save the order.
    fn reorder_editing_kits(&mut self, request: &EditingKitReorderRequest) {
        let previous = self.model.prefs.custom_editing_kit_profiles.clone();
        if reorder_editing_kit_profiles(&mut self.model.prefs.custom_editing_kit_profiles, request) {
            let prefs = self.current_prefs();
            if let Err(error) = save_gui_prefs(
                &prefs,
                &self.kit_tools.terminal_open_games,
                self.dialogs.get::<FirstRunWizardState>().is_none(),
            ) {
                self.model.prefs.custom_editing_kit_profiles = previous;
                self.model.status = error;
            } else {
                self.saved_prefs = prefs;
                self.kit_tools.saved_terminal_open_games = self.kit_tools.terminal_open_games.clone();
                self.model.status = "Editing kit order saved".to_owned();
            }
        }
    }
}

/// The Settings window, while it is open, with its editing-kit dialogs.
/// It edits a draft of the preferences; once drawn, a changed draft is
/// sent first and then whatever the window asked for, so those see the
/// preferences as the user just set them.
/// The Settings window: which tab it shows, and what is being typed into it
/// before it becomes a preference.
pub(in crate::app) struct SettingsWindow {
    pub(in crate::app) tab: SettingsTab,
    /// The UI scale while its slider is dragged, applied on release.
    pub(in crate::app) pending_ui_scale: f32,
    /// The Blender path as typed, applied when it names a file.
    pub(in crate::app) blender_path_input: String,
    /// The Chimp USMAP path as typed, applied on Enter or Apply.
    pub(in crate::app) usmap_input: String,
}

impl SettingsWindow {
    /// Settings on `tab`, its inputs starting from `prefs`.
    pub(in crate::app) fn new(prefs: &GuiPrefs, tab: SettingsTab) -> Self {
        Self {
            tab,
            pending_ui_scale: prefs.ui_scale,
            blender_path_input: blender_path_input(prefs),
            usmap_input: prefs
                .chimp_usmap_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        }
    }
}

impl SettingsDraw<'_> {
    /// Open `dialog` over Settings once it has drawn.
    fn opened_dialog(&mut self, dialog: impl Dialog) {
        self.opened.push(Box::new(dialog));
    }
}

impl Dialog for SettingsWindow {
    fn show(&mut self, cx: &Ctx, app: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut s = SettingsDraw {
            prefs: cx.model.prefs.clone(),
            window: self,
            app,
            opened: Vec::new(),
            effects: Vec::new(),
        };

        let mut open = true;
        egui::Window::new("Settings")
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::new("app_settings"))
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_width(window_width(ctx, 760.0))
            .default_height(window_height(ctx, 640.0, false))
            .show(ctx, |ui| {
                let mut selected = s.window.tab;
                settings_window_body(ui, &mut open, &mut selected, |ui, tab| match tab {
                    SettingsTab::Startup => draw_settings_startup_tab(cx, ui, &mut s),
                    SettingsTab::Browser => draw_settings_browser_tab(ui, &mut s),
                    SettingsTab::EditingKits => draw_settings_editing_kits_tab(cx, ui, &mut s),
                    SettingsTab::Appearance => draw_settings_appearance_tab(cx, ui, &mut s),
                    SettingsTab::Tools => draw_settings_tools_tab(cx, ui, &mut s),
                });
                s.window.tab = selected;
            });
        let SettingsDraw {
            prefs,
            opened,
            effects,
            ..
        } = s;
        // The draft goes first, so what the window asked for sees the
        // preferences as the user just set them.
        if prefs != cx.model.prefs {
            cx.edit_prefs(move |live| *live = prefs);
        }
        for effect in effects {
            cx.send(effect);
        }
        for dialog in opened {
            cx.send(crate::app::context::Command::OpenDialog(dialog));
        }
        open
    }
}

fn draw_settings_startup_tab(cx: &Ctx, ui: &mut Ui, s: &mut SettingsDraw) {
    ui.label(
        RichText::new("When reopening Baboon with a previous session:").color(text_dark()),
    );
    ui.add_space(2.0);
    ui.radio_value(
        &mut s.prefs.session_restore,
        SessionRestore::Ask,
        "Ask which windows to reopen",
    );
    ui.radio_value(
        &mut s.prefs.session_restore,
        SessionRestore::Always,
        "Reopen the last session automatically",
    );
    ui.radio_value(
        &mut s.prefs.session_restore,
        SessionRestore::Never,
        "Start fresh (never reopen)",
    );

    ui.add_space(10.0);
    ui.separator();
    ui.label(RichText::new("Saving").color(text_dark()).strong());
    ui.add_space(4.0);
    if s.prefs.expert_mode {
        ui.checkbox(
            &mut s.prefs.confirm_container_overwrite,
            "Confirm before Save overwrites Campaign Evolved game files",
        );
        ui.label(
            RichText::new(
                "Expert mode lets Save write a tag straight back into the game's own pak files. That edits the installed game in place; File \u{2192} Export Mod\u{2026} bundles the same changes into a separate mod instead.",
            )
            .color(subtle_dark())
            .small(),
        );
    } else {
        // The setting guards a route that is not reachable outside expert
        // mode, and a checkbox for something that cannot happen is worse
        // than no checkbox.
        ui.label(
            RichText::new(
                "Saving a tag loaded from a Campaign Evolved container keeps the change in this workspace and offers to export it as a mod; the game's own pak files are never written. Turn on expert mode below to allow overwriting them in place.",
            )
            .color(subtle_dark())
            .small(),
        );
    }

    ui.add_space(10.0);
    ui.separator();
    ui.label(
        RichText::new("Chimp — Unreal packages")
            .color(text_dark())
            .strong(),
    );
    ui.add_space(4.0);
    let chimp_changed = ui
        .checkbox(
            &mut s.prefs.enable_chimp,
            "Enable Chimp workspace for Campaign Evolved",
        )
        .changed();
    ui.label(
        RichText::new(
            "Chimp shares Campaign Evolved's configured path and writes supported property edits to a separate _P mod container.",
        )
        .color(subtle_dark())
        .small(),
    );
    ui.horizontal(|ui| {
        let output = s
            .prefs
            .chimp_output_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Default: Paks/~mods/Chimp".to_owned());
        ui.label(RichText::new(output).color(subtle_dark()).small());
        if ui.button("Output folder…").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Choose Chimp mod output folder")
                .pick_folder()
        {
            s.prefs.chimp_output_dir = Some(path);
        }
        if s.prefs.chimp_output_dir.is_some() && ui.button("Use default").clicked() {
            s.prefs.chimp_output_dir = None;
        }
    });
    if chimp_changed {
        let dirty = cx
            .model
            .kits
            .iter()
            .any(|kit| kit.chimp.documents.values().any(|document| document.dirty));
        if !s.prefs.enable_chimp && dirty {
            s.prefs.enable_chimp = true;
            cx.set_status("Build the Chimp mod before disabling a workspace with recovered edits.");
            return;
        }
        s.effects.push(if s.prefs.enable_chimp {
            SettingsCommand::MountChimpEverywhere
        } else {
            SettingsCommand::DropChimpEverywhere
        });
    }

    ui.add_space(10.0);
    ui.separator();
    ui.label(RichText::new("Runtime poking").color(text_dark()).strong());
    ui.add_space(4.0);
    ui.checkbox(
        &mut s.prefs.confirm_runtime_poke,
        "Confirm before poking the running game",
    );
    ui.label(
        RichText::new(
            "File \u{2192} Poke Current Tag\u{2026} (Ctrl+P) shows the preflight plan and waits for confirmation. Turn this off to write to the running game as soon as the poke is requested.",
        )
        .color(subtle_dark())
        .small(),
    );

    ui.add_space(10.0);
    ui.separator();
    ui.label(RichText::new("Updates").color(text_dark()).strong());
    ui.add_space(4.0);
    if draw_update_channel_picker(ui, &mut s.prefs) {
        s.effects.push(SettingsCommand::ForgetUpdateCheck);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("Check now").clicked() {
            s.effects.push(SettingsCommand::CheckForUpdates);
        }
        draw_update_check_result(ui, s.app.shell);
    });
}

/// Radio rows for which build track update checks follow, plus whether the
/// check runs at startup. Returns whether the channel changed, which makes
/// the last check's verdict stale: it says nothing about the new channel.
/// Shared by Settings and the first-run wizard so the two cannot drift.
pub(in crate::app) fn draw_update_channel_picker(ui: &mut Ui, prefs: &mut GuiPrefs) -> bool {
    ui.label(RichText::new("Check for updates on").color(text_dark()));
    let mut changed = false;
    for option in UpdateChannel::ALL {
        changed |= ui
            .radio_value(&mut prefs.update_channel, option, option.label())
            .on_hover_text(option.help())
            .changed();
    }
    ui.add_space(4.0);
    ui.checkbox(
        &mut prefs.check_updates_on_startup,
        "Check for updates when Baboon starts",
    );
    changed
}

/// One line describing what the last check concluded, with a link when
/// there is something to go and get.
fn draw_update_check_result(ui: &mut Ui, shell: &ShellFeature) {
    if let Some(update) = shell.available_update.as_ref() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Update available:")
                    .color(text_dark())
                    .strong(),
            );
            ui.hyperlink_to(update.short_name(), &update.release_url);
        });
        return;
    }
    ui.label(
        RichText::new(shell.update_check_summary())
            .color(subtle_dark())
            .small(),
    );
}

/// Radio row for how nested containers in the tag editor start out.
/// Shared by Settings and the first-run wizard so the two cannot drift.
pub(in crate::app) fn draw_nested_default_picker(ui: &mut Ui, nested_default: &mut NestedDefault) {
    ui.label(RichText::new("Groups, structs and blocks start").color(text_dark()));
    ui.horizontal(|ui| {
        for option in NestedDefault::ALL {
            ui.radio_value(nested_default, option, option.label())
                .on_hover_text(option.help());
        }
    });
    ui.label(
        RichText::new(
            "Applies to tags opened from now on. A group you open or close yourself keeps \
             the state you chose.",
        )
        .color(subtle_dark())
        .small(),
    );
}

fn draw_settings_browser_tab(ui: &mut Ui, s: &mut SettingsDraw) {
    ui.checkbox(
        &mut s.prefs.double_click_to_open_tags,
        "Double-click to open tags",
    );
    ui.checkbox(
        &mut s.prefs.folders_before_tags,
        "List subfolders before tags in browser",
    );
    ui.add_space(12.0);
    ui.label(RichText::new("Tag editor").color(text_dark()).strong());
    ui.add_space(4.0);
    draw_nested_default_picker(ui, &mut s.prefs.nested_default);
}

fn draw_settings_editing_kits_tab(cx: &Ctx, ui: &mut Ui, s: &mut SettingsDraw) {
    ui.label(
        RichText::new(
            "Add editing kits for quick loading, or auto-detect supported Steam installations.",
        )
        .color(subtle_dark()),
    );
    ui.horizontal(|ui| {
        if icon_text_button(ui, ButtonIcon::Add, "Add Editing Kit", true).clicked() {
            s.opened_dialog(CustomEditingKitDraft::new());
        }
        if ui.button("Auto Detect").clicked() {
            s.effects.push(SettingsCommand::AutoDetectEditingKits);
        }
        if ui.button("Refresh Status").clicked() {
            s.effects.push(SettingsCommand::RefreshEditingKitStatus);
        }
    });
    ui.add_space(6.0);

    if s.prefs.custom_editing_kit_profiles.is_empty() {
        ui.label(RichText::new("No editing kits configured").color(subtle_dark()));
    }
    for profile in s.prefs.custom_editing_kit_profiles.clone() {
        let validation = s.app.kit_tools.editing_kit_validation.custom(&profile.id);
        let warning = s
            .app
            .kit_tools.editing_kit_validation
            .custom_icon_error(&profile.id)
            .map(str::to_owned);
        let texture = s.app.shell.artwork.workspace_banner(
            ui.ctx(),
            &cx.model.prefs.custom_editing_kit_profiles,
            profile.game_id(),
            Some(&profile.id),
        );
        let (load, edit, remove) = ui
            .push_id(&profile.id, |ui| {
                editing_kit_card_with_read_only(
                    ui,
                    &profile.name,
                    profile_location(&profile, validation.as_ref().ok()),
                    texture.as_ref(),
                    validation.as_ref().err().map(String::as_str),
                    warning.as_deref(),
                    Some(&profile.id),
                    profile.read_only && !profile.is_campaign_evolved(),
                )
            })
            .inner;
        if load {
            s.effects.push(SettingsCommand::LoadEditingKit(profile.clone()));
        }
        if edit {
            s.opened_dialog(CustomEditingKitDraft::from_profile(&profile));
        }
        if remove {
            s.opened_dialog(CustomEditingKitRemoval {
                id: profile.id.clone(),
                name: profile.name.clone(),
            });
        }
    }
    let reorder = ui.ctx().data_mut(|data| {
        let key = egui::Id::new("editing_kit_reorder_request");
        let request = data.get_temp::<EditingKitReorderRequest>(key);
        data.remove::<EditingKitReorderRequest>(key);
        request
    });
    if let Some(request) = reorder {
        s.effects.push(SettingsCommand::ReorderEditingKits(request));
    }
}

/// Adding or editing a custom editing kit, opened from Settings. Save sends
/// the draft to be committed; one the commit refuses comes back with its
/// reason.
impl Dialog for CustomEditingKitDraft {
    fn show(&mut self, cx: &Ctx, app: &AppReads) -> bool {
        let ctx = cx.egui;
        let title = if self.editing_id.is_some() {
            "Edit Editing Kit"
        } else {
            "Add Editing Kit"
        };
        let mut open = true;
        let custom_texture = draft_editing_kit_icon_texture(ctx, &self.icon);
        let texture = custom_texture.or_else(|| {
            app.shell
                .artwork
                .game_banner(ctx, GameId::from_id(&self.game))
        });
        let mut actions = EditingKitFormActions::default();
        egui::Window::new(title)
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::new("custom_editing_kit_dialog"))
            .title_bar(false)
            .collapsible(false)
            .auto_sized()
            .default_width(window_width(ctx, 580.0))
            .max_width(window_width(ctx, 580.0))
            .max_height(window_height(
                ctx,
                (ctx.content_rect().height() - 32.0).max(0.0),
                false,
            ))
            .scroll([false, true])
            .show(ctx, |ui| {
                crate::app::search::draw_icon_window_header(ui, title, ButtonIcon::Edit, &mut open);
                ui.separator();
                egui::Frame::NONE
                    .inner_margin(ui.spacing().window_margin)
                    .show(ui, |ui| {
                        actions = draw_editing_kit_form(ui, self, texture.as_ref());
                    });
            });
        let EditingKitFormActions {
            save,
            cancel,
            remove,
        } = actions;

        if remove {
            cx.open_dialog(CustomEditingKitRemoval {
                id: self.editing_id.clone().unwrap(),
                name: self.name.clone(),
            });
            return false;
        }
        if cancel || !open {
            return false;
        }
        if save {
            cx.send(SettingsCommand::CommitEditingKitDraft(self.clone()));
            return false;
        }
        true
    }
}

/// Removing a custom editing kit: its files stay where they are.
impl Dialog for CustomEditingKitRemoval {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Remove Editing Kit?")
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::new("remove_custom_editing_kit"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Remove “{}” from Baboon? Its editing-kit files will not be deleted.",
                    self.name
                ));
                ui.horizontal(|ui| {
                    confirm = ui.button("Remove").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if confirm {
            cx.send(SettingsCommand::RemoveEditingKit(self.clone()));
            return false;
        }
        open && !cancel
    }
}

fn draw_settings_appearance_tab(cx: &Ctx, ui: &mut Ui, s: &mut SettingsDraw) {
    ui.checkbox(&mut s.prefs.dark_mode, "Dark mode");
    ui.checkbox(&mut s.prefs.angles_in_degrees, "Angles in degrees")
        .on_hover_text(
            "Angle fields hold radians on disk. Guerilla and the other Halo tools show them \
             in degrees, and so does Baboon — turn this off to read and type the stored \
             radians instead. Field search, TSV copy/paste and the tag diff follow the same \
             setting.",
        );
    ui.horizontal(|ui| {
        ui.label(RichText::new("UI scale").color(subtle_dark()));
        ui.add(
            egui::Slider::new(&mut s.window.pending_ui_scale, MIN_UI_SCALE..=MAX_UI_SCALE)
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        );
        draw_ui_scale_input(ui, &mut s.window.pending_ui_scale);
        if ui.button("Apply").clicked() {
            s.prefs.ui_scale = s.window.pending_ui_scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
            cx.set_status("UI scale applied");
        }
        if ui.button("Reset").clicked() {
            s.window.pending_ui_scale = DEFAULT_UI_SCALE;
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Model viewport").color(subtle_dark()));
        ui.add(
            egui::Slider::new(
                &mut s.prefs.model_preview_size,
                MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
            )
            .show_value(false)
            .clamping(egui::SliderClamping::Always),
        );
        draw_model_viewport_size_input(ui, &mut s.prefs.model_preview_size);
        if ui.button("Reset").clicked() {
            s.prefs.model_preview_size = DEFAULT_MODEL_PREVIEW_SIZE;
        }
    });
    draw_speed_row(
        ui,
        "Scroll speed",
        "How far the mouse wheel or trackpad scrolls lists and panels. \
         100% is the original speed.",
        &mut s.prefs.scroll_speed,
        MIN_SCROLL_SPEED..=MAX_SCROLL_SPEED,
        DEFAULT_SCROLL_SPEED,
    );
    draw_speed_row(
        ui,
        "Zoom speed",
        "How fast the mouse wheel zooms the model and bitmap viewports. \
         100% is the original speed.",
        &mut s.prefs.zoom_speed,
        MIN_ZOOM_SPEED..=MAX_ZOOM_SPEED,
        DEFAULT_ZOOM_SPEED,
    );
}

fn draw_settings_tools_tab(cx: &Ctx, ui: &mut Ui, s: &mut SettingsDraw) {
    ui.label(RichText::new("Blender").color(text_dark()).strong());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Path").color(subtle_dark()));
        let path_response = ui
            .add(egui::TextEdit::singleline(&mut s.window.blender_path_input).desired_width(360.0));
        if lost_focus_once(&path_response)
            && ui.input(|input| input.key_pressed(egui::Key::Enter))
        {
            let trimmed = s.window.blender_path_input.trim();
            s.prefs.blender_path = if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            };
            cx.set_status(if let Some(path) = &s.prefs.blender_path {
                format!("Blender path set to {}", path.display())
            } else {
                "Blender path cleared".to_owned()
            });
        }
        if icon_text_button(ui, ButtonIcon::Browse, "Browse...", true).clicked() {
            s.effects.push(SettingsCommand::ChooseBlenderPath);
        }
        if icon_text_button(ui, ButtonIcon::Clear, "Clear", true).clicked() {
            s.prefs.blender_path = None;
            s.window.blender_path_input.clear();
            cx.set_status("Blender path cleared");
        }
    });

    ui.add_space(10.0);
    ui.separator();
    ui.label(
        RichText::new("Chimp — Unreal mappings")
            .color(text_dark())
            .strong(),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "USMAP files describe cooked Unreal classes and properties so Chimp can name and decode package data. Leave this blank to use Baboon's bundled Campaign Evolved mappings.",
        )
        .color(subtle_dark())
        .small(),
    );
    ui.horizontal(|ui| {
        ui.label(RichText::new("Path").color(subtle_dark()));
        let path_response = ui.add(
            egui::TextEdit::singleline(&mut s.window.usmap_input)
                .desired_width(360.0)
                .hint_text(placeholder_text("Bundled Campaign Evolved USMAP")),
        );
        if lost_focus_once(&path_response)
            && ui.input(|input| input.key_pressed(egui::Key::Enter))
        {
            s.effects.push(SettingsCommand::CommitChimpUsmapInput(
                s.window.usmap_input.clone(),
            ));
        }
        if ui.button("Browse...").clicked() {
            s.effects.push(SettingsCommand::ChooseChimpUsmap);
        }
    });
    ui.add_space(8.0);
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), BUTTON_HEIGHT),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            if s.prefs.chimp_usmap_path.is_some() && ui.button("Use bundled").clicked() {
                s.effects.push(SettingsCommand::UseBundledUsmap);
            }
        },
    );
}

fn draw_ui_scale_input(ui: &mut Ui, ui_scale: &mut f32) {
    let mut percent = ui_scale_percent(*ui_scale);
    let response = ui.add(
        egui::DragValue::new(&mut percent)
            .range(ui_scale_percent(MIN_UI_SCALE)..=ui_scale_percent(MAX_UI_SCALE))
            .speed(1.0)
            .max_decimals(0)
            .suffix("%"),
    );
    if response.changed() {
        *ui_scale = ui_scale_from_percent(percent);
    }
}

/// A speed multiplier: slider, percentage box, and Reset.
fn draw_speed_row(
    ui: &mut Ui,
    label: &str,
    hover: &str,
    speed: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(subtle_dark()))
            .on_hover_text(hover);
        ui.add(
            egui::Slider::new(speed, range.clone())
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        );
        let mut percent = *speed * 100.0;
        let response = ui.add(
            egui::DragValue::new(&mut percent)
                .range(range.start() * 100.0..=range.end() * 100.0)
                .speed(1.0)
                .max_decimals(0)
                .suffix("%"),
        );
        if response.changed() {
            *speed = (percent / 100.0).clamp(*range.start(), *range.end());
        }
        if ui.button("Reset").clicked() {
            *speed = default;
        }
    });
}

fn ui_scale_percent(ui_scale: f32) -> f32 {
    ui_scale * 100.0
}

fn ui_scale_from_percent(percent: f32) -> f32 {
    (percent / 100.0).clamp(MIN_UI_SCALE, MAX_UI_SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::shell::frame::EditingKitMenuEntry;
    use crate::app::shell::frame::visible_editing_kit_menu_entries;

    #[test]
    fn editing_kit_inputs_match_button_height() {
        let ctx = egui::Context::default();
        ctx.set_global_style(foundation_style());
        let _ = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let mut value = String::from("Editing kit");
                for interactive in [true, false] {
                    let top = ui.next_widget_position().y;
                    let response =
                        ui.add(editing_kit_text_input(&mut value, 200.0).interactive(interactive));
                    // egui 0.36: TextEdit's response rect includes its frame margins.
                    assert_eq!(response.rect.height(), 24.0);
                    assert_eq!(
                        ui.next_widget_position().y - top - ui.spacing().item_spacing.y,
                        24.0
                    );
                }
                let engine = egui::ComboBox::from_id_salt("height_test_engine")
                    .selected_text("Halo 2")
                    .show_ui(ui, |_| {});
                assert_eq!(engine.response.rect.height(), 24.0);
            });
        });
    }

    #[test]
    fn editing_kit_read_only_policy_tracks_profiles_and_excludes_campaign_evolved() {
        let mut profile = CustomEditingKitProfile {
            read_only: true,
            git_tracked: false,
            id: "read-only-kit".to_owned(),
            name: "Protected kit".to_owned(),
            game: "halo2_mcc".to_owned(),
            root: PathBuf::from("C:/Kits/Protected"),
            icon: None,
            tags_folder: None,
            data_folder: None,
        };
        let identity = EditingKitProfileIdentity {
            id: profile.id.clone(),
            name: profile.name.clone(),
        };
        assert!(profile.is_read_only_for(Some(&identity), None));
        assert!(profile.is_read_only_for(None, Some(&profile.root.join("tags"))));
        assert!(profile.is_read_only_for(
            None,
            Some(&profile.root.join("tags/objects/example.weapon"))
        ));
        assert!(!profile.is_read_only_for(None, Some(Path::new("C:/Kits/Other"))));
        // A Halo 2 kit can share its root with another kit using another tags
        // folder; this kit's read-only setting doesn't reach that one.
        assert!(!profile.is_read_only_for(None, Some(&profile.root.join("tags_moda"))));
        let mut halo3 = profile.clone();
        halo3.game = "halo3_mcc".to_owned();
        assert!(halo3.is_read_only_for(None, Some(&halo3.root)));
        assert!(halo3.is_read_only_for(None, Some(&halo3.root.join("tags_moda"))));
        assert!(CustomEditingKitDraft::from_profile(&profile).read_only);
        profile.git_tracked = true;
        assert!(CustomEditingKitDraft::from_profile(&profile).git_tracked);
        profile.read_only = false;
        assert!(!profile.is_read_only_for(Some(&identity), None));
        profile.read_only = true;
        profile.game = "haloce_evolved".to_owned();
        assert!(!profile.is_read_only_for(Some(&identity), Some(&profile.root)));

        let ctx = egui::Context::default();
        ctx.set_global_style(foundation_style());
        for game in ["halo2_mcc", "haloce_mcc", "halo3_mcc", "haloce_evolved"] {
            let mut draft = CustomEditingKitDraft::new();
            draft.game = game.to_owned();
            let output = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    draw_editing_kit_form(ui, &mut draft, None);
                });
            });
            let checkbox_visible = output.shapes.iter().any(|shape| {
                matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "Read-Only")
            });
            assert_eq!(checkbox_visible, game != "haloce_evolved");
            let git_checkbox_visible = output.shapes.iter().any(|shape| {
                matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "Tracked in Git")
            });
            assert_eq!(git_checkbox_visible, game != "haloce_evolved");
            // Only kits whose tools take -tags_dir/-data_dir offer the folders.
            let folders_visible = output.shapes.iter().any(|shape| {
                matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "Tags Folder")
            });
            assert_eq!(
                folders_visible,
                matches!(game, "haloce_mcc" | "halo2_mcc"),
                "{game}"
            );
        }
    }

    /// Choosing a root fills the folders the user hasn't chosen with the root's
    /// own `tags` and `data`; a folder the user picked survives a root change.
    #[test]
    fn choosing_a_root_fills_only_the_folders_still_on_auto() {
        let outer = crate::core::test_kits::unique_temp_dir("kit-folder-autofill");
        let first = outer.join("H2EK");
        let second = outer.join("H2EK-copy");
        for root in [&first, &second] {
            for folder in ["tags", "Data", "tags_moda"] {
                std::fs::create_dir_all(root.join(folder)).unwrap();
            }
        }
        let mut draft = CustomEditingKitDraft::new();
        draft.game = "halo2_mcc".to_owned();
        draft.root_input = first.display().to_string();
        refill_kit_folders(&mut draft);
        let filled = (
            draft.tags_folder_input.clone(),
            draft.data_folder_input.clone(),
        );

        draft.tags_folder_input = "tags_moda".to_owned();
        draft.tags_folder_auto = false;
        draft.root_input = second.display().to_string();
        refill_kit_folders(&mut draft);
        let after_user_pick = draft.tags_folder_input.clone();

        let mut halo3 = CustomEditingKitDraft::new();
        halo3.game = "halo3_mcc".to_owned();
        halo3.root_input = first.display().to_string();
        refill_kit_folders(&mut halo3);
        let _ = std::fs::remove_dir_all(&outer);

        assert_eq!(filled, ("tags".to_owned(), "Data".to_owned()));
        assert_eq!(after_user_pick, "tags_moda");
        assert!(halo3.tags_folder_input.is_empty() && halo3.data_folder_input.is_empty());
    }

    #[test]
    fn editing_kit_form_preview_tracks_draft_name_and_keeps_fields_inside_dialog() {
        let ctx = egui::Context::default();
        ctx.set_global_style(foundation_style());
        egui_extras::install_image_loaders(&ctx);
        let mut draft = CustomEditingKitDraft::new();
        draft.root_input = r"C:\Program Files (x86)\Steam\steamapps\common\H2EK".to_owned();
        for name in ["Halo 2: Rebalance", "Renamed kit"] {
            draft.name = name.to_owned();
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        // Tall enough for the Halo 2 form's folder rows; the
                        // dialog scrolls when a screen is shorter.
                        Vec2::new(600.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let right = ui.max_rect().right();
                        let actions = draw_editing_kit_form(ui, &mut draft, None);
                        assert!(!actions.save && !actions.cancel && !actions.remove);
                        assert!(ui.min_rect().right() <= right + 1.0, "form fields overflow");
                        assert!(
                            ui.next_widget_position().y < 880.0,
                            "form unexpectedly fills height"
                        );
                    });
                },
            );
            assert!(
                output.shapes.iter().any(|shape| matches!(
                    &shape.shape, egui::Shape::Text(text) if text.galley.text() == name
                        && text.galley.job.sections.iter().all(|section| section.format.font_id.size == 14.0)
                )),
                "live preview did not display changed name"
            );
            assert!(output.shapes.iter().any(|shape| matches!(
                &shape.shape, egui::Shape::Rect(rect) if rect.fill == foundation_documentation_bg()
            )), "preview is missing its translucent card fill");
        }
        assert!(
            draft_editing_kit_icon_texture(&ctx, &CustomEditingKitIconDraft::Default).is_none()
        );
        let icon = CustomEditingKitIconDraft::Selected(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/Game Icons/h2.png"),
        );
        let texture =
            draft_editing_kit_icon_texture(&ctx, &icon).expect("selected PNG has no preview");
        let cached = draft_editing_kit_icon_texture(&ctx, &icon).unwrap();
        assert_eq!(texture.id(), cached.id());
        assert!(
            draft_editing_kit_icon_texture(&ctx, &CustomEditingKitIconDraft::Default).is_none()
        );
    }

    #[test]
    fn editing_kit_action_columns_match_shared_button_sizes_at_high_dpi() {
        for scale in [1.0, 2.0, 3.0] {
            let ctx = egui::Context::default();
            ctx.set_global_style(foundation_style());
            ctx.set_pixels_per_point(scale);
            egui_extras::install_image_loaders(&ctx);
            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(400.0, 200.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        for (icon, label) in
                            [(ButtonIcon::Open, "Open"), (ButtonIcon::Edit, "Edit")]
                        {
                            let reserved = editing_kit_action_width(ui, label);
                            let button = icon_text_button(ui, icon, label, true);
                            assert!((button.rect.width() - reserved).abs() <= 1.0);
                            assert_eq!(button.rect.height(), BUTTON_HEIGHT);
                            assert!(
                                button.rect.width() < 80.0,
                                "button still has oversized fixed width"
                            );
                        }
                    });
                },
            );
        }
    }

    fn settings_frame(
        ctx: &egui::Context,
        tab: SettingsTab,
        events: Vec<egui::Event>,
    ) -> egui::Rect {
        let mut rect = egui::Rect::NOTHING;
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1200.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |_| {
                rect = egui::Window::new("Settings")
                    .id(egui::Id::new("settings_resize_test"))
                    .title_bar(false)
                    .collapsible(false)
                    .resizable(true)
                    .default_pos(egui::pos2(100.0, 100.0))
                    .default_size(window_size(ctx, Vec2::new(760.0, 400.0), false))
                    .show(ctx, |ui| {
                        let mut open = true;
                        let mut selected = tab;
                        settings_window_body(ui, &mut open, &mut selected, |ui, _| {
                            ui.label("Settings content");
                        });
                    })
                    .unwrap()
                    .response
                    .rect;
            },
        );
        rect
    }

    #[test]
    fn settings_horizontal_edge_resize_does_not_grow_height() {
        for tab in [
            SettingsTab::Startup,
            SettingsTab::Browser,
            SettingsTab::EditingKits,
            SettingsTab::Appearance,
            SettingsTab::Tools,
        ] {
            for right_edge in [false, true] {
                let ctx = egui::Context::default();
                let mut rect = settings_frame(&ctx, tab, vec![]);
                for _ in 0..4 {
                    rect = settings_frame(&ctx, tab, vec![]);
                }
                let initial = rect;
                assert!(
                    initial.height() < 500.0,
                    "settings unexpectedly expanded to full height"
                );
                let start = egui::pos2(
                    if right_edge {
                        rect.right() - 1.0
                    } else {
                        rect.left() + 1.0
                    },
                    rect.center().y,
                );
                settings_frame(&ctx, tab, vec![egui::Event::PointerMoved(start)]);
                settings_frame(
                    &ctx,
                    tab,
                    vec![egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                let mut end = start;
                for step in 1..=8 {
                    end =
                        start + Vec2::new(if right_edge { -10.0 } else { 10.0 } * step as f32, 0.0);
                    rect = settings_frame(&ctx, tab, vec![egui::Event::PointerMoved(end)]);
                    assert!(
                        (rect.height() - initial.height()).abs() <= 1.0,
                        "horizontal resize changed height: {} -> {}",
                        initial.height(),
                        rect.height()
                    );
                }
                settings_frame(
                    &ctx,
                    tab,
                    vec![egui::Event::PointerButton {
                        pos: end,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                rect = settings_frame(&ctx, tab, vec![]);
                assert!(
                    (rect.width() - initial.width()).abs() > 20.0,
                    "edge drag did not resize"
                );
                assert!((rect.height() - initial.height()).abs() <= 1.0);
            }
        }
    }

    #[test]
    fn settings_window_can_shrink_vertically() {
        for tab in [
            SettingsTab::Startup,
            SettingsTab::Browser,
            SettingsTab::EditingKits,
            SettingsTab::Appearance,
            SettingsTab::Tools,
        ] {
            let ctx = egui::Context::default();
            let mut rect = settings_frame(&ctx, tab, vec![]);
            for _ in 0..4 {
                rect = settings_frame(&ctx, tab, vec![]);
            }
            let initial = rect;
            assert!(
                initial.height() < 500.0,
                "settings unexpectedly expanded to full height"
            );
            let start = egui::pos2(rect.center().x, rect.bottom() - 1.0);
            settings_frame(&ctx, tab, vec![egui::Event::PointerMoved(start)]);
            settings_frame(
                &ctx,
                tab,
                vec![egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            let end = start - Vec2::new(0.0, 100.0);
            settings_frame(&ctx, tab, vec![egui::Event::PointerMoved(end)]);
            settings_frame(
                &ctx,
                tab,
                vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            rect = settings_frame(&ctx, tab, vec![]);
            assert!(
                rect.height() < initial.height() - 80.0,
                "vertical resize is locked: {} -> {}",
                initial.height(),
                rect.height()
            );
        }
    }

    #[test]
    fn editing_kit_reordering_moves_entries_before_and_after_without_changing_identity() {
        let mut profiles: Vec<_> = ["one", "two", "three"]
            .into_iter()
            .map(|id| CustomEditingKitProfile {
                read_only: false,
                git_tracked: false,
                id: id.to_owned(),
                name: id.to_owned(),
                game: "halo2_mcc".to_owned(),
                root: PathBuf::from(format!("C:/Kits/{id}")),
                icon: None,
                tags_folder: None,
                data_folder: None,
            })
            .collect();
        let original = profiles.clone();
        assert!(reorder_editing_kit_profiles(
            &mut profiles,
            &EditingKitReorderRequest {
                source: "one".to_owned(),
                target: "three".to_owned(),
                after: true,
            }
        ));
        assert_eq!(
            profiles,
            vec![
                original[1].clone(),
                original[2].clone(),
                original[0].clone()
            ]
        );
        assert!(reorder_editing_kit_profiles(
            &mut profiles,
            &EditingKitReorderRequest {
                source: "one".to_owned(),
                target: "two".to_owned(),
                after: false,
            }
        ));
        assert_eq!(profiles, original);
        assert!(!reorder_editing_kit_profiles(
            &mut profiles,
            &EditingKitReorderRequest {
                source: "one".to_owned(),
                target: "two".to_owned(),
                after: false,
            }
        ));
        let validation = EditingKitValidationCache::new(&HashMap::new(), &profiles);
        let entries = visible_editing_kit_menu_entries(&profiles, &validation);
        assert_eq!(entries.len(), 3);
        for (entry, profile) in entries.iter().zip(&profiles) {
            assert!(matches!(entry, EditingKitMenuEntry::Custom(entry) if entry.id == profile.id));
        }
    }

    #[test]
    fn editing_kit_grabber_drag_produces_reorder_request() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let mut positions = [egui::Pos2::ZERO; 2];
        let mut frame = |events| {
            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(360.0, 240.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        for (index, id) in ["one", "two"].into_iter().enumerate() {
                            let top = ui.next_widget_position();
                            positions[index] = top + Vec2::new(9.0, 22.0);
                            ui.push_id(id, |ui| {
                                editing_kit_card(
                                    ui,
                                    id,
                                    Path::new("C:/Kit"),
                                    None,
                                    None,
                                    None,
                                    Some(id),
                                );
                            });
                        }
                    });
                },
            );
            positions
        };
        let positions = frame(vec![]);
        let start = positions[0];
        frame(vec![egui::Event::PointerMoved(start)]);
        frame(vec![egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        frame(vec![egui::Event::PointerMoved(
            start + Vec2::new(0.0, 10.0),
        )]);
        let end = positions[1] + Vec2::new(80.0, 12.0);
        frame(vec![egui::Event::PointerMoved(end)]);
        frame(vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        let request = ctx
            .data(|data| {
                data.get_temp::<EditingKitReorderRequest>(egui::Id::new(
                    "editing_kit_reorder_request",
                ))
            })
            .expect("grabber drag did not produce a reorder request");
        assert_eq!(request.source, "one");
        assert_eq!(request.target, "two");
        assert!(request.after);
    }

    #[test]
    fn editing_kit_cards_fit_settings_widths_with_long_paths() {
        for width in [360.0, 760.0] {
            for error in [None, Some("Folder not found")] {
                let ctx = egui::Context::default();
                egui_extras::install_image_loaders(&ctx);
                let output = crate::app::run_ui_test(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(width, 400.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            let right = ui.max_rect().right();
                            let first_top = ui.next_widget_position().y;
                            assert_eq!(
                                editing_kit_card(
                                    ui,
                                    "My Halo 2 editing kit",
                                    Path::new(
                                        r"C:\Program Files (x86)\Steam\steamapps\common\H2EK"
                                    ),
                                    None,
                                    error,
                                    None,
                                    Some("first-kit"),
                                ),
                                (false, false, false)
                            );
                            let second_top = ui.next_widget_position().y;
                            editing_kit_card(
                                ui,
                                "Second kit",
                                Path::new(r"C:\H2EK"),
                                None,
                                error,
                                Some("Custom image unavailable"),
                                Some("second-kit"),
                            );
                            let third_top = ui.next_widget_position().y;
                            let first_height = second_top - first_top;
                            let second_height = third_top - second_top;
                            assert!(
                                first_height <= 60.0,
                                "first row is too tall: {first_height}"
                            );
                            assert!(
                                (first_height - second_height).abs() <= 1.0,
                                "row heights differ: {first_height} vs {second_height}"
                            );
                            assert!(
                                ui.min_rect().right() <= right + 1.0,
                                "editing kit card overflows at width {width}"
                            );
                        });
                    },
                );
                for label in ["Open", "Edit"] {
                    let positions: Vec<f32> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| {
                            if let egui::Shape::Text(text) = &shape.shape {
                                (text.galley.text() == label).then_some(text.pos.x)
                            } else {
                                None
                            }
                        })
                        .collect();
                    assert_eq!(positions.len(), 2, "missing {label} buttons");
                    assert!(
                        (positions[0] - positions[1]).abs() <= 1.0,
                        "{label} buttons are not column-aligned"
                    );
                }
            }
        }
    }

    #[test]
    fn ui_scale_percentage_conversion_clamps_to_supported_range() {
        assert_eq!(ui_scale_percent(1.25), 125.0);
        assert_eq!(ui_scale_from_percent(125.0), 1.25);
        assert_eq!(ui_scale_from_percent(20.0), MIN_UI_SCALE);
        assert_eq!(ui_scale_from_percent(400.0), MAX_UI_SCALE);
    }
}
