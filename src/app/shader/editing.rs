//! Shader override, reset, range, and value-edit application helpers.
//! It owns shader-specific models, edits, and presentation helpers; generic field editing and controller orchestration belong elsewhere.

use super::*;

const SHADER_MODIFIED_ACCENT: Color32 = Color32::from_rgb(224, 158, 62);

/// Whether an explicitly overridden row's value differs from its default.
/// Colors render identical text (`"color: RGB"`) so are compared by hex;
/// inherited rows never count as modified.
pub(in crate::app) fn row_differs_from_default(row: &ShaderGridRow) -> bool {
    let Some(default) = row.default_cell.as_ref() else {
        return false;
    };
    if !row.is_overridden {
        return false;
    }
    match (row.value_cell.color.as_ref(), default.color.as_ref()) {
        (Some(value), Some(default)) => value.sc_hex != default.sc_hex,
        _ => !shader_value_text_eq(&row.value_cell.text, &default.text),
    }
}

/// A `BlockOp` that clears a shader override by deleting the owning
/// `parameters[n]` element. This is Foundation's ClearValue semantics: an
/// explicit value equal to the default is still an override, so reset must
/// remove the sparse parameter entry instead of writing the default value.
pub(super) fn reset_op_for_row(row: &ShaderGridRow) -> Option<BlockOp> {
    if !row.is_overridden {
        return None;
    }
    let row_edit = row.edit.as_ref()?;
    if matches!(
        row_edit.kind,
        ShaderRowEditKind::CreateScalarParam { .. }
            | ShaderRowEditKind::CreateFunctionColor { .. }
            | ShaderRowEditKind::CreateFunctionScalar { .. }
            | ShaderRowEditKind::H2CreateFunctionScalar { .. }
            | ShaderRowEditKind::H2CreateFunctionColor { .. }
            | ShaderRowEditKind::H2CreateTemplateValue { .. }
            | ShaderRowEditKind::H2CreateTemplateColor { .. }
    ) {
        return None;
    }
    shader_parameter_delete_op_from_field_path(&row_edit.path)
}

fn shader_parameter_delete_op_from_field_path(path: &str) -> Option<BlockOp> {
    let slash = path.rfind('/')?;
    let parent = &path[..slash];
    let open = parent.rfind('[')?;
    let close = parent[open + 1..].find(']')? + open + 1;
    if close + 1 != parent.len() {
        return None;
    }
    let index = parent[open + 1..close].parse::<usize>().ok()?;
    Some(BlockOp {
        path: parent[..open].to_owned(),
        kind: BlockOpKind::Delete(index),
    })
}

fn push_shader_override_create(edit: &mut FieldEditContext<'_>, row_edit: &ShaderRowEdit) -> bool {
    match &row_edit.kind {
        ShaderRowEditKind::BitmapRef { create, .. } => {
            push_shader_value_edit(edit, row_edit, create.as_ref(), row_edit.current.clone());
            create.is_some()
        }
        ShaderRowEditKind::Bool { create } => {
            push_shader_value_edit(edit, row_edit, create.as_ref(), row_edit.current.clone());
            create.is_some()
        }
        ShaderRowEditKind::CreateScalarParam {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
        } => {
            edit.shader_param_ops.push(ShaderParamOp {
                parameters_block_path: parameters_block_path.clone(),
                parameter_name: parameter_name.clone(),
                initial_fields: vec![
                    shader_parameter_type_initial_field(*parameter_type_index),
                    ShaderParamInitialField {
                        field: "real".to_owned(),
                        input: row_edit.current.clone(),
                    },
                ],
                animated_parameters: Vec::new(),
            });
            true
        }
        ShaderRowEditKind::CreateFunctionColor { target } => {
            let rgba = parse_shader_rgba(&row_edit.current).unwrap_or([1.0, 1.0, 1.0, 1.0]);
            push_shader_context_action(
                edit,
                &shader_function_action(
                    target,
                    constant_color_function_hex(rgba[0], rgba[1], rgba[2], rgba[3]),
                ),
            );
            true
        }
        ShaderRowEditKind::CreateFunctionScalar { target } => {
            let value = row_edit.current.trim().parse::<f32>().unwrap_or_default();
            push_shader_context_action(
                edit,
                &shader_function_action(target, constant_function_hex(value)),
            );
            true
        }
        ShaderRowEditKind::H2CreateFunctionColor { create_op } => {
            edit.h2_shader_param_ops.push(create_op.clone());
            true
        }
        ShaderRowEditKind::H2CreateFunctionScalar { create_op } => {
            let value = row_edit.current.trim().parse::<f32>().unwrap_or_default();
            let mut op = create_op.clone();
            if let H2ShaderParamOp::EnsureAnimationProperty {
                initial_function_data,
                ..
            } = &mut op
            {
                *initial_function_data =
                    decode_hex(&constant_function_hex(value)).unwrap_or_default();
            }
            edit.h2_shader_param_ops.push(op);
            true
        }
        ShaderRowEditKind::H2CreateTemplateValue {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
            field,
        } => {
            edit.h2_shader_param_ops
                .push(H2ShaderParamOp::EditTemplateBackedValue {
                    parameters_block_path: parameters_block_path.clone(),
                    parameter_name: parameter_name.clone(),
                    parameter_type_index: *parameter_type_index,
                    field: field.clone(),
                    input: h2_template_value_input(field, &row_edit.current),
                });
            true
        }
        ShaderRowEditKind::H2CreateTemplateColor {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
            field,
        } => {
            let rgba = parse_shader_rgba(&row_edit.current).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            edit.h2_shader_param_ops
                .push(H2ShaderParamOp::EditTemplateBackedValue {
                    parameters_block_path: parameters_block_path.clone(),
                    parameter_name: parameter_name.clone(),
                    parameter_type_index: *parameter_type_index,
                    field: field.clone(),
                    input: format!("{}, {}, {}", rgba[0], rgba[1], rgba[2]),
                });
            true
        }
        _ => false,
    }
}

fn parse_shader_rgba(input: &str) -> Option<[f32; 4]> {
    let values = input
        .split(',')
        .map(str::trim)
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    match values.as_slice() {
        [r, g, b] => Some([*r, *g, *b, 1.0]),
        [r, g, b, a] => Some([*r, *g, *b, *a]),
        _ => None,
    }
}

/// Compare two grid-cell value texts, tolerating numeric formatting differences
/// (e.g. `value: 1` vs `value: 1.0`) that arise on the classic (H2) path.
fn shader_value_text_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    if a == b {
        return true;
    }
    let na = a.rsplit(": ").next().unwrap_or(a);
    let nb = b.rsplit(": ").next().unwrap_or(b);
    match (na.parse::<f64>(), nb.parse::<f64>()) {
        (Ok(x), Ok(y)) => (x - y).abs() < 1e-5,
        _ => false,
    }
}

fn shader_label_width_id() -> egui::Id {
    egui::Id::new("shader_grid_label_width")
}

/// Session-persisted width of the shader grid's label column (Phase 4.3 resizable
/// columns); dragged via the per-row splitter, read by every row.
pub(super) fn shader_label_width(ui: &Ui) -> f32 {
    ui.data(|d| d.get_temp::<f32>(shader_label_width_id()))
        .unwrap_or(230.0)
        .clamp(120.0, 460.0)
}

