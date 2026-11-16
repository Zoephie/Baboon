//! Script tags (`scenario_hs_source_file`, Halo 2 to Reach) and the files
//! and scenarios around them, for opening a script in VS Code and
//! recompiling it.
//!
//! A script tag holds a copy of one script; the kit's tools compile the
//! `.hsc` file beside it under `data\`: `tags\levels\a\scripts\
//! a_mission.scenario_hs_source_file` compiles from `data\levels\a\scripts\
//! a_mission.hsc`. A scenario lists the script tags it compiles in
//! `scenario resources[0]/script source`.

use std::path::{Path, PathBuf};

use blam_tags::{TagFieldData, TagFile};

use crate::core::game::GameId;
use crate::core::source::KitLayout;
use crate::core::tag_path;

/// The group of a tag holding one script.
pub(crate) const SCRIPT_TAG_GROUP: &str = "scenario_hs_source_file";

/// Whether Baboon can open and recompile `game`'s script tags: the games
/// whose script tags sit beside a `data\` file (Halo 2, 3, ODST, Reach).
pub(crate) fn supports_script_tags(game: Option<GameId>) -> bool {
    matches!(
        game,
        Some(GameId::Halo2 | GameId::Halo3 | GameId::Halo3Odst | GameId::HaloReach)
    )
}

/// The `.hsc` file under `data\` the script tag at `display_path` (with its
/// extension) compiles from. `None` when the kit has no folder around its
/// tags folder.
pub(crate) fn data_file(tags_root: &Path, display_path: &str) -> Option<PathBuf> {
    let layout = KitLayout::from_tags_folder(tags_root)?;
    let stem = tag_path::without_extension(display_path);
    let folder = tag_path::parent(stem).unwrap_or("");
    let file = format!("{}.hsc", tag_path::name(stem));
    Some(layout.data.join(tag_path::to_relative(folder)).join(file))
}

/// The script tags a scenario lists, as tag paths with their extension.
pub(crate) fn listed_scripts(scenario: &TagFile) -> Vec<String> {
    let root = scenario.root();
    let Some(block) = root
        .field_path("scenario resources[0]/script source")
        .and_then(|field| field.as_block())
    else {
        return Vec::new();
    };
    (0..block.len())
        .filter_map(|index| match block.element(index)?.field_path("reference")?.value()? {
            TagFieldData::TagReference(reference) => {
                let (_, path) = reference.group_tag_and_name?;
                (!path.is_empty()).then(|| format!("{path}.{SCRIPT_TAG_GROUP}"))
            }
            _ => None,
        })
        .collect()
}

/// The folder a script tag's scenario would be in: the one holding its
/// `scripts` folder, or its own when it isn't in one.
pub(crate) fn scenario_folder(display_path: &str) -> &str {
    let folder = tag_path::parent(display_path).unwrap_or("");
    match tag_path::parent(folder) {
        Some(level) if tag_path::name(folder).eq_ignore_ascii_case("scripts") => level,
        _ => folder,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definitions() -> PathBuf {
        crate::core::bundled::locate_definitions_root()
    }

    #[test]
    fn a_script_tags_data_file_is_beside_it_under_data() {
        let kit = Path::new("kit");
        assert_eq!(
            data_file(&kit.join("tags"), r"levels\solo\m10\scripts\m10_mission.scenario_hs_source_file"),
            Some(kit.join("data").join("levels").join("solo").join("m10").join("scripts").join("m10_mission.hsc"))
        );
    }

    #[test]
    fn a_scenario_lists_its_script_tags() {
        let mut tag = TagFile::new(definitions().join("halo3_mcc/scenario.json")).unwrap();
        assert!(listed_scripts(&tag).is_empty());
        {
            let mut root = tag.root_mut();
            let mut resources = root.field_mut("scenario resources").unwrap();
            let mut resources = resources.as_block_mut().unwrap();
            let index = resources.add_element();
            let mut element = resources.element_mut(index).unwrap();
            let mut sources = element.field_mut("script source").unwrap();
            sources.as_block_mut().unwrap().add_element();
        }
        tag.root_mut()
            .field_path_mut("scenario resources[0]/script source[0]/reference")
            .unwrap()
            .set(TagFieldData::TagReference(blam_tags::TagReferenceData {
                group_tag_and_name: Some((u32::from_be_bytes(*b"hsc*"), r"levels\a\scripts\a_mission".to_owned())),
            }))
            .unwrap();
        assert_eq!(listed_scripts(&tag), [r"levels\a\scripts\a_mission.scenario_hs_source_file"]);
    }

    #[test]
    fn a_scripts_scenario_is_beside_its_scripts_folder() {
        assert_eq!(scenario_folder(r"levels\a\scripts\a_mission.scenario_hs_source_file"), r"levels\a");
        assert_eq!(scenario_folder(r"globals\global_scripts.scenario_hs_source_file"), "globals");
    }

    #[test]
    fn halo_2_to_reach_have_script_tags_to_open() {
        assert!(supports_script_tags(Some(GameId::Halo2)));
        assert!(supports_script_tags(Some(GameId::HaloReach)));
        assert!(!supports_script_tags(Some(GameId::HaloCe)));
        assert!(!supports_script_tags(Some(GameId::Halo4)));
    }
}
