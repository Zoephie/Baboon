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
            .resizable(false)
            .open(&mut open)
            .default_width(760.0)
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
                        SettingsTab::EditingKitAliases,
                        "Editing Kit Aliases",
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
                match self.settings_tab {
                    SettingsTab::Startup => self.draw_settings_startup_tab(ui),
                    SettingsTab::Browser => self.draw_settings_browser_tab(ui),
                    SettingsTab::EditingKits => self.draw_settings_editing_kits_tab(ui),
                    SettingsTab::EditingKitAliases => self.draw_settings_aliases_tab(ui),
                    SettingsTab::Appearance => self.draw_settings_appearance_tab(ui),
                    SettingsTab::Tools => self.draw_settings_tools_tab(ui),
                }
            });
        if !open {
            self.pending_ui_scale = self.ui_scale;
        }
        self.settings_open = open;
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
    }

    pub(super) fn draw_settings_editing_kits_tab(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Editing Kits").color(text_dark()).strong());
        ui.add_space(4.0);
        ui.label(
            RichText::new("Configure each editing-kit root or its tags folder for quick loading.")
                .color(subtle_dark()),
        );
        if ui.button("Auto Detect").clicked() {
            self.auto_detect_editing_kit_paths();
        }
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
                            Vec2::new(72.0, 20.0),
                            egui::Label::new(RichText::new(shortcut.label).color(text_dark())),
                        );
                        changed = ui
                            .add(
                                egui::TextEdit::singleline(&mut input)
                                    .hint_text("editing-kit root or tags folder")
                                    .desired_width(360.0),
                            )
                            .changed();
                        browse = ui.button("Browse...").clicked();
                        load = ui.button("Load").clicked();
                        clear = ui.button("Clear").clicked();
                    });
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
                self.editing_kit_path_inputs
                    .insert(shortcut.game.to_owned(), String::new());
                if self.editing_kit_path_attention.as_deref() == Some(shortcut.game) {
                    self.editing_kit_path_attention = None;
                }
                self.status = format!("{} path cleared", shortcut.label);
            }
        }
    }

    pub(super) fn draw_settings_aliases_tab(&mut self, ui: &mut Ui) {
        ui.label(
            RichText::new("Editing Kit Folder Aliases")
                .color(text_dark())
                .strong(),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Map custom kit folder names to a game profile, for example h2rek -> halo2_mcc.",
            )
            .color(subtle_dark()),
        );
        ui.add_space(4.0);
        let mut remove_alias = None;
        let mut aliases_changed = false;
        ui.label(RichText::new("Configured aliases").color(subtle_dark()));
        if self.ek_folder_aliases.is_empty() {
            ui.label(RichText::new("No custom aliases added").color(subtle_dark()));
        }
        for index in 0..self.ek_folder_aliases.len() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Folder").color(subtle_dark()));
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.ek_folder_aliases[index].folder_name)
                            .desired_width(160.0),
                    )
                    .changed()
                {
                    aliases_changed = true;
                }
                let selected_label = ek_game_label(&self.ek_folder_aliases[index].game);
                let (_, wheel_delta) = combo_box_with_scroll(
                    ui,
                    egui::ComboBox::from_id_salt(("ek_folder_alias_game", index))
                        .selected_text(selected_label)
                        .width(210.0),
                    |ui| {
                        for (label, game) in SUPPORTED_EK_GAMES {
                            if ui
                                .selectable_value(
                                    &mut self.ek_folder_aliases[index].game,
                                    (*game).to_owned(),
                                    *label,
                                )
                                .changed()
                            {
                                aliases_changed = true;
                            }
                        }
                    },
                );
                if let Some(delta) = wheel_delta {
                    let current = SUPPORTED_EK_GAMES
                        .iter()
                        .position(|(_, game)| *game == self.ek_folder_aliases[index].game)
                        .unwrap_or(0);
                    if let Some(next) =
                        combo_scroll_next_index(current, SUPPORTED_EK_GAMES.len(), delta)
                    {
                        let game = SUPPORTED_EK_GAMES[next].1.to_owned();
                        self.ek_folder_aliases[index].game = game;
                        aliases_changed = true;
                    }
                }
                ui.label(
                    RichText::new(format!("-> {}", self.ek_folder_aliases[index].game))
                        .color(subtle_dark()),
                );
                if ui.small_button("Remove").clicked() {
                    remove_alias = Some(index);
                }
            });
        }
        if let Some(index) = remove_alias {
            self.ek_folder_aliases.remove(index);
            aliases_changed = true;
            self.status = "Editing kit folder alias removed".to_owned();
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("New").color(subtle_dark()));
            ui.add(
                egui::TextEdit::singleline(&mut self.new_ek_alias_name)
                    .hint_text(
                        RichText::new("example: h2rek")
                            .italics()
                            .color(placeholder_text()),
                    )
                    .desired_width(160.0),
            );
            let (_, wheel_delta) = combo_box_with_scroll(
                ui,
                egui::ComboBox::from_id_salt("new_ek_folder_alias_game")
                    .selected_text(ek_game_label(&self.new_ek_alias_game))
                    .width(210.0),
                |ui| {
                    for (label, game) in SUPPORTED_EK_GAMES {
                        ui.selectable_value(
                            &mut self.new_ek_alias_game,
                            (*game).to_owned(),
                            *label,
                        );
                    }
                },
            );
            if let Some(delta) = wheel_delta {
                let current = SUPPORTED_EK_GAMES
                    .iter()
                    .position(|(_, game)| *game == self.new_ek_alias_game)
                    .unwrap_or(0);
                if let Some(next) =
                    combo_scroll_next_index(current, SUPPORTED_EK_GAMES.len(), delta)
                {
                    self.new_ek_alias_game = SUPPORTED_EK_GAMES[next].1.to_owned();
                }
            }
            if ui.button("Add").clicked() {
                let folder_name = self.new_ek_alias_name.trim().to_owned();
                if folder_name.is_empty() {
                    self.status = "Enter a folder name before adding an alias".to_owned();
                } else if supported_ek_game_id(&self.new_ek_alias_game).is_none() {
                    self.status = "Choose a supported game before adding an alias".to_owned();
                } else if let Some(existing) = self
                    .ek_folder_aliases
                    .iter_mut()
                    .find(|alias| alias.folder_name.trim().eq_ignore_ascii_case(&folder_name))
                {
                    existing.folder_name = folder_name.clone();
                    existing.game = self.new_ek_alias_game.clone();
                    self.new_ek_alias_name.clear();
                    aliases_changed = true;
                    self.status = format!("Updated editing kit alias {folder_name}");
                } else {
                    self.ek_folder_aliases.push(EkFolderAlias {
                        folder_name: folder_name.clone(),
                        game: self.new_ek_alias_game.clone(),
                    });
                    self.new_ek_alias_name.clear();
                    aliases_changed = true;
                    self.status = format!("Added editing kit alias {folder_name}");
                }
            }
        });
        if aliases_changed {
            self.reapply_current_folder_profile();
        }
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
