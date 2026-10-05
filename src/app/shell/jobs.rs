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
                    if self.model.resolve_stamp(stamp).is_some() && request_id == self.search.find.all_request_id
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
                    if self.model.resolve_stamp(stamp).is_some() && request_id == self.search.find.all_request_id
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
mod worker_panic_tests {
    //! A background job that panics must still settle what the UI marked as in
    //! flight. Each test makes the job panic before it does any work (see
    //! `with_panicking_workers`) and checks the state it would have left stuck
    //! when it was a bare `thread::spawn`, which sent nothing.

    use super::*;
    use crate::app::import::TagImportDialog;
    use crate::app::chimp::ChimpMount;

    /// Every loader reserves the kit for the path it is loading; the reservation
    /// is what reads as "starting up" and what a second open of the same path
    /// switches to. A loader that panicked used to keep it forever.
    #[test]
    fn a_source_load_that_panics_releases_its_kit() {
        let folder = crate::core::test_kits::unique_temp_dir("panicking-load");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("rifle.weapon");
        type Begin = fn(&mut Baboon, PathBuf, egui::Context);
        let loaders: [(&str, Begin, PathBuf); 5] = [
            ("single tag", Baboon::begin_load_single_path, file.clone()),
            ("folder", Baboon::begin_load_folder_path, folder.clone()),
            ("monolithic cache", Baboon::begin_load_monolithic_path, folder.join("blob_index.dat")),
            ("container", Baboon::begin_load_iostore_container_path, folder.join("a.utoc")),
            ("container set", |app, path, ctx| {
                app.begin_load_iostore_container_set_path(path.clone(), path, ctx)
            }, folder.join("Paks")),
        ];
        for (loader, begin, path) in loaders {
            let mut app = Baboon::for_test();
            let ctx = egui::Context::default();
            with_panicking_workers(|| begin(&mut app, path.clone(), ctx.clone()));
            assert!(app.model.kits[0].requested_path.is_some(), "{loader}: reserved");

            assert!(apply_next_worker_message(&mut app), "{loader}: the load answered");
            assert_eq!(app.model.kits[0].requested_path, None, "{loader}: the kit is free again");
            assert!(app.model.status.contains("crashed"), "{loader}: {}", app.model.status);
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    fn campaign_evolved_source(root: &Path) -> LoadedSourceData {
        LoadedSourceData {
            label: "Campaign Evolved".to_owned(),
            source: TagSource::IoStoreContainerSet {
                root: root.to_path_buf(),
                containers: Vec::new(),
                index: Default::default(),
                packages: Default::default(),
                shipped: Default::default(),
            },
            names: TagNameIndex::default(),
            game: Some(GameId::CampaignEvolved),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        }
    }

    /// A mount still `Loading` refuses every container write, so a mount that
    /// panicked locked the containers for the session.
    #[test]
    fn a_chimp_mount_that_panics_does_not_stay_loading() {
        let mut app = Baboon::for_test();
        app.model.prefs.enable_chimp = true;
        app.install_loaded_source(campaign_evolved_source(Path::new("/no/such/Paks")));
        let ctx = egui::Context::default();
        with_panicking_workers(|| app.begin_chimp_mount(0, ctx.clone()));
        assert!(matches!(app.model.kits[0].chimp.mount, ChimpMount::Loading));

        assert!(apply_next_worker_message(&mut app), "the mount answered");
        assert!(
            matches!(app.model.kits[0].chimp.mount, ChimpMount::Failed(_)),
            "the mount settled as failed"
        );
    }

    fn loose_kit(app: &mut Baboon, root: &Path) {
        app.install_loaded_source(LoadedSourceData {
            label: "kit".to_owned(),
            source: TagSource::LooseFolder {
                root: root.to_path_buf(),
                game: Some(GameId::Halo3),
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: Some(GameId::Halo3),
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
    }

    /// A building index is never started again, so a build that panicked left
    /// field-value search without its index for the session.
    #[test]
    fn a_field_index_build_that_panics_stops_building() {
        let root = crate::core::test_kits::unique_temp_dir("panicking-field-index");
        let mut app = Baboon::for_test();
        loose_kit(&mut app, &root);
        let ctx = egui::Context::default();
        with_panicking_workers(|| app.begin_build_field_index(ctx.clone()));
        assert!(app.model.kits[0].field_index.is_building());

        assert!(apply_next_worker_message(&mut app), "the build answered");
        assert!(!app.model.kits[0].field_index.is_building());
        assert!(app.model.status.contains("crashed"), "{}", app.model.status);
    }

    /// While it resolves its source the import dialog spins and repaints every
    /// frame; a resolve that panicked left it doing so for good.
    #[test]
    fn an_import_source_check_that_panics_stops_spinning() {
        let root = crate::core::test_kits::unique_temp_dir("panicking-import");
        std::fs::create_dir_all(root.join("tags")).unwrap();
        let mut app = Baboon::for_test();
        loose_kit(&mut app, &root.join("tags"));
        app.open_tag_import_dialog(None);
        let dialog = app
            .dialogs
            .get_mut::<TagImportDialog>()
            .expect("the dialog opened");
        dialog.source_input = root.join("elsewhere").display().to_string();
        let ctx = egui::Context::default();
        with_panicking_workers(|| app.resolve_import_source(&ctx));
        assert!(app.dialogs.get::<TagImportDialog>().unwrap().resolving);

        assert!(apply_next_worker_message(&mut app), "the check answered");
        let dialog = app.dialogs.get::<TagImportDialog>().unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert!(!dialog.resolving, "no longer spinning");
        assert!(dialog.error.as_deref().is_some_and(|error| error.contains("crashed")));
    }

    /// A palette table still `Loading` gates nothing and is never asked for
    /// again, so a read that panicked left Sapien drops ungated for the session.
    #[test]
    fn a_palette_read_that_panics_is_unreadable_not_loading() {
        let mut app = Baboon::for_test();
        let ctx = egui::Context::default();
        with_panicking_workers(|| app.scenario_palettes_for_game(GameId::Halo3, &ctx).is_none());
        assert!(matches!(
            app.kit_tools.kit_tool_drag.palettes.get(&GameId::Halo3),
            Some(PaletteTable::Loading)
        ));

        assert!(apply_next_worker_message(&mut app), "the read answered");
        assert!(matches!(
            app.kit_tools.kit_tool_drag.palettes.get(&GameId::Halo3),
            Some(PaletteTable::Unreadable)
        ));
    }
}
