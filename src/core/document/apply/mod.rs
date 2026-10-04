//! Applying a batch of edits to a document: field values, block structure
//! (with block-index references remapped), function bytes, shader and Halo 2
//! shader parameters, and model variants. Each batch is one undo step, and a
//! batch that fails partway leaves the tag as it was.

use std::collections::HashMap;

use blam_tags::{H2Function, TagField, TagFieldData, TagFieldType, TagFile};

use crate::core::document::ops::{
    BlockOp, BlockOpKind, FunctionDataOp, H2ShaderParamOp, ModelVariantOp,
    ModelVariantRegionChoice, PendingFieldEdit, ShaderOp, ShaderParamOp,
};
use crate::core::document::value::{
    append_field_path_for, escape_field_path_segment, parse_gui_field_value,
};
use crate::core::document::{Dirty, TagDocument};

pub(crate) struct FieldEditOutcome {
    pub(crate) path: String,
    pub(crate) input: String,
    pub(crate) result: Result<(), String>,
}

pub(crate) struct AppliedFieldEdits {
    pub(crate) status: Option<String>,
    pub(crate) outcomes: Vec<FieldEditOutcome>,
}

/// Every kind of edit a tag pane collects while it draws. They are applied
/// together once the draw has finished, behind one undo snapshot.
#[derive(Default)]
pub(crate) struct DeferredOps {
    pub(crate) pending: Vec<PendingFieldEdit>,
    pub(crate) block_ops: Vec<BlockOp>,
    pub(crate) shader_ops: Vec<ShaderOp>,
    pub(crate) shader_param_ops: Vec<ShaderParamOp>,
    pub(crate) h2_shader_param_ops: Vec<H2ShaderParamOp>,
    pub(crate) model_variant_ops: Vec<ModelVariantOp>,
    /// Halo 2 function byte-block writes from the function editor.
    pub(crate) function_data_ops: Vec<FunctionDataOp>,
}

impl DeferredOps {
    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
            && self.block_ops.is_empty()
            && self.shader_ops.is_empty()
            && self.shader_param_ops.is_empty()
            && self.h2_shader_param_ops.is_empty()
            && self.model_variant_ops.is_empty()
            && self.function_data_ops.is_empty()
    }
}

/// Whether a batch of edits is its own undo step or joins the one open.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum UndoStep {
    /// A tag pane's per-frame edits: typing into a field keeps extending one
    /// step, and a frame with no edits closes it.
    Coalesce,
    /// A popup, picker or paste confirmed once: one step, closed at once.
    Own,
}

pub(crate) struct AppliedDeferredOps {
    /// The last batch's status line, if any batch set one.
    pub(crate) status: Option<String>,
    /// Per-draft outcomes of the plain field edits.
    pub(crate) outcomes: Vec<FieldEditOutcome>,
    /// A model-variant op ran, so a cached model preview is stale.
    pub(crate) model_variants_changed: bool,
}

/// Apply one frame's deferred edits to `doc`. Any edit at all opens (or
/// extends) an undo window first; a frame with none closes it.
///
/// The undo decision used to list the op kinds by hand, and missed the H2
/// shader-parameter and function-data ops: the H2 shader grid's main edits
/// changed the tag with no snapshot to undo to.
pub(crate) fn apply_deferred_ops(
    doc: &mut TagDocument,
    ops: DeferredOps,
    label: &str,
) -> AppliedDeferredOps {
    if ops.is_empty() {
        doc.journal.end_edit_window();
        return AppliedDeferredOps {
            status: None,
            outcomes: Vec::new(),
            model_variants_changed: false,
        };
    }
    doc.journal.begin_edit(&doc.tag, label);
    let DeferredOps {
        pending,
        block_ops,
        shader_ops,
        shader_param_ops,
        h2_shader_param_ops,
        model_variant_ops,
        function_data_ops,
    } = ops;
    let tag = &mut doc.tag;
    let dirty = &mut doc.dirty;
    let applied = apply_pending_edits(tag, pending, dirty);
    let mut status = applied.status;
    let mut keep = |next: Option<String>| {
        if next.is_some() {
            status = next;
        }
    };
    keep(apply_block_ops(tag, block_ops, dirty));
    keep(apply_shader_ops(tag, shader_ops, dirty));
    keep(apply_shader_param_ops(tag, shader_param_ops, dirty));
    keep(apply_h2_shader_param_ops(tag, h2_shader_param_ops, dirty));
    let variant_status = apply_model_variant_ops(tag, model_variant_ops, dirty);
    let model_variants_changed = variant_status.is_some();
    keep(variant_status);
    keep(apply_function_data_ops(tag, function_data_ops, dirty));
    AppliedDeferredOps {
        status,
        outcomes: applied.outcomes,
        model_variants_changed,
    }
}


