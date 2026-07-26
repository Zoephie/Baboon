//! About, documentation, and map-name help window.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

impl Baboon {
    pub(super) fn draw_about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }

        let mut open = self.about_open;
        egui::Window::new("Baboon Help")
            .id(egui::Id::new("baboon_help"))
            .collapsible(false)
            .resizable(true)
            .constrain(true)
            .open(&mut open)
            .default_size(Vec2::new(780.0, 560.0))
            .min_size(Vec2::new(520.0, 360.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.help_panel_tab, HelpPanelTab::About, "About");
                    ui.selectable_value(&mut self.help_panel_tab, HelpPanelTab::Doc, "Doc");
                    ui.selectable_value(
                        &mut self.help_panel_tab,
                        HelpPanelTab::Tutorials,
                        "Tutorials",
                    );
                    ui.selectable_value(
                        &mut self.help_panel_tab,
                        HelpPanelTab::ScriptDoc,
                        "Script Doc",
                    );
                    ui.selectable_value(
                        &mut self.help_panel_tab,
                        HelpPanelTab::MapNames,
                        "Map Names",
                    );
                });
                ui.separator();
                ui.add_space(8.0);
                match self.help_panel_tab {
                    HelpPanelTab::About => draw_about_tab(ui),
                    HelpPanelTab::Doc => draw_doc_tab(ui, &self.help_docs),
                    HelpPanelTab::Tutorials => {
                        draw_tutorials_tab(ui, &self.tutorials, &mut self.tutorials_game)
                    }
                    HelpPanelTab::ScriptDoc => self.draw_script_doc_tab(ui),
                    HelpPanelTab::MapNames => draw_map_names_tab(ui, &mut self.map_names_game_tab),
                }
            });
        self.about_open = open;
    }
}

impl Baboon {
    fn draw_script_doc_tab(&mut self, ui: &mut Ui) {
        self.script_docs.ensure_loaded(&locate_help_docs_root());
        if let Some(error) = self.script_docs.error() {
            doc_load_error(ui, &format!("Script documentation failed to load: {error}"));
            return;
        }

        let old_game = self.script_docs.game.clone();
        let old_category = self.script_docs.category;
        let old_network_filter = self.script_docs.network_filter;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Game").color(subtle_dark()));
            egui::ComboBox::from_id_salt("script_docs_game")
                .selected_text(
                    SCRIPT_DOC_GAMES
                        .iter()
                        .find(|(id, _)| *id == self.script_docs.game)
                        .map(|(_, title)| *title)
                        .unwrap_or("Unknown game"),
                )
                .show_ui(ui, |ui| {
                    for (id, title) in SCRIPT_DOC_GAMES {
                        ui.selectable_value(&mut self.script_docs.game, id.to_owned(), title);
                    }
                });
            ui.separator();
            ui.selectable_value(
                &mut self.script_docs.category,
                ScriptDocCategory::Functions,
                "Functions",
            );
            ui.selectable_value(
                &mut self.script_docs.category,
                ScriptDocCategory::Globals,
                "Globals",
            );
            ui.selectable_value(
                &mut self.script_docs.category,
                ScriptDocCategory::Types,
                "Types",
            );
            if self.script_docs.category == ScriptDocCategory::Functions {
                ui.separator();
                ui.label(RichText::new("Network safe").color(subtle_dark()));
                egui::ComboBox::from_id_salt("script_docs_network_safe")
                    .selected_text(match self.script_docs.network_filter {
                        ScriptDocNetworkFilter::All => "All",
                        ScriptDocNetworkFilter::Yes => "Yes",
                        ScriptDocNetworkFilter::Unknown => "Unknown",
                        ScriptDocNetworkFilter::No => "No",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.script_docs.network_filter,
                            ScriptDocNetworkFilter::All,
                            "All",
                        );
                        ui.selectable_value(
                            &mut self.script_docs.network_filter,
                            ScriptDocNetworkFilter::Yes,
                            "Yes",
                        );
                        ui.selectable_value(
                            &mut self.script_docs.network_filter,
                            ScriptDocNetworkFilter::Unknown,
                            "Unknown",
                        );
                        ui.selectable_value(
                            &mut self.script_docs.network_filter,
                            ScriptDocNetworkFilter::No,
                            "No",
                        );
                    });
            }
        });
        let search_changed = ui
            .add(
                egui::TextEdit::singleline(&mut self.script_docs.search)
                    .hint_text("Search names, signatures, descriptions, types, or examples...")
                    .desired_width(f32::INFINITY),
            )
            .changed();
        if old_game != self.script_docs.game
            || old_category != self.script_docs.category
            || old_network_filter != self.script_docs.network_filter
            || search_changed
        {
            self.script_docs.invalidate();
        }
        self.script_docs.refresh();
        ui.add_space(6.0);
        ui.separator();

        let available = ui.available_size();
        let list_width = (available.x * 0.42).clamp(280.0, 390.0);
        let mut clicked = None;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(list_width, available.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{} results", self.script_docs.rows.len()))
                                .color(subtle_dark()),
                        );
                    });
                    ui.separator();
                    let selected = self.script_docs.selected.as_deref();
                    ScrollArea::vertical()
                        .id_salt("script_docs_results")
                        .auto_shrink([false, false])
                        .show_rows(ui, 42.0, self.script_docs.rows.len(), |ui, range| {
                            for index in range {
                                let row = &self.script_docs.rows[index];
                                let response = ui
                                    .allocate_ui(Vec2::new(ui.available_width(), 42.0), |ui| {
                                        let response = ui.selectable_label(
                                            selected == Some(row.key.as_str()),
                                            RichText::new(format!("{}  : {}", row.name, row.kind))
                                                .color(text_dark()),
                                        );
                                        let summary =
                                            row.summary.chars().take(48).collect::<String>();
                                        ui.label(
                                            RichText::new(summary).color(subtle_dark()).small(),
                                        );
                                        response
                                    })
                                    .inner;
                                response.clone().on_hover_text(&row.summary);
                                if response.clicked() {
                                    clicked = Some(row.key.clone());
                                }
                            }
                        });
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), available.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ScrollArea::vertical()
                        .id_salt("script_docs_detail")
                        .auto_shrink([false, false])
                        .show(ui, |ui| match &self.script_docs.detail {
                            Some(detail) => draw_script_doc_detail(ui, detail),
                            None => {
                                ui.label(
                                    RichText::new("Select a result to view its documentation.")
                                        .color(subtle_dark()),
                                );
                            }
                        });
                },
            );
        });
        if let Some(key) = clicked {
            self.script_docs.select(key);
        }
    }
}

