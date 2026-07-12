//! Scalar, enum, bounds, color, and multi-component value rows.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

pub(in crate::app) fn draw_foundation_value_row(
    ui: &mut Ui,
    field: TagField<'_>,
    meta: &FieldDisplayMeta,
    type_name: &str,
    value: &TagFieldData,
    names: &TagNameIndex,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
    // Resolved (element labels, target block field name) for a block-index field
    // whose target block was found among the struct's siblings; `None` for
    // non-block-index fields and unresolvable (custom) indices → numeric editor.
    block_index: Option<&(Vec<String>, String)>,
    semantic_short_index: Option<&(Vec<String>, String)>,
    tag_reference_value_width: f32,
) {
    if let (Some((labels, target_path)), Some(index)) = (block_index, block_index_value(value)) {
        draw_foundation_block_index_row(ui, meta, index, labels, target_path, depth, path, edit);
        return;
    }
    if let (TagFieldData::ShortInteger(index), Some((labels, target_path))) =
        (value, semantic_short_index)
    {
        draw_foundation_block_index_row(
            ui,
            meta,
            *index as i64,
            labels,
            target_path,
            depth,
            path,
            edit,
        );
        return;
    }
    if let TagFieldData::TagReference(reference) = value {
        let formatted = format_foundation_scalar_value(names, value);
        // The on-disk tag-ref path is null-terminated; strip the trailing NUL
        // so it resolves on disk (and tool-import paths are clean).
        let target = reference
            .group_tag_and_name
            .as_ref()
            .map(|(g, p)| (*g, sanitize_ref_path(p)))
            .filter(|(_, p)| !p.is_empty());
        let import_verb = target
            .as_ref()
            .and_then(|(group, _)| geometry_import_verb(names, *group));
        draw_foundation_tag_reference_row(
            ui,
            meta,
            &formatted,
            target,
            import_verb,
            depth,
            path,
            edit,
            tag_reference_value_width,
        );
        return;
    }

    if let Some((raw, flag_names)) = flag_value_parts(value) {
        draw_foundation_flags_row(ui, meta, raw, &flag_names, field, depth, path, edit);
        return;
    }

    if let Some(blam_tags::TagOptions::Enum {
        names: options,
        current,
    }) = field.options()
    {
        draw_foundation_enum_row(ui, meta, &options, current, depth, path, edit);
        return;
    }

    if matches!(
        value,
        TagFieldData::RealRgbColor(_)
            | TagFieldData::RealArgbColor(_)
            | TagFieldData::RgbColor(_)
            | TagFieldData::ArgbColor(_)
    ) {
        draw_foundation_color_row(ui, meta, value, depth, path, edit);
        return;
    }

    if let Some((lower, upper)) = foundation_bounds_values(value) {
        draw_foundation_bounds_row(
            ui,
            meta,
            &lower,
            &upper,
            field_suffix(meta, type_name).as_str(),
            depth,
            path,
            edit,
        );
        return;
    }

    if let Some(parts) = foundation_editable_component_parts(value) {
        draw_foundation_component_edit_row(
            ui,
            meta,
            &parts,
            field_suffix(meta, type_name).as_str(),
            depth,
            path,
            edit,
        );
        return;
    }

    let formatted = format_foundation_scalar_value(names, value);
    if edit.editable && !meta.read_only && is_text_editable_value(value) {
        draw_foundation_editable_text_row(
            ui,
            meta,
            &formatted,
            field_suffix(meta, type_name).as_str(),
            depth,
            path,
            edit,
        );
        return;
    }

    if let Some(parts) = foundation_value_parts(value) {
        draw_foundation_multi_value_row(
            ui,
            meta,
            &parts,
            field_suffix(meta, type_name).as_str(),
            depth,
        );
        return;
    }

    draw_foundation_meta_text_row(
        ui,
        meta,
        &formatted,
        field_suffix(meta, type_name).as_str(),
        depth,
    );
}

/// A color value row: channel readouts plus a clickable swatch that opens the
/// color picker. ARGB rows show all four components in a/r/g/b order.

