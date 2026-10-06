//! Source-loading status helpers shared by worker-result application.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;
use crate::app::browser::actions::reset_lazy_folder_browser;

/// How often a loaded loose folder's entry index is checked against disk.
const ENTRY_INDEX_REFRESH_INTERVAL_SECS: f64 = 30.0;

impl Baboon {
    /// Applies `WorkerMessage::SourceLoaded`, including source reset and follow-up index work.
    pub(in crate::app) fn handle_source_loaded(
        &mut self,
        kit: KitId,
        result: Result<LoadedSourceData, String>,
        recent_path: Option<PathBuf>,
        ctx: &egui::Context,
    ) -> bool {
        // A load targets the kit it was started for. If that kit closed while
        // the load was in flight the result is dropped rather than landing in
        // whichever kit happens to be active now.
        let Some(index) = self.model.resolve_kit(kit) else {
            self.settle_restored_kit(kit);
            return true;
        };
        self.focus_kit(index);
        let mut loaded = match result {
            Ok(loaded) => loaded,
            Err(error) => {
                self.release_source_load(kit);
                self.settle_restored_kit(kit);
                self.model.status = error;
                return false;
            }
        };
        // Check the outgoing project to disk before its source is replaced, and
        // refuse the switch if that fails rather than losing its edits.
        let outgoing = self.model.active;
        if self.model.current_source_is_campaign_project_capable(outgoing)
            && let Err(error) =
                self.checkpoint_campaign_project(outgoing, ctx.input(|input| input.time))
        {
            self.model.status = format!(
                "Could not switch sources because the Campaign Evolved project checkpoint failed: {error}"
            );
            return false;
        }
        // Order matters: `install_loaded_source` rebuilds the kit from empty,
        // which is what replaces the old explicit clearing of document state.
        // Everything scoped to the new source must therefore be applied after
        // it, not before, or it would be wiped on the way in.
        let runtime_source_changed =
            matches!(&loaded.source, TagSource::IoStoreContainerSet { .. })
                || self.model.kits[self.model.active]
                    .source
                    .as_ref()
                    .is_some_and(|source| {
                        matches!(&source.source, TagSource::IoStoreContainerSet { .. })
                    });
        let initial_tag = loaded.initial_tag.take();
        let game = loaded.game.clone();
        if let Some(path) = recent_path {
            self.remember_recent_folder(path);
        }
        if runtime_source_changed {
            self.reset_runtime_poke_source_state();
        }
        self.model.status = loaded_source_status(&loaded);
        self.install_loaded_source(loaded);
        // Every tag of the kit's old source is gone; another kit's popups
        // are not.
        let kit = self.model.kits[self.model.active].id;
        self.close_tag_popups(kit, |_| true);
        self.apply_loaded_source_identity(game);
        if let Some((key, tag)) = initial_tag {
            let mut kit = self.kit_and_view(self.model.active);
            kit.kit.parsed_tags.insert(key.clone(), TagDocument::clean(tag));
            kit.open_tag_pane(&key);
        }
        let installed = self.model.active;
        self.apply_pending_campaign_project(installed, ctx.input(|input| input.time), ctx);
        self.refresh_favorite_entries_for(installed);
        self.model.kits[self.model.active].generation = self.model.kits[self.model.active].generation.wrapping_add(1);
        let campaign_evolved = self.model.kits[installed]
            .source
            .as_ref()
            .is_some_and(|source| matches!(&source.source, TagSource::IoStoreContainerSet { .. }));
        if campaign_evolved {
            if let Some(TagSource::IoStoreContainerSet { containers, .. }) = self.model.kits[installed]
                .source
                .as_ref()
                .map(|source| &source.source)
            {
                crate::app::model_preview::prewarm_ce_mesh_sync_index(containers.clone());
            }
            // Tags is the primary Campaign Evolved workspace. Chimp still
            // mounts eagerly when enabled so it is ready if the user selects
            // it, but loading a project must not switch surfaces implicitly.
            self.views[self.model.kits[installed].id].surface = campaign_evolved_surface_on_load();
            if self.model.prefs.enable_chimp {
                self.begin_chimp_mount(installed, ctx.clone());
            }
        }
        // A fresh source for this kit: none of its old index work applies.
        // Other kits' jobs are theirs, and are left running.
        self.model.kits[installed].index_jobs = IndexJobs::default();
        let loose_folder_source = self.model.source().is_some_and(|source| {
            source.game.is_some() && matches!(source.source, TagSource::LooseFolder { .. })
        });
        let has_cached_entries = self
            .model.source()
            .is_some_and(|source| !source.all_entries.is_empty());
        if loose_folder_source {
            if has_cached_entries {
                self.begin_refresh_entry_index(ctx.clone());
            } else {
                self.begin_scan_all_entries(ctx.clone());
            }
        } else {
            self.schedule_next_entry_index_refresh(installed, ctx);
        }
        self.finish_pending_session_restore(ctx.clone());
        // This load made its own kit active. If a session restore is still in
        // flight that is only provisional — the focus belongs to the kit the
        // session named, once every restored kit has landed.
        self.settle_restored_kit(kit);
        self.finish_pending_command_line_launch(ctx.clone());
        false
    }

    /// Apply the per-kit identity that follows from the freshly installed
    /// source: where its terminal runs, whether the terminal starts open for
    /// this game, and which keyword sidecar it uses.
    fn apply_loaded_source_identity(&mut self, game: Option<GameId>) {
        let terminal_open = game.is_some_and(|game| self.kit_tools.terminal_open_games.contains(game.as_str()));
        let kit = &mut self.model.kits[self.model.active];
        let view = &mut self.views[kit.id];
        view.terminal.work_dir = kit
            .source
            .as_ref()
            .and_then(LoadedSourceData::kit_layout)
            .map(|layout| layout.root);
        view.terminal.open = terminal_open;
        kit.keywords.load_for_game(game.map(GameId::as_str));
    }

