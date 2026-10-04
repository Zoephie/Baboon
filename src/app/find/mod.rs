//! Exact field matching and Find-dialog navigation.

use super::*;
use std::fmt::Write as _;

/// Temporary egui-memory key for the Find data shared with field widgets.
pub(in crate::app) fn find_render_snapshot_id() -> egui::Id {
    egui::Id::new("find_render_snapshot")
}

/// Temporary egui-memory key identifying the Foundation cell being rendered.
pub(in crate::app) fn find_render_cell_id() -> egui::Id {
    egui::Id::new("find_render_cell")
}

/// Return non-overlapping byte ranges matching `query` in `text`.
pub(in crate::app) fn find_text_ranges(
    text: &str,
    query: &str,
    match_case: bool,
    whole_word: bool,
) -> Vec<std::ops::Range<usize>> {
    find_text_matches(text, query, match_case, whole_word).collect()
}

/// Whether `query` matches anywhere in `text`, without allocating.
pub(in crate::app) fn find_text_has_match(
    text: &str,
    query: &str,
    match_case: bool,
    whole_word: bool,
) -> bool {
    find_text_matches(text, query, match_case, whole_word)
        .next()
        .is_some()
}

/// The matches behind [`find_text_ranges`], found lazily and without copying
/// either string: Find tests every field of a tag against the query, and
/// lowercasing each candidate cost more than the whole tree traversal.
///
/// Case folding is ASCII-only, so a match's byte range is the same in the
/// folded and original text. A match rejected by `whole_word` still consumes
/// its bytes; the search resumes at its end.
fn find_text_matches<'a>(
    text: &'a str,
    query: &'a str,
    match_case: bool,
    whole_word: bool,
) -> impl Iterator<Item = std::ops::Range<usize>> + 'a {
    let haystack = text.as_bytes();
    let needle = query.as_bytes();
    let mut offset = 0;
    std::iter::from_fn(move || {
        if needle.is_empty() {
            return None;
        }
        loop {
            let found = haystack
                .get(offset..)?
                .windows(needle.len())
                .position(|window| {
                    if match_case {
                        window == needle
                    } else {
                        window.eq_ignore_ascii_case(needle)
                    }
                })?;
            let start = offset + found;
            let end = start + needle.len();
            offset = end;
            let boundary_ok = !whole_word
                || (!text[..start]
                    .chars()
                    .next_back()
                    .is_some_and(is_find_word_char)
                    && !text[end..].chars().next().is_some_and(is_find_word_char));
            if boundary_ok {
                return Some(start..end);
            }
        }
    })
}

fn is_find_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// What Find needs from one struct definition: its fields' labels and path
/// segments, where its injected documentation falls, and whether each of
/// those matches the query. None of it depends on the element being visited,
/// and a scenario repeats a few thousand definitions across millions of
/// fields, so a walk builds each plan once and reuses it for every instance.
pub(in crate::app) struct FindStructPlan<'a> {
    pub(in crate::app) fields: Vec<FindFieldPlan>,
    pub(in crate::app) entries: &'a [DefEntry],
    /// Documentation entries rendered after the last field.
    pub(in crate::app) trailing_docs: std::ops::Range<usize>,
    /// Per documentation entry: whether its cleaned title / body match.
    pub(in crate::app) doc_title_matches: Vec<bool>,
    pub(in crate::app) doc_body_matches: Vec<bool>,
}

pub(in crate::app) struct FindFieldPlan {
    /// Documentation entries rendered just before this field.
    pub(in crate::app) docs_before: std::ops::Range<usize>,
    /// Markup-free name: the canonical path segment, and the renderer
    /// segment of an inherited-parent wrapper.
    pub(in crate::app) clean: Box<str>,
    /// Renderer path segment, `clean#ordinal` (see `append_field_path_for`).
    pub(in crate::app) segment: Box<str>,
    /// The label Find matches: the block title for blocks and arrays, the
    /// clean name otherwise.
    pub(in crate::app) label: Box<str>,
    pub(in crate::app) label_matches: bool,
    pub(in crate::app) explanation_matches: bool,
    pub(in crate::app) inherited_parent: bool,
    pub(in crate::app) is_block: bool,
    pub(in crate::app) is_documentation: bool,
}

