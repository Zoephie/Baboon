//! Which scenario palette a tag group lands in, per game, read from that game's scenario definition.
//! It owns the palette table only; how a tag reaches a palette (a Sapien drop, an edit) belongs to the controller.

use super::*;

/// One top-level palette block of a game's scenario: its name and the groups
/// its entries may reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct ScenarioPalette {
    /// The block's name with its annotations stripped, e.g. `vehicle palette`.
    pub(in crate::app) name: String,
    /// Group tags the palette's entry reference allows, e.g. `vehi`. Empty
    /// for a palette whose entries hold no tag reference (weather, acoustics),
    /// and for Halo CE and Halo 2, whose definitions do not record what a
    /// palette entry may reference; neither of those Sapiens takes a dropped
    /// file, so nothing asks.
    pub(in crate::app) groups: Vec<u32>,
}

/// Every top-level palette block of `game`'s scenario, in definition order.
pub(in crate::app) fn scenario_palettes(
    definitions_root: &Path,
    game: &str,
) -> Result<Vec<ScenarioPalette>, String> {
    let path = definitions_root.join(game).join("scenario.json");
    let bytes =
        fs::read(&path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let definition: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
    palettes_in_definition(&definition).map_err(|what| format!("{}: {what}", path.display()))
}

/// The palettes that take `group_tag`, in definition order. Empty when the
/// game's scenario has no palette for the group.
pub(in crate::app) fn palettes_for_group(
    palettes: &[ScenarioPalette],
    group_tag: u32,
) -> Vec<&ScenarioPalette> {
    palettes
        .iter()
        .filter(|palette| palette.groups.contains(&group_tag))
        .collect()
}

/// The palette blocks among the root struct's own fields. Nested blocks are
/// not walked: a palette inside a map-variant or zone block is not what a
/// dropped tag joins.
fn palettes_in_definition(definition: &Value) -> Result<Vec<ScenarioPalette>, String> {
    let root = definition
        .get("block")
        .and_then(Value::as_str)
        .ok_or_else(|| "no root block".to_owned())?;
    let mut palettes = Vec::new();
    for field in block_fields(definition, root)? {
        if field.get("type").and_then(Value::as_str) != Some("block") {
            continue;
        }
        let Some(name) = field
            .get("name")
            .and_then(Value::as_str)
            .map(strip_annotations)
            .filter(|name| is_palette_name(name))
        else {
            continue;
        };
        let Some(block) = field.get("definition").and_then(Value::as_str) else {
            continue;
        };
        let groups = block_fields(definition, block)?
            .iter()
            .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("tag_reference"))
            .flat_map(|entry| {
                entry
                    .get("definition")
                    .and_then(|reference| reference.get("allowed"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(Value::as_str)
            .filter_map(group_tag_from_str)
            .collect();
        palettes.push(ScenarioPalette { name, groups });
    }
    Ok(palettes)
}

/// The fields of the struct a block is made of.
fn block_fields<'a>(definition: &'a Value, block: &str) -> Result<&'a Vec<Value>, String> {
    let struct_name = definition
        .get("blocks")
        .and_then(|blocks| blocks.get(block))
        .and_then(|block| block.get("struct"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("block {block} names no struct"))?;
    definition
        .get("structs")
        .and_then(|structs| structs.get(struct_name))
        .and_then(|layout| layout.get("fields"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("struct {struct_name} has no fields"))
}

/// A field name without Guerilla's inline annotations: `{alias}`, `#help`,
/// `!` and `*` markers, `^` block-name markers and `:units`.
fn strip_annotations(name: &str) -> String {
    let end = name
        .find(['{', '#', '!', '*', '^', ':'])
        .unwrap_or(name.len());
    name[..end].trim().to_owned()
}

/// Ends with the word "palette": `vehicle palette` yes, `map variant
/// palettes` no.
fn is_palette_name(name: &str) -> bool {
    name.rsplit(' ').next() == Some("palette")
}

fn group_tag_from_str(group: &str) -> Option<u32> {
    let bytes: [u8; 4] = group.as_bytes().try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VEHI: u32 = u32::from_be_bytes(*b"vehi");
    const WEAP: u32 = u32::from_be_bytes(*b"weap");
    const BIPD: u32 = u32::from_be_bytes(*b"bipd");
    const SCEN: u32 = u32::from_be_bytes(*b"scen");
    const BLOC: u32 = u32::from_be_bytes(*b"bloc");
    const BITM: u32 = u32::from_be_bytes(*b"bitm");

    fn palette<'a>(palettes: &'a [ScenarioPalette], name: &str) -> &'a ScenarioPalette {
        palettes
            .iter()
            .find(|palette| palette.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "no {name}; have {:?}",
                    palettes.iter().map(|p| p.name.as_str()).collect::<Vec<_>>()
                )
            })
    }

    fn shipped_games() -> Vec<String> {
        let root = locate_definitions_root();
        let mut games = fs::read_dir(&root)
            .unwrap_or_else(|error| panic!("read {}: {error}", root.display()))
            .flatten()
            .filter(|entry| entry.path().join("scenario.json").is_file())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        games.sort();
        assert!(!games.is_empty(), "no game under {}", root.display());
        games
    }

    /// Every shipped game's scenario keeps the four object palettes a dropped
    /// tag most often joins. Where the definition records what an entry may
    /// reference, each names its group. Only the two classic games record no
    /// groups at all, and those Sapiens take no drops.
    #[test]
    fn every_shipped_game_has_the_core_object_palettes() {
        for game in shipped_games() {
            let palettes = scenario_palettes(&locate_definitions_root(), &game)
                .unwrap_or_else(|error| panic!("{game}: {error}"));
            let records_groups = palettes.iter().any(|palette| !palette.groups.is_empty());
            for (name, group) in [
                ("vehicle palette", VEHI),
                ("weapon palette", WEAP),
                ("biped palette", BIPD),
                ("scenery palette", SCEN),
            ] {
                let palette = palette(&palettes, name);
                if records_groups {
                    assert!(
                        palette.groups.contains(&group),
                        "{game}: {name} does not allow the expected group"
                    );
                } else {
                    assert!(palette.groups.is_empty(), "{game}: {name} allows something");
                }
            }
            if !records_groups {
                assert!(
                    ["haloce_mcc", "halo2_mcc"].contains(&game.as_str()),
                    "{game} records no palette groups"
                );
            }
        }
    }

    /// Halo 3's crate palette is the one that takes crates, a bitmap has no
    /// palette anywhere, and the annotated names come out clean.
    #[test]
    fn halo3_palettes_are_named_cleanly_and_route_crates() {
        let palettes = scenario_palettes(&locate_definitions_root(), "halo3_mcc").unwrap();
        assert!(palette(&palettes, "crate palette").groups.contains(&BLOC));
        let for_crates = palettes_for_group(&palettes, BLOC);
        assert_eq!(for_crates.len(), 1);
        assert_eq!(for_crates[0].name, "crate palette");
        assert!(palettes_for_group(&palettes, BITM).is_empty());
        palette(&palettes, "acoustics palette");
        palette(&palettes, "OLD background sound palette");
        // A palette whose entries are settings rather than tags is listed,
        // and takes no group.
        assert!(palette(&palettes, "weather palette").groups.is_empty());
        assert!(
            palettes
                .iter()
                .all(|palette| !palette.name.contains(['{', '!', '#'])),
            "an annotation survived: {:?}",
            palettes.iter().map(|p| p.name.as_str()).collect::<Vec<_>>()
        );
    }

    /// Halo CE has no crate palette (and no crates), but does have actors.
    #[test]
    fn halo_ce_has_actors_but_no_crate_palette() {
        let palettes = scenario_palettes(&locate_definitions_root(), "haloce_mcc").unwrap();
        assert!(palettes_for_group(&palettes, BLOC).is_empty());
        palette(&palettes, "actor palette");
    }

    /// A `#help` annotation is cut off with its text.
    #[test]
    fn halo4_playtest_palette_loses_its_help_text() {
        let palettes = scenario_palettes(&locate_definitions_root(), "halo4_mcc").unwrap();
        palette(&palettes, "Playtest req palette");
    }

    #[test]
    fn a_missing_game_names_the_file_it_wanted() {
        let error = scenario_palettes(&locate_definitions_root(), "no_such_game").unwrap_err();
        assert!(error.contains("no_such_game"), "{error}");
        assert!(error.contains("scenario.json"), "{error}");
    }

    #[test]
    fn plural_palettes_and_nested_names_are_not_palettes() {
        assert!(is_palette_name("vehicle palette"));
        assert!(is_palette_name(&strip_annotations(
            "acoustics palette{background sound palette}"
        )));
        assert!(!is_palette_name("map variant palettes"));
        assert!(!is_palette_name("palette index"));
        assert_eq!(
            strip_annotations("Playtest req palette#requisition for SvE"),
            "Playtest req palette"
        );
        assert_eq!(
            strip_annotations("sound environment palette!"),
            "sound environment palette"
        );
    }
}
