//! Tag-reference rows, validation, and path selection.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;
use std::borrow::Cow;

pub(in crate::app) fn tag_reference_catalog_for_source(
    source: &LoadedSourceData,
    expert_mode: bool,
) -> Option<TagReferenceCatalog<'_>> {
    matches!(&source.source, TagSource::IoStoreContainerSet { .. }).then_some(TagReferenceCatalog {
        entries: &source.entries,
        group_tree: &source.group_tree,
        expert_mode,
    })
}

/// Whether the picker offers a tag of `candidate_group`: one of
/// `allowed_groups` (already expanded to their descendants), or any group
/// when the schema allows none, as the tool takes any for such a field.
pub(super) fn tag_reference_picker_group_allowed(
    allowed_groups: &[u32],
    candidate_group: u32,
    expert_mode: bool,
) -> bool {
    expert_mode || allowed_groups.is_empty() || allowed_groups.contains(&candidate_group)
}

pub(super) fn tag_reference_catalog_entry_matches(entry: &TagEntry, filter: &str) -> bool {
    entry_matches(entry, filter)
}

pub(in crate::app) fn draw_tag_reference_catalog_picker_contents(
    ui: &mut Ui,
    picker_id: egui::Id,
    catalog: TagReferenceCatalog<'_>,
    allowed_groups: &[u32],
    filter: &mut String,
) -> Option<String> {
    let search = ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text(placeholder_text("Search tag names or groups"))
            .desired_width(f32::INFINITY),
    );
    search.on_hover_text(
        "Searches tag filenames, friendly group names, and four-character group codes",
    );
    if catalog.expert_mode {
        ui.label(
            RichText::new("Expert mode: showing all tag groups")
                .color(Color32::from_rgb(214, 166, 64))
                .small(),
        );
    }
    ui.separator();

    let query = filter.trim();
    let mut picked = None;
    let mut visible_entries = 0usize;
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for group_node in &catalog.group_tree.children {
                let matching = group_node
                    .entries
                    .iter()
                    .copied()
                    .filter(|&index| {
                        catalog.entries.get(index).is_some_and(|entry| {
                            tag_reference_picker_group_allowed(
                                allowed_groups,
                                entry.group_tag,
                                catalog.expert_mode,
                            ) && tag_reference_catalog_entry_matches(entry, query)
                        })
                    })
                    .collect::<Vec<_>>();
                if matching.is_empty() {
                    continue;
                }
                visible_entries += matching.len();
                egui::CollapsingHeader::new(format!("{} ({})", group_node.label, matching.len()))
                    .id_salt(picker_id.with(("group", &group_node.label)))
                    .open((!query.is_empty()).then_some(true))
                    .show(ui, |ui| {
                        for index in matching {
                            let Some(entry) = catalog.entries.get(index) else {
                                continue;
                            };
                            if ui
                                .selectable_label(false, &entry.display_path)
                                .on_hover_text(&entry.display_path)
                                .clicked()
                            {
                                picked = Some(entry_reference_input(entry));
                            }
                        }
                    });
            }
            if visible_entries == 0 {
                ui.label(
                    RichText::new("No compatible tags match this search")
                        .color(subtle_dark())
                        .small(),
                );
            }
        });
    picked
}

/// [`reference_target_missing`], answered from egui memory for a couple of
/// seconds at a time.
///
/// Reference rows are redrawn every frame, and each one checked its target
/// with a filesystem stat every time: dozens of stats a frame on a tag full
/// of references, and a stall per frame on a slow or network drive. A file
/// created or deleted outside shows up within the recheck interval.
pub(in crate::app) fn reference_target_missing_cached(
    ui: &Ui,
    names: Option<&TagNameIndex>,
    tags_root: Option<&Path>,
    group_tag: u32,
    rel_path: &str,
) -> bool {
    let Some(root) = tags_root else {
        return false;
    };
    crate::app::shell::frame::recheck_cached(
        ui.ctx(),
        ("reference_target_missing", root, group_tag, rel_path),
        || reference_target_missing(names, tags_root, group_tag, rel_path),
    )
}

pub(in crate::app) fn reference_target_missing(
    names: Option<&TagNameIndex>,
    tags_root: Option<&Path>,
    group_tag: u32,
    rel_path: &str,
) -> bool {
    let Some(root) = tags_root else {
        return false;
    };
    let Some(ext) = names
        .and_then(|names| names.name_for(group_tag))
        .or_else(|| blam_tags::paths::group_tag_to_extension(group_tag))
    else {
        return false;
    };
    let mut rel = rel_path.replace('/', "\\");
    if !ext.is_empty() {
        if let Some(stripped) = rel
            .strip_suffix(&format!(".{ext}"))
            .or_else(|| rel.strip_suffix(&format!(".{}", ext.to_ascii_uppercase())))
        {
            rel = stripped.to_owned();
        }
    }
    if rel.trim().is_empty() {
        return false;
    }
    !blam_tags::paths::resolve_tag_path(root, &rel, ext).exists()
}

/// Resolve the current contents of a tag-reference input to the source entry
/// understood by the shared asynchronous thumbnail service. This deliberately
/// accepts the live draft rather than only the committed tag value, so a pasted
/// or typed bitmap reference begins previewing as soon as it becomes valid.
pub(super) fn bitmap_reference_hover_entry(
    entries: Option<&[TagEntry]>,
    tags_root: Option<&Path>,
    names: Option<&TagNameIndex>,
    input: &str,
    committed: Option<&(u32, String)>,
) -> Option<TagEntry> {
    let (group_tag, path) = parse_tag_reference(input)
        .ok()
        .and_then(|reference| reference.group_tag_and_name)
        .or_else(|| committed.cloned())?;
    if group_tag != u32::from_be_bytes(*b"bitm") {
        return None;
    }
    let mut rel_path = sanitize_ref_path(&path).replace('/', "\\");
    if rel_path.to_ascii_lowercase().ends_with(".bitmap") {
        rel_path.truncate(rel_path.len() - ".bitmap".len());
    }
    if rel_path.is_empty() {
        return None;
    }

    if let Some(entry) = entries.and_then(|entries| {
        entries.iter().find(|entry| {
            entry.group_tag == group_tag
                && entry_rel_path(entry)
                    .replace('/', "\\")
                    .eq_ignore_ascii_case(&rel_path)
        })
    }) {
        return Some(entry.clone());
    }

    let root = tags_root?;
    let path = blam_tags::paths::resolve_tag_path(root, &rel_path, "bitmap");
    Some(TagEntry {
        key: file_entry_key(&path),
        display_path: format!("{}.bitmap", rel_path.replace('\\', "/")),
        group_tag,
        group_name: names
            .and_then(|names| names.name_for(group_tag))
            .map(str::to_owned),
        location: TagEntryLocation::LooseFile(path),
    })
}