pub(in crate::app) fn draw_shader_grid_row(
    ui: &mut Ui,
    row: &ShaderGridRow,
    depth: usize,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    edit: &mut FieldEditContext<'_>,
) {
    let tag_key = edit.tag_key;
    let editable = edit.editable;
    let available = ui.available_width().max(780.0);
    let indent = depth as f32 * 10.0;
    let base_label_width = shader_label_width(ui);
    let label_width = (base_label_width - indent).max(110.0);
    let default_width = 110.0;
    let has_h2_range = h2_range_control_for_row(row).is_some();
    let right_controls_width = shader_right_controls_width(row, has_h2_range);
    let height = shader_grid_row_height(row);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(available, height), Sense::click());
    let row_text = material_text_for_bg(row.fill);
    ui.painter().rect_filled(rect, 0.0, row.fill);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, material_grid_light()),
    );
    let modified = row_differs_from_default(row);
    if modified {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), Vec2::new(3.0, height)),
            0.0,
            SHADER_MODIFIED_ACCENT,
        );
    }

    let label_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(4.0 + indent, 0.0),
        Vec2::new(label_width, height),
    );
    ui.painter().text(
        label_rect.right_center() - Vec2::new(6.0, 0.0),
        Align2::RIGHT_CENTER,
        truncate_for_cell(&row.label, label_width - 12.0),
        FontId::proportional(12.5),
        row_text,
    );
    // Per-parameter "help": hovering the label shows the full (untruncated) name
    // plus its parameter type — rmop parameters carry no description text.
    if !row.label.is_empty() {
        let hover = match row.parameter_type.as_deref() {
            Some(parameter_type) => format!("{}\n{}", row.label, parameter_type),
            None => row.label.clone(),
        };
        ui.interact(
            label_rect,
            ui.make_persistent_id(("shader_label_hover", &row.label)),
            Sense::hover(),
        )
        .on_hover_text(hover);
    }
    // Resizable label column (Phase 4.3): a drag handle at the label/value
    // boundary updates the shared session width that every row reads.
    let split_x = label_rect.right() + 1.0;
    let split_resp = ui.interact(
        egui::Rect::from_center_size(egui::pos2(split_x, rect.center().y), Vec2::new(6.0, height)),
        ui.make_persistent_id(("shader_col_split", &row.label)),
        Sense::drag(),
    );
    if split_resp.hovered() || split_resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        ui.painter().line_segment(
            [
                egui::pos2(split_x, rect.top()),
                egui::pos2(split_x, rect.bottom()),
            ],
            Stroke::new(1.0, row_text),
        );
    }
    if split_resp.dragged() {
        let new_width = (base_label_width + split_resp.drag_delta().x).clamp(120.0, 460.0);
        ui.data_mut(|d| d.insert_temp(shader_label_width_id(), new_width));
    }

    let default_rect = egui::Rect::from_min_size(
        label_rect.right_top() + Vec2::new(2.0, 2.0),
        Vec2::new(default_width, height - 4.0),
    );
    draw_shader_grid_cell(
        ui,
        default_rect,
        row.default_cell.as_ref(),
        &format!("default:{}", row.label),
        color_popup,
    );

    let value_left = default_rect.right() + 6.0;
    let controls_left = rect.right() - right_controls_width;
    let value_right = (controls_left - 4.0).max(value_left + 40.0);
    let mut value_rect = egui::Rect::from_min_max(
        egui::pos2(value_left, default_rect.top()),
        egui::pos2(value_right, default_rect.bottom()),
    );
    let reset = (editable && row.is_overridden)
        .then(|| reset_op_for_row(row))
        .flatten();
    let reset_rect = reset.as_ref().map(|_| {
        let rect = egui::Rect::from_min_size(
            value_rect.right_top() - Vec2::new(20.0, 0.0),
            Vec2::new(18.0, value_rect.height()),
        );
        value_rect.max.x = (rect.left() - 3.0).max(value_rect.left() + 40.0);
        rect
    });

    // Editable value cell when the row carries an edit path and the tag is
    // writable; otherwise the read-only painted cell.
    if editable
        && !row.is_overridden
        && let Some(row_edit) = row.edit.as_ref()
        && matches!(
            row_edit.kind,
            ShaderRowEditKind::BitmapRef {
                create: Some(_),
                ..
            } | ShaderRowEditKind::Bool { create: Some(_) }
                | ShaderRowEditKind::CreateScalarParam { .. }
                | ShaderRowEditKind::CreateFunctionColor { .. }
                | ShaderRowEditKind::CreateFunctionScalar { .. }
                | ShaderRowEditKind::H2CreateFunctionScalar { .. }
                | ShaderRowEditKind::H2CreateFunctionColor { .. }
                | ShaderRowEditKind::H2CreateTemplateValue { .. }
                | ShaderRowEditKind::H2CreateTemplateColor { .. }
        )
    {
        ui.scope_builder(egui::UiBuilder::new().max_rect(value_rect), |ui| {
            let response = ui.add_sized(
                value_rect.size(),
                egui::Button::new(RichText::new("Override Default").color(material_text()))
                    .fill(material_pending_input()),
            );
            if response
                .on_hover_text("Create an explicit override initialized from the default")
                .clicked()
            {
                push_shader_override_create(edit, row_edit);
            }
        });
    } else if let (true, Some(row_edit)) = (editable, row.edit.as_ref()) {
        draw_shader_editable_value(ui, value_rect, &row.label, row_edit, edit, color_popup);
    } else {
        draw_shader_grid_cell(
            ui,
            value_rect,
            Some(&row.value_cell),
            &format!("value:{}", row.label),
            color_popup,
        );
    }
    if let (Some(reset), Some(reset_rect)) = (reset, reset_rect) {
        ui.painter().rect_filled(reset_rect, 0.0, material_input());
        ui.painter()
            .rect_stroke(reset_rect, 0.0, Stroke::new(1.0, material_input_edge()));
        ui.painter().text(
            reset_rect.center(),
            Align2::CENTER_CENTER,
            "×",
            FontId::proportional(13.0),
            material_delete_text(),
        );
        if ui
            .interact(
                reset_rect,
                ui.make_persistent_id(format!("shader_override_clear:{}", row.label)),
                Sense::click(),
            )
            .on_hover_text("Clear override and inherit the default")
            .clicked()
        {
            edit.block_ops.push(reset);
        }
    }

    let mut next_function_x = controls_left.max(value_rect.right() + 4.0);
    if let Some(control) = h2_range_control_for_row(row) {
        let range_rect = egui::Rect::from_min_size(
            egui::pos2(next_function_x, value_rect.top() + 2.0),
            Vec2::new(H2_RANGE_CONTROL_WIDTH, height - 4.0),
        );
        draw_h2_function_range_control(ui, range_rect, row, &control, edit);
        next_function_x = range_rect.right() + 4.0;
    }

    if let Some(function) = row.function.as_ref() {
        // Orange function row: range: checkbox + f() button + × delete button.
        let button_rect = egui::Rect::from_min_size(
            egui::pos2(next_function_x, value_rect.top() + 1.0),
            Vec2::new(28.0, height - 4.0),
        );
        ui.painter().rect_filled(button_rect, 0.0, material_input());
        ui.painter()
            .rect_stroke(button_rect, 0.0, Stroke::new(1.0, material_input_edge()));
        let icon_rect = egui::Rect::from_center_size(button_rect.center(), Vec2::splat(16.0));
        paint_button_icon_at(ui, ButtonIcon::Function, icon_rect, material_text());

        let click_response = ui
            .interact(
                rect,
                ui.make_persistent_id(format!("shader_function:{}", row.label)),
                Sense::click(),
            )
            .on_hover_text("Click to open function viewer");
        if response.clicked() || click_response.clicked() {
            *function_popup = Some(FunctionPopup::new(
                tag_key.to_owned(),
                row.label.clone(),
                function.clone(),
                editable && function.edit.is_some(),
            ));
        }

        // × delete button: removes the animated parameter from the block.
        if editable && !is_h2_function_view(function) {
            if let Some(edit_paths) = function.edit.as_ref() {
                let del_rect = egui::Rect::from_min_size(
                    button_rect.right_top() + Vec2::new(4.0, 0.0),
                    Vec2::new(18.0, height - 4.0),
                );
                ui.painter().rect_filled(del_rect, 0.0, material_input());
                ui.painter()
                    .rect_stroke(del_rect, 0.0, Stroke::new(1.0, material_input_edge()));
                ui.painter().text(
                    del_rect.center(),
                    Align2::CENTER_CENTER,
                    "×",
                    FontId::proportional(13.0),
                    material_delete_text(),
                );
                if ui
                    .interact(
                        del_rect,
                        ui.make_persistent_id(format!("shader_fn_del:{}", row.label)),
                        Sense::click(),
                    )
                    .on_hover_text("Remove animated parameter")
                    .clicked()
                {
                    edit.block_ops.push(BlockOp {
                        path: edit_paths.block_path.clone(),
                        kind: BlockOpKind::Delete(edit_paths.block_index),
                    });
                }
            }
        }
    } else if let Some(func_view) = row.constant_function_view.as_ref() {
        // Constant-function scalar row: small "f()" to open graph + "×" delete.
        let f_rect = egui::Rect::from_min_size(
            egui::pos2(next_function_x, value_rect.top() + 2.0),
            Vec2::new(26.0, height - 4.0),
        );
        ui.painter().rect_filled(f_rect, 0.0, material_input());
        ui.painter()
            .rect_stroke(f_rect, 0.0, Stroke::new(1.0, material_input_edge()));
        let icon_rect = egui::Rect::from_center_size(f_rect.center(), Vec2::splat(16.0));
        paint_button_icon_at(ui, ButtonIcon::Function, icon_rect, material_text());
        if ui
            .interact(
                f_rect,
                ui.make_persistent_id(format!("shader_cfn_open:{}", row.label)),
                Sense::click(),
            )
            .on_hover_text("Open function graph editor")
            .clicked()
            || ui
                .interact(
                    value_rect,
                    ui.make_persistent_id(format!("shader_cfn_value_open:{}", row.label)),
                    Sense::click(),
                )
                .on_hover_text("Double-click to open function graph editor")
                .double_clicked()
        {
            *function_popup = Some(FunctionPopup::new(
                tag_key.to_owned(),
                row.label.clone(),
                func_view.clone(),
                editable && func_view.edit.is_some(),
            ));
        }

        if editable && !is_h2_function_view(func_view) {
            if let Some(edit_paths) = func_view.edit.as_ref() {
                let del_rect = egui::Rect::from_min_size(
                    f_rect.right_top() + Vec2::new(2.0, 0.0),
                    Vec2::new(18.0, height - 4.0),
                );
                ui.painter().rect_filled(del_rect, 0.0, material_input());
                ui.painter()
                    .rect_stroke(del_rect, 0.0, Stroke::new(1.0, material_input_edge()));
                ui.painter().text(
                    del_rect.center(),
                    Align2::CENTER_CENTER,
                    "×",
                    FontId::proportional(13.0),
                    material_delete_text(),
                );
                if ui
                    .interact(
                        del_rect,
                        ui.make_persistent_id(format!("shader_cfn_del:{}", row.label)),
                        Sense::click(),
                    )
                    .on_hover_text("Remove animated parameter")
                    .clicked()
                {
                    edit.block_ops.push(BlockOp {
                        path: edit_paths.block_path.clone(),
                        kind: BlockOpKind::Delete(edit_paths.block_index),
                    });
                }
            }
        }
    } else if let (true, Some(action)) = (editable, row.create_anim_op.as_ref()) {
        // No animated parameter yet — show an "f()+" button to create one.
        let button_rect = egui::Rect::from_min_size(
            egui::pos2(next_function_x, value_rect.top() + 2.0),
            Vec2::new(34.0, height - 4.0),
        );
        ui.painter().rect_filled(button_rect, 0.0, material_input());
        ui.painter()
            .rect_stroke(button_rect, 0.0, Stroke::new(1.0, material_input_edge()));
        ui.painter().text(
            button_rect.center(),
            Align2::CENTER_CENTER,
            if matches!(action, ShaderContextAction::H2ParameterOp(_)) {
                "f0"
            } else {
                "f()+"
            },
            FontId::proportional(11.0),
            material_text(),
        );
        let add_response = ui
            .interact(
                button_rect,
                ui.make_persistent_id(format!("shader_create_anim:{}", row.label)),
                Sense::click(),
            )
            .on_hover_text("Create animated parameter");
        if add_response.clicked() {
            push_shader_context_action(edit, action);
        }
    } else {
        // context_menu takes &self so call it first; on_hover_text takes self.
        let reset = (editable && row.is_overridden)
            .then(|| reset_op_for_row(row))
            .flatten();
        let menu_items = row
            .context_menu
            .as_ref()
            .filter(|_| editable)
            .map(|menu| menu.items.as_slice())
            .filter(|items| !items.is_empty());
        if reset.is_some() || menu_items.is_some() {
            response.context_menu(|ui| {
                if let Some(reset) = reset.clone() {
                    if ui.button("Reset to default").clicked() {
                        edit.block_ops.push(reset);
                        ui.close_menu();
                    }
                }
                if let Some(items) = menu_items {
                    if reset.is_some() {
                        ui.separator();
                    }
                    ui.label("Add optional argument:");
                    ui.separator();
                    for item in items {
                        if ui.button(&item.label).clicked() {
                            push_shader_context_action(edit, &item.action);
                            ui.close_menu();
                        }
                    }
                }
            });
        }
        if let Some(parameter_type) = row.parameter_type.as_deref() {
            response.on_hover_text(parameter_type);
        }
    }
}

