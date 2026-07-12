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

/// Foundation's five user-facing master types.  Several on-disk curve forms
/// intentionally share the Curve presentation; the compact-editing API needed
/// to convert those forms without losing segments is supplied by blam-tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FoundationMasterType {
    Basic,
    Curve,
    Periodic,
    Exponent,
    Transition,
}

impl FoundationMasterType {
    pub(super) fn from_function_type(kind: FunctionType) -> Self {
        match kind {
            FunctionType::Constant => Self::Basic,
            FunctionType::Periodic => Self::Periodic,
            FunctionType::Exponent => Self::Exponent,
            FunctionType::Transition => Self::Transition,
            _ => Self::Curve,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Curve => "curve",
            Self::Periodic => "periodic",
            Self::Exponent => "exponent",
            Self::Transition => "transition",
        }
    }

    pub(super) fn target_function_type(self) -> Option<FunctionType> {
        match self {
            Self::Basic => Some(FunctionType::Constant),
            // Creating a valid MultiSpline needs the compact constructors and
            // derived-data rebuild support requested from blam-tags.
            Self::Curve => None,
            Self::Periodic => Some(FunctionType::Periodic),
            Self::Exponent => Some(FunctionType::Exponent),
            Self::Transition => Some(FunctionType::Transition),
        }
    }
}

pub(super) fn foundation_master_type_combo(
    ui: &mut Ui,
    function: &mut TagFunction,
    editable: bool,
) -> bool {
    let current = FoundationMasterType::from_function_type(function.function_type());
    if !editable {
        foundation_input_cell(ui, current.label(), 130.0);
        return false;
    }
    let mut changed = false;
    egui::ComboBox::from_id_salt("foundation_fn_type")
        .selected_text(current.label())
        .width(130.0)
        .show_ui(ui, |ui| {
            for master in [
                FoundationMasterType::Basic,
                FoundationMasterType::Curve,
                FoundationMasterType::Periodic,
                FoundationMasterType::Exponent,
                FoundationMasterType::Transition,
            ] {
                let response = ui.add_enabled(
                    master.target_function_type().is_some() || master == current,
                    egui::SelectableLabel::new(master == current, master.label()),
                );
                let response = if master == FoundationMasterType::Curve && master != current {
                    response.on_hover_text(
                        "Creating a curve is disabled until blam-tags can create a valid MultiSpline compact.",
                    )
                } else {
                    response
                };
                if response.clicked() && master != current {
                    if let Some(kind) = master.target_function_type() {
                        function.set_function_type(kind);
                        changed = true;
                    }
                }
            }
        });
    changed
}

/// The H3+ Foundation presentation.  It deliberately avoids raw compact-byte
/// mutation: controls which need the pending blam-tags compact API are shown
/// as read-only, while header, color, wrapper, and existing LinearKey edits
/// remain safe and update the preview live.
pub(in crate::app) fn draw_foundation_h3_function_editor_contents(
    ui: &mut Ui,
    view: &mut FunctionView,
    editable: bool,
    selected_point: &mut usize,
) -> bool {
    let mut changed = false;
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
        let mut ranged = view.function.flags().is_ranged();
        if ui
            .add_enabled(false, egui::Checkbox::new(&mut ranged, "Range:"))
            .on_hover_text("Ranged compact creation requires the pending blam-tags editing API.")
            .changed()
        {
            unreachable!("disabled range checkbox cannot change");
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
        changed |= foundation_master_type_combo(ui, &mut view.function, editable);
    });

    let master = FoundationMasterType::from_function_type(view.function.function_type());
    if master == FoundationMasterType::Basic {
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
        return changed;
    }

    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        // Do not let a curve drag silently replace MultiSpline/Spline/etc.
        // with LinearKey.  Existing LinearKey payloads retain their safe
        // point-editing path while other curve forms stay visually inspectable.
        let graph_editable = editable && view.function.linear_key_points().is_some();
        changed |=
            draw_function_graph_preview(ui, &mut view.function, graph_editable, selected_point);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            let is_color = view.function.color_graph_type() != ColorGraphType::Scalar;
            if is_color {
                changed |= draw_function_color_stop_editors(ui, &mut view.function, editable);
                let count_label = match view.function.color_graph_type() {
                    ColorGraphType::TwoColor => "2-color",
                    ColorGraphType::ThreeColor => "3-color",
                    ColorGraphType::FourColor => "4-color",
                    _ => "1-color",
                };
                foundation_input_cell(ui, count_label, 90.0);
            } else {
                let header = view.function.header();
                ui.label(RichText::new(format!("Max: {:.3}", header.clamp_range_max)).small());
                ui.label(RichText::new(format!("Min: {:.3}", header.clamp_range_min)).small());
            }
        });
    });

    ui.add_space(8.0);
    match master {
        FoundationMasterType::Curve => {
            ui.label(RichText::new("Curve").color(text_dark()).strong());
            if view.function.linear_key_points().is_some() {
                ui.label(
                    RichText::new("Drag/select points in the graph. Multi-segment controls are pending blam-tags compact editing support.")
                        .color(subtle_dark())
                        .small(),
                );
            } else {
                ui.label(
                    RichText::new("This curve form is displayed without conversion; segment editing is pending blam-tags compact editing support.")
                        .color(subtle_dark())
                        .small(),
                );
            }
        }
        FoundationMasterType::Periodic => draw_pending_compact_panel(
            ui,
            "Periodic",
            &["Function", "Frequency", "Max", "Phase", "Min"],
        ),
        FoundationMasterType::Exponent => {
            draw_pending_compact_panel(ui, "Exponent", &["Exponent", "Max", "Min"])
        }
        FoundationMasterType::Transition => {
            draw_pending_compact_panel(ui, "Transition", &["Function", "Max", "Min"])
        }
        FoundationMasterType::Basic => unreachable!(),
    }
    changed
}

pub(super) fn draw_pending_compact_panel(ui: &mut Ui, title: &str, fields: &[&str]) {
    ui.label(RichText::new(title).color(text_dark()).strong());
    ui.horizontal_wrapped(|ui| {
        for field in fields {
            ui.label(
                RichText::new(format!("{field}:"))
                    .color(subtle_dark())
                    .small(),
            );
            foundation_input_cell(ui, "—", 72.0);
        }
    });
    ui.label(
        RichText::new("Compact parameter editing (including the ranged second graph) awaits the required blam-tags API.")
            .color(subtle_dark())
            .small(),
    );
}
