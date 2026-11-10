//! Scalar, enum, bounds, color, and multi-component value rows.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

pub(super) const FOUNDATION_LABEL_INSET: f32 = 11.0;

pub(super) fn stacked_field_row(ui: &Ui) -> bool {
    ui.available_width() < 760.0
}

/// Keep the label beside its controls when there is room; in narrow panes
/// give the controls their own wrapping row instead of a minimum-width canvas.
pub(super) fn foundation_field_row<R>(
    ui: &mut Ui, meta: &FieldDisplayMeta, depth: usize,
    controls: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    foundation_field_row_at(ui, meta, depth, 760.0, controls)
}

pub(super) fn foundation_field_row_at<R>(
    ui: &mut Ui, meta: &FieldDisplayMeta, depth: usize, breakpoint: f32,
    controls: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    foundation_field_row_inner(ui, meta, depth, breakpoint, 0.0, controls)
}

fn foundation_field_row_inner<R>(
    ui: &mut Ui, meta: &FieldDisplayMeta, _depth: usize, breakpoint: f32, leading_label_space: f32,
    controls: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    // Container bodies already position nested fields; row padding is local.
    // Controls retain one identity even when the label moves to another row.
    let controls_id = ui.next_auto_id().with("foundation_field_controls");
    let controls = |ui: &mut Ui| ui.scope_builder(egui::UiBuilder::new().scope_id(controls_id), controls).inner;
    if ui.available_width() < breakpoint {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                foundation_stacked_label_cell(ui, &meta.label, meta.help.as_deref());
            });
            // Keep the stacked label close to its own input even when the
            // outer field rows use a larger vertical gap.
            ui.add_space(1.0 - ui.spacing().item_spacing.y);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(FOUNDATION_LABEL_INSET);
                controls(ui)
            }).inner
        })
    } else {
        ui.horizontal(|ui| {
            foundation_label_cell_with_trailing_space(ui, &meta.label, meta.help.as_deref(), leading_label_space);
            controls(ui)
        })
    }
}

pub(super) fn foundation_caption_space(ui: &Ui, caption: &str) -> f32 {
    if caption.is_empty() { return 0.0; }
    ui.painter().layout_no_wrap(caption.to_owned(), TextStyle::Small.resolve(ui.style()), subtle_dark())
        .size().x + ui.spacing().item_spacing.x
}

pub(super) fn foundation_field_breakpoint(ui: &Ui, _depth: usize, controls_width: f32, suffix: &str) -> f32 {
    FOUNDATION_LABEL_WIDTH + controls_width
        + foundation_caption_space(ui, suffix) + 28.0
}

pub(super) fn foundation_picker_controls_width(ui: &Ui, caption: &str, suffix: &str, extra: f32) -> f32 {
    150.0 + ui.painter().layout_no_wrap(caption.to_owned(), TextStyle::Button.resolve(ui.style()), text_dark()).size().x
        + 16.0 + 24.0 + 2.0 * ui.spacing().button_padding.x + 2.0 * ui.spacing().item_spacing.x
        + foundation_caption_space(ui, suffix) + extra
}

pub(super) fn foundation_picker_value_width(
    ui: &Ui, value: &str, available: f32,
    picker_caption: &str, suffix: &str, extra_controls: f32,
) -> f32 {
    let preferred = foundation_value_width(ui, value, available).min(300.0);
    let reserve = foundation_picker_controls_width(ui, picker_caption, suffix, extra_controls) - 150.0;
    preferred.min((ui.available_size_before_wrap().x - reserve).max(1.0))
}

const COMPONENT_LABEL_GAP: f32 = 4.0;
const COMPONENT_GROUP_GAP: f32 = 12.0;

fn component_label_width(ui: &Ui, label: &str) -> f32 {
    if label.is_empty() { return 0.0; }
    ui.painter().layout_no_wrap(label.to_owned(), TextStyle::Small.resolve(ui.style()), subtle_dark()).size().x
}

fn foundation_component_row<R>(
    ui: &mut Ui, meta: &FieldDisplayMeta, depth: usize, parts: &[(String, String)],
    controls: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let prefix = parts.first().map_or(0.0, |(label, _)| component_label_width(ui, label) + COMPONENT_LABEL_GAP);
    foundation_field_row_inner(ui, meta, depth, 760.0, prefix, controls)
}

fn foundation_component_width(ui: &Ui, parts: &[(String, String)], preferred: f32, reserve: f32) -> f32 {
    let labels = parts.iter().skip(1).filter(|(label, _)| !label.is_empty()).map(|(label, _)|
        component_label_width(ui, label) + COMPONENT_LABEL_GAP).sum::<f32>();
    let gaps = parts.len().saturating_sub(1) as f32 * COMPONENT_GROUP_GAP.max(ui.spacing().item_spacing.x);
    ((ui.available_size_before_wrap().x - labels - gaps - reserve).max(1.0) / parts.len().max(1) as f32).min(preferred)
}

/// Keep a component label with its box. The first label hangs in the gutter,
/// leaving its box on the same column as an ordinary single-value field.
fn foundation_component_cell<R>(
    ui: &mut Ui, index: usize, label: &str, width: f32, draw: impl FnOnce(&mut Ui) -> R,
) -> R {
    if index > 0 { ui.add_space((COMPONENT_GROUP_GAP - ui.spacing().item_spacing.x).max(0.0)); }
    let label_width = component_label_width(ui, label);
    let prefix = if index > 0 && !label.is_empty() { label_width + COMPONENT_LABEL_GAP } else { 0.0 };
    let row = ui.allocate_ui_with_layout(Vec2::new(width + prefix, 24.0),
        egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = COMPONENT_LABEL_GAP;
            if index > 0 && !label.is_empty() { ui.label(RichText::new(label).color(subtle_dark()).small()); }
            draw(ui)
        });
    if index == 0 && !label.is_empty() {
        let rect = egui::Rect::from_min_max(ui.clip_rect().min,
            egui::pos2(row.response.rect.left() - 1.0, ui.clip_rect().bottom()));
        ui.painter().with_clip_rect(rect).text(
            row.response.rect.left_center() - egui::vec2(COMPONENT_LABEL_GAP, 0.0),
            Align2::RIGHT_CENTER, label, TextStyle::Small.resolve(ui.style()), subtle_dark());
    }
    row.inner
}

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
    // The target block of a block-index or block-flags field, when it could be
    // found among the struct's siblings or ancestors; `None` for other fields
    // and unresolvable (custom) indices → numeric editor.
    block_index: Option<&BlockIndexTarget>,
    tag_reference_value_width: f32,
) {
    if let (Some(target), Some(index)) = (block_index, block_index_value(value)) {
        draw_foundation_block_index_row(ui, meta, index, target, depth, path, edit);
        return;
    }
    if let TagFieldData::Data(bytes) = value {
        draw_foundation_data_row(ui, field, meta, bytes, depth, path, edit);
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

    if let Some((raw, mut flag_names)) = flag_value_parts(value) {
        if let Some(target) = block_index.filter(|_| is_block_flags(value)) {
            flag_names = block_flags_names(edit, target, value);
        }
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
            matches!(value, TagFieldData::Point2d(_)),
            edit,
        );
        return;
    }

    let formatted = format_foundation_scalar_value(names, value);
    if let Some(range) = slider_range(meta, value) {
        draw_foundation_slider_row(
            ui,
            meta,
            &formatted,
            range,
            field_suffix(meta, type_name).as_str(),
            depth,
            path,
            edit,
        );
        return;
    }
    let raw_string_id = match value {
        TagFieldData::StringId(id) | TagFieldData::OldStringId(id) => Some(id.string.as_str()),
        _ => None,
    };
    if matches!(
        value,
        TagFieldData::StringId(_) | TagFieldData::OldStringId(_) | TagFieldData::String(_) | TagFieldData::LongString(_)
    ) && let Some((editors, editor)) = string_editors::field_string_editor(edit, path)
    {
        string_editors::draw_string_editor_row(
            ui,
            meta,
            raw_string_id.unwrap_or(&formatted),
            field_suffix(meta, type_name).as_str(),
            depth,
            path,
            edit,
            &editors,
            &editor,
        );
        return;
    }
    if let Some(value) = raw_string_id {
        let function_lookup = is_h2_hud_function_tag(edit.game, edit.group_tag)
            && matches!(field.name(), "input name" | "range name");
        let function_menu = function_lookup.then(|| {
            // HUD effects consume four local inputs. Shader lookups keep the
            // full built-in catalog, including these names in its HUD submenu.
            blam_tags::string_editor::Menu {
                items: (1..=4).map(|index| blam_tags::string_editor::MenuItem::item(format!("hud_input{index}"))).collect(),
                accepts_anything: true,
                ..Default::default()
            }
        });
        let enabled = edit.can_edit(meta);
        let buffer_key = format!("{}|{}", edit.tag_key, path);
        let id = edit.widget_id(("text", &buffer_key));
        let function_menu_id = edit.widget_id(("function choices", &buffer_key));
        let draft = edit.buffers.draft_mut(&buffer_key, value);
        let menu_width = if function_lookup { 48.0 + ui.spacing().item_spacing.x } else { 0.0 };
        draw_foundation_cell_row_with_min_width(ui, meta, &field_suffix(meta, type_name), depth, 150.0 + menu_width, |ui, available_width| {
            let width = foundation_value_width(ui, &draft.text, available_width - menu_width);
            let response = foundation_string_id_value_cell(ui, &mut draft.text, width, id, enabled, false);
            draft.note_response(&response);
            if enabled && draft.should_commit(ui, &response) { edit.pending.extend(field_edit_ops(path, &draft.text).pending); }
            draft.keep_commit(|| single_field_commit(edit.tag_key, &buffer_key, path));
            // Culling offscreen rows changes automatic UI IDs. Anchor the
            // popup to this field so focusing its search cannot replace it.
            let picked = function_lookup && ui.scope_builder(egui::UiBuilder::new().scope_id(function_menu_id), |ui| {
                function_name_suggestions(ui, &buffer_key, &mut draft.text, enabled, function_menu.as_ref())
            }).inner;
            if picked {
                edit.pending.extend(field_edit_ops(path, &draft.text).pending);
                draft.mark_committed();
            }
        });
        return;
    }
    if edit.can_edit(meta) && is_text_editable_value(value) {
        draw_foundation_editable_text_row(
            ui,
            meta,
            &formatted,
            field_suffix(meta, type_name).as_str(),
            depth,
            if matches!(value, TagFieldData::String(_) | TagFieldData::LongString(_)) { 300.0 } else { 140.0 },
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
        if matches!(value, TagFieldData::String(_) | TagFieldData::LongString(_)) { 300.0 } else { 140.0 },
    );
}

/// A color value row: one editable cell per channel plus a clickable swatch
/// that opens the color picker. ARGB rows show all four components in a/r/g/b
/// order.

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
    // Same order the color parser reads: "a, r, g, b" / "r, g, b".
    let channels: &[(&str, f32)] = if argb {
        &[("a", a), ("r", r), ("g", g), ("b", b)]
    } else {
        &[("r", r), ("g", g), ("b", b)]
    };
    let parts = channels
        .iter()
        .map(|(label, channel)| ((*label).to_owned(), format_pc_float(*channel)))
        .collect::<Vec<_>>();
    let swatch = Color32::from_rgb(
        float_channel_to_u8(r),
        float_channel_to_u8(g),
        float_channel_to_u8(b),
    );
    let editable = edit.can_edit(meta);

    foundation_component_row(ui, meta, depth, &parts, |ui| {
        const SWATCH_WIDTH: f32 = 48.0;
        let width = foundation_component_width(ui, &parts, 76.0, SWATCH_WIDTH + ui.spacing().item_spacing.x);
        draw_foundation_component_cells(ui, &parts, width, path, editable, false, edit);

        let response = crate::app::ui_kit::read_only_input_scope(ui, !editable, |ui|
            color_preview_button(ui, Vec2::new(SWATCH_WIDTH, BUTTON_HEIGHT), swatch));
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(if editable {
                "Click to edit color"
            } else {
                "Click to inspect color"
            });
        if response.clicked() {
            let mut popup =
                MaterialColorPopup::new(&meta.label, r, g, b, a).with_alpha_available(argb);
            if editable {
                popup = popup.with_color_field(edit.tag_key, path, argb);
            }
            *edit.color_request = Some(popup);
        }
    });
}

