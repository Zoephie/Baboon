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
    ForgetRecent(std::path::PathBuf),
    ForgetAllRecents,
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
    /// `kit_index` is the empty workspace this screen is filling. Its actions
    /// load into the active kit, and this pane's kit is the one being asked.
    pub(in crate::app) fn draw_welcome_screen(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
    ) {
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
                    ui.add_space(30.0);

                    // `Ui::link` carries the pointing-hand cursor and the
                    // hover underline; a click-sensing Label looks like a link
                    // but still shows the text-selection cursor.
                    //
                    // `override_text_color` has to be cleared for the link
                    // colour to survive: it is set application-wide, and it
                    // bakes its colour into the galley before `Link` gets to
                    // apply `hyperlink_color`, which is only a fallback for
                    // text that has none. Every other row here sets its own
                    // colour explicitly, so clearing it affects only the links.
                    ui.visuals_mut().override_text_color = None;
                    ui.visuals_mut().hyperlink_color = link_color();

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
                                        let label = ui.link(game_display_name(shortcut.game));
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
                            ui.horizontal(|ui| {
                                if ui
                                    .add(egui::Button::new("×").small().frame(false))
                                    .on_hover_text("Remove from recent folders")
                                    .clicked()
                                {
                                    action = Some(WelcomeAction::ForgetRecent(path.clone()));
                                }
                                if ui
                                    .link(name)
                                    .on_hover_text(path.display().to_string())
                                    .clicked()
                                {
                                    action = Some(WelcomeAction::LoadRecent(path.clone()));
                                }
                                ui.label(RichText::new(parent).color(subtle_dark()).small());
                            });
                        }
                        if !recents.is_empty() {
                            ui.add_space(8.0);
                            if welcome_link(ui, "Clear recent folders").clicked() {
                                action = Some(WelcomeAction::ForgetAllRecents);
                            }
                        }
                    });
                });
            });

        // `open_kit_for` reuses the active kit only when it is still an empty
        // workspace; with another game active it would add a third kit and
        // leave this pane empty. Press-activation normally beats the click by a
        // frame, but not when a frame runs long.
        if action.is_some() {
            self.active = kit_index;
        }
        match action {
            Some(WelcomeAction::LoadFolder) => self.begin_load_folder(ctx.clone()),
            Some(WelcomeAction::LoadTag) => self.begin_load_single(ctx.clone()),
            Some(WelcomeAction::LoadMonolithic) => self.begin_load_monolithic(ctx.clone()),
            Some(WelcomeAction::LoadContainer) => self.begin_load_iostore_container(ctx.clone()),
            Some(WelcomeAction::LoadRecent(path)) => self.load_recent_folder(path, ctx.clone()),
            Some(WelcomeAction::ForgetRecent(path)) => {
                self.remove_recent_folder(&path);
                self.status = format!("Removed {} from recent folders", path.display());
            }
            Some(WelcomeAction::ForgetAllRecents) => {
                self.recent_folders.clear();
                self.status = "Cleared recent folders".to_owned();
            }
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

/// A link row. Deliberately not a button: the screen should read as a short
/// list of ways in, not a wall of controls.
///
/// Wrapped in a horizontal strip because `Ui::columns` lays its children out
/// with `Layout::top_down_justified`, which stretches every widget to the full
/// column width — leaving the link clickable far to the right of where its
/// text ends. A horizontal layout gives each widget its natural width.
fn welcome_link(ui: &mut Ui, text: &str) -> egui::Response {
    ui.horizontal(|ui| ui.link(text)).inner
}

fn link_color() -> Color32 {
    if is_dark_mode() {
        Color32::from_rgb(122, 176, 214)
    } else {
        Color32::from_rgb(38, 108, 158)
    }
}
