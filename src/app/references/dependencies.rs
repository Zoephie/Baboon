//! Building the reverse-dependency index, listing unreferenced tags, and fixing
//! a tag's broken dependencies against the tags that exist.

use super::*;
use crate::app::kits::terminal::trim_terminal_lines;
use crate::app::tag_ops::refactor::send_folder_refactor_progress;

impl Baboon {
    pub(in crate::app) fn fix_current_tag_dependencies(&mut self) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        let Some(key) = self.model.kits[self.model.active].selected_key.clone() else {
            self.model.status = "No tag selected".to_owned();
            return;
        };
        let Some(entry) = self.model.entry_for_key(&key).cloned() else {
            self.model.status = "Selected tag is no longer in the source".to_owned();
            return;
        };
        let TagEntryLocation::LooseFile(_) = entry.location else {
            self.model.status = "Fix Tag Dependencies requires a loose-folder tag".to_owned();
            return;
        };
        let Some(root) = self.model.loaded_tags_root() else {
            self.model.status = "Fix Tag Dependencies requires a loaded tags folder".to_owned();
            return;
        };

        let entries = match self.model.dependency_database_entries() {
            Ok(entries) => entries,
            Err(error) => {
                self.model.status = format!("Could not build dependency database: {error}");
                return;
            }
        };
        let names = self.model.names().clone();
        let index = build_dependency_candidate_index(&entries, &names);
        let Some(doc) = self.model.kits[self.model.active].parsed_tags.get_mut(&key) else {
            self.model.status = "Load the selected tag before fixing dependencies".to_owned();
            return;
        };
        if doc.tag.endian != Endian::Le {
            self.model.status = "Only little-endian loose tags can be edited".to_owned();
            return;
        }

