//! Classic Halo 2 shader and template model construction.
//! It owns shader-specific models, edits, and presentation helpers; generic field editing and controller orchestration belong elsewhere.

use super::*;

pub(in crate::app) fn build_h2ek_shader_editor_model(
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
) -> Option<ShaderEditorModel> {
    if tag.classic_engine()? != blam_tags::classic::ClassicEngine::Halo2V4 {
        return None;
    }
    if !is_h2ek_shader_family_group(entry.group_tag) {
        return None;
    }

    let root = tag.root();
    let template_tag = h2_load_shader_template(source, root);
    let template_root = template_tag.as_ref().map(|template| template.root());
    let mut sections = Vec::new();
    h2_push_section(
        &mut sections,
        "STANDARD_PARAMETERS",
        h2_standard_parameter_rows(root, names),
    );
    h2_push_section(
        &mut sections,
        &h2_template_parameter_section_title(root, template_root, names),
        h2_compact_parameter_rows(root, template_root, names),
    );
    h2_push_section(&mut sections, "RAW PARAMETERS", h2_raw_parameter_rows(root));

    if sections.is_empty() {
        return None;
    }

    Some(ShaderEditorModel {
        has_material_row: false,
        global_material_type: String::new(),
        global_material_edit_path: String::new(),
        definition_path: String::new(),
        shader_template_path: None,
        categories: Vec::new(),
        sections,
        atmosphere_flags: ShaderFlagsRow {
            label: String::new(),
            path: String::new(),
            raw: 0,
            options: Vec::new(),
        },
        custom_fog_setting_index: empty_shader_grid_row(),
        sort_layer: empty_shader_grid_row(),
    })
}

fn h2_load_shader_template(source: Option<&TagSource>, root: TagStruct<'_>) -> Option<TagFile> {
    let source = source?;
    let reference = h2_shader_template_reference(root)?;
    load_referenced_tag_from_source(source, &reference, "shader_template", b"stem").ok()
}

fn h2_shader_template_reference(root: TagStruct<'_>) -> Option<String> {
    let value = root.field("template")?.value()?;
    let TagFieldData::TagReference(reference) = value else {
        return None;
    };
    let (group_tag, path) = reference.group_tag_and_name.as_ref()?;
    if *group_tag != u32::from_be_bytes(*b"stem") || path.is_empty() {
        return None;
    }
    Some(h2_normalize_shader_template_reference(path))
}

pub(super) fn h2_normalize_shader_template_reference(path: &str) -> String {
    let mut normalized = path.trim_end_matches('\0').to_owned();
    let lower = normalized.to_ascii_lowercase();
    for suffix in [".shader_template", ".stem"] {
        if lower.ends_with(suffix) {
            normalized.truncate(normalized.len() - suffix.len());
            break;
        }
    }
    normalized
}

fn h2_push_section(sections: &mut Vec<ShaderEditorSection>, title: &str, rows: Vec<ShaderGridRow>) {
    if rows.is_empty() {
        return;
    }
    sections.push(ShaderEditorSection {
        title: title.to_owned(),
        option_name: String::new(),
        rows,
    });
}

fn h2_standard_parameter_rows(root: TagStruct<'_>, names: &TagNameIndex) -> Vec<ShaderGridRow> {
    let mut rows = Vec::new();
    for field_name in [
        "template",
        "material name",
        "flags",
        "Added depth bias offset",
        "Added depth bias slope scale",
        "specular type",
        "lightmap type",
        "lightmap specular brightness",
        "lightmap ambient bias",
        "shader LOD bias",
    ] {
        h2_push_direct_field_row(root, field_name, "", names, &mut rows);
    }
    if let Some(runtime) = root
        .field("runtime properties")
        .and_then(|field| field.as_block())
    {
        if let Some(element) = runtime.element(0) {
            for field in element.fields() {
                let path = format!(
                    "runtime properties[0]/{}",
                    escape_field_path_segment(field.name())
                );
                if let Some(row) = h2_shader_row_from_field(element, field, &path, "", names) {
                    rows.push(row);
                }
            }
        }
    }
    rows
}

fn h2_push_direct_field_row(
    tag_struct: TagStruct<'_>,
    field_name: &str,
    path_prefix: &str,
    names: &TagNameIndex,
    rows: &mut Vec<ShaderGridRow>,
) {
    let Some(field) = tag_struct.field(field_name) else {
        return;
    };
    let path = if path_prefix.is_empty() {
        escape_field_path_segment(field_name)
    } else {
        append_field_path(path_prefix, &escape_field_path_segment(field_name))
    };
    if let Some(mut row) = h2_shader_row_from_field(tag_struct, field, &path, "", names) {
        if path_prefix.is_empty() {
            row.label = h2_standard_field_label(field_name).to_owned();
            h2_apply_standard_field_widget(field_name, &mut row);
        }
        rows.push(row);
    }
}

fn h2_apply_standard_field_widget(field_name: &str, row: &mut ShaderGridRow) {
    let Some(edit) = row.edit.as_mut() else {
        return;
    };
    match field_name {
        "flags" => {
            edit.kind = ShaderRowEditKind::Flags(vec![
                "water".to_owned(),
                "sort first".to_owned(),
                "no active camo".to_owned(),
            ]);
            row.value_cell.text = edit.current.clone();
            row.parameter_type = Some("flags".to_owned());
        }
        "specular type" => {
            edit.kind = ShaderRowEditKind::Enum(vec![
                "none".to_owned(),
                "default shiny".to_owned(),
                "dull".to_owned(),
            ]);
            row.value_cell.text =
                h2_enum_display_value(&edit.current, &["none", "default shiny", "dull"]);
        }
        "lightmap type" => {
            edit.kind = ShaderRowEditKind::Enum(vec![
                "diffuse".to_owned(),
                "default specular".to_owned(),
                "dull specular".to_owned(),
                "shiny specular".to_owned(),
            ]);
            row.value_cell.text = h2_enum_display_value(
                &edit.current,
                &[
                    "diffuse",
                    "default specular",
                    "dull specular",
                    "shiny specular",
                ],
            );
        }
        "shader LOD bias" => {
            edit.kind = ShaderRowEditKind::Enum(vec![
                "none".to_owned(),
                "4x size".to_owned(),
                "2x size".to_owned(),
                "1/2 size".to_owned(),
                "1/4 size".to_owned(),
                "never".to_owned(),
                "cinematic".to_owned(),
                "lowest".to_owned(),
            ]);
            row.value_cell.text = h2_enum_display_value(
                &edit.current,
                &[
                    "none",
                    "4x size",
                    "2x size",
                    "1/2 size",
                    "1/4 size",
                    "never",
                    "cinematic",
                    "lowest",
                ],
            );
        }
        _ => {}
    }
}

fn h2_enum_display_value(current: &str, options: &[&str]) -> String {
    current
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|index| options.get(index).copied())
        .unwrap_or(current)
        .to_owned()
}

fn h2_standard_field_label(field_name: &str) -> &str {
    match field_name {
        "material name" => "material_name",
        "Added depth bias offset" => "depth_bias_offset",
        "Added depth bias slope scale" => "depth_bias_slope_scale",
        "specular type" => "dynamic_light_specular_type",
        "lightmap type" => "lightmap_type",
        "lightmap specular brightness" => "lightmap_specular_brightness",
        "lightmap ambient bias" => "lightmap_ambient_bias",
        "shader LOD bias" => "shader_lod_bias",
        other => other,
    }
}

fn h2_template_parameter_section_title(
    root: TagStruct<'_>,
    template: Option<TagStruct<'_>>,
    names: &TagNameIndex,
) -> String {
    let category = template
        .and_then(|template| template.field("categories"))
        .and_then(|field| field.as_block())
        .and_then(|block| block.element(0))
        .and_then(|category| category.read_string_id("name"))
        .filter(|name| !name.is_empty());
    if let Some(category) = category {
        return category.replace('_', " ").to_ascii_uppercase();
    }
    let Some(value) = root.field("template").and_then(|field| field.value()) else {
        return "PARAMETERS".to_owned();
    };
    let formatted = trim_formatted_value(&format_value(names, &value, false));
    let normalized = formatted.replace('\\', "/").to_ascii_lowercase();
    let Some(pos) = normalized.find("shader_templates/") else {
        return "PARAMETERS".to_owned();
    };
    let rest = &normalized[pos + "shader_templates/".len()..];
    let Some((folder, _)) = rest.split_once('/') else {
        return "PARAMETERS".to_owned();
    };
    if folder.is_empty() {
        "PARAMETERS".to_owned()
    } else {
        folder.replace('_', " ").to_ascii_uppercase()
    }
}

