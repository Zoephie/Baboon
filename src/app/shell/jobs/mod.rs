//! Applying what background work sends back: each frame drains the worker
//! channel and hands every message to the feature it belongs to.

use super::*;

impl Baboon {
    pub(in crate::app) fn process_worker_messages(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.rx.try_recv() {
            self.apply_worker_message(message, ctx);
        }
    }

    /// Apply one worker result. Handlers drop a result whose source is stale
    /// themselves; what they return about it is not used here.
    pub(in crate::app) fn apply_worker_message(&mut self, message: WorkerMessage, ctx: &egui::Context) {
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
                    self.kit_tools.kit_tool_drag.palettes.insert(game, table);
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
                    if self.resolve_stamp(stamp).is_some() && request_id == self.search.find.all_request_id
                    {
                        self.search.find.progress = Some((processed, total));
                    }
                    false
                }
                WorkerMessage::FindAllFinished {
                    stamp,
                    request_id,
                    occurrences,
                    unreadable,
                } => {
                    if self.resolve_stamp(stamp).is_some() && request_id == self.search.find.all_request_id
                    {
                        self.search.find.all_closed_occurrences = occurrences;
                        self.search.find.unreadable = unreadable;
                        self.search.find.searching = false;
                        self.search.find.progress = None;
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
}

#[cfg(test)]
mod worker_panic_tests;
