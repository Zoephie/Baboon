//! Shader editor models, engine-specific parameter mapping, and grid widgets.
//! It owns shader-specific models, edits, and presentation helpers; generic field editing, and the commands that apply edits, belong elsewhere.

use super::*;

mod h2;
pub(in crate::app) use h2::*;
mod render_method;
pub(in crate::app) use render_method::*;
mod rows;
pub(in crate::app) use rows::*;
mod editing;
pub(in crate::app) use editing::*;
mod widgets;
pub(in crate::app) use widgets::*;

/// Normalized material parameter prepared before the grid is drawn.
/// `priority` supplies stable ordering without changing the source block order.
pub(in crate::app) struct MaterialParameterValue {
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
pub(in crate::app) struct ShaderGridCell {
    text: String,
    value_kind: &'static str,
    color: Option<MaterialColorPopup>,
}

/// One logical shader parameter row with inherited/default and current values.
/// Creation and edit targets are mutually contextual: absent backing data uses a
/// create operation, while explicit backing data uses `edit` and can be reset.
pub(in crate::app) struct ShaderGridRow {
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
pub(in crate::app) struct ShaderContextMenu {
    items: Vec<ShaderContextItem>,
}

/// One action available in a `ShaderContextMenu`.
pub(in crate::app) struct ShaderContextItem {
    label: String,
    action: ShaderContextAction,
}

#[derive(Clone)]
/// Deferred action selected from a shader row context menu.
/// Actions are applied after drawing so no tag block is mutated while borrowed.
pub(in crate::app) enum ShaderContextAction {
    AnimatedParameter(ShaderOp),
    FieldEdits(Vec<PendingFieldEdit>),
    ParameterOp(ShaderParamOp),
    H2ParameterOp(H2ShaderParamOp),
}

/// Editable backing for a shader grid row's value cell.
/// Paths identify either an existing field or the parent needed to materialize a
/// missing parameter, as described by the corresponding edit kind.
#[derive(Clone)]
pub(in crate::app) struct ShaderRowEdit {
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
pub(in crate::app) enum ShaderRowEditKind {
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
    /// A structural tag reference — a shader's `definition`
    /// (render_method_definition) or `shader template`
    /// (render_method_template). Foundation exposes both for editing under
    /// expert mode and reconciles nothing afterwards, so this commits as a
    /// plain reference write and leaves the existing parameters alone.
    StructuralRef {
        group_tag: u32,
        extension: &'static str,
    },
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
pub(in crate::app) struct ShaderParamCreateTarget {
    parameters_block_path: String,
    parameter_name: String,
    parameter_type_index: i32,
    field: &'static str,
}

#[derive(Clone)]
/// Creation target for a constant function, distinguishing an existing parent
/// parameter from one that must be created with its animated child atomically.
pub(in crate::app) enum ShaderFunctionCreateTarget {
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
pub(in crate::app) struct ShaderEditorModel {
    /// True only for the 7 material-bearing shader types (shader/terrain/
    /// custom/halogram/foliage/skin/cortana); gates the MATERIAL section.
    has_material_row: bool,
    /// The shader's global material types (the root's `material name`
    /// fields), one row each.
    materials: Vec<ShaderMaterialName>,
    definition_path: String,
    /// Absolute tag field paths for the two structural references. Foundation
    /// exposes both for editing under expert mode — its expert gate is a
    /// blanket field-panel switch, and it carries no shader-aware code that
    /// could reconcile parameters afterwards, so re-pointing one leaves the
    /// existing parameters describing the old definition. Empty when the field
    /// could not be located.
    definition_edit_path: String,
    shader_template_path: Option<String>,
    shader_template_edit_path: String,
    categories: Vec<ShaderEditorCategory>,
    sections: Vec<ShaderEditorSection>,
    atmosphere_flags: ShaderFlagsRow,
    custom_fog_setting_index: ShaderGridRow,
    sort_layer: ShaderGridRow,
}

/// The 7 shader types that carry a `global material type` row (the first 8
/// interface ctors in Guerilla, minus the base). The 6 effect-style shaders
/// (particle/contrail/light_volume/beam/decal/water) have no material row.
pub(in crate::app) fn shader_type_has_material_row(group_tag: u32) -> bool {
    matches!(
        &group_tag.to_be_bytes(),
        b"rmsh" | b"rmtr" | b"rmcs" | b"rmhg" | b"rmfl" | b"rmsk" | b"rmct"
    )
}

pub(in crate::app) struct ShaderEditorCategory {
    index: usize,
    name: String,
    options: Vec<String>,
    selected: i16,
    edit_path: Option<String>,
}

pub(in crate::app) struct ShaderEditorSection {
    title: String,
    option_name: String,
    rows: Vec<ShaderGridRow>,
}

pub(in crate::app) struct ShaderFlagsRow {
    label: String,
    path: String,
    raw: u64,
    options: Vec<ShaderFlagOption>,
}

pub(in crate::app) struct ShaderFlagOption {
    bit: u32,
    label: &'static str,
}

pub(in crate::app) fn build_shader_editor_model(
    tag: &TagFile,
    group_tag: u32,
    source: Option<&TagSource>,
    rmdf_cache: &mut HashMap<String, Option<Arc<RenderMethodDefinition>>>,
    rmop_cache: &mut HashMap<String, Option<Arc<RenderMethodOption>>>,
) -> Option<ShaderEditorModel> {
    let source = source?;
    // Guarded: a recompiled shader can carry animated-parameter type names
    // blam-tags' enum resolver panics on, and this runs mid-frame.
    let render_method = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        RenderMethod::from_tag(tag).ok()
    }))
    .ok()
    .flatten()?;
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

