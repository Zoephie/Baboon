//! browser application state.
//! It owns state types only; drawing them and acting on them belong to the feature's other modules.

use super::*;

pub(in crate::app) enum BrowserAction {
    /// Open a docked browser pane rooted at one real folder.
    OpenFolderBrowser {
        rel_path: PathBuf,
        label: String,
        /// Explicit new-tab requests never reuse an existing folder pane.
        open_in_new_tab: bool,
    },
    ToggleFolderFavorite(PathBuf),
    Select(String),
    ToggleFavorite(String),
    CopyTagName(String),
    CopyFolderPath(PathBuf),
    DumpJson(String),
    OpenInExplorer(String),
    DumpLoadedFolderJson(Vec<String>),
    DumpLooseFolderJson {
        rel_path: PathBuf,
        label: String,
    },
    /// Rename this loose folder in place and rewrite every reference to the
    /// tags beneath it.
    RenameLooseFolder {
        rel_path: PathBuf,
        label: String,
    },
    MoveLooseFolder {
        rel_path: PathBuf,
        label: String,
    },
    CopyLooseFolder {
        rel_path: PathBuf,
        label: String,
    },
    /// Open Import Tags aimed at this loose folder, so a tag or a whole tree
    /// from another game's kit lands here. Carries no label: the destination is
    /// the folder's path, and the source's own name supplies the leaf.
    ImportTagsIntoLooseFolder {
        rel_path: PathBuf,
    },
    /// Show this loose folder in File Explorer. Loose only: a container folder
    /// is a path inside a pak, with no directory to open.
    OpenLooseFolderInExplorer {
        rel_path: PathBuf,
    },
    /// Convert this monolithic-cache folder into an open editing kit.
    ///
    /// Carries the folder's path as a tag-name prefix rather than a list of
    /// keys: the run follows references out of the folder, so the set it ends up
    /// converting is not knowable from the browser — and filtering by prefix is
    /// the worker's job either way.
    ImportCacheFolderIntoKit {
        prefix: String,
    },
    /// One cache tag, landing somewhere the user picks rather than at its
    /// own path.
    ImportCacheTagIntoKit {
        key: String,
    },
    ExtractRaw(String),
    ExtractBitmap(String),
    ExtractBitmapFolder(Vec<String>),
    /// Recover a bitmap tag's source image (its color plate).
    ExtractBitmapSource(String),
    /// Recover the source images of a folder's bitmap tags, keeping their
    /// tag folders.
    ExtractBitmapSourceFolder(Vec<String>),
    /// Extract the audio represented by one or more `.sound` tags without
    /// opening them. `all_languages = false` uses the shared audio language;
    /// true exports every language available to each tag/source.
    ExtractSound {
        keys: Vec<String>,
        all_languages: bool,
    },
    /// Recursively discover a loose folder's tags so its bulk-extraction
    /// counts and actions cover unopened subfolders as well.
    LoadFolderExtractables {
        rel_path: PathBuf,
        label: String,
    },
    ExtractGeometry(String),
    ExtractImportInfo(String),
    ExtractAnimation(String),
    ExtractMaterialShaderSources(String),
    ExtractMaterialShaderSourceFolder(Vec<String>),
    ExtractHlslIncludeSource(String),
    ExtractHlslIncludeFolder(Vec<String>),
    /// Rebuild a loose geometry/animation tag from its editing-kit data files
    /// using the same tool command offered beside compatible tag references.
    ReimportGeometry(String),
    /// Write every shipped tag beneath one container folder to a chosen folder,
    /// laid out like an editing kit. The narrow-scope twin of File → Extract All
    /// Tags to Folder, and it shares that action's worker, progress and cancel.
    /// `label` is the folder's display path, carried for the confirmation only.
    ExtractContainerFolderTags {
        label: String,
        keys: Vec<String>,
    },
    /// Write a scenario's `source files` block out as a folder of `.hsc` files.
    ExtractScenarioScripts(String),
    /// Replace a scenario's `source files` block from a folder of `.hsc` files.
    /// Leaves the document modified rather than saving it.
    ImportScenarioScripts(String),
    FindReferences(String),
    ExploreReferences(String),
    /// Write every tag this one pulls in, recursively, to a text file the user
    /// picks. Outbound only — the inbound half is [`BrowserAction::FindReferences`].
    DumpReferences(String),
    /// Open this scenario in the kit's own tools. Same two launches the tag
    /// pane's header offers, reachable without opening the tag first.
    LaunchScenarioInSapien(String),
    LaunchScenarioInTagTest(String),
    RenameTag(String),
    DuplicateTag(String),
    DeleteTag(String),
    MoveTag(String),
    /// Import a tag file into a Campaign Evolved container, at `folder_rel`
    /// (`None` = root).
    ImportTagInFolder {
        folder_rel: Option<String>,
    },
    /// Create a new Campaign Evolved tag at `folder_rel` (`None` = root).
    NewTagInFolder {
        folder_rel: Option<String>,
    },
    /// Make a folder inside `parent_rel` (`None` = container root). Nothing is
    /// written to any pak: a folder only reaches the container's directory
    /// index once a tag lands in it.
    NewContainerFolder {
        parent_rel: Option<String>,
    },
    /// Rename a pending folder — one no tag has landed in yet, so this moves
    /// nothing on disk.
    RenameContainerFolder {
        rel: String,
    },
    /// Retire a pending folder. Offered only for one drawn as empty.
    DeleteContainerFolder {
        rel: String,
    },
}

