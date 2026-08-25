//! The cross-game conversion worker: walk a folder of tags, convert each, report back.
//!
//! What a tag *becomes* in another game — group routing, struct pairing, field
//! matching, value translation, companion synthesis — lives in
//! `blam_tags::convert`. The dialog that drives this is `import.rs`; this module
//! owns only the job itself and the shape of what it reports.

use super::*;
use std::collections::{BTreeSet, VecDeque};

use crate::app::controller::collect_tag_dependency_refs;
// Re-exported, not merely imported: `app.rs` pulls this module in with
// `use conversion::*`, and the controller, dialogs and document state all reach the
// conversion types through that one path. A private `use` would resolve here and
// nowhere else.
pub(in crate::app) use blam_tags::convert::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum FolderConversionFileStatus {
    NativeLayout,
    GeneratedLayout,
    Failed,
}

pub(in crate::app) struct FolderConversionFileResult {
    pub(in crate::app) source: String,
    pub(in crate::app) output: Option<PathBuf>,
    pub(in crate::app) status: FolderConversionFileStatus,
    pub(in crate::app) overwritten: bool,
    pub(in crate::app) detail: String,
}

/// A tag the run declined to write because it would lose audited data.
///
/// Held rather than failed: it converts, and somebody who has been shown what
/// goes missing may well still want it. Carries the loss so the question can be
/// asked with the answer in front of them.
pub(in crate::app) struct FolderConversionHeldBack {
    pub(in crate::app) source: String,
    /// The source tag's own identity, so a second pass can be restricted to
    /// exactly these. A key rather than a path because a cache tag has no path
    /// — and `TagEntry::key` is the identity every other cache in the
    /// application already keys off.
    pub(in crate::app) key: String,
    pub(in crate::app) losses: Vec<String>,
}

pub(in crate::app) struct FolderConversionReport {
    /// The folder the tags were read from. Kept in the report because a reader
    /// asking "where did these come from?" cannot get it from anywhere else —
    /// each row's `source` is relative to the source's own root, not to this.
    pub(in crate::app) source_root: PathBuf,
    pub(in crate::app) source_game: String,
    pub(in crate::app) target_game: String,
    pub(in crate::app) destination_root: PathBuf,
    pub(in crate::app) files: Vec<FolderConversionFileResult>,
    pub(in crate::app) ignored_files: Vec<String>,
    /// Tags that convert but were not written, pending a decision about the
    /// data they lose. Empty once the user has accepted.
    pub(in crate::app) held_back: Vec<FolderConversionHeldBack>,
    /// Tags the converted set reaches that this run did not convert.
    ///
    /// The question the dialog asks afterwards. Empty means the folder stands
    /// on its own, which is the only case where a folder import is finished
    /// when the run is.
    pub(in crate::app) outside_references: Vec<OutsideReference>,
    /// References that name a tag the source does not hold, deduped.
    ///
    /// Reported rather than skipped, because it is the one kind of dangling
    /// reference the import did not cause: the tag was already missing where
    /// these came from. Saying nothing would leave the user hunting a hole in
    /// the destination for a hole that was in the source.
    pub(in crate::app) unresolved_references: BTreeSet<String>,
    /// Levels whose baked lighting did not come across, by folder.
    ///
    /// Its own list because of what it costs. A level's
    /// `scenario_lightmap_bsp_data` is what the engine loads its lighting from,
    /// and a bsp whose lightmap will not load is one the environment reports as
    /// having *failed to load completely* -- Sapien refuses the whole scenario
    /// over it, with an error that names the bsp and says nothing about lighting.
    /// Most of a 2011 build's lightmaps were never in it, so this is the
    /// difference between a level that runs and one that cannot, and it needs
    /// saying in those words rather than as one failed tag among hundreds.
    pub(in crate::app) levels_without_lighting: BTreeSet<String>,
    /// Whether the user stopped the run. A cancelled report is still a real
    /// report — everything it lists was written — but it is not a complete
    /// one, and saying so is the difference between a short run and a silently
    /// truncated one.
    pub(in crate::app) cancelled: bool,
}

impl FolderConversionReport {
    pub(in crate::app) fn native_count(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.status == FolderConversionFileStatus::NativeLayout)
            .count()
    }

    pub(in crate::app) fn generated_count(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.status == FolderConversionFileStatus::GeneratedLayout)
            .count()
    }

    pub(in crate::app) fn failed_count(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.status == FolderConversionFileStatus::Failed)
            .count()
    }

    pub(in crate::app) fn converted_count(&self) -> usize {
        self.native_count() + self.generated_count()
    }
}

/// Which tags a run starts from, and where they land.
///
/// The two sources answer "what is in this folder?" and "where does it go?"
/// differently enough that one set of fields could only serve both by leaving
/// half of them meaningless for whichever source is not in play.
pub(in crate::app) enum FolderConversionScope {
    /// Everything on disk under `source_rel_path`, which is relative to the
    /// loose kit's own root. The destination is chosen by the user, so the
    /// converted tree mirrors the source's shape under a folder they named.
    LooseSubtree {
        source_rel_path: PathBuf,
        /// The leaf folder the converted tags land in, under
        /// `destination_parent`. Named for the destination rather than the
        /// source because that is what it decides: it is free to differ from
        /// the source folder's own name.
        destination_label: String,
        destination_parent: PathBuf,
    },
    /// The cache tags whose name starts with `prefix`.
    ///
    /// These land at their **own** path under the target's tags root, not at a
    /// destination the user picks, and that is the whole reason references
    /// survive: `convert_reference` carries a reference's path string across
    /// untouched, so a tag written where the source said it lived is a tag every
    /// other tag already points at correctly.
    CacheSubtree {
        /// `""` for the whole cache.
        prefix: String,
        /// Every tag in the cache, not just the ones being converted. The rest
        /// is the index a reference resolves through, so working out what the
        /// run left behind does not mean going back to the kit from a worker
        /// thread.
        entries: Vec<TagEntry>,
        /// Which of `entries` this run converts.
        seed: CacheSeed,
        /// Where the tags land, when the user has chosen somewhere other than
        /// their own path. A folder relative to the target's tags root.
        ///
        /// Only ever set for a single tag, and the dialog says what it costs:
        /// every reference to this tag names the path the build gave it, and
        /// nothing rewrites those, so a tag moved somewhere else is a tag
        /// nothing points at any more.
        relocate_to: Option<PathBuf>,
    },
}

/// Which cache tags a run converts.
///
/// A run never follows a reference on its own. It converts what it was given,
/// then reports what that turned out to need, and the user decides whether a
/// second run brings those too. Following silently was the old behaviour and it
/// meant asking for one folder and getting several thousand tags.
pub(in crate::app) enum CacheSeed {
    /// Everything under the scope's `prefix`.
    Folder,
    /// Exactly these [`TagEntry::key`]s — the second run, after the user has
    /// seen what the folder reached for and picked which of it to bring.
    Keys(HashSet<String>),
}

/// A tag a run needed but did not convert.
///
/// Not an error, and not necessarily something to fix: a folder that references
/// a shader two directories away is a normal folder. It is a question, and the
/// dialog asks it.
pub(in crate::app) struct OutsideReference {
    pub(in crate::app) key: String,
    pub(in crate::app) display_path: String,
    /// Where it is grouped in the dialog — the first two path segments, or the
    /// first if that is all there is. Chosen so the question stays answerable:
    /// a character folder can reach a couple of thousand tags, and nobody reads
    /// a list that long, but they will read forty folder names with counts.
    pub(in crate::app) folder: String,
}

