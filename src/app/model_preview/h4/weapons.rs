//! How a Halo 4 unit holds a weapon.

use super::super::common::fields::{elements, holder_of};
use super::super::weapons::string_id_in;
use blam_tags::TagFile;

/// Halo 4's `unit_show_weapon_from_inventory` (X360 0x84AB9B40): the unit's
/// `weapon specific markers` entry for the weapon's class and name gives the
/// hand's marker before `right_hand_node` / `left_hand_node` do. No kit unit
/// fills the block.
pub(in crate::app::model_preview) fn weapon_specific_marker(
    unit: &TagFile,
    weapon: &TagFile,
    left_hand: bool,
) -> Option<String> {
    let holder = holder_of(unit.root(), "weapon specific markers")?;
    let (class, name) = (
        string_id_in(weapon, "weapon class"),
        string_id_in(weapon, "weapon name"),
    );
    let entry = elements(holder, "weapon specific markers")
        .into_iter()
        .find(|entry| {
            entry.read_string_id("weapon class").unwrap_or_default() == class
                && entry.read_string_id("weapon name").unwrap_or_default() == name
        })?;
    entry
        .read_string_id(if left_hand {
            "left hand marker"
        } else {
            "right hand marker"
        })
        .filter(|marker| !marker.is_empty())
}
