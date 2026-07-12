//! Shared Foundation-styled controls, cells, and resource presentation.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

pub(in crate::app) fn foundation_label_cell(ui: &mut Ui, text: &str, help: Option<&str>) {
    let width = FOUNDATION_LABEL_WIDTH;
    let height = 24.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    // Reserve a gutter for the "?" documentation cue. Foundation always reserves
    // the space (the cue is Hidden, not Collapsed, when absent) so field names
    // stay aligned whether or not a field has a doc string.
    let gutter = 11.0;
    if help.is_some() {
        // The cue: a bold blue "?" left of the name (Foundation uses #3DA1CC).
        ui.painter().text(
            rect.left_center() + Vec2::new(2.0, 0.0),
            Align2::LEFT_CENTER,
            "?",
            bold_font(12.5),
            Color32::from_rgb(61, 161, 204),
        );
    }
    let shown = truncate_for_cell(text, width - gutter - 4.0);
    let truncated = shown != text;
    ui.painter().text(
        rect.left_center() + Vec2::new(gutter, 0.0),
        Align2::LEFT_CENTER,
        shown,
        FontId::proportional(12.5),
        text_dark(),
    );
    // Hovering the name (or the cue) shows the field documentation (prefixed with
    // the full name when the displayed label was truncated).
    let tip = match (help, truncated) {
        (Some(help), true) => Some(format!("{text}\n\n{help}")),
        (Some(help), false) => Some(help.to_owned()),
        (None, true) => Some(text.to_owned()),
        (None, false) => None,
    };
    if let Some(tip) = tip {
        response.on_hover_text(tip);
    }
}

pub(in crate::app) fn foundation_input_cell(ui: &mut Ui, text: &str, width: f32) {
    foundation_input_cell_colored(ui, text, width, text_dark(), None);
}

/// Like [`foundation_input_cell`] but with an explicit text color and an
/// optional hover tooltip override (used to flag missing tag references in red).
pub(in crate::app) fn foundation_input_cell_colored(
    ui: &mut Ui,
    text: &str,
    width: f32,
    color: Color32,
    hover: Option<&str>,
) {
    let height = 24.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    ui.painter().rect_filled(rect, 0.0, foundation_input());
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
    ui.painter().text(
        rect.left_center() + Vec2::new(5.0, 0.0),
        Align2::LEFT_CENTER,
        truncate_for_cell(text, width - 10.0),
        FontId::proportional(12.5),
        color,
    );
    if response.hovered() {
        response.on_hover_text(hover.unwrap_or(text));
    }
}

fn tag_reference_value_width(available: f32) -> f32 {
    available.min(520.0).max(300.0)
}

pub(in crate::app) fn shared_tag_reference_value_width(ui: &Ui, depth: usize) -> f32 {
    let indent = depth as f32 * 12.0;
    let available =
        (ui.available_width() - indent - FOUNDATION_LABEL_WIDTH - 260.0).clamp(220.0, 760.0);
    tag_reference_value_width(available)
}

fn tag_reference_icon_footprint() -> f32 {
    3.0 + 16.0 + 3.0
}

fn tag_reference_text_x(rect: egui::Rect) -> f32 {
    rect.left() + tag_reference_icon_footprint()
}

fn paint_tag_reference_value_cell(ui: &Ui, rect: egui::Rect, icon_group: Option<u32>) {
    ui.painter().rect_filled(rect, 0.0, foundation_input());
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
    paint_tag_reference_icon(ui, rect, icon_group);
}

fn paint_tag_reference_icon(ui: &Ui, rect: egui::Rect, icon_group: Option<u32>) {
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 3.0 + 8.0, rect.center().y),
        Vec2::splat(16.0),
    );
    paint_tag_icon_at(ui, icon_group, icon_rect);
}

