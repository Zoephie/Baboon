use super::*;

pub(super) struct MaterialParameterValue {
    label: String,
    value: String,
    fill: Color32,
    value_kind: &'static str,
    color: Option<MaterialColorPopup>,
    priority: u8,
}

#[derive(Clone)]
pub(super) struct ShaderGridCell {
    text: String,
    value_kind: &'static str,
    color: Option<MaterialColorPopup>,
}

pub(super) struct ShaderGridRow {
    label: String,
    default_cell: Option<ShaderGridCell>,
    value_cell: ShaderGridCell,
    fill: Color32,
    parameter_type: Option<String>,
    function: Option<FunctionView>,
    /// When present, the value cell is rendered as an editable widget that
    /// writes back to this tag field path (instead of a read-only label).
    edit: Option<ShaderRowEdit>,
    /// Right-click context menu for adding optional animated parameters
    /// (bitmap transform sub-rows). Only shown when the tag is editable.
    context_menu: Option<ShaderContextMenu>,
    /// When the row represents a function-backed channel but no animated
    /// parameter exists yet, show an "f()+" button that pushes this
    /// `ShaderOp` to create a constant animated parameter.
    create_anim_op: Option<ShaderContextAction>,
    /// When the row's animated parameter is a *constant* function (displayed
    /// as an editable scalar), this holds the full `FunctionView` (with edit
    /// paths) so the user can open the graph editor via an "f()" button and
    /// optionally switch to curve mode without losing the existing parameter.
    constant_function_view: Option<FunctionView>,
}

/// Items shown in a right-click context menu on a shader grid row.
pub(super) struct ShaderContextMenu {
    items: Vec<ShaderContextItem>,
}

/// One action available in a `ShaderContextMenu`.
pub(super) struct ShaderContextItem {
    label: String,
    action: ShaderContextAction,
}

#[derive(Clone)]
pub(super) enum ShaderContextAction {
    AnimatedParameter(ShaderOp),
    FieldEdits(Vec<PendingFieldEdit>),
    ParameterOp(ShaderParamOp),
}

/// Editable backing for a shader grid row's value cell.
#[derive(Clone)]
pub(super) struct ShaderRowEdit {
    /// Full tag field path (slashes in field names escaped as `\/`).
    path: String,
    /// Clean current value used to seed/sync the in-place editor.
    current: String,
    kind: ShaderRowEditKind,
}

#[derive(Clone)]
pub(super) enum ShaderRowEditKind {
    /// Real number text box.
    Scalar,
    /// Integer text box (also used for bool as 0/1).
    Int,
    /// String-id text box (renders identically to Scalar; parsing is type-driven).
    StringId,
    /// Bitmap tag reference (text + browse + Clear).
    BitmapRef {
        group_tag: u32,
        create: Option<ShaderParamCreateTarget>,
    },
    /// Boolean checkbox backed by an existing field or a new shader parameter.
    Bool {
        create: Option<ShaderParamCreateTarget>,
    },
    /// Index-valued dropdown over the given option labels.
    Enum(Vec<String>),
    /// Animated parameter that is currently a constant function: shows as an
    /// editable float text box. The `ShaderRowEdit.path` is the `function/data`
    /// hex path; `current` is the scalar value as a string. On commit a new
    /// 32-byte Constant function blob is written. The `×` button removes the
    /// animated parameter element from its parent block.
    FunctionScalar {
        block_path: String,
        block_index: usize,
    },
    /// Animated parameter that is a constant 1-color function: shown as a
    /// clickable color swatch that opens an editable color popup. The path is
    /// `function/data`; current is `"r,g,b,a"` floats. On OK a new 32-byte
    /// Constant 1-color blob is written.
    FunctionColor {
        block_path: String,
        block_index: usize,
    },
    /// Plain shader parameter color field (`parameters[n]/color`): shown as a
    /// swatch and written directly instead of creating an animated parameter.
    ColorField { argb: bool },
    /// No Color animated parameter exists yet. The swatch opens the color
    /// popup and OK creates one initialized to the selected constant color.
    CreateFunctionColor { target: ShaderFunctionCreateTarget },
    /// No animated scalar function exists yet. Editing the numeric value creates
    /// one initialized to the entered constant.
    CreateFunctionScalar { target: ShaderFunctionCreateTarget },
    /// No parameter instance exists yet. On commit a new `parameters[]`
    /// element is created via `ShaderParamOp`.
    CreateScalarParam {
        parameters_block_path: String,
        parameter_name: String,
        parameter_type_index: i32,
    },
}

#[derive(Clone)]
pub(super) struct ShaderParamCreateTarget {
    parameters_block_path: String,
    parameter_name: String,
    parameter_type_index: i32,
    field: &'static str,
}

#[derive(Clone)]
pub(super) enum ShaderFunctionCreateTarget {
    ExistingParameter {
        animated_block_path: String,
        output_type_index: i32,
    },
    NewParameter {
        parameters_block_path: String,
        parameter_name: String,
        parameter_type_index: i32,
        output_type_index: i32,
    },
}

pub(super) struct ShaderEditorModel {
    /// True only for the 7 material-bearing shader types (shader/terrain/
    /// custom/halogram/foliage/skin/cortana); gates the MATERIAL section.
    has_material_row: bool,
    global_material_type: String,
    /// Absolute tag field path for editing the `global material type`
    /// string-id. Empty when the field could not be located.
    global_material_edit_path: String,
    definition_path: String,
    shader_template_path: Option<String>,
    categories: Vec<ShaderEditorCategory>,
    sections: Vec<ShaderEditorSection>,
    atmosphere_flags: ShaderFlagsRow,
    custom_fog_setting_index: ShaderGridRow,
    sort_layer: ShaderGridRow,
}

/// The 7 shader types that carry a `global material type` row (the first 8
/// interface ctors in Guerilla, minus the base). The 6 effect-style shaders
/// (particle/contrail/light_volume/beam/decal/water) have no material row.
pub(super) fn shader_type_has_material_row(group_tag: u32) -> bool {
    matches!(
        &group_tag.to_be_bytes(),
        b"rmsh" | b"rmtr" | b"rmcs" | b"rmhg" | b"rmfl" | b"rmsk" | b"rmct"
    )
}

pub(super) struct ShaderEditorCategory {
    index: usize,
    name: String,
    options: Vec<String>,
    selected: i16,
    edit_path: Option<String>,
}

pub(super) struct ShaderEditorSection {
    title: String,
    option_name: String,
    rows: Vec<ShaderGridRow>,
}

pub(super) struct ShaderFlagsRow {
    label: String,
    path: String,
    raw: u64,
    options: Vec<ShaderFlagOption>,
}

pub(super) struct ShaderFlagOption {
    bit: u32,
    label: &'static str,
}

pub(super) fn build_shader_editor_model(
    tag: &TagFile,
    group_tag: u32,
    source: Option<&TagSource>,
    rmdf_cache: &mut HashMap<String, Option<RenderMethodDefinition>>,
    rmop_cache: &mut HashMap<String, Option<RenderMethodOption>>,
) -> Option<ShaderEditorModel> {
    let source = source?;
    let render_method = RenderMethod::from_tag(tag).ok()?;
    if render_method.definition_path.is_empty() {
        return None;
    }
    let definition =
        cached_render_method_definition(source, &render_method.definition_path, rmdf_cache)?;
    let edit_prefix = render_method_edit_prefix(tag);

    let mut categories = Vec::new();
    let mut sections = Vec::new();
    for (index, category) in definition.categories.iter().enumerate() {
        let selected = render_method.options.get(index).copied().unwrap_or(0);
        let option_names = category
            .options
            .iter()
            .map(|option| option.option_name.clone())
            .collect::<Vec<_>>();
        let selected_index = selected.max(0) as usize;
        let selected_option = category.options.get(selected_index);
        categories.push(ShaderEditorCategory {
            index,
            name: category.category_name.clone(),
            options: option_names,
            selected,
            edit_path: (index < render_method.options.len())
                .then(|| append_field_path(&edit_prefix, &format!("options[{index}]/short"))),
        });

        let Some(selected_option) = selected_option else {
            continue;
        };
        if selected_option.option_path.is_empty() {
            continue;
        }
        let Some(option) =
            cached_render_method_option(source, &selected_option.option_path, rmop_cache)
        else {
            continue;
        };
        let rows = shader_rows_from_option(tag, &render_method, &option, &edit_prefix);
        if rows.is_empty() {
            continue;
        }
        sections.push(ShaderEditorSection {
            title: category.category_name.to_ascii_uppercase(),
            option_name: selected_option.option_name.clone(),
            rows,
        });
    }

    let global_material_type = read_global_material_type(tag);
    let global_material_edit_path = append_field_path(&edit_prefix, "global material type");
    let shader_flags_path =
        render_method_existing_field_path(tag, &edit_prefix, &["shader flags", "shader flags*"]);
    let custom_fog_path =
        render_method_existing_field_path(tag, &edit_prefix, &["Custom fog setting index"]);
    let sort_layer_path =
        render_method_existing_field_path(tag, &edit_prefix, &["sort layer", "sort layer*"]);
    let atmosphere_flags = ShaderFlagsRow {
        label: "Flags".to_owned(),
        path: shader_flags_path,
        raw: render_method_flags_mask(&render_method),
        options: vec![
            ShaderFlagOption {
                bit: 0,
                label: "don't fog me",
            },
            ShaderFlagOption {
                bit: 1,
                label: "use custom setting",
            },
            ShaderFlagOption {
                bit: 2,
                label: "calculate Z camera",
            },
        ],
    };
    let custom_fog_setting_index = shader_int_value_row(
        "Custom Setting Index".to_owned(),
        "0".to_owned(),
        render_method.custom_fog_setting_index.to_string(),
        custom_fog_path,
    );
    let sort_layer = shader_enum_value_row(
        "Sort layer".to_owned(),
        "normal".to_owned(),
        render_method.sort_layer.raw().max(0) as usize,
        vec![
            "invalid".to_owned(),
            "pre-pass".to_owned(),
            "normal".to_owned(),
            "post-pass".to_owned(),
        ],
        sort_layer_path,
    );

    Some(ShaderEditorModel {
        has_material_row: shader_type_has_material_row(group_tag),
        global_material_type,
        global_material_edit_path,
        definition_path: render_method.definition_path,
        shader_template_path: render_method
            .postprocess_definition
            .as_ref()
            .map(|postprocess| postprocess.template_path.clone())
            .filter(|path| !path.is_empty()),
        categories,
        sections,
        atmosphere_flags,
        custom_fog_setting_index,
        sort_layer,
    })
}

