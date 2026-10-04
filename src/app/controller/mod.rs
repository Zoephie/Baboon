//! Application actions and asynchronous workflow coordination for [`Baboon`].
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;
use crate::app::kits::loading::loose_entry_key_for_canonical_path;
use crate::app::mods::in_place::ContainerSaveRoute;
use crate::app::mods::in_place::container_save_route;
use anyhow::Context as _;

mod updates;
use updates::*;
// Re-exported: the browser's row menus gate on this, and its drawing functions
// reach it through egui memory rather than through `Baboon`.
pub(in crate::app) mod saving;
pub(super) use saving::available_definition_games;
use saving::{
    ordered_unique_keys, save_as_extension, save_as_file_name, save_as_start_dir,
};
mod documents;
pub(super) use crate::core::created_tags::{CreatedTagLedger, CreatedTagRecord};
mod group_report;

#[cfg(test)]
mod folder_extractable_tree_tests;

impl Baboon {
    /// Resolve (and cache) the Wwise media a Campaign Evolved `sound` tag binds
    /// to. Returns `None` for every other game and source kind.
    ///
    /// Resolution walks package imports and parses several cooked packages, so
    /// the result is memoized per tag key — the sound panel asks for it every
    /// frame. An unbound tag caches an empty binding, so stubs are not re-walked.
    pub(in crate::app) fn ce_sound_binding(
        &mut self,
        kit_index: usize,
        tag_key: &str,
        entry: &TagEntry,
    ) -> Option<std::sync::Arc<crate::core::source::ce_audio::CeSoundBinding>> {
        use crate::core::source::ce_audio;

        if !crate::app::editor::is_sound_group(entry.group_tag) {
            return None;
        }
        if let Some(hit) = self.kits[kit_index].ce_sound_bindings.get(tag_key) {
            return Some(hit.clone());
        }

        let TagEntryLocation::Container { rel_path, .. } = &entry.location else {
            return None;
        };
        let package = ce_audio::tag_package_for_rel_path(rel_path)?;
        self.ce_binding_for_package(kit_index, tag_key, &package)
    }

    /// The Wwise binding of a `sound` tag reached by *reference* rather than by
    /// browser entry — a `sound_looping` track's component, a dialogue
    /// vocalization. Resolves the reference against the mounted containers and
    /// then follows the same walk, memoized under the same per-kit cache.
    pub(in crate::app) fn ce_sound_binding_for_ref(
        &mut self,
        kit_index: usize,
        group_tag: u32,
        reference: &str,
    ) -> Option<std::sync::Arc<crate::core::source::ce_audio::CeSoundBinding>> {
        use crate::core::source::ce_audio;

        let Some(TagSource::IoStoreContainerSet { index, .. }) =
            self.kits[kit_index].source.as_ref().map(|s| &s.source)
        else {
            return None;
        };
        let (_, rel_path) = index.lookup(group_tag, reference)?;
        let package = ce_audio::tag_package_for_rel_path(rel_path)?;
        let cache_key = format!("ref:{package}");
        self.ce_binding_for_package(kit_index, &cache_key, &package)
    }

    /// Walk one cooked package out to its Wwise media, memoized under
    /// `cache_key` in this kit. Shared by both entry points above.
    fn ce_binding_for_package(
        &mut self,
        kit_index: usize,
        cache_key: &str,
        package: &str,
    ) -> Option<std::sync::Arc<crate::core::source::ce_audio::CeSoundBinding>> {
        use crate::core::source::ce_audio;

        if let Some(hit) = self.kits[kit_index].ce_sound_bindings.get(cache_key) {
            return Some(hit.clone());
        }

        // Checked before the usmap is parsed, as it always has been: a non-CE
        // source must bail out without paying for the bundled reflection data.
        // The `matches!` ends its borrow immediately, which the destructure
        // below could not do across the `ce_usmap` assignment.
        if !matches!(
            self.kits[kit_index].source.as_ref().map(|s| &s.source),
            Some(TagSource::IoStoreContainerSet { .. })
        ) {
            return None;
        }
        if self.ce_usmap.is_none() {
            match blam_tags::iostore::usmap::Usmap::meteorite() {
                Ok(u) => self.ce_usmap = Some(std::sync::Arc::new(u)),
                Err(err) => {
                    eprintln!("campaign evolved: could not parse bundled usmap: {err}");
                    return None;
                }
            }
        }
        let usmap = self.ce_usmap.clone()?;

        let Some(TagSource::IoStoreContainerSet {
            root,
            containers,
            packages,
            ..
        }) = self.kits[kit_index].source.as_ref().map(|s| &s.source)
        else {
            return None;
        };

        // The pak set is needed for events whose media is cooked inside a
        // SoundBank. A decode worker holds the lock only while it reads a
        // file's bytes.
        let store = self.audio.ce_media.clone();
        let mut store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let binding = std::sync::Arc::new(ce_audio::resolve_sound_binding(
            containers,
            packages,
            &usmap,
            package,
            Some((root.as_path(), &mut store)),
        ));
        self.kits[kit_index]
            .ce_sound_bindings
            .insert(cache_key.to_owned(), binding.clone());
        Some(binding)
    }

    /// Drain a queued referenced-sound click from a container source: resolve
    /// the reference's own Wwise binding, then queue the same playback or
    /// extraction the primary sound player would.
    pub(super) fn process_ce_sound_ref(&mut self) {
        let Some((kit_id, tab_key, request)) = self.pending_ce_sound_ref.take() else {
            return;
        };
        let Some(kit_index) = self.kit_index(kit_id) else {
            return;
        };
        let paks_root = match self.kits[kit_index].source.as_ref().map(|s| &s.source) {
            Some(TagSource::IoStoreContainerSet { root, .. }) => root.clone(),
            _ => return,
        };
        let Some(binding) =
            self.ce_sound_binding_for_ref(kit_index, request.group_tag, &request.reference)
        else {
            self.status = format!("{} not found in mounted containers", request.label);
            return;
        };
        if binding.is_empty() {
            self.status = format!("{} has no audio bound", request.label);
            return;
        }

        let language = binding.language_to_show(self.audio.language.as_deref());
        let media: Vec<crate::core::source::ce_audio::CeSoundMedia> = binding
            .media_for_language(&language)
            .into_iter()
            .cloned()
            .collect();
        if !request.extract {
            // Play the first permutation, matching the loose-folder player.
            let Some(first) = media.into_iter().next() else {
                return;
            };
            self.audio.pending.push_back(crate::app::audio::SoundRequest {
                owner: Some(crate::app::audio::SoundOwner {
                    kit: kit_id,
                    key: tab_key,
                }),
                clip: request.clip.clone(),
                preview: request.preview,
                action: crate::app::audio::SoundAction::PlayCeMedia {
                    paks_root,
                    label: format!("{} \u{00B7} {}", request.label, first.display_name()),
                    media: Box::new(first),
                },
            });
            return;
        }
        let Some(base) = rfd::FileDialog::new()
            .set_title(format!("Extract {}", request.label))
            .pick_folder()
        else {
            return;
        };
        let items = media
            .into_iter()
            .map(|m| crate::app::export::sound_extract::ExtractItem {
                out_path: base.join(format!(
                    "{}.wav",
                    crate::app::export::sound_extract::sanitize_component(&m.display_name())
                )),
                source: crate::app::export::sound_extract::ExtractSource::CeMedia {
                    paks_root: paks_root.clone(),
                    media: Box::new(m),
                },
            })
            .collect();
        self.pending_sound_extract = Some(crate::app::export::sound_extract::ExtractRequest {
            items,
            tags_root: None,
            label: request.label,
        });
    }

    pub(super) fn process_worker_messages(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.rx.try_recv() {
            self.apply_worker_message(message, ctx);
        }
    }

