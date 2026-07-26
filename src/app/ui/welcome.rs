//! The empty-workspace welcome screen.
//! It owns what an unloaded workspace offers the user; loading itself belongs to the controller.

use super::*;

/// What the welcome screen asks the app to do once the frame is drawn.
///
/// Collected rather than acted on inline: every one of these borrows `self`
/// mutably, and the screen is drawn from inside a pane closure.
enum WelcomeAction {
    LoadFolder,
    LoadTag,
    LoadMonolithic,
    LoadContainer,
    LoadRecent(std::path::PathBuf),
    LoadKit(EditingKitShortcut),
    OpenSettings,
}

impl Baboon {
    /// Draw the welcome screen shown in place of a browser and editor when a
    /// workspace has nothing loaded.
    ///
    /// The previous empty state was an empty tag browser beside "No tag
    /// selected" — two panels, neither of which could do anything about it.
    /// Everything here is a way to open something.
    pub(in crate::app) fn draw_welcome_screen(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let mut action = None;
        let recents = self.recent_folders.clone();
        let configured: Vec<EditingKitShortcut> = EDITING_KIT_SHORTCUTS
            .into_iter()
            .filter(|shortcut| self.editing_kit_paths.contains_key(shortcut.game))
            .collect();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(56.0);
                ui.vertical_centered(|ui| {
                    ui.set_max_width(820.0);
                    ui.horizontal(|ui| {
                        ui.heading(RichText::new("Baboon").color(text_dark()).size(32.0));
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                                .color(subtle_dark()),
                        );
                    });
                    ui.label(RichText::new("Halo tag editor").color(subtle_dark()));
                    ui.add_space(30.0);

                    ui.columns(2, |columns| {
                        let ui = &mut columns[0];
                        section_heading(ui, "Start");
                        if welcome_link(ui, "Open a tags folder…").clicked() {
                            action = Some(WelcomeAction::LoadFolder);
                        }
                        if welcome_link(ui, "Open a single tag…").clicked() {
                            action = Some(WelcomeAction::LoadTag);
                        }
                        if welcome_link(ui, "Open a monolithic cache…").clicked() {
                            action = Some(WelcomeAction::LoadMonolithic);
                        }
                        if welcome_link(ui, "Open a Campaign Evolved container…").clicked() {
                            action = Some(WelcomeAction::LoadContainer);
                        }

                        if !configured.is_empty() {
                            ui.add_space(24.0);
                            section_heading(ui, "Editing kits");
                            for shortcut in &configured {
                                let texture = self.game_emblem_texture(ctx, shortcut.game).cloned();
                                let path = self.editing_kit_paths.get(shortcut.game).cloned();
                                let clicked = ui
                                    .horizontal(|ui| {
                                        match texture {
                                            Some(texture) => {
                                                ui.add(egui::Image::new(
                                                    egui::load::SizedTexture::new(
                                                        texture.id(),
                                                        Vec2::splat(16.0),
                                                    ),
                                                ));
                                            }
                                            None => ui.add_space(16.0),
                                        }
                                        let label = ui.add(
                                            egui::Label::new(
                                                RichText::new(game_display_name(shortcut.game))
                                                    .color(link_color()),
                                            )
                                            .sense(Sense::click()),
                                        );
                                        match &path {
                                            Some(path) => {
                                                label.on_hover_text(path.display().to_string())
                                            }
                                            None => label,
                                        }
                                        .clicked()
                                    })
                                    .inner;
                                if clicked {
                                    action = Some(WelcomeAction::LoadKit(*shortcut));
                                }
                            }
                        }

                        ui.add_space(24.0);
                        section_heading(ui, "Set up");
                        if welcome_link(ui, "Settings…").clicked() {
                            action = Some(WelcomeAction::OpenSettings);
                        }

                        let ui = &mut columns[1];
                        section_heading(ui, "Recent");
                        if recents.is_empty() {
                            ui.label(
                                RichText::new("Folders you open will appear here.")
                                    .color(subtle_dark()),
                            );
                        }
                        for path in &recents {
                            let name = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.display().to_string());
                            let parent = path
                                .parent()
                                .map(|parent| parent.display().to_string())
                                .unwrap_or_default();
                            let clicked = ui
                                .horizontal(|ui| {
                                    let label = ui
                                        .add(
                                            egui::Label::new(RichText::new(name).color(link_color()))
                                                .sense(Sense::click()),
                                        )
                                        .on_hover_text(path.display().to_string());
                                    ui.label(
                                        RichText::new(parent).color(subtle_dark()).small(),
                                    );
                                    label.clicked()
                                })
                                .inner;
                            if clicked {
                                action = Some(WelcomeAction::LoadRecent(path.clone()));
                            }
                        }
                    });
                });
            });

        match action {
            Some(WelcomeAction::LoadFolder) => self.begin_load_folder(ctx.clone()),
            Some(WelcomeAction::LoadTag) => self.begin_load_single(ctx.clone()),
            Some(WelcomeAction::LoadMonolithic) => self.begin_load_monolithic(ctx.clone()),
            Some(WelcomeAction::LoadContainer) => self.begin_load_iostore_container(ctx.clone()),
            Some(WelcomeAction::LoadRecent(path)) => self.load_recent_folder(path, ctx.clone()),
            Some(WelcomeAction::LoadKit(shortcut)) => {
                self.load_editing_kit_shortcut(shortcut, ctx.clone())
            }
            Some(WelcomeAction::OpenSettings) => self.settings_open = true,
            None => {}
        }
    }
}

fn section_heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(text_dark()).strong().size(15.0));
    ui.add_space(6.0);
}

/// A clickable text row. Deliberately not a button: the screen should read as a
/// short list of ways in, not a wall of controls.
fn welcome_link(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(text).color(link_color())).sense(Sense::click()))
}

fn link_color() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(122, 176, 214)
    } else {
        Color32::from_rgb(38, 108, 158)
    }
}