/// Per-tab state for a docked browser rooted at one folder.
pub(in crate::app) struct FolderBrowserState {
    pub(in crate::app) rel_path: PathBuf,
    pub(in crate::app) label: String,
    pub(in crate::app) filter: String,
    pub(in crate::app) focus_search: bool,
    pub(in crate::app) mode: BrowserMode,
    pub(in crate::app) sort: BrowserSort,
    pub(in crate::app) cached_generation: u64,
    pub(in crate::app) cached_source_len: usize,
    pub(in crate::app) tree: TagTree,
    pub(in crate::app) group_tree: TagTree,
    /// The generation and entry count `group_tree` was built from.
    pub(in crate::app) group_tree_for: Option<(u64, usize)>,
    pub(in crate::app) filter_cache: FilterCache,
    pub(in crate::app) date_cache: FolderDateCache,
    pub(in crate::app) table_layout: FolderTableLayout,
    pub(in crate::app) search_scope: BrowserSearchScope,
    pub(in crate::app) assets_view: bool,
    pub(in crate::app) asset_bitmaps: bool,
    pub(in crate::app) asset_models: bool,
    pub(in crate::app) asset_cell_size: f32,
}

pub(in crate::app) const FOLDER_PANE_PREFIX: &str = "\u{1f}folder:";

pub(in crate::app) fn folder_pane_key(rel_path: &Path) -> String {
    format!(
        "{FOLDER_PANE_PREFIX}{}",
        rel_path.to_string_lossy().replace('\\', "/")
    )
}

pub(in crate::app) fn is_folder_pane_key(key: &str) -> bool {
    key.starts_with(FOLDER_PANE_PREFIX)
}

/// The New/Rename Folder dialog for a container source.
///
/// A folder here is a workspace-level intention, not a container edit: a pak's
/// directory index cannot encode a directory with no file beneath it, so this
/// only ever moves entries in the kit's pending-folder set.
pub(in crate::app) struct ContainerFolderDialog {
    /// Workspace this was raised from. Resolved on apply, because a modeless
    /// dialog outlives the frame that opened it and the user can focus another
    /// workspace in between.
    pub(in crate::app) kit: KitId,
    /// Parent folder, `None` for the container root.
    pub(in crate::app) parent_rel: Option<String>,
    /// Full path of the folder being renamed; `None` when creating one.
    pub(in crate::app) renaming: Option<String>,
    pub(in crate::app) name_input: String,
    pub(in crate::app) focus_input: bool,
    /// Validation failure from the last apply, shown beside the field.
    pub(in crate::app) error: Option<String>,
}