pub(in crate::app) fn draw_foundation_tag_reference_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    target: Option<(u32, String)>,
    // `Some(verb)` for references to a geometry tag (render/collision/physics
    // model or animation graph): shows an Import button that runs `tool <verb>`.
    import_verb: Option<&'static str>,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
    value_width: f32,
) {
    let suffix = "tag reference";
    let indent = depth as f32 * 12.0;
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let id = edit.widget_id(("tag_ref", &buffer_key));
    let droppable = edit.can_edit(meta);
    let draft = edit.buffers.draft_mut(&buffer_key, value);

    let hierarchy = group_hierarchy(edit.definitions_root, edit.game);
    let accepted = tag_reference_accepted_groups(meta, &hierarchy);
    let row_response = ui
        .horizontal(|ui| {
            ui.add_space(indent);
            foundation_label_cell(ui, &meta.label, meta.help.as_deref());
            let editable = droppable;
            let has_ref = target.is_some();
            let icon_group =
                tag_reference_value_icon_group(meta, target.as_ref(), &draft.text, edit.game);
            // A non-empty reference whose target file is absent on disk.
            let missing = target.as_ref().is_some_and(|(group, rel)| {
                reference_target_missing_cached(ui, edit.names, edit.tags_root, *group, rel)
            });
            let is_bitmap_reference = icon_group == Some(u32::from_be_bytes(*b"bitm"));
            let icon = tag_icon(icon_group, edit.game);
            let value_response = if editable {
                let response = foundation_tag_reference_text_edit_cell(
                    ui,
                    &mut draft.text,
                    value_width,
                    id,
                    icon,
                );

                draft.note_response(&response);
                if draft.should_commit(ui, &response) {
                    let input = draft.text.trim().to_owned();
                    commit_tag_reference_input(
                        edit.pending,
                        edit.status.as_deref_mut(),
                        path,
                        input,
                        accepted.as_deref(),
                        edit.names,
                        edit.game,
                    );
                }
                draft.keep_commit(|| {
                    let (path, accepted, game) = (path.to_owned(), accepted.clone(), edit.game);
                    DraftCommit::new(edit.tag_key, vec![buffer_key.clone()], move |texts| {
                        tag_reference_input_ops(&path, texts[0], accepted.as_deref(), None, game)
                    })
                });
                response
            } else if !has_ref {
                foundation_tag_reference_input_cell_colored(
                    ui,
                    "(no reference)",
                    value_width,
                    subtle_dark(),
                    Some("This reference is empty"),
                    icon,
                    true,
                )
            } else if missing {
                foundation_tag_reference_input_cell_colored(
                    ui,
                    value,
                    value_width,
                    REFERENCE_MISSING_COLOR,
                    Some("Referenced tag not found on disk"),
                    icon,
                    true,
                )
            } else {
                foundation_tag_reference_input_cell_colored(
                    ui,
                    value,
                    value_width,
                    text_dark(),
                    None,
                    icon,
                    !is_bitmap_reference,
                )
            };
            // Resolve only the field under the pointer. Looking every bitmap
            // reference up in a large source index on every frame would turn a
            // purely visual affordance into editor-wide work.
            if value_response.hovered()
                && is_bitmap_reference
                && let Some(entry) = bitmap_reference_hover_entry(
                    edit.bitmap_hover_entries,
                    edit.tags_root,
                    edit.names,
                    &draft.text,
                    if draft.changed { None } else { target.as_ref() },
                )
                && let Some(Some(texture)) = bitmap_hover_texture(ui, &entry)
            {
                paint_bitmap_hover_preview(ui, &value_response, &texture, &entry.display_path);
            }
            // Flag a broken reference even while the field is being edited.
            if missing {
                ui.label(
                    RichText::new("⚠ missing")
                        .color(REFERENCE_MISSING_COLOR)
                        .small(),
                )
                .on_hover_text("Referenced tag not found on disk");
            }
            let browse_clicked = if let Some(catalog) = edit.tag_reference_catalog {
                if editable && !catalog.entries.is_empty() {
                    if foundation_header_button_clicked(ui, "...", true) {
                        *edit.tag_reference_picker = Some(TagReferencePickerState {
                            tag_key: edit.tag_key.to_owned(),
                            field_path: path.to_owned(),
                            allowed_groups: hierarchy.expand(&meta.tag_reference_allowed),
                            search: String::new(),
                        });
                    }
                } else {
                    let _ = foundation_header_button_clicked(ui, "...", false);
                }
                false
            } else {
                foundation_header_button_clicked(ui, "...", editable && edit.tags_root.is_some())
            };
            // Open: load the referenced tag through the active source (loose
            // folder or mounted Campaign Evolved catalog).
            if foundation_header_button_clicked(ui, "Open", target.is_some()) {
                if let Some((group_tag, rel_path)) = target.clone() {
                    // Alt-click opens the referenced tag in a floating window.

                    let float = ui.input(|i| i.modifiers.alt);
                    *edit.open_request = Some(OpenTagRequest {
                        group_tag,
                        rel_path,
                        float,
                    });
                }
            }
            // Import: only for geometry references (render/collision/physics model,
            // animation graph). Runs the matching `tool` command in the background.
            if let (Some(verb), Some((_, rel_path))) = (import_verb, target.as_ref()) {
                if foundation_header_button_clicked(ui, "Import", edit.tags_root.is_some()) {
                    *edit.tool_import = Some(ToolImportRequest {
                        verb,
                        source_dir: model_source_dir(rel_path),
                    });
                }
            } else {
                let _ = foundation_header_button_clicked(ui, "Import", false);
            }
            if browse_clicked {
                if let Some(tags_root) = edit.tags_root {
                    let start_ref = target.as_ref().map(|(_, rel_path)| rel_path.as_str());
                    match choose_tag_reference_input(
                        tags_root,
                        start_ref,
                        accepted.as_deref(),
                        edit.names,
                    ) {
                        Ok(Some(input)) => {
                            draft.set_clean(input.clone());
                            edit.pending.push(PendingFieldEdit {
                                path: path.to_owned(),
                                input,
                            });
                        }
                        Ok(None) => {}
                        Err(error) => {
                            if let Some(status) = edit.status.as_deref_mut() {
                                *status = error;
                            }
                        }
                    }
                }
            }
            let draft_text = draft.text.trim();
            let can_clear = has_ref
                || (!draft_text.is_empty() && !draft_text.eq_ignore_ascii_case("none"));
            if foundation_header_button_clicked(ui, "Clear", editable && can_clear) {
                draft.set_clean("");
                // Clicking Clear may blur the text edit and queue its draft
                // first. Clear supersedes that edit; an already-empty stored
                // reference needs no mutation when only a draft was discarded.
                edit.pending.retain(|pending| pending.path != path);
                if has_ref {
                    edit.pending.push(PendingFieldEdit {
                        path: path.to_owned(),
                        input: "NONE".to_owned(),
                    });
                }
            }
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        })
        .response;

    // Drag-and-drop: drop a tag from the browser onto this row to set the
    // reference. Accept only when the field is editable and the dropped group
    // is one the reference takes.
    if droppable {
        let accepts = |payload: &DraggedTagRef| {
            accepted
                .as_ref()
                .is_none_or(|accepted| accepted.contains(&payload.group_tag))
        };
        if let Some(payload) = row_response.dnd_hover_payload::<DraggedTagRef>() {
            let color = if accepts(&payload) {
                Color32::from_rgb(120, 170, 90)
            } else {
                REFERENCE_MISSING_COLOR
            };
            ui.painter()
                .rect_stroke(
                    row_response.rect,
                    3.0,
                    Stroke::new(1.5_f32, color),
                    egui::StrokeKind::Middle,
                );
        }
        if let Some(payload) = row_response.dnd_release_payload::<DraggedTagRef>() {
            if accepts(&payload) {
                draft.set_clean(payload.input.clone());
                edit.pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input: payload.input.clone(),
                });
            }
        }
    }
}

