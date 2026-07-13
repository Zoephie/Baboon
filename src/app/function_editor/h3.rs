//! Foundation H3+ function controls and binary-backed fields.
//! It owns function decoding, visualization, and edit construction; applying edits to documents and unrelated shader layout belong elsewhere.

use super::*;

pub(in crate::app) fn is_editable_function_type(kind: FunctionType) -> bool {
    matches!(
        kind,
        FunctionType::Identity
            | FunctionType::Constant
            | FunctionType::Transition
            | FunctionType::Periodic
            | FunctionType::Linear
            | FunctionType::LinearKey
            | FunctionType::MultiLinearKey
            | FunctionType::Spline
            | FunctionType::MultiSpline
            | FunctionType::Exponent
            | FunctionType::Spline2
    )
}

pub(in crate::app) const EDITABLE_FUNCTION_TYPES: [FunctionType; 11] = [
    FunctionType::Identity,
    FunctionType::Constant,
    FunctionType::Transition,
    FunctionType::Periodic,
    FunctionType::Linear,
    FunctionType::LinearKey,
    FunctionType::MultiLinearKey,
    FunctionType::Spline,
    FunctionType::MultiSpline,
    FunctionType::Exponent,
    FunctionType::Spline2,
];

/// Curated function-input string_ids offered in the Input/Range combos.
/// The current value is always added if missing, and free text is
/// accepted, so this is only a convenience seed.
pub(in crate::app) const COMMON_FUNCTION_INPUTS: [&str; 7] = [
    "",
    "time",
    "frame",
    "random",
    "shield vitality",
    "change color primary",
    "distance to camera",
];

pub(in crate::app) const OUTPUT_TYPE_OPTIONS: [(i32, &str); 9] = [
    (0, "value"),
    (1, "color"),
    (2, "scale uniform"),
    (3, "scale x"),
    (4, "scale y"),
    (5, "translation x"),
    (6, "translation y"),
    (7, "frame index"),
    (8, "alpha"),
];

pub(in crate::app) const H2_OUTPUT_TYPE_OPTIONS: [(u8, &str); 5] = [
    (0, "scalar (intensity)"),
    (1, "scalar (alpha)"),
    (0x20, "2-color"),
    (0x40, "3-color"),
    (0x80, "4-color"),
];

pub(in crate::app) const H2_FUNCTION_TYPE_OPTIONS: [(u8, &str); 11] = [
    (0, "identity"),
    (1, "constant"),
    (2, "transition"),
    (3, "periodic"),
    (4, "linear"),
    (5, "linear key"),
    (6, "multi-linear key"),
    (7, "spline"),
    (8, "multi-spline"),
    (9, "exponent"),
    (10, "spline2"),
];

pub(in crate::app) const H2_EXPONENT_OPTIONS: [(u8, &str); 13] = [
    (0, "one"),
    (1, "zero"),
    (2, "cosine"),
    (3, "cosine variable"),
    (4, "diagonal wave"),
    (5, "diagonal wave variable"),
    (6, "slide"),
    (7, "slide variable"),
    (8, "noise"),
    (9, "jitter"),
    (10, "slide"),
    (11, "wander"),
    (12, "spark"),
];

pub(in crate::app) const H2_TRANSITION_EXPONENT_OPTIONS: [(u8, &str); 8] = [
    (0, "linear"),
    (1, "early"),
    (2, "late"),
    (3, "very early"),
    (4, "very late"),
    (5, "cosine"),
    (6, "zero"),
    (7, "one"),
];

pub(in crate::app) const COLOR_GRAPH_OPTIONS: [(ColorGraphType, &str); 5] = [
    (ColorGraphType::Scalar, "scalar"),
    (ColorGraphType::OneColor, "1-color"),
    (ColorGraphType::TwoColor, "2-color"),
    (ColorGraphType::ThreeColor, "3-color"),
    (ColorGraphType::FourColor, "4-color"),
];

