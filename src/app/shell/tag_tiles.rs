//! Tiled layout of a kit's open tags: tab groups, splits, and drag-to-rearrange.
//! It owns the editor area's layout; one pane's contents belong to `tag_pane`.

use super::*;
use crate::app::shell::frame::centered_empty_state;
use crate::app::shell::frame::tint_toward;
use crate::app::shell::frame::wheel_scroll_tab_bar;

/// Draws a kit's tag panes for `egui_tiles` while the kit's tree is moved out
/// of its view.
///
/// Closes are collected rather than applied inline: removing a tile while
/// `egui_tiles` is walking the tree would invalidate its iteration, and a close
/// has to go through the unsaved-changes prompt anyway. They, and the tab
/// menus' other choices, are sent as commands once the walk is over.
struct TagPaneBehavior<'a, 'c, 'p> {
    cx: &'a Ctx<'c>,
    parts: &'a mut TileParts<'p>,
    inputs: &'a TileInputs,
    kit_index: usize,
    kit: KitId,
    /// Tab label and group tag for each open key, resolved in one pass before
    /// the tree is walked. `egui_tiles` asks for both once per tab per frame,
    /// and each answer used to be a linear scan of the source's entry lists --
    /// two scans per tab, ~0.4 ms a frame across six tabs of a 12,291-tag
    /// Campaign Evolved source, and worse the more tabs are open.
    tab_labels: HashMap<String, (String, Option<u32>)>,
    close_requests: Vec<String>,
    /// Deferred tab context-menu choices, applied after the tree is drawn for
    /// the same reason closes are: they mutate the layout or the open set.
    reveal: Option<String>,
    /// Show the tab's tag in File Explorer. The browser's tag menu already
    /// offers this; a tab is the other place a tag is "the one you have", and
    /// reaching it through Reveal in browser first is a detour.
    reveal_in_explorer: Option<String>,
    discard: Option<String>,
    /// Tag to expand or collapse throughout, and which of the two.
    expand: Option<(String, bool)>,
    close_all: bool,
    close_all_but: Option<String>,
}