/// The Rename Folder dialog for a loose tags folder.
///
/// What it will change is counted when the dialog opens, so the user confirms
/// against numbers rather than a promise. The apply is the folder move job with
/// the folder's own parent as the destination and the new name as its leaf.
pub(in crate::app) struct LooseFolderRenameState {
    /// Workspace this was raised from; resolved again on apply.
    pub(in crate::app) kit: KitId,
    /// The folder, relative to the tags root.
    pub(in crate::app) rel_path: PathBuf,
    /// Its parent, forward slashes, empty for the root. Shown read-only.
    pub(in crate::app) parent_display: String,
    /// Its current name.
    pub(in crate::app) old_name: String,
    pub(in crate::app) name_input: String,
    pub(in crate::app) focus_input: bool,
    /// Validation failure from the last apply, shown beside the field.
    pub(in crate::app) error: Option<String>,
    /// Tags beneath the folder, nested folders included. Each one's path changes.
    pub(in crate::app) tag_count: usize,
    /// Tags outside the folder that reference one inside it, as display paths.
    /// `None` when no dependency index is loaded: the job still finds and
    /// rewrites them, it just cannot be counted up front.
    pub(in crate::app) outside_referrers: Option<Vec<String>>,
}

/// The name dialog's product-level operation. Storage details such as whether
/// an entry comes from a container remain separate; workflow decisions must not
/// be inferred from a pair of booleans that happen to describe the storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum TagNameOperation {
    Rename,
    SaveAsOverlay,
    Duplicate,
}

/// The "Rename / Move tag (fix references)" dialog. Shows the referrers that
/// will be rewritten (preview) and an editable destination path; applying moves
/// the file on disk and rewrites every referencing tag.
pub(in crate::app) struct RenameTagState {
    /// Workspace this was raised from. The confirm applies against the active
    /// kit, and a modeless dialog outlives the frame that opened it, so the
    /// user can focus another game in between; resolving this first is what
    /// keeps the action on the workspace it was started in.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) key: String,
    /// Current display path (forward slashes, with extension) — shown read-only.
    pub(in crate::app) old_display: String,
    /// File extension (kept fixed; the group can't change on rename).
    pub(in crate::app) extension: String,
    /// The explicit operation being performed by this dialog.
    pub(in crate::app) operation: TagNameOperation,
    /// Editable destination: relative path, forward slashes, NO extension.
    pub(in crate::app) new_path_input: String,
    /// Fixed display parent for Duplicate; Rename and Save As retain their
    /// existing path presentation rules.
    pub(in crate::app) fixed_parent: String,
    /// The first draw of a Duplicate dialog focuses and selects this input.
    pub(in crate::app) focus_input: bool,
    /// Display paths of tags that reference this one and will be updated.
    pub(in crate::app) referrers: Vec<String>,
    /// True when no reverse-dependency index was available to list referrers.
    pub(in crate::app) referrers_unavailable: bool,
    /// Source is a Campaign Evolved container: apply writes an override
    /// container instead of moving a loose file + rewriting references.
    pub(in crate::app) is_container: bool,
    /// Source is a brand-new (never-saved) Campaign Evolved tag: apply rewrites
    /// the in-memory entry rather than writing any container, so the whole
    /// destination path is editable — this is how a new tag is moved as well as
    /// renamed. Implies `is_container`.
    pub(in crate::app) is_new_container: bool,
    /// Whether the whole destination path is editable, or only the leaf name.
    ///
    /// Stored rather than derived from the two booleans above, because it is a
    /// question about the *workflow* and those two describe *storage* — which
    /// is the distinction this module's own note warns about keeping. It is
    /// true for a brand-new tag, whose rename and move are one in-memory edit,
    /// and for Move on a tag already in a pak, where the folder is precisely
    /// the thing being changed. Those two have nothing else in common.
    pub(in crate::app) whole_path_editable: bool,
    /// For a Rename of a tag already in a pak: the pack it would be moved
    /// inside, or `None` when applying writes an overlay container instead.
    ///
    /// Resolved once, when the dialog opens, and then used by *both* the
    /// consequence text and the apply. Deciding it twice is how a dialog comes
    /// to promise one thing and do another — which is exactly what it did while
    /// the text was hard-coded to describe the overlay route.
    pub(in crate::app) in_place_pak: Option<String>,
}