pub(in crate::app) fn draw_foundation_multi_value_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    parts: &[(String, String)],
    suffix: &str,
    depth: usize,
) {
    foundation_component_row(ui, meta, depth, parts, |ui| {
        let width = foundation_component_width(ui, parts, 92.0, foundation_caption_space(ui, suffix));
        for (index, (label, value)) in parts.iter().enumerate() {
            foundation_component_cell(ui, index, label, width, |ui| foundation_input_cell(ui, value, width));
        }
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
    });
}

/// The edit a row setting `path` from `input` commits.
pub(in crate::app) fn field_edit_ops(path: &str, input: &str) -> DeferredOps {
    DeferredOps {
        pending: vec![PendingFieldEdit {
            path: path.to_owned(),
            input: input.trim().to_owned(),
        }],
        ..DeferredOps::default()
    }
}

/// How a row that sets `path` from its one box, `buffer_key`, commits without
/// being drawn.
pub(in crate::app) fn single_field_commit(tag_key: &str, buffer_key: &str, path: &str) -> DraftCommit {
    let path = path.to_owned();
    DraftCommit::new(tag_key, vec![buffer_key.to_owned()], move |texts| {
        Ok(field_edit_ops(&path, texts[0]))
    })
}

/// A bounds value from its two boxes, as the field's parser reads it.
fn bounds_input(lower: &str, upper: &str) -> String {
    format!("{}..{}", lower.trim(), upper.trim())
}

/// A multi-component value from its boxes, in the order the field's parser
/// reads them.
fn components_input(texts: &[&str]) -> String {
    texts.iter().map(|text| text.trim()).collect::<Vec<_>>().join(", ")
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
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let lower_key = format!("{buffer_key}|lower");
    let upper_key = format!("{buffer_key}|upper");
    let lower_id = edit.widget_id(("bounds_lower", &buffer_key));
    let upper_id = edit.widget_id(("bounds_upper", &buffer_key));
    let mut lower = edit.buffers.take(&lower_key, lower_value);
    let mut upper = edit.buffers.take(&upper_key, upper_value);

    let controls_width = 184.0 + foundation_caption_space(ui, "to") + 2.0 * ui.spacing().item_spacing.x;
    let breakpoint = foundation_field_breakpoint(ui, depth, controls_width, suffix);
    foundation_field_row_at(ui, meta, depth, breakpoint, |ui| {
        let editable = edit.can_edit(meta);
        let width = ((ui.available_size_before_wrap().x - foundation_caption_space(ui, suffix)
            - foundation_caption_space(ui, "to") - 2.0 * ui.spacing().item_spacing.x) * 0.5).clamp(1.0, 92.0);
        let lower_response = foundation_value_cell(ui, &mut lower.text, width, lower_id, editable);
        ui.label(RichText::new("to").color(subtle_dark()).small());
        let upper_response = foundation_value_cell(ui, &mut upper.text, width, upper_id, editable);
        if editable {
            lower.note_response(&lower_response);
            upper.note_response(&upper_response);
            // Both asked, so a commit in one box is seen even when the other
            // commits too; the edit carries both.
            let lower_commit = lower.should_commit(ui, &lower_response);
            let upper_commit = upper.should_commit(ui, &upper_response);
            if lower_commit || upper_commit {
                edit.push_ops(field_edit_ops(path, &bounds_input(&lower.text, &upper.text)));
                lower.mark_committed();
                upper.mark_committed();
            }
            if lower.changed || upper.changed {
                let path = path.to_owned();
                let commit = DraftCommit::new(
                    edit.tag_key,
                    vec![lower_key.clone(), upper_key.clone()],
                    move |texts| Ok(field_edit_ops(&path, &bounds_input(texts[0], texts[1]))),
                );
                lower.keep_commit(|| commit.clone());
                upper.keep_commit(|| commit);
            }
        }
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
    });

    edit.buffers.put(lower_key, lower);
    edit.buffers.put(upper_key, upper);
}

pub(in crate::app) fn draw_foundation_component_edit_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    parts: &[(String, String)],
    suffix: &str,
    depth: usize,
    path: &str,
    integer_coordinates: bool,
    edit: &mut FieldEditContext<'_>,
) {
    foundation_component_row(ui, meta, depth, parts, |ui| {
        let editable = edit.can_edit(meta);
        let width = foundation_component_width(ui, parts, 92.0, foundation_caption_space(ui, suffix));
        draw_foundation_component_cells(ui, parts, width, path, editable, integer_coordinates, edit);
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
    });
}

fn integer_coordinate_warning(text: &str) -> Option<String> {
    if text.trim().parse::<i16>().is_ok() { return None; }
    Some(match crate::core::document::value::parse_integer_point_coordinate(text) {
        Ok(rounded) => format!("Will round to {rounded} on Enter or when leaving the field."),
        Err(reason) => reason,
    })
}

fn component_input_ops(path: &str, texts: &[&str], integer_coordinates: bool) -> Result<DeferredOps, String> {
    if !integer_coordinates { return Ok(field_edit_ops(path, &components_input(texts))); }
    let coordinates = texts.iter().map(|text|
        crate::core::document::value::parse_integer_point_coordinate(text).map(|value| value.to_string())
    ).collect::<Result<Vec<_>, _>>()?;
    Ok(field_edit_ops(path, &coordinates.join(", ")))
}

/// One labelled cell per component, inside the caller's row layout. When
/// `editable`, each cell is a text box, and once any of them commits a change
/// the whole value is queued as one edit: every component's text joined with
/// `", "`, in `parts` order — the order the field's parser reads them in.
fn draw_foundation_component_cells(
    ui: &mut Ui,
    parts: &[(String, String)],
    width: f32,
    path: &str,
    editable: bool,
    integer_coordinates: bool,
    edit: &mut FieldEditContext<'_>,
) {
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let ids = parts
        .iter()
        .map(|(label, _)| edit.widget_id(("component", &buffer_key, label)))
        .collect::<Vec<_>>();
    let mut drafts = Vec::with_capacity(parts.len());
    for (label, value) in parts {
        let key = format!("{buffer_key}|component|{label}");
        let mut draft = edit.buffers.take(&key, value);
        // Saving flushes drafts before this row draws. Clear a rounded draft
        // even when it rounded back to the value the tag already contained.
        if integer_coordinates && !draft.changed
            && crate::core::document::value::parse_integer_point_coordinate(&draft.text)
                .is_ok_and(|rounded| rounded.to_string() == value.trim()) {
            draft.set_clean(value);
        }
        drafts.push((key, draft));
    }

    let mut responses = Vec::with_capacity(parts.len());
    for (index, ((label, value), (_, draft))) in parts.iter().zip(drafts.iter_mut()).enumerate() {
        if integer_coordinates && ui.memory(|memory| memory.has_focus(ids[index]) || memory.had_focus_last_frame(ids[index]))
            && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            draft.set_clean(value);
        }
        let warning = integer_coordinates.then(|| integer_coordinate_warning(&draft.text)).flatten();
        let response = foundation_component_cell(ui, index, label, width,
            |ui| foundation_value_cell_with_warning(ui, &mut draft.text, width, ids[index], editable, warning.as_deref()));
        if editable {
            draft.note_response(&response);
        }
        responses.push(response);
    }
    if editable {
        // Every box asked, not just until one says yes, so each sees its own
        // focus loss; the edit carries all of them.
        let mut committed = false;
        for (response, (_, draft)) in responses.iter().zip(drafts.iter_mut()) {
            committed |= draft.should_commit(ui, response);
        }
        if committed {
            let texts = drafts.iter().map(|(_, draft)| draft.text.as_str()).collect::<Vec<_>>();
            match component_input_ops(path, &texts, integer_coordinates) {
                Ok(ops) => {
                    edit.push_ops(ops);
                    if integer_coordinates {
                        for (_, draft) in &mut drafts {
                            if let Ok(value) = crate::core::document::value::parse_integer_point_coordinate(&draft.text) {
                                draft.text = value.to_string();
                            }
                        }
                    }
                }
                Err(reason) => { if let Some(status) = edit.status.as_deref_mut() { *status = reason; } }
            }
            for (_, draft) in &mut drafts {
                draft.mark_committed();
            }
        }
        if drafts.iter().any(|(_, draft)| draft.changed) {
            let path = path.to_owned();
            let commit = DraftCommit::new(
                edit.tag_key,
                drafts.iter().map(|(key, _)| key.clone()).collect(),
                move |texts| component_input_ops(&path, texts, integer_coordinates),
            );
            for (_, draft) in &mut drafts {
                draft.keep_commit(|| commit.clone());
            }
        }
    }

    for (key, draft) in drafts {
        edit.buffers.put(key, draft);
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
    draw_foundation_meta_text_row(ui, &meta, value, suffix, depth, 300.0);
}