/// Plans for one walk, keyed by struct definition index within the tag's layout.
pub(in crate::app) struct FindPlans<'a> {
    docs: Option<&'a DefDocs>,
    query: &'a str,
    match_case: bool,
    whole_word: bool,
    by_definition: HashMap<usize, std::rc::Rc<FindStructPlan<'a>>>,
}

impl<'a> FindPlans<'a> {
    pub(in crate::app) fn new(
        docs: Option<&'a DefDocs>,
        query: &'a str,
        match_case: bool,
        whole_word: bool,
    ) -> Self {
        Self {
            docs,
            query,
            match_case,
            whole_word,
            by_definition: HashMap::new(),
        }
    }

    pub(in crate::app) fn matches(&self, text: &str) -> bool {
        find_text_has_match(text, self.query, self.match_case, self.whole_word)
    }

    pub(in crate::app) fn plan(&mut self, tag_struct: &TagStruct<'_>) -> std::rc::Rc<FindStructPlan<'a>> {
        let index = tag_struct.definition().index();
        if let Some(plan) = self.by_definition.get(&index) {
            return plan.clone();
        }
        let plan = std::rc::Rc::new(self.build(tag_struct));
        self.by_definition.insert(index, plan.clone());
        plan
    }

    fn build(&self, tag_struct: &TagStruct<'_>) -> FindStructPlan<'a> {
        let entries: &'a [DefEntry] = self
            .docs
            .map(|docs| docs.entries_for_struct(tag_struct))
            .unwrap_or(&[]);
        let mut doc_title_matches = Vec::with_capacity(entries.len());
        let mut doc_body_matches = Vec::with_capacity(entries.len());
        for entry in entries {
            let (title, body) = match entry {
                DefEntry::Explanation { title, body } => (
                    self.matches(&clean_field_name(title)),
                    self.matches(body.trim_end()),
                ),
                _ => (false, false),
            };
            doc_title_matches.push(title);
            doc_body_matches.push(body);
        }
        let mut doc_cursor = 0usize;
        let fields = tag_struct
            .fields()
            .map(|field| {
                let mut docs_before = 0..0;
                if let Some(match_idx) = (doc_cursor..entries.len()).find(|&index| {
                    matches!(&entries[index], DefEntry::Field { clean_name, .. } if clean_name == field.name())
                }) {
                    docs_before = doc_cursor..match_idx;
                    doc_cursor = match_idx + 1;
                }
                let clean = field.clean_name().into_owned();
                let is_block = field.as_block().is_some() || field.as_array().is_some();
                let is_documentation = field.field_type() == TagFieldType::Explanation;
                let label = if is_block {
                    foundation_block_title(field.name())
                } else {
                    clean.clone()
                };
                FindFieldPlan {
                    docs_before,
                    segment: format!("{clean}#{}", field.ordinal()).into(),
                    label_matches: self.matches(&label),
                    explanation_matches: field
                        .explanation()
                        .is_some_and(|body| self.matches(body.trim_end())),
                    inherited_parent: field.as_struct().is_some()
                        && is_inherited_parent_name(field.name()),
                    is_block,
                    is_documentation,
                    label: label.into(),
                    clean: clean.into(),
                }
            })
            .collect();
        FindStructPlan {
            fields,
            entries,
            trailing_docs: doc_cursor..entries.len(),
            doc_title_matches,
            doc_body_matches,
        }
    }
}

/// Collect exact label/value occurrences from a parsed tag in render order.
pub(in crate::app) fn collect_find_occurrences(
    tag: &TagFile,
    tag_key: &str,
    names: &TagNameIndex,
    docs: Option<&DefDocs>,
    query: &str,
    look_in: FindLookIn,
    match_case: bool,
    whole_word: bool,
) -> Vec<FindOccurrence> {
    let mut out = Vec::new();
    let mut walk = FindWalk {
        tag_key,
        names,
        plans: FindPlans::new(docs, query, match_case, whole_word),
        query,
        look_in,
        match_case,
        whole_word,
        path: String::new(),
        out: &mut out,
    };
    walk.collect_struct(tag.root(), true);
    out
}

/// One occurrence walk. `path` is the renderer path of the node being
/// visited, grown and truncated in place: a scenario has millions of fields,
/// and only the few that match need a path of their own.
struct FindWalk<'a> {
    tag_key: &'a str,
    names: &'a TagNameIndex,
    plans: FindPlans<'a>,
    query: &'a str,
    look_in: FindLookIn,
    match_case: bool,
    whole_word: bool,
    path: String,
    out: &'a mut Vec<FindOccurrence>,
}

impl FindWalk<'_> {
    fn collect_struct(&mut self, tag_struct: TagStruct<'_>, following_inherited_chain: bool) {
        let plan = self.plans.plan(&tag_struct);
        for (field, field_plan) in tag_struct.fields().zip(&plan.fields) {
            self.documentation(&plan, field_plan.docs_before.clone());
            let inherited_wrapper = following_inherited_chain && field_plan.inherited_parent;
            let parent_len = self.path.len();
            if parent_len > 0 {
                self.path.push('/');
            }
            self.path.push_str(if inherited_wrapper {
                &field_plan.clean
            } else {
                &field_plan.segment
            });
            // TODO(find-phantom-results): these schema entries can inflate the counter
            // with matches that have no corresponding rendered widget. Audit inherited
            // parent wrapper labels skipped by Foundation, advanced/internal fields
            // hidden by the editor, and inline function structures rendered as one
            // consolidated row.
            let label_kind = if field_plan.is_documentation {
                FindTargetKind::Documentation
            } else if field_plan.is_block {
                FindTargetKind::Block
            } else {
                FindTargetKind::Label
            };
            let include_label = if field_plan.is_block || field_plan.is_documentation {
                self.look_in.includes_blocks()
            } else {
                self.look_in.includes_field_names()
            };
            if include_label {
                if field_plan.label_matches {
                    self.append(label_kind, &field_plan.label);
                }
                if field_plan.is_documentation && field_plan.explanation_matches {
                    if let Some(body) = field.explanation() {
                        self.append(FindTargetKind::Documentation, body.trim_end());
                    }
                }
            }
            if let Some(block) = field.as_block() {
                for index in 0..block.len() {
                    if let Some(child) = block.element(index) {
                        self.collect_element(child, index);
                    }
                }
            } else if let Some(array) = field.as_array() {
                for index in 0..array.len() {
                    if let Some(child) = array.element(index) {
                        self.collect_element(child, index);
                    }
                }
            } else if let Some(child) = field.as_struct() {
                self.collect_struct(child, inherited_wrapper);
            } else if self.look_in.includes_values() {
                if let Some(value) = field.value() {
                    let text = format_foundation_scalar_value(self.names, &value);
                    self.append(FindTargetKind::Value, &text);
                }
            }
            self.path.truncate(parent_len);
        }
        self.documentation(&plan, plan.trailing_docs.clone());
    }

