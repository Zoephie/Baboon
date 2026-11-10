//! The New Tag window.
//! It owns presentation and the group and path chosen; creating the
//! tag is [`TagOpsCommand::CreateNewTag`].

use super::*;
use crate::app::editor::{dropdown_wheel_delta, picker_popup_width, picker_results, picker_search_field, COMBO_POPUP_MAX_HEIGHT};

fn group_matches(group: &NewTagGroup, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty() || group.name.to_lowercase().contains(&query)
        || group.extension.to_lowercase().contains(&query)
        || format_group_tag(group.group_tag).to_lowercase().contains(&query)
}

fn path_name_field(ui: &mut Ui, path: &mut String, extension: &str, width: f32) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, BUTTON_HEIGHT), Sense::hover());
    let background = ui.painter().add(egui::Shape::Noop);
    let hint = ui.painter().layout_no_wrap(extension.to_owned(),
        TextStyle::Body.resolve(ui.style()), text_dark().gamma_multiply(0.5));
    let hint_width = hint.size().x.min((width - 80.0).max(0.0));
    let mut edit_rect = rect;
    edit_rect.max.x -= hint_width + 12.0;
    let response = ui.place(edit_rect, crate::app::ui_kit::singleline_text_edit(path)
        .hint_text(placeholder_text("e.g. objects\\props\\new_tag"))
        .prefix(button_icon_image(ui, ButtonIcon::FolderOpen, text_dark(), 16.0))
        .desired_width(edit_rect.width())
        .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
        .vertical_align(egui::Align::Center)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
            left: 4, right: 0, top: 0, bottom: 0,
        })));
    let visuals = ui.style().interact(&response);
    let stroke = if response.has_focus() { ui.visuals().selection.stroke } else { visuals.bg_stroke };
    ui.painter().set(background, egui::epaint::RectShape::new(rect, visuals.corner_radius,
        ui.visuals().text_edit_bg_color(), stroke, egui::StrokeKind::Inside));
    let hint_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - 6.0 - hint_width, rect.top()), rect.right_bottom());
    ui.painter().with_clip_rect(hint_rect).galley(
        egui::pos2(rect.right() - 6.0 - hint.size().x, rect.center().y - hint.size().y * 0.5),
        hint, text_dark().gamma_multiply(0.5));
    response
}

fn group_picker(ui: &mut Ui, dialog: &mut NewTagDialog, width: f32) {
    let popup_id = ui.make_persistent_id("new_tag_group");
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let game = GameId::from_id(&dialog.game);
    let button = if let Some(group) = dialog.groups.get(dialog.selected_group) {
        egui::Button::image_and_text(
            tag_icon_image(ui.ctx(), tag_icon(Some(group.group_tag), game), 16.0)
                .fit_to_exact_size(Vec2::splat(16.0)),
            format!("{} ({})", group.name, format_group_tag(group.group_tag)),
        )
    } else {
        egui::Button::new("No schemas")
    };
    let response = ui.add_enabled_ui(!dialog.groups.is_empty(), |ui| {
        ui.add_sized(Vec2::new(width, BUTTON_HEIGHT), button
            .right_text(button_icon_image(ui, ButtonIcon::Down, text_dark(), BUTTON_ICON_SIZE))
            .truncate())
    }).inner;
    let just_opened = response.clicked() && !open;
    if response.clicked() {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }
    let popup_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    egui::Popup::from_response(&response)
        .id(popup_id)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            picker_popup_width(ui, &response);
            let filter_id = popup_id.with("group_search");
            let mut filter = ui.data(|data| data.get_temp::<String>(filter_id)).unwrap_or_default();
            let changed = picker_search_field(ui, &mut filter, "Search tag groups…", just_opened);
            ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
            ui.separator();
            picker_results(ui, COMBO_POPUP_MAX_HEIGHT, changed, |ui| {
                let mut shown = 0;
                for (index, group) in dialog.groups.iter().enumerate() {
                    if !group_matches(group, &filter) { continue; }
                    shown += 1;
                    let label = format!("{} ({})", group.name, format_group_tag(group.group_tag));
                    let row = ui.add_sized(Vec2::new(ui.available_width(), BUTTON_HEIGHT), egui::Button::image_and_text(
                        tag_icon_image(ui.ctx(), tag_icon(Some(group.group_tag), game), 16.0)
                            .fit_to_exact_size(Vec2::splat(16.0)),
                        label.clone(),
                    ).selected(index == dialog.selected_group)
                        .right_text("")
                        .frame_when_inactive(index == dialog.selected_group)
                        .frame(true)
                        .truncate()).on_hover_text(label);
                    if just_opened && filter.trim().is_empty() && index == dialog.selected_group {
                        row.scroll_to_me(Some(egui::Align::Center));
                    }
                    if row.clicked() {
                        dialog.selected_group = index;
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                }
                if shown == 0 {
                    ui.label(RichText::new("No tag groups match.").color(subtle_dark()));
                }
            });
        });
    if let Some(delta) = dropdown_wheel_delta(ui, &response, popup_open)
        && let Some(next) = combo_scroll_next_index(dialog.selected_group, dialog.groups.len(), delta)
    {
        dialog.selected_group = next;
    }
}