    /// Apply one worker result. Handlers drop a result whose source is stale
    /// themselves; what they return about it is not used here.
    pub(super) fn apply_worker_message(&mut self, message: WorkerMessage, ctx: &egui::Context) {
        {
            let _stale = match message {
                WorkerMessage::TerminalLine(line) => self.handle_terminal_line(line),
                WorkerMessage::TerminalLogError(error) => self.handle_terminal_log_error(error),
                WorkerMessage::TerminalDone { run_id } => self.handle_terminal_done(run_id),
                WorkerMessage::UpdateCheckFinished { silent, result } => {
                    self.handle_update_check_finished(silent, result)
                }
                WorkerMessage::FieldValueSearchFinished {
                    stamp,
                    query,
                    result,
                } => self.handle_field_value_search_finished(stamp, query, result),
                WorkerMessage::ScenarioPalettesRead { game, palettes } => {
                    let table = palettes.map_or(PaletteTable::Unreadable, PaletteTable::Ready);
                    self.kit_tool_drag.palettes.insert(game, table);
                    // A drag hovering Sapien with the mouse held still gets
                    // no event of its own to redraw the palette name with.
                    ctx.request_repaint();
                    false
                }
                WorkerMessage::FieldIndexBuilt { stamp, blobs } => {
                    self.handle_field_index_built(stamp, blobs)
                }
                WorkerMessage::FindAllProgress {
                    stamp,
                    request_id,
                    processed,
                    total,
                } => {
                    if self.resolve_stamp(stamp).is_some() && request_id == self.find.all_request_id
                    {
                        self.find.progress = Some((processed, total));
                    }
                    false
                }
                WorkerMessage::FindAllFinished {
                    stamp,
                    request_id,
                    occurrences,
                    unreadable,
                } => {
                    if self.resolve_stamp(stamp).is_some() && request_id == self.find.all_request_id
                    {
                        self.find.all_closed_occurrences = occurrences;
                        self.find.unreadable = unreadable;
                        self.find.searching = false;
                        self.find.progress = None;
                    }
                    false
                }
                WorkerMessage::SourceListingReady { stamp, results } => {
                    self.handle_source_listing_ready(stamp, results)
                }
                WorkerMessage::ReverseDependenciesBuilt {
                    stamp,
                    index,
                    missing,
                } => self.handle_reverse_dependencies_built(stamp, index, missing),
                WorkerMessage::ReferenceIndexProgress {
                    stamp,
                    processed,
                    total,
                } => self.handle_reference_index_progress(stamp, processed, total, ctx),
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path,
                } => self.handle_source_loaded(kit, result, recent_path, ctx),
                WorkerMessage::ChimpMounted { stamp, result } => {
                    self.handle_chimp_mounted(stamp, result, ctx.clone())
                }
                WorkerMessage::ChimpTypesIndexed { stamp, index } => {
                    self.handle_chimp_types_indexed(stamp, index)
                }
                WorkerMessage::ChimpReferrersScanned {
                    stamp,
                    package,
                    scan,
                } => self.handle_chimp_referrers_scanned(stamp, package, scan),
                WorkerMessage::ChimpModBuilt {
                    kit,
                    output,
                    temporary,
                    written,
                    result,
                } => self.handle_chimp_mod_built(kit, output, temporary, written, result, ctx),
                WorkerMessage::ChimpSourcesOverwritten {
                    kit,
                    leases,
                    containers,
                    touched,
                    written,
                    result,
                } => self.handle_chimp_sources_overwritten(
                    kit, leases, containers, touched, written, result, ctx,
                ),
                WorkerMessage::TagCompareGit { request, update } => {
                    self.handle_tag_compare_git(request, update)
                }
                WorkerMessage::GitReviewUpdated { kit, request, view } => {
                    self.handle_git_review_updated(kit, request, view)
                }
                WorkerMessage::ChimpPackageLoaded {
                    stamp,
                    package,
                    result,
                } => self.handle_chimp_package_loaded(stamp, package, result),
                WorkerMessage::TagLoaded { kit, key, result } => {
                    self.handle_tag_loaded(kit, key, result)
                }
                WorkerMessage::RefJumpOccurrences {
                    kit,
                    index,
                    key,
                    target,
                    result,
                } => self.handle_ref_jump_occurrences(kit, index, key, target, result),
                WorkerMessage::BitmapReimportFinished { kit, key, result } => {
                    self.handle_bitmap_reimport_finished(kit, key, result)
                }
                WorkerMessage::BlamImportProgress {
                    stamp,
                    kind,
                    message,
                } => self.handle_blam_import_progress(stamp, kind, message),
                WorkerMessage::BlamImportFinished {
                    stamp,
                    outcomes,
                    created,
                } => self.handle_blam_import_finished(stamp, outcomes, created),
                WorkerMessage::ContainerDuplicateFinished {
                    stamp,
                    lease,
                    result,
                } => self.handle_container_duplicate_finished(stamp, lease, result, ctx),
                WorkerMessage::ContainerRenameFinished {
                    stamp,
                    lease,
                    result,
                } => self.handle_container_rename_finished(stamp, lease, result, ctx),
                WorkerMessage::InPlaceOverwriteFinished {
                    job,
                    lease,
                    written,
                } => self.handle_in_place_overwrite_finished(*job, lease, written),
                WorkerMessage::ContainerDeleteFinished {
                    stamp,
                    lease,
                    result,
                } => self.handle_container_delete_finished(stamp, lease, result),
                WorkerMessage::ChimpLevelProgress {
                    kit,
                    phase,
                    done,
                    total,
                } => self.handle_chimp_level_progress(kit, phase, done, total),
                WorkerMessage::ContainerDumpProgress { stamp, done, total } => {
                    self.handle_container_dump_progress(stamp, done, total)
                }
                WorkerMessage::ContainerDumpFinished { stamp, result } => {
                    self.handle_container_dump_finished(stamp, result)
                }
                WorkerMessage::ExportFinished(result) => self.handle_export_finished(result),
                WorkerMessage::ChimpLevelExportFinished { job, result } => {
                    self.handle_chimp_level_export_finished(job, result)
                }
                WorkerMessage::PokePreflightFinished { kit, key, result } => {
                    self.handle_poke_preflight(kit, key, result);
                    false
                }
                WorkerMessage::PokeWriteFinished { kit, key, result } => {
                    self.handle_poke_write(kit, key, result);
                    false
                }
                WorkerMessage::PokeDirectFinished { kit, key, result } => {
                    self.handle_poke_direct(kit, key, result);
                    false
                }
                WorkerMessage::PokeUndoFinished { result, unapplied } => {
                    self.handle_poke_undo(result, unapplied);
                    false
                }
                WorkerMessage::CampaignProjectSaved {
                    revision,
                    path,
                    fingerprint,
                    result,
                } => self.handle_campaign_project_saved(revision, path, fingerprint, result),
                WorkerMessage::FolderRefactorProgress(progress) => {
                    self.handle_folder_refactor_progress(progress)
                }
                WorkerMessage::FolderRefactorFinished { stamp, result } => {
                    self.handle_folder_refactor_finished(stamp, result)
                }
                WorkerMessage::FolderConversionProgress(progress) => {
                    self.handle_folder_conversion_progress(progress)
                }
                WorkerMessage::FolderConversionFinished(report) => {
                    self.handle_folder_conversion_finished(report, ctx)
                }
                WorkerMessage::CacheImportProgress(progress) => {
                    self.handle_cache_import_progress(progress)
                }
                WorkerMessage::CacheImportFinished { stamp, result } => {
                    self.handle_cache_import_finished(stamp, result, ctx)
                }
                WorkerMessage::CacheImportConflicts { stamp, conflicts } => {
                    self.handle_cache_import_conflicts(stamp, conflicts)
                }
                WorkerMessage::ImportSourceResolved { input, result } => {
                    self.handle_import_source_resolved(input, result)
                }
                WorkerMessage::ImportAnalysisFinished { result, templates } => {
                    self.handle_import_analysis_finished(result, templates, ctx)
                }
                WorkerMessage::ModelTexturesResolved {
                    stamp,
                    key,
                    textures_id,
                    textures,
                } => self.handle_model_textures_resolved(stamp, key, textures_id, textures),
                WorkerMessage::BitmapThumbnailDecoded { stamp, key, result } => {
                    self.handle_thumbnail_ready::<Bitmaps>(stamp, key, result, ctx)
                }
                WorkerMessage::ModelThumbnailRendered { stamp, key, result } => {
                    self.handle_thumbnail_ready::<Models>(stamp, key, result, ctx)
                }
                WorkerMessage::ModelPreviewLoaded {
                    stamp,
                    key,
                    request_id,
                    result,
                } => self.handle_model_preview_loaded(stamp, key, request_id, result),
                WorkerMessage::ModelOverlaysBuilt {
                    stamp,
                    key,
                    geometry_id,
                    collision,
                    physics,
                } => self.handle_model_overlays_built(stamp, key, geometry_id, collision, physics),
                WorkerMessage::ModelAnimationsListed { stamp, key, result } => {
                    self.handle_model_animations_listed(stamp, key, result)
                }
                WorkerMessage::ModelAnimationDecoded {
                    stamp,
                    key,
                    animation_index,
                    result,
                } => self.handle_model_animation_decoded(stamp, key, animation_index, result),
                WorkerMessage::AllEntriesScanned { stamp, result } => {
                    self.handle_all_entries_scanned(stamp, result, ctx)
                }
                WorkerMessage::FolderExtractablesLoaded {
                    stamp,
                    rel_path,
                    label,
                    result,
                } => self.handle_folder_extractables_loaded(stamp, rel_path, label, result),
                WorkerMessage::EntryIndexScanProgress {
                    stamp,
                    processed,
                    total,
                    matched,
                } => self.handle_entry_index_scan_progress(stamp, processed, total, matched, ctx),
                WorkerMessage::EntryIndexRefreshed { stamp, result } => {
                    self.handle_entry_index_refreshed(stamp, result, ctx)
                }
                WorkerMessage::EntryIndexSaved {
                    stamp,
                    path,
                    result,
                } => self.handle_entry_index_saved(stamp, path, result),
            };
        }
    }

    pub(super) fn loaded_tags_root(&self) -> Option<PathBuf> {
        self.loaded_tags_root_for(self.active)
    }

    /// A specific kit's loose tags root. Background work has to name its kit:
    /// the one it started in may no longer be the focused one when it lands.
    pub(super) fn loaded_tags_root_for(&self, kit: usize) -> Option<PathBuf> {
        let TagSource::LooseFolder { root, .. } = &self.kits.get(kit)?.source.as_ref()?.source
        else {
            return None;
        };
        Some(root.clone())
    }

    /// Starts the non-blocking release lookup and returns its result through `WorkerMessage`.
    ///
    /// A `silent` check announces neither its start nor an uneventful result —
    /// that is the automatic startup check, which must not spend the status
    /// line on "up to date" or on a failure the user never asked about.
    pub(super) fn begin_check_for_updates(&mut self, ctx: egui::Context, silent: bool) {
        if !silent {
            self.status = "Checking for updates...".to_owned();
        }
        let channel = self.prefs.update_channel;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = fetch_latest_release(channel);
            let _ = tx.send(WorkerMessage::UpdateCheckFinished { silent, result });
            ctx.request_repaint();
        });
    }

    /// Whether the automatic startup check should run.
    pub(super) fn should_check_updates_on_startup(&self) -> bool {
        self.prefs.check_updates_on_startup
    }

    /// Throw away a tag's unsaved edits and put it back the way its source has
    /// it: drop the parsed document and everything derived from it, forget any
    /// stashed Campaign Evolved overlay, then reload the tag if it is open.
    ///
    /// Forgetting the overlay is the load-bearing half for a container source.
    /// The project autosaves every dirty tag within a second of the edit, so
    /// clearing the dirty flag alone leaves the edited bytes stashed and
    /// reopening the tag restores them — the edit would be unremovable.
    pub(super) fn discard_tag_changes(&mut self, kit: usize, key: &str, ctx: &egui::Context) {
        // Reloading below goes through the active-kit path, and discarding is a
        // user action on this kit either way.
        self.active = kit;
        let was_dirty = self.kits[kit]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set());
        let had_overlay = self.forget_campaign_overlay(kit, key);
        if !was_dirty && !had_overlay {
            self.status = "That tag has no unsaved changes".to_owned();
            return;
        }
        let label = self.tag_path_label(key);
        let kit_state = &mut self.kits[kit];
        kit_state.parsed_tags.remove(key);
        kit_state.loading_tags.remove(key);
        kit_state.bitmap_previews.remove(key);
        kit_state.model_previews.remove(key);
        kit_state.find_filter_applied.remove(key);
        kit_state.edit_buffers.forget_tag(key);
        // Persist the removal. The document is gone by now, so the capture
        // below cannot put the overlay straight back.
        if had_overlay {
            let now = ctx.input(|input| input.time);
            if let Err(error) = self.checkpoint_campaign_project(kit, now) {
                self.status = format!("Could not update the Campaign Evolved project: {error}");
                return;
            }
        }
        // A brand-new tag has no source to reload from — the document that was
        // just dropped WAS the tag. Take its browser entry with it instead of
        // leaving a row that errors on every reopen.
        if self.forget_new_container_entry(kit, key) {
            self.status = format!("Discarded the unsaved new tag {label}");
            return;
        }
        // Still open: reload it as the source has it, rather than leaving an
        // empty pane behind.
        if self.kits[kit].open_tabs.iter().any(|open| open == key) {
            self.select_entry(key.to_owned(), ctx.clone());
        }
        self.status = format!("Discarded unsaved changes to {label}");
    }

    /// Drop a brand-new (never-saved) container tag's browser entry, closing its
    /// pane and dropping everything derived from it. No-op — returning `false` —
    /// for any other kind of entry.
    ///
    /// Load-bearing for every path that discards a new tag's document: the
    /// document is the tag's ONLY copy (there is no `.ubulk` behind it), so an
    /// entry that outlives it is a row whose every reopen fails in `read_entry`
    /// with "unsaved new tag is no longer loaded".
    pub(super) fn forget_new_container_entry(&mut self, kit: usize, key: &str) -> bool {
        if !matches!(
            self.entry_for_key_in(kit, key).map(|entry| &entry.location),
            Some(TagEntryLocation::NewContainer { .. })
        ) {
            return false;
        }
        self.kits[kit].close_tag_pane(key);
        let folder_seeds = self.kits[kit].folder_seeds();
        let kit_state = &mut self.kits[kit];
        kit_state.parsed_tags.remove(key);
        kit_state.loading_tags.remove(key);
        kit_state.bitmap_previews.remove(key);
        kit_state.model_previews.remove(key);
        kit_state.find_filter_applied.remove(key);
        kit_state.edit_buffers.forget_tag(key);
        if kit_state.selected_key.as_deref() == Some(key) {
            kit_state.selected_key = None;
        }
        if let Some(source) = kit_state.source.as_mut() {
            source.remove_entry(key, &folder_seeds);
        }
        kit_state.set_tag_references(key, None);
        kit_state.generation = kit_state.generation.wrapping_add(1);
        true
    }

    /// Whether `key` has anything to discard — unsaved edits, or bytes stashed
    /// in this kit's project from an earlier session.
    pub(super) fn tag_has_discardable_changes(&self, kit: usize, key: &str) -> bool {
        self.kits[kit]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set())
            || self.tag_has_stashed_overlay(kit, key)
    }

    pub(super) fn select_entry(&mut self, key: String, ctx: egui::Context) {
        self.kits[self.active].open_tag_pane(&key);
        self.kits[self.active].selected_key = Some(key.clone());
        // A tag the project has an overlay for opens from the project, not from
        // disk — otherwise reopening it would silently discard its edits.
        if !self.load_campaign_overlay_for_key(self.active, &key) {
            self.ensure_tag_loading(key, ctx);
        }
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(super) fn ensure_tag_loading(&mut self, key: String, ctx: egui::Context) {
        if self.kits[self.active].parsed_tags.contains_key(&key)
            || self.kits[self.active].loading_tags.contains(&key)
        {
            return;
        }
        let Some(source) = self.source() else {
            return;
        };
        // Check both the lazily-loaded entries and the full scan set (all_entries).
        // Flat search results reference all_entries, which may not overlap with entries.
        let Some(entry) = source
            .entry_for_key(&key)
            .or_else(|| {
                self.kits[self.active]
                    .active_favorite_entries
                    .iter()
                    .find(|e| e.key == key)
            })
            .cloned()
        else {
            return;
        };
        // A new tag reaching here has lost its document and its project overlay
        // (`select_entry` tries the overlay first), so there is nothing left to
        // read — `read_entry` would only fail on the worker thread. Retire the
        // entry here instead of leaving a row that fails forever.
        if matches!(entry.location, TagEntryLocation::NewContainer { .. }) {
            let kit = self.active;
            self.forget_new_container_entry(kit, &key);
            self.status = format!("The unsaved new tag {} was discarded", entry.display_path);
            return;
        }
        let source_kind = source.source.clone();
        let kit = self.active_kit_id();
        self.kits[self.active].loading_tags.insert(key.clone());
        self.status = format!("Loading {}", entry.display_path);
        let panic_key = key.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || {
                let result = read_entry(&source_kind, &entry).map_err(|error| format!("{error:#}"));
                WorkerMessage::TagLoaded { kit, key, result }
            },
            move |error| WorkerMessage::TagLoaded {
                kit,
                key: panic_key,
                result: Err(error),
            },
        );
    }

    /// Kept for save/export paths that address "the current tag".
    #[allow(dead_code)]
    pub(super) fn selected_entry(&self) -> Option<&TagEntry> {
        let key = self.kits[self.active].selected_key.as_ref()?;
        self.entry_for_key(key)
    }

    pub(super) fn entry_for_key(&self, key: &str) -> Option<&TagEntry> {
        self.entry_for_key_in(self.active, key)
    }

    /// Resolve a tag key against a specific kit. Anything that runs for a kit
    /// other than the focused one has to use this: a key only means something
    /// inside its own source, so resolving it against the active kit silently
    /// finds nothing and the caller skips the tag.
    pub(super) fn entry_for_key_in(&self, kit: usize, key: &str) -> Option<&TagEntry> {
        self.kits.get(kit)?.entry_for_key(key)
    }

    pub(super) fn close_tab(&mut self, key: &str) {
        // `close_tag_pane` re-derives the open set and moves the selection off
        // a removed tag, so there is nothing to fix up afterwards.
        self.kits[self.active].close_tag_pane(key);
        self.unload_tag(key);
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn request_close_action(&mut self, action: PendingCloseAction, ctx: &egui::Context) {
        // Chimp's recovery checkpoints wait for edits to pause; one still
        // waiting when the app or a workspace closes would be lost.
        self.flush_all_chimp_checkpoints();
        if self.save_changes_prompt.visible
            || self.chimp_discard_prompt.is_some()
            || self.has_chimp_save_dialog()
        {
            return;
        }
        // A Chimp save is writing containers on a worker. The close waits
        // for it and runs from its completion, like a close the save dialog
        // was opened for.
        let writing = match &action {
            PendingCloseAction::CloseApp => self.chimp_writes.keys().next().copied(),
            PendingCloseAction::CloseKit(id) => self.chimp_writes.contains_key(id).then_some(*id),
            _ => None,
        };
        if let Some(kit) = writing {
            self.chimp_writes.insert(kit, Some(action));
            self.status = "Closing once the Chimp save finishes…".to_owned();
            return;
        }
        // The save prompt and every save path below it address documents by
        // tag key against the active kit. Point `active` at the kit the prompt
        // will be about first, so all of that — including the project check
        // just below — resolves against the right kit.
        match &action {
            PendingCloseAction::CloseKit(id) => {
                if let Some(index) = self.kit_index(*id) {
                    self.active = index;
                }
            }
            PendingCloseAction::CloseApp => {
                if let Some(index) = self.first_dirty_kit() {
                    self.active = index;
                }
            }
            _ => {}
        }
        // Upstream skipped the unsaved-changes prompt entirely for a container
        // source, checkpointing the project instead on the grounds that the
        // project retains the edits. It does — but silently, and there was no
        // way to say no: overlays were only ever inserted, so an edit could not
        // be taken back once stashed. The prompt is raised for these sources
        // too, and offers stashing as a third, named choice.
        let can_stash = self.current_source_is_campaign_project_capable(self.active);
        let dirty_tags = self.dirty_tags_for_close_action(&action);
        if !dirty_tags.is_empty() {
            // What discarding would cost, resolved here rather than described in the
            // abstract: these edits were stashed into the workspace's project within
            // a second of being typed, so declining to save deletes them from a file
            // that outlives the session.
            let stashed = dirty_tags
                .iter()
                .filter(|entry| self.tag_has_stashed_overlay(self.active, &entry.tag_id))
                .count();
            let stash_file = self.kits[self.active]
                .campaign_project
                .as_ref()
                .map(|project| project.recovery_path.clone());
            self.save_changes_prompt = SaveChangesPrompt {
                visible: true,
                can_stash,
                dirty_tags,
                pending_action: action,
                error: None,
                allow_app_close_once: self.save_changes_prompt.allow_app_close_once,
                stash_file,
                stashed,
                confirm_discard: false,
            };
            return;
        }

        let chimp_packages = self.dirty_chimp_for_close_action(&action);
        if !chimp_packages.is_empty() {
            self.open_chimp_discard_prompt(self.active, chimp_packages, Some(action), None);
            return;
        }

        self.execute_close_action(action, ctx);
    }

    /// Native app close is a two-step flow in eframe 0.29: when the OS close
    /// request arrives, Baboon sends `CancelClose` to veto it, shows the shared
    /// save prompt, then re-issues `ViewportCommand::Close` only after the user
    /// chooses Save or Don't Save. `allow_app_close_once` lets that confirmed
    /// second close request pass without opening the prompt again.
    pub(super) fn handle_app_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        if self.save_changes_prompt.allow_app_close_once {
            self.save_changes_prompt.allow_app_close_once = false;
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        // Quitting would kill the worker partway through rewriting references,
        // leaving some tags pointing at a path that no longer exists.
        if self.folder_refactor.is_some() {
            self.status = "Wait for the folder move/rename to finish before closing".to_owned();
            return;
        }
        if self.save_changes_prompt.visible
            || self.chimp_discard_prompt.is_some()
            || self.has_chimp_save_dialog()
        {
            return;
        }
        self.defer_file_action(DeferredFileAction::Close(PendingCloseAction::CloseApp), ctx);
    }

    fn dirty_tags_for_close_action(&self, action: &PendingCloseAction) -> Vec<DirtyTagEntry> {
        self.close_action_tag_keys(action)
            .into_iter()
            .filter_map(|key| {
                let doc = self.kits[self.active].parsed_tags.get(&key)?;
                if !doc.dirty.is_set() {
                    return None;
                }
                // Edits to a tag that has no writer (a monolithic build, a
                // big-endian tag) are session-scratch by construction. Listing
                // them here would offer a Save that always fails, and — for
                // CloseApp, which re-checks for dirty work after the prompt —
                // a close that never terminates.
                if !document_edits_are_saveable(&self.kits[self.active], &key, doc) {
                    return None;
                }
                Some(DirtyTagEntry {
                    path: self.tag_path_label(&key),
                    tag_id: key,
                    checked: true,
                })
            })
            .collect()
    }

    fn dirty_chimp_for_close_action(&self, action: &PendingCloseAction) -> Vec<String> {
        if close_action_includes_chimp(action) {
            self.chimp_dirty_packages(self.active)
        } else {
            Vec::new()
        }
    }

    fn close_action_tag_keys(&self, action: &PendingCloseAction) -> Vec<String> {
        match action {
            PendingCloseAction::CloseApp | PendingCloseAction::CloseAllTabs => {
                ordered_unique_keys(self.kits[self.active].open_tabs.iter())
            }
            PendingCloseAction::CloseTab(key) => vec![key.clone()],
            PendingCloseAction::CloseAllButThis(kept_key) => ordered_unique_keys(
                self.kits[self.active]
                    .open_tabs
                    .iter()
                    .filter(|key| *key != kept_key),
            ),
            // `request_close_action` has already made this kit active, so the
            // active-kit lookups above address the right documents.
            PendingCloseAction::CloseKit(_) => {
                ordered_unique_keys(self.kits[self.active].open_tabs.iter())
            }
        }
    }

    pub(super) fn tag_path_label(&self, key: &str) -> String {
        let Some(entry) = self.entry_for_key(key) else {
            return key.to_owned();
        };
        match &entry.location {
            TagEntryLocation::LooseFile(path) => path.display().to_string(),
            TagEntryLocation::Monolithic { .. }
            | TagEntryLocation::Container { .. }
            | TagEntryLocation::NewContainer { .. } => entry.display_path.clone(),
        }
    }

    /// Whether the loaded document for `key` still has unsaved edits. Save
    /// paths that report through `status` (container writes) use this to tell
    /// success from failure.
    pub(super) fn tag_is_dirty(&self, key: &str) -> bool {
        self.kits[self.active]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set())
    }

    fn execute_close_action(&mut self, action: PendingCloseAction, ctx: &egui::Context) {
        match action {
            PendingCloseAction::CloseApp => {
                // `request_close_action` is the close coordinator and only
                // calls this once every dirty workspace has been resolved. Do
                // not call it recursively here: a dirty Chimp document used
                // to be counted by this check but omitted from the tag prompt,
                // creating an infinite CloseApp -> request_close_action loop.
                if self.any_kit_dirty() {
                    self.status = "Could not close while unsaved workspace data remains".to_owned();
                    return;
                }
                if let Some(session) = self.current_session_state() {
                    if let Err(error) = save_last_session(&session) {
                        self.status = error;
                        return;
                    }
                } else {
                    clear_last_session();
                }
                self.save_changes_prompt.allow_app_close_once = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            PendingCloseAction::CloseTab(key) => self.close_tab(&key),
            PendingCloseAction::CloseAllTabs => self.close_all_tabs(),
            PendingCloseAction::CloseAllButThis(key) => self.close_all_tabs_but(&key),
            PendingCloseAction::CloseKit(id) => {
                self.remove_kit(id);
                self.color_popup = None;
                self.function_popup = None;
                self.status = "Closed kit".to_owned();
            }
        }
    }

    /// Snapshot every kit's source and open tag/folder panes for the restore prompt.
    pub(super) fn current_session_state(&self) -> Option<LastSessionState> {
        let kits = (0..self.kits.len())
            .filter_map(|index| self.session_kit_state(index))
            .collect::<Vec<_>>();
        (!kits.is_empty()).then_some(LastSessionState { kits })
    }

    fn session_kit_state(&self, kit_index: usize) -> Option<LastSessionKit> {
        let kit = &self.kits[kit_index];
        let was_active = kit_index == self.active;
        let source = kit.source.as_ref()?;
        let (source_kind, source_path) = match &source.source {
            TagSource::SingleFile { path } => (LastSessionSourceKind::SingleFile, path.clone()),
            TagSource::LooseFolder { root, .. } => {
                (LastSessionSourceKind::LooseFolder, root.clone())
            }
            TagSource::MonolithicCache { root, .. } => {
                (LastSessionSourceKind::MonolithicCache, root.clone())
            }
            TagSource::IoStoreContainerSet { root, .. } => {
                (LastSessionSourceKind::IoStoreContainerSet, root.clone())
            }
        };
        // Record the folder the user actually chose, not the directory the
        // source ended up reading from. They differ for exactly the sources
        // whose root is resolved inwards: a container set mounts from
        // `<install>/Meteorite/Content/Paks`, and a loose kit from
        // `<kit>/tags`. Storing the resolved one meant every session restore
        // reloaded that inner path and remembered *it* as a recent folder, so
        // "Paks" reappeared in the recents list after each restart however
        // often it was removed.
        let source_path = kit.requested_path.clone().unwrap_or(source_path);
        let mut tags = Vec::new();
        for key in ordered_unique_keys(kit.open_tabs.iter()) {
            let Some(entry) = source.entry_for_key(&key) else {
                continue;
            };
            let path = match &entry.location {
                TagEntryLocation::LooseFile(path) => {
                    Some(fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
                }
                TagEntryLocation::Monolithic { .. }
                | TagEntryLocation::Container { .. }
                | TagEntryLocation::NewContainer { .. } => None,
            };
            tags.push(LastSessionTag {
                key: entry.key.clone(),
                label: format!(
                    "{} - {}",
                    entry.display_path,
                    group_label(&kit.names, entry.group_tag)
                ),
                group_tag: entry.group_tag,
                path,
            });
        }
        let folders = ordered_unique_keys(kit.open_tabs.iter())
            .into_iter()
            .filter_map(|key| kit.folder_browsers.get(&key))
            .map(|folder| LastSessionFolder {
                rel_path: folder.rel_path.clone(),
                label: folder.label.clone(),
            })
            .collect();
        let chimp_packages = ordered_unique_keys(kit.chimp.open_packages.iter());
        let active_chimp_package = kit
            .chimp
            .selected_package
            .clone()
            .filter(|active| chimp_packages.contains(active));
        // The source itself is part of the workspace session, even when the
        // user has no tag, Chimp package, or project open in it. Otherwise a
        // second loaded editing kit disappears from the next-session prompt
        // simply because its tags were not selected yet.
        Some(LastSessionKit {
            source_kind,
            source_path,
            game: source.game.map(|game| game.as_str().to_owned()),
            profile_id: kit.profile.as_ref().map(|profile| profile.id.clone()),
            // The `.baboon` this workspace has open, if any — not its recovery
            // file, which the next session finds from the source root anyway.
            project_path: kit
                .campaign_project
                .as_ref()
                .and_then(|project| project.project_path.clone()),
            has_project: kit.campaign_project.is_some(),
            browser_mode: Some(kit.browser_mode),
            browser_sort: Some(kit.browser_sort),
            tags,
            folders,
            chimp_packages,
            active_chimp_package,
            // Read off the open tabs rather than the tag list: the libraries'
            // pane keys resolve to no entry, so the loop above skipped them.
            bitmap_library_open: kit.open_tabs.iter().any(|key| key == BITMAP_LIBRARY_KEY),
            model_library_open: kit.open_tabs.iter().any(|key| key == MODEL_LIBRARY_KEY),
            was_active,
        })
    }

    /// Reopen each saved kit. Every kit gets its own load, and its panes are
    /// staged on the kit itself rather than in one shared slot, so the loads
    /// can finish in any order without stealing each other's restore state.
    pub(super) fn begin_last_session_restore(&mut self, kits: Vec<RestoreKit>, ctx: egui::Context) {
        for RestoreKit {
            source_kind,
            source_path,
            profile_id,
            project_path,
            browser_mode,
            browser_sort,
            tags,
            folders,
            chimp_packages,
            active_chimp_package,
            bitmap_library_open,
            model_library_open,
            was_active,
        } in kits
        {
            match source_kind {
                LastSessionSourceKind::SingleFile => {
                    self.begin_load_single_path(source_path, ctx.clone())
                }
                LastSessionSourceKind::LooseFolder => {
                    let started = if let Some(profile) = profile_id
                        .as_deref()
                        .and_then(|id| {
                            self.prefs
                                .custom_editing_kit_profiles
                                .iter()
                                .find(|profile| profile.id == id)
                        })
                        .cloned()
                    {
                        self.load_custom_editing_kit_profile(profile, ctx.clone())
                    } else {
                        self.begin_load_folder_path(source_path, ctx.clone());
                        true
                    };
                    if !started {
                        continue;
                    }
                }
                LastSessionSourceKind::MonolithicCache => {
                    let blob_index = if source_path.is_dir() {
                        source_path.join("blob_index.dat")
                    } else {
                        source_path
                    };
                    self.begin_load_monolithic_path(blob_index, ctx.clone());
                }
                // Upstream added container sources to the session format, so a
                // Campaign Evolved install now comes back with the rest.
                LastSessionSourceKind::IoStoreContainerSet => {
                    self.begin_load_folder_path(install_root_for_paks(&source_path), ctx.clone())
                }
            }
            // The loaders route to a kit and leave it active, so this stages
            // the tags on the kit the load will land in.
            //
            // Each load also finishes by making its own kit active, so the
            // focused workspace would otherwise be whichever one happened to
            // load last. Remember the kit the session named and every kit still
            // to land, so the focus can be set once they all have.
            let restoring = self.kits[self.active].id;
            self.restoring_kits.insert(restoring);
            if was_active {
                self.restored_active_kit = Some(restoring);
            }
            self.kits[self.active].pending_restore_tags = tags;
            self.kits[self.active].pending_restore_folders = folders;
            self.kits[self.active].pending_restore_chimp_packages = chimp_packages;
            self.kits[self.active].pending_restore_bitmap_library = bitmap_library_open;
            self.kits[self.active].pending_restore_model_library = model_library_open;
            self.kits[self.active].pending_restore_active_chimp_package = active_chimp_package;
            // Its browser view is staged the same way: `install_loaded_source`
            // carries it across the load rather than resetting it, so each
            // workspace comes back in the view it was left in.
            if let Some(mode) = browser_mode {
                self.kits[self.active].browser_mode = mode;
            }
            if let Some(sort) = browser_sort {
                self.kits[self.active].browser_sort = sort;
            }
            // The project file it had open is queued the same way, and is
            // attached as this workspace's save target once the source has
            // mounted. The edits themselves come back from the recovery file.
            if let Some(project_path) = project_path {
                let restoring = self.active;
                self.queue_campaign_project_target(restoring, project_path);
            }
        }
    }

    /// Record the session as the event loop tears down.
    ///
    /// Baboon's whole shutdown chain hangs off a window close request:
    /// `handle_app_close_request` only acts on `close_requested()`, and it is
    /// what eventually reaches [`Self::execute_close_action`] and saves the
    /// session. macOS never sends one for Cmd+Q — AppKit posts
    /// `applicationWillTerminate:`, which closes each window directly rather
    /// than asking it to close, so no `CloseRequested` is ever emitted and none
    /// of that runs. The session file was then left holding whatever last wrote
    /// it, which for a Campaign Evolved workspace is its project autosave: quit
    /// with a Halo 3 kit open and the next launch restored Campaign Evolved,
    /// because that was the last session anything had recorded.
    ///
    /// This runs on every shutdown, including the ordinary one that already
    /// saved a moment earlier — the write is the same document either way. It
    /// cannot prompt: the loop is already exiting and `LoopExiting` cannot be
    /// vetoed, so unsaved tag edits still go unremarked on a Cmd+Q.
    pub(super) fn persist_session_on_exit(&mut self) {
        match self.current_session_state() {
            Some(session) => {
                let _ = save_last_session(&session);
            }
            None => clear_last_session(),
        }
    }

    /// Mark one restored kit's load as settled, whatever became of it, and once
    /// none are left hand the focus to the kit the session named.
    ///
    /// Every completed load makes its own kit active, so during a restore the
    /// focused workspace is otherwise decided by which source finishes first —
    /// a loose folder against a container set is not a race with a stable
    /// winner. The saved kit is only honoured while it is still open and it
    /// still loaded; a kit the user unchecked in the restore prompt, or whose
    /// source has since moved, leaves the focus wherever the loads put it.
    pub(super) fn settle_restored_kit(&mut self, kit: KitId) {
        let Some(active) =
            focus_after_restore(&mut self.restoring_kits, &mut self.restored_active_kit, kit)
        else {
            return;
        };
        if let Some(index) = self.kit_index(active) {
            self.active = index;
        }
    }

    /// Reopen the panes staged for the kit that just finished loading.
    pub(in crate::app) fn finish_pending_session_restore(&mut self, ctx: egui::Context) {
        // Ahead of the early return below: a workspace whose only open tab was
        // the Bitmap Library has no tags staged, and would otherwise come back
        // without it.
        if std::mem::take(&mut self.kits[self.active].pending_restore_bitmap_library) {
            self.open_bitmap_library();
        }
        if std::mem::take(&mut self.kits[self.active].pending_restore_model_library) {
            self.open_model_library();
        }
        let restore_folders = std::mem::take(&mut self.kits[self.active].pending_restore_folders);
        for folder in &restore_folders {
            self.handle_browser_action(
                BrowserAction::OpenFolderBrowser {
                    rel_path: folder.rel_path.clone(),
                    label: folder.label.clone(),
                    open_in_new_tab: true,
                },
                ctx.clone(),
            );
        }
        let restore = std::mem::take(&mut self.kits[self.active].pending_restore_tags);
        if restore.is_empty() && restore_folders.is_empty() {
            return;
        }
        let mut opened = restore_folders.len();
        let mut missing = 0usize;
        for tag in restore {
            if let Some(current_key) = self.restored_tag_entry_key(&tag) {
                self.select_entry(current_key, ctx.clone());
                opened += 1;
            } else {
                missing += 1;
            }
        }
        if opened > 0 {
            self.status = if missing > 0 {
                format!("Restored {opened} window(s); skipped {missing} missing item(s)")
            } else {
                format!("Restored {opened} window(s)")
            };
        } else if missing > 0 {
            self.status = "No saved windows could be restored".to_owned();
        }
    }

    /// Resolve a saved pane to the key used by the freshly mounted source.
    ///
    /// Loose-file keys include a displayed filesystem path. Windows accepts
    /// both separators, and older sessions could therefore persist a mixed
    /// `file:C:\.../objects\...` spelling that no longer compared equal to the
    /// newly scanned entry. Rediscovering the file was not enough: restore then
    /// opened the stale saved key and reported that the tag had disappeared.
    /// Return the source's current key so existing sessions recover in place.
    fn restored_tag_entry_key(&mut self, tag: &LastSessionTag) -> Option<String> {
        if let Some(entry) = self.entry_for_key(&tag.key) {
            return Some(entry.key.clone());
        }
        let path = tag.path.as_ref()?;
        if !path.is_file() {
            return None;
        }
        let source = self.source()?;
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return None;
        };
        let root = fs::canonicalize(root).ok()?;
        let path = fs::canonicalize(path).ok()?;
        if !path.starts_with(&root) {
            return None;
        }
        if let Some(current_key) = loose_entry_key_for_canonical_path(
            source.entries.iter().chain(source.all_entries.iter()),
            &path,
        ) {
            return Some(current_key);
        }
        let entry = loose_file_entry(&root, &path, &source.names).ok()??;
        let current_key = entry.key.clone();
        let folder_seeds = self.kits[self.active].folder_seeds();
        if let Some(source) = self.source_mut() {
            if tag.key != current_key {
                source.remove_entry(&tag.key, &folder_seeds);
            }
            source.upsert_entry(entry, &folder_seeds);
        }
        self.kits[self.active].generation = self.kits[self.active].generation.wrapping_add(1);
        Some(current_key)
    }

    pub(super) fn close_all_tabs(&mut self) {
        let id = self.kits[self.active].id;
        self.kits[self.active].tag_tree = egui_tiles::Tree::empty(tag_tree_id(id));
        self.kits[self.active].open_tabs.clear();
        self.kits[self.active].drop_documents_except(None);
        self.kits[self.active].selected_key = None;
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn close_all_tabs_but(&mut self, key: &str) {
        for open in self.kits[self.active].tabs_from_tree() {
            if open != key {
                self.kits[self.active].close_tag_pane(&open);
            }
        }
        self.kits[self.active].drop_documents_except(Some(key));
        self.kits[self.active].selected_key = (!is_folder_pane_key(key)).then(|| key.to_owned());
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn unload_tag(&mut self, key: &str) {
        self.kits[self.active].drop_document(key);
    }

    /// Start resolving a loaded model's materials to textures, if it needs it.
    ///
    /// Idempotent and cheap to call every frame: it only spawns when a model is
    /// loaded, a textured mode is on, its textures are still absent, and no job for it
    /// is already running.
    pub(in crate::app) fn maybe_request_model_textures(
        &mut self,
        kit_index: usize,
        key: &str,
        ctx: &egui::Context,
    ) {
        let Some(state) = self.kits[kit_index].model_previews.get(key) else {
            return;
        };
        if !state.render_mode.uses_textures() || state.textures_pending {
            return;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return;
        };
        if data.textures.is_some() || data.preview.materials.is_empty() {
            return;
        }
        let Some(source) = self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.source.clone())
        else {
            return;
        };
        let materials = data.preview.materials.clone();
        let textures_id = data.textures_id;
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
        };
        if let Some(state) = self.kits[kit_index].model_previews.get_mut(key) {
            state.textures_pending = true;
        }

        let (key, panic_key) = (key.to_owned(), key.to_owned());
        spawn_worker(
            &self.tx,
            ctx,
            move || WorkerMessage::ModelTexturesResolved {
                stamp,
                key,
                textures_id,
                textures: resolve_model_textures(&source, &materials),
            },
            // No textures: the preview draws untextured, and stops waiting.
            move |_| WorkerMessage::ModelTexturesResolved {
                stamp,
                key: panic_key,
                textures_id,
                textures: Vec::new(),
            },
        );
    }

    pub(in crate::app) fn handle_model_textures_resolved(
        &mut self,
        stamp: KitStamp,
        key: String,
        textures_id: u64,
        textures: Vec<MaterialTextures>,
    ) -> bool {
        let Some(kit_index) = self.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.resolve_stamp(stamp).is_none();
        let Some(state) = self.kits[kit_index].model_previews.get_mut(&key) else {
            return true;
        };
        // The in-flight marker is cleared before the staleness check: a result
        // dropped for a generation bump used to leave it set, and the preview
        // then waited for it for good ("Loading shaders…", repainting every
        // frame).
        state.textures_pending = false;
        if stale {
            return true;
        }
        let Some(Ok(data)) = state.data.as_mut() else {
            return true;
        };
        // The model was reloaded while this ran, and these textures are
        // indexed against the materials of the old one.
        if data.textures_id != textures_id {
            return true;
        }
        data.textures = Some(std::sync::Arc::new(textures));
        false
    }

    pub(super) fn active_game_is_campaign_evolved(&self) -> bool {
        self.source_game().is_some_and(GameId::is_campaign_evolved)
    }

    pub(super) fn save_current_tag(&mut self, ctx: &egui::Context) {
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
                    self.overwrite_confirm = Some(OverwriteConfirm {
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

    pub(super) fn save_tag_by_key(&mut self, key: &str) -> Result<PathBuf, String> {
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
    fn record_saved_tag_in_indexes(&mut self, entry: &TagEntry, dependencies: Vec<DependencyRef>) {
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

    pub(super) fn save_current_tag_as(&mut self) {
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

    /// Apply pasted TSV (header row = field names, one data row per element) to
    /// the target block's EXISTING elements, cell-by-cell via `apply_field_edit`.
    /// Rows beyond the current element count are reported and ignored (no
    /// structural changes — fully covered by undo). Returns a status summary.
    pub(super) fn apply_tsv_paste(&mut self) {
        // The document is looked up in the active kit below, and two workspaces
        // of the same game share a key space, so a paste answered after a
        // switch could land in the wrong game's tag rather than simply missing.
        let Some(kit) = self.tsv_paste.as_ref().map(|paste| paste.kit) else {
            return;
        };
        if !self.focus_navigation_kit(kit) {
            self.set_tsv_paste_status("The workspace this paste came from is closed.");
            return;
        }
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        let Some(paste) = self.tsv_paste.as_ref() else {
            return;
        };
        let tag_key = paste.tag_key.clone();
        let block_path = paste.block_path.clone();
        let text = paste.text.clone();

        let Some(doc) = self.kits[self.active].parsed_tags.get_mut(&tag_key) else {
            self.set_tsv_paste_status("Tag is no longer open.");
            return;
        };
        let Some(block) = doc
            .tag
            .root()
            .field_path(&block_path)
            .and_then(|field| field.as_block())
        else {
            self.set_tsv_paste_status("Block no longer resolves in this tag.");
            return;
        };
        let element_count = block.len();
        let columns = block_leaf_columns(&block); // (clean, full) per leaf field

        let mut lines = text.lines();
        let Some(header_line) = lines.next() else {
            self.set_tsv_paste_status("Nothing to paste.");
            return;
        };
        // Map each pasted column index → the full field name to write.
        let header_to_full = map_tsv_header_to_fields(header_line, &columns);
        if header_to_full.iter().all(Option::is_none) {
            self.set_tsv_paste_status("No pasted column headers matched this block's fields.");
            return;
        }

        let mut edits = Vec::new();
        let mut data_rows = 0usize;
        let mut skipped_rows = 0usize;
        for (row_index, line) in lines.enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            data_rows += 1;
            if row_index >= element_count {
                skipped_rows += 1;
                continue;
            }
            for (col_index, cell) in line.split('\t').enumerate() {
                if let Some(Some(full)) = header_to_full.get(col_index) {
                    edits.push(PendingFieldEdit {
                        path: format!("{block_path}[{row_index}]/{full}"),
                        input: cell.trim().to_owned(),
                    });
                }
            }
        }

        if edits.is_empty() {
            self.set_tsv_paste_status("No editable cells matched.");
            return;
        }
        let applied_rows = data_rows.saturating_sub(skipped_rows);
        let active = self.active;
        let ops = DeferredOps {
            pending: edits,
            ..DeferredOps::default()
        };
        let Some(applied) = self.apply_doc_ops(active, &tag_key, "Paste TSV", ops, UndoStep::Own)
        else {
            return;
        };
        self.invalidate_tag_caches_in(active, &tag_key);

        let summary =
            tsv_paste_summary(&applied.outcomes, applied_rows, skipped_rows, element_count);
        self.status = summary.clone();
        self.set_tsv_paste_status(&summary);
    }

    fn set_tsv_paste_status(&mut self, message: &str) {
        if let Some(paste) = self.tsv_paste.as_mut() {
            paste.status = Some(message.to_owned());
        }
    }

    /// The documentation overlay (help/units + explanations) for a group,
    /// parsed once from its definition JSON and cached. `None` when the
    /// definitions can't be located (e.g. non-loose sources).
    /// Documentation overlay for `entry`'s group, resolved against `kit_index`
    /// rather than the active kit — in a split the two panes can be different
    /// games, whose definitions and group naming differ.
    pub(super) fn def_docs_for_entry(
        &mut self,
        kit_index: usize,
        entry: &TagEntry,
    ) -> Option<Rc<DefDocs>> {
        let source = self.kits[kit_index].source.as_ref()?;
        let root = match &source.source {
            TagSource::LooseFolder {
                definitions_root, ..
            } => definitions_root.clone(),
            _ => return None,
        };
        let game = source.game.clone()?;
        let group = self.kits[kit_index]
            .names
            .name_for(entry.group_tag)
            .or_else(|| group_tag_to_extension(entry.group_tag))?
            .to_owned();
        // Cache key is the group's own JSON path; the docs themselves merge the
        // whole `parent_tag` inheritance chain (object-family fields live in
        // parent files).
        let path = root.join(game.as_str()).join(format!("{group}.json"));
        if let Some(docs) = self.def_docs_cache.get(&path) {
            return Some(docs.clone());
        }
        let docs = Rc::new(build_def_docs(&root, game, &group));
        self.def_docs_cache.insert(path, docs.clone());
        Some(docs)
    }

    /// Drop cached previews derived from a tag's contents so they rebuild from
    /// the (newly restored) tag bytes after an undo/redo.
    /// Drop derived previews for `key` in `kit`, after its document changed.
    pub(super) fn invalidate_tag_caches_in(&mut self, kit: usize, key: &str) {
        if let Some(preview) = self.kits[kit].model_previews.get_mut(key) {
            preview.invalidate_load();
        }
        if let Some(bitmap) = self.kits[kit].bitmap_previews.get_mut(key) {
            bitmap.decoded = None;
            bitmap.decoding = None;
            bitmap.texture = None;
            bitmap.texture_dirty = true;
        }
        // rmdf/rmop caches are keyed by external render-method paths, not by this
        // tag's contents, and the shader grid's model is keyed by the document's
        // dirty revision, which the change has already moved — so nothing to
        // clear there.
    }

    /// Whether the active kit is showing its Chimp surface rather than tags.
    ///
    /// Undo and redo act on the selected tag, which is hidden there, and Chimp
    /// has no undo of its own yet; so on that surface they do nothing rather
    /// than silently changing a tag the user cannot see.
    fn chimp_surface_is_active(&self) -> bool {
        self.prefs.enable_chimp && self.kits[self.active].surface == KitSurface::Chimp
    }

    pub(super) fn undo_current_tag(&mut self) {
        if self.chimp_surface_is_active() {
            self.status = "Chimp has no undo yet; undo applies to tags.".to_owned();
            return;
        }
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        let Some(key) = self.kits[self.active].selected_key.clone() else {
            self.status = "Nothing to undo".to_owned();
            return;
        };
        let restored = self.kits[self.active]
            .parsed_tags
            .get_mut(&key)
            .and_then(|doc| doc.journal.undo(&doc.tag));
        self.restore_snapshot(&key, restored, "Undo");
    }

    pub(super) fn redo_current_tag(&mut self) {
        if self.chimp_surface_is_active() {
            self.status = "Chimp has no redo yet; redo applies to tags.".to_owned();
            return;
        }
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        let Some(key) = self.kits[self.active].selected_key.clone() else {
            self.status = "Nothing to redo".to_owned();
            return;
        };
        let restored = self.kits[self.active]
            .parsed_tags
            .get_mut(&key)
            .and_then(|doc| doc.journal.redo(&doc.tag));
        self.restore_snapshot(&key, restored, "Redo");
    }

    /// Apply a snapshot returned by the journal: re-parse the bytes into the
    /// document and invalidate derived caches.
    fn restore_snapshot(
        &mut self,
        key: &str,
        restored: Option<(Arc<Vec<u8>>, String)>,
        verb: &str,
    ) {
        // Classic (Halo CE / Halo 2) snapshots are serialized in classic format,
        // which `read_from_bytes` can't parse — re-parse with the JSON layout.
        let group_tag = self.kits[self.active]
            .parsed_tags
            .get(key)
            .map(|doc| doc.tag.group().tag);
        let game = self.source_game();
        let definitions_root = self.source_definitions_root().map(Path::to_owned);
        match restored {
            Some((bytes, label)) => {
                match group_tag
                    .context("no open tag to restore")
                    .and_then(|group_tag| {
                        crate::core::source::read_tag_from_bytes(
                            &bytes,
                            game,
                            definitions_root.as_deref(),
                            group_tag,
                        )
                    }) {
                    Ok(tag) => {
                        if let Some(doc) = self.kits[self.active].parsed_tags.get_mut(key) {
                            doc.tag = tag;
                            doc.dirty.touch();
                        }
                        let active = self.active;
                        self.invalidate_tag_caches_in(active, key);
                        self.status = format!("{verb}: {label}");
                    }
                    Err(error) => {
                        self.status = format!("{verb} failed: {error}");
                    }
                }
            }
            None => {
                self.status = format!("Nothing to {}", verb.to_ascii_lowercase());
            }
        }
    }

    pub(super) fn can_undo_current(&self) -> bool {
        if self.chimp_surface_is_active() || self.editing_kit_is_read_only(self.active) {
            return false;
        }
        self.kits[self.active]
            .selected_key
            .as_ref()
            .and_then(|key| self.kits[self.active].parsed_tags.get(key))
            .is_some_and(|doc| doc.journal.can_undo())
    }

    pub(super) fn can_redo_current(&self) -> bool {
        if self.chimp_surface_is_active() || self.editing_kit_is_read_only(self.active) {
            return false;
        }
        self.kits[self.active]
            .selected_key
            .as_ref()
            .and_then(|key| self.kits[self.active].parsed_tags.get(key))
            .is_some_and(|doc| doc.journal.can_redo())
    }

    pub(super) fn current_prefs(&self) -> GuiPrefs {
        GuiPrefs {
            // The focused workspace's view is what a new one is seeded with,
            // so a single-workspace session remembers its choice as before.
            browser_mode: self.kits[self.active].browser_mode,
            browser_sort: self.kits[self.active].browser_sort,
            ..self.prefs.clone()
        }
    }

    /// Render the block delete/delete-all confirmation modal (if pending) and
    /// apply the op on confirm.
    pub(super) fn handle_block_confirm(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.block_confirm.as_ref() else {
            return;
        };
        // The op is applied to the active kit's document, and two workspaces of
        // the same game share a key space, so a confirmation answered after a
        // switch could delete from the wrong game's tag.
        let confirm_kit = confirm.kit;
        let message = confirm.message.clone();
        let confirm_label = confirm.confirm_label.clone();
        let mut do_apply = false;
        let mut do_cancel = false;
        egui::Window::new("Confirm")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(RichText::new(message).color(text_dark()));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(&confirm_label)
                                    .color(Color32::from_rgb(230, 230, 228)),
                            )
                            .fill(Color32::from_rgb(150, 48, 40))
                            .min_size(Vec2::new(80.0, 24.0)),
                        )
                        .clicked()
                    {
                        do_apply = true;
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(Vec2::new(80.0, 24.0)))
                        .clicked()
                    {
                        do_cancel = true;
                    }
                });
            });
        if do_apply {
            let routed = confirm_kit.is_some_and(|kit| self.focus_navigation_kit(kit));
            if routed && self.refuse_read_only_edit(self.active) {
                self.block_confirm = None;
                return;
            }
            if let Some(confirm) = self.block_confirm.take()
                && routed
            {
                let deletes_model_variant = confirm.path == "variants"
                    && matches!(confirm.kind, BlockOpKind::Delete(_))
                    && self.kits[self.active]
                        .parsed_tags
                        .get(&confirm.tag_key)
                        .is_some_and(|doc| doc.tag.header.group_tag.to_be_bytes() == *b"hlmt");
                let ops = DeferredOps {
                    block_ops: vec![BlockOp {
                        path: confirm.path,
                        kind: confirm.kind,
                    }],
                    ..DeferredOps::default()
                };
                let active = self.active;
                let applied =
                    self.apply_doc_ops(active, &confirm.tag_key, "Block edit", ops, UndoStep::Own);
                let refresh_model_preview = deletes_model_variant
                    && applied.is_some_and(|applied| applied.status.is_some());
                if refresh_model_preview
                    && let Some(preview) = self.kits[self.active]
                        .model_previews
                        .get_mut(&confirm.tag_key)
                {
                    preview.selected_variant = None;
                    preview.invalidate_load();
                }
            }
        } else if do_cancel {
            self.block_confirm = None;
        }
    }

    pub(super) fn handle_save_changes_prompt(&mut self, ctx: &egui::Context) {
        let action = render_save_changes_prompt(ctx, &mut self.save_changes_prompt);
        match action {
            SaveChangesPromptAction::None => {}
            SaveChangesPromptAction::Cancel => {
                self.save_changes_prompt.visible = false;
                self.save_changes_prompt.dirty_tags.clear();
                self.save_changes_prompt.error = None;
                self.save_changes_prompt.confirm_discard = false;
            }
            // Arming, not acting: the click that deletes is the next one.
            SaveChangesPromptAction::ConfirmDiscard => {
                self.save_changes_prompt.confirm_discard = true;
            }
            SaveChangesPromptAction::StashForMod => {
                let action = self.save_changes_prompt.pending_action.clone();
                let now = ctx.input(|input| input.time);
                match self.checkpoint_campaign_project(self.active, now) {
                    Ok(_) => {
                        // The project holds these bytes now, so they are no
                        // longer unsaved work: leaving them dirty would prompt
                        // again on the next close and, for a CloseApp walking
                        // several kits, would never terminate.
                        for entry in &self.save_changes_prompt.dirty_tags {
                            if let Some(document) =
                                self.kits[self.active].parsed_tags.get_mut(&entry.tag_id)
                            {
                                document.dirty.clear();
                            }
                        }
                        self.save_changes_prompt.visible = false;
                        self.save_changes_prompt.dirty_tags.clear();
                        self.save_changes_prompt.error = None;
                        self.save_changes_prompt.confirm_discard = false;
                        self.status = match self.kits[self.active]
                            .campaign_project
                            .as_ref()
                            .and_then(|project| project.project_path.clone())
                        {
                            // Named, because the file the user thinks of as their
                            // project is not the one this wrote.
                            Some(path) => format!(
                                "Stashed for Export Mod. {} is unchanged until you save it",
                                path.display()
                            ),
                            None => "Stashed for Export Mod".to_owned(),
                        };
                        self.request_close_action(action, ctx);
                    }
                    Err(error) => {
                        self.save_changes_prompt.error =
                            Some(format!("Could not stash into the project: {error}"));
                    }
                }
            }
            SaveChangesPromptAction::DontSave => {
                let action = self.save_changes_prompt.pending_action.clone();
                // Discarding is explicit, so drop the dirty flags the prompt
                // listed. Without this, a CloseApp that spans several kits
                // would see the same unsaved work again and re-prompt forever.
                //
                // On a stashing workspace this also deletes the stashed copies —
                // which is why the button is named Discard there and takes a
                // second, confirming click. What it deletes from is the
                // workspace's own recovery file; a `.baboon` the user opened or
                // saved is never written by a close.
                let kit = self.active;
                let tag_ids: Vec<String> = self
                    .save_changes_prompt
                    .dirty_tags
                    .iter()
                    .map(|entry| entry.tag_id.clone())
                    .collect();
                for tag_id in &tag_ids {
                    if let Some(doc) = self.kits[kit].parsed_tags.get_mut(tag_id) {
                        doc.dirty.clear();
                    }
                    // And forget anything the project stashed for it. Autosave
                    // captures a dirty tag within a second of the edit, so
                    // without this "Don't Save" cleared a flag while the edited
                    // bytes stayed behind and came back on reopen.
                    self.forget_campaign_overlay(kit, tag_id);
                    self.kits[kit].edit_buffers.forget_tag(tag_id);
                    // Declining to save a brand-new tag discards the tag, not
                    // just its edits: nothing backs it but the document the
                    // close is about to drop. Its browser entry goes with it.
                    self.forget_new_container_entry(kit, tag_id);
                }
                let now = ctx.input(|input| input.time);
                if let Err(error) = self.checkpoint_campaign_project(kit, now) {
                    self.status = format!("Could not update the Campaign Evolved project: {error}");
                }
                self.save_changes_prompt.visible = false;
                self.save_changes_prompt.dirty_tags.clear();
                self.save_changes_prompt.error = None;
                self.save_changes_prompt.confirm_discard = false;
                self.request_close_action(action, ctx);
            }
            SaveChangesPromptAction::Save(tag_ids) => {
                let mut saved = Vec::new();
                let mut errors = Vec::new();
                for tag_id in tag_ids {
                    // Container tags have no loose file to write. A brand-new
                    // one saves via a file dialog (new override container); an
                    // existing one is overwritten inside the game's pak, with
                    // this prompt's Save button standing in for the separate
                    // overwrite confirmation. Both report through `status`
                    // instead of returning a path, so success is read back off
                    // the document's dirty flag.
                    match close_prompt_save_route(
                        self.entry_for_key(&tag_id).map(|entry| &entry.location),
                    ) {
                        ClosePromptSave::NewContainer => {
                            self.save_new_container_tag(&tag_id);
                            if self.tag_is_dirty(&tag_id) {
                                let label = self.tag_path_label(&tag_id);
                                errors.push(format!("{label}: not saved"));
                            } else {
                                saved.push(tag_id.clone());
                            }
                            continue;
                        }
                        ClosePromptSave::ContainerInPlace => {
                            self.overwrite_current_tag_in_place(&tag_id);
                            if self.tag_is_dirty(&tag_id) {
                                // The overwrite failure reason is in `status`.
                                let label = self.tag_path_label(&tag_id);
                                let detail = self.status.clone();
                                errors.push(format!("{label}: {detail}"));
                            } else {
                                saved.push(tag_id.clone());
                            }
                            continue;
                        }
                        ClosePromptSave::File => {}
                    }
                    match self.save_tag_by_key(&tag_id) {
                        Ok(path) => saved.push(path.display().to_string()),
                        Err(error) => {
                            let label = self.tag_path_label(&tag_id);
                            errors.push(format!("{label}: {error}"));
                        }
                    }
                }
                if errors.is_empty() {
                    let action = self.save_changes_prompt.pending_action.clone();
                    self.save_changes_prompt.visible = false;
                    self.save_changes_prompt.dirty_tags.clear();
                    self.save_changes_prompt.error = None;
                    self.status = if saved.is_empty() {
                        "No files selected to save".to_owned()
                    } else {
                        format!("Saved {} file(s)", saved.len())
                    };
                    self.request_close_action(action, ctx);
                } else {
                    let message = format!("Save failed: {}", errors.join("; "));
                    let pending_action = self.save_changes_prompt.pending_action.clone();
                    self.save_changes_prompt.dirty_tags =
                        self.dirty_tags_for_close_action(&pending_action);
                    // A failed save leaves the prompt up, and an armed discard
                    // has no business surviving into it.
                    self.save_changes_prompt.confirm_discard = false;
                    self.status = message.clone();
                    self.save_changes_prompt.error = Some(message);
                }
            }
        }
    }

    pub(super) fn handle_last_opened_windows_prompt(&mut self, ctx: &egui::Context) {
        let action = render_last_opened_windows_prompt(ctx, self.last_opened_windows.as_mut());
        match action {
            LastOpenedWindowsAction::None => {}
            LastOpenedWindowsAction::OpenSettings => {
                self.last_opened_windows = None;
                self.settings_open = true;
            }
            LastOpenedWindowsAction::Cancel { remember } => {
                if remember {
                    self.prefs.session_restore = SessionRestore::Never;
                }
                self.last_opened_windows = None;
            }
            LastOpenedWindowsAction::Restore { kits, remember } => {
                if remember {
                    self.prefs.session_restore = SessionRestore::Always;
                }
                self.last_opened_windows = None;
                self.begin_last_session_restore(kits, ctx.clone());
            }
        }
    }

    pub(super) fn persist_prefs_if_changed(&mut self) {
        let _ = self.try_persist_prefs();
    }

    /// Write prefs if they changed; whether a write failed.
    fn try_persist_prefs(&mut self) -> bool {
        let prefs = self.current_prefs();
        if prefs == self.saved_prefs && self.terminal_open_games == self.saved_terminal_open_games {
            return false;
        }
        match save_gui_prefs(&prefs, &self.terminal_open_games, true) {
            Ok(()) => {
                self.saved_prefs = prefs;
                self.saved_terminal_open_games = self.terminal_open_games.clone();
                false
            }
            Err(error) => {
                self.status = error;
                true
            }
        }
    }

    /// The per-frame prefs check, at most once a second.
    ///
    /// It ran every frame: a full GuiPrefs rebuilt (recents, favorites, kit
    /// profiles and swatches cloned) just to compare, and while a window, the
    /// UI-scale slider or a splitter was being dragged the value changed every
    /// frame, so prefs.json was rewritten at the frame rate. A failed write was
    /// retried, and reported, every frame too. Explicit calls (settings, runtime
    /// poke) still write at once, and exit flushes whatever is pending.
    pub(super) fn persist_prefs_throttled(&mut self, now: f64) {
        const CHECK_INTERVAL: f64 = 1.0;
        const RETRY_AFTER_FAILURE: f64 = 10.0;
        if now < self.prefs_next_check_at {
            return;
        }
        let failed = self.try_persist_prefs();
        self.prefs_next_check_at = now
            + if failed {
                RETRY_AFTER_FAILURE
            } else {
                CHECK_INTERVAL
            };
    }
}