pub(in crate::app) fn draw_foundation_color_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &TagFieldData,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    // (alpha, red, green, blue, is_argb). RGB rows pin alpha to 1.0.
    let (a, r, g, b, argb) = match value {
        TagFieldData::RealRgbColor(c) => (1.0, c.red, c.green, c.blue, false),
        TagFieldData::RealArgbColor(c) => (c.alpha, c.red, c.green, c.blue, true),
        TagFieldData::RgbColor(c) => {
            let raw = c.0;
            (
                1.0,
                ((raw >> 16) & 0xFF) as f32 / 255.0,
                ((raw >> 8) & 0xFF) as f32 / 255.0,
                (raw & 0xFF) as f32 / 255.0,
                false,
            )
        }
        TagFieldData::ArgbColor(c) => {
            let raw = c.0;
            (
                ((raw >> 24) & 0xFF) as f32 / 255.0,
                ((raw >> 16) & 0xFF) as f32 / 255.0,
                ((raw >> 8) & 0xFF) as f32 / 255.0,
                (raw & 0xFF) as f32 / 255.0,
                true,
            )
        }
        _ => return,
    };
    let channels: &[(&str, f32)] = if argb {
        &[("a", a), ("r", r), ("g", g), ("b", b)]
    } else {
        &[("r", r), ("g", g), ("b", b)]
    };
    let swatch = Color32::from_rgb(
        float_channel_to_u8(r),
        float_channel_to_u8(g),
        float_channel_to_u8(b),
    );
    let editable = edit.editable && !meta.read_only;

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        for (label, channel) in channels {
            ui.label(RichText::new(*label).color(subtle_dark()).small());
            foundation_input_cell(ui, &format_pc_float(*channel), 76.0);
        }

        let (rect, response) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
        ui.painter().rect_filled(rect, 2.0, swatch);
        ui.painter()
            .rect_stroke(rect, 2.0, Stroke::new(1.0, foundation_input_edge()));
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(if editable {
                "Click to edit color"
            } else {
                "Click to inspect color"
            });
        if response.clicked() {
            let mut popup = MaterialColorPopup::new(&meta.label, r, g, b, a);
            if editable {
                popup = popup.with_color_field(edit.tag_key, path, argb);
            }
            *edit.color_request = Some(popup);
        }
        draw_field_help(ui, meta);
    });
}

pub(in crate::app) fn draw_foundation_multi_value_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    parts: &[(String, String)],
    suffix: &str,
    depth: usize,
) {
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        for (label, value) in parts {
            if !label.is_empty() {
                ui.label(RichText::new(label).color(subtle_dark()).small());
            }
            foundation_input_cell(ui, value, 92.0);
        }
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });
}

pub(in crate::app) fn draw_foundation_bounds_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    lower_value: &str,
    upper_value: &str,
    suffix: &str,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let indent = depth as f32 * 12.0;
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let lower_key = format!("{buffer_key}|lower");
    let upper_key = format!("{buffer_key}|upper");
    let lower_id = edit.widget_id(("bounds_lower", &buffer_key));
    let upper_id = edit.widget_id(("bounds_upper", &buffer_key));
    let lower_has_focus = ui.memory(|memory| memory.has_focus(lower_id));
    let upper_has_focus = ui.memory(|memory| memory.has_focus(upper_id));
    let mut lower = edit
        .buffers
        .remove(&lower_key)
        .unwrap_or_else(|| lower_value.to_owned());
    let mut upper = edit
        .buffers
        .remove(&upper_key)
        .unwrap_or_else(|| upper_value.to_owned());
    if !lower_has_focus && !upper_has_focus {
        if lower != lower_value {
            lower = lower_value.to_owned();
        }
        if upper != upper_value {
            upper = upper_value.to_owned();
        }
    }

    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        let editable = edit.editable && !meta.read_only;
        if editable {
            let lower_response = foundation_text_edit_cell(ui, &mut lower, 92.0, lower_id);
            ui.label(RichText::new("to").color(subtle_dark()).small());
            let upper_response = foundation_text_edit_cell(ui, &mut upper, 92.0, upper_id);
            let commit = (lower_response.lost_focus() || upper_response.lost_focus())
                && (lower.trim() != lower_value.trim() || upper.trim() != upper_value.trim());
            if commit {
                edit.pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input: format!("{}..{}", lower.trim(), upper.trim()),
                });
            }
        } else {
            foundation_input_cell(ui, lower_value, 92.0);
            ui.label(RichText::new("to").color(subtle_dark()).small());
            foundation_input_cell(ui, upper_value, 92.0);
        }
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });

    edit.buffers.insert(lower_key, lower);
    edit.buffers.insert(upper_key, upper);
}