pub(in crate::app) struct FolderConversionJob {
    pub(in crate::app) source: TagSource,
    pub(in crate::app) names: TagNameIndex,
    pub(in crate::app) scope: FolderConversionScope,
    pub(in crate::app) source_game: String,
    pub(in crate::app) target_game: String,
    pub(in crate::app) target_tags_root: PathBuf,
    /// Every configured editing kit, so a tag the direct pair refuses can be
    /// routed through the engines in between. Only consulted when that happens:
    /// indexing a kit walks its whole tag tree.
    pub(in crate::app) kit_roots: HashMap<String, PathBuf>,
    /// Write tags that lose audited data instead of holding them back. Set only
    /// for the second pass, after the user has seen what goes missing.
    pub(in crate::app) accept_loss: bool,
    /// Restrict the run to these source tags, by [`TagEntry::key`]. `None`
    /// converts the whole folder; the second pass names exactly the tags that
    /// were held back, so accepting the loss does not mean redoing everything.
    pub(in crate::app) only: Option<HashSet<String>>,
    /// Set from the UI thread by the Cancel button. Checked between tags, which
    /// is as fine-grained as this can be: a single `scenario_structure_bsp` is
    /// minutes of work with no safe point inside it.
    pub(in crate::app) cancel: Arc<AtomicBool>,
}

/// What happened to one file in a folder run.
///
/// A third state was needed: a tag held back over data loss has not failed — it
/// converts — and counting it as a failure would tell the user something untrue
/// about a tag they are about to be offered.
enum FileOutcome {
    Written(FolderConversionFileResult),
    HeldBack(Vec<String>),
}

fn send_folder_conversion_progress(
    tx: &Sender<WorkerMessage>,
    scope: &FolderConversionScope,
    phase: &str,
    current: &str,
    processed: usize,
    total: usize,
    converted: usize,
    failed: usize,
) {
    let progress = FolderConversionProgress {
        phase: phase.to_owned(),
        current: current.to_owned(),
        processed,
        total,
        converted,
        failed,
    };
    // Addressed by scope rather than by whichever dialog happens to be up: the
    // two imports have separate windows, and nothing stops both running.
    let _ = tx.send(match scope {
        FolderConversionScope::LooseSubtree { .. } => {
            WorkerMessage::FolderConversionProgress(progress)
        }
        FolderConversionScope::CacheSubtree { .. } => WorkerMessage::CacheImportProgress(progress),
    });
}

