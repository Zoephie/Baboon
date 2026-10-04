//! Mandatory first-run setup shown before the normal application shell.

use super::*;

/// Whether the scale the slider is showing should be applied to the window yet.
///
/// The value this slider edits is the zoom factor of the window the slider is
/// drawn in, so applying it every frame of a drag rescaled the slider under the
/// pointer: the handle slid away from the cursor and the value could not be aimed
/// at all. Deferring to the end of the interaction fixes that.
///
/// Phrased as "the two disagree and nothing is holding the widget" rather than as
/// an event, because the events do not cover it. `drag_stopped()` never fires for
/// a click that moves the handle without crossing the drag threshold, and
/// `changed()` is true only on the frame the value moves — which for a click is
/// the frame the button is still down. Either one alone leaves a value showing in
/// the slider that never reaches the window. A difference, by contrast, cannot be
/// missed: whatever frame the pointer comes up on, it is applied then.
pub(in crate::app) fn commit_ui_scale_now(response: &egui::Response, pending: f32, live: f32) -> bool {
    pending != live && !response.is_pointer_button_down_on()
}

/// What the wizard draws on: a draft of the preferences, the state it owns
/// and what its buttons asked for, held until the draft has been sent.
struct FirstRunDraw<'a> {
    prefs: GuiPrefs,
    wizard: &'a mut FirstRunWizardState,
    app: &'a AppReads<'a>,
    effects: Vec<FirstRunCommand>,
}

/// What the first-run wizard's buttons ask for. Each page's Next (and
/// Finish) saves the preferences so far, moving on only when that worked and
/// otherwise keeping the page up with the reason.
pub(in crate::app) enum FirstRunCommand {
    /// Keep Baboon's state in `mode`'s location, and go on to the interface
    /// page.
    CommitStorage(crate::core::storage::StorageMode),
    /// Go on to the editing kits page, detecting kits the first time.
    LeaveInterface,
    /// Save the setup and close the wizard.
    Finish,
    ChooseBlenderPath,
    SetEditingKitPath(EditingKitShortcut, String),
    ChooseEditingKitPath(EditingKitShortcut),
    /// The update channel changed: forget the last check's verdict.
    ForgetUpdateCheck,
}

impl Baboon {
    pub(in crate::app) fn apply_first_run_command(&mut self, command: FirstRunCommand) {
        match command {
            FirstRunCommand::CommitStorage(mode) => {
                crate::core::storage::activate(mode);
                match self.save_first_run_checkpoint(false) {
                    Ok(()) => {
                        if let Some(state) = self.dialogs.get_mut::<FirstRunWizardState>() {
                            state.committed_storage = Some(mode);
                            state.page = FirstRunPage::Interface;
                            state.validation_error = None;
                        }
                    }
                    Err(error) => self.first_run_failed(error),
                }
            }
            FirstRunCommand::LeaveInterface => match self.save_first_run_checkpoint(false) {
                Ok(()) => {
                    let should_detect = self
                        .dialogs
                        .get::<FirstRunWizardState>()
                        .is_some_and(|state| !state.editing_kit_detection_ran);
                    if should_detect {
                        self.auto_detect_editing_kit_paths();
                    }
                    if let Some(state) = self.dialogs.get_mut::<FirstRunWizardState>() {
                        state.editing_kit_detection_ran = true;
                        state.validation_error = None;
                        state.page = FirstRunPage::EditingKits;
                    }
                }
                Err(error) => self.first_run_failed(error),
            },
            FirstRunCommand::Finish => match self.save_first_run_checkpoint(true) {
                Ok(()) => {
                    self.dialogs.close::<FirstRunWizardState>();
                    self.model.status = "Setup complete".to_owned();
                }
                Err(error) => self.first_run_failed(error),
            },
            FirstRunCommand::ChooseBlenderPath => self.choose_blender_path(),
            FirstRunCommand::SetEditingKitPath(shortcut, input) => {
                self.set_editing_kit_path_input(shortcut, input)
            }
            FirstRunCommand::ChooseEditingKitPath(shortcut) => self.choose_editing_kit_path(shortcut),
            FirstRunCommand::ForgetUpdateCheck => self.forget_update_check(),
        }
    }

