//! Importing a monolithic cache's tags into an editing kit: the dialog's state,
//! and the workflow that runs one.
//! It owns choosing a destination kit and driving the conversion worker; what a
//! tag becomes on the way is `blam_tags::convert`, and the run itself is
//! `conversion.rs`.

use super::*;

use std::collections::{BTreeMap, BTreeSet};

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
    /// The last run's outside references as a folder tree.
    pub(in crate::app) outside_tree: OutsideTree,
    /// Which of them the user has ticked, keyed by [`OutsideReference::key`].
    /// Rebuilt each time a run finishes, all ticked, because bringing
    /// everything the folder needs is the common answer and leaving something
    /// out is the deliberate one.
    pub(in crate::app) outside_picked: BTreeMap<String, bool>,
    /// Set when the window was opened for one tag rather than a folder.
    ///
    /// A folder import lands every tag at its own path, because that is what
    /// keeps references working. One tag is the case where somebody wants it
    /// somewhere else, and is in a position to know what that costs.
    pub(in crate::app) single: Option<SingleTagImport>,
    pub(in crate::app) running: bool,
    /// Set from the UI thread by Cancel; the worker reads it between tags.
    pub(in crate::app) cancel: Arc<AtomicBool>,
    pub(in crate::app) progress: Option<FolderConversionProgress>,
    pub(in crate::app) report: Option<FolderConversionReport>,
    pub(in crate::app) error: Option<String>,
}

/// The outside references arranged the way they are stored, so the question can
/// be answered at whatever depth the answer lives at.
///
/// The flat list this replaces grouped by the first two path segments, which is
/// readable but blunt: `objects/characters` is one tick and two thousand tags,
/// and wanting the elite but not the whole cast meant taking both or neither.
/// A folder here is a real folder, opens, and can be answered at any level down
/// to the individual tag.
///
/// Built once when a run reports, not per frame: a run can reach thousands of
/// references and the shape does not change while the answer is being given.
#[derive(Default)]
pub(in crate::app) struct OutsideTree {
    /// Folder path → the folders directly inside it.
    pub(in crate::app) folders: BTreeMap<String, BTreeSet<String>>,
    /// Folder path → the tags directly in it, as `(key, leaf name)`.
    pub(in crate::app) tags: BTreeMap<String, Vec<(String, String)>>,
    /// Folder path → how many tags its whole subtree holds.
    pub(in crate::app) totals: BTreeMap<String, usize>,
    /// The folders with no parent.
    pub(in crate::app) roots: BTreeSet<String>,
}

impl OutsideTree {
    /// Arrange a run's outside references into folders.
    pub(in crate::app) fn build(references: &[OutsideReference]) -> Self {
        let mut tree = Self::default();
        for reference in references {
            let normalized = reference.display_path.replace('\\', "/");
            let (folder, leaf) = match normalized.rsplit_once('/') {
                Some((folder, leaf)) => (folder.to_owned(), leaf.to_owned()),
                // A tag at the top with no folder of its own still needs one to
                // hang from, and an empty name is what the roots loop skips.
                None => (String::new(), normalized.clone()),
            };
            tree.tags
                .entry(folder.clone())
                .or_default()
                .push((reference.key.clone(), leaf));
            // Every ancestor, so a folder that only holds folders still appears.
            let mut path = folder.as_str();
            loop {
                *tree.totals.entry(path.to_owned()).or_default() += 1;
                match path.rsplit_once('/') {
                    Some((parent, child)) => {
                        tree.folders
                            .entry(parent.to_owned())
                            .or_default()
                            .insert(child.to_owned());
                        path = parent;
                    }
                    None => {
                        if !path.is_empty() {
                            tree.roots.insert(path.to_owned());
                        }
                        break;
                    }
                }
            }
        }
        for tags in tree.tags.values_mut() {
            tags.sort();
        }
        tree
    }

    /// Every tag key at or under `folder`.
    pub(in crate::app) fn keys_under(&self, folder: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut pending = vec![folder.to_owned()];
        while let Some(path) = pending.pop() {
            if let Some(tags) = self.tags.get(&path) {
                out.extend(tags.iter().map(|(key, _)| key.clone()));
            }
            if let Some(children) = self.folders.get(&path) {
                for child in children {
                    pending.push(if path.is_empty() {
                        child.clone()
                    } else {
                        format!("{path}/{child}")
                    });
                }
            }
        }
        out
    }

    /// How many tags under `folder` are ticked, and how many there are.
    pub(in crate::app) fn tally(
        &self,
        folder: &str,
        picked: &BTreeMap<String, bool>,
    ) -> (usize, usize) {
        let keys = self.keys_under(folder);
        let total = keys.len();
        let chosen = keys
            .iter()
            .filter(|key| picked.get(*key).copied().unwrap_or(false))
            .count();
        (chosen, total)
    }
}

