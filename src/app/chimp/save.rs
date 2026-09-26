//! Chimp saving: the save and discard dialogs, rebuilding dirty packages and writing mods.
//! It owns getting edited packages onto disk; editing and extracting belong elsewhere.

use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ChimpSaveMode {
    #[default]
    ExportMod,
    OverwriteSources,
}

pub(super) struct ChimpSaveDialog {
    mode: ChimpSaveMode,
    name: String,
    folder: PathBuf,
    overwrite_acknowledged: bool,
    pending_close_action: Option<PendingCloseAction>,
}

enum ChimpSaveAction {
    Export(PathBuf),
    Overwrite,
}

impl Baboon {
    fn chimp_default_output_folder(&self, kit_index: usize) -> Option<PathBuf> {
        let root = match &self.kits.get(kit_index)?.source.as_ref()?.source {
            TagSource::IoStoreContainerSet { root, .. } => root,
            _ => return None,
        };
        Some(
            self.prefs
                .chimp_output_dir
                .clone()
                .unwrap_or_else(|| root.clone()),
        )
    }

    pub(in crate::app) fn open_chimp_save_dialog(&mut self, kit_index: usize) {
        self.open_chimp_save_dialog_with_pending(kit_index, None);
    }

    pub(super) fn open_chimp_save_dialog_for_close(
        &mut self,
        kit_index: usize,
        action: PendingCloseAction,
    ) -> bool {
        self.open_chimp_save_dialog_with_pending(kit_index, Some(action))
    }

    pub(in crate::app) fn has_chimp_save_dialog(&self) -> bool {
        self.kits.iter().any(|kit| kit.chimp.save_dialog.is_some())
    }

    fn open_chimp_save_dialog_with_pending(
        &mut self,
        kit_index: usize,
        pending_close_action: Option<PendingCloseAction>,
    ) -> bool {
        let dirty = self.kits[kit_index]
            .chimp
            .documents
            .values()
            .filter(|document| document.dirty)
            .count();
        if dirty == 0 {
            self.status = "Chimp has no modified packages to save".to_owned();
            return false;
        }
        let Some(folder) = self.chimp_default_output_folder(kit_index) else {
            self.status = "Chimp does not have a Paks output folder".to_owned();
            return false;
        };
        self.kits[kit_index].chimp.save_dialog = Some(ChimpSaveDialog {
            mode: ChimpSaveMode::ExportMod,
            name: "ChimpMod".to_owned(),
            folder,
            overwrite_acknowledged: false,
            pending_close_action,
        });
        true
    }

