//! worker application state.
//! It owns passive cross-frame state and operation messages; rendering and workflow execution belong to UI and controller modules.

use super::*;

/// Completed in-place Campaign Evolved duplicate, ready for UI-thread source
/// and document registration.
pub(in crate::app) struct ContainerDuplicateResult {
    pub(in crate::app) source_key: String,
    pub(in crate::app) target_container: usize,
    /// The `.utoc` the duplicate was actually written into. Checked against the
    /// live source before the result is applied, so a workspace that reloaded
    /// onto different containers mid-write cannot adopt the wrong provenance.
    pub(in crate::app) target_utoc: PathBuf,
    pub(in crate::app) archive: Arc<blam_tags::iostore::IoStoreArchive>,
    pub(in crate::app) entry: TagEntry,
    pub(in crate::app) tag: TagFile,
    pub(in crate::app) package: String,
    pub(in crate::app) uasset_path: String,
    pub(in crate::app) ubulk_path: String,
    pub(in crate::app) target_label: String,
    pub(in crate::app) is_mod: bool,
    pub(in crate::app) backup: DuplicateBackupPaths,
    /// What to write into the duplicate ledger once the UI thread has accepted
    /// this result. Deletion is only ever offered for a copy recorded there.
    pub(in crate::app) record: CreatedTagRecord,
}

/// Completed in-place Campaign Evolved deletion, ready for UI-thread teardown.
pub(in crate::app) struct ContainerDeleteResult {
    pub(in crate::app) key: String,
    pub(in crate::app) target_container: usize,
    pub(in crate::app) target_utoc: PathBuf,
    pub(in crate::app) archive: Arc<blam_tags::iostore::IoStoreArchive>,
    pub(in crate::app) display_path: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) package: String,
    pub(in crate::app) uasset_path: String,
    pub(in crate::app) ubulk_path: String,
    pub(in crate::app) target_label: String,
    pub(in crate::app) is_mod: bool,
    pub(in crate::app) backup: DuplicateBackupPaths,
}

