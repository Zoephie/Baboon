use super::*;

pub(super) fn draw_entry_header(ui: &mut Ui, entry: &TagEntry, names: &TagNameIndex) {
    ui.heading(RichText::new(&entry.display_path).color(text_dark()));
    ui.horizontal(|ui| {
        ui.label(RichText::new("Group:").color(subtle_dark()));
        ui.monospace(RichText::new(group_label(names, entry.group_tag)).color(text_dark()));
        if let Some(name) = &entry.group_name {
            ui.label(RichText::new(name).color(subtle_dark()));
        }
    });
    ui.separator();
}

pub(super) fn draw_tag(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    rmdf_cache: &mut HashMap<String, Option<RenderMethodDefinition>>,
    rmop_cache: &mut HashMap<String, Option<RenderMethodOption>>,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    model_preview: &mut ModelPreviewState,
    model_preview_size: &mut f32,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
) {
    let is_object_family = is_object_family_group(entry.group_tag);
    let is_shaderish =
        is_material_tag(entry) || is_material_shader_tag(entry) || is_shader_tag(entry);
    let is_model = is_model_group(entry.group_tag, names);

    draw_tag_metadata(ui, tag, names);
    if !is_object_family {
        draw_object_model_summary(ui, tag, entry, names, edit);
    }

    if is_model {
        draw_model_tag_panel_tabs(ui, model_preview);
    }
    ui.add_space(6.0);

    if is_model && model_preview.active_tab == ModelTagPanelTab::RenderModel {
        draw_model_preview_panel(
            ui,
            tag,
            entry,
            names,
            source,
            model_preview,
            model_preview_size,
            edit,
        );
        return;
    }

    draw_tag_fields_scroll(
        ui,
        tag,
        entry,
        names,
        source,
        rmdf_cache,
        rmop_cache,
        color_popup,
        function_popup,
        expert_mode,
        edit,
        is_object_family,
        is_shaderish,
    );
}

fn draw_model_tag_panel_tabs(ui: &mut Ui, model_preview: &mut ModelPreviewState) {
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut model_preview.active_tab,
            ModelTagPanelTab::Fields,
            "Fields",
        );
        ui.selectable_value(
            &mut model_preview.active_tab,
            ModelTagPanelTab::RenderModel,
            "Render model",
        );
    });
}

fn draw_tag_fields_scroll(
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
    is_object_family: bool,
    is_shaderish: bool,
) {
    let scroll_height = ui.available_height().max(0.0);
    if is_shaderish {
        // The Guerilla-style shader grid is the single editing surface — no
        // separate field tab. The grid's bitmap/scalar/int/function/category
        // cells are editable inline; when the grid can't be built it falls
        // back to the standard editable field tree (inside draw_material_tag).
        ScrollArea::both()
            .id_salt(("tag_scroll", edit.view_scope, edit.tag_key))
            .max_height(scroll_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(TAG_FIELD_SCROLL_MIN_WIDTH);
                draw_material_tag(
                    ui,
                    tag,
                    entry,
                    names,
                    source,
                    rmdf_cache,
                    rmop_cache,
                    color_popup,
                    function_popup,
                    expert_mode,
                    edit,
                );
            });
        return;
    }

    ScrollArea::both()
        .id_salt(("tag_scroll", edit.view_scope, edit.tag_key))
        .max_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(TAG_FIELD_SCROLL_MIN_WIDTH);
            if is_object_family {
                draw_inherited_object_fields(ui, tag.root(), names, expert_mode, edit);
            } else {
                draw_struct_fields(ui, tag.root(), names, 0, expert_mode, "", edit);
            }
        });
}

const TAG_FIELD_SCROLL_MIN_WIDTH: f32 = 980.0;

/// Object-family tag groups (each derives from `obje` and carries a `.model`
/// reference). For these we surface the connected model at the very top.
pub(super) fn is_object_family_group(group_tag: u32) -> bool {
    matches!(
        &group_tag.to_be_bytes(),
        b"bipd" // biped
            | b"vehi" // vehicle
            | b"weap" // weapon
            | b"eqip" // equipment
            | b"scen" // scenery
            | b"mach" // device_machine
            | b"ctrl" // device_control
            | b"crat" // crate
            | b"bloc" // crate-like block
            | b"ssce" // sound_scenery
            | b"gint" // giant
            | b"proj" // projectile
            | b"obje" // object (base)
    )
}

/// Show the connected `.model` reference at the top of object-family tags
/// (biped, vehicle, weapon, scenery, …) with a working Open button.
pub(super) fn draw_object_model_summary(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    edit: &mut FieldEditContext<'_>,
) {
    if !is_object_family_group(entry.group_tag) {
        return;
    }
    let Some(model) = find_model_reference(tag.root(), names, 0, "") else {
        return;
    };
    let formatted = format_reference_path(names, model.group_tag, &model.rel_path);
    let meta = FieldDisplayMeta {
        label: "model".to_owned(),
        unit: None,
        help: Some("Object model tag reference".to_owned()),
        read_only: false,
        advanced: false,
    };
    ui.add_space(4.0);
    let import_verb = geometry_import_verb(names, model.group_tag);
    draw_foundation_tag_reference_row(
        ui,
        &meta,
        &formatted,
        Some((model.group_tag, model.rel_path)),
        import_verb,
        0,
        &model.field_path,
        edit,
    );
}

pub(super) struct ModelReferenceInfo {
    pub(super) group_tag: u32,
    pub(super) rel_path: String,
    pub(super) field_path: String,
}