/// The New Tag dialog: a game, a group and where the tag goes. Create sends
/// [`TagOpsCommand::CreateNewTag`], which takes the dialog back from the host
/// and returns it with the reason if the tag cannot be made.
impl Dialog for NewTagDialog {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        let mut refresh_groups = false;
        let mut create = false;
        let mut close_requested = false;
        // Campaign Evolved container sources create the tag in memory (no loose
        // tags folder, no filesystem picker) at a container-relative path.
        // Both read off the kit the dialog was opened for: Create goes there,
        // whichever game has focus while the window is up.
        let is_container = self.kit_is_container(cx.model);
        let tags_root = self.tags_root(cx.model);
        egui::Window::new("New Tag")
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::unique("new_tag_dialog"))
            .collapsible(false)
            .resizable([true, false])
            .min_width(window_width(ctx, 460.0))
            .title_bar(false)
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin {
                left: 20, right: 20, top: 0, bottom: 0,
            }))
            .default_width(window_width(ctx, 620.0))
            .show(ctx, |ui| {
                ui.spacing_mut().window_margin = egui::Margin {
                    left: 20, right: 20, top: 0, bottom: 0,
                };
                crate::app::search::draw_icon_window_header(ui, "New Tag", ButtonIcon::NewTag, &mut open);
                ui.add_space(10.0);
                if !is_container && tags_root.is_none() {
                    ui.label(
                        RichText::new(
                            "Load a loose editing-kit tags folder before creating a tag.",
                        )
                        .color(subtle_dark()),
                    );
                    ui.add_space(8.0);
                }

                let selected_group_before = self.selected_group;
                ui.columns(2, |columns| {
                    let ui = &mut columns[0];
                    ui.label(RichText::new("Game Engine").strong());
                    if is_container {
                        ui.add_sized(Vec2::new(ui.available_width(), BUTTON_HEIGHT),
                            egui::Label::new("Halo: Campaign Evolved"));
                    } else {
                        let before = self.game.clone();
                        let games = crate::app::documents::saving::available_definition_games();
                        let (_, wheel_delta) = combo_box_with_scroll(
                            ui,
                            crate::app::ui_kit::ValueComboBox::from_id_salt("new_tag_game")
                                .selected_text(&self.game)
                                .width(ui.available_width()),
                            |ui| {
                                for game in &games {
                                    ui.selectable_value(&mut self.game, game.clone(), game);
                                }
                            },
                        );
                        if let Some(delta) = wheel_delta {
                            let current = games
                                .iter()
                                .position(|game| game == &self.game)
                                .unwrap_or(0);
                            if let Some(next) = combo_scroll_next_index(current, games.len(), delta)
                            {
                                self.game = games[next].clone();
                            }
                        }
                        if self.game != before {
                            refresh_groups = true;
                        }
                    }
                    let ui = &mut columns[1];
                    ui.label(RichText::new("Tag Group").strong());
                    group_picker(ui, self, ui.available_width());
                });
                if let Some((authorable, note)) = self.authorability.clone() {
                    ui.label(
                        RichText::new(note)
                            .color(if authorable {
                                subtle_dark()
                            } else {
                                Color32::from_rgb(170, 130, 60)
                            })
                            .small(),
                    );
                }
                if self.selected_group != selected_group_before {
                    self.refresh_authorability(cx.model);
                    // The container path is group-independent (the user types it);
                    // only the loose filesystem output path is tied to the group.
                    if !is_container {
                        self.output_path = None;
                    }
                    self.error = None;
                }

                ui.add_space(6.0);
                ui.label(RichText::new("Path & Name").strong());
                ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), BUTTON_HEIGHT),
                    egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Allocate Browse at its actual width before filling the rest.
                    // Guessing its width caused the auto-sized window to grow each frame.
                    if !is_container && icon_text_button(ui, ButtonIcon::Browse, "Browse",
                        tags_root.is_some() && !self.groups.is_empty()).clicked()
                    {
                        self.choose_output_path(cx.model);
                    }
                    let field_width = ui.available_width();
                    let extension = self.groups.get(self.selected_group)
                        .map(|group| format!(".{}", group.extension.trim_start_matches('.')))
                        .unwrap_or_default();
                    let field = path_name_field(ui, &mut self.rel_path, &extension, field_width);
                    if field.changed() {
                        self.output_path = None;
                        self.error = None;
                    }
                });

                if let Some(error) = &self.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(error).color(material_delete_text()));
                }

                ui.add_space(10.0);
                ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 40.0),
                    egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let background = ui.max_rect().expand2(Vec2::new(20.0, 0.0));
                    let mut painter = ui.painter().clone();
                    painter.set_clip_rect(background.intersect(ctx.content_rect()));
                    let mut rounding = ui.visuals().window_corner_radius;
                    rounding.nw = 0;
                    rounding.ne = 0;
                    painter.rect_filled(background, rounding, foundation_block_bar());
                    if ui.button("Cancel").clicked() {
                        close_requested = true;
                    }
                    let can_create = !self.groups.is_empty()
                        && if is_container {
                            self.has_tag_name()
                        } else {
                            tags_root.is_some() && self.has_tag_name()
                        };
                    if icon_text_button(ui, ButtonIcon::Add, "Create", can_create)
                        .on_disabled_hover_text(if self.groups.is_empty() {
                            "No tag groups are available for this game"
                        } else if is_container {
                            "Enter a path for the new tag"
                        } else if tags_root.is_none() {
                            "Load a loose editing-kit tags folder first"
                        } else {
                            "Enter a name for the new tag, or choose a save location"
                        })
                        .clicked()
                    {
                        create = true;
                    }
                });
            });

        if refresh_groups {
            self.refresh_groups(cx.model);
        }
        if create {
            cx.send(TagOpsCommand::CreateNewTag);
        }
        open && !close_requested
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_tag_paths_cannot_overlap_the_extension_hint() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let mut path = "objects\\characters\\a_very_long_folder_name\\".repeat(4);
        path.push_str("new_tag");
        for width in [300.0, 600.0] {
            let output = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    path_name_field(ui, &mut path, ".scenery", width);
                });
            });
            let hint = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == ".scenery" => Some(text),
                _ => None,
            }).expect("the extension stays visible");
            let editable = output.shapes.iter().find(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == path)).expect("editable path");
            assert!(editable.clip_rect.right() < hint.pos.x);
            assert!(hint.galley.job.sections.iter().all(|section|
                section.format.color == text_dark().gamma_multiply(0.5)));
        }
    }

    #[test]
    fn tag_group_search_matches_names_extensions_and_fourcc_case_insensitively() {
        let group = NewTagGroup {
            group_tag: u32::from_be_bytes(*b"bipd"),
            name: "biped".to_owned(),
            extension: "biped_tag".to_owned(),
            schema_path: PathBuf::new(),
        };
        for query in ["", " BIPED ", "BIPD", "_tag"] {
            assert!(group_matches(&group, query), "{query}");
        }
        assert!(!group_matches(&group, "scenery"));
    }

    #[test]
    fn tag_group_popup_filters_visible_rows_and_shows_an_empty_result() {
        let mut app = Baboon::for_test();
        app.open_new_tag_dialog();
        let mut dialog = app.dialogs.close::<NewTagDialog>().unwrap();
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        for (query, expected, excluded) in [
            ("BIPD", "biped (bipd)", "scenery (scen)"),
            ("no_such_tag_group", "No tag groups match.", "biped (bipd)"),
        ] {
            let mut draw = || crate::app::run_ui_test(&ctx, Default::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let popup_id = ui.make_persistent_id("new_tag_group");
                    egui::Popup::open_id(ui.ctx(), popup_id);
                    ui.data_mut(|data| data.insert_temp(popup_id.with("group_search"), query.to_owned()));
                    group_picker(ui, &mut dialog, 320.0);
                });
            });
            // Popups first run an invisible sizing pass.
            let _ = draw();
            let _ = draw();
            let output = draw();
            let painted: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            }).collect();
            assert!(painted.contains(&expected), "{painted:?}");
            assert!(!painted.contains(&excluded), "{painted:?}");
        }
    }
}
