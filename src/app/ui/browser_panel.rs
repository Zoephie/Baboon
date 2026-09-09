//! The tag browser for one kit: source header, search, and the folder/group tree.
//! It owns browser presentation and request collection for a single kit; layout of the panels belongs to the shell.

use super::*;

const FOLDER_HEADER_ACTIONS_SINGLE_ROW_BREAKPOINT: f32 = 900.0;
const FOLDER_HEADER_LAUNCHER_WIDTH: f32 = 190.0;
const FOLDER_BROWSER_SEARCH_STACK_BREAKPOINT: f32 = 600.0;

fn folder_browser_search_stacks(available_width: f32) -> bool {
    available_width < FOLDER_BROWSER_SEARCH_STACK_BREAKPOINT
}

fn folder_browser_search_hint(folder_name: &str) -> String {
    format!("search {folder_name} folder")
}

impl Baboon {
    /// Draw a folder as a first-class docked pane beside ordinary tag panes.
    pub(in crate::app) fn draw_folder_browser_pane(
        &mut self,
        ui: &mut Ui,
        _ctx: &egui::Context,
        kit_index: usize,
        pane_key: &str,
    ) -> Option<BrowserAction> {
        let Some(mut pane) = self.kits[kit_index].folder_browsers.remove(pane_key) else {
            ui.label(RichText::new("This folder is no longer open").color(subtle_dark()));
            return None;
        };

        self.refresh_modified_tags(kit_index);
        self.refresh_deletable_keys(kit_index);
        let source_len = self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.full_entry_set().len())
            .unwrap_or(0);
        let generation = self.kits[kit_index].generation;
        if pane.cached_generation != generation || pane.cached_source_len != source_len {
            if let Some(source) = self.kits[kit_index].source.as_ref() {
                let entries = source.full_entry_set();
                pane.tree = crate::source::build_tree_beneath(entries, &pane.rel_path);
                pane.group_tree = crate::source::build_group_tree_beneath(entries, &pane.rel_path);
            } else {
                pane.tree = TagTree::default();
                pane.group_tree = TagTree::default();
            }
            pane.filter_cache = FilterCache::default();
            pane.cached_generation = generation;
            pane.cached_source_len = source_len;
        }

