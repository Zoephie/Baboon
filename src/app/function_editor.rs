use super::*;

pub(super) fn is_editable_function_type(kind: FunctionType) -> bool {
    matches!(
        kind,
        FunctionType::Identity
            | FunctionType::Constant
            | FunctionType::Linear
            | FunctionType::LinearKey
            | FunctionType::MultiLinearKey
    )
}

pub(super) const EDITABLE_FUNCTION_TYPES: [FunctionType; 4] = [
    FunctionType::Identity,
    FunctionType::Constant,
    FunctionType::Linear,
    FunctionType::LinearKey,
];

/// Curated function-input string_ids offered in the Input/Range combos.
/// The current value is always added if missing, and free text is
/// accepted, so this is only a convenience seed.
pub(super) const COMMON_FUNCTION_INPUTS: [&str; 7] = [
    "",
    "time",
    "frame",
    "random",
    "shield vitality",
    "change color primary",
    "distance to camera",
];

pub(super) const OUTPUT_TYPE_OPTIONS: [(i32, &str); 9] = [
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

pub(super) const COLOR_GRAPH_OPTIONS: [(ColorGraphType, &str); 5] = [
    (ColorGraphType::Scalar, "scalar"),
    (ColorGraphType::OneColor, "1-color"),
    (ColorGraphType::TwoColor, "2-color"),
    (ColorGraphType::ThreeColor, "3-color"),
    (ColorGraphType::FourColor, "4-color"),
];

pub(super) fn function_type_label(kind: FunctionType) -> String {
    match kind {
        FunctionType::LinearKey | FunctionType::MultiLinearKey => "curve".to_owned(),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}

/// Editable combo seeded from the curated list + current value, with a
/// free-text box for arbitrary string_ids. Returns whether `value`
/// changed.
pub(super) fn seeded_name_combo(ui: &mut Ui, id: &str, value: &mut String, editable: bool) -> bool {
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
    egui::ComboBox::from_id_salt(id)
        .selected_text(if value.is_empty() {
            "none".to_owned()
        } else {
            value.clone()
        })
        .width(120.0)
        .show_ui(ui, |ui| {
            for opt in &options {
                let label = if opt.is_empty() { "none" } else { opt.as_str() };
                if ui.selectable_label(value == opt, label).clicked() {
                    *value = opt.clone();
                    changed = true;
                }
            }
        });
    let response = ui.add(egui::TextEdit::singleline(value).desired_width(90.0));
    text_edit_cursor_to_start_on_tab_focus(ui, &response);
    if response.changed() {
        changed = true;
    }
    changed
}

pub(super) fn function_type_combo(ui: &mut Ui, function: &mut TagFunction, editable: bool) -> bool {
    let current = function.function_type();
    if !editable {
        foundation_input_cell(ui, &function_type_label(current), 130.0);
        return false;
    }
    let mut changed = false;
    egui::ComboBox::from_id_salt("fn_type")
        .selected_text(function_type_label(current))
        .width(130.0)
        .show_ui(ui, |ui| {
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
        });
    changed
}

pub(super) fn output_type_combo(
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
    egui::ComboBox::from_id_salt("fn_output")
        .selected_text(label)
        .width(120.0)
        .show_ui(ui, |ui| {
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
        });
    changed
}

pub(super) fn color_graph_combo(ui: &mut Ui, function: &mut TagFunction, editable: bool) -> bool {
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
    egui::ComboBox::from_id_salt("fn_colorgraph")
        .selected_text(label)
        .width(90.0)
        .show_ui(ui, |ui| {
            for (kind, name) in COLOR_GRAPH_OPTIONS {
                if ui.selectable_label(current == kind, name).clicked() && current != kind {
                    function.set_color_graph_type(kind);
                    changed = true;
                }
            }
        });
    changed
}

/// The interactive function editor body. When `editable` is false every
/// control is shown read-only. Returns whether `view` changed this frame.
pub(super) fn draw_function_editor_contents(
    ui: &mut Ui,
    view: &mut FunctionView,
    editable: bool,
    selected_point: &mut usize,
) -> bool {
    let mut changed = false;
    let ftype = view.function.function_type();
    let type_editable = editable && is_editable_function_type(ftype);
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
        ui.label(RichText::new("Function type:").color(text_dark()).small());
        changed |= function_type_combo(ui, &mut view.function, editable);
        ui.add_space(8.0);
        ui.label(RichText::new("Input:").color(text_dark()).small());
        changed |= seeded_name_combo(ui, "fn_input", &mut view.input_name, input_editable);

        let mut ranged = view.function.flags().is_ranged();
        if ui
            .add_enabled(type_editable, egui::Checkbox::new(&mut ranged, ""))
            .changed()
        {
            view.function.set_flag(FunctionFlags::RANGE, ranged);
            changed = true;
        }
        ui.label(RichText::new("Range:").color(text_dark()).small());
        if ranged {
            changed |= seeded_name_combo(ui, "fn_range", &mut view.range_name, range_editable);
        } else {
            foundation_input_cell(ui, "NONE", 120.0);
        }

        ui.label(RichText::new("Output:").color(text_dark()).small());
        changed |= output_type_combo(ui, &mut view.output_index, output_editable);
        ui.label(RichText::new("Color:").color(text_dark()).small());
        changed |= color_graph_combo(ui, &mut view.function, type_editable);
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(shader_function_grid_text(&view.function))
            .color(text_dark())
            .small(),
    );
    ui.add_space(8.0);

    ui.horizontal_top(|ui| {
        // Pass `editable` (not `type_editable`) so ANY writable function
        // can be dragged. The graph converts non-key types to LinearKey
        // on the first drag via `ensure_editable_curve`.
        changed |= draw_function_graph_preview(ui, &mut view.function, editable, selected_point);
        ui.add_space(8.0);

        let is_color = view.function.color_graph_type() != ColorGraphType::Scalar;
        let mut high = view.function.header().clamp_range_max;
        let mut low = view.function.header().clamp_range_min;

        // Output-range axis: high at top, low at bottom (Guerilla style).
        // Only shown for scalar functions — for color graphs, clamp_range
        // bytes carry packed ARGB and are not a meaningful float range.
        if !is_color {
            ui.vertical(|ui| {
                if ui
                    .add_enabled(type_editable, egui::DragValue::new(&mut high).speed(0.01))
                    .changed()
                {
                    view.function.set_clamp_range(low, high);
                    changed = true;
                }
                ui.add_space(118.0);
                if ui
                    .add_enabled(type_editable, egui::DragValue::new(&mut low).speed(0.01))
                    .changed()
                {
                    view.function.set_clamp_range(low, high);
                    changed = true;
                }
            });
            ui.add_space(8.0);
        } else {
            ui.add_space(8.0);
        }

        // Readout + numeric x/y for the selected control point.
        let control_points = function_control_points(&view.function);
        let sel = (*selected_point).min(control_points.len().saturating_sub(1));
        let (sx, sy) = control_points.get(sel).copied().unwrap_or((0.0, 0.0));
        // For scalar functions, Y is the output-mapped value. For color
        // functions `clamp_range` bytes are ARGB bits, not float ranges,
        // so just show the normalised [0,1] shape position instead.
        let y_display = if is_color {
            sy
        } else {
            low + sy * (high - low)
        };
        let is_key = view.function.linear_key_points().is_some();
        let point_editable = type_editable && is_key;
        ui.vertical(|ui| {
            Frame::none()
                .fill(foundation_group_bg())
                .stroke(Stroke::new(1.0, foundation_input_edge()))
                .inner_margin(egui::Margin::same(6.0))
                .show(ui, |ui| {
                    ui.set_min_width(78.0);
                    ui.label(
                        RichText::new(format!("X: {sx:.2}"))
                            .color(text_dark())
                            .small(),
                    );
                    ui.label(
                        RichText::new(format!("Y: {y_display:.2}"))
                            .color(text_dark())
                            .small(),
                    );
                    if is_color {
                        let c = view.function.evaluate_color(sx, sx);
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!("R: {}", float_channel_to_u8(c.red)))
                                .color(text_dark())
                                .small(),
                        );
                        ui.label(
                            RichText::new(format!("G: {}", float_channel_to_u8(c.green)))
                                .color(text_dark())
                                .small(),
                        );
                        ui.label(
                            RichText::new(format!("B: {}", float_channel_to_u8(c.blue)))
                                .color(text_dark())
                                .small(),
                        );
                    }
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("x").color(subtle_dark()).small());
                let mut px = sx;
                if ui
                    .add_enabled(point_editable, egui::DragValue::new(&mut px).speed(0.01))
                    .changed()
                {
                    view.function
                        .set_linear_key_point(sel, px.clamp(0.0, 1.0), sy);
                    changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("y").color(subtle_dark()).small());
                let mut py = sy;
                if ui
                    .add_enabled(point_editable, egui::DragValue::new(&mut py).speed(0.01))
                    .changed()
                {
                    view.function
                        .set_linear_key_point(sel, sx, py.clamp(0.0, 1.0));
                    changed = true;
                }
            });
        });

        // Color stops (editable swatches) for N-color graphs.
        // Color editing is always permitted regardless of curve type
        // (you can change stop colors even on a non-editable multispline).
        if view.function.color_graph_type() != ColorGraphType::Scalar {
            ui.add_space(8.0);
            changed |= draw_function_color_stop_editors(ui, &mut view.function, editable);
        }
    });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("time period").color(text_dark()).small());
        if ui
            .add_enabled(
                time_editable,
                egui::DragValue::new(&mut view.time_period_in_seconds)
                    .speed(0.1)
                    .range(0.0..=f32::MAX),
            )
            .changed()
        {
            changed = true;
        }
        ui.label(RichText::new("seconds").color(subtle_dark()).small());
    });
    changed
}

