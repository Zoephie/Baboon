//! Chimp saving: the save and discard dialogs, rebuilding dirty packages and writing mods.
//! It owns getting edited packages onto disk; editing and extracting belong elsewhere.

use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ChimpSaveMode {
    #[default]
    ExportMod,
    OverwriteSources,
}

pub(in crate::app) struct ChimpSaveDialog {
    /// The workspace whose modified packages it saves.
    pub(super) kit: KitId,
    mode: ChimpSaveMode,
    name: String,
    folder: PathBuf,
    overwrite_acknowledged: bool,
    pending_close_action: Option<PendingCloseAction>,
}

pub(in crate::app) enum ChimpSaveAction {
    Export(PathBuf),
    Overwrite,
}

impl Baboon {


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

    /// Open the save dialog without the modified packages it normally
    /// requires, for tests that draw it.
    #[cfg(test)]
    pub(in crate::app) fn open_chimp_save_dialog_for_test(&mut self, kit_index: usize) {
        self.dialogs.open(ChimpSaveDialog {
            kit: self.model.kits[kit_index].id,
            mode: ChimpSaveMode::ExportMod,
            name: "ChimpMod".to_owned(),
            folder: PathBuf::from("/no/such/Paks"),
            overwrite_acknowledged: false,
            pending_close_action: None,
        });
    }



    fn open_chimp_save_dialog_with_pending(
        &mut self,
        kit_index: usize,
        pending_close_action: Option<PendingCloseAction>,
    ) -> bool {
        let dirty = self.model.kits[kit_index]
            .chimp
            .documents
            .values()
            .filter(|document| document.dirty)
            .count();
        if dirty == 0 {
            self.model.status = "Chimp has no modified packages to save".to_owned();
            return false;
        }
        let Some(folder) = self.model.chimp_default_output_folder(kit_index) else {
            self.model.status = "Chimp does not have a Paks output folder".to_owned();
            return false;
        };
        self.dialogs.open(ChimpSaveDialog {
            kit: self.model.kits[kit_index].id,
            mode: ChimpSaveMode::ExportMod,
            name: "ChimpMod".to_owned(),
            folder,
            overwrite_acknowledged: false,
            pending_close_action,
        });
        true
    }

    /// The discard prompt's Save: open the save dialog for its close, or put
    /// the prompt back saying why not.
    pub(super) fn save_chimp_before_close(&mut self, prompt: ChimpDiscardPrompt) {
        let ChimpDiscardPrompt {
            kit,
            packages,
            pending_action,
            ..
        } = prompt;
        let (Some(index), Some(action)) = (self.model.resolve_kit(kit), pending_action) else {
            return;
        };
        if !self.open_chimp_save_dialog_for_close(index, action.clone()) {
            self.dialogs.open(ChimpDiscardPrompt {
                kit,
                packages,
                pending_action: Some(action),
                error: Some(self.model.status.clone()),
            });
        }
    }

    /// The discard prompt's Discard: restore its packages and run its close,
    /// or put the prompt back saying why not.
    pub(super) fn discard_chimp_for_prompt(
        &mut self,
        prompt: ChimpDiscardPrompt,
        ctx: &egui::Context,
    ) {
        let Some(index) = self.model.resolve_kit(prompt.kit) else {
            return;
        };
        match self.discard_chimp_packages(index, &prompt.packages) {
            Ok(count) => {
                self.focus_kit(index);
                self.model.status = format!("Discarded {count} modified Chimp package(s)");
                if let Some(action) = prompt.pending_action {
                    self.request_close_action(action, ctx);
                }
            }
            Err(error) => {
                self.dialogs.open(ChimpDiscardPrompt {
                    error: Some(error),
                    ..prompt
                });
            }
        }
    }