impl egui_tiles::Behavior<String> for TagPaneBehavior<'_, '_, '_> {
    fn pane_ui(
        &mut self,
        ui: &mut Ui,
        tile_id: egui_tiles::TileId,
        pane: &mut String,
    ) -> egui_tiles::UiResponse {
        let key = pane.clone();
        // As with kits: pressing inside the pane selects it, hovering does
        // not. `selected_key` is what "Save Current Tag" acts on, so passing
        // the cursor over another pane must not change the target. Sent before
        // the pane draws, so it lands before anything the pane sends.
        if ui.input(|input| input.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect()) {
            self.cx.send(EditorCommand::FocusTab {
                kit: self.kit,
                key: key.clone(),
            });
        }
        let cx = self.cx;
        let kit_index = self.kit_index;
        let parts = &mut *self.parts;
        let view = &mut parts.views[self.kit];
        // The panes that are not tags. Handled before the entry lookup,
        // because nothing in the source answers to their keys by design.
        if is_folder_pane_key(&key) {
            let language = parts.audio.language.as_deref();
            draw_folder_browser_pane(cx, ui, kit_index, &key, view, language);
            return egui_tiles::UiResponse::None;
        }
        if key == BITMAP_LIBRARY_KEY {
            draw_thumbnail_library::<Bitmaps>(cx, ui, kit_index, &mut view.bitmap_browser);
            return egui_tiles::UiResponse::None;
        }
        if key == MODEL_LIBRARY_KEY {
            draw_thumbnail_library::<Models>(cx, ui, kit_index, &mut view.model_browser);
            return egui_tiles::UiResponse::None;
        }
        if key == GIT_REVIEW_KEY {
            draw_git_review(cx, ui, self.kit, &mut view.git_review);
            return egui_tiles::UiResponse::None;
        }
        if key == BLAM_KEY {
            draw_blam_pane(cx, ui, kit_index, &mut view.blam);
            return egui_tiles::UiResponse::None;
        }
        let Some(entry) = tag_pane_entry(&cx.model.kits[kit_index], &key) else {
            ui.label(RichText::new("This tag is no longer in the source").color(subtle_dark()));
            return egui_tiles::UiResponse::None;
        };

        // The scope salts every widget id under the pane, so the same tag shown
        // in two panes keeps independent scroll, focus, and collapse state
        // while both edit the one shared document.
        let scope = format!("tile{}", tile_id.0);
        let caches = self.inputs.tags.get(&(self.kit, key));
        Frame::NONE
            .inner_margin(egui::Margin {
                left: 10,
                right: 10,
                top: 8,
                bottom: 8,
            })
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("tag_tile", tile_id.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let inputs = PaneInputs {
                            find: &parts.search.find,
                            field_nav: parts.references.field_nav.as_ref(),
                            audio: parts.audio,
                            def_docs: caches.and_then(|caches| caches.def_docs.clone()),
                            ce_sound: caches.and_then(|caches| caches.ce_sound.clone()),
                        };
                        draw_tag_pane(
                            cx,
                            ui,
                            kit_index,
                            &entry,
                            &scope,
                            true,
                            inputs,
                            &mut parts.views[self.kit],
                            parts.editor,
                        );
                    });
            });
        egui_tiles::UiResponse::None
    }

    fn top_bar_right_ui(
        &mut self,
        _tiles: &egui_tiles::Tiles<String>,
        ui: &mut Ui,
        _tile_id: egui_tiles::TileId,
        _tabs: &egui_tiles::Tabs,
        scroll_offset: &mut f32,
    ) {
        wheel_scroll_tab_bar(ui, scroll_offset);
    }

    fn tab_title_for_pane(&mut self, pane: &String) -> egui::WidgetText {
        if pane == BITMAP_LIBRARY_KEY {
            return RichText::new(BITMAP_LIBRARY_TITLE)
                .color(text_dark())
                .into();
        }
        if pane == MODEL_LIBRARY_KEY {
            return RichText::new(MODEL_LIBRARY_TITLE).color(text_dark()).into();
        }
        if pane == GIT_REVIEW_KEY {
            return RichText::new(GIT_REVIEW_TITLE).color(text_dark()).into();
        }
        if pane == BLAM_KEY {
            return RichText::new(BLAM_TITLE).color(text_dark()).into();
        }
        if is_folder_pane_key(pane) {
            let label = self.parts.views[self.kit]
                .browser
                .folder_browsers
                .get(pane)
                .map(|folder| folder.label.clone())
                .unwrap_or_else(|| "Folder".to_owned());
            return RichText::new(label).color(text_dark()).into();
        }
        let dirty = self.cx.model.kits[self.kit_index]
            .parsed_tags
            .get(pane)
            .is_some_and(|document| document.dirty.is_set());
        let label = self
            .tab_labels
            .get(pane)
            .map(|(label, _)| label.clone())
            .unwrap_or_else(|| pane.clone());
        let text = if dirty { format!("• {label}") } else { label };
        RichText::new(text).color(text_dark()).into()
    }

    fn is_tab_closable(
        &self,
        _tiles: &egui_tiles::Tiles<String>,
        _tile_id: egui_tiles::TileId,
    ) -> bool {
        true
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        if let Some(egui_tiles::Tile::Pane(key)) = tiles.get(tile_id) {
            self.close_requests.push(key.clone());
        }
        // Never remove here: the close is routed through the unsaved-changes
        // prompt, which may cancel it.
        false
    }

    /// Restores the tab context menu and middle-click-to-close the hand-rolled
    /// tab rack used to carry. Both hang off the tab's own response, which is
    /// why they live here rather than in `tab_ui`.
    fn on_tab_button(
        &mut self,
        tiles: &mut egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
        button_response: egui::Response,
    ) -> egui::Response {
        let Some(egui_tiles::Tile::Pane(key)) = tiles.get(tile_id) else {
            return button_response;
        };
        let key = key.clone();
        // Queued exactly as the close button queues it, so a middle-click on a
        // tab with unsaved edits still raises the prompt instead of discarding
        // them. `tiles` is shared here, so the removal happens after the walk.
        if button_response.middle_clicked() {
            self.close_requests.push(key.clone());
        }
        // Clicking a tab focuses its tag, by the same rule as a press inside
        // its pane: `selected_key` is what Save and Save As act on, and a tab
        // brought forward by its title used to leave them on the tag before,
        // so Save As offered that tag's type and saved that tag.
        if button_response.clicked() {
            self.cx.send(EditorCommand::FocusTab {
                kit: self.kit,
                key: key.clone(),
            });
        }
        if is_folder_pane_key(&key) || key == GIT_REVIEW_KEY {
            context_menu(&button_response, |ui| {
                if ui.button("Close").clicked() {
                    self.close_requests.push(key.clone());
                    close_menu(ui);
                }
                if ui.button("Close all").clicked() {
                    self.close_all = true;
                    close_menu(ui);
                }
                if ui.button("Close all but this").clicked() {
                    self.close_all_but = Some(key.clone());
                    close_menu(ui);
                }
            });
            return button_response;
        }
        let discardable = self
            .cx
            .model
            .tag_has_discardable_changes(self.kit_index, &key);
        context_menu(&button_response, |ui| {
            if ui.button("Reveal in browser").clicked() {
                self.reveal = Some(key.clone());
                close_menu(ui);
            }
            if ui.button("Open with File Explorer").clicked() {
                self.reveal_in_explorer = Some(key.clone());
                close_menu(ui);
            }
            ui.separator();
            // Offered for every game. For a loose kit this drops the in-memory
            // edits and re-reads the file; for a container kit it also forgets
            // what the project stashed, or the edit comes straight back.
            if ui
                .add_enabled(discardable, egui::Button::new("Discard unsaved changes"))
                .on_disabled_hover_text("This tag has no unsaved changes")
                .clicked()
            {
                self.discard = Some(key.clone());
                close_menu(ui);
            }
            ui.separator();
            // Every container in the tag resolves its open state through one
            // place, so these reach groups, structs, blocks and arrays alike,
            // however deeply nested.
            if ui.button("Expand all").clicked() {
                self.expand = Some((key.clone(), true));
                close_menu(ui);
            }
            if ui.button("Collapse all").clicked() {
                self.expand = Some((key.clone(), false));
                close_menu(ui);
            }
            ui.separator();
            if ui.button("Close all").clicked() {
                self.close_all = true;
                close_menu(ui);
            }
            if ui.button("Close all but this").clicked() {
                self.close_all_but = Some(key.clone());
                close_menu(ui);
            }
        });
        button_response
    }

    /// Reimplements egui_tiles' default tab so each tab can carry its tag's
    /// group icon, as the hand-rolled tab rack did. Everything else — the
    /// close button, the drag sense, the active-tab hairline — mirrors the
    /// default; only the icon and the width it needs are new.
    fn tab_ui(
        &mut self,
        tiles: &mut egui_tiles::Tiles<String>,
        ui: &mut Ui,
        id: egui::Id,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        const ICON: f32 = 14.0;
        const ICON_GAP: f32 = 4.0;

        let pane_key = match tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(key)) => Some(key),
            _ => None,
        };
        let group_tag = match pane_key {
            Some(key) => self.group_tag_for_key(key),
            _ => None,
        };
        let folder_icon = pane_key.is_some_and(|key| is_folder_pane_key(key));
        let git_icon = pane_key.is_some_and(|key| key == GIT_REVIEW_KEY);
        let text = self.tab_title_for_tile(tiles, tile_id);
        let close_size = Vec2::splat(self.close_button_outer_size());
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, font_id);
        let x_margin = self.tab_title_spacing(ui.visuals());
        let icon_width = if group_tag.is_some() || folder_icon || git_icon {
            ICON + ICON_GAP
        } else {
            0.0
        };
        let width = galley.size().x
            + 2.0 * x_margin
            + icon_width
            + f32::from(state.closable) * (4.0 + close_size.x);
        let (_, tab_rect) = ui.allocate_space(egui::vec2(width, ui.available_height()));
        let response = ui
            .interact(tab_rect, id, Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Grab);

        if ui.is_rect_visible(tab_rect) && !state.is_being_dragged {
            let bg = self.tab_bg_color(ui.visuals(), tiles, tile_id, state);
            let stroke = self.tab_outline_stroke(ui.visuals(), tiles, tile_id, state);
            ui.painter().rect(tab_rect.shrink(0.5), 0.0, bg, stroke, egui::StrokeKind::Middle);
            if state.active {
                ui.painter().hline(
                    tab_rect.x_range(),
                    tab_rect.bottom(),
                    Stroke::new(stroke.width + 1.0, bg),
                );
            }
            let inner = tab_rect.shrink(x_margin);
            if group_tag.is_some() {
                let icon_rect = egui::Rect::from_center_size(
                    egui::pos2(inner.left() + ICON / 2.0, inner.center().y),
                    Vec2::splat(ICON),
                );
                let game = self.cx.model.kits[self.kit_index]
                    .source
                    .as_ref()
                    .and_then(|source| source.game);
                paint_tag_icon_at(ui, group_tag, game, icon_rect);
            } else if folder_icon {
                let icon_rect = egui::Rect::from_center_size(
                    egui::pos2(inner.left() + ICON / 2.0, inner.center().y),
                    Vec2::splat(ICON),
                );
                paint_button_icon_at(ui, ButtonIcon::FolderOpen, icon_rect, text_dark());
            } else if git_icon {
                let icon_rect = egui::Rect::from_center_size(
                    egui::pos2(inner.left() + ICON / 2.0, inner.center().y),
                    Vec2::splat(ICON),
                );
                paint_button_icon_at(ui, ButtonIcon::Git, icon_rect, text_dark());
            }
            let text_color = self.tab_text_color(ui.visuals(), tiles, tile_id, state);
            let text_pos = egui::Align2::LEFT_CENTER
                .align_size_within_rect(galley.size(), inner.translate(egui::vec2(icon_width, 0.0)))
                .min;
            ui.painter().galley(text_pos, galley, text_color);

            if state.closable {
                let close_rect =
                    egui::Align2::RIGHT_CENTER.align_size_within_rect(close_size, inner);
                let close_id = ui.auto_id_with("tab_close_btn");
                let close_response = ui
                    .interact(close_rect, close_id, Sense::click_and_drag())
                    .on_hover_cursor(egui::CursorIcon::Default);
                let visuals = ui.style().interact(&close_response);
                let rect = close_rect
                    .shrink(self.close_button_inner_margin())
                    .expand(visuals.expansion);
                let stroke = visuals.fg_stroke;
                ui.painter()
                    .line_segment([rect.left_top(), rect.right_bottom()], stroke);
                ui.painter()
                    .line_segment([rect.right_top(), rect.left_bottom()], stroke);
                if close_response.clicked() && self.on_tab_close(tiles, tile_id) {
                    tiles.remove(tile_id);
                }
            }
        }

        self.on_tab_button(tiles, tile_id, response)
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            // Keep a lone pane wrapped in its tab group so it still shows a tab
            // bar to drag, close, and drop onto.
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> Color32 {
        row_type()
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        tiles: &egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        let base = if state.active {
            active_tab()
        } else {
            row_type()
        };
        let dirty = matches!(tiles.get(tile_id), Some(egui_tiles::Tile::Pane(key))
            if self.cx.model.kits[self.kit_index]
                .parsed_tags
                .get(key)
                .is_some_and(|document| document.dirty.is_set()));
        if dirty {
            tint_toward(base, Color32::from_rgb(184, 134, 11), 0.20)
        } else {
            base
        }
    }

    fn tab_text_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<String>,
        _tile_id: egui_tiles::TileId,
        _state: &egui_tiles::TabState,
    ) -> Color32 {
        text_dark()
    }
}