fn close_action_includes_chimp(action: &PendingCloseAction) -> bool {
    matches!(
        action,
        PendingCloseAction::CloseApp | PendingCloseAction::CloseKit(_)
    )
}


#[cfg(test)]
mod save_changes_prompt_tests;

#[cfg(test)]
mod campaign_import_gate_tests;

#[cfg(test)]
mod chimp_surface_undo_tests;

#[cfg(test)]
mod worker_panic_tests;

#[cfg(test)]
pub(in crate::app) mod loose_fixture;

#[cfg(test)]
mod save_close_session_tests;


enum SaveChangesPromptAction {
    None,
    Save(Vec<String>),
    /// Keep the edits in this workspace's Baboon project, ready for Export
    /// Mod, without writing anything into the game's own files.
    StashForMod,
    /// Arm the discard. Only raised on a stashing workspace, where discarding
    /// deletes stashed bytes rather than just dropping an in-memory edit.
    ConfirmDiscard,
    DontSave,
    Cancel,
}

struct DiscardButton {
    label: &'static str,
    width: f32,
    /// Whether this click only arms the discard. False means it deletes.
    arming: bool,
}

/// What the prompt's discard button says and does.
///
/// On a workspace that stashes, this button deletes bytes that persist across
/// sessions — an edit is stashed within a second of being typed, and exporting a
/// mod does not clear it — so it is named for what it does and takes a second,
/// confirming click. A loose kit has no stash to lose and keeps the one-click
/// "Don't Save" every editor has.
fn discard_button(can_stash: bool, confirmed: bool) -> DiscardButton {
    match (can_stash, confirmed) {
        (false, _) => DiscardButton {
            label: "Don't Save",
            width: 78.0,
            arming: false,
        },
        (true, false) => DiscardButton {
            label: "Discard...",
            width: 96.0,
            arming: true,
        },
        (true, true) => DiscardButton {
            label: "Delete Stashed Edits",
            width: 150.0,
            arming: false,
        },
    }
}