/// Results of a tag query (find-references / unreferenced), shown in a floating
/// results window. Each entry is clickable to open the tag.
pub(in crate::app) struct TagQueryResults {
    /// The kit the query ran against. Its rows name tags in that kit, so
    /// opening one has to go back to it rather than to whichever kit is
    /// active by the time the row is clicked.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) title: String,
    pub(in crate::app) entries: Vec<TagEntry>,
    /// Optional per-entry annotation (parallel to `entries`), e.g. the map id.
    /// Empty when there are no annotations.
    pub(in crate::app) annotations: Vec<String>,
    /// Optional explanatory note (e.g. when the reference index is unavailable).
    pub(in crate::app) note: Option<String>,
    /// For a "References to X" query: the referenced tag's `(group_tag, rel_path)`
    /// so a clicked row can jump to the exact referencing field. `None` for other
    /// query kinds (unreferenced, map-id, …), which only open the tag.
    pub(in crate::app) ref_target: Option<(u32, String)>,
}

/// One place a referrer tag points at the "References to X" target, shown in the
/// popup's per-referrer expander. `field_path` is the exact indexed path handed
/// to `navigate_to_field`; `label` is its human breadcrumb.
pub(in crate::app) struct RefOccurrence {
    pub(in crate::app) label: String,
    pub(in crate::app) field_path: String,
}

/// A reference-jump awaiting its referrer tag to finish loading. Once that tag
/// is the focused tab and parsed, the application walks it for the exact field
/// referencing `(group_tag, rel_path)` and hands off to a [`FieldNav`].
#[derive(Clone)]
pub(in crate::app) struct PendingRefJump {
    /// The kit the referring tag belongs to.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) tag_key: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) rel_path: String,
}

/// Active "jump to a referencing field" navigation: force the target field's
/// ancestor blocks open and glow the field until `glow_until` (egui time,
/// seconds). The scroll target is set once via egui temp-data when the nav is
/// created; element selection is applied by each pane as it draws the block.
pub(in crate::app) struct FieldNav {
    /// The kit holding the tag being navigated.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) tag_key: String,
    /// Exact indexed field path, e.g. `custom references[3]/sounds[1]/melee sound`.
    pub(in crate::app) field_path: String,
    /// The element to select in each ancestor block, keyed by the block's drawn
    /// path. Applied by the renderer rather than written into egui memory up
    /// front: selection is keyed by the pane's view scope, and a tag can be
    /// open in any number of tiles whose scopes only the renderer knows.
    pub(in crate::app) block_indices: Vec<(String, usize)>,
    pub(in crate::app) glow_until: f64,
}

/// Drag-and-drop payload carried when dragging a tag from the browser onto a
/// tag-reference cell. `input` is the ready-to-apply reference string
/// (`"fourcc:back\\slash\\path"`); `group_tag` lets a drop target validate it.
#[derive(Clone)]
pub(in crate::app) struct DraggedTagRef {
    pub(in crate::app) group_tag: u32,
    /// Foundation reference-cell form: `"fourcc:back\\slash\\path"` (no ext).
    pub(in crate::app) input: String,
    /// Shader bitmap-row form: forward-slash relative path, no extension.
    pub(in crate::app) rel_path: String,
    /// The tag's file on disk, when it has one. This is what leaves Baboon
    /// when the drag ends over Sapien's window; a cache or container tag has
    /// no file to hand over and stays `None`.
    pub(in crate::app) file_path: Option<PathBuf>,
}