pub(in crate::app) fn function_type_label(kind: FunctionType) -> String {
    match kind {
        FunctionType::LinearKey | FunctionType::MultiLinearKey => "curve".to_owned(),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}

/// Editable combo seeded from the curated list + current value, with a
/// free-text box for arbitrary string_ids. Returns whether `value`
/// changed.
pub(in crate::app) fn seeded_name_combo(
    ui: &mut Ui,
    id: &str,
    value: &mut String,
    editable: bool,
) -> bool {
    if !editable {
        foundation_input_cell(ui, if value.is_empty() { "none" } else { value }, 120.0);
        return false;
    }
    let mut changed = false;
    let mut options: Vec<String> = COMMON_FUNCTION_INPUTS
        .iter()
        .map(|s| s.to_string())
        .collect();
    if !value.is_empty() && !options.iter().any(|o| o == value) {
        options.push(value.clone());
    }
    let (_, wheel_delta) = combo_box_with_scroll(
        ui,
        egui::ComboBox::from_id_salt(id)
            .selected_text(if value.is_empty() {
                "none".to_owned()
            } else {
                value.clone()
            })
            .width(120.0),
        |ui| {
            for opt in &options {
                let label = if opt.is_empty() { "none" } else { opt.as_str() };
                if ui.selectable_label(value == opt, label).clicked() {
                    *value = opt.clone();
                    changed = true;
                }
            }
        },
    );
    if let Some(delta) = wheel_delta {
        let current = options.iter().position(|opt| opt == value).unwrap_or(0);
        if let Some(next) = combo_scroll_next_index(current, options.len(), delta) {
            *value = options[next].clone();
            changed = true;
        }
    }
    let response = ui.add(egui::TextEdit::singleline(value).desired_width(90.0));
    text_edit_cursor_to_start_on_tab_focus(ui, &response);
    if response.changed() {
        changed = true;
    }
    changed
}

pub(in crate::app) fn function_type_combo(
    ui: &mut Ui,
    function: &mut TagFunction,
    editable: bool,
) -> bool {
    let current = function.function_type();
    if !editable {
        foundation_input_cell(ui, &function_type_label(current), 130.0);
        return false;
    }
    let mut changed = false;
    let (_, wheel_delta) = combo_box_with_scroll(
        ui,
        egui::ComboBox::from_id_salt("fn_type")
            .selected_text(function_type_label(current))
            .width(130.0),
        |ui| {
            for kind in EDITABLE_FUNCTION_TYPES {
                if ui
                    .selectable_label(current == kind, function_type_label(kind))
                    .clicked()
                    && current != kind
                {
                    function.set_function_type(kind);
                    changed = true;
                }
            }
        },
    );
    if let Some(delta) = wheel_delta {
        let current_index = EDITABLE_FUNCTION_TYPES
            .iter()
            .position(|kind| *kind == current)
            .unwrap_or(0);
        if let Some(next) =
            combo_scroll_next_index(current_index, EDITABLE_FUNCTION_TYPES.len(), delta)
        {
            let kind = EDITABLE_FUNCTION_TYPES[next];
            function.set_function_type(kind);
            changed = true;
        }
    }
    changed
}

pub(in crate::app) fn output_type_combo(
    ui: &mut Ui,
    output_index: &mut Option<i32>,
    editable: bool,
) -> bool {
    let label = output_index
        .and_then(|i| {
            OUTPUT_TYPE_OPTIONS
                .iter()
                .find(|(v, _)| *v == i)
                .map(|(_, n)| *n)
        })
        .unwrap_or("—");
    if !editable {
        foundation_input_cell(ui, label, 120.0);
        return false;
    }
    let mut changed = false;
    let (_, wheel_delta) = combo_box_with_scroll(
        ui,
        egui::ComboBox::from_id_salt("fn_output")
            .selected_text(label)
            .width(120.0),
        |ui| {
            for (value, name) in OUTPUT_TYPE_OPTIONS {
                if ui
                    .selectable_label(*output_index == Some(value), name)
                    .clicked()
                    && *output_index != Some(value)
                {
                    *output_index = Some(value);
                    changed = true;
                }
            }
        },
    );
    if let Some(delta) = wheel_delta {
        let current_index = OUTPUT_TYPE_OPTIONS
            .iter()
            .position(|(value, _)| *output_index == Some(*value))
            .unwrap_or(0);
        if let Some(next) = combo_scroll_next_index(current_index, OUTPUT_TYPE_OPTIONS.len(), delta)
        {
            let value = OUTPUT_TYPE_OPTIONS[next].0;
            *output_index = Some(value);
            changed = true;
        }
    }
    changed
}

pub(in crate::app) fn color_graph_combo(
    ui: &mut Ui,
    function: &mut TagFunction,
    editable: bool,
) -> bool {
    let current = function.color_graph_type();
    let label = COLOR_GRAPH_OPTIONS
        .iter()
        .find(|(k, _)| *k == current)
        .map(|(_, n)| *n)
        .unwrap_or("scalar");
    if !editable {
        foundation_input_cell(ui, label, 90.0);
        return false;
    }
    let mut changed = false;
    let (_, wheel_delta) = combo_box_with_scroll(
        ui,
        egui::ComboBox::from_id_salt("fn_colorgraph")
            .selected_text(label)
            .width(90.0),
        |ui| {
            for (kind, name) in COLOR_GRAPH_OPTIONS {
                if ui.selectable_label(current == kind, name).clicked() && current != kind {
                    function.set_color_graph_type(kind);
                    changed = true;
                }
            }
        },
    );
    if let Some(delta) = wheel_delta {
        let current_index = COLOR_GRAPH_OPTIONS
            .iter()
            .position(|(kind, _)| *kind == current)
            .unwrap_or(0);
        if let Some(next) = combo_scroll_next_index(current_index, COLOR_GRAPH_OPTIONS.len(), delta)
        {
            let kind = COLOR_GRAPH_OPTIONS[next].0;
            function.set_color_graph_type(kind);
            changed = true;
        }
    }
    changed
}

fn master_label(master: EngineMasterType) -> &'static str {
    match master {
        EngineMasterType::Basic => "basic",
        EngineMasterType::Curve => "curve",
        EngineMasterType::Periodic => "periodic",
        EngineMasterType::Exponent => "exponent",
        EngineMasterType::Transition => "transition",
    }
}

