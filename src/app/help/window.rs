//! About, documentation, and map-name help window.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

/// The Help window: what it shows and the choices made in it. The documents
/// and tutorials are read once per session and shared from [`HelpFeature`].
pub(in crate::app) struct HelpWindow {
    pub(in crate::app) tab: HelpPanelTab,
    docs: Rc<HelpDocsState>,
    tutorials: Rc<TutorialsState>,
    tutorials_game: String,
    tutorials_category: TutorialCategory,
    script_docs: ScriptDocsUiState,
    pub(in crate::app) tag_compat: TagCompatUiState,
    map_names_game_tab: MapNamesGameTab,
}

impl HelpWindow {
    /// A Help window on `tab`, over the session's documents.
    pub(in crate::app) fn new(help: &HelpFeature, tab: HelpPanelTab) -> Self {
        Self {
            tab,
            docs: help.docs.clone(),
            tutorials: help.tutorials.clone(),
            tutorials_game: GameId::CampaignEvolved.as_str().to_owned(),
            tutorials_category: TutorialCategory::ThreeD,
            script_docs: ScriptDocsUiState::default(),
            tag_compat: TagCompatUiState::default(),
            map_names_game_tab: MapNamesGameTab::HaloCe,
        }
    }
}

impl Dialog for HelpWindow {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        egui::Window::new("Baboon Help")
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::new("baboon_help"))
            .collapsible(false)
            .resizable(true)
            .constrain(true)
            .open(&mut open)
            .default_size(window_size(ctx, Vec2::new(780.0, 560.0), true))
            .min_size(window_size(ctx, Vec2::new(520.0, 360.0), true))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, HelpPanelTab::About, "About");
                    ui.selectable_value(&mut self.tab, HelpPanelTab::Doc, "Doc");
                    ui.selectable_value(&mut self.tab, HelpPanelTab::Tutorials, "Tutorials");
                    ui.selectable_value(&mut self.tab, HelpPanelTab::ScriptDoc, "Script Doc");
                    ui.selectable_value(
                        &mut self.tab,
                        HelpPanelTab::TagCompat,
                        "Tag Compatibility",
                    );
                    ui.selectable_value(&mut self.tab, HelpPanelTab::MapNames, "Map Names");
                });
                ui.separator();
                ui.add_space(8.0);
                match self.tab {
                    HelpPanelTab::About => draw_about_tab(ui),
                    HelpPanelTab::Doc => draw_doc_tab(ui, &self.docs),
                    HelpPanelTab::Tutorials => draw_tutorials_tab(
                        ui,
                        &self.tutorials,
                        &mut self.tutorials_game,
                        &mut self.tutorials_category,
                    ),
                    HelpPanelTab::ScriptDoc => draw_script_doc_tab(ui, &mut self.script_docs),
                    HelpPanelTab::TagCompat => draw_tag_compat_tab(cx, ui, &mut self.tag_compat),
                    HelpPanelTab::MapNames => draw_map_names_tab(ui, &mut self.map_names_game_tab),
                }
            });
        open
    }
}

