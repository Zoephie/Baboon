//! Shader grid, category, cell, and thumbnail rendering.
//! It owns shader-specific models, edits, and presentation helpers; generic field editing, and the commands that apply edits, belong elsewhere.

use super::*;

fn draw_shader_section(
    ui: &mut Ui,
    title: &str,
    key: impl std::hash::Hash + std::fmt::Debug,
    edit: &mut FieldEditContext<'_>,
    contents: impl FnOnce(&mut Ui, &mut FieldEditContext<'_>),
) {
    draw_foundation_clipped_shader_header(
        ui,
        title.to_owned(),
        ("shader_section", edit.view_scope, edit.tag_key, key),
        |ui| contents(ui, edit),
    );
}

/// Horizontal scrolling covers both the column header and vertically scrolling body.
pub(in crate::app) fn draw_shader_scroll_area(
    ui: &mut Ui,
    scope: impl std::hash::Hash + std::fmt::Debug,
    contents: impl FnOnce(&mut Ui),
) {
    let height = ui.available_height().max(0.0);
    ScrollArea::horizontal()
        .id_salt(("shader_horizontal", &scope))
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(TAG_FIELD_SCROLL_MIN_WIDTH);
            draw_shader_columns_header(ui);
            ScrollArea::vertical()
                .id_salt(("shader_vertical", &scope))
                .max_height(ui.available_height().max(0.0))
                .auto_shrink([false, false])
                .show(ui, contents);
        });
}