    let materials = read_shader_material_names(tag);
    // Empty when the field is not there, so the row stays read-only rather than
    // offering an edit that would fail to commit. `render_method_existing_field_path`
    // falls back to its first candidate whether or not it exists, which is the
    // wrong shape here.
    let resolve_edit_path = |candidates: &[String]| -> String {
        candidates
            .iter()
            .find(|path| tag.root().field_path(path).is_some())
            .cloned()
            .unwrap_or_default()
    };
    let definition_edit_path = resolve_edit_path(&[
        append_field_path(&edit_prefix, "definition"),
        append_field_path(&edit_prefix, "definition*"),
    ]);
    // The template reference hangs off the postprocess block, not the
    // render_method root — `RenderMethod` reads it from `postprocess[0]`.
    let shader_template_edit_path = resolve_edit_path(&[
        append_field_path(&edit_prefix, "postprocess[0]/shader template"),
        append_field_path(&edit_prefix, "shader template"),
    ]);
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
        materials,
        definition_path: render_method.definition_path,
        definition_edit_path,
        shader_template_edit_path,
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
mod tests {
    use super::*;
    use blam_tags::render_method::{
    BitmapAddressMode, BitmapComparisonFunction, BitmapFilterMode, RenderMethodDefinitionCategory,
    RenderMethodDefinitionCategoryOption, RenderMethodParameterType,
};
    use blam_tags::{Enum, TagFieldData, TagReferenceData};

    // The Halo 3 shader grid, drawn headless over a synthetic shader.
    //
    // Characterization: each test draws the grid the tag pane draws, interacts
    // with one row the way a user would, and checks the deferred op the row
    // raised and what applying it does to the tag. Nothing is read off disk:
    // the shader comes from the definitions, and its render method definition
    // and options are put straight into the caches the grid reads them from.

    const GAME: &str = "halo3_mcc";

    const TAG_KEY: &str = "file:shaders/grid.shader";

    /// The six parameter kinds an option can declare, in the order the fixture
    /// gives them to an option's parameters.
    const KINDS: [RenderMethodParameterType; 6] = [
        RenderMethodParameterType::Bitmap,
        RenderMethodParameterType::Color,
        RenderMethodParameterType::Real,
        RenderMethodParameterType::Int,
        RenderMethodParameterType::Bool,
        RenderMethodParameterType::ArgbColor,
    ];

    fn new_tag(group: &str) -> TagFile {
        TagFile::new(
            locate_definitions_root()
                .join(GAME)
                .join(format!("{group}.json")),
        )
        .unwrap_or_else(|error| panic!("{GAME}/{group}.json: {error:?}"))
    }

    /// A shader naming `shaders\grid` as its definition, with one options slot
    /// per category.
    fn synthetic_shader(categories: usize) -> TagFile {
        let mut tag = new_tag("shader");
        let mut root = tag.root_mut();
        root.field_path_mut("render_method/definition")
            .expect("shader has render_method/definition")
            .set(TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: Some((u32::from_be_bytes(*b"rmdf"), "shaders\\grid".to_owned())),
            }))
            .expect("set the shader's definition");
        let mut field = root
            .field_path_mut("render_method/options")
            .expect("shader has render_method/options");
        let mut options = field.as_block_mut().expect("options is a block");
        for _ in 0..categories {
            options.add_element();
        }
        tag
    }