fn foundation_master_type_combo(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
) -> bool {
    let current = editor.master_type();
    if !editable {
        foundation_input_cell(ui, master_label(current), 130.0);
        return false;
    }
    let mut changed = false;
    egui::ComboBox::from_id_salt("foundation_fn_type")
        .selected_text(master_label(current))
        .width(130.0)
        .show_ui(ui, |ui| {
            for master in [
                EngineMasterType::Basic,
                EngineMasterType::Curve,
                EngineMasterType::Periodic,
                EngineMasterType::Exponent,
                EngineMasterType::Transition,
            ] {
                if ui
                    .selectable_label(master == current, master_label(master))
                    .clicked()
                    && master != current
                    && editor.set_master_type(master).is_ok()
                {
                    changed = true;
                }
            }
        });
    changed
}

pub(in crate::app) fn draw_foundation_h3_function_editor_contents(
    ui: &mut Ui,
    view: &mut FunctionView,
    editable: bool,
    selected_graph: &mut usize,
    selected_point: &mut usize,
    color_popup: &mut Option<MaterialColorPopup>,
) -> bool {
    let mut changed = false;
    let mut editor = TagFunctionEditor::from_function(view.function.clone());
    let input_editable = editable
        && view
            .edit
            .as_ref()
            .is_some_and(|paths| !paths.input_name.is_empty());
    let range_editable = editable
        && view
            .edit
            .as_ref()
            .is_some_and(|paths| !paths.range_name.is_empty());
    let output_editable = editable
        && view
            .edit
            .as_ref()
            .is_some_and(|paths| !paths.parameter_type.is_empty());
    let time_editable = editable
        && view
            .edit
            .as_ref()
            .is_some_and(|paths| !paths.time_period.is_empty());

    ui.horizontal(|ui| {
        ui.label(RichText::new("Input:").color(text_dark()).small());
        changed |= seeded_name_combo(
            ui,
            "foundation_fn_input",
            &mut view.input_name,
            input_editable,
        );
        let mut ranged = editor.is_ranged();
        if ui
            .add_enabled(editable, egui::Checkbox::new(&mut ranged, "Range:"))
            .changed()
            && editor.set_ranged(ranged).is_ok()
        {
            *selected_graph = (*selected_graph).min(editor.graph_count().saturating_sub(1));
            changed = true;
        }
        if ranged {
            changed |= seeded_name_combo(
                ui,
                "foundation_fn_range",
                &mut view.range_name,
                range_editable,
            );
        } else {
            foundation_input_cell(ui, "none", 120.0);
        }
        ui.label(RichText::new("Output:").color(text_dark()).small());
        changed |= output_type_combo(ui, &mut view.output_index, output_editable);
        ui.label(RichText::new("Function type:").color(text_dark()).small());
        if foundation_master_type_combo(ui, &mut editor, editable) {
            *selected_graph = 0;
            *selected_point = 0;
            changed = true;
        }
    });

    let master = editor.master_type();
    if master == EngineMasterType::Basic {
        ui.add_space(8.0);
        if editor.color_graph_type() != ColorGraphType::Scalar {
            changed |= draw_foundation_right_rail(ui, &mut editor, editable, color_popup);
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("time period").color(text_dark()).small());
            changed |= ui
                .add_enabled(
                    time_editable,
                    egui::DragValue::new(&mut view.time_period_in_seconds)
                        .speed(0.1)
                        .range(0.0..=f32::MAX),
                )
                .changed();
            ui.label(RichText::new("seconds").color(subtle_dark()).small());
        });
        if changed {
            view.function = editor.into_function();
        }
        return changed;
    }

    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        changed |= draw_foundation_graph(ui, &mut editor, editable, selected_graph, selected_point);
        ui.add_space(8.0);
        changed |= draw_foundation_right_rail(ui, &mut editor, editable, color_popup);
    });

    ui.add_space(8.0);
    match master {
        EngineMasterType::Curve => {
            changed |= draw_curve_panel(ui, &mut editor, editable, selected_graph, selected_point)
        }
        EngineMasterType::Periodic => changed |= draw_periodic_panel(ui, &mut editor, editable),
        EngineMasterType::Exponent => changed |= draw_exponent_panel(ui, &mut editor, editable),
        EngineMasterType::Transition => changed |= draw_transition_panel(ui, &mut editor, editable),
        EngineMasterType::Basic => unreachable!(),
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("time period").color(text_dark()).small());
        changed |= ui
            .add_enabled(
                time_editable,
                egui::DragValue::new(&mut view.time_period_in_seconds)
                    .speed(0.1)
                    .range(0.0..=f32::MAX),
            )
            .changed();
        ui.label(RichText::new("seconds").color(subtle_dark()).small());
    });
    if changed {
        view.function = editor.into_function();
    }
    changed
}

