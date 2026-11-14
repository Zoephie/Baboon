//! What tag a script argument expects, from the bundled script docs'
//! function signatures, and which tag groups that type accepts.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::core::game::GameId;
use crate::core::group_definitions::GroupDefinitions;

/// The tag groups a script value type accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TagKind {
    /// Exactly these groups.
    Groups(&'static [&'static str]),
    /// Any object group: biped, vehicle, weapon, scenery …
    Objects,
    /// Any shader group.
    Shaders,
    /// Any tag at all.
    Any,
}

/// What a position in a script expects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Expected {
    /// A tag; `name` is the type as the docs spell it, for display.
    Tag { kind: TagKind, name: String },
    /// A value that is not a tag (a string, a number, an AI squad).
    NotTag,
    /// Can't tell: a generic expression, an unknown function, or overloads
    /// that disagree.
    Unknown,
}

/// A script value type, as the docs spell it, sorted into tag kinds.
pub(super) fn classify(value_type: &str) -> Expected {
    let normalized = value_type
        .trim()
        .trim_end_matches("(s)")
        .trim()
        .to_ascii_lowercase()
        .replace(' ', "_");
    let tag = |kind| Expected::Tag {
        kind,
        name: value_type.trim().to_owned(),
    };
    match normalized.as_str() {
        "animation_graph" => tag(TagKind::Groups(&[
            "model_animation_graph",
            "model_animations",
        ])),
        "object_definition" => tag(TagKind::Objects),
        "render_model" => tag(TagKind::Groups(&["render_model"])),
        "sound" => tag(TagKind::Groups(&["sound"])),
        "looping_sound" => tag(TagKind::Groups(&["sound_looping"])),
        "effect" => tag(TagKind::Groups(&["effect"])),
        "damage" => tag(TagKind::Groups(&["damage_effect"])),
        "shader" => tag(TagKind::Shaders),
        "bitmap" => tag(TagKind::Groups(&["bitmap"])),
        "structure_bsp" => tag(TagKind::Groups(&["scenario_structure_bsp"])),
        "style" => tag(TagKind::Groups(&["style"])),
        "actor_variant" => tag(TagKind::Groups(&["actor_variant"])),
        "bink_definition" => tag(TagKind::Groups(&["bink"])),
        "cinematic_definition" => tag(TagKind::Groups(&["cinematic"])),
        "cinematic_scene_definition" => tag(TagKind::Groups(&["cinematic_scene"])),
        "cinematic_transition_definition" => tag(TagKind::Groups(&["cinematic_transition"])),
        "cui_screen_definition" => tag(TagKind::Groups(&["cui_screen"])),
        "sound_budget_reference" => tag(TagKind::Groups(&["sound"])),
        "looping_sound_budget_reference" => tag(TagKind::Groups(&["sound_looping"])),
        "animation_budget_reference" => tag(TagKind::Groups(&["model_animation_graph"])),
        "any_tag" | "any_tag_not_resolving" => tag(TagKind::Any),
        "passthrough" | "expression" | "then" | "else" | "result1" | "result2" | "value"
        | "condition" | "" => Expected::Unknown,
        _ => Expected::NotTag,
    }
}

/// One game's function signatures: each function's overloads, each a list
/// of argument types.
#[derive(Default)]
pub(super) struct Signatures {
    functions: HashMap<String, Vec<Vec<String>>>,
}

impl Signatures {
    /// The game's signatures from the bundled script docs, read once per run.
    /// Empty (everything [`Expected::Unknown`]) if the docs are missing.
    pub fn for_game(game: GameId) -> Arc<Signatures> {
        static CACHE: OnceLock<Mutex<HashMap<GameId, Arc<Signatures>>>> = OnceLock::new();
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache
            .entry(game)
            .or_insert_with(|| {
                let path = crate::core::bundled::locate_help_docs_root()
                    .join(crate::core::bundled::SCRIPT_DOCS_FILE);
                Arc::new(Self::load(&path, game.as_str()).unwrap_or_default())
            })
            .clone()
    }

