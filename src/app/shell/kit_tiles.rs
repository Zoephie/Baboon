//! Tiled layout of loaded kits: the toplevel game tabs and their splits.
//! It owns which kit workspaces are visible and how they are arranged; one workspace's contents belong to the browser and tag-tile modules.

use super::recents::{RecentAction, draw_recent_folders_menu};
use super::*;
use crate::app::shell::frame::wheel_scroll_tab_bar;


fn draw_home_tab(ui: &mut Ui, group: egui_tiles::TileId, active: bool) -> egui::Response {
    let (_, rect) = ui.allocate_space(Vec2::new(32.0, 26.0));
    let response = ui.interact(rect, egui::Id::new("workspace_home_tab").with(group), Sense::click()).on_hover_text("Home");
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, "Home"));
    let fill = if active { left_panel() } else if response.hovered() { context_menu_hover() }
        else { Color32::TRANSPARENT };
    crate::app::shell::frame::paint_connected_tab(ui, rect, fill, active);
    paint_button_icon_at(ui, ButtonIcon::Home,
        egui::Rect::from_center_size(rect.center(), Vec2::splat(18.0)), text_dark());
    response
}

/// Which loader the "+" menu on the kit tab bar should start.
#[derive(Clone, Copy)]
enum LoadKind {
    Folder,
    SingleFile,
    Monolithic,
    Container,
}

/// The application state the kits' tiles draw into, borrowed field by field
/// beside the [`Ctx`] that holds the model. Built by [`tile_parts!`].
pub(in crate::app) struct TileParts<'a> {
    pub(in crate::app) views: &'a mut KitViews,
    pub(in crate::app) shell: &'a mut ShellFeature,
    pub(in crate::app) browser: &'a mut BrowserFeature,
    pub(in crate::app) chimp: &'a mut ChimpFeature,
    pub(in crate::app) editor: &'a mut EditorFeature,
    pub(in crate::app) kit_tools: &'a KitsFeature,
    pub(in crate::app) audio: &'a crate::app::audio::AudioState,
    pub(in crate::app) search: &'a SearchFeature,
    pub(in crate::app) references: &'a ReferencesFeature,
}

/// The [`TileParts`] of an application, borrowed beside a [`cx!`] of it.
macro_rules! tile_parts {
    ($app:expr) => {
        $crate::app::shell::kit_tiles::TileParts {
            views: &mut $app.views,
            shell: &mut $app.shell,
            browser: &mut $app.browser,
            chimp: &mut $app.chimp,
            editor: &mut $app.editor,
            kit_tools: &$app.kit_tools,
            audio: &$app.audio,
            search: &$app.search,
            references: &$app.references,
        }
    };
}

/// Draws the kits for `egui_tiles` while the kit tree is moved off the
/// application.
///
/// Mirrors [`super::tag_tiles`] one level up: anything that changes the kit
/// list is collected during the walk and sent as a command after it.
struct KitPaneBehavior<'a, 'c, 'p> {
    cx: &'a Ctx<'c>,
    parts: TileParts<'p>,
    inputs: &'a TileInputs,
    close_requests: Vec<KitId>,
    add_kit: Option<LoadKind>,
    recent_action: Option<RecentAction>,
    load_target: Option<KitId>,
    tab_groups: HashMap<egui_tiles::TileId, egui_tiles::TileId>,
    primary_group: Option<egui_tiles::TileId>,
    inactive_dividers: HashSet<egui_tiles::TileId>,
}