pub(in crate::app) fn draw_shader_editor_model(
    ui: &mut Ui,
    model: &ShaderEditorModel,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    edit: &mut FieldEditContext<'_>,
    expert_mode: bool,
) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.set_width(shader_grid_width(ui));
    for row in &model.top_rows {
        draw_shader_grid_row(ui, row, 0, color_popup, function_popup, edit);
    }
    // MATERIAL section only for material-bearing shader types (Guerilla
    // vtable+0x70 gate). Effect-style shaders have no global material type.
    if model.has_material_row && !model.materials.is_empty() {
        draw_shader_section(ui, "MATERIAL", "material", edit, |ui, edit| {
            for material in &model.materials {
                let material_row = ShaderGridRow {
                    label: material.label.clone(),
                    default_cell: Some(ShaderGridCell {
                        text: "default_material".to_owned(),
                        value_kind: "default",
                        color: None,
                    }),
                    value_cell: ShaderGridCell {
                        text: material.value.clone(),
                        value_kind: "value",
                        color: None,
                    },
                    fill: material_data_row(),
                    parameter_type: Some("string id".to_owned()),
                    is_overridden: true,
                    function: None,
                    edit: Some(ShaderRowEdit {
                        path: material.edit_path.clone(),
                        current: material.value.clone(),
                        kind: ShaderRowEditKind::StringId,
                    }),
                    context_menu: None,
                    create_anim_op: None,
                    constant_function_view: None,
                };
                draw_shader_grid_row(ui, &material_row, 0, color_popup, function_popup, edit);
            }
        });
    }

    if !model.definition_path.is_empty() {
        let definition_row = ShaderGridRow {
            label: "definition".to_owned(),
            default_cell: None,
            value_cell: ShaderGridCell {
                text: format!("{}.render_method_definition", model.definition_path),
                value_kind: "value",
                color: None,
            },
            fill: material_ref_row(),
            parameter_type: Some("tag reference".to_owned()),
            is_overridden: true,
            function: None,
            edit: structural_reference_edit(
                expert_mode,
                &model.definition_edit_path,
                &model.definition_path,
                *b"rmdf",
                "render_method_definition",
            ),
            context_menu: None,
            create_anim_op: None,
            constant_function_view: None,
        };
        draw_shader_grid_row(ui, &definition_row, 0, color_popup, function_popup, edit);
    }

    if let Some(template_path) = model.shader_template_path.as_deref() {
        let template_row = ShaderGridRow {
            label: "shader template".to_owned(),
            default_cell: None,
            value_cell: ShaderGridCell {
                text: format!("{template_path}.render_method_template"),
                value_kind: "value",
                color: None,
            },
            fill: material_ref_row(),
            parameter_type: Some("tag reference".to_owned()),
            is_overridden: true,
            function: None,
            edit: structural_reference_edit(
                expert_mode,
                &model.shader_template_edit_path,
                template_path,
                *b"rmt2",
                "render_method_template",
            ),
            context_menu: None,
            create_anim_op: None,
            constant_function_view: None,
        };
        draw_shader_grid_row(ui, &template_row, 0, color_popup, function_popup, edit);
    }

    if !model.categories.is_empty() {
        draw_shader_section(ui, "CATEGORIES", "categories", edit, |ui, edit| {
            for category in &model.categories {
                draw_shader_category_row(ui, category, edit);
            }
        });
    }

    for (section_index, section) in model.sections.iter().enumerate() {
        draw_shader_section(
            ui,
            &section.title,
            ("parameters", section_index),
            edit,
            |ui, edit| {
                // Scoped per section: every row's widget ids key off its label, and
                // labels repeat across sections — every section has a
                // "selected option" row, and two rmops are free to declare the same
                // parameter name. Without this scope those rows share egui ids and
                // their hover/drag state cross-wires (the debug build paints the
                // "first use of widget ID" clash warning right on the grid).
                ui.push_id(("shader_section", section_index), |ui| {
                    if !section.option_name.is_empty() {
                        let option_row = ShaderGridRow {
                            label: "selected option".to_owned(),
                            default_cell: None,
                            value_cell: ShaderGridCell {
                                text: section.option_name.clone(),
                                value_kind: "value",
                                color: None,
                            },
                            fill: material_data_row(),
                            parameter_type: Some("option".to_owned()),
                            is_overridden: true,
                            function: None,
                            edit: None,
                            context_menu: None,
                            create_anim_op: None,
                            constant_function_view: None,
                        };
                        draw_shader_grid_row(ui, &option_row, 0, color_popup, function_popup, edit);
                    }
                    for row in &section.rows {
                        draw_shader_grid_row(ui, row, 0, color_popup, function_popup, edit);
                    }
                    for (parameter_index, parameter) in model
                        .unused_parameters
                        .iter()
                        .enumerate()
                        .filter(|(_, parameter)| {
                            parameter.category.as_ref() == Some(&section.title)
                        })
                    {
                        for (row_index, row) in parameter.rows.iter().enumerate() {
                            ui.push_id(("unused_parameter", parameter_index, row_index), |ui| {
                                draw_unused_shader_grid_row(
                                    ui,
                                    row,
                                    color_popup,
                                    function_popup,
                                    edit,
                                    &parameter.delete,
                                );
                            });
                        }
                    }
                });
            },
        );
    }

    if model
        .unused_parameters
        .iter()
        .any(|parameter| parameter.category.is_none())
    {
        draw_shader_section(
            ui,
            "UNUSED PARAMETERS",
            "unused_parameters",
            edit,
            |ui, edit| {
                for (parameter_index, parameter) in model
                    .unused_parameters
                    .iter()
                    .enumerate()
                    .filter(|(_, parameter)| parameter.category.is_none())
                {
                    for (row_index, row) in parameter.rows.iter().enumerate() {
                        ui.push_id(("unused_parameter", parameter_index, row_index), |ui| {
                            draw_unused_shader_grid_row(
                                ui,
                                row,
                                color_popup,
                                function_popup,
                                edit,
                                &parameter.delete,
                            );
                        });
                    }
                }
            },
        );
    }

    if !model.atmosphere_flags.options.is_empty()
        || !model.custom_fog_setting_index.label.is_empty()
    {
        draw_shader_section(
            ui,
            "ATMOSPHERE PROPERTIES",
            "atmosphere",
            edit,
            |ui, edit| {
                if !model.atmosphere_flags.options.is_empty() {
                    draw_shader_flags_row(ui, &model.atmosphere_flags, edit);
                }
                if !model.custom_fog_setting_index.label.is_empty() {
                    draw_shader_grid_row(
                        ui,
                        &model.custom_fog_setting_index,
                        0,
                        color_popup,
                        function_popup,
                        edit,
                    );
                }
            },
        );
    }

    if !model.sort_layer.label.is_empty() {
        draw_shader_section(ui, "SORTING PROPERTIES", "sorting", edit, |ui, edit| {
            draw_shader_grid_row(ui, &model.sort_layer, 0, color_popup, function_popup, edit);
        });
    }
}