fn h2_compact_parameter_rows(
    root: TagStruct<'_>,
    template: Option<TagStruct<'_>>,
    names: &TagNameIndex,
) -> Vec<ShaderGridRow> {
    if let Some(template) = template {
        let rows = h2_template_parameter_rows(root, template, names);
        if !rows.is_empty() {
            return rows;
        }
    }
    let mut rows = Vec::new();
    let Some(block) = root.field("parameters").and_then(|field| field.as_block()) else {
        return rows;
    };
    for (index, element) in block.iter().enumerate() {
        if let Some(row) = h2_compact_parameter_row(element, index, names) {
            rows.push(row);
        }
        if let Some(animated) = element
            .field("animation properties")
            .and_then(|field| field.as_block())
        {
            for (anim_index, animation) in animated.iter().enumerate() {
                let path = format!("parameters[{index}]/animation properties[{anim_index}]");
                if let Some(row) = h2_animation_parameter_row(element, animation, &path) {
                    rows.push(row);
                }
            }
        }
    }
    rows
}

fn h2_template_parameter_rows(
    shader_root: TagStruct<'_>,
    template_root: TagStruct<'_>,
    names: &TagNameIndex,
) -> Vec<ShaderGridRow> {
    let mut rows = Vec::new();
    let instances = h2_shader_parameter_instances(shader_root);
    let postprocess = H2PostprocessBindings::from_root(shader_root);
    let Some(categories) = template_root
        .field("categories")
        .and_then(|field| field.as_block())
    else {
        return rows;
    };
    let mut template_index = 0usize;
    for category in categories.iter() {
        let Some(parameters) = category
            .field("parameters")
            .and_then(|field| field.as_block())
        else {
            continue;
        };
        for template_param in parameters.iter() {
            let name = h2_template_parameter_name(template_param);
            if name.is_empty() {
                continue;
            }
            let instance = instances.iter().find(|instance| instance.name == name);
            rows.extend(h2_template_parameter_display_rows(
                template_param,
                instance,
                &postprocess,
                template_index,
                names,
            ));
            template_index += 1;
        }
    }
    rows
}

struct H2ParameterInstance<'a> {
    index: usize,
    name: String,
    element: TagStruct<'a>,
}

struct H2LiveElement<'a> {
    index: usize,
    path: String,
    element: TagStruct<'a>,
}

impl H2LiveElement<'_> {
    fn path(&self, field: &str) -> String {
        append_field_path(&self.path, &escape_field_path_segment(field))
    }
}

struct H2PostprocessBindings<'a> {
    values: Vec<H2LiveElement<'a>>,
    colors: Vec<H2LiveElement<'a>>,
    bitmap_transforms: Vec<H2LiveElement<'a>>,
    value_overlays: Vec<H2LiveElement<'a>>,
    color_overlays: Vec<H2LiveElement<'a>>,
    bitmap_transform_overlays: Vec<H2LiveElement<'a>>,
    overlays: Vec<H2LiveElement<'a>>,
    overlay_references: Vec<H2LiveElement<'a>>,
    animated_parameters: Vec<H2LiveElement<'a>>,
    animated_parameter_references: Vec<H2LiveElement<'a>>,
}

impl<'a> H2PostprocessBindings<'a> {
    fn from_root(root: TagStruct<'a>) -> Self {
        let empty = Self {
            values: Vec::new(),
            colors: Vec::new(),
            bitmap_transforms: Vec::new(),
            value_overlays: Vec::new(),
            color_overlays: Vec::new(),
            bitmap_transform_overlays: Vec::new(),
            overlays: Vec::new(),
            overlay_references: Vec::new(),
            animated_parameters: Vec::new(),
            animated_parameter_references: Vec::new(),
        };
        let Some(postprocess) = root
            .field("postprocess definition")
            .and_then(|field| field.as_block())
            .and_then(|block| block.element(0))
        else {
            return empty;
        };
        let base = "postprocess definition[0]";
        let values = h2_collect_postprocess_elements(postprocess, base, "values");
        let colors = h2_collect_postprocess_elements(postprocess, base, "colors");
        Self {
            values: if values.is_empty() {
                h2_collect_postprocess_elements(postprocess, base, "value properties")
            } else {
                values
            },
            colors: if colors.is_empty() {
                h2_collect_postprocess_elements(postprocess, base, "color properties")
            } else {
                colors
            },
            bitmap_transforms: h2_collect_postprocess_elements(
                postprocess,
                base,
                "bitmap transforms",
            ),
            value_overlays: h2_collect_postprocess_elements(postprocess, base, "value overlays"),
            color_overlays: h2_collect_postprocess_elements(postprocess, base, "color overlays"),
            bitmap_transform_overlays: h2_collect_postprocess_elements(
                postprocess,
                base,
                "bitmap transform overlays",
            ),
            overlays: h2_collect_postprocess_elements(postprocess, base, "overlays"),
            overlay_references: h2_collect_postprocess_elements(
                postprocess,
                base,
                "overlay references",
            ),
            animated_parameters: h2_collect_postprocess_elements(
                postprocess,
                base,
                "animated parameters",
            ),
            animated_parameter_references: h2_collect_postprocess_elements(
                postprocess,
                base,
                "animated parameter references",
            ),
        }
    }

    fn value(&self, parameter_index: usize) -> Option<&H2LiveElement<'a>> {
        h2_find_postprocess_by_parameter(&self.values, parameter_index)
    }

    fn color(&self, parameter_index: usize) -> Option<&H2LiveElement<'a>> {
        h2_find_postprocess_by_parameter(&self.colors, parameter_index)
    }

    fn bitmap_transform(
        &self,
        parameter_index: usize,
        animation_type: i32,
    ) -> Option<&H2LiveElement<'a>> {
        h2_find_postprocess_transform(&self.bitmap_transforms, parameter_index, animation_type)
    }

    fn function(&self, parameter_index: usize, animation_type: i32) -> Option<FunctionView> {
        let legacy = match animation_type {
            11 => h2_find_postprocess_by_parameter(&self.value_overlays, parameter_index),
            12 => h2_find_postprocess_by_parameter(&self.color_overlays, parameter_index),
            _ => h2_find_postprocess_transform(
                &self.bitmap_transform_overlays,
                parameter_index,
                animation_type,
            ),
        };
        if let Some(live) = legacy {
            let function_struct = h2_named_struct_field(live.element, "function")?;
            let function_path = live.path("function");
            return classic_halo2_function_view_from_struct(
                live.element,
                function_struct,
                &function_path,
                "function",
            );
        }
        let live = self.new_layout_overlay(parameter_index, animation_type)?;
        let function_struct = h2_named_struct_field(live.element, "function")?;
        let function_path = live.path("function");
        classic_halo2_function_view_from_struct(
            live.element,
            function_struct,
            &function_path,
            "function",
        )
    }

    fn new_layout_overlay(
        &self,
        parameter_index: usize,
        animation_type: i32,
    ) -> Option<&H2LiveElement<'a>> {
        let animated_index = self
            .animated_parameter_references
            .iter()
            .position(|reference| {
                h2_read_usize(reference.element, "parameter index") == Some(parameter_index)
            })?;
        let animated = self.animated_parameters.get(animated_index)?;
        let overlay_reference_index = animated
            .element
            .field("overlay references")
            .and_then(|field| field.as_struct())
            .and_then(|overlay_refs| h2_read_usize(overlay_refs, "block index data"))?;
        let overlay_reference = self.overlay_references.get(overlay_reference_index)?;
        let transform_index = h2_read_i32(overlay_reference.element, "transform index");
        if animation_type != 11
            && animation_type != 12
            && !transform_index.is_some_and(|index| {
                h2_bitmap_transform_index_aliases(animation_type).contains(&index)
            })
        {
            return None;
        }
        let overlay_index = h2_read_usize(overlay_reference.element, "overlay index")?;
        self.overlays.get(overlay_index)
    }
}

fn h2_collect_postprocess_elements<'a>(
    postprocess: TagStruct<'a>,
    base_path: &str,
    block_name: &str,
) -> Vec<H2LiveElement<'a>> {
    let Some(block) = postprocess
        .field(block_name)
        .and_then(|field| field.as_block())
    else {
        return Vec::new();
    };
    let escaped = escape_field_path_segment(block_name);
    block
        .iter()
        .enumerate()
        .map(|(index, element)| H2LiveElement {
            index,
            path: format!("{base_path}/{escaped}[{index}]"),
            element,
        })
        .collect()
}

fn h2_find_postprocess_by_parameter<'a, 'b>(
    elements: &'b [H2LiveElement<'a>],
    parameter_index: usize,
) -> Option<&'b H2LiveElement<'a>> {
    elements.iter().find(|element| {
        h2_read_usize(element.element, "parameter index")
            .map(|index| index == parameter_index)
            .unwrap_or(element.index == parameter_index)
    })
}

fn h2_find_postprocess_transform<'a, 'b>(
    elements: &'b [H2LiveElement<'a>],
    parameter_index: usize,
    animation_type: i32,
) -> Option<&'b H2LiveElement<'a>> {
    elements.iter().find(|element| {
        if h2_read_usize(element.element, "parameter index") != Some(parameter_index) {
            return false;
        }
        let transform_index = h2_read_i32(element.element, "bitmap transform index")
            .or_else(|| h2_read_i32(element.element, "transform index"));
        let overlay_type = h2_read_i32(element.element, "animation property type");
        overlay_type == Some(animation_type)
            || transform_index.is_some_and(|index| {
                h2_bitmap_transform_index_aliases(animation_type).contains(&index)
            })
    })
}