    pub(in crate::app) fn draw_chimp_discard_window(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.chimp_discard_prompt.as_ref() else {
            return;
        };
        let kit = prompt.kit;
        let packages = prompt.packages.clone();
        let pending_action = prompt.pending_action.clone();
        let error = prompt.error.clone();
        let mut open = true;
        let mut discard = false;
        let mut save = false;
        let mut cancel = false;

        egui::Window::new("Discard Chimp changes?")
            .id(egui::Id::new("chimp_discard_changes"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(if pending_action.is_some() {
                        "The following modified Chimp packages must be saved or discarded before closing."
                    } else {
                        "Every listed Chimp package will return to its original source data."
                    })
                    .color(text_dark()),
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for package in &packages {
                            ui.label(RichText::new(package).color(text_dark()).monospace());
                        }
                    });
                if let Some(error) = error.as_deref() {
                    ui.add_space(6.0);
                    ui.colored_label(Color32::from_rgb(180, 48, 40), error);
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "This removes the unsaved recovery copy. Exported mods and source PAK changes already saved are not affected, and this cannot be undone.",
                    )
                    .color(Color32::from_rgb(210, 120, 90)),
                );
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if pending_action.is_some()
                        && ui.button("Save Chimp Changes…").clicked()
                    {
                        save = true;
                    }
                    if ui.button("Discard Changes").clicked() {
                        discard = true;
                    }
                });
            });

        if !open || cancel {
            self.chimp_discard_prompt = None;
            return;
        }

        let Some(index) = self.resolve_kit(kit) else {
            self.chimp_discard_prompt = None;
            return;
        };

        if save {
            let Some(action) = pending_action else {
                return;
            };
            self.chimp_discard_prompt = None;
            if !self.open_chimp_save_dialog_for_close(index, action.clone()) {
                self.chimp_discard_prompt = Some(ChimpDiscardPrompt {
                    kit,
                    packages,
                    pending_action: Some(action),
                    error: Some(self.status.clone()),
                });
            }
        } else if discard {
            self.chimp_discard_prompt = None;
            match self.discard_chimp_packages(index, &packages) {
                Ok(count) => {
                    self.active = index;
                    self.status = format!("Discarded {count} modified Chimp package(s)");
                    if let Some(action) = pending_action {
                        self.request_close_action(action, ctx);
                    }
                }
                Err(error) => {
                    self.chimp_discard_prompt = Some(ChimpDiscardPrompt {
                        kit,
                        packages,
                        pending_action,
                        error: Some(error),
                    });
                }
            }
        }
    }

    pub(in crate::app) fn draw_chimp_save_window(&mut self, ctx: &egui::Context) {
        let Some(kit_index) = self
            .kits
            .iter()
            .position(|kit| kit.chimp.save_dialog.is_some())
        else {
            return;
        };
        let dirty_packages = self.chimp_dirty_packages(kit_index);
        let source_containers: Vec<PathBuf> = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => {
                let mut paths: Vec<_> = dirty_packages
                    .iter()
                    .filter_map(|package| {
                        let document = self.kits[kit_index].chimp.documents.get(package)?;
                        world
                            .containers()
                            .get(document.provider.container)
                            .map(|container| container.path.clone())
                    })
                    .collect();
                paths.sort();
                paths.dedup();
                paths
            }
            _ => Vec::new(),
        };
        let mut close = false;
        let mut action = None;
        let expert_mode = self.prefs.expert_mode;
        let dialog = self.kits[kit_index]
            .chimp
            .save_dialog
            .as_mut()
            .expect("checked above");
        // Overwriting the installed game's own containers is an expert-mode
        // route. A dialog left on that mode when expert mode is turned off
        // would still act on it, so the mode is corrected here rather than only
        // hidden below.
        if !expert_mode && dialog.mode == ChimpSaveMode::OverwriteSources {
            dialog.mode = ChimpSaveMode::ExportMod;
            dialog.overwrite_acknowledged = false;
        }
        egui::Window::new("Save Chimp changes")
            .id(egui::Id::new("chimp_save_changes"))
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} modified Unreal package(s) will be saved together.",
                    dirty_packages.len()
                ));
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for package in &dirty_packages {
                            ui.label(package);
                        }
                    });
                ui.separator();
                // Without expert mode there is one route, so it is stated
                // rather than offered as a choice of one.
                if expert_mode {
                    let mode_before = dialog.mode;
                    ui.radio_value(
                        &mut dialog.mode,
                        ChimpSaveMode::ExportMod,
                        "Export mod (recommended)",
                    );
                    ui.radio_value(
                        &mut dialog.mode,
                        ChimpSaveMode::OverwriteSources,
                        "Overwrite source PAKs",
                    );
                    if dialog.mode != mode_before {
                        dialog.overwrite_acknowledged = false;
                    }
                } else {
                    ui.label(
                        RichText::new(
                            "Saved as a mod, leaving the installed game untouched. Overwriting \
                             the game's own PAKs needs expert mode.",
                        )
                        .color(subtle_dark())
                        .small(),
                    );
                }
                ui.separator();

                let mut can_save;
                match dialog.mode {
                    ChimpSaveMode::ExportMod => {
                        ui.horizontal(|ui| {
                            ui.label("Mod name");
                            ui.text_edit_singleline(&mut dialog.name);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Destination");
                            ui.label(dialog.folder.display().to_string());
                            if ui.button("Browse…").clicked()
                                && let Some(folder) = rfd::FileDialog::new()
                                    .set_title("Choose Chimp mod folder")
                                    .set_directory(&dialog.folder)
                                    .pick_folder()
                            {
                                dialog.folder = folder;
                                dialog.overwrite_acknowledged = false;
                            }
                        });
                        let stem = chimp_mod_stem(&dialog.name);
                        can_save = !sanitize_mod_name(&dialog.name).is_empty();
                        if can_save {
                            ui.label(format!("Output: {stem}.utoc / .ucas / .pak"));
                        } else {
                            ui.colored_label(
                                Color32::from_rgb(210, 120, 80),
                                "Enter a file-safe mod name.",
                            );
                        }
                        let existing =
                            chimp_existing_triplet(&dialog.folder.join(format!("{stem}.utoc")));
                        if !existing.is_empty() {
                            ui.colored_label(
                                Color32::from_rgb(210, 120, 80),
                                format!("This will replace: {}", existing.join(", ")),
                            );
                            ui.checkbox(
                                &mut dialog.overwrite_acknowledged,
                                "Replace the existing mod container",
                            );
                            can_save &= dialog.overwrite_acknowledged;
                        }
                    }
                    ChimpSaveMode::OverwriteSources => {
                        ui.colored_label(
                            Color32::from_rgb(190, 72, 56),
                            "This replaces package indexes in the installed game containers.",
                        );
                        for path in &source_containers {
                            ui.label(path.display().to_string());
                        }
                        ui.checkbox(
                            &mut dialog.overwrite_acknowledged,
                            "I understand these source containers will be modified",
                        );
                        can_save = dialog.overwrite_acknowledged && !source_containers.is_empty();
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    let label = match dialog.mode {
                        ChimpSaveMode::ExportMod => "Export mod",
                        ChimpSaveMode::OverwriteSources => "Overwrite source PAKs",
                    };
                    if ui.add_enabled(can_save, egui::Button::new(label)).clicked() {
                        action = Some(match dialog.mode {
                            ChimpSaveMode::ExportMod => ChimpSaveAction::Export(
                                dialog
                                    .folder
                                    .join(format!("{}.utoc", chimp_mod_stem(&dialog.name))),
                            ),
                            ChimpSaveMode::OverwriteSources => ChimpSaveAction::Overwrite,
                        });
                    }
                });
            });
        let pending_close_action = dialog.pending_close_action.clone();
        if close || action.is_some() {
            self.kits[kit_index].chimp.save_dialog = None;
        }
        match action {
            Some(ChimpSaveAction::Export(output)) => {
                self.prefs.chimp_output_dir = output.parent().map(Path::to_path_buf);
                let action = ChimpSaveAction::Export(output);
                if !self.begin_chimp_write(kit_index, action, pending_close_action.clone(), ctx)
                    && let Some(action) = pending_close_action
                {
                    self.finish_chimp_close_after_save(kit_index, action, ctx);
                }
            }
            // Guarded here as well as in the dialog: this is the one action in
            // the app that edits the installed game's own containers, and it
            // should not be reachable by any route expert mode has not opened.
            Some(ChimpSaveAction::Overwrite) if !self.prefs.expert_mode => {
                self.status =
                    "Overwriting the game's own PAKs needs expert mode — save this as a mod \
                     instead"
                        .to_owned();
            }
            Some(ChimpSaveAction::Overwrite) => {
                let action = ChimpSaveAction::Overwrite;
                if !self.begin_chimp_write(kit_index, action, pending_close_action.clone(), ctx)
                    && let Some(action) = pending_close_action
                {
                    self.finish_chimp_close_after_save(kit_index, action, ctx);
                }
            }
            None => {}
        }
    }

    fn finish_chimp_close_after_save(
        &mut self,
        kit_index: usize,
        action: PendingCloseAction,
        ctx: &egui::Context,
    ) {
        let packages = self.chimp_dirty_packages(kit_index);
        if packages.is_empty() {
            self.request_close_action(action, ctx);
        } else {
            self.open_chimp_discard_prompt(
                kit_index,
                packages,
                Some(action),
                Some(self.status.clone()),
            );
        }
    }

    /// Rebuild every dirty package in the kit, recording the edit count each
    /// was rebuilt at.
    fn rebuild_dirty_chimp_documents(
        &self,
        kit_index: usize,
        world: &World,
    ) -> Result<Vec<ChimpRebuilt>, String> {
        let mut rebuilt = Vec::new();
        for (package, document) in self.kits[kit_index]
            .chimp
            .documents
            .iter()
            .filter(|(_, document)| document.dirty)
        {
            let (bytes, store) = rebuild_chimp_document(world, document)?;
            rebuilt.push(ChimpRebuilt {
                package: package.clone(),
                provider: document.provider.clone(),
                bytes,
                store,
                edits: document.edits,
            });
        }
        Ok(rebuilt)
    }

    /// Start a Chimp save. Returns false when nothing was started, in which
    /// case a pending close is the caller's to settle now; otherwise it is
    /// kept with the write and settled when the write finishes.
    ///
    /// The packages are rebuilt here, because the documents live on this
    /// thread. The container work — building and re-reading a mod container,
    /// or appending to the game's own — runs on a worker.
    fn begin_chimp_write(
        &mut self,
        kit_index: usize,
        action: ChimpSaveAction,
        pending_close: Option<PendingCloseAction>,
        ctx: &egui::Context,
    ) -> bool {
        let kit = self.kits[kit_index].id;
        if self.chimp_writes.contains_key(&kit) {
            self.status = "A Chimp save is already running".to_owned();
            return false;
        }
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return false;
        };
        let world = world.clone();
        let rebuilt = match self.rebuild_dirty_chimp_documents(kit_index, &world) {
            Ok(rebuilt) => rebuilt,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        if rebuilt.is_empty() {
            self.status = "Chimp has no modified packages to save".to_owned();
            return false;
        }
        match action {
            ChimpSaveAction::Export(output) => {
                if let Some(parent) = output.parent()
                    && let Err(error) = fs::create_dir_all(parent)
                {
                    self.status = format!("Could not create {}: {error}", parent.display());
                    return false;
                }
                self.status = format!("Building {}…", output.display());
                let panic_output = output.clone();
                spawn_worker(
                    &self.tx,
                    ctx,
                    move || {
                        let temporary = chimp_staging_utoc(&output);
                        let result = build_chimp_mod(&world, &rebuilt, &temporary);
                        WorkerMessage::ChimpModBuilt {
                            kit,
                            output,
                            temporary,
                            written: rebuilt.into_iter().map(ChimpWritten::from).collect(),
                            result,
                        }
                    },
                    move |error| WorkerMessage::ChimpModBuilt {
                        kit,
                        temporary: chimp_staging_utoc(&panic_output),
                        output: panic_output,
                        written: Vec::new(),
                        result: Err(error),
                    },
                );
            }
            ChimpSaveAction::Overwrite => {
                let mut groups: BTreeMap<usize, Vec<ChimpRebuilt>> = BTreeMap::new();
                for package in rebuilt {
                    groups
                        .entry(package.provider.container)
                        .or_default()
                        .push(package);
                }
                // The lease Duplicate, Rename and Delete take on the same
                // files: it refuses a tag-side write to any of these
                // containers while this one runs, and remounts every Chimp
                // workspace whose parsed TOCs the write makes stale.
                let mut leases = Vec::new();
                for &container in groups.keys() {
                    let utoc = world.containers()[container].path.clone();
                    match self
                        .acquire_container_write_lease(&utoc, ContainerWriteMode::AppendInPlace)
                    {
                        Ok(lease) => leases.push(self.park_container_write_lease(lease)),
                        Err(failure) => {
                            for id in leases {
                                if let Some(lease) = self.take_container_write_lease(id) {
                                    self.release_in_place_lease(
                                        lease,
                                        ContainerWriteOutcome::Unchanged,
                                    );
                                }
                            }
                            self.status = failure.to_string();
                            return false;
                        }
                    }
                }
                self.status = format!("Overwriting {} source container(s)…", groups.len());
                let panic_leases = leases.clone();
                spawn_worker(
                    &self.tx,
                    ctx,
                    move || {
                        let containers = groups.len();
                        let (touched, result) = overwrite_chimp_sources(&world, &groups);
                        WorkerMessage::ChimpSourcesOverwritten {
                            kit,
                            leases,
                            containers,
                            touched,
                            written: groups
                                .into_values()
                                .flatten()
                                .map(ChimpWritten::from)
                                .collect(),
                            result,
                        }
                    },
                    // A panic mid-write may have appended; say so, so the
                    // lease remounts Chimp rather than trusting its TOCs.
                    move |error| WorkerMessage::ChimpSourcesOverwritten {
                        kit,
                        leases: panic_leases,
                        containers: 0,
                        touched: true,
                        written: Vec::new(),
                        result: Err(error),
                    },
                );
            }
        }
        self.chimp_writes.insert(kit, pending_close);
        true
    }

    /// Clear `dirty` on the written packages that were not edited while the
    /// write ran, and drop their recovery checkpoints. A package edited in the
    /// meantime stays dirty: what was written is not what it now holds.
    fn settle_chimp_written(
        &mut self,
        kit_index: usize,
        written: &[ChimpWritten],
        reread_payloads: bool,
    ) -> Result<(), String> {
        let mut clean = Vec::new();
        for write in written {
            let Some(document) = self.kits[kit_index].chimp.documents.get_mut(&write.package)
            else {
                continue;
            };
            if reread_payloads {
                // What is on disk now, whether or not the document has moved
                // on: it is the baseline a discard returns to.
                document.original = write.bytes.clone();
            }
            if document.edits != write.edits {
                continue;
            }
            if reread_payloads && let Ok(payloads) = read_payloads(&document.header, &write.bytes) {
                document.payloads = payloads;
            }
            clean.push(write.package.clone());
        }
        self.clear_chimp_recovery_packages(kit_index, &clean)?;
        for package in &clean {
            if let Some(document) = self.kits[kit_index].chimp.documents.get_mut(package) {
                document.dirty = false;
            }
        }
        Ok(())
    }

    /// Run the close a save was started for, now that the save has settled.
    fn finish_chimp_write(&mut self, kit: KitId, ctx: &egui::Context) {
        let pending = self.chimp_writes.remove(&kit).flatten();
        if let (Some(action), Some(kit_index)) = (pending, self.kit_index(kit)) {
            self.finish_chimp_close_after_save(kit_index, action, ctx);
        }
    }

    /// Applies `WorkerMessage::ChimpModBuilt`: install the validated staging
    /// container over the output.
    pub(in crate::app) fn handle_chimp_mod_built(
        &mut self,
        kit: KitId,
        output: PathBuf,
        temporary: PathBuf,
        written: Vec<ChimpWritten>,
        result: Result<(), String>,
        ctx: &egui::Context,
    ) -> bool {
        self.install_chimp_mod(kit, &output, &temporary, &written, result, ctx);
        self.finish_chimp_write(kit, ctx);
        false
    }

    fn install_chimp_mod(
        &mut self,
        kit: KitId,
        output: &Path,
        temporary: &Path,
        written: &[ChimpWritten],
        result: Result<(), String>,
        ctx: &egui::Context,
    ) {
        if let Err(error) = result {
            remove_chimp_triplet(temporary);
            self.status = format!("Could not build {}: {error}", output.display());
            return;
        }
        let Some(kit_index) = self.kit_index(kit) else {
            remove_chimp_triplet(temporary);
            return;
        };
        // The active Chimp World (and possibly Baboon's tag mount, and possibly
        // a second workspace on the same install) can have the existing output
        // memory-mapped. The replacement is finished at a staging path first;
        // taking the lease idles every Chimp mount that covers it, unmapping
        // drops the tag mounts, and only then is the triplet swapped with a
        // rollback copy. The shipped game containers are never touched.
        let mut lease =
            match self.acquire_container_write_lease(output, ContainerWriteMode::Replace) {
                Ok(lease) => lease,
                Err(failure) => {
                    remove_chimp_triplet(temporary);
                    self.status = failure.to_string();
                    return;
                }
            };
        if let Err(failure) = self.unmap_leased_containers(&mut lease) {
            remove_chimp_triplet(temporary);
            self.status = failure.to_string();
            self.release_container_write_lease(lease, ContainerWriteOutcome::Unchanged, ctx);
            return;
        }
        let replaced = replace_chimp_triplet(temporary, output);
        // Remounting is the lease's job — it knows which workspaces it idled,
        // which is not necessarily this one: an output outside the game's
        // `Paks` was never mapped and never needed idling.
        let report = self.release_container_write_lease(
            lease,
            if replaced.is_ok() {
                ContainerWriteOutcome::Committed
            } else {
                ContainerWriteOutcome::Unchanged
            },
            ctx,
        );
        if let Err(error) = replaced {
            self.status = format!("Could not install {}: {error}", output.display());
            return;
        }
        if let Err(error) = self.settle_chimp_written(kit_index, written, false) {
            self.status = format!(
                "Built {} but could not clear Chimp recovery: {error}",
                output.display()
            );
            return;
        }
        self.status = format!(
            "Built {} modified Unreal package(s) into {}",
            written.len(),
            output.display()
        );
        if !report.reopen_failures.is_empty() {
            self.status.push_str(&format!(
                "; {} tag mount(s) could not be reopened",
                report.reopen_failures.len()
            ));
        }
    }

    /// Applies `WorkerMessage::ChimpSourcesOverwritten`.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app) fn handle_chimp_sources_overwritten(
        &mut self,
        kit: KitId,
        leases: Vec<ContainerLeaseId>,
        containers: usize,
        touched: bool,
        written: Vec<ChimpWritten>,
        result: Result<(), String>,
        ctx: &egui::Context,
    ) -> bool {
        // Released before anything else can return: a lease that outlived its
        // write would refuse every later write to these containers. Releasing
        // a touched one remounts every Chimp workspace covering it, this one
        // included, since each holds its own parsed copy of the TOC.
        let outcome = if touched {
            ContainerWriteOutcome::Committed
        } else {
            ContainerWriteOutcome::Unchanged
        };
        for id in leases {
            if let Some(lease) = self.take_container_write_lease(id) {
                self.release_in_place_lease(lease, outcome);
            }
        }
        self.drain_pending_chimp_remounts(ctx);
        match (result, self.kit_index(kit)) {
            (Err(error), _) => self.status = error,
            (Ok(()), None) => {}
            (Ok(()), Some(kit_index)) => {
                self.status = match self.settle_chimp_written(kit_index, &written, true) {
                    Ok(()) => format!(
                        "Overwrote {} modified Unreal package(s) across {containers} source \
                         container(s)",
                        written.len()
                    ),
                    Err(error) => format!(
                        "Overwrote the source packages, but could not clear Chimp recovery: \
                         {error}"
                    ),
                };
            }
        }
        self.finish_chimp_write(kit, ctx);
        false
    }
}