    fn collect_element(&mut self, element: TagStruct<'_>, index: usize) {
        let len = self.path.len();
        let _ = write!(self.path, "[{index}]");
        self.collect_struct(element, false);
        self.path.truncate(len);
    }

    /// Injected documentation rows, addressed like `documentation_path`.
    fn documentation(&mut self, plan: &FindStructPlan<'_>, range: std::ops::Range<usize>) {
        if !self.look_in.includes_blocks() {
            return;
        }
        for index in range {
            let (title_hit, body_hit) = (plan.doc_title_matches[index], plan.doc_body_matches[index]);
            if !(title_hit || body_hit) {
                continue;
            }
            let DefEntry::Explanation { title, body } = &plan.entries[index] else {
                continue;
            };
            let len = self.path.len();
            if len > 0 {
                self.path.push('/');
            }
            let _ = write!(self.path, "@documentation {index}");
            if title_hit {
                self.append(FindTargetKind::Documentation, &clean_field_name(title));
            }
            if body_hit {
                self.append(FindTargetKind::Documentation, body.trim_end());
            }
            self.path.truncate(len);
        }
    }

    /// Record every match of the query in `text` at the current path.
    fn append(&mut self, kind: FindTargetKind, text: &str) {
        for range in find_text_matches(text, self.query, self.match_case, self.whole_word) {
            self.out.push(FindOccurrence {
                tag_key: self.tag_key.to_owned(),
                field_path: self.path.clone(),
                kind,
                text: text.to_owned(),
                range,
            });
        }
    }
}

