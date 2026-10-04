//! Editing-kit folder resolution, aliases, and game detection.
//! It owns source identity, discovery, indexing, and source-aware reads; editor presentation and application workflow state belong elsewhere.

use super::*;
use crate::core::game::{GameId, game_for_kit_folder, game_for_saved_id};

pub(crate) fn resolve_folder_root(
    selected_root: &Path,
    aliases: &[EkFolderAlias],
) -> Result<FolderRootInfo> {
    let ek_root = detect_ek_root_with_aliases(selected_root, aliases);
    let game = ek_root.as_ref().map(|(_, game)| *game);
    let scan_root = if is_tags_folder(selected_root) {
        selected_root.to_path_buf()
    } else if let Some(tags) = ek_root
        .as_ref()
        .and_then(|(ek_root, _)| other_tags_folder(ek_root, selected_root))
    {
        tags
    } else if let Some((ek_root, _)) = ek_root {
        let tags = ek_root.join("tags");
        if !tags.is_dir() {
            anyhow::bail!(
                "recognized {} as an EK root, but expected tags folder was missing: {}",
                ek_root.display(),
                tags.display()
            );
        }
        tags
    } else {
        find_tags_folder(selected_root).unwrap_or_else(|| selected_root.to_path_buf())
    };
    let label = folder_source_label(selected_root, &scan_root, game);
    Ok(FolderRootInfo {
        scan_root,
        label,
        game,
    })
}

/// The folder under `ek_root` holding `selected`, when its name contains
/// `tags` but isn't the root's own `tags` folder: a kit keeping another tags
/// folder (`tags_moda`) beside the stock one. Picking it, or a folder inside
/// it, browses that folder rather than the root's `tags`.
fn other_tags_folder(ek_root: &Path, selected: &Path) -> Option<PathBuf> {
    let child = selected.strip_prefix(ek_root).ok()?.components().next()?;
    let std::path::Component::Normal(name) = child else {
        return None;
    };
    let name = name.to_str()?;
    if name.eq_ignore_ascii_case("tags") || !name.to_ascii_lowercase().contains("tags") {
        return None;
    }
    let folder = ek_root.join(name);
    folder.is_dir().then_some(folder)
}

fn find_tags_folder(selected_root: &Path) -> Option<PathBuf> {
    if is_tags_folder(selected_root) {
        return Some(selected_root.to_path_buf());
    }

    let direct = selected_root.join("tags");
    if direct.is_dir() {
        return Some(direct);
    }

    WalkDir::new(selected_root)
        .min_depth(1)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| {
            entry.file_type().is_dir()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case("tags"))
        })
        .map(|entry| entry.into_path())
}

fn is_tags_folder(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("tags"))
}

#[cfg(test)]
pub(super) fn detect_ek_game(path: &Path) -> Option<GameId> {
    detect_ek_root_with_aliases(path, &[]).map(|(_, game)| game)
}

pub(super) fn detect_ek_root_with_aliases(
    path: &Path,
    aliases: &[EkFolderAlias],
) -> Option<(PathBuf, GameId)> {
    let built_in = path
        .ancestors()
        .filter_map(|ancestor| {
            ancestor
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| game_for_kit_folder(name).map(|game| (ancestor.to_path_buf(), game)))
        })
        .next();
    if built_in.is_some() {
        return built_in;
    }

    path.ancestors()
        .filter_map(|ancestor| {
            let name = ancestor.file_name().and_then(|name| name.to_str())?;
            let game = alias_folder_game(name, aliases)?;
            Some((ancestor.to_path_buf(), game))
        })
        .next()
}


fn alias_folder_game(name: &str, aliases: &[EkFolderAlias]) -> Option<GameId> {
    aliases.iter().rev().find_map(|alias| {
        let folder_name = alias.folder_name.trim();
        if folder_name.is_empty() || !folder_name.eq_ignore_ascii_case(name) {
            return None;
        }
        game_for_saved_id(&alias.game)
    })
}

fn folder_source_label(
    selected_root: &Path,
    scan_root: &Path,
    game: Option<GameId>,
) -> String {
    let selected_label = selected_root
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| selected_root.display().to_string());
    let mut label = if scan_root != selected_root {
        let scan_name = scan_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("tags");
        format!("{selected_label}/{scan_name}")
    } else {
        selected_label
    };
    if let Some(game) = game {
        label.push_str(&format!(" ({game})"));
    }
    label
}
