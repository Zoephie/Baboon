//! Field-value indexing, searching, and TSV header mapping.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::FieldValueSearchFinished`, rejecting stale source generations.
    pub(super) fn handle_field_value_search_finished(
        &mut self,
        generation: u64,
        query: String,
        result: Result<Vec<FieldValueMatch>, String>,
    ) -> bool {
        self.field_value_searching = false;
        if generation != self.source_generation {
            return true;
        }
        match result {
            Ok(matches) => {
                let entries: Vec<TagEntry> = matches.iter().map(|m| m.entry.clone()).collect();
                let annotations: Vec<String> = matches.iter().map(|m| m.label.clone()).collect();
                let note = entries
                    .is_empty()
                    .then(|| format!("No tag field values contain \"{query}\"."));
                self.status = format!("Field search for \"{query}\": {} match(es)", entries.len());
                self.query_results = Some(TagQueryResults {
                    title: format!("Field value '{query}' ({})", entries.len()),
                    entries,
                    annotations,
                    note,
                    ref_target: None,
                });
            }
            Err(error) => self.status = format!("Field search failed: {error}"),
        }
        false
    }

    /// Applies `WorkerMessage::FieldIndexBuilt` when its source generation is current.
    pub(super) fn handle_field_index_built(
        &mut self,
        generation: u64,
        blobs: Vec<(String, String)>,
    ) -> bool {
        if generation == self.source_generation {
            self.field_index.install(generation, blobs);
        }
        false
    }
}

pub(super) fn run_field_value_search(
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
        let Ok(tag) = crate::source::read_entry(source, entry) else {
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

pub(super) fn map_tsv_header_to_fields(
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

pub(super) fn build_field_value_index(
    source: &TagSource,
    entries: &[TagEntry],
) -> Vec<(String, String)> {
    let mut blobs = Vec::new();
    for entry in entries {
        let Ok(tag) = crate::source::read_entry(source, entry) else {
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

pub(super) fn collect_searchable_text(element: &TagStruct, blob: &mut String, depth: usize) {
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

pub(super) fn append_searchable_text(blob: &mut String, text: &str) {
    if !blob.is_empty() {
        blob.push_str(" · ");
    }
    blob.push_str(&text.to_ascii_lowercase());
}

pub(super) fn first_field_value_match(
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

pub(super) fn field_searchable_text(value: Option<TagFieldData>) -> Option<String> {
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
