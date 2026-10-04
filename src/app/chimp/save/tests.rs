use super::*;

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
        chimp_staging_utoc(&output),
        Vec::new(),
        Err("stopped".to_owned()),
        &ctx,
    );
    assert!(app.chimp.chimp_writes.is_empty());
    assert!(app.model.kit_index(kit).is_none(), "and runs once it lands");
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
        app.dialogs.draw(&cx!(app, &ctx));
        app.apply_commands(&ctx);
    }
}

/// Draw the open dialogs and apply what they sent, as a frame does.
fn draw_discard(app: &mut Baboon) -> impl FnMut(&mut egui::Ui) + '_ {
    move |ui| {
        let ctx = ui.ctx().clone();
        app.dialogs.draw(&cx!(app, &ctx));
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

    assert!(apply_next_worker_message(&mut app), "the overwrite answered");
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