    /// Applies `WorkerMessage::AllEntriesScanned`, rejecting stale source generations.
    pub(in crate::app) fn handle_all_entries_scanned(
        &mut self,
        stamp: KitStamp,
        result: Result<Vec<TagEntry>, String>,
        ctx: &egui::Context,
    ) -> bool {
        // A scan refuses to start while another runs, so this is the only one:
        // it is over whatever became of its source. Cleared only for a current
        // result, a refresh during the scan left every later scan refused and
        // everything waiting on one -- Find All, the libraries -- waiting.
        if let Some(kit_index) = self.model.resolve_kit(stamp.kit) {
            self.model.kits[kit_index].scanning_entries = false;
            self.model.kits[kit_index].index_jobs.entry_progress = None;
            if self.model.resolve_stamp(stamp).is_none() {
                // Whatever asked for the whole folder still needs it, now as
                // the folder stands.
                self.begin_scan_all_entries_in(kit_index, ctx.clone(), "Indexing tags...");
                return true;
            }
        }
        let Some(kit_index) = self.model.resolve_stamp(stamp) else {
            return true;
        };
        match result {
            Ok(scanned) => {
                let mut build_reference_index = false;
                let n = scanned.len();
                // Moves the generation too: a folder pane only rebuilds its
                // tree (positions in the lazy list, just cleared) on a new one.
                // Done before the jobs below take their stamps.
                let browser_refresh_error = self.install_complete_entry_set(kit_index, scanned);
                let kit = &mut self.model.kits[kit_index];
                if let Some(source) = kit.source.as_mut() {
                    source.reverse_dependencies = None;
                    self.model.status = browser_refresh_error.map_or_else(
                        || format!("Tag index complete: {n} tags; building reference index..."),
                        |error| format!("Tag index complete, but browser refresh failed: {error}"),
                    );
                    build_reference_index = true;
                    if let (Some(game), TagSource::LooseFolder { root, .. }) =
                        (source.game.clone(), &source.source)
                    {
                        let root = root.clone();
                        let entries = source.all_entries.clone();
                        let tx = self.tx.clone();
                        let ctx = ctx.clone();
                        let stamp = KitStamp {
                            kit: kit.id,
                            generation: kit.generation,
                        };
                        let path = crate::core::source::index_db_path();
                        let panic_path = path.clone();
                        spawn_worker(
                            &tx,
                            &ctx,
                            move || WorkerMessage::EntryIndexSaved {
                                stamp,
                                path,
                                result: crate::core::source::save_entry_index(game.as_str(), &root, &entries)
                                    .map_err(|error| error.to_string()),
                            },
                            move |error| WorkerMessage::EntryIndexSaved {
                                stamp,
                                path: panic_path,
                                result: Err(format!("saving the index crashed: {error}")),
                            },
                        );
                    }
                }
                self.schedule_next_entry_index_refresh(kit_index, ctx);
                if build_reference_index {
                    self.begin_build_reverse_dependencies_for_entry_index(ctx.clone());
                } else {
                    self.dialogs.close::<IndexingNotice>();
                }
            }
            Err(e) => {
                self.dialogs.close::<IndexingNotice>();
                self.model.status = format!("Scan failed: {e}");
            }
        }
        false
    }

    pub(in crate::app) fn handle_folder_extractables_loaded(
        &mut self,
        stamp: KitStamp,
        rel_path: PathBuf,
        label: String,
        result: Result<Vec<TagEntry>, String>,
    ) -> bool {
        // Over whatever became of its source, as a full scan is.
        if let Some(kit_index) = self.model.resolve_kit(stamp.kit) {
            self.model.kits[kit_index].scanning_entries = false;
            self.model.kits[kit_index].index_jobs.entry_progress = None;
        }
        let Some(kit_index) = self.model.resolve_stamp(stamp) else {
            self.model.status =
                format!("The {label} folder changed while it was loading; load it again.");
            return true;
        };
        match result {
            Ok(entries) => {
                let count = entries.len();
                self.install_folder_extractables(kit_index, &rel_path, entries);
                self.model.status = format!(
                    "Loaded the entire {label} folder: {count} tag(s) available for extraction"
                );
            }
            Err(error) => {
                self.model.status = format!("Could not load the entire {label} folder: {error}")
            }
        }
        false
    }

    /// Applies `WorkerMessage::EntryIndexScanProgress`, rejecting stale or inactive scans.
    pub(in crate::app) fn handle_entry_index_scan_progress(
        &mut self,
        stamp: KitStamp,
        processed: usize,
        total: usize,
        matched: usize,
        ctx: &egui::Context,
    ) -> bool {
        let Some(kit_index) = self.model.resolve_stamp(stamp) else {
            return true;
        };
        if !self.model.kits[kit_index].scanning_entries {
            return true;
        }
        if let Some(progress) = self.model.kits[kit_index].index_jobs.entry_progress.as_mut() {
            progress.processed = processed;
            progress.total = total;
            progress.matched = matched;
        }
        ctx.request_repaint();
        false
    }

    /// Applies `WorkerMessage::EntryIndexRefreshed`, rejecting stale source generations.
    pub(in crate::app) fn handle_entry_index_refreshed(
        &mut self,
        stamp: KitStamp,
        result: Result<EntryIndexRefresh, String>,
        ctx: &egui::Context,
    ) -> bool {
        let Some(kit_index) = self.model.resolve_kit(stamp.kit) else {
            return true;
        };
        self.model.kits[kit_index].index_jobs.refreshing = false;
        if self.model.resolve_stamp(stamp).is_none() {
            return true;
        }
        self.schedule_next_entry_index_refresh(kit_index, ctx);
        match result {
            Ok(refresh) if refresh.changed => {
                // A definition or option written by anything but a Baboon
                // save (a recompile, an import, a new file) reaches the
                // shader grid here.
                if refresh_touches_render_methods(&refresh) {
                    self.views[self.model.kits[kit_index].id].caches.forget_render_methods();
                }
                self.apply_entry_index_refresh(kit_index, refresh, ctx.clone())
            }
            Ok(_) => {}
            Err(error) => self.model.status = format!("Index refresh failed: {error}"),
        }
        false
    }
}

/// Whether a refresh added, changed or removed a render-method definition or
/// option. Removed tags are known only by key, which for a loose file ends in
/// its extension.
fn refresh_touches_render_methods(refresh: &EntryIndexRefresh) -> bool {
    refresh
        .touched
        .iter()
        .any(|entry| is_render_method_layout_group(entry.group_tag))
        || refresh.removed_keys.iter().any(|key| {
            let key = key.to_ascii_lowercase();
            key.ends_with(".render_method_definition") || key.ends_with(".render_method_option")
        })
}

fn campaign_evolved_surface_on_load() -> KitSurface {
    KitSurface::Tags
}

pub(in crate::app) fn loaded_source_status(source: &LoadedSourceData) -> String {
    match &source.source {
        TagSource::LooseFolder { .. } if source.all_entries.is_empty() => {
            format!("Browsing tags from {}", source.label)
        }
        TagSource::LooseFolder { .. } => {
            format!(
                "Found {} tag(s) in {}",
                source.all_entries.len(),
                source.label
            )
        }
        // A mounted mod overrides the game's own tags, exactly as it does in
        // game, so the browser is showing its values rather than the shipped
        // ones. Say which mods, at the one moment the user is looking: silently
        // serving a mod's tags as the base game is indistinguishable from the
        // base game having those values.
        TagSource::IoStoreContainerSet { containers, .. }
            if containers.iter().any(|container| container.is_mod) =>
        {
            let mods = containers
                .iter()
                .filter(|container| container.is_mod)
                .map(|container| container.chunk_label.as_str())
                .collect::<Vec<_>>();
            format!(
                "Loaded {} tag(s) from {} — overridden by mounted mod(s): {}",
                source.entries.len(),
                source.label,
                mods.join(", ")
            )
        }
        _ => format!(
            "Loaded {} tag(s) from {}",
            source.entries.len(),
            source.label
        ),
    }
}

impl Baboon {
    pub(in crate::app) fn begin_load_single(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new().set_title("Load Tag").pick_file() else {
            return;
        };
        self.begin_load_single_path(path, ctx);
    }

