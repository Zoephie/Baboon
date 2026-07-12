//! editing application state.
//! It owns passive cross-frame state and operation messages; rendering and workflow execution belong to UI and controller modules.

use super::*;

#[derive(Clone)]
pub(in crate::app) struct PendingFieldEdit {
    pub(in crate::app) path: String,
    pub(in crate::app) input: String,
}

#[derive(Clone)]
/// Replacement bytes for one function's backing storage.
/// The whole blob is retained so edits can preserve unrecognized layout bytes.
pub(in crate::app) struct FunctionDataOp {
    pub(in crate::app) block_path: String,
    pub(in crate::app) data: Vec<u8>,
}

#[derive(Clone)]
/// Atomic structural mutations needed by classic Halo 2 shader parameters.
/// These operations defer block creation and byte writes until immutable render
/// borrows have ended; unknown bytes in existing function blobs must survive.
pub(in crate::app) enum H2ShaderParamOp {
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
    SwitchTemplate {
        parameters_block_path: String,
        allowed_parameter_names: Vec<String>,
    },
}

/// A deferred structural edit to a block (add/insert/duplicate/delete),
/// applied to the tag after the immutable render borrow ends.
#[derive(Clone)]
pub(in crate::app) enum BlockOpKind {
    Add,
    Insert(usize),
    Duplicate(usize),
    Delete(usize),
    DeleteAll,
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
pub(in crate::app) struct BlockOp {
    pub(in crate::app) path: String,
    pub(in crate::app) kind: BlockOpKind,
}

/// A copied block element, held on the app so it can be pasted into a block of
/// the same shape in another open tag. `group_tag` + `block_path` gate which
/// blocks accept the paste (same group, same block); the library re-validates
/// element compatibility before inserting.
#[derive(Clone)]
pub(in crate::app) struct BlockClipboard {
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) block_path: String,
    /// Human label for the menu, e.g. "initial permutation".
    pub(in crate::app) label: String,
    /// One element (Copy element) or every element (Copy entire block).
    pub(in crate::app) elements: Vec<blam_tags::TagBlockElement>,
}

/// A pending destructive block op awaiting user confirmation. Lives on the
/// app (persists across frames) and is shown as a modal.
pub(in crate::app) struct BlockConfirm {
    pub(in crate::app) tag_key: String,
    pub(in crate::app) path: String,
    pub(in crate::app) kind: BlockOpKind,
    pub(in crate::app) message: String,
    /// Label for the confirm button (e.g. "Delete", "Replace").
    pub(in crate::app) confirm_label: String,
}

/// A request to open a referenced tag in a new tab (from an "Open" button on
/// a tag-reference row). Resolved against the loose-folder tags root.
#[derive(Clone)]
pub(in crate::app) struct OpenTagRequest {
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) rel_path: String,
    /// When true, open the tag in a floating (torn-off) window instead of the
    /// docked tab rack. Set by Alt-clicking a reference's Open button.
    pub(in crate::app) float: bool,
}

/// A request to (re)import a geometry tag via `tool` (from the Import button on
/// a render/collision/physics-model or animation-graph reference).
#[derive(Clone)]
pub(in crate::app) struct ToolImportRequest {
    /// `tool` verb: "render" / "collision" / "physics" /
    /// "model-animations-uncompressed".
    pub(in crate::app) verb: &'static str,
    /// Source directory argument, e.g. `objects\characters\masterchief`.
    pub(in crate::app) source_dir: String,
}

/// A deferred shader mutation: append one `animated parameters[]` element to
/// the given block path, then initialise its `type` and `function/data`
/// fields. Applied after the frame's draw pass, like `BlockOp`, but in its
/// own pass so the add + field init can be done atomically.
#[derive(Clone)]
pub(in crate::app) struct ShaderOp {
    /// Absolute path to the `animated parameters` block, e.g.
    /// `render_method/parameters[2]/animated parameters`.
    pub(in crate::app) animated_block_path: String,
    /// Output channel index (`RenderMethodAnimatedParameterType as i32`).
    pub(in crate::app) output_type_index: i32,
    /// Hex-encoded initial `mapping_function` blob for `function/data`.
    pub(in crate::app) initial_function_hex: String,
}

