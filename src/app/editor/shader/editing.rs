//! Shader override, reset, range, and value-edit application helpers.
//! It owns shader-specific models, edits, and presentation helpers; generic field editing, and the commands that apply edits, belong elsewhere.

use super::*;

const SHADER_MODIFIED_ACCENT: Color32 = Color32::from_rgb(224, 158, 62);

/// Queue a shader row's commit, or put why it was refused on the status line.
/// A number box used to drop text that wasn't a number without a word.
fn push_shader_commit(edit: &mut FieldEditContext<'_>, commit: Result<DeferredOps, String>) {
    match commit {
        Ok(ops) => edit.push_ops(ops),
        Err(error) => {
            if let Some(status) = edit.status.as_deref_mut() {
                *status = error;
            }
        }
    }
}

/// The number typed into a shader box.
fn parse_shader_number(text: &str) -> Result<f32, String> {
    let text = text.trim();
    text.parse::<f32>()
        .map_err(|_| format!("{text:?} is not a number"))
}

/// How a shader row's one box commits without the row, from the same
/// function the row's own commit runs.
fn shader_draft_commit(
    tag_key: &str,
    buffer_key: &str,
    ops: impl Fn(&str) -> Result<DeferredOps, String> + 'static,
) -> DraftCommit {
    DraftCommit::new(tag_key, vec![buffer_key.to_owned()], move |texts| {
        ops(texts[0])
    })
}

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
    if row
        .function
        .as_ref()
        .or(row.constant_function_view.as_ref())
        .is_some_and(|view| TagFunctionEditor::from_function(view.function.clone()).is_ranged())
    {
        return true;
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

pub(in crate::app) enum ShaderFunctionReset {
    Field(PendingFieldEdit),
    Halo2(H2ShaderParamOp),
    Create(ShaderContextAction),
}

/// Reset function backing in place with the encoding required by its engine.
pub(in crate::app) fn shader_function_default_edit(
    row: &ShaderGridRow,
) -> Option<ShaderFunctionReset> {
    if let Some(ShaderRowEdit {
        kind: ShaderRowEditKind::CreateFunctionScalar { target },
        ..
    }) = row.edit.as_ref()
    {
        let value = row
            .default_cell
            .as_ref()?
            .text
            .rsplit(": ")
            .next()?
            .trim()
            .parse::<f32>()
            .ok()?;
        return Some(ShaderFunctionReset::Create(shader_function_action(
            target,
            constant_function_hex(value),
        )));
    }
    let storage = match row.edit.as_ref().map(|edit| &edit.kind) {
        Some(
            ShaderRowEditKind::FunctionScalar {
                block_path,
                block_index,
            }
            | ShaderRowEditKind::FunctionColor {
                block_path,
                block_index,
            },
        ) => FunctionDataStorage::DataField(format!("{block_path}[{block_index}]/function/data")),
        Some(
            ShaderRowEditKind::H2FunctionScalar { block_path, .. }
            | ShaderRowEditKind::H2FunctionColor { block_path, .. },
        ) => FunctionDataStorage::Halo2ByteBlock(block_path.clone()),
        _ => row
            .function
            .as_ref()
            .or(row.constant_function_view.as_ref())?
            .edit
            .as_ref()?
            .data
            .clone(),
    };
    let default = row.default_cell.as_ref()?;
    let color = default.color.as_ref().map(|color| {
        let rgba = color.color32();
        [
            byte_to_float(rgba.r()),
            byte_to_float(rgba.g()),
            byte_to_float(rgba.b()),
            byte_to_float(rgba.a()),
        ]
    });
    let value = if color.is_none() {
        Some(
            default
                .text
                .rsplit(": ")
                .next()?
                .trim()
                .parse::<f32>()
                .ok()?,
        )
    } else {
        None
    };
    match storage {
        FunctionDataStorage::DataField(path) => {
            let input = if let Some([r, g, b, a]) = color {
                constant_color_function_hex(r, g, b, a)
            } else {
                constant_function_hex(value?)
            };
            Some(ShaderFunctionReset::Field(PendingFieldEdit { path, input }))
        }
        FunctionDataStorage::Halo2ByteBlock(block_path) => {
            let data = if let Some([r, g, b, a]) = color {
                h2_constant_color_function_data(r, g, b, a, None)
            } else {
                h2_constant_scalar_function_data(value?, None)
            };
            Some(ShaderFunctionReset::Halo2(
                H2ShaderParamOp::EditFunctionData { block_path, data },
            ))
        }
    }
}

pub(super) const SHADER_ROW_RIGHT_PADDING: f32 = 14.0;

pub(super) fn shader_label_top_padding(ui: &Ui) -> f32 {
    let font = FontId::proportional(12.5);
    let height = ui
        .painter()
        .layout_no_wrap("flags".to_owned(), font, material_text())
        .size()
        .y;
    (BUTTON_HEIGHT + 8.0 - height) / 2.0
}

fn shader_reference_group(row: &ShaderGridRow) -> Option<u32> {
    match row.edit.as_ref().map(|edit| &edit.kind) {
        Some(
            ShaderRowEditKind::BitmapRef { group_tag, .. }
            | ShaderRowEditKind::StructuralRef { group_tag, .. },
        ) => Some(*group_tag),
        Some(ShaderRowEditKind::ShaderTemplateRef) => Some(u32::from_be_bytes(*b"stem")),
        _ => match row.value_cell.text.rsplit('.').next()? {
            "bitmap" => Some(u32::from_be_bytes(*b"bitm")),
            "render_method_template" => Some(u32::from_be_bytes(*b"rmt2")),
            "render_method_definition" => Some(u32::from_be_bytes(*b"rmdf")),
            "shader_template" => Some(u32::from_be_bytes(*b"stem")),
            _ => None,
        },
    }
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
                *initial_function_data = h2_constant_scalar_function_data(value, None);
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
        .clamp(120.0, 600.0)
}

pub(super) fn shader_default_width(ui: &Ui) -> f32 {
    ui.data(|d| d.get_temp::<f32>(egui::Id::new("shader_grid_default_width")))
        .unwrap_or(150.0)
        .clamp(60.0, 600.0)
}

pub(super) fn shader_grid_width(ui: &Ui) -> f32 {
    ui.data(|d| d.get_temp::<f32>(egui::Id::new("shader_grid_value_width")))
        .map(|value| shader_label_width(ui) + shader_default_width(ui) + value + 16.0)
        .unwrap_or_else(|| ui.available_width().max(780.0))
}

pub(super) fn draw_shader_columns_header(ui: &mut Ui) {
    let label = shader_label_width(ui);
    let default = shader_default_width(ui);
    let width = shader_grid_width(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::hover());
    ui.painter().rect_filled(rect, 4.0, foundation_block_bar());
    ui.painter().rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, foundation_block_edge()),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(label - 2.0, 0.0),
        Align2::RIGHT_CENTER,
        "field",
        FontId::proportional(12.0),
        material_muted_text(),
    );
    for (x, text) in [
        (label + 12.0, "default value"),
        (label + default + 16.0, "override value"),
    ] {
        ui.painter().text(
            rect.left_center() + Vec2::new(x, 0.0),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(12.0),
            material_muted_text(),
        );
    }
    for (name, x, current, min, max) in [
        ("shader_grid_label_width", label + 5.0, label, 120.0, 600.0),
        (
            "shader_grid_default_width",
            label + default + 9.0,
            default,
            60.0,
            600.0,
        ),
        (
            "shader_grid_value_width",
            width - 1.0,
            width - label - default - 16.0,
            240.0,
            2400.0,
        ),
    ] {
        let x = rect.left() + x;
        let handle = egui::Rect::from_center_size(
            egui::pos2(x, rect.center().y),
            Vec2::new(8.0, rect.height()),
        );
        let response = ui
            .interact(
                handle,
                ui.make_persistent_id(("shader_column_resize", name)),
                Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        ui.painter()
            .vline(x, rect.y_range(), Stroke::new(1.0, foundation_block_edge()));
        let start_id = egui::Id::new(("shader_column_drag_start", name));
        if response.drag_started() {
            ui.data_mut(|d| d.insert_temp(start_id, current));
        }
        if response.dragged() {
            let start = ui.data(|d| d.get_temp::<f32>(start_id)).unwrap_or(current);
            let next = (start + response.total_drag_delta().unwrap_or_default().x).clamp(min, max);
            ui.data_mut(|d| d.insert_temp(egui::Id::new(name), next));
        }
        if response.double_clicked() {
            ui.data_mut(|d| d.remove::<f32>(egui::Id::new(name)));
        }
    }
}

pub(super) fn shader_input_box(ui: &mut Ui, rect: egui::Rect) {
    let visuals = &ui.visuals().widgets.inactive;
    ui.painter()
        .rect_filled(rect, visuals.corner_radius, material_input());
    ui.painter().rect_stroke(
        rect,
        visuals.corner_radius,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
}

pub(super) fn shader_cell_scope<R>(
    ui: &mut Ui,
    rect: egui::Rect,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let inner = contents(&mut child);
    egui::InnerResponse {
        inner,
        response: child.response(),
    }
}

pub(super) fn shader_row_separator(ui: &Ui, rect: egui::Rect) {
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, foundation_block_edge()),
    );
}

pub(super) fn shader_reference_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Native buttons in shader cells share the app's theme, hover, and focus styles.
fn shader_action_button(
    ui: &mut Ui,
    rect: egui::Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    icon: ButtonIcon,
    label: &str,
    enabled: bool,
) -> egui::Response {
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
    child.spacing_mut().interact_size.y = BUTTON_HEIGHT;
    let color = if icon == ButtonIcon::Clear {
        material_delete_text()
    } else {
        text_dark()
    };
    let response = child
        .add_enabled_ui(enabled, |ui| {
            let size = if label.is_empty() {
                ICON_BUTTON_SIZE
            } else {
                rect.size()
            };
            let response = ui.add(egui::Button::new("").min_size(size));
            let icon_center = if label.is_empty() {
                response.rect.center()
            } else {
                egui::pos2(
                    response.rect.left() + BUTTON_TEXT_PADDING_X + BUTTON_ICON_SIZE / 2.0,
                    response.rect.center().y,
                )
            };
            let icon_rect =
                egui::Rect::from_center_size(icon_center, Vec2::splat(BUTTON_ICON_SIZE));
            paint_button_icon_at(ui, icon, icon_rect, color);
            if !label.is_empty() {
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
                });
                ui.painter().text(
                    egui::pos2(
                        icon_rect.right() + BUTTON_ICON_TEXT_GAP,
                        response.rect.center().y,
                    ),
                    Align2::LEFT_CENTER,
                    label,
                    egui::TextStyle::Button.resolve(ui.style()),
                    color,
                );
            }
            response
        })
        .inner;
    #[cfg(test)]
    if icon == ButtonIcon::Clear {
        ui.data_mut(|d| {
            let rects = d.get_temp_mut_or_default::<Vec<egui::Rect>>(egui::Id::new(
                "shader_clear_test_rects",
            ));
            rects.push(response.rect);
        });
    }
    response
}

