//! Field-value indexing, searching, and TSV header mapping.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::FieldValueSearchFinished`, rejecting stale source generations.
    pub(in crate::app) fn handle_field_value_search_finished(
        &mut self,
        stamp: KitStamp,
        query: String,
        result: Result<Vec<FieldValueMatch>, String>,
    ) -> bool {
        self.search.field_value_searching = false;
        // The results name tags in the kit the search ran in, which may not be
        // the one focused by the time they arrive.
        if self.model.resolve_stamp(stamp).is_none() {
            return true;
        }
        match result {
            Ok(matches) => {
                let entries: Vec<TagEntry> = matches.iter().map(|m| m.entry.clone()).collect();
                let annotations: Vec<String> = matches.iter().map(|m| m.label.clone()).collect();
                let note = entries
                    .is_empty()
                    .then(|| format!("No tag field values contain \"{query}\"."));
                self.model.status = format!("Field search for \"{query}\": {} match(es)", entries.len());
                self.search.query_results = Some(TagQueryResults {
                    kit: stamp.kit,
                    title: format!("Field value '{query}' ({})", entries.len()),
                    entries,
                    annotations,
                    note,
                    ref_target: None,
                });
            }
            Err(error) => self.model.status = format!("Field search failed: {error}"),
        }
        false
    }

    /// Applies `WorkerMessage::FieldIndexBuilt` when its source generation is current.
    pub(in crate::app) fn handle_field_index_built(
        &mut self,
        stamp: KitStamp,
        blobs: Result<Vec<(String, String)>, String>,
    ) -> bool {
        if let Some(kit_index) = self.model.resolve_stamp(stamp) {
            match blobs {
                Ok(blobs) => self.model.kits[kit_index]
                    .field_index
                    .install(stamp.generation, blobs),
                Err(error) => {
                    // Not building and not ready, so the next search tries
                    // again rather than waiting on a build that ended.
                    self.model.kits[kit_index].field_index.invalidate();
                    self.model.status = error;
                }
            }
        }
        false
    }
}

pub(in crate::app) fn run_field_value_search(
    source: &TagSource,
    entries: &[TagEntry],
    query_lower: &str,
) -> Result<Vec<FieldValueMatch>, String> {
    const MATCH_CAP: usize = 1000;
    let mut matches = Vec::new();
    for entry in entries {
        if matches.len() >= MATCH_CAP {
            break;
        }
        let Ok(tag) = crate::core::source::read_entry(source, entry) else {
            continue;
        };
        if let Some((field_path, value)) = first_field_value_match(&tag.root(), query_lower, "") {
            matches.push(FieldValueMatch {
                entry: entry.clone(),
                label: format!("{field_path} = {}", truncate_field_value(&value)),
            });
        }
    }
    Ok(matches)
}

pub(in crate::app) fn map_tsv_header_to_fields(
    header_line: &str,
    columns: &[(String, String)],
) -> Vec<Option<String>> {
    header_line
        .split('\t')
        .map(|raw| {
            let clean = raw.trim();
            columns
                .iter()
                .find(|(col_clean, _)| col_clean.eq_ignore_ascii_case(clean))
                .map(|(_, full)| full.clone())
        })
        .collect()
}

pub(in crate::app) fn build_field_value_index(
    source: &TagSource,
    entries: &[TagEntry],
) -> Vec<(String, String)> {
    let mut blobs = Vec::new();
    for entry in entries {
        let Ok(tag) = crate::core::source::read_entry(source, entry) else {
            continue;
        };
        let mut blob = String::new();
        collect_searchable_text(&tag.root(), &mut blob, 0);
        if !blob.is_empty() {
            blobs.push((entry.key.clone(), blob));
        }
    }
    blobs
}

pub(in crate::app) fn collect_searchable_text(element: &TagStruct, blob: &mut String, depth: usize) {
    const CAP: usize = 4000;
    if blob.len() >= CAP || depth > 32 {
        return;
    }
    for field in element.fields() {
        if blob.len() >= CAP {
            return;
        }
        if let Some(block) = field.as_block() {
            for index in 0..block.len() {
                let Some(child) = block.element(index) else {
                    continue;
                };
                collect_searchable_text(&child, blob, depth + 1);
                if blob.len() >= CAP {
                    return;
                }
            }
            continue;
        }
        if let Some(nested) = field.as_struct() {
            collect_searchable_text(&nested, blob, depth + 1);
            continue;
        }
        let Some(text) = field_searchable_text(field.value()) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        append_searchable_text(blob, &text);
    }
}

