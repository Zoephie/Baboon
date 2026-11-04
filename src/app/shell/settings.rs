//! Preferences window and its settings tabs.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;
use crate::app::shell::frame::draw_kit_banner_tile;
use crate::app::shell::frame::editing_kit_title_text;
use crate::app::shell::frame::recheck_cached;

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
        settings_card_fill(is_dark_mode())
    };
    let mut load = false;
    let mut edit = false;
    let mut remove = false;
    let card = Frame::NONE
        .fill(fill)
        .stroke(Stroke::new(1.0, foundation_group_edge()))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(5))
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

fn editing_kit_text_input(value: &mut String, width: f32) -> SinglelineTextEdit<'_> {
    crate::app::ui_kit::singleline_text_edit(value)
        .desired_width(width)
        .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
        .vertical_align(egui::Align::Center)
}

fn editing_kit_input_state(draft: &CustomEditingKitDraft, field: CustomEditingKitInput) -> InputState {
    if draft.error.is_some() && draft.error_input.as_ref() == Some(&field) {
        InputState::Invalid
    } else if field == CustomEditingKitInput::Icon && draft.icon_warning.is_some() {
        InputState::Warning
    } else {
        InputState::Normal
    }
}

fn draw_editing_kit_form(
    ui: &mut Ui,
    draft: &mut CustomEditingKitDraft,
    texture: Option<&egui::TextureHandle>,
) -> EditingKitFormActions {
    let mut actions = EditingKitFormActions::default();
    egui::Frame::NONE.inner_margin(ui.spacing().window_margin).show(ui, |ui| {
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
        let state = editing_kit_input_state(draft, CustomEditingKitInput::Name);
        if columns[0]
            .add(
                editing_kit_text_input(&mut draft.name, width)
                    .validation_state(state)
                    .hint_text(placeholder_text("Editing kit name")),
            )
            .changed()
        {
            if draft.error_input == Some(CustomEditingKitInput::Name) { draft.error = None; }
            ui_repaint_for_kit_draft(&columns[0]);
        }
        editing_kit_field_label(&mut columns[1], "Engine");
        crate::app::ui_kit::ValueComboBox::from_id_salt("editing_kit_form_engine")
            .selected_text(game_display_name(&draft.game))
            .width(columns[1].available_width())
            .show_ui(&mut columns[1], |ui| {
                for game in GameId::ALL {
                    if ui
                        .selectable_value(
                            &mut draft.game,
                            game.as_str().to_owned(),
                            game.display_name(),
                        )
                        .changed()
                    {
                        draft.error = None;
                        refill_kit_folders(draft);
                        ui_repaint_for_kit_draft(ui);
                    }
                }
            });
    });
    ui.add_space(8.0);
    editing_kit_field_label(ui, "Editing Kit Root Folder");
    let root_state = editing_kit_input_state(draft, CustomEditingKitInput::Root);
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
                    .validation_state(root_state)
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
    let icon_state = editing_kit_input_state(draft, CustomEditingKitInput::Icon);
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
                .validation_state(icon_state)
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
                Err(error) => {
                    draft.error = Some(error);
                    draft.error_input = Some(CustomEditingKitInput::Icon);
                }
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
        ui.baboon_checkbox(&mut draft.read_only, "Read-Only");
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
        ui.baboon_checkbox(&mut draft.git_tracked, "Tracked in Git");
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
    });
    ui.add_space(8.0);
    popup_footer(ui, |ui| {
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
    });
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
    let field = match folder { KitFolder::Tags => CustomEditingKitInput::Tags, KitFolder::Data => CustomEditingKitInput::Data };
    let state = editing_kit_input_state(draft, field.clone());
    let mut changed = false;
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
            .add(editing_kit_text_input(input, width).validation_state(state).hint_text(placeholder_text(&hint)))
            .changed()
        {
            changed = true;
            *auto = input.trim().is_empty();
            ui_repaint_for_kit_draft(ui);
        }
        if icon_text_button(ui, ButtonIcon::Browse, "Browse", true).clicked() {
            let mut dialog = rfd::FileDialog::new().set_title(format!("Select {}", folder.label()));
            if root.is_dir() {
                dialog = dialog.set_directory(&root);
            }
            if let Some(path) = dialog.pick_folder() {
                changed = true;
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
            changed = true;
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
                    changed = true;
                    *input = name.clone();
                    *auto = false;
                    ui_repaint_for_kit_draft(ui);
                }
            }
        });
    }
    if changed && draft.error_input == Some(field) { draft.error = None; }
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
    ui.add_space(8.0);
    ScrollArea::horizontal()
        .id_salt("settings_tabs_scroll")
        .max_height(BUTTON_ICON_SIZE + 20.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (tab, label) in [
                    (SettingsTab::General, "General"),
                    (SettingsTab::Interface, "Interface"),
                    (SettingsTab::EditingKits, "Editing Kits"),
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

/// Cards always consume the available width, including after a window resize.
fn settings_card_fill(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgba_premultiplied(13, 13, 13, 13)
    } else {
        Color32::from_black_alpha(13)
    }
}

fn settings_card<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::NONE
        .fill(settings_card_fill(is_dark_mode()))
        .stroke(Stroke::new(1.0, foundation_group_edge()))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !ui.visuals().dark_mode {
                let background = editor_bg().blend(settings_card_fill(false));
                opaque_widget_fills(ui.visuals_mut(), background);
            }
            content(ui)
        })
        .inner
}

