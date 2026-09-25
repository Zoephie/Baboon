//! worker application state.
//! It owns passive cross-frame state and operation messages; rendering and workflow execution belong to UI and controller modules.

use super::*;
use crate::app::controller::{InPlaceOverwrite, InPlaceOverwriteJob};
use crate::app::ui::tag_compare::TagCompareGitUpdate;

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

/// Completed in-place Campaign Evolved rename, ready for UI-thread rekeying.
///
/// Carries both halves of every path, because the UI thread has to *move* state
/// rather than insert it: three container indices are keyed by the old path and
/// the new one, and a rename that knew only where the tag ended up could not
/// remove where it had been.
pub(in crate::app) struct ContainerRenameResult {
    pub(in crate::app) old_key: String,
    pub(in crate::app) target_container: usize,
    pub(in crate::app) target_utoc: PathBuf,
    pub(in crate::app) archive: Arc<blam_tags::iostore::IoStoreArchive>,
    /// The tag at its new path, with the key the browser will address it by.
    pub(in crate::app) entry: TagEntry,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) old_package: String,
    pub(in crate::app) new_package: String,
    pub(in crate::app) new_uasset_path: String,
    pub(in crate::app) old_ubulk_path: String,
    pub(in crate::app) new_ubulk_path: String,
    pub(in crate::app) old_display: String,
    pub(in crate::app) target_label: String,
    pub(in crate::app) is_mod: bool,
    pub(in crate::app) backup: DuplicateBackupPaths,
    /// The ledger row for the tag's new home. Its origin is decided by
    /// `record_rename` from the row being replaced, not taken from here.
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
    /// A game's scenario palette table, read from the definitions for a drag
    /// heading for Sapien. Not kit-scoped: the definitions are shared.
    /// `None` means the definition could not be read.
    ScenarioPalettesRead {
        game: String,
        palettes: Option<Vec<ScenarioPalette>>,
    },
    ChimpMounted {
        stamp: KitStamp,
        result: Result<Arc<blam_tags::iostore::world::World>, String>,
    },
    ChimpTypesIndexed {
        stamp: KitStamp,
        index: ChimpTypeIndex,
    },
    /// A Chimp mod container built and checked at `temporary`, to be
    /// installed over `output` on the UI thread.
    ChimpModBuilt {
        kit: KitId,
        output: PathBuf,
        temporary: PathBuf,
        written: Vec<ChimpWritten>,
        result: Result<(), String>,
    },
    /// Chimp packages written into their own source containers.
    ChimpSourcesOverwritten {
        kit: KitId,
        leases: Vec<ContainerLeaseId>,
        containers: usize,
        touched: bool,
        written: Vec<ChimpWritten>,
        result: Result<(), String>,
    },
    /// A Compare window Git read; `request` says which.
    TagCompareGit {
        request: u64,
        update: Result<TagCompareGitUpdate, String>,
    },
    /// A Git Review job's finished view; `request` says which job it was.
    GitReviewUpdated {
        kit: KitId,
        request: u64,
        view: Result<GitReviewView, String>,
    },
    ChimpPackageLoaded {
        stamp: KitStamp,
        package: String,
        result: Result<ChimpDocument, String>,
    },
    /// A sweep for the packages that import `package`. There is no reverse
    /// index in the paks, so this reads every mounted header and cannot run on
    /// the UI thread.
    ChimpReferrersScanned {
        stamp: KitStamp,
        package: String,
        scan: ChimpReferrerScan,
    },
    TagLoaded {
        kit: KitId,
        key: String,
        result: Result<TagFile, String>,
    },
    /// Where one unopened referrer points at the "References to" target, read
    /// and walked off the UI thread.
    RefJumpOccurrences {
        kit: KitId,
        index: usize,
        key: String,
        target: (u32, String),
        result: Result<Vec<RefOccurrence>, String>,
    },
    /// One Bitmap Library thumbnail, decoded off the UI thread.
    BitmapThumbnailDecoded {
        stamp: KitStamp,
        key: String,
        result: Result<ThumbnailImage, String>,
    },
    /// One Model Library thumbnail, rasterized off the UI thread.
    ModelThumbnailRendered {
        stamp: KitStamp,
        key: String,
        result: Result<ThumbnailImage, String>,
    },
    /// A `.model`'s animation-graph listing, read off the UI thread.
    ModelAnimationsListed {
        stamp: KitStamp,
        key: String,
        result: Result<Vec<PreviewAnimationEntry>, String>,
    },
    /// One animation decoded into per-frame node transforms, off the UI
    /// thread, in skeleton order — the handler maps it onto the preview's
    /// nodes by name.
    ModelAnimationDecoded {
        stamp: KitStamp,
        key: String,
        animation_index: usize,
        result: Result<DecodedAnimationPose, String>,
    },
    /// A `.model`'s collision/physics overlay geometry, built off the UI
    /// thread so the preview's toggles can be instant draw-time filters.
    ModelOverlaysBuilt {
        stamp: KitStamp,
        /// The tag whose `ModelPreviewState` asked for these.
        key: String,
        /// The base preview the overlays were built against; a reload in the
        /// meantime orphans the build.
        geometry_id: u64,
        collision: Option<RenderModelPreview>,
        physics: Option<RenderModelPreview>,
    },
    /// One model's materials resolved to decoded textures, off the UI thread.
    ModelTexturesResolved {
        stamp: KitStamp,
        /// The tag whose `ModelPreviewState` asked for these.
        key: String,
        /// Identifies which load this answers, so a reply that arrives after the
        /// user switched detail level or reloaded is dropped rather than paired
        /// with geometry it does not belong to.
        textures_id: u64,
        textures: Vec<MaterialTextures>,
    },
    BitmapReimportFinished {
        kit: KitId,
        key: String,
        result: Result<TagFile, String>,
    },
    /// A progress line from a running Blam! import: appended to the pane's
    /// log window, and (for Info lines) shown in its status bar as the
    /// current step.
    BlamImportProgress {
        stamp: KitStamp,
        kind: BlamLogKind,
        message: String,
    },
    /// A finished Blam! import: one outcome per pipeline the user ticked
    /// (label, then a summary or the reason it failed), plus every tag the
    /// worker wrote to disk, parsed back and ready to register.
    BlamImportFinished {
        stamp: KitStamp,
        outcomes: Vec<(String, Result<String, String>)>,
        created: Vec<(crate::source::TagEntry, TagFile)>,
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
    ContainerRenameFinished {
        stamp: KitStamp,
        /// Round-tripped through the worker exactly as the duplicate's is, so
        /// the completion handler can put back everything the lease took on the
        /// failure path as well as the success one.
        lease: ContainerLeaseId,
        result: Result<ContainerRenameResult, String>,
    },
    /// A tag written into its own container on a worker.
    InPlaceOverwriteFinished {
        job: Box<InPlaceOverwriteJob>,
        lease: ContainerLeaseId,
        written: InPlaceOverwrite,
    },
    ContainerDeleteFinished {
        stamp: KitStamp,
        /// The container write this job holds, round-tripped like Duplicate's
        /// and Rename's so the handler can release it on every path.
        lease: ContainerLeaseId,
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
    /// The same shape, for the import that reads a monolithic cache. Kept apart
    /// from the pair above because the two have separate windows and nothing
    /// stops both running — one stream feeding both would put a cache run's
    /// numbers in the Import Tags progress bar.
    CacheImportProgress(FolderConversionProgress),
    /// Stamped, unlike its loose-folder twin: this run writes into a *different*
    /// workspace from the one it reads, over minutes, and the destination can be
    /// closed or reloaded while it works.
    CacheImportFinished {
        stamp: KitStamp,
        result: Result<FolderConversionReport, String>,
    },
    /// Which of the tags an import would write are already in the destination
    /// kit. Off the UI thread because answering it is one `exists` per tag and
    /// the whole cache is a hundred thousand of them.
    CacheImportConflicts {
        stamp: KitStamp,
        conflicts: Vec<OutsideReference>,
    },
    /// What an Import Tags source path turned out to be. Carries the input it
    /// was measured from, because the walk can outlast the user's typing and a
    /// result for a path they have since changed has to be dropped, not shown.
    ImportSourceResolved {
        input: String,
        result: Result<ImportSourceFacts, String>,
    },
    /// A converted single tag, ready to write. Off-thread because it indexes
    /// the destination kit's native layout templates, which walks its whole
    /// tree. `templates` is that index coming back so the next import can reuse
    /// it — it rides on the message rather than a shared handle because it
    /// memoises internally and so is `Send` but not `Sync`.
    ImportAnalysisFinished {
        result: Result<ImportAnalysis, String>,
        templates: NativeTemplateCache,
    },
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
    /// A whole-source listing from the Tools menu, read off the UI thread.
    SourceListingReady {
        stamp: KitStamp,
        results: TagQueryResults,
    },
    ReverseDependenciesBuilt {
        stamp: KitStamp,
        index: ReverseDependencyIndex,
        /// Tags left out because the thread reading them crashed. Non-zero
        /// means the index is incomplete, so it is used but not saved.
        missing: usize,
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

/// Run `job` on a worker thread and send the message it returns, then wake
/// the UI.
///
/// If `job` panics, `on_panic` builds the message instead, from the panic's
/// text. Most workers were plain `thread::spawn`s: a panic there sent
/// nothing, so whatever the UI had marked as in flight (a loading tag, a
/// running search, a container write) stayed that way for the session.
/// Going through this, every job answers.
pub(in crate::app) fn spawn_worker<J, P>(
    tx: &std::sync::mpsc::Sender<WorkerMessage>,
    ctx: &egui::Context,
    job: J,
    on_panic: P,
) where
    J: FnOnce() -> WorkerMessage + Send + 'static,
    P: FnOnce(String) -> WorkerMessage + Send + 'static,
{
    let (tx, ctx) = (tx.clone(), ctx.clone());
    std::thread::spawn(move || {
        let message = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
            .unwrap_or_else(|panic| on_panic(panic_text(panic.as_ref())));
        let _ = tx.send(message);
        ctx.request_repaint();
    });
}

/// A panic payload as text, for a message saying the job crashed.
pub(in crate::app) fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    let detail = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&'static str>().copied())
        .unwrap_or("no message");
    format!("the worker crashed: {detail}")
}

#[cfg(test)]
mod spawn_worker_tests {
    use super::*;
    use std::time::Duration;

    /// A worker that panics still answers, so whatever the UI marked as in
    /// flight is settled. A plain `thread::spawn` sent nothing.
    #[test]
    fn a_panicking_worker_still_sends_its_message() {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = egui::Context::default();
        spawn_worker(
            &tx,
            &ctx,
            || panic!("decoder fell over"),
            |error| WorkerMessage::TagLoaded {
                kit: KitId(0),
                key: "k".to_owned(),
                result: Err(error),
            },
        );
        spawn_worker(
            &tx,
            &ctx,
            || WorkerMessage::TagLoaded {
                kit: KitId(0),
                key: "fine".to_owned(),
                result: Err("not a panic".to_owned()),
            },
            |_| unreachable!(),
        );

        let mut results = Vec::new();
        for _ in 0..2 {
            let Ok(WorkerMessage::TagLoaded { key, result, .. }) =
                rx.recv_timeout(Duration::from_secs(10))
            else {
                panic!("a worker did not answer");
            };
            results.push((key, result.unwrap_err()));
        }
        results.sort();
        assert_eq!(results[0], ("fine".to_owned(), "not a panic".to_owned()));
        assert_eq!(results[1].0, "k");
        assert!(
            results[1].1.contains("decoder fell over"),
            "{}",
            results[1].1
        );
    }
}
