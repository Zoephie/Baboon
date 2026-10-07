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
    let mut ops = ops;
    // An edit that sets a field to the value it already holds changes
    // nothing: taken as an edit, it marked the tag modified, took an undo step
    // and cleared the redo history. It still reports as applied, so the box
    // it came from settles.
    let unchanged: Vec<FieldEditOutcome> = ops
        .pending
        .extract_if(.., |edit| edit_changes_nothing(&doc.tag, edit))
        .map(|edit| FieldEditOutcome {
            path: edit.path,
            input: edit.input,
            result: Ok(()),
        })
        .collect();
    if ops.is_empty() && !unchanged.is_empty() {
        return AppliedDeferredOps {
            status: None,
            outcomes: unchanged,
            model_variants_changed: false,
        };
    }
    if ops.is_empty() {
        doc.journal.end_edit_window();
        return AppliedDeferredOps {
            status: None,
            outcomes: Vec::new(),
            model_variants_changed: false,
        };
    }
    doc.journal.begin_edit(&doc.tag, label);
    if !(ops.block_ops.is_empty()
        && ops.shader_ops.is_empty()
        && ops.shader_param_ops.is_empty()
        && ops.h2_shader_param_ops.is_empty()
        && ops.model_variant_ops.is_empty())
    {
        doc.note_layout_change();
    }
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
    let mut outcomes = applied.outcomes;
    outcomes.extend(unchanged);
    AppliedDeferredOps {
        status,
        outcomes,
        model_variants_changed,
    }
}

