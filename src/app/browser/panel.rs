//! The tag browser for one kit: source header, search, and the folder/group tree.
//! It owns browser presentation and request collection for a single kit; layout of the panels belongs to the shell.

use super::*;
use crate::app::shell::frame::pane_header_breadcrumbs;
use crate::app::shell::frame::PANE_HEADER_ICON_TEXT_GAP;
use crate::app::shell::frame::PANE_HEADER_ICON_SIZE;
use crate::app::shell::frame::pane_header_inline_left_width;
use crate::app::shell::frame::PANE_HEADER_SECTION_GAP;
use crate::app::shell::frame::PANE_HEADER_COMMON_ACTIONS_WIDTH;
use crate::app::shell::frame::pane_header_path_parts;
use crate::app::editor::view_tab_button;
use crate::app::shell::AppAction;
use crate::app::shell::frame::{PANE_HEADER_ACTION_GAP, PANE_HEADER_BOTTOM_SPACE};
use crate::app::shell::frame::browser_favorites_divider;
use crate::app::shell::frame::draw_game_banner_header;
use crate::app::shell::frame::sidebar_source_path_label;
use crate::app::shell::frame::navigate_folder_browser;
use crate::app::shell::frame::browser_search_field;
use crate::app::shell::frame::is_file_cached;

const FOLDER_HEADER_ACTIONS_SINGLE_ROW_BREAKPOINT: f32 = 900.0;
const FOLDER_HEADER_LAUNCHER_WIDTH: f32 = 190.0;
const FOLDER_BROWSER_SEARCH_STACK_BREAKPOINT: f32 = 600.0;

/// A full-width, borderless navigation row like the entries in a menu/list.
/// It stays transparent at rest and uses only a soft fill for hover/press.
fn sidebar_list_button(ui: &mut Ui, icon: ButtonIcon, label: &str) -> egui::Response {
    sidebar_shortcut_button(ui, label, |ui, rect| paint_button_icon_at(ui, icon, rect, text_dark()))
}

fn sidebar_app_button(ui: &mut Ui, texture: Option<&egui::TextureHandle>, label: &str,
    fallback: &str, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        sidebar_shortcut_button(ui, label, |ui, rect| {
            if let Some(texture) = texture {
                ui.painter().image(texture.id(), rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, fallback,
                    TextStyle::Button.resolve(ui.style()), text_dark());
            }
        })
    }).inner
}

fn sidebar_shortcut_button(ui: &mut Ui, label: &str,
    paint_icon: impl FnOnce(&Ui, egui::Rect)) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), BUTTON_HEIGHT),
        Sense::click_and_drag(),
    );
    if response.hovered() || response.is_pointer_button_down_on() {
        let fill = if response.is_pointer_button_down_on() {
            ui.visuals().widgets.active.weak_bg_fill
        } else {
            context_menu_hover()
        };
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.hovered.corner_radius, fill);
    }
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + ui.spacing().indent + ui.spacing().item_spacing.x + BUTTON_ICON_SIZE * 0.5, rect.center().y),
        Vec2::splat(BUTTON_ICON_SIZE),
    );
    paint_icon(ui, icon_rect);
    ui.painter().text(
        egui::pos2(icon_rect.right() + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        TextStyle::Button.resolve(ui.style()),
        text_dark(),
    );
    response
}

