use super::*;

pub(super) fn draw_material_tag(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    rmdf_cache: &mut HashMap<String, Option<RenderMethodDefinition>>,
    rmop_cache: &mut HashMap<String, Option<RenderMethodOption>>,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
) {
    Frame::none()
        .fill(MATERIAL_PANEL)
        .stroke(Stroke::new(1.0, MATERIAL_PANEL_EDGE))
        .inner_margin(egui::Margin {
            left: 2.0,
            right: 2.0,
            top: 2.0,
            bottom: 2.0,
        })
        .show(ui, |ui| {
            if is_shader_tag(entry) {
                let model =
                    build_h2ek_shader_editor_model(tag, entry, names, source).or_else(|| {
                        build_shader_editor_model(
                            tag,
                            entry.group_tag,
                            source,
                            rmdf_cache,
                            rmop_cache,
                        )
                    });
                if let Some(model) = model {
                    draw_shader_editor_model(ui, &model, color_popup, function_popup, edit);
                    return;
                }
                // Shader grid couldn't be built (rmdf/rmop chain didn't
                // resolve). Fall back to the standard EDITABLE field view so
                // the shader is still fully editable, rather than the
                // read-only material struct view.
                draw_struct_fields(ui, tag.root(), names, 0, expert_mode, "", edit);
                return;
            }
            draw_material_template_summary(ui, tag, names, color_popup);
            ui.add_space(2.0);
            draw_material_struct_fields(
                ui,
                tag.root(),
                names,
                0,
                color_popup,
                function_popup,
                expert_mode,
            );
        });
}

pub(super) fn draw_material_struct_fields(
    ui: &mut Ui,
    tag_struct: TagStruct<'_>,
    names: &TagNameIndex,
    depth: usize,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    expert_mode: bool,
) {
    for field in tag_struct.fields() {
        draw_material_field(
            ui,
            field,
            names,
            depth,
            color_popup,
            function_popup,
            expert_mode,
        );
    }
}

pub(super) fn draw_material_field(
    ui: &mut Ui,
    field: TagField<'_>,
    names: &TagNameIndex,
    depth: usize,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    expert_mode: bool,
) {
    let key = clean_field_key(field.name());
    if is_shader_template_reference_key(&key) {
        return;
    }

    if let Some(function) = field.as_function() {
        draw_material_function_value_row(ui, field.name(), &function, depth, function_popup);
        return;
    }
    if let Some(value) = field.value() {
        if is_hidden_non_expert_value(&value, expert_mode) {
            return;
        }
        let formatted = format_value(names, &value, false);
        let color = color_popup_for_value(field.name(), &value, &formatted);
        draw_material_value_row(
            ui,
            field.name(),
            &formatted,
            material_row_tint(&value),
            material_value_kind(&value),
            depth,
            color,
            color_popup,
        );
        return;
    }

    if let Some(nested) = field.as_struct() {
        egui::CollapsingHeader::new(material_section_text(clean_field_name(field.name())))
            .default_open(depth == 0)
            .show(ui, |ui| {
                draw_material_struct_fields(
                    ui,
                    nested,
                    names,
                    depth + 1,
                    color_popup,
                    function_popup,
                    expert_mode,
                )
            });
    } else if let Some(block) = field.as_block() {
        if block.is_empty() {
            return;
        }
        if is_material_parameters_field(field.name()) {
            draw_material_parameters_block(ui, block, names, depth, color_popup, function_popup);
            return;
        }
        egui::CollapsingHeader::new(material_section_text(format!(
            "{}  [{} elements]",
            clean_field_name(field.name()),
            block.len()
        )))
        .default_open(depth == 0 || is_priority_section(field.name()))
        .show(ui, |ui| {
            for (index, element) in block.iter().enumerate() {
                egui::CollapsingHeader::new(material_section_text(format!(
                    "[{index}] {}",
                    element.name()
                )))
                .default_open(index == 0 && is_priority_section(field.name()))
                .show(ui, |ui| {
                    draw_material_struct_fields(
                        ui,
                        element,
                        names,
                        depth + 1,
                        color_popup,
                        function_popup,
                        expert_mode,
                    )
                });
            }
        });
    } else if let Some(array) = field.as_array() {
        if array.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(material_section_text(format!(
            "{}  [{} elements]",
            clean_field_name(field.name()),
            array.len()
        )))
        .default_open(depth == 0)
        .show(ui, |ui| {
            for (index, element) in array.iter().enumerate() {
                egui::CollapsingHeader::new(material_section_text(format!(
                    "[{index}] {}",
                    element.name()
                )))
                .show(ui, |ui| {
                    draw_material_struct_fields(
                        ui,
                        element,
                        names,
                        depth + 1,
                        color_popup,
                        function_popup,
                        expert_mode,
                    )
                });
            }
        });
    } else if let Some(resource) = field.as_resource() {
        draw_material_resource(
            ui,
            field.name(),
            resource,
            names,
            depth,
            color_popup,
            function_popup,
            expert_mode,
        );
    }
}