    /// Act on the save dialog's choice for a kit.
    pub(super) fn save_chimp_changes(
        &mut self,
        kit: KitId,
        action: ChimpSaveAction,
        pending_close_action: Option<PendingCloseAction>,
        ctx: &egui::Context,
    ) {
        let Some(kit_index) = self.model.kit_index(kit) else {
            return;
        };
        match action {
            ChimpSaveAction::Export(output) => {
                self.model.prefs.chimp_output_dir = output.parent().map(Path::to_path_buf);
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
            ChimpSaveAction::Overwrite if !self.model.prefs.expert_mode => {
                self.model.status =
                    "Overwriting the game's own PAKs needs expert mode — save this as a mod \
                     instead"
                        .to_owned();
            }
            ChimpSaveAction::Overwrite => {
                let action = ChimpSaveAction::Overwrite;
                if !self.begin_chimp_write(kit_index, action, pending_close_action.clone(), ctx)
                    && let Some(action) = pending_close_action
                {
                    self.finish_chimp_close_after_save(kit_index, action, ctx);
                }
            }
        }
    }


    fn finish_chimp_close_after_save(
        &mut self,
        kit_index: usize,
        action: PendingCloseAction,
        ctx: &egui::Context,
    ) {
        let packages = self.model.chimp_dirty_packages(kit_index);
        if packages.is_empty() {
            self.request_close_action(action, ctx);
        } else {
            self.open_chimp_discard_prompt(
                kit_index,
                packages,
                Some(action),
                Some(self.model.status.clone()),
            );
        }
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
        let kit = self.model.kits[kit_index].id;
        if self.chimp.chimp_writes.contains_key(&kit) {
            self.model.status = "A Chimp save is already running".to_owned();
            return false;
        }
        let ChimpMount::Ready(world) = &self.model.kits[kit_index].chimp.mount else {
            return false;
        };
        let world = world.clone();
        let rebuilt = match self.model.rebuild_dirty_chimp_documents(kit_index, &world) {
            Ok(rebuilt) => rebuilt,
            Err(error) => {
                self.model.status = error;
                return false;
            }
        };
        if rebuilt.is_empty() {
            self.model.status = "Chimp has no modified packages to save".to_owned();
            return false;
        }
        match action {
            ChimpSaveAction::Export(output) => {
                if let Some(parent) = output.parent()
                    && let Err(error) = fs::create_dir_all(parent)
                {
                    self.model.status = format!("Could not create {}: {error}", parent.display());
                    return false;
                }
                self.model.status = format!("Building {}…", output.display());
                // Staged under the output's own file name: the writer stamps
                // CityHash64 of the file stem into the container as its id.
                let temporary = staging_utoc_for(&output);
                let panic_output = output.clone();
                let panic_temporary = temporary.clone();
                spawn_worker(
                    &self.tx,
                    ctx,
                    move || {
                        let result = temporary
                            .parent()
                            .map_or(Ok(()), fs::create_dir_all)
                            .map_err(|error| error.to_string())
                            .and_then(|()| build_chimp_mod(&world, &rebuilt, &temporary));
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
                        temporary: panic_temporary,
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
                            self.model.status = failure.to_string();
                            return false;
                        }
                    }
                }
                self.model.status = format!("Overwriting {} source container(s)…", groups.len());
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
        self.chimp.chimp_writes.insert(kit, pending_close);
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
            let Some(document) = self.model.kits[kit_index].chimp.documents.get_mut(&write.package)
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
            if let Some(document) = self.model.kits[kit_index].chimp.documents.get_mut(package) {
                document.dirty = false;
            }
        }
        Ok(())
    }

    /// Run the close a save was started for, now that the save has settled.
    fn finish_chimp_write(&mut self, kit: KitId, ctx: &egui::Context) {
        let pending = self.chimp.chimp_writes.remove(&kit).flatten();
        if let (Some(action), Some(kit_index)) = (pending, self.model.kit_index(kit)) {
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
            discard_staging(temporary);
            self.model.status = format!("Could not build {}: {error}", output.display());
            return;
        }
        let Some(kit_index) = self.model.kit_index(kit) else {
            discard_staging(temporary);
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
                    discard_staging(temporary);
                    self.model.status = failure.to_string();
                    return;
                }
            };
        if let Err(failure) = self.unmap_leased_containers(&mut lease) {
            discard_staging(temporary);
            self.model.status = failure.to_string();
            self.release_container_write_lease(lease, ContainerWriteOutcome::Unchanged, ctx);
            return;
        }
        let replaced = replace_chimp_triplet(temporary, output);
        discard_staging(temporary);
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
            self.model.status = format!("Could not install {}: {error}", output.display());
            return;
        }
        if let Err(error) = self.settle_chimp_written(kit_index, written, false) {
            self.model.status = format!(
                "Built {} but could not clear Chimp recovery: {error}",
                output.display()
            );
            return;
        }
        self.model.status = format!(
            "Built {} modified Unreal package(s) into {}",
            written.len(),
            output.display()
        );
        if !report.reopen_failures.is_empty() {
            self.model.status.push_str(&format!(
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
        match (result, self.model.kit_index(kit)) {
            (Err(error), _) => self.model.status = error,
            (Ok(()), None) => {}
            (Ok(()), Some(kit_index)) => {
                self.model.status = match self.settle_chimp_written(kit_index, &written, true) {
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

fn replace_chimp_triplet(temporary: &Path, output: &Path) -> Result<(), String> {
    crate::app::mods::container_write::swap_container_triplet(temporary, output)
        .map_err(|failure| failure.to_string())
}

impl Dialog for ChimpDiscardPrompt {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let prompt = &*self;
        let packages = &prompt.packages;
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
            .default_width(window_width(ctx, 520.0))
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
                        for package in packages {
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
            return false;
        }
        // Left open for the save or discard, which takes it back from the host.
        if save && pending_action.is_some() {
            cx.send(ChimpCommand::SaveBeforeClose);
        } else if discard {
            cx.send(ChimpCommand::Discard);
        }
        true
    }
}

impl Dialog for ChimpSaveDialog {
    /// One per workspace. With one for the whole app, a second workspace's
    /// Ctrl+S replaced the first's dialog, and with it the close the first
    /// was waiting on.
    fn instance(&self) -> u64 {
        self.kit.0
    }

    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let model = cx.model;
        let Some(kit_index) = model.kit_index(self.kit) else {
            return false;
        };
        let dirty_packages = model.chimp_dirty_packages(kit_index);
        let source_containers: Vec<PathBuf> = match &model.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => {
                let mut paths: Vec<_> = dirty_packages
                    .iter()
                    .filter_map(|package| {
                        let document = model.kits[kit_index].chimp.documents.get(package)?;
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
        let expert_mode = model.prefs.expert_mode;
        let kit = self.kit;
        let dialog = &mut *self;
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
            .default_width(window_width(ctx, 620.0))
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
        let chosen = action.is_some();
        if let Some(action) = action {
            cx.send(ChimpCommand::Save {
                kit,
                action,
                pending_close_action,
            });
        }
        !close && !chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::shell::{apply_next_worker_message, apply_one_worker_message};

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
            app.model.kits[0]
                .chimp
                .documents
                .insert(package.to_owned(), document);
        }
        let payloads_before = app.model.kits[0].chimp.documents["/Game/B"].payloads.clone();
        // Both were rebuilt at one edit; B took a second while the write ran.
        let written = ["/Game/A", "/Game/B"].map(|package| ChimpWritten {
            package: package.to_owned(),
            bytes: vec![7; 4],
            edits: 1,
        });
        app.settle_chimp_written(0, &written, true).unwrap();

        let documents = &app.model.kits[0].chimp.documents;
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
        let kit = app.model.kits[0].id;
        app.chimp.chimp_writes.insert(kit, None);
        let ctx = egui::Context::default();
        app.request_close_action(PendingCloseAction::CloseKit(kit), &ctx);
        assert!(app.model.kit_index(kit).is_some(), "the close waits for the save");
        assert!(matches!(
            app.chimp.chimp_writes.get(&kit),
            Some(Some(PendingCloseAction::CloseKit(_)))
        ));

        let output = std::env::temp_dir().join("baboon-chimp-close-test/Mod_P.utoc");
        app.handle_chimp_mod_built(
            kit,
            output.clone(),
            staging_utoc_for(&output),
            Vec::new(),
            Err("stopped".to_owned()),
            &ctx,
        );
        assert!(app.chimp.chimp_writes.is_empty());
        assert!(app.model.kit_index(kit).is_none(), "and runs once it lands");
    }

    /// A Chimp mod is staged under its own file name, in a folder of its own
    /// beside the output: the container id the writer stamps is CityHash64 of
    /// the file stem, and a `Mod_P.building` stem shipped a container that
    /// declared itself under a name nothing else uses. A failed build leaves
    /// neither the staged files nor the folder behind.
    #[test]
    fn a_chimp_mod_is_staged_under_its_own_name_and_cleaned_up() {
        let mut app = Baboon::for_test();
        let kit = app.model.kits[0].id;
        let ctx = egui::Context::default();
        let root = crate::core::test_kits::unique_temp_dir("chimp-staging");
        let output = root.join("Mod_P.utoc");
        let staging = staging_utoc_for(&output);
        assert_eq!(staging.file_name(), output.file_name());
        assert_eq!(staging.parent().and_then(Path::parent), Some(root.as_path()));

        let folder = staging.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&folder).unwrap();
        for file in triplet(&staging) {
            std::fs::write(file, b"partial").unwrap();
        }
        app.chimp.chimp_writes.insert(kit, None);
        app.handle_chimp_mod_built(
            kit,
            output,
            staging,
            Vec::new(),
            Err("stopped".to_owned()),
            &ctx,
        );
        assert!(!folder.exists(), "the staging folder is removed with its files");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The overwrite's leases are parked for the worker. A failed write has
    /// to give them back, or every later write to those containers is refused.
    #[test]
    fn a_failed_source_overwrite_releases_its_leases() {
        let mut app = Baboon::for_test();
        let kit = app.model.kits[0].id;
        let utoc = std::env::temp_dir().join("baboon-chimp-lease-test/pakchunk0-Windows.utoc");
        let lease = app
            .acquire_container_write_lease(&utoc, ContainerWriteMode::AppendInPlace)
            .unwrap();
        let id = app.park_container_write_lease(lease);
        app.chimp.chimp_writes.insert(kit, None);
        app.handle_chimp_sources_overwritten(
            kit,
            vec![id],
            1,
            false,
            Vec::new(),
            Err("could not overwrite".to_owned()),
            &egui::Context::default(),
        );
        assert_eq!(app.model.status, "could not overwrite");
        assert!(app.chimp.chimp_writes.is_empty());
        let again = app
            .acquire_container_write_lease(&utoc, ContainerWriteMode::AppendInPlace)
            .expect("the container is writable again");
        app.release_in_place_lease(again, ContainerWriteOutcome::Unchanged);
    }

    fn count(document: &ChimpDocument) -> i64 {
        match first_value(document, "Count") {
            PropValue::Int(value) => *value,
            other => panic!("Count is {other:?}"),
        }
    }

    /// The synthetic install with `Thing` open and its `Count` edited to 42,
    /// checkpointed, and a fresh folder outside the install to save into.
    fn edited() -> (SyntheticInstall, Baboon, PathBuf) {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING]);
        let document = app.model.kits[0].chimp.documents.get_mut(THING).unwrap();
        set_first_value(document, "Count", PropValue::Int(42));
        document.dirty = true;
        document.edits = 1;
        document.checkpoint_due = Some(0.0);
        app.flush_all_chimp_checkpoints();
        let staging =
            std::env::temp_dir().join(format!("baboon-chimp-mod-{}", uuid::Uuid::new_v4()));
        (install, app, staging)
    }

    /// Draw the open dialogs and apply what they sent, as a frame does.
    fn draw_save(app: &mut Baboon) -> impl FnMut(&mut egui::Ui) + '_ {
        move |ui| {
            let ctx = ui.ctx().clone();
            app.dialogs.draw(&cx!(app, &ctx), &app_reads!(app));
            app.apply_commands(&ctx);
        }
    }

    /// Draw the open dialogs and apply what they sent, as a frame does.
    fn draw_discard(app: &mut Baboon) -> impl FnMut(&mut egui::Ui) + '_ {
        move |ui| {
            let ctx = ui.ctx().clone();
            app.dialogs.draw(&cx!(app, &ctx), &app_reads!(app));
            app.apply_commands(&ctx);
        }
    }

    /// The save dialog needs something modified and a Paks folder to default
    /// its output to.
    #[test]
    fn the_save_dialog_opens_only_with_modified_packages_and_a_paks_folder() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING]);
        app.open_chimp_save_dialog(0);
        assert!(!app.has_chimp_save_dialog());
        assert_eq!(app.model.status, "Chimp has no modified packages to save");

        app.model.kits[0].chimp.documents.get_mut(THING).unwrap().dirty = true;
        let source = app.model.kits[0].source.take();
        app.open_chimp_save_dialog(0);
        assert!(!app.has_chimp_save_dialog());
        assert_eq!(app.model.status, "Chimp does not have a Paks output folder");

        app.model.kits[0].source = source;
        app.open_chimp_save_dialog(0);
        let dialog = app.dialogs.get::<ChimpSaveDialog>().unwrap();
        assert_eq!(dialog.mode, ChimpSaveMode::ExportMod);
        assert_eq!(dialog.name, "ChimpMod");
        assert_eq!(dialog.folder, install.root, "the Paks root, with no saved preference");
        assert!(!dialog.overwrite_acknowledged);
        assert!(dialog.pending_close_action.is_none());

        let elsewhere = install.root.join("Mods");
        app.dialogs.close::<ChimpSaveDialog>();
        app.model.prefs.chimp_output_dir = Some(elsewhere.clone());
        app.open_chimp_save_dialog(0);
        assert_eq!(
            app.dialogs.get::<ChimpSaveDialog>().unwrap().folder,
            elsewhere,
            "the last folder saved to"
        );
    }