fn sidebar_kit_app_button(ui: &mut Ui, label: &str, executable: Option<&Path>, uri: &'static str,
    bytes: &'static [u8], enabled: bool) -> egui::Response {
    let texture = executable.and_then(|path| crate::app::shell::shortcuts::app_icon(ui.ctx(), path));
    ui.add_enabled_ui(enabled, |ui| {
        sidebar_shortcut_button(ui, label, |ui, rect| {
            if let Some(texture) = texture {
                ui.painter().image(texture.id(), rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                egui::Image::from_bytes(uri, bytes).fit_to_exact_size(rect.size()).paint_at(ui, rect);
            }
        })
    }).inner
}

#[derive(Clone, Copy)]
enum KitShortcut { GitReview, TagTest, Sapien }

fn folder_browser_search_stacks(available_width: f32) -> bool {
    available_width < FOLDER_BROWSER_SEARCH_STACK_BREAKPOINT
}

impl Baboon {
    /// Bring a docked folder pane up to date with its kit before it draws:
    /// the modified and deletable sets its rows mark, and its own tree, which
    /// for a loose folder loads the folder's tags into the source. Its Asset
    /// Browser draws from the bitmap and model libraries, refreshed with it.
    pub(in crate::app) fn refresh_folder_browser_pane(
        &mut self,
        kit_index: usize,
        pane_key: &str,
        ctx: &egui::Context,
    ) {
        let assets_view = self.views[self.model.kits[kit_index].id]
            .browser
            .folder_browsers
            .get(pane_key)
            .is_some_and(|pane| pane.assets_view);
        if assets_view {
            self.refresh_thumbnail_library::<Bitmaps>(kit_index, ctx);
            self.refresh_thumbnail_library::<Models>(kit_index, ctx);
        }
        self.refresh_modified_tags(kit_index);
        self.refresh_deletable_keys(kit_index);
        let mut kit_status = None;
        let kit = &mut self.model.kits[kit_index];
        let Some(pane) = self.views[kit.id].browser.folder_browsers.get_mut(pane_key) else {
            return;
        };
        // A loose pane owns a lazy subtree whose indices address the shared
        // lazy entry vector. Its growth must not rebuild/collapse the pane.
        // Eager sources still use entry count as cache invalidation.
        let source_len = kit
            .source
            .as_ref()
            .map(|source| match source.source {
                TagSource::LooseFolder { .. } => 0,
                _ => source.full_entry_set().len(),
            })
            .unwrap_or(0);
        let generation = kit.generation;
        if pane.cached_generation != generation || pane.cached_source_len != source_len {
            if let Some(source) = kit.source.as_mut() {
                if let TagSource::LooseFolder { root, .. } = &source.source {
                    let root = root.clone();
                    let names = source.names.clone();
                    match crate::core::source::build_lazy_folder_tree_beneath(
                        &root,
                        &pane.rel_path,
                        &mut source.entries,
                        &names,
                    ) {
                        Ok(tree) => pane.tree = tree,
                        Err(error) => {
                            pane.tree = TagTree::default();
                            kit_status = Some(format!("Could not load folder tab: {error}"));
                        }
                    }
                    pane.group_tree = TagTree::default();
                } else {
                    let entries = source.full_entry_set();
                    pane.tree = crate::core::source::build_tree_beneath(entries, &pane.rel_path);
                    pane.group_tree =
                        crate::core::source::build_group_tree_beneath(entries, &pane.rel_path);
                }
            } else {
                pane.tree = TagTree::default();
                pane.group_tree = TagTree::default();
            }
            pane.filter_cache = FilterCache::default();
            pane.cached_generation = generation;
            pane.cached_source_len = source_len;
        }
        if let Some(status) = kit_status {
            self.model.status = status;
        }
    }
}

/// Draw a folder as a first-class docked pane beside ordinary tag panes. It
/// is refreshed beforehand by [`Baboon::refresh_folder_browser_pane`].
pub(in crate::app) fn draw_folder_browser_pane(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    pane_key: &str,
    view: &mut KitView,
    sound_language: Option<&str>,
) {
    let kit_id = cx.model.kits[kit_index].id;
    let Some(mut pane) = view.browser.folder_browsers.remove(pane_key) else {
        ui.label(RichText::new("This folder is no longer open").color(subtle_dark()));
        return;
    };
    // The whole pane, for saying why a drag can't land here.
    let pane_rect = ui.max_rect();
    let generation = cx.model.kits[kit_index].generation;
    let before_view = (pane.mode, pane.assets_view, pane.filter.clone(), pane.search_scope, pane.asset_bitmaps, pane.asset_models);
    let before_sort = (pane.sort, cx.model.prefs.folders_before_tags);
    ui.data_mut(|data|data.insert_temp(browser_selection_owner_id().with("drawing"),(kit_id,pane_key.to_owned())));
    set_browser_selected_tag(ui, pane.selected_tag.clone());
    begin_browser_navigation(ui, egui::Id::unique(("folder_navigation", kit_id, pane_key)));
    let bitmap_hover_requests = begin_bitmap_hovers(ui, Arc::clone(&view.bitmap_browser.thumbnails));

    let is_loose = cx.model.kits[kit_index]
        .source
        .as_ref()
        .is_some_and(|source| matches!(source.source, TagSource::LooseFolder { .. }));
    let is_container = cx.model.kits[kit_index]
        .source
        .as_ref()
        .is_some_and(|source| matches!(source.source, TagSource::IoStoreContainerSet { .. }));
    let favorite_keys: HashSet<String> = cx.model.kits[kit_index].active_favorite_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect();
    let pane_favorite_folders =
        std::sync::Arc::new(cx.model.kits[kit_index].active_favorite_folders.clone());
    let selected = current_browser_item(&cx.model.kits[kit_index], view);
    let modified_tags = std::sync::Arc::clone(&view.browser.modified_tags);
    let deletable_keys = std::sync::Arc::clone(&view.browser.deletable_keys);
    let game = cx.model.kits[kit_index].source.as_ref().and_then(|source| source.game);
    let sound_tags_root = cx.model.kits[kit_index].source.as_ref().and_then(|source| {
        if let TagSource::LooseFolder { root, .. } = &source.source {
            Some(root.clone())
        } else {
            None
        }
    });
    let scenario_launch = cx.model.kits[kit_index]
        .source
        .as_ref()
        .map(|source| {
            crate::app::kits::scenario_launch::scenario_launch_availability_with(source, |path| {
                is_file_cached(ui.ctx(), path)
            })
        })
        .unwrap_or_default();
    let mut show_browser_prefixes = cx.model.prefs.show_browser_prefixes;
    let mut folders_before_tags = cx.model.prefs.folders_before_tags;
    let double_click_to_open = cx.model.prefs.double_click_to_open_tags;
    let search_hint = pane.search_scope.hint();
    let previous_search_scope = pane.search_scope;
    let mut action = None;
    let mut need_scan = false;
    let mut load_requests = Vec::new();
    let scanning = cx.model.kits[kit_index].scanning_entries;
    let table_keywords = cx.model.kits[kit_index].keywords.snapshot();
    let source = cx.model.kits[kit_index].source.as_ref();
    let (bitmap_library, model_library) = (&mut view.bitmap_browser, &mut view.model_browser);

    Frame::NONE
        .inner_margin(pane_content_margin())
        .show(ui, |ui| {
            set_browser_modified_tags(ui, modified_tags);
            set_browser_favorite_folders(ui, is_loose.then_some(pane_favorite_folders));
            set_browser_deletable_keys(ui, deletable_keys);
            set_browser_sound_language(ui, game, sound_language);
            set_browser_sound_available_languages(
                ui,
                game,
                sound_tags_root.as_deref(),
            );
            set_browser_entries_scanning(ui, scanning);
            set_browser_loose_source(ui, is_loose);
            set_browser_game(ui, game);
            set_browser_scenario_launch(ui, scenario_launch);
            set_browser_is_folder_pane(ui, true);

            let Some(source) = source else {
                ui.label(
                    RichText::new("This folder source is no longer loaded")
                        .color(subtle_dark()),
                );
                return;
            };

            set_operation_entries(
                ui, kit_id, generation, source.full_entry_set(),
                cx.model.editing_kit_is_read_only(kit_index), !is_loose || source.complete_scan,
            );
            let header_entries = if is_loose {
                &source.entries[..]
            } else {
                source.full_entry_set()
            };
            draw_folder_pane_header(
                ui,
                &mut pane,
                header_entries,
                is_loose,
                is_container,
                &mut action,
            );
            ui.add_space(PANE_HEADER_BOTTOM_SPACE);
            ui.separator();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if view_tab_button(ui, if pane.mode==BrowserMode::Flat {ButtonIcon::FolderClosed}else{ButtonIcon::FolderOpen}, "Folder View", !pane.assets_view)
                    .clicked()
                {
                    pane.assets_view = false;
                }
                if view_tab_button(ui, ButtonIcon::AssetBrowser, "Asset Browser", pane.assets_view)
                    .clicked()
                {
                    pane.assets_view = true;
                }
            });
            ui.add_space(6.0);
            egui::Frame::NONE.stroke(Stroke::new(1.0, foundation_block_edge()))
                .corner_radius(crate::app::editor::fields::FOUNDATION_CONTAINER_RADIUS).show(ui, |ui| {
            let header_background = ui.painter().add(egui::Shape::Noop);
            let header = egui::Frame::NONE.inner_margin(6)
                .show(ui, |ui| {
            if folder_browser_search_stacks(ui.available_width()) {
                let search_response = browser_search_field(ui, &mut pane.filter, &search_hint);
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    draw_folder_browser_controls(
                        ui,
                        &mut pane,
                        &mut show_browser_prefixes,
                        &mut folders_before_tags,
                        (is_loose || is_container) && !cx.model.editing_kit_is_read_only(kit_index),
                        &mut action,
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
                        (is_loose || is_container) && !cx.model.editing_kit_is_read_only(kit_index),
                        &mut action,
                    );
                    ui.add_space(6.0);
                    browser_search_field(ui, &mut pane.filter, &search_hint)
                })
                .inner
            }
            });
            let search_response = header.inner;
            paint_frame_band(ui, header_background, header.response.rect,
                egui::CornerRadius::same(crate::app::editor::fields::FOUNDATION_CONTAINER_RADIUS as u8),
                Stroke::new(1.0, foundation_block_edge()), foundation_block_bar(), true);
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
            if before_sort != (pane.sort, folders_before_tags) { invalidate_browser_navigation(ui.ctx()); }
            if before_view != (pane.mode, pane.assets_view, pane.filter.clone(), pane.search_scope, pane.asset_bitmaps, pane.asset_models) { invalidate_browser_navigation(ui.ctx()); pane.selection.clear(); pane.selected_tag = None; set_browser_selected_tag(ui,None); }
            let body_height = (ui.available_height() - BUTTON_HEIGHT - 12.0 - ui.spacing().item_spacing.y).max(0.0);
            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(),body_height), egui::Layout::top_down(egui::Align::Min), |ui| {
            egui::Frame::NONE.inner_margin(6).show(ui, |ui| {
            ui.set_min_height((ui.available_height() - 12.0).max(0.0));
            let search_root = match &source.source {
                TagSource::LooseFolder { root, .. } => Some(root.as_path()),
                _ => None,
            };

            if pane.assets_view {
                if is_loose && !source.complete_scan {
                    need_scan = !scanning;
                    ui.label(
                        RichText::new("Indexing assets in this folder…").color(subtle_dark()),
                    );
                    return;
                }
                pane.filter_cache.refresh_scoped_async(
                    generation,
                    &pane.filter,
                    source.full_entry_set(),
                    false,
                    &pane.rel_path,
                    None,
                    pane.search_scope,
                    &table_keywords,
                    ui.ctx(),
                );
                if pane.filter_cache.waiting_for_results() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Searching assets…").color(subtle_dark()));
                    });
                    return;
                }
                begin_multi(ui,pane_key,kit_id,&pane.selection,true);
                draw_folder_asset_grid(
                    cx,
                    ui,
                    kit_index,
                    pane_key,
                    &pane,
                    &pane.filter_cache.entries,
                    pane.filter_cache.signature(),
                    bitmap_library,
                    model_library,
                );
                if let Some(state) = end_multi(ui) { pane.selection = state.selection; }
                return;
            }

            // The folder view is a table: name, then columns the user can
            // resize, reorder, sort by, and hide.
            ScrollArea::horizontal()
                .id_salt(("folder_columns", pane_key))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(pane.table_layout.width());
                    begin_folder_table(
                        ui,
                        search_root.map(Path::to_path_buf),
                        Arc::clone(&table_keywords),
                        pane.date_cache.clone(),
                        &mut pane.table_layout,
                    );
                    if pane.mode == BrowserMode::Flat {
                        flat::draw(ui, &mut pane, pane_key, kit_id, source, generation, folders_before_tags, double_click_to_open, &favorite_keys, &table_keywords, &mut need_scan, &mut load_requests, &mut action);
                    } else { draw_folder_pane_tree(
                        ui,
                        &mut pane,
                        pane_key,
                        source,
                        FolderPaneTree {
                            generation,
                            is_loose,
                            is_container,
                            scanning,
                            selected: selected.as_deref(),
                            show_prefixes: show_browser_prefixes,
                            folders_before_tags,
                            double_click_to_open,
                            favorite_keys: &favorite_keys,
                            search_root,
                            keywords: &table_keywords,
                        },
                        &mut need_scan,
                        &mut load_requests,
                        &mut action,
                    );
                    }
                    end_folder_table(ui);
                });
            });
            });
            draw_browser_footer(cx, ui, &pane, pane_key, source, generation);
            });
        });

    finish_browser_navigation(ui);
    pane.selected_tag = browser_selected_tag(ui);


    queue_bitmap_hover_thumbnails(cx, kit_index, &mut view.bitmap_browser, &bitmap_hover_requests);
    edit_browser_prefs(cx, show_browser_prefixes, folders_before_tags);
    if pane.search_scope != previous_search_scope {
        // The last choice seeds new browsers, in this session and the next.
        let scope = pane.search_scope;
        cx.edit_prefs(move |prefs| prefs.browser_search_scope = scope);
    }

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
    // A Tree view folder row that items were dropped on.
    if action.is_none() {
        action = super::drag_move::take_tree_drop(ui.ctx());
    }
    // Flat view takes dropped items anywhere, Tree view on its folder rows;
    // anywhere else in this tab says why. Its tab header always takes them.
    let refusal = if pane.assets_view {
        Some(super::drag_move::DROP_IN_FLAT_VIEW)
    } else {
        match pane.mode {
            BrowserMode::Flat => None,
            BrowserMode::Folders => Some(super::drag_move::DROP_ONTO_A_FOLDER),
            BrowserMode::Groups => Some(super::drag_move::SWITCH_TO_FLAT_VIEW),
        }
    };
    if let Some(reason) = refusal {
        super::drag_move::refuse_drop(ui.ctx(), ui.rect_contains_pointer(pane_rect), reason);
    }
    view.browser.folder_browsers.insert(pane_key.to_owned(), pane);
    if !load_requests.is_empty() {
        cx.send(BrowserCommand::LoadFolders {
            kit: kit_id,
            tree: LazyTree::Pane(pane_key.to_owned()),
            paths: load_requests,
        });
    }
    if need_scan {
        cx.send(BrowserCommand::ScanAllEntries { kit: kit_id });
    }
    if let Some(action) = action {
        cx.send(BrowserCommand::Action { kit: kit_id, action });
    }
}

