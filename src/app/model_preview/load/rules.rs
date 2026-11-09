//! What each game's loads do differently, as one value per game. Each game's
//! folder defines its own, `load/games.rs` picks it by the kit's game, and the
//! shared readers read its flags rather than naming a game.

use super::super::gen3::rules::Gen3Rules;
use blam_tags::TagFile;

/// One game's choices, flag by flag.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GameRules {
    /// Whether a unit holds the settings' weapons and its variants attach
    /// their children: every game past Halo CE.
    pub(crate) unit_weapons: bool,
    /// Whether a held weapon places the hands by the graph's legacy
    /// `weapon ik`. Halo 4's graphs use data-driven animation, which places
    /// them by IK sets and chains (`biped_postprocess_node_matrices`, X360
    /// 0x84B1A738).
    pub(crate) legacy_weapon_ik: bool,
    /// Whether a variant's NONE instance group shows every instance, as Reach
    /// and Halo 4 (the 360's 0x8325C1B8) do.
    pub(crate) none_instance_takes_all: bool,
    /// The hand marker a unit names for a weapon of its own, before its
    /// `right_hand_node` / `left_hand_node`: Halo 4's `weapon specific
    /// markers`.
    pub(crate) weapon_hand_marker: Option<WeaponHandMarker>,
    /// Whether the unit's IK reaches for its held weapon's markers where they
    /// ride: Reach's IK chains and Halo 4's IK sets.
    pub(crate) ik_markers_on_weapons: bool,
    /// The Halo 3 family's choices.
    pub(crate) gen3: Gen3Rules,
}

/// A unit's own hand marker for a weapon: the unit, the weapon, and whether
/// it goes in the left hand.
pub(crate) type WeaponHandMarker = fn(&TagFile, &TagFile, bool) -> Option<String>;

impl GameRules {
    /// A game with none of its own: what the shared readers do unless told.
    pub(crate) const DEFAULT: Self = Self {
        unit_weapons: false,
        legacy_weapon_ik: true,
        none_instance_takes_all: false,
        weapon_hand_marker: None,
        ik_markers_on_weapons: false,
        gen3: Gen3Rules::DEFAULT,
    };
}