fn settings_section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong().color(text_dark()));
}

fn settings_description(ui: &mut Ui, title: &str, help: &str) {
    ui.label(RichText::new(title).strong().color(text_dark()));
    if !help.is_empty() {
        settings_hint(ui, help);
    }
}

fn settings_hint(ui: &mut Ui, help: &str) {
    let width = ui.available_width().min(300.0);
    ui.allocate_ui_with_layout(
        Vec2::new(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.add(egui::Label::new(RichText::new(help).small().color(subtle_dark())).wrap());
        },
    );
}

fn settings_dropdown_option<T: PartialEq>(
    ui: &mut Ui,
    selected: &mut T,
    value: T,
    title: &str,
    hint: &str,
) -> egui::Response {
    ui.set_width(300.0);
    let mut text = egui::text::LayoutJob::default();
    text.append(
        &format!("{title}\n"),
        0.0,
        egui::TextFormat {
            font_id: bold_font(TextStyle::Button.resolve(ui.style()).size),
            color: text_dark(),
            ..Default::default()
        },
    );
    text.append(
        hint,
        0.0,
        egui::TextFormat {
            font_id: TextStyle::Small.resolve(ui.style()),
            color: subtle_dark(),
            ..Default::default()
        },
    );
    ui.selectable_value(selected, value, text)
}

fn settings_actions<R>(ui: &mut Ui, actions: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), BUTTON_HEIGHT),
        egui::Layout::right_to_left(egui::Align::Center),
        actions,
    )
    .inner
}

fn settings_label_layout(help: &str) -> egui::Layout {
    if help.is_empty() {
        egui::Layout::left_to_right(egui::Align::Center)
    } else {
        egui::Layout::top_down(egui::Align::Min)
    }
}

/// Anchor actions to the right edge, including when narrow rows stack.
/// Action closures draw from right to left, starting with the last button.
fn settings_row<R>(
    ui: &mut Ui,
    title: &str,
    help: &str,
    control_width: f32,
    control: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.push_id(title, |ui| {
        settings_card(ui, |ui| {
            let min_label_width = if help.is_empty() { 90.0 } else { 180.0 };
            if ui.available_width() < control_width + min_label_width {
                settings_description(ui, title, help);
                ui.add_space(4.0);
                settings_actions(ui, control)
            } else {
                ui.horizontal(|ui| {
                    let label_width =
                        ui.available_width() - control_width - ui.spacing().item_spacing.x;
                    ui.allocate_ui_with_layout(
                        Vec2::new(label_width, BUTTON_HEIGHT),
                        settings_label_layout(help),
                        |ui| {
                            ui.set_min_width(label_width);
                            settings_description(ui, title, help);
                        },
                    );
                    ui.allocate_ui_with_layout(
                        Vec2::new(control_width, BUTTON_HEIGHT),
                        egui::Layout::right_to_left(egui::Align::Center),
                        control,
                    )
                    .inner
                })
                .inner
            }
        })
    })
    .inner
}

fn settings_switch(ui: &mut Ui, value: &mut bool, label: &str) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(46.0, 24.0), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, label)
    });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, *value);
        let fill = if *value {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().extreme_bg_color
        };
        ui.painter().rect(
            rect,
            12.0,
            fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let center = egui::pos2(
            if *value {
                rect.right() - 12.0
            } else {
                rect.left() + 12.0
            },
            rect.center().y,
        );
        ui.painter().circle_filled(
            center,
            9.0,
            if *value {
                Color32::WHITE
            } else {
                visuals.fg_stroke.color
            },
        );
    }
    response
}

fn settings_toggle(ui: &mut Ui, title: &str, help: &str, value: &mut bool) -> bool {
    ui.push_id(title, |ui| {
        settings_card(ui, |ui| {
            ui.horizontal(|ui| {
                let label_width =
                    (ui.available_width() - 46.0 - ui.spacing().item_spacing.x).max(0.0);
                let label_clicked = ui
                    .allocate_ui_with_layout(
                        Vec2::new(label_width, BUTTON_HEIGHT),
                        settings_label_layout(help),
                        |ui| {
                            ui.set_min_width(label_width);
                            let clicked = ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(title).strong().color(text_dark()),
                                    )
                                    .sense(Sense::click()),
                                )
                                .clicked();
                            if !help.is_empty() {
                                settings_hint(ui, help);
                            }
                            clicked
                        },
                    )
                    .inner;
                if label_clicked {
                    *value = !*value;
                }
                settings_switch(ui, value, title)
                    .on_hover_text(title)
                    .changed()
                    || label_clicked
            })
            .inner
        })
    })
    .inner
}

