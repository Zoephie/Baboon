//! Main application shell: menus, toolbar, sidebar, tabs, terminal, and status areas.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::recents::draw_recent_folders_menu;
use super::*;
use crate::app::shell::frame::terminal_line_is_strong;
use crate::app::shell::frame::terminal_line_color;
use crate::app::kits::terminal::open_terminal_log;
use crate::app::shell::frame::draw_index_progress_bar;
use crate::app::shell::frame::editing_kit_menu_row;
use crate::app::shell::frame::editing_kit_menu_row_with_read_only;
use crate::app::shell::frame::EditingKitMenuEntry;
use crate::app::shell::frame::visible_editing_kit_menu_entries;
use crate::app::shell::frame::EDITING_KIT_MENU_MIN_WIDTH;

/// How often a progress bar is redrawn while its job runs.
const PROGRESS_REPAINT: std::time::Duration = std::time::Duration::from_millis(200);

impl Baboon {
    pub(in crate::app) fn draw_root_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        if self.shell.first_run_wizard.is_some() {
            ctx.set_zoom_factor(self.prefs.ui_scale);
            set_dark_mode(self.prefs.dark_mode);
            ctx.set_visuals(foundation_visuals());
            egui::CentralPanel::default().show(ui, |_ui| {});
            self.draw_first_run_wizard(ctx);
            return;
        }
        self.prepare_root_frame(ctx);

