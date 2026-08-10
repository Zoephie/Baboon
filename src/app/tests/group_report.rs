//! Whether a Campaign Evolved group can be authored, and what the answer rests
//! on.
//!
//! Uses the bundled Meteorite mappings, which are dumped from the game's own
//! binary — so these are assertions about the real class table rather than about
//! a fixture, and a group moving between verdicts would mean the game changed.

use super::*;

fn usmap() -> blam_tags::iostore::object::usmap::Usmap {
    blam_tags::iostore::object::usmap::Usmap::meteorite().expect("the bundled Meteorite mappings")
}

/// A group with shipped tags never needs deriving, whatever its class declares
/// — cloning one is the path with the most mileage on it.
#[test]
fn a_group_the_game_ships_is_authorable_by_copying() {
    let verdict = group_authorability("biped", 42, &usmap());
    assert_eq!(verdict, GroupAuthorability::Donor { shipped: 42 });
    assert!(verdict.authorable());
    assert!(verdict.summary().contains("42 shipped tags"));
}

/// `cinematic_scene` is the reported case that started this: the game declares
/// the class and ships no instance, and the class adds nothing over the base —
/// so it is buildable from the group alone.
#[test]
fn a_bare_group_with_nothing_shipped_can_still_be_built() {
    let verdict = group_authorability("cinematic_scene", 0, &usmap());
    assert_eq!(verdict, GroupAuthorability::Derived);
    assert!(verdict.authorable());
}

/// Halo's abstract bases have no standalone instances by design. They are
/// refused, and the refusal names what a wrapper would have had to declare
/// rather than stopping at "cannot".
#[test]
fn an_abstract_base_group_says_what_it_would_have_needed() {
    for group in ["object", "unit", "item", "device"] {
        let verdict = group_authorability(group, 0, &usmap());
        assert!(
            !verdict.authorable(),
            "{group} is one of Halo's abstract bases"
        );
        let GroupAuthorability::NeedsDonor { properties } = &verdict else {
            panic!("{group} should have a class that declares extra properties, got {verdict:?}");
        };
        assert!(
            !properties.is_empty(),
            "{group} should name the properties in the way"
        );
        assert!(verdict.summary().contains("nothing shipped to copy"));
    }
}

/// The verdict that no amount of work on Baboon would change. A group whose
/// class the binary never declared is out of reach from a pak entirely, and
/// saying so is more useful than a generic failure.
#[test]
fn a_group_with_no_class_in_the_binary_is_out_of_reach() {
    let verdict = group_authorability("not_a_real_halo_group", 0, &usmap());
    assert_eq!(verdict, GroupAuthorability::NoClass);
    assert!(!verdict.authorable());
    assert!(verdict.summary().contains("no class"));
}

/// The summary is what the group list shows, so it has to stay short when a
/// class declares a lot. Three names then a count.
#[test]
fn a_long_property_list_is_summarised_rather_than_dumped() {
    let verdict = GroupAuthorability::NeedsDonor {
        properties: ["Model", "Materials", "Physics", "Collision", "Sound"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
    };
    let summary = verdict.summary();
    assert!(summary.contains("Model, Materials, Physics"));
    assert!(summary.contains("and 2 more"), "{summary}");
    assert!(!summary.contains("Collision"), "{summary}");
}

/// The count comes from the mount, and the mount answers with `entries` for a
/// container source and `all_entries` once a loose folder has been scanned.
/// Reading the wrong one reports every group as unshipped.
#[test]
fn shipped_counts_read_whichever_entry_set_the_source_filled() {
    let mut source = LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::SingleFile {
            path: PathBuf::from("C:/x.biped"),
        },
        names: TagNameIndex::default(),
        game: None,
        entries: vec![
            TagEntry {
                key: "a".to_owned(),
                display_path: "a.biped".to_owned(),
                group_tag: 0x6269_7064,
                group_name: Some("biped".to_owned()),
                location: TagEntryLocation::LooseFile(PathBuf::from("C:/a.biped")),
            },
            TagEntry {
                key: "b".to_owned(),
                display_path: "b.biped".to_owned(),
                group_tag: 0x6269_7064,
                group_name: Some("biped".to_owned()),
                location: TagEntryLocation::LooseFile(PathBuf::from("C:/b.biped")),
            },
        ],
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
    };
    assert_eq!(shipped_counts_by_group(&source).get(&0x6269_7064), Some(&2));

    // Once a scan fills `all_entries`, that becomes the whole set.
    source.all_entries = source.entries.clone();
    source.all_entries.push(TagEntry {
        key: "c".to_owned(),
        display_path: "c.weapon".to_owned(),
        group_tag: 0x7765_6170,
        group_name: Some("weapon".to_owned()),
        location: TagEntryLocation::LooseFile(PathBuf::from("C:/c.weapon")),
    });
    let counts = shipped_counts_by_group(&source);
    assert_eq!(counts.get(&0x6269_7064), Some(&2));
    assert_eq!(counts.get(&0x7765_6170), Some(&1));
}