impl Baboon {
    /// Refresh synchronous Current/Open Tag results and publish render highlights.
    ///
    /// Called every frame. The walk behind the results is far slower than a
    /// frame on a large tag, so it reruns only when [`Self::find_results_key`]
    /// changes; otherwise only the cheap render snapshot is republished.
    pub(super) fn refresh_find(&mut self, ctx: &egui::Context) {
        if !self.find.open || self.find.query.is_empty() || self.find.look_in.is_empty() {
            self.find.occurrences.clear();
            self.find.active = None;
            self.find.results_key = None;
            self.find.matching_cells = Default::default();
            ctx.data_mut(|data| data.remove::<std::sync::Arc<FindRenderSnapshot>>(find_render_snapshot_id()));
            return;
        }
        let key = self.find_results_key();
        if key.is_some() && key == self.find.results_key {
            self.publish_find_snapshot(ctx);
            return;
        }
        let old_active = self.find.active_occurrence().cloned();
        if self.find.within == FindWithin::AllTags {
            self.refresh_all_tag_find(ctx);
        } else {
            self.refresh_open_tag_find();
        }
        // Taken after the refresh: starting an All Tags search moves the
        // request id and `searching`, both part of the key.
        self.find.results_key = self.find_results_key();
        self.finish_find_refresh(ctx, old_active);
    }

    /// Everything Find's results depend on, or `None` while they cannot be
    /// cached (All Tags waiting on the tag index).
    fn find_results_key(&self) -> Option<String> {
        use std::fmt::Write as _;
        let kit = &self.kits[self.active];
        let mut key = format!(
            "{:?}|{}|{:?}|{:?}|{}|{}|{}|{}|{}",
            kit.id,
            kit.generation,
            self.find.within,
            self.find.look_in,
            self.find.match_case,
            self.find.whole_word,
            self.find.all_request_id,
            self.find.searching,
            self.find.query,
        );
        let keys: Vec<&String> = match self.find.within {
            FindWithin::CurrentTag => kit.selected_key.iter().collect(),
            FindWithin::OpenTags => kit.open_tabs.iter().collect(),
            FindWithin::AllTags => {
                let source = kit.source.as_ref()?;
                if matches!(source.source, TagSource::LooseFolder { .. })
                    && source.all_entries.is_empty()
                {
                    return None;
                }
                let _ = write!(
                    key,
                    "|{}|{}",
                    source.entries.len(),
                    source.all_entries.len()
                );
                let mut keys = kit.parsed_tags.keys().collect::<Vec<_>>();
                keys.sort();
                keys
            }
        };
        for tag_key in keys {
            let _ = write!(key, "\u{1f}{tag_key}");
            if let Some(doc) = kit.parsed_tags.get(tag_key) {
                let _ = write!(key, "@{:?}", doc.content_stamp());
            }
        }
        Some(key)
    }

    fn refresh_open_tag_find(&mut self) {
        let keys = match self.find.within {
            FindWithin::CurrentTag => self.kits[self.active]
                .selected_key
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            FindWithin::OpenTags => self.kits[self.active].open_tabs.clone(),
            FindWithin::AllTags => unreachable!(),
        };
        let mut occurrences = Vec::new();
        for key in keys {
            let Some(entry) = self.entry_for_key(&key).cloned() else {
                continue;
            };
            if !supports_field_search(&entry) {
                continue;
            }
            let docs = self.def_docs_for_entry(self.active, &entry);
            let Some(doc) = self.kits[self.active].parsed_tags.get(&key) else {
                continue;
            };
            occurrences.extend(collect_find_occurrences(
                &doc.tag,
                &key,
                self.names(),
                docs.as_deref(),
                &self.find.query,
                self.find.look_in,
                self.find.match_case,
                self.find.whole_word,
            ));
        }
        self.find.occurrences = occurrences;
    }

    fn finish_find_refresh(&mut self, ctx: &egui::Context, old_active: Option<FindOccurrence>) {
        self.find.active = old_active
            .and_then(|active| self.find.occurrences.iter().position(|hit| *hit == active))
            .or_else(|| (!self.find.occurrences.is_empty()).then_some(0));
        let parsed_tags = &self.kits[self.active].parsed_tags;
        self.find.matching_cells = std::sync::Arc::new(
            self.find
                .occurrences
                .iter()
                .filter(|hit| parsed_tags.contains_key(&hit.tag_key))
                .map(|hit| (hit.tag_key.clone(), hit.field_path.clone(), hit.kind))
                .collect(),
        );
        self.publish_find_snapshot(ctx);
    }

