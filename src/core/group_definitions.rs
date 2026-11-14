//! Each game's tag groups as its definition files describe them: name,
//! four-character tag, parent, and whether the definition itself holds a tag
//! reference. Read once per game, and shared by everything that follows a
//! group's `parent_tag` chain in core.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;

use crate::core::game::GameId;

/// One group's definition.
#[derive(Clone, Debug)]
pub(crate) struct GroupDefinition {
    /// The group's name, as tag files are named: `biped`.
    pub name: String,
    /// Its four-character tag: `bipd`.
    pub tag: String,
    /// The parent's tag, or now and then its name.
    parent: Option<String>,
    /// Whether the definition has a `tag_reference` field of its own.
    pub holds_reference: bool,
}

/// The groups of one game's definitions folder.
#[derive(Debug, Default)]
pub(crate) struct GroupDefinitions {
    by_tag: HashMap<String, GroupDefinition>,
    tag_by_name: HashMap<String, String>,
}

/// The fields read from a definition; the rest (structs, often megabytes) is
/// skipped by the parser.
#[derive(Deserialize)]
struct Head {
    name: Option<String>,
    tag: Option<String>,
    parent_tag: Option<String>,
}

impl GroupDefinitions {
    /// `game`'s groups, from the bundled definitions, read once per run.
    pub fn for_game(game: GameId) -> Arc<Self> {
        static CACHE: OnceLock<Mutex<HashMap<GameId, Arc<GroupDefinitions>>>> = OnceLock::new();
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache
            .entry(game)
            .or_insert_with(|| {
                Arc::new(Self::read(
                    &crate::core::bundled::locate_definitions_root().join(game.as_str()),
                ))
            })
            .clone()
    }

    /// The groups in a game's definitions folder. Files starting with `_`
    /// are shared pieces, not groups.
    pub fn read(folder: &Path) -> Self {
        let mut definitions = Self::default();
        let Ok(listing) = std::fs::read_dir(folder) else {
            return definitions;
        };
        for entry in listing.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json")
                || path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('_'))
            {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(head) = serde_json::from_str::<Head>(&text) else {
                continue;
            };
            let (Some(name), Some(tag)) = (head.name, head.tag) else {
                continue;
            };
            definitions
                .tag_by_name
                .insert(name.to_ascii_lowercase(), tag.clone());
            definitions.by_tag.insert(
                tag.clone(),
                GroupDefinition {
                    name,
                    tag,
                    parent: head.parent_tag.filter(|parent| !parent.is_empty()),
                    holds_reference: text.contains("\"tag_reference\""),
                },
            );
        }
        definitions
    }

    pub fn groups(&self) -> impl Iterator<Item = &GroupDefinition> {
        self.by_tag.values()
    }

    /// The group's parents, nearest first, and whether the chain ends where
    /// it should: `false` when a parent isn't among the definitions (or the
    /// chain loops).
    pub fn ancestors(&self, group: &GroupDefinition) -> (Vec<&GroupDefinition>, bool) {
        let mut ancestors = Vec::new();
        let mut parent = group.parent.as_deref();
        for _ in 0..16 {
            let Some(key) = parent else {
                return (ancestors, true);
            };
            // A parent is a four-character tag, or now and then a name.
            let tag = self
                .tag_by_name
                .get(&key.to_ascii_lowercase())
                .map(String::as_str)
                .unwrap_or(key);
            let Some(found) = self.by_tag.get(tag) else {
                return (ancestors, false);
            };
            ancestors.push(found);
            parent = found.parent.as_deref();
        }
        (ancestors, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_biped_descends_from_unit_and_object() {
        let definitions = GroupDefinitions::for_game(GameId::Halo2);
        let biped = definitions
            .groups()
            .find(|group| group.name == "biped")
            .expect("biped");
        let (ancestors, complete) = definitions.ancestors(biped);
        let names: Vec<_> = ancestors.iter().map(|group| group.name.as_str()).collect();
        assert_eq!(names, ["unit", "object"]);
        assert!(complete);
    }
}
