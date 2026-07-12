//! Shader grid row and cell model construction.
//! It owns shader-specific models, edits, and presentation helpers; generic field editing and controller orchestration belong elsewhere.

use super::*;

pub(in crate::app) fn shader_rows_from_option(
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

pub(in crate::app) fn push_shader_parameter_rows(
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
pub(in crate::app) fn shader_param_field_path(
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

pub(in crate::app) fn shader_param_existing_field_path(
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
pub(in crate::app) const CONSTANT_FUNCTION_1_HEX: &str =
    "012000000000803f0000803f0000000000000000000000000000000000000000";

/// 32-byte `mapping_function` blob for a Constant function with value 0.0.
/// Used as the default for translation/frame-index animated parameters.
pub(in crate::app) const CONSTANT_FUNCTION_0_HEX: &str =
    "0120000000000000000000000000000000000000000000000000000000000000";

/// Build a 32-byte `mapping_function` hex blob for `Constant(v)`.
///
/// Layout: byte 0 = 1 (Constant), bytes 1-3 = 0, bytes 4-7 = v (f32 LE),
/// bytes 8-11 = v (f32 LE, clamp_range_max mirrors min for unranged), rest 0.
pub(in crate::app) fn constant_function_hex(v: f32) -> String {
    let mut blob = [0u8; 32];
    blob[0] = 1; // FunctionType::Constant
    blob[1] = FunctionFlags::GPU;
    blob[4..8].copy_from_slice(&v.to_le_bytes());
    blob[8..12].copy_from_slice(&v.to_le_bytes());
    blob.iter().map(|b| format!("{b:02x}")).collect()
}

pub(in crate::app) fn is_h2_legacy_constant_function_data(data: &[u8]) -> bool {
    data.len() >= 8 && is_h2_legacy_function_data(data) && matches!(data.first(), Some(1))
}

pub(super) fn is_h2_legacy_function_data(data: &[u8]) -> bool {
    !data.is_empty() && data.len() != 32
}

pub(super) fn is_h2_legacy_nonconstant_function_data(data: &[u8]) -> bool {
    is_h2_legacy_function_data(data) && !is_h2_legacy_constant_function_data(data)
}

pub(super) fn h2_legacy_constant_scalar(data: &[u8]) -> Option<f32> {
    if !is_h2_legacy_constant_function_data(data) {
        return None;
    }
    Some(f32::from_le_bytes(data.get(4..8)?.try_into().ok()?))
}

pub(super) fn h2_legacy_constant_color(data: &[u8]) -> Option<[f32; 4]> {
    if !is_h2_legacy_constant_function_data(data)
        || data.len() < 8
        || data.get(1).copied()? & 0x20 == 0
    {
        return None;
    }
    Some([
        byte_to_float(data[6]),
        byte_to_float(data[5]),
        byte_to_float(data[4]),
        byte_to_float(data[7]),
    ])
}

pub(in crate::app) fn h2_constant_scalar_function_data(
    value: f32,
    existing: Option<&[u8]>,
) -> Vec<u8> {
    if let Some(existing) = existing.filter(|data| is_h2_legacy_constant_function_data(data)) {
        let mut data = existing.to_vec();
        let old = h2_legacy_constant_scalar(existing);
        data[4..8].copy_from_slice(&value.to_le_bytes());
        if data.len() >= 12
            && old.is_some_and(|old| {
                f32::from_le_bytes(data[8..12].try_into().unwrap_or_default()) == old
            })
        {
            data[8..12].copy_from_slice(&value.to_le_bytes());
        }
        return data;
    }
    decode_hex(&constant_function_hex(value)).unwrap_or_default()
}

pub(in crate::app) fn h2_constant_color_function_data(
    r: f32,
    g: f32,
    b: f32,
    a: f32,
    existing: Option<&[u8]>,
) -> Vec<u8> {
    if let Some(existing) = existing.filter(|data| is_h2_legacy_constant_function_data(data)) {
        let mut data = existing.to_vec();
        data[4] = float_channel_to_u8(b);
        data[5] = float_channel_to_u8(g);
        data[6] = float_channel_to_u8(r);
        data[7] = float_channel_to_u8(a);
        return data;
    }
    decode_hex(&constant_color_function_hex(r, g, b, a)).unwrap_or_default()
}

/// True when `f` is a Constant-type function with a color (not scalar) output.
/// Used to decide whether to show a constant color swatch vs a graph row.
pub(in crate::app) fn is_constant_color_fn(f: &TagFunction) -> bool {
    f.color_graph_type() != ColorGraphType::Scalar
        && matches!(f.kind(), FunctionKind::Constant { .. })
}

/// Extract the (r, g, b, a) components from a constant 1-color function.
/// Returns None for scalar functions or non-constant types.
pub(in crate::app) fn extract_constant_color(f: &TagFunction) -> Option<[f32; 4]> {
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
pub(in crate::app) fn constant_color_function_hex(r: f32, g: f32, b: f32, a: f32) -> String {
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
pub(in crate::app) const BITMAP_TRANSFORM_TYPES: &[(
    RenderMethodAnimatedParameterType,
    &str,
    &str,
)] = &[
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

pub(in crate::app) struct BitmapSamplerOverride {
    menu_label: &'static str,
    field: &'static str,
    flag_bit: i16,
}

pub(in crate::app) const BITMAP_SAMPLER_OVERRIDES: &[BitmapSamplerOverride] = &[
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

pub(in crate::app) fn shader_bitmap_row(
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
        let existing_types: std::collections::HashSet<RenderMethodAnimatedParameterType> = instance
            .iter()
            .flat_map(|inst| &inst.animated_parameters)
            .filter_map(|ap| ap.parameter_type.map(|t| t.get()))
            .collect();
        let mut items = Vec::new();
        if let Some(pidx) = param_index {
            let animated_block_path = append_field_path(
                edit_prefix,
                &format!("parameters[{pidx}]/animated parameters"),
            );
            for (kind, suffix, hex) in BITMAP_TRANSFORM_TYPES {
                if existing_types.contains(kind) {
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
                parameter.default_filter_mode.name()
            } else {
                parameter.default_address_mode.name()
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
                            input: initial_value.to_owned(),
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
        is_overridden: instance.is_some(),
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

pub(in crate::app) fn shader_bitmap_expansion_rows(
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
                option_index_for_name(&filter_opts, parameter.default_filter_mode.name()) as i16,
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
                option_index_for_name(&addr_opts, parameter.default_address_mode.name()) as i16,
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
                option_index_for_name(&addr_opts, parameter.default_address_mode.name()) as i16,
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
                option_index_for_name(&addr_opts, parameter.default_address_mode.name()) as i16,
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
                    is_overridden: true,
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

pub(in crate::app) fn shader_scalar_row(
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
                    is_overridden: true,
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
            is_overridden: true,
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
        is_overridden: false,
        function: None,
        edit,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

pub(in crate::app) fn shader_int_row(
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
    row.is_overridden = instance.is_some();
    row.edit =
        shader_param_field_path(edit_prefix, param_index, "int/bool").map(|path| ShaderRowEdit {
            path,
            current: value.to_string(),
            kind: ShaderRowEditKind::Int,
        });
    row
}

pub(in crate::app) fn shader_bool_row(
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
    row.is_overridden = instance.is_some();
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

pub(in crate::app) fn shader_color_row(
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
    let default_function_hex =
        default_shader_color_function_hex(parameter.default_color.0, is_argb_parameter);
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
                    is_overridden: true,
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
            is_overridden: true,
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
        is_overridden: false,
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

fn default_shader_color_function_hex(argb: u32, is_argb_parameter: bool) -> String {
    let a8 = ((argb >> 24) & 0xFF) as u8;
    let r8 = ((argb >> 16) & 0xFF) as u8;
    let g8 = ((argb >> 8) & 0xFF) as u8;
    let b8 = (argb & 0xFF) as u8;
    let function_alpha = if is_argb_parameter {
        a8 as f32 / 255.0
    } else {
        1.0
    };
    constant_color_function_hex(
        r8 as f32 / 255.0,
        g8 as f32 / 255.0,
        b8 as f32 / 255.0,
        function_alpha,
    )
}

pub(in crate::app) fn argb_to_rgba(argb: u32) -> [f32; 4] {
    [
        ((argb >> 16) & 0xFF) as f32 / 255.0,
        ((argb >> 8) & 0xFF) as f32 / 255.0,
        (argb & 0xFF) as f32 / 255.0,
        ((argb >> 24) & 0xFF) as f32 / 255.0,
    ]
}

pub(in crate::app) fn shader_alpha_row(
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
                    is_overridden: true,
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
        is_overridden: instance.is_some(),
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

pub(in crate::app) fn shader_option_value_row(
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

pub(in crate::app) fn shader_int_value_row(
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

/// Resolve an enum value to its position in a display-option list by name,
/// so the UI selects/pre-fills by the schema name rather than the raw wire
/// integer. Falls back to 0 when the name isn't in the list.
pub(in crate::app) fn option_index_for_name(options: &[String], name: &str) -> usize {
    options
        .iter()
        .position(|opt| opt.eq_ignore_ascii_case(name))
        .unwrap_or(0)
}

pub(in crate::app) fn shader_enum_value_row(
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
pub(in crate::app) fn bitmap_filter_option_labels() -> Vec<String> {
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
pub(in crate::app) fn bitmap_address_option_labels() -> Vec<String> {
    ["wrap", "clamp", "mirror", "black border"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub(in crate::app) fn bitmap_extern_option_labels() -> Vec<String> {
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
pub(in crate::app) fn shader_sampler_enum_row(
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
    row.is_overridden = param_index.is_some();
    row.edit = shader_param_field_path(edit_prefix, param_index, field).map(|path| ShaderRowEdit {
        path,
        current: current_index.to_string(),
        kind: ShaderRowEditKind::Enum(options),
    });
    row
}

pub(in crate::app) fn shader_plain_value_row(
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
        is_overridden: false,
        function: None,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

pub(in crate::app) fn shader_function_grid_row(
    label: String,
    function: FunctionView,
) -> ShaderGridRow {
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
        is_overridden: true,
        function: Some(function),
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

pub(in crate::app) fn shader_value_cell(text: String) -> ShaderGridCell {
    ShaderGridCell {
        text,
        value_kind: "value",
        color: None,
    }
}

pub(in crate::app) fn shader_default_value_cell(text: String) -> ShaderGridCell {
    ShaderGridCell {
        text,
        value_kind: "default",
        color: None,
    }
}

pub(in crate::app) fn material_color_from_argb(title: &str, argb: u32) -> MaterialColorPopup {
    let alpha = ((argb >> 24) & 0xFF) as f32 / 255.0;
    MaterialColorPopup::new(
        title,
        ((argb >> 16) & 0xFF) as f32 / 255.0,
        ((argb >> 8) & 0xFF) as f32 / 255.0,
        (argb & 0xFF) as f32 / 255.0,
        if alpha == 0.0 { 1.0 } else { alpha },
    )
}

pub(in crate::app) fn none_if_empty(value: &str) -> String {
    if value.is_empty() {
        "NONE".to_owned()
    } else {
        value.to_owned()
    }
}

pub(in crate::app) fn format_shader_float(value: f32) -> String {
    let mut text = format!("{value:.4}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.push('0');
    }
    text
}

pub(in crate::app) fn shader_function_grid_text(function: &TagFunction) -> String {
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

    match function.kind() {
        FunctionKind::Identity { .. } => format!("identity: {}", function_sample_summary(function)),
        FunctionKind::Constant { header } => {
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
        FunctionKind::Transition { compact, .. } => format!(
            "transition {}: {}",
            compact.function_index,
            function_sample_summary(function)
        ),
        FunctionKind::Periodic { compact, .. } => format!(
            "periodic {} freq {} phase {}: {}",
            compact.function_index,
            format_shader_float(compact.frequency),
            format_shader_float(compact.phase),
            function_sample_summary(function)
        ),
        FunctionKind::Linear { compact, .. } => format!(
            "linear: {}*x + {} ({})",
            format_shader_float(compact.slope),
            format_shader_float(compact.offset),
            function_sample_summary(function)
        ),
        FunctionKind::LinearKey { compact, .. } => {
            format!("curve: {}", function_points_summary(&compact.graph_points))
        }
        FunctionKind::MultiLinearKey { compact, .. } => {
            format!(
                "multi curve: {}",
                function_points_summary(&compact.graph_points)
            )
        }
        FunctionKind::Spline { compact, .. } => format!(
            "spline: {}, {}, {}, {} ({})",
            format_shader_float(compact.i),
            format_shader_float(compact.j),
            format_shader_float(compact.k),
            format_shader_float(compact.l),
            function_sample_summary(function)
        ),
        FunctionKind::Spline2 { compact, .. } => format!(
            "spline2: x {} width {} bias {} ({})",
            format_shader_float(compact.left_x),
            format_shader_float(compact.width),
            format_shader_float(compact.bias),
            function_sample_summary(function)
        ),
        FunctionKind::MultiSpline { compact, .. } => format!(
            "multi-part curve: {} segment{} ({})",
            compact.parts.len(),
            if compact.parts.len() == 1 { "" } else { "s" },
            function_sample_summary(function)
        ),
        FunctionKind::Exponent { compact, .. } => format!(
            "exponent: {} to {}, pow {} ({})",
            format_shader_float(compact.amplitude_min),
            format_shader_float(compact.amplitude_max),
            format_shader_float(compact.exponent),
            function_sample_summary(function)
        ),
        FunctionKind::Unsupported { header, raw } => format!(
            "{:?}: {} bytes",
            header.function_type,
            raw.len().saturating_sub(32)
        ),
    }
}

pub(in crate::app) fn function_sample_summary(function: &TagFunction) -> String {
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

pub(in crate::app) fn function_points_summary(points: &[(f32, f32); 4]) -> String {
    points
        .iter()
        .map(|(x, y)| format!("({}, {})", format_shader_float(*x), format_shader_float(*y)))
        .collect::<Vec<_>>()
        .join(" ")
}