fn draw_foundation_right_rail(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    color_popup: &mut Option<MaterialColorPopup>,
) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        if editor.color_graph_type() == ColorGraphType::Scalar {
            let mut min = editor.function().header().clamp_range_min;
            let mut max = editor.function().header().clamp_range_max;
            if labeled_drag(ui, "Max", &mut max, editable) {
                set_editor_clamp_range(editor, min, max);
                changed = true;
            }
            if labeled_drag(ui, "Min", &mut min, editable) {
                set_editor_clamp_range(editor, min, max);
                changed = true;
            }
        } else {
            for index in (0..editor.color_count()).rev() {
                let Some(argb) = editor.get_color(index) else {
                    continue;
                };
                let alpha = (argb >> 24) as u8;
                let color = color32_from_argb(argb);
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::splat(24.0),
                    if editable {
                        Sense::click()
                    } else {
                        Sense::hover()
                    },
                );
                ui.painter().rect_filled(rect, 0.0, color);
                ui.painter()
                    .rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
                if response.clicked() {
                    *color_popup = Some(
                        MaterialColorPopup::new(
                            &format!("Function color {}", index + 1),
                            color.r() as f32 / 255.0,
                            color.g() as f32 / 255.0,
                            color.b() as f32 / 255.0,
                            1.0,
                        )
                        .with_function_draft_color(
                            FunctionDraftColorTarget::H3Logical(index),
                            alpha,
                        ),
                    );
                }
            }
            if matches!(
                editor.color_graph_type(),
                ColorGraphType::TwoColor | ColorGraphType::ThreeColor | ColorGraphType::FourColor
            ) {
                let current = editor.color_graph_type();
                egui::ComboBox::from_id_salt("foundation_color_count")
                    .selected_text(color_count_label(current))
                    .width(90.0)
                    .show_ui(ui, |ui| {
                        for target in [
                            ColorGraphType::TwoColor,
                            ColorGraphType::ThreeColor,
                            ColorGraphType::FourColor,
                        ] {
                            if ui
                                .add_enabled(
                                    editable,
                                    egui::SelectableLabel::new(
                                        target == current,
                                        color_count_label(target),
                                    ),
                                )
                                .clicked()
                                && target != current
                            {
                                remap_editor_color_count(editor, target);
                                changed = true;
                            }
                        }
                    });
            }
        }
    });
    changed
}

