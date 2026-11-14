//! The content explorer: browsing a tag's references and referrers, with
//! history.

use super::*;

use crate::app::browser::bulk_rename::{draw_links_box, draw_tag_band, padded_body};

/// Each list's box height: about a dozen rows, then it scrolls.
const EXPLORER_BOX_HEIGHT: f32 = 380.0;
/// The tooltip line under a row's path.
const EXPLORE_HINT: &str = "Click to explore this tag's references";

/// The Content Explorer window, while it is open.
impl Dialog for ContentExplorer {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let mut open = true;
        let mut act: Option<ExplorerAct> = None;
        let explorer_kit = self.kit;
        let explorer_kit_index = cx
            .model
            .resolve_kit(explorer_kit)
            .unwrap_or(cx.model.active);
        let mut filter = self.filter.clone();
        let game = cx.model.kits[explorer_kit_index].source.as_ref().and_then(|source| source.source.game());
        {
            let explorer = &*self;
            egui::Window::new("Content Explorer")
                .constrain_to(window_work_area(ctx))
                .id(egui::Id::unique("content_explorer"))
                .open(&mut open)
                .default_width(window_width(ctx, 720.0))
                .show(ctx, |ui| {
                    // The tag being explored, on the band the Move and Rename
                    // windows show their folder on.
                    draw_tag_band(ui, explorer.focus.group_tag, game, &explorer.focus.display_path);
                    ui.add_space(10.0);
                    padded_body(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(!explorer.back.is_empty(), egui::Button::new("← Back"))
                                .clicked()
                            {
                                act = Some(ExplorerAct::Back);
                            }
                            if ui
                                .add_enabled(
                                    !explorer.forward.is_empty(),
                                    egui::Button::new("Forward →"),
                                )
                                .clicked()
                            {
                                act = Some(ExplorerAct::Forward);
                            }
                            ui.separator();
                            if ui.button("Open in editor").clicked() {
                                act = Some(ExplorerAct::Open(explorer.focus.key.clone()));
                            }
                            if ui.button("Reveal in browser").clicked() {
                                act = Some(ExplorerAct::Reveal(explorer.focus.key.clone()));
                            }
                            ui.separator();
                            ui.add(
                                crate::app::ui_kit::singleline_text_edit(&mut filter)
                                    .hint_text(placeholder_text("filter"))
                                    .desired_width(140.0),
                            );
                        });
                        if explorer.index_unavailable {
                            let note = if cx.model.kits[cx.model.active]
                                .index_jobs
                                .building_references
                                || cx.model.kits[explorer_kit_index].scanning_entries
                            {
                                "Reference index is building — reopen this in a moment."
                            } else {
                                "Reference index unavailable — run Tools → Build Reference Index."
                            };
                            ui.add_space(6.0);
                            ui.label(RichText::new(note).color(subtle_dark()));
                        }
                        ui.add_space(12.0);
                        let query = filter.trim();
                        let shown = |entries: &[TagEntry]| -> Vec<TagEntry> {
                            entries
                                .iter()
                                .filter(|entry| contains_ignore_ascii_case(&entry.display_path, query))
                                .cloned()
                                .collect()
                        };
                        let parents = shown(&explorer.parents);
                        let children = shown(&explorer.children);
                        let gap = 20.0;
                        let column = ((ui.available_width() - gap) / 2.0).floor();
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for (id, title, list, total, empty) in [
                                ("ce_parents", "Referenced By", &parents, explorer.parents.len(), "No tags reference this tag."),
                                ("ce_children", "References", &children, explorer.children.len(), "This tag references no other tags."),
                            ] {
                                ui.allocate_ui_with_layout(
                                    Vec2::new(column, EXPLORER_BOX_HEIGHT),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_width(column);
                                        let rows: Vec<(String, u32)> =
                                            list.iter().map(|entry| (entry.display_path.clone(), entry.group_tag)).collect();
                                        let count = if list.len() == total {
                                            format!("({total})")
                                        } else {
                                            format!("({} of {total})", list.len())
                                        };
                                        let notes = match (list.len(), total) {
                                            (0, 0) => vec![empty.to_owned()],
                                            (0, _) => vec!["None match the filter.".to_owned()],
                                            _ => Vec::new(),
                                        };
                                        if let Some(row) = draw_links_box(
                                            ui,
                                            id,
                                            &format!("{title} {count}"),
                                            &notes,
                                            &rows,
                                            EXPLORE_HINT,
                                            game,
                                            EXPLORER_BOX_HEIGHT,
                                        ) {
                                            act = Some(ExplorerAct::Navigate(list[row].clone()));
                                        }
                                    },
                                );
                            }
                        });
                    });
                    ui.add_space(8.0);
                });
        }
        self.filter = filter;
        if let Some(act) = act {
            cx.send(ReferencesCommand::Explorer {
                kit: explorer_kit,
                act,
            });
        }
        open
    }
}