fn chimp_mod_stem(name: &str) -> String {
    let sanitized = sanitize_mod_name(name);
    if sanitized
        .get(sanitized.len().saturating_sub(2)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case("_p"))
    {
        sanitized
    } else {
        format!("{sanitized}_P")
    }
}

fn chimp_existing_triplet(path: &Path) -> Vec<String> {
    triplet(path)
        .into_iter()
        .filter(|path| path.exists())
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("container file")
                .to_owned()
        })
        .collect()
}

/// One dirty package, rebuilt for a save.
pub(super) struct ChimpRebuilt {
    package: String,
    provider: PackageProvider,
    bytes: Vec<u8>,
    store: blam_tags::iostore::container::header::StoreEntry,
    /// The document's [`ChimpDocument::edits`] when it was rebuilt.
    edits: u64,
}

/// What a finished save reports back about one package.
pub(in crate::app) struct ChimpWritten {
    package: String,
    bytes: Vec<u8>,
    edits: u64,
}

impl From<ChimpRebuilt> for ChimpWritten {
    fn from(rebuilt: ChimpRebuilt) -> Self {
        Self {
            package: rebuilt.package,
            bytes: rebuilt.bytes,
            edits: rebuilt.edits,
        }
    }
}

/// Where a mod container is built before it replaces `output`.
fn chimp_staging_utoc(output: &Path) -> PathBuf {
    output.with_file_name(format!(
        "{}.building.utoc",
        output
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Chimp_P")
    ))
}

