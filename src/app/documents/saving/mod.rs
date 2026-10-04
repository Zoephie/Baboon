//! Definition discovery and validated new-tag output paths.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;
use crate::app::mods::in_place::ContainerSaveRoute;
use crate::app::mods::in_place::container_save_route;

use std::time::Instant;

impl Baboon {
    /// Applies `WorkerMessage::ExportFinished` to the application status.
    ///
    /// It leaves `chimp_level_job` alone: many exports share this message,
    /// and one finishing while a level export runs used to clear the level's
    /// progress and release the container-write guard while its worker was
    /// still reading. Only the level export's own message ends it.
    pub(in crate::app) fn handle_export_finished(&mut self, result: Result<String, String>) -> bool {
        self.status = match result {
            Ok(message) => message,
            Err(error) => error,
        };
        false
    }

    /// Applies `WorkerMessage::ChimpLevelExportFinished`: ends the level job
    /// it names, if that is still the current one, and reports the result.
    pub(in crate::app) fn handle_chimp_level_export_finished(
        &mut self,
        job: u64,
        result: Result<String, String>,
    ) -> bool {
        if self
            .chimp.chimp_level_job
            .as_ref()
            .is_some_and(|current| current.id == job)
        {
            self.chimp.chimp_level_job = None;
        }
        self.handle_export_finished(result)
    }

    /// Applies `WorkerMessage::ChimpLevelProgress`.
    ///
    /// Dropped when the kit it belongs to has closed: the export outlives the
    /// workspace that started it, and a level read that is no longer wanted
    /// should not keep writing over the status of whatever replaced it.
    pub(in crate::app) fn handle_chimp_level_progress(
        &mut self,
        kit: KitId,
        phase: ChimpLevelPhase,
        done: usize,
        total: usize,
    ) -> bool {
        if !self.kits.iter().any(|existing| existing.id == kit) {
            self.chimp.chimp_level_job = None;
            return true;
        }
        let Some(job) = self.chimp.chimp_level_job.as_mut() else {
            return false;
        };
        if job.kit != kit {
            return false;
        }
        // A new phase restarts the clock: an estimate carried over from reading
        // cells would describe work that is already finished.
        if job.phase != phase {
            job.phase = phase;
            job.phase_started = Instant::now();
        }
        job.done = done;
        job.total = total;
        false
    }

    /// Applies `WorkerMessage::ContainerDumpProgress`.
    pub(in crate::app) fn handle_container_dump_progress(
        &mut self,
        stamp: KitStamp,
        done: usize,
        total: usize,
    ) -> bool {
        if self.resolve_stamp(stamp).is_none() {
            return true;
        }
        let Some(job) = self
            .export.container_dump_job
            .as_mut()
            .filter(|job| job.kit == stamp.kit)
        else {
            return false;
        };
        job.done = done;
        job.total = total;
        false
    }

    /// Applies `WorkerMessage::ContainerDumpFinished`.
    ///
    /// The result goes to a notice rather than the status line: an extraction
    /// runs for minutes, the user has looked away, and a tally that expires on a
    /// timer is a tally nobody reads. The job is cleared either way — including
    /// for a workspace that closed mid-run, which would otherwise leave a
    /// progress bar on screen with nothing behind it.
    pub(in crate::app) fn handle_container_dump_finished(
        &mut self,
        stamp: KitStamp,
        result: Result<ContainerDumpReport, String>,
    ) -> bool {
        if self
            .export.container_dump_job
            .as_ref()
            .is_some_and(|job| job.kit == stamp.kit)
        {
            self.export.container_dump_job = None;
        }
        // Deliberately not gated on `resolve_stamp`: the files were written
        // whatever became of the workspace, and silently dropping the outcome of
        // an operation that just took ten minutes is worse than a late notice.
        match result {
            Ok(report) => {
                let mut message = format!(
                    "Wrote {} tag(s), {}.",
                    report.written,
                    format_byte_count(report.bytes as usize)
                );
                if report.skipped > 0 {
                    message.push_str(&format!(
                        "\n{} tag(s) skipped: only a mod provides them, so the game ships no copy \
                         to extract.",
                        report.skipped
                    ));
                }
                if report.failed > 0 {
                    message.push_str(&format!("\n{} tag(s) failed:", report.failed));
                    for failure in &report.failures {
                        message.push_str(&format!("\n  {failure}"));
                    }
                    if report.failed > report.failures.len() {
                        message.push_str(&format!(
                            "\n  ...and {} more",
                            report.failed - report.failures.len()
                        ));
                    }
                }
                self.status = if report.cancelled {
                    format!("Extraction cancelled after {} tag(s)", report.written)
                } else {
                    format!("Extracted {} tag(s)", report.written)
                };
                self.operation_notice = Some(OperationNotice {
                    title: if report.cancelled {
                        "Extraction cancelled".to_owned()
                    } else {
                        "Extraction finished".to_owned()
                    },
                    message,
                    // A cancelled run did what was asked of it. Only the tags
                    // that could not be written are a failure.
                    failed: report.failed > 0,
                });
            }
            Err(error) => {
                self.status = format!("Extraction failed: {error}");
                self.operation_notice = Some(OperationNotice {
                    title: "Extraction failed".to_owned(),
                    message: error,
                    failed: true,
                });
            }
        }
        false
    }