fn color_count_label(kind: ColorGraphType) -> &'static str {
    match kind {
        ColorGraphType::TwoColor => "2-color",
        ColorGraphType::ThreeColor => "3-color",
        ColorGraphType::FourColor => "4-color",
        ColorGraphType::OneColor => "1-color",
        ColorGraphType::Scalar => "scalar",
    }
}

fn remap_editor_color_count(editor: &mut TagFunctionEditor, target: ColorGraphType) {
    let old = (0..editor.color_count())
        .filter_map(|index| editor.get_color(index))
        .collect::<Vec<_>>();
    editor.set_color_graph_type(target);
    let count = editor.color_count();
    for index in 0..count {
        let t = if count <= 1 {
            0.0
        } else {
            index as f32 / (count - 1) as f32
        };
        let argb = sample_argb_stops(&old, t);
        let _ = editor.set_color(index, argb);
    }
}

fn sample_argb_stops(stops: &[u32], t: f32) -> u32 {
    if stops.is_empty() {
        return 0;
    }
    if stops.len() == 1 {
        return stops[0];
    }
    let position = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let index = position.floor() as usize;
    let next = (index + 1).min(stops.len() - 1);
    let local = position - index as f32;
    let lerp = |shift: u32| {
        let a = ((stops[index] >> shift) & 0xff) as f32;
        let b = ((stops[next] >> shift) & 0xff) as f32;
        (a + (b - a) * local).round() as u32
    };
    (lerp(24) << 24) | (lerp(16) << 16) | (lerp(8) << 8) | lerp(0)
}

fn set_editor_clamp_range(editor: &mut TagFunctionEditor, min: f32, max: f32) {
    let mut function = editor.function().clone();
    function.set_clamp_range(min, max);
    *editor = TagFunctionEditor::from_function(function);
}

fn set_editor_flag(editor: &mut TagFunctionEditor, flag: u8, value: bool) {
    let mut function = editor.function().clone();
    function.set_flag(flag, value);
    *editor = TagFunctionEditor::from_function(function);
}

fn labeled_drag(ui: &mut Ui, label: &str, value: &mut f32, editable: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{label}:"))
                .color(text_dark())
                .small(),
        );
        changed = ui
            .add_enabled(
                editable,
                egui::DragValue::new(value).speed(0.01).max_decimals(5),
            )
            .changed();
    });
    changed
}

