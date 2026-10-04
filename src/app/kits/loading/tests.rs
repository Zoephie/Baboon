use super::*;

/// A refresh that saw a definition or option change, appear or go asks
/// the shader grid to re-read them; one that saw only other tags does not.
#[test]
fn a_refresh_notices_render_method_changes() {
    let entry = |group: &[u8; 4], key: &str| TagEntry {
        key: key.to_owned(),
        display_path: key.to_owned(),
        group_tag: u32::from_be_bytes(*group),
        group_name: None,
        location: TagEntryLocation::LooseFile(PathBuf::from(key)),
    };
    let refresh = |touched: Vec<TagEntry>, removed: Vec<&str>| EntryIndexRefresh {
        entries: Vec::new(),
        changed: true,
        added: 0,
        updated: 0,
        removed: 0,
        touched,
        removed_keys: removed.into_iter().map(str::to_owned).collect(),
        touched_dependencies: Vec::new(),
        errors: Vec::new(),
    };
    assert!(!refresh_touches_render_methods(&refresh(
        vec![entry(b"hlmt", "file:a.model")],
        vec!["file:b.weapon"],
    )));
    assert!(refresh_touches_render_methods(&refresh(
        vec![entry(b"rmdf", "file:shaders/shader.render_method_definition")],
        Vec::new(),
    )));
    assert!(refresh_touches_render_methods(&refresh(
        Vec::new(),
        vec!["file:shaders/bump.render_method_option"],
    )));
}

#[test]
fn campaign_evolved_projects_open_on_tags() {
    assert_eq!(campaign_evolved_surface_on_load(), KitSurface::Tags);
}

fn loose_kit_at(tags: &Path) -> Baboon {
    loose_kit_with(tags, "halo3_mcc", None)
}

fn loose_kit_with(tags: &Path, game: &str, chosen: Option<KitLayout>) -> Baboon {
    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: tags.to_path_buf(),
            game: GameId::from_id(game),
            definitions_root: PathBuf::new(),
        },
        names: TagNameIndex::default(),
        game: GameId::from_id(game),
        entries: Vec::new(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: chosen,
    });
    app.apply_loaded_source_identity(GameId::from_id(game));
    app
}

/// A kit whose profile chose its folders uses exactly those: its data
/// folder is the chosen one, not the root's `data`, and its Halo CE tools
/// are told where both are.
#[test]
fn a_kit_with_chosen_folders_uses_them_everywhere() {
    let ek = PathBuf::from("/ek/HCEEK");
    let chosen = KitLayout {
        root: ek.clone(),
        tags: ek.join("tags_moda"),
        data: ek.join("data_moda"),
    };
    let app = loose_kit_with(&chosen.tags, "haloce_mcc", Some(chosen.clone()));
    assert_eq!(app.kit_layout_for(0), Some(chosen.clone()));
    assert_eq!(app.loaded_data_root(), Some(ek.join("data_moda")));
    assert_eq!(app.kits[0].terminal_work_dir, Some(ek.clone()));
    assert_eq!(
        app.active_kit_tool_folder_options(),
        vec![
            ("-tags_dir", ek.join("tags_moda")),
            ("-data_dir", ek.join("data_moda")),
        ]
    );
    // The same folders opened as a Halo 3 kit get no options: its tools
    // can't take them.
    let halo3 = loose_kit_with(&chosen.tags.clone(), "halo3_mcc", Some(chosen));
    assert!(halo3.active_kit_tool_folder_options().is_empty());
}

/// Opening a profile's chosen tags folder as a folder opens the profile,
/// so its data folder and tool options come with it.
#[test]
fn a_chosen_tags_folder_belongs_to_its_profile() {
    let outer = crate::test_kits::unique_temp_dir("chosen-tags-profile");
    let root = outer.join("H2EK");
    for folder in ["tags", "data", "tags_moda", "data_moda"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    let mut app = Baboon::for_test();
    let profile = |id: &str, tags: Option<&str>| CustomEditingKitProfile {
        read_only: false,
        git_tracked: false,
        id: id.to_owned(),
        name: id.to_owned(),
        game: "halo2_mcc".to_owned(),
        root: root.clone(),
        icon: None,
        tags_folder: tags.map(PathBuf::from),
        data_folder: tags.map(|_| PathBuf::from("data_moda")),
    };
    app.prefs.custom_editing_kit_profiles =
        vec![profile("stock", None), profile("moda", Some("tags_moda"))];
    app.refresh_editing_kit_validation();
    let moda = app.profile_using_chosen_tags_folder(&root.join("tags_moda"));
    let stock = app.profile_using_chosen_tags_folder(&root.join("tags"));
    let _ = std::fs::remove_dir_all(&outer);
    assert_eq!(moda.map(|profile| profile.id).as_deref(), Some("moda"));
    // The stock kit opens as a folder, as it always has.
    assert_eq!(stock, None);
}

/// Every way the app asks where a loaded kit's root and data folder are
/// answers from the one layout, so they cannot drift apart again.
#[test]
fn a_loaded_kits_folders_all_come_from_its_layout() {
    let ek = PathBuf::from("/ek/H3EK");
    let app = loose_kit_at(&ek.join("tags"));
    assert_eq!(app.editing_kit_root(), Some(ek.clone()));
    assert_eq!(app.loaded_data_root(), Some(ek.join("data")));
    assert_eq!(app.kits[0].terminal_work_dir, Some(ek.clone()));
    assert_eq!(app.kit_tool_path("sapien.exe"), Some(ek.join("sapien.exe")));
}

/// A loose folder with another name used to be its own kit root for tool
/// launches and Open Data Folder, while the terminal and sound extraction
/// used its parent. Now all of them use the parent.
#[test]
fn a_folder_not_named_tags_has_its_parent_for_a_root_everywhere() {
    let app = loose_kit_at(Path::new("/ek/H3EK/tags_moda"));
    let ek = PathBuf::from("/ek/H3EK");
    assert_eq!(app.editing_kit_root(), Some(ek.clone()));
    assert_eq!(app.loaded_data_root(), Some(ek.join("data")));
    assert_eq!(app.kits[0].terminal_work_dir, Some(ek));
}

/// The read-only check now starts from the tags folder, which is under the
/// kit root, so a read-only profile still covers the kit it names.
#[test]
fn a_read_only_profile_still_covers_its_kit() {
    let mut app = loose_kit_at(Path::new("/ek/H3EK/tags"));
    app.prefs.custom_editing_kit_profiles = vec![CustomEditingKitProfile {
        read_only: true,
        git_tracked: false,
        id: "00000000-0000-4000-8000-000000000001".to_owned(),
        name: "H3EK".to_owned(),
        game: "halo3_mcc".to_owned(),
        root: PathBuf::from("/ek/H3EK"),
        icon: None,
        tags_folder: None,
        data_folder: None,
    }];
    assert!(app.editing_kit_is_read_only(0));
    app.prefs.custom_editing_kit_profiles[0].root = PathBuf::from("/ek/other");
    assert!(!app.editing_kit_is_read_only(0));
}
