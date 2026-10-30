//! Saved-data localized string references: warning-only input and compact search.
use super::*;
use crate::app::editor::string_picker;
use blam_tags::string_editor::{StringReferenceList, StringReferenceValidation};

thread_local! {
    static REFERENCES: RefCell<HashMap<(String,String), ReferenceCache>> = RefCell::new(HashMap::new());
}
struct ReferenceCache {
    editor: String,
    document: u64,
    source: SourceIdentity,
    list: Arc<StringReferenceList>,
}

enum SourceIdentity {
    Loose(PathBuf, Option<GameId>, PathBuf),
    Single(PathBuf),
    Monolithic(std::sync::Weak<blam_tags::monolithic::MonolithicCache>),
    Containers(std::sync::Weak<crate::core::source::ContainerTagIndex>),
}
impl PartialEq for SourceIdentity {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Loose(a, ag, ad), Self::Loose(b, bg, bd)) => a == b && ag == bg && ad == bd,
            (Self::Single(a), Self::Single(b)) => a == b,
            (Self::Monolithic(a), Self::Monolithic(b)) => a.ptr_eq(b),
            (Self::Containers(a), Self::Containers(b)) => a.ptr_eq(b),
            _ => false,
        }
    }
}
impl Eq for SourceIdentity {}
impl SourceIdentity {
    fn of(source: &crate::core::source::TagSource) -> Self {
        use crate::core::source::TagSource;
        match source {
            TagSource::LooseFolder {
                root,
                game,
                definitions_root,
            } => Self::Loose(root.clone(), *game, definitions_root.clone()),
            TagSource::SingleFile { path } => Self::Single(path.clone()),
            TagSource::MonolithicCache { cache, .. } => Self::Monolithic(Arc::downgrade(cache)),
            TagSource::IoStoreContainerSet { index, .. } => Self::Containers(Arc::downgrade(index)),
        }
    }
}
/// Saved tags are reread when the popup opens.
struct SavedTags<'a> {
    source: &'a LoadedSourceData,
}
impl<'a> SavedTags<'a> {
    fn new(source: &'a LoadedSourceData) -> Self {
        Self { source }
    }
}
impl blam_tags::string_editor::TagSource for SavedTags<'_> {
    fn tag(&self, group: u32, path: &str) -> Option<Arc<blam_tags::TagFile>> {
        let extension = self.source.names.name_for(group)?;
        crate::app::export::load_referenced_tag_from_source(
            &self.source.source,
            path,
            extension,
            &group.to_be_bytes(),
        )
        .ok()
        .map(Arc::new)
    }
}

pub(super) fn is_hud_message_list(edit: &FieldEditContext<'_>) -> bool {
    let Some(tag) = edit.string_editor_tag else { return false; };
    let Some(editors) = string_editors(edit.definitions_root, edit.game) else { return false; };
    let path = crate::app::browser::entry_rel_path(tag.entry);
    let context = EditorContext { tag: tag.tag, tag_path: &path, field_path: "" };
    let Some(list) = editors.string_references("hud_message", &context, &SavedTags::new(tag.source)) else { return false; };
    let normalize = |path: &str| path.trim_end_matches('\0').replace('/', "\\").to_ascii_lowercase();
    list.source.iter().chain(list.groups.iter().map(|group| &group.source))
        .any(|source| source.group_tag == tag.entry.group_tag && normalize(&source.path) == normalize(&path))
}

fn snapshot(
    edit: &FieldEditContext<'_>,
    editors: &StringEditors,
    editor: &str,
    path: &str,
    refresh: bool,
) -> Arc<StringReferenceList> {
    let Some(tag) = edit.string_editor_tag else {
        return Arc::new(StringReferenceList::unavailable(
            "This view has no saved tag source.",
        ));
    };
    let key = (edit.tag_key.to_owned(), editor.to_owned());
    let source_identity = SourceIdentity::of(&tag.source.source);
    let cached = REFERENCES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let entry = cache.get_mut(&key)?;
        if refresh
            || entry.document != tag.stamp.0
            || entry.editor != editor
            || entry.source != source_identity
        {
            return None;
        }
        Some(entry.list.clone())
    });
    if let Some(list) = cached {
        return list;
    }
    let tag_path = kit_relative_path(tag.source, tag.entry).unwrap_or_default();
    let context = EditorContext {
        tag: tag.tag,
        tag_path: &tag_path,
        field_path: path,
    };
    let saved = SavedTags::new(tag.source);
    let mut list = editors
        .string_references(editor, &context, &saved)
        .unwrap_or_else(|| StringReferenceList::unavailable("No string-reference rule was found."));
    let syntax = crate::app::editor::string_list::text_syntax(edit.definitions_root, edit.game);
    for entry in &mut list.entries {
        entry.preview = entry
            .preview
            .as_deref()
            .map(|text| blam_tags::string_list::format_shown(text, &syntax));
    }
    let list = Arc::new(list);
    REFERENCES.with(|cache| {
        cache.borrow_mut().insert(
            key,
            ReferenceCache {
                editor: editor.to_owned(),
                document: tag.stamp.0,
                source: source_identity,
                list: list.clone(),
            },
        );
    });
    list
}

fn matches(entry: &blam_tags::string_editor::StringReferenceEntry, query: &str) -> bool {
    let query = query.to_lowercase();
    entry.id.to_lowercase().contains(&query)
        || entry
            .preview
            .as_ref()
            .is_some_and(|text| text.to_lowercase().contains(&query))
}

fn entry_button(
    ui: &mut Ui,
    entry: &blam_tags::string_editor::StringReferenceEntry,
    enabled: bool,
    picked: &mut Option<String>,
) {
    let details = entry.preview.as_deref().unwrap_or("No English text is available for this ID.");
    if string_picker::entry(ui, &entry.id, enabled, Some(details))
        .clicked()
    {
        *picked = Some(entry.id.clone());
        crate::app::ui_kit::button_icons::close_menu(ui);
    }
}

fn grouped_entries(
    ui: &mut Ui,
    entries: &[blam_tags::string_editor::StringReferenceEntry],
    filter: &str,
    popup_id: egui::Id,
    enabled: bool,
    picked: &mut Option<String>,
) {
    use crate::app::editor::string_groups::{StringIdGroup, group_string_ids};
    for group in group_string_ids(entries.iter().map(|entry| entry.id.as_str())) {
        match group {
            StringIdGroup::Entry(index) => {
                let entry = &entries[index];
                if matches(entry, filter) {
                    entry_button(ui, entry, enabled, picked);
                }
            }
            StringIdGroup::Prefix { prefix, indices } => {
                // Group membership comes from the full source, not the filtered
                // result, so a group doesn't disappear when one member matches.
                let entries: Vec<_> = indices
                    .iter()
                    .map(|&index| &entries[index])
                    .filter(|entry| matches(entry, filter))
                    .collect();
                if entries.is_empty() {
                    continue;
                }
                let caption = string_picker::group_caption(ui, &prefix, entries.len());
                if filter.is_empty() {
                    let group_id = popup_id.with(&prefix);
                    string_picker::submenu(ui, group_id, caption, |ui| {
                        let width = string_picker::menu_width(ui, entries.iter().map(|entry| (entry.id.clone(), false)));
                        ui.set_width(width);
                        string_picker::scroll(ui, group_id.with("choices")).show(ui, |ui| {
                            for entry in entries {
                                entry_button(ui, entry, enabled, picked);
                            }
                        });
                    });
                } else {
                    // Search shows every matching member inline beneath its group.
                    string_picker::group_heading(ui, caption);
                    ui.indent((popup_id, &prefix, "search"), |ui| {
                        for entry in entries {
                            entry_button(ui, entry, enabled, picked);
                        }
                    });
                }
            }
        }
    }
}

