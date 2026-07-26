//! Tiled layout of loaded kits: the toplevel game tabs and their splits.
//! It owns which kit workspaces are visible and how they are arranged; one workspace's contents belong to the browser and tag-tile modules.

use super::*;
use super::recents::{RecentAction, draw_recent_folders_menu};

/// Which loader the "+" menu on the kit tab bar should start.
#[derive(Clone, Copy)]
enum LoadKind {
    Folder,
    SingleFile,
    Monolithic,
    Container,
}

/// Bridges `egui_tiles` back to [`Baboon`] while the kit tree is being drawn.
///
/// Mirrors [`super::tag_tiles`] one level up: the tree is moved off `Baboon`
/// for the duration so a pane can borrow the app, and anything that mutates
/// the kit list is collected and applied after the walk.
struct KitPaneBehavior<'a> {
    app: &'a mut Baboon,
    ctx: egui::Context,
    close_requests: Vec<KitId>,
    focused: Option<KitId>,
    add_kit: Option<LoadKind>,
    recent_action: Option<RecentAction>,
}

impl egui_tiles::Behavior<KitId> for KitPaneBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut KitId,
    ) -> egui_tiles::UiResponse {
        let kit_id = *pane;
        let Some(kit_index) = self.app.kit_index(kit_id) else {
            ui.label(RichText::new("This kit is no longer open").color(subtle_dark()));
            return egui_tiles::UiResponse::None;
        };
        // Activate on a press inside the pane, not on hover. `active` decides
        // where deferred work lands — the save prompt, Ctrl+S, an open colour
        // picker or reference picker — and none of that should retarget just
        // because the cursor crossed another game's pane on its way somewhere.
        if ui.input(|input| input.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect())
        {
            self.focused = Some(kit_id);
        }
        // An unloaded workspace has no tags to browse or edit, so it offers
        // ways to open one instead of an empty browser and an empty editor.
        if self.app.kits[kit_index].is_empty_workspace() {
            self.app.draw_welcome_screen(ui, &self.ctx);
            return egui_tiles::UiResponse::None;
        }
        // Each workspace carries its own browser, so two games side by side can
        // be browsed independently rather than sharing one panel that
        // retargets as focus moves.
        egui::SidePanel::left(egui::Id::new(("kit_browser_panel", kit_id.0)))
            .resizable(true)
            .default_width(330.0)
            .frame(Frame::none().fill(left_panel()).inner_margin(egui::Margin {
                left: 8.0,
                right: 8.0,
                top: 6.0,
                bottom: 6.0,
            }))
            .show_inside(ui, |ui| {
                self.app.draw_kit_browser(ui, &self.ctx, kit_index);
            });
        egui::CentralPanel::default()
            .frame(Frame::none().fill(editor_bg()).inner_margin(egui::Margin {
                left: 10.0,
                right: 10.0,
                top: 8.0,
                bottom: 8.0,
            }))
            .show_inside(ui, |ui| {
                self.app.draw_tag_tiles(ui, &self.ctx, kit_index);
            });
        egui_tiles::UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &KitId) -> egui::WidgetText {
        let Some(index) = self.app.kit_index(*pane) else {
            return RichText::new("(closed)").color(subtle_dark()).into();
        };
        let kit = &self.app.kits[index];
        // Stashed edits count as modifications too. They are not written into
        // the game, and the tag holding them may not even be open, so without
        // this a workspace carrying work from an earlier session looked clean.
        let stashed = kit
            .campaign_project
            .as_ref()
            .is_some_and(|project| !project.overlays.is_empty());
        let dirty = stashed || kit.parsed_tags.values().any(|document| document.dirty);
        let label = kit_strip_label(kit);
        let text = if dirty {
            format!("• {label}")
        } else {
            label
        };
        RichText::new(text).color(text_dark()).strong().into()
    }

    fn is_tab_closable(
        &self,
        _tiles: &egui_tiles::Tiles<KitId>,
        _tile_id: egui_tiles::TileId,
    ) -> bool {
        true
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<KitId>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        if let Some(egui_tiles::Tile::Pane(kit_id)) = tiles.get(tile_id) {
            self.close_requests.push(*kit_id);
        }
        // The kit close is routed through the unsaved-changes prompt, which can
        // cancel it, so the tile is never removed here.
        false
    }

    /// The "+" that opens another game, kept on the tab bar where the kit tabs
    /// themselves are.
    fn top_bar_right_ui(
        &mut self,
        _tiles: &egui_tiles::Tiles<KitId>,
        ui: &mut Ui,
        _tile_id: egui_tiles::TileId,
        _tabs: &egui_tiles::Tabs,
        _scroll_offset: &mut f32,
    ) {
        let recents = self.app.recent_folders.clone();
        ui.menu_button("+", |ui| {
            ui.set_min_width(240.0);
            if ui.button("Load Folder...").clicked() {
                ui.close_menu();
                self.add_kit = Some(LoadKind::Folder);
            }
            if ui.button("Load Tag...").clicked() {
                ui.close_menu();
                self.add_kit = Some(LoadKind::SingleFile);
            }
            if ui.button("Load Monolithic blob_index.dat...").clicked() {
                ui.close_menu();
                self.add_kit = Some(LoadKind::Monolithic);
            }
            if ui
                .button("Open Campaign Evolved container (.utoc)...")
                .clicked()
            {
                ui.close_menu();
                self.add_kit = Some(LoadKind::Container);
            }
            ui.separator();
            ui.menu_button("Recent", |ui| {
                self.recent_action = draw_recent_folders_menu(ui, &recents);
            });
        })
        .response
        .on_hover_text("Open another game in its own workspace");
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        26.0
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
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
        tiles: &egui_tiles::Tiles<KitId>,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        let base = if state.active { menu_bar() } else { left_panel() };
        let dirty = matches!(tiles.get(tile_id), Some(egui_tiles::Tile::Pane(kit_id))
            if self.app.kit_index(*kit_id).is_some_and(|index| {
                self.app.kits[index]
                    .parsed_tags
                    .values()
                    .any(|document| document.dirty)
            }));
        if dirty {
            tint_toward(base, Color32::from_rgb(184, 134, 11), 0.20)
        } else {
            base
        }
    }

    fn tab_text_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<KitId>,
        _tile_id: egui_tiles::TileId,
        _state: &egui_tiles::TabState,
    ) -> Color32 {
        text_dark()
    }
}