fn is_h2_function_view(function: &FunctionView) -> bool {
    function
        .edit
        .as_ref()
        .is_some_and(|edit| matches!(edit.data, FunctionDataStorage::Halo2ByteBlock(_)))
}

fn shader_grid_row_height(row: &ShaderGridRow) -> f32 {
    if row
        .edit
        .as_ref()
        .is_some_and(|edit| matches!(edit.kind, ShaderRowEditKind::Flags(_)))
    {
        58.0
    } else {
        25.0
    }
}

const H2_RANGE_CONTROL_WIDTH: f32 = 136.0;

fn shader_right_controls_width(row: &ShaderGridRow, has_h2_range: bool) -> f32 {
    let mut width = 8.0;
    if has_h2_range {
        width += H2_RANGE_CONTROL_WIDTH + 4.0;
    }
    if let Some(function) = row.function.as_ref() {
        width += 28.0;
        if !is_h2_function_view(function) {
            width += 22.0;
        }
    } else if let Some(function) = row.constant_function_view.as_ref() {
        width += 26.0;
        if !is_h2_function_view(function) {
            width += 20.0;
        }
    } else if row.create_anim_op.is_some() {
        width += 34.0;
    }
    width
}

#[derive(Clone)]
enum H2RangeControl {
    Existing { block_path: String, data: Vec<u8> },
    Create { op: H2ShaderParamOp, data: Vec<u8> },
}

fn h2_range_control_for_row(row: &ShaderGridRow) -> Option<H2RangeControl> {
    if let Some(function) = row
        .function
        .as_ref()
        .filter(|function| is_h2_function_view(function))
    {
        return h2_range_control_from_function(function);
    }
    if let Some(function) = row
        .constant_function_view
        .as_ref()
        .filter(|function| is_h2_function_view(function))
    {
        return h2_range_control_from_function(function);
    }
    if let Some(edit) = row.edit.as_ref() {
        match &edit.kind {
            ShaderRowEditKind::H2FunctionScalar {
                block_path,
                legacy_data,
            }
            | ShaderRowEditKind::H2FunctionColor {
                block_path,
                legacy_data,
            } => {
                let data = legacy_data
                    .clone()
                    .or_else(|| {
                        row.constant_function_view
                            .as_ref()
                            .map(FunctionView::data_bytes)
                    })
                    .unwrap_or_default();
                if !data.is_empty() {
                    return Some(H2RangeControl::Existing {
                        block_path: block_path.clone(),
                        data,
                    });
                }
            }
            ShaderRowEditKind::H2CreateFunctionScalar { create_op }
            | ShaderRowEditKind::H2CreateFunctionColor { create_op } => {
                if let Some(data) = h2_initial_function_data_from_op(create_op) {
                    return Some(H2RangeControl::Create {
                        op: create_op.clone(),
                        data,
                    });
                }
            }
            _ => {}
        }
    }
    if let Some(ShaderContextAction::H2ParameterOp(op)) = row.create_anim_op.as_ref() {
        if let Some(data) = h2_initial_function_data_from_op(op) {
            return Some(H2RangeControl::Create {
                op: op.clone(),
                data,
            });
        }
    }
    None
}

fn h2_range_control_from_function(function: &FunctionView) -> Option<H2RangeControl> {
    let edit = function.edit.as_ref()?;
    let FunctionDataStorage::Halo2ByteBlock(block_path) = &edit.data else {
        return None;
    };
    Some(H2RangeControl::Existing {
        block_path: block_path.clone(),
        data: function.data_bytes(),
    })
}

fn h2_initial_function_data_from_op(op: &H2ShaderParamOp) -> Option<Vec<u8>> {
    match op {
        H2ShaderParamOp::EnsureAnimationProperty {
            initial_function_data,
            ..
        } => Some(initial_function_data.clone()),
        _ => None,
    }
}

pub(super) fn h2_function_range_enabled(data: &[u8]) -> bool {
    data.get(1)
        .copied()
        .is_some_and(|flags| flags & FunctionFlags::RANGE != 0)
}

pub(super) fn h2_function_range_value(data: &[u8]) -> Option<f32> {
    Some(f32::from_le_bytes(data.get(8..12)?.try_into().ok()?))
}

pub(super) fn h2_function_data_with_range(
    data: &[u8],
    enabled: bool,
    value: Option<f32>,
) -> Vec<u8> {
    let mut next = data.to_vec();
    if next.len() < 12 {
        next.resize(12, 0);
    }
    if enabled {
        next[1] |= FunctionFlags::RANGE;
    } else {
        next[1] &= !FunctionFlags::RANGE;
    }
    if let Some(value) = value {
        next[8..12].copy_from_slice(&value.to_le_bytes());
    }
    next
}

fn h2_push_range_data_edit(
    edit: &mut FieldEditContext<'_>,
    control: &H2RangeControl,
    data: Vec<u8>,
) {
    match control {
        H2RangeControl::Existing { block_path, .. } => {
            edit.h2_shader_param_ops
                .push(H2ShaderParamOp::EditFunctionData {
                    block_path: block_path.clone(),
                    data,
                });
        }
        H2RangeControl::Create { op, .. } => {
            let mut op = op.clone();
            if let H2ShaderParamOp::EnsureAnimationProperty {
                initial_function_data,
                ..
            } = &mut op
            {
                *initial_function_data = data;
                edit.h2_shader_param_ops.push(op);
            }
        }
    }
}