fn draw_curve_panel(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    selected_graph: &mut usize,
    selected_point: &mut usize,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        for graph in 0..editor.graph_count() {
            let label = if graph == 0 {
                "Graph 1 (green)"
            } else {
                "Graph 2 (red)"
            };
            ui.selectable_value(selected_graph, graph, label);
        }
    });
    let graph = (*selected_graph).min(editor.graph_count().saturating_sub(1));
    let point_count = editor.curve_control_point_count(graph).unwrap_or(0);
    *selected_point = (*selected_point).min(point_count.saturating_sub(1));
    if let Some((mut x, mut y)) = editor.curve_control_point(graph, *selected_point) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Selected point").color(text_dark()).strong());
            if labeled_drag(ui, "X", &mut x, editable)
                && editor
                    .set_curve_control_point(graph, *selected_point, (x, y))
                    .is_ok()
            {
                changed = true;
            }
            if labeled_drag(ui, "Y", &mut y, editable)
                && editor
                    .set_curve_control_point(graph, *selected_point, (x, y))
                    .is_ok()
            {
                changed = true;
            }
            if ui
                .add_enabled(
                    editable
                        && editor
                            .curve_is_graph_point(graph, *selected_point)
                            .unwrap_or(false),
                    egui::Button::new("Delete point"),
                )
                .clicked()
                && editor.delete_curve_point(graph, *selected_point).is_ok()
            {
                *selected_point = (*selected_point).saturating_sub(1);
                changed = true;
            }
        });
    }

    let segment_count = editor.curve_segment_count(graph).unwrap_or(0);
    ui.horizontal_wrapped(|ui| {
        for segment in 0..segment_count {
            let Some(current) = editor.curve_segment_type(graph, segment) else {
                continue;
            };
            ui.label(RichText::new(format!("Segment {}", segment + 1)).small());
            egui::ComboBox::from_id_salt(("curve_segment", graph, segment))
                .selected_text(curve_segment_label(current))
                .show_ui(ui, |ui| {
                    for target in [
                        CurveSegmentType::Linear,
                        CurveSegmentType::Spline,
                        CurveSegmentType::Spline2,
                    ] {
                        if ui
                            .add_enabled(
                                editable,
                                egui::SelectableLabel::new(
                                    target == current,
                                    curve_segment_label(target),
                                ),
                            )
                            .clicked()
                            && target != current
                            && editor
                                .set_curve_segment_type(graph, segment, target)
                                .is_ok()
                        {
                            changed = true;
                        }
                    }
                });
            if segment > 0 {
                let mode = editor
                    .curve_join_mode(graph, segment)
                    .unwrap_or(CurvePointMode::Corner);
                for target in [CurvePointMode::Corner, CurvePointMode::Smooth] {
                    if ui
                        .add_enabled(
                            editable,
                            egui::SelectableLabel::new(
                                mode == target,
                                if target == CurvePointMode::Corner {
                                    "corner"
                                } else {
                                    "smooth"
                                },
                            ),
                        )
                        .clicked()
                        && mode != target
                        && editor.set_curve_join_mode(graph, segment, target).is_ok()
                    {
                        changed = true;
                    }
                }
            }
        }
    });

    ui.horizontal(|ui| {
        let mut clamped = editor.is_clamped();
        if ui
            .add_enabled(editable, egui::Checkbox::new(&mut clamped, "clamped"))
            .changed()
        {
            set_editor_flag(editor, FunctionFlags::CLAMPED, clamped);
            changed = true;
        }
        let mut cyclic = editor.is_cyclic();
        if ui
            .add_enabled(editable, egui::Checkbox::new(&mut cyclic, "cyclic"))
            .changed()
        {
            set_editor_flag(editor, FunctionFlags::CYCLIC, cyclic);
            changed = true;
        }
        let mut exclusion = editor.is_exclusion();
        if ui
            .add_enabled(editable, egui::Checkbox::new(&mut exclusion, "exclusion"))
            .changed()
        {
            set_editor_flag(editor, FunctionFlags::EXCLUSION, exclusion);
            changed = true;
        }
        if exclusion {
            ui.label(
                RichText::new(format!(
                    "range {:.3} – {:.3}",
                    editor.function().exclusion_min(),
                    editor.function().exclusion_max()
                ))
                .color(subtle_dark())
                .small(),
            );
        }
    });
    changed
}

fn curve_segment_label(kind: CurveSegmentType) -> &'static str {
    match kind {
        CurveSegmentType::Linear => "linear",
        CurveSegmentType::Spline => "spline",
        CurveSegmentType::Spline2 => "spline2",
    }
}