impl egui_tiles::Behavior<KitId> for KitPaneBehavior<'_, '_, '_> {
    fn tab_bar_left_ui(
        &mut self, tiles: &egui_tiles::Tiles<KitId>, ui: &mut Ui,
        tile_id: egui_tiles::TileId, tabs: &egui_tiles::Tabs,
    ) {
        for child in &tabs.children { self.tab_groups.insert(*child, tile_id); }
        let empty = tabs.active.and_then(|id| tiles.get(id)).is_some_and(|tile| {
            matches!(tile, egui_tiles::Tile::Pane(kit) if self.cx.model.kit_index(*kit)
                .is_some_and(|index| self.cx.model.kits[index].can_accept_source_load()))
        });
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(4.0);
        // Tile traversal visits the top/left child first, including nested
        // splits, so the first tab bar is the primary workspace's bar.
        if *self.primary_group.get_or_insert(tile_id) == tile_id {
            if draw_home_tab(ui, tile_id, empty || self.parts.shell.home_groups.contains(&tile_id)).clicked() {
                self.cx.send(AppAction::ShowHome { group: tile_id });
            }
        } else {
            self.parts.shell.home_groups.remove(&tile_id);
        }
        // Record neighbors here: the tab container is temporarily removed
        // from Tiles while its children draw. Home makes every kit inactive.
        let home = self.parts.shell.home_groups.contains(&tile_id);
        let visible = tabs.children.iter().copied().filter(|id| tiles.is_visible(*id)).collect::<Vec<_>>();
        for pair in visible.windows(2) {
            if home || (!tabs.is_active(pair[0]) && !tabs.is_active(pair[1])) {
                self.inactive_dividers.insert(pair[0]);
            }
        }
    }
    fn pane_ui(
        &mut self,
        ui: &mut Ui,
        tile_id: egui_tiles::TileId,
        pane: &mut KitId,
    ) -> egui_tiles::UiResponse {
        let kit_id = *pane;
        let cx = self.cx;
        let Some(kit_index) = cx.model.kit_index(kit_id) else {
            ui.label(RichText::new("This kit is no longer open").color(subtle_dark()));
            return egui_tiles::UiResponse::None;
        };
        let group = self.tab_groups.get(&tile_id).copied();
        let kit = &cx.model.kits[kit_index];
        let home = kit.is_empty_workspace()
            || group.is_some_and(|id| self.parts.shell.home_groups.contains(&id));
        if ui.input(|input| input.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect()) {
            if home && let Some(group) = group.filter(|id| Some(*id) == self.primary_group) {
                cx.send(AppAction::ShowHome { group });
            } else {
                cx.send(AppAction::FocusKit(kit_id));
            }
        }
        let parts = &mut self.parts;
        if home {
            ui.painter().rect_filled(ui.max_rect(), 0.0, left_panel());
            draw_welcome_screen(cx, ui, kit_index, parts.shell,
                &parts.kit_tools.editing_kit_validation);
            return egui_tiles::UiResponse::None;
        }
        if is_campaign_evolved(kit) && cx.model.prefs.enable_chimp {
            Frame::NONE
                .fill(menu_bar())
                .inner_margin(egui::Margin {
                    left: 8,
                    right: 8,
                    top: 4,
                    bottom: 4,
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Campaign Evolved").strong());
                        ui.separator();
                        for (surface, label, hover) in KitSurface::TABS {
                            ui.selectable_value(&mut parts.views[kit_id].surface, surface, label)
                                .on_hover_text(hover);
                        }
                    });
                });
            if parts.views[kit_id].surface == KitSurface::Chimp {
                draw_chimp_workspace(
                    ui,
                    cx,
                    parts.chimp,
                    &mut parts.views[kit_id].chimp,
                    kit_index,
                );
                return egui_tiles::UiResponse::None;
            }
        }
        // Each workspace carries its own browser, so two games side by side can
        // be browsed independently rather than sharing one panel that
        // retargets as focus moves.
        let browser_id = egui::Id::new(("kit_browser_panel", kit_id.0));
        let browser_panel = egui::Panel::left(browser_id)
            .resizable(true)
            .show_separator_line(false)
            .default_size(330.0)
            .frame(Frame::NONE.fill(left_panel()).inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top: 6,
                bottom: 6,
            }))
            .show(ui, |ui| {
                let game = kit.source.as_ref().and_then(|source| source.game);
                let profile = kit.profile.as_ref().map(|profile| profile.id.as_str());
                let banner = game.and_then(|game| {
                    parts.shell.artwork.workspace_banner(
                        cx.egui,
                        &cx.model.prefs.custom_editing_kit_profiles,
                        Some(game),
                        profile,
                    )
                });
                draw_kit_browser(
                    cx,
                    ui,
                    kit_index,
                    &mut parts.views[kit_id],
                    parts.browser,
                    banner,
                    parts.shell.blender_icon.as_ref(),
                    parts.audio.language.as_deref(),
                );
            });
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(left_panel()).inner_margin(egui::Margin {
                left: 6, right: 6, top: 6, bottom: 6,
            }))
            .show(ui, |ui| {
                draw_tag_tiles(ui, cx, parts, self.inputs, kit_index);
            });
        // Paint after the editor so its background cannot cover the transient
        // separator. The panel retains its native resize behavior and width.
        if let Some(handle) = ui.ctx().read_response(browser_id.with("__resize"))
            && (handle.hovered() || handle.dragged())
        {
            ui.painter().vline(browser_panel.response.rect.right(), browser_panel.response.rect.y_range(),
                Stroke::new(if handle.dragged() { 2.0 } else { 1.0 }, subtle_dark()));
        }
        egui_tiles::UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &KitId) -> egui::WidgetText {
        let Some(index) = self.cx.model.kit_index(*pane) else {
            return RichText::new("(closed)").color(subtle_dark()).into();
        };
        let kit = &self.cx.model.kits[index];
        let label = kit_strip_label(kit);
        RichText::new(label).color(text_dark()).strong().into()
    }

    fn tab_ui(
        &mut self, tiles: &mut egui_tiles::Tiles<KitId>, ui: &mut Ui,
        id: egui::Id, tile_id: egui_tiles::TileId, state: &egui_tiles::TabState,
    ) -> egui::Response {
        const CLOSE_GAP: f32 = 20.0;
        if self.cx.model.kits.len() == 1 && matches!(tiles.get(tile_id), Some(egui_tiles::Tile::Pane(kit))
            if self.cx.model.kit_index(*kit).is_some_and(|index| self.cx.model.kits[index].can_accept_source_load())) {
            return ui.allocate_response(Vec2::ZERO, Sense::hover());
        }
        let mut state = state.clone();
        state.active &= !self.tab_groups.get(&tile_id)
            .is_some_and(|group| self.parts.shell.home_groups.contains(group));
        let state = &state;
        let dirty = matches!(tiles.get(tile_id), Some(egui_tiles::Tile::Pane(kit))
            if self.cx.model.kit_index(*kit).is_some_and(|index| self.cx.model.kits[index].has_unwritten_modifications()));
        let text = self.tab_title_for_tile(tiles, tile_id);
        let font = egui::TextStyle::Button.resolve(ui.style());
        let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, font);
        let margin = self.tab_title_spacing(ui.visuals());
        let close_size = Vec2::splat(self.close_button_outer_size());
        let width = galley.size().x + margin * 2.0
            + f32::from(state.closable) * (CLOSE_GAP + close_size.x);
        let (_, rect) = ui.allocate_space(egui::vec2(width, ui.available_height()));
        let response = ui.interact(rect, id, Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Grab);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button,
            ui.is_enabled(), state.active, galley.text()));
        if ui.is_rect_visible(rect) && !state.is_being_dragged {
            let fill = if !state.active && response.hovered() { context_menu_hover() }
                else { self.tab_bg_color(ui.visuals(), tiles, tile_id, state) };
            crate::app::shell::frame::paint_connected_tab(ui, rect, fill, state.active);
            if !state.active && self.inactive_dividers.contains(&tile_id) {
                ui.painter().vline(rect.right() - 0.5, (rect.top() + 5.0)..=(rect.bottom() - 5.0),
                    Stroke::new(1.0, grid_line()));
            }
            let inner = rect.shrink(margin);
            let text_pos = egui::Align2::LEFT_CENTER.align_size_within_rect(galley.size(), inner).min;
            ui.painter().galley(text_pos, galley, text_dark());
            if state.closable {
                let close_rect = egui::Align2::RIGHT_CENTER.align_size_within_rect(close_size, inner);
                if dirty {
                    ui.painter().circle_filled(
                        egui::pos2(close_rect.left() - CLOSE_GAP * 0.5, close_rect.center().y),
                        2.0, text_dark(),
                    );
                }
                let close = ui.interact(close_rect, id.with("close"), Sense::click_and_drag())
                    .on_hover_cursor(egui::CursorIcon::Default);
                close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close"));
                paint_close_control(ui, &close);
                if (close.clicked() || response.middle_clicked()) && self.on_tab_close(tiles, tile_id) {
                    tiles.remove(tile_id);
                }
            }
        }
        self.on_tab_button(tiles, tile_id, response)
    }

    fn close_button_outer_size(&self) -> f32 { 20.0 }

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

    /// Clicking a game tab focuses its kit, by the same rule as a press inside
    /// its pane. The tab bar sits outside every pane, so without this the
    /// click shows one game while the File menu still acts on another.
    fn on_tab_button(
        &mut self,
        tiles: &mut egui_tiles::Tiles<KitId>,
        tile_id: egui_tiles::TileId,
        button_response: egui::Response,
    ) -> egui::Response {
        if button_response.clicked()
            && let Some(egui_tiles::Tile::Pane(kit_id)) = tiles.get(tile_id)
        {
            self.cx.send(AppAction::FocusKit(*kit_id));
        }
        button_response
    }

    fn top_bar_right_ui(
        &mut self,
        _tiles: &egui_tiles::Tiles<KitId>,
        ui: &mut Ui,
        _tile_id: egui_tiles::TileId,
        _tabs: &egui_tiles::Tabs,
        scroll_offset: &mut f32,
    ) {
        let bar = ui.max_rect();
        ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, grid_line()));
        wheel_scroll_tab_bar(ui, scroll_offset);
    }

    /// Scroll with the kit tabs, immediately after the last one, like Home's
    /// icon-only tab rather than a toolbar button pinned to the window edge.
    fn tab_bar_trailing_ui(
        &mut self, tiles: &egui_tiles::Tiles<KitId>, ui: &mut Ui,
        group: egui_tiles::TileId, tabs: &egui_tiles::Tabs,
    ) {
        let (_, rect) = ui.allocate_space(Vec2::new(32.0, 26.0));
        let response = ui.interact(rect, egui::Id::new("workspace_add_tab").with(group), Sense::click())
            .on_hover_cursor(egui::CursorIcon::Default)
            .on_hover_text("Add workspace");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Add workspace"));
        let config = menu_config();
        let popup = egui::Popup::menu(&response)
            .close_behavior(config.close_behavior)
            .style(config.style.clone())
            .info(egui::UiStackInfo::new(egui::UiKind::Menu)
                .with_tag_value(egui::containers::menu::MenuConfig::MENU_CONFIG_TAG, config));
        let fill = if response.hovered() || popup.is_open() { context_menu_hover() }
            else { Color32::TRANSPARENT };
        crate::app::shell::frame::paint_connected_tab(ui, rect, fill, false);
        paint_button_icon_at(ui, ButtonIcon::AddWorkspace,
            egui::Rect::from_center_size(rect.center(), Vec2::splat(18.0)), text_dark());
        let home = self.parts.shell.home_groups.contains(&group);
        let hidden_empty_kit = self.cx.model.kits.len() == 1 && self.cx.model.kits[0].can_accept_source_load();
        if !hidden_empty_kit && tabs.children.iter().rev().find(|id| tiles.is_visible(**id))
            .is_some_and(|id| home || !tabs.is_active(*id))
        {
            ui.painter().vline(rect.left() - 0.5, (rect.top() + 5.0)..=(rect.bottom() - 5.0),
                Stroke::new(1.0, grid_line()));
        }
        let recents = &self.cx.model.prefs.recent_folders;
        let menu_margin = Frame::menu(ui.style()).total_margin();
        let recent_popup_width = 240.0 + menu_margin.left + menu_margin.right;
        let had_request = self.add_kit.is_some() || self.recent_action.is_some();
        popup.show(|ui| {
            style_list_menu(ui);
            ui.set_width(320.0);
            if ui.button("Load Folder...").clicked() {
                close_menu(ui);
                self.add_kit = Some(LoadKind::Folder);
            }
            if ui.button("Load Tag...").clicked() {
                close_menu(ui);
                self.add_kit = Some(LoadKind::SingleFile);
            }
            if ui.button("Load Monolithic blob_index.dat...").clicked() {
                close_menu(ui);
                self.add_kit = Some(LoadKind::Monolithic);
            }
            if ui
                .button("Open Campaign Evolved container (.utoc)...")
                .clicked()
            {
                close_menu(ui);
                self.add_kit = Some(LoadKind::Container);
            }
            ui.separator();
            if let Some(recent_action) =
                left_opening_menu_button(ui, "Recent", recent_popup_width, |ui| {
                    style_list_menu(ui);
                    draw_recent_folders_menu(ui, recents)
                })
                .flatten()
            {
                self.recent_action = Some(recent_action);
                close_menu(ui);
            }
        });
        if !had_request && (self.add_kit.is_some() || self.recent_action.is_some()) {
            self.load_target = tabs.active.and_then(|id| match tiles.get(id) {
                Some(egui_tiles::Tile::Pane(kit)) => Some(*kit),
                _ => None,
            });
        }
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

    fn gap_width(&self, _style: &egui::Style) -> f32 { 5.0 }

    fn resize_stroke(&self, _style: &egui::Style, state: egui_tiles::ResizeState) -> Stroke {
        match state {
            egui_tiles::ResizeState::Idle => Stroke::new(2.0, grid_line()),
            egui_tiles::ResizeState::Hovering => Stroke::new(2.0, subtle_dark()),
            egui_tiles::ResizeState::Dragging => Stroke::new(2.0, text_dark()),
        }
    }
    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> Color32 {
        row_type()
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<KitId>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        if state.active {
            left_panel()
        } else {
            Color32::TRANSPARENT
        }
    }

    fn tab_outline_stroke(
        &self, _visuals: &egui::Visuals, _tiles: &egui_tiles::Tiles<KitId>,
        _tile_id: egui_tiles::TileId, state: &egui_tiles::TabState,
    ) -> Stroke {
        if state.active { Stroke::new(1.0, grid_line()) } else { Stroke::NONE }
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

/// Whether `kit` is a Campaign Evolved install, which can show Chimp's
/// surface instead of its tags.
fn is_campaign_evolved(kit: &Kit) -> bool {
    kit.source
        .as_ref()
        .is_some_and(|source| matches!(&source.source, TagSource::IoStoreContainerSet { .. }))
}

/// Draw every open kit as a tiled workspace. Dragging a game tab against a
/// pane edge splits the window between two games.
pub(in crate::app) fn draw_kit_tiles(
    ui: &mut Ui,
    cx: &Ctx,
    parts: TileParts,
    inputs: &TileInputs,
    kit_tree: &mut egui_tiles::Tree<KitId>,
) {
    let mut behavior = KitPaneBehavior {
        cx,
        parts,
        inputs,
        close_requests: Vec::new(),
        add_kit: None,
        recent_action: None,
        load_target: None,
        tab_groups: HashMap::new(),
        primary_group: None,
        inactive_dividers: HashSet::new(),
    };
    ui.style_mut().interaction.resize_grab_radius_side = 7.0;
    kit_tree.ui(&mut behavior, ui);
    crate::app::shell::frame::paint_kit_split_grips(ui, kit_tree);
    behavior.parts.shell.home_groups.retain(|id| matches!(kit_tree.tiles.get(*id),
        Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(_))))
        && Some(*id) == behavior.primary_group);
    let active_kit = cx.model.kits[cx.model.active].id;
    behavior.parts.shell.home_open = kit_tree.tiles.iter().find_map(|(id, tile)| {
        matches!(tile, egui_tiles::Tile::Pane(kit) if *kit == active_kit)
            .then(|| behavior.tab_groups.get(id)).flatten()
    }).is_some_and(|group| behavior.parts.shell.home_groups.contains(group));
    let KitPaneBehavior {
        close_requests,
        add_kit,
        recent_action,
        load_target,
        ..
    } = behavior;

    for kit_id in close_requests {
        cx.send(AppAction::Close(PendingCloseAction::CloseKit(kit_id)));
    }
    if let Some(kit) = load_target { cx.send(AppAction::FocusKit(kit)); }
    if let Some(action) = recent_action {
        cx.send(AppAction::Recent(action));
    }
    if let Some(kind) = add_kit {
        cx.send(match kind {
            LoadKind::Folder => AppAction::LoadFolder,
            LoadKind::SingleFile => AppAction::LoadTag,
            LoadKind::Monolithic => AppAction::LoadMonolithic,
            LoadKind::Container => AppAction::LoadContainer,
        });
    }
}

