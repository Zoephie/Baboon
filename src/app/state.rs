use super::*;

pub(super) enum WorkerMessage {
    SourceLoaded(Result<LoadedSourceData, String>),
    TagLoaded {
        key: String,
        result: Result<TagFile, String>,
    },
    BitmapReimportFinished {
        key: String,
        result: Result<TagFile, String>,
    },
    ExportFinished(Result<String, String>),
    FolderRefactorProgress(FolderRefactorProgress),
    FolderRefactorFinished(Result<FolderRefactorFinished, String>),
    // Full recursive entry scan finished for a loose-folder source.
    AllEntriesScanned(Result<Vec<TagEntry>, String>),
    // One line of streamed terminal output.
    TerminalLine(String),
    // Terminal process finished.
    TerminalDone,
    // GitHub latest-release lookup finished.
    UpdateCheckFinished(Result<UpdateCheckResult, String>),
}

pub(super) struct FolderRefactorProgress {
    pub(super) label: String,
    pub(super) phase: String,
    pub(super) progress: Option<f32>,
}

pub(super) struct FolderRefactorFinished {
    pub(super) status: String,
    pub(super) lines: Vec<String>,
    pub(super) tree: TagTree,
    pub(super) all_entries: Vec<TagEntry>,
    pub(super) reverse_dependencies: Option<ReverseDependencyIndex>,
    pub(super) old_to_new_keys: HashMap<String, String>,
    pub(super) moved: bool,
}

pub(super) struct FolderRefactorUiState {
    pub(super) label: String,
    pub(super) phase: String,
    pub(super) progress: Option<f32>,
}

pub(super) struct UpdateCheckResult {
    pub(super) latest_tag: String,
    pub(super) release_url: String,
}

pub(super) struct TerminalState {
    pub(super) input: String,
    pub(super) lines: Vec<String>,
    pub(super) history: Vec<String>,
    pub(super) history_cursor: Option<usize>,
    pub(super) refocus_input: bool,
    pub(super) running: bool,
    pub(super) scroll_to_bottom: bool,
}

