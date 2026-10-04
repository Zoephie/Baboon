//! Compile-time access to the generated editing-kit command catalogs.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use crate::core::game::GameId;

/// The kit tool's command catalog for `game`. ODST's tool takes Halo 3's
/// commands; Halo 2 Anniversary Multiplayer and Campaign Evolved have none.
pub fn get_tool_commands_json(game: GameId) -> Option<&'static str> {
    match game {
        GameId::HaloCe => Some(include_root_str!("tool_commands/haloce_mcc.json")),
        GameId::Halo2 => Some(include_root_str!("tool_commands/halo2_mcc.json")),
        GameId::Halo3 | GameId::Halo3Odst => Some(include_root_str!("tool_commands/halo3_mcc.json")),
        GameId::HaloReach => Some(include_root_str!("tool_commands/haloreach_mcc.json")),
        GameId::Halo4 => Some(include_root_str!("tool_commands/halo4_mcc.json")),
        GameId::Halo2Amp | GameId::CampaignEvolved => None,
    }
}
