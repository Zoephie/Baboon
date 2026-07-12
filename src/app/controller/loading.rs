//! Source-loading status helpers shared by worker-result application.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::SourceLoaded`, including source reset and follow-up index work.
    pub(super) fn handle_source_loaded(
        &mut self,
        result: Result<LoadedSourceData, String>,
        recent_path: Option<PathBuf>,
        ctx: &egui::Context,
    ) -> bool {
        let mut loaded = match result {
            Ok(loaded) => loaded,
            Err(error) => {
                self.pending_session_restore = None;
                self.status = error;
                return false;
            }
        };
        self.apply_loaded_source_identity(&loaded, recent_path);
        self.clear_source_bound_document_state();
        if let Some((key, tag)) = loaded.initial_tag.take() {
            self.selected_key = Some(key.clone());
            self.open_tabs.push(key.clone());
            self.remember_tag_use(&key);
            self.parsed_tags.insert(key, TagDocument::clean(tag));
        }
        self.status = loaded_source_status(&loaded);
        self.keywords.load_for_game(loaded.game.as_deref());
        self.field_index.invalidate();
        self.source = Some(loaded);
        self.refresh_active_favorite_entries();
        self.source_generation = self.source_generation.wrapping_add(1);
        self.refreshing_entry_index = false;
        self.building_reverse_dependencies = false;
        self.building_reference_for_entry_index = false;
        self.reference_index_progress = None;
        self.next_entry_index_refresh_at = 0.0;
        let loose_folder_source = self.source.as_ref().is_some_and(|source| {
            source.game.is_some() && matches!(source.source, TagSource::LooseFolder { .. })
        });
        let has_cached_entries = self
            .source
            .as_ref()
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

    fn apply_loaded_source_identity(
        &mut self,
        loaded: &LoadedSourceData,
        recent_path: Option<PathBuf>,
    ) {
        if let Some(path) = recent_path {
            self.remember_recent_folder(path);
        }
        self.terminal_work_dir = if let TagSource::LooseFolder { root, .. } = &loaded.source {
            root.parent().map(|p| p.to_owned())
        } else {
            None
        };
        self.terminal_open = loaded
            .game
            .as_deref()
            .map(|g| self.terminal_open_games.contains(g))
            .unwrap_or(false);
        self.names = loaded.names.clone();
        self.names.merge_missing(self.default_names.clone());
    }

    fn clear_source_bound_document_state(&mut self) {
        self.parsed_tags.clear();
        self.tag_cache_order.clear();
        self.loading_tags.clear();
        self.bitmap_previews.clear();
        self.rmdf_cache.clear();
        self.rmop_cache.clear();
        self.color_popup = None;
        self.function_popup = None;
        self.selected_key = None;
        self.open_tabs.clear();
        self.floating_tabs.clear();
    }

    /// Applies `WorkerMessage::AllEntriesScanned`, rejecting stale source generations.
    pub(super) fn handle_all_entries_scanned(
        &mut self,
        generation: u64,
        result: Result<Vec<TagEntry>, String>,
        ctx: &egui::Context,
    ) -> bool {
        if generation != self.source_generation {
            return true;
        }
        self.scanning_entries = false;
        self.entry_index_progress = None;
        match result {
            Ok(scanned) => {
                let mut build_reference_index = false;
                if let Some(source) = self.source.as_mut() {
                    let n = scanned.len();
                    source.group_tree = crate::source::build_group_tree(&scanned);
                    source.all_entries = scanned;
                    source.reverse_dependencies = None;
                    self.field_index.invalidate();
                    self.status =
                        format!("Tag index complete: {n} tags; building reference index...");
                    build_reference_index = true;
                    if let (Some(game), TagSource::LooseFolder { root, .. }) =
                        (source.game.clone(), &source.source)
                    {
                        let root = root.clone();
                        let entries = source.all_entries.clone();
                        let tx = self.tx.clone();
                        let ctx = ctx.clone();
                        let generation = self.source_generation;
                        let path = crate::source::index_db_path();
                        thread::spawn(move || {
                            let result = crate::source::save_entry_index(&game, &root, &entries)
                                .map_err(|error| error.to_string());
                            let _ = tx.send(WorkerMessage::EntryIndexSaved {
                                generation,
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
        generation: u64,
        processed: usize,
        total: usize,
        matched: usize,
        ctx: &egui::Context,
    ) -> bool {
        if generation != self.source_generation || !self.scanning_entries {
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
        generation: u64,
        result: Result<EntryIndexRefresh, String>,
        ctx: &egui::Context,
    ) -> bool {
        self.refreshing_entry_index = false;
        if generation != self.source_generation {
            return true;
        }
        self.schedule_next_entry_index_refresh(ctx);
        match result {
            Ok(refresh) if refresh.changed => self.apply_entry_index_refresh(refresh, ctx.clone()),
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
