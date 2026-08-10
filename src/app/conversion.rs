//! The cross-game conversion worker: walk a folder of tags, convert each, report back.
//!
//! What a tag *becomes* in another game — group routing, struct pairing, field
//! matching, value translation, companion synthesis — lives in
//! `blam_tags::convert`. The dialog that drives this is `import.rs`; this module
//! owns only the job itself and the shape of what it reports.

use super::*;
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
    /// Source file, so a second pass can be restricted to exactly these.
    pub(in crate::app) path: PathBuf,
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

pub(in crate::app) struct FolderConversionJob {
    pub(in crate::app) source: TagSource,
    pub(in crate::app) names: TagNameIndex,
    pub(in crate::app) source_rel_path: PathBuf,
    /// The leaf folder the converted tags land in, under `destination_parent`.
    /// Named for the destination rather than the source because that is what it
    /// decides: it is free to differ from the source folder's own name.
    pub(in crate::app) destination_label: String,
    pub(in crate::app) source_game: String,
    pub(in crate::app) target_game: String,
    pub(in crate::app) target_tags_root: PathBuf,
    pub(in crate::app) destination_parent: PathBuf,
    /// Every configured editing kit, so a tag the direct pair refuses can be
    /// routed through the engines in between. Only consulted when that happens:
    /// indexing a kit walks its whole tag tree.
    pub(in crate::app) kit_roots: HashMap<String, PathBuf>,
    /// Write tags that lose audited data instead of holding them back. Set only
    /// for the second pass, after the user has seen what goes missing.
    pub(in crate::app) accept_loss: bool,
    /// Restrict the run to these source files. `None` converts the whole folder;
    /// the second pass names exactly the tags that were held back, so accepting
    /// the loss does not mean redoing everything.
    pub(in crate::app) only: Option<HashSet<PathBuf>>,
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
    phase: &str,
    current: &str,
    processed: usize,
    total: usize,
    converted: usize,
    failed: usize,
) {
    let _ = tx.send(WorkerMessage::FolderConversionProgress(
        FolderConversionProgress {
            phase: phase.to_owned(),
            current: current.to_owned(),
            processed,
            total,
            converted,
            failed,
        },
    ));
}