/// Like `find_model_reference` but returns the raw `(group_tag, rel_path)` so
/// the caller can resolve/open the target.
pub(super) fn find_model_reference(
    tag_struct: TagStruct<'_>,
    names: &TagNameIndex,
    depth: usize,
    path_prefix: &str,
) -> Option<ModelReferenceInfo> {
    if depth > 8 {
        return None;
    }
    for field in tag_struct.fields() {
        let field_path = append_field_path(path_prefix, field.name());
        if let Some(value) = field.value() {
            if let TagFieldData::TagReference(reference) = value {
                if let Some((group_tag, path)) = reference.group_tag_and_name.as_ref() {
                    if is_model_group(*group_tag, names) && !path.is_empty() {
                        return Some(ModelReferenceInfo {
                            group_tag: *group_tag,
                            rel_path: path.clone(),
                            field_path,
                        });
                    }
                }
            }
            continue;
        }
        if let Some(nested) = field.as_struct() {
            if let Some(found) = find_model_reference(nested, names, depth + 1, &field_path) {
                return Some(found);
            }
        } else if let Some(block) = field.as_block() {
            for (index, element) in block.iter().take(4).enumerate() {
                let element_path = format!("{field_path}[{index}]");
                if let Some(found) = find_model_reference(element, names, depth + 1, &element_path)
                {
                    return Some(found);
                }
            }
        } else if let Some(array) = field.as_array() {
            for (index, element) in array.iter().take(8).enumerate() {
                let element_path = format!("{field_path}[{index}]");
                if let Some(found) = find_model_reference(element, names, depth + 1, &element_path)
                {
                    return Some(found);
                }
            }
        }
    }
    None
}

pub(super) fn is_model_group(group_tag: u32, names: &TagNameIndex) -> bool {
    group_tag == u32::from_be_bytes(*b"hlmt")
        || names.name_for(group_tag) == Some("model")
        || group_tag_to_extension(group_tag) == Some("model")
}

pub(super) fn format_reference_path(names: &TagNameIndex, group_tag: u32, path: &str) -> String {
    if let Some(extension) = names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag))
    {
        format!("{path}.{extension}")
    } else {
        format!("{}:{path}", format_group_tag(group_tag))
    }
}

