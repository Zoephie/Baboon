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
            return true;
        };
        self.active = index;
        let mut loaded = match result {
            Ok(loaded) => loaded,
            Err(error) => {
                self.pending_session_restore = None;
                self.status = error;
                return false;
            }
        };
        // Order matters: `install_loaded_source` rebuilds the kit from empty,
        // which is what replaces the old explicit clearing of document state.
        // Everything scoped to the new source must therefore be applied after
        // it, not before, or it would be wiped on the way in.
        let initial_tag = loaded.initial_tag.take();
        let game = loaded.game.clone();
        if let Some(path) = recent_path {
            self.remember_recent_folder(path);
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
        self.refresh_active_favorite_entries();
        self.kits[self.active].generation = self.kits[self.active].generation.wrapping_add(1);
        self.refreshing_entry_index = false;
        self.building_reverse_dependencies = false;
        self.building_reference_for_entry_index = false;
        self.reference_index_progress = None;
        self.next_entry_index_refresh_at = 0.0;
        let loose_folder_source = self.source().is_some_and(|source| {
            source.game.is_some() && matches!(source.source, TagSource::LooseFolder { .. })
        });
        let has_cached_entries = self.source()
            .is_some_and(|source| !source.all_entries.is_empty());
        if loose_folder_source {
            if has_cached_entries {
                self.begin_refresh_entry_index(ctx.clone());
            } else {
                self.begin_scan_all_entries(ctx.clone());
            }
        } else {
            self.schedule_next_entry_index_refresh(ctx);
        }
        self.finish_pending_session_restore(ctx.clone());
        false
    }

    /// Apply the per-kit identity that follows from the freshly installed
    /// source: where its terminal runs, whether the terminal starts open for
    /// this game, and which keyword sidecar it uses.
    fn apply_loaded_source_identity(&mut self, game: Option<&str>) {
        let terminal_open = game.is_some_and(|game| self.terminal_open_games.contains(game));
        let kit = &mut self.kits[self.active];
        kit.terminal_work_dir = match kit.source.as_ref().map(|source| &source.source) {
            Some(TagSource::LooseFolder { root, .. }) => root.parent().map(|p| p.to_owned()),
            _ => None,
        };
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
        self.entry_index_progress = None;
        match result {
            Ok(scanned) => {
                let mut build_reference_index = false;
                let kit = &mut self.kits[kit_index];
                if let Some(source) = kit.source.as_mut() {
                    let n = scanned.len();
                    source.group_tree = crate::source::build_group_tree(&scanned);
                    source.all_entries = scanned;
                    let browser_refresh_error = if let TagSource::LooseFolder { root, .. } =
                        &source.source
                    {
                        reset_lazy_folder_browser(root, &mut source.tree, &mut source.entries).err()
                    } else {
                        None
                    };
                    source.reverse_dependencies = None;
                    kit.field_index.invalidate();
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
                        let stamp = KitStamp { kit: kit.id, generation: kit.generation };
                        let path = crate::source::index_db_path();
                        thread::spawn(move || {
                            let result = crate::source::save_entry_index(&game, &root, &entries)
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
                self.schedule_next_entry_index_refresh(ctx);
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
        if let Some(progress) = self.entry_index_progress.as_mut() {
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
        self.refreshing_entry_index = false;
        let Some(kit_index) = self.resolve_stamp(stamp) else {
            return true;
        };
        self.schedule_next_entry_index_refresh(ctx);
        match result {
            Ok(refresh) if refresh.changed => {
                self.apply_entry_index_refresh(kit_index, refresh, ctx.clone())
            }
            Ok(_) => {}
            Err(error) => self.status = format!("Index refresh failed: {error}"),
        }
        false
    }
}

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
        _ => format!(
            "Loaded {} tag(s) from {}",
            source.entries.len(),
            source.label
        ),
    }
}