impl Baboon {
    pub(in crate::app) fn set_editing_kit_path_input(
        &mut self,
        shortcut: EditingKitShortcut,
        input: String,
    ) {
        let trimmed = input.trim().to_owned();
        if trimmed.is_empty() {
            self.model
                .prefs
                .editing_kit_paths
                .remove(shortcut.game.as_str());
        } else {
            self.model
                .prefs
                .editing_kit_paths
                .insert(shortcut.game.as_str().to_owned(), PathBuf::from(&trimmed));
        }
        self.kit_tools
            .editing_kit_path_inputs
            .insert(shortcut.game.as_str().to_owned(), input);
        if self.kit_tools.editing_kit_path_attention.as_deref() == Some(shortcut.game.as_str())
            && !trimmed.is_empty()
        {
            self.kit_tools.editing_kit_path_attention = None;
        }
        self.refresh_builtin_editing_kit_validation(shortcut);
    }

    fn commit_custom_editing_kit_draft(&mut self, draft: &mut CustomEditingKitDraft) -> bool {
        draft.error_input = None;
        let name = draft.name.trim().to_owned();
        if name.is_empty() {
            draft.error = Some("Enter an editing kit name".to_owned());
            draft.error_input = Some(CustomEditingKitInput::Name);
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
                draft.error_input = Some(if choosable && root_input.is_dir()
                    && (tags_input.is_some() || data_input.is_some()) {
                    if validate_kit_layout(&root_input, game.as_str(), tags_input.as_deref(), None).is_err() {
                        CustomEditingKitInput::Tags
                    } else { CustomEditingKitInput::Data }
                } else { CustomEditingKitInput::Root });
                return false;
            }
        };
        if custom_profile_tags_conflicts(
            &self.model.prefs.custom_editing_kit_profiles,
            draft.editing_id.as_deref(),
            &layout.tags,
        ) {
            draft.error = Some("Another editing kit already uses this tags folder".to_owned());
            draft.error_input = Some(if choosable { CustomEditingKitInput::Tags } else { CustomEditingKitInput::Root });
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
            .model
            .prefs
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
                        draft.error_input = Some(CustomEditingKitInput::Icon);
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
            .model
            .prefs
            .custom_editing_kit_profiles
            .iter()
            .position(|existing| existing.id == id)
        {
            self.model.prefs.custom_editing_kit_profiles[index] = profile.clone();
        } else {
            self.model
                .prefs
                .custom_editing_kit_profiles
                .push(profile.clone());
        }
        let prefs = self.current_prefs();
        if let Err(error) = save_gui_prefs(&prefs, &self.kit_tools.terminal_open_games, true) {
            self.model.prefs.custom_editing_kit_profiles = previous_profiles;
            if previous.as_ref().and_then(|profile| profile.icon.as_ref()) != profile.icon.as_ref()
                && let Some(icon) = &profile.icon
            {
                let _ = remove_unreferenced_custom_icon(
                    icon,
                    &self.model.prefs.custom_editing_kit_profiles,
                );
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
            .model
            .prefs
            .custom_editing_kit_profiles
            .iter()
            .find(|profile| profile.id == removal.id)
            .cloned();
        self.model
            .prefs
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
            && let Err(error) = remove_unreferenced_custom_icon(
                &icon,
                &self.model.prefs.custom_editing_kit_profiles,
            )
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
    /// Use the USMAP at this typed path, or the bundled one when it is empty.
    CommitChimpUsmapInput(String),
    ChooseChimpUsmap,
    UseBundledUsmap,
}

impl Baboon {
    pub(in crate::app) fn apply_settings_command(
        &mut self,
        command: SettingsCommand,
        ctx: &egui::Context,
    ) {
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
            SettingsCommand::RemoveEditingKit(removal) => {
                self.remove_custom_editing_kit_profile(&removal)
            }
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
                tab.unwrap_or(SettingsTab::General),
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
        if reorder_editing_kit_profiles(&mut self.model.prefs.custom_editing_kit_profiles, request)
        {
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
                self.kit_tools.saved_terminal_open_games =
                    self.kit_tools.terminal_open_games.clone();
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
    /// The UI scale draft, committed only by the Apply button.
    pub(in crate::app) pending_ui_scale: f32,
    /// The Chimp USMAP path as typed, applied on Enter.
    pub(in crate::app) usmap_input: String,
}

impl SettingsWindow {
    /// Settings on `tab`, its inputs starting from `prefs`.
    pub(in crate::app) fn new(prefs: &GuiPrefs, tab: SettingsTab) -> Self {
        Self {
            tab,
            pending_ui_scale: prefs.ui_scale,
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
                    SettingsTab::General => draw_settings_general_tab(cx, ui, &mut s),
                    SettingsTab::EditingKits => draw_settings_editing_kits_tab(cx, ui, &mut s),
                    SettingsTab::Interface => draw_settings_interface_tab(cx, ui, &mut s),
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

fn draw_settings_general_tab(cx: &Ctx, ui: &mut Ui, s: &mut SettingsDraw) {
    let restore_label = |restore| match restore {
        SessionRestore::Ask => "Ask every time",
        SessionRestore::Always => "Reopen last session",
        SessionRestore::Never => "Start fresh",
    };
    settings_row(
        ui,
        "Session Restore",
        "Choose what happens when opening Baboon after exiting with open windows.",
        170.0,
        |ui| {
            crate::app::ui_kit::ValueComboBox::from_id_salt("session_restore")
                .width(170.0)
                .selected_text(restore_label(s.prefs.session_restore))
                .show_ui(ui, |ui| {
                    for option in [
                        SessionRestore::Ask,
                        SessionRestore::Always,
                        SessionRestore::Never,
                    ] {
                        settings_dropdown_option(
                            ui,
                            &mut s.prefs.session_restore,
                            option,
                            restore_label(option),
                            match option {
                            SessionRestore::Ask => "Ask which windows to reopen from the previous session each time Baboon starts.",
                            SessionRestore::Always => "Automatically reopen all windows from the previous session.",
                            SessionRestore::Never => "Start with a fresh workspace without restoring or prompting about previous windows.",
                            },
                        );
                    }
                });
        },
    );

    settings_section(ui, "Updates");
    settings_card(ui, |ui| {
        settings_row_contents_channel(ui, &mut s.prefs, &mut s.effects);
        ui.separator();
        settings_description(
            ui,
            "Current Version",
            &format!(
                "{} · {}",
                env!("CARGO_PKG_VERSION"),
                BABOON_BUILD_COMMIT.chars().take(7).collect::<String>()
            ),
        );
    });
    settings_card(ui, |ui| {
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 46.0 - ui.spacing().item_spacing.x).max(0.0);
            ui.allocate_ui_with_layout(
                Vec2::new(width, BUTTON_HEIGHT),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(width);
                    settings_description(
                        ui,
                        "Check for Updates",
                        "Automatically check the selected update channel on app startup.",
                    );
                },
            );
            settings_switch(
                ui,
                &mut s.prefs.check_updates_on_startup,
                "Check for updates on startup",
            )
            .on_hover_text("Check for updates on startup");
        });
        ui.separator();
        settings_actions(ui, |ui| {
            if ui.button("Check Now").clicked() {
                s.effects.push(SettingsCommand::CheckForUpdates);
            }
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                Vec2::new(width, BUTTON_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    draw_update_check_result(ui, s.app.shell);
                },
            );
        });
    });

    settings_section(ui, "Runtime Poking");
    settings_toggle(
        ui,
        "Confirm before Poking game",
        "Show the preflight plan and wait for confirmation before writing to the running game.",
        &mut s.prefs.confirm_runtime_poke,
    );
    settings_section(ui, "Campaign Evolved");
    if s.prefs.expert_mode {
        settings_toggle(
            ui,
            "Confirm before overwriting Campaign Evolved game files",
            "Save can write directly into the installed game's pak files. Export Mod bundles changes into a separate mod instead.",
            &mut s.prefs.confirm_container_overwrite,
        );
    } else {
        settings_card(ui, |ui| {
            settings_description(
                ui,
                "Saving Campaign Evolved game files",
                "Saving keeps changes in this workspace and offers to export a mod. Turn on expert mode to allow overwriting the game's pak files in place.",
            );
        });
    }

    let chimp_changed = settings_toggle(
        ui,
        "Enable Chimp workspace",
        "Edit Unreal package properties using Campaign Evolved's configured path. Changes are written to a separate _P mod container.",
        &mut s.prefs.enable_chimp,
    );
    settings_card(ui, |ui| {
        ui.label(
            RichText::new("Chimp Output Folder")
                .strong()
                .color(text_dark()),
        );
        ui.add_space(4.0);
        settings_actions(ui, |ui| {
            if icon_text_button(
                ui,
                ButtonIcon::Refresh,
                "Restore Default",
                s.prefs.chimp_output_dir.is_some(),
            )
            .on_hover_text("Use the default Paks/~mods/Chimp output folder")
            .clicked()
            {
                s.prefs.chimp_output_dir = None;
            }
            if icon_text_button(ui, ButtonIcon::Browse, "Browse...", true).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("Choose Chimp mod output folder")
                    .pick_folder()
            {
                s.prefs.chimp_output_dir = Some(path);
            }
            let mut output = s
                .prefs
                .chimp_output_dir
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            ui.add(
                editing_kit_text_input(&mut output, (ui.available_width() - 40.0).max(40.0))
                    .interactive(false)
                    .hint_text(placeholder_text("Default: Paks/~mods/Chimp")),
            );
            ui.label(RichText::new("Path").color(subtle_dark()));
        });
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
        } else {
            s.effects.push(if s.prefs.enable_chimp {
                SettingsCommand::MountChimpEverywhere
            } else {
                SettingsCommand::DropChimpEverywhere
            });
        }
    }

    draw_settings_chimp_mappings(ui, s);
}

fn settings_row_contents_channel(
    ui: &mut Ui,
    prefs: &mut GuiPrefs,
    effects: &mut Vec<SettingsCommand>,
) {
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 170.0 - ui.spacing().item_spacing.x).max(0.0);
        ui.allocate_ui_with_layout(
            Vec2::new(width, BUTTON_HEIGHT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_width(width);
                settings_description(
                    ui,
                    "Update Channel",
                    "Choose which builds Baboon checks for.",
                );
            },
        );
        let previous = prefs.update_channel;
        crate::app::ui_kit::ValueComboBox::from_id_salt("settings_update_channel")
            .width(170.0)
            .selected_text(prefs.update_channel.label())
            .show_ui(ui, |ui| {
                for option in UpdateChannel::ALL {
                    settings_dropdown_option(
                        ui,
                        &mut prefs.update_channel,
                        option,
                        option.label(),
                        option.help(),
                    );
                }
            });
        if previous != prefs.update_channel {
            effects.push(SettingsCommand::ForgetUpdateCheck);
        }
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
            .baboon_radio_value(&mut prefs.update_channel, option, option.label())
            .on_hover_text(option.help())
            .changed();
    }
    ui.add_space(4.0);
    ui.baboon_checkbox(
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
            ui.baboon_radio_value(nested_default, option, option.label())
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

fn draw_settings_browser_section(ui: &mut Ui, s: &mut SettingsDraw) {
    settings_section(ui, "Browser");
    settings_toggle(
        ui,
        "List Subfolders before Tags",
        "",
        &mut s.prefs.folders_before_tags,
    );
    settings_toggle(
        ui,
        "Double-Click to Open",
        "Open tags, folders and shortcuts in the browser with a double click.",
        &mut s.prefs.double_click_to_open_tags,
    );
    settings_toggle(ui, "Reveal Active Tab in Browser", "Automatically navigate the browser to the active tag or folder tab. Turn off to keep your place and use Reveal in Browser when needed.", &mut s.prefs.reveal_active_tab_in_browser);
}

fn draw_settings_expansion_behavior(ui: &mut Ui, s: &mut SettingsDraw) {
    settings_row(
        ui,
        "Block Default Expansion Behavior",
        "Applies to tags opened from now on. Groups you open or close yourself keep the state you chose.",
        150.0,
        |ui| {
            crate::app::ui_kit::ValueComboBox::from_id_salt("settings_nested_default")
                .width(150.0)
                .selected_text(s.prefs.nested_default.label())
                .show_ui(ui, |ui| {
                    for option in NestedDefault::ALL {
                        settings_dropdown_option(
                            ui,
                            &mut s.prefs.nested_default,
                            option,
                            option.label(),
                            option.help(),
                        );
                    }
                });
        },
    );
}
fn draw_settings_editing_kits_tab(cx: &Ctx, ui: &mut Ui, s: &mut SettingsDraw) {
    Frame::NONE
        .stroke(Stroke::new(1.0, foundation_group_edge()))
        .corner_radius(egui::CornerRadius::same(4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::NONE
                .fill(settings_card_fill(is_dark_mode()))
                .corner_radius(egui::CornerRadius {
                    nw: 4,
                    ne: 4,
                    sw: 0,
                    se: 0,
                })
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        if icon_text_button(ui, ButtonIcon::Add, "Add Editing Kit", true)
                            .on_hover_text("Add an editing kit for quick loading")
                            .clicked()
                        {
                            s.opened_dialog(CustomEditingKitDraft::new());
                        }
                        if ui
                            .button("Auto Detect")
                            .on_hover_text("Find supported Steam installations")
                            .clicked()
                        {
                            s.effects.push(SettingsCommand::AutoDetectEditingKits);
                        }
                        if icon_text_button(ui, ButtonIcon::Refresh, "Refresh Status", true)
                            .on_hover_text("Recheck the configured editing kits and their paths")
                            .clicked()
                        {
                            s.effects.push(SettingsCommand::RefreshEditingKitStatus);
                        }
                    });
                });
            Frame::NONE
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    draw_settings_editing_kits_contents(cx, ui, s);
                });
        });
}

