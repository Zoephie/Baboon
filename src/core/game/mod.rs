//! The games Baboon edits, and what Baboon knows about each beyond the tags:
//! names, editing-kit folders and launch flags, the programs a kit ships.
//!
//! Every per-game fact lives here, in one `match` per fact, keyed by the
//! engine's [`GameId`]; nothing else in Baboon spells out a game id. Saved
//! files keep the id string, so an id this build does not know survives a
//! round trip; it is parsed into a `GameId` where it is used.

pub(crate) use blam_tags::game::GameId;

/// What Baboon knows about a game, as methods on [`GameId`].
pub(crate) trait GameFacts: Copy {
    /// The game's name as the UI shows it.
    fn display_name(self) -> &'static str;
    /// The editing kit's short name (`H3EK`), or `None` for a game without a
    /// standalone editing kit (Campaign Evolved).
    fn kit_name(self) -> Option<&'static str>;
    /// The program an editing kit runs a scenario in, or `None` for a game
    /// without one.
    fn tag_test_executable(self) -> Option<&'static str>;
    /// Whether the kit's Sapien takes a scenario on its command line; Halo
    /// CE's does not.
    fn sapien_takes_scenario_argument(self) -> bool;
    /// Whether a scenario can be launched from the kit at all.
    fn launches_scenarios(self) -> bool;
    /// The console command that loads a scenario at startup.
    fn scenario_startup_command(self) -> &'static str;
    /// The kit tool's `monitor-*` verbs, which run until stopped.
    fn monitor_commands(self) -> &'static [&'static str];
    /// Whether HaloScript documentation exists for the game.
    fn has_script_docs(self) -> bool;
}

impl GameFacts for GameId {
    fn display_name(self) -> &'static str {
        match self {
            GameId::HaloCe => "Halo: Combat Evolved",
            GameId::Halo2 => "Halo 2",
            GameId::Halo2Amp => "Halo 2 Anniversary Multiplayer",
            GameId::Halo3 => "Halo 3",
            GameId::Halo3Odst => "Halo 3: ODST",
            GameId::HaloReach => "Halo: Reach",
            GameId::Halo4 => "Halo 4",
            GameId::CampaignEvolved => "Halo: Campaign Evolved",
        }
    }

    fn kit_name(self) -> Option<&'static str> {
        Some(match self {
            GameId::HaloCe => "HCEEK",
            GameId::Halo2 => "H2EK",
            GameId::Halo2Amp => "H2AMPEK",
            GameId::Halo3 => "H3EK",
            GameId::Halo3Odst => "H3ODSTEK",
            GameId::HaloReach => "HREK",
            GameId::Halo4 => "H4EK",
            GameId::CampaignEvolved => return None,
        })
    }

    fn tag_test_executable(self) -> Option<&'static str> {
        Some(match self {
            GameId::HaloCe => "halo_tag_test.exe",
            GameId::Halo2 => "halo2_tag_test.exe",
            GameId::Halo2Amp => "halo2a_tag_test.exe",
            GameId::Halo3 => "halo3_tag_test.exe",
            GameId::Halo3Odst => "atlas_tag_test.exe",
            GameId::HaloReach => "reach_tag_test.exe",
            GameId::Halo4 => "halo4_tag_test.exe",
            GameId::CampaignEvolved => return None,
        })
    }

    fn sapien_takes_scenario_argument(self) -> bool {
        self.launches_scenarios() && self != GameId::HaloCe
    }

    fn launches_scenarios(self) -> bool {
        self.kit_name().is_some()
    }

    fn scenario_startup_command(self) -> &'static str {
        if self == GameId::HaloCe {
            "map_name"
        } else {
            "game_start"
        }
    }

    fn monitor_commands(self) -> &'static [&'static str] {
        match self {
            GameId::Halo2 => &[
                "monitor-bitmaps",
                "monitor-bitmaps-data-and-tags",
                "monitor-models",
                "monitor-structures",
            ],
            GameId::Halo3 | GameId::Halo3Odst => &[
                "monitor-bitmaps",
                "monitor-models",
                "monitor-models-draft",
                "monitor-strings",
                "monitor-structures",
            ],
            GameId::HaloReach => &[
                "monitor-bitmaps",
                "monitor-models",
                "monitor-models-draft",
                "monitor-strings",
            ],
            GameId::Halo4 => &["monitor-bitmaps", "monitor-strings"],
            GameId::HaloCe | GameId::Halo2Amp | GameId::CampaignEvolved => &[],
        }
    }

    fn has_script_docs(self) -> bool {
        !self.is_campaign_evolved()
    }
}

/// The game a folder name stands for: an editing kit's folder (`H3EK`, and
/// older spellings such as `H1EK`) or a game id (`halo3_mcc`), in any case.
/// Users often keep tags under a folder named after the game rather than the
/// kit.
pub(crate) fn game_for_kit_folder(name: &str) -> Option<GameId> {
    let upper = name.to_ascii_uppercase();
    let older = match upper.as_str() {
        "H1EK" | "HALOCEEK" => Some(GameId::HaloCe),
        "HALO2EK" => Some(GameId::Halo2),
        "H2AEK" => Some(GameId::Halo2Amp),
        _ => None,
    };
    older.or_else(|| {
        GameId::ALL.into_iter().find(|game| {
            game.kit_name() == Some(upper.as_str())
                || (game.kit_name().is_some() && game.as_str().eq_ignore_ascii_case(name))
        })
    })
}

/// The game a `-H3EK`-style command-line flag opens, in any case, including
/// the older `-H1EK` and `-H2AEK`.
pub(crate) fn game_for_launch_flag(flag: &str) -> Option<GameId> {
    let kit = flag.strip_prefix('-')?.to_ascii_uppercase();
    match kit.as_str() {
        "H1EK" => Some(GameId::HaloCe),
        "H2AEK" => Some(GameId::Halo2Amp),
        _ => GameId::ALL
            .into_iter()
            .find(|game| game.kit_name() == Some(kit.as_str())),
    }
}

#[cfg(test)]
mod tests;