/// Cross-frame state of a tag drag that may end on a kit tool's window
/// (Sapien, Guerilla) rather than inside Baboon. See `kit_tool_drop`.
#[derive(Default)]
pub(in crate::app) struct KitToolDragState {
    /// The kit tool window the drag is over right now, if any. The hover
    /// feedback is redone when this changes and undone when it goes away.
    pub(in crate::app) hover: Option<KitToolDropTarget>,
    /// The status line as it was before the hover feedback replaced it, and
    /// when it was shown; put back when the drag leaves the tool's window
    /// without dropping, unless it had already run its course.
    pub(in crate::app) saved_status: Option<(String, f64)>,
    /// Executables of the processes a drag has passed over, by process id,
    /// so a drag hovering a window costs one process query rather than one
    /// per frame. Emptied between drags: a process id can be reused.
    pub(in crate::app) executables: HashMap<u32, Option<PathBuf>>,
    /// Palette tables per game, read from the definitions on a worker.
    pub(in crate::app) palettes: HashMap<GameId, PaletteTable>,
}

/// A game's scenario palette table on its way from the definitions.
pub(in crate::app) enum PaletteTable {
    /// Requested from a worker; a drop meanwhile goes through ungated.
    Loading,
    /// The definition could not be read. Remembered so a hover does not ask
    /// again every frame.
    Unreadable,
    Ready(Vec<ScenarioPalette>),
}

/// What an extraction that asks for its target game will extract.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum ExtractKind {
    Geometry,
    Animation,
}

/// The window that asks which game's tools an extraction is for, before the
/// folder is picked: the JMS, ASS and JMA versions follow it.
pub(in crate::app) struct ExtractTargetPrompt {
    /// The workspace the tag is in; the extraction runs there, whichever is
    /// active when the window is confirmed.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) key: String,
    pub(in crate::app) display_path: String,
    pub(in crate::app) kind: ExtractKind,
    /// The generation the tag comes from: the default, and the option marked
    /// current.
    pub(in crate::app) source: blam_tags::game::Game,
    pub(in crate::app) target: blam_tags::game::Game,
}

/// A one-shot "reveal in browser tree" request: force-open the folder nodes in
/// `ancestors` (root→parent labels) and scroll the entry `key` into view.
/// Consumed (taken) during the browser draw.
pub(in crate::app) struct RevealRequest {
    /// The kit whose browser should reveal it.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) key: String,
    pub(in crate::app) ancestors: Vec<String>,
}

/// Reference-graph navigator centered on one tag: who references it (parents)
/// and what it references (children). Navigating to a parent/child re-centers
/// and records back/forward history.
pub(in crate::app) struct ContentExplorer {
    /// The kit whose reference graph this is.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) focus: TagEntry,
    pub(in crate::app) parents: Vec<TagEntry>,
    pub(in crate::app) children: Vec<TagEntry>,
    /// Substring filter applied to both the parents and children lists.
    pub(in crate::app) filter: String,
    /// True when no reverse-dependency index was available to build the view.
    pub(in crate::app) index_unavailable: bool,
    pub(in crate::app) back: Vec<TagEntry>,
    pub(in crate::app) forward: Vec<TagEntry>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum BrowserMode {
    #[default]
    Folders,
    Groups,
}

/// Ordering of tags within a browser folder/group node.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub(in crate::app) enum BrowserSort {
    /// Filesystem / natural order (as built).
    #[default]
    Natural,
    /// By filename, A→Z.
    Name,
    /// By group (type), then filename.
    Type,
}

