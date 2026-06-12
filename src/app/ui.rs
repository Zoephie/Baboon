use super::*;

/// A toolbar launcher button: shows the decoded `.ico` icon when available,
/// otherwise falls back to a single-letter label. Returns the response so the
/// caller can attach a hover tooltip and read `.clicked()`.
fn launcher_button(
    ui: &mut Ui,
    icon: Option<&egui::TextureHandle>,
    fallback: &str,
    enabled: bool,
) -> egui::Response {
    match icon {
        Some(texture) => ui.add_enabled(
            enabled,
            egui::ImageButton::new(egui::load::SizedTexture::new(
                texture.id(),
                Vec2::splat(20.0),
            )),
        ),
        None => ui.add_enabled(
            enabled,
            egui::Button::new(fallback).min_size(Vec2::splat(22.0)),
        ),
    }
}

impl Baboon {
    /// "Search fields" bar (Guerilla-style): typing a block or field name
    /// collapses the editor to just the matching node(s) and their ancestors.
    pub(super) fn draw_field_search_bar(&mut self, ui: &mut Ui, tag_key: &str) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Search fields:").color(text_dark()));
            let query = self.field_search.entry(tag_key.to_owned()).or_default();
            ui.add(
                egui::TextEdit::singleline(query)
                    .hint_text("block or field name")
                    .desired_width(220.0),
            );
            if ui
                .add(egui::Button::new("x").min_size(Vec2::new(22.0, 22.0)))
                .on_hover_text("Clear search")
                .clicked()
            {
                query.clear();
            }
        });
        ui.add_space(4.0);
    }

    fn draw_tool_launcher_buttons(&mut self, ui: &mut Ui) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new("B").min_size(Vec2::splat(22.0)))
                .on_hover_text("Launch Blender")
                .clicked()
            {
                self.launch_blender();
            }

            let tag_test_ready = self
                .kit_tool_path(self.tag_test_executable())
                .is_some_and(|path| path.is_file());
            if launcher_button(ui, self.tag_test_icon.as_ref(), "T", tag_test_ready)
                .on_hover_text("Launch tag_test")
                .clicked()
            {
                self.launch_tag_test();
            }

            let sapien_ready = self
                .kit_tool_path("sapien.exe")
                .is_some_and(|path| path.is_file());
            if launcher_button(ui, self.sapien_icon.as_ref(), "S", sapien_ready)
                .on_hover_text("Launch Sapien")
                .clicked()
            {
                self.launch_sapien();
            }
        });
    }

    fn draw_settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }

        let mut open = self.settings_open;
        egui::Window::new("Settings")
            .id(egui::Id::new("app_settings"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Browser").color(text_dark()).strong());
                ui.add_space(4.0);
                ui.checkbox(
                    &mut self.double_click_to_open_tags,
                    "Double-click to open tags",
                );
                ui.add_space(10.0);

                ui.label(RichText::new("Appearance").color(text_dark()).strong());
                ui.add_space(4.0);
                ui.checkbox(&mut self.dark_mode, "Dark mode");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("UI scale").color(subtle_dark()));
                    ui.add(
                        egui::Slider::new(&mut self.ui_scale, MIN_UI_SCALE..=MAX_UI_SCALE)
                            .show_value(false)
                            .clamping(egui::SliderClamping::Always),
                    );
                    ui.label(
                        RichText::new(format!("{:.0}%", self.ui_scale * 100.0))
                            .color(subtle_dark()),
                    );
                    if ui.button("Reset").clicked() {
                        self.ui_scale = DEFAULT_UI_SCALE;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Model viewport").color(subtle_dark()));
                    ui.add(
                        egui::Slider::new(
                            &mut self.model_preview_size,
                            MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
                        )
                        .show_value(false)
                        .clamping(egui::SliderClamping::Always),
                    );
                    ui.label(
                        RichText::new(format!("{:.0}%", self.model_preview_size * 100.0))
                            .color(subtle_dark()),
                    );
                    if ui.button("Reset").clicked() {
                        self.model_preview_size = DEFAULT_MODEL_PREVIEW_SIZE;
                    }
                });
                ui.add_space(10.0);

                ui.label(RichText::new("Blender").color(text_dark()).strong());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Path").color(subtle_dark()));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.blender_path_input)
                            .desired_width(360.0),
                    );
                    if ui.button("Browse...").clicked() {
                        self.choose_blender_path();
                    }
                });
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Apply").clicked() {
                        let trimmed = self.blender_path_input.trim();
                        self.blender_path = if trimmed.is_empty() {
                            None
                        } else {
                            Some(PathBuf::from(trimmed))
                        };
                        self.status = if let Some(path) = &self.blender_path {
                            format!("Blender path set to {}", path.display())
                        } else {
                            "Blender path cleared".to_owned()
                        };
                    }
                    if ui.button("Clear").clicked() {
                        self.blender_path = None;
                        self.blender_path_input.clear();
                        self.status = "Blender path cleared".to_owned();
                    }
                });
            });
        self.settings_open = open;
    }

    fn draw_about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }

        let mut open = self.about_open;
        egui::Window::new("Baboon Help")
            .id(egui::Id::new("baboon_help"))
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .default_width(780.0)
            .default_height(560.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.help_panel_tab, HelpPanelTab::About, "About");
                    ui.selectable_value(&mut self.help_panel_tab, HelpPanelTab::Doc, "Doc");
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
                    HelpPanelTab::Doc => draw_doc_tab(ui),
                    HelpPanelTab::MapNames => draw_map_names_tab(ui, &mut self.map_names_game_tab),
                }
            });
        self.about_open = open;
    }
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
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(8.0);
    ui.label(RichText::new("Source").color(text_dark()).strong());
    ui.hyperlink_to(BABOON_GITHUB_URL, BABOON_GITHUB_URL);
}