        self.draw_menu_bar(ui);
        self.draw_status_bar(ui);
        self.draw_entry_index_wait_notice(ctx);
        // Terminal panel — rendered AFTER status so it sits above it.
        self.draw_terminal_panel(ui);
        set_window_work_area(ctx, ui.available_rect_before_wrap());

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(editor_bg()))
            .show(ui, |ui| {
                self.draw_kit_tiles(ui, ctx);
            });
        self.draw_auxiliary_windows(ctx);
        // Every kit, not just the active one: a background kit's sidecar can be
        // dirty from edits made before the user switched away.
        let mut keyword_notice = None;
        for kit in &mut self.kits {
            kit.keywords.save_if_dirty();
            if let Some(notice) = kit.keywords.take_notice() {
                keyword_notice = Some(notice);
            }
        }
        if let Some(notice) = keyword_notice {
            self.status = notice;
        }
        self.draw_and_apply_color_popup(ctx);
        self.draw_and_apply_function_popup(ctx);
        self.process_frame_requests(ctx);
    }

    /// The top menu bar: the File, Edit, Tools, View, Help and Editing Kits
    /// menus, then the tool launcher buttons.
    fn draw_menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        egui::Panel::top("menu")
            .frame(Frame::NONE.fill(menu_bar()).inner_margin(egui::Margin {
                left: 6,
                right: 6,
                top: 2,
                bottom: 2,
            }))
            .show(ui, |ui| {
                egui::MenuBar::new().config(menu_config()).ui(ui, |ui| {
                    aligned_menu_button(ui, "File", |ui| {
                        self.draw_file_menu(ui, ctx);
                    });
                    aligned_menu_button(ui, "Edit", |ui| {
                        self.draw_edit_menu(ui, ctx);
                    });
                    aligned_menu_button(ui, "Tools", |ui| {
                        self.draw_tools_menu(ui, ctx);
                    });
                    aligned_menu_button(ui, "View", |ui| {
                        self.draw_view_menu(ui);
                    });
                    aligned_menu_button(ui, "Help", |ui| {
                        self.draw_help_menu(ui, ctx);
                    });
                    aligned_menu_button(ui, "Editing Kits", |ui| {
                        self.draw_editing_kits_menu(ui, ctx);
                    });
                    self.draw_tool_launcher_buttons(ui);
                });
            });
    }

    /// The File menu: loading sources, saving tags and projects, and closing.
    fn draw_file_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        style_list_menu(ui);
        if ui
            .add_enabled(
                !self.editing_kit_is_read_only(self.active),
                egui::Button::new("New Tag..."),
            )
            .clicked()
        {
            close_menu(ui);
            self.open_new_tag_dialog();
        }
        // One entry, two implementations. A container tag is a
        // package rather than a file, so it lands through the
        // Campaign Evolved path; a loose kit converts from
        // another game. Splitting them in the menu would make
        // the user answer a question about Baboon's internals
        // to do the same thing.
        let can_import = self.current_source_is_container() || self.can_import_tags();
        if ui
            .add_enabled(can_import, egui::Button::new("Import Tags..."))
            .on_hover_text(if self.current_source_is_container() {
                "Bring a tag file into these containers"
            } else {
                "Bring a tag, or a whole folder of them, in from another game's editing kit"
            })
            .on_disabled_hover_text("Load an editing kit or a Campaign Evolved container first")
            .clicked()
        {
            close_menu(ui);
            if self.current_source_is_container() {
                self.begin_import_tag(None);
            } else {
                self.open_tag_import_dialog(None);
            }
        }
        if ui.button("Load Tag...").clicked() {
            close_menu(ui);
            self.begin_load_single(ctx.clone());
        }
        if ui.button("Load Folder...").clicked() {
            close_menu(ui);
            self.begin_load_folder(ctx.clone());
        }
        if ui.button("Load Monolithic blob_index.dat...").clicked() {
            close_menu(ui);
            self.begin_load_monolithic(ctx.clone());
        }
        if ui
            .button("Open Campaign Evolved container (.utoc)...")
            .clicked()
        {
            close_menu(ui);
            self.begin_load_iostore_container(ctx.clone());
        }
        if ui.button("Open Baboon Project...").clicked() {
            close_menu(ui);
            self.begin_open_campaign_project(ctx.clone());
        }
        // A workspace's edits are autosaved to a recovery file
        // whether or not they are ever saved anywhere else, so
        // these write a copy the user owns and can move, back up
        // or hand to someone. Before them, the only way to get a
        // `.baboon` out of Baboon was to export a mod.
        let can_save_project = self.current_source_is_campaign_project_capable(self.active);
        let project_target = self.kits[self.active]
            .project.active
            .as_ref()
            .and_then(|project| project.project_path.clone());
        if ui
            .add_enabled(can_save_project, egui::Button::new("Save Baboon Project"))
            .on_hover_text(match project_target.as_deref() {
                Some(path) => format!("Write this workspace's changes to {}", path.display()),
                None => "Choose a .baboon file to keep this workspace's changes in".to_owned(),
            })
            .on_disabled_hover_text("Baboon projects hold changes to Campaign Evolved containers")
            .clicked()
        {
            close_menu(ui);
            self.defer_file_action(DeferredFileAction::SaveProject, ctx);
        }
        if ui
            .add_enabled(
                can_save_project,
                egui::Button::new("Save Baboon Project As..."),
            )
            .clicked()
        {
            close_menu(ui);
            self.defer_file_action(DeferredFileAction::SaveProjectAs, ctx);
        }
        ui.separator();
        let has_loaded_folder = self.loaded_tags_root().is_some();
        if ui
            .add_enabled(has_loaded_folder, egui::Button::new("Open Tags Folder"))
            .clicked()
        {
            close_menu(ui);
            self.open_loaded_tags_folder();
        }
        if ui
            .add_enabled(has_loaded_folder, egui::Button::new("Open Data Folder"))
            .clicked()
        {
            close_menu(ui);
            self.open_loaded_data_folder();
        }
        let recent_action = right_opening_menu_button(ui, "Recent Folders", 280.0, |ui| {
            style_list_menu(ui);
            draw_recent_folders_menu(ui, &self.prefs.recent_folders)
        })
        .inner
        .flatten();
        if let Some(action) = recent_action {
            close_menu(ui);
            self.apply_recent_action(action, ctx);
        }
        ui.separator();
        let save_label =
            if self.prefs.enable_chimp && self.kits[self.active].surface == KitSurface::Chimp {
                "Save Chimp Changes...    Ctrl+S"
            } else {
                "Save Current Tag    Ctrl+S"
            };
        if icon_text_button(
            ui,
            ButtonIcon::Save,
            save_label,
            !self.editing_kit_is_read_only(self.active),
        )
        .clicked()
        {
            close_menu(ui);
            self.defer_file_action(DeferredFileAction::SaveCurrentTag, ctx);
        }
        if ui
            .add_enabled(
                self.kits[self.active].selected_key.is_some()
                    && !self.editing_kit_is_read_only(self.active),
                egui::Button::new("Save Current Tag As..."),
            )
            .clicked()
        {
            close_menu(ui);
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
                close_menu(ui);
                self.defer_file_action(DeferredFileAction::PokeCurrentTag, ctx);
            }
            if self.poke.last_poke.is_some()
                && ui
                    .add_enabled(!self.poke.poke_undo_running, egui::Button::new("Undo Last Poke"))
                    .on_hover_text("Restore the bytes from Baboon's last verified runtime poke")
                    .clicked()
            {
                close_menu(ui);
                self.begin_undo_last_poke(ctx.clone());
            }
            if ui
                .add_enabled(
                    self.kits[self.active].parsed_tags.values().any(|d| d.dirty.is_set())
                        || self.kits[self.active]
                            .project.active
                            .as_ref()
                            .is_some_and(|project| !project.overlays.is_empty()),
                    egui::Button::new("Export Mod..."),
                )
                .on_hover_text(
                    "Bundle every modified project tag into one portable mod overlay, with a copy of this project saved beside it",
                )
                .clicked()
            {
                close_menu(ui);
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
                close_menu(ui);
                self.review_changes();
            }
            // Expert-gated because it is the one action here
            // that writes tens of thousands of files: useful
            // for getting the tag set out to diff or grep, and
            // not something to trip over while editing.
            if self.prefs.expert_mode
                && ui
                    .add_enabled(
                        self.export.container_dump_job.is_none(),
                        egui::Button::new("Extract All Tags to Folder\u{2026}"),
                    )
                    .on_hover_text(
                        "Expert feature: write every tag these containers ship to a folder laid out like an editing kit. Tens of thousands of files \u{2014} this takes a while",
                    )
                    .on_disabled_hover_text(
                        "An extraction is already running",
                    )
                    .clicked()
            {
                close_menu(ui);
                self.defer_file_action(
                    DeferredFileAction::ExtractAllContainerTags,
                    ctx,
                );
            }
        }
        ui.separator();
        if ui
            .add_enabled(
                self.kits[self.active].selected_key.is_some(),
                egui::Button::new("Close Current Tag    Ctrl+W"),
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
            close_menu(ui);
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
            close_menu(ui);
        }
        ui.separator();
        // Goes through the same close request as the window's own close
        // button, so unsaved tags are still offered for saving first.
        if ui.button("Exit").clicked() {
            close_menu(ui);
            self.defer_file_action(DeferredFileAction::Close(PendingCloseAction::CloseApp), ctx);
        }
    }

    /// The Edit menu: undo and redo, and discarding unsaved changes.
    fn draw_edit_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        style_list_menu(ui);
        if ui
            .add_enabled(self.can_undo_current(), egui::Button::new("Undo    Ctrl+Z"))
            .clicked()
        {
            close_menu(ui);
            self.undo_current_tag();
        }
        if ui
            .add_enabled(
                self.can_redo_current(),
                egui::Button::new("Redo    Ctrl+Shift+Z"),
            )
            .clicked()
        {
            close_menu(ui);
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
            .add_enabled(discardable, egui::Button::new("Discard Unsaved Changes"))
            .on_hover_text("Return the current tag to the way its source has it")
            .clicked()
        {
            close_menu(ui);
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
                .on_disabled_hover_text("This workspace has no unsaved modifications")
                .clicked()
            {
                close_menu(ui);
                self.mods.clear_stash_confirm = Some(ClearStashConfirm {
                    kit: self.active_kit_id(),
                    stashed,
                    unsaved,
                });
            }
        }

        ui.separator();
        if icon_text_button(ui, ButtonIcon::Settings, "Settings...", true).clicked() {
            self.shell.settings_open = true;
            close_menu(ui);
        }
    }

    /// The Tools menu, in four sections: the tool and asset launchers, actions
    /// on the current tag, searches and listings across the workspace, and the
    /// indexes those searches run on.
    fn draw_tools_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        style_list_menu(ui);
        if ui.button("Run Tool...").clicked() {
            close_menu(ui);
            self.kit_tools.tool_commands.open = true;
        }
        self.draw_monitor_tools_menu(ui);
        self.draw_assets_tools_menu(ui);

        ui.separator();
        let has_current = self.kits[self.active].selected_key.is_some();
        if ui
            .add_enabled(
                has_current,
                egui::Button::new("Find References to Current Tag"),
            )
            .clicked()
        {
            close_menu(ui);
            if let Some(key) = self.kits[self.active].selected_key.clone() {
                self.show_references_for(&key);
            }
        }
        if ui
            .add_enabled(
                has_current,
                egui::Button::new("Explore References to Current Tag..."),
            )
            .clicked()
        {
            close_menu(ui);
            if let Some(key) = self.kits[self.active].selected_key.clone() {
                self.open_content_explorer(&key);
            }
        }
        if icon_text_button(ui, ButtonIcon::Compare, "Compare Tags...", has_current).clicked() {
            close_menu(ui);
            if let Some(key) = self.kits[self.active].selected_key.clone() {
                self.compare.tag_diff = Some(TagDiffState {
                    kit: self.active_kit_id(),
                    a_key: key,
                    source: TagCompareSource::OpenTag,
                    b_kit: None,
                    b_key: None,
                    b_path: None,
                    comparison_kit_root: None,
                    git_history: GitHistoryState::default(),
                    error: None,
                    filters: TagDiffFilters::default(),
                    swapped: false,
                    results: None,
                    git_pending: None,
                });
            }
        }
        let can_fix_dependencies = has_current
            && self
                .source()
                .is_some_and(|source| matches!(source.source, TagSource::LooseFolder { .. }));
        if ui
            .add_enabled(
                can_fix_dependencies,
                egui::Button::new("Fix Tag Dependencies"),
            )
            .clicked()
        {
            close_menu(ui);
            self.fix_current_tag_dependencies();
        }

        ui.separator();
        if ui.button("Search Field Values...").clicked() {
            close_menu(ui);
            self.search.field_value_search_open = true;
        }
        if ui.button("Browse Keywords...").clicked() {
            close_menu(ui);
            self.browser.keyword_chooser_open = true;
        }
        if ui.button("Find Unreferenced Tags...").clicked() {
            close_menu(ui);
            self.show_unreferenced_tags();
        }
        if ui.button("List Scenario Map IDs...").clicked() {
            close_menu(ui);
            self.show_map_ids(ctx);
        }
        if ui.button("List Sounds by Class...").clicked() {
            close_menu(ui);
            self.show_sounds_by_class(ctx);
        }
        if ui.button("List Uncompressed Sounds...").clicked() {
            close_menu(ui);
            self.show_uncompressed_sounds(ctx);
        }

        ui.separator();
        {
            // Loose folders and Campaign Evolved containers can
            // both be indexed; cache sources cannot.
            let indexable = self.source().is_some_and(|source| {
                matches!(
                    source.source,
                    TagSource::LooseFolder { .. } | TagSource::IoStoreContainerSet { .. }
                )
            });
            let has_index = self
                .source()
                .is_some_and(|source| source.reverse_dependencies.is_some());
            let label = if self.kits[self.active].index_jobs.building_references {
                "Building Reference Index…"
            } else if has_index {
                "Rebuild Reference Index"
            } else {
                "Build Reference Index"
            };
            if ui
                .add_enabled(
                    indexable && !self.kits[self.active].index_jobs.building_references,
                    egui::Button::new(label),
                )
                .on_hover_text("Which tags reference which, for the reference searches above")
                .clicked()
            {
                close_menu(ui);
                self.begin_build_reverse_dependencies(ctx.clone(), true);
            }
        }
        // Regenerate the tag index: force a fresh full scan and overwrite the
        // cached index file.
        let can_regen = self
            .source()
            .map(|s| matches!(s.source, TagSource::LooseFolder { .. }) && s.game.is_some())
            .unwrap_or(false);
        if ui
            .add_enabled(
                can_regen && !self.kits[self.active].scanning_entries,
                egui::Button::new("Regenerate Tag Index"),
            )
            .on_hover_text("Rescan every tag in the folder from disk")
            .clicked()
        {
            close_menu(ui);
            // Clear cached entries so the scan runs fresh.
            if let Some(s) = self.source_mut() {
                s.all_entries.clear();
                s.group_tree = crate::core::source::build_group_tree(&[]);
                s.reverse_dependencies = None;
            }
            self.kits[self.active].field_index.invalidate();
            self.begin_scan_all_entries_with_label(ctx.clone(), "Rebuilding index...");
        }
        let can_refresh_browser = self.source().is_some_and(|source| {
            matches!(source.source, TagSource::LooseFolder { .. }) && source.game.is_some()
        });
        if ui
            .add_enabled(
                can_refresh_browser
                    && !self.kits[self.active].scanning_entries
                    && !self.kits[self.active].index_jobs.refreshing,
                egui::Button::new("Refresh Tag Browser"),
            )
            .clicked()
        {
            close_menu(ui);
            self.refresh_tag_browser(ctx.clone());
        }
    }

    /// The View menu: browser mode and sort, display toggles, expert mode, and
    /// the terminal.
    fn draw_view_menu(&mut self, ui: &mut Ui) {
        style_list_menu(ui);
        // The browser view belongs to a workspace, so this
        // menu shows and sets the focused kit's — matching the
        // Folders/Groups buttons in that kit's own toolbar.
        let kit = &mut self.kits[self.active];
        if ui
            .selectable_label(kit.browser.mode == BrowserMode::Folders, "Folders")
            .clicked()
        {
            kit.browser.mode = BrowserMode::Folders;
            close_menu(ui);
        }
        if ui
            .selectable_label(kit.browser.mode == BrowserMode::Groups, "Tag Groups")
            .clicked()
        {
            kit.browser.mode = BrowserMode::Groups;
            close_menu(ui);
        }
        ui.separator();
        let selected_sort = right_opening_menu_button(
            ui,
            format!("Sort by: {}", kit.browser.sort.label()),
            220.0,
            |ui| {
                style_list_menu(ui);
                for option in BrowserSort::ALL {
                    if ui
                        .selectable_label(kit.browser.sort == option, option.label())
                        .clicked()
                    {
                        return Some(option);
                    }
                }
                None
            },
        )
        .inner
        .flatten();
        if let Some(option) = selected_sort {
            kit.browser.sort = option;
            close_menu(ui);
        }
        ui.separator();
        ui.checkbox(&mut self.prefs.show_browser_prefixes, "Show [tag]/[folder]");
        ui.checkbox(&mut self.prefs.show_block_sizes, "Show block sizes");
        ui.checkbox(&mut self.prefs.angles_in_degrees, "Angles in degrees")
            .on_hover_text(
                "Angle fields hold radians on disk. Guerilla and the other Halo \
                 tools show them in degrees, and so does Baboon — turn this off to \
                 read and type the stored radians instead.",
            );
        ui.checkbox(
            &mut self.prefs.scroll_to_cycle_dropdowns,
            "Scroll wheel cycles dropdowns",
        );
        ui.checkbox(&mut self.prefs.expert_mode, "Expert mode");
        ui.separator();
        let terminal_enabled = self.kits[self.active].terminal.work_dir.is_some();
        if ui
            .add_enabled(
                terminal_enabled,
                egui::Button::selectable(self.kits[self.active].terminal.open, "Terminal"),
            )
            .clicked()
        {
            self.kits[self.active].terminal.open = !self.kits[self.active].terminal.open;
            self.remember_terminal_open_for_game();
            close_menu(ui);
        }
    }

    /// The Help menu: About, documentation, tutorials, and the update check.
    fn draw_help_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        style_list_menu(ui);
        if ui.button("About...").clicked() {
            self.help.help_panel_tab = HelpPanelTab::About;
            self.help.about_open = true;
            close_menu(ui);
        }
        if icon_text_button(ui, ButtonIcon::Doc, "Doc...", true).clicked() {
            self.help.help_panel_tab = HelpPanelTab::Doc;
            self.help.about_open = true;
            close_menu(ui);
        }
        if ui.button("Tutorials...").clicked() {
            self.help.help_panel_tab = HelpPanelTab::Tutorials;
            self.help.about_open = true;
            close_menu(ui);
        }
        if ui.button("Tag Compatibility...").clicked() {
            self.help.help_panel_tab = HelpPanelTab::TagCompat;
            self.help.about_open = true;
            close_menu(ui);
        }
        if ui.button("Map Names...").clicked() {
            self.help.help_panel_tab = HelpPanelTab::MapNames;
            self.help.about_open = true;
            close_menu(ui);
        }
        if ui.button("Check for updates").clicked() {
            self.begin_check_for_updates(ctx.clone(), false);
            close_menu(ui);
        }
        if let Some(update) = self.shell.available_update.as_ref() {
            let label = format!("Update available: {}...", update.short_name());
            let url = update.release_url.clone();
            if ui.button(label).clicked() {
                ctx.open_url(egui::OpenUrl::new_tab(url));
                close_menu(ui);
            }
        }
    }

    /// The Editing Kits menu: one entry per configured editing kit.
    fn draw_editing_kits_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.set_min_width(EDITING_KIT_MENU_MIN_WIDTH);
        let entries = visible_editing_kit_menu_entries(
            &self.prefs.custom_editing_kit_profiles,
            &self.kit_tools.editing_kit_validation,
        );
        let total_rows = entries.len();
        for (index, entry) in entries.into_iter().enumerate() {
            match entry {
                EditingKitMenuEntry::Custom(profile) => {
                    let validation = self.kit_tools.editing_kit_validation.custom(&profile.id);
                    let enabled = validation.is_ok();
                    let tooltip = validation
                        .as_ref()
                        .map(|layout| {
                            format!(
                                "Load {} from {}",
                                profile.name,
                                profile_location(&profile, Some(layout)).display()
                            )
                        })
                        .unwrap_or_else(|error| {
                            format!("{} is unavailable: {error}", profile.name)
                        });
                    let texture =
                        self.workspace_banner_texture(ui.ctx(), profile.game_id(), Some(&profile.id));
                    let response = editing_kit_menu_row_with_read_only(
                        ui,
                        &profile.name,
                        "EK",
                        texture.as_ref(),
                        texture.is_none(),
                        enabled,
                        profile.read_only && !profile.is_campaign_evolved(),
                    );
                    let response = if enabled {
                        response.on_hover_text(tooltip)
                    } else {
                        response.on_disabled_hover_text(tooltip)
                    };
                    if response.clicked() {
                        close_menu(ui);
                        self.load_custom_editing_kit_profile(profile, ctx.clone());
                    }
                }
                EditingKitMenuEntry::BuiltIn(shortcut) => {
                    let texture = self.game_banner_texture(ui.ctx(), Some(shortcut.game)).cloned();
                    let configured_path = self
                        .prefs
                        .editing_kit_paths
                        .get(shortcut.game.as_str())
                        .expect("validated built-in path");
                    let tooltip =
                        format!("Load {} from {}", shortcut.label, configured_path.display());
                    if editing_kit_menu_row(
                        ui,
                        shortcut.game.display_name(),
                        shortcut.fallback,
                        texture.as_ref(),
                        false,
                        true,
                    )
                    .on_hover_text(tooltip)
                    .clicked()
                    {
                        close_menu(ui);
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
        if icon_text_button(ui, ButtonIcon::Settings, "Editing Kit Settings...", true).clicked() {
            self.shell.settings_tab = SettingsTab::EditingKits;
            self.shell.settings_open = true;
            close_menu(ui);
        }
    }

    /// The status bar: the status line, index and job progress, the update
    /// link and the workspace's project.
    fn draw_status_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        egui::Panel::bottom("status")
            .frame(Frame::NONE.fill(menu_bar()).inner_margin(egui::Margin {
                left: 6,
                right: 6,
                top: 2,
                bottom: 2,
            }))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Status").strong());
                    ui.separator();
                    if self.kits[self.active].scanning_entries {
                        let progress = self.kits[self.active].index_jobs.entry_progress.as_ref();
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
                    } else if self.kits[self.active].index_jobs.building_references {
                        let progress = self.kits[self.active]
                            .index_jobs
                            .reference_progress
                            .as_ref();
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
                    // Additive rather than part of the chain above: the
                    // extraction outlives whatever the user does next, and its
                    // bar is the only place a cancel is reachable from.
                    if let Some(job) = &self.export.container_dump_job {
                        let (fraction, done, total) = (job.fraction(), job.done, job.total);
                        let remaining = job.remaining();
                        ui.separator();
                        ui.label(RichText::new("Extracting tags").strong())
                            // Where it is writing. The folder was chosen minutes
                            // ago in a native dialog and is nowhere else on
                            // screen once the confirm has closed.
                            .on_hover_text(format!("Writing to {}", job.output.display()));
                        draw_index_progress_bar(
                            ui,
                            220.0,
                            Some(fraction),
                            &format!("{done} / {total} tags"),
                        );
                        if let Some(remaining) = remaining {
                            ui.label(
                                RichText::new(format!("{} left", format_remaining(remaining)))
                                    .color(subtle_dark())
                                    .small(),
                            );
                        }
                        if ui.small_button("Cancel").clicked() {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                        // A few times a second moves the bar and the estimate;
                        // every frame kept the app at full frame rate for the
                        // whole of a multi-minute extraction.
                        ctx.request_repaint_after(PROGRESS_REPAINT);
                    }
                    if let Some(progress) = &self.tag_ops.folder_refactor {
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
                        // An indeterminate bar asks for its own frames while
                        // it animates.
                        ctx.request_repaint_after(PROGRESS_REPAINT);
                    }
                    // Anchored to the right edge, out of the way of the status
                    // text and the progress bars that share this row. The
                    // status line expires on a timer, so an update found by the
                    // silent startup check would otherwise scroll past unread;
                    // this link stays until the next check clears it.
                    let update = self.shell.available_update.clone();
                    // Which `.baboon` this workspace's changes belong to, and
                    // where they are actually being kept. Autosave and Save write
                    // different files, and a workspace that has never been saved
                    // writes only the recovery file — none of which was visible
                    // anywhere before.
                    let project = self
                        .current_source_is_campaign_project_capable(self.active)
                        .then(|| self.kits[self.active].project.active.as_ref())
                        .flatten()
                        // A workspace with neither a project file nor a stash has
                        // nothing to say here, and saying it anyway on every
                        // Campaign Evolved kit would just be furniture.
                        .filter(|project| {
                            project.project_path.is_some() || !project.overlays.is_empty()
                        })
                        .map(|project| {
                            let mut hover = match project.project_path.as_deref() {
                                Some(path) => format!("Baboon project: {}", path.display()),
                                None => "This workspace has no saved Baboon project yet — use \
                                         File > Save Baboon Project"
                                    .to_owned(),
                            };
                            hover.push_str(&format!(
                                "\nAutosaved to {}",
                                project.recovery_path.display()
                            ));
                            (format!("Project: {}", project.label()), hover)
                        });
                    if update.is_some() || project.is_some() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if let Some(update) = update {
                                let label = format!("Update available: {}", update.short_name());
                                let hover = match update.channel {
                                    UpdateChannel::Stable => "Open the release page on GitHub",
                                    UpdateChannel::Development => {
                                        "Open the latest development build on GitHub"
                                    }
                                };
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
                                    &update.release_url,
                                )
                                .on_hover_text(hover);
                            }
                            if let Some((label, hover)) = project {
                                ui.label(RichText::new(label).small().color(subtle_dark()))
                                    .on_hover_text(hover);
                            }
                        });
                    }
                });
            });
    }

    /// The "please wait" window shown while the active kit is still indexing.
    fn draw_entry_index_wait_notice(&mut self, ctx: &egui::Context) {
        if self.kit_tools.show_entry_index_wait_notice
            && (self.kits[self.active].scanning_entries
                || self.kits[self.active].index_jobs.references_for_entry_index)
        {
            let mut open = self.kit_tools.show_entry_index_wait_notice;
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
                        let progress = self.kits[self.active].index_jobs.entry_progress.as_ref();
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
                    } else if self.kits[self.active].index_jobs.references_for_entry_index {
                        ui.label(RichText::new("Building reference index...").strong());
                        if let Some(progress) = self.kits[self.active]
                            .index_jobs
                            .reference_progress
                            .as_ref()
                        {
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
            self.kit_tools.show_entry_index_wait_notice = open && !hide_notice;
        }
    }

    /// The terminal panel, when the active kit has it open.
    fn draw_terminal_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        if self.kits[self.active].terminal.open {
            let work_dir_label = self.kits[self.active]
                .terminal.work_dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            egui::Panel::bottom("terminal")
                .resizable(true)
                .default_size(180.0)
                .size_range(90.0..=600.0)
                .frame(
                    Frame::NONE
                        .fill(foundation_group_bg())
                        .inner_margin(egui::Margin {
                            left: 6,
                            right: 6,
                            top: 4,
                            bottom: 4,
                        }),
                )
                .show(ui, |ui| {
                    // Header pinned to the top of the panel.
                    egui::Panel::top("terminal_header")
                        .frame(Frame::NONE)
                        .show(ui, |ui| {
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
                                            self.kits[self.active].terminal.open = false;
                                            self.remember_terminal_open_for_game();
                                        }
                                        if icon_button(
                                            ui,
                                            ButtonIcon::Clear,
                                            "Clear terminal",
                                            true,
                                            text_dark(),
                                        )
                                        .clicked()
                                        {
                                            self.kit_tools.terminal.lines.clear();
                                        }
                                        let open_log_enabled =
                                            self.kit_tools.terminal.last_log_path.is_some();
                                        let mut open_log_button = ui.add_enabled(
                                            open_log_enabled,
                                            egui::Button::new(
                                                RichText::new("Open full log").small(),
                                            ),
                                        );
                                        if let Some(path) = self.kit_tools.terminal.last_log_path.as_ref() {
                                            open_log_button = open_log_button
                                                .on_hover_text(path.display().to_string());
                                        }
                                        if open_log_button.clicked()
                                            && let Some(path) = self.kit_tools.terminal.last_log_path.clone()
                                            && let Err(error) = open_terminal_log(&path)
                                        {
                                            self.status = error;
                                        }
                                        if self.kit_tools.terminal.running {
                                            if self.kit_tools.terminal.process.is_some()
                                                && ui.small_button("Stop").clicked()
                                            {
                                                self.stop_terminal_command();
                                            }
                                            let running_label = self
                                                .kit_tools.terminal
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
                    egui::Panel::bottom("terminal_input")
                        .frame(Frame::NONE)
                        .show(ui, |ui| {
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
                                    !self.kit_tools.terminal.running,
                                    egui::TextEdit::singleline(&mut self.kit_tools.terminal.input)
                                        .desired_width(text_w)
                                        .font(egui::TextStyle::Monospace)
                                        .hint_text(placeholder_text("tool <command> …")),
                                );
                                if self.kit_tools.terminal.refocus_input && !self.kit_tools.terminal.running {
                                    resp.request_focus();
                                    self.kit_tools.terminal.refocus_input = false;
                                }
                                let run_clicked = ui
                                    .add_enabled(!self.kit_tools.terminal.running, egui::Button::new("Run"))
                                    .clicked();
                                let enter = lost_focus_once(&resp)
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if resp.has_focus() && !self.kit_tools.terminal.running {
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
                            Frame::NONE
                                .fill(Color32::from_rgb(24, 24, 23))
                                .inner_margin(egui::Margin {
                                    left: 6,
                                    right: 6,
                                    top: 4,
                                    bottom: 4,
                                }),
                        )
                        .show(ui, |ui| {
                            let want_scroll_bottom = self.kit_tools.terminal.scroll_to_bottom;
                            self.kit_tools.terminal.scroll_to_bottom = false;
                            draw_terminal_output(ui, &self.kit_tools.terminal.lines, want_scroll_bottom);
                        });
                });
        }
    }

    /// Draw the color picker popup and apply what it returns to the kit it was
    /// opened from.
    fn draw_and_apply_color_popup(&mut self, ctx: &egui::Context) {
        if let Some(result) = draw_color_popup(
            ctx,
            &mut self.editor.color_popup,
            &mut self.prefs.custom_color_swatches,
            &mut self.prefs.palette_last_dir,
        ) {
            let (tag_key, label, ops) = match result {
                ColorPopupResult::FieldEdit { tag_key, edit } => {
                    let ops = DeferredOps {
                        pending: vec![edit],
                        ..DeferredOps::default()
                    };
                    (tag_key, "Edit color", ops)
                }
                ColorPopupResult::ShaderOp { tag_key, op } => {
                    let ops = DeferredOps {
                        shader_ops: vec![op],
                        ..DeferredOps::default()
                    };
                    (tag_key, "Shader edit", ops)
                }
                ColorPopupResult::ShaderParamOp { tag_key, op } => {
                    let ops = DeferredOps {
                        shader_param_ops: vec![op],
                        ..DeferredOps::default()
                    };
                    (tag_key, "Shader parameter", ops)
                }
                ColorPopupResult::H2ShaderParamOp { tag_key, op } => {
                    let ops = DeferredOps {
                        h2_shader_param_ops: vec![op],
                        ..DeferredOps::default()
                    };
                    (tag_key, "Shader parameter", ops)
                }
                ColorPopupResult::FunctionDraftColor { target, argb } => {
                    if let Some(popup) = self.editor.function_popup.as_mut() {
                        popup.apply_draft_color(target, argb);
                    }
                    return;
                }
            };
            // Apply to the kit the picker was opened from.
            if let Some(kit) = self.popup_target_kit(self.editor.color_popup_kit) {
                self.apply_doc_ops(kit, &tag_key, label, ops, UndoStep::Own);
            }
        }
        if self.editor.color_popup.is_none() {
            self.editor.color_popup_kit = None;
        }
    }

    /// Draw the function editor popup and apply what it returns to the kit it
    /// was opened from.
    fn draw_and_apply_function_popup(&mut self, ctx: &egui::Context) {
        if let Some(batch) =
            draw_function_popup(ctx, &mut self.editor.function_popup, &mut self.editor.color_popup)
        {
            let ops = DeferredOps {
                pending: batch.edits,
                function_data_ops: batch.data_ops,
                ..DeferredOps::default()
            };
            if let Some(kit) = self.popup_target_kit(self.editor.function_popup_kit) {
                self.apply_doc_ops(kit, &batch.tag_key, "Edit function", ops, UndoStep::Own);
            }
        }
        if self.editor.function_popup.is_none() {
            self.editor.function_popup_kit = None;
        }
    }

    /// Show popups a tag pane opened this frame, recording the kit they were
    /// opened from so confirming one later edits that kit's document rather
    /// than whichever kit is active, or last opened a popup, by then.
    pub(in crate::app) fn adopt_opened_popups(
        &mut self,
        kit: KitId,
        color: Option<MaterialColorPopup>,
        function: Option<FunctionPopup>,
    ) {
        if let Some(popup) = color {
            self.editor.color_popup = Some(popup);
            self.editor.color_popup_kit = Some(kit);
        }
        if let Some(popup) = function {
            self.editor.function_popup = Some(popup);
            self.editor.function_popup_kit = Some(kit);
        }
    }

    /// The kit a confirmed popup applies to: the one it was opened from, or
    /// none if that kit has closed since, so the edit is dropped rather than
    /// landing in another kit's tag that happens to share its key. A popup
    /// with no recorded kit applies to the active one.
    pub(in crate::app) fn popup_target_kit(&mut self, opened_from: Option<KitId>) -> Option<usize> {
        match opened_from {
            Some(kit) => {
                let index = self.resolve_kit(kit);
                if index.is_none() {
                    self.status =
                        "The editing kit this was opened from has closed; the edit was dropped."
                            .to_owned();
                }
                index
            }
            None => Some(self.active),
        }
    }

    /// Settle what this frame queued after every window has drawn: prompts,
    /// pending opens and field navigation, and the sound drains.
    fn process_frame_requests(&mut self, ctx: &egui::Context) {
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
        // Playback follows its tab: paused once another tab (or another kit)
        // has focus, disposed of once its tab is closed, whichever way that
        // happened. Checked against the open tabs rather than hooked into each
        // close path, so a close confirmed after the save prompt counts and a
        // cancelled one does not.
        let focus = self.kits.get(self.active).and_then(|kit| {
            kit.selected_key
                .clone()
                .map(|key| crate::app::audio::SoundOwner { kit: kit.id, key })
        });
        let kits = &self.kits;
        self.audio.follow_tabs(focus.as_ref(), |owner| {
            kits.iter()
                .find(|kit| kit.id == owner.kit)
                .is_some_and(|kit| kit.open_tabs.contains(&owner.key))
        });
        // And the players forget what they kept for a tab that is gone.
        crate::app::editor::forget_closed_players(ctx, |tag_key| {
            kits.iter()
                .any(|kit| kit.open_tabs.iter().any(|key| key == tag_key))
        });
        let sound_root = if !self.audio.pending.is_empty() {
            self.source_tags_root().map(std::path::Path::to_path_buf)
        } else {
            None
        };
        self.audio.process(sound_root.as_deref(), ctx);
        // A single interaction can queue a transport update before playback
        // (most notably H3's language fallback followed by Play). Draining only
        // one item left Play waiting for a repaint that might never arrive,
        // making the click silently do nothing. Preserve queue order, but
        // settle the whole interaction in this frame.
        while !self.audio.pending.is_empty() {
            self.audio.process(sound_root.as_deref(), ctx);
        }
        // Drain a queued sound extraction (decode + write files off the render
        // hot loop) and a reimport hand-off (opens the tool runner pre-filled).
        if let Some(request) = self.export.pending_sound_extract.take() {
            self.audio.run_extract(request, ctx);
            if let Some(status) = self.audio.status.clone() {
                self.status = status;
            }
        }
        self.process_pending_tool_import(ctx);
    }

    /// Clear the status line once its message has been up for a while.
    ///
    /// Runs after the worker drain so a message set this frame is timed from
    /// this frame. Progress states are rendered from their own fields rather
    /// than from `status`, so expiring it never blanks a running scan.
    pub(in crate::app) fn expire_status(&mut self, ctx: &egui::Context) {
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

    pub(in crate::app) fn run_deferred_file_action(&mut self, ctx: &egui::Context) {
        match self.editor.deferred_file_action.take() {
            Some(DeferredFileAction::SaveCurrentTag)
                if self.prefs.enable_chimp
                    && self.kits[self.active].surface == KitSurface::Chimp =>
            {
                self.open_chimp_save_dialog(self.active)
            }
            Some(DeferredFileAction::SaveCurrentTag) => self.save_current_tag(ctx),
            Some(DeferredFileAction::SaveProject) => {
                let (kit, now) = (self.active, ctx.input(|input| input.time));
                self.save_campaign_project_file(kit, now);
            }
            Some(DeferredFileAction::SaveProjectAs) => {
                let (kit, now) = (self.active, ctx.input(|input| input.time));
                self.save_campaign_project_file_as(kit, now);
            }
            Some(DeferredFileAction::ExportMod) => self.export_mod(),
            Some(DeferredFileAction::ExtractAllContainerTags) => {
                self.begin_extract_all_container_tags(ctx.clone())
            }
            Some(DeferredFileAction::PokeCurrentTag) => self.begin_poke_current_tag(ctx.clone()),
            Some(DeferredFileAction::Close(action)) => self.request_close_action(action, ctx),
            Some(DeferredFileAction::CloseCurrentTab)
                if self.prefs.enable_chimp
                    && self.kits[self.active].surface == KitSurface::Chimp =>
            {
                if let Some(package) = self.kits[self.active].chimp.selected_package.clone() {
                    let kit = self.active;
                    if !self.close_chimp_package(kit, &package) {
                        self.status =
                            "Save or discard modified Chimp packages before closing them."
                                .to_owned();
                    }
                }
            }
            Some(DeferredFileAction::CloseCurrentTab) => {
                if let Some(key) = self.kits[self.active].selected_key.clone() {
                    self.request_close_action(PendingCloseAction::CloseTab(key), ctx);
                }
            }
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
        self.editor.deferred_file_action = Some(action);
        // It runs at the start of the next frame, which a window with
        // nothing else to do would otherwise never draw.
        ctx.request_repaint();
    }

    fn prepare_root_frame(&mut self, ctx: &egui::Context) {
        ctx.set_zoom_factor(self.prefs.ui_scale);
        self.handle_pixels_per_point_change(ctx);
        self.maybe_refresh_entry_index(ctx.clone());
        set_dark_mode(self.prefs.dark_mode);
        // Pushed the same way and for the same reason as the theme: the two
        // halves of the angle conversion are free functions on opposite sides
        // of the frame, and neither can reach `Baboon`.
        crate::core::format::set_angles_in_degrees(self.prefs.angles_in_degrees);
        ctx.set_visuals(foundation_visuals());
        set_combo_scroll_cycle_enabled(ctx, self.prefs.scroll_to_cycle_dropdowns);
        apply_scroll_speed(ctx, self.prefs.scroll_speed);
        set_zoom_speed(ctx, self.prefs.zoom_speed);
        // Opened before any pane draws and settled after the last one, so a
        // dropdown can only claim a gesture on the frame it began.
        begin_wheel_gesture(ctx);
        // A folder move or rename is rewriting tags on disk. Nothing may edit,
        // save or open them until it lands, so no shortcut or dropped file is
        // taken, and no text field keeps the keyboard.
        if self.tag_ops.folder_refactor.is_some() {
            ctx.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            self.search.find.open = true;
            self.search.find.focus_query = true;
        }
        self.refresh_find(ctx);
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::S)) {
            self.defer_file_action(DeferredFileAction::SaveCurrentTag, ctx);
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::P)) {
            self.defer_file_action(DeferredFileAction::PokeCurrentTag, ctx);
        }
        // Deferred like the File menu's Close Current Tag: the close runs after
        // the editor renders, so an edit still focused in a field is committed
        // before the dirty check decides whether to prompt.
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::W)) {
            self.defer_file_action(DeferredFileAction::CloseCurrentTab, ctx);
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
                .map(|file| file.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if !dropped_paths.is_empty() {
            self.open_dropped_files(dropped_paths, ctx.clone());
        }
        // The other direction: a browser drag that ends on Sapien's window.
        self.track_kit_tool_drop(ctx);
    }

    fn draw_auxiliary_windows(&mut self, ctx: &egui::Context) {
        self.draw_tag_reference_picker_window(ctx);
        self.draw_settings_window(ctx);
        self.draw_tool_commands_window(ctx);
        self.draw_new_tag_window(ctx);
        self.draw_import_tag_window(ctx);
        self.draw_import_discard_confirm(ctx);
        self.draw_overwrite_confirm_window(ctx);
        self.draw_chimp_discard_window(ctx);
        self.draw_chimp_save_window(ctx);
        self.draw_clear_stash_confirm_window(ctx);
        self.draw_container_duplicate_confirm_window(ctx);
        self.draw_container_dump_confirm_window(ctx);
        self.draw_delete_confirm_window(ctx);
        self.draw_chimp_mesh_texture_prompt(ctx);
        self.draw_chimp_texture_export_prompt(ctx);
        self.draw_chimp_level_export_prompt(ctx);
        self.draw_operation_notice_window(ctx);
        self.draw_mod_export_window(ctx);
        self.draw_exported_mod_window(ctx);
        self.draw_poke_window(ctx);
        self.draw_tag_import_window(ctx);
        self.draw_cache_import_window(ctx);
        self.draw_about_window(ctx);
        self.draw_query_results_window(ctx);
        self.draw_tag_diff_window(ctx);
        self.draw_content_explorer_window(ctx);
        self.draw_keyword_chooser_window(ctx);
        self.draw_field_value_search_window(ctx);
        self.draw_find_window(ctx);
        self.draw_tsv_paste_window(ctx);
        self.draw_rename_tag_window(ctx);
        self.draw_container_folder_window(ctx);
        self.draw_loose_folder_rename_window(ctx);
        self.draw_extract_target_window(ctx);
        self.draw_folder_refactor_lock(ctx);
        end_wheel_gesture(ctx);
    }

    /// While a folder move or rename runs, cover the whole window with a layer
    /// that takes every click, drag and scroll, and show its progress on it.
    ///
    /// The job rewrites tags on disk from a snapshot taken when it started; an
    /// edit, save or second refactor in the meantime would be overwritten or
    /// would race it. Drawn last and in the foreground so no window or panel
    /// sits above it.
    fn draw_folder_refactor_lock(&mut self, ctx: &egui::Context) {
        let Some(progress) = &self.tag_ops.folder_refactor else {
            return;
        };
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("folder_refactor_lock"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_black_alpha(140));
                let panel = egui::Rect::from_center_size(rect.center(), egui::vec2(360.0, 96.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| {
                    Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(panel.width());
                        ui.label(RichText::new(&progress.label).strong().color(text_dark()));
                        ui.add_space(4.0);
                        let bar = match progress.progress {
                            Some(value) => egui::ProgressBar::new(value.clamp(0.0, 1.0)),
                            None => egui::ProgressBar::new(0.0).animate(true),
                        };
                        ui.add(bar.text(RichText::new(&progress.phase).color(text_dark())));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Baboon is locked until references are updated.")
                                .color(subtle_dark())
                                .small(),
                        );
                    });
                });
            });
        ctx.request_repaint_after(PROGRESS_REPAINT);
    }
}

