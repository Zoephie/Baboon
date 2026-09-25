//! Search, query, content-explorer, and structural-diff windows.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

impl Baboon {
    pub(super) fn draw_tag_reference_picker_window(&mut self, ctx: &egui::Context) {
        if self.tag_reference_picker.is_none() {
            return;
        }
        let expert_mode = self.expert_mode;
        // The catalog has to come from the kit the picker was opened from, the
        // same kit its selection is applied to — otherwise it would offer
        // another game's tags to pick from.
        let picker_kit = self
            .tag_reference_picker_kit
            .and_then(|kit| self.resolve_kit(kit))
            .unwrap_or(self.active);
        let Some(catalog) = self.kits[picker_kit]
            .source
            .as_ref()
            .and_then(|source| tag_reference_catalog_for_source(source, expert_mode))
        else {
            self.tag_reference_picker = None;
            return;
        };

        let mut open = true;
        let mut picked = None;
        {
            let picker = self
                .tag_reference_picker
                .as_mut()
                .expect("picker presence checked above");
            egui::Window::new("Select Tag Reference")
                .id(egui::Id::new(
                    "campaign_evolved_tag_reference_picker_window",
                ))
                .open(&mut open)
                .movable(true)
                .resizable(true)
                .collapsible(false)
                .default_size(Vec2::new(620.0, 420.0))
                .min_size(Vec2::new(420.0, 220.0))
                .show(ctx, |ui| {
                    picked = draw_tag_reference_catalog_picker_contents(
                        ui,
                        egui::Id::new("campaign_evolved_tag_reference_picker_contents"),
                        catalog,
                        &picker.allowed_groups,
                        picker.current_group,
                        &mut picker.search,
                    );
                });
        }

        if let Some(input) = picked {
            let picker = self
                .tag_reference_picker
                .take()
                .expect("picker remains open while processing selection");
            let kit = picker_kit;
            if self.kits[kit].parsed_tags.contains_key(&picker.tag_key) {
                let ops = DeferredOps {
                    pending: vec![PendingFieldEdit {
                        path: picker.field_path.clone(),
                        input: input.clone(),
                    }],
                    ..DeferredOps::default()
                };
                let applied = self.apply_doc_ops(
                    kit,
                    &picker.tag_key,
                    "Change tag reference",
                    ops,
                    UndoStep::Own,
                );
                if applied.is_some() {
                    // `insert_clean` from upstream: the picked reference is
                    // now the document's value, so the draft starts
                    // unmodified rather than looking like an uncommitted edit.
                    self.kits[kit]
                        .edit_buffers
                        .insert_clean(format!("{}|{}", picker.tag_key, picker.field_path), input);
                    self.invalidate_tag_caches_in(kit, &picker.tag_key);
                }
            } else {
                self.status = "The tag being edited is no longer open".to_owned();
            }
        } else if !open {
            self.tag_reference_picker = None;
        }
    }

