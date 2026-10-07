//! The edits the UI collects while it draws and applies afterwards, once
//! nothing borrows the tag: field values, function bytes, block structure,
//! shader parameters and model variants.

#[derive(Clone)]
pub(crate) struct PendingFieldEdit {
    pub(crate) path: String,
    pub(crate) input: String,
}

#[derive(Clone)]
/// Replacement bytes for one function's backing storage.
/// The whole blob is retained so edits can preserve unrecognized layout bytes.
pub(crate) struct FunctionDataOp {
    pub(crate) block_path: String,
    pub(crate) data: Vec<u8>,
}

#[derive(Clone)]
/// Atomic structural mutations needed by classic Halo 2 shader parameters.
/// These operations defer block creation and byte writes until immutable render
/// borrows have ended; unknown bytes in existing function blobs must survive.
pub(crate) enum H2ShaderParamOp {
    EnsureAnimationProperty {
        parameters_block_path: String,
        parameter_name: String,
        parameter_type_index: i32,
        animation_type_index: i32,
        initial_function_data: Vec<u8>,
    },
    EditFunctionData {
        block_path: String,
        data: Vec<u8>,
    },
    EditTemplateBackedValue {
        parameters_block_path: String,
        parameter_name: String,
        parameter_type_index: i32,
        field: String,
        input: String,
    },
}

/// A deferred structural edit to a block (add/insert/duplicate/delete),
/// applied to the tag after the immutable render borrow ends.
#[derive(Clone)]
pub(crate) enum BlockOpKind {
    Add,
    Insert(usize),
    Duplicate(usize),
    Delete(usize),
    DeleteAll,
    /// Final ordering: each new position identifies its original element.
    Reorder { order: Vec<usize> },
    /// Insert copied element(s) at the given index.
    Paste {
        at: usize,
        elements: Vec<blam_tags::TagBlockElement>,
    },
    /// Replace the element at `at` with the copied element(s).
    ReplaceElement {
        at: usize,
        elements: Vec<blam_tags::TagBlockElement>,
    },
    /// Clear the block and fill it with the copied element(s).
    ReplaceBlock {
        elements: Vec<blam_tags::TagBlockElement>,
    },
}

#[derive(Clone)]
pub(crate) struct BlockOp {
    pub(crate) path: String,
    pub(crate) kind: BlockOpKind,
}

/// A deferred shader mutation: append one `animated parameters[]` element to
/// the given block path, then initialise its `type` and `function/data`
/// fields. Applied after the frame's draw pass, like `BlockOp`, but in its
/// own pass so the add + field init can be done atomically.
#[derive(Clone)]
pub(crate) struct ShaderOp {
    /// Absolute path to the `animated parameters` block, e.g.
    /// `render_method/parameters[2]/animated parameters`.
    pub(crate) animated_block_path: String,
    /// Output channel index (`RenderMethodAnimatedParameterType as i32`).
    pub(crate) output_type_index: i32,
    /// Hex-encoded initial `mapping_function` blob for `function/data`.
    pub(crate) initial_function_hex: String,
}

/// A deferred shader mutation: create a new `parameters[]` element, set its
/// `parameter name`, then initialise one or more leaf fields. Used when the
/// user edits a shader parameter that has no existing instance in the tag.
#[derive(Clone)]
pub(crate) struct ShaderParamOp {
    /// Absolute path to the `parameters` block, e.g. `render_method/parameters`.
    pub(crate) parameters_block_path: String,
    /// The parameter name to write into the new element's `parameter name`.
    pub(crate) parameter_name: String,
    /// Leaf field edits relative to the newly-created parameter element.
    pub(crate) initial_fields: Vec<ShaderParamInitialField>,
    /// Animated parameter children to append below the newly-created element.
    pub(crate) animated_parameters: Vec<ShaderParamInitialAnimated>,
}

#[derive(Clone)]
pub(crate) struct ShaderParamInitialField {
    pub(crate) field: String,
    pub(crate) input: String,
}

#[derive(Clone)]
pub(crate) struct ShaderParamInitialAnimated {
    pub(crate) output_type_index: i32,
    pub(crate) initial_function_hex: String,
}

#[derive(Clone)]
pub(crate) enum ModelVariantOp {
    Create {
        name: String,
        regions: Vec<ModelVariantRegionChoice>,
    },
    Update {
        variant_index: usize,
        regions: Vec<ModelVariantRegionChoice>,
    },
}

#[derive(Clone)]
pub(crate) struct ModelVariantRegionChoice {
    pub(crate) region_name: String,
    pub(crate) permutation_name: String,
}
