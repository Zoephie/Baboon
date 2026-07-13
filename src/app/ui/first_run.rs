//! Mandatory first-run setup shown before the normal application shell.

use super::*;

impl Baboon {
    pub(super) fn draw_first_run_wizard(&mut self, ctx: &egui::Context) {
        let Some(page) = self.first_run_wizard.as_ref().map(|state| state.page) else {
            return;
        };

        egui::Window::new("Welcome to Baboon")
            .id(egui::Id::new("first_run_wizard"))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .default_width(720.0)
            .show(ctx, |ui| match page {
                FirstRunPage::Storage => self.draw_first_run_storage(ui),
                FirstRunPage::Interface => self.draw_first_run_interface(ui),
                FirstRunPage::EditingKits => self.draw_first_run_editing_kits(ui),
            });
    }

    fn draw_first_run_storage(&mut self, ui: &mut Ui) {
        ui.heading("Welcome to Baboon");
        ui.label(
            "Welcome to Baboon, the all-in-one tag editor created by Zoephie Sinyard and Camden Smallwood.",
        );
        ui.add_space(12.0);
        ui.label("Choose where Baboon should keep its automatic settings and cache files.");
        ui.add_space(8.0);

        let locked = self
            .first_run_wizard
            .as_ref()
            .and_then(|state| state.committed_storage)
            .is_some();
        let state = self.first_run_wizard.as_mut().expect("wizard exists");
        ui.add_enabled_ui(!locked, |ui| {
            ui.radio_value(
                &mut state.selected_storage,
                Some(crate::storage::StorageMode::Installed),
                "Installed mode (recommended)",
            );
            ui.indent("installed_description", |ui| {
                ui.label("Store preferences, sessions, indexes, keywords, and logs in AppData.");
            });
            ui.add_space(6.0);
            ui.radio_value(
                &mut state.selected_storage,
                Some(crate::storage::StorageMode::Portable),
                "Portable mode",
            );
            ui.indent("portable_description", |ui| {
                ui.label("Store all automatic Baboon state beside the executable.");
            });
        });
        if locked {
            ui.add_space(6.0);
            ui.label(RichText::new("The storage location was saved for this setup.").italics());
        }
        self.draw_first_run_error(ui);
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let selected = self
                .first_run_wizard
                .as_ref()
                .and_then(|state| state.selected_storage);
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Next"))
                .clicked()
            {
                let mode = selected.expect("enabled only with a selection");
                crate::storage::activate(mode);
                match self.save_first_run_checkpoint(false) {
                    Ok(()) => {
                        let state = self.first_run_wizard.as_mut().expect("wizard exists");
                        state.committed_storage = Some(mode);
                        state.page = FirstRunPage::Interface;
                        state.validation_error = None;
                    }
                    Err(error) => {
                        self.first_run_wizard
                            .as_mut()
                            .expect("wizard exists")
                            .validation_error = Some(error);
                    }
                }
            }
        });
    }

    fn draw_first_run_interface(&mut self, ui: &mut Ui) {
        ui.heading("Blender and interface");
        ui.label("Blender is optional. You can change any of these settings later.");
        ui.add_space(10.0);
        ui.label(RichText::new("Blender executable").strong());
        ui.horizontal(|ui| {
            if ui
                .add(egui::TextEdit::singleline(&mut self.blender_path_input).desired_width(470.0))
                .changed()
            {
                let value = self.blender_path_input.trim();
                self.blender_path = (!value.is_empty()).then(|| PathBuf::from(value));
            }
            if ui.button("Browse...").clicked() {
                self.choose_blender_path();
            }
            if ui.button("Clear").clicked() {
                self.blender_path = None;
                self.blender_path_input.clear();
            }
        });
        ui.add_space(12.0);
        ui.label(RichText::new("Appearance").strong());
        ui.checkbox(&mut self.dark_mode, "Dark mode");
        ui.horizontal(|ui| {
            ui.label("UI scale");
            if ui
                .add(egui::Slider::new(
                    &mut self.pending_ui_scale,
                    MIN_UI_SCALE..=MAX_UI_SCALE,
                ))
                .changed()
            {
                self.ui_scale = self.pending_ui_scale;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Model viewport size");
            ui.add(egui::Slider::new(
                &mut self.model_preview_size,
                MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
            ));
        });
        ui.add_space(12.0);
        ui.label(RichText::new("Tag browser").strong());
        ui.checkbox(
            &mut self.double_click_to_open_tags,
            "Double-click to open tags",
        );
        ui.checkbox(&mut self.folders_before_tags, "List subfolders before tags");
        self.draw_first_run_error(ui);
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                self.first_run_wizard.as_mut().expect("wizard exists").page = FirstRunPage::Storage;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Next").clicked() {
                    match self.save_first_run_checkpoint(false) {
                        Ok(()) => {
                            let should_detect = !self
                                .first_run_wizard
                                .as_ref()
                                .expect("wizard exists")
                                .editing_kit_detection_ran;
                            if should_detect {
                                self.auto_detect_editing_kit_paths();
                            }
                            let state = self.first_run_wizard.as_mut().expect("wizard exists");
                            state.editing_kit_detection_ran = true;
                            state.validation_error = None;
                            state.page = FirstRunPage::EditingKits;
                        }
                        Err(error) => {
                            self.first_run_wizard
                                .as_mut()
                                .expect("wizard exists")
                                .validation_error = Some(error);
                        }
                    }
                }
            });
        });
    }

    fn draw_first_run_editing_kits(&mut self, ui: &mut Ui) {
        ui.heading("Editing kits");
        ui.label("Detected paths fill only empty entries. Every editing-kit path is optional.");
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                for shortcut in EDITING_KIT_SHORTCUTS {
                    let mut input = self
                        .editing_kit_path_inputs
                        .get(shortcut.game)
                        .cloned()
                        .unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.add_sized([82.0, 20.0], egui::Label::new(shortcut.label));
                        if ui
                            .add(egui::TextEdit::singleline(&mut input).desired_width(430.0))
                            .changed()
                        {
                            self.set_editing_kit_path_input(shortcut, input.clone());
                        }
                        if ui.button("Browse...").clicked() {
                            self.choose_editing_kit_path(shortcut);
                        }
                        if ui.button("Clear").clicked() {
                            self.set_editing_kit_path_input(shortcut, String::new());
                        }
                    });
                }
            });
        self.draw_first_run_error(ui);
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui.button("Back").clicked() {
                self.first_run_wizard.as_mut().expect("wizard exists").page =
                    FirstRunPage::Interface;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Finish").clicked() {
                    match self.save_first_run_checkpoint(true) {
                        Ok(()) => {
                            self.first_run_wizard = None;
                            self.status = "Setup complete".to_owned();
                        }
                        Err(error) => {
                            self.first_run_wizard
                                .as_mut()
                                .expect("wizard exists")
                                .validation_error = Some(error);
                        }
                    }
                }
            });
        });
    }

    fn draw_first_run_error(&self, ui: &mut Ui) {
        if let Some(error) = self
            .first_run_wizard
            .as_ref()
            .and_then(|state| state.validation_error.as_deref())
        {
            ui.add_space(8.0);
            ui.colored_label(Color32::from_rgb(220, 70, 70), error);
        }
    }

    fn save_first_run_checkpoint(&mut self, complete: bool) -> Result<(), String> {
        let prefs = self.current_prefs();
        save_gui_prefs(&prefs, &self.terminal_open_games, complete)?;
        self.saved_prefs = prefs;
        self.saved_terminal_open_games = self.terminal_open_games.clone();
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/first_run.rs"]
mod tests;