pub(super) fn draw_material_resource(
    ui: &mut Ui,
    name: &str,
    resource: TagResource<'_>,
    names: &TagNameIndex,
    depth: usize,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    expert_mode: bool,
) {
    let kind = match resource.kind() {
        TagResourceKind::Null => "null",
        TagResourceKind::Exploded => "exploded",
        TagResourceKind::Xsync => "xsync",
    };
    egui::CollapsingHeader::new(material_section_text(format!(
        "{}  ({kind})",
        clean_field_name(name)
    )))
    .show(ui, |ui| {
        draw_material_value_row(
            ui,
            "inline bytes",
            &hex_bytes(resource.inline_bytes()),
            MATERIAL_DATA_ROW,
            "value",
            depth + 1,
            None,
            color_popup,
        );
        if let Some(payload) = resource.exploded_payload() {
            draw_material_value_row(
                ui,
                "exploded payload",
                &format!("{} bytes", payload.len()),
                MATERIAL_DATA_ROW,
                "value",
                depth + 1,
                None,
                color_popup,
            );
        }
        if let Some(payload) = resource.xsync_payload() {
            draw_material_value_row(
                ui,
                "xsync payload",
                &format!("{} bytes", payload.len()),
                MATERIAL_DATA_ROW,
                "value",
                depth + 1,
                None,
                color_popup,
            );
        }
        if let Some(nested) = resource.as_struct() {
            draw_material_struct_fields(
                ui,
                nested,
                names,
                depth + 1,
                color_popup,
                function_popup,
                expert_mode,
            );
        }
    });
}

