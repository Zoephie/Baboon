//! Which tag groups can hold a tag reference at all.
//!
//! Building the reference index reads every tag, and a tag without a
//! dependency list has to be parsed in full. Some groups (bitmaps, collision
//! and physics models, string lists, …) have no `tag_reference` field
//! anywhere in their definition or their parents', so they never reference
//! anything and need not be read. Halo 2's bitmaps alone were about a sixth
//! of a full index build.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::core::game::GameId;

/// Whether a `group` (its name, as tag files are named: `bitmap`) tag in
/// `game` can never hold a tag reference. `false` whenever that can't be
/// told from the definitions.
pub(crate) fn cannot_reference(game: GameId, group: &str) -> bool {
    reference_free_groups(game).contains(&group.to_ascii_lowercase())
}

fn reference_free_groups(game: GameId) -> Arc<HashSet<String>> {
    static CACHE: OnceLock<Mutex<HashMap<GameId, Arc<HashSet<String>>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .entry(game)
        .or_insert_with(|| Arc::new(read(&crate::core::bundled::locate_definitions_root().join(game.as_str()))))
        .clone()
}

/// The groups in a game's definitions folder with no `tag_reference` field in
/// themselves or any parent.
fn read(folder: &Path) -> HashSet<String> {
    let Ok(listing) = std::fs::read_dir(folder) else {
        return HashSet::new();
    };
    // Group tag → (name, parent group tag, has a reference of its own).
    let mut groups: HashMap<String, (String, Option<String>, bool)> = HashMap::new();
    for entry in listing.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with('_')) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(definition) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let field = |key: &str| definition.get(key).and_then(|value| value.as_str()).map(str::to_owned);
        let (Some(name), Some(tag)) = (field("name"), field("tag")) else { continue };
        groups.insert(tag, (name, field("parent_tag"), text.contains("\"tag_reference\"")));
    }
    let can_reference = |tag: &str| {
        let mut current = Some(tag);
        for _ in 0..16 {
            let Some((_, parent, own)) = current.and_then(|tag| groups.get(tag)) else {
                // A parent the definitions don't describe: assume it might.
                return current.is_some();
            };
            if *own {
                return true;
            }
            current = parent.as_deref();
        }
        true
    };
    groups
        .iter()
        .filter(|(tag, _)| !can_reference(tag))
        .map(|(_, (name, _, _))| name.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_without_tag_reference_fields_never_reference_anything() {
        for group in ["bitmap", "collision_model", "physics_model", "multilingual_unicode_string_list"] {
            assert!(cannot_reference(GameId::Halo2, group), "{group}");
        }
        // Their own fields or a parent's (a biped's come from unit and object).
        for group in ["biped", "scenario", "shader", "render_model", "sound", "scenario_structure_bsp"] {
            assert!(!cannot_reference(GameId::Halo2, group), "{group}");
        }
        assert!(!cannot_reference(GameId::Halo2, "not_a_group"));
    }

    #[test]
    fn later_games_skip_bitmaps_and_never_scenarios() {
        for game in [GameId::Halo3, GameId::Halo3Odst, GameId::HaloReach] {
            assert!(cannot_reference(game, "bitmap"), "{game:?}");
        }
        for game in [GameId::HaloCe, GameId::Halo2, GameId::Halo3, GameId::HaloReach] {
            assert!(!cannot_reference(game, "scenario"), "{game:?}");
        }
    }
}
