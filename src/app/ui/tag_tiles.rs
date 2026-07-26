//! Tiled layout of a kit's open tags: tab groups, splits, and drag-to-rearrange.
//! It owns the editor area's layout; one pane's contents belong to `tag_pane`.

use super::*;

/// Bridges `egui_tiles` back to [`Baboon`] while a kit's tree is being drawn.
///
/// The tree lives on the kit, but rendering a pane needs `&mut Baboon`, so the
/// caller moves the tree out for the duration and hands the app to the
/// behavior. Closes are collected rather than applied inline: removing a tile
/// while `egui_tiles` is walking the tree would invalidate its iteration, and
/// a close has to go through the unsaved-changes prompt anyway.
struct TagPaneBehavior<'a> {
    app: &'a mut Baboon,
    kit_index: usize,
    ctx: egui::Context,
    close_requests: Vec<String>,
    focused: Option<String>,
    /// Deferred tab context-menu choices, applied after the tree is drawn for
    /// the same reason closes are: they mutate the layout or the open set.
    reveal: Option<String>,
    close_all: bool,
    close_all_but: Option<String>,
}

impl egui_tiles::Behavior<String> for TagPaneBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut Ui,
        tile_id: egui_tiles::TileId,
        pane: &mut String,
    ) -> egui_tiles::UiResponse {
        let key = pane.clone();
        let Some(entry) = self.app.kits[self.kit_index]
            .source
            .as_ref()
            .and_then(|source| {
                source
                    .entries
                    .iter()
                    .chain(source.all_entries.iter())
                    .find(|entry| entry.key == key)
            })
            .cloned()
            .or_else(|| {
                self.app.kits[self.kit_index]
                    .active_favorite_entries
                    .iter()
                    .find(|entry| entry.key == key)
                    .cloned()
            })
        else {
            ui.label(RichText::new("This tag is no longer in the source").color(subtle_dark()));
            return egui_tiles::UiResponse::None;
        };

        // The scope salts every widget id under the pane, so the same tag shown
        // in two panes keeps independent scroll, focus, and collapse state
        // while both edit the one shared document.
        let scope = format!("tile{}", tile_id.0);
        if ui.rect_contains_pointer(ui.max_rect()) {
            self.focused = Some(key.clone());
        }
        egui::ScrollArea::vertical()
            .id_salt(("tag_tile", tile_id.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.app
                    .draw_tag_pane(ui, &self.ctx, &entry, &scope, true);
            });
        egui_tiles::UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &String) -> egui::WidgetText {
        let dirty = self.app.kits[self.kit_index]
            .parsed_tags
            .get(pane)
            .is_some_and(|document| document.dirty);
        let label = self
            .app
            .kits[self.kit_index]
            .source
            .as_ref()
            .and_then(|source| {
                source
                    .entries
                    .iter()
                    .chain(source.all_entries.iter())
                    .find(|entry| &entry.key == pane)
            })
            .map(tag_tab_label)
            .unwrap_or_else(|| pane.clone());
        let text = if dirty {
            format!("● {label}")
        } else {
            label
        };
        RichText::new(text).color(text_dark()).into()
    }

    fn is_tab_closable(&self, _tiles: &egui_tiles::Tiles<String>, _tile_id: egui_tiles::TileId) -> bool {
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

    /// Restores the tab context menu the hand-rolled tab rack used to carry.
    fn on_tab_button(
        &mut self,
        tiles: &egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
        button_response: egui::Response,
    ) -> egui::Response {
        let Some(egui_tiles::Tile::Pane(key)) = tiles.get(tile_id) else {
            return button_response;
        };
        let key = key.clone();
        button_response.context_menu(|ui| {
            if ui.button("Reveal in browser").clicked() {
                self.reveal = Some(key.clone());
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Close all").clicked() {
                self.close_all = true;
                ui.close_menu();
            }
            if ui.button("Close all but this").clicked() {
                self.close_all_but = Some(key.clone());
                ui.close_menu();
            }
        });
        button_response
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
        let base = if state.active { menu_bar() } else { row_type() };
        let dirty = matches!(tiles.get(tile_id), Some(egui_tiles::Tile::Pane(key))
            if self.app.kits[self.kit_index]
                .parsed_tags
                .get(key)
                .is_some_and(|document| document.dirty));
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

impl Baboon {
    /// Draw the active kit's open tags as a tiled layout.
    pub(super) fn draw_tag_tiles(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let kit_index = self.active;
        if self.kits[kit_index].tag_tree.is_empty() {
            if self.kits[kit_index].source.is_none() {
                ui.heading("No tag selected");
                ui.label("Load a source from File, then select a tag in the browser.");
            } else {
                ui.heading("No tag selected");
                ui.label("Select a tag in the browser to open it here.");
            }
            return;
        }

        // Move the tree out for the duration: the behavior needs `&mut Baboon`,
        // and the tree lives on a kit inside it.
        let placeholder = egui_tiles::Tree::empty(tag_tree_id(self.kits[kit_index].id));
        let mut tree = std::mem::replace(&mut self.kits[kit_index].tag_tree, placeholder);
        let mut behavior = TagPaneBehavior {
            app: self,
            kit_index,
            ctx: ctx.clone(),
            close_requests: Vec::new(),
            focused: None,
            reveal: None,
            close_all: false,
            close_all_but: None,
        };
        tree.ui(&mut behavior, ui);
        let close_requests = std::mem::take(&mut behavior.close_requests);
        let focused = behavior.focused.take();
        let reveal = behavior.reveal.take();
        let close_all = behavior.close_all;
        let close_all_but = behavior.close_all_but.take();
        self.kits[kit_index].tag_tree = tree;

        // The tree owns the layout, so a drag or split there is what changes
        // the open set — re-derive it rather than the other way round.
        self.kits[kit_index].sync_open_tabs();
        if let Some(key) = focused {
            self.kits[kit_index].selected_key = Some(key);
        }
        if let Some(key) = reveal {
            self.reveal_in_browser(&key);
        }
        if close_all {
            self.request_close_action(PendingCloseAction::CloseAllTabs, ctx);
        } else if let Some(key) = close_all_but {
            self.request_close_action(PendingCloseAction::CloseAllButThis(key), ctx);
        }
        for key in close_requests {
            self.request_close_action(PendingCloseAction::CloseTab(key), ctx);
        }
    }
}
