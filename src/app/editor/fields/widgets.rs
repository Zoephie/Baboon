//! Shared Foundation-styled controls, cells, and resource presentation.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

/// Whether `response`'s widget gave up keyboard focus this frame.
///
/// egui 0.36 reports [`egui::Response::lost_focus`] on two frames in a row,
/// so that a focus taken mid-frame still reaches a widget drawn earlier.
/// Every caller here acts on the loss — commits an edit, resolves a path —
/// and must act once, so this keeps only the frame egui 0.29 reported: the
/// widget had focus when the frame began and has it no longer.
pub(in crate::app) fn lost_focus_once(response: &egui::Response) -> bool {
    response.lost_focus()
        && response
            .ctx
            .memory(|memory| memory.had_focus_last_frame(response.id))
}

/// Paint text through the original painter path unless this cell has a Find match.
pub(in crate::app) fn paint_findable_text(
    ui: &Ui,
    pos: egui::Pos2,
    anchor: Align2,
    text: &str,
    font_id: FontId,
    color: Color32,
    kind: FindTargetKind,
) {
    if !findable_text_has_match(ui, text, kind) {
        ui.painter().text(pos, anchor, text, font_id, color);
        return;
    }
    let galley = findable_galley(ui, text, font_id, color, kind);
    let rect = anchor.anchor_size(pos, galley.size());
    ui.painter().galley(rect.min, galley, color);
}

fn findable_galley(
    ui: &Ui,
    text: &str,
    font_id: FontId,
    color: Color32,
    kind: FindTargetKind,
) -> std::sync::Arc<egui::Galley> {
    let job = findable_layout_job(ui, text, font_id, color, kind);
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// Return whether the current Foundation cell has a rendered match of `kind`.
pub(in crate::app) fn findable_text_has_match(ui: &Ui, text: &str, kind: FindTargetKind) -> bool {
    findable_highlight_data(ui, text, kind).is_some()
}

/// Build highlighted widget text only when the current cell contains a Find match.
pub(in crate::app) fn highlighted_widget_text(
    ui: &Ui,
    text: &str,
    text_style: TextStyle,
    color: Color32,
    kind: FindTargetKind,
) -> Option<egui::WidgetText> {
    findable_text_has_match(ui, text, kind).then(|| {
        let font_id = ui.style().text_styles[&text_style].clone();
        findable_layout_job(ui, text, font_id, color, kind).into()
    })
}

/// Build highlighted italic widget text while preserving Find match styling.
pub(in crate::app) fn highlighted_italic_widget_text(
    ui: &Ui,
    text: &str,
    text_style: TextStyle,
    color: Color32,
    kind: FindTargetKind,
) -> Option<egui::WidgetText> {
    findable_text_has_match(ui, text, kind).then(|| {
        let font_id = ui.style().text_styles[&text_style].clone();
        let mut job = findable_layout_job(ui, text, font_id, color, kind);
        for section in &mut job.sections {
            section.format.italics = true;
        }
        job.into()
    })
}

fn findable_highlight_data(
    ui: &Ui,
    text: &str,
    kind: FindTargetKind,
) -> Option<(Vec<std::ops::Range<usize>>, Option<std::ops::Range<usize>>)> {
    let snapshot = ui.data(|data| {
        data.get_temp::<std::sync::Arc<FindRenderSnapshot>>(find_render_snapshot_id())
    })?;
    let cell = ui.data(|data| data.get_temp::<FindRenderCell>(find_render_cell_id()))?;
    if !snapshot
        .matching_cells
        .contains(&(cell.tag_key.clone(), cell.field_path.clone(), kind))
    {
        return None;
    }
    let ranges = find_text_ranges(
        text,
        &snapshot.query,
        snapshot.match_case,
        snapshot.whole_word,
    );
    if ranges.is_empty() {
        return None;
    }
    let active = snapshot.active.as_ref().and_then(|active| {
        (active.tag_key == cell.tag_key
            && active.field_path == cell.field_path
            && active.kind == kind)
            .then_some(active)
            .filter(|active| active.text == text)
            .map(|active| active.range.clone())
    });
    Some((ranges, active))
}

fn findable_layout_job(
    ui: &Ui,
    text: &str,
    font_id: FontId,
    color: Color32,
    kind: FindTargetKind,
) -> egui::text::LayoutJob {
    let Some((ranges, active)) = findable_highlight_data(ui, text, kind) else {
        let mut job = egui::text::LayoutJob::default();
        job.append(
            text,
            0.0,
            egui::TextFormat {
                font_id,
                color,
                ..Default::default()
            },
        );
        return job;
    };
    let mut job = egui::text::LayoutJob::default();
    let mut cursor = 0;
    for range in ranges {
        if cursor < range.start {
            job.append(
                &text[cursor..range.start],
                0.0,
                egui::TextFormat {
                    font_id: font_id.clone(),
                    color,
                    ..Default::default()
                },
            );
        }
        job.append(
            &text[range.clone()],
            0.0,
            egui::TextFormat {
                font_id: font_id.clone(),
                color: Color32::BLACK,
                background: if active.as_ref() == Some(&range) {
                    Color32::from_rgb(255, 168, 38)
                } else {
                    Color32::from_rgb(255, 220, 70)
                },
                ..Default::default()
            },
        );
        cursor = range.end;
    }
    if cursor < text.len() {
        job.append(
            &text[cursor..],
            0.0,
            egui::TextFormat {
                font_id,
                color,
                ..Default::default()
            },
        );
    }
    job
}

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
    paint_findable_text(
        ui,
        rect.left_center() + Vec2::new(gutter, 0.0),
        Align2::LEFT_CENTER,
        &shown,
        FontId::proportional(12.5),
        text_dark(),
        FindTargetKind::Label,
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
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, foundation_input());
    ui.painter()
        .rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0_f32, foundation_input_edge()),
            egui::StrokeKind::Middle,
        );
    let response = foundation_read_only_text_cell(ui, rect, text, color, 5.0);
    if response.hovered() {
        response.on_hover_text(hover.unwrap_or(text));
    }
}

