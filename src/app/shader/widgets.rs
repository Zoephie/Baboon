//! Shader grid, category, cell, and thumbnail rendering.
//! It owns shader-specific models, edits, and presentation helpers; generic field editing and controller orchestration belong elsewhere.

use super::*;

pub(in crate::app) fn draw_shader_editor_model(
    ui: &mut Ui,
    model: &ShaderEditorModel,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    edit: &mut FieldEditContext<'_>,
) {
    // MATERIAL section only for material-bearing shader types (Guerilla
    // vtable+0x70 gate). Effect-style shaders have no global material type.
    if model.has_material_row {
        draw_shader_grid_section_header(ui, "MATERIAL");
        let mat_edit_path = &model.global_material_edit_path;
        let material_row = ShaderGridRow {
            label: "global material type".to_owned(),
            default_cell: Some(ShaderGridCell {
                text: "default_material".to_owned(),
                value_kind: "default",
                color: None,
            }),
            value_cell: ShaderGridCell {
                text: model.global_material_type.clone(),
                value_kind: "value",
                color: None,
            },
            fill: material_data_row(),
            parameter_type: Some("string id".to_owned()),
            is_overridden: true,
            function: None,
            edit: if mat_edit_path.is_empty() {
                None
            } else {
                Some(ShaderRowEdit {
                    path: mat_edit_path.clone(),
                    current: model.global_material_type.clone(),
                    kind: ShaderRowEditKind::StringId,
                })
            },
            context_menu: None,
            create_anim_op: None,
            constant_function_view: None,
        };
        draw_shader_grid_row(ui, &material_row, 0, color_popup, function_popup, edit);
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
            edit: None,
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
            edit: None,
            context_menu: None,
            create_anim_op: None,
            constant_function_view: None,
        };
        draw_shader_grid_row(ui, &template_row, 0, color_popup, function_popup, edit);
    }

    if !model.categories.is_empty() {
        draw_shader_grid_section_header(ui, "CATEGORIES");
        for category in &model.categories {
            draw_shader_category_row(ui, category, edit);
        }
    }

    for section in &model.sections {
        draw_shader_grid_section_header(ui, &section.title);
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
    }

    if !model.atmosphere_flags.options.is_empty()
        || !model.custom_fog_setting_index.label.is_empty()
    {
        draw_shader_grid_section_header(ui, "ATMOSPHERE PROPERTIES");
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
    }

    if !model.sort_layer.label.is_empty() {
        draw_shader_grid_section_header(ui, "SORTING PROPERTIES");
        draw_shader_grid_row(ui, &model.sort_layer, 0, color_popup, function_popup, edit);
    }
}

pub(in crate::app) fn draw_shader_category_row(
    ui: &mut Ui,
    category: &ShaderEditorCategory,
    edit: &mut FieldEditContext<'_>,
) {
    let available = ui.available_width().max(780.0);
    let label_width = shader_label_width(ui);
    let default_width = 110.0;
    let value_width = (available - label_width - default_width - 32.0).max(240.0);
    let height = 25.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(available, height), Sense::hover());
    let row_fill = material_data_row();
    ui.painter().rect_filled(rect, 0.0, row_fill);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, material_grid_light()),
    );
    let label_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(4.0, 0.0),
        Vec2::new(label_width, height),
    );
    ui.painter().text(
        label_rect.right_center() - Vec2::new(6.0, 0.0),
        Align2::RIGHT_CENTER,
        truncate_for_cell(&category.name, label_width - 12.0),
        FontId::proportional(12.5),
        material_text_for_bg(row_fill),
    );

    let default_rect = egui::Rect::from_min_size(
        label_rect.right_top() + Vec2::new(2.0, 2.0),
        Vec2::new(default_width, height - 4.0),
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
        Vec2::new(value_width, height - 4.0),
    );
    let selected_index = category.selected.max(0) as usize;
    let selected_text = category
        .options
        .get(selected_index)
        .cloned()
        .unwrap_or_else(|| "NONE".to_owned());
    let editable = edit.editable && category.edit_path.is_some();
    ui.scope_builder(egui::UiBuilder::new().max_rect(combo_rect), |ui| {
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
    let mut pending = Vec::new();
    let mut block_ops = Vec::new();
    let mut shader_ops = Vec::new();
    let mut shader_param_ops = Vec::new();
    let mut h2_shader_param_ops = Vec::new();
    let mut function_data_ops = Vec::new();
    let mut model_variant_ops = Vec::new();
    let mut block_confirm = None;
    let mut open_request = None;
    let mut sound_play_request = None;
    let mut sound_extract_request = None;
    let mut tool_import = None;
    let mut bitmap_reimport = None;
    let mut buffers = HashMap::new();
    let mut color_request = None;
    let mut function_request = None;
    let mut block_clip_request = None;
    let mut tsv_paste_request = None;
    let mut ctx = FieldEditContext {
        view_scope: "readonly",
        tag_key: "",
        group_tag: 0,
        root: None,
        game: None,
        definitions_root: None,
        names: None,
        tags_root: None,
        status: None,
        editable: false,
        show_block_sizes: false,
        buffers: &mut buffers,
        pending: &mut pending,
        block_ops: &mut block_ops,
        block_confirm: &mut block_confirm,
        open_request: &mut open_request,
        sound_play_request: &mut sound_play_request,
        sound_status: None,
        sound_volume: 1.0,
        sound_extract_request: &mut sound_extract_request,
        sound_language: None,
        tool_import: &mut tool_import,
        bitmap_reimport: &mut bitmap_reimport,
        shader_ops: &mut shader_ops,
        shader_param_ops: &mut shader_param_ops,
        h2_shader_param_ops: &mut h2_shader_param_ops,
        function_data_ops: &mut function_data_ops,
        model_variant_ops: &mut model_variant_ops,
        color_request: &mut color_request,
        function_request: &mut function_request,
        block_clipboard: None,
        docs: None,
        tsv_paste_request: &mut tsv_paste_request,
        block_clip_request: &mut block_clip_request,
        field_filter: None,
        field_nav: None,
    };
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
    let available = ui.available_width().max(640.0);
    let height = 22.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(available, height), Sense::hover());
    let header_fill = material_section_header();
    ui.painter().rect_filled(rect, 0.0, header_fill);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, material_grid_light()),
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(4.0, 0.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.0),
        material_text_for_bg(header_fill),
    );
}