pub(in crate::app) fn append_searchable_text(blob: &mut String, text: &str) {
    if !blob.is_empty() {
        blob.push_str(" · ");
    }
    blob.push_str(&text.to_ascii_lowercase());
}

pub(in crate::app) fn first_field_value_match(
    element: &TagStruct,
    query_lower: &str,
    path: &str,
) -> Option<(String, String)> {
    for field in element.fields() {
        let clean = clean_field_name(field.name());
        let field_path = if path.is_empty() {
            clean.clone()
        } else {
            format!("{path}/{clean}")
        };
        if let Some(block) = field.as_block() {
            for index in 0..block.len() {
                if let Some(child) = block.element(index) {
                    if let Some(hit) = first_field_value_match(
                        &child,
                        query_lower,
                        &format!("{field_path}[{index}]"),
                    ) {
                        return Some(hit);
                    }
                }
            }
            continue;
        }
        if let Some(nested) = field.as_struct() {
            if let Some(hit) = first_field_value_match(&nested, query_lower, &field_path) {
                return Some(hit);
            }
            continue;
        }
        let Some(text) = field_searchable_text(field.value()) else {
            continue;
        };
        if text.is_empty() || !text.to_ascii_lowercase().contains(query_lower) {
            continue;
        }
        return Some((field_path, text));
    }
    None
}

pub(in crate::app) fn field_searchable_text(value: Option<TagFieldData>) -> Option<String> {
    match value? {
        TagFieldData::String(s) | TagFieldData::LongString(s) => Some(s),
        TagFieldData::StringId(d) | TagFieldData::OldStringId(d) => Some(d.string),
        TagFieldData::TagReference(r) => r.group_tag_and_name.map(|(_, path)| path),
        TagFieldData::CharEnum { name, .. }
        | TagFieldData::ShortEnum { name, .. }
        | TagFieldData::LongEnum { name, .. } => name,
        _ => None,
    }
}

