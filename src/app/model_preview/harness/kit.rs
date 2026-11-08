//! Where a test finds a game's editing kit, and the preview of one of its
//! tags as the panel loads it.
//!
//! Every kit test asks [`kit_root`] or [`kit_source`], so a kit that is not
//! set up skips the test the same way everywhere, with a line that names the
//! variable to set. A kit's root is its `tags` folder, named by an
//! environment variable, never a path on one machine.

use std::path::{Path, PathBuf};

use crate::app::model_preview::load::{PreviewLoadSettings, load_model_preview};
use crate::app::model_preview::ModelPreviewData;
use crate::core::format::TagNameIndex;
use crate::core::game::GameId;
use crate::core::source::{TagEntry, TagEntryLocation, TagSource};
use crate::app::export::load_referenced_tag_from_source;

/// The variable that names `game`'s editing kit's `tags` folder.
fn kit_variable(game: GameId) -> &'static str {
    match game {
        GameId::HaloCe => "BLAM_TEST_HCEEK",
        GameId::Halo2 => "BLAM_TEST_H2EK",
        GameId::Halo3 => "BLAM_TEST_H3EK",
        GameId::Halo3Odst => "BLAM_TEST_ODSTEK",
        GameId::HaloReach => "BLAM_TEST_HREK",
        GameId::Halo4 => "BLAM_TEST_H4EK",
        other => panic!("no editing kit variable for {other:?}"),
    }
}

/// `game`'s editing kit's `tags` folder, or `None` (saying which variable to
/// set) when its variable is unset, empty or not a folder, so the test skips.
pub(in crate::app::model_preview) fn kit_root(game: GameId) -> Option<PathBuf> {
    let variable = kit_variable(game);
    let root = std::env::var_os(variable).filter(|value| !value.is_empty()).map(PathBuf::from);
    match root {
        Some(root) if root.is_dir() => Some(root),
        _ => {
            eprintln!("kit skipped: set {variable} to a {game:?} editing kit's tags folder");
            None
        }
    }
}

/// `game`'s editing kit as a loose-folder source, read against this
/// repository's definitions, or `None` as [`kit_root`] skips.
pub(in crate::app::model_preview) fn kit_source(game: GameId) -> Option<TagSource> {
    Some(TagSource::LooseFolder {
        root: kit_root(game)?,
        game: Some(game),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    })
}

/// The preview of the kit tag at `path`, as the panel loads it.
pub(in crate::app::model_preview) fn kit_preview(
    root: impl AsRef<Path>,
    game: GameId,
    path: &str,
    extension: &str,
    group: &[u8; 4],
) -> ModelPreviewData {
    kit_preview_with(root, game, path, extension, group, &PreviewLoadSettings::default())
}

/// [`kit_preview`] with the panel's load settings given.
pub(in crate::app::model_preview) fn kit_preview_with(
    root: impl AsRef<Path>,
    game: GameId,
    path: &str,
    extension: &str,
    group: &[u8; 4],
    settings: &PreviewLoadSettings,
) -> ModelPreviewData {
    let root = root.as_ref();
    let source = TagSource::LooseFolder {
        root: root.to_path_buf(),
        game: Some(game),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let TagSource::LooseFolder { definitions_root, .. } = &source else { unreachable!() };
    let names = TagNameIndex::load_game(definitions_root, game).unwrap();
    let tag = load_referenced_tag_from_source(&source, path, extension, group).unwrap();
    let key = format!("{path}.{extension}");
    let entry = TagEntry {
        key: key.clone(),
        display_path: key.clone(),
        group_tag: u32::from_be_bytes(*group),
        group_name: Some(extension.to_owned()),
        location: TagEntryLocation::LooseFile(root.join(key.replace('\\', "/"))),
    };
    load_model_preview(&tag, &entry, &names, Some(&source), settings).unwrap()
}