/// Results and progress events delivered from background work to the UI thread.
///
/// Kit-scoped variants carry a [`KitStamp`] identifying which kit the job ran
/// for and against which revision of it; handlers resolve the stamp and drop
/// the result if the kit has closed or its source was replaced, while
/// preserving receive order for current work.
pub(in crate::app) enum WorkerMessage {
    SourceLoaded {
        /// Which kit this load was started for.
        kit: KitId,
        result: Result<LoadedSourceData, String>,
        recent_path: Option<PathBuf>,
    },
    ChimpMounted {
        stamp: KitStamp,
        result: Result<Arc<blam_tags::iostore::world::World>, String>,
    },
    ChimpTypesIndexed {
        stamp: KitStamp,
        index: ChimpTypeIndex,
    },
    ChimpPackageLoaded {
        stamp: KitStamp,
        package: String,
        result: Result<ChimpDocument, String>,
    },
    TagLoaded {
        kit: KitId,
        key: String,
        result: Result<TagFile, String>,
    },
    BitmapReimportFinished {
        kit: KitId,
        key: String,
        result: Result<TagFile, String>,
    },
    ContainerDuplicateFinished {
        stamp: KitStamp,
        /// The container write this job holds. Round-tripped through the worker
        /// so the completion handler can find the lease again on the UI thread
        /// and put back everything it took, on the failure path as well as the
        /// success one.
        lease: ContainerLeaseId,
        result: Result<ContainerDuplicateResult, String>,
    },
    ContainerDeleteFinished {
        stamp: KitStamp,
        result: Result<ContainerDeleteResult, String>,
    },
    /// Exporting a level is minutes of work over thousands of cells and
    /// hundreds of meshes, so it reports where it has got to rather than going
    /// quiet for a quarter of an hour.
    ChimpLevelProgress {
        kit: KitId,
        phase: ChimpLevelPhase,
        done: usize,
        total: usize,
    },
    /// Bulk extraction of a container set's shipped tags is tens of thousands
    /// of reads and writes, so it says where it has got to. Stamped rather than
    /// carrying a bare kit id: the job outlives a source reload, and progress
    /// for containers that are no longer mounted should be dropped, not drawn.
    ContainerDumpProgress {
        stamp: KitStamp,
        done: usize,
        total: usize,
    },
    ContainerDumpFinished {
        stamp: KitStamp,
        result: Result<ContainerDumpReport, String>,
    },
    ExportFinished(Result<String, String>),
    PokePreflightFinished {
        kit: KitId,
        key: String,
        result: Result<PokePlan, String>,
    },
    PokeWriteFinished {
        kit: KitId,
        key: String,
        result: Result<(LastPoke, PokeReport), String>,
    },
    PokeDirectFinished {
        kit: KitId,
        key: String,
        result: Result<Option<(LastPoke, PokeReport)>, String>,
    },
    PokeUndoFinished {
        result: Result<PokeReport, String>,
    },
    CampaignProjectSaved {
        revision: u64,
        path: PathBuf,
        fingerprint: Vec<u8>,
        result: Result<(), String>,
    },
    FolderRefactorProgress(FolderRefactorProgress),
    /// Stamped because the finish rewrites its kit's whole browser tree and
    /// drops that kit's open documents: unrouted, a refactor that outlived a
    /// workspace switch applied all of that to whichever kit was focused.
    FolderRefactorFinished {
        stamp: KitStamp,
        result: Result<FolderRefactorFinished, String>,
    },
    FolderConversionProgress(FolderConversionProgress),
    FolderConversionFinished(Result<FolderConversionReport, String>),
    // Full recursive entry scan finished for a loose-folder source.
    AllEntriesScanned {
        stamp: KitStamp,
        result: Result<Vec<TagEntry>, String>,
    },
    // Full recursive entry scan progress for a loose-folder source.
    EntryIndexScanProgress {
        stamp: KitStamp,
        processed: usize,
        total: usize,
        matched: usize,
    },
    // Reverse-dependency reference index progress.
    ReferenceIndexProgress {
        stamp: KitStamp,
        processed: usize,
        total: usize,
    },
    // Incremental metadata-backed index refresh finished for a loose-folder source.
    EntryIndexRefreshed {
        stamp: KitStamp,
        result: Result<EntryIndexRefresh, String>,
    },
    // Entry index cache save finished after a full scan or incremental refresh.
    EntryIndexSaved {
        stamp: KitStamp,
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
    // GitHub release lookup finished. `silent` marks the automatic check made
    // at startup, which stays quiet unless it actually found an update.
    UpdateCheckFinished {
        silent: bool,
        result: Result<UpdateCheckResult, String>,
    },
    // Background field-value search finished; the stamp discards results for
    // a kit that has since closed or reloaded.
    FieldValueSearchFinished {
        stamp: KitStamp,
        query: String,
        result: Result<Vec<FieldValueMatch>, String>,
    },
    // Background field-value index build finished. `blobs` is (entry key,
    // lowercased searchable text) pairs; the stamp guards against staleness.
    FieldIndexBuilt {
        stamp: KitStamp,
        blobs: Vec<(String, String)>,
    },
    /// Progress from an exact all-tag Find scan.
    FindAllProgress {
        stamp: KitStamp,
        request_id: u64,
        processed: usize,
        total: usize,
    },
    /// Exact closed-document occurrences from an all-tag Find scan.
    FindAllFinished {
        stamp: KitStamp,
        request_id: u64,
        occurrences: Vec<FindOccurrence>,
        unreadable: usize,
    },
    // Background reverse-dependency index build finished; the stamp guards
    // against staleness after a source reload.
    ReverseDependenciesBuilt {
        stamp: KitStamp,
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

#[derive(Clone, Debug)]
pub(in crate::app) struct UpdateCheckResult {
    pub(in crate::app) channel: UpdateChannel,
    pub(in crate::app) latest_tag: String,
    pub(in crate::app) release_url: String,
    /// The release's `target_commitish`. The only thing that distinguishes one
    /// development build from the next, since they share the `dev` tag.
    pub(in crate::app) commit: String,
}

impl UpdateCheckResult {
    /// A short, human-sized name for this release: the tag for a stable
    /// release, the abbreviated commit for a development build.
    pub(in crate::app) fn short_name(&self) -> String {
        match self.channel {
            UpdateChannel::Stable => self.latest_tag.clone(),
            UpdateChannel::Development => short_commit(&self.commit),
        }
    }
}

/// Abbreviates a commit hash for display, leaving anything that is not one
/// (an empty string, a branch name) alone.
pub(in crate::app) fn short_commit(commit: &str) -> String {
    let (hash, dirty) = match commit.strip_suffix("-dirty") {
        Some(hash) => (hash, "-dirty"),
        None => (commit, ""),
    };
    let short = hash.get(..7).unwrap_or(hash);
    format!("{short}{dirty}")
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

/// A bulk container extraction in flight, and how far along it is.
///
/// Only one runs at a time across the whole application: it saturates the disk,
/// and two of them would each report half a bar while taking twice as long.
pub(in crate::app) struct ContainerDumpJob {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) output: PathBuf,
    pub(in crate::app) done: usize,
    pub(in crate::app) total: usize,
    pub(in crate::app) started: std::time::Instant,
    /// Set from the UI thread by the Cancel button; the workers check it between
    /// tags, so a run stops within one tag rather than at the next file boundary
    /// of a job that has minutes left.
    pub(in crate::app) cancel: Arc<AtomicBool>,
}

impl ContainerDumpJob {
    pub(in crate::app) fn fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.done as f32 / self.total as f32).clamp(0.0, 1.0)
    }

    /// How much longer this looks like taking, or `None` until there is enough
    /// of it done to say anything honest.
    pub(in crate::app) fn remaining(&self) -> Option<std::time::Duration> {
        if self.done == 0 || self.done >= self.total {
            return None;
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        // A first few tags are not a rate. Guessing from them produces a number
        // that swings by minutes and teaches the user to ignore it.
        if elapsed < 1.5 {
            return None;
        }
        let each = elapsed / self.done as f64;
        Some(std::time::Duration::from_secs_f64(
            each * (self.total - self.done) as f64,
        ))
    }
}