impl TagPaneBehavior<'_, '_, '_> {
    fn group_tag_for_key(&self, key: &str) -> Option<u32> {
        // The Bitmap Library, Model Library, and Blam! are not tags and have no
        // group icon; `Some(0)` here would reserve icon space and paint
        // whatever group zero resolves to.
        if key == BITMAP_LIBRARY_KEY
            || key == MODEL_LIBRARY_KEY
            || key == GIT_REVIEW_KEY
            || key == BLAM_KEY
        {
            return None;
        }
        self.tab_labels
            .get(key)
            .and_then(|(_, group_tag)| *group_tag)
    }
}

/// Resolve every open pane's tab label and group tag in a single pass over
/// the source, keyed by tag key.
fn tab_labels_for_open_panes(
    kit: &Kit,
    view: &KitView,
    tree: &egui_tiles::Tree<String>,
) -> HashMap<String, (String, Option<u32>)> {
    let mut labels = HashMap::new();
    // One targeted scan per open pane. Iterating the entries instead and
    // testing each against a set of the open keys was measurably *slower*:
    // it hashes all 24,000 keys rather than comparing a few thousand that
    // mostly differ early.
    for tile in tree.tiles.tiles() {
        let egui_tiles::Tile::Pane(key) = tile else {
            continue;
        };
        if labels.contains_key(key) {
            continue;
        }
        if key == BITMAP_LIBRARY_KEY {
            // No group, so no icon: `group_tag_for_key` reads this map, and
            // the bitmap group's icon would claim this is a bitmap tag.
            labels.insert(key.clone(), (BITMAP_LIBRARY_TITLE.to_owned(), None));
            continue;
        }
        if key == MODEL_LIBRARY_KEY {
            labels.insert(key.clone(), (MODEL_LIBRARY_TITLE.to_owned(), None));
            continue;
        }
        if key == GIT_REVIEW_KEY {
            labels.insert(key.clone(), (GIT_REVIEW_TITLE.to_owned(), None));
            continue;
        }
        if key == BLAM_KEY {
            labels.insert(key.clone(), (BLAM_TITLE.to_owned(), None));
            continue;
        }
        if is_folder_pane_key(key) {
            let label = view
                .browser
                .folder_browsers
                .get(key)
                .map(|folder| folder.label.clone())
                .unwrap_or_else(|| "Folder".to_owned());
            labels.insert(key.clone(), (label, None));
            continue;
        }
        let found = kit
            .source
            .as_ref()
            .and_then(|source| source.entry_for_key(key))
            .or_else(|| {
                kit.active_favorite_entries
                    .iter()
                    .find(|entry| &entry.key == key)
            });
        if let Some(entry) = found {
            labels.insert(key.clone(), (tag_tab_label(entry), Some(entry.group_tag)));
        }
    }
    labels
}