pub(super) fn foundation_tag_reference_input_cell_colored(
    ui: &mut Ui,
    text: &str,
    width: f32,
    color: Color32,
    hover: Option<&str>,
    icon_group: Option<u32>,
) {
    let height = 24.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    paint_tag_reference_value_cell(ui, rect, icon_group);
    let text_left = tag_reference_text_x(rect);
    let text_width = (rect.right() - text_left - 5.0).max(12.0);
    ui.painter().text(
        egui::pos2(text_left, rect.center().y),
        Align2::LEFT_CENTER,
        truncate_for_cell(text, text_width),
        FontId::proportional(12.5),
        color,
    );
    if response.hovered() {
        response.on_hover_text(hover.unwrap_or(text));
    }
}

pub(super) fn foundation_tag_reference_text_edit_cell(
    ui: &mut Ui,
    text: &mut String,
    width: f32,
    id: egui::Id,
    icon_group: Option<u32>,
) -> egui::Response {
    let size = Vec2::new(width, 24.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let margin = egui::Margin {
        left: tag_reference_icon_footprint(),
        right: 4.0,
        top: 2.0,
        bottom: 2.0,
    };
    let response = ui
        .scope(|ui| {
            ui.visuals_mut().widgets.inactive.bg_fill = foundation_input();
            ui.visuals_mut().widgets.hovered.bg_fill = foundation_input();
            ui.visuals_mut().widgets.active.bg_fill = foundation_input();
            ui.visuals_mut().widgets.inactive.fg_stroke = Stroke::new(1.0, text_dark());

            ui.visuals_mut().widgets.hovered.fg_stroke = Stroke::new(1.0, text_dark());
            ui.visuals_mut().widgets.active.fg_stroke = Stroke::new(1.0, text_dark());
            ui.put(
                rect,
                egui::TextEdit::singleline(text)
                    .id(id)
                    .font(TextStyle::Monospace)
                    .text_color(text_dark())
                    .vertical_align(egui::Align::Center)
                    .margin(margin)
                    .desired_width(width)
                    .clip_text(true),
            )
        })
        .inner;
    paint_tag_reference_icon(ui, response.rect + margin, icon_group);
    text_edit_cursor_to_start_on_tab_focus(ui, &response);
    response
}

pub(in crate::app) fn foundation_text_edit_cell(
    ui: &mut Ui,
    text: &mut String,
    width: f32,
    id: egui::Id,
) -> egui::Response {
    let response = ui
        .scope(|ui| {
            ui.visuals_mut().widgets.inactive.bg_fill = foundation_input();
            ui.visuals_mut().widgets.hovered.bg_fill = foundation_input();
            ui.visuals_mut().widgets.active.bg_fill = foundation_input();
            ui.visuals_mut().widgets.inactive.fg_stroke = Stroke::new(1.0, text_dark());
            ui.visuals_mut().widgets.hovered.fg_stroke = Stroke::new(1.0, text_dark());
            ui.visuals_mut().widgets.active.fg_stroke = Stroke::new(1.0, text_dark());
            ui.add_sized(
                [width, 24.0],
                egui::TextEdit::singleline(text)
                    .id(id)
                    .font(TextStyle::Monospace)
                    .text_color(text_dark())
                    .vertical_align(egui::Align::Center)
                    .margin(Vec2::new(4.0, 2.0)),
            )
        })
        .inner;
    text_edit_cursor_to_start_on_tab_focus(ui, &response);
    response
}

pub(in crate::app) fn text_edit_cursor_to_start_on_tab_focus(ui: &Ui, response: &egui::Response) {
    if response.gained_focus() && ui.input(|input| input.key_pressed(egui::Key::Tab)) {
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(0),
                )));
            state.store(ui.ctx(), response.id);
        }
    }
}

pub(in crate::app) fn foundation_value_width(value: &str, available: f32) -> f32 {
    if value.len() > 48 {
        available
    } else if value.len() > 18 {
        available.min(520.0).max(300.0)
    } else {
        available.min(180.0).max(140.0)
    }
}