pub(in crate::app) fn draw_shader_category_row(
    ui: &mut Ui,
    category: &ShaderEditorCategory,
    edit: &mut FieldEditContext<'_>,
) {
    let available = shader_grid_width(ui);
    let label_width = shader_label_width(ui);
    let default_width = shader_default_width(ui);
    let value_width = (available - label_width - default_width - 26.0).max(240.0);
    let height = BUTTON_HEIGHT + 8.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(available, height), Sense::hover());
    let row_fill = Color32::TRANSPARENT;
    ui.painter().rect_filled(rect, 0.0, row_fill);
    shader_row_separator(ui, rect);
    let label_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(4.0, 0.0),
        Vec2::new(label_width, height),
    );
    ui.painter().text(
        label_rect.right_center() - Vec2::new(6.0, 0.0),
        Align2::RIGHT_CENTER,
        truncate_for_cell(&category.name, label_width - 12.0),
        FontId::proportional(12.5),
        material_text(),
    );

    let default_rect = egui::Rect::from_min_size(
        label_rect.right_top() + Vec2::new(2.0, 4.0),
        Vec2::new(default_width, height - 8.0),
    );
    let default_text = category
        .options
        .first()
        .cloned()
        .unwrap_or_else(|| "NONE".to_owned());
    let default_cell = ShaderGridCell {
        text: default_text,
        value_kind: "default",
        color: None,
    };
    let mut no_color_popup = None;
    draw_shader_grid_cell(
        ui,
        default_rect,
        Some(&default_cell),
        &format!("category_default:{}", category.name),
        &mut no_color_popup,
    );

    let combo_rect = egui::Rect::from_min_size(
        default_rect.right_top() + Vec2::new(6.0, 0.0),
        Vec2::new(value_width, height - 8.0),
    );
    let selected_index = category.selected.max(0) as usize;
    let selected_text = category
        .options
        .get(selected_index)
        .cloned()
        .unwrap_or_else(|| "NONE".to_owned());
    let editable = edit.editable && category.edit_path.is_some();
    shader_cell_scope(ui, combo_rect, |ui| {
        ui.add_enabled_ui(editable, |ui| {
            let (_, wheel_delta) = combo_box_with_scroll(
                ui,
                egui::ComboBox::from_id_salt((
                    edit.view_scope,
                    edit.tag_key,
                    "shader_category",
                    category.index,
                ))
                .selected_text(selected_text)
                .width(value_width),
                |ui| {
                    for (index, option) in category.options.iter().enumerate() {
                        let selected = index == selected_index;
                        if ui.selectable_label(selected, option).clicked() {
                            if let Some(path) = category.edit_path.as_ref() {
                                edit.pending.push(PendingFieldEdit {
                                    path: path.clone(),
                                    input: index.to_string(),
                                });
                            }
                        }
                    }
                },
            );
            if let Some(delta) = wheel_delta
                && let Some(next) =
                    combo_scroll_next_index(selected_index, category.options.len(), delta)
                && let Some(path) = category.edit_path.as_ref()
            {
                edit.pending.push(PendingFieldEdit {
                    path: path.clone(),
                    input: next.to_string(),
                });
            }
        });
    });

    if !editable {
        ui.painter().text(
            combo_rect.right_center() + Vec2::new(8.0, 0.0),
            Align2::LEFT_CENTER,
            if edit.editable {
                "missing option slot"
            } else {
                "read-only"
            },
            FontId::proportional(11.0),
            material_muted_text(),
        );
    }
}

pub(in crate::app) fn draw_material_template_summary(
    ui: &mut Ui,
    tag: &TagFile,
    names: &TagNameIndex,
    color_popup: &mut Option<MaterialColorPopup>,
) {
    let mut references = Vec::new();
    collect_shader_template_references(tag.root(), names, 0, &mut references);
    if references.is_empty() {
        return;
    }

    draw_shader_grid_section_header(ui, "SHADER TEMPLATE");
    let mut seen = HashSet::new();
    let mut no_function_popup = None;
    for (label, value) in references {
        if !seen.insert(format!("{label}:{value}")) {
            continue;
        }
        let cell = ShaderGridCell {
            text: value,
            value_kind: "value",
            color: None,
        };
        let row = ShaderGridRow {
            label,
            default_cell: None,
            value_cell: cell,
            fill: material_ref_row(),
            parameter_type: Some("tag reference".to_owned()),
            is_overridden: true,
            function: None,
            edit: None,
            context_menu: None,
            create_anim_op: None,
            constant_function_view: None,
        };
        draw_shader_grid_row_readonly(ui, &row, 0, color_popup, &mut no_function_popup);
    }
}