    pub(super) fn draw_content_explorer_window(&mut self, ctx: &egui::Context) {
        if self.content_explorer.is_none() {
            return;
        }
        enum ExplorerAct {
            Navigate(TagEntry),
            Back,
            Forward,
            Open(String),
            Reveal(String),
        }
        let mut open = true;
        let mut act: Option<ExplorerAct> = None;
        let explorer_kit = self
            .content_explorer
            .as_ref()
            .map(|explorer| explorer.kit)
            .expect("checked above");
        let explorer_kit_index = self.resolve_kit(explorer_kit).unwrap_or(self.active);
        let mut filter = self
            .content_explorer
            .as_ref()
            .map(|explorer| explorer.filter.clone())
            .unwrap_or_default();
        {
            let explorer = self.content_explorer.as_ref().expect("checked above");
            egui::Window::new("Content Explorer")
                .id(egui::Id::new("content_explorer"))
                .open(&mut open)
                .default_width(720.0)
                .show(ctx, |ui| {
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
                            egui::TextEdit::singleline(&mut filter)
                                .hint_text(placeholder_text("filter"))
                                .desired_width(140.0),
                        );
                    });
                    ui.separator();
                    ui.label(
                        RichText::new(explorer.focus.display_path.replace('\\', "/"))
                            .strong()
                            .color(text_dark()),
                    );
                    if explorer.index_unavailable {
                        let note = if self.kits[self.active].index_jobs.building_references
                            || self.kits[explorer_kit_index].scanning_entries
                        {
                            "Reference index is building — reopen this in a moment."
                        } else {
                            "Reference index unavailable — run Tools → Build Reference Index."
                        };
                        ui.label(RichText::new(note).color(subtle_dark()));
                    }
                    ui.separator();
                    let query = filter.trim();
                    let matches =
                        |entry: &TagEntry| contains_ignore_ascii_case(&entry.display_path, query);
                    let parents: Vec<&TagEntry> =
                        explorer.parents.iter().filter(|e| matches(e)).collect();
                    let children: Vec<&TagEntry> =
                        explorer.children.iter().filter(|e| matches(e)).collect();
                    let count_label = |shown: usize, total: usize| {
                        if shown == total {
                            format!("({total})")
                        } else {
                            format!("({shown}/{total})")
                        }
                    };
                    ui.columns(2, |cols| {
                        cols[0].label(
                            RichText::new(format!(
                                "Referenced by {}",
                                count_label(parents.len(), explorer.parents.len())
                            ))
                            .strong()
                            .color(text_dark()),
                        );
                        if parents.is_empty() {
                            cols[0].label(RichText::new("(none)").color(subtle_dark()));
                        }
                        // A widely used tag has thousands of referrers: only
                        // the rows in view are drawn.
                        let row_height = cols[0].spacing().interact_size.y;
                        egui::ScrollArea::vertical()
                            .id_salt("ce_parents")
                            .max_height(380.0)
                            .show_rows(&mut cols[0], row_height, parents.len(), |ui, rows| {
                                for entry in &parents[rows] {
                                    if fixed_height_row(ui, row_height, |ui| {
                                        explorer_entry_row(ui, entry)
                                    }) {
                                        act = Some(ExplorerAct::Navigate((*entry).clone()));
                                    }
                                }
                            });
                        cols[1].label(
                            RichText::new(format!(
                                "References {}",
                                count_label(children.len(), explorer.children.len())
                            ))
                            .strong()
                            .color(text_dark()),
                        );
                        if children.is_empty() {
                            cols[1].label(RichText::new("(none)").color(subtle_dark()));
                        }
                        // A widely used tag has thousands of referrers: only
                        // the rows in view are drawn.
                        let row_height = cols[1].spacing().interact_size.y;
                        egui::ScrollArea::vertical()
                            .id_salt("ce_children")
                            .max_height(380.0)
                            .show_rows(&mut cols[1], row_height, children.len(), |ui, rows| {
                                for entry in &children[rows] {
                                    if fixed_height_row(ui, row_height, |ui| {
                                        explorer_entry_row(ui, entry)
                                    }) {
                                        act = Some(ExplorerAct::Navigate((*entry).clone()));
                                    }
                                }
                            });
                    });
                });
        }
        if let Some(explorer) = self.content_explorer.as_mut() {
            explorer.filter = filter;
        }
        match act {
            // The graph belongs to one kit; go back to it before acting, and
            // close the window if that kit has gone.
            Some(_) if !self.focus_navigation_kit(explorer_kit) => {
                self.content_explorer = None;
                self.status = "That workspace has been closed".to_owned();
            }
            Some(ExplorerAct::Navigate(entry)) => self.content_explorer_navigate(entry),
            Some(ExplorerAct::Back) => self.content_explorer_back(),
            Some(ExplorerAct::Forward) => self.content_explorer_forward(),
            Some(ExplorerAct::Open(key)) => self.select_entry(key, ctx.clone()),
            Some(ExplorerAct::Reveal(key)) => self.reveal_in_browser(&key),
            None => {}
        }
        if !open {
            self.content_explorer = None;
        }
    }

    /// Floating window listing the results of a tag query (find-references /
    /// unreferenced). Clicking an entry opens it.
    pub(in crate::app) fn source_game(&self) -> Option<&str> {
        self.source().and_then(|source| source.game.as_deref())
    }

    pub(in crate::app) fn source_tags_root(&self) -> Option<&std::path::Path> {
        self.source().and_then(|source| match &source.source {
            TagSource::LooseFolder { root, .. } => Some(root.as_path()),
            _ => None,
        })
    }

    pub(in crate::app) fn source_definitions_root(&self) -> Option<&std::path::Path> {
        self.source().and_then(|source| match &source.source {
            TagSource::LooseFolder {
                definitions_root, ..
            } => Some(definitions_root.as_path()),
            _ => None,
        })
    }

    pub(super) fn draw_query_results_window(&mut self, ctx: &egui::Context) {
        // Walk any expanded-but-uncached referrer rows before we take the results
        // (this reads `self.query_results`).
        self.refresh_ref_jump_occurrences(ctx);
        let Some(results) = self.query_results.take() else {
            return;
        };
        let mut open = true;
        let mut to_open: Option<String> = None;
        let mut to_reveal: Option<String> = None;
        let mut to_toggle: Vec<usize> = Vec::new();
        let mut to_jump: Option<(String, String)> = None;
        let expanded = &self.ref_jump_expanded;
        let occurrences = &self.ref_jump_occurrences;
        egui::Window::new(&results.title)
            .id(egui::Id::new("tag_query_results"))
            .open(&mut open)
            .default_width(440.0)
            .show(ctx, |ui| {
                if let Some(note) = &results.note {
                    ui.label(RichText::new(note).color(subtle_dark()));
                }
                if !results.entries.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{} tag(s)", results.entries.len()))
                                .color(subtle_dark())
                                .small(),
                        );
                        if ui
                            .small_button("Copy paths")
                            .on_hover_text("Copy all result tag paths (one per line)")
                            .clicked()
                        {
                            let text = results
                                .entries
                                .iter()
                                .map(|entry| {
                                    crate::format::to_native_path_string(&entry.display_path)
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            ui.output_mut(|output| output.copied_text = text);
                        }
                    });
                    ui.separator();
                    // A references popup lets each row expand to its per-occurrence
                    // list; other query kinds render a plain clickable row.
                    let expandable = results.ref_target.is_some();
                    // One line per row, entries and their expanded fields
                    // alike, so only the rows in view are drawn. A query over
                    // a whole source lists thousands of tags.
                    let rows = query_result_rows(results.entries.len(), expandable, |index| {
                        expanded
                            .contains(&index)
                            .then(|| occurrences.get(&index).map(Vec::len))
                    });
                    let row_height = ui.spacing().interact_size.y;
                    egui::ScrollArea::vertical().max_height(460.0).show_rows(
                        ui,
                        row_height,
                        rows.len(),
                        |ui, range| {
                            for row in &rows[range] {
                                #[cfg(test)]
                                tests::ROWS_BUILT.with(|built| built.set(built.get() + 1));
                                fixed_height_row(ui, row_height, |ui| match *row {
                                    QueryResultRow::Entry(index) => {
                                        let entry = &results.entries[index];
                                        let path = entry.display_path.replace('\\', "/");
                                        let label = match results.annotations.get(index) {
                                            Some(annotation) => format!("{annotation}  —  {path}"),
                                            None => path,
                                        };
                                        if expandable {
                                            let arrow = if expanded.contains(&index) {
                                                "▼"
                                            } else {
                                                "▶"
                                            };
                                            if ui
                                                .add(
                                                    egui::Button::new(RichText::new(arrow).small())
                                                        .frame(false),
                                                )
                                                .on_hover_text(
                                                    "Show every field that references this tag",
                                                )
                                                .clicked()
                                            {
                                                to_toggle.push(index);
                                            }
                                        }
                                        let row = ui
                                            .add(
                                                egui::Label::new(
                                                    RichText::new(&label).color(text_dark()),
                                                )
                                                .sense(Sense::click()),
                                            )
                                            .on_hover_text(
                                                "Click to jump to the first reference · \
                                                 right-click to reveal",
                                            );
                                        if row.clicked() {
                                            to_open = Some(entry.key.clone());
                                        }
                                        row.context_menu(|ui| {
                                            if ui.button("Open").clicked() {
                                                to_open = Some(entry.key.clone());
                                                ui.close_menu();
                                            }
                                            if ui.button("Reveal in browser").clicked() {
                                                to_reveal = Some(entry.key.clone());
                                                ui.close_menu();
                                            }
                                        });
                                    }
                                    QueryResultRow::Occurrence(index, position) => {
                                        let entry = &results.entries[index];
                                        let occ = &occurrences[&index][position];
                                        ui.add_space(22.0);
                                        let jump = icon_button(
                                            ui,
                                            ButtonIcon::JumpTo,
                                            "Jump to this field",
                                            true,
                                            text_dark(),
                                        );
                                        let label = ui
                                            .add(
                                                egui::Label::new(
                                                    RichText::new(format!("↳ {}", occ.label))
                                                        .color(subtle_dark()),
                                                )
                                                .sense(Sense::click()),
                                            )
                                            .on_hover_text("Jump to this field");
                                        if jump.clicked() || label.clicked() {
                                            to_jump =
                                                Some((entry.key.clone(), occ.field_path.clone()));
                                        }
                                    }
                                    QueryResultRow::NoOccurrences(_) => {
                                        ui.add_space(22.0);
                                        ui.label(
                                            RichText::new("no direct field found")
                                                .italics()
                                                .color(subtle_dark())
                                                .small(),
                                        );
                                    }
                                    QueryResultRow::Loading(_) => {
                                        ui.add_space(22.0);
                                        ui.label(
                                            RichText::new("loading…")
                                                .italics()
                                                .color(subtle_dark())
                                                .small(),
                                        );
                                    }
                                });
                            }
                        },
                    );
                }
            });
        for index in to_toggle {
            if self.ref_jump_expanded.remove(&index) {
                // Collapsed — drop the cache so a re-expand re-reads fresh.
                self.ref_jump_occurrences.remove(&index);
            } else {
                self.ref_jump_expanded.insert(index);
            }
        }
        // Every row names a tag in the kit the query ran against, so go back to
        // that kit before acting. If it has closed the row is inert rather than
        // opening some unrelated tag that happens to share the key.
        let acting = to_jump.is_some() || to_open.is_some() || to_reveal.is_some();
        if acting && !self.focus_navigation_kit(results.kit) {
            self.status = "That workspace has been closed".to_owned();
            if open {
                self.query_results = Some(results);
            }
            return;
        }
        if let Some((key, field_path)) = to_jump {
            // The referrer is already loaded (we walked it for occurrences), so
            // focus it and navigate to the exact field directly.
            self.select_entry(key.clone(), ctx.clone());
            self.navigate_to_field(ctx, &key, &field_path);
        }
        if let Some(key) = to_open {
            // For a "References to X" result, queue a jump to the exact field in
            // the referrer that points at X (resolved once the tag loads).
            if let Some((group_tag, rel_path)) = &results.ref_target {
                self.pending_ref_jump = Some(PendingRefJump {
                    kit: self.active_kit_id(),
                    tag_key: key.clone(),
                    group_tag: *group_tag,
                    rel_path: rel_path.clone(),
                });
            }
            self.select_entry(key, ctx.clone());
        }
        if let Some(key) = to_reveal {
            self.reveal_in_browser(&key);
        }
        // Keep the window's results until it is closed.
        if open {
            self.query_results = Some(results);
        }
    }

    pub(super) fn draw_field_value_search_window(&mut self, ctx: &egui::Context) {
        if !self.field_value_search_open {
            return;
        }
        let mut open = true;
        let mut do_search = false;
        let mut do_build = false;
        egui::Window::new("Search Field Values")
            .id(egui::Id::new("field_value_search"))
            .open(&mut open)
            .default_width(400.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Find tags whose field values contain text — strings, string IDs, tag \
                         references, and enum names.",
                    )
                    .color(subtle_dark()),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let response = ui.add_enabled(
                        !self.field_value_searching,
                        egui::TextEdit::singleline(&mut self.field_value_query)
                            .hint_text(placeholder_text("value to find"))
                            .desired_width(240.0),
                    );
                    let submitted =
                        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if self.field_value_searching {
                        ui.spinner();
                        ui.label(RichText::new("searching…").color(subtle_dark()));
                    } else if icon_text_button(ui, ButtonIcon::Search, "Search", true).clicked()
                        || submitted
                    {
                        do_search = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("group").color(subtle_dark()).small());
                    ui.add(
                        egui::TextEdit::singleline(&mut self.field_value_group)
                            .hint_text(placeholder_text("any (e.g. weap / weapon)"))
                            .desired_width(180.0),
                    )
                    .on_hover_text("Optional: limit the search to a tag group (four-CC or name).");
                });
                ui.add_space(4.0);
                let indexed = self.kits[self.active]
                    .field_index
                    .is_ready_for(self.kits[self.active].generation);
                ui.horizontal(|ui| {
                    if indexed {
                        ui.label(
                            RichText::new("• indexed — searches are instant")
                                .color(Color32::from_rgb(120, 170, 90))
                                .small(),
                        );
                    } else if self.kits[self.active].field_index.is_building() {
                        ui.spinner();
                        ui.label(
                            RichText::new("building index…")
                                .color(subtle_dark())
                                .small(),
                        );
                    } else {
                        ui.label(
                            RichText::new("not indexed — first search scans live")
                                .color(subtle_dark())
                                .small(),
                        );
                        if ui.small_button("Build index").clicked() {
                            do_build = true;
                        }
                    }
                });
            });
        if do_search && !self.field_value_query.trim().is_empty() {
            self.begin_field_value_search(ctx.clone());
        }
        if do_build {
            self.begin_build_field_index(ctx.clone());
        }
        self.field_value_search_open = open;
    }
}