/// Select the whole value when a numeric text field is double-clicked.
///
/// egui's native double-click selects a "word", and a decimal point splits
/// `0.0` into two of them — so a double-click grabbed only the fraction and
/// replacing a value took several clicks. Whole-value selection is what
/// egui's own DragValue does on interaction, and what a small value cell
/// wants; the path and reference fields keep word selection, which is useful
/// for editing one segment.
fn select_all_on_double_click(ui: &Ui, response: &egui::Response, text: &str) {
    if !response.double_clicked() {
        return;
    }
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            )));
        state.store(ui.ctx(), response.id);
    }
}

pub(in crate::app) fn draw_shader_grid_row(
    ui: &mut Ui,
    row: &ShaderGridRow,
    depth: usize,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    edit: &mut FieldEditContext<'_>,
) {
    draw_shader_grid_row_inner(ui, row, depth, color_popup, function_popup, edit, None);
}

pub(super) fn draw_unused_shader_grid_row(
    ui: &mut Ui,
    row: &ShaderGridRow,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    edit: &mut FieldEditContext<'_>,
    delete: &BlockOp,
) {
    draw_shader_grid_row_inner(ui, row, 0, color_popup, function_popup, edit, Some(delete));
}

fn draw_shader_grid_row_inner(
    ui: &mut Ui,
    row: &ShaderGridRow,
    depth: usize,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    edit: &mut FieldEditContext<'_>,
    unused_delete: Option<&BlockOp>,
) {
    let tag_key = edit.tag_key;
    let editable = edit.editable;
    let available = shader_grid_width(ui);
    let indent = depth as f32 * 10.0;
    let base_label_width = shader_label_width(ui);
    let label_width = (base_label_width - indent).max(110.0);
    let default_width = shader_default_width(ui);
    let show_override = editable
        && !row.is_overridden
        && row.edit.as_ref().is_some_and(|row_edit| {
            matches!(
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
        });
    let range_control = shader_range_control(row);
    let create_function = row.create_anim_op.clone().or_else(|| {
        let row_edit = row.edit.as_ref()?;
        match &row_edit.kind {
            ShaderRowEditKind::CreateFunctionScalar { target } => Some(shader_function_action(
                target,
                constant_function_hex(row_edit.current.parse::<f32>().ok()?),
            )),
            ShaderRowEditKind::CreateFunctionColor { target } => {
                let [r, g, b, a] = parse_shader_rgba(&row_edit.current)?;
                Some(shader_function_action(
                    target,
                    constant_color_function_hex(r, g, b, a),
                ))
            }
            _ => None,
        }
    });
    let function_capable = row.function.is_some()
        || row.constant_function_view.is_some()
        || range_control.is_some()
        || row.create_anim_op.is_some()
        || row.edit.as_ref().is_some_and(|edit| {
            matches!(
                edit.kind,
                ShaderRowEditKind::CreateFunctionScalar { .. }
                    | ShaderRowEditKind::CreateFunctionColor { .. }
                    | ShaderRowEditKind::H2CreateFunctionScalar { .. }
                    | ShaderRowEditKind::H2CreateFunctionColor { .. }
            )
        });
    let has_range_input = !show_override && function_capable;
    let ranged = !show_override
        && range_control.as_ref().is_some_and(|control| {
            control.function.color_graph_type() == ColorGraphType::Scalar
                && TagFunctionEditor::from_function(control.function.clone()).is_ranged()
        });
    let function_reset = (editable && unused_delete.is_none())
        .then(|| shader_function_default_edit(row))
        .flatten();
    let reset = (editable && function_reset.is_none())
        .then(|| unused_delete.cloned().or_else(|| reset_op_for_row(row)))
        .flatten();
    let has_clear = !show_override && (function_reset.is_some() || reset.is_some());
    let right_controls_width = if show_override {
        SHADER_ROW_RIGHT_PADDING
    } else if function_capable {
        // Keep value and range columns aligned even before function backing
        // exists, or when this row has no Clear action yet.
        SHADER_ROW_RIGHT_PADDING + BUTTON_HEIGHT * 2.0 + 4.0
    } else {
        shader_right_controls_width(row, false, has_clear)
    };
    let value_width =
        (available - label_width - indent - default_width - 16.0 - right_controls_width).max(40.0);
    let height = if ranged {
        64.0
    } else {
        shader_grid_row_height(ui, row, value_width)
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(available, height), Sense::click());
    let nonconstant_function = row
        .function
        .as_ref()
        .or(row.constant_function_view.as_ref())
        .is_some_and(|view| view.function.function_type() != FunctionType::Constant);
    let bitmap = matches!(
        row.parameter_type.as_deref(),
        Some("bitmap" | "tag reference")
    ) || row.edit.as_ref().is_some_and(|edit| {
        matches!(
            edit.kind,
            ShaderRowEditKind::BitmapRef { .. }
                | ShaderRowEditKind::ShaderTemplateRef
                | ShaderRowEditKind::StructuralRef { .. }
        )
    });
    let fill = if nonconstant_function {
        row.fill
    } else if bitmap {
        material_ref_row()
    } else {
        Color32::TRANSPARENT
    };
    let row_text = material_text();
    ui.painter().rect_filled(rect, 0.0, fill);
    let modified = row_differs_from_default(row);
    if modified || unused_delete.is_some() {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), Vec2::new(3.0, height)),
            0.0,
            if unused_delete.is_some() {
                material_delete_text()
            } else {
                SHADER_MODIFIED_ACCENT
            },
        );
    }

    let label_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(4.0 + indent, 0.0),
        Vec2::new(label_width, height),
    );
    if function_capable && !nonconstant_function {
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                rect.left_top() + Vec2::new(3.0, 0.0),
                label_rect.right_bottom(),
            ),
            0.0,
            material_function_row(),
        );
    }
    let flags_row = row
        .edit
        .as_ref()
        .is_some_and(|edit| matches!(edit.kind, ShaderRowEditKind::Flags(_)));
    let (label_position, label_alignment) = if ranged {
        (
            label_rect.right_top() + Vec2::new(-6.0, 16.0),
            Align2::RIGHT_CENTER,
        )
    } else if flags_row {
        (
            label_rect.right_top() + Vec2::new(-6.0, shader_label_top_padding(ui)),
            Align2::RIGHT_TOP,
        )
    } else {
        (
            label_rect.right_center() - Vec2::new(6.0, 0.0),
            Align2::RIGHT_CENTER,
        )
    };
    ui.painter().text(
        label_position,
        label_alignment,
        truncate_for_cell(&row.label, label_width - 12.0),
        FontId::proportional(12.5),
        if unused_delete.is_some() {
            material_delete_text()
        } else {
            row_text
        },
    );
    // Per-parameter "help": hovering the label shows the full (untruncated) name
    // plus its parameter type — rmop parameters carry no description text.
    if unused_delete.is_some() || truncate_for_cell(&row.label, label_width - 12.0) != row.label {
        let hover = if unused_delete.is_some() {
            format!(
                "{}\nUnused in the current shader template. Clear removes this saved parameter and its functions.",
                row.label
            )
        } else {
            match row.parameter_type.as_deref() {
                Some(parameter_type) => format!("{}\n{}", row.label, parameter_type),
                None => row.label.clone(),
            }
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
            Stroke::new(1.0_f32, row_text),
        );
    }
    if split_resp.dragged() {
        let new_width = (base_label_width + split_resp.drag_delta().x).clamp(120.0, 600.0);
        ui.data_mut(|d| d.insert_temp(shader_label_width_id(), new_width));
    }

    let default_rect = egui::Rect::from_min_size(
        label_rect.right_top() + Vec2::new(2.0, 4.0),
        Vec2::new(default_width, height - 8.0),
    );
    if let Some(ShaderRowEdit {
        kind: ShaderRowEditKind::Flags(options),
        ..
    }) = row.edit.as_ref()
    {
        let mask = row
            .default_cell
            .as_ref()
            .and_then(|cell| cell.text.parse::<u64>().ok())
            .unwrap_or(0);
        let mut defaults = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("shader_default_flags", &row.label))
                .max_rect(default_rect),
        );
        defaults.multiply_opacity(0.5);
        defaults.visuals_mut().disabled_alpha = 1.0;
        shader_input_box(&mut defaults, default_rect);
        shader_cell_scope(&mut defaults, default_rect.shrink(4.0), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (bit, option) in options.iter().enumerate() {
                let mut checked = mask & (1 << bit) != 0;
                ui.add_enabled(false, egui::Checkbox::new(&mut checked, option));
            }
        });
    } else {
        draw_shader_grid_cell_with_icon(
            ui,
            default_rect,
            row.default_cell.as_ref(),
            &format!("default:{}", row.label),
            color_popup,
            shader_reference_group(row),
            edit.game,
        );
    }

    let value_left = default_rect.right() + 6.0;
    let controls_left = rect.right() - right_controls_width;
    let value_right = (controls_left
        - if right_controls_width > SHADER_ROW_RIGHT_PADDING {
            4.0
        } else {
            0.0
        })
    .max(value_left + 40.0);
    let full_value_rect = egui::Rect::from_min_max(
        egui::pos2(value_left, default_rect.top()),
        egui::pos2(value_right, default_rect.bottom()),
    );
    let mut value_rect = full_value_rect;
    let range_input_rect = if has_range_input {
        let field_width = (full_value_rect.width() - H2_RANGE_CONTROL_WIDTH - 12.0) / 2.0;
        value_rect.max.x = value_rect.left() + field_width;
        Some(egui::Rect::from_min_max(
            egui::pos2(value_rect.right() + 8.0, full_value_rect.top()),
            full_value_rect.right_bottom(),
        ))
    } else {
        None
    };
    // Editable value cell when the row carries an edit path and the tag is
    // writable; otherwise the read-only painted cell.
    if ranged && let Some(control) = range_control.as_ref() {
        draw_shader_range_values(ui, value_rect, row, control, edit);
    } else if show_override && let Some(row_edit) = row.edit.as_ref() {
        shader_cell_scope(ui, value_rect, |ui| {
            let response = ui
                .add(egui::Button::new("Override Default").min_size(Vec2::new(0.0, BUTTON_HEIGHT)));
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
        draw_shader_grid_cell_with_icon(
            ui,
            value_rect,
            Some(&row.value_cell),
            &format!("value:{}", row.label),
            color_popup,
            shader_reference_group(row),
            edit.game,
        );
    }
    let next_function_x = controls_left.max(full_value_rect.right() + 4.0);
    if let Some(range_input_rect) = range_input_rect {
        let range_rect = egui::Rect::from_min_size(
            range_input_rect.min,
            Vec2::new(H2_RANGE_CONTROL_WIDTH, BUTTON_HEIGHT),
        );
        if let Some(control) = range_control.as_ref() {
            draw_shader_range_checkbox(ui, range_rect, control, edit);
        } else {
            ui.painter().text(
                range_rect.left_center(),
                Align2::LEFT_CENTER,
                "range:",
                egui::TextStyle::Body.resolve(ui.style()),
                material_text(),
            );
        }
        let input_rect = egui::Rect::from_min_max(
            range_rect.right_top() + Vec2::new(4.0, 0.0),
            egui::pos2(range_input_rect.right(), range_rect.bottom()),
        );
        draw_shader_range_name(ui, input_rect, row, edit);
    }

    if !show_override && let Some(function) = row.function.as_ref() {
        // Function viewer; the shared Clear control is placed last below.
        let button_rect = egui::Rect::from_min_size(
            egui::pos2(next_function_x, value_rect.top()),
            ICON_BUTTON_SIZE,
        );
        let click_response = shader_action_button(
            ui,
            button_rect,
            format!("shader_function:{}", row.label),
            ButtonIcon::Function,
            "",
            true,
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
    } else if !show_override && let Some(func_view) = row.constant_function_view.as_ref() {
        // Constant-function row: open the graph without intercepting its input.
        let f_rect = egui::Rect::from_min_size(
            egui::pos2(next_function_x, value_rect.top()),
            ICON_BUTTON_SIZE,
        );
        if shader_action_button(
            ui,
            f_rect,
            format!("shader_cfn_open:{}", row.label),
            ButtonIcon::Function,
            "",
            true,
        )
        .on_hover_text("Open function graph editor")
        .clicked()
        {
            *function_popup = Some(FunctionPopup::new(
                tag_key.to_owned(),
                row.label.clone(),
                func_view.clone(),
                editable && func_view.edit.is_some(),
            ));
        }
    } else if let (true, Some(action)) = (editable && !show_override, create_function.as_ref()) {
        // No animated parameter yet — show an "f()+" button to create one.
        let button_rect = egui::Rect::from_min_size(
            egui::pos2(next_function_x, value_rect.top()),
            Vec2::new(24.0, BUTTON_HEIGHT),
        );
        let add_response = shader_action_button(
            ui,
            button_rect,
            format!("shader_create_anim:{}", row.label),
            ButtonIcon::Function,
            "",
            true,
        )
        .on_hover_text("Add function initialized to the default value");
        if add_response.clicked() {
            push_shader_context_action(edit, action);
        }
    } else {
        // context_menu takes &self so call it first; on_hover_text takes self.
        let reset = (editable && row.is_overridden)
            .then(|| unused_delete.cloned().or_else(|| reset_op_for_row(row)))
            .flatten();
        let menu_items = row
            .context_menu
            .as_ref()
            .filter(|_| editable)
            .map(|menu| menu.items.as_slice())
            .filter(|items| !items.is_empty());
        if reset.is_some() || menu_items.is_some() {
            context_menu(&response, |ui| {
                if let Some(reset) = reset.clone() {
                    if ui
                        .button(if unused_delete.is_some() {
                            "Remove unused parameter"
                        } else {
                            "Reset to default"
                        })
                        .clicked()
                    {
                        edit.block_ops.push(reset);
                        close_menu(ui);
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
                            close_menu(ui);
                        }
                    }
                }
            });
        }
        if let Some(parameter_type) = row.parameter_type.as_deref() {
            response.on_hover_text(parameter_type);
        }
    }
    if has_clear {
        let clear_rect = egui::Rect::from_min_size(
            egui::pos2(
                rect.right() - SHADER_ROW_RIGHT_PADDING - 24.0,
                value_rect.top(),
            ),
            Vec2::splat(24.0),
        );
        if shader_action_button(
            ui,
            clear_rect,
            ("shader_clear_default", &row.label),
            ButtonIcon::Clear,
            "",
            true,
        )
        .on_hover_text(if unused_delete.is_some() {
            "Remove this unused parameter and its saved functions"
        } else {
            "Restore the default value"
        })
        .clicked()
        {
            if let Some(reset) = function_reset {
                match reset {
                    ShaderFunctionReset::Field(value) => edit.pending.push(value),
                    ShaderFunctionReset::Halo2(op) => edit.h2_shader_param_ops.push(op),
                    ShaderFunctionReset::Create(action) => {
                        push_shader_context_action(edit, &action)
                    }
                }
            } else if let Some(reset) = reset {
                edit.block_ops.push(reset);
            }
        }
    }
    // Paint after fills and controls so the row border stays visible.
    shader_row_separator(ui, rect);
}

fn is_h2_function_view(function: &FunctionView) -> bool {
    function
        .edit
        .as_ref()
        .is_some_and(|edit| matches!(edit.data, FunctionDataStorage::Halo2ByteBlock(_)))
}

fn shader_grid_row_height(ui: &mut Ui, row: &ShaderGridRow, value_width: f32) -> f32 {
    if let Some(ShaderRowEdit {
        kind: ShaderRowEditKind::Flags(options),
        ..
    }) = row.edit.as_ref()
    {
        // Measure the same checkbox layout used by the value cell. Its height
        // depends on control styling, flag count, and wrapping at this width.
        let mut measure = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("shader_flags_measure", &row.label))
                .max_rect(egui::Rect::from_min_size(
                    ui.cursor().min,
                    Vec2::new(value_width, 0.0),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min))
                .invisible(),
        );
        measure.spacing_mut().item_spacing.y = 0.0;
        for option in options {
            measure.checkbox(&mut false, option);
        }
        (measure.min_rect().height() + 16.0).max(BUTTON_HEIGHT + 8.0)
    } else {
        BUTTON_HEIGHT + 8.0
    }
}

