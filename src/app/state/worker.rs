//! worker application state.
//! It owns passive cross-frame state and operation messages; rendering and workflow execution belong to UI and controller modules.

use super::*;

/// Results and progress events delivered from background work to the UI thread.
///
/// Variants that depend on the active source carry its generation; handlers must
/// ignore stale generations while preserving receive order for current work.
pub(in crate::app) enum WorkerMessage {
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
    FolderConversionProgress(FolderConversionProgress),
    FolderConversionFinished(Result<FolderConversionReport, String>),
    // Full recursive entry scan finished for a loose-folder source.
    AllEntriesScanned {
        generation: u64,
        result: Result<Vec<TagEntry>, String>,
    },
    // Full recursive entry scan progress for a loose-folder source.
    EntryIndexScanProgress {
        generation: u64,
        processed: usize,
        total: usize,
        matched: usize,
    },
    // Reverse-dependency reference index progress.
    ReferenceIndexProgress {
        generation: u64,
        processed: usize,
        total: usize,
    },
    // Incremental metadata-backed index refresh finished for a loose-folder source.
    EntryIndexRefreshed {
        generation: u64,
        result: Result<EntryIndexRefresh, String>,
    },
    // Entry index cache save finished after a full scan or incremental refresh.
    EntryIndexSaved {
        generation: u64,
        path: std::path::PathBuf,
        result: Result<(), String>,
    },
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
pub(in crate::app) struct FieldValueMatch {
    pub(in crate::app) entry: TagEntry,
    pub(in crate::app) label: String,
}

pub(in crate::app) struct FolderRefactorProgress {
    pub(in crate::app) label: String,
    pub(in crate::app) phase: String,
    pub(in crate::app) progress: Option<f32>,
}

pub(in crate::app) struct FolderRefactorFinished {
    pub(in crate::app) status: String,
    pub(in crate::app) lines: Vec<String>,
    pub(in crate::app) tree: TagTree,
    pub(in crate::app) all_entries: Vec<TagEntry>,
    pub(in crate::app) reverse_dependencies: Option<ReverseDependencyIndex>,
    pub(in crate::app) old_to_new_keys: HashMap<String, String>,
    pub(in crate::app) moved: bool,
}

pub(in crate::app) struct FolderRefactorUiState {
    pub(in crate::app) label: String,
    pub(in crate::app) phase: String,
    pub(in crate::app) progress: Option<f32>,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct FolderConversionProgress {
    pub(in crate::app) phase: String,
    pub(in crate::app) current: String,
    pub(in crate::app) processed: usize,
    pub(in crate::app) total: usize,
    pub(in crate::app) converted: usize,
    pub(in crate::app) failed: usize,
}

pub(in crate::app) struct UpdateCheckResult {
    pub(in crate::app) latest_tag: String,
    pub(in crate::app) release_url: String,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct EntryIndexProgressState {
    pub(in crate::app) label: String,
    pub(in crate::app) processed: usize,
    pub(in crate::app) total: usize,
    pub(in crate::app) matched: usize,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct ReferenceIndexProgressState {
    pub(in crate::app) label: String,
    pub(in crate::app) processed: usize,
    pub(in crate::app) total: usize,
}