    /// Starts source work off the UI thread and reports completion through `WorkerMessage`.
    /// Captured source identity prevents stale results from replacing newer state.
    pub(in crate::app) fn begin_load_single_path(&mut self, path: PathBuf, ctx: egui::Context) {
        if self.open_kit_for(&path) {
            self.model.status = format!("Switched to {}", path.display());
            return;
        }
        let tx = self.tx.clone();
        let kit = self.model.active_kit_id();
        let names = self.model.default_names.clone();
        self.model.status = format!("Loading {}", path.display());
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_single_file(path, &names).map_err(|e| e.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: None,
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
    }

    pub(in crate::app) fn begin_load_folder(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Load Folder")
            .pick_folder()
        else {
            return;
        };
        self.begin_load_folder_path(path, ctx);
    }

    /// Starts source work off the UI thread and reports completion through `WorkerMessage`.
    /// Captured source identity prevents stale results from replacing newer state.
    pub(in crate::app) fn begin_load_folder_path(&mut self, path: PathBuf, ctx: egui::Context) {
        // A UE5 `Paks` directory (Halo: Campaign Evolved) is mounted as a
        // container set rather than walked as loose files.
        if let Some(paks) = crate::core::source::find_paks_dir(&path) {
            // Remember the folder the user picked, not the container directory
            // found inside it — the same way a loose kit remembers its root
            // rather than the `tags/` subfolder it actually scans.
            self.begin_load_iostore_container_set_path(paks, path, ctx);
            return;
        }
        // A kit's own tags folder opens that kit, with the data folder and
        // tool options it names, rather than as a bare folder whose data
        // folder would be guessed.
        if let Some(profile) = self.profile_using_chosen_tags_folder(&path) {
            self.load_custom_editing_kit_profile(profile, ctx);
            return;
        }
        if self.open_kit_for(&path) {
            self.model.status = format!("Switched to {}", path.display());
            return;
        }
        let tx = self.tx.clone();
        let kit = self.model.active_kit_id();
        let names = self.model.default_names.clone();
        let definitions_root = locate_definitions_root();
        let ek_folder_aliases = self.model.prefs.ek_folder_aliases.clone();
        let folder_info = match resolve_folder_root(&path, &ek_folder_aliases) {
            Ok(info) => info,
            Err(error) => {
                self.release_source_load(kit);
                self.model.status = error.to_string();
                return;
            }
        };
        self.model.status = match folder_info.game {
            Some(game) => format!("Indexing {} as {game}", folder_info.scan_root.display()),
            None => format!("Indexing {}", folder_info.scan_root.display()),
        };
        let recent_path = clean_recent_path(path.clone());
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_folder(path, &names, &definitions_root, &ek_folder_aliases)
                    .map_err(|e| e.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: Some(recent_path),
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
    }

    /// The profile that chose `path` as its tags folder, if any. Read from the
    /// validated layouts already cached, so asking costs no disk access.
    pub(in crate::app) fn profile_using_chosen_tags_folder(&self, path: &Path) -> Option<CustomEditingKitProfile> {
        let path = canonical_or_clean(path);
        self.model.prefs
            .custom_editing_kit_profiles
            .iter()
            .filter(|profile| profile.has_chosen_folders())
            .find(|profile| {
                self.kit_tools.editing_kit_validation
                    .custom(&profile.id)
                    .is_ok_and(|layout| same_recent_path(&layout.tags, &path))
            })
            .cloned()
    }

    pub(in crate::app) fn begin_load_monolithic(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Load Monolithic blob_index.dat")
            .add_filter("blob index", &["dat"])
            .pick_file()
        else {
            return;
        };
        self.begin_load_monolithic_path(path, ctx);
    }

    /// Starts source work off the UI thread and reports completion through `WorkerMessage`.
    /// Captured source identity prevents stale results from replacing newer state.
    pub(in crate::app) fn begin_load_monolithic_path(&mut self, path: PathBuf, ctx: egui::Context) {
        if self.open_kit_for(&path) {
            self.model.status = format!("Switched to {}", path.display());
            return;
        }
        let tx = self.tx.clone();
        let kit = self.model.active_kit_id();
        let names = self.model.default_names.clone();
        self.model.status = format!("Opening {}", path.display());
        let recent_path = clean_recent_path(path.clone());
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_monolithic_blob_index(path, &names).map_err(|e| e.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: Some(recent_path),
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
    }

    pub(in crate::app) fn begin_load_iostore_container(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open Halo: Campaign Evolved container (.utoc)")
            .add_filter("IoStore TOC", &["utoc"])
            .pick_file()
        else {
            return;
        };
        self.begin_load_iostore_container_path(path, ctx);
    }

    /// Mounts a single IoStore container (`.utoc`) off the UI thread; completion
    /// is reported through `WorkerMessage::SourceLoaded` like the other loaders.
    pub(in crate::app) fn begin_load_iostore_container_path(&mut self, path: PathBuf, ctx: egui::Context) {
        if self.open_kit_for(&path) {
            self.model.status = format!("Switched to {}", path.display());
            return;
        }
        let tx = self.tx.clone();
        let kit = self.model.active_kit_id();
        let names = self.model.default_names.clone();
        let definitions_root = locate_definitions_root();
        // Mount the container against the install's `Paks` directory. A mod
        // installed in `Paks/~mods` carries no directory index of its own, and
        // only the base containers it overrides can name its chunks.
        let pak_root = self.model.campaign_evolved_pak_root();
        self.model.status = format!("Mounting {}", path.display());
        let recent_path = clean_recent_path(path.clone());
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_iostore_container(path, pak_root, &names, &definitions_root)
                    .map_err(|e| e.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: Some(recent_path),
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
    }

    /// Mounts every container in a `Paks` directory as one merged set.
    /// Mount every container in `paks_dir`. `requested` is the folder the user
    /// actually picked — usually the game's install root, with `paks_dir`
    /// discovered inside it — and is what the kit is remembered and matched by,
    /// so reopening the install switches to it instead of adding a second kit.
    pub(in crate::app) fn begin_load_iostore_container_set_path(
        &mut self,
        paks_dir: PathBuf,
        requested: PathBuf,
        ctx: egui::Context,
    ) {
        if self.open_kit_for(&requested) {
            self.model.status = format!("Switched to {}", requested.display());
            return;
        }
        let tx = self.tx.clone();
        let kit = self.model.active_kit_id();
        let names = self.model.default_names.clone();
        let definitions_root = locate_definitions_root();
        self.model.status = format!("Mounting containers in {}", paks_dir.display());
        let recent_path = clean_recent_path(requested);
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_iostore_container_set(paks_dir, &names, &definitions_root)
                    .map_err(|e| e.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: Some(recent_path),
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
    }

    pub(in crate::app) fn load_recent_folder(&mut self, path: PathBuf, ctx: egui::Context) {
        if !path.exists() {
            self.model.status = format!("Folder not found: {}", path.display());
            self.remove_recent_folder(&path);
            return;
        }
        if path.is_dir() {
            self.begin_load_folder_path(path, ctx);
        } else {
            self.begin_load_monolithic_path(path, ctx);
        }
    }

    pub(in crate::app) fn remember_recent_folder(&mut self, path: PathBuf) {
        let path = clean_recent_path(path);
        self.model.prefs
            .recent_folders
            .retain(|existing| !same_recent_path(existing, &path));
        self.model.prefs.recent_folders.insert(0, path);
        self.model.prefs.recent_folders.truncate(MAX_RECENT_FOLDERS);
    }

    pub(in crate::app) fn remove_recent_folder(&mut self, path: &Path) {
        self.model.prefs
            .recent_folders
            .retain(|existing| !same_recent_path(existing, path));
    }

    pub(in crate::app) fn open_dropped_files(&mut self, paths: Vec<PathBuf>, ctx: egui::Context) {
        if paths.is_empty() {
            return;
        }

        let count = paths.len();
        for path in paths {
            match self.open_dropped_file(path, ctx.clone()) {
                Ok(true) => return,
                Ok(false) => {}
                Err(error) => {
                    self.model.status = error;
                    return;
                }
            }
        }

        self.model.status = if count == 1 {
            "Dropped file is not a supported tag".to_owned()
        } else {
            "No supported tag files were dropped".to_owned()
        };
    }

    pub(in crate::app) fn open_dropped_file(&mut self, path: PathBuf, ctx: egui::Context) -> Result<bool, String> {
        if !path.is_file() {
            return Ok(false);
        }

        let Some(source) = self.model.source() else {
            return Err("Load an editing-kit tags folder before dropping tag files".to_owned());
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return Err("Drop-to-open requires a loaded loose tags folder".to_owned());
        };

        let root = fs::canonicalize(root)
            .map_err(|error| format!("Could not resolve loaded tags folder: {error}"))?;
        let path = fs::canonicalize(&path)
            .map_err(|error| format!("Could not resolve dropped file: {error}"))?;
        if !path.starts_with(&root) {
            return Err(format!(
                "Dropped tag must be inside the loaded tags folder: {}",
                root.display()
            ));
        }

        if let Some(key) = self.model.key_for_loose_path(&path) {
            self.select_entry(key, ctx);
            return Ok(true);
        }

        let Some(entry) = loose_file_entry(&root, &path, &source.names)
            .map_err(|error| format!("Could not inspect dropped tag: {error:#}"))?
        else {
            return Ok(false);
        };

        let key = entry.key.clone();
        let folder_seeds = self.model.kits[self.model.active].folder_seeds();
        if let Some(source) = self.source_mut() {
            source.upsert_entry(entry, &folder_seeds);
        }
        self.model.kits[self.model.active].generation = self.model.kits[self.model.active].generation.wrapping_add(1);
        self.select_entry(key, ctx);
        Ok(true)
    }

    /// Trigger a background full recursive scan of a LooseFolder source so
    /// that Groups mode and search work without needing to expand every tree
    /// node first. No-op if already scanning or source is not a LooseFolder.
    pub(in crate::app) fn begin_scan_all_entries(&mut self, ctx: egui::Context) {
        self.begin_scan_all_entries_with_label(ctx, "Indexing tags...");
    }

    /// Starts source work off the UI thread and reports completion through `WorkerMessage`.
    /// Captured source identity prevents stale results from replacing newer state.
    pub(in crate::app) fn begin_scan_all_entries_with_label(
        &mut self,
        ctx: egui::Context,
        label: impl Into<String>,
    ) {
        self.begin_scan_all_entries_in(self.model.active, ctx, label);
    }

    /// Scan `kit_index`'s folder, which need not be the focused kit: the Model
    /// and Bitmap Libraries ask for their own kit's scan. They used to call the
    /// active-kit version, which scanned whichever kit had focus and left
    /// theirs waiting for a scan it had recorded as requested.
    pub(in crate::app) fn begin_scan_all_entries_in(
        &mut self,
        kit_index: usize,
        ctx: egui::Context,
        label: impl Into<String>,
    ) {
        let kit = &self.model.kits[kit_index];
        if kit.scanning_entries {
            return;
        }
        let Some(source) = kit.source.as_ref() else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return; // monolithic/single-file already have all entries
        };
        let root = root.clone();
        let names = source.names.clone();
        let tx = self.tx.clone();
        let kit = &mut self.model.kits[kit_index];
        kit.index_jobs.refreshing = false;
        kit.generation = kit.generation.wrapping_add(1);
        kit.field_index.invalidate();
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        let label = label.into();
        kit.scanning_entries = true;
        kit.index_jobs.entry_progress = Some(EntryIndexProgressState {
            label: label.clone(),
            processed: 0,
            total: 0,
            matched: 0,
        });
        self.dialogs.open(IndexingNotice);
        self.model.status = label;
        let progress_ctx = ctx.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || {
                let result = scan_folder_subtree_entries_with_progress(
                    &root,
                    std::path::Path::new(""),
                    &names,
                    move |progress| {
                        let _ = tx.send(WorkerMessage::EntryIndexScanProgress {
                            stamp,
                            processed: progress.processed,
                            total: progress.total,
                            matched: progress.matched,
                        });
                        progress_ctx.request_repaint();
                    },
                )
                .map_err(|e| e.to_string());
                WorkerMessage::AllEntriesScanned { stamp, result }
            },
            move |error| WorkerMessage::AllEntriesScanned {
                stamp,
                result: Err(error),
            },
        );
    }

    /// Recursively materialize one loose browser folder for the bulk extract
    /// menu. A completed global index can satisfy this immediately; otherwise
    /// the same progress-reporting scanner used by indexing runs on a worker.
    pub(in crate::app) fn begin_load_folder_extractables(
        &mut self,
        rel_path: PathBuf,
        label: String,
        ctx: egui::Context,
    ) {
        let kit_index = self.model.active;
        if self.model.kits[kit_index].scanning_entries {
            self.model.status = "A folder scan is already running".to_owned();
            return;
        }
        let Some(source) = self.model.kits[kit_index].source.as_ref() else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return;
        };

        if source.complete_scan {
            let entries = source
                .all_entries
                .iter()
                .filter(|entry| crate::core::source::entry_is_beneath_folder(entry, &rel_path))
                .cloned()
                .collect();
            self.install_folder_extractables(kit_index, &rel_path, entries);
            self.model.status = format!("Loaded the entire {label} folder for extraction");
            return;
        }

        let root = root.clone();
        let names = source.names.clone();
        let tx = self.tx.clone();
        let kit = &mut self.model.kits[kit_index];
        kit.generation = kit.generation.wrapping_add(1);
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        let progress_label = format!("Loading the entire {label} folder for extraction...");
        kit.scanning_entries = true;
        kit.index_jobs.entry_progress = Some(EntryIndexProgressState {
            label: progress_label.clone(),
            processed: 0,
            total: 0,
            matched: 0,
        });
        self.model.status = progress_label;
        let progress_ctx = ctx.clone();
        let worker_path = rel_path.clone();
        let worker_label = label.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || {
                let result = scan_folder_subtree_entries_with_progress(
                    &root,
                    &worker_path,
                    &names,
                    move |progress| {
                        let _ = tx.send(WorkerMessage::EntryIndexScanProgress {
                            stamp,
                            processed: progress.processed,
                            total: progress.total,
                            matched: progress.matched,
                        });
                        progress_ctx.request_repaint();
                    },
                )
                .map_err(|error| error.to_string());
                WorkerMessage::FolderExtractablesLoaded {
                    stamp,
                    rel_path: worker_path,
                    label: worker_label,
                    result,
                }
            },
            move |error| WorkerMessage::FolderExtractablesLoaded {
                stamp,
                rel_path,
                label,
                result: Err(error),
            },
        );
    }

    /// Merge a recursively-scanned scope into the lazy entry list and replace
    /// that scope in every browser tree whose root overlaps it.
    pub(in crate::app) fn install_folder_extractables(
        &mut self,
        kit_index: usize,
        rel_path: &Path,
        scanned: Vec<TagEntry>,
    ) {
        let kit = &mut self.model.kits[kit_index];
        let view = &mut self.views[kit.id];
        let Some(source) = kit.source.as_mut() else {
            return;
        };
        let mut known: HashSet<String> = source
            .entries
            .iter()
            .map(|entry| entry.key.clone())
            .collect();
        source.entries.extend(
            scanned
                .into_iter()
                .filter(|entry| known.insert(entry.key.clone())),
        );
        replace_loaded_tree_scope(&mut source.tree, Path::new(""), rel_path, &source.entries);
        source.group_tree = crate::core::source::build_group_tree(&source.entries);
        let new_generation = kit.generation.wrapping_add(1);
        for pane in view.browser.folder_browsers.values_mut() {
            replace_loaded_tree_scope(&mut pane.tree, &pane.rel_path, rel_path, &source.entries);
            // Keep the materialized tree installed above. Marking it stale
            // caused the next frame to replace it with a direct-only lazy tree.
            pane.cached_generation = new_generation;
            pane.group_tree_for = None;
            pane.filter_cache = FilterCache::default();
        }
        kit.generation = new_generation;
    }

    /// Starts source work off the UI thread and reports completion through `WorkerMessage`.
    /// Captured source identity prevents stale results from replacing newer state.
    pub(in crate::app) fn maybe_refresh_entry_index(&mut self, ctx: egui::Context) {
        if self.model.kits[self.model.active].scanning_entries
            || self.model.kits[self.model.active].index_jobs.refreshing
            || self.model.kits[self.model.active].index_jobs.building_references
        {
            return;
        }
        let now = ctx.input(|input| input.time);
        if now < self.model.kits[self.model.active].index_jobs.next_refresh_at {
            return;
        }
        let should_refresh = self.model.source().is_some_and(|source| {
            source.game.is_some()
                && !source.all_entries.is_empty()
                && matches!(source.source, TagSource::LooseFolder { .. })
        });
        if should_refresh {
            self.begin_refresh_entry_index(ctx);
        } else {
            self.schedule_next_entry_index_refresh(self.model.active, &ctx);
        }
    }

    pub(in crate::app) fn begin_refresh_entry_index(&mut self, ctx: egui::Context) {
        if self.model.kits[self.model.active].scanning_entries || self.model.kits[self.model.active].index_jobs.refreshing {
            return;
        }
        let Some(source) = self.model.source() else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return;
        };
        let Some(game) = source.game.clone() else {
            return;
        };
        let root = root.clone();
        let names = source.names.clone();
        let tag_source = source.source.clone();
        let stamp = self.model.kit_stamp();
        self.model.kits[self.model.active].index_jobs.refreshing = true;
        spawn_worker(
            &self.tx,
            &ctx,
            move || WorkerMessage::EntryIndexRefreshed {
                stamp,
                result: crate::core::source::refresh_entry_index(game.as_str(), &root, &names)
                    .map(|refresh| persist_entry_index_changes(game.as_str(), &root, &tag_source, refresh))
                    .map_err(|e| e.to_string()),
            },
            move |error| WorkerMessage::EntryIndexRefreshed {
                stamp,
                result: Err(error),
            },
        );
    }

