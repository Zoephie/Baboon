use super::*;

pub(super) enum WorkerMessage {
    SourceLoaded {
        result: Result<LoadedSourceData, String>,
        recent_path: Option<PathBuf>,
    },
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
    // Non-fatal terminal log failure.
    TerminalLogError(String),
    // Terminal process finished.
    TerminalDone {
        run_id: u64,
    },
    // GitHub latest-release lookup finished.
    UpdateCheckFinished(Result<UpdateCheckResult, String>),
    // Background field-value search finished. Carries the source generation it
    // ran against so stale results (after a reload) can be discarded.
    FieldValueSearchFinished {
        generation: u64,
        query: String,
        result: Result<Vec<FieldValueMatch>, String>,
    },
    // Background field-value index build finished. `blobs` is (entry key,
    // lowercased searchable text) pairs; `generation` guards against staleness.
    FieldIndexBuilt {
        generation: u64,
        blobs: Vec<(String, String)>,
    },
    // Background reverse-dependency index build finished. `generation` guards
    // against staleness after a source reload.
    ReverseDependenciesBuilt {
        generation: u64,
        index: ReverseDependencyIndex,
    },
}

/// One tag whose field values matched a field-value search, with the first
/// matching `field path = value` to show as an annotation.
pub(super) struct FieldValueMatch {
    pub(super) entry: TagEntry,
    pub(super) label: String,
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
    pub(super) lines: Vec<TerminalLineEntry>,
    pub(super) history: Vec<String>,
    pub(super) history_cursor: Option<usize>,
    pub(super) refocus_input: bool,
    pub(super) running: bool,
    pub(super) running_id: Option<u64>,
    pub(super) next_run_id: u64,
    pub(super) running_command: Option<String>,
    pub(super) last_log_path: Option<PathBuf>,
    pub(super) process: Option<TerminalProcess>,
    pub(super) scroll_to_bottom: bool,
}

pub(super) struct TerminalLineEntry {
    pub(super) text: String,
    pub(super) severity: TerminalLineSeverity,
}