impl Baboon {
    /// Draw the kits' tiles: fill the caches they read, then draw them from a
    /// [`Ctx`] with the kit tree moved off the application for the walk.
    pub(in crate::app) fn draw_workspace_tiles(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        self.sync_kit_tree();
        let inputs = self.prepare_tiles(ctx);
        let placeholder = egui_tiles::Tree::empty(egui::Id::new("kit_tree_placeholder"));
        let mut tree = std::mem::replace(&mut self.kit_tree, placeholder);
        draw_kit_tiles(ui, &cx!(self, ctx), tile_parts!(self), &inputs, &mut tree);
        self.kit_tree = tree;
    }

    /// Whether the kit at `kit_index` is showing Chimp's surface rather than
    /// its tags.
    pub(in crate::app) fn shows_chimp_surface(&self, kit_index: usize) -> bool {
        let kit = &self.model.kits[kit_index];
        is_campaign_evolved(kit)
            && self.model.prefs.enable_chimp
            && self.views[kit.id].surface == KitSurface::Chimp
    }

    /// Reconcile the layout tree with the kit list: add panes for kits opened
    /// since the last frame, drop panes for kits that have closed.
    ///
    /// The kit list is the content store and the tree only references it, so
    /// this only ever repairs the tree — it never creates or removes a kit.
    pub(in crate::app) fn sync_kit_tree(&mut self) {
        let live: Vec<KitId> = self.model.kits.iter().map(|kit| kit.id).collect();
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
            let active_kit = self.model.kits[self.model.active].id;
            let target = self.kit_tree.tiles.iter().find_map(|(id, tile)|
                matches!(tile, egui_tiles::Tile::Pane(kit) if *kit == active_kit).then_some(*id))
                .or_else(|| self.kit_tree.active_tiles().into_iter().find(|id|
                    matches!(self.kit_tree.tiles.get(*id), Some(egui_tiles::Tile::Pane(_)))));
            let tile_id = self.kit_tree.tiles.insert_pane(kit_id);
            match target {
                Some(target) => {
                    let parent = self.kit_tree.tiles.parent_of(target);
                    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) =
                        parent.and_then(|id| self.kit_tree.tiles.get_mut(id))
                    {
                        tabs.children.push(tile_id);
                    } else {
                        // Wrap a bare pane in place, preserving its split slot.
                        let tabs = self.kit_tree.tiles.insert_tab_tile(vec![target, tile_id]);
                        if let Some(egui_tiles::Tile::Container(container)) =
                            parent.and_then(|id| self.kit_tree.tiles.get_mut(id))
                        {
                            let _ = container.replace_child(target, tabs);
                        } else {
                            self.kit_tree.root = Some(tabs);
                        }
                    }
                }
                None => self.kit_tree.root = Some(tile_id),
            }
            self.kit_tree.make_active(|id, _| id == tile_id);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Which workspace is active decides where Ctrl+S, the save prompt and open
    //! pickers land, so it changes on a press inside a workspace and never because
    //! the cursor passed over one. Two workspaces side by side, driven headless.

