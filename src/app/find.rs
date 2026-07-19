//! Exact field matching and Find-dialog navigation.

use super::*;

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
    if query.is_empty() {
        return Vec::new();
    }
    let haystack = if match_case {
        text.to_owned()
    } else {
        text.to_ascii_lowercase()
    };
    let needle = if match_case {
        query.to_owned()
    } else {
        query.to_ascii_lowercase()
    };
    let mut ranges = Vec::new();
    let mut offset = 0;
    while offset <= haystack.len().saturating_sub(needle.len()) {
        let Some(found) = haystack[offset..].find(&needle) else {
            break;
        };
        let start = offset + found;
        let end = start + needle.len();
        let boundary_ok = !whole_word
            || (!text[..start]
                .chars()
                .next_back()
                .is_some_and(is_find_word_char)
                && !text[end..].chars().next().is_some_and(is_find_word_char));
        if boundary_ok {
            ranges.push(start..end);
        }
        offset = end.max(start + 1);
    }
    ranges
}

fn is_find_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Collect exact label/value occurrences from a parsed tag in render order.
pub(in crate::app) fn collect_find_occurrences(
    tag: &TagFile,
    tag_key: &str,
    names: &TagNameIndex,
    query: &str,
    look_in: FindLookIn,
    match_case: bool,
    whole_word: bool,
) -> Vec<FindOccurrence> {
    let mut out = Vec::new();
    collect_find_struct(
        tag.root(),
        tag_key,
        names,
        query,
        look_in,
        match_case,
        whole_word,
        true,
        "",
        &mut out,
    );
    out
}