pub(super) fn draw_material_value_row(
    ui: &mut Ui,
    name: &str,
    value: &str,
    fill: Color32,
    value_kind: &str,
    depth: usize,
    color: Option<MaterialColorPopup>,
    color_popup: &mut Option<MaterialColorPopup>,
) {
    let available = ui.available_width().max(520.0);
    let label_width = (210.0 - (depth as f32 * 10.0)).clamp(150.0, 210.0);
    let value_width = (available - label_width - 30.0).max(220.0);
    let height = 28.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(available, height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, fill);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, MATERIAL_GRID),
    );

    let label_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(8.0 + depth as f32 * 10.0, 0.0),
        Vec2::new(label_width, height),
    );
    ui.painter().text(
        label_rect.left_center(),
        Align2::LEFT_CENTER,
        clean_field_name(name),
        FontId::proportional(13.0),
        MATERIAL_TEXT,
    );

    let value_rect = egui::Rect::from_min_size(
        label_rect.right_top() + Vec2::new(8.0, 4.0),
        Vec2::new(value_width.min(560.0), height - 8.0),
    );
    let (value_fill, value_text) = if value_kind == "default" {
        (MATERIAL_DEFAULT_BOX, MATERIAL_MUTED_TEXT)
    } else {
        (Color32::WHITE, MATERIAL_TEXT)
    };
    ui.painter().rect_filled(value_rect, 0.0, value_fill);
    ui.painter()
        .rect_stroke(value_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
    let text_offset = if let Some(color) = color {
        let swatch_size = (value_rect.height() - 4.0).max(12.0);
        let swatch_rect = egui::Rect::from_min_size(
            value_rect.left_top() + Vec2::new(4.0, 2.0),
            Vec2::splat(swatch_size),
        );
        ui.painter().rect_filled(swatch_rect, 0.0, color.color32());
        ui.painter()
            .rect_stroke(swatch_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
        let swatch_response = ui
            .interact(
                swatch_rect,
                ui.make_persistent_id(format!("material_color:{name}:{value}")),
                Sense::click(),
            )
            .on_hover_text("Click to show Foundation color values");
        if swatch_response.clicked() {
            *color_popup = Some(color);
        }
        swatch_size + 12.0
    } else {
        0.0
    };

    ui.painter().text(
        value_rect.left_center() + Vec2::new(6.0 + text_offset, 0.0),
        Align2::LEFT_CENTER,
        truncate_for_cell(value, value_rect.width() - text_offset),
        FontId::monospace(12.5),
        value_text,
    );
    if response.hovered() && value.len() > 40 {
        response.on_hover_text(value);
    }
}

pub(super) fn draw_material_function_value_row(
    ui: &mut Ui,
    name: &str,
    function: &TagFunction,
    depth: usize,
    function_popup: &mut Option<FunctionPopup>,
) {
    let available = ui.available_width().max(520.0);
    let label_width = (210.0 - (depth as f32 * 10.0)).clamp(150.0, 210.0);
    let height = 34.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(available, height), Sense::click());
    ui.painter().rect_filled(rect, 0.0, MATERIAL_FUNCTION_ROW);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, MATERIAL_GRID),
    );

    let label_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(8.0 + depth as f32 * 10.0, 0.0),
        Vec2::new(label_width, height),
    );
    ui.painter().text(
        label_rect.left_center(),
        Align2::LEFT_CENTER,
        clean_field_name(name),
        FontId::proportional(13.0),
        MATERIAL_TEXT,
    );

    let function_rect = egui::Rect::from_min_max(
        label_rect.right_top() + Vec2::new(8.0, 5.0),
        rect.right_bottom() - Vec2::new(44.0, 5.0),
    );
    ui.painter().rect_filled(function_rect, 0.0, Color32::WHITE);
    ui.painter()
        .rect_stroke(function_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
    ui.painter().text(
        function_rect.left_center() + Vec2::new(6.0, 0.0),
        Align2::LEFT_CENTER,
        truncate_for_cell(
            &shader_function_grid_text(function),
            function_rect.width() - 12.0,
        ),
        FontId::monospace(12.5),
        MATERIAL_TEXT,
    );

    let button_rect = egui::Rect::from_min_size(
        rect.right_top() + Vec2::new(-36.0, 5.0),
        Vec2::new(30.0, height - 10.0),
    );
    ui.painter().rect_filled(button_rect, 0.0, Color32::WHITE);
    ui.painter()
        .rect_stroke(button_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
    ui.painter().text(
        button_rect.center(),
        Align2::CENTER_CENTER,
        "f()",
        FontId::proportional(12.0),
        MATERIAL_TEXT,
    );

    if response.hovered() {
        response
            .clone()
            .on_hover_text("Click to open function viewer");
    }
    if response.clicked() {
        *function_popup = Some(FunctionPopup::new(
            String::new(),
            clean_field_name(name),
            FunctionView::from_function(function.clone()),
            false,
        ));
    }
}

#[derive(Clone)]
pub(super) struct MaterialColorPopup {
    title: String,
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
    pub(super) sc_hex: String,
    /// When Some, clicking OK writes a constant-color function blob to this path.
    write_path: Option<String>,
    /// When Some, clicking OK writes a plain RGB/ARGB color value to this path
    /// (e.g. a permutation `color lower bound` field), not a function blob.
    write_color_field: Option<ColorFieldWrite>,
    /// When Some, clicking OK creates a constant-color animated parameter.
    create_shader_op: Option<ShaderOp>,
    /// When Some, clicking OK creates a shader parameter with a constant-color
    /// animated child.
    create_shader_param_op: Option<ShaderParamOp>,
    /// Tag key that owns the write_path. Used by draw_color_popup to route the edit.
    tag_key: String,
}

/// Target for writing a picked color back into a plain color-valued field.
#[derive(Clone)]
pub(super) struct ColorFieldWrite {
    path: String,
    /// True for `real_argb_color` (4 channels); false for `real_rgb_color`.
    argb: bool,
}

impl MaterialColorPopup {
    pub(super) fn new(title: &str, red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        let red = red.clamp(0.0, 1.0);
        let green = green.clamp(0.0, 1.0);
        let blue = blue.clamp(0.0, 1.0);
        let alpha = alpha.clamp(0.0, 1.0);
        Self {
            title: clean_field_name(title),
            red,
            green,
            blue,
            alpha,
            sc_hex: format!(
                "sc#{}, {}, {}, {}",
                format_pc_float(alpha),
                format_pc_float(red),
                format_pc_float(green),
                format_pc_float(blue)
            ),
            write_path: None,
            write_color_field: None,
            create_shader_op: None,
            create_shader_param_op: None,
            tag_key: String::new(),
        }
    }

    pub(super) fn with_write(
        mut self,
        tag_key: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        self.tag_key = tag_key.into();
        self.write_path = Some(path.into());
        self
    }

    /// Configure the popup to write a plain color value (RGB or ARGB) back into
    /// the given field path when the user clicks OK.
    pub(super) fn with_color_field(
        mut self,
        tag_key: impl Into<String>,
        path: impl Into<String>,
        argb: bool,
    ) -> Self {
        self.tag_key = tag_key.into();
        self.write_color_field = Some(ColorFieldWrite {
            path: path.into(),
            argb,
        });
        self
    }

    pub(super) fn with_shader_op(mut self, tag_key: impl Into<String>, op: ShaderOp) -> Self {
        self.tag_key = tag_key.into();
        self.create_shader_op = Some(op);
        self
    }

    pub(super) fn with_shader_param_op(
        mut self,
        tag_key: impl Into<String>,
        op: ShaderParamOp,
    ) -> Self {
        self.tag_key = tag_key.into();
        self.create_shader_param_op = Some(op);
        self
    }

    pub(super) fn color32(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(
            float_channel_to_u8(self.red),
            float_channel_to_u8(self.green),
            float_channel_to_u8(self.blue),
            float_channel_to_u8(self.alpha),
        )
    }
}

pub(super) fn color_popup_for_value(
    title: &str,
    value: &TagFieldData,
    formatted: &str,
) -> Option<MaterialColorPopup> {
    match value {
        TagFieldData::RealRgbColor(color) => Some(MaterialColorPopup::new(
            title,
            color.red,
            color.green,
            color.blue,
            1.0,
        )),
        TagFieldData::RealArgbColor(color) => Some(MaterialColorPopup::new(
            title,
            color.red,
            color.green,
            color.blue,
            color.alpha,
        )),
        TagFieldData::RgbColor(color) => {
            let raw = color.0;
            Some(MaterialColorPopup::new(
                title,
                byte_to_float(((raw >> 16) & 0xFF) as u8),
                byte_to_float(((raw >> 8) & 0xFF) as u8),
                byte_to_float((raw & 0xFF) as u8),
                1.0,
            ))
        }
        TagFieldData::ArgbColor(color) => {
            let raw = color.0;
            Some(MaterialColorPopup::new(
                title,
                byte_to_float(((raw >> 16) & 0xFF) as u8),
                byte_to_float(((raw >> 8) & 0xFF) as u8),
                byte_to_float((raw & 0xFF) as u8),
                byte_to_float(((raw >> 24) & 0xFF) as u8),
            ))
        }
        _ if formatted.starts_with("sc#") => parse_sc_color(title, formatted),
        _ => None,
    }
}

pub(super) fn material_parameter_color_title(
    element: TagStruct<'_>,
    names: &TagNameIndex,
    fallback: &str,
) -> String {
    material_parameter_name(element, names).unwrap_or_else(|| clean_field_name(fallback))
}

pub(super) fn parse_sc_color(title: &str, formatted: &str) -> Option<MaterialColorPopup> {
    let values = formatted.strip_prefix("sc#")?;
    let parts = values
        .split(',')
        .map(str::trim)
        .filter_map(|part| part.parse::<f32>().ok())
        .collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    Some(MaterialColorPopup::new(
        title, parts[1], parts[2], parts[3], parts[0],
    ))
}

pub(super) enum ColorPopupResult {
    FieldEdit {
        tag_key: String,
        edit: PendingFieldEdit,
    },
    ShaderOp {
        tag_key: String,
        op: ShaderOp,
    },
    ShaderParamOp {
        tag_key: String,
        op: ShaderParamOp,
    },
}

/// Draw the color inspector / editor popup.
///
/// Returns a write result when the user clicks OK on an editable popup.
pub(super) fn draw_color_popup(
    ctx: &egui::Context,
    color_popup: &mut Option<MaterialColorPopup>,
) -> Option<ColorPopupResult> {
    let color = color_popup.as_mut()?;
    let mut open = true;
    let mut close = false;
    let editable = color.write_path.is_some()
        || color.write_color_field.is_some()
        || color.create_shader_op.is_some()
        || color.create_shader_param_op.is_some();
    let mut result: Option<ColorPopupResult> = None;
    egui::Window::new(color.title.clone())
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .default_size(Vec2::new(448.0, 480.0))
        .show(ctx, |ui| {
            if editable {
                draw_color_picker_editor(ui, color);
            } else {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(80.0), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, color.color32());
                    ui.painter()
                        .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
                    ui.add_space(14.0);
                    draw_color_channel_table(ui, color);
                });
            }
            ui.add_space(10.0);
            let sc_hex = format!(
                "sc#{}, {}, {}, {}",
                format_pc_float(color.alpha),
                format_pc_float(color.red),
                format_pc_float(color.green),
                format_pc_float(color.blue)
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new("PC Hex:").color(text_dark()));
                let response = draw_copy_text(ui, &sc_hex, 225.0);
                if response.clicked() {
                    ui.output_mut(|output| output.copied_text = sc_hex.clone());
                }
            });
            if !editable {
                ui.small(RichText::new("Click PC Hex to copy").color(subtle_dark()));
            }
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("OK").clicked() {
                    if let Some(field) = color.write_color_field.clone() {
                        // Plain color value: emit the channel string the field
                        // parser expects (RGB = "r, g, b", ARGB = "a, r, g, b").
                        let input = if field.argb {
                            format!(
                                "{}, {}, {}, {}",
                                color.alpha, color.red, color.green, color.blue
                            )
                        } else {
                            format!("{}, {}, {}", color.red, color.green, color.blue)
                        };
                        result = Some(ColorPopupResult::FieldEdit {
                            tag_key: color.tag_key.clone(),
                            edit: PendingFieldEdit {
                                path: field.path,
                                input,
                            },
                        });
                    } else if let Some(path) = color.write_path.clone() {
                        let hex = constant_color_function_hex(
                            color.red,
                            color.green,
                            color.blue,
                            color.alpha,
                        );
                        result = Some(ColorPopupResult::FieldEdit {
                            tag_key: color.tag_key.clone(),
                            edit: PendingFieldEdit { path, input: hex },
                        });
                    } else if let Some(mut op) = color.create_shader_op.clone() {
                        op.initial_function_hex = constant_color_function_hex(
                            color.red,
                            color.green,
                            color.blue,
                            color.alpha,
                        );
                        result = Some(ColorPopupResult::ShaderOp {
                            tag_key: color.tag_key.clone(),
                            op,
                        });
                    } else if let Some(mut op) = color.create_shader_param_op.clone() {
                        if let Some(animated) = op.animated_parameters.first_mut() {
                            animated.initial_function_hex = constant_color_function_hex(
                                color.red,
                                color.green,
                                color.blue,
                                color.alpha,
                            );
                        }
                        result = Some(ColorPopupResult::ShaderParamOp {
                            tag_key: color.tag_key.clone(),
                            op,
                        });
                    }
                    close = true;
                }
                if editable && ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    if close || !open {
        *color_popup = None;
    }
    result
}