pub(super) fn build_classic_shader_editor_model(
    tag: &TagFile,
    names: &TagNameIndex,
) -> Option<ShaderEditorModel> {
    tag.classic_engine()?;

    let mut general_rows = Vec::new();
    let mut sections = Vec::new();
    let root = tag.root();
    for field in root.fields() {
        let path = escape_field_path_segment(field.name());
        if let Some(row) = classic_shader_row_from_field(root, field, &path, "", names) {
            general_rows.push(row);
            continue;
        }
        if let Some(nested) = field.as_struct() {
            let mut rows = Vec::new();
            push_classic_shader_rows(nested, &path, "", names, &mut rows);
            if !rows.is_empty() {
                sections.push(ShaderEditorSection {
                    title: clean_field_name(field.name()).to_ascii_uppercase(),
                    option_name: String::new(),
                    rows,
                });
            }
            continue;
        }
        if let Some(block) = field.as_block() {
            for (index, element) in block.iter().enumerate() {
                let mut rows = Vec::new();
                let block_path = format!("{path}[{index}]");
                push_classic_shader_rows(element, &block_path, "", names, &mut rows);
                if !rows.is_empty() {
                    let suffix = if block.len() > 1 {
                        format!(" [{}]", index)
                    } else {
                        String::new()
                    };
                    sections.push(ShaderEditorSection {
                        title: format!(
                            "{}{}",
                            clean_field_name(field.name()).to_ascii_uppercase(),
                            suffix
                        ),
                        option_name: String::new(),
                        rows,
                    });
                }
            }
        }
    }

    if !general_rows.is_empty() {
        sections.insert(
            0,
            ShaderEditorSection {
                title: "SHADER".to_owned(),
                option_name: String::new(),
                rows: general_rows,
            },
        );
    }
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

pub(super) fn build_h2ek_shader_editor_model(
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

fn h2_normalize_shader_template_reference(path: &str) -> String {
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
        "light response",
        "shader LOD bias",
        "specular type",
        "lightmap type",
        "lightmap specular brightness",
        "lightmap ambient bias",
        "Added depth bias offset",
        "Added depth bias slope scale",
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
    if let Some(row) = h2_shader_row_from_field(tag_struct, field, &path, "", names) {
        rows.push(row);
    }
}

fn h2_template_parameter_section_title(
    root: TagStruct<'_>,
    template: Option<TagStruct<'_>>,
    names: &TagNameIndex,
) -> String {
    if let Some(template) = template {
        if let Some(category) = template
            .field("categories")
            .and_then(|field| field.as_block())
            .and_then(|block| block.element(0))
            .and_then(|category| category.read_string_id("name"))
            .filter(|name| !name.is_empty())
        {
            return category.replace('_', " ").to_ascii_uppercase();
        }
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
    let Some(categories) = template_root
        .field("categories")
        .and_then(|field| field.as_block())
    else {
        return rows;
    };
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
                names,
            ));
        }
    }
    rows
}

struct H2ParameterInstance<'a> {
    index: usize,
    name: String,
    element: TagStruct<'a>,
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
    names: &TagNameIndex,
) -> Vec<ShaderGridRow> {
    let mut rows = Vec::new();
    if let Some(row) = h2_template_base_parameter_row(template_param, instance, names) {
        rows.push(row);
    }
    if h2_template_parameter_type_index(template_param) == 0 {
        rows.extend(h2_template_bitmap_animation_rows(template_param, instance));
    } else if h2_template_flags(template_param) & 1 != 0 {
        rows.push(h2_template_value_animation_row(template_param, instance));
    }
    rows
}