fn draw_script_doc_tab(ui: &mut Ui, docs: &mut ScriptDocsUiState) {
    docs.ensure_loaded(&locate_help_docs_root());
    if let Some(error) = docs.error() {
        doc_load_error(ui, &format!("Script documentation failed to load: {error}"));
        return;
    }

    let old_game = docs.game.clone();
    let old_category = docs.category;
    let old_network_filter = docs.network_filter;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Game").color(subtle_dark()));
        egui::ComboBox::from_id_salt("script_docs_game")
            .selected_text(
                script_doc_games()
                    .find(|game| game.as_str() == docs.game)
                    .map_or("Unknown game", GameFacts::display_name),
            )
            .show_ui(ui, |ui| {
                for game in script_doc_games() {
                    ui.selectable_value(
                        &mut docs.game,
                        game.as_str().to_owned(),
                        game.display_name(),
                    );
                }
            });
        ui.separator();
        ui.selectable_value(
            &mut docs.category,
            ScriptDocCategory::Functions,
            "Functions",
        );
        ui.selectable_value(
            &mut docs.category,
            ScriptDocCategory::Globals,
            "Globals",
        );
        ui.selectable_value(
            &mut docs.category,
            ScriptDocCategory::Types,
            "Types",
        );
        if docs.category == ScriptDocCategory::Functions {
            ui.separator();
            ui.label(RichText::new("Network safe").color(subtle_dark()));
            egui::ComboBox::from_id_salt("script_docs_network_safe")
                .selected_text(match docs.network_filter {
                    ScriptDocNetworkFilter::All => "All",
                    ScriptDocNetworkFilter::Yes => "Yes",
                    ScriptDocNetworkFilter::Unknown => "Unknown",
                    ScriptDocNetworkFilter::No => "No",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut docs.network_filter,
                        ScriptDocNetworkFilter::All,
                        "All",
                    );
                    ui.selectable_value(
                        &mut docs.network_filter,
                        ScriptDocNetworkFilter::Yes,
                        "Yes",
                    );
                    ui.selectable_value(
                        &mut docs.network_filter,
                        ScriptDocNetworkFilter::Unknown,
                        "Unknown",
                    );
                    ui.selectable_value(
                        &mut docs.network_filter,
                        ScriptDocNetworkFilter::No,
                        "No",
                    );
                });
        }
    });
    let search_changed = ui
        .add(
            egui::TextEdit::singleline(&mut docs.search)
                .hint_text(placeholder_text(
                    "Search names, signatures, descriptions, types, or examples...",
                ))
                .desired_width(f32::INFINITY),
        )
        .changed();
    if old_game != docs.game
        || old_category != docs.category
        || old_network_filter != docs.network_filter
        || search_changed
    {
        docs.invalidate();
    }
    docs.refresh();
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
                        RichText::new(format!("{} results", docs.rows.len()))
                            .color(subtle_dark()),
                    );
                });
                ui.separator();
                let selected = docs.selected.as_deref();
                ScrollArea::vertical()
                    .id_salt("script_docs_results")
                    .auto_shrink([false, false])
                    .show_rows(ui, 42.0, docs.rows.len(), |ui, range| {
                        for index in range {
                            let row = &docs.rows[index];
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
                    .show(ui, |ui| match &docs.detail {
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
        docs.select(key);
    }
}

/// What a tag loses crossing between two games, read out of the generated
/// compatibility database.
fn draw_tag_compat_tab(cx: &Ctx, ui: &mut Ui, state: &mut TagCompatUiState) {
    state.ensure_loaded(&locate_help_docs_root());
    match draw_tag_compat_body(ui, state) {
        Some(TagCompatRequest::ExportSheet) => cx.send(HelpCommand::ExportTagCompatSheet),
        None => {}
    }
}

impl Baboon {
    /// Ask where to write the compatibility rows on show, and write them.
    pub(in crate::app) fn export_tag_compat_sheet(&mut self) {
        let Some(compat) = self
            .dialogs
            .get::<HelpWindow>()
            .map(|help| &help.tag_compat)
        else {
            return;
        };
        let pair = compat.pairs.get(compat.pair);
        let stem = pair
            .map(|pair| format!("{}-to-{}", pair.source_game, pair.target_game))
            .unwrap_or_else(|| "tag-compat".to_owned());
        let group = compat.selected_group.clone().unwrap_or_default();
        let csv = compat.visible_csv();
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export compatibility sheet")
            .add_filter("Comma-separated values", &["csv"])
            .set_file_name(format!("{stem}-{group}.csv"))
            .save_file()
        else {
            return;
        };
        self.model.status = match std::fs::write(&path, csv) {
            Ok(()) => format!("Wrote {}", path.display()),
            Err(error) => format!("Could not write {}: {error}", path.display()),
        };
    }
}

/// What the compatibility tab wants done. The tab itself only collects the
/// request — picking a path and writing the file is
/// [`HelpCommand::ExportTagCompatSheet`]'s job, per this module's contract.
enum TagCompatRequest {
    ExportSheet,
}

/// The tab body. Free-standing and doing no I/O, so a test can lay it out
/// without a whole `Baboon`.
fn draw_tag_compat_body(ui: &mut Ui, state: &mut TagCompatUiState) -> Option<TagCompatRequest> {
    if let Some(error) = state.error() {
        doc_load_error(
            ui,
            &format!("Tag compatibility data failed to load: {error}"),
        );
        return None;
    }
    if state.pairs.is_empty() {
        doc_load_error(
            ui,
            "The tag compatibility database covers no profile pairs. Rebuild it with              `cargo run --bin build_tag_compat`.",
        );
        return None;
    }

    let mut request = None;
    ui.horizontal(|ui| {
        let current = state.pairs[state.pair].label();
        egui::ComboBox::from_id_salt("tag_compat_pair")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (index, pair) in state.pairs.iter().enumerate() {
                    ui.selectable_value(&mut state.pair, index, pair.label());
                }
            });
        ui.separator();
        ui.checkbox(&mut state.losses_only, "Only what is lost")
            .on_hover_text(
                "Hide every field that transfers unchanged. There are tens of thousands                  of those, and they are not what you came to find out.",
            );
        ui.separator();
        ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .hint_text(placeholder_text("Filter groups"))
                .desired_width(180.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(!state.fields.is_empty(), egui::Button::new("Export sheet..."))
                .on_hover_text("Write the rows shown here to a CSV file")
                .clicked()
            {
                request = Some(TagCompatRequest::ExportSheet);
            }
        });
    });
    ui.add_space(6.0);

    state.refresh();

    let mut clicked: Option<String> = None;
    egui::Panel::left("tag_compat_groups")
        .resizable(true)
        .default_size(300.0)
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("{} group(s)", state.groups.len()))
                    .color(subtle_dark())
                    .small(),
            );
            egui::ScrollArea::vertical()
                .id_salt("tag_compat_group_list")
                .show(ui, |ui| {
                    for row in &state.groups {
                        let selected = state.selected_group.as_deref() == Some(&row.group);
                        let response = ui.selectable_label(
                            selected,
                            RichText::new(format!("{}  ·  {}", row.group, row.verdict.label()))
                                .color(row.verdict.color()),
                        );
                        let response = match &row.blocked_reason {
                            Some(reason) => response.on_hover_text(reason),
                            None => response.on_hover_text(format!(
                                "{}
{} struct(s) change size · {} field(s) dropped ·                                  {} left at default",
                                row.verdict.explain(),
                                row.size_diff_structs,
                                row.source_only_fields,
                                row.target_only_fields,
                            )),
                        };
                        if response.clicked() {
                            clicked = Some(row.group.clone());
                        }
                    }
                });
        });

    egui::CentralPanel::default().show(ui, |ui| {
        let Some(group) = state.selected_group.clone() else {
            ui.label(
                RichText::new("Select a tag group to see what happens to each of its fields.")
                    .color(subtle_dark()),
            );
            return;
        };
        if state.fields.is_empty() {
            ui.label(
                RichText::new(format!(
                    "Nothing to report for {group} — every field transfers unchanged."
                ))
                .color(subtle_dark()),
            );
            return;
        }
        egui::ScrollArea::both()
            .id_salt("tag_compat_fields")
            .show(ui, |ui| {
                egui::Grid::new("tag_compat_field_grid")
                    .num_columns(4)
                    .striped(true)
                    .show(ui, |ui| {
                        for header in ["Where", "Source", "Target", "What happens"] {
                            ui.label(RichText::new(header).color(subtle_dark()).small());
                        }
                        ui.end_row();
                        for row in &state.fields {
                            ui.label(RichText::new(&row.first_path).small())
                                .on_hover_text(&row.struct_key);
                            ui.label(field_cell(&row.source_name, &row.source_type));
                            ui.label(field_cell(&row.target_name, &row.target_type));
                            let mut text = row.verdict.label().to_owned();
                            if !row.detail.is_empty() {
                                text.push_str("  ·  ");
                                text.push_str(&row.detail);
                            }
                            ui.label(RichText::new(text).color(row.verdict.color()).small())
                                .on_hover_text(format!(
                                    "{}
matched by: {}",
                                    row.verdict.explain(),
                                    row.rule,
                                ));
                            ui.end_row();
                        }
                    });
            });
    });

    if let Some(group) = clicked {
        state.select_group(group);
    }
    request
}