/// The groups a reference takes: those its schema allows, with every group
/// descended from them (an `object` field takes bipeds, weapons, scenery …),
/// or — when the schema names none — the group it already points at. `None`
/// when nothing narrows it.
/// The groups a reference takes: those its schema allows and every group
/// descended from them, or `None` — any group — when the schema lists none,
/// as the tool does for such a field.
pub(super) fn tag_reference_accepted_groups(
    meta: &FieldDisplayMeta,
    hierarchy: &GroupHierarchy,
) -> Option<Vec<u32>> {
    (!meta.tag_reference_allowed.is_empty()).then(|| hierarchy.expand(&meta.tag_reference_allowed))
}

/// A group's file extension, or its four-character code.
fn group_extension(group: u32, names: Option<&TagNameIndex>) -> String {
    names
        .and_then(|names| names.name_for(group))
        .or_else(|| blam_tags::paths::group_tag_to_extension(group))
        .map(str::to_owned)
        .unwrap_or_else(|| format_group_tag(group))
}

/// How a message names the groups a reference takes: `object (biped,
/// crate, …)`, or just `bitmap`.
fn accepted_groups_label(accepted: &[u32], names: Option<&TagNameIndex>) -> String {
    let Some((&first, rest)) = accepted.split_first() else {
        return "any".to_owned();
    };
    let first = group_extension(first, names);
    if rest.is_empty() {
        return first;
    }
    let shown: Vec<String> = rest
        .iter()
        .take(6)
        .map(|&group| group_extension(group, names))
        .collect();
    let more = if rest.len() > shown.len() {
        ", \u{2026}"
    } else {
        ""
    };
    format!("{first} ({}{more})", shown.join(", "))
}

pub(super) fn commit_tag_reference_input(
    pending: &mut Vec<PendingFieldEdit>,
    status: Option<&mut String>,
    path: &str,
    input: String,
    accepted: Option<&[u32]>,
    names: Option<&TagNameIndex>,
    game: Option<GameId>,
) {
    match tag_reference_input_ops(path, &input, accepted, names, game) {
        Ok(ops) => pending.extend(ops.pending),
        Err(error) => {
            if let Some(status) = status {
                *status = error;
            }
        }
    }
}

/// The edit setting the reference at `path` to `input`, or why it is
/// refused: a group the field doesn't take, or text that isn't a reference.
pub(super) fn tag_reference_input_ops(
    path: &str,
    input: &str,
    accepted: Option<&[u32]>,
    names: Option<&TagNameIndex>,
    game: Option<GameId>,
) -> Result<DeferredOps, String> {
    let input = tag_reference_input_in_game(input, game);
    let input = input.as_ref();
    if let Some(accepted) = accepted {
        match parse_tag_reference(input) {
            Ok(parsed) if tag_reference_group_allowed(&parsed, accepted) => {}
            Ok(_) => {
                return Err(format!(
                    "Reference must be a {} tag",
                    accepted_groups_label(accepted, names)
                ));
            }
            Err(error) => return Err(format!("Invalid tag reference: {error}")),
        }
    }
    Ok(field_edit_ops(path, input))
}

/// A typed `path.extension` reference spelled out as `GROUP:path`, with the
/// group the tag's game gives that extension. An extension is a group's name,
/// and games name different groups alike: `.shader` is Halo CE's `shdr`,
/// Halo 2's `shad` and Halo 3's `rmsh`, `.model` Halo CE's `mode` and
/// everyone else's `hlmt`. Left as typed when it already names its group, is
/// empty or `none`, or the game has no group by that name.
pub(in crate::app) fn tag_reference_input_in_game(
    input: &str,
    game: Option<GameId>,
) -> Cow<'_, str> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") || trimmed.contains(':') {
        return Cow::Borrowed(input);
    }
    let Some((path, extension)) = trimmed.rsplit_once('.') else {
        return Cow::Borrowed(input);
    };
    match crate::app::help::bundled_group_hierarchy(game).group_named(extension) {
        Some(group_tag) => Cow::Owned(format_tag_reference_input(group_tag, path)),
        None => Cow::Borrowed(input),
    }
}

pub(super) fn tag_reference_value_icon_group(
    meta: &FieldDisplayMeta,
    target: Option<&(u32, String)>,
    input: &str,
    game: Option<GameId>,
) -> Option<u32> {
    if let Ok(parsed) = parse_tag_reference(&tag_reference_input_in_game(input, game))
        && let Some((group, _)) = parsed.group_tag_and_name
    {
        return Some(group);
    }
    if let Some((group, _)) = target {
        return Some(*group);
    }
    match (
        input.trim().is_empty() || input.eq_ignore_ascii_case("none"),
        meta.tag_reference_allowed.as_slice(),
    ) {
        (true, [group]) => Some(*group),
        _ => None,
    }
}

/// Strip the trailing NUL terminator (and surrounding whitespace) from an
/// on-disk tag-reference path so it resolves as a real file path.
pub(in crate::app) fn sanitize_ref_path(path: &str) -> String {
    path.replace('\u{0}', "").trim().to_owned()
}

/// The `tool` verb to (re)import the geometry tag a reference points at, or
/// `None` for any other group. Matched on the resolved group name so it's
/// independent of fourcc byte order.
pub(in crate::app) fn geometry_import_verb(
    names: &TagNameIndex,
    group_tag: u32,
) -> Option<&'static str> {
    // Prefer the loaded name index, but fall back to the library's built-in
    // group→extension table so the button still appears if definitions failed
    // to load for this source.
    let group_name = names
        .name_for(group_tag)
        .or_else(|| blam_tags::paths::group_tag_to_extension(group_tag))?;
    geometry_import_verb_for_group_name(group_name)
}

pub(in crate::app) fn geometry_import_verb_for_group_name(
    group_name: &str,
) -> Option<&'static str> {
    match group_name {
        "render_model" => Some("render"),
        "collision_model" => Some("collision"),
        "physics_model" => Some("physics"),
        "model_animation_graph" => Some("model-animations-uncompressed"),
        _ => None,
    }
}