/// Read the tags a run starts from.
///
/// Returns the seed entries, the files that were not tags, and a row for each
/// one that could not be identified — those are reported rather than fatal,
/// because one unreadable file in a kit should not cost the other 96,000.
fn scan_folder_conversion_source(
    job: &FolderConversionJob,
    tx: &Sender<WorkerMessage>,
) -> Result<(Vec<TagEntry>, Vec<String>, Vec<FolderConversionFileResult>), String> {
    let mut entries = Vec::new();
    let mut ignored_files = Vec::new();
    let mut scan_failures = Vec::new();
    match &job.scope {
        FolderConversionScope::LooseSubtree {
            source_rel_path, ..
        } => {
            let TagSource::LooseFolder { root, .. } = &job.source else {
                return Err("Folder conversion requires a loose-folder source".to_owned());
            };
            let source_folder = normalize_conversion_path(&root.join(source_rel_path));
            let mut disk_paths = Vec::new();
            for item in walkdir::WalkDir::new(&source_folder).follow_links(false) {
                let item = item.map_err(|error| {
                    format!("Could not scan {}: {error}", source_folder.display())
                })?;
                if item.file_type().is_file() {
                    disk_paths.push(item.into_path());
                }
            }
            disk_paths.sort();
            let total_files = disk_paths.len();
            for (index, path) in disk_paths.iter().enumerate() {
                let display = path
                    .strip_prefix(root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/");
                match loose_file_entry(root, path, &job.names) {
                    Ok(Some(entry)) => entries.push(entry),
                    Ok(None) => ignored_files.push(display.clone()),
                    Err(error) => scan_failures.push(FolderConversionFileResult {
                        source: display.clone(),
                        output: None,
                        status: FolderConversionFileStatus::Failed,
                        overwritten: false,
                        detail: format!("Could not identify tag: {error:#}"),
                    }),
                }
                send_folder_conversion_progress(
                    tx,
                    &job.scope,
                    "Scanning source folder",
                    &display,
                    index + 1,
                    total_files,
                    0,
                    scan_failures.len(),
                );
            }
        }
        FolderConversionScope::CacheSubtree {
            prefix,
            entries: all,
            relocate_to: _,
            seed,
        } => {
            if !matches!(job.source, TagSource::MonolithicCache { .. }) {
                return Err("Cache conversion requires a monolithic-cache source".to_owned());
            }
            // Nothing to probe or reject: a cache holds tags and only tags, so
            // there is no `ignored_files` to fill and nothing to fail on.
            entries.extend(
                all.iter()
                    .filter(|entry| match seed {
                        CacheSeed::Folder => cache_entry_is_under(entry, prefix),
                        CacheSeed::Keys(keys) => keys.contains(&entry.key),
                    })
                    .cloned(),
            );
            send_folder_conversion_progress(
                tx,
                &job.scope,
                "Scanning source folder",
                prefix,
                entries.len(),
                entries.len(),
                0,
                0,
            );
        }
    }
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    Ok((entries, ignored_files, scan_failures))
}

/// Whether a cache entry sits at or beneath `prefix`.
///
/// Compared segment-wise rather than as a plain string prefix, so picking
/// `objects\weapons\rifle` does not also drag in `objects\weapons\rifleman`.
/// An empty prefix is the whole cache.
pub(in crate::app) fn cache_entry_is_under(entry: &TagEntry, prefix: &str) -> bool {
    let TagEntryLocation::Monolithic { name, .. } = &entry.location else {
        return false;
    };
    if prefix.is_empty() {
        return true;
    }
    let name = name.replace('/', "\\").to_ascii_lowercase();
    let prefix = prefix.replace('/', "\\").to_ascii_lowercase();
    let prefix = prefix.trim_end_matches('\\');
    name == prefix || name.starts_with(&format!("{prefix}\\"))
}

pub(in crate::app) fn run_folder_conversion_job(
    job: FolderConversionJob,
    tx: &Sender<WorkerMessage>,
) -> Result<FolderConversionReport, String> {
    let target_tags_root = normalize_conversion_path(&job.target_tags_root);
    // Where the run reads from and writes to, said once. The cache case has no
    // choice about either: it reads a name prefix, and it writes at the tags
    // root, because landing a tag anywhere else is what breaks its references.
    let (source_root, destination_root) = match &job.scope {
        FolderConversionScope::LooseSubtree {
            source_rel_path,
            destination_label,
            destination_parent,
        } => {
            let TagSource::LooseFolder { root, .. } = &job.source else {
                return Err("Folder conversion requires a loose-folder source".to_owned());
            };
            (
                normalize_conversion_path(&root.join(source_rel_path)),
                normalize_conversion_path(&destination_parent.join(destination_label)),
            )
        }
        FolderConversionScope::CacheSubtree { prefix, .. } => {
            (PathBuf::from(prefix), target_tags_root.clone())
        }
    };
    if !destination_root.starts_with(&target_tags_root) {
        return Err("Folder conversion destination escapes the target tags folder".to_owned());
    }

    let (entries, ignored_files, scan_failures) = scan_folder_conversion_source(&job, tx)?;

    let definitions_root = locate_definitions_root();
    let source_groups = GameTagIndex::load(&definitions_root, &job.source_game)?;
    // What each source class becomes in the target, asked once per class rather
    // than once per tag: the answer depends only on the group, and getting it can
    // mean walking a route. The engine answers, because a folder run naming its
    // own output by canonical name disagrees with the converter exactly where a
    // class was renamed — which is every contrail_system into Halo 4, and every
    // shader into a game that still declares `shader` and ships none.
    //
    // Filled as classes turn up rather than up front, because following
    // references brings in classes the selected folder never held.
    let mut landing_groups: HashMap<u32, Result<(u32, String), String>> = HashMap::new();
    send_folder_conversion_progress(
        tx,
        &job.scope,
        "Indexing native target layouts",
        "",
        0,
        entries.len(),
        0,
        scan_failures.len(),
    );
    // The cache is threaded through the whole run: one index per engine the
    // tags pass through, built once. Seeded with the destination, which every
    // tag needs; anything else is built only if a tag turns out to need routing.
    let mut templates = NativeTemplateCache::default();
    templates.ensure(&job.target_game, &target_tags_root, &definitions_root);
    let mut kit_roots = job.kit_roots.clone();
    kit_roots.insert(job.target_game.clone(), target_tags_root.clone());

    // Only the loose path can collide: two tags in one folder whose classes land
    // on the same extension. A cache tag's destination is its own name plus its
    // own class's extension, which no other tag in the cache shares.
    let mut destination_counts = HashMap::<String, usize>::new();
    if matches!(job.scope, FolderConversionScope::LooseSubtree { .. }) {
        for entry in &entries {
            let landed = landing_group_for(
                entry,
                &source_groups,
                &job,
                &definitions_root,
                &mut landing_groups,
            );
            let Ok(path) = target_destination_for_entry(
                entry,
                &job.scope,
                &source_root,
                &destination_root,
                landed,
            ) else {
                continue;
            };
            let key = normalize_conversion_path(&path)
                .to_string_lossy()
                .to_ascii_lowercase();
            *destination_counts.entry(key).or_default() += 1;
        }
    }

    // The index a reference resolves through. Built for a cache run whatever
    // it converts, because working out what the run left behind needs it.
    let by_key: HashMap<String, TagEntry> = match &job.scope {
        FolderConversionScope::CacheSubtree { entries: all, .. } => all
            .iter()
            .filter_map(|entry| match &entry.location {
                TagEntryLocation::Monolithic { name, group_tag } => {
                    Some((folded_cache_key(*group_tag, name), entry.clone()))
                }
                _ => None,
            })
            .collect(),
        _ => HashMap::new(),
    };
    let mut queue: VecDeque<TagEntry> = entries
        .into_iter()
        .filter(|entry| {
            job.only
                .as_ref()
                .is_none_or(|only| only.contains(&entry.key))
        })
        .collect();
    let converting: HashSet<String> = queue.iter().map(|entry| entry.key.clone()).collect();
    // What the converted tags turned out to need, gathered as they are read
    // so nothing is parsed twice for it.
    let wants_outside_report = matches!(job.scope, FolderConversionScope::CacheSubtree { .. });
    let mut reached: Vec<DependencyRef> = Vec::new();
    let selected = queue.len();

    let mut report = FolderConversionReport {
        source_root,
        source_game: job.source_game.clone(),
        target_game: job.target_game.clone(),
        destination_root: destination_root.clone(),
        files: scan_failures,
        ignored_files,
        held_back: Vec::new(),
        outside_references: Vec::new(),
        unresolved_references: BTreeSet::new(),
        levels_without_lighting: BTreeSet::new(),
        cancelled: false,
    };
    // Collected on the side because a held-back tag is neither a success nor a
    // failure: it converted, and it is waiting on an answer.
    let mut held_back: Vec<FolderConversionHeldBack> = Vec::new();
    let mut converted = 0;
    let mut failed = report.files.len();
    let mut processed = 0;
    let mut claimed_outputs = HashSet::<String>::new();
    while let Some(entry) = queue.pop_front() {
        if job.cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let source_label = entry.display_path.clone();
        // Filled by the closure below whether the tag converted or not: a tag
        // that failed, and a tag held back pending an answer, both still name
        // the tags they need, and the held-back one may yet be accepted.
        let mut discovered: Vec<DependencyRef> = Vec::new();
        let landed = landing_group_for(
            &entry,
            &source_groups,
            &job,
            &definitions_root,
            &mut landing_groups,
        );
        let destination = target_destination_for_entry(
            &entry,
            &job.scope,
            &report.source_root,
            &destination_root,
            landed,
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut output = destination?;
            let key = normalize_conversion_path(&output)
                .to_string_lossy()
                .to_ascii_lowercase();
            if destination_counts.get(&key).copied().unwrap_or(0) > 1 {
                return Err(format!(
                    "Multiple source tags map to the same destination: {}",
                    output.display()
                ));
            }
            let source_tag = read_entry(&job.source, &entry)
                .map_err(|error| format!("Could not read source tag: {error:#}"))?;
            if wants_outside_report {
                collect_tag_dependency_refs(source_tag.root(), &mut discovered);
            }
            let mut draft = match convert_tag_outcome(
                &source_tag,
                &job.source_game,
                &job.target_game,
                &definitions_root,
                &kit_roots,
                &mut templates,
            ) {
                ConversionOutcome::Clean(draft) => draft,
                // Converts, but gives up audited data. Nothing is written until
                // somebody has seen what goes and said to go ahead.
                ConversionOutcome::Lossy { draft, .. } if !job.accept_loss => {
                    return Ok(FileOutcome::HeldBack(
                        draft.report.fail_closed_losses.clone(),
                    ));
                }
                ConversionOutcome::Lossy { draft, .. } => *draft,
                ConversionOutcome::Failed(error) => return Err(error),
            };
            // The planned name was a prediction; this is what the tag turned out
            // to be. The two agree unless the direct pair was refused and the
            // route renamed the class further than the direct pair would have, and
            // a file whose extension disagrees with its own group header is one
            // the destination's tools will not open.
            output.set_extension(&draft.target_extension);
            let dependency_schema = definitions_root
                .join(&job.target_game)
                .join("tag_dependency_list.json");
            let companion_outputs = prepare_companion_outputs(
                &mut draft,
                &output,
                &target_tags_root,
                &dependency_schema,
            )?;
            let all_outputs = std::iter::once(&output)
                .chain(companion_outputs.iter())
                .collect::<Vec<_>>();
            let mut local_outputs = HashSet::new();
            for path in &all_outputs {
                let key = normalize_conversion_path(path)
                    .to_string_lossy()
                    .to_ascii_lowercase();
                if !local_outputs.insert(key.clone()) || claimed_outputs.contains(&key) {
                    return Err(format!(
                        "Multiple generated tags map to the same destination: {}",
                        path.display()
                    ));
                }
            }
            claimed_outputs.extend(local_outputs);
            let overwritten = all_outputs.iter().any(|path| path.exists());
            let native_layout_template = draft.native_layout_template.clone();
            let route = draft.route.clone();
            let conversion_details = draft
                .report
                .issues
                .iter()
                .map(|issue| {
                    let kind = match issue.kind {
                        ConversionIssueKind::Unsupported => "unsupported",
                        ConversionIssueKind::Truncated => "truncated",
                        ConversionIssueKind::Warning => "warning",
                    };
                    format!("{kind}: {} — {}", issue.path, issue.message)
                })
                .collect::<Vec<_>>();
            for path in &all_outputs {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("Could not create {}: {error}", parent.display())
                    })?;
                }
            }
            for (companion, path) in draft.companion_tags.iter().zip(&companion_outputs) {
                companion
                    .tag
                    .write_atomic(path)
                    .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
            }
            draft
                .tag
                .write_atomic(&output)
                .map_err(|error| format!("Could not save {}: {error}", output.display()))?;
            let status = if native_layout_template.is_some() {
                FolderConversionFileStatus::NativeLayout
            } else {
                FolderConversionFileStatus::GeneratedLayout
            };
            let mut detail = if let Some(template) = native_layout_template {
                format!("Started from kit tag: {}", template.display())
            } else {
                "Built from the target profile's own definitions".to_owned()
            };
            if !route.is_empty() {
                // Named per file, not once per run: a folder can hold a mix, and
                // "some of these took a detour" is not an answer.
                detail.push_str(&format!(" | routed via {}", route.join(" -> ")));
            }
            if !conversion_details.is_empty() {
                detail.push_str(" | ");
                detail.push_str(&conversion_details.join("; "));
            }
            if !companion_outputs.is_empty() {
                detail.push_str(" | generated companions: ");
                detail.push_str(
                    &companion_outputs
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            Ok(FileOutcome::Written(FolderConversionFileResult {
                source: source_label.clone(),
                output: Some(output),
                status,
                overwritten,
                detail,
            }))
        }))
        .unwrap_or_else(|payload| {
            let detail = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    payload
                        .downcast_ref::<&str>()
                        .map(|text| (*text).to_owned())
                })
                .unwrap_or_else(|| "unknown panic payload".to_owned());
            Err(format!("Conversion panicked: {detail}"))
        });

        // Banked, not acted on. What this tag needs is part of the answer the
        // run reports at the end, and a tag that refused to convert still
        // names what it would have needed.
        reached.append(&mut discovered);
        processed += 1;
        let total = processed + queue.len();

        let file_result = match result {
            Ok(FileOutcome::Written(result)) => {
                converted += 1;
                result
            }
            // Neither converted nor failed: it is waiting on an answer, so it
            // stays out of both counts and out of the file list.
            Ok(FileOutcome::HeldBack(losses)) => {
                held_back.push(FolderConversionHeldBack {
                    source: source_label.clone(),
                    key: entry.key.clone(),
                    losses,
                });
                send_folder_conversion_progress(
                    tx,
                    &job.scope,
                    "Converting tags",
                    &source_label,
                    processed,
                    total,
                    converted,
                    failed,
                );
                continue;
            }
            Err(error) => {
                failed += 1;
                if entry.group_tag == u32::from_be_bytes(*b"Lbsp")
                    && let TagEntryLocation::Monolithic { name, .. } = &entry.location
                {
                    let folder = name
                        .rsplit_once(['\\', '/'])
                        .map(|(folder, _)| folder.to_owned())
                        .unwrap_or_else(|| name.clone());
                    report.levels_without_lighting.insert(folder);
                }
                FolderConversionFileResult {
                    source: source_label.clone(),
                    output: None,
                    status: FolderConversionFileStatus::Failed,
                    overwritten: false,
                    detail: error,
                }
            }
        };
        let status_label = match file_result.status {
            FolderConversionFileStatus::NativeLayout => "native",
            FolderConversionFileStatus::GeneratedLayout => "generated",
            FolderConversionFileStatus::Failed => "failed",
        };
        let _ = tx.send(WorkerMessage::TerminalLine(format!(
            "[folder conversion/{status_label}] {}: {}",
            file_result.source, file_result.detail
        )));
        report.files.push(file_result);
        send_folder_conversion_progress(
            tx,
            &job.scope,
            "Converting tags",
            &source_label,
            processed,
            total,
            converted,
            failed,
        );
    }
    if wants_outside_report && !report.cancelled {
        let (outside, unresolved) = discover_outside_references(
            &job,
            &by_key,
            &converting,
            std::mem::take(&mut reached),
            &destination_root,
            &mut landing_groups,
            &source_groups,
            &definitions_root,
            tx,
        );
        report.outside_references = outside;
        report.unresolved_references = unresolved;
    }
    report
        .files
        .sort_by(|left, right| left.source.cmp(&right.source));
    held_back.sort_by(|left, right| left.source.cmp(&right.source));
    report.held_back = held_back;
    let _ = tx.send(WorkerMessage::TerminalLine(format!(
        "Folder conversion {}: {} converted, {} failed, {} held back, {} ignored, {} tag(s) \
         reached outside the {selected} selected, {} reference(s) missing from the source",
        if report.cancelled { "cancelled" } else { "complete" },
        report.converted_count(),
        report.failed_count(),
        report.held_back.len(),
        report.ignored_files.len(),
        report.outside_references.len(),
        report.unresolved_references.len(),
    )));
    Ok(report)
}