/// One line of the query results window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueryResultRow {
    Entry(usize),
    /// An expanded entry's field, by position in its occurrence list.
    Occurrence(usize, usize),
    /// An expanded entry whose walk found no field.
    NoOccurrences(usize),
    /// An expanded entry whose walk has not finished.
    Loading(usize),
}

/// The results as lines. `expansion(index)` is `None` for a collapsed entry,
/// `Some(None)` for one still loading, and `Some(Some(count))` for one whose
/// fields are known.
fn query_result_rows(
    entries: usize,
    expandable: bool,
    expansion: impl Fn(usize) -> Option<Option<usize>>,
) -> Vec<QueryResultRow> {
    let mut rows = Vec::with_capacity(entries);
    for index in 0..entries {
        rows.push(QueryResultRow::Entry(index));
        if !expandable {
            continue;
        }
        match expansion(index) {
            None => {}
            Some(None) => rows.push(QueryResultRow::Loading(index)),
            Some(Some(0)) => rows.push(QueryResultRow::NoOccurrences(index)),
            Some(Some(count)) => {
                rows.extend((0..count).map(|position| QueryResultRow::Occurrence(index, position)))
            }
        }
    }
    rows
}

/// Lay a row out left to right at exactly `height`, so a `show_rows` list
/// stays where its row count says it is.
fn fixed_height_row<R>(ui: &mut Ui, height: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::left_to_right(egui::Align::Center),
        add,
    )
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Result rows laid out, to tell a virtualized list from one that
        /// lays out every row and lets egui cull the painting.
        pub(super) static ROWS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    #[test]
    fn an_expanded_result_lists_its_fields_under_it() {
        use QueryResultRow::*;
        let expansion = |index: usize| match index {
            1 => Some(None),
            2 => Some(Some(0)),
            3 => Some(Some(2)),
            _ => None,
        };
        assert_eq!(
            query_result_rows(5, true, expansion),
            [
                Entry(0),
                Entry(1),
                Loading(1),
                Entry(2),
                NoOccurrences(2),
                Entry(3),
                Occurrence(3, 0),
                Occurrence(3, 1),
                Entry(4),
            ]
        );
        assert_eq!(
            query_result_rows(2, false, expansion),
            [Entry(0), Entry(1)],
            "only a references query expands"
        );
    }

    /// The window draws the rows in view, not all of them.
    #[test]
    fn the_query_results_window_draws_only_rows_in_view() {
        let mut app = Baboon::for_test();
        let entries: Vec<TagEntry> = (0..5_000)
            .map(|index| TagEntry {
                key: format!("file:sound/row_{index}.sound"),
                display_path: format!("sound/row_{index}.sound"),
                group_tag: u32::from_be_bytes(*b"snd!"),
                group_name: Some("sound".to_owned()),
                location: TagEntryLocation::LooseFile(format!("sound/row_{index}.sound").into()),
            })
            .collect();
        let ctx = egui::Context::default();
        let mut painted = Vec::new();
        for _ in 0..2 {
            ROWS_BUILT.with(|built| built.set(0));
            app.query_results = Some(TagQueryResults {
                kit: app.kits[0].id,
                title: "Sounds".to_owned(),
                entries: entries.clone(),
                annotations: Vec::new(),
                note: None,
                ref_target: None,
            });
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.draw_query_results_window(ctx),
            );
            painted = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .filter(|text| text.starts_with("sound/row_"))
                .collect();
        }
        assert!(painted.contains(&"sound/row_0.sound".to_owned()));
        assert!(!painted.contains(&"sound/row_4999.sound".to_owned()));
        let built = ROWS_BUILT.with(std::cell::Cell::get);
        assert!(built < 100, "laid out {built} of 5,000 rows");
    }
}
