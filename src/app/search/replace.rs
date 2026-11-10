//! Staged, validated replacement and coordinated undo for open documents.
use super::*;
use blam_tags::string_list::{self as strings, StringListEdit};

#[derive(PartialEq)]
struct ReplaceApproval {
    kit: KitId,
    generation: u64,
    query: FindQuery,
    replacement: String,
    hits: Vec<FindOccurrence>,
    documents: Vec<(String, u64, String)>,
    locked_fields: usize,
    all: bool,
}

struct LockedReplaceConfirm { approval: ReplaceApproval }
struct LockedReplaceNotice { title: &'static str, message: String }

impl Dialog for LockedReplaceConfirm {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let mut cancel = false;
        let mut confirm = false;
        let response = egui::Modal::new(egui::Id::unique("locked_replace_confirm")).show(cx.egui, |ui| {
            ui.set_width(420.0);
            ui.heading("Replace Normally Locked Fields?");
            ui.label(format!("This replacement includes {} normally locked field(s). Expert mode allows these fields to be edited. Replace {} matching occurrence(s)?",
                self.approval.locked_fields, self.approval.hits.len()));
            ui.horizontal(|ui| {
                confirm = ui.button(if self.approval.all { "Replace All" } else { "Replace" }).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if confirm { cx.send(SearchCommand::ConfirmLockedReplace); }
        !cancel && !response.should_close()
    }
}

impl Dialog for LockedReplaceNotice {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let mut close = false;
        let response = egui::Modal::new(egui::Id::unique("locked_replace_notice")).show(cx.egui, |ui| {
            ui.set_width(420.0);
            ui.heading(self.title);
            ui.label(&self.message);
            close = ui.button("OK").clicked();
        });
        !close && !response.should_close()
    }
}

fn field_requires_expert(tag: &TagFile, docs: Option<&DefDocs>, path: &str) -> Result<bool, String> {
    let mut plans = FindPlans::new(docs, "", false, false);
    let path = blam_tags::TagFieldPath::parse(path);
    let mut node = tag.root();
    let mut hidden = false;
    for (index, segment) in path.segments.iter().enumerate() {
        let mut address = segment.clone();
        address.index = None;
        let field = node.field_path(&address.to_string()).ok_or("A matching field no longer exists. Run Find again.")?;
        let plan = plans.plan(&node);
        let field_plan = plan.fields.get(field.ordinal()).ok_or("The matching field definition changed. Run Find again.")?;
        hidden |= field_plan.hidden;
        if index + 1 == path.segments.len() { return Ok(hidden || field_plan.read_only); }
        node = if let Some(index) = segment.index {
            field.as_block().and_then(|block| block.element(index))
                .or_else(|| field.as_array().and_then(|array| array.element(index)))
        } else { field.as_struct() }.ok_or("The matching field path changed. Run Find again.")?;
    }
    Err("The matching field path is empty.".into())
}

#[derive(Clone)]
pub(in crate::app) struct ReplaceLink {
    label: &'static str,
    documents: Vec<(KitId, String, u64)>,
    steps: Vec<u64>,
    redo: bool,
    capture: Arc<FindMatchingCells>,
    before: Arc<ReplaceMatches>,
    after: Arc<ReplaceMatches>,
    drafts: Vec<ReplaceDraft>,
}

#[derive(Clone)]
struct ReplaceDraft {
    tag_key: String,
    key: String,
    kind: FindTargetKind,
    original: String,
    input: String,
    syntax: Arc<strings::TextSyntax>,
    game: Option<GameId>,
}

impl ReplaceDraft {
    fn apply(&self, buffers: &mut crate::app::editor::EditDrafts) {
        use crate::app::editor::DraftCommit;
        use crate::core::document::{apply::DeferredOps, ops::StringListOp};
        let FindTargetKind::StringText { index, language } = self.kind else { return; };
        let draft = buffers.draft_mut_exact(&self.key, &self.original);
        draft.text = self.input.clone();
        draft.changed = true;
        let (tag_key, key, syntax, game) = (self.tag_key.clone(), self.key.clone(), self.syntax.clone(), self.game);
        draft.keep_commit(|| DraftCommit::new(&tag_key, vec![key], move |texts| {
            let text = strings::parse_shown(texts[0], &syntax)
                .map_err(|_| "Correct the string text marked in red before committing it. A glyph must be a known &name or &U+XXXX.".to_owned())?;
            Ok(DeferredOps { string_list_ops: vec![StringListOp { game,
                edit: StringListEdit::SetText { index, language, text } }], ..Default::default() })
        }));
    }
}

struct ReplaceMatches {
    occurrences: Vec<FindOccurrence>,
    active: Option<usize>,
}

impl ReplaceMatches {
    fn capture(find: &FindDialogState) -> Arc<Self> {
        Arc::new(Self { occurrences: find.occurrences.clone(), active: find.active })
    }
}

/// All ranges refer to the original text. Never search inserted text.
fn replace_ranges(
    text: &str,
    hits: &[FindOccurrence],
    replacement: &str,
) -> Result<String, String> {
    let mut ranges: Vec<_> = hits.iter().map(|hit| hit.range.clone()).collect();
    ranges.sort_by_key(|range| range.start);
    ranges.dedup();
    let mut result = String::new();
    let mut cursor = 0;
    for range in ranges {
        if range.start < cursor || range.is_empty() {
            return Err("Search results changed. Run Find again.".to_owned());
        }
        result.push_str(
            text.get(cursor..range.start)
                .ok_or("Search results changed. Run Find again.")?,
        );
        text.get(range.clone())
            .ok_or("Search results changed. Run Find again.")?;
        result.push_str(replacement);
        cursor = range.end;
    }
    result.push_str(
        text.get(cursor..)
            .ok_or("Search results changed. Run Find again.")?,
    );
    Ok(result)
}

impl Baboon {
    /// Seed the confirmation for the shared window-rendering smoke tests.
    #[cfg(test)]
    pub(in crate::app) fn open_locked_replace_smoke_confirm(&mut self, all: bool) {
        let kit = &self.model.kits[self.model.active];
        self.dialogs.open(LockedReplaceConfirm { approval: ReplaceApproval {
            kit: kit.id,
            generation: kit.generation,
            query: self.search.find.query(),
            replacement: "replacement".to_owned(),
            hits: vec![FindOccurrence {
                tag_key: "smoke".to_owned(), field_path: "locked".to_owned(),
                kind: FindTargetKind::Value, text: "match".to_owned(), range: 0..5,
            }],
            documents: Vec::new(), locked_fields: 1, all,
        }});
    }

    pub(in crate::app) fn set_find_mode(&mut self, replace: bool, ctx: &egui::Context) {
        if replace {
            if let Some(context) = &self.search.find.string_context {
                if let Some(latest) = ctx.data(|data| data.get_temp::<StringFindContext>(string_find_context_id(context.kit, &context.tag_key))) {
                    self.search.find.string_context = Some(latest);
                }
            }
        }
        let default_selection = replace && self.search.find.strings.is_some()
            && self.search.find.string_context.as_ref().is_some_and(|context|
                context.editor_active && context.kit == self.model.active_kit_id() && context.selected.len() > 1);
        let change_scope = default_selection && self.search.find.strings.as_ref().is_some_and(|options| !options.selected_rows);
        if self.search.find.replace_mode == replace && !change_scope { return; }
        let has_results = replace && self.search.find.open && !self.search.find.searching
            && self.search.find.within != FindWithin::AllTags && !self.search.find.query.is_empty()
            && self.search.find.occurrences.iter().any(|hit| matches!(hit.kind,
                FindTargetKind::Value | FindTargetKind::StringId(_) | FindTargetKind::StringText { .. }));
        // Validate the existing Find results before capturing them. Opening
        // Replace on its own must still wait for an explicit Find.
        if has_results { self.refresh_find(ctx); }
        let carry_results = has_results && (!self.search.find.replace_mode || !self.search.find.replace_needs_refresh())
            && self.search.find.occurrences.iter().any(|hit| matches!(hit.kind,
                FindTargetKind::Value | FindTargetKind::StringId(_) | FindTargetKind::StringText { .. }))
            && (!self.search.find.strings.as_ref().is_some_and(|options| options.selected_rows)
                || !self.search.find.selection_needs_refresh());
        let active = carry_results.then(|| self.search.find.active_occurrence().cloned()).flatten();
        let find = &mut self.search.find;
        let entering_replace = replace && !find.replace_mode;
        find.replace_mode = replace;
        find.replace_run = None;
        if replace {
            if entering_replace { find.find_look_in = Some(find.look_in); }
            if default_selection {
                find.strings.as_mut().unwrap().selected_rows = true;
            }
            find.look_in = FindLookIn {
                field_names: false,
                field_values: true,
                blocks: false,
            };
            if find.within == FindWithin::AllTags {
                find.within = FindWithin::CurrentTag;
            }
        } else if let Some(options) = find.find_look_in.take() {
            find.look_in = options;
        }
        find.results_key = None;
        if !carry_results { find.selection_run = None; find.filter_run = None; }
        find.all_requested = false;
        find.all_request_id += 1;
        find.all_signature = None;
        find.all_closed_occurrences.clear();
        find.searching = false;
        if carry_results {
            find.occurrences.retain(|hit| matches!(hit.kind,
                FindTargetKind::Value | FindTargetKind::StringId(_) | FindTargetKind::StringText { .. }));
            if change_scope {
                let selected = &find.string_context.as_ref().unwrap().selected;
                find.occurrences.retain(|hit| hit.kind.string_row().is_some_and(|index| selected.contains(&index)));
            }
        } else {
            find.occurrences.clear();
            find.active = None;
        }
        find.replace_status = None;
        self.search.pending_find_jump = None;
        if carry_results {
            let query = self.search.find.query();
            let context = self.search.find.string_context.clone();
            if let Some(run) = &mut self.search.find.selection_run {
                run.query = query;
                if change_scope { run.context = context.unwrap(); }
            } else if let Some(context) = self.search.find.string_context.clone() {
                if let Some(doc) = self.model.kits[self.model.active].parsed_tags.get(&context.tag_key)
                    && let Some(list) = strings::read_string_list(&doc.tag) {
                    self.search.find.selection_run = Some(StringSelectionRun {
                        query, context, document_stamp: format!("{:?}", doc.content_stamp()),
                        document_id: doc.id, rows: list.entries, changed: false,
                    });
                }
            }
            self.capture_replace_results(ctx, active);
        }
        self.refresh_find(ctx);
    }

    pub(super) fn run_replace_find(&mut self, ctx: &egui::Context) {
        self.search.find.replace_run = None;
        self.search.find.selection_run = None;
        self.search.find.replace_status = None;
        if self.search.find.query.is_empty() || self.search.find.within == FindWithin::AllTags {
            self.refresh_find(ctx);
            return;
        }
        if self.search.find.strings.is_some() {
            self.run_selected_string_find(ctx);
            if self.search.find.selection_run.is_none() { return; }
        } else {
            self.refresh_open_tag_find();
        }
        self.capture_replace_results(ctx, None);
        if let Some(hit) = self.search.find.active_occurrence().cloned() { self.activate_find_occurrence(ctx, hit); }
    }

    fn capture_replace_results(&mut self, ctx: &egui::Context, active: Option<FindOccurrence>) {
        self.finish_find_refresh(ctx, active);
        let kit = &self.model.kits[self.model.active];
        let keys: std::collections::BTreeSet<_> = self.search.find.occurrences.iter().map(|hit| hit.tag_key.clone()).collect();
        let documents = keys.into_iter().filter_map(|key| {
            let doc = kit.parsed_tags.get(&key)?;
            Some((key, doc.id, format!("{:?}", doc.layout_stamp()), format!("{:?}", doc.content_stamp())))
        }).collect();
        self.search.find.replace_run = Some(ReplaceRun {
            query: self.search.find.query(), kit: kit.id,
            selected_tag: (self.search.find.within == FindWithin::CurrentTag).then(|| kit.selected_key.clone()).flatten(),
            documents, matching_cells: self.search.find.matching_cells.clone(), changed: false,
        });
        self.capture_find_filter();
        self.publish_find_snapshot(ctx);
    }

    /// Return true only when the string capture still needs its row-identity
    /// rebasing. Standard results remain fixed, even after their values change.
    pub(super) fn refresh_replace_capture(&mut self, ctx: &egui::Context) -> bool {
        let kit = &self.model.kits[self.model.active];
        let strings = self.search.find.strings.is_some();
        // Index refreshes change the kit generation without replacing its
        // open documents. Captures belong to those documents, not the index.
        let valid = self.search.find.replace_run.as_ref().is_some_and(|run|
            run.query == self.search.find.query() && run.kit == kit.id
            && run.selected_tag.as_ref().is_none_or(|key| kit.selected_key.as_ref() == Some(key))
            && run.documents.iter().all(|(key, id, layout, _)| kit.parsed_tags.get(key).is_some_and(|doc|
                doc.id == *id && (strings || format!("{:?}", doc.layout_stamp()) == *layout))));
        if !valid {
            if self.search.find.replace_run.is_some() { self.search.find.filter_results = false; }
            self.search.find.replace_run = None;
            self.search.find.selection_run = None;
            self.search.find.occurrences.clear();
            self.search.find.active = None;
            self.search.find.matching_cells = Default::default();
            self.publish_find_snapshot(ctx);
            return false;
        }
        let run = self.search.find.replace_run.as_mut().unwrap();
        let changed_keys: std::collections::HashSet<_> = run.documents.iter().filter_map(|(key, _, _, stamp)| {
            (format!("{:?}", kit.parsed_tags[key].content_stamp()) != *stamp).then_some(key.clone())
        }).collect();
        if !changed_keys.is_empty() {
            run.changed = true;
            if !strings {
                for hit in &mut self.search.find.occurrences {
                    if changed_keys.contains(&hit.tag_key) { hit.range = 0..0; }
                }
            }
        }
        self.search.find.matching_cells = run.matching_cells.clone();
        if strings { true } else { self.publish_find_snapshot(ctx); false }
    }

    fn parse_replace_tag(&self, index: usize, key: &str, bytes: &[u8]) -> Result<TagFile, String> {
        let kit = &self.model.kits[index];
        let source = kit.source.as_ref();
        let definitions = source.and_then(|source| match &source.source {
            TagSource::LooseFolder {
                definitions_root, ..
            } => Some(definitions_root.as_path()),
            _ => None,
        });
        crate::core::source::read_tag_from_bytes(
            bytes,
            source.and_then(|source| source.game),
            definitions,
            kit.parsed_tags[key].tag.group().tag,
        )
        .map_err(|error| error.to_string())
    }

    pub(in crate::app) fn replace_find_matches(&mut self, all: bool, ctx: &egui::Context) {
        if let Err(error) = self.try_replace_find_matches(all, ctx, None) {
            self.model.status = error.clone();
            self.search.find.replace_status = Some(error.clone());
            self.dialogs.open(LockedReplaceNotice { title: "Replacement Could Not Be Completed", message: error });
        }
        ctx.request_repaint();
    }

    pub(in crate::app) fn confirm_locked_replace(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.dialogs.close::<LockedReplaceConfirm>() else { return; };
        if let Err(error) = self.try_replace_find_matches(prompt.approval.all, ctx, Some(&prompt.approval)) {
            self.model.status = error.clone();
            self.search.find.replace_status = Some(error.clone());
            self.dialogs.open(LockedReplaceNotice { title: "Replacement Could Not Be Completed", message: error });
        }
        ctx.request_repaint();
    }

    fn try_replace_find_matches(&mut self, all: bool, ctx: &egui::Context, approved: Option<&ReplaceApproval>) -> Result<(), String> {
        let index = self.model.active;
        if self.model.editing_kit_is_read_only(index) {
            return Err("This workspace is read-only.".to_owned());
        }
        self.refresh_find(ctx);
        let find = &self.search.find;
        if !find.open
            || !find.replace_mode
            || find.query.is_empty()
            || find.within == FindWithin::AllTags
            || find.searching
        {
            return Err("Run Find in an editable scope before replacing.".to_owned());
        }
        if find.replace_needs_refresh() {
            return Err("Press Find to refresh the results before replacing.".to_owned());
        }
        let mut hits = if all {
            find.occurrences.clone()
        } else {
            find.active_occurrence().cloned().into_iter().collect()
        };
        if hits.is_empty() {
            return Err("No matches to replace.".to_owned());
        }
        let total = hits.len();
        let mut locked = std::collections::HashSet::new();
        let mut checked = std::collections::HashSet::new();
        for hit in &hits {
            if hit.kind != FindTargetKind::Value { continue; }
            if !checked.insert((hit.tag_key.clone(), hit.field_path.clone())) { continue; }
            let entry = self.model.entry_for_key(&hit.tag_key).cloned().ok_or("A matching tag is no longer open.")?;
            let docs = self.def_docs_for_entry(index, &entry);
            let doc = self.model.kits[index].parsed_tags.get(&hit.tag_key).ok_or("A matching tag is no longer open.")?;
            if field_requires_expert(&doc.tag, docs.as_deref(), &hit.field_path)? {
                locked.insert((hit.tag_key.clone(), hit.field_path.clone()));
            }
        }
        let skipped = if self.model.prefs.expert_mode { 0 } else {
            hits.retain(|hit| !locked.contains(&(hit.tag_key.clone(), hit.field_path.clone())));
            total - hits.len()
        };
        let notice = (skipped > 0).then(|| format!("{skipped} out of {total} matching occurrence(s) could not be replaced because Expert mode is off. These matches are in {} normally locked field(s). {} editable occurrence(s) replaced.", locked.len(), hits.len()));
        if hits.is_empty() {
            let message = notice.unwrap();
            self.search.find.replace_status = Some(message.clone());
            self.model.status = message.clone();
            self.dialogs.open(LockedReplaceNotice { title: "Some Matches Could Not Be Replaced", message });
            if !all { self.step_find(ctx, 1); }
            return Ok(());
        }
        let mut documents = Vec::new();
        for hit in &hits {
            if documents.iter().any(|(key, _, _)| key == &hit.tag_key) { continue; }
            let doc = &self.model.kits[index].parsed_tags[&hit.tag_key];
            documents.push((hit.tag_key.clone(), doc.id, format!("{:?}", doc.content_stamp())));
        }
        let approval = ReplaceApproval { kit: self.model.active_kit_id(), generation: self.model.kits[index].generation,
            query: self.search.find.query(), replacement: self.search.find.replacement.clone(), hits: hits.clone(),
            documents, locked_fields: locked.len(), all };
        if approved.is_some_and(|approved| approved != &approval) || (approved.is_some() && !self.model.prefs.expert_mode) {
            return Err("The search, replacement, or matching fields changed. Review the replacement again.".into());
        }
        let find = &self.search.find;
        let replacement = find.replacement.clone();
        let previous = find.active_occurrence().cloned();
        let original_occurrences = find.occurrences.clone();
        let next_order: Vec<_> = if all {
            Vec::new()
        } else {
            let after = find.active.map_or(0, |index| index + 1);
            find.occurrences[after..]
                .iter()
                .chain(&find.occurrences[..after])
                .cloned()
                .collect()
        };
        let mut groups: Vec<Vec<FindOccurrence>> = Vec::new();
        let mut group_indices = HashMap::new();
        for hit in &hits {
            if !matches!(
                hit.kind,
                FindTargetKind::Value
                    | FindTargetKind::StringId(_)
                    | FindTargetKind::StringText { .. }
            ) {
                return Err(
                    "Only field values, string text and string IDs can be replaced.".to_owned(),
                );
            }
            let identity = (hit.tag_key.clone(), hit.field_path.clone(), hit.kind);
            let group = *group_indices.entry(identity).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[group].push(hit.clone());
        }
        // Serialize originals, then edit independent candidates. No journal,
        // draft, or live value changes until every candidate has succeeded.
        let mut candidates: Vec<(String, TagFile)> = Vec::new();
        let mut drafts = Vec::new();
        for group in &groups {
            let hit = &group[0];
            if !candidates.iter().any(|(key, _)| key == &hit.tag_key) {
                let doc = self.model.kits[index]
                    .parsed_tags
                    .get(&hit.tag_key)
                    .ok_or("A matching tag is no longer open. Run Find again.")?;
                if self.views[self.model.kits[index].id]
                    .edit_buffers
                    .has_changed_prefix(&format!("{}|", hit.tag_key))
                {
                    return Err(format!(
                        "{}: finish or cancel the current field edit before replacing.",
                        hit.tag_key
                    ));
                }
                let bytes = doc
                    .tag
                    .write_to_bytes()
                    .map_err(|error| format!("{}: {error}", hit.tag_key))?;
                candidates.push((
                    hit.tag_key.clone(),
                    self.parse_replace_tag(index, &hit.tag_key, &bytes)?,
                ));
            }
        }
        for (key, candidate) in &mut candidates {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
                let mut edits = Vec::new();
                let string_list = self.search.find.strings.as_ref().and_then(|_| strings::read_string_list(candidate));
                for group in groups.iter().filter(|group| group[0].tag_key == *key) {
                    let hit = &group[0];
                    let label = if hit.field_path.is_empty() { format!("{:?}", hit.kind) } else { hit.field_path.clone() };
                    let mut apply = || -> Result<(), String> {
                        match hit.kind {
                            FindTargetKind::Value => {
                                let current = candidate.root().field_path(&hit.field_path).and_then(|field| field.value())
                                    .map(|value| format_foundation_scalar_value(self.model.names(), &value))
                                    .ok_or("The field is no longer editable.")?;
                                if current != hit.text { return Err("Search results changed. Run Find again.".to_owned()); }
                                let input = replace_ranges(&current, group, &replacement)?;
                                crate::core::document::apply::apply_field_edit(candidate, &hit.field_path, &input)?;
                            }
                            FindTargetKind::StringId(row) | FindTargetKind::StringText { index: row, .. } => {
                                let context = self.search.find.string_context.as_ref().ok_or("The string context has closed.")?;
                                if context.kit != self.model.kits[index].id || context.tag_key != *key { return Err("The string context changed. Run Find again.".to_owned()); }
                                let list = string_list.as_ref().ok_or("The string list is unavailable.")?;
                                let entry = list.entries.get(row).ok_or("The string row has changed.")?;
                                let syntax = crate::app::editor::string_list::tag_text_syntax(context.definitions_root.as_deref(), context.game, &context.tag_key);
                                let current = match hit.kind {
                                    FindTargetKind::StringId(_) => entry.id.clone().ok_or("This list has no string IDs.")?,
                                    FindTargetKind::StringText { language, .. } => strings::format_shown(entry.texts.get(language).and_then(Option::as_deref).unwrap_or_default(), &syntax),
                                    _ => unreachable!(),
                                };
                                if current != hit.text { return Err("Search results changed. Run Find again.".to_owned()); }
                                let input = replace_ranges(&current, group, &replacement)?;
                                edits.push(match hit.kind {
                                    FindTargetKind::StringId(_) => {
                                        if input.is_empty() || input.trim() != input { return Err("An ID cannot be empty or have surrounding whitespace.".to_owned()); }
                                        StringListEdit::SetId { index: row, id: input }
                                    }
                                    FindTargetKind::StringText { language, .. } => {
                                        let text = match strings::parse_shown(&input, &syntax) {
                                            Ok(text) => text,
                                            Err(_) => {
                                                drafts.push(ReplaceDraft { tag_key: key.clone(),
                                                    key: format!("{key}|strings|{row}|{language}"), kind: hit.kind,
                                                    original: current, input, syntax: Arc::new(syntax), game: context.game });
                                                return Ok(());
                                            }
                                        };
                                        StringListEdit::SetText { index: row, language, text }
                                    },
                                    _ => unreachable!(),
                                });
                            }
                            _ => unreachable!(),
                        }
                        Ok(())
                    };
                    apply().map_err(|error| format!("{label}: {error}"))?;
                }
                if !edits.is_empty() {
                    strings::apply_string_list_edits(candidate, self.search.find.string_context.as_ref().and_then(|context| context.game), &edits)?;
                }
                candidate.write_to_bytes().map_err(|error| error.to_string())?;
                Ok(())
            })).map_err(|_| format!("{key}: the editor could not apply the replacement."))?;
            result.map_err(|error| format!("{key}: {error}"))?;
        }
        let mut updated_text = HashMap::new();
        for (key, candidate) in &candidates {
            let context = self.search.find.string_context.as_ref();
            let list = context.and_then(|_| strings::read_string_list(candidate));
            let syntax = context.map(|context| crate::app::editor::string_list::tag_text_syntax(context.definitions_root.as_deref(), context.game, &context.tag_key));
            for group in groups.iter().filter(|group| group[0].tag_key == *key) {
                let hit = &group[0];
                let text = match hit.kind {
                    FindTargetKind::Value => candidate.root().field_path(&hit.field_path).and_then(|field| field.value())
                        .map(|value| format_foundation_scalar_value(self.model.names(), &value)).ok_or("The replacement field is unavailable.")?,
                    FindTargetKind::StringId(row) => list.as_ref().and_then(|list| list.entries.get(row)).and_then(|row| row.id.clone()).ok_or("The replacement ID is unavailable.")?,
                    FindTargetKind::StringText { index, language } => if let Some(draft) = drafts.iter().find(|draft| draft.tag_key == *key && draft.kind == hit.kind) {
                        draft.input.clone()
                    } else { strings::format_shown(
                        list.as_ref().and_then(|list| list.entries.get(index)).and_then(|row| row.texts.get(language)).and_then(Option::as_deref).unwrap_or_default(),
                        syntax.as_ref().ok_or("The string syntax is unavailable.")?) },
                    _ => unreachable!(),
                };
                updated_text.insert((key.clone(), hit.field_path.clone(), hit.kind), text);
            }
        }
        let kit = self.model.kits[index].id;
        if self.model.prefs.expert_mode && !locked.is_empty() && approved.is_none() {
            self.dialogs.open(LockedReplaceConfirm { approval });
            return Ok(());
        }
        let before = ReplaceMatches::capture(&self.search.find);
        let mut link = ReplaceLink {
            label: if all { "Replace All" } else { "Replace" },
            documents: Vec::new(),
            steps: Vec::new(),
            redo: false,
            capture: self.search.find.replace_run.as_ref().unwrap().matching_cells.clone(),
            before: before.clone(),
            after: before,
            drafts: drafts.clone(),
        };
        for (key, candidate) in candidates {
            let doc = self.model.kits[index].parsed_tags.get_mut(&key).unwrap();
            if doc.tag.write_to_bytes().ok() == candidate.write_to_bytes().ok()
                && !drafts.iter().any(|draft| draft.tag_key == key) {
                continue;
            }
            doc.journal.end_edit_window();
            doc.journal
                .begin_edit(&doc.tag, if all { "Replace All" } else { "Replace" });
            doc.journal.end_edit_window();
            doc.tag = candidate;
            doc.dirty.touch();
            if self.search.find.strings.is_some() {
                doc.note_layout_change();
            }
            link.documents.push((kit, key.clone(), doc.id));
            link.steps
                .push(doc.journal.top_id().expect("original was serializable"));
            self.views[kit].edit_buffers.forget_tag(&key);
            self.invalidate_tag_caches_in(index, &key);
        }
        for draft in &drafts { draft.apply(&mut self.views[kit].edit_buffers); }
        if let Some(run) = &mut self.search.find.replace_run {
            for (key, _, layout, stamp) in &mut run.documents {
                let doc = &self.model.kits[index].parsed_tags[key];
                *layout = format!("{:?}", doc.layout_stamp());
                *stamp = format!("{:?}", doc.content_stamp());
            }
        }
        if let Some(run) = &mut self.search.find.selection_run {
            let doc = &self.model.kits[index].parsed_tags[&run.context.tag_key];
            if let Some(list) = strings::read_string_list(&doc.tag) { run.rows = list.entries; }
            run.document_stamp = format!("{:?}", doc.content_stamp());
            run.changed = false;
        }
        // Preserve captured filter membership. Only retire processed matches
        // and shift the other original ranges; never run a new query here.
        self.search.find.occurrences = original_occurrences.into_iter().filter_map(|mut hit| {
            let identity = (hit.tag_key.clone(), hit.field_path.clone(), hit.kind);
            let Some(text) = updated_text.get(&identity) else { return Some(hit); };
            if all || previous.as_ref() == Some(&hit) { return None; }
            let previous = previous.as_ref()?;
            let old_range = hit.range.clone();
            if hit.range.start >= previous.range.end {
                let end = text.len().saturating_sub(previous.text.len() - previous.range.end);
                hit.range = (end + hit.range.start - previous.range.end)..(end + hit.range.end - previous.range.end);
            }
            if text.get(hit.range.clone()) != hit.text.get(old_range) { return None; }
            hit.text = text.clone();
            Some(hit)
        }).collect();
        self.finish_find_refresh(ctx, None);
        if !all && let Some(previous) = previous {
            // Continue after the inserted span, excluding it when wrapping.
            let same = |hit: &FindOccurrence| {
                hit.tag_key == previous.tag_key
                    && hit.field_path == previous.field_path
                    && hit.kind == previous.kind
            };
            // Glyph notation and typed values can normalize on commit. Derive
            // the cursor from the resulting display, not the input's byte count.
            let end = updated_text.get(&(previous.tag_key.clone(), previous.field_path.clone(), previous.kind))
                .map(|text| text.len().saturating_sub(previous.text.len() - previous.range.end))
                .unwrap_or(previous.range.start + replacement.len());
            let next = self
                .search
                .find
                .occurrences
                .iter()
                .enumerate()
                .find(|(_, hit)| same(hit) && hit.range.start >= end)
                .or_else(|| {
                    next_order.iter().filter(|hit| !same(hit)).find_map(|old| {
                        self.search
                            .find
                            .occurrences
                            .iter()
                            .enumerate()
                            .find(|(_, hit)| {
                                hit.tag_key == old.tag_key
                                    && hit.field_path == old.field_path
                                    && hit.kind == old.kind
                            })
                    })
                })
                .or_else(|| {
                    self.search
                        .find
                        .occurrences
                        .iter()
                        .enumerate()
                        .find(|(_, hit)| same(hit) && hit.range.end <= previous.range.start)
                })
                .map(|(index, _)| index);
            self.search.find.active = next;
            self.finish_find_refresh(ctx, self.search.find.active_occurrence().cloned());
            if let Some(hit) = self.search.find.active_occurrence().cloned() {
                self.activate_find_occurrence(ctx, hit);
            }
        }
        // Bind the search states to this exact captured pass and history step.
        // Undo restores its original ranges rather than searching inserted text.
        if !link.documents.is_empty() {
            link.after = ReplaceMatches::capture(&self.search.find);
            // A new edit discards Redo, and journals evict old steps. Do not
            // retain their potentially large match snapshots indefinitely.
            let model = &self.model;
            self.search.replace_links.retain(|link| link.documents.iter().zip(&link.steps).all(|((kit, key, id), step)| {
                model.kit_index(*kit).and_then(|index| model.kits[index].parsed_tags.get(key)).is_some_and(|doc| {
                    let (undo, redo) = doc.journal.stacks();
                    doc.id == *id && undo.iter().chain(redo).any(|snapshot| snapshot.id == *step)
                })
            }));
            self.search.replace_links.push(link);
        }
        let mut status = format!(
            "Replaced {} occurrence{}.",
            hits.len(),
            if hits.len() == 1 { "" } else { "s" }
        );
        if !drafts.is_empty() {
            status.push_str(&format!(" {} cell(s) contain invalid text marked in red; correct them before committing.", drafts.len()));
        }
        self.model.status = status;
        self.search.find.replace_status = None;
        if let Some(message) = notice { self.dialogs.open(LockedReplaceNotice { title: "Some Matches Could Not Be Replaced", message }); }
        Ok(())
    }

