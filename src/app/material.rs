//! Material-specific field presentation, color editing, and palette handling.
//! It owns material-specific presentation and color workflows; generic field editing and document persistence belong elsewhere.

use super::*;

mod color_picker;
pub(super) use color_picker::*;

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

pub(super) fn format_rgb_hex(red: f32, green: f32, blue: f32) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        float_channel_to_u8(red),
        float_channel_to_u8(green),
        float_channel_to_u8(blue)
    )
}

pub(super) fn parse_rgb_hex(input: &str) -> Result<[u8; 3], String> {
    let hex = input.trim().strip_prefix('#').unwrap_or(input.trim());
    if hex.len() != 6 {
        return Err("Use #RRGGBB or RRGGBB".to_owned());
    }
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Hex colour contains invalid characters".to_owned());
    }
    Ok([
        u8::from_str_radix(&hex[0..2], 16)
            .map_err(|_| "Hex colour contains invalid red value".to_owned())?,
        u8::from_str_radix(&hex[2..4], 16)
            .map_err(|_| "Hex colour contains invalid green value".to_owned())?,
        u8::from_str_radix(&hex[4..6], 16)
            .map_err(|_| "Hex colour contains invalid blue value".to_owned())?,
    ])
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
