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
    create_anim_op: Option<ShaderOp>,
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
    op: ShaderOp,
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
    BitmapRef { group_tag: u32 },
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
    /// No Color animated parameter exists yet. The swatch opens the color
    /// popup and OK creates one initialized to the selected constant color.
    CreateFunctionColor { op: ShaderOp },
    /// No parameter instance exists yet. On commit a new `parameters[]`
    /// element is created via `ShaderParamOp`.
    CreateScalarParam {
        parameters_block_path: String,
        parameter_name: String,
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
        let rows = shader_rows_from_option(&render_method, &option, &edit_prefix);
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
    })
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
        data: append_field_path(&base, "function/data"),
        parameter_type: append_field_path(&base, "type"),
        input_name: append_field_path(&base, "input name"),
        range_name: append_field_path(&base, "range name"),
        time_period: append_field_path(&base, "time period"),
        block_path,
        block_index: animated_index,
    }
}

pub(super) fn shader_rows_from_option(
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
        push_shader_parameter_rows(&mut rows, parameter, instance, edit_prefix, instance_index);
    }
    rows
}

pub(super) fn push_shader_parameter_rows(
    rows: &mut Vec<ShaderGridRow>,
    parameter: &RenderMethodOptionParameter,
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
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

/// 32-byte `mapping_function` blob for a Constant function with value 1.0.
/// Used as the default for scale transform animated parameters.
pub(super) const CONSTANT_FUNCTION_1_HEX: &str =
    "010000000000803f0000803f0000000000000000000000000000000000000000";

/// 32-byte `mapping_function` blob for a Constant function with value 0.0.
/// Used as the default for translation/frame-index animated parameters.
pub(super) const CONSTANT_FUNCTION_0_HEX: &str =
    "0100000000000000000000000000000000000000000000000000000000000000";

/// Build a 32-byte `mapping_function` hex blob for `Constant(v)`.
///
/// Layout: byte 0 = 1 (Constant), bytes 1-3 = 0, bytes 4-7 = v (f32 LE),
/// bytes 8-11 = v (f32 LE, clamp_range_max mirrors min for unranged), rest 0.
pub(super) fn constant_function_hex(v: f32) -> String {
    let mut blob = [0u8; 32];
    blob[0] = 1; // FunctionType::Constant
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
    Some([
        ((argb >> 16) & 0xFF) as f32 / 255.0, // r
        ((argb >> 8) & 0xFF) as f32 / 255.0,  // g
        (argb & 0xFF) as f32 / 255.0,         // b
        ((argb >> 24) & 0xFF) as f32 / 255.0, // a
    ])
}

/// Build a 32-byte `mapping_function` hex blob for a Constant 1-color function.
/// Layout: byte 0 = 1 (Constant), byte 2 = 1 (OneColor), bytes 4-7 = ARGB u32 LE.
pub(super) fn constant_color_function_hex(r: f32, g: f32, b: f32, a: f32) -> String {
    let mut blob = [0u8; 32];
    blob[0] = 1; // FunctionType::Constant
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

    // Build right-click context menu: offer transform types not yet present.
    let context_menu = param_index.map(|pidx| {
        let animated_block_path = append_field_path(
            edit_prefix,
            &format!("parameters[{pidx}]/animated parameters"),
        );
        let existing_types: std::collections::HashSet<i32> = instance
            .iter()
            .flat_map(|inst| &inst.animated_parameters)
            .filter_map(|ap| ap.parameter_type.map(|t| t.raw()))
            .collect();
        let items: Vec<ShaderContextItem> = BITMAP_TRANSFORM_TYPES
            .iter()
            .filter(|(kind, _, _)| !existing_types.contains(&(*kind as i32)))
            .map(|(kind, suffix, hex)| ShaderContextItem {
                label: format!("Add {}_{}", parameter.parameter_name, suffix),
                op: ShaderOp {
                    animated_block_path: animated_block_path.clone(),
                    output_type_index: *kind as i32,
                    initial_function_hex: hex.to_string(),
                },
            })
            .collect();
        ShaderContextMenu { items }
    });

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
        fill: MATERIAL_REF_ROW,
        parameter_type: Some("bitmap".to_owned()),
        function: None,
        edit: shader_param_field_path(edit_prefix, param_index, "bitmap").map(|path| {
            ShaderRowEdit {
                path,
                current: if value.is_empty() {
                    "NONE".to_owned()
                } else {
                    format!("{}.bitmap", value.replace('\\', "/"))
                },
                kind: ShaderRowEditKind::BitmapRef {
                    group_tag: u32::from_be_bytes(*b"bitm"),
                },
            }
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

    // filter_mode — sampler filter enum dropdown.
    rows.push(shader_sampler_enum_row(
        format!("{name}_filter_mode"),
        parameter.default_filter_mode.raw(),
        instance
            .map(|p| p.bitmap_filter_mode as i16)
            .unwrap_or(parameter.default_filter_mode.raw()),
        filter_opts,
        edit_prefix,
        param_index,
        "bitmap filter mode",
    ));
    // wrap_mode / wrap_mode_x / wrap_mode_y — sampler address enum dropdowns.
    for (suffix, field, current) in [
        (
            "wrap_mode",
            "bitmap address mode",
            instance
                .map(|p| p.bitmap_address_mode as i16)
                .unwrap_or(parameter.default_address_mode.raw()),
        ),
        (
            "wrap_mode_x",
            "bitmap address mode x",
            instance
                .map(|p| p.bitmap_address_mode_x as i16)
                .unwrap_or(parameter.default_address_mode.raw()),
        ),
        (
            "wrap_mode_y",
            "bitmap address mode y",
            instance
                .map(|p| p.bitmap_address_mode_y as i16)
                .unwrap_or(parameter.default_address_mode.raw()),
        ),
    ] {
        rows.push(shader_sampler_enum_row(
            format!("{name}_{suffix}"),
            parameter.default_address_mode.raw(),
            current,
            addr_opts.clone(),
            edit_prefix,
            param_index,
            field,
        ));
    }
    // comparison_function / extern_texture — no authoritative option-name
    // table, so edit as plain integers.
    rows.push(shader_sampler_int_row(
        format!("{name}_comparison_function"),
        parameter.default_comparison_function.raw(),
        instance
            .map(|p| p.bitmap_comparison_function as i16)
            .unwrap_or(parameter.default_comparison_function.raw()),
        edit_prefix,
        param_index,
        "bitmap comparison function",
    ));
    rows.push(shader_sampler_int_row(
        format!("{name}_extern_texture"),
        0,
        instance
            .and_then(|p| p.bitmap_extern_mode.map(|mode| mode as i16))
            .unwrap_or(0),
        edit_prefix,
        param_index,
        "bitmap extern RTT mode",
    ));

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
                    .map(|e| e.data.clone())
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
                    fill: MATERIAL_NUMERIC_ROW,
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
    } else if parameter.default_bitmap_scale != 0.0 {
        rows.push(shader_plain_value_row(
            format!("{name}_scale_uniform"),
            format_shader_float(parameter.default_bitmap_scale),
            "value".to_owned(),
            MATERIAL_FUNCTION_ROW,
            Some("function".to_owned()),
        ));
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
                    .map(|e| e.data.clone())
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
                    fill: MATERIAL_NUMERIC_ROW,
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
            fill: MATERIAL_NUMERIC_ROW,
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
            },
        })
    } else {
        None
    };
    ShaderGridRow {
        label: parameter.parameter_name.clone(),
        default_cell: Some(shader_default_value_cell(default_val.clone())),
        value_cell: shader_value_cell(format!("value: {current}")),
        fill: MATERIAL_NUMERIC_ROW,
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
        MATERIAL_DATA_ROW,
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
        MATERIAL_DATA_ROW,
        Some("bool".to_owned()),
    );
    row.edit =
        shader_param_field_path(edit_prefix, param_index, "int/bool").map(|path| ShaderRowEdit {
            path,
            current: raw.to_string(),
            kind: ShaderRowEditKind::Enum(vec!["false".to_owned(), "true".to_owned()]),
        });
    row
}