    pub(in crate::app) fn schedule_next_entry_index_refresh(&mut self, kit: usize, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        self.model.kits[kit].index_jobs.next_refresh_at = now + ENTRY_INDEX_REFRESH_INTERVAL_SECS;
    }

    pub(in crate::app) fn apply_entry_index_refresh(
        &mut self,
        kit_index: usize,
        refresh: EntryIndexRefresh,
        _ctx: egui::Context,
    ) {
        let EntryIndexRefresh {
            entries,
            added,
            updated,
            removed,
            removed_keys,
            touched_dependencies,
            errors,
            ..
        } = refresh;
        let n = entries.len();
        let browser_refresh_error = self.install_complete_entry_set(kit_index, entries);
        // Patched, not dropped: the worker read the changed tags' references
        // and already wrote them (and the index rows) to disk. Dropping it, as
        // this used to, left "References to" unavailable until a manual
        // rebuild after any change the refresh noticed, including the user's
        // own saves.
        for key in &removed_keys {
            self.model.kits[kit_index].set_tag_references(key, None);
        }
        for (key, deps) in touched_dependencies {
            self.model.kits[kit_index].set_tag_references(&key, Some(deps));
        }
        self.model.status = browser_refresh_error.map_or_else(
            || {
                format!(
                    "Index updated: {n} tags ({added} added, {updated} changed, {removed} removed)"
                )
            },
            |error| format!("Index updated, but browser refresh failed: {error}"),
        );
        if let Some(first) = errors.first() {
            self.model.status = format!(
                "{}; {} tag(s) could not be indexed, first {first}",
                self.model.status,
                errors.len()
            );
        }
    }