const H2_RANGE_CONTROL_WIDTH: f32 = 76.0;

fn shader_right_controls_width(row: &ShaderGridRow, has_h2_range: bool, has_clear: bool) -> f32 {
    let mut width = SHADER_ROW_RIGHT_PADDING;
    if has_h2_range {
        width += H2_RANGE_CONTROL_WIDTH + 4.0;
    }
    let function_width = if row.function.is_some()
        || row.constant_function_view.is_some()
        || row.create_anim_op.is_some()
    {
        BUTTON_HEIGHT
    } else {
        0.0
    };
    width += function_width;
    if has_clear {
        width += 24.0 + if function_width > 0.0 { 4.0 } else { 0.0 };
    }
    width
}

#[derive(Clone)]
enum H2RangeControl {
    Existing { block_path: String, data: Vec<u8> },
    Create { op: H2ShaderParamOp, data: Vec<u8> },
}

/// The range control a row offers, for a scalar function only: a color
/// function's bytes 4-19 hold its colors, not a range, so a range written there
/// overwrote color slot 1 (the engine's `set_clamp_range` refuses the same).
#[cfg(test)]
fn h2_range_control_for_row(row: &ShaderGridRow) -> Option<H2RangeControl> {
    h2_range_control_candidate(row).filter(|control| match control {
        H2RangeControl::Existing { data, .. } => !h2_is_color_function(data),
        H2RangeControl::Create { data, .. } => !h2_is_color_function(data),
    })
}

fn h2_range_control_candidate(row: &ShaderGridRow) -> Option<H2RangeControl> {
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

#[cfg(test)]
pub(super) fn h2_function_range_enabled(data: &[u8]) -> bool {
    data.get(1)
        .copied()
        .is_some_and(|flags| flags & h2_flags::RANGE != 0)
}

#[cfg(test)]
pub(super) fn h2_function_range_value(data: &[u8]) -> Option<f32> {
    Some(f32::from_le_bytes(data.get(8..12)?.try_into().ok()?))
}

#[cfg(test)]
use blam_tags::tag_function::h2::flags as h2_flags;

/// Whether H2 function bytes describe a color function: the flags' high nibble
/// is its color count, zero for a scalar.
#[cfg(test)]
fn h2_is_color_function(data: &[u8]) -> bool {
    data.get(1)
        .is_some_and(|flags| flags >> h2_flags::COLOR_GRAPH_TYPE_SHIFT != 0)
}

/// `data` with its range turned on or off and its range value set. A color
/// function is returned unchanged: it has no range.
#[cfg(test)]
pub(super) fn h2_function_data_with_range(
    data: &[u8],
    enabled: bool,
    value: Option<f32>,
) -> Vec<u8> {
    if h2_is_color_function(data) {
        return data.to_vec();
    }
    let mut next = data.to_vec();
    if next.len() < 12 {
        next.resize(12, 0);
    }
    if enabled {
        next[1] |= h2_flags::RANGE;
    } else {
        next[1] &= !h2_flags::RANGE;
    }
    if let Some(value) = value {
        next[8..12].copy_from_slice(&value.to_le_bytes());
    }
    next
}

fn h2_range_data_ops(control: &H2RangeControl, data: Vec<u8>) -> DeferredOps {
    let mut ops = DeferredOps::default();
    match control {
        H2RangeControl::Existing { block_path, .. } => {
            ops.h2_shader_param_ops
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
                ops.h2_shader_param_ops.push(op);
            }
        }
    }
    ops
}

#[derive(Clone)]
enum ShaderRangeOwner {
    Halo2(H2RangeControl),
    Field(String),
    Create(ShaderFunctionCreateTarget),
}

#[derive(Clone)]
struct ShaderRangeControl {
    owner: ShaderRangeOwner,
    function: TagFunction,
}