pub(in crate::app) fn flag_value_parts(value: &TagFieldData) -> Option<(u64, Vec<(u32, String)>)> {
    match value {
        TagFieldData::ByteFlags { value, names } => Some((*value as u64, names.clone())),

        TagFieldData::WordFlags { value, names } => Some((*value as u64, names.clone())),
        TagFieldData::LongFlags { value, names } => Some((*value as u32 as u64, names.clone())),
        TagFieldData::ByteBlockFlags(value) => Some((*value as u64, Vec::new())),
        TagFieldData::WordBlockFlags(value) => Some((*value as u64, Vec::new())),
        TagFieldData::LongBlockFlags(value) => Some((*value as u32 as u64, Vec::new())),
        _ => None,
    }
}

pub(in crate::app) fn foundation_value_parts(
    value: &TagFieldData,
) -> Option<Vec<(String, String)>> {
    let pair = |a: &str, av: String, b: &str, bv: String| {
        Some(vec![(a.to_owned(), av), (b.to_owned(), bv)])
    };
    let triple = |a: &str, av: String, b: &str, bv: String, c: &str, cv: String| {
        Some(vec![
            (a.to_owned(), av),
            (b.to_owned(), bv),
            (c.to_owned(), cv),
        ])
    };
    match value {
        TagFieldData::Point2d(p) => pair("x", p.x.to_string(), "y", p.y.to_string()),
        TagFieldData::Rectangle2d(r) => Some(vec![
            ("top".to_owned(), r.top.to_string()),
            ("left".to_owned(), r.left.to_string()),
            ("bottom".to_owned(), r.bottom.to_string()),
            ("right".to_owned(), r.right.to_string()),
        ]),
        TagFieldData::RealPoint2d(p) => pair("x", fmt_real(p.x), "y", fmt_real(p.y)),
        TagFieldData::RealPoint3d(p) => {
            triple("x", fmt_real(p.x), "y", fmt_real(p.y), "z", fmt_real(p.z))
        }
        TagFieldData::RealVector2d(v) => pair("i", fmt_real(v.i), "j", fmt_real(v.j)),
        TagFieldData::RealVector3d(v) => {
            triple("i", fmt_real(v.i), "j", fmt_real(v.j), "k", fmt_real(v.k))
        }
        TagFieldData::RealQuaternion(q) => Some(vec![
            ("i".to_owned(), fmt_real(q.i)),
            ("j".to_owned(), fmt_real(q.j)),
            ("k".to_owned(), fmt_real(q.k)),
            ("w".to_owned(), fmt_real(q.w)),
        ]),
        TagFieldData::RealEulerAngles2d(e) => {
            pair("yaw", fmt_real(e.yaw), "pitch", fmt_real(e.pitch))
        }
        TagFieldData::RealEulerAngles3d(e) => Some(vec![
            ("yaw".to_owned(), fmt_real(e.yaw)),
            ("pitch".to_owned(), fmt_real(e.pitch)),
            ("roll".to_owned(), fmt_real(e.roll)),
        ]),
        TagFieldData::RealPlane2d(p) => {
            triple("i", fmt_real(p.i), "j", fmt_real(p.j), "d", fmt_real(p.d))
        }
        TagFieldData::RealPlane3d(p) => Some(vec![
            ("i".to_owned(), fmt_real(p.i)),
            ("j".to_owned(), fmt_real(p.j)),
            ("k".to_owned(), fmt_real(p.k)),
            ("d".to_owned(), fmt_real(p.d)),
        ]),
        TagFieldData::ShortIntegerBounds(b) => {
            pair("low", b.lower.to_string(), "high", b.upper.to_string())
        }
        TagFieldData::AngleBounds(b)
        | TagFieldData::RealBounds(b)
        | TagFieldData::FractionBounds(b) => {
            pair("low", fmt_real(b.lower), "high", fmt_real(b.upper))
        }
        _ => None,
    }
}

