//! Foundation H3+ function controls and binary-backed fields.
//! It owns function decoding, visualization, and edit construction; applying edits to documents and unrelated shader layout belong elsewhere.

use super::*;

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

pub(in crate::app) const COLOR_GRAPH_OPTIONS: [(ColorGraphType, &str); 5] = [
    (ColorGraphType::Scalar, "scalar"),
    (ColorGraphType::OneColor, "1-color"),
    (ColorGraphType::TwoColor, "2-color"),
    (ColorGraphType::ThreeColor, "3-color"),
    (ColorGraphType::FourColor, "4-color"),
];

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

/// Which color graph types a field offers. Guerilla restricts them per field:
/// shader scalar animations are scalar-only, shader color animations 2/3/4-
/// color only, everything else all five.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum ColorTypeChoices {
    #[default]
    All,
    ScalarOnly,
    MultiColorOnly,
}

impl ColorTypeChoices {
    fn allows(self, kind: ColorGraphType) -> bool {
        match self {
            Self::All => true,
            Self::ScalarOnly => kind == ColorGraphType::Scalar,
            Self::MultiColorOnly => is_multi_color(kind),
        }
    }
}

fn is_multi_color(kind: ColorGraphType) -> bool {
    matches!(kind, ColorGraphType::TwoColor | ColorGraphType::ThreeColor | ColorGraphType::FourColor)
}

fn color_type_label(editor: &TagFunctionEditor, kind: ColorGraphType) -> &'static str {
    match editor.function().encoding() {
        FunctionEncoding::H2 => blam_tags::tag_function::h2::color_graph_type_name(kind),
        FunctionEncoding::Blob => COLOR_GRAPH_OPTIONS
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or("scalar", |(_, name)| *name),
    }
}

/// The color graph type, offering what the field allows. Moving between
/// color counts resamples the existing colors.
fn color_type_combo(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    choices: ColorTypeChoices,
    editable: bool,
) -> bool {
    let current = editor.color_graph_type();
    if !editable {
        foundation_input_cell(ui, color_type_label(editor, current), 120.0);
        return false;
    }
    let mut changed = false;
    egui::ComboBox::from_id_salt("function_color_type")
        .selected_text(color_type_label(editor, current))
        .width(120.0)
        .show_ui(ui, |ui| {
            for (kind, _) in COLOR_GRAPH_OPTIONS {
                if !choices.allows(kind) && kind != current {
                    continue;
                }
                if ui.selectable_label(kind == current, color_type_label(editor, kind)).clicked() && kind != current {
                    let resample = is_multi_color(current) && is_multi_color(kind);
                    changed |= if resample {
                        remap_editor_color_count(editor, kind)
                    } else {
                        editor.set_color_graph_type(kind).is_ok()
                    };
                }
            }
        });
    changed
}

/// Halo 2's type picker: the raw type list, as Guerilla shows it.
fn h2_function_type_combo(ui: &mut Ui, editor: &mut TagFunctionEditor, editable: bool) -> bool {
    use blam_tags::tag_function::h2::{FUNCTION_TYPES, function_type_name};
    let current = editor.function_type();
    let label = function_type_name(current);
    if !editable {
        foundation_input_cell(ui, label, 130.0);
        return false;
    }
    let mut changed = false;
    egui::ComboBox::from_id_salt("h2_function_type")
        .selected_text(label)
        .width(130.0)
        .show_ui(ui, |ui| {
            for kind in FUNCTION_TYPES {
                if ui.selectable_label(kind == current, function_type_name(kind)).clicked()
                    && kind != current
                    && editor.set_function_type(kind).is_ok()
                {
                    changed = true;
                }
            }
        });
    changed
}

