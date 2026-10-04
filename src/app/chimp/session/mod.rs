//! Chimp session lifecycle: mounting, opening, recovery checkpoints and discarding.
//! It owns how packages enter and leave a kit's workspace; saving, extracting and drawing belong elsewhere.

use super::*;

/// How long edits must pause before a Chimp document's recovery checkpoint.
///
/// A checkpoint rebuilds every export of the package, serializes it and
/// writes it to disk. It used to run on every change, and a property field
/// changes on every keystroke and every frame of a drag.
pub(super) const CHIMP_CHECKPOINT_DELAY: f64 = 1.0;

#[derive(Default, Deserialize, Serialize)]
struct ChimpRecoveryManifest {
    source: String,
    packages: HashMap<String, String>,
}

/// The folder a Paks root's Chimp checkpoints live in, named by a hash of the
/// root's spelling. Spell the root differently and the checkpoints are not
/// found, so this name is part of the saved format.
fn chimp_recovery_dir_name(root: &Path) -> String {
    let digest = Sha256::digest(root.to_string_lossy().as_bytes());
    let key: String = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("chimp-recovery-{key}")
}

impl Baboon {
    pub(in crate::app) fn apply_chimp_usmap_path(
        &mut self,
        path: Option<PathBuf>,
        ctx: egui::Context,
    ) {
        if let Err(error) = load_chimp_usmap(path.as_deref()) {
            self.model.status = error;
            return;
        }
        if self
            .model.kits
            .iter()
            .any(|kit| kit.chimp.documents.values().any(|document| document.dirty))
        {
            self.model.status =
                "Build or discard modified Chimp packages before changing the USMAP.".to_owned();
            return;
        }

        self.model.prefs.chimp_usmap_path = path;
        self.chimp.chimp_usmap_path_input = self
            .model.prefs
            .chimp_usmap_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let remount: Vec<usize> = self
            .model.kits
            .iter()
            .enumerate()
            .filter_map(|(index, kit)| {
                matches!(
                    kit.source.as_ref().map(|source| &source.source),
                    Some(TagSource::IoStoreContainerSet { .. })
                )
                .then_some(index)
            })
            .collect();
        for &index in &remount {
            self.model.kits[index].chimp = ChimpState::default();
            self.begin_chimp_mount(index, ctx.clone());
        }
        self.model.status = match &self.model.prefs.chimp_usmap_path {
            Some(path) if remount.is_empty() => {
                format!("Chimp USMAP set to {}", path.display())
            }
            Some(path) => format!("Chimp USMAP set to {}; remounting Chimp", path.display()),
            None if remount.is_empty() => {
                "Chimp will use the bundled Campaign Evolved USMAP".to_owned()
            }
            None => "Using the bundled Campaign Evolved USMAP; remounting Chimp".to_owned(),
        };
    }

    pub(in crate::app) fn commit_chimp_usmap_path_input(&mut self, ctx: egui::Context) {
        let trimmed = self.chimp.chimp_usmap_path_input.trim();
        let path = (!trimmed.is_empty()).then(|| PathBuf::from(trimmed));
        self.apply_chimp_usmap_path(path, ctx);
    }

    pub(in crate::app) fn choose_chimp_usmap_path(&mut self, ctx: egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Select Chimp USMAP")
            .add_filter("Unreal mappings", &["usmap"]);
        if let Some(directory) = self
            .model.prefs
            .chimp_usmap_path
            .as_ref()
            .and_then(|path| path.parent())
            .filter(|path| path.is_dir())
        {
            dialog = dialog.set_directory(directory);
        }
        if let Some(path) = dialog.pick_file() {
            self.apply_chimp_usmap_path(Some(path), ctx);
        }
    }