        let is_loose = self.kits[kit_index]
            .source
            .as_ref()
            .is_some_and(|source| matches!(source.source, TagSource::LooseFolder { .. }));
        let is_container = self.kits[kit_index]
            .source
            .as_ref()
            .is_some_and(|source| matches!(source.source, TagSource::IoStoreContainerSet { .. }));
        let favorite_keys: HashSet<String> = self.kits[kit_index]
            .active_favorite_entries
            .iter()
            .map(|entry| entry.key.clone())
            .collect();
        let pane_favorite_folders =
            std::sync::Arc::new(self.kits[kit_index].active_favorite_folders.clone());
        let selected = self.kits[kit_index].selected_key.clone();
        let modified_tags = std::sync::Arc::clone(&self.kits[kit_index].modified_tags);
        let deletable_keys = std::sync::Arc::clone(&self.kits[kit_index].deletable_keys);
        let game = self.kits[kit_index]
            .source
            .as_ref()
            .and_then(|source| source.game.clone());
        let scenario_launch = self.kits[kit_index]
            .source
            .as_ref()
            .map(crate::app::controller::scenario_launch_availability)
            .unwrap_or_default();
        let entries = self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.full_entry_set())
            .unwrap_or_default();
        let mut show_browser_prefixes = self.show_browser_prefixes;
        let mut folders_before_tags = self.folders_before_tags;
        let double_click_to_open = self.double_click_to_open_tags;
        let search_hint = folder_browser_search_hint(&pane.label);
        let mut action = None;

        Frame::none()
            .inner_margin(egui::Margin {
                left: 10.0,
                right: 10.0,
                top: 8.0,
                bottom: 8.0,
            })
            .show(ui, |ui| {
                set_browser_modified_tags(ui, modified_tags);
                set_browser_favorite_folders(ui, is_loose.then_some(pane_favorite_folders));
                set_browser_deletable_keys(ui, deletable_keys);
                set_browser_game(ui, game);
                set_browser_scenario_launch(ui, scenario_launch);
                set_browser_is_folder_pane(ui, true);

                draw_folder_pane_header(ui, &mut pane, is_loose, &mut action);
                ui.add_space(14.0);
                ui.separator();
                ui.add_space(10.0);

                let search_response = if folder_browser_search_stacks(ui.available_width()) {
                    let search_response = browser_search_field(ui, &mut pane.filter, &search_hint);
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        draw_folder_browser_controls(
                            ui,
                            &mut pane,
                            &mut show_browser_prefixes,
                            &mut folders_before_tags,
                        );
                    });
                    search_response
                } else {
                    ui.horizontal(|ui| {
                        draw_folder_browser_controls(
                            ui,
                            &mut pane,
                            &mut show_browser_prefixes,
                            &mut folders_before_tags,
                        );
                        ui.add_space(6.0);
                        browser_search_field(ui, &mut pane.filter, &search_hint)
                    })
                    .inner
                };
                if pane.focus_search {
                    search_response.request_focus();
                    pane.focus_search = false;
                }
                if let Some(warning) = browser::browser_filter_warning(&pane.filter) {
                    ui.label(
                        RichText::new(warning)
                            .small()
                            .color(Color32::from_rgb(184, 134, 11)),
                    );
                }
                ui.add_space(8.0);

                let filter = pane.filter.trim();
                let groups_mode = pane.mode == BrowserMode::Groups;
                let (tree, entries) = if filter.is_empty() {
                    (
                        if groups_mode {
                            &pane.group_tree
                        } else {
                            &pane.tree
                        },
                        entries,
                    )
                } else {
                    pane.filter_cache.refresh_beneath(
                        pane.cached_generation,
                        filter,
                        entries,
                        groups_mode,
                        &pane.rel_path,
                    );
                    (
                        &pane.filter_cache.tree,
                        pane.filter_cache.entries.as_slice(),
                    )
                };
                ScrollArea::vertical()
                    .id_salt(("folder_pane", pane_key))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if entries.is_empty() {
                            ui.label(RichText::new("No matching tags").color(subtle_dark()));
                            return;
                        }
                        let tree_action = draw_tree(
                            ui,
                            tree,
                            entries,
                            selected.as_deref(),
                            filter,
                            show_browser_prefixes,
                            double_click_to_open,
                            groups_mode,
                            None,
                            pane.sort,
                            !groups_mode && folders_before_tags,
                            is_loose.then_some(&favorite_keys),
                            is_container,
                        );
                        if action.is_none() {
                            action = tree_action;
                        }
                    });
            });

        self.show_browser_prefixes = show_browser_prefixes;
        self.folders_before_tags = folders_before_tags;

        action = match action {
            Some(BrowserAction::OpenFolderBrowser {
                rel_path,
                label,
                open_in_new_tab: false,
            }) => {
                navigate_folder_browser(&mut pane, rel_path, label);
                ui.ctx().request_repaint();
                None
            }
            other => other,
        };
        self.kits[kit_index]
            .folder_browsers
            .insert(pane_key.to_owned(), pane);
        action
    }

    /// Draw the tag browser for `kit_index`.
    ///
    /// Takes the kit explicitly rather than reading the active one, so a split
    /// view can render a different kit's browser in each pane.
    ///
    /// Every widget id beneath is salted with the kit's id. Without that, two
    /// browsers in the same frame would share state for anything egui keys by
    /// label — folder collapse state in particular is keyed on the folder name
    /// alone, so expanding `objects` in one kit would expand it in the other.
    pub(in crate::app) fn draw_kit_browser(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
    ) {
        let salt = self.kits[kit_index].id.0;
        ui.push_id(salt, |ui| self.draw_kit_browser_inner(ui, ctx, kit_index));
    }

    fn draw_kit_browser_inner(&mut self, ui: &mut Ui, ctx: &egui::Context, kit_index: usize) {
        // This kit's own source, not `source()` — that reads the *active* kit,
        // so in a split every browser drew the focused kit's banner and the
        // header flickered between games as the cursor moved between panes.
        let sidebar_header = self.kits[kit_index].source.as_ref().map(|source| {
            (
                source.game.clone(),
                source.source.origin_label(),
                sidebar_source_path_label(&source.source),
                self.kits[kit_index]
                    .profile
                    .as_ref()
                    .map(|profile| profile.id.clone()),
            )
        });
        if let Some((Some(game), _origin, path_label, profile_id)) = sidebar_header.as_ref() {
            draw_game_banner_header(ui, self, game, path_label, profile_id.as_deref());
        } else {
            ui.heading(RichText::new("Tags").color(text_dark()));
            if let Some((_, origin, _, _)) = sidebar_header.as_ref() {
                ui.small(RichText::new(origin).color(subtle_dark()));
                ui.add_space(8.0);
            }
        }

        let active_favorite_entries = self.kits[kit_index].active_favorite_entries.clone();
        let active_favorite_folders =
            std::sync::Arc::new(self.kits[kit_index].active_favorite_folders.clone());
        let favorite_keys: HashSet<String> = active_favorite_entries
            .iter()
            .map(|entry| entry.key.clone())
            .collect();
        // Indexed directly rather than through `source_mut()`: the block
        // below also borrows `self.kits[kit_index].filter`, `self.kits[kit_index].filter_cache`, and
        // `self.status`, and a method call would borrow all of `self`.
        let kit_id = self.kits[kit_index].id;
        // Refreshed before the tree is drawn, and published into egui memory so
        // the row and folder painters can reach it without threading it through
        // every drawing function. The browsers draw one after another, so what
        // is in memory during this tree's draw is this kit's own set.
        self.refresh_modified_tags(kit_index);
        set_browser_modified_tags(
            ui,
            std::sync::Arc::clone(&self.kits[kit_index].modified_tags),
        );
        set_browser_favorite_folders(
            ui,
            self.kits[kit_index]
                .source
                .as_ref()
                .is_some_and(|source| matches!(source.source, TagSource::LooseFolder { .. }))
                .then_some(active_favorite_folders),
        );
        set_browser_game(
            ui,
            self.kits[kit_index]
                .source
                .as_ref()
                .and_then(|source| source.game.clone()),
        );
        set_browser_scenario_launch(
            ui,
            self.kits[kit_index]
                .source
                .as_ref()
                .map(crate::app::controller::scenario_launch_availability)
                .unwrap_or_default(),
        );
        set_browser_is_folder_pane(ui, false);
        self.refresh_deletable_keys(kit_index);
        set_browser_deletable_keys(
            ui,
            std::sync::Arc::clone(&self.kits[kit_index].deletable_keys),
        );
        let kit = &mut self.kits[kit_index];
        if let Some(source) = kit.source.as_mut() {
            ui.add_space(8.0);
            let scanning = kit.scanning_entries;
            // Collect deferred scan-trigger here; execute after borrow ends.
            let mut need_scan = false;
            let prev_filter_empty = kit.filter.is_empty();
            browser_search_field(ui, &mut kit.filter, "search tags");
            if let Some(warning) = browser::browser_filter_warning(&kit.filter) {
                ui.label(
                    RichText::new(warning)
                        .small()
                        .color(Color32::from_rgb(184, 134, 11)),
                );
            }
            ui.add_space(6.0);
            // Wrapped, and with the toolbar visuals hoisted out of the two
            // scopes that used to group these buttons. A scope is placed as one
            // unit, so grouped buttons could only wrap in blocks — and the
            // widest block became the sidebar's minimum width. Individually
            // wrapping buttons let the panel shrink to a single button.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let groups_clicked = browser_toolbar_controls(
                    ui,
                    &mut kit.browser_mode,
                    &mut kit.browser_sort,
                    &mut self.show_browser_prefixes,
                    &mut self.folders_before_tags,
                );
                if groups_clicked
                    && matches!(source.source, TagSource::LooseFolder { .. })
                    && source.all_entries.is_empty()
                    && !scanning
                {
                    need_scan = true;
                }
            });
            if prev_filter_empty
                && !kit.filter.is_empty()
                && matches!(source.source, TagSource::LooseFolder { .. })
                && source.all_entries.is_empty()
                && !scanning
            {
                need_scan = true;
            }
            ui.add_space(4.0);
            let selected = kit.selected_key.clone();
            let filter = kit.filter.trim().to_owned();
            let mode = kit.browser_mode;
            let show_prefixes = self.show_browser_prefixes;
            let folders_before_tags = self.folders_before_tags;
            let double_click_to_open = self.double_click_to_open_tags;
            let mut status_update = None;
            // Groups and filtered Folders use all_entries (background
            // scan) so every tag is visible, not just visited folders.
            let has_all = !source.all_entries.is_empty();
            let groups_mode = matches!(mode, BrowserMode::Groups);
            // Hoisted so the filtered and unfiltered trees cannot disagree about
            // it: passing `false` here once cost the CE folder menu the moment a
            // search filter was typed. `draw_tree_node` gates on `!groups_mode`
            // itself, so this stays correct in Groups mode.
            let is_container_source =
                matches!(source.source, TagSource::IoStoreContainerSet { .. });
            let favorite_context =
                matches!(source.source, TagSource::LooseFolder { .. }).then_some(&favorite_keys);
            // One-shot "reveal in tree" request (force-open ancestors +
            // scroll). Borrowed into the Copy `Reveal` for the draw.
            //
            // Only this kit's browser may consume it: with two browsers on
            // screen, whichever drew first would otherwise swallow a reveal
            // meant for the other and scroll to a tag it does not have.
            let reveal_owned = match &self.reveal_target {
                Some(request) if request.kit == kit_id => self.reveal_target.take(),
                _ => None,
            };
            let reveal = reveal_owned.as_ref().map(|request| Reveal {
                key: request.key.as_str(),
                remaining: request.ancestors.as_slice(),
            });
            let sort = kit.browser_sort;
            let action = ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let (favorite_action, favorites_visible) = draw_favorites(
                        ui,
                        &active_favorite_entries,
                        selected.as_deref(),
                        &filter,
                        show_prefixes,
                        double_click_to_open,
                        &favorite_keys,
                    );
                    browser_favorites_divider(ui, favorites_visible);

                    let tree_action = if !filter.is_empty() {
                        // Active search renders a memoized, pruned tree. A loose
                        // source waits for its complete background index first.
                        let entries: &[TagEntry] = if has_all {
                            &source.all_entries
                        } else {
                            &source.entries
                        };
                        if scanning && !has_all {
                            ui.label(RichText::new("Indexing tags…").color(subtle_dark()).small());
                            None
                        } else {
                            kit.filter_cache.refresh(
                                kit.generation,
                                &filter,
                                entries,
                                has_all,
                                groups_mode,
                            );
                            let cache = &kit.filter_cache;
                            if cache.entries.is_empty() {
                                ui.label(RichText::new("No matching tags").color(subtle_dark()));
                                None
                            } else {
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
                                    reveal,
                                    sort,
                                    !groups_mode && folders_before_tags,
                                    favorite_context,
                                    is_container_source,
                                )
                            }
                        }
                    } else {
                        match mode {
                            BrowserMode::Folders => {
                                if let TagSource::LooseFolder { root, .. } = &source.source {
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
                                        is_container_source,
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
                                        false,
                                    )
                                }
                            }
                        }
                    };
                    favorite_action.or(tree_action)
                })
                .inner;
            if let Some(status) = status_update {
                self.status = status;
            }
            // Browser actions and the scan below all resolve against the active
            // kit, and this browser is drawn inside the workspace-tree walk,
            // where `active` is still whatever it was when the frame began.
            // Press-activation usually beats the click by a frame, but a press
            // and its release land in the same frame whenever one runs long --
            // which is exactly what loading a large tag or indexing does. This
            // browser's own kit is the right target either way.
            if action.is_some() || need_scan {
                self.active = kit_index;
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
    }
}