    use super::*;

    #[test]
    fn regression_new_kits_join_the_last_active_split() {
        for wrapped in [false, true] {
            for focused in [0, 1] {
                let (mut app, kits) = tabbed_kits(2);
                app.kit_tree = egui_tiles::Tree::new_horizontal("kit_open_regression", kits.clone());
                let pane = |app: &Baboon, kit| app.kit_tree.tiles.iter().find_map(|(id, tile)|
                    matches!(tile, egui_tiles::Tile::Pane(open) if *open == kit).then_some(*id)).unwrap();
                let root = app.kit_tree.root().unwrap();
                if wrapped {
                    for kit in &kits {
                        let child = pane(&app, *kit);
                        let tabs = app.kit_tree.tiles.insert_tab_tile(vec![child]);
                        if let Some(egui_tiles::Tile::Container(container)) = app.kit_tree.tiles.get_mut(root) {
                            let _ = container.replace_child(child, tabs);
                        }
                    }
                }
                app.focus_kit(focused);
                let added = app.add_kit();
                app.sync_kit_tree();
                assert_eq!(app.kit_tree.root(), Some(root));
                let Some(egui_tiles::Tile::Container(container)) = app.kit_tree.tiles.get(root) else { panic!("split disappeared") };
                assert_eq!(container.children().count(), 2, "opening a kit must not add a third split");
                assert_eq!(app.kit_tree.tiles.parent_of(pane(&app, added)),
                    app.kit_tree.tiles.parent_of(pane(&app, kits[focused])),
                    "the new kit belongs to the last active pane's tab group");
                assert!(visible_kits(&app).contains(&added));
            }
        }
    }

