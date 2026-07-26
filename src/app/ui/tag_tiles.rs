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
                    .draw_tag_pane(ui, &self.ctx, self.kit_index, &entry, &scope, true);
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

        let group_tag = match tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(key)) => self.group_tag_for_key(key),
            _ => None,
        };
        let text = self.tab_title_for_tile(tiles, tile_id);
        let close_size = Vec2::splat(self.close_button_outer_size());
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, font_id);
        let x_margin = self.tab_title_spacing(ui.visuals());
        let icon_width = if group_tag.is_some() {
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
            ui.painter().rect(tab_rect.shrink(0.5), 0.0, bg, stroke);
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
                paint_tag_icon_at(ui, group_tag, icon_rect);
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

impl TagPaneBehavior<'_> {
    fn group_tag_for_key(&self, key: &str) -> Option<u32> {
        let source = self.app.kits[self.kit_index].source.as_ref()?;
        source
            .entries
            .iter()
            .chain(source.all_entries.iter())
            .find(|entry| entry.key == key)
            .map(|entry| entry.group_tag)
    }
}

impl Baboon {
    /// Draw one kit's open tags as a tiled layout.
    pub(super) fn draw_tag_tiles(&mut self, ui: &mut Ui, ctx: &egui::Context, kit_index: usize) {
        if self.kits[kit_index].tag_tree.is_empty() {
            // An unloaded workspace never reaches here — it shows the welcome
            // screen instead — so this is only ever "loaded, nothing open yet".
            ui.heading(RichText::new("No tag open").color(text_dark()));
            ui.label(
                RichText::new("Select a tag in the browser to open it here.").color(subtle_dark()),
            );
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