/// A deferred shader mutation: create a new `parameters[]` element, set its
/// `parameter name`, then initialise one or more leaf fields. Used when the
/// user edits a shader parameter that has no existing instance in the tag.
#[derive(Clone)]
pub(in crate::app) struct ShaderParamOp {
    /// Absolute path to the `parameters` block, e.g. `render_method/parameters`.
    pub(in crate::app) parameters_block_path: String,
    /// The parameter name to write into the new element's `parameter name`.
    pub(in crate::app) parameter_name: String,
    /// Leaf field edits relative to the newly-created parameter element.
    pub(in crate::app) initial_fields: Vec<ShaderParamInitialField>,
    /// Animated parameter children to append below the newly-created element.
    pub(in crate::app) animated_parameters: Vec<ShaderParamInitialAnimated>,
}

#[derive(Clone)]
pub(in crate::app) struct ShaderParamInitialField {
    pub(in crate::app) field: String,
    pub(in crate::app) input: String,
}

#[derive(Clone)]
pub(in crate::app) struct ShaderParamInitialAnimated {
    pub(in crate::app) output_type_index: i32,
    pub(in crate::app) initial_function_hex: String,
}

#[derive(Clone)]
pub(in crate::app) enum ModelVariantOp {
    Create {
        name: String,
        regions: Vec<ModelVariantRegionChoice>,
    },
    Update {
        variant_index: usize,
        regions: Vec<ModelVariantRegionChoice>,
    },
    Drop {
        variant_index: usize,
    },
}

#[derive(Clone)]
pub(in crate::app) struct ModelVariantRegionChoice {
    pub(in crate::app) region_name: String,
    pub(in crate::app) permutation_name: String,
}

/// What the user clicked in a block header this frame.
#[derive(Default)]
pub(in crate::app) struct BlockHeaderActions {
    pub(in crate::app) add: bool,
    pub(in crate::app) insert: bool,
    pub(in crate::app) duplicate: bool,
    pub(in crate::app) delete: bool,
    pub(in crate::app) delete_all: bool,
    pub(in crate::app) new_selection: Option<usize>,
    /// Right-click → "Copy element" on the selected element.
    pub(in crate::app) copy: bool,
    /// Right-click → "Copy entire block".
    pub(in crate::app) copy_block: bool,
    /// Right-click → "Copy block as TSV" (plaintext, Excel-friendly).
    pub(in crate::app) copy_block_tsv: bool,
    /// Right-click → "Paste TSV…" (open the import window for this block).
    pub(in crate::app) paste_tsv: bool,
    /// Right-click → "Paste" (insert clipboard element(s) after the selection).
    pub(in crate::app) paste: bool,
    /// Right-click → "Replace selected element" with the clipboard.
    pub(in crate::app) replace_element: bool,
    /// Right-click → "Replace entire block" with the clipboard.
    pub(in crate::app) replace_block: bool,
}

/// Emitted by a block header when the user picks "Paste TSV…" — the app hoists
/// it into `tsv_paste` and opens the import window.
pub(in crate::app) struct TsvPasteRequest {
    pub(in crate::app) block_path: String,
    pub(in crate::app) block_label: String,
    pub(in crate::app) element_count: usize,
}

/// The open TSV-import window: the user pastes tab-separated rows and applies
/// them to the target block's existing elements (per-cell, via `apply_field_edit`).
pub(in crate::app) struct TsvPasteState {
    pub(in crate::app) tag_key: String,
    pub(in crate::app) block_path: String,
    pub(in crate::app) block_label: String,
    pub(in crate::app) element_count: usize,
    pub(in crate::app) text: String,
    pub(in crate::app) status: Option<String>,
}