pub(in crate::app) fn collect_shader_template_references(
    tag_struct: TagStruct<'_>,
    names: &TagNameIndex,
    depth: usize,
    out: &mut Vec<(String, String)>,
) {
    for field in tag_struct.fields() {
        let key = clean_field_key(field.name());
        if is_shader_template_reference_key(&key) {
            if let Some(value) = field.value() {
                let formatted = trim_formatted_value(&format_value(names, &value, false));
                if !formatted.is_empty() && !is_none_like_value(&formatted) {
                    out.push((shader_template_label(&key), formatted));
                }
            }
            continue;
        }

        if depth >= 2 || is_material_parameters_field(field.name()) {
            continue;
        }
        if let Some(nested) = field.as_struct() {
            collect_shader_template_references(nested, names, depth + 1, out);
        } else if key.contains("postprocess") {
            if let Some(block) = field.as_block() {
                for element in block.iter().take(2) {
                    collect_shader_template_references(element, names, depth + 1, out);
                }
            }
        }
    }
}

/// The editor for a shader's `definition` / `shader template` reference, or
/// `None` when the field is not on this tag — in which case the row stays the
/// read-only text it has always been rather than offering an edit that could
/// not commit. Whether the user may actually type into it is the field
/// editor's own `editable` gate, which is what expert mode drives.
fn structural_reference_edit(
    expert_mode: bool,
    edit_path: &str,
    current_path: &str,
    group_tag: [u8; 4],
    extension: &'static str,
) -> Option<ShaderRowEdit> {
    // Re-pointing either reference changes which parameters the tag is
    // supposed to carry, and nothing reconciles the ones already on it —
    // Foundation gates the same edit behind expert mode and reconciles nothing
    // either. Outside expert mode the row stays the text it has always been.
    if !expert_mode || edit_path.is_empty() {
        return None;
    }
    let current = if current_path.is_empty() {
        "NONE".to_owned()
    } else {
        format!("{}.{extension}", current_path.replace('\\', "/"))
    };
    Some(ShaderRowEdit {
        path: edit_path.to_owned(),
        current,
        kind: ShaderRowEditKind::StructuralRef {
            group_tag: u32::from_be_bytes(group_tag),
            extension,
        },
    })
}

pub(in crate::app) fn shader_template_label(key: &str) -> String {
    match key {
        "material shader" => "material shader".to_owned(),
        "shader template" => "shader template".to_owned(),
        "definition" => "shader definition".to_owned(),
        _ => key.to_owned(),
    }
}

pub(in crate::app) fn is_shader_template_reference_key(key: &str) -> bool {
    matches!(key, "material shader" | "shader template" | "definition")
}

pub(in crate::app) fn draw_material_parameters_block(
    ui: &mut Ui,
    block: TagBlock<'_>,
    names: &TagNameIndex,
    depth: usize,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
) {
    egui::CollapsingHeader::new(material_section_text(format!(
        "material parameters  [{} elements]",
        block.len()
    )))
    .default_open(true)
    .show(ui, |ui| {
        let mut rows: Vec<(&'static str, ShaderGridRow)> = Vec::new();
        for (index, element) in block.iter().enumerate() {
            let label = material_parameter_name(element, names)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("[{index}] {}", element.name()));
            let parameter_type = material_parameter_type(element, names);
            let mut values = material_parameter_values(element, names);
            values.sort_by_key(|value| value.priority);
            let function = find_first_function(element);
            let row = shader_grid_row_from_parameter(&label, parameter_type, values, function);
            rows.push((material_parameter_section(&label), row));
        }

        let mut last_section = "";
        for section in MATERIAL_PARAMETER_SECTIONS {
            for (_, row) in rows
                .iter()
                .filter(|(row_section, _)| row_section == section)
            {
                if last_section != *section {
                    draw_shader_grid_section_header(ui, section);
                    last_section = section;
                }
                draw_shader_grid_row_readonly(ui, row, depth + 1, color_popup, function_popup);
            }
        }
        for (section, row) in rows
            .iter()
            .filter(|(row_section, _)| !MATERIAL_PARAMETER_SECTIONS.contains(row_section))
        {
            if last_section != *section {
                draw_shader_grid_section_header(ui, section);
                last_section = section;
            }
            draw_shader_grid_row_readonly(ui, row, depth + 1, color_popup, function_popup);
        }
    });
}

