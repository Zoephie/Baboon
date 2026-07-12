//! Editing-kit folder resolution, aliases, and game detection.
//! It owns source identity, discovery, indexing, and source-aware reads; editor presentation and application workflow state belong elsewhere.

use super::*;

pub(crate) fn resolve_folder_root(
    selected_root: &Path,
    aliases: &[EkFolderAlias],
) -> Result<FolderRootInfo> {
    let ek_root = detect_ek_root_with_aliases(selected_root, aliases);
    let game = ek_root.as_ref().map(|(_, game)| *game);
    let scan_root = if is_tags_folder(selected_root) {
        selected_root.to_path_buf()
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
pub(super) fn detect_ek_game(path: &Path) -> Option<&'static str> {
    detect_ek_root_with_aliases(path, &[]).map(|(_, game)| game)
}

pub(super) fn detect_ek_root_with_aliases(
    path: &Path,
    aliases: &[EkFolderAlias],
) -> Option<(PathBuf, &'static str)> {
    let built_in = path
        .ancestors()
        .filter_map(|ancestor| {
            ancestor
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| ek_folder_game(name).map(|game| (ancestor.to_path_buf(), game)))
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

fn ek_folder_game(name: &str) -> Option<&'static str> {
    // Recognize both the editing-kit folder names (e.g. `H3EK`) and the
    // canonical game-id folder names (e.g. `halo3_mcc`) — users often keep tags
    // under a folder named after the game, not the EK.
    match name.to_ascii_uppercase().as_str() {
        "HCEEK" | "H1EK" | "HALOCEEK" | "HALOCE_MCC" => Some("haloce_mcc"),
        "H2EK" | "HALO2EK" | "HALO2_MCC" => Some("halo2_mcc"),
        "HREK" | "HALOREACH_MCC" => Some("haloreach_mcc"),
        "H4EK" | "HALO4_MCC" => Some("halo4_mcc"),
        "H3ODSTEK" | "HALO3ODST_MCC" => Some("halo3odst_mcc"),
        "H3EK" | "HALO3_MCC" => Some("halo3_mcc"),
        "H2AMPEK" | "H2AEK" | "HALO2AMP_MCC" => Some("halo2amp_mcc"),
        _ => None,
    }
}

fn alias_folder_game(name: &str, aliases: &[EkFolderAlias]) -> Option<&'static str> {
    aliases.iter().rev().find_map(|alias| {
        let folder_name = alias.folder_name.trim();
        if folder_name.is_empty() || !folder_name.eq_ignore_ascii_case(name) {
            return None;
        }
        supported_ek_game_id(&alias.game)
    })
}

pub(crate) fn supported_ek_game_id(game: &str) -> Option<&'static str> {
    SUPPORTED_EK_GAMES
        .iter()
        .find_map(|(_, id)| id.eq_ignore_ascii_case(game).then_some(*id))
}

fn folder_source_label(
    selected_root: &Path,
    scan_root: &Path,
    game: Option<&'static str>,
) -> String {
    let selected_label = selected_root
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| selected_root.display().to_string());
    let mut label = if scan_root != selected_root {
        format!("{selected_label}/tags")
    } else {
        selected_label
    };
    if let Some(game) = game {
        label.push_str(&format!(" ({game})"));
    }
    label
}