/// Everything the converted tags reach that this run did not convert.
///
/// Walked transitively, so the answer is the whole of what is missing rather
/// than one layer of it. That matters for the question it feeds: bringing the
/// tags a folder names directly, and no further, leaves those new arrivals
/// dangling in turn, and the user would be asked again and again.
///
/// A tag already sitting at its destination is not missing, so a second run
/// after the user accepted the first answer does not ask about the same tags
/// twice.
///
/// Reading each one costs a parse. That is the price of an honest count, and it
/// is far cheaper than converting them, which is the alternative the user is
/// being asked about.
fn discover_outside_references(
    job: &FolderConversionJob,
    by_key: &HashMap<String, TagEntry>,
    converted: &HashSet<String>,
    seed_refs: Vec<DependencyRef>,
    destination_root: &Path,
    landing: &mut HashMap<u32, Result<(u32, String), String>>,
    source_groups: &GameTagIndex,
    definitions_root: &Path,
    tx: &Sender<WorkerMessage>,
) -> (Vec<OutsideReference>, BTreeSet<String>) {
    let mut unresolved = BTreeSet::new();
    let mut found: Vec<OutsideReference> = Vec::new();
    let mut seen: HashSet<String> = converted.clone();
    let mut queue: VecDeque<DependencyRef> = seed_refs.into();
    while let Some(reference) = queue.pop_front() {
        if job.cancel.load(Ordering::Relaxed) {
            break;
        }
        let Some(entry) = by_key.get(&folded_cache_key(reference.group_tag, &reference.rel_path))
        else {
            // Already broken where it came from — the 2011 build names tags that
            // were deleted before it shipped. Recorded so the hole in the
            // destination has an explanation that is not this import.
            unresolved.insert(format!(
                "{}.{}",
                reference.rel_path.replace('\\', "/"),
                group_tag_to_extension(reference.group_tag).unwrap_or("unknown")
            ));
            continue;
        };
        if !seen.insert(entry.key.clone()) {
            continue;
        }
        // Already in the kit, from an earlier run or from the kit's own tags.
        let landed = landing_group_for(entry, source_groups, job, definitions_root, landing);
        let existing = target_destination_for_entry(
            entry,
            &job.scope,
            Path::new(""),
            destination_root,
            landed,
        );
        if existing.as_ref().is_ok_and(|path| path.exists()) {
            continue;
        }
        found.push(OutsideReference {
            key: entry.key.clone(),
            display_path: entry.display_path.clone(),
            folder: grouping_folder(&entry.display_path),
        });
        send_folder_conversion_progress(
            tx,
            &job.scope,
            "Finding tags outside the folder",
            &entry.display_path,
            found.len(),
            found.len() + queue.len(),
            0,
            0,
        );
        // Keep walking through it, whether or not the user ends up bringing it:
        // what *it* needs is part of the same answer.
        if let Ok(tag) = read_entry(&job.source, entry) {
            let mut refs = Vec::new();
            collect_tag_dependency_refs(tag.root(), &mut refs);
            queue.extend(refs);
        }
    }
    found.sort_by(|left, right| left.display_path.cmp(&right.display_path));
    (found, unresolved)
}

