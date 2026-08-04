//! Creating a Campaign Evolved tag in a group the game ships no instance of.
//!
//! The dialog offers all 139 groups the definitions define, but the game ships a
//! tag for only 101 of them. Creation used to require an existing same-group
//! `.uasset` to donate the UE5 package structure, so the other 38 -- among them
//! `cinematic_scene`, the reported case -- could not be created at all. The
//! donor is now allowed to come from another group, because everything
//! group-shaped in the wrapper is derived from the destination package path.

use super::*;

fn definitions() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions")
}

fn definition(group: &str) -> std::path::PathBuf {
    definitions().join("haloce_evolved").join(format!("{group}.json"))
}

/// A mounted container tag, as the browser would have indexed it.
fn container_entry(container: usize, path: &str, group: &str) -> TagEntry {
    let group_tag = TagFile::new(definition(group))
        .unwrap_or_else(|e| panic!("{group} has no CE schema: {e}"))
        .header
        .group_tag;
    TagEntry {
        key: format!("ublock:pakchunk0:{path}"),
        display_path: format!("{path}.{group}"),
        group_tag,
        group_name: Some(group.to_owned()),
        location: TagEntryLocation::Container {
            container,
            rel_path: format!("Tags/{path}-{group}.ubulk"),
        },
    }
}

fn group_tag_of(group: &str) -> u32 {
    TagFile::new(definition(group))
        .unwrap_or_else(|e| panic!("{group} has no CE schema: {e}"))
        .header
        .group_tag
}

/// `TagFile::new` zeroes the whole file-header generation, and nothing stamped
/// Campaign Evolved's over it — the CE arm was simply missing from
/// `apply_editing_kit_mcc_header`, which only knew the five MCC games.
#[test]
fn a_new_campaign_evolved_tag_carries_the_games_build_version() {
    let mut tag = TagFile::new(definition("cinematic_scene")).expect("build from the CE schema");
    assert_eq!(tag.header.build_version, 0, "TagFile::new starts at zero");
    apply_editing_kit_mcc_header(&mut tag, CAMPAIGN_EVOLVED_GAME).expect("CE is a known game");
    assert_eq!(tag.header.build_version, CAMPAIGN_EVOLVED_BUILD_VERSION);
}

/// Campaign Evolved must not pick up the MCC defaults: `version = u32::MAX` is
/// what guerilla writes when it has no source revision, and CE is not read by
/// guerilla at all.
#[test]
fn campaign_evolved_does_not_take_the_mcc_defaults() {
    let mut ce = TagFile::new(definition("cinematic_scene")).expect("build from the CE schema");
    apply_editing_kit_mcc_header(&mut ce, CAMPAIGN_EVOLVED_GAME).expect("CE is a known game");
    assert_ne!(ce.header.version, u32::MAX, "that is guerilla's sentinel");
    assert_eq!(ce.header.build_number, 0);
}

/// Adding the Campaign Evolved arm must not have moved the MCC ones.
///
/// `apply_editing_kit_mcc_header` is the only function the CE creation path
/// shares with the five editing-kit games, so it is the only place a CE change
/// could reach H2EK/H3EK/H3ODSTEK/H4EK/H2AMPEK tags. The generations are pinned
/// here as well as in `editing_kit_header_defaults_match_profile_generations`
/// because that test asserts on serialized bytes, and this one is about the
/// branch: CE takes an early return, and every other game must fall through it
/// untouched.
#[test]
fn the_campaign_evolved_arm_leaves_the_editing_kit_games_alone() {
    for (game, build_number) in [
        ("halo3_mcc", 1),
        ("halo3odst_mcc", 1),
        ("haloreach_mcc", 2),
        ("halo4_mcc", 2),
        ("halo2amp_mcc", 2),
    ] {
        let schema = definitions().join(game).join("globals.json");
        let mut tag = TagFile::new(&schema).unwrap_or_else(|e| panic!("{game} globals: {e}"));
        apply_editing_kit_mcc_header(&mut tag, game).unwrap_or_else(|e| panic!("{game}: {e}"));
        assert_eq!(tag.header.build_version, 1, "{game} build_version");
        assert_eq!(tag.header.build_number, build_number, "{game} build_number");
        assert_eq!(tag.header.version, u32::MAX, "{game} version");
        assert_ne!(
            tag.header.build_version, CAMPAIGN_EVOLVED_BUILD_VERSION,
            "{game} took Campaign Evolved's stamp"
        );
    }
}