pub(super) fn apply_pending_edits(
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

pub(super) fn apply_block_ops(
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

pub(super) fn apply_function_data_ops(
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

pub(super) fn replace_halo2_function_byte_block(
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

pub(super) fn apply_h2_shader_param_ops(
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

pub(super) fn apply_one_h2_shader_param_op(
    tag: &mut TagFile,
    op: &H2ShaderParamOp,
) -> Result<String, String> {
    match op {
        H2ShaderParamOp::EnsureParameter {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
        } => {
            let index = ensure_h2_shader_parameter(
                tag,
                parameters_block_path,
                parameter_name,
                *parameter_type_index,
            )?;
            Ok(format!(
                "Ensured H2 parameter '{}' at {}[{}]",
                parameter_name, parameters_block_path, index
            ))
        }
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

pub(super) fn apply_one_block_op(tag: &mut TagFile, op: &BlockOp) -> Result<String, String> {
    let mut root = tag.root_mut();
    let mut field = root
        .field_path_mut(&op.path)
        .ok_or_else(|| "block path no longer resolves".to_owned())?;
    let mut block = field
        .as_block_mut()
        .ok_or_else(|| "field is not a block".to_owned())?;
    match &op.kind {
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
    }
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

pub(super) fn apply_field_edit(tag: &mut TagFile, path: &str, input: &str) -> Result<(), String> {
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

pub(super) fn apply_shader_ops(
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

pub(super) fn apply_shader_param_ops(
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

pub(super) fn apply_model_variant_ops(
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

fn add_block_element(tag: &mut TagFile, path: &str) -> Result<usize, String> {
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

pub(super) fn apply_one_shader_param_op(
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

pub(super) fn apply_one_shader_op(tag: &mut TagFile, op: &ShaderOp) -> Result<String, String> {
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

pub(super) fn is_editable_tag(entry: &TagEntry, tag: &TagFile) -> bool {
    matches!(entry.location, TagEntryLocation::LooseFile(_))
        && (tag.classic_engine().is_some() || tag.endian == Endian::Le)
}

pub(super) fn append_field_path(prefix: &str, field_name: &str) -> String {
    if prefix.is_empty() {
        field_name.to_owned()
    } else {
        format!("{prefix}/{field_name}")
    }
}

pub(super) fn escape_field_path_segment(field_name: &str) -> String {
    field_name.replace('\\', "\\\\").replace('/', "\\/")
}

pub(super) fn is_text_editable_value(value: &TagFieldData) -> bool {
    !matches!(
        value,
        TagFieldData::Data(_)
            | TagFieldData::ApiInterop(_)
            | TagFieldData::Custom(_)
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
            | TagFieldData::RgbColor(_)
            | TagFieldData::ArgbColor(_)
            | TagFieldData::RealRgbColor(_)
            | TagFieldData::RealArgbColor(_)
            | TagFieldData::RealHsvColor(_)
            | TagFieldData::RealAhsvColor(_)
            | TagFieldData::ShortIntegerBounds(_)
            | TagFieldData::AngleBounds(_)
            | TagFieldData::RealBounds(_)
            | TagFieldData::FractionBounds(_)
    )
}

pub(super) fn parse_gui_field_value(
    field: &TagField<'_>,
    input: &str,
) -> Result<TagFieldData, String> {
    let trimmed = input.trim();
    match field.field_type() {
        TagFieldType::CharInteger => parse_value(trimmed, "i8").map(TagFieldData::CharInteger),
        TagFieldType::ShortInteger => parse_value(trimmed, "i16").map(TagFieldData::ShortInteger),
        TagFieldType::LongInteger => parse_value(trimmed, "i32").map(TagFieldData::LongInteger),
        TagFieldType::Int64Integer => parse_value(trimmed, "i64").map(TagFieldData::Int64Integer),
        TagFieldType::ByteInteger => parse_value(trimmed, "u8").map(TagFieldData::ByteInteger),
        TagFieldType::WordInteger => parse_value(trimmed, "u16").map(TagFieldData::WordInteger),
        TagFieldType::DwordInteger => parse_value(trimmed, "u32").map(TagFieldData::DwordInteger),
        TagFieldType::QwordInteger => parse_value(trimmed, "u64").map(TagFieldData::QwordInteger),
        TagFieldType::Tag => parse_group_tag(trimmed)
            .map(TagFieldData::Tag)
            .ok_or_else(|| "expected 1..=4 ASCII group tag".to_owned()),
        TagFieldType::Angle => parse_value(trimmed, "f32").map(TagFieldData::Angle),
        TagFieldType::ShortIntegerBounds => {
            let (lower, upper) = parse_short_bounds(trimmed, "short bounds")?;
            Ok(TagFieldData::ShortIntegerBounds(
                blam_tags::math::ShortBounds { lower, upper },
            ))
        }
        TagFieldType::AngleBounds => {
            let (lower, upper) = parse_float_bounds(trimmed, "angle bounds")?;
            Ok(TagFieldData::AngleBounds(blam_tags::math::AngleBounds {
                lower,
                upper,
            }))
        }
        TagFieldType::RealBounds => {
            let (lower, upper) = parse_float_bounds(trimmed, "real bounds")?;
            Ok(TagFieldData::RealBounds(blam_tags::math::RealBounds {
                lower,
                upper,
            }))
        }
        TagFieldType::FractionBounds => {
            let (lower, upper) = parse_float_bounds(trimmed, "fraction bounds")?;
            Ok(TagFieldData::FractionBounds(
                blam_tags::math::FractionBounds { lower, upper },
            ))
        }
        TagFieldType::RealVector2d => {
            let [i, j] = parse_float_channels::<2>(trimmed, "real vector 2d")?;
            Ok(TagFieldData::RealVector2d(blam_tags::math::RealVector2d {
                i,
                j,
            }))
        }
        TagFieldType::RealVector3d => {
            let [i, j, k] = parse_float_channels::<3>(trimmed, "real vector 3d")?;
            Ok(TagFieldData::RealVector3d(blam_tags::math::RealVector3d {
                i,
                j,
                k,
            }))
        }
        TagFieldType::RealPoint2d => {
            let [x, y] = parse_float_channels::<2>(trimmed, "real point 2d")?;
            Ok(TagFieldData::RealPoint2d(blam_tags::math::RealPoint2d {
                x,
                y,
            }))
        }
        TagFieldType::RealPoint3d => {
            let [x, y, z] = parse_float_channels::<3>(trimmed, "real point 3d")?;
            Ok(TagFieldData::RealPoint3d(blam_tags::math::RealPoint3d {
                x,
                y,
                z,
            }))
        }
        TagFieldType::RealQuaternion => {
            let [i, j, k, w] = parse_float_channels::<4>(trimmed, "real quaternion")?;
            Ok(TagFieldData::RealQuaternion(
                blam_tags::math::RealQuaternion { i, j, k, w },
            ))
        }
        TagFieldType::Real => parse_value(trimmed, "f32").map(TagFieldData::Real),
        TagFieldType::RealSlider => parse_value(trimmed, "f32").map(TagFieldData::RealSlider),
        TagFieldType::RealFraction => parse_value(trimmed, "f32").map(TagFieldData::RealFraction),
        TagFieldType::CharEnum => Ok(TagFieldData::CharEnum {
            value: parse_enum_value(field, trimmed)? as i8,
            name: None,
        }),
        TagFieldType::ShortEnum => Ok(TagFieldData::ShortEnum {
            value: parse_enum_value(field, trimmed)? as i16,
            name: None,
        }),
        TagFieldType::LongEnum => Ok(TagFieldData::LongEnum {
            value: parse_enum_value(field, trimmed)?,
            name: None,
        }),
        TagFieldType::ByteFlags => Ok(TagFieldData::ByteFlags {
            value: parse_int_mask(trimmed)? as u8,
            names: Vec::new(),
        }),
        TagFieldType::WordFlags => Ok(TagFieldData::WordFlags {
            value: parse_int_mask(trimmed)? as u16,
            names: Vec::new(),
        }),
        TagFieldType::LongFlags => Ok(TagFieldData::LongFlags {
            value: parse_int_mask(trimmed)? as i32,
            names: Vec::new(),
        }),
        TagFieldType::ByteBlockFlags => {
            Ok(TagFieldData::ByteBlockFlags(parse_int_mask(trimmed)? as u8))
        }
        TagFieldType::WordBlockFlags => {
            Ok(TagFieldData::WordBlockFlags(parse_int_mask(trimmed)? as u16))
        }
        TagFieldType::LongBlockFlags => {
            Ok(TagFieldData::LongBlockFlags(parse_int_mask(trimmed)? as i32))
        }
        TagFieldType::CharBlockIndex => Ok(TagFieldData::CharBlockIndex(
            parse_block_index(trimmed)? as i8,
        )),
        TagFieldType::CustomCharBlockIndex => Ok(TagFieldData::CustomCharBlockIndex(
            parse_block_index(trimmed)? as i8,
        )),
        TagFieldType::ShortBlockIndex => Ok(TagFieldData::ShortBlockIndex(parse_block_index(
            trimmed,
        )? as i16)),
        TagFieldType::CustomShortBlockIndex => Ok(TagFieldData::CustomShortBlockIndex(
            parse_block_index(trimmed)? as i16,
        )),
        TagFieldType::LongBlockIndex => {
            Ok(TagFieldData::LongBlockIndex(parse_block_index(trimmed)?))
        }
        TagFieldType::CustomLongBlockIndex => Ok(TagFieldData::CustomLongBlockIndex(
            parse_block_index(trimmed)?,
        )),
        TagFieldType::String => Ok(TagFieldData::String(trimmed.to_owned())),
        TagFieldType::LongString => Ok(TagFieldData::LongString(trimmed.to_owned())),
        TagFieldType::StringId => Ok(TagFieldData::StringId(StringIdData {
            string: parse_none_string(trimmed),
        })),
        TagFieldType::OldStringId => Ok(TagFieldData::OldStringId(StringIdData {
            string: parse_none_string(trimmed),
        })),
        TagFieldType::TagReference => parse_tag_reference(trimmed).map(TagFieldData::TagReference),
        // Color values: comma-separated floats, written by the color picker
        // swatch. RGB = "r, g, b"; ARGB = "a, r, g, b".
        TagFieldType::RgbColor => {
            let (_, r, g, b) = parse_rgb_or_argb_color_channels(trimmed)?;
            let raw = ((color_float_to_u8(r) as u32) << 16)
                | ((color_float_to_u8(g) as u32) << 8)
                | color_float_to_u8(b) as u32;
            Ok(TagFieldData::RgbColor(blam_tags::math::RgbColor(raw)))
        }
        TagFieldType::ArgbColor => {
            let (a, r, g, b) = parse_rgb_or_argb_color_channels(trimmed)?;
            let raw = ((color_float_to_u8(a) as u32) << 24)
                | ((color_float_to_u8(r) as u32) << 16)
                | ((color_float_to_u8(g) as u32) << 8)
                | color_float_to_u8(b) as u32;
            Ok(TagFieldData::ArgbColor(blam_tags::math::ArgbColor(raw)))
        }
        TagFieldType::RealRgbColor => {
            let [r, g, b] = parse_color_channels::<3>(trimmed)?;
            Ok(TagFieldData::RealRgbColor(blam_tags::math::RealRgbColor {
                red: r,
                green: g,
                blue: b,
            }))
        }
        TagFieldType::RealArgbColor => {
            let [a, r, g, b] = parse_color_channels::<4>(trimmed)?;
            Ok(TagFieldData::RealArgbColor(
                blam_tags::math::RealArgbColor {
                    alpha: a,
                    red: r,
                    green: g,
                    blue: b,
                },
            ))
        }
        // Raw byte blobs (e.g. a `mapping_function` `data` field) are
        // carried through the string edit channel as lowercase hex. The
        // function editor produces these via `push_function_edit`.
        TagFieldType::Data => decode_hex(trimmed).map(TagFieldData::Data),
        _ => Err(format!(
            "editing {} fields is not supported yet",
            field.type_name()
        )),
    }
}

/// Decode a contiguous lowercase/uppercase hex string (no separators)
/// into bytes. Used to ferry function blobs through `PendingFieldEdit`.
pub(super) fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    let s = input.trim();
    if s.len() % 2 != 0 {
        return Err("hex blob must have an even number of digits".to_owned());
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = (pair[0] as char)
            .to_digit(16)
            .ok_or_else(|| "invalid hex digit".to_owned())?;
        let lo = (pair[1] as char)
            .to_digit(16)
            .ok_or_else(|| "invalid hex digit".to_owned())?;
        out.push(((hi << 4) | lo) as u8);
    }
    Ok(out)
}

/// Encode bytes as a contiguous lowercase hex string.
pub(super) fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0xF) as u32, 16).unwrap());
    }
    out
}

pub(super) fn parse_value<T: std::str::FromStr>(input: &str, expected: &str) -> Result<T, String> {
    input
        .parse()
        .map_err(|_| format!("expected {expected} value"))
}

/// Parse exactly `N` comma-separated float channels (used for color values).
pub(super) fn parse_color_channels<const N: usize>(input: &str) -> Result<[f32; N], String> {
    let parts: Vec<f32> = input
        .split(',')
        .map(|part| part.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("expected {N} comma-separated color channels"))?;
    parts
        .try_into()
        .map_err(|_: Vec<f32>| format!("expected {N} comma-separated color channels"))
}

pub(super) fn parse_rgb_or_argb_color_channels(
    input: &str,
) -> Result<(f32, f32, f32, f32), String> {
    let parts = input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| parse_value::<f32>(part, "color channel"))
        .collect::<Result<Vec<_>, _>>()?;
    match parts.as_slice() {
        [r, g, b] => Ok((1.0, *r, *g, *b)),
        [a, r, g, b] => Ok((*a, *r, *g, *b)),
        _ => Err("expected 3 or 4 comma-separated color channels".to_owned()),
    }
}

pub(super) fn color_float_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub(super) fn parse_float_channels<const N: usize>(
    input: &str,
    expected: &str,
) -> Result<[f32; N], String> {
    let parts = if input.contains(',') {
        input
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
    } else {
        input.split_whitespace().collect::<Vec<_>>()
    };
    let values = parts
        .into_iter()
        .map(|part| part.parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| format!("expected {N} values for {expected}"))?;
    values
        .try_into()
        .map_err(|_: Vec<f32>| format!("expected {N} values for {expected}"))
}

pub(super) fn parse_float_bounds(input: &str, expected: &str) -> Result<(f32, f32), String> {
    let (lower, upper) = parse_bounds_parts(input, expected)?;
    Ok((
        lower
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
        upper
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
    ))
}

pub(super) fn parse_short_bounds(input: &str, expected: &str) -> Result<(i16, i16), String> {
    let (lower, upper) = parse_bounds_parts(input, expected)?;
    Ok((
        lower
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
        upper
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
    ))
}

pub(super) fn parse_bounds_parts<'a>(
    input: &'a str,
    expected: &str,
) -> Result<(&'a str, &'a str), String> {
    if let Some((lower, upper)) = input.split_once("..") {
        return Ok((lower.trim(), upper.trim()));
    }
    if let Some((lower, upper)) = input.split_once(" to ") {
        return Ok((lower.trim(), upper.trim()));
    }

    let parts = if input.contains(',') {
        input
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
    } else {
        input.split_whitespace().collect::<Vec<_>>()
    };
    let [lower, upper]: [&str; 2] = parts
        .try_into()
        .map_err(|_| format!("expected {expected} as lower..upper"))?;
    Ok((lower, upper))
}

pub(super) fn parse_none_string(input: &str) -> String {
    if input.eq_ignore_ascii_case("none") {
        String::new()
    } else {
        input.to_owned()
    }
}

pub(super) fn parse_block_index(input: &str) -> Result<i32, String> {
    if input.eq_ignore_ascii_case("none") {
        Ok(-1)
    } else {
        parse_value(input, "block index")
    }
}

pub(super) fn parse_int_mask(input: &str) -> Result<u64, String> {
    if let Some(hex) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).map_err(|_| "expected integer mask".to_owned())
    } else {
        input
            .parse()
            .map_err(|_| "expected integer mask".to_owned())
    }
}

pub(super) fn parse_enum_value(field: &TagField<'_>, input: &str) -> Result<i32, String> {
    if let Ok(value) = input.parse() {
        return Ok(value);
    }
    if let Some(blam_tags::TagOptions::Enum { names, .. }) = field.options() {
        if let Some((index, _)) = names
            .iter()
            .enumerate()
            .find(|(_, name)| name.eq_ignore_ascii_case(input))
        {
            return Ok(index as i32);
        }
    }
    Err("expected enum name or integer".to_owned())
}

pub(super) fn parse_tag_reference(input: &str) -> Result<TagReferenceData, String> {
    if input.eq_ignore_ascii_case("none") || input.is_empty() {
        return Ok(TagReferenceData {
            group_tag_and_name: None,
        });
    }
    if let Some((group, path)) = input.split_once(':') {
        let group_tag = parse_group_tag(group)
            .ok_or_else(|| "tag reference group must be 1..=4 ASCII chars".to_owned())?;
        return Ok(TagReferenceData {
            group_tag_and_name: Some((group_tag, path.replace('/', "\\"))),
        });
    }
    let Some((path, extension)) = input.rsplit_once('.') else {
        return Err("expected <path>.<group> or GROUP:<path>".to_owned());
    };
    let group_tag = extension_to_group_tag(extension)
        .or_else(|| parse_group_tag(extension))
        .ok_or_else(|| format!("unknown tag group {extension:?}"))?;
    Ok(TagReferenceData {
        group_tag_and_name: Some((group_tag, path.replace('/', "\\"))),
    })
}

pub(super) fn field_display_meta(name: &str) -> FieldDisplayMeta {
    let mut text = name.trim().to_owned();
    let advanced = text.ends_with('*');
    if advanced {
        text.pop();
    }
    let read_only = text.ends_with('!');
    if read_only {
        text.pop();
    }
    let (text, help) = match text.split_once('#') {
        Some((label, help)) => (label.trim().to_owned(), Some(help.trim().to_owned())),
        None => (text, None),
    };
    let (label, unit) = match text.split_once(':') {
        Some((label, unit)) => (label.trim().to_owned(), Some(unit.trim().to_owned())),
        None => (text.trim().to_owned(), None),
    };
    FieldDisplayMeta {
        label: clean_field_name_basic(&label),
        unit: unit.filter(|unit| !unit.is_empty()),
        help: help.filter(|help| !help.is_empty()),
        read_only,
        advanced,
    }
}

pub(super) fn field_suffix(meta: &FieldDisplayMeta, type_name: &str) -> String {
    meta.unit
        .clone()
        .unwrap_or_else(|| clean_type_name(type_name))
}

pub(super) fn draw_field_help(ui: &mut Ui, meta: &FieldDisplayMeta) {
    if let Some(help) = &meta.help {
        ui.label(
            RichText::new("?")
                .color(Color32::from_rgb(40, 128, 168))
                .small(),
        )
        .on_hover_text(help);
    }
    if meta.read_only {
        ui.label(RichText::new("read-only").color(subtle_dark()).small());
    }
}

pub(super) fn enum_option_label(options: &[&str], selected: i64) -> String {
    if selected < 0 {
        return "NONE".to_owned();
    }
    options
        .get(selected as usize)
        .map(|name| format!("{selected}. {name}"))
        .unwrap_or_else(|| selected.to_string())
}

pub(super) fn extension_to_group_tag(extension: &str) -> Option<u32> {
    let fourcc = match extension {
        "material" => "mat",
        "material_shader" => "mats",
        "material_effects" => "foot",
        "object" => "obje",
        "model" => "hlmt",
        "character" => "char",
        "style" => "styl",
        "unit" => "unit",
        "render_model" => "mode",
        "collision_model" => "coll",
        "physics_model" => "phmo",
        "model_animation_graph" => "jmad",
        "biped" => "bipd",
        "vehicle" => "vehi",
        "weapon" => "weap",
        "equipment" => "eqip",
        "item" => "item",
        "giant" => "gint",
        "creature" => "crea",
        "scenery" => "scen",
        "crate" => "crat",
        "bitmap" => "bitm",
        "scenario_structure_bsp" => "sbsp",
        "scenario" => "scnr",
        "projectile" => "proj",
        "effect" => "effe",
        "effect_scenery" => "efsc",
        "damage_effect" => "jpt!",
        "sound" => "snd!",
        "sound_looping" => "lsnd",
        "sound_scenery" => "ssce",
        "dialogue" => "udlg",
        "light" => "ligh",
        "lens_flare" => "lens",
        "camera_track" => "trak",
        "device" => "devi",
        "device_control" => "ctrl",
        "device_machine" => "mach",
        "device_terminal" => "term",
        "globals" => "matg",
        "shader" => "rmsh",
        "shader_terrain" => "rmtr",
        "shader_water" => "rmw ",
        "shader_foliage" => "rmfl",
        "shader_decal" => "rmd ",
        "shader_halogram" => "rmhg",
        "shader_skin" => "rmsk",
        "shader_cortana" => "rmct",
        "shader_custom" => "rmcs",
        "shader_particle" => "rmp ",
        "shader_beam" => "rmb ",
        "shader_contrail" => "rmco",
        "shader_light_volume" => "rmlv",
        _ => return None,
    };
    parse_group_tag(fourcc)
}

pub(super) fn draw_tag_metadata(ui: &mut Ui, tag: &TagFile, names: &TagNameIndex) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Header group:").color(subtle_dark()));
        ui.monospace(RichText::new(group_label(names, tag.group().tag)).color(text_dark()));
        ui.label(RichText::new("Version:").color(subtle_dark()));
        ui.monospace(RichText::new(tag.group().version.to_string()).color(text_dark()));
        ui.label(RichText::new("Endian:").color(subtle_dark()));
        ui.monospace(
            RichText::new(match tag.endian {
                Endian::Le => "LE",
                Endian::Be => "BE",
            })
            .color(text_dark()),
        );
    });
}

pub(super) fn draw_bitmap_tag(
    ui: &mut Ui,
    ctx: &egui::Context,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    _color_popup: &mut Option<MaterialColorPopup>,
    preview: &mut BitmapPreviewState,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
) {
    draw_tag_metadata(ui, tag, names);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let can_reimport = bitmap_reimport_data_path(entry, edit.tags_root).is_some();
        if ui
            .add_enabled(can_reimport, egui::Button::new("Reimport"))
            .on_hover_text("Run tool bitmaps for this bitmap source path, then reload the tag")
            .clicked()
        {
            *edit.bitmap_reimport = Some(entry.key.clone());
        }
        ui.separator();
        ui.selectable_value(&mut preview.active_tab, BitmapPanelTab::Fields, "Fields");
        ui.selectable_value(
            &mut preview.active_tab,
            BitmapPanelTab::Texture,
            "Texture preview",
        );
    });
    ui.separator();

    match preview.active_tab {
        BitmapPanelTab::Fields => {
            ScrollArea::both()
                .id_salt(("bitmap_fields_scroll", edit.view_scope, edit.tag_key))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(TAG_FIELD_SCROLL_MIN_WIDTH);
                    draw_struct_fields(ui, tag.root(), names, 0, expert_mode, "", edit);
                });
        }
        BitmapPanelTab::Texture => draw_bitmap_preview(ui, ctx, tag, entry, preview),
    }
}