fn draw_h2_function_range_control(
    ui: &mut Ui,
    rect: egui::Rect,
    row: &ShaderGridRow,
    control: &H2RangeControl,
    edit: &mut FieldEditContext<'_>,
) {
    let data = match control {
        H2RangeControl::Existing { data, .. } | H2RangeControl::Create { data, .. } => data,
    };
    if data.len() < 12 {
        return;
    }
    let enabled = h2_function_range_enabled(data);
    let mut checked = enabled;
    let check_rect =
        egui::Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 2.0), Vec2::splat(14.0));
    let response = ui
        .scope_builder(egui::UiBuilder::new().max_rect(check_rect), |ui| {
            ui.add_enabled(edit.editable, egui::Checkbox::new(&mut checked, ""))
        })
        .inner;
    ui.painter().text(
        check_rect.right_center() + Vec2::new(4.0, 0.0),
        Align2::LEFT_CENTER,
        "range:",
        FontId::proportional(12.0),
        material_text_for_bg(row.fill),
    );
    if response.changed() {
        h2_push_range_data_edit(
            edit,
            control,
            h2_function_data_with_range(data, checked, h2_function_range_value(data)),
        );
    }

    let value_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(66.0, 0.0),
        Vec2::new((rect.width() - 66.0).max(42.0), rect.height()),
    );
    let current = if enabled {
        h2_function_range_value(data)
            .map(format_shader_float)
            .unwrap_or_default()
    } else {
        String::new()
    };
    let id = edit.widget_id(("h2_range", row.label.as_str()));
    let buffer_key = format!("{}|h2_range:{}", edit.tag_key, row.label);
    let buffer = edit
        .buffers
        .entry(buffer_key)
        .or_insert_with(|| current.clone());
    if !ui.memory(|m| m.has_focus(id)) && *buffer != current {
        *buffer = current.clone();
    }
    let mut commit_value = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(value_rect), |ui| {
        ui.visuals_mut().extreme_bg_color = material_input();
        let resp = ui.add_enabled(
            edit.editable && enabled,
            egui::TextEdit::singleline(buffer)
                .id(id)
                .desired_width(value_rect.width())
                .text_color(material_text())
                .font(egui::TextStyle::Monospace),
        );
        text_edit_cursor_to_start_on_tab_focus(ui, &resp);
        if resp.lost_focus()
            && enabled
            && buffer.trim() != current.trim()
            && let Ok(value) = buffer.trim().parse::<f32>()
        {
            commit_value = Some(value);
        }
    });
    if let Some(value) = commit_value {
        h2_push_range_data_edit(
            edit,
            control,
            h2_function_data_with_range(data, true, Some(value)),
        );
    }
}

fn draw_h2_value_prefixed_text_edit(
    ui: &mut Ui,
    id: egui::Id,
    buffer: &mut String,
    width: f32,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.label(RichText::new("value:").color(material_text()).monospace());
        ui.add(
            egui::TextEdit::singleline(buffer)
                .id(id)
                .desired_width((width - 42.0).max(40.0))
                .text_color(material_text())
                .font(egui::TextStyle::Monospace),
        )
    })
    .inner
}

/// Render an editable widget inside a shader grid value cell and push a
/// `PendingFieldEdit` on commit. The leaf field type drives parsing in
/// `apply_field_edit`, so scalars/ints/refs all just emit the text.
/// Decode a referenced bitmap into a small thumbnail texture, cached in egui
/// memory keyed by ref path (Phase 4.2). `Some(None)` is cached for refs that
/// fail to load/decode so the decode isn't retried every frame.
fn shader_bitmap_thumbnail(
    ui: &Ui,
    edit: &FieldEditContext<'_>,
    group_tag: u32,
    open_ref: &str,
) -> Option<egui::TextureHandle> {
    let cache_id = egui::Id::new(("shader_bitmap_thumb", group_tag, open_ref));
    if let Some(cached) = ui.data(|d| d.get_temp::<Option<egui::TextureHandle>>(cache_id)) {
        return cached;
    }
    let decoded = decode_shader_bitmap_thumbnail(ui.ctx(), edit, group_tag, open_ref);
    ui.data_mut(|d| d.insert_temp(cache_id, decoded.clone()));
    decoded
}

fn decode_shader_bitmap_thumbnail(
    ctx: &egui::Context,
    edit: &FieldEditContext<'_>,
    group_tag: u32,
    open_ref: &str,
) -> Option<egui::TextureHandle> {
    let root = edit.tags_root?;
    let ext = blam_tags::paths::group_tag_to_extension(group_tag)?;
    let path = blam_tags::paths::resolve_tag_path(root, open_ref, ext);
    // Use the source-aware loader so classic (Halo CE / Halo 2) bitmaps decode
    // too — they need a JSON layout, not the plain `TagFile::read`.
    let tag =
        crate::source::read_tag_at_path(&path, edit.game, edit.definitions_root, group_tag).ok()?;
    let data = build_bitmap_preview(&tag, 0, 0).ok()?;
    // Cap at 256px: drawn small inline (GPU downscales) and at native size in the
    // hover preview popup, matching Foundation's 256px help-popup image.
    let (rgba, w, h) = downscale_rgba(&data.rgba, data.width, data.height, 256);
    if w == 0 || h == 0 {
        return None;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
    Some(ctx.load_texture(
        format!("shader_thumb:{group_tag}:{open_ref}"),
        image,
        egui::TextureOptions::LINEAR,
    ))
}

/// Nearest-neighbour downscale of an RGBA8 image to fit within `max` px.
pub(super) fn downscale_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    max: u32,
) -> (Vec<u8>, usize, usize) {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return (Vec::new(), 0, 0);
    }
    let scale = (max as f32 / w.max(h) as f32).min(1.0);
    let nw = ((w as f32 * scale).round() as usize).max(1);
    let nh = ((h as f32 * scale).round() as usize).max(1);
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        let sy = (y * h / nh).min(h - 1);
        for x in 0..nw {
            let sx = (x * w / nw).min(w - 1);
            let si = (sy * w + sx) * 4;
            let di = (y * nw + x) * 4;
            out[di..di + 4].copy_from_slice(&rgba[si..si + 4]);
        }
    }
    (out, nw, nh)
}