    fn input(time: f64, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn a_workspace_activates_on_a_press_not_on_hover() {
        let mut app = Baboon::for_test();
        app.add_kit();
        assert_eq!(app.model.kits.len(), 2);
        let (left, right) = (app.model.kits[0].id, app.model.kits[1].id);
        app.kit_tree = egui_tiles::Tree::new_horizontal("kit_activation_test", vec![left, right]);
        app.model.active = 1;

        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut frame = |app: &mut Baboon, events: Vec<egui::Event>| {
            time += 0.1;
            let _ = crate::app::run_ui_test(&ctx, input(time, events), |ui| {
                let ctx = ui.ctx().clone();
                egui::CentralPanel::default().show(ui, |ui| app.draw_workspace_tiles(ui, &ctx));
                app.apply_commands(&ctx);
            });
        };
        let over_left = egui::pos2(200.0, 300.0);
        let over_right = egui::pos2(600.0, 300.0);
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };

        frame(&mut app, Vec::new());
        for _ in 0..3 {
            frame(&mut app, vec![egui::Event::PointerMoved(over_left)]);
        }
        assert_eq!(app.model.active, 1, "hovering the left workspace changes nothing");

        frame(&mut app, vec![press(over_left, true)]);
        frame(&mut app, vec![press(over_left, false)]);
        assert_eq!(app.model.active, 0, "a press inside the left workspace activates it");

        for _ in 0..3 {
            frame(&mut app, vec![egui::Event::PointerMoved(over_right)]);
        }
        assert_eq!(app.model.active, 0, "hovering the right workspace changes nothing");

        frame(&mut app, vec![press(over_right, true)]);
        frame(&mut app, vec![press(over_right, false)]);
        assert_eq!(app.model.active, 1);
        assert!(!app.shell.home_open, "an empty secondary split must not activate primary Home");
    }

