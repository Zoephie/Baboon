//! Decoded string-list Find targets, shared by the matcher and Strings Editor.

use super::*;
use blam_tags::string_list::{self as strings, StringList, TextSyntax};

pub(in crate::app) fn string_find_context_id(kit: KitId, key: &str) -> egui::Id {
    egui::Id::unique(("string find context", kit, key))
}

pub(in crate::app) fn string_find_jump_id(scope: &str, key: &str) -> egui::Id {
    egui::Id::unique(("string find jump", scope, key))
}

/// Match editor notation directly. A literal game glyph also searches both
/// its named spelling and its numeric fallback, with ranges in displayed text.
pub(in crate::app) fn string_find_ranges(
    text: &str,
    query: &str,
    syntax: &TextSyntax,
    match_case: bool,
    whole_word: bool,
) -> Vec<std::ops::Range<usize>> {
    let mut named = String::new();
    let mut numeric = String::new();
    for c in query.chars() {
        let raw = c.to_string();
        let formatted = strings::format_shown(&raw, syntax);
        if !c.is_ascii() && formatted.starts_with('&') {
            named.push_str(&formatted);
            let _ =
                std::fmt::Write::write_fmt(&mut numeric, format_args!("&U+{:04X}", u32::from(c)));
        } else {
            named.push(c);
            numeric.push(c);
        }
    }
    let mut ranges = find_text_ranges(text, &named, match_case, whole_word);
    if numeric != named {
        ranges.extend(find_text_ranges(text, &numeric, match_case, whole_word));
        ranges.sort_by_key(|range| (range.start, range.end));
        ranges.dedup();
    }
    ranges
}

pub(in crate::app) fn collect_string_find_occurrences(
    list: &StringList,
    context: &StringFindContext,
    options: &StringFindOptions,
    query: &str,
    match_case: bool,
    whole_word: bool,
    syntax: &TextSyntax,
) -> Vec<FindOccurrence> {
    let mut hits = Vec::new();
    for (index, entry) in list.entries.iter().enumerate() {
        if options.selected_rows && !context.selected.contains(&index) { continue; }
        if options.ids
            && let Some(id) = &entry.id
        {
            for range in find_text_ranges(id, query, match_case, whole_word) {
                hits.push(FindOccurrence {
                    tag_key: context.tag_key.clone(),
                    field_path: String::new(),
                    kind: FindTargetKind::StringId(index),
                    text: id.clone(),
                    range,
                });
            }
        }
        if !options.text {
            continue;
        }
        for (language, text) in entry.texts.iter().enumerate() {
            let include = match options.language {
                StringFindLanguage::Current => language == context.language,
                StringFindLanguage::All => true,
                StringFindLanguage::Language(selected) => language == selected,
            };
            if !include {
                continue;
            }
            let Some(text) = text else {
                continue;
            };
            let shown = strings::format_shown(text, syntax);
            for range in string_find_ranges(&shown, query, syntax, match_case, whole_word) {
                hits.push(FindOccurrence {
                    tag_key: context.tag_key.clone(),
                    field_path: String::new(),
                    kind: FindTargetKind::StringText { index, language },
                    text: shown.clone(),
                    range,
                });
            }
        }
    }
    hits
}

