//! Whole-row clipboard data, spreadsheet conversion and destination planning.
//! Clipboard text is untrusted; only a locally held token can authorize Cut.
use super::*;
use blam_tags::string_list::{self as strings, StringEntry, StringList, StringListEdit, Storage, TextSyntax};
use serde::{Serialize, Deserialize};
use std::collections::BTreeSet;

#[path = "string_row_clipboard_app.rs"]
mod application;
pub(in crate::app) use application::RowCommand;

const CLIPBOARD_ID: &str = "string row clipboard";
const ROW_MODE_ID: &str = "string row clipboard mode";
const DISMISSED_ID: &str = "dismissed string row clipboard";
const CURRENT_ID: &str = "current string row clipboard";
const CONSUMED_ID: &str = "consumed string row clipboard";
const MARKER: &str = "<!--BABOON_STRING_ROWS_V1:";
const MAX_CLIPBOARD: usize = 16 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub(in crate::app) struct RowData {
    pub token: String,
    pub classic: bool,
    #[serde(default)]
    pub hud: bool,
    pub languages: Vec<String>,
    pub language: usize,
    pub rows: Vec<Row>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub(in crate::app) struct Row {
    pub id: Option<String>,
    pub texts: Vec<Option<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct RowDocument {
    pub kit: KitId,
    pub key: String,
    pub id: u64,
}

#[derive(Clone)]
pub(in crate::app) struct ActiveRows {
    pub cutting: bool,
    pub source: RowDocument,
    pub revision: u64,
    pub indices: BTreeSet<usize>,
    pub token: String,
    pub data: RowData,
}

impl ActiveRows {
    pub(in crate::app) fn border_color(&self) -> Color32 {
        if self.cutting { Color32::from_rgb(255, 209, 56) } else { Color32::from_rgb(71, 174, 235) }
    }
}

pub(in crate::app) fn set_active(ctx: &egui::Context, active: ActiveRows) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique(ROW_MODE_ID), Arc::new(active)));
    ctx.request_repaint();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum PasteMode { Overwrite, Above, Below }

#[derive(Clone)]
pub(in crate::app) enum RowAction {
    Copy { cut: bool },
    Paste { mode: PasteMode, data: RowData },
}

#[derive(Clone)]
pub(in crate::app) struct RowRequest {
    pub action: RowAction,
    pub selected: BTreeSet<usize>,
    pub language: usize,
    pub scope: String,
}

#[derive(Clone)]
pub(in crate::app) struct LinkedMove {
    pub documents: [RowDocument; 2],
    pub steps: [u64; 2],
    pub redo: bool,
}

pub(in crate::app) fn active(ctx: &egui::Context) -> Option<Arc<ActiveRows>> {
    ctx.data(|data| data.get_temp(egui::Id::unique(ROW_MODE_ID)))
}

pub(in crate::app) fn cut(ctx: &egui::Context) -> Option<Arc<ActiveRows>> {
    active(ctx).filter(|active| active.cutting)
}

pub(in crate::app) fn cancel_cut(ctx: &egui::Context) {
    if let Some(active) = cut(ctx) {
        let mut active = (*active).clone();
        active.cutting = false;
        set_active(ctx, active);
    }
    ctx.request_repaint();
}

fn cancel_mode(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<Arc<ActiveRows>>(egui::Id::unique(ROW_MODE_ID)));
    ctx.request_repaint();
}

/// A completed move consumes the cut snapshot, even if the OS clipboard is busy.
/// Only clear the OS clipboard while it still holds that snapshot.
fn consume_cut(ctx: &egui::Context, rows: &RowData) -> Result<(), String> {
    let current = read(ctx).filter(|(text, html)| html.as_deref().and_then(decode)
        .is_some_and(|(data, visible)| data == *rows && visible == *text));
    cancel_mode(ctx);
    ctx.data_mut(|memory| {
        memory.remove::<(String, String)>(egui::Id::unique(CLIPBOARD_ID));
        if let Some((text, html)) = &current {
            memory.insert_temp(egui::Id::unique(CONSUMED_ID), fingerprint(text, html.as_deref()));
        }
    });
    #[cfg(not(test))]
    if current.is_some() {
        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.clear())
            .map_err(|error| format!("The rows were moved, but the system clipboard could not be cleared: {error}"))?;
    }
    Ok(())
}