/// The `tool` source directory for a geometry tag reference: the parent of the
/// tag path. e.g. `objects\characters\masterchief\masterchief` →
/// `objects\characters\masterchief` (the dir `tool render` expects).
pub(in crate::app) fn model_source_dir(rel_path: &str) -> String {
    rel_path
        .rsplit_once('\\')
        .map(|(parent, _)| parent.to_owned())
        .unwrap_or_else(|| rel_path.to_owned())
}

pub(in crate::app) fn tag_reference_start_dir(tags_root: &Path, rel_path: &str) -> PathBuf {
    let cleaned = sanitize_ref_path(rel_path).replace('/', "\\");
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("NONE") {
        return tags_root.to_path_buf();
    }

    let candidate = tags_root.join(PathBuf::from(cleaned));
    candidate
        .parent()
        .filter(|parent| parent.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| tags_root.to_path_buf())
}

pub(in crate::app) fn choose_tag_reference_input(
    tags_root: &Path,
    start_ref: Option<&str>,
    accepted: Option<&[u32]>,
    names: Option<&TagNameIndex>,
) -> Result<Option<String>, String> {
    let start_dir = start_ref
        .map(|rel_path| tag_reference_start_dir(tags_root, rel_path))
        .unwrap_or_else(|| tags_root.to_path_buf());
    let mut dialog = rfd::FileDialog::new()
        .set_title("Select Tag Reference")
        .set_directory(start_dir);
    if let Some((name, extensions)) = tag_reference_dialog_filter(accepted, names) {
        dialog = dialog.add_filter(name, &extensions);
    }
    let picked = dialog.pick_file();
    let Some(picked) = picked else {
        return Ok(None);
    };
    let rel = tag_reference_relative_path(&picked, tags_root)?;
    let extension = rel
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| "Selected tag file has no extension".to_owned())?;
    let group_tag = tag_reference_group_for_extension(extension, accepted, names)?;
    let path = rel.with_extension("").to_string_lossy().into_owned();
    Ok(Some(format_tag_reference_input(group_tag, &path)))
}

/// The browse dialog's filter: named for what the schema allows, over every
/// extension the reference takes — an `object` field lists .biped, .weapon,
/// .scenery and the rest. `None` when nothing narrows it.
pub(super) fn tag_reference_dialog_filter(
    accepted: Option<&[u32]>,
    names: Option<&TagNameIndex>,
) -> Option<(String, Vec<String>)> {
    let accepted = accepted.filter(|accepted| !accepted.is_empty())?;
    let mut extensions: Vec<String> = Vec::new();
    for &group in accepted {
        let extension = group_extension(group, names);
        if !extensions.contains(&extension) {
            extensions.push(extension);
        }
    }
    Some((group_extension(accepted[0], names), extensions))
}

pub(super) fn tag_reference_group_for_extension(
    extension: &str,
    accepted: Option<&[u32]>,
    names: Option<&TagNameIndex>,
) -> Result<u32, String> {
    if let Some(accepted) = accepted.filter(|accepted| !accepted.is_empty()) {
        if let Some(&group) = accepted
            .iter()
            .find(|&&group| group_extension(group, names).eq_ignore_ascii_case(extension))
        {
            return Ok(group);
        }
        return Err(format!(
            "Selected tag must be a {} tag",
            accepted_groups_label(accepted, names)
        ));
    }

    names
        .and_then(|names| names.group_tag_for(extension))
        .or_else(|| extension_to_group_tag(extension))
        .ok_or_else(|| format!("Unknown tag extension: {extension}"))
}

pub(in crate::app) fn format_tag_reference_input(group_tag: u32, path: &str) -> String {
    format!(
        "{}:{}",
        format_group_tag(group_tag),
        path.replace('/', "\\")
    )
}

pub(super) fn tag_reference_group_allowed(reference: &TagReferenceData, accepted: &[u32]) -> bool {
    reference
        .group_tag_and_name
        .as_ref()
        .is_none_or(|(group, _)| accepted.contains(group))
}

pub(in crate::app) fn tag_reference_relative_path(
    picked: &Path,
    tags_root: &Path,
) -> Result<PathBuf, String> {
    picked
        .strip_prefix(tags_root)
        .map(Path::to_path_buf)
        .map_err(|_| "Selected file must be inside the tags folder".to_owned())
}

pub(in crate::app) fn tag_reference_relative_path_with_extension(
    picked: &Path,
    tags_root: &Path,
) -> Result<String, String> {
    let rel = tag_reference_relative_path(picked, tags_root)?;
    if rel.extension().and_then(|ext| ext.to_str()).is_none() {
        return Err("Selected tag file has no extension".to_owned());
    }
    Ok(rel.to_string_lossy().replace('/', "\\"))
}