/// Map old indices to their new positions, only when the projected edits
/// exactly explain the resulting list. Failed/unknown edits use safe matching.
fn edited_row_mapping(before: &[strings::StringEntry], after: &[strings::StringEntry], ops: &crate::core::document::apply::DeferredOps) -> Option<Vec<Option<usize>>> {
    use strings::StringListEdit;
    use crate::core::document::ops::BlockOpKind;
    let mut projected: Vec<_> = before.iter().cloned().enumerate().map(|(index, row)| (Some(index), row)).collect();
    for op in &ops.string_list_ops {
        match &op.edit {
            StringListEdit::SetText { index, language, text } => {
                *projected.get_mut(*index)?.1.texts.get_mut(*language)? = (!text.is_empty()).then(|| text.clone());
            }
            StringListEdit::SetId { index, id } => projected.get_mut(*index)?.1.id = Some(id.clone()),
            StringListEdit::Insert { at, entries } => {
                if *at > projected.len() { return None; }
                projected.splice(*at..*at, entries.iter().cloned().map(|row| (None, row)));
            }
            StringListEdit::Remove { indices } => {
                if indices.iter().any(|&index| index >= projected.len()) { return None; }
                let removed: std::collections::BTreeSet<_> = indices.iter().copied().collect();
                projected = projected.into_iter().enumerate().filter_map(|(index, row)| (!removed.contains(&index)).then_some(row)).collect();
            }
        }
    }
    for op in &ops.block_ops {
        if !matches!(op.path.as_str(), "string references" | "strings" | "messages") { continue; }
        match &op.kind {
            BlockOpKind::Reorder { order } => {
                if order.len() != projected.len() || order.iter().collect::<std::collections::BTreeSet<_>>().len() != order.len() { return None; }
                projected = order.iter().map(|&index| projected.get(index).cloned()).collect::<Option<_>>()?;
            }
            _ => return None,
        }
    }
    if !projected.iter().map(|(_, row)| row).eq(after.iter()) { return None; }
    let mut mapping = vec![None; before.len()];
    for (new, (old, _)) in projected.into_iter().enumerate() { if let Some(old) = old { mapping[old] = Some(new); } }
    Some(mapping)
}

/// Undo, reloads and whole-document pastes have no edit-index mapping. Match
/// unique IDs, then unique complete rows; never guess between identical rows.
fn surviving_row_mapping(before: &[strings::StringEntry], after: &[strings::StringEntry]) -> Vec<Option<usize>> {
    use std::collections::BTreeMap;
    let index = |rows: &[strings::StringEntry], ids: bool| {
        let mut found: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (position, row) in rows.iter().enumerate() {
            let key = if ids { row.id.clone() } else { Some(format!("{row:?}")) };
            if let Some(key) = key { found.entry(key).or_default().push(position); }
        }
        found
    };
    let old_ids = index(before, true);
    let new_ids = index(after, true);
    let old_rows = index(before, false);
    let new_rows = index(after, false);
    before.iter().map(|row| {
        if let Some(id) = &row.id
            && old_ids.get(id).is_some_and(|rows| rows.len() == 1)
            && let Some(rows) = new_ids.get(id).filter(|rows| rows.len() == 1) { return Some(rows[0]); }
        let key = format!("{row:?}");
        (old_rows[&key].len() == 1).then(|| new_rows.get(&key).filter(|rows| rows.len() == 1).map(|rows| rows[0])).flatten()
    }).collect()
}

impl Baboon {
    pub(in crate::app) fn run_selected_string_find(&mut self, ctx: &egui::Context) {
        // Include selected rows hidden by the current results filter.
        if let Some(context) = &self.search.find.string_context
            && let Some(latest) = ctx.data(|data| data.get_temp::<StringFindContext>(string_find_context_id(context.kit, &context.tag_key))) {
            self.search.find.string_context = Some(latest);
        }
        let find = &self.search.find;
        let selected_rows = find.strings.as_ref().is_some_and(|options| options.selected_rows);
        if !selected_rows && !find.replace_mode { return; }
        let Some(context) = find.string_context.clone().filter(|context| (!selected_rows || !context.selected.is_empty()) && context.kit == self.model.active_kit_id()) else { return; };
        let Some(doc) = self.model.kits[self.model.active].parsed_tags.get(&context.tag_key) else { return; };
        let Some(list) = strings::read_string_list(&doc.tag) else { return; };
        self.search.find.selection_run = Some(StringSelectionRun {
            query: find.query(), context, document_stamp: format!("{:?}", doc.content_stamp()),
            document_id: doc.id, rows: list.entries, changed: false,
        });
        self.refresh_string_find();
        self.search.find.active = None;
        self.finish_find_refresh(ctx, None);
        self.capture_find_filter();
        self.publish_find_snapshot(ctx);
        if let Some(hit) = self.search.find.active_occurrence().cloned() { self.activate_find_occurrence(ctx, hit); }
    }

