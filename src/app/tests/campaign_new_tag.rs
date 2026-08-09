//! Creating a Campaign Evolved tag in a group the game ships no instance of.
//!
//! The dialog offers all 141 groups the definitions define, but the game ships a
//! tag for only 101 of them. Creation used to require an existing same-group
//! `.uasset` to donate the UE5 package structure, so the other 40 -- among them
//! `cinematic_scene`, the reported case -- could not be created at all. The
//! wrapper is now *derived* from the group's own rules when no same-group tag
//! ships, which covers 36 of those 40. The last four are `object`, `unit`,
//! `item` and `device`: Halo's abstract base groups, which have no standalone
//! instances by design and stay refused.

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
/// `apply_editing_kit_mcc_header`, which only knew the five MCC games. An
/// unstamped tag parses in Baboon and is unlike anything the game ships.
///
/// The values are measured, not chosen: all 12,289 shipped CE tag blobs read
/// `1 / 2 / 0xffffffff`, across all 101 groups, with no variation. See
/// `the_shipped_tag_header_generation` in blam-tags.
#[test]
fn a_new_campaign_evolved_tag_carries_the_shipped_generation() {
    let mut tag = TagFile::new(definition("cinematic_scene")).expect("build from the CE schema");
    assert_eq!(
        (tag.header.build_version, tag.header.build_number, tag.header.version),
        (0, 0, 0),
        "TagFile::new starts at zero, which is what makes the stamp necessary"
    );
    apply_editing_kit_mcc_header(&mut tag, CAMPAIGN_EVOLVED_GAME).expect("CE is a known game");
    assert_eq!(
        (tag.header.build_version, tag.header.build_number, tag.header.version),
        CAMPAIGN_EVOLVED_GENERATION
    );
}

/// Campaign Evolved's generation is Halo Reach's, because a CE `.ubulk` *is* a
/// Reach-format tag — the UE5 package around it is only a wrapper. Pinned as an
/// equality rather than two copies of the same literals, so the two can never be
/// changed apart by accident.
#[test]
fn campaign_evolved_carries_the_same_generation_as_reach() {
    let stamp = |game: &str| {
        let mut tag = TagFile::new(definition("cinematic_scene")).expect("any tag will do");
        apply_editing_kit_mcc_header(&mut tag, game).unwrap_or_else(|e| panic!("{game}: {e}"));
        (tag.header.build_version, tag.header.build_number, tag.header.version)
    };
    assert_eq!(stamp(CAMPAIGN_EVOLVED_GAME), stamp("haloreach_mcc"));
    assert_eq!(stamp(CAMPAIGN_EVOLVED_GAME), CAMPAIGN_EVOLVED_GENERATION);
}

/// Adding Campaign Evolved must not have moved the editing-kit games.
///
/// `apply_editing_kit_mcc_header` is the only function the CE creation path
/// shares with them, so it is the only place a CE change could reach
/// H2EK/H3EK/H3ODSTEK/H4EK/H2AMPEK tags. The generations are pinned here as well
/// as in `editing_kit_header_defaults_match_profile_generations` because that
/// test asserts on serialized bytes and this one asserts on the match arms —
/// CE joined the `build_number = 2` group, and H3/ODST must stay at 1.
#[test]
fn adding_campaign_evolved_left_the_editing_kit_games_alone() {
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
    }
}

/// The CE arm is an early return on one exact string, so it must not have
/// widened what the function accepts. A game with no known generation is still
/// an error rather than a tag stamped with someone else's.
#[test]
fn a_game_with_no_known_generation_is_still_rejected() {
    for game in ["", "haloce_evolved_x", "halo5", "haloce"] {
        let mut tag = TagFile::new(definition("cinematic_scene")).expect("any tag will do");
        assert!(
            apply_editing_kit_mcc_header(&mut tag, game).is_err(),
            "{game:?} should have no known tag-header defaults"
        );
        assert_eq!(tag.header.build_version, 0, "{game:?} was stamped anyway");
    }
}

/// The classic profiles are a deliberate no-op, not an error.
///
/// Halo CE and Halo 2 tags have no MCC generation: `write_classic_tag` copies the
/// original 64-byte header through and patches only the checksum, so those three
/// fields are not part of the format. Returning an error here would make every
/// classic conversion fail; stamping them would corrupt the header the kit reads.
/// The contract is "leave it alone, and say that went fine".
#[test]
fn a_classic_profile_is_left_unstamped_without_failing() {
    for game in ["haloce_mcc", "halo2_mcc"] {
        let mut tag = TagFile::new(definition("cinematic_scene")).expect("any tag will do");
        assert!(
            apply_editing_kit_mcc_header(&mut tag, game).is_ok(),
            "{game:?} should be a no-op, not an error"
        );
        assert_eq!(tag.header.build_version, 0, "{game:?} was stamped anyway");
        assert_eq!(tag.header.build_number, 0, "{game:?} was stamped anyway");
        assert_eq!(tag.header.version, 0, "{game:?} was stamped anyway");
    }
}