fn draw_script_doc_detail(ui: &mut Ui, detail: &ScriptDocDetail) {
    match detail {
        ScriptDocDetail::Function {
            name,
            overloads,
            examples,
        } => {
            ui.heading(RichText::new(name).color(foundation_blue()));
            for (index, overload) in overloads.iter().enumerate() {
                if overloads.len() > 1 {
                    ui.label(
                        RichText::new(format!("Overload {}", index + 1))
                            .color(subtle_dark())
                            .strong(),
                    );
                }
                script_code(ui, &overload.signature);
                ui.label(
                    RichText::new(format!("Returns: {}", overload.return_type))
                        .color(subtle_dark()),
                );
                if !overload.description.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(&overload.description).color(text_dark()))
                            .wrap(),
                    );
                }
                if let Some(network_safe) = &overload.network_safe {
                    ui.label(
                        RichText::new(format!("Network safe: {network_safe}")).color(subtle_dark()),
                    );
                }
                ui.add_space(10.0);
            }
            ui.separator();
            ui.label(RichText::new("Examples").color(foundation_blue()).strong());
            if examples.is_empty() {
                ui.label(
                    RichText::new("No matching usage was found in the supplied HSC examples. Use the documented signature above as syntax.")
                        .color(subtle_dark()),
                );
            } else {
                for example in examples {
                    ui.label(
                        RichText::new(format!("{}:{}", example.source_file, example.source_line))
                            .color(subtle_dark()),
                    );
                    script_code(ui, &example.code);
                    ui.add_space(6.0);
                }
            }
        }
        ScriptDocDetail::Global {
            name,
            value_type,
            signature,
            description,
        } => {
            ui.heading(RichText::new(name).color(foundation_blue()));
            ui.label(RichText::new(format!("Type: {value_type}")).color(subtle_dark()));
            script_code(ui, signature);
            if !description.is_empty() {
                ui.add(egui::Label::new(RichText::new(description).color(text_dark())).wrap());
            } else {
                ui.label(
                    RichText::new("The source document provides no additional description for this external global.")
                        .color(subtle_dark()),
                );
            }
        }
        ScriptDocDetail::Type { name, usages } => {
            ui.heading(RichText::new(name).color(foundation_blue()));
            ui.label(
                RichText::new(
                    "Structural reference from documented signatures and external globals.",
                )
                .color(subtle_dark()),
            );
            ui.add_space(8.0);
            for usage in usages {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [70.0, 18.0],
                        egui::Label::new(RichText::new(&usage.role).color(subtle_dark())),
                    );
                    ui.label(
                        RichText::new(&usage.symbol_name)
                            .color(text_dark())
                            .strong(),
                    );
                });
                script_code(ui, &usage.signature);
                ui.add_space(4.0);
            }
        }
    }
}