pub(in crate::app) fn shader_grid_row_from_parameter(
    label: &str,
    parameter_type: Option<String>,
    values: Vec<MaterialParameterValue>,
    function: Option<FunctionView>,
) -> ShaderGridRow {
    let mut values = values.into_iter();
    let first = values.next();
    let second = values.next();

    let default_cell = first.as_ref().map(shader_cell_from_material_value);
    let mut value_cell = second
        .as_ref()
        .or(first.as_ref())
        .map(shader_cell_from_material_value)
        .unwrap_or_else(|| ShaderGridCell {
            text: "Override Default".to_owned(),
            value_kind: "default",
            color: None,
        });

    let mut fill = second
        .as_ref()
        .or(first.as_ref())
        .map(|value| value.fill)
        .unwrap_or(material_data_row());

    if function.is_some() {
        if let Some(function) = function.as_ref() {
            value_cell.text = shader_function_grid_text(&function.function);
        }
        value_cell.value_kind = "value";
        fill = material_function_row();
    }

    ShaderGridRow {
        label: label.to_owned(),
        default_cell: default_cell.or_else(|| shader_default_cell(parameter_type.as_deref())),
        value_cell,
        fill,
        parameter_type,
        is_overridden: true,
        function,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

/// Render a shader grid row with no edit capability (used by the read-only
/// `.material` / `.material_shader` views, which have no edit context).
pub(in crate::app) fn draw_shader_grid_row_readonly(
    ui: &mut Ui,
    row: &ShaderGridRow,
    depth: usize,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
) {
    let mut sinks = EditSinks::default();
    // The shader grid draws no collapsible containers of its own.
    let mut ctx = FieldEditContext::read_only(&mut sinks, "readonly", "");
    draw_shader_grid_row(ui, row, depth, color_popup, function_popup, &mut ctx);
}

pub(in crate::app) fn shader_cell_from_material_value(
    value: &MaterialParameterValue,
) -> ShaderGridCell {
    ShaderGridCell {
        text: shader_grid_value_text(value),
        value_kind: value.value_kind,
        color: value.color.clone(),
    }
}

pub(in crate::app) fn shader_grid_value_text(value: &MaterialParameterValue) -> String {
    let key = clean_field_key(&value.label);
    if value.color.is_some() {
        return "color: RGB".to_owned();
    }
    if key == "real" {
        return format!("value: {}", value.value);
    }
    if key == "vector" {
        return format!("vector: {}", value.value);
    }
    if key == "int/bool" {
        return format!("value: {}", value.value);
    }
    value.value.clone()
}

pub(in crate::app) fn shader_default_cell(parameter_type: Option<&str>) -> Option<ShaderGridCell> {
    let parameter_type = parameter_type?;
    Some(ShaderGridCell {
        text: parameter_type.to_owned(),
        value_kind: "default",
        color: None,
    })
}

pub(in crate::app) fn draw_shader_grid_section_header(ui: &mut Ui, title: &str) {
    draw_foundation_collapsing_header(
        ui,
        title.to_owned(),
        ("shader_static_section", title),
        0,
        false,
        None,
        foundation_block_bar(),
        FindTargetKind::Block,
        false,
        None,
        |_| {},
    );
}

pub(in crate::app) fn draw_shader_flags_row(
    ui: &mut Ui,
    row: &ShaderFlagsRow,
    edit: &mut FieldEditContext<'_>,
) {
    let available = shader_grid_width(ui);
    let label_width = shader_label_width(ui);
    let default_width = shader_default_width(ui);
    let value_width =
        (available - label_width - default_width - 12.0 - SHADER_ROW_RIGHT_PADDING).max(40.0);
    let mut measure = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("shader_flags_measure", &row.path))
            .max_rect(egui::Rect::from_min_size(
                ui.cursor().min,
                Vec2::new(value_width - 8.0, 0.0),
            ))
            .layout(egui::Layout::top_down(egui::Align::Min))
            .invisible(),
    );
    measure.spacing_mut().item_spacing.y = 0.0;
    for option in &row.options {
        measure.checkbox(&mut false, option.label);
    }
    let height = (measure.min_rect().height() + 16.0).max(BUTTON_HEIGHT + 8.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(available, height), Sense::hover());
    let label_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(4.0, 0.0),
        Vec2::new(label_width, height),
    );
    ui.painter().text(
        label_rect.right_top() + Vec2::new(-6.0, shader_label_top_padding(ui)),
        Align2::RIGHT_TOP,
        truncate_for_cell(&row.label, label_width - 12.0),
        FontId::proportional(12.5),
        material_text(),
    );
    let default_rect = egui::Rect::from_min_size(
        label_rect.right_top() + Vec2::new(2.0, 4.0),
        Vec2::new(default_width, height - 8.0),
    );
    draw_shader_grid_cell(ui, default_rect, None, "default:flags", &mut None);
    let value_rect = egui::Rect::from_min_max(
        default_rect.right_top() + Vec2::new(6.0, 0.0),
        egui::pos2(
            rect.right() - SHADER_ROW_RIGHT_PADDING,
            default_rect.bottom(),
        ),
    );
    shader_input_box(ui, value_rect);
    let enabled = edit.editable && !row.path.is_empty();
    shader_cell_scope(ui, value_rect.shrink(4.0), |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for option in &row.options {
                let mut checked = row.raw & (1u64 << option.bit) != 0;
                let response = ui
                    .push_id(
                        (edit.view_scope, edit.tag_key, &row.path, option.bit),
                        |ui| {
                            ui.add_enabled(enabled, egui::Checkbox::new(&mut checked, option.label))
                        },
                    )
                    .inner;
                if response.changed() {
                    let next_mask = if checked {
                        row.raw | (1u64 << option.bit)
                    } else {
                        row.raw & !(1u64 << option.bit)
                    };
                    edit.pending.push(PendingFieldEdit {
                        path: row.path.clone(),
                        input: next_mask.to_string(),
                    });
                }
            }
        });
    });
    shader_row_separator(ui, rect);
}