/// The one tag a single-tag import is for, and where the user wants it.
pub(in crate::app) struct SingleTagImport {
    /// The cache key, which is what the run converts.
    pub(in crate::app) key: String,
    /// Its path in the build, for the window to name it by.
    pub(in crate::app) display_path: String,
    /// Folder inside the target's tags root, or `None` for the tag's own path.
    pub(in crate::app) destination: Option<PathBuf>,
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
            outside_tree: OutsideTree::default(),
            outside_picked: BTreeMap::new(),
            single: None,
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: None,
            report: None,
            error: None,
        });
    }

    /// Open the window for one tag in the active cache workspace.
    ///
    /// Same window as the folder import, seeded with a single tag and a
    /// destination the user chooses. Sharing the window rather than adding
    /// another keeps one place to read what happened, and a single tag can
    /// still reach for others -- the outside-reference question is the same
    /// question whether one tag asked it or a thousand.
    pub(super) fn open_cache_import_dialog_for_tag(&mut self, key: String) {
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
        let Some(entry) = source_data.entries.iter().find(|entry| entry.key == key) else {
            self.status = "That tag is no longer in this cache".to_owned();
            return;
        };
        let display_path = entry.display_path.clone();
        let targets = self.cache_import_targets();
        if targets.is_empty() {
            self.status =
                "Open the editing kit this tag should land in first — File › Load Folder"
                    .to_owned();
            return;
        }
        self.cache_import_dialog = Some(CacheImportDialog {
            kit,
            prefix: display_path.clone(),
            selected: 1,
            targets,
            target_index: 0,
            outside_tree: OutsideTree::default(),
            outside_picked: BTreeMap::new(),
            single: Some(SingleTagImport {
                key,
                display_path,
                destination: None,
            }),
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
        // A single-tag run converts that tag on the first pass; a second pass
        // for its references is the same as any other, and lands them at their
        // own paths, because that is where the references point.
        let (single_seed, relocate_to) = match (dialog.single.as_ref(), only.is_none()) {
            (Some(single), true) => (
                Some(HashSet::from([single.key.clone()])),
                single.destination.clone(),
            ),
            _ => (None, None),
        };
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
                seed: match (only, single_seed) {
                    (Some(keys), _) | (None, Some(keys)) => CacheSeed::Keys(keys),
                    (None, None) => CacheSeed::Folder,
                },
                relocate_to,
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
                dialog.outside_tree = OutsideTree::build(&report.outside_references);
                dialog.outside_picked = report
                    .outside_references
                    .iter()
                    .map(|reference| (reference.key.clone(), true))
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

#[cfg(test)]
mod outside_tree_tests {
    use super::*;

    fn reference(path: &str) -> OutsideReference {
        OutsideReference {
            key: format!("cache:bitm:{}", path.replace('/', "\\")),
            display_path: path.to_owned(),
            folder: String::new(),
        }
    }

    /// A folder holds what is under it, however deep that is.
    ///
    /// The point of the tree over the flat list it replaced: `objects` counting
    /// only the tags directly inside it would report two thousand tags as none,
    /// and ticking it would bring nothing.
    #[test]
    fn a_folder_counts_and_carries_its_whole_subtree() {
        let tree = OutsideTree::build(&[
            reference("objects/characters/elite/elite.biped"),
            reference("objects/characters/elite/bitmaps/elite_diffuse.bitmap"),
            reference("objects/weapons/rifle/assault_rifle.weapon"),
            reference("fx/decals/scorch.bitmap"),
        ]);

        assert_eq!(tree.roots.iter().cloned().collect::<Vec<_>>(), ["fx", "objects"]);
        assert_eq!(tree.totals.get("objects").copied(), Some(3));
        assert_eq!(tree.totals.get("objects/characters/elite").copied(), Some(2));
        assert_eq!(tree.keys_under("objects").len(), 3);
        assert_eq!(tree.keys_under("objects/weapons").len(), 1);
        // The leaves hang off the folder that actually holds them, not off the
        // first two segments the old grouping used.
        assert_eq!(
            tree.tags
                .get("objects/characters/elite")
                .map(|tags| tags.len()),
            Some(1),
        );
    }

    /// A folder's tick reports how much of it is chosen, not just whether any is.
    #[test]
    fn a_partly_chosen_folder_says_so() {
        let references = [
            reference("objects/characters/elite/elite.biped"),
            reference("objects/characters/elite/bitmaps/elite_diffuse.bitmap"),
            reference("objects/weapons/rifle/assault_rifle.weapon"),
        ];
        let tree = OutsideTree::build(&references);
        let mut picked: BTreeMap<String, bool> =
            references.iter().map(|r| (r.key.clone(), false)).collect();

        assert_eq!(tree.tally("objects", &picked), (0, 3));
        picked.insert(references[0].key.clone(), true);
        assert_eq!(tree.tally("objects", &picked), (1, 3));
        assert_eq!(tree.tally("objects/characters", &picked), (1, 2));
        assert_eq!(tree.tally("objects/weapons", &picked), (0, 1));
        for wanted in picked.values_mut() {
            *wanted = true;
        }
        assert_eq!(tree.tally("objects", &picked), (3, 3));
    }

    /// A tag with no folder still lands somewhere the window can draw it.
    #[test]
    fn a_tag_at_the_top_is_not_lost() {
        let tree = OutsideTree::build(&[reference("globals.globals")]);
        assert!(tree.roots.is_empty());
        assert_eq!(tree.tags.get("").map(|tags| tags.len()), Some(1));
        assert_eq!(tree.keys_under("").len(), 1);
    }
}