pub(super) fn shader_color_row(
    parameter: &RenderMethodOptionParameter,
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
) -> ShaderGridRow {
    let (slot, _) = compile_real_constant(parameter, instance);
    let default_color =
        material_color_from_argb(&parameter.parameter_name, parameter.default_color.0);
    // slot = [r, g, b, a] from compile_real_constant for Color type.
    let value_color = MaterialColorPopup::new(
        &parameter.parameter_name,
        slot[0],
        slot[1],
        slot[2],
        slot[3],
    );

    // Check if there is a constant 1-color Color animated parameter.
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

            if let Some(rgba) = extract_constant_color(function) {
                // Constant 1-color: show as inline editable color swatch.
                let data_path = view
                    .edit
                    .as_ref()
                    .map(|e| e.data.clone())
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
                    fill: MATERIAL_NUMERIC_ROW,
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

    // No Color animated parameter — show static color swatch + f()+ create button.
    let create_anim_op = param_index.map(|pidx| ShaderOp {
        animated_block_path: append_field_path(
            edit_prefix,
            &format!("parameters[{pidx}]/animated parameters"),
        ),
        output_type_index: RenderMethodAnimatedParameterType::Color as i32,
        initial_function_hex: {
            // Build a constant-color blob using the template default color.
            let argb = parameter.default_color.0;
            let a8 = ((argb >> 24) & 0xFF) as u8;
            let r8 = ((argb >> 16) & 0xFF) as u8;
            let g8 = ((argb >> 8) & 0xFF) as u8;
            let b8 = (argb & 0xFF) as u8;
            constant_color_function_hex(
                r8 as f32 / 255.0,
                g8 as f32 / 255.0,
                b8 as f32 / 255.0,
                a8 as f32 / 255.0,
            )
        },
    });
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
        fill: MATERIAL_NUMERIC_ROW,
        parameter_type: Some("color".to_owned()),
        function: None,
        edit: create_anim_op.clone().map(|op| ShaderRowEdit {
            path: format!("create:{}", parameter.parameter_name),
            current: format!("{},{},{},{}", slot[0], slot[1], slot[2], slot[3]),
            kind: ShaderRowEditKind::CreateFunctionColor { op },
        }),
        context_menu: None,
        create_anim_op,
        constant_function_view: None,
    }
}