    /// Rebase captured results after a known edit, without running the query
    /// again. Explicit operation mappings also distinguish identical CE rows.
    pub(in crate::app) fn track_selected_string_find_edits(&mut self, kit: KitId, key: &str, ops: &crate::core::document::apply::DeferredOps) {
        let Some(run) = self.search.find.selection_run.as_ref().filter(|run| run.context.kit == kit && run.context.tag_key == key) else { return; };
        let Some(index) = self.model.kit_index(kit) else { return; };
        let Some(doc) = self.model.kits[index].parsed_tags.get(key) else { return; };
        if doc.id != run.document_id || format!("{:?}", doc.content_stamp()) == run.document_stamp { return; }
        let Some(list) = strings::read_string_list(&doc.tag) else { return; };
        let mapping = edited_row_mapping(&run.rows, &list.entries, ops);
        self.rebase_selected_string_find(list.entries, mapping, format!("{:?}", doc.content_stamp()));
    }

    pub(super) fn rebase_selected_string_find(&mut self, rows: Vec<strings::StringEntry>, mapping: Option<Vec<Option<usize>>>, stamp: String) {
        let find = &mut self.search.find;
        let Some(run) = &mut find.selection_run else { return; };
        let mapping = mapping.unwrap_or_else(|| surviving_row_mapping(&run.rows, &rows));
        if let Some(capture) = &mut find.replace_run {
            capture.matching_cells = std::sync::Arc::new(capture.matching_cells.iter().filter_map(|(key, path, kind)| {
                let index = mapping.get(kind.string_row()?).copied().flatten()?;
                let kind = match kind {
                    FindTargetKind::StringId(_) => FindTargetKind::StringId(index),
                    FindTargetKind::StringText { language, .. } => FindTargetKind::StringText { index, language: *language },
                    _ => return None,
                };
                Some((key.clone(), path.clone(), kind))
            }).collect());
            if let Some(filter) = &mut find.filter_run { filter.cells = capture.matching_cells.clone(); }
        }
        let syntax = crate::app::editor::string_list::tag_text_syntax(run.context.definitions_root.as_deref(), run.context.game, &run.context.tag_key);
        find.occurrences.retain_mut(|hit| {
            let Some(index) = hit.kind.string_row().and_then(|old| mapping.get(old).copied().flatten()) else { return false; };
            let Some(row) = rows.get(index) else { return false; };
            let text = match hit.kind {
                FindTargetKind::StringId(_) => {
                    hit.kind = FindTargetKind::StringId(index);
                    row.id.clone().unwrap_or_default()
                }
                FindTargetKind::StringText { language, .. } => {
                    hit.kind = FindTargetKind::StringText { index, language };
                    strings::format_shown(row.texts.get(language).and_then(Option::as_deref).unwrap_or_default(), &syntax)
                }
                _ => return false,
            };
            if hit.text != text {
                // Keep the captured row navigable, without an obsolete range.
                hit.text = text;
                hit.range = 0..0;
            }
            true
        });
        run.context.selected = run.context.selected.iter().filter_map(|&old| mapping.get(old).copied().flatten()).collect();
        run.rows = rows;
        run.document_stamp = stamp;
        run.changed = true;
        find.active = (!find.occurrences.is_empty()).then_some(0);
        find.matching_cells = std::sync::Arc::new(find.occurrences.iter().map(|hit| (hit.tag_key.clone(), hit.field_path.clone(), hit.kind)).collect());
        if find.replace_run.is_none() {
            if let Some(filter) = &mut find.filter_run { filter.cells = find.matching_cells.clone(); }
        }
    }