/// Editable color swatches for the N populated color slots of a
/// color-graph function. Returns whether any color changed.
pub(super) fn draw_function_color_stop_editors(
    ui: &mut Ui,
    function: &mut TagFunction,
    editable: bool,
) -> bool {
    let slots = color_graph_slots(function.color_graph_type());
    if slots.is_empty() {
        return false;
    }
    let mut changed = false;
    ui.vertical(|ui| {
        // Render high-end color at top (last slot) and low-end at bottom
        // (first slot), matching Guerilla's layout (top = y=1, bottom = y=0).
        for &slot in slots.iter().rev() {
            let argb = function.header().colors[slot];
            let orig_alpha = (argb >> 24) as u8;
            let mut color = color32_from_argb(argb);
            ui.horizontal(|ui| {
                // Swatch: always use color_edit_button so it's clickable even
                // for non-key curve types — color stops are always editable.
                let resp = if editable {
                    ui.color_edit_button_srgba(&mut color)
                } else {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, color);
                    ui.painter()
                        .rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
                    resp
                };
                // Hex code label (#RRGGBB)
                ui.label(
                    RichText::new(format!(
                        "#{:02X}{:02X}{:02X}",
                        color.r(),
                        color.g(),
                        color.b()
                    ))
                    .color(subtle_dark())
                    .small()
                    .monospace(),
                );
                if resp.changed() {
                    // Preserve the original alpha byte; Halo function headers
                    // store these as "opaque ARGB" with alpha=0 meaning unused.
                    let new_argb = ((orig_alpha as u32) << 24)
                        | ((color.r() as u32) << 16)
                        | ((color.g() as u32) << 8)
                        | color.b() as u32;
                    function.set_color(slot, new_argb);
                    changed = true;
                }
            });
        }
    });
    changed
}

