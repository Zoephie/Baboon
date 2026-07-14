//! Search, query, content-explorer, and structural-diff windows.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

use super::*;

impl Baboon {
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
                                .hint_text("filter")
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
                        let note = if self.building_reverse_dependencies || self.scanning_entries {
                            "Reference index is building — reopen this in a moment."
                        } else {
                            "Reference index unavailable — run Tools → Build Reference Index."
                        };
                        ui.label(RichText::new(note).color(subtle_dark()));
                    }
                    ui.separator();
                    let filter_lower = filter.trim().to_ascii_lowercase();
                    let matches = |entry: &TagEntry| {
                        filter_lower.is_empty()
                            || entry
                                .display_path
                                .to_ascii_lowercase()
                                .contains(&filter_lower)
                    };
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
                        egui::ScrollArea::vertical()
                            .id_salt("ce_parents")
                            .max_height(380.0)
                            .show(&mut cols[0], |ui| {
                                if parents.is_empty() {
                                    ui.label(RichText::new("(none)").color(subtle_dark()));
                                }
                                for entry in &parents {
                                    if explorer_entry_row(ui, entry) {
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
                        egui::ScrollArea::vertical()
                            .id_salt("ce_children")
                            .max_height(380.0)
                            .show(&mut cols[1], |ui| {
                                if children.is_empty() {
                                    ui.label(RichText::new("(none)").color(subtle_dark()));
                                }
                                for entry in &children {
                                    if explorer_entry_row(ui, entry) {
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
        self.source
            .as_ref()
            .and_then(|source| source.game.as_deref())
    }

    pub(in crate::app) fn source_tags_root(&self) -> Option<&std::path::Path> {
        self.source
            .as_ref()
            .and_then(|source| match &source.source {
                TagSource::LooseFolder { root, .. } => Some(root.as_path()),
                _ => None,
            })
    }

    pub(in crate::app) fn source_definitions_root(&self) -> Option<&std::path::Path> {
        self.source
            .as_ref()
            .and_then(|source| match &source.source {
                TagSource::LooseFolder {
                    definitions_root, ..
                } => Some(definitions_root.as_path()),
                _ => None,
            })
    }

    pub(super) fn draw_tag_diff_window(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.tag_diff.take() else {
            return;
        };
        let a_group = self
            .parsed_tags
            .get(&state.a_key)
            .map(|doc| doc.tag.group().tag);
        let mut open = true;
        let mut compute = false;
        let mut browse = false;
        egui::Window::new("Compare Tags")
            .id(egui::Id::new("tag_diff_window"))
            .open(&mut open)
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("A:").strong().color(text_dark()));
                    ui.monospace(state.a_key.replace('\\', "/"));
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("B:").strong().color(text_dark()));
                    let selected = state
                        .b_display
                        .clone()
                        .map(|k| k.replace('\\', "/"))
                        .unwrap_or_else(|| "(open tag)".to_owned());
                    let mut keys: Vec<String> = self
                        .parsed_tags
                        .keys()
                        .filter(|k| {
                            **k != state.a_key
                                && self.parsed_tags.get(*k).map(|d| d.tag.group().tag) == a_group
                        })
                        .cloned()
                        .collect();
                    keys.sort();
                    let (_, wheel_delta) = combo_box_with_scroll(
                        ui,
                        egui::ComboBox::from_id_salt("tag_diff_b")
                            .selected_text(selected)
                            .width(380.0),
                        |ui| {
                            for key in &keys {
                                if ui
                                    .selectable_label(
                                        state.b_key.as_deref() == Some(key.as_str()),
                                        key.replace('\\', "/"),
                                    )
                                    .clicked()
                                {
                                    state.b_key = Some(key.clone());
                                    state.b_display = Some(key.clone());
                                    state.results = None;
                                }
                            }
                        },
                    );
                    if let Some(delta) = wheel_delta {
                        let current = state
                            .b_key
                            .as_ref()
                            .and_then(|selected| keys.iter().position(|key| key == selected))
                            .unwrap_or(0);
                        if let Some(next) = combo_scroll_next_index(current, keys.len(), delta) {
                            let key = keys[next].clone();
                            state.b_key = Some(key.clone());
                            state.b_display = Some(key);
                            state.results = None;
                        }
                    }
                    if ui
                        .add_enabled(state.b_key.is_some(), egui::Button::new("Compare"))
                        .clicked()
                    {
                        compute = true;
                    }
                    if ui
                        .button("Browse file…")
                        .on_hover_text("Pick any tag of the same group from disk")
                        .clicked()
                    {
                        browse = true;
                    }
                });
                ui.label(
                    RichText::new("Compares field-by-field against an open tab or a tag on disk.")
                        .small()
                        .color(subtle_dark()),
                );

                if let Some(results) = &state.results {
                    ui.separator();
                    if results.diffs.is_empty() {
                        ui.label(RichText::new("No differences.").color(subtle_dark()));
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} differing field(s){}",
                                    results.diffs.len(),
                                    if results.truncated { " (capped)" } else { "" }
                                ))
                                .color(subtle_dark())
                                .small(),
                            );
                            if icon_text_button(ui, ButtonIcon::Copy, "Copy", true)
                                .on_hover_text("Copy the diff as tab-separated rows")
                                .clicked()
                            {
                                let text = std::iter::once("field\tA\tB".to_owned())
                                    .chain(
                                        results
                                            .diffs
                                            .iter()
                                            .map(|d| format!("{}\t{}\t{}", d.path, d.a, d.b)),
                                    )
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                ui.output_mut(|output| output.copied_text = text);
                            }
                        });
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .max_height(460.0)
                            .show(ui, |ui| {
                                egui::Grid::new("tag_diff_grid")
                                    .num_columns(3)
                                    .striped(true)
                                    .show(ui, |ui| {
                                        for diff in &results.diffs {
                                            ui.label(RichText::new(&diff.path).monospace().small());
                                            ui.label(RichText::new(&diff.a).color(text_dark()));
                                            ui.label(RichText::new(&diff.b).color(text_dark()));
                                            ui.end_row();
                                        }
                                    });
                            });
                    }
                }
            });

        if compute {
            if let Some(b_key) = state.b_key.clone() {
                let names = TagNameIndex::default();
                let diff = match (
                    self.parsed_tags.get(&state.a_key),
                    self.parsed_tags.get(&b_key),
                ) {
                    (Some(a), Some(b)) => Some(diff_tags(&a.tag, &b.tag, &names, 5000)),
                    _ => None,
                };
                if let Some((diffs, truncated)) = diff {
                    state.results = Some(TagDiffResults { diffs, truncated });
                }
            }
        }
        if browse {
            if let Some(group) = a_group {
                let ext = group_tag_to_extension(group).unwrap_or("");
                let mut dialog = rfd::FileDialog::new().set_title("Pick tag B to compare");
                if !ext.is_empty() {
                    dialog = dialog.add_filter(ext, &[ext]);
                }
                if let Some(root) = self.source_tags_root() {
                    dialog = dialog.set_directory(root);
                }
                if let Some(path) = dialog.pick_file() {
                    let game = self.source_game();
                    let definitions_root = self.source_definitions_root();
                    match crate::source::read_tag_at_path(&path, game, definitions_root, group) {
                        Ok(b_tag) => {
                            if let Some(a) = self.parsed_tags.get(&state.a_key) {
                                let names = TagNameIndex::default();
                                let (diffs, truncated) = diff_tags(&a.tag, &b_tag, &names, 5000);
                                state.b_key = None;
                                state.b_display = Some(path.display().to_string());
                                state.results = Some(TagDiffResults { diffs, truncated });
                            }
                        }
                        Err(error) => {
                            self.status =
                                format!("Compare: could not load {}: {error}", path.display());
                        }
                    }
                }
            }
        }
        if open {
            self.tag_diff = Some(state);
        }
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
                                .map(|entry| entry.display_path.replace('\\', "/"))
                                .collect::<Vec<_>>()
                                .join("\n");
                            ui.output_mut(|output| output.copied_text = text);
                        }
                    });
                    ui.separator();
                    // A references popup lets each row expand to its per-occurrence
                    // list; other query kinds render a plain clickable row.
                    let expandable = results.ref_target.is_some();
                    egui::ScrollArea::vertical()
                        .max_height(460.0)
                        .show(ui, |ui| {
                            for (index, entry) in results.entries.iter().enumerate() {
                                let path = entry.display_path.replace('\\', "/");
                                let label = match results.annotations.get(index) {
                                    Some(annotation) => format!("{annotation}  —  {path}"),
                                    None => path,
                                };
                                let is_expanded = expanded.contains(&index);
                                let make_row = |ui: &mut egui::Ui| {
                                    ui.add(
                                        egui::Label::new(RichText::new(&label).color(text_dark()))
                                            .sense(Sense::click()),
                                    )
                                    .on_hover_text(
                                        "Click to jump to the first reference · right-click to reveal",
                                    )
                                };
                                let row = if expandable {
                                    ui.horizontal(|ui| {
                                        let arrow = if is_expanded { "▼" } else { "▶" };
                                        if ui
                                            .add(
                                                egui::Button::new(RichText::new(arrow).small())
                                                    .frame(false),
                                            )
                                            .on_hover_text("Show every field that references this tag")
                                            .clicked()
                                        {
                                            to_toggle.push(index);
                                        }
                                        make_row(ui)
                                    })
                                    .inner
                                } else {
                                    make_row(ui)
                                };
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
                                if expandable && is_expanded {
                                    match occurrences.get(&index) {
                                        Some(list) if !list.is_empty() => {
                                            for occ in list {
                                                ui.horizontal(|ui| {
                                                    ui.add_space(22.0);
                                                    let jump = icon_button(
                                                        ui,
                                                        ButtonIcon::JumpTo,
                                                        "Jump to this field",
                                                        true,
                                                        Vec2::new(22.0, 22.0),
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
                                                        to_jump = Some((
                                                            entry.key.clone(),
                                                            occ.field_path.clone(),
                                                        ));
                                                    }
                                                });
                                            }
                                        }
                                        Some(_) => {
                                            ui.horizontal(|ui| {
                                                ui.add_space(22.0);
                                                ui.label(
                                                    RichText::new("no direct field found")
                                                        .italics()
                                                        .color(subtle_dark())
                                                        .small(),
                                                );
                                            });
                                        }
                                        None => {
                                            ui.horizontal(|ui| {
                                                ui.add_space(22.0);
                                                ui.label(
                                                    RichText::new("loading…")
                                                        .italics()
                                                        .color(subtle_dark())
                                                        .small(),
                                                );
                                            });
                                        }
                                    }
                                }
                            }
                        });
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
                            .hint_text("value to find")
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
                            .hint_text("any (e.g. weap / weapon)")
                            .desired_width(180.0),
                    )
                    .on_hover_text("Optional: limit the search to a tag group (four-CC or name).");
                });
                ui.add_space(4.0);
                let indexed = self.field_index.is_ready_for(self.source_generation);
                ui.horizontal(|ui| {
                    if indexed {
                        ui.label(
                            RichText::new("● indexed — searches are instant")
                                .color(Color32::from_rgb(120, 170, 90))
                                .small(),
                        );
                    } else if self.field_index.is_building() {
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