    fn load(path: &Path, game: &str) -> Option<Signatures> {
        // Docs with another schema are read as none: every argument Unknown.
        let connection = crate::core::bundled::open_script_docs(path).ok()?;
        let mut statement = connection
            .prepare("SELECT name, signature FROM functions WHERE game_id = ?1")
            .ok()?;
        let rows = statement
            .query_map([game], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .ok()?;
        let mut signatures = Signatures::default();
        for (name, signature) in rows.flatten() {
            signatures.add(&name, &signature);
        }
        Some(signatures)
    }

    /// Add `(<return> name <arg> <arg>…)`.
    pub fn add(&mut self, name: &str, signature: &str) {
        let mut types = Vec::new();
        let mut rest = signature;
        while let Some(open) = rest.find('<') {
            let Some(close) = rest[open..].find('>') else {
                break;
            };
            types.push(rest[open + 1..open + close].to_owned());
            rest = &rest[open + close + 1..];
        }
        // The first is the return type.
        let arguments = types.into_iter().skip(1).collect();
        self.functions
            .entry(name.to_ascii_lowercase())
            .or_default()
            .push(arguments);
    }

    /// What argument `index` of `function` expects, in a call passing
    /// `count` arguments when known. Only overloads taking that many are
    /// considered (all of them if none do), and those must agree.
    pub fn expected(&self, function: &str, index: usize, count: Option<usize>) -> Expected {
        let Some(overloads) = self.functions.get(function) else {
            return Expected::Unknown;
        };
        let takes = |arguments: &Vec<String>, count: usize| match arguments.last() {
            Some(last) if last.ends_with("(s)") => count + 1 >= arguments.len(),
            _ => count == arguments.len(),
        };
        let fitting: Vec<_> = match count {
            Some(count) => overloads
                .iter()
                .filter(|arguments| takes(arguments, count))
                .collect(),
            None => Vec::new(),
        };
        let overloads: Vec<_> = if fitting.is_empty() {
            overloads.iter().collect()
        } else {
            fitting
        };
        let mut found: Option<Expected> = None;
        for arguments in overloads {
            let value_type = match arguments.get(index) {
                Some(value_type) => value_type,
                // A trailing `<thing(s)>` repeats.
                None => match arguments.last() {
                    Some(last) if last.ends_with("(s)") => last,
                    _ => continue,
                },
            };
            let expected = classify(value_type);
            match &found {
                None => found = Some(expected),
                Some(previous) if same_kind(previous, &expected) => {}
                Some(_) => return Expected::Unknown,
            }
        }
        found.unwrap_or(Expected::Unknown)
    }
}

fn same_kind(a: &Expected, b: &Expected) -> bool {
    match (a, b) {
        (Expected::Tag { kind: a, .. }, Expected::Tag { kind: b, .. }) => a == b,
        (a, b) => a == b,
    }
}

/// Every object group in `game`: `object`, and the groups whose parent chain
/// reaches it.
pub(super) fn object_groups(game: GameId) -> Arc<HashSet<String>> {
    static CACHE: OnceLock<Mutex<HashMap<GameId, Arc<HashSet<String>>>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .entry(game)
        .or_insert_with(|| {
            let definitions = GroupDefinitions::for_game(game);
            let mut objects: HashSet<String> = definitions
                .groups()
                .filter(|group| {
                    definitions
                        .ancestors(group)
                        .0
                        .iter()
                        .any(|parent| parent.tag == "obje")
                })
                .map(|group| group.name.clone())
                .collect();
            objects.insert("object".to_owned());
            Arc::new(objects)
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halo_2_signatures_say_what_each_tag_argument_expects() {
        let signatures = Signatures::for_game(GameId::Halo2);
        let kind = |function: &str, index| match signatures.expected(function, index, None) {
            Expected::Tag { kind, .. } => Some(kind),
            _ => None,
        };
        assert_eq!(
            kind("custom_animation", 1),
            Some(TagKind::Groups(&[
                "model_animation_graph",
                "model_animations"
            ]))
        );
        assert_eq!(kind("object_type_predict", 0), Some(TagKind::Objects));
        assert_eq!(
            kind("sound_impulse_start", 0),
            Some(TagKind::Groups(&["sound"]))
        );
        assert_eq!(
            signatures.expected("custom_animation", 2, None),
            Expected::NotTag
        );
        assert_eq!(
            signatures.expected("no_such_function", 0, None),
            Expected::Unknown
        );
    }

    #[test]
    fn overloads_that_disagree_are_unknown_and_repeated_arguments_repeat() {
        let mut signatures = Signatures::default();
        signatures.add("pick", "(<void> pick <sound>)");
        signatures.add("pick", "(<void> pick <effect>)");
        signatures.add("many", "(<void> many <short> <object_definition(s)>)");
        assert_eq!(signatures.expected("pick", 0, None), Expected::Unknown);
        assert!(matches!(
            signatures.expected("many", 4, None),
            Expected::Tag {
                kind: TagKind::Objects,
                ..
            }
        ));
    }

    /// Overloads that disagree are told apart by how many arguments the call
    /// passes; a count no overload takes leaves them all in play.
    #[test]
    fn the_argument_count_picks_the_overload() {
        let mut signatures = Signatures::default();
        signatures.add("play", "(<void> play <sound>)");
        signatures.add("play", "(<void> play <effect> <short>)");
        signatures.add("many", "(<void> many <short> <object_definition(s)>)");
        let kind = |index, count| match signatures.expected("play", index, count) {
            Expected::Tag { kind, .. } => Some(kind),
            _ => None,
        };
        assert_eq!(kind(0, Some(1)), Some(TagKind::Groups(&["sound"])));
        assert_eq!(kind(0, Some(2)), Some(TagKind::Groups(&["effect"])));
        assert_eq!(kind(0, Some(5)), None, "no overload takes five");
        assert_eq!(kind(0, None), None);
        assert!(matches!(
            signatures.expected("many", 3, Some(4)),
            Expected::Tag {
                kind: TagKind::Objects,
                ..
            }
        ));

        // Reach's cs_custom_animation takes its graph first, or third after
        // an AI and a flag.
        let reach = Signatures::for_game(GameId::HaloReach);
        let graph = |index, count| matches!(reach.expected("cs_custom_animation", index, Some(count)), Expected::Tag { kind: TagKind::Groups(groups), .. } if groups.contains(&"model_animation_graph"));
        assert!(graph(0, 3) && graph(0, 4) && graph(2, 5) && graph(2, 6));
        assert_eq!(
            reach.expected("cs_custom_animation", 0, None),
            Expected::Unknown,
            "uncounted, the overloads disagree"
        );
    }

    #[test]
    fn object_groups_follow_parents_to_object() {
        let groups = object_groups(GameId::Halo2);
        for group in ["biped", "vehicle", "weapon", "scenery", "object"] {
            assert!(groups.contains(group), "{group}");
        }
        for group in ["render_model", "model_animation_graph", "sound"] {
            assert!(!groups.contains(group), "{group}");
        }
    }
}