/// Whether `edit` would set its field to the value the field already holds.
/// Compared through the values' `Debug` forms, which are exact: `TagFieldData`
/// has no equality of its own.
fn edit_changes_nothing(tag: &TagFile, edit: &PendingFieldEdit) -> bool {
    let root = tag.root();
    let Some(field) = root.field_path(&edit.path) else {
        return false;
    };
    let (Some(current), Ok(parsed)) = (field.value(), parse_gui_field_value(&field, &edit.input)) else {
        return false;
    };
    format!("{current:?}") == format!("{parsed:?}")
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
            BlockOpKind::Reorder { order } => {
                // Swap entries and raw regions together, retaining all child data.
                let mut at_position: Vec<usize> = (0..order.len()).collect();
                let mut position_of = at_position.clone();
                for (new, &old) in order.iter().enumerate() {
                    let from = position_of[old];
                    block.swap_elements(new, from).map_err(|e| format!("{e:?}"))?;
                    let displaced = at_position[new];
                    at_position.swap(new, from);
                    position_of[old] = new;
                    position_of[displaced] = from;
                }
                Ok(format!("Reorganized {}", op.path))
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
/// longer exists. The same mapping repairs references after insertions,
/// deletions, and block-table reordering.
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
        BlockOpKind::Reorder { order } => {
            if order.len() != len {
                return Err("Reorder must include every block entry exactly once".to_owned());
            }
            let mut old_to_new = vec![None; len];
            for (new, &old) in order.iter().enumerate() {
                if old >= len || old_to_new[old].replace(new).is_some() {
                    return Err("Reorder contains a duplicate or invalid entry index".to_owned());
                }
            }
            Some(BlockElementRemap {
                old_to_new,
                excluded_new_elements: None,
            })
        }
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
mod tests {
    use super::*;
    use blam_tags::TagFile;
    use blam_tags::{Endian, TagStruct};
    use crate::core::bundled::locate_definitions_root;
    use crate::core::document::TagDocument;
    use crate::core::document::apply::apply_field_edit;
    use crate::core::document::ops::{BlockOp, BlockOpKind};
    use crate::core::document::ops::{FunctionDataOp, H2ShaderParamOp};
    use crate::core::document::ops::{ShaderParamInitialField, ShaderParamOp};
    use crate::core::document::value::{append_field_path, is_editable_tag, is_saveable_tag};
    use crate::core::format::TagNameIndex;
    use crate::core::game::GameId;
    use crate::core::test_kits::test_definition_path;

    fn test_tag() -> TagFile {
        TagFile::new(crate::core::test_kits::definitions().join(
            "haloreach_mcc/test_tag.json",
        ))
        .expect("load test-tag definition")
    }

    fn add_basic_elements(tag: &mut TagFile, count: usize) {
        for _ in 0..count {
            apply_one_block_op(
                tag,
                &BlockOp {
                    path: "basic block".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .expect("add basic block element");
        }
    }

    fn set_test_indices(tag: &mut TagFile, char_index: i64, short_index: i64, long_index: i64) {
        apply_field_edit(tag, "char block index", &char_index.to_string()).unwrap();
        apply_field_edit(tag, "short block index", &short_index.to_string()).unwrap();
        apply_field_edit(tag, "long block index", &long_index.to_string()).unwrap();
    }

    fn test_indices(tag: &TagFile) -> [i128; 3] {
        let root = tag.root();
        [
            root.read_int_any("char block index").unwrap(),
            root.read_int_any("short block index").unwrap(),
            root.read_int_any("long block index").unwrap(),
        ]
    }

    #[test]
    fn insert_and_delete_preserve_declared_block_index_targets() {
        let mut tag = test_tag();
        add_basic_elements(&mut tag, 3);
        set_test_indices(&mut tag, 0, 1, 2);

        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "basic block".to_owned(),
                kind: BlockOpKind::Insert(1),
            },
        )
        .unwrap();
        assert_eq!(test_indices(&tag), [0, 2, 3]);

        // Removing the newly inserted element restores every old position.
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "basic block".to_owned(),
                kind: BlockOpKind::Delete(1),
            },
        )
        .unwrap();
        assert_eq!(test_indices(&tag), [0, 1, 2]);

        // A reference to the removed entry becomes <none>; later references
        // move down while earlier references remain unchanged.
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "basic block".to_owned(),
                kind: BlockOpKind::Delete(1),
            },
        )
        .unwrap();
        assert_eq!(test_indices(&tag), [0, -1, 1]);
    }

    #[test]
    fn duplicate_shifts_only_entries_after_the_copy_source() {
        let mut tag = test_tag();
        add_basic_elements(&mut tag, 3);
        set_test_indices(&mut tag, 0, 1, 2);

        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "basic block".to_owned(),
                kind: BlockOpKind::Duplicate(1),
            },
        )
        .unwrap();
        assert_eq!(test_indices(&tag), [0, 1, 3]);
    }

    #[test]
    fn reordering_repairs_external_indices_of_every_integer_width() {
        let mut tag = test_tag();
        add_basic_elements(&mut tag, 3);
        set_test_indices(&mut tag, 0, 1, 2);

        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "basic block".to_owned(),
                kind: BlockOpKind::Reorder { order: vec![1, 2, 0] },
            },
        ).unwrap();
        assert_eq!(test_indices(&tag), [2, 0, 1]);
    }

    #[test]
    fn nested_declared_reference_resolves_its_ancestor_target() {
        let mut tag =
            TagFile::new(crate::core::test_kits::definitions().join("halo2_mcc/model.json")).unwrap();
        add_elements_at(&mut tag, "variants", 3);
        add_elements_at(&mut tag, "variants[0]/regions", 1);
        apply_field_edit(&mut tag, "variants[0]/regions[0]/parent variant", "2").unwrap();

        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "variants".to_owned(),
                kind: BlockOpKind::Insert(1),
            },
        )
        .unwrap();
        assert_eq!(
            tag.root()
                .descend("variants[0]/regions[0]")
                .and_then(|region| region.read_int_any("parent variant")),
            Some(3)
        );
    }

    #[test]
    fn classic_parent_node_reference_is_remapped() {
        let mut tag =
            TagFile::new(crate::core::test_kits::definitions().join("haloce_mcc/model.json")).unwrap();
        add_elements_at(&mut tag, "nodes", 3);
        apply_field_edit(&mut tag, "nodes[0]/parent node index", "2").unwrap();

        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "nodes".to_owned(),
                kind: BlockOpKind::Insert(1),
            },
        )
        .unwrap();
        assert_eq!(
            tag.root()
                .descend("nodes[0]")
                .and_then(|node| node.read_int_any("parent node index")),
            Some(3)
        );
    }

    fn add_elements_at(tag: &mut TagFile, path: &str, count: usize) {
        for _ in 0..count {
            apply_one_block_op(
                tag,
                &BlockOp {
                    path: path.to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
        }
    }

    const PARAMETERS: &str = "render_method/parameters";

    fn parameter_count(tag: &TagFile) -> usize {
        tag.root()
            .field_path(PARAMETERS)
            .and_then(|field| field.as_block())
            .map(|block| block.len())
            .expect("a Halo 3 shader has a parameters block")
    }

    /// A value typed into a new scalar row that does not parse must fail the
    /// whole op. It used to fail only its last step, leaving the parameter it
    /// had just added — named, empty, and unknown to the dirty flag.
    #[test]
    fn a_failed_shader_parameter_op_leaves_no_parameter_behind() {
        let schema = locate_definitions_root().join("halo3_mcc/shader.json");
        let mut tag = TagFile::new(schema).unwrap();
        let before = parameter_count(&tag);

        let result = apply_one_shader_param_op(
            &mut tag,
            &ShaderParamOp {
                parameters_block_path: PARAMETERS.to_owned(),
                parameter_name: "specular_coefficient".to_owned(),
                initial_fields: vec![ShaderParamInitialField {
                    field: "real".to_owned(),
                    input: "not a number".to_owned(),
                }],
                animated_parameters: Vec::new(),
            },
        );

        assert!(result.is_err());
        assert_eq!(parameter_count(&tag), before);

        // And the same op with a value that parses still adds one.
        apply_one_shader_param_op(
            &mut tag,
            &ShaderParamOp {
                parameters_block_path: PARAMETERS.to_owned(),
                parameter_name: "specular_coefficient".to_owned(),
                initial_fields: vec![ShaderParamInitialField {
                    field: "real".to_owned(),
                    input: "0.5".to_owned(),
                }],
                animated_parameters: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(parameter_count(&tag), before + 1);
    }

    fn document() -> TagDocument {
        let schema = locate_definitions_root().join("halo3_mcc/render_model.json");
        TagDocument::clean(TagFile::new(schema).unwrap())
    }

    /// Every kind of deferred op must open an undo window, not just the ones
    /// someone remembered to list. The H2 shader grid's value edits and the
    /// function editor's byte-block writes go through the two kinds the old
    /// hand-written condition left out, so they changed the tag with nothing
    /// to undo to. Whether the op then applies is beside the point here: the
    /// snapshot is taken before it runs.
    #[test]
    fn every_deferred_op_kind_opens_an_undo_window() {
        let mut h2_param = document();
        apply_deferred_ops(
            &mut h2_param,
            DeferredOps {
                h2_shader_param_ops: vec![H2ShaderParamOp::EditFunctionData {
                    block_path: "missing".to_owned(),
                    data: Vec::new(),
                }],
                ..DeferredOps::default()
            },
            "Edit",
        );
        assert!(h2_param.journal.can_undo(), "H2 shader parameter op");

        let mut function_data = document();
        let applied = apply_deferred_ops(
            &mut function_data,
            DeferredOps {
                function_data_ops: vec![FunctionDataOp {
                    block_path: "missing".to_owned(),
                    data: Vec::new(),
                }],
                ..DeferredOps::default()
            },
            "Edit",
        );
        assert!(function_data.journal.can_undo(), "function data op");
        assert!(
            applied
                .status
                .is_some_and(|status| status.starts_with("Function edit failed for missing")),
            "the function data op never ran"
        );

        let mut untouched = document();
        apply_deferred_ops(&mut untouched, DeferredOps::default(), "Edit");
        assert!(!untouched.journal.can_undo(), "a frame with no ops");
    }

    // Foundation unit tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    #[test]
    fn block_index_value_reads_all_variants() {
        use blam_tags::TagFieldData::*;
        assert_eq!(block_index_value(&CharBlockIndex(-1)), Some(-1));
        assert_eq!(block_index_value(&ShortBlockIndex(5)), Some(5));
        assert_eq!(block_index_value(&LongBlockIndex(42)), Some(42));
        assert_eq!(block_index_value(&CustomShortBlockIndex(3)), Some(3));
        // Non-block-index values don't read as a block index.
        assert_eq!(block_index_value(&LongInteger(7)), None);
    }

    // Editor unit and fixture tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    /// Regression (skip-if-absent): clearing a classic Halo CE tag_reference to
    /// NONE — or saving a freshly-created reference — must reset the inline
    /// group + path-length words. When the reference's sub-chunk payload is
    /// emptied (`TagReferenceData::to_bytes(None)` yields no bytes), the classic
    /// encoder used to leave the stale on-disk path length in place while
    /// writing no trailing path, so re-decoding hit "unexpected EOF reading
    /// tag_reference path: need N bytes, have M" and `write_atomic` failed
    /// verification — corrupting Save As / new-tag saves. This drives the exact
    /// Baboon load→edit→save path (read_tag_at_path → apply_field_edit →
    /// write_atomic) that the field reported.
    #[test]
    #[ignore]
    fn ce_shader_model_clear_reference_saves() {
        let defs = crate::core::test_kits::definitions();
        let tag_path = std::path::Path::new(crate::core::test_kits::tag_path(
            "haloce_mcc",
            "characters/crewman/shaders/crewman_body.shader_model",
        ));
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no CE tag/defs");
            return;
        }
        let group = u32::from_be_bytes(*b"soso");
        let out = std::env::temp_dir().join("baboon_ce_clear_ref.shader_model");
        let load = || {
            crate::core::source::read_tag_at_path(tag_path, Some(GameId::HaloCe), Some(defs), group)
                .expect("read CE shader_model tag")
        };

        // Save As of the unmodified tag round-trips.
        load()
            .write_atomic(&out)
            .expect("Save As of unmodified tag");

        // Setting a reference to a new path round-trips (the pre-existing path).
        let mut edited = load();

        apply_field_edit(
            &mut edited,
            "base map",
            "weapons\\smg\\bitmaps\\smg.bitmap",
        )
        .expect("set base map");
        edited.write_atomic(&out).expect("save after set");

        // Clearing a long-path reference to NONE round-trips (the regression).
        let mut cleared = load();
        apply_field_edit(&mut cleared, "base map", "none").expect("clear base map");
        cleared
            .write_atomic(&out)
            .expect("save after clear-to-none must verify");

        // The cleared reference reads back as a null reference — an empty path,
        // the same shape a genuine stock null (e.g. the detail map) decodes to,
        // which Baboon renders as NONE.
        let reread = crate::core::source::read_tag_at_path(&out, Some(GameId::HaloCe), Some(defs), group)
            .expect("reread cleared tag");
        let root = reread.root();
        let base = root
            .field_path("base map")
            .and_then(|f| f.value())
            .expect("base map field present");
        match base {
            TagFieldData::TagReference(r) => {
                let path = r.group_tag_and_name.as_ref().map(|(_, p)| p.as_str());
                assert!(
                    path.is_none_or(str::is_empty),
                    "cleared ref should have no path, got {path:?}"
                );
            }
            other => panic!("expected tag reference, got {other:?}"),
        }
    }

    #[test]
    fn euler_angle_edit_round_trips_through_tag_serialization() {
        let _units = crate::core::format::AngleUnitGuard::set(true);
        let mut tag = TagFile::new(test_definition_path("haloreach_mcc/test_tag.json")).unwrap();
        let mut dirty = Dirty::default();

        let status = apply_pending_edits(
            &mut tag,
            vec![PendingFieldEdit {
                path: "real euler angles 3d".to_owned(),
                input: "-0.65, 0, 1.25".to_owned(),
            }],
            &mut dirty,
        );

        assert_eq!(
            status.status.as_deref(),
            Some("Edited real euler angles 3d")
        );
        assert_eq!(status.outcomes.len(), 1);
        assert_eq!(status.outcomes[0].path, "real euler angles 3d");
        assert_eq!(status.outcomes[0].input, "-0.65, 0, 1.25");
        assert!(status.outcomes[0].result.is_ok());
        assert!(dirty.is_set());
        let bytes = tag.write_to_bytes().unwrap();
        let reopened = TagFile::read_from_bytes(&bytes).unwrap();
        let value = reopened
            .root()
            .field("real euler angles 3d")
            .unwrap()
            .value()
            .unwrap();
        let TagFieldData::RealEulerAngles3d(value) = value else {
            panic!("expected real euler angles 3d");
        };
        // The typed degrees survive as the radians they mean.
        assert!((value.yaw + 0.65f32.to_radians()).abs() < 0.0001);
        assert!(value.pitch.abs() < 0.0001);
        assert!((value.roll - 1.25f32.to_radians()).abs() < 0.0001);
    }

    #[test]
    fn model_variant_ops_create_update_and_drop_regions() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/model.json")).unwrap();
        let mut dirty = Dirty::default();

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
        assert!(dirty.is_set());
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

        let mut dirty = Dirty::default();
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
            status.status.as_deref(),
            Some("Edited marker groups[0]/markers[0]/rotation")
        );
        assert!(status.outcomes.iter().all(|outcome| outcome.result.is_ok()));
        assert!(dirty.is_set());
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

    /// A little-endian tag holds an edit little-endian — the anchor for the
    /// big-endian claim below, and the reason that claim needs real data:
    /// element bytes are stored in the source wire order, so an edit's byte
    /// order is a property of the tag it was read from, not of the editor.
    #[test]
    fn an_edit_is_stored_in_the_tags_own_byte_order() {
        let mut tag = TagFile::new(test_definition_path("halo4_mcc/camera_track.json")).unwrap();
        add_block_element(&mut tag, "control points").unwrap();
        apply_field_edit(&mut tag, "control points[0]/position", "1.5, 2.5, 3.5").unwrap();

        let bytes = tag.write_to_bytes().unwrap();
        let mut wanted = Vec::new();
        for value in [1.5f32, 2.5, 3.5] {
            wanted.extend_from_slice(&value.to_le_bytes());
        }
        assert!(
            bytes.windows(wanted.len()).any(|window| window == wanted),
            "a little-endian tag must hold the edit little-endian"
        );
    }

    /// Skip-if-absent, and the only place the big-endian edit path can actually
    /// be exercised: a tag's element bytes carry the byte order they were read
    /// in, and nothing here can manufacture those — flipping `TagFile::endian`
    /// on a tag built from a schema changes the file's wire marker and not one
    /// byte of its block data, so a synthetic "big-endian" tag would agree with
    /// a broken encoder.
    ///
    /// Sweeps the whole build (or `BABOON_MONOLITHIC_TAGS` tags of it) and
    /// reports a ledger: every tag counted before anything is filtered, and
    /// every skip named, so a run that reached almost nothing cannot read as a
    /// clean pass.
    ///
    /// Run against a real Halo 4 development build:
    ///   BABOON_MONOLITHIC="/path/to/tag_cache/blob_index.dat" \
    ///     cargo test a_monolithic_tag_edit -- --ignored --nocapture
    #[test]
    #[ignore = "requires a monolithic tag build; set BABOON_MONOLITHIC"]
    fn a_monolithic_tag_edit_is_stored_big_endian() {
        let Ok(blob_index) = std::env::var("BABOON_MONOLITHIC") else {
            eprintln!("skip: BABOON_MONOLITHIC not set");
            return;
        };
        let blob_index = std::path::PathBuf::from(blob_index);
        if !blob_index.exists() {
            eprintln!("skip: no monolithic build at {}", blob_index.display());
            return;
        }
        let limit = std::env::var("BABOON_MONOLITHIC_TAGS")
            .ok()
            .and_then(|count| count.parse::<usize>().ok())
            .unwrap_or(usize::MAX);
        let names = TagNameIndex::load_from_definitions(&locate_definitions_root());
        let loaded = crate::core::source::load_monolithic_blob_index(blob_index, &names)
            .expect("open the monolithic build");

        let total = loaded.entries.len().min(limit);
        let (mut unreadable, mut no_real_field, mut edited) = (0usize, 0usize, 0usize);
        let mut read_panics = Vec::new();
        let mut failures = Vec::new();
        // Reading a tag can panic inside the engine's geometry decoder on this
        // build. That is not what this test is about, so it is caught, counted,
        // and named rather than allowed to end the sweep at the first one.
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        // Distinctive enough that finding its 4 bytes in a tag is not a
        // coincidence, and asymmetric under byte-swap.
        let value = 1234.5677f32;
        let big = value.to_be_bytes();
        let little = value.to_le_bytes();

        for entry in loaded.entries.iter().take(limit) {
            let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::core::source::read_entry(&loaded.source, entry)
            }));
            let mut tag = match read {
                Ok(Ok(tag)) => tag,
                // Counted and named, not silently passed over: a build Baboon
                // cannot parse is a different result from one it edits cleanly.
                Ok(Err(_)) => {
                    unreadable += 1;
                    continue;
                }
                Err(_) => {
                    read_panics.push(entry.display_path.clone());
                    continue;
                }
            };
            assert_eq!(tag.endian, Endian::Be, "a monolithic build is big-endian");
            assert!(
                is_editable_tag(entry, &tag),
                "every tag in the build must be editable"
            );
            assert!(
                !is_saveable_tag(entry, &tag),
                "and none of them saveable: {}",
                entry.display_path
            );
            let Some(path) = first_real_field_path(tag.root(), 0, "") else {
                no_real_field += 1;
                continue;
            };

            if let Err(error) = apply_field_edit(&mut tag, &path, &value.to_string()) {
                failures.push(format!(
                    "{} / {path}: edit failed: {error}",
                    entry.display_path
                ));
                continue;
            }
            let read_back = tag.root().field_path(&path).and_then(|field| field.value());
            if !matches!(read_back, Some(TagFieldData::Real(back)) if (back - value).abs() < 0.001)
            {
                failures.push(format!(
                    "{} / {path}: read back as {read_back:?}, not the value typed",
                    entry.display_path
                ));
                continue;
            }

            let bytes = tag.write_to_bytes().unwrap();
            if !bytes.windows(4).any(|window| window == big) {
                failures.push(format!(
                    "{} / {path}: the edit did not land big-endian",
                    entry.display_path
                ));
                continue;
            }
            if bytes.windows(4).any(|window| window == little) {
                failures.push(format!(
                    "{} / {path}: the value also appears little-endian",
                    entry.display_path
                ));
                continue;
            }
            edited += 1;
        }

        std::panic::set_hook(previous_hook);
        eprintln!(
            "{total} tag(s) in the build: {edited} edited big-endian, {no_real_field} with no \
             real field to type into, {unreadable} unreadable, {} panicked on read, {} failed",
            read_panics.len(),
            failures.len()
        );
        // Caught so one bad tag cannot end the sweep, but still a failure: the
        // whole build read clean once `half_to_f32` stopped overflowing on
        // subnormal halves, and a tag that panics on read is a tag whose tab
        // sits on "Loading…" forever.
        assert!(
            read_panics.is_empty(),
            "{} tag(s) panicked while being read: {}",
            read_panics.len(),
            read_panics
                .iter()
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(
            failures.is_empty(),
            "{} failure(s):\n{}",
            failures.len(),
            failures
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(edited > 0, "no tag in the build had a real field to edit");
    }

    /// Path of the first plain `real` field in `tag_struct`, searching nested
    /// structs and the first element of each block.
    fn first_real_field_path(
        tag_struct: TagStruct<'_>,
        depth: usize,
        prefix: &str,
    ) -> Option<String> {
        if depth > 3 {
            return None;
        }
        for field in tag_struct.fields() {
            let path = append_field_path(prefix, field.name());
            if matches!(field.value(), Some(TagFieldData::Real(_))) {
                return Some(path);
            }
            if let Some(nested) = field.as_struct() {
                if let Some(found) = first_real_field_path(nested, depth + 1, &path) {
                    return Some(found);
                }
            } else if let Some(block) = field.as_block() {
                if let Some(element) = block.element(0) {
                    let element_path = format!("{path}[0]");
                    if let Some(found) = first_real_field_path(element, depth + 1, &element_path) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    /// The root element of a freshly created tag gets every block index at 0,
    /// while an element added to a block gets NONE (-1). The engine's
    /// invariant is NONE everywhere; the root is a known gap in
    /// `TagBlockData::new_root_default`. This pins today's behaviour so that
    /// closing the gap is a deliberate change, and shows the contrast.
    #[test]
    fn a_fresh_root_has_block_indices_at_zero_unlike_a_new_element() {
        let block_index = |tag: &TagFile, path: &str| match tag.root().field_path(path)?.value()? {
            TagFieldData::CharBlockIndex(v) | TagFieldData::CustomCharBlockIndex(v) => {
                Some(v as i64)
            }
            TagFieldData::ShortBlockIndex(v) | TagFieldData::CustomShortBlockIndex(v) => {
                Some(v as i64)
            }
            TagFieldData::LongBlockIndex(v) | TagFieldData::CustomLongBlockIndex(v) => {
                Some(v as i64)
            }
            _ => None,
        };
        let effect = TagFile::new(test_definition_path("haloreach_mcc/effect.json")).unwrap();
        // Known gap: the engine invariant says this should be NONE (-1).
        assert_eq!(block_index(&effect, "loop start event"), Some(0));

        let mut physics =
            TagFile::new(test_definition_path("haloreach_mcc/physics_model.json")).unwrap();
        physics
            .root_mut()
            .field_path_mut("materials")
            .unwrap()
            .as_block_mut()
            .unwrap()
            .add_element();
        assert_eq!(block_index(&physics, "materials[0]/phantom type"), Some(-1));
    }
}