/// Accent painted on the left edge of a shader row whose value differs from the
/// rmop/template default (Phase 4.1 "differs-from-default" indicator).
pub(in crate::app) fn draw_shader_grid_cell(
    ui: &mut Ui,
    rect: egui::Rect,
    cell: Option<&ShaderGridCell>,
    id_source: &str,
    color_popup: &mut Option<MaterialColorPopup>,
) {
    draw_shader_grid_cell_with_icon(ui, rect, cell, id_source, color_popup, None, None);
}

pub(super) fn shader_tag_icon_rect(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_center_size(
        egui::pos2(rect.left() + 11.0, rect.top() + BUTTON_HEIGHT / 2.0),
        Vec2::splat(16.0),
    )
}

pub(super) fn draw_shader_grid_cell_with_icon(
    ui: &mut Ui,
    rect: egui::Rect,
    cell: Option<&ShaderGridCell>,
    id_source: &str,
    color_popup: &mut Option<MaterialColorPopup>,
    icon_group: Option<u32>,
    game: Option<GameId>,
) {
    let mut cell_ui = ui.new_child(egui::UiBuilder::new().id_salt(id_source).max_rect(rect));
    if id_source.starts_with("default:") || id_source.starts_with("category_default:") {
        cell_ui.multiply_opacity(0.5);
    }
    let ui = &mut cell_ui;
    let fill = material_input();
    let text_color = material_text();
    let visuals = &ui.visuals().widgets.inactive;
    ui.painter().rect_filled(rect, visuals.corner_radius, fill);
    ui.painter().rect_stroke(
        rect,
        visuals.corner_radius,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );

    if icon_group.is_some() {
        paint_tag_icon_at(ui, icon_group, game, shader_tag_icon_rect(rect));
    }

    let Some(cell) = cell else {
        return;
    };

    let mut text_left =
        rect.left_center() + Vec2::new(if icon_group.is_some() { 23.0 } else { 5.0 }, 0.0);
    if rect.height() > BUTTON_HEIGHT {
        text_left.y = rect.top() + BUTTON_HEIGHT / 2.0;
    }
    if let Some(color) = cell.color.as_ref() {
        let swatch_rect = shader_color_swatch_rect(rect);
        text_left.x = swatch_rect.right() + 5.0;
        draw_shader_color_swatch(ui, rect, color.color32());
        let swatch_response = ui
            .interact(
                swatch_rect,
                ui.make_persistent_id(format!("shader_color:{id_source}:{}", cell.text)),
                Sense::click(),
            )
            .on_hover_text("Click to show Foundation color values");
        if swatch_response.clicked() {
            *color_popup = Some(color.clone());
        }
    }

    let shown = truncate_for_cell(
        shader_reference_name(&cell.text),
        rect.right() - text_left.x - 5.0,
    );
    if shown != cell.text {
        ui.interact(
            rect,
            ui.make_persistent_id(("shader_cell_tooltip", id_source)),
            Sense::hover(),
        )
        .on_hover_text(&cell.text);
    }
    ui.painter().text(
        text_left,
        Align2::LEFT_CENTER,
        shown,
        egui::TextStyle::Body.resolve(ui.style()),
        text_color,
    );
}