impl BrowserSort {
    pub(in crate::app) const ALL: [BrowserSort; 3] =
        [BrowserSort::Natural, BrowserSort::Name, BrowserSort::Type];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            BrowserSort::Natural => "Natural",
            BrowserSort::Name => "Name",
            BrowserSort::Type => "Type",
        }
    }
}

#[derive(Default)]
pub(in crate::app) struct FilterCache {
    scoped_signature: Option<u64>,
    snapshot_for: Option<(u64, usize, usize)>,
    snapshot: Arc<Vec<TagEntry>>,
    folder_paths_for: Option<(u64, PathBuf)>,
    folder_paths: Arc<Vec<String>>,
    pending_since: Option<std::time::Instant>,
    job_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    receiver: Option<std::sync::mpsc::Receiver<SearchResults>>,
    pub(in crate::app) entries: Vec<TagEntry>,
    pub(in crate::app) tree: TagTree,
}
struct SearchResults {
    signature: u64,
    entries: Vec<TagEntry>,
    tree: TagTree,
    folders: Option<Arc<Vec<String>>>,
}
struct SearchRequest {
    signature: u64,
    query: String,
    groups: bool,
    folder: PathBuf,
    root: Option<PathBuf>,
    scope: BrowserSearchScope,
}
impl Drop for FilterCache {
    fn drop(&mut self) {
        if let Some(cancel) = &self.job_cancel {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
impl FilterCache {
    /// Names the result set `entries` holds once a search has answered: it
    /// changes with the scope, the query and the source.
    pub(in crate::app) fn signature(&self) -> Option<u64> {
        self.scoped_signature
    }

    pub(in crate::app) fn is_searching(&self) -> bool {
        self.pending_since.is_some() || self.receiver.is_some()
    }

    /// Unchanged frames only check the request and reply. One source snapshot
    /// is shared across queries; filtering, directory walking and tree building
    /// happen on a cancellable worker after typing pauses.
    pub(in crate::app) fn refresh_scoped_async(
        &mut self,
        generation: u64,
        query: &str,
        entries: &[TagEntry],
        groups: bool,
        folder: &Path,
        root: Option<&Path>,
        scope: BrowserSearchScope,
        keywords: &Arc<std::collections::BTreeMap<String, Vec<String>>>,
        ctx: &egui::Context,
    ) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let snapshot_for = (generation, entries.len(), entries.as_ptr() as usize);
        (
            snapshot_for,
            query,
            groups,
            folder,
            root,
            scope,
            Arc::as_ptr(keywords) as usize,
        )
            .hash(&mut hasher);
        let signature = hasher.finish();
        if self.scoped_signature != Some(signature) {
            if let Some(cancel) = self.job_cancel.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            self.receiver = None;
            self.scoped_signature = Some(signature);
            self.pending_since = Some(std::time::Instant::now());
            self.entries.clear();
            self.tree = TagTree::default();
        }
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    if result.signature == signature {
                        self.entries = result.entries;
                        self.tree = result.tree;
                        if let Some(folders) = result.folders {
                            self.folder_paths = folders;
                            self.folder_paths_for = root.map(|root| (generation, root.to_owned()));
                        }
                    }
                    self.receiver = None;
                    self.job_cancel = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.receiver = None;
                    self.job_cancel = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let Some(since) = self.pending_since else {
            return;
        };
        let debounce = if query.is_empty() {
            std::time::Duration::ZERO
        } else {
            std::time::Duration::from_millis(120)
        };
        if since.elapsed() < debounce {
            ctx.request_repaint_after(debounce - since.elapsed());
            return;
        }
        self.pending_since = None;
        // Clone the source once per revision, then share it across keystrokes.
        if self.snapshot_for != Some(snapshot_for) {
            self.snapshot = Arc::new(entries.to_vec());
            self.snapshot_for = Some(snapshot_for);
        }
        let known_folders = root.and_then(|root| {
            (self.folder_paths_for.as_ref() == Some(&(generation, root.to_owned())))
                .then(|| Arc::clone(&self.folder_paths))
        });
        let request = SearchRequest {
            signature,
            query: query.to_owned(),
            groups,
            folder: folder.to_owned(),
            root: root.map(Path::to_owned),
            scope,
        };
        let snapshot = Arc::clone(&self.snapshot);
        let keywords = Arc::clone(keywords);
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.job_cancel = Some(Arc::clone(&cancel));
        let (sender, receiver) = std::sync::mpsc::channel();
        self.receiver = Some(receiver);
        let ctx = ctx.clone();
        // A panic drops the sender, and the next refresh sees it disconnected.
        crate::app::shell::worker::spawn_background("browser search", move || {
            if let Some(result) =
                run_browser_search(&request, &snapshot, &keywords, known_folders, &cancel)
            {
                if sender.send(result).is_ok() {
                    ctx.request_repaint();
                }
            }
        });
    }

    #[cfg(test)]
    pub(in crate::app) fn refresh_scoped(
        &mut self,
        generation: u64,
        query: &str,
        entries: &[TagEntry],
        groups: bool,
        folder: &Path,
        root: Option<&Path>,
        scope: BrowserSearchScope,
        keywords: &std::collections::BTreeMap<String, Vec<String>>,
    ) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (
            generation,
            query,
            entries.len(),
            groups,
            folder,
            root,
            scope,
            keywords,
        )
            .hash(&mut hasher);
        let signature = hasher.finish();
        if self.scoped_signature == Some(signature) {
            return;
        }
        let request = SearchRequest {
            signature,
            query: query.to_owned(),
            groups,
            folder: folder.to_owned(),
            root: root.map(Path::to_owned),
            scope,
        };
        let known = root.and_then(|root| {
            (self.folder_paths_for.as_ref() == Some(&(generation, root.to_owned())))
                .then(|| Arc::clone(&self.folder_paths))
        });
        let result = run_browser_search(
            &request,
            entries,
            keywords,
            known,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        self.scoped_signature = Some(signature);
        self.entries = result.entries;
        self.tree = result.tree;
        if let Some(folders) = result.folders {
            self.folder_paths = folders;
            self.folder_paths_for = root.map(|root| (generation, root.to_owned()));
        }
    }
}

fn run_browser_search(
    request: &SearchRequest,
    source: &[TagEntry],
    keywords: &std::collections::BTreeMap<String, Vec<String>>,
    known_folders: Option<Arc<Vec<String>>>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Option<SearchResults> {
    let cancelled = || cancel.load(std::sync::atomic::Ordering::Relaxed);
    let query = ScopedSearchQuery::new(&request.query);
    let normalized = request.folder.to_string_lossy().replace('\\', "/");
    let prefix = if normalized.is_empty() {
        String::new()
    } else {
        format!("{}/", normalized.trim_end_matches('/'))
    };
    let mut entries = Vec::new();
    for (index, entry) in source.iter().enumerate() {
        if index % 128 == 0 && cancelled() {
            return None;
        }
        if search_entry_is_beneath(entry, &prefix)
            && query.entry(
                entry,
                request.scope,
                keywords
                    .get(&entry.key)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            )
        {
            entries.push(entry.clone());
        }
    }
    let mut folders = None;
    let mut extra = Vec::new();
    if request.scope.folders && !request.groups && !request.query.trim().is_empty() {
        if let Some(root) = &request.root {
            let paths = match known_folders {
                Some(paths) => paths,
                None => {
                    let mut paths = Vec::new();
                    for item in walkdir::WalkDir::new(root)
                        .into_iter()
                        .filter_map(Result::ok)
                    {
                        if cancelled() {
                            return None;
                        }
                        if item.file_type().is_dir() {
                            if let Ok(path) = item.path().strip_prefix(root) {
                                paths.push(path.to_string_lossy().replace('\\', "/"));
                            }
                        }
                    }
                    Arc::new(paths)
                }
            };
            for path in paths.iter() {
                if cancelled() {
                    return None;
                }
                if query.folder(path) {
                    extra.push(path.clone());
                }
            }
            folders = Some(paths);
        }
    }
    if cancelled() {
        return None;
    }
    let tree = if request.groups {
        crate::core::source::build_group_tree(&entries)
    } else if extra.is_empty() {
        crate::core::source::build_tree_beneath(&entries, &request.folder)
    } else {
        let mut tree = crate::core::source::build_tree_with_folders(&entries, &extra);
        if !request.folder.as_os_str().is_empty() {
            fn take_beneath(nodes: &mut [TagTreeNode], folder: &Path) -> Option<TagTree> {
                for node in nodes {
                    if paths_match_case_insensitive(&node.rel_path, folder) {
                        return Some(TagTree {
                            children: std::mem::take(&mut node.children),
                            entries: std::mem::take(&mut node.entries),
                        });
                    }
                    if let Some(tree) = take_beneath(&mut node.children, folder) {
                        return Some(tree);
                    }
                }
                None
            }
            tree = take_beneath(&mut tree.children, &request.folder).unwrap_or_default();
        }
        tree
    };
    if cancelled() {
        return None;
    }
    Some(SearchResults {
        signature: request.signature,
        entries,
        tree,
        folders,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_is_debounced_and_only_the_latest_query_is_published() {
        let entries: Vec<_> = (0..10_000)
            .map(|index| TagEntry {
                key: format!("tag{index}"),
                display_path: format!("objects/tag{index}.bitmap"),
                group_tag: u32::from_be_bytes(*b"bitm"),
                group_name: None,
                location: crate::core::source::TagEntryLocation::LooseFile(format!("tag{index}").into()),
            })
            .collect();
        let keywords = Arc::new(std::collections::BTreeMap::new());
        let ctx = egui::Context::default();
        let mut cache = FilterCache::default();
        for query in ["t", "ta", "tag"] {
            cache.refresh_scoped_async(
                1,
                query,
                &entries,
                false,
                Path::new(""),
                None,
                BrowserSearchScope::default(),
                &keywords,
                &ctx,
            );
            assert!(cache.is_searching());
            assert!(
                cache.receiver.is_none(),
                "Typing alone must not start a worker on every keystroke"
            );
        }
        cache.pending_since =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(200));
        cache.refresh_scoped_async(
            1,
            "tag",
            &entries,
            false,
            Path::new(""),
            None,
            BrowserSearchScope::default(),
            &keywords,
            &ctx,
        );
        let snapshot = Arc::clone(&cache.snapshot);
        cache.refresh_scoped_async(
            1,
            "^tag9999.bitmap$",
            &entries,
            false,
            Path::new(""),
            None,
            BrowserSearchScope::default(),
            &keywords,
            &ctx,
        );
        assert!(
            cache.entries.is_empty(),
            "A superseded query cannot publish its rows"
        );
        cache.pending_since =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(200));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            cache.refresh_scoped_async(
                1,
                "^tag9999.bitmap$",
                &entries,
                false,
                Path::new(""),
                None,
                BrowserSearchScope::default(),
                &keywords,
                &ctx,
            );
            if !cache.is_searching() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Search did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.entries[0].key, "tag9999");
        assert!(
            Arc::ptr_eq(&snapshot, &cache.snapshot),
            "Subsequent queries share the source snapshot"
        );
        let rows = cache.entries.as_ptr();
        for _ in 0..100 {
            cache.refresh_scoped_async(
                1,
                "^tag9999.bitmap$",
                &entries,
                false,
                Path::new(""),
                None,
                BrowserSearchScope::default(),
                &keywords,
                &ctx,
            );
        }
        assert_eq!(
            rows,
            cache.entries.as_ptr(),
            "Unchanged frames must not rebuild results"
        );
    }
}