fn render_save_changes_prompt(
    ctx: &egui::Context,
    prompt: &mut SaveChangesPrompt,
) -> SaveChangesPromptAction {
    if !prompt.visible {
        return SaveChangesPromptAction::None;
    }

    let mut action = SaveChangesPromptAction::None;
    egui::Window::new("Baboon - Save Changes?")
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(window_width(ctx, 520.0))
        .default_height(window_height(ctx, 260.0, true))
        .show(ctx, |ui| {
            ui.label(
                RichText::new("The following files have been modified. Select the files to save.")
                    .color(text_dark()),
            );
            if prompt.can_stash {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Save overwrites the game's own pak files in place. Stash for Mod keeps \
                         the edits in this workspace's project instead, ready for Export Mod. \
                         Discard throws them away, including the copy the project is holding.",
                    )
                    .small()
                    .color(subtle_dark()),
                );
            }
            ui.add_space(8.0);
            if let Some(error) = prompt.error.as_deref() {
                ui.label(RichText::new(error).color(Color32::from_rgb(180, 48, 40)));
                ui.add_space(6.0);
            }
            // Spelling out what the destructive button costs, and where from.
            // Exporting a mod does not clear these edits — the mod is a copy —
            // so this is the prompt an exporter sees on every exit, and it used
            // to delete the stash on one unlabelled click.
            if prompt.confirm_discard {
                ui.label(
                    RichText::new(match (prompt.stashed, prompt.stash_file.as_deref()) {
                        (0, _) => "Discard these edits? They are not stashed anywhere, so they \
                                   cannot be recovered."
                            .to_owned(),
                        (count, Some(file)) => format!(
                            "Discard deletes the stashed copy of {count} tag(s) from {}. Other \
                             stashed tags in this workspace, and mods you have already exported, \
                             are not affected.",
                            file.display()
                        ),
                        (count, None) => format!(
                            "Discard deletes the stashed copy of {count} tag(s). Mods you have \
                             already exported are not affected."
                        ),
                    })
                    .color(Color32::from_rgb(210, 120, 90)),
                );
                ui.add_space(6.0);
            }
            ScrollArea::both().max_height(150.0).show(ui, |ui| {
                for dirty in &mut prompt.dirty_tags {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut dirty.checked, "");
                        ui.label(RichText::new(&dirty.path).color(text_dark()));
                    });
                }
            });
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("Cancel").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    action = SaveChangesPromptAction::Cancel;
                }
                let DiscardButton {
                    label,
                    width,
                    arming,
                } = discard_button(prompt.can_stash, prompt.confirm_discard);
                if ui
                    .add(egui::Button::new(label).min_size(Vec2::new(width, 24.0)))
                    .clicked()
                {
                    action = if arming {
                        SaveChangesPromptAction::ConfirmDiscard
                    } else {
                        SaveChangesPromptAction::DontSave
                    };
                }
                if prompt.can_stash
                    && ui
                        .add(egui::Button::new("Stash for Mod").min_size(Vec2::new(110.0, 24.0)))
                        .on_hover_text(
                            "Keep these edits in this workspace's Baboon project, ready for \
                             Export Mod. The game's own files are left untouched.",
                        )
                        .clicked()
                {
                    action = SaveChangesPromptAction::StashForMod;
                }
                if ui
                    .add(egui::Button::new("Save").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    let tag_ids = prompt
                        .dirty_tags
                        .iter()
                        .filter(|dirty| dirty.checked)
                        .map(|dirty| dirty.tag_id.clone())
                        .collect();
                    action = SaveChangesPromptAction::Save(tag_ids);
                }
            });
        });
    action
}