    /// Adopt a complete entry set for a kit: the full list and its group tree,
    /// a reset lazy browser, and a new generation so panes and caches rebuild.
    /// Shared by the full scan and the periodic refresh, which used to do this
    /// separately and had drifted (only one of them moved the generation).
    /// Returns the browser reset's error, if it failed.
    pub(in crate::app) fn install_complete_entry_set(
        &mut self,
        kit_index: usize,
        entries: Vec<TagEntry>,
    ) -> Option<String> {
        let kit = &mut self.model.kits[kit_index];
        let source = kit.source.as_mut()?;
        source.group_tree = crate::core::source::build_group_tree(&entries);
        source.all_entries = entries;
        source.complete_scan = true;
        let error = if let TagSource::LooseFolder { root, .. } = &source.source {
            reset_lazy_folder_browser(root, &mut source.tree, &mut source.entries).err()
        } else {
            None
        };
        kit.field_index.invalidate();
        kit.generation = kit.generation.wrapping_add(1);
        error
    }
}

pub(in crate::app) fn extraction_scope_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_matches('/')
        .to_ascii_lowercase()
}

pub(in crate::app) fn scope_contains(parent: &str, child: &str) -> bool {
    parent.is_empty()
        || child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub(in crate::app) fn mark_tree_loaded(tree: &mut TagTree) {
    fn mark(node: &mut TagTreeNode) {
        node.children_loaded = true;
        node.entries_loaded = true;
        for child in &mut node.children {
            mark(child);
        }
    }
    for node in &mut tree.children {
        mark(node);
    }
}

pub(in crate::app) fn find_tree_node_mut<'a>(
    nodes: &'a mut [TagTreeNode],
    wanted: &str,
) -> Option<&'a mut TagTreeNode> {
    for node in nodes {
        if extraction_scope_key(&node.rel_path) == wanted {
            return Some(node);
        }
        if let Some(found) = find_tree_node_mut(&mut node.children, wanted) {
            return Some(found);
        }
    }
    None
}

/// Replace the overlap between a browser tree rooted at `tree_root` and one
/// recursively loaded extraction scope. Entry indices continue to address the
/// shared lazy `entries` vector.
pub(in crate::app) fn replace_loaded_tree_scope(
    tree: &mut TagTree,
    tree_root: &Path,
    loaded_scope: &Path,
    entries: &[TagEntry],
) {
    let root_key = extraction_scope_key(tree_root);
    let scope_key = extraction_scope_key(loaded_scope);
    if scope_contains(&scope_key, &root_key) {
        *tree = crate::core::source::build_tree_beneath(entries, tree_root);
        mark_tree_loaded(tree);
    } else if scope_contains(&root_key, &scope_key) {
        let mut replacement = crate::core::source::build_tree_beneath(entries, loaded_scope);
        mark_tree_loaded(&mut replacement);
        if let Some(node) = find_tree_node_mut(&mut tree.children, &scope_key) {
            node.children = replacement.children;
            node.entries = replacement.entries;
            node.children_loaded = true;
            node.entries_loaded = true;
        }
    }
}

