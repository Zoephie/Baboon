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
    LoadCustomKit(CustomEditingKitProfile),
    OpenAbout,
    OpenSettings,
    OpenUrl(&'static str),
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
        // A load reserves the kit (`requested_path`) before its worker starts
        // and installs the source only when it lands — that window is "this
        // workspace is starting up". Replace the whole screen with a wait
        // notice for its duration: a second click on H3EK while the first was
        // still indexing queued a duplicate load, and nothing on this screen
        // is safe to offer until the kit is in.
        if self.kits[kit_index].source.is_none()
            && let Some(path) = self.kits[kit_index].requested_path.clone()
        {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(format!("Please wait — {name} is starting up…"))
                            .color(text_dark())
                            .size(16.0),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Large editing kits can take a moment to index.")
                            .color(subtle_dark()),
                    );
                });
            });
            return;
        }

        let mut action = None;
        let recents = self.recent_folders.clone();
        let editing_kits = visible_editing_kit_menu_entries(
            &self.custom_editing_kit_profiles,
            &self.editing_kit_validation,
        );

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(32.0);
                ui.vertical_centered(|ui| {
                    const WELCOME_CARD_WIDTH: f32 = 820.0;
                    const BANNER_ASPECT_RATIO: f32 = 1680.0 / 320.0;

                    let card_width = WELCOME_CARD_WIDTH.min(ui.available_width());
                    ui.set_width(card_width);

                    Frame::none()
                        .fill(foundation_group_bg())
                        .stroke(Stroke::new(1.0, foundation_group_edge()))
                        .show(ui, |ui| {
                            let content_item_spacing_y = ui.spacing().item_spacing.y;
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let banner_width = ui.available_width();
                            let banner = ui.add(
                                egui::Image::from_bytes(
                                    "bytes://baboon_branding/welcome-banner.png",
                                    include_bytes!("../../../assets/branding/welcome-banner.png")
                                        .as_slice(),
                                )
                                .fit_to_exact_size(Vec2::new(
                                    banner_width,
                                    banner_width / BANNER_ASPECT_RATIO,
                                )),
                            );
                            let banner_scale = banner.rect.width() / WELCOME_CARD_WIDTH;
                            ui.painter().text(
                                banner.rect.left_top()
                                    + Vec2::new(315.0 * banner_scale, 31.0 * banner_scale),
                                egui::Align2::LEFT_TOP,
                                format!("v{}", env!("CARGO_PKG_VERSION")),
                                FontId::proportional(12.0 * banner_scale.max(0.8)),
                                Color32::from_rgb(255, 190, 151),
                            );

                            Frame::none()
                                .inner_margin(egui::Margin {
                                    left: 0.0,
                                    right: 0.0,
                                    top: 0.0,
                                    bottom: 0.0,
                                })
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = content_item_spacing_y;
                                    ui.columns(2, |columns| {
                                        Frame::none()
                                            .fill(Color32::from_black_alpha(51))
                                            .inner_margin(egui::Margin::same(28.0))
                                            .show(&mut columns[0], |ui| {
                                                section_heading(ui, "Start", text_dark());
                                                if welcome_icon_button(
                                                    ui,
                                                    ButtonIcon::FolderOpen,
                                                    "Open a tags folder…",
                                                    text_dark(),
                                                )
                                                .clicked()
                                                {
                                                    action = Some(WelcomeAction::LoadFolder);
                                                }
                                                if welcome_icon_button(
                                                    ui,
                                                    ButtonIcon::Tag,
                                                    "Open a single tag…",
                                                    text_dark(),
                                                )
                                                .clicked()
                                                {
                                                    action = Some(WelcomeAction::LoadTag);
                                                }
                                                if welcome_icon_button(
                                                    ui,
                                                    ButtonIcon::Cache,
                                                    "Open a monolithic cache…",
                                                    text_dark(),
                                                )
                                                .clicked()
                                                {
                                                    action = Some(WelcomeAction::LoadMonolithic);
                                                }
                                                if welcome_icon_button(
                                                    ui,
                                                    ButtonIcon::Container,
                                                    "Open a Campaign Evolved container…",
                                                    text_dark(),
                                                )
                                                .clicked()
                                                {
                                                    action = Some(WelcomeAction::LoadContainer);
                                                }

                                                if !editing_kits.is_empty() {
                                                    ui.add_space(24.0);
                                                    section_heading(ui, "Projects", text_dark());
                                                    for entry in &editing_kits {
                                                        match entry {
                                                            EditingKitMenuEntry::Custom(
                                                                profile,
                                                            ) => {
                                                                let validation = self
                                                                    .editing_kit_validation
                                                                    .custom(&profile.id);
                                                                let enabled = validation.is_ok();
                                                                let tooltip = validation
                                                            .as_ref()
                                                            .map(|layout| {
                                                                layout.root.display().to_string()
                                                            })
                                                            .unwrap_or_else(|error| {
                                                                format!(
                                                                    "{} is unavailable: {error}",
                                                                    profile.name
                                                                )
                                                            });
                                                                let texture = self
                                                                    .custom_editing_kit_texture(
                                                                        ctx, profile,
                                                                    )
                                                                    .cloned();
                                                                let image = match texture {
                                                            Some(texture) => egui::Image::new(
                                                                egui::load::SizedTexture::new(
                                                                    texture.id(),
                                                                    Vec2::splat(16.0),
                                                                ),
                                                            ),
                                                            None => button_icon_image(
                                                                ui,
                                                                ButtonIcon::FolderOpen,
                                                                text_dark(),
                                                                16.0,
                                                            ),
                                                        };
                                                                let response = welcome_image_button(
                                                                    ui,
                                                                    image,
                                                                    &profile.name,
                                                                    text_dark(),
                                                                    enabled,
                                                                );
                                                                let clicked = if enabled {
                                                                    response.on_hover_text(tooltip)
                                                                } else {
                                                                    response.on_disabled_hover_text(
                                                                        tooltip,
                                                                    )
                                                                }
                                                                .clicked();
                                                                if clicked {
                                                                    action =
                                                                Some(WelcomeAction::LoadCustomKit(
                                                                    profile.clone(),
                                                                ));
                                                                }
                                                            }
                                                            EditingKitMenuEntry::BuiltIn(
                                                                shortcut,
                                                            ) => {
                                                                let texture = self
                                                                    .game_emblem_texture(
                                                                        ctx,
                                                                        shortcut.game,
                                                                    )
                                                                    .cloned();
                                                                let path = self
                                                                    .editing_kit_paths
                                                                    .get(shortcut.game)
                                                                    .cloned();
                                                                let image = texture.map_or_else(
                                                                    || {
                                                                        button_icon_image(
                                                                            ui,
                                                                            ButtonIcon::FolderOpen,
                                                                            text_dark(),
                                                                            16.0,
                                                                        )
                                                                    },
                                                                    |texture| {
                                                                        egui::Image::new(
                                                                    egui::load::SizedTexture::new(
                                                                        texture.id(),
                                                                        Vec2::splat(16.0),
                                                                    ),
                                                                )
                                                                    },
                                                                );
                                                                let label = welcome_image_button(
                                                                    ui,
                                                                    image,
                                                                    game_display_name(
                                                                        shortcut.game,
                                                                    ),
                                                                    text_dark(),
                                                                    true,
                                                                );
                                                                let clicked = match &path {
                                                                    Some(path) => label
                                                                        .on_hover_text(
                                                                            path.display()
                                                                                .to_string(),
                                                                        ),
                                                                    None => label,
                                                                }
                                                                .clicked();
                                                                if clicked {
                                                                    action = Some(
                                                                        WelcomeAction::LoadKit(
                                                                            *shortcut,
                                                                        ),
                                                                    );
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            });

                                        Frame::none().inner_margin(egui::Margin::same(28.0)).show(
                                            &mut columns[1],
                                            |ui| {
                                                section_heading(ui, "Recent", text_dark());
                                                if recents.is_empty() {
                                                    ui.label(
                                                        RichText::new(
                                                            "Folders you open will appear here.",
                                                        )
                                                        .color(subtle_dark()),
                                                    );
                                                }
                                                for path in &recents {
                                                    let name = path
                                                        .file_name()
                                                        .map(|name| {
                                                            name.to_string_lossy().into_owned()
                                                        })
                                                        .unwrap_or_else(|| {
                                                            path.display().to_string()
                                                        });
                                                    let full_path = path.display().to_string();
                                                    let text_width =
                                                        (ui.available_width() - 28.0).max(80.0);
                                                    let display_name =
                                                        truncate_for_cell(&name, text_width);
                                                    let row_rect = egui::Rect::from_min_size(
                                                        ui.cursor().min,
                                                        Vec2::new(
                                                            ui.available_width(),
                                                            BUTTON_HEIGHT,
                                                        ),
                                                    );
                                                    let row_hovered = ui.input(|input| {
                                                        input.pointer.hover_pos().is_some_and(
                                                            |pos| row_rect.contains(pos),
                                                        )
                                                    });
                                                    ui.horizontal(|ui| {
                                                        let remove_width = BUTTON_HEIGHT;
                                                        let button_width = (ui.available_width()
                                                            - remove_width
                                                            - ui.spacing().item_spacing.x)
                                                            .max(0.0);
                                                        let image = welcome_recent_icon(ui, path);
                                                        let open_clicked = ui
                                                            .allocate_ui(
                                                                Vec2::new(
                                                                    button_width,
                                                                    BUTTON_HEIGHT,
                                                                ),
                                                                |ui| {
                                                                    ui.set_width(button_width);
                                                                    welcome_image_button(
                                                                        ui,
                                                                        image,
                                                                        &display_name,
                                                                        text_dark(),
                                                                        true,
                                                                    )
                                                                },
                                                            )
                                                            .inner
                                                            .on_hover_text(&full_path)
                                                            .clicked();
                                                        if open_clicked {
                                                            action =
                                                                Some(WelcomeAction::LoadRecent(
                                                                    path.clone(),
                                                                ));
                                                        }
                                                        if row_hovered {
                                                            if ui
                                                                .add_sized(
                                                                    Vec2::splat(remove_width),
                                                                    egui::Button::new("×")
                                                                        .frame(false),
                                                                )
                                                                .on_hover_text(
                                                                    "Remove from recent folders",
                                                                )
                                                                .clicked()
                                                            {
                                                                action = Some(
                                                                    WelcomeAction::ForgetRecent(
                                                                        path.clone(),
                                                                    ),
                                                                );
                                                            }
                                                        } else {
                                                            ui.allocate_space(Vec2::splat(
                                                                remove_width,
                                                            ));
                                                        }
                                                    });
                                                    ui.add_space(3.0);
                                                }
                                                if !recents.is_empty() {
                                                    ui.add_space(8.0);
                                                    if welcome_icon_button(
                                                        ui,
                                                        ButtonIcon::Clear,
                                                        "Clear recent folders",
                                                        text_dark(),
                                                    )
                                                    .clicked()
                                                    {
                                                        action =
                                                            Some(WelcomeAction::ForgetAllRecents);
                                                    }
                                                }
                                                ui.add_space(24.0);
                                                section_heading(ui, "Misc.", text_dark());
                                                if welcome_icon_button(
                                                    ui,
                                                    ButtonIcon::About,
                                                    "About…",
                                                    text_dark(),
                                                )
                                                .clicked()
                                                {
                                                    action = Some(WelcomeAction::OpenAbout);
                                                }
                                                if welcome_icon_button(
                                                    ui,
                                                    ButtonIcon::Settings,
                                                    "Settings…",
                                                    text_dark(),
                                                )
                                                .clicked()
                                                {
                                                    action = Some(WelcomeAction::OpenSettings);
                                                }
                                                if welcome_icon_button(
                                                    ui,
                                                    ButtonIcon::Open,
                                                    "Baboon GitHub",
                                                    text_dark(),
                                                )
                                                .clicked()
                                                {
                                                    action = Some(WelcomeAction::OpenUrl(
                                                        BABOON_GITHUB_URL,
                                                    ));
                                                }
                                                if welcome_icon_button(
                                                    ui,
                                                    ButtonIcon::Group,
                                                    "Halo Mods Discord",
                                                    text_dark(),
                                                )
                                                .clicked()
                                                {
                                                    action = Some(WelcomeAction::OpenUrl(
                                                        "https://discord.com/invite/4pKEpNW",
                                                    ));
                                                }
                                            },
                                        );
                                    });
                                });
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
            Some(WelcomeAction::LoadCustomKit(profile)) => {
                self.load_custom_editing_kit_profile(profile, ctx.clone());
            }
            Some(WelcomeAction::OpenAbout) => {
                self.help_panel_tab = HelpPanelTab::About;
                self.about_open = true;
            }
            Some(WelcomeAction::OpenSettings) => {
                self.settings_tab = SettingsTab::EditingKits;
                self.settings_open = true;
            }
            Some(WelcomeAction::OpenUrl(url)) => {
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            None => {}
        }
    }
}

fn section_heading(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(
        RichText::new(text)
            .color(color.gamma_multiply(0.6))
            .strong()
            .size(12.0),
    );
    ui.add_space(6.0);
}

fn welcome_image_button(
    ui: &mut Ui,
    image: egui::Image<'static>,
    text: &str,
    color: Color32,
    enabled: bool,
) -> egui::Response {
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
        ui.add_enabled(
            enabled,
            egui::Button::image_and_text(image, RichText::new(text).color(color))
                .min_size(Vec2::new(ui.available_width(), BUTTON_HEIGHT)),
        )
    })
    .inner
}

fn welcome_icon_button(
    ui: &mut Ui,
    icon: ButtonIcon,
    text: &str,
    color: Color32,
) -> egui::Response {
    let image = button_icon_image(ui, icon, color, BUTTON_ICON_SIZE);
    welcome_image_button(ui, image, text, color, true)
}

fn welcome_recent_icon(ui: &Ui, path: &std::path::Path) -> egui::Image<'static> {
    if path.is_dir() {
        return button_icon_image(ui, ButtonIcon::FolderClosed, text_dark(), 16.0);
    }
    let group_tag = path
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(extension_to_group_tag);
    let group = group_tag
        .map(format_group_tag)
        .unwrap_or_else(|| "default".to_owned());
    egui::Image::from_bytes(
        tag_icon_uri(ui.ctx(), &group),
        get_icon_svg(&group).as_bytes(),
    )
    .fit_to_exact_size(Vec2::splat(16.0))
}