enum LastOpenedWindowsAction {
    None,
    OpenSettings,
    Restore {
        /// Each kit to reopen, with the tags checked for it.
        kits: Vec<RestoreKit>,
        /// "Don't ask again" was ticked — remember this as `Always`.
        remember: bool,
    },
    Cancel {
        /// "Don't ask again" was ticked — remember this as `Never`.
        remember: bool,
    },
}

fn last_opened_workspace_heading(
    profile: Option<(&str, &Path)>,
    game: Option<&str>,
    source_path: &Path,
    project_path: Option<&Path>,
) -> (String, Option<String>) {
    if let Some((name, root)) = profile {
        return (name.to_owned(), Some(root.display().to_string()));
    }
    if let Some(project_path) = project_path {
        let name = project_path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .or_else(|| project_path.file_name().and_then(|name| name.to_str()))
            .map(str::to_owned)
            .unwrap_or_else(|| project_path.display().to_string());
        return (name, Some(project_path.display().to_string()));
    }

    let heading = match game {
        Some(game) => game_display_name(game).to_owned(),
        None => source_path.display().to_string(),
    };
    (heading, None)
}

fn render_last_opened_windows_prompt(
    ctx: &egui::Context,
    prompt: Option<&mut LastOpenedWindowsPrompt>,
) -> LastOpenedWindowsAction {
    let Some(prompt) = prompt else {
        return LastOpenedWindowsAction::None;
    };
    if !prompt.visible {
        return LastOpenedWindowsAction::None;
    }

    let mut action = LastOpenedWindowsAction::None;
    egui::Window::new("Last Opened Windows")
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(window_width(ctx, 520.0))
        .default_height(window_height(ctx, 300.0, true))
        .show(ctx, |ui| {
            ui.label(
                RichText::new("These windows were opened the last time you used Baboon.")
                    .color(text_dark()),
            );
            ui.label(RichText::new("Which of these would you like to reopen?").color(text_dark()));
            ui.add_space(8.0);
            ScrollArea::both().max_height(260.0).show(ui, |ui| {
                for (index, kit) in prompt.kits.iter_mut().enumerate() {
                    if index > 0 {
                        ui.add_space(10.0);
                    }
                    let displayed_source_path =
                        kit.profile_root.as_deref().unwrap_or(&kit.source_path);
                    let (heading, project_path) = last_opened_workspace_heading(
                        kit.profile_name.as_deref().zip(kit.profile_root.as_deref()),
                        kit.game.as_deref(),
                        &kit.source_path,
                        kit.project_path.as_deref(),
                    );
                    let heading_response =
                        ui.label(RichText::new(heading).color(text_dark()).strong());
                    if let Some(project_path) = project_path {
                        heading_response.on_hover_text(&project_path);
                        ui.label(RichText::new(project_path).color(subtle_dark()).small());
                    } else {
                        heading_response.on_hover_text(kit.source_path.display().to_string());
                    }
                    if !kit.source_available {
                        ui.label(
                            RichText::new(format!(
                                "Missing source: {}",
                                displayed_source_path.display()
                            ))
                            .color(Color32::from_rgb(180, 48, 40)),
                        );
                    }
                    // Why a workspace is listed with nothing under it: its
                    // session is its stash, which comes back from its own
                    // recovery file rather than from a list of tabs.
                    if kit.entries.is_empty() && kit.has_project {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("Unsaved changes stashed in this workspace")
                                    .color(subtle_dark())
                                    .small(),
                            );
                        });
                    }
                    if !kit.entries.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Tags").color(subtle_dark()).strong());
                        });
                    }
                    for entry in &mut kit.entries {
                        ui.add_enabled_ui(entry.available, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.checkbox(&mut entry.checked, "");
                                let label = if entry.available {
                                    entry.tag.label.clone()
                                } else {
                                    format!("{} (missing)", entry.tag.label)
                                };
                                ui.label(RichText::new(label).color(text_dark()));
                            });
                        });
                    }
                    if !kit.folder_entries.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Folders").color(subtle_dark()).strong());
                        });
                    }
                    for entry in &mut kit.folder_entries {
                        ui.add_enabled_ui(entry.available, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.checkbox(&mut entry.checked, "");
                                let label = if entry.available {
                                    entry.folder.rel_path.display().to_string()
                                } else {
                                    format!("{} (missing source)", entry.folder.rel_path.display())
                                };
                                ui.label(RichText::new(label).color(text_dark()));
                            });
                        });
                    }
                    if !kit.chimp_entries.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Chimp").color(subtle_dark()).strong());
                        });
                    }
                    for entry in &mut kit.chimp_entries {
                        ui.add_enabled_ui(entry.available, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.checkbox(&mut entry.checked, "");
                                let label = if entry.available {
                                    entry.package.clone()
                                } else {
                                    format!("{} (missing source)", entry.package)
                                };
                                ui.label(RichText::new(label).color(text_dark()));
                            });
                        });
                    }
                }
            });
            ui.add_space(6.0);
            ui.checkbox(&mut prompt.dont_ask_again, "Don't ask again")
                .on_hover_text(
                    "Remember this choice: OK always reopens the last session, \
                     Cancel never does. Change it later in File > Settings.",
                );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Options for this window available in")
                        .color(subtle_dark())
                        .small(),
                );
                if ui.link("File > Settings").clicked() {
                    action = LastOpenedWindowsAction::OpenSettings;
                }
            });
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("Cancel").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    action = LastOpenedWindowsAction::Cancel {
                        remember: prompt.dont_ask_again,
                    };
                }
                if ui
                    .add(egui::Button::new("OK").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    action = LastOpenedWindowsAction::Restore {
                        kits: prompt.checked_kits(),
                        remember: prompt.dont_ask_again,
                    };
                }
            });
        });
    action
}