pub(crate) fn apply_pending_edits(
    tag: &mut TagFile,
    edits: Vec<PendingFieldEdit>,
    dirty: &mut Dirty,
) -> AppliedFieldEdits {
    let mut status = None;
    let mut outcomes = Vec::with_capacity(edits.len());
    for edit in edits {
        let result = catch_edit_unwind(|| apply_field_edit(tag, &edit.path, &edit.input));
        match &result {
            Ok(()) => {
                dirty.touch();
                status = Some(format!("Edited {}", edit.path));
            }
            Err(error) => {
                status = Some(format!("Edit failed for {}: {error}", edit.path));
            }
        }
        outcomes.push(FieldEditOutcome {
            path: edit.path,
            input: edit.input,
            result,
        });
    }
    AppliedFieldEdits { status, outcomes }
}

pub(crate) fn apply_block_ops(
    tag: &mut TagFile,
    ops: Vec<BlockOp>,
    dirty: &mut Dirty,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        let result = apply_one_block_op(tag, &op);
        match result {
            Ok(msg) => {
                dirty.touch();

                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("Block edit failed for {}: {error}", op.path));
            }
        }
    }
    status
}

pub(crate) fn apply_function_data_ops(
    tag: &mut TagFile,
    ops: Vec<FunctionDataOp>,
    dirty: &mut Dirty,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        let result =
            catch_edit_unwind(|| replace_halo2_function_byte_block(tag, &op.block_path, &op.data));
        match result {
            Ok(()) => {
                dirty.touch();
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

pub(crate) fn replace_halo2_function_byte_block(
    tag: &mut TagFile,
    block_path: &str,
    data: &[u8],
) -> Result<(), String> {
    // A Halo 2 byte-block holds the H2 encoding and nothing else.
    if H2Function::parse(data).is_err() {
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

pub(crate) fn apply_h2_shader_param_ops(
    tag: &mut TagFile,
    ops: Vec<H2ShaderParamOp>,
    dirty: &mut Dirty,
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
                dirty.touch();
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("H2 shader edit failed: {error}"));
            }
        }
    }
    status
}

pub(crate) fn apply_one_h2_shader_param_op(
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
            let parameter = ensure_h2_shader_parameter(
                tag,
                parameters_block_path,
                parameter_name,
                *parameter_type_index,
            )?;
            let mut animation = None;
            let result = (|| {
                let property = ensure_h2_animation_property(
                    tag,
                    parameters_block_path,
                    parameter.index,
                    *animation_type_index,
                )?;
                animation = Some(property);
                let data_path = format!(
                    "{}[{}]/animation properties[{}]/function/data",
                    parameters_block_path, parameter.index, property.index
                );
                replace_halo2_function_byte_block(tag, &data_path, initial_function_data)
            })();
            if result.is_err() {
                // Remove whatever this op made, outermost first: deleting a
                // created parameter takes its animation properties with it.
                if parameter.created {
                    delete_block_element(tag, parameters_block_path, parameter.index);
                } else if let Some(property) = animation.filter(|property| property.created) {
                    delete_block_element(
                        tag,
                        &format!(
                            "{parameters_block_path}[{}]/animation properties",
                            parameter.index
                        ),
                        property.index,
                    );
                }
            }
            result?;
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
            let parameter = ensure_h2_shader_parameter(
                tag,
                parameters_block_path,
                parameter_name,
                *parameter_type_index,
            )?;
            let path = format!(
                "{}[{}]/{}",
                parameters_block_path,
                parameter.index,
                escape_field_path_segment(field)
            );
            // A value that does not parse must not leave behind the empty
            // parameter created to hold it.
            if let Err(error) = apply_field_edit(tag, &path, input) {
                if parameter.created {
                    delete_block_element(tag, parameters_block_path, parameter.index);
                }
                return Err(error);
            }
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
) -> Result<Ensured, String> {
    if let Some(index) = h2_shader_parameter_index(tag, parameters_block_path, parameter_name) {
        return Ok(Ensured {
            index,
            created: false,
        });
    }
    let index = with_new_element(tag, parameters_block_path, |tag, index| {
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
    })?;
    Ok(Ensured {
        index,
        created: true,
    })
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
) -> Result<Ensured, String> {
    let animation_block_path =
        format!("{parameters_block_path}[{parameter_index}]/animation properties");
    if let Some(index) =
        h2_animation_property_index(tag, &animation_block_path, animation_type_index)
    {
        return Ok(Ensured {
            index,
            created: false,
        });
    }
    let index = with_new_element(tag, &animation_block_path, |tag, index| {
        apply_field_edit(
            tag,
            &format!("{animation_block_path}[{index}]/type"),
            &animation_type_index.to_string(),
        )?;
        Ok(index)
    })?;
    Ok(Ensured {
        index,
        created: true,
    })
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

pub(crate) fn apply_one_block_op(
    tag: &mut TagFile,
    op: &BlockOp,
) -> Result<String, String> {
    let remap = block_element_remap(tag, op)?;
    let message = apply_one_block_structure_op(tag, op)?;

    // Repair index-based links only after the structural operation succeeds.
    // The walker sees references inside moved, duplicated, or pasted elements
    // at their new paths and adjusts those along with every outside reference.
    if let Some(remap) = remap {
        remap_block_index_references(tag, &op.path, &remap)?;
    }

    Ok(message)
}

fn apply_one_block_structure_op(tag: &mut TagFile, op: &BlockOp) -> Result<String, String> {
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

/// An old block index to its new position. `None` means that the old element no
/// longer exists. Keeping this as a general mapping (rather than baking insert
/// and delete arithmetic into the walker) also gives a future block-table
/// reorder operation exactly the primitive it will need.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockElementRemap {
    old_to_new: Vec<Option<usize>>,
    /// Fresh clipboard/default elements were not part of the old ordering, so
    /// their authored index values must not be interpreted through old_to_new.
    excluded_new_elements: Option<std::ops::Range<usize>>,
}

impl BlockElementRemap {
    fn inserted(old_len: usize, at: usize, count: usize) -> Self {
        Self {
            old_to_new: (0..old_len)
                .map(|old| Some(if old >= at { old + count } else { old }))
                .collect(),
            excluded_new_elements: None,
        }
    }

    fn deleted(old_len: usize, at: usize) -> Self {
        Self {
            old_to_new: (0..old_len)
                .map(|old| match old.cmp(&at) {
                    std::cmp::Ordering::Less => Some(old),
                    std::cmp::Ordering::Equal => None,
                    std::cmp::Ordering::Greater => Some(old - 1),
                })
                .collect(),
            excluded_new_elements: None,
        }
    }

    fn replaced(old_len: usize, at: usize, replacement_count: usize) -> Self {
        Self {
            old_to_new: (0..old_len)
                .map(|old| {
                    if old < at {
                        Some(old)
                    } else if old == at {
                        (replacement_count > 0).then_some(at)
                    } else {
                        Some(old + replacement_count - 1)
                    }
                })
                .collect(),
            excluded_new_elements: None,
        }
    }

    fn removed_all(old_len: usize) -> Self {
        Self {
            old_to_new: vec![None; old_len],
            excluded_new_elements: None,
        }
    }

    fn excluding_new_elements(mut self, range: std::ops::Range<usize>) -> Self {
        self.excluded_new_elements = Some(range);
        self
    }

    fn map(&self, old: i64) -> Option<Option<usize>> {
        usize::try_from(old)
            .ok()
            .and_then(|old| self.old_to_new.get(old).copied())
    }
}

/// Validate a structural operation and describe how it moves the old entries.
/// Appending does not move any old index, so it needs no remap.
fn block_element_remap(tag: &TagFile, op: &BlockOp) -> Result<Option<BlockElementRemap>, String> {
    let field = tag
        .root()
        .field_path(&op.path)
        .ok_or_else(|| "block path no longer resolves".to_owned())?;
    // Arrays are fixed-count and their only supported operation replaces an
    // element in place, so no index can move.
    let Some(block) = field.as_block() else {
        return Ok(None);
    };
    let len = block.len();

    let remap = match &op.kind {
        BlockOpKind::Add => None,
        BlockOpKind::Insert(at) => {
            if *at > len {
                return Err(format!(
                    "index {at} is out of range for block of length {len}"
                ));
            }
            Some(BlockElementRemap::inserted(len, *at, 1).excluding_new_elements(*at..*at + 1))
        }
        BlockOpKind::Duplicate(at) | BlockOpKind::Delete(at) => {
            if *at >= len {
                return Err(format!(
                    "index {at} is out of range for block of length {len}"
                ));
            }
            if matches!(op.kind, BlockOpKind::Duplicate(_)) {
                Some(BlockElementRemap::inserted(len, at + 1, 1))
            } else {
                Some(BlockElementRemap::deleted(len, *at))
            }
        }
        BlockOpKind::DeleteAll => Some(BlockElementRemap::removed_all(len)),
        BlockOpKind::Paste { at, elements } => {
            if *at > len {
                return Err(format!(
                    "index {at} is out of range for block of length {len}"
                ));
            }
            (!elements.is_empty()).then(|| {
                BlockElementRemap::inserted(len, *at, elements.len())
                    .excluding_new_elements(*at..*at + elements.len())
            })
        }
        BlockOpKind::ReplaceElement { at, elements } => {
            if *at >= len {
                return Err(format!(
                    "index {at} is out of range for block of length {len}"
                ));
            }
            Some(
                BlockElementRemap::replaced(len, *at, elements.len())
                    .excluding_new_elements(*at..*at + elements.len()),
            )
        }
        // A wholesale replacement has no defensible identity correspondence.
        // References to old entries are therefore cleared instead of silently
        // retargeted to unrelated pasted entries.
        BlockOpKind::ReplaceBlock { elements } => {
            Some(BlockElementRemap::removed_all(len).excluding_new_elements(0..elements.len()))
        }
    };
    Ok(remap)
}

#[derive(Debug)]
struct BlockIndexEdit {
    path: String,
    value: i64,
}

fn remap_block_index_references(
    tag: &mut TagFile,
    target_path: &str,
    remap: &BlockElementRemap,
) -> Result<usize, String> {
    let root = tag.root();
    let target_path = path_without_field_ordinals(target_path);
    let mut edits = Vec::new();
    collect_block_index_edits(&root, "", root, &target_path, remap, &mut edits);

    for edit in &edits {
        apply_field_edit(tag, &edit.path, &edit.value.to_string())?;
    }
    Ok(edits.len())
}

fn collect_block_index_edits(
    tag_struct: &blam_tags::TagStruct<'_>,
    struct_path: &str,
    root: blam_tags::TagStruct<'_>,
    target_path: &str,
    remap: &BlockElementRemap,
    edits: &mut Vec<BlockIndexEdit>,
) {
    // Everything here depends on the struct, not the field, so it is worked
    // out once per struct. It used to be redone for every field of the tag on
    // every structural block edit, target resolution included, and that
    // resolution descends from the root once per ancestor.
    let is_new_element = remap
        .excluded_new_elements
        .as_ref()
        .is_some_and(|range| path_is_within_target_element(struct_path, target_path, range));
    // Per target, whether it resolves to the edited block; `None` when it does
    // not resolve at all.
    let mut declared: HashMap<String, Option<bool>> = HashMap::new();
    let is_target = |resolved: Option<String>| {
        resolved.map(|path| path_without_field_ordinals(&path) == target_path)
    };

    for field in tag_struct.fields_all() {
        let declared_state = (!is_new_element)
            .then(|| field.definition().block_index_target())
            .flatten()
            .and_then(|target| {
                *declared.entry(target.name().to_owned()).or_insert_with(|| {
                    is_target(
                        declared_block_index_target(
                            tag_struct,
                            Some(root),
                            struct_path,
                            target.name(),
                        )
                        .map(|target| target.path),
                    )
                })
            });
        if declared_state == Some(true) {
            let field_path = append_field_path_for(struct_path, &field);
            if let Some(old) = field.value().as_ref().and_then(block_index_value)
                && let Some(mapped) = remap.map(old)
            {
                let new = mapped.map(|index| index as i64).unwrap_or(-1);
                if new != old {
                    edits.push(BlockIndexEdit {
                        path: field_path,
                        value: new,
                    });
                }
            }
        }

        if let Some(block) = field.as_block() {
            let field_path = append_field_path_for(struct_path, &field);
            for (index, element) in block.iter().enumerate() {
                collect_block_index_edits(
                    &element,
                    &format!("{field_path}[{index}]"),
                    root,
                    target_path,
                    remap,
                    edits,
                );
            }
        } else if let Some(array) = field.as_array() {
            let field_path = append_field_path_for(struct_path, &field);
            for (index, element) in array.iter().enumerate() {
                collect_block_index_edits(
                    &element,
                    &format!("{field_path}[{index}]"),
                    root,
                    target_path,
                    remap,
                    edits,
                );
            }
        } else if let Some(nested) = field.as_struct() {
            let field_path = append_field_path_for(struct_path, &field);
            collect_block_index_edits(&nested, &field_path, root, target_path, remap, edits);
        }
    }
}

fn path_is_within_target_element(
    struct_path: &str,
    target_path: &str,
    range: &std::ops::Range<usize>,
) -> bool {
    let normalized = path_without_field_ordinals(struct_path);
    let Some(suffix) = normalized.strip_prefix(target_path) else {
        return false;
    };
    let Some(index) = suffix
        .strip_prefix('[')
        .and_then(|suffix| suffix.split_once(']'))
        .and_then(|(index, _)| index.parse::<usize>().ok())
    else {
        return false;
    };
    range.contains(&index)
}

/// The block a declared block-index field points into: the first sibling, or
/// failing that the nearest ancestor's field, holding a block of the target
/// definition.
///
/// The one rule for this, used both by the field's element dropdown and by
/// the renumbering that follows an element insert, delete or move, so what
/// the dropdown offers is what renumbering keeps pointing at. `root` is only
/// needed to climb past the field's own struct.
pub(crate) fn declared_block_index_target(
    tag_struct: &blam_tags::TagStruct<'_>,
    root: Option<blam_tags::TagStruct<'_>>,
    struct_path: &str,
    target_definition: &str,
) -> Option<BlockIndexTarget> {
    if target_definition.is_empty() {
        return None;
    }
    find_block_target(tag_struct, root, struct_path, |field| {
        field
            .as_block()
            .is_some_and(|block| block.definition().name() == target_definition)
    })
}

fn find_block_target(
    tag_struct: &blam_tags::TagStruct<'_>,
    root: Option<blam_tags::TagStruct<'_>>,
    struct_path: &str,
    matches: impl Fn(&TagField<'_>) -> bool + Copy,
) -> Option<BlockIndexTarget> {
    find_sibling_block(tag_struct, struct_path, matches)
        .or_else(|| find_ancestor_block(root?, struct_path, matches))
}

fn find_sibling_block(
    tag_struct: &blam_tags::TagStruct<'_>,
    struct_path: &str,
    matches: impl Fn(&TagField<'_>) -> bool,
) -> Option<BlockIndexTarget> {
    tag_struct
        .fields_all()
        .find(|field| matches(field))
        .map(|field| BlockIndexTarget {
            path: append_field_path_for(struct_path, &field),
            len: field.as_block().map_or(0, |block| block.len()),
        })
}

fn find_ancestor_block(
    root: blam_tags::TagStruct<'_>,
    struct_path: &str,
    matches: impl Fn(&TagField<'_>) -> bool + Copy,
) -> Option<BlockIndexTarget> {
    let mut current = struct_path;
    while !current.is_empty() {
        let parent = current.rsplit_once('/').map(|(path, _)| path).unwrap_or("");
        let ancestor = if parent.is_empty() {
            root
        } else {
            root.descend(parent)?
        };
        if let Some(found) = find_sibling_block(&ancestor, parent, matches) {
            return Some(found);
        }
        if parent.is_empty() {
            break;
        }
        current = parent;
    }
    None
}

/// Normalize exact field paths for comparison while retaining concrete block
/// element subscripts. Rendered paths carry `#ordinal`; hand-authored block ops
/// often do not, but both resolve to the same field.
fn path_without_field_ordinals(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut ordinal = false;
    for ch in path.chars() {
        match ch {
            '#' => ordinal = true,
            '/' | '[' if ordinal => {
                ordinal = false;
                out.push(ch);
            }
            _ if ordinal => {}
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod block_index_remap_tests;

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

pub(crate) fn apply_field_edit(
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

pub(crate) fn apply_shader_ops(
    tag: &mut TagFile,
    ops: Vec<ShaderOp>,
    dirty: &mut Dirty,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        match apply_one_shader_op(tag, &op) {
            Ok(msg) => {
                dirty.touch();
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("Shader op failed: {error}"));
            }
        }
    }
    status
}

pub(crate) fn apply_shader_param_ops(
    tag: &mut TagFile,
    ops: Vec<ShaderParamOp>,
    dirty: &mut Dirty,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        match apply_one_shader_param_op(tag, &op) {
            Ok(msg) => {
                dirty.touch();
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("Shader param op failed: {error}"));
            }
        }
    }
    status
}

pub(crate) fn apply_model_variant_ops(
    tag: &mut TagFile,
    ops: Vec<ModelVariantOp>,
    dirty: &mut Dirty,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        match apply_one_model_variant_op(tag, &op) {
            Ok(msg) => {
                dirty.touch();
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
            with_new_element(tag, "variants", |tag, index| {
                apply_field_edit(tag, &format!("variants[{index}]/name"), name)?;
                write_model_variant_regions(tag, index, regions)?;
                Ok(format!("Created model variant '{name}'"))
            })
        }
        ModelVariantOp::Update {
            variant_index,
            regions,
        } => {
            ensure_block_element_exists(tag, "variants", *variant_index)?;
            write_model_variant_regions(tag, *variant_index, regions)?;
            Ok(format!("Updated model variant {}", variant_index))
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

pub(crate) fn add_block_element(tag: &mut TagFile, path: &str) -> Result<usize, String> {
    let mut root = tag.root_mut();
    let mut field = root
        .field_path_mut(path)
        .ok_or_else(|| format!("{path} block not found"))?;
    let mut block = field
        .as_block_mut()
        .ok_or_else(|| format!("{path} is not a block"))?;
    Ok(block.add_element())
}

/// Add an element to the block at `path` and fill it with `fill`. If filling
/// fails, the element is deleted again, so a failed op leaves the tag as it
/// found it.
///
/// Ops that add an element and then write its fields used to stop at the
/// first failed write and leave the half-built element behind. Their callers
/// only mark the document dirty on success, so that element was an unsaved
/// change nothing knew about.
fn with_new_element<T>(
    tag: &mut TagFile,
    path: &str,
    fill: impl FnOnce(&mut TagFile, usize) -> Result<T, String>,
) -> Result<T, String> {
    let index = add_block_element(tag, path)?;
    fill(tag, index).inspect_err(|_| delete_block_element(tag, path, index))
}

/// Undo an element added by this module. Best effort: it runs on a path that
/// is already reporting an error.
fn delete_block_element(tag: &mut TagFile, path: &str, index: usize) {
    let mut root = tag.root_mut();
    if let Some(mut field) = root.field_path_mut(path)
        && let Some(mut block) = field.as_block_mut()
    {
        let _ = block.delete_element(index);
    }
}

/// An element found by name, or created because it was missing. A caller that
/// fails later removes it only if it made it.
#[derive(Clone, Copy)]
struct Ensured {
    index: usize,
    created: bool,
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

pub(crate) fn apply_one_shader_param_op(
    tag: &mut TagFile,
    op: &ShaderParamOp,
) -> Result<String, String> {
    let block_path = &op.parameters_block_path;
    with_new_element(tag, block_path, |tag, new_idx| {
        let name_path = format!("{block_path}[{new_idx}]/parameter name");
        apply_field_edit(tag, &name_path, &op.parameter_name)?;
        for initial in &op.initial_fields {
            let field = escape_field_path_segment(&initial.field);
            apply_field_edit(
                tag,
                &format!("{block_path}[{new_idx}]/{field}"),
                &initial.input,
            )?;
        }
        for animated in &op.animated_parameters {
            apply_one_shader_op(
                tag,
                &ShaderOp {
                    animated_block_path: format!("{block_path}[{new_idx}]/animated parameters"),
                    output_type_index: animated.output_type_index,
                    initial_function_hex: animated.initial_function_hex.clone(),
                },
            )?;
        }
        Ok(format!(
            "Created parameter '{}' at {block_path}[{new_idx}]",
            op.parameter_name
        ))
    })
}

pub(crate) fn apply_one_shader_op(
    tag: &mut TagFile,
    op: &ShaderOp,
) -> Result<String, String> {
    let block_path = &op.animated_block_path;
    with_new_element(tag, block_path, |tag, new_idx| {
        let type_path = format!("{block_path}[{new_idx}]/type");
        apply_field_edit(tag, &type_path, &op.output_type_index.to_string())?;
        // The initial `mapping_function` blob goes into `function/data`.
        let data_path = format!("{block_path}[{new_idx}]/function/data");
        apply_field_edit(tag, &data_path, &op.initial_function_hex)?;
        Ok(format!(
            "Added animated parameter (type {}) at {block_path}[{new_idx}]",
            op.output_type_index
        ))
    })
}

/// The signed index held by any block-index value variant.
pub(crate) fn block_index_value(value: &TagFieldData) -> Option<i64> {
    match value {
        TagFieldData::CharBlockIndex(v) | TagFieldData::CustomCharBlockIndex(v) => Some(*v as i64),
        TagFieldData::ShortBlockIndex(v) | TagFieldData::CustomShortBlockIndex(v) => {
            Some(*v as i64)
        }
        TagFieldData::LongBlockIndex(v) | TagFieldData::CustomLongBlockIndex(v) => Some(*v as i64),
        _ => None,
    }
}

/// Resolve a block-index field's target block, returning `(element labels, full
/// target block path)`. Checks the field's own struct first (sibling target),
/// then walks up the ancestry from `root` (ancestor target — e.g. weapon's
/// "primary barrel" → the root "barrels" block). `None` for non-(plain)
/// block-index fields, custom indices (no target in the definition), or targets
/// that don't resolve — callers fall back to the numeric editor.
/// The block a block-index field points into. Only its path and length are
/// resolved per frame; the element labels, one per target element, are built
/// by the row when it needs them.
pub(crate) struct BlockIndexTarget {
    pub(crate) path: String,
    pub(crate) len: usize,
}


#[cfg(test)]
mod rollback_tests;

#[cfg(test)]
mod deferred_ops_tests;