fn choice_rows(entries: &[blam_tags::string_editor::StringReferenceEntry], filter: &str, grouped: bool) -> Vec<(String, bool, usize)> {
    use crate::app::editor::string_groups::{StringIdGroup, group_string_ids};
    let mut rows = Vec::new();
    if grouped {
        for group in group_string_ids(entries.iter().map(|entry| entry.id.as_str())) {
            match group {
                StringIdGroup::Entry(index) => {
                    let entry = &entries[index];
                    if matches(entry, filter) { rows.push((entry.id.clone(), false, 0)); }
                }
                StringIdGroup::Prefix { prefix, indices } => {
                    let entries: Vec<_> = indices.iter().map(|&index| &entries[index])
                        .filter(|entry| matches(entry, filter)).collect();
                    if entries.is_empty() { continue; }
                    rows.push((format!("{prefix} ({})", entries.len()), filter.is_empty(), 0));
                    if !filter.is_empty() {
                        rows.extend(entries.iter().map(|entry| (entry.id.clone(), false, 1)));
                    }
                }
            }
        }
    } else {
        rows.extend(entries.iter().filter(|entry| matches(entry, filter))
            .map(|entry| (entry.id.clone(), false, 0)));
    }
    rows
}

fn source_label(list: &StringReferenceList, group: &blam_tags::string_editor::StringReferenceGroup) -> String {
    let basename = |path: &str| path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned();
    let name = basename(&group.source.path);
    if list.groups.iter().filter(|g| basename(&g.source.path).eq_ignore_ascii_case(&name)).count() > 1 {
        group.source.path.clone()
    } else { name }
}

fn popup_width(ui: &Ui, list: &StringReferenceList, filter: &str, grouped: bool) -> f32 {
    let rows = if list.groups.len() > 1 {
        list.groups.iter().flat_map(|group| {
            let entries = &list.entries[group.entries.clone()];
            let count = entries.iter().filter(|e| matches(e, filter)).count();
            if !filter.is_empty() && count == 0 { return Vec::new(); }
            let mut rows = vec![(format!("{} ({count})", source_label(list, group)), filter.is_empty(), 0)];
            if !filter.is_empty() { rows.extend(choice_rows(entries, filter, grouped).into_iter().map(|(s,b,d)|(s,b,d+1))); }
            rows
        }).collect()
    } else { choice_rows(&list.entries, filter, grouped) };
    string_picker::popup_width(ui, rows)
}

fn source_entries(ui: &mut Ui, list: &StringReferenceList, filter: &str, grouped: bool, popup_id: egui::Id, enabled: bool, picked: &mut Option<String>) {
    if !filter.is_empty() && !list.entries.iter().any(|entry| matches(entry, filter)) {
        ui.label("No matching String IDs.");
        return;
    }
    for (index, group) in list.groups.iter().enumerate() {
        let entries = &list.entries[group.entries.clone()];
        let count = entries.iter().filter(|e| matches(e, filter)).count();
        if !filter.is_empty() && count == 0 { continue; }
        let caption = string_picker::group_caption(ui, &source_label(list, group), count);
        let group_id = popup_id.with(("source", index));
        if filter.is_empty() {
            string_picker::submenu(ui, group_id, caption, |ui| {
                ui.set_width(string_picker::popup_width(ui, choice_rows(entries, "", grouped)));
                ui.label(RichText::new(&group.source.path).small().color(subtle_dark()));
                ui.separator();
                string_picker::scroll(ui, group_id.with(("choices", grouped))).show(ui, |ui| {
                    if let Some(reason) = &group.unavailable { ui.label(reason); }
                    else if entries.is_empty() { ui.label("The source string list is empty."); }
                    else if grouped { grouped_entries(ui, entries, "", group_id, enabled, picked); }
                    else { for entry in entries { entry_button(ui, entry, enabled, picked); } }
                });
            });
        } else {
            string_picker::group_heading(ui, caption);
            ui.indent(group_id.with("search"), |ui| {
                if grouped { grouped_entries(ui, entries, filter, group_id, enabled, picked); }
                else { for entry in entries.iter().filter(|e| matches(e, filter)) { entry_button(ui, entry, enabled, picked); } }
            });
        }
    }
}