    /// What the Unreal package workspace is busy with, for a message that has
    /// to tell the user why a container cannot be replaced. Deliberately
    /// specific: "the workspace is open" is not something anyone can act on,
    /// while "still indexing package types" is.
    pub(in crate::app) fn chimp_activity(&self, kit_index: usize) -> String {
        let Some(kit) = self.model.kits.get(kit_index) else {
            return "mounted".to_owned();
        };
        if matches!(kit.chimp.mount, ChimpMount::Loading) {
            return "still mounting".to_owned();
        }
        if kit.chimp.type_indexing {
            return "indexing package types".to_owned();
        }
        if !kit.chimp.loading_packages.is_empty() {
            return "loading a package".to_owned();
        }
        if self.chimp.chimp_level_job.is_some() {
            return "exporting a level".to_owned();
        }
        "mounted".to_owned()
    }

    pub(in crate::app) fn begin_chimp_mount(&mut self, kit_index: usize, ctx: egui::Context) {
        if !self.model.prefs.enable_chimp {
            return;
        }
        let Some(source) = self.model.kits.get(kit_index).and_then(|kit| kit.source.as_ref()) else {
            return;
        };
        let TagSource::IoStoreContainerSet { root, .. } = &source.source else {
            return;
        };
        let stamp = KitStamp {
            kit: self.model.kits[kit_index].id,
            generation: self.model.kits[kit_index].generation,
        };
        let root = root.clone();
        let usmap_path = self.model.prefs.chimp_usmap_path.clone();
        self.model.kits[kit_index].chimp.mount = ChimpMount::Loading;
        let tx = self.tx.clone();
        // A mount that panicked used to send nothing and stay `Loading`, which
        // refuses every container write for the session. Through
        // `spawn_worker` a panic answers whichever message is still owed: the
        // mount's, or once that is sent, the type index's.
        let mounted = Arc::new(AtomicBool::new(false));
        let mounted_sent = Arc::clone(&mounted);
        let wake = ctx.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || {
                let result = (|| {
                    let usmap = load_chimp_usmap(usmap_path.as_deref())?;
                    // Keep startup to container discovery + the lightweight package
                    // index. Generated Blueprint schema recovery is intentionally
                    // lazy/future work; doing the whole corpus here would leave the
                    // workspace saying "loading" while reading every package.
                    let world = World::open(&root, usmap).map_err(|error| error.to_string())?;
                    Ok(Arc::new(world))
                })();
                let world = match result {
                    Ok(world) => world,
                    Err(error) => {
                        return WorkerMessage::ChimpMounted {
                            stamp,
                            result: Err(error),
                        };
                    }
                };
                let _ = tx.send(WorkerMessage::ChimpMounted {
                    stamp,
                    result: Ok(world.clone()),
                });
                mounted_sent.store(true, Ordering::SeqCst);
                wake.request_repaint();
                let index = index_chimp_package_types(&world);
                WorkerMessage::ChimpTypesIndexed { stamp, index }
            },
            move |error| {
                if mounted.load(Ordering::SeqCst) {
                    WorkerMessage::ChimpTypesIndexed {
                        stamp,
                        index: ChimpTypeIndex {
                            package_types: Vec::new(),
                            type_counts: BTreeMap::new(),
                            failures: 0,
                        },
                    }
                } else {
                    WorkerMessage::ChimpMounted {
                        stamp,
                        result: Err(format!("Mounting failed: {error}")),
                    }
                }
            },
        );
    }

    pub(in crate::app) fn handle_chimp_mounted(
        &mut self,
        stamp: KitStamp,
        result: Result<Arc<World>, String>,
        ctx: egui::Context,
    ) -> bool {
        let Some(index) = self.model.resolve_stamp(stamp) else {
            return true;
        };
        self.model.kits[index].chimp.reset_filter();
        match result {
            Ok(world) => {
                let packages = world.packages().len();
                let files = world.pak_files().len();
                self.model.kits[index].chimp.mount = ChimpMount::Ready(world.clone());
                self.model.kits[index].chimp.type_indexing = true;
                self.model.kits[index].chimp.package_types.clear();
                self.model.status =
                    format!("Chimp indexed {packages} Unreal packages and {files} pak files");
                self.reconcile_chimp_providers(index, &world);
                self.restore_chimp_recovery(index, &world);
                self.finish_pending_chimp_session_restore(index, ctx);
            }
            Err(error) => {
                self.model.kits[index].chimp.mount = ChimpMount::Failed(error.clone());
                self.model.status = format!("Chimp could not open: {error}");
            }
        }
        false
    }

    /// Re-resolve every open document's provider against the world that just
    /// mounted.
    ///
    /// A `PackageProvider` addresses its container *positionally*, and a remount
    /// rebuilds that list — so after one, an open document's provider names
    /// whatever now sits at that index. It is read by `rebuild_chimp_document`
    /// and by both save paths, which is how an edit could be written into a
    /// container it never came from.
    ///
    /// This is not new to any one feature: every remount already has the
    /// problem — changing the USMAP, overwriting source paks, and the write
    /// lease's own remount all leave the providers behind. The panes keep
    /// drawing either way, because a `ChimpDocument` holds its own bytes.
    fn reconcile_chimp_providers(&mut self, kit_index: usize, world: &World) {
        let mut orphaned = Vec::new();
        for (package, document) in &mut self.model.kits[kit_index].chimp.documents {
            match world
                .package(package)
                .and_then(|record| record.active_provider())
            {
                Some(provider) => {
                    document.provider = provider.clone();
                    document.orphaned = false;
                }
                // Left addressing its old container would be worse than saying
                // so: the document still holds its bytes, and extraction still
                // works, but nothing may be written back through a provider
                // that no longer describes anything.
                None => {
                    document.orphaned = true;
                    orphaned.push(package.clone());
                }
            }
        }
        if !orphaned.is_empty() {
            orphaned.sort();
            self.model.status = format!(
                "{} open Chimp package(s) are no longer in the mounted containers: {}",
                orphaned.len(),
                orphaned.join(", ")
            );
        }
    }

    fn finish_pending_chimp_session_restore(&mut self, kit_index: usize, ctx: egui::Context) {
        let packages = std::mem::take(&mut self.model.kits[kit_index].restore.pending_restore_chimp_packages);
        if packages.is_empty() {
            self.model.kits[kit_index].restore.pending_restore_active_chimp_package = None;
            return;
        }
        let world = match &self.model.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => world.clone(),
            _ => return,
        };
        let mut queued = 0usize;
        let mut missing = 0usize;
        for package in packages {
            if world.package(&package).is_none() {
                missing += 1;
                continue;
            }
            if self.model.kits[kit_index].documents_contains_chimp(&package) {
                let kit = self.model.kits[kit_index].id;
                self.model.kits[kit_index].chimp.open_document_pane(kit, &package);
            } else {
                self.begin_chimp_open_package(kit_index, package, ctx.clone());
            }
            queued += 1;
        }
        if self.model.kits[kit_index].chimp.loading_packages.is_empty()
            && let Some(active) = self.model.kits[kit_index]
                .restore.pending_restore_active_chimp_package
                .take()
            && self.model.kits[kit_index].documents_contains_chimp(&active)
        {
            let kit = self.model.kits[kit_index].id;
            self.model.kits[kit_index].chimp.selected_package = Some(active.clone());
            self.model.kits[kit_index].chimp.open_document_pane(kit, &active);
        }
        if queued > 0 || missing > 0 {
            self.model.status = match (queued, missing) {
                (queued, 0) => format!("Reopening {queued} Chimp package(s)"),
                (0, missing) => format!("Could not find {missing} saved Chimp package(s)"),
                (queued, missing) => format!(
                    "Reopening {queued} Chimp package(s); {missing} saved package(s) are missing"
                ),
            };
        }
    }

    pub(in crate::app) fn handle_chimp_types_indexed(
        &mut self,
        stamp: KitStamp,
        type_index: ChimpTypeIndex,
    ) -> bool {
        let Some(index) = self.model.resolve_stamp(stamp) else {
            return true;
        };
        let classified = type_index
            .package_types
            .len()
            .saturating_sub(type_index.failures);
        let kinds = type_index.type_counts.len();
        let chimp = &mut self.model.kits[index].chimp;
        chimp.package_types = type_index.package_types;
        chimp.type_indexing = false;
        chimp.reset_filter();
        self.model.status =
            format!("Chimp classified {classified} packages into {kinds} Unreal file types");
        false
    }



    fn load_chimp_recovery_manifest(
        &self,
        kit_index: usize,
    ) -> Option<(PathBuf, ChimpRecoveryManifest)> {
        let directory = self.model.chimp_recovery_dir(kit_index)?;
        let text = fs::read_to_string(directory.join("manifest.json")).ok()?;
        let manifest = serde_json::from_str(&text).ok()?;
        Some((directory, manifest))
    }

    fn restore_chimp_recovery(&mut self, kit_index: usize, world: &Arc<World>) {
        let Some((directory, manifest)) = self.load_chimp_recovery_manifest(kit_index) else {
            return;
        };
        let expected_source = self.model.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.source.root_path().display().to_string())
            .unwrap_or_default();
        if manifest.source != expected_source {
            return;
        }
        let mut restored = 0usize;
        let mut failures: Vec<String> = Vec::new();
        for (package, filename) in
            chimp_recovery_still_closed(&self.model.kits[kit_index].chimp, manifest.packages)
        {
            let Some(provider) = world
                .package(&package)
                .and_then(|record| record.active_provider())
                .cloned()
            else {
                failures.push(format!("{package} is no longer mounted"));
                continue;
            };
            let bytes = match fs::read(directory.join(&filename)) {
                Ok(bytes) => bytes,
                Err(error) => {
                    failures.push(format!("{package}: {error}"));
                    continue;
                }
            };
            let mut document = match decode_chimp_document(world, provider.clone(), bytes) {
                Ok(document) => document,
                Err(error) => {
                    failures.push(format!("{package}: {error}"));
                    continue;
                }
            };
            // The recovery file contains the edited view of the package. Keep
            // the mounted source bytes as the discard baseline so a restored
            // edit can still be returned to the actual shipped package.
            let source_bytes = match world.read_provider(&provider) {
                Ok(bytes) => bytes,
                Err(error) => {
                    failures.push(format!("{package}: {error}"));
                    continue;
                }
            };
            document.original = source_bytes;
            document.dirty = true;
            self.model.kits[kit_index]
                .chimp
                .documents
                .insert(package.clone(), document);
            let kit_id = self.model.kits[kit_index].id;
            self.model.kits[kit_index]
                .chimp
                .open_document_pane(kit_id, &package);
            restored += 1;
        }
        // The recovery files are left in place either way, so an edit that
        // could not come back this time is not deleted by having been tried.
        self.model.status = match (restored, failures.first()) {
            (0, None) => return,
            (restored, None) => format!("Chimp recovered {restored} unsaved package edit(s)"),
            (restored, Some(first)) => format!(
                "Chimp recovered {restored} unsaved package edit(s); {} could not be \
                 restored ({first})",
                failures.len()
            ),
        };
    }

    /// Checkpoint every document in the kit whose edits have paused, and wake
    /// the UI in time for the next one that is still waiting.
    pub(in crate::app) fn run_due_chimp_checkpoints(
        &mut self,
        kit_index: usize,
        ctx: &egui::Context,
    ) {
        let now = ctx.input(|input| input.time);
        let mut due = Vec::new();
        let mut next: Option<f64> = None;
        for (package, document) in &self.model.kits[kit_index].chimp.documents {
            match document.checkpoint_due {
                Some(at) if at <= now => due.push(package.clone()),
                Some(at) => next = Some(next.map_or(at, |next| next.min(at))),
                None => {}
            }
        }
        for package in due {
            self.flush_chimp_checkpoint(kit_index, &package);
        }
        if let Some(at) = next {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((at - now).max(0.0)));
        }
    }

    /// Checkpoint `package` now if it has a checkpoint waiting.
    pub(super) fn flush_chimp_checkpoint(&mut self, kit_index: usize, package: &str) {
        let waiting = self.model.kits[kit_index]
            .chimp
            .documents
            .get_mut(package)
            .and_then(|document| document.checkpoint_due.take())
            .is_some();
        // Said, not swallowed: the user is relying on this copy to survive a
        // crash, and a failed one leaves them unprotected without knowing.
        if waiting && let Err(error) = self.checkpoint_chimp_document(kit_index, package) {
            self.model.status = format!("Chimp could not save a recovery copy of {package}: {error}");
        }
    }

    /// Checkpoint every waiting document in every kit, before the app or a
    /// workspace closes and the delay would lose them.
    pub(in crate::app) fn flush_all_chimp_checkpoints(&mut self) {
        for kit_index in 0..self.model.kits.len() {
            let packages: Vec<String> = self.model.kits[kit_index]
                .chimp
                .documents
                .iter()
                .filter(|(_, document)| document.checkpoint_due.is_some())
                .map(|(package, _)| package.clone())
                .collect();
            for package in packages {
                self.flush_chimp_checkpoint(kit_index, &package);
            }
        }
    }

    /// Write `package`'s recovery checkpoint. Nothing to checkpoint (no
    /// container source, no mount, no document) is not an error.
    fn checkpoint_chimp_document(&mut self, kit_index: usize, package: &str) -> Result<(), String> {
        let Some(directory) = self.model.chimp_recovery_dir(kit_index) else {
            return Ok(());
        };
        let ChimpMount::Ready(world) = &self.model.kits[kit_index].chimp.mount else {
            return Ok(());
        };
        let Some(document) = self.model.kits[kit_index].chimp.documents.get(package) else {
            return Ok(());
        };
        let (bytes, _) = rebuild_chimp_document(world, document)?;
        fs::create_dir_all(&directory)
            .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
        let digest = Sha256::digest(package.as_bytes());
        let filename = format!(
            "{}.uasset",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let path = directory.join(&filename);
        fs::write(&path, bytes)
            .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
        let source = self.model.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.source.root_path().display().to_string())
            .unwrap_or_default();
        let mut manifest = self
            .load_chimp_recovery_manifest(kit_index)
            .map(|(_, manifest)| manifest)
            .unwrap_or_default();
        manifest.source = source;
        manifest.packages.insert(package.to_owned(), filename);
        let bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("Could not encode the recovery manifest: {error}"))?;
        let path = directory.join("manifest.json");
        fs::write(&path, bytes)
            .map_err(|error| format!("Could not write {}: {error}", path.display()))
    }

    pub(super) fn clear_chimp_recovery_packages(
        &self,
        kit_index: usize,
        packages: &[String],
    ) -> Result<(), String> {
        let Some((directory, mut manifest)) = self.load_chimp_recovery_manifest(kit_index) else {
            return Ok(());
        };
        let mut removed_files = Vec::new();
        for package in packages {
            if let Some(filename) = manifest.packages.remove(package) {
                removed_files.push(filename);
            }
        }
        if manifest.packages.is_empty() {
            match fs::remove_file(directory.join("manifest.json")) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "Could not remove Chimp recovery manifest {}: {error}",
                        directory.display()
                    ));
                }
            }
        } else {
            let bytes = serde_json::to_vec_pretty(&manifest)
                .map_err(|error| format!("Could not encode Chimp recovery manifest: {error}"))?;
            fs::write(directory.join("manifest.json"), bytes).map_err(|error| {
                format!(
                    "Could not update Chimp recovery manifest {}: {error}",
                    directory.display()
                )
            })?;
        }
        // The manifest is the recovery authority. Once it no longer names the
        // packages, stale payload cleanup cannot make discarded edits return.
        for filename in removed_files {
            let _ = fs::remove_file(directory.join(filename));
        }
        if manifest.packages.is_empty() {
            let _ = fs::remove_dir(&directory);
        }
        Ok(())
    }



    pub(in crate::app) fn open_chimp_discard_prompt(
        &mut self,
        kit_index: usize,
        packages: Vec<String>,
        pending_action: Option<PendingCloseAction>,
        error: Option<String>,
    ) {
        if packages.is_empty() {
            self.model.status = "Chimp has no modified packages".to_owned();
            return;
        }
        self.chimp.chimp_discard_prompt = Some(ChimpDiscardPrompt {
            kit: self.model.kits[kit_index].id,
            packages,
            pending_action,
            error,
        });
    }

    pub(super) fn discard_chimp_packages(
        &mut self,
        kit_index: usize,
        packages: &[String],
    ) -> Result<usize, String> {
        if self.chimp.chimp_writes.contains_key(&self.model.kits[kit_index].id) {
            return Err("A Chimp save is still running; discard once it finishes".to_owned());
        }
        let ChimpMount::Ready(world) = &self.model.kits[kit_index].chimp.mount else {
            return Err(
                "Chimp is not mounted; the original package data is unavailable".to_owned(),
            );
        };
        let world = world.clone();
        let mut restored = Vec::new();
        for package in packages {
            let Some(document) = self.model.kits[kit_index].chimp.documents.get(package) else {
                continue;
            };
            if !document.dirty {
                continue;
            }
            let selected_export = document.selected_export;
            let view = document.view;
            let mut replacement = load_chimp_document(&world, package)
                .map_err(|error| format!("Could not restore {package}: {error}"))?;
            replacement.selected_export =
                selected_export.min(replacement.exports.len().saturating_sub(1));
            replacement.view = view;
            restored.push((package.clone(), replacement));
        }

        self.clear_chimp_recovery_packages(kit_index, packages)?;
        let restored_count = restored.len();
        for (package, document) in restored {
            self.model.kits[kit_index]
                .chimp
                .documents
                .insert(package, document);
        }
        Ok(restored_count)
    }

    pub(super) fn begin_chimp_open_package(
        &mut self,
        kit_index: usize,
        package: String,
        ctx: egui::Context,
    ) {
        if self.model.kits[kit_index].documents_contains_chimp(&package) {
            let kit_id = self.model.kits[kit_index].id;
            self.model.kits[kit_index]
                .chimp
                .open_document_pane(kit_id, &package);
            return;
        }
        self.model.kits[kit_index].chimp.selected_package = Some(package.clone());
        if !self.model.kits[kit_index]
            .chimp
            .loading_packages
            .insert(package.clone())
        {
            return;
        }
        let ChimpMount::Ready(world) = &self.model.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let stamp = KitStamp {
            kit: self.model.kits[kit_index].id,
            generation: self.model.kits[kit_index].generation,
        };
        let panic_package = package.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || WorkerMessage::ChimpPackageLoaded {
                stamp,
                result: load_chimp_document(&world, &package),
                package,
            },
            move |error| WorkerMessage::ChimpPackageLoaded {
                stamp,
                package: panic_package,
                result: Err(error),
            },
        );
    }

    /// Start a sweep for the packages that import `package`.
    ///
    /// On a worker because there is no reverse index in the paks: the only way
    /// to answer it is to read every mounted header, which is the same cost as
    /// the type index at mount and far too much for a draw.
    pub(super) fn begin_chimp_referrer_scan(
        &mut self,
        kit_index: usize,
        package: String,
        ctx: egui::Context,
    ) {
        let ChimpMount::Ready(world) = &self.model.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let Some(document) = self.model.kits[kit_index].chimp.documents.get_mut(&package) else {
            return;
        };
        if matches!(document.referrers, ChimpReferrerState::Scanning) {
            return;
        }
        document.referrers = ChimpReferrerState::Scanning;
        let stamp = KitStamp {
            kit: self.model.kits[kit_index].id,
            generation: self.model.kits[kit_index].generation,
        };
        // A scan that panicked used to leave the document `Scanning` for good,
        // which also refuses a second scan.
        let panic_package = package.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || WorkerMessage::ChimpReferrersScanned {
                stamp,
                scan: Ok(scan_chimp_referrers(&world, &package)),
                package,
            },
            move |error| WorkerMessage::ChimpReferrersScanned {
                stamp,
                package: panic_package,
                scan: Err(format!("Searching for referrers failed: {error}")),
            },
        );
    }

    pub(in crate::app) fn handle_chimp_referrers_scanned(
        &mut self,
        stamp: KitStamp,
        package: String,
        scan: Result<ChimpReferrerScan, String>,
    ) -> bool {
        let Some(index) = self.model.resolve_stamp(stamp) else {
            return true;
        };
        let state = match scan {
            Ok(scan) => ChimpReferrerState::Done(scan),
            Err(error) => {
                self.model.status = error;
                ChimpReferrerState::Idle
            }
        };
        if let Some(document) = self.model.kits[index].chimp.documents.get_mut(&package) {
            document.referrers = state;
        }
        false
    }

    pub(in crate::app) fn handle_chimp_package_loaded(
        &mut self,
        stamp: KitStamp,
        package: String,
        result: Result<ChimpDocument, String>,
    ) -> bool {
        let Some(index) = self.model.resolve_stamp(stamp) else {
            return true;
        };
        self.model.kits[index].chimp.loading_packages.remove(&package);
        match result {
            Ok(document) => {
                self.model.kits[index]
                    .chimp
                    .documents
                    .insert(package.clone(), document);
                let kit_id = self.model.kits[index].id;
                self.model.kits[index].chimp.open_document_pane(kit_id, &package);
            }
            Err(error) => self.model.status = error,
        }
        if self.model.kits[index].chimp.loading_packages.is_empty()
            && let Some(active) = self.model.kits[index].restore.pending_restore_active_chimp_package.take()
            && self.model.kits[index].documents_contains_chimp(&active)
        {
            let kit_id = self.model.kits[index].id;
            self.model.kits[index].chimp.selected_package = Some(active.clone());
            self.model.kits[index].chimp.open_document_pane(kit_id, &active);
        }
        false
    }

    /// Close one Chimp package pane and drop its document.
    ///
    /// Returns `false` when the package was kept because it holds unsaved
    /// edits. Chimp has no save-changes prompt of its own — a dirty package
    /// simply refuses to close — so the caller reports that, since one blocked
    /// package in a "close all" is not worth a message per package.
    pub(in crate::app) fn close_chimp_package(&mut self, kit_index: usize, package: &str) -> bool {
        if self.model.kits[kit_index]
            .chimp
            .documents
            .get(package)
            .is_some_and(|document| document.dirty)
        {
            return false;
        }
        self.model.kits[kit_index].chimp.close_document_pane(package);
        self.model.kits[kit_index].chimp.documents.remove(package);
        true
    }
}