pub(in crate::app) fn draw_shader_editable_value(
    ui: &mut Ui,
    rect: egui::Rect,
    label: &str,
    row_edit: &ShaderRowEdit,
    edit: &mut FieldEditContext<'_>,
    color_popup: &mut Option<MaterialColorPopup>,
) {
    let buffer_key = format!("{}|{}", edit.tag_key, row_edit.path);
    match &row_edit.kind {
        ShaderRowEditKind::Enum(options) => {
            let current_idx = row_edit.current.parse::<usize>().unwrap_or(0);
            let selected_text = options
                .get(current_idx)
                .cloned()
                .unwrap_or_else(|| row_edit.current.clone());
            let mut chosen = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                let (_, wheel_delta) = combo_box_with_scroll(
                    ui,
                    egui::ComboBox::from_id_salt((
                        edit.view_scope,
                        edit.tag_key,
                        &buffer_key,
                        "shader_enum",
                    ))
                    .selected_text(selected_text)
                    .width(rect.width()),
                    |ui| {
                        for (i, opt) in options.iter().enumerate() {
                            if ui.selectable_label(i == current_idx, opt).clicked() {
                                chosen = Some(i);
                            }
                        }
                    },
                );
                if let Some(delta) = wheel_delta
                    && let Some(next) = combo_scroll_next_index(current_idx, options.len(), delta)
                {
                    chosen = Some(next);
                }
            });
            if let Some(i) = chosen {
                edit.pending.push(PendingFieldEdit {
                    path: row_edit.path.clone(),
                    input: i.to_string(),
                });
            }
        }

        ShaderRowEditKind::Flags(options) => {
            let current_mask = row_edit.current.trim().parse::<u64>().unwrap_or(0);
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (bit, option) in options.iter().enumerate() {
                        let mut checked = current_mask & (1u64 << bit) != 0;
                        let response = ui.add_enabled(
                            edit.editable,
                            egui::Checkbox::new(&mut checked, option.as_str()),
                        );
                        if response.changed() {
                            let mut next_mask = current_mask;
                            if checked {
                                next_mask |= 1u64 << bit;
                            } else {
                                next_mask &= !(1u64 << bit);
                            }
                            edit.pending.push(PendingFieldEdit {
                                path: row_edit.path.clone(),
                                input: next_mask.to_string(),
                            });
                        }
                    }
                });
            });
        }

        // Constant animated-parameter scalar: text box + × delete button.
        // The f() button to open the graph editor is rendered in draw_shader_grid_row
        // via constant_function_view, not here.
        ShaderRowEditKind::FunctionScalar {
            block_path,
            block_index,
        } => {
            let current = row_edit.current.clone();
            // Reserve 20px on the right for the × delete button.
            let del_rect = egui::Rect::from_min_size(
                rect.right_top() - Vec2::new(20.0, 0.0),
                Vec2::new(18.0, rect.height()),
            );
            let text_rect = egui::Rect::from_min_size(
                rect.left_top(),
                Vec2::new((rect.width() - 22.0).max(40.0), rect.height()),
            );
            let id = edit.widget_id(("shader_fn_scalar", &buffer_key));
            let buffer = edit
                .buffers
                .entry(buffer_key.clone())
                .or_insert_with(|| current.clone());
            if !ui.memory(|m| m.has_focus(id)) && *buffer != current {
                *buffer = current.clone();
            }
            let mut commit_val: Option<f32> = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                ui.visuals_mut().extreme_bg_color = material_input();
                let resp = ui.add(
                    egui::TextEdit::singleline(buffer)
                        .id(id)
                        .desired_width(text_rect.width())
                        .text_color(material_text())
                        .font(egui::TextStyle::Monospace),
                );
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                if resp.lost_focus() && buffer.trim() != current.trim() {
                    if let Ok(v) = buffer.trim().parse::<f32>() {
                        commit_val = Some(v);
                    }
                }
            });
            if let Some(v) = commit_val {
                edit.pending.push(PendingFieldEdit {
                    path: row_edit.path.clone(),
                    input: constant_function_hex(v),
                });
            }
            // × delete button
            ui.painter().rect_filled(del_rect, 0.0, material_input());
            ui.painter()
                .rect_stroke(del_rect, 0.0, Stroke::new(1.0, material_input_edge()));
            ui.painter().text(
                del_rect.center(),
                Align2::CENTER_CENTER,
                "×",
                FontId::proportional(13.0),
                material_delete_text(),
            );
            if ui
                .interact(
                    del_rect,
                    ui.make_persistent_id(format!("shader_scalar_del:{}", buffer_key)),
                    Sense::click(),
                )
                .on_hover_text("Remove animated parameter")
                .clicked()
            {
                edit.block_ops.push(BlockOp {
                    path: block_path.clone(),
                    kind: BlockOpKind::Delete(*block_index),
                });
            }
        }

        // BitmapRef → text box + Open + "..." browse button.
        ShaderRowEditKind::BitmapRef { group_tag, create } => {
            let current = row_edit.current.clone();
            // Reserve the right edge: "..." browse (24px) then Open (40px).
            let browse_rect = egui::Rect::from_min_size(
                rect.right_top() - Vec2::new(26.0, 0.0),
                Vec2::new(24.0, rect.height()),
            );
            let open_rect = egui::Rect::from_min_size(
                rect.right_top() - Vec2::new(70.0, 0.0),
                Vec2::new(40.0, rect.height()),
            );
            // The grid stores the path with a ".bitmap" suffix and forward
            // slashes; strip both so it resolves like a normal tag reference.
            let cleaned = sanitize_ref_path(&current);
            let open_ref = cleaned
                .strip_suffix(".bitmap")
                .unwrap_or(&cleaned)
                .replace('/', "\\");
            let open_enabled = !open_ref.is_empty() && open_ref != "NONE";
            // Inline thumbnail of the referenced bitmap (Phase 4.2), at the left.
            let thumb = open_enabled
                .then(|| shader_bitmap_thumbnail(ui, edit, *group_tag, &open_ref))
                .flatten();
            let (thumb_w, thumb_gap) = if thumb.is_some() {
                (rect.height() - 2.0, 4.0)
            } else {
                (0.0, 0.0)
            };
            if let Some(texture) = &thumb {
                let thumb_rect = egui::Rect::from_min_size(
                    rect.left_top() + Vec2::new(0.0, 1.0),
                    Vec2::splat(rect.height() - 2.0),
                );
                ui.painter().image(
                    texture.id(),
                    thumb_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                // Hover → enlarged preview popup (up to native, ≤256px) + path,
                // mirroring Foundation's help-popup image.
                ui.interact(
                    thumb_rect,
                    ui.make_persistent_id(("shader_thumb_hover", &open_ref)),
                    Sense::hover(),
                )
                .on_hover_ui(|ui| {
                    let native = texture.size_vec2();
                    let scale = (256.0 / native.x.max(native.y).max(1.0)).min(1.0);
                    ui.add(egui::Image::new(egui::load::SizedTexture::new(
                        texture.id(),
                        native * scale,
                    )));
                    ui.label(
                        RichText::new(&open_ref)
                            .small()
                            .color(material_muted_text()),
                    );
                });
            }
            let text_rect = egui::Rect::from_min_size(
                rect.left_top() + Vec2::new(thumb_w + thumb_gap, 0.0),
                Vec2::new(
                    (rect.width() - 72.0 - thumb_w - thumb_gap).max(40.0),
                    rect.height(),
                ),
            );
            // Open the referenced bitmap in a new tab (when the ref is set).
            ui.painter().rect_filled(
                open_rect,
                0.0,
                if open_enabled {
                    material_input()
                } else {
                    material_disabled_input()
                },
            );
            ui.painter()
                .rect_stroke(open_rect, 0.0, Stroke::new(1.0, material_input_edge()));
            let icon_rect = egui::Rect::from_center_size(open_rect.center(), Vec2::splat(16.0));
            let icon_color = if open_enabled {
                material_text()
            } else {
                material_muted_text()
            };
            paint_button_icon_at(ui, ButtonIcon::Open, icon_rect, icon_color);
            if open_enabled
                && ui
                    .interact(
                        open_rect,
                        ui.make_persistent_id(format!("shader_bitmap_open:{}", buffer_key)),
                        Sense::click(),
                    )
                    .on_hover_text("Open the referenced bitmap tag (Alt: floating window)")
                    .clicked()
            {
                let float = ui.input(|i| i.modifiers.alt);
                *edit.open_request = Some(OpenTagRequest {
                    group_tag: *group_tag,
                    rel_path: open_ref.clone(),
                    float,
                });
            }
            let id = edit.widget_id(("shader_text", &buffer_key));
            let buffer = edit
                .buffers
                .entry(buffer_key.clone())
                .or_insert_with(|| current.clone());
            if !ui.memory(|m| m.has_focus(id)) && *buffer != current {
                *buffer = current.clone();
            }
            // Flag a referenced bitmap that is missing on disk (red text).
            let missing = open_enabled
                && reference_target_missing(edit.names, edit.tags_root, *group_tag, &open_ref);
            let text_color = if missing {
                REFERENCE_MISSING_COLOR
            } else {
                material_text()
            };
            let mut commit = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                ui.visuals_mut().extreme_bg_color = material_input();
                let resp = ui.add(
                    egui::TextEdit::singleline(buffer)
                        .id(id)
                        .desired_width(text_rect.width())
                        .hint_text("(no reference)")
                        .text_color(text_color)
                        .font(egui::TextStyle::Monospace),
                );
                if missing {
                    resp.clone()
                        .on_hover_text("Referenced bitmap not found on disk");
                }
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                if resp.lost_focus() && buffer.trim() != current.trim() {
                    commit = Some(buffer.trim().to_owned());
                }
            });
            if let Some(input) = commit {
                push_shader_value_edit(edit, row_edit, create.as_ref(), input);
            }
            // Drag-and-drop: drop a bitmap tag from the browser onto the cell to
            // set the reference. Accept only bitmap-group tags.
            if edit.editable {
                let drop = ui.interact(
                    text_rect,
                    ui.make_persistent_id(("shader_bitmap_drop", &buffer_key)),
                    Sense::hover(),
                );
                let is_bitmap =
                    |payload: &DraggedTagRef| &payload.group_tag.to_be_bytes() == b"bitm";
                if let Some(payload) = drop.dnd_hover_payload::<DraggedTagRef>() {
                    let color = if is_bitmap(&payload) {
                        Color32::from_rgb(120, 170, 90)
                    } else {
                        REFERENCE_MISSING_COLOR
                    };
                    ui.painter()
                        .rect_stroke(text_rect, 2.0, Stroke::new(1.5, color));
                }
                if let Some(payload) = drop.dnd_release_payload::<DraggedTagRef>() {
                    if is_bitmap(&payload) {
                        edit.buffers
                            .insert(buffer_key.clone(), payload.rel_path.clone());
                        push_shader_value_edit(
                            edit,
                            row_edit,
                            create.as_ref(),
                            payload.rel_path.clone(),
                        );
                    }
                }
            }
            // "..." browse button
            ui.painter().rect_filled(browse_rect, 0.0, material_input());
            ui.painter()
                .rect_stroke(browse_rect, 0.0, Stroke::new(1.0, material_input_edge()));
            ui.painter().text(
                browse_rect.center(),
                Align2::CENTER_CENTER,
                "...",
                FontId::proportional(11.0),
                material_text(),
            );
            if ui
                .interact(
                    browse_rect,
                    ui.make_persistent_id(format!("shader_bitmap_browse:{}", buffer_key)),
                    Sense::click(),
                )
                .on_hover_text("Browse for a .bitmap tag file")
                .clicked()
            {
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("Bitmap tag", &["bitmap"])
                    .set_title("Select Bitmap Tag");
                if let Some(tags_root) = edit.tags_root {
                    dialog = dialog.set_directory(tag_reference_start_dir(tags_root, &open_ref));
                }
                if let Some(path) = dialog.pick_file() {
                    match normalize_bitmap_browse_path(&path, edit.tags_root) {
                        Ok(rel) => {
                            let buf = edit.buffers.entry(buffer_key).or_insert_with(String::new);
                            *buf = rel.clone();
                            push_shader_value_edit(edit, row_edit, create.as_ref(), rel);
                        }
                        Err(error) => {
                            if let Some(status) = edit.status.as_deref_mut() {
                                *status = error;
                            }
                        }
                    }
                }
            }
        }

        // Shader template tag reference → text box + Open + "..." browse button.
        ShaderRowEditKind::ShaderTemplateRef => {
            let current = row_edit.current.clone();
            let browse_rect = egui::Rect::from_min_size(
                rect.right_top() - Vec2::new(26.0, 0.0),
                Vec2::new(24.0, rect.height()),
            );
            let open_rect = egui::Rect::from_min_size(
                rect.right_top() - Vec2::new(70.0, 0.0),
                Vec2::new(40.0, rect.height()),
            );
            let text_rect = egui::Rect::from_min_size(
                rect.left_top(),
                Vec2::new((rect.width() - 72.0).max(40.0), rect.height()),
            );
            let cleaned = sanitize_ref_path(&current);
            let open_ref = cleaned
                .strip_suffix(".shader_template")
                .unwrap_or(&cleaned)
                .replace('/', "\\");
            let open_enabled = !open_ref.is_empty() && open_ref != "NONE";
            ui.painter().rect_filled(
                open_rect,
                0.0,
                if open_enabled {
                    material_input()
                } else {
                    material_disabled_input()
                },
            );
            ui.painter()
                .rect_stroke(open_rect, 0.0, Stroke::new(1.0, material_input_edge()));
            let icon_rect = egui::Rect::from_center_size(open_rect.center(), Vec2::splat(16.0));
            let icon_color = if open_enabled {
                material_text()
            } else {
                material_muted_text()
            };
            paint_button_icon_at(ui, ButtonIcon::Open, icon_rect, icon_color);
            if open_enabled
                && ui
                    .interact(
                        open_rect,
                        ui.make_persistent_id(format!("shader_template_open:{}", buffer_key)),
                        Sense::click(),
                    )
                    .on_hover_text("Open the referenced shader_template tag")
                    .clicked()
            {
                *edit.open_request = Some(OpenTagRequest {
                    group_tag: u32::from_be_bytes(*b"stem"),
                    rel_path: open_ref.clone(),
                    float: ui.input(|i| i.modifiers.alt),
                });
            }

            let id = edit.widget_id(("shader_template_text", &buffer_key));
            let buffer = edit
                .buffers
                .entry(buffer_key.clone())
                .or_insert_with(|| current.clone());
            if !ui.memory(|m| m.has_focus(id)) && *buffer != current {
                *buffer = current.clone();
            }
            let mut commit = None;
            let text_response = ui
                .scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                    ui.visuals_mut().extreme_bg_color = material_input();
                    let resp = ui.add(
                        egui::TextEdit::singleline(buffer)
                            .id(id)
                            .desired_width(text_rect.width())
                            .text_color(material_text())
                            .font(egui::TextStyle::Monospace),
                    );
                    text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                    if resp.lost_focus() && buffer.trim() != current.trim() {
                        commit = Some(buffer.trim().to_owned());
                    }
                    resp
                })
                .inner;
            // Drop a shader_template tag from the browser onto the cell.
            let shader_template_group = u32::from_be_bytes(*b"stem");
            let template_ok = |payload: &DraggedTagRef| payload.group_tag == shader_template_group;
            if let Some(payload) = text_response.dnd_hover_payload::<DraggedTagRef>() {
                let color = if template_ok(&payload) {
                    Color32::from_rgb(120, 170, 90)
                } else {
                    REFERENCE_MISSING_COLOR
                };
                ui.painter()
                    .rect_stroke(text_response.rect, 2.0, Stroke::new(1.5, color));
            }
            if edit.editable {
                if let Some(payload) = text_response.dnd_release_payload::<DraggedTagRef>() {
                    if template_ok(&payload) {
                        commit = Some(payload.rel_path.clone());
                    }
                }
            }
            if let Some(input) = commit {
                push_h2_template_reference_edit(edit, row_edit, input);
            }

            ui.painter().rect_filled(browse_rect, 0.0, material_input());
            ui.painter()
                .rect_stroke(browse_rect, 0.0, Stroke::new(1.0, material_input_edge()));
            ui.painter().text(
                browse_rect.center(),
                Align2::CENTER_CENTER,
                "...",
                FontId::proportional(11.0),
                material_text(),
            );
            if ui
                .interact(
                    browse_rect,
                    ui.make_persistent_id(format!("shader_template_browse:{}", buffer_key)),
                    Sense::click(),
                )
                .on_hover_text("Browse for a .shader_template tag file")
                .clicked()
            {
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("Shader template tag", &["shader_template", "stem"])
                    .set_title("Select Shader Template Tag");
                if let Some(tags_root) = edit.tags_root {
                    dialog = dialog.set_directory(tag_reference_start_dir(tags_root, &open_ref));
                }
                if let Some(path) = dialog.pick_file() {
                    match normalize_shader_template_browse_path(&path, edit.tags_root) {
                        Ok(rel) => {
                            let buf = edit.buffers.entry(buffer_key).or_insert_with(String::new);
                            *buf = rel.clone();
                            push_h2_template_reference_edit(edit, row_edit, rel);
                        }
                        Err(error) => {
                            if let Some(status) = edit.status.as_deref_mut() {
                                *status = error;
                            }
                        }
                    }
                }
            }
        }

        ShaderRowEditKind::Bool { create } => {
            let current_raw = row_edit.current.trim().parse::<i32>().unwrap_or(0);
            let mut checked = current_raw != 0;
            let response = ui
                .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.add_enabled(edit.editable, egui::Checkbox::new(&mut checked, ""))
                })
                .inner;
            if response.changed() {
                push_shader_value_edit(
                    edit,
                    row_edit,
                    create.as_ref(),
                    if checked { "1" } else { "0" }.to_owned(),
                );
            }
        }

        // Constant color animated parameter: clickable swatch → editable color popup + × delete.
        ShaderRowEditKind::FunctionColor {
            block_path,
            block_index,
        } => {
            let del_rect = egui::Rect::from_min_size(
                rect.right_top() - Vec2::new(20.0, 0.0),
                Vec2::new(18.0, rect.height()),
            );
            let swatch_rect = egui::Rect::from_min_size(
                rect.left_top(),
                Vec2::new((rect.width() - 22.0).max(30.0), rect.height()),
            );
            // Parse current "r,g,b,a" into a color.
            let parts: Vec<f32> = row_edit
                .current
                .split(',')
                .filter_map(|s| s.trim().parse::<f32>().ok())
                .collect();
            let (r, g, b, a) = if parts.len() == 4 {
                (parts[0], parts[1], parts[2], parts[3])
            } else {
                (1.0, 1.0, 1.0, 1.0)
            };
            let color32 = Color32::from_rgba_unmultiplied(
                float_channel_to_u8(r),
                float_channel_to_u8(g),
                float_channel_to_u8(b),
                float_channel_to_u8(a),
            );
            draw_shader_color_swatch(ui, swatch_rect, color32);
            let inner = swatch_rect.shrink(3.0);
            ui.painter().text(
                swatch_rect.left_center() + Vec2::new(inner.width() + 8.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                FontId::monospace(12.0),
                material_text(),
            );
            if ui
                .interact(
                    swatch_rect,
                    ui.make_persistent_id(format!("shader_color_edit:{label}")),
                    Sense::click(),
                )
                .on_hover_text("Click to edit color")
                .clicked()
            {
                *color_popup = Some(
                    MaterialColorPopup::new(label, r, g, b, a)
                        .with_write(edit.tag_key, row_edit.path.clone()),
                );
            }
            // × delete button
            ui.painter().rect_filled(del_rect, 0.0, material_input());
            ui.painter()
                .rect_stroke(del_rect, 0.0, Stroke::new(1.0, material_input_edge()));
            ui.painter().text(
                del_rect.center(),
                Align2::CENTER_CENTER,
                "×",
                FontId::proportional(13.0),
                material_delete_text(),
            );
            if ui
                .interact(
                    del_rect,
                    ui.make_persistent_id(format!("shader_color_del:{label}")),
                    Sense::click(),
                )
                .on_hover_text("Remove color animated parameter")
                .clicked()
            {
                edit.block_ops.push(BlockOp {
                    path: block_path.clone(),
                    kind: BlockOpKind::Delete(*block_index),
                });
            }
        }

        ShaderRowEditKind::ColorField { argb } => {
            let parts: Vec<f32> = row_edit
                .current
                .split(',')
                .filter_map(|s| s.trim().parse::<f32>().ok())
                .collect();
            let (r, g, b, a) = if parts.len() == 4 {
                (parts[0], parts[1], parts[2], parts[3])
            } else {
                (1.0, 1.0, 1.0, 1.0)
            };
            let color32 = Color32::from_rgba_unmultiplied(
                float_channel_to_u8(r),
                float_channel_to_u8(g),
                float_channel_to_u8(b),
                float_channel_to_u8(a),
            );
            draw_shader_color_swatch(ui, rect, color32);
            let inner = rect.shrink(3.0);
            ui.painter().text(
                rect.left_center() + Vec2::new(inner.width() + 8.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                FontId::monospace(12.0),
                material_text(),
            );
            if ui
                .interact(
                    rect,
                    ui.make_persistent_id(format!("shader_color_field:{label}")),
                    Sense::click(),
                )
                .on_hover_text("Click to edit color")
                .clicked()
            {
                *color_popup = Some(MaterialColorPopup::new(label, r, g, b, a).with_color_field(
                    edit.tag_key,
                    row_edit.path.clone(),
                    *argb,
                ));
            }
        }

        ShaderRowEditKind::CreateFunctionColor { target } => {
            let parts: Vec<f32> = row_edit
                .current
                .split(',')
                .filter_map(|s| s.trim().parse::<f32>().ok())
                .collect();
            let (r, g, b, a) = if parts.len() == 4 {
                (parts[0], parts[1], parts[2], parts[3])
            } else {
                (1.0, 1.0, 1.0, 1.0)
            };
            let color32 = Color32::from_rgba_unmultiplied(
                float_channel_to_u8(r),
                float_channel_to_u8(g),
                float_channel_to_u8(b),
                float_channel_to_u8(a),
            );
            draw_shader_color_swatch(ui, rect, color32);
            let inner = rect.shrink(3.0);
            ui.painter().text(
                rect.left_center() + Vec2::new(inner.width() + 8.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                FontId::monospace(12.0),
                material_text(),
            );
            if ui
                .interact(
                    rect,
                    ui.make_persistent_id(format!("shader_color_create:{label}")),
                    Sense::click(),
                )
                .on_hover_text("Click to edit color")
                .clicked()
            {
                *color_popup = Some(shader_color_create_popup(
                    edit.tag_key,
                    label,
                    r,
                    g,
                    b,
                    a,
                    target,
                ));
            }
        }

        ShaderRowEditKind::CreateFunctionScalar { target } => {
            let current = row_edit.current.clone();
            let create_buf_key = format!("{}|create_fn_scalar:{label}", edit.tag_key);
            let id = edit.widget_id(("shader_create_fn_scalar", label));
            let buffer = edit
                .buffers
                .entry(create_buf_key)
                .or_insert_with(|| current.clone());
            if !ui.memory(|m| m.has_focus(id)) && *buffer != current {
                *buffer = current.clone();
            }
            let mut commit_val: Option<f32> = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.visuals_mut().extreme_bg_color = material_pending_input();
                let resp = ui.add(
                    egui::TextEdit::singleline(buffer)
                        .id(id)
                        .desired_width(rect.width())
                        .text_color(material_text())
                        .font(egui::TextStyle::Monospace),
                );
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                if resp.lost_focus() && buffer.trim() != current.trim() {
                    if let Ok(v) = buffer.trim().parse::<f32>() {
                        commit_val = Some(v);
                    }
                }
            });
            if let Some(v) = commit_val {
                let action = shader_function_action(target, constant_function_hex(v));
                push_shader_context_action(edit, &action);
            }
        }

        ShaderRowEditKind::H2FunctionColor {
            block_path,
            legacy_data,
        } => {
            let parts: Vec<f32> = row_edit
                .current
                .split(',')
                .filter_map(|s| s.trim().parse::<f32>().ok())
                .collect();
            let (r, g, b, a) = if parts.len() == 4 {
                (parts[0], parts[1], parts[2], parts[3])
            } else {
                (1.0, 1.0, 1.0, 1.0)
            };
            let color32 = Color32::from_rgba_unmultiplied(
                float_channel_to_u8(r),
                float_channel_to_u8(g),
                float_channel_to_u8(b),
                float_channel_to_u8(a),
            );
            draw_shader_color_swatch(ui, rect, color32);
            let inner = rect.shrink(3.0);
            ui.painter().text(
                rect.left_center() + Vec2::new(inner.width() + 8.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                FontId::monospace(12.0),
                material_text(),
            );
            if ui
                .interact(
                    rect,
                    ui.make_persistent_id(format!("h2_shader_color_fn:{label}")),
                    Sense::click(),
                )
                .on_hover_text("Click to edit H2 color function")
                .clicked()
            {
                *color_popup = Some(
                    MaterialColorPopup::new(label, r, g, b, a).with_h2_shader_param_op(
                        edit.tag_key,
                        H2ShaderParamOp::EditFunctionData {
                            block_path: block_path.clone(),
                            data: h2_constant_color_function_data(
                                r,
                                g,
                                b,
                                a,
                                legacy_data.as_deref(),
                            ),
                        },
                    ),
                );
            }
        }

        ShaderRowEditKind::H2CreateFunctionColor { create_op } => {
            let parts: Vec<f32> = row_edit
                .current
                .split(',')
                .filter_map(|s| s.trim().parse::<f32>().ok())
                .collect();
            let (r, g, b, a) = if parts.len() == 4 {
                (parts[0], parts[1], parts[2], parts[3])
            } else {
                (1.0, 1.0, 1.0, 1.0)
            };
            let color32 = Color32::from_rgba_unmultiplied(
                float_channel_to_u8(r),
                float_channel_to_u8(g),
                float_channel_to_u8(b),
                float_channel_to_u8(a),
            );
            draw_shader_color_swatch(ui, rect, color32);
            let inner = rect.shrink(3.0);
            ui.painter().text(
                rect.left_center() + Vec2::new(inner.width() + 8.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                FontId::monospace(12.0),
                material_text(),
            );
            if ui
                .interact(
                    rect,
                    ui.make_persistent_id(format!("h2_shader_color_create_fn:{label}")),
                    Sense::click(),
                )
                .on_hover_text("Click to edit H2 color function")
                .clicked()
            {
                *color_popup = Some(
                    MaterialColorPopup::new(label, r, g, b, a)
                        .with_h2_shader_param_op(edit.tag_key, create_op.clone()),
                );
            }
        }

        ShaderRowEditKind::H2FunctionScalar {
            block_path,
            legacy_data,
        } => {
            let current = row_edit.current.clone();
            let id = edit.widget_id(("h2_shader_fn_scalar", &buffer_key));
            let buffer = edit
                .buffers
                .entry(buffer_key.clone())
                .or_insert_with(|| current.clone());
            if !ui.memory(|m| m.has_focus(id)) && *buffer != current {
                *buffer = current.clone();
            }
            let mut commit_val: Option<f32> = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.visuals_mut().extreme_bg_color = material_input();
                let resp = draw_h2_value_prefixed_text_edit(ui, id, buffer, rect.width());
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                if resp.lost_focus() && buffer.trim() != current.trim() {
                    if let Ok(v) = buffer.trim().parse::<f32>() {
                        commit_val = Some(v);
                    }
                }
            });
            if let Some(v) = commit_val {
                edit.h2_shader_param_ops
                    .push(H2ShaderParamOp::EditFunctionData {
                        block_path: block_path.clone(),
                        data: h2_constant_scalar_function_data(v, legacy_data.as_deref()),
                    });
            }
        }

        ShaderRowEditKind::H2CreateFunctionScalar { create_op } => {
            let current = row_edit.current.clone();
            let create_buf_key = format!("{}|{}", edit.tag_key, row_edit.path);
            let id = edit.widget_id(("h2_shader_create_fn_scalar", label));
            let buffer = edit
                .buffers
                .entry(create_buf_key)
                .or_insert_with(|| current.clone());
            if !ui.memory(|m| m.has_focus(id)) && *buffer != current {
                *buffer = current.clone();
            }
            let mut commit_val: Option<f32> = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.visuals_mut().extreme_bg_color = material_pending_input();
                let resp = draw_h2_value_prefixed_text_edit(ui, id, buffer, rect.width());
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                if resp.lost_focus() && buffer.trim() != current.trim() {
                    if let Ok(v) = buffer.trim().parse::<f32>() {
                        commit_val = Some(v);
                    }
                }
            });
            if let Some(v) = commit_val {
                let mut op = create_op.clone();
                if let H2ShaderParamOp::EnsureAnimationProperty {
                    initial_function_data,
                    ..
                } = &mut op
                {
                    *initial_function_data =
                        decode_hex(&constant_function_hex(v)).unwrap_or_default();
                }
                edit.h2_shader_param_ops.push(op);
            }
        }

        // No instance yet: text box for default value; on commit create the parameter entry.
        ShaderRowEditKind::CreateScalarParam {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
        } => {
            let current = row_edit.current.clone();
            let create_buf_key = format!("{}|create:{label}", edit.tag_key);
            let id = edit.widget_id(("shader_create_scalar", label));
            let buffer = edit
                .buffers
                .entry(create_buf_key)
                .or_insert_with(|| current.clone());
            if !ui.memory(|m| m.has_focus(id)) && *buffer != current {
                *buffer = current.clone();
            }
            let mut commit_val: Option<f32> = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.visuals_mut().extreme_bg_color = material_pending_input();
                let resp = ui.add(
                    egui::TextEdit::singleline(buffer)
                        .id(id)
                        .desired_width(rect.width())
                        .text_color(material_text())
                        .font(egui::TextStyle::Monospace),
                );
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                if resp.lost_focus() && buffer.trim() != current.trim() {
                    if let Ok(v) = buffer.trim().parse::<f32>() {
                        commit_val = Some(v);
                    }
                }
            });
            if let Some(v) = commit_val {
                edit.shader_param_ops.push(ShaderParamOp {
                    parameters_block_path: parameters_block_path.clone(),
                    parameter_name: parameter_name.clone(),
                    initial_fields: vec![
                        shader_parameter_type_initial_field(*parameter_type_index),
                        ShaderParamInitialField {
                            field: "real".to_owned(),
                            input: v.to_string(),
                        },
                    ],
                    animated_parameters: Vec::new(),
                });
            }
        }

        ShaderRowEditKind::H2CreateTemplateValue {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
            field,
        } => {
            let current = row_edit.current.clone();
            let create_buf_key = format!("{}|h2_create:{label}", edit.tag_key);
            let id = edit.widget_id(("h2_shader_create_value", label));
            let buffer = edit
                .buffers
                .entry(create_buf_key)
                .or_insert_with(|| current.clone());
            if !ui.memory(|m| m.has_focus(id)) && *buffer != current {
                *buffer = current.clone();
            }
            let mut commit = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.visuals_mut().extreme_bg_color = material_pending_input();
                let resp = draw_h2_value_prefixed_text_edit(ui, id, buffer, rect.width());
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                if resp.lost_focus() && buffer.trim() != current.trim() {
                    commit = Some(buffer.trim().to_owned());
                }
            });
            if let Some(input) = commit {
                edit.h2_shader_param_ops
                    .push(H2ShaderParamOp::EditTemplateBackedValue {
                        parameters_block_path: parameters_block_path.clone(),
                        parameter_name: parameter_name.clone(),
                        parameter_type_index: *parameter_type_index,
                        field: field.clone(),
                        input: h2_template_value_input(field, &input),
                    });
            }
        }

        ShaderRowEditKind::H2CreateTemplateColor {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
            field,
        } => {
            let parts: Vec<f32> = row_edit
                .current
                .split(',')
                .filter_map(|s| s.trim().parse::<f32>().ok())
                .collect();
            let (r, g, b, a) = if parts.len() == 4 {
                (parts[0], parts[1], parts[2], parts[3])
            } else {
                (0.0, 0.0, 0.0, 1.0)
            };
            let color32 = Color32::from_rgba_unmultiplied(
                float_channel_to_u8(r),
                float_channel_to_u8(g),
                float_channel_to_u8(b),
                float_channel_to_u8(a),
            );
            draw_shader_color_swatch(ui, rect, color32);
            if ui
                .interact(
                    rect,
                    ui.make_persistent_id(format!("h2_shader_color_create:{label}")),
                    Sense::click(),
                )
                .on_hover_text("Click to edit color")
                .clicked()
            {
                *color_popup = Some(
                    MaterialColorPopup::new(label, r, g, b, a).with_h2_shader_param_op(
                        edit.tag_key,
                        H2ShaderParamOp::EditTemplateBackedValue {
                            parameters_block_path: parameters_block_path.clone(),
                            parameter_name: parameter_name.clone(),
                            parameter_type_index: *parameter_type_index,
                            field: field.clone(),
                            input: format!("{r}, {g}, {b}"),
                        },
                    ),
                );
            }
        }

        // Scalar / Int / StringId → plain single-line text box.
        ShaderRowEditKind::Scalar | ShaderRowEditKind::Int | ShaderRowEditKind::StringId => {
            let current = row_edit.current.clone();
            let id = edit.widget_id(("shader_text", &buffer_key));
            let buffer = edit
                .buffers
                .entry(buffer_key.clone())
                .or_insert_with(|| current.clone());
            if !ui.memory(|m| m.has_focus(id)) && *buffer != current {
                *buffer = current.clone();
            }
            let mut commit = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.visuals_mut().extreme_bg_color = material_input();
                let resp = ui.add(
                    egui::TextEdit::singleline(buffer)
                        .id(id)
                        .desired_width(rect.width())
                        .text_color(material_text())
                        .font(egui::TextStyle::Monospace),
                );
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                if resp.lost_focus() && buffer.trim() != current.trim() {
                    commit = Some(buffer.trim().to_owned());
                }
            });
            if let Some(input) = commit {
                edit.pending.push(PendingFieldEdit {
                    path: row_edit.path.clone(),
                    input,
                });
            }
        }
    }
}