#[allow(clippy::too_many_arguments)]
fn collect_find_struct(
    tag_struct: TagStruct<'_>,
    tag_key: &str,
    names: &TagNameIndex,
    query: &str,
    look_in: FindLookIn,
    match_case: bool,
    whole_word: bool,
    following_inherited_chain: bool,
    prefix: &str,
    out: &mut Vec<FindOccurrence>,
) {
    for field in tag_struct.fields() {
        let label = clean_field_name(field.name());
        let inherited_wrapper = following_inherited_chain
            && field.as_struct().is_some()
            && is_inherited_parent_name(field.name());
        let path = if inherited_wrapper {
            append_field_path(prefix, field.clean_name().as_ref())
        } else {
            append_field_path_for(prefix, &field)
        };
        // TODO(find-phantom-results): these schema entries can inflate the counter
        // with matches that have no corresponding rendered widget. Audit inherited
        // parent wrapper labels skipped by Foundation, advanced/internal fields
        // hidden by the editor, and inline function structures rendered as one
        // consolidated row.
        if look_in.includes_labels() {
            append_find_occurrences(
                out,
                tag_key,
                &path,
                FindTargetKind::Label,
                &label,
                query,
                match_case,
                whole_word,
            );
        }
        if let Some(block) = field.as_block() {
            for index in 0..block.len() {
                if let Some(child) = block.element(index) {
                    collect_find_struct(
                        child,
                        tag_key,
                        names,
                        query,
                        look_in,
                        match_case,
                        whole_word,
                        false,
                        &format!("{path}[{index}]"),
                        out,
                    );
                }
            }
        } else if let Some(array) = field.as_array() {
            for index in 0..array.len() {
                if let Some(child) = array.element(index) {
                    collect_find_struct(
                        child,
                        tag_key,
                        names,
                        query,
                        look_in,
                        match_case,
                        whole_word,
                        false,
                        &format!("{path}[{index}]"),
                        out,
                    );
                }
            }
        } else if let Some(child) = field.as_struct() {
            collect_find_struct(
                child,
                tag_key,
                names,
                query,
                look_in,
                match_case,
                whole_word,
                inherited_wrapper,
                &path,
                out,
            );
        } else if look_in.includes_values() {
            if let Some(value) = field.value() {
                let text = format_foundation_scalar_value(names, &value);
                append_find_occurrences(
                    out,
                    tag_key,
                    &path,
                    FindTargetKind::Value,
                    &text,
                    query,
                    match_case,
                    whole_word,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn append_find_occurrences(
    out: &mut Vec<FindOccurrence>,
    tag_key: &str,
    field_path: &str,
    kind: FindTargetKind,
    text: &str,
    query: &str,
    match_case: bool,
    whole_word: bool,
) {
    out.extend(
        find_text_ranges(text, query, match_case, whole_word)
            .into_iter()
            .map(|range| FindOccurrence {
                tag_key: tag_key.to_owned(),
                field_path: field_path.to_owned(),
                kind,
                text: text.to_owned(),
                range,
            }),
    );
}

impl Baboon {
    /// Refresh synchronous Current/Open Tag results and publish render highlights.
    pub(super) fn refresh_find(&mut self, ctx: &egui::Context) {
        if !self.find.open || self.find.query.is_empty() {
            self.find.occurrences.clear();
            self.find.active = None;
            ctx.data_mut(|data| data.remove::<FindRenderSnapshot>(find_render_snapshot_id()));
            return;
        }
        let old_active = self.find.active_occurrence().cloned();
        if self.find.within == FindWithin::AllTags {
            self.refresh_all_tag_find(ctx);
            self.finish_find_refresh(ctx, old_active);
            return;
        }
        let keys = match self.find.within {
            FindWithin::CurrentTag => self.selected_key.iter().cloned().collect::<Vec<_>>(),
            FindWithin::OpenTags => self.open_tabs.clone(),
            FindWithin::AllTags => unreachable!(),
        };
        let mut occurrences = Vec::new();
        for key in keys {
            let Some(entry) = self.entry_for_key(&key) else {
                continue;
            };
            if !supports_field_search(&entry) {
                continue;
            }
            let Some(doc) = self.parsed_tags.get(&key) else {
                continue;
            };
            occurrences.extend(collect_find_occurrences(
                &doc.tag,
                &key,
                &self.names,
                &self.find.query,
                self.find.look_in,
                self.find.match_case,
                self.find.whole_word,
            ));
        }
        self.find.occurrences = occurrences;
        self.finish_find_refresh(ctx, old_active);
    }

    fn finish_find_refresh(&mut self, ctx: &egui::Context, old_active: Option<FindOccurrence>) {
        self.find.active = old_active
            .and_then(|active| self.find.occurrences.iter().position(|hit| *hit == active))
            .or_else(|| (!self.find.occurrences.is_empty()).then_some(0));
        ctx.data_mut(|data| {
            let matching_cells = self
                .find
                .occurrences
                .iter()
                .filter(|hit| self.parsed_tags.contains_key(&hit.tag_key))
                .map(|hit| (hit.tag_key.clone(), hit.field_path.clone(), hit.kind))
                .collect();
            data.insert_temp(
                find_render_snapshot_id(),
                FindRenderSnapshot {
                    query: self.find.query.clone(),
                    match_case: self.find.match_case,
                    whole_word: self.find.whole_word,
                    active: self.find.active_occurrence().cloned(),
                    matching_cells,
                },
            )
        });
    }

    fn refresh_all_tag_find(&mut self, ctx: &egui::Context) {
        let needs_full_scan = self.source.as_ref().is_some_and(|source| {
            matches!(source.source, TagSource::LooseFolder { .. }) && source.all_entries.is_empty()
        });
        if needs_full_scan {
            if !self.scanning_entries {
                self.begin_scan_all_entries_with_label(ctx.clone(), "Indexing tags for Find...");
            }
            self.find.searching = true;
            self.find.progress = self
                .entry_index_progress
                .as_ref()
                .map(|progress| (progress.processed, progress.total));
            self.find.occurrences.clear();
            return;
        }
        let Some(source) = self.source.as_ref() else {
            self.find.occurrences.clear();
            return;
        };
        let entries = if source.all_entries.is_empty() {
            source.entries.clone()
        } else {
            source.all_entries.clone()
        };
        let mut open_keys = self.parsed_tags.keys().cloned().collect::<Vec<_>>();
        open_keys.sort();
        let signature = format!(
            "{}|{:?}|{}|{}|{}|{}|{}|{}",
            self.source_generation,
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
        if self.find.all_signature.as_deref() != Some(signature.as_str()) {
            let closed_entries = entries
                .iter()
                .filter(|entry| !self.parsed_tags.contains_key(&entry.key))
                .cloned()
                .collect::<Vec<_>>();
            self.find.all_signature = Some(signature);
            self.find.all_order = entries.iter().map(|entry| entry.key.clone()).collect();
            self.begin_all_tag_find(ctx.clone(), closed_entries);
        }

        let mut by_key: HashMap<String, Vec<FindOccurrence>> = HashMap::new();
        for hit in &self.find.all_closed_occurrences {
            if !self.parsed_tags.contains_key(&hit.tag_key) {
                by_key
                    .entry(hit.tag_key.clone())
                    .or_default()
                    .push(hit.clone());
            }
        }
        for key in open_keys {
            let Some(entry) = self.entry_for_key(&key) else {
                continue;
            };
            if !supports_field_search(entry) {
                continue;
            }
            let Some(doc) = self.parsed_tags.get(&key) else {
                continue;
            };
            by_key.insert(
                key.clone(),
                collect_find_occurrences(
                    &doc.tag,
                    &key,
                    &self.names,
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
        let Some(source) = self.source.as_ref() else {
            return;
        };
        self.find.all_request_id = self.find.all_request_id.wrapping_add(1);
        let request_id = self.find.all_request_id;
        let generation = self.source_generation;
        let tag_source = source.source.clone();
        let names = self.names.clone();
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
        thread::spawn(move || {
            let mut occurrences = Vec::new();
            let mut unreadable = 0;
            for (index, entry) in entries.into_iter().enumerate() {
                if supports_field_search(&entry) {
                    match crate::source::read_entry(&tag_source, &entry) {
                        Ok(tag) => occurrences.extend(collect_find_occurrences(
                            &tag, &entry.key, &names, &query, look_in, match_case, whole_word,
                        )),
                        Err(_) => unreadable += 1,
                    }
                }
                let processed = index + 1;
                if processed == total || processed % 32 == 0 {
                    let _ = tx.send(WorkerMessage::FindAllProgress {
                        generation,
                        request_id,
                        processed,
                        total,
                    });
                    ctx.request_repaint();
                }
            }
            let _ = tx.send(WorkerMessage::FindAllFinished {
                generation,
                request_id,
                occurrences,
                unreadable,
            });
            ctx.request_repaint();
        });
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
        if self.selected_key.as_deref() != Some(hit.tag_key.as_str()) {
            self.select_entry(hit.tag_key.clone(), ctx.clone());
        }
        if !self.parsed_tags.contains_key(&hit.tag_key) {
            self.pending_find_jump = Some(hit);
            return;
        }
        if let Some(entry) = self.entry_for_key(&hit.tag_key) {
            if is_model_group(entry.group_tag, &self.names) {
                self.model_previews
                    .entry(hit.tag_key.clone())
                    .or_default()
                    .active_tab = ModelTagPanelTab::Fields;
            }
        }
        self.pending_find_jump = None;
        self.navigate_to_field(ctx, &hit.tag_key, &hit.field_path);
        if hit.kind == FindTargetKind::Label {
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
mod tests {
    use super::*;

    fn tag_with_one_ai_properties_element() -> TagFile {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/object.json")).unwrap();
        let field_index = tag
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| clean_field_name(field.name()) == "ai properties")
            .expect("object schema has ai properties")
            .0;
        tag.root_mut()
            .field_at_mut(field_index)
            .unwrap()
            .as_block_mut()
            .unwrap()
            .add_element();
        tag
    }

    fn ai_type_name_render_path(tag: &TagFile) -> String {
        let root = tag.root();
        let ai_properties = root
            .fields()
            .find(|field| clean_field_name(field.name()) == "ai properties")
            .expect("object schema has ai properties");
        let block_path = append_field_path_for("", &ai_properties);
        let element = ai_properties
            .as_block()
            .unwrap()
            .element(0)
            .expect("test block has one element");
        let ai_type_name = element
            .fields()
            .find(|field| clean_field_name(field.name()) == "ai type name")
            .expect("ai properties has ai type name");
        append_field_path_for(&format!("{block_path}[0]"), &ai_type_name)
    }

    fn biped_with_one_inherited_ai_properties_element() -> TagFile {
        let mut tag = TagFile::new(test_definition_path("halo3_mcc/biped.json")).unwrap();
        tag.root_mut()
            .field_path_mut("unit/object/ai properties")
            .expect("biped inherits the object ai properties block")
            .as_block_mut()
            .unwrap()
            .add_element();
        tag
    }

    fn inherited_ai_type_name_render_path(tag: &TagFile) -> String {
        let unit = tag
            .root()
            .fields()
            .find(|field| is_inherited_parent_name(field.name()))
            .expect("biped has an inherited unit wrapper");
        let unit_struct = unit.as_struct().unwrap();
        let object = unit_struct
            .fields()
            .find(|field| is_inherited_parent_name(field.name()))
            .expect("unit has an inherited object wrapper");
        let object_struct = object.as_struct().unwrap();
        let ai_properties = object_struct
            .fields()
            .find(|field| clean_field_name(field.name()) == "ai properties")
            .expect("object has ai properties");
        let block_path = append_field_path_for("unit/object", &ai_properties);
        let element = ai_properties
            .as_block()
            .unwrap()
            .element(0)
            .expect("test block has one element");
        let ai_type_name = element
            .fields()
            .find(|field| clean_field_name(field.name()) == "ai type name")
            .expect("ai properties has ai type name");
        append_field_path_for(&format!("{block_path}[0]"), &ai_type_name)
    }

    #[test]
    fn collected_nested_path_matches_renderer_ordinal_path() {
        let tag = tag_with_one_ai_properties_element();
        let expected = ai_type_name_render_path(&tag);
        let occurrences = collect_find_occurrences(
            &tag,
            "test.object",
            &TagNameIndex::default(),
            "ai type name",
            FindLookIn::Labels,
            false,
            false,
        );
        let hit = occurrences
            .iter()
            .find(|hit| hit.text == "ai type name")
            .expect("nested label should be collected");
        assert_eq!(hit.field_path, expected);
    }

    #[test]
    fn collected_match_identity_is_accepted_by_widget_lookup() {
        let tag = tag_with_one_ai_properties_element();
        let rendered_path = ai_type_name_render_path(&tag);
        let occurrences = collect_find_occurrences(
            &tag,
            "test.object",
            &TagNameIndex::default(),
            "ai type name",
            FindLookIn::Labels,
            false,
            false,
        );
        let matching_cells = occurrences
            .iter()
            .map(|hit| (hit.tag_key.clone(), hit.field_path.clone(), hit.kind))
            .collect::<HashSet<_>>();
        assert!(matching_cells.contains(&(
            "test.object".to_owned(),
            rendered_path,
            FindTargetKind::Label,
        )));
    }

    /// Inheritance wrappers are presentation-only path segments: unlike ordinary
    /// fields, `unit/object` intentionally carry no `#ordinal` in Foundation.
    #[test]
    fn collected_biped_inherited_path_matches_plain_wrapper_renderer_path() {
        let tag = biped_with_one_inherited_ai_properties_element();
        let expected = inherited_ai_type_name_render_path(&tag);
        let occurrences = collect_find_occurrences(
            &tag,
            "test.biped",
            &TagNameIndex::default(),
            "ai type name",
            FindLookIn::Labels,
            false,
            false,
        );
        let hit = occurrences
            .iter()
            .find(|hit| hit.text == "ai type name")
            .expect("inherited nested label should be collected");
        assert_eq!(hit.field_path, expected);
        assert!(expected.starts_with("unit/object/ai properties#"));
    }

    #[test]
    fn ranges_honor_case_and_word_boundaries() {
        assert_eq!(
            find_text_ranges("Brute brute", "brute", false, false),
            vec![0..5, 6..11]
        );
        assert_eq!(
            find_text_ranges("Brute brute", "brute", true, false),
            vec![6..11]
        );
        assert_eq!(
            find_text_ranges("brute brute_captain", "brute", false, true),
            vec![0..5]
        );
    }

    #[test]
    fn ranges_are_non_overlapping_and_empty_query_is_safe() {
        assert_eq!(
            find_text_ranges("aaaa", "aa", true, false),
            vec![0..2, 2..4]
        );
        assert!(find_text_ranges("abc", "", false, false).is_empty());
    }

    #[test]
    fn appending_keeps_label_then_value_occurrence_order() {
        let mut out = Vec::new();
        append_find_occurrences(
            &mut out,
            "tag",
            "field",
            FindTargetKind::Label,
            "needle label",
            "needle",
            false,
            false,
        );
        append_find_occurrences(
            &mut out,
            "tag",
            "field",
            FindTargetKind::Value,
            "needle value",
            "needle",
            false,
            false,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, FindTargetKind::Label);
        assert_eq!(out[1].kind, FindTargetKind::Value);
    }

    #[test]
    fn all_tag_merge_uses_source_order() {
        let occurrence = |key: &str| FindOccurrence {
            tag_key: key.to_owned(),
            field_path: "field".to_owned(),
            kind: FindTargetKind::Value,
            text: "hit".to_owned(),
            range: 0..3,
        };
        let by_key = HashMap::from([
            ("b".to_owned(), vec![occurrence("b")]),
            ("a".to_owned(), vec![occurrence("a")]),
        ]);
        let ordered = order_find_occurrences(&["a".to_owned(), "b".to_owned()], by_key);
        assert_eq!(ordered[0].tag_key, "a");
        assert_eq!(ordered[1].tag_key, "b");
    }
}
