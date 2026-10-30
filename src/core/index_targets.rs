//! Shared per-game index metadata for picker presentation and structural edits.
use blam_tags::{game::GameId, index_targets::IndexTargets};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

/// Loose kits pass their definitions root; other sources use bundled schemas.
/// A failed load is retained as an error, never replaced by an empty rule table.
pub(crate) fn load(
    root: Option<&Path>,
    game: Option<GameId>,
    halo5: bool,
) -> Result<Arc<IndexTargets>, String> {
    let root = root
        .map(Path::to_path_buf)
        .unwrap_or_else(crate::core::bundled::locate_definitions_root);
    let path = match game {
        Some(game) => root.join(game.as_str()),
        None if halo5 => root.join(crate::core::format::HALO5_DEFINITIONS),
        None => root,
    };
    load_directory(path, game.is_none() && !halo5)
}

fn load_directory(path: PathBuf, all_games: bool) -> Result<Arc<IndexTargets>, String> {
    type Cached = Result<Arc<IndexTargets>, String>;
    static TABLES: OnceLock<Mutex<HashMap<(PathBuf, bool), Cached>>> = OnceLock::new();
    let mut tables = TABLES
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "Index-target metadata cache is unavailable.".to_owned())?;
    tables
        .entry((path.clone(), all_games))
        .or_insert_with(|| {
            let targets = if all_games {
                IndexTargets::load_all(&path)
            } else {
                IndexTargets::load(&path)
            };
            targets
                .map(Arc::new)
                .map_err(|error| format!("Cannot load index-target metadata: {error}"))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_tags_load_all_bundled_game_rules() {
        load(None, None, false).unwrap();
    }

    #[test]
    fn broken_rules_are_errors_and_custom_roots_are_not_replaced_by_bundled_rules() {
        let root = crate::core::test_kits::unique_temp_path("broken-index-targets");
        let game = root.join("halo3_mcc");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("bad.json"), r#"{"structs":{"bad":{"fields":[{"type":"custom_short_block_index","name":"index","index_target":{"unsupported":"value"}}]}}}"#).unwrap();
        let error = load(Some(&root), Some(GameId::Halo3), false).unwrap_err();
        assert!(error.contains("bad.json"), "{error}");
        assert!(load(Some(&root), Some(GameId::Halo3), false).is_err());
        assert!(
            load(Some(&root), Some(GameId::Halo2), false).is_err(),
            "missing custom schemas must not fall back to bundled ones"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