/// Selection belongs to the browser container, including inactive split panes.
fn draw_browser_footer(cx: &Ctx, ui: &mut Ui, pane: &FolderBrowserState, pane_key: &str, source: &LoadedSourceData, generation: u64) {
    let multi = pane.mode == BrowserMode::Flat || pane.assets_view;
    let (tags, folders) = if multi {
        let folders = pane.selection.keys.iter().filter(|key| is_folder_pane_key(key)).count();
        (pane.selection.keys.len() - folders, folders)
    } else {
        match browser_selected_tag(ui) {
            Some(key) if is_folder_pane_key(&key) => (0, 1), Some(_) => (1, 0), None => (0, 0),
        }
    };
    let total = if pane.assets_view {
        thumbnail_library::asset_grid_order(ui, &format!("footer:{:?}:{pane_key}", ui.scope_id()), pane, source.full_entry_set(), Some(generation)).len()
    } else {
        // Listing totals are independent of search and selection. Cache the full
        // entry walk so large editing kits pay for it only when contents change.
        let entries = source.full_entry_set();
        let signature = (generation, entries.as_ptr() as usize, entries.len(), pane.rel_path.clone(), flat::immediate_folders(pane, source).len());
        let id = ui.make_persistent_id(("folder_item_total", pane_key));
        let cached = ui.data(|data|data.get_temp::<((u64,usize,usize,PathBuf,usize),usize)>(id));
        if let Some((_,count)) = cached.filter(|(key,_)| key == &signature) {count} else {
            let parent=tag_path::relative_tag_path(&pane.rel_path);
            let count=entries.iter().filter(|entry|tag_path::parent(&entry.display_path).unwrap_or("").eq_ignore_ascii_case(&parent)).count()+flat::immediate_folders(pane, source).len();
            ui.data_mut(|data|data.insert_temp(id,(signature,count)));count
        }
    };
    let shown = if multi {pane.selection.rows.len().min(total)} else {total};
    let noun = if total == 1 {"item"} else {"items"};
    let total_label = if shown == total {format!("{total} {noun}")} else {format!("{shown} of {total} {noun}")};
    let selection = match (tags,folders) {
        (0,0)=>"0 selected".into(),
        (tags,0)=>format!("{tags} {} selected", if tags==1 {"tag"} else {"tags"}),
        (0,folders)=>format!("{folders} {} selected", if folders==1 {"folder"} else {"folders"}),
        (tags,folders)=>format!("{tags} {} and {folders} {} selected",if tags==1 {"tag"}else{"tags"},if folders==1 {"folder"}else{"folders"}),
    };
    let background = ui.painter().add(egui::Shape::Noop);
    let footer = egui::Frame::NONE.inner_margin(6).show(ui,|ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.set_height(BUTTON_HEIGHT);
            if icon_button(ui, ButtonIcon::About, "About Folder Browser", true, text_dark()).clicked() {
                cx.send(crate::app::help::HelpCommand::ShowDocSection(crate::app::help::FOLDER_BROWSER_HELP_SECTION));
            }
            let mut counts=egui::text::LayoutJob::default();
            RichText::new(format!("{total_label} · ")).color(subtle_dark()).append_to(&mut counts,ui.style(),egui::FontSelection::Default,egui::Align::Center);
            RichText::new(&selection).color(text_dark()).append_to(&mut counts,ui.style(),egui::FontSelection::Default,egui::Align::Center);
            ui.add(egui::Label::new(counts).truncate()).on_hover_text(format!("{total_label} · {selection}"));
        });
    });
    paint_frame_band(ui,background,footer.response.rect,
        egui::CornerRadius::same(crate::app::editor::fields::FOUNDATION_CONTAINER_RADIUS as u8),
        Stroke::new(1.0,foundation_block_edge()),foundation_block_bar(),false);
}

/// What a folder pane's tree reads besides the pane itself.
struct FolderPaneTree<'a> {
    generation: u64,
    is_loose: bool,
    is_container: bool,
    scanning: bool,
    selected: Option<&'a str>,
    show_prefixes: bool,
    folders_before_tags: bool,
    double_click_to_open: bool,
    favorite_keys: &'a HashSet<String>,
    /// A loose folder's root, which a folder search walks.
    search_root: Option<&'a Path>,
    keywords: &'a Arc<std::collections::BTreeMap<String, Vec<String>>>,
}