pub(super) enum BrowserAction {
    Select(String),
    CopyTagName(String),
    DumpJson(String),
    OpenInExplorer(String),
    DumpLoadedFolderJson(Vec<String>),
    DumpLooseFolderJson { rel_path: PathBuf, label: String },
    MoveLooseFolder { rel_path: PathBuf, label: String },
    CopyLooseFolder { rel_path: PathBuf, label: String },
    ExtractRaw(String),
    ExtractBitmap(String),
    ExtractBitmapFolder(Vec<String>),
    ExtractGeometry(String),
    ExtractImportInfo(String),
    ExtractAnimation(String),
    ExtractMaterialShaderSources(String),
    ExtractMaterialShaderSourceFolder(Vec<String>),
    ExtractHlslIncludeSource(String),
    ExtractHlslIncludeFolder(Vec<String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BrowserMode {
    Folders,
    Groups,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HelpPanelTab {
    About,
    Doc,
    MapNames,
}

/// Memoized search results for the tag browser.
///
/// Filtering the full tag set (100k+ entries) and lowercasing each name is far
/// too expensive to redo every frame while the user types or scrolls. This
/// caches a *pruned* tree containing only the matching tags (in folder- or
/// group-hierarchy form) and only rebuilds it when the query, the source
/// generation, the entry universe (`all_entries` vs `entries`), or the browser
/// mode actually changes — see [`FilterCache::refresh`].
///
/// The pruned tree is rendered with folders collapsed, so the user drills down
/// the same way as the unfiltered tree; collapsed headers don't build their
/// children, which keeps per-frame cost bounded to what's actually expanded.
#[derive(Default)]
pub(super) struct FilterCache {
    /// `source_generation` the cached tree was built for.
    generation: u64,
    /// The (trimmed) query string the tree was built for.
    query: String,
    /// Whether matches came from `all_entries` (true) or `entries` (false).
    used_all: bool,
    /// Whether the cached tree is grouped by tag group (true) or by folder.
    groups: bool,
    /// The matching entries (cloned subset of the source), referenced by index
    /// from [`tree`]. Kept owned so rendering needs no borrow of the source.
    pub(super) entries: Vec<TagEntry>,
    /// Pruned hierarchy over [`entries`] — folder tree or group tree per mode.
    pub(super) tree: TagTree,
}

impl FilterCache {
    /// Rebuild the pruned match tree if anything it depends on changed;
    /// otherwise reuse the cached tree.
    pub(super) fn refresh(
        &mut self,
        generation: u64,
        query: &str,
        entries: &[TagEntry],
        used_all: bool,
        groups: bool,
    ) {
        if self.generation == generation
            && self.query == query
            && self.used_all == used_all
            && self.groups == groups
        {
            return;
        }
        self.generation = generation;
        self.query = query.to_owned();
        self.used_all = used_all;
        self.groups = groups;
        self.entries = compute_filter_matches(entries, query)
            .into_iter()
            .map(|index| entries[index].clone())
            .collect();
        self.tree = if groups {
            crate::source::build_group_tree(&self.entries)
        } else {
            crate::source::build_tree(&self.entries)
        };
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct GuiPrefs {
    pub(super) browser_mode: BrowserMode,
    pub(super) show_browser_prefixes: bool,
    pub(super) double_click_to_open_tags: bool,
    pub(super) show_block_sizes: bool,
    pub(super) expert_mode: bool,
    pub(super) dark_mode: bool,
    pub(super) ui_scale: f32,
    pub(super) model_preview_size: f32,
    pub(super) blender_path: Option<PathBuf>,
}

pub(super) struct TagDocument {
    pub(super) tag: TagFile,
    pub(super) dirty: bool,
}

impl TagDocument {
    pub(super) fn clean(tag: TagFile) -> Self {
        Self { tag, dirty: false }
    }
}

#[derive(Clone, Debug)]
pub(super) struct NewTagGroup {
    pub(super) group_tag: u32,
    pub(super) name: String,
    pub(super) schema_path: PathBuf,
    pub(super) extension: String,
}

#[derive(Clone, Debug)]
pub(super) struct NewTagDialog {
    pub(super) game: String,
    pub(super) rel_path: String,
    pub(super) output_path: Option<PathBuf>,
    pub(super) groups: Vec<NewTagGroup>,
    pub(super) selected_group: usize,
    pub(super) error: Option<String>,
}

impl Default for NewTagDialog {
    fn default() -> Self {
        Self {
            game: "halo3_mcc".to_owned(),
            rel_path: String::new(),
            output_path: None,
            groups: Vec::new(),
            selected_group: 0,
            error: None,
        }
    }
}

#[derive(Clone)]
pub(super) struct PendingFieldEdit {
    pub(super) path: String,
    pub(super) input: String,
}

#[derive(Clone)]
pub(super) struct FunctionDataOp {
    pub(super) block_path: String,
    pub(super) data: Vec<u8>,
}

/// A deferred structural edit to a block (add/insert/duplicate/delete),
/// applied to the tag after the immutable render borrow ends.
#[derive(Clone)]
pub(super) enum BlockOpKind {
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
pub(super) struct BlockOp {
    pub(super) path: String,
    pub(super) kind: BlockOpKind,
}

/// A copied block element, held on the app so it can be pasted into a block of
/// the same shape in another open tag. `group_tag` + `block_path` gate which
/// blocks accept the paste (same group, same block); the library re-validates
/// element compatibility before inserting.
#[derive(Clone)]
pub(super) struct BlockClipboard {
    pub(super) group_tag: u32,
    pub(super) block_path: String,
    /// Human label for the menu, e.g. "initial permutation".
    pub(super) label: String,
    /// One element (Copy element) or every element (Copy entire block).
    pub(super) elements: Vec<blam_tags::TagBlockElement>,
}

/// A pending destructive block op awaiting user confirmation. Lives on the
/// app (persists across frames) and is shown as a modal.
pub(super) struct BlockConfirm {
    pub(super) tag_key: String,
    pub(super) path: String,
    pub(super) kind: BlockOpKind,
    pub(super) message: String,
    /// Label for the confirm button (e.g. "Delete", "Replace").
    pub(super) confirm_label: String,
}

/// A request to open a referenced tag in a new tab (from an "Open" button on
/// a tag-reference row). Resolved against the loose-folder tags root.
#[derive(Clone)]
pub(super) struct OpenTagRequest {
    pub(super) group_tag: u32,
    pub(super) rel_path: String,
}

/// A request to (re)import a geometry tag via `tool` (from the Import button on
/// a render/collision/physics-model or animation-graph reference).
#[derive(Clone)]
pub(super) struct ToolImportRequest {
    /// `tool` verb: "render" / "collision" / "physics" /
    /// "model-animations-uncompressed".
    pub(super) verb: &'static str,
    /// Source directory argument, e.g. `objects\characters\masterchief`.
    pub(super) source_dir: String,
}

/// A deferred shader mutation: append one `animated parameters[]` element to
/// the given block path, then initialise its `type` and `function/data`
/// fields. Applied after the frame's draw pass, like `BlockOp`, but in its
/// own pass so the add + field init can be done atomically.
#[derive(Clone)]
pub(super) struct ShaderOp {
    /// Absolute path to the `animated parameters` block, e.g.
    /// `render_method/parameters[2]/animated parameters`.
    pub(super) animated_block_path: String,
    /// Output channel index (`RenderMethodAnimatedParameterType as i32`).
    pub(super) output_type_index: i32,
    /// Hex-encoded initial `mapping_function` blob for `function/data`.
    pub(super) initial_function_hex: String,
}

/// A deferred shader mutation: create a new `parameters[]` element, set its
/// `parameter name`, then initialise one or more leaf fields. Used when the
/// user edits a shader parameter that has no existing instance in the tag.
#[derive(Clone)]
pub(super) struct ShaderParamOp {
    /// Absolute path to the `parameters` block, e.g. `render_method/parameters`.
    pub(super) parameters_block_path: String,
    /// The parameter name to write into the new element's `parameter name`.
    pub(super) parameter_name: String,
    /// Leaf field edits relative to the newly-created parameter element.
    pub(super) initial_fields: Vec<ShaderParamInitialField>,
    /// Animated parameter children to append below the newly-created element.
    pub(super) animated_parameters: Vec<ShaderParamInitialAnimated>,
}

#[derive(Clone)]
pub(super) struct ShaderParamInitialField {
    pub(super) field: String,
    pub(super) input: String,
}

#[derive(Clone)]
pub(super) struct ShaderParamInitialAnimated {
    pub(super) output_type_index: i32,
    pub(super) initial_function_hex: String,
}

#[derive(Clone)]
pub(super) enum ModelVariantOp {
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
pub(super) struct ModelVariantRegionChoice {
    pub(super) region_name: String,
    pub(super) permutation_name: String,
}

/// What the user clicked in a block header this frame.
#[derive(Default)]
pub(super) struct BlockHeaderActions {
    pub(super) add: bool,
    pub(super) insert: bool,
    pub(super) duplicate: bool,
    pub(super) delete: bool,
    pub(super) delete_all: bool,
    pub(super) new_selection: Option<usize>,
    /// Right-click → "Copy element" on the selected element.
    pub(super) copy: bool,
    /// Right-click → "Copy entire block".
    pub(super) copy_block: bool,
    /// Right-click → "Paste" (insert clipboard element(s) after the selection).
    pub(super) paste: bool,
    /// Right-click → "Replace selected element" with the clipboard.
    pub(super) replace_element: bool,
    /// Right-click → "Replace entire block" with the clipboard.
    pub(super) replace_block: bool,
}

pub(super) struct FieldEditContext<'a> {
    pub(super) view_scope: &'a str,
    pub(super) tag_key: &'a str,
    /// Group tag of the tag being rendered — gates block paste compatibility.
    pub(super) group_tag: u32,
    pub(super) tags_root: Option<&'a Path>,
    pub(super) editable: bool,
    pub(super) show_block_sizes: bool,
    pub(super) buffers: &'a mut HashMap<String, String>,
    pub(super) pending: &'a mut Vec<PendingFieldEdit>,
    pub(super) block_ops: &'a mut Vec<BlockOp>,
    pub(super) block_confirm: &'a mut Option<BlockConfirm>,
    /// Set when the user clicks "Open" on a tag-reference row.
    pub(super) open_request: &'a mut Option<OpenTagRequest>,
    /// Set when the user clicks "Import" on a geometry tag-reference row.
    pub(super) tool_import: &'a mut Option<ToolImportRequest>,
    /// Set when the user clicks "Reimport" on a bitmap tag.
    pub(super) bitmap_reimport: &'a mut Option<String>,
    /// Shader-specific deferred ops (add animated parameter + init).
    pub(super) shader_ops: &'a mut Vec<ShaderOp>,
    /// Shader-specific deferred ops (create parameter entry + set real value).
    pub(super) shader_param_ops: &'a mut Vec<ShaderParamOp>,
    /// Model-preview variant edits queued from the render model tab.
    pub(super) model_variant_ops: &'a mut Vec<ModelVariantOp>,
    /// Set when the user clicks a color swatch on a value row; the caller hoists
    /// it into `self.color_popup` after rendering so the shared popup handler
    /// can show the picker and apply the edit.
    pub(super) color_request: &'a mut Option<MaterialColorPopup>,
    /// Set when the user clicks a function row; the caller hoists it into
    /// `self.function_popup` after rendering so the shared popup handler can
    /// show the graph editor and apply function-data edits.
    pub(super) function_request: &'a mut Option<FunctionPopup>,
    /// The current block clipboard (read), for gating "Paste" in block menus.
    pub(super) block_clipboard: Option<&'a BlockClipboard>,
    /// Set when the user clicks "Copy element"; the caller hoists it into
    /// `self.block_clipboard` after rendering.
    pub(super) block_clip_request: &'a mut Option<BlockClipboard>,
    /// Present only on the single frame a "Search fields" query changes. It
    /// forces every collapsible node's open-state once (matched nodes open /
    /// rest closed, or restored to defaults when the query is cleared), then
    /// later frames leave `None` so the user can expand/collapse freely again.
    pub(super) field_filter: Option<&'a FieldFilterAction>,
}

impl FieldEditContext<'_> {
    pub(super) fn widget_id(&self, salt: impl std::hash::Hash) -> egui::Id {
        egui::Id::new(("field_edit", self.view_scope, self.tag_key, salt))
    }

    /// Decide the forced open-state for a collapsible node at `node_path`,
    /// whose normal default is `default_open`. `None` means "leave the node's
    /// stored state alone" (no filter applied this frame); `Some(open)` forces
    /// it this frame.
    pub(super) fn resolve_open(&self, node_path: &str, default_open: bool) -> Option<bool> {
        match self.field_filter? {
            // Query cleared: snap every node back to its normal default.
            FieldFilterAction::RestoreDefaults => Some(default_open),
            FieldFilterAction::Apply(filter) => {
                let canon = strip_node_indices(node_path);
                // The implicit root group has no path — always keep it visible
                // so the matched nodes inside it can be reached.
                Some(canon.is_empty() || filter.open_paths.contains(&canon))
            }
        }
    }
}

/// What a "Search fields" change should do to the editor's collapse state on
/// the frame it is applied.
pub(super) enum FieldFilterAction {
    /// Collapse to the matched nodes (+ ancestors); everything else closed.
    Apply(FieldFilter),
    /// Re-expand every node to its normal default (query was cleared).
    RestoreDefaults,
}

/// Which collapsible nodes a "Search fields" query wants open. Paths are the
/// canonical field paths with element indices (`[3]`) stripped, so they're
/// independent of which block element happens to be selected.
pub(super) struct FieldFilter {
    pub(super) open_paths: std::collections::HashSet<String>,
}

#[derive(Clone)]
pub(super) struct FieldDisplayMeta {
    pub(super) label: String,
    pub(super) unit: Option<String>,
    pub(super) help: Option<String>,
    pub(super) read_only: bool,
    pub(super) advanced: bool,
}

impl Default for GuiPrefs {
    fn default() -> Self {
        Self {
            browser_mode: BrowserMode::Folders,
            show_browser_prefixes: false,
            double_click_to_open_tags: false,
            show_block_sizes: false,
            expert_mode: false,
            dark_mode: false,
            ui_scale: DEFAULT_UI_SCALE,
            model_preview_size: DEFAULT_MODEL_PREVIEW_SIZE,
            blender_path: None,
        }
    }
}

pub(super) const DEFAULT_UI_SCALE: f32 = 1.0;
pub(super) const MIN_UI_SCALE: f32 = 0.6;
pub(super) const MAX_UI_SCALE: f32 = 1.5;

pub(super) const DEFAULT_MODEL_PREVIEW_SIZE: f32 = 1.0;
pub(super) const MIN_MODEL_PREVIEW_SIZE: f32 = 0.8;
pub(super) const MAX_MODEL_PREVIEW_SIZE: f32 = 2.6;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BitmapPanelTab {
    Fields,
    Texture,
}

impl Default for BitmapPanelTab {
    fn default() -> Self {
        Self::Fields
    }
}

pub(super) struct BitmapPreviewState {
    pub(super) active_tab: BitmapPanelTab,
    pub(super) show_red: bool,
    pub(super) show_green: bool,
    pub(super) show_blue: bool,
    pub(super) show_alpha: bool,
    pub(super) decoded: Option<Result<BitmapPreviewData, String>>,
    pub(super) texture: Option<egui::TextureHandle>,
    pub(super) texture_dirty: bool,
    pub(super) zoom: f32,
    /// Pan offset of the image center relative to the canvas center, in
    /// screen pixels. Updated by drag-to-pan and zoom-to-cursor.
    pub(super) pan: Vec2,
    /// False until zoom is initialized to fit the image on first decode.
    pub(super) zoom_initialized: bool,
}

impl Default for BitmapPreviewState {
    fn default() -> Self {
        Self {
            active_tab: BitmapPanelTab::Fields,
            show_red: true,
            show_green: true,
            show_blue: true,
            show_alpha: true,
            decoded: None,
            texture: None,
            texture_dirty: true,
            zoom: 1.0,
            pan: Vec2::ZERO,
            zoom_initialized: false,
        }
    }
}

pub(super) struct BitmapPreviewData {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) image_count: usize,
    pub(super) format_name: String,
    pub(super) type_name: String,
    pub(super) rgba: Vec<u8>,
}

pub(super) struct ModelPreviewState {
    pub(super) loaded_key: Option<String>,
    pub(super) render_model_path: Option<String>,
    pub(super) data: Option<Result<ModelPreviewData, String>>,
    pub(super) active_tab: ModelTagPanelTab,
    pub(super) new_variant_name: String,
    pub(super) selected_variant: Option<usize>,
    pub(super) region_selections: HashMap<String, ModelRegionSelection>,
    pub(super) projected_triangles: Vec<ModelProjectedTriangle>,
    pub(super) show_markers: bool,
    pub(super) show_wireframe: bool,
    pub(super) show_backfaces: bool,
    pub(super) scale: f32,
    pub(super) yaw: f32,
    pub(super) pitch: f32,
    pub(super) pan: Vec2,
}

impl Default for ModelPreviewState {
    fn default() -> Self {
        Self {
            loaded_key: None,
            render_model_path: None,
            data: None,
            active_tab: ModelTagPanelTab::Fields,
            new_variant_name: String::new(),
            selected_variant: None,
            region_selections: HashMap::new(),
            projected_triangles: Vec::new(),
            show_markers: false,
            show_wireframe: false,
            show_backfaces: false,
            scale: 1.0,
            yaw: -0.45,
            pitch: 0.25,
            pan: Vec2::ZERO,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ModelTagPanelTab {
    Fields,
    RenderModel,
}

#[derive(Clone)]
pub(super) struct ModelRegionSelection {
    pub(super) enabled: bool,
    pub(super) permutation: String,
}

#[derive(Clone)]
pub(super) struct ModelPreviewData {
    pub(super) source_key: String,
    pub(super) render_model_path: String,
    pub(super) preview: RenderModelPreview,
    pub(super) draw_triangles: Vec<ModelSourceTriangle>,
    pub(super) variants: Vec<ModelVariantPreview>,
}

#[derive(Clone)]
pub(super) struct ModelVariantPreview {
    pub(super) name: String,
    pub(super) regions: HashMap<String, String>,
    pub(super) has_explicit_regions: bool,
}

#[derive(Clone, Copy)]
pub(super) struct ModelSourceTriangle {
    pub(super) batch_index: usize,
    pub(super) positions: [[f32; 3]; 3],
    pub(super) normals: [[f32; 3]; 3],
    pub(super) fill: Color32,
}

pub(super) struct ModelProjectedTriangle {
    pub(super) points: [egui::Pos2; 3],
    pub(super) depth: f32,
    pub(super) fills: [Color32; 3],
}