/// Draw the function curve and, for any editable function, allow
/// dragging the control points. Non-key functions become an editable
/// key curve on the first drag (seeded from their current shape).
/// Returns whether the function changed.
pub(super) fn draw_function_graph_preview(
    ui: &mut Ui,
    function: &mut TagFunction,
    editable: bool,
    selected_point: &mut usize,
) -> bool {
    let size = Vec2::new(440.0, 190.0);
    let sense = if editable {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let plot = rect.shrink2(Vec2::new(22.0, 18.0));
    let point_screen = |x: f32, y: f32| {
        egui::pos2(
            egui::lerp(plot.left()..=plot.right(), x.clamp(0.0, 1.0)),
            egui::lerp(plot.bottom()..=plot.top(), y.clamp(0.0, 1.0)),
        )
    };

    // --- Interaction first, so handles/line reflect this frame's edit. ---
    // Within HANDLE_HIT pixels of an existing handle: select/drag it.
    // Outside: add a new point (click) or add-and-drag (drag).
    const HANDLE_HIT: f32 = 14.0;

    let mut changed = false;
    if editable {
        // Snapshot handles before any mutation this frame.
        let hit_pts = function_control_points(function);

        let nearest_handle = |pos: egui::Pos2| -> Option<(usize, f32)> {
            hit_pts
                .iter()
                .enumerate()
                .map(|(i, &(x, y))| (i, point_screen(x, y).distance(pos)))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        };

        if let Some(pos) = response.interact_pointer_pos() {
            // First frame of a drag gesture.
            if response.drag_started() {
                match nearest_handle(pos) {
                    Some((i, d)) if d < HANDLE_HIT => {
                        *selected_point = i;
                    }
                    _ => {
                        // Empty area drag: convert and insert, then drag it.
                        ensure_editable_curve(function);
                        let nx = egui::remap_clamp(pos.x, plot.left()..=plot.right(), 0.0..=1.0);
                        let ny = egui::remap_clamp(pos.y, plot.bottom()..=plot.top(), 0.0..=1.0);
                        if let Some(idx) = function.insert_linear_key_point(nx, ny) {
                            *selected_point = idx;
                            changed = true;
                        }
                    }
                }
            }

            // Drag in progress: move the selected handle.
            if response.dragged() {
                let n = function.active_linear_key_point_count().max(1);
                *selected_point = (*selected_point).min(n - 1);
                let nx = egui::remap_clamp(pos.x, plot.left()..=plot.right(), 0.0..=1.0);
                let ny = egui::remap_clamp(pos.y, plot.bottom()..=plot.top(), 0.0..=1.0);
                function.set_linear_key_point(*selected_point, nx, ny);
                changed = true;
            }

            // Pure click (no drag): select near handle, or insert a new point.
            if response.clicked() {
                match nearest_handle(pos) {
                    Some((i, d)) if d < HANDLE_HIT => {
                        *selected_point = i;
                    }
                    _ => {
                        ensure_editable_curve(function);
                        let nx = egui::remap_clamp(pos.x, plot.left()..=plot.right(), 0.0..=1.0);
                        let ny = egui::remap_clamp(pos.y, plot.bottom()..=plot.top(), 0.0..=1.0);
                        if let Some(idx) = function.insert_linear_key_point(nx, ny) {
                            *selected_point = idx;
                            changed = true;
                        }
                    }
                }
            }
        }

        // Delete / Backspace while the pointer is over the graph removes
        // the currently selected handle (minimum 2 points kept).
        if response.hovered()
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            let n = function.active_linear_key_point_count();
            let i = (*selected_point).min(n.saturating_sub(1));
            if function.delete_linear_key_point(i) {
                let new_n = function.active_linear_key_point_count();
                if *selected_point >= new_n {
                    *selected_point = new_n.saturating_sub(1);
                }
                changed = true;
            }
        }
    }

    // --- Background, grid, normalized-shape curve. ---
    {
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, Color32::BLACK);
        if function.color_graph_type() == ColorGraphType::Scalar {
            painter.rect_filled(plot, 0.0, function_plot_bg());
        } else {
            draw_function_color_gradient_vertical(painter, plot, &function_color_stops(function));
        }
        painter.rect_stroke(plot, 0.0, Stroke::new(1.0, grid_line()));
        for i in 1..10 {
            let x = egui::lerp(plot.left()..=plot.right(), i as f32 / 10.0);
            painter.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                Stroke::new(1.0, function_grid_line()),
            );
            let y = egui::lerp(plot.bottom()..=plot.top(), i as f32 / 10.0);
            painter.line_segment(
                [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                Stroke::new(1.0, function_grid_line()),
            );
        }
        // Plot the normalized curve SHAPE (0..1), not the output-mapped
        // value — so curves with output ranges outside [0,1] (or
        // inverted, like high=-1/low=0) still show their real shape.
        let samples = (0..=80)
            .map(|i| {
                let x = i as f32 / 80.0;
                let y = function.evaluate_shape(x, x).clamp(0.0, 1.0);
                egui::pos2(
                    egui::lerp(plot.left()..=plot.right(), x),
                    egui::lerp(plot.bottom()..=plot.top(), y),
                )
            })
            .collect::<Vec<_>>();
        painter.add(egui::Shape::line(
            samples,
            Stroke::new(2.0, Color32::from_rgb(54, 132, 58)),
        ));
    }

    // --- Handles (recomputed after any edit). ---
    {
        let control_points = function_control_points(function);
        let painter = ui.painter();
        for (i, (x, y)) in control_points.iter().enumerate() {
            let point = point_screen(*x, *y);
            let selected = editable && i == *selected_point;
            let handle =
                egui::Rect::from_center_size(point, Vec2::splat(if selected { 9.0 } else { 7.0 }));
            painter.rect_filled(
                handle,
                0.0,
                if selected {
                    Color32::from_rgb(120, 220, 120)
                } else {
                    Color32::from_rgb(240, 240, 238)
                },
            );
            painter.rect_stroke(handle, 0.0, Stroke::new(1.0, Color32::BLACK));
        }
        painter.text(
            rect.left_bottom() + Vec2::new(6.0, -4.0),
            Align2::LEFT_BOTTOM,
            "0",
            FontId::proportional(11.0),
            text_dark(),
        );
        painter.text(
            rect.right_top() + Vec2::new(-6.0, 4.0),
            Align2::RIGHT_TOP,
            "1",
            FontId::proportional(11.0),
            text_dark(),
        );
    }

    if function.color_graph_type() != ColorGraphType::Scalar {
        let bar = egui::Rect::from_min_size(
            rect.left_bottom() + Vec2::new(28.0, 10.0),
            Vec2::new(330.0, 24.0),
        );
        draw_function_color_gradient_horizontal(ui.painter(), bar, &function_color_stops(function));
        ui.allocate_space(Vec2::new(0.0, 36.0));
    }

    changed
}

