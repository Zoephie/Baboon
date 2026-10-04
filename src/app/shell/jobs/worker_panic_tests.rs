//! A background job that panics must still settle what the UI marked as in
//! flight. Each test makes the job panic before it does any work (see
//! `with_panicking_workers`) and checks the state it would have left stuck
//! when it was a bare `thread::spawn`, which sent nothing.

use super::*;
use crate::app::chimp::ChimpMount;

/// Every loader reserves the kit for the path it is loading; the reservation
/// is what reads as "starting up" and what a second open of the same path
/// switches to. A loader that panicked used to keep it forever.
#[test]
fn a_source_load_that_panics_releases_its_kit() {
    let folder = crate::test_kits::unique_temp_dir("panicking-load");
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
    let root = crate::test_kits::unique_temp_dir("panicking-field-index");
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
    let root = crate::test_kits::unique_temp_dir("panicking-import");
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