pub(super) fn draw_color_picker_editor(ui: &mut Ui, color: &mut MaterialColorPopup) {
    ui.horizontal(|ui| {
        draw_color_sv_square(ui, color);
        ui.add_space(8.0);
        draw_color_hue_strip(ui, color);
        ui.add_space(10.0);
        ui.vertical(|ui| {
            draw_color_numeric_editor(ui, color);
            ui.add_space(8.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(84.0, 56.0), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
            ui.painter()
                .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
            ui.painter()
                .rect_filled(rect.shrink(5.0), 0.0, color.color32());
        });
    });
    ui.add_space(8.0);
    draw_palette_grid(ui, color);
}

pub(super) fn draw_color_sv_square(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let size = Vec2::new(248.0, 268.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let (h, s, b) = rgb_to_hsb_255(color.red, color.green, color.blue);
    for y in 0..64 {
        let y0 = rect.top() + rect.height() * y as f32 / 64.0;
        let y1 = rect.top() + rect.height() * (y + 1) as f32 / 64.0;
        let bri = 1.0 - (y as f32 + 0.5) / 64.0;
        for x in 0..64 {
            let x0 = rect.left() + rect.width() * x as f32 / 64.0;
            let x1 = rect.left() + rect.width() * (x + 1) as f32 / 64.0;
            let sat = (x as f32 + 0.5) / 64.0;
            let (r, g, blue) = hsb_to_rgb(h as f32 / 255.0, sat, bri);
            ui.painter().rect_filled(
                egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1)),
                0.0,
                Color32::from_rgb(
                    float_channel_to_u8(r),
                    float_channel_to_u8(g),
                    float_channel_to_u8(blue),
                ),
            );
        }
    }
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
    let cursor = egui::pos2(
        egui::lerp(rect.left()..=rect.right(), s as f32 / 255.0),
        egui::lerp(rect.bottom()..=rect.top(), b as f32 / 255.0),
    );
    ui.painter()
        .circle_stroke(cursor, 5.0, Stroke::new(1.0, Color32::BLACK));
    ui.painter()
        .circle_stroke(cursor, 4.0, Stroke::new(1.0, Color32::WHITE));
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let sat = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let bri = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            let (r, g, b) = hsb_to_rgb(h as f32 / 255.0, sat, bri);
            color.red = r;
            color.green = g;
            color.blue = b;
        }
    }
}