/// Per-frame capability bundle passed through schema-driven field rendering.
///
/// Renderers append deferred operations instead of mutating the borrowed tag.
/// Optional services deliberately make secondary/read-only views usable without
/// inventing source roots, definitions, or writable status.
pub(in crate::app) struct FieldEditContext<'a> {
    pub(in crate::app) view_scope: &'a str,
    pub(in crate::app) tag_key: &'a str,
    /// Group tag of the tag being rendered — gates block paste compatibility.
    pub(in crate::app) group_tag: u32,
    /// Root struct of the tag being rendered — used to resolve block-index
    /// fields whose target block is an ancestor (not a sibling). `None` in
    /// read-only/secondary contexts where ancestor resolution isn't needed.
    pub(in crate::app) root: Option<blam_tags::TagStruct<'a>>,
    pub(in crate::app) game: Option<&'a str>,
    pub(in crate::app) definitions_root: Option<&'a Path>,
    pub(in crate::app) names: Option<&'a TagNameIndex>,
    pub(in crate::app) tags_root: Option<&'a Path>,
    pub(in crate::app) status: Option<&'a mut String>,
    pub(in crate::app) editable: bool,
    pub(in crate::app) show_block_sizes: bool,
    pub(in crate::app) buffers: &'a mut HashMap<String, String>,
    pub(in crate::app) pending: &'a mut Vec<PendingFieldEdit>,
    pub(in crate::app) block_ops: &'a mut Vec<BlockOp>,
    pub(in crate::app) block_confirm: &'a mut Option<BlockConfirm>,
    /// Set when the user clicks "Open" on a tag-reference row.
    pub(in crate::app) open_request: &'a mut Option<OpenTagRequest>,
    /// Set when the user clicks a Play/Stop control in the sound-player panel;
    /// the app drains it after rendering to drive FMOD bank playback.
    pub(in crate::app) sound_play_request: &'a mut Option<super::audio::SoundAction>,
    /// Last sound-player status line (bank/resolve/playback result), for display.
    pub(in crate::app) sound_status: Option<&'a str>,
    /// Current playback volume (linear, 0.0..=1.0), for the sound-player slider.
    pub(in crate::app) sound_volume: f32,
    /// Set when the user extracts sound audio to disk (per-perm or whole-tag);
    /// the app drains it to decode + write the files.
    pub(in crate::app) sound_extract_request: &'a mut Option<super::sound_extract::ExtractRequest>,
    /// Selected localized sound language (`None` = default), for the player's
    /// language selector + `data_<lang>\` extraction routing.
    pub(in crate::app) sound_language: Option<&'a str>,
    /// Set when the user clicks "Import" on a geometry tag-reference row.
    pub(in crate::app) tool_import: &'a mut Option<ToolImportRequest>,
    /// Set when the user clicks "Reimport" on a bitmap tag.
    pub(in crate::app) bitmap_reimport: &'a mut Option<String>,
    /// Shader-specific deferred ops (add animated parameter + init).
    pub(in crate::app) shader_ops: &'a mut Vec<ShaderOp>,
    /// Shader-specific deferred ops (create parameter entry + set real value).
    pub(in crate::app) shader_param_ops: &'a mut Vec<ShaderParamOp>,
    /// H2EK-specific deferred ops (create classic shader parameters/animations).
    pub(in crate::app) h2_shader_param_ops: &'a mut Vec<H2ShaderParamOp>,
    /// Function byte-block edits emitted by inline function editors.
    pub(in crate::app) function_data_ops: &'a mut Vec<FunctionDataOp>,
    /// Model-preview variant edits queued from the render model tab.
    pub(in crate::app) model_variant_ops: &'a mut Vec<ModelVariantOp>,
    /// Set when the user clicks a color swatch on a value row; the caller hoists
    /// it into `self.color_popup` after rendering so the shared popup handler
    /// can show the picker and apply the edit.
    pub(in crate::app) color_request: &'a mut Option<MaterialColorPopup>,
    /// Set when the user clicks a function row; the caller hoists it into
    /// `self.function_popup` after rendering so the shared popup handler can
    /// show the graph editor and apply function-data edits.
    pub(in crate::app) function_request: &'a mut Option<FunctionPopup>,
    /// Documentation overlay (help/units + explanation blocks) for this tag's
    /// group, parsed from the JSON definition. Used to restore field tooltips
    /// and explanation rows that shipped tags strip from their layout.
    pub(in crate::app) docs: Option<&'a DefDocs>,
    /// Set when the user picks "Paste TSV…" on a block; the caller hoists it
    /// into `self.tsv_paste` to open the import window.
    pub(in crate::app) tsv_paste_request: &'a mut Option<TsvPasteRequest>,
    /// The current block clipboard (read), for gating "Paste" in block menus.
    pub(in crate::app) block_clipboard: Option<&'a BlockClipboard>,
    /// Set when the user clicks "Copy element"; the caller hoists it into
    /// `self.block_clipboard` after rendering.
    pub(in crate::app) block_clip_request: &'a mut Option<BlockClipboard>,
    /// Present only on the single frame a "Search fields" query changes. It
    /// forces every collapsible node's open-state once (matched nodes open /
    /// rest closed, or restored to defaults when the query is cleared), then
    /// later frames leave `None` so the user can expand/collapse freely again.
    pub(in crate::app) field_filter: Option<&'a FieldFilterAction>,
    /// Active reference-jump navigation. When set for this tag, its target
    /// field's ancestor blocks are force-opened and the field is glowed.
    pub(in crate::app) field_nav: Option<&'a FieldNav>,
}