/// The folder a tag is offered under in the dialog.
///
/// Two segments, because one is too coarse to choose by — `objects` covers
/// most of a build — and the full parent path is too many rows to read.
fn grouping_folder(display_path: &str) -> String {
    let normalized = display_path.replace('\\', "/");
    let mut segments = normalized.split('/');
    match (segments.next(), segments.next(), segments.next()) {
        (Some(first), Some(second), Some(_)) => format!("{first}/{second}"),
        (Some(first), Some(_), None) | (Some(first), None, None) => first.to_owned(),
        _ => normalized,
    }
}

/// A cache key, folded so a reference and an entry that name the same tag agree.
///
/// The shape is the one `load_monolithic_blob_index` builds, with two things
/// forced: separators to backslashes, and case down. Tag paths inside a tag are
/// written by whoever authored it, so the same tag turns up as
/// `objects\Weapons\rifle` in one reference and `objects\weapons\rifle` in
/// the next, and a case-sensitive lookup would follow one and drop the other.
fn folded_cache_key(group_tag: u32, name: &str) -> String {
    format!(
        "cache:{}:{}",
        format_group_tag(group_tag),
        name.replace('/', "\\").to_ascii_lowercase()
    )
}

/// What class this entry lands in, asked once per class and remembered.
///
/// Memoised on the way through rather than filled in up front, because
/// following references brings in classes the selected folder never held.
fn landing_group_for<'a>(
    entry: &TagEntry,
    source_groups: &GameTagIndex,
    job: &FolderConversionJob,
    definitions_root: &Path,
    memo: &'a mut HashMap<u32, Result<(u32, String), String>>,
) -> &'a Result<(u32, String), String> {
    memo.entry(entry.group_tag).or_insert_with(|| {
        match source_groups.by_tag.get(&entry.group_tag) {
            Some(name) => {
                converted_group(name, &job.source_game, &job.target_game, definitions_root)
                    .and_then(|found| {
                        found.ok_or_else(|| format!("Target profile has no {name} group"))
                    })
            }
            None => Err(format!(
                "Unknown source group {}",
                format_group_tag(entry.group_tag)
            )),
        }
    })
}