/// An immutable TextBuffer keeps selection, focus and copying enabled without
/// permitting typing, paste, cut or deletion to alter the displayed value.
fn foundation_read_only_text_cell(
    ui: &mut Ui,
    rect: egui::Rect,
    text: &str,
    color: Color32,
    left_padding: f32,
) -> egui::Response {
    let mut text = text;
    let font_id = FontId::proportional(12.5);
    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
        let text = text.as_str();
        findable_galley(ui, text, font_id.clone(), color, FindTargetKind::Value)
    };
    let response = ui.put(
        rect,
        egui::TextEdit::singleline(&mut text)
            .frame(egui::Frame::NONE)
            .font(FontId::proportional(12.5))
            .text_color(color)
            .vertical_align(egui::Align::Center)
            .margin(egui::Margin {
                left: (left_padding) as i8,
                right: 5,
                top: 2,
                bottom: 2,
            })
            .clip_text(true)
            .layouter(&mut layouter),
    );
    text_edit_cursor_to_start_on_tab_focus(ui, &response);
    response
}

#[cfg(test)]
mod read_only_input_tests {
    use super::*;

    #[test]
    fn read_only_inputs_allow_selection_and_copy_but_reject_mutations() {
        let ctx = egui::Context::default();
        ctx.set_global_style(foundation_style());
        let value = "A long editing kit value that must be copied in full";
        let mut id = egui::Id::NULL;
        let mut frame = |events| {
            crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(120.0, 24.0), Sense::hover());
                        let response =
                            foundation_read_only_text_cell(ui, rect, value, text_dark(), 5.0);
                        id = response.id;
                        response.request_focus();
                        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(value.chars().count()),
                            )));
                        state.store(&ctx, id);
                    });
                },
            )
        };
        let _ = frame(vec![]);
        let copy = frame(vec![egui::Event::Copy]);
        assert_eq!(crate::app::copied_text(&copy.platform_output), value);
        let _ = frame(vec![
            egui::Event::Text("modified".to_owned()),
            egui::Event::Paste("pasted".to_owned()),
            egui::Event::Cut,
            egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let copy = frame(vec![egui::Event::Copy]);
        assert_eq!(crate::app::copied_text(&copy.platform_output), value);
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

fn paint_tag_reference_value_cell(ui: &Ui, rect: egui::Rect, icon_group: Option<u32>) {
    ui.painter().rect_filled(rect, 0.0, foundation_input());
    ui.painter()
        .rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0_f32, foundation_input_edge()),
            egui::StrokeKind::Middle,
        );
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
    show_hover_text: bool,
) -> egui::Response {
    let height = 24.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    paint_tag_reference_value_cell(ui, rect, icon_group);
    let response =
        foundation_read_only_text_cell(ui, rect, text, color, tag_reference_icon_footprint());
    if response.hovered() && show_hover_text {
        return response.on_hover_text(hover.unwrap_or(text));
    }
    response
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
        left: (tag_reference_icon_footprint()) as i8,
        right: 4,
        top: 2,
        bottom: 2,
    };
    let has_highlight = findable_text_has_match(ui, text, FindTargetKind::Value);
    let response = ui
        .scope(|ui| {
            ui.visuals_mut().widgets.inactive.bg_fill = foundation_input();
            ui.visuals_mut().widgets.hovered.bg_fill = foundation_input();
            ui.visuals_mut().widgets.active.bg_fill = foundation_input();
            ui.visuals_mut().widgets.inactive.fg_stroke = Stroke::new(1.0_f32, text_dark());

            ui.visuals_mut().widgets.hovered.fg_stroke = Stroke::new(1.0_f32, text_dark());
            ui.visuals_mut().widgets.active.fg_stroke = Stroke::new(1.0_f32, text_dark());
            let edit = egui::TextEdit::singleline(text)
                .id(id)
                .font(TextStyle::Monospace)
                .text_color(text_dark())
                .vertical_align(egui::Align::Center)
                .margin(margin)
                .desired_width(width)
                .clip_text(true);
            if has_highlight {
                let font_id = ui.style().text_styles[&TextStyle::Monospace].clone();
                let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
        let text = text.as_str();
                    findable_galley(
                        ui,
                        text,
                        font_id.clone(),
                        text_dark(),
                        FindTargetKind::Value,
                    )
                };
                ui.put(rect, edit.layouter(&mut layouter))
            } else {
                ui.put(rect, edit)
            }
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
    let has_highlight = findable_text_has_match(ui, text, FindTargetKind::Value);
    let response = ui
        .scope(|ui| {
            ui.visuals_mut().widgets.inactive.bg_fill = foundation_input();
            ui.visuals_mut().widgets.hovered.bg_fill = foundation_input();
            ui.visuals_mut().widgets.active.bg_fill = foundation_input();
            ui.visuals_mut().widgets.inactive.fg_stroke = Stroke::new(1.0_f32, text_dark());
            ui.visuals_mut().widgets.hovered.fg_stroke = Stroke::new(1.0_f32, text_dark());
            ui.visuals_mut().widgets.active.fg_stroke = Stroke::new(1.0_f32, text_dark());
            let edit = egui::TextEdit::singleline(text)
                .id(id)
                .font(TextStyle::Monospace)
                .text_color(text_dark())
                .vertical_align(egui::Align::Center)
                .margin(Vec2::new(4.0, 2.0));
            if has_highlight {
                let font_id = ui.style().text_styles[&TextStyle::Monospace].clone();
                let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
        let text = text.as_str();
                    findable_galley(
                        ui,
                        text,
                        font_id.clone(),
                        text_dark(),
                        FindTargetKind::Value,
                    )
                };
                ui.add_sized([width, 24.0], edit.layouter(&mut layouter))
            } else {
                ui.add_sized([width, 24.0], edit)
            }
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
        // Angles, so `fmt_angle` like every other angle-typed value. This path
        // draws the read-only copy of a field the editable path renders with
        // `foundation_editable_component_parts`; the two used to disagree, so a
        // euler field in a read-only tag showed radians while the same field in
        // a loose one showed degrees.
        TagFieldData::RealEulerAngles2d(e) => {
            pair("yaw", fmt_angle(e.yaw), "pitch", fmt_angle(e.pitch))
        }
        TagFieldData::RealEulerAngles3d(e) => Some(vec![
            ("yaw".to_owned(), fmt_angle(e.yaw)),
            ("pitch".to_owned(), fmt_angle(e.pitch)),
            ("roll".to_owned(), fmt_angle(e.roll)),
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
        TagFieldData::AngleBounds(b) => pair("low", fmt_angle(b.lower), "high", fmt_angle(b.upper)),
        TagFieldData::RealBounds(b) | TagFieldData::FractionBounds(b) => {
            pair("low", fmt_real(b.lower), "high", fmt_real(b.upper))
        }
        _ => None,
    }
}

pub(in crate::app) fn foundation_bounds_values(value: &TagFieldData) -> Option<(String, String)> {
    match value {
        TagFieldData::ShortIntegerBounds(b) => Some((b.lower.to_string(), b.upper.to_string())),
        TagFieldData::AngleBounds(b) => Some((fmt_angle(b.lower), fmt_angle(b.upper))),
        TagFieldData::RealBounds(b) | TagFieldData::FractionBounds(b) => {
            Some((fmt_real(b.lower), fmt_real(b.upper)))
        }
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
        // Euler angles are radians on disk too, and are edited in degrees.
        TagFieldData::RealEulerAngles2d(e) => Some(vec![
            ("yaw".to_owned(), fmt_angle(e.yaw)),
            ("pitch".to_owned(), fmt_angle(e.pitch)),
        ]),
        TagFieldData::RealEulerAngles3d(e) => Some(vec![
            ("yaw".to_owned(), fmt_angle(e.yaw)),
            ("pitch".to_owned(), fmt_angle(e.pitch)),
            ("roll".to_owned(), fmt_angle(e.roll)),
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
        // Radians on disk, degrees in the editor — see `fmt_angle`.
        TagFieldData::Angle(v) => fmt_angle(*v),
        TagFieldData::Real(v) | TagFieldData::RealSlider(v) | TagFieldData::RealFraction(v) => {
            fmt_real(*v)
        }
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

/// A real value as editable text.
///
/// The shortest decimal that reads back as the same `f32`, which is both tidy
/// (`0.3`, `70`, `50000`) and lossless. It used to truncate to two decimals — and
/// because this same text seeds the edit box, committing a field wrote the
/// truncation back: anything under 0.01 displayed as `0` and became `0` the moment
/// it was touched.
pub(in crate::app) fn fmt_real(value: f32) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let text = format!("{value}");
    if text == "-0" { "0".to_owned() } else { text }
}

/// How many significant digits an angle is shown to.
///
/// Guerilla's six, so a value edited in one tool and read in the other agrees
/// digit for digit.
const ANGLE_SIGNIFICANT_DIGITS: i32 = 6;

/// An angle-typed value as editable text, in whichever unit is selected.
///
/// Angle fields — `angle`, `angle_bounds`, `real_euler_angles_2d/3d` — hold
/// radians on disk, and every Halo tool presents them in degrees; the field names
/// themselves say `:degrees`. Showing the stored radians instead made a
/// `0.01 degrees` field read as `0` (0.000175 rad, below the old two-decimal
/// display) and, worse, made a `0.15` typed into a box labelled degrees mean
/// 0.15 *radians* — 8.59°, fifty-seven times what was asked for. So degrees are
/// the default, and [`crate::core::format::angles_in_degrees`] turns them off for
/// anyone who wants to see what is actually stored.
///
/// Degrees are rounded to six significant digits rather than round-tripped
/// exactly, because the conversion itself is inexact: 20° stored is
/// `0.34906584`, which comes back as 19.999998. Six digits shows that as `20`,
/// and repeated edits are stable. **Radians get no such rounding** — nothing is
/// converted, so the shortest decimal that reads back as the same `f32` is both
/// exact and stable, and rounding 0.15 rad to six digits of *radians* would be
/// a precision loss with nothing to buy it.
pub(in crate::app) fn fmt_angle(radians: f32) -> String {
    if !crate::core::format::angles_in_degrees() {
        return fmt_real(radians);
    }
    let degrees = radians.to_degrees();
    if !degrees.is_finite() {
        return degrees.to_string();
    }
    if degrees == 0.0 {
        return "0".to_owned();
    }
    // Enough decimals to leave six significant digits, whatever the magnitude:
    // 1440.00 for a big one, 0.00001 for a small one.
    let exponent = degrees.abs().log10().floor() as i32;
    let decimals = (ANGLE_SIGNIFICANT_DIGITS - 1 - exponent).clamp(0, 9) as usize;
    let mut text = format!("{degrees:.decimals$}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
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

/// Keep the popup's outer frame exactly as wide as its selector button.
pub(in crate::app) fn picker_popup_width(ui: &mut Ui, response: &egui::Response) {
    let margin = egui::Frame::popup(ui.style()).total_margin().sum().x;
    ui.set_width((response.rect.width() - margin).max(1.0));
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
}

/// Shared search field for entry pickers, with an inline clear control.
pub(in crate::app) fn picker_search_field(
    ui: &mut Ui,
    filter: &mut String,
    hint: &str,
    just_opened: bool,
) -> bool {
    let search = ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text(placeholder_text(hint))
            .desired_width((ui.available_width() - 32.0).max(1.0))
            .margin(egui::Margin {
                left: 4,
                right: 28,
                top: 2,
                bottom: 2,
            }),
    );
    let mut changed = search.changed();
    if just_opened {
        search.request_focus();
    }
    if !filter.is_empty() {
        // egui 0.36's response rect already includes the text edit's margin.
        let field_rect = search.rect;
        let clear = search_clear_control(ui, field_rect, search.id.with("clear_search"));
        if clear.clicked() {
            filter.clear();
            search.request_focus();
            changed = true;
        }
    }
    changed
}

/// Give results their own height budget instead of the popup's remembered
/// height. This lets a filtered popup grow again as the search is cleared.
pub(in crate::app) fn picker_results<R>(
    ui: &mut Ui,
    max_height: f32,
    filter_changed: bool,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let height = max_height.min((ui.ctx().content_rect().height() - 80.0).max(32.0));
    let rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), height));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        let mut scroll = ScrollArea::vertical()
            .id_salt("picker_results")
            .auto_shrink([false, true])
            .max_height(height);
        if filter_changed {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show(ui, add_contents).inner
    })
    .inner
}

/// Shared picker button with a muted total beside the dropdown arrow.
pub(in crate::app) fn picker_button(
    ui: &mut Ui,
    popup_id: egui::Id,
    label: &str,
    count: usize,
    width: f32,
    foreground: Color32,
    enabled: bool,
) -> egui::Response {
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let response = ui
        .scope(|ui| {
            if !enabled {
                ui.disable();
            }
            if open {
                ui.visuals_mut().widgets.inactive.weak_bg_fill =
                    ui.visuals().widgets.open.weak_bg_fill;
            }
            ui.add_sized(Vec2::new(width, BUTTON_HEIGHT), egui::Button::new(""))
        })
        .inner;
    let foreground = if !enabled || !ui.is_enabled() {
        ui.visuals().widgets.noninteractive.fg_stroke.color
    } else {
        foreground
    };
    let count_label = format!("({count})");
    let count_font = TextStyle::Small.resolve(ui.style());
    let count_width = ui
        .painter()
        .layout_no_wrap(count_label.clone(), count_font.clone(), foreground)
        .size()
        .x;
    ui.painter().text(
        response.rect.left_center() + Vec2::new(8.0, 0.0),
        Align2::LEFT_CENTER,
        truncate_for_cell(label, response.rect.width() - 42.0 - count_width),
        TextStyle::Button.resolve(ui.style()),
        foreground,
    );
    let arrow_rect = egui::Rect::from_center_size(
        egui::pos2(response.rect.right() - 12.0, response.rect.center().y),
        Vec2::splat(BUTTON_ICON_SIZE),
    );
    paint_button_icon_at(ui, ButtonIcon::Down, arrow_rect, foreground);
    let count_pos = egui::pos2(arrow_rect.left() - 5.0, response.rect.center().y);
    ui.painter().text(
        count_pos,
        Align2::RIGHT_CENTER,
        count_label,
        count_font,
        foreground.gamma_multiply(0.5),
    );
    ui.interact(
        egui::Rect::from_center_size(
            count_pos - egui::vec2(count_width * 0.5, 0.0),
            Vec2::new(count_width, response.rect.height()),
        ),
        popup_id.with("entry_count"),
        Sense::hover(),
    )
    .on_hover_text(format!("{count} entries"));
    response
}

/// The same edge-aligned, muted clear control for custom search pills and pickers.
pub(in crate::app) fn search_clear_control(
    ui: &mut Ui,
    field_rect: egui::Rect,
    id: egui::Id,
) -> egui::Response {
    let clear_rect = egui::Rect::from_center_size(
        egui::pos2(field_rect.right() - 10.0, field_rect.center().y),
        Vec2::splat(20.0),
    );
    search_clear_control_at(ui, clear_rect, id, 2.0)
}

pub(in crate::app) fn search_clear_control_at(
    ui: &mut Ui,
    clear_rect: egui::Rect,
    id: egui::Id,
    corner_radius: f32,
) -> egui::Response {
    let clear = ui
        .interact(clear_rect, id, Sense::click())
        .on_hover_text("Clear search");
    if clear.hovered() {
        ui.painter().rect_filled(
            clear_rect,
            corner_radius,
            ui.visuals().widgets.hovered.weak_bg_fill,
        );
    }
    let color = if is_dark_mode() {
        Color32::WHITE
    } else {
        Color32::BLACK
    }
    .gamma_multiply(0.5);
    let icon_rect = clear_rect.shrink((clear_rect.width() - 10.0) * 0.5);
    let stroke = Stroke::new(1.5_f32, color);
    ui.painter()
        .line_segment([icon_rect.left_top(), icon_rect.right_bottom()], stroke);
    ui.painter()
        .line_segment([icon_rect.right_top(), icon_rect.left_bottom()], stroke);
    clear
}

#[cfg(test)]
mod tests {
    use super::*;

    // Foundation unit tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    /// Named components, and each one shown in degrees: euler angles are stored
    /// in radians, and the editor edits degrees like every other Halo tool.
    #[test]
    fn euler_angles_use_editable_named_components() {
        let _units = crate::core::format::AngleUnitGuard::set(true);
        let parts = foundation_editable_component_parts(&TagFieldData::RealEulerAngles2d(
            blam_tags::math::RealEulerAngles2d {
                yaw: 45f32.to_radians(),
                pitch: (-90f32).to_radians(),
            },
        ))
        .unwrap();
        assert_eq!(
            parts,
            vec![
                ("yaw".to_owned(), "45".to_owned()),
                ("pitch".to_owned(), "-90".to_owned()),
            ]
        );

        let parts = foundation_editable_component_parts(&TagFieldData::RealEulerAngles3d(
            blam_tags::math::RealEulerAngles3d {
                yaw: (-0.65f32).to_radians(),
                pitch: 0.0,
                roll: 1.25f32.to_radians(),
            },
        ))
        .unwrap();
        assert_eq!(
            parts,
            vec![
                ("yaw".to_owned(), "-0.65".to_owned()),
                ("pitch".to_owned(), "0".to_owned()),
                ("roll".to_owned(), "1.25".to_owned()),
            ]
        );
    }

    // Editor unit and fixture tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    /// Angle-typed fields hold radians and are edited in degrees, as Guerilla
    /// presents them and as their own `:degrees` field names say.
    ///
    /// The numbers are the ones from the report that found this: an ODST weapon
    /// whose `minimum error` read `0.01` in Guerilla and `0` in Baboon, and which
    /// after typing `0.15` into Baboon read `8.59437` in Guerilla — a factor of
    /// 180/π, applied to every angle field in every game.
    #[test]
    fn angle_fields_are_edited_in_degrees_and_stored_in_radians() {
        use crate::app::editor::fields::{format_foundation_scalar_value, foundation_bounds_values};
        let _units = crate::core::format::AngleUnitGuard::set(true);
        let tag = TagFile::new(test_definition_path("haloreach_mcc/test_tag.json")).unwrap();
        let root = tag.root();
        let names = TagNameIndex::default();

        // Typing 0.15 means 0.15 degrees, not 0.15 radians.
        let angle = root.field("angle").unwrap();
        let TagFieldData::Angle(stored) = parse_gui_field_value(&angle, "0.15").unwrap() else {
            panic!("expected an angle");
        };
        assert!(
            (stored - 0.15f32.to_radians()).abs() < 1e-7,
            "0.15 degrees stored as {stored} radians"
        );

        // And the reverse, against Guerilla's own reading of the same tag: the
        // 0.15 radians Baboon used to write shows as 8.59437 degrees.
        assert_eq!(
            format_foundation_scalar_value(&names, &TagFieldData::Angle(0.15)),
            "8.59437"
        );
        // The value the reporter saw as `0` — 0.01 degrees is 0.000175 radians,
        // which the old two-decimal display rounded away entirely.
        assert_eq!(
            format_foundation_scalar_value(&names, &TagFieldData::Angle(0.01f32.to_radians())),
            "0.01"
        );

        // Bounds are two angles, and get the same treatment.
        let bounds = root.field("angle bounds").unwrap();
        let TagFieldData::AngleBounds(stored) =
            parse_gui_field_value(&bounds, "0.05..0.5").unwrap()
        else {
            panic!("expected angle bounds");
        };
        assert!((stored.lower - 0.05f32.to_radians()).abs() < 1e-7);
        assert!((stored.upper - 0.5f32.to_radians()).abs() < 1e-7);
        assert_eq!(
            foundation_bounds_values(&TagFieldData::AngleBounds(blam_tags::math::AngleBounds {
                lower: 0.25,
                upper: 2.0,
            })),
            Some(("14.3239".to_owned(), "114.592".to_owned())),
            "the other two values Guerilla showed for the same tag"
        );
    }

    /// With degrees turned off, an angle is shown and typed as the radians it
    /// actually holds — no conversion in either direction.
    ///
    /// The failure this guards against is a half-flipped switch: a display that
    /// still converted while the parser did not would divide every angle the
    /// user retyped by 57.3, silently, which is the exact bug that made degrees
    /// unconditional in the first place.
    #[test]
    fn angles_are_shown_and_typed_as_radians_when_degrees_are_off() {
        use crate::app::editor::fields::{format_foundation_scalar_value, foundation_bounds_values};
        let _units = crate::core::format::AngleUnitGuard::set(false);
        let tag = TagFile::new(test_definition_path("haloreach_mcc/test_tag.json")).unwrap();
        let root = tag.root();
        let names = TagNameIndex::default();

        // 0.15 typed now means 0.15 radians, stored verbatim.
        let angle = root.field("angle").unwrap();
        let TagFieldData::Angle(stored) = parse_gui_field_value(&angle, "0.15").unwrap() else {
            panic!("expected an angle");
        };
        assert_eq!(stored, 0.15, "radians mode must store what was typed");

        // The same value the degrees test reads as 8.59437.
        assert_eq!(
            format_foundation_scalar_value(&names, &TagFieldData::Angle(0.15)),
            "0.15"
        );

        let bounds = root.field("angle bounds").unwrap();
        let TagFieldData::AngleBounds(stored) = parse_gui_field_value(&bounds, "0.25..2").unwrap()
        else {
            panic!("expected angle bounds");
        };
        assert_eq!((stored.lower, stored.upper), (0.25, 2.0));
        assert_eq!(
            foundation_bounds_values(&TagFieldData::AngleBounds(blam_tags::math::AngleBounds {
                lower: 0.25,
                upper: 2.0,
            })),
            Some(("0.25".to_owned(), "2".to_owned())),
        );
    }

    /// Radians are not rounded to the six significant digits degrees get: there
    /// is no conversion to be inexact, so the shortest round-tripping decimal is
    /// both exact and already a fixed point. Rounding here would lose precision
    /// with nothing to buy it.
    #[test]
    fn radians_round_trip_exactly_rather_than_to_six_digits() {
        use crate::app::editor::fields::format_foundation_scalar_value;
        let _units = crate::core::format::AngleUnitGuard::set(false);
        let tag = TagFile::new(test_definition_path("haloreach_mcc/test_tag.json")).unwrap();
        let angle = tag.root().field("angle").unwrap();
        let names = TagNameIndex::default();

        // A value with more than six significant digits, which degrees mode
        // would round: 0.34906584 is 20 degrees.
        let mut value = 0.34906584f32;
        for step in 0..8 {
            let shown = format_foundation_scalar_value(&names, &TagFieldData::Angle(value));
            let TagFieldData::Angle(reparsed) = parse_gui_field_value(&angle, &shown).unwrap()
            else {
                panic!("expected an angle");
            };
            assert_eq!(
                reparsed, value,
                "step {step} changed {value} to {reparsed} via {shown:?}"
            );
            value = reparsed;
        }
    }

    /// Euler angles are angles, so they follow the unit too — in both the
    /// editable and the read-only renderer, which used to disagree.
    #[test]
    fn euler_angles_follow_the_unit_in_both_renderers() {
        use crate::app::editor::fields::{foundation_editable_component_parts, foundation_value_parts};
        let euler = TagFieldData::RealEulerAngles3d(blam_tags::math::RealEulerAngles3d {
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            roll: 0.0,
        });

        {
            let _units = crate::core::format::AngleUnitGuard::set(true);
            let editable = foundation_editable_component_parts(&euler).unwrap();
            let read_only = foundation_value_parts(&euler).unwrap();
            assert_eq!(editable[0].1, "90", "half pi is 90 degrees");
            assert_eq!(
                read_only, editable,
                "a read-only euler field used to show radians while an editable one showed degrees"
            );
        }

        let _units = crate::core::format::AngleUnitGuard::set(false);
        let editable = foundation_editable_component_parts(&euler).unwrap();
        let read_only = foundation_value_parts(&euler).unwrap();
        assert!(editable[0].1.starts_with("1.57"), "{editable:?}");
        assert_eq!(read_only, editable);
    }

    /// Editing an angle repeatedly must not walk it: degrees are shown to six
    /// significant digits, so the display has to be a fixed point of
    /// parse-then-format even though rad↔deg is not exact.
    #[test]
    fn angle_display_survives_repeated_edits() {
        use crate::app::editor::fields::format_foundation_scalar_value;
        let _units = crate::core::format::AngleUnitGuard::set(true);
        let tag = TagFile::new(test_definition_path("haloreach_mcc/test_tag.json")).unwrap();
        let field = tag.root().field("angle").unwrap();
        let names = TagNameIndex::default();

        for typed in [
            "0.01", "0.15", "20", "45", "70", "360", "1440", "-90", "8.59437",
        ] {
            let TagFieldData::Angle(stored) = parse_gui_field_value(&field, typed).unwrap() else {
                panic!("expected an angle");
            };
            let shown = format_foundation_scalar_value(&names, &TagFieldData::Angle(stored));
            assert_eq!(shown, typed, "{typed} degrees came back as {shown}");

            // And again, from what was shown — the fixed point that matters when a
            // field is opened, committed, reopened and committed again.
            let TagFieldData::Angle(second) = parse_gui_field_value(&field, &shown).unwrap() else {
                panic!("expected an angle");
            };
            assert_eq!(
                format_foundation_scalar_value(&names, &TagFieldData::Angle(second)),
                typed,
                "{typed} drifted on the second edit"
            );
        }
    }

    /// Only angle-typed fields convert. A `real` labelled `:degrees` — the ODST
    /// weapon's `distribution angle` is one — is already in whatever unit its name
    /// claims, which is why it was the one field in that group both tools agreed
    /// on.
    #[test]
    fn plain_reals_are_left_alone() {
        use crate::app::editor::fields::{format_foundation_scalar_value, foundation_bounds_values};
        let tag = TagFile::new(test_definition_path("haloreach_mcc/test_tag.json")).unwrap();
        let root = tag.root();
        let names = TagNameIndex::default();

        let real = root.field("real").unwrap();
        let TagFieldData::Real(stored) = parse_gui_field_value(&real, "0.15").unwrap() else {
            panic!("expected a real");
        };
        assert_eq!(stored, 0.15);
        assert_eq!(
            format_foundation_scalar_value(&names, &TagFieldData::Real(0.15)),
            "0.15"
        );

        let bounds = root.field("real bounds").unwrap();
        let TagFieldData::RealBounds(stored) = parse_gui_field_value(&bounds, "0.05..0.5").unwrap()
        else {
            panic!("expected real bounds");
        };
        assert_eq!((stored.lower, stored.upper), (0.05, 0.5));
        assert_eq!(
            foundation_bounds_values(&TagFieldData::RealBounds(blam_tags::math::RealBounds {
                lower: 0.25,
                upper: 2.0,
            })),
            Some(("0.25".to_owned(), "2".to_owned()))
        );
    }

    /// The editable text is what a commit writes back, so it cannot be a rounded
    /// version of the value. Two decimals meant every real under 0.01 displayed as
    /// `0` and became `0` the moment the field was touched.
    #[test]
    fn small_reals_are_not_displayed_as_zero() {
        use crate::app::editor::fields::fmt_real;
        for value in [0.001f32, 0.0001, 0.75, 1e-7, 0.123456, -0.005] {
            let shown = fmt_real(value);
            let parsed: f32 = shown.parse().expect("editable text parses back");
            assert_eq!(parsed, value, "{value} displayed as {shown}");
        }
        assert_eq!(fmt_real(0.0), "0");
        assert_eq!(fmt_real(-0.0), "0");
        assert_eq!(fmt_real(70.0), "70");
    }

    #[test]
    fn supported_scenario_schemas_define_object_rotation_as_euler_angles() {
        fn has_object_rotation(value: &serde_json::Value) -> bool {
            match value {
                serde_json::Value::Object(object) => {
                    (object.get("name").and_then(serde_json::Value::as_str) == Some("rotation")
                        && object.get("type").and_then(serde_json::Value::as_str)
                            == Some("real_euler_angles_3d"))
                        || object.values().any(has_object_rotation)
                }
                serde_json::Value::Array(values) => values.iter().any(has_object_rotation),
                _ => false,
            }
        }

        for game in [
            "haloce_mcc",
            "halo2_mcc",
            "halo2amp_mcc",
            "halo3_mcc",
            "halo3odst_mcc",
            "haloreach_mcc",
            "halo4_mcc",
            "haloce_evolved",
        ] {
            let path = test_definition_path(&format!("{game}/scenario.json"));
            let value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert!(
                has_object_rotation(&value),
                "{} has no real_euler_angles_3d object rotation",
                path.display()
            );
        }
    }

    #[test]
    fn block_to_tsv_exports_header_and_one_row_per_element() {
        let mut tag = TagFile::new("definitions/halo2_mcc/model.json").unwrap();
        let mut dirty = Dirty::default();
        for name in ["alpha", "beta"] {
            apply_model_variant_ops(
                &mut tag,
                vec![ModelVariantOp::Create {
                    name: name.to_owned(),
                    regions: Vec::new(),
                }],
                &mut dirty,
            );
        }
        let variants = tag
            .root()
            .field("variants")
            .and_then(|field| field.as_block())
            .unwrap();
        let tsv = super::block_to_tsv(&variants, &TagNameIndex::default());

        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines.len(), 3, "header + 2 element rows");
        assert!(
            lines[0].split('\t').any(|col| col == "name"),
            "header should include the `name` column"
        );
        assert!(tsv.contains("alpha") && tsv.contains("beta"));
    }
}