    /// Applies `WorkerMessage::EntryIndexSaved`, rejecting stale source generations.
    pub(in crate::app) fn handle_entry_index_saved(
        &mut self,
        stamp: KitStamp,
        path: PathBuf,
        result: Result<(), String>,
    ) -> bool {
        // Reports through the global status line only.
        let Some(kit_index) = self.resolve_stamp(stamp) else {
            return true;
        };
        match result {
            Ok(()) => {
                if !self.kits[kit_index].index_jobs.references_for_entry_index {
                    self.status = format!("Index saved: {}", path.display());
                }
            }
            Err(error) => {
                self.status = format!("Index save failed: {} ({error})", path.display());
            }
        }
        false
    }
}

pub(in crate::app) fn ordered_unique_keys<'a>(keys: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for key in keys {
        if seen.insert(key.clone()) {
            ordered.push(key.clone());
        }
    }
    ordered
}

pub(in crate::app) fn save_as_extension(app: &Baboon, entry: &TagEntry) -> Option<String> {
    app.names()
        .name_for(entry.group_tag)
        .or_else(|| group_tag_to_extension(entry.group_tag))
        .map(|extension| extension.trim().to_owned())
        .filter(|extension| !extension.is_empty())
}

pub(in crate::app) fn register_saved_copy_in_loaded_source(
    source: &mut LoadedSourceData,
    path: &Path,
) -> Result<bool, String> {
    let TagSource::LooseFolder { root, .. } = &source.source else {
        return Ok(false);
    };
    let Some(path) = crate::core::source::path_on_root(root, path)
        .map_err(|error| format!("Could not resolve saved tag path: {error}"))?
    else {
        return Ok(false);
    };
    let Some(entry) = loose_file_entry(root, &path, &source.names)
        .map_err(|error| format!("Could not inspect saved tag: {error:#}"))?
    else {
        return Ok(false);
    };
    // The folder tree is re-read from disk, so pending (empty) folders are
    // not needed here.
    source.upsert_entry(entry, &[]);
    Ok(true)
}

pub(in crate::app) fn save_as_file_name(entry: &TagEntry, extension: Option<&str>) -> String {
    let path = match &entry.location {
        TagEntryLocation::LooseFile(path) => path,
        TagEntryLocation::Monolithic { .. }
        | TagEntryLocation::Container { .. }
        | TagEntryLocation::NewContainer { .. } => Path::new(&entry.display_path),
    };
    let mut file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .or_else(|| {
            Path::new(&entry.display_path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| clean_file_name(&entry.display_path));
    if Path::new(&file_name).extension().is_none() {
        if let Some(extension) = extension {
            file_name.push('.');
            file_name.push_str(extension);
        }
    }
    file_name
}

pub(in crate::app) fn save_as_start_dir(entry: &TagEntry) -> Option<PathBuf> {
    match &entry.location {
        TagEntryLocation::LooseFile(path) => path.parent().map(Path::to_path_buf),
        TagEntryLocation::Monolithic { .. }
        | TagEntryLocation::Container { .. }
        | TagEntryLocation::NewContainer { .. } => None,
    }
}

pub(in crate::app) fn entries_for_keys(source: &LoadedSourceData, keys: &[String]) -> Vec<TagEntry> {
    let key_set = keys.iter().map(String::as_str).collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    source
        .entries
        .iter()
        .chain(source.all_entries.iter())
        .filter(|entry| key_set.contains(entry.key.as_str()))
        .filter(|entry| seen.insert(entry.key.as_str()))
        .cloned()
        .collect()
}

fn clean_file_name(value: &str) -> String {
    let mut name = value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("tag")
        .trim()
        .to_owned();
    name.retain(|ch| !matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'));
    if name.is_empty() {
        "tag".to_owned()
    } else {
        name
    }
}

pub(in crate::app) fn available_definition_games() -> Vec<String> {
    let root = locate_definitions_root();
    let mut games = fs::read_dir(root)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            let path = entry.path();
            path.join("_meta.json")
                .is_file()
                .then(|| entry.file_name().to_string_lossy().trim().to_owned())
        })
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    games.sort();
    games.dedup();
    if games.is_empty() {
        games.push(GameId::Halo3.as_str().to_owned());
    }
    games
}

pub(in crate::app) fn load_new_tag_groups(game: &str) -> Result<Vec<NewTagGroup>, String> {
    let game_dir = locate_definitions_root().join(game);
    if !game_dir.parent().is_some_and(|root| root.is_dir()) {
        return Err(definitions_missing_message(&locate_definitions_root()));
    }
    let meta_path = game_dir.join("_meta.json");
    let bytes = fs::read(&meta_path).map_err(|error| {
        if !locate_definitions_root().is_dir() {
            definitions_missing_message(&locate_definitions_root())
        } else {
            format!("Could not read {}: {error}", meta_path.display())
        }
    })?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse {}: {error}", meta_path.display()))?;
    let Some(tag_index) = value.get("tag_index").and_then(Value::as_object) else {
        return Err(format!("{} is missing tag_index", meta_path.display()));
    };
    let mut groups = Vec::new();
    for (fourcc, name_value) in tag_index {
        let Some(name) = name_value.as_str() else {
            continue;
        };
        let Some(group_tag) = parse_group_tag(fourcc) else {
            continue;
        };
        let disk_schema_path = game_dir.join(format!("{name}.json"));
        if !disk_schema_path.is_file() {
            continue;
        }
        groups.push(NewTagGroup {
            group_tag,
            name: name.to_owned(),
            schema_path: disk_schema_path,
            extension: group_tag_to_extension(group_tag)
                .unwrap_or(name)
                .trim()
                .to_owned(),
        });
    }
    groups.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.group_tag.cmp(&b.group_tag))
    });
    Ok(groups)
}