pub(in crate::app) fn material_parameter_name(
    element: TagStruct<'_>,
    names: &TagNameIndex,
) -> Option<String> {
    for field in element.fields() {
        if !clean_field_key(field.name()).starts_with("parameter name") {
            continue;
        }
        let value = field.value()?;
        return Some(trim_formatted_value(&format_value(names, &value, false)));
    }
    None
}

pub(in crate::app) fn material_parameter_type(
    element: TagStruct<'_>,
    names: &TagNameIndex,
) -> Option<String> {
    for field in element.fields() {
        if !clean_field_key(field.name()).starts_with("parameter type") {
            continue;
        }
        let value = field.value()?;
        let formatted = trim_formatted_value(&format_value(names, &value, false));
        return enum_display_name(&formatted).or(Some(formatted));
    }
    None
}

pub(in crate::app) fn material_parameter_section(label: &str) -> &'static str {
    let key = label.to_ascii_lowercase();
    if key.contains("base")
        || key.contains("albedo")
        || key.contains("change_color")
        || key.contains("change color")
        || key.contains("detail")
        || key.contains("color_map")
        || key.contains("color map")
    {
        "ALBEDO"
    } else if key.contains("bump") || key.contains("normal") {
        "BUMP_MAPPING"
    } else if key.contains("env") || key.contains("environment") {
        "ENVIRONMENT_MAPPING"
    } else if key.contains("self_illum") || key.contains("self illum") || key.contains("illum") {
        "SELF_ILLUMINATION"
    } else if key.contains("atmosphere")
        || key.contains("fog")
        || key.contains("soft")
        || key.contains("distortion")
        || key.contains("parallax")
        || key.contains("misc")
    {
        "ATMOSPHERE PROPERTIES"
    } else if key.contains("diffuse")
        || key.contains("specular")
        || key.contains("fresnel")
        || key.contains("roughness")
        || key.contains("coefficient")
        || key.contains("material")
        || key.contains("blend")
        || key.contains("analytic")
        || key.contains("area")
        || key.contains("dynamic")
        || key.contains("order3")
    {
        "MATERIAL_MODEL"
    } else {
        "MISC"
    }
}

pub(in crate::app) fn material_parameter_values(
    element: TagStruct<'_>,
    names: &TagNameIndex,
) -> Vec<MaterialParameterValue> {
    let parameter_type = material_parameter_type(element, names)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut values = Vec::new();

    for field in element.fields() {
        let key = clean_field_key(field.name());
        if is_material_parameter_metadata(&key) {
            continue;
        }
        let Some(value) = field.value() else {
            continue;
        };
        if !material_parameter_field_matches_type(&key, &parameter_type) {
            continue;
        }

        let raw_formatted = format_value(names, &value, false);
        let mut formatted = trim_formatted_value(&raw_formatted);
        if formatted.is_empty() || should_skip_material_parameter_value(&key, &formatted) {
            continue;
        }
        let color = color_popup_for_value(
            material_parameter_color_title(element, names, field.name()).as_str(),
            &value,
            &formatted,
        );
        if let Some(color) = color.as_ref() {
            formatted = color.sc_hex.clone();
        }

        values.push(MaterialParameterValue {
            label: field.name().to_owned(),
            value: formatted,
            fill: material_row_tint(&value),
            value_kind: material_value_kind(&value),
            color,
            priority: material_parameter_value_priority(&key),
        });
    }

    values
}