/// The recovery manifest's packages that are not open in `chimp` already.
///
/// Recovery runs on every mount, and a kit remounts while documents are open:
/// after a save over a mounted container, a USMAP change, a write lease. The
/// open document is newer than its last checkpoint (it holds every edit since,
/// its view and its place), so replacing it with the checkpoint lost the
/// latest edits. Only a package nothing has open is restored.
fn chimp_recovery_still_closed(
    chimp: &ChimpState,
    packages: HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut packages: Vec<(String, String)> = packages
        .into_iter()
        .filter(|(package, _)| !chimp.documents.contains_key(package))
        .collect();
    packages.sort();
    packages
}

fn sorted_unique_dirty_chimp_keys<'a>(
    documents: impl IntoIterator<Item = (&'a str, bool)>,
) -> Vec<String> {
    let mut packages = documents
        .into_iter()
        .filter_map(|(key, dirty)| dirty.then(|| key.to_owned()))
        .collect::<Vec<_>>();
    packages.sort();
    packages.dedup();
    packages
}

#[cfg(test)]
mod tests;

impl Model {
    fn chimp_recovery_dir(&self, kit_index: usize) -> Option<PathBuf> {
        let root = match &self.kits.get(kit_index)?.source.as_ref()?.source {
            TagSource::IoStoreContainerSet { root, .. } => root,
            _ => return None,
        };
        Some(crate::core::storage::data_path(&chimp_recovery_dir_name(root)))
    }

    pub(in crate::app) fn chimp_dirty_packages(&self, kit_index: usize) -> Vec<String> {
        sorted_unique_dirty_chimp_keys(
            self.kits[kit_index]
                .chimp
                .documents
                .iter()
                .map(|(key, document)| (key.as_str(), document.dirty)),
        )
    }
}
