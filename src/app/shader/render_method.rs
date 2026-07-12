//! Render-method lookup, cache access, and edit-target construction.
//! It owns shader-specific models, edits, and presentation helpers; generic field editing and controller orchestration belong elsewhere.

use super::*;

pub(in crate::app) fn render_method_flags_mask(render_method: &RenderMethod) -> u64 {
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

pub(in crate::app) fn cached_render_method_definition(
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

pub(in crate::app) fn cached_render_method_option(
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

pub(in crate::app) fn render_method_edit_prefix(tag: &TagFile) -> String {
    if tag.root().field("render_method").is_some() {
        "render_method".to_owned()
    } else {
        String::new()
    }
}

pub(in crate::app) fn render_method_existing_field_path(
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
pub(in crate::app) fn read_global_material_type(tag: &TagFile) -> String {
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
pub(in crate::app) fn animated_param_paths(
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

pub(in crate::app) fn existing_shader_function_target(
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

pub(in crate::app) fn new_shader_function_target(
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

pub(in crate::app) fn shader_parameter_type_index(parameter: &RenderMethodOptionParameter) -> i32 {
    // Canonical schema index, consumed only as an internal selector by
    // shader_parameter_type_initial_field's write (which routes through the
    // edit system's declaration-index == wire assumption). Avoids exposing
    // the raw wire value.
    parameter
        .parameter_type
        .map(|kind| kind.get() as i32)
        .unwrap_or(RenderMethodParameterType::Real as i32)
}

pub(in crate::app) fn shader_parameter_type_initial_field(
    parameter_type_index: i32,
) -> ShaderParamInitialField {
    ShaderParamInitialField {
        field: "parameter type".to_owned(),
        input: parameter_type_index.to_string(),
    }
}

pub(in crate::app) fn shader_function_action(
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

pub(in crate::app) fn push_shader_context_action(
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
        ShaderContextAction::H2ParameterOp(op) => {
            edit.h2_shader_param_ops.push(op.clone());
        }
    }
}
