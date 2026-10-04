//! Preferences window and its settings tabs.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

#[cfg(test)]
mod editing_kit_card_tests;

#[derive(Clone)]
struct EditingKitDrag(String);

#[derive(Clone)]
struct EditingKitReorderRequest {
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
        draft.read_only && draft.game != "haloce_evolved",
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
    if draft.game != "haloce_evolved" {
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
    super::find::draw_icon_window_header(ui, "Settings", ButtonIcon::Settings, open);
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
    pub(super) fn draw_settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }

        let mut open = self.settings_open;
        egui::Window::new("Settings")
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::new("app_settings"))
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_width(window_width(ctx, 760.0))
            .default_height(window_height(ctx, 640.0, false))
            .show(ctx, |ui| {
                let mut selected = self.settings_tab;
                settings_window_body(ui, &mut open, &mut selected, |ui, tab| match tab {
                    SettingsTab::Startup => self.draw_settings_startup_tab(ui),
                    SettingsTab::Browser => self.draw_settings_browser_tab(ui),
                    SettingsTab::EditingKits => self.draw_settings_editing_kits_tab(ui),
                    SettingsTab::Appearance => self.draw_settings_appearance_tab(ui),
                    SettingsTab::Tools => self.draw_settings_tools_tab(ui),
                });
                self.settings_tab = selected;
            });
        if !open {
            self.pending_ui_scale = self.prefs.ui_scale;
        }
        self.settings_open = open;
        self.draw_custom_editing_kit_dialog(ctx);
        self.draw_custom_editing_kit_removal_dialog(ctx);
    }

    pub(super) fn set_editing_kit_path_input(
        &mut self,
        shortcut: EditingKitShortcut,
        input: String,
    ) {
        let trimmed = input.trim().to_owned();
        if trimmed.is_empty() {
            self.prefs.editing_kit_paths.remove(shortcut.game);
        } else {
            self.prefs
                .editing_kit_paths
                .insert(shortcut.game.to_owned(), PathBuf::from(&trimmed));
        }
        self.editing_kit_path_inputs
            .insert(shortcut.game.to_owned(), input);
        if self.editing_kit_path_attention.as_deref() == Some(shortcut.game) && !trimmed.is_empty()
        {
            self.editing_kit_path_attention = None;
        }
        self.refresh_builtin_editing_kit_validation(shortcut);
    }

    pub(super) fn draw_settings_startup_tab(&mut self, ui: &mut Ui) {
        ui.label(
            RichText::new("When reopening Baboon with a previous session:").color(text_dark()),
        );
        ui.add_space(2.0);
        ui.radio_value(
            &mut self.prefs.session_restore,
            SessionRestore::Ask,
            "Ask which windows to reopen",
        );
        ui.radio_value(
            &mut self.prefs.session_restore,
            SessionRestore::Always,
            "Reopen the last session automatically",
        );
        ui.radio_value(
            &mut self.prefs.session_restore,
            SessionRestore::Never,
            "Start fresh (never reopen)",
        );

        ui.add_space(10.0);
        ui.separator();
        ui.label(RichText::new("Saving").color(text_dark()).strong());
        ui.add_space(4.0);
        if self.prefs.expert_mode {
            ui.checkbox(
                &mut self.prefs.confirm_container_overwrite,
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
                &mut self.prefs.enable_chimp,
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
            let output = self
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
                self.prefs.chimp_output_dir = Some(path);
            }
            if self.prefs.chimp_output_dir.is_some() && ui.button("Use default").clicked() {
                self.prefs.chimp_output_dir = None;
            }
        });
        if chimp_changed {
            let dirty = self
                .kits
                .iter()
                .any(|kit| kit.chimp.documents.values().any(|document| document.dirty));
            if !self.prefs.enable_chimp && dirty {
                self.prefs.enable_chimp = true;
                self.status =
                    "Build the Chimp mod before disabling a workspace with recovered edits."
                        .to_owned();
                return;
            }
            if self.prefs.enable_chimp {
                let indices: Vec<usize> = self
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
                    self.begin_chimp_mount(index, ui.ctx().clone());
                }
            } else {
                for kit in &mut self.kits {
                    kit.surface = KitSurface::Tags;
                    kit.chimp = ChimpState::default();
                }
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.label(RichText::new("Runtime poking").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.prefs.confirm_runtime_poke,
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
        self.draw_update_channel_picker(ui);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Check now").clicked() {
                let ctx = ui.ctx().clone();
                self.begin_check_for_updates(ctx, false);
            }
            self.draw_update_check_result(ui);
        });
    }

    /// Radio rows for which build track update checks follow, plus whether the
    /// check runs at startup.
    /// Shared by Settings and the first-run wizard so the two cannot drift.
    pub(super) fn draw_update_channel_picker(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Check for updates on").color(text_dark()));
        for option in UpdateChannel::ALL {
            if ui
                .radio_value(&mut self.prefs.update_channel, option, option.label())
                .on_hover_text(option.help())
                .changed()
            {
                // The previous channel's verdict says nothing about this one.
                self.available_update = None;
                self.last_update_check = None;
            }
        }
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.prefs.check_updates_on_startup,
            "Check for updates when Baboon starts",
        );
    }

    /// One line describing what the last check concluded, with a link when
    /// there is something to go and get.
    fn draw_update_check_result(&self, ui: &mut Ui) {
        if let Some(update) = self.available_update.as_ref() {
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
            RichText::new(self.update_check_summary())
                .color(subtle_dark())
                .small(),
        );
    }

    /// Radio row for how nested containers in the tag editor start out.
    /// Shared by Settings and the first-run wizard so the two cannot drift.
    pub(super) fn draw_nested_default_picker(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Groups, structs and blocks start").color(text_dark()));
        ui.horizontal(|ui| {
            for option in NestedDefault::ALL {
                ui.radio_value(&mut self.prefs.nested_default, option, option.label())
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

    pub(super) fn draw_settings_browser_tab(&mut self, ui: &mut Ui) {
        ui.checkbox(
            &mut self.prefs.double_click_to_open_tags,
            "Double-click to open tags",
        );
        ui.checkbox(
            &mut self.prefs.folders_before_tags,
            "List subfolders before tags in browser",
        );
        ui.add_space(12.0);
        ui.label(RichText::new("Tag editor").color(text_dark()).strong());
        ui.add_space(4.0);
        self.draw_nested_default_picker(ui);
    }

    pub(super) fn draw_settings_editing_kits_tab(&mut self, ui: &mut Ui) {
        ui.label(
            RichText::new(
                "Add editing kits for quick loading, or auto-detect supported Steam installations.",
            )
            .color(subtle_dark()),
        );
        ui.horizontal(|ui| {
            if icon_text_button(ui, ButtonIcon::Add, "Add Editing Kit", true).clicked() {
                self.custom_editing_kit_draft = Some(CustomEditingKitDraft::new());
            }
            if ui.button("Auto Detect").clicked() {
                self.auto_detect_editing_kit_paths();
            }
            if ui.button("Refresh Status").clicked() {
                self.refresh_editing_kit_validation();
                self.status = "Editing-kit status refreshed".to_owned();
            }
        });
        ui.add_space(6.0);

        if self.prefs.custom_editing_kit_profiles.is_empty() {
            ui.label(RichText::new("No editing kits configured").color(subtle_dark()));
        }
        for profile in self.prefs.custom_editing_kit_profiles.clone() {
            let validation = self.editing_kit_validation.custom(&profile.id);
            let warning = self
                .editing_kit_validation
                .custom_icon_error(&profile.id)
                .map(str::to_owned);
            let texture = self.workspace_banner_texture(ui.ctx(), &profile.game, Some(&profile.id));
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
                        profile.read_only && profile.game != "haloce_evolved",
                    )
                })
                .inner;
            if load {
                self.load_custom_editing_kit_profile(profile.clone(), ui.ctx().clone());
            }
            if edit {
                self.custom_editing_kit_draft = Some(CustomEditingKitDraft::from_profile(&profile));
            }
            if remove {
                self.custom_editing_kit_removal = Some(CustomEditingKitRemoval {
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
            let previous = self.prefs.custom_editing_kit_profiles.clone();
            if reorder_editing_kit_profiles(&mut self.prefs.custom_editing_kit_profiles, &request) {
                let prefs = self.current_prefs();
                if let Err(error) = save_gui_prefs(
                    &prefs,
                    &self.terminal_open_games,
                    self.first_run_wizard.is_none(),
                ) {
                    self.prefs.custom_editing_kit_profiles = previous;
                    self.status = error;
                } else {
                    self.saved_prefs = prefs;
                    self.saved_terminal_open_games = self.terminal_open_games.clone();
                    self.status = "Editing kit order saved".to_owned();
                }
            }
        }
    }

    fn draw_custom_editing_kit_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.custom_editing_kit_draft.take() else {
            return;
        };
        let title = if draft.editing_id.is_some() {
            "Edit Editing Kit"
        } else {
            "Add Editing Kit"
        };
        let mut open = true;
        let custom_texture = draft_editing_kit_icon_texture(ctx, &draft.icon);
        let texture =
            custom_texture.or_else(|| self.game_banner_texture(ctx, &draft.game).cloned());
        let mut actions = EditingKitFormActions::default();
        egui::Window::new(title)
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::new("custom_editing_kit_dialog"))
            .title_bar(false)
            .collapsible(false)
            .auto_sized()
            .default_width(window_width(ctx, 580.0))
            .max_width(window_width(ctx, 580.0))
            .max_height(window_height(ctx, (ctx.content_rect().height() - 32.0).max(0.0), false))
            .scroll([false, true])
            .show(ctx, |ui| {
                super::find::draw_icon_window_header(ui, title, ButtonIcon::Edit, &mut open);
                ui.separator();
                egui::Frame::NONE
                    .inner_margin(ui.spacing().window_margin)
                    .show(ui, |ui| {
                        actions = draw_editing_kit_form(ui, &mut draft, texture.as_ref());
                    });
            });
        let EditingKitFormActions {
            save,
            cancel,
            remove,
        } = actions;

        if remove {
            self.custom_editing_kit_removal = Some(CustomEditingKitRemoval {
                id: draft.editing_id.clone().unwrap(),
                name: draft.name.clone(),
            });
            open = false;
        }
        if save && self.commit_custom_editing_kit_draft(&mut draft) {
            open = false;
        }
        if cancel {
            open = false;
        }
        if open {
            self.custom_editing_kit_draft = Some(draft);
        }
    }

    fn commit_custom_editing_kit_draft(&mut self, draft: &mut CustomEditingKitDraft) -> bool {
        let name = draft.name.trim().to_owned();
        if name.is_empty() {
            draft.error = Some("Enter an editing kit name".to_owned());
            return false;
        }
        let Some(game) = game_for_saved_id(&draft.game).map(|game| game.as_str().to_owned()) else {
            draft.error = Some("Choose a supported editing-kit engine".to_owned());
            return false;
        };
        let root_input = PathBuf::from(draft.root_input.trim());
        let choosable = kit_folders_are_choosable(&game);
        let folder_input = |input: &str| {
            Some(input.trim())
                .filter(|input| choosable && !input.is_empty())
                .map(PathBuf::from)
        };
        let tags_input = folder_input(&draft.tags_folder_input);
        let data_input = folder_input(&draft.data_folder_input);
        let layout = match validate_kit_layout(
            &root_input,
            &game,
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
            &self.prefs.custom_editing_kit_profiles,
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
                        return false;
                    }
                }
            }
        };
        let profile = CustomEditingKitProfile {
            read_only: draft.read_only && game != "haloce_evolved",
            git_tracked: draft.git_tracked && game != "haloce_evolved",
            id: id.clone(),
            name: name.clone(),
            game,
            root: layout.root,
            icon,
            tags_folder,
            data_folder,
        };
        let previous_profiles = self.prefs.custom_editing_kit_profiles.clone();
        let previous = existing_profile;
        if let Some(index) = self
            .prefs
            .custom_editing_kit_profiles
            .iter()
            .position(|existing| existing.id == id)
        {
            self.prefs.custom_editing_kit_profiles[index] = profile.clone();
        } else {
            self.prefs.custom_editing_kit_profiles.push(profile.clone());
        }
        let prefs = self.current_prefs();
        if let Err(error) = save_gui_prefs(&prefs, &self.terminal_open_games, true) {
            self.prefs.custom_editing_kit_profiles = previous_profiles;
            if previous.as_ref().and_then(|profile| profile.icon.as_ref()) != profile.icon.as_ref()
                && let Some(icon) = &profile.icon
            {
                let _ =
                    remove_unreferenced_custom_icon(icon, &self.prefs.custom_editing_kit_profiles);
            }
            draft.error = Some(error);
            return false;
        }
        self.saved_prefs = prefs;
        self.saved_terminal_open_games = self.terminal_open_games.clone();
        self.custom_editing_kit_textures.remove(&id);
        self.custom_editing_kit_texture_failures.remove(&id);
        self.refresh_editing_kit_validation();

        if let Some(previous) = previous {
            let source_changed = previous.game != profile.game
                || !same_recent_path(&previous.root, &profile.root)
                || previous.tags_folder != profile.tags_folder
                || previous.data_folder != profile.data_folder;
            for kit in &mut self.kits {
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
                    &self.prefs.custom_editing_kit_profiles,
                )
            {
                self.status = error;
                return true;
            }
        }
        self.status = format!("Saved editing kit {}", profile.name);
        true
    }

    fn draw_custom_editing_kit_removal_dialog(&mut self, ctx: &egui::Context) {
        let Some(removal) = self.custom_editing_kit_removal.clone() else {
            return;
        };
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
                    removal.name
                ));
                ui.horizontal(|ui| {
                    confirm = ui.button("Remove").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if confirm {
            self.remove_custom_editing_kit_profile(&removal);
            open = false;
        }
        if cancel {
            open = false;
        }
        if !open {
            self.custom_editing_kit_removal = None;
        }
    }

    fn remove_custom_editing_kit_profile(&mut self, removal: &CustomEditingKitRemoval) {
        let previous_profiles = self.prefs.custom_editing_kit_profiles.clone();
        let removed = self
            .prefs
            .custom_editing_kit_profiles
            .iter()
            .find(|profile| profile.id == removal.id)
            .cloned();
        self.prefs
            .custom_editing_kit_profiles
            .retain(|profile| profile.id != removal.id);
        let prefs = self.current_prefs();
        if let Err(error) = save_gui_prefs(&prefs, &self.terminal_open_games, true) {
            self.prefs.custom_editing_kit_profiles = previous_profiles;
            self.status = error;
            return;
        }
        self.saved_prefs = prefs;
        self.saved_terminal_open_games = self.terminal_open_games.clone();
        self.custom_editing_kit_textures.remove(&removal.id);
        self.custom_editing_kit_texture_failures.remove(&removal.id);
        self.refresh_editing_kit_validation();
        for kit in &mut self.kits {
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
                remove_unreferenced_custom_icon(&icon, &self.prefs.custom_editing_kit_profiles)
        {
            self.status = error;
            return;
        }
        self.status = format!("Removed editing kit {}", removal.name);
    }

    pub(super) fn draw_settings_appearance_tab(&mut self, ui: &mut Ui) {
        ui.checkbox(&mut self.prefs.dark_mode, "Dark mode");
        ui.checkbox(&mut self.prefs.angles_in_degrees, "Angles in degrees")
            .on_hover_text(
                "Angle fields hold radians on disk. Guerilla and the other Halo tools show them \
                 in degrees, and so does Baboon — turn this off to read and type the stored \
                 radians instead. Field search, TSV copy/paste and the tag diff follow the same \
                 setting.",
            );
        ui.horizontal(|ui| {
            ui.label(RichText::new("UI scale").color(subtle_dark()));
            ui.add(
                egui::Slider::new(&mut self.pending_ui_scale, MIN_UI_SCALE..=MAX_UI_SCALE)
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
            );
            draw_ui_scale_input(ui, &mut self.pending_ui_scale);
            if ui.button("Apply").clicked() {
                self.prefs.ui_scale = self.pending_ui_scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
                self.status = "UI scale applied".to_owned();
            }
            if ui.button("Reset").clicked() {
                self.pending_ui_scale = DEFAULT_UI_SCALE;
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Model viewport").color(subtle_dark()));
            ui.add(
                egui::Slider::new(
                    &mut self.prefs.model_preview_size,
                    MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
                )
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
            );
            draw_model_viewport_size_input(ui, &mut self.prefs.model_preview_size);
            if ui.button("Reset").clicked() {
                self.prefs.model_preview_size = DEFAULT_MODEL_PREVIEW_SIZE;
            }
        });
        draw_speed_row(
            ui,
            "Scroll speed",
            "How far the mouse wheel or trackpad scrolls lists and panels. \
             100% is the original speed.",
            &mut self.prefs.scroll_speed,
            MIN_SCROLL_SPEED..=MAX_SCROLL_SPEED,
            DEFAULT_SCROLL_SPEED,
        );
        draw_speed_row(
            ui,
            "Zoom speed",
            "How fast the mouse wheel zooms the model and bitmap viewports. \
             100% is the original speed.",
            &mut self.prefs.zoom_speed,
            MIN_ZOOM_SPEED..=MAX_ZOOM_SPEED,
            DEFAULT_ZOOM_SPEED,
        );
    }

    pub(super) fn draw_settings_tools_tab(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Blender").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Path").color(subtle_dark()));
            let path_response = ui
                .add(egui::TextEdit::singleline(&mut self.blender_path_input).desired_width(360.0));
            if lost_focus_once(&path_response)
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
            {
                let trimmed = self.blender_path_input.trim();
                self.prefs.blender_path = if trimmed.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(trimmed))
                };
                self.status = if let Some(path) = &self.prefs.blender_path {
                    format!("Blender path set to {}", path.display())
                } else {
                    "Blender path cleared".to_owned()
                };
            }
            if icon_text_button(ui, ButtonIcon::Browse, "Browse...", true).clicked() {
                self.choose_blender_path();
            }
            if icon_text_button(ui, ButtonIcon::Clear, "Clear", true).clicked() {
                self.prefs.blender_path = None;
                self.blender_path_input.clear();
                self.status = "Blender path cleared".to_owned();
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
                egui::TextEdit::singleline(&mut self.chimp_usmap_path_input)
                    .desired_width(360.0)
                    .hint_text(placeholder_text("Bundled Campaign Evolved USMAP")),
            );
            if lost_focus_once(&path_response)
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
            {
                self.commit_chimp_usmap_path_input(ui.ctx().clone());
            }
            if ui.button("Browse...").clicked() {
                self.choose_chimp_usmap_path(ui.ctx().clone());
            }
        });
        ui.add_space(8.0);
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), BUTTON_HEIGHT),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if self.prefs.chimp_usmap_path.is_some() && ui.button("Use bundled").clicked() {
                    self.apply_chimp_usmap_path(None, ui.ctx().clone());
                }
            },
        );
    }
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
mod tests;