    fn first_run_failed(&mut self, error: String) {
        if let Some(state) = self.dialogs.get_mut::<FirstRunWizardState>() {
            state.validation_error = Some(error);
        }
    }

    fn save_first_run_checkpoint(&mut self, complete: bool) -> Result<(), String> {
        let prefs = self.current_prefs();
        save_gui_prefs(&prefs, &self.kit_tools.terminal_open_games, complete)?;
        self.saved_prefs = prefs;
        self.kit_tools.saved_terminal_open_games = self.kit_tools.terminal_open_games.clone();
        Ok(())
    }
}

/// The first-run wizard, while setup is unfinished. Like Settings it
/// edits a draft of the preferences; once drawn, a changed draft is sent
/// first and then what a page's button asked for, which saves the
/// preferences as just set.
impl Dialog for FirstRunWizardState {
    fn show(&mut self, cx: &Ctx, app: &AppReads) -> bool {
        let page = self.page;
        let ctx = cx.egui;
        let mut s = FirstRunDraw {
            prefs: cx.model.prefs.clone(),
            wizard: self,
            app,
            effects: Vec::new(),
        };

        egui::Window::new("Welcome to Baboon")
            .id(egui::Id::new("first_run_wizard"))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .default_width(window_width(ctx, 720.0))
            .show(ctx, |ui| match page {
                FirstRunPage::Storage => draw_first_run_storage(ui, &mut s),
                FirstRunPage::Interface => draw_first_run_interface(ui, &mut s),
                FirstRunPage::EditingKits => draw_first_run_editing_kits(ui, &mut s),
            });
        let FirstRunDraw { prefs, effects, .. } = s;
        if prefs != cx.model.prefs {
            cx.edit_prefs(move |live| *live = prefs);
        }
        for effect in effects {
            cx.send(effect);
        }
        // Finish closes it, once the setup is saved.
        true
    }
}

fn draw_first_run_storage(ui: &mut Ui, s: &mut FirstRunDraw) {
    ui.heading("Welcome to Baboon");
    ui.label(
        "Welcome to Baboon, the all-in-one tag editor created by Zoephie Sinyard and Camden Smallwood.",
    );
    ui.add_space(12.0);
    ui.label("Choose where Baboon should keep its automatic settings and cache files.");
    ui.add_space(8.0);

    let locked = Some(&*s.wizard)
        .and_then(|state| state.committed_storage)
        .is_some();
    let state = &mut *s.wizard;
    ui.add_enabled_ui(!locked, |ui| {
        ui.radio_value(
            &mut state.selected_storage,
            Some(crate::core::storage::StorageMode::Installed),
            "Installed mode (recommended)",
        );
        ui.indent("installed_description", |ui| {
            ui.label("Store preferences, sessions, indexes, keywords, and logs in AppData.");
        });
        ui.add_space(6.0);
        ui.radio_value(
            &mut state.selected_storage,
            Some(crate::core::storage::StorageMode::Portable),
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
    draw_first_run_error(ui, s.wizard);
    ui.add_space(14.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let selected = Some(&*s.wizard).and_then(|state| state.selected_storage);
        if ui
            .add_enabled(selected.is_some(), egui::Button::new("Next"))
            .clicked()
        {
            let mode = selected.expect("enabled only with a selection");
            s.effects.push(FirstRunCommand::CommitStorage(mode));
        }
    });
}

fn draw_first_run_interface(ui: &mut Ui, s: &mut FirstRunDraw) {
    ui.heading("Updates and interface");
    ui.label("Blender is optional. You can change any of these settings later.");
    ui.add_space(10.0);
    ui.label(RichText::new("Updates").strong());
    if draw_update_channel_picker(ui, &mut s.prefs) {
        s.effects.push(FirstRunCommand::ForgetUpdateCheck);
    }
    ui.add_space(12.0);
    ui.label(RichText::new("Blender executable").strong());
    ui.horizontal(|ui| {
        if ui
            .add(egui::TextEdit::singleline(&mut s.wizard.blender_path_input).desired_width(470.0))
            .changed()
        {
            let value = s.wizard.blender_path_input.trim();
            s.prefs.blender_path = (!value.is_empty()).then(|| PathBuf::from(value));
        }
        if ui.button("Browse...").clicked() {
            s.effects.push(FirstRunCommand::ChooseBlenderPath);
        }
        if ui.button("Clear").clicked() {
            s.prefs.blender_path = None;
            s.wizard.blender_path_input.clear();
        }
    });
    ui.add_space(12.0);
    ui.label(RichText::new("Tag editor").strong());
    draw_nested_default_picker(ui, &mut s.prefs.nested_default);
    ui.add_space(12.0);
    ui.label(RichText::new("Appearance").strong());
    ui.checkbox(&mut s.prefs.dark_mode, "Dark mode");
    ui.horizontal(|ui| {
        ui.label("UI scale");
        let response = ui.add(egui::Slider::new(
            &mut s.wizard.pending_ui_scale,
            MIN_UI_SCALE..=MAX_UI_SCALE,
        ));
        if commit_ui_scale_now(&response, s.wizard.pending_ui_scale, s.prefs.ui_scale) {
            s.prefs.ui_scale = s.wizard.pending_ui_scale;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Model viewport size");
        ui.add(egui::Slider::new(
            &mut s.prefs.model_preview_size,
            MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
        ));
    });
    ui.add_space(12.0);
    ui.label(RichText::new("Tag browser").strong());
    ui.checkbox(
        &mut s.prefs.double_click_to_open_tags,
        "Double-click to open tags",
    );
    ui.checkbox(
        &mut s.prefs.folders_before_tags,
        "List subfolders before tags",
    );
    draw_first_run_error(ui, s.wizard);
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if ui.button("Back").clicked() {
            s.wizard.page = FirstRunPage::Storage;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Next").clicked() {
                s.effects.push(FirstRunCommand::LeaveInterface);
            }
        });
    });
}