pub(in crate::app) fn draw_foundation_meta_text_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    suffix: &str,
    depth: usize,
    minimum_width: f32,
) {
    draw_foundation_cell_row_with_min_width(ui, meta, suffix, depth, minimum_width, |ui, available_width| {
        foundation_input_cell(ui, value, foundation_value_width(ui, value, available_width));
    });
}

pub(in crate::app) fn draw_foundation_editable_text_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    suffix: &str,
    depth: usize,
    minimum_width: f32,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let id = edit.widget_id(("text", &buffer_key));
    let draft = edit.buffers.draft_mut(&buffer_key, value);
    draw_foundation_cell_row_with_min_width(ui, meta, suffix, depth, minimum_width, |ui, available_width| {
        let width = foundation_value_width(ui, &draft.text, available_width);
        let response = foundation_value_cell(ui, &mut draft.text, width, id, true);
        draft.note_response(&response);
        if draft.should_commit(ui, &response) {
            edit.pending.extend(field_edit_ops(path, &draft.text).pending);
        }
        draft.keep_commit(|| single_field_commit(edit.tag_key, &buffer_key, path));
    });
}

/// The game's data definitions, read once per definitions folder and game.
fn data_definitions(
    definitions_root: Option<&std::path::Path>,
    game: Option<GameId>,
) -> Option<std::sync::Arc<blam_tags::data_text::DataDefinitions>> {
    use std::sync::{Arc, Mutex, OnceLock};
    type Definitions = Option<Arc<blam_tags::data_text::DataDefinitions>>;
    static CACHE: OnceLock<Mutex<std::collections::HashMap<(std::path::PathBuf, GameId), Definitions>>> =
        OnceLock::new();
    let (root, game) = (definitions_root?, game?);
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry((root.to_path_buf(), game))
        .or_insert_with(|| blam_tags::data_text::DataDefinitions::load(root.join(game.as_str())).map(Arc::new).ok())
        .clone()
}

/// The tallest a text data field's box starts, as Foundation's grows to
/// before it scrolls.
const DATA_TEXT_MAX_HEIGHT: f32 = 200.0;
/// How wide a text data field's box is at least when there is room,
/// whatever its text: Foundation's.
const DATA_TEXT_WIDTH: f32 = 600.0;
/// The space between a text data field's box and its text.
pub(super) const DATA_TEXT_PADDING: egui::Vec2 = egui::vec2(4.0, 2.0);
/// The smallest the corner grip makes a text data field's box.
const DATA_TEXT_MIN_WIDTH: f32 = 200.0;
const DATA_TEXT_MIN_HEIGHT: f32 = 40.0;
/// Room left between a text data field's box and the pane's right edge, where
/// the pane's floating scroll bar would take a press meant for the grip.
pub(super) const DATA_TEXT_EDGE_GAP: f32 = 24.0;

/// A data field's size as Foundation writes it: bytes up to 1 KB, then KB,
/// MB or GB to two places with the exact count after.
fn data_size_text(len: usize) -> String {
    const UNITS: [&str; 6] = ["bytes", "KB", "MB", "GB", "TB", "PB"];
    let mut size = len as f64;
    let mut unit = 0;
    while size > 1024.0 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("Data size:  {len} bytes")
    } else {
        format!("Data size:  {size:.2} {}    ({len} bytes)", UNITS[unit])
    }
}

/// A data field as Foundation shows one: its size, and, when its definition
/// marks it as text (HaloScript source, a shader include, an import log),
/// the text in a box, editable when the field is. Text stored with CRLF line
/// ends is shown with plain ones and written back with CRLF. Enter types a
/// line break, so an edit commits when the box loses focus.
fn draw_foundation_data_row(
    ui: &mut Ui,
    field: TagField<'_>,
    meta: &FieldDisplayMeta,
    bytes: &[u8],
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    foundation_field_row(ui, meta, depth, |ui| {
        ui.label(RichText::new(data_size_text(bytes.len())).color(text_dark()));
    });
    let Some(definition) = data_definitions(edit.definitions_root, edit.game)
        .and_then(|definitions| definitions.of_field(edit.group_tag, &field))
        .filter(|definition| definition.is_text())
    else {
        return;
    };
    let stored = blam_tags::data_text::text_from_data(bytes, definition);
    let crlf = stored.contains("\r\n");
    let shown = if crlf { stored.replace("\r\n", "\n") } else { stored };
    let editable = edit.can_edit(meta);
    let tag_key = edit.tag_key;
    let buffer_key = format!("{tag_key}|{path}");
    let id = edit.widget_id(("data_text", &buffer_key));
    let draft = edit.buffers.draft_mut(&buffer_key, &shown);
    // The bytes the box's text is stored as, with the line ends it was read
    // with.
    let stored_bytes = move |text: &str| {
        let text = if crlf { text.replace('\n', "\r\n") } else { text.to_owned() };
        blam_tags::data_text::data_from_text(&text, definition)
    };
    let data_edit = |path: &str, bytes: Vec<u8>| {
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        field_edit_ops(path, &hex)
    };
    ui.horizontal_top(|ui| {
        ui.add_space(if stacked_field_row(ui) { FOUNDATION_LABEL_INSET } else { FOUNDATION_LABEL_WIDTH + ui.spacing().item_spacing.x });
        // Foundation's box is at least 600 wide and grows to 200 tall; Halo 2
        // Guerilla's is a fixed 540 by 128 in an 11-pixel fixed-pitch font.
        // Neither wraps. This one starts as wide as there is room for and as
        // tall as its text up to 200, and its corner grip makes it any size
        // that fits; the size it is dragged to is kept.
        let available = ui.available_width() - DATA_TEXT_EDGE_GAP;
        let font = FontId::monospace(12.0);
        // The text's own extent, padded as the box pads it. The box is never
        // dragged past it: a box bigger than its text is only empty space.
        let extent = findable_galley(ui, &draft.text, font.clone(), text_dark(), FindTargetKind::Value).size()
            + 2.0 * DATA_TEXT_PADDING;
        let largest = egui::vec2(
            extent.x.max(DATA_TEXT_WIDTH).min(available),
            extent.y.max(DATA_TEXT_MIN_HEIGHT),
        );
        egui::Resize::default()
            .id(id.with("resize"))
            .with_stroke(false)
            .default_size([largest.x, largest.y.min(DATA_TEXT_MAX_HEIGHT)])
            .min_size([DATA_TEXT_MIN_WIDTH.min(largest.x), DATA_TEXT_MIN_HEIGHT])
            .max_size(largest)
            .show(ui, |ui| {
                crate::app::ui_kit::read_only_input_scope(ui, !editable, |ui| {
                    // The box is the whole resizable area, so its grip is the
                    // box's own corner. Its fill and border are painted under the
                    // text, the border once focus is known.
                    let frame_rect = ui.max_rect();
                    let background = ui.painter().add(egui::Shape::Noop);
                    // Greyed when it can't be edited, as the value boxes are.
                    let text_color = if stored_bytes(&draft.text).is_err() {
                        invalid_value_text()
                    } else { text_dark() };
                    let scroll = crate::app::ScrollArea::both()
                        .id_salt(id.with("scroll"))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
                                findable_galley(ui, text.as_str(), font.clone(), text_color, FindTargetKind::Value)
                            };
                            let mut read_only = draft.text.as_str();
                            let buffer: &mut dyn egui::TextBuffer =
                                if editable { &mut draft.text } else { &mut read_only };
                            // As big as the box, so a press anywhere in it puts the
                            // caret in the text.
                            let response = ui.add(
                                egui::TextEdit::multiline(buffer)
                                    .id(id)
                                    .font(font.clone())
                                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(
                                        DATA_TEXT_PADDING.x as i8,
                                        DATA_TEXT_PADDING.y as i8,
                                    )))
                                    .min_size(frame_rect.size())
                                    .desired_width(frame_rect.width() - 2.0 * DATA_TEXT_PADDING.x)
                                    .layouter(&mut layouter),
                            );
                            if !editable {
                                return;
                            }
                            draft.note_response(&response);
                            if draft.changed
                                && lost_focus_once(&response)
                                && let Ok(bytes) = stored_bytes(&draft.text)
                            {
                                edit.pending.extend(data_edit(path, bytes).pending);
                                draft.mark_committed();
                            }
                        });
                    let invalid = stored_bytes(&draft.text).is_err();
                    let appearance = input_appearance(ui,
                        if invalid { InputState::Invalid } else { InputState::Normal },
                        !editable, ui.memory(|memory| memory.has_focus(id)));
                    ui.painter().set(
                        background,
                        egui::Shape::Vec(vec![
                            egui::Shape::rect_filled(
                                frame_rect,
                                appearance.corners,
                                appearance.fill,
                            ),
                            egui::Shape::rect_stroke(frame_rect, appearance.corners, appearance.stroke, egui::StrokeKind::Inside),
                        ]),
                    );
                    // While its text can still scroll, the box keeps the wheel:
                    // otherwise what it doesn't use scrolls the pane as well, and
                    // the two move at once.
                    let can_scroll = scroll.content_size - scroll.inner_rect.size();
                    if ui.rect_contains_pointer(frame_rect) {
                        ui.input_mut(|input| {
                            if can_scroll.x > 0.5 {
                                input.smooth_scroll_delta.x = 0.0;
                            }
                            if can_scroll.y > 0.5 {
                                input.smooth_scroll_delta.y = 0.0;
                            }
                        });
                    }
                });
            });
    });
    if !draft.changed {
        return;
    }
    let owned_path = path.to_owned();
    draft.keep_commit(|| {
        DraftCommit::new(tag_key, vec![buffer_key.clone()], move |texts| {
            let bytes = stored_bytes(texts[0]).map_err(|too_long| too_long.to_string())?;
            Ok(data_edit(&owned_path, bytes))
        })
    });
    if let Err(too_long) = stored_bytes(&draft.text) {
        ui.horizontal(|ui| {
            ui.add_space(if stacked_field_row(ui) { FOUNDATION_LABEL_INSET } else { FOUNDATION_LABEL_WIDTH + ui.spacing().item_spacing.x });
            ui.label(RichText::new(too_long.to_string()).color(invalid_value_text()).small());
        });
    }
}

