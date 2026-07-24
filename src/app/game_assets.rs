//! Game display names and embedded banner/emblem asset mappings.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

pub(super) fn get_game_banner_bytes(game: &str) -> &'static [u8] {
    match game {
        "haloce_mcc" => include_bytes!("../../assets/Game Icons/ce.png"),
        "halo2_mcc" => include_bytes!("../../assets/Game Icons/h2.png"),
        "halo2amp_mcc" => include_bytes!("../../assets/Game Icons/h2amp.png"),
        "halo3_mcc" => include_bytes!("../../assets/Game Icons/h3.png"),
        "halo3odst_mcc" => include_bytes!("../../assets/Game Icons/h3odst.png"),
        "haloreach_mcc" => include_bytes!("../../assets/Game Icons/reach.png"),
        "halo4_mcc" => include_bytes!("../../assets/Game Icons/h4.png"),
        "haloce_evolved" => include_bytes!("../../assets/Game Icons/ce.png"),
        _ => include_bytes!("../../assets/Game Icons/ce.png"),
    }
}

/// Engine emblems used only by the compact top-toolbar editing-kit shortcuts.
/// These intentionally remain separate from the larger game banner artwork.
pub(super) fn get_game_emblem_bytes(game: &str) -> Option<&'static [u8]> {
    match game {
        "haloce_mcc" => Some(include_bytes!("../../assets/Game Icons/emblems/h1.png")),
        "halo2_mcc" => Some(include_bytes!("../../assets/Game Icons/emblems/h2.png")),
        "halo2amp_mcc" => Some(include_bytes!("../../assets/Game Icons/emblems/h2a.png")),
        "halo3_mcc" => Some(include_bytes!("../../assets/Game Icons/emblems/h3.png")),
        "halo3odst_mcc" => Some(include_bytes!("../../assets/Game Icons/emblems/h3odst.png")),
        "haloreach_mcc" => Some(include_bytes!("../../assets/Game Icons/emblems/hreach.png")),
        "halo4_mcc" => Some(include_bytes!("../../assets/Game Icons/emblems/h4.png")),
        "haloce_evolved" => Some(include_bytes!("../../assets/Game Icons/emblems/h1.png")),
        _ => None,
    }
}

pub(super) fn game_display_name(game: &str) -> &'static str {
    match game {
        "haloce_mcc" => "Halo: Combat Evolved",
        "halo2_mcc" => "Halo 2",
        "halo2amp_mcc" => "Halo 2 Anniversary Multiplayer",
        "halo3_mcc" => "Halo 3",
        "halo3odst_mcc" => "Halo 3: ODST",
        "haloreach_mcc" => "Halo: Reach",
        "halo4_mcc" => "Halo 4",
        "haloce_evolved" => "Halo: Campaign Evolved",
        _ => "Unknown Game",
    }
}

/// Platform/edition suffix shown after the game name (e.g. "MCC", "PC").
pub(super) fn game_platform_label(game: &str) -> &'static str {
    match game {
        "haloce_evolved" => "PC",
        _ => "MCC",
    }
}
