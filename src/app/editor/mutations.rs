//! Field, block, shader, function, and model-variant mutation batches.
//! It owns tag-editor presentation and deferred edit construction; source loading and application lifecycle coordination belong elsewhere.

use super::*;

pub(in crate::app) fn apply_pending_edits(
    tag: &mut TagFile,
    edits: Vec<PendingFieldEdit>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for edit in edits {
        let result = catch_edit_unwind(|| apply_field_edit(tag, &edit.path, &edit.input));
        match result {
            Ok(()) => {
                *dirty = true;
                status = Some(format!("Edited {}", edit.path));
            }
            Err(error) => {
                status = Some(format!("Edit failed for {}: {error}", edit.path));
            }
        }
    }
    status
}

pub(in crate::app) fn apply_block_ops(
    tag: &mut TagFile,
    ops: Vec<BlockOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        let result = apply_one_block_op(tag, &op);
        match result {
            Ok(msg) => {
                *dirty = true;

                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("Block edit failed for {}: {error}", op.path));
            }
        }
    }
    status
}

pub(in crate::app) fn apply_function_data_ops(
    tag: &mut TagFile,
    ops: Vec<FunctionDataOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        let result =
            catch_edit_unwind(|| replace_halo2_function_byte_block(tag, &op.block_path, &op.data));
        match result {
            Ok(()) => {
                *dirty = true;
                status = Some(format!("Edited {}", op.block_path));
            }
            Err(error) => {
                status = Some(format!(
                    "Function edit failed for {}: {error}",
                    op.block_path
                ));
            }
        }
    }
    status
}

fn catch_edit_unwind(f: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .map_err(|panic| panic_message(panic))?
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<String>() {
        format!("internal edit panic: {message}")
    } else if let Some(message) = panic.downcast_ref::<&'static str>() {
        format!("internal edit panic: {message}")
    } else {
        "internal edit panic".to_owned()
    }
}

pub(in crate::app) fn replace_halo2_function_byte_block(
    tag: &mut TagFile,
    block_path: &str,
    data: &[u8],
) -> Result<(), String> {
    if TagFunction::parse(data).is_err()
        && !is_h2_legacy_constant_function_data(data)
        && !is_h2_legacy_editable_function_data(data)
        && !is_damage_effect_vibration_function_data(data)
    {
        return Err("invalid mapping_function data".to_owned());
    }
    let current_len = tag
        .root()
        .field_path(block_path)
        .and_then(|field| field.as_block())
        .map(|block| block.len());
    let Some(current_len) = current_len else {
        return replace_halo2_wrapped_function_byte_block(tag, block_path, data)
            .ok_or_else(|| format!("function byte block not found: {block_path}"))?;
    };
    if current_len == data.len() && current_len > 0 {
        for (index, byte) in data.iter().copied().enumerate() {
            let value = (byte as i8).to_string();
            apply_field_edit(tag, &format!("{block_path}[{index}]/Value"), &value)?;
        }
        return Ok(());
    }
    clear_block(tag, block_path)?;
    for (index, byte) in data.iter().copied().enumerate() {
        add_block_element(tag, block_path)?;
        let value = (byte as i8).to_string();
        apply_field_edit(tag, &format!("{block_path}[{index}]/Value"), &value)?;
    }
    Ok(())
}

fn is_h2_legacy_editable_function_data(data: &[u8]) -> bool {
    data.len() >= 20 && data.len() != 32 && data.first().is_some_and(|kind| *kind <= 10)
}

fn is_damage_effect_vibration_function_data(data: &[u8]) -> bool {
    data.len() == 36
        && data.first().is_some_and(|kind| *kind <= 10)
        && data.get(2).is_some_and(|exponent| *exponent <= 7)
        && data.get(20..24).is_some_and(|bytes| {
            f32::from_le_bytes(bytes.try_into().unwrap_or_default()).is_finite()
        })
        && data.get(24..28).is_some_and(|bytes| {
            f32::from_le_bytes(bytes.try_into().unwrap_or_default()).is_finite()
        })
}