/// Draw the controls shared by the sidebar and docked folder browser. Callers
/// choose the surrounding layout and may place search beside or above them.
fn browser_toolbar_controls(
    ui: &mut Ui,
    mode: &mut BrowserMode,
    sort: &mut BrowserSort,
    show_prefixes: &mut bool,
    folders_before_tags: &mut bool,
) -> bool {
    ui.visuals_mut().widgets.inactive.bg_fill = browser_toolbar_bg();
    ui.visuals_mut().widgets.hovered.bg_fill = browser_toolbar_active();
    ui.visuals_mut().widgets.active.bg_fill = browser_toolbar_active();

    if selectable_icon_text_button(
        ui,
        ButtonIcon::FolderOpen,
        "Folders",
        *mode == BrowserMode::Folders,
    )
    .clicked()
    {
        *mode = BrowserMode::Folders;
    }
    let groups_clicked = selectable_icon_text_button(
        ui,
        ButtonIcon::Group,
        "Groups",
        *mode == BrowserMode::Groups,
    )
    .clicked();
    if groups_clicked {
        *mode = BrowserMode::Groups;
    }
    icon_menu_button(ui, ButtonIcon::Sort, "Sort", |ui| {
        for option in BrowserSort::ALL {
            if ui
                .selectable_label(*sort == option, option.label())
                .clicked()
            {
                *sort = option;
                ui.close_menu();
            }
        }
    });
    icon_menu_button(ui, ButtonIcon::Other, "Other browser options", |ui| {
        ui.checkbox(show_prefixes, "Show prefixes");
        ui.checkbox(folders_before_tags, "Folders before tags");
    });
    groups_clicked
}