fn h2_bitmap_transform_index_aliases(animation_type: i32) -> &'static [i32] {
    match animation_type {
        0 => &[0],
        1 => &[0, 1],
        2 => &[1, 2],
        3 => &[2, 3],
        4 => &[1, 2, 4],
        5 => &[2, 3, 5],
        6 => &[3, 6],
        7 => &[4, 7],
        13 => &[5, 13],
        _ => &[],
    }
}

fn h2_read_i32(element: TagStruct<'_>, field: &str) -> Option<i32> {
    element
        .read_int_any(field)
        .and_then(|value| i32::try_from(value).ok())
}

fn h2_read_usize(element: TagStruct<'_>, field: &str) -> Option<usize> {
    element
        .read_int_any(field)
        .and_then(|value| usize::try_from(value).ok())
}

fn h2_named_struct_field<'a>(element: TagStruct<'a>, name: &str) -> Option<TagStruct<'a>> {
    element
        .fields()
        .find(|field| field.name() == name && field.field_type() == TagFieldType::Struct)
        .and_then(|field| field.as_struct())
}

fn h2_shader_parameter_instances(root: TagStruct<'_>) -> Vec<H2ParameterInstance<'_>> {
    let Some(block) = root.field("parameters").and_then(|field| field.as_block()) else {
        return Vec::new();
    };
    block
        .iter()
        .enumerate()
        .map(|(index, element)| H2ParameterInstance {
            index,
            name: h2_parameter_name(element, index),
            element,
        })
        .collect()
}

fn h2_template_parameter_display_rows(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
    names: &TagNameIndex,
) -> Vec<ShaderGridRow> {
    let mut rows = Vec::new();
    if let Some(row) =
        h2_template_base_parameter_row(template_param, instance, postprocess, template_index, names)
    {
        rows.push(row);
    }
    if h2_template_parameter_type_index(template_param) == 0 {
        rows.extend(h2_template_bitmap_animation_rows(
            template_param,
            instance,
            postprocess,
            template_index,
        ));
    } else if h2_template_flags(template_param) & 1 != 0 {
        rows.push(h2_template_value_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
        ));
    }
    rows
}

fn h2_template_base_parameter_row(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
    names: &TagNameIndex,
) -> Option<ShaderGridRow> {
    let label = h2_template_parameter_name(template_param);
    let parameter_type = h2_template_parameter_type_index(template_param);
    let (field_name, default_field, parameter_type_label, fill) = match parameter_type {
        0 => ("bitmap", "default bitmap", "bitmap", material_ref_row()),
        2 => (
            "const color",
            "default const color",
            "color",
            material_numeric_row(),
        ),
        1 | 3 => (
            "const value",
            "default const value",
            "value",
            material_numeric_row(),
        ),
        _ => (
            "const value",
            "default const value",
            "value",
            material_numeric_row(),
        ),
    };
    if parameter_type == 0 && h2_template_flags(template_param) & 2 != 0 {
        return None;
    }
    let default_cell =
        h2_template_default_cell(template_param, default_field, names).or_else(|| {
            Some(ShaderGridCell {
                text: h2_parameter_type_label(parameter_type).to_owned(),
                value_kind: "default",
                color: None,
            })
        });
    if parameter_type == 2 {
        if let Some(function) = instance.and_then(|instance| {
            h2_find_animation_by_type(instance.element, 12).and_then(|(anim_index, anim)| {
                let path = format!(
                    "parameters[{}]/animation properties[{anim_index}]",
                    instance.index
                );
                let function_struct = anim.field("function")?.as_struct()?;
                let function_path = append_field_path(&path, "function");
                h2_function_view_from_animation_property(anim, function_struct, &function_path)
            })
        }) {
            let mut row = h2_function_template_row(label, function, template_param, 12);
            row.default_cell = default_cell;
            row.parameter_type = Some(parameter_type_label.to_owned());
            return Some(row);
        }
        if let Some(mut row) =
            h2_legacy_animation_constant_row(&label, instance, template_param, 12)
        {
            row.default_cell = default_cell;
            row.parameter_type = Some(parameter_type_label.to_owned());
            return Some(row);
        }
    }
    let postprocess_value = match parameter_type {
        1 | 3 => postprocess.value(template_index).and_then(|live| {
            live.element
                .field("value")
                .and_then(|field| field.value())
                .map(|value| (live.path("value"), value))
        }),
        2 => postprocess.color(template_index).and_then(|live| {
            live.element
                .field("color")
                .and_then(|field| field.value())
                .map(|value| (live.path("color"), value))
        }),
        _ => None,
    };
    let parameter_value = instance.and_then(|instance| {
        instance
            .element
            .field(field_name)
            .and_then(|field| field.value())
            .map(|value| {
                (
                    format!(
                        "parameters[{index}]/{}",
                        escape_field_path_segment(field_name),
                        index = instance.index
                    ),
                    value,
                )
            })
    });
    let live_value = postprocess_value.or(parameter_value);
    let (value_text, value_kind, color, edit) = if let Some((path, value)) = live_value {
        let formatted = format_value(names, &value, false);
        let color = color_popup_for_value(&label, &value, &formatted);
        (
            if color.is_some() {
                "color: RGB".to_owned()
            } else {
                formatted.clone()
            },
            "value",
            color,
            classic_shader_row_edit(&path, &value, &formatted),
        )
    } else {
        let fallback = h2_template_default_text(template_param, default_field, names)
            .unwrap_or_else(|| String::new());
        let current = if parameter_type == 2 && fallback.is_empty() {
            "0,0,0,1".to_owned()
        } else {
            fallback.clone()
        };
        let color =
            (parameter_type == 2).then(|| MaterialColorPopup::new(&label, 0.0, 0.0, 0.0, 1.0));
        let kind = if parameter_type == 2 {
            ShaderRowEditKind::H2CreateTemplateColor {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: label.clone(),
                parameter_type_index: parameter_type,
                field: field_name.to_owned(),
            }
        } else {
            ShaderRowEditKind::H2CreateTemplateValue {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: label.clone(),
                parameter_type_index: parameter_type,
                field: field_name.to_owned(),
            }
        };
        (
            fallback,
            "default",
            color,
            Some(ShaderRowEdit {
                path: format!(
                    "parameters/<{}>/{}",
                    label,
                    escape_field_path_segment(field_name)
                ),
                current,
                kind,
            }),
        )
    };
    Some(ShaderGridRow {
        label,
        default_cell,
        value_cell: ShaderGridCell {
            text: value_text,
            value_kind,
            color,
        },
        fill,
        parameter_type: Some(parameter_type_label.to_owned()),
        is_overridden: instance.is_some(),
        function: None,
        edit,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    })
}

fn h2_template_bitmap_animation_rows(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
) -> Vec<ShaderGridRow> {
    let flags = h2_template_bitmap_animation_flags(template_param);
    let is_3d = h2_template_bitmap_type_index(template_param) != 0;
    let mut rows = Vec::new();
    if flags & (1 << 0) != 0 {
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            0,
            "scale",
        ));
    }
    if flags & (1 << 1) != 0 {
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            1,
            "scale_x",
        ));
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            2,
            "scale_y",
        ));
        if is_3d {
            rows.push(h2_template_animation_row(
                template_param,
                instance,
                postprocess,
                template_index,
                3,
                "scale_z",
            ));
        }
    }
    if flags & (1 << 2) != 0 {
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            4,
            "translation_x",
        ));
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            5,
            "translation_y",
        ));
        if is_3d {
            rows.push(h2_template_animation_row(
                template_param,
                instance,
                postprocess,
                template_index,
                6,
                "translation_z",
            ));
        }
    }
    if flags & (1 << 3) != 0 {
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            7,
            "rotation",
        ));
    }
    if flags & (1 << 4) != 0 {
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            13,
            "index",
        ));
    }
    rows
}

fn h2_template_value_animation_row(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
) -> ShaderGridRow {
    let is_color = h2_template_parameter_type_index(template_param) == 2;
    let suffix = if is_color { "tint" } else { "value" };
    h2_template_animation_row(
        template_param,
        instance,
        postprocess,
        template_index,
        if is_color { 12 } else { 11 },
        suffix,
    )
}

