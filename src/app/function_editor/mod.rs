//! Function popup coordination and shared function-editor entry points.
//! It owns function decoding, visualization, and edit construction; applying edits to documents and unrelated shader layout belong elsewhere.

use super::*;

mod graph;
mod h2;
mod h3;

pub(super) use graph::*;
pub(super) use h2::*;
pub(super) use h3::*;

/// The interactive function editor body. When `editable` is false every
/// control is shown read-only. Returns whether `view` changed this frame.
pub(in crate::app) fn draw_function_editor_contents(
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

    let show_color_controls = !(view.hide_scalar_color_controls
        && view.function.color_graph_type() == ColorGraphType::Scalar);
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
        if show_color_controls {
            ui.label(RichText::new("Color:").color(text_dark()).small());
            changed |= color_graph_combo(ui, &mut view.function, type_editable);
        }
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
            // Color graph: show the evaluated endpoint colors as swatches on the
            // output axis (top = input 1.0, bottom = input 0.0), matching the
            // scalar high/low layout.
            ui.vertical(|ui| {
                let endpoint_swatch = |ui: &mut Ui, x: f32| {
                    let c = view.function.evaluate_color(x, x);
                    let (r, g, b) = (
                        float_channel_to_u8(c.red),
                        float_channel_to_u8(c.green),
                        float_channel_to_u8(c.blue),
                    );
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::Vec2::new(22.0, 18.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
                    ui.painter()
                        .rect_stroke(rect, 2.0, egui::Stroke::new(1.0, grid_line()));
                    resp.on_hover_text(format!("input {x:.0}: R{r} G{g} B{b}"));
                };
                endpoint_swatch(ui, 1.0);
                ui.add_space(118.0);
                endpoint_swatch(ui, 0.0);
            });
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

/// Diff a view's current values against the last-applied snapshot and
/// build `PendingFieldEdit`s for the fields that changed. The blob is
/// hex-encoded into the string edit channel; wrapper fields use their
/// normal text representations.
pub(in crate::app) fn push_function_edit(
    paths: &FunctionEditPaths,
    prev: &FunctionSnapshot,
    view: &FunctionView,
) -> FunctionEditBatch {
    let mut edits = Vec::new();
    let mut data_ops = Vec::new();
    let data = view.data_bytes();
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

pub(in crate::app) fn draw_function_popup(
    ctx: &egui::Context,
    function_popup: &mut Option<FunctionPopup>,
    use_new_h3_function_editor: bool,
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
            if popup.view.h2_legacy.is_some() {
                draw_h2_legacy_function_editor_contents(ui, &mut popup.view, editable);
            } else {
                let presentation =
                    *popup
                        .h3_presentation
                        .get_or_insert(if use_new_h3_function_editor {
                            H3FunctionEditorPresentation::Foundation
                        } else {
                            H3FunctionEditorPresentation::Legacy
                        });
                match presentation {
                    H3FunctionEditorPresentation::Foundation => {
                        draw_foundation_h3_function_editor_contents(
                            ui,
                            &mut popup.view,
                            editable,
                            &mut popup.selected_point,
                        );
                    }
                    H3FunctionEditorPresentation::Legacy => {
                        draw_function_editor_contents(
                            ui,
                            &mut popup.view,
                            editable,
                            &mut popup.selected_point,
                        );
                    }
                }
            }
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