/// Folder-page wrapper for the shared controls. Keeping all four controls in
/// this single helper means the wide row and wrapped narrow row cannot drift
/// into different button sets or spacing.
fn draw_folder_browser_controls(
    ui: &mut Ui,
    pane: &mut FolderBrowserState,
    show_prefixes: &mut bool,
    folders_before_tags: &mut bool,
) {
    ui.spacing_mut().item_spacing.x = PANE_HEADER_ACTION_GAP;
    browser_toolbar_controls(
        ui,
        &mut pane.mode,
        &mut pane.sort,
        show_prefixes,
        folders_before_tags,
    );
}

fn draw_folder_pane_header(
    ui: &mut Ui,
    pane: &mut FolderBrowserState,
    is_loose: bool,
    action: &mut Option<BrowserAction>,
) {
    let normalized = pane.rel_path.to_string_lossy().replace('\\', "/");
    let (breadcrumbs, title) = pane_header_path_parts(&normalized);
    let is_favorite = browser_favorite_folders(ui).is_some_and(|folders| {
        folders.iter().any(|path| {
            path.to_string_lossy()
                .replace('\\', "/")
                .eq_ignore_ascii_case(&normalized)
        })
    });
    let available = ui.available_width();
    let actions_single_row = available >= FOLDER_HEADER_ACTIONS_SINGLE_ROW_BREAKPOINT;
    let actions_stacked = !actions_single_row;
    let action_width = if actions_stacked {
        PANE_HEADER_COMMON_ACTIONS_WIDTH.max(FOLDER_HEADER_LAUNCHER_WIDTH)
    } else {
        PANE_HEADER_COMMON_ACTIONS_WIDTH + PANE_HEADER_SECTION_GAP + FOLDER_HEADER_LAUNCHER_WIDTH
    };
    let inline_left_width = pane_header_inline_left_width(available, action_width);
    let actions_inline = inline_left_width.is_some();
    let left_width = inline_left_width.unwrap_or(available);

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = PANE_HEADER_SECTION_GAP;
        ui.allocate_ui_with_layout(
            Vec2::new(left_width, PANE_HEADER_ICON_SIZE),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = PANE_HEADER_ICON_TEXT_GAP;
                    let (icon_rect, _) =
                        ui.allocate_exact_size(Vec2::splat(PANE_HEADER_ICON_SIZE), Sense::hover());
                    paint_button_icon_at(ui, ButtonIcon::FolderOpen, icon_rect, text_dark());
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        if let Some((path, label)) = pane_header_breadcrumbs(ui, &breadcrumbs) {
                            navigate_folder_browser(pane, path, label);
                            ui.ctx().request_repaint();
                        }
                        ui.label(RichText::new(title).size(15.0).strong().color(text_dark()));
                    });
                });
            },
        );

        if actions_inline {
            let action_height = if actions_stacked {
                BUTTON_HEIGHT * 2.0 + 8.0
            } else {
                BUTTON_HEIGHT
            };
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), action_height),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.set_height(action_height);
                    if actions_stacked {
                        ui.vertical(|ui| {
                            ui.set_height(action_height);
                            ui.spacing_mut().item_spacing.y = 8.0;
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| draw_folder_header_launcher(ui, pane, is_loose, action),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    draw_folder_header_common_actions(
                                        ui,
                                        pane,
                                        is_loose,
                                        is_favorite,
                                        action,
                                    );
                                },
                            );
                        });
                    } else {
                        draw_folder_header_common_actions(ui, pane, is_loose, is_favorite, action);
                        ui.add_space(PANE_HEADER_SECTION_GAP);
                        draw_folder_header_launcher(ui, pane, is_loose, action);
                    }
                },
            );
        }
    });

    if !actions_inline {
        ui.add_space(8.0);
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), BUTTON_HEIGHT),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| draw_folder_header_launcher(ui, pane, is_loose, action),
        );
        ui.add_space(8.0);
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), BUTTON_HEIGHT),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                draw_folder_header_common_actions(ui, pane, is_loose, is_favorite, action);
            },
        );
    }
}