fn shader_range_control(row: &ShaderGridRow) -> Option<ShaderRangeControl> {
    if let Some(control) = h2_range_control_candidate(row) {
        let (H2RangeControl::Existing { data, .. } | H2RangeControl::Create { data, .. }) =
            &control;
        let function = h2_tag_function(data)?;
        return Some(ShaderRangeControl {
            owner: ShaderRangeOwner::Halo2(control),
            function,
        });
    }
    if let Some(view) = row
        .function
        .as_ref()
        .or(row.constant_function_view.as_ref())
    {
        return Some(ShaderRangeControl {
            owner: ShaderRangeOwner::Field(view.edit.as_ref()?.data.data_field_path()?.to_owned()),
            function: view.function.clone(),
        });
    }
    if let Some(ShaderContextAction::AnimatedParameter(op)) = row.create_anim_op.as_ref() {
        let function = TagFunction::parse(&decode_hex(&op.initial_function_hex).ok()?).ok()?;
        return Some(ShaderRangeControl {
            owner: ShaderRangeOwner::Create(ShaderFunctionCreateTarget::ExistingParameter {
                animated_block_path: op.animated_block_path.clone(),
                output_type_index: op.output_type_index,
            }),
            function,
        });
    }
    let row_edit = row.edit.as_ref()?;
    let target = match &row_edit.kind {
        ShaderRowEditKind::CreateFunctionScalar { target }
        | ShaderRowEditKind::CreateFunctionColor { target } => target.clone(),
        ShaderRowEditKind::CreateScalarParam {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
        } => ShaderFunctionCreateTarget::NewParameter {
            parameters_block_path: parameters_block_path.clone(),
            parameter_name: parameter_name.clone(),
            parameter_type_index: *parameter_type_index,
            output_type_index: RenderMethodAnimatedParameterType::Value as i32,
        },
        ShaderRowEditKind::Scalar if row_edit.path.ends_with("/real") => {
            ShaderFunctionCreateTarget::ExistingParameter {
                animated_block_path: format!(
                    "{}/animated parameters",
                    row_edit.path.strip_suffix("/real")?
                ),
                output_type_index: RenderMethodAnimatedParameterType::Value as i32,
            }
        }
        _ => return None,
    };
    let hex = if matches!(row_edit.kind, ShaderRowEditKind::CreateFunctionColor { .. }) {
        let values: Vec<f32> = row_edit
            .current
            .split(',')
            .map(str::trim)
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        let [r, g, b, a] = values.as_slice() else {
            return None;
        };
        constant_color_function_hex(*r, *g, *b, *a)
    } else {
        constant_function_hex(row_edit.current.parse::<f32>().ok()?)
    };
    let function = TagFunction::parse(&decode_hex(&hex).ok()?).ok()?;
    Some(ShaderRangeControl {
        owner: ShaderRangeOwner::Create(target),
        function,
    })
}

fn push_shader_range_edit(
    edit: &mut FieldEditContext<'_>,
    control: &ShaderRangeControl,
    editor: TagFunctionEditor,
) {
    edit.push_ops(shader_range_ops(control, editor));
}

fn shader_range_ops(control: &ShaderRangeControl, editor: TagFunctionEditor) -> DeferredOps {
    let data = editor.to_bytes();
    match &control.owner {
        ShaderRangeOwner::Halo2(owner) => h2_range_data_ops(owner, data),
        ShaderRangeOwner::Field(path) => field_edit_ops(path, &encode_hex(&data)),
        ShaderRangeOwner::Create(target) => {
            shader_context_action_ops(&shader_function_action(target, encode_hex(&data)))
        }
    }
}

fn shader_range_values_ops(
    control: &ShaderRangeControl,
    start: &str,
    end: &str,
) -> Result<DeferredOps, String> {
    let mut editor = TagFunctionEditor::from_function(control.function.clone());
    editor
        .set_clamp_range(parse_shader_number(start)?, parse_shader_number(end)?)
        .map_err(|error| error.to_string())?;
    Ok(shader_range_ops(control, editor))
}

fn draw_shader_range_checkbox(
    ui: &mut Ui,
    rect: egui::Rect,
    control: &ShaderRangeControl,
    edit: &mut FieldEditContext<'_>,
) {
    let mut editor = TagFunctionEditor::from_function(control.function.clone());
    let mut ranged = editor.is_ranged();
    let response = shader_cell_scope(ui, rect, |ui| {
        ui.add_enabled(edit.editable, egui::Checkbox::new(&mut ranged, "range:"))
    })
    .inner;
    if response.changed() && editor.set_ranged(ranged).is_ok() {
        push_shader_range_edit(edit, control, editor);
    }
}

fn draw_shader_range_name(
    ui: &mut Ui,
    rect: egui::Rect,
    row: &ShaderGridRow,
    edit: &mut FieldEditContext<'_>,
) {
    let view = row
        .function
        .as_ref()
        .or(row.constant_function_view.as_ref());
    let path = view
        .and_then(|view| view.edit.as_ref())
        .map(|paths| paths.range_name.as_str())
        .filter(|path| !path.is_empty());
    let current = view.map(|view| view.range_name.as_str()).unwrap_or("");
    let key = format!("{}|shader_range_name:{}", edit.tag_key, row.label);
    let id = edit.widget_id(("shader_range_name", &row.label));
    let draft = edit.buffers.draft_mut(&key, current);
    shader_cell_scope(ui, rect, |ui| {
        ui.visuals_mut().extreme_bg_color = material_input();
        let response = ui.add_enabled(
            edit.editable && path.is_some(),
            egui::TextEdit::singleline(&mut draft.text)
                .id(id)
                .desired_width(rect.width())
                .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
                .font(egui::TextStyle::Body)
                .vertical_align(egui::Align::Center)
                .hint_text("Range input"),
        );
        if path.is_none() {
            response
                .clone()
                .on_hover_text("Enable range or add a function to edit its range input");
        }
        draft.note_response(&response);
        if draft.should_commit(ui, &response)
            && let Some(path) = path
        {
            edit.pending.push(PendingFieldEdit {
                path: path.to_owned(),
                input: draft.text.trim().to_owned(),
            });
        }
        if let Some(path) = path {
            draft.keep_commit(|| single_field_commit(edit.tag_key, &key, path));
        }
    });
}