    /// The kits whose workspaces are on screen.
    fn visible_kits(app: &Baboon) -> Vec<KitId> {
        app.kit_tree
            .active_tiles()
            .into_iter()
            .filter_map(|id| match app.kit_tree.tiles.get(id) {
                Some(egui_tiles::Tile::Pane(kit)) => Some(*kit),
                _ => None,
            })
            .collect()
    }

    /// `count` kits as tabs in one group, laid out the way loading them does:
    /// the last one added is in front and active.
    fn tabbed_kits(count: usize) -> (Baboon, Vec<KitId>) {
        let mut app = Baboon::for_test();
        for _ in 1..count {
            app.add_kit();
        }
        app.sync_kit_tree();
        let kits: Vec<KitId> = app.model.kits.iter().map(|kit| kit.id).collect();
        assert_eq!(visible_kits(&app), vec![kits[count - 1]]);
        assert_eq!(app.model.active, count - 1);
        (app, kits)
    }

    #[test]
    fn clicking_a_workspace_tab_activates_its_kit() {
        let (mut app, kits) = tabbed_kits(2);
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut frame = |app: &mut Baboon, events: Vec<egui::Event>| {
            time += 0.1;
            let _ = crate::app::run_ui_test(&ctx, input(time, events), |ui| {
                let ctx = ui.ctx().clone();
                egui::CentralPanel::default().show(ui, |ui| app.draw_workspace_tiles(ui, &ctx));
                app.apply_commands(&ctx);
            });
        };
        // On the first tab's label, in the tab bar above both panes.
        let on_first_tab = egui::pos2(90.0, 20.0);
        let press = |pressed| egui::Event::PointerButton {
            pos: on_first_tab,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };

        frame(&mut app, Vec::new());
        for _ in 0..3 {
            frame(&mut app, vec![egui::Event::PointerMoved(on_first_tab)]);
        }
        frame(&mut app, vec![press(true)]);
        frame(&mut app, vec![press(false)]);
        frame(&mut app, Vec::new());

        assert_eq!(visible_kits(&app), vec![kits[0]], "the click shows the first workspace");
        assert_eq!(app.model.active, 0, "and the menus act on the workspace being shown");
    }

