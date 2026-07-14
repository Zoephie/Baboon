//! Tag-reference rows, validation, and path selection.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

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
    let buffer = edit
        .buffers
        .entry(buffer_key.clone())
        .or_insert_with(|| value.to_owned());
    if !ui.memory(|memory| memory.has_focus(id)) && buffer != value {
        *buffer = value.to_owned();
    }

    let droppable = edit.editable && !meta.read_only;
    let required_group = tag_reference_required_group(meta, target.as_ref());
    let row_response = ui
        .horizontal(|ui| {
            ui.add_space(indent);
            foundation_label_cell(ui, &meta.label, meta.help.as_deref());
            let editable = edit.editable && !meta.read_only;
            let has_ref = target.is_some();
            let icon_group = tag_reference_value_icon_group(meta, target.as_ref(), buffer);
            // A non-empty reference whose target file is absent on disk.
            let missing = target.as_ref().is_some_and(|(group, rel)| {
                reference_target_missing(edit.names, edit.tags_root, *group, rel)
            });
            if editable {
                let response = foundation_tag_reference_text_edit_cell(
                    ui,
                    buffer,
                    value_width,
                    id,
                    icon_group,
                );

                if response.lost_focus() && buffer.trim() != value.trim() {
                    let input = buffer.trim().to_owned();
                    commit_tag_reference_input(
                        edit.pending,
                        edit.status.as_deref_mut(),
                        path,
                        input,
                        required_group,
                    );
                }
            } else if !has_ref {
                foundation_tag_reference_input_cell_colored(
                    ui,
                    "(no reference)",
                    value_width,
                    subtle_dark(),
                    Some("This reference is empty"),
                    icon_group,
                );
            } else if missing {
                foundation_tag_reference_input_cell_colored(
                    ui,
                    value,
                    value_width,
                    REFERENCE_MISSING_COLOR,
                    Some("Referenced tag not found on disk"),
                    icon_group,
                );
            } else {
                foundation_tag_reference_input_cell_colored(
                    ui,
                    value,
                    value_width,
                    text_dark(),
                    None,
                    icon_group,
                );
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
            let browse_clicked =
                foundation_header_button_clicked(ui, "...", editable && edit.tags_root.is_some());
            // Open: load the referenced tag in a new tab (resolved against the
            // loose-folder tags root). Enabled only when the ref is non-empty.
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
                        required_group,
                        edit.names,
                    ) {
                        Ok(Some(input)) => {
                            *buffer = input.clone();
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
            if foundation_header_button_clicked(ui, "Clear", editable) {
                buffer.clear();
                edit.pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input: "NONE".to_owned(),
                });
            }
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
            draw_field_help(ui, meta);
        })
        .response;

    // Drag-and-drop: drop a tag from the browser onto this row to set the
    // reference. Accept only when the field is editable and the dropped group
    // matches either the current target or a single group required by schema.
    if droppable {
        let accepts =
            |payload: &DraggedTagRef| required_group.is_none_or(|group| group == payload.group_tag);
        if let Some(payload) = row_response.dnd_hover_payload::<DraggedTagRef>() {
            let color = if accepts(&payload) {
                Color32::from_rgb(120, 170, 90)
            } else {
                REFERENCE_MISSING_COLOR
            };
            ui.painter()
                .rect_stroke(row_response.rect, 3.0, Stroke::new(1.5, color));
        }
        if let Some(payload) = row_response.dnd_release_payload::<DraggedTagRef>() {
            if accepts(&payload) {
                *buffer = payload.input.clone();
                edit.pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input: payload.input.clone(),
                });
            }
        }
    }
}

pub(super) fn tag_reference_required_group(
    meta: &FieldDisplayMeta,
    target: Option<&(u32, String)>,
) -> Option<u32> {
    target
        .map(|(group, _)| *group)
        .or_else(|| match meta.tag_reference_allowed.as_slice() {
            [group] => Some(*group),
            _ => None,
        })
}

fn commit_tag_reference_input(
    pending: &mut Vec<PendingFieldEdit>,
    mut status: Option<&mut String>,
    path: &str,
    input: String,
    required_group: Option<u32>,
) {
    if let Some(required_group) = required_group {
        match parse_tag_reference(&input) {
            Ok(parsed) if tag_reference_group_allowed(&parsed, required_group) => {
                pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input,
                });
            }
            Ok(_) => {
                if let Some(status) = status.as_deref_mut() {
                    *status = format!(
                        "Reference must be a {} tag",
                        tag_group_display_name(required_group)
                    );
                }
            }
            Err(error) => {
                if let Some(status) = status.as_deref_mut() {
                    *status = format!("Invalid tag reference: {error}");
                }
            }
        }
    } else {
        pending.push(PendingFieldEdit {
            path: path.to_owned(),
            input,
        });
    }
}