impl FieldEditContext<'_> {
    pub(in crate::app) fn widget_id(&self, salt: impl std::hash::Hash) -> egui::Id {
        egui::Id::new(("field_edit", self.view_scope, self.tag_key, salt))
    }

    /// Decide the forced open-state for a collapsible node at `node_path`,
    /// whose normal default is `default_open`. `None` means "leave the node's
    /// stored state alone" (no filter applied this frame); `Some(open)` forces
    /// it this frame.
    pub(in crate::app) fn resolve_open(&self, node_path: &str, default_open: bool) -> Option<bool> {
        // A reference-jump forces every ancestor of its target field open so the
        // field can be scrolled into view. Takes precedence over the search filter.
        if let Some(nav) = self.field_nav {
            if nav.tag_key == self.tag_key
                && path_is_ancestor(
                    &strip_node_indices(node_path),
                    &strip_node_indices(&nav.field_path),
                )
            {
                return Some(true);
            }
        }
        match self.field_filter? {
            // Query cleared: snap every node back to its normal default.
            FieldFilterAction::RestoreDefaults => Some(default_open),
            FieldFilterAction::Apply(filter) => {
                let canon = strip_node_indices(node_path);
                // Every rendered container is on a match path (others are hidden
                // by `field_visible`), so expand it to reveal the match in
                // context. The implicit root group has no path — always open.
                if canon.is_empty() || filter.visible_paths.contains(&canon) {
                    Some(true)
                } else {
                    Some(false)
                }
            }
        }
    }

    /// Whether a "Search fields" filter is applied this frame — i.e. the editor
    /// is hiding non-matches. Used to also suppress injected section/explanation
    /// rows so no orphan headers remain.
    pub(in crate::app) fn is_active_filter(&self) -> bool {
        matches!(self.field_filter, Some(FieldFilterAction::Apply(_)))
    }

    /// Whether `path`'s field should render at all. While a query is active only
    /// matches, their ancestor containers, and name-matched containers' contents
    /// are shown; everything else is hidden. Always visible with no query.
    pub(in crate::app) fn field_visible(&self, path: &str) -> bool {
        match self.field_filter {
            Some(FieldFilterAction::Apply(filter)) => {
                filter.visible_paths.contains(&strip_node_indices(path))
            }
            _ => true,
        }
    }

    /// Whether the exact `indexed_path` field is the live reference-jump target
    /// and still within its glow window — used to pulse the landed-on field.
    pub(in crate::app) fn field_nav_glow(&self, indexed_path: &str, now: f64) -> bool {
        self.field_nav.is_some_and(|nav| {
            nav.tag_key == self.tag_key && nav.field_path == indexed_path && now < nav.glow_until
        })
    }
}

/// Whether `ancestor` is `target` itself or an ancestor of it, compared
/// segment-wise so `"custom references"` is an ancestor of
/// `"custom references/sounds"` but not of `"custom references extra"`. Both
/// paths must already be index-stripped (see [`strip_node_indices`]).
fn path_is_ancestor(ancestor: &str, target: &str) -> bool {
    if ancestor.is_empty() {
        return true;
    }
    target == ancestor
        || (target.len() > ancestor.len()
            && target.as_bytes()[ancestor.len()] == b'/'
            && target.starts_with(ancestor))
}

/// What a "Search fields" change should do to the editor's collapse state on
/// the frame it is applied.
pub(in crate::app) enum FieldFilterAction {
    /// Hide everything except matches and their ancestor containers; expand the
    /// containers that remain.
    Apply(FieldFilter),
    /// Re-expand every node to its normal default (query was cleared).
    RestoreDefaults,
}

/// Which collapsible nodes a "Search fields" query wants open. Paths are the
/// canonical field paths with element indices (`[3]`) stripped, so they're
/// independent of which block element happens to be selected.
pub(in crate::app) struct FieldFilter {
    /// Canonical paths of every field that should render while searching:
    /// matches, their ancestor containers, and the contents of name-matched
    /// containers. Fields absent from this set are hidden.
    pub(in crate::app) visible_paths: std::collections::HashSet<String>,
}

#[derive(Clone)]
pub(in crate::app) struct FieldDisplayMeta {
    pub(in crate::app) label: String,
    pub(in crate::app) unit: Option<String>,
    /// A `[min,max]` range/bounds hint (shown after the unit/type), e.g.
    /// `[0,+inf]`. Parsed out of the unit slot or the bare name.
    pub(in crate::app) range: Option<String>,
    pub(in crate::app) help: Option<String>,
    /// Tag groups declared by the JSON definition for tag_reference fields.
    /// The runtime blam-tags layout keeps only reference flags, so Baboon
    /// carries this through the docs overlay for display-only affordances.
    pub(in crate::app) tag_reference_allowed: Vec<u32>,
    pub(in crate::app) read_only: bool,
    pub(in crate::app) advanced: bool,
}
