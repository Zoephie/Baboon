//! Tiled layout of loaded kits: the toplevel game tabs and their splits.
//! It owns which kit workspaces are visible and how they are arranged; one workspace's contents belong to the browser and tag-tile modules.

use super::recents::{RecentAction, draw_recent_folders_menu};
use super::*;
use crate::app::shell::frame::tint_toward;
use crate::app::shell::frame::wheel_scroll_tab_bar;

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
}

impl egui_tiles::Behavior<KitId> for KitPaneBehavior<'_, '_, '_> {
    fn pane_ui(
        &mut self,
        ui: &mut Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut KitId,
    ) -> egui_tiles::UiResponse {
        let kit_id = *pane;
        let cx = self.cx;
        let Some(kit_index) = cx.model.kit_index(kit_id) else {
            ui.label(RichText::new("This kit is no longer open").color(subtle_dark()));
            return egui_tiles::UiResponse::None;
        };
        // Activate on a press inside the pane, not on hover. `active` decides
        // where deferred work lands — the save prompt, Ctrl+S, an open colour
        // picker or reference picker — and none of that should retarget just
        // because the cursor crossed another game's pane on its way somewhere.
        // Sent before the pane draws, so it lands before anything the pane
        // sends.
        if ui.input(|input| input.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect())
        {
            cx.send(AppAction::FocusKit(kit_id));
        }
        let parts = &mut self.parts;
        let kit = &cx.model.kits[kit_index];
        // An unloaded workspace has no tags to browse or edit, so it offers
        // ways to open one instead of an empty browser and an empty editor.
        if kit.is_empty_workspace() {
            draw_welcome_screen(
                cx,
                ui,
                kit_index,
                parts.shell,
                &parts.kit_tools.editing_kit_validation,
            );
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
        egui::Panel::left(egui::Id::new(("kit_browser_panel", kit_id.0)))
            .resizable(true)
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
                    parts.audio.language.as_deref(),
                );
            });
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(editor_bg()))
            .show(ui, |ui| {
                draw_tag_tiles(ui, cx, parts, self.inputs, kit_index);
            });
        egui_tiles::UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &KitId) -> egui::WidgetText {
        let Some(index) = self.cx.model.kit_index(*pane) else {
            return RichText::new("(closed)").color(subtle_dark()).into();
        };
        let kit = &self.cx.model.kits[index];
        let dirty = kit.has_unwritten_modifications();
        let label = kit_strip_label(kit);
        let text = if dirty { format!("• {label}") } else { label };
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
        scroll_offset: &mut f32,
    ) {
        wheel_scroll_tab_bar(ui, scroll_offset);
        let recents = &self.cx.model.prefs.recent_folders;
        let menu_margin = Frame::menu(ui.style()).total_margin();
        let root_popup_width = 320.0 + menu_margin.left + menu_margin.right;
        let recent_popup_width = 240.0 + menu_margin.left + menu_margin.right;
        right_aligned_menu_button(ui, "+", root_popup_width, |ui| {
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
        let base = if state.active {
            active_tab()
        } else {
            left_panel()
        };
        let dirty = matches!(tiles.get(tile_id), Some(egui_tiles::Tile::Pane(kit_id))
            if self.cx.model.kit_index(*kit_id)
                .is_some_and(|index| self.cx.model.kits[index].has_unwritten_modifications()));
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
    mut parts: TileParts,
    inputs: &TileInputs,
    kit_tree: &mut egui_tiles::Tree<KitId>,
) {
    // Nothing to tab between: draw the welcome screen directly rather than
    // wrapping it in a tree whose tab bar would be an empty strip. A
    // zero-height tab bar still leaves the "+" and the bar's own painting
    // behind, so the tree is skipped outright.
    if cx.model.kits.len() == 1 && cx.model.kits[0].is_empty_workspace() {
        draw_welcome_screen(
            cx,
            ui,
            0,
            &mut parts.shell,
            &parts.kit_tools.editing_kit_validation,
        );
        return;
    }
    let mut behavior = KitPaneBehavior {
        cx,
        parts,
        inputs,
        close_requests: Vec::new(),
        add_kit: None,
        recent_action: None,
    };
    kit_tree.ui(&mut behavior, ui);
    let KitPaneBehavior {
        close_requests,
        add_kit,
        recent_action,
        ..
    } = behavior;

    for kit_id in close_requests {
        cx.send(AppAction::Close(PendingCloseAction::CloseKit(kit_id)));
    }
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
    fn sync_kit_tree(&mut self) {
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

#[cfg(test)]
mod kit_activation_tests;