fn draw_first_run_editing_kits(ui: &mut Ui, s: &mut FirstRunDraw) {
    ui.heading("Editing kits");
    ui.label("Detected paths fill only empty entries. Every editing-kit path is optional.");
    ui.add_space(8.0);
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            for shortcut in EDITING_KIT_SHORTCUTS {
                let mut input = s
                    .app
                    .kit_tools
                    .editing_kit_path_inputs
                    .get(shortcut.game.as_str())
                    .cloned()
                    .unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.add_sized([130.0, 20.0], egui::Label::new(shortcut.label));
                    if ui
                        .add(egui::TextEdit::singleline(&mut input).desired_width(382.0))
                        .changed()
                    {
                        s.effects.push(FirstRunCommand::SetEditingKitPath(shortcut, input.clone()));
                    }
                    if ui.button("Browse...").clicked() {
                        s.effects.push(FirstRunCommand::ChooseEditingKitPath(shortcut));
                    }
                    if ui.button("Clear").clicked() {
                        s.effects.push(FirstRunCommand::SetEditingKitPath(shortcut, String::new()));
                    }
                });
            }
        });
    draw_first_run_error(ui, s.wizard);
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if ui.button("Back").clicked() {
            s.wizard.page = FirstRunPage::Interface;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Finish").clicked() {
                s.effects.push(FirstRunCommand::Finish);
            }
        });
    });
}

fn draw_first_run_error(ui: &mut Ui, wizard: &FirstRunWizardState) {
    if let Some(error) = wizard.validation_error.as_deref() {
        ui.add_space(8.0);
        ui.colored_label(Color32::from_rgb(220, 70, 70), error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_wizard_requires_storage_first() {
        let state = FirstRunWizardState::new(None, &GuiPrefs::default());
        assert_eq!(state.page, FirstRunPage::Storage);
        assert_eq!(state.selected_storage, None);
        assert!(!state.editing_kit_detection_ran);
    }

    #[test]
    fn interrupted_wizard_resumes_after_storage_selection() {
        let state = FirstRunWizardState::new(
            Some(crate::core::storage::StorageMode::Portable),
            &GuiPrefs::default(),
        );
        assert_eq!(state.page, FirstRunPage::Interface);
        assert_eq!(
            state.committed_storage,
            Some(crate::core::storage::StorageMode::Portable)
        );
    }
}