pub(in crate::app) fn find_first_function(tag_struct: TagStruct<'_>) -> Option<FunctionView> {
    for field in tag_struct.fields() {
        if let Some(function) = field.as_function() {
            return Some(FunctionView::from_function(function));
        }
        if let Some(nested) = field.as_struct() {
            if let Some(function) = find_first_function(nested) {
                return Some(function);
            }
        }
        if let Some(block) = field.as_block() {
            for element in block.iter() {
                if let Some(function) = find_first_function(element) {
                    return Some(function);
                }
            }
        }
        if let Some(array) = field.as_array() {
            for element in array.iter() {
                if let Some(function) = find_first_function(element) {
                    return Some(function);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod shader_section_tests {
    use super::*;
    use crate::app::editor::fields::with_test_edit_context;

    #[test]
    fn shader_section_clips_the_last_row_accent_and_suppresses_duplicate_bottom_border() {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        egui_extras::install_image_loaders(&ctx);
        let body = std::cell::Cell::new(egui::Rect::NOTHING);
        let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            with_test_edit_context(|edit| {
                draw_shader_section(ui, "TEXTURE", "clip_test", edit, |ui, _| {
                    let (rect, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
                    body.set(rect);
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(rect.min, Vec2::new(3.0, 32.0)),
                        0.0,
                        Color32::from_rgb(224, 158, 62),
                    );
                    shader_row_separator(ui, rect);
                });
            });
        });
        let accent = output
            .shapes
            .iter()
            .find_map(|paint| match &paint.shape {
                egui::Shape::Path(path) if path.fill == Color32::from_rgb(224, 158, 62) => {
                    Some((paint, path))
                }
                _ => None,
            })
            .expect("the accent is clipped to a polygon at the rounded corner");
        let center = body.get().left_bottom() + Vec2::new(5.0, -5.0);
        for point in &accent.1.points {
            assert!(accent.0.clip_rect.contains(*point));
            if point.x < center.x && point.y > center.y {
                assert!(
                    point.distance_sq(center) <= 16.01,
                    "accent must stay inside the rounded corner"
                );
            }
        }
        assert!(!output.shapes.iter().any(|paint| matches!(&paint.shape,
            egui::Shape::LineSegment { points, .. } if points.iter().all(|point| (point.y - body.get().bottom() + 0.5).abs() < 0.1))),
            "the container footer replaces the last row separator");
    }

    #[test]
    fn shader_column_header_stays_fixed_while_the_body_scrolls() {
        let ctx = egui::Context::default();
        let body_y = std::cell::Cell::new(0.0);
        let frame = |events: Vec<egui::Event>| {
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1200.0, 260.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        draw_shader_scroll_area(ui, "sticky_test", |ui| {
                            body_y.set(ui.cursor().min.y);
                            for index in 0..40 {
                                ui.label(format!("row {index}"));
                            }
                        });
                    });
                },
            );
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "field" => Some(text.pos.y),
                    _ => None,
                })
                .unwrap()
        };
        let header_y = frame(Vec::new());
        let initial_body_y = body_y.get();
        frame(vec![egui::Event::PointerMoved(egui::pos2(300.0, 120.0))]);
        frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, -160.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        }]);
        for _ in 0..8 {
            assert_eq!(frame(Vec::new()), header_y);
        }
        assert!(
            body_y.get() < initial_body_y - 1.0,
            "only the body should move vertically"
        );
    }

    #[test]
    fn shader_section_toggle_keeps_other_tags_open() {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        egui_extras::install_image_loaders(&ctx);
        let point = std::cell::Cell::new(egui::Pos2::ZERO);
        let visible = std::cell::Cell::new(false);
        let frame = |events: Vec<egui::Event>, tag: &'static str| {
            visible.set(false);
            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    point.set(ui.cursor().min + Vec2::new(20.0, 28.0));
                    with_test_edit_context(|edit| {
                        edit.tag_key = tag;
                        draw_shader_section(ui, "TEXTURE", "texture", edit, |ui, _| {
                            visible.set(true);
                            ui.label("base_map");
                        });
                    });
                },
            );
        };
        frame(Vec::new(), "shader-a");
        assert!(visible.get());
        let pos = point.get();
        frame(
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            "shader-a",
        );
        frame(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            "shader-a",
        );
        frame(Vec::new(), "shader-a");
        assert!(!visible.get());
        frame(Vec::new(), "shader-b");
        assert!(visible.get());
        frame(Vec::new(), "shader-a");
        assert!(!visible.get());
    }
}