impl Baboon {
    /// Carry out what the Content Explorer over `kit` asked for. The graph
    /// belongs to one kit; go back to it before acting, and close the window
    /// if that kit has gone.
    pub(in crate::app) fn apply_explorer_act(&mut self, kit: KitId, act: ExplorerAct, ctx: &egui::Context) {
        if !self.focus_navigation_kit(kit) {
            self.dialogs.close::<ContentExplorer>();
            self.model.status = "That workspace has been closed".to_owned();
            return;
        }
        match act {
            ExplorerAct::Navigate(entry) => self.content_explorer_navigate(entry),
            ExplorerAct::Back => self.content_explorer_back(),
            ExplorerAct::Forward => self.content_explorer_forward(),
            ExplorerAct::Open(key) => self.select_entry(key, ctx.clone()),
            ExplorerAct::Reveal(key) => self.reveal_in_browser(&key),
        }
    }
}

/// What the Content Explorer can ask for.
pub(in crate::app) enum ExplorerAct {
    Navigate(TagEntry),
    Back,
    Forward,
    Open(String),
    Reveal(String),
}

impl Baboon {






    /// Open the Content Explorer centered on `key`.
    pub(in crate::app) fn open_content_explorer(&mut self, key: &str) {
        let Some(focus) = self.model.entry_for_key(key).cloned() else {
            return;
        };
        let (parents, parents_unavailable) = match self.model.references_to_entry(&focus) {
            Some(parents) => (parents, false),
            None => (Vec::new(), true),
        };
        let (children, children_unavailable) = self.model.children_of_entry(key);
        self.dialogs.open(ContentExplorer {
            kit: self.model.active_kit_id(),
            focus,
            parents,
            children,
            filter: String::new(),
            index_unavailable: parents_unavailable && children_unavailable,
            back: Vec::new(),
            forward: Vec::new(),
        });
    }

    /// Re-center the open Content Explorer on `entry`, recording history.
    pub(in crate::app) fn content_explorer_navigate(&mut self, entry: TagEntry) {
        let key = entry.key.clone();
        let (parents, parents_unavailable) = match self.model.references_to_entry(&entry) {
            Some(parents) => (parents, false),
            None => (Vec::new(), true),
        };
        let (children, children_unavailable) = self.model.children_of_entry(&key);
        if let Some(explorer) = self.dialogs.get_mut::<ContentExplorer>() {
            explorer.back.push(explorer.focus.clone());
            explorer.forward.clear();
            explorer.focus = entry;
            explorer.parents = parents;
            explorer.children = children;
            explorer.index_unavailable = parents_unavailable && children_unavailable;
        }
    }

    pub(in crate::app) fn content_explorer_back(&mut self) {
        let Some(prev) = self
            .dialogs
            .get_mut::<ContentExplorer>()
            .and_then(|explorer| explorer.back.pop())
        else {
            return;
        };
        self.recenter_explorer(prev, true);
    }

    pub(in crate::app) fn content_explorer_forward(&mut self) {
        let Some(next) = self
            .dialogs
            .get_mut::<ContentExplorer>()
            .and_then(|explorer| explorer.forward.pop())
        else {
            return;
        };
        self.recenter_explorer(next, false);
    }

