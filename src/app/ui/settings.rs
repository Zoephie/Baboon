//! Preferences window and its settings tabs.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

impl Baboon {
    pub(super) fn draw_settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }

        let mut open = self.settings_open;
        egui::Window::new("Settings")
            .id(egui::Id::new("app_settings"))
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .default_width(760.0)
            .default_height(640.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.settings_tab, SettingsTab::Startup, "Startup");
                    ui.selectable_value(&mut self.settings_tab, SettingsTab::Browser, "Browser");
                    ui.selectable_value(
                        &mut self.settings_tab,
                        SettingsTab::EditingKits,
                        "Editing Kits",
                    );
                    ui.selectable_value(
                        &mut self.settings_tab,
                        SettingsTab::Appearance,
                        "Appearance",
                    );
                    ui.selectable_value(&mut self.settings_tab, SettingsTab::Tools, "Tools");
                });
                ui.separator();
                ui.add_space(8.0);
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    match self.settings_tab {
                        SettingsTab::Startup => self.draw_settings_startup_tab(ui),
                        SettingsTab::Browser => self.draw_settings_browser_tab(ui),
                        SettingsTab::EditingKits => self.draw_settings_editing_kits_tab(ui),
                        SettingsTab::Appearance => self.draw_settings_appearance_tab(ui),
                        SettingsTab::Tools => self.draw_settings_tools_tab(ui),
                    }
                });
            });
        if !open {
            self.pending_ui_scale = self.ui_scale;
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
            self.editing_kit_paths.remove(shortcut.game);
        } else {
            self.editing_kit_paths
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
        ui.label(RichText::new("Startup").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.label(
            RichText::new("When reopening Baboon with a previous session:").color(text_dark()),
        );
        ui.add_space(2.0);
        ui.radio_value(
            &mut self.session_restore,
            SessionRestore::Ask,
            "Ask which windows to reopen",
        );
        ui.radio_value(
            &mut self.session_restore,
            SessionRestore::Always,
            "Reopen the last session automatically",
        );
        ui.radio_value(
            &mut self.session_restore,
            SessionRestore::Never,
            "Start fresh (never reopen)",
        );

        ui.add_space(10.0);
        ui.separator();
        ui.label(RichText::new("Saving").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.confirm_container_overwrite,
            "Confirm before Save overwrites Campaign Evolved game files",
        );
        ui.label(
            RichText::new(
                "Saving a tag loaded from a Campaign Evolved container overwrites the game's pak files in place. Use File \u{2192} Export Mod\u{2026} to bundle changes into a separate mod instead.",
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
                .radio_value(&mut self.update_channel, option, option.label())
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
            &mut self.check_updates_on_startup,
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
                ui.radio_value(&mut self.nested_default, option, option.label())
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
        ui.label(RichText::new("Browser").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.double_click_to_open_tags,
            "Double-click to open tags",
        );
        ui.checkbox(
            &mut self.folders_before_tags,
            "List subfolders before tags in browser",
        );
        ui.add_space(12.0);
        ui.label(RichText::new("Tag editor").color(text_dark()).strong());
        ui.add_space(4.0);
        self.draw_nested_default_picker(ui);
    }

    pub(super) fn draw_settings_editing_kits_tab(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Editing Kits").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Configure each editing-kit root or tags folder, plus the Campaign Evolved game install, for quick loading.",
            )
            .color(subtle_dark()),
        );
        ui.horizontal(|ui| {
            if ui.button("Auto Detect").clicked() {
                self.auto_detect_editing_kit_paths();
            }
            if ui.button("Refresh Status").clicked() {
                self.refresh_editing_kit_validation();
                self.status = "Editing-kit status refreshed".to_owned();
            }
        });
        ui.add_space(6.0);

        for shortcut in EDITING_KIT_SHORTCUTS {
            let attention = self.editing_kit_path_attention.as_deref() == Some(shortcut.game);
            let fill = if attention {
                if is_dark_mode() {
                    Color32::from_rgb(62, 45, 39)
                } else {
                    Color32::from_rgb(255, 226, 212)
                }
            } else {
                Color32::TRANSPARENT
            };
            let texture = self.game_banner_texture(ui.ctx(), shortcut.game).cloned();
            let mut input = self
                .editing_kit_path_inputs
                .get(shortcut.game)
                .cloned()
                .unwrap_or_default();
            let mut changed = false;
            let mut browse = false;
            let mut load = false;
            let mut clear = false;
            let status = self.editing_kit_validation.builtin(shortcut);

            Frame::none()
                .fill(fill)
                .inner_margin(egui::Margin::same(4.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(texture) = &texture {
                            ui.add(
                                egui::Image::new(egui::load::SizedTexture::new(
                                    texture.id(),
                                    Vec2::splat(20.0),
                                ))
                                .fit_to_exact_size(Vec2::splat(20.0)),
                            );
                        } else {
                            ui.label(RichText::new(shortcut.fallback).color(text_dark()).strong());
                        }
                        ui.add_sized(
                            Vec2::new(128.0, 20.0),
                            egui::Label::new(RichText::new(shortcut.label).color(text_dark())),
                        );
                        let hint = if shortcut.game == "haloce_evolved" {
                            "game install or Paks folder"
                        } else {
                            "editing-kit root or tags folder"
                        };
                        changed = ui
                            .add(
                                egui::TextEdit::singleline(&mut input)
                                    .hint_text(hint)
                                    .desired_width(304.0),
                            )
                            .changed();
                        browse = ui.button("Browse...").clicked();
                        load = ui
                            .add_enabled(status.layout().is_some(), egui::Button::new("Load"))
                            .clicked();
                        clear = ui.button("Clear").clicked();
                    });
                    let status_color = match status {
                        EditingKitPathStatus::Ready(_) => Color32::from_rgb(92, 180, 92),
                        EditingKitPathStatus::Invalid(_) => material_delete_text(),
                        EditingKitPathStatus::Unconfigured => subtle_dark(),
                    };
                    ui.label(RichText::new(status.message()).color(status_color).small());
                });

            if changed {
                self.set_editing_kit_path_input(shortcut, input);
            }
            if browse {
                self.choose_editing_kit_path(shortcut);
            }
            if load {
                self.load_editing_kit_shortcut(shortcut, ui.ctx().clone());
            }
            if clear {
                self.editing_kit_paths.remove(shortcut.game);
                self.refresh_builtin_editing_kit_validation(shortcut);
                self.editing_kit_path_inputs
                    .insert(shortcut.game.to_owned(), String::new());
                if self.editing_kit_path_attention.as_deref() == Some(shortcut.game) {
                    self.editing_kit_path_attention = None;
                }
                self.status = format!("{} path cleared", shortcut.label);
            }
        }
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Custom projects").color(text_dark()).strong());
            if ui.button("Add Custom Project...").clicked() {
                self.custom_editing_kit_draft = Some(CustomEditingKitDraft::new());
            }
        });
        ui.label(
            RichText::new(
                "Custom projects require sibling tags and data directories and use an explicitly selected engine.",
            )
            .color(subtle_dark()),
        );
        if self.custom_editing_kit_profiles.is_empty() {
            ui.label(RichText::new("No custom projects configured").color(subtle_dark()));
        }
        let profiles = self.custom_editing_kit_profiles.clone();
        for profile in profiles {
            let validation = self.editing_kit_validation.custom(&profile.id);
            Frame::none()
                .inner_margin(egui::Margin::same(4.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            Vec2::new(220.0, 20.0),
                            egui::Label::new(RichText::new(&profile.name).color(text_dark()).strong()),
                        );
                        ui.label(
                            RichText::new(game_display_name(&profile.game)).color(subtle_dark()),
                        );
                        if ui.small_button("Edit...").clicked() {
                            self.custom_editing_kit_draft =
                                Some(CustomEditingKitDraft::from_profile(&profile));
                        }
                        if ui.small_button("Remove...").clicked() {
                            self.custom_editing_kit_removal = Some(CustomEditingKitRemoval {
                                id: profile.id.clone(),
                                name: profile.name.clone(),
                            });
                        }
                    });
                    ui.label(
                        RichText::new(profile.root.display().to_string())
                            .color(subtle_dark())
                            .small(),
                    );
                    match validation {
                        Ok(_) => {
                            ui.label(
                                RichText::new("Ready")
                                    .color(Color32::from_rgb(92, 180, 92))
                                    .small(),
                            );
                        }
                        Err(error) => {
                            ui.label(RichText::new(error).color(material_delete_text()).small());
                        }
                    }
                    if let Some(error) =
                        self.editing_kit_validation.custom_icon_error(&profile.id)
                    {
                        ui.label(
                            RichText::new(error)
                                .color(Color32::from_rgb(220, 170, 70))
                                .small(),
                        );
                    }
                });
            ui.separator();
        }
    }

    fn draw_custom_editing_kit_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.custom_editing_kit_draft.take() else {
            return;
        };
        let title = if draft.editing_id.is_some() {
            "Edit Custom Project"
        } else {
            "Add Custom Project"
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new(title)
            .id(egui::Id::new("custom_editing_kit_dialog"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                egui::Grid::new("custom_editing_kit_fields")
                    .num_columns(2)
                    .spacing(Vec2::new(10.0, 8.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new("Name").color(text_dark()));
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.name)
                                .desired_width(380.0)
                                .hint_text("Project display name"),
                        );
                        ui.end_row();

                        ui.label(RichText::new("Engine").color(text_dark()));
                        egui::ComboBox::from_id_salt("custom_editing_kit_engine")
                            .selected_text(game_display_name(&draft.game))
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                for (label, game) in SUPPORTED_EK_GAMES {
                                    if *game != "haloce_evolved" {
                                        ui.selectable_value(
                                            &mut draft.game,
                                            (*game).to_owned(),
                                            *label,
                                        );
                                    }
                                }
                            });
                        ui.end_row();

                        ui.label(RichText::new("Root").color(text_dark()));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut draft.root_input)
                                    .desired_width(300.0)
                                    .hint_text("Folder containing tags and data"),
                            );
                            if ui.button("Browse...").clicked()
                                && let Some(path) = rfd::FileDialog::new()
                                    .set_title("Select Custom Editing Kit Root")
                                    .pick_folder()
                            {
                                draft.root_input = path.display().to_string();
                                draft.error = None;
                            }
                        });
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.label(RichText::new("Project icon").color(text_dark()).strong());
                ui.radio_value(
                    &mut draft.icon,
                    CustomEditingKitIconDraft::Default,
                    "Use bundled default project icon",
                );
                ui.horizontal(|ui| {
                    let custom = !matches!(draft.icon, CustomEditingKitIconDraft::Default);
                    if ui.radio(custom, "Use custom PNG").clicked() && !custom {
                        if let Some(existing) = draft
                            .editing_id
                            .as_deref()
                            .and_then(|id| {
                                self.custom_editing_kit_profiles
                                    .iter()
                                    .find(|profile| profile.id == id)
                            })
                            .and_then(|profile| profile.icon.clone())
                        {
                            draft.icon = CustomEditingKitIconDraft::Existing(existing);
                        }
                    }
                    if ui.button("Choose PNG...").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_title("Select Custom Project Icon")
                            .add_filter("PNG image", &["png"])
                            .pick_file()
                    {
                        match validate_custom_icon_source(&path) {
                            Ok((width, height)) => {
                                draft.icon = CustomEditingKitIconDraft::Selected(path);
                                draft.icon_warning =
                                    (width != RECOMMENDED_CUSTOM_ICON_SIZE
                                        || height != RECOMMENDED_CUSTOM_ICON_SIZE)
                                        .then(|| {
                                            format!(
                                                "This image is {width} × {height}; 200 × 200 is recommended."
                                            )
                                        });
                                draft.error = None;
                            }
                            Err(error) => draft.error = Some(error),
                        }
                    }
                });
                ui.label(
                    RichText::new("A 200 × 200 PNG is recommended for correct downsampling.")
                        .color(subtle_dark())
                        .small(),
                );
                match &draft.icon {
                    CustomEditingKitIconDraft::Existing(path)
                    | CustomEditingKitIconDraft::Selected(path) => {
                        ui.label(
                            RichText::new(path.display().to_string())
                                .color(subtle_dark())
                                .small(),
                        );
                    }
                    CustomEditingKitIconDraft::Default => {}
                }
                if let Some(warning) = &draft.icon_warning {
                    ui.label(
                        RichText::new(warning)
                            .color(Color32::from_rgb(220, 170, 70))
                            .small(),
                    );
                }
                if let Some(error) = &draft.error {
                    ui.label(RichText::new(error).color(material_delete_text()));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    save = ui.button("Save").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });

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

    fn commit_custom_editing_kit_draft(
        &mut self,
        draft: &mut CustomEditingKitDraft,
    ) -> bool {
        let name = draft.name.trim().to_owned();
        if name.is_empty() {
            draft.error = Some("Enter a project name".to_owned());
            return false;
        }
        let Some(game) = supported_ek_game_id(&draft.game)
            .filter(|game| *game != "haloce_evolved")
            .map(str::to_owned)
        else {
            draft.error = Some("Choose a supported loose editing-kit engine".to_owned());
            return false;
        };
        let root_input = PathBuf::from(draft.root_input.trim());
        let layout = match validate_custom_editing_kit_layout(&root_input) {
            Ok(layout) => layout,
            Err(error) => {
                draft.error = Some(error);
                return false;
            }
        };
        if custom_profile_root_conflicts(
            &self.custom_editing_kit_profiles,
            draft.editing_id.as_deref(),
            &layout.root,
        ) {
            draft.error =
                Some("Another custom project already uses this editing-kit root".to_owned());
            return false;
        }

        let id = draft
            .editing_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let existing_profile = self
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
            id: id.clone(),
            name: name.clone(),
            game,
            root: layout.root,
            icon,
        };
        let previous_profiles = self.custom_editing_kit_profiles.clone();
        let previous = existing_profile;
        if let Some(index) = self
            .custom_editing_kit_profiles
            .iter()
            .position(|existing| existing.id == id)
        {
            self.custom_editing_kit_profiles[index] = profile.clone();
        } else {
            self.custom_editing_kit_profiles.push(profile.clone());
        }
        let prefs = self.current_prefs();
        if let Err(error) = save_gui_prefs(&prefs, &self.terminal_open_games, true) {
            self.custom_editing_kit_profiles = previous_profiles;
            if previous.as_ref().and_then(|profile| profile.icon.as_ref())
                != profile.icon.as_ref()
                && let Some(icon) = &profile.icon
            {
                let _ = remove_unreferenced_custom_icon(
                    icon,
                    &self.custom_editing_kit_profiles,
                );
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
            let source_changed =
                previous.game != profile.game || !same_recent_path(&previous.root, &profile.root);
            for kit in &mut self.kits {
                if kit.profile.as_ref().is_some_and(|identity| identity.id == id) {
                    if source_changed {
                        kit.profile = None;
                    } else if let Some(identity) = &mut kit.profile {
                        identity.name = profile.name.clone();
                    }
                }
            }
            if previous.icon != profile.icon
                && let Some(old_icon) = previous.icon
                && let Err(error) =
                    remove_unreferenced_custom_icon(&old_icon, &self.custom_editing_kit_profiles)
            {
                self.status = error;
                return true;
            }
        }
        self.status = format!("Saved custom project {}", profile.name);
        true
    }

    fn draw_custom_editing_kit_removal_dialog(&mut self, ctx: &egui::Context) {
        let Some(removal) = self.custom_editing_kit_removal.clone() else {
            return;
        };
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Remove Custom Project?")
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
        let previous_profiles = self.custom_editing_kit_profiles.clone();
        let removed = self
            .custom_editing_kit_profiles
            .iter()
            .find(|profile| profile.id == removal.id)
            .cloned();
        self.custom_editing_kit_profiles
            .retain(|profile| profile.id != removal.id);
        let prefs = self.current_prefs();
        if let Err(error) = save_gui_prefs(&prefs, &self.terminal_open_games, true) {
            self.custom_editing_kit_profiles = previous_profiles;
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
                remove_unreferenced_custom_icon(&icon, &self.custom_editing_kit_profiles)
        {
            self.status = error;
            return;
        }
        self.status = format!("Removed custom project {}", removal.name);
    }

    pub(super) fn draw_settings_appearance_tab(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Appearance").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.checkbox(&mut self.dark_mode, "Dark mode");
        ui.horizontal(|ui| {
            ui.label(RichText::new("UI scale").color(subtle_dark()));
            ui.add(
                egui::Slider::new(&mut self.pending_ui_scale, MIN_UI_SCALE..=MAX_UI_SCALE)
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
            );
            draw_ui_scale_input(ui, &mut self.pending_ui_scale);
            if ui.button("Reset").clicked() {
                self.pending_ui_scale = DEFAULT_UI_SCALE;
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Model viewport").color(subtle_dark()));
            ui.add(
                egui::Slider::new(
                    &mut self.model_preview_size,
                    MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
                )
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
            );
            draw_model_viewport_size_input(ui, &mut self.model_preview_size);
            if ui.button("Reset").clicked() {
                self.model_preview_size = DEFAULT_MODEL_PREVIEW_SIZE;
            }
        });
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Apply UI scale").clicked() {
                self.ui_scale = self.pending_ui_scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
                self.status = "UI scale applied".to_owned();
            }
        });
    }

    pub(super) fn draw_settings_tools_tab(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Blender").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Path").color(subtle_dark()));
            let path_response = ui
                .add(egui::TextEdit::singleline(&mut self.blender_path_input).desired_width(360.0));
            if path_response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                let trimmed = self.blender_path_input.trim();
                self.blender_path = if trimmed.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(trimmed))
                };
                self.status = if let Some(path) = &self.blender_path {
                    format!("Blender path set to {}", path.display())
                } else {
                    "Blender path cleared".to_owned()
                };
            }
            if ui.button("Browse...").clicked() {
                self.choose_blender_path();
            }
        });
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Clear").clicked() {
                self.blender_path = None;
                self.blender_path_input.clear();
                self.status = "Blender path cleared".to_owned();
            }
        });
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

fn ui_scale_percent(ui_scale: f32) -> f32 {
    ui_scale * 100.0
}

fn ui_scale_from_percent(percent: f32) -> f32 {
    (percent / 100.0).clamp(MIN_UI_SCALE, MAX_UI_SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_scale_percentage_conversion_clamps_to_supported_range() {
        assert_eq!(ui_scale_percent(1.25), 125.0);
        assert_eq!(ui_scale_from_percent(125.0), 1.25);
        assert_eq!(ui_scale_from_percent(20.0), MIN_UI_SCALE);
        assert_eq!(ui_scale_from_percent(400.0), MAX_UI_SCALE);
    }
}