fn draw_periodic_panel(ui: &mut Ui, editor: &mut TagFunctionEditor, editable: bool) -> bool {
    let mut changed = false;
    ui.columns(editor.graph_count(), |columns| {
        for (slot, column) in columns.iter_mut().enumerate() {
            let Some(mut params) = editor.periodic_params(slot) else {
                continue;
            };
            column.label(
                RichText::new(if slot == 0 {
                    "Function"
                } else {
                    "Function (range)"
                })
                .color(text_dark())
                .strong(),
            );
            let mut slot_changed = periodic_function_combo(column, slot, &mut params, editable);
            slot_changed |= labeled_drag(column, "Frequency", &mut params.frequency, editable);
            slot_changed |= labeled_drag(column, "Max", &mut params.amplitude_max, editable);
            slot_changed |= labeled_drag(column, "Phase", &mut params.phase, editable);
            slot_changed |= labeled_drag(column, "Min", &mut params.amplitude_min, editable);
            if slot_changed && editor.set_periodic_params(slot, params).is_ok() {
                changed = true;
            }
        }
    });
    changed
}

fn periodic_function_combo(
    ui: &mut Ui,
    slot: usize,
    params: &mut PeriodicParams,
    editable: bool,
) -> bool {
    let current = params.function_index as usize;
    let label = PERIODIC_FUNCTIONS
        .get(current)
        .copied()
        .unwrap_or("unknown");
    let mut changed = false;
    egui::ComboBox::from_id_salt(("periodic_function", slot))
        .selected_text(label)
        .width(180.0)
        .show_ui(ui, |ui| {
            for (index, label) in PERIODIC_FUNCTIONS.iter().enumerate() {
                if ui
                    .add_enabled(
                        editable,
                        egui::SelectableLabel::new(index == current, *label),
                    )
                    .clicked()
                {
                    params.function_index = index as u8;
                    changed = true;
                }
            }
        });
    changed
}

fn draw_exponent_panel(ui: &mut Ui, editor: &mut TagFunctionEditor, editable: bool) -> bool {
    let mut changed = false;
    ui.columns(editor.graph_count(), |columns| {
        for (slot, column) in columns.iter_mut().enumerate() {
            let Some(mut params) = editor.exponent_params(slot) else {
                continue;
            };
            column.label(
                RichText::new(if slot == 0 {
                    "Exponent"
                } else {
                    "Exponent (range)"
                })
                .color(text_dark())
                .strong(),
            );
            let slot_changed = labeled_drag(column, "Exponent", &mut params.exponent, editable)
                | labeled_drag(column, "Max", &mut params.amplitude_max, editable)
                | labeled_drag(column, "Min", &mut params.amplitude_min, editable);
            if slot_changed && editor.set_exponent_params(slot, params).is_ok() {
                changed = true;
            }
        }
    });
    changed
}

fn draw_transition_panel(ui: &mut Ui, editor: &mut TagFunctionEditor, editable: bool) -> bool {
    let mut changed = false;
    ui.columns(editor.graph_count(), |columns| {
        for (slot, column) in columns.iter_mut().enumerate() {
            let Some(mut params) = editor.transition_params(slot) else {
                continue;
            };
            column.label(
                RichText::new(if slot == 0 {
                    "Function"
                } else {
                    "Function (range)"
                })
                .color(text_dark())
                .strong(),
            );
            let mut slot_changed = transition_function_combo(column, slot, &mut params, editable);
            slot_changed |= labeled_drag(column, "Max", &mut params.amplitude_max, editable);
            slot_changed |= labeled_drag(column, "Min", &mut params.amplitude_min, editable);
            if slot_changed && editor.set_transition_params(slot, params).is_ok() {
                changed = true;
            }
        }
    });
    changed
}

fn transition_function_combo(
    ui: &mut Ui,
    slot: usize,
    params: &mut TransitionParams,
    editable: bool,
) -> bool {
    let current = params.function_index as usize;
    let label = TRANSITION_FUNCTIONS
        .get(current)
        .copied()
        .unwrap_or("unknown");
    let mut changed = false;
    egui::ComboBox::from_id_salt(("transition_function", slot))
        .selected_text(label)
        .width(160.0)
        .show_ui(ui, |ui| {
            for (index, label) in TRANSITION_FUNCTIONS.iter().enumerate() {
                if ui
                    .add_enabled(
                        editable,
                        egui::SelectableLabel::new(index == current, *label),
                    )
                    .clicked()
                {
                    params.function_index = index as u8;
                    changed = true;
                }
            }
        });
    changed
}

#[cfg(test)]
#[path = "../tests/function_editor_h3.rs"]
mod tests;