fn draw_shader_range_values(
    ui: &mut Ui,
    rect: egui::Rect,
    row: &ShaderGridRow,
    control: &ShaderRangeControl,
    edit: &mut FieldEditContext<'_>,
) {
    let mut editor = TagFunctionEditor::from_function(control.function.clone());
    let Some((mut start, mut end)) = editor.clamp_range() else {
        return;
    };
    let mut changed = false;
    let keys = [
        format!("{}|range:{}:0", edit.tag_key, row.label),
        format!("{}|range:{}:1", edit.tag_key, row.label),
    ];
    let commit_control = control.clone();
    let range_commit = DraftCommit::new(edit.tag_key, keys.to_vec(), move |texts| {
        shader_range_values_ops(&commit_control, texts[0], texts[1])
    });
    for (index, (label, value)) in [("start:", &mut start), ("end:", &mut end)]
        .into_iter()
        .enumerate()
    {
        let line = egui::Rect::from_min_size(
            rect.min + Vec2::new(0.0, index as f32 * 32.0),
            Vec2::new(rect.width(), BUTTON_HEIGHT),
        );
        ui.painter().text(
            line.left_center(),
            Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Body.resolve(ui.style()),
            material_text(),
        );
        let input =
            egui::Rect::from_min_max(line.left_top() + Vec2::new(46.0, 0.0), line.right_bottom());
        let key = format!("{}|range:{}:{index}", edit.tag_key, row.label);
        let id = edit.widget_id(("shader_range_value", &key));
        let current = format_shader_float(*value);
        let draft = edit.buffers.draft_mut(&key, &current);
        shader_cell_scope(ui, input, |ui| {
            ui.visuals_mut().extreme_bg_color = material_input();
            let response = ui.add_enabled(
                edit.editable,
                egui::TextEdit::singleline(&mut draft.text)
                    .id(id)
                    .desired_width(input.width())
                    .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
                    .font(egui::TextStyle::Body)
                    .vertical_align(egui::Align::Center),
            );
            select_all_on_double_click(ui, &response, &draft.text);
            draft.note_response(&response);
            if draft.should_commit(ui, &response) {
                match parse_shader_number(&draft.text) {
                    Ok(next) => {
                        *value = next;
                        changed = true;
                    }
                    Err(error) => {
                        if let Some(status) = edit.status.as_deref_mut() {
                            *status = error;
                        }
                    }
                }
            }
            draft.keep_commit(|| range_commit.clone());
        });
    }
    if changed && editor.set_clamp_range(start, end).is_ok() {
        push_shader_range_edit(edit, control, editor);
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
                .font(egui::TextStyle::Body)
                .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
                .vertical_align(egui::Align::Center),
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
    // Keyed by the source too: the decode resolves `open_ref` against this
    // kit's tags root, and a relative tag path means different bitmaps in
    // different games. Without the root, whichever workspace decoded first won
    // and the other showed its thumbnail.
    let cache_id = egui::Id::new(("shader_bitmap_thumb", edit.tags_root, group_tag, open_ref));
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
        crate::core::source::read_tag_at_path(&path, edit.game, edit.definitions_root, group_tag)
            .ok()?;
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
pub(in crate::app) fn downscale_rgba(
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
            shader_cell_scope(ui, rect, |ui| {
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
            shader_input_box(ui, rect);
            shader_cell_scope(ui, rect.shrink(4.0), |ui| {
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

        // Constant animated-parameter scalar input.
        // The f() button to open the graph editor is rendered in draw_shader_grid_row
        // via constant_function_view, not here.
        ShaderRowEditKind::FunctionScalar { .. } => {
            let current = row_edit.current.clone();
            let text_rect = rect;
            let id = edit.widget_id(("shader_fn_scalar", &buffer_key));
            let draft = edit.buffers.draft_mut(&buffer_key, &current);
            let mut commit = None;
            shader_cell_scope(ui, text_rect, |ui| {
                ui.visuals_mut().extreme_bg_color = material_input();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut draft.text)
                        .id(id)
                        .desired_width(text_rect.width())
                        .text_color(material_text())
                        .font(egui::TextStyle::Body)
                        .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
                        .vertical_align(egui::Align::Center),
                );
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                select_all_on_double_click(ui, &resp, &draft.text);
                draft.note_response(&resp);
                if draft.should_commit(ui, &resp) {
                    commit = Some(function_scalar_ops(&row_edit.path, &draft.text));
                }
                draft.keep_commit(|| {
                    let path = row_edit.path.clone();
                    shader_draft_commit(edit.tag_key, &buffer_key, move |text| {
                        function_scalar_ops(&path, text)
                    })
                });
            });
            if let Some(commit) = commit {
                push_shader_commit(edit, commit);
            }
        }

        // Tag references: text box + Open + "..." browse, and a drop target.
        ShaderRowEditKind::BitmapRef { group_tag, create } => {
            let cell = ShaderReferenceCell {
                id: "shader_bitmap",
                group_tag: *group_tag,
                extension: "bitmap",
                browse_extensions: &["bitmap"],
                thumbnail: true,
                // The cell's canonical form carries the `.bitmap` suffix (see
                // the row builders); the payload's rel_path does not, and the
                // edit applier refuses an extension-less tag reference.
                drop_value: |payload| format!("{}.bitmap", payload.rel_path),
                normalize: normalize_bitmap_browse_path,
            };
            let commit_ops = |_: &FieldEditContext<'_>| -> ReferenceCommitOps {
                let (path, create) = (row_edit.path.clone(), create.clone());
                Box::new(move |text| {
                    Ok(shader_value_edit_ops(
                        &path,
                        create.as_ref(),
                        text.trim().to_owned(),
                    ))
                })
            };
            if let Some(input) = draw_shader_reference_cell(
                ui,
                edit,
                rect,
                &buffer_key,
                row_edit,
                &cell,
                &commit_ops,
            ) {
                push_shader_value_edit(edit, row_edit, create.as_ref(), input);
            }
        }

        ShaderRowEditKind::ShaderTemplateRef => {
            let cell = ShaderReferenceCell {
                id: "shader_template",
                group_tag: u32::from_be_bytes(*b"stem"),
                extension: "shader_template",
                browse_extensions: &["shader_template", "stem"],
                thumbnail: false,
                drop_value: |payload| payload.rel_path.clone(),
                normalize: normalize_shader_template_browse_path,
            };
            let commit_ops = |edit: &FieldEditContext<'_>| -> ReferenceCommitOps {
                let path = row_edit.path.clone();
                let tags_root = edit.tags_root.map(std::path::Path::to_path_buf);
                let definitions_root = edit.definitions_root.map(std::path::Path::to_path_buf);
                let game = edit.game;
                Box::new(move |text| {
                    Ok(h2_template_reference_ops(
                        &path,
                        text,
                        tags_root.as_deref(),
                        game,
                        definitions_root.as_deref(),
                    ))
                })
            };
            if let Some(input) = draw_shader_reference_cell(
                ui,
                edit,
                rect,
                &buffer_key,
                row_edit,
                &cell,
                &commit_ops,
            ) {
                push_h2_template_reference_edit(edit, row_edit, input);
            }
        }

        // A shader's structural references: `definition` and `shader template`,
        // committed straight through the generic field-edit path. Nothing is
        // reconciled afterwards, which matches Foundation — it carries no
        // shader-aware code to reconcile with, and its expert mode is a
        // blanket field-panel switch.
        ShaderRowEditKind::StructuralRef {
            group_tag,
            extension,
        } => {
            let cell = ShaderReferenceCell {
                id: "structural_ref",
                group_tag: *group_tag,
                extension,
                browse_extensions: &[extension],
                thumbnail: false,
                // `input` is the `GROUP:path` form, which the edit applier
                // parses for any group; the bare rel_path would be refused as
                // an extension-less tag reference.
                drop_value: |payload| payload.input.clone(),
                normalize: normalize_bitmap_browse_path,
            };
            let commit_ops = |_: &FieldEditContext<'_>| -> ReferenceCommitOps {
                let path = row_edit.path.clone();
                Box::new(move |text| Ok(field_edit_ops(&path, text)))
            };
            if let Some(input) = draw_shader_reference_cell(
                ui,
                edit,
                rect,
                &buffer_key,
                row_edit,
                &cell,
                &commit_ops,
            ) {
                edit.push_ops(field_edit_ops(&row_edit.path, &input));
            }
        }

        ShaderRowEditKind::Bool { create } => {
            let current_raw = row_edit.current.trim().parse::<i32>().unwrap_or(0);
            let mut checked = current_raw != 0;
            let response = shader_cell_scope(ui, rect, |ui| {
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

        // Constant color animated parameter: clickable swatch and color popup.
        ShaderRowEditKind::FunctionColor { .. } => {
            let swatch_rect = rect;
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
            let inner = shader_color_swatch_rect(swatch_rect);
            ui.painter().text(
                swatch_rect.left_center() + Vec2::new(inner.width() + 6.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                egui::TextStyle::Body.resolve(ui.style()),
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
            let inner = shader_color_swatch_rect(rect);
            ui.painter().text(
                rect.left_center() + Vec2::new(inner.width() + 6.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                egui::TextStyle::Body.resolve(ui.style()),
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
            let inner = shader_color_swatch_rect(rect);
            ui.painter().text(
                rect.left_center() + Vec2::new(inner.width() + 6.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                egui::TextStyle::Body.resolve(ui.style()),
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
            let draft = edit.buffers.draft_mut(&create_buf_key, &current);
            let mut commit = None;
            shader_cell_scope(ui, rect, |ui| {
                ui.visuals_mut().extreme_bg_color = material_input();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut draft.text)
                        .id(id)
                        .desired_width(rect.width())
                        .text_color(material_text())
                        .font(egui::TextStyle::Body)
                        .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
                        .vertical_align(egui::Align::Center),
                );
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                select_all_on_double_click(ui, &resp, &draft.text);
                draft.note_response(&resp);
                if draft.should_commit(ui, &resp) {
                    commit = Some(create_function_scalar_ops(target, &draft.text));
                }
                draft.keep_commit(|| {
                    let target = target.clone();
                    shader_draft_commit(edit.tag_key, &create_buf_key, move |text| {
                        create_function_scalar_ops(&target, text)
                    })
                });
            });
            if let Some(commit) = commit {
                push_shader_commit(edit, commit);
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
            let inner = shader_color_swatch_rect(rect);
            ui.painter().text(
                rect.left_center() + Vec2::new(inner.width() + 6.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                egui::TextStyle::Body.resolve(ui.style()),
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
            let inner = shader_color_swatch_rect(rect);
            ui.painter().text(
                rect.left_center() + Vec2::new(inner.width() + 6.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                egui::TextStyle::Body.resolve(ui.style()),
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
            let draft = edit.buffers.draft_mut(&buffer_key, &current);
            let mut commit = None;
            shader_cell_scope(ui, rect, |ui| {
                ui.visuals_mut().extreme_bg_color = material_input();
                let resp = draw_h2_value_prefixed_text_edit(ui, id, &mut draft.text, rect.width());
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                select_all_on_double_click(ui, &resp, &draft.text);
                draft.note_response(&resp);
                if draft.should_commit(ui, &resp) {
                    commit = Some(h2_function_scalar_ops(
                        block_path,
                        legacy_data.as_deref(),
                        &draft.text,
                    ));
                }
                draft.keep_commit(|| {
                    let (block_path, legacy_data) = (block_path.clone(), legacy_data.clone());
                    shader_draft_commit(edit.tag_key, &buffer_key, move |text| {
                        h2_function_scalar_ops(&block_path, legacy_data.as_deref(), text)
                    })
                });
            });
            if let Some(commit) = commit {
                push_shader_commit(edit, commit);
            }
        }

        ShaderRowEditKind::H2CreateFunctionScalar { create_op } => {
            let current = row_edit.current.clone();
            let create_buf_key = format!("{}|{}", edit.tag_key, row_edit.path);
            let id = edit.widget_id(("h2_shader_create_fn_scalar", label));
            let draft = edit.buffers.draft_mut(&create_buf_key, &current);
            let mut commit = None;
            shader_cell_scope(ui, rect, |ui| {
                ui.visuals_mut().extreme_bg_color = material_input();
                let resp = draw_h2_value_prefixed_text_edit(ui, id, &mut draft.text, rect.width());
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                select_all_on_double_click(ui, &resp, &draft.text);
                draft.note_response(&resp);
                if draft.should_commit(ui, &resp) {
                    commit = Some(h2_create_function_scalar_ops(create_op, &draft.text));
                }
                draft.keep_commit(|| {
                    let create_op = create_op.clone();
                    shader_draft_commit(edit.tag_key, &create_buf_key, move |text| {
                        h2_create_function_scalar_ops(&create_op, text)
                    })
                });
            });
            if let Some(commit) = commit {
                push_shader_commit(edit, commit);
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
            let draft = edit.buffers.draft_mut(&create_buf_key, &current);
            let mut commit = None;
            shader_cell_scope(ui, rect, |ui| {
                ui.visuals_mut().extreme_bg_color = material_input();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut draft.text)
                        .id(id)
                        .desired_width(rect.width())
                        .text_color(material_text())
                        .font(egui::TextStyle::Body)
                        .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
                        .vertical_align(egui::Align::Center),
                );
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                select_all_on_double_click(ui, &resp, &draft.text);
                draft.note_response(&resp);
                if draft.should_commit(ui, &resp) {
                    commit = Some(create_scalar_param_ops(
                        parameters_block_path,
                        parameter_name,
                        *parameter_type_index,
                        &draft.text,
                    ));
                }
                draft.keep_commit(|| {
                    let (block, name, type_index) = (
                        parameters_block_path.clone(),
                        parameter_name.clone(),
                        *parameter_type_index,
                    );
                    shader_draft_commit(edit.tag_key, &create_buf_key, move |text| {
                        create_scalar_param_ops(&block, &name, type_index, text)
                    })
                });
            });
            if let Some(commit) = commit {
                push_shader_commit(edit, commit);
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
            let draft = edit.buffers.draft_mut(&create_buf_key, &current);
            let mut commit = None;
            shader_cell_scope(ui, rect, |ui| {
                ui.visuals_mut().extreme_bg_color = material_input();
                let resp = draw_h2_value_prefixed_text_edit(ui, id, &mut draft.text, rect.width());
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                select_all_on_double_click(ui, &resp, &draft.text);
                draft.note_response(&resp);
                if draft.should_commit(ui, &resp) {
                    commit = Some(h2_create_template_value_ops(
                        parameters_block_path,
                        parameter_name,
                        *parameter_type_index,
                        field,
                        &draft.text,
                    ));
                }
                draft.keep_commit(|| {
                    let (block, name, type_index, field) = (
                        parameters_block_path.clone(),
                        parameter_name.clone(),
                        *parameter_type_index,
                        field.clone(),
                    );
                    shader_draft_commit(edit.tag_key, &create_buf_key, move |text| {
                        h2_create_template_value_ops(&block, &name, type_index, &field, text)
                    })
                });
            });
            if let Some(commit) = commit {
                push_shader_commit(edit, commit);
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
            let draft = edit.buffers.draft_mut(&buffer_key, &current);
            let mut commit = None;
            shader_cell_scope(ui, rect, |ui| {
                ui.visuals_mut().extreme_bg_color = material_input();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut draft.text)
                        .id(id)
                        .desired_width(rect.width())
                        .text_color(material_text())
                        .font(egui::TextStyle::Body)
                        .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
                        .vertical_align(egui::Align::Center),
                );
                text_edit_cursor_to_start_on_tab_focus(ui, &resp);
                select_all_on_double_click(ui, &resp, &draft.text);
                draft.note_response(&resp);
                if draft.should_commit(ui, &resp) {
                    commit = Some(field_edit_ops(&row_edit.path, &draft.text));
                }
                draft
                    .keep_commit(|| single_field_commit(edit.tag_key, &buffer_key, &row_edit.path));
            });
            if let Some(ops) = commit {
                edit.push_ops(ops);
            }
        }
    }
}

/// How one kind of tag-reference cell differs from the others.
struct ShaderReferenceCell<'a> {
    /// Prefix for the cell's widget ids.
    id: &'static str,
    /// What Open opens, and what a drop must be.
    group_tag: u32,
    /// The referenced tag's extension, as the cell may spell it.
    extension: &'a str,
    browse_extensions: &'a [&'a str],
    /// Show the referenced bitmap beside the path.
    thumbnail: bool,
    /// The cell text a dropped tag becomes.
    drop_value: fn(&DraggedTagRef) -> String,
    /// A browsed file's path, as the cell text.
    normalize: fn(&std::path::Path, Option<&std::path::Path>) -> Result<String, String>,
}

/// How a reference cell's text becomes edits, for committing it without the
/// cell; the same edits the cell's caller makes from the text it returns.
type ReferenceCommitOps = Box<dyn Fn(&str) -> Result<DeferredOps, String>>;

/// A tag-reference cell in the shader grid: an optional thumbnail, the path
/// in a text box, Open, "..." browse, and a drop target for a tag dragged
/// from the browser. Returns the text to commit; the caller decides how it
/// is applied.
///
/// Bitmap, shader-template and structural references were three copies of
/// this that had drifted: only the bitmap cell marked a missing target, hinted
/// an empty one, and ignored drops on a read-only tag, and only it and browse
/// updated the text box after a drop.
fn draw_shader_reference_cell(
    ui: &mut Ui,
    edit: &mut FieldEditContext<'_>,
    rect: egui::Rect,
    buffer_key: &str,
    row_edit: &ShaderRowEdit,
    cell: &ShaderReferenceCell<'_>,
    commit_ops: &dyn Fn(&FieldEditContext<'_>) -> ReferenceCommitOps,
) -> Option<String> {
    let current = row_edit.current.clone();
    let extension = cell.extension;
    // Browse fits its label and shared icon/text padding; every gap is 4px.
    let browse_width = ui
        .painter()
        .layout_no_wrap(
            "Browse".to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            text_dark(),
        )
        .size()
        .x
        .ceil()
        + BUTTON_ICON_SIZE
        + BUTTON_ICON_TEXT_GAP
        + BUTTON_TEXT_PADDING_X * 2.0;
    let open_rect = egui::Rect::from_min_size(
        rect.right_top() - Vec2::new(BUTTON_HEIGHT, 0.0),
        ICON_BUTTON_SIZE,
    );
    let browse_rect = egui::Rect::from_min_size(
        open_rect.left_top() - Vec2::new(browse_width + 4.0, 0.0),
        Vec2::new(browse_width, BUTTON_HEIGHT),
    );
    // The grid stores the path with its extension and forward slashes; strip
    // both so it resolves like a normal tag reference.
    let cleaned = sanitize_ref_path(&current);
    let open_ref = cleaned
        .strip_suffix(&format!(".{extension}"))
        .unwrap_or(&cleaned)
        .replace('/', "\\");
    let open_enabled = !open_ref.is_empty() && open_ref != "NONE";
    let thumb = (cell.thumbnail && open_enabled)
        .then(|| shader_bitmap_thumbnail(ui, edit, cell.group_tag, &open_ref))
        .flatten();
    let text_rect = egui::Rect::from_min_size(
        rect.left_top(),
        Vec2::new(
            (browse_rect.left() - rect.left() - 4.0).max(40.0),
            rect.height(),
        ),
    );
    let icon_inset = if thumb.is_some() { 28 } else { 23 };

    if shader_action_button(
        ui,
        open_rect,
        (cell.id, "open", buffer_key),
        ButtonIcon::Open,
        "",
        open_enabled,
    )
    .on_hover_text(format!(
        "Open the referenced {extension} tag (Alt: floating window)"
    ))
    .clicked()
    {
        *edit.open_request = Some(OpenTagRequest {
            group_tag: cell.group_tag,
            rel_path: open_ref.clone(),
            float: ui.input(|i| i.modifiers.alt),
        });
    }

    let id = edit.widget_id((cell.id, "text", buffer_key));
    let mut draft = edit.buffers.take(buffer_key, &current);
    // Flag a referenced tag that is missing on disk (red text).
    let missing = open_enabled
        && reference_target_missing_cached(
            ui,
            edit.names,
            edit.tags_root,
            cell.group_tag,
            &open_ref,
        );
    let text_color = if missing {
        REFERENCE_MISSING_COLOR
    } else {
        material_text()
    };
    let mut commit = None;
    shader_cell_scope(ui, text_rect, |ui| {
        ui.visuals_mut().extreme_bg_color = material_input();
        let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _width: f32| {
            let display = if ui.memory(|m| m.has_focus(id)) {
                text.as_str()
            } else {
                shader_reference_name(text.as_str())
            };
            ui.painter().layout_no_wrap(
                display.to_owned(),
                egui::TextStyle::Body.resolve(ui.style()),
                text_color,
            )
        };
        let resp = ui.add(
            egui::TextEdit::singleline(&mut draft.text)
                .id(id)
                .margin(egui::Margin {
                    left: icon_inset,
                    right: 4,
                    top: 2,
                    bottom: 2,
                })
                .desired_width(text_rect.width())
                .layouter(&mut layouter)
                .hint_text(placeholder_text("(no reference)"))
                .text_color(text_color)
                .font(egui::TextStyle::Body)
                .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
                .vertical_align(egui::Align::Center),
        );
        let shown = shader_reference_name(&draft.text);
        if shown != draft.text
            || ui
                .painter()
                .layout_no_wrap(
                    shown.to_owned(),
                    egui::TextStyle::Body.resolve(ui.style()),
                    text_color,
                )
                .size()
                .x
                > text_rect.width() - icon_inset as f32 - 4.0
        {
            resp.clone().on_hover_text(draft.text.clone());
        }
        if missing {
            resp.clone()
                .on_hover_text(format!("Referenced {extension} not found on disk"));
        }
        text_edit_cursor_to_start_on_tab_focus(ui, &resp);
        draft.note_response(&resp);
        if draft.should_commit(ui, &resp) {
            commit = Some(draft.text.trim().to_owned());
        }
    });
    draft.keep_commit(|| {
        let ops = commit_ops(edit);
        DraftCommit::new(edit.tag_key, vec![buffer_key.to_owned()], move |texts| {
            ops(texts[0])
        })
    });

    if let Some(texture) = &thumb {
        let thumb_rect = egui::Rect::from_min_size(
            text_rect.left_top() + Vec2::new(1.0, 1.0),
            Vec2::splat(text_rect.height() - 2.0),
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
            bitmap_hover_preview_ui(ui, texture, &open_ref, material_muted_text());
        });
    }
    if thumb.is_none() {
        paint_tag_icon_at(
            ui,
            Some(cell.group_tag),
            edit.game,
            shader_tag_icon_rect(text_rect),
        );
    }

    // Drop a tag of the right group from the browser onto the cell, received
    // by a hover interaction laid over the text box — the structure the
    // browser's drag test drives end to end.
    if edit.editable {
        let drop = ui.interact(
            text_rect,
            ui.make_persistent_id((cell.id, "drop", buffer_key)),
            Sense::hover(),
        );
        let accepts = |payload: &DraggedTagRef| payload.group_tag == cell.group_tag;
        if let Some(payload) = drop.dnd_hover_payload::<DraggedTagRef>() {
            let color = if accepts(&payload) {
                Color32::from_rgb(120, 170, 90)
            } else {
                REFERENCE_MISSING_COLOR
            };
            ui.painter().rect_stroke(
                text_rect,
                2.0,
                Stroke::new(1.5_f32, color),
                egui::StrokeKind::Middle,
            );
        }
        if let Some(payload) = drop.dnd_release_payload::<DraggedTagRef>()
            && accepts(&payload)
        {
            let value = (cell.drop_value)(&payload);
            draft.set_clean(value.clone());
            commit = Some(value);
        }
    }

    if shader_action_button(
        ui,
        browse_rect,
        (cell.id, "browse", buffer_key),
        ButtonIcon::Browse,
        "Browse",
        edit.editable,
    )
    .on_hover_text(format!("Browse for a .{extension} tag file"))
    .clicked()
    {
        let mut dialog = rfd::FileDialog::new()
            .add_filter(format!("{extension} tag"), cell.browse_extensions)
            .set_title(format!("Select {extension} Tag"));
        if let Some(tags_root) = edit.tags_root {
            dialog = dialog.set_directory(tag_reference_start_dir(tags_root, &open_ref));
        }
        if let Some(path) = dialog.pick_file() {
            match (cell.normalize)(&path, edit.tags_root) {
                Ok(rel) => {
                    draft.set_clean(rel.clone());
                    commit = Some(rel);
                }
                Err(error) => {
                    if let Some(status) = edit.status.as_deref_mut() {
                        *status = error;
                    }
                }
            }
        }
    }
    edit.buffers.put(buffer_key.to_owned(), draft);
    commit
}

/// The edit a function-scalar box commits: a constant function of the
/// typed value.
fn function_scalar_ops(path: &str, text: &str) -> Result<DeferredOps, String> {
    Ok(field_edit_ops(
        path,
        &constant_function_hex(parse_shader_number(text)?),
    ))
}

/// The edit a not-yet-created function-scalar box commits.
fn create_function_scalar_ops(
    target: &ShaderFunctionCreateTarget,
    text: &str,
) -> Result<DeferredOps, String> {
    let value = parse_shader_number(text)?;
    Ok(shader_context_action_ops(&shader_function_action(
        target,
        constant_function_hex(value),
    )))
}

/// The edit a Halo 2 function-scalar box commits.
fn h2_function_scalar_ops(
    block_path: &str,
    legacy_data: Option<&[u8]>,
    text: &str,
) -> Result<DeferredOps, String> {
    let value = parse_shader_number(text)?;
    Ok(DeferredOps {
        h2_shader_param_ops: vec![H2ShaderParamOp::EditFunctionData {
            block_path: block_path.to_owned(),
            data: h2_constant_scalar_function_data(value, legacy_data),
        }],
        ..DeferredOps::default()
    })
}

/// The edit a not-yet-created Halo 2 function-scalar box commits.
fn h2_create_function_scalar_ops(
    create_op: &H2ShaderParamOp,
    text: &str,
) -> Result<DeferredOps, String> {
    let value = parse_shader_number(text)?;
    let mut op = create_op.clone();
    if let H2ShaderParamOp::EnsureAnimationProperty {
        initial_function_data,
        ..
    } = &mut op
    {
        *initial_function_data = h2_constant_scalar_function_data(value, None);
    }
    Ok(DeferredOps {
        h2_shader_param_ops: vec![op],
        ..DeferredOps::default()
    })
}

/// The edit a not-yet-created scalar parameter box commits.
fn create_scalar_param_ops(
    parameters_block_path: &str,
    parameter_name: &str,
    parameter_type_index: i32,
    text: &str,
) -> Result<DeferredOps, String> {
    let value = parse_shader_number(text)?;
    Ok(DeferredOps {
        shader_param_ops: vec![ShaderParamOp {
            parameters_block_path: parameters_block_path.to_owned(),
            parameter_name: parameter_name.to_owned(),
            initial_fields: vec![
                shader_parameter_type_initial_field(parameter_type_index),
                ShaderParamInitialField {
                    field: "real".to_owned(),
                    input: value.to_string(),
                },
            ],
            animated_parameters: Vec::new(),
        }],
        ..DeferredOps::default()
    })
}

/// The edit a not-yet-created Halo 2 template value box commits.
fn h2_create_template_value_ops(
    parameters_block_path: &str,
    parameter_name: &str,
    parameter_type_index: i32,
    field: &str,
    text: &str,
) -> Result<DeferredOps, String> {
    Ok(DeferredOps {
        h2_shader_param_ops: vec![H2ShaderParamOp::EditTemplateBackedValue {
            parameters_block_path: parameters_block_path.to_owned(),
            parameter_name: parameter_name.to_owned(),
            parameter_type_index,
            field: field.to_owned(),
            input: h2_template_value_input(field, text.trim()),
        }],
        ..DeferredOps::default()
    })
}

pub(super) fn shader_color_swatch_rect(rect: egui::Rect) -> egui::Rect {
    let size = (rect.height() - 2.0).min(22.0).max(1.0);
    egui::Rect::from_min_size(
        egui::pos2(rect.left() + 1.0, rect.center().y - size / 2.0),
        Vec2::splat(size),
    )
}

pub(in crate::app) fn draw_shader_color_swatch(ui: &mut Ui, rect: egui::Rect, color: Color32) {
    let display_color = Color32::from_rgb(color.r(), color.g(), color.b());
    ui.painter().rect_filled(rect, 0.0, material_input());
    ui.painter().rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0_f32, material_input_edge()),
        egui::StrokeKind::Middle,
    );
    let inner = shader_color_swatch_rect(rect);
    ui.painter().rect_filled(inner, 0.0, display_color);
    ui.painter().rect_stroke(
        inner,
        0.0,
        Stroke::new(1.25_f32, material_color_swatch_edge(display_color)),
        egui::StrokeKind::Middle,
    );
}

pub(in crate::app) fn push_shader_value_edit(
    edit: &mut FieldEditContext<'_>,
    row_edit: &ShaderRowEdit,
    create: Option<&ShaderParamCreateTarget>,
    input: String,
) {
    edit.push_ops(shader_value_edit_ops(&row_edit.path, create, input));
}

/// The edit setting a shader value: the field at `path`, or a new parameter
/// holding it when `create` says it doesn't exist yet.
fn shader_value_edit_ops(
    path: &str,
    create: Option<&ShaderParamCreateTarget>,
    input: String,
) -> DeferredOps {
    let mut ops = DeferredOps::default();
    if let Some(create) = create {
        ops.shader_param_ops.push(ShaderParamOp {
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
        ops.pending.push(PendingFieldEdit {
            path: path.to_owned(),
            input,
        });
    }
    ops
}

fn push_h2_template_reference_edit(
    edit: &mut FieldEditContext<'_>,
    row_edit: &ShaderRowEdit,
    input: String,
) {
    let ops = h2_template_reference_ops(
        &row_edit.path,
        &input,
        edit.tags_root,
        edit.game,
        edit.definitions_root,
    );
    edit.push_ops(ops);
}

/// The edits switching a Halo 2 shader to the template `input` names: the
/// reference, retaining authored parameters for recovery and explicit cleanup.
fn h2_template_reference_ops(
    path: &str,
    input: &str,
    _tags_root: Option<&std::path::Path>,
    _game: Option<GameId>,
    _definitions_root: Option<&std::path::Path>,
) -> DeferredOps {
    let normalized = h2_normalize_shader_template_reference(&sanitize_ref_path(input));
    let pending_input = if normalized.is_empty() || normalized.eq_ignore_ascii_case("none") {
        "none".to_owned()
    } else {
        format!("stem:{}", normalized.replace('/', "\\"))
    };
    DeferredOps {
        pending: vec![PendingFieldEdit {
            path: path.to_owned(),
            input: pending_input,
        }],
        ..DeferredOps::default()
    }
}

pub(super) fn h2_template_parameter_names(root: TagStruct<'_>) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::browser::draw_entry;
    use crate::app::editor::fields::with_test_edit_context;

    fn pending_h2_scalar_row() -> ShaderGridRow {
        let mut row = empty_shader_grid_row();
        row.label = "scale".to_owned();
        row.edit = Some(ShaderRowEdit {
            path: String::new(),
            current: "1".to_owned(),
            kind: ShaderRowEditKind::H2CreateFunctionScalar {
                create_op: H2ShaderParamOp::EnsureAnimationProperty {
                    parameters_block_path: "parameters".to_owned(),
                    parameter_name: "bitmap".to_owned(),
                    parameter_type_index: 0,
                    animation_type_index: 0,
                    initial_function_data: h2_constant_scalar_function_data(1.0, None),
                },
            },
        });
        row
    }

    #[test]
    fn h2_range_controls_support_missing_and_existing_scalar_functions() {
        let mut row = pending_h2_scalar_row();
        assert!(h2_range_control_for_row(&row).is_some());
        row.edit.as_mut().unwrap().kind = ShaderRowEditKind::H2FunctionScalar {
            block_path: "parameters[0]/animation properties[0]/function/data".to_owned(),
            legacy_data: Some(h2_constant_scalar_function_data(1.0, None)),
        };
        assert!(h2_range_control_for_row(&row).is_some());
        row.edit.as_mut().unwrap().kind = ShaderRowEditKind::H2FunctionColor {
            block_path: "parameters[0]/animation properties[0]/function/data".to_owned(),
            legacy_data: Some(h2_constant_color_function_data(1.0, 0.0, 0.0, 1.0, None)),
        };
        assert!(h2_range_control_for_row(&row).is_none());
    }

    #[test]
    fn enabling_a_missing_h2_range_uses_h2_function_encoding() {
        let row = pending_h2_scalar_row();
        let control = shader_range_control(&row).unwrap();
        let mut editor = TagFunctionEditor::from_function(control.function.clone());
        editor.set_ranged(true).unwrap();
        editor.set_clamp_range(1.0, 3.5).unwrap();
        with_test_edit_context(|edit| {
            push_shader_range_edit(edit, &control, editor);
            let H2ShaderParamOp::EnsureAnimationProperty {
                initial_function_data,
                ..
            } = &edit.h2_shader_param_ops[0]
            else {
                panic!("an empty H2 range must create its animation property");
            };
            let function = h2_tag_function(initial_function_data).unwrap();
            let editor = TagFunctionEditor::from_function(function);
            assert!(editor.is_ranged());
            assert_eq!(editor.clamp_range(), Some((1.0, 3.5)));
            assert!(
                edit.pending.is_empty(),
                "H2 must not receive H3 hex field edits"
            );
        });
    }

    #[test]
    fn toggling_h2_color_range_preserves_color_payload() {
        let mut row = pending_h2_scalar_row();
        let original = h2_constant_color_function_data(0.25, 0.5, 0.75, 1.0, None);
        row.edit.as_mut().unwrap().kind = ShaderRowEditKind::H2FunctionColor {
            block_path: "parameters[0]/animation properties[0]/function/data".to_owned(),
            legacy_data: Some(original.clone()),
        };
        let control = shader_range_control(&row).unwrap();
        let mut editor = TagFunctionEditor::from_function(control.function.clone());
        for enabled in [true, false] {
            editor.set_ranged(enabled).unwrap();
            with_test_edit_context(|edit| {
                push_shader_range_edit(edit, &control, editor.clone());
                let H2ShaderParamOp::EditFunctionData { data, .. } = &edit.h2_shader_param_ops[0]
                else {
                    panic!("expected H2 function data edit");
                };
                assert_eq!(&data[4..], &original[4..]);
                assert_eq!(h2_tag_function(data).unwrap().is_ranged(), enabled);
            });
        }
    }

    #[test]
    fn shader_icon_buttons_are_exactly_24px_with_a_4px_gap() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            let first = egui::Rect::from_min_size(ui.cursor().min, ICON_BUTTON_SIZE);
            let second = first.translate(Vec2::new(28.0, 0.0));
            let function =
                shader_action_button(ui, first, "function", ButtonIcon::Function, "", true);
            let clear = shader_action_button(ui, second, "clear", ButtonIcon::Clear, "", true);
            assert_eq!(function.rect.size(), ICON_BUTTON_SIZE);
            assert_eq!(clear.rect.size(), ICON_BUTTON_SIZE);
            assert_eq!(clear.rect.left() - function.rect.right(), 4.0);
        });
    }

    #[test]
    fn unused_shader_parameter_label_is_red() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let mut row = empty_shader_grid_row();
        row.label = "unused_value".to_owned();
        row.is_overridden = true;
        let delete = BlockOp {
            path: "parameters".to_owned(),
            kind: BlockOpKind::Delete(0),
        };
        let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            with_test_edit_context(|edit| {
                draw_unused_shader_grid_row(ui, &row, &mut None, &mut None, edit, &delete);
            });
        });
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "unused_value" && text.fallback_color == material_delete_text())));
    }

    #[test]
    fn shader_row_border_is_painted_after_the_function_label_fill() {
        let ctx = egui::Context::default();
        let row = pending_h2_scalar_row();
        let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            with_test_edit_context(|edit| {
                draw_shader_grid_row(ui, &row, 0, &mut None, &mut None, edit);
            });
        });
        let fill = output
            .shapes
            .iter()
            .position(|shape| {
                matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.fill == material_function_row())
            })
            .unwrap();
        let border = output.shapes.iter().rposition(|shape| matches!(&shape.shape,
            egui::Shape::LineSegment { stroke, .. } if stroke.width == 1.0 && stroke.color == foundation_block_edge())).unwrap();
        assert!(
            border > fill,
            "the border must paint above the colored label"
        );
    }

    #[test]
    fn shader_column_header_resizes_each_column_by_dragging() {
        let ctx = egui::Context::default();
        let point = std::cell::Cell::new(egui::Pos2::ZERO);
        let frame = |events: Vec<egui::Event>, column: usize| {
            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1200.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let label = shader_label_width(ui);
                    let default = shader_default_width(ui);
                    let x = [
                        label + 5.0,
                        label + default + 9.0,
                        shader_grid_width(ui) - 1.0,
                    ][column];
                    point.set(ui.cursor().min + Vec2::new(x, 14.0));
                    draw_shader_columns_header(ui);
                },
            );
        };
        for (column, key, initial) in [
            (0, "shader_grid_label_width", 230.0),
            (1, "shader_grid_default_width", 150.0),
            (2, "shader_grid_value_width", 0.0),
        ] {
            frame(Vec::new(), column);
            let start = point.get();
            let initial = if column == 2 {
                start.x
                    - ctx
                        .data(|d| d.get_temp::<f32>(shader_label_width_id()))
                        .unwrap()
                    - ctx
                        .data(|d| d.get_temp::<f32>(egui::Id::new("shader_grid_default_width")))
                        .unwrap()
                    - 15.0
            } else {
                initial
            };
            frame(
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                column,
            );
            let end = start + Vec2::new(40.0, 0.0);
            frame(vec![egui::Event::PointerMoved(end)], column);
            frame(
                vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                column,
            );
            let width = ctx.data(|d| d.get_temp::<f32>(egui::Id::new(key))).unwrap();
            assert!((width - initial - 40.0).abs() < 0.1, "{key}: {width}");
        }
    }

    #[test]
    fn shader_flags_row_contains_all_checkboxes_at_larger_control_sizes() {
        let ctx = egui::Context::default();
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            ui.spacing_mut().interact_size.y = 32.0;
            for count in [1, 3, 8] {
                let mut row = empty_shader_grid_row();
                row.label = "flags".to_owned();
                row.edit = Some(ShaderRowEdit {
                    path: "flags".to_owned(),
                    current: "0".to_owned(),
                    kind: ShaderRowEditKind::Flags(
                        (0..count).map(|index| format!("flag {index}")).collect(),
                    ),
                });
                let height = shader_grid_row_height(ui, &row, 300.0);
                assert!(height >= count as f32 * 32.0 + 8.0);
                let rect = egui::Rect::from_min_size(
                    ui.cursor().min + Vec2::new(0.0, 4.0),
                    Vec2::new(300.0, height - 8.0),
                );
                let mut value_ui =
                    ui.new_child(egui::UiBuilder::new().id_salt(count).max_rect(rect));
                with_test_edit_context(|edit| {
                    draw_shader_editable_value(
                        &mut value_ui,
                        rect,
                        &row.label,
                        row.edit.as_ref().unwrap(),
                        edit,
                        &mut None,
                    );
                });
                assert!(value_ui.min_rect().bottom() <= rect.bottom() + 0.1);
            }
        });
    }

    /// Drag `entry` from a real browser row onto a real shader reference cell
    /// of `kind`, and return the field edits the cell committed.
    fn drop_onto(kind: ShaderRowEditKind, entry: &TagEntry, editable: bool) -> Vec<String> {
        let row_edit = ShaderRowEdit {
            path: "field".to_owned(),
            current: String::new(),
            kind,
        };
        let ctx = egui::Context::default();
        let row_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let cell_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut committed = Vec::new();
        let mut frame = |events: Vec<egui::Event>| {
            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::new(600.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let top = ui.cursor().min;
                        draw_entry(ui, entry, None, false, false, None, None, true, None);
                        row_rect.set(egui::Rect::from_min_size(
                            top,
                            Vec2::new(240.0, ui.spacing().interact_size.y),
                        ));
                        ui.add_space(120.0);
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(360.0, 22.0), Sense::hover());
                        // The cell's text box, left of Open and browse.
                        cell_rect.set(egui::Rect::from_min_size(rect.min, Vec2::new(200.0, 22.0)));
                        with_test_edit_context(|edit| {
                            edit.editable = editable;
                            draw_shader_editable_value(
                                ui,
                                rect,
                                "Reference",
                                &row_edit,
                                edit,
                                &mut None,
                            );
                            committed.extend(edit.pending.iter().map(|edit| edit.input.clone()));
                        });
                    });
                },
            );
        };
        frame(Vec::new());
        let (start, end) = (row_rect.get().center(), cell_rect.get().center());
        frame(vec![egui::Event::PointerMoved(start)]);
        frame(vec![egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        frame(vec![egui::Event::PointerMoved(
            start + Vec2::new(0.0, 40.0),
        )]);
        frame(vec![egui::Event::PointerMoved(end)]);
        frame(vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        committed
    }

    fn entry(path: &str, group: &[u8; 4]) -> TagEntry {
        TagEntry {
            key: format!("file:{path}"),
            display_path: path.to_owned(),
            group_tag: u32::from_be_bytes(*group),
            group_name: None,
            location: TagEntryLocation::LooseFile(std::path::PathBuf::from(path)),
        }
    }

    /// Every kind of reference cell takes a dropped tag of its own group, in
    /// its own committed form, through the one shared cell.
    #[test]
    fn each_reference_cell_takes_a_dropped_tag() {
        let bitmap = entry("objects/rifle/rifle.bitmap", b"bitm");
        assert_eq!(
            drop_onto(
                ShaderRowEditKind::BitmapRef {
                    group_tag: u32::from_be_bytes(*b"bitm"),
                    create: None,
                },
                &bitmap,
                true,
            ),
            ["objects/rifle/rifle.bitmap"],
        );
        let template = entry("shaders/opaque.shader_template", b"stem");
        assert_eq!(
            drop_onto(ShaderRowEditKind::ShaderTemplateRef, &template, true),
            ["stem:shaders\\opaque"],
        );
        let definition = entry("shaders/shader.render_method_definition", b"rmdf");
        let structural = || ShaderRowEditKind::StructuralRef {
            group_tag: u32::from_be_bytes(*b"rmdf"),
            extension: "render_method_definition",
        };
        let dropped = drop_onto(structural(), &definition, true);
        assert_eq!(dropped.len(), 1, "{dropped:?}");

        // A tag of another group is refused, and so is any drop on a
        // read-only tag — which only the bitmap cell used to refuse.
        assert!(drop_onto(structural(), &bitmap, true).is_empty());
        assert!(drop_onto(ShaderRowEditKind::ShaderTemplateRef, &template, false).is_empty());
    }

    /// Switching templates must retain authored parameters for recovery and explicit cleanup.
    #[test]
    fn switching_a_template_preserves_saved_parameters() {
        let row_edit = ShaderRowEdit {
            path: "template".to_owned(),
            current: String::new(),
            kind: ShaderRowEditKind::ShaderTemplateRef,
        };
        with_test_edit_context(|edit| {
            push_h2_template_reference_edit(
                edit,
                &row_edit,
                "shaders/other.shader_template".to_owned(),
            );
            assert_eq!(edit.pending.len(), 1);
            assert!(edit.h2_shader_param_ops.is_empty());
            assert!(edit.block_ops.is_empty());
        });
    }

    // Shader model, editing, and thumbnail unit tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    fn cell(text: &str) -> ShaderGridCell {
        ShaderGridCell {
            text: text.to_owned(),
            value_kind: "value",
            color: None,
        }
    }

    #[test]
    fn differs_compares_value_vs_default() {
        let mut row = empty_shader_grid_row();
        row.default_cell = Some(cell("value: 1.0"));
        row.value_cell = cell("value: 1.0");
        row.is_overridden = true;
        assert!(!row_differs_from_default(&row), "equal values don't differ");
        row.value_cell = cell("value: 2.0");
        assert!(row_differs_from_default(&row), "changed value differs");
        // numeric tolerance: "1" vs "1.0" are equal
        row.value_cell = cell("value: 1");
        assert!(!row_differs_from_default(&row));
        // inherited rows never count as modified, regardless of displayed text
        row.is_overridden = false;
        row.value_cell = cell("Override Default");
        assert!(!row_differs_from_default(&row));
        // no default => never modified
        row.default_cell = None;
        row.value_cell = cell("value: 9");
        assert!(!row_differs_from_default(&row));
    }

    #[test]
    fn downscale_rgba_caps_dimensions_and_preserves_corners() {
        // 4×2 image, two colors per row; downscale to fit within 2px.
        let red = [255u8, 0, 0, 255];
        let blue = [0u8, 0, 255, 255];
        let mut rgba = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                rgba.extend_from_slice(if x < 2 { &red } else { &blue });
            }
        }
        let (out, w, h) = downscale_rgba(&rgba, 4, 2, 2);
        assert_eq!((w, h), (2, 1), "scaled to fit within 2px, aspect kept");
        assert_eq!(out.len(), w * h * 4);
        // left sample is red, right sample is blue.
        assert_eq!(&out[0..4], &red);
        assert_eq!(&out[4..8], &blue);
        // malformed input yields an empty image.
        assert_eq!(downscale_rgba(&[], 4, 2, 2).1, 0);
    }

    #[test]
    fn reset_op_deletes_sparse_parameter_for_scalar_override() {
        let mut row = empty_shader_grid_row();
        row.default_cell = Some(cell("value: 0.5"));
        row.is_overridden = true;
        row.edit = Some(ShaderRowEdit {
            path: "parameters[0]/value".to_owned(),
            current: "2.0".to_owned(),
            kind: ShaderRowEditKind::Scalar,
        });
        let reset = reset_op_for_row(&row).expect("scalar override is clearable");
        assert_eq!(reset.path, "parameters");
        assert!(matches!(reset.kind, BlockOpKind::Delete(0)));
        // inherited rows do not produce a clear op.
        row.is_overridden = false;
        assert!(reset_op_for_row(&row).is_none());
        // rows without an edit path can't reset
        row.is_overridden = true;
        row.edit = None;
        assert!(reset_op_for_row(&row).is_none());
    }
}