pub(in crate::app) fn draw_shader_color_swatch(ui: &mut Ui, rect: egui::Rect, color: Color32) {
    let display_color = Color32::from_rgb(color.r(), color.g(), color.b());
    ui.painter().rect_filled(rect, 0.0, material_input());
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, material_input_edge()));
    let inner = rect.shrink(3.0);
    ui.painter().rect_filled(inner, 0.0, display_color);
    ui.painter().rect_stroke(
        inner,
        0.0,
        Stroke::new(1.25, material_color_swatch_edge(display_color)),
    );
}

pub(in crate::app) fn push_shader_value_edit(
    edit: &mut FieldEditContext<'_>,
    row_edit: &ShaderRowEdit,
    create: Option<&ShaderParamCreateTarget>,
    input: String,
) {
    if let Some(create) = create {
        edit.shader_param_ops.push(ShaderParamOp {
            parameters_block_path: create.parameters_block_path.clone(),
            parameter_name: create.parameter_name.clone(),
            initial_fields: vec![
                shader_parameter_type_initial_field(create.parameter_type_index),
                ShaderParamInitialField {
                    field: create.field.to_owned(),
                    input,
                },
            ],
            animated_parameters: Vec::new(),
        });
    } else {
        edit.pending.push(PendingFieldEdit {
            path: row_edit.path.clone(),
            input,
        });
    }
}