fn value_preview(list: &StringReferenceList, id: &str) -> String {
    if list.groups.len() > 1 {
        let previews: Vec<_> = list.groups.iter().flat_map(|group| {
            list.entries[group.entries.clone()].iter().filter(|entry| entry.id == id).map(|entry| {
                format!("{}: {}", group.source.path, entry.preview.as_deref().unwrap_or("No English text is available for this ID."))
            })
        }).collect();
        if !previews.is_empty() { return previews.join("\n\n"); }
    }
    list.entries.iter().find(|entry|entry.id==id).and_then(|entry|entry.preview.clone())
        .unwrap_or_else(||"Choose a HUD message from the list.".to_owned())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    suffix: &str,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
    editors: &StringEditors,
    editor: &str,
) {
    let list = snapshot(edit, editors, editor, path, false);
    let caption = editors.caption(editor);
    let enabled = edit.can_edit(meta);
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let id = edit.widget_id(("text", &buffer_key));
    let popup_id = edit.widget_id(("string references", path));
    let filter_id = popup_id.with("filter");
    let was_open_id = popup_id.with("was open");
    let grouping_id = egui::Id::new("HUD message picker prefix grouping");
    let can_group = editor == "hud_message";
    let mut grouped = can_group
        && ui
            .data(|data| data.get_temp::<bool>(grouping_id))
            .unwrap_or(true);
    let mut filter = ui
        .data_mut(|data| data.get_temp::<String>(filter_id))
        .unwrap_or_default();
    let mut picked = None;
    let mut refresh = false;
    let mut popup_drawn = false;
    let mut open = false;
    let mut open_source = None;
    // Read the draft before the row borrows its other sinks.
    let draft = edit.buffers.draft_mut(&buffer_key, value);
    let validation = list.validate(&draft.text);
    draw_foundation_labelled_cell_row(ui, meta, "", depth, |ui, available_width| {
        let width = foundation_value_width(&draft.text, available_width).clamp(150.0, 300.0);
        let response = foundation_string_id_value_cell(
            ui,
            &mut draft.text,
            width,
            id,
            enabled,
            validation == StringReferenceValidation::Missing,
        );
        draft.note_response(&response);
        let hover = match validation {
            StringReferenceValidation::Missing => "This ID was not found in the HUD message list. Manual edits and saving are still allowed.".to_owned(),
            StringReferenceValidation::Unavailable => format!("Could not validate: {}",list.unavailable.as_deref().unwrap_or("source unavailable")),
            StringReferenceValidation::Valid => value_preview(&list, &draft.text),
        };
        response.clone().on_hover_text(hover);
        if enabled && draft.should_commit(ui, &response) {
            edit.pending
                .extend(field_edit_ops(path, &draft.text).pending);
        }
        draft.keep_commit(|| single_field_commit(edit.tag_key, &buffer_key, path));

        // Row culling changes preceding widget counts, especially inside blocks.
        // An explicit UI ID keeps the trigger's interaction and popup IDs stable.
        ui.scope_builder(egui::UiBuilder::new().id(popup_id), |ui| {
            let image = tag_icon_image(
                ui.ctx(), tag_icon(Some(u32::from_be_bytes(*b"unic")), edit.game), 16.0,
            );
            crate::app::ui_kit::button_icons::text_dropdown_button(ui, &caption, Some(image), |ui| {
                popup_drawn = true;
                let just_opened = !ui
                    .data(|data| data.get_temp::<bool>(was_open_id))
                    .unwrap_or(false);
                if just_opened {
                    refresh = true;
                    filter.clear();
                }
                let width = popup_width(ui, &list, &filter, grouped);
                string_picker::search(ui, &mut filter, filter_id, just_opened, width, "Search String IDs or Text");
                if can_group {
                    ui.checkbox(&mut grouped, "Group by Prefix");
                }
                ui.separator();
                if let Some(reason) = &list.unavailable {
                    ui.label(format!("Could not validate: {reason}"));
                }
                {
                    let entries: Vec<_> = list
                        .entries
                        .iter()
                        .filter(|entry| matches(entry, &filter))
                        .collect();
                    let scroll = string_picker::scroll(
                        ui, popup_id.with(("choices", grouped)),
                    );
                    let empty_message = if list.entries.is_empty() {
                        "The source string list is empty."
                    } else {
                        "No matching String IDs."
                    };
                    if list.groups.len() > 1 {
                        scroll.show(ui, |ui| source_entries(ui, &list, &filter, grouped, popup_id, enabled, &mut picked));
                    } else if grouped {
                        scroll.show(ui, |ui| {
                            if entries.is_empty() && list.unavailable.is_none() {
                                ui.label(empty_message);
                            } else {
                                grouped_entries(ui, &list.entries, &filter, popup_id, enabled, &mut picked);
                            }
                        });
                    } else {
                        scroll.show_rows(ui, BUTTON_HEIGHT, entries.len(), |ui, range| {
                            if entries.is_empty() && list.unavailable.is_none() {
                                ui.label(empty_message);
                            }
                            for index in range {
                                entry_button(ui, entries[index], enabled, &mut picked);
                            }
                        });
                    }
                }
            });
        });
        // Navigation remains available for read-only fields, just like tag references.
        open = ui
            .scope_builder(egui::UiBuilder::new().id(popup_id.with("open")), |ui| {
                if list.groups.len() > 1 {
                    let image = button_icon_image(ui, ButtonIcon::Open, text_dark(), 16.0);
                    crate::app::ui_kit::button_icons::text_dropdown_button(ui, "Open", Some(image), |ui| {
                        ui.set_width(250.0);
                        for group in &list.groups {
                            if string_picker::entry(ui, &source_label(&list, group), true, Some(&group.source.path)).clicked() {
                                open_source = Some(group.source.clone());
                                crate::app::ui_kit::button_icons::close_menu(ui);
                            }
                        }
                    });
                    false
                } else {
                    icon_button(ui, ButtonIcon::Open, "Open source string list", list.source.is_some(), text_dark()).clicked()
                }
            })
            .inner;
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        if validation == StringReferenceValidation::Unavailable {
            ui.label(
                RichText::new("Could not validate")
                    .color(subtle_dark())
                    .small(),
            );
        }
    });
    if let Some(id) = picked {
        edit.buffers.draft_mut(&buffer_key, value).text = id.clone();
        edit.pending.extend(field_edit_ops(path, &id).pending);
    }
    if let Some(source) = open_source.as_ref().or_else(|| open.then_some(list.source.as_ref()).flatten()) {
        *edit.open_request = Some(OpenTagRequest {
            group_tag: source.group_tag,
            rel_path: source.path.clone(),
            float: ui.input(|input| input.modifiers.alt),
        });
    }
    ui.data_mut(|data| {
        data.insert_temp(filter_id, filter);
        data.insert_temp(was_open_id, popup_drawn);
        if can_group {
            data.insert_temp(grouping_id, grouped);
        }
    });
    if refresh {
        snapshot(edit, editors, editor, path, true);
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::LooseKit;
    use blam_tags::classic::ClassicEngine;
    use blam_tags::fields::{TagFieldData, TagReferenceData};
    use blam_tags::string_list::{self as strings, StringListEdit};
    use blam_tags::{StringIdData, TagFile};
    const FIELD: &str = "item/pickup message";
    fn native(group: &str) -> TagFile {
        TagFile::new_classic(
            locate_definitions_root().join(format!("halo2_mcc/{group}.json")),
            ClassicEngine::Halo2V4,
        )
        .unwrap()
    }
    fn set_ref(tag: &mut TagFile, path: &str, group: &[u8; 4], target: &str) {
        tag.root_mut()
            .field_path_mut(path)
            .unwrap()
            .set(TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: Some((u32::from_be_bytes(*group), target.into())),
            }))
            .unwrap();
    }
    fn write(kit: &LooseKit, path: &str, tag: &TagFile) {
        let path = kit.path(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        tag.write(path).unwrap();
    }
    fn string_tag(rows: &[(&str, Option<&str>)]) -> TagFile {
        let mut tag = native("multilingual_unicode_string_list");
        let list = strings::read_string_list(&tag).unwrap();
        let entries = rows
            .iter()
            .map(|(id, text)| {
                let mut row = list.new_entry();
                row.id = Some((*id).into());
                row.texts[0] = text.map(str::to_owned);
                row
            })
            .collect();
        strings::apply_string_list_edits(
            &mut tag,
            Some(GameId::Halo2),
            &[StringListEdit::Insert { at: 0, entries }],
        )
        .unwrap();
        tag
    }
    struct Panel {
        game: GameId,
        field: &'static str,
        kit: LooseKit,
        source: LoadedSourceData,
        tag: TagFile,
        entry: TagEntry,
        ctx: egui::Context,
        sinks: EditSinks,
        time: f64,
        enabled: bool,
        filter_id: Option<egui::Id>,
        field_id: Option<egui::Id>,
    }
    struct Frame {
        icons: Vec<egui::Rect>,
        popup_height: Option<f32>,
        none_color: Option<Color32>,
        texts: Vec<(String, egui::Rect)>,
        pink: bool,
        pending: Vec<PendingFieldEdit>,
        open: Option<OpenTagRequest>,
    }
    impl Frame {
        fn has(&self, text: &str) -> bool {
            self.texts.iter().any(|(s, _)| s == text)
        }
        fn at(&self, text: &str) -> egui::Pos2 {
            self.texts
                .iter()
                .find(|(s, _)| s == text)
                .unwrap_or_else(|| panic!("{text}: {:?}", self.texts))
                .1
                .center()
        }
    }
    impl Panel {
        fn new(value: &str, rows: Option<&[(&str, Option<&str>)]>) -> Self {
            let kit = LooseKit::new("h2-hud-string-reference", "halo2_mcc");
            let mut globals = native("globals");
            {
                let mut root = globals.root_mut();
                let mut field = root.field_path_mut("interface tags").unwrap();
                field.as_block_mut().unwrap().add_element();
            }
            set_ref(
                &mut globals,
                "interface tags[0]/hud globals",
                b"hudg",
                "ui\\custom",
            );
            write(&kit, "globals/globals.globals", &globals);
            let mut hud = native("hud_globals");
            set_ref(&mut hud, "new globals/hud text", b"unic", "ui\\messages");
            write(&kit, "ui/custom.hud_globals", &hud);
            if let Some(rows) = rows {
                write(
                    &kit,
                    "ui/messages.multilingual_unicode_string_list",
                    &string_tag(rows),
                );
            }
            let mut tag = native("weapon");
            tag.root_mut()
                .field_path_mut(FIELD)
                .unwrap()
                .set(TagFieldData::StringId(StringIdData {
                    string: value.into(),
                }))
                .unwrap();
            write(&kit, "objects/test.weapon", &tag);
            let source = kit.source();
            let entry = source
                .entry_for_key(&kit.key("objects/test.weapon"))
                .unwrap()
                .clone();
            Self {
                game: GameId::Halo2,
                field: FIELD,
                kit,
                source,
                tag,
                entry,
                ctx: egui::Context::default(),
                sinks: EditSinks::default(),
                time: 0.0,
                enabled: true,
                filter_id: None,
                field_id: None,
            }
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> Frame {
            self.time += 0.2;
            let defs = locate_definitions_root();
            let mut pending = Vec::new();
            let mut open = None;
            let mut filter_id = None;
            let mut field_id = None;
            let input = crate::app::loose_fixture::screen(events, self.time);
            let output = crate::app::run_ui_test(&self.ctx, input, |ui| {
                let mut edit =
                    FieldEditContext::read_only(&mut self.sinks, "reference-test", &self.entry.key);
                edit.root = Some(self.tag.root());
                edit.game = Some(self.game);
                edit.definitions_root = Some(&defs);
                edit.editable = self.enabled;
                edit.string_editor_tag = Some(EditedTag {
                    tag: &self.tag,
                    entry: &self.entry,
                    stamp: (1, 0),
                    source: &self.source,
                });
                field_string_editor(&edit, self.field).expect("annotated inherited field");
                filter_id = Some(edit.widget_id(("string references", self.field)).with("filter"));
                field_id = Some(edit.widget_id(("text", format!("{}|{}", self.entry.key, self.field))));
                let field = self.tag.root().field_path(self.field).unwrap();
                draw_foundation_value_row(
                    ui,
                    field,
                    &field_display_meta("pickup message"),
                    "string id",
                    &field.value().unwrap(),
                    &self.source.names,
                    0,
                    self.field,
                    &mut edit,
                    None,
                    300.0,
                );
                pending = edit.pending.clone();
                edit.pending.clear();
                open = edit.open_request.take();
            });
            self.filter_id = filter_id;
            self.field_id = field_id;
            let mut texts = Vec::new();
            let mut pink = false;
            let mut icons = Vec::new();
            let mut none_color = None;
            for clipped in output.shapes {
                match clipped.shape {
                    egui::Shape::Text(t) => {
                        if t.galley.text() == "NONE" {
                            none_color = t
                                .galley
                                .job
                                .sections
                                .first()
                                .map(|section| section.format.color);
                        }
                        texts.push((
                            t.galley.text().into(),
                            egui::Rect::from_min_size(t.pos, t.galley.size()),
                        ));
                    }
                    egui::Shape::Rect(rect) => {
                        pink |= rect.fill == invalid_value_fill();
                        // The headless harness has no SVG loader. Measure the
                        // actual icon button frame rather than its image mesh.
                        if (rect.rect.width() - 24.0).abs() < 1.0
                            && (rect.rect.height() - 24.0).abs() < 1.0
                        {
                            icons.push(rect.rect);
                        }
                    }
                    _ => {}
                }
            }
            let popup_height = self.ctx.memory(|memory| {
                memory
                    .areas()
                    .visible_layer_ids()
                    .into_iter()
                    .filter(|layer| layer.order == egui::Order::Foreground)
                    .filter_map(|layer| memory.area_rect(layer.id).map(|rect| rect.height()))
                    .max_by(f32::total_cmp)
            });
            Frame {
                icons,
                popup_height,
                none_color,
                texts,
                pink,
                pending,
                open,
            }
        }
        fn click(&mut self, pos: egui::Pos2) -> Frame {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ])
        }
        fn resolve(&mut self, check_now: bool) -> Arc<StringReferenceList> {
            let defs = locate_definitions_root();
            let mut edit =
                FieldEditContext::read_only(&mut self.sinks, "reference-test", &self.entry.key);
            edit.root = Some(self.tag.root());
            edit.game = Some(self.game);
            edit.definitions_root = Some(&defs);
            edit.string_editor_tag = Some(EditedTag {
                tag: &self.tag,
                entry: &self.entry,
                stamp: (1, 0),
                source: &self.source,
            });
            let (editors, editor) = field_string_editor(&edit, self.field).unwrap();
            snapshot(&edit, &editors, &editor, self.field, check_now)
        }
    }
    #[test]
    fn string_references_empty_id_is_a_dim_placeholder_and_typing_starts_empty() {
        for available in [true, false] {
            for editable in [true, false] {
                let rows = [("valid", Some("A message"))];
                let mut panel = Panel::new("", if available { Some(&rows) } else { None });
                panel.enabled = editable;
                let frame = panel.frame(Vec::new());
                assert!(frame.has("NONE"));
                assert_eq!(frame.none_color, Some(text_dark().gamma_multiply(0.5)));
                assert!(!frame.pink, "unset IDs are never missing references");
                assert!(
                    !frame.has("Could not validate"),
                    "unset IDs need no lookup warning"
                );
                assert!(
                    frame.pending.is_empty(),
                    "drawing a placeholder must not edit the tag"
                );
                if editable {
                    panel
                        .ctx
                        .memory_mut(|memory| memory.request_focus(panel.field_id.unwrap()));
                    panel.frame(Vec::new());
                    panel.frame(vec![egui::Event::Text("new_id".into())]);
                    let frame = panel.frame(Vec::new());
                    assert!(frame.has("new_id"), "typed text should appear: {:?}; focused {:?}; expected {:?}", frame.texts, panel.ctx.memory(|memory| memory.focused()), panel.field_id);
                    assert!(!frame.has("NONE"));
                    let frame = panel.frame(vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }]);
                    assert!(
                        frame
                            .pending
                            .iter()
                            .any(|edit| edit.path == FIELD && edit.input == "new_id"),
                        "typing must not append to the NONE placeholder"
                    );
                }
            }
        }
    }
    #[test]
    fn string_references_docked_h2_first_click_without_prior_field_focus() {
        assert_docked_picker_first_click("halo2_mcc", "weapon");
    }
    #[test]
    fn string_references_vehicle_seat_first_click_without_prior_field_focus() {
        for game in ["halo2_mcc", "halo3_mcc", "haloreach_mcc"] {
            assert_docked_picker_first_click(game, "vehicle");
        }
    }
    fn assert_docked_picker_first_click(game: &'static str, group: &str) {
        let ids: Vec<_> = (0..400).map(|index| format!("message{index:03}")).collect();
        let rows: Vec<_> = ids
            .iter()
            .map(|id| (id.as_str(), Some("HUD prompt")))
            .collect();
        let kit = if game == "halo2_mcc" {
            Panel::new("", Some(&rows)).kit
        } else {
            let kit = LooseKit::new("hud-reference-seats", game);
            let new_tag = |group: &str| {
                TagFile::new(
                    locate_definitions_root()
                        .join(game)
                        .join(format!("{group}.json")),
                )
                .unwrap()
            };
            let mut globals = new_tag("globals");
            globals
                .root_mut()
                .field_path_mut("interface tags")
                .unwrap()
                .as_block_mut()
                .unwrap()
                .add_element();
            set_ref(
                &mut globals,
                "interface tags[0]/chud globals",
                b"chgd",
                "ui\\custom",
            );
            write(&kit, "globals/globals.globals", &globals);
            let mut chud = new_tag("chud_globals_definition");
            chud.root_mut()
                .field_path_mut("skins")
                .unwrap()
                .as_block_mut()
                .unwrap()
                .add_element();
            set_ref(&mut chud, "skins[0]/chud text", b"unic", "ui\\messages");
            write(&kit, "ui/custom.chud_globals_definition", &chud);
            let mut tag = new_tag("multilingual_unicode_string_list");
            let list = strings::read_string_list(&tag).unwrap();
            let entries = ids
                .iter()
                .map(|id| {
                    let mut entry = list.new_entry();
                    entry.id = Some(id.clone());
                    entry.texts[0] = Some("HUD prompt".into());
                    entry
                })
                .collect();
            strings::apply_string_list_edits(
                &mut tag,
                GameId::from_id(game),
                &[StringListEdit::Insert { at: 0, entries }],
            )
            .unwrap();
            write(&kit, "ui/messages.multilingual_unicode_string_list", &tag);
            kit
        };
        let rel = format!("objects/test.{group}");
        if group == "vehicle" {
            let mut vehicle = if game == "halo2_mcc" {
                native("vehicle")
            } else {
                TagFile::new(locate_definitions_root().join(game).join("vehicle.json")).unwrap()
            };
            vehicle
                .root_mut()
                .field_path_mut("unit/seats")
                .unwrap()
                .as_block_mut()
                .unwrap()
                .add_element();
            write(&kit, &rel, &vehicle);
        }
        let external = if group == "vehicle" {
            "BLAM_TEST_H2_VEHICLE"
        } else {
            "BLAM_TEST_H2_WEAPON"
        };
        if let Some(path) = std::env::var_os(external).filter(|_| game == "halo2_mcc") {
            std::fs::copy(path, kit.path(&rel)).unwrap();
        }
        let mut app = crate::app::Baboon::for_test();
        kit.install(&mut app);
        let _key = kit.open(&mut app, &rel);
        let ctx = egui::Context::default();
        crate::app::Baboon::configure_context(&ctx);
        let mut time = 0.0;
        let mut frame = |events| {
            time += 0.1;
            let mut input = crate::app::loose_fixture::screen(events, time);
            input.screen_rect.as_mut().unwrap().max.x = 1600.0;
            let output = crate::app::run_ui_test(&ctx, input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    app.draw_workspace_tiles(ui, &ctx);
                });
                app.apply_commands(&ctx);
            });
            output
                .shapes
                .into_iter()
                .filter_map(|clipped| match clipped.shape {
                    egui::Shape::Text(text) if clipped.clip_rect.contains(text.pos) => Some((
                        text.galley.text().to_owned(),
                        egui::Rect::from_min_size(text.pos, text.galley.size()),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let mut texts = Vec::new();
        for _ in 0..4 {
            texts = frame(Vec::new());
        }
        let mut pos = None;
        for _ in 0..45 {
            pos = texts
                .iter()
                .find(|(text, rect)| text == "HUD Messages" && rect.center().y < 780.0)
                .map(|(_, rect)| rect.center());
            if pos.is_some() {
                break;
            }
            texts = frame(vec![
                egui::Event::PointerMoved(egui::pos2(1300.0, 650.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0.0, -240.0),
                    modifiers: Default::default(),
                },
            ]);
            for _ in 0..4 {
                texts = frame(Vec::new());
            }
        }
        let pos = pos.unwrap_or_else(|| panic!("HUD Messages must be visible: {texts:?}"));
        assert!(ctx.memory(|memory| memory.focused()).is_none());
        frame(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
        for tick in 0..8 {
            let texts = frame(Vec::new());
            assert!(
                texts.iter().any(|(text, _)| text == "Search String IDs or Text"),
                "HUD picker must stay open before any field gets focus ({game}/{group}, frame {tick})"
            );
        }
    }
    #[test]
    fn string_references_type_label_follows_both_adjacent_buttons() {
        let mut panel = Panel::new("weapon_swap", Some(&[("weapon_swap", Some("Swap weapon"))]));
        let frame = panel.frame(Vec::new());
        assert_eq!(frame.icons.len(), 1);
        let label = frame
            .texts
            .iter()
            .find(|(text, _)| text == "string id")
            .unwrap()
            .1;
        let dropdown = frame
            .texts
            .iter()
            .find(|(text, _)| text == "HUD Messages")
            .unwrap()
            .1;
        assert!(dropdown.right() <= frame.icons[0].left());
        assert!(frame.icons[0].right() <= label.left());
    }
    #[test]
    fn string_references_optional_campaign_evolved_saved_container_lookup() {
        let paks = crate::core::test_kits::ce_paks();
        if !paks.exists() {
            return;
        }
        let defs = locate_definitions_root();
        let names =
            crate::core::format::TagNameIndex::load_game(&defs, GameId::CampaignEvolved).unwrap();
        let source = crate::core::source::load_iostore_container_set(paks, &names, &defs).unwrap();
        let editors = string_editors(None, Some(GameId::CampaignEvolved))
            .expect("bundled rules without loose-source definitions");
        let tag = TagFile::new(defs.join("haloce_evolved/weapon.json")).unwrap();
        assert_eq!(editors.editor_at(tag.root(), FIELD), Some("hud_message"));
        let list = editors
            .string_references(
                "hud_message",
                &EditorContext {
                    tag: &tag,
                    tag_path: "",
                    field_path: FIELD,
                },
                &SavedTags::new(&source),
            )
            .unwrap();
        assert!(list.unavailable.is_none(), "{:?}", list.unavailable);
        assert!(!list.entries.is_empty());
        assert!(list.source.is_some());
        eprintln!(
            "Campaign Evolved: resolved {} saved HUD IDs",
            list.entries.len()
        );
    }
    #[test]
    fn string_references_picker_searches_preview_without_expanding_rows_and_inserts_id() {
        let mut panel = Panel::new(
            "",
            Some(&[
                ("weapon_swap", Some("Hold action for a rifle")),
                ("empty_english", None),
            ]),
        );
        let frame = panel.frame(vec![]);
        assert!(!frame.pink);
        assert!(!frame.has("Open"));
        assert!(!frame.has("Hold action for a rifle"));
        panel.click(frame.at("HUD Messages"));
        let frame = panel.frame(vec![]);
        let search = panel.ctx.read_response(panel.filter_id.unwrap()).unwrap();
        assert!(
            (search.rect.height() - BUTTON_HEIGHT).abs() < 0.1,
            "HUD search height: {}",
            search.rect.height()
        );
        assert!(frame.has("weapon_swap"));
        assert!(frame.has("empty_english"));
        assert!(!frame.has("Hold action for a rifle"));
        assert!(!frame.has("Saved tags only"));
        panel
            .ctx
            .memory_mut(|memory| memory.request_focus(panel.filter_id.unwrap()));
        let frame = panel.frame(vec![egui::Event::Text("rifle".into())]);
        assert!(frame.has("weapon_swap"));
        assert!(!frame.has("empty_english"));
        let frame = panel.click(frame.at("weapon_swap"));
        assert!(
            frame
                .pending
                .iter()
                .any(|edit| edit.path == FIELD && edit.input == "weapon_swap")
        );
    }
    #[test]
    fn string_references_popup_width_follows_labels_and_truncated_tooltip_keeps_details() {
        for (name, expected) in [("plain".to_owned(), "minimum"), ("m".repeat(35), "content"), ("M".repeat(100), "cap")] {
            let prompt = "Full English prompt retained alongside the complete String ID.";
            let mut panel = Panel::new("", Some(&[(&name, Some(prompt))]));
            Baboon::configure_context(&panel.ctx);
            let frame = panel.frame(vec![]);
            panel.click(frame.at("HUD Messages"));
            panel.frame(vec![]);
            let frame = panel.frame(vec![]);
            let width = panel.ctx.read_response(panel.filter_id.unwrap()).unwrap().rect.width();
            match expected {
                "minimum" => assert!((width - 250.0).abs() < 1.0, "minimum width: {width}"),
                "content" => assert!(width > 250.0 && width < 400.0, "content width: {width}"),
                _ => assert!((width - 400.0).abs() < 1.0, "capped width: {width}"),
            }
            if expected == "cap" {
                panel.frame(vec![egui::Event::PointerMoved(frame.at(&name))]);
                let mut tooltip = panel.frame(vec![]);
                for _ in 0..8 { tooltip = panel.frame(vec![]); }
                assert!(tooltip.texts.iter().any(|(text, _)| text.contains(&name) && text.contains(prompt)),
                    "truncated IDs and their English text must share a tooltip: {:?}", tooltip.texts);
            }
        }
    }

    #[test]
    fn string_references_popup_shrinks_to_matches_and_grows_when_search_clears() {
        let names: Vec<_> = (0..80).map(|n| format!("prefix_{n:03}")).collect();
        let rows: Vec<_> = names
            .iter()
            .map(|name| (name.as_str(), Some("Prompt")))
            .collect();
        let mut panel = Panel::new("", Some(&rows));
        let frame = panel.frame(vec![]);
        panel.click(frame.at("HUD Messages"));
        let frame = panel.frame(vec![]);
        panel.click(frame.at("Group by Prefix"));
        let frame = panel.frame(vec![]);
        let initial = frame.popup_height.expect("open popup");
        assert!(initial > 200.0, "initial height {initial}");
        panel
            .ctx
            .memory_mut(|memory| memory.request_focus(panel.filter_id.unwrap()));
        panel.frame(vec![egui::Event::Text("prefix_079".into())]);
        let narrowed = panel.frame(vec![]).popup_height.unwrap();
        let id = panel.filter_id.unwrap();
        let mut state = egui::TextEdit::load_state(&panel.ctx, id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(10),
            )));
        state.store(&panel.ctx, id);
        panel.frame(vec![egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        let cleared = panel.frame(vec![]).popup_height.unwrap();
        assert!(
            narrowed < initial - 100.0 && (cleared - initial).abs() < 2.0,
            "popup height initial={initial}, narrowed={narrowed}, cleared={cleared}"
        );
    }

    #[test]
    fn string_references_prefix_groups_are_optional_and_search_reveals_members() {
        let mut panel = Panel::new(
            "",
            Some(&[
                ("BR_PICKUP", Some("Take the weapon")),
                ("BR_SWAP", Some("Swap for a rifle")),
                ("odd_entry", None),
                ("PLAIN", None),
            ]),
        );
        let frame = panel.frame(vec![]);
        panel.click(frame.at("HUD Messages"));
        let frame = panel.frame(vec![]);
        assert!(frame.has("BR (2)"));
        assert!(frame.has("odd_entry"));
        assert!(frame.has("PLAIN"));
        assert!(!frame.has("BR_PICKUP"));
        panel.click(frame.at("BR (2)"));
        let frame = panel.frame(vec![]);
        assert!(frame.has("BR_PICKUP"));
        assert!(frame.has("BR_SWAP"));
        let popup_count = panel.ctx.memory(|memory| memory.areas().visible_layer_ids().into_iter()
            .filter(|layer| layer.order == egui::Order::Foreground).count());
        assert!(popup_count >= 2, "HUD prefix browsing opens a separate submenu");
        panel.click(frame.at("Group by Prefix"));
        let frame = panel.frame(vec![]);
        assert!(!frame.has("BR (2)"));
        assert!(frame.has("BR_PICKUP"));
        assert!(frame.has("BR_SWAP"));
        panel.click(frame.at("Group by Prefix"));
        panel.frame(vec![]);
        panel
            .ctx
            .memory_mut(|memory| memory.request_focus(panel.filter_id.unwrap()));
        let frame = panel.frame(vec![egui::Event::Text("rifle".into())]);
        assert!(frame.has("BR (1)"));
        assert!(frame.has("BR_SWAP"));
        assert!(!frame.has("BR_PICKUP"));
        let frame = panel.click(frame.at("BR_SWAP"));
        assert!(frame.pending.iter().any(|edit| edit.input == "BR_SWAP"));
    }

    #[test]
    fn string_references_missing_id_warns_but_manual_input_commits() {
        let mut panel = Panel::new("missing", Some(&[("known", Some("Known text"))]));
        let frame = panel.frame(vec![]);
        assert!(frame.pink);
        assert!(!frame.has("Could not validate"));
        let id = panel.field_id.unwrap();
        panel.ctx.memory_mut(|memory| memory.request_focus(id));
        let mut state = egui::TextEdit::load_state(&panel.ctx, id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(7),
            )));
        state.store(&panel.ctx, id);
        panel.frame(vec![egui::Event::Text("manual_id".into())]);
        let frame = panel.frame(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        assert!(
            frame
                .pending
                .iter()
                .any(|edit| edit.path == FIELD && edit.input == "manual_id")
        );
        let mut document = TagDocument::clean(panel.tag);
        let applied = crate::core::document::apply::apply_deferred_ops(
            &mut document,
            crate::core::document::apply::DeferredOps {
                pending: frame.pending,
                ..Default::default()
            },
            "Manual string ID",
        );
        assert!(
            applied
                .outcomes
                .iter()
                .all(|outcome| outcome.result.is_ok())
        );
        let bytes = document
            .tag
            .write_to_bytes()
            .expect("unmatched IDs still serialize");
        let reopened = crate::core::source::read_tag_from_bytes(
            &bytes,
            Some(GameId::Halo2),
            Some(&locate_definitions_root()),
            u32::from_be_bytes(*b"weap"),
        )
        .unwrap();
        assert_eq!(
            reopened
                .root()
                .descend("item")
                .unwrap()
                .read_string_id("pickup message")
                .as_deref(),
            Some("manual_id")
        );
        assert!(
            document.journal.undo(&document.tag).is_some(),
            "manual input remains undoable"
        );
    }
    #[test]
    fn string_references_open_is_outside_popup_and_available_read_only() {
        let mut panel = Panel::new("known", Some(&[("known", Some("Known"))]));
        panel.enabled = false;
        let frame = panel.frame(vec![]);
        assert!(!frame.has("Open"));
        assert!(!frame.has("Search String IDs or Text"));
        let frame = panel.frame(vec![]);
        let frame = panel.click(frame.icons.first().expect("source Open icon").center());
        let open = frame.open.expect("adjacent source Open action");
        assert_eq!(open.group_tag, u32::from_be_bytes(*b"unic"));
        assert_eq!(open.rel_path, "ui\\messages");
        assert!(frame.pending.is_empty());
    }
    #[test]
    fn string_references_unavailable_and_empty_sources_have_different_feedback() {
        let mut panel = Panel::new("missing", None);
        let frame = panel.frame(vec![]);
        assert!(!frame.pink);
        assert!(frame.has("Could not validate"));
        panel.click(frame.at("HUD Messages"));
        let frame = panel.frame(vec![]);
        assert!(frame.popup_height.is_some(), "the popup must be open");
        assert!(!frame.has("The source string list is empty."), "an unavailable source is not an empty source");
        let mut panel = Panel::new("missing", Some(&[]));
        let frame = panel.frame(vec![]);
        assert!(frame.pink);
        assert!(!frame.has("Could not validate"));
        panel.click(frame.at("HUD Messages"));
        let frame = panel.frame(vec![]);
        assert!(frame.popup_height.is_some(), "the popup must be open");
        assert!(frame.has("The source string list is empty."));
    }
    #[test]
    fn string_references_hud_list_default_uses_the_globals_destination() {
        let panel = Panel::new("",Some(&[("A_one",Some("One")),("A_two",Some("Two"))]));
        let strings = string_tag(&[("A_one",Some("One")),("A_two",Some("Two"))]);
        let key = panel.kit.key("ui/messages.multilingual_unicode_string_list");
        let entry = panel.source.entry_for_key(&key).unwrap();
        let defs = locate_definitions_root();
        let mut sinks = EditSinks::default();
        let mut edit = FieldEditContext::read_only(&mut sinks,"default-grouping",&key);
        edit.game = Some(GameId::Halo2); edit.definitions_root = Some(&defs);
        edit.string_editor_tag = Some(EditedTag { tag: &strings, entry, stamp: (1,0), source: &panel.source });
        assert!(is_hud_message_list(&edit));
        let mut unrelated = entry.clone();
        unrelated.display_path = "ui\\hud_messages.multilingual_unicode_string_list".to_owned();
        edit.string_editor_tag = Some(EditedTag { tag: &strings, entry: &unrelated, stamp: (1,0), source: &panel.source });
        assert!(!is_hud_message_list(&edit),"a familiar filename alone does not identify the HUD list");
    }

    #[test]
    fn string_references_mounted_source_identity_changes_on_remount() {
        use crate::core::source::TagSource;
        let panel = Panel::new("", None);
        let mut source = panel.kit.source();
        source.source = TagSource::IoStoreContainerSet {
            root: panel.kit.root.clone(),
            containers: Vec::new(),
            index: Arc::new(Default::default()),
            packages: Arc::new(Default::default()),
            shipped: Arc::new(Default::default()),
        };
        let original = SourceIdentity::of(&source.source);
        assert!(original == SourceIdentity::of(&source.source));
        if let TagSource::IoStoreContainerSet { index, .. } = &mut source.source {
            *index = Arc::new(Default::default());
        }
        assert!(original != SourceIdentity::of(&source.source));
    }
    #[test]
    fn string_references_reopening_picker_reloads_saved_messages() {
        let mut panel = Panel::new("old", Some(&[("old", Some("Original"))]));
        let frame = panel.frame(Vec::new());
        panel.click(frame.at("HUD Messages"));
        panel.frame(Vec::new());
        write(
            &panel.kit,
            "ui/messages.multilingual_unicode_string_list",
            &string_tag(&[("replacement", Some("Saved replacement"))]),
        );
        let frame = panel.frame(Vec::new());
        assert!(
            frame.has("old"),
            "saved files remain cached while the picker stays open"
        );
        assert!(!frame.has("replacement"));
        panel.click(frame.at("HUD Messages"));
        let frame = panel.frame(Vec::new());
        assert!(!frame.has("Search String IDs or Text"));
        panel.click(frame.at("HUD Messages"));
        panel.frame(Vec::new());
        let frame = panel.frame(Vec::new());
        assert!(frame.has("replacement"));
        assert!(frame.pink, "the existing field value is now absent from the refreshed list");
    }
    #[test]
    fn string_references_saved_snapshot_checks_on_open_and_ignores_unsaved_sources() {
        let mut panel = Panel::new("old", Some(&[("old", Some("Old text"))]));
        assert_eq!(panel.resolve(false).entries[0].id, "old");
        write(
            &panel.kit,
            "ui/messages.multilingual_unicode_string_list",
            &string_tag(&[("saved", Some("Saved change"))]),
        );
        assert_eq!(panel.resolve(false).entries[0].id, "old");
        assert_eq!(panel.resolve(true).entries[0].id, "saved");
        write(
            &panel.kit,
            "ui/other.multilingual_unicode_string_list",
            &string_tag(&[("other", None)]),
        );
        let mut hud = native("hud_globals");
        set_ref(&mut hud, "new globals/hud text", b"unic", "ui\\other");
        write(&panel.kit, "ui/custom.hud_globals", &hud);
        assert_eq!(
            panel.resolve(false).source.as_ref().unwrap().path,
            "ui\\messages"
        );
        let list = panel.resolve(true);
        assert_eq!(list.entries[0].id, "other");
        assert_eq!(list.source.as_ref().unwrap().path, "ui\\other");
        // An unsaved version of that source is intentionally never given to the resolver.
        let unsaved = string_tag(&[("unsaved", Some("Not on disk"))]);
        assert_eq!(
            strings::read_string_list(&unsaved).unwrap().entries[0]
                .id
                .as_deref(),
            Some("unsaved")
        );
        assert_eq!(panel.resolve(true).entries[0].id, "other");
    }

    impl Panel {
        fn collection(game: GameId) -> Self {
            let kit=LooseKit::new("multi-list-hud-picker",game.as_str());
            let new=|group:&str| TagFile::new(locate_definitions_root().join(game.as_str()).join(format!("{group}.json"))).unwrap();
            let add=|tag:&mut TagFile,path:&str| {tag.root_mut().field_path_mut(path).unwrap().as_block_mut().unwrap().add_element();};
            let mut globals=new("globals");add(&mut globals,"interface tags");
            set_ref(&mut globals,"interface tags[0]/multiplayer ui globals",b"wgtz","ui\\multiplayer");
            write(&kit,"globals/globals.globals",&globals);
            let mut ui=new("user_interface_globals_definition");set_ref(&mut ui,"HUD globals",b"uihg","ui\\hud");
            write(&kit,"ui/multiplayer.user_interface_globals_definition",&ui);
            let mut hud=new("user_interface_hud_globals_definition");
            for(index,name,rows)in [
                (0,"ingame",vec![("vehicle_enter","Enter vehicle"),("vehicle_flip","Flip vehicle"),("shared","First shared prompt")]),
                (1,"ingame_interact",vec![("device_use","Use device"),("shared","Second shared prompt")]),
            ]{
                add(&mut hud,"string references");set_ref(&mut hud,&format!("string references[{index}]/string list"),b"unic",&format!("ui\\strings\\{name}"));
                let mut tag=new("multilingual_unicode_string_list");let list=strings::read_string_list(&tag).unwrap();
                let entries=rows.into_iter().map(|(id,text)|{let mut row=list.new_entry();row.id=Some(id.into());row.texts[0]=Some(text.into());row}).collect();
                strings::apply_string_list_edits(&mut tag,Some(game),&[StringListEdit::Insert{at:0,entries}]).unwrap();
                write(&kit,&format!("ui/strings/{name}.multilingual_unicode_string_list"),&tag);
            }
            write(&kit,"ui/hud.user_interface_hud_globals_definition",&hud);
            let tag=new("vehicle");write(&kit,"objects/test.vehicle",&tag);let source=kit.source();
            let entry=source.entry_for_key(&kit.key("objects/test.vehicle")).unwrap().clone();
            Self{kit,source,tag,entry,game,field:"flip message",ctx:egui::Context::default(),sinks:EditSinks::default(),time:0.0,enabled:true,filter_id:None,field_id:None}
        }
    }
    #[test]
    fn string_references_source_groups_precede_prefix_groups_and_pick_exact_id() {
        for game in [GameId::Halo2Amp,GameId::Halo4] {
            let mut panel=Panel::collection(game);let frame=panel.frame(vec![]);panel.click(frame.at("HUD Messages"));
            panel.frame(vec![]);let frame=panel.frame(vec![]);
            assert!(frame.has("ingame (3)"));assert!(frame.has("ingame_interact (2)"));assert!(!frame.has("vehicle (2)"));
            panel.click(frame.at("ingame (3)"));let frame=panel.frame(vec![]);
            assert!(frame.has("vehicle (2)"));assert!(!frame.has("vehicle_flip"));
            panel.click(frame.at("vehicle (2)"));let frame=panel.frame(vec![]);
            assert!(frame.has("vehicle_flip"));
            let frame=panel.click(frame.at("vehicle_flip"));
            assert!(frame.pending.iter().any(|edit|edit.path=="flip message"&&edit.input=="vehicle_flip"));
        }
    }
    #[test]
    fn string_references_searches_all_sources_and_retains_duplicate_previews() {
        let mut panel=Panel::collection(GameId::Halo2Amp);let list=panel.resolve(true);
        let preview=value_preview(&list,"shared");assert!(preview.contains("First shared prompt"));assert!(preview.contains("Second shared prompt"));
        let frame=panel.frame(vec![]);panel.click(frame.at("HUD Messages"));panel.frame(vec![]);
        panel.ctx.memory_mut(|memory|memory.request_focus(panel.filter_id.unwrap()));
        let frame=panel.frame(vec![egui::Event::Text("shared prompt".into())]);
        assert!(frame.has("ingame (1)"));assert!(frame.has("ingame_interact (1)"));
        let shared:Vec<_>=frame.texts.iter().filter(|(text,_)|text=="shared").collect();assert_eq!(shared.len(),2);
        assert!(!frame.has("device_use"));let frame=panel.click(shared[1].1.center());
        assert!(frame.pending.iter().any(|edit|edit.input=="shared"));
    }
    #[test]
    fn string_references_multi_source_open_menu_navigates_to_selected_list() {
        let mut panel=Panel::collection(GameId::Halo2Amp);panel.enabled=false;
        let frame=panel.frame(vec![]);panel.click(frame.at("Open"));let frame=panel.frame(vec![]);
        let frame=panel.click(frame.at("ingame_interact"));
        assert_eq!(frame.open.unwrap().rel_path,"ui\\strings\\ingame_interact");
        assert!(frame.pending.is_empty());
    }

    #[test]
    fn string_references_h2a_and_h4_verified_fields_share_the_multi_list_lookup() {
        let cases=[("biped","assassination chud text"),("vehicle","flip message"),
            ("device_control","action string"),("device_control","action denied string"),("device_control","MP team use denied string"),
            ("vehicle","unit/seats[0]/enter seat string"),("vehicle","unit/seats[0]/boarding grenade string"),
            ("vehicle","unit/seats[0]/boarding melee string"),("vehicle","unit/seats[0]/in-seat string"),("vehicle","unit/seats[0]/Seat Selection String"),
            ("weapon","item/pickup message"),("weapon","item/swap message"),("weapon","item/pickup message (dual)"),("weapon","item/swap message (dual)"),
            ("weapon","item/switch-to msg"),("weapon","item/switch-to from ai msg"),("weapon","item/notify empty msg")];
        for game in [GameId::Halo2Amp,GameId::Halo4]{
            let mut panel=Panel::collection(game);let list=panel.resolve(true);
            assert_eq!(list.validate("vehicle_flip"),StringReferenceValidation::Valid);
            assert_eq!(list.validate("device_use"),StringReferenceValidation::Valid);
            let editors=StringEditors::load(locate_definitions_root().join(game.as_str())).unwrap();
            for(group,path)in cases{
                let mut tag=TagFile::new(locate_definitions_root().join(game.as_str()).join(format!("{group}.json"))).unwrap();
                if path.contains("/seats["){tag.root_mut().field_path_mut("unit/seats").unwrap().as_block_mut().unwrap().add_element();}
                assert_eq!(editors.editor_at(tag.root(),path),Some("hud_message"),"{game:?}/{group}/{path}");
            }
            // Do not turn unrelated item/icon or terminal-page strings into HUD IDs.
            let weapon=TagFile::new(locate_definitions_root().join(game.as_str()).join("weapon.json")).unwrap();
            assert_ne!(editors.editor_at(weapon.root(),"item/picked up msg"),Some("hud_message"));
        }
    }
}
