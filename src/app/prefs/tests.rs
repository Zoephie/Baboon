use super::*;

#[test]
fn last_session_v1_remains_compatible() {
    let session = parse_last_session(
        &serde_json::from_str::<Value>(
            r#"{
                "version": 1,
                "source": {
                    "kind": "loose_folder",
                    "path": "C:/tags",
                    "game": "haloreach"
                },
                "tags": [{
                    "key": "objects/test.weapon",
                    "label": "test",
                    "group_tag": 2003132784
                }]
            }"#,
        )
        .expect("valid json"),
    )
    .expect("version 1 session");
    // A single-source file loads as one kit.
    assert_eq!(session.kits.len(), 1);
    assert_eq!(
        session.kits[0].source_kind,
        LastSessionSourceKind::LooseFolder
    );
    assert_eq!(session.kits[0].project_path, None);
    assert_eq!(session.kits[0].tags.len(), 1);
}

#[test]
fn project_pointer_survives_with_no_open_tabs() {
    let session = parse_last_session(
        &serde_json::from_str::<Value>(
            r#"{
                "version": 2,
                "source": {
                    "kind": "iostore_container_set",
                    "path": "C:/CampaignEvolved/Paks",
                    "game": "haloce_evolved",
                    "project_path": "C:/mods/recovery.baboon"
                },
                "tags": []
            }"#,
        )
        .expect("valid json"),
    )
    .expect("project-only session");
    assert_eq!(session.kits.len(), 1);
    assert_eq!(
        session.kits[0].source_kind,
        LastSessionSourceKind::IoStoreContainerSet
    );
    assert_eq!(
        session.kits[0].project_path,
        Some(PathBuf::from("C:/mods/recovery.baboon"))
    );
    assert!(session.kits[0].tags.is_empty());
}

#[test]
fn legacy_custom_color_swatches_migrate_to_last_row() {
    let value = serde_json::json!({
        "custom_color_swatches": [
            "#FF0000FF",
            null,
            "#33669980",
            "not-a-color"
        ]
    });

    let swatches = load_custom_color_swatches(&value);
    assert_eq!(swatches.len(), CUSTOM_COLOR_SWATCH_COUNT);
    assert_eq!(
        swatches[0],
        Some(ColorPaletteSwatch::unnamed([255, 0, 0, 255]))
    );
    assert_eq!(
        swatches[48],
        Some(ColorPaletteSwatch::unnamed([255, 0, 0, 255]))
    );
    assert_eq!(swatches[49], None);
    assert_eq!(
        swatches[50],
        Some(ColorPaletteSwatch::unnamed([51, 102, 153, 128]))
    );
    assert_eq!(swatches[51], None);
}

#[test]
fn named_color_swatches_load_from_preferences() {
    let value = serde_json::json!({
        "custom_color_swatches": [
            { "rgba": "#FF0000FF", "name": "Red" }
        ]
    });

    let swatches = load_custom_color_swatches(&value);
    assert_eq!(
        swatches[48],
        Some(ColorPaletteSwatch::named([255, 0, 0, 255], "Red"))
    );
}

#[test]
fn load_editing_kit_paths_ignores_empty_and_unknown_entries() {
    let value = json!({
        "editing_kit_paths": {
            "halo3_mcc": "C:/Games/H3EK",
            "haloce_evolved": "D:/Games/Halo Campaign Evolved",
            "halo4_mcc": "",
            "unknown": "C:/Games/Unknown"
        }
    });

    let paths = load_editing_kit_paths(&value);

    assert_eq!(paths.len(), 2);
    assert_eq!(
        paths.get("halo3_mcc"),
        Some(&PathBuf::from("C:/Games/H3EK"))
    );
    assert_eq!(
        paths.get("haloce_evolved"),
        Some(&PathBuf::from("D:/Games/Halo Campaign Evolved"))
    );
    assert!(!paths.contains_key("halo4_mcc"));
    assert!(!paths.contains_key("unknown"));
}