pub(in crate::app) fn new_tag_output_path_from_dialog(
    tags_root: &Path,
    picked_path: &Path,
    extension: &str,
) -> Result<(PathBuf, String), String> {
    let extension = extension.trim_start_matches('.');
    let mut output = picked_path.to_path_buf();
    output.set_extension(extension);
    let root = lexical_normalize_path(tags_root);
    let output = lexical_normalize_path(&output);
    if !output.starts_with(&root) {
        return Err("Choose a location inside the loaded tags folder".to_owned());
    }
    let rel = output
        .strip_prefix(&root)
        .map_err(|_| "Choose a location inside the loaded tags folder".to_owned())?;
    if rel.as_os_str().is_empty()
        || rel.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return Err("Choose a tag name inside the loaded tags folder".to_owned());
    }
    let display = rel.to_string_lossy().replace('\\', "/");
    Ok((output, display))
}

pub(in crate::app) fn lexical_normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod level_export_job_tests;

impl Baboon {
    pub(in crate::app) fn save_current_tag(&mut self, ctx: &egui::Context) {
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        let Some(key) = self.kits[self.active].selected_key.clone() else {
            self.status = "No tag selected".to_owned();
            return;
        };
        // A brand-new (in-memory) container tag has no baseline to overwrite —
        // "Save" writes it as a new `_P` override container instead.
        if matches!(
            self.entry_for_key(&key).map(|entry| &entry.location),
            Some(TagEntryLocation::NewContainer { .. })
        ) {
            self.save_new_container_tag(&key);
            return;
        }
        // For a container tag, "Save" overwrites the tag inside the game's pak
        // in place, which is destructive and is not how anyone should be
        // shipping a change — so it is an expert-mode route now. Everyone else
        // gets the export, which is the supported one.
        if self.current_source_is_container() {
            match container_save_route(
                self.prefs.expert_mode,
                self.prefs.confirm_container_overwrite,
            ) {
                ContainerSaveRoute::ExportReview => {
                    self.status = "Your change is kept in this workspace — export it as a mod to \
                                   put it in the game"
                        .to_owned();
                    self.export_mod();
                }
                ContainerSaveRoute::ConfirmOverwriteInPlace => {
                    self.mods.overwrite_confirm = Some(OverwriteConfirm {
                        kit: self.active_kit_id(),
                        key,
                    });
                }
                ContainerSaveRoute::OverwriteInPlace => {
                    self.begin_overwrite_current_tag_in_place(&key, ctx)
                }
            }
            return;
        }
        match self.save_tag_by_key(&key) {
            Ok(path) => self.status = format!("Saved {}", path.display()),
            Err(error) => self.status = format!("Save failed: {error}"),
        }
    }