/// The range a value row's slider covers, if it has one: a `sled` real's or
/// integer's from its definition, a `real_slider`'s from the `[min...max]`
/// in its name. An integer's slider steps by whole numbers.
fn slider_range(meta: &FieldDisplayMeta, value: &TagFieldData) -> Option<SliderRange> {
    match value {
        TagFieldData::Real(_) => meta.slider,
        TagFieldData::CharInteger(_)
        | TagFieldData::ShortInteger(_)
        | TagFieldData::LongInteger(_)
        | TagFieldData::Int64Integer(_)
        | TagFieldData::ByteInteger(_)
        | TagFieldData::WordInteger(_)
        | TagFieldData::DwordInteger(_)
        | TagFieldData::QwordInteger(_) => meta.slider.map(|range| SliderRange {
            step: Some(range.step.unwrap_or(1.0).round().max(1.0)),
            ..range
        }),
        TagFieldData::RealSlider(_) => meta
            .slider
            .or_else(|| meta.range.as_deref().and_then(slider_range_from_hint)),
        _ => None,
    }
}

/// A `[min...max]` (or `[min,max]`) range hint as a slider range.
fn slider_range_from_hint(hint: &str) -> Option<SliderRange> {
    let inner = hint.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (min, max) = inner.split_once("...").or_else(|| inner.split_once(','))?;
    let (min, max) = (min.trim().parse::<f32>().ok()?, max.trim().parse::<f32>().ok()?);
    (min < max).then_some(SliderRange { min, max, step: None })
}

/// `value` as a slider sets it: snapped to the step and written with only as
/// many decimals as the step has, the way Foundation's slider does.
fn slider_value_text(value: f32, step: Option<f32>) -> String {
    let Some(step) = step else {
        return fmt_real(value);
    };
    let snapped = (value / step).round() * step;
    let decimals = (1.0 / step).log10().ceil().max(0.0) as usize;
    let mut text = format!("{snapped:.decimals$}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    if text == "-0" { "0".to_owned() } else { text }
}

/// The widest a slider row's slider gets and the narrowest it shrinks to in
/// a narrow pane, and the same for the value box beside it.
const SLIDER_WIDTH: f32 = 180.0;
const SLIDER_MIN_WIDTH: f32 = 40.0;
const SLIDER_VALUE_WIDTH: f32 = 92.0;
const SLIDER_VALUE_MIN_WIDTH: f32 = 50.0;

/// A value edited with a slider over its recommended range and a box for
/// typing any value, outside the range too. A drag changes the box as it
/// goes and commits once, when it ends.
#[allow(clippy::too_many_arguments)]
fn draw_foundation_slider_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    range: SliderRange,
    suffix: &str,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let editable = edit.can_edit(meta);
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let id = edit.widget_id(("text", &buffer_key));
    let draft = edit.buffers.draft_mut(&buffer_key, value);
    draw_foundation_labelled_cell_row(ui, meta, suffix, depth, |ui, _| {
        let limit = |value: f32| RichText::new(fmt_real(value)).color(subtle_dark()).small();
        // The slider takes what the rest of the row leaves, up to 180: in a
        // narrow pane a fixed width pushed the value box and unit past the
        // row's edge, where they were cut off.
        let small = ui.style().text_styles[&TextStyle::Small].clone();
        let text_width = |text: String| ui.painter().layout_no_wrap(text, small.clone(), subtle_dark()).size().x;
        let unit = if suffix.is_empty() { 0.0 } else { text_width(suffix.to_owned()) };
        // What the slider and the value box share.
        let room = ui.available_width() - unit - 3.0 * ui.spacing().item_spacing.x;
        let value_width = (room - SLIDER_MIN_WIDTH).clamp(SLIDER_VALUE_MIN_WIDTH, SLIDER_VALUE_WIDTH);
        let limits = text_width(fmt_real(range.min))
            + text_width(fmt_real(range.max))
            + 2.0 * ui.spacing().item_spacing.x;
        // Too tight even for the smallest slider, the limits give their room
        // to it and say themselves when it's hovered; tighter still, the
        // value box narrows.
        let show_limits = room - value_width - limits >= SLIDER_MIN_WIDTH;
        let slider_room = room - value_width - if show_limits { limits } else { 0.0 };
        let slider_width = slider_room.clamp(SLIDER_MIN_WIDTH, SLIDER_WIDTH);
        if show_limits {
            ui.label(limit(range.min));
        }
        let mut position = draft.text.trim().parse::<f32>().unwrap_or(range.min);
        ui.spacing_mut().slider_width = slider_width;
        let slider = egui::Slider::new(&mut position, range.min..=range.max)
            .show_value(false)
            .clamping(egui::SliderClamping::Never);
        let slider = match range.step {
            Some(step) => slider.step_by(step as f64),
            None => slider,
        };
        let mut slid = ui.add_enabled(editable, slider);
        if show_limits {
            ui.label(limit(range.max));
        } else {
            slid = slid.on_hover_text(format!("{} to {}", fmt_real(range.min), fmt_real(range.max)));
        }
        if slid.changed() {
            draft.text = slider_value_text(position, range.step);
        }
        draft.note_response(&slid);
        let typed = foundation_value_cell(ui, &mut draft.text, value_width, id, editable);
        if !editable {
            return;
        }
        draft.note_response(&typed);
        // A drag commits when it ends; a click or a key moves it at once.
        let slider_commit = draft.changed && (slid.drag_stopped() || (slid.changed() && !slid.dragged()));
        if slider_commit {
            draft.mark_committed();
        }
        if slider_commit || draft.should_commit(ui, &typed) {
            edit.pending.extend(field_edit_ops(path, &draft.text).pending);
        }
        draft.keep_commit(|| single_field_commit(edit.tag_key, &buffer_key, path));
        if draft.text.trim().parse::<f32>().is_ok_and(|value| value < range.min || value > range.max) {
            ui.label(RichText::new("outside recommended range").color(subtle_dark()).small());
        }
    });
}

/// A labelled row holding one value box, read-only or editable: `cell`
/// draws the box, given the width there is for it.
pub(super) fn draw_foundation_labelled_cell_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    suffix: &str,
    depth: usize,
    cell: impl FnOnce(&mut Ui, f32),
) {
    draw_foundation_cell_row_at(ui, meta, suffix, depth, 760.0, cell);
}

pub(super) fn draw_foundation_cell_row_with_min_width(
    ui: &mut Ui, meta: &FieldDisplayMeta, suffix: &str, depth: usize, minimum_width: f32,
    cell: impl FnOnce(&mut Ui, f32),
) {
    let breakpoint = foundation_field_breakpoint(ui, depth, minimum_width, suffix);
    draw_foundation_cell_row_at(ui, meta, suffix, depth, breakpoint, cell);
}

fn draw_foundation_cell_row_at(
    ui: &mut Ui, meta: &FieldDisplayMeta, suffix: &str, depth: usize, breakpoint: f32,
    cell: impl FnOnce(&mut Ui, f32),
) {
    let stacked = ui.available_width() < breakpoint;
    let suffix_reserve = foundation_caption_space(ui, suffix);
    let available_value_width = if stacked {
        (ui.available_width() - FOUNDATION_LABEL_INSET - 8.0 - suffix_reserve).max(1.0)
    } else {
        (ui.available_width() - FOUNDATION_LABEL_WIDTH - suffix_reserve - 28.0).clamp(1.0, 920.0)
    };
    foundation_field_row_at(ui, meta, depth, breakpoint, |ui| {
        cell(ui, available_value_width);
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
    });
}

/// Red used to flag tag references whose target file is missing on disk.
pub(in crate::app) const REFERENCE_MISSING_COLOR: Color32 = Color32::from_rgb(216, 92, 92);

fn is_block_flags(value: &TagFieldData) -> bool {
    matches!(
        value,
        TagFieldData::ByteBlockFlags(_)
            | TagFieldData::WordBlockFlags(_)
            | TagFieldData::LongBlockFlags(_)
    )
}