pub(in crate::app) fn foundation_bounds_values(value: &TagFieldData) -> Option<(String, String)> {
    match value {
        TagFieldData::ShortIntegerBounds(b) => Some((b.lower.to_string(), b.upper.to_string())),
        TagFieldData::AngleBounds(b)
        | TagFieldData::RealBounds(b)
        | TagFieldData::FractionBounds(b) => Some((fmt_real(b.lower), fmt_real(b.upper))),
        _ => None,
    }
}

pub(in crate::app) fn foundation_editable_component_parts(
    value: &TagFieldData,
) -> Option<Vec<(String, String)>> {
    match value {
        TagFieldData::RealPoint2d(p) => Some(vec![
            ("x".to_owned(), fmt_real(p.x)),
            ("y".to_owned(), fmt_real(p.y)),
        ]),
        TagFieldData::RealPoint3d(p) => Some(vec![
            ("x".to_owned(), fmt_real(p.x)),
            ("y".to_owned(), fmt_real(p.y)),
            ("z".to_owned(), fmt_real(p.z)),
        ]),
        TagFieldData::RealVector2d(v) => Some(vec![
            ("i".to_owned(), fmt_real(v.i)),
            ("j".to_owned(), fmt_real(v.j)),
        ]),
        TagFieldData::RealVector3d(v) => Some(vec![
            ("i".to_owned(), fmt_real(v.i)),
            ("j".to_owned(), fmt_real(v.j)),
            ("k".to_owned(), fmt_real(v.k)),
        ]),
        TagFieldData::RealQuaternion(q) => Some(vec![
            ("i".to_owned(), fmt_real(q.i)),
            ("j".to_owned(), fmt_real(q.j)),
            ("k".to_owned(), fmt_real(q.k)),
            ("w".to_owned(), fmt_real(q.w)),
        ]),
        _ => None,
    }
}

/// Export a block's elements as tab-separated rows (header = leaf scalar field
/// names; one row per element). Nested block/struct fields are omitted (flat
/// export). Tabs/newlines in values are flattened to spaces so columns align.
pub(in crate::app) fn block_to_tsv(block: &TagBlock<'_>, names: &TagNameIndex) -> String {
    elements_to_tsv(block.len(), names, |index| block.element(index))
}

/// TSV export for a fixed-size array (read-only — arrays have no clipboard
/// snapshot, but their values can still be copied out).
pub(in crate::app) fn array_to_tsv(
    array: &blam_tags::TagArray<'_>,
    names: &TagNameIndex,
) -> String {
    elements_to_tsv(array.len(), names, |index| array.element(index))
}

/// Shared TSV body: header row of leaf scalar field names, one row per element.
fn elements_to_tsv<'a>(
    count: usize,
    names: &TagNameIndex,
    get: impl Fn(usize) -> Option<TagStruct<'a>>,
) -> String {
    let Some(first) = get(0) else {
        return String::new();
    };
    let is_leaf = |field: &TagField<'_>| {
        field.as_block().is_none() && field.as_struct().is_none() && field.value().is_some()
    };
    let columns: Vec<String> = first
        .fields()
        .filter(is_leaf)
        .map(|field| clean_field_name(field.name()))
        .collect();
    if columns.is_empty() {
        return String::new();
    }
    let mut out = columns.join("\t");
    for index in 0..count {
        out.push('\n');
        if let Some(element) = get(index) {
            let cells: Vec<String> = element
                .fields()
                .filter(is_leaf)
                .filter_map(|field| {
                    field.value().map(|value| {
                        format_foundation_scalar_value(names, &value).replace(['\t', '\n'], " ")
                    })
                })
                .collect();
            out.push_str(&cells.join("\t"));
        }
    }
    out
}