impl Baboon {
    /// Draw every open kit as a tiled workspace. Dragging a game tab against a
    /// pane edge splits the window between two games.
    pub(super) fn draw_kit_tiles(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        self.sync_kit_tree();
        // Nothing to tab between: draw the welcome screen directly rather than
        // wrapping it in a tree whose tab bar would be an empty strip. A
        // zero-height tab bar still leaves the "+" and the bar's own painting
        // behind, so the tree is skipped outright.
        if self.kits.len() == 1 && self.kits[0].is_empty_workspace() {
            self.draw_welcome_screen(ui, ctx);
            return;
        }
        let placeholder = egui_tiles::Tree::empty(egui::Id::new("kit_tree_placeholder"));
        let mut tree = std::mem::replace(&mut self.kit_tree, placeholder);
        let mut behavior = KitPaneBehavior {
            app: self,
            ctx: ctx.clone(),
            close_requests: Vec::new(),
            focused: None,
            add_kit: None,
            recent_action: None,
        };
        tree.ui(&mut behavior, ui);
        let close_requests = std::mem::take(&mut behavior.close_requests);
        let focused = behavior.focused.take();
        let add_kit = behavior.add_kit.take();
        let recent_action = behavior.recent_action.take();
        self.kit_tree = tree;

        if let Some(kit_id) = focused
            && let Some(index) = self.kit_index(kit_id)
        {
            self.active = index;
        }
        for kit_id in close_requests {
            self.request_close_action(PendingCloseAction::CloseKit(kit_id), ctx);
        }
        if let Some(action) = recent_action {
            self.apply_recent_action(action, ctx);
        }
        if let Some(kind) = add_kit {
            match kind {
                LoadKind::Folder => self.begin_load_folder(ctx.clone()),
                LoadKind::SingleFile => self.begin_load_single(ctx.clone()),
                LoadKind::Monolithic => self.begin_load_monolithic(ctx.clone()),
                LoadKind::Container => self.begin_load_iostore_container(ctx.clone()),
            }
        }
    }

    /// Reconcile the layout tree with the kit list: add panes for kits opened
    /// since the last frame, drop panes for kits that have closed.
    ///
    /// The kit list is the content store and the tree only references it, so
    /// this only ever repairs the tree — it never creates or removes a kit.
    fn sync_kit_tree(&mut self) {
        let live: Vec<KitId> = self.kits.iter().map(|kit| kit.id).collect();
        let laid_out: Vec<(egui_tiles::TileId, KitId)> = self
            .kit_tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| match tile {
                egui_tiles::Tile::Pane(kit_id) => Some((*id, *kit_id)),
                _ => None,
            })
            .collect();
        for (tile_id, kit_id) in &laid_out {
            if !live.contains(kit_id) {
                self.kit_tree.remove_recursively(*tile_id);
            }
        }
        for kit_id in live {
            if laid_out.iter().any(|(_, laid)| *laid == kit_id) {
                continue;
            }
            let tile_id = self.kit_tree.tiles.insert_pane(kit_id);
            match self.kit_tree.root() {
                Some(root) => {
                    if let Some(egui_tiles::Tile::Container(container)) =
                        self.kit_tree.tiles.get_mut(root)
                    {
                        container.add_child(tile_id);
                    } else {
                        let tabs = self.kit_tree.tiles.insert_tab_tile(vec![root, tile_id]);
                        self.kit_tree.root = Some(tabs);
                    }
                }
                None => self.kit_tree.root = Some(tile_id),
            }
            self.kit_tree.make_active(|id, _| id == tile_id);
        }
    }
}