/// Build the rebuilt packages into a mod container at `temporary` and read
/// every one back to check it survived exactly. Runs on a worker.
fn build_chimp_mod(
    world: &World,
    rebuilt: &[ChimpRebuilt],
    temporary: &Path,
) -> Result<(), String> {
    let overrides: Vec<PackageOverride<'_>> = rebuilt
        .iter()
        .map(|package| PackageOverride {
            archive: &world.archives()[package.provider.container],
            uasset_path: &package.provider.entry_path,
            bytes: package.bytes.clone(),
            store: package.store.clone(),
        })
        .collect();
    write_package_mod_container(&overrides, temporary).map_err(|error| error.to_string())?;
    drop(overrides);
    let archive = blam_tags::iostore::IoStoreArchive::open(temporary)
        .map_err(|error| format!("Could not reopen temporary mod: {error}"))?;
    for package in rebuilt {
        let provider = &package.provider;
        let source = &world.archives()[provider.container];
        let source_index = source
            .chunk_index_for(&provider.entry_path)
            .map_err(|error| error.to_string())?;
        let chunk_id = source
            .chunk_id(source_index)
            .map_err(|error| error.to_string())?;
        let saved_index = archive
            .find_chunk(&chunk_id)
            .ok_or_else(|| format!("Temporary mod is missing {}", provider.entry_path))?;
        let saved = archive
            .read_chunk(saved_index)
            .map_err(|error| error.to_string())?;
        if saved != package.bytes {
            return Err(format!(
                "Temporary mod did not preserve {} exactly",
                provider.entry_path
            ));
        }
    }
    Ok(())
}