fn draw_folder_header_common_actions(
    ui: &mut Ui,
    pane: &mut FolderBrowserState,
    is_loose: bool,
    is_favorite: bool,
    action: &mut Option<BrowserAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = PANE_HEADER_ACTION_GAP;
        icon_menu_button(ui, ButtonIcon::Other, "Other folder actions", |ui| {
            if is_loose {
                if let Some(menu_action) =
                    loose_folder_transfer_menu_items(ui, &pane.rel_path, &pane.label)
                {
                    action.replace(menu_action);
                }
                context_menu_separator(ui);
            }
            if ui.button("Dump folder to JSON...").clicked() {
                action.replace(BrowserAction::DumpLooseFolderJson {
                    rel_path: pane.rel_path.clone(),
                    label: pane.label.clone(),
                });
                ui.close_menu();
            }
        });
        let favorite_icon = if is_favorite {
            ButtonIcon::FavouriteFilled
        } else {
            ButtonIcon::Favourite
        };
        if icon_text_button(
            ui,
            favorite_icon,
            if is_favorite { "Favorited" } else { "Favorite" },
            is_loose,
        )
        .on_disabled_hover_text("Only loose editing-kit folders can be favorited")
        .clicked()
        {
            action.replace(BrowserAction::ToggleFolderFavorite(pane.rel_path.clone()));
        }
        if icon_text_button(ui, ButtonIcon::Find, "Find", true).clicked() {
            pane.focus_search = true;
        }
    });
}