fn h2_template_animation_row(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
    animation_type: i32,
    suffix: &str,
) -> ShaderGridRow {
    let base = h2_template_parameter_name(template_param);
    let label = format!("{base}_{suffix}");
    let function = postprocess
        .function(template_index, animation_type)
        .or_else(|| {
            instance.and_then(|instance| {
                h2_find_animation_by_type(instance.element, animation_type).and_then(
                    |(anim_index, anim)| {
                        let path = format!(
                            "parameters[{}]/animation properties[{anim_index}]",
                            instance.index
                        );
                        let function_struct = anim.field("function")?.as_struct()?;
                        let function_path = append_field_path(&path, "function");
                        h2_function_view_from_animation_property(
                            anim,
                            function_struct,
                            &function_path,
                        )
                    },
                )
            })
        });
    let mut row = if let Some(function) = function {
        h2_function_template_row(label, function, template_param, animation_type)
    } else if let Some(row) =
        h2_postprocess_constant_animation_row(&label, postprocess, template_index, animation_type)
    {
        row
    } else if let Some(row) =
        h2_legacy_animation_constant_row(&label, instance, template_param, animation_type)
    {
        row
    } else {
        let initial_function_data =
            h2_template_initial_function_data(template_param, animation_type);
        h2_missing_function_row(
            label,
            h2_template_animation_default_value(template_param, animation_type),
            h2_template_animation_default_color(template_param, animation_type),
            H2ShaderParamOp::EnsureAnimationProperty {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: base,
                parameter_type_index: h2_template_parameter_type_index(template_param),
                animation_type_index: animation_type,
                initial_function_data,
            },
        )
    };
    row.default_cell = Some(ShaderGridCell {
        text: String::new(),
        value_kind: "default",
        color: None,
    });
    row
}

fn h2_find_animation_by_type(
    instance: TagStruct<'_>,
    animation_type: i32,
) -> Option<(usize, TagStruct<'_>)> {
    let block = instance
        .field("animation properties")
        .and_then(|field| field.as_block())?;
    block.iter().enumerate().find(|(_, animation)| {
        animation
            .read_int_any("type")
            .and_then(|value| i32::try_from(value).ok())
            == Some(animation_type)
    })
}

fn h2_function_template_row(
    label: String,
    function: FunctionView,
    template_param: TagStruct<'_>,
    animation_type: i32,
) -> ShaderGridRow {
    if function.function.color_graph_type() != ColorGraphType::Scalar {
        if let Some(rgba) = extract_constant_color(&function.function) {
            let block_path = match function.edit.as_ref().map(|edit| &edit.data) {
                Some(FunctionDataStorage::Halo2ByteBlock(path)) => path.clone(),
                _ => String::new(),
            };
            let color = MaterialColorPopup::new(&label, rgba[0], rgba[1], rgba[2], rgba[3]);
            let mut row = ShaderGridRow {
                label,
                default_cell: Some(ShaderGridCell {
                    text: "color: RGB".to_owned(),
                    value_kind: "default",
                    color: h2_template_animation_default_color(template_param, animation_type).map(
                        |rgba| MaterialColorPopup::new("", rgba[0], rgba[1], rgba[2], rgba[3]),
                    ),
                }),
                value_cell: ShaderGridCell {
                    text: "color: RGB".to_owned(),
                    value_kind: "value",
                    color: Some(color),
                },
                fill: material_numeric_row(),
                parameter_type: Some("color".to_owned()),
                is_overridden: true,
                function: None,
                edit: (!block_path.is_empty()).then(|| ShaderRowEdit {
                    path: block_path.clone(),
                    current: h2_color_edit_current(rgba),
                    kind: ShaderRowEditKind::H2FunctionColor {
                        block_path,
                        legacy_data: None,
                    },
                }),
                context_menu: None,
                create_anim_op: None,
                constant_function_view: None,
            };
            row.constant_function_view = Some(function);
            return row;
        }
        let mut row = shader_function_grid_row(label, function);
        row.default_cell =
            h2_template_animation_default_color(template_param, animation_type).map(|rgba| {
                ShaderGridCell {
                    text: "color: RGB".to_owned(),
                    value_kind: "default",
                    color: Some(MaterialColorPopup::new(
                        "", rgba[0], rgba[1], rgba[2], rgba[3],
                    )),
                }
            });
        return row;
    }

    if let Some(value) = function.function.as_constant() {
        let block_path = match function.edit.as_ref().map(|edit| &edit.data) {
            Some(FunctionDataStorage::Halo2ByteBlock(path)) => path.clone(),
            _ => String::new(),
        };
        let current = format_shader_float(value);
        let mut row = ShaderGridRow {
            label,
            default_cell: Some(ShaderGridCell {
                text: String::new(),
                value_kind: "default",
                color: None,
            }),
            value_cell: shader_value_cell(format!("value: {current}")),
            fill: material_numeric_row(),
            parameter_type: Some("animated scalar".to_owned()),
            is_overridden: true,
            function: None,
            edit: (!block_path.is_empty()).then(|| ShaderRowEdit {
                path: block_path.clone(),
                current,
                kind: ShaderRowEditKind::H2FunctionScalar {
                    block_path,
                    legacy_data: None,
                },
            }),
            context_menu: None,
            create_anim_op: None,
            constant_function_view: None,
        };
        row.constant_function_view = Some(function);
        return row;
    }
    let mut row = shader_function_grid_row(label, function);
    row.default_cell = Some(ShaderGridCell {
        text: format!(
            "value: {}",
            format_shader_float(h2_template_animation_default_value(
                template_param,
                animation_type
            ))
        ),
        value_kind: "default",
        color: None,
    });
    row
}

fn h2_legacy_animation_constant_row(
    label: &str,
    instance: Option<&H2ParameterInstance<'_>>,
    template_param: TagStruct<'_>,
    animation_type: i32,
) -> Option<ShaderGridRow> {
    let instance = instance?;
    let (anim_index, anim) = h2_find_animation_by_type(instance.element, animation_type)?;
    let path = format!(
        "parameters[{}]/animation properties[{anim_index}]",
        instance.index
    );
    let function_struct = anim.field("function")?.as_struct()?;
    let function_path = append_field_path(&path, "function");
    let block_path = h2_function_data_path(function_struct, &function_path)?;
    let bytes = halo2_function_bytes_from_struct(function_struct)?;
    if is_h2_legacy_nonconstant_function_data(&bytes) {
        return Some(h2_legacy_function_placeholder_row(
            label,
            h2_legacy_function_view(anim, function_struct, &function_path),
        ));
    }

    if animation_type == 12 {
        let rgba = h2_legacy_constant_color(&bytes)?;
        let color = MaterialColorPopup::new(label, rgba[0], rgba[1], rgba[2], rgba[3]);
        let synthetic = h2_synthetic_function_view_for_constant_color(rgba, anim, animation_type);
        return Some(ShaderGridRow {
            label: label.to_owned(),
            default_cell: Some(ShaderGridCell {
                text: "color: RGB".to_owned(),
                value_kind: "default",
                color: h2_template_animation_default_color(template_param, animation_type)
                    .map(|rgba| MaterialColorPopup::new("", rgba[0], rgba[1], rgba[2], rgba[3])),
            }),
            value_cell: ShaderGridCell {
                text: "color: RGB".to_owned(),
                value_kind: "value",
                color: Some(color),
            },
            fill: material_numeric_row(),
            parameter_type: Some("color".to_owned()),
            is_overridden: true,
            function: None,
            edit: Some(ShaderRowEdit {
                path: block_path.clone(),
                current: h2_color_edit_current(rgba),
                kind: ShaderRowEditKind::H2FunctionColor {
                    block_path,
                    legacy_data: Some(bytes),
                },
            }),
            context_menu: None,
            create_anim_op: None,
            constant_function_view: synthetic,
        });
    }

    let value = h2_legacy_constant_scalar(&bytes)?;
    let current = format_shader_float(value);
    let synthetic = h2_synthetic_function_view_for_constant_scalar(value, anim, animation_type);
    Some(ShaderGridRow {
        label: label.to_owned(),
        default_cell: Some(ShaderGridCell {
            text: String::new(),
            value_kind: "default",
            color: None,
        }),
        value_cell: shader_value_cell(format!("value: {current}")),
        fill: material_numeric_row(),
        parameter_type: Some("animated scalar".to_owned()),
        is_overridden: true,
        function: None,
        edit: Some(ShaderRowEdit {
            path: block_path.clone(),
            current,
            kind: ShaderRowEditKind::H2FunctionScalar {
                block_path,
                legacy_data: Some(bytes),
            },
        }),
        context_menu: None,
        create_anim_op: None,
        constant_function_view: synthetic,
    })
}

fn h2_legacy_function_placeholder_row(
    label: &str,
    function: Option<FunctionView>,
) -> ShaderGridRow {
    ShaderGridRow {
        label: label.to_owned(),
        default_cell: Some(ShaderGridCell {
            text: String::new(),
            value_kind: "default",
            color: None,
        }),
        value_cell: ShaderGridCell {
            text: "<function data goes here>".to_owned(),
            value_kind: "value",
            color: None,
        },
        fill: material_function_row(),
        parameter_type: Some("function".to_owned()),
        is_overridden: true,
        function: None,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: function,
    }
}

