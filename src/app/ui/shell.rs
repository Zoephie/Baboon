//! Main application shell: menus, toolbar, sidebar, tabs, terminal, and status areas.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;
use super::recents::draw_recent_folders_menu;

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
                        if ui.button("Open Baboon Project...").clicked() {
                            ui.close_menu();
                            self.begin_open_campaign_project(ctx.clone());
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
                        let mut recent_action = None;
                        ui.menu_button("Recent Folders", |ui| {
                            recent_action = draw_recent_folders_menu(ui, &self.recent_folders);
                        });
                        if let Some(action) = recent_action {
                            self.apply_recent_action(action, ctx);
                        }
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
                            self.defer_file_action(DeferredFileAction::SaveCurrentTag, ctx);
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
                                    self.can_poke_current_tag(),
                                    egui::Button::new("Poke Current Tag...    Ctrl+P"),
                                )
                                .on_hover_text(
                                    "Apply supported changes to this already-loaded tag in the verified Campaign Evolved process",
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.defer_file_action(DeferredFileAction::PokeCurrentTag, ctx);
                            }
                            if self.last_poke.is_some()
                                && ui
                                    .add_enabled(
                                        !self.poke_undo_running,
                                        egui::Button::new("Undo Last Poke"),
                                    )
                                    .on_hover_text(
                                        "Restore the bytes from Baboon's last verified runtime poke",
                                    )
                                    .clicked()
                            {
                                ui.close_menu();
                                self.begin_undo_last_poke(ctx.clone());
                            }
                            if ui
                                .add_enabled(
                                    self.kits[self.active].parsed_tags.values().any(|d| d.dirty.is_set())
                                        || self.kits[self.active]
                                            .campaign_project
                                            .as_ref()
                                            .is_some_and(|project| !project.overlays.is_empty()),
                                    egui::Button::new("Export Mod..."),
                                )
                                .on_hover_text(
                                    "Bundle every modified project tag into one portable mod overlay and .baboon recovery file",
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.defer_file_action(DeferredFileAction::ExportMod, ctx);
                            }
                            // The same review, opened to look rather than to
                            // export -- which is how you check what a workspace
                            // is carrying before quitting.
                            if ui
                                .add_enabled(
                                    self.kits[self.active].has_unwritten_modifications(),
                                    egui::Button::new("Review Changes..."),
                                )
                                .on_hover_text(
                                    "See every edit this workspace is holding that is not written into the game",
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.review_changes();
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
                            // Deferred, per upstream: the close runs after the
                            // editor renders, so an edit committed by the menu
                            // taking focus is applied before the dirty check.
                            if let Some(key) = self.kits[self.active].selected_key.clone() {
                                self.defer_file_action(
                                    DeferredFileAction::Close(PendingCloseAction::CloseTab(key)),
                                    ctx,
                                );
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
                            self.defer_file_action(
                                DeferredFileAction::Close(PendingCloseAction::CloseAllTabs),
                                ctx,
                            );
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
                        ui.separator();
                        // The same two actions as the tab context menu and the
                        // toolbar, spelled out. An unlabelled trash icon among
                        // the tool launchers is not where anyone looks for this.
                        let selected = self.kits[self.active].selected_key.clone();
                        let discardable = selected
                            .as_deref()
                            .is_some_and(|key| self.tag_has_discardable_changes(self.active, key));
                        if ui
                            .add_enabled(
                                discardable,
                                egui::Button::new("Discard Unsaved Changes"),
                            )
                            .on_hover_text(
                                "Return the current tag to the way its source has it",
                            )
                            .clicked()
                        {
                            ui.close_menu();
                            if let Some(key) = selected {
                                self.discard_tag_changes(self.active, &key, ctx);
                            }
                        }
                        if self.current_source_is_campaign_project_capable(self.active) {
                            let stashed = self.stashed_campaign_tags(self.active);
                            let unsaved = self.kits[self.active]
                                .parsed_tags
                                .values()
                                .filter(|document| document.dirty.is_set())
                                .count();
                            if ui
                                .add_enabled(
                                    !stashed.is_empty() || unsaved > 0,
                                    egui::Button::new("Clear All Unsaved Modifications..."),
                                )
                                .on_hover_text(
                                    "Return every tag in this workspace to the way the game \
                                     ships it, including edits stashed in earlier sessions",
                                )
                                .on_disabled_hover_text(
                                    "This workspace has no unsaved modifications",
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.clear_stash_confirm = Some(ClearStashConfirm {
                                    kit: self.active_kit_id(),
                                    stashed,
                                    unsaved,
                                });
                            }
                        }
                    });
                    ui.menu_button("Tools", |ui| {
                        if ui.button("Run Tool...").clicked() {
                            ui.close_menu();
                            self.tool_commands.open = true;
                        }
                        self.draw_monitor_tools_menu(ui);
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
                                    kit: self.active_kit_id(),
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
                        // The browser view belongs to a workspace, so this
                        // menu shows and sets the focused kit's — matching the
                        // Folders/Groups buttons in that kit's own toolbar.
                        let kit = &mut self.kits[self.active];
                        if ui
                            .selectable_label(kit.browser_mode == BrowserMode::Folders, "Folders")
                            .clicked()
                        {
                            kit.browser_mode = BrowserMode::Folders;
                            ui.close_menu();
                        }
                        if ui
                            .selectable_label(kit.browser_mode == BrowserMode::Groups, "Tag Groups")
                            .clicked()
                        {
                            kit.browser_mode = BrowserMode::Groups;
                            ui.close_menu();
                        }
                        ui.separator();
                        ui.menu_button(format!("Sort by: {}", kit.browser_sort.label()), |ui| {
                            for option in BrowserSort::ALL {
                                if ui
                                    .selectable_label(kit.browser_sort == option, option.label())
                                    .clicked()
                                {
                                    kit.browser_sort = option;
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
                            self.begin_check_for_updates(ctx.clone(), false);
                            ui.close_menu();
                        }
                        if let Some(update) = self.available_update.as_ref() {
                            let label = format!("Update available: {}...", update.short_name());
                            let url = update.release_url.clone();
                            if ui.button(label).clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(url));
                                ui.close_menu();
                            }
                        }
                    });
                    ui.menu_button("Editing Kits", |ui| {
                        ui.set_min_width(EDITING_KIT_MENU_MIN_WIDTH);
                        let entries = visible_editing_kit_menu_entries(
                            &self.custom_editing_kit_profiles,
                            &self.editing_kit_validation,
                        );
                        let total_rows = entries.len();
                        for (index, entry) in entries.into_iter().enumerate() {
                            match entry {
                                EditingKitMenuEntry::Custom(profile) => {
                                    let validation =
                                        self.editing_kit_validation.custom(&profile.id);
                                    let enabled = validation.is_ok();
                                    let tooltip = validation
                                        .as_ref()
                                        .map(|layout| {
                                            format!(
                                                "Load {} from {}",
                                                profile.name,
                                                layout.root.display()
                                            )
                                        })
                                        .unwrap_or_else(|error| {
                                            format!("{} is unavailable: {error}", profile.name)
                                        });
                                    let texture = self
                                        .custom_editing_kit_texture(ui.ctx(), &profile)
                                        .cloned();
                                    let response = editing_kit_menu_row(
                                        ui,
                                        &profile.name,
                                        "EK",
                                        texture.as_ref(),
                                        profile.icon.is_none() || texture.is_none(),
                                        enabled,
                                    );
                                    let response = if enabled {
                                        response.on_hover_text(tooltip)
                                    } else {
                                        response.on_disabled_hover_text(tooltip)
                                    };
                                    if response.clicked() {
                                        ui.close_menu();
                                        self.load_custom_editing_kit_profile(
                                            profile,
                                            ctx.clone(),
                                        );
                                    }
                                }
                                EditingKitMenuEntry::BuiltIn(shortcut) => {
                                    let texture =
                                        self.game_banner_texture(ui.ctx(), shortcut.game).cloned();
                                    let configured_path = self
                                        .editing_kit_paths
                                        .get(shortcut.game)
                                        .expect("validated built-in path");
                                    let tooltip = format!(
                                        "Load {} from {}",
                                        shortcut.label,
                                        configured_path.display()
                                    );
                                    if editing_kit_menu_row(
                                        ui,
                                        game_display_name(shortcut.game),
                                        shortcut.fallback,
                                        texture.as_ref(),
                                        false,
                                        true,
                                    )
                                    .on_hover_text(tooltip)
                                    .clicked()
                                    {
                                        ui.close_menu();
                                        self.load_editing_kit_shortcut(shortcut, ctx.clone());
                                    }
                                }
                            }
                            if index + 1 < total_rows {
                                ui.separator();
                            }
                        }
                        if total_rows == 0 {
                            ui.add_enabled(false, egui::Button::new("No configured editing kits"));
                        }
                        ui.separator();
                        if ui.button("Editing Kit Settings...").clicked() {
                            self.settings_tab = SettingsTab::EditingKits;
                            self.settings_open = true;
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
                    // Anchored to the right edge, out of the way of the status
                    // text and the progress bars that share this row. The
                    // status line expires on a timer, so an update found by the
                    // silent startup check would otherwise scroll past unread;
                    // this link stays until the next check clears it.
                    if let Some(update) = self.available_update.as_ref() {
                        let label = format!("Update available: {}", update.short_name());
                        let url = update.release_url.clone();
                        let hover = match update.channel {
                            UpdateChannel::Stable => "Open the release page on GitHub",
                            UpdateChannel::Development => {
                                "Open the latest development build on GitHub"
                            }
                        };
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // An explicit colour beats the app-wide
                            // `override_text_color`, which would otherwise
                            // flatten both this and the link colour to
                            // ordinary body text. `strong()` only brightens;
                            // the weight comes from the bold family, at the
                            // body size of the row it sits in.
                            ui.hyperlink_to(
                                RichText::new(label)
                                    .font(bold_font(12.0))
                                    .color(good_news()),
                                &url,
                            )
                            .on_hover_text(hover);
                        });
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

        egui::CentralPanel::default()
            .frame(Frame::none().fill(editor_bg()))
            .show(ctx, |ui| {
                self.draw_kit_tiles(ui, ctx);
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
            // Apply to the kit the picker was opened from. A closed kit drops
            // the edit rather than letting it land somewhere else.
            let kit = self
                .color_popup_kit
                .and_then(|kit| self.resolve_kit(kit))
                .unwrap_or(self.active);
            match result {
                ColorPopupResult::FieldEdit { tag_key, edit } => {
                    if let Some(doc) = self.kits[kit].parsed_tags.get_mut(&tag_key) {
                        doc.journal.begin_edit(&doc.tag, "Edit color");
                        if let Some(status) =
                            apply_pending_edits(&mut doc.tag, vec![edit], &mut doc.dirty).status
                        {
                            self.status = status;
                        }
                        doc.journal.end_edit_window();
                    }
                }
                ColorPopupResult::ShaderOp { tag_key, op } => {
                    if let Some(doc) = self.kits[kit].parsed_tags.get_mut(&tag_key) {
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
                    if let Some(doc) = self.kits[kit].parsed_tags.get_mut(&tag_key) {
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
                    if let Some(doc) = self.kits[kit].parsed_tags.get_mut(&tag_key) {
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
            let kit = self
                .function_popup_kit
                .and_then(|kit| self.resolve_kit(kit))
                .unwrap_or(self.active);
            if let Some(doc) = self.kits[kit].parsed_tags.get_mut(&batch.tag_key) {
                if !batch.edits.is_empty() || !batch.data_ops.is_empty() {
                    doc.journal.begin_edit(&doc.tag, "Edit function");
                }
                if let Some(status) =
                    apply_pending_edits(&mut doc.tag, batch.edits, &mut doc.dirty).status
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
        // A referenced sound on a container source resolves to its own Wwise
        // binding first, and queues an ordinary play/extract from there — so it
        // must run before both drains below, not after.
        self.process_ce_sound_ref();
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


    /// Clear the status line once its message has been up for a while.
    ///
    /// Runs after the worker drain so a message set this frame is timed from
    /// this frame. Progress states are rendered from their own fields rather
    /// than from `status`, so expiring it never blanks a running scan.
    fn expire_status(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        if self.status != self.status_shown {
            self.status_shown = self.status.clone();
            self.status_changed_at = now;
        }
        if self.status.is_empty() {
            return;
        }
        let elapsed = now - self.status_changed_at;
        if elapsed >= STATUS_LINGER_SECS {
            self.status.clear();
            self.status_shown.clear();
        } else {
            // Nothing else may be animating, so ask for the frame that will
            // do the clearing rather than waiting for the next interaction.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                STATUS_LINGER_SECS - elapsed,
            ));
        }
    }

    pub(super) fn run_deferred_file_action(&mut self, ctx: &egui::Context) {
        match self.deferred_file_action.take() {
            Some(DeferredFileAction::SaveCurrentTag) => self.save_current_tag(),
            Some(DeferredFileAction::ExportMod) => self.export_mod(),
            Some(DeferredFileAction::PokeCurrentTag) => self.begin_poke_current_tag(ctx.clone()),
            Some(DeferredFileAction::Close(action)) => self.request_close_action(action, ctx),
            None => {}
        }
    }

    pub(in crate::app) fn defer_file_action(
        &mut self,
        action: DeferredFileAction,
        ctx: &egui::Context,
    ) {
        ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
        self.deferred_file_action = Some(action);
    }

    fn prepare_root_frame(&mut self, ctx: &egui::Context) {
        self.process_worker_messages(ctx);
        self.expire_status(ctx);
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
            self.defer_file_action(DeferredFileAction::SaveCurrentTag, ctx);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::P)) {
            self.defer_file_action(DeferredFileAction::PokeCurrentTag, ctx);
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
        self.draw_clear_stash_confirm_window(ctx);
        self.draw_mod_export_window(ctx);
        self.draw_exported_mod_window(ctx);
        self.draw_poke_window(ctx);
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

pub(super) fn recent_folder_menu_label(path: &Path) -> String {
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