pub(in crate::app) fn run_folder_conversion_job(
    job: FolderConversionJob,
    tx: &Sender<WorkerMessage>,
) -> Result<FolderConversionReport, String> {
    let TagSource::LooseFolder { root, .. } = &job.source else {
        return Err("Folder conversion requires a loose-folder source".to_owned());
    };
    let source_folder = normalize_conversion_path(&root.join(&job.source_rel_path));
    let target_tags_root = normalize_conversion_path(&job.target_tags_root);
    let destination_root =
        normalize_conversion_path(&job.destination_parent.join(&job.destination_label));
    if !destination_root.starts_with(&target_tags_root) {
        return Err("Folder conversion destination escapes the target tags folder".to_owned());
    }

    let mut disk_paths = Vec::new();
    for item in walkdir::WalkDir::new(&source_folder).follow_links(false) {
        let item =
            item.map_err(|error| format!("Could not scan {}: {error}", source_folder.display()))?;
        if item.file_type().is_file() {
            disk_paths.push(item.into_path());
        }
    }
    disk_paths.sort();
    let total_files = disk_paths.len();
    let mut entries = Vec::new();
    let mut ignored_files = Vec::new();
    let mut scan_failures = Vec::new();
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
                detail: format!("Could not identify tag: {error}"),
            }),
        }
        send_folder_conversion_progress(
            tx,
            "Scanning source folder",
            &display,
            index + 1,
            total_files,
            0,
            scan_failures.len(),
        );
    }

    let definitions_root = locate_definitions_root();
    let source_groups = GameTagIndex::load(&definitions_root, &job.source_game)?;
    // What each source class becomes in the target, asked once per class rather
    // than once per tag: the answer depends only on the group, and getting it can
    // mean walking a route. The engine answers, because a folder run naming its
    // own output by canonical name disagrees with the converter exactly where a
    // class was renamed — which is every contrail_system into Halo 4, and every
    // shader into a game that still declares `shader` and ships none.
    let mut landing_groups: HashMap<u32, Result<(u32, String), String>> = HashMap::new();
    for entry in &entries {
        if landing_groups.contains_key(&entry.group_tag) {
            continue;
        }
        let landed = match source_groups.by_tag.get(&entry.group_tag) {
            Some(name) => converted_group(
                name,
                &job.source_game,
                &job.target_game,
                &definitions_root,
            )
            .and_then(|found| {
                found.ok_or_else(|| format!("Target profile has no {name} group"))
            }),
            None => Err(format!(
                "Unknown source group {}",
                format_group_tag(entry.group_tag)
            )),
        };
        landing_groups.insert(entry.group_tag, landed);
    }
    send_folder_conversion_progress(
        tx,
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

    let mut planned = Vec::new();
    let mut destination_counts = HashMap::<String, usize>::new();
    for entry in entries {
        let destination =
            target_destination_for_entry(&entry, &source_folder, &destination_root, &landing_groups);
        if let Ok(path) = &destination {
            let key = normalize_conversion_path(path)
                .to_string_lossy()
                .to_ascii_lowercase();
            *destination_counts.entry(key).or_default() += 1;
        }
        planned.push((entry, destination));
    }
    if let Some(only) = &job.only {
        planned.retain(|(entry, _)| match &entry.location {
            TagEntryLocation::LooseFile(path) => only.contains(path),
            _ => false,
        });
    }

    let total = planned.len();
    let mut report = FolderConversionReport {
        source_root: source_folder.clone(),
        source_game: job.source_game.clone(),
        target_game: job.target_game.clone(),
        destination_root,
        files: scan_failures,
        ignored_files,
        held_back: Vec::new(),
    };
    // Collected on the side because a held-back tag is neither a success nor a
    // failure: it converted, and it is waiting on an answer.
    let mut held_back: Vec<FolderConversionHeldBack> = Vec::new();
    let mut converted = 0;
    let mut failed = report.files.len();
    let mut claimed_outputs = HashSet::<String>::new();
    for (index, (entry, destination)) in planned.into_iter().enumerate() {
        let source_label = entry.display_path.clone();
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
                .map_err(|error| format!("Could not read source tag: {error}"))?;
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
                format!("Native layout template: {}", template.display())
            } else {
                "Generated layout; native editing-kit compatibility is unverified".to_owned()
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

        let file_result = match result {
            Ok(FileOutcome::Written(result)) => {
                converted += 1;
                result
            }
            // Neither converted nor failed: it is waiting on an answer, so it
            // stays out of both counts and out of the file list.
            Ok(FileOutcome::HeldBack(losses)) => {
                if let TagEntryLocation::LooseFile(path) = &entry.location {
                    held_back.push(FolderConversionHeldBack {
                        source: source_label.clone(),
                        path: path.clone(),
                        losses,
                    });
                }
                send_folder_conversion_progress(
                    tx,
                    "Converting tags",
                    &source_label,
                    index + 1,
                    total,
                    converted,
                    failed,
                );
                continue;
            }
            Err(error) => {
                failed += 1;
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
            "Converting tags",
            &source_label,
            index + 1,
            total,
            converted,
            failed,
        );
    }
    report
        .files
        .sort_by(|left, right| left.source.cmp(&right.source));
    held_back.sort_by(|left, right| left.source.cmp(&right.source));
    report.held_back = held_back;
    let _ = tx.send(WorkerMessage::TerminalLine(format!(
        "Folder conversion complete: {} converted, {} failed, {} held back, {} ignored",
        report.converted_count(),
        report.failed_count(),
        report.held_back.len(),
        report.ignored_files.len()
    )));
    Ok(report)
}

/// Where one source tag is expected to land.
///
/// Expected, not decided: the extension comes from the class the converter is
/// predicted to produce, and the draft has the final say once the tag has
/// actually been through. This runs first only because two sources colliding on
/// one destination has to be caught before either is written.
fn target_destination_for_entry(
    entry: &TagEntry,
    source_folder: &Path,
    destination_root: &Path,
    landing_groups: &HashMap<u32, Result<(u32, String), String>>,
) -> Result<PathBuf, String> {
    let TagEntryLocation::LooseFile(source_path) = &entry.location else {
        return Err("Folder conversion only supports loose tags".to_owned());
    };
    let relative = normalize_conversion_path(source_path)
        .strip_prefix(source_folder)
        .map(Path::to_path_buf)
        .map_err(|_| "Source tag escapes the selected folder".to_owned())?;
    let (target_group, target_group_name) = landing_groups
        .get(&entry.group_tag)
        .ok_or_else(|| format!("Unknown source group {}", format_group_tag(entry.group_tag)))?
        .as_ref()
        .map_err(String::clone)?;
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
                source_rel_path: PathBuf::from("characters/jackal"),
                destination_label: "jackal".to_owned(),
                source_game: "halo3_mcc".to_owned(),
                target_game: "haloreach_mcc".to_owned(),
                target_tags_root: target_tags,
                destination_parent: destination_parent.clone(),
                kit_roots: HashMap::new(),
                accept_loss: false,
                only: None,
            },
            &tx,
        )
        .unwrap();

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
                source_rel_path: PathBuf::from("fx"),
                destination_label: "fx".to_owned(),
                source_game: "halo3_mcc".to_owned(),
                target_game: "halo4_mcc".to_owned(),
                target_tags_root: target_tags,
                destination_parent: destination_parent.clone(),
                kit_roots: HashMap::new(),
                accept_loss: true,
                only: None,
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
}