/// The entry a tag pane shows: from the kit's source, or one of its favorites
/// from outside it.
fn tag_pane_entry(kit: &Kit, key: &str) -> Option<TagEntry> {
    kit.source
        .as_ref()
        .and_then(|source| source.entry_for_key(key))
        .or_else(|| {
            kit.active_favorite_entries
                .iter()
                .find(|entry| entry.key == key)
        })
        .cloned()
}

/// One open tag pane's caches, filled before the tiles draw.
struct TagPaneCaches {
    def_docs: Option<Rc<DefDocs>>,
    ce_sound: Option<Arc<crate::core::source::ce_audio::CeSoundBinding>>,
}

/// What the kits' tiles draw from that only the application can work out:
/// caches the draws read but cannot fill. See [`Baboon::prepare_tiles`].
#[derive(Default)]
pub(in crate::app) struct TileInputs {
    tags: HashMap<(KitId, String), TagPaneCaches>,
}

impl Baboon {
    /// Fill every cache the kits' tiles will read, before they draw: the
    /// browser's modified and deletable marks, folder panes (which can load a
    /// lazy folder into the source), thumbnail libraries, and each open tag's
    /// definition docs and Campaign Evolved sound binding.
    ///
    /// Every pane in every tree, not only the visible ones: a tab clicked this
    /// frame draws this frame, and each of these is cached, so what is already
    /// current costs a lookup.
    pub(in crate::app) fn prepare_tiles(&mut self, ctx: &egui::Context) -> TileInputs {
        let mut inputs = TileInputs::default();
        for kit_index in 0..self.model.kits.len() {
            let kit = &self.model.kits[kit_index];
            let kit_id = kit.id;
            if kit.is_empty_workspace() || self.shows_chimp_surface(kit_index) {
                continue;
            }
            self.refresh_modified_tags(kit_index);
            self.refresh_deletable_keys(kit_index);
            let keys: Vec<String> = self.views[kit_id]
                .tag_tree
                .tiles
                .tiles()
                .filter_map(|tile| match tile {
                    egui_tiles::Tile::Pane(key) => Some(key.clone()),
                    _ => None,
                })
                .collect();
            for key in keys {
                if is_folder_pane_key(&key) {
                    self.refresh_folder_browser_pane(kit_index, &key, ctx);
                } else if key == BITMAP_LIBRARY_KEY {
                    self.refresh_thumbnail_library::<Bitmaps>(kit_index, ctx);
                } else if key == MODEL_LIBRARY_KEY {
                    self.refresh_thumbnail_library::<Models>(kit_index, ctx);
                } else if key == GIT_REVIEW_KEY || key == BLAM_KEY {
                } else if let Some(entry) = tag_pane_entry(&self.model.kits[kit_index], &key) {
                    let def_docs = self.def_docs_for_entry(kit_index, &entry);
                    let ce_sound = self.ce_sound_binding(kit_index, &entry.key, &entry);
                    inputs
                        .tags
                        .insert((kit_id, key), TagPaneCaches { def_docs, ce_sound });
                }
            }
        }
        inputs
    }
}