fn replace_halo2_wrapped_function_byte_block(
    tag: &mut TagFile,
    block_path: &str,
    data: &[u8],
) -> Option<Result<(), String>> {
    let wrapper_path = block_path.strip_suffix("/function/data")?;
    let mut root = tag.root_mut();
    let mut wrapper_field = root.field_path_mut(wrapper_path)?;
    let mut wrapper = wrapper_field.as_struct_mut()?;
    let mut result = None;
    wrapper.for_each_field_mut(|mut field| {
        if result.is_some()
            || field.as_ref().name() != "function"
            || field.as_ref().field_type() != TagFieldType::Struct
        {
            return;
        }

        let Some(mut mapping) = field.as_struct_mut() else {
            return;
        };
        let Some(mut data_field) = mapping.field_mut("data") else {
            return;
        };
        let Some(mut block) = data_field.as_block_mut() else {
            return;
        };
        block.clear();
        for byte in data.iter().copied() {
            let index = block.add_element();
            let Some(mut element) = block.element_mut(index) else {
                result = Some(Err("failed to create function byte element".to_owned()));
                return;
            };
            let Some(mut value_field) = element.field_mut("Value") else {
                result = Some(Err("function byte element missing Value field".to_owned()));
                return;
            };
            if let Err(error) = value_field.set(TagFieldData::CharInteger(byte as i8)) {
                result = Some(Err(format!("{error:?}")));
                return;
            }
        }
        result = Some(Ok(()));
    });
    result
}

pub(in crate::app) fn apply_h2_shader_param_ops(
    tag: &mut TagFile,
    ops: Vec<H2ShaderParamOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply_one_h2_shader_param_op(tag, &op)
        }))
        .map_err(panic_message)
        .and_then(|result| result);
        match result {
            Ok(msg) => {
                *dirty = true;
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("H2 shader edit failed: {error}"));
            }
        }
    }
    status
}

pub(in crate::app) fn apply_one_h2_shader_param_op(
    tag: &mut TagFile,
    op: &H2ShaderParamOp,
) -> Result<String, String> {
    match op {
        H2ShaderParamOp::EnsureAnimationProperty {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
            animation_type_index,
            initial_function_data,
        } => {
            let parameter_index = ensure_h2_shader_parameter(
                tag,
                parameters_block_path,
                parameter_name,
                *parameter_type_index,
            )?;
            let animation_index = ensure_h2_animation_property(
                tag,
                parameters_block_path,
                parameter_index,
                *animation_type_index,
            )?;
            let data_path = format!(
                "{}[{}]/animation properties[{}]/function/data",
                parameters_block_path, parameter_index, animation_index
            );
            replace_halo2_function_byte_block(tag, &data_path, initial_function_data)?;
            Ok(format!(
                "Created H2 function row '{}' type {}",
                parameter_name, animation_type_index
            ))
        }
        H2ShaderParamOp::EditFunctionData { block_path, data } => {
            replace_halo2_function_byte_block(tag, block_path, data)?;
            Ok(format!("Edited H2 function data at {block_path}"))
        }
        H2ShaderParamOp::EditTemplateBackedValue {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
            field,
            input,
        } => {
            let index = ensure_h2_shader_parameter(
                tag,
                parameters_block_path,
                parameter_name,
                *parameter_type_index,
            )?;
            let path = format!(
                "{}[{}]/{}",
                parameters_block_path,
                index,
                escape_field_path_segment(field)
            );
            apply_field_edit(tag, &path, input)?;
            Ok(format!(
                "Edited H2 parameter '{}' {}",
                parameter_name, field
            ))
        }
        H2ShaderParamOp::SwitchTemplate {
            parameters_block_path,
            allowed_parameter_names,
        } => {
            let allowed = allowed_parameter_names
                .iter()
                .map(|name| name.to_ascii_lowercase())
                .collect::<std::collections::HashSet<_>>();
            let Some(block) = tag
                .root()
                .field_path(parameters_block_path)
                .and_then(|field| field.as_block())
            else {
                return Ok("Updated H2 shader template".to_owned());
            };
            let mut delete_indices = Vec::new();
            for index in 0..block.len() {
                let Some(parameter) = block.element(index) else {
                    continue;
                };
                let name = parameter
                    .read_string_id("name")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if name.is_empty() || !allowed.contains(&name) {
                    delete_indices.push(index);
                }
            }
            let removed = delete_indices.len();
            for index in delete_indices.into_iter().rev() {
                apply_one_block_op(
                    tag,
                    &BlockOp {
                        path: parameters_block_path.clone(),
                        kind: BlockOpKind::Delete(index),
                    },
                )?;
            }
            Ok(format!(
                "Updated H2 shader template; pruned {removed} parameter(s)"
            ))
        }
    }
}

