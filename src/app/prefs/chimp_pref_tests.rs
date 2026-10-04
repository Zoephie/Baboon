use super::*;

#[test]
fn editing_kit_migration_is_stable_and_saves_only_unified_profiles() {
    let legacy = json!({
        "editing_kit_paths": {
            "halo2_mcc": "Z:/Unavailable/H2EK",
            "haloce_evolved": "Z:/Unavailable/Halo Campaign Evolved"
        },
        "custom_editing_kit_profiles": [{
            "id": "11111111-1111-4111-8111-111111111111",
            "name": "My mod", "game": "halo3_mcc", "root": "Z:/Unavailable/Mod",
            "icon": "editing kit icons/mod/icon.png"
        }]
    });
    let prefs = prefs_from_value(&legacy);
    assert!(prefs.editing_kit_paths.is_empty());
    assert_eq!(prefs.custom_editing_kit_profiles.len(), 3);
    assert_eq!(
        prefs.custom_editing_kit_profiles,
        prefs_from_value(&legacy).custom_editing_kit_profiles
    );
    assert_eq!(prefs.custom_editing_kit_profiles[0].name, "My mod");
    assert!(prefs.custom_editing_kit_profiles[0].icon.is_some());
    let saved = prefs_to_value(&prefs, &HashSet::new(), true);
    assert!(saved.get("editing_kit_paths").is_none());
    assert!(saved.get("custom_editing_kit_profiles").is_none());
    assert_eq!(
        prefs.custom_editing_kit_profiles,
        prefs_from_value(&saved).custom_editing_kit_profiles
    );
    let mut removed = saved;
    removed["editing_kit_profiles"] = json!([]);
    removed["editing_kit_paths"] = legacy["editing_kit_paths"].clone();
    assert!(
        prefs_from_value(&removed)
            .custom_editing_kit_profiles
            .is_empty()
    );
}

#[test]
fn editing_kit_detection_profiles_deduplicate_roots_without_changing_edits() {
    let paths = HashMap::from([("halo2_mcc".to_owned(), PathBuf::from("Z:/Unavailable/H2EK"))]);
    let mut profiles = Vec::new();
    assert_eq!(add_standard_editing_kit_profiles(&mut profiles, &paths), 1);
    profiles[0].name = "Renamed kit".to_owned();
    profiles[0].icon = Some(PathBuf::from("editing kit icons/mod/icon.png"));
    let previous = profiles.clone();
    assert_eq!(add_standard_editing_kit_profiles(&mut profiles, &paths), 0);
    assert_eq!(profiles, previous);
    let legacy = json!({
        "editing_kit_paths": { "halo2_mcc": "Z:/Unavailable/H2EK" },
        "custom_editing_kit_profiles": custom_editing_kit_profiles_value(&profiles)
    });
    assert_eq!(
        prefs_from_value(&legacy).custom_editing_kit_profiles,
        previous
    );
}

/// A profile or folder alias naming a game this build does not know (one
/// a newer Baboon supports, or none) used to be dropped on load, and so
/// deleted from the file by the next save of any preference.
#[test]
fn kits_for_unsupported_games_survive_a_save_but_are_not_offered() {
    let usable = json!({
        "id": "6f1c3f9e-1d1b-4c2a-9a55-0d6f1d1b2c3a",
        "name": "Halo 3",
        "game": "halo3_mcc",
        "root": "/kits/h3",
    });
    let future = json!({
        "id": "0b5e2a7c-3a43-4f37-8f2e-6a9d2c1b4e5f",
        "name": "Halo Infinite",
        "game": "haloinfinite",
        "root": "/kits/hi",
        "some_newer_setting": [1, 2, 3],
    });
    let gameless = json!({
        "id": "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f",
        "name": "No game",
        "game": "",
        "root": "/kits/none",
    });
    let future_alias = json!({ "folder_name": "HIEK", "game": "haloinfinite" });
    let usable_alias = json!({ "folder_name": "H3EK", "game": "halo3_mcc" });
    let stored = json!({
        "editing_kit_profiles": [usable, future, gameless],
        "ek_folder_aliases": [usable_alias, future_alias],
    });

    let prefs = prefs_from_value(&stored);
    let offered: Vec<&str> = prefs
        .custom_editing_kit_profiles
        .iter()
        .map(|profile| profile.game.as_str())
        .collect();
    assert_eq!(offered, ["halo3_mcc"], "only a supported game is a kit");
    assert_eq!(prefs.ek_folder_aliases.len(), 1);

    let written = prefs_to_value(&prefs, &HashSet::new(), true);
    let profiles = written["editing_kit_profiles"].as_array().unwrap();
    assert!(profiles.contains(&future), "{profiles:#?}");
    assert!(profiles.contains(&gameless), "{profiles:#?}");
    assert_eq!(profiles.len(), 3);
    let aliases = written["ek_folder_aliases"].as_array().unwrap();
    assert!(aliases.contains(&future_alias), "{aliases:#?}");
    assert_eq!(aliases.len(), 2);

    // Written back and read again, nothing changes.
    assert!(prefs_from_value(&written) == prefs);
}

#[test]
fn chimp_is_enabled_for_preferences_written_before_it_existed() {
    let prefs = prefs_from_value(&json!({}));
    assert!(prefs.enable_chimp);
    assert_eq!(prefs.chimp_output_dir, None);
    assert_eq!(prefs.chimp_usmap_path, None);
}

#[test]
fn chimp_visibility_and_output_directory_round_trip() {
    let prefs = GuiPrefs {
        enable_chimp: false,
        chimp_output_dir: Some(PathBuf::from("D:/Mods/Chimp")),
        chimp_usmap_path: Some(PathBuf::from("D:/Mappings/Meteorite.usmap")),
        ..GuiPrefs::default()
    };
    let value = prefs_to_value(&prefs, &HashSet::new(), true);
    let restored = prefs_from_value(&value);
    assert!(!restored.enable_chimp);
    assert_eq!(
        restored.chimp_output_dir,
        Some(PathBuf::from("D:/Mods/Chimp"))
    );
    assert_eq!(
        restored.chimp_usmap_path,
        Some(PathBuf::from("D:/Mappings/Meteorite.usmap"))
    );
}