    pub(super) fn refresh_string_find(&mut self) {
        let Some(context) = &self.search.find.string_context else {
            return;
        };
        if context.kit != self.model.active_kit_id() {
            self.search.find.occurrences.clear();
            return;
        }
        let Some(doc) = self.model.kits[self.model.active]
            .parsed_tags
            .get(&context.tag_key)
        else {
            self.search.find.occurrences.clear();
            return;
        };
        let Some(list) = strings::read_string_list(&doc.tag) else {
            self.search.find.occurrences.clear();
            return;
        };
        let syntax = crate::app::editor::string_list::text_syntax(
            context.definitions_root.as_deref(),
            context.game,
        ).with_hud_messages(list.storage == strings::Storage::HudMessages);
        self.search.find.occurrences = collect_string_find_occurrences(
            &list,
            context,
            self.search.find.strings.as_ref().expect("string Find mode"),
            &self.search.find.query,
            self.search.find.match_case,
            self.search.find.whole_word,
            &syntax,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::{LooseKit, PromptDriver};
    use crate::core::bundled::locate_definitions_root;
    use blam_tags::string_list::{StringEntry, StringListEdit};

    fn context(key: &str, kit: KitId) -> StringFindContext {
        StringFindContext {
            selected: Default::default(),
            kit,
            tag_key: key.to_owned(),
            scope: "test".to_owned(),
            game: Some(GameId::Halo3),
            definitions_root: None,
            language: 0,
            languages: vec!["English".to_owned(), "French".to_owned()],
            has_ids: true,
            editor_active: true,
        }
    }

    fn fixture_tag() -> TagFile {
        let mut tag = TagFile::new(
            locate_definitions_root().join("halo3_mcc/multilingual_unicode_string_list.json"),
        )
        .unwrap();
        let entries = vec![
            StringEntry {
                id: Some("find_one".to_owned()),
                texts: vec![
                    Some("Find it twice: Find".to_owned()),
                    Some("Trouver".to_owned()),
                ],
            },
            StringEntry {
                id: Some("other".to_owned()),
                texts: vec![None, Some("Find en français".to_owned())],
            },
        ];
        strings::apply_string_list_edits(
            &mut tag,
            None,
            &[StringListEdit::Insert { at: 0, entries }],
        )
        .unwrap();
        tag
    }

    #[test]
    fn decoded_string_find_counts_occurrences_and_limits_targets_and_languages() {
        let list = strings::read_string_list(&fixture_tag()).unwrap();
        let mut cx = context("strings", KitId(1));
        let syntax = crate::app::editor::string_list::text_syntax(None, cx.game);
        let mut options = StringFindOptions::default();
        let find = |options: &StringFindOptions, cx: &StringFindContext| {
            collect_string_find_occurrences(&list, cx, options, "find", false, false, &syntax)
        };
        assert_eq!(
            find(&options, &cx).len(),
            3,
            "one ID and two text occurrences"
        );
        options.language = StringFindLanguage::All;
        let all = find(&options, &cx);
        assert_eq!(all.len(), 4, "shared IDs are counted once");
        assert_eq!(
            all[3].kind,
            FindTargetKind::StringText {
                index: 1,
                language: 1
            }
        );
        options.ids = false;
        options.language = StringFindLanguage::Language(1);
        assert_eq!(find(&options, &cx).len(), 1);
        options.language = StringFindLanguage::Current;
        cx.language = 1;
        assert_eq!(find(&options, &cx).len(), 1);
        options.text = false;
        assert!(find(&options, &cx).is_empty());
    }

    #[test]
    fn literal_glyph_queries_match_named_and_numeric_notation_and_keep_utf8_ranges() {
        let syntax = crate::app::editor::string_list::text_syntax(None, Some(GameId::Halo3));
        let text = "日本語 &a_button and &U+E100\\r\\n";
        let ranges = string_find_ranges(text, "\u{E100}", &syntax, false, false);
        assert_eq!(
            ranges
                .iter()
                .map(|range| &text[range.clone()])
                .collect::<Vec<_>>(),
            vec!["&a_button", "&U+E100"]
        );
        assert_eq!(
            string_find_ranges(text, "\\r\\n", &syntax, true, false).len(),
            1
        );
        assert_eq!(
            string_find_ranges(text, "日本語", &syntax, false, true),
            vec![0..9]
        );
        assert_eq!(
            string_find_ranges("Go GO goal", "Go", &syntax, true, true),
            vec![0..2]
        );
        assert_eq!(
            string_find_ranges("Go GO goal", "Go", &syntax, false, true),
            vec![0..2, 3..5]
        );
    }

    #[test]
    fn string_find_opens_in_context_refreshes_edits_and_keeps_ordinary_find() {
        let fixture = LooseKit::new("string-find", "halo3_mcc");
        fixture.write_mcc("ui/strings", "multilingual_unicode_string_list", |tag| {
            *tag = fixture_tag();
        });
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "ui/strings.multilingual_unicode_string_list");
        let ctx = egui::Context::default();
        let mut cx = context(&key, app.model.active_kit_id());
        ctx.data_mut(|data| data.insert_temp(string_find_context_id(cx.kit, &key), cx.clone()));
        app.search.find.within = FindWithin::OpenTags;
        app.search.find.query = "Find".to_owned();
        app.open_find_in_context(&ctx);
        assert_eq!(app.search.find.within, FindWithin::CurrentTag);
        assert!(app.search.find.strings.is_some());
        app.refresh_find(&ctx);
        assert_eq!(app.search.find.occurrences.len(), 3);
        app.step_find(&ctx, 1);
        assert!(
            ctx.data(|data| data.get_temp::<FindOccurrence>(string_find_jump_id("test", &key)))
                .is_some()
        );
        app.search.find.strings.as_mut().unwrap().ids = false;
        cx.language = 1;
        ctx.data_mut(|data| data.insert_temp(string_find_context_id(cx.kit, &key), cx.clone()));
        app.refresh_find(&ctx);
        assert_eq!(app.search.find.occurrences.len(), 1);
        assert_eq!(
            app.search.find.occurrences[0].kind,
            FindTargetKind::StringText {
                index: 1,
                language: 1
            }
        );
        let active = app.model.active;
        app.apply_doc_ops(
            active,
            &key,
            "Edit string",
            crate::core::document::apply::DeferredOps {
                string_list_ops: vec![crate::core::document::ops::StringListOp {
                    game: Some(GameId::Halo3),
                    edit: StringListEdit::SetText {
                        index: 1,
                        language: 1,
                        text: "Changed".to_owned(),
                    },
                }],
                ..Default::default()
            },
            crate::core::document::apply::UndoStep::Own,
        )
        .unwrap();
        app.refresh_find(&ctx);
        assert!(
            app.search.find.occurrences.is_empty(),
            "document edits invalidate results"
        );
        cx.editor_active = false;
        ctx.data_mut(|data| data.insert_temp(string_find_context_id(cx.kit, &key), cx));
        app.open_find_in_context(&ctx);
        assert!(
            app.search.find.strings.is_none(),
            "Tag Fields uses ordinary Find"
        );
        app.refresh_find(&ctx);
        assert!(
            app.search
                .find
                .occurrences
                .iter()
                .all(|hit| hit.kind.string_row().is_none())
        );
    }

    #[test]
    fn captured_string_find_keeps_edited_rows_and_tracks_structural_edits() {
        use crate::core::document::apply::{DeferredOps, UndoStep};
        use crate::core::document::ops::{StringListOp, BlockOp, BlockOpKind};
        let fixture = LooseKit::new("captured-string-find-edits", "halo3_mcc");
        fixture.write_mcc("ui/strings", "multilingual_unicode_string_list", |tag| *tag = fixture_tag());
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "ui/strings.multilingual_unicode_string_list");
        let ctx = egui::Context::default();
        let mut cx = context(&key, app.model.active_kit_id());
        cx.selected.insert(0);
        ctx.data_mut(|data| data.insert_temp(string_find_context_id(cx.kit, &key), cx));
        app.search.find.query = "Find".to_owned();
        app.open_find_in_context(&ctx);
        app.search.find.strings.as_mut().unwrap().selected_rows = true;
        app.run_selected_string_find(&ctx);
        app.search.find.filter_results = true;
        let apply = |app: &mut Baboon, edits: Vec<StringListEdit>, blocks: Vec<BlockOp>| {
            app.apply_doc_ops(0, &key, "Test string edit", DeferredOps {
                string_list_ops: edits.into_iter().map(|edit| StringListOp { game: Some(GameId::Halo3), edit }).collect(),
                block_ops: blocks, ..Default::default()
            }, UndoStep::Own).unwrap();
            app.refresh_find(&ctx);
        };
        apply(&mut app, vec![StringListEdit::SetText { index: 0, language: 0, text: "No match anymore".to_owned() }], Vec::new());
        assert_eq!(app.search.find.occurrences.len(), 3, "capture membership stays fixed");
        assert!(app.search.find.occurrences.iter().filter(|hit| matches!(hit.kind, FindTargetKind::StringText { .. })).all(|hit| hit.range.is_empty()));
        assert!(app.search.find.matching_cells.iter().all(|(_, _, kind)| kind.string_row() == Some(0)));
        assert!(app.search.find.filter_results);
        assert!(app.search.find.selection_needs_refresh());
        app.undo_current_tag();
        app.refresh_find(&ctx);
        assert!(app.search.find.occurrences.iter().all(|hit| hit.kind.string_row() == Some(0)), "undo retains the same ID");
        let list = strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap();
        let mut inserted = list.new_entry();
        inserted.id = Some("find_new".to_owned());
        inserted.texts[0] = Some("Find new".to_owned());
        apply(&mut app, vec![StringListEdit::Insert { at: 0, entries: vec![inserted] }], Vec::new());
        assert!(app.search.find.occurrences.iter().all(|hit| hit.kind.string_row() == Some(1)), "insertion follows captured row, excludes new matches");
        apply(&mut app, Vec::new(), vec![BlockOp { path: "string references".to_owned(), kind: BlockOpKind::Reorder { order: vec![1, 0, 2] } }]);
        assert!(app.search.find.occurrences.iter().all(|hit| hit.kind.string_row() == Some(0)), "reorder follows captured row");
        apply(&mut app, vec![StringListEdit::SetId { index: 0, id: "renamed".to_owned() }], Vec::new());
        assert!(app.search.find.occurrences.iter().all(|hit| hit.kind.string_row() == Some(0)), "renaming preserves capture identity");
        apply(&mut app, vec![StringListEdit::Remove { indices: vec![0] }], Vec::new());
        assert!(app.search.find.occurrences.is_empty(), "deleted rows cannot navigate to a different row");
        assert!(app.search.find.filter_results);
    }