fn ensure_h2_shader_parameter(
    tag: &mut TagFile,
    parameters_block_path: &str,
    parameter_name: &str,
    parameter_type_index: i32,
) -> Result<usize, String> {
    if let Some(index) = h2_shader_parameter_index(tag, parameters_block_path, parameter_name) {
        return Ok(index);
    }
    let index = add_block_element(tag, parameters_block_path)?;
    apply_field_edit(
        tag,
        &format!("{parameters_block_path}[{index}]/name"),
        parameter_name,
    )?;
    apply_field_edit(
        tag,
        &format!("{parameters_block_path}[{index}]/type"),
        &parameter_type_index.to_string(),
    )?;
    Ok(index)
}

fn h2_shader_parameter_index(
    tag: &TagFile,
    parameters_block_path: &str,
    parameter_name: &str,
) -> Option<usize> {
    let block = tag
        .root()
        .field_path(parameters_block_path)
        .and_then(|field| field.as_block())?;
    block.iter().enumerate().find_map(|(index, element)| {
        (element.read_string_id("name").as_deref() == Some(parameter_name)).then_some(index)
    })
}

fn ensure_h2_animation_property(
    tag: &mut TagFile,
    parameters_block_path: &str,
    parameter_index: usize,
    animation_type_index: i32,
) -> Result<usize, String> {
    let animation_block_path =
        format!("{parameters_block_path}[{parameter_index}]/animation properties");
    if let Some(index) =
        h2_animation_property_index(tag, &animation_block_path, animation_type_index)
    {
        return Ok(index);
    }
    let index = add_block_element(tag, &animation_block_path)?;
    apply_field_edit(
        tag,
        &format!("{animation_block_path}[{index}]/type"),
        &animation_type_index.to_string(),
    )?;
    Ok(index)
}

fn h2_animation_property_index(
    tag: &TagFile,

    animation_block_path: &str,
    animation_type_index: i32,
) -> Option<usize> {
    let block = tag
        .root()
        .field_path(animation_block_path)
        .and_then(|field| field.as_block())?;
    block.iter().enumerate().find_map(|(index, element)| {
        (element
            .read_int_any("type")
            .and_then(|value| i32::try_from(value).ok())
            == Some(animation_type_index))
        .then_some(index)
    })
}

pub(in crate::app) fn apply_one_block_op(
    tag: &mut TagFile,
    op: &BlockOp,
) -> Result<String, String> {
    let mut root = tag.root_mut();
    let mut field = root
        .field_path_mut(&op.path)
        .ok_or_else(|| "block path no longer resolves".to_owned())?;
    if let Some(mut block) = field.as_block_mut() {
        return match &op.kind {
            BlockOpKind::Add => {
                let idx = block.add_element();
                Ok(format!("Added element {idx} to {}", op.path))
            }
            BlockOpKind::Insert(i) => {
                block.insert_element(*i).map_err(|e| format!("{e:?}"))?;
                Ok(format!("Inserted element at {i} in {}", op.path))
            }
            BlockOpKind::Duplicate(i) => {
                let idx = block.duplicate_element(*i).map_err(|e| format!("{e:?}"))?;
                Ok(format!("Duplicated element {i} → {idx} in {}", op.path))
            }
            BlockOpKind::Delete(i) => {
                block.delete_element(*i).map_err(|e| format!("{e:?}"))?;
                Ok(format!("Deleted element {i} from {}", op.path))
            }
            BlockOpKind::DeleteAll => {
                block.clear();
                Ok(format!("Cleared {}", op.path))
            }
            BlockOpKind::Paste { at, elements } => {
                paste_elements(&mut block, *at, elements)?;
                Ok(format!(
                    "Pasted {} element(s) into {}",
                    elements.len(),
                    op.path
                ))
            }
            BlockOpKind::ReplaceElement { at, elements } => {
                block.delete_element(*at).map_err(|e| format!("{e:?}"))?;
                paste_elements(&mut block, *at, elements)?;
                Ok(format!(
                    "Replaced element {at} with {} element(s) in {}",
                    elements.len(),
                    op.path
                ))
            }
            BlockOpKind::ReplaceBlock { elements } => {
                block.clear();
                paste_elements(&mut block, 0, elements)?;
                Ok(format!(
                    "Replaced {} with {} element(s)",
                    op.path,
                    elements.len()
                ))
            }
        };
    }
    // Arrays are fixed-count: insert/delete can't apply, but an element can be
    // replaced in place with a copied element of the same struct.
    if let Some(mut array) = field.as_array_mut() {
        return match &op.kind {
            BlockOpKind::ReplaceElement { at, elements } => {
                let element = elements
                    .first()
                    .ok_or_else(|| "clipboard has no element".to_owned())?;
                array
                    .replace_element(*at, element)
                    .map_err(|error| format!("{error:?}"))?;
                Ok(format!("Replaced element {at} in {}", op.path))
            }
            _ => Err(
                "arrays are fixed-size — only replacing an element in place is supported"
                    .to_owned(),
            ),
        };
    }
    Err("field is not a block or array".to_owned())
}