pub(super) fn draw_color_hue_strip(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let (h, s, b) = rgb_to_hsb_255(color.red, color.green, color.blue);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(22.0, 268.0), Sense::click_and_drag());
    for i in 0..128 {
        let t0 = i as f32 / 128.0;
        let t1 = (i + 1) as f32 / 128.0;
        let hue = 1.0 - (i as f32 + 0.5) / 128.0;
        let (r, g, blue) = hsb_to_rgb(hue, 1.0, 1.0);
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), egui::lerp(rect.top()..=rect.bottom(), t0)),
                egui::pos2(rect.right(), egui::lerp(rect.top()..=rect.bottom(), t1)),
            ),
            0.0,
            Color32::from_rgb(
                float_channel_to_u8(r),
                float_channel_to_u8(g),
                float_channel_to_u8(blue),
            ),
        );
    }
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
    let marker_y = egui::lerp(rect.bottom()..=rect.top(), h as f32 / 255.0);
    ui.painter().line_segment(
        [
            egui::pos2(rect.left() - 4.0, marker_y),
            egui::pos2(rect.right() + 4.0, marker_y),
        ],
        Stroke::new(1.0, Color32::BLACK),
    );
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let hue = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            let (r, g, blue) = hsb_to_rgb(hue, s as f32 / 255.0, b as f32 / 255.0);
            color.red = r;
            color.green = g;
            color.blue = blue;
        }
    }
}

pub(super) fn draw_color_numeric_editor(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let (mut h, mut s, mut b) = rgb_to_hsb_255(color.red, color.green, color.blue);
    egui::Grid::new("material_color_picker_values")
        .spacing(Vec2::new(6.0, 4.0))
        .show(ui, |ui| {
            ui.label("");
            ui.label(RichText::new("Xenon").color(text_dark()).small());
            ui.label(RichText::new("PC").color(text_dark()).small());
            ui.end_row();
            let h_pc = h as f32 / 255.0;
            let s_pc = s as f32 / 255.0;
            let b_pc = b as f32 / 255.0;
            let h_changed = draw_color_byte_row(ui, "H:", &mut h, h_pc);
            let s_changed = draw_color_byte_row(ui, "S:", &mut s, s_pc);
            let b_changed = draw_color_byte_row(ui, "B:", &mut b, b_pc);
            if h_changed || s_changed || b_changed {
                let (r, g, blue) = hsb_to_rgb(h as f32 / 255.0, s as f32 / 255.0, b as f32 / 255.0);
                color.red = r;
                color.green = g;
                color.blue = blue;
            }
            let mut r = float_channel_to_u8(color.red);
            let mut g = float_channel_to_u8(color.green);
            let mut blue = float_channel_to_u8(color.blue);
            let mut a = float_channel_to_u8(color.alpha);
            if draw_color_byte_row(ui, "R:", &mut r, color.red) {
                color.red = byte_to_float(r);
            }
            if draw_color_byte_row(ui, "G:", &mut g, color.green) {
                color.green = byte_to_float(g);
            }
            if draw_color_byte_row(ui, "B:", &mut blue, color.blue) {
                color.blue = byte_to_float(blue);
            }
            if draw_color_byte_row(ui, "A:", &mut a, color.alpha) {
                color.alpha = byte_to_float(a);
            }
        });
}