fn push_h2_template_reference_edit(
    edit: &mut FieldEditContext<'_>,
    row_edit: &ShaderRowEdit,
    input: String,
) {
    let normalized = h2_normalize_shader_template_reference(&sanitize_ref_path(&input));
    let pending_input = if normalized.is_empty() || normalized.eq_ignore_ascii_case("none") {
        "none".to_owned()
    } else {
        format!("stem:{}", normalized.replace('/', "\\"))
    };
    edit.pending.push(PendingFieldEdit {
        path: row_edit.path.clone(),
        input: pending_input,
    });

    if let Some(tags_root) = edit.tags_root {
        if let Some(allowed_parameter_names) =
            h2_template_parameter_names_from_reference(tags_root, &normalized)
        {
            edit.h2_shader_param_ops
                .push(H2ShaderParamOp::SwitchTemplate {
                    parameters_block_path: "parameters".to_owned(),
                    allowed_parameter_names,
                });
        }
    }
}

fn h2_template_parameter_names_from_reference(
    tags_root: &std::path::Path,
    reference: &str,
) -> Option<Vec<String>> {
    let rel = reference.replace('/', "\\");
    let path = tags_root.join(format!("{rel}.shader_template"));
    h2_template_parameter_names_from_file(&path)
}

fn h2_template_parameter_names_from_file(path: &std::path::Path) -> Option<Vec<String>> {
    let bytes = std::fs::read(path).ok()?;
    blam_tags::classic::ClassicHeader::parse(&bytes)?;
    let schema_path = locate_definitions_root()
        .join("halo2_mcc")
        .join("shader_template.json");
    let layout = blam_tags::TagLayout::from_json(schema_path).ok()?;
    let tag = blam_tags::classic::read_classic_tag_file(&bytes, layout).ok()?;
    Some(h2_template_parameter_names(tag.root()))
}