fn h2_legacy_function_view(
    animation_property: TagStruct<'_>,
    function_struct: TagStruct<'_>,
    function_path: &str,
) -> Option<FunctionView> {
    let data_block_path = h2_function_data_path(function_struct, function_path)?;
    let bytes = halo2_function_bytes_from_struct(function_struct)?;
    let h2_legacy = H2LegacyFunctionView::parse(bytes.clone());
    let function =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| TagFunction::parse(&bytes)))
            .ok()
            .and_then(Result::ok)
            .or_else(|| {
                decode_hex(&constant_function_hex(0.0))
                    .ok()
                    .and_then(|data| TagFunction::parse(&data).ok())
            })?;
    let mut view = if let Some(h2_legacy) = h2_legacy {
        FunctionView::from_function(function).with_h2_legacy(h2_legacy)
    } else {
        FunctionView::from_function(function).with_h2_scalar_ui()
    };
    view.input_name = animation_property
        .read_string_id("input name")
        .unwrap_or_default();
    view.range_name = animation_property
        .read_string_id("range name")
        .unwrap_or_default();
    if view.h2_legacy.is_none() {
        view.output_index = animation_property
            .read_int_any("type")
            .and_then(|value| i32::try_from(value).ok());
    }
    view.time_period_in_seconds = animation_property
        .read_real("time period")
        .or_else(|| animation_property.read_real("time period in seconds"))
        .unwrap_or_default();

    let animation_path = function_path
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or("");
    let sibling_path = |name: &str| {
        if animation_path.is_empty() {
            escape_field_path_segment(name)
        } else {
            append_field_path(animation_path, &escape_field_path_segment(name))
        }
    };
    let time_field = if animation_property.field("time period").is_some() {
        "time period"
    } else if animation_property.field("time period in seconds").is_some() {
        "time period in seconds"
    } else {
        ""
    };

    Some(
        view.with_edit(FunctionEditPaths {
            data: FunctionDataStorage::Halo2ByteBlock(data_block_path),
            parameter_type: animation_property
                .field("type")
                .and_then(|field| field.value())
                .is_some()
                .then(|| sibling_path("type"))
                .unwrap_or_default(),
            input_name: animation_property
                .field("input name")
                .and_then(|field| field.value())
                .is_some()
                .then(|| sibling_path("input name"))
                .unwrap_or_default(),
            range_name: animation_property
                .field("range name")
                .and_then(|field| field.value())
                .is_some()
                .then(|| sibling_path("range name"))
                .unwrap_or_default(),
            time_period: (!time_field.is_empty()
                && animation_property
                    .field(time_field)
                    .and_then(|field| field.value())
                    .is_some())
            .then(|| sibling_path(time_field))
            .unwrap_or_default(),
            block_path: animation_path.to_owned(),
            block_index: animation_path
                .rsplit_once('[')
                .and_then(|(_, rest)| rest.strip_suffix(']'))
                .and_then(|index| index.parse::<usize>().ok())
                .unwrap_or(0),
        }),
    )
}

fn h2_synthetic_function_view_for_constant_scalar(
    value: f32,
    animation_property: TagStruct<'_>,
    animation_type: i32,
) -> Option<FunctionView> {
    let data = decode_hex(&constant_function_hex(value)).ok()?;
    let function = TagFunction::parse(&data).ok()?;
    Some(h2_readonly_function_view(
        function,
        animation_property,
        animation_type,
    ))
}

fn h2_synthetic_function_view_for_constant_color(
    rgba: [f32; 4],
    animation_property: TagStruct<'_>,
    animation_type: i32,
) -> Option<FunctionView> {
    let data = decode_hex(&constant_color_function_hex(
        rgba[0], rgba[1], rgba[2], rgba[3],
    ))
    .ok()?;
    let function = TagFunction::parse(&data).ok()?;
    Some(h2_readonly_function_view(
        function,
        animation_property,
        animation_type,
    ))
}

fn h2_readonly_function_view(
    function: TagFunction,
    animation_property: TagStruct<'_>,
    animation_type: i32,
) -> FunctionView {
    let mut view = FunctionView::from_function(function).with_h2_scalar_ui();
    view.input_name = animation_property
        .read_string_id("input name")
        .unwrap_or_default();
    view.range_name = animation_property
        .read_string_id("range name")
        .unwrap_or_default();
    view.output_index = Some(animation_type);
    view.time_period_in_seconds = animation_property
        .read_real("time period")
        .or_else(|| animation_property.read_real("time period in seconds"))
        .unwrap_or_default();
    view
}

fn h2_postprocess_constant_animation_row(
    label: &str,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
    animation_type: i32,
) -> Option<ShaderGridRow> {
    let (live, field_name, parameter_type, fill) = match animation_type {
        11 => (
            postprocess.value(template_index)?,
            "value",
            "value",
            material_numeric_row(),
        ),
        12 => (
            postprocess.color(template_index)?,
            "color",
            "color",
            material_numeric_row(),
        ),
        _ => (
            postprocess
                .bitmap_transform(template_index, animation_type)
                .or_else(|| {
                    (animation_type == 0)
                        .then(|| postprocess.value(template_index))
                        .flatten()
                })?,
            "value",
            "value",
            material_numeric_row(),
        ),
    };
    let field = live.element.field(field_name)?;
    let value = field.value()?;
    let formatted = format_value(&TagNameIndex::default(), &value, false);
    let color = color_popup_for_value(label, &value, &formatted);
    let path = live.path(field_name);
    let edit = classic_shader_row_edit(&path, &value, &formatted);
    Some(ShaderGridRow {
        label: label.to_owned(),
        default_cell: None,
        value_cell: ShaderGridCell {
            text: if color.is_some() {
                "color: RGB".to_owned()
            } else {
                format!("value: {}", trim_formatted_value(&formatted))
            },
            value_kind: "value",
            color,
        },
        fill,
        parameter_type: Some(parameter_type.to_owned()),
        is_overridden: false,
        function: None,
        edit,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    })
}

fn h2_missing_function_row(
    label: String,
    default_value: f32,
    default_color: Option<[f32; 4]>,
    op: H2ShaderParamOp,
) -> ShaderGridRow {
    let edit_path = format!(
        "h2-create-function:{}:{}",
        label,
        format_shader_float(default_value)
    );
    if let Some(rgba) = default_color {
        return ShaderGridRow {
            label,
            default_cell: None,
            value_cell: ShaderGridCell {
                text: "color: RGB".to_owned(),
                value_kind: "default",
                color: Some(MaterialColorPopup::new(
                    "", rgba[0], rgba[1], rgba[2], rgba[3],
                )),
            },
            fill: material_numeric_row(),
            parameter_type: Some("function".to_owned()),
            is_overridden: false,
            function: None,
            edit: Some(ShaderRowEdit {
                path: edit_path,
                current: h2_color_edit_current(rgba),
                kind: ShaderRowEditKind::H2CreateFunctionColor {
                    create_op: op.clone(),
                },
            }),
            context_menu: None,
            create_anim_op: Some(ShaderContextAction::H2ParameterOp(op)),
            constant_function_view: None,
        };
    }
    ShaderGridRow {
        label,
        default_cell: None,
        value_cell: ShaderGridCell {
            text: format!("value: {}", format_shader_float(default_value)),
            value_kind: "default",
            color: None,
        },
        fill: material_numeric_row(),
        parameter_type: Some("function".to_owned()),
        is_overridden: false,
        function: None,
        edit: Some(ShaderRowEdit {
            path: edit_path,
            current: format_shader_float(default_value),
            kind: ShaderRowEditKind::H2CreateFunctionScalar {
                create_op: op.clone(),
            },
        }),
        context_menu: None,
        create_anim_op: Some(ShaderContextAction::H2ParameterOp(op)),
        constant_function_view: None,
    }
}

fn h2_template_animation_default_value(template_param: TagStruct<'_>, animation_type: i32) -> f32 {
    match animation_type {
        0 | 1 | 2 | 3 => template_param.read_real("bitmap scale").unwrap_or(1.0),
        11 => template_param
            .read_real("default const value")
            .unwrap_or_default(),
        _ => 0.0,
    }
}

fn h2_template_animation_default_color(
    template_param: TagStruct<'_>,
    animation_type: i32,
) -> Option<[f32; 4]> {
    (animation_type == 12)
        .then(|| h2_template_default_color(template_param))
        .flatten()
}

fn h2_template_initial_function_data(
    template_param: TagStruct<'_>,
    animation_type: i32,
) -> Vec<u8> {
    if let Some([r, g, b, a]) = h2_template_animation_default_color(template_param, animation_type)
    {
        return decode_hex(&constant_color_function_hex(r, g, b, a))
            .unwrap_or_else(|_| vec![0; 32]);
    }
    decode_hex(&constant_function_hex(h2_template_animation_default_value(
        template_param,
        animation_type,
    )))
    .unwrap_or_else(|_| vec![0; 32])
}

fn h2_template_default_color(template_param: TagStruct<'_>) -> Option<[f32; 4]> {
    let value = template_param.field("default const color")?.value()?;
    color_value_to_rgba(&value)
}