pub(super) fn function_control_points(function: &TagFunction) -> Vec<(f32, f32)> {
    match function.kind() {
        FunctionKind::LinearKey { .. } | FunctionKind::MultiLinearKey { .. } => {
            // Only return the active (non-padding) points. Trailing slots
            // that are bit-identical to the preceding slot are padding.
            let pts = function.linear_key_points().unwrap();
            let n = function.active_linear_key_point_count();
            pts[..n].to_vec()
        }
        FunctionKind::MultiSpline { compact, .. } => {
            // Expose the segment join points (the visible kinks) so each
            // one can be clicked and inspected.
            let mut result = vec![(0.0_f32, function.evaluate_shape(0.0, 0.0))];
            for part in &compact.parts {
                let x = part.ending_x.clamp(0.0, 1.0);
                result.push((x, function.evaluate_shape(x, x)));
            }
            result
        }
        _ => vec![
            (0.0, function.evaluate_shape(0.0, 0.0)),
            (1.0, function.evaluate_shape(1.0, 1.0)),
        ],
    }
}

/// Convert any non-key function into a 2-point LinearKey curve, seeding
/// the endpoints from the current normalised shape so the curve doesn't
/// visually jump. No-op if it's already a key curve. Slots 2 and 3 are
/// set to bit-identical copies of slot 1 so `active_lk_count` treats
/// them as padding.
pub(super) fn ensure_editable_curve(function: &mut TagFunction) {
    if function.linear_key_points().is_some() {
        return;
    }
    let y0 = function.evaluate_shape(0.0, 0.0).clamp(0.0, 1.0);
    let y1 = function.evaluate_shape(1.0, 1.0).clamp(0.0, 1.0);
    function.set_function_type(FunctionType::LinearKey);
    function.set_linear_key_point(0, 0.0, y0);
    function.set_linear_key_point(1, 1.0, y1);
    function.set_linear_key_point(2, 1.0, y1); // padding
    function.set_linear_key_point(3, 1.0, y1); // padding
}

