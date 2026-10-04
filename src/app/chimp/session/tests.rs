use super::*;

/// Edits schedule one recovery checkpoint for when they pause, instead of
/// a full rebuild and write per keystroke.
#[test]
fn a_chimp_checkpoint_waits_for_edits_to_pause() {
    let mut app = Baboon::for_test();
    let mut document = rename_fixture();
    document.checkpoint_due = Some(5.0);
    app.model.kits[0]
        .chimp
        .documents
        .insert("/Game/Test/Thing".to_owned(), document);
    let ctx = egui::Context::default();
    let due_at = |time: f64, app: &mut Baboon| {
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |_| app.run_due_chimp_checkpoints(0, &ctx),
        );
        app.model.kits[0].chimp.documents["/Game/Test/Thing"].checkpoint_due
    };

    assert_eq!(
        due_at(4.0, &mut app),
        Some(5.0),
        "still editing: nothing yet"
    );
    assert_eq!(due_at(6.0, &mut app), None, "paused: checkpointed once");
}

/// While the window is minimized eframe runs `App::logic` and no UI pass,
/// and egui's clock stays at the last frame shown. A checkpoint that falls
/// due then is still written, on the clock eframe stamps on the input.
#[test]
fn a_checkpoint_falls_due_while_the_window_is_hidden() {
    let mut app = Baboon::for_test();
    let mut document = rename_fixture();
    document.checkpoint_due = Some(5.0);
    app.model.kits[0]
        .chimp
        .documents
        .insert("/Game/Test/Thing".to_owned(), document);
    let ctx = egui::Context::default();
    let shown = egui::RawInput {
        time: Some(1.0),
        ..Default::default()
    };
    let _ = crate::app::run_ui_test(&ctx, shown, |_| {});
    let mut hidden = egui::RawInput {
        time: Some(6.0),
        ..Default::default()
    };
    hidden
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .minimized = Some(true);
    let due = |app: &Baboon| app.model.kits[0].chimp.documents["/Game/Test/Thing"].checkpoint_due;

    // egui alone still reads 1.0, so nothing is due yet.
    let _ = ctx.run_logic(&hidden, |ctx| app.run_logic(ctx));
    assert_eq!(due(&app), Some(5.0), "egui's clock does not move while hidden");

    eframe::App::raw_input_hook(&mut app, &ctx, &mut hidden);
    let _ = ctx.run_logic(&hidden, |ctx| app.run_logic(ctx));
    assert_eq!(due(&app), None, "checkpointed on eframe's clock");
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
    app.model.kits[0]
        .chimp
        .documents
        .insert("/Game/Test/Thing".to_owned(), open);
    let manifest = HashMap::from([
        ("/Game/Test/Thing".to_owned(), "aaaa.uasset".to_owned()),
        ("/Game/Test/Closed".to_owned(), "bbbb.uasset".to_owned()),
    ]);

    let restoring = chimp_recovery_still_closed(&app.model.kits[0].chimp, manifest.clone());
    assert_eq!(
        restoring,
        [("/Game/Test/Closed".to_owned(), "bbbb.uasset".to_owned())]
    );

    // With nothing open, both come back.
    app.model.kits[0].chimp.documents.clear();
    assert_eq!(
        chimp_recovery_still_closed(&app.model.kits[0].chimp, manifest).len(),
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
        kit: app.model.kits[0].id,
        generation: app.model.kits[0].generation,
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
    let document = app.model.kits[0].chimp.documents.get_mut(package).unwrap();
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
    assert!(app.model.kits[0].chimp.loading_packages.contains(THING));
    assert_eq!(app.model.kits[0].chimp.selected_package.as_deref(), Some(THING));
    assert_eq!(app.chimp_activity(0), "loading a package");

    assert!(apply_next_worker_message(&mut app));
    let chimp = &app.model.kits[0].chimp;
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
    assert!(app.model.kits[0].chimp.loading_packages.is_empty());
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
    assert!(app.model.kits[0].chimp.loading_packages.is_empty());
    assert!(app.model.kits[0].chimp.documents.is_empty());
    assert_eq!(app.model.status, "/Game/Test/Missing is not mounted");
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
    let _ = crate::app::run_ui_test(
        &ctx,
        egui::RawInput {
            time: Some(1.0),
            ..Default::default()
        },
        |ui| app.run_due_chimp_checkpoints(0, ui.ctx()),
    );
    assert_eq!(app.model.kits[0].chimp.documents[THING].checkpoint_due, None);

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
    let document = &app.model.kits[0].chimp.documents[THING];
    assert_eq!(
        written,
        rebuild_chimp_document(&install.world, document).unwrap().0,
        "the checkpoint is the rebuilt package"
    );
    let reread =
        decode_chimp_document(&install.world, document.provider.clone(), written).unwrap();
    assert_eq!(int(&reread, "Count"), 42);
    assert_eq!(app.model.status, "Ready", "a checkpoint that worked says nothing");

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
    app.model.kits[0].chimp.mount = ChimpMount::Idle;
    app.flush_all_chimp_checkpoints();
    assert_eq!(app.model.kits[0].chimp.documents[THING].checkpoint_due, None);
    assert!(!app.chimp_recovery_dir(0).unwrap().exists());
    assert_eq!(app.model.status, "Ready");
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
    app.model.kits[0].source = Some(install.source());
    let ctx = egui::Context::default();
    app.handle_chimp_mounted(stamp(&app), Ok(install.world.clone()), ctx.clone());
    assert!(matches!(app.model.kits[0].chimp.mount, ChimpMount::Ready(_)));
    assert!(app.model.kits[0].chimp.type_indexing);
    assert_eq!(app.chimp_activity(0), "indexing package types");
    assert_eq!(app.model.status, "Chimp recovered 1 unsaved package edit(s)");
    let chimp = &app.model.kits[0].chimp;
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
    app.model.kits[0]
        .chimp
        .documents
        .get_mut(THING)
        .unwrap()
        .checkpoint_due = None;
    app.handle_chimp_mounted(stamp(&app), Ok(install.world.clone()), ctx);
    assert_eq!(int(&app.model.kits[0].chimp.documents[THING], "Count"), 43);
    assert_eq!(
        app.model.status, "Chimp indexed 2 Unreal packages and 0 pak files",
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
    app.model.kits[0].source = Some(install.source());
    app.handle_chimp_mounted(
        stamp(&app),
        Ok(install.world.clone()),
        egui::Context::default(),
    );
    assert!(app.model.kits[0].chimp.documents.is_empty());
    assert_eq!(
        app.model.status,
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
    app.model.kits[0].source = Some(install.source());
    app.handle_chimp_mounted(
        stamp(&app),
        Err("no containers".to_owned()),
        egui::Context::default(),
    );
    assert!(matches!(&app.model.kits[0].chimp.mount, ChimpMount::Failed(error) if error == "no containers"));
    assert_eq!(app.model.status, "Chimp could not open: no containers");
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
    app.model.kits[0]
        .chimp
        .documents
        .insert("/Game/Test/Gone".to_owned(), stray);
    app.model.kits[0]
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
    let documents = &app.model.kits[0].chimp.documents;
    assert!(!documents[THING].orphaned);
    assert_eq!(documents[THING].provider.container, 0);
    assert!(documents["/Game/Test/Gone"].orphaned);
    assert_eq!(
        app.model.status,
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
    app.model.kits[0].source = Some(install.source());
    app.model.kits[0].restore.pending_restore_chimp_packages = vec![
        THING.to_owned(),
        "/Game/Test/Missing".to_owned(),
        OTHER.to_owned(),
    ];
    app.model.kits[0].restore.pending_restore_active_chimp_package = Some(THING.to_owned());
    app.handle_chimp_mounted(
        stamp(&app),
        Ok(install.world.clone()),
        egui::Context::default(),
    );
    assert_eq!(
        app.model.status,
        "Reopening 2 Chimp package(s); 1 saved package(s) are missing"
    );
    assert_eq!(app.model.kits[0].chimp.loading_packages.len(), 2);
    apply_until(&mut app, |app| app.model.kits[0].chimp.loading_packages.is_empty());
    let chimp = &app.model.kits[0].chimp;
    assert!(chimp.loading_packages.is_empty());
    assert_eq!(chimp.documents.len(), 2);
    let mut open = chimp.open_packages.clone();
    open.sort();
    assert_eq!(open, [OTHER, THING]);
    assert_eq!(chimp.selected_package.as_deref(), Some(THING));
    assert!(app.model.kits[0].restore.pending_restore_active_chimp_package.is_none());
    assert!(app.model.kits[0].restore.pending_restore_chimp_packages.is_empty());
}

/// Changing the USMAP is refused while anything is modified, and
/// otherwise remounts the workspace from scratch on a worker: the mount,
/// then the package-type index.
#[test]
fn changing_the_usmap_remounts_unless_something_is_modified() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[THING]);
    app.model.prefs.enable_chimp = true;
    let ctx = egui::Context::default();
    edit_count(&mut app, THING, 42);
    app.apply_chimp_usmap_path(None, ctx.clone());
    assert_eq!(
        app.model.status,
        "Build or discard modified Chimp packages before changing the USMAP."
    );
    assert!(app.model.kits[0].chimp.documents.contains_key(THING));
    assert!(matches!(app.model.kits[0].chimp.mount, ChimpMount::Ready(_)));

    app.model.kits[0].chimp.documents.get_mut(THING).unwrap().dirty = false;
    app.apply_chimp_usmap_path(None, ctx);
    assert_eq!(
        app.model.status,
        "Using the bundled Campaign Evolved USMAP; remounting Chimp"
    );
    assert!(
        app.model.kits[0].chimp.documents.is_empty(),
        "a remount starts the workspace over"
    );
    assert!(matches!(app.model.kits[0].chimp.mount, ChimpMount::Loading));
    assert_eq!(app.chimp_activity(0), "still mounting");

    // The mount and the type index behind it may land in one frame.
    apply_until(&mut app, |app| {
        !matches!(app.model.kits[0].chimp.mount, ChimpMount::Loading) && !app.model.kits[0].chimp.type_indexing
    });
    if let ChimpMount::Failed(error) = &app.model.kits[0].chimp.mount {
        panic!("the remount failed: {error}");
    }
    let ChimpMount::Ready(world) = &app.model.kits[0].chimp.mount else {
        unreachable!()
    };
    assert_eq!(world.packages().len(), 2);
    let chimp = &app.model.kits[0].chimp;
    assert!(!chimp.type_indexing);
    // The synthetic class is a package import, which the type index does
    // not classify.
    assert_eq!(chimp.package_types, [None, None]);
    assert_eq!(
        app.model.status,
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
    app.model.kits[0]
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
    let document = &app.model.kits[0].chimp.documents[THING];
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
    let kit = app.model.kits[0].id;
    app.chimp.chimp_writes.insert(kit, None);
    assert_eq!(
        app.discard_chimp_packages(0, &[THING.to_owned()]),
        Err("A Chimp save is still running; discard once it finishes".to_owned())
    );
    app.chimp.chimp_writes.clear();
    app.model.kits[0].chimp.mount = ChimpMount::Idle;
    assert_eq!(
        app.discard_chimp_packages(0, &[THING.to_owned()]),
        Err("Chimp is not mounted; the original package data is unavailable".to_owned())
    );
    assert_eq!(int(&app.model.kits[0].chimp.documents[THING], "Count"), 42);
    assert!(app.model.kits[0].chimp.documents[THING].dirty);
}

/// A modified package refuses to close; a clean one closes, taking its
/// document and pane with it.
#[test]
fn a_modified_package_refuses_to_close() {
    let install = SyntheticInstall::new();
    let mut app = install.app_with_open(&[THING, OTHER]);
    edit_count(&mut app, THING, 42);
    assert!(!app.close_chimp_package(0, THING));
    assert!(app.model.kits[0].chimp.documents.contains_key(THING));
    assert!(app.close_chimp_package(0, OTHER));
    let chimp = &app.model.kits[0].chimp;
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
    assert!(app.chimp.chimp_discard_prompt.is_none());
    assert_eq!(app.model.status, "Chimp has no modified packages");
    app.open_chimp_discard_prompt(0, vec![THING.to_owned()], None, None);
    let prompt = app.chimp.chimp_discard_prompt.as_ref().unwrap();
    assert_eq!(prompt.kit, app.model.kits[0].id);
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
        app.model.kits[0].chimp.documents[OTHER].referrers,
        ChimpReferrerState::Scanning
    ));
    assert!(apply_next_worker_message(&mut app));
    let ChimpReferrerState::Done(scan) = &app.model.kits[0].chimp.documents[OTHER].referrers else {
        panic!("the scan settled");
    };
    assert_eq!(scan.referrers, [THING]);
    assert_eq!((scan.scanned, scan.unreadable), (1, 0));

    app.begin_chimp_referrer_scan(0, THING.to_owned(), egui::Context::default());
    assert!(apply_next_worker_message(&mut app));
    let ChimpReferrerState::Done(scan) = &app.model.kits[0].chimp.documents[THING].referrers else {
        panic!("the scan settled");
    };
    assert!(scan.referrers.is_empty());
    assert_eq!(scan.scanned, 1);
}
