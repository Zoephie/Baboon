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
                        if self.current_source_is_container()
                            && ui.button("Import Tag...").clicked()
                        {
                            ui.close_menu();
                            self.begin_import_tag(None);
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
                        if ui
                            .button("Open Campaign Evolved container (.utoc)...")
                            .clicked()
                        {
                            ui.close_menu();
                            self.begin_load_iostore_container(ctx.clone());
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
                        if icon_text_button(
                            ui,
                            ButtonIcon::Save,
                            "Save Current Tag    Ctrl+S",
                            true,
                        )
                        .clicked()
                        {
                            ui.close_menu();
                            self.save_current_tag();
                        }
                        if ui
                            .add_enabled(
                                self.kits[self.active].selected_key.is_some(),
                                egui::Button::new("Save Current Tag As..."),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            self.save_current_tag_as();
                        }
                        if self.current_source_is_container() {
                            if ui
                                .add_enabled(
                                    self.kits[self.active].parsed_tags.values().any(|d| d.dirty),
                                    egui::Button::new("Export Mod..."),
                                )
                                .on_hover_text(
                                    "Bundle every open, modified tag into one portable mod overlay (the base game is left untouched)",
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.export_mod();
                            }
                        }
                        if self.expert_mode {
                            if ui
                                .add_enabled(
                                    self.can_convert_current_tag(),
                                    egui::Button::new("Save Current Tag for Another Game..."),
                                )
                                .on_hover_text(
                                    "Expert feature: convert the selected MCC editing-kit tag using another game's definitions",
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.open_tag_conversion_dialog();
                            }
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                self.kits[self.active].selected_key.is_some(),
                                egui::Button::new("Close Current Tag"),
                            )
                            .clicked()
                        {
                            if let Some(key) = self.kits[self.active].selected_key.clone() {
                                self.request_close_action(PendingCloseAction::CloseTab(key), ctx);
                            }
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                !self.kits[self.active].open_tabs.is_empty(),
                                egui::Button::new("Close All Tags"),
                            )
                            .clicked()
                        {
                            self.request_close_action(PendingCloseAction::CloseAllTabs, ctx);
                            ui.close_menu();
                        }
                        ui.separator();
                        let can_fix_dependencies = self.kits[self.active].selected_key.is_some()
                            && self.source().is_some_and(|source| {
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
                        let can_regen = self.source()
                            .map(|s| {
                                matches!(s.source, TagSource::LooseFolder { .. })
                                    && s.game.is_some()
                            })
                            .unwrap_or(false);
                        if ui
                            .add_enabled(
                                can_regen && !self.kits[self.active].scanning_entries,
                                egui::Button::new("Regenerate Index"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            // Clear cached entries so the scan runs fresh.
                            if let Some(s) = self.source_mut() {
                                s.all_entries.clear();
                                s.group_tree = crate::source::build_group_tree(&[]);
                                s.reverse_dependencies = None;
                            }
                            self.kits[self.active].field_index.invalidate();
                            self.begin_scan_all_entries_with_label(
                                ctx.clone(),
                                "Rebuilding index...",
                            );
                        }
                        let can_refresh_browser = self.source().is_some_and(|source| {
                            matches!(source.source, TagSource::LooseFolder { .. })
                                && source.game.is_some()
                        });
                        if ui
                            .add_enabled(
                                can_refresh_browser
                                    && !self.kits[self.active].scanning_entries
                                    && !self.refreshing_entry_index,
                                egui::Button::new("Refresh Tag Browser"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            self.refresh_tag_browser(ctx.clone());
                        }
                        ui.separator();
                        if icon_text_button(ui, ButtonIcon::Settings, "Settings...", true).clicked()
                        {
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
                                self.kits[self.active].selected_key.is_some(),
                                egui::Button::new("Find References to Current Tag"),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            if let Some(key) = self.kits[self.active].selected_key.clone() {
                                self.show_references_for(&key);
                            }
                        }
                        if ui
                            .add_enabled(
                                self.kits[self.active].selected_key.is_some(),
                                egui::Button::new("Explore References to Current Tag..."),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            if let Some(key) = self.kits[self.active].selected_key.clone() {
                                self.open_content_explorer(&key);
                            }
                        }
                        if ui.button("Find Unreferenced Tags...").clicked() {
                            ui.close_menu();
                            self.show_unreferenced_tags();
                        }
                        {
                            // Loose folders and Campaign Evolved containers can
                            // both be indexed; cache sources cannot.
                            let indexable = self.source().is_some_and(|source| {
                                matches!(
                                    source.source,
                                    TagSource::LooseFolder { .. }
                                        | TagSource::IoStoreContainerSet { .. }
                                )
                            });
                            let has_index = self.source()
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
                                    indexable && !self.building_reverse_dependencies,
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
                                self.kits[self.active].selected_key.is_some(),
                                egui::Button::new("Compare Current Tag With..."),
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            if let Some(key) = self.kits[self.active].selected_key.clone() {
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
                        let terminal_enabled = self.kits[self.active].terminal_work_dir.is_some();
                        if ui
                            .add_enabled(
                                terminal_enabled,
                                egui::SelectableLabel::new(self.kits[self.active].terminal_open, "Terminal"),
                            )
                            .clicked()
                        {
                            self.kits[self.active].terminal_open = !self.kits[self.active].terminal_open;
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
                        if icon_text_button(ui, ButtonIcon::Doc, "Doc...", true).clicked() {
                            self.help_panel_tab = HelpPanelTab::Doc;
                            self.about_open = true;
                            ui.close_menu();
                        }
                        if ui.button("Tutorials...").clicked() {
                            self.help_panel_tab = HelpPanelTab::Tutorials;
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

        self.draw_kit_strip(ctx);

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
                    if self.kits[self.active].scanning_entries {
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
            && (self.kits[self.active].scanning_entries || self.building_reference_for_entry_index)
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
                    if self.kits[self.active].scanning_entries {
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
        if self.kits[self.active].terminal_open {
            let work_dir_label = self.kits[self.active]
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
                                            self.kits[self.active].terminal_open = false;
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
                let kit_index = self.active;
                self.draw_kit_browser(ui, ctx, kit_index);
            });

        egui::CentralPanel::default()
            .frame(Frame::none().fill(editor_bg()).inner_margin(egui::Margin {
                left: 10.0,
                right: 10.0,
                top: 8.0,
                bottom: 8.0,
            }))
            .show(ctx, |ui| {
                self.draw_tag_tiles(ui, ctx);
            });
        self.draw_auxiliary_windows(ctx);
        self.persist_prefs_if_changed();
        // Every kit, not just the active one: a background kit's sidecar can be
        // dirty from edits made before the user switched away.
        for kit in &mut self.kits {
            kit.keywords.save_if_dirty();
        }
        if let Some(result) = draw_color_popup(
            ctx,
            &mut self.color_popup,
            &mut self.custom_color_swatches,
            &mut self.palette_last_dir,
        ) {
            match result {
                ColorPopupResult::FieldEdit { tag_key, edit } => {
                    if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(&tag_key) {
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
                    if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(&tag_key) {
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
                    if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(&tag_key) {
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
                    if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(&tag_key) {
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
            if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(&batch.tag_key) {
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

    /// Strip of loaded kits above the browser, one button each.
    ///
    /// Hidden while a single empty workspace is the only kit, so an unloaded
    /// Baboon looks exactly as it did when it was single-source.
    fn draw_kit_strip(&mut self, ctx: &egui::Context) {
        if self.kits.len() == 1 && self.kits[0].is_empty_workspace() {
            return;
        }
        let mut activate = None;
        let mut close = None;
        let mut add_kit_then: Option<LoadKind> = None;
        let recents = self.recent_folders.clone();
        egui::TopBottomPanel::top("kit_strip")
            .frame(Frame::none().fill(row_type()).inner_margin(egui::Margin {
                left: 6.0,
                right: 6.0,
                top: 3.0,
                bottom: 3.0,
            }))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (index, kit) in self.kits.iter().enumerate() {
                        let active = index == self.active;
                        let dirty = kit.parsed_tags.values().any(|document| document.dirty);
                        let label = kit_strip_label(kit);
                        let shown = if dirty {
                            format!("● {label}")
                        } else {
                            label.clone()
                        };
                        let fill = if active { menu_bar() } else { left_panel() };
                        let fill = if dirty {
                            tint_toward(fill, Color32::from_rgb(184, 134, 11), 0.20)
                        } else {
                            fill
                        };
                        Frame::none()
                            .fill(fill)
                            .stroke(Stroke::new(1.0, grid_line()))
                            .inner_margin(egui::Margin {
                                left: 4.0,
                                right: 3.0,
                                top: 2.0,
                                bottom: 2.0,
                            })
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 3.0;
                                    if ui
                                        .add(egui::SelectableLabel::new(
                                            active,
                                            RichText::new(shown).color(text_dark()).strong(),
                                        ))
                                        .on_hover_text(&label)
                                        .clicked()
                                    {
                                        activate = Some(index);
                                    }
                                    if ui
                                        .add(egui::Button::new("x").min_size(Vec2::splat(16.0)))
                                        .on_hover_text("Close this kit")
                                        .clicked()
                                    {
                                        close = Some(kit.id);
                                    }
                                });
                            });
                    }
                    ui.menu_button("+", |ui| {
                        ui.set_min_width(240.0);
                        if ui.button("Load Folder...").clicked() {
                            ui.close_menu();
                            add_kit_then = Some(LoadKind::Folder);
                        }
                        if ui.button("Load Tag...").clicked() {
                            ui.close_menu();
                            add_kit_then = Some(LoadKind::SingleFile);
                        }
                        if ui.button("Load Monolithic blob_index.dat...").clicked() {
                            ui.close_menu();
                            add_kit_then = Some(LoadKind::Monolithic);
                        }
                        if ui
                            .button("Open Campaign Evolved container (.utoc)...")
                            .clicked()
                        {
                            ui.close_menu();
                            add_kit_then = Some(LoadKind::Container);
                        }
                        ui.separator();
                        ui.menu_button("Recent", |ui| {
                            if recents.is_empty() {
                                ui.add_enabled(false, egui::Button::new("No recent folders"));
                            }
                            for path in &recents {
                                let full = path.display().to_string();
                                if ui
                                    .button(recent_folder_menu_label(path))
                                    .on_hover_text(full)
                                    .clicked()
                                {
                                    ui.close_menu();
                                    add_kit_then = Some(LoadKind::Recent(path.clone()));
                                }
                            }
                        });
                    })
                    .response
                    .on_hover_text("Open another game in its own kit");
                });
            });
        if let Some(index) = activate {
            self.active = index;
        }
        if let Some(id) = close {
            self.request_close_action(PendingCloseAction::CloseKit(id), ctx);
        }
        if let Some(kind) = add_kit_then {
            // No need to add a kit here: the loaders route to one themselves,
            // reusing an empty workspace and adding a kit only when the current
            // one is occupied.
            match kind {
                LoadKind::Folder => self.begin_load_folder(ctx.clone()),
                LoadKind::SingleFile => self.begin_load_single(ctx.clone()),
                LoadKind::Monolithic => self.begin_load_monolithic(ctx.clone()),
                LoadKind::Container => self.begin_load_iostore_container(ctx.clone()),
                LoadKind::Recent(path) => self.load_recent_folder(path, ctx.clone()),
            }
        }
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
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            self.find.open = true;
            self.find.focus_query = true;
        }
        self.refresh_find(ctx);
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
        self.draw_tag_reference_picker_window(ctx);
        self.draw_settings_window(ctx);
        self.draw_tool_commands_window(ctx);
        self.draw_new_tag_window(ctx);
        self.draw_import_tag_window(ctx);
        self.draw_import_discard_confirm(ctx);
        self.draw_overwrite_confirm_window(ctx);
        self.draw_tag_conversion_window(ctx);
        self.draw_folder_conversion_window(ctx);
        self.draw_about_window(ctx);
        self.draw_query_results_window(ctx);
        self.draw_tag_diff_window(ctx);
        self.draw_content_explorer_window(ctx);
        self.draw_keyword_chooser_window(ctx);
        self.draw_field_value_search_window(ctx);
        self.draw_find_window(ctx);
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

/// Which loader the kit strip's "+" menu should start after adding a kit.
#[derive(Clone)]
enum LoadKind {
    Folder,
    SingleFile,
    Monolithic,
    Container,
    Recent(std::path::PathBuf),
}