/// The function editor, for every game: drawn in the f() window and, read-only,
/// as the inline preview in the tag editor.
pub(in crate::app) fn draw_function_editor(
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
        // The output enum is the H3+ render-method parameter type; Halo 2's
        // animation type is not part of the function.
        if editor.function().encoding() == FunctionEncoding::Blob {
            ui.label(RichText::new("Output:").color(text_dark()).small());
            changed |= output_type_combo(ui, &mut view.output_index, output_editable);
        }
        ui.label(RichText::new("Function type:").color(text_dark()).small());
        let retyped = match editor.function().encoding() {
            FunctionEncoding::Blob => foundation_master_type_combo(ui, &mut editor, editable),
            FunctionEncoding::H2 => h2_function_type_combo(ui, &mut editor, editable),
        };
        if retyped {
            *selected_graph = 0;
            *selected_point = 0;
            changed = true;
        }
        ui.label(RichText::new("Color:").color(text_dark()).small());
        changed |= color_type_combo(ui, &mut editor, view.color_types, editable);
    });

    let master = editor.master_type();
    if master == EngineMasterType::Basic {
        // A constant's value is its output range (min, and max when ranged).
        ui.add_space(8.0);
        changed |= draw_foundation_right_rail(ui, &mut editor, editable, color_popup);
        ui.add_space(6.0);
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
        if let Some((mut min, mut max)) = editor.clamp_range() {
            if labeled_drag(ui, "Max", &mut max, editable) {
                changed |= editor.set_clamp_range(min, max).is_ok();
            }
            if labeled_drag(ui, "Min", &mut min, editable) {
                changed |= editor.set_clamp_range(min, max).is_ok();
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
                            FunctionDraftColorTarget::Logical(index),
                            alpha,
                        ),
                    );
                }
            }
        }
    });
    changed
}

/// Returns whether the color graph type changed (Halo 2 functions refuse it).
fn remap_editor_color_count(editor: &mut TagFunctionEditor, target: ColorGraphType) -> bool {
    let old = (0..editor.color_count())
        .filter_map(|index| editor.get_color(index))
        .collect::<Vec<_>>();
    if editor.set_color_graph_type(target).is_err() {
        return false;
    }
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
    true
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

fn set_editor_flag(editor: &mut TagFunctionEditor, flag: u8, value: bool) {
    let mut function = editor.function().clone();
    if let Some(blob) = function.as_blob_mut() {
        blob.set_flag(flag, value);
        *editor = TagFunctionEditor::from_function(function);
    }
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
            let x_movable = editor.curve_point_x_movable(graph, *selected_point);
            if labeled_drag(ui, "X", &mut x, editable && x_movable)
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
                        && editor.curve_points_are_editable_structure()
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

    // Clamped/cyclic/exclusion are H3+ header flags; Halo 2 has none.
    if editor.function().encoding() == FunctionEncoding::H2 {
        return changed;
    }
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
                    editor.function().as_blob().map_or(0.0, BlobFunction::exclusion_min),
                    editor.function().as_blob().map_or(0.0, BlobFunction::exclusion_max)
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
    let names = editor.periodic_functions();
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
            let mut slot_changed = periodic_function_combo(column, slot, &mut params, names, editable);
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
    names: &[&str],
    editable: bool,
) -> bool {
    let current = params.function_index as usize;
    let label = names
        .get(current)
        .copied()
        .unwrap_or("unknown");
    let mut changed = false;
    egui::ComboBox::from_id_salt(("periodic_function", slot))
        .selected_text(label)
        .width(180.0)
        .show_ui(ui, |ui| {
            for (index, label) in names.iter().enumerate() {
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
    let names = editor.transition_functions();
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
            let mut slot_changed = transition_function_combo(column, slot, &mut params, names, editable);
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
    names: &[&str],
    editable: bool,
) -> bool {
    let current = params.function_index as usize;
    let label = names
        .get(current)
        .copied()
        .unwrap_or("unknown");
    let mut changed = false;
    egui::ComboBox::from_id_salt(("transition_function", slot))
        .selected_text(label)
        .width(160.0)
        .show_ui(ui, |ui| {
            for (index, label) in names.iter().enumerate() {
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
