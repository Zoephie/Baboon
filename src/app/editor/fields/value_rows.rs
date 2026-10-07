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
    // The target block of a block-index field, when it could be found among
    // the struct's siblings or ancestors; `None` for non-block-index fields and
    // unresolvable (custom) indices → numeric editor.
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
    if edit.can_edit(meta) && is_text_editable_value(value) {
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

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        draw_foundation_component_cells(ui, &parts, 76.0, path, editable, edit);

        let (rect, response) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
        ui.painter().rect_filled(rect, 2.0, swatch);
        ui.painter()
            .rect_stroke(
                rect,
                2.0,
                Stroke::new(1.0_f32, foundation_input_edge()),
                egui::StrokeKind::Middle,
            );
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
    let indent = depth as f32 * 12.0;
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let lower_key = format!("{buffer_key}|lower");
    let upper_key = format!("{buffer_key}|upper");
    let lower_id = edit.widget_id(("bounds_lower", &buffer_key));
    let upper_id = edit.widget_id(("bounds_upper", &buffer_key));
    let mut lower = edit.buffers.take(&lower_key, lower_value);
    let mut upper = edit.buffers.take(&upper_key, upper_value);

    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        let editable = edit.can_edit(meta);
        let lower_response = foundation_value_cell(ui, &mut lower.text, 92.0, lower_id, editable);
        ui.label(RichText::new("to").color(subtle_dark()).small());
        let upper_response = foundation_value_cell(ui, &mut upper.text, 92.0, upper_id, editable);
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
        draw_field_help(ui, meta);
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
    edit: &mut FieldEditContext<'_>,
) {
    let indent = depth as f32 * 12.0;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        let editable = edit.can_edit(meta);
        draw_foundation_component_cells(ui, parts, 92.0, path, editable, edit);
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });
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
        let draft = edit.buffers.take(&key, value);
        drafts.push((key, draft));
    }

    let mut responses = Vec::with_capacity(parts.len());
    for (index, ((label, _), (_, draft))) in parts.iter().zip(drafts.iter_mut()).enumerate() {
        if !label.is_empty() {
            ui.label(RichText::new(label.as_str()).color(subtle_dark()).small());
        }
        let response = foundation_value_cell(ui, &mut draft.text, width, ids[index], editable);
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
            edit.push_ops(field_edit_ops(path, &components_input(&texts)));
            for (_, draft) in &mut drafts {
                draft.mark_committed();
            }
        }
        if drafts.iter().any(|(_, draft)| draft.changed) {
            let path = path.to_owned();
            let commit = DraftCommit::new(
                edit.tag_key,
                drafts.iter().map(|(key, _)| key.clone()).collect(),
                move |texts| Ok(field_edit_ops(&path, &components_input(texts))),
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
    draw_foundation_meta_text_row(ui, &meta, value, suffix, depth);
}

pub(in crate::app) fn draw_foundation_meta_text_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    suffix: &str,
    depth: usize,
) {
    draw_foundation_labelled_cell_row(ui, meta, suffix, depth, |ui, available_width| {
        foundation_input_cell(ui, value, foundation_value_width(value, available_width));
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
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let id = edit.widget_id(("text", &buffer_key));
    let draft = edit.buffers.draft_mut(&buffer_key, value);
    draw_foundation_labelled_cell_row(ui, meta, suffix, depth, |ui, available_width| {
        let width = foundation_value_width(&draft.text, available_width);
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

/// The narrowest a text data field's box gets while there is room, and the
/// tallest it grows before it scrolls: Foundation's sizes.
const DATA_TEXT_MIN_WIDTH: f32 = 600.0;
const DATA_TEXT_MAX_HEIGHT: f32 = 200.0;

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
    let indent = depth as f32 * 12.0;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        ui.label(RichText::new(data_size_text(bytes.len())).color(text_dark()));
        draw_field_help(ui, meta);
    });
    let Some(definition) = data_definitions(edit.definitions_root, edit.game)
        .and_then(|definitions| definitions.of_field(&field))
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
        ui.add_space(indent + FOUNDATION_LABEL_WIDTH + ui.spacing().item_spacing.x);
        // Foundation's box is at least 600 wide and grows to 200 tall; Halo 2
        // Guerilla's is a fixed 540 by 128 in an 11-pixel fixed-pitch font.
        // Neither wraps.
        let width = DATA_TEXT_MIN_WIDTH.min(ui.available_width()).max(ui.available_width() - 8.0);
        egui::ScrollArea::both()
            .id_salt(id.with("scroll"))
            .auto_shrink([false, true])
            .max_width(width)
            .max_height(DATA_TEXT_MAX_HEIGHT)
            .show(ui, |ui| {
                let font = FontId::monospace(12.0);
                let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
                    findable_galley(ui, text.as_str(), font.clone(), text_dark(), FindTargetKind::Value)
                };
                let mut read_only = draft.text.as_str();
                let buffer: &mut dyn egui::TextBuffer = if editable { &mut draft.text } else { &mut read_only };
                let response = ui.add(
                    egui::TextEdit::multiline(buffer)
                        .id(id)
                        .font(FontId::monospace(12.0))
                        .desired_width(width - ui.spacing().scroll.bar_width - 8.0)
                        .desired_rows(1)
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
            ui.add_space(indent + FOUNDATION_LABEL_WIDTH + ui.spacing().item_spacing.x);
            ui.label(RichText::new(too_long.to_string()).color(REFERENCE_MISSING_COLOR).small());
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
        ui.label(limit(range.min));
        let mut position = draft.text.trim().parse::<f32>().unwrap_or(range.min);
        ui.spacing_mut().slider_width = 180.0;
        let slider = egui::Slider::new(&mut position, range.min..=range.max)
            .show_value(false)
            .clamping(egui::SliderClamping::Never);
        let slider = match range.step {
            Some(step) => slider.step_by(step as f64),
            None => slider,
        };
        let slid = ui.add_enabled(editable, slider);
        ui.label(limit(range.max));
        if slid.changed() {
            draft.text = slider_value_text(position, range.step);
        }
        draft.note_response(&slid);
        let typed = foundation_value_cell(ui, &mut draft.text, 92.0, id, editable);
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
fn draw_foundation_labelled_cell_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    suffix: &str,
    depth: usize,
    cell: impl FnOnce(&mut Ui, f32),
) {
    let indent = depth as f32 * 12.0;
    let suffix_reserve = if suffix.is_empty() { 0.0 } else { 96.0 };
    let available_value_width =
        (ui.available_width() - indent - FOUNDATION_LABEL_WIDTH - suffix_reserve - 28.0)
            .clamp(180.0, 920.0);
    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        cell(ui, available_value_width);
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });
}

/// Red used to flag tag references whose target file is missing on disk.
pub(in crate::app) const REFERENCE_MISSING_COLOR: Color32 = Color32::from_rgb(216, 92, 92);

#[cfg(test)]
mod tests {
    use super::*;

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
        ReadOnly,
        ReadOnlyInExpertMode,
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
            edit.expert_mode = matches!(marked, Marked::ReadOnlyInExpertMode);
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
                        meta.read_only = !matches!(marked, Marked::Editable);
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
