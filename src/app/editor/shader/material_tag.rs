//! The editable grid for a `material` (`mat `): Halo 4's, Halo 2
//! Anniversary's and Halo 5's.
//!
//! A material names its `material_shader` and overrides some of that
//! shader's parameters by name, as a render method overrides its options'.
//! So it gets the render method's grid: one row per parameter the shader
//! declares, its default beside the material's value, and an override made
//! by writing to it and cleared by deleting it. The rows and their edits
//! are the render method's own: the parameter blocks differ only in name
//! (`material parameters`, and `function parameters` for the functions,
//! which `apply_one_shader_op` resolves by the parameter's schema).
//!
//! What the material holds is what the game reads: neither tag ships a baked
//! postprocess (6977 of 6977 Halo 4 kit materials have an empty one), and
//! the game builds it from these parameters at load (blam-tags
//! `material::bake`, after `c_material::postprocess`). The bake takes the
//! material's value, then lets its functions replace it, so a row with a
//! function edits the function, as the render method grid's do.

use super::*;
use blam_tags::material::{MaterialFunctionType, MaterialParameterType};

/// `material parameters`, the block a material's overrides live in.
const PARAMETERS: &str = "material parameters";

/// `material_shader_parameter_type_enum` index of `kind`, as the tag
/// stores it: the order of [`MaterialParameterType::ALL`].
fn parameter_type_index(kind: MaterialParameterType) -> i32 {
    MaterialParameterType::ALL
        .iter()
        .position(|candidate| *candidate == kind)
        .unwrap_or(0) as i32
}

/// `material_animated_parameter_type_enum` index of `kind`. Its first nine
/// names match the render method's `animated parameter type` in order, which
/// is what the function editor's output list is.
fn function_type_index(kind: MaterialFunctionType) -> i32 {
    kind as i32
}

/// One parameter of a material or its shader, read with its element index.
struct Parameter {
    index: usize,
    parsed: blam_tags::material::MaterialParameter,
    /// `display name`, `display group` and `display help text`: the shader's
    /// labels for it. Empty on a material's own copies.
    display_name: String,
    display_group: String,
}

fn display_text(element: &TagStruct<'_>, field: &str) -> String {
    element
        .field(field)
        .and_then(|field| field.as_data())
        .map(|bytes| String::from_utf8_lossy(bytes).trim_matches(char::from(0)).trim().to_owned())
        .unwrap_or_default()
}

/// The `material parameters` of `root`, with their element indices, as
/// `parsed` (blam-tags' reading of the same block) has them. blam-tags
/// leaves out an element whose `parameter type` it can't read, so the same
/// test pairs the rest with their indices.
fn read_parameters(root: &TagStruct<'_>, parsed: Vec<blam_tags::material::MaterialParameter>) -> Vec<Parameter> {
    let Some(block) = root.field(PARAMETERS).and_then(|field| field.as_block()) else {
        return Vec::new();
    };
    let readable = block.iter().enumerate().filter(|(_, element)| {
        element
            .read_int_any("parameter type")
            .and_then(|kind| usize::try_from(kind).ok())
            .is_some_and(|kind| kind < MaterialParameterType::ALL.len())
    });
    readable
        .zip(parsed)
        .map(|((index, element), parsed)| Parameter {
            index,
            parsed,
            display_name: display_text(&element, "display name"),
            display_group: display_text(&element, "display group"),
        })
        .collect()
}