fn draw_settings_editing_kits_contents(cx: &Ctx, ui: &mut Ui, s: &mut SettingsDraw) {
    if s.prefs.custom_editing_kit_profiles.is_empty() {
        settings_description(
            ui,
            "No editing kits configured",
            "Add editing kits for quick loading, or auto-detect supported Steam installations.",
        );
    }
    for profile in s.prefs.custom_editing_kit_profiles.clone() {
        let validation = s.app.kit_tools.editing_kit_validation.custom(&profile.id);
        let warning = s
            .app
            .kit_tools
            .editing_kit_validation
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
            s.effects
                .push(SettingsCommand::LoadEditingKit(profile.clone()));
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
                ui.add_space(8.0);
                actions = draw_editing_kit_form(ui, self, texture.as_ref());
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

pub(in crate::app) fn draw_theme_picker(ui: &mut Ui, prefs: &mut GuiPrefs) {
    let mut theme = if prefs.follow_system_theme {
        egui::ThemePreference::System
    } else if prefs.dark_mode {
        egui::ThemePreference::Dark
    } else {
        egui::ThemePreference::Light
    };
    let label = |theme| match theme {
        egui::ThemePreference::System => "Use system setting",
        egui::ThemePreference::Dark => "Dark",
        egui::ThemePreference::Light => "Light",
    };
    crate::app::ui_kit::ValueComboBox::from_id_salt("settings_theme")
        .width(170.0)
        .selected_text(label(theme))
        .show_ui(ui, |ui| {
            for option in [
                egui::ThemePreference::System,
                egui::ThemePreference::Light,
                egui::ThemePreference::Dark,
            ] {
                ui.selectable_value(&mut theme, option, label(option));
            }
        });
    prefs.follow_system_theme = theme == egui::ThemePreference::System;
    if !prefs.follow_system_theme {
        prefs.dark_mode = theme == egui::ThemePreference::Dark;
    }
}

fn draw_settings_interface_tab(cx: &Ctx, ui: &mut Ui, s: &mut SettingsDraw) {
    settings_row(ui, "Theme", "", 170.0, |ui| {
        draw_theme_picker(ui, &mut s.prefs)
    });
    if draw_settings_ui_scale(ui, &mut s.window.pending_ui_scale, &mut s.prefs.ui_scale) {
        cx.set_status("UI scale applied");
    }
    draw_speed_row(
        ui,
        "Scroll Speed",
        "How far the mouse wheel or trackpad scrolls lists and panels. 100% is the original speed.",
        &mut s.prefs.scroll_speed,
        MIN_SCROLL_SPEED..=MAX_SCROLL_SPEED,
        DEFAULT_SCROLL_SPEED,
    );
    draw_settings_browser_section(ui, s);
    settings_section(ui, "Tag Editor");
    settings_row(
        ui,
        "Display Angles as",
        "Angle fields hold radians on disk. Guerilla and the other Halo tools show them in degrees, and so does Baboon. Choose Radians to read and type the stored radians instead. Field search, TSV copy/paste and the tag diff follow the same setting.",
        150.0,
        |ui| {
            crate::app::ui_kit::ValueComboBox::from_id_salt("settings_angles")
                .width(150.0)
                .selected_text(if s.prefs.angles_in_degrees {
                    "Degrees"
                } else {
                    "Radians"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.prefs.angles_in_degrees, true, "Degrees");
                    ui.selectable_value(&mut s.prefs.angles_in_degrees, false, "Radians");
                });
        },
    );
    draw_settings_expansion_behavior(ui, s);
    settings_section(ui, "Viewport");
    settings_row(ui, "Window Scale", "", 220.0, |ui| {
        ui.spacing_mut().slider_width = 100.0;
        ui.spacing_mut().interact_size.x = 48.0;
        if ui.button("Reset").clicked() {
            s.prefs.model_preview_size = DEFAULT_MODEL_PREVIEW_SIZE;
        }
        draw_model_viewport_size_input(ui, &mut s.prefs.model_preview_size);
        ui.add(
            egui::Slider::new(
                &mut s.prefs.model_preview_size,
                MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
            )
            .show_value(false)
            .clamping(egui::SliderClamping::Always),
        );
    });
    draw_speed_row(
        ui,
        "Zoom Speed",
        "How fast the mouse wheel zooms model and bitmap viewports. 100% is the original speed.",
        &mut s.prefs.zoom_speed,
        MIN_ZOOM_SPEED..=MAX_ZOOM_SPEED,
        DEFAULT_ZOOM_SPEED,
    );
}
fn draw_settings_chimp_mappings(ui: &mut Ui, s: &mut SettingsDraw) {
    settings_card(ui, |ui| {
        ui.label(
            RichText::new("Chimp — Unreal mappings")
                .color(text_dark())
                .strong(),
        );
        ui.add_space(4.0);
        settings_hint(
            ui,
            "USMAP files describe cooked Unreal classes and properties so Chimp can name and decode package data. Leave this blank to use Baboon's bundled Campaign Evolved mappings.",
        );
        settings_actions(ui, |ui| {
            if icon_text_button(
                ui,
                ButtonIcon::Refresh,
                "Restore Default",
                s.prefs.chimp_usmap_path.is_some() || !s.window.usmap_input.is_empty(),
            )
            .on_hover_text("Use Baboon's bundled Campaign Evolved mappings")
            .clicked()
            {
                s.effects.push(SettingsCommand::UseBundledUsmap);
                s.window.usmap_input.clear();
            }
            if icon_text_button(ui, ButtonIcon::Browse, "Browse...", true).clicked() {
                s.effects.push(SettingsCommand::ChooseChimpUsmap);
            }
            let path_response = ui.add(
                editing_kit_text_input(
                    &mut s.window.usmap_input,
                    (ui.available_width() - 40.0).max(40.0),
                )
                .hint_text(placeholder_text("Bundled Campaign Evolved USMAP")),
            );
            if lost_focus_once(&path_response)
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
            {
                s.effects.push(SettingsCommand::CommitChimpUsmapInput(
                    s.window.usmap_input.clone(),
                ));
            }
            ui.label(RichText::new("Path").color(subtle_dark()));
        });
    });
}

fn draw_settings_ui_scale(ui: &mut Ui, pending: &mut f32, live: &mut f32) -> bool {
    settings_row(
        ui,
        "UI Scale",
        "Adjusts the scale of the entire interface. Adjust the scale, then select Apply.",
        270.0,
        |ui| {
            ui.spacing_mut().slider_width = 100.0;
            ui.spacing_mut().interact_size.x = 48.0;
            let applied = ui
                .add_enabled(*pending != *live, egui::Button::new("Apply"))
                .on_hover_text("Apply the UI scale to Baboon")
                .clicked();
            if ui.button("Reset").clicked() {
                *pending = DEFAULT_UI_SCALE;
            }
            draw_ui_scale_input(ui, pending);
            ui.add(
                egui::Slider::new(pending, MIN_UI_SCALE..=MAX_UI_SCALE)
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
            );
            if applied {
                *live = (*pending).clamp(MIN_UI_SCALE, MAX_UI_SCALE);
            }
            applied
        },
    )
}
fn draw_ui_scale_input(ui: &mut Ui, ui_scale: &mut f32) -> egui::Response {
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
    response
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
    settings_row(ui, label, "", 220.0, |ui| {
        ui.spacing_mut().slider_width = 100.0;
        ui.spacing_mut().interact_size.x = 48.0;
        if ui.button("Reset").clicked() {
            *speed = default;
        }
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
        ui.add(
            egui::Slider::new(speed, range.clone())
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        )
        .on_hover_text(hover);
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
    #[test]
    fn kit_settings_mark_the_invalid_input_only_after_validation() {
        let mut app = Baboon::for_test();
        let mut draft = CustomEditingKitDraft::new();
        assert_eq!(editing_kit_input_state(&draft, CustomEditingKitInput::Name), InputState::Normal);
        assert!(!app.commit_custom_editing_kit_draft(&mut draft));
        assert_eq!(draft.error_input, Some(CustomEditingKitInput::Name));
        assert_eq!(editing_kit_input_state(&draft, CustomEditingKitInput::Name), InputState::Invalid);
        draft.name = "Test kit".to_owned();
        draft.root_input = "C:/missing-test-kit-4ad686d7".to_owned();
        assert!(!app.commit_custom_editing_kit_draft(&mut draft));
        assert_eq!(draft.error_input, Some(CustomEditingKitInput::Root));
        assert_eq!(editing_kit_input_state(&draft, CustomEditingKitInput::Name), InputState::Normal);
        let ctx = egui::Context::default();
        ctx.set_global_style(foundation_style());
        let output = crate::app::run_ui_test(&ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 1000.0))),
            ..Default::default()
        }, |ui| { draw_editing_kit_form(ui, &mut draft, None); });
        let borders: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.stroke == invalid_value_stroke() => Some(rect),
            _ => None,
        }).collect();
        assert_eq!(borders.len(), 1, "the error highlights its field, not the entire form");
        assert_eq!(borders[0].corner_radius, egui::CornerRadius::same(2));
        draft.error = None;
        draft.icon_warning = Some("Image dimensions differ from the recommendation".to_owned());
        assert_eq!(editing_kit_input_state(&draft, CustomEditingKitInput::Icon), InputState::Warning);
    }

    use super::*;
    use crate::app::shell::frame::EditingKitMenuEntry;
    use crate::app::shell::frame::visible_editing_kit_menu_entries;

    #[test]
    fn settings_ui_scale_waits_for_apply_and_reset_changes_only_the_draft() {
        let ctx = egui::Context::default();
        ctx.set_global_style(foundation_style());
        let mut pending = 1.25;
        let mut live = 1.0;
        let mut frame = |events| {
            let mut applied = false;
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(600.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        applied = draw_settings_ui_scale(ui, &mut pending, &mut live);
                    });
                },
            );
            let button = |label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape {
                            (text.galley.text() == label)
                                .then(|| text.pos + text.galley.size() * 0.5)
                        } else {
                            None
                        }
                    })
                    .unwrap()
            };
            (live, pending, applied, button("Apply"), button("Reset"))
        };
        let (live, pending, applied, apply, reset) = frame(vec![]);
        assert_eq!((live, pending, applied), (1.0, 1.25, false));
        let pointer = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(pointer(apply, true));
        let (live, _, applied, _, _) = frame(pointer(apply, false));
        assert_eq!(live, 1.25);
        assert!(applied);
        frame(pointer(reset, true));
        let (live, pending, applied, _, _) = frame(pointer(reset, false));
        assert_eq!((live, pending, applied), (1.25, DEFAULT_UI_SCALE, false));
        // Apply precedes Reset in the inline row, so allow its newly enabled
        // hit target to be registered before the next press.
        frame(vec![]);
        frame(pointer(apply, true));
        let (live, _, applied, _, _) = frame(pointer(apply, false));
        assert_eq!(live, DEFAULT_UI_SCALE);
        assert!(applied);
    }

    #[test]
    fn settings_cards_fill_width_without_overflowing() {
        for width in [360.0, 560.0, 900.0] {
            let ctx = egui::Context::default();
            ctx.set_global_style(foundation_style());
            let mut action_rights = Vec::new();
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let right = ui.max_rect().right();
                        settings_row(ui, "Block Default Expansion Behavior", "Applies to tags opened from now on. A group you open or close yourself keeps the state you chose.", 150.0, |ui| {
                            crate::app::ui_kit::ValueComboBox::from_id_salt("test_expansion").width(150.0).selected_text("Expanded").show_ui(ui, |_| {});
                        });
                        let mut speed = 1.0;
                        draw_speed_row(ui, "Scroll Speed", "Scroll help", &mut speed, 0.5..=2.0, 1.0);
                        settings_row(ui, "Actions", "", 270.0, |ui| {
                            action_rights.push(ui.button("Apply").rect.right());
                        });
                        settings_card(ui, |ui| {
                            settings_actions(ui, |ui| {
                                action_rights.push(ui.button("Check Now").rect.right());
                            });
                        });
                        let mut enabled = true;
                        settings_toggle(ui, "Confirm before overwriting Campaign Evolved game files", "Save writes a tag directly into the game's own pak files when expert mode is enabled.", &mut enabled);
                        assert!((ui.min_rect().right() - right).abs() <= 1.0, "cards do not fit width {width}: {:?}", ui.min_rect());
                    });
                },
            );
            assert!(
                (action_rights[0] - action_rights[1]).abs() <= 1.0,
                "action edges differ at width {width}"
            );
            let text_rect = |label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape {
                            (text.galley.text() == label)
                                .then(|| egui::Rect::from_min_size(text.pos, text.galley.size()))
                        } else {
                            None
                        }
                    })
                    .unwrap()
            };
            assert!(
                (text_rect("Scroll Speed").center().y - text_rect("Reset").center().y).abs() <= 1.0,
                "no-hint label is not vertically centered at width {width}"
            );
            assert!(text_rect("Applies to tags opened from now on. A group you open or close yourself keeps the state you chose.").width() <= 301.0, "hint exceeds 300 points at width {width}");
        }
    }

    #[test]
    fn settings_switch_click_changes_value_once() {
        let ctx = egui::Context::default();
        let mut value = false;
        let mut rect = egui::Rect::NOTHING;
        let mut changed = false;
        let mut frame = |events| {
            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let response = settings_switch(ui, &mut value, "Test switch");
                        rect = response.rect;
                        changed = response.changed();
                    });
                },
            );
            (rect, value, changed)
        };
        let (rect, _, _) = frame(vec![]);
        let pos = rect.center();
        frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let (_, value, changed) = frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(value && changed);
        let (_, value, changed) = frame(vec![]);
        assert!(value && !changed);
    }

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
                let engine = crate::app::ui_kit::ValueComboBox::from_id_salt("height_test_engine")
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
            SettingsTab::General,
            SettingsTab::EditingKits,
            SettingsTab::Interface,
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
            SettingsTab::General,
            SettingsTab::EditingKits,
            SettingsTab::Interface,
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