    /// Install the render snapshot. Cheap: the match set is shared, and the
    /// active occurrence is re-read because stepping moves it between walks.
    fn publish_find_snapshot(&self, ctx: &egui::Context) {
        let snapshot = std::sync::Arc::new(FindRenderSnapshot {
            query: self.find.query.clone(),
            match_case: self.find.match_case,
            whole_word: self.find.whole_word,
            active: self.find.active_occurrence().cloned(),
            matching_cells: self.find.matching_cells.clone(),
        });
        ctx.data_mut(|data| data.insert_temp(find_render_snapshot_id(), snapshot));
    }

    fn refresh_all_tag_find(&mut self, ctx: &egui::Context) {
        let needs_full_scan = self.source().is_some_and(|source| {
            matches!(source.source, TagSource::LooseFolder { .. }) && source.all_entries.is_empty()
        });
        if needs_full_scan {
            if !self.kits[self.active].scanning_entries {
                self.begin_scan_all_entries_with_label(ctx.clone(), "Indexing tags for Find...");
            }
            self.find.searching = true;
            self.find.progress = self.kits[self.active]
                .index_jobs
                .entry_progress
                .as_ref()
                .map(|progress| (progress.processed, progress.total));
            self.find.occurrences.clear();
            return;
        }
        if self.source().is_none() {
            self.find.occurrences.clear();
            return;
        }
        let mut open_keys = self.kits[self.active]
            .parsed_tags
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        open_keys.sort();
        // Read in place: this runs on every edit while Find is open, and the
        // entry list is the whole kit. It used to be cloned, all of it, before
        // the signature said whether anything needed re-searching at all.
        let (signature, fresh) = {
            let kit = &self.kits[self.active];
            let source = kit.source.as_ref().expect("checked above");
            let entries = if source.all_entries.is_empty() {
                &source.entries
            } else {
                &source.all_entries
            };
            let signature = format!(
                "{}|{:?}|{}|{}|{}|{}|{}|{}",
                kit.generation,
                self.find.look_in,
                self.find.match_case,
                self.find.whole_word,
                self.find.query,
                entries.len(),
                entries
                    .first()
                    .map(|entry| entry.key.as_str())
                    .unwrap_or(""),
                open_keys.join("\u{1f}"),
            );
            let fresh =
                (self.find.all_signature.as_deref() != Some(signature.as_str())).then(|| {
                    let closed_entries = entries
                        .iter()
                        .filter(|entry| !kit.parsed_tags.contains_key(&entry.key))
                        .cloned()
                        .collect::<Vec<_>>();
                    let order = entries.iter().map(|entry| entry.key.clone()).collect();
                    (closed_entries, order)
                });
            (signature, fresh)
        };
        if let Some((closed_entries, order)) = fresh {
            self.find.all_signature = Some(signature);
            self.find.all_order = order;
            self.begin_all_tag_find(ctx.clone(), closed_entries);
        }

        let mut by_key: HashMap<String, Vec<FindOccurrence>> = HashMap::new();
        for hit in &self.find.all_closed_occurrences {
            if !self.kits[self.active]
                .parsed_tags
                .contains_key(&hit.tag_key)
            {
                by_key
                    .entry(hit.tag_key.clone())
                    .or_default()
                    .push(hit.clone());
            }
        }
        for key in open_keys {
            let Some(entry) = self.entry_for_key(&key).cloned() else {
                continue;
            };
            if !supports_field_search(&entry) {
                continue;
            }
            let docs = self.def_docs_for_entry(self.active, &entry);
            let Some(doc) = self.kits[self.active].parsed_tags.get(&key) else {
                continue;
            };
            by_key.insert(
                key.clone(),
                collect_find_occurrences(
                    &doc.tag,
                    &key,
                    self.names(),
                    docs.as_deref(),
                    &self.find.query,
                    self.find.look_in,
                    self.find.match_case,
                    self.find.whole_word,
                ),
            );
        }
        self.find.occurrences = order_find_occurrences(&self.find.all_order, by_key);
    }