fn draw_folder_header_launcher(
    ui: &mut Ui,
    pane: &FolderBrowserState,
    is_loose: bool,
    action: &mut Option<BrowserAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = PANE_HEADER_ACTION_GAP;
        if icon_text_button(ui, ButtonIcon::FileExplorer, "File Explorer", is_loose)
            .on_disabled_hover_text("Only loose editing-kit folders exist in File Explorer")
            .clicked()
        {
            action.replace(BrowserAction::OpenLooseFolderInExplorer {
                rel_path: pane.rel_path.clone(),
            });
        }
        ui.label(RichText::new("Open folder in:").color(subtle_dark()));
    });
}

#[cfg(test)]
mod responsive_folder_toolbar_tests {
    use super::*;

    #[test]
    fn search_stacks_before_the_toolbar_reaches_600_points() {
        assert!(!folder_browser_search_stacks(600.0));
        assert!(folder_browser_search_stacks(599.0));
    }

    #[test]
    fn search_hint_names_the_current_folder() {
        assert_eq!(folder_browser_search_hint("brute"), "search brute folder");
    }

    #[test]
    fn sidebar_paths_can_wrap_after_each_separator() {
        assert_eq!(
            sidebar_wrappable_path_label(r"C:\tags\objects"),
            "C:\\\u{200b}tags\\\u{200b}objects"
        );
    }
}