        let report = fix_tag_dependencies_in_tag(&mut doc.tag, &root, &names, &index);
        if report.fixed > 0 {
            doc.dirty.touch();
        }
        let status = report.status();
        self.kit_tools.terminal
            .lines
            .extend(report.lines.into_iter().map(TerminalLineEntry::new));
        trim_terminal_lines(&mut self.kit_tools.terminal.lines);
        self.kit_tools.terminal.scroll_to_bottom = true;
        self.model.status = status;
    }

    pub(in crate::app) fn show_unreferenced_tags(&mut self) {
        match self.model.unreferenced_entries() {
            Some(entries) => {
                let note = entries
                    .is_empty()
                    .then(|| "Every tag is referenced by at least one other tag.".to_owned());
                self.dialogs.open(QueryResultsWindow::new(TagQueryResults {
                    kit: self.model.active_kit_id(),
                    title: format!("Unreferenced tags ({})", entries.len()),
                    entries,
                    annotations: Vec::new(),
                    note,
                    ref_target: None,
                }));
            }
            None => {
                self.dialogs.open(QueryResultsWindow::new(TagQueryResults {
                    kit: self.model.active_kit_id(),
                    title: "Unreferenced tags".to_owned(),
                    entries: Vec::new(),
                    annotations: Vec::new(),
                    note: Some(self.model.reference_index_unavailable_note()),
                    ref_target: None,
                }));
            }
        }
    }

    /// Build the reverse-dependency index in the background so the
    /// find-references / unreferenced / Content Explorer features work without
    /// first running a move/rename. Idempotent: skips while a build is running,
    /// and skips an already-present index unless `force` is set (Tools →
    /// Rebuild). Loose-folder sources only; the result is persisted to disk so
    /// future launches load it instantly.
    /// Starts source-scoped indexing or search work without blocking the UI thread.
    /// Generation-tagged completion is ignored if the active source changes first.
    pub(in crate::app) fn begin_build_reverse_dependencies(&mut self, ctx: egui::Context, force: bool) {
        self.begin_build_reverse_dependencies_in(self.model.active, ctx, force, false);
    }

    /// The index build that follows `kit_index`'s completed scan. Named by
    /// kit rather than read off the focus: the scan finishes whenever it
    /// finishes, and the user may be in another game by then.
    pub(in crate::app) fn begin_build_reverse_dependencies_for_entry_index(
        &mut self,
        kit_index: usize,
        ctx: egui::Context,
    ) {
        self.begin_build_reverse_dependencies_in(kit_index, ctx, false, true);
    }

    pub(in crate::app) fn begin_build_reverse_dependencies_in(
        &mut self,
        kit_index: usize,
        ctx: egui::Context,
        force: bool,
        paired_entry_index_build: bool,
    ) {
        if self.model.kits[kit_index].index_jobs.building_references
            || self.model.kits[kit_index].scanning_entries
        {
            return;
        }
        let Some(source) = self.model.kits[kit_index].source.as_ref() else {
            return;
        };
        // Loose folders index automatically after their scan. Containers are
        // indexable too, but only on request (Tools → Build Reference Index):
        // container tags carry no dependency-list stream, so every tag has to be
        // parsed — for Campaign Evolved that is ~12k tags and several GB of
        // reads, too much to run behind every mount.
        let is_loose = matches!(source.source, TagSource::LooseFolder { .. });
        if !is_loose && !matches!(source.source, TagSource::IoStoreContainerSet { .. }) {
            return;
        }
        if source.reverse_dependencies.is_some() && !force {
            return;
        }
        // A container mount enumerates every tag up front. A loose folder must
        // use its completed scan only: an index built from the lazy browser
        // subset would be wrong (it would flag tags as unreferenced just because
        // their referrers weren't scanned).
        let entries = if is_loose {
            source.all_entries.clone()
        } else {
            source.full_entry_set().to_vec()
        };
        if entries.is_empty() && is_loose && source.complete_scan {
            // Scanned, and there is nothing in it: an empty graph, not a
            // reason to scan again (which is what an empty folder did, forever).
            if let Some(source) = self.model.kits[kit_index].source.as_mut() {
                source.reverse_dependencies = Some(ReverseDependencyIndex::default());
            }
            return;
        }
        if entries.is_empty() {
            // The full entry set isn't ready yet, so kick the scan first.
            // `begin_scan_all_entries` is idempotent (guards on
            // `scanning_entries`); the update loop re-enters here and builds the
            // index once the scan lands. Containers have nothing to scan — an
            // empty mount simply has nothing to index.
            if is_loose {
                if !self.model.kits[kit_index].scanning_entries {
                    self.model.status = "Indexing tags, then building reference index…".to_owned();
                }
                self.begin_scan_all_entries_in(
                    kit_index,
                    ctx,
                    "Indexing tags, then building reference index...",
                );
            }
            return;
        }
        let tag_source = source.source.clone();
        let kit = &self.model.kits[kit_index];
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        let tx = self.tx.clone();
        self.model.kits[kit_index].index_jobs.building_references = true;
        self.model.kits[kit_index]
            .index_jobs
            .references_changed_during_build
            .clear();
        self.model.kits[kit_index].index_jobs.references_for_entry_index = paired_entry_index_build;
        self.model.kits[kit_index].index_jobs.reference_progress = Some(ReferenceIndexProgressState {
            label: "Building reference index...".to_owned(),
            processed: 0,
            total: entries.len(),
        });
        if paired_entry_index_build {
            self.dialogs.open(IndexingNotice);
        }
        self.model.status = "Building reference index…".to_owned();
        // A build that panicked used to send nothing and leave the index
        // "building" for the session; it reports every tag missing instead.
        let entry_total = entries.len();
        let (worker_tx, worker_ctx) = (tx.clone(), ctx.clone());
        spawn_worker(&tx, &ctx, move || {
            let (tx, ctx) = (worker_tx, worker_ctx);
            let total = entries.len();
            let _ = tx.send(WorkerMessage::ReferenceIndexProgress {
                stamp,
                processed: 0,
                total,
            });
            let worker_count = std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(1)
                .clamp(1, total.max(1));
            let chunk_size = total.div_ceil(worker_count).max(1);
            let processed = std::sync::atomic::AtomicUsize::new(0);

            let mut index = ReverseDependencyIndex::default();
            let mut missing = 0usize;
            std::thread::scope(|scope| {
                let mut handles = Vec::new();
                for chunk in entries.chunks(chunk_size) {
                    let tag_source = &tag_source;
                    let progress_tx = tx.clone();
                    let progress_ctx = ctx.clone();
                    let processed = &processed;
                    handles.push(scope.spawn(move || {
                        let mut chunk_results = Vec::new();
                        for entry in chunk {
                            if let Ok(deps) = read_entry_dependencies(tag_source, entry) {
                                chunk_results.push((entry.key.clone(), deps));
                            }
                            let processed_now =
                                processed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            if processed_now == total || processed_now % 32 == 0 {
                                let _ = progress_tx.send(WorkerMessage::ReferenceIndexProgress {
                                    stamp,
                                    processed: processed_now,
                                    total,
                                });
                                progress_ctx.request_repaint();
                            }
                        }
                        chunk_results
                    }));
                }

                // A chunk whose thread panicked used to vanish from the index
                // without a word. Its tags are counted instead, so the index is
                // reported (and not saved) as incomplete.
                for (handle, chunk) in handles.into_iter().zip(entries.chunks(chunk_size)) {
                    match handle.join() {
                        Ok(chunk_results) => {
                            for (key, deps) in chunk_results {
                                index.set_tag_dependencies(key, deps);
                            }
                        }
                        Err(_) => missing += chunk.len(),
                    }
                }
            });
            WorkerMessage::ReverseDependenciesBuilt {
                stamp,
                index,
                missing,
            }
        }, move |_| WorkerMessage::ReverseDependenciesBuilt {
            stamp,
            index: ReverseDependencyIndex::default(),
            missing: entry_total,
        });
    }
}

type DependencyCandidateIndex = HashMap<(u32, String), Vec<String>>;

#[derive(Default)]
pub(in crate::app) struct DependencyFixReport {
    scanned: usize,
    fixed: usize,
    already_ok: usize,
    unresolved: usize,
    ambiguous: usize,
    skipped: usize,
    lines: Vec<String>,
}

