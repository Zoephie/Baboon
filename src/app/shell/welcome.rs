//! The empty-workspace welcome screen.
//! It owns what an unloaded workspace offers the user; loading itself is the `AppAction` it sends.

use super::*;
use super::recents::RecentAction;
use crate::app::shell::frame::editing_kit_title_text;
use crate::app::shell::frame::EditingKitMenuEntry;
use crate::app::shell::frame::visible_editing_kit_menu_entries;

/// Paint the left pane to the taller column's bottom, not just its own content.
fn draw_welcome_columns(ui: &mut Ui, contents: impl FnOnce(&mut [Ui])) {
    let painter = ui.painter().clone();
    let background = painter.add(egui::Shape::Noop);
    let mut left = egui::Rect::NOTHING;
    ui.columns(2, |columns| {
        let bounds = columns[0].max_rect();
        contents(columns);
        let bottom = columns
            .iter()
            .map(|column| column.min_rect().bottom())
            .fold(bounds.top(), f32::max);
        left = egui::Rect::from_min_max(bounds.min, egui::pos2(bounds.right(), bottom));
    });
    painter.set(
        background,
        egui::Shape::rect_filled(left, 0.0, Color32::from_black_alpha(51)),
    );
}

/// What the welcome screen asks the app to do once the frame is drawn.
///
/// Collected while drawing, and sent once the screen is drawn: each becomes an
/// [`AppAction`] (or opens Help, or a link).
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

