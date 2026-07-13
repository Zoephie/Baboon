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