/// A material's grid, or `None` when `tag` isn't a material.
pub(in crate::app) fn build_material_tag_editor_model(
    tag: &TagFile,
    group_tag: u32,
    source: Option<&TagSource>,
    templates: &mut H2TemplateCache,
) -> Option<ShaderEditorModel> {
    if group_tag != u32::from_be_bytes(*b"mat ") {
        return None;
    }
    let root = tag.root();
    let mut shader_path = root.read_tag_ref_path("material shader").unwrap_or_default();
    // A Halo 5 tag names what it references by tag id, which reads as a
    // placeholder: id 0 is none, any other is a tag no path can be found
    // for here.
    let unresolved = shader_path.starts_with("<unresolved tag");
    if shader_path == "<unresolved tag 0x00000000>" {
        shader_path.clear();
    }
    let shader_tag = (!shader_path.is_empty() && !unresolved)
        .then(|| source.and_then(|source| templates.get_tag(source, &shader_path, "material_shader", *b"mats")))
        .flatten();
    let declared = shader_tag
        .as_ref()
        .map(|shader| {
            read_parameters(&shader.root(), blam_tags::material::MaterialShader::from_tag(shader).parameters)
        })
        .unwrap_or_default();
    let overrides = read_parameters(&root, blam_tags::material::Material::from_struct(&root).parameters);

    let mut top_rows = vec![material_shader_row(&shader_path)];
    if !shader_path.is_empty() && shader_tag.is_none() {
        let why = if unresolved {
            "named by a tag id this source can't open: its parameters can't be listed"
        } else {
            "did not load: its parameters can't be listed"
        };
        top_rows.push(shader_plain_value_row("material shader".to_owned(), String::new(), why.to_owned(), Some("note".to_owned())));
    }

    // Sections in the shader's order, named for their display groups.
    let mut sections: Vec<ShaderEditorSection> = Vec::new();
    for parameter in &declared {
        let title = if parameter.display_group.is_empty() {
            "PARAMETERS".to_owned()
        } else {
            parameter.display_group.to_uppercase()
        };
        let own = overrides.iter().find(|own| own.parsed.name == parameter.parsed.name);
        let rows = parameter_rows(parameter, own);
        match sections.iter_mut().find(|section| section.title == title) {
            Some(section) => section.rows.extend(rows),
            None => sections.push(ShaderEditorSection { title, option_name: String::new(), rows }),
        }
    }

    // Overrides of parameters the shader doesn't declare: the game ignores
    // them, and Clear removes them.
    let unused_parameters = overrides
        .iter()
        .filter(|own| !declared.iter().any(|parameter| parameter.parsed.name == own.parsed.name))
        .map(|own| {
            let mut rows = vec![shader_plain_value_row(
                own.parsed.name.clone(),
                String::new(),
                describe_value(&own.parsed),
                Some(type_label(own.parsed.kind).to_owned()),
            )];
            mark_unused_shader_rows(&mut rows);
            UnusedShaderParameter {
                name: own.parsed.name.clone(),
                category: None,
                rows,
                delete: BlockOp {
                    path: PARAMETERS.to_owned(),
                    kind: BlockOpKind::Delete(own.index),
                },
            }
        })
        .collect();

    sections.push(ShaderEditorSection {
        title: "MATERIAL PROPERTIES".to_owned(),
        option_name: String::new(),
        rows: property_rows(&root),
    });

    Some(ShaderEditorModel {
        has_material_row: false,
        materials: Vec::new(),
        definition_path: String::new(),
        definition_edit_path: String::new(),
        shader_template_path: None,
        shader_template_edit_path: String::new(),
        categories: Vec::new(),
        unused_parameters,
        top_rows,
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

/// The `material shader` reference: what the material is, so it is edited
/// as the material's first row, with no expert-mode gate. Re-pointing it
/// leaves the overrides as they are, as Foundation does a render method's
/// definition; those the new shader doesn't declare show as unused.
fn material_shader_row(shader_path: &str) -> ShaderGridRow {
    let shown = if shader_path.is_empty() {
        "NONE".to_owned()
    } else {
        format!("{shader_path}.material_shader")
    };
    ShaderGridRow {
        label: "material shader".to_owned(),
        default_cell: None,
        value_cell: shader_value_cell(shown),
        parameter_type: Some("tag reference".to_owned()),
        is_overridden: true,
        function: None,
        edit: Some(ShaderRowEdit {
            path: "material shader".to_owned(),
            current: shader_path.to_owned(),
            kind: ShaderRowEditKind::StructuralRef {
                group_tag: u32::from_be_bytes(*b"mats"),
                extension: "material_shader",
            },
        }),
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

fn type_label(kind: MaterialParameterType) -> &'static str {
    match kind {
        MaterialParameterType::Bitmap => "bitmap",
        MaterialParameterType::Real => "real",
        MaterialParameterType::Int => "int",
        MaterialParameterType::Bool => "bool",
        MaterialParameterType::Color => "color",
    }
}

fn describe_value(parameter: &blam_tags::material::MaterialParameter) -> String {
    match parameter.kind {
        MaterialParameterType::Bitmap => parameter
            .bitmap
            .clone()
            .unwrap_or_else(|| none_if_empty(&parameter.bitmap_path)),
        MaterialParameterType::Real => format!("value: {}", format_shader_float(parameter.real)),
        MaterialParameterType::Int => parameter.int_bool.to_string(),
        MaterialParameterType::Bool => (parameter.int_bool != 0).to_string(),
        MaterialParameterType::Color => "color: RGB".to_owned(),
    }
}

/// The path of `field` of override `index`.
fn field_path(index: usize, field: &str) -> String {
    format!("{PARAMETERS}[{index}]/{}", escape_field_path_segment(field))
}

/// The function editor's write targets for function `function` of override
/// `index`.
fn function_paths(index: usize, function: usize) -> FunctionEditPaths {
    let block_path = format!("{PARAMETERS}[{index}]/function parameters");
    let base = format!("{block_path}[{function}]");
    FunctionEditPaths {
        data: FunctionDataStorage::DataField(format!("{base}/function/data")),
        parameter_type: format!("{base}/type"),
        input_name: format!("{base}/input name"),
        range_name: format!("{base}/range name"),
        time_period: format!("{base}/time period"),
        block_path,
        block_index: function,
    }
}

fn function_view(
    function: &blam_tags::material::MaterialFunctionParameter,
    curve: TagFunction,
    paths: FunctionEditPaths,
) -> FunctionView {
    let mut view = FunctionView::from_function(curve);
    view.input_name = function.input_name.clone();
    view.range_name = function.range_name.clone();
    view.output_index = Some(function_type_index(function.kind));
    view.time_period_in_seconds = function.time_period;
    view.with_edit(paths)
}

/// What creates a function of `kind` on the parameter: on its override when
/// it has one, else on a new override.
fn function_target(
    parameter: &Parameter,
    own: Option<&Parameter>,
    kind: MaterialFunctionType,
) -> ShaderFunctionCreateTarget {
    match own {
        Some(own) => ShaderFunctionCreateTarget::ExistingParameter {
            animated_block_path: format!("{PARAMETERS}[{}]/function parameters", own.index),
            output_type_index: function_type_index(kind),
        },
        None => ShaderFunctionCreateTarget::NewParameter {
            parameters_block_path: PARAMETERS.to_owned(),
            parameter_name: parameter.parsed.name.clone(),
            parameter_type_index: parameter_type_index(parameter.parsed.kind),
            output_type_index: function_type_index(kind),
        },
    }
}

/// A new override of `parameter` with `field` set, made on commit.
fn create_target(parameter: &Parameter, field: &'static str) -> ShaderParamCreateTarget {
    ShaderParamCreateTarget {
        parameters_block_path: PARAMETERS.to_owned(),
        parameter_name: parameter.parsed.name.clone(),
        parameter_type_index: parameter_type_index(parameter.parsed.kind),
        field,
    }
}

/// The rows of one declared parameter: its value, then any function of its
/// override the value row doesn't stand for (a bitmap's scale or offset, an
/// alpha), each as a function row.
fn parameter_rows(parameter: &Parameter, own: Option<&Parameter>) -> Vec<ShaderGridRow> {
    let label = if parameter.display_name.is_empty() {
        parameter.parsed.name.clone()
    } else {
        parameter.display_name.clone()
    };
    let value_function = match parameter.parsed.kind {
        MaterialParameterType::Real => Some(MaterialFunctionType::Value),
        MaterialParameterType::Color => Some(MaterialFunctionType::Color),
        _ => None,
    };
    let mut rows = vec![value_row(&label, parameter, own, value_function)];
    if parameter.parsed.kind == MaterialParameterType::Bitmap
        && let Some(own) = own
    {
        rows.extend(transform_rows(&label, own));
    }
    if let Some(own) = own {
        let shown_by_value = own
            .parsed
            .functions
            .iter()
            .position(|function| Some(function.kind) == value_function);
        for (index, function) in own.parsed.functions.iter().enumerate() {
            if Some(index) == shown_by_value {
                continue;
            }
            let Some(curve) = function.function.clone() else {
                continue;
            };
            let view = function_view(function, curve, function_paths(own.index, index));
            rows.push(shader_function_grid_row(
                format!("{label} {}", function_kind_label(function.kind)),
                view,
            ));
        }
    }
    rows
}

fn function_kind_label(kind: MaterialFunctionType) -> &'static str {
    match kind {
        MaterialFunctionType::Value => "value",
        MaterialFunctionType::Color => "color",
        MaterialFunctionType::ScaleUniform => "scale uniform",
        MaterialFunctionType::ScaleU => "scale u",
        MaterialFunctionType::ScaleV => "scale v",
        MaterialFunctionType::OffsetU => "offset u",
        MaterialFunctionType::OffsetV => "offset v",
        MaterialFunctionType::FrameIndex => "frame index",
        MaterialFunctionType::Alpha => "alpha",
    }
}

/// A bitmap override's transform, as the bake reads it: `real` and `vector`
/// are scale u, then scale v, offset u and offset v.
fn transform_rows(label: &str, own: &Parameter) -> [ShaderGridRow; 2] {
    let scale = format_shader_float(own.parsed.real);
    let mut scale_u = shader_plain_value_row(
        format!("{label} scale u"),
        String::new(),
        scale.clone(),
        Some("real".to_owned()),
    );
    scale_u.is_overridden = true;
    scale_u.edit = Some(ShaderRowEdit { path: field_path(own.index, "real"), current: scale, kind: ShaderRowEditKind::Scalar });
    let [i, j, k] = own.parsed.vector;
    let vector = format!("{}, {}, {}", format_shader_float(i), format_shader_float(j), format_shader_float(k));
    let mut rest = shader_plain_value_row(
        format!("{label} scale v, offset u, offset v"),
        String::new(),
        vector.clone(),
        Some("vector".to_owned()),
    );
    rest.is_overridden = true;
    // A text box: the field parser reads the three components.
    rest.edit = Some(ShaderRowEdit { path: field_path(own.index, "vector"), current: vector, kind: ShaderRowEditKind::StringId });
    [scale_u, rest]
}

/// The row that shows and edits a parameter's value.
fn value_row(
    label: &str,
    parameter: &Parameter,
    own: Option<&Parameter>,
    value_function: Option<MaterialFunctionType>,
) -> ShaderGridRow {
    let default = &parameter.parsed;
    // A function standing for the value replaces it in the bake, so it is
    // what the row edits.
    if let (Some(own), Some(kind)) = (own, value_function)
        && let Some((index, function)) =
            own.parsed.functions.iter().enumerate().find(|(_, function)| function.kind == kind)
        && let Some(curve) = function.function.clone()
    {
        return function_value_row(label, default, own.index, index, function, curve);
    }
    match default.kind {
        MaterialParameterType::Real => {
            let current = own.map_or(default.real, |own| own.parsed.real);
            let mut row = shader_plain_value_row(
                label.to_owned(),
                format!("value: {}", format_shader_float(default.real)),
                format!("value: {}", format_shader_float(current)),
                Some("real".to_owned()),
            );
            row.is_overridden = own.is_some();
            row.edit = Some(match own {
                Some(own) => ShaderRowEdit {
                    path: field_path(own.index, "real"),
                    current: format_shader_float(current),
                    kind: ShaderRowEditKind::Scalar,
                },
                None => ShaderRowEdit {
                    path: String::new(),
                    current: format_shader_float(default.real),
                    kind: ShaderRowEditKind::CreateScalarParam {
                        parameters_block_path: PARAMETERS.to_owned(),
                        parameter_name: default.name.clone(),
                        parameter_type_index: parameter_type_index(default.kind),
                        field: "real",
                    },
                },
            });
            row.create_anim_op = Some(shader_function_action(
                &function_target(parameter, own, MaterialFunctionType::Value),
                constant_function_hex(current),
            ));
            row
        }
        MaterialParameterType::Int => {
            let current = own.map_or(default.int_bool, |own| own.parsed.int_bool);
            let mut row = shader_plain_value_row(
                label.to_owned(),
                default.int_bool.to_string(),
                current.to_string(),
                Some("int".to_owned()),
            );
            row.is_overridden = own.is_some();
            row.edit = Some(match own {
                Some(own) => ShaderRowEdit {
                    path: field_path(own.index, "int/bool"),
                    current: current.to_string(),
                    kind: ShaderRowEditKind::Int,
                },
                None => ShaderRowEdit {
                    path: String::new(),
                    current: default.int_bool.to_string(),
                    kind: ShaderRowEditKind::CreateScalarParam {
                        parameters_block_path: PARAMETERS.to_owned(),
                        parameter_name: default.name.clone(),
                        parameter_type_index: parameter_type_index(default.kind),
                        field: "int/bool",
                    },
                },
            });
            row
        }
        MaterialParameterType::Bool => {
            let current = own.map_or(default.int_bool, |own| own.parsed.int_bool);
            let mut row = shader_plain_value_row(
                label.to_owned(),
                (default.int_bool != 0).to_string(),
                (current != 0).to_string(),
                Some("bool".to_owned()),
            );
            row.is_overridden = own.is_some();
            row.edit = Some(ShaderRowEdit {
                path: own.map(|own| field_path(own.index, "int/bool")).unwrap_or_default(),
                current: current.to_string(),
                kind: ShaderRowEditKind::Bool {
                    create: own.is_none().then(|| create_target(parameter, "int/bool")),
                },
            });
            row
        }
        MaterialParameterType::Bitmap => {
            let shown = |parameter: &blam_tags::material::MaterialParameter| {
                parameter
                    .bitmap
                    .clone()
                    .unwrap_or_else(|| none_if_empty(&parameter.bitmap_path))
            };
            let current = own.map_or_else(|| shown(default), |own| shown(&own.parsed));
            let mut row = shader_plain_value_row(
                label.to_owned(),
                shown(default),
                current,
                Some("bitmap".to_owned()),
            );
            row.is_overridden = own.is_some();
            row.edit = Some(ShaderRowEdit {
                path: own.map(|own| field_path(own.index, "bitmap")).unwrap_or_default(),
                current: own
                    .and_then(|own| own.parsed.bitmap.clone())
                    .or_else(|| default.bitmap.clone())
                    .unwrap_or_default(),
                kind: ShaderRowEditKind::BitmapRef {
                    group_tag: u32::from_be_bytes(*b"bitm"),
                    create: own.is_none().then(|| create_target(parameter, "bitmap")),
                },
            });
            row
        }
        MaterialParameterType::Color => {
            // Stored alpha, red, green, blue.
            let rgba = |color: [f32; 4]| [color[1], color[2], color[3], color[0]];
            let default_rgba = rgba(default.color);
            let current = own.map_or(default_rgba, |own| rgba(own.parsed.color));
            let swatch = |rgba: [f32; 4]| {
                MaterialColorPopup::new(&default.name, rgba[0], rgba[1], rgba[2], rgba[3])
                    .with_alpha_available(true)
            };
            let current_text = format!("{},{},{},{}", current[0], current[1], current[2], current[3]);
            let create_action = shader_function_action(
                &function_target(parameter, own, MaterialFunctionType::Color),
                constant_color_function_hex(current[0], current[1], current[2], current[3]),
            );
            ShaderGridRow {
                label: label.to_owned(),
                default_cell: Some(ShaderGridCell {
                    text: "color: RGB".to_owned(),
                    value_kind: "default",
                    color: Some(swatch(default_rgba)),
                }),
                value_cell: ShaderGridCell {
                    text: "color: RGB".to_owned(),
                    value_kind: "value",
                    color: Some(swatch(current)),
                },
                parameter_type: Some("color".to_owned()),
                is_overridden: own.is_some(),
                function: None,
                edit: Some(match own {
                    Some(own) => ShaderRowEdit {
                        path: field_path(own.index, "color"),
                        current: current_text,
                        kind: ShaderRowEditKind::ColorField { argb: true },
                    },
                    // A new override is made with a constant colour function,
                    // as the kit's own materials carry their colours.
                    None => ShaderRowEdit {
                        path: format!("create:{}", default.name),
                        current: current_text,
                        kind: ShaderRowEditKind::CreateFunctionColor {
                            target: function_target(parameter, own, MaterialFunctionType::Color),
                        },
                    },
                }),
                context_menu: None,
                create_anim_op: Some(create_action),
                constant_function_view: None,
            }
        }
    }
}

/// A value row backed by function `function` of override `index`: a
/// constant one edits in place, any other is a function row.
fn function_value_row(
    label: &str,
    default: &blam_tags::material::MaterialParameter,
    index: usize,
    function_index: usize,
    function: &blam_tags::material::MaterialFunctionParameter,
    curve: TagFunction,
) -> ShaderGridRow {
    let paths = function_paths(index, function_index);
    let data_path = format!("{}[{function_index}]/function/data", paths.block_path);
    let block_path = paths.block_path.clone();
    let view = function_view(function, curve.clone(), paths);
    let is_color = function.kind == MaterialFunctionType::Color;
    if is_color && let Some(rgba) = extract_constant_color(&curve) {
        let swatch = |rgba: [f32; 4]| {
            MaterialColorPopup::new(&default.name, rgba[0], rgba[1], rgba[2], rgba[3]).with_alpha_available(true)
        };
        let default_rgba = [default.color[1], default.color[2], default.color[3], default.color[0]];
        let mut row = ShaderGridRow {
            label: label.to_owned(),
            default_cell: Some(ShaderGridCell {
                text: "color: RGB".to_owned(),
                value_kind: "default",
                color: Some(swatch(default_rgba)),
            }),
            value_cell: ShaderGridCell {
                text: "color: RGB".to_owned(),
                value_kind: "value",
                color: Some(swatch(rgba)),
            },
            parameter_type: Some("color".to_owned()),
            is_overridden: true,
            function: None,
            edit: Some(ShaderRowEdit {
                path: data_path,
                current: format!("{},{},{},{}", rgba[0], rgba[1], rgba[2], rgba[3]),
                kind: ShaderRowEditKind::FunctionColor { block_path, block_index: function_index },
            }),
            context_menu: None,
            create_anim_op: None,
            constant_function_view: None,
        };
        row.constant_function_view = Some(view);
        return row;
    }
    if !is_color && let Some(value) = curve.as_constant() {
        let current = format_shader_float(value);
        let mut row = shader_plain_value_row(
            label.to_owned(),
            format!("value: {}", format_shader_float(default.real)),
            format!("value: {current}"),
            Some("animated scalar".to_owned()),
        );
        row.is_overridden = true;
        row.edit = Some(ShaderRowEdit {
            path: data_path,
            current,
            kind: ShaderRowEditKind::FunctionScalar { block_path, block_index: function_index },
        });
        row.constant_function_view = Some(view);
        return row;
    }
    let mut row = shader_function_grid_row(label.to_owned(), view);
    row.default_cell = Some(shader_default_value_cell(describe_value(default)));
    row
}

/// The material's own fields, under its parameters.
fn property_rows(root: &TagStruct<'_>) -> Vec<ShaderGridRow> {
    let mut rows = Vec::new();
    for field in root.fields() {
        let name = field.name();
        let path = append_field_path_for("", &field);
        match field.options() {
            Some(blam_tags::TagOptions::Enum { names, current }) => {
                let options: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
                let current = current.unwrap_or(0).max(0) as usize;
                rows.push(shader_enum_value_row(clean_field_name(name), String::new(), current, options, path));
            }
            Some(blam_tags::TagOptions::Flags(options)) => {
                let raw = options
                    .iter()
                    .filter(|option| option.is_set)
                    .fold(0u64, |mask, option| mask | (1u64 << option.bit));
                let labels: Vec<String> = options.iter().map(|option| option.name.to_owned()).collect();
                let mut row = shader_plain_value_row(
                    clean_field_name(name),
                    String::new(),
                    raw.to_string(),
                    Some("flags".to_owned()),
                );
                row.is_overridden = true;
                row.edit = Some(ShaderRowEdit { path, current: raw.to_string(), kind: ShaderRowEditKind::Flags(labels) });
                rows.push(row);
            }
            None => match field.value() {
                Some(TagFieldData::Real(value)) => {
                    let current = format_shader_float(value);
                    let mut row = shader_plain_value_row(
                        clean_field_name(name),
                        String::new(),
                        current.clone(),
                        Some("real".to_owned()),
                    );
                    row.is_overridden = true;
                    row.edit = Some(ShaderRowEdit { path, current, kind: ShaderRowEditKind::Scalar });
                    rows.push(row);
                }
                Some(TagFieldData::StringId(_)) => {
                    let current = root.read_string_id(name).unwrap_or_default();
                    let mut row = shader_plain_value_row(
                        clean_field_name(name),
                        String::new(),
                        none_if_empty(&current),
                        Some("string id".to_owned()),
                    );
                    row.is_overridden = true;
                    row.edit = Some(ShaderRowEdit { path, current, kind: ShaderRowEditKind::StringId });
                    rows.push(row);
                }
                _ => {}
            },
        }
    }
    rows
}