#[cfg(test)]
mod tests;

/// What a TSV paste did, counted from the per-cell outcomes. It used to count
/// the cells it tried, so a paste whose cells all failed to parse still said
/// every one of them was pasted.
fn tsv_paste_summary(
    outcomes: &[FieldEditOutcome],
    applied_rows: usize,
    skipped_rows: usize,
    element_count: usize,
) -> String {
    let failed: Vec<&FieldEditOutcome> = outcomes
        .iter()
        .filter(|outcome| outcome.result.is_err())
        .collect();
    let pasted = outcomes.len() - failed.len();
    let mut summary = if failed.is_empty() {
        format!("Pasted {pasted} cell(s) across {applied_rows} row(s)")
    } else {
        format!(
            "Pasted {pasted} of {} cell(s) across {applied_rows} row(s)",
            outcomes.len()
        )
    };
    if let Some(first) = failed.first() {
        let error = first
            .result
            .as_ref()
            .err()
            .map(String::as_str)
            .unwrap_or("");
        summary.push_str(&format!(
            " — {} failed; first: {} = \"{}\": {error}",
            failed.len(),
            first.path,
            first.input
        ));
    }
    if skipped_rows > 0 {
        summary.push_str(&format!(
            " — {skipped_rows} extra row(s) ignored (block has {element_count} elements; add more first)"
        ));
    }
    summary
}

