//! documents application state.
//! It owns passive cross-frame state and operation messages; rendering and workflow execution belong to UI and controller modules.

use super::*;

/// Parsed tag plus its unsaved-state and byte-snapshot edit history.
/// `dirty` reflects divergence from the last successful save, while journal
/// entries may still exist after saving to support later undo operations.
pub(in crate::app) struct TagDocument {
    pub(in crate::app) tag: TagFile,
    pub(in crate::app) dirty: bool,
    pub(in crate::app) journal: EditJournal,
}

impl TagDocument {
    pub(in crate::app) fn clean(tag: TagFile) -> Self {
        Self {
            tag,
            dirty: false,
            journal: EditJournal::default(),
        }
    }

    /// A document that starts life already modified — used for tags that exist
    /// only in memory (a newly-created or imported container tag with no backing
    /// payload on disk or in a pak), so closing it prompts to save and it feeds
    /// Export Mod as a modified tag.
    pub(in crate::app) fn modified(tag: TagFile) -> Self {
        Self {
            tag,
            dirty: true,
            journal: EditJournal::default(),
        }
    }
}

#[derive(Clone, Debug)]
/// Close transaction retained while the save/discard prompt spans UI frames.
/// Key-bearing variants use the same stable keys as `parsed_tags` and tabs.
pub(in crate::app) enum PendingCloseAction {
    CloseApp,
    CloseTab(String),
    CloseAllTabs,
    CloseAllButThis(String),
}

pub(in crate::app) struct DirtyTagEntry {
    pub(in crate::app) path: String,
    pub(in crate::app) tag_id: String,
    pub(in crate::app) checked: bool,
}

/// Foundation-style confirmation shown when a close action would discard
/// edited tags. `allow_app_close_once` is set only after the user confirms an
/// app exit; the next native close request is then allowed through instead of
/// being vetoed and prompting again.
pub(in crate::app) struct SaveChangesPrompt {
    pub(in crate::app) visible: bool,
    pub(in crate::app) dirty_tags: Vec<DirtyTagEntry>,
    pub(in crate::app) pending_action: PendingCloseAction,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) allow_app_close_once: bool,
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

pub(in crate::app) struct ProjectCheckpointPrompt {
    pub(in crate::app) action: PendingCloseAction,
    pub(in crate::app) error: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum LastSessionSourceKind {
    SingleFile,
    LooseFolder,
    MonolithicCache,
    IoStoreContainerSet,
}

impl LastSessionSourceKind {
    pub(in crate::app) fn as_str(self) -> &'static str {
        match self {
            LastSessionSourceKind::SingleFile => "single_file",
            LastSessionSourceKind::LooseFolder => "loose_folder",
            LastSessionSourceKind::MonolithicCache => "monolithic_cache",
            LastSessionSourceKind::IoStoreContainerSet => "iostore_container_set",
        }
    }