pub(in crate::app) fn recent_folder_menu_label(path: &Path) -> String {
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

fn terminal_line_text(line: &TerminalLineEntry) -> RichText {
    let text = RichText::new(&line.text).color(terminal_line_color(line.severity));
    if terminal_line_is_strong(line.severity) {
        text.font(bold_font(13.0)).strong()
    } else {
        text.monospace().font(FontId::monospace(13.0))
    }
}

/// The terminal's output lines, scrolled.
///
/// Only the lines in view are laid out and drawn. The rest are placed by
/// their wrapped heights, which each line keeps for the width it was last
/// wrapped at: a width change re-measures once, an appended line measures
/// itself. Drawing every line as a label, up to the 20,000 kept, cost 4.5 ms
/// a frame in a release build.
pub(in crate::app) fn draw_terminal_output(
    ui: &mut Ui,
    lines: &[TerminalLineEntry],
    want_scroll_bottom: bool,
) {
    let gap = ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical()
        .id_salt("terminal_output")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            ui.visuals_mut().override_text_color = None;
            ui.set_min_width(ui.available_width());
            let width = ui.available_width();
            let height_of = |line: &TerminalLineEntry| match line.wrapped.get() {
                Some((at, height)) if at == width => height,
                _ => {
                    let height = egui::WidgetText::from(terminal_line_text(line))
                        .into_galley(ui, Some(egui::TextWrapMode::Wrap), width, TextStyle::Body)
                        .size()
                        .y;
                    line.wrapped.set(Some((width, height)));
                    height
                }
            };
            // Where each line starts, from the top of the content.
            let mut top = 0.0;
            let mut first = None;
            let mut first_top = 0.0;
            let mut last = 0;
            for (index, line) in lines.iter().enumerate() {
                let bottom = top + height_of(line);
                if first.is_none() && bottom >= viewport.min.y {
                    first = Some(index);
                    first_top = top;
                }
                if top <= viewport.max.y {
                    last = index + 1;
                }
                top = bottom + gap;
            }
            let total = (top - gap).max(0.0);
            ui.set_height(total);
            let origin = ui.max_rect().top();
            if let Some(first) = first {
                let rect = egui::Rect::from_x_y_ranges(
                    ui.max_rect().x_range(),
                    origin + first_top..=origin + total,
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.skip_ahead_auto_ids(first);
                    for line in &lines[first..last.max(first)] {
                        #[cfg(test)]
                        terminal_output_tests::LINES_BUILT.with(|built| built.set(built.get() + 1));
                        ui.add(egui::Label::new(terminal_line_text(line)).wrap());
                    }
                });
            }
            if want_scroll_bottom {
                let bottom = egui::Rect::from_x_y_ranges(
                    ui.max_rect().x_range(),
                    origin + total..=origin + total,
                );
                ui.scroll_to_rect(bottom, Some(egui::Align::BOTTOM));
            }
        });
}

#[cfg(test)]
pub(in crate::app) mod terminal_output_tests;

#[cfg(test)]
mod folder_refactor_lock_tests;

#[cfg(test)]
mod popup_kit_stamp_tests;