    pub(in crate::app) fn save_tag_by_key(&mut self, key: &str) -> Result<PathBuf, String> {
        if self.refuse_read_only_edit(self.active) {
            return Err(self.status.clone());
        }
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return Err("Selected tag is no longer in the source".to_owned());
        };
        let Some(doc) = self.kits[self.active].parsed_tags.get(key) else {
            return Err("Load the selected tag before saving".to_owned());
        };
        if let Some(reason) = unsaveable_reason(&entry, &doc.tag) {
            return Err(reason.to_owned());
        }
        let TagEntryLocation::LooseFile(path) = &entry.location else {
            // Container tags are writable, just not through the loose-file
            // path — reaching here means a caller skipped the container
            // routing, so say that rather than blaming a monolithic cache.
            return Err(match &entry.location {
                TagEntryLocation::Container { .. } | TagEntryLocation::NewContainer { .. } => {
                    "Container tags cannot be saved as loose files".to_owned()
                }
                _ => "Monolithic cache tags are read-only".to_owned(),
            });
        };
        let output = path.clone();
        doc.tag
            .write_atomic(&output)
            .map_err(|error| error.to_string())?;
        // What the tag now points at, from the document just written.
        let dependencies = {
            let mut refs = Vec::new();
            collect_tag_dependency_refs(doc.tag.root(), &mut refs);
            refs
        };
        if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(key) {
            doc.dirty.clear();
        }
        // The save also writes the index row, so the periodic refresh will
        // not see this file change; the shader grid has to hear it here.
        if is_render_method_layout_group(entry.group_tag) {
            self.kits[self.active].forget_render_methods();
        }
        self.record_saved_tag_in_indexes(&entry, dependencies);
        Ok(output)
    }

    /// Bring the on-disk indexes and the reference index up to date with a tag
    /// the user just saved.
    ///
    /// A plain Save touched neither. The next periodic refresh then saw the
    /// file's new modified time as a change, and (before refreshes were
    /// patched in) dropped the whole reference index for it. Writing the row
    /// here means the refresh sees nothing to do, and the references are the
    /// ones the saved document holds.
    ///
    /// The row and the references are written in one transaction, so a
    /// failure leaves both as they were and the refresh picks the change up;
    /// the failure goes to the terminal, with the other index warnings.
    pub(in crate::app) fn record_saved_tag_in_indexes(&mut self, entry: &TagEntry, dependencies: Vec<DependencyRef>) {
        let Some(source) = self.source_mut() else {
            return;
        };
        if let (TagSource::LooseFolder { root, .. }, Some(game)) =
            (&source.source, source.game.map(GameId::as_str))
            && !source.all_entries.is_empty()
            && let Err(error) = crate::core::source::upsert_entry_with_dependencies(
                game,
                root,
                entry,
                Some(&dependencies),
            )
        {
            let _ = self.tx.send(WorkerMessage::TerminalLine(format!(
                "Warning: could not record {} in the tag index: {error:#}",
                entry.display_path
            )));
        }
        self.kits[self.active].set_tag_references(&entry.key, Some(dependencies));
    }

    pub(in crate::app) fn save_current_tag_as(&mut self) {
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        let Some(key) = self.kits[self.active].selected_key.clone() else {
            self.status = "No tag selected".to_owned();
            return;
        };
        // For a container tag, "Save As" opens the rename dialog in duplicate
        // mode (new name, no reference redirect) and writes an override.
        if self.current_source_is_container() {
            self.open_container_duplicate(&key);
            return;
        }
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.status = "Selected tag is no longer in the source".to_owned();
            return;
        };
        let Some(doc) = self.kits[self.active].parsed_tags.get(&key) else {
            self.status = "Load the selected tag before saving".to_owned();
            return;
        };
        if let Some(reason) = unsaveable_reason(&entry, &doc.tag) {
            self.status = reason.to_owned();
            return;
        }

        let extension = save_as_extension(self, &entry);
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save Current Tag As")
            .set_file_name(save_as_file_name(&entry, extension.as_deref()));
        if let Some(parent) = save_as_start_dir(&entry) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(extension) = extension.as_deref() {
            dialog = dialog.add_filter("Tag file", &[extension]);
        }
        let Some(mut output) = dialog.save_file() else {
            return;
        };
        if output.extension().is_none() {
            if let Some(extension) = extension.as_deref() {
                output.set_extension(extension);
            }
        }

        match doc.tag.write_atomic(&output) {
            Ok(()) => {
                self.status = match self.register_saved_copy_if_in_loaded_folder(&output) {
                    Ok(_) => format!("Saved copy to {}", output.display()),
                    Err(error) => format!(
                        "Saved copy to {}, but did not update browser: {error}",
                        output.display()
                    ),
                };
            }
            Err(error) => self.status = format!("Save As failed: {error}"),
        }
    }
}

#[cfg(test)]
mod saved_tag_index_tests;

#[cfg(test)]
mod save_as_tests;
