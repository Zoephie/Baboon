//! Main application shell: menus, toolbar, sidebar, tabs, terminal, and status areas.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

impl Baboon {
    pub(super) fn draw_root_ui(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first_run_wizard.is_some() {
            ctx.set_zoom_factor(self.ui_scale);
            set_dark_mode(self.dark_mode);
            ctx.set_visuals(foundation_visuals());
            egui::CentralPanel::default().show(ctx, |_ui| {});
            self.draw_first_run_wizard(ctx);
            return;
        }
        self.prepare_root_frame(ctx);

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
                        if ui.button("New Tag...").clicked() {
                            ui.close_menu();
                            self.open_new_tag_dialog();
                        }
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
                        let has_loaded_folder = self.loaded_tags_root().is_some();
                        if ui
                            .add_enabled(has_loaded_folder, egui::Button::new("Open Tags Folder"))
                            .clicked()
                        {
                            ui.close_menu();
                            self.open_loaded_tags_folder();
                        }
                        if ui
                            .add_enabled(has_loaded_folder, egui::Button::new("Open Data Folder"))
                            .clicked()
                        {
                            ui.close_menu();
                            self.open_loaded_data_folder();
                        }
                        ui.menu_button("Recent Folders", |ui| {
                            if self.recent_folders.is_empty() {
                                ui.add_enabled(false, egui::Button::new("No recent folders"));
                            } else {
                                for path in self.recent_folders.clone() {
                                    let full_path = path.display().to_string();
                                    let label = recent_folder_menu_label(&path);
                                    if ui.button(label).on_hover_text(full_path).clicked() {
                                        ui.close_menu();
                                        self.load_recent_folder(path, ctx.clone());
                                    }
                                }
                                ui.separator();
                                if ui.button("Clear Recent Folders").clicked() {
                                    self.recent_folders.clear();
                                    ui.close_menu();
                                }
                            }
                        });
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
                        ui.separator();
                        if ui
                            .add_enabled(
                                self.selected_key.is_some(),
                                egui::Button::new("Close Current Tag"),
                            )
                            .clicked()
                        {
                            if let Some(key) = self.selected_key.clone() {
                                self.request_close_action(PendingCloseAction::CloseTab(key), ctx);
                            }
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                !self.open_tabs.is_empty() || !self.floating_tabs.is_empty(),
                                egui::Button::new("Close All Tags"),
                            )
                            .clicked()
                        {
                            self.request_close_action(PendingCloseAction::CloseAllTabs, ctx);
                            ui.close_menu();
                        }
                        ui.separator();
                        let can_fix_dependencies = self.selected_key.is_some()
                            && self.source.as_ref().is_some_and(|source| {
                                matches!(source.source, TagSource::LooseFolder { .. })
                            });
                        if ui
                            .add_enabled(
                                can_fix_dependencies,
                                egui::Button::new("Fix Tag Dependencies"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            self.fix_current_tag_dependencies();
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
                                s.reverse_dependencies = None;
                            }
                            self.field_index.invalidate();
                            self.begin_scan_all_entries_with_label(
                                ctx.clone(),
                                "Rebuilding index...",
                            );
                        }
                        ui.separator();
                        if ui.button("Settings...").clicked() {
                            self.settings_open = true;
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui
                            .add_enabled(
                                self.can_undo_current(),
                                egui::Button::new("Undo    Ctrl+Z"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            self.undo_current_tag();
                        }
                        if ui
                            .add_enabled(
                                self.can_redo_current(),
                                egui::Button::new("Redo    Ctrl+Shift+Z"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            self.redo_current_tag();
                        }
                    });
                    ui.menu_button("Tools", |ui| {
                        if ui.button("Run Tool...").clicked() {
                            ui.close_menu();
                            self.tool_commands.open = true;
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                self.selected_key.is_some(),
                                egui::Button::new("Find References to Current Tag"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            if let Some(key) = self.selected_key.clone() {
                                self.show_references_for(&key);
                            }
                        }
                        if ui
                            .add_enabled(
                                self.selected_key.is_some(),
                                egui::Button::new("Explore References to Current Tag..."),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            if let Some(key) = self.selected_key.clone() {
                                self.open_content_explorer(&key);
                            }
                        }
                        if ui.button("Find Unreferenced Tags...").clicked() {
                            ui.close_menu();
                            self.show_unreferenced_tags();
                        }
                        {
                            let is_loose = self.source.as_ref().is_some_and(|source| {
                                matches!(source.source, TagSource::LooseFolder { .. })
                            });
                            let has_index = self
                                .source
                                .as_ref()
                                .is_some_and(|source| source.reverse_dependencies.is_some());
                            let label = if self.building_reverse_dependencies {
                                "Building Reference Index…"
                            } else if has_index {
                                "Rebuild Reference Index"
                            } else {
                                "Build Reference Index"
                            };
                            if ui
                                .add_enabled(
                                    is_loose && !self.building_reverse_dependencies,
                                    egui::Button::new(label),
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.begin_build_reverse_dependencies(ctx.clone(), true);
                            }
                        }
                        if ui.button("List Scenario Map IDs...").clicked() {
                            ui.close_menu();
                            self.show_map_ids();
                        }
                        if ui.button("List Sounds by Class...").clicked() {
                            ui.close_menu();
                            self.show_sounds_by_class();
                        }
                        if ui.button("List Uncompressed Sounds...").clicked() {
                            ui.close_menu();
                            self.show_uncompressed_sounds();
                        }
                        if ui.button("Search Field Values...").clicked() {
                            ui.close_menu();
                            self.field_value_search_open = true;
                        }
                        if ui
                            .add_enabled(
                                self.selected_key.is_some(),
                                egui::Button::new("Compare Current Tag With..."),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            if let Some(key) = self.selected_key.clone() {
                                self.tag_diff = Some(TagDiffState {
                                    a_key: key,
                                    b_key: None,
                                    b_display: None,
                                    results: None,
                                });
                            }
                        }
                        ui.separator();
                        if ui.button("Browse Keywords...").clicked() {
                            ui.close_menu();
                            self.keyword_chooser_open = true;
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
                        ui.menu_button(format!("Sort by: {}", self.browser_sort.label()), |ui| {
                            for option in BrowserSort::ALL {
                                if ui
                                    .selectable_label(self.browser_sort == option, option.label())
                                    .clicked()
                                {
                                    self.browser_sort = option;
                                    ui.close_menu();
                                }
                            }
                        });
                        ui.separator();
                        ui.checkbox(&mut self.show_browser_prefixes, "Show [tag]/[folder]");
                        ui.checkbox(&mut self.show_block_sizes, "Show block sizes");
                        ui.checkbox(
                            &mut self.scroll_to_cycle_dropdowns,
                            "Scroll wheel cycles dropdowns",
                        );
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
                    if self.scanning_entries {
                        let progress = self.entry_index_progress.as_ref();
                        let label = progress
                            .map(|progress| progress.label.as_str())
                            .unwrap_or("Indexing tags...");
                        ui.label(RichText::new(label).strong());
                        if let Some(progress) = progress {
                            let fraction = if progress.total == 0 {
                                0.0
                            } else {
                                progress.processed as f32 / progress.total as f32
                            };
                            let text = if progress.total == 0 {
                                "Discovering files...".to_owned()
                            } else {
                                format!(
                                    "{} / {} files, {} tags",
                                    progress.processed, progress.total, progress.matched
                                )
                            };
                            draw_index_progress_bar(ui, 260.0, Some(fraction), &text);
                        }
                    } else if self.building_reverse_dependencies {
                        let progress = self.reference_index_progress.as_ref();
                        let label = progress
                            .map(|progress| progress.label.as_str())
                            .unwrap_or("Building reference index...");
                        ui.label(RichText::new(label).strong());
                        if let Some(progress) = progress {
                            let fraction = if progress.total == 0 {
                                0.0
                            } else {
                                progress.processed as f32 / progress.total as f32
                            };
                            let text = format!("{} / {} tags", progress.processed, progress.total);
                            draw_index_progress_bar(ui, 260.0, Some(fraction), &text);
                        }
                    } else {
                        ui.label(&self.status);
                    }
                    if let Some(progress) = &self.folder_refactor {
                        ui.separator();
                        ui.label(RichText::new(&progress.label).strong());
                        let mut bar = if let Some(value) = progress.progress {
                            egui::ProgressBar::new(value.clamp(0.0, 1.0))
                        } else {
                            egui::ProgressBar::new(0.0).animate(true)
                        };
                        bar = bar
                            .desired_width(180.0)
                            .text(RichText::new(&progress.phase).color(text_dark()));
                        ui.add(bar);
                        ctx.request_repaint();
                    }
                });
            });

        if self.show_entry_index_wait_notice
            && (self.scanning_entries || self.building_reference_for_entry_index)
        {
            let mut open = self.show_entry_index_wait_notice;
            let mut hide_notice = false;
            egui::Window::new("Indexing")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.set_min_width(360.0);
                    ui.label("Please wait until indexing is completed for best compatibility.");
                    ui.add_space(8.0);
                    if self.scanning_entries {
                        let progress = self.entry_index_progress.as_ref();
                        let label = progress
                            .map(|progress| progress.label.as_str())
                            .unwrap_or("Indexing tags...");
                        ui.label(RichText::new(label).strong());
                        if let Some(progress) = progress {
                            let fraction = if progress.total == 0 {
                                0.0
                            } else {
                                progress.processed as f32 / progress.total as f32
                            };
                            let text = if progress.total == 0 {
                                "Discovering files...".to_owned()
                            } else {
                                format!(
                                    "{} / {} files, {} tags",
                                    progress.processed, progress.total, progress.matched
                                )
                            };
                            draw_index_progress_bar(ui, 330.0, Some(fraction), &text);
                        }
                    } else if self.building_reference_for_entry_index {
                        ui.label(RichText::new("Building reference index...").strong());
                        if let Some(progress) = self.reference_index_progress.as_ref() {
                            let fraction = if progress.total == 0 {
                                0.0
                            } else {
                                progress.processed as f32 / progress.total as f32
                            };
                            let text = format!("{} / {} tags", progress.processed, progress.total);
                            draw_index_progress_bar(ui, 330.0, Some(fraction), &text);
                        } else {
                            draw_index_progress_bar(
                                ui,
                                330.0,
                                None,
                                "Scanning tag dependencies...",
                            );
                        }
                    }
                    ui.add_space(8.0);
                    if ui.button("Hide").clicked() {
                        hide_notice = true;
                    }
                });
            self.show_entry_index_wait_notice = open && !hide_notice;
        }

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
                                        if icon_button(
                                            ui,
                                            ButtonIcon::Clear,
                                            "Clear terminal",
                                            true,
                                            Vec2::new(22.0, 20.0),
                                            text_dark(),
                                        )
                                        .clicked()
                                        {
                                            self.terminal.lines.clear();
                                        }
                                        let open_log_enabled =
                                            self.terminal.last_log_path.is_some();
                                        let mut open_log_button = ui.add_enabled(
                                            open_log_enabled,
                                            egui::Button::new(
                                                RichText::new("Open full log").small(),
                                            ),
                                        );
                                        if let Some(path) = self.terminal.last_log_path.as_ref() {
                                            open_log_button = open_log_button
                                                .on_hover_text(path.display().to_string());
                                        }
                                        if open_log_button.clicked()
                                            && let Some(path) = self.terminal.last_log_path.clone()
                                            && let Err(error) = open_terminal_log(&path)
                                        {
                                            self.status = error;
                                        }
                                        if self.terminal.running {
                                            if self.terminal.process.is_some()
                                                && ui.small_button("Stop").clicked()
                                            {
                                                self.stop_terminal_command();
                                            }
                                            let running_label = self
                                                .terminal
                                                .running_command
                                                .as_deref()
                                                .unwrap_or("running...");
                                            ui.small(
                                                RichText::new(running_label)
                                                    .color(subtle_dark())
                                                    .monospace(),
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
                                    ui.visuals_mut().override_text_color = None;
                                    ui.set_min_width(ui.available_width());
                                    for line in &self.terminal.lines {
                                        let mut text = RichText::new(&line.text)
                                            .color(terminal_line_color(line.severity));
                                        if terminal_line_is_strong(line.severity) {
                                            text = text.font(bold_font(13.0)).strong();
                                        } else {
                                            text = text.monospace().font(FontId::monospace(13.0));
                                        }
                                        ui.add(egui::Label::new(text).wrap());
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
                let sidebar_header = self.source.as_ref().map(|source| {
                    (
                        source.game.clone(),
                        source.source.origin_label(),
                        sidebar_source_path_label(&source.source),
                    )
                });
                if let Some((Some(game), _origin, path_label)) = sidebar_header.as_ref() {
                    draw_game_banner_header(ui, self, game, path_label);
                } else {
                    ui.heading(RichText::new("Tags").color(text_dark()));
                    if let Some((_, origin, _)) = sidebar_header.as_ref() {
                        ui.small(RichText::new(origin).color(subtle_dark()));
                        ui.add_space(8.0);
                    }
                }

                let active_favorite_entries = self.active_favorite_entries.clone();
                let favorite_keys: HashSet<String> = active_favorite_entries
                    .iter()
                    .map(|entry| entry.key.clone())
                    .collect();
                if let Some(source) = &mut self.source {
                    ui.add_space(8.0);
                    let scanning = self.scanning_entries;
                    // Collect deferred scan-trigger here; execute after borrow ends.
                    let mut need_scan = false;
                    let prev_filter_empty = self.filter.is_empty();
                    ui.scope(|ui| {
                        ui.visuals_mut().override_text_color = Some(text_dark());
                        ui.visuals_mut().extreme_bg_color = browser_search_bg();
                        ui.visuals_mut().widgets.inactive.bg_fill = browser_search_bg();
                        ui.visuals_mut().widgets.hovered.bg_fill = browser_search_hover();
                        ui.visuals_mut().widgets.active.bg_fill = browser_search_hover();
                        ui.add(
                            egui::TextEdit::singleline(&mut self.filter)
                                .hint_text("search tags")
                                .desired_width(f32::INFINITY),
                        );
                    });
                    if let Some(warning) = browser::browser_filter_warning(&self.filter) {
                        ui.label(
                            RichText::new(warning)
                                .small()
                                .color(Color32::from_rgb(184, 134, 11)),
                        );
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let groups_btn = ui.scope(|ui| {
                            ui.visuals_mut().widgets.inactive.bg_fill = browser_toolbar_bg();
                            ui.visuals_mut().widgets.hovered.bg_fill = browser_toolbar_active();
                            ui.visuals_mut().widgets.active.bg_fill = browser_toolbar_active();
                            ui.selectable_value(
                                &mut self.browser_mode,
                                BrowserMode::Folders,
                                "Folders",
                            );
                            ui.selectable_value(
                                &mut self.browser_mode,
                                BrowserMode::Groups,
                                "Groups",
                            )
                        });
                        let groups_btn = groups_btn.inner;
                        if groups_btn.clicked()
                            && matches!(source.source, TagSource::LooseFolder { .. })
                            && source.all_entries.is_empty()
                            && !scanning
                        {
                            need_scan = true;
                        }
                        ui.add_space(4.0);
                        ui.scope(|ui| {
                            ui.visuals_mut().widgets.inactive.bg_fill = browser_toolbar_bg();

                            ui.visuals_mut().widgets.hovered.bg_fill = browser_toolbar_active();
                            ui.visuals_mut().widgets.active.bg_fill = browser_toolbar_active();
                            ui.menu_button("Sort", |ui| {
                                for option in BrowserSort::ALL {
                                    if ui
                                        .selectable_label(
                                            self.browser_sort == option,
                                            option.label(),
                                        )
                                        .clicked()
                                    {
                                        self.browser_sort = option;
                                        ui.close_menu();
                                    }
                                }
                            });
                            ui.menu_button("Filter", |ui| {
                                ui.checkbox(&mut self.show_browser_prefixes, "Show prefixes");
                            });
                            ui.menu_button("…", |ui| {
                                ui.checkbox(&mut self.folders_before_tags, "Folders before tags");
                            });
                        });
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
                    let folders_before_tags = self.folders_before_tags;
                    let double_click_to_open = self.double_click_to_open_tags;
                    let mut status_update = None;
                    // Groups and filtered Folders use all_entries (background
                    // scan) so every tag is visible, not just visited folders.
                    let has_all = !source.all_entries.is_empty();
                    let groups_mode = matches!(mode, BrowserMode::Groups);
                    let favorite_context = matches!(source.source, TagSource::LooseFolder { .. })
                        .then_some(&favorite_keys);
                    // One-shot "reveal in tree" request (force-open ancestors +
                    // scroll). Borrowed into the Copy `Reveal` for the draw.
                    let reveal_owned = self.reveal_target.take();
                    let reveal = reveal_owned.as_ref().map(|request| Reveal {
                        key: request.key.as_str(),
                        remaining: request.ancestors.as_slice(),
                    });
                    let sort = self.browser_sort;
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
                                    let favorite_action = draw_favorites(
                                        ui,
                                        &active_favorite_entries,
                                        selected.as_deref(),
                                        &filter,
                                        show_prefixes,
                                        double_click_to_open,
                                        &favorite_keys,
                                    );
                                    ui.label(
                                        RichText::new("Indexing tags…")
                                            .color(subtle_dark())
                                            .small(),
                                    );
                                    favorite_action
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
                                    let favorite_action = draw_favorites(
                                        ui,
                                        &active_favorite_entries,
                                        selected.as_deref(),
                                        &filter,
                                        show_prefixes,
                                        double_click_to_open,
                                        &favorite_keys,
                                    );
                                    if cache.entries.is_empty() {
                                        ui.label(
                                            RichText::new("No matching tags").color(subtle_dark()),
                                        );
                                        return favorite_action;
                                    }
                                    // Empty filter → tree renders every (already
                                    // pruned) entry with folders collapsed.
                                    let tree_action = draw_tree(
                                        ui,
                                        &cache.tree,
                                        &cache.entries,
                                        selected.as_deref(),
                                        "",
                                        show_prefixes,
                                        double_click_to_open,
                                        groups_mode,
                                        reveal,
                                        sort,
                                        !groups_mode && folders_before_tags,
                                        favorite_context,
                                    );
                                    favorite_action.or(tree_action)
                                })
                                .inner
                        }
                    } else {
                        ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let favorite_action = draw_favorites(
                                    ui,
                                    &active_favorite_entries,
                                    selected.as_deref(),
                                    &filter,
                                    show_prefixes,
                                    double_click_to_open,
                                    &favorite_keys,
                                );
                                let tree_action = match mode {
                                    BrowserMode::Folders => {
                                        if let TagSource::LooseFolder { root, .. } = &source.source
                                        {
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
                                                reveal,
                                                sort,
                                                folders_before_tags,
                                                favorite_context,
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
                                                reveal,
                                                sort,
                                                folders_before_tags,
                                                None,
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
                                                reveal,
                                                sort,
                                                false,
                                                favorite_context,
                                            )
                                        }
                                    }
                                };
                                favorite_action.or(tree_action)
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
                    let mut reveal_key = None;
                    let mut rack_rect = None;
                    if self.open_tabs.is_empty() {
                        let response = ui.label(
                            RichText::new("Drop popped tag here")
                                .color(subtle_dark())
                                .strong(),
                        );
                        rack_rect = Some(response.rect);
                    } else {
                        const TAB_BUTTON_SIZE: f32 = 18.0;
                        const TAB_MIN_LABEL_WIDTH: f32 = 48.0;
                        const TAB_MAX_LABEL_WIDTH: f32 = 170.0;
                        const TAB_SIDE_PADDING: f32 = 8.0;
                        const TAB_INNER_GAP: f32 = 3.0;

                        let available_width = ui.available_width().max(120.0);
                        let row_gap = 3.0;
                        // (key, label, active, dirty, label_width, group_tag)
                        let mut rows = Vec::<Vec<(String, String, bool, bool, f32, u32)>>::new();
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
                            let label_width = tab_label_width(
                                ui,
                                &label,
                                TAB_MIN_LABEL_WIDTH,
                                TAB_MAX_LABEL_WIDTH,
                            );
                            let tab_width = TAB_SIDE_PADDING
                                + 16.0
                                + TAB_INNER_GAP
                                + label_width
                                + TAB_INNER_GAP
                                + TAB_BUTTON_SIZE
                                + TAB_INNER_GAP
                                + TAB_BUTTON_SIZE;
                            let next_width = if row.is_empty() {
                                tab_width
                            } else {
                                row_width + row_gap + tab_width
                            };
                            if !row.is_empty() && next_width > available_width {
                                rows.push(row);
                                row = Vec::new();
                                row_width = 0.0;
                            }
                            if !row.is_empty() {
                                row_width += row_gap;
                            }
                            row_width += tab_width;
                            row.push((key, label, active, dirty, label_width, entry.group_tag));
                        }
                        if !row.is_empty() {
                            rows.push(row);
                        }

                        for row in rows {
                            let row_response = ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = row_gap;
                                for (key, label, active, dirty, label_width, group_tag) in row {
                                    let shown_label = truncate_for_cell(&label, label_width);
                                    let base_fill = if active { menu_bar() } else { row_type() };
                                    // Subtle amber tint flags tabs with unsaved edits
                                    // (on top of the ● marker in the label).
                                    let fill = if dirty {
                                        tint_toward(
                                            base_fill,
                                            Color32::from_rgb(184, 134, 11),
                                            0.20,
                                        )
                                    } else {
                                        base_fill
                                    };
                                    let tab_response = Frame::none()
                                        .fill(fill)
                                        .stroke(Stroke::new(1.0, grid_line()))
                                        .inner_margin(egui::Margin {
                                            left: 3.0,
                                            right: 3.0,
                                            top: 2.0,
                                            bottom: 2.0,
                                        })
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = TAB_INNER_GAP;
                                                draw_tag_icon(ui, group_tag, 16.0);
                                                let label_response = ui
                                                    .add_sized(
                                                        Vec2::new(label_width, 18.0),
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
                                                if label_response.middle_clicked() {
                                                    close_key = Some(key.clone());
                                                }
                                                label_response.context_menu(|ui| {
                                                    if ui.button("Reveal in browser").clicked() {
                                                        reveal_key = Some(key.clone());
                                                        ui.close_menu();
                                                    }
                                                    ui.separator();
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
                                                        egui::Button::new("⇱")
                                                            .min_size(Vec2::splat(TAB_BUTTON_SIZE)),
                                                    )
                                                    .on_hover_text("Pop tab out")
                                                    .clicked()
                                                {
                                                    pop_key = Some(key.clone());
                                                }
                                                if ui
                                                    .add(
                                                        egui::Button::new("x")
                                                            .min_size(Vec2::splat(TAB_BUTTON_SIZE)),
                                                    )
                                                    .on_hover_text("Close tab")
                                                    .clicked()
                                                {
                                                    close_key = Some(key.clone());
                                                }
                                            });
                                        });
                                    if tab_response.response.middle_clicked() {
                                        close_key = Some(key.clone());
                                    }
                                }
                            });
                            rack_rect = Some(match rack_rect {
                                Some(rect) => rect.union(row_response.response.rect),
                                None => row_response.response.rect,
                            });
                        }
                    }
                    if close_all {
                        self.request_close_action(PendingCloseAction::CloseAllTabs, ctx);
                    } else if let Some(key) = close_all_but {
                        self.request_close_action(PendingCloseAction::CloseAllButThis(key), ctx);
                    } else if let Some(key) = close_key {
                        self.request_close_action(PendingCloseAction::CloseTab(key), ctx);
                    } else if let Some(key) = pop_key {
                        self.pop_tab(&key);
                    }
                    if let Some(key) = reveal_key {
                        self.reveal_in_browser(&key);
                    }
                    self.tab_rack_rect = rack_rect;
                    ui.add_space(6.0);
                } else {
                    self.tab_rack_rect = None;
                }

                if let Some(entry) = self.selected_entry().cloned() {
                    let selected_key = entry.key.clone();
                    draw_entry_header(ui, &entry, &self.names);
                    self.draw_keyword_bar(ui, &selected_key);

                    // "Search fields" collapses the editor to matching blocks.
                    // Not offered for shader/sound tags (their own surfaces).
                    let supports_field_search = supports_field_search(&entry);
                    if supports_field_search {
                        self.draw_field_search_bar(ui, &selected_key);
                    }

                    let mut bitmap_reimport_request = None;
                    // Documentation overlay (fetched before borrowing parsed_tags).
                    let def_docs = self.def_docs_for_entry(&entry);
                    if let Some(doc) = self.parsed_tags.get_mut(&selected_key) {
                        let mut pending = Vec::new();
                        let mut block_ops = Vec::new();
                        let mut shader_ops = Vec::new();
                        let mut shader_param_ops = Vec::new();
                        let mut h2_shader_param_ops = Vec::new();
                        let mut function_data_ops = Vec::new();
                        let mut model_variant_ops = Vec::new();
                        let mut color_request = None;
                        let mut function_request = None;
                        let mut block_clip_request = None;
                        let mut bitmap_reimport = None;
                        let mut tsv_paste_request = None;
                        let field_filter = compute_pending_field_filter(
                            &doc.tag,
                            supports_field_search,
                            &selected_key,
                            &self.field_search,
                            &mut self.field_search_applied,
                        );
                        let sound_volume = self.audio.volume();
                        let mut edit_context = FieldEditContext {
                            view_scope: "docked",
                            tag_key: &selected_key,
                            group_tag: entry.group_tag,
                            root: Some(doc.tag.root()),
                            game: self
                                .source
                                .as_ref()
                                .and_then(|source| source.game.as_deref()),
                            definitions_root: self.source.as_ref().and_then(|source| match &source
                                .source
                            {
                                TagSource::LooseFolder {
                                    definitions_root, ..
                                } => Some(definitions_root.as_path()),
                                _ => None,
                            }),
                            names: Some(&self.names),
                            tags_root: self.source.as_ref().and_then(|source| {
                                match &source.source {
                                    TagSource::LooseFolder { root, .. } => Some(root.as_path()),
                                    _ => None,
                                }
                            }),
                            status: Some(&mut self.status),
                            editable: is_editable_tag(&entry, &doc.tag),
                            show_block_sizes: self.show_block_sizes,
                            buffers: &mut self.edit_buffers,
                            pending: &mut pending,
                            block_ops: &mut block_ops,
                            block_confirm: &mut self.block_confirm,
                            open_request: &mut self.pending_open,
                            sound_play_request: &mut self.audio.pending,
                            sound_status: self.audio.status.as_deref(),
                            sound_volume,
                            sound_extract_request: &mut self.pending_sound_extract,
                            sound_language: self.audio.language.as_deref(),
                            tool_import: &mut self.pending_tool_import,
                            bitmap_reimport: &mut bitmap_reimport,
                            shader_ops: &mut shader_ops,
                            shader_param_ops: &mut shader_param_ops,
                            h2_shader_param_ops: &mut h2_shader_param_ops,
                            function_data_ops: &mut function_data_ops,
                            model_variant_ops: &mut model_variant_ops,
                            color_request: &mut color_request,
                            function_request: &mut function_request,
                            docs: def_docs.as_deref(),
                            tsv_paste_request: &mut tsv_paste_request,
                            block_clipboard: self.block_clipboard.as_ref(),
                            block_clip_request: &mut block_clip_request,
                            field_filter: field_filter.as_ref(),
                            field_nav: self.field_nav.as_ref(),
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
                        // Snapshot for undo before a mutating batch. Coalesces
                        // continuous edits into one entry; closes the window on
                        // frames with no edits.
                        if !pending.is_empty()
                            || !block_ops.is_empty()
                            || !shader_ops.is_empty()
                            || !shader_param_ops.is_empty()
                            || !model_variant_ops.is_empty()
                        {
                            doc.journal.begin_edit(&doc.tag, "Edit");
                        } else {
                            doc.journal.end_edit_window();
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
                        if let Some(status) = apply_h2_shader_param_ops(
                            &mut doc.tag,
                            h2_shader_param_ops,
                            &mut doc.dirty,
                        ) {
                            self.status = status;
                        }
                        if let Some(status) =
                            apply_function_data_ops(&mut doc.tag, function_data_ops, &mut doc.dirty)
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
                        if let Some(popup) = function_request {
                            self.function_popup = Some(popup);
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
                        // "Paste TSV…" was chosen: open the import window.
                        if let Some(req) = tsv_paste_request {
                            self.tsv_paste = Some(TsvPasteState {
                                tag_key: selected_key.clone(),
                                block_path: req.block_path,
                                block_label: req.block_label,
                                element_count: req.element_count,
                                text: String::new(),
                                status: None,
                            });
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
        self.draw_auxiliary_windows(ctx);
        self.persist_prefs_if_changed();
        self.keywords.save_if_dirty();
        self.draw_floating_tabs(ctx);
        self.handle_floating_tab_drop(ctx);
        if let Some(result) = draw_color_popup(
            ctx,
            &mut self.color_popup,
            &mut self.custom_color_swatches,
            &mut self.palette_last_dir,
        ) {
            match result {
                ColorPopupResult::FieldEdit { tag_key, edit } => {
                    if let Some(doc) = self.parsed_tags.get_mut(&tag_key) {
                        doc.journal.begin_edit(&doc.tag, "Edit color");
                        if let Some(status) =
                            apply_pending_edits(&mut doc.tag, vec![edit], &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        doc.journal.end_edit_window();
                    }
                }
                ColorPopupResult::ShaderOp { tag_key, op } => {
                    if let Some(doc) = self.parsed_tags.get_mut(&tag_key) {
                        doc.journal.begin_edit(&doc.tag, "Shader edit");
                        if let Some(status) =
                            apply_shader_ops(&mut doc.tag, vec![op], &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        doc.journal.end_edit_window();
                    }
                }
                ColorPopupResult::ShaderParamOp { tag_key, op } => {
                    if let Some(doc) = self.parsed_tags.get_mut(&tag_key) {
                        doc.journal.begin_edit(&doc.tag, "Shader parameter");
                        if let Some(status) =
                            apply_shader_param_ops(&mut doc.tag, vec![op], &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        doc.journal.end_edit_window();
                    }
                }
                ColorPopupResult::H2ShaderParamOp { tag_key, op } => {
                    if let Some(doc) = self.parsed_tags.get_mut(&tag_key) {
                        doc.journal.begin_edit(&doc.tag, "Shader parameter");
                        if let Some(status) =
                            apply_h2_shader_param_ops(&mut doc.tag, vec![op], &mut doc.dirty)
                        {
                            self.status = status;
                        }
                        doc.journal.end_edit_window();
                    }
                }
                ColorPopupResult::FunctionDraftColor { target, argb } => {
                    if let Some(popup) = self.function_popup.as_mut() {
                        popup.apply_draft_color(target, argb);
                    }
                }
            }
        }
        if let Some(batch) =
            draw_function_popup(ctx, &mut self.function_popup, &mut self.color_popup)
        {
            if let Some(doc) = self.parsed_tags.get_mut(&batch.tag_key) {
                if !batch.edits.is_empty() || !batch.data_ops.is_empty() {
                    doc.journal.begin_edit(&doc.tag, "Edit function");
                }
                if let Some(status) = apply_pending_edits(&mut doc.tag, batch.edits, &mut doc.dirty)
                {
                    self.status = status;
                }
                if let Some(status) =
                    apply_function_data_ops(&mut doc.tag, batch.data_ops, &mut doc.dirty)
                {
                    self.status = status;
                }
                doc.journal.end_edit_window();
            }
        }
        self.handle_block_confirm(ctx);
        self.handle_save_changes_prompt(ctx);
        self.handle_last_opened_windows_prompt(ctx);
        self.process_pending_open(ctx);
        self.apply_field_nav(ctx);
        // Drain queued sound-player actions: resolve the permutation against the
        // FMOD banks, decode (cached), and play/stop. Runs every frame so voices
        // are reaped even when idle; the tags root is only cloned when acting.
        let sound_root = if self.audio.pending.is_some() {
            self.source_tags_root().map(std::path::Path::to_path_buf)
        } else {
            None
        };
        self.audio.process(sound_root.as_deref(), ctx);
        // Drain a queued sound extraction (decode + write files off the render
        // hot loop) and a reimport hand-off (opens the tool runner pre-filled).
        if let Some(request) = self.pending_sound_extract.take() {
            self.audio.run_extract(request);
            if let Some(status) = self.audio.status.clone() {
                self.status = status;
            }
        }
        // While the Wwise index builds off-thread, keep repainting so the drain
        // loop polls it (the worker also pings on completion, but this covers
        // the "loading…" status update).
        if self.audio.is_busy() {
            ctx.request_repaint();
        }
        self.process_pending_tool_import(ctx);
    }

    fn prepare_root_frame(&mut self, ctx: &egui::Context) {
        self.process_worker_messages(ctx);
        ctx.set_zoom_factor(self.ui_scale);
        self.handle_pixels_per_point_change(ctx);
        self.maybe_refresh_entry_index(ctx.clone());
        set_dark_mode(self.dark_mode);
        ctx.set_visuals(foundation_visuals());
        set_combo_scroll_cycle_enabled(ctx, self.scroll_to_cycle_dropdowns);
        self.handle_app_close_request(ctx);
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::S)) {
            self.save_current_tag();
        }
        // Undo: Ctrl+Z. Redo: Ctrl+Shift+Z or Ctrl+Y.
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Z)) {
            self.undo_current_tag();
        }
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z)
        }) || ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Y))
        {
            self.redo_current_tag();
        }
        let dropped_paths = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        if !dropped_paths.is_empty() {
            self.open_dropped_files(dropped_paths, ctx.clone());
        }
    }

    fn draw_auxiliary_windows(&mut self, ctx: &egui::Context) {
        self.draw_settings_window(ctx);
        self.draw_tool_commands_window(ctx);
        self.draw_new_tag_window(ctx);
        self.draw_about_window(ctx);
        self.draw_query_results_window(ctx);
        self.draw_tag_diff_window(ctx);
        self.draw_content_explorer_window(ctx);
        self.draw_keyword_chooser_window(ctx);
        self.draw_field_value_search_window(ctx);
        self.draw_tsv_paste_window(ctx);
        self.draw_rename_tag_window(ctx);
    }
}

fn recent_folder_menu_label(path: &Path) -> String {
    const MAX_CHARS: usize = 54;
    let text = path.display().to_string();
    let count = text.chars().count();
    if count <= MAX_CHARS {
        return text;
    }
    let keep = MAX_CHARS.saturating_sub(3);
    let tail = text
        .chars()
        .rev()
        .take(keep)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("...{tail}")
}