fn draw_doc_tab(ui: &mut Ui) {
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            doc_section(
                ui,
                "Load Folder",
                &[
                    "Use File > Load Folder to choose the root of your editing kit, such as H3EK, HREK, H4EK, H3ODSTEK, or H2AMPEK/H2AEK.",
                    "Baboon will load the kit's tags folder from that root. Picking the kit root also lets the terminal, launcher buttons, and tool import commands resolve the correct working directory.",
                    "Choosing the tags folder directly works for browsing tags, but the kit root is the safest habit when you want Baboon to work with external tools.",
                ],
            );
            doc_section(
                ui,
                "Tag Browser Context Menus",
                &[
                    "Right-click tags in the browser to open actions for that tag.",
                    "Render models, models, scenarios, BSPs, collision models, and physics models can expose geometry extraction actions from this menu.",
                    "Model animation graph tags can expose animation extraction actions from this menu.",
                    "Bitmap tags can extract bitmap images, and monolithic cache entries can extract raw tag files.",
                ],
            );
            doc_section(
                ui,
                "Block Context Menus",
                &[
                    "Right-click a block name in the editor header to copy and paste block data.",
                    "Copy element copies the selected block entry. Copy entire block copies every entry in that block.",
                    "Paste, Replace selected element, and Replace entire block appear when the clipboard is compatible with the current tag group and block path.",
                ],
            );
            doc_section(
                ui,
                "Appearance Settings",
                &[
                    "Use File > Settings > Appearance to switch Dark mode on or off and adjust UI scale.",
                    "Appearance settings are saved in Baboon's preferences, so they stay set the next time you launch the app.",
                    "The same Appearance section also has the default Model viewport size used by .model render previews.",
                ],
            );
            doc_section(
                ui,
                "Model Render View",
                &[
                    "Open a .model tag and choose the Render model tab to inspect the referenced render_model without scrolling through the field tree.",
                    "The Viewport slider in the render tab scales the preview from 80% to 260%; the value is global and saved in preferences.",
                    "If the preview becomes too wide for the editor panel, the region and variant controls move below it automatically.",
                ],
            );
            doc_section(
                ui,
                "Moving Through Blocks",
                &[
                    "Click or focus a block's element selector, then use the mouse wheel to move up and down through entries.",
                    "Arrow Up and Arrow Down also move through the selected block entries.",
                    "The < and > buttons beside the selector do the same one-entry step when you prefer clicking.",
                ],
            );
            doc_section(
                ui,
                "Import Buttons",
                &[
                    "Tag-reference rows for render_model, collision_model, physics_model, and model_animation_graph can show an Import button.",
                    "Import runs the matching editing-kit tool command from the kit root, so it needs a loaded editing-kit folder.",
                    "For animation graphs, Baboon uses the model-animations-uncompressed tool command.",
                ],
            );
        });
}

