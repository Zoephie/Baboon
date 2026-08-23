//! Importing a monolithic cache's tags into an editing kit: the dialog's state,
//! and the workflow that runs one.
//! It owns choosing a destination kit and driving the conversion worker; what a
//! tag becomes on the way is `blam_tags::convert`, and the run itself is
//! `conversion.rs`.

use super::*;

use std::collections::BTreeMap;

/// The Import Cache Folder window, from the question it asks to the report it
/// leaves behind.
///
/// One window rather than a confirmation and a separate progress dialog: the run
/// is long, the destination has to be chosen before it starts, and the report is
/// what the user came for. Splitting those across three surfaces would leave
/// three places to look for the answer to "what happened to my folder?".
pub(in crate::app) struct CacheImportDialog {
    /// The cache workspace the tags come out of. Resolved when the window opens,
    /// so focusing another workspace mid-run still gives the import that was
    /// asked for.
    pub(in crate::app) kit: KitId,
    /// Tag-name prefix. `""` is the whole cache.
    pub(in crate::app) prefix: String,
    /// How many tags sit under `prefix`, before any reference is followed.
    pub(in crate::app) selected: usize,
    /// The open loose kits this could land in. Resolved once when the window
    /// opens: re-resolving every frame would move the list under the user's
    /// cursor, and a kit that closes mid-run is caught by the stamp instead.
    pub(in crate::app) targets: Vec<CacheImportTarget>,
    pub(in crate::app) target_index: usize,
    /// Which folders of the last run's outside references the user has ticked,
    /// keyed by [`OutsideReference::folder`]. Rebuilt each time a run finishes,
    /// all ticked, because bringing everything the folder needs is the common
    /// answer and unticking is the deliberate one.
    pub(in crate::app) outside_groups: BTreeMap<String, bool>,
    pub(in crate::app) running: bool,
    /// Set from the UI thread by Cancel; the worker reads it between tags.
    pub(in crate::app) cancel: Arc<AtomicBool>,
    pub(in crate::app) progress: Option<FolderConversionProgress>,
    pub(in crate::app) report: Option<FolderConversionReport>,
    pub(in crate::app) error: Option<String>,
}

/// One editing kit an import could land in.
pub(in crate::app) struct CacheImportTarget {
    pub(in crate::app) kit: KitId,
    pub(in crate::app) label: String,
    pub(in crate::app) game: String,
    pub(in crate::app) tags_root: PathBuf,
}

impl CacheImportDialog {
    pub(in crate::app) fn target(&self) -> Option<&CacheImportTarget> {
        self.targets.get(self.target_index)
    }
}

impl Baboon {
    /// Open the window for a folder in the active cache workspace.
    ///
    /// Refuses up front rather than opening an empty window: with no loose kit
    /// loaded there is nowhere for the tags to go, and finding that out after
    /// choosing a folder is a worse way to learn it.
    pub(super) fn open_cache_import_dialog(&mut self, prefix: String) {
        if self.cache_import_dialog.is_some() {
            self.status = "A cache import is already open".to_owned();
            return;
        }
        let kit = self.active_kit_id();
        let Some(source_data) = self.source() else {
            return;
        };
        if !matches!(source_data.source, TagSource::MonolithicCache { .. }) {
            self.status = "This is not a monolithic cache workspace".to_owned();
            return;
        }
        let selected = source_data
            .entries
            .iter()
            .filter(|entry| cache_entry_is_under(entry, &prefix))
            .count();
        if selected == 0 {
            self.status = format!("{prefix} holds no tags to import");
            return;
        }
        let targets = self.cache_import_targets();
        if targets.is_empty() {
            self.status =
                "Open the editing kit these tags should land in first — File › Load \
                 Folder"
                    .to_owned();
            return;
        }
        self.cache_import_dialog = Some(CacheImportDialog {
            kit,
            prefix,
            selected,
            targets,
            target_index: 0,
            outside_groups: BTreeMap::new(),
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: None,
            report: None,
            error: None,
        });
    }

    /// Every open loose workspace with a detected game, as somewhere to land.
    ///
    /// Loose only, and for the same reason Import Tags is: a cache is read-only
    /// and a Campaign Evolved container is a set of packages rather than files.
    /// Ordered by label so the list does not shuffle between openings.
    fn cache_import_targets(&self) -> Vec<CacheImportTarget> {
        let mut targets: Vec<CacheImportTarget> = self
            .kits
            .iter()
            .filter_map(|kit| {
                let source = kit.source.as_ref()?;
                let TagSource::LooseFolder { root, game, .. } = &source.source else {
                    return None;
                };
                Some(CacheImportTarget {
                    kit: kit.id,
                    label: source.label.clone(),
                    game: game.clone()?,
                    tags_root: root.clone(),
                })
            })
            .collect();
        targets.sort_by(|left, right| left.label.cmp(&right.label));
        targets
    }
}

