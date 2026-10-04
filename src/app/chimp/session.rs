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
            self.status = error;
            return;
        }
        if self
            .kits
            .iter()
            .any(|kit| kit.chimp.documents.values().any(|document| document.dirty))
        {
            self.status =
                "Build or discard modified Chimp packages before changing the USMAP.".to_owned();
            return;
        }

        self.prefs.chimp_usmap_path = path;
        self.chimp_usmap_path_input = self
            .prefs
            .chimp_usmap_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let remount: Vec<usize> = self
            .kits
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
            self.kits[index].chimp = ChimpState::default();
            self.begin_chimp_mount(index, ctx.clone());
        }
        self.status = match &self.prefs.chimp_usmap_path {
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
        let trimmed = self.chimp_usmap_path_input.trim();
        let path = (!trimmed.is_empty()).then(|| PathBuf::from(trimmed));
        self.apply_chimp_usmap_path(path, ctx);
    }

    pub(in crate::app) fn choose_chimp_usmap_path(&mut self, ctx: egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Select Chimp USMAP")
            .add_filter("Unreal mappings", &["usmap"]);
        if let Some(directory) = self
            .prefs
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
        let Some(kit) = self.kits.get(kit_index) else {
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
        if self.chimp_level_job.is_some() {
            return "exporting a level".to_owned();
        }
        "mounted".to_owned()
    }

    pub(in crate::app) fn begin_chimp_mount(&mut self, kit_index: usize, ctx: egui::Context) {
        if !self.prefs.enable_chimp {
            return;
        }
        let Some(source) = self.kits.get(kit_index).and_then(|kit| kit.source.as_ref()) else {
            return;
        };
        let TagSource::IoStoreContainerSet { root, .. } = &source.source else {
            return;
        };
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
        };
        let root = root.clone();
        let usmap_path = self.prefs.chimp_usmap_path.clone();
        self.kits[kit_index].chimp.mount = ChimpMount::Loading;
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
        let Some(index) = self.resolve_stamp(stamp) else {
            return true;
        };
        self.kits[index].chimp.reset_filter();
        match result {
            Ok(world) => {
                let packages = world.packages().len();
                let files = world.pak_files().len();
                self.kits[index].chimp.mount = ChimpMount::Ready(world.clone());
                self.kits[index].chimp.type_indexing = true;
                self.kits[index].chimp.package_types.clear();
                self.status =
                    format!("Chimp indexed {packages} Unreal packages and {files} pak files");
                self.reconcile_chimp_providers(index, &world);
                self.restore_chimp_recovery(index, &world);
                self.finish_pending_chimp_session_restore(index, ctx);
            }
            Err(error) => {
                self.kits[index].chimp.mount = ChimpMount::Failed(error.clone());
                self.status = format!("Chimp could not open: {error}");
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
        for (package, document) in &mut self.kits[kit_index].chimp.documents {
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
            self.status = format!(
                "{} open Chimp package(s) are no longer in the mounted containers: {}",
                orphaned.len(),
                orphaned.join(", ")
            );
        }
    }

    fn finish_pending_chimp_session_restore(&mut self, kit_index: usize, ctx: egui::Context) {
        let packages = std::mem::take(&mut self.kits[kit_index].pending_restore_chimp_packages);
        if packages.is_empty() {
            self.kits[kit_index].pending_restore_active_chimp_package = None;
            return;
        }
        let world = match &self.kits[kit_index].chimp.mount {
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
            if self.kits[kit_index].documents_contains_chimp(&package) {
                let kit = self.kits[kit_index].id;
                self.kits[kit_index].chimp.open_document_pane(kit, &package);
            } else {
                self.begin_chimp_open_package(kit_index, package, ctx.clone());
            }
            queued += 1;
        }
        if self.kits[kit_index].chimp.loading_packages.is_empty()
            && let Some(active) = self.kits[kit_index]
                .pending_restore_active_chimp_package
                .take()
            && self.kits[kit_index].documents_contains_chimp(&active)
        {
            let kit = self.kits[kit_index].id;
            self.kits[kit_index].chimp.selected_package = Some(active.clone());
            self.kits[kit_index].chimp.open_document_pane(kit, &active);
        }
        if queued > 0 || missing > 0 {
            self.status = match (queued, missing) {
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
        let Some(index) = self.resolve_stamp(stamp) else {
            return true;
        };
        let classified = type_index
            .package_types
            .len()
            .saturating_sub(type_index.failures);
        let kinds = type_index.type_counts.len();
        let chimp = &mut self.kits[index].chimp;
        chimp.package_types = type_index.package_types;
        chimp.type_indexing = false;
        chimp.reset_filter();
        self.status =
            format!("Chimp classified {classified} packages into {kinds} Unreal file types");
        false
    }

    fn chimp_recovery_dir(&self, kit_index: usize) -> Option<PathBuf> {
        let root = match &self.kits.get(kit_index)?.source.as_ref()?.source {
            TagSource::IoStoreContainerSet { root, .. } => root,
            _ => return None,
        };
        Some(crate::storage::data_path(&chimp_recovery_dir_name(root)))
    }

    fn load_chimp_recovery_manifest(
        &self,
        kit_index: usize,
    ) -> Option<(PathBuf, ChimpRecoveryManifest)> {
        let directory = self.chimp_recovery_dir(kit_index)?;
        let text = fs::read_to_string(directory.join("manifest.json")).ok()?;
        let manifest = serde_json::from_str(&text).ok()?;
        Some((directory, manifest))
    }

    fn restore_chimp_recovery(&mut self, kit_index: usize, world: &Arc<World>) {
        let Some((directory, manifest)) = self.load_chimp_recovery_manifest(kit_index) else {
            return;
        };
        let expected_source = self.kits[kit_index]
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
            chimp_recovery_still_closed(&self.kits[kit_index].chimp, manifest.packages)
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
            self.kits[kit_index]
                .chimp
                .documents
                .insert(package.clone(), document);
            let kit_id = self.kits[kit_index].id;
            self.kits[kit_index]
                .chimp
                .open_document_pane(kit_id, &package);
            restored += 1;
        }
        // The recovery files are left in place either way, so an edit that
        // could not come back this time is not deleted by having been tried.
        self.status = match (restored, failures.first()) {
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
        for (package, document) in &self.kits[kit_index].chimp.documents {
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
        let waiting = self.kits[kit_index]
            .chimp
            .documents
            .get_mut(package)
            .and_then(|document| document.checkpoint_due.take())
            .is_some();
        // Said, not swallowed: the user is relying on this copy to survive a
        // crash, and a failed one leaves them unprotected without knowing.
        if waiting && let Err(error) = self.checkpoint_chimp_document(kit_index, package) {
            self.status = format!("Chimp could not save a recovery copy of {package}: {error}");
        }
    }

    /// Checkpoint every waiting document in every kit, before the app or a
    /// workspace closes and the delay would lose them.
    pub(in crate::app) fn flush_all_chimp_checkpoints(&mut self) {
        for kit_index in 0..self.kits.len() {
            let packages: Vec<String> = self.kits[kit_index]
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
        let Some(directory) = self.chimp_recovery_dir(kit_index) else {
            return Ok(());
        };
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return Ok(());
        };
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
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
        let source = self.kits[kit_index]
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

    pub(in crate::app) fn chimp_dirty_packages(&self, kit_index: usize) -> Vec<String> {
        sorted_unique_dirty_chimp_keys(
            self.kits[kit_index]
                .chimp
                .documents
                .iter()
                .map(|(key, document)| (key.as_str(), document.dirty)),
        )
    }

    pub(in crate::app) fn open_chimp_discard_prompt(
        &mut self,
        kit_index: usize,
        packages: Vec<String>,
        pending_action: Option<PendingCloseAction>,
        error: Option<String>,
    ) {
        if packages.is_empty() {
            self.status = "Chimp has no modified packages".to_owned();
            return;
        }
        self.chimp_discard_prompt = Some(ChimpDiscardPrompt {
            kit: self.kits[kit_index].id,
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
        if self.chimp_writes.contains_key(&self.kits[kit_index].id) {
            return Err("A Chimp save is still running; discard once it finishes".to_owned());
        }
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return Err(
                "Chimp is not mounted; the original package data is unavailable".to_owned(),
            );
        };
        let world = world.clone();
        let mut restored = Vec::new();
        for package in packages {
            let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
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
            self.kits[kit_index]
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
        if self.kits[kit_index].documents_contains_chimp(&package) {
            let kit_id = self.kits[kit_index].id;
            self.kits[kit_index]
                .chimp
                .open_document_pane(kit_id, &package);
            return;
        }
        self.kits[kit_index].chimp.selected_package = Some(package.clone());
        if !self.kits[kit_index]
            .chimp
            .loading_packages
            .insert(package.clone())
        {
            return;
        }
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
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
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let Some(document) = self.kits[kit_index].chimp.documents.get_mut(&package) else {
            return;
        };
        if matches!(document.referrers, ChimpReferrerState::Scanning) {
            return;
        }
        document.referrers = ChimpReferrerState::Scanning;
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
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
        let Some(index) = self.resolve_stamp(stamp) else {
            return true;
        };
        let state = match scan {
            Ok(scan) => ChimpReferrerState::Done(scan),
            Err(error) => {
                self.status = error;
                ChimpReferrerState::Idle
            }
        };
        if let Some(document) = self.kits[index].chimp.documents.get_mut(&package) {
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
        let Some(index) = self.resolve_stamp(stamp) else {
            return true;
        };
        self.kits[index].chimp.loading_packages.remove(&package);
        match result {
            Ok(document) => {
                self.kits[index]
                    .chimp
                    .documents
                    .insert(package.clone(), document);
                let kit_id = self.kits[index].id;
                self.kits[index].chimp.open_document_pane(kit_id, &package);
            }
            Err(error) => self.status = error,
        }
        if self.kits[index].chimp.loading_packages.is_empty()
            && let Some(active) = self.kits[index].pending_restore_active_chimp_package.take()
            && self.kits[index].documents_contains_chimp(&active)
        {
            let kit_id = self.kits[index].id;
            self.kits[index].chimp.selected_package = Some(active.clone());
            self.kits[index].chimp.open_document_pane(kit_id, &active);
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
        if self.kits[kit_index]
            .chimp
            .documents
            .get(package)
            .is_some_and(|document| document.dirty)
        {
            return false;
        }
        self.kits[kit_index].chimp.close_document_pane(package);
        self.kits[kit_index].chimp.documents.remove(package);
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
mod tests {
    use super::*;

    /// Edits schedule one recovery checkpoint for when they pause, instead of
    /// a full rebuild and write per keystroke.
    #[test]
    fn a_chimp_checkpoint_waits_for_edits_to_pause() {
        let mut app = Baboon::for_test();
        let mut document = rename_fixture();
        document.checkpoint_due = Some(5.0);
        app.kits[0]
            .chimp
            .documents
            .insert("/Game/Test/Thing".to_owned(), document);
        let ctx = egui::Context::default();
        let due_at = |time: f64, app: &mut Baboon| {
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| app.run_due_chimp_checkpoints(0, ctx),
            );
            app.kits[0].chimp.documents["/Game/Test/Thing"].checkpoint_due
        };

        assert_eq!(
            due_at(4.0, &mut app),
            Some(5.0),
            "still editing: nothing yet"
        );
        assert_eq!(due_at(6.0, &mut app), None, "paused: checkpointed once");
    }

    /// A remount re-runs recovery while documents are still open. The open
    /// document is newer than its checkpoint, so it must be left alone; only a
    /// package nothing has open is restored.
    #[test]
    fn recovery_on_remount_leaves_open_documents_alone() {
        let mut app = Baboon::for_test();
        let mut open = rename_fixture();
        open.dirty = true;
        open.edits = 7;
        app.kits[0]
            .chimp
            .documents
            .insert("/Game/Test/Thing".to_owned(), open);
        let manifest = HashMap::from([
            ("/Game/Test/Thing".to_owned(), "aaaa.uasset".to_owned()),
            ("/Game/Test/Closed".to_owned(), "bbbb.uasset".to_owned()),
        ]);

        let restoring = chimp_recovery_still_closed(&app.kits[0].chimp, manifest.clone());
        assert_eq!(
            restoring,
            [("/Game/Test/Closed".to_owned(), "bbbb.uasset".to_owned())]
        );

        // With nothing open, both come back.
        app.kits[0].chimp.documents.clear();
        assert_eq!(
            chimp_recovery_still_closed(&app.kits[0].chimp, manifest).len(),
            2
        );
    }

    #[test]
    fn chimp_discard_uses_dirty_document_keys_for_prompts() {
        assert_eq!(
            sorted_unique_dirty_chimp_keys([
                ("/Game/ZetaRequestedKey", true),
                ("/Game/AlphaRequestedKey", true),
                ("/Game/CleanRequestedKey", false),
            ]),
            ["/Game/AlphaRequestedKey", "/Game/ZetaRequestedKey"]
        );
    }

    #[test]
    fn recovery_manifest_round_trips_package_files() {
        let manifest = ChimpRecoveryManifest {
            source: "Paks".to_owned(),
            packages: HashMap::from([("/Game/UI/Probe".to_owned(), "012345.uasset".to_owned())]),
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let restored: ChimpRecoveryManifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.source, "Paks");
        assert_eq!(
            restored.packages.get("/Game/UI/Probe").map(String::as_str),
            Some("012345.uasset")
        );
    }

    /// The saved sample in `testdata/compat`: its folder is the one this
    /// build would look in for its root, and its manifest still parses.
    #[test]
    fn compat_chimp_recovery_sample() {
        let chimp = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/compat/samples/chimp");
        let directory = fs::read_dir(&chimp)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.is_dir())
            .expect("a recovery folder");
        let manifest: ChimpRecoveryManifest =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap())
                .expect("manifest");
        assert_eq!(
            directory.file_name().unwrap().to_string_lossy(),
            chimp_recovery_dir_name(Path::new(&manifest.source))
        );
        assert_ne!(
            chimp_recovery_dir_name(Path::new(&manifest.source.replace('\\', "/"))),
            chimp_recovery_dir_name(Path::new(&manifest.source)),
            "the spelling is hashed as is"
        );
        assert_eq!(manifest.packages.len(), 1);
        for filename in manifest.packages.values() {
            assert!(directory.join(filename).is_file(), "{filename}");
        }
    }

    fn stamp(app: &Baboon) -> KitStamp {
        KitStamp {
            kit: app.kits[0].id,
            generation: app.kits[0].generation,
        }
    }

    fn int(document: &ChimpDocument, property: &str) -> i64 {
        match first_value(document, property) {
            PropValue::Int(value) => *value,
            other => panic!("{property} is {other:?}"),
        }
    }

    /// Edit `Count` on `package` the way the pane does: dirty, counted, and
    /// due a checkpoint.
    fn edit_count(app: &mut Baboon, package: &str, value: i64) {
        let document = app.kits[0].chimp.documents.get_mut(package).unwrap();
        set_first_value(document, "Count", PropValue::Int(value));
        document.dirty = true;
        document.edits += 1;
        document.checkpoint_due = Some(0.0);
    }

    /// Opening a package reads it on a worker and lands it as a decoded,
    /// clean document in a focused pane. A second open of the same package
    /// focuses the pane without reading it again.
    #[test]
    fn opening_a_package_decodes_it_into_a_clean_focused_pane() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[]);
        let ctx = egui::Context::default();
        app.begin_chimp_open_package(0, THING.to_owned(), ctx.clone());
        assert!(app.kits[0].chimp.loading_packages.contains(THING));
        assert_eq!(app.kits[0].chimp.selected_package.as_deref(), Some(THING));
        assert_eq!(app.chimp_activity(0), "loading a package");

        assert!(apply_next_worker_message(&mut app));
        let chimp = &app.kits[0].chimp;
        assert!(chimp.loading_packages.is_empty());
        assert_eq!(chimp.open_packages, [THING]);
        assert_eq!(chimp.selected_package.as_deref(), Some(THING));
        let document = &chimp.documents[THING];
        assert_eq!(document.package, THING);
        assert!(!document.dirty);
        assert_eq!(document.edits, 0);
        assert_eq!(document.checkpoint_due, None);
        assert_eq!(document.view, ChimpDocumentView::Document);
        assert_eq!(document.selected_export, 0);
        assert!(document.texture_previews.is_empty());
        assert!(document.mesh_kind.is_none());
        assert_eq!(document.original, install.world.read_package(THING).unwrap());
        assert!(!document.document_text_dirty && !document.metadata_text_dirty);
        assert!(document.document_text.contains("Warthog"));
        assert!(document.metadata_text.contains("pakchunk0-Windows.utoc"));
        assert_eq!(int(document, "Count"), 7);
        assert_eq!(app.chimp_activity(0), "mounted");

        app.begin_chimp_open_package(0, THING.to_owned(), ctx);
        assert!(app.kits[0].chimp.loading_packages.is_empty());
        assert!(app.rx.try_recv().is_err(), "nothing was read again");
    }

    /// A package the mount does not provide fails to open with a status, and
    /// is not left loading.
    #[test]
    fn opening_a_missing_package_reports_it_and_stops_loading() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[]);
        app.begin_chimp_open_package(0, "/Game/Test/Missing".to_owned(), egui::Context::default());
        assert!(apply_next_worker_message(&mut app));
        assert!(app.kits[0].chimp.loading_packages.is_empty());
        assert!(app.kits[0].chimp.documents.is_empty());
        assert_eq!(app.status, "/Game/Test/Missing is not mounted");
    }

    /// A checkpoint rebuilds the edited package, writes it under a name
    /// derived from the package, and records it in the manifest against the
    /// kit's Paks root.
    #[test]
    fn a_checkpoint_writes_the_edited_package_and_names_it_in_the_manifest() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING]);
        edit_count(&mut app, THING, 42);
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                time: Some(1.0),
                ..Default::default()
            },
            |ctx| app.run_due_chimp_checkpoints(0, ctx),
        );
        assert_eq!(app.kits[0].chimp.documents[THING].checkpoint_due, None);

        let directory = app.chimp_recovery_dir(0).unwrap();
        assert_eq!(directory, install.recovery_dir());
        let manifest: ChimpRecoveryManifest =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest.source, install.root.display().to_string());
        let filename = format!(
            "{}.uasset",
            Sha256::digest(THING.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        assert_eq!(
            manifest.packages,
            HashMap::from([(THING.to_owned(), filename.clone())])
        );
        let written = fs::read(directory.join(&filename)).unwrap();
        let document = &app.kits[0].chimp.documents[THING];
        assert_eq!(
            written,
            rebuild_chimp_document(&install.world, document).unwrap().0,
            "the checkpoint is the rebuilt package"
        );
        let reread =
            decode_chimp_document(&install.world, document.provider.clone(), written).unwrap();
        assert_eq!(int(&reread, "Count"), 42);
        assert_eq!(app.status, "Ready", "a checkpoint that worked says nothing");

        app.clear_chimp_recovery_packages(0, &[THING.to_owned()])
            .unwrap();
        assert!(!directory.exists(), "the last package out removes the folder");
    }

    /// A checkpoint with nowhere to go (no Campaign Evolved source) or nothing
    /// mounted is not an error, and leaves nothing behind.
    #[test]
    fn a_checkpoint_without_a_mount_is_quietly_skipped() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING]);
        edit_count(&mut app, THING, 42);
        app.kits[0].chimp.mount = ChimpMount::Idle;
        app.flush_all_chimp_checkpoints();
        assert_eq!(app.kits[0].chimp.documents[THING].checkpoint_due, None);
        assert!(!app.chimp_recovery_dir(0).unwrap().exists());
        assert_eq!(app.status, "Ready");
    }

    /// Mounting restores a checkpointed edit that nothing has open, against
    /// the shipped bytes as its discard baseline. A later remount, with that
    /// document open and edited further, leaves it alone.
    #[test]
    fn a_mount_restores_a_closed_checkpoint_and_a_remount_keeps_the_open_one() {
        let install = SyntheticInstall::new();
        let mut first = install.app_with_open(&[THING]);
        edit_count(&mut first, THING, 42);
        first.flush_all_chimp_checkpoints();

        let mut app = Baboon::for_test();
        app.kits[0].source = Some(install.source());
        let ctx = egui::Context::default();
        app.handle_chimp_mounted(stamp(&app), Ok(install.world.clone()), ctx.clone());
        assert!(matches!(app.kits[0].chimp.mount, ChimpMount::Ready(_)));
        assert!(app.kits[0].chimp.type_indexing);
        assert_eq!(app.chimp_activity(0), "indexing package types");
        assert_eq!(app.status, "Chimp recovered 1 unsaved package edit(s)");
        let chimp = &app.kits[0].chimp;
        assert_eq!(chimp.open_packages, [THING]);
        let document = &chimp.documents[THING];
        assert!(document.dirty);
        assert_eq!(int(document, "Count"), 42);
        assert_eq!(
            document.original,
            install.world.read_package(THING).unwrap(),
            "the discard baseline is the shipped package, not the checkpoint"
        );

        edit_count(&mut app, THING, 43);
        app.kits[0]
            .chimp
            .documents
            .get_mut(THING)
            .unwrap()
            .checkpoint_due = None;
        app.handle_chimp_mounted(stamp(&app), Ok(install.world.clone()), ctx);
        assert_eq!(int(&app.kits[0].chimp.documents[THING], "Count"), 43);
        assert_eq!(
            app.status, "Chimp indexed 2 Unreal packages and 0 pak files",
            "nothing was restored over the open document"
        );
        app.clear_chimp_recovery_packages(0, &[THING.to_owned()])
            .unwrap();
    }

    /// A recovery entry for a package the mount no longer provides is
    /// reported and left on disk for a later mount.
    #[test]
    fn a_checkpoint_for_an_unmounted_package_is_reported_and_kept() {
        let install = SyntheticInstall::new();
        let mut first = install.app_with_open(&[THING]);
        edit_count(&mut first, THING, 42);
        first.flush_all_chimp_checkpoints();
        let directory = first.chimp_recovery_dir(0).unwrap();
        let mut manifest: ChimpRecoveryManifest =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        let file = manifest.packages.remove(THING).unwrap();
        manifest
            .packages
            .insert("/Game/Test/Gone".to_owned(), file.clone());
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let mut app = Baboon::for_test();
        app.kits[0].source = Some(install.source());
        app.handle_chimp_mounted(
            stamp(&app),
            Ok(install.world.clone()),
            egui::Context::default(),
        );
        assert!(app.kits[0].chimp.documents.is_empty());
        assert_eq!(
            app.status,
            "Chimp recovered 0 unsaved package edit(s); 1 could not be restored \
             (/Game/Test/Gone is no longer mounted)"
        );
        assert!(directory.join(&file).exists(), "kept for a later mount");
        app.clear_chimp_recovery_packages(0, &["/Game/Test/Gone".to_owned()])
            .unwrap();
    }

    /// A failed mount records its error and says so.
    #[test]
    fn a_failed_mount_is_recorded() {
        let install = SyntheticInstall::new();
        let mut app = Baboon::for_test();
        app.kits[0].source = Some(install.source());
        app.handle_chimp_mounted(
            stamp(&app),
            Err("no containers".to_owned()),
            egui::Context::default(),
        );
        assert!(matches!(&app.kits[0].chimp.mount, ChimpMount::Failed(error) if error == "no containers"));
        assert_eq!(app.status, "Chimp could not open: no containers");
    }

    /// A mount re-resolves open documents' providers, and marks one the
    /// mount no longer provides as orphaned rather than leaving it pointing
    /// at whatever container now sits at its old index.
    #[test]
    fn a_remount_orphans_a_document_its_containers_no_longer_provide() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING]);
        let mut stray = install.document(OTHER);
        stray.package = "/Game/Test/Gone".to_owned();
        stray.provider.container = 7;
        app.kits[0]
            .chimp
            .documents
            .insert("/Game/Test/Gone".to_owned(), stray);
        app.kits[0]
            .chimp
            .documents
            .get_mut(THING)
            .unwrap()
            .provider
            .container = 3;
        app.handle_chimp_mounted(
            stamp(&app),
            Ok(install.world.clone()),
            egui::Context::default(),
        );
        let documents = &app.kits[0].chimp.documents;
        assert!(!documents[THING].orphaned);
        assert_eq!(documents[THING].provider.container, 0);
        assert!(documents["/Game/Test/Gone"].orphaned);
        assert_eq!(
            app.status,
            "1 open Chimp package(s) are no longer in the mounted containers: /Game/Test/Gone"
        );
    }

    /// The packages a saved session had open reopen once the mount lands;
    /// missing ones are counted, and the one that was active is focused once
    /// every load has answered.
    #[test]
    fn a_mount_reopens_the_saved_session_packages() {
        let install = SyntheticInstall::new();
        let mut app = Baboon::for_test();
        app.kits[0].source = Some(install.source());
        app.kits[0].pending_restore_chimp_packages = vec![
            THING.to_owned(),
            "/Game/Test/Missing".to_owned(),
            OTHER.to_owned(),
        ];
        app.kits[0].pending_restore_active_chimp_package = Some(THING.to_owned());
        app.handle_chimp_mounted(
            stamp(&app),
            Ok(install.world.clone()),
            egui::Context::default(),
        );
        assert_eq!(
            app.status,
            "Reopening 2 Chimp package(s); 1 saved package(s) are missing"
        );
        assert_eq!(app.kits[0].chimp.loading_packages.len(), 2);
        apply_until(&mut app, |app| app.kits[0].chimp.loading_packages.is_empty());
        let chimp = &app.kits[0].chimp;
        assert!(chimp.loading_packages.is_empty());
        assert_eq!(chimp.documents.len(), 2);
        let mut open = chimp.open_packages.clone();
        open.sort();
        assert_eq!(open, [OTHER, THING]);
        assert_eq!(chimp.selected_package.as_deref(), Some(THING));
        assert!(app.kits[0].pending_restore_active_chimp_package.is_none());
        assert!(app.kits[0].pending_restore_chimp_packages.is_empty());
    }

    /// Changing the USMAP is refused while anything is modified, and
    /// otherwise remounts the workspace from scratch on a worker: the mount,
    /// then the package-type index.
    #[test]
    fn changing_the_usmap_remounts_unless_something_is_modified() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING]);
        app.prefs.enable_chimp = true;
        let ctx = egui::Context::default();
        edit_count(&mut app, THING, 42);
        app.apply_chimp_usmap_path(None, ctx.clone());
        assert_eq!(
            app.status,
            "Build or discard modified Chimp packages before changing the USMAP."
        );
        assert!(app.kits[0].chimp.documents.contains_key(THING));
        assert!(matches!(app.kits[0].chimp.mount, ChimpMount::Ready(_)));

        app.kits[0].chimp.documents.get_mut(THING).unwrap().dirty = false;
        app.apply_chimp_usmap_path(None, ctx);
        assert_eq!(
            app.status,
            "Using the bundled Campaign Evolved USMAP; remounting Chimp"
        );
        assert!(
            app.kits[0].chimp.documents.is_empty(),
            "a remount starts the workspace over"
        );
        assert!(matches!(app.kits[0].chimp.mount, ChimpMount::Loading));
        assert_eq!(app.chimp_activity(0), "still mounting");

        // The mount and the type index behind it may land in one frame.
        apply_until(&mut app, |app| {
            !matches!(app.kits[0].chimp.mount, ChimpMount::Loading) && !app.kits[0].chimp.type_indexing
        });
        if let ChimpMount::Failed(error) = &app.kits[0].chimp.mount {
            panic!("the remount failed: {error}");
        }
        let ChimpMount::Ready(world) = &app.kits[0].chimp.mount else {
            unreachable!()
        };
        assert_eq!(world.packages().len(), 2);
        let chimp = &app.kits[0].chimp;
        assert!(!chimp.type_indexing);
        // The synthetic class is a package import, which the type index does
        // not classify.
        assert_eq!(chimp.package_types, [None, None]);
        assert_eq!(
            app.status,
            "Chimp classified 0 packages into 0 Unreal file types"
        );
    }

    /// Discarding returns a modified package to its shipped bytes, keeps the
    /// view it was on, and drops its checkpoint.
    #[test]
    fn discarding_restores_the_shipped_package_and_drops_its_checkpoint() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING, OTHER]);
        edit_count(&mut app, THING, 42);
        app.kits[0]
            .chimp
            .documents
            .get_mut(THING)
            .unwrap()
            .view = ChimpDocumentView::Properties;
        app.flush_all_chimp_checkpoints();
        let directory = app.chimp_recovery_dir(0).unwrap();
        assert!(directory.join("manifest.json").exists());
        assert_eq!(app.chimp_dirty_packages(0), [THING]);

        assert_eq!(
            app.discard_chimp_packages(0, &[THING.to_owned(), OTHER.to_owned()]),
            Ok(1),
            "only the modified package is restored"
        );
        let document = &app.kits[0].chimp.documents[THING];
        assert!(!document.dirty);
        assert_eq!(document.edits, 0);
        assert_eq!(int(document, "Count"), 7);
        assert_eq!(document.view, ChimpDocumentView::Properties);
        assert!(!directory.exists());
        assert!(app.chimp_dirty_packages(0).is_empty());
        assert_eq!(app.discard_chimp_packages(0, &[THING.to_owned()]), Ok(0));
    }

    /// A discard needs the mounted source to restore from, and waits for a
    /// running save. Neither refusal touches the document.
    #[test]
    fn a_discard_is_refused_while_saving_or_unmounted() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING]);
        edit_count(&mut app, THING, 42);
        let kit = app.kits[0].id;
        app.chimp_writes.insert(kit, None);
        assert_eq!(
            app.discard_chimp_packages(0, &[THING.to_owned()]),
            Err("A Chimp save is still running; discard once it finishes".to_owned())
        );
        app.chimp_writes.clear();
        app.kits[0].chimp.mount = ChimpMount::Idle;
        assert_eq!(
            app.discard_chimp_packages(0, &[THING.to_owned()]),
            Err("Chimp is not mounted; the original package data is unavailable".to_owned())
        );
        assert_eq!(int(&app.kits[0].chimp.documents[THING], "Count"), 42);
        assert!(app.kits[0].chimp.documents[THING].dirty);
    }

    /// A modified package refuses to close; a clean one closes, taking its
    /// document and pane with it.
    #[test]
    fn a_modified_package_refuses_to_close() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING, OTHER]);
        edit_count(&mut app, THING, 42);
        assert!(!app.close_chimp_package(0, THING));
        assert!(app.kits[0].chimp.documents.contains_key(THING));
        assert!(app.close_chimp_package(0, OTHER));
        let chimp = &app.kits[0].chimp;
        assert!(!chimp.documents.contains_key(OTHER));
        assert_eq!(chimp.open_packages, [THING]);
        assert_eq!(chimp.selected_package.as_deref(), Some(THING));
    }

    /// The discard prompt needs something to discard.
    #[test]
    fn the_discard_prompt_opens_only_for_modified_packages() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING]);
        app.open_chimp_discard_prompt(0, Vec::new(), None, None);
        assert!(app.chimp_discard_prompt.is_none());
        assert_eq!(app.status, "Chimp has no modified packages");
        app.open_chimp_discard_prompt(0, vec![THING.to_owned()], None, None);
        let prompt = app.chimp_discard_prompt.as_ref().unwrap();
        assert_eq!(prompt.kit, app.kits[0].id);
        assert_eq!(prompt.packages, [THING]);
    }

    /// The referrer sweep reads every other mounted header on a worker and
    /// lists the packages whose imports name the target.
    #[test]
    fn a_referrer_scan_finds_the_packages_that_import_the_target() {
        let install = SyntheticInstall::new();
        let mut app = install.app_with_open(&[THING, OTHER]);
        app.begin_chimp_referrer_scan(0, OTHER.to_owned(), egui::Context::default());
        assert!(matches!(
            app.kits[0].chimp.documents[OTHER].referrers,
            ChimpReferrerState::Scanning
        ));
        assert!(apply_next_worker_message(&mut app));
        let ChimpReferrerState::Done(scan) = &app.kits[0].chimp.documents[OTHER].referrers else {
            panic!("the scan settled");
        };
        assert_eq!(scan.referrers, [THING]);
        assert_eq!((scan.scanned, scan.unreadable), (1, 0));

        app.begin_chimp_referrer_scan(0, THING.to_owned(), egui::Context::default());
        assert!(apply_next_worker_message(&mut app));
        let ChimpReferrerState::Done(scan) = &app.kits[0].chimp.documents[THING].referrers else {
            panic!("the scan settled");
        };
        assert!(scan.referrers.is_empty());
        assert_eq!(scan.scanned, 1);
    }
}
