//! Building the reverse-dependency index, listing unreferenced tags, and fixing
//! a tag's broken dependencies against the tags that exist.

use super::*;
use crate::app::kits::terminal::trim_terminal_lines;
use crate::app::tag_ops::refactor::send_folder_refactor_progress;

impl Baboon {
    pub(in crate::app) fn fix_current_tag_dependencies(&mut self) {
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        let Some(key) = self.kits[self.active].selected_key.clone() else {
            self.status = "No tag selected".to_owned();
            return;
        };
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.status = "Selected tag is no longer in the source".to_owned();
            return;
        };
        let TagEntryLocation::LooseFile(_) = entry.location else {
            self.status = "Fix Tag Dependencies requires a loose-folder tag".to_owned();
            return;
        };
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Fix Tag Dependencies requires a loaded tags folder".to_owned();
            return;
        };

        let entries = match self.dependency_database_entries() {
            Ok(entries) => entries,
            Err(error) => {
                self.status = format!("Could not build dependency database: {error}");
                return;
            }
        };
        let names = self.names().clone();
        let index = build_dependency_candidate_index(&entries, &names);
        let Some(doc) = self.kits[self.active].parsed_tags.get_mut(&key) else {
            self.status = "Load the selected tag before fixing dependencies".to_owned();
            return;
        };
        if doc.tag.endian != Endian::Le {
            self.status = "Only little-endian loose tags can be edited".to_owned();
            return;
        }

        let report = fix_tag_dependencies_in_tag(&mut doc.tag, &root, &names, &index);
        if report.fixed > 0 {
            doc.dirty.touch();
        }
        let status = report.status();
        self.terminal
            .lines
            .extend(report.lines.into_iter().map(TerminalLineEntry::new));
        trim_terminal_lines(&mut self.terminal.lines);
        self.terminal.scroll_to_bottom = true;
        self.status = status;
    }

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

    pub(in crate::app) fn show_unreferenced_tags(&mut self) {
        match self.unreferenced_entries() {
            Some(entries) => {
                let note = entries
                    .is_empty()
                    .then(|| "Every tag is referenced by at least one other tag.".to_owned());
                self.search.query_results = Some(TagQueryResults {
                    kit: self.active_kit_id(),
                    title: format!("Unreferenced tags ({})", entries.len()),
                    entries,
                    annotations: Vec::new(),
                    note,
                    ref_target: None,
                });
            }
            None => {
                self.search.query_results = Some(TagQueryResults {
                    kit: self.active_kit_id(),
                    title: "Unreferenced tags".to_owned(),
                    entries: Vec::new(),
                    annotations: Vec::new(),
                    note: Some(self.reference_index_unavailable_note()),
                    ref_target: None,
                });
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
        self.begin_build_reverse_dependencies_inner(ctx, force, false);
    }

    pub(in crate::app) fn begin_build_reverse_dependencies_for_entry_index(&mut self, ctx: egui::Context) {
        self.begin_build_reverse_dependencies_inner(ctx, false, true);
    }

    pub(in crate::app) fn begin_build_reverse_dependencies_inner(
        &mut self,
        ctx: egui::Context,
        force: bool,
        paired_entry_index_build: bool,
    ) {
        if self.kits[self.active].index_jobs.building_references
            || self.kits[self.active].scanning_entries
        {
            return;
        }
        let Some(source) = self.source() else {
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
            if let Some(source) = self.source_mut() {
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
                if !self.kits[self.active].scanning_entries {
                    self.status = "Indexing tags, then building reference index…".to_owned();
                }
                self.begin_scan_all_entries_with_label(
                    ctx,
                    "Indexing tags, then building reference index...",
                );
            }
            return;
        }
        let tag_source = source.source.clone();
        let stamp = self.kit_stamp();
        let tx = self.tx.clone();
        self.kits[self.active].index_jobs.building_references = true;
        self.kits[self.active]
            .index_jobs
            .references_changed_during_build
            .clear();
        self.kits[self.active].index_jobs.references_for_entry_index = paired_entry_index_build;
        self.kits[self.active].index_jobs.reference_progress = Some(ReferenceIndexProgressState {
            label: "Building reference index...".to_owned(),
            processed: 0,
            total: entries.len(),
        });
        if paired_entry_index_build {
            self.show_entry_index_wait_notice = true;
        }
        self.status = "Building reference index…".to_owned();
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

#[cfg(test)]
mod dependency_tests;

#[cfg(test)]
mod dependency_database_tests;

#[cfg(test)]
mod refresh_reference_tests;

#[cfg(test)]
mod container_dependency_tests;