    pub(in crate::app) fn from_str(value: &str) -> Option<Self> {
        match value {
            "single_file" => Some(Self::SingleFile),
            "loose_folder" => Some(Self::LooseFolder),
            "monolithic_cache" => Some(Self::MonolithicCache),
            "iostore_container_set" => Some(Self::IoStoreContainerSet),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct LastSessionTag {
    pub(in crate::app) key: String,
    pub(in crate::app) label: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct LastSessionState {
    pub(in crate::app) source_kind: LastSessionSourceKind,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) game: Option<String>,
    pub(in crate::app) project_path: Option<PathBuf>,
    pub(in crate::app) tags: Vec<LastSessionTag>,
}

pub(in crate::app) struct LastOpenedWindowEntry {
    pub(in crate::app) tag: LastSessionTag,
    pub(in crate::app) checked: bool,
    pub(in crate::app) available: bool,
}

/// Launch-time restore prompt backed by `last_session.json`. OK first reloads
/// the saved source (loose folder, monolithic cache, or single file); once the
/// async source load completes, the queued tag keys are reopened through the
/// normal `select_entry` path.
pub(in crate::app) struct LastOpenedWindowsPrompt {
    pub(in crate::app) visible: bool,
    pub(in crate::app) source_kind: LastSessionSourceKind,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) source_available: bool,
    pub(in crate::app) project_path: Option<PathBuf>,
    pub(in crate::app) entries: Vec<LastOpenedWindowEntry>,
    /// "Don't ask again": on OK, remember as Always; on Cancel, as Never.
    pub(in crate::app) dont_ask_again: bool,
}

impl LastOpenedWindowsPrompt {
    pub(in crate::app) fn from_session(session: LastSessionState) -> Option<Self> {
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
            LastSessionSourceKind::IoStoreContainerSet => {
                crate::source::find_paks_dir(&session.source_path).is_some()
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
        if entries.is_empty() && session.project_path.is_none() {
            return None;
        }
        Some(Self {
            visible: true,
            source_kind: session.source_kind,
            source_path: session.source_path,
            source_available,
            project_path: session.project_path,
            entries,
            dont_ask_again: false,
        })
    }

    pub(in crate::app) fn checked_tags(&self) -> Vec<LastSessionTag> {
        self.entries
            .iter()
            .filter(|entry| entry.available && entry.checked)
            .map(|entry| entry.tag.clone())
            .collect()
    }
}

pub(in crate::app) struct PendingSessionRestore {
    pub(in crate::app) tags: Vec<LastSessionTag>,
}

/// Import-a-tag-file dialog for a Campaign Evolved container source. Owns the
/// parsed imported `TagFile` (moved out on confirm) and the schema-comparison
/// result against our shipped JSON. Not `Clone` — `TagFile` isn't cloneable.
pub(in crate::app) struct ImportTagDialog {
    pub(in crate::app) source_path: PathBuf,
    /// Pre-filled container folder (empty for the root); the leaf name is `name`.
    pub(in crate::app) folder_rel: String,
    pub(in crate::app) name: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) group_name: String,
    pub(in crate::app) extension: String,
    /// The parsed imported tag; `take()`n when the user confirms.
    pub(in crate::app) tag: Option<TagFile>,
    /// Structural comparison of the imported tag against our JSON definition.
    pub(in crate::app) comparison: Option<blam_tags::LayoutComparison>,
    /// User override for benign (field-count) schema drift.
    pub(in crate::app) import_anyway: bool,
    pub(in crate::app) error: Option<String>,
}

/// A parsed imported tag awaiting a "discard unsaved edits?" confirmation before
/// it overwrites an already-open, dirty document at `target_key`.
pub(in crate::app) struct PendingImport {
    pub(in crate::app) tag: TagFile,
    pub(in crate::app) target_key: String,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct NewTagGroup {
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) name: String,
    pub(in crate::app) schema_path: PathBuf,
    pub(in crate::app) extension: String,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct NewTagDialog {
    pub(in crate::app) game: String,
    pub(in crate::app) rel_path: String,
    pub(in crate::app) output_path: Option<PathBuf>,
    pub(in crate::app) groups: Vec<NewTagGroup>,
    pub(in crate::app) selected_group: usize,
    pub(in crate::app) error: Option<String>,
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

pub(in crate::app) struct TagFieldDiff {
    pub(in crate::app) path: String,
    pub(in crate::app) a: String,
    pub(in crate::app) b: String,
}

/// State for the "Compare Tags" window: tag A (fixed to the launch tag), the
/// chosen tag B, and the computed diff (once "Compare" is clicked).
pub(in crate::app) struct TagDiffState {
    pub(in crate::app) a_key: String,
    /// Open-tab key of tag B (when B is an open tag); `None` when B was picked
    /// from disk (then `results`/`b_display` are set directly).
    pub(in crate::app) b_key: Option<String>,
    /// Display label for tag B (open key or picked disk path).
    pub(in crate::app) b_display: Option<String>,
    pub(in crate::app) results: Option<TagDiffResults>,
}

pub(in crate::app) struct TagDiffResults {
    pub(in crate::app) diffs: Vec<TagFieldDiff>,
    /// True when the diff hit the cap and more differences exist.
    pub(in crate::app) truncated: bool,
}