pub(in crate::app) fn loose_entry_key_for_canonical_path<'a>(
    mut entries: impl Iterator<Item = &'a TagEntry>,
    canonical_path: &Path,
) -> Option<String> {
    entries.find_map(|entry| match &entry.location {
        TagEntryLocation::LooseFile(entry_path)
            if fs::canonicalize(entry_path)
                .ok()
                .is_some_and(|path| path == canonical_path) =>
        {
            Some(entry.key.clone())
        }
        _ => None,
    })
}

/// Write what a refresh found into the on-disk indexes, row by row, and read
/// the references of the tags that changed. Runs on the refresh worker.
///
/// A refresh used to hand the whole entry list back to the UI, which dropped
/// the reference index and spawned a rewrite of every index row, a stat per
/// tag although the refresh had just taken them all. Only the tags that
/// changed are touched now.
pub(in crate::app) fn persist_entry_index_changes(
    game: &str,
    root: &Path,
    tag_source: &TagSource,
    mut refresh: EntryIndexRefresh,
) -> EntryIndexRefresh {
    // One connection for the whole refresh. `None` when the folder has no
    // index yet, which writes nothing, as the per-tag calls did.
    let mut writer = if refresh.removed_keys.is_empty() && refresh.touched.is_empty() {
        None
    } else {
        match crate::core::source::EntryIndexWriter::open(game, root) {
            Ok(writer) => writer,
            Err(error) => {
                refresh.errors.push(format!("could not open the index: {error:#}"));
                None
            }
        }
    };
    for key in &refresh.removed_keys {
        if let Some(writer) = writer.as_mut()
            && let Err(error) = writer.delete_with_dependencies(key)
        {
            refresh.errors.push(format!("{key}: {error:#}"));
        }
    }
    // Each tag's row (its fingerprint) and its references go in one
    // transaction. Written apart with the errors dropped, a failure between
    // them left a current fingerprint over stale references, and no later
    // refresh would look at that tag again.
    for entry in &refresh.touched {
        let references = read_entry_dependencies(tag_source, entry);
        let written = match writer.as_mut() {
            Some(writer) => writer.upsert_with_dependencies(root, entry, references.as_deref().ok()),
            None => Ok(()),
        };
        if let Err(error) = written {
            refresh
                .errors
                .push(format!("{}: {error:#}", entry.display_path));
        }
        match references {
            Ok(references) => refresh
                .touched_dependencies
                .push((entry.key.clone(), references)),
            Err(error) => refresh.errors.push(format!(
                "{}: could not read its references: {error}",
                entry.display_path
            )),
        }
    }
    refresh
}

impl Model {
    /// The `Paks` directory of the Campaign Evolved install this session is
    /// working with — an already-mounted container set's own root, else the
    /// install configured in Settings. `None` when neither is known, which is
    /// the only case where a container has to be mounted on its own.
    pub(in crate::app) fn campaign_evolved_pak_root(&self) -> Option<PathBuf> {
        let mounted = self.kits.iter().find_map(|kit| {
            match kit.source.as_ref().map(|source| &source.source) {
                Some(TagSource::IoStoreContainerSet { root, .. }) => Some(root.clone()),
                _ => None,
            }
        });
        mounted.or_else(|| {
            self.prefs
                .custom_editing_kit_profiles
                .iter()
                .filter(|profile| profile.is_campaign_evolved())
                .find_map(|profile| crate::core::source::find_paks_dir(&profile.root))
        })
    }