/// A folder pane's tree, inside its table: the lazy folder tree when nothing
/// filters it, otherwise the pane's groups or its search results.
#[allow(clippy::too_many_arguments)]
fn draw_folder_pane_tree(
    ui: &mut Ui,
    pane: &mut FolderBrowserState,
    pane_key: &str,
    source: &LoadedSourceData,
    reads: FolderPaneTree<'_>,
    need_scan: &mut bool,
    load_requests: &mut Vec<PathBuf>,
    action: &mut Option<BrowserAction>,
) {
    let game = source.game;
    let filter = pane.filter.trim().to_owned();
    let groups_mode = pane.mode == BrowserMode::Groups;
    let needs_complete_index = reads.is_loose && (groups_mode || !filter.is_empty());
    if needs_complete_index && !source.complete_scan {
        *need_scan = !reads.scanning;
        ui.label(
            RichText::new(if reads.scanning {
                "Indexing tags…"
            } else {
                "Preparing tag index…"
            })
            .color(subtle_dark())
            .small(),
        );
        return;
    }
    if reads.is_loose && !groups_mode && filter.is_empty() {
        ScrollArea::vertical()
            .id_salt(("folder_pane", pane_key))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let tree_action = draw_tree_lazy(
                    ui,
                    &pane.tree,
                    &source.entries,
                    reads.selected,
                    "",
                    reads.show_prefixes,
                    reads.double_click_to_open,
                    load_requests,
                    None,
                    pane.sort,
                    reads.folders_before_tags,
                    Some(reads.favorite_keys),
                    game,
                );
                if action.is_none() {
                    *action = tree_action;
                }
            });
        return;
    }
    let entries = source.full_entry_set();
    // Rebuilt when the entries change, not every frame: it walks the whole
    // index and allocates per entry. A loose folder's full set only grows or
    // is replaced, and either moves the generation or the count.
    let built_for = (reads.generation, entries.len());
    if groups_mode && pane.group_tree_for != Some(built_for) {
        pane.group_tree = crate::core::source::build_group_tree_beneath(entries, &pane.rel_path);
        pane.group_tree_for = Some(built_for);
    }
    let (tree, visible_entries) = if filter.is_empty() {
        (
            if groups_mode {
                &pane.group_tree
            } else {
                &pane.tree
            },
            entries,
        )
    } else {
        pane.filter_cache.refresh_scoped_async(
            pane.cached_generation,
            &filter,
            entries,
            groups_mode,
            &pane.rel_path,
            reads.search_root,
            pane.search_scope,
            reads.keywords,
            ui.ctx(),
        );
        (
            &pane.filter_cache.tree,
            pane.filter_cache.entries.as_slice(),
        )
    };
    let searching = !filter.is_empty() && pane.filter_cache.waiting_for_results();
    if searching {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("Searching…").color(subtle_dark()));
        });
    }
    ScrollArea::vertical()
        .id_salt(("folder_pane", pane_key))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if visible_entries.is_empty() && tree.children.is_empty() {
                if !searching {
                    ui.label(RichText::new("No matching results").color(subtle_dark()));
                }
                return;
            }
            // Already filtered by `filter_cache`: drawn with folders open
            // rather than matched a second time.
            let tree_action = draw_tree(
                ui,
                tree,
                visible_entries,
                reads.selected,
                "",
                !filter.is_empty(),
                reads.show_prefixes,
                reads.double_click_to_open,
                groups_mode,
                None,
                pane.sort,
                !groups_mode && reads.folders_before_tags,
                reads.is_loose.then_some(reads.favorite_keys),
                reads.is_container,
                game,
            );
            if action.is_none() {
                *action = tree_action;
            }
        });
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
/// The modified and deletable sets are refreshed beforehand, and the
/// banner resolved, since both fill caches.
pub(in crate::app) fn draw_kit_browser(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    view: &mut KitView,
    browser: &mut BrowserFeature,
    banner: Option<egui::TextureHandle>,
    blender_icon: Option<&egui::TextureHandle>,
    sound_language: Option<&str>,
) {
    let salt = cx.model.kits[kit_index].id.0;
    let rect = ui.max_rect();
    let kit = cx.model.kits[kit_index].id;
    // Its folder rows take dropped Flat view items.
    super::drag_move::begin_navigation_drops(ui, kit);
    ui.push_id(salt, |ui| {
        draw_kit_browser_inner(
            cx,
            ui,
            kit_index,
            view,
            browser,
            banner,
            blender_icon,
            sound_language,
        )
    });
    super::drag_move::end_navigation_drops(ui);
    if let Some(action) = super::drag_move::take_tree_drop(ui.ctx()) {
        cx.send(BrowserCommand::Action { kit, action });
    }
    // Anywhere but a folder row says where to drop instead.
    let reason = if view.browser.mode == BrowserMode::Groups {
        super::drag_move::SWITCH_NAVIGATION_TO_FOLDERS
    } else {
        super::drag_move::DROP_ONTO_A_FOLDER
    };
    super::drag_move::refuse_drop(ui.ctx(), ui.rect_contains_pointer(rect), reason);
}