fn script_code(ui: &mut Ui, code: &str) {
    Frame::none()
        .fill(if is_dark_mode() {
            Color32::from_rgb(24, 27, 29)
        } else {
            Color32::from_rgb(238, 241, 243)
        })
        .inner_margin(egui::Margin::same(6.0))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(code).monospace().color(text_dark()))
                    .wrap()
                    .selectable(true),
            );
        });
}

fn draw_about_tab(ui: &mut Ui) {
    ui.heading(RichText::new("Baboon").color(text_dark()));
    ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(subtle_dark()));
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("blam-tags created by").color(text_dark()));
        ui.label(
            RichText::new("Camden Smallwood")
                .color(foundation_blue())
                .strong(),
        );
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Baboon created by").color(text_dark()));
        ui.label(
            RichText::new("Zoephie Sinyard")
                .color(foundation_blue())
                .strong(),
        );
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Icons by").color(text_dark()));
        ui.label(RichText::new("Paddy Tee").color(foundation_blue()).strong());
    });
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(8.0);
    ui.label(RichText::new("Source").color(text_dark()).strong());
    ui.hyperlink_to(BABOON_GITHUB_URL, BABOON_GITHUB_URL);
}

fn draw_doc_tab(ui: &mut Ui, docs: &HelpDocsState) {
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match docs {
            HelpDocsState::Loaded(docs) => {
                if let Some(tab) = docs.tab("doc") {
                    for section in &tab.sections {
                        doc_section(ui, section);
                    }
                } else {
                    doc_load_error(ui, "Documentation failed to load: missing doc tab.");
                }
            }
            HelpDocsState::Failed(error) => {
                doc_load_error(ui, &format!("Documentation failed to load: {error}"));
            }
        });
}

fn draw_tutorials_tab(ui: &mut Ui, tutorials: &TutorialsState, selected_game: &mut String) {
    let available = ui.available_size();
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(190.0, available.y),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.label(RichText::new("Games").color(subtle_dark()).strong());
                ui.add_space(4.0);
                ScrollArea::vertical()
                    .id_salt("tutorial_game_sidebar")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for shortcut in EDITING_KIT_SHORTCUTS {
                            if ui
                                .selectable_label(
                                    selected_game == shortcut.game,
                                    game_display_name(shortcut.game),
                                )
                                .clicked()
                            {
                                *selected_game = shortcut.game.to_owned();
                            }
                        }
                    });
            },
        );
        ui.separator();
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), available.y),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.heading(RichText::new(game_display_name(selected_game)).color(text_dark()));
                ui.add_space(6.0);
                ScrollArea::vertical()
                    .id_salt("tutorial_cards")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match tutorials {
                        TutorialsState::Loaded(catalog) => {
                            let entries =
                                catalog.entries_for_game(selected_game).collect::<Vec<_>>();
                            if entries.is_empty() {
                                ui.label(
                                    RichText::new(format!(
                                        "No tutorials are available for {} yet.",
                                        game_display_name(selected_game)
                                    ))
                                    .color(subtle_dark()),
                                );
                            } else {
                                for tutorial in entries {
                                    draw_tutorial_card(ui, tutorial);
                                    ui.add_space(10.0);
                                }
                            }
                        }
                        TutorialsState::Failed(error) => {
                            doc_load_error(
                                ui,
                                &format!("Tutorial catalog failed to load: {error}"),
                            );
                        }
                    });
            },
        );
    });
}