impl Baboon {
    /// Start the run the window has been configured for.
    ///
    /// Everything the worker needs is resolved here, on the UI thread, and moved
    /// in: the entry list, the destination, the kit roots. The worker never
    /// reaches back into the application, which is what lets the user carry on
    /// editing while thousands of tags convert.
    /// `only` names the tags to convert instead of the folder — the second run,
    /// after the user has seen what the folder reached for and ticked which of
    /// it to bring.
    pub(super) fn start_cache_import(
        &mut self,
        ctx: egui::Context,
        only: Option<HashSet<String>>,
    ) {
        let Some(dialog) = self.cache_import_dialog.as_ref() else {
            return;
        };
        if dialog.running {
            return;
        }
        let Some(target) = dialog.target() else {
            return;
        };
        let (kit, prefix) = (dialog.kit, dialog.prefix.clone());
        let (target_game, target_tags_root) = (target.game.clone(), target.tags_root.clone());
        let cancel = dialog.cancel.clone();
        cancel.store(false, Ordering::Relaxed);

        let Some(index) = self.kit_index(kit) else {
            return;
        };
        let Some(source_data) = self.kits[index].source.as_ref() else {
            return;
        };
        // Cloning the source shares the open cache rather than reopening it:
        // `TagSource::MonolithicCache` holds it behind an `Arc`.
        let source = source_data.source.clone();
        let names = source_data.names.clone();
        // The whole cache, not just the folder. The worker filters for the seed
        // and keeps the rest as the index a followed reference resolves through.
        let entries = source_data.entries.clone();
        let stamp = KitStamp {
            kit,
            generation: self.kits[index].generation,
        };
        // A cache tag is Reach's own format at another byte order, so the source
        // profile is the destination's. That pair is refused everywhere else and
        // legal here only because the source is big-endian; see
        // `blam_tags::convert::analyze_conversion_inner`.
        let source_game = target_game.clone();
        let job = FolderConversionJob {
            source,
            names,
            scope: FolderConversionScope::CacheSubtree {
                prefix,
                entries,
                seed: match only {
                    Some(keys) => CacheSeed::Keys(keys),
                    None => CacheSeed::Folder,
                },
            },
            source_game,
            target_game,
            target_tags_root,
            kit_roots: self
                .editing_kit_paths
                .iter()
                .map(|(game, root)| (game.clone(), import_tags_root(root)))
                .collect(),
            accept_loss: false,
            only: None,
            cancel,
        };
        if let Some(dialog) = self.cache_import_dialog.as_mut() {
            dialog.running = true;
            dialog.report = None;
            dialog.error = None;
            dialog.progress = Some(FolderConversionProgress {
                phase: "Preparing".to_owned(),
                current: String::new(),
                processed: 0,
                total: 0,
                converted: 0,
                failed: 0,
            });
        }
        self.status = "Importing cache tags".to_owned();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_folder_conversion_job(job, &tx)
            }))
            .unwrap_or_else(|_| Err("The cache import worker crashed".to_owned()));
            let _ = tx.send(WorkerMessage::CacheImportFinished { stamp, result });
            ctx.request_repaint();
        });
    }

    pub(super) fn handle_cache_import_progress(
        &mut self,
        progress: FolderConversionProgress,
    ) -> bool {
        if let Some(dialog) = self.cache_import_dialog.as_mut()
            && dialog.running
        {
            dialog.progress = Some(progress);
        }
        false
    }

    pub(super) fn handle_cache_import_finished(
        &mut self,
        stamp: KitStamp,
        result: Result<FolderConversionReport, String>,
        ctx: &egui::Context,
    ) -> bool {
        // The tags landed in a kit that may since have closed or been reloaded.
        // Dropping the result then is the point of the stamp: reporting it would
        // attach a run's outcome to whatever workspace happens to hold that slot
        // now.
        if self.resolve_stamp(stamp).is_none() {
            return false;
        }
        let Some(dialog) = self.cache_import_dialog.as_mut() else {
            return false;
        };
        dialog.running = false;
        dialog.progress = None;
        let target_kit = dialog.target().map(|target| target.kit);
        match result {
            Ok(report) => {
                // All ticked: bringing what the folder needs is the common
                // answer, and leaving something out is the deliberate one.
                dialog.outside_groups = report
                    .outside_references
                    .iter()
                    .map(|reference| (reference.folder.clone(), true))
                    .collect();
                self.status = format!(
                    "Imported {} tag(s), {} failed, {} held back{}",
                    report.converted_count(),
                    report.failed_count(),
                    report.held_back.len(),
                    if report.cancelled { " (cancelled)" } else { "" },
                );
                dialog.report = Some(report);
                // The destination gained files this application did not write
                // through its own document layer, so its browser index is stale.
                if let Some(kit) = target_kit {
                    self.refresh_after_import(kit, &ctx);
                }
            }
            Err(error) => {
                self.status = format!("Cache import failed: {error}");
                dialog.error = Some(error);
            }
        }
        false
    }
}
