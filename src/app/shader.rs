//! Shader editor models, engine-specific parameter mapping, and grid widgets.
//! It owns shader-specific models, edits, and presentation helpers; generic field editing and controller orchestration belong elsewhere.

use super::*;

mod h2;
pub(super) use h2::*;
mod render_method;
pub(super) use render_method::*;
mod rows;
pub(super) use rows::*;
mod editing;
pub(super) use editing::*;
mod widgets;
pub(super) use widgets::*;

/// Normalized material parameter prepared before the grid is drawn.
/// `priority` supplies stable ordering without changing the source block order.
pub(super) struct MaterialParameterValue {
    label: String,
    value: String,
    fill: Color32,
    value_kind: &'static str,
    color: Option<MaterialColorPopup>,
    priority: u8,
}

#[derive(Clone)]
/// Display-ready shader cell; `value_kind` is also used to compare inherited
/// and overridden values without depending on widget presentation.
pub(super) struct ShaderGridCell {
    text: String,
    value_kind: &'static str,
    color: Option<MaterialColorPopup>,
}

/// One logical shader parameter row with inherited/default and current values.
/// Creation and edit targets are mutually contextual: absent backing data uses a
/// create operation, while explicit backing data uses `edit` and can be reset.
pub(super) struct ShaderGridRow {
    label: String,
    default_cell: Option<ShaderGridCell>,
    value_cell: ShaderGridCell,
    fill: Color32,
    parameter_type: Option<String>,
    /// True when this row is backed by an explicit shader parameter/template
    /// instance. False means the visible value is inherited from the
    /// render-method option or H2 shader-template default.
    is_overridden: bool,
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
/// Deferred action selected from a shader row context menu.
/// Actions are applied after drawing so no tag block is mutated while borrowed.
pub(super) enum ShaderContextAction {
    AnimatedParameter(ShaderOp),
    FieldEdits(Vec<PendingFieldEdit>),
    ParameterOp(ShaderParamOp),
    H2ParameterOp(H2ShaderParamOp),
}

/// Editable backing for a shader grid row's value cell.
/// Paths identify either an existing field or the parent needed to materialize a
/// missing parameter, as described by the corresponding edit kind.
#[derive(Clone)]
pub(super) struct ShaderRowEdit {
    /// Full tag field path (slashes in field names escaped as `\/`).
    path: String,
    /// Clean current value used to seed/sync the in-place editor.
    current: String,
    kind: ShaderRowEditKind,
}

#[derive(Clone)]
/// Widget and commit semantics for a shader row.
/// Variants encode storage differences that look similar in the UI but require
/// distinct byte/block edits, especially classic H2 function-backed values.
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
    ShaderTemplateRef,
    /// Boolean checkbox backed by an existing field or a new shader parameter.
    Bool {
        create: Option<ShaderParamCreateTarget>,
    },
    /// Index-valued dropdown over the given option labels.
    Enum(Vec<String>),
    /// Bitmask rendered as labelled checkboxes.
    Flags(Vec<String>),
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
    ColorField {
        argb: bool,
    },
    /// No Color animated parameter exists yet. The swatch opens the color
    /// popup and OK creates one initialized to the selected constant color.
    CreateFunctionColor {
        target: ShaderFunctionCreateTarget,
    },
    /// No animated scalar function exists yet. Editing the numeric value creates
    /// one initialized to the entered constant.
    CreateFunctionScalar {
        target: ShaderFunctionCreateTarget,
    },
    H2FunctionScalar {
        block_path: String,
        legacy_data: Option<Vec<u8>>,
    },
    H2CreateFunctionScalar {
        create_op: H2ShaderParamOp,
    },
    H2FunctionColor {
        block_path: String,
        legacy_data: Option<Vec<u8>>,
    },
    H2CreateFunctionColor {
        create_op: H2ShaderParamOp,
    },
    /// No parameter instance exists yet. On commit a new `parameters[]`
    /// element is created via `ShaderParamOp`.
    CreateScalarParam {
        parameters_block_path: String,
        parameter_name: String,
        parameter_type_index: i32,
    },
    H2CreateTemplateValue {
        parameters_block_path: String,
        parameter_name: String,
        parameter_type_index: i32,
        field: String,
    },
    H2CreateTemplateColor {
        parameters_block_path: String,
        parameter_name: String,
        parameter_type_index: i32,
        field: String,
    },
}

#[derive(Clone)]
/// Location and schema defaults required to create a missing parameter element.
pub(super) struct ShaderParamCreateTarget {
    parameters_block_path: String,
    parameter_name: String,
    parameter_type_index: i32,
    field: &'static str,
}

#[derive(Clone)]
/// Creation target for a constant function, distinguishing an existing parent
/// parameter from one that must be created with its animated child atomically.
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

/// Complete immutable shader view model prepared before rendering.
/// Current values may be inherited; consumers must honor each row's
/// `is_overridden` flag rather than inferring ownership from displayed text.
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
    let sort_layer_options = vec![
        "invalid".to_owned(),
        "pre-pass".to_owned(),
        "normal".to_owned(),
        "post-pass".to_owned(),
    ];
    let sort_layer = shader_enum_value_row(
        "Sort layer".to_owned(),
        "normal".to_owned(),
        option_index_for_name(&sort_layer_options, render_method.sort_layer.name()),
        sort_layer_options,
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

#[cfg(test)]
#[path = "shader/tests.rs"]
mod extracted_tests;