impl DependencyFixReport {
    fn status(&self) -> String {
        if self.fixed > 0 {
            format!(
                "Fixed {} dependenc{} ({} unresolved, {} ambiguous)",
                self.fixed,
                if self.fixed == 1 { "y" } else { "ies" },
                self.unresolved,
                self.ambiguous
            )
        } else if self.unresolved == 0 && self.ambiguous == 0 {
            format!(
                "No broken dependencies found across {} reference(s)",
                self.scanned
            )
        } else {
            format!(
                "No dependencies auto-fixed ({} unresolved, {} ambiguous)",
                self.unresolved, self.ambiguous
            )
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct TagReferenceUse {
    pub(in crate::app) field_path: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) rel_path: String,
}

pub(in crate::app) fn fix_tag_dependencies_in_tag(
    tag: &mut TagFile,
    tags_root: &Path,
    names: &TagNameIndex,
    index: &DependencyCandidateIndex,
) -> DependencyFixReport {
    let mut refs = Vec::new();
    collect_tag_references(tag.root(), "", &mut refs);

    let mut report = DependencyFixReport {
        scanned: refs.len(),
        lines: vec![format!(
            "Fix Tag Dependencies: scanned {} reference(s)",
            refs.len()
        )],
        ..Default::default()
    };
    let mut fixes = Vec::new();
    for reference in refs {
        let Some(extension) = names
            .name_for(reference.group_tag)
            .or_else(|| group_tag_to_extension(reference.group_tag))
        else {
            report.skipped += 1;
            report.lines.push(format!(
                "Skipped {}: unknown group {}",
                reference.field_path,
                format_group_tag(reference.group_tag)
            ));
            continue;
        };
        if dependency_target_exists(tags_root, &reference.rel_path, extension) {
            report.already_ok += 1;
            continue;
        }

        let leaf = dependency_leaf_key(&reference.rel_path);
        let key = (reference.group_tag, leaf.clone());
        let candidates = index.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        match candidates {
            [candidate] if !candidate.eq_ignore_ascii_case(&reference.rel_path) => {
                fixes.push((reference.clone(), candidate.clone()));
            }
            [] => {
                report.unresolved += 1;
                report.lines.push(format!(
                    "Unresolved {}: {}",
                    reference.field_path,
                    format_reference_path(names, reference.group_tag, &reference.rel_path)
                ));
            }
            _ => {
                report.ambiguous += 1;
                report.lines.push(format!(
                    "Ambiguous {}: {} candidate(s) named {}.{}",
                    reference.field_path,
                    candidates.len(),
                    leaf,
                    extension
                ));
            }
        }
    }

    for (reference, fixed_path) in fixes {
        let mut root = tag.root_mut();
        let Some(mut field) = root.field_path_mut(&reference.field_path) else {
            report.unresolved += 1;
            report.lines.push(format!(
                "Skipped {}: field path no longer resolves",
                reference.field_path
            ));
            continue;
        };
        let result = field.set(TagFieldData::TagReference(TagReferenceData {
            group_tag_and_name: Some((reference.group_tag, fixed_path.clone())),
        }));
        match result {
            Ok(()) => {
                report.fixed += 1;
                report.lines.push(format!(
                    "Fixed {}: {} -> {}",
                    reference.field_path,
                    format_reference_path(names, reference.group_tag, &reference.rel_path),
                    format_reference_path(names, reference.group_tag, &fixed_path)
                ));
            }
            Err(error) => {
                report.unresolved += 1;
                report.lines.push(format!(
                    "Skipped {}: could not write dependency ({error:?})",
                    reference.field_path
                ));
            }
        }
    }

    report.lines.push(report.status());
    report
}

pub(in crate::app) fn collect_tag_references(
    tag_struct: TagStruct<'_>,
    path_prefix: &str,
    refs: &mut Vec<TagReferenceUse>,
) {
    for field in tag_struct.fields() {
        let field_path = append_field_path_for(path_prefix, &field);
        match field.value() {
            Some(TagFieldData::TagReference(reference)) => {
                let Some((group_tag, rel_path)) = reference.group_tag_and_name else {
                    continue;
                };
                let rel_path = sanitize_ref_path(&rel_path).replace('/', "\\");
                if rel_path.is_empty() || rel_path.eq_ignore_ascii_case("none") {
                    continue;
                }
                refs.push(TagReferenceUse {
                    field_path,
                    group_tag,
                    rel_path,
                });
                continue;
            }
            Some(_) => continue,
            None => {}
        }
        if let Some(nested) = field.as_struct() {
            collect_tag_references(nested, &field_path, refs);
        } else if let Some(block) = field.as_block() {
            for (index, element) in block.iter().enumerate() {
                let element_path = format!("{field_path}[{index}]");
                collect_tag_references(element, &element_path, refs);
            }
        } else if let Some(array) = field.as_array() {
            for (index, element) in array.iter().enumerate() {
                let element_path = format!("{field_path}[{index}]");
                collect_tag_references(element, &element_path, refs);
            }
        }
    }
}

/// Collect just the reference *targets* in a tag, without the field-path
/// bookkeeping [`collect_tag_references`] does for the reference-jump UI.
/// Indexing walks every element of every block across the whole tag set, where
/// building a path string per visited field dominates the cost — and the
/// dependency index discards those paths.
pub(in crate::app) fn collect_tag_dependency_refs(
    tag_struct: TagStruct<'_>,
    refs: &mut Vec<DependencyRef>,
) {
    for field in tag_struct.fields() {
        match field.value() {
            Some(TagFieldData::TagReference(reference)) => {
                let Some((group_tag, rel_path)) = reference.group_tag_and_name else {
                    continue;
                };
                let rel_path = sanitize_ref_path(&rel_path).replace('/', "\\");
                if rel_path.is_empty() || rel_path.eq_ignore_ascii_case("none") {
                    continue;
                }
                refs.push(DependencyRef {
                    group_tag,
                    rel_path,
                });
                continue;
            }
            Some(_) => continue,
            None => {}
        }
        if let Some(nested) = field.as_struct() {
            collect_tag_dependency_refs(nested, refs);
        } else if let Some(block) = field.as_block() {
            for element in block.iter() {
                collect_tag_dependency_refs(element, refs);
            }
        } else if let Some(array) = field.as_array() {
            for element in array.iter() {
                collect_tag_dependency_refs(element, refs);
            }
        }
    }
}

pub(in crate::app) fn build_dependency_candidate_index(
    entries: &[TagEntry],
    names: &TagNameIndex,
) -> DependencyCandidateIndex {
    let mut index: DependencyCandidateIndex = HashMap::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let Some(rel_path) = dependency_entry_reference_path(entry, names) else {
            continue;
        };
        if !seen.insert((entry.group_tag, rel_path.to_ascii_lowercase())) {
            continue;
        }
        let leaf = dependency_leaf_key(&rel_path);
        index
            .entry((entry.group_tag, leaf))
            .or_default()
            .push(rel_path);
    }
    for candidates in index.values_mut() {
        candidates.sort();
    }
    index
}

pub(in crate::app) fn build_reverse_dependency_index(
    root: &Path,
    source: &TagSource,
    entries: &[TagEntry],
    label: &str,
    tx: &Sender<WorkerMessage>,
) -> ReverseDependencyIndex {
    let mut index = ReverseDependencyIndex::default();
    let total = entries.len();
    for (entry_index, entry) in entries.iter().enumerate() {
        if entry_index == 0 || (entry_index + 1) % 50 == 0 || entry_index + 1 == total {
            let progress = if total == 0 {
                None
            } else {
                Some((entry_index + 1) as f32 / total as f32)
            };
            send_folder_refactor_progress(
                tx,
                label,
                &format!("Building dependency index {}/{}", entry_index + 1, total),
                progress,
            );
        }
        let deps = match read_entry_dependencies(source, entry) {
            Ok(deps) => deps,
            Err(error) => {
                let _ = tx.send(WorkerMessage::TerminalLine(format!(
                    "Warning: skipped dependency index for {}: {error}",
                    entry.display_path
                )));
                continue;
            }
        };
        index.set_tag_dependencies(entry.key.clone(), deps);
    }
    let _ = tx.send(WorkerMessage::TerminalLine(format!(
        "Built dependency index for {} tag(s) under {}",
        index.len(),
        root.display()
    )));
    index
}

pub(in crate::app) fn read_entry_dependencies(
    source: &TagSource,
    entry: &TagEntry,
) -> Result<Vec<DependencyRef>, String> {
    match &entry.location {
        // A loose tag usually carries a `want` (dependency-list) stream, which
        // is far cheaper to read than the whole tag.
        TagEntryLocation::LooseFile(path) => {
            if let Some(refs) = TagFile::read_dependency_references(path)
                .map_err(|error| format!("Could not read dependency list: {error}"))?
            {
                return Ok(refs
                    .into_iter()
                    .map(|(group_tag, rel_path)| DependencyRef {
                        group_tag,
                        rel_path: sanitize_ref_path(&rel_path).replace('/', "\\"),
                    })
                    .collect());
            }
        }
        // Cache and container tags have no separate dependency-list stream to
        // shortcut through (verified: none of Campaign Evolved's 12,291
        // container tags has a `want` chunk), so they fall through to the parse
        // path below — which `read_entry` supports for both.
        TagEntryLocation::Monolithic { .. } | TagEntryLocation::Container { .. } => {}
        // A brand-new tag exists only as an in-memory document; it has no
        // payload to read here, and it is not yet referenced by anything.
        TagEntryLocation::NewContainer { .. } => return Ok(Vec::new()),
    }
    let tag = read_entry(source, entry).map_err(|error| format!("Could not parse tag: {error}"))?;
    let mut refs = Vec::new();
    collect_tag_dependency_refs(tag.root(), &mut refs);
    Ok(refs)
}

pub(in crate::app) fn reference_path_from_abs_file(
    tags_root: &Path,
    path: &Path,
    group_tag: u32,
    names: &TagNameIndex,
) -> Option<String> {
    let rel = path.strip_prefix(tags_root).ok()?;
    reference_path_from_rel_file(rel, group_tag, names)
}

pub(in crate::app) fn reference_path_from_rel_file(
    rel_file: &Path,
    group_tag: u32,
    names: &TagNameIndex,
) -> Option<String> {
    reference_path_without_group_extension(&rel_file.to_string_lossy(), group_tag, names)
}

impl Model {
    /// Every tag in the loaded folder, for Fix Tag Dependencies to match
    /// broken references against.
    ///
    /// This used to rescan the whole folder on the UI thread on every use,
    /// even with the completed scan already in memory, then replace
    /// `all_entries` without moving the kit generation and rewrite the whole
    /// index. The completed scan is kept current by single-tag upserts and the
    /// periodic refresh, so it is used as it is; before it exists, this says
    /// so rather than blocking on a scan of its own.
    pub(in crate::app) fn dependency_database_entries(&self) -> Result<Vec<TagEntry>, String> {
        let source = self.kits[self.active]
            .source
            .as_ref()
            .ok_or_else(|| "no tag source is loaded".to_owned())?;
        if !matches!(source.source, TagSource::LooseFolder { .. }) {
            return Err("load a loose editing-kit tags folder first".to_owned());
        }
        if source.all_entries.is_empty() {
            return Err(
                "the tag index is still being built; try again once indexing finishes".to_owned(),
            );
        }
        Ok(source.all_entries.clone())
    }