fn draw_kit_browser_inner(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    view: &mut KitView,
    browser: &mut BrowserFeature,
    banner: Option<egui::TextureHandle>,
    _blender_icon: Option<&egui::TextureHandle>,
    sound_language: Option<&str>,
) {
    let game = cx.model.kits[kit_index]
        .source
        .as_ref()
        .and_then(|source| source.game);
    // This kit's own source, not `source()` — that reads the *active* kit,
    // so in a split every browser drew the focused kit's banner and the
    // header flickered between games as the cursor moved between panes.
    let sidebar_header = cx.model.kits[kit_index].source.as_ref().map(|source| {
        (
            source.game,
            source.source.origin_label(),
            sidebar_source_path_label(&source.source),
            cx.model.kits[kit_index]
                .profile
                .as_ref()
                .map(|profile| profile.id.clone()),
        )
    });
    if let Some((Some(game), _origin, path_label, profile_id)) = sidebar_header.as_ref() {
        draw_game_banner_header(ui, cx.model, banner.as_ref(), *game, path_label, profile_id.as_deref());
    } else {
        ui.heading(RichText::new("Tags").color(text_dark()));
        if let Some((_, origin, _, _)) = sidebar_header.as_ref() {
            ui.small(RichText::new(origin).color(subtle_dark()));
            ui.add_space(8.0);
        }
    }

    let active_favorite_entries = cx.model.kits[kit_index].active_favorite_entries.clone();
    let active_favorite_folders =
        std::sync::Arc::new(cx.model.kits[kit_index].active_favorite_folders.clone());
    let favorite_keys: HashSet<String> = active_favorite_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect();
    let kit_id = cx.model.kits[kit_index].id;
    // Published into egui memory so the row and folder painters can reach
    // it without threading it through every drawing function. The browsers
    // draw one after another, so what is in memory during this tree's draw
    // is this kit's own set.
    set_browser_modified_tags(ui, std::sync::Arc::clone(&view.browser.modified_tags));
    set_browser_selected_tag(ui, view.browser.selected_tag.clone());
    begin_browser_navigation(ui, egui::Id::unique(("sidebar_navigation", kit_id)));
    set_browser_favorite_folders(
        ui,
        cx.model.kits[kit_index]
            .source
            .as_ref()
            .is_some_and(|source| matches!(source.source, TagSource::LooseFolder { .. }))
            .then_some(active_favorite_folders),
    );
    set_browser_game(
        ui,
        cx.model.kits[kit_index]
            .source
            .as_ref()
            .and_then(|source| source.game),
    );
    set_browser_sound_language(
        ui,
        cx.model.kits[kit_index]
            .source
            .as_ref()
            .and_then(|source| source.game),
        sound_language,
    );
    let browser_game = cx.model.kits[kit_index].source.as_ref().and_then(|source| source.game);
    let browser_tags_root = cx.model.kits[kit_index].source.as_ref().and_then(|source| {
        if let TagSource::LooseFolder { root, .. } = &source.source {
            Some(root.as_path())
        } else {
            None
        }
    });
    set_browser_sound_available_languages(ui, browser_game, browser_tags_root);
    set_browser_entries_scanning(ui, cx.model.kits[kit_index].scanning_entries);
    set_browser_loose_source(
        ui,
        cx.model.kits[kit_index]
            .source
            .as_ref()
            .is_some_and(|source| matches!(source.source, TagSource::LooseFolder { .. })),
    );
    set_browser_scenario_launch(
        ui,
        cx.model.kits[kit_index]
            .source
            .as_ref()
            .map(crate::app::kits::scenario_launch::scenario_launch_availability)
            .unwrap_or_default(),
    );
    set_browser_is_folder_pane(ui, false);
    set_browser_deletable_keys(ui, std::sync::Arc::clone(&view.browser.deletable_keys));
    let bitmap_hover_requests = begin_bitmap_hovers(ui, Arc::clone(&view.bitmap_browser.thumbnails));
    let mut shortcut = None;
    let mut custom_shortcut = None;
    let git_review_enabled = cx.model.git_review_enabled_for_kit(kit_index);
    let tool_root = cx.model.editing_kit_root_for(kit_index);
    let tag_test_ready = tool_root.as_ref().is_some_and(|root| {
        is_file_cached(ui.ctx(), &root.join(cx.model.tag_test_executable_for(kit_index)))
    });
    let sapien_ready =
        tool_root.as_ref().is_some_and(|root| is_file_cached(ui.ctx(), &root.join("sapien.exe")));
    let kit = &cx.model.kits[kit_index];
    let search_keywords = kit.keywords.snapshot();
    if let Some(source) = kit.source.as_ref() {
        set_operation_entries(
            ui, kit_id, kit.generation, source.full_entry_set(),
            cx.model.editing_kit_is_read_only(kit_index),
            !matches!(source.source, TagSource::LooseFolder { .. }) || source.complete_scan,
        );
        ui.add_space(8.0);
        let scanning = kit.scanning_entries;
        let mut need_scan = false;
        let mut show_prefixes = cx.model.prefs.show_browser_prefixes;
        let mut folders_before_tags = cx.model.prefs.folders_before_tags;
        let prev_filter_empty = view.browser.filter.is_empty();
        browser_search_field(ui, &mut view.browser.filter, &view.browser.search_scope.hint());
        if let Some(warning) = browser::browser_filter_warning(&view.browser.filter) {
            ui.label(
                RichText::new(warning)
                    .small()
                    .color(Color32::from_rgb(184, 134, 11)),
            );
        }
        ui.add_space(6.0);
        // Keep the controls on one row as the sidebar narrows. The scroll
        // viewport, rather than the toolbar's contents, determines its width.
        let previous_search_scope = view.browser.search_scope;
        ScrollArea::horizontal().id_salt("browser_toolbar_scroll")
            .auto_shrink([false, true]).show(ui, |ui| { ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let groups_clicked = browser_toolbar_controls(
                ui,
                &mut view.browser.mode,
                &mut view.browser.sort,
                &mut show_prefixes,
                &mut folders_before_tags,
                &mut view.browser.search_scope,
                None,
                true,
            );
            if groups_clicked
                && matches!(source.source, TagSource::LooseFolder { .. })
                && source.all_entries.is_empty()
                && !scanning
            {
                need_scan = true;
            }
        }); });
        if view.browser.search_scope != previous_search_scope {
            // The last choice seeds new browsers, in this session and the next.
            let scope = view.browser.search_scope;
            cx.edit_prefs(move |prefs| prefs.browser_search_scope = scope);
        }
        if prev_filter_empty
            && !view.browser.filter.is_empty()
            && matches!(source.source, TagSource::LooseFolder { .. })
            && source.all_entries.is_empty()
            && !scanning
        {
            need_scan = true;
        }
        ui.add_space(4.0);
        let selected = current_browser_item(kit, view);
        let filter = view.browser.filter.trim().to_owned();
        let mode = view.browser.mode;
        edit_browser_prefs(cx, show_prefixes, folders_before_tags);
        let double_click_to_open = cx.model.prefs.double_click_to_open_tags;
        let mut load_requests = Vec::new();
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
        //
        // Following the active tab is opt-in. An explicit reveal already
        // waiting always takes precedence over automatic navigation.
        if !cx.model.prefs.reveal_active_tab_in_browser {
            view.browser.followed_item = None;
        } else if selected != view.browser.followed_item && browser.reveal_target.is_none() {
            view.browser.followed_item = selected.clone();
            browser.reveal_target = selected.as_deref().and_then(|item| follow_reveal(kit_id, kit, item));
        }
        let reveal_owned = match &browser.reveal_target {
            Some(request) if request.kit == kit_id => browser.reveal_target.take(),
            _ => None,
        };
        set_reveal_align(
            ui.ctx(),
            reveal_owned.as_ref().map_or(Some(egui::Align::Center), |request| request.align),
        );
        let reveal = reveal_owned.as_ref().map(|request| Reveal {
            key: request.key.as_str(),
            remaining: request.ancestors.as_slice(),
            folder: request.folder,
        });
        let sort = view.browser.sort;
        let section_key = crate::app::shell::shortcuts::shortcut_kit_key(cx.model, kit_index);
        let saved_sections = cx.model.prefs.browser_section_states.get(&section_key).copied().unwrap_or_default();
        let mut sections = saved_sections;
        let action = ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| with_browser_section_state(ui, &mut sections, |ui| {
                let (favorite_action, _) = draw_favorites(
                    ui,
                    &active_favorite_entries,
                    // Favorites follow the visible folder browser's lazy
                    // boundary; a saved/global index must not make an
                    // unopened favorite look materialized.
                    &source.entries,
                    match &source.source {
                        TagSource::LooseFolder { root, .. } => Some(root.as_path()),
                        _ => None,
                    },
                    selected.as_deref(),
                    &filter,
                    show_prefixes,
                    double_click_to_open,
                    &favorite_keys,
                    view.browser.search_scope,
                    &search_keywords,
                    game,
                );
                let (_, add_shortcut) = show_browser_navigation_section_with_add(
                    ui,
                    "browser_shortcuts",
                    "Shortcuts",
                    ButtonIcon::Pin,
                    text_dark(),
                    true,
                    |ui| {
                        let shortcut_area = ui.scope_id();
                        let shortcut_kit = crate::app::shell::shortcuts::shortcut_kit_key(cx.model, kit_index);
                        use crate::app::shell::shortcuts::{shortcut_keys, shortcut_interactions};
                        for key in shortcut_keys(&cx.model.prefs, &shortcut_kit) {
                            ui.push_id(&key, |ui| {
                                let response = match key.as_str() {
                                    "builtin:git_review" if git_review_enabled => {
                                        let app = crate::app::shell::shortcuts::builtin_shortcut(&cx.model.prefs, &shortcut_kit, &key, tool_root.as_deref(), cx.model.tag_test_executable_for(kit_index));
                                        let response = sidebar_list_button(ui, ButtonIcon::Git, &app.name);
                                        if browser_shortcut_open_requested(&response, double_click_to_open) { shortcut = Some(KitShortcut::GitReview); }
                                        response
                                    }
                                    "builtin:tag_test" | "builtin:sapien" => {
                                        let tag_test = key == "builtin:tag_test";
                                        let (exe, uri, bytes, ready) = if tag_test {
                                            (cx.model.tag_test_executable_for(kit_index),
                                             "bytes://baboon_app_icons/tag-test.png", include_root_bytes!("assets/App Icons/Tag Test.png").as_slice(), tag_test_ready)
                                        } else {
                                            ("sapien.exe", "bytes://baboon_app_icons/sapien.png",
                                             include_root_bytes!("assets/App Icons/Sapien.png").as_slice(), sapien_ready)
                                        };
                                        let default_path = tool_root.as_ref().map(|root| root.join(exe)).unwrap_or_default();
                                        let app = crate::app::shell::shortcuts::builtin_shortcut(&cx.model.prefs, &shortcut_kit, &key, tool_root.as_deref(), cx.model.tag_test_executable_for(kit_index));
                                        let overridden = app.path != default_path;
                                        let ready = if overridden {
                                            crate::core::app_shortcut::validate_app_path(&app.path).is_ok()
                                        } else { ready };
                                        let response = sidebar_kit_app_button(ui, &app.name, Some(&app.path), uri, bytes, true)
                                            .on_hover_text(app.path.display().to_string());
                                        if browser_shortcut_open_requested(&response, double_click_to_open) && ready {
                                            if overridden {
                                                cx.send(AppAction::FocusKit(kit_id));
                                                cx.send(AppAction::LaunchShortcutApp(app));
                                            }
                                            else { shortcut = Some(if tag_test { KitShortcut::TagTest } else { KitShortcut::Sapien }); }
                                        }
                                        response
                                    }
                                    _ => {
                                        let Some((index, app)) = cx.model.prefs.app_shortcuts.iter().enumerate().find(|(_, app)| app.key() == key) else { return; };
                                        let texture = crate::app::shell::shortcuts::app_icon(ui.ctx(), &app.path);
                                        let response = sidebar_app_button(ui, texture.as_ref(), &app.name, "↗", true)
                                            .on_hover_text(app.path.display().to_string());
                                        if browser_shortcut_open_requested(&response, double_click_to_open) { custom_shortcut = Some(index); }
                                        response
                                    }
                                };
                                shortcut_interactions(cx, ui, &response, &key, shortcut_area, tool_root.as_deref(), kit_index);
                            });
                        }
                    },
                );
                if add_shortcut {
                    cx.open_dialog(crate::app::shell::shortcuts::AddShortcutDialog::for_kit(cx.model, kit_index));
                }
                let recent_action = draw_recents(ui, &view.browser.recent_entries, selected.as_deref(),
                    &filter, show_prefixes, double_click_to_open, &favorite_keys,
                    view.browser.search_scope, &search_keywords, game);
                browser_favorites_divider(ui, true);

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
                        view.browser.filter_cache.refresh_scoped_async(
                            kit.generation,
                            &filter,
                            entries,
                            groups_mode,
                            Path::new(""),
                            match &source.source {
                                TagSource::LooseFolder { root, .. } => Some(root.as_path()),
                                _ => None,
                            },
                            view.browser.search_scope,
                            &search_keywords,
                            ui.ctx(),
                        );
                        let cache = &view.browser.filter_cache;
                        if cache.waiting_for_results() {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Searching…").color(subtle_dark()));
                            });
                            None
                        } else if cache.entries.is_empty() && cache.tree.children.is_empty() {
                            ui.label(RichText::new("No matching results").color(subtle_dark()));
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
                                true,
                                show_prefixes,
                                double_click_to_open,
                                groups_mode,
                                reveal,
                                sort,
                                !groups_mode && folders_before_tags,
                                favorite_context,
                                is_container_source,
                                game,
                            )
                        }
                    }
                } else {
                    match mode {
                        BrowserMode::Flat | BrowserMode::Folders => {
                            if let TagSource::LooseFolder { .. } = &source.source {
                                draw_tree_lazy(
                                    ui,
                                    &source.tree,
                                    &source.entries,
                                    selected.as_deref(),
                                    &filter,
                                    show_prefixes,
                                    double_click_to_open,
                                    &mut load_requests,
                                    reveal,
                                    sort,
                                    folders_before_tags,
                                    favorite_context,
                                    game,
                                )
                            } else {
                                draw_tree(
                                    ui,
                                    &source.tree,
                                    &source.entries,
                                    selected.as_deref(),
                                    &filter,
                                    false,
                                    show_prefixes,
                                    double_click_to_open,
                                    false,
                                    reveal,
                                    sort,
                                    folders_before_tags,
                                    None,
                                    is_container_source,
                                    game,
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
                                    false,
                                    show_prefixes,
                                    double_click_to_open,
                                    true,
                                    reveal,
                                    sort,
                                    false,
                                    favorite_context,
                                    false,
                                    game,
                                )
                            }
                        }
                    }
                };
                favorite_action.or(recent_action).or(tree_action)
            }))
            .inner;
        if sections != saved_sections {
            cx.edit_prefs(move |prefs| { prefs.browser_section_states.insert(section_key, sections); });
        }
        if !load_requests.is_empty() {
            cx.send(BrowserCommand::LoadFolders {
                kit: kit_id,
                tree: LazyTree::Source,
                paths: load_requests,
            });
            // A reveal opens one unloaded folder a frame, each loading
            // before the next can open, so it stays until it gets there.
            if reveal_owned.is_some() && browser.reveal_target.is_none() {
                browser.reveal_target = reveal_owned;
            }
        }
        // Browser actions and the scan resolve against the active kit, and
        // this browser draws inside the workspace-tree walk, where `active`
        // is still whatever it was when the frame began; the commands name
        // this browser's own kit, which they make active first.
        if let Some(action) = action {
            cx.send(BrowserCommand::Action { kit: kit_id, action });
        }
        if need_scan {
            cx.send(BrowserCommand::ScanAllEntries { kit: kit_id });
        }
    } else {
        ui.label("Use File to load a tag, folder, or monolithic cache.");
    }
    if let Some(index) = custom_shortcut {
        cx.send(AppAction::FocusKit(kit_id));
        cx.send(AppAction::LaunchAppShortcut(index));
    }
    match shortcut {
        Some(KitShortcut::GitReview) => cx.send(CompareCommand::OpenGitReview { kit: kit_id }),
        // The tools launch for the focused kit: this browser's, first.
        Some(KitShortcut::TagTest) => {
            cx.send(AppAction::FocusKit(kit_id));
            cx.send(AppAction::LaunchTagTest);
        }
        Some(KitShortcut::Sapien) => {
            cx.send(AppAction::FocusKit(kit_id));
            cx.send(AppAction::LaunchSapien);
        }
        None => {}
    }
    finish_browser_navigation(ui);
    view.browser.selected_tag = browser_selected_tag(ui);
    queue_bitmap_hover_thumbnails(cx, kit_index, &mut view.bitmap_browser, &bitmap_hover_requests);
}