/// Lay out the compatibility tab against a state, for tests. A native GL window
/// is not something a test can click through, and an egui panel that panics on
/// an empty selection or a grid whose column count disagrees with its cells
/// shows up nowhere until something actually lays it out.
#[cfg(test)]
pub(in crate::app) fn draw_tag_compat_body_for_tests(ui: &mut Ui, state: &mut TagCompatUiState) {
    let _ = draw_tag_compat_body(ui, state);
}

/// One side of a field row: its name, with the type underneath when the two
/// sides disagree about it.
fn field_cell(name: &Option<String>, type_name: &Option<String>) -> RichText {
    match (name, type_name) {
        (Some(name), Some(type_name)) => RichText::new(format!("{name}\n{type_name}")).small(),
        (Some(name), None) => RichText::new(name.clone()).small(),
        _ => RichText::new("—").color(subtle_dark()).small(),
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
    Frame::NONE
        .fill(if is_dark_mode() {
            Color32::from_rgb(24, 27, 29)
        } else {
            Color32::from_rgb(238, 241, 243)
        })
        .inner_margin(egui::Margin::same(6))
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

fn draw_tutorials_tab(
    ui: &mut Ui,
    tutorials: &TutorialsState,
    selected_game: &mut String,
    selected_category: &mut TutorialCategory,
) {
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
                                    selected_game.as_str() == shortcut.game.as_str(),
                                    shortcut.game.display_name(),
                                )
                                .clicked()
                            {
                                *selected_game = shortcut.game.as_str().to_owned();
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
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Category").color(subtle_dark()).strong());
                    for category in TUTORIAL_CATEGORIES {
                        ui.selectable_value(selected_category, category, category.label());
                    }
                });
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                ScrollArea::vertical()
                    .id_salt("tutorial_cards")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match tutorials {
                        TutorialsState::Loaded(catalog) => {
                            let entries = catalog
                                .entries_for(selected_game, *selected_category)
                                .collect::<Vec<_>>();
                            if entries.is_empty() {
                                ui.label(
                                    RichText::new(format!(
                                        "No {} tutorials are available for {} yet.",
                                        selected_category.label(),
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
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(&tutorial.creator)
                    .color(subtle_dark())
                    .strong(),
            );
            let title = RichText::new(&tutorial.title)
                .color(foundation_blue())
                .font(FontId::proportional(18.0))
                .strong();
            if let Some(title_url) = tutorial.title_url.as_deref() {
                ui.hyperlink_to(title, title_url);
            } else {
                ui.add(egui::Label::new(title).wrap());
            }
            ui.add_space(8.0);

            match tutorial.kind {
                TutorialKind::Video => draw_video_tutorial_body(ui, tutorial),
                TutorialKind::Article => draw_article_tutorial_body(ui, &tutorial.blocks),
            }
        });
}

fn draw_video_tutorial_body(ui: &mut Ui, tutorial: &TutorialEntry) {
    let Some(url) = tutorial.url.as_deref() else {
        return;
    };
    let thumbnail_width = ui.available_width().min(720.0).max(1.0);
    let thumbnail_size = Vec2::new(thumbnail_width, thumbnail_width * 9.0 / 16.0);
    let response = match &tutorial.thumbnail_texture {
        Some(texture) => ui.add(
            egui::Image::new(egui::load::SizedTexture::new(
                texture.id(),
                texture.size_vec2(),
            ))
            .fit_to_exact_size(thumbnail_size)
            .corner_radius(6.0)
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
        open_tutorial_url(ui.ctx(), url);
    }

    ui.add_space(8.0);
    if ui.button("Watch on YouTube").clicked() {
        open_tutorial_url(ui.ctx(), url);
    }
}

fn draw_article_tutorial_body(ui: &mut Ui, blocks: &[TutorialBlock]) {
    for block in blocks {
        match block {
            TutorialBlock::Heading { text } => {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(text)
                        .color(foundation_blue())
                        .font(FontId::proportional(15.0))
                        .strong(),
                );
            }
            TutorialBlock::Paragraph { spans } => {
                draw_tutorial_spans(ui, spans);
            }
            TutorialBlock::NumberedSteps { items } => {
                for (index, spans) in items.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        ui.add_sized(
                            Vec2::new(24.0, 18.0),
                            egui::Label::new(
                                RichText::new(format!("{}.", index + 1))
                                    .color(subtle_dark())
                                    .strong(),
                            ),
                        );
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width());
                            draw_tutorial_spans(ui, spans);
                        });
                    });
                    ui.add_space(4.0);
                }
            }
        }
        ui.add_space(6.0);
    }
}