/// Every way a Campaign Evolved tag comes into existence has to end up with the
/// same generation, or "correct" depends on which menu item was used.
///
/// Three paths reach `add_new_container_tag`: New Tag builds from the schema and
/// stamps; Copy round-trips an existing CE tag through its own bytes and so
/// inherits a header that is already right; Import takes a file from disk, whose
/// header is the *source's* and has to be restamped. This pins the two that go
/// through a `TagFile` the caller supplies.
#[test]
fn an_imported_tag_is_restamped_for_campaign_evolved() {
    // A tag carrying another kit's generation — Halo 3's, `build_number = 1`.
    let mut foreign = TagFile::new(definition("cinematic_scene")).expect("any tag will do");
    apply_editing_kit_mcc_header(&mut foreign, "halo3_mcc").expect("H3 is a known game");
    assert_ne!(
        (foreign.header.build_version, foreign.header.build_number, foreign.header.version),
        CAMPAIGN_EVOLVED_GENERATION,
        "the fixture has to start wrong for this to prove anything"
    );

    apply_editing_kit_mcc_header(&mut foreign, CAMPAIGN_EVOLVED_GAME).expect("CE is a known game");
    assert_eq!(
        (foreign.header.build_version, foreign.header.build_number, foreign.header.version),
        CAMPAIGN_EVOLVED_GENERATION
    );

    // And a zeroed header — what an older Baboon wrote — restamps too.
    let mut zeroed = TagFile::new(definition("cinematic_scene")).expect("any tag will do");
    apply_editing_kit_mcc_header(&mut zeroed, CAMPAIGN_EVOLVED_GAME).expect("CE is a known game");
    assert_eq!(
        (zeroed.header.build_version, zeroed.header.build_number, zeroed.header.version),
        CAMPAIGN_EVOLVED_GENERATION
    );
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
///
/// It now derives instead, which is why the scan returning `None` is the
/// *expected* answer here rather than the failure it used to be. Both halves are
/// asserted: no donor is found, and the group is still creatable.
#[test]
fn a_group_the_game_ships_no_tag_of_is_derived_rather_than_donated() {
    let entries = vec![
        container_entry(0, "objects/characters/elite/elite", "biped"),
        container_entry(1, "objects/vehicles/warthog/warthog", "collision_model"),
    ];
    let donor = pick_container_template(entries.iter(), group_tag_of("cinematic_scene"));
    assert!(
        donor.is_none(),
        "no other group may stand in as a donor, got {donor:?}"
    );
    assert!(
        matches!(
            new_container_template_for(donor, "cinematic_scene"),
            Ok(NewContainerTemplate::Derived { .. })
        ),
        "cinematic_scene has to remain creatable without one"
    );
}

/// The four groups that are neither shipped nor derivable are Halo's abstract
/// base groups. Refusing them is the point: they have no standalone instances,
/// and a wrapper donated from some bare group would declare none of the
/// properties their classes actually carry — the silent failure cloning had.
///
/// This is the half that makes the test above mean something. Asserting only
/// that `cinematic_scene` is allowed would pass just as well if the gate had
/// been deleted outright.
#[test]
fn an_abstract_base_group_is_refused_by_name() {
    for group in ["object", "unit", "item", "device"] {
        let error = new_container_template_for(None, group)
            .expect_err("an abstract base group has no wrapper to derive");
        assert!(
            error.contains(group),
            "the refusal has to name the group, got {error:?}"
        );
    }
}

/// A `biped` wrapper holds an `AssetReference` indexed against
/// `BlamBipedTagDataAsset`'s schema, so donating it to another group would name
/// a different property under the destination's.
#[test]
fn a_donor_of_another_group_is_never_offered() {
    let entries = vec![container_entry(0, "objects/characters/elite/elite", "biped")];
    assert!(
        pick_container_template(entries.iter(), group_tag_of("cinematic_scene")).is_none(),
        "biped is not a cross-group donor"
    );
    // Nor is a bare one. `collision_model` carries no properties of its own, so
    // `blam-tags` would accept it as a donor — the reason it is refused is the
    // destination, not the donor.
    let bare = vec![container_entry(
        0,
        "objects/vehicles/warthog/warthog",
        "collision_model",
    )];
    assert!(
        pick_container_template(bare.iter(), group_tag_of("cinematic_scene")).is_none(),
        "a bare donor is still the wrong group"
    );
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
            template: NewContainerTemplate::Donor {
                container: 0,
                rel_path: "Tags/other-collision_model.uasset".into(),
            },
            package: "/Game/Tags/test/probe-collision_model".into(),
            group_tag: group_tag_of("collision_model"),
        },
    }];
    assert!(
        pick_container_template(entries.iter(), group_tag_of("collision_model")).is_none(),
        "an unsaved tag has no .uasset to donate"
    );
}