/// The CE arm is an early return on one exact string, so it must not have
/// widened what the function accepts. A game with no known generation is still
/// an error rather than a tag stamped with someone else's.
#[test]
fn a_game_with_no_known_generation_is_still_rejected() {
    for game in ["haloce_mcc", "halo2_mcc", "", "haloce_evolved_x"] {
        let mut tag = TagFile::new(definition("cinematic_scene")).expect("any tag will do");
        assert!(
            apply_editing_kit_mcc_header(&mut tag, game).is_err(),
            "{game:?} should have no known tag-header defaults"
        );
        assert_eq!(tag.header.build_version, 0, "{game:?} was stamped anyway");
    }
}

/// Every group the dialog offers must have a schema on disk, or the group list
/// and the creation path disagree about what is creatable.
#[test]
fn every_offered_campaign_evolved_group_has_a_schema() {
    let groups = load_new_tag_groups("haloce_evolved").expect("the CE group table loads");
    assert!(
        groups.len() > 100,
        "expected the full CE group table, got {}",
        groups.len()
    );
    for group in &groups {
        assert!(group.schema_path.is_file(), "{} has no schema", group.name);
    }
    assert!(
        groups.iter().any(|g| g.name == "cinematic_scene"),
        "cinematic_scene is offered by the dialog"
    );
}

/// A group the game *does* ship donates its own wrapper, which is both the
/// closest match and the only donor a binding can be carried through.
#[test]
fn a_shipped_group_donates_its_own_wrapper() {
    let entries = vec![
        container_entry(0, "objects/vehicles/warthog/warthog", "collision_model"),
        container_entry(0, "objects/characters/elite/elite", "biped"),
    ];
    let (container, rel) = pick_container_template(entries.iter(), group_tag_of("biped"))
        .expect("biped is mounted, so it donates its own");
    assert_eq!(container, 0);
    assert!(rel.ends_with("-biped.uasset"), "got {rel}");
}

/// The reported bug. `cinematic_scene` ships no tag, so the same-group scan
/// finds nothing — and creation used to stop there with "No existing
/// cinematic_scene tag in the mounted paks to use as a template".
#[test]
fn a_group_the_game_ships_no_tag_of_still_finds_a_donor() {
    let entries = vec![
        container_entry(0, "objects/characters/elite/elite", "biped"),
        container_entry(1, "objects/vehicles/warthog/warthog", "collision_model"),
    ];
    let (container, rel) =
        pick_container_template(entries.iter(), group_tag_of("cinematic_scene"))
            .expect("a group with no shipped tag must still find a donor");
    assert_eq!(container, 1, "the donor's own container has to come with it");
    assert!(
        rel.ends_with("-collision_model.uasset"),
        "expected a bare-wrapper donor, got {rel}"
    );
}

/// The fallback is restricted to groups whose wrapper carries nothing. A `biped`
/// wrapper holds an `AssetReference` indexed against `BlamBipedTagDataAsset`'s
/// schema, so donating it to another group would name a different property —
/// `blam-tags` rejects that, and picking one here would only move the failure.
#[test]
fn a_donor_whose_wrapper_carries_properties_is_not_offered() {
    let entries = vec![container_entry(0, "objects/characters/elite/elite", "biped")];
    assert!(
        pick_container_template(entries.iter(), group_tag_of("cinematic_scene")).is_none(),
        "biped is not a safe cross-group donor"
    );
    for group in BARE_WRAPPER_DONOR_GROUPS {
        assert!(
            definition(group).is_file(),
            "{group} is offered as a donor but has no CE schema"
        );
    }
}

/// Only mounted container tags can donate. An in-memory tag created earlier in
/// the session has no `.uasset` in any pak to read.
#[test]
fn an_unsaved_tag_is_not_a_donor() {
    let entries = vec![TagEntry {
        key: "newtag:/Game/Tags/test/probe-collision_model".into(),
        display_path: "test/probe.collision_model".into(),
        group_tag: group_tag_of("collision_model"),
        group_name: Some("collision_model".into()),
        location: TagEntryLocation::NewContainer {
            template_container: 0,
            template_rel: "Tags/other-collision_model.uasset".into(),
            package: "/Game/Tags/test/probe-collision_model".into(),
            group_tag: group_tag_of("collision_model"),
        },
    }];
    assert!(
        pick_container_template(entries.iter(), group_tag_of("cinematic_scene")).is_none(),
        "an unsaved tag has no .uasset to donate"
    );
}