pub(super) fn bitmap_reimport_data_path(
    entry: &TagEntry,
    tags_root: Option<&Path>,
) -> Option<String> {
    let TagEntryLocation::LooseFile(path) = &entry.location else {
        return None;
    };
    let tags_root = tags_root?;
    let rel = path.strip_prefix(tags_root).ok()?;
    let mut source = rel.to_path_buf();
    source.set_extension("");
    Some(source.to_string_lossy().replace('/', "\\"))
}

pub(super) fn draw_bitmap_preview(
    ui: &mut Ui,
    ctx: &egui::Context,
    tag: &TagFile,
    entry: &TagEntry,
    preview: &mut BitmapPreviewState,
) {
    if preview.decoded.is_none() {
        preview.decoded = Some(build_bitmap_preview(tag).map_err(|error| error.to_string()));
        preview.texture_dirty = true;
    }

    let Some(decoded) = preview.decoded.as_ref() else {
        return;
    };
    let data = match decoded {
        Ok(data) => data,
        Err(error) => {
            ui.colored_label(Color32::from_rgb(130, 32, 24), error);
            return;
        }
    };

    ui.horizontal(|ui| {
        let red_changed = ui.checkbox(&mut preview.show_red, "Red").changed();
        let green_changed = ui.checkbox(&mut preview.show_green, "Green").changed();
        let blue_changed = ui.checkbox(&mut preview.show_blue, "Blue").changed();
        let alpha_changed = ui.checkbox(&mut preview.show_alpha, "Alpha").changed();
        if red_changed || green_changed || blue_changed || alpha_changed {
            preview.texture_dirty = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Image 0").color(subtle_dark()));
        ui.monospace(RichText::new(format!("{} x {}", data.width, data.height)).color(text_dark()));
        ui.label(RichText::new(&data.format_name).color(subtle_dark()));
        ui.label(RichText::new(&data.type_name).color(subtle_dark()));
        if data.image_count > 1 {
            ui.label(RichText::new(format!("{} images", data.image_count)).color(subtle_dark()));
        }
        ui.separator();
        ui.label(RichText::new(format!("Zoom {:.0}%", preview.zoom * 100.0)).color(subtle_dark()));
        if ui.button("Reset zoom").clicked() {
            preview.zoom_initialized = false; // triggers fit-to-view on next frame
            preview.pan = Vec2::ZERO;
        }
    });
    ui.add_space(6.0);

    if preview.texture_dirty || preview.texture.is_none() {
        let rgba = filtered_bitmap_rgba(data, preview);
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [data.width as usize, data.height as usize],
            &rgba,
        );
        if let Some(texture) = preview.texture.as_mut() {
            texture.set(image, egui::TextureOptions::NEAREST);
        } else {
            preview.texture = Some(ctx.load_texture(
                format!("bitmap_preview_{}", entry.key),
                image,
                egui::TextureOptions::NEAREST,
            ));
        }
        preview.texture_dirty = false;
    }

    let Some(texture) = preview.texture.as_ref() else {
        return;
    };
    let image_size = texture.size_vec2();

    // Allocate the whole remaining area as a fixed canvas and handle pan/zoom
    // manually. Using a ScrollArea here causes the scroll wheel to both zoom
    // (our code) and pan the viewport (egui), which fight and "teleport".
    let canvas_size = ui.available_size();
    let (canvas_rect, canvas_resp) = ui.allocate_exact_size(canvas_size, Sense::click_and_drag());

    // Fit zoom = the scale at which the whole texture fits the canvas (never
    // upscaling past 1:1). This is both the initial zoom and the minimum the
    // user can zoom out to — you can't shrink the texture smaller than fit.
    let fit_zoom = if canvas_rect.width() > 1.0
        && canvas_rect.height() > 1.0
        && image_size.x > 0.0
        && image_size.y > 0.0
    {
        let fit_w = canvas_rect.width() / image_size.x;
        let fit_h = canvas_rect.height() / image_size.y;
        fit_w.min(fit_h).min(1.0).max(0.001)
    } else {
        0.001
    };

    // On first load, set zoom to fit and center.
    if !preview.zoom_initialized && fit_zoom > 0.001 {
        preview.zoom = fit_zoom;
        preview.pan = Vec2::ZERO;
        preview.zoom_initialized = true;
    }

    // Scroll-to-zoom, anchored at the cursor (the image pixel under the
    // pointer stays fixed). All math is self-contained in this frame, so
    // there's no one-frame feedback lag.
    if canvas_resp.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll.abs() > f32::EPSILON {
            let old_zoom = preview.zoom;
            let factor = (scroll / 240.0).exp();
            // Floor at fit_zoom so the texture can't be zoomed out smaller
            // than the size where it fully fits the canvas.
            let new_zoom = (old_zoom * factor).clamp(fit_zoom, 32.0);
            if (new_zoom - old_zoom).abs() > f32::EPSILON {
                if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
                    // Image top-left in screen space at the current zoom.
                    let center = canvas_rect.center();
                    let img_tl = center + preview.pan - image_size * old_zoom * 0.5;
                    // Pixel coordinate under the cursor.
                    let img_px = (ptr - img_tl) / old_zoom;
                    // Solve for the pan that keeps img_px under the cursor.
                    let new_img_tl = ptr - img_px * new_zoom;
                    preview.pan = new_img_tl - center + image_size * new_zoom * 0.5;
                }
                preview.zoom = new_zoom;
            }
        }
    }

    // Drag to pan.
    if canvas_resp.dragged() {
        preview.pan += canvas_resp.drag_delta();
    }

    // Clamp the pan so the image always covers the canvas — you can't drag
    // into empty background past the image edge. When the image is smaller
    // than the canvas on an axis (e.g. at fit zoom), it stays centered there.
    let draw_size = image_size * preview.zoom;
    let half_extra_x = ((draw_size.x - canvas_rect.width()) * 0.5).max(0.0);
    let half_extra_y = ((draw_size.y - canvas_rect.height()) * 0.5).max(0.0);
    preview.pan.x = preview.pan.x.clamp(-half_extra_x, half_extra_x);
    preview.pan.y = preview.pan.y.clamp(-half_extra_y, half_extra_y);

    // Draw: dark background, then the image clipped to the canvas.
    let painter = ui.painter();
    painter.rect_filled(canvas_rect, 0.0, Color32::from_rgb(64, 64, 64));
    painter.rect_stroke(canvas_rect, 0.0, Stroke::new(1.0, grid_line()));

    let img_tl = canvas_rect.center() + preview.pan - draw_size * 0.5;
    let img_rect = egui::Rect::from_min_size(img_tl, draw_size);
    painter.with_clip_rect(canvas_rect).image(
        texture.id(),
        img_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

pub(super) fn build_bitmap_preview(tag: &TagFile) -> anyhow::Result<BitmapPreviewData> {
    let bitmap = Bitmap::new(tag)?;
    if bitmap.is_empty() {
        anyhow::bail!("bitmap tag has no images");
    }
    let image = bitmap
        .image(0)
        .ok_or_else(|| anyhow::anyhow!("bitmap tag has no image 0"))?;
    let format = image.format()?;
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        anyhow::bail!("bitmap image has empty dimensions");
    }
    let pixel_bytes = image.pixel_bytes()?;
    let mip0_len = format.level_bytes(width, height) as usize;
    if pixel_bytes.len() < mip0_len {
        anyhow::bail!(
            "bitmap image mip 0 needs {} bytes but only {} were available",
            mip0_len,
            pixel_bytes.len()
        );
    }
    let rgba = decode_to_rgba8(format, width, height, &pixel_bytes[..mip0_len])?;
    Ok(BitmapPreviewData {
        width,
        height,
        image_count: bitmap.len(),
        format_name: image.format_name().unwrap_or_else(|| format!("{format:?}")),
        type_name: image.type_name().unwrap_or_else(|| "2D texture".to_owned()),
        rgba,
    })
}

