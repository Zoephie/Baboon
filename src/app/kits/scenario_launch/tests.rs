use super::*;

fn loaded_source(root: PathBuf, game: &str) -> LoadedSourceData {
    LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root,
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
        chosen_kit_layout: None,
    }
}

fn scenario_entry(path: PathBuf) -> TagEntry {
    TagEntry {
        key: path.display().to_string(),
        display_path: "levels/test/my map/my map.scenario".to_owned(),
        group_tag: SCENARIO_GROUP_TAG,
        group_name: Some("scenario".to_owned()),
        location: TagEntryLocation::LooseFile(path),
    }
}

#[test]
fn launch_context_builds_an_extensionless_engine_path() {
    let kit_root = std::env::temp_dir().join("baboon-scenario-context");
    let tags_root = kit_root.join("tags");
    let scenario_file = tags_root
        .join("levels")
        .join("test")
        .join("my map")
        .join("my map.scenario");
    let entry = scenario_entry(scenario_file.clone());
    let context =
        scenario_launch_context(&loaded_source(tags_root, "halo3_mcc"), &entry).unwrap();
    assert_eq!(context.kit_root, kit_root);
    assert_eq!(context.scenario_file, scenario_file);
    assert_eq!(context.scenario_path, "levels\\test\\my map\\my map");
    assert_eq!(context.game, GameId::Halo3);
}

/// The browser's row menu gates on this rather than on the per-entry
/// context, so what it refuses is worth pinning separately: a kit that
/// cannot launch at all must offer neither item, and a kit whose Sapien
/// takes no scenario must offer only tag_test.
#[test]
fn availability_refuses_unsupported_kits_and_hides_halo_ce_sapien() {
    let tags_root = std::env::temp_dir()
        .join("baboon-availability")
        .join("tags");

    let unsupported =
        scenario_launch_availability(&loaded_source(tags_root.clone(), "haloce_evolved"));
    assert!(
        !unsupported.supported,
        "Campaign Evolved launches no scenarios"
    );
    assert!(!unsupported.offers_sapien);

    // Combat Evolved keeps tag_test but can never take a scenario in Sapien.
    let halo_ce = scenario_launch_availability(&loaded_source(tags_root.clone(), "haloce_mcc"));
    assert!(halo_ce.supported);
    assert!(
        !halo_ce.offers_sapien,
        "Halo CE's Sapien takes no scenario argument"
    );

    let halo3 = scenario_launch_availability(&loaded_source(tags_root.clone(), "halo3_mcc"));
    assert!(halo3.supported && halo3.offers_sapien);
    // Nothing is on disk under a temp path, so neither executable is found
    // and both items would draw disabled rather than missing.
    assert!(!halo3.sapien_present && !halo3.tag_test_present);

    // A folder that is not the kit's `tags` root cannot resolve a kit root.
    let not_tags = scenario_launch_availability(&loaded_source(
        tags_root.parent().unwrap().to_path_buf(),
        "halo3_mcc",
    ));
    assert!(!not_tags.supported);
}

#[test]
fn launch_context_rejects_unsupported_sources_and_escaping_paths() {
    let kit_root = std::env::temp_dir().join("baboon-scenario-context-errors");
    let tags_root = kit_root.join("tags");
    let outside = scenario_entry(kit_root.join("outside.scenario"));
    assert!(
        scenario_launch_context(&loaded_source(tags_root.clone(), "halo3_mcc"), &outside)
            .is_err()
    );

    let valid = scenario_entry(tags_root.join("levels").join("test.scenario"));
    assert!(
        scenario_launch_context(&loaded_source(tags_root, "haloce_evolved"), &valid).is_err()
    );
}

/// The same answer decides whether the scenario's Sapien button is drawn at
/// all, so this covers every game Baboon ships definitions for rather than
/// only the ones that support it — a game added without a decision here
/// would silently get no button.
#[test]
fn sapien_scenario_arguments_exclude_halo_ce() {
    // Combat Evolved's Sapien takes no scenario argument, and Campaign
    // Evolved has no Sapien. Both mean "no button", not "greyed out".
    assert!(!GameId::HaloCe.sapien_takes_scenario_argument());
    assert!(!GameId::CampaignEvolved.sapien_takes_scenario_argument());
    for game in [
        "halo2_mcc",
        "halo3_mcc",
        "halo3odst_mcc",
        "haloreach_mcc",
        "halo4_mcc",
        "halo2amp_mcc",
    ] {
        let game = GameId::from_id(game).unwrap();
        assert!(game.sapien_takes_scenario_argument(), "{game}");
    }
}

