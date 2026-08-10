//! The tag browser for one kit: source header, search, and the folder/group tree.
//! It owns browser presentation and request collection for a single kit; layout of the panels belongs to the shell.

use super::*;

impl Baboon {
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
        set_browser_modified_tags(ui, std::sync::Arc::clone(&self.kits[kit_index].modified_tags),);
        set_browser_game(
            ui,
            self.kits[kit_index]
                .source
                .as_ref()
                .and_then(|source| source.game.clone()),
        );
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
            ui.scope(|ui| {
                ui.visuals_mut().override_text_color = Some(text_dark());
                ui.visuals_mut().extreme_bg_color = browser_search_bg();
                ui.visuals_mut().widgets.inactive.bg_fill = browser_search_bg();
                ui.visuals_mut().widgets.hovered.bg_fill = browser_search_hover();
                ui.visuals_mut().widgets.active.bg_fill = browser_search_hover();
                ui.horizontal(|ui| {
                    let (icon_rect, _) =
                        ui.allocate_exact_size(Vec2::new(18.0, 22.0), Sense::hover());
                    let icon_rect =
                        egui::Rect::from_center_size(icon_rect.center(), Vec2::splat(16.0));
                    paint_button_icon_at(ui, ButtonIcon::SearchBar, icon_rect, text_dark());
                    ui.add(
                        egui::TextEdit::singleline(&mut kit.filter)
                            .hint_text("search tags")
                            .desired_width(f32::INFINITY),
                    );
                });
            });
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
                ui.visuals_mut().widgets.inactive.bg_fill = browser_toolbar_bg();
                ui.visuals_mut().widgets.hovered.bg_fill = browser_toolbar_active();
                ui.visuals_mut().widgets.active.bg_fill = browser_toolbar_active();
                let groups_btn = {
                    let folders = ui.add(
                        egui::Button::image_and_text(
                            button_icon_image(ui, ButtonIcon::FolderOpen, text_dark(), 16.0),
                            "Folders",
                        )
                        .selected(kit.browser_mode == BrowserMode::Folders),
                    );
                    if folders.clicked() {
                        kit.browser_mode = BrowserMode::Folders;
                    }
                    let groups = ui.add(
                        egui::Button::image_and_text(
                            button_icon_image(ui, ButtonIcon::Group, text_dark(), 16.0),
                            "Groups",
                        )
                        .selected(kit.browser_mode == BrowserMode::Groups),
                    );
                    if groups.clicked() {
                        kit.browser_mode = BrowserMode::Groups;
                    }
                    groups
                };
                if groups_btn.clicked()
                    && matches!(source.source, TagSource::LooseFolder { .. })
                    && source.all_entries.is_empty()
                    && !scanning
                {
                    need_scan = true;
                }
                ui.menu_image_button(
                    button_icon_image(ui, ButtonIcon::Sort, text_dark(), 16.0),
                    |ui| {
                        for option in BrowserSort::ALL {
                            if ui
                                .selectable_label(kit.browser_sort == option, option.label())
                                .clicked()
                            {
                                kit.browser_sort = option;
                                ui.close_menu();
                            }
                        }
                    },
                )
                .response
                .on_hover_text("Sort");
                ui.menu_image_button(
                    button_icon_image(ui, ButtonIcon::Filter, text_dark(), 16.0),
                    |ui| {
                        ui.checkbox(&mut self.show_browser_prefixes, "Show prefixes");
                    },
                )
                .response
                .on_hover_text("Filter");
                ui.menu_image_button(
                    button_icon_image(ui, ButtonIcon::Other, text_dark(), 16.0),
                    |ui| {
                        ui.checkbox(&mut self.folders_before_tags, "Folders before tags");
                    },
                )
                .response
                .on_hover_text("Other browser options");
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
            let is_container_source = matches!(source.source, TagSource::IoStoreContainerSet { .. });
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
                            ui.label(RichText::new("Indexing tags…").color(subtle_dark()).small());
                            favorite_action
                        })
                        .inner
                } else {
                    kit.filter_cache.refresh(
                        kit.generation,
                        &filter,
                        entries,
                        has_all,
                        groups_mode,
                    );
                    let cache = &kit.filter_cache;
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
                                ui.label(RichText::new("No matching tags").color(subtle_dark()));
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
                                is_container_source,
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
                        };
                        favorite_action.or(tree_action)
                    })
                    .inner
            };
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