/// The engine stores color stops at non-contiguous slots in the header
/// colors[4] array, defined by the IDA remap table `byte_140CDE670`:
///   0:[0,0,0,0]  1:[0,3,0,0]  2:[0,1,3,0]  3:[0,1,2,3]
/// Empirically verified from real tag data: TwoColor uses slots [0,3]
/// (colors[1] and colors[2] are always zero for TwoColor).
pub(super) fn color_graph_slots(cgt: ColorGraphType) -> &'static [usize] {
    match cgt {
        ColorGraphType::Scalar => &[],
        ColorGraphType::OneColor => &[0],
        ColorGraphType::TwoColor => &[0, 3],
        ColorGraphType::ThreeColor => &[0, 1, 3],
        ColorGraphType::FourColor => &[0, 1, 2, 3],
    }
}

pub(super) fn function_color_stops(function: &TagFunction) -> Vec<Color32> {
    let header = function.header();
    let slots = color_graph_slots(header.color_graph_type);
    let mut stops: Vec<Color32> = slots
        .iter()
        .map(|&i| color32_from_argb(header.colors[i]))
        .collect();
    if stops.is_empty() {
        let color = function.evaluate_color(0.0, 0.0);
        stops.push(Color32::from_rgb(
            float_channel_to_u8(color.red),
            float_channel_to_u8(color.green),
            float_channel_to_u8(color.blue),
        ));
    }
    if stops.len() == 1 {
        stops.push(stops[0]);
    }
    stops
}

