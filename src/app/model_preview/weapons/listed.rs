//! A unit's weapons, listed off the load: the picker asks for them when it
//! first opens, as listing reads every weapon in the kit.

use super::Equippable;
use crate::app::model_preview::ce::weapons::{CeEquippable, list_ce_weapons};
use crate::app::model_preview::load::{TagReader, read_opened_tag};
use crate::core::game::GameId;
use crate::core::source::{TagEntry, TagSource};

/// What a unit can hold, by its game's rules.
#[derive(Debug)]
pub(crate) enum ListedWeapons {
    /// Halo 2's and every later game's.
    Unit(Vec<Equippable>),
    HaloCe(Vec<CeEquippable>),
}

/// The weapons the unit `entry` names can hold, read as the panel opened it
/// (`edits` its unsaved ones). A unit that will not read holds none.
pub(in crate::app::model_preview) fn list_weapons(
    source: &TagSource,
    entry: &TagEntry,
    edits: Option<Result<Vec<u8>, String>>,
) -> ListedWeapons {
    let reader = TagReader::new(source);
    let unit = read_opened_tag(source, entry, edits).ok();
    if source.game() == Some(GameId::HaloCe) {
        let listed = unit.map(|unit| list_ce_weapons(&reader, &unit));
        ListedWeapons::HaloCe(listed.unwrap_or_default())
    } else {
        let listed = unit.map(|unit| super::list_unit_weapons(&reader, &unit));
        ListedWeapons::Unit(listed.unwrap_or_default())
    }
}
