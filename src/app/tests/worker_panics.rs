//! A background job that panics must still settle what the UI marked as in
//! flight. Each test makes the job panic before it does any work (see
//! `with_panicking_workers`) and checks the state it would have left stuck
//! when it was a bare `thread::spawn`, which sent nothing.

use super::*;

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
        assert!(app.kits[0].requested_path.is_some(), "{loader}: reserved");

        assert!(apply_next_worker_message(&mut app), "{loader}: the load answered");
        assert_eq!(app.kits[0].requested_path, None, "{loader}: the kit is free again");
        assert!(app.status.contains("crashed"), "{loader}: {}", app.status);
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
        game: Some("haloce_evolved".to_owned()),
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
    app.prefs.enable_chimp = true;
    app.install_loaded_source(campaign_evolved_source(Path::new("/no/such/Paks")));
    let ctx = egui::Context::default();
    with_panicking_workers(|| app.begin_chimp_mount(0, ctx.clone()));
    assert!(matches!(app.kits[0].chimp.mount, ChimpMount::Loading));

    assert!(apply_next_worker_message(&mut app), "the mount answered");
    assert!(
        matches!(app.kits[0].chimp.mount, ChimpMount::Failed(_)),
        "the mount settled as failed"
    );
}