    /// Explain why a reference lookup found no index, tailored to whether one is
    /// currently building (auto after the full scan, or via Tools → Build
    /// Reference Index).
    pub(in crate::app) fn reference_index_unavailable_note(&self) -> String {
        if self.kits[self.active].index_jobs.building_references
            || self.kits[self.active].scanning_entries
        {
            "Reference index is building — try again in a moment.".to_owned()
        } else {
            "Reference index unavailable — run Tools → Build Reference Index.".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::kits::loading::loaded_source_status;
    use crate::app::tag_ops::new_tag::new_container_package;
    use crate::app::tag_ops::new_tag::new_container_template_for;
    use crate::app::tag_ops::new_tag::normalize_container_tag_rel;
    use crate::app::tag_ops::refactor::affected_move_rewrite_entries;
    use crate::app::tag_ops::refactor::build_folder_reference_rewrites;
    use crate::app::tag_ops::refactor::bytes_contain_any_ascii_case_insensitive;
    use crate::app::tag_ops::refactor::rewrite_reference_needles;

    fn entry(display_path: &str, group_tag: u32) -> TagEntry {
        TagEntry {
            key: format!("file:{display_path}"),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
        }
    }

    fn abs_entry(root: &Path, display_path: &str, group_tag: u32) -> TagEntry {
        TagEntry {
            key: file_entry_key(&root.join(display_path)),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: None,
            location: TagEntryLocation::LooseFile(root.join(display_path)),
        }
    }

    fn container_entry(key: &str, display_path: &str, group_tag: u32) -> TagEntry {
        TagEntry {
            key: key.to_owned(),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: None,
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: format!("Tags/{display_path}.ubulk"),
            },
        }
    }

    fn loose_source_with_counts(label: &str, entries: Vec<TagEntry>) -> LoadedSourceData {
        LoadedSourceData {
            label: label.to_owned(),
            source: TagSource::LooseFolder {
                root: PathBuf::from("C:/kit/tags"),
                game: Some(GameId::Halo3),
                definitions_root: PathBuf::from("C:/kit/definitions"),
            },
            names: TagNameIndex::default(),
            game: Some(GameId::Halo3),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: entries,
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        }
    }

    #[test]
    fn loose_folder_status_does_not_report_zero_loaded_tags_before_scan() {
        let source = loose_source_with_counts("H3EK/tags (halo3_mcc)", Vec::new());

        assert_eq!(
            loaded_source_status(&source),
            "Browsing tags from H3EK/tags (halo3_mcc)"
        );
    }

    #[test]
    fn loose_folder_status_uses_recursive_index_when_available() {
        let shader = parse_group_tag("rmsh").unwrap();
        let source = loose_source_with_counts(
            "H3EK/tags (halo3_mcc)",
            vec![
                entry("objects/a.shader", shader),
                entry("objects/b.shader", shader),
            ],
        );

        assert_eq!(
            loaded_source_status(&source),
            "Found 2 tag(s) in H3EK/tags (halo3_mcc)"
        );
    }

    #[test]
    fn dependency_entry_reference_path_strips_only_group_extension() {
        let names = TagNameIndex::default();
        let bitmap = parse_group_tag("bitm").unwrap();
        let entry = entry("objects/weapons/decal_road_1.bitmap.bitmap", bitmap);

        assert_eq!(
            dependency_entry_reference_path(&entry, &names).unwrap(),
            "objects\\weapons\\decal_road_1.bitmap"
        );
    }

    #[test]
    fn container_reference_resolution_uses_group_and_normalized_path() {
        let names = TagNameIndex::default();
        let render_model = parse_group_tag("mode").unwrap();
        let weapon = parse_group_tag("weap").unwrap();
        let display_stem = "objects/shared/example";
        let entries = vec![
            entry(&format!("{display_stem}.render_model"), render_model),
            container_entry(
                "ublock:shared:model",
                &format!("{display_stem}.render_model"),
                render_model,
            ),
            container_entry(
                "ublock:shared:weapon",
                &format!("{display_stem}.weapon"),
                weapon,
            ),
        ];

        let model = container_entry_for_reference(
            &entries,
            render_model,
            "OBJECTS/SHARED/EXAMPLE.RENDER_MODEL",
            &names,
        )
        .expect("render-model reference should resolve");
        assert_eq!(model.key, "ublock:shared:model");

        let weapon_entry =
            container_entry_for_reference(&entries, weapon, r"objects\shared\example", &names)
                .expect("same path in another group should resolve independently");
        assert_eq!(weapon_entry.key, "ublock:shared:weapon");

        assert!(
            container_entry_for_reference(
                &entries,
                render_model,
                r"objects\shared\missing",
                &names
            )
            .is_none()
        );
    }

    /// A tag created this session is a reference target like any other: it is
    /// addressed by the same logical path, and "Open referenced tag" resolves
    /// through this lookup. Excluding it reported the tag as missing.
    #[test]
    fn container_reference_resolution_finds_an_unsaved_new_tag() {
        let names = TagNameIndex::default();
        let camera_track = parse_group_tag("trak").unwrap();
        let entries = vec![new_container_entry(
            "test/example.camera_track",
            camera_track,
            "camera_track",
        )];

        let found = container_entry_for_reference(
            &entries,
            camera_track,
            r"test\example.camera_track",
            &names,
        )
        .expect("a new tag should resolve as a reference target");
        assert_eq!(found.key, "newtag:/Game/Tags/test/example-camera_track");
    }

    /// The capability matrix for a brand-new container tag, in one place: what
    /// it can do, and the two things it deliberately cannot. Every one of these
    /// gates gets its answer from a `match` on `TagEntryLocation`, and each one
    /// that forgot the `NewContainer` arm broke the tag in a different way —
    /// the editability gate made every field and block button inert.
    #[test]
    fn a_new_container_tag_has_the_expected_capabilities() {
        let camera_track = parse_group_tag("trak").unwrap();
        let entry = new_container_entry("test/example.camera_track", camera_track, "camera_track");
        let tag = TagFile::new("definitions/haloce_evolved/camera_track.json").unwrap();

        assert!(
            crate::core::document::value::is_editable_tag(&entry, &tag),
            "fields and block controls must be live for a new tag"
        );
        assert!(
            crate::app::browser::supports_rename_menu(&entry),
            "rename/move is the only way to correct a mistyped new-tag path"
        );
        // No `.ubulk` behind it, so there is nothing to pull out.
        assert!(
            !crate::app::browser::is_embedded_tag_entry(&entry),
            "a new tag has no embedded payload to extract"
        );
    }

    fn new_container_entry(display_path: &str, group_tag: u32, group_name: &str) -> TagEntry {
        let logical = display_path
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(display_path);
        let package = new_container_package(logical, group_name);
        TagEntry {
            key: new_tag_entry_key(&package),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: Some(group_name.to_owned()),
            location: TagEntryLocation::NewContainer {
                template: NewContainerTemplate::Donor {
                    container: 0,
                    rel_path: "Tags/other-camera_track.uasset".to_owned(),
                },
                package,
                group_tag,
            },
        }
    }

    /// A group the game ships no tag of is authorable when its wrapper can be
    /// derived, and refused when it cannot.
    ///
    /// Both halves matter. Only checking that `cinematic_scene` is allowed
    /// would pass just as well if the gate had been deleted outright, and the
    /// refusal is what keeps a tag from being created that could never be
    /// saved — the group's Unreal class names other packages, and no import map
    /// for those can be derived from the group alone.
    #[test]
    fn a_group_with_no_shipped_tag_is_authorable_only_when_its_wrapper_derives() {
        // Nothing to clone: the decision falls to whether the group is bare.
        let derived = new_container_template_for(None, "cinematic_scene")
            .expect("cinematic_scene ships no tag but its wrapper derives");
        assert!(matches!(
            derived,
            NewContainerTemplate::Derived { ref group } if group == "cinematic_scene"
        ));
        for group in ["scenario_hs_source_file", "flock", "point_physics"] {
            assert!(
                matches!(
                    new_container_template_for(None, group),
                    Ok(NewContainerTemplate::Derived { .. })
                ),
                "{group} is bare and should derive"
            );
        }

        // `object` and `unit` carry `AssetReference`, so there is nothing to
        // derive and nothing to clone.
        for group in ["object", "unit", "item", "device"] {
            let error = new_container_template_for(None, group)
                .expect_err("{group} must not be authorable without a donor");
            assert!(
                error.contains(group),
                "the refusal should name the group, got: {error}"
            );
        }

        // A donor always wins, bare or not: cloning is the path with the most
        // mileage on it and is right for every group the game actually ships.
        let donor =
            new_container_template_for(Some((3, "Tags/x-biped.uasset".to_owned())), "biped")
                .expect("a donor is always usable");
        assert!(matches!(
            donor,
            NewContainerTemplate::Donor { container: 3, .. }
        ));
    }

    /// Renaming a new tag must land on exactly the key and package that
    /// creating it at that path would have produced — the save and project-
    /// overlay paths identify the tag by them, so a second derivation that
    /// drifted would strand the renamed tag.
    #[test]
    fn renaming_a_new_tag_derives_the_same_identity_as_creating_it_there() {
        let created = new_container_package("objects/foo/bar", "camera_track");
        assert_eq!(created, "/Game/Tags/objects/foo/bar-camera_track");
        assert_eq!(
            new_tag_entry_key(&created),
            "newtag:/Game/Tags/objects/foo/bar-camera_track"
        );

        // The rename path normalizes its input first — backslashes, case, and
        // stray separators must not fork the identity.
        let renamed = new_container_package(
            &normalize_container_tag_rel("/Objects\\Foo//Bar/"),
            "camera_track",
        );
        assert_eq!(renamed, created);
    }

    #[test]
    fn dependency_candidate_index_matches_by_group_and_leaf_name() {
        let names = TagNameIndex::default();
        let bitmap = parse_group_tag("bitm").unwrap();
        let shader = parse_group_tag("rmsh").unwrap();
        let entries = vec![
            entry("objects/new/run.bitmap", bitmap),
            entry("objects/new/run.shader", shader),
        ];

        let index = build_dependency_candidate_index(&entries, &names);

        assert_eq!(
            index
                .get(&(bitmap, "run".to_owned()))
                .cloned()
                .unwrap_or_default(),
            vec!["objects\\new\\run".to_owned()]
        );
        assert_eq!(
            index
                .get(&(shader, "run".to_owned()))
                .cloned()
                .unwrap_or_default(),
            vec!["objects\\new\\run".to_owned()]
        );
    }

    #[test]
    fn folder_reference_rewrites_point_moved_tags_at_new_folder() {
        let names = TagNameIndex::default();
        let bitmap = parse_group_tag("bitm").unwrap();
        let root = Path::new("C:/kit/tags");
        let source = root.join("objects/old");
        let destination = root.join("objects/new/old");
        let entries = vec![abs_entry(
            root,
            "objects/old/decal_road_1.bitmap.bitmap",
            bitmap,
        )];

        let rewrites =
            build_folder_reference_rewrites(root, &source, &destination, &entries, &names);

        assert_eq!(
            rewrites
                .get(&(bitmap, "objects\\old\\decal_road_1.bitmap".to_owned()))
                .cloned(),
            Some("objects\\new\\old\\decal_road_1.bitmap".to_owned())
        );
    }

    #[test]
    fn rewrite_reference_prefilter_matches_ascii_case_insensitively() {
        let shader = parse_group_tag("rmsh").unwrap();
        let mut rewrites = HashMap::new();
        rewrites.insert(
            (shader, "objects\\characters\\bugger\\bugger".to_owned()),
            "zoeph_test\\bugger\\bugger".to_owned(),
        );
        let needles = rewrite_reference_needles(&rewrites);

        assert!(bytes_contain_any_ascii_case_insensitive(
            b"xx OBJECTS\\CHARACTERS\\BUGGER\\BUGGER yy",
            &needles
        ));
        assert!(!bytes_contain_any_ascii_case_insensitive(
            b"objects\\characters\\dervish\\dervish",
            &needles
        ));
    }

    #[test]
    fn affected_move_entries_include_moved_tags_and_external_dependents() {
        let shader = parse_group_tag("rmsh").unwrap();
        let model = parse_group_tag("hlmt").unwrap();
        let old_shader = entry("objects/characters/jackal/jackal.shader", shader);
        let new_shader = entry("zoeph_test/jackal/jackal.shader", shader);
        let outside_model = entry("objects/characters/shared/shared.model", model);
        let unrelated = entry("objects/characters/brute/brute.model", model);
        let all_entries = vec![old_shader.clone(), outside_model.clone(), unrelated];
        let old_entries = vec![old_shader.clone()];
        let new_entries = vec![new_shader.clone()];
        let mut rewrites = HashMap::new();
        rewrites.insert(
            (shader, "objects\\characters\\jackal\\jackal".to_owned()),
            "zoeph_test\\jackal\\jackal".to_owned(),
        );
        let mut reverse = ReverseDependencyIndex::default();
        reverse.set_tag_dependencies(
            outside_model.key.clone(),
            vec![DependencyRef {
                group_tag: shader,
                rel_path: "objects\\characters\\jackal\\jackal".to_owned(),
            }],
        );
        reverse.set_tag_dependencies(
            old_shader.key.clone(),
            vec![DependencyRef {
                group_tag: shader,
                rel_path: "objects\\characters\\jackal\\jackal".to_owned(),
            }],
        );

        let affected = affected_move_rewrite_entries(
            &all_entries,
            &old_entries,
            &new_entries,
            &rewrites,
            Some(&reverse),
        );
        let affected_keys = affected
            .into_iter()
            .map(|entry| entry.key)
            .collect::<HashSet<_>>();

        assert_eq!(affected_keys.len(), 2);
        assert!(affected_keys.contains(&new_shader.key));
        assert!(affected_keys.contains(&outside_model.key));
    }

    fn loose(root: &Path, all_entries: Vec<TagEntry>) -> LoadedSourceData {
        LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root: root.to_path_buf(),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: None,
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries,
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        }
    }

