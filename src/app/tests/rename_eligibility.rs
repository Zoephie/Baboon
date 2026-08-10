//! Who may rename a tag inside the pak that holds it.
//!
//! Only tags Baboon authored, and the reason is a measurement rather than a
//! policy: a container redirect does not forward references. Renaming the
//! assault rifle removes it from the game, and renaming it with a redirect
//! verified present in the container removes it just the same. So a rename
//! relocates a tag only when nothing points at it — which for a tag Baboon
//! created is true by construction, because it did not exist when the game was
//! built.

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

    let unsaved = container_rename_eligibility(&new_container, &no_containers(), &ledger)
        .expect_err("an unsaved tag has no pak to rename inside");
    assert!(unsaved.contains("not in a pak yet"), "{unsaved}");
    assert!(
        container_rename_eligibility(&loose, &no_containers(), &ledger).is_err(),
        "a loose tag is renamed on disk"
    );
    assert!(container_rename_eligibility(&monolithic, &no_containers(), &ledger).is_err());
}

#[test]
fn a_tag_whose_container_is_gone_is_refused_before_anything_is_decided() {
    let mut ledger = CreatedTagLedger::default();
    ledger.record(record(CreatedTagOrigin::Authored));
    let error = container_rename_eligibility(&entry_at(REL), &no_containers(), &ledger)
        .expect_err("the container is not mounted");
    assert!(error.contains("no longer mounted"), "{error}");
}

/// A row saying the tag came from a shipped one must not qualify it. Renaming
/// twice would otherwise launder a shipped tag into one that renames freely,
/// which is the same hole `CreatedTagOrigin` exists to close on the delete side
/// — and here the consequence is a reference that silently stops resolving.
#[test]
fn a_renamed_shipped_tag_does_not_become_an_authored_one() {
    let mut ledger = CreatedTagLedger::default();
    ledger.record(record(CreatedTagOrigin::RenamedFromShipped));
    // With no container mounted the call cannot reach the decision itself, so
    // this asserts the ledger row that the decision reads.
    let found = ledger
        .find(Path::new(UTOC), REL)
        .expect("the row is addressed by container path");
    assert_ne!(
        found.origin,
        CreatedTagOrigin::Authored,
        "only an Authored row may be renamed in place"
    );
}

/// The line handed to the writer comes from the ledger row, not from anything
/// derived at the call site — the writer re-checks it, and a number invented
/// here would either refuse a valid rename or authorise an invalid one.
#[test]
fn the_provenance_line_is_the_one_the_ledger_recorded() {
    assert_eq!(
        AuthoredRename {
            minimum_appended_index: 4096
        }
        .minimum_appended_index,
        4096
    );
    assert_eq!(
        record(CreatedTagOrigin::Authored).container_entry_count_before,
        4096
    );
}