/// Where one source tag is expected to land.
///
/// Expected, not decided: the extension comes from the class the converter is
/// predicted to produce, and the draft has the final say once the tag has
/// actually been through. This runs first only because two sources colliding on
/// one destination has to be caught before either is written.
fn target_destination_for_entry(
    entry: &TagEntry,
    scope: &FolderConversionScope,
    source_root: &Path,
    destination_root: &Path,
    landed: &Result<(u32, String), String>,
) -> Result<PathBuf, String> {
    let relative = match (&entry.location, scope) {
        (TagEntryLocation::LooseFile(source_path), FolderConversionScope::LooseSubtree { .. }) => {
            normalize_conversion_path(source_path)
                .strip_prefix(source_root)
                .map(Path::to_path_buf)
                .map_err(|_| "Source tag escapes the selected folder".to_owned())?
        }
        // The tag's own name, so it lands where the cache said it lived. Not a
        // convenience: every reference in every other tag names this path, and
        // nothing rewrites them.
        (
            TagEntryLocation::Monolithic { name, .. },
            FolderConversionScope::CacheSubtree { relocate_to, .. },
        ) => {
            let own = PathBuf::from(name.replace('\\', "/"));
            match relocate_to {
                // Somewhere the user picked: the tag keeps its own name and
                // nothing else of its path.
                Some(folder) => {
                    let leaf = own
                        .file_name()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| own.clone());
                    folder.join(leaf)
                }
                None => own,
            }
        }
        _ => return Err("Folder conversion cannot place this tag".to_owned()),
    };
    let (target_group, target_group_name) = landed.as_ref().map_err(String::clone)?;
    let extension = group_tag_to_extension(*target_group).unwrap_or(target_group_name);
    let mut output = normalize_conversion_path(&destination_root.join(relative));
    output.set_extension(extension);
    if !output.starts_with(destination_root) {
        return Err("Destination escapes the selected target folder".to_owned());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::TagSource;
    use blam_tags::TagOptions;
    // Both the editor and the engine define this, identically, and two globs make
    // the bare name ambiguous. Name the engine's explicitly: this scaffolding is a
    // copy of the engine's own, so it should key fields the way the engine does.
    use blam_tags::convert::clean_field_key;

    /// A cache tag lands at its own path, unless the user picked one.
    ///
    /// Its own path is what makes a folder import work at all: a reference
    /// carries the path string the build gave it and nothing rewrites those, so
    /// a tag written where the cache said it lived is a tag every other tag
    /// already points at. Somewhere else is the single-tag case, where the
    /// window says what it costs and the user answers anyway.
    #[test]
    fn a_relocated_cache_tag_keeps_its_name_and_nothing_else_of_its_path() {
        let entry = TagEntry {
            key: r"cache:bitm:objects\weapons\rifle\bitmaps\ar_diffuse".to_owned(),
            display_path: r"objects\weapons\rifle\bitmaps\ar_diffuse.bitmap".to_owned(),
            group_tag: u32::from_be_bytes(*b"bitm"),
            group_name: Some("bitmap".to_owned()),
            location: TagEntryLocation::Monolithic {
                name: r"objects\weapons\rifle\bitmaps\ar_diffuse".to_owned(),
                group_tag: u32::from_be_bytes(*b"bitm"),
            },
        };
        let landed = Ok((u32::from_be_bytes(*b"bitm"), "bitmap".to_owned()));
        let destination_root = PathBuf::from("D:/HREK/tags");
        let scope = |relocate_to: Option<PathBuf>| FolderConversionScope::CacheSubtree {
            prefix: String::new(),
            entries: Vec::new(),
            seed: CacheSeed::Folder,
            relocate_to,
        };

        let own = target_destination_for_entry(
            &entry,
            &scope(None),
            Path::new(""),
            &destination_root,
            &landed,
        )
        .expect("its own path");
        assert_eq!(
            normalize_conversion_path(&own),
            normalize_conversion_path(&destination_root
                .join("objects/weapons/rifle/bitmaps/ar_diffuse.bitmap")),
        );

        let moved = target_destination_for_entry(
            &entry,
            &scope(Some(PathBuf::from("scratch/imported"))),
            Path::new(""),
            &destination_root,
            &landed,
        )
        .expect("the folder that was picked");
        assert_eq!(
            normalize_conversion_path(&moved),
            normalize_conversion_path(
                &destination_root.join("scratch/imported/ar_diffuse.bitmap")
            ),
        );
    }

    #[test]
    fn folder_conversion_recurses_overwrites_and_continues_after_failure() {
        let definitions = locate_definitions_root();
        let unique = format!(
            "baboon_folder_conversion_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let source_root = root.join("source_tags");
        let source_folder = source_root.join("characters/jackal");
        let target_tags = root.join("target_tags");
        let destination_parent = target_tags.join("objects/characters");
        fs::create_dir_all(source_folder.join("nested")).unwrap();
        fs::create_dir_all(&target_tags).unwrap();

        let mut source = TagFile::new(definitions.join("halo3_mcc/weapon.json")).unwrap();
        seed_weapon_fields(&mut source);
        apply_editing_kit_mcc_header(&mut source, "halo3_mcc").unwrap();
        source
            .write_atomic(source_folder.join("nested/jackal.weapon"))
            .unwrap();
        source
            .write_atomic(source_folder.join("nested/jackal_alt.weapon"))
            .unwrap();
        fs::write(source_folder.join("notes.txt"), b"not a tag").unwrap();

        let mut bad = TagFile::new(definitions.join("halo3_mcc/light.json")).unwrap();
        apply_editing_kit_mcc_header(&mut bad, "halo3_mcc").unwrap();
        let bad_bytes = bad.write_to_bytes().unwrap();
        fs::write(source_folder.join("broken.light"), &bad_bytes[..64]).unwrap();

        let output = destination_parent.join("jackal/nested/jackal.weapon");
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        let mut existing = TagFile::new(definitions.join("haloreach_mcc/weapon.json")).unwrap();
        apply_editing_kit_mcc_header(&mut existing, "haloreach_mcc").unwrap();
        existing.header.version = 8;
        existing.write_atomic(&output).unwrap();

        let source = TagSource::LooseFolder {
            root: source_root,
            game: Some("halo3_mcc".to_owned()),
            definitions_root: definitions.clone(),
        };
        let names = TagNameIndex::load_from_definitions(&definitions);
        let (tx, _rx) = mpsc::channel();
        let report = run_folder_conversion_job(
            FolderConversionJob {
                source,
                names,
                scope: FolderConversionScope::LooseSubtree {
                    source_rel_path: PathBuf::from("characters/jackal"),
                    destination_label: "jackal".to_owned(),
                    destination_parent: destination_parent.clone(),
                },
                source_game: "halo3_mcc".to_owned(),
                target_game: "haloreach_mcc".to_owned(),
                target_tags_root: target_tags,
                kit_roots: HashMap::new(),
                accept_loss: false,
                only: None,
                cancel: Arc::new(AtomicBool::new(false)),
            },
            &tx,
        )
        .unwrap();

        // The pre-existing Reach weapon written to the output path below is a
        // usable template, and a kit tag still wins over the definitions while
        // the from-definitions path is unproven against the native tools. Set
        // `BLAM_BUILD_FROM_DEFINITIONS` and these two swap.
        assert_eq!(report.native_count(), 2);
        assert_eq!(report.generated_count(), 0);
        assert_eq!(report.failed_count(), 1);
        assert_eq!(report.ignored_files, vec!["characters/jackal/notes.txt"]);
        assert!(report.files.iter().any(|file| {
            file.source == "characters/jackal/nested/jackal.weapon" && file.overwritten
        }));
        let reopened = TagFile::read(&output).unwrap();
        let mut references = Vec::new();
        collect_reference_values(reopened.root(), "", &mut references);
        assert!(references.iter().any(|reference| {
            reference.group_tag == u32::from_be_bytes(*b"bitm")
                && reference.tag_path == "objects\\test\\icon"
        }));
        assert!(
            destination_parent
                .join("jackal/nested/jackal_alt.weapon")
                .is_file()
        );

        fs::remove_dir_all(root).unwrap();
    }

    /// A folder run names a renamed class the way the converter does.
    ///
    /// Halo 4 calls Halo 3's `contrail_system` a `tracer_system`. Importing one
    /// tag worked and importing the folder it sits in failed on the same tag,
    /// because the folder run planned its output by canonical name: Halo 4 has no
    /// `contrail_system` group, so the file could not be named and the tag was
    /// reported as unconvertible before anything tried to convert it.
    ///
    /// The quieter half of the same bug is worse and is why the extension comes
    /// from the draft as well: Halo 4 *does* still declare `shader`, so a Reach
    /// shader was named `.shader` while the converter built a `.material`.
    #[test]
    fn a_folder_run_writes_a_renamed_class_under_its_new_name() {
        let definitions = locate_definitions_root();
        let unique = format!(
            "baboon_renamed_class_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let source_root = root.join("source_tags");
        let source_folder = source_root.join("fx");
        let target_tags = root.join("target_tags");
        let destination_parent = target_tags.join("fx");
        fs::create_dir_all(&source_folder).unwrap();
        fs::create_dir_all(&target_tags).unwrap();

        let mut source = TagFile::new(definitions.join("halo3_mcc/contrail_system.json")).unwrap();
        apply_editing_kit_mcc_header(&mut source, "halo3_mcc").unwrap();
        source
            .write_atomic(source_folder.join("smoke.contrail_system"))
            .unwrap();

        let source = TagSource::LooseFolder {
            root: source_root,
            game: Some("halo3_mcc".to_owned()),
            definitions_root: definitions.clone(),
        };
        let names = TagNameIndex::load_from_definitions(&definitions);
        let (tx, _rx) = mpsc::channel();
        let report = run_folder_conversion_job(
            FolderConversionJob {
                source,
                names,
                scope: FolderConversionScope::LooseSubtree {
                    source_rel_path: PathBuf::from("fx"),
                    destination_label: "fx".to_owned(),
                    destination_parent: destination_parent.clone(),
                },
                source_game: "halo3_mcc".to_owned(),
                target_game: "halo4_mcc".to_owned(),
                target_tags_root: target_tags,
                kit_roots: HashMap::new(),
                accept_loss: true,
                only: None,
                cancel: Arc::new(AtomicBool::new(false)),
            },
            &tx,
        )
        .unwrap();

        let written = destination_parent.join("fx/smoke.tracer_system");
        assert_eq!(
            report.failed_count(),
            0,
            "{:?}",
            report
                .files
                .iter()
                .map(|file| file.detail.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(report.converted_count(), 1);
        assert!(written.is_file(), "expected {}", written.display());
        assert!(!destination_parent.join("fx/smoke.contrail_system").exists());

        fs::remove_dir_all(root).unwrap();
    }

    // Duplicated from the engine's own conversion tests rather than exported:
    // scaffolding that seeds a tag with one of each field kind is test code, and
    // making the engine ship it just so this one worker test can call it would put
    // test-only surface in the published library.
    #[derive(Clone)]
    struct LeafSeed {
        ordinal: usize,
        field_type: TagFieldType,
        option: Option<String>,
    }

    fn first_direct_leaf(tag: &TagFile, wanted: impl Fn(TagFieldType) -> bool) -> LeafSeed {
        tag.root()
            .fields()
            .enumerate()
            .find_map(|(ordinal, field)| {
                wanted(field.field_type()).then(|| {
                    let option = match field.options() {
                        Some(TagOptions::Enum { names, .. }) => {
                            names.get(1).or(names.first()).map(|s| (*s).to_owned())
                        }
                        Some(TagOptions::Flags(options)) => {
                            options.first().map(|option| option.name.to_owned())
                        }
                        None => None,
                    };
                    LeafSeed {
                        ordinal,
                        field_type: field.field_type(),
                        option,
                    }
                })
            })
            .expect("expected direct field type")
    }

    fn seed_weapon_fields(tag: &mut TagFile) {
        let reference =
            first_direct_leaf(tag, |field_type| field_type == TagFieldType::TagReference);
        tag.root_mut()
            .field_at_mut(reference.ordinal)
            .unwrap()
            .set(TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: Some((
                    u32::from_be_bytes(*b"bitm"),
                    "objects\\test\\icon".to_owned(),
                )),
            }))
            .unwrap();

        let real = first_direct_leaf(tag, is_real_scalar);
        tag.root_mut()
            .field_at_mut(real.ordinal)
            .unwrap()
            .set(real_field_value(real.field_type, 0.625))
            .unwrap();

        let enumeration = first_direct_leaf(tag, is_enum_type);
        let enum_name = enumeration.option.unwrap();
        let enum_value = match enumeration.field_type {
            TagFieldType::CharEnum => TagFieldData::CharEnum {
                value: 1,
                name: Some(enum_name),
            },
            TagFieldType::ShortEnum => TagFieldData::ShortEnum {
                value: 1,
                name: Some(enum_name),
            },
            TagFieldType::LongEnum => TagFieldData::LongEnum {
                value: 1,
                name: Some(enum_name),
            },
            _ => unreachable!(),
        };
        tag.root_mut()
            .field_at_mut(enumeration.ordinal)
            .unwrap()
            .set(enum_value)
            .unwrap();

        let flags = first_direct_leaf(tag, is_flags_type);
        let flag_name = flags.option.unwrap();
        let flag_value = match flags.field_type {
            TagFieldType::ByteFlags => TagFieldData::ByteFlags {
                value: 1,
                names: vec![(0, flag_name)],
            },
            TagFieldType::WordFlags => TagFieldData::WordFlags {
                value: 1,
                names: vec![(0, flag_name)],
            },
            TagFieldType::LongFlags => TagFieldData::LongFlags {
                value: 1,
                names: vec![(0, flag_name)],
            },
            _ => unreachable!(),
        };
        tag.root_mut()
            .field_at_mut(flags.ordinal)
            .unwrap()
            .set(flag_value)
            .unwrap();

        let string_id = first_direct_leaf(tag, is_string_id_type);
        let string_value = if string_id.field_type == TagFieldType::StringId {
            TagFieldData::StringId(StringIdData {
                string: "converted-label".to_owned(),
            })
        } else {
            TagFieldData::OldStringId(StringIdData {
                string: "converted-label".to_owned(),
            })
        };
        tag.root_mut()
            .field_at_mut(string_id.ordinal)
            .unwrap()
            .set(string_value)
            .unwrap();

        let magazines = tag
            .root()
            .fields()
            .enumerate()
            .find(|(_, field)| {
                field.field_type() == TagFieldType::Block
                    && clean_field_key(field.name()) == "magazines"
            })
            .map(|(ordinal, _)| ordinal)
            .expect("weapon has magazines block");
        let mut root = tag.root_mut();
        let mut field = root.field_at_mut(magazines).unwrap();
        let mut block = field.as_block_mut().unwrap();
        block.add_element();
    }

    /// A folder import brings the folder, and asks about the rest.
    ///
    /// Two things are checked together because either alone would pass while the
    /// feature was broken. The first run attempts what is under the folder and
    /// nothing else — a folder import that quietly pulled in two thousand tags
    /// is the behaviour this replaced. And what it *reports* has to be complete:
    /// every tag it needed is either written, named in the report, or was
    /// already missing from the build, because that list is the question the
    /// user answers and a gap in it is a gap they never see.
    ///
    /// The destination here is an empty directory, so every tag refuses: a
    /// byte-order upgrade will not build a tag from the schema, and an empty
    /// kit ships no example of anything (see
    /// `blam_tags::convert::analyze_conversion_inner`). That is deliberate. It
    /// keeps the test off the user's real kit, and the scope and reporting this
    /// is about happen either way — a tag names what it needs whether or not
    /// it converts.
    #[test]
    fn a_cache_folder_import_converts_the_folder_and_reports_what_it_reaches() {
        let (Some(cache_root), definitions) = (reach_x360_cache_root(), locate_definitions_root())
        else {
            eprintln!("skipping: needs BABOON_REACH_X360_CACHE");
            return;
        };
        let names = TagNameIndex::load_from_definitions(&definitions);
        let loaded = match crate::source::load_monolithic_blob_index(
            cache_root.join("blob_index.dat"),
            &names,
        ) {
            Ok(loaded) => loaded,
            Err(error) => {
                eprintln!("skipping: could not open the cache: {error}");
                return;
            }
        };
        // A weapon is the useful seed: small, and it reaches a model, a
        // collision model and a set of shaders without dragging in a scenario.
        let Some(seed) = loaded
            .entries
            .iter()
            .find(|entry| entry.group_tag == u32::from_be_bytes(*b"weap"))
        else {
            eprintln!("skipping: no weapon in the cache");
            return;
        };
        let TagEntryLocation::Monolithic { name, .. } = &seed.location else {
            unreachable!("a cache entry is monolithic");
        };
        let prefix = name
            .rsplit_once('\\')
            .map(|(folder, _)| folder.to_owned())
            .unwrap_or_default();
        let folder_keys: HashSet<String> = loaded
            .entries
            .iter()
            .filter(|entry| cache_entry_is_under(entry, &prefix))
            .map(|entry| entry.key.clone())
            .collect();

        let target_tags = std::env::temp_dir().join(format!(
            "baboon_cache_import_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&target_tags).unwrap();

        let run = |seed: CacheSeed| {
            let (tx, _rx) = mpsc::channel();
            run_folder_conversion_job(
                FolderConversionJob {
                    source: loaded.source.clone(),
                    names: names.clone(),
                    scope: FolderConversionScope::CacheSubtree {
                        relocate_to: None,
                        prefix: prefix.clone(),
                        entries: loaded.entries.clone(),
                        seed,
                    },
                    // The pair the byte order makes real. See
                    // `blam_tags::convert::analyze_conversion_inner`.
                    source_game: "haloreach_mcc".to_owned(),
                    target_game: "haloreach_mcc".to_owned(),
                    target_tags_root: target_tags.clone(),
                    kit_roots: HashMap::new(),
                    accept_loss: false,
                    only: None,
                    cancel: Arc::new(AtomicBool::new(false)),
                },
                &tx,
            )
            .unwrap()
        };

        let report = run(CacheSeed::Folder);
        assert_eq!(
            report.files.len() + report.held_back.len(),
            folder_keys.len(),
            "the run did not account for every tag in {prefix}"
        );
        // Only the folder. Anything else it needed is a question, not a fait
        // accompli.
        for file in &report.files {
            let source = file.source.replace('/', "\\").to_ascii_lowercase();
            assert!(
                source.starts_with(&prefix.to_ascii_lowercase()),
                "{} is outside {prefix} and was converted anyway",
                file.source
            );
        }
        assert!(
            !report.outside_references.is_empty(),
            "a weapon folder reached nothing outside itself, which cannot be right"
        );
        // Grouped so the question stays answerable.
        assert!(
            report
                .outside_references
                .iter()
                .all(|reference| !reference.folder.is_empty()),
            "an outside reference had no folder to be offered under"
        );

        // Accepting the answer brings them, and only them.
        let accepted: HashSet<String> = report
            .outside_references
            .iter()
            .map(|reference| reference.key.clone())
            .collect();
        let second = run(CacheSeed::Keys(accepted.clone()));
        assert_eq!(
            second.files.len() + second.held_back.len(),
            accepted.len(),
            "the second run did not account for every tag it was given"
        );

        // Nothing points at a tag nobody accounted for. Written is the good
        // case; failed, held back, still-outside and already-broken-in-the-build
        // are the honest ones, because each is named in a report the user reads.
        // Silence is the bug this catches.
        let mut written = HashSet::new();
        let mut accounted: HashSet<String> = HashSet::new();
        for report in [&report, &second] {
            for file in &report.files {
                let stem = file
                    .source
                    .rsplit_once('.')
                    .map(|(stem, _)| stem)
                    .unwrap_or(&file.source)
                    .replace('\\', "/")
                    .to_ascii_lowercase();
                accounted.insert(stem.clone());
                if let Some(output) = file.output.as_ref() {
                    assert!(
                        output.starts_with(&target_tags),
                        "{} escaped the kit",
                        output.display()
                    );
                    let tag = TagFile::read(output).unwrap_or_else(|error| {
                        panic!("{} did not reparse: {error}", output.display())
                    });
                    assert_eq!(
                        tag.endian,
                        Endian::Le,
                        "{} was written big-endian",
                        output.display()
                    );
                    written.insert(stem);
                }
            }
            for entry in &report.held_back {
                accounted.insert(
                    entry
                        .source
                        .rsplit_once('.')
                        .map(|(stem, _)| stem)
                        .unwrap_or(&entry.source)
                        .replace('\\', "/")
                        .to_ascii_lowercase(),
                );
            }
            for reference in &report.outside_references {
                accounted.insert(
                    reference
                        .display_path
                        .rsplit_once('.')
                        .map(|(stem, _)| stem)
                        .unwrap_or(&reference.display_path)
                        .replace('\\', "/")
                        .to_ascii_lowercase(),
                );
            }
            for missing in &report.unresolved_references {
                accounted.insert(
                    missing
                        .rsplit_once('.')
                        .map(|(stem, _)| stem)
                        .unwrap_or(missing)
                        .replace('\\', "/")
                        .to_ascii_lowercase(),
                );
            }
        }
        let mut dangling = Vec::new();
        for output in report
            .files
            .iter()
            .chain(&second.files)
            .filter_map(|file| file.output.as_ref())
        {
            let tag = TagFile::read(output).unwrap();
            let mut refs = Vec::new();
            collect_tag_dependency_refs(tag.root(), &mut refs);
            for reference in refs {
                let wanted = reference.rel_path.replace('\\', "/").to_ascii_lowercase();
                if !accounted.contains(&wanted) {
                    dangling.push(format!("{} -> {wanted}", output.display()));
                }
            }
        }
        let _ = fs::remove_dir_all(&target_tags);
        let _ = folder_keys;
        assert!(
            dangling.is_empty(),
            "{} reference(s) nobody accounted for: {:?}",
            dangling.len(),
            dangling.iter().take(8).collect::<Vec<_>>()
        );
    }

    /// The cache root this was developed against, from the environment.
    ///
    /// Not a fixture that can be committed: it is a 27 GB game build. Absent, the
    /// tests above say so and return, the same bargain the kit-backed tests make.
    fn reach_x360_cache_root() -> Option<PathBuf> {
        let root = PathBuf::from(std::env::var("BABOON_REACH_X360_CACHE").ok()?);
        root.join("blob_index.dat").is_file().then_some(root)
    }

    fn cache_entry(name: &str, group: &[u8; 4]) -> TagEntry {
        let group_tag = u32::from_be_bytes(*group);
        TagEntry {
            key: format!("cache:{}:{name}", format_group_tag(group_tag)),
            display_path: name.replace('\\', "/"),
            group_tag,
            group_name: None,
            location: TagEntryLocation::Monolithic {
                name: name.to_owned(),
                group_tag,
            },
        }
    }

    /// A folder is a run of path segments, not a run of characters.
    ///
    /// The distinction is the whole of it: `objects\weapons\rifle` and
    /// `objects\weapons\rifleman` share a string prefix and share no folder, and
    /// a plain `starts_with` would drag the second into a run asked for the
    /// first.
    #[test]
    fn a_cache_folder_takes_its_own_tags_and_not_its_neighbours() {
        let rifle = cache_entry(r"objects\weapons\rifle\assault_rifle", b"weap");
        let deeper = cache_entry(r"objects\weapons\rifle\scope\scope", b"weap");
        let neighbour = cache_entry(r"objects\weapons\rifleman\rifleman", b"weap");
        let elsewhere = cache_entry(r"objects\vehicles\warthog\warthog", b"vehi");

        for entry in [&rifle, &deeper] {
            assert!(
                cache_entry_is_under(entry, r"objects\weapons\rifle"),
                "{} should be under the folder",
                entry.display_path
            );
        }
        for entry in [&neighbour, &elsewhere] {
            assert!(
                !cache_entry_is_under(entry, r"objects\weapons\rifle"),
                "{} should not be",
                entry.display_path
            );
        }
        // Forward slashes and case are what the browser hands back, not what the
        // cache stores.
        assert!(cache_entry_is_under(&rifle, "Objects/Weapons/Rifle"));
        // Trailing separators come from the same place.
        assert!(cache_entry_is_under(&rifle, r"objects\weapons\rifle\"));
        // The whole cache.
        assert!(cache_entry_is_under(&elsewhere, ""));
    }

    /// A reference and the entry it names have to fold to the same key.
    ///
    /// Tag paths inside a tag are written by whoever authored it, so the same
    /// tag turns up spelled several ways across a build. A lookup that respected
    /// those differences would follow some references and quietly drop others.
    #[test]
    fn a_reference_folds_to_the_key_of_the_tag_it_names() {
        let entry = cache_entry(r"objects\weapons\rifle\assault_rifle", b"weap");
        let TagEntryLocation::Monolithic { name, group_tag } = &entry.location else {
            unreachable!()
        };
        let canonical = folded_cache_key(*group_tag, name);
        for spelling in [
            r"objects\weapons\rifle\assault_rifle",
            "objects/weapons/rifle/assault_rifle",
            r"Objects\Weapons\Rifle\Assault_Rifle",
        ] {
            assert_eq!(
                folded_cache_key(u32::from_be_bytes(*b"weap"), spelling),
                canonical,
                "{spelling} folded to a different key"
            );
        }
        // The group is part of the identity: two classes can share a path.
        assert_ne!(folded_cache_key(u32::from_be_bytes(*b"hlmt"), name), canonical);
    }
}