    /// `categories` categories of two options each; every option declares one
    /// parameter of each kind in [`KINDS`], named `p{category}_{kind index}`.
    fn render_method_caches(
        categories: usize,
    ) -> (
        HashMap<String, Option<Arc<RenderMethodDefinition>>>,
        HashMap<String, Option<Arc<RenderMethodOption>>>,
    ) {
        let mut rmdf = HashMap::new();
        let mut rmop = HashMap::new();
        let mut definition_categories = Vec::new();
        for category in 0..categories {
            let mut options = Vec::new();
            for option in 0..2 {
                let option_path = format!("shaders\\grid_options\\cat{category}_opt{option}");
                options.push(RenderMethodDefinitionCategoryOption {
                    option_name: format!("option_{option}"),
                    option_path: option_path.clone(),
                    vertex_function: String::new(),
                    pixel_function: String::new(),
                });
                let parameters = KINDS
                    .iter()
                    .enumerate()
                    .map(|(index, kind)| RenderMethodOptionParameter {
                        parameter_name: format!("p{category}_{index}"),
                        parameter_type: Some(Enum::from_variant(*kind)),
                        source_extern: None,
                        default_bitmap_path: String::new(),
                        default_real_value: index as f32,
                        default_int_bool_value: 0,
                        flags: 0,
                        default_filter_mode: Enum::from_variant(BitmapFilterMode::Trilinear),
                        default_comparison_function: Enum::from_variant(
                            BitmapComparisonFunction::Never,
                        ),
                        default_address_mode: Enum::from_variant(BitmapAddressMode::Wrap),
                        default_filter_mode_index: 0,
                        default_address_mode_index: 0,
                        anisotropy_amount: 0,
                        default_color: blam_tags::math::ArgbColor(0xff80_4020),
                        default_bitmap_scale: 1.0,
                        help_text: String::new(),
                    })
                    .collect();
                rmop.insert(
                    format!("rmop:{option_path}"),
                    Some(Arc::new(RenderMethodOption {
                        parameters,
                        filter_mode_names: Vec::new(),
                        address_mode_names: Vec::new(),
                    })),
                );
            }
            definition_categories.push(RenderMethodDefinitionCategory {
                category_name: format!("grid_category_{category}"),
                vertex_function: String::new(),
                pixel_function: String::new(),
                options,
            });
        }
        rmdf.insert(
            "rmdf:shaders\\grid".to_owned(),
            Some(Arc::new(RenderMethodDefinition {
                global_options_path: String::new(),
                categories: definition_categories,
                shared_pixel_shaders_path: String::new(),
                shared_vertex_shaders_path: String::new(),
                flags: 0,
                version: 0,
            })),
        );
        (rmdf, rmop)
    }

    /// What one frame painted: each text with its screen rectangle.
    type Painted = Vec<(String, egui::Rect)>;

    /// The shader pane, without the rest of the app: the document, the caches,
    /// the popups the grid opens, and the ops it raised since the last apply.
    struct Grid {
        ctx: egui::Context,
        time: f64,
        doc: TagDocument,
        revision: u64,
        entry: TagEntry,
        names: TagNameIndex,
        source: TagSource,
        rmdf: HashMap<String, Option<Arc<RenderMethodDefinition>>>,
        rmop: HashMap<String, Option<Arc<RenderMethodOption>>>,
        h2_templates: H2TemplateCache,
        buffers: EditDrafts,
        ops: DeferredOps,
        color_popup: Option<MaterialColorPopup>,
        function_popup: Option<FunctionPopup>,
        painted: Painted,
    }

    impl Grid {
        fn new(categories: usize) -> Self {
            let (rmdf, rmop) = render_method_caches(categories);
            let shader = synthetic_shader(categories);
            Self {
                ctx: egui::Context::default(),
                time: 0.0,
                entry: TagEntry {
                    key: TAG_KEY.to_owned(),
                    display_path: "shaders/grid.shader".to_owned(),
                    group_tag: shader.header.group_tag,
                    group_name: Some("shader".to_owned()),
                    location: TagEntryLocation::LooseFile("shaders/grid.shader".into()),
                },
                doc: TagDocument::clean(shader),
                revision: 1,
                names: TagNameIndex::load_from_definitions(&locate_definitions_root()),
                source: TagSource::SingleFile {
                    path: PathBuf::from("grid-synthetic"),
                },
                rmdf,
                rmop,
                h2_templates: H2TemplateCache::default(),
                buffers: EditDrafts::default(),
                ops: DeferredOps::default(),
                color_popup: None,
                function_popup: None,
                painted: Vec::new(),
            }
        }