pub(super) fn tag_reference_value_icon_group(
    meta: &FieldDisplayMeta,
    target: Option<&(u32, String)>,
    input: &str,
) -> Option<u32> {
    if let Ok(parsed) = parse_tag_reference(input)
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
    required_group: Option<u32>,
    names: Option<&TagNameIndex>,
) -> Result<Option<String>, String> {
    let start_dir = start_ref
        .map(|rel_path| tag_reference_start_dir(tags_root, rel_path))
        .unwrap_or_else(|| tags_root.to_path_buf());
    let mut dialog = rfd::FileDialog::new()
        .set_title("Select Tag Reference")
        .set_directory(start_dir);
    if let Some(group_tag) = required_group
        && let Some(extension) = names
            .and_then(|names| names.name_for(group_tag))
            .or_else(|| blam_tags::paths::group_tag_to_extension(group_tag))
    {
        dialog = dialog.add_filter(extension, &[extension]);
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
    let group_tag = tag_reference_group_for_extension(extension, required_group, names)?;
    let path = rel.with_extension("").to_string_lossy().into_owned();
    Ok(Some(format_tag_reference_input(group_tag, &path)))
}

pub(super) fn tag_reference_group_for_extension(
    extension: &str,
    required_group: Option<u32>,
    names: Option<&TagNameIndex>,
) -> Result<u32, String> {
    if let Some(required_group) = required_group {
        let expected = names
            .and_then(|names| names.name_for(required_group))
            .or_else(|| blam_tags::paths::group_tag_to_extension(required_group));
        if expected.is_some_and(|expected| expected.eq_ignore_ascii_case(extension)) {
            return Ok(required_group);
        }
        if let Some(expected) = expected {
            return Err(format!("Selected tag must be a .{expected} tag"));
        }
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

pub(super) fn tag_reference_group_allowed(
    reference: &TagReferenceData,
    required_group: u32,
) -> bool {
    reference
        .group_tag_and_name
        .as_ref()
        .is_none_or(|(group, _)| *group == required_group)
}

fn tag_group_display_name(group_tag: u32) -> String {
    blam_tags::paths::group_tag_to_extension(group_tag)
        .map(str::to_owned)
        .unwrap_or_else(|| format_group_tag(group_tag))
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

    let label_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(indent + 4.0, 4.0),
        Vec2::new(FOUNDATION_LABEL_WIDTH - 8.0, 24.0),
    );
    painter.text(
        label_rect.left_center(),
        Align2::LEFT_CENTER,
        truncate_for_cell(&meta.label, label_rect.width()),
        FontId::proportional(12.5),
        text_dark(),
    );

    let flags_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(indent + FOUNDATION_LABEL_WIDTH, 0.0),
        Vec2::new(panel_width, panel_height),
    );
    painter.rect_filled(flags_rect, 0.0, foundation_input());
    painter.rect_stroke(flags_rect, 0.0, Stroke::new(1.0, foundation_input_edge()));

    if display_flags.is_empty() {
        painter.text(
            flags_rect.left_center() + Vec2::new(8.0, 0.0),
            Align2::LEFT_CENTER,
            format!("0x{raw:04X} (none set)"),
            FontId::proportional(12.5),
            text_dark(),
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
            let enabled = edit.editable && !meta.read_only;
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
                Stroke::new(1.0, foundation_input_edge()),
            );
            if *is_set {
                let stroke = Stroke::new(1.6, text_dark());
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

            painter.text(
                row_rect.left_center() + Vec2::new(20.0, 0.0),
                Align2::LEFT_CENTER,
                truncate_for_cell(label, row_rect.width() - 24.0),
                FontId::proportional(12.5),
                text_dark(),
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

        painter.text(
            flags_rect.left_bottom() + Vec2::new(8.0, -5.0),
            Align2::LEFT_BOTTOM,
            format!("0x{raw:04X}"),
            FontId::proportional(11.5),
            subtle_dark(),
        );
    }

    if meta.help.is_some() || meta.read_only {
        ui.allocate_new_ui(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                flags_rect.right_top() + Vec2::new(8.0, 0.0),
                Vec2::new(120.0, 24.0),
            )),
            |ui| draw_field_help(ui, meta),
        );
    }
    ui.add_space(4.0);
}