fn color_value_to_rgba(value: &TagFieldData) -> Option<[f32; 4]> {
    match value {
        TagFieldData::RealRgbColor(color) => Some([color.red, color.green, color.blue, 1.0]),
        TagFieldData::RealArgbColor(color) => {
            Some([color.red, color.green, color.blue, color.alpha])
        }
        TagFieldData::RgbColor(color) => {
            let raw = color.0;
            Some([
                byte_to_float(((raw >> 16) & 0xFF) as u8),
                byte_to_float(((raw >> 8) & 0xFF) as u8),
                byte_to_float((raw & 0xFF) as u8),
                1.0,
            ])
        }
        TagFieldData::ArgbColor(color) => {
            let raw = color.0;
            Some([
                byte_to_float(((raw >> 16) & 0xFF) as u8),
                byte_to_float(((raw >> 8) & 0xFF) as u8),
                byte_to_float((raw & 0xFF) as u8),
                byte_to_float(((raw >> 24) & 0xFF) as u8),
            ])
        }
        _ => None,
    }
}

fn h2_color_edit_current(rgba: [f32; 4]) -> String {
    format!("{},{},{},{}", rgba[0], rgba[1], rgba[2], rgba[3])
}

pub(super) fn h2_template_parameter_name(template_param: TagStruct<'_>) -> String {
    template_param.read_string_id("name").unwrap_or_default()
}

fn h2_template_parameter_type_index(template_param: TagStruct<'_>) -> i32 {
    template_param
        .read_int_any("type")
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or_default()
}

fn h2_template_flags(template_param: TagStruct<'_>) -> u32 {
    template_param
        .read_int_any("flags")
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_default()
}

fn h2_template_bitmap_animation_flags(template_param: TagStruct<'_>) -> u32 {
    template_param
        .read_int_any("bitmap animation flags")
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_default()
}

fn h2_template_bitmap_type_index(template_param: TagStruct<'_>) -> i32 {
    template_param
        .read_int_any("bitmap type")
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or_default()
}

fn h2_template_default_cell(
    template_param: TagStruct<'_>,
    field_name: &str,
    names: &TagNameIndex,
) -> Option<ShaderGridCell> {
    let text = h2_template_default_text(template_param, field_name, names)?;
    Some(ShaderGridCell {
        text,
        value_kind: "default",
        color: None,
    })
}

fn h2_template_default_text(
    template_param: TagStruct<'_>,
    field_name: &str,
    names: &TagNameIndex,
) -> Option<String> {
    let value = template_param.field(field_name)?.value()?;
    Some(trim_formatted_value(&format_value(names, &value, false)))
}

fn h2_compact_parameter_row(
    element: TagStruct<'_>,
    index: usize,
    names: &TagNameIndex,
) -> Option<ShaderGridRow> {
    let label = h2_parameter_name(element, index);
    let parameter_type = h2_parameter_type_index(element);
    let (field_name, parameter_type_label, fill) = match parameter_type {
        0 => ("bitmap", "bitmap", material_ref_row()),
        2 => (
            "const color",
            "color",
            material_row_tint(&element.field("const color")?.value()?),
        ),
        1 | 3 => ("const value", "value", material_numeric_row()),
        _ => ("const value", "value", material_numeric_row()),
    };
    let field = element.field(field_name)?;
    let path = format!(
        "parameters[{index}]/{}",
        escape_field_path_segment(field_name)
    );
    let value = field.value()?;
    let formatted = format_value(names, &value, false);
    let color = color_popup_for_value(&label, &value, &formatted);
    let edit = classic_shader_row_edit(&path, &value, &formatted);
    Some(ShaderGridRow {
        label,
        default_cell: Some(ShaderGridCell {
            text: h2_parameter_type_label(parameter_type).to_owned(),
            value_kind: "default",
            color: None,
        }),
        value_cell: ShaderGridCell {
            text: if color.is_some() {
                "color: RGB".to_owned()
            } else {
                formatted
            },
            value_kind: "value",
            color,
        },
        fill,
        parameter_type: Some(parameter_type_label.to_owned()),
        is_overridden: true,
        function: None,
        edit,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    })
}

fn h2_animation_parameter_row(
    parameter: TagStruct<'_>,
    animation: TagStruct<'_>,
    animation_path: &str,
) -> Option<ShaderGridRow> {
    let function_struct = animation.field("function")?.as_struct()?;
    let function_path = append_field_path(animation_path, "function");
    let view =
        h2_function_view_from_animation_property(animation, function_struct, &function_path)?;
    let label = h2_animation_row_label(parameter, animation);
    let mut row = shader_function_grid_row(label, view);
    row.default_cell = Some(ShaderGridCell {
        text: h2_animation_type_label(animation).to_owned(),
        value_kind: "default",
        color: None,
    });
    Some(row)
}

fn h2_raw_parameter_rows(root: TagStruct<'_>) -> Vec<ShaderGridRow> {
    let count = root
        .field("parameters")
        .and_then(|field| field.as_block())
        .map(|block| block.len())
        .unwrap_or_default();
    vec![ShaderGridRow {
        label: "parameters".to_owned(),
        default_cell: None,
        value_cell: ShaderGridCell {
            text: count.to_string(),
            value_kind: "value",
            color: None,
        },
        fill: material_data_row(),
        parameter_type: Some("count".to_owned()),
        is_overridden: false,
        function: None,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }]
}

fn h2_parameter_name(element: TagStruct<'_>, index: usize) -> String {
    element
        .read_string_id("name")
        .or_else(|| element.read_string_id("parameter name"))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("parameter_{index}"))
}

fn h2_parameter_type_index(element: TagStruct<'_>) -> i32 {
    element
        .read_int_any("type")
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or_default()
}

fn h2_parameter_type_label(index: i32) -> &'static str {
    match index {
        0 => "bitmap",
        1 => "value",
        2 => "color",
        3 => "switch",
        _ => "value",
    }
}