fn draw_tutorial_spans(ui: &mut Ui, spans: &[TutorialSpan]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for span in spans {
            for chunk in span.text.split_inclusive(char::is_whitespace) {
                if let Some(url) = span.url.as_deref() {
                    ui.hyperlink_to(RichText::new(chunk).color(foundation_blue()), url);
                } else {
                    ui.label(RichText::new(chunk).color(text_dark()));
                }
            }
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
            for category in TUTORIAL_CATEGORIES {
                let mut selected_game = "haloce_evolved".to_owned();
                let mut selected_category = category;
                let output = crate::app::run_ui_test(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(520.0, 360.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            draw_tutorials_tab(
                                ui,
                                &tutorials,
                                &mut selected_game,
                                &mut selected_category,
                            );
                        });
                    },
                );
                assert!(!output.shapes.is_empty());
            }
        }
    }

    #[test]
    fn tutorial_url_requests_a_new_browser_tab() {
        let ctx = egui::Context::default();
        let url = "https://www.youtube.com/watch?v=2xL2AiuaFwE";
        let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |_| {
            open_tutorial_url(&ctx, url);
        });
        let request = output
            .platform_output
            .commands
            .iter()
            .find_map(|c| match c {
                egui::OutputCommand::OpenUrl(open) => Some(open.clone()),
                _ => None,
            })
            .expect("tutorial action should request an external URL");
        assert_eq!(request.url, url);
        assert!(request.new_tab);
    }
}