pub(in crate::app) fn draw_shader_flags_row(
    ui: &mut Ui,
    row: &ShaderFlagsRow,
    edit: &mut FieldEditContext<'_>,
) {
    let available = ui.available_width().max(780.0);
    let label_width = 230.0;
    let default_width = 110.0;
    let value_width = (available - label_width - default_width - 30.0).max(240.0);
    let line_height = 17.0;
    let height = (8.0 + line_height * row.options.len() as f32 + 5.0).max(25.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(available, height), Sense::hover());
    let row_fill = material_data_row();
    let row_text = material_text_for_bg(row_fill);
    ui.painter().rect_filled(rect, 0.0, row_fill);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, material_grid_light()),
    );

    let label_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(4.0, 0.0),
        Vec2::new(label_width, height),
    );
    ui.painter().text(
        label_rect.right_center() - Vec2::new(6.0, 0.0),
        Align2::RIGHT_CENTER,
        truncate_for_cell(&row.label, label_width - 12.0),
        FontId::proportional(12.5),
        row_text,
    );

    let default_rect = egui::Rect::from_min_size(
        label_rect.right_top() + Vec2::new(2.0, 2.0),
        Vec2::new(default_width, height - 4.0),
    );
    ui.painter()
        .rect_filled(default_rect, 0.0, material_default_input());
    ui.painter()
        .rect_stroke(default_rect, 0.0, Stroke::new(1.0, material_input_edge()));

    let value_rect = egui::Rect::from_min_size(
        default_rect.right_top() + Vec2::new(6.0, 0.0),
        Vec2::new(value_width, height - 4.0),
    );
    ui.painter().rect_filled(value_rect, 0.0, material_input());
    ui.painter()
        .rect_stroke(value_rect, 0.0, Stroke::new(1.0, material_input_edge()));

    let enabled = edit.editable && !row.path.is_empty();
    for (index, option) in row.options.iter().enumerate() {
        let row_rect = egui::Rect::from_min_size(
            value_rect.left_top() + Vec2::new(8.0, 4.0 + index as f32 * line_height),
            Vec2::new(value_rect.width() - 16.0, line_height),
        );
        let checkbox_rect =
            egui::Rect::from_min_size(row_rect.left_top() + Vec2::new(0.0, 2.0), Vec2::splat(13.0));
        let id = ui.make_persistent_id((
            edit.view_scope,
            edit.tag_key,
            &row.path,
            "shader_flag",
            option.bit,
        ));
        let response = ui
            .interact(
                row_rect,
                id,
                if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            )
            .on_hover_text(option.label);
        if response.hovered() {
            ui.painter().rect_filled(row_rect, 0.0, material_hover());
        }

        let is_set = row.raw & (1u64 << option.bit) != 0;
        ui.painter().rect_filled(
            checkbox_rect,
            0.0,
            if enabled {
                material_input()
            } else {
                material_checkbox_disabled()
            },
        );
        ui.painter()
            .rect_stroke(checkbox_rect, 0.0, Stroke::new(1.0, material_input_edge()));
        if is_set {
            let stroke = Stroke::new(1.6, material_text());
            ui.painter().line_segment(
                [
                    checkbox_rect.left_center() + Vec2::new(3.0, 0.0),
                    checkbox_rect.center() + Vec2::new(-1.0, 3.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    checkbox_rect.center() + Vec2::new(-1.0, 3.0),
                    checkbox_rect.right_center() + Vec2::new(-2.0, -4.0),
                ],
                stroke,
            );
        }
        ui.painter().text(
            row_rect.left_center() + Vec2::new(20.0, 0.0),
            Align2::LEFT_CENTER,
            option.label,
            FontId::proportional(12.0),
            row_text,
        );

        if response.clicked() {
            let mut next_mask = row.raw;
            if is_set {
                next_mask &= !(1u64 << option.bit);
            } else {
                next_mask |= 1u64 << option.bit;
            }
            edit.pending.push(PendingFieldEdit {
                path: row.path.clone(),
                input: next_mask.to_string(),
            });
        }
    }
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
    let (fill, text_color) = match cell.map(|cell| cell.value_kind) {
        Some("default") | None => {
            let fill = material_default_box();
            (fill, material_text_for_bg(fill))
        }
        _ => {
            let fill = material_input();
            (fill, material_text_for_bg(fill))
        }
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, material_input_edge()));

    let Some(cell) = cell else {
        return;
    };

    let text_left = rect.left_center() + Vec2::new(5.0, 0.0);
    if let Some(color) = cell.color.as_ref() {
        let swatch_size = (rect.height() - 5.0).max(12.0);
        let swatch_rect = egui::Rect::from_min_size(
            rect.right_top() - Vec2::new(swatch_size + 4.0, -2.5),
            Vec2::splat(swatch_size),
        );
        draw_shader_color_swatch(ui, swatch_rect, color.color32());
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

    ui.painter().text(
        text_left,
        Align2::LEFT_CENTER,
        truncate_for_cell(&cell.text, rect.width() - 12.0),
        FontId::monospace(12.0),
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
