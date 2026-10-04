use std::collections::HashSet;

use super::*;

#[test]
fn every_game_has_its_own_name_and_kit() {
    let names: HashSet<_> = GameId::ALL.iter().map(|game| game.display_name()).collect();
    assert_eq!(names.len(), GameId::ALL.len());
    let kits: Vec<_> = GameId::ALL.iter().filter_map(|game| game.kit_name()).collect();
    assert_eq!(kits.len(), 7, "every MCC game has a kit; Campaign Evolved has none");
    assert_eq!(kits.iter().collect::<HashSet<_>>().len(), kits.len());
}

#[test]
fn kit_folders_name_their_game_in_any_case() {
    for game in GameId::ALL {
        let Some(kit) = game.kit_name() else {
            continue;
        };
        assert_eq!(game_for_kit_folder(kit), Some(game), "{kit}");
        assert_eq!(game_for_kit_folder(&kit.to_ascii_lowercase()), Some(game), "{kit}");
        assert_eq!(game_for_kit_folder(game.as_str()), Some(game));
        assert_eq!(game_for_kit_folder(&game.as_str().to_ascii_uppercase()), Some(game));
    }
    for (older, game) in [
        ("H1EK", GameId::HaloCe),
        ("HaloCEEK", GameId::HaloCe),
        ("Halo2EK", GameId::Halo2),
        ("H2AEK", GameId::Halo2Amp),
    ] {
        assert_eq!(game_for_kit_folder(older), Some(game), "{older}");
    }
    // Campaign Evolved has no kit folder, and other folders name no game.
    assert_eq!(game_for_kit_folder("haloce_evolved"), None);
    assert_eq!(game_for_kit_folder("tags"), None);
    assert_eq!(game_for_kit_folder("H5EK"), None);
}

#[test]
fn launch_flags_name_their_game_and_kit() {
    for game in GameId::ALL {
        if let Some(kit) = game.kit_name() {
            assert_eq!(game_for_launch_flag(&format!("-{kit}")), Some(game));
            assert_eq!(game_for_launch_flag(&format!("-{}", kit.to_ascii_lowercase())), Some(game));
        }
    }
    assert_eq!(game_for_launch_flag("-H1EK"), Some(GameId::HaloCe));
    assert_eq!(game_for_launch_flag("-H2AEK"), Some(GameId::Halo2Amp));
    assert_eq!(game_for_launch_flag("H3EK"), None, "a flag starts with a dash");
    assert_eq!(game_for_launch_flag("-halo3_mcc"), None);
    assert_eq!(game_for_launch_flag("-H5EK"), None);
}

#[test]
fn only_kit_games_launch_scenarios_and_halo_ce_starts_them_by_map_name() {
    for game in GameId::ALL {
        assert_eq!(game.launches_scenarios(), game.tag_test_executable().is_some(), "{game}");
        assert_eq!(
            game.sapien_takes_scenario_argument(),
            game.launches_scenarios() && game != GameId::HaloCe,
            "{game}"
        );
    }
    assert!(!GameId::CampaignEvolved.launches_scenarios());
    assert_eq!(GameId::HaloCe.scenario_startup_command(), "map_name");
    assert_eq!(GameId::HaloReach.scenario_startup_command(), "game_start");
}