/// Draw one kit's open tags as a tiled layout.
pub(in crate::app) fn draw_tag_tiles(
    ui: &mut Ui,
    cx: &Ctx,
    parts: &mut TileParts,
    inputs: &TileInputs,
    kit_index: usize,
) {
    let kit = cx.model.kits[kit_index].id;
    if parts.views[kit].tag_tree.is_empty() {
        // An unloaded workspace never reaches here — it shows the welcome
        // screen instead — so this is only ever "loaded, nothing open yet".
        centered_empty_state(
            ui,
            "Select a Tag (or Folder) from the browser to open it here.",
        );
        return;
    }

    // Move the tree out for the duration: the behavior draws into the view
    // the tree lives in.
    let placeholder = egui_tiles::Tree::empty(tag_tree_id(kit));
    let mut tree = std::mem::replace(&mut parts.views[kit].tag_tree, placeholder);
    let tab_labels = tab_labels_for_open_panes(&cx.model.kits[kit_index], &parts.views[kit], &tree);
    let mut behavior = TagPaneBehavior {
        cx,
        parts,
        inputs,
        kit_index,
        kit,
        tab_labels,
        close_requests: Vec::new(),
        reveal: None,
        reveal_in_explorer: None,
        discard: None,
        expand: None,
        close_all: false,
        close_all_but: None,
    };
    tree.ui(&mut behavior, ui);
    let TagPaneBehavior {
        parts,
        close_requests,
        reveal,
        reveal_in_explorer,
        discard,
        expand,
        close_all,
        close_all_but,
        ..
    } = behavior;
    parts.views[kit].tag_tree = tree;

    // The tree owns the layout, so a drag or split there is what changes the
    // open set — re-derive it rather than the other way round.
    cx.send(EditorCommand::SyncOpenTabs { kit });
    // Everything below addresses the *active* kit: the close prompt and the
    // save paths under it resolve documents there, and `reveal_in_browser`
    // scrolls that kit's browser. These all answer a click on a tab in *this*
    // pane, so the pane's kit has to be active before they run.
    //
    // Without this, closing a tab in an unfocused pane of a split closes it
    // in the other game.
    if reveal.is_some()
        || reveal_in_explorer.is_some()
        || close_all
        || close_all_but.is_some()
        || !close_requests.is_empty()
    {
        cx.send(AppAction::FocusKit(kit));
    }
    if let Some(key) = reveal {
        cx.send(AppAction::RevealInBrowser { kit, key });
    }
    if let Some(key) = reveal_in_explorer {
        cx.send(BrowserCommand::Action {
            kit,
            action: BrowserAction::OpenInExplorer(key),
        });
    }
    if let Some(key) = discard {
        cx.send(AppAction::DiscardChanges { kit, key });
    }
    if let Some((key, open)) = expand {
        parts.views[kit].pending_expand.insert(key, open);
    }
    if close_all {
        cx.send(AppAction::Close(PendingCloseAction::CloseAllTabs));
    } else if let Some(key) = close_all_but {
        cx.send(AppAction::Close(PendingCloseAction::CloseAllButThis(key)));
    }
    for key in close_requests {
        cx.send(AppAction::Close(PendingCloseAction::CloseTab(key)));
    }
}