pub(super) fn color32_from_argb(argb: u32) -> Color32 {
    // The alpha byte in Halo function ARGB color fields is typically 0
    // (unused/unset), not a transparency value. Force opaque for display.
    Color32::from_rgb(
        ((argb >> 16) & 0xFF) as u8,
        ((argb >> 8) & 0xFF) as u8,
        (argb & 0xFF) as u8,
    )
}

pub(super) fn draw_function_color_gradient_vertical(
    painter: &egui::Painter,
    rect: egui::Rect,
    stops: &[Color32],
) {
    // Reverse so stop[0] renders at the bottom (y=0, low output) and
    // stop[last] at the top (y=1, high output), matching Guerilla's layout.
    let reversed: Vec<Color32> = stops.iter().rev().cloned().collect();
    draw_function_color_gradient(painter, rect, &reversed, true);
}

pub(super) fn draw_function_color_gradient_horizontal(
    painter: &egui::Painter,
    rect: egui::Rect,
    stops: &[Color32],
) {
    draw_function_color_gradient(painter, rect, stops, false);
}

pub(super) fn draw_function_color_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    stops: &[Color32],
    vertical: bool,
) {
    let stops = if stops.is_empty() {
        &[Color32::BLACK, Color32::BLACK][..]
    } else {
        stops
    };
    let steps = if vertical {
        rect.height().round().max(1.0) as usize
    } else {
        rect.width().round().max(1.0) as usize
    }
    .min(256);
    for step in 0..steps {
        let t0 = step as f32 / steps as f32;
        let t1 = (step + 1) as f32 / steps as f32;
        let color = sample_color_stops(stops, t0);
        let strip = if vertical {
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), egui::lerp(rect.top()..=rect.bottom(), t0)),
                egui::pos2(rect.right(), egui::lerp(rect.top()..=rect.bottom(), t1)),
            )
        } else {
            egui::Rect::from_min_max(
                egui::pos2(egui::lerp(rect.left()..=rect.right(), t0), rect.top()),
                egui::pos2(egui::lerp(rect.left()..=rect.right(), t1), rect.bottom()),
            )
        };
        painter.rect_filled(strip, 0.0, color);
    }
}

pub(super) fn sample_color_stops(stops: &[Color32], t: f32) -> Color32 {
    if stops.len() == 1 {
        return stops[0];
    }
    let scaled = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let index = scaled.floor() as usize;
    let next = (index + 1).min(stops.len() - 1);
    let local = scaled - index as f32;
    lerp_color(stops[index], stops[next], local)
}

pub(super) fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |a: u8, b: u8| -> u8 {
        (a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0)).round() as u8
    };
    Color32::from_rgba_unmultiplied(
        lerp(a.r(), b.r()),
        lerp(a.g(), b.g()),
        lerp(a.b(), b.b()),
        lerp(a.a(), b.a()),
    )
}

#[derive(Clone)]
pub(super) enum FunctionDataStorage {
    DataField(String),
    Halo2ByteBlock(String),
}

impl FunctionDataStorage {
    pub(super) fn data_field_path(&self) -> Option<&str> {
        match self {
            Self::DataField(path) => Some(path),
            Self::Halo2ByteBlock(_) => None,
        }
    }
}