/// Overwrite the rebuilt packages inside their own source containers,
/// restoring every `.utoc` if any container fails. Runs on a worker. The flag
/// says whether any container was written to, rollback or not.
fn overwrite_chimp_sources(
    world: &World,
    groups: &BTreeMap<usize, Vec<ChimpRebuilt>>,
) -> (bool, Result<(), String>) {
    let mut originals = BTreeMap::new();
    for &container in groups.keys() {
        let path = world.containers()[container].path.clone();
        match fs::read(&path) {
            Ok(bytes) => {
                originals.insert(container, (path, bytes));
            }
            Err(error) => {
                return (
                    false,
                    Err(format!(
                        "Could not stage rollback for {}: {error}",
                        path.display()
                    )),
                );
            }
        }
    }
    let mut failure = None;
    for (&container, packages) in groups {
        let replacements: Vec<_> = packages
            .iter()
            .map(|package| PackageReplacement {
                uasset_path: &package.provider.entry_path,
                rebuilt_bytes: &package.bytes,
                store: &package.store,
            })
            .collect();
        let path = &originals[&container].0;
        if let Err(error) =
            overwrite_packages_in_place_with(&world.archives()[container], path, &replacements)
        {
            failure = Some(format!("Could not overwrite {}: {error}", path.display()));
            break;
        }
    }
    let Some(mut error) = failure else {
        return (true, Ok(()));
    };
    let mut rollback_failures = Vec::new();
    for (path, bytes) in originals.values() {
        if let Err(rollback_error) = fs::write(path, bytes) {
            rollback_failures.push(format!("{}: {rollback_error}", path.display()));
        }
    }
    if !rollback_failures.is_empty() {
        error.push_str(&format!(
            "; rollback also failed for {}",
            rollback_failures.join(", ")
        ));
    }
    (true, Err(error))
}