/// Draw the controls shared by the sidebar and docked folder browser. Callers
/// choose the surrounding layout and may place search beside or above them.
/// Send the two browser toggles both browsers' toolbars bind to, when the
/// draw changed them.
fn edit_browser_prefs(cx: &Ctx, show_prefixes: bool, folders_before_tags: bool) {
    if show_prefixes != cx.model.prefs.show_browser_prefixes {
        cx.edit_prefs(move |prefs| prefs.show_browser_prefixes = show_prefixes);
    }
    if folders_before_tags != cx.model.prefs.folders_before_tags {
        cx.edit_prefs(move |prefs| prefs.folders_before_tags = folders_before_tags);
    }
}

fn style_browser_toolbar(ui: &mut Ui) {
    ui.visuals_mut().widgets.inactive.bg_fill = browser_toolbar_bg();
    ui.visuals_mut().widgets.hovered.bg_fill = browser_toolbar_active();
    ui.visuals_mut().widgets.active.bg_fill = browser_toolbar_active();

}

fn browser_sort_control(
    ui: &mut Ui, sort: &mut BrowserSort, folders_before_tags: &mut bool,
    mut table_layout: Option<&mut FolderTableLayout>,
) {
    icon_text_dropdown_button(ui, ButtonIcon::Sort, "Sort", |ui| {
        style_list_menu(ui);
        for option in BrowserSort::ALL {
            if ui
                .selectable_label(
                    *sort == option
                        && !table_layout
                            .as_ref()
                            .is_some_and(|layout| layout.has_sort()),
                    option.label(),
                )
                .clicked()
            {
                *sort = option;
                // A column the table sorts by would otherwise keep winning.
                if let Some(layout) = table_layout.as_deref_mut() {
                    layout.clear_sort();
                }
                close_menu(ui);
            }
        }
        ui.separator();
        ui.baboon_checkbox(folders_before_tags, "Folders before tags");
    });
}

fn browser_other_control(ui: &mut Ui, search_scope: &mut BrowserSearchScope) {
    icon_menu_button(ui, ButtonIcon::Other, "Other browser options", |ui| {
        style_list_menu(ui);
        search_scope_controls(ui, search_scope);
    });
}

fn browser_toolbar_controls(
    ui: &mut Ui,
    mode: &mut BrowserMode,
    sort: &mut BrowserSort,
    show_prefixes: &mut bool,
    folders_before_tags: &mut bool,
    search_scope: &mut BrowserSearchScope,
    table_layout: Option<&mut FolderTableLayout>,
    show_modes: bool,
) -> bool {
    style_browser_toolbar(ui);

    let mut groups_clicked = false;
    let dedicated = table_layout.is_some();
    browser_sort_control(ui, sort, folders_before_tags, table_layout);
    if show_modes {
        icon_text_dropdown_button(ui, ButtonIcon::View, "View", |ui| {
            style_list_menu(ui);
            if dedicated && selectable_icon_text_button(ui, ButtonIcon::FolderClosed, "Flat", *mode == BrowserMode::Flat).on_hover_text("Current folder only. Mixed multi-selection and bulk actions.").clicked() { *mode = BrowserMode::Flat; close_menu(ui); }
            if selectable_icon_text_button(ui, ButtonIcon::FolderOpen, if dedicated { "Tree" } else { "Folders" }, *mode == BrowserMode::Folders).on_hover_text("Expandable folder tree. Single selection; use chevrons to expand.").clicked() {
                *mode = BrowserMode::Folders;
                close_menu(ui);
            }
            if selectable_icon_text_button(ui, ButtonIcon::Group, "Groups", *mode == BrowserMode::Groups).on_hover_text("Recursive grouping by tag type. Single selection.").clicked() {
                *mode = BrowserMode::Groups;
                groups_clicked = true;
                close_menu(ui);
            }
            ui.separator();
            ui.baboon_checkbox(show_prefixes, "Show prefixes");
        });
    }
    browser_other_control(ui, search_scope);
    groups_clicked
}