#[cfg(windows)]
#[test]
fn clean_recent_path_hides_windows_verbatim_prefixes() {
    assert_eq!(
        clean_recent_path(PathBuf::from(r"\\?\D:\Games\H2EK")),
        PathBuf::from(r"D:\Games\H2EK")
    );
    assert_eq!(
        clean_recent_path(PathBuf::from(r"\\?\UNC\server\share\H3EK")),
        PathBuf::from(r"\\server\share\H3EK")
    );
    assert_eq!(
        clean_recent_path(PathBuf::from(r"D:\Games\H4EK")),
        PathBuf::from(r"D:\Games\H4EK")
    );
}

#[test]
fn custom_editing_kit_profiles_round_trip_in_creation_order() {
    assert!(load_custom_editing_kit_profiles(&json!({})).is_empty());
    let value = json!({
        "custom_editing_kit_profiles": [
            {
                "id": "11111111-1111-4111-8111-111111111111",
                "name": "Reach Project",
                "read_only": true,
                "git_tracked": true,
                "game": "haloreach_mcc",
                "root": "\\\\?\\D:\\Kits\\ReachProject",
                "icon": "editing kit icons/reach-11111111/icon-a.png"
            },
            {
                "id": "22222222-2222-4222-8222-222222222222",
                "name": "Second Project",
                "game": "halo3_mcc",
                "root": "D:/Kits/H3Project",
                "icon": "../../unsafe.png"
            }
        ]
    });
    let profiles = load_custom_editing_kit_profiles(&value);
    assert!(profiles[0].read_only);
    assert!(profiles[0].git_tracked);
    assert!(!profiles[1].read_only, "old entries must remain writable");
    assert!(!profiles[1].git_tracked, "old entries must not enable Git");
    assert_eq!(
        profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Reach Project", "Second Project"]
    );
    assert_eq!(
        profiles[0].icon.as_deref(),
        Some(Path::new("editing kit icons/reach-11111111/icon-a.png"))
    );
    assert_eq!(profiles[1].icon, None);
    #[cfg(windows)]
    assert_eq!(profiles[0].root, PathBuf::from(r"D:\Kits\ReachProject"));

    let serialized = json!({
        "custom_editing_kit_profiles": custom_editing_kit_profiles_value(&profiles)
    });
    assert_eq!(load_custom_editing_kit_profiles(&serialized), profiles);
}

#[test]
fn editing_kit_favorites_are_scoped_by_tags_root() {
    let value = json!({
        "editing_kit_favorites": [
            {
                "tags_root": "C:/Games/H2EK/tags",
                "tags": [
                    "objects/brute.model",
                    "objects/brute.model",
                    "../outside.model"
                ],
                "folders": [
                    "objects/characters/brute",
                    "objects/characters/brute",
                    "../outside"
                ]
            },
            {
                "tags_root": "C:/Games/H3EK/tags",
                "tags": ["objects/brute.model"]
            }
        ]
    });

    let favorites = load_editing_kit_favorites(&value);

    assert_eq!(favorites.len(), 2);
    assert_eq!(favorites[0].tags_root, PathBuf::from("C:/Games/H2EK/tags"));
    assert_eq!(
        favorites[0].tags,
        vec![PathBuf::from("objects/brute.model")]
    );
    assert_eq!(
        favorites[0].folders,
        vec![PathBuf::from("objects/characters/brute")]
    );
    assert_eq!(favorites[1].tags_root, PathBuf::from("C:/Games/H3EK/tags"));
    assert_eq!(
        favorites[1].tags,
        vec![PathBuf::from("objects/brute.model")]
    );
    assert!(favorites[1].folders.is_empty());
}

#[test]
fn favorite_paths_must_be_relative_and_normalized() {
    assert_eq!(
        clean_favorite_relative_path(PathBuf::from("objects/brute.model")),
        Some(PathBuf::from("objects/brute.model"))
    );
    assert!(clean_favorite_relative_path(PathBuf::from("../brute.model")).is_none());
    assert!(clean_favorite_relative_path(PathBuf::from("./brute.model")).is_none());
    assert!(clean_favorite_relative_path(PathBuf::new()).is_none());
}