    #[test]
    fn classic_capture_mapping_distinguishes_identical_rows_and_never_guesses() {
        use crate::core::document::apply::DeferredOps;
        use crate::core::document::ops::{StringListOp, BlockOp, BlockOpKind};
        let row = |text: &str| StringEntry { id: None, texts: vec![Some(text.to_owned())] };
        let before = vec![row("A"), row("A"), row("B")];
        let edits = vec![StringListEdit::SetText { index: 0, language: 0, text: "Changed".to_owned() },
            StringListEdit::Insert { at: 0, entries: vec![row("New")] }, StringListEdit::Remove { indices: vec![2] }];
        let ops = DeferredOps {
            string_list_ops: edits.into_iter().map(|edit| StringListOp { game: Some(GameId::HaloCe), edit }).collect(),
            block_ops: vec![BlockOp { path: "string references".to_owned(), kind: BlockOpKind::Reorder { order: vec![2, 0, 1] } }], ..Default::default()
        };
        assert_eq!(edited_row_mapping(&before, &[row("B"), row("New"), row("Changed")], &ops), Some(vec![Some(2), None, Some(0)]));
        assert_eq!(surviving_row_mapping(&before, &[row("A"), row("B")]), vec![None, None, Some(1)], "unknown changes never choose an arbitrary duplicate");
    }