pub(super) fn draw_color_byte_row(ui: &mut Ui, label: &str, value: &mut u8, pc: f32) -> bool {
    ui.label(RichText::new(label).color(text_dark()).strong());
    let mut v = *value as i32;
    let changed = ui
        .add_sized(
            Vec2::new(48.0, 20.0),
            egui::DragValue::new(&mut v).range(0..=255).speed(1.0),
        )
        .changed();
    if changed {
        *value = v.clamp(0, 255) as u8;
    }
    let mut pc_value = pc;
    let pc_changed = ui
        .add_sized(
            Vec2::new(54.0, 20.0),
            egui::DragValue::new(&mut pc_value)
                .range(0.0..=1.0)
                .speed(0.01),
        )
        .changed();
    if pc_changed {
        *value = float_channel_to_u8(pc_value);
    }
    ui.end_row();
    changed || pc_changed
}

pub(super) fn draw_palette_grid(ui: &mut Ui, color: &mut MaterialColorPopup) {
    const PALETTE: &[(u8, u8, u8)] = &[
        (255, 0, 0),
        (0, 255, 0),
        (0, 0, 255),
        (255, 255, 0),
        (0, 255, 255),
        (255, 0, 255),
        (255, 255, 255),
        (224, 224, 224),
        (192, 192, 192),
        (160, 160, 160),
        (128, 128, 128),
        (96, 96, 96),
        (64, 64, 64),
        (32, 32, 32),
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (0, 0, 128),
        (128, 128, 0),
        (0, 128, 128),
        (128, 0, 128),
        (255, 128, 128),
        (128, 255, 128),
        (128, 128, 255),
        (255, 192, 128),
        (255, 128, 0),
        (128, 64, 0),
        (64, 32, 0),
        (255, 220, 180),
        (180, 120, 80),
        (90, 50, 35),
        (60, 32, 24),
        (255, 180, 220),
        (220, 90, 160),
        (140, 50, 120),
        (70, 30, 80),
        (180, 220, 255),
        (90, 160, 220),
        (40, 100, 180),
        (20, 60, 110),
        (210, 255, 180),
        (140, 220, 80),
        (80, 160, 40),
        (40, 90, 24),
        (240, 240, 220),
        (210, 200, 150),
        (160, 145, 90),
        (95, 85, 55),
    ];
    egui::Grid::new("material_color_palette")
        .spacing(Vec2::new(5.0, 5.0))
        .show(ui, |ui| {
            for (i, &(r, g, b)) in PALETTE.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_rgb(r, g, b));
                ui.painter()
                    .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
                if response.clicked() {
                    color.red = byte_to_float(r);
                    color.green = byte_to_float(g);
                    color.blue = byte_to_float(b);
                }
                if (i + 1) % 16 == 0 {
                    ui.end_row();
                }
            }
        });
}

pub(super) fn hsb_to_rgb(h: f32, s: f32, b: f32) -> (f32, f32, f32) {
    let h = (h.fract() * 6.0).clamp(0.0, 5.999);
    let i = h.floor() as i32;
    let f = h - i as f32;
    let p = b * (1.0 - s);
    let q = b * (1.0 - s * f);
    let t = b * (1.0 - s * (1.0 - f));
    match i {
        0 => (b, t, p),
        1 => (q, b, p),
        2 => (p, b, t),
        3 => (p, q, b),
        4 => (t, p, b),
        _ => (b, p, q),
    }
}

pub(super) fn draw_color_channel_table(ui: &mut Ui, color: &MaterialColorPopup) {
    let (hue, saturation, brightness) = rgb_to_hsb_255(color.red, color.green, color.blue);
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.add_space(34.0);
            ui.label(RichText::new("0-255").color(subtle_dark()));
            ui.add_space(23.0);
            ui.label(RichText::new("PC (float)").color(subtle_dark()));
        });
        egui::Grid::new("material_color_channels")
            .spacing(Vec2::new(6.0, 4.0))
            .show(ui, |ui| {
                draw_color_channel_row(ui, "R:", float_channel_to_u8(color.red), color.red);
                draw_color_channel_row(ui, "G:", float_channel_to_u8(color.green), color.green);
                draw_color_channel_row(ui, "B:", float_channel_to_u8(color.blue), color.blue);
                draw_color_channel_row(ui, "A:", float_channel_to_u8(color.alpha), color.alpha);
                draw_hsb_row(ui, "H:", hue);
                draw_hsb_row(ui, "S:", saturation);
                draw_hsb_row(ui, "B:", brightness);
            });
    });
}

