//! browser application state.
//! It owns passive cross-frame state and operation messages; rendering and workflow execution belong to UI and controller modules.

use super::*;

pub(in crate::app) enum BrowserAction {
    Select(String),
    ToggleFavorite(String),
    CopyTagName(String),
    DumpJson(String),
    OpenInExplorer(String),
    DumpLoadedFolderJson(Vec<String>),
    DumpLooseFolderJson { rel_path: PathBuf, label: String },
    MoveLooseFolder { rel_path: PathBuf, label: String },
    CopyLooseFolder { rel_path: PathBuf, label: String },
    ConvertLooseFolder { rel_path: PathBuf, label: String },
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
    MoveTag(String),
}

/// The "Rename / Move tag (fix references)" dialog. Shows the referrers that
/// will be rewritten (preview) and an editable destination path; applying moves
/// the file on disk and rewrites every referencing tag.
pub(in crate::app) struct RenameTagState {
    pub(in crate::app) key: String,
    /// Current display path (forward slashes, with extension) — shown read-only.
    pub(in crate::app) old_display: String,
    /// File extension (kept fixed; the group can't change on rename).
    pub(in crate::app) extension: String,
    /// Editable destination: relative path, forward slashes, NO extension.
    pub(in crate::app) new_path_input: String,
    /// Display paths of tags that reference this one and will be updated.
    pub(in crate::app) referrers: Vec<String>,
    /// True when no reverse-dependency index was available to list referrers.
    pub(in crate::app) referrers_unavailable: bool,
}

/// Results of a tag query (find-references / unreferenced), shown in a floating
/// results window. Each entry is clickable to open the tag.
pub(in crate::app) struct TagQueryResults {
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
/// is the focused tab and parsed, the controller walks it for the exact field
/// referencing `(group_tag, rel_path)` and hands off to a [`FieldNav`].
#[derive(Clone)]
pub(in crate::app) struct PendingRefJump {
    pub(in crate::app) tag_key: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) rel_path: String,
}

/// Active "jump to a referencing field" navigation: force the target field's
/// ancestor blocks open and glow the field until `glow_until` (egui time,
/// seconds). Element selection along the path and the scroll target are set once
/// via egui temp-data when the nav is created.
pub(in crate::app) struct FieldNav {
    pub(in crate::app) tag_key: String,
    /// Exact indexed field path, e.g. `custom references[3]/sounds[1]/melee sound`.
    pub(in crate::app) field_path: String,
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
}

/// A one-shot "reveal in browser tree" request: force-open the folder nodes in
/// `ancestors` (root→parent labels) and scroll the entry `key` into view.
/// Consumed (taken) during the browser draw.
pub(in crate::app) struct RevealRequest {
    pub(in crate::app) key: String,
    pub(in crate::app) ancestors: Vec<String>,
}

/// Reference-graph navigator centered on one tag: who references it (parents)
/// and what it references (children). Navigating to a parent/child re-centers
/// and records back/forward history.
pub(in crate::app) struct ContentExplorer {
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum BrowserMode {
    Folders,
    Groups,
}

/// Ordering of tags within a browser folder/group node.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum BrowserSort {
    /// Filesystem / natural order (as built).
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
    pub(in crate::app) entries: Vec<TagEntry>,
    /// Pruned hierarchy over [`entries`] — folder tree or group tree per mode.
    pub(in crate::app) tree: TagTree,
}

impl FilterCache {
    /// Rebuild the pruned match tree if anything it depends on changed;
    /// otherwise reuse the cached tree.
    pub(in crate::app) fn refresh(
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