    /// Re-center without clearing history; pushes the current focus onto the
    /// opposite stack (used by back/forward).
    pub(in crate::app) fn recenter_explorer(&mut self, entry: TagEntry, going_back: bool) {
        let key = entry.key.clone();
        let (parents, parents_unavailable) = match self.model.references_to_entry(&entry) {
            Some(parents) => (parents, false),
            None => (Vec::new(), true),
        };
        let (children, children_unavailable) = self.model.children_of_entry(&key);
        if let Some(explorer) = self.dialogs.get_mut::<ContentExplorer>() {
            let current = std::mem::replace(&mut explorer.focus, entry);
            if going_back {
                explorer.forward.push(current);
            } else {
                explorer.back.push(current);
            }
            explorer.parents = parents;
            explorer.children = children;
            explorer.index_unavailable = parents_unavailable && children_unavailable;
        }
    }

    pub(in crate::app) fn show_references_for(&mut self, key: &str) {
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            return;
        };
        // Fresh query — drop any expander state from a previous references popup.
        let title = format!("References to {}", entry.display_path);
        // The referenced tag's dependency path, so a clicked row can jump to the
        // exact field that points here.
        let ref_target =
            dependency_entry_reference_path(&entry, self.model.names()).map(|rel| (entry.group_tag, rel));
        match self.model.references_to_entry(&entry) {
            Some(entries) => {
                let note = entries
                    .is_empty()
                    .then(|| "No other tags reference this tag.".to_owned());
                self.dialogs.open(QueryResultsWindow::new(TagQueryResults {
                    kit: self.model.active_kit_id(),
                    title,
                    entries,
                    annotations: Vec::new(),
                    note,
                    ref_target,
                }));
            }
            None => {
                self.dialogs.open(QueryResultsWindow::new(TagQueryResults {
                    kit: self.model.active_kit_id(),
                    title,
                    entries: Vec::new(),
                    annotations: Vec::new(),
                    note: Some(self.model.reference_index_unavailable_note()),
                    ref_target: None,
                }));
            }
        }
    }
}

impl Model {
    pub(in crate::app) fn references_to_entry(&self, entry: &TagEntry) -> Option<Vec<TagEntry>> {
        let source = self.source()?;
        let index = source.reverse_dependencies.as_ref()?;
        let rel = dependency_entry_reference_path(entry, self.names())?;
        let referrer_keys = index
            .dependents_for(entry.group_tag, &rel)
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut out: Vec<TagEntry> = source
            .full_entry_set()
            .iter()
            .filter(|entry| referrer_keys.contains(entry.key.as_str()))
            .cloned()
            .collect();
        out.sort_by_cached_key(|entry| crate::core::source::natural_key(&entry.display_path));
        Some(out)
    }

    /// All tags that nothing references (orphans / roots). `None` when no index
    /// is available.
    pub(in crate::app) fn unreferenced_entries(&self) -> Option<Vec<TagEntry>> {
        let source = self.source()?;
        let index = source.reverse_dependencies.as_ref()?;
        let mut out: Vec<TagEntry> = source
            .full_entry_set()
            .iter()
            .filter(|entry| {
                dependency_entry_reference_path(entry, self.names())
                    .map(|rel| index.dependents_for(entry.group_tag, &rel).is_empty())
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        out.sort_by_cached_key(|entry| crate::core::source::natural_key(&entry.display_path));
        Some(out)
    }

    /// Resolve the dependencies a tag declares (children) into browseable
    /// entries, via a one-shot dependency-key → entry lookup over all entries.
    pub(in crate::app) fn children_of_entry(&self, key: &str) -> (Vec<TagEntry>, bool) {
        let Some(source) = self.source() else {
            return (Vec::new(), true);
        };
        let Some(index) = source.reverse_dependencies.as_ref() else {
            return (Vec::new(), true);
        };
        let deps = index.dependencies_of(key);
        let mut by_key: HashMap<String, &TagEntry> = HashMap::new();
        for entry in source.full_entry_set() {
            if let Some(rel) = dependency_entry_reference_path(entry, self.names()) {
                by_key
                    .entry(crate::core::source::dependency_key(entry.group_tag, &rel))
                    .or_insert(entry);
            }
        }
        let mut children: Vec<TagEntry> = deps
            .iter()
            .filter_map(|dep| {
                by_key
                    .get(&crate::core::source::dependency_key(dep.group_tag, &dep.rel_path))
                    .map(|entry| (*entry).clone())
            })
            .collect();
        children.sort_by_cached_key(|entry| crate::core::source::natural_key(&entry.display_path));
        children.dedup_by(|a, b| a.key == b.key);
        (children, false)
    }
}