/// Leaf scalar columns of a block element as `(clean name, full stored name)`
/// pairs — the inverse of [`block_to_tsv`]'s header, used by TSV import to map a
/// pasted column header back to the field path segment to write.
pub(in crate::app) fn block_leaf_columns(block: &TagBlock<'_>) -> Vec<(String, String)> {
    let Some(first) = block.element(0) else {
        return Vec::new();
    };
    first
        .fields()
        .filter(|field| {
            field.as_block().is_none() && field.as_struct().is_none() && field.value().is_some()
        })
        .map(|field| (clean_field_name(field.name()), field.name().to_owned()))
        .collect()
}

pub(in crate::app) fn format_foundation_scalar_value(
    names: &TagNameIndex,
    value: &TagFieldData,
) -> String {
    match value {
        TagFieldData::Angle(v)
        | TagFieldData::Real(v)
        | TagFieldData::RealSlider(v)
        | TagFieldData::RealFraction(v) => fmt_real(*v),
        TagFieldData::RealRgbColor(c) => format!(
            "r {}  g {}  b {}",
            fmt_real(c.red),
            fmt_real(c.green),
            fmt_real(c.blue)
        ),
        TagFieldData::RealArgbColor(c) => format!(
            "a {}  r {}  g {}  b {}",
            fmt_real(c.alpha),
            fmt_real(c.red),
            fmt_real(c.green),
            fmt_real(c.blue)
        ),
        TagFieldData::RealHsvColor(c) => format!(
            "h {}  s {}  v {}",
            fmt_real(c.hue),
            fmt_real(c.saturation),
            fmt_real(c.value)
        ),
        TagFieldData::RealAhsvColor(c) => format!(
            "a {}  h {}  s {}  v {}",
            fmt_real(c.alpha),
            fmt_real(c.hue),
            fmt_real(c.saturation),
            fmt_real(c.value)
        ),
        _ => trim_formatted_value(&format_value(names, value, false)),
    }
}

pub(in crate::app) fn fmt_real(value: f32) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let truncated = (value * 100.0).trunc() / 100.0;
    let mut text = format!("{truncated:.2}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".to_owned() } else { text }
}

pub(in crate::app) fn is_hidden_non_expert_value(value: &TagFieldData, expert_mode: bool) -> bool {
    !expert_mode && matches!(value, TagFieldData::Custom(bytes) if bytes.is_empty())
}

pub(in crate::app) fn draw_resource(
    ui: &mut Ui,
    name: &str,
    resource: TagResource<'_>,
    names: &TagNameIndex,
    depth: usize,
    expert_mode: bool,
    path_prefix: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let kind = match resource.kind() {
        TagResourceKind::Null => "null",
        TagResourceKind::Exploded => "exploded",
        TagResourceKind::Xsync => "xsync",
    };
    draw_foundation_bar(
        ui,
        format!("{}    pageable resource ({kind})", clean_field_name(name)),
        depth,
        false,
        |ui| {
            draw_foundation_text_row(
                ui,
                "inline bytes",
                &hex_bytes(resource.inline_bytes()),
                "bytes",
                depth + 1,
            );
            if let Some(payload) = resource.exploded_payload() {
                draw_foundation_text_row(
                    ui,
                    "exploded payload",
                    &format!("{} bytes", payload.len()),
                    "bytes",
                    depth + 1,
                );
            }
            if let Some(payload) = resource.xsync_payload() {
                draw_foundation_text_row(
                    ui,
                    "xsync payload",
                    &format!("{} bytes", payload.len()),
                    "bytes",
                    depth + 1,
                );
            }
            if resource.xsync_state().is_some() {
                draw_foundation_text_row(
                    ui,
                    "hydration",
                    "hydrated from XSync state",
                    "xsync",
                    depth + 1,
                );
            }
            if let Some(nested) = resource.as_struct() {
                ui.separator();
                draw_struct_fields_inline(
                    ui,
                    nested,
                    names,
                    depth + 1,
                    expert_mode,
                    path_prefix,
                    edit,
                );
            }
        },
    );
}