    /// Fix Tag Dependencies uses the completed scan it already has. It used to
    /// rescan the whole folder on the UI thread every time.
    #[test]
    fn fix_dependencies_uses_the_completed_scan_without_rescanning() {
        // An empty folder on disk: a rescan would find nothing.
        let root = std::env::temp_dir().join(format!(
            "baboon-fix-deps-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let known = TagEntry {
            key: "file:objects/a.model".to_owned(),
            display_path: "objects/a.model".to_owned(),
            group_tag: u32::from_be_bytes(*b"hlmt"),
            group_name: None,
            location: TagEntryLocation::LooseFile(root.join("objects/a.model")),
        };
        let mut app = Baboon::for_test();
        app.install_loaded_source(loose(&root, vec![known]));
        let scanned = app
            .model.dependency_database_entries()
            .map(|entries| entries.len());

        let mut unscanned = Baboon::for_test();
        unscanned.install_loaded_source(loose(&root, Vec::new()));
        let waiting = unscanned.model.dependency_database_entries().is_err();

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            scanned,
            Ok(1),
            "the in-memory scan, not a rescan of the empty folder"
        );
        assert!(
            waiting,
            "no scan yet: say so rather than scan on the UI thread"
        );
    }

    /// A refresh patches the reference index with what changed. It used to
    /// drop it, so "References to" said the index was unavailable until a
    /// manual rebuild after any change the refresh noticed.
    #[test]
    fn a_refresh_patches_the_reference_index_instead_of_dropping_it() {
        let target = DependencyRef {
            group_tag: u32::from_be_bytes(*b"bitm"),
            rel_path: "shared\\texture".to_owned(),
        };
        let mut index = ReverseDependencyIndex::default();
        index.set_tag_dependencies("file:kept".to_owned(), vec![target.clone()]);
        index.set_tag_dependencies("file:gone".to_owned(), vec![target.clone()]);
        let root = std::env::temp_dir();
        let mut app = Baboon::for_test();
        app.install_loaded_source(LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::SingleFile {
                path: root.join("x"),
            },
            names: TagNameIndex::default(),
            game: None,
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: Some(index),
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        });