pub(in crate::app) fn draw_foundation_flags_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    raw: u64,
    flag_names: &[(u32, String)],
    field: TagField<'_>,

    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let options = match field.options() {
        Some(blam_tags::TagOptions::Flags(options)) => options,
        _ => Vec::new(),
    };
    let display_flags = if options.is_empty() {
        flag_names
            .iter()
            .map(|(bit, label)| (*bit, label.clone(), true))
            .collect::<Vec<_>>()
    } else {
        options
            .iter()
            .map(|option| (option.bit, option.name.to_owned(), option.is_set))
            .collect::<Vec<_>>()
    };

    let indent = depth as f32 * 12.0;
    let row_width = ui.available_width().max(620.0);
    let panel_width = (row_width - indent - FOUNDATION_LABEL_WIDTH - 40.0).clamp(360.0, 760.0);
    let flag_row_height = 21.0;
    let panel_height = if display_flags.is_empty() {
        32.0
    } else {
        12.0 + flag_row_height * display_flags.len() as f32 + 24.0
    };
    let total_height = panel_height.max(32.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(row_width, total_height), Sense::hover());
    let painter = ui.painter().clone();

    // The label cell every other row uses (help cue, gutter, hover docs), level
    // with the first flag; the panel starts where other rows' values do.
    let mut label_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(
                rect.left_top() + Vec2::new(indent, 4.0),
                Vec2::new(FOUNDATION_LABEL_WIDTH, 24.0),
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    foundation_label_cell(&mut label_ui, &meta.label, meta.help.as_deref());

    let flags_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(indent + FOUNDATION_LABEL_WIDTH + ui.spacing().item_spacing.x, 0.0),
        Vec2::new(panel_width, panel_height),
    );
    painter.rect_filled(flags_rect, 0.0, foundation_input());
    painter.rect_stroke(
        flags_rect,
        0.0,
        Stroke::new(1.0_f32, foundation_input_edge()),
        egui::StrokeKind::Middle,
    );

    if display_flags.is_empty() {
        paint_findable_text(
            ui,
            flags_rect.left_center() + Vec2::new(8.0, 0.0),
            Align2::LEFT_CENTER,
            &format!("0x{raw:04X} (none set)"),
            FontId::proportional(12.5),
            text_dark(),
            FindTargetKind::Value,
        );
    } else {
        let mut next_mask = raw;
        for (index, (bit, label, is_set)) in display_flags.iter().enumerate() {
            let row_top = flags_rect.top() + 6.0 + index as f32 * flag_row_height;
            let row_rect = egui::Rect::from_min_size(
                egui::pos2(flags_rect.left() + 8.0, row_top),
                Vec2::new(flags_rect.width() - 16.0, flag_row_height),
            );
            let checkbox_rect = egui::Rect::from_min_size(
                row_rect.left_top() + Vec2::new(0.0, 3.0),
                Vec2::splat(13.0),
            );
            let enabled = edit.can_edit(meta);
            let response = ui.interact(
                row_rect,
                ui.make_persistent_id((edit.view_scope, edit.tag_key, path, "flag", *bit)),
                if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            if response.hovered() {
                painter.rect_filled(row_rect, 0.0, foundation_flag_hover());
                response.clone().on_hover_text(label);
            }

            painter.rect_filled(checkbox_rect, 0.0, foundation_checkbox_bg(enabled));
            painter.rect_stroke(
                checkbox_rect,
                0.0,
                Stroke::new(1.0_f32, foundation_input_edge()),
                egui::StrokeKind::Middle,
            );
            if *is_set {
                let stroke = Stroke::new(1.6_f32, text_dark());
                painter.line_segment(
                    [
                        checkbox_rect.left_center() + Vec2::new(3.0, 0.0),
                        checkbox_rect.center() + Vec2::new(-1.0, 3.0),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        checkbox_rect.center() + Vec2::new(-1.0, 3.0),
                        checkbox_rect.right_center() + Vec2::new(-2.0, -4.0),
                    ],
                    stroke,
                );
            }

            paint_findable_text(
                ui,
                row_rect.left_center() + Vec2::new(20.0, 0.0),
                Align2::LEFT_CENTER,
                &truncate_for_cell(label, row_rect.width() - 24.0),
                FontId::proportional(12.5),
                text_dark(),
                FindTargetKind::Value,
            );

            if response.clicked() {
                if let Some(bit_mask) = 1u64.checked_shl(*bit) {
                    if *is_set {
                        next_mask &= !bit_mask;
                    } else {
                        next_mask |= bit_mask;
                    }
                    edit.pending.push(PendingFieldEdit {
                        path: path.to_owned(),
                        input: next_mask.to_string(),
                    });
                }
            }
        }

        paint_findable_text(
            ui,
            flags_rect.left_bottom() + Vec2::new(8.0, -5.0),
            Align2::LEFT_BOTTOM,
            &format!("0x{raw:04X}"),
            FontId::proportional(11.5),
            subtle_dark(),
            FindTargetKind::Value,
        );
    }

    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::editor::{model_source_dir, sanitize_ref_path};

    // Foundation unit tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    /// A read-only flags row leaves the cursor below its panel, so the next
    /// field starts under it. Its `read-only` hint was drawn in a scope at the
    /// panel's top, which pulled the cursor back up and laid the next field
    /// over the flags.
    #[test]
    fn read_only_flags_row_keeps_the_next_field_below_it() {
        let tag = blam_tags::TagFile::new(locate_definitions_root().join("halo3_mcc/damage_effect.json")).unwrap();
        let root = tag.root();
        let field = root
            .fields_all()
            .find(|field| field.value().as_ref().and_then(flag_value_parts).is_some_and(|(_, names)| names.len() >= 2)
                || matches!(field.options(), Some(blam_tags::TagOptions::Flags(options)) if options.len() >= 2))
            .expect("a flags field with several options");
        let (raw, names) = field.value().as_ref().and_then(flag_value_parts).unwrap_or_default();
        let mut meta = field_display_meta(field.name());
        meta.read_only = true;
        let ctx = egui::Context::default();
        let mut moved = 0.0;
        crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            super::with_test_edit_context(|edit| {
                let before = ui.cursor().top();
                draw_foundation_flags_row(ui, &meta, raw, &names, field, 0, "flags", edit);
                moved = ui.cursor().top() - before;
            });
        });
        assert!(moved > 50.0, "the cursor moved {moved}px past a flags panel of several rows");
    }

    /// A flags row lines up with every other row: its label where theirs are
    /// painted, its panel where their values start.
    #[test]
    fn flags_row_lines_up_with_other_rows() {
        let tag = blam_tags::TagFile::new(locate_definitions_root().join("halo3_mcc/damage_effect.json")).unwrap();
        let root = tag.root();
        let field = root
            .fields_all()
            .find(|field| matches!(field.options(), Some(blam_tags::TagOptions::Flags(options)) if options.len() >= 2))
            .expect("a flags field with several options");
        let mut meta = field_display_meta(field.name());
        meta.label = "flagsrow".to_owned();
        meta.read_only = true;
        let ctx = egui::Context::default();
        let mut value_left = 0.0;
        let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                foundation_label_cell(ui, "otherrow", None);
                value_left = ui.allocate_exact_size(Vec2::new(100.0, 24.0), Sense::hover()).0.left();
            });
            super::with_test_edit_context(|edit| {
                draw_foundation_flags_row(ui, &meta, 0, &[], field, 1, "flags", edit);
            });
        });
        let mut text_x = std::collections::HashMap::new();
        let mut panels = Vec::new();
        for clipped in &output.shapes {
            match &clipped.shape {
                egui::Shape::Text(text) => {
                    text_x.insert(text.galley.text().to_owned(), text.pos.x);
                }
                egui::Shape::Rect(rect) if rect.rect.height() > 40.0 && rect.fill == foundation_input() => panels.push(rect.rect.left()),
                _ => {}
            }
        }
        assert_eq!(text_x.get("flagsrow"), text_x.get("otherrow"), "labels: {text_x:?}");
        assert!(text_x.contains_key("flagsrow"));
        assert_eq!(panels.first().copied(), Some(value_left), "panel left vs value left");
    }

    #[test]
    fn ce_collision_geometry_reference_uses_loaded_game_extension() {
        let definitions_root = locate_definitions_root();
        let ce_names = TagNameIndex::load_game(&definitions_root, GameId::HaloCe).unwrap();
        let h3_names = TagNameIndex::load_game(&definitions_root, GameId::Halo3).unwrap();
        let coll = parse_group_tag("coll").unwrap();
        let root = std::env::temp_dir().join(format!(
            "baboon_ce_collision_reference_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("weapons").join("assault rifle")).unwrap();
        let rel = "weapons\\assault rifle\\assault rifle";
        std::fs::write(
            root.join("weapons")
                .join("assault rifle")
                .join("assault rifle.model_collision_geometry"),
            [],
        )
        .unwrap();

        assert!(!reference_target_missing(
            Some(&ce_names),
            Some(&root),
            coll,
            rel
        ));
        assert!(reference_target_missing(
            Some(&h3_names),
            Some(&root),
            coll,
            rel
        ));
        assert!(reference_target_missing(None, Some(&root), coll, rel));
        std::fs::write(
            root.join("weapons")
                .join("assault rifle")
                .join("assault rifle.collision_model"),
            [],
        )
        .unwrap();
        assert!(!reference_target_missing(
            Some(&h3_names),
            Some(&root),
            coll,
            rel
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A reference row's "missing on disk" check is answered from memory for a
    /// second, not by a stat every frame, and still notices a file
    /// that disappears once that interval has passed.
    #[test]
    fn reference_rows_recheck_their_target_every_second() {
        let root = std::env::temp_dir().join(format!(
            "baboon-ref-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("objects")).unwrap();
        let file = root.join("objects/crate.bitmap");
        std::fs::write(&file, []).unwrap();
        let bitmap = u32::from_be_bytes(*b"bitm");
        let ctx = egui::Context::default();
        let check_at = |time: f64| {
            let mut missing = None;
            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        missing = Some(reference_target_missing_cached(
                            ui,
                            None,
                            Some(&root),
                            bitmap,
                            "objects\\crate",
                        ));
                    });
                },
            );
            missing.unwrap()
        };

        assert!(!check_at(10.0));
        std::fs::remove_file(&file).unwrap();
        let within = check_at(10.5);
        let after = check_at(11.5);

        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            !within,
            "within the interval the remembered answer stands (no stat)"
        );
        assert!(after, "after it, the missing file is noticed");
    }

    /// Clear queues an edit only when a reference is stored; a draft alone is
    /// discarded, and nothing is written for an empty reference.
    #[test]
    fn reference_clear_only_queues_an_edit_for_a_stored_reference() {
        for (value, draft_text, has_reference) in [
            ("", None, false),
            ("NONE", None, false),
            ("  none  ", None, false),
            ("NONE", Some("mode:objects/draft"), false),
            ("mode:objects/test", None, true),
        ] {
            let ctx = egui::Context::default();
            ctx.set_fonts(foundation_fonts());
            ctx.set_global_style(foundation_style());
            with_test_edit_context(|edit| {
                let path = "model";
                if let Some(text) = draft_text {
                    let draft = edit.buffers.draft_mut(&format!("{}|{path}", edit.tag_key), value);
                    draft.text = text.to_owned();
                    draft.changed = true;
                }
                let frame = |events, edit: &mut FieldEditContext<'_>| {
                    crate::app::run_ui_test(&ctx, egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 200.0))),
                        events, ..Default::default()
                    }, |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            draw_foundation_tag_reference_row(
                                ui, &field_display_meta(path), value,
                                has_reference.then(|| (u32::from_be_bytes(*b"mode"), "objects/test".to_owned())),
                                None, 0, path, edit, 300.0,
                            );
                        });
                    })
                };
                let output = frame(Vec::new(), edit);
                let suffix = output.shapes.iter().find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(shape) if shape.galley.text() == "tag reference" => Some(shape),
                    _ => None,
                }).expect("reference row suffix");
                // Clear is the square button immediately before the suffix.
                let pos = egui::pos2(
                    suffix.pos.x - ctx.global_style().spacing.item_spacing.x - ICON_BUTTON_SIZE.x * 0.5,
                    suffix.pos.y + suffix.galley.size().y * 0.5,
                );
                let pointer = |pressed| egui::Event::PointerButton {
                    pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default(),
                };
                frame(vec![egui::Event::PointerMoved(pos), pointer(true)], edit);
                frame(vec![pointer(false)], edit);
                frame(Vec::new(), edit);
                assert_eq!(edit.pending.len(), usize::from(has_reference), "value {value:?}, draft {draft_text:?}");
                if has_reference {
                    assert_eq!(edit.pending[0].input, "NONE");
                }
                if draft_text.is_some() {
                    let draft = edit.buffers.draft_mut(&format!("{}|{path}", edit.tag_key), value);
                    assert!(!draft.changed);
                    assert!(draft.text.is_empty() || draft.text.eq_ignore_ascii_case("none"));
                }
            });
        }
    }

    #[test]
    fn tag_reference_picker_paths_must_be_under_tags_root() {
        let tags_root = PathBuf::from("tags");
        let picked = tags_root
            .join("objects")
            .join("characters")
            .join("brute")
            .join("bitmaps")
            .join("mask.bitmap");

        assert_eq!(
            tag_reference_relative_path_with_extension(&picked, &tags_root).unwrap(),
            r"objects\characters\brute\bitmaps\mask.bitmap"
        );

        let outside = PathBuf::from("data")
            .join("objects")
            .join("characters")
            .join("brute")
            .join("bitmaps")
            .join("mask.tif");
        assert_eq!(
            tag_reference_relative_path_with_extension(&outside, &tags_root).unwrap_err(),
            "Selected file must be inside the tags folder"
        );
    }

    #[test]
    fn bitmap_hover_resolves_a_loaded_entry_from_reference_text() {
        let bitmap_group = parse_group_tag("bitm").unwrap();
        let entry = TagEntry {
            key: "bitmap-key".to_owned(),
            display_path: "ui/hud/scope.bitmap".to_owned(),
            group_tag: bitmap_group,
            group_name: Some("bitmap".to_owned()),
            location: TagEntryLocation::LooseFile(PathBuf::from("tags/ui/hud/scope.bitmap")),
        };

        let resolved = bitmap_reference_hover_entry(
            Some(std::slice::from_ref(&entry)),
            None,
            None,
            r"ui\hud\scope.bitmap",
            None,
        )
        .unwrap();

        assert_eq!(resolved.key, entry.key);
    }

    #[test]
    fn bitmap_hover_synthesizes_an_unvisited_loose_entry() {
        let root = PathBuf::from("tags");
        let resolved =
            bitmap_reference_hover_entry(None, Some(&root), None, r"ui\hud\scope.bitmap", None)
                .unwrap();

        assert_eq!(resolved.display_path, "ui/hud/scope.bitmap");
        assert!(matches!(
            resolved.location,
            TagEntryLocation::LooseFile(path) if path == root.join("ui").join("hud").join("scope.bitmap")
        ));
    }

    #[test]
    fn non_bitmap_references_do_not_request_bitmap_hovers() {
        assert!(
            bitmap_reference_hover_entry(
                None,
                Some(Path::new("tags")),
                None,
                r"ui\hud\scope.shader",
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn tag_reference_group_validator_allows_none_and_matching_group() {
        let render_model = parse_group_tag("mode").unwrap();
        let collision_model = parse_group_tag("coll").unwrap();
        let empty = TagReferenceData {
            group_tag_and_name: None,
        };
        let matching = TagReferenceData {
            group_tag_and_name: Some((render_model, r"objects\foo\foo".to_owned())),
        };
        let mismatched = TagReferenceData {
            group_tag_and_name: Some((collision_model, r"objects\foo\foo".to_owned())),
        };

        assert!(tag_reference_group_allowed(&empty, &[render_model]));
        assert!(tag_reference_group_allowed(&matching, &[render_model]));
        assert!(!tag_reference_group_allowed(&mismatched, &[render_model]));
    }

    #[test]
    fn empty_schema_constrained_reference_keeps_its_required_group() {
        let structure_design = parse_group_tag("sddt").unwrap();
        let meta = FieldDisplayMeta {
            label: "structure design".to_owned(),
            unit: None,
            range: None,
            help: None,
            tag_reference_allowed: vec![structure_design],
            read_only: false,
            advanced: false,
            slider: None,
        };

        assert_eq!(
            tag_reference_accepted_groups(&meta, &GroupHierarchy::default()),
            Some(vec![structure_design])
        );
    }

    /// The schema decides what a reference takes: an `object` field takes
    /// every object type, and a field whose schema lists no group takes any,
    /// as in the tool. The group it points at now has no say: it once
    /// narrowed a list-less Halo CE shader reference to its current type.
    #[test]
    fn the_schema_decides_the_accepted_groups() {
        let hierarchy = group_hierarchy(Some(&locate_definitions_root()), Some(GameId::HaloReach));
        let object = parse_group_tag("obje").unwrap();
        let meta = |allowed| FieldDisplayMeta {
            label: "object".to_owned(),
            unit: None,
            range: None,
            help: None,
            tag_reference_allowed: allowed,
            read_only: false,
            advanced: false,
            slider: None,
        };
        let accepted = tag_reference_accepted_groups(&meta(vec![object]), &hierarchy).unwrap();
        for group in ["scen", "weap"] {
            assert!(accepted.contains(&parse_group_tag(group).unwrap()), "{group}");
        }
        assert_eq!(tag_reference_accepted_groups(&meta(Vec::new()), &hierarchy), None);
    }

    /// A Halo CE model's shader reference allows `shader`, as tool.exe
    /// declares it, and so takes every shader type: changing a shader_model
    /// to a shader_environment needs no clearing first. The definitions
    /// carried no groups for any Halo CE or Halo 2 reference before.
    #[test]
    fn a_halo_ce_model_shader_takes_every_shader_type() {
        let definitions_root = locate_definitions_root();
        let hierarchy = group_hierarchy(Some(&definitions_root), Some(GameId::HaloCe));
        let docs =
            crate::app::help::field_docs::build_def_docs(&definitions_root, GameId::HaloCe, "gbxmodel");
        let allowed: Vec<u32> = docs
            .all_entries()
            .find_map(|entry| match entry {
                DefEntry::Field {
                    clean_name,
                    tag_reference_allowed,
                    ..
                } if clean_name == "shader" => Some(tag_reference_allowed.clone()),
                _ => None,
            })
            .expect("no shader field in gbxmodel");
        assert_eq!(allowed, [parse_group_tag("shdr").unwrap()]);
        let meta = FieldDisplayMeta {
            label: "shader".to_owned(),
            unit: None,
            range: None,
            help: None,
            tag_reference_allowed: allowed,
            read_only: false,
            advanced: false,
            slider: None,
        };
        let accepted = tag_reference_accepted_groups(&meta, &hierarchy).unwrap();
        for group in ["shdr", "soso", "senv", "schi", "swat"] {
            assert!(accepted.contains(&parse_group_tag(group).unwrap()), "{group}");
        }
        assert!(!accepted.contains(&parse_group_tag("bitm").unwrap()));
    }

    /// Issue #46: Reach's multiplayer object type list `object` field allows
    /// `object`, and must take every object type — a `.weapon` picked in the
    /// browse dialog, a typed `weap:` reference — while still refusing what
    /// is not an object.
    #[test]
    fn an_object_reference_takes_every_object_type() {
        let definitions_root = locate_definitions_root();
        let names = TagNameIndex::load_game(&definitions_root, GameId::HaloReach).unwrap();
        let hierarchy = group_hierarchy(Some(&definitions_root), Some(GameId::HaloReach));
        let docs = crate::app::help::field_docs::build_def_docs(
            &definitions_root,
            GameId::HaloReach,
            "multiplayer_object_type_list",
        );
        let allowed: Vec<u32> = docs
            .all_entries()
            .find_map(|entry| match entry {
                DefEntry::Field {
                    clean_name,
                    tag_reference_allowed,
                    ..
                } if clean_name == "object" => Some(tag_reference_allowed.clone()),
                _ => None,
            })
            .expect("no object field in the multiplayer object type list");
        assert_eq!(allowed, [parse_group_tag("obje").unwrap()]);
        let meta = FieldDisplayMeta {
            label: "object".to_owned(),
            unit: None,
            range: None,
            help: None,
            tag_reference_allowed: allowed,
            read_only: false,
            advanced: false,
            slider: None,
        };
        let accepted = tag_reference_accepted_groups(&meta, &hierarchy).unwrap();
        for (extension, group) in [
            ("biped", "bipd"),
            ("weapon", "weap"),
            ("scenery", "scen"),
            ("vehicle", "vehi"),
        ] {
            assert_eq!(
                tag_reference_group_for_extension(extension, Some(&accepted), Some(&names)),
                Ok(parse_group_tag(group).unwrap()),
                "{extension}"
            );
        }
        // The browse dialog filters on all of them, under the schema's name.
        let (filter_name, extensions) =
            tag_reference_dialog_filter(Some(&accepted), Some(&names)).unwrap();
        assert_eq!(filter_name, "object");
        for extension in [
            "biped",
            "weapon",
            "scenery",
            "vehicle",
            "equipment",
            "crate",
        ] {
            assert!(
                extensions.iter().any(|e| e == extension),
                "the dialog hides .{extension}"
            );
        }
        assert!(!extensions.iter().any(|e| e == "bitmap"));
        let refused = tag_reference_group_for_extension("bitmap", Some(&accepted), Some(&names));
        assert!(
            refused
                .as_ref()
                .is_err_and(|message| message.starts_with("Selected tag must be a object (")),
            "{refused:?}"
        );

        let mut pending = Vec::new();
        commit_tag_reference_input(
            &mut pending,
            None,
            "object types[0]/object",
            "weap:objects\\weapons\\rifle\\assault_rifle\\assault_rifle".to_owned(),
            Some(&accepted),
            Some(&names),
            Some(GameId::HaloReach),
        );
        assert_eq!(pending.len(), 1, "a typed weapon reference was refused");
    }

    /// The catalog picker offers what the schema allows, or every group when
    /// it allows none: the group the reference points at now has no say.
    #[test]
    fn catalog_picker_offers_the_schema_groups_or_any() {
        let animation = parse_group_tag("jmad").unwrap();
        let biped = parse_group_tag("bipd").unwrap();
        let vehicle = parse_group_tag("vehi").unwrap();
        let weapon = parse_group_tag("weap").unwrap();

        assert!(tag_reference_picker_group_allowed(&[animation], animation, false));
        assert!(!tag_reference_picker_group_allowed(&[animation], weapon, false));
        assert!(tag_reference_picker_group_allowed(&[biped, vehicle], vehicle, false));
        assert!(!tag_reference_picker_group_allowed(&[biped, vehicle], weapon, false));
        assert!(tag_reference_picker_group_allowed(&[], animation, false));
        assert!(tag_reference_picker_group_allowed(&[], weapon, false));
        assert!(tag_reference_picker_group_allowed(&[animation], weapon, true));
    }

    #[test]
    fn catalog_picker_searches_names_and_groups_not_parent_folders() {
        let model = parse_group_tag("mode").unwrap();
        let weapon = parse_group_tag("weap").unwrap();
        let parent_only = TagEntry {
            key: "ublock:model".to_owned(),
            display_path: "objects/characters/elite/garbage/hg_arm.render_model".to_owned(),
            group_tag: model,
            group_name: Some("render_model".to_owned()),
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: "Tags/objects/characters/elite/garbage/hg_arm-render_model.ubulk"
                    .to_owned(),
            },
        };
        let rifle = TagEntry {
            key: "ublock:weapon".to_owned(),
            display_path: "objects/weapons/rifle/battle_rifle.weapon".to_owned(),
            group_tag: weapon,
            group_name: Some("weapon".to_owned()),
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: "Tags/objects/weapons/rifle/battle_rifle-weapon.ubulk".to_owned(),
            },
        };

        assert!(!tag_reference_catalog_entry_matches(&parent_only, "elite"));
        assert!(tag_reference_catalog_entry_matches(&rifle, "rifle"));
        assert!(tag_reference_catalog_entry_matches(&rifle, "weapon"));
        assert!(tag_reference_catalog_entry_matches(&rifle, "WEAP"));
    }

    #[test]
    fn catalog_picker_is_exposed_only_for_iostore_sources() {
        let container_source = LoadedSourceData {
            label: "Campaign Evolved".to_owned(),
            source: TagSource::IoStoreContainerSet {
                root: PathBuf::from("C:/CampaignEvolved/Content/Paks"),
                containers: Vec::new(),
                index: std::sync::Arc::new(crate::core::source::ContainerTagIndex::default()),
                packages: std::sync::Arc::new(crate::core::source::ContainerPackageIndex::default()),
                shipped: std::sync::Arc::new(crate::core::source::ShippedTagIndex::default()),
            },
            names: TagNameIndex::default(),
            game: Some(GameId::CampaignEvolved),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        };
        let catalog = tag_reference_catalog_for_source(&container_source, true)
            .expect("container source should expose a catalog");
        assert!(catalog.expert_mode);

        let loose_source = LoadedSourceData {
            label: "H3EK".to_owned(),
            source: TagSource::LooseFolder {
                root: PathBuf::from("C:/H3EK/tags"),
                game: Some(GameId::Halo3),
                definitions_root: PathBuf::from("C:/H3EK/definitions"),
            },
            names: TagNameIndex::default(),
            game: Some(GameId::Halo3),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        };
        assert!(tag_reference_catalog_for_source(&loose_source, true).is_none());
    }

    #[test]
    fn picker_resolves_structure_design_from_loaded_game_definitions() {
        let definitions_root = locate_definitions_root();
        for game in ["halo3_mcc", "halo3odst_mcc", "haloreach_mcc", "halo4_mcc"] {
            let names = TagNameIndex::load_game(&definitions_root, GameId::from_id(game).unwrap()).unwrap();
            let structure_design = parse_group_tag("sddt").unwrap();
            assert_eq!(
                tag_reference_group_for_extension(
                    "structure_design",
                    Some(&[structure_design]),
                    Some(&names),
                )
                .unwrap(),
                structure_design,
                "{game}"
            );
        }
    }

    #[test]
    fn tag_reference_value_icon_prefers_typed_or_committed_group() {
        let render_model = parse_group_tag("mode").unwrap();
        let collision_model = parse_group_tag("coll").unwrap();
        let biped = parse_group_tag("bipd").unwrap();
        let vehicle = parse_group_tag("vehi").unwrap();
        let bitmap = parse_group_tag("bitm").unwrap();
        let target = (collision_model, r"objects\foo\foo".to_owned());
        let meta = |allowed| FieldDisplayMeta {
            label: "reference".to_owned(),
            unit: None,
            range: None,
            help: None,
            tag_reference_allowed: allowed,
            read_only: false,
            advanced: false,
            slider: None,
        };

        assert_eq!(
            tag_reference_value_icon_group(
                &meta(vec![render_model]),
                Some(&target),
                r"objects\foo\foo.bitmap",
                Some(GameId::Halo3)
            ),
            Some(bitmap)
        );
        assert_eq!(
            tag_reference_value_icon_group(
                &meta(vec![render_model]),
                Some(&target),
                r"objects\foo\foo",
                Some(GameId::Halo3)
            ),
            Some(collision_model)
        );
        assert_eq!(
            tag_reference_value_icon_group(&meta(vec![render_model]), None, "NONE", None),
            Some(render_model)
        );
        assert_eq!(
            tag_reference_value_icon_group(&meta(vec![biped, vehicle]), None, "NONE", None),
            None
        );
    }

    /// A typed `path.extension` reference names the group the tag's game
    /// gives that extension, not whichever game's was loaded first: `.shader`
    /// is Halo CE's `shdr`, Halo 2's `shad` and Reach's `rmsh`.
    #[test]
    fn a_typed_reference_takes_its_group_from_the_tags_game() {
        let typed = r"levels\a\shaders\floor.shader";
        for (game, group) in [
            (GameId::HaloCe, "shdr"),
            (GameId::Halo2, "shad"),
            (GameId::HaloReach, "rmsh"),
        ] {
            assert_eq!(
                tag_reference_input_in_game(typed, Some(game)),
                format!(r"{group}:levels\a\shaders\floor"),
                "{game:?}"
            );
        }
        assert_eq!(
            tag_reference_input_in_game(r"a\b.model", Some(GameId::HaloCe)),
            r"mode:a\b"
        );
        assert_eq!(
            tag_reference_input_in_game(r"a\b.model", Some(GameId::HaloReach)),
            r"hlmt:a\b"
        );
        // Already naming its group, empty, none, or no such group: as typed.
        for input in [r"weap:a\b", "", "NONE", r"a\b.not_a_group"] {
            assert_eq!(
                tag_reference_input_in_game(input, Some(GameId::HaloCe)),
                input
            );
        }

        // A Halo CE field that takes shaders takes a typed `.shader`.
        let shader = u32::from_be_bytes(*b"shdr");
        let ops =
            tag_reference_input_ops("shader", typed, Some(&[shader]), None, Some(GameId::HaloCe))
                .expect("a typed Halo CE shader reference is a shader");
        assert_eq!(ops.pending.len(), 1);
        assert_eq!(ops.pending[0].input, r"shdr:levels\a\shaders\floor");
    }

    #[test]
    fn tag_ref_path_helpers() {
        // Null terminator stripped so the ref resolves on disk.
        assert_eq!(
            sanitize_ref_path("objects\\characters\\masterchief\\masterchief\u{0}"),
            "objects\\characters\\masterchief\\masterchief"
        );
        // tool source dir is the parent of the tag path.
        assert_eq!(
            model_source_dir("objects\\characters\\masterchief\\masterchief"),
            "objects\\characters\\masterchief"
        );
        assert_eq!(model_source_dir("solo"), "solo");
    }
}