    /// Preserve captured identities across a known value-only history step.
    fn preserve_replace_capture_after_history(&mut self, kit: KitId, key: &str, previous_layout: &str) {
        let Some(index) = self.model.kit_index(kit) else { return; };
        let doc = &self.model.kits[index].parsed_tags[key];
        let Some(run) = self.search.find.replace_run.as_mut().filter(|run| run.kit == kit) else { return; };
        let Some((_, _, layout, stamp)) = run.documents.iter_mut()
            .find(|(path, id, layout, _)| path == key && *id == doc.id && layout == previous_layout)
        else { return; };
        *layout = format!("{:?}", doc.layout_stamp());
        *stamp = format!("{:?}", doc.content_stamp());
        run.changed = true;
        // A recorded replacement edits values only. Its inverse has the same
        // field paths and row order, even if it restores a String ID or CE text.
        if self.search.find.strings.is_some() {
            if let Some(list) = strings::read_string_list(&doc.tag) {
                let mapping = (0..list.entries.len()).map(Some).collect();
                let stamp = format!("{:?}", doc.content_stamp());
                self.rebase_selected_string_find(list.entries, Some(mapping), stamp);
            }
        } else {
            for hit in &mut self.search.find.occurrences {
                if hit.tag_key == key { hit.range = 0..0; }
            }
        }
    }