impl TerminalLineEntry {
    pub(super) fn new(text: String) -> Self {
        let severity = TerminalLineSeverity::classify(&text);
        Self { text, severity }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalLineSeverity {
    Normal,
    Warning,
    Error,
    Success,
    Summary,
}

impl TerminalLineSeverity {
    fn classify(line: &str) -> Self {
        let trimmed = line.trim_start();
        let lower = line.to_ascii_lowercase();
        if line.contains("-ERROR-")
            || lower.contains("[error]")
            || (trimmed.starts_with("[exit ")
                && !trimmed.starts_with("[exit 0]")
                && trimmed
                    .strip_prefix("[exit ")
                    .and_then(|rest| rest.strip_suffix(']'))
                    .and_then(|code| code.parse::<i32>().ok())
                    .is_some())
        {
            Self::Error
        } else if lower.contains("warning") {
            Self::Warning
        } else if trimmed.starts_with("[exit 0]") {
            Self::Success
        } else if trimmed.starts_with("===") {
            Self::Summary
        } else {
            Self::Normal
        }
    }
}

pub(super) struct TerminalProcess {
    pub(super) child: Arc<Mutex<Option<std::process::Child>>>,
    pub(super) stop_requested: Arc<AtomicBool>,
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
    FindReferences(String),
    ExploreReferences(String),
    RenameTag(String),
}

/// The "Rename / Move tag (fix references)" dialog. Shows the referrers that
/// will be rewritten (preview) and an editable destination path; applying moves
/// the file on disk and rewrites every referencing tag.
pub(super) struct RenameTagState {
    pub(super) key: String,
    pub(super) group_tag: u32,
    /// Current display path (forward slashes, with extension) — shown read-only.
    pub(super) old_display: String,
    /// File extension (kept fixed; the group can't change on rename).
    pub(super) extension: String,
    /// Editable destination: relative path, forward slashes, NO extension.
    pub(super) new_path_input: String,
    /// Display paths of tags that reference this one and will be updated.
    pub(super) referrers: Vec<String>,
    /// True when no reverse-dependency index was available to list referrers.
    pub(super) referrers_unavailable: bool,
}

/// Results of a tag query (find-references / unreferenced), shown in a floating
/// results window. Each entry is clickable to open the tag.
pub(super) struct TagQueryResults {
    pub(super) title: String,
    pub(super) entries: Vec<TagEntry>,
    /// Optional per-entry annotation (parallel to `entries`), e.g. the map id.
    /// Empty when there are no annotations.
    pub(super) annotations: Vec<String>,
    /// Optional explanatory note (e.g. when the reference index is unavailable).
    pub(super) note: Option<String>,
    /// For a "References to X" query: the referenced tag's `(group_tag, rel_path)`
    /// so a clicked row can jump to the exact referencing field. `None` for other
    /// query kinds (unreferenced, map-id, …), which only open the tag.
    pub(super) ref_target: Option<(u32, String)>,
}

/// One place a referrer tag points at the "References to X" target, shown in the
/// popup's per-referrer expander. `field_path` is the exact indexed path handed
/// to `navigate_to_field`; `label` is its human breadcrumb.
pub(super) struct RefOccurrence {
    pub(super) field_path: String,
    pub(super) label: String,
}

/// A reference-jump awaiting its referrer tag to finish loading. Once that tag
/// is the focused tab and parsed, the controller walks it for the exact field
/// referencing `(group_tag, rel_path)` and hands off to a [`FieldNav`].
#[derive(Clone)]
pub(super) struct PendingRefJump {
    pub(super) tag_key: String,
    pub(super) group_tag: u32,
    pub(super) rel_path: String,
}

/// Active "jump to a referencing field" navigation: force the target field's
/// ancestor blocks open and glow the field until `glow_until` (egui time,
/// seconds). Element selection along the path and the scroll target are set once
/// via egui temp-data when the nav is created.
pub(super) struct FieldNav {
    pub(super) tag_key: String,
    /// Exact indexed field path, e.g. `custom references[3]/sounds[1]/melee sound`.
    pub(super) field_path: String,
    pub(super) glow_until: f64,
}

/// Drag-and-drop payload carried when dragging a tag from the browser onto a
/// tag-reference cell. `input` is the ready-to-apply reference string
/// (`"fourcc:back\\slash\\path"`); `group_tag` lets a drop target validate it.
#[derive(Clone)]
pub(super) struct DraggedTagRef {
    pub(super) group_tag: u32,
    /// Foundation reference-cell form: `"fourcc:back\\slash\\path"` (no ext).
    pub(super) input: String,
    /// Shader bitmap-row form: forward-slash relative path, no extension.
    pub(super) rel_path: String,
    pub(super) label: String,
}

/// A one-shot "reveal in browser tree" request: force-open the folder nodes in
/// `ancestors` (root→parent labels) and scroll the entry `key` into view.
/// Consumed (taken) during the browser draw.
pub(super) struct RevealRequest {
    pub(super) key: String,
    pub(super) ancestors: Vec<String>,
}

/// Reference-graph navigator centered on one tag: who references it (parents)
/// and what it references (children). Navigating to a parent/child re-centers
/// and records back/forward history.
pub(super) struct ContentExplorer {
    pub(super) focus: TagEntry,
    pub(super) parents: Vec<TagEntry>,
    pub(super) children: Vec<TagEntry>,
    /// Substring filter applied to both the parents and children lists.
    pub(super) filter: String,
    /// True when no reverse-dependency index was available to build the view.
    pub(super) index_unavailable: bool,
    pub(super) back: Vec<TagEntry>,
    pub(super) forward: Vec<TagEntry>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BrowserMode {
    Folders,
    Groups,
}

/// Ordering of tags within a browser folder/group node.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BrowserSort {
    /// Filesystem / natural order (as built).
    Natural,
    /// By filename, A→Z.
    Name,
    /// By group (type), then filename.
    Type,
}

impl BrowserSort {
    pub(super) const ALL: [BrowserSort; 3] =
        [BrowserSort::Natural, BrowserSort::Name, BrowserSort::Type];

    pub(super) fn label(self) -> &'static str {
        match self {
            BrowserSort::Natural => "Natural",
            BrowserSort::Name => "Name",
            BrowserSort::Type => "Type",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HelpPanelTab {
    About,
    Doc,
    MapNames,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsTab {
    Startup,
    Browser,
    EditingKits,
    EditingKitAliases,
    Appearance,
    Tools,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct EditingKitShortcut {
    pub(super) label: &'static str,
    pub(super) game: &'static str,
    pub(super) fallback: &'static str,
}

pub(super) const EDITING_KIT_SHORTCUTS: [EditingKitShortcut; 7] = [
    EditingKitShortcut {
        label: "HCEEK",
        game: "haloce_mcc",
        fallback: "CE",
    },
    EditingKitShortcut {
        label: "H2EK",
        game: "halo2_mcc",
        fallback: "H2",
    },
    EditingKitShortcut {
        label: "H3EK",
        game: "halo3_mcc",
        fallback: "H3",
    },
    EditingKitShortcut {
        label: "H3ODSTEK",
        game: "halo3odst_mcc",
        fallback: "ODST",
    },
    EditingKitShortcut {
        label: "HREK",
        game: "haloreach_mcc",
        fallback: "R",
    },
    EditingKitShortcut {
        label: "H4EK",
        game: "halo4_mcc",
        fallback: "H4",
    },
    EditingKitShortcut {
        label: "H2AMPEK",
        game: "halo2amp_mcc",
        fallback: "H2A",
    },
];

/// What Baboon does with the previous session's open windows on startup.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SessionRestore {
    /// Ask which windows to reopen (shows the "Last Opened Windows" prompt).
    Ask,
    /// Silently reopen the last session.
    Always,
    /// Start fresh — never reopen and never ask.
    Never,
}

impl SessionRestore {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            SessionRestore::Ask => "ask",
            SessionRestore::Always => "always",
            SessionRestore::Never => "never",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value {
            "ask" => Some(SessionRestore::Ask),
            "always" => Some(SessionRestore::Always),
            "never" => Some(SessionRestore::Never),
            _ => None,
        }
    }
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
    pub(super) browser_sort: BrowserSort,
    pub(super) show_browser_prefixes: bool,
    pub(super) folders_before_tags: bool,
    pub(super) double_click_to_open_tags: bool,
    pub(super) session_restore: SessionRestore,
    pub(super) show_block_sizes: bool,
    pub(super) scroll_to_cycle_dropdowns: bool,
    pub(super) expert_mode: bool,
    pub(super) dark_mode: bool,
    pub(super) ui_scale: f32,
    pub(super) model_preview_size: f32,
    pub(super) blender_path: Option<PathBuf>,
    pub(super) editing_kit_paths: HashMap<String, PathBuf>,
    pub(super) ek_folder_aliases: Vec<EkFolderAlias>,
    pub(super) tool_commands_window_pos: Option<egui::Pos2>,
    pub(super) tool_commands_window_size: Option<Vec2>,
    pub(super) tool_commands_left_width: f32,
    pub(super) tool_commands_collapsed_categories: HashSet<String>,
    pub(super) recent_folders: Vec<PathBuf>,
    pub(super) custom_color_swatches: Vec<Option<[u8; 4]>>,
    pub(super) palette_last_dir: Option<PathBuf>,
}

pub(super) struct TagDocument {
    pub(super) tag: TagFile,
    pub(super) dirty: bool,
    pub(super) journal: EditJournal,
}

impl TagDocument {
    pub(super) fn clean(tag: TagFile) -> Self {
        Self {
            tag,
            dirty: false,
            journal: EditJournal::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum PendingCloseAction {
    CloseApp,
    CloseTab(String),
    CloseAllTabs,
    CloseAllButThis(String),
}

pub(super) struct DirtyTagEntry {
    pub(super) path: String,
    pub(super) tag_id: String,
    pub(super) checked: bool,
}

/// Foundation-style confirmation shown when a close action would discard
/// edited tags. `allow_app_close_once` is set only after the user confirms an
/// app exit; the next native close request is then allowed through instead of
/// being vetoed and prompting again.
pub(super) struct SaveChangesPrompt {
    pub(super) visible: bool,
    pub(super) dirty_tags: Vec<DirtyTagEntry>,
    pub(super) pending_action: PendingCloseAction,
    pub(super) error: Option<String>,
    pub(super) allow_app_close_once: bool,
}

impl Default for SaveChangesPrompt {
    fn default() -> Self {
        Self {
            visible: false,
            dirty_tags: Vec::new(),
            pending_action: PendingCloseAction::CloseApp,
            error: None,
            allow_app_close_once: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LastSessionSourceKind {
    SingleFile,
    LooseFolder,
    MonolithicCache,
}

impl LastSessionSourceKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            LastSessionSourceKind::SingleFile => "single_file",
            LastSessionSourceKind::LooseFolder => "loose_folder",
            LastSessionSourceKind::MonolithicCache => "monolithic_cache",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value {
            "single_file" => Some(Self::SingleFile),
            "loose_folder" => Some(Self::LooseFolder),
            "monolithic_cache" => Some(Self::MonolithicCache),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct LastSessionTag {
    pub(super) key: String,
    pub(super) label: String,
    pub(super) group_tag: u32,
    pub(super) path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(super) struct LastSessionState {
    pub(super) source_kind: LastSessionSourceKind,
    pub(super) source_path: PathBuf,
    pub(super) game: Option<String>,
    pub(super) tags: Vec<LastSessionTag>,
}

pub(super) struct LastOpenedWindowEntry {
    pub(super) tag: LastSessionTag,
    pub(super) checked: bool,
    pub(super) available: bool,
}

/// Launch-time restore prompt backed by `last_session.json`. OK first reloads
/// the saved source (loose folder, monolithic cache, or single file); once the
/// async source load completes, the queued tag keys are reopened through the
/// normal `select_entry` path.
pub(super) struct LastOpenedWindowsPrompt {
    pub(super) visible: bool,
    pub(super) source_kind: LastSessionSourceKind,
    pub(super) source_path: PathBuf,
    pub(super) source_available: bool,
    pub(super) entries: Vec<LastOpenedWindowEntry>,
    /// "Don't ask again": on OK, remember as Always; on Cancel, as Never.
    pub(super) dont_ask_again: bool,
}

impl LastOpenedWindowsPrompt {
    pub(super) fn from_session(session: LastSessionState) -> Option<Self> {
        let source_available = match session.source_kind {
            LastSessionSourceKind::SingleFile => session.source_path.is_file(),
            LastSessionSourceKind::LooseFolder => session.source_path.is_dir(),
            LastSessionSourceKind::MonolithicCache => {
                if session.source_path.is_dir() {
                    session.source_path.join("blob_index.dat").is_file()
                } else {
                    session.source_path.is_file()
                        && session
                            .source_path
                            .file_name()
                            .is_some_and(|name| name.eq_ignore_ascii_case("blob_index.dat"))
                }
            }
        };
        let entries = session
            .tags
            .into_iter()
            .map(|tag| {
                let tag_available = tag.path.as_ref().map(|path| path.exists()).unwrap_or(true);
                let available = source_available && tag_available;
                LastOpenedWindowEntry {
                    tag,
                    checked: available,
                    available,
                }
            })
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return None;
        }
        Some(Self {
            visible: true,
            source_kind: session.source_kind,
            source_path: session.source_path,
            source_available,
            entries,
            dont_ask_again: false,
        })
    }

    pub(super) fn checked_tags(&self) -> Vec<LastSessionTag> {
        self.entries
            .iter()
            .filter(|entry| entry.available && entry.checked)
            .map(|entry| entry.tag.clone())
            .collect()
    }
}

pub(super) struct PendingSessionRestore {
    pub(super) tags: Vec<LastSessionTag>,
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

#[derive(Clone)]
pub(super) enum H2ShaderParamOp {
    #[allow(dead_code)]
    EnsureParameter {
        parameters_block_path: String,
        parameter_name: String,
        parameter_type_index: i32,
    },
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
    /// When true, open the tag in a floating (torn-off) window instead of the
    /// docked tab rack. Set by Alt-clicking a reference's Open button.
    pub(super) float: bool,
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
    /// Right-click → "Copy block as TSV" (plaintext, Excel-friendly).
    pub(super) copy_block_tsv: bool,
    /// Right-click → "Paste TSV…" (open the import window for this block).
    pub(super) paste_tsv: bool,
    /// Right-click → "Paste" (insert clipboard element(s) after the selection).
    pub(super) paste: bool,
    /// Right-click → "Replace selected element" with the clipboard.
    pub(super) replace_element: bool,
    /// Right-click → "Replace entire block" with the clipboard.
    pub(super) replace_block: bool,
}

/// Emitted by a block header when the user picks "Paste TSV…" — the app hoists
/// it into `tsv_paste` and opens the import window.
pub(super) struct TsvPasteRequest {
    pub(super) block_path: String,
    pub(super) block_label: String,
    pub(super) element_count: usize,
}

/// The open TSV-import window: the user pastes tab-separated rows and applies
/// them to the target block's existing elements (per-cell, via `apply_field_edit`).
pub(super) struct TsvPasteState {
    pub(super) tag_key: String,
    pub(super) block_path: String,
    pub(super) block_label: String,
    pub(super) element_count: usize,
    pub(super) text: String,
    pub(super) status: Option<String>,
}

pub(super) struct FieldEditContext<'a> {
    pub(super) view_scope: &'a str,
    pub(super) tag_key: &'a str,
    /// Group tag of the tag being rendered — gates block paste compatibility.
    pub(super) group_tag: u32,
    /// Root struct of the tag being rendered — used to resolve block-index
    /// fields whose target block is an ancestor (not a sibling). `None` in
    /// read-only/secondary contexts where ancestor resolution isn't needed.
    pub(super) root: Option<blam_tags::TagStruct<'a>>,
    pub(super) game: Option<&'a str>,
    pub(super) definitions_root: Option<&'a Path>,
    pub(super) names: Option<&'a TagNameIndex>,
    pub(super) definition_group_name: Option<&'a str>,
    pub(super) tags_root: Option<&'a Path>,
    pub(super) status: Option<&'a mut String>,
    pub(super) editable: bool,
    pub(super) show_block_sizes: bool,
    pub(super) buffers: &'a mut HashMap<String, String>,
    pub(super) pending: &'a mut Vec<PendingFieldEdit>,
    pub(super) block_ops: &'a mut Vec<BlockOp>,
    pub(super) block_confirm: &'a mut Option<BlockConfirm>,
    /// Set when the user clicks "Open" on a tag-reference row.
    pub(super) open_request: &'a mut Option<OpenTagRequest>,
    /// Set when the user clicks a Play/Stop control in the sound-player panel;
    /// the app drains it after rendering to drive FMOD bank playback.
    pub(super) sound_play_request: &'a mut Option<super::audio::SoundAction>,
    /// Last sound-player status line (bank/resolve/playback result), for display.
    pub(super) sound_status: Option<&'a str>,
    /// Current playback volume (linear, 0.0..=1.0), for the sound-player slider.
    pub(super) sound_volume: f32,
    /// Set when the user extracts sound audio to disk (per-perm or whole-tag);
    /// the app drains it to decode + write the files.
    pub(super) sound_extract_request: &'a mut Option<super::sound_extract::ExtractRequest>,
    /// Selected localized sound language (`None` = default), for the player's
    /// language selector + `data_<lang>\` extraction routing.
    pub(super) sound_language: Option<&'a str>,
    /// Set when the user clicks "Import" on a geometry tag-reference row.
    pub(super) tool_import: &'a mut Option<ToolImportRequest>,
    /// Set when the user clicks "Reimport" on a bitmap tag.
    pub(super) bitmap_reimport: &'a mut Option<String>,
    /// Shader-specific deferred ops (add animated parameter + init).
    pub(super) shader_ops: &'a mut Vec<ShaderOp>,
    /// Shader-specific deferred ops (create parameter entry + set real value).
    pub(super) shader_param_ops: &'a mut Vec<ShaderParamOp>,
    /// H2EK-specific deferred ops (create classic shader parameters/animations).
    pub(super) h2_shader_param_ops: &'a mut Vec<H2ShaderParamOp>,
    /// Function byte-block edits emitted by inline function editors.
    pub(super) function_data_ops: &'a mut Vec<FunctionDataOp>,
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
    /// Documentation overlay (help/units + explanation blocks) for this tag's
    /// group, parsed from the JSON definition. Used to restore field tooltips
    /// and explanation rows that shipped tags strip from their layout.
    pub(super) docs: Option<&'a DefDocs>,
    /// Set when the user picks "Paste TSV…" on a block; the caller hoists it
    /// into `self.tsv_paste` to open the import window.
    pub(super) tsv_paste_request: &'a mut Option<TsvPasteRequest>,
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
    /// Active reference-jump navigation. When set for this tag, its target
    /// field's ancestor blocks are force-opened and the field is glowed.
    pub(super) field_nav: Option<&'a FieldNav>,
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
    pub(super) fn is_active_filter(&self) -> bool {
        matches!(self.field_filter, Some(FieldFilterAction::Apply(_)))
    }

    /// Whether `path`'s field should render at all. While a query is active only
    /// matches, their ancestor containers, and name-matched containers' contents
    /// are shown; everything else is hidden. Always visible with no query.
    pub(super) fn field_visible(&self, path: &str) -> bool {
        match self.field_filter {
            Some(FieldFilterAction::Apply(filter)) => {
                filter.visible_paths.contains(&strip_node_indices(path))
            }
            _ => true,
        }
    }

    /// Whether the exact `indexed_path` field is the live reference-jump target
    /// and still within its glow window — used to pulse the landed-on field.
    pub(super) fn field_nav_glow(&self, indexed_path: &str, now: f64) -> bool {
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
pub(super) enum FieldFilterAction {
    /// Hide everything except matches and their ancestor containers; expand the
    /// containers that remain.
    Apply(FieldFilter),
    /// Re-expand every node to its normal default (query was cleared).
    RestoreDefaults,
}

/// Which collapsible nodes a "Search fields" query wants open. Paths are the
/// canonical field paths with element indices (`[3]`) stripped, so they're
/// independent of which block element happens to be selected.
pub(super) struct FieldFilter {
    /// Canonical paths of every field that should render while searching:
    /// matches, their ancestor containers, and the contents of name-matched
    /// containers. Fields absent from this set are hidden.
    pub(super) visible_paths: std::collections::HashSet<String>,
}

#[derive(Clone)]
pub(super) struct FieldDisplayMeta {
    pub(super) label: String,
    pub(super) unit: Option<String>,
    /// A `[min,max]` range/bounds hint (shown after the unit/type), e.g.
    /// `[0,+inf]`. Parsed out of the unit slot or the bare name.
    pub(super) range: Option<String>,
    pub(super) help: Option<String>,
    /// Tag groups declared by the JSON definition for tag_reference fields.
    /// The runtime blam-tags layout keeps only reference flags, so Baboon
    /// carries this through the docs overlay for display-only affordances.
    pub(super) tag_reference_allowed: Vec<u32>,
    pub(super) read_only: bool,
    pub(super) advanced: bool,
}

impl Default for GuiPrefs {
    fn default() -> Self {
        Self {
            browser_mode: BrowserMode::Folders,
            browser_sort: BrowserSort::Natural,
            show_browser_prefixes: false,
            folders_before_tags: false,
            double_click_to_open_tags: false,
            show_block_sizes: false,
            scroll_to_cycle_dropdowns: true,
            expert_mode: false,
            dark_mode: false,
            ui_scale: DEFAULT_UI_SCALE,
            model_preview_size: DEFAULT_MODEL_PREVIEW_SIZE,
            blender_path: None,
            editing_kit_paths: HashMap::new(),
            session_restore: SessionRestore::Ask,
            ek_folder_aliases: Vec::new(),
            tool_commands_window_pos: None,
            tool_commands_window_size: None,
            tool_commands_left_width: DEFAULT_TOOL_COMMANDS_LEFT_WIDTH,
            tool_commands_collapsed_categories: HashSet::new(),
            recent_folders: Vec::new(),
            custom_color_swatches: vec![None; CUSTOM_COLOR_SWATCH_COUNT],
            palette_last_dir: None,
        }
    }
}

pub(super) const DEFAULT_UI_SCALE: f32 = 1.0;
pub(super) const MIN_UI_SCALE: f32 = 0.6;
pub(super) const MAX_UI_SCALE: f32 = 1.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_kit_shortcuts_include_expected_profiles() {
        let pairs: Vec<(&str, &str)> = EDITING_KIT_SHORTCUTS
            .iter()
            .map(|shortcut| (shortcut.label, shortcut.game))
            .collect();

        assert_eq!(
            pairs,
            vec![
                ("HCEEK", "haloce_mcc"),
                ("H2EK", "halo2_mcc"),
                ("H3EK", "halo3_mcc"),
                ("H3ODSTEK", "halo3odst_mcc"),
                ("HREK", "haloreach_mcc"),
                ("H4EK", "halo4_mcc"),
                ("H2AMPEK", "halo2amp_mcc"),
            ]
        );
    }
}

pub(super) const DEFAULT_MODEL_PREVIEW_SIZE: f32 = 1.0;
pub(super) const MIN_MODEL_PREVIEW_SIZE: f32 = 0.8;
pub(super) const MAX_MODEL_PREVIEW_SIZE: f32 = 2.6;

pub(super) const DEFAULT_TOOL_COMMANDS_WINDOW_SIZE: Vec2 = Vec2::new(800.0, 600.0);
pub(super) const MIN_TOOL_COMMANDS_WINDOW_SIZE: Vec2 = Vec2::new(600.0, 400.0);
pub(super) const DEFAULT_TOOL_COMMANDS_LEFT_WIDTH: f32 = 280.0;
pub(super) const MIN_TOOL_COMMANDS_LEFT_WIDTH: f32 = 200.0;
pub(super) const MAX_RECENT_FOLDERS: usize = 10;
pub(super) const CUSTOM_COLOR_SWATCH_COUNT: usize = 16;

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

/// Background fill behind the bitmap preview image. Helps judge alpha edges
/// against light/dark/saturated backdrops.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BitmapPreviewBg {
    DarkGray,
    Black,
    White,
    Magenta,
}

impl BitmapPreviewBg {
    pub(super) const ALL: [Self; 4] = [Self::DarkGray, Self::Black, Self::White, Self::Magenta];

    pub(super) fn color(self) -> egui::Color32 {
        match self {
            Self::DarkGray => egui::Color32::from_rgb(64, 64, 64),
            Self::Black => egui::Color32::BLACK,
            Self::White => egui::Color32::WHITE,
            Self::Magenta => egui::Color32::from_rgb(255, 0, 255),
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::DarkGray => "Dark gray",
            Self::Black => "Black",
            Self::White => "White",
            Self::Magenta => "Magenta",
        }
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
    /// Background fill behind the previewed image.
    pub(super) bg: BitmapPreviewBg,
    /// Selected image (sequence) index and mipmap level being previewed.
    pub(super) image_index: usize,
    pub(super) mip_index: usize,
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
            bg: BitmapPreviewBg::DarkGray,
            image_index: 0,
            mip_index: 0,
        }
    }
}

pub(super) struct BitmapPreviewData {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) image_count: usize,
    /// Mipmap level count of the currently-decoded image (≥ 1).
    pub(super) mip_count: usize,
    pub(super) format_name: String,
    pub(super) type_name: String,
    pub(super) rgba: Vec<u8>,
}

/// One differing leaf field between two compared tags (Tag Diff).
pub(super) struct TagFieldDiff {
    pub(super) path: String,
    pub(super) a: String,
    pub(super) b: String,
}

/// State for the "Compare Tags" window: tag A (fixed to the launch tag), the
/// chosen tag B, and the computed diff (once "Compare" is clicked).
pub(super) struct TagDiffState {
    pub(super) a_key: String,
    /// Open-tab key of tag B (when B is an open tag); `None` when B was picked
    /// from disk (then `results`/`b_display` are set directly).
    pub(super) b_key: Option<String>,
    /// Display label for tag B (open key or picked disk path).
    pub(super) b_display: Option<String>,
    pub(super) results: Option<TagDiffResults>,
}

pub(super) struct TagDiffResults {
    pub(super) diffs: Vec<TagFieldDiff>,
    /// True when the diff hit the cap and more differences exist.
    pub(super) truncated: bool,
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
    /// Case-insensitive substring filter on marker names (empty = show all).
    /// Only applied while `show_markers` is on.
    pub(super) marker_filter: String,
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
            marker_filter: String::new(),
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

#[derive(Clone, PartialEq, Eq)]
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
    /// Region name → resolved permutation (own perm or parent-inherited).
    pub(super) regions: HashMap<String, String>,
    /// Region names the variant's block LISTS at all — including ones listed with
    /// an empty permutation (which means "explicitly removed", e.g. spec-ops elite
    /// has no helmet). A region NOT in this set is simply uncustomised and falls
    /// back to its base permutation (e.g. major elite helmet → base), rather than
    /// being hidden. Distinguishes "removed" from "not customised".
    pub(super) listed_regions: std::collections::HashSet<String>,
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