    #[test]
    fn a_restored_session_shows_the_kit_it_focuses() {
        let (mut app, kits) = tabbed_kits(2);
        app.shell.restoring_kits.insert(kits[0]);
        app.shell.restored_active_kit = Some(kits[0]);

        app.settle_restored_kit(kits[0]);

        assert_eq!(app.model.active, 0);
        assert_eq!(visible_kits(&app), vec![kits[0]]);
    }

    #[test]
    fn reopening_an_open_source_shows_its_kit() {
        let (mut app, kits) = tabbed_kits(2);
        let path = PathBuf::from("/kits/halo3/tags");
        app.model.kits[0].requested_path = Some(path.clone());

        assert!(app.open_kit_for(&path), "the source is already open");

        assert_eq!(app.model.active, 0);
        assert_eq!(visible_kits(&app), vec![kits[0]]);
    }

    #[test]
    fn closing_a_workspace_shows_the_kit_that_becomes_active() {
        let (mut app, kits) = tabbed_kits(3);
        app.focus_kit(1);
        assert_eq!(visible_kits(&app), vec![kits[1]]);

        app.remove_kit(kits[1]);
        app.sync_kit_tree();

        let active = app.model.kits[app.model.active].id;
        assert_eq!(active, kits[2], "the kit that slid into the closed one's place");
        assert_eq!(visible_kits(&app), vec![active]);
    }

    #[test]
    fn closing_a_kit_preserves_the_home_group() {
        for home in [false, true] {
            let (mut app, kits) = tabbed_kits(2);
            let group = app.kit_tree.tiles.iter().find_map(|(id, tile)|
                matches!(tile, egui_tiles::Tile::Container(egui_tiles::Container::Tabs(_))).then_some(*id)).unwrap();
            if home { app.shell.home_groups.insert(group); }
            app.shell.home_open = home;
            app.remove_kit(kits[1]);
            app.sync_kit_tree();
            assert_eq!(app.shell.home_open, home);
            assert_eq!(app.shell.home_groups.contains(&group), home);
        }
    }
}
