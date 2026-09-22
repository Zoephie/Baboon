//! Inline function rows and edit-path construction.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

pub(in crate::app) fn draw_foundation_function_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    function: &TagFunction,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    ui.horizontal_top(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        Frame::none()
            .fill(foundation_group_bg())
            .stroke(Stroke::new(1.0, foundation_group_edge()))
            .inner_margin(egui::Margin::same(6.0))
            .show(ui, |ui| {
                // `Frame::show` inherits the parent layout, and this row is
                // built inside a `horizontal_top`. Force a vertical layout so
                // the function editor stacks its controls / graph / time-period
                // top-to-bottom (Guerilla-style) instead of sprawling to the
                // right.
                ui.vertical(|ui| {
                    ui.set_min_width(640.0);
                    ui.horizontal(|ui| {
                        foundation_input_cell(ui, &shader_function_grid_text(function), 520.0);
                        let can_edit = edit.editable && !meta.read_only;
                        let function_button = foundation_header_button_clicked_hint(
                            ui,
                            "f()",
                            can_edit,
                            Some("Function is read-only"),
                        );
                        if function_button {
                            *edit.function_request = Some(FunctionPopup::new(
                                edit.tag_key.to_owned(),
                                canonical_field_path(path),
                                FunctionView::from_function(function.clone())
                                    .with_edit(foundation_function_edit_paths(path, function.encoding())),
                                true,
                            ));
                        }
                    });
                    ui.add_space(4.0);
                    ui.push_id(("function", path), |ui| {
                        // Inline preview is always read-only; the editable
                        // editor lives in the f() popup.
                        let mut view = FunctionView::from_function(function.clone());
                        let (mut graph, mut point, mut no_popup) = (0usize, 0usize, None);
                        draw_function_editor(ui, &mut view, false, &mut graph, &mut point, &mut no_popup);
                    });
                });
            });
        draw_field_help(ui, meta);
    });
}

pub(in crate::app) fn draw_foundation_inline_function_row(
    ui: &mut Ui,
    label: String,
    mut view: FunctionView,
    depth: usize,
    data_path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let encoding = view.function.encoding();
    view = view.with_edit(foundation_function_edit_paths(data_path, encoding));

    // Every function, whatever its game, shows the same row: its summary, the
    // f() button that opens the editor, and the editor itself as a read-only
    // preview.
    draw_foundation_wrapped_function_row(ui, label, view, depth, edit);
}

fn draw_foundation_wrapped_function_row(
    ui: &mut Ui,
    label: String,
    view: FunctionView,
    depth: usize,
    edit: &mut FieldEditContext<'_>,
) {
    ui.horizontal_top(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &label, None);
        Frame::none()
            .fill(foundation_group_bg())
            .stroke(Stroke::new(1.0, foundation_group_edge()))
            .inner_margin(egui::Margin::same(6.0))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(640.0);
                    ui.horizontal(|ui| {
                        foundation_input_cell(
                            ui,
                            &shader_function_grid_text(&view.function),
                            520.0,
                        );
                        let function_button = foundation_header_button_clicked_hint(
                            ui,
                            "f()",
                            edit.editable,
                            Some("Function is read-only"),
                        );
                        if function_button {
                            *edit.function_request = Some(FunctionPopup::new(
                                edit.tag_key.to_owned(),
                                label.clone(),
                                view.clone(),
                                true,
                            ));
                        }
                    });
                    ui.add_space(4.0);
                    ui.push_id(("wrapped_function", data_path_id(&view)), |ui| {
                        let mut preview = view.clone();
                        let (mut graph, mut point, mut no_popup) = (0usize, 0usize, None);
                        draw_function_editor(ui, &mut preview, false, &mut graph, &mut point, &mut no_popup);
                    });
                });
            });
    });
}

fn data_path_id(view: &FunctionView) -> &str {
    view.edit
        .as_ref()
        .and_then(|paths| paths.data.data_field_path())
        .unwrap_or("function")
}

/// Write targets for a function at `data_path`. Where its bytes go follows
/// from the encoding: a Halo 2 function lives in a byte-block, an H3+ blob in a
/// data field.
pub(in crate::app) fn foundation_function_edit_paths(
    data_path: &str,
    encoding: FunctionEncoding,
) -> FunctionEditPaths {
    FunctionEditPaths {
        data: match encoding {
            FunctionEncoding::H2 => FunctionDataStorage::Halo2ByteBlock(data_path.to_owned()),
            FunctionEncoding::Blob => FunctionDataStorage::DataField(data_path.to_owned()),
        },
        parameter_type: String::new(),
        input_name: String::new(),
        range_name: String::new(),
        time_period: String::new(),
        block_path: String::new(),
        block_index: 0,
    }
}

#[cfg(test)]
#[path = "../tests/function_editor_routing.rs"]
mod function_editor_routing_tests;

/// First-pass editable function types — others stay read-only (graph +
/// controls disabled) but still round-trip on save.
pub(in crate::app) fn draw_foundation_enum_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    options: &[&str],
    current: Option<i64>,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let mut selected = current.unwrap_or(-1);
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        ui.add_enabled_ui(edit.editable && !meta.read_only, |ui| {
            let selected_label = enum_option_label(options, selected);
            let selected_text = highlighted_widget_text(
                ui,
                &selected_label,
                TextStyle::Button,
                text_dark(),
                FindTargetKind::Value,
            )
            .unwrap_or_else(|| selected_label.clone().into());
            let (_, wheel_delta) = combo_box_with_scroll(
                ui,
                egui::ComboBox::from_id_salt((edit.view_scope, edit.tag_key, path, "enum"))
                    .width(240.0)
                    .selected_text(selected_text),
                |ui| {
                    for (index, option) in options.iter().enumerate() {
                        ui.selectable_value(&mut selected, index as i64, *option);
                    }
                },
            );
            if let Some(delta) = wheel_delta
                && let Some(next) =
                    combo_scroll_next_i64(selected, 0, options.len() as i64 - 1, delta)
            {
                selected = next;
            }
        });
        if Some(selected) != current && selected >= 0 {
            edit.pending.push(PendingFieldEdit {
                path: path.to_owned(),
                input: selected.to_string(),
            });
        }
        draw_field_help(ui, meta);
    });
}
