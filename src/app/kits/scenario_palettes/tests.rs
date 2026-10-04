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

fn shipped_games() -> Vec<GameId> {
    let root = locate_definitions_root();
    let mut games = fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("read {}: {error}", root.display()))
        .flatten()
        .filter(|entry| entry.path().join("scenario.json").is_file())
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            GameId::from_id(&name).unwrap_or_else(|| panic!("definitions/{name} names no game"))
        })
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
        let palettes = scenario_palettes(&locate_definitions_root(), game)
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
    let palettes = scenario_palettes(&locate_definitions_root(), GameId::Halo3).unwrap();
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
    let palettes = scenario_palettes(&locate_definitions_root(), GameId::HaloCe).unwrap();
    assert!(palettes_for_group(&palettes, BLOC).is_empty());
    palette(&palettes, "actor palette");
}

/// A `#help` annotation is cut off with its text.
#[test]
fn halo4_playtest_palette_loses_its_help_text() {
    let palettes = scenario_palettes(&locate_definitions_root(), GameId::Halo4).unwrap();
    palette(&palettes, "Playtest req palette");
}

#[test]
fn a_missing_definition_names_the_file_it_wanted() {
    let empty = crate::test_kits::unique_temp_dir("no-definitions");
    let error = scenario_palettes(&empty, GameId::Halo3).unwrap_err();
    let _ = fs::remove_dir_all(&empty);
    assert!(error.contains("halo3_mcc"), "{error}");
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