pub(super) fn draw_color_channel_row(
    ui: &mut Ui,
    label: &str,
    channel_255: u8,
    channel_float: f32,
) {
    ui.label(RichText::new(label).color(text_dark()).strong());
    draw_copy_text(ui, &channel_255.to_string(), 56.0);
    draw_copy_text(ui, &format_pc_float(channel_float), 72.0);
    ui.end_row();
}

pub(super) fn draw_hsb_row(ui: &mut Ui, label: &str, value: u8) {
    ui.label(RichText::new(label).color(text_dark()).strong());
    draw_copy_text(ui, &value.to_string(), 56.0);
    ui.label("");
    ui.end_row();
}

pub(super) fn draw_copy_text(ui: &mut Ui, value: &str, width: f32) -> egui::Response {
    let height = 22.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let fill = if response.hovered() {
        Color32::from_rgb(246, 246, 244)
    } else {
        Color32::from_rgb(238, 238, 235)
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
    ui.painter().text(
        rect.left_center() + Vec2::new(6.0, 0.0),
        Align2::LEFT_CENTER,
        truncate_for_cell(value, width - 10.0),
        FontId::monospace(12.5),
        MATERIAL_TEXT,
    );
    response
}

pub(super) fn truncate_for_cell(text: &str, width: f32) -> String {
    let max_chars = (width / 7.0).floor().max(8.0) as usize;
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out = text
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

pub(super) fn material_section_text(text: String) -> RichText {
    RichText::new(text).color(MATERIAL_TEXT).strong()
}

pub(super) fn clean_field_name(name: &str) -> String {
    field_display_meta(name).label
}

pub(super) fn clean_field_name_basic(name: &str) -> String {
    name.replace(['*', '!'], "")
        .replace(['#', ':'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn group_folder_label(label: &str, show_prefixes: bool) -> String {
    if show_prefixes {
        format!("[folder] {label}")
    } else {
        label.to_owned()
    }
}

pub(super) fn clean_field_key(name: &str) -> String {
    clean_field_name(name)
        .replace('^', "")
        .trim()
        .to_ascii_lowercase()
}

pub(super) fn clean_type_name(type_name: &str) -> String {
    type_name.replace('_', " ")
}

pub(super) fn is_priority_section(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "material parameters" | "material parameters*"
    )
}

pub(super) fn is_material_parameters_field(name: &str) -> bool {
    matches!(
        clean_field_key(name).as_str(),
        "material parameters" | "parameters"
    )
}

pub(super) fn is_material_parameter_metadata(key: &str) -> bool {
    key.starts_with("parameter name")
        || key.starts_with("parameter type")
        || key.starts_with("parameter index")
        || key.starts_with("display ")
        || key.starts_with("register ")
}

pub(super) fn material_parameter_field_matches_type(key: &str, parameter_type: &str) -> bool {
    if parameter_type.contains("bitmap") {
        return key == "bitmap" || key == "bitmap path";
    }
    if parameter_type.contains("color") {
        return key == "color";
    }
    if parameter_type.contains("real") || parameter_type.contains("scalar") {
        return key == "real";
    }
    if parameter_type.contains("vector") {
        return key == "vector";
    }
    if parameter_type.contains("int") || parameter_type.contains("bool") {
        return key == "int/bool";
    }

    matches!(
        key,
        "bitmap" | "bitmap path" | "color" | "real" | "vector" | "int/bool"
    )
}

pub(super) fn material_parameter_value_priority(key: &str) -> u8 {
    match key {
        "bitmap" => 0,
        "bitmap path" => 1,
        "color" => 2,
        "real" => 3,
        "vector" => 4,
        "int/bool" => 5,
        _ => 9,
    }
}

pub(super) fn should_skip_material_parameter_value(key: &str, value: &str) -> bool {
    if matches!(key, "bitmap" | "bitmap path") {
        return is_none_like_value(value);
    }
    false
}

pub(super) fn is_none_like_value(value: &str) -> bool {
    matches!(value.trim(), "" | "NONE" | "\"NONE\"")
}

pub(super) fn trim_formatted_value(value: &str) -> String {
    value.trim().trim_matches('"').to_owned()
}

pub(super) fn enum_display_name(value: &str) -> Option<String> {
    let start = value.find('(')?;
    let end = value.rfind(')')?;
    if start >= end {
        return None;
    }
    Some(value[start + 1..end].trim().to_owned())
}

pub(super) fn float_channel_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub(super) fn byte_to_float(value: u8) -> f32 {
    value as f32 / 255.0
}

pub(super) fn format_pc_float(value: f32) -> String {
    let mut text = format!("{:.7}", value.clamp(0.0, 1.0));
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

pub(super) fn rgb_to_hsb_255(red: f32, green: f32, blue: f32) -> (u8, u8, u8) {
    let red = red.clamp(0.0, 1.0);
    let green = green.clamp(0.0, 1.0);
    let blue = blue.clamp(0.0, 1.0);
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let delta = max - min;

    let mut hue = if delta == 0.0 {
        0.0
    } else if max == red {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if max == green {
        60.0 * (((blue - red) / delta) + 2.0)
    } else {
        60.0 * (((red - green) / delta) + 4.0)
    };
    if hue < 0.0 {
        hue += 360.0;
    }

    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    (
        ((hue / 360.0) * 255.0).round() as u8,
        float_channel_to_u8(saturation),
        float_channel_to_u8(max),
    )
}

pub(super) fn is_material_tag(entry: &TagEntry) -> bool {
    entry.group_name.as_deref() == Some("material")
        || entry.group_tag == u32::from_be_bytes(*b"mat ")
        || entry.display_path.to_ascii_lowercase().ends_with(".mat")
}

pub(super) fn is_material_shader_tag(entry: &TagEntry) -> bool {
    entry.group_name.as_deref() == Some("material_shader")
        || entry.group_tag == u32::from_be_bytes(*b"mats")
        || entry
            .display_path
            .to_ascii_lowercase()
            .ends_with(".material_shader")
}

pub(super) fn is_shader_tag(entry: &TagEntry) -> bool {
    let group_name = entry.group_name.as_deref().unwrap_or_default();
    if group_name == "render_method" || group_name.starts_with("shader") {
        return true;
    }
    let display_path = entry.display_path.to_ascii_lowercase();
    if display_path.ends_with(".shader") || display_path.contains(".shader_") {
        return true;
    }
    matches!(
        entry.group_tag,
        tag if tag == u32::from_be_bytes(*b"rmsh")
            || tag == u32::from_be_bytes(*b"rmtr")
            || tag == u32::from_be_bytes(*b"rmw ")
            || tag == u32::from_be_bytes(*b"rmfl")
            || tag == u32::from_be_bytes(*b"rmd ")
            || tag == u32::from_be_bytes(*b"rmhg")
            || tag == u32::from_be_bytes(*b"rmsk")
            || tag == u32::from_be_bytes(*b"rmct")
            || tag == u32::from_be_bytes(*b"rmcs")
            || tag == u32::from_be_bytes(*b"rmp ")
            || tag == u32::from_be_bytes(*b"rmb ")
            || tag == u32::from_be_bytes(*b"rmco")
            || tag == u32::from_be_bytes(*b"rmlv")
    )
}

pub(super) fn is_h2ek_shader_family_group(group_tag: u32) -> bool {
    matches!(
        &group_tag.to_be_bytes(),
        b"rmsh"
            | b"shad"
            | b"rmtr"
            | b"rmcs"
            | b"rmhg"
            | b"rmfl"
            | b"rmsk"
            | b"rmct"
            | b"rmp "
            | b"rmb "
            | b"rmd "
            | b"rmw "
    )
}

pub(super) fn material_row_tint(value: &TagFieldData) -> Color32 {
    match value {
        TagFieldData::Data(_) | TagFieldData::ApiInterop(_) | TagFieldData::Custom(_) => {
            MATERIAL_DATA_ROW
        }
        TagFieldData::RealRgbColor(_)
        | TagFieldData::RealArgbColor(_)
        | TagFieldData::RealHsvColor(_)
        | TagFieldData::RealAhsvColor(_)
        | TagFieldData::RgbColor(_)
        | TagFieldData::ArgbColor(_)
        | TagFieldData::Real(_)
        | TagFieldData::RealSlider(_)
        | TagFieldData::RealFraction(_)
        | TagFieldData::Angle(_)
        | TagFieldData::CharInteger(_)
        | TagFieldData::ShortInteger(_)
        | TagFieldData::LongInteger(_)
        | TagFieldData::Int64Integer(_)
        | TagFieldData::ByteInteger(_)
        | TagFieldData::WordInteger(_)
        | TagFieldData::DwordInteger(_)
        | TagFieldData::QwordInteger(_)
        | TagFieldData::Point2d(_)
        | TagFieldData::Rectangle2d(_)
        | TagFieldData::RealPoint2d(_)
        | TagFieldData::RealPoint3d(_)
        | TagFieldData::RealVector2d(_)
        | TagFieldData::RealVector3d(_)
        | TagFieldData::RealQuaternion(_)
        | TagFieldData::RealEulerAngles2d(_)
        | TagFieldData::RealEulerAngles3d(_)
        | TagFieldData::RealPlane2d(_)
        | TagFieldData::RealPlane3d(_)
        | TagFieldData::ShortIntegerBounds(_)
        | TagFieldData::AngleBounds(_)
        | TagFieldData::RealBounds(_)
        | TagFieldData::FractionBounds(_) => MATERIAL_NUMERIC_ROW,
        _ => MATERIAL_REF_ROW,
    }
}

pub(super) fn material_value_kind(value: &TagFieldData) -> &'static str {
    match value {
        TagFieldData::StringId(s) | TagFieldData::OldStringId(s) if s.string.is_empty() => {
            "default"
        }
        TagFieldData::TagReference(r) if r.group_tag_and_name.is_none() => "default",
        _ => "value",
    }
}
