//! One current directory, rendered with the existing folder table columns.
use super::*;
use std::hash::{Hash, Hasher};
type FlatRow = (String, String, Option<usize>, Option<usize>);
struct FlatListing {
    rows: Vec<FlatRow>,
    nodes: Vec<TagTreeNode>,
    /// `rows` as the selection keeps them, built once per listing.
    selection_rows: Arc<Vec<(String, String)>>,
}

fn path_key(path: &Path) -> String {
    tag_path::relative_tag_path(path).to_ascii_lowercase()
}
fn children<'a>(nodes: &'a [TagTreeNode], path: &Path) -> Option<&'a [TagTreeNode]> {
    for node in nodes {
        if path_key(&node.rel_path) == path_key(path) {
            return Some(&node.children);
        }
        if let Some(found) = children(&node.children, path) {
            return Some(found);
        }
    }
    None
}
pub(super) fn immediate_folders<'a>(
    pane: &'a FolderBrowserState,
    source: &'a LoadedSourceData,
) -> &'a [TagTreeNode] {
    if matches!(source.source, TagSource::LooseFolder { .. }) {
        &pane.tree.children
    } else if pane.rel_path.as_os_str().is_empty() {
        &source.tree.children
    } else {
        children(&source.tree.children, &pane.rel_path).unwrap_or(&[])
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    ui: &mut Ui,
    pane: &mut FolderBrowserState,
    pane_key: &str,
    kit: KitId,
    source: &LoadedSourceData,
    generation: u64,
    folders_first: bool,
    double_click: bool,
    favorites: &HashSet<String>,
    keywords: &Arc<std::collections::BTreeMap<String, Vec<String>>>,
    need_scan: &mut bool,
    action: &mut Option<BrowserAction>,
) {
    if matches!(source.source, TagSource::LooseFolder { .. }) && !source.complete_scan {
        *need_scan = true;
        ui.spinner();
        ui.label("Indexing folder contents…");
        return;
    }
    let prefix = path_key(&pane.rel_path);
    let entries = source.full_entry_set();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let sort = folder_table_sort_signature(ui);
    (
        generation,
        entries.as_ptr() as usize,
        entries.len(),
        &prefix,
        &pane.filter,
        pane.search_scope,
        pane.sort,
        folders_first,
        sort,
        Arc::as_ptr(keywords) as usize,
    )
        .hash(&mut hasher);
    let signature = hasher.finish();
    let cache_id = egui::Id::unique(("flat_listing", kit, pane_key));
    let cached = ui.data(|data| data.get_temp::<Arc<(u64, FlatListing)>>(cache_id));
    let listing = if let Some(cached) = cached.filter(|cached| cached.0 == signature) {
        cached
    } else {
        invalidate_browser_navigation(ui.ctx());
        let mut folders = std::collections::BTreeMap::<String, PathBuf>::new();
        let folder_nodes = immediate_folders(pane, source);
        let pending: HashSet<_> = folder_nodes
            .iter()
            .filter(|node| folder_is_pending_and_empty(node))
            .map(|node| node.rel_path.clone())
            .collect();
        for node in folder_nodes {
            folders.insert(node.label.to_ascii_lowercase(), node.rel_path.clone());
        }
        let mut indices = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let display = entry.display_path.replace('/', "\\");
            let parent = tag_path::parent(&display).unwrap_or("");
            if parent.eq_ignore_ascii_case(&prefix) {
                if scoped_entry_matches(
                    entry,
                    &pane.filter,
                    pane.search_scope,
                    keywords.get(&entry.key).map(Vec::as_slice).unwrap_or(&[]),
                ) {
                    indices.push(index);
                }
            } else if let Some(relative) = display.get(prefix.len()..).filter(|_| {
                prefix.is_empty()
                    || display
                        .to_ascii_lowercase()
                        .starts_with(&(prefix.clone() + "\\"))
            }) {
                let relative = relative.trim_start_matches('\\');
                if let Some((name, _)) = relative.split_once('\\') {
                    folders
                        .entry(name.to_ascii_lowercase())
                        .or_insert_with(|| pane.rel_path.join(name));
                }
            }
        }
        match pane.sort {
            BrowserSort::Natural => indices.sort_by_cached_key(|&i| {
                crate::core::source::natural_key(tag_path::name(&entries[i].display_path))
            }),
            BrowserSort::Name => indices
                .sort_by_cached_key(|&i| tag_path::name(&entries[i].display_path).to_lowercase()),
            BrowserSort::Type => indices.sort_by_cached_key(|&i| {
                (
                    entries[i].group_name.clone(),
                    tag_path::name(&entries[i].display_path).to_lowercase(),
                )
            }),
        }
        if let Some(order) = table_order_entries(ui, &indices, entries) {
            indices = order;
        }
        let nodes: Vec<_> = folders
            .into_iter()
            .filter(|(_, path)| {
                pane.filter.is_empty()
                    || (pane.search_scope.folders
                        && scoped_folder_matches(
                            &path.file_name().unwrap_or_default().to_string_lossy(),
                            &pane.filter,
                        ))
            })
            .map(|(_, path)| TagTreeNode {
                label: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                pending: pending.contains(&path),
                rel_path: path,
                ..Default::default()
            })
            .collect();
        let folder_order =
            table_order_folders(ui, &nodes).unwrap_or_else(|| (0..nodes.len()).collect());
        let mut rows: Vec<(String, String, Option<usize>, Option<usize>)> = folder_order
            .into_iter()
            .map(|i| {
                (
                    folder_pane_key(&nodes[i].rel_path),
                    nodes[i].label.clone(),
                    None,
                    Some(i),
                )
            })
            .collect();
        let tags: Vec<_> = indices
            .into_iter()
            .map(|i| {
                (
                    entries[i].key.clone(),
                    tag_path::name(&entries[i].display_path).into(),
                    Some(i),
                    None,
                )
            })
            .collect();
        if folders_first {
            rows.extend(tags);
        } else {
            rows.splice(0..0, tags);
        }
        let selection_rows = Arc::new(
            rows.iter()
                .map(|row| (row.0.clone(), row.1.clone()))
                .collect(),
        );
        let value = Arc::new((
            signature,
            FlatListing {
                rows,
                nodes,
                selection_rows,
            },
        ));
        ui.data_mut(|data| data.insert_temp(cache_id, value.clone()));
        value
    };
    let rows = &listing.1.rows;
    let nodes = &listing.1.nodes;
    pane.selection.rows = listing.1.selection_rows.clone();
    pane.selection.retain_rows();
    begin_multi(ui, pane_key, kit, &pane.selection, false);
    // Whether drops may change this kit's files.
    let writable =
        matches!(source.source, TagSource::LooseFolder { .. }) && !operation_read_only(ui);
    let row_height = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
    let mut scroll = ScrollArea::vertical()
        .id_salt(("flat_folder", pane_key))
        .auto_shrink([false, false]);
    let viewport_id = egui::Id::unique(("flat_browser_viewport", kit, pane_key));
    if let Some((key, _)) = browser_navigation_target(ui) {
        if let Some(index) = rows.iter().position(|row| row.0 == key) {
            let (offset, height) = ui
                .data(|data| data.get_temp::<(f32, f32)>(viewport_id))
                .unwrap_or((0.0, ui.available_height()));
            let top = index as f32 * row_height;
            let bottom = top + row_height;
            let desired = if top < offset {
                Some(top)
            } else if bottom > offset + height {
                Some((bottom - height).max(0.0))
            } else {
                None
            };
            if let Some(offset) = desired {
                scroll = scroll.vertical_scroll_offset(offset);
            }
        }
    }
    let output = scroll.show_rows(
        ui,
        row_height - ui.spacing().item_spacing.y,
        rows.len(),
        |ui, visible| {
            if browser_navigation_building(ui) {
                for (key, label, entry, _) in rows {
                    let id = if entry.is_some() {
                        ui.make_persistent_id(("browser_tag", key))
                    } else {
                        ui.make_persistent_id(("flat_folder", key))
                    };
                    register_browser_navigation(ui, key, id);
                    register_browser_name(ui, key, label);
                }
            }
            let at_end = visible.end == rows.len();
            for (key, label, entry, folder) in &rows[visible] {
                if let Some(i) = entry {
                    if let Some(request) = draw_entry(
                        ui,
                        &entries[*i],
                        None,
                        false,
                        double_click,
                        None,
                        Some(favorites),
                        false,
                        source.game,
                    ) {
                        *action = Some(request);
                    }
                } else if let Some(i) = folder {
                    let node = &nodes[*i];
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(
                            ui.available_width(),
                            row_height - ui.spacing().item_spacing.y,
                        ),
                        egui::Sense::hover(),
                    );
                    // Dragged to move or copy it, and a place to drop others.
                    let response = ui.interact(
                        rect,
                        ui.make_persistent_id(("flat_folder", key)),
                        egui::Sense::click_and_drag(),
                    );
                    #[cfg(test)]
                    TREE_ROW_TOPS.with(|tops| tops.borrow_mut().push((key.clone(), rect.top())));
                    register_browser_folder(ui, response.id, None, &node.rel_path, label, false);
                    register_browser_name(ui, key, label);
                    let selected = select_browser_tag(ui, &response, key);
                    if response.drag_started() {
                        super::drag_move::start(ui, &response, key, None);
                    }
                    if let Some(drop) = super::drag_move::drop_target(
                        ui.ctx(),
                        ui.painter(),
                        ui.rect_contains_pointer(rect),
                        rect,
                        kit,
                        &node.rel_path,
                        writable,
                    ) {
                        *action = Some(drop);
                    }
                    if let Some(shape) = browser_row_hover_shape(ui, &response, rect, false) {
                        ui.painter().add(shape);
                    }
                    if selected {
                        paint_browser_selection_border(ui, rect);
                    }
                    let icon = egui::Rect::from_center_size(
                        egui::pos2(rect.left() + FLAT_ROW_CONTENT_INSET + 8.0, rect.center().y),
                        egui::vec2(16.0, 16.0),
                    );
                    paint_button_icon_at(
                        ui,
                        ButtonIcon::FolderClosed,
                        icon,
                        disclosure_triangle_blue(),
                    );
                    ui.painter().with_clip_rect(folder_name_clip(ui)).text(
                        egui::pos2(rect.left() + FLAT_ROW_CONTENT_INSET + 21.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::TextStyle::Body.resolve(ui.style()),
                        text_dark(),
                    );
                    paint_folder_columns(ui, rect, None, Some(&node.rel_path));
                    if response.double_clicked()
                        && !ui.input(|input| {
                            input.modifiers.ctrl || input.modifiers.command || input.modifiers.shift
                        })
                    {
                        *action = Some(BrowserAction::OpenFolderBrowser {
                            rel_path: node.rel_path.clone(),
                            label: label.clone(),
                            open_in_new_tab: false,
                        });
                    }
                    context_menu(&response, |ui| {
                        if multi(ui).is_some_and(|state| state.selection.keys.len() > 1) {
                            if let Some(request) = draw_multi_menu(ui) {
                                *action = Some(request);
                            }
                        } else {
                            // Flat rows intentionally omit descendants, but the standard
                            // folder menu needs the complete subtree and matching indices.
                            let tree =
                                crate::core::source::build_tree_beneath(entries, &node.rel_path);
                            let menu_node = TagTreeNode {
                                label: node.label.clone(),
                                rel_path: node.rel_path.clone(),
                                children: tree.children,
                                entries: tree.entries,
                                pending: node.pending,
                                ..Default::default()
                            };
                            if let Some(request) = draw_folder_context_menu_contents(
                                ui,
                                &menu_node,
                                entries,
                                false,
                                matches!(source.source, TagSource::IoStoreContainerSet { .. }),
                                browser_loose_source(ui).then_some(favorites),
                            ) {
                                *action = Some(request);
                            }
                        }
                    });
                }
            }
            if at_end && ui.cursor().top() < ui.clip_rect().bottom() {
                let rect = egui::Rect::from_min_max(ui.cursor().min, ui.clip_rect().max);
                let blank =
                    ui.interact(rect, ui.scope_id().with("flat_blank"), egui::Sense::click());
                clear_multi_on_blank(ui, &blank);
            }
        },
    );
    ui.data_mut(|data| {
        data.insert_temp(
            viewport_id,
            (output.state.clamped_offset().y, output.inner_rect.height()),
        )
    });
    // Anywhere else in the pane drops into the folder it shows. Rows drawn
    // above have already claimed a drop onto a folder row.
    let area = output.inner_rect;
    if let Some(drop) = super::drag_move::drop_target(
        ui.ctx(),
        ui.painter(),
        ui.rect_contains_pointer(area),
        area,
        kit,
        &pane.rel_path,
        writable,
    ) {
        *action = Some(drop);
    }
    if let Some(state) = end_multi(ui) {
        pane.selection = state.selection;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::*;
    struct Harness {
        app: Baboon,
        ctx: egui::Context,
        pane: String,
        source: LoadedSourceData,
        time: f64,
        keywords: Arc<std::collections::BTreeMap<String, Vec<String>>>,
        /// What the last frame's draw asked for.
        action: Option<BrowserAction>,
    }
    impl Harness {
        fn new(kit: &LooseKit) -> Self {
            let mut app = app();
            kit.install(&mut app);
            let ctx = ctx();
            Baboon::configure_context(&ctx);
            app.handle_browser_action(
                BrowserAction::OpenFolderBrowser {
                    rel_path: PathBuf::new(),
                    label: "Tags".into(),
                    open_in_new_tab: true,
                },
                ctx.clone(),
            );
            let pane = app.model.kits[0].current_pane.clone().unwrap();
            Self {
                app,
                ctx,
                pane,
                source: kit.source(),
                time: 1.0,
                keywords: Arc::default(),
                action: None,
            }
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> HashMap<String, egui::Pos2> {
            self.time += 0.1;
            let kit = self.app.model.active_kit_id();
            crate::app::run_ui_test(
                &self.ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 200.0),
                    )),
                    events,
                    time: Some(self.time),
                    ..Default::default()
                },
                |ui| {
                    self.app.handle_browser_selection_input(&self.ctx);
                    self.app
                        .refresh_folder_browser_pane(0, &self.pane, &self.ctx);
                    egui::CentralPanel::default().show(ui, |ui| {
                        let mut pane = self.app.views[kit]
                            .browser
                            .folder_browsers
                            .remove(&self.pane)
                            .unwrap();
                        set_browser_is_folder_pane(ui, true);
                        set_browser_loose_source(ui, true);
                        // As the pane does: the kit's tags, and that it can change.
                        set_operation_entries(
                            ui,
                            kit,
                            1,
                            self.source.full_entry_set(),
                            false,
                            true,
                        );
                        ui.data_mut(|data| {
                            data.insert_temp(
                                browser_selection_owner_id().with("drawing"),
                                (kit, self.pane.clone()),
                            )
                        });
                        set_browser_selected_tag(ui, pane.selected_tag.clone());
                        begin_browser_navigation(
                            ui,
                            egui::Id::unique(("folder_navigation", kit, &self.pane)),
                        );
                        TREE_ROW_TOPS.with(|tops| tops.borrow_mut().clear());
                        draw(
                            ui,
                            &mut pane,
                            &self.pane,
                            kit,
                            &self.source,
                            1,
                            true,
                            true,
                            &HashSet::new(),
                            &self.keywords,
                            &mut false,
                            &mut self.action,
                        );
                        finish_browser_navigation(ui);
                        pane.selected_tag = browser_selected_tag(ui);
                        self.app.views[kit]
                            .browser
                            .folder_browsers
                            .insert(self.pane.clone(), pane);
                    });
                },
            );
            TREE_ROW_TOPS.with(|tops| {
                tops.borrow()
                    .iter()
                    .map(|(key, top)| (key.clone(), egui::pos2(100.0, top + 8.0)))
                    .collect()
            })
        }
        fn click(&mut self, pos: egui::Pos2, mods: egui::Modifiers) {
            self.frame(vec![
                egui::Event::ModifiersChanged(mods),
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: mods,
                },
            ]);
            self.frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: mods,
            }]);
        }
        /// Press at `from`, drag past the threshold to `to`, and release there
        /// holding `mods`. Returns what the drop asked for.
        fn drag(
            &mut self,
            from: egui::Pos2,
            to: egui::Pos2,
            mods: egui::Modifiers,
        ) -> Option<BrowserAction> {
            self.action = None;
            self.frame(vec![egui::Event::PointerMoved(from)]);
            self.frame(vec![egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]);
            self.frame(vec![egui::Event::PointerMoved(from + egui::vec2(0.0, 6.0))]);
            self.frame(vec![egui::Event::PointerMoved(to)]);
            assert!(
                egui::DragAndDrop::has_any_payload(&self.ctx),
                "the drag never started"
            );
            self.frame(vec![
                egui::Event::ModifiersChanged(mods),
                egui::Event::PointerMoved(to),
            ]);
            self.frame(vec![egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: mods,
            }]);
            self.frame(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
            self.action.take()
        }
        fn selected(&self) -> &BrowserSelection {
            &self.app.views[self.app.model.active_kit_id()]
                .browser
                .folder_browsers[&self.pane]
                .selection
        }
    }
    fn dropped(action: Option<BrowserAction>) -> Option<(Vec<String>, bool, PathBuf)> {
        match action? {
            BrowserAction::DropItems {
                keys, copy, folder, ..
            } => Some((keys, copy, folder)),
            _ => None,
        }
    }
    fn drag_kit(name: &str) -> LooseKit {
        let kit = LooseKit::new(name, "halo3_mcc");
        kit.write_mcc("alpha", "model", |_| {});
        kit.write_mcc("beta", "model", |_| {});
        kit.write_mcc("dest/inside", "model", |_| {});
        kit.write_mcc("src/inside", "model", |_| {});
        kit
    }
    #[test]
    fn dragging_a_folder_onto_another_moves_it_there() {
        let kit = drag_kit("flat-drag-folder");
        let mut h = Harness::new(&kit);
        let rows = h.frame(Vec::new());
        let (src, dest) = (
            folder_pane_key(Path::new("src")),
            folder_pane_key(Path::new("dest")),
        );
        let dropped = dropped(h.drag(rows[&src], rows[&dest], egui::Modifiers::NONE));
        assert_eq!(
            dropped,
            Some((vec![src.clone()], false, PathBuf::from("dest")))
        );
        assert!(
            h.selected().keys.contains(&src),
            "the dragged folder became the selection"
        );
    }
    #[test]
    fn dragging_the_selection_with_ctrl_copies_every_selected_item() {
        let kit = drag_kit("flat-drag-copy");
        let mut h = Harness::new(&kit);
        let rows = h.frame(Vec::new());
        let (alpha, beta) = (kit.key("alpha.model"), kit.key("beta.model"));
        h.click(rows[&alpha], egui::Modifiers::NONE);
        h.click(rows[&beta], egui::Modifiers::CTRL);
        let dest = folder_pane_key(Path::new("dest"));
        let (keys, copy, folder) =
            dropped(h.drag(rows[&beta], rows[&dest], egui::Modifiers::CTRL)).expect("a drop");
        assert_eq!((copy, folder), (true, PathBuf::from("dest")));
        assert_eq!(keys.len(), 2, "both selected tags go: {keys:?}");
        assert!(keys.contains(&alpha) && keys.contains(&beta));
    }
    #[test]
    fn a_folder_dropped_on_itself_or_its_own_folder_does_nothing() {
        let kit = drag_kit("flat-drag-refused");
        let mut h = Harness::new(&kit);
        let rows = h.frame(Vec::new());
        let src = folder_pane_key(Path::new("src"));
        // Back onto its own row: a folder can't go inside itself.
        assert!(
            dropped(h.drag(
                rows[&src],
                rows[&src] + egui::vec2(30.0, 0.0),
                egui::Modifiers::NONE
            ))
            .is_none()
        );
        // Onto a tag beside it, which drops into the folder this pane shows,
        // where it already is.
        let alpha = kit.key("alpha.model");
        assert!(dropped(h.drag(rows[&src], rows[&alpha], egui::Modifiers::NONE)).is_none());
        // With Ctrl the same drop duplicates it in place.
        let (keys, copy, folder) =
            dropped(h.drag(rows[&src], rows[&alpha], egui::Modifiers::CTRL)).expect("a copy");
        assert_eq!((keys, copy, folder), (vec![src], true, PathBuf::new()));
    }
    #[test]
    fn flat_listing_selects_mixed_siblings_and_never_selects_hidden_descendants() {
        let kit = LooseKit::new("flat-mixed-ui", "halo3_mcc");
        kit.write_mcc("alpha", "model", |_| {});
        kit.write_mcc("beta", "model", |_| {});
        kit.write_mcc("sub/hidden", "model", |_| {});
        std::fs::create_dir_all(kit.path("empty")).unwrap();
        let mut h = Harness::new(&kit);
        h.frame(vec![]);
        let rows = h.frame(vec![]);
        let a = kit.key("alpha.model");
        let b = kit.key("beta.model");
        let folder = folder_pane_key(Path::new("sub"));
        assert!(!rows.contains_key(&kit.key("sub/hidden.model")));
        assert!(rows.contains_key(&folder_pane_key(Path::new("empty"))));
        h.click(rows[&folder], egui::Modifiers::NONE);
        h.click(rows[&a], egui::Modifiers::CTRL);
        assert_eq!(h.selected().keys.len(), 2);
        assert!(h.selected().keys.contains(&folder));
        h.frame(vec![
            egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            },
        ]);
        assert_eq!(h.selected().keys.len(), 4);
        h.frame(vec![
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::Text("b".into()),
        ]);
        for _ in 0..3 {
            h.frame(vec![]);
        }
        assert_eq!(h.selected().ordered_keys(), vec![b]);
        assert!(
            h.app.model.kits[0].parsed_tags.is_empty(),
            "typing must not open a tag"
        );
    }
    #[test]
    fn type_ahead_reaches_offscreen_items_and_builds_a_prefix() {
        let kit = LooseKit::new("flat-typeahead-ui", "halo3_mcc");
        for i in 0..40 {
            kit.write_mcc(&format!("a{i:02}"), "model", |_| {});
        }
        kit.write_mcc("beta", "model", |_| {});
        kit.write_mcc("bravo", "model", |_| {});
        let mut h = Harness::new(&kit);
        h.frame(vec![]);
        let rows = h.frame(vec![]);
        h.click(rows[&kit.key("a00.model")], egui::Modifiers::NONE);
        h.frame(vec![egui::Event::Text("b".into())]);
        for _ in 0..3 {
            h.frame(vec![]);
        }
        assert_eq!(h.selected().ordered_keys(), vec![kit.key("beta.model")]);
        h.frame(vec![egui::Event::Text("r".into())]);
        for _ in 0..3 {
            h.frame(vec![]);
        }
        assert_eq!(h.selected().ordered_keys(), vec![kit.key("bravo.model")]);
    }
    #[test]
    fn shift_ranges_use_visible_order_and_escape_leaves_typeahead_available() {
        let kit = LooseKit::new("flat-shift-escape", "halo3_mcc");
        for name in ["alpha", "beta", "gamma"] {
            kit.write_mcc(name, "model", |_| {});
        }
        let mut h = Harness::new(&kit);
        h.frame(vec![]);
        let rows = h.frame(vec![]);
        h.click(rows[&kit.key("alpha.model")], egui::Modifiers::NONE);
        h.click(rows[&kit.key("gamma.model")], egui::Modifiers::SHIFT);
        assert_eq!(h.selected().keys.len(), 3);
        h.frame(vec![
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        assert!(h.selected().keys.is_empty());
        h.frame(vec![egui::Event::Text("b".into())]);
        for _ in 0..3 {
            h.frame(vec![]);
        }
        assert_eq!(h.selected().ordered_keys(), vec![kit.key("beta.model")]);
    }
    #[test]
    fn flat_navigation_includes_empty_children_of_an_unexpanded_sidebar_folder() {
        let kit = LooseKit::new("flat-nested-empty", "halo3_mcc");
        std::fs::create_dir_all(kit.path("unopened/empty/deep")).unwrap();
        let mut h = Harness::new(&kit);
        assert!(h.source.tree.children[0].children.is_empty());
        let id = h.app.model.active_kit_id();
        crate::app::shell::frame::navigate_folder_browser(
            h.app.views[id]
                .browser
                .folder_browsers
                .get_mut(&h.pane)
                .unwrap(),
            PathBuf::from("unopened"),
            "unopened".into(),
        );
        h.frame(vec![]);
        let rows = h.frame(vec![]);
        assert!(rows.contains_key(&folder_pane_key(Path::new("unopened/empty"))));
    }
    #[test]
    fn typing_to_an_already_visible_row_does_not_move_the_listing() {
        let kit = LooseKit::new("flat-typeahead-stable-viewport", "halo3_mcc");
        for index in 0..20 {
            kit.write_mcc(&format!("alpha{index:02}"), "model", |_| {});
        }
        let mut h = Harness::new(&kit);
        h.frame(vec![]);
        let before = h.frame(vec![]);
        let first = kit.key("alpha00.model");
        h.click(before[&first], egui::Modifiers::NONE);
        h.frame(vec![egui::Event::Text("a".into())]);
        for _ in 0..4 {
            let rows = h.frame(vec![]);
            assert!(
                rows.get(&first)
                    .is_some_and(|position| (position.y - before[&first].y).abs() < 0.5),
                "typing to a visible item moved the entire listing: before={:?}, after={:?}",
                before[&first],
                rows.get(&first)
            );
        }
        assert_eq!(h.selected().ordered_keys(), vec![kit.key("alpha01.model")]);
    }
    /// Alt+Up works without the browser holding focus, so it must act on the
    /// kit being worked in, not on a pane last clicked in another kit.
    #[test]
    fn alt_up_acts_on_the_active_kit_not_the_last_clicked_pane() {
        let kit_a = LooseKit::new("flat-up-kit-a", "halo3_mcc");
        std::fs::create_dir_all(kit_a.path("a/inner")).unwrap();
        let kit_b = LooseKit::new("flat-up-kit-b", "halo3_mcc");
        std::fs::create_dir_all(kit_b.path("b/inner")).unwrap();
        let mut app = app();
        let ctx = ctx();
        let open = |app: &mut Baboon, rel: &str| {
            app.handle_browser_action(
                BrowserAction::OpenFolderBrowser {
                    rel_path: PathBuf::from(rel),
                    label: rel.into(),
                    open_in_new_tab: true,
                },
                ctx.clone(),
            );
            app.model.kits[app.model.active]
                .current_pane
                .clone()
                .unwrap()
        };
        kit_a.install(&mut app);
        let pane_a = open(&mut app, "a/inner");
        let a = app.model.active_kit_id();
        app.add_kit();
        kit_b.install(&mut app);
        let pane_b = open(&mut app, "b/inner");
        let b = app.model.active_kit_id();
        // A Flat row was clicked in kit B, then work moved to kit A.
        ctx.data_mut(|data| {
            data.insert_temp(
                browser_selection_owner_id().with("pane"),
                (b, pane_b.clone()),
            )
        });
        app.focus_kit(0);
        let alt_up = egui::Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::ALT,
        };
        let input = egui::RawInput {
            events: vec![alt_up],
            ..Default::default()
        };
        let _ = crate::app::run_ui_test(&ctx, input, |_| app.handle_browser_selection_input(&ctx));
        assert_eq!(
            app.views[b].browser.folder_browsers[&pane_b].rel_path,
            PathBuf::from("b/inner"),
            "kit B untouched"
        );
        assert_eq!(
            app.views[a].browser.folder_browsers[&pane_a].rel_path,
            PathBuf::from("a"),
            "kit A went up"
        );
    }
    #[test]
    fn alt_up_reselects_and_reveals_the_departed_folder_with_keyboard_focus() {
        let kit = LooseKit::new("flat-up-selection", "halo3_mcc");
        for index in 0..30 {
            std::fs::create_dir_all(kit.path(&format!("folder{index:02}"))).unwrap();
        }
        std::fs::create_dir_all(kit.path("z_child/empty")).unwrap();
        let mut h = Harness::new(&kit);
        let id = h.app.model.active_kit_id();
        crate::app::shell::frame::navigate_folder_browser(
            h.app.views[id]
                .browser
                .folder_browsers
                .get_mut(&h.pane)
                .unwrap(),
            PathBuf::from("z_child"),
            "z_child".into(),
        );
        h.frame(vec![]);
        let rows = h.frame(vec![]);
        h.click(
            rows[&folder_pane_key(Path::new("z_child/empty"))],
            egui::Modifiers::NONE,
        );
        h.frame(vec![egui::Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::ALT,
        }]);
        let mut visible = HashMap::new();
        for _ in 0..4 {
            visible = h.frame(vec![]);
        }
        let selected = folder_pane_key(Path::new("z_child"));
        assert_eq!(h.selected().ordered_keys(), vec![selected.clone()]);
        assert!(
            h.app.views[id].browser.folder_browsers[&h.pane]
                .rel_path
                .as_os_str()
                .is_empty()
        );
        assert!(
            visible
                .get(&selected)
                .is_some_and(|position| (0.0..200.0).contains(&position.y)),
            "selected folder should be visible: {visible:?}"
        );
        let owner = h
            .ctx
            .data(|data| data.get_temp::<(egui::Id, egui::Rect)>(browser_selection_owner_id()))
            .unwrap()
            .0;
        assert_eq!(h.ctx.memory(|memory| memory.focused()), Some(owner));
        assert_eq!(h.app.views[id].browser.folder_browsers.len(), 1);
    }
}