fn truncate_field_value(value: &str) -> String {
    const MAX: usize = 80;
    if value.chars().count() > MAX {
        let head: String = value.chars().take(MAX).collect();
        format!("{head}…")
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod field_search_kit_tests;

impl Baboon {
    /// Run a field-value search for the current query. If the in-memory index is
    /// ready it answers instantly from cache; otherwise it kicks off a live
    /// background scan (correct, slower) and builds the index for next time.
    /// Starts source-scoped indexing or search work without blocking the UI thread.
    /// Generation-tagged completion is ignored if the active source changes first.
    pub(in crate::app) fn begin_field_value_search(&mut self, ctx: egui::Context) {
        let display = self.search.field_value_query.trim().to_owned();
        if display.is_empty() {
            return;
        }
        let query_lower = display.to_ascii_lowercase();
        let group_filter = self.search.field_value_group.trim().to_ascii_lowercase();
        let stamp = self.model.kit_stamp();

        // Fast path: answer from the cached index.
        if self.model.kits[self.model.active]
            .field_index
            .is_ready_for(stamp.generation)
        {
            // Over-fetch when group-filtering so the cap applies post-filter.
            let raw_cap = if group_filter.is_empty() { 1000 } else { 8000 };
            let hits = self.model.kits[self.model.active]
                .field_index
                .query(&query_lower, raw_cap);
            let mut entries = Vec::new();
            let mut annotations = Vec::new();
            for (key, snippet) in hits {
                if let Some(entry) = self.model.entry_for_key(&key).cloned() {
                    if !group_filter.is_empty()
                        && !self.model.group_label_matches(entry.group_tag, &group_filter)
                    {
                        continue;
                    }
                    entries.push(entry);
                    annotations.push(snippet);
                    if entries.len() >= 1000 {
                        break;
                    }
                }
            }
            let note = entries
                .is_empty()
                .then(|| format!("No tag field values contain \"{display}\"."));
            self.model.status = format!(
                "Field search for \"{display}\": {} match(es) (indexed)",
                entries.len()
            );
            self.search.query_results = Some(TagQueryResults {
                kit: self.model.active_kit_id(),
                title: format!("Field value '{display}' ({})", entries.len()),
                entries,
                annotations,
                note,
                ref_target: None,
            });
            return;
        }

        if self.model.source().is_none() {
            return;
        }
        let base_entries: Vec<TagEntry> = {
            let source = self.model.source().expect("checked");
            if source.all_entries.is_empty() {
                source.entries.clone()
            } else {
                source.all_entries.clone()
            }
        };
        let entries: Vec<TagEntry> = if group_filter.is_empty() {
            base_entries
        } else {
            base_entries
                .into_iter()
                .filter(|entry| self.model.group_label_matches(entry.group_tag, &group_filter))
                .collect()
        };
        let tag_source = self.model.source().expect("checked").source.clone();
        self.search.field_value_searching = true;
        self.model.status = format!("Searching field values for \"{display}\"…");
        let panic_query = display.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || WorkerMessage::FieldValueSearchFinished {
                stamp,
                query: display,
                result: run_field_value_search(&tag_source, &entries, &query_lower),
            },
            move |error| WorkerMessage::FieldValueSearchFinished {
                stamp,
                query: panic_query,
                result: Err(error),
            },
        );
        // Build the index in the background so the next search is instant.
        self.begin_build_field_index(ctx);
    }



    /// Build the in-memory searchable-text index in the background (idempotent —
    /// skips if already ready for this generation or already building).
    /// Starts source-scoped indexing or search work without blocking the UI thread.
    /// Generation-tagged completion is ignored if the active source changes first.
    pub(in crate::app) fn begin_build_field_index(&mut self, ctx: egui::Context) {
        let stamp = self.model.kit_stamp();
        if self.model.kits[self.model.active]
            .field_index
            .is_ready_for(stamp.generation)
            || self.model.kits[self.model.active].field_index.is_building()
        {
            return;
        }
        let Some(source) = self.model.source() else {
            return;
        };
        let entries: Vec<TagEntry> = if source.all_entries.is_empty() {
            source.entries.clone()
        } else {
            source.all_entries.clone()
        };
        let tag_source = source.source.clone();
        self.model.kits[self.model.active].field_index.mark_building();
        // A build that panicked used to leave the index building forever, and
        // a building index is never started again.
        spawn_worker(
            &self.tx,
            &ctx,
            move || WorkerMessage::FieldIndexBuilt {
                stamp,
                blobs: Ok(build_field_value_index(&tag_source, &entries)),
            },
            move |error| WorkerMessage::FieldIndexBuilt {
                stamp,
                blobs: Err(format!("Building the field search index failed: {error}")),
            },
        );
    }

    pub(in crate::app) fn show_tags_with_keyword(&mut self, keyword: &str) {
        let keys = self.model.kits[self.model.active].keywords.tags_with(keyword);
        let entries: Vec<TagEntry> = keys
            .iter()
            .filter_map(|key| self.model.entry_for_key(key).cloned())
            .collect();
        let note = entries
            .is_empty()
            .then(|| "No tags with this keyword are in the current source.".to_owned());
        self.search.query_results = Some(TagQueryResults {
            kit: self.model.active_kit_id(),
            title: format!("Tags tagged '{keyword}' ({})", entries.len()),
            entries,
            annotations: Vec::new(),
            note,
            ref_target: None,
        });
    }
}

#[cfg(test)]
mod field_search_tests;

impl Model {
    /// Whether a group matches a (lowercased) group filter — by four-CC or by a
    /// substring of the group's name/extension (e.g. "weap" or "weapon").
    pub(in crate::app) fn group_label_matches(&self, group_tag: u32, filter_lower: &str) -> bool {
        if format_group_tag(group_tag).to_ascii_lowercase() == filter_lower {
            return true;
        }
        self.names()
            .name_for(group_tag)
            .or_else(|| group_tag_to_extension(group_tag))
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains(filter_lower)
    }
}