fn draw_tutorial_card(ui: &mut Ui, tutorial: &TutorialEntry) {
    Frame::group(ui.style())
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(&tutorial.creator)
                    .color(subtle_dark())
                    .strong(),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(&tutorial.title)
                        .color(foundation_blue())
                        .font(FontId::proportional(18.0))
                        .strong(),
                )
                .wrap(),
            );
            ui.add_space(8.0);

            let thumbnail_width = ui.available_width().min(720.0).max(1.0);
            let thumbnail_size = Vec2::new(thumbnail_width, thumbnail_width * 9.0 / 16.0);
            let response = match &tutorial.thumbnail_texture {
                Some(texture) => ui.add(
                    egui::Image::new(egui::load::SizedTexture::new(
                        texture.id(),
                        texture.size_vec2(),
                    ))
                    .fit_to_exact_size(thumbnail_size)
                    .rounding(6.0)
                    .sense(Sense::click()),
                ),
                None => {
                    let (rect, response) = ui.allocate_exact_size(thumbnail_size, Sense::click());
                    ui.painter()
                        .rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Thumbnail unavailable",
                        FontId::proportional(14.0),
                        subtle_dark(),
                    );
                    response
                }
            };

            draw_tutorial_play_overlay(ui, response.rect, response.hovered());
            let mut response = response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Watch on YouTube");
            if let Some(error) = tutorial.thumbnail_error.as_deref() {
                response = response.on_hover_text(error);
            }
            if response.clicked() {
                open_tutorial_url(ui.ctx(), &tutorial.url);
            }

            ui.add_space(8.0);
            if ui.button("Watch on YouTube").clicked() {
                open_tutorial_url(ui.ctx(), &tutorial.url);
            }
        });
}

fn draw_tutorial_play_overlay(ui: &Ui, rect: egui::Rect, hovered: bool) {
    let center = rect.center();
    let radius = 28.0;
    let background = if hovered {
        Color32::from_black_alpha(220)
    } else {
        Color32::from_black_alpha(185)
    };
    ui.painter().circle_filled(center, radius, background);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            center + Vec2::new(-7.0, -11.0),
            center + Vec2::new(-7.0, 11.0),
            center + Vec2::new(12.0, 0.0),
        ],
        Color32::WHITE,
        Stroke::NONE,
    ));
}

fn open_tutorial_url(ctx: &egui::Context, url: &str) {
    ctx.open_url(egui::OpenUrl::new_tab(url));
}

fn doc_section(ui: &mut Ui, section: &HelpDocSection) {
    ui.label(
        RichText::new(&section.title)
            .color(foundation_blue())
            .font(FontId::proportional(14.0))
            .strong(),
    );
    ui.add_space(4.0);
    for block in &section.blocks {
        match block {
            HelpDocBlock::Paragraph { text } => {
                ui.add(
                    egui::Label::new(RichText::new(text).color(text_dark()))
                        .wrap()
                        .selectable(false),
                );
                ui.add_space(4.0);
            }
            HelpDocBlock::Bullets { items } => {
                for item in items {
                    doc_bullet(ui, item);
                }
            }
        }
    }
    ui.add_space(12.0);
}

fn doc_bullet(ui: &mut Ui, line: &str) {
    ui.horizontal_top(|ui| {
        ui.label(RichText::new("-").color(subtle_dark()));
        ui.add(
            egui::Label::new(RichText::new(line).color(text_dark()))
                .wrap()
                .selectable(false),
        );
    });
}

fn doc_load_error(ui: &mut Ui, message: &str) {
    ui.add(
        egui::Label::new(RichText::new(message).color(text_dark()))
            .wrap()
            .selectable(false),
    );
}

#[cfg(test)]
mod tutorial_ui_tests {
    use super::*;

    #[test]
    fn tutorial_tab_renders_at_minimum_size_in_both_themes() {
        let ctx = egui::Context::default();
        let tutorials = TutorialsState::load(&ctx);

        for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
            ctx.set_visuals(visuals);
            let mut selected_game = "haloce_evolved".to_owned();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(520.0, 360.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        draw_tutorials_tab(ui, &tutorials, &mut selected_game);
                    });
                },
            );
            assert!(!output.shapes.is_empty());
        }
    }

    #[test]
    fn tutorial_url_requests_a_new_browser_tab() {
        let ctx = egui::Context::default();
        let url = "https://www.youtube.com/watch?v=2xL2AiuaFwE";
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            open_tutorial_url(ctx, url);
        });
        let request = output
            .platform_output
            .open_url
            .expect("tutorial action should request an external URL");
        assert_eq!(request.url, url);
        assert!(request.new_tab);
    }
}