pub(super) fn shader_alpha_row(
    parameter: &RenderMethodOptionParameter,
    instance: Option<&RenderMethodParameter>,
    edit_prefix: &str,
    param_index: Option<usize>,
) -> ShaderGridRow {
    let function = first_render_method_function(instance, edit_prefix, param_index, |kind| {
        matches!(kind, RenderMethodAnimatedParameterType::Alpha)
    });
    let (slot, _) = compile_real_constant(parameter, instance);
    let default_alpha = ((parameter.default_color.0 >> 24) & 0xFF) as f32 / 255.0;
    if let Some(function) = function {
        return shader_function_grid_row(format!("{}_alpha", parameter.parameter_name), function);
    }
    let create_anim_op = param_index.map(|pidx| ShaderOp {
        animated_block_path: append_field_path(
            edit_prefix,
            &format!("parameters[{pidx}]/animated parameters"),
        ),
        output_type_index: RenderMethodAnimatedParameterType::Alpha as i32,
        initial_function_hex: CONSTANT_FUNCTION_1_HEX.to_owned(),
    });
    ShaderGridRow {
        label: format!("{}_alpha", parameter.parameter_name),
        default_cell: Some(shader_default_value_cell(format!(
            "value: {}",
            format_shader_float(default_alpha)
        ))),
        value_cell: shader_value_cell(format!("value: {}", format_shader_float(slot[0]))),
        fill: MATERIAL_NUMERIC_ROW,
        parameter_type: Some("alpha".to_owned()),
        function: None,
        edit: None,
        context_menu: None,
        create_anim_op,
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
        MATERIAL_DATA_ROW,
        Some("option".to_owned()),
    )
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

/// A bitmap sampler sub-row edited as a plain integer (comparison function /
/// extern texture mode — no authoritative option-name table).
pub(super) fn shader_sampler_int_row(
    label: String,
    default_value: i16,
    current_value: i16,
    edit_prefix: &str,
    param_index: Option<usize>,
    field: &str,
) -> ShaderGridRow {
    let mut row =
        shader_option_value_row(label, default_value.to_string(), current_value.to_string());
    row.edit = shader_param_field_path(edit_prefix, param_index, field).map(|path| ShaderRowEdit {
        path,
        current: current_value.to_string(),
        kind: ShaderRowEditKind::Int,
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
        fill: MATERIAL_FUNCTION_ROW,
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
    MaterialColorPopup::new(
        title,
        ((argb >> 16) & 0xFF) as f32 / 255.0,
        ((argb >> 8) & 0xFF) as f32 / 255.0,
        (argb & 0xFF) as f32 / 255.0,
        ((argb >> 24) & 0xFF) as f32 / 255.0,
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
            fill: MATERIAL_DATA_ROW,
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

    let definition_row = ShaderGridRow {
        label: "definition".to_owned(),
        default_cell: None,
        value_cell: ShaderGridCell {
            text: format!("{}.render_method_definition", model.definition_path),
            value_kind: "value",
            color: None,
        },
        fill: MATERIAL_REF_ROW,
        parameter_type: Some("tag reference".to_owned()),
        function: None,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    };
    draw_shader_grid_row(ui, &definition_row, 0, color_popup, function_popup, edit);

    if let Some(template_path) = model.shader_template_path.as_deref() {
        let template_row = ShaderGridRow {
            label: "shader template".to_owned(),
            default_cell: None,
            value_cell: ShaderGridCell {
                text: format!("{template_path}.render_method_template"),
                value_kind: "value",
                color: None,
            },
            fill: MATERIAL_REF_ROW,
            parameter_type: Some("tag reference".to_owned()),
            function: None,
            edit: None,
            context_menu: None,
            create_anim_op: None,
            constant_function_view: None,
        };
        draw_shader_grid_row(ui, &template_row, 0, color_popup, function_popup, edit);
    }

    draw_shader_grid_section_header(ui, "CATEGORIES");
    for category in &model.categories {
        draw_shader_category_row(ui, category, edit);
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
                fill: MATERIAL_DATA_ROW,
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
    ui.painter().rect_filled(rect, 0.0, MATERIAL_DATA_ROW);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, MATERIAL_GRID_LIGHT),
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
        MATERIAL_TEXT,
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
            MATERIAL_MUTED_TEXT,
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
            fill: MATERIAL_REF_ROW,
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
        .unwrap_or(MATERIAL_DATA_ROW);

    if function.is_some() {
        if let Some(function) = function.as_ref() {
            value_cell.text = shader_function_grid_text(&function.function);
        }
        value_cell.value_kind = "value";
        fill = MATERIAL_FUNCTION_ROW;
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
    let mut block_clip_request = None;
    let mut ctx = FieldEditContext {
        view_scope: "readonly",
        tag_key: "",
        group_tag: 0,
        tags_root: None,
        editable: false,
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
    ui.painter().rect_filled(rect, 0.0, MATERIAL_SECTION_HEADER);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, MATERIAL_GRID_LIGHT),
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(4.0, 0.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.0),
        MATERIAL_TEXT,
    );
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
        Stroke::new(1.0, MATERIAL_GRID_LIGHT),
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
        MATERIAL_TEXT,
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
        ui.painter().rect_filled(check_rect, 0.0, Color32::WHITE);
        ui.painter()
            .rect_stroke(check_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
        ui.painter().text(
            check_rect.right_center() + Vec2::new(4.0, 0.0),
            Align2::LEFT_CENTER,
            "range:",
            FontId::proportional(12.0),
            MATERIAL_TEXT,
        );

        let button_rect = egui::Rect::from_min_size(
            range_rect.right_top() + Vec2::new(4.0, -1.0),
            Vec2::new(28.0, height - 4.0),
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
                ui.painter().rect_filled(del_rect, 0.0, Color32::WHITE);
                ui.painter()
                    .rect_stroke(del_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
                ui.painter().text(
                    del_rect.center(),
                    Align2::CENTER_CENTER,
                    "×",
                    FontId::proportional(13.0),
                    Color32::DARK_RED,
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
        ui.painter().rect_filled(f_rect, 0.0, Color32::WHITE);
        ui.painter()
            .rect_stroke(f_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
        ui.painter().text(
            f_rect.center(),
            Align2::CENTER_CENTER,
            "f()",
            FontId::proportional(11.0),
            MATERIAL_TEXT,
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
                ui.painter().rect_filled(del_rect, 0.0, Color32::WHITE);
                ui.painter()
                    .rect_stroke(del_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
                ui.painter().text(
                    del_rect.center(),
                    Align2::CENTER_CENTER,
                    "×",
                    FontId::proportional(13.0),
                    Color32::DARK_RED,
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
    } else if let (true, Some(op)) = (editable, row.create_anim_op.as_ref()) {
        // No animated parameter yet — show an "f()+" button to create one.
        let button_rect = egui::Rect::from_min_size(
            value_rect.right_top() + Vec2::new(12.0, 2.0),
            Vec2::new(34.0, height - 4.0),
        );
        ui.painter().rect_filled(button_rect, 0.0, Color32::WHITE);
        ui.painter()
            .rect_stroke(button_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
        ui.painter().text(
            button_rect.center(),
            Align2::CENTER_CENTER,
            "f()+",
            FontId::proportional(11.0),
            MATERIAL_TEXT,
        );
        let add_response = ui
            .interact(
                button_rect,
                ui.make_persistent_id(format!("shader_create_anim:{}", row.label)),
                Sense::click(),
            )
            .on_hover_text("Create animated parameter");
        if add_response.clicked() {
            edit.shader_ops.push(op.clone());
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
                            edit.shader_ops.push(item.op.clone());
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
                ui.visuals_mut().extreme_bg_color = Color32::WHITE;
                let resp = ui.add(
                    egui::TextEdit::singleline(buffer)
                        .id(id)
                        .desired_width(text_rect.width())
                        .text_color(MATERIAL_TEXT)
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
            ui.painter().rect_filled(del_rect, 0.0, Color32::WHITE);
            ui.painter()
                .rect_stroke(del_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
            ui.painter().text(
                del_rect.center(),
                Align2::CENTER_CENTER,
                "×",
                FontId::proportional(13.0),
                Color32::DARK_RED,
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
        ShaderRowEditKind::BitmapRef { group_tag } => {
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
                    Color32::WHITE
                } else {
                    Color32::from_gray(210)
                },
            );
            ui.painter()
                .rect_stroke(open_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
            ui.painter().text(
                open_rect.center(),
                Align2::CENTER_CENTER,
                "Open",
                FontId::proportional(11.0),
                MATERIAL_TEXT,
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
                ui.visuals_mut().extreme_bg_color = Color32::WHITE;
                let resp = ui.add(
                    egui::TextEdit::singleline(buffer)
                        .id(id)
                        .desired_width(text_rect.width())
                        .text_color(MATERIAL_TEXT)
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
            // "..." browse button
            ui.painter().rect_filled(browse_rect, 0.0, Color32::WHITE);
            ui.painter()
                .rect_stroke(browse_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
            ui.painter().text(
                browse_rect.center(),
                Align2::CENTER_CENTER,
                "...",
                FontId::proportional(11.0),
                MATERIAL_TEXT,
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
                    edit.pending.push(PendingFieldEdit {
                        path: row_edit.path.clone(),
                        input: rel,
                    });
                }
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
            ui.painter().rect_filled(swatch_rect, 0.0, Color32::WHITE);
            ui.painter()
                .rect_stroke(swatch_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
            // Inner color swatch.
            let inner = swatch_rect.shrink(3.0);
            ui.painter().rect_filled(inner, 0.0, color32);
            ui.painter()
                .rect_stroke(inner, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
            ui.painter().text(
                swatch_rect.left_center() + Vec2::new(inner.width() + 8.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                FontId::monospace(12.0),
                MATERIAL_TEXT,
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
            ui.painter().rect_filled(del_rect, 0.0, Color32::WHITE);
            ui.painter()
                .rect_stroke(del_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
            ui.painter().text(
                del_rect.center(),
                Align2::CENTER_CENTER,
                "×",
                FontId::proportional(13.0),
                Color32::DARK_RED,
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

        ShaderRowEditKind::CreateFunctionColor { op } => {
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
            ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
            ui.painter()
                .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
            let inner = rect.shrink(3.0);
            ui.painter().rect_filled(inner, 0.0, color32);
            ui.painter()
                .rect_stroke(inner, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
            ui.painter().text(
                rect.left_center() + Vec2::new(inner.width() + 8.0, 0.0),
                Align2::LEFT_CENTER,
                "color: RGB",
                FontId::monospace(12.0),
                MATERIAL_TEXT,
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
                *color_popup = Some(
                    MaterialColorPopup::new(label, r, g, b, a)
                        .with_shader_op(edit.tag_key, op.clone()),
                );
            }
        }

        // No instance yet: text box for default value; on commit create the parameter entry.
        ShaderRowEditKind::CreateScalarParam {
            parameters_block_path,
            parameter_name,
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
                ui.visuals_mut().extreme_bg_color = Color32::from_rgb(255, 252, 235);
                let resp = ui.add(
                    egui::TextEdit::singleline(buffer)
                        .id(id)
                        .desired_width(rect.width())
                        .text_color(MATERIAL_TEXT)
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
                    real_value: v,
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
                ui.visuals_mut().extreme_bg_color = Color32::WHITE;
                let resp = ui.add(
                    egui::TextEdit::singleline(buffer)
                        .id(id)
                        .desired_width(rect.width())
                        .text_color(MATERIAL_TEXT)
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
        Some("default") | None => (MATERIAL_DEFAULT_BOX, MATERIAL_MUTED_TEXT),
        _ => (Color32::WHITE, MATERIAL_TEXT),
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));

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
        ui.painter().rect_filled(swatch_rect, 0.0, color.color32());
        ui.painter()
            .rect_stroke(swatch_rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
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