    pub(in crate::app) fn key_for_loose_path(&self, path: &Path) -> Option<String> {
        let source = self.source()?;
        source
            .entries
            .iter()
            .chain(source.all_entries.iter())
            .find_map(|entry| {
                let TagEntryLocation::LooseFile(existing) = &entry.location else {
                    return None;
                };
                if existing == path || fs::canonicalize(existing).ok().as_deref() == Some(path) {
                    Some(entry.key.clone())
                } else {
                    None
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::kits::loading::replace_loaded_tree_scope;

    /// A refresh that saw a definition or option change, appear or go asks
    /// the shader grid to re-read them; one that saw only other tags does not.
    #[test]
    fn a_refresh_notices_render_method_changes() {
        let entry = |group: &[u8; 4], key: &str| TagEntry {
            key: key.to_owned(),
            display_path: key.to_owned(),
            group_tag: u32::from_be_bytes(*group),
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(key)),
        };
        let refresh = |touched: Vec<TagEntry>, removed: Vec<&str>| EntryIndexRefresh {
            entries: Vec::new(),
            changed: true,
            added: 0,
            updated: 0,
            removed: 0,
            touched,
            removed_keys: removed.into_iter().map(str::to_owned).collect(),
            touched_dependencies: Vec::new(),
            errors: Vec::new(),
        };
        assert!(!refresh_touches_render_methods(&refresh(
            vec![entry(b"hlmt", "file:a.model")],
            vec!["file:b.weapon"],
        )));
        assert!(refresh_touches_render_methods(&refresh(
            vec![entry(b"rmdf", "file:shaders/shader.render_method_definition")],
            Vec::new(),
        )));
        assert!(refresh_touches_render_methods(&refresh(
            Vec::new(),
            vec!["file:shaders/bump.render_method_option"],
        )));
    }

    #[test]
    fn campaign_evolved_projects_open_on_tags() {
        assert_eq!(campaign_evolved_surface_on_load(), KitSurface::Tags);
    }

    fn loose_kit_at(tags: &Path) -> Baboon {
        loose_kit_with(tags, "halo3_mcc", None)
    }

    fn loose_kit_with(tags: &Path, game: &str, chosen: Option<KitLayout>) -> Baboon {
        let mut app = Baboon::for_test();
        app.install_loaded_source(LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root: tags.to_path_buf(),
                game: GameId::from_id(game),
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: GameId::from_id(game),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: chosen,
        });
        app.apply_loaded_source_identity(GameId::from_id(game));
        app
    }

    /// A kit whose profile chose its folders uses exactly those: its data
    /// folder is the chosen one, not the root's `data`, and its Halo CE tools
    /// are told where both are.
    #[test]
    fn a_kit_with_chosen_folders_uses_them_everywhere() {
        let ek = PathBuf::from("/ek/HCEEK");
        let chosen = KitLayout {
            root: ek.clone(),
            tags: ek.join("tags_moda"),
            data: ek.join("data_moda"),
        };
        let app = loose_kit_with(&chosen.tags, "haloce_mcc", Some(chosen.clone()));
        assert_eq!(app.model.kit_layout_for(0), Some(chosen.clone()));
        assert_eq!(app.model.loaded_data_root(), Some(ek.join("data_moda")));
        assert_eq!(app.views[app.model.kits[0].id].terminal.work_dir, Some(ek.clone()));
        assert_eq!(
            app.model.active_kit_tool_folder_options(),
            vec![
                ("-tags_dir", ek.join("tags_moda")),
                ("-data_dir", ek.join("data_moda")),
            ]
        );
        // The same folders opened as a Halo 3 kit get no options: its tools
        // can't take them.
        let halo3 = loose_kit_with(&chosen.tags.clone(), "halo3_mcc", Some(chosen));
        assert!(halo3.model.active_kit_tool_folder_options().is_empty());
    }

    /// Opening a profile's chosen tags folder as a folder opens the profile,
    /// so its data folder and tool options come with it.
    #[test]
    fn a_chosen_tags_folder_belongs_to_its_profile() {
        let outer = crate::core::test_kits::unique_temp_dir("chosen-tags-profile");
        let root = outer.join("H2EK");
        for folder in ["tags", "data", "tags_moda", "data_moda"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        let mut app = Baboon::for_test();
        let profile = |id: &str, tags: Option<&str>| CustomEditingKitProfile {
            read_only: false,
            git_tracked: false,
            id: id.to_owned(),
            name: id.to_owned(),
            game: "halo2_mcc".to_owned(),
            root: root.clone(),
            icon: None,
            tags_folder: tags.map(PathBuf::from),
            data_folder: tags.map(|_| PathBuf::from("data_moda")),
        };
        app.model.prefs.custom_editing_kit_profiles =
            vec![profile("stock", None), profile("moda", Some("tags_moda"))];
        app.refresh_editing_kit_validation();
        let moda = app.profile_using_chosen_tags_folder(&root.join("tags_moda"));
        let stock = app.profile_using_chosen_tags_folder(&root.join("tags"));
        let _ = std::fs::remove_dir_all(&outer);
        assert_eq!(moda.map(|profile| profile.id).as_deref(), Some("moda"));
        // The stock kit opens as a folder, as it always has.
        assert_eq!(stock, None);
    }

    /// Every way the app asks where a loaded kit's root and data folder are
    /// answers from the one layout, so they cannot drift apart again.
    #[test]
    fn a_loaded_kits_folders_all_come_from_its_layout() {
        let ek = PathBuf::from("/ek/H3EK");
        let app = loose_kit_at(&ek.join("tags"));
        assert_eq!(app.model.editing_kit_root(), Some(ek.clone()));
        assert_eq!(app.model.loaded_data_root(), Some(ek.join("data")));
        assert_eq!(app.views[app.model.kits[0].id].terminal.work_dir, Some(ek.clone()));
        assert_eq!(app.model.kit_tool_path("sapien.exe"), Some(ek.join("sapien.exe")));
    }

    /// A loose folder with another name used to be its own kit root for tool
    /// launches and Open Data Folder, while the terminal and sound extraction
    /// used its parent. Now all of them use the parent.
    #[test]
    fn a_folder_not_named_tags_has_its_parent_for_a_root_everywhere() {
        let app = loose_kit_at(Path::new("/ek/H3EK/tags_moda"));
        let ek = PathBuf::from("/ek/H3EK");
        assert_eq!(app.model.editing_kit_root(), Some(ek.clone()));
        assert_eq!(app.model.loaded_data_root(), Some(ek.join("data")));
        assert_eq!(app.views[app.model.kits[0].id].terminal.work_dir, Some(ek));
    }

    /// The read-only check now starts from the tags folder, which is under the
    /// kit root, so a read-only profile still covers the kit it names.
    #[test]
    fn a_read_only_profile_still_covers_its_kit() {
        let mut app = loose_kit_at(Path::new("/ek/H3EK/tags"));
        app.model.prefs.custom_editing_kit_profiles = vec![CustomEditingKitProfile {
            read_only: true,
            git_tracked: false,
            id: "00000000-0000-4000-8000-000000000001".to_owned(),
            name: "H3EK".to_owned(),
            game: "halo3_mcc".to_owned(),
            root: PathBuf::from("/ek/H3EK"),
            icon: None,
            tags_folder: None,
            data_folder: None,
        }];
        assert!(app.model.editing_kit_is_read_only(0));
        app.model.prefs.custom_editing_kit_profiles[0].root = PathBuf::from("/ek/other");
        assert!(!app.model.editing_kit_is_read_only(0));
    }

    /// A finished scan replaces the lists folder panes index into, so it has
    /// to move the generation the panes rebuild on.
    #[test]
    fn a_finished_scan_moves_the_kit_generation() {
        let root = std::env::temp_dir().join(format!(
            "baboon-scan-generation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = Baboon::for_test();
        app.install_loaded_source(LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root: root.clone(),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: None,
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        });
        let before = app.model.kits[0].generation;
        let stamp = app.model.kit_stamp();
        // Not empty: an empty scan leaves the reference build thinking the
        // scan is unfinished, and it starts another scan, which bumps the
        // generation on its own and would hide a missing bump here.
        let scanned = vec![TagEntry {
            key: "file:objects/a.model".to_owned(),
            display_path: "objects/a.model".to_owned(),
            group_tag: u32::from_be_bytes(*b"hlmt"),
            group_name: None,
            location: TagEntryLocation::LooseFile(root.join("objects/a.model")),
        }];

        app.handle_all_entries_scanned(stamp, Ok(scanned), &egui::Context::default());
        assert!(!app.model.kits[0].scanning_entries, "no second scan was started");

        std::fs::remove_dir_all(&root).unwrap();
        assert_ne!(app.model.kits[0].generation, before);
    }

    /// Loading a source into one kit leaves another kit's index work alone.
    /// The flags were app-wide, so any kit finishing a load cleared another
    /// kit's running reference build (and its progress bar), which let a
    /// second build start over it.
    #[test]
    fn loading_one_kit_leaves_another_kits_index_build_running() {
        let mut app = Baboon::for_test();
        app.model.kits[0].index_jobs.building_references = true;
        let second = KitId(app.model.kits[0].id.0 + 1);
        app.push_kit(Kit::empty(second, TagNameIndex::default()));

        app.handle_source_loaded(
            second,
            Ok(LoadedSourceData {
                label: "second".to_owned(),
                source: TagSource::SingleFile {
                    path: PathBuf::from("second.model"),
                },
                names: TagNameIndex::default(),
                game: None,
                entries: Vec::new(),
                tree: TagTree::default(),
                group_tree: TagTree::default(),
                all_entries: Vec::new(),
                reverse_dependencies: None,
                initial_tag: None,
                key_hints: Default::default(),
                complete_scan: false,
                chosen_kit_layout: None,
            }),
            None,
            &egui::Context::default(),
        );

        assert!(app.model.kits[0].index_jobs.building_references);
    }

    /// An empty tags folder scans to nothing, and that is a finished scan.
    /// The reference build used to read the empty list as "not scanned yet"
    /// and start another scan, which landed empty and started another.
    #[test]
    fn an_empty_folder_is_scanned_once() {
        let root = std::env::temp_dir().join(format!(
            "baboon-empty-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = Baboon::for_test();
        app.install_loaded_source(LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root: root.clone(),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: None,
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        });
        let stamp = app.model.kit_stamp();

        app.handle_all_entries_scanned(stamp, Ok(Vec::new()), &egui::Context::default());

        std::fs::remove_dir_all(&root).unwrap();
        assert!(!app.model.kits[0].scanning_entries, "no second scan was started");
        let index = app.model.kits[0]
            .source
            .as_ref()
            .unwrap()
            .reverse_dependencies
            .as_ref();
        assert!(
            index.is_some(),
            "an empty folder has an empty reference graph"
        );
    }

    fn sound(path: &str) -> TagEntry {
        TagEntry {
            key: path.to_owned(),
            display_path: format!("{path}.sound"),
            group_tag: u32::from_be_bytes(*b"snd!"),
            group_name: Some("sound".to_owned()),
            location: TagEntryLocation::LooseFile(PathBuf::from(format!(
                "C:/kit/tags/{path}.sound"
            ))),
        }
    }

    #[test]
    fn loading_an_extraction_scope_materializes_its_nested_tags() {
        let entries = vec![sound("sound/a"), sound("sound/sub/b")];
        let mut tree = TagTree {
            children: vec![TagTreeNode {
                label: "sound".to_owned(),
                rel_path: PathBuf::from("sound"),
                children_loaded: true,
                entries_loaded: true,
                ..Default::default()
            }],
            entries: Vec::new(),
        };

        replace_loaded_tree_scope(&mut tree, Path::new(""), Path::new("sound"), &entries);

        let sound_node = &tree.children[0];
        assert!(sound_node.children_loaded && sound_node.entries_loaded);
        assert_eq!(
            crate::app::browser::collect_sound_keys(sound_node, &entries),
            vec!["sound/a".to_owned(), "sound/sub/b".to_owned()]
        );
    }

    // Lazily listed tags have to be keyed exactly as the full scan keys them:
    // an index refresh drops the lazy entries, and a tab whose key the full
    // index spells differently is orphaned with "no longer in the source".
    // The browser holds relative folder paths with '/', so on Windows, where
    // both separators work, a path joined as it was came out mixed.

    /// Removes the fixture's folder however the test ends.
    struct TempRoot(PathBuf);
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A loose kit holding one shader directly in `objects/characters/brute`
    /// and one beneath it in `shaders`, installed with nothing scanned yet.
    fn lazy_shader_kit(forward_slash_root: bool) -> (Baboon, TempRoot, TagFile) {
        let mut root = crate::core::test_kits::unique_temp_path("tag-key-refresh");
        if forward_slash_root {
            root = PathBuf::from(root.to_string_lossy().replace('\\', "/"));
        }
        let cleanup = TempRoot(root.clone());
        let brute = root.join("objects").join("characters").join("brute");
        let tag = TagFile::new(crate::app::test_definition_path("halo2_mcc/shader.json")).unwrap();
        for folder in [brute.clone(), brute.join("shaders")] {
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("brute.shader"), tag.write_to_bytes().unwrap()).unwrap();
        }
        let mut app = Baboon::for_test();
        app.install_loaded_source(LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root: root.clone(),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: None,
            entries: Vec::new(),
            all_entries: Vec::new(),
            tree: crate::core::source::build_folder_directory_tree(&root).unwrap(),
            group_tree: TagTree::default(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        });
        (app, cleanup, tag)
    }

    /// The key the full scan gives the tag at `rel` (with '/').
    fn full_scan_key(root: &Path, rel: &str) -> String {
        crate::core::source::scan_folder_subtree_entries(root, Path::new(""), &TagNameIndex::default())
            .unwrap()
            .into_iter()
            .find(|entry| entry.display_path.replace('\\', "/") == rel)
            .unwrap_or_else(|| panic!("the full scan has no {rel}"))
            .key
    }

    /// Open `key` with an unsaved edit, refresh the index twice, and check the
    /// tab and its edit are still there.
    fn assert_open_tag_survives_refresh(app: &mut Baboon, root: &Path, key: &str, tag: TagFile) {
        let bytes = tag.write_to_bytes().unwrap();
        let document = TagDocument::modified(tag);
        let stamp = document.content_stamp();
        app.model.kits[0].parsed_tags.insert(key.to_owned(), document);
        let ctx = egui::Context::default();
        app.select_entry(key.to_owned(), ctx.clone());
        for _ in 0..2 {
            let entries = crate::core::source::scan_folder_subtree_entries(
                root,
                Path::new(""),
                &TagNameIndex::default(),
            )
            .unwrap();
            app.apply_entry_index_refresh(
                0,
                EntryIndexRefresh {
                    entries,
                    changed: true,
                    added: 0,
                    updated: 0,
                    removed: 0,
                    touched: Vec::new(),
                    removed_keys: Vec::new(),
                    touched_dependencies: Vec::new(),
                    errors: Default::default(),
                },
                ctx.clone(),
            );
            assert!(app.model.kits[0].source.as_ref().unwrap().entries.is_empty());
            assert!(
                app.model.entry_for_key_in(0, key).is_some(),
                "the full index still resolves the open tab"
            );
            assert!(app.model.kits[0].open_tabs.iter().any(|tab| tab == key));
            let document = &app.model.kits[0].parsed_tags[key];
            assert_eq!(document.content_stamp(), stamp, "refresh must not reload unsaved edits");
            assert!(document.dirty.is_set());
            assert_eq!(document.tag.write_to_bytes().unwrap(), bytes);
        }
    }

    /// Expanding a folder: its node path is held with '/', and its children
    /// are joined with the platform's separator.
    #[cfg(windows)]
    fn assert_expanded_tag_survives_refresh(forward_slash_root: bool) {
        let (mut app, cleanup, tag) = lazy_shader_kit(forward_slash_root);
        let root = cleanup.0.clone();
        let mut node = crate::core::source::TagTreeNode {
            rel_path: PathBuf::from("objects/characters/brute").join("shaders"),
            ..Default::default()
        };
        let source = app.model.kits[0].source.as_mut().unwrap();
        let names = source.names.clone();
        crate::core::source::load_folder_node_entries(&root, &mut node, &mut source.entries, &names)
            .unwrap();
        let key = source.entries[0].key.clone();
        assert_eq!(key, full_scan_key(&root, "objects/characters/brute/shaders/brute.shader"));
        assert_eq!(
            node.rel_path,
            PathBuf::from("objects").join("characters").join("brute").join("shaders")
        );
        assert_open_tag_survives_refresh(&mut app, &root, &key, tag);
    }

    // Only Windows accepts both separators; elsewhere the path has one
    // spelling and the mismatch cannot arise.
    #[cfg(windows)]
    #[test]
    fn an_expanded_folders_tag_keeps_its_tab_through_a_refresh() {
        assert_expanded_tag_survives_refresh(false);
    }

    #[cfg(windows)]
    #[test]
    fn a_forward_slash_root_keeps_its_spelling_through_a_refresh() {
        assert_expanded_tag_survives_refresh(true);
    }

    /// A folder tab restored from the last session: its path comes back with
    /// '/', and the tags directly in it are listed by
    /// `build_lazy_folder_tree_beneath` rather than by expanding a node. Run
    /// everywhere; only on Windows can the two spellings differ.
    #[test]
    fn a_restored_folder_tabs_own_tag_keeps_its_tab_through_a_refresh() {
        let (mut app, cleanup, tag) = lazy_shader_kit(false);
        let root = cleanup.0.clone();
        let source = app.model.kits[0].source.as_mut().unwrap();
        let names = source.names.clone();
        let tree = crate::core::source::build_lazy_folder_tree_beneath(
            &root,
            Path::new("objects/characters/brute"),
            &mut source.entries,
            &names,
        )
        .unwrap();
        assert_eq!(tree.entries.len(), 1, "the folder's own tag, not the nested one");
        let key = source.entries[tree.entries[0]].key.clone();
        assert_eq!(key, full_scan_key(&root, "objects/characters/brute/brute.shader"));
        assert!(
            tree.children
                .iter()
                .all(|child| child.rel_path == child.rel_path.components().collect::<PathBuf>()),
            "child folders are respelled too"
        );
        assert_open_tag_survives_refresh(&mut app, &root, &key, tag);
    }
}