/// Every game with definitions is either offered the button or explicitly
/// not, so adding a game cannot leave this unanswered by accident.
#[test]
fn every_shipped_game_has_a_decision_about_the_sapien_button() {
    let definitions = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let games: Vec<String> = std::fs::read_dir(&definitions)
        .expect("the definitions submodule is required to build")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    assert!(!games.is_empty(), "definitions/ has per-game folders");
    let without_sapien: Vec<&String> = games
        .iter()
        .filter(|game| {
            let game = GameId::from_id(game).unwrap_or_else(|| panic!("definitions/{game} names no game"));
            !game.sapien_takes_scenario_argument()
        })
        .collect();
    assert_eq!(
        without_sapien.len(),
        2,
        "exactly two shipped games have no scenario-capable Sapien, got {without_sapien:?}"
    );
}

#[test]
fn startup_commands_and_executables_match_supported_games() {
    let cases = [
        (
            "haloce_mcc",
            "map_name levels\\test\\map",
            "halo_tag_test.exe",
        ),
        (
            "halo2_mcc",
            "game_start levels\\test\\map",
            "halo2_tag_test.exe",
        ),
        (
            "halo3_mcc",
            "game_start levels\\test\\map",
            "halo3_tag_test.exe",
        ),
        (
            "halo3odst_mcc",
            "game_start levels\\test\\map",
            "atlas_tag_test.exe",
        ),
        (
            "haloreach_mcc",
            "game_start levels\\test\\map",
            "reach_tag_test.exe",
        ),
        (
            "halo4_mcc",
            "game_start levels\\test\\map",
            "halo4_tag_test.exe",
        ),
        (
            "halo2amp_mcc",
            "game_start levels\\test\\map",
            "halo2a_tag_test.exe",
        ),
    ];
    for (game, command, executable) in cases {
        let game = GameId::from_id(game).unwrap();
        assert_eq!(scenario_startup_command(game, "levels\\test\\map"), command);
        assert_eq!(tag_test_executable_for_game(Some(game)), executable);
    }
    assert_eq!(
        scenario_startup_command(GameId::Halo3, "levels\\my map\\my map"),
        "game_start \"levels\\my map\\my map\""
    );
    assert_eq!(
        scenario_startup_command(GameId::Halo3, "levels\\semi;colon"),
        "game_start \"levels\\semi;colon\""
    );
}

#[test]
fn startup_update_preserves_settings_comments_bom_and_crlf() {
    let existing =
        b"\xEF\xBB\xBF; game_start old\\comment\r\ndebug_objects 1\r\ngame_start old\\map ; primary note\r\nmap_name duplicate ; duplicate note\r\n";
    let updated = update_startup_file_bytes(existing, "game_start levels\\new\\map").unwrap();
    assert_eq!(
        updated,
        b"\xEF\xBB\xBF; game_start old\\comment\r\ndebug_objects 1\r\ngame_start levels\\new\\map ; primary note\r\n; duplicate note\r\n"
    );
}

#[test]
fn startup_update_appends_and_preserves_missing_trailing_newline() {
    let updated =
        update_startup_file_bytes(b"debug_objects 1", "game_start levels\\new\\map").unwrap();
    assert_eq!(updated, b"debug_objects 1\ngame_start levels\\new\\map");

    let created = update_startup_file_bytes(b"", "map_name levels\\test\\map").unwrap();
    assert_eq!(created, b"map_name levels\\test\\map\n");
}

#[test]
fn startup_update_rejects_non_utf8_files() {
    assert!(update_startup_file_bytes(&[0xff], "game_start map").is_err());
    assert!(clear_startup_file_bytes(&[0xff]).is_err());
}

#[test]
fn startup_clear_removes_active_launches_and_preserves_other_content() {
    let existing =
        b"\xEF\xBB\xBF; game_start commented\r\ndebug_objects 1\r\ngame_start old\\map ; first note\r\nmap_name duplicate ; second note\r\n";
    let cleared = clear_startup_file_bytes(existing).unwrap();
    assert_eq!(
        cleared,
        b"\xEF\xBB\xBF; game_start commented\r\ndebug_objects 1\r\n; first note\r\n; second note\r\n"
    );
}

#[test]
fn startup_file_is_created_and_atomically_replaced() {
    let root = std::env::temp_dir().join(format!(
        "baboon-scenario-init-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("init.txt");
    update_scenario_startup_file(&path, "game_start levels\\first").unwrap();
    update_scenario_startup_file(&path, "game_start levels\\second").unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "game_start levels\\second\n"
    );
    fs::remove_dir_all(root).unwrap();
}