/// Draw the welcome screen shown in place of a browser and editor when a
/// workspace has nothing loaded.
///
/// The previous empty state was an empty tag browser beside "No tag
/// selected" — two panels, neither of which could do anything about it.
/// Everything here is a way to open something.
/// `kit_index` is the empty workspace this screen is filling. Its actions
/// load into the active kit, and this pane's kit is the one being asked.
pub(in crate::app) fn draw_welcome_screen(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    shell: &mut ShellFeature,
    validation: &EditingKitValidationCache,
) {
    let ctx = cx.egui;
    // A load reserves the kit (`requested_path`) before its worker starts
    // and installs the source only when it lands — that window is "this
    // workspace is starting up". Replace the whole screen with a wait
    // notice for its duration: a second click on H3EK while the first was
    // still indexing queued a duplicate load, and nothing on this screen
    // is safe to offer until the kit is in.
    if cx.model.kits[kit_index].source.is_none()
        && let Some(path) = cx.model.kits[kit_index].requested_path.clone()
    {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        centered_loading_state(
            ui,
            &format!("Please wait — {name} is starting up…"),
            "Large editing kits can take a moment to index.",
        );
        return;
    }

    let mut action = None;
    let recents = cx.model.prefs.recent_folders.clone();
    let editing_kits = visible_editing_kit_menu_entries(
        &cx.model.prefs.custom_editing_kit_profiles,
        validation,
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

                Frame::NONE
                    .fill(foundation_group_bg())
                    .stroke(Stroke::new(1.0_f32, foundation_group_edge()))
                    .show(ui, |ui| {
                        let content_item_spacing_y = ui.spacing().item_spacing.y;
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let banner_width = ui.available_width();
                        let banner = ui.add(
                            egui::Image::from_bytes(
                                "bytes://baboon_branding/welcome-banner.png",
                                include_root_bytes!("assets/branding/welcome-banner.png")
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

                        Frame::NONE
                            .inner_margin(egui::Margin {
                                left: 0,
                                right: 0,
                                top: 0,
                                bottom: 0,
                            })
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = content_item_spacing_y;
                                draw_welcome_columns(ui, |columns| {
                                    Frame::NONE.inner_margin(egui::Margin::same(28)).show(
                                        &mut columns[0],
                                        |ui| {
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
                                                section_heading(
                                                    ui,
                                                    "Editing Kits",
                                                    text_dark(),
                                                );
                                                for entry in &editing_kits {
                                                    match entry {
                                                        EditingKitMenuEntry::Custom(
                                                            profile,
                                                        ) => {
                                                            let validation = validation.custom(&profile.id);
                                                            let enabled = validation.is_ok();
                                                            let tooltip = validation
                                                        .as_ref()
                                                        .map(|layout| {
                                                            profile_location(
                                                                profile,
                                                                Some(layout),
                                                            )
                                                            .display()
                                                            .to_string()
                                                        })
                                                        .unwrap_or_else(|error| {
                                                            format!(
                                                                "{} is unavailable: {error}",
                                                                profile.name
                                                            )
                                                        });
                                                            let texture = shell
                                                                .artwork
                                                                .workspace_banner(
                                                                ctx,
                                                                &cx.model
                                                                    .prefs
                                                                    .custom_editing_kit_profiles,
                                                                profile.game_id(),
                                                                Some(&profile.id),
                                                            );
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
                                                            let title = editing_kit_title_text(
                                                                ui,
                                                                &profile.name,
                                                                profile.read_only
                                                                    && !profile.is_campaign_evolved(),
                                                                TextStyle::Button
                                                                    .resolve(ui.style())
                                                                    .size,
                                                                false,
                                                            );
                                                            let response = welcome_image_button(
                                                                ui,
                                                                image,
                                                                title,
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
                                                        EditingKitMenuEntry::BuiltIn(shortcut) => {
                                                            let texture = shell
                                                                .artwork
                                                                .game_emblem(ctx, shortcut.game);
                                                            let path = cx
                                                                .model.prefs
                                                                .editing_kit_paths
                                                                .get(shortcut.game.as_str())
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
                                                                shortcut.game.display_name(),
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
                                        },
                                    );

                                    Frame::NONE.inner_margin(egui::Margin::same(28)).show(
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
                                                    "Clear Recent Folders",
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
                                                ButtonIcon::GitHub,
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
                                                ButtonIcon::HaloMods,
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

    let Some(action) = action else {
        return;
    };
    // Its actions load into the active kit, and `open_kit_for` reuses the
    // active kit only when it is still an empty workspace; with another
    // game active it would add a third kit and leave this pane empty. So
    // this pane's kit is made active first.
    cx.send(AppAction::FocusKit(cx.model.kits[kit_index].id));
    match action {
        WelcomeAction::LoadFolder => cx.send(AppAction::LoadFolder),
        WelcomeAction::LoadTag => cx.send(AppAction::LoadTag),
        WelcomeAction::LoadMonolithic => cx.send(AppAction::LoadMonolithic),
        WelcomeAction::LoadContainer => cx.send(AppAction::LoadContainer),
        WelcomeAction::LoadRecent(path) => cx.send(AppAction::Recent(RecentAction::Open(path))),
        WelcomeAction::ForgetRecent(path) => cx.send(AppAction::Recent(RecentAction::Forget(path))),
        WelcomeAction::ForgetAllRecents => cx.send(AppAction::Recent(RecentAction::ForgetAll)),
        WelcomeAction::LoadKit(shortcut) => cx.send(AppAction::LoadBuiltInEditingKit(shortcut)),
        WelcomeAction::LoadCustomKit(profile) => cx.send(AppAction::LoadEditingKit(profile)),
        WelcomeAction::OpenAbout => cx.send(HelpCommand::Open(HelpPanelTab::About)),
        WelcomeAction::OpenSettings => cx.send(AppAction::OpenSettings(Some(SettingsTab::EditingKits))),
        WelcomeAction::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
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
    text: impl Into<egui::WidgetText>,
    color: Color32,
    enabled: bool,
) -> egui::Response {
    // A button aligns its contents by the enclosing layout, and a horizontal
    // row centres them; keep the icon and label at the left in any parent.
    let mut layout = *ui.layout();
    if layout.is_horizontal() {
        layout.main_align = egui::Align::Min;
    } else {
        layout.cross_align = egui::Align::Min;
    }
    ui.with_layout(layout, |ui| {
        ui.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
        let text = text.into();
        let text = if matches!(&text, egui::WidgetText::LayoutJob(_)) {
            text
        } else {
            text.color(color)
        };
        ui.add_enabled(
            enabled,
            egui::Button::image_and_text(image, text)
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
    let Some(group) = recent_tag_icon_group(path) else {
        return button_icon_image(ui, ButtonIcon::FolderClosed, text_dark(), 16.0);
    };
    egui::Image::from_bytes(
        tag_icon_uri(ui.ctx(), &group),
        get_icon_svg(&group).as_bytes(),
    )
    .fit_to_exact_size(Vec2::splat(16.0))
}

/// Classify a recent path without touching the filesystem. Welcome rendering
/// runs every frame, and metadata checks can block on stale network paths or
/// disconnected drives.
fn recent_tag_icon_group(path: &std::path::Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(extension_to_group_tag)
        .map(format_group_tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_left_background_reaches_taller_column_bottom() {
        for taller_left in [false, true] {
            let ctx = egui::Context::default();
            let mut bottom = 0.0;
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(820.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        draw_welcome_columns(ui, |columns| {
                            for (index, column) in columns.iter_mut().enumerate() {
                                let height = if (index == 0) == taller_left {
                                    400.0
                                } else {
                                    120.0
                                };
                                column.allocate_exact_size(
                                    Vec2::new(column.available_width(), height),
                                    Sense::hover(),
                                );
                            }
                            bottom = columns
                                .iter()
                                .map(|column| column.min_rect().bottom())
                                .fold(0.0, f32::max);
                        });
                    });
                },
            );
            let background = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == Color32::from_black_alpha(51) => {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .expect("missing left pane background");
            assert_eq!(background.bottom(), bottom);
        }
    }

    /// egui 0.36 aligns a button's icon and label by the enclosing layout,
    /// and a horizontal row centres them; welcome rows keep them at the left.
    #[test]
    fn welcome_rows_keep_their_icon_and_label_at_the_left() {
        for horizontal in [false, true] {
            let ctx = egui::Context::default();
            let mut button = egui::Rect::NOTHING;
            let output = crate::app::run_ui_test(
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
                        let row = |ui: &mut Ui| {
                            ui.set_width(300.0);
                            button = welcome_icon_button(
                                ui,
                                ButtonIcon::FolderClosed,
                                "halo3_mcc",
                                text_dark(),
                            )
                            .rect;
                        };
                        if horizontal {
                            ui.horizontal(row);
                        } else {
                            ui.vertical(row);
                        }
                    });
                },
            );
            let label = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "halo3_mcc" => {
                        Some(text.visual_bounding_rect())
                    }
                    _ => None,
                })
                .expect("missing row label");
            assert!(
                label.left() - button.left() < 48.0,
                "label starts {} px into a {} px row (horizontal: {horizontal})",
                label.left() - button.left(),
                button.width(),
            );
            assert!(button.y_range().contains(label.center().y));
        }
    }

    #[test]
    fn recent_icon_classification_does_not_require_the_path_to_exist() {
        let missing_tag = std::path::Path::new("Z:/missing/network/path/example.scenario");
        let missing_folder = std::path::Path::new("Z:/missing/network/path/tags");

        assert_eq!(recent_tag_icon_group(missing_tag).as_deref(), Some("scnr"));
        assert_eq!(recent_tag_icon_group(missing_folder), None);
    }
}