/// One flag per element of a block-flags field's target block, named as the
/// editor labels the element: bit `n` is element `n`. Elements past the
/// field's width have no bit, and are left off.
pub(in crate::app) fn block_flags_names(
    edit: &FieldEditContext<'_>,
    target: &BlockIndexTarget,
    value: &TagFieldData,
) -> Vec<(u32, String)> {
    let bits = match value {
        TagFieldData::ByteBlockFlags(_) => 8,
        TagFieldData::WordBlockFlags(_) => 16,
        _ => 32,
    };
    let Some(block) = edit
        .root
        .and_then(|root| root.field_path(&target.path))
        .and_then(|field| field.as_block())
    else {
        return Vec::new();
    };
    let default_names = TagNameIndex::default();
    let names = edit.names.unwrap_or(&default_names);
    let labeler = BlockLabeler::new(edit, names);
    (0..target.len.min(bits))
        .map(|index| (index as u32, labeler.bare_label(&target.path, block, index)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_control_identity_survives_a_breakpoint_change() {
        let ctx = egui::Context::default();
        let mut ids = Vec::new();
        for width in [1000.0, 500.0, 1000.0] {
            crate::app::run_ui_test(&ctx, egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, 200.0))),
                ..Default::default()
            }, |ui| {
                foundation_field_row(ui, &field_display_meta("enum"), 0, |ui| {
                    ids.push(ui.make_persistent_id("enum_dropdown"));
                });
            });
        }
        assert_eq!(ids[0], ids[1]);
        assert_eq!(ids[1], ids[2]);
    }

    #[test]
    fn string_id_boxes_only_grow_when_the_text_needs_more_space() {
        let ctx = egui::Context::default();
        Baboon::configure_context(&ctx);
        crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let normal = foundation_picker_value_width(ui, "shield_glow_source", 600.0, "Object Functions", "string id", 0.0);
                for value in ["current_shield_damage".to_owned(), "é".repeat(18)] {
                    let measured = ui.painter().layout_no_wrap(value.clone(), TextStyle::Monospace.resolve(ui.style()), text_dark()).size().x;
                    assert!(measured + 10.0 <= normal, "the example text already fits in a normal box");
                    let width = foundation_picker_value_width(ui, &value, 600.0, "Object Functions", "string id", 0.0);
                    assert!((width - normal).abs() < 0.1, "{value:?} should not widen a box it already fits in");
                }
                let long = "a_very_long_object_function_name_that_needs_more_space";
                let width = foundation_picker_value_width(ui, long, 600.0, "Object Functions", "string id", 0.0);
                assert!(width > normal, "long values can still widen their input");
                let narrow = foundation_picker_value_width(ui, long, 120.0, "Object Functions", "string id", 0.0);
                assert!(narrow <= 120.0, "growing values respect the available width");
            });
        });
    }

    #[test]
    fn component_boxes_share_the_scalar_column_and_have_clear_group_spacing() {
        for width in [280.0, 450.0, 1200.0] {
            for labels in [
                &["r", "g", "b"][..], &["a", "r", "g", "b"][..],
                &["x", "y", "z"][..], &["yaw", "pitch", "roll"][..],
            ] {
                for editable in [true, false] {
                    let ctx = egui::Context::default();
                    Baboon::configure_context(&ctx);
                    let parts = labels.iter().map(|label| ((*label).to_owned(), "0".to_owned())).collect::<Vec<_>>();
                    let mut scalar = egui::Rect::NOTHING;
                    let mut boxes = Vec::new();
                    let mut overflow = 0.0;
                    let mut painted = Vec::new();
                    for _ in 0..2 {
                        let output = crate::app::run_ui_test(&ctx, egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 400.0))),
                            ..Default::default()
                        }, |ui| {
                            egui::CentralPanel::default().show(ui, |ui| {
                                let right = ui.max_rect().right();
                                super::with_test_edit_context(|edit| {
                                    draw_foundation_editable_text_row(ui, &field_display_meta("single value"), "0", "", 1, 452.0, "single", edit);
                                    scalar = ctx.read_response(edit.widget_id(("text", format!("{}|single", edit.tag_key)))).unwrap().rect;
                                    edit.editable = editable;
                                    draw_foundation_component_edit_row(ui, &field_display_meta("components"), &parts, "vector", 1, "components", false, edit);
                                    boxes = labels.iter().map(|label| ctx.read_response(edit.widget_id(
                                        ("component", format!("{}|components", edit.tag_key), *label))).unwrap().rect).collect();
                                });
                                overflow = ui.min_rect().right() - right;
                            });
                        });
                        painted = output.shapes.iter().filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) => Some((text.galley.text().to_owned(), text.galley.rect.translate(text.pos.to_vec2()))),
                            _ => None,
                        }).collect::<Vec<_>>();
                    }
                    assert!((boxes[0].left() - scalar.left()).abs() < 1.0, "first component aligns at {width}px: {labels:?}");
                    assert!(overflow < 1.0, "components fit the pane at {width}px");
                    let first_label = painted.iter().find(|(text, _)| text == labels[0]).unwrap().1;
                    assert!(first_label.right() <= boxes[0].left() - 2.0, "the first label hangs to the left of its box");
                    for index in 1..labels.len() {
                        let label = painted.iter().find(|(text, _)| text == labels[index]).unwrap().1;
                        assert!(label.left() - boxes[index - 1].right() >= COMPONENT_GROUP_GAP - 0.5,
                            "component groups have a wider gap at {width}px");
                        assert!((boxes[index].left() - label.right() - COMPONENT_LABEL_GAP).abs() < 1.0,
                            "each label stays close to its own input");
                    }
                }
            }
        }
    }

    #[test]
    fn compact_values_stay_horizontal_after_references_stack() {
        let damage = blam_tags::TagFile::new(locate_definitions_root().join("halo3_mcc/damage_effect.json")).unwrap();
        let model = blam_tags::TagFile::new(locate_definitions_root().join("halo3_mcc/model.json")).unwrap();
        let damage_root = damage.root();
        let model_root = model.root();
        let number = damage_root.fields_all().find(|field| field.clean_name() == "damage lower bound").unwrap();
        let identifier = model_root.fields_all().find(|field| matches!(field.value(), Some(TagFieldData::StringId(_)))).unwrap();
        let reference = model_root.fields_all().find(|field| matches!(field.value(), Some(TagFieldData::TagReference(_)))).unwrap();
        for width in [450.0, 660.0, 1100.0] {
            let ctx = egui::Context::default();
            Baboon::configure_context(&ctx);
            let mut rects = Vec::new();
            let mut painted = Vec::new();
            let mut overflow = 0.0;
            for _ in 0..2 {
                let output = crate::app::run_ui_test(&ctx, egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 800.0))),
                    ..Default::default()
                }, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let right = ui.max_rect().right();
                        rects.clear();
                        super::with_test_edit_context(|edit| {
                            for (label, field, type_name, path, widget) in [
                                ("compact number", number, "real", "number", "text"),
                                ("compact identifier", identifier, "string id", "identifier", "text"),
                                ("wide reference", reference, "tag reference", "reference", "tag_ref"),
                            ] {
                                draw_foundation_value_row(ui, field, &field_display_meta(label), type_name,
                                    &field.value().unwrap(), &TagNameIndex::default(), 0, path, edit, None, 520.0);
                                rects.push(ctx.read_response(edit.widget_id((widget, format!("{}|{path}", edit.tag_key)))).unwrap().rect);
                            }
                        });
                        overflow = ui.min_rect().right() - right;
                    });
                });
                painted = output.shapes.iter().filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some((text.galley.text().to_owned(), text.galley.rect.translate(text.pos.to_vec2()))),
                    _ => None,
                }).collect::<Vec<_>>();
            }
            assert!(overflow < 1.0, "fields fit the {width}px pane");
            for (index, label) in ["compact number", "compact identifier", "wide reference"].iter().enumerate() {
                let label_rect = painted.iter().find(|(text, _)| text == label).unwrap().1;
                let stacked = width < 500.0 || (index == 2 && width < 760.0);
                if stacked {
                    assert!(label_rect.bottom() <= rects[index].top(), "{label} stacks at {width}px");
                } else {
                    assert!((label_rect.center().y - rects[index].center().y).abs() < 3.0,
                        "{label} stays horizontal at {width}px");
                }
            }
        }
    }

    #[test]
    fn narrow_fields_stack_labels_and_keep_block_controls_within_the_pane() {
        let tag = blam_tags::TagFile::new(locate_definitions_root().join("halo3_mcc/damage_effect.json")).unwrap();
        let root = tag.root();
        let flags = root.fields_all().find(|field|
            matches!(field.options(), Some(blam_tags::TagOptions::Flags(options)) if options.len() >= 2)).unwrap();
        for width in [280.0, 480.0, 720.0, 1200.0] {
            let ctx = egui::Context::default();
            Baboon::configure_context(&ctx);
            let mut text_rect = egui::Rect::NOTHING;
            let mut label_rect = egui::Rect::NOTHING;
            let mut reference_rect = egui::Rect::NOTHING;
            let mut caption_rect = egui::Rect::NOTHING;
            let mut reference_caption = egui::Rect::NOTHING;
            let mut block_title_rect = egui::Rect::NOTHING;
            let mut header_top = 0.0;
            let mut overflow = 0.0_f32;
            let mut header_height = 0.0;
            let mut text_stacked = false;
            for _ in 0..3 {
                let output = crate::app::run_ui_test(&ctx, egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 1400.0))),
                    ..Default::default()
                }, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let right = ui.max_rect().right();
                        super::with_test_edit_context(|edit| {
                            let meta = field_display_meta("responsive field");
                            text_stacked = ui.available_width() < foundation_field_breakpoint(ui, 1, 300.0, "world units");
                            let id = edit.widget_id(("text", format!("{}|text", edit.tag_key)));
                            draw_foundation_editable_text_row(ui, &meta, &"long value ".repeat(10), "world units", 1, 300.0, "text", edit);
                            text_rect = ctx.read_response(id).unwrap().rect;
                            draw_foundation_tag_reference_row(ui, &field_display_meta("reference"), "NONE", None, None, 1, "reference", edit, 520.0);
                            reference_rect = ctx.read_response(edit.widget_id(("tag_ref", format!("{}|reference", edit.tag_key)))).unwrap().rect;
                            draw_foundation_enum_row(ui, &field_display_meta("option"), &["one", "two"], Some(0), 1, "option", edit);
                            draw_foundation_flags_row(ui, &field_display_meta("flags"), 0, &[], flags, 1, "flags", edit);
                        });
                        let top = ui.cursor().top();
                        header_top = top;
                        draw_foundation_block_control(ui, "A block with a longer title", "0. example", 0, 4,
                            None, true, true, "responsive", "test", "block", 1, true, None, false,
                            PasteGate::Empty, Some("160 bytes"), |index| format!("{index}. example"), |_| {});
                        header_height = ui.cursor().top() - top;
                        overflow = ui.min_rect().right() - right;
                    });
                });
                label_rect = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "responsive field" =>
                        Some(text.galley.rect.translate(text.pos.to_vec2())),
                    _ => None,
                }).unwrap();
                let text_rect = |needle: &str| output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().starts_with(needle) =>
                        Some(text.galley.rect.translate(text.pos.to_vec2())),
                    _ => None,
                }).unwrap();
                caption_rect = text_rect("world units");
                reference_caption = text_rect("tag reference");
                block_title_rect = text_rect("A Block");
            }
            assert!(overflow < 1.0, "{width}px pane overflowed by {overflow}px");
            if text_stacked {
                assert!(label_rect.bottom() <= text_rect.top(), "the label moves above its input at {width}px");
                assert!((label_rect.left() - text_rect.left()).abs() < 1.0, "stacked labels align with their fields at {width}px");
            } else {
                assert!((label_rect.center().y - text_rect.center().y).abs() < 3.0);
            }
            assert!((caption_rect.center().y - text_rect.center().y).abs() < 3.0, "type captions stay beside their controls at {width}px");
            assert!((reference_caption.center().y - reference_rect.center().y).abs() < 3.0, "reference type captions stay beside their buttons at {width}px");
            if width < 900.0 {
                assert!(block_title_rect.top() < header_top + 24.0, "block titles stay on the first row at {width}px");
                assert!(header_height > 60.0, "the block toolbar stacks at {width}px");
            }
        }
    }

    /// A block-flags field lists its target block's elements as checkboxes,
    /// named as the editor names them, and a click sets the element's bit.
    /// ODST's zone sets showed `bsp zone flags` as a bare `0x0FFF`.
    #[test]
    fn block_flags_list_the_target_blocks_elements() {
        let mut tag = blam_tags::TagFile::new(crate::app::test_definition_path(
            "halo3odst_mcc/scenario.json",
        ))
        .unwrap();
        let add = |tag: &mut blam_tags::TagFile, path: &str| {
            crate::core::document::apply::apply_block_ops(
                tag,
                vec![crate::core::document::ops::BlockOp {
                    path: path.to_owned(),
                    kind: crate::core::document::ops::BlockOpKind::Add,
                }],
                &mut Default::default(),
            );
        };
        add(&mut tag, "structure bsps");
        add(&mut tag, "structure bsps");
        add(&mut tag, "zone sets");
        for (index, name) in ["h100_000", "h100_shared"].iter().enumerate() {
            crate::core::document::apply::apply_field_edit(
                &mut tag,
                &format!("structure bsps[{index}]/structure bsp"),
                &format!("sbsp:levels\\atlas\\h100\\{name}"),
            )
            .unwrap();
        }
        crate::core::document::apply::apply_field_edit(&mut tag, "zone sets[0]/bsp zone flags", "1").unwrap();

        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        let mut time = 0.0;
        let mut frame = |pointer: Vec<egui::Event>, pending: &mut Vec<PendingFieldEdit>| {
            time += 1.0 / 60.0;
            let input = egui::RawInput {
                time: Some(time),
                events: pointer,
                ..Default::default()
            };
            let definitions_root = crate::core::bundled::locate_definitions_root();
            let mut sinks = EditSinks::default();
            let mut edit = FieldEditContext::read_only(&mut sinks, "test", "test");
            let root = tag.root();
            edit.root = Some(root);
            edit.game = Some(GameId::Halo3Odst);
            edit.group_tag = u32::from_be_bytes(*b"scnr");
            edit.definitions_root = Some(definitions_root.as_path());
            edit.editable = true;
            let zone_set = root.descend("zone sets[0]").unwrap();
            let output = crate::app::run_ui_test(&ctx, input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    draw_fields_with_docs(
                        ui,
                        &zone_set,
                        &TagNameIndex::default(),
                        0,
                        true,
                        "zone sets[0]",
                        &mut edit,
                        None,
                    );
                });
            });
            pending.append(edit.pending);
            output
        };

        let mut pending = Vec::new();
        let output = frame(Vec::new(), &mut pending);
        let texts = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((text.galley.text().to_owned(), text.pos)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            !texts.iter().any(|(text, _)| text == "0x0001 (none set)"),
            "bsp zone flags still painted as a bare mask: {texts:?}"
        );
        let shared = texts
            .iter()
            .find(|(text, _)| text == "h100_shared")
            .unwrap_or_else(|| panic!("no h100_shared flag among {texts:?}"))
            .1;
        assert!(texts.iter().any(|(text, _)| text == "h100_000"), "no h100_000 flag among {texts:?}");

        let target = shared + egui::vec2(4.0, 6.0);
        for step in 0..3 {
            let at = target - egui::vec2(12.0 * (2 - step) as f32, 0.0);
            frame(vec![egui::Event::PointerMoved(at)], &mut pending);
        }
        for pressed in [true, false] {
            frame(
                vec![egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
                &mut pending,
            );
        }
        assert_eq!(pending.len(), 1, "one click, one edit: {:?}", pending.iter().map(|e| (&e.path, &e.input)).collect::<Vec<_>>());
        crate::core::document::apply::apply_field_edit(&mut tag, &pending[0].path, &pending[0].input).unwrap();
        let value = tag.root().field_path("zone sets[0]/bsp zone flags").and_then(|field| field.value());
        assert!(
            matches!(value, Some(TagFieldData::LongBlockFlags(3))),
            "clicking h100_shared should add bit 1 to bit 0: {value:?}"
        );
    }

    // Foundation unit tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    /// Both editable and read-only values must accept focus for selection/copy.
    /// Drive real pointer input through component rows, not just the cell helper.
    #[test]
    fn editable_and_read_only_value_rows_can_be_clicked_into() {
        assert!(
            click_across_value_row(true),
            "an editable value row must take a caret, or its text cannot be selected or copied"
        );
        assert!(
            click_across_value_row(false),
            "a read-only row must accept focus so its value can be selected and copied"
        );
    }

    /// Render one `real_vector_3d` row (the `Point 0 / x y z` shape from the
    /// report) and click along it until something takes keyboard focus.
    /// Returns whether anything ever did.
    fn click_across_value_row(editable: bool) -> bool {
        let ctx = egui::Context::default();
        let mut tag = TagFile::new(crate::app::test_definition_path(
            "halo4_mcc/camera_track.json",
        ))
        .unwrap();
        crate::core::document::apply::add_block_element(&mut tag, "control points").unwrap();
        let mut focused = false;

        with_test_edit_context(|edit| {
            edit.editable = editable;
            // Sweep the row rather than trusting a hand-computed cell position:
            // the widths are layout details, and a click that lands on the label
            // by accident would report "not editable" for the wrong reason.
            for step in 0..90 {
                let pointer = egui::Pos2::new(step as f32 * 10.0, 12.0);
                let click = |pressed| egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::new(900.0, 200.0),
                    )),
                    events: vec![
                        egui::Event::PointerMoved(pointer),
                        click(true),
                        click(false),
                    ],
                    ..Default::default()
                };
                let _ = crate::app::run_ui_test(&ctx, input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let field = tag
                            .root()
                            .field_path("control points[0]/position")
                            .expect("the control point's position field");
                        let value = field.value().expect("position has a value");
                        let meta = field_display_meta(field.name());
                        draw_foundation_value_row(
                            ui,
                            field,
                            &meta,
                            field.type_name(),
                            &value,
                            &TagNameIndex::default(),
                            0,
                            "control points[0]/position",
                            edit,
                            None,
                            300.0,
                        );
                    });
                });
                if ctx.memory(|memory| memory.focused()).is_some() {
                    focused = true;
                    break;
                }
            }
        });

        focused
    }

    /// Typing a channel value into a color row edits the color. The cells used
    /// to be read-only text, so the swatch's picker was the only way in, and
    /// setting exact RGB numbers was impossible (reported by a Halo CE modder).
    /// Drives real pointer and keyboard input through the whole value row, then
    /// applies whatever edit it queued the way the editor does.
    #[test]
    fn color_channels_are_typed_into_directly() {
        // Float ARGB: the first cell is alpha.
        let mut light = TagFile::new(crate::app::test_definition_path("haloce_mcc/light.json")).unwrap();
        let path = "color lower bound";
        let pending = type_into_first_value_cell(&light, path, "0.25", Marked::Editable);
        assert_eq!(pending.len(), 1, "one committed edit for the whole color");
        assert_eq!(pending[0].path, path);
        crate::core::document::apply::apply_field_edit(&mut light, path, &pending[0].input)
            .unwrap();
        match light.root().field_path(path).unwrap().value() {
            Some(TagFieldData::RealArgbColor(c)) => {
                assert_eq!((c.alpha, c.red, c.green, c.blue), (0.25, 0.0, 0.0, 0.0));
            }
            other => panic!("expected a real ARGB color, got {other:?}"),
        }

        // Packed ARGB: edited in the same 0-1 channels the row shows, stored as bytes.
        let mut hud =
            TagFile::new(crate::app::test_definition_path("haloce_mcc/grenade_hud_interface.json")).unwrap();
        let path = "override icon color";
        let pending = type_into_first_value_cell(&hud, path, "1", Marked::Editable);
        assert_eq!(pending.len(), 1);
        crate::core::document::apply::apply_field_edit(&mut hud, path, &pending[0].input).unwrap();
        match hud.root().field_path(path).unwrap().value() {
            Some(TagFieldData::ArgbColor(c)) => assert_eq!(c.0, 0xFF00_0000),
            other => panic!("expected a packed ARGB color, got {other:?}"),
        }
    }

    /// Click along a value row until a cell takes focus, replace its text with
    /// `text`, press Enter, and return the edits the row queued.
    /// A `sled` field's range comes from its definition, in H3 (where only the
    /// real carries it) and in Reach (where a `sled` custom field precedes it
    /// too); a `real_slider`'s comes from the range in its name.
    #[test]
    fn slider_ranges_come_from_sled_definitions_and_real_slider_names() {
        let root = locate_definitions_root();
        let slider_named = |game, group, name: &str| {
            let docs = crate::app::help::build_def_docs(&root, game, group);
            docs.all_entries()
                .find_map(|entry| match entry {
                    DefEntry::Field { clean_name, slider: Some(slider), .. } if clean_name == name => {
                        Some(*slider)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no slider for {name} in {group}"))
        };
        assert_eq!(
            slider_named(GameId::Halo3, "cinematic", "environment darken"),
            SliderRange { min: 0.0, max: 1.0, step: Some(0.01) }
        );
        assert_eq!(
            slider_named(GameId::HaloReach, "sound_mix", "output gain"),
            SliderRange { min: -64.0, max: 12.0, step: Some(0.005) }
        );
        let brightness = field_display_meta("brightness:[-1...1]");
        assert_eq!(
            slider_range(&brightness, &TagFieldData::RealSlider(0.0)),
            Some(SliderRange { min: -1.0, max: 1.0, step: None })
        );
        assert_eq!(slider_range(&brightness, &TagFieldData::Real(0.0)), None, "a plain real with a range hint");
    }

    /// The edits a light's radius row, drawn as a 0..1 slider stepped by 0.01,
    /// queues on each of `frames`.
    fn slider_row_edits(frames: &[Vec<egui::Event>], editable: bool) -> Vec<Vec<PendingFieldEdit>> {
        slider_row_frames(frames, editable, false, &RADIUS).0
    }

    /// A field drawn as a slider: its definition, path and range.
    struct SliderCase {
        definition: &'static str,
        path: &'static str,
        range: SliderRange,
    }

    const RADIUS: SliderCase = SliderCase {
        definition: "haloce_mcc/light.json",
        path: "radius",
        range: SliderRange { min: 0.0, max: 1.0, step: Some(0.01) },
    };

    /// Like [`slider_row_edits`], also returning the last frame's texts; with
    /// `focus_box`, the value box has keyboard focus on every frame.
    fn slider_row_frames(
        frames: &[Vec<egui::Event>],
        editable: bool,
        focus_box: bool,
        case: &SliderCase,
    ) -> (Vec<Vec<PendingFieldEdit>>, Vec<String>) {
        let mut texts = Vec::new();
        let tag = TagFile::new(crate::app::test_definition_path(case.definition)).unwrap();
        let ctx = egui::Context::default();
        let mut edits = Vec::new();
        with_test_edit_context(|edit| {
            edit.editable = editable;
            edit.group_tag = tag.group().tag;
            let box_id = edit.widget_id(("text", &format!("{}|{}", edit.tag_key, case.path)));
            for events in frames {
                if focus_box {
                    ctx.memory_mut(|memory| memory.request_focus(box_id));
                }
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(900.0, 200.0))),
                    events: events.clone(),
                    ..Default::default()
                };
                let output = crate::app::run_ui_test(&ctx, input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let field = tag.root().field_path(case.path).expect("the slider's field");
                        let value = field.value().expect("the slider's value");
                        let mut meta = field_display_meta(field.name());
                        meta.slider = Some(case.range);
                        draw_foundation_value_row(
                            ui, field, &meta, field.type_name(), &value,
                            &TagNameIndex::default(), 0, case.path, edit, None, 300.0,
                        );
                    });
                });
                edits.push(std::mem::take(edit.pending));
                texts = output
                    .shapes
                    .iter()
                    .filter_map(|clipped| match &clipped.shape {
                        egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                        _ => None,
                    })
                    .collect();
            }
        });
        (edits, texts)
    }

    fn press(x: f32, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: egui::Pos2::new(x, 12.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    fn moved(x: f32) -> egui::Event {
        egui::Event::PointerMoved(egui::Pos2::new(x, 12.0))
    }

    /// Where along the row a click first sets the value: the slider's left end.
    fn slider_left(editable: bool) -> Option<(f32, Vec<PendingFieldEdit>)> {
        (0..300).map(|step| step as f32 * 3.0).find_map(|x| {
            let frames = [vec![moved(x)], vec![press(x, true)], vec![press(x, false)]];
            let edits = slider_row_edits(&frames, editable);
            let edits = edits.into_iter().flatten().collect::<Vec<_>>();
            (!edits.is_empty()).then_some((x, edits))
        })
    }

    /// Clicking the slider sets the value at once, snapped to the step; a drag
    /// changes nothing until it ends, then commits once.
    #[test]
    fn a_slider_commits_snapped_values_once_a_drag_ends() {
        let (left, clicked) = slider_left(true).expect("no click along the row set a value");
        assert_eq!(clicked.len(), 1);
        let value: f32 = clicked[0].input.parse().unwrap();
        assert!((0.0..=1.0).contains(&value), "{value}");
        assert!(clicked[0].input.split('.').nth(1).is_none_or(|decimals| decimals.len() <= 2), "{:?}", clicked[0].input);

        let start = left + 10.0;
        let mut frames = vec![vec![moved(start)], vec![press(start, true)]];
        frames.extend((1..=10).map(|step| vec![moved(start + step as f32 * 9.0)]));
        frames.push(vec![press(start + 90.0, false)]);
        let edits = slider_row_edits(&frames, true);
        let (during, end) = edits.split_at(edits.len() - 1);
        assert!(during.iter().all(Vec::is_empty), "a drag committed before it ended");
        assert_eq!(end[0].len(), 1, "the drag's end committed {} edits", end[0].len());
        let dragged: f32 = end[0][0].input.parse().unwrap();
        assert!(dragged > value, "dragging right moved the value from {value} to {dragged}");
    }

    /// An integer's slider moves in whole numbers, whatever step its
    /// definition gives, and commits them as integers.
    #[test]
    fn an_integer_slider_commits_whole_numbers() {
        let case = SliderCase {
            definition: "haloce_mcc/actor_variant.json",
            path: "forced shader permutation",
            range: SliderRange { min: 0.0, max: 256.0, step: Some(0.005) },
        };
        let committed = (0..300).map(|step| step as f32 * 3.0).find_map(|x| {
            let start = x + 10.0;
            let mut frames = vec![vec![moved(x)], vec![press(x, true)], vec![press(x, false)]];
            frames.extend([vec![moved(start)], vec![press(start, true)], vec![moved(start + 47.0)]]);
            frames.push(vec![press(start + 47.0, false)]);
            let edits = slider_row_frames(&frames, true, false, &case).0;
            let edits = edits.into_iter().flatten().collect::<Vec<_>>();
            (edits.len() == 2).then_some(edits)
        });
        let committed = committed.expect("no click and drag along the row set the value twice");
        for edit in &committed {
            let value: i64 = edit.input.parse().unwrap_or_else(|_| panic!("{:?} is not a whole number", edit.input));
            assert!((0..=256).contains(&value), "{value}");
        }
        assert!(
            committed[1].input.parse::<i64>().unwrap() > committed[0].input.parse::<i64>().unwrap(),
            "dragging right moved {} to {}",
            committed[0].input,
            committed[1].input
        );
        let meta = FieldDisplayMeta { slider: Some(case.range), ..field_display_meta("count") };
        let step = slider_range(&meta, &TagFieldData::ShortInteger(0)).unwrap().step;
        assert_eq!(step, Some(1.0));
        assert_eq!(slider_value_text(12.34, step), "12", "a slider between whole numbers");
    }

    /// In a narrow pane the slider gives up width, so the value box and the
    /// unit after it stay inside the row instead of running past its edge.
    #[test]
    fn a_slider_row_fits_a_narrow_pane() {
        let tag = TagFile::new(crate::app::test_definition_path(RADIUS.definition)).unwrap();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        // Wide enough for the row with a shorter slider, too narrow for the
        // full one; and narrower than the row can be with its limits.
        for width in [620.0, 480.0] {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 300.0));
        let mut panel = egui::Rect::NOTHING;
        let mut output = None;
        with_test_edit_context(|edit| {
            for _ in 0..2 {
                let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
                output = Some(crate::app::run_ui_test(&ctx, input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        panel = ui.max_rect();
                        let field = tag.root().field_path(RADIUS.path).unwrap();
                        let value = field.value().unwrap();
                        let mut meta = field_display_meta(field.name());
                        meta.unit = Some("world units".to_owned());
                        meta.slider = Some(RADIUS.range);
                        draw_foundation_value_row(
                            ui, field, &meta, field.type_name(), &value,
                            &TagNameIndex::default(), 0, RADIUS.path, edit, None, 300.0,
                        );
                    });
                }));
            }
        });
        let output = output.unwrap();
        let value_box = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill == crate::app::ui_kit::foundation_input() => Some(rect.rect),
                _ => None,
            })
            .expect("the value box is painted");
        let unit = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == "world units" => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
            .expect("the unit is painted");
        assert!(value_box.right() <= panel.right() + 0.5, "{width}: the value box {value_box:?} runs past {panel:?}");
        assert!(unit.right() <= panel.right() + 0.5, "{width}: the unit {unit:?} runs past {panel:?}");
        }
    }

    /// A read-only slider row takes no clicks.
    #[test]
    fn a_read_only_slider_sets_nothing() {
        assert!(slider_left(false).is_none());
    }

    /// The box beside the slider takes any value, outside the range too,
    /// and the row says it is outside.
    #[test]
    fn a_slider_rows_box_takes_values_outside_the_range() {
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let frames = [
            vec![],
            vec![key(egui::Key::Backspace), key(egui::Key::Backspace), egui::Event::Text("5".to_owned())],
            vec![key(egui::Key::Enter)],
        ];
        let (edits, texts) = slider_row_frames(&frames, true, true, &RADIUS);
        let edits = edits.into_iter().flatten().collect::<Vec<_>>();
        assert_eq!(edits.len(), 1, "{:?}", edits.iter().map(|edit| &edit.input).collect::<Vec<_>>());
        assert_eq!(edits[0].input, "5");
        assert!(texts.iter().any(|t| t == "outside recommended range"), "{texts:?}");
    }

    #[test]
    fn data_sizes_read_as_foundation_writes_them() {
        assert_eq!(data_size_text(512), "Data size:  512 bytes");
        assert_eq!(data_size_text(1024), "Data size:  1024 bytes");
        assert_eq!(data_size_text(2444), "Data size:  2.39 KB    (2444 bytes)");
    }

    /// Frames of a Halo 3 shader include whose text is `stored`, drawn with
    /// `events` and, with `typing`, the text box focused. Returns each frame's
    /// committed edits and the last frame's painted texts.
    fn data_row_frames(
        stored: &[u8],
        editable: bool,
        frames: &[(bool, Vec<egui::Event>)],
    ) -> (Vec<Vec<PendingFieldEdit>>, Vec<String>) {
        let (edits, shapes) = data_row_shapes(stored, editable, frames);
        let texts = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        (edits, texts)
    }

    /// [`data_row_frames`], returning the last frame's shapes.
    fn data_row_shapes(
        stored: &[u8],
        editable: bool,
        frames: &[(bool, Vec<egui::Event>)],
    ) -> (Vec<Vec<PendingFieldEdit>>, Vec<egui::epaint::ClippedShape>) {
        let mut tag = TagFile::new(crate::app::test_definition_path("halo3_mcc/hlsl_include.json")).unwrap();
        tag.root_mut()
            .field_path_mut("include file")
            .unwrap()
            .set(TagFieldData::Data(stored.to_vec()))
            .unwrap();
        let ctx = egui::Context::default();
        let (mut edits, mut shapes) = (Vec::new(), Vec::new());
        with_test_edit_context(|edit| {
            edit.editable = editable;
            edit.group_tag = tag.group().tag;
            let box_id = edit.widget_id(("data_text", &format!("{}|include file", edit.tag_key)));
            for (typing, events) in frames {
                ctx.memory_mut(|memory| {
                    if *typing {
                        memory.request_focus(box_id);
                    } else {
                        memory.surrender_focus(box_id);
                    }
                });
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    events: events.clone(),
                    ..Default::default()
                };
                let output = crate::app::run_ui_test(&ctx, input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let field = tag.root().field_path("include file").unwrap();
                        let value = field.value().unwrap();
                        let meta = field_display_meta(field.name());
                        draw_foundation_value_row(
                            ui, field, &meta, field.type_name(), &value,
                            &TagNameIndex::default(), 0, "include file", edit, None, 300.0,
                        );
                    });
                });
                edits.push(std::mem::take(edit.pending));
                shapes = output.shapes;
            }
        });
        (edits, shapes)
    }

    fn hex_bytes(hex: &str) -> Vec<u8> {
        crate::core::document::value::decode_hex(hex).unwrap()
    }

    /// A shader include's text is shown under its size, with plain line
    /// breaks, and an edit is written back with the CRLF line ends and the NUL
    /// it was read with, once the box loses focus.
    #[test]
    fn text_data_is_shown_and_edited_as_text() {
        let frames = [
            (true, vec![]),
            (true, vec![egui::Event::Text("x".to_owned())]),
            (false, vec![]),
        ];
        let (edits, texts) = data_row_frames(b"a\r\nb\0", true, &frames);
        assert!(texts.iter().any(|t| t == "Data size:  5 bytes"), "{texts:?}");
        assert!(edits[..2].iter().all(Vec::is_empty), "an edit committed while the box had focus");
        let committed: Vec<_> = edits.into_iter().flatten().collect();
        assert_eq!(committed.len(), 1);
        let bytes = hex_bytes(&committed[0].input);
        assert!(bytes.ends_with(b"\0") && bytes.windows(2).any(|pair| pair == b"\r\n"), "{bytes:?}");
        assert_eq!(bytes.iter().filter(|&&byte| byte == b'x').count(), 1, "{bytes:?}");
        assert!(!bytes.windows(2).any(|pair| pair[1] == b'\n' && pair[0] != b'\r'), "a bare LF in {bytes:?}");

        let (_, texts) = data_row_frames(b"a\r\nb\0", true, &[(false, vec![])]);
        assert!(texts.iter().any(|t| t == "a\nb"), "the text, with plain line breaks: {texts:?}");
    }

    /// Text data is laid out as the editors lay it out: a fixed-pitch face,
    /// one row per line however long, in a box that stops growing at
    /// Foundation's 200 and scrolls.
    #[test]
    fn text_data_is_monospaced_unwrapped_and_scrolls_past_its_height() {
        let line = "x".repeat(400);
        let text = vec![line.as_str(); 60].join("\r\n") + "\0";
        let (_, shapes) = data_row_shapes(text.as_bytes(), true, &[(false, vec![])]);
        let (galley, clip) = shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(shape) if shape.galley.text().starts_with("xxx") => {
                    Some((shape.galley.clone(), clipped.clip_rect))
                }
                _ => None,
            })
            .expect("the text is painted");
        assert_eq!(galley.job.sections[0].format.font_id.family, egui::FontFamily::Monospace);
        assert_eq!(galley.rows.len(), 60, "a long line wrapped");
        // The text is clipped a pixel inside the box's edge.
        assert!((clip.height() - DATA_TEXT_MAX_HEIGHT).abs() < 4.0, "the box is {} tall", clip.height());
    }

    #[test]
    fn read_only_text_data_takes_no_typing() {
        let frames = [(true, vec![]), (true, vec![egui::Event::Text("x".to_owned())]), (false, vec![])];
        let (edits, texts) = data_row_frames(b"a\0", false, &frames);
        assert!(edits.iter().all(Vec::is_empty));
        assert!(texts.iter().any(|t| t == "a"), "{texts:?}");
    }

    /// Text past the field's maximum size is refused, and the row says why.
    #[test]
    fn text_data_past_its_maximum_size_is_refused() {
        let frames = [
            (true, vec![]),
            (true, vec![egui::Event::Paste("x".repeat(262_140))]),
            (false, vec![]),
        ];
        let (edits, texts) = data_row_frames(b"\0", true, &frames);
        assert!(edits.iter().all(Vec::is_empty), "an oversized edit committed");
        assert!(texts.iter().any(|t| t.contains("at most 262140")), "{texts:?}");
    }

    /// How the field typed into is marked, and whether expert mode is on.
    #[derive(Clone, Copy)]
    enum Marked {
        Editable,
        EditableInExpertMode,
        ReadOnly,
        ReadOnlyInExpertMode,
    }

    #[test]
    fn fractional_hud_offsets_warn_then_round_when_committed() {
        for (text, expected) in [("200.5", 201), ("-200.5", -201), ("200.4", 200), ("0.4", 0)] {
            for route in ["enter", "blur", "save"] {
                let mut tag = TagFile::new_classic(
                    crate::app::test_definition_path("halo2_mcc/new_hud_definition.json"),
                    blam_tags::classic::ClassicEngine::Halo2V4,
                ).unwrap();
                crate::core::document::apply::apply_one_block_op(&mut tag, &BlockOp {
                    path: "bitmap widgets".into(), kind: BlockOpKind::Add,
                }).unwrap();
                let path = "bitmap widgets[0]/fullscreen offset";
                let ctx = egui::Context::default();
                with_test_edit_context(|edit| {
                    let buffer_key = format!("{}|{path}", edit.tag_key);
                    let key = format!("{buffer_key}|component|x");
                    let draft = edit.buffers.draft_mut(&key, "0");
                    draft.text = text.to_owned();
                    draft.changed = true;
                    let frame = |tag: &TagFile, events, edit: &mut FieldEditContext<'_>| {
                        crate::app::run_ui_test(&ctx, egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 250.0))),
                            events, ..Default::default()
                        }, |ui| {
                            egui::CentralPanel::default().show(ui, |ui| {
                                let field = tag.root().field_path(path).unwrap();
                                draw_foundation_value_row(ui, field, &field_display_meta(field.name()),
                                    field.type_name(), &field.value().unwrap(), &TagNameIndex::default(),
                                    0, path, edit, None, 300.0);
                            });
                        })
                    };
                    let output = frame(&tag, vec![], edit);
                    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                        egui::Shape::Rect(rect) if rect.fill == unapplied_value_fill())),
                        "fractional integer coordinates need an amber field");
                    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                        egui::Shape::Text(text) if text.galley.text().contains("round"))),
                        "rounding hints belong in the tooltip, not under the row");
                    ctx.memory_mut(|memory| memory.request_focus(edit.widget_id(("component", &buffer_key, "x"))));
                    frame(&tag, vec![], edit);
                    match route {
                        "enter" => { frame(&tag, vec![egui::Event::Key {
                            key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }], edit); }
                        "blur" => {
                            ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique("another field")));
                            frame(&tag, vec![], edit);
                        }
                        _ => {
                            let mut commits = edit.buffers.take_uncommitted(DraftFlush::All);
                            assert_eq!(commits.len(), 1);
                            edit.push_ops(commits.remove(0).1.unwrap());
                        }
                    }
                    assert_eq!(edit.pending.len(), 1, "{route}: {text}");
                    let pending = edit.pending.remove(0);
                    crate::core::document::apply::apply_field_edit(&mut tag, path, &pending.input).unwrap();
                    assert!(matches!(tag.root().field_path(path).unwrap().value(),
                        Some(TagFieldData::Point2d(point)) if point.x == expected && point.y == 0));
                    let output = frame(&tag, vec![], edit);
                    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                        egui::Shape::Rect(rect) if rect.fill == unapplied_value_fill())));
                });
            }
        }
    }

    #[test]
    fn halo2_hud_screen_offsets_edit_in_normal_and_expert_modes() {
        for block in ["bitmap widgets", "text widgets"] {
            for name in ["fullscreen offset", "halfscreen offset", "quarterscreen offset"] {
                for marked in [Marked::Editable, Marked::EditableInExpertMode] {
                    let mut tag = TagFile::new_classic(
                        crate::app::test_definition_path("halo2_mcc/new_hud_definition.json"),
                        blam_tags::classic::ClassicEngine::Halo2V4,
                    ).unwrap();
                    crate::core::document::apply::apply_one_block_op(&mut tag, &BlockOp {
                        path: block.to_owned(), kind: BlockOpKind::Add,
                    }).unwrap();
                    let path = format!("{block}[0]/{name}");
                    tag.root_mut().field_path_mut(&path).unwrap().set(
                        TagFieldData::Point2d(blam_tags::math::Point2d { x: 0, y: 17 }),
                    ).unwrap();
                    let pending = type_into_first_value_cell(&tag, &path, "-123", marked);
                    assert_eq!(pending.len(), 1, "{path} must accept an offset edit");
                    crate::core::document::apply::apply_field_edit(
                        &mut tag, &path, &pending[0].input,
                    ).unwrap();
                    assert!(matches!(tag.root().field_path(&path).unwrap().value(),
                        Some(TagFieldData::Point2d(point)) if point.x == -123 && point.y == 17),
                        "editing x must preserve y: {path}");
                    let field = tag.root().field_path(&path).unwrap();
                    for invalid in ["32768, 0", "0, -32769", "NaN, 0", "inf, 0", "1", "1, 2, 3", "1,,2", ",2"] {
                        assert!(crate::core::document::value::parse_gui_field_value(&field, invalid).is_err(), "{invalid}");
                    }
                    crate::core::document::apply::apply_field_edit(
                        &mut tag, &path, "-32768, 32767",
                    ).unwrap();
                    let bytes = tag.write_to_bytes().unwrap();
                    let reopened = blam_tags::classic::read_classic_tag_file(
                        &bytes,
                        blam_tags::layout::TagLayout::from_json(
                            crate::app::test_definition_path("halo2_mcc/new_hud_definition.json"),
                        ).unwrap(),
                    ).unwrap();
                    assert!(matches!(reopened.root().field_path(&path).unwrap().value(),
                        Some(TagFieldData::Point2d(point)) if point.x == i16::MIN && point.y == i16::MAX),
                        "integer offsets must survive saving: {path}");
                }
            }
        }
    }

    /// A field the definitions mark read-only is shown but not editable,
    /// until expert mode is on.
    #[test]
    fn expert_mode_edits_read_only_fields() {
        let light = TagFile::new(crate::app::test_definition_path("haloce_mcc/light.json")).unwrap();
        let path = "color lower bound";
        assert!(
            type_into_first_value_cell(&light, path, "0.25", Marked::ReadOnly).is_empty(),
            "a read-only field committed an edit outside expert mode"
        );
        let pending = type_into_first_value_cell(&light, path, "0.25", Marked::ReadOnlyInExpertMode);
        assert_eq!(pending.len(), 1, "a read-only field took no edit in expert mode");
        assert_eq!(pending[0].path, path);
    }

    fn type_into_first_value_cell(
        tag: &TagFile,
        path: &str,
        text: &str,
        marked: Marked,
    ) -> Vec<PendingFieldEdit> {
        let ctx = egui::Context::default();
        let mut pending = Vec::new();
        with_test_edit_context(|edit| {
            edit.expert_mode = matches!(marked, Marked::ReadOnlyInExpertMode | Marked::EditableInExpertMode);
            let frame = |events: Vec<egui::Event>, edit: &mut FieldEditContext<'_>| {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(900.0, 200.0))),
                    events,
                    ..Default::default()
                };
                let _ = crate::app::run_ui_test(&ctx, input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let field = tag.root().field_path(path).expect("color field");
                        let value = field.value().expect("color value");
                        let mut meta = field_display_meta(field.name());
                        meta.read_only = matches!(marked, Marked::ReadOnly | Marked::ReadOnlyInExpertMode);
                        draw_foundation_value_row(
                            ui, field, &meta, field.type_name(), &value,
                            &TagNameIndex::default(), 0, path, edit, None, 300.0,
                        );
                    });
                });
            };
            let key = |key, modifiers| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            };
            for step in 0..90 {
                let pointer = egui::Pos2::new(step as f32 * 10.0, 12.0);
                let click = |pressed| egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                frame(vec![egui::Event::PointerMoved(pointer), click(true), click(false)], edit);
                if ctx.memory(|memory| memory.focused()).is_some() {
                    break;
                }
            }
            assert!(ctx.memory(|memory| memory.focused()).is_some(), "no cell in the row took focus");
            frame(
                vec![
                    key(egui::Key::A, egui::Modifiers::COMMAND),
                    egui::Event::Text(text.to_owned()),
                ],
                edit,
            );
            assert!(edit.pending.is_empty(), "typing alone must not commit");
            frame(vec![key(egui::Key::Enter, egui::Modifiers::NONE)], edit);
            pending = std::mem::take(edit.pending);
        });
        pending
    }
}