    /// All peers must have this action at the top. Stage every restored tag
    /// before moving any journal, so a failed undo cannot partially apply.
    pub(in crate::app) fn step_linked_replace(&mut self, redo: bool) -> bool {
        let kit = &self.model.kits[self.model.active];
        let Some(key) = &kit.selected_key else {
            return false;
        };
        let Some(doc) = kit.parsed_tags.get(key) else {
            return false;
        };
        let stack = if redo {
            doc.journal.stacks().1
        } else {
            doc.journal.stacks().0
        };
        let Some(step) = stack.last().map(|snapshot| snapshot.id) else {
            return false;
        };
        let Some(link_index) =
            self.search.replace_links.iter().position(|link| {
                link.redo == redo
                    && link.documents.iter().zip(&link.steps).any(
                        |((id, path, document), expected)| {
                            *id == kit.id && path == key && *document == doc.id && *expected == step
                        },
                    )
            })
        else {
            return false;
        };
        let link = self.search.replace_links[link_index].clone();
        let expected_matches = if redo { &link.before } else { &link.after };
        let restore_matches = self.search.find.open && self.search.find.replace_mode
            && !self.search.find.replace_needs_refresh()
            && self.search.find.occurrences == expected_matches.occurrences
            && self.search.find.replace_run.as_ref().is_some_and(|run|
                run.kit == kit.id && Arc::ptr_eq(&run.matching_cells, &link.capture)
                && run.selected_tag.as_ref().is_none_or(|key| kit.selected_key.as_ref() == Some(key))
                && run.documents.iter().all(|(key, id, layout, stamp)| kit.parsed_tags.get(key).is_some_and(|doc|
                    doc.id == *id && format!("{:?}", doc.layout_stamp()) == *layout
                    && format!("{:?}", doc.content_stamp()) == *stamp)));
        let mut restored = Vec::new();
        for ((kit, key, document), expected) in link.documents.iter().zip(&link.steps) {
            let Some(index) = self.model.kit_index(*kit).filter(|&index| {
                self.model.kits[index]
                    .parsed_tags
                    .get(key)
                    .is_some_and(|doc| doc.id == *document)
            }) else {
                // Closed or reloaded documents have their own save/discard
                // lifecycle. Preserve the surviving documents' local history.
                self.search.replace_links.remove(link_index);
                return false;
            };
            let doc = &self.model.kits[index].parsed_tags[key];
            let stack = if redo {
                doc.journal.stacks().1
            } else {
                doc.journal.stacks().0
            };
            if self.model.editing_kit_is_read_only(index)
                || stack.last().is_none_or(|snapshot| snapshot.id != *expected)
                || doc.tag.write_to_bytes().is_err()
                || self.views[*kit]
                    .edit_buffers
                    .has_changed_prefix_except(&format!("{key}|"), |key, text|
                        !redo && link.drafts.iter().any(|draft| draft.key == key && draft.input == text))
            {
                self.model.status = "Finish current edits and step newer history in the affected tags before undoing or redoing Replace All.".to_owned();
                return true;
            }
            match self.parse_replace_tag(index, key, &stack.last().unwrap().bytes) {
                Ok(tag) => restored.push((index, tag)),
                Err(error) => {
                    self.model.status = format!("Could not restore Replace All: {error}");
                    return true;
                }
            }
        }
        let mut steps = Vec::new();
        for (((kit, key, _), _), (index, tag)) in
            link.documents.iter().zip(&link.steps).zip(restored)
        {
            let doc = self.model.kits[index].parsed_tags.get_mut(key).unwrap();
            let previous_layout = format!("{:?}", doc.layout_stamp());
            if redo {
                doc.journal.redo(&doc.tag);
            } else {
                doc.journal.undo(&doc.tag);
            }
            doc.tag = tag;
            doc.dirty.touch();
            doc.note_layout_change();
            doc.settle_after_step();
            crate::app::editor::string_list::clear_selection_after_history(&self.egui_ctx,doc.id);
            let opposite = if redo {
                doc.journal.stacks().0
            } else {
                doc.journal.stacks().1
            };
            steps.push(opposite.last().unwrap().id);
            self.preserve_replace_capture_after_history(*kit, key, &previous_layout);
            self.views[*kit].edit_buffers.forget_tag(key);
            if redo {
                for draft in link.drafts.iter().filter(|draft| draft.tag_key == *key) {
                    draft.apply(&mut self.views[*kit].edit_buffers);
                }
            }
            self.invalidate_tag_caches_in(index, key);
        }
        self.search.replace_links[link_index].steps = steps;
        self.search.replace_links[link_index].redo = !redo;
        if restore_matches {
            let find = &mut self.search.find;
            let matches = if redo { &link.after } else { &link.before };
            find.occurrences = matches.occurrences.clone();
            find.active = matches.active;
            find.reveal_restored_active = true;
            find.matching_cells = link.capture.clone();
            let run = find.replace_run.as_mut().unwrap();
            run.matching_cells = link.capture.clone();
            run.changed = false;
            if let Some(run) = &mut find.selection_run { run.changed = false; }
            find.replace_status = None;
        }
        self.model.status = format!("{}: {}", if redo { "Redo" } else { "Undo" }, link.label);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::LooseKit;

    fn halo2_keywords_fixture() -> (Baboon, LooseKit, egui::Context, String) {
            let fixture = LooseKit::new("h2-keyword-repro", "halo2_mcc");
            let mut tag = TagFile::new_classic(crate::core::bundled::locate_definitions_root()
                .join("halo2_mcc/multilingual_unicode_string_list.json"), blam_tags::classic::ClassicEngine::Halo2V4).unwrap();
            let mut row = strings::read_string_list(&tag).unwrap().new_entry();
            row.id = Some("sword".into());
            row.texts[0] = Some("Hold this: \u{E11F}".into());
            let mut second = row.clone();
            second.id = Some("another_sword".into());
            strings::apply_string_list_edits(&mut tag, Some(GameId::Halo2),
                &[StringListEdit::Insert { at: 0, entries: vec![row, second] }]).unwrap();
            std::fs::write(fixture.path("hud_messages.multilingual_unicode_string_list"), tag.write_to_bytes().unwrap()).unwrap();
            let mut app = Baboon::for_test();
            fixture.install(&mut app);
            let key = fixture.open(&mut app, "hud_messages.multilingual_unicode_string_list");
            let ctx = egui::Context::default();
            let list = strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap();
            app.search.find.string_context = Some(StringFindContext {
                selected: Default::default(), kit: app.model.active_kit_id(), tag_key: key.clone(), scope: "h2-repro".into(),
                game: Some(GameId::Halo2), definitions_root: None, language: 0,
                languages: list.languages.iter().map(|language| language.name.clone()).collect(), has_ids: true, editor_active: true,
            });
            app.search.find.strings = Some(StringFindOptions { ids: false, ..Default::default() });
            app.search.find.query = "&plasma_sword".into();
            app.open_find();
            app.set_find_mode(true, &ctx);
            (app, fixture, ctx, key)
    }

    #[test]
    fn halo2_keyword_replacements_accept_known_names_and_unicode() {
        for (replacement, expected) in [("&plasma_rifle", "\u{E11E}"), ("\u{E11E}", "\u{E11E}"), ("&U+E11E", "\u{E11E}"), ("&U+0123", "\u{0123}")] {
            let (mut app, _fixture, ctx, key) = halo2_keywords_fixture();
            app.search.find.replacement = replacement.into();
            app.run_replace_find(&ctx);
            assert!(!app.search.find.occurrences.is_empty());
            app.replace_find_matches(false, &ctx);
            assert!(app.search.find.replace_status.is_none(), "{replacement}: {:?}", app.search.find.replace_status);
            assert_eq!(app.model.status, "Replaced 1 occurrence.");
            assert_eq!(strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap().entries[0].texts[0].as_deref(), Some(format!("Hold this: {expected}").as_str()));
        }
    }

    #[test]
    fn hud_find_replace_validates_percent_tokens_and_undoes() {
        let fixture=LooseKit::new("hud-replace","haloce_mcc");
        let mut tag=TagFile::new_classic(crate::core::bundled::locate_definitions_root().join("haloce_mcc/hud_message_text.json"),blam_tags::classic::ClassicEngine::HaloCe).unwrap();
        strings::apply_string_list_edits(&mut tag,Some(GameId::HaloCe),&[strings::StringListEdit::Insert {at:0,entries:vec![strings::StringEntry {id:Some("pickup".into()),texts:vec![Some("Hold %{18} %{32}".into())]}]}]).unwrap();
        std::fs::write(fixture.path("messages.hud_message_text"),tag.write_to_bytes().unwrap()).unwrap();
        let mut app=Baboon::for_test(); fixture.install(&mut app); let key=fixture.open(&mut app,"messages.hud_message_text"); let ctx=egui::Context::default();
        app.search.find.string_context=Some(StringFindContext {selected:Default::default(),kit:app.model.active_kit_id(),tag_key:key.clone(),scope:"hud-test".into(),game:Some(GameId::HaloCe),definitions_root:None,language:0,languages:vec!["Text".into()],has_ids:true,editor_active:true});
        app.search.find.strings=Some(StringFindOptions {ids:false,..Default::default()}); app.search.find.query="%action".into(); app.search.find.replacement="%jump".into();
        app.open_find(); app.set_find_mode(true,&ctx);
        let before=app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap();
        app.run_replace_find(&ctx); assert_eq!(app.search.find.occurrences.len(),1); app.replace_find_matches(true,&ctx);
        assert_eq!(strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap().entries[0].texts[0].as_deref(),Some("Hold %{22} %{32}"));
        app.undo_current_tag(); assert_eq!(app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap(),before);
        app.search.find.replacement="%bogus".into(); app.run_replace_find(&ctx); app.replace_find_matches(true,&ctx);
        assert_eq!(app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap(),before);
        assert!(app.views[app.model.active_kit_id()].edit_buffers.has_changed_prefix(&format!("{key}|")));
    }

    #[test]
    fn invalid_keyword_replacements_remain_visible_as_red_editable_drafts() {
        for replacement in ["&lightsaber", "&U+123"] {
            for all in [false, true] {
                let (mut app, _fixture, ctx, key) = halo2_keywords_fixture();
                ctx.set_fonts(crate::app::ui_kit::foundation_fonts());
                app.search.find.replacement = replacement.into();
                app.run_replace_find(&ctx);
                app.search.find.filter_results = true;
                let before = app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap();
                app.replace_find_matches(all, &ctx);
                let expected = format!("Hold this: {replacement}");
                assert!(app.search.find.replace_status.is_none(), "invalid notation should become an editable draft");
                let output = rendered_pane_output(&mut app, &ctx, &key, "h2-repro");
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == expected)));
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == crate::app::ui_kit::invalid_value_fill())), "the affected cell is visibly marked red");
                assert_eq!(app.search.replace_links.last().unwrap().drafts.len(), if all { 2 } else { 1 });
                assert_eq!(app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap(), before, "invalid text is not committed to the tag");
                assert!(app.views[app.model.active_kit_id()].edit_buffers.has_changed_prefix(&format!("{key}|")));
                app.undo_current_tag();
                app.refresh_find(&ctx);
                assert!(!app.views[app.model.active_kit_id()].edit_buffers.has_changed_prefix(&format!("{key}|")));
                assert!(!app.search.find.replace_needs_refresh());
                app.redo_current_tag();
                app.refresh_find(&ctx);
                assert!(rendered_pane_texts(&mut app, &ctx, &key, "h2-repro").iter().any(|text| text == &expected));
                let commits = app.views[app.model.active_kit_id()].edit_buffers.take_uncommitted(crate::app::editor::DraftFlush::All);
                assert_eq!(commits.len(), if all { 2 } else { 1 });
                assert!(commits.iter().all(|(_, result)| result.is_err()), "invalid drafts cannot be committed");
            }
        }
    }

    #[test]
    fn invalid_replacement_draft_can_be_corrected_before_the_row_is_drawn() {
        let (mut app, _fixture, ctx, key) = halo2_keywords_fixture();
        app.search.find.replacement = "&lightsaber".into();
        app.run_replace_find(&ctx);
        app.replace_find_matches(false, &ctx);
        let buffer_key = format!("{key}|strings|0|0");
        let draft = app.views[app.model.active_kit_id()].edit_buffers.draft_mut_exact(&buffer_key, "Hold this: &plasma_sword");
        draft.text = "Hold this: &plasma_rifle".into();
        assert!(app.commit_drafts(crate::app::editor::DraftFlush::All));
        assert_eq!(strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap().entries[0].texts[0].as_deref(), Some("Hold this: \u{E11E}"));
    }

    #[test]
    fn replacement_history_does_not_discard_subsequent_draft_edits() {
        let (mut app, _fixture, ctx, key) = halo2_keywords_fixture();
        app.search.find.replacement = "&lightsaber".into();
        app.run_replace_find(&ctx);
        app.replace_find_matches(false, &ctx);
        let buffer_key = format!("{key}|strings|0|0");
        let draft = app.views[app.model.active_kit_id()].edit_buffers.draft_mut_exact(&buffer_key, "Hold this: &plasma_sword");
        draft.text = "Hold this: &other_invalid_name".into();
        app.undo_current_tag();
        assert!(app.model.status.contains("Finish current edits"));
        assert_eq!(app.views[app.model.active_kit_id()].edit_buffers.draft_mut_exact(&buffer_key, "Hold this: &plasma_sword").text, "Hold this: &other_invalid_name");
    }

    fn locked_fields_fixture(expert: bool) -> (Baboon, LooseKit, egui::Context, String) {
        let fixture = LooseKit::new("replace-locked-fields", "halo3_mcc");
        fixture.write_mcc("model", "render_model", |tag| {
            for field in ["node list checksum", "runtime import info checksum", "don't draw over camera cosine angle"] {
                crate::core::document::apply::apply_field_edit(tag, field, "17").unwrap();
            }
        });
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "model.render_model");
        app.model.prefs.expert_mode = expert;
        let ctx = egui::Context::default();
        app.open_find();
        app.search.find.query = "17".into();
        app.set_find_mode(true, &ctx);
        app.search.find.replacement = "18".into();
        app.run_replace_find(&ctx);
        (app, fixture, ctx, key)
    }

    fn field_text(app: &Baboon, key: &str, path: &str) -> String {
        format_foundation_scalar_value(app.model.names(), &app.model.kits[0].parsed_tags[key].tag.root().field_path(path).unwrap().value().unwrap())
    }

    #[test]
    fn normal_replace_skips_locked_fields_and_reports_counts() {
        let (mut app, _fixture, ctx, key) = locked_fields_fixture(false);
        assert_eq!(app.search.find.occurrences.len(), 2);
        app.replace_find_matches(true, &ctx);
        assert_eq!(field_text(&app, &key, "node list checksum"), "17");
        assert_eq!(field_text(&app, &key, "don't draw over camera cosine angle"), "18");
        let notice = app.dialogs.get::<LockedReplaceNotice>().unwrap();
        assert!(notice.message.contains("1 out of 2"));
        assert!(notice.message.contains("Expert mode is off"));
        ctx.set_fonts(crate::app::ui_kit::foundation_fonts());
        let mut driver = crate::app::loose_fixture::PromptDriver::on(ctx.clone(), 1.0);
        driver.click(&mut app, "OK");
        assert!(app.dialogs.get::<LockedReplaceNotice>().is_none());
        app.undo_current_tag();
        app.refresh_find(&ctx);
        assert_eq!(field_text(&app, &key, "don't draw over camera cosine angle"), "17");
        assert!(!app.search.find.replace_needs_refresh());
    }

    #[test]
    fn expert_replace_confirms_locked_fields_before_mutating() {
        let (mut app, _fixture, ctx, key) = locked_fields_fixture(true);
        let before = app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap();
        app.replace_find_matches(true, &ctx);
        let confirm = app.dialogs.get::<LockedReplaceConfirm>().unwrap();
        assert_eq!(confirm.approval.locked_fields, 2);
        assert_eq!(confirm.approval.hits.len(), 3);
        assert_eq!(app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap(), before);
        assert!(!app.model.kits[0].parsed_tags[&key].journal.can_undo());
        app.confirm_locked_replace(&ctx);
        for field in ["node list checksum", "runtime import info checksum", "don't draw over camera cosine angle"] {
            assert_eq!(field_text(&app, &key, field), "18");
        }
    }

    #[test]
    fn expert_replace_cancel_and_stale_confirmation_change_nothing() {
        let (mut app, _fixture, ctx, key) = locked_fields_fixture(true);
        let before = app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap();
        app.replace_find_matches(true, &ctx);
        ctx.set_fonts(crate::app::ui_kit::foundation_fonts());
        let mut driver = crate::app::loose_fixture::PromptDriver::on(ctx.clone(), 1.0);
        driver.click(&mut app, "Cancel");
        assert!(app.dialogs.get::<LockedReplaceConfirm>().is_none());
        assert_eq!(app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap(), before);
        app.replace_find_matches(true, &ctx);
        app.search.find.replacement = "19".into();
        app.confirm_locked_replace(&ctx);
        assert!(app.model.status.contains("changed"));
        assert_eq!(app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap(), before);
        assert!(!app.model.kits[0].parsed_tags[&key].journal.can_undo());
    }

    #[test]
    fn invalid_expert_replacement_is_refused_before_confirmation() {
        let (mut app, _fixture, ctx, key) = locked_fields_fixture(true);
        app.search.find.replacement = "invalid number".into();
        let before = app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap();
        app.replace_find_matches(true, &ctx);
        assert!(app.dialogs.get::<LockedReplaceConfirm>().is_none());
        assert_eq!(app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap(), before);
    }

    #[test]
    fn explicit_find_keeps_the_last_filter_until_find_runs_again() {
        for replace in [false, true] {
            let fixture = LooseKit::new("manual-find-filter", "halo3_mcc");
            fixture.write_mcc("tag", "damage_effect", |tag| {
                crate::core::document::apply::apply_field_edit(tag, "stun time", "17").unwrap();
            });
            let mut app = Baboon::for_test();
            fixture.install(&mut app);
            let key = fixture.open(&mut app, "tag.damage_effect");
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::app::ui_kit::foundation_fonts());
            app.open_find();
            app.search.find.query = "17".into();
            if replace { app.set_find_mode(true, &ctx); }
            else { app.search.find.within = FindWithin::AllTags; }
            app.apply_search_command(SearchCommand::FindRunSelection, &ctx);
            app.search.find.filter_results = true;
            let visible = |app: &mut Baboon| rendered_pane_texts(app, &ctx, &key, "test").iter().any(|text| text == "stun time");
            assert!(visible(&mut app));
            for text in ["nothing", "", "18"] {
                let mut query = app.search.find.query();
                query.text = text.into();
                app.apply_search_command(SearchCommand::FindChanged(query), &ctx);
                assert!(visible(&mut app), "typing must not change the applied filter");
            }
            app.apply_search_command(SearchCommand::FindRunSelection, &ctx);
            assert!(!visible(&mut app), "Find applies the pending query to the filter");
        }
    }

    #[test]
    fn manual_string_filter_waits_for_find_after_typing() {
        let (mut app, _fixture, ctx, key) = strings_fixture("manual-string-filter");
        ctx.set_fonts(crate::app::ui_kit::foundation_fonts());
        app.search.find.query = "a a".into();
        app.run_replace_find(&ctx);
        app.search.find.filter_results = true;
        app.refresh_find(&ctx);
        let visible = |app: &mut Baboon| rendered_pane_texts(app, &ctx, &key, "replace-test").iter().any(|text| text == "first");
        assert!(visible(&mut app));
        let mut query = app.search.find.query();
        query.text = "français".into();
        app.apply_search_command(SearchCommand::FindChanged(query), &ctx);
        assert!(visible(&mut app));
        assert_eq!(ctx.data(|data| data.get_temp::<Arc<FindRenderSnapshot>>(find_render_snapshot_id())).unwrap().query, "a a");
        app.run_replace_find(&ctx);
        assert!(!visible(&mut app));
    }

    fn hit(text: &str, range: std::ops::Range<usize>) -> FindOccurrence {
        FindOccurrence {
            tag_key: "test".into(),
            field_path: "field".into(),
            kind: FindTargetKind::Value,
            text: text.into(),
            range,
        }
    }

    fn rendered_pane_output(app: &mut Baboon, ctx: &egui::Context, key: &str, scope: &str) -> egui::FullOutput {
        let entry = app.model.kits[0].source.as_ref().unwrap().all_entries.iter()
            .find(|entry| entry.key == key).unwrap().clone();
        let output = crate::app::run_ui_test(ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            ..Default::default()
        }, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                crate::app::editor::draw_tag_pane(&cx!(app, ctx), ui, 0, &entry, scope, false,
                    crate::app::editor::PaneInputs {
                        find: &app.search.find, field_nav: app.references.field_nav.as_ref(), audio: &app.audio,
                        def_docs: None, ce_sound: None,
                    }, &mut app.views[app.model.active_kit_id()], &mut app.editor);
            });
        });
        output
    }

    fn rendered_pane_texts(app: &mut Baboon, ctx: &egui::Context, key: &str, scope: &str) -> Vec<String> {
        rendered_pane_output(app, ctx, key, scope).shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        }).collect()
    }

    #[test]
    fn replacements_use_original_unicode_ranges_and_literal_text() {
        let text = "é a a";
        assert_eq!(
            replace_ranges(text, &[hit(text, 3..4), hit(text, 5..6)], "aa").unwrap(),
            "é aa aa"
        );
        assert_eq!(
            replace_ranges(text, &[hit(text, 3..4)], "$1\\n").unwrap(),
            "é $1\\n a"
        );
        assert_eq!(
            replace_ranges(text, &[hit(text, 3..4), hit(text, 5..6)], "").unwrap(),
            "é  "
        );
        assert!(replace_ranges(text, &[hit(text, 1..2)], "x").is_err());
    }

    #[test]
    fn index_refresh_preserves_captured_replace_matches_and_filter() {
        let (mut app, fixture, ctx, key) = strings_fixture("replace-index-refresh");
        app.run_replace_find(&ctx);
        app.search.find.active = Some(1);
        app.search.find.filter_results = true;
        let hits = app.search.find.occurrences.clone();
        assert_eq!(hits.len(), 3);
        let document_id = app.model.kits[0].parsed_tags[&key].id;
        fixture.write_mcc("unrelated", "damage_effect", |_| {});
        let entries = crate::core::source::scan_folder_subtree_entries(&fixture.root, std::path::Path::new(""), &fixture.names()).unwrap();
        app.apply_entry_index_refresh(0, crate::core::source::EntryIndexRefresh {
            entries, changed: true, added: 1, updated: 0, removed: 0,
            touched: Vec::new(), removed_keys: Vec::new(), touched_dependencies: Vec::new(), errors: Vec::new(),
        }, ctx.clone());
        assert_eq!(app.model.kits[0].parsed_tags[&key].id, document_id, "the searched document is unchanged");
        app.refresh_find(&ctx);
        assert_eq!(app.search.find.occurrences, hits, "index refresh must not clear captured matches");
        assert_eq!(app.search.find.active, Some(1));
        assert!(app.search.find.filter_results);
        assert!(!app.search.find.replace_needs_refresh());
        app.search.find.replacement = "fixed".into();
        app.replace_find_matches(false, &ctx);
        let list = strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap();
        assert_eq!(list.entries[0].texts[0].as_deref(), Some("a fixed"));
    }

    #[test]
    fn index_refresh_preserves_open_tag_field_replace_results() {
        let fixture = LooseKit::new("replace-fields-index-refresh", "halo3_mcc");
        for path in ["a", "b"] {
            fixture.write_mcc(path, "damage_effect", |tag| {
                crate::core::document::apply::apply_field_edit(tag, "stun time", "17").unwrap();
            });
        }
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        fixture.open(&mut app, "a.damage_effect");
        fixture.open(&mut app, "b.damage_effect");
        let ctx = egui::Context::default();
        app.open_find();
        app.search.find.within = FindWithin::OpenTags;
        app.search.find.query = "17".into();
        app.set_find_mode(true, &ctx);
        app.run_replace_find(&ctx);
        app.search.find.active = Some(1);
        app.search.find.filter_results = true;
        let hits = app.search.find.occurrences.clone();
        assert_eq!(hits.len(), 2);
        fixture.write_mcc("unrelated", "damage_effect", |_| {});
        let entries = crate::core::source::scan_folder_subtree_entries(&fixture.root, std::path::Path::new(""), &fixture.names()).unwrap();
        app.apply_entry_index_refresh(0, crate::core::source::EntryIndexRefresh {
            entries, changed: true, added: 1, updated: 0, removed: 0,
            touched: Vec::new(), removed_keys: Vec::new(), touched_dependencies: Vec::new(), errors: Vec::new(),
        }, ctx.clone());
        app.refresh_find(&ctx);
        assert_eq!(app.search.find.occurrences, hits);
        assert_eq!(app.search.find.active, Some(1));
        assert!(app.search.find.filter_results && !app.search.find.replace_needs_refresh());
        app.search.find.replacement = "18".into();
        app.replace_find_matches(false, &ctx);
        assert_eq!(app.model.status, "Replaced 1 occurrence.");
        assert!(app.search.find.replace_status.is_none());
    }

    #[test]
    fn reloading_a_captured_document_still_invalidates_replace_results() {
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-reloaded-document");
        app.run_replace_find(&ctx);
        assert!(!app.search.find.occurrences.is_empty());
        let bytes = app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap();
        let reloaded = app.parse_replace_tag(0, &key, &bytes).unwrap();
        app.model.kits[0].parsed_tags.insert(key, TagDocument::clean(reloaded));
        app.refresh_find(&ctx);
        assert!(app.search.find.occurrences.is_empty());
        assert!(app.search.find.replace_needs_refresh());
    }

    fn strings_fixture(name: &str) -> (Baboon, LooseKit, egui::Context, String) {
        let fixture = LooseKit::new(name, "halo3_mcc");
        fixture.write_mcc("ui/strings", "multilingual_unicode_string_list", |tag| {
            strings::apply_string_list_edits(
                tag,
                Some(GameId::Halo3),
                &[StringListEdit::Insert {
                    at: 0,
                    entries: vec![
                        strings::StringEntry {
                            id: Some("first".into()),
                            texts: vec![Some("a a".into()), Some("a français".into())],
                        },
                        strings::StringEntry {
                            id: Some("a".into()),
                            texts: vec![Some("a".into()), Some("a".into())],
                        },
                    ],
                }],
            )
            .unwrap();
        });
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "ui/strings.multilingual_unicode_string_list");
        let ctx = egui::Context::default();
        app.search.find.string_context = Some(StringFindContext {
            selected: [0].into_iter().collect(),
            kit: app.model.active_kit_id(),
            tag_key: key.clone(),
            scope: "replace-test".into(),
            game: Some(GameId::Halo3),
            definitions_root: None,
            language: 0,
            languages: vec!["English".into(), "French".into()],
            has_ids: true,
            editor_active: true,
        });
        app.search.find.strings = Some(StringFindOptions {
            ids: false,
            ..Default::default()
        });
        app.search.find.query = "a".into();
        app.open_find();
        app.set_find_mode(true, &ctx);
        (app, fixture, ctx, key)
    }

    #[test]
    fn string_replace_all_converts_unicode_preserves_languages_and_undoes() {
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-unicode");
        let before = app.model.kits[0].parsed_tags[&key]
            .tag
            .write_to_bytes()
            .unwrap();
        app.search.find.replacement = "&U+00E9".into();
        app.run_replace_find(&ctx);
        app.replace_find_matches(true, &ctx);
        assert_eq!(
            app.model.status,
            "Replaced 3 occurrences."
        );
        let list = strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap();
        assert_eq!(list.entries[0].texts[0].as_deref(), Some("é é"));
        assert_eq!(list.entries[0].texts[1].as_deref(), Some("a français"));
        app.undo_current_tag();
        assert_eq!(
            app.model.kits[0].parsed_tags[&key]
                .tag
                .write_to_bytes()
                .unwrap(),
            before
        );
        app.redo_current_tag();
        assert_eq!(
            strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag)
                .unwrap()
                .entries[0]
                .texts[0]
                .as_deref(),
            Some("é é")
        );
    }

    #[test]
    fn invalid_id_aborts_all_replacements_and_creates_no_history() {
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-invalid-id");
        let before = app.model.kits[0].parsed_tags[&key]
            .tag
            .write_to_bytes()
            .unwrap();
        app.search.find.strings.as_mut().unwrap().ids = true;
        app.search.find.replacement.clear();
        app.run_replace_find(&ctx);
        app.replace_find_matches(true, &ctx);
        assert!(
            app.model.status.contains("ID cannot be empty"),
            "{}",
            app.model.status
        );
        let notice = app.dialogs.get::<LockedReplaceNotice>().expect("replacement failures require acknowledgment");
        assert_eq!(notice.title, "Replacement Could Not Be Completed");
        assert!(notice.message.contains("ID cannot be empty"));
        ctx.set_fonts(crate::app::ui_kit::foundation_fonts());
        let mut driver = crate::app::loose_fixture::PromptDriver::on(ctx.clone(), 1.0);
        driver.click(&mut app, "OK");
        assert!(app.dialogs.get::<LockedReplaceNotice>().is_none());
        assert_eq!(
            app.model.kits[0].parsed_tags[&key]
                .tag
                .write_to_bytes()
                .unwrap(),
            before
        );
        assert!(!app.model.kits[0].parsed_tags[&key].journal.can_undo());
    }

    #[test]
    fn opening_replace_defaults_to_multiple_selected_string_rows() {
        for count in [0, 1, 2] {
            let (mut app, _fixture, ctx, key) = strings_fixture("replace-selection-default");
            let mut context = app.search.find.string_context.clone().unwrap();
            context.selected = (0..count).collect();
            ctx.data_mut(|data| data.insert_temp(string_find_context_id(context.kit, &key), context));
            app.search.find.close();
            app.search.find.strings = None;
            app.search.find.string_context = None;
            app.apply_app_action(crate::app::shell::AppAction::OpenReplace, &ctx);
            assert!(app.search.find.open && app.search.find.replace_mode);
            assert_eq!(app.search.find.strings.as_ref().unwrap().selected_rows, count > 1);
            assert!(app.search.find.replace_needs_refresh(), "opening still waits for Find");
        }
    }

    #[test]
    fn reopening_replace_uses_the_latest_multiple_selection_without_losing_results() {
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-reopen-selection");
        app.run_replace_find(&ctx);
        app.search.find.filter_results = true;
        let look_in = app.search.find.find_look_in;
        let mut context = app.search.find.string_context.clone().unwrap();
        context.selected = [0, 1].into_iter().collect();
        ctx.data_mut(|data| data.insert_temp(string_find_context_id(context.kit, &key), context));
        app.apply_app_action(crate::app::shell::AppAction::OpenReplace, &ctx);
        assert!(app.search.find.strings.as_ref().unwrap().selected_rows);
        assert!(app.search.find.filter_results && !app.search.find.replace_needs_refresh());
        assert_eq!(app.search.find.occurrences.len(), 3);
        assert_eq!(app.search.find.find_look_in, look_in);
    }

    #[test]
    fn switching_to_replace_narrows_carried_results_to_multiple_selected_rows() {
        use crate::core::document::{apply::{DeferredOps, UndoStep}, ops::StringListOp};
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-selection-carry");
        app.apply_doc_ops(0, &key, "Add outside row", DeferredOps {
            string_list_ops: vec![StringListOp { game: Some(GameId::Halo3),
                edit: StringListEdit::Insert { at: 2, entries: vec![strings::StringEntry {
                    id: Some("outside".into()), texts: vec![Some("a".into())],
                }] } }], ..Default::default()
        }, UndoStep::Own);
        app.set_find_mode(false, &ctx);
        let mut context = app.search.find.string_context.clone().unwrap();
        context.selected = [0, 1].into_iter().collect();
        ctx.data_mut(|data| data.insert_temp(string_find_context_id(context.kit, &key), context));
        app.refresh_find(&ctx);
        assert_eq!(app.search.find.occurrences.len(), 4);
        app.search.find.active = Some(3);
        app.search.find.filter_results = true;
        app.set_find_mode(true, &ctx);
        assert!(app.search.find.strings.as_ref().unwrap().selected_rows);
        assert_eq!(app.search.find.occurrences.len(), 3);
        assert!(app.search.find.occurrences.iter().all(|hit| hit.kind.string_row().unwrap() < 2));
        assert!(app.search.find.filter_results && !app.search.find.replace_needs_refresh());
        app.search.find.replacement = "selected".into();
        app.replace_find_matches(true, &ctx);
        let list = strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap();
        assert_eq!(list.entries[0].texts[0].as_deref(), Some("selected selected"));
        assert_eq!(list.entries[1].texts[0].as_deref(), Some("selected"));
        assert_eq!(list.entries[2].texts[0].as_deref(), Some("a"));
    }

    #[test]
    fn selected_rows_replace_requires_find_and_excludes_other_rows() {
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-selected");
        app.search.find.strings.as_mut().unwrap().selected_rows = true;
        app.search.find.replacement = "aa".into();
        app.replace_find_matches(true, &ctx);
        assert!(app.model.status.contains("Press Find"));
        app.run_replace_find(&ctx);
        app.replace_find_matches(true, &ctx);
        let list = strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap();
        assert_eq!(list.entries[0].texts[0].as_deref(), Some("aa aa"));
        assert_eq!(list.entries[1].texts[0].as_deref(), Some("a"));
        assert_eq!(
            app.model.status,
            "Replaced 2 occurrences."
        );
    }

    #[test]
    fn replace_current_advances_past_inserted_matches() {
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-advance");
        app.search.find.replacement = "aa".into();
        app.run_replace_find(&ctx);
        app.replace_find_matches(false, &ctx);
        let active = app.search.find.active_occurrence().unwrap();
        assert_eq!(
            active.kind,
            FindTargetKind::StringText {
                index: 0,
                language: 0
            }
        );
        assert_eq!(active.range, 3..4);
        assert_eq!(
            strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag)
                .unwrap()
                .entries[0]
                .texts[0]
                .as_deref(),
            Some("aa a")
        );
        app.replace_find_matches(false, &ctx);
        assert_eq!(app.search.find.active_occurrence().unwrap().kind, FindTargetKind::StringText { index: 1, language: 0 });
        app.replace_find_matches(false, &ctx);
        assert!(app.search.find.occurrences.is_empty(), "inserted matches must not re-enter this replacement pass");
        let list = strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap();
        assert_eq!(list.entries[0].texts[0].as_deref(), Some("aa aa"));
        assert_eq!(list.entries[1].texts[0].as_deref(), Some("aa"));
        app.apply_search_command(SearchCommand::FindRunSelection, &ctx);
        assert_eq!(app.search.find.occurrences.len(), 6, "an explicit new Find includes inserted text");
    }

    #[test]
    fn switching_modes_restores_targets_but_keeps_current_scope() {
        let mut app = Baboon::for_test();
        let ctx = egui::Context::default();
        let original = app.search.find.look_in;
        app.search.find.within = FindWithin::AllTags;
        app.set_find_mode(true, &ctx);
        assert_eq!(app.search.find.within, FindWithin::CurrentTag);
        assert!(
            !app.search.find.look_in.field_names
                && app.search.find.look_in.field_values
                && !app.search.find.look_in.blocks
        );
        app.set_find_mode(false, &ctx);
        assert_eq!(app.search.find.within, FindWithin::CurrentTag);
        assert_eq!(app.search.find.look_in, original);
    }

    #[test]
    fn replaced_field_stays_visible_in_filtered_pane_until_find_runs_again() {
        let fixture = LooseKit::new("replace-filtered-pane", "halo3_mcc");
        fixture.write_mcc("model", "damage_effect", |tag| {
            crate::core::document::apply::apply_field_edit(tag, "stun time", "17").unwrap();
        });
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "model.damage_effect");
        let ctx = egui::Context::default();
        ctx.set_global_style(crate::app::ui_kit::foundation_style());
        ctx.set_fonts(crate::app::ui_kit::foundation_fonts());
        app.open_find();
        app.search.find.query = "17".into();
        app.set_find_mode(true, &ctx);
        app.search.find.replacement = "18".into();
        app.run_replace_find(&ctx);
        app.search.find.filter_results = true;
        let draw = |app: &mut Baboon| {
            rendered_pane_texts(app, &ctx, &key, "test").iter().any(|text| text == "stun time")
        };
        assert!(draw(&mut app), "matching field is visible before replacement");
        app.replace_find_matches(true, &ctx);
        app.refresh_find(&ctx);
        assert!(draw(&mut app), "replaced field must remain visible until Find is pressed again");
        app.undo_current_tag();
        app.refresh_find(&ctx);
        assert!(app.search.find.filter_results, "Undo must keep Filter Results active");
        assert!(!app.search.find.replace_needs_refresh());
        assert_eq!(app.search.find.occurrences.len(), 1);
        assert!(draw(&mut app), "the captured field stays visible through Undo");
        app.redo_current_tag();
        app.refresh_find(&ctx);
        assert!(app.search.find.filter_results, "Redo must keep Filter Results active");
        assert!(draw(&mut app), "the captured field stays visible through Redo");
        app.run_replace_find(&ctx);
        assert!(!draw(&mut app), "Find refreshes the filter to the remaining matches");
    }

    #[test]
    fn replaced_string_stays_visible_in_filtered_pane_until_find_runs_again() {
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-filtered-string-pane");
        ctx.set_global_style(crate::app::ui_kit::foundation_style());
        ctx.set_fonts(crate::app::ui_kit::foundation_fonts());
        app.search.find.query = "a a".into();
        app.search.find.replacement = "gone".into();
        app.run_replace_find(&ctx);
        app.search.find.filter_results = true;
        let draw = |app: &mut Baboon| rendered_pane_texts(app, &ctx, &key, "replace-test");
        assert!(draw(&mut app).iter().any(|text| text == "first"));
        app.replace_find_matches(true, &ctx);
        app.refresh_find(&ctx);
        let texts = draw(&mut app);
        assert!(texts.iter().any(|text| text == "first"), "replaced string row stays visible");
        assert!(texts.iter().any(|text| text == "gone"), "the visible row shows its new text");
        app.undo_current_tag();
        app.refresh_find(&ctx);
        assert!(app.search.find.filter_results, "Undo keeps the string filter active");
        assert!(draw(&mut app).iter().any(|text| text == "a a"));
        app.redo_current_tag();
        app.refresh_find(&ctx);
        assert!(app.search.find.filter_results, "Redo keeps the string filter active");
        assert!(draw(&mut app).iter().any(|text| text == "gone"));
        app.run_replace_find(&ctx);
        assert!(!draw(&mut app).iter().any(|text| text == "first"));
    }

    #[test]
    fn undo_restores_replaceable_matches_without_rerunning_find() {
        for all in [false, true] {
            let (mut app, _fixture, ctx, _) = strings_fixture("replace-undo-repeat");
            app.search.find.replacement = "aa".into();
            app.run_replace_find(&ctx);
            app.search.find.filter_results = true;
            let hits = app.search.find.occurrences.clone();
            let active = app.search.find.active;
            for _ in 0..3 {
                app.replace_find_matches(all, &ctx);
                assert_eq!(app.search.replace_links.len(), 1, "discarded Redo releases its match history");
                let replaced_hits = app.search.find.occurrences.clone();
                app.undo_current_tag();
                app.refresh_find(&ctx);
                assert!(!app.search.find.replace_needs_refresh(), "Undo re-enables Replace");
                assert_eq!(app.search.find.occurrences, hits);
                assert_eq!(app.search.find.active, active);
                assert!(app.search.find.filter_results);
                app.redo_current_tag();
                app.refresh_find(&ctx);
                assert!(!app.search.find.replace_needs_refresh());
                assert_eq!(app.search.find.occurrences, replaced_hits);
                app.undo_current_tag();
                app.refresh_find(&ctx);
                assert_eq!(app.search.find.occurrences, hits);
            }
        }
    }

    #[test]
    fn undo_and_redo_move_the_row_highlight_with_the_restored_active_match() {
        let fixture = LooseKit::new("replace-history-row-highlight", "halo3_mcc");
        for path in ["a", "b"] {
            fixture.write_mcc(path, "damage_effect", |tag| {
                crate::core::document::apply::apply_field_edit(tag, "stun time", "17").unwrap();
            });
        }
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        fixture.open(&mut app, "a.damage_effect");
        fixture.open(&mut app, "b.damage_effect");
        let ctx = egui::Context::default();
        app.open_find();
        app.search.find.within = FindWithin::OpenTags;
        app.search.find.query = "17".into();
        app.set_find_mode(true, &ctx);
        app.search.find.replacement = "18".into();
        app.run_replace_find(&ctx);
        let first = app.search.find.active_occurrence().unwrap().clone();
        app.replace_find_matches(false, &ctx);
        let next = app.search.find.active_occurrence().unwrap().clone();
        assert_ne!(first.tag_key, next.tag_key);
        // Undo acts on the document that owns the replacement, while Find's
        // row navigation is still pointing at the next result.
        app.model.kits[0].selected_key = Some(first.tag_key.clone());
        app.undo_current_tag();
        app.refresh_find(&ctx);
        assert_eq!(app.search.find.active_occurrence(), Some(&first));
        let nav = app.references.field_nav.as_ref().unwrap();
        assert_eq!(nav.tag_key, first.tag_key, "yellow row highlight follows Undo's active match");
        assert_eq!(nav.field_path, first.field_path);
        app.redo_current_tag();
        app.refresh_find(&ctx);
        assert_eq!(app.search.find.active_occurrence(), Some(&next));
        let nav = app.references.field_nav.as_ref().unwrap();
        assert_eq!(nav.tag_key, next.tag_key, "yellow row highlight follows Redo's active match");
        assert_eq!(nav.field_path, next.field_path);
    }

    #[test]
    fn undoing_an_older_replacement_in_another_tag_does_not_restore_stale_matches() {
        let fixture = LooseKit::new("replace-history-out-of-order", "halo3_mcc");
        for path in ["a", "b"] {
            fixture.write_mcc(path, "damage_effect", |tag| {
                crate::core::document::apply::apply_field_edit(tag, "stun time", "17").unwrap();
            });
        }
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        fixture.open(&mut app, "a.damage_effect");
        fixture.open(&mut app, "b.damage_effect");
        let ctx = egui::Context::default();
        app.open_find();
        app.search.find.within = FindWithin::OpenTags;
        app.search.find.query = "17".into();
        app.set_find_mode(true, &ctx);
        app.search.find.replacement = "18".into();
        app.run_replace_find(&ctx);
        let first_key = app.search.find.active_occurrence().unwrap().tag_key.clone();
        app.replace_find_matches(false, &ctx);
        app.replace_find_matches(false, &ctx);
        assert!(app.search.find.occurrences.is_empty());
        app.model.kits[0].selected_key = Some(first_key);
        app.undo_current_tag();
        app.refresh_find(&ctx);
        assert!(app.search.find.replace_needs_refresh());
        assert!(app.search.find.occurrences.is_empty(), "the other tag still contains its replacement");
    }

    #[test]
    fn undo_does_not_restore_replace_matches_from_an_obsolete_search() {
        for new_search in [false, true] {
            let (mut app, _fixture, ctx, key) = strings_fixture("replace-history-obsolete");
            app.search.find.replacement = "gone".into();
            app.run_replace_find(&ctx);
            app.replace_find_matches(true, &ctx);
            if new_search {
                app.run_replace_find(&ctx);
                assert!(app.search.find.occurrences.is_empty());
            } else {
                use crate::core::document::apply::{DeferredOps, UndoStep};
                use crate::core::document::ops::StringListOp;
                app.apply_doc_ops(0, &key, "Edit", DeferredOps {
                    string_list_ops: vec![StringListOp { game: Some(GameId::Halo3),
                        edit: StringListEdit::SetText { index: 0, language: 0, text: "unrelated".into() } }],
                    ..Default::default()
                }, UndoStep::Own);
                app.refresh_find(&ctx);
                app.undo_current_tag();
                app.refresh_find(&ctx);
            }
            app.undo_current_tag();
            app.refresh_find(&ctx);
            if !new_search {
                assert!(app.search.find.replace_needs_refresh(), "unrelated edits require Find");
            }
            assert!(app.search.find.occurrences.iter().all(|hit| hit.range.is_empty()));
        }
    }

    #[test]
    fn undoing_string_id_replacement_preserves_captured_row_identity() {
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-id-filter-history");
        ctx.set_fonts(crate::app::ui_kit::foundation_fonts());
        app.search.find.query = "first".into();
        let options = app.search.find.strings.as_mut().unwrap();
        options.ids = true;
        options.text = false;
        app.search.find.replacement = "renamed".into();
        app.run_replace_find(&ctx);
        app.search.find.filter_results = true;
        let cells = app.search.find.replace_run.as_ref().unwrap().matching_cells.clone();
        app.replace_find_matches(false, &ctx);
        app.undo_current_tag();
        app.refresh_find(&ctx);
        assert!(app.search.find.filter_results);
        assert_eq!(app.search.find.replace_run.as_ref().unwrap().matching_cells, cells);
        assert!(!app.search.find.replace_needs_refresh());
        assert!(rendered_pane_texts(&mut app, &ctx, &key, "replace-test").iter().any(|text| text == "first"));
        app.redo_current_tag();
        app.refresh_find(&ctx);
        assert!(app.search.find.filter_results);
        assert_eq!(app.search.find.replace_run.as_ref().unwrap().matching_cells, cells);
        assert!(rendered_pane_texts(&mut app, &ctx, &key, "replace-test").iter().any(|text| text == "renamed"));
    }

    #[test]
    fn replace_waits_for_find_and_retains_filtered_rows_until_reload() {
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-capture");
        app.search.find.replacement = "gone".into();
        app.refresh_find(&ctx);
        assert!(app.search.find.occurrences.is_empty(), "Current Tag Replace waits for Find");
        app.replace_find_matches(true, &ctx);
        assert!(app.model.status.contains("Press Find"));
        app.run_replace_find(&ctx);
        assert_eq!(app.search.find.occurrences.len(), 3);
        app.search.find.filter_results = true;
        let cells = app.search.find.replace_run.as_ref().unwrap().matching_cells.clone();
        app.replace_find_matches(true, &ctx);
        app.refresh_find(&ctx);
        assert!(app.search.find.occurrences.is_empty());
        let snapshot = ctx.data(|data| data.get_temp::<std::sync::Arc<FindRenderSnapshot>>(find_render_snapshot_id())).unwrap();
        assert!(snapshot.filter_results);
        assert_eq!(snapshot.matching_cells, cells, "replaced rows remain visible");
        assert!(snapshot.matching_cells.contains(&(key, String::new(), FindTargetKind::StringText { index: 0, language: 0 })));
        app.run_replace_find(&ctx);
        assert!(app.search.find.replace_run.as_ref().unwrap().matching_cells.is_empty(), "only Find reloads membership");
    }

    #[test]
    fn replace_query_edits_wait_and_replacement_text_preserves_the_capture() {
        let (mut app, _fixture, ctx, _key) = strings_fixture("replace-query-capture");
        app.run_replace_find(&ctx);
        let hits = app.search.find.occurrences.clone();
        app.apply_search_command(SearchCommand::ReplacementChanged("aa".into()), &ctx);
        app.refresh_find(&ctx);
        assert_eq!(app.search.find.occurrences, hits);
        let mut query = app.search.find.query();
        query.text = "français".into();
        app.apply_search_command(SearchCommand::FindChanged(query), &ctx);
        app.refresh_find(&ctx);
        assert!(app.search.find.occurrences.is_empty());
        assert!(app.search.find.replace_needs_refresh());
    }

    #[test]
    fn switching_from_find_carries_string_results_active_match_and_filter() {
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-carry-current");
        app.set_find_mode(false, &ctx);
        assert_eq!(app.search.find.occurrences.len(), 3);
        app.search.find.active = Some(2);
        app.search.find.filter_results = true;
        let hits = app.search.find.occurrences.clone();
        app.set_find_mode(true, &ctx);
        assert_eq!(app.search.find.occurrences, hits);
        assert_eq!(app.search.find.active, Some(2));
        assert!(app.search.find.filter_results);
        assert!(!app.search.find.replace_needs_refresh());
        app.search.find.replacement = "carried".into();
        app.replace_find_matches(false, &ctx);
        let list = strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap();
        assert_eq!(list.entries[1].texts[0].as_deref(), Some("carried"));
        assert_eq!(list.entries[0].texts[0].as_deref(), Some("a a"));
    }

    #[test]
    fn switching_from_selected_rows_find_reuses_the_executed_capture() {
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-carry-selected");
        app.set_find_mode(false, &ctx);
        app.search.find.strings.as_mut().unwrap().selected_rows = true;
        app.run_selected_string_find(&ctx);
        app.search.find.active = Some(1);
        let hits = app.search.find.occurrences.clone();
        app.set_find_mode(true, &ctx);
        assert_eq!(app.search.find.occurrences, hits);
        assert_eq!(app.search.find.active, Some(1));
        assert!(!app.search.find.replace_needs_refresh());
        app.search.find.replacement = "carried".into();
        app.replace_find_matches(true, &ctx);
        let list = strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap();
        assert_eq!(list.entries[0].texts[0].as_deref(), Some("carried carried"));
        assert_eq!(list.entries[1].texts[0].as_deref(), Some("a"));
    }

    #[test]
    fn changing_from_all_tags_to_replace_does_not_carry_results() {
        let (mut app, _fixture, ctx, _key) = strings_fixture("replace-carry-all-tags");
        app.set_find_mode(false, &ctx);
        assert!(!app.search.find.occurrences.is_empty());
        app.search.find.within = FindWithin::AllTags;
        app.set_find_mode(true, &ctx);
        assert_eq!(app.search.find.within, FindWithin::CurrentTag);
        assert!(app.search.find.occurrences.is_empty());
        assert!(app.search.find.replace_needs_refresh());
    }

    #[test]
    fn switching_from_field_find_carries_values_but_not_label_only_results() {
        let fixture = LooseKit::new("replace-carry-fields", "halo3_mcc");
        fixture.write_mcc("a", "damage_effect", |tag| {
            crate::core::document::apply::apply_field_edit(tag, "stun time", "17").unwrap();
        });
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "a.damage_effect");
        let ctx = egui::Context::default();
        app.open_find();
        app.search.find.query = "17".into();
        app.refresh_find(&ctx);
        let hits = app.search.find.occurrences.clone();
        assert_eq!(hits.len(), 1);
        app.search.find.filter_results = true;
        app.set_find_mode(true, &ctx);
        assert_eq!(app.search.find.occurrences, hits);
        assert!(app.search.find.filter_results && !app.search.find.replace_needs_refresh());
        app.search.find.replacement = "18".into();
        app.replace_find_matches(true, &ctx);
        let doc = &app.model.kits[0].parsed_tags[&key];
        assert_eq!(format_foundation_scalar_value(app.model.names(), &doc.tag.root().field_path("stun time").unwrap().value().unwrap()), "18");
        app.set_find_mode(false, &ctx);
        let mut query = app.search.find.query();
        query.text = "stun".into();
        query.look_in = FindLookIn { field_names: true, field_values: false, blocks: false };
        app.apply_search_command(SearchCommand::FindChanged(query), &ctx);
        assert!(!app.search.find.occurrences.is_empty());
        assert!(app.search.find.occurrences.iter().all(|hit| hit.kind == FindTargetKind::Label));
        app.set_find_mode(true, &ctx);
        assert!(app.search.find.occurrences.is_empty());
        assert!(app.search.find.replace_needs_refresh());
    }

    #[test]
    fn editing_a_captured_string_keeps_it_visible_and_requires_find_before_replacing() {
        use crate::core::document::apply::{DeferredOps, UndoStep};
        use crate::core::document::ops::StringListOp;
        let (mut app, _fixture, ctx, key) = strings_fixture("replace-edited-capture");
        app.run_replace_find(&ctx);
        app.search.find.filter_results = true;
        app.apply_doc_ops(0, &key, "Edit", DeferredOps {
            string_list_ops: vec![StringListOp { game: Some(GameId::Halo3),
                edit: StringListEdit::SetText { index: 0, language: 0, text: "gone".into() } }],
            ..Default::default()
        }, UndoStep::Own);
        app.refresh_find(&ctx);
        let snapshot = ctx.data(|data| data.get_temp::<std::sync::Arc<FindRenderSnapshot>>(find_render_snapshot_id())).unwrap();
        let cell = (key.clone(), String::new(), FindTargetKind::StringText { index: 0, language: 0 });
        assert!(snapshot.filter_results && snapshot.matching_cells.contains(&cell));
        assert!(app.search.find.replace_needs_refresh());
        app.replace_find_matches(true, &ctx);
        assert!(app.model.status.contains("Press Find"));
        app.run_replace_find(&ctx);
        assert!(!app.search.find.replace_run.as_ref().unwrap().matching_cells.contains(&cell));
        assert_eq!(app.search.find.occurrences.len(), 1);
    }

    #[test]
    fn replace_current_advances_using_converted_unicode_length() {
        let (mut app, _fixture, ctx, _key) = strings_fixture("replace-unicode-advance");
        app.search.find.replacement = "&U+00E9".into();
        app.run_replace_find(&ctx);
        app.replace_find_matches(false, &ctx);
        let active = app.search.find.active_occurrence().unwrap();
        assert_eq!(
            active.kind,
            FindTargetKind::StringText {
                index: 0,
                language: 0
            }
        );
        assert_eq!(active.range, 3..4);
    }

    #[test]
    fn all_tags_waits_for_find_and_query_changes_cancel_the_search() {
        let fixture = LooseKit::new("manual-all-tags", "halo3_mcc");
        fixture.write_mcc("a", "damage_effect", |_| {});
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        let ctx = egui::Context::default();
        app.open_find();
        let mut query = app.search.find.query();
        query.text = "stun".into();
        query.within = FindWithin::AllTags;
        app.apply_search_command(SearchCommand::FindChanged(query.clone()), &ctx);
        assert!(!app.search.find.searching);
        assert!(!app.model.kits[0].scanning_entries);
        assert!(app.search.find.occurrences.is_empty());
        app.apply_search_command(SearchCommand::FindRunSelection, &ctx);
        assert!(app.search.find.all_requested);
        assert!(app.search.find.searching);
        let request = app.search.find.all_request_id;
        query.text = "checksum".into();
        app.apply_search_command(SearchCommand::FindChanged(query), &ctx);
        assert!(!app.search.find.all_requested);
        assert!(!app.search.find.searching);
        assert!(app.search.find.all_request_id > request);
        assert!(app.search.find.occurrences.is_empty());
    }

    #[test]
    fn replace_across_open_tags_is_one_coordinated_undo_and_redo() {
        let fixture = LooseKit::new("replace-open-tags", "halo3_mcc");
        for path in ["a", "b", "closed"] {
            fixture.write_mcc(path, "damage_effect", |tag| {
                crate::core::document::apply::apply_field_edit(tag, "stun time", "17")
                    .unwrap()
            });
        }
        let mut app = Baboon::for_test();
        fixture.install(&mut app);
        let a = fixture.open(&mut app, "a.damage_effect");
        let b = fixture.open(&mut app, "b.damage_effect");
        let ctx = egui::Context::default();
        app.open_find();
        app.search.find.within = FindWithin::OpenTags;
        app.search.find.query = "17".into();
        app.set_find_mode(true, &ctx);
        app.search.find.replacement = "18".into();
        assert!(app.search.find.occurrences.is_empty(), "Open Tags Replace waits for Find");
        app.run_replace_find(&ctx);
        app.search.find.filter_results = true;
        let captured = app.search.find.replace_run.as_ref().unwrap().matching_cells.clone();
        let bytes = |app: &Baboon, key: &str| {
            app.model.kits[0].parsed_tags[key]
                .tag
                .write_to_bytes()
                .unwrap()
        };
        let before = [bytes(&app, &a), bytes(&app, &b)];
        app.replace_find_matches(true, &ctx);
        assert_eq!(
            app.model.status,
            "Replaced 2 occurrences."
        );
        let after = [bytes(&app, &a), bytes(&app, &b)];
        assert_ne!(after, before);
        app.refresh_find(&ctx);
        let snapshot = ctx.data(|data| data.get_temp::<std::sync::Arc<FindRenderSnapshot>>(find_render_snapshot_id())).unwrap();
        assert!(snapshot.filter_results);
        assert_eq!(snapshot.matching_cells, captured, "replaced fields remain in Filter Results");
        assert!(app.search.find.occurrences.is_empty());
        assert_eq!(
            std::fs::read(fixture.path("a.damage_effect")).unwrap(),
            before[0]
        );
        app.undo_current_tag();
        assert_eq!([bytes(&app, &a), bytes(&app, &b)], before);
        app.refresh_find(&ctx);
        assert!(app.search.find.filter_results, "linked Undo preserves the filter");
        assert_eq!(app.search.find.replace_run.as_ref().unwrap().matching_cells, captured);
        assert!(!app.search.find.replace_needs_refresh());
        assert_eq!(app.search.find.occurrences.len(), 2);
        app.redo_current_tag();
        assert_eq!([bytes(&app, &a), bytes(&app, &b)], after);
        app.refresh_find(&ctx);
        assert!(app.search.find.filter_results, "linked Redo preserves the filter");
    }
}