/// Insert `elements` consecutively starting at `at`, preserving their order.
fn paste_elements(
    block: &mut blam_tags::TagBlockMut<'_>,
    at: usize,
    elements: &[blam_tags::TagBlockElement],
) -> Result<(), String> {
    for (offset, element) in elements.iter().enumerate() {
        block
            .paste_element(at + offset, element)
            .map_err(|e| format!("{e:?}"))?;
    }
    Ok(())
}

pub(in crate::app) fn apply_field_edit(
    tag: &mut TagFile,
    path: &str,
    input: &str,
) -> Result<(), String> {
    let mut root = tag.root_mut();
    let mut field = root
        .field_path_mut(path)
        .ok_or_else(|| "field path no longer resolves".to_owned())?;
    let field_ref = field.as_ref();
    if is_subchunk_backed_field(field_ref.field_type()) && field_ref.value().is_none() {
        return Err("field data is absent in this tag version".to_owned());
    }
    let value = parse_gui_field_value(&field_ref, input)?;
    field.set(value).map_err(|error| format!("{error:?}"))
}

fn is_subchunk_backed_field(field_type: TagFieldType) -> bool {
    matches!(
        field_type,
        TagFieldType::StringId
            | TagFieldType::OldStringId
            | TagFieldType::TagReference
            | TagFieldType::Data
            | TagFieldType::ApiInterop
    )
}

pub(in crate::app) fn apply_shader_ops(
    tag: &mut TagFile,
    ops: Vec<ShaderOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        match apply_one_shader_op(tag, &op) {
            Ok(msg) => {
                *dirty = true;
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("Shader op failed: {error}"));
            }
        }
    }
    status
}

pub(in crate::app) fn apply_shader_param_ops(
    tag: &mut TagFile,
    ops: Vec<ShaderParamOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        match apply_one_shader_param_op(tag, &op) {
            Ok(msg) => {
                *dirty = true;
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("Shader param op failed: {error}"));
            }
        }
    }
    status
}

pub(in crate::app) fn apply_model_variant_ops(
    tag: &mut TagFile,
    ops: Vec<ModelVariantOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        match apply_one_model_variant_op(tag, &op) {
            Ok(msg) => {
                *dirty = true;
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("Model variant edit failed: {error}"));
            }
        }
    }
    status
}

fn apply_one_model_variant_op(tag: &mut TagFile, op: &ModelVariantOp) -> Result<String, String> {
    match op {
        ModelVariantOp::Create { name, regions } => {
            let variant_index = add_block_element(tag, "variants")?;
            apply_field_edit(tag, &format!("variants[{variant_index}]/name"), name)?;
            write_model_variant_regions(tag, variant_index, regions)?;
            Ok(format!("Created model variant '{name}'"))
        }
        ModelVariantOp::Update {
            variant_index,
            regions,
        } => {
            ensure_block_element_exists(tag, "variants", *variant_index)?;
            write_model_variant_regions(tag, *variant_index, regions)?;
            Ok(format!("Updated model variant {}", variant_index))
        }
        ModelVariantOp::Drop { variant_index } => {
            let mut root = tag.root_mut();
            let mut field = root
                .field_path_mut("variants")
                .ok_or_else(|| "variants block not found".to_owned())?;
            let mut block = field
                .as_block_mut()
                .ok_or_else(|| "variants is not a block".to_owned())?;
            block
                .delete_element(*variant_index)
                .map_err(|e| format!("{e:?}"))?;
            Ok(format!("Deleted model variant {}", variant_index))
        }
    }
}

fn write_model_variant_regions(
    tag: &mut TagFile,
    variant_index: usize,
    regions: &[ModelVariantRegionChoice],
) -> Result<(), String> {
    let regions_path = format!("variants[{variant_index}]/regions");
    clear_block(tag, &regions_path)?;
    for region in regions {
        let region_index = add_block_element(tag, &regions_path)?;
        apply_field_edit(
            tag,
            &format!("{regions_path}[{region_index}]/region name"),
            &region.region_name,
        )?;
        let permutations_path = format!("{regions_path}[{region_index}]/permutations");
        let permutation_index = add_block_element(tag, &permutations_path)?;
        apply_field_edit(
            tag,
            &format!("{permutations_path}[{permutation_index}]/permutation name"),
            &region.permutation_name,
        )?;
    }
    Ok(())
}