#[cfg(test)]
mod listing_entries_tests;

#[cfg(test)]
mod tsv_paste_summary_tests;

#[cfg(test)]
mod container_dependency_tests;

/// How the close prompt's Save writes one tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClosePromptSave {
    /// A brand-new container tag: a new override container, via a dialog.
    NewContainer,
    /// A mounted container tag: overwritten inside its own pak.
    ContainerInPlace,
    /// Everything else is a file on disk, or has none and is refused there.
    File,
}

/// A container tag has no file to write, so sending one down the file path
/// would fail, or worse, write the payload somewhere it does not belong.
fn close_prompt_save_route(location: Option<&TagEntryLocation>) -> ClosePromptSave {
    match location {
        Some(TagEntryLocation::NewContainer { .. }) => ClosePromptSave::NewContainer,
        Some(TagEntryLocation::Container { .. }) => ClosePromptSave::ContainerInPlace,
        Some(TagEntryLocation::LooseFile(_) | TagEntryLocation::Monolithic { .. }) | None => {
            ClosePromptSave::File
        }
    }
}

pub(in crate::app) fn same_entry_key(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a.eq_ignore_ascii_case(b)
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}


#[cfg(test)]
mod tsv_paste_tests;

/// Retire one restored kit's load and report the kit that should take the focus
/// — `None` while any restore is still outstanding, or when the session named
/// no kit and there is nothing to honour.
///
/// Split out from [`Baboon::settle_restored_kit`] because it is the whole
/// decision: the app half only turns the answer into an index.
fn focus_after_restore(
    restoring: &mut HashSet<KitId>,
    restored_active: &mut Option<KitId>,
    settled: KitId,
) -> Option<KitId> {
    // A load that was not part of the restore settles nothing, and neither does
    // one that still leaves others in flight.
    if !restoring.remove(&settled) || !restoring.is_empty() {
        return None;
    }
    restored_active.take()
}

#[cfg(test)]
mod restore_focus_tests;

#[cfg(test)]
mod field_search_tests;

#[cfg(test)]
mod saved_tag_index_tests;


#[cfg(test)]
mod prefs_throttle_tests;