fn h2_template_parameter_names(root: TagStruct<'_>) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(categories) = root.field("categories").and_then(|field| field.as_block()) {
        for category in categories.iter() {
            if let Some(parameters) = category
                .field("parameters")
                .and_then(|field| field.as_block())
            {
                for parameter in parameters.iter() {
                    let name = h2_template_parameter_name(parameter);
                    if !name.is_empty() {
                        names.push(name);
                    }
                }
            }
        }
    }
    names
}

fn h2_template_value_input(field: &str, input: &str) -> String {
    if field == "bitmap"
        && !input.eq_ignore_ascii_case("none")
        && !input.trim().is_empty()
        && !input.contains(':')
        && !input
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("bitmap"))
    {
        format!("bitm:{input}")
    } else {
        input.to_owned()
    }
}

pub(in crate::app) fn shader_color_create_popup(
    tag_key: &str,
    label: &str,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
    target: &ShaderFunctionCreateTarget,
) -> MaterialColorPopup {
    let popup = MaterialColorPopup::new(label, r, g, b, a);
    match target {
        ShaderFunctionCreateTarget::ExistingParameter {
            animated_block_path,
            output_type_index,
        } => popup.with_shader_op(
            tag_key,
            ShaderOp {
                animated_block_path: animated_block_path.clone(),
                output_type_index: *output_type_index,
                initial_function_hex: constant_color_function_hex(r, g, b, a),
            },
        ),
        ShaderFunctionCreateTarget::NewParameter {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
            output_type_index,
        } => popup.with_shader_param_op(
            tag_key,
            ShaderParamOp {
                parameters_block_path: parameters_block_path.clone(),
                parameter_name: parameter_name.clone(),
                initial_fields: vec![shader_parameter_type_initial_field(*parameter_type_index)],
                animated_parameters: vec![ShaderParamInitialAnimated {
                    output_type_index: *output_type_index,
                    initial_function_hex: constant_color_function_hex(r, g, b, a),
                }],
            },
        ),
    }
}

/// Convert an absolute `.bitmap` file path from the OS file-picker into the
/// tag-reference path format used inside shader tags: tags-root-relative with
/// the `.bitmap` extension preserved.
pub(in crate::app) fn normalize_bitmap_browse_path(
    path: &std::path::Path,
    tags_root: Option<&std::path::Path>,
) -> Result<String, String> {
    let Some(root) = tags_root else {
        return Err("Selected file must be inside the tags folder".to_owned());
    };
    tag_reference_relative_path_with_extension(path, root)
}

pub(in crate::app) fn normalize_shader_template_browse_path(
    path: &std::path::Path,
    tags_root: Option<&std::path::Path>,
) -> Result<String, String> {
    let normalized = normalize_bitmap_browse_path(path, tags_root)?;
    Ok(h2_normalize_shader_template_reference(&normalized))
}