/// Folder-page wrapper for the shared controls. Keeping the controls in
/// this single helper means the wide row and wrapped narrow row cannot drift
/// into different button sets or spacing.
fn draw_folder_browser_controls(
    ui: &mut Ui,
    pane: &mut FolderBrowserState,
    show_prefixes: &mut bool,
    folders_before_tags: &mut bool,
    can_create: bool,
    action: &mut Option<BrowserAction>,
) {
    ui.spacing_mut().item_spacing.x = PANE_HEADER_ACTION_GAP;
    if !pane.assets_view {
        ui.add_enabled_ui(can_create, |ui| {
            icon_text_dropdown_button(ui, ButtonIcon::Add, "New", |ui| {
                style_list_menu(ui);
                let folder = tag_path::relative_tag_path(&pane.rel_path);
                let folder_rel = (!folder.is_empty()).then_some(folder);
                if icon_text_button(ui, ButtonIcon::NewTag, "New Tag...", true).clicked() {
                    *action = Some(BrowserAction::NewTagInFolder { folder_rel: folder_rel.clone() });
                    close_menu(ui);
                }
                if icon_text_button(ui, ButtonIcon::FolderOpen, "New Folder...", true).clicked() {
                    *action = Some(BrowserAction::NewContainerFolder { parent_rel: folder_rel });
                    close_menu(ui);
                }
            });
        });
    }
    if pane.assets_view {
        style_browser_toolbar(ui);
        browser_sort_control(ui, &mut pane.sort, folders_before_tags, Some(&mut pane.table_layout));
        icon_text_dropdown_button(ui, ButtonIcon::View, "View", |ui| {
            ui.label("Thumbnail size");
            ui.add(egui::Slider::new(&mut pane.asset_cell_size, MIN_CELL..=MAX_CELL));
        });
        icon_menu_button(ui, ButtonIcon::Filter, "Filter asset types", |ui| {
            style_list_menu(ui);
            ui.label("Asset types");
            ui.baboon_checkbox(&mut pane.asset_bitmaps, "Bitmaps");
            ui.baboon_checkbox(&mut pane.asset_models, "Models");
        });
        browser_other_control(ui, &mut pane.search_scope);
        return;
    }
    browser_toolbar_controls(
        ui,
        &mut pane.mode,
        &mut pane.sort,
        show_prefixes,
        folders_before_tags,
        &mut pane.search_scope,
        Some(&mut pane.table_layout),
        true,
    );
}

fn draw_folder_pane_header(
    ui: &mut Ui,
    pane: &mut FolderBrowserState,
    entries: &[TagEntry],
    is_loose: bool,
    is_container: bool,
    action: &mut Option<BrowserAction>,
) {
    let folder = tag_path::relative_tag_path(&pane.rel_path);
    let (breadcrumbs, title) = pane_header_path_parts(&folder);
    let is_favorite = browser_favorite_folders(ui).is_some_and(|folders| {
        folders.iter().any(|path| paths_match_case_insensitive(path, &pane.rel_path))
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
                                        entries,
                                        is_loose,
                                        is_container,
                                        is_favorite,
                                        action,
                                    );
                                },
                            );
                        });
                    } else {
                        draw_folder_header_common_actions(
                            ui,
                            pane,
                            entries,
                            is_loose,
                            is_container,
                            is_favorite,
                            action,
                        );
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
                draw_folder_header_common_actions(
                    ui,
                    pane,
                    entries,
                    is_loose,
                    is_container,
                    is_favorite,
                    action,
                );
            },
        );
    }
}

fn draw_folder_header_common_actions(
    ui: &mut Ui,
    pane: &mut FolderBrowserState,
    entries: &[TagEntry],
    is_loose: bool,
    is_container: bool,
    is_favorite: bool,
    action: &mut Option<BrowserAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = PANE_HEADER_ACTION_GAP;
        right_aligned_icon_menu_button(
            ui,
            ButtonIcon::Other,
            "Other folder actions",
            CONTEXT_MENU_WIDTH,
            |ui| {
                style_tag_context_menu(ui);
                let extract_label = tag_path::relative_tag_path(&pane.rel_path);
                let extract_label = if extract_label.is_empty() {
                    pane.label.clone()
                } else {
                    extract_label
                };
                if is_loose {
                    if let Some(menu_action) = loose_folder_primary_menu_items(
                        ui,
                        &pane.rel_path,
                        &pane.label,
                        Some(is_favorite),
                        false,
                        |ui| {
                            folder_tree_extract_menu_button(
                                ui,
                                &pane.tree,
                                entries,
                                extract_label.clone(),
                                pane.rel_path.clone(),
                                is_container,
                                is_loose,
                                true,
                            )
                        },
                    ) {
                        action.replace(menu_action);
                    }
                }
                if context_menu_button(ui, "Copy Folder Path").clicked() {
                    action.replace(BrowserAction::CopyFolderPath(pane.rel_path.clone()));
                    close_menu(ui);
                }
                context_menu_separator(ui);
                if !is_loose
                    && let Some(menu_action) = folder_tree_extract_menu_button(
                        ui,
                        &pane.tree,
                        entries,
                        extract_label,
                        pane.rel_path.clone(),
                        is_container,
                        is_loose,
                        true,
                    )
                {
                    action.replace(menu_action);
                }
                if !is_loose {
                    context_menu_separator(ui);
                }
                if context_menu_button(ui, "Dump folder to JSON...").clicked() {
                    action.replace(BrowserAction::DumpLooseFolderJson {
                        rel_path: pane.rel_path.clone(),
                        label: pane.label.clone(),
                    });
                    close_menu(ui);
                }
            },
        );
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
        if icon_text_button(ui, ButtonIcon::FileExplorer, crate::app::browser::actions::file_manager_name(), is_loose)
            .on_disabled_hover_text("Only loose editing-kit folders exist on disk")
            .clicked()
        {
            action.replace(BrowserAction::OpenLooseFolderInExplorer {
                rel_path: pane.rel_path.clone(),
            });
        }
        ui.label(RichText::new("Open folder in:").color(subtle_dark()));
    });
}

/// What the browser highlights: the tag whose tab the user is on, or, when
/// that tab is a folder pane, the folder it shows as a folder-pane key.
/// `None` when no tab is open.
pub(in crate::app) fn current_browser_item(kit: &Kit, view: &KitView) -> Option<String> {
    if let Some(key) = &kit.selected_key {
        return Some(key.clone());
    }
    let pane = kit.current_pane.as_deref()?;
    view.browser.folder_browsers.get(pane).map(|folder| folder_pane_key(&folder.rel_path))
}