    #[test]
    fn selected_rows_find_captures_on_run_and_clears_results_when_query_changes() {
        let fixture = LooseKit::new("selected-string-find", "halo3_mcc");
        fixture.write_mcc("ui/strings", "multilingual_unicode_string_list", |tag| *tag = fixture_tag());
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "ui/strings.multilingual_unicode_string_list");
        let ctx = egui::Context::default();
        let mut cx = context(&key, app.model.active_kit_id());
        cx.selected.insert(0);
        let publish = |cx: &StringFindContext| ctx.data_mut(|data| data.insert_temp(string_find_context_id(cx.kit, &key), cx.clone()));
        publish(&cx);
        app.search.find.query = "Find".to_owned();
        app.open_find_in_context(&ctx);
        let mut query = app.search.find.query();
        query.strings.as_mut().unwrap().selected_rows = true;
        query.strings.as_mut().unwrap().language = StringFindLanguage::All;
        app.apply_search_command(SearchCommand::FindChanged(query), &ctx);
        assert!(app.search.find.occurrences.is_empty(), "manual scope waits for Find");
        app.run_selected_string_find(&ctx);
        let original = app.search.find.occurrences.clone();
        assert_eq!(original.len(), 3);
        assert!(original.iter().all(|hit| hit.kind.string_row() == Some(0)));
        assert!(!app.search.find.selection_needs_refresh());
        cx.selected = [1].into_iter().collect();
        publish(&cx);
        app.refresh_find(&ctx);
        assert_eq!(app.search.find.occurrences, original);
        assert!(app.search.find.selection_needs_refresh());
        app.step_find(&ctx, 1);
        assert_eq!(app.search.find.selection_run.as_ref().unwrap().context.selected, [0].into_iter().collect());
        let mut query = app.search.find.query();
        query.text = "français".to_owned();
        app.apply_search_command(SearchCommand::FindChanged(query), &ctx);
        assert!(app.search.find.occurrences.is_empty(), "changed query requires another explicit Find");
        let snapshot = ctx.data(|data| data.get_temp::<std::sync::Arc<FindRenderSnapshot>>(find_render_snapshot_id())).unwrap();
        assert_eq!(snapshot.query, "français", "obsolete highlights are cleared with the results");
        app.run_selected_string_find(&ctx);
        assert_eq!(app.search.find.occurrences.len(), 1);
        assert_eq!(app.search.find.occurrences[0].kind.string_row(), Some(1));
        assert!(!app.search.find.selection_needs_refresh());
        cx.selected.clear();
        publish(&cx);
        let previous = app.search.find.occurrences.clone();
        app.run_selected_string_find(&ctx);
        assert_eq!(app.search.find.occurrences, previous, "empty selection cannot execute a new search");
        let mut query = app.search.find.query();
        query.strings.as_mut().unwrap().selected_rows = false;
        query.text = "Find".to_owned();
        app.apply_search_command(SearchCommand::FindChanged(query), &ctx);
        assert_eq!(app.search.find.occurrences.len(), 4, "Current Tag searches remain live");
    }

    #[test]
    fn familiar_find_window_exposes_only_string_options_in_string_mode() {
        let mut app = Baboon::for_test();
        app.search.find.strings = Some(StringFindOptions::default());
        app.search.find.string_context = Some(context("strings", app.model.active_kit_id()));
        app.open_find();
        let mut driver = PromptDriver::new();
        driver.frame(&mut app, Vec::new());
        let labels = driver.frame(&mut app, Vec::new());
        assert!(labels.iter().any(|(text, _)| text == "String IDs + String Text"));
        assert!(!labels.iter().any(|(text, _)| text == "String IDs" || text == "String Text"), "targets live in the dropdown");
        assert!(
            labels
                .iter()
                .any(|(text, _)| text.starts_with("Current Language"))
        );
        assert!(
            !labels
                .iter()
                .any(|(text, _)| text == "Field names" || text == "Field values")
        );
        driver.click(&mut app, "String IDs + String Text");
        let targets = driver.frame(&mut app, Vec::new());
        assert!(targets.iter().any(|(text, _)| text == "String IDs"));
        assert!(targets.iter().any(|(text, _)| text == "String Text"));
        driver.click(&mut app, "String IDs");
        assert!(!app.search.find.strings.as_ref().unwrap().ids);
        assert!(app.search.find.strings.as_ref().unwrap().text);
        driver.click(&mut app, "String Text");
        driver.click(&mut app, "String IDs");
        assert!(app.search.find.strings.as_ref().unwrap().ids);
        driver.click(&mut app, "Filter Results");
        assert!(app.search.find.filter_results);
        app.search.find.strings.as_mut().unwrap().selected_rows = true;
        driver.frame(&mut app, Vec::new());
        let labels = driver.frame(&mut app, Vec::new());
        assert!(labels.iter().any(|(text, _)| text == "Selected Rows"));
        assert!(!labels.iter().any(|(text, _)| text == "Select rows in the Strings Editor to search"), "instructions no longer take up space in the window");
        let find = labels.iter().rev().find(|(text, _)| text == "Find").unwrap().1.center();
        driver.frame(&mut app, vec![egui::Event::PointerMoved(find)]);
        let mut tooltip_shown = false;
        for _ in 0..12 {
            let labels = driver.frame(&mut app, Vec::new());
            tooltip_shown |= labels.iter().any(|(text, _)| text == "Select rows in the Strings Editor to search");
        }
        assert!(tooltip_shown, "the disabled Find button explains how to select rows on hover");
    }
}