fn doc_section(ui: &mut Ui, title: &str, lines: &[&str]) {
    ui.label(
        RichText::new(title)
            .color(foundation_blue())
            .font(FontId::proportional(14.0))
            .strong(),
    );
    ui.add_space(4.0);
    for line in lines {
        ui.horizontal_top(|ui| {
            ui.label(RichText::new("-").color(subtle_dark()));
            ui.add(
                egui::Label::new(RichText::new(*line).color(text_dark()))
                    .wrap()
                    .selectable(false),
            );
        });
    }
    ui.add_space(12.0);
}

impl eframe::App for Baboon {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_worker_messages();
        ctx.set_zoom_factor(self.ui_scale);
        set_dark_mode(self.dark_mode);
        ctx.set_visuals(foundation_visuals());
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::S)) {
            self.save_current_tag();
        }

        egui::TopBottomPanel::top("menu")
            .frame(Frame::none().fill(menu_bar()).inner_margin(egui::Margin {
                left: 6.0,
                right: 6.0,
                top: 2.0,
                bottom: 2.0,
            }))
            .show(ctx, |ui| {
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("Load Tag...").clicked() {
                            ui.close_menu();
                            self.begin_load_single(ctx.clone());
                        }
                        if ui.button("Load Folder...").clicked() {
                            ui.close_menu();
                            self.begin_load_folder(ctx.clone());
                        }
                        if ui.button("Load Monolithic blob_index.dat...").clicked() {
                            ui.close_menu();
                            self.begin_load_monolithic(ctx.clone());
                        }
                        ui.separator();
                        if ui.button("Save Current Tag    Ctrl+S").clicked() {
                            ui.close_menu();
                            self.save_current_tag();
                        }
                        if ui
                            .add_enabled(
                                self.selected_key.is_some(),
                                egui::Button::new("Save Current Tag As..."),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            self.save_current_tag_as();
                        }
                        // Regenerate Index: force a fresh full scan and
                        // overwrite the cached index file.
                        let can_regen = self
                            .source
                            .as_ref()
                            .map(|s| {
                                matches!(s.source, TagSource::LooseFolder { .. })
                                    && s.game.is_some()
                            })
                            .unwrap_or(false);
                        if ui
                            .add_enabled(
                                can_regen && !self.scanning_entries,
                                egui::Button::new("Regenerate Index"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            // Clear cached entries so the scan runs fresh.
                            if let Some(s) = self.source.as_mut() {
                                s.all_entries.clear();
                                s.group_tree = crate::source::build_group_tree(&[]);
                            }
                            self.begin_scan_all_entries(ctx.clone());
                        }
                        ui.separator();
                        if ui.button("Settings...").clicked() {
                            self.settings_open = true;
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("View", |ui| {
                        if ui
                            .selectable_label(self.browser_mode == BrowserMode::Folders, "Folders")
                            .clicked()
                        {
                            self.browser_mode = BrowserMode::Folders;
                            ui.close_menu();
                        }
                        if ui
                            .selectable_label(
                                self.browser_mode == BrowserMode::Groups,
                                "Tag Groups",
                            )
                            .clicked()
                        {
                            self.browser_mode = BrowserMode::Groups;
                            ui.close_menu();
                        }
                        ui.separator();
                        ui.checkbox(&mut self.show_browser_prefixes, "Show [tag]/[folder]");
                        ui.checkbox(&mut self.expert_mode, "Expert mode");
                        ui.separator();
                        let terminal_enabled = self.terminal_work_dir.is_some();
                        if ui
                            .add_enabled(
                                terminal_enabled,
                                egui::SelectableLabel::new(self.terminal_open, "Terminal"),
                            )
                            .clicked()
                        {
                            self.terminal_open = !self.terminal_open;
                            self.remember_terminal_open_for_game();
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Help", |ui| {
                        if ui.button("About...").clicked() {
                            self.help_panel_tab = HelpPanelTab::About;
                            self.about_open = true;
                            ui.close_menu();
                        }
                        if ui.button("Doc...").clicked() {
                            self.help_panel_tab = HelpPanelTab::Doc;
                            self.about_open = true;
                            ui.close_menu();
                        }
                        if ui.button("Map Names...").clicked() {
                            self.help_panel_tab = HelpPanelTab::MapNames;
                            self.about_open = true;
                            ui.close_menu();
                        }
                        if ui.button("Check for updates").clicked() {
                            self.begin_check_for_updates(ctx.clone());
                            ui.close_menu();
                        }
                    });
                    self.draw_tool_launcher_buttons(ui);
                });
            });

        egui::TopBottomPanel::bottom("status")
            .frame(Frame::none().fill(menu_bar()).inner_margin(egui::Margin {
                left: 6.0,
                right: 6.0,
                top: 2.0,
                bottom: 2.0,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Status").strong());
                    ui.separator();
                    ui.label(&self.status);
                });
            });

        // Terminal panel — rendered AFTER status so it sits above it.
        if self.terminal_open {
            let work_dir_label = self
                .terminal_work_dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            egui::TopBottomPanel::bottom("terminal")
                .resizable(true)
                .default_height(180.0)
                .height_range(90.0..=600.0)
                .frame(
                    Frame::none()
                        .fill(foundation_group_bg())
                        .inner_margin(egui::Margin {
                            left: 6.0,
                            right: 6.0,
                            top: 4.0,
                            bottom: 4.0,
                        }),
                )
                .show(ctx, |ui| {
                    // Header pinned to the top of the panel.
                    egui::TopBottomPanel::top("terminal_header")
                        .frame(Frame::none())
                        .show_inside(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(RichText::new("Terminal").color(text_dark()));
                                ui.small(
                                    RichText::new(&work_dir_label)
                                        .color(subtle_dark())
                                        .monospace(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button("×")
                                            .on_hover_text("Close terminal")
                                            .clicked()
                                        {
                                            self.terminal_open = false;
                                            self.remember_terminal_open_for_game();
                                        }
                                        if ui.small_button("Clear").clicked() {
                                            self.terminal.lines.clear();
                                        }
                                        if self.terminal.running {
                                            ui.small(
                                                RichText::new("running…").color(subtle_dark()),
                                            );
                                        }
                                    },
                                );
                            });
                            ui.add_space(2.0);
                        });

                    // Input row pinned to the bottom of the panel.
                    egui::TopBottomPanel::bottom("terminal_input")
                        .frame(Frame::none())
                        .show_inside(ui, |ui| {
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(">").monospace().color(subtle_dark()));
                                // Reserve a fixed width for the Run button on
                                // the right; the TextEdit fills the rest. (Do
                                // NOT wrap the button in a right_to_left layout
                                // — that consumes all remaining width and leaves
                                // nothing for the input field.)
                                let button_w = 52.0;
                                let text_w = (ui.available_width() - button_w - 8.0).max(40.0);
                                let resp = ui.add_enabled(
                                    !self.terminal.running,
                                    egui::TextEdit::singleline(&mut self.terminal.input)
                                        .desired_width(text_w)
                                        .font(egui::TextStyle::Monospace)
                                        .hint_text("tool <command> …"),
                                );
                                if self.terminal.refocus_input && !self.terminal.running {
                                    resp.request_focus();
                                    self.terminal.refocus_input = false;
                                }
                                let run_clicked = ui
                                    .add_enabled(!self.terminal.running, egui::Button::new("Run"))
                                    .clicked();
                                let enter = resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if resp.has_focus() && !self.terminal.running {
                                    let recall = ui.input(|i| {
                                        if i.key_pressed(egui::Key::ArrowUp) {
                                            -1
                                        } else if i.key_pressed(egui::Key::ArrowDown) {
                                            1
                                        } else {
                                            0
                                        }
                                    });
                                    if recall != 0 {
                                        self.recall_terminal_history(recall);
                                        resp.request_focus();
                                    }
                                }
                                if run_clicked || enter {
                                    self.begin_terminal_command(ctx.clone());
                                    // Refocus the input so the user can keep typing.
                                    resp.request_focus();
                                }
                            });
                        });

                    // Output fills the remaining center space. The CentralPanel
                    // bounds the scroll area exactly, so there's no available_height
                    // feedback to fight the resize handle.
                    egui::CentralPanel::default()
                        .frame(
                            Frame::none()
                                .fill(Color32::from_rgb(24, 24, 23))
                                .inner_margin(egui::Margin {
                                    left: 6.0,
                                    right: 6.0,
                                    top: 4.0,
                                    bottom: 4.0,
                                }),
                        )
                        .show_inside(ui, |ui| {
                            let want_scroll_bottom = self.terminal.scroll_to_bottom;
                            self.terminal.scroll_to_bottom = false;
                            egui::ScrollArea::vertical()
                                .id_salt("terminal_output")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    for line in &self.terminal.lines {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(line)
                                                    .monospace()
                                                    .font(FontId::monospace(13.0))
                                                    .color(Color32::from_rgb(232, 232, 228)),
                                            )
                                            .wrap(),
                                        );
                                    }
                                    if want_scroll_bottom {
                                        ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                                    }
                                });
                        });
                });
        }

        egui::SidePanel::left("tag_browser")
            .resizable(true)
            .default_width(330.0)
            .frame(Frame::none().fill(left_panel()).inner_margin(egui::Margin {
                left: 8.0,
                right: 8.0,
                top: 6.0,
                bottom: 6.0,
            }))
            .show(ctx, |ui| {
                ui.heading(RichText::new("Tags").color(text_dark()));
                if let Some(source) = &mut self.source {
                    ui.label(
                        RichText::new(&source.label)
                            .color(foundation_blue())
                            .strong(),
                    );
                    ui.small(RichText::new(source.source.origin_label()).color(subtle_dark()));
                    ui.add_space(8.0);
                    let scanning = self.scanning_entries;
                    // Collect deferred scan-trigger here; execute after borrow ends.
                    let mut need_scan = false;
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.browser_mode,
                            BrowserMode::Folders,
                            "Folders",
                        );
                        let groups_btn = ui.selectable_value(
                            &mut self.browser_mode,
                            BrowserMode::Groups,
                            "Groups",
                        );
                        if groups_btn.clicked()
                            && matches!(source.source, TagSource::LooseFolder { .. })
                            && source.all_entries.is_empty()
                            && !scanning
                        {
                            need_scan = true;
                        }
                    });
                    ui.checkbox(&mut self.show_browser_prefixes, "Show prefixes");
                    ui.add_space(6.0);
                    let prev_filter_empty = self.filter.is_empty();
                    ui.scope(|ui| {
                        ui.visuals_mut().override_text_color = Some(text_dark());
                        let search_bg = if is_dark_mode() {
                            Color32::from_rgb(48, 48, 46)
                        } else {
                            Color32::from_rgb(246, 246, 244)
                        };
                        let search_hover = if is_dark_mode() {
                            Color32::from_rgb(58, 58, 55)
                        } else {
                            Color32::from_rgb(255, 255, 252)
                        };
                        ui.visuals_mut().extreme_bg_color = search_bg;
                        ui.visuals_mut().widgets.inactive.bg_fill = search_bg;
                        ui.visuals_mut().widgets.hovered.bg_fill = search_hover;
                        ui.visuals_mut().widgets.active.bg_fill = search_hover;
                        ui.add(
                            egui::TextEdit::singleline(&mut self.filter)
                                .hint_text("search tags")
                                .desired_width(f32::INFINITY),
                        );
                    });
                    if prev_filter_empty
                        && !self.filter.is_empty()
                        && matches!(source.source, TagSource::LooseFolder { .. })
                        && source.all_entries.is_empty()
                        && !scanning
                    {
                        need_scan = true;
                    }
                    ui.add_space(4.0);
                    let selected = self.selected_key.clone();
                    let filter = self.filter.trim().to_owned();
                    let mode = self.browser_mode;
                    let show_prefixes = self.show_browser_prefixes;
                    let double_click_to_open = self.double_click_to_open_tags;
                    let mut status_update = None;
                    // Groups and filtered Folders use all_entries (background
                    // scan) so every tag is visible, not just visited folders.
                    let has_all = !source.all_entries.is_empty();
                    let groups_mode = matches!(mode, BrowserMode::Groups);
                    let action = if !filter.is_empty() {
                        // Active search: render a *pruned* tree containing only
                        // the matching tags, with folders collapsed so the user
                        // drills down to find them. The pruned tree is memoized
                        // in `filter_cache` (rebuilt once per keystroke, not per
                        // frame), and collapsed folders don't build their
                        // children — so per-frame cost stays bounded.
                        let entries: &[TagEntry] = if has_all {
                            &source.all_entries
                        } else {
                            &source.entries
                        };
                        if scanning && !has_all {
                            ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new("Indexing tags…")
                                            .color(subtle_dark())
                                            .small(),
                                    );
                                    None
                                })
                                .inner
                        } else {
                            self.filter_cache.refresh(
                                self.source_generation,
                                &filter,
                                entries,
                                has_all,
                                groups_mode,
                            );
                            let cache = &self.filter_cache;
                            ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    if cache.entries.is_empty() {
                                        ui.label(
                                            RichText::new("No matching tags").color(subtle_dark()),
                                        );
                                        return None;
                                    }
                                    // Empty filter → tree renders every (already
                                    // pruned) entry with folders collapsed.
                                    draw_tree(
                                        ui,
                                        &cache.tree,
                                        &cache.entries,
                                        selected.as_deref(),
                                        "",
                                        show_prefixes,
                                        double_click_to_open,
                                        groups_mode,
                                    )
                                })
                                .inner
                        }
                    } else {
                        ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| match mode {
                                BrowserMode::Folders => {
                                    if let TagSource::LooseFolder { root } = &source.source {
                                        let root = root.clone();
                                        draw_tree_lazy(
                                            ui,
                                            &mut source.tree,
                                            &mut source.entries,
                                            &mut source.group_tree,
                                            &root,
                                            &source.names,
                                            selected.as_deref(),
                                            &filter,
                                            show_prefixes,
                                            double_click_to_open,
                                            &mut status_update,
                                        )
                                    } else {
                                        draw_tree(
                                            ui,
                                            &source.tree,
                                            &source.entries,
                                            selected.as_deref(),
                                            &filter,
                                            show_prefixes,
                                            double_click_to_open,
                                            false,
                                        )
                                    }
                                }
                                BrowserMode::Groups => {
                                    if scanning && !has_all {
                                        ui.label(
                                            RichText::new("Indexing tags…")
                                                .color(subtle_dark())
                                                .small(),
                                        );
                                        None
                                    } else {
                                        let entries = if has_all {
                                            &source.all_entries[..]
                                        } else {
                                            &source.entries[..]
                                        };
                                        draw_tree(
                                            ui,
                                            &source.group_tree,
                                            entries,
                                            selected.as_deref(),
                                            &filter,
                                            show_prefixes,
                                            double_click_to_open,
                                            true,
                                        )
                                    }
                                }
                            })
                            .inner
                    };
                    if let Some(status) = status_update {
                        self.status = status;
                    }
                    if let Some(action) = action {
                        self.handle_browser_action(action, ctx.clone());
                    }
                    // Deferred: begin_scan_all_entries needs &mut self, so
                    // it must be called after the `source` borrow ends.
                    if need_scan {
                        self.begin_scan_all_entries(ctx.clone());
                    }
                } else {
                    ui.label("Use File to load a tag, folder, or monolithic cache.");
                }
            });

        egui::CentralPanel::default()
            .frame(Frame::none().fill(editor_bg()).inner_margin(egui::Margin {
                left: 10.0,
                right: 10.0,
                top: 8.0,
                bottom: 8.0,
            }))
            .show(ctx, |ui| {
                if !self.open_tabs.is_empty() || self.dragging_floating_tab.is_some() {
                    let mut close_key = None;
                    let mut pop_key = None;
                    let mut close_all = false;
                    let mut close_all_but = None;
                    let mut rack_rect = None;
                    if self.open_tabs.is_empty() {
                        let response = ui.label(
                            RichText::new("Drop popped tag here")
                                .color(subtle_dark())
                                .strong(),
                        );
                        rack_rect = Some(response.rect);
                    } else {
                        const TAB_LABEL_WIDTH: f32 = 170.0;
                        const TAB_WIDTH: f32 = 248.0;

                        let available_width = ui.available_width().max(TAB_WIDTH);
                        let row_gap = ui.spacing().item_spacing.x;
                        let mut rows = Vec::<Vec<(String, String, bool)>>::new();
                        let mut row = Vec::new();
                        let mut row_width = 0.0;

                        for key in self.open_tabs.clone() {
                            let Some(entry) = self.entry_for_key(&key) else {
                                continue;
                            };
                            let active = self.selected_key.as_deref() == Some(key.as_str());
                            let dirty = self
                                .parsed_tags
                                .get(&key)
                                .map(|doc| doc.dirty)
                                .unwrap_or(false);
                            let label = if dirty {
                                format!("● {}", tag_tab_label(entry))
                            } else {
                                tag_tab_label(entry)
                            };
                            let next_width = if row.is_empty() {
                                TAB_WIDTH
                            } else {
                                row_width + row_gap + TAB_WIDTH
                            };
                            if !row.is_empty() && next_width > available_width {
                                rows.push(row);
                                row = Vec::new();
                                row_width = 0.0;
                            }
                            if !row.is_empty() {
                                row_width += row_gap;
                            }
                            row_width += TAB_WIDTH;
                            row.push((key, label, active));
                        }
                        if !row.is_empty() {
                            rows.push(row);
                        }

                        for row in rows {
                            let row_response = ui.horizontal(|ui| {
                                for (key, label, active) in row {
                                    let shown_label = truncate_for_cell(&label, TAB_LABEL_WIDTH);
                                    let fill = if active { menu_bar() } else { row_type() };
                                    Frame::none()
                                        .fill(fill)
                                        .stroke(Stroke::new(1.0, grid_line()))
                                        .inner_margin(egui::Margin {
                                            left: 6.0,
                                            right: 4.0,
                                            top: 3.0,
                                            bottom: 3.0,
                                        })
                                        .show(ui, |ui| {
                                            ui.set_width(TAB_WIDTH);
                                            ui.horizontal(|ui| {
                                                let label_response = ui
                                                    .add_sized(
                                                        Vec2::new(TAB_LABEL_WIDTH, 20.0),
                                                        egui::SelectableLabel::new(
                                                            active,
                                                            RichText::new(shown_label.clone())
                                                                .color(text_dark())
                                                                .strong(),
                                                        ),
                                                    )
                                                    .on_hover_text(label.clone());
                                                if label_response.clicked() {
                                                    self.selected_key = Some(key.clone());
                                                    self.ensure_tag_loading(
                                                        key.clone(),
                                                        ctx.clone(),
                                                    );
                                                }
                                                label_response.context_menu(|ui| {
                                                    if ui.button("Close all").clicked() {
                                                        close_all = true;
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Close all but this").clicked() {
                                                        close_all_but = Some(key.clone());
                                                        ui.close_menu();
                                                    }
                                                });
                                                if ui
                                                    .add(
                                                        egui::Button::new("pop")
                                                            .min_size(Vec2::new(30.0, 18.0)),
                                                    )
                                                    .clicked()
                                                {
                                                    pop_key = Some(key.clone());
                                                }
                                                if ui
                                                    .add(
                                                        egui::Button::new("x")
                                                            .min_size(Vec2::new(18.0, 18.0)),
                                                    )
                                                    .clicked()
                                                {
                                                    close_key = Some(key.clone());
                                                }
                                            });
                                        });
                                }
                            });
                            rack_rect = Some(match rack_rect {
                                Some(rect) => rect.union(row_response.response.rect),
                                None => row_response.response.rect,
                            });
                        }
                    }
                    if close_all {
                        self.close_all_tabs();
                    } else if let Some(key) = close_all_but {
                        self.close_all_tabs_but(&key);
                    } else if let Some(key) = close_key {
                        self.close_tab(&key);
                    } else if let Some(key) = pop_key {
                        self.pop_tab(&key);
                    }
                    self.tab_rack_rect = rack_rect;
                    ui.add_space(6.0);
                } else {
                    self.tab_rack_rect = None;
                }

                if let Some(entry) = self.selected_entry().cloned() {
                    let selected_key = entry.key.clone();
                    draw_entry_header(ui, &entry, &self.names);

                    // "Search fields" collapses the editor to matching blocks.
                    // Not offered for shader/sound tags (their own surfaces).
                    let supports_field_search = supports_field_search(&entry);
                    if supports_field_search {
                        self.draw_field_search_bar(ui, &selected_key);
                    }

                    let mut bitmap_reimport_request = None;
                    if let Some(doc) = self.parsed_tags.get_mut(&selected_key) {
                        let mut pending = Vec::new();
                        let mut block_ops = Vec::new();
                        let mut shader_ops = Vec::new();
                        let mut shader_param_ops = Vec::new();
                        let mut model_variant_ops = Vec::new();
                        let mut color_request = None;
                        let mut block_clip_request = None;
                        let mut bitmap_reimport = None;
                        let field_filter = compute_pending_field_filter(
                            &doc.tag,
                            supports_field_search,
                            &selected_key,
                            &self.field_search,
                            &mut self.field_search_applied,
                        );
                        let mut edit_context = FieldEditContext {
                            view_scope: "docked",
                            tag_key: &selected_key,
                            group_tag: entry.group_tag,
                            tags_root: self.source.as_ref().and_then(|source| {
                                match &source.source {
                                    TagSource::LooseFolder { root } => Some(root.as_path()),
                                    _ => None,
                                }
                            }),
                            editable: is_editable_tag(&entry, &doc.tag),
                            buffers: &mut self.edit_buffers,
                            pending: &mut pending,
                            block_ops: &mut block_ops,
                            block_confirm: &mut self.block_confirm,
                            open_request: &mut self.pending_open,
                            tool_import: &mut self.pending_tool_import,
                            bitmap_reimport: &mut bitmap_reimport,
                            shader_ops: &mut shader_ops,
                            shader_param_ops: &mut shader_param_ops,
                            model_variant_ops: &mut model_variant_ops,
                            color_request: &mut color_request,
                            block_clipboard: self.block_clipboard.as_ref(),
                            block_clip_request: &mut block_clip_request,
                            field_filter: field_filter.as_ref(),
                        };
                        if is_bitmap_tag(&entry) {
                            let preview = self
                                .bitmap_previews
                                .entry(selected_key.clone())
                                .or_default();
                            draw_bitmap_tag(
                                ui,
                                ctx,
                                &doc.tag,
                                &entry,
                                &self.names,
                                &mut self.color_popup,
                                preview,
                                self.expert_mode,
                                &mut edit_context,
                            );
                        } else {
                            let mut local_model_preview;
                            let model_preview = if is_model_group(entry.group_tag, &self.names) {
                                self.model_previews.entry(selected_key.clone()).or_default()
                            } else {
                                local_model_preview = ModelPreviewState::default();
                                &mut local_model_preview
                            };
                            draw_tag(
                                ui,
                                &doc.tag,
                                &entry,
                                &self.names,
                                self.source.as_ref().map(|source| &source.source),
                                &mut self.rmdf_cache,
                                &mut self.rmop_cache,
                                &mut self.color_popup,
                                &mut self.function_popup,
                                model_preview,
                                &mut self.model_preview_size,
                                self.expert_mode,
                                &mut edit_context,
                            );
                        }
                        if let Some(status) =
                            apply_pending_edits(&mut doc.tag, pending, &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        if let Some(status) =
                            apply_block_ops(&mut doc.tag, block_ops, &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        if let Some(status) =
                            apply_shader_ops(&mut doc.tag, shader_ops, &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        if let Some(status) =
                            apply_shader_param_ops(&mut doc.tag, shader_param_ops, &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        if let Some(status) =
                            apply_model_variant_ops(&mut doc.tag, model_variant_ops, &mut doc.dirty)
                        {
                            self.status = status;
                            if let Some(preview) = self.model_previews.get_mut(&selected_key) {
                                preview.loaded_key = None;
                                preview.data = None;
                            }
                        }
                        // A color swatch was clicked: open the shared picker.
                        if let Some(popup) = color_request {
                            self.color_popup = Some(popup);
                        }
                        // Element(s) were copied: stash them on the clipboard.
                        if let Some(clip) = block_clip_request {
                            self.status = format!(
                                "Copied {} '{}' element(s)",
                                clip.elements.len(),
                                clip.label
                            );
                            self.block_clipboard = Some(clip);
                        }
                        bitmap_reimport_request = bitmap_reimport;
                    } else if self.loading_tags.contains(&selected_key) {
                        ui.label("Loading tag data...");
                    } else {
                        ui.label("Select the tag again to load it.");
                    }
                    if let Some(key) = bitmap_reimport_request {
                        self.begin_reimport_bitmap(key, ctx.clone());
                    }
                } else {
                    ui.heading("No tag selected");
                    ui.label("Load a source from File, then select a tag in the browser.");
                }
            });
        self.draw_settings_window(ctx);
        self.draw_about_window(ctx);
        self.persist_prefs_if_changed();
        self.draw_floating_tabs(ctx);
        self.handle_floating_tab_drop(ctx);
        if let Some(result) = draw_color_popup(ctx, &mut self.color_popup) {
            match result {
                ColorPopupResult::FieldEdit { tag_key, edit } => {
                    if let Some(doc) = self.parsed_tags.get_mut(&tag_key) {
                        if let Some(status) =
                            apply_pending_edits(&mut doc.tag, vec![edit], &mut doc.dirty)
                        {
                            self.status = status;
                        }
                    }
                }
                ColorPopupResult::ShaderOp { tag_key, op } => {
                    if let Some(doc) = self.parsed_tags.get_mut(&tag_key) {
                        if let Some(status) =
                            apply_shader_ops(&mut doc.tag, vec![op], &mut doc.dirty)
                        {
                            self.status = status;
                        }
                    }
                }
            }
        }
        if let Some(batch) = draw_function_popup(ctx, &mut self.function_popup) {
            if let Some(doc) = self.parsed_tags.get_mut(&batch.tag_key) {
                if let Some(status) = apply_pending_edits(&mut doc.tag, batch.edits, &mut doc.dirty)
                {
                    self.status = status;
                }
            }
        }
        self.handle_block_confirm(ctx);
        self.process_pending_open(ctx);
        self.process_pending_tool_import(ctx);
    }
}