fn h2_animation_type_label(animation: TagStruct<'_>) -> &'static str {
    match animation
        .read_int_any("type")
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or_default()
    {
        0 => "scale",
        1 => "scale x",
        2 => "scale y",
        3 => "scale z",
        4 => "translation x",
        5 => "translation y",
        6 => "translation z",
        7 => "rotation angle",
        8 => "rotation axis x",
        9 => "rotation axis y",
        10 => "rotation axis z",
        11 => "value",
        12 => "color",
        13 => "bitmap index",
        _ => "function",
    }
}

fn h2_animation_row_label(parameter: TagStruct<'_>, animation: TagStruct<'_>) -> String {
    let base = h2_parameter_name(parameter, 0);
    let suffix = h2_animation_type_label(animation).replace(' ', "_");
    if suffix == "value" || suffix == "color" {
        base
    } else {
        format!("{base}_{suffix}")
    }
}

fn h2_shader_row_from_field(
    parent: TagStruct<'_>,
    field: TagField<'_>,
    path: &str,
    label_prefix: &str,
    names: &TagNameIndex,
) -> Option<ShaderGridRow> {
    if let Some(function) = field.as_function() {
        return Some(shader_function_grid_row(
            h2_nested_label(label_prefix, field.name()),
            FunctionView::from_function(function),
        ));
    }
    if field.name() == "function" {
        if let Some(nested) = field.as_struct() {
            if let Some(function) = h2_function_view_from_animation_property(parent, nested, path) {
                return Some(shader_function_grid_row(
                    h2_nested_label(label_prefix, "animation function"),
                    function,
                ));
            }
        }
    }

    let value = field.value()?;
    if matches!(
        value,
        TagFieldData::Data(_) | TagFieldData::ApiInterop(_) | TagFieldData::Custom(_)
    ) {
        return None;
    }
    let label = h2_nested_label(label_prefix, field.name());
    let formatted = format_value(names, &value, false);
    let color = color_popup_for_value(&label, &value, &formatted);
    let edit = classic_shader_row_edit(path, &value, &formatted).or_else(|| {
        (field.name() == "flags").then(|| ShaderRowEdit {
            path: path.to_owned(),
            current: parent
                .read_int_any(field.name())
                .unwrap_or_default()
                .to_string(),
            kind: ShaderRowEditKind::Flags(vec![
                "water".to_owned(),
                "sort first".to_owned(),
                "no active camo".to_owned(),
            ]),
        })
    });
    let value_kind = if is_none_like_value(&formatted) {
        "default"
    } else {
        "value"
    };
    Some(ShaderGridRow {
        label,
        default_cell: None,
        value_cell: ShaderGridCell {
            text: if color.is_some() {
                "color: RGB".to_owned()
            } else {
                formatted
            },
            value_kind,
            color,
        },
        fill: material_row_tint(&value),
        parameter_type: Some(classic_shader_value_kind(&value).to_owned()),
        is_overridden: false,
        function: None,
        edit,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    })
}

fn h2_function_view_from_animation_property(
    animation_property: TagStruct<'_>,
    function_struct: TagStruct<'_>,
    function_path: &str,
) -> Option<FunctionView> {
    let data_block_path = h2_function_data_path(function_struct, function_path)?;
    let bytes = halo2_function_bytes_from_struct(function_struct)?;
    if is_h2_legacy_function_data(&bytes) {
        return None;
    }
    let function = TagFunction::parse(&bytes).ok()?;
    let mut view = FunctionView::from_function(function);
    view.input_name = animation_property
        .read_string_id("input name")
        .unwrap_or_default();
    view.range_name = animation_property
        .read_string_id("range name")
        .unwrap_or_default();
    view.output_index = animation_property
        .read_int_any("type")
        .and_then(|value| i32::try_from(value).ok());
    view.time_period_in_seconds = animation_property
        .read_real("time period")
        .or_else(|| animation_property.read_real("time period in seconds"))
        .unwrap_or_default();

    let animation_path = function_path
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or("");
    let sibling_path = |name: &str| {
        if animation_path.is_empty() {
            escape_field_path_segment(name)
        } else {
            append_field_path(animation_path, &escape_field_path_segment(name))
        }
    };
    let time_field = if animation_property.field("time period").is_some() {
        "time period"
    } else if animation_property.field("time period in seconds").is_some() {
        "time period in seconds"
    } else {
        ""
    };
    Some(
        view.with_h2_scalar_ui().with_edit(FunctionEditPaths {
            data: FunctionDataStorage::Halo2ByteBlock(data_block_path),
            parameter_type: animation_property
                .field("type")
                .and_then(|field| field.value())
                .is_some()
                .then(|| sibling_path("type"))
                .unwrap_or_default(),
            input_name: animation_property
                .field("input name")
                .and_then(|field| field.value())
                .is_some()
                .then(|| sibling_path("input name"))
                .unwrap_or_default(),
            range_name: animation_property
                .field("range name")
                .and_then(|field| field.value())
                .is_some()
                .then(|| sibling_path("range name"))
                .unwrap_or_default(),
            time_period: (!time_field.is_empty()
                && animation_property
                    .field(time_field)
                    .and_then(|field| field.value())
                    .is_some())
            .then(|| sibling_path(time_field))
            .unwrap_or_default(),
            block_path: animation_path.to_owned(),
            block_index: animation_path
                .rsplit_once('[')
                .and_then(|(_, rest)| rest.strip_suffix(']'))
                .and_then(|index| index.parse::<usize>().ok())
                .unwrap_or_default(),
        }),
    )
}

fn h2_function_data_path(function_struct: TagStruct<'_>, function_path: &str) -> Option<String> {
    function_struct.field("data")?.as_block()?;
    Some(append_field_path(function_path, "data"))
}

fn h2_nested_label(prefix: &str, name: &str) -> String {
    classic_nested_label(prefix, name)
}

fn classic_halo2_function_view_from_struct(
    parent: TagStruct<'_>,
    tag_struct: TagStruct<'_>,
    path: &str,
    _field_name: &str,
) -> Option<FunctionView> {
    let (data_block_path, bytes) = if let Some(bytes) = halo2_function_bytes_from_struct(tag_struct)
    {
        (append_field_path(path, "data"), bytes)
    } else {
        let inner = h2_named_struct_field(tag_struct, "function")?;
        (
            append_field_path(path, "function/data"),
            halo2_function_bytes_from_struct(inner)?,
        )
    };
    let function = TagFunction::parse(&bytes).ok()?;
    let mut view = FunctionView::from_function(function);
    view.input_name = parent.read_string_id("input name").unwrap_or_default();
    view.range_name = parent.read_string_id("range name").unwrap_or_default();
    view.time_period_in_seconds = parent
        .read_real("time period")
        .or_else(|| parent.read_real("time period in seconds"))
        .unwrap_or_default();

    let parent_path = path.rsplit_once('/').map(|(base, _)| base).unwrap_or("");
    let sibling_path = |name: &str| {
        if parent_path.is_empty() {
            escape_field_path_segment(name)
        } else {
            append_field_path(parent_path, &escape_field_path_segment(name))
        }
    };
    let time_field = if parent.field("time period").is_some() {
        "time period"
    } else if parent.field("time period in seconds").is_some() {
        "time period in seconds"
    } else {
        ""
    };
    let input_editable = parent
        .field("input name")
        .and_then(|field| field.value())
        .is_some();
    let range_editable = parent
        .field("range name")
        .and_then(|field| field.value())
        .is_some();
    let time_editable = !time_field.is_empty()
        && parent
            .field(time_field)
            .and_then(|field| field.value())
            .is_some();

    Some(
        view.with_h2_scalar_ui().with_edit(FunctionEditPaths {
            data: FunctionDataStorage::Halo2ByteBlock(data_block_path),
            parameter_type: String::new(),
            input_name: input_editable
                .then(|| sibling_path("input name"))
                .unwrap_or_default(),
            range_name: range_editable
                .then(|| sibling_path("range name"))
                .unwrap_or_default(),
            time_period: time_editable
                .then(|| sibling_path(time_field))
                .unwrap_or_default(),
            block_path: String::new(),
            block_index: 0,
        }),
    )
}

pub(in crate::app) fn halo2_function_bytes_from_struct(
    tag_struct: TagStruct<'_>,
) -> Option<Vec<u8>> {
    let block = tag_struct.field("data")?.as_block()?;
    let mut bytes = Vec::with_capacity(block.len());
    for element in block.iter() {
        let value = element.read_int_any("Value")?;
        bytes.push(value as i8 as u8);
    }
    Some(bytes)
}

#[cfg(test)]
pub(in crate::app) fn first_halo2_byte_block_function_row(
    model: &ShaderEditorModel,
) -> Option<(Vec<u8>, String)> {
    for row in model
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
    {
        if let Some(view) = row.function.as_ref() {
            if let Some(edit) = view.edit.as_ref() {
                if let FunctionDataStorage::Halo2ByteBlock(path) = &edit.data {
                    return Some((view.function.to_bytes(), path.clone()));
                }
            }
        }
    }
    None
}

#[cfg(test)]
pub(in crate::app) fn shader_row_edit_path_and_kind(
    model: &ShaderEditorModel,
    label: &str,
) -> Option<(String, &'static str)> {
    let edit = model
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .find(|row| row.label == label)?
        .edit
        .as_ref()?;
    let kind = shader_row_edit_kind_name(&edit.kind);
    Some((edit.path.clone(), kind))
}

#[cfg(test)]
pub(in crate::app) fn shader_row_value_text_for_test(
    model: &ShaderEditorModel,
    label: &str,
) -> Option<String> {
    model
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .find(|row| row.label == label)
        .map(|row| row.value_cell.text.clone())
}

#[cfg(test)]
pub(in crate::app) fn h2_function_data_range_for_test(data: &[u8]) -> (bool, Option<f32>) {
    (
        h2_function_range_enabled(data),
        h2_function_range_value(data),
    )
}

#[cfg(test)]
pub(in crate::app) fn h2_function_data_with_range_for_test(
    data: &[u8],
    enabled: bool,
    value: Option<f32>,
) -> Vec<u8> {
    h2_function_data_with_range(data, enabled, value)
}

#[cfg(test)]
fn shader_row_edit_kind_name(kind: &ShaderRowEditKind) -> &'static str {
    match kind {
        ShaderRowEditKind::Scalar => "scalar",
        ShaderRowEditKind::Int => "int",
        ShaderRowEditKind::StringId => "string_id",
        ShaderRowEditKind::BitmapRef { .. } => "bitmap_ref",
        ShaderRowEditKind::ShaderTemplateRef => "shader_template_ref",
        ShaderRowEditKind::Bool { .. } => "bool",
        ShaderRowEditKind::Enum(_) => "enum",
        ShaderRowEditKind::Flags(_) => "flags",
        ShaderRowEditKind::FunctionScalar { .. } => "function_scalar",
        ShaderRowEditKind::FunctionColor { .. } => "function_color",
        ShaderRowEditKind::ColorField { .. } => "color",
        ShaderRowEditKind::CreateFunctionColor { .. } => "create_function_color",
        ShaderRowEditKind::CreateFunctionScalar { .. } => "create_function_scalar",
        ShaderRowEditKind::H2FunctionScalar { .. } => "h2_function_scalar",
        ShaderRowEditKind::H2CreateFunctionScalar { .. } => "h2_create_function_scalar",
        ShaderRowEditKind::H2FunctionColor { .. } => "h2_function_color",
        ShaderRowEditKind::H2CreateFunctionColor { .. } => "h2_create_function_color",
        ShaderRowEditKind::CreateScalarParam { .. } => "create_scalar_param",
        ShaderRowEditKind::H2CreateTemplateValue { .. } => "h2_create_template_value",
        ShaderRowEditKind::H2CreateTemplateColor { .. } => "h2_create_template_color",
    }
}

#[cfg(test)]
pub(in crate::app) fn h2_template_row_labels_for_test(
    shader: &TagFile,
    template: &TagFile,
) -> Vec<String> {
    h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default())
        .into_iter()
        .map(|row| row.label)
        .collect()
}

#[cfg(test)]
pub(in crate::app) fn h2_template_row_edit_kind_for_test(
    shader: &TagFile,
    template: &TagFile,
    label: &str,
) -> Option<&'static str> {
    let rows = h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default());
    let edit = rows.iter().find(|row| row.label == label)?.edit.as_ref()?;
    Some(shader_row_edit_kind_name(&edit.kind))
}

