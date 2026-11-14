//! Which tag groups can hold a tag reference at all.
//!
//! Building the reference index reads every tag, and a tag without a
//! dependency list has to be parsed in full. Some groups (bitmaps, collision
//! and physics models, string lists, …) have no `tag_reference` field
//! anywhere in their definition or their parents', so they never reference
//! anything and need not be read. Halo 2's bitmaps alone were about a sixth
//! of a full index build.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use crate::core::game::GameId;
use crate::core::group_definitions::GroupDefinitions;

/// Whether a `group` (its name, as tag files are named: `bitmap`) tag in
/// `game` can never hold a tag reference. `false` whenever that can't be
/// told from the definitions.
pub(crate) fn cannot_reference(game: GameId, group: &str) -> bool {
    reference_free_groups(game).contains(&group.to_ascii_lowercase())
}

fn reference_free_groups(game: GameId) -> Arc<HashSet<String>> {
    static CACHE: OnceLock<Mutex<HashMap<GameId, Arc<HashSet<String>>>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .entry(game)
        .or_insert_with(|| Arc::new(reference_free(&GroupDefinitions::for_game(game))))
        .clone()
}

/// The groups with no `tag_reference` field in themselves or any parent. A
/// parent the definitions don't describe might have one.
fn reference_free(definitions: &GroupDefinitions) -> HashSet<String> {
    definitions
        .groups()
        .filter(|group| {
            let (ancestors, complete) = definitions.ancestors(group);
            complete
                && !group.holds_reference
                && !ancestors.iter().any(|parent| parent.holds_reference)
        })
        .map(|group| group.name.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_without_tag_reference_fields_never_reference_anything() {
        for group in [
            "bitmap",
            "collision_model",
            "physics_model",
            "multilingual_unicode_string_list",
        ] {
            assert!(cannot_reference(GameId::Halo2, group), "{group}");
        }
        // Their own fields or a parent's (a biped's come from unit and object).
        for group in [
            "biped",
            "scenario",
            "shader",
            "render_model",
            "sound",
            "scenario_structure_bsp",
        ] {
            assert!(!cannot_reference(GameId::Halo2, group), "{group}");
        }
        assert!(!cannot_reference(GameId::Halo2, "not_a_group"));
    }

    #[test]
    fn later_games_skip_bitmaps_and_never_scenarios() {
        for game in [GameId::Halo3, GameId::Halo3Odst, GameId::HaloReach] {
            assert!(cannot_reference(game, "bitmap"), "{game:?}");
        }
        for game in [
            GameId::HaloCe,
            GameId::Halo2,
            GameId::Halo3,
            GameId::HaloReach,
        ] {
            assert!(!cannot_reference(game, "scenario"), "{game:?}");
        }
    }
}