    fn begin_all_tag_find(&mut self, ctx: egui::Context, entries: Vec<TagEntry>) {
        let Some(source) = self.kits[self.active].source.as_ref() else {
            return;
        };
        self.find.all_request_id = self.find.all_request_id.wrapping_add(1);
        let request_id = self.find.all_request_id;
        let stamp = self.kit_stamp();
        let tag_source = source.source.clone();
        let documentation_source = match (&source.source, source.game) {
            (
                TagSource::LooseFolder {
                    definitions_root, ..
                },
                Some(game),
            ) => Some((definitions_root.clone(), game)),
            _ => None,
        };
        let names = self.names().clone();
        let query = self.find.query.clone();
        let look_in = self.find.look_in;
        let match_case = self.find.match_case;
        let whole_word = self.find.whole_word;
        let total = entries.len();
        let tx = self.tx.clone();
        self.find.all_closed_occurrences.clear();
        self.find.searching = true;
        self.find.progress = Some((0, total));
        self.find.unreadable = 0;
        let worker_ctx = ctx.clone();
        let progress_tx = tx.clone();
        spawn_worker(
            &tx,
            &worker_ctx,
            move || {
                let tx = progress_tx;
                let mut occurrences = Vec::new();
                let mut unreadable = 0;
                let mut docs_by_group = HashMap::new();
                for (index, entry) in entries.into_iter().enumerate() {
                    if supports_field_search(&entry) {
                        let docs = documentation_source.as_ref().and_then(|(root, game)| {
                            let group = names
                                .name_for(entry.group_tag)
                                .or_else(|| group_tag_to_extension(entry.group_tag))?;
                            Some(
                                docs_by_group
                                    .entry(entry.group_tag)
                                    .or_insert_with(|| build_def_docs(root, *game, group)),
                            )
                        });
                        match crate::core::source::read_entry(&tag_source, &entry) {
                            Ok(tag) => occurrences.extend(collect_find_occurrences(
                                &tag,
                                &entry.key,
                                &names,
                                docs.map(|docs| &*docs),
                                &query,
                                look_in,
                                match_case,
                                whole_word,
                            )),
                            Err(_) => unreadable += 1,
                        }
                    }
                    let processed = index + 1;
                    if processed == total || processed % 32 == 0 {
                        let _ = tx.send(WorkerMessage::FindAllProgress {
                            stamp,
                            request_id,
                            processed,
                            total,
                        });
                        ctx.request_repaint();
                    }
                }
                WorkerMessage::FindAllFinished {
                    stamp,
                    request_id,
                    occurrences,
                    unreadable,
                }
            },
            // A crashed search ends like an empty one, so Find stops waiting.
            move |_| WorkerMessage::FindAllFinished {
                stamp,
                request_id,
                occurrences: Vec::new(),
                unreadable: 0,
            },
        );
    }

    /// Move the active Find occurrence with wraparound and reveal its field.
    pub(super) fn step_find(&mut self, ctx: &egui::Context, delta: isize) {
        let len = self.find.occurrences.len();
        if len == 0 {
            self.find.active = None;
            return;
        }
        let current = self.find.active.unwrap_or(0) as isize;
        self.find.active = Some((current + delta).rem_euclid(len as isize) as usize);
        let Some(hit) = self.find.active_occurrence().cloned() else {
            return;
        };
        self.activate_find_occurrence(ctx, hit);
    }

    /// Select a Find result's tag and navigate immediately or after its load completes.
    pub(super) fn activate_find_occurrence(&mut self, ctx: &egui::Context, hit: FindOccurrence) {
        if self.kits[self.active].selected_key.as_deref() != Some(hit.tag_key.as_str()) {
            self.select_entry(hit.tag_key.clone(), ctx.clone());
        }
        if !self.kits[self.active]
            .parsed_tags
            .contains_key(&hit.tag_key)
        {
            self.pending_find_jump = Some(hit);
            return;
        }
        if let Some(entry) = self.entry_for_key(&hit.tag_key) {
            let source_game = self.kits[self.active]
                .source
                .as_ref()
                .and_then(|source| source.game);
            if is_previewable_geometry_group_for_game(entry.group_tag, self.names(), source_game) {
                self.kits[self.active]
                    .model_previews
                    .entry(hit.tag_key.clone())
                    .or_default()
                    .active_tab = ModelTagPanelTab::Fields;
            }
        }
        self.pending_find_jump = None;
        self.navigate_to_field(ctx, &hit.tag_key, &hit.field_path);
        if hit.kind != FindTargetKind::Value {
            ctx.data_mut(|data| data.insert_temp(jump_target_id(), hit.field_path));
        }
    }
}

fn order_find_occurrences(
    order: &[String],
    mut by_key: HashMap<String, Vec<FindOccurrence>>,
) -> Vec<FindOccurrence> {
    order
        .iter()
        .filter_map(|key| by_key.remove(key))
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests;