#[derive(Clone)]
pub(super) struct FunctionEditPaths {
    /// Backing storage for the raw `mapping_function` blob.
    pub(super) data: FunctionDataStorage,
    /// `type` — the Output enum (`RenderMethodAnimatedParameterType`).
    pub(super) parameter_type: String,
    /// `input name` — string_id.
    pub(super) input_name: String,
    /// `range name` — string_id.
    pub(super) range_name: String,
    /// `time period` — real (seconds).
    pub(super) time_period: String,
    /// Parent `animated parameters` block path — used to push a delete op.
    pub(super) block_path: String,
    /// Index of this animated parameter within `block_path`.
    pub(super) block_index: usize,
}

#[derive(Clone)]
pub(super) struct FunctionView {
    pub(super) function: TagFunction,
    pub(super) input_name: String,
    pub(super) range_name: String,
    /// Output enum index (`RenderMethodAnimatedParameterType`), when the
    /// view came from an animated parameter. Drives the Output dropdown
    /// and the wrapper write-back.
    pub(super) output_index: Option<i32>,
    pub(super) time_period_in_seconds: f32,
    /// Tag write targets. `None` when the function has no resolvable
    /// path (material parameter blocks, template summaries) → the editor
    /// renders read-only.
    pub(super) edit: Option<FunctionEditPaths>,
}

impl FunctionView {
    pub(super) fn from_function(function: TagFunction) -> Self {
        Self {
            function,
            input_name: String::new(),
            range_name: String::new(),
            output_index: None,
            time_period_in_seconds: 0.0,
            edit: None,
        }
    }

    pub(super) fn from_animated(
        animated: &RenderMethodAnimatedParameter,
        function: TagFunction,
    ) -> Self {
        Self {
            function,
            input_name: animated.input_name.clone(),
            range_name: animated.range_name.clone(),
            output_index: animated.parameter_type.and_then(|kind| {
                OUTPUT_TYPE_OPTIONS
                    .iter()
                    .find(|(_, name)| name.eq_ignore_ascii_case(kind.name()))
                    .map(|(value, _)| *value)
            }),
            time_period_in_seconds: animated.time_period_in_seconds,
            edit: None,
        }
    }

    pub(super) fn with_edit(mut self, paths: FunctionEditPaths) -> Self {
        self.edit = Some(paths);
        self
    }
}

#[derive(Clone)]
pub(super) struct FunctionPopup {
    /// The tag the function belongs to — edits target this tag's doc.
    tag_key: String,
    title: String,
    view: FunctionView,
    /// Whether the owning tag is writable (LE loose file). Read-only
    /// tags still open the dialog but disable the controls.
    editable: bool,
    /// Snapshot of the values last pushed as edits; lets us emit a
    /// `PendingFieldEdit` only when something actually changed.
    last_applied: FunctionSnapshot,
    /// Currently selected LinearKey control point (drag/x-y target).
    selected_point: usize,
}

impl FunctionPopup {
    pub(super) fn new(tag_key: String, title: String, view: FunctionView, editable: bool) -> Self {
        let last_applied = FunctionSnapshot::from_view(&view);
        Self {
            tag_key,
            title,
            view,
            editable,
            last_applied,
            selected_point: 0,
        }
    }
}

/// Values that map to writable tag fields. Compared frame-to-frame to
/// decide which `PendingFieldEdit`s to emit.
#[derive(Clone, PartialEq)]
pub(super) struct FunctionSnapshot {
    data: Vec<u8>,
    output_index: Option<i32>,
    input_name: String,
    range_name: String,
    time_period: f32,
}

impl FunctionSnapshot {
    pub(super) fn from_view(view: &FunctionView) -> Self {
        Self {
            data: view.function.to_bytes(),
            output_index: view.output_index,
            input_name: view.input_name.clone(),
            range_name: view.range_name.clone(),
            time_period: view.time_period_in_seconds,
        }
    }
}

/// Edits produced by the function dialog this frame, plus the tag they
/// belong to.
pub(super) struct FunctionEditBatch {
    pub(super) tag_key: String,
    pub(super) edits: Vec<PendingFieldEdit>,
    pub(super) data_ops: Vec<FunctionDataOp>,
}