fn remove_chimp_triplet(path: &Path) {
    remove_container_triplet(path);
}

fn replace_chimp_triplet(temporary: &Path, output: &Path) -> Result<(), String> {
    super::controller::swap_container_triplet(temporary, output)
        .map_err(|failure| failure.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A save rebuilds packages on the UI thread and writes them on a worker.
    /// An edit that lands in between is not in what was written, so that
    /// package has to stay dirty — and keep its own payloads.
    #[test]
    fn a_package_edited_during_a_save_stays_dirty() {
        let mut app = Baboon::for_test();
        for (package, edits) in [("/Game/A", 1), ("/Game/B", 2)] {
            let mut document = rename_fixture();
            document.package = package.to_owned();
            document.dirty = true;
            document.edits = edits;
            app.kits[0]
                .chimp
                .documents
                .insert(package.to_owned(), document);
        }
        let payloads_before = app.kits[0].chimp.documents["/Game/B"].payloads.clone();
        // Both were rebuilt at one edit; B took a second while the write ran.
        let written = ["/Game/A", "/Game/B"].map(|package| ChimpWritten {
            package: package.to_owned(),
            bytes: vec![7; 4],
            edits: 1,
        });
        app.settle_chimp_written(0, &written, true).unwrap();

        let documents = &app.kits[0].chimp.documents;
        assert!(!documents["/Game/A"].dirty, "written as it stands: clean");
        assert!(documents["/Game/B"].dirty, "edited mid-save: still dirty");
        assert_eq!(documents["/Game/B"].payloads, payloads_before);
        assert_eq!(
            documents["/Game/B"].original,
            vec![7; 4],
            "the discard baseline is what is on disk now"
        );
    }

    /// Closing a workspace while its Chimp save runs used to be moot — the
    /// save blocked the UI. Now the close waits and runs when the save lands.
    #[test]
    fn a_close_during_a_chimp_save_waits_for_it() {
        let mut app = Baboon::for_test();
        let kit = app.kits[0].id;
        app.chimp_writes.insert(kit, None);
        let ctx = egui::Context::default();
        app.request_close_action(PendingCloseAction::CloseKit(kit), &ctx);
        assert!(app.kit_index(kit).is_some(), "the close waits for the save");
        assert!(matches!(
            app.chimp_writes.get(&kit),
            Some(Some(PendingCloseAction::CloseKit(_)))
        ));

        let output = std::env::temp_dir().join("baboon-chimp-close-test/Mod_P.utoc");
        app.handle_chimp_mod_built(
            kit,
            output.clone(),
            chimp_staging_utoc(&output),
            Vec::new(),
            Err("stopped".to_owned()),
            &ctx,
        );
        assert!(app.chimp_writes.is_empty());
        assert!(app.kit_index(kit).is_none(), "and runs once it lands");
    }

    /// The overwrite's leases are parked for the worker. A failed write has
    /// to give them back, or every later write to those containers is refused.
    #[test]
    fn a_failed_source_overwrite_releases_its_leases() {
        let mut app = Baboon::for_test();
        let kit = app.kits[0].id;
        let utoc = std::env::temp_dir().join("baboon-chimp-lease-test/pakchunk0-Windows.utoc");
        let lease = app
            .acquire_container_write_lease(&utoc, ContainerWriteMode::AppendInPlace)
            .unwrap();
        let id = app.park_container_write_lease(lease);
        app.chimp_writes.insert(kit, None);
        app.handle_chimp_sources_overwritten(
            kit,
            vec![id],
            1,
            false,
            Vec::new(),
            Err("could not overwrite".to_owned()),
            &egui::Context::default(),
        );
        assert_eq!(app.status, "could not overwrite");
        assert!(app.chimp_writes.is_empty());
        let again = app
            .acquire_container_write_lease(&utoc, ContainerWriteMode::AppendInPlace)
            .expect("the container is writable again");
        app.release_in_place_lease(again, ContainerWriteOutcome::Unchanged);
    }

    #[test]
    fn chimp_mod_names_are_sanitized_and_priority_suffixed() {
        assert_eq!(chimp_mod_stem("My Cool Mod"), "My-Cool-Mod_P");
        assert_eq!(chimp_mod_stem("Already_p"), "Already_p");
        assert_eq!(chimp_mod_stem("../../unsafe"), "unsafe_P");
    }

    #[test]
    fn chimp_mod_stem_rejects_an_empty_name_at_the_dialog_boundary() {
        assert!(sanitize_mod_name(" ! ").is_empty());
        assert_eq!(chimp_mod_stem(" ! "), "_P");
    }

    #[test]
    fn output_triplet_replacement_replaces_all_files_and_cleans_backups() {
        let directory = std::env::temp_dir().join(format!("baboon-chimp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let incoming = directory.join("incoming.utoc");
        let output = directory.join("Chimp_P.utoc");
        for file in triplet(&incoming) {
            std::fs::write(file, b"new").unwrap();
        }
        for file in triplet(&output) {
            std::fs::write(file, b"old").unwrap();
        }
        replace_chimp_triplet(&incoming, &output).unwrap();
        for file in triplet(&output) {
            assert_eq!(std::fs::read(file).unwrap(), b"new");
        }
        assert!(!output.with_extension("utoc.previous").exists());
        assert!(!output.with_extension("ucas.previous").exists());
        assert!(!output.with_extension("pak.previous").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