pub(super) fn filtered_bitmap_rgba(
    data: &BitmapPreviewData,
    preview: &BitmapPreviewState,
) -> Vec<u8> {
    let alpha_only =
        !preview.show_red && !preview.show_green && !preview.show_blue && preview.show_alpha;
    let mut out = data.rgba.clone();
    for pixel in out.chunks_exact_mut(4) {
        let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
        if alpha_only {
            pixel[0] = a;
            pixel[1] = a;
            pixel[2] = a;
            pixel[3] = 255;
        } else {
            pixel[0] = if preview.show_red { r } else { 0 };
            pixel[1] = if preview.show_green { g } else { 0 };
            pixel[2] = if preview.show_blue { b } else { 0 };
            pixel[3] = if preview.show_alpha { a } else { 255 };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_variant_ops_create_update_and_drop_regions() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/model.json")).unwrap();
        let mut dirty = false;

        let status = apply_model_variant_ops(
            &mut tag,
            vec![ModelVariantOp::Create {
                name: "test".to_owned(),
                regions: vec![ModelVariantRegionChoice {
                    region_name: "body".to_owned(),
                    permutation_name: "default".to_owned(),
                }],
            }],
            &mut dirty,
        );
        assert_eq!(status.as_deref(), Some("Created model variant 'test'"));
        assert!(dirty);
        assert_variant(&tag, 0, "test", "body", "default");

        let status = apply_model_variant_ops(
            &mut tag,
            vec![ModelVariantOp::Update {
                variant_index: 0,
                regions: vec![ModelVariantRegionChoice {
                    region_name: "head".to_owned(),
                    permutation_name: "damaged".to_owned(),
                }],
            }],
            &mut dirty,
        );
        assert_eq!(status.as_deref(), Some("Updated model variant 0"));
        assert_variant(&tag, 0, "test", "head", "damaged");

        let status = apply_model_variant_ops(
            &mut tag,
            vec![ModelVariantOp::Drop { variant_index: 0 }],
            &mut dirty,
        );
        assert_eq!(status.as_deref(), Some("Deleted model variant 0"));
        let variants = tag
            .root()
            .field("variants")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(variants.len(), 0);
    }

    #[test]
    fn h2_render_model_marker_translation_and_rotation_are_editable() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/render_model.json")).unwrap();
        tag.container = test_halo2_render_model_container();
        {
            let mut root = tag.root_mut();
            let mut field = root.field_path_mut("marker groups").unwrap();
            let mut marker_groups = field.as_block_mut().unwrap();
            marker_groups.add_element();
        }
        {
            let mut root = tag.root_mut();
            let mut field = root.field_path_mut("marker groups[0]/markers").unwrap();
            let mut markers = field.as_block_mut().unwrap();
            markers.add_element();
        }

        let mut dirty = false;
        let status = apply_pending_edits(
            &mut tag,
            vec![
                PendingFieldEdit {
                    path: "marker groups[0]/markers[0]/translation".to_owned(),
                    input: "-0.27, 0, 0.73".to_owned(),
                },
                PendingFieldEdit {
                    path: "marker groups[0]/markers[0]/rotation".to_owned(),
                    input: "-0.38, 0, -0.92, 0".to_owned(),
                },
            ],
            &mut dirty,
        );

        assert_eq!(
            status.as_deref(),
            Some("Edited marker groups[0]/markers[0]/rotation")
        );
        assert!(dirty);
        let root = tag.root();
        let translation = root
            .field_path("marker groups[0]/markers[0]/translation")
            .unwrap()
            .value()
            .unwrap();
        let TagFieldData::RealPoint3d(translation) = translation else {
            panic!("translation should be a real point 3d");
        };
        assert!((translation.x + 0.27).abs() < 0.0001);
        assert!((translation.y - 0.0).abs() < 0.0001);
        assert!((translation.z - 0.73).abs() < 0.0001);

        let rotation = root
            .field_path("marker groups[0]/markers[0]/rotation")
            .unwrap()
            .value()
            .unwrap();
        let TagFieldData::RealQuaternion(rotation) = rotation else {
            panic!("rotation should be a real quaternion");
        };
        assert!((rotation.i + 0.38).abs() < 0.0001);
        assert!((rotation.j - 0.0).abs() < 0.0001);
        assert!((rotation.k + 0.92).abs() < 0.0001);
        assert!((rotation.w - 0.0).abs() < 0.0001);
        assert_h2_render_model_write_atomic_verifies(&tag);
    }

    fn test_halo2_render_model_container() -> blam_tags::file::TagContainer {
        let mut header = vec![0; 64];
        header[36..40].copy_from_slice(b"edom");
        header[56..58].copy_from_slice(&0u16.to_le_bytes());
        header[60..64].copy_from_slice(b"!MLB");
        blam_tags::file::TagContainer::Classic {
            engine: blam_tags::classic::ClassicEngine::Halo2V4,
            header,
        }
    }

    fn assert_h2_render_model_write_atomic_verifies(tag: &TagFile) {
        let mut path = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!(
            "baboon_h2_render_model_marker_{}_{}.render_model",
            std::process::id(),
            stamp
        ));
        let _ = std::fs::remove_file(&path);
        tag.write_atomic(&path).unwrap_or_else(|error| {
            panic!(
                "write_atomic verification failed for {}: {error}",
                path.display()
            )
        });
        let _ = std::fs::remove_file(&path);
    }

    fn assert_variant(
        tag: &TagFile,
        variant_index: usize,
        variant_name: &str,
        region_name: &str,
        permutation_name: &str,
    ) {
        let variants = tag
            .root()
            .field("variants")
            .and_then(|field| field.as_block())
            .unwrap();
        let variant = variants.element(variant_index).unwrap();
        assert_eq!(
            variant.read_string_id("name").as_deref(),
            Some(variant_name)
        );
        let regions = variant
            .field("regions")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(regions.len(), 1);
        let region = regions.element(0).unwrap();
        assert_eq!(
            region.read_string_id("region name").as_deref(),
            Some(region_name)
        );
        let permutations = region
            .field("permutations")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(permutations.len(), 1);
        let permutation = permutations.element(0).unwrap();
        assert_eq!(
            permutation.read_string_id("permutation name").as_deref(),
            Some(permutation_name)
        );
    }
}
