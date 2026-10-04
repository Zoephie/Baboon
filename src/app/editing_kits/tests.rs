use super::*;

fn temp_dir(label: &str) -> PathBuf {
    crate::test_kits::unique_temp_dir(&format!("editing-kits-{label}"))
}

#[test]
fn icon_storage_uses_active_data_root_and_preserves_legacy_icons() {
    let root = temp_dir("icon-storage-modes");
    let installed = root.join("AppData").join("Baboon");
    let portable = root.join("PortableBaboon");
    let source = root.join("source.png");
    image::RgbaImage::new(32, 32).save(&source).unwrap();
    // The old behaviour, and portable mode, use the executable directory.
    let relative = copy_custom_icon_at(&portable, &source, "My Kit", "12345678", None).unwrap();
    let legacy = portable.join(&relative);
    assert!(legacy.is_file());
    assert_eq!(
        resolve_custom_icon_path_in_roots(&installed, Some(&portable), &relative).unwrap(),
        legacy,
    );
    assert_eq!(
        resolve_custom_icon_path_at(&portable, &relative).unwrap(),
        legacy
    );

    // Newly saved installed-mode copies go beneath the data directory.
    let saved = copy_custom_icon_at(&installed, &source, "My Kit", "12345678", Some(&relative))
        .unwrap();
    assert_eq!(saved, relative);
    assert!(installed.join(&saved).is_file());
    assert!(
        legacy.is_file(),
        "saving a new copy must not move the existing icon"
    );
    assert_eq!(
        resolve_custom_icon_path_in_roots(&installed, Some(&portable), &saved).unwrap(),
        installed.join(&saved),
    );
    // Cleanup targets the same location as lookup, not the old executable copy.
    remove_unreferenced_custom_icon_in_roots(&installed, Some(&portable), &saved, &[]).unwrap();
    assert!(!installed.join(&saved).exists());
    assert!(legacy.is_file());
    assert_eq!(
        resolve_custom_icon_path_in_roots(&installed, Some(&portable), &relative).unwrap(),
        legacy,
    );
    assert!(
        resolve_custom_icon_path_in_roots(
            &installed,
            Some(&portable),
            Path::new("../outside.png"),
        )
        .is_err()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn custom_layout_accepts_direct_selected_and_nested_roots() {
    let direct = temp_dir("direct");
    fs::create_dir_all(direct.join("tags")).unwrap();
    fs::create_dir_all(direct.join("data")).unwrap();
    fs::create_dir_all(direct.join("archive").join("tags")).unwrap();
    fs::create_dir_all(direct.join("archive").join("data")).unwrap();
    let layout = validate_custom_editing_kit_layout(&direct).unwrap();
    assert_eq!(layout.root, canonical_or_clean(&direct));
    assert_eq!(
        validate_custom_editing_kit_layout(&direct.join("tags"))
            .unwrap()
            .root,
        layout.root
    );

    let outer = temp_dir("nested");
    let nested = outer.join("projects").join("my-kit");
    fs::create_dir_all(nested.join("tags")).unwrap();
    fs::create_dir_all(nested.join("data")).unwrap();
    assert_eq!(
        validate_custom_editing_kit_layout(&outer).unwrap().root,
        canonical_or_clean(&nested)
    );
    let _ = fs::remove_dir_all(direct);
    let _ = fs::remove_dir_all(outer);
}

#[test]
fn validation_cache_changes_only_when_refreshed() {
    let root = temp_dir("cached");
    fs::create_dir_all(root.join("tags")).unwrap();
    let shortcut = EDITING_KIT_SHORTCUTS
        .into_iter()
        .find(|shortcut| shortcut.game == "halo3_mcc")
        .unwrap();
    let paths = HashMap::from([(shortcut.game.to_owned(), root.clone())]);
    let mut cache = EditingKitValidationCache::new(&paths, &[]);
    assert!(cache.builtin(shortcut).layout().is_some());

    fs::remove_dir_all(root.join("tags")).unwrap();
    assert!(cache.builtin(shortcut).layout().is_some());
    cache.refresh(&paths, &[]);
    assert!(matches!(
        cache.builtin(shortcut),
        EditingKitPathStatus::Invalid(_)
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn custom_layout_reports_missing_data_and_ambiguous_projects() {
    let missing = temp_dir("missing-data");
    fs::create_dir_all(missing.join("tags")).unwrap();
    let error = validate_custom_editing_kit_layout(&missing).unwrap_err();
    assert!(error.contains("data directory"), "{error}");

    let ambiguous = temp_dir("ambiguous");
    for name in ["one", "two"] {
        fs::create_dir_all(ambiguous.join(name).join("tags")).unwrap();
        fs::create_dir_all(ambiguous.join(name).join("data")).unwrap();
    }
    let error = validate_custom_editing_kit_layout(&ambiguous).unwrap_err();
    assert!(error.contains("Multiple editing-kit layouts"), "{error}");
    let _ = fs::remove_dir_all(missing);
    let _ = fs::remove_dir_all(ambiguous);
}

#[test]
fn built_in_validation_keeps_existing_tags_only_contract() {
    let root = temp_dir("builtin");
    fs::create_dir_all(root.join("tags")).unwrap();
    let shortcut = EDITING_KIT_SHORTCUTS
        .into_iter()
        .find(|shortcut| shortcut.game == "halo3_mcc")
        .unwrap();
    let status = validate_builtin_editing_kit(shortcut, Some(&root));
    assert!(validate_editing_kit_profile_layout(&root, shortcut.game).is_ok());
    // Read on Windows only, but the `expect` is the check everywhere.
    #[cfg_attr(not(windows), allow(unused_variables))]
    let layout = status.layout().expect("built-in layout should be ready");
    #[cfg(windows)]
    assert!(
        !layout.root.to_string_lossy().starts_with(r"\\?\"),
        "verbatim Windows prefix leaked into validated path"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn campaign_evolved_validation_requires_discoverable_paks() {
    let root = temp_dir("campaign-evolved");
    let shortcut = EDITING_KIT_SHORTCUTS
        .into_iter()
        .find(|shortcut| shortcut.game == "haloce_evolved")
        .unwrap();
    assert!(matches!(
        validate_builtin_editing_kit(shortcut, Some(&root)),
        EditingKitPathStatus::Invalid(_)
    ));

    let paks = root.join("Meteorite").join("Content").join("Paks");
    fs::create_dir_all(&paks).unwrap();
    fs::write(paks.join("campaign.utoc"), []).unwrap();
    assert!(matches!(
        validate_builtin_editing_kit(shortcut, Some(&root)),
        EditingKitPathStatus::Ready(_)
    ));
    assert!(validate_editing_kit_profile_layout(&root, shortcut.game).is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn duplicate_custom_roots_use_resolved_layouts() {
    let outer = temp_dir("duplicates");
    let root = outer.join("kit");
    fs::create_dir_all(root.join("tags")).unwrap();
    fs::create_dir_all(root.join("data")).unwrap();
    let profiles = vec![CustomEditingKitProfile {
        read_only: false,
        git_tracked: false,
        id: "existing".to_owned(),
        name: "Existing".to_owned(),
        game: "halo3_mcc".to_owned(),
        root: root.clone(),
        icon: None,
        tags_folder: None,
        data_folder: None,
    }];
    assert!(custom_profile_tags_conflicts(
        &profiles,
        None,
        &canonical_or_clean(&root.join("tags"))
    ));
    assert!(!custom_profile_tags_conflicts(
        &profiles,
        Some("existing"),
        &canonical_or_clean(&root.join("tags"))
    ));
    let _ = fs::remove_dir_all(outer);
}

#[test]
fn icon_paths_are_sanitised_relative_unique_and_validated() {
    assert_eq!(sanitise_project_name("  CON  "), "project");
    assert_eq!(sanitise_project_name("My: Kit / Test"), "My-Kit-Test");
    assert!(safe_custom_icon_relative_path(Path::new(
        "editing kit icons/my-kit-12345678/icon-a.png"
    )));
    assert!(!safe_custom_icon_relative_path(Path::new("../icon.png")));
    assert!(!safe_custom_icon_relative_path(Path::new(
        "editing kit icons/../../icon.png"
    )));

    let base = temp_dir("icons");
    let source = base.join("source.png");
    image::RgbaImage::new(32, 40).save(&source).unwrap();
    assert_eq!(validate_custom_icon_source(&source).unwrap(), (32, 40));
    let relative = copy_custom_icon_at(
        &base,
        &source,
        "My: Kit",
        "12345678-1234-1234-1234-123456789abc",
        None,
    )
    .unwrap();
    assert!(relative.starts_with(CUSTOM_ICON_FOLDER));
    assert!(
        resolve_custom_icon_path_at(&base, &relative)
            .unwrap()
            .is_file()
    );

    let referencing_profile = CustomEditingKitProfile {
        read_only: false,
        git_tracked: false,
        id: "profile".to_owned(),
        name: "Profile".to_owned(),
        game: "halo3_mcc".to_owned(),
        root: base.clone(),
        icon: Some(relative.clone()),
        tags_folder: None,
        data_folder: None,
    };
    remove_unreferenced_custom_icon_at(
        &base,
        &relative,
        std::slice::from_ref(&referencing_profile),
    )
    .unwrap();
    assert!(
        resolve_custom_icon_path_at(&base, &relative)
            .unwrap()
            .is_file()
    );
    remove_unreferenced_custom_icon_at(&base, &relative, &[]).unwrap();
    assert!(
        !resolve_custom_icon_path_at(&base, &relative)
            .unwrap()
            .exists()
    );

    let renamed_relative = copy_custom_icon_at(
        &base,
        &source,
        "Renamed Kit",
        "12345678-1234-1234-1234-123456789abc",
        Some(&relative),
    )
    .unwrap();
    assert_eq!(renamed_relative.parent(), relative.parent());
    let _ = fs::remove_dir_all(base);
}

fn kit_with_two_folder_sets(label: &str) -> (PathBuf, PathBuf) {
    let outer = temp_dir(label);
    let root = outer.join("H2EK");
    for folder in ["tags", "data", "tags_moda", "data_moda"] {
        fs::create_dir_all(root.join(folder)).unwrap();
    }
    (outer, canonical_or_clean(&root))
}

fn profile(
    id: &str,
    game: &str,
    root: &Path,
    tags: Option<&str>,
    data: Option<&str>,
) -> CustomEditingKitProfile {
    CustomEditingKitProfile {
        read_only: false,
        git_tracked: false,
        id: id.to_owned(),
        name: id.to_owned(),
        game: game.to_owned(),
        root: root.to_path_buf(),
        icon: None,
        tags_folder: tags.map(PathBuf::from),
        data_folder: data.map(PathBuf::from),
    }
}

#[test]
fn chosen_folders_resolve_against_the_root_and_must_exist() {
    let (outer, root) = kit_with_two_folder_sets("chosen-folders");
    let chosen = validate_kit_layout(
        &root,
        "halo2_mcc",
        Some(Path::new("tags_moda")),
        Some(Path::new("data_moda")),
    )
    .unwrap();
    // Only one named: the other is the root's own.
    let tags_only =
        validate_kit_layout(&root, "haloce_mcc", Some(Path::new("tags_moda")), None).unwrap();
    // Absolute folders stand on their own.
    let absolute =
        validate_kit_layout(&root, "halo2_mcc", Some(&root.join("tags_moda")), None).unwrap();
    let missing = validate_kit_layout(&root, "halo2_mcc", Some(Path::new("tags_modb")), None)
        .unwrap_err();
    // An engine whose tools can't follow them ignores them.
    let halo3 = validate_kit_layout(
        &root,
        "halo3_mcc",
        Some(Path::new("tags_moda")),
        Some(Path::new("data_moda")),
    )
    .unwrap();
    let _ = fs::remove_dir_all(outer);

    assert_eq!(chosen.root, root);
    assert_eq!(chosen.tags, root.join("tags_moda"));
    assert_eq!(chosen.data, Some(root.join("data_moda")));
    assert_eq!(tags_only.tags, root.join("tags_moda"));
    assert_eq!(tags_only.data, Some(root.join("data")));
    assert_eq!(absolute.tags, root.join("tags_moda"));
    assert!(missing.contains("Tags folder not found"), "{missing}");
    assert_eq!(halo3.tags, root.join("tags"));
}

#[test]
fn only_folders_other_than_the_roots_own_are_stored() {
    let (outer, root) = kit_with_two_folder_sets("stored-folders");
    let elsewhere = outer.join("elsewhere_tags");
    fs::create_dir_all(&elsewhere).unwrap();
    let elsewhere = canonical_or_clean(&elsewhere);
    let own = folder_to_store(&root, Some(&root.join("tags")), "tags");
    let inside = folder_to_store(&root, Some(&root.join("tags_moda")), "tags");
    let outside = folder_to_store(&root, Some(&elsewhere), "tags");
    let _ = fs::remove_dir_all(outer);
    assert_eq!(own, None);
    assert_eq!(inside, Some(PathBuf::from("tags_moda")));
    assert_eq!(outside, Some(elsewhere));
}

/// Kits may share a root; they may not share a tags folder.
#[test]
fn kits_sharing_a_root_conflict_only_on_a_shared_tags_folder() {
    let (outer, root) = kit_with_two_folder_sets("shared-root");
    let profiles = vec![profile("stock", "halo2_mcc", &root, None, None)];
    let stock_tags = canonical_or_clean(&root.join("tags"));
    let moda_tags = canonical_or_clean(&root.join("tags_moda"));
    let conflicts_with_stock = custom_profile_tags_conflicts(&profiles, None, &stock_tags);
    let conflicts_with_moda = custom_profile_tags_conflicts(&profiles, None, &moda_tags);
    let moda = profile(
        "moda",
        "halo2_mcc",
        &root,
        Some("tags_moda"),
        Some("data_moda"),
    );
    let identity = profile_tags_folder(&moda);
    let _ = fs::remove_dir_all(outer);
    assert!(conflicts_with_stock);
    assert!(!conflicts_with_moda);
    assert_eq!(identity, moda_tags);
}

#[test]
fn the_quick_picks_are_the_roots_matching_folders() {
    let (outer, root) = kit_with_two_folder_sets("candidates");
    let tags = kit_folder_candidates(&root, "tags");
    let data = kit_folder_candidates(&root, "data");
    let default_tags = default_kit_folder_name(&root, "tags");
    let _ = fs::remove_dir_all(outer);
    assert_eq!(tags, vec!["tags".to_owned(), "tags_moda".to_owned()]);
    assert_eq!(data, vec!["data".to_owned(), "data_moda".to_owned()]);
    assert_eq!(default_tags.as_deref(), Some("tags"));
}
