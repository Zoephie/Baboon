use super::*;

fn empty_source() -> LoadedSourceData {
    LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: PathBuf::from("/nonexistent-baboon-view-test"),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names: TagNameIndex::default(),
        game: None,
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

/// Every open kit has a view and every view belongs to an open kit.
fn assert_in_step(app: &Baboon) {
    for kit in &app.model.kits {
        assert!(app.views.views.contains_key(&kit.id), "{:?} has no view", kit.id);
    }
    assert_eq!(app.views.views.len(), app.model.kits.len(), "a closed kit kept its view");
}

#[test]
fn adding_and_closing_kits_keeps_their_views_in_step() {
    let mut app = Baboon::for_test();
    assert_in_step(&app);
    let first = app.model.kits[0].id;
    let second = app.add_kit();
    assert_in_step(&app);
    app.remove_kit(first);
    assert_in_step(&app);
    assert!(!app.views.views.contains_key(&first));
    // Closing the last kit leaves a fresh empty one, with its own view.
    app.remove_kit(second);
    assert_in_step(&app);
    assert_ne!(app.model.kits[0].id, second);
}

/// A new kit's browser opens in the view the prefs say, as before the split.
#[test]
fn a_new_kit_opens_its_browser_as_the_prefs_say() {
    let mut app = Baboon::for_test();
    app.model.prefs.browser_mode = BrowserMode::Groups;
    app.model.prefs.browser_sort = BrowserSort::Type;
    let id = app.add_kit();
    assert_eq!(app.views[id].browser.mode, BrowserMode::Groups);
    assert_eq!(app.views[id].browser.sort, BrowserSort::Type);
}

/// Loading a source into a kit starts its view over but keeps how its browser
/// lists tags, which belongs to the workspace rather than the source.
#[test]
fn a_reload_starts_the_view_over_but_keeps_the_browser_mode() {
    let mut app = Baboon::for_test();
    let id = app.model.kits[0].id;
    app.views[id].browser.mode = BrowserMode::Groups;
    app.views[id].browser.filter = "warthog".to_owned();
    app.views[id].pending_expand.insert("tag".to_owned(), true);
    app.install_loaded_source(empty_source());
    assert_in_step(&app);
    assert_eq!(app.views[id].browser.mode, BrowserMode::Groups);
    assert!(app.views[id].browser.filter.is_empty());
    assert!(app.views[id].pending_expand.is_empty());
}
