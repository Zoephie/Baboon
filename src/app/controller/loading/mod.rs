//! Source-loading status helpers shared by worker-result application.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::SourceLoaded`, including source reset and follow-up index work.
    pub(super) fn handle_source_loaded(
        &mut self,
        kit: KitId,
        result: Result<LoadedSourceData, String>,
        recent_path: Option<PathBuf>,
        ctx: &egui::Context,
    ) -> bool {
        // A load targets the kit it was started for. If that kit closed while
        // the load was in flight the result is dropped rather than landing in
        // whichever kit happens to be active now.
        let Some(index) = self.resolve_kit(kit) else {
            self.settle_restored_kit(kit);
            return true;
        };
        self.active = index;
        let mut loaded = match result {
            Ok(loaded) => loaded,
            Err(error) => {
                self.release_source_load(kit);
                self.settle_restored_kit(kit);
                self.status = error;
                return false;
            }
        };
        // Check the outgoing project to disk before its source is replaced, and
        // refuse the switch if that fails rather than losing its edits.
        let outgoing = self.active;
        if self.current_source_is_campaign_project_capable(outgoing)
            && let Err(error) =
                self.checkpoint_campaign_project(outgoing, ctx.input(|input| input.time))
        {
            self.status = format!(
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
                || self.kits[self.active]
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
        self.status = loaded_source_status(&loaded);
        self.install_loaded_source(loaded);
        self.color_popup = None;
        self.function_popup = None;
        self.apply_loaded_source_identity(game.as_deref());
        if let Some((key, tag)) = initial_tag {
            let kit = &mut self.kits[self.active];
            kit.parsed_tags.insert(key.clone(), TagDocument::clean(tag));
            kit.open_tag_pane(&key);
        }
        let installed = self.active;
        self.apply_pending_campaign_project(installed, ctx.input(|input| input.time), ctx);
        self.refresh_favorite_entries_for(installed);
        self.kits[self.active].generation = self.kits[self.active].generation.wrapping_add(1);
        let campaign_evolved = self.kits[installed]
            .source
            .as_ref()
            .is_some_and(|source| matches!(&source.source, TagSource::IoStoreContainerSet { .. }));
        if campaign_evolved {
            if let Some(TagSource::IoStoreContainerSet { containers, .. }) = self.kits[installed]
                .source
                .as_ref()
                .map(|source| &source.source)
            {
                crate::app::model_preview::prewarm_ce_mesh_sync_index(containers.clone());
            }
            // Tags is the primary Campaign Evolved workspace. Chimp still
            // mounts eagerly when enabled so it is ready if the user selects
            // it, but loading a project must not switch surfaces implicitly.
            self.kits[installed].surface = campaign_evolved_surface_on_load();
            if self.prefs.enable_chimp {
                self.begin_chimp_mount(installed, ctx.clone());
            }
        }
        // A fresh source for this kit: none of its old index work applies.
        // Other kits' jobs are theirs, and are left running.
        self.kits[installed].index_jobs = IndexJobs::default();
        let loose_folder_source = self.source().is_some_and(|source| {
            source.game.is_some() && matches!(source.source, TagSource::LooseFolder { .. })
        });
        let has_cached_entries = self
            .source()
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
    fn apply_loaded_source_identity(&mut self, game: Option<&str>) {
        let terminal_open = game.is_some_and(|game| self.terminal_open_games.contains(game));
        let kit = &mut self.kits[self.active];
        kit.terminal_work_dir = kit
            .source
            .as_ref()
            .and_then(LoadedSourceData::kit_layout)
            .map(|layout| layout.root);
        kit.terminal_open = terminal_open;
        kit.keywords.load_for_game(game);
    }

    /// Applies `WorkerMessage::AllEntriesScanned`, rejecting stale source generations.
    pub(super) fn handle_all_entries_scanned(
        &mut self,
        stamp: KitStamp,
        result: Result<Vec<TagEntry>, String>,
        ctx: &egui::Context,
    ) -> bool {
        let Some(kit_index) = self.resolve_stamp(stamp) else {
            return true;
        };
        self.kits[kit_index].scanning_entries = false;
        self.kits[kit_index].index_jobs.entry_progress = None;
        match result {
            Ok(scanned) => {
                let mut build_reference_index = false;
                let n = scanned.len();
                // Moves the generation too: a folder pane only rebuilds its
                // tree (positions in the lazy list, just cleared) on a new one.
                // Done before the jobs below take their stamps.
                let browser_refresh_error = self.install_complete_entry_set(kit_index, scanned);
                let kit = &mut self.kits[kit_index];
                if let Some(source) = kit.source.as_mut() {
                    source.reverse_dependencies = None;
                    self.status = browser_refresh_error.map_or_else(
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
                        thread::spawn(move || {
                            let result = crate::core::source::save_entry_index(&game, &root, &entries)
                                .map_err(|error| error.to_string());
                            let _ = tx.send(WorkerMessage::EntryIndexSaved {
                                stamp,
                                path,
                                result,
                            });
                            ctx.request_repaint();
                        });
                    }
                }
                self.schedule_next_entry_index_refresh(kit_index, ctx);
                if build_reference_index {
                    self.begin_build_reverse_dependencies_for_entry_index(ctx.clone());
                } else {
                    self.show_entry_index_wait_notice = false;
                }
            }
            Err(e) => {
                self.show_entry_index_wait_notice = false;
                self.status = format!("Scan failed: {e}");
            }
        }
        false
    }

    pub(super) fn handle_folder_extractables_loaded(
        &mut self,
        stamp: KitStamp,
        rel_path: PathBuf,
        label: String,
        result: Result<Vec<TagEntry>, String>,
    ) -> bool {
        let Some(kit_index) = self.resolve_stamp(stamp) else {
            return true;
        };
        self.kits[kit_index].scanning_entries = false;
        self.kits[kit_index].index_jobs.entry_progress = None;
        match result {
            Ok(entries) => {
                let count = entries.len();
                self.install_folder_extractables(kit_index, &rel_path, entries);
                self.status = format!(
                    "Loaded the entire {label} folder: {count} tag(s) available for extraction"
                );
            }
            Err(error) => {
                self.status = format!("Could not load the entire {label} folder: {error}")
            }
        }
        false
    }

    /// Applies `WorkerMessage::EntryIndexScanProgress`, rejecting stale or inactive scans.
    pub(super) fn handle_entry_index_scan_progress(
        &mut self,
        stamp: KitStamp,
        processed: usize,
        total: usize,
        matched: usize,
        ctx: &egui::Context,
    ) -> bool {
        let Some(kit_index) = self.resolve_stamp(stamp) else {
            return true;
        };
        if !self.kits[kit_index].scanning_entries {
            return true;
        }
        if let Some(progress) = self.kits[kit_index].index_jobs.entry_progress.as_mut() {
            progress.processed = processed;
            progress.total = total;
            progress.matched = matched;
        }
        ctx.request_repaint();
        false
    }

    /// Applies `WorkerMessage::EntryIndexRefreshed`, rejecting stale source generations.
    pub(super) fn handle_entry_index_refreshed(
        &mut self,
        stamp: KitStamp,
        result: Result<EntryIndexRefresh, String>,
        ctx: &egui::Context,
    ) -> bool {
        let Some(kit_index) = self.resolve_kit(stamp.kit) else {
            return true;
        };
        self.kits[kit_index].index_jobs.refreshing = false;
        if self.resolve_stamp(stamp).is_none() {
            return true;
        }
        self.schedule_next_entry_index_refresh(kit_index, ctx);
        match result {
            Ok(refresh) if refresh.changed => {
                // A definition or option written by anything but a Baboon
                // save (a recompile, an import, a new file) reaches the
                // shader grid here.
                if refresh_touches_render_methods(&refresh) {
                    self.kits[kit_index].forget_render_methods();
                }
                self.apply_entry_index_refresh(kit_index, refresh, ctx.clone())
            }
            Ok(_) => {}
            Err(error) => self.status = format!("Index refresh failed: {error}"),
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

#[cfg(test)]
mod tests;

pub(super) fn loaded_source_status(source: &LoadedSourceData) -> String {
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

#[cfg(test)]
mod scan_generation_tests;