        app.apply_entry_index_refresh(
            0,
            EntryIndexRefresh {
                entries: Vec::new(),
                changed: true,
                added: 1,
                updated: 0,
                removed: 1,
                touched: Vec::new(),
                removed_keys: vec!["file:gone".to_owned()],
                touched_dependencies: vec![("file:new".to_owned(), vec![target.clone()])],
                errors: Vec::new(),
            },
            egui::Context::default(),
        );

        let index = app.model.kits[0]
            .source
            .as_ref()
            .and_then(|source| source.reverse_dependencies.as_ref())
            .expect("the reference index survives a refresh");
        let mut referrers = index
            .dependents_for(target.group_tag, &target.rel_path)
            .to_vec();
        referrers.sort();
        assert_eq!(referrers, ["file:kept", "file:new"]);
    }

    static CE_PAKS: std::sync::LazyLock<&'static str> =
        std::sync::LazyLock::new(|| crate::core::test_kits::leak(crate::core::test_kits::ce_paks()));

    fn find_entry<'a>(
        loaded: &'a crate::core::source::LoadedSourceData,
        group: &[u8; 4],
        path: &str,
    ) -> &'a TagEntry {
        let group_tag = u32::from_be_bytes(*group);
        loaded
            .entries
            .iter()
            .chain(loaded.all_entries.iter())
            .find(|entry| {
                entry.group_tag == group_tag
                    && entry.display_path.to_ascii_lowercase().replace('\\', "/") == path
            })
            .unwrap_or_else(|| panic!("no {path} entry in the mounted containers"))
    }

    /// Container tags carry no `want` stream, so their dependencies have to come
    /// out of the parsed tag. This walks the real path end to end: a Campaign
    /// Evolved biped must report outbound references, and the reverse index they
    /// feed must resolve back to the referenced tag's own entry — i.e. the
    /// reference strings inside a container tag normalize to the same key as the
    /// entry display paths built from the pak directory. Skips without the paks.
    #[test]
    fn campaign_evolved_container_tags_report_their_dependencies() {
        let paks = PathBuf::from(*CE_PAKS);
        if !paks.exists() {
            eprintln!("skip: CE paks not found");
            return;
        }
        let definitions = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let loaded =
            crate::core::source::load_iostore_container_set(paks, &TagNameIndex::default(), &definitions)
                .expect("mount CE container set");
        let names = TagNameIndex::load_game(&definitions, GameId::CampaignEvolved)
            .expect("load Campaign Evolved tag names");

        // The index build and every lookup read the complete set through
        // `full_entry_set`; a container mount keeps it in `entries`.
        assert!(loaded.all_entries.is_empty());
        assert_eq!(loaded.full_entry_set().len(), loaded.entries.len());
        assert!(
            loaded.full_entry_set().len() > 10_000,
            "expected the full CE tag set, got {}",
            loaded.full_entry_set().len()
        );

        let biped = find_entry(&loaded, b"bipd", "objects/characters/elite/elite.biped").clone();
        let deps = read_entry_dependencies(&loaded.source, &biped).expect("read biped deps");
        assert!(
            !deps.is_empty(),
            "elite.biped reported no dependencies; the container parse path is not running"
        );

        // The model reference must land on the entry the browser shows.
        let model = find_entry(&loaded, b"hlmt", "objects/characters/elite/elite.model").clone();
        let model_ref =
            dependency_entry_reference_path(&model, &names).expect("model reference path");
        let mut index = ReverseDependencyIndex::default();
        index.set_tag_dependencies(biped.key.clone(), deps);
        assert!(
            index
                .dependents_for(model.group_tag, &model_ref)
                .contains(&biped.key),
            "elite.model has no recorded referrer; container reference paths do not \
             normalize to entry display paths"
        );
    }

    /// The whole-corpus build, for when the cost of indexing containers is in
    /// question — it parses every tag. Ignored by default (minutes in a debug
    /// build); run with:
    ///   cargo test --release -- --ignored campaign_evolved_full_reference_index
    #[test]
    #[ignore = "parses all ~12k Campaign Evolved tags"]
    fn campaign_evolved_full_reference_index_resolves_referrers() {
        let paks = PathBuf::from(*CE_PAKS);
        if !paks.exists() {
            eprintln!("skip: CE paks not found");
            return;
        }
        let definitions = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let loaded =
            crate::core::source::load_iostore_container_set(paks, &TagNameIndex::default(), &definitions)
                .expect("mount CE container set");
        let names = TagNameIndex::load_game(&definitions, GameId::CampaignEvolved)
            .expect("load Campaign Evolved tag names");

        let started = std::time::Instant::now();
        let mut index = ReverseDependencyIndex::default();
        let mut failed = 0usize;
        for entry in loaded.full_entry_set() {
            match read_entry_dependencies(&loaded.source, entry) {
                Ok(deps) => index.set_tag_dependencies(entry.key.clone(), deps),
                Err(_) => failed += 1,
            }
        }
        eprintln!(
            "[perf] indexed {} tags in {:.1?} ({failed} unreadable)",
            loaded.full_entry_set().len(),
            started.elapsed()
        );
        assert_eq!(failed, 0, "some container tags could not be read");

        // A shared tag must come back with many referrers, and the elite biped
        // must be among the referrers of its own model.
        let model = find_entry(&loaded, b"hlmt", "objects/characters/elite/elite.model").clone();
        let model_ref =
            dependency_entry_reference_path(&model, &names).expect("model reference path");
        let referrers = index.dependents_for(model.group_tag, &model_ref);
        assert!(
            !referrers.is_empty(),
            "elite.model has no referrers in the full index"
        );
        let unreferenced = loaded
            .full_entry_set()
            .iter()
            .filter(|entry| {
                dependency_entry_reference_path(entry, &names)
                    .map(|rel| index.dependents_for(entry.group_tag, &rel).is_empty())
                    .unwrap_or(false)
            })
            .count();
        eprintln!(
            "[perf] {unreferenced} of {} tags are unreferenced",
            loaded.full_entry_set().len()
        );
        assert!(
            unreferenced < loaded.full_entry_set().len() / 2,
            "most tags came back unreferenced ({unreferenced}); reference paths are \
             probably not matching entry paths"
        );
    }
}