fn h2_template_base_parameter_row(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
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
    let instance_value = instance.and_then(|instance| {
        instance
            .element
            .field(field_name)
            .and_then(|field| field.value())
            .map(|value| (instance.index, value))
    });
    let (value_text, value_kind, color, edit) = if let Some((index, value)) = instance_value {
        let formatted = format_value(names, &value, false);
        let color = color_popup_for_value(&label, &value, &formatted);
        let path = format!(
            "parameters[{index}]/{}",
            escape_field_path_segment(field_name)
        );
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
        (fallback, "default", None, None)
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
) -> Vec<ShaderGridRow> {
    let flags = h2_template_bitmap_animation_flags(template_param);
    let mut rows = Vec::new();
    let specs: &[(u32, &[(i32, &str)])] = &[
        (1 << 0, &[(0, "scale")]),
        (1 << 1, &[(1, "scale_x"), (2, "scale_y")]),
        (1 << 2, &[(4, "translation_x"), (5, "translation_y")]),
        (1 << 3, &[(7, "rotation")]),
        (1 << 4, &[(13, "index")]),
    ];
    for (bit, entries) in specs {
        if flags & bit == 0 {
            continue;
        }
        for (animation_type, suffix) in *entries {
            rows.push(h2_template_animation_row(
                template_param,
                instance,
                *animation_type,
                suffix,
            ));
        }
    }
    rows
}

fn h2_template_value_animation_row(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
) -> ShaderGridRow {
    let suffix = if h2_template_parameter_type_index(template_param) == 2 {
        "tint"
    } else {
        "value"
    };
    h2_template_animation_row(template_param, instance, 11, suffix)
}

fn h2_template_animation_row(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
    animation_type: i32,
    suffix: &str,
) -> ShaderGridRow {
    let base = h2_template_parameter_name(template_param);
    let label = format!("{base}_{suffix}");
    let function = instance.and_then(|instance| {
        h2_find_animation_by_type(instance.element, animation_type).and_then(
            |(anim_index, anim)| {
                let path = format!(
                    "parameters[{}]/animation properties[{anim_index}]",
                    instance.index
                );
                let function_struct = anim.field("function")?.as_struct()?;
                let function_path = append_field_path(&path, "function");
                h2_function_view_from_animation_property(anim, function_struct, &function_path)
            },
        )
    });
    let mut row = if let Some(function) = function {
        shader_function_grid_row(label, function)
    } else {
        h2_missing_function_row(label)
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

fn h2_missing_function_row(label: String) -> ShaderGridRow {
    ShaderGridRow {
        label,
        default_cell: None,
        value_cell: ShaderGridCell {
            text: String::new(),
            value_kind: "default",
            color: None,
        },
        fill: material_numeric_row(),
        parameter_type: Some("function".to_owned()),
        function: None,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

fn h2_template_parameter_name(template_param: TagStruct<'_>) -> String {
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
    let edit = classic_shader_row_edit(path, &value, &formatted);
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

fn push_classic_shader_rows(
    tag_struct: TagStruct<'_>,
    path_prefix: &str,
    label_prefix: &str,
    names: &TagNameIndex,
    rows: &mut Vec<ShaderGridRow>,
) {
    for field in tag_struct.fields() {
        let path = append_field_path(path_prefix, &escape_field_path_segment(field.name()));
        if let Some(row) =
            classic_shader_row_from_field(tag_struct, field, &path, label_prefix, names)
        {
            rows.push(row);
            continue;
        }
        let label = classic_nested_label(label_prefix, field.name());
        if let Some(nested) = field.as_struct() {
            push_classic_shader_rows(nested, &path, &label, names, rows);
        } else if let Some(block) = field.as_block() {
            for (index, element) in block.iter().enumerate() {
                let block_path = format!("{path}[{index}]");
                let block_label = if block.len() > 1 {
                    format!("{label} {index}")
                } else {
                    label.clone()
                };
                push_classic_shader_rows(element, &block_path, &block_label, names, rows);
            }
        }
    }
}

fn classic_shader_row_from_field(
    parent: TagStruct<'_>,
    field: TagField<'_>,
    path: &str,
    label_prefix: &str,
    names: &TagNameIndex,
) -> Option<ShaderGridRow> {
    if let Some(function) = field.as_function() {
        return Some(shader_function_grid_row(
            classic_nested_label(label_prefix, field.name()),
            FunctionView::from_function(function),
        ));
    }
    if let Some(nested) = field.as_struct() {
        if let Some(function) =
            classic_halo2_function_view_from_struct(parent, nested, path, field.name())
        {
            return Some(shader_function_grid_row(
                classic_nested_label(label_prefix, field.name()),
                function,
            ));
        }
    }
    let value = field.value()?;
    if matches!(
        value,
        TagFieldData::Data(_) | TagFieldData::ApiInterop(_) | TagFieldData::Custom(_)
    ) {
        return None;
    }
    let label = classic_nested_label(label_prefix, field.name());
    let formatted = format_value(names, &value, false);
    let color = color_popup_for_value(&label, &value, &formatted);
    let edit = classic_shader_row_edit(path, &value, &formatted);
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
        function: None,
        edit,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    })
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
        let inner = tag_struct.field("function")?.as_struct()?;
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
        view.with_edit(FunctionEditPaths {
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

pub(super) fn halo2_function_bytes_from_struct(tag_struct: TagStruct<'_>) -> Option<Vec<u8>> {
    let block = tag_struct.field("data")?.as_block()?;
    let mut bytes = Vec::with_capacity(block.len());
    for element in block.iter() {
        let value = element.read_int_any("Value")?;
        bytes.push(value as i8 as u8);
    }
    Some(bytes)
}

#[cfg(test)]
pub(super) fn first_halo2_byte_block_function_row(
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
pub(super) fn shader_row_edit_path_and_kind(
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
    let kind = match &edit.kind {
        ShaderRowEditKind::Scalar => "scalar",
        ShaderRowEditKind::Int => "int",
        ShaderRowEditKind::StringId => "string_id",
        ShaderRowEditKind::BitmapRef { .. } => "bitmap_ref",
        ShaderRowEditKind::Bool { .. } => "bool",
        ShaderRowEditKind::Enum(_) => "enum",
        ShaderRowEditKind::FunctionScalar { .. } => "function_scalar",
        ShaderRowEditKind::FunctionColor { .. } => "function_color",
        ShaderRowEditKind::ColorField { .. } => "color",
        ShaderRowEditKind::CreateFunctionColor { .. } => "create_function_color",
        ShaderRowEditKind::CreateFunctionScalar { .. } => "create_function_scalar",
        ShaderRowEditKind::CreateScalarParam { .. } => "create_scalar_param",
    };
    Some((edit.path.clone(), kind))
}

#[cfg(test)]
pub(super) fn h2_template_row_labels_for_test(shader: &TagFile, template: &TagFile) -> Vec<String> {
    h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default())
        .into_iter()
        .map(|row| row.label)
        .collect()
}

#[cfg(test)]
pub(super) fn h2_shader_template_reference_for_test(tag: &TagFile) -> Option<String> {
    h2_shader_template_reference(tag.root())
}

#[cfg(test)]
pub(super) struct H2FunctionEditSummary {
    pub(super) bytes: Vec<u8>,
    pub(super) output_index: Option<i32>,
    pub(super) input_name: String,
    pub(super) range_name: String,
    pub(super) time_period: f32,
    pub(super) data_path: String,
    pub(super) parameter_type_path: String,
    pub(super) input_name_path: String,
    pub(super) range_name_path: String,
    pub(super) time_period_path: String,
}

#[cfg(test)]
pub(super) fn first_h2_function_edit_summary(
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

fn empty_shader_grid_row() -> ShaderGridRow {
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
        function: None,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

pub(super) fn render_method_flags_mask(render_method: &RenderMethod) -> u64 {
    let mut mask = 0u64;
    for flag in render_method.flags.get() {
        let bit = match flag {
            GlobalRenderMethodFlags::DontFogMe => 0,
            GlobalRenderMethodFlags::UseCustomSetting => 1,
            GlobalRenderMethodFlags::CalculateZCamera => 2,
        };
        mask |= 1u64 << bit;
    }
    mask
}

pub(super) fn cached_render_method_definition(
    source: &TagSource,
    reference: &str,
    cache: &mut HashMap<String, Option<RenderMethodDefinition>>,
) -> Option<RenderMethodDefinition> {
    if reference.is_empty() {
        return None;
    }
    let key = format!("rmdf:{reference}");
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let parsed =
        load_referenced_tag_from_source(source, reference, "render_method_definition", b"rmdf")
            .ok()
            .and_then(|tag| RenderMethodDefinition::from_tag(&tag).ok());
    cache.insert(key, parsed.clone());
    parsed
}

pub(super) fn cached_render_method_option(
    source: &TagSource,
    reference: &str,
    cache: &mut HashMap<String, Option<RenderMethodOption>>,
) -> Option<RenderMethodOption> {
    if reference.is_empty() {
        return None;
    }
    let key = format!("rmop:{reference}");
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let parsed =
        load_referenced_tag_from_source(source, reference, "render_method_option", b"rmop")
            .ok()
            .and_then(|tag| RenderMethodOption::from_tag(&tag).ok());
    cache.insert(key, parsed.clone());
    parsed
}

pub(super) fn render_method_edit_prefix(tag: &TagFile) -> String {
    if tag.root().field("render_method").is_some() {
        "render_method".to_owned()
    } else {
        String::new()
    }
}

pub(super) fn render_method_existing_field_path(
    tag: &TagFile,
    edit_prefix: &str,
    candidates: &[&str],
) -> String {
    for candidate in candidates {
        let path = append_field_path(edit_prefix, candidate);
        if tag.root().field_path(&path).is_some() {
            return path;
        }
    }
    candidates
        .first()
        .map(|candidate| append_field_path(edit_prefix, candidate))
        .unwrap_or_default()
}

/// Read the `global material type` string-id from the render_method block.
/// Returns the string name (e.g. `"default_material"`) or a fallback.
pub(super) fn read_global_material_type(tag: &TagFile) -> String {
    let root = tag.root();
    let rm = root.descend("render_method").unwrap_or(root);
    rm.read_string_id("global material type")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default_material".to_owned())
}

/// Build the tag field paths for the `animated_index`-th animated
/// parameter of `param_index`-th render-method parameter. Relies on the
/// parsed `parameters` / `animated parameters` Vecs being 1:1 with their
/// schema blocks (both `from_struct` readers are infallible, so no
/// elements are skipped).
pub(super) fn animated_param_paths(
    prefix: &str,
    param_index: usize,
    animated_index: usize,
) -> FunctionEditPaths {
    let block_path = append_field_path(
        prefix,
        &format!("parameters[{param_index}]/animated parameters"),
    );
    let base = format!("{block_path}[{animated_index}]");
    FunctionEditPaths {
        data: FunctionDataStorage::DataField(append_field_path(&base, "function/data")),
        parameter_type: append_field_path(&base, "type"),
        input_name: append_field_path(&base, "input name"),
        range_name: append_field_path(&base, "range name"),
        time_period: append_field_path(&base, "time period"),
        block_path,
        block_index: animated_index,
    }
}

pub(super) fn existing_shader_function_target(
    edit_prefix: &str,
    param_index: usize,
    output_type_index: i32,
) -> ShaderFunctionCreateTarget {
    ShaderFunctionCreateTarget::ExistingParameter {
        animated_block_path: append_field_path(
            edit_prefix,
            &format!("parameters[{param_index}]/animated parameters"),
        ),
        output_type_index,
    }
}

pub(super) fn new_shader_function_target(
    edit_prefix: &str,
    parameter_name: &str,
    parameter_type_index: i32,
    output_type_index: i32,
) -> ShaderFunctionCreateTarget {
    ShaderFunctionCreateTarget::NewParameter {
        parameters_block_path: append_field_path(edit_prefix, "parameters"),
        parameter_name: parameter_name.to_owned(),
        parameter_type_index,
        output_type_index,
    }
}

pub(super) fn shader_parameter_type_index(parameter: &RenderMethodOptionParameter) -> i32 {
    parameter
        .parameter_type
        .map(|kind| kind.raw() as i32)
        .unwrap_or(RenderMethodParameterType::Real as i32)
}

pub(super) fn shader_parameter_type_initial_field(
    parameter_type_index: i32,
) -> ShaderParamInitialField {
    ShaderParamInitialField {
        field: "parameter type".to_owned(),
        input: parameter_type_index.to_string(),
    }
}

pub(super) fn shader_function_action(
    target: &ShaderFunctionCreateTarget,
    initial_function_hex: String,
) -> ShaderContextAction {
    match target {
        ShaderFunctionCreateTarget::ExistingParameter {
            animated_block_path,
            output_type_index,
        } => ShaderContextAction::AnimatedParameter(ShaderOp {
            animated_block_path: animated_block_path.clone(),
            output_type_index: *output_type_index,
            initial_function_hex,
        }),
        ShaderFunctionCreateTarget::NewParameter {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
            output_type_index,
        } => ShaderContextAction::ParameterOp(ShaderParamOp {
            parameters_block_path: parameters_block_path.clone(),
            parameter_name: parameter_name.clone(),
            initial_fields: vec![shader_parameter_type_initial_field(*parameter_type_index)],
            animated_parameters: vec![ShaderParamInitialAnimated {
                output_type_index: *output_type_index,
                initial_function_hex,
            }],
        }),
    }
}

pub(super) fn push_shader_context_action(
    edit: &mut FieldEditContext<'_>,
    action: &ShaderContextAction,
) {
    match action {
        ShaderContextAction::AnimatedParameter(op) => {
            edit.shader_ops.push(op.clone());
        }
        ShaderContextAction::FieldEdits(edits) => {
            edit.pending.extend(edits.iter().cloned());
        }
        ShaderContextAction::ParameterOp(op) => {
            edit.shader_param_ops.push(op.clone());
        }
    }
}

pub(super) fn shader_rows_from_option(
    tag: &TagFile,
    render_method: &RenderMethod,
    option: &RenderMethodOption,
    edit_prefix: &str,
) -> Vec<ShaderGridRow> {
    let mut rows = Vec::new();
    for parameter in &option.parameters {
        if parameter.parameter_name.is_empty() {
            continue;
        }
        let instance_index = render_method
            .parameters
            .iter()
            .position(|value| value.parameter_name == parameter.parameter_name);
        let instance = instance_index.map(|i| &render_method.parameters[i]);
        push_shader_parameter_rows(
            &mut rows,
            parameter,
            instance,
            edit_prefix,
            instance_index,
            tag,
        );
    }
    rows
}

pub(super) fn push_shader_parameter_rows(
    rows: &mut Vec<ShaderGridRow>,
    parameter: &RenderMethodOptionParameter,
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
    tag: &TagFile,
) {
    match parameter
        .parameter_type
        .map(|kind| kind.get())
        .unwrap_or(RenderMethodParameterType::Real)
    {
        RenderMethodParameterType::Bitmap => {
            rows.push(shader_bitmap_row(
                parameter,
                instance,
                edit_prefix,
                param_index,
            ));
            rows.extend(shader_bitmap_expansion_rows(
                parameter,
                instance,
                edit_prefix,
                param_index,
            ));
        }
        RenderMethodParameterType::Color | RenderMethodParameterType::ArgbColor => {
            rows.push(shader_color_row(
                parameter,
                instance,
                edit_prefix,
                param_index,
                tag,
            ));
            rows.push(shader_alpha_row(
                parameter,
                instance,
                edit_prefix,
                param_index,
            ));
        }
        RenderMethodParameterType::Real => rows.push(shader_scalar_row(
            parameter,
            instance,
            edit_prefix,
            param_index,
        )),
        RenderMethodParameterType::Int => rows.push(shader_int_row(
            parameter,
            instance,
            edit_prefix,
            param_index,
        )),
        RenderMethodParameterType::Bool => rows.push(shader_bool_row(
            parameter,
            instance,
            edit_prefix,
            param_index,
        )),
    }
}

/// Build the tag field path to a leaf field of `parameters[param_index]`,
/// escaping any literal `/` in the field name (e.g. `int/bool`). Returns
/// `None` when there's no instance parameter to write to.
pub(super) fn shader_param_field_path(
    prefix: &str,
    param_index: Option<usize>,
    field: &str,
) -> Option<String> {
    let i = param_index?;
    let escaped = field.replace('/', "\\/");
    Some(append_field_path(
        prefix,
        &format!("parameters[{i}]/{escaped}"),
    ))
}

pub(super) fn shader_param_existing_field_path(
    tag: &TagFile,
    prefix: &str,
    param_index: Option<usize>,
    field: &str,
) -> Option<String> {
    let path = shader_param_field_path(prefix, param_index, field)?;
    tag.root().field_path(&path).is_some().then_some(path)
}

/// 32-byte `mapping_function` blob for a Constant function with value 1.0.
/// Used as the default for scale transform animated parameters.
pub(super) const CONSTANT_FUNCTION_1_HEX: &str =
    "012000000000803f0000803f0000000000000000000000000000000000000000";

/// 32-byte `mapping_function` blob for a Constant function with value 0.0.
/// Used as the default for translation/frame-index animated parameters.
pub(super) const CONSTANT_FUNCTION_0_HEX: &str =
    "0120000000000000000000000000000000000000000000000000000000000000";

/// Build a 32-byte `mapping_function` hex blob for `Constant(v)`.
///
/// Layout: byte 0 = 1 (Constant), bytes 1-3 = 0, bytes 4-7 = v (f32 LE),
/// bytes 8-11 = v (f32 LE, clamp_range_max mirrors min for unranged), rest 0.
pub(super) fn constant_function_hex(v: f32) -> String {
    let mut blob = [0u8; 32];
    blob[0] = 1; // FunctionType::Constant
    blob[1] = FunctionFlags::GPU;
    blob[4..8].copy_from_slice(&v.to_le_bytes());
    blob[8..12].copy_from_slice(&v.to_le_bytes());
    blob.iter().map(|b| format!("{b:02x}")).collect()
}

/// True when `f` is a Constant-type function with a color (not scalar) output.
/// Used to decide whether to show a constant color swatch vs a graph row.
pub(super) fn is_constant_color_fn(f: &TagFunction) -> bool {
    f.color_graph_type() != ColorGraphType::Scalar && matches!(f, TagFunction::Constant { .. })
}

/// Extract the (r, g, b, a) components from a constant 1-color function.
/// Returns None for scalar functions or non-constant types.
pub(super) fn extract_constant_color(f: &TagFunction) -> Option<[f32; 4]> {
    if !is_constant_color_fn(f) {
        return None;
    }
    let argb = f.header().colors[0];
    let alpha = ((argb >> 24) & 0xFF) as f32 / 255.0;
    Some([
        ((argb >> 16) & 0xFF) as f32 / 255.0, // r
        ((argb >> 8) & 0xFF) as f32 / 255.0,  // g
        (argb & 0xFF) as f32 / 255.0,         // b
        if alpha == 0.0 { 1.0 } else { alpha },
    ])
}

/// Build a 32-byte `mapping_function` hex blob for a Constant 1-color function.
/// Layout: byte 0 = 1 (Constant), byte 2 = 1 (OneColor), bytes 4-7 = ARGB u32 LE.
pub(super) fn constant_color_function_hex(r: f32, g: f32, b: f32, a: f32) -> String {
    let mut blob = [0u8; 32];
    blob[0] = 1; // FunctionType::Constant
    blob[1] = FunctionFlags::GPU;
    blob[2] = 1; // ColorGraphType::OneColor
    let a8 = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
    let r8 = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
    let g8 = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
    let b8 = (b.clamp(0.0, 1.0) * 255.0).round() as u8;
    let argb = ((a8 as u32) << 24) | ((r8 as u32) << 16) | ((g8 as u32) << 8) | (b8 as u32);
    blob[4..8].copy_from_slice(&argb.to_le_bytes());
    blob.iter().map(|b| format!("{b:02x}")).collect()
}

/// The optional bitmap-transform output types, in Guerilla `sub_140651C30` order.
pub(super) const BITMAP_TRANSFORM_TYPES: &[(RenderMethodAnimatedParameterType, &str, &str)] = &[
    (
        RenderMethodAnimatedParameterType::ScaleUniform,
        "scale_uniform",
        CONSTANT_FUNCTION_1_HEX,
    ),
    (
        RenderMethodAnimatedParameterType::ScaleX,
        "scale_x",
        CONSTANT_FUNCTION_1_HEX,
    ),
    (
        RenderMethodAnimatedParameterType::ScaleY,
        "scale_y",
        CONSTANT_FUNCTION_1_HEX,
    ),
    (
        RenderMethodAnimatedParameterType::TranslationX,
        "translation_x",
        CONSTANT_FUNCTION_0_HEX,
    ),
    (
        RenderMethodAnimatedParameterType::TranslationY,
        "translation_y",
        CONSTANT_FUNCTION_0_HEX,
    ),
    (
        RenderMethodAnimatedParameterType::FrameIndex,
        "frame_index",
        CONSTANT_FUNCTION_0_HEX,
    ),
];

const BITMAP_FLAG_FILTER: i16 = 0x01;
const BITMAP_FLAG_ADDRESS: i16 = 0x02;
const BITMAP_FLAG_ADDRESS_X: i16 = 0x04;
const BITMAP_FLAG_ADDRESS_Y: i16 = 0x08;

pub(super) struct BitmapSamplerOverride {
    menu_label: &'static str,
    field: &'static str,
    flag_bit: i16,
}

pub(super) const BITMAP_SAMPLER_OVERRIDES: &[BitmapSamplerOverride] = &[
    BitmapSamplerOverride {
        menu_label: "wrap mode",
        field: "bitmap address mode",
        flag_bit: BITMAP_FLAG_ADDRESS,
    },
    BitmapSamplerOverride {
        menu_label: "wrap mode x",
        field: "bitmap address mode x",
        flag_bit: BITMAP_FLAG_ADDRESS_X,
    },
    BitmapSamplerOverride {
        menu_label: "wrap mode y",
        field: "bitmap address mode y",
        flag_bit: BITMAP_FLAG_ADDRESS_Y,
    },
    BitmapSamplerOverride {
        menu_label: "filter mode",
        field: "bitmap filter mode",
        flag_bit: BITMAP_FLAG_FILTER,
    },
];

pub(super) fn shader_bitmap_row(
    parameter: &RenderMethodOptionParameter,
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
) -> ShaderGridRow {
    let value = instance
        .map(|param| param.bitmap_path.as_str())
        .filter(|path| !path.is_empty())
        .unwrap_or(parameter.default_bitmap_path.as_str());
    // The bitmap is shown WITH its .bitmap extension so the editor round-trips
    // the same string format as a normal tag-reference field.
    let display = if value.is_empty() {
        "NONE".to_owned()
    } else {
        format!("{}.bitmap", value.replace('\\', "/"))
    };
    let bitmap_input = display.clone();
    let parameter_type_index = shader_parameter_type_index(parameter);

    // Build right-click context menu: offer transform types not yet present.
    let context_menu = {
        let existing_types: std::collections::HashSet<i32> = instance
            .iter()
            .flat_map(|inst| &inst.animated_parameters)
            .filter_map(|ap| ap.parameter_type.map(|t| t.raw()))
            .collect();
        let mut items = Vec::new();
        if let Some(pidx) = param_index {
            let animated_block_path = append_field_path(
                edit_prefix,
                &format!("parameters[{pidx}]/animated parameters"),
            );
            for (kind, suffix, hex) in BITMAP_TRANSFORM_TYPES {
                if existing_types.contains(&(*kind as i32)) {
                    continue;
                }
                items.push(ShaderContextItem {
                    label: suffix.replace('_', " "),
                    action: ShaderContextAction::AnimatedParameter(ShaderOp {
                        animated_block_path: animated_block_path.clone(),
                        output_type_index: *kind as i32,
                        initial_function_hex: hex.to_string(),
                    }),
                });
            }
        } else {
            let parameters_block_path = append_field_path(edit_prefix, "parameters");
            for (kind, suffix, hex) in BITMAP_TRANSFORM_TYPES {
                items.push(ShaderContextItem {
                    label: suffix.replace('_', " "),
                    action: ShaderContextAction::ParameterOp(ShaderParamOp {
                        parameters_block_path: parameters_block_path.clone(),
                        parameter_name: parameter.parameter_name.clone(),
                        initial_fields: vec![
                            shader_parameter_type_initial_field(parameter_type_index),
                            ShaderParamInitialField {
                                field: "bitmap".to_owned(),
                                input: bitmap_input.clone(),
                            },
                        ],
                        animated_parameters: vec![ShaderParamInitialAnimated {
                            output_type_index: *kind as i32,
                            initial_function_hex: hex.to_string(),
                        }],
                    }),
                });
            }
        }
        let current_flags = instance.map(|inst| inst.bitmap_flags).unwrap_or(0);
        for sampler in BITMAP_SAMPLER_OVERRIDES {
            if current_flags & sampler.flag_bit != 0 {
                continue;
            }
            let initial_value = if sampler.flag_bit == BITMAP_FLAG_FILTER {
                parameter.default_filter_mode.raw()
            } else {
                parameter.default_address_mode.raw()
            };
            if let Some(pidx) = param_index {
                let flag_path =
                    append_field_path(edit_prefix, &format!("parameters[{pidx}]/bitmap flags"));
                let field_path = append_field_path(
                    edit_prefix,
                    &format!("parameters[{pidx}]/{}", sampler.field),
                );
                items.push(ShaderContextItem {
                    label: sampler.menu_label.to_owned(),
                    action: ShaderContextAction::FieldEdits(vec![
                        PendingFieldEdit {
                            path: flag_path,
                            input: (current_flags | sampler.flag_bit).to_string(),
                        },
                        PendingFieldEdit {
                            path: field_path,
                            input: initial_value.to_string(),
                        },
                    ]),
                });
            } else {
                items.push(ShaderContextItem {
                    label: sampler.menu_label.to_owned(),
                    action: ShaderContextAction::ParameterOp(ShaderParamOp {
                        parameters_block_path: append_field_path(edit_prefix, "parameters"),
                        parameter_name: parameter.parameter_name.clone(),
                        initial_fields: vec![
                            shader_parameter_type_initial_field(parameter_type_index),
                            ShaderParamInitialField {
                                field: "bitmap".to_owned(),
                                input: bitmap_input.clone(),
                            },
                            ShaderParamInitialField {
                                field: "bitmap flags".to_owned(),
                                input: sampler.flag_bit.to_string(),
                            },
                            ShaderParamInitialField {
                                field: sampler.field.to_owned(),
                                input: initial_value.to_string(),
                            },
                        ],
                        animated_parameters: Vec::new(),
                    }),
                });
            }
        }
        if instance.and_then(|inst| inst.bitmap_extern_mode).is_none() {
            if let Some(pidx) = param_index {
                let field_path = append_field_path(
                    edit_prefix,
                    &format!("parameters[{pidx}]/bitmap extern RTT mode"),
                );
                items.push(ShaderContextItem {
                    label: "extern mode".to_owned(),
                    action: ShaderContextAction::FieldEdits(vec![PendingFieldEdit {
                        path: field_path,
                        input: "1".to_owned(),
                    }]),
                });
            } else {
                items.push(ShaderContextItem {
                    label: "extern mode".to_owned(),
                    action: ShaderContextAction::ParameterOp(ShaderParamOp {
                        parameters_block_path: append_field_path(edit_prefix, "parameters"),
                        parameter_name: parameter.parameter_name.clone(),
                        initial_fields: vec![
                            shader_parameter_type_initial_field(parameter_type_index),
                            ShaderParamInitialField {
                                field: "bitmap".to_owned(),
                                input: bitmap_input.clone(),
                            },
                            ShaderParamInitialField {
                                field: "bitmap extern RTT mode".to_owned(),
                                input: "1".to_owned(),
                            },
                        ],
                        animated_parameters: Vec::new(),
                    }),
                });
            }
        }
        Some(ShaderContextMenu { items })
    };

    ShaderGridRow {
        label: parameter.parameter_name.clone(),
        default_cell: Some(ShaderGridCell {
            text: none_if_empty(&parameter.default_bitmap_path),
            value_kind: "default",
            color: None,
        }),
        value_cell: ShaderGridCell {
            text: display,
            value_kind: if value.is_empty() { "default" } else { "value" },
            color: None,
        },
        fill: material_ref_row(),
        parameter_type: Some("bitmap".to_owned()),
        function: None,
        edit: shader_param_field_path(edit_prefix, param_index, "bitmap")
            .map(|path| ShaderRowEdit {
                path,
                current: if value.is_empty() {
                    "NONE".to_owned()
                } else {
                    format!("{}.bitmap", value.replace('\\', "/"))
                },
                kind: ShaderRowEditKind::BitmapRef {
                    group_tag: u32::from_be_bytes(*b"bitm"),
                    create: None,
                },
            })
            .or_else(|| {
                Some(ShaderRowEdit {
                    path: String::new(),
                    current: if value.is_empty() {
                        "NONE".to_owned()
                    } else {
                        format!("{}.bitmap", value.replace('\\', "/"))
                    },
                    kind: ShaderRowEditKind::BitmapRef {
                        group_tag: u32::from_be_bytes(*b"bitm"),
                        create: Some(ShaderParamCreateTarget {
                            parameters_block_path: append_field_path(edit_prefix, "parameters"),
                            parameter_name: parameter.parameter_name.clone(),
                            parameter_type_index,
                            field: "bitmap",
                        }),
                    },
                })
            }),
        context_menu,
        create_anim_op: None,
        constant_function_view: None,
    }
}

pub(super) fn shader_bitmap_expansion_rows(
    parameter: &RenderMethodOptionParameter,
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
) -> Vec<ShaderGridRow> {
    let mut rows = Vec::new();
    let name = &parameter.parameter_name;
    let filter_opts = bitmap_filter_option_labels();
    let addr_opts = bitmap_address_option_labels();

    if let Some(instance) = instance {
        let flags = instance.bitmap_flags;
        if flags & BITMAP_FLAG_FILTER != 0 {
            rows.push(shader_sampler_enum_row(
                format!("{name}_filter_mode"),
                parameter.default_filter_mode.raw(),
                instance.bitmap_filter_mode as i16,
                filter_opts,
                edit_prefix,
                param_index,
                "bitmap filter mode",
            ));
        }
        if flags & BITMAP_FLAG_ADDRESS != 0 {
            rows.push(shader_sampler_enum_row(
                format!("{name}_wrap_mode"),
                parameter.default_address_mode.raw(),
                instance.bitmap_address_mode as i16,
                addr_opts.clone(),
                edit_prefix,
                param_index,
                "bitmap address mode",
            ));
        }
        if flags & BITMAP_FLAG_ADDRESS_X != 0 {
            rows.push(shader_sampler_enum_row(
                format!("{name}_wrap_mode_x"),
                parameter.default_address_mode.raw(),
                instance.bitmap_address_mode_x as i16,
                addr_opts.clone(),
                edit_prefix,
                param_index,
                "bitmap address mode x",
            ));
        }
        if flags & BITMAP_FLAG_ADDRESS_Y != 0 {
            rows.push(shader_sampler_enum_row(
                format!("{name}_wrap_mode_y"),
                parameter.default_address_mode.raw(),
                instance.bitmap_address_mode_y as i16,
                addr_opts.clone(),
                edit_prefix,
                param_index,
                "bitmap address mode y",
            ));
        }
        if let Some(mode) = instance.bitmap_extern_mode {
            rows.push(shader_sampler_enum_row(
                format!("{name}_extern_mode"),
                0,
                mode as i16,
                bitmap_extern_option_labels(),
                edit_prefix,
                param_index,
                "bitmap extern RTT mode",
            ));
        }
    }

    if let Some(instance) = instance {
        for (j, animated) in instance.animated_parameters.iter().enumerate() {
            let Some(function) = animated.function.clone() else {
                continue;
            };
            let suffix = match animated.parameter_type.map(|kind| kind.get()) {
                Some(RenderMethodAnimatedParameterType::ScaleUniform) => "scale_uniform",
                Some(RenderMethodAnimatedParameterType::ScaleX) => "scale_x",
                Some(RenderMethodAnimatedParameterType::ScaleY) => "scale_y",
                Some(RenderMethodAnimatedParameterType::TranslationX) => "translation_x",
                Some(RenderMethodAnimatedParameterType::TranslationY) => "translation_y",
                Some(RenderMethodAnimatedParameterType::FrameIndex) => "frame_index",
                _ => continue,
            };
            let mut view = FunctionView::from_animated(animated, function.clone());
            if let Some(i) = param_index {
                view = view.with_edit(animated_param_paths(edit_prefix, i, j));
            }

            // Constant functions render as editable scalar rows (no graph
            // popup required by default). Non-constant curves stay orange.
            if let Some(const_val) = function.as_constant() {
                let label = format!("{name}_{suffix}");
                let data_path = view
                    .edit
                    .as_ref()
                    .and_then(|e| e.data.data_field_path().map(str::to_owned))
                    .unwrap_or_default();
                let (block_path, block_index) = match view.edit.as_ref() {
                    Some(e) => (e.block_path.clone(), e.block_index),
                    None => (String::new(), 0),
                };
                let default_val = if suffix.ends_with("scale_uniform")
                    || suffix.ends_with("scale_x")
                    || suffix.ends_with("scale_y")
                {
                    "value: 1.00"
                } else {
                    "value: 0.00"
                };
                rows.push(ShaderGridRow {
                    label: label.clone(),
                    default_cell: Some(shader_default_value_cell(default_val.to_owned())),
                    value_cell: shader_value_cell(format!(
                        "value: {}",
                        format_shader_float(const_val)
                    )),
                    fill: material_numeric_row(),
                    parameter_type: Some("animated scalar".to_owned()),
                    function: None,
                    edit: if data_path.is_empty() {
                        None
                    } else {
                        Some(ShaderRowEdit {
                            path: data_path,
                            current: format_shader_float(const_val),
                            kind: ShaderRowEditKind::FunctionScalar {
                                block_path,
                                block_index,
                            },
                        })
                    },
                    context_menu: None,
                    create_anim_op: None,
                    constant_function_view: None,
                    // FunctionView stored here so the user can open the full
                    // graph editor via the "f()" button in draw_shader_grid_row.
                });
                // Patch constant_function_view back in.
                if let Some(row) = rows.last_mut() {
                    row.constant_function_view = Some(view);
                }
            } else {
                rows.push(shader_function_grid_row(format!("{name}_{suffix}"), view));
            }
        }
    }
    rows
}

pub(super) fn shader_scalar_row(
    parameter: &RenderMethodOptionParameter,
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
) -> ShaderGridRow {
    // Look for an existing animated parameter (Value output).
    if let Some(inst) = instance {
        for (j, animated) in inst.animated_parameters.iter().enumerate() {
            if !matches!(
                animated.parameter_type.map(|kind| kind.get()),
                Some(RenderMethodAnimatedParameterType::Value)
            ) {
                continue;
            }
            let Some(function) = animated.function.clone() else {
                continue;
            };
            let mut view = FunctionView::from_animated(animated, function.clone());
            if let Some(i) = param_index {
                view = view.with_edit(animated_param_paths(edit_prefix, i, j));
            }
            // Constant animated scalar → editable FunctionScalar row.
            if let Some(const_val) = function.as_constant() {
                let data_path = view
                    .edit
                    .as_ref()
                    .and_then(|e| e.data.data_field_path().map(str::to_owned))
                    .unwrap_or_default();
                let (block_path, block_index) = match view.edit.as_ref() {
                    Some(e) => (e.block_path.clone(), e.block_index),
                    None => (String::new(), 0),
                };
                let current = format_shader_float(const_val);
                let mut row = ShaderGridRow {
                    label: parameter.parameter_name.clone(),
                    default_cell: Some(shader_default_value_cell(format!(
                        "value: {}",
                        format_shader_float(parameter.default_real_value)
                    ))),
                    value_cell: shader_value_cell(format!("value: {current}")),
                    fill: material_numeric_row(),
                    parameter_type: Some("animated scalar".to_owned()),
                    function: None,
                    edit: if data_path.is_empty() {
                        None
                    } else {
                        Some(ShaderRowEdit {
                            path: data_path,
                            current,
                            kind: ShaderRowEditKind::FunctionScalar {
                                block_path,
                                block_index,
                            },
                        })
                    },
                    context_menu: None,
                    create_anim_op: None,
                    constant_function_view: None,
                };
                row.constant_function_view = Some(view);
                return row;
            } else {
                // Non-constant animated scalar → orange graph row.
                return shader_function_grid_row(parameter.parameter_name.clone(), view);
            }
        }
    }

    let (slot, _) = compile_real_constant(parameter, instance);
    let current = format_shader_float(slot[0]);
    let default_val = format!(
        "value: {}",
        format_shader_float(parameter.default_real_value)
    );

    // Parameter has an instance entry — use the plain real field path.
    if let Some(path) = shader_param_field_path(edit_prefix, param_index, "real") {
        return ShaderGridRow {
            label: parameter.parameter_name.clone(),
            default_cell: Some(shader_default_value_cell(default_val)),
            value_cell: shader_value_cell(format!("value: {current}")),
            fill: material_numeric_row(),
            parameter_type: Some("real".to_owned()),
            function: None,
            edit: Some(ShaderRowEdit {
                path,
                current,
                kind: ShaderRowEditKind::Scalar,
            }),
            context_menu: None,
            create_anim_op: None,
            constant_function_view: None,
        };
    }

    // No instance entry yet — show a text box backed by a create-param op.
    let parameters_block_path = append_field_path(edit_prefix, "parameters");
    let edit = if !parameters_block_path.is_empty() {
        Some(ShaderRowEdit {
            path: String::new(), // sentinel — not a direct field path
            current: format_shader_float(parameter.default_real_value),
            kind: ShaderRowEditKind::CreateScalarParam {
                parameters_block_path,
                parameter_name: parameter.parameter_name.clone(),
                parameter_type_index: shader_parameter_type_index(parameter),
            },
        })
    } else {
        None
    };
    ShaderGridRow {
        label: parameter.parameter_name.clone(),
        default_cell: Some(shader_default_value_cell(default_val.clone())),
        value_cell: shader_value_cell(format!("value: {current}")),
        fill: material_numeric_row(),
        parameter_type: Some("real".to_owned()),
        function: None,
        edit,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

pub(super) fn shader_int_row(
    parameter: &RenderMethodOptionParameter,
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
) -> ShaderGridRow {
    let value = instance
        .map(|param| param.int_parameter)
        .unwrap_or(parameter.default_int_bool_value);
    let mut row = shader_plain_value_row(
        parameter.parameter_name.clone(),
        parameter.default_int_bool_value.to_string(),
        value.to_string(),
        material_data_row(),
        Some("enum".to_owned()),
    );
    row.edit =
        shader_param_field_path(edit_prefix, param_index, "int/bool").map(|path| ShaderRowEdit {
            path,
            current: value.to_string(),
            kind: ShaderRowEditKind::Int,
        });
    row
}

pub(super) fn shader_bool_row(
    parameter: &RenderMethodOptionParameter,
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
) -> ShaderGridRow {
    let raw = instance
        .map(|param| param.int_parameter)
        .unwrap_or(parameter.default_int_bool_value);
    let mut row = shader_plain_value_row(
        parameter.parameter_name.clone(),
        (parameter.default_int_bool_value != 0).to_string(),
        (raw != 0).to_string(),
        material_data_row(),
        Some("bool".to_owned()),
    );
    row.edit = shader_param_field_path(edit_prefix, param_index, "int/bool")
        .map(|path| ShaderRowEdit {
            path,
            current: raw.to_string(),
            kind: ShaderRowEditKind::Bool { create: None },
        })
        .or_else(|| {
            Some(ShaderRowEdit {
                path: String::new(),
                current: raw.to_string(),
                kind: ShaderRowEditKind::Bool {
                    create: Some(ShaderParamCreateTarget {
                        parameters_block_path: append_field_path(edit_prefix, "parameters"),
                        parameter_name: parameter.parameter_name.clone(),
                        parameter_type_index: shader_parameter_type_index(parameter),
                        field: "int/bool",
                    }),
                },
            })
        });
    row
}

pub(super) fn shader_color_row(
    parameter: &RenderMethodOptionParameter,
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
    tag: &TagFile,
) -> ShaderGridRow {
    let (slot, _) = compile_real_constant(parameter, instance);
    let default_color =
        material_color_from_argb(&parameter.parameter_name, parameter.default_color.0);
    let is_argb_parameter = matches!(
        parameter.parameter_type.map(|kind| kind.get()),
        Some(RenderMethodParameterType::ArgbColor)
    );
    let mut raw_color = instance
        .map(|param| argb_to_rgba(param.color_parameter.0))
        .unwrap_or(slot);
    if !is_argb_parameter {
        raw_color[3] = 1.0;
    }
    let color_field_path = shader_param_existing_field_path(tag, edit_prefix, param_index, "color");
    let value_color = MaterialColorPopup::new(
        &parameter.parameter_name,
        raw_color[0],
        raw_color[1],
        raw_color[2],
        raw_color[3],
    );
    let argb = parameter.default_color.0;
    let a8 = ((argb >> 24) & 0xFF) as u8;
    let r8 = ((argb >> 16) & 0xFF) as u8;
    let g8 = ((argb >> 8) & 0xFF) as u8;
    let b8 = (argb & 0xFF) as u8;
    let function_alpha = if is_argb_parameter {
        a8 as f32 / 255.0
    } else {
        1.0
    };
    let default_function_hex = constant_color_function_hex(
        r8 as f32 / 255.0,
        g8 as f32 / 255.0,
        b8 as f32 / 255.0,
        function_alpha,
    );
    let create_target = param_index
        .map(|pidx| {
            existing_shader_function_target(
                edit_prefix,
                pidx,
                RenderMethodAnimatedParameterType::Color as i32,
            )
        })
        .unwrap_or_else(|| {
            new_shader_function_target(
                edit_prefix,
                &parameter.parameter_name,
                shader_parameter_type_index(parameter),
                RenderMethodAnimatedParameterType::Color as i32,
            )
        });
    let create_action = shader_function_action(&create_target, default_function_hex);

    // If a Color animated parameter already exists, use that as the editable
    // backing. Editing the plain fallback color while function data is present
    // leaves Guerilla/the runtime reading the old animated value.
    if let Some(inst) = instance {
        for (j, animated) in inst.animated_parameters.iter().enumerate() {
            if !matches!(
                animated.parameter_type.map(|kind| kind.get()),
                Some(RenderMethodAnimatedParameterType::Color)
            ) {
                continue;
            }
            let Some(ref function) = animated.function else {
                continue;
            };
            let mut view = FunctionView::from_animated(animated, function.clone());
            if let Some(i) = param_index {
                view = view.with_edit(animated_param_paths(edit_prefix, i, j));
            }

            if let Some(mut rgba) = extract_constant_color(function) {
                if !is_argb_parameter {
                    rgba[3] = 1.0;
                }
                // Constant 1-color: show as inline editable color swatch.
                let data_path = view
                    .edit
                    .as_ref()
                    .and_then(|e| e.data.data_field_path().map(str::to_owned))
                    .unwrap_or_default();
                let (block_path, block_index) = match view.edit.as_ref() {
                    Some(e) => (e.block_path.clone(), e.block_index),
                    None => (String::new(), 0),
                };
                let color_val = MaterialColorPopup::new(
                    &parameter.parameter_name,
                    rgba[0],
                    rgba[1],
                    rgba[2],
                    rgba[3],
                );
                let mut row = ShaderGridRow {
                    label: parameter.parameter_name.clone(),
                    default_cell: Some(ShaderGridCell {
                        text: "color: RGB".to_owned(),
                        value_kind: "default",
                        color: Some(default_color),
                    }),
                    value_cell: ShaderGridCell {
                        text: "color: RGB".to_owned(),
                        value_kind: "value",
                        color: Some(color_val),
                    },
                    fill: material_numeric_row(),
                    parameter_type: Some("color".to_owned()),
                    function: None,
                    edit: if data_path.is_empty() {
                        None
                    } else {
                        Some(ShaderRowEdit {
                            path: data_path,
                            current: format!("{},{},{},{}", rgba[0], rgba[1], rgba[2], rgba[3]),
                            kind: ShaderRowEditKind::FunctionColor {
                                block_path,
                                block_index,
                            },
                        })
                    },
                    context_menu: None,
                    create_anim_op: None,
                    constant_function_view: None,
                };
                row.constant_function_view = Some(view);
                return row;
            } else {
                // Non-constant color animated param → orange graph row.
                return shader_function_grid_row(parameter.parameter_name.clone(), view);
            }
        }
    }

    // No Color animated parameter exists. Use the plain shader color field as a
    // solid color backing when this tag layout exposes one.
    if let Some(path) = color_field_path.clone() {
        return ShaderGridRow {
            label: parameter.parameter_name.clone(),
            default_cell: Some(ShaderGridCell {
                text: "color: RGB".to_owned(),
                value_kind: "default",
                color: Some(default_color),
            }),
            value_cell: ShaderGridCell {
                text: "color: RGB".to_owned(),
                value_kind: "value",
                color: Some(value_color),
            },
            fill: material_numeric_row(),
            parameter_type: Some("color".to_owned()),
            function: None,
            edit: Some(ShaderRowEdit {
                path,
                current: format!(
                    "{},{},{},{}",
                    raw_color[0], raw_color[1], raw_color[2], raw_color[3]
                ),
                kind: ShaderRowEditKind::ColorField {
                    argb: is_argb_parameter,
                },
            }),
            context_menu: None,
            create_anim_op: Some(create_action),
            constant_function_view: None,
        };
    }

    // No Color animated parameter — show a solid color swatch. Clicking it
    // creates constant-color backing data; f()+ is available for users who
    // explicitly want to add/open function data.
    ShaderGridRow {
        label: parameter.parameter_name.clone(),
        default_cell: Some(ShaderGridCell {
            text: "color: RGB".to_owned(),
            value_kind: "default",
            color: Some(default_color),
        }),
        value_cell: ShaderGridCell {
            text: "color: RGB".to_owned(),
            value_kind: "value",
            color: Some(value_color),
        },
        fill: material_numeric_row(),
        parameter_type: Some("color".to_owned()),
        function: None,
        edit: color_field_path
            .map(|path| ShaderRowEdit {
                path,
                current: format!("{},{},{},{}", slot[0], slot[1], slot[2], slot[3]),
                kind: ShaderRowEditKind::ColorField {
                    argb: is_argb_parameter,
                },
            })
            .or_else(|| {
                Some(ShaderRowEdit {
                    path: format!("create:{}", parameter.parameter_name),
                    current: format!("{},{},{},{}", slot[0], slot[1], slot[2], slot[3]),
                    kind: ShaderRowEditKind::CreateFunctionColor {
                        target: create_target.clone(),
                    },
                })
            }),
        context_menu: None,
        create_anim_op: Some(create_action),
        constant_function_view: None,
    }
}

pub(super) fn argb_to_rgba(argb: u32) -> [f32; 4] {
    [
        ((argb >> 16) & 0xFF) as f32 / 255.0,
        ((argb >> 8) & 0xFF) as f32 / 255.0,
        (argb & 0xFF) as f32 / 255.0,
        ((argb >> 24) & 0xFF) as f32 / 255.0,
    ]
}

pub(super) fn shader_alpha_row(
    parameter: &RenderMethodOptionParameter,
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
) -> ShaderGridRow {
    let (slot, _) = compile_real_constant(parameter, instance);
    let default_alpha = ((parameter.default_color.0 >> 24) & 0xFF) as f32 / 255.0;
    let current_alpha = slot[3];
    if let Some(inst) = instance {
        for (j, animated) in inst.animated_parameters.iter().enumerate() {
            if !matches!(
                animated.parameter_type.map(|kind| kind.get()),
                Some(RenderMethodAnimatedParameterType::Alpha)
            ) {
                continue;
            }
            let Some(function) = animated.function.clone() else {
                continue;
            };
            let mut view = FunctionView::from_animated(animated, function.clone());
            if let Some(i) = param_index {
                view = view.with_edit(animated_param_paths(edit_prefix, i, j));
            }
            if let Some(const_val) = function.as_constant() {
                let data_path = view
                    .edit
                    .as_ref()
                    .and_then(|e| e.data.data_field_path().map(str::to_owned))
                    .unwrap_or_default();
                let (block_path, block_index) = match view.edit.as_ref() {
                    Some(e) => (e.block_path.clone(), e.block_index),
                    None => (String::new(), 0),
                };
                let current = format_shader_float(const_val);
                let mut row = ShaderGridRow {
                    label: format!("{}_alpha", parameter.parameter_name),
                    default_cell: Some(shader_default_value_cell(format!(
                        "value: {}",
                        format_shader_float(default_alpha)
                    ))),
                    value_cell: shader_value_cell(format!("value: {current}")),
                    fill: material_numeric_row(),
                    parameter_type: Some("alpha".to_owned()),
                    function: None,
                    edit: if data_path.is_empty() {
                        None
                    } else {
                        Some(ShaderRowEdit {
                            path: data_path,
                            current,
                            kind: ShaderRowEditKind::FunctionScalar {
                                block_path,
                                block_index,
                            },
                        })
                    },
                    context_menu: None,
                    create_anim_op: None,
                    constant_function_view: None,
                };
                row.constant_function_view = Some(view);
                return row;
            }
            return shader_function_grid_row(format!("{}_alpha", parameter.parameter_name), view);
        }
    }
    let create_target = param_index
        .map(|pidx| {
            existing_shader_function_target(
                edit_prefix,
                pidx,
                RenderMethodAnimatedParameterType::Alpha as i32,
            )
        })
        .unwrap_or_else(|| {
            new_shader_function_target(
                edit_prefix,
                &parameter.parameter_name,
                shader_parameter_type_index(parameter),
                RenderMethodAnimatedParameterType::Alpha as i32,
            )
        });
    let current = format_shader_float(current_alpha);
    ShaderGridRow {
        label: format!("{}_alpha", parameter.parameter_name),
        default_cell: Some(shader_default_value_cell(format!(
            "value: {}",
            format_shader_float(default_alpha)
        ))),
        value_cell: shader_value_cell(format!("value: {current}")),
        fill: material_numeric_row(),
        parameter_type: Some("alpha".to_owned()),
        function: None,
        edit: Some(ShaderRowEdit {
            path: format!("create:{}_alpha", parameter.parameter_name),
            current,
            kind: ShaderRowEditKind::CreateFunctionScalar {
                target: create_target,
            },
        }),
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

/// Find the first animated parameter whose output kind matches, plus
/// its block index (so callers can build write-back paths). The index
/// is the position in `animated_parameters`, which is 1:1 with the
/// `animated parameters` schema block.
pub(super) fn first_render_method_function_indexed(
    instance: Option<&RenderMethodParameter>,
    matches_kind: impl Fn(RenderMethodAnimatedParameterType) -> bool,
) -> Option<(usize, FunctionView)> {
    instance.and_then(|param| {
        param
            .animated_parameters
            .iter()
            .enumerate()
            .find_map(|(j, animated)| {
                let kind = animated.parameter_type?.get();
                if matches_kind(kind) {
                    animated
                        .function
                        .clone()
                        .map(|function| (j, FunctionView::from_animated(animated, function)))
                } else {
                    None
                }
            })
    })
}

/// As [`first_render_method_function_indexed`] but attaches the edit
/// paths when a parameter index is known.
pub(super) fn first_render_method_function(
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
    matches_kind: impl Fn(RenderMethodAnimatedParameterType) -> bool,
) -> Option<FunctionView> {
    first_render_method_function_indexed(instance, matches_kind).map(
        |(j, view)| match param_index {
            Some(i) => view.with_edit(animated_param_paths(edit_prefix, i, j)),
            None => view,
        },
    )
}

pub(super) fn shader_option_value_row(
    label: String,
    default: String,
    value: String,
) -> ShaderGridRow {
    shader_plain_value_row(
        label,
        default,
        value,
        material_data_row(),
        Some("option".to_owned()),
    )
}

pub(super) fn shader_int_value_row(
    label: String,
    default: String,
    value: String,
    path: String,
) -> ShaderGridRow {
    let mut row = shader_plain_value_row(
        label,
        default,
        value.clone(),
        material_data_row(),
        Some("integer".to_owned()),
    );
    if !path.is_empty() {
        row.edit = Some(ShaderRowEdit {
            path,
            current: value,
            kind: ShaderRowEditKind::Int,
        });
    }
    row
}

pub(super) fn shader_enum_value_row(
    label: String,
    default: String,
    current_index: usize,
    options: Vec<String>,
    path: String,
) -> ShaderGridRow {
    let current = options
        .get(current_index)
        .cloned()
        .unwrap_or_else(|| current_index.to_string());
    let mut row = shader_option_value_row(label, default, current);
    if !path.is_empty() {
        row.edit = Some(ShaderRowEdit {
            path,
            current: current_index.to_string(),
            kind: ShaderRowEditKind::Enum(options),
        });
    }
    row
}

/// Sampler-state filter modes (Guerilla `off_14143B738` order).
pub(super) fn bitmap_filter_option_labels() -> Vec<String> {
    [
        "trilinear",
        "point",
        "bilinear",
        "anisotropic (1)",
        "anisotropic (2) expensive",
        "anisotropic (3) expensive",
        "anisotropic (4) expensive",
        "lightprobe texture array",
        "comparison point",
        "comparison bilinear",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Sampler-state address/wrap modes (Guerilla `off_14143B858` order).
pub(super) fn bitmap_address_option_labels() -> Vec<String> {
    ["wrap", "clamp", "mirror", "black border"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub(super) fn bitmap_extern_option_labels() -> Vec<String> {
    [
        "none",
        "texaccum target",
        "normal target",
        "z target",
        "shadow 1 target",
        "shadow 2 target",
        "shadow 3 target",
        "shadow 4 target",
        "texture camera target",
        "reflection target",
        "refraction target",
        "lightprobe texture",
        "dominant light intensity texture",
        "unused 1",
        "unused 2",
        "change color primary",
        "change color secondary",
        "change color tertiary",
        "change color quaternary",
        "change color quinary",
        "emblem color background",
        "emblem color primary",
        "emblem color secondary",
        "dynamic environment map 1",
        "dynamic environment map 2",
        "cook torrance cc0236",
        "cook torrance dd0236",
        "cook torrance c78d78",
        "light dir 0",
        "light color 0",
        "light dir 1",
        "light color 1",
        "light dir 2",
        "light color 2",
        "light dir 3",
        "light color 3",
        "unused 3",
        "unused 4",
        "unused 5",
        "dynamic light gel 0",
        "flat envmap matrix x",
        "flat envmap matrix y",
        "flat envmap matrix z",
        "debug tint",
        "screen constants",
        "active camo distortion texture",
        "scene ldr texture",
        "scene hdr texture",
        "water memexport addr",
        "tree animation timer",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// A bitmap sampler sub-row backed by an enum dropdown (filter / wrap modes).
/// The underlying tag field is a plain short integer, so the dropdown writes
/// the selected option index.
pub(super) fn shader_sampler_enum_row(
    label: String,
    default_index: i16,
    current_index: i16,
    options: Vec<String>,
    edit_prefix: &str,
    param_index: Option<usize>,
    field: &str,
) -> ShaderGridRow {
    let default_label = options
        .get(default_index.max(0) as usize)
        .cloned()
        .unwrap_or_else(|| default_index.to_string());
    let current_label = options
        .get(current_index.max(0) as usize)
        .cloned()
        .unwrap_or_else(|| current_index.to_string());
    let mut row = shader_option_value_row(label, default_label, current_label);
    row.edit = shader_param_field_path(edit_prefix, param_index, field).map(|path| ShaderRowEdit {
        path,
        current: current_index.to_string(),
        kind: ShaderRowEditKind::Enum(options),
    });
    row
}

pub(super) fn shader_plain_value_row(
    label: String,
    default: String,
    value: String,
    fill: Color32,
    parameter_type: Option<String>,
) -> ShaderGridRow {
    ShaderGridRow {
        label,
        default_cell: Some(ShaderGridCell {
            text: default,
            value_kind: "default",
            color: None,
        }),
        value_cell: ShaderGridCell {
            text: value,
            value_kind: "value",
            color: None,
        },
        fill,
        parameter_type,
        function: None,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

pub(super) fn shader_function_grid_row(label: String, function: FunctionView) -> ShaderGridRow {
    ShaderGridRow {
        label,
        default_cell: Some(ShaderGridCell {
            text: "value: 1.00".to_owned(),
            value_kind: "default",
            color: None,
        }),
        value_cell: ShaderGridCell {
            text: shader_function_grid_text(&function.function),
            value_kind: "value",
            color: None,
        },
        fill: material_function_row(),
        parameter_type: Some("function".to_owned()),
        function: Some(function),
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

pub(super) fn shader_value_cell(text: String) -> ShaderGridCell {
    ShaderGridCell {
        text,
        value_kind: "value",
        color: None,
    }
}

pub(super) fn shader_default_value_cell(text: String) -> ShaderGridCell {
    ShaderGridCell {
        text,
        value_kind: "default",
        color: None,
    }
}

pub(super) fn material_color_from_argb(title: &str, argb: u32) -> MaterialColorPopup {
    let alpha = ((argb >> 24) & 0xFF) as f32 / 255.0;
    MaterialColorPopup::new(
        title,
        ((argb >> 16) & 0xFF) as f32 / 255.0,
        ((argb >> 8) & 0xFF) as f32 / 255.0,
        (argb & 0xFF) as f32 / 255.0,
        if alpha == 0.0 { 1.0 } else { alpha },
    )
}

pub(super) fn none_if_empty(value: &str) -> String {
    if value.is_empty() {
        "NONE".to_owned()
    } else {
        value.to_owned()
    }
}

pub(super) fn format_shader_float(value: f32) -> String {
    let mut text = format!("{value:.4}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.push('0');
    }
    text
}

pub(super) fn shader_function_grid_text(function: &TagFunction) -> String {
    if function.color_graph_type() != ColorGraphType::Scalar {
        let color = function.evaluate_color(0.0, 0.0);
        let prefix = if function.is_constant() {
            "color"
        } else {
            "function color"
        };
        return format!(
            "{prefix}: RGB sc#1, {}, {}, {}",
            format_pc_float(color.red),
            format_pc_float(color.green),
            format_pc_float(color.blue)
        );
    }

    if let Some(value) = function.as_constant() {
        return format!("value: {}", format_shader_float(value));
    }

    match function {
        TagFunction::Identity { .. } => format!("identity: {}", function_sample_summary(function)),
        TagFunction::Constant { header } => {
            if header.flags.is_ranged() {
                format!(
                    "range value: {} to {}",
                    format_shader_float(header.clamp_range_min),
                    format_shader_float(header.clamp_range_max)
                )
            } else {
                format!("value: {}", format_shader_float(header.clamp_range_min))
            }
        }
        TagFunction::Transition { compact, .. } => format!(
            "transition {}: {}",
            compact.function_index,
            function_sample_summary(function)
        ),
        TagFunction::Periodic { compact, .. } => format!(
            "periodic {} freq {} phase {}: {}",
            compact.function_index,
            format_shader_float(compact.frequency),
            format_shader_float(compact.phase),
            function_sample_summary(function)
        ),
        TagFunction::Linear { compact, .. } => format!(
            "linear: {}*x + {} ({})",
            format_shader_float(compact.slope),
            format_shader_float(compact.offset),
            function_sample_summary(function)
        ),
        TagFunction::LinearKey { compact, .. } => {
            format!("curve: {}", function_points_summary(&compact.graph_points))
        }
        TagFunction::MultiLinearKey { compact, .. } => {
            format!(
                "multi curve: {}",
                function_points_summary(&compact.graph_points)
            )
        }
        TagFunction::Spline { compact, .. } => format!(
            "spline: {}, {}, {}, {} ({})",
            format_shader_float(compact.i),
            format_shader_float(compact.j),
            format_shader_float(compact.k),
            format_shader_float(compact.l),
            function_sample_summary(function)
        ),
        TagFunction::Spline2 { compact, .. } => format!(
            "spline2: x {} width {} bias {} ({})",
            format_shader_float(compact.left_x),
            format_shader_float(compact.width),
            format_shader_float(compact.bias),
            function_sample_summary(function)
        ),
        TagFunction::MultiSpline { compact, .. } => format!(
            "multi-part curve: {} segment{} ({})",
            compact.parts.len(),
            if compact.parts.len() == 1 { "" } else { "s" },
            function_sample_summary(function)
        ),
        TagFunction::Exponent { compact, .. } => format!(
            "exponent: {} to {}, pow {} ({})",
            format_shader_float(compact.amplitude_min),
            format_shader_float(compact.amplitude_max),
            format_shader_float(compact.exponent),
            function_sample_summary(function)
        ),
        TagFunction::Unsupported { header, raw } => format!(
            "{:?}: {} bytes",
            header.function_type,
            raw.len().saturating_sub(32)
        ),
    }
}

pub(super) fn function_sample_summary(function: &TagFunction) -> String {
    let low = function.evaluate(0.0, 0.0);
    let mid = function.evaluate(0.5, 0.5);
    let high = function.evaluate(1.0, 1.0);
    if (low - mid).abs() < 0.0001 && (mid - high).abs() < 0.0001 {
        format_shader_float(low)
    } else {
        format!(
            "{} -> {} -> {}",
            format_shader_float(low),
            format_shader_float(mid),
            format_shader_float(high)
        )
    }
}

pub(super) fn function_points_summary(points: &[(f32, f32); 4]) -> String {
    points
        .iter()
        .map(|(x, y)| format!("({}, {})", format_shader_float(*x), format_shader_float(*y)))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn draw_shader_editor_model(
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

pub(super) fn draw_shader_category_row(
    ui: &mut Ui,
    category: &ShaderEditorCategory,
    edit: &mut FieldEditContext<'_>,
) {
    let available = ui.available_width().max(780.0);
    let label_width = 230.0;
    let value_width = (available - label_width - 24.0).max(240.0);
    let height = 25.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(available, height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, material_data_row());
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
        material_text(),
    );

    let combo_rect = egui::Rect::from_min_size(
        label_rect.right_top() + Vec2::new(8.0, 2.0),
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
            egui::ComboBox::from_id_salt((
                edit.view_scope,
                edit.tag_key,
                "shader_category",
                category.index,
            ))
            .selected_text(selected_text)
            .width(value_width)
            .show_ui(ui, |ui| {
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
            });
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

pub(super) fn draw_material_template_summary(
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
            function: None,
            edit: None,
            context_menu: None,
            create_anim_op: None,
            constant_function_view: None,
        };
        draw_shader_grid_row_readonly(ui, &row, 0, color_popup, &mut no_function_popup);
    }
}

pub(super) fn collect_shader_template_references(
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

pub(super) fn shader_template_label(key: &str) -> String {
    match key {
        "material shader" => "material shader".to_owned(),
        "shader template" => "shader template".to_owned(),
        "definition" => "shader definition".to_owned(),
        _ => key.to_owned(),
    }
}

pub(super) fn is_shader_template_reference_key(key: &str) -> bool {
    matches!(key, "material shader" | "shader template" | "definition")
}

pub(super) fn draw_material_parameters_block(
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

pub(super) fn shader_grid_row_from_parameter(
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
        function,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

/// Render a shader grid row with no edit capability (used by the read-only
/// `.material` / `.material_shader` views, which have no edit context).
pub(super) fn draw_shader_grid_row_readonly(
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
    let mut model_variant_ops = Vec::new();
    let mut block_confirm = None;
    let mut open_request = None;
    let mut tool_import = None;
    let mut bitmap_reimport = None;
    let mut buffers = HashMap::new();
    let mut color_request = None;
    let mut function_request = None;
    let mut block_clip_request = None;
    let mut ctx = FieldEditContext {
        view_scope: "readonly",
        tag_key: "",
        group_tag: 0,
        tags_root: None,
        editable: false,
        show_block_sizes: false,
        buffers: &mut buffers,
        pending: &mut pending,
        block_ops: &mut block_ops,
        block_confirm: &mut block_confirm,
        open_request: &mut open_request,
        tool_import: &mut tool_import,
        bitmap_reimport: &mut bitmap_reimport,
        shader_ops: &mut shader_ops,
        shader_param_ops: &mut shader_param_ops,
        model_variant_ops: &mut model_variant_ops,
        color_request: &mut color_request,
        function_request: &mut function_request,
        block_clipboard: None,
        block_clip_request: &mut block_clip_request,
        field_filter: None,
    };
    draw_shader_grid_row(ui, row, depth, color_popup, function_popup, &mut ctx);
}

pub(super) fn shader_cell_from_material_value(value: &MaterialParameterValue) -> ShaderGridCell {
    ShaderGridCell {
        text: shader_grid_value_text(value),
        value_kind: value.value_kind,
        color: value.color.clone(),
    }
}

pub(super) fn shader_grid_value_text(value: &MaterialParameterValue) -> String {
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

pub(super) fn shader_default_cell(parameter_type: Option<&str>) -> Option<ShaderGridCell> {
    let parameter_type = parameter_type?;
    Some(ShaderGridCell {
        text: parameter_type.to_owned(),
        value_kind: "default",
        color: None,
    })
}

pub(super) fn draw_shader_grid_section_header(ui: &mut Ui, title: &str) {
    let available = ui.available_width().max(640.0);
    let height = 22.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(available, height), Sense::hover());
    ui.painter()
        .rect_filled(rect, 0.0, material_section_header());
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, material_grid_light()),
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(4.0, 0.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.0),
        material_text(),
    );
}

pub(super) fn draw_shader_flags_row(
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
    ui.painter().rect_filled(rect, 0.0, material_data_row());
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
        material_text(),
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
            material_text(),
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

pub(super) fn draw_shader_grid_row(
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
    let label_width = (230.0_f32 - indent).max(150.0);
    let default_width = 110.0;
    // Orange function rows: range checkbox + "f()" + "×" delete = ~132px.
    // Constant-function scalar rows: "f()" + "×" = ~52px.
    // All other rows: 18px placeholder keeps value_width stable.
    let function_width = if row.function.is_some() {
        132.0
    } else if row.constant_function_view.is_some() {
        52.0
    } else {
        18.0
    };
    let value_width =
        (available - indent - label_width - default_width - function_width - 12.0).max(240.0);
    let height = 25.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(available, height), Sense::click());
    ui.painter().rect_filled(rect, 0.0, row.fill);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, material_grid_light()),
    );

    let label_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(4.0 + indent, 0.0),
        Vec2::new(label_width, height),
    );
    ui.painter().text(
        label_rect.right_center() - Vec2::new(6.0, 0.0),
        Align2::RIGHT_CENTER,
        truncate_for_cell(&row.label, label_width - 12.0),
        FontId::proportional(12.5),
        material_text(),
    );

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

    let value_rect = egui::Rect::from_min_size(
        default_rect.right_top() + Vec2::new(6.0, 0.0),
        Vec2::new(value_width, height - 4.0),
    );

    // Editable value cell when the row carries an edit path and the tag is
    // writable; otherwise the read-only painted cell.
    if let (true, Some(row_edit)) = (editable, row.edit.as_ref()) {
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

    if let Some(function) = row.function.as_ref() {
        // Orange function row: range: checkbox + f() button + × delete button.
        let range_rect = egui::Rect::from_min_size(
            value_rect.right_top() + Vec2::new(8.0, 2.0),
            Vec2::new(64.0, height - 8.0),
        );
        let check_rect = egui::Rect::from_min_size(
            range_rect.left_top() + Vec2::new(0.0, 2.0),
            Vec2::splat(14.0),
        );
        ui.painter().rect_filled(check_rect, 0.0, material_input());
        ui.painter()
            .rect_stroke(check_rect, 0.0, Stroke::new(1.0, material_input_edge()));
        ui.painter().text(
            check_rect.right_center() + Vec2::new(4.0, 0.0),
            Align2::LEFT_CENTER,
            "range:",
            FontId::proportional(12.0),
            material_text(),
        );

        let button_rect = egui::Rect::from_min_size(
            range_rect.right_top() + Vec2::new(4.0, -1.0),
            Vec2::new(28.0, height - 4.0),
        );
        ui.painter().rect_filled(button_rect, 0.0, material_input());
        ui.painter()
            .rect_stroke(button_rect, 0.0, Stroke::new(1.0, material_input_edge()));
        ui.painter().text(
            button_rect.center(),
            Align2::CENTER_CENTER,
            "f()",
            FontId::proportional(12.0),
            material_text(),
        );

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
        if editable {
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
            value_rect.right_top() + Vec2::new(4.0, 2.0),
            Vec2::new(26.0, height - 4.0),
        );
        ui.painter().rect_filled(f_rect, 0.0, material_input());
        ui.painter()
            .rect_stroke(f_rect, 0.0, Stroke::new(1.0, material_input_edge()));
        ui.painter().text(
            f_rect.center(),
            Align2::CENTER_CENTER,
            "f()",
            FontId::proportional(11.0),
            material_text(),
        );
        if ui
            .interact(
                f_rect,
                ui.make_persistent_id(format!("shader_cfn_open:{}", row.label)),
                Sense::click(),
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

        if editable {
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
            value_rect.right_top() + Vec2::new(12.0, 2.0),
            Vec2::new(34.0, height - 4.0),
        );
        ui.painter().rect_filled(button_rect, 0.0, material_input());
        ui.painter()
            .rect_stroke(button_rect, 0.0, Stroke::new(1.0, material_input_edge()));
        ui.painter().text(
            button_rect.center(),
            Align2::CENTER_CENTER,
            "f()+",
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
        if let (true, Some(menu)) = (editable, row.context_menu.as_ref()) {
            if !menu.items.is_empty() {
                response.context_menu(|ui| {
                    ui.label("Add optional argument:");
                    ui.separator();
                    for item in &menu.items {
                        if ui.button(&item.label).clicked() {
                            push_shader_context_action(edit, &item.action);
                            ui.close_menu();
                        }
                    }
                });
            }
        }
        if let Some(parameter_type) = row.parameter_type.as_deref() {
            response.on_hover_text(parameter_type);
        }
    }
}

/// Render an editable widget inside a shader grid value cell and push a
/// `PendingFieldEdit` on commit. The leaf field type drives parsing in
/// `apply_field_edit`, so scalars/ints/refs all just emit the text.
pub(super) fn draw_shader_editable_value(
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
                egui::ComboBox::from_id_salt((
                    edit.view_scope,
                    edit.tag_key,
                    &buffer_key,
                    "shader_enum",
                ))
                .selected_text(selected_text)
                .width(rect.width())
                .show_ui(ui, |ui| {
                    for (i, opt) in options.iter().enumerate() {
                        if ui.selectable_label(i == current_idx, opt).clicked() {
                            chosen = Some(i);
                        }
                    }
                });
            });
            if let Some(i) = chosen {
                edit.pending.push(PendingFieldEdit {
                    path: row_edit.path.clone(),
                    input: i.to_string(),
                });
            }
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
            let text_rect = egui::Rect::from_min_size(
                rect.left_top(),
                Vec2::new((rect.width() - 72.0).max(40.0), rect.height()),
            );
            // Open the referenced bitmap in a new tab (when the ref is set).
            // The grid stores the path with a ".bitmap" suffix and forward
            // slashes; strip both so it resolves like a normal tag reference.
            let cleaned = sanitize_ref_path(&current);
            let open_ref = cleaned
                .strip_suffix(".bitmap")
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
            ui.painter().text(
                open_rect.center(),
                Align2::CENTER_CENTER,
                "Open",
                FontId::proportional(11.0),
                material_text(),
            );
            if open_enabled
                && ui
                    .interact(
                        open_rect,
                        ui.make_persistent_id(format!("shader_bitmap_open:{}", buffer_key)),
                        Sense::click(),
                    )
                    .on_hover_text("Open the referenced bitmap tag")
                    .clicked()
            {
                *edit.open_request = Some(OpenTagRequest {
                    group_tag: *group_tag,
                    rel_path: open_ref.clone(),
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
            let mut commit = None;
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
                    commit = Some(buffer.trim().to_owned());
                }
            });
            if let Some(input) = commit {
                push_shader_value_edit(edit, row_edit, create.as_ref(), input);
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
                    // Normalise to forward slashes and strip .bitmap extension
                    // so the value matches the tag-reference path format.
                    let rel = normalize_bitmap_browse_path(&path, edit.tags_root);
                    // Update the text buffer so the row shows the chosen path.
                    let buf = edit.buffers.entry(buffer_key).or_insert_with(String::new);
                    *buf = rel.clone();
                    push_shader_value_edit(edit, row_edit, create.as_ref(), rel);
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

pub(super) fn draw_shader_color_swatch(ui: &mut Ui, rect: egui::Rect, color: Color32) {
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

pub(super) fn push_shader_value_edit(
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

pub(super) fn shader_color_create_popup(
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
/// tag-reference path format used inside shader tags: forward-slash separated,
/// `.bitmap` extension stripped, and relative to the first `tags` directory
/// ancestor when possible. Falls back to the bare stem if no `tags` root is
/// found.
pub(super) fn normalize_bitmap_browse_path(
    path: &std::path::Path,
    tags_root: Option<&std::path::Path>,
) -> String {
    use blam_tags::paths::derive_tags_root;
    if let Some(root) = tags_root {
        if let Ok(rel) = path.strip_prefix(root) {
            let without_ext = rel.with_extension("");
            return without_ext.to_string_lossy().replace('\\', "/");
        }
    }
    // Try to find the tags root and build a relative path from it.
    if let Some(root) = derive_tags_root(path) {
        if let Ok(rel) = path.strip_prefix(&root) {
            let without_ext = rel.with_extension("");
            return without_ext.to_string_lossy().replace('\\', "/");
        }
    }
    // Fall back: use the file stem (no directory, no extension).
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub(super) fn draw_shader_grid_cell(
    ui: &mut Ui,
    rect: egui::Rect,
    cell: Option<&ShaderGridCell>,
    id_source: &str,
    color_popup: &mut Option<MaterialColorPopup>,
) {
    let (fill, text_color) = match cell.map(|cell| cell.value_kind) {
        Some("default") | None => (material_default_box(), material_muted_text()),
        _ => (material_input(), material_text()),
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

pub(super) fn material_parameter_name(
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

pub(super) fn material_parameter_type(
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

pub(super) fn material_parameter_section(label: &str) -> &'static str {
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

pub(super) fn material_parameter_values(
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

pub(super) fn find_first_function(tag_struct: TagStruct<'_>) -> Option<FunctionView> {
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