/// Diff a view's current values against the last-applied snapshot and
/// build `PendingFieldEdit`s for the fields that changed. The blob is
/// hex-encoded into the string edit channel; wrapper fields use their
/// normal text representations.
pub(super) fn push_function_edit(
    paths: &FunctionEditPaths,
    prev: &FunctionSnapshot,
    view: &FunctionView,
) -> FunctionEditBatch {
    let mut edits = Vec::new();
    let mut data_ops = Vec::new();
    let data = view.function.to_bytes();
    if data != prev.data {
        match &paths.data {
            FunctionDataStorage::DataField(path) if !path.is_empty() => {
                edits.push(PendingFieldEdit {
                    path: path.clone(),
                    input: encode_hex(&data),
                });
            }
            FunctionDataStorage::Halo2ByteBlock(block_path) if !block_path.is_empty() => {
                data_ops.push(FunctionDataOp {
                    block_path: block_path.clone(),
                    data,
                });
            }
            _ => {}
        }
    }
    if view.output_index != prev.output_index && !paths.parameter_type.is_empty() {
        if let Some(index) = view.output_index {
            // Write the schema name (resolved by parse_enum_value) rather than
            // a raw integer, so the edit doesn't depend on wire-value order.
            let input = OUTPUT_TYPE_OPTIONS
                .iter()
                .find(|(value, _)| *value == index)
                .map(|(_, name)| (*name).to_owned())
                .unwrap_or_else(|| index.to_string());
            edits.push(PendingFieldEdit {
                path: paths.parameter_type.clone(),
                input,
            });
        }
    }
    if view.input_name != prev.input_name && !paths.input_name.is_empty() {
        edits.push(PendingFieldEdit {
            path: paths.input_name.clone(),
            input: if view.input_name.is_empty() {
                "none".to_owned()
            } else {
                view.input_name.clone()
            },
        });
    }
    if view.range_name != prev.range_name && !paths.range_name.is_empty() {
        edits.push(PendingFieldEdit {
            path: paths.range_name.clone(),
            input: if view.range_name.is_empty() {
                "none".to_owned()
            } else {
                view.range_name.clone()
            },
        });
    }
    if view.time_period_in_seconds != prev.time_period && !paths.time_period.is_empty() {
        edits.push(PendingFieldEdit {
            path: paths.time_period.clone(),
            input: view.time_period_in_seconds.to_string(),
        });
    }
    FunctionEditBatch {
        tag_key: String::new(),
        edits,
        data_ops,
    }
}

pub(super) fn draw_function_popup(
    ctx: &egui::Context,
    function_popup: &mut Option<FunctionPopup>,
) -> Option<FunctionEditBatch> {
    let popup = function_popup.as_mut()?;
    let mut open = true;
    let mut close = false;
    let mut commit = false;
    let editable = popup.editable;
    egui::Window::new(popup.title.clone())
        .collapsible(false)
        .resizable(false)
        .default_size(Vec2::new(700.0, 440.0))
        .open(&mut open)
        .show(ctx, |ui| {
            if !editable {
                ui.label(
                    RichText::new("read-only (function has no writable path on this tag)")
                        .color(subtle_dark())
                        .small(),
                );
            }
            draw_function_editor_contents(ui, &mut popup.view, editable, &mut popup.selected_point);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("OK").clicked() {
                        commit = true;
                        close = true;
                    }
                });
            });
        });

    // Commit edits only when OK is pressed. Live-writing while a modal is
    // open can invalidate classic H2 wrapper fields underneath combo boxes.
    let mut batch = None;
    if editable && commit {
        if let Some(paths) = popup.view.edit.clone() {
            let mut edits = push_function_edit(&paths, &popup.last_applied, &popup.view);
            if !edits.edits.is_empty() || !edits.data_ops.is_empty() {
                popup.last_applied = FunctionSnapshot::from_view(&popup.view);
                edits.tag_key = popup.tag_key.clone();
                batch = Some(FunctionEditBatch {
                    tag_key: edits.tag_key,
                    edits: edits.edits,
                    data_ops: edits.data_ops,
                });
            }
        }
    }

    if close || !open {
        *function_popup = None;
    }
    batch
}