pub(in crate::app) fn draw_foundation_component_edit_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    parts: &[(String, String)],
    suffix: &str,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let indent = depth as f32 * 12.0;
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let mut values = Vec::with_capacity(parts.len());
    let mut responses = Vec::with_capacity(parts.len());
    let ids = parts
        .iter()
        .map(|(label, _)| edit.widget_id(("component", &buffer_key, label)))
        .collect::<Vec<_>>();
    let any_focus = ids
        .iter()
        .any(|id| ui.memory(|memory| memory.has_focus(*id)));
    for (label, value) in parts {
        let key = format!("{buffer_key}|component|{label}");
        let mut buffer = edit.buffers.remove(&key).unwrap_or_else(|| value.clone());
        if !any_focus && buffer != *value {
            buffer = value.clone();
        }
        values.push((label.clone(), value.clone(), key, buffer));
    }

    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        let editable = edit.editable && !meta.read_only;
        for (index, (label, _, _, buffer)) in values.iter_mut().enumerate() {
            if !label.is_empty() {
                ui.label(RichText::new(label.as_str()).color(subtle_dark()).small());
            }
            if editable {
                responses.push(foundation_text_edit_cell(ui, buffer, 92.0, ids[index]));
            } else {
                foundation_input_cell(ui, buffer, 92.0);
            }
        }
        if editable {
            let changed = values
                .iter()
                .any(|(_, value, _, buffer)| buffer.trim() != value.trim());
            let committed = responses.iter().any(egui::Response::lost_focus);
            if committed && changed {
                edit.pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input: values
                        .iter()
                        .map(|(_, _, _, buffer)| buffer.trim())
                        .collect::<Vec<_>>()
                        .join(", "),
                });
            }
        }
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });

    for (_, _, key, buffer) in values {
        edit.buffers.insert(key, buffer);
    }
}

pub(in crate::app) fn draw_foundation_text_row(
    ui: &mut Ui,
    name: &str,
    value: &str,
    suffix: &str,
    depth: usize,
) {
    let meta = field_display_meta(name);
    draw_foundation_meta_text_row(ui, &meta, value, suffix, depth);
}

pub(in crate::app) fn draw_foundation_meta_text_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    suffix: &str,
    depth: usize,
) {
    let indent = depth as f32 * 12.0;
    let suffix_reserve = if suffix.is_empty() { 0.0 } else { 96.0 };
    let available_value_width =
        (ui.available_width() - indent - FOUNDATION_LABEL_WIDTH - suffix_reserve - 28.0)
            .clamp(180.0, 920.0);
    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        foundation_input_cell(
            ui,
            value,
            foundation_value_width(value, available_value_width),
        );
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });
}

pub(in crate::app) fn draw_foundation_editable_text_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    suffix: &str,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let indent = depth as f32 * 12.0;
    let suffix_reserve = if suffix.is_empty() { 0.0 } else { 96.0 };
    let available_value_width =
        (ui.available_width() - indent - FOUNDATION_LABEL_WIDTH - suffix_reserve - 28.0)
            .clamp(180.0, 920.0);
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let id = edit.widget_id(("text", &buffer_key));
    let buffer = edit
        .buffers
        .entry(buffer_key.clone())
        .or_insert_with(|| value.to_owned());
    if !ui.memory(|memory| memory.has_focus(id)) && buffer != value {
        *buffer = value.to_owned();
    }

    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());

        let width = foundation_value_width(buffer, available_value_width);
        let response = foundation_text_edit_cell(ui, buffer, width, id);
        let commit = response.lost_focus() && buffer.trim() != value.trim();
        if commit {
            edit.pending.push(PendingFieldEdit {
                path: path.to_owned(),
                input: buffer.trim().to_owned(),
            });
        }
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });
}

/// Red used to flag tag references whose target file is missing on disk.
pub(in crate::app) const REFERENCE_MISSING_COLOR: Color32 = Color32::from_rgb(216, 92, 92);