    /// Exporting a mod rebuilds the modified package, builds the container on
    /// a worker, installs it, and settles the document: clean, and its
    /// recovery copy gone. Mounted over the game, the mod is what the package
    /// now reads as.
    #[test]
    fn exporting_a_mod_installs_a_container_that_overrides_the_package() {
        let (install, mut app, staging) = edited();
        let recovery = install.recovery_dir();
        assert!(recovery.join("manifest.json").exists());
        app.open_chimp_save_dialog(0);
        app.dialogs.get_mut::<ChimpSaveDialog>().unwrap().folder = staging.clone();

        let mut frames = Frames::new();
        frames.frame(Vec::new(), &mut draw_save(&mut app));
        frames.frame(Vec::new(), &mut draw_save(&mut app));
        assert!(frames.shows("1 modified Unreal package(s) will be saved together."));
        assert!(frames.shows(THING));
        assert!(frames.shows("Saved as a mod, leaving the installed game untouched."));
        assert!(frames.shows("Output: ChimpMod_P.utoc / .ucas / .pak"));
        assert!(!frames.shows("Overwrite source PAKs"), "an expert-mode route");

        frames.click("Export mod", &mut draw_save(&mut app));
        let output = staging.join("ChimpMod_P.utoc");
        assert!(!app.has_chimp_save_dialog());
        assert_eq!(app.model.prefs.chimp_output_dir.as_deref(), Some(staging.as_path()));
        assert!(app.chimp.chimp_writes.contains_key(&app.model.kits[0].id));
        assert_eq!(app.model.status, format!("Building {}…", output.display()));
        let rebuilt = rebuild_chimp_document(&install.world, &app.model.kits[0].chimp.documents[THING])
            .unwrap()
            .0;

        assert!(apply_next_worker_message(&mut app), "the build answered");
        assert_eq!(
            app.model.status,
            format!("Built 1 modified Unreal package(s) into {}", output.display())
        );
        assert!(app.chimp.chimp_writes.is_empty());
        let document = &app.model.kits[0].chimp.documents[THING];
        assert!(!document.dirty);
        assert_eq!(count(document), 42);
        assert!(!recovery.exists(), "the saved package's recovery copy is gone");
        let mut written: Vec<String> = fs::read_dir(&staging)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        written.sort();
        assert_eq!(
            written,
            ["ChimpMod_P.pak", "ChimpMod_P.ucas", "ChimpMod_P.utoc"],
            "the triplet, and no staging copy left behind"
        );

        let source = &install.world.archives()[0];
        let chunk = source
            .chunk_id(source.chunk_index_for("Meteorite/Content/Test/Thing.uasset").unwrap())
            .unwrap();
        let built = blam_tags::iostore::IoStoreArchive::open(&output).unwrap();
        assert_eq!(
            built.read_chunk(built.find_chunk(&chunk).unwrap()).unwrap(),
            rebuilt,
            "the mod carries the rebuilt package under the game's chunk id"
        );

        // The game's container and the mod, mounted together as the game
        // would: the mod wins.
        let layered =
            std::env::temp_dir().join(format!("baboon-chimp-layered-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&layered).unwrap();
        for extension in ["utoc", "ucas"] {
            fs::copy(
                install.root.join(format!("Paks/pakchunk0-Windows.{extension}")),
                layered.join(format!("pakchunk0-Windows.{extension}")),
            )
            .unwrap();
        }
        for file in &written {
            fs::copy(staging.join(file), layered.join(file)).unwrap();
        }
        let world = World::open(&layered, synthetic_usmap()).unwrap();
        assert_eq!(world.package(THING).unwrap().providers.len(), 2);
        assert_eq!(count(&load_chimp_document(&world, THING).unwrap()), 42);
        assert_eq!(count(&install.document(THING)), 7, "the game is untouched");
        drop(world);
        let _ = fs::remove_dir_all(&layered);
        let _ = fs::remove_dir_all(&staging);
    }

    /// Cancel closes the dialog and writes nothing; an unusable name and an
    /// unacknowledged replacement each keep the export disabled.
    #[test]
    fn the_save_dialog_refuses_a_bad_name_and_an_unacknowledged_replace() {
        let (_install, mut app, staging) = edited();
        app.open_chimp_save_dialog(0);
        app.dialogs.get_mut::<ChimpSaveDialog>().unwrap().name = " ! ".to_owned();
        app.dialogs.get_mut::<ChimpSaveDialog>().unwrap().folder = staging.clone();
        let mut frames = Frames::new();
        frames.frame(Vec::new(), &mut draw_save(&mut app));
        frames.frame(Vec::new(), &mut draw_save(&mut app));
        assert!(frames.shows("Enter a file-safe mod name."));
        frames.click("Export mod", &mut draw_save(&mut app));
        assert!(app.has_chimp_save_dialog(), "disabled: nothing happens");

        fs::create_dir_all(&staging).unwrap();
        for file in triplet(&staging.join("ChimpMod_P.utoc")) {
            fs::write(file, b"old").unwrap();
        }
        app.dialogs.get_mut::<ChimpSaveDialog>().unwrap().name = "ChimpMod".to_owned();
        frames.frame(Vec::new(), &mut draw_save(&mut app));
        assert!(frames.shows(
            "This will replace: ChimpMod_P.utoc, ChimpMod_P.ucas, ChimpMod_P.pak"
        ));
        frames.click("Export mod", &mut draw_save(&mut app));
        assert!(app.has_chimp_save_dialog(), "not acknowledged: nothing happens");
        frames.click(
            "Replace the existing mod container",
            &mut draw_save(&mut app),
        );
        assert!(
            app.dialogs
                .get::<ChimpSaveDialog>()
                .unwrap()
                .overwrite_acknowledged
        );

        frames.click("Cancel", &mut draw_save(&mut app));
        assert!(!app.has_chimp_save_dialog());
        assert!(app.chimp.chimp_writes.is_empty());
        assert!(app.model.kits[0].chimp.documents[THING].dirty);
        assert_eq!(fs::read(staging.join("ChimpMod_P.utoc")).unwrap(), b"old");
        app.clear_chimp_recovery_packages(0, &[THING.to_owned()])
            .unwrap();
        let _ = fs::remove_dir_all(&staging);
    }

    /// Overwriting the game's own containers is offered only in expert mode,
    /// needs an acknowledgement, and a dialog left on it falls back to a mod
    /// when expert mode is turned off.
    #[test]
    fn overwriting_sources_is_an_acknowledged_expert_route() {
        let (install, mut app, _staging) = edited();
        app.model.prefs.expert_mode = true;
        app.open_chimp_save_dialog(0);
        let mut frames = Frames::new();
        frames.frame(Vec::new(), &mut draw_save(&mut app));
        frames.frame(Vec::new(), &mut draw_save(&mut app));
        assert!(frames.shows("Export mod (recommended)"));
        frames.click("Overwrite source PAKs", &mut draw_save(&mut app));
        assert_eq!(
            app.dialogs.get::<ChimpSaveDialog>().unwrap().mode,
            ChimpSaveMode::OverwriteSources
        );
        assert!(frames.shows("This replaces package indexes in the installed game containers."));
        let utoc = install.root.join("Paks").join("pakchunk0-Windows.utoc");
        assert!(frames.shows(&utoc.display().to_string()));
        // The radio and the (disabled) action share a label.
        frames.click_nth("Overwrite source PAKs", 1, &mut draw_save(&mut app));
        assert!(app.has_chimp_save_dialog(), "not acknowledged: nothing happens");

        frames.click(
            "I understand these source containers will be modified",
            &mut draw_save(&mut app),
        );
        assert!(
            app.dialogs
                .get::<ChimpSaveDialog>()
                .unwrap()
                .overwrite_acknowledged
        );
        app.model.prefs.expert_mode = false;
        frames.frame(Vec::new(), &mut draw_save(&mut app));
        let dialog = app.dialogs.get::<ChimpSaveDialog>().unwrap();
        assert_eq!(dialog.mode, ChimpSaveMode::ExportMod);
        assert!(!dialog.overwrite_acknowledged);
        app.clear_chimp_recovery_packages(0, &[THING.to_owned()])
            .unwrap();
    }

    /// The overwrite appends the rebuilt package to the game's own container
    /// on a worker, settles the document against what is now on disk, and
    /// remounts the workspace whose parsed TOC it made stale.
    #[test]
    fn overwriting_sources_rewrites_the_container_and_remounts() {
        let (install, mut app, _staging) = edited();
        app.model.prefs.expert_mode = true;
        app.model.prefs.enable_chimp = true;
        app.open_chimp_save_dialog(0);
        {
            let dialog = app.dialogs.get_mut::<ChimpSaveDialog>().unwrap();
            dialog.mode = ChimpSaveMode::OverwriteSources;
            dialog.overwrite_acknowledged = true;
        }
        let rebuilt = rebuild_chimp_document(&install.world, &app.model.kits[0].chimp.documents[THING])
            .unwrap()
            .0;
        let mut frames = Frames::new();
        frames.frame(Vec::new(), &mut draw_save(&mut app));
        frames.click_nth("Overwrite source PAKs", 1, &mut draw_save(&mut app));
        assert!(!app.has_chimp_save_dialog());
        assert_eq!(app.model.status, "Overwriting 1 source container(s)…");
        assert!(app.chimp.chimp_writes.contains_key(&app.model.kits[0].id));

        // Only its answer: the remount it starts is quick enough to land in
        // the same drain, and its classification would replace the status.
        assert!(apply_one_worker_message(&mut app), "the overwrite answered");
        assert_eq!(
            app.model.status,
            "Overwrote 1 modified Unreal package(s) across 1 source container(s)"
        );
        assert!(app.chimp.chimp_writes.is_empty());
        assert!(app.mods.container_write_leases.is_empty(), "the lease is released");
        let document = &app.model.kits[0].chimp.documents[THING];
        assert!(!document.dirty);
        assert_eq!(document.original, rebuilt, "the discard baseline is the disk");
        assert!(!install.recovery_dir().exists());
        assert!(
            matches!(app.model.kits[0].chimp.mount, ChimpMount::Loading),
            "the stale TOC is remounted"
        );
        apply_until(&mut app, |app| {
            !matches!(app.model.kits[0].chimp.mount, ChimpMount::Loading) && !app.views[app.model.kits[0].id].chimp.type_indexing
        });
        if let ChimpMount::Failed(error) = &app.model.kits[0].chimp.mount {
            panic!("remount failed: {error}");
        }
        assert!(matches!(app.model.kits[0].chimp.mount, ChimpMount::Ready(_)), "{}", app.model.status);
        assert!(app.model.kits[0].chimp.documents.contains_key(THING), "documents survive");

        let world = World::open(&install.root, synthetic_usmap()).unwrap();
        assert_eq!(world.read_package(THING).unwrap(), rebuilt);
        assert_eq!(count(&load_chimp_document(&world, THING).unwrap()), 42);
    }

    /// Closing a workspace with a modified package asks first. "Save Chimp
    /// Changes…" opens the save dialog for the close, and the close runs once
    /// the mod is built.
    #[test]
    fn closing_a_workspace_with_modified_packages_saves_then_closes() {
        let (_install, mut app, staging) = edited();
        let kit = app.model.kits[0].id;
        let ctx = egui::Context::default();
        app.request_close_action(PendingCloseAction::CloseKit(kit), &ctx);
        let prompt = app
            .dialogs
            .get::<ChimpDiscardPrompt>()
            .expect("the prompt opened");
        assert_eq!(prompt.packages, [THING]);
        assert!(matches!(prompt.pending_action, Some(PendingCloseAction::CloseKit(_))));

        let mut frames = Frames::new();
        frames.frame(Vec::new(), &mut draw_discard(&mut app));
        frames.frame(Vec::new(), &mut draw_discard(&mut app));
        assert!(frames.shows(
            "The following modified Chimp packages must be saved or discarded before closing."
        ));
        assert!(frames.shows(THING));
        frames.click("Save Chimp Changes", &mut draw_discard(&mut app));
        assert!(app.dialogs.get::<ChimpDiscardPrompt>().is_none());
        let dialog = app
            .dialogs
            .get_mut::<ChimpSaveDialog>()
            .expect("the save dialog");
        assert!(matches!(
            dialog.pending_close_action,
            Some(PendingCloseAction::CloseKit(_))
        ));
        dialog.folder = staging.clone();

        frames.click("Export mod", &mut draw_save(&mut app));
        assert!(app.model.kit_index(kit).is_some(), "the close waits for the save");
        assert!(apply_next_worker_message(&mut app), "the build answered");
        assert!(app.model.kit_index(kit).is_none(), "and the workspace closed");
        assert!(staging.join("ChimpMod_P.utoc").exists());
        let _ = fs::remove_dir_all(&staging);
    }

    /// "Discard Changes" on the close prompt restores the package and lets
    /// the close through.
    #[test]
    fn closing_a_workspace_can_discard_its_modified_packages() {
        let (install, mut app, _staging) = edited();
        let kit = app.model.kits[0].id;
        let recovery = install.recovery_dir();
        app.request_close_action(PendingCloseAction::CloseKit(kit), &egui::Context::default());
        let mut frames = Frames::new();
        frames.click("Discard Changes", &mut draw_discard(&mut app));
        assert!(app.dialogs.get::<ChimpDiscardPrompt>().is_none());
        assert!(app.model.kit_index(kit).is_none(), "the workspace closed");
        assert!(!recovery.exists(), "the recovery copy went with the edit");
    }

    /// The toolbar's discard has no close behind it: it restores the listed
    /// packages and says how many. Cancel leaves them modified; a discard
    /// that fails reopens the prompt with the reason.
    #[test]
    fn the_discard_prompt_restores_cancels_and_reports_a_refusal() {
        let (_install, mut app, _staging) = edited();
        let mut frames = Frames::new();
        app.open_chimp_discard_prompt(0, vec![THING.to_owned()], None, None);
        frames.frame(Vec::new(), &mut draw_discard(&mut app));
        frames.frame(Vec::new(), &mut draw_discard(&mut app));
        assert!(frames.shows("Every listed Chimp package will return to its original source data."));
        assert!(!frames.shows("Save Chimp Changes"), "nothing to save it for");
        frames.click("Cancel", &mut draw_discard(&mut app));
        assert!(app.dialogs.get::<ChimpDiscardPrompt>().is_none());
        assert!(app.model.kits[0].chimp.documents[THING].dirty);

        let kit = app.model.kits[0].id;
        app.chimp.chimp_writes.insert(kit, None);
        app.open_chimp_discard_prompt(0, vec![THING.to_owned()], None, None);
        frames.click("Discard Changes", &mut draw_discard(&mut app));
        let prompt = app.dialogs.get::<ChimpDiscardPrompt>().expect("reopened");
        assert_eq!(
            prompt.error.as_deref(),
            Some("A Chimp save is still running; discard once it finishes")
        );
        frames.frame(Vec::new(), &mut draw_discard(&mut app));
        assert!(frames.shows("A Chimp save is still running"));

        app.chimp.chimp_writes.clear();
        frames.click("Discard Changes", &mut draw_discard(&mut app));
        assert!(app.dialogs.get::<ChimpDiscardPrompt>().is_none());
        assert_eq!(app.model.status, "Discarded 1 modified Chimp package(s)");
        let document = &app.model.kits[0].chimp.documents[THING];
        assert!(!document.dirty);
        assert_eq!(count(document), 7);
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

    /// Resetting a kit's Chimp state — a remount, or Chimp turned off — closes
    /// the save dialog it had open, which would otherwise offer to save packages
    /// that are no longer there.
    #[test]
    fn resetting_chimp_closes_that_kit_s_save_dialog() {
        let (_install, mut app, _staging) = edited();
        app.open_chimp_save_dialog(0);
        assert!(app.has_chimp_save_dialog());
        app.reset_chimp(0);
        assert!(!app.has_chimp_save_dialog());
    }

    /// Each workspace keeps its own save dialog: a second workspace's used to
    /// replace the first's, and the close the first was waiting on with it.
    #[test]
    fn each_workspace_keeps_its_own_save_dialog() {
        let mut app = Baboon::for_test();
        app.add_kit();
        app.open_chimp_save_dialog_for_test(0);
        app.open_chimp_save_dialog_for_test(1);
        let (first, second) = (app.model.kits[0].id, app.model.kits[1].id);
        assert!(app.dialogs.any::<ChimpSaveDialog>(|dialog| dialog.kit == first));
        assert!(app.dialogs.any::<ChimpSaveDialog>(|dialog| dialog.kit == second));
        app.reset_chimp(1);
        assert!(app.dialogs.any::<ChimpSaveDialog>(|dialog| dialog.kit == first), "the other is untouched");
        assert!(!app.dialogs.any::<ChimpSaveDialog>(|dialog| dialog.kit == second));
    }
}

impl Model {
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
}

impl Baboon {
    /// Whether any kit has its Chimp save dialog up.
    pub(in crate::app) fn has_chimp_save_dialog(&self) -> bool {
        self.model.kits.iter().any(|kit| {
            self.dialogs
                .any::<ChimpSaveDialog>(|dialog| dialog.kit == kit.id)
        })
    }
}