fn ensure_block_element_exists(tag: &TagFile, path: &str, index: usize) -> Result<(), String> {
    let block = tag
        .root()
        .field_path(path)
        .and_then(|field| field.as_block())
        .ok_or_else(|| format!("{path} block not found"))?;
    if index < block.len() {
        Ok(())
    } else {
        Err(format!("{path}[{index}] is out of range"))
    }
}

pub(in crate::app) fn add_block_element(tag: &mut TagFile, path: &str) -> Result<usize, String> {
    let mut root = tag.root_mut();
    let mut field = root
        .field_path_mut(path)
        .ok_or_else(|| format!("{path} block not found"))?;
    let mut block = field
        .as_block_mut()
        .ok_or_else(|| format!("{path} is not a block"))?;
    Ok(block.add_element())
}

fn clear_block(tag: &mut TagFile, path: &str) -> Result<(), String> {
    let mut root = tag.root_mut();
    let mut field = root
        .field_path_mut(path)
        .ok_or_else(|| format!("{path} block not found"))?;
    let mut block = field
        .as_block_mut()
        .ok_or_else(|| format!("{path} is not a block"))?;
    block.clear();
    Ok(())
}

pub(in crate::app) fn apply_one_shader_param_op(
    tag: &mut TagFile,
    op: &ShaderParamOp,
) -> Result<String, String> {
    // Step 1: append a new element to the parameters block.
    let new_idx = {
        let mut root = tag.root_mut();
        let mut field = root
            .field_path_mut(&op.parameters_block_path)
            .ok_or_else(|| format!("parameters block not found: {}", op.parameters_block_path))?;
        let mut block = field
            .as_block_mut()
            .ok_or_else(|| format!("not a block: {}", op.parameters_block_path))?;
        block.add_element()
    };

    // Step 2: write parameter name.
    let name_path = format!("{}[{}]/parameter name", op.parameters_block_path, new_idx);
    apply_field_edit(tag, &name_path, &op.parameter_name)?;

    // Step 3: initialise requested fields.
    for initial in &op.initial_fields {
        let field = escape_field_path_segment(&initial.field);
        let field_path = format!("{}[{}]/{}", op.parameters_block_path, new_idx, field);
        apply_field_edit(tag, &field_path, &initial.input)?;
    }

    for animated in &op.animated_parameters {
        let animated_block_path = format!(
            "{}[{}]/animated parameters",
            op.parameters_block_path, new_idx
        );
        apply_one_shader_op(
            tag,
            &ShaderOp {
                animated_block_path,
                output_type_index: animated.output_type_index,
                initial_function_hex: animated.initial_function_hex.clone(),
            },
        )?;
    }

    Ok(format!(
        "Created parameter '{}' at {}[{}]",
        op.parameter_name, op.parameters_block_path, new_idx
    ))
}

pub(in crate::app) fn apply_one_shader_op(
    tag: &mut TagFile,
    op: &ShaderOp,
) -> Result<String, String> {
    // Step 1: append one element to the animated-parameters block and capture its index.
    let new_idx = {
        let mut root = tag.root_mut();
        let mut field = root
            .field_path_mut(&op.animated_block_path)
            .ok_or_else(|| {
                format!(
                    "animated params block not found: {}",
                    op.animated_block_path
                )
            })?;
        let mut block = field
            .as_block_mut()
            .ok_or_else(|| format!("not a block: {}", op.animated_block_path))?;
        block.add_element()
    };

    // Step 2: set the output `type` field on the newly created element.
    let type_path = format!("{}[{}]/type", op.animated_block_path, new_idx);
    apply_field_edit(tag, &type_path, &op.output_type_index.to_string())?;

    // Step 3: write the initial `mapping_function` blob into `function/data`.
    let data_path = format!("{}[{}]/function/data", op.animated_block_path, new_idx);
    apply_field_edit(tag, &data_path, &op.initial_function_hex)?;

    Ok(format!(
        "Added animated parameter (type {}) at {}[{}]",
        op.output_type_index, op.animated_block_path, new_idx
    ))
}