fn fingerprint(text: &str, html: Option<&str>) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(text.len().to_le_bytes());
    hash.update(text.as_bytes());
    hash.update(html.unwrap_or_default().as_bytes());
    hash.finalize().into()
}

/// Escape ends the row-copy mode without destroying the system clipboard.
/// Remember its identity so opening Insert does not reactivate the same copy.
pub(in crate::app) fn dismiss(ctx: &egui::Context) {
    let identity = read(ctx).map(|(text, html)| fingerprint(&text, html.as_deref()))
        .or_else(|| ctx.data(|data| data.get_temp::<[u8; 32]>(egui::Id::unique(CURRENT_ID))));
    if let Some(identity) = identity { ctx.data_mut(|data| data.insert_temp(egui::Id::unique(DISMISSED_ID), identity)); }
    cancel_mode(ctx);
}

pub(in crate::app) fn insert_active(ctx: &egui::Context) -> bool {
    ctx.data(|data| {
        let dismissed = data.get_temp::<[u8; 32]>(egui::Id::unique(DISMISSED_ID));
        dismissed.is_none() || dismissed != data.get_temp::<[u8; 32]>(egui::Id::unique(CURRENT_ID))
    })
}

pub(in crate::app) fn refresh_cut(model: &Model, ctx: &egui::Context) -> bool {
    if let Some(active) = active(ctx) {
        let valid = model.kit_index(active.source.kit)
            .and_then(|index| model.kits[index].parsed_tags.get(&active.source.key))
            .is_some_and(|doc| doc.id == active.source.id && doc.dirty.revision() == active.revision);
        if !valid && active.cutting { cancel_cut(ctx); return true; }
        let now = ctx.input(|input| input.time);
        let check_id = egui::Id::unique("string cut clipboard check");
        let next = ctx.data(|data| data.get_temp::<f64>(check_id)).unwrap_or_default();
        if now >= next {
            ctx.data_mut(|data| data.insert_temp(check_id, now + 0.25));
            if let Some((text, html)) = read(ctx) {
                let same = html.as_deref().and_then(decode).is_some_and(|(data, visible)| visible == text && data == active.data);
                if !same { cancel_mode(ctx); }
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    false
}

fn field(text: &str) -> String {
    if text.contains(['\t', '\r', '\n', '"']) { format!("\"{}\"", text.replace('"', "\"\"")) }
    else { text.to_owned() }
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub(in crate::app) fn encode(data: &RowData, syntax: &TextSyntax) -> Result<(String, String), String> {
    let mut table = "<table>".to_owned();
    let mut text = String::new();
    for row in &data.rows {
        let mut cells = Vec::new();
        if !data.classic { cells.push(row.id.clone().unwrap_or_default()); }
        // Spreadsheet formats mirror the visible columns. The hidden payload
        // below retains every language for whole-row pastes within Baboon.
        cells.push(strings::format_shown(row.texts.get(data.language).and_then(Option::as_deref).unwrap_or_default(), syntax));
        text.push_str(&cells.iter().map(|cell| field(cell)).collect::<Vec<_>>().join("\t"));
        text.push_str("\r\n");
        table.push_str("<tr>");
        for cell in cells { table.push_str(&format!("<td>{}</td>", html_escape(&cell))); }
        table.push_str("</tr>");
    }
    table.push_str("</table>");
    let bytes = serde_json::to_vec(&(data, &text)).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_CLIPBOARD { return Err("The copied rows exceed the clipboard size limit.".to_owned()); }
    let mut html = MARKER.to_owned();
    use std::fmt::Write;
    for byte in bytes { write!(html, "{byte:02x}").unwrap(); }
    html.push_str("-->");
    html.push_str(&table);
    Ok((text, html))
}

fn decode(html: &str) -> Option<(RowData, String)> {
    let encoded = html.split_once(MARKER)?.1.split_once("-->")?.0;
    if encoded.len() > MAX_CLIPBOARD * 2 || !encoded.len().is_multiple_of(2) { return None; }
    let bytes = encoded.as_bytes().chunks_exact(2).map(|pair| {
        let hex = std::str::from_utf8(pair).ok()?;
        u8::from_str_radix(hex, 16).ok()
    }).collect::<Option<Vec<_>>>()?;
    let (data, text): (RowData, String) = serde_json::from_slice(&bytes).ok()?;
    (!data.rows.is_empty() && !data.languages.is_empty() && data.languages.len() <= 64
        && data.language < data.languages.len()
        && (!data.classic || data.languages.len() == 1)
        && data.rows.iter().all(|row| row.texts.len() == data.languages.len()
            && row.id.as_deref().is_none_or(|id| id.trim() == id && !id.contains(['\r', '\n'])))).then_some((data, text))
}

pub(in crate::app) fn write(ctx: &egui::Context, data: &RowData, syntax: &TextSyntax) -> Result<(), String> {
    let (text, html) = encode(data, syntax)?;
    #[cfg(not(test))]
    arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_html(html.clone(), Some(text.clone())))
        .map_err(|error| format!("Could not write the clipboard: {error}"))?;
    ctx.data_mut(|memory| memory.insert_temp(egui::Id::unique(CLIPBOARD_ID), (text.clone(), html)));
    ctx.data_mut(|memory| memory.remove::<[u8; 32]>(egui::Id::unique(DISMISSED_ID)));
    #[cfg(test)]
    ctx.copy_text(text);
    Ok(())
}

fn read(_ctx: &egui::Context) -> Option<(String, Option<String>)> {
    #[cfg(test)]
    {
        if let Some(text) = _ctx.data(|data| data.get_temp::<String>(egui::Id::unique("test string row clipboard"))) {
            return Some((text, None));
        }
        _ctx.data(|data| data.get_temp::<(String, String)>(egui::Id::unique(CLIPBOARD_ID))).map(|(text, html)| (text, Some(html)))
    }
    #[cfg(not(test))]
    {
        let mut clipboard = arboard::Clipboard::new().ok()?;
        let text = clipboard.get_text().ok()?;
        let html = clipboard.get().html().ok();
        Some((text, html))
    }
}

pub(in crate::app) fn clipboard(ctx: &egui::Context, list: &StringList, language: usize, syntax: &TextSyntax) -> Option<RowData> {
    let (text, html) = read(ctx)?;
    let identity = fingerprint(&text, html.as_deref());
    if ctx.data(|memory| memory.get_temp::<[u8; 32]>(egui::Id::unique(CONSUMED_ID))) == Some(identity) { return None; }
    ctx.data_mut(|memory| memory.insert_temp(egui::Id::unique(CURRENT_ID), identity));
    if text.len() > MAX_CLIPBOARD { return None; }
    if let Some((data, _)) = html.as_deref().and_then(decode).filter(|(_, visible)| *visible == text) {
        // External apps can preserve HTML while changing plain text. Do not use
        // stale hidden rows when their visible counterpart no longer matches.
        if active(ctx).is_some_and(|active| active.data != data) { cancel_mode(ctx); }
        if cut(ctx).is_some_and(|active| active.token == data.token) && (data.classic != (list.storage == Storage::Classic) || data.hud != (list.storage == Storage::HudMessages)) { return None; }
        if !data.classic && !data.hud && list.storage == Storage::Multilingual && data.languages.iter().all(|name| !list.languages.iter().any(|language| language_key(name) == language_key(&language.name))) { return None; }
        return Some(data);
    }
    if active(ctx).is_some() { cancel_mode(ctx); }
    external(&text, list, language, syntax).ok()
}

pub(in crate::app) fn external(text: &str, list: &StringList, language: usize, syntax: &TextSyntax) -> Result<RowData, String> {
    let cells = crate::app::export::string_transfer::delimited_rows(text, '\t')?;
    let expected = if list.storage == Storage::Classic { 1 } else { 2 };
    if cells.is_empty() || cells.iter().any(|row| row.len() != expected) { return Err(format!("Paste requires exactly {expected} column(s).")); }
    let rows = cells.into_iter().map(|cells| {
        let shown = super::string_list::shown_input(cells.last().unwrap(), syntax);
        let text = strings::parse_shown(&shown, syntax).map_err(|_| "The clipboard contains an unsupported keyword or invalid string escape.".to_owned())?;
        let id = (expected == 2).then(|| cells[0].clone());
        if id.as_deref().is_some_and(|id| !id.is_empty() && (id.trim() != id || id.contains(['\r', '\n']))) { return Err("A String ID cannot contain line breaks or surrounding whitespace.".to_owned()); }
        Ok(Row { id, texts: vec![Some(text)] })
    }).collect::<Result<Vec<_>, String>>()?;
    Ok(RowData { hud: list.storage == Storage::HudMessages, token: uuid::Uuid::new_v4().to_string(), classic: expected == 1,
        languages: vec![list.languages[language].name.clone()], language: 0, rows })
}

fn language_key(name: &str) -> String {
    name.to_lowercase().chars().filter(char::is_ascii_alphanumeric).collect()
}

pub(in crate::app) fn language_difference(data: &RowData, list: &StringList) -> (Vec<String>, Vec<String>) {
    if data.classic || data.hud || list.storage != Storage::Multilingual { return (Vec::new(), Vec::new()); }
    let missing = data.languages.iter().filter(|name| !list.languages.iter().any(|language| language_key(&language.name) == language_key(name))).cloned().collect();
    let extra = list.languages.iter().filter(|language| !data.languages.iter().any(|name| language_key(name) == language_key(&language.name))).map(|language| language.name.clone()).collect();
    (missing, extra)
}

fn numbered_id(id: &str, taken: &mut BTreeSet<String>, storage: Storage) -> String {
    let id = if id.is_empty() { "new_string" } else { id };
    if taken.insert(id.to_owned()) { return id.to_owned(); }
    let base = id.rsplit_once(" (").filter(|(_, suffix)| suffix.strip_suffix(')').is_some_and(|number| number.parse::<usize>().is_ok())).map_or(id, |(base, _)| base);
    for number in 1.. {
        let suffix = format!(" ({number})");
        let base = if storage == Storage::HudMessages { base.chars().take(31 - suffix.len()).collect::<String>() } else { base.to_owned() };
        let candidate = format!("{base}{suffix}");
        if taken.insert(candidate.clone()) { return candidate; }
    }
    unreachable!()
}

pub(in crate::app) struct PastePlan {
    pub rows: Vec<StringEntry>,
    pub selected: BTreeSet<usize>,
}

pub(in crate::app) fn plan(list: &StringList, data: &RowData, language: usize, selected: &BTreeSet<usize>, mode: PasteMode, local_cut: &BTreeSet<usize>) -> Result<PastePlan, String> {
    if data.rows.is_empty() { return Err("The clipboard has no string rows.".to_owned()); }
    let start = match mode {
        PasteMode::Below => selected.last().map_or(list.entries.len(), |index| index + 1),
        _ => selected.first().copied().unwrap_or(list.entries.len()),
    }.min(list.entries.len());
    let mut rows: Vec<_> = list.entries.iter().cloned().enumerate().map(|(index, row)| (Some(index), row, false)).collect();
    for (offset, row) in data.rows.iter().enumerate() {
        let at = start + offset;
        let mut entry = if mode == PasteMode::Overwrite { list.entries.get(at).cloned().unwrap_or_else(|| list.new_entry()) } else { list.new_entry() };
        entry.id = list.storage.has_ids().then(|| row.id.clone().unwrap_or_default());
        if list.storage != Storage::Multilingual {
            entry.texts = vec![row.texts.get(if data.classic { 0 } else { data.language }).cloned().flatten()];
        } else if data.classic || data.hud {
            entry.texts[language] = row.texts.first().cloned().flatten();
        } else {
            let mut matched = false;
            for (column, source) in data.languages.iter().enumerate() {
                if let Some(target) = list.languages.iter().position(|language| language_key(&language.name) == language_key(source)) {
                    entry.texts[target] = row.texts.get(column).cloned().flatten();
                    matched = true;
                }
            }
            if !matched { return Err("The clipboard and destination have no matching languages.".to_owned()); }
        }
        if entry.texts.iter().flatten().any(|text| text.encode_utf16().count() > list.max_text_units()) { return Err("A pasted string exceeds this engine's string length limit.".to_owned()); }
        if mode == PasteMode::Overwrite && at < rows.len() { rows[at] = (Some(at), entry, true); }
        else { rows.insert(at, (None, entry, true)); }
    }
    rows.retain(|(origin, _, pasted)| *pasted || !origin.is_some_and(|index| local_cut.contains(&index)));
    let mut taken = rows.iter().filter(|(_, _, pasted)| !pasted).filter_map(|(_, row, _)| row.id.clone()).collect();
    let mut selected = BTreeSet::new();
    for (index, (_, row, pasted)) in rows.iter_mut().enumerate() {
        if *pasted {
            selected.insert(index);
            if let Some(id) = &mut row.id { *id = numbered_id(id, &mut taken, list.storage); }
        }
    }
    Ok(PastePlan { rows: rows.into_iter().map(|(_, row, _)| row).collect(), selected })
}

pub(in crate::app) fn replace_ops(list: &StringList, rows: Vec<StringEntry>, game: Option<GameId>) -> DeferredOps {
    let mut edits = Vec::new();
    if !list.entries.is_empty() { edits.push(StringListEdit::Remove { indices: (0..list.entries.len()).collect() }); }
    if !rows.is_empty() { edits.push(StringListEdit::Insert { at: 0, entries: rows }); }
    DeferredOps { string_list_ops: edits.into_iter().map(|edit| crate::core::document::ops::StringListOp { game, edit }).collect(), ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(classic: bool, names: &[&str]) -> StringList {
        let path = crate::core::bundled::locate_definitions_root().join(if classic { "haloce_mcc/unicode_string_list.json" } else { "halo3_mcc/multilingual_unicode_string_list.json" });
        let tag = TagFile::new(path).unwrap();
        let mut list = strings::read_string_list(&tag).unwrap();
        list.entries = names.iter().map(|name| {
            let mut entry = list.new_entry();
            entry.id = (!classic).then(|| (*name).to_owned());
            for (index, text) in entry.texts.iter_mut().enumerate() { *text = Some(format!("{name}:{index}")); }
            entry
        }).collect();
        list
    }

    fn data(list: &StringList, indices: &[usize]) -> RowData {
        RowData { hud: list.storage == Storage::HudMessages, token: "test-token".to_owned(), classic: list.storage == Storage::Classic,
            languages: list.languages.iter().map(|language| language.name.clone()).collect(), language: 0,
            rows: indices.iter().map(|&index| Row { id: list.entries[index].id.clone(), texts: list.entries[index].texts.clone() }).collect() }
    }

    fn ids(rows: &[StringEntry]) -> Vec<&str> { rows.iter().map(|row| row.id.as_deref().unwrap()).collect() }

    #[test]
    fn insertion_at_a_destination_row_preserves_local_cut_ids_and_all_languages() {
        let list=list(false,&["A_one","B_one","A_two","plain"]);
        let source=data(&list,&[3]);
        let moved=plan(&list,&source,0,&BTreeSet::from([1]),PasteMode::Above,&BTreeSet::from([3])).unwrap();
        assert_eq!(ids(&moved.rows),vec!["A_one","plain","B_one","A_two"]);
        assert_eq!(moved.rows[1].texts,list.entries[3].texts);
        assert_eq!(moved.selected,BTreeSet::from([1]));
    }

    #[test]
    fn overwrite_inserts_append_and_noncontiguous_copies_use_numbered_names() {
        let list = list(false, &["A", "B", "C", "D", "E", "F"]);
        let source = data(&list, &[1, 3, 5]);
        let pasted = plan(&list, &source, 0, &BTreeSet::from([4, 5]), PasteMode::Overwrite, &BTreeSet::new()).unwrap();
        assert_eq!(ids(&pasted.rows), ["A", "B", "C", "D", "B (1)", "D (1)", "F"]);
        assert_eq!(pasted.selected, BTreeSet::from([4, 5, 6]));
        assert_eq!(pasted.rows[4].texts, list.entries[1].texts, "hidden languages are copied too");
        let inserted = plan(&list, &source, 0, &BTreeSet::from([1, 4]), PasteMode::Below, &BTreeSet::new()).unwrap();
        assert_eq!(ids(&inserted.rows), ["A", "B", "C", "D", "E", "B (1)", "D (1)", "F (1)", "F"]);
        let above = plan(&list, &source, 0, &BTreeSet::from([1, 4]), PasteMode::Above, &BTreeSet::new()).unwrap();
        assert_eq!(above.selected, BTreeSet::from([1, 2, 3]));
    }

    #[test]
    fn overlapping_cut_reuses_destination_rows_and_deletes_only_unused_sources() {
        let list = list(false, &["A", "B", "C", "D", "E", "F"]);
        let source = data(&list, &[1, 2]);
        let selected = BTreeSet::from([1, 2]);
        let moved = plan(&list, &source, 0, &BTreeSet::from([2]), PasteMode::Overwrite, &selected).unwrap();
        assert_eq!(ids(&moved.rows), ["A", "B", "C", "E", "F"]);
        assert_eq!(moved.selected, BTreeSet::from([1, 2]));
        assert_eq!(moved.rows[1].texts, list.entries[1].texts);
        assert_eq!(moved.rows[2].texts, list.entries[2].texts);
        for mode in [PasteMode::Above, PasteMode::Below, PasteMode::Overwrite] {
            let unchanged = plan(&list, &source, 0, &selected, mode, &selected).unwrap();
            assert_eq!(unchanged.rows, list.entries, "moving onto the original group is a no-op: {mode:?}");
        }
    }

    #[test]
    fn spreadsheet_input_converts_unicode_and_breaks_preserves_other_languages() {
        let list = list(false, &["A", "B"]);
        let syntax = super::super::string_list::text_syntax(None, Some(GameId::Halo3));
        let data = external("A\t\"日本語\n\u{E100}\"\r\n\tnew", &list, 0, &syntax).unwrap();
        let result = plan(&list, &data, 0, &BTreeSet::from([0]), PasteMode::Overwrite, &BTreeSet::new()).unwrap();
        assert_eq!(result.rows[0].texts[0].as_deref(), Some("日本語\n\u{E100}"));
        assert_eq!(result.rows[0].texts[1], list.entries[0].texts[1]);
        assert_eq!(result.rows[1].id.as_deref(), Some("new_string"));
        let blank = external("A\t", &list, 0, &syntax).unwrap();
        assert_eq!(plan(&list, &blank, 0, &BTreeSet::from([0]), PasteMode::Overwrite, &BTreeSet::new()).unwrap().rows[0].texts[0].as_deref(), Some(""));
        for invalid in ["one cell", "a\tb\tc", "a\tb\nwrong", "\"unterminated"] { assert!(external(invalid, &list, 0, &syntax).is_err(), "{invalid}"); }
    }

    #[test]
    fn ce_conversion_uses_selected_language_and_native_unicode_values() {
        let multilingual = list(false, &["A"]);
        let classic = list(true, &[]);
        let mut source = data(&multilingual, &[0]);
        source.language = 1;
        source.rows[0].texts[1] = Some("日本語 \u{E100}\r\n".to_owned());
        let converted = plan(&classic, &source, 0, &BTreeSet::new(), PasteMode::Overwrite, &BTreeSet::new()).unwrap();
        assert_eq!(converted.rows[0].id, None);
        assert_eq!(converted.rows[0].texts[0].as_deref(), Some("日本語 \u{E100}\r\n"));
        let classic_source = list(true, &["CE"]);
        let converted = plan(&multilingual, &data(&classic_source, &[0]), 2, &BTreeSet::new(), PasteMode::Above, &BTreeSet::new()).unwrap();
        assert_eq!(converted.rows[1].id.as_deref(), Some("new_string"));
        assert_eq!(converted.rows[1].texts[2], classic_source.entries[0].texts[0]);
        assert!(converted.rows[1].texts[1].is_none());
        assert!(external("one\r\ntwo", &classic, 0, &TextSyntax::unknown()).is_ok());
        assert_eq!(external("one\r\n\r\ntwo\r\n", &classic, 0, &TextSyntax::unknown()).unwrap().rows.len(), 3);
        assert_eq!(external("\r\n", &classic, 0, &TextSyntax::unknown()).unwrap().rows[0].texts[0].as_deref(), Some(""));
        assert!(external("id\ttext", &classic, 0, &TextSyntax::unknown()).is_err());
    }

    #[test]
    fn clipboard_keeps_all_languages_internally_and_only_visible_columns_externally() {
        let classic = data(&list(true, &["CE"]), &[0]);
        let list = list(false, &["A", "B"]);
        let mut source = data(&list, &[0, 1]);
        source.rows[0].texts[0] = Some("Quote \" comma, tab\tline\n\u{E100}".to_owned());
        let syntax = super::super::string_list::text_syntax(None, Some(GameId::Halo3));
        let (text, html) = encode(&source, &syntax).unwrap();
        assert!(text.starts_with("A\t"));
        assert!(!text.contains("String ID"));
        assert!(text.contains("&a_button"));
        let (decoded, visible) = decode(&html).unwrap();
        assert_eq!(decoded, source);
        assert_eq!(visible, text);
        let cells = crate::app::export::string_transfer::delimited_rows(&text, '\t').unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].len(), 2);
        assert_eq!(cells[0][1], strings::format_shown(source.rows[0].texts[0].as_deref().unwrap(), &syntax));
        let table = html.split_once("-->").unwrap().1;
        assert_eq!(table.matches("<td>").count(), 4);
        assert_eq!(table.matches("<tr>").count(), 2);
        source.language = 1;
        let (other_text, other_html) = encode(&source, &syntax).unwrap();
        let other_cells = crate::app::export::string_transfer::delimited_rows(&other_text, '\t').unwrap();
        assert_eq!(other_cells[0], ["A", source.rows[0].texts[1].as_deref().unwrap()]);
        assert_eq!(decode(&other_html).unwrap().0, source);
        source.language = 0;
        let ctx = egui::Context::default();
        ctx.data_mut(|memory| memory.insert_temp(egui::Id::unique(CLIPBOARD_ID), (text, html.clone())));
        assert_eq!(clipboard(&ctx, &list, 0, &syntax).unwrap(), source);
        ctx.data_mut(|memory| memory.insert_temp(egui::Id::unique(CLIPBOARD_ID), ("Changed\ttext".to_owned(), html)));
        assert_eq!(clipboard(&ctx, &list, 0, &syntax).unwrap().rows[0].id.as_deref(), Some("Changed"));
        assert!(decode("<!--BABOON_STRING_ROWS_V1:invalid-->").is_none());
        let (text, html) = encode(&classic, &TextSyntax::unknown()).unwrap();
        assert_eq!(crate::app::export::string_transfer::delimited_rows(&text, '\t').unwrap()[0].len(), 1);
        assert_eq!(html.split_once("-->").unwrap().1.matches("<td>").count(), 1);
    }

    #[test]
    fn language_columns_match_by_name_and_missing_columns_are_reported() {
        let list = list(false, &["A"]);
        let mut source = data(&list, &[0]);
        source.languages.swap(0, 1);
        source.rows[0].texts.swap(0, 1);
        let pasted = plan(&list, &source, 0, &BTreeSet::from([0]), PasteMode::Overwrite, &BTreeSet::new()).unwrap();
        assert_eq!(pasted.rows, list.entries);
        source.languages[0] = "Unsupported language".to_owned();
        let (missing, extra) = language_difference(&source, &list);
        assert_eq!(missing, ["Unsupported language"]);
        assert_eq!(extra, [list.languages[1].name.clone()]);
    }
}