        /// Draw one frame of the grid with `events`, collecting what it raised.
        fn frame(&mut self, events: Vec<egui::Event>) -> &Painted {
            self.time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 2400.0),
                )),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            };
            let definitions_root = locate_definitions_root();
            let Self {
                ctx,
                doc,
                revision,
                entry,
                names,
                source,
                rmdf,
                rmop,
                h2_templates,
                buffers,
                ops,
                color_popup,
                function_popup,
                ..
            } = self;
            let output = crate::app::run_ui_test(&ctx, input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let mut sinks = EditSinks::default();
                    let mut edit = FieldEditContext::read_only(&mut sinks, "test", TAG_KEY);
                    edit.group_tag = doc.tag.header.group_tag;
                    edit.root = Some(doc.tag.root());
                    edit.game = GameId::from_id(GAME);
                    edit.definitions_root = Some(definitions_root.as_path());
                    edit.names = Some(names);
                    edit.editable = true;
                    edit.buffers = buffers;
                    edit.pending = &mut ops.pending;
                    edit.block_ops = &mut ops.block_ops;
                    edit.shader_ops = &mut ops.shader_ops;
                    edit.shader_param_ops = &mut ops.shader_param_ops;
                    edit.h2_shader_param_ops = &mut ops.h2_shader_param_ops;
                    edit.model_variant_ops = &mut ops.model_variant_ops;
                    draw_material_tag(
                        ui,
                        &doc.tag,
                        (*revision, 0, 0, 0),
                        entry,
                        names,
                        Some(source),
                        rmdf,
                        rmop,
                        h2_templates,
                        color_popup,
                        function_popup,
                        false,
                        &mut edit,
                    );
                });
            });
            self.painted = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        text.galley.rect.translate(text.pos.to_vec2()),
                    )),
                    _ => None,
                })
                .collect();
            &self.painted
        }

        fn idle(&mut self, frames: usize) {
            for _ in 0..frames {
                self.frame(Vec::new());
            }
        }

        fn texts(&self) -> Vec<&str> {
            self.painted.iter().map(|(text, _)| text.as_str()).collect()
        }

        fn count(&self, text: &str) -> usize {
            self.painted.iter().filter(|(shown, _)| shown == text).count()
        }

        /// The `nth` painted text equal to `text`, top to bottom.
        fn find(&self, text: &str, nth: usize) -> egui::Rect {
            let mut found: Vec<egui::Rect> = self
                .painted
                .iter()
                .filter(|(shown, _)| shown == text)
                .map(|(_, rect)| *rect)
                .collect();
            found.sort_by(|a, b| {
                a.top()
                    .total_cmp(&b.top())
                    .then(a.left().total_cmp(&b.left()))
            });
            *found
                .get(nth)
                .unwrap_or_else(|| panic!("no {nth}th {text:?} in {:?}", self.texts()))
        }

        /// Slide the pointer onto `pos` over a few frames, then click it.
        fn click_at(&mut self, pos: egui::Pos2) {
            for step in 1..=3 {
                let t = step as f32 / 3.0;
                let from = egui::pos2(pos.x - 40.0, pos.y - 40.0);
                self.frame(vec![egui::Event::PointerMoved(from + (pos - from) * t)]);
            }
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            // A popup the click opened lays itself out unseen on its first
            // frame; one more shows it.
            self.frame(Vec::new());
        }

        fn right_click_at(&mut self, pos: egui::Pos2) {
            for step in 1..=3 {
                let t = step as f32 / 3.0;
                let from = egui::pos2(pos.x - 40.0, pos.y - 40.0);
                self.frame(vec![egui::Event::PointerMoved(from + (pos - from) * t)]);
            }
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.frame(Vec::new());
        }

        fn click(&mut self, text: &str, nth: usize) {
            let pos = self.find(text, nth).center();
            self.click_at(pos);
        }

        fn type_text(&mut self, text: &str) {
            let select_all = egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            };
            self.frame(vec![select_all]);
            self.frame(vec![egui::Event::Text(text.to_owned())]);
            let enter = egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![enter]);
        }

        /// Apply what the grid raised, as the tag pane does after its draw, and
        /// move the document to a new revision so the grid rebuilds from it.
        fn apply(&mut self) -> AppliedDeferredOps {
            let ops = std::mem::take(&mut self.ops);
            let applied = apply_deferred_ops(&mut self.doc, ops, "test");
            // The pane applies every frame; the next frame's empty batch is what
            // closes the undo step.
            apply_deferred_ops(&mut self.doc, DeferredOps::default(), "test");
            self.revision += 1;
            self.idle(2);
            applied
        }

        fn parameters(&self) -> Vec<(String, TagStruct<'_>)> {
            let root = self.doc.tag.root();
            let Some(block) = root
                .field_path("render_method/parameters")
                .and_then(|field| field.as_block())
            else {
                return Vec::new();
            };
            (0..block.len())
                .map(|index| {
                    let element = block.element(index).unwrap();
                    let name = match element.field("parameter name").and_then(|f| f.value()) {
                        Some(TagFieldData::StringId(id)) => id.string,
                        other => panic!("parameter name is {other:?}"),
                    };
                    (name, element)
                })
                .collect()
        }
    }

    /// Every row kind an option can declare is drawn, inherited from the
    /// option's defaults until the shader overrides it.
    #[test]
    fn the_grid_draws_every_row_kind_from_the_options_defaults() {
        let mut grid = Grid::new(2);
        grid.idle(2);
        for header in ["CATEGORIES", "GRID_CATEGORY_0", "GRID_CATEGORY_1"] {
            assert_eq!(grid.count(header), 1, "{header}: {:?}", grid.texts());
        }
        for category in 0..2 {
            assert_eq!(grid.count(&format!("grid_category_{category}")), 1);
            for kind in 0..KINDS.len() {
                assert_eq!(
                    grid.count(&format!("p{category}_{kind}")),
                    1,
                    "p{category}_{kind}: {:?}",
                    grid.texts()
                );
            }
        }
        // The selected option is shown per section, and each category's combo
        // shows the shader's choice (slot 0 → option_0) beside its default.
        assert_eq!(grid.count("selected option"), 2);
        assert_eq!(grid.count("option_0"), 2 + 2 + 2);
        // Every inherited row but the int offers "Override Default": bitmap,
        // both colors and their alpha rows, real and bool — seven a section.
        // The colors also offer "f()+" to create function data.
        assert_eq!(grid.count("Override Default"), 7 * 2, "{:?}", grid.texts());
        assert_eq!(grid.count("f()+"), 2 * 2);
        assert_eq!(grid.count("value: 2.0"), 2, "the real default, per section");
        for alpha in ["p0_1_alpha", "p0_5_alpha", "p1_1_alpha", "p1_5_alpha"] {
            assert_eq!(grid.count(alpha), 1, "{alpha}");
        }
        // The int row has no instance to write to, so it is a read-only "0"
        // beside its default; the bool reads "false"; the bitmap "NONE".
        assert_eq!(grid.count("false"), 2);
        assert_eq!(grid.count("NONE"), 2);
        // The grid's fixed sections follow the categories.
        for section in ["MATERIAL", "ATMOSPHERE PROPERTIES", "SORTING PROPERTIES"] {
            assert_eq!(grid.count(section), 1, "{section}");
        }
        assert_eq!(grid.count("shaders\\grid.render_method_definition"), 1);
        assert!(grid.ops.is_empty(), "drawing alone raised an edit");
    }

    /// Picking another option in a category's combo writes the option index into
    /// the shader's slot for that category.
    #[test]
    fn choosing_a_category_option_writes_its_index() {
        let mut grid = Grid::new(2);
        grid.idle(2);
        // The second "option_0" in top-to-bottom order is category 0's combo
        // (the first is its default cell).
        let combo = grid.find("option_0", 1);
        grid.click_at(combo.center());
        grid.click("option_1", 0);
        assert_eq!(grid.ops.pending.len(), 1);
        assert_eq!(grid.ops.pending[0].path, "render_method/options[0]/short");
        assert_eq!(grid.ops.pending[0].input, "1");
        grid.apply();
        let render_method = RenderMethod::from_tag(&grid.doc.tag).unwrap();
        assert_eq!(render_method.options, vec![1, 0]);
        assert!(grid.doc.dirty.is_set());
        // The section now draws the newly selected option.
        assert_eq!(grid.count("option_1"), 1 + 1, "{:?}", grid.texts());
    }

    /// "Override Default" on an inherited real creates the parameter at its
    /// default; the row then edits that value in place, and its × resets it.
    #[test]
    fn a_real_parameter_is_overridden_edited_and_reset() {
        let mut grid = Grid::new(1);
        grid.idle(2);
        // Rows are bitmap, color, color alpha, real…: the real's button is the
        // fourth "Override Default".
        grid.click("Override Default", 3);
        assert_eq!(grid.ops.shader_param_ops.len(), 1, "one create op");
        let op = &grid.ops.shader_param_ops[0];
        assert_eq!(op.parameters_block_path, "render_method/parameters");
        assert_eq!(op.parameter_name, "p0_2");
        let fields: Vec<(&str, &str)> = op
            .initial_fields
            .iter()
            .map(|field| (field.field.as_str(), field.input.as_str()))
            .collect();
        assert_eq!(fields, [("parameter type", "2"), ("real", "2.0")]);
        grid.apply();

        let parameters = grid.parameters();
        assert_eq!(parameters.len(), 1);
        assert_eq!(parameters[0].0, "p0_2");
        assert_eq!(parameters[0].1.read_real("real"), Some(2.0));
        assert_eq!(grid.count("Override Default"), 6, "the real is overridden");

        // The row is a text box over `parameters[0]/real` now.
        let value = grid.find("2.0", 0);
        grid.click_at(value.center());
        grid.type_text("5.5");
        assert_eq!(grid.ops.pending.len(), 1, "{:?}", grid.ops.pending.len());
        assert_eq!(grid.ops.pending[0].path, "render_method/parameters[0]/real");
        assert_eq!(grid.ops.pending[0].input, "5.5");
        grid.apply();
        assert_eq!(grid.parameters()[0].1.read_real("real"), Some(5.5));

        // The × beside it deletes the parameter again.
        grid.click("×", 0);
        assert_eq!(grid.ops.block_ops.len(), 1);
        assert_eq!(grid.ops.block_ops[0].path, "render_method/parameters");
        assert!(matches!(grid.ops.block_ops[0].kind, BlockOpKind::Delete(0)));
        grid.apply();
        assert!(grid.parameters().is_empty());
        assert_eq!(grid.count("Override Default"), 7);
    }

    /// A shader value typed and not yet committed leaves behind the edit its
    /// box would commit, for a save or a close to commit without the row.
    #[test]
    fn a_typed_shader_value_commits_without_its_row() {
        let mut grid = Grid::new(1);
        grid.idle(2);
        grid.click("Override Default", 3);
        grid.apply();
        let value = grid.find("2.0", 0);
        grid.click_at(value.center());
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        grid.frame(vec![select_all]);
        grid.frame(vec![egui::Event::Text("5.5".to_owned())]);
        assert!(grid.ops.pending.is_empty(), "nothing committed yet");

        let commits = grid.buffers.take_uncommitted(DraftFlush::All);
        let [(tag, Ok(ops))] = commits.as_slice() else {
            panic!("one commit, got {}", commits.len());
        };
        assert_eq!(tag, TAG_KEY);
        assert_eq!(ops.pending.len(), 1);
        assert_eq!(ops.pending[0].path, "render_method/parameters[0]/real");
        assert_eq!(ops.pending[0].input, "5.5");
        assert!(grid.buffers.take_uncommitted(DraftFlush::All).is_empty(), "and only once");
    }

    /// Frames of a free-standing popup (the colour picker, the function editor),
    /// drawn by `draw` against the popup state it owns.
    struct Popup {
        ctx: egui::Context,
        time: f64,
        painted: Painted,
    }

    impl Popup {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                time: 0.0,
                painted: Vec::new(),
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>, draw: &mut dyn FnMut(&mut egui::Ui)) {
            self.time += 1.0 / 60.0;
            let output = crate::app::run_ui_test(
                &self.ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 1000.0),
                    )),
                    time: Some(self.time),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ui| draw(ui),
            );
            self.painted = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        text.galley.rect.translate(text.pos.to_vec2()),
                    )),
                    _ => None,
                })
                .collect();
        }

        fn find(&self, text: &str) -> egui::Rect {
            self.painted
                .iter()
                .find(|(shown, _)| shown == text)
                .map(|(_, rect)| *rect)
                .unwrap_or_else(|| {
                    let texts: Vec<_> = self.painted.iter().map(|(text, _)| text).collect();
                    panic!("no {text:?} in {texts:?}")
                })
        }

        fn click_at(&mut self, pos: egui::Pos2, draw: &mut dyn FnMut(&mut egui::Ui)) {
            for step in 1..=3 {
                let t = step as f32 / 3.0;
                let from = egui::pos2(pos.x - 40.0, pos.y - 40.0);
                self.frame(vec![egui::Event::PointerMoved(from + (pos - from) * t)], draw);
            }
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![button(true)], draw);
            self.frame(vec![button(false)], draw);
            self.frame(Vec::new(), draw);
        }

        fn click(&mut self, text: &str, draw: &mut dyn FnMut(&mut egui::Ui)) {
            let pos = self.find(text).center();
            self.click_at(pos, draw);
        }
    }

    /// The constant colour an animated parameter's function holds, if it is one.
    fn constant_color(tag: &TagFile, parameter: usize, animated: usize) -> Option<[f32; 4]> {
        let render_method = RenderMethod::from_tag(tag).ok()?;
        let function = render_method.parameters[parameter].animated_parameters[animated]
            .function
            .clone()?;
        extract_constant_color(&function)
    }

    /// The rightmost painted `text` on the row labelled `label`.
    fn rightmost_on_row(grid: &Grid, label: &str, text: &str) -> egui::Rect {
        let row_y = grid.find(label, 0).center().y;
        grid.painted
            .iter()
            .filter(|(shown, rect)| shown == text && (rect.center().y - row_y).abs() < 4.0)
            .map(|(_, rect)| *rect)
            .max_by(|a, b| a.left().total_cmp(&b.left()))
            .unwrap_or_else(|| panic!("no {text:?} on {label}'s row: {:?}", grid.texts()))
    }

    /// An inherited colour is overridden with a constant colour function at the
    /// option's default; its swatch then opens the picker, whose OK writes the
    /// picked colour back as that function's data.
    #[test]
    fn a_color_is_overridden_and_repicked_through_the_color_picker() {
        let mut grid = Grid::new(1);
        grid.idle(2);
        // The colour's button is the second "Override Default" (after the
        // bitmap's).
        grid.click("Override Default", 1);
        assert_eq!(grid.ops.shader_param_ops.len(), 1);
        let op = &grid.ops.shader_param_ops[0];
        assert_eq!(op.parameter_name, "p0_1");
        assert_eq!(op.initial_fields.len(), 1, "only the parameter type");
        assert_eq!(op.initial_fields[0].field, "parameter type");
        assert_eq!(op.animated_parameters.len(), 1);
        assert_eq!(
            op.animated_parameters[0].output_type_index,
            RenderMethodAnimatedParameterType::Color as i32
        );
        grid.apply();
        let parameters = grid.parameters();
        assert_eq!(parameters.len(), 1);
        assert_eq!(parameters[0].0, "p0_1");
        // The default 0xff804020, as a constant RGB function at full alpha.
        let rgba = constant_color(&grid.doc.tag, 0, 0).expect("a constant colour");
        assert_eq!(
            rgba.map(float_channel_to_u8),
            [0x80, 0x40, 0x20, 0xff],
            "{rgba:?}"
        );

        // Its value cell is a swatch now; the "color: RGB" it paints sits just
        // right of it.
        // Both the colour and its alpha row are backed by the new parameter.
        assert_eq!(grid.count("Override Default"), 5);
        let label = rightmost_on_row(&grid, "p0_1", "color: RGB");
        grid.click_at(egui::pos2(label.left() - 20.0, label.center().y));
        let popup = grid.color_popup.take().expect("the swatch opened the picker");
        assert!(grid.ops.is_empty(), "opening the picker edits nothing");

        // Type a hex colour into the picker and press OK.
        let mut color_popup = Some(popup);
        let mut swatches = default_color_swatches();
        let mut last_dir = None;
        let mut result = None;
        let mut draw = |ui: &mut egui::Ui| {
            if let Some(done) =
                draw_color_popup(ui.ctx(), &mut color_popup, &mut swatches, &mut last_dir)
            {
                result = Some(done);
            }
        };
        let mut picker = Popup::new();
        // A window's first frame lays out and paints nothing.
        picker.frame(Vec::new(), &mut draw);
        picker.frame(Vec::new(), &mut draw);
        let hex = picker.find("Hex:");
        picker.click_at(egui::pos2(hex.right() + 60.0, hex.center().y), &mut draw);
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        picker.frame(vec![select_all], &mut draw);
        picker.frame(vec![egui::Event::Text("#00FF00".to_owned())], &mut draw);
        picker.frame(Vec::new(), &mut draw);
        picker.click("OK", &mut draw);
        drop(draw);
        assert!(color_popup.is_none(), "OK closes the picker");
        let Some(ColorPopupResult::FieldEdit { tag_key, edit }) = result else {
            panic!("OK on a function colour writes its data");
        };
        assert_eq!(tag_key, TAG_KEY);
        assert_eq!(
            edit.path,
            "render_method/parameters[0]/animated parameters[0]/function/data"
        );
        assert_eq!(edit.input, constant_color_function_hex(0.0, 1.0, 0.0, 1.0));
        grid.ops.pending.push(edit);
        grid.apply();
        let rgba = constant_color(&grid.doc.tag, 0, 0).expect("still a constant colour");
        assert_eq!(rgba.map(float_channel_to_u8), [0, 0xff, 0, 0xff]);
    }

    /// A constant colour's "f()" opens the function editor on that animated
    /// parameter. Retyping the function there and pressing OK writes the new
    /// function data, and leaves the parameter's wrapper fields alone.
    #[test]
    fn the_function_editor_retypes_an_animated_parameter() {
        let mut grid = Grid::new(1);
        grid.idle(2);
        grid.click("Override Default", 1);
        grid.apply();

        // The row's controls, right to left: the "×" that removes the animated
        // parameter, and "f()" just before it.
        let delete = rightmost_on_row(&grid, "p0_1", "×");
        grid.click_at(egui::pos2(delete.center().x - 24.0, delete.center().y));
        let popup = grid
            .function_popup
            .take()
            .expect("f() opened the function editor");
        assert!(grid.ops.is_empty());

        let mut function_popup = Some(popup);
        let mut color_popup = None;
        let mut batch = None;
        let mut draw = |ui: &mut egui::Ui| {
            if let Some(done) = draw_function_popup(ui.ctx(), &mut function_popup, &mut color_popup) {
                batch = Some(done);
            }
        };
        let mut editor = Popup::new();
        editor.frame(Vec::new(), &mut draw);
        editor.frame(Vec::new(), &mut draw);
        // A constant is a "basic" function: no graph, just its value rail.
        editor.click("basic", &mut draw);
        editor.click("periodic", &mut draw);
        editor.click("OK", &mut draw);
        drop(draw);
        assert!(function_popup.is_none(), "OK closes the editor");
        let batch = batch.expect("a retyped function is an edit");
        assert_eq!(batch.tag_key, TAG_KEY);
        assert!(batch.data_ops.is_empty(), "H3 data is a field, not a byte block");
        let paths: Vec<&str> = batch.edits.iter().map(|edit| edit.path.as_str()).collect();
        assert_eq!(
            paths,
            ["render_method/parameters[0]/animated parameters[0]/function/data"]
        );
        grid.ops.pending.extend(batch.edits);
        grid.apply();
        let render_method = RenderMethod::from_tag(&grid.doc.tag).unwrap();
        let function = render_method.parameters[0].animated_parameters[0]
            .function
            .clone()
            .expect("a function");
        assert_eq!(
            TagFunctionEditor::from_function(function).master_type(),
            EngineMasterType::Periodic
        );
    }

    /// An inherited bool overrides to its default, then toggles through its
    /// checkbox.
    #[test]
    fn a_bool_parameter_is_overridden_then_toggled() {
        let mut grid = Grid::new(1);
        grid.idle(2);
        // bitmap, color, color alpha, real, bool: the fifth button.
        grid.click("Override Default", 4);
        assert_eq!(grid.ops.shader_param_ops.len(), 1);
        let op = &grid.ops.shader_param_ops[0];
        assert_eq!(op.parameter_name, "p0_4");
        let fields: Vec<(&str, &str)> = op
            .initial_fields
            .iter()
            .map(|field| (field.field.as_str(), field.input.as_str()))
            .collect();
        assert_eq!(fields, [("parameter type", "4"), ("int/bool", "0")]);
        grid.apply();
        assert_eq!(grid.parameters()[0].0, "p0_4");

        // The value cell is a checkbox at the start of the value column, which
        // starts where the "selected option" row's value text does.
        let row_y = grid.find("p0_4", 0).center().y;
        let value_left = grid.find("option_0", 2).left();
        grid.click_at(egui::pos2(value_left + 6.0, row_y));
        assert_eq!(grid.ops.pending.len(), 1, "one toggle");
        assert_eq!(
            grid.ops.pending[0].path,
            "render_method/parameters[0]/int\\bool"
        );
        assert_eq!(grid.ops.pending[0].input, "1");
        grid.apply();
        let render_method = RenderMethod::from_tag(&grid.doc.tag).unwrap();
        assert_eq!(render_method.parameters[0].int_parameter, 1);
    }

    /// The rows that edit the render method itself rather than a parameter:
    /// the material type, the atmosphere flags, the custom fog index and the
    /// sort layer.
    #[test]
    fn the_render_method_rows_edit_its_own_fields() {
        let mut grid = Grid::new(1);
        grid.idle(2);

        // The material is the shader root's `material name`; typing one into the
        // row writes it there. (The row used to read and write
        // `render_method/global material type`, which no shader has, so it showed
        // `default_material` whatever the tag held and an edit failed to apply.)
        assert_eq!(grid.count("MATERIAL"), 1);
        let material = rightmost_on_row(&grid, "material name", "default_material");
        grid.click_at(material.center());
        grid.type_text("hard_metal_thin");
        assert_eq!(grid.ops.pending.len(), 1);
        grid.apply();
        let stored = match grid.doc.tag.root().field("material name").and_then(|field| field.value()) {
            Some(TagFieldData::StringId(id)) => id.string,
            other => panic!("material name is {other:?}"),
        };
        assert_eq!(stored, "hard_metal_thin");

        // A flag toggles its bit in the mask.
        grid.click("use custom setting", 0);
        assert_eq!(grid.ops.pending.len(), 1);
        assert_eq!(grid.ops.pending[0].input, "2");
        grid.apply();
        let render_method = RenderMethod::from_tag(&grid.doc.tag).unwrap();
        assert_eq!(render_method_flags_mask(&render_method), 2);

        // The custom fog index is a number box (its right-hand "0").
        let fog = rightmost_on_row(&grid, "Custom Setting Index", "0");
        grid.click_at(fog.center());
        grid.type_text("7");
        assert_eq!(grid.ops.pending.len(), 1);
        grid.apply();
        let render_method = RenderMethod::from_tag(&grid.doc.tag).unwrap();
        assert_eq!(render_method.custom_fog_setting_index, 7);

        // The sort layer is a combo; a fresh tag holds "invalid".
        grid.click("invalid", 0);
        grid.click("post-pass", 0);
        assert_eq!(grid.ops.pending.len(), 1);
        assert_eq!(grid.ops.pending[0].input, "3");
        grid.apply();
        let render_method = RenderMethod::from_tag(&grid.doc.tag).unwrap();
        assert_eq!(render_method.sort_layer.name(), "post-pass");
        assert_eq!(grid.doc.journal.stacks().0.len(), 4, "undo steps");
    }

    /// "Override Default" on an inherited bitmap creates the parameter with no
    /// bitmap; the row then offers its sampler and transform rows.
    #[test]
    fn a_bitmap_parameter_is_overridden_and_expands() {
        let mut grid = Grid::new(1);
        grid.idle(2);
        grid.click("Override Default", 0);
        assert_eq!(grid.ops.shader_param_ops.len(), 1);
        let op = &grid.ops.shader_param_ops[0];
        assert_eq!(op.parameter_name, "p0_0");
        let fields: Vec<(&str, &str)> = op
            .initial_fields
            .iter()
            .map(|field| (field.field.as_str(), field.input.as_str()))
            .collect();
        assert_eq!(fields, [("parameter type", "0"), ("bitmap", "NONE")]);
        grid.apply();
        assert_eq!(grid.parameters()[0].0, "p0_0");
        assert_eq!(grid.count("Override Default"), 6);
        // The row is a reference cell now: the path box (still "NONE"), a
        // missing-target marker, browse and clear.
        for control in ["\u{26A0}", "...", "\u{d7}"] {
            assert_eq!(grid.count(control), 1, "{control}: {:?}", grid.texts());
        }

        // Typing a path into the box points the parameter at that bitmap.
        let path = rightmost_on_row(&grid, "p0_0", "NONE");
        grid.click_at(path.center());
        grid.type_text("shaders\\textures\\rock.bitmap");
        assert_eq!(grid.ops.pending.len(), 1);
        assert_eq!(grid.ops.pending[0].path, "render_method/parameters[0]/bitmap");
        let input = grid.ops.pending[0].input.clone();
        let applied = grid.apply();
        assert_eq!(applied.outcomes[0].result, Ok(()), "{input}");
        let render_method = RenderMethod::from_tag(&grid.doc.tag).unwrap();
        assert_eq!(render_method.parameters[0].bitmap_path, "shaders\\textures\\rock");
        // The box shows the reference as the tag holds it now, not the text
        // typed: a committed box goes back to the tag's value.
        assert_eq!(grid.count("shaders/textures/rock.bitmap"), 1, "{:?}", grid.texts());

        // Its context menu adds optional sampler and transform arguments;
        // "filter mode" sets the flag and the mode, which then gets a row.
        let label = grid.find("p0_0", 0).center();
        grid.right_click_at(label);
        grid.click("filter mode", 0);
        let edits: Vec<(&str, &str)> = grid
            .ops
            .pending
            .iter()
            .map(|edit| (edit.path.as_str(), edit.input.as_str()))
            .collect();
        assert_eq!(
            edits,
            [
                ("render_method/parameters[0]/bitmap flags", "1"),
                ("render_method/parameters[0]/bitmap filter mode", "0"),
            ]
        );
        // The mode field is a short: the default goes in as its index.
        let applied = grid.apply();
        for outcome in &applied.outcomes {
            assert_eq!(outcome.result, Ok(()));
        }
        assert_eq!(grid.count("p0_0_filter_mode"), 1, "{:?}", grid.texts());
    }
}
