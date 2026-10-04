use super::*;

#[test]
fn chimp_is_idle_and_unfiltered_by_default() {
    let state = ChimpState::default();
    assert!(matches!(state.mount, ChimpMount::Idle));
    assert_eq!(state.browser, ChimpBrowser::Folders);
    assert!(state.filter.is_empty());
    assert!(state.open_packages.is_empty());
    assert!(
        !state.filter_is_current(""),
        "the initial empty query must populate the browser once"
    );
}

#[test]
fn chimp_browser_tabs_follow_the_asset_browsing_order() {
    assert_eq!(
        ChimpBrowser::TABS,
        [
            (ChimpBrowser::Folders, "Folders"),
            (ChimpBrowser::Groups, "Groups"),
            (ChimpBrowser::Files, "Pak files"),
            (ChimpBrowser::Archives, "Archives"),
            (ChimpBrowser::Packages, "Packages"),
        ]
    );
}

#[test]
fn campaign_evolved_surface_tabs_put_tags_before_chimp() {
    assert_eq!(KitSurface::TABS[0].0, KitSurface::Tags);
    assert_eq!(KitSurface::TABS[0].1, "Tags");
    assert_eq!(KitSurface::TABS[1].0, KitSurface::Chimp);
    assert_eq!(KitSurface::TABS[1].1, "Chimp");
}

#[test]
fn chimp_document_tree_tracks_open_close_and_selection() {
    let mut state = ChimpState::default();
    let kit = KitId(7);
    state.open_document_pane(kit, "/Game/Textures/A");
    state.open_document_pane(kit, "/Game/Textures/B");
    assert_eq!(state.open_packages.len(), 2);
    assert_eq!(state.selected_package.as_deref(), Some("/Game/Textures/B"));
    assert!(
        state
            .document_tree
            .as_ref()
            .is_some_and(|tree| !tree.is_empty())
    );

    state.close_document_pane("/Game/Textures/B");
    assert_eq!(state.open_packages, ["/Game/Textures/A"]);
    assert_eq!(state.selected_package.as_deref(), Some("/Game/Textures/A"));
    state.close_document_pane("/Game/Textures/A");
    assert!(state.open_packages.is_empty());
    assert!(state.selected_package.is_none());
}

#[test]
fn package_tree_groups_every_path_segment_and_counts_descendants() {
    let mut tree = ChimpFolderNode::default();
    tree.insert_package(0, "/Game/UI/Menu");
    tree.insert_package(1, "/Game/UI/Hud");
    tree.insert_package(2, "/Engine/Config");
    tree.insert_file(0, "../../../Meteorite/Content/Audio/menu.bnk");
    assert_eq!(tree.package_count, 3);
    assert_eq!(tree.file_count, 1);
    assert_eq!(tree.entry_count(), 4);
    let game = tree.folders.get("Game").unwrap();
    assert_eq!(game.package_count, 2);
    let ui = game.folders.get("UI").unwrap();
    assert_eq!(ui.package_count, 2);
    assert_eq!(
        ui.packages
            .iter()
            .map(|leaf| leaf.name.as_str())
            .collect::<Vec<_>>(),
        ["Menu", "Hud"]
    );
    let engine = &tree.folders["Engine"];
    assert_eq!(engine.package_count, 1);
    assert_eq!(engine.packages[0].name, "Config");
    assert_eq!(
        tree.folders["Meteorite"].folders["Content"].folders["Audio"].files[0].name,
        "menu.bnk"
    );
}