/// The reveal that follows the current tab to `item`, as
/// [`current_browser_item`] names it: a tag by its key, a folder by its path.
/// It scrolls no further than it must.
fn follow_reveal(kit_id: KitId, kit: &Kit, item: &str) -> Option<RevealRequest> {
    let (key, ancestors, folder) = match item.strip_prefix(FOLDER_PANE_PREFIX) {
        Some(path) => {
            let mut labels: Vec<String> = tag_path::segments(path).map(str::to_owned).collect();
            let label = labels.pop()?;
            (label, labels, true)
        }
        None => {
            let entry = kit.entry_for_key(item)?;
            (entry.key.clone(), ancestor_labels(&entry.display_path), false)
        }
    };
    Some(RevealRequest { kit: kit_id, key, ancestors, folder, align: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::compare::GIT_REVIEW_TITLE;
    use crate::app::shell::frame::sidebar_wrappable_path_label;

    #[test]
    fn favorite_folders_and_shortcuts_follow_double_click_preference() {
        for double_click in [false, true] {
            for favorite_folder in [false, true] {
                let ctx = egui::Context::default();
                ctx.global_style_mut(|style| style.animation_time = 0.0);
                let mut time = 0.0;
                let mut frame = |events| {
                    time += 0.04;
                    let mut opened = false;
                    let output = crate::app::run_ui_test(
                        &ctx,
                        egui::RawInput { time: Some(time), events, ..Default::default() },
                        |ui| {
                            egui::CentralPanel::default().show(ui, |ui| {
                                if favorite_folder {
                                    set_browser_favorite_folders(ui,
                                        Some(Arc::new(vec![PathBuf::from("favorite")])));
                                    opened = draw_favorites(ui, &[], &[], None, None, "", false,
                                        double_click, &HashSet::new(), BrowserSearchScope::default(),
                                        &std::collections::BTreeMap::new(), None).0.is_some();
                                } else {
                                    let response = sidebar_list_button(ui, ButtonIcon::Git, "favorite");
                                    opened = browser_open_requested(&response, double_click);
                                }
                            });
                        },
                    );
                    let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == "favorite" =>
                            Some(text.galley.rect.translate(text.pos.to_vec2()).center()),
                        _ => None,
                    }).expect("the row is drawn");
                    (pos, opened)
                };
                let pos = frame(Vec::new()).0;
                let pointer = |pressed| egui::Event::PointerButton {
                    pos, pressed, button: egui::PointerButton::Primary, modifiers: Default::default(),
                };
                frame(vec![egui::Event::PointerMoved(pos), pointer(true)]);
                assert_eq!(frame(vec![pointer(false)]).1, !double_click,
                    "first click: folder={favorite_folder}, double_click={double_click}");
                frame(vec![pointer(true)]);
                assert!(frame(vec![pointer(false)]).1,
                    "second click: folder={favorite_folder}, double_click={double_click}");
            }
        }
    }

    #[test]
    fn shortcut_add_button_click_keeps_section_open() {
        let ctx = egui::Context::default();
        ctx.global_style_mut(|style| style.animation_time = 0.0);
        let frame = |events| {
            let mut result = None;
            let mut body_visible = false;
            let _ = crate::app::run_ui_test(&ctx, egui::RawInput { events, ..Default::default() }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    result = Some(show_browser_navigation_section_with_add(ui, "shortcuts", "Shortcuts",
                        ButtonIcon::Pin, text_dark(), true, |_| { body_visible = true; }));
                });
            });
            (result.unwrap(), body_visible)
        };
        let ((header, _), _) = frame(Vec::new());
        let pos = egui::pos2(header.rect.right() - SHORTCUT_ADD_RIGHT_PADDING - ICON_BUTTON_SIZE.x * 0.5, header.rect.center().y);
        frame(vec![egui::Event::PointerMoved(pos)]);
        let pointer = |pressed| egui::Event::PointerButton {
            pos, pressed, button: egui::PointerButton::Primary, modifiers: Default::default(),
        };
        frame(vec![pointer(true)]);
        let ((_, added), visible) = frame(vec![pointer(false)]);
        assert!(added);
        assert!(visible);
        assert!(frame(Vec::new()).1);
    }

    #[test]
    fn browser_categories_restore_saved_choices_in_a_fresh_ui() {
        use crate::app::prefs::BrowserSectionState;
        fn frame(ctx: &egui::Context, state: &mut BrowserSectionState, events: Vec<egui::Event>)
            -> ([egui::Rect; 3], [bool; 3])
        {
            let mut headers = [egui::Rect::NOTHING; 3];
            let mut visible = [false; 3];
            let _ = crate::app::run_ui_test(ctx, egui::RawInput { events, ..Default::default() }, |ui| {
                with_browser_section_state(ui, state, |ui| {
                    for (index, (id, title)) in [
                        ("browser_favorites", "Favorites"),
                        ("browser_shortcuts", "Shortcuts"),
                        ("browser_recents", "Recents"),
                    ].into_iter().enumerate() {
                        headers[index] = show_browser_navigation_section(ui, id, title,
                            ButtonIcon::Pin, text_dark(), |ui| {
                                visible[index] = true;
                                ui.label("Entry");
                            }).rect;
                    }
                });
            });
            (headers, visible)
        }
        let context = || {
            let ctx = egui::Context::default();
            ctx.global_style_mut(|style| style.animation_time = 0.0);
            ctx
        };
        let ctx = context();
        let mut saved = BrowserSectionState::default();
        assert_eq!(frame(&ctx, &mut saved, vec![]).1, [false; 3]);
        for index in [0, 2] {
            let pos = frame(&ctx, &mut saved, vec![]).0[index].center();
            frame(&ctx, &mut saved, vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(&ctx, &mut saved, vec![egui::Event::PointerButton {
                    pos, pressed, button: egui::PointerButton::Primary, modifiers: Default::default(),
                }]);
            }
        }
        assert_eq!(saved, BrowserSectionState { favorites: true, shortcuts: false, recents: true });
        let fresh = context();
        frame(&fresh, &mut saved, vec![]);
        assert_eq!(frame(&fresh, &mut saved, vec![]).1, [true, false, true]);
        let mut other_kit = BrowserSectionState::default();
        frame(&fresh, &mut other_kit, vec![]);
        assert_eq!(frame(&fresh, &mut other_kit, vec![]).1, [false; 3]);
        assert_eq!(frame(&fresh, &mut saved, vec![]).1, [true, false, true]);
    }

    #[test]
    fn shortcut_sections_collapse_independently_for_each_kit() {
        let ctx = egui::Context::default();
        ctx.global_style_mut(|style| style.animation_time = 0.0);
        let mut ids = [egui::Id::NULL; 2];
        let mut visible = [false; 2];
        let mut frame = || {
            visible.fill(false);
            let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    for kit in 0..2 {
                        ui.push_id(kit, |ui| {
                            ids[kit] = ui.make_persistent_id("browser_shortcuts");
                            show_browser_navigation_section(ui, "browser_shortcuts", "Shortcuts",
                                ButtonIcon::Pin, text_dark(), |ui| {
                                    visible[kit] = true;
                                    sidebar_list_button(ui, ButtonIcon::Git, GIT_REVIEW_TITLE);
                                });
                        });
                    }
                });
            });
            (ids, visible)
        };
        let (ids, visible) = frame();
        assert_eq!(visible, [true, true]);
        assert_ne!(ids[0], ids[1]);
        let mut state = egui::collapsing_header::CollapsingState::load(&ctx, ids[0]).unwrap();
        state.set_open(false);
        state.store(&ctx);
        assert_eq!(frame().1, [false, true]);
    }

    #[test]
    fn unavailable_app_shortcuts_cannot_launch() {
        let ctx = egui::Context::default();
        let frame = |enabled, events| {
            let mut response = None;
            let _ = crate::app::run_ui_test(&ctx, egui::RawInput { events, ..Default::default() }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    response = Some(sidebar_app_button(ui, None, "Tag Test", "T", enabled));
                });
            });
            response.unwrap()
        };
        let pos = frame(false, Vec::new()).rect.center();
        let pointer = |pressed| egui::Event::PointerButton {
            pos, pressed, button: egui::PointerButton::Primary, modifiers: Default::default(),
        };
        frame(false, vec![egui::Event::PointerMoved(pos), pointer(true)]);
        assert!(!frame(false, vec![pointer(false)]).clicked());
        // Publish the enabled hit target before the next pointer press.
        frame(true, Vec::new());
        frame(true, vec![egui::Event::PointerMoved(pos), pointer(true)]);
        assert!(frame(true, vec![pointer(false)]).clicked());
    }

    #[test]
    fn asset_toolbar_hides_folder_and_group_modes() {
        for show_modes in [false, true] {
            let ctx = egui::Context::default();
            let mut mode = BrowserMode::Folders;
            let mut sort = BrowserSort::Natural;
            let mut prefixes = false;
            let mut folders_first = true;
            let mut scope = BrowserSearchScope::default();
            let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        browser_toolbar_controls(
                            ui,
                            &mut mode,
                            &mut sort,
                            &mut prefixes,
                            &mut folders_first,
                            &mut scope,
                            None,
                            show_modes,
                        );
                    });
                });
            });
            let visible = output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.job.text.trim() == "View"));
            assert_eq!(visible, show_modes);
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if matches!(text.galley.job.text.as_str(), "Folders" | "Groups"))));
        }
    }

    #[test]
    fn search_stacks_before_the_toolbar_reaches_600_points() {
        assert!(!folder_browser_search_stacks(600.0));
        assert!(folder_browser_search_stacks(599.0));
    }

    #[test]
    fn search_hint_names_the_selected_search_targets() {
        assert_eq!(
            BrowserSearchScope {
                tags: true,
                folders: false,
                keywords: true
            }
            .hint(),
            "search tags & keywords"
        );
    }

    #[test]
    fn sidebar_paths_can_wrap_after_each_separator() {
        assert_eq!(
            sidebar_wrappable_path_label(r"C:\tags\objects"),
            "C:\\\u{200b}tags\\\u{200b}objects"
        );
    }
}

