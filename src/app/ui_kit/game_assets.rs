//! Game display names and embedded banner/emblem asset mappings.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use crate::core::game::{GameFacts, GameId};

/// The banner for a game, or Halo CE's for a game id this build does not know.
pub(in crate::app) fn get_game_banner_bytes(game: Option<GameId>) -> &'static [u8] {
    let Some(game) = game else {
        return include_root_bytes!("assets/Game Icons/ce.png");
    };
    match game {
        GameId::HaloCe => include_root_bytes!("assets/Game Icons/ce.png"),
        GameId::Halo2 => include_root_bytes!("assets/Game Icons/h2.png"),
        GameId::Halo2Amp => include_root_bytes!("assets/Game Icons/h2amp.png"),
        GameId::Halo3 => include_root_bytes!("assets/Game Icons/h3.png"),
        GameId::Halo3Odst => include_root_bytes!("assets/Game Icons/h3odst.png"),
        GameId::HaloReach => include_root_bytes!("assets/Game Icons/reach.png"),
        GameId::Halo4 => include_root_bytes!("assets/Game Icons/h4.png"),
        GameId::CampaignEvolved => {
            include_root_bytes!("assets/Game Icons/campaignevolved.png")
        }
    }
}

/// Compact engine emblems used by editing-kit links on the welcome screen.
/// These intentionally remain separate from the larger game banner artwork.
pub(in crate::app) fn get_game_emblem_bytes(game: GameId) -> &'static [u8] {
    match game {
        GameId::HaloCe => include_root_bytes!("assets/Game Icons/emblems/h1.png"),
        GameId::Halo2 => include_root_bytes!("assets/Game Icons/emblems/h2.png"),
        GameId::Halo2Amp => include_root_bytes!("assets/Game Icons/emblems/h2a.png"),
        GameId::Halo3 => include_root_bytes!("assets/Game Icons/emblems/h3.png"),
        GameId::Halo3Odst => include_root_bytes!("assets/Game Icons/emblems/h3odst.png"),
        GameId::HaloReach => include_root_bytes!("assets/Game Icons/emblems/hreach.png"),
        GameId::Halo4 => include_root_bytes!("assets/Game Icons/emblems/h4.png"),
        GameId::CampaignEvolved => include_root_bytes!("assets/Game Icons/emblems/campaignevolved.png"),
    }
}

/// The display name for a saved game id, which may be one this build does not
/// know; a [`GameId`] in hand has [`GameFacts::display_name`].
pub(in crate::app) fn game_display_name(game: &str) -> &'static str {
    GameId::from_id(game).map_or("Unknown Game", GameFacts::display_name)
}

/// Platform/edition suffix shown after the game name (e.g. "MCC", "PC").
pub(in crate::app) fn game_platform_label(game: GameId) -> &'static str {
    if game.is_campaign_evolved() {
        "PC"
    } else {
        "MCC"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_emblems_are_separate_from_game_banners() {
        // Every game has an emblem, distinct from its banner and from the others.
        let emblems: std::collections::HashSet<_> =
            GameId::ALL.iter().map(|game| get_game_emblem_bytes(*game)).collect();
        assert_eq!(emblems.len(), GameId::ALL.len());
        assert_ne!(
            get_game_emblem_bytes(GameId::HaloCe),
            get_game_banner_bytes(Some(GameId::HaloCe))
        );
        assert_ne!(
            get_game_banner_bytes(Some(GameId::CampaignEvolved)),
            get_game_banner_bytes(Some(GameId::HaloCe))
        );
        // A saved id this build does not know falls back to Halo CE's banner.
        assert_eq!(get_game_banner_bytes(None), get_game_banner_bytes(Some(GameId::HaloCe)));
    }
}