#[cfg(test)]
pub(in crate::app) fn h2_template_row_value_text_for_test(
    shader: &TagFile,
    template: &TagFile,
    label: &str,
) -> Option<String> {
    let rows = h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default());
    rows.into_iter()
        .find(|row| row.label == label)
        .map(|row| row.value_cell.text)
}

#[cfg(test)]
pub(in crate::app) fn h2_template_row_value_color_for_test(
    shader: &TagFile,
    template: &TagFile,
    label: &str,
) -> Option<(u8, u8, u8, u8)> {
    let rows = h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default());
    rows.into_iter()
        .find(|row| row.label == label)
        .and_then(|row| row.value_cell.color)
        .map(|color| {
            let color = color.color32();
            (color.r(), color.g(), color.b(), color.a())
        })
}

#[cfg(test)]
pub(in crate::app) fn h2_template_row_function_data_path_for_test(
    shader: &TagFile,
    template: &TagFile,
    label: &str,
) -> Option<String> {
    let rows = h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default());
    let row = rows.into_iter().find(|row| row.label == label)?;
    let view = row
        .function
        .as_ref()
        .or(row.constant_function_view.as_ref())?;
    let edit = view.edit.as_ref()?;
    let FunctionDataStorage::Halo2ByteBlock(path) = &edit.data else {
        return None;
    };
    Some(path.clone())
}

#[cfg(test)]
pub(in crate::app) fn h2_shader_template_reference_for_test(tag: &TagFile) -> Option<String> {
    h2_shader_template_reference(tag.root())
}

#[cfg(test)]
pub(in crate::app) struct H2FunctionEditSummary {
    pub(in crate::app) bytes: Vec<u8>,
    pub(in crate::app) output_index: Option<i32>,
    pub(in crate::app) input_name: String,
    pub(in crate::app) range_name: String,
    pub(in crate::app) time_period: f32,
    pub(in crate::app) data_path: String,
    pub(in crate::app) parameter_type_path: String,
    pub(in crate::app) input_name_path: String,
    pub(in crate::app) range_name_path: String,
    pub(in crate::app) time_period_path: String,
}

#[cfg(test)]
pub(in crate::app) fn first_h2_function_edit_summary(
    model: &ShaderEditorModel,
) -> Option<H2FunctionEditSummary> {
    for row in model
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
    {
        let Some(view) = row.function.as_ref() else {
            continue;
        };
        let Some(edit) = view.edit.as_ref() else {
            continue;
        };
        let FunctionDataStorage::Halo2ByteBlock(data_path) = &edit.data else {
            continue;
        };
        return Some(H2FunctionEditSummary {
            bytes: view.function.to_bytes(),
            output_index: view.output_index,
            input_name: view.input_name.clone(),
            range_name: view.range_name.clone(),
            time_period: view.time_period_in_seconds,
            data_path: data_path.clone(),
            parameter_type_path: edit.parameter_type.clone(),
            input_name_path: edit.input_name.clone(),
            range_name_path: edit.range_name.clone(),
            time_period_path: edit.time_period.clone(),
        });
    }
    None
}

fn classic_shader_row_edit(
    path: &str,
    value: &TagFieldData,
    formatted: &str,
) -> Option<ShaderRowEdit> {
    match value {
        TagFieldData::RealRgbColor(color) => Some(ShaderRowEdit {
            path: path.to_owned(),
            current: format!("{},{},{},1", color.red, color.green, color.blue),
            kind: ShaderRowEditKind::ColorField { argb: false },
        }),
        TagFieldData::RealArgbColor(color) => Some(ShaderRowEdit {
            path: path.to_owned(),
            current: format!(
                "{},{},{},{}",
                color.red, color.green, color.blue, color.alpha
            ),
            kind: ShaderRowEditKind::ColorField { argb: true },
        }),
        TagFieldData::RgbColor(color) => {
            let raw = color.0;
            Some(ShaderRowEdit {
                path: path.to_owned(),
                current: format!(
                    "{},{},{},1",
                    byte_to_float(((raw >> 16) & 0xFF) as u8),
                    byte_to_float(((raw >> 8) & 0xFF) as u8),
                    byte_to_float((raw & 0xFF) as u8)
                ),
                kind: ShaderRowEditKind::ColorField { argb: false },
            })
        }
        TagFieldData::ArgbColor(color) => {
            let raw = color.0;
            Some(ShaderRowEdit {
                path: path.to_owned(),
                current: format!(
                    "{},{},{},{}",
                    byte_to_float(((raw >> 16) & 0xFF) as u8),
                    byte_to_float(((raw >> 8) & 0xFF) as u8),
                    byte_to_float((raw & 0xFF) as u8),
                    byte_to_float(((raw >> 24) & 0xFF) as u8)
                ),
                kind: ShaderRowEditKind::ColorField { argb: true },
            })
        }
        TagFieldData::TagReference(reference) => {
            let Some((group_tag, name)) = reference.group_tag_and_name.as_ref() else {
                return Some(ShaderRowEdit {
                    path: path.to_owned(),
                    current: "NONE".to_owned(),
                    kind: ShaderRowEditKind::StringId,
                });
            };
            if *group_tag != u32::from_be_bytes(*b"bitm") {
                if *group_tag == u32::from_be_bytes(*b"stem") {
                    let current = if name.is_empty() {
                        "NONE".to_owned()
                    } else {
                        format!(
                            "{}.shader_template",
                            h2_normalize_shader_template_reference(name).replace('\\', "/")
                        )
                    };
                    return Some(ShaderRowEdit {
                        path: path.to_owned(),
                        current,
                        kind: ShaderRowEditKind::ShaderTemplateRef,
                    });
                }
                return Some(ShaderRowEdit {
                    path: path.to_owned(),
                    current: formatted.to_owned(),
                    kind: ShaderRowEditKind::StringId,
                });
            }
            let current = if name.is_empty() {
                "NONE".to_owned()
            } else {
                format!("{}.bitmap", name.replace('\\', "/"))
            };
            Some(ShaderRowEdit {
                path: path.to_owned(),
                current,
                kind: ShaderRowEditKind::BitmapRef {
                    group_tag: *group_tag,
                    create: None,
                },
            })
        }
        TagFieldData::StringId(value) | TagFieldData::OldStringId(value) => Some(ShaderRowEdit {
            path: path.to_owned(),
            current: value.string.clone(),
            kind: ShaderRowEditKind::StringId,
        }),
        TagFieldData::Real(value)
        | TagFieldData::RealSlider(value)
        | TagFieldData::RealFraction(value)
        | TagFieldData::Angle(value) => Some(ShaderRowEdit {
            path: path.to_owned(),
            current: value.to_string(),
            kind: ShaderRowEditKind::Scalar,
        }),
        TagFieldData::CharInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::ShortInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::LongInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::ByteInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::WordInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::DwordInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::CharEnum { value, .. } => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::ShortEnum { value, .. } => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::LongEnum { value, .. } => classic_int_edit(path, *value as i64, formatted),
        _ => None,
    }
}

fn classic_int_edit(path: &str, value: i64, formatted: &str) -> Option<ShaderRowEdit> {
    let normalized = formatted.trim().to_ascii_lowercase();
    let kind = if matches!(normalized.as_str(), "true" | "false") {
        ShaderRowEditKind::Bool { create: None }
    } else {
        ShaderRowEditKind::Int
    };
    Some(ShaderRowEdit {
        path: path.to_owned(),
        current: value.to_string(),
        kind,
    })
}

fn classic_shader_value_kind(value: &TagFieldData) -> &'static str {
    match value {
        TagFieldData::TagReference(_) => "tag reference",
        TagFieldData::RealRgbColor(_)
        | TagFieldData::RealArgbColor(_)
        | TagFieldData::RgbColor(_)
        | TagFieldData::ArgbColor(_) => "color",
        TagFieldData::Real(_)
        | TagFieldData::RealSlider(_)
        | TagFieldData::RealFraction(_)
        | TagFieldData::Angle(_) => "real",
        TagFieldData::CharEnum { .. }
        | TagFieldData::ShortEnum { .. }
        | TagFieldData::LongEnum { .. } => "enum",
        TagFieldData::ByteFlags { .. }
        | TagFieldData::WordFlags { .. }
        | TagFieldData::LongFlags { .. }
        | TagFieldData::ByteBlockFlags(_)
        | TagFieldData::WordBlockFlags(_)
        | TagFieldData::LongBlockFlags(_) => "flags",
        _ => "value",
    }
}

fn classic_nested_label(prefix: &str, name: &str) -> String {
    let name = clean_field_name(name);
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix} {name}")
    }
}

pub(super) fn empty_shader_grid_row() -> ShaderGridRow {
    ShaderGridRow {
        label: String::new(),
        default_cell: None,
        value_cell: ShaderGridCell {
            text: String::new(),
            value_kind: "value",
            color: None,
        },
        fill: material_data_row(),
        parameter_type: None,
        is_overridden: false,
        function: None,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}
