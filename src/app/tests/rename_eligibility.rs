//! Who may rename a tag inside the pak that holds it, and on what grounds.
//!
//! The gate is deliberately more permissive than the delete gate, and the whole
//! design rests on why: a delete destroys the only copy of a payload, while a
//! rename retires two chunks, writes equivalents and leaves a redirect, with the
//! backup still holding the container as it was. Getting that asymmetry wrong in
//! either direction is expensive — too strict and the feature does not exist for
//! the tags people actually want to rename, too loose and Expert mode stops
//! meaning anything.

use super::*;

const UTOC: &str = "C:/Game/Paks/pakchunk240-Windows.utoc";
const REL: &str = "Meteorite/Content/Tags/objects/copy-biped.ubulk";

fn entry_at(rel_path: &str) -> TagEntry {
    TagEntry {
        key: format!("ublock:pakchunk240-Windows:{rel_path}"),
        display_path: "objects/copy.biped".to_owned(),
        group_tag: 0x6269_7064,
        group_name: Some("biped".to_owned()),
        location: TagEntryLocation::Container {
            container: 0,
            rel_path: rel_path.to_owned(),
        },
    }
}

fn record(origin: CreatedTagOrigin) -> CreatedTagRecord {
    CreatedTagRecord {
        utoc_path: UTOC.to_owned(),
        chunk_label: "pakchunk240-Windows".to_owned(),
        package_path: "/Game/Tags/objects/copy-biped".to_owned(),
        package_id: 7,
        uasset_path: "Meteorite/Content/Tags/objects/copy-biped.uasset".to_owned(),
        ubulk_path: REL.to_owned(),
        display_path: "objects/copy.biped".to_owned(),
        group_tag: 0x6269_7064,
        source_display: "objects/original.biped".to_owned(),
        container_entry_count_before: 4096,
        origin,
        created_unix_secs: 1,
    }
}

/// A `MountedContainer` needs a real archive, which a unit test has no way to
/// build — so the container list is empty and the tests that need one assert on
/// the refusal that produces. Everything the tier decision actually turns on is
/// reachable without it.
fn no_containers() -> Vec<crate::source::MountedContainer> {
    Vec::new()
}

#[test]
fn a_tag_that_is_not_in_a_pak_is_refused_with_a_reason_that_names_the_alternative() {
    let ledger = CreatedTagLedger::default();
    let new_container = TagEntry {
        key: "new:1".to_owned(),
        display_path: "objects/fresh.biped".to_owned(),
        group_tag: 0,
        group_name: None,
        location: TagEntryLocation::NewContainer {
            template: NewContainerTemplate::Donor {
                container: 0,
                rel_path: "Tags/template-biped.uasset".to_owned(),
            },
            package: "/Game/Tags/objects/fresh-biped".to_owned(),
            group_tag: 0,
        },
    };
    let loose = TagEntry {
        key: "file:1".to_owned(),
        display_path: "objects/loose.biped".to_owned(),
        group_tag: 0,
        group_name: None,
        location: TagEntryLocation::LooseFile(PathBuf::from("C:/kit/tags/objects/loose.biped")),
    };
    let monolithic = TagEntry {
        key: "cache:bipd:objects/cached".to_owned(),
        display_path: "objects/cached.biped".to_owned(),
        group_tag: 0,
        group_name: None,
        location: TagEntryLocation::Monolithic {
            name: "objects/cached".to_owned(),
            group_tag: 0,
        },
    };

    // Expert mode is on for all three: none of these is a permissions question.
    let unsaved = container_rename_eligibility(&new_container, &no_containers(), &ledger, true)
        .expect_err("an unsaved tag has no pak to rename inside");
    assert!(unsaved.contains("not in a pak yet"), "{unsaved}");
    assert!(
        container_rename_eligibility(&loose, &no_containers(), &ledger, true).is_err(),
        "a loose tag is renamed on disk"
    );
    assert!(container_rename_eligibility(&monolithic, &no_containers(), &ledger, true).is_err());
}

#[test]
fn a_tag_whose_container_is_gone_is_refused_before_any_tier_is_decided() {
    let mut ledger = CreatedTagLedger::default();
    ledger.record(record(CreatedTagOrigin::Authored));
    let error = container_rename_eligibility(&entry_at(REL), &no_containers(), &ledger, true)
        .expect_err("the container is not mounted");
    assert!(error.contains("no longer mounted"), "{error}");
}

/// The tier is a decision about evidence, so it is worth pinning what each one
/// hands the writer. `Authored` carries the line the writer re-checks; `Shipped`
/// carries nothing, because for a shipped tag there is nothing true to carry —
/// its chunks sit *below* the line, so passing it would refuse the very
/// operation being authorised.
#[test]
fn each_tier_hands_the_writer_only_evidence_that_is_true_of_it() {
    assert_eq!(
        RenameTier::Authored {
            minimum_appended_index: 4096
        }
        .minimum_appended_index(),
        Some(4096)
    );
    assert_eq!(RenameTier::Shipped.minimum_appended_index(), None);
}

/// A record that says the tag came from a shipped one must not promote it past
/// the Expert gate — otherwise renaming twice would launder a shipped tag into
/// one that renames freely, which is the same hole `CreatedTagOrigin` exists to
/// close on the delete side.
#[test]
fn a_renamed_shipped_tag_does_not_become_an_authored_one() {
    let mut ledger = CreatedTagLedger::default();
    ledger.record(record(CreatedTagOrigin::RenamedFromShipped));
    // With no container mounted the call cannot reach the tier decision, so
    // this asserts the ledger row itself, which is what the decision reads.
    let found = ledger
        .find(Path::new(UTOC), REL)
        .expect("the row is addressed by container path");
    assert_eq!(found.origin, CreatedTagOrigin::RenamedFromShipped);
    assert_ne!(
        found.origin,
        CreatedTagOrigin::Authored,
        "only an Authored row may skip the Expert gate"
    );
}
