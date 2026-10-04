use std::path::PathBuf;
use std::fs;
use std::path::Path;
use super::*;

fn record(utoc: &str, ubulk: &str) -> CreatedTagRecord {
    CreatedTagRecord {
        utoc_path: utoc.to_owned(),
        chunk_label: "pakchunk240-WinGDK".to_owned(),
        package_path: "/Game/Tags/objects/copy-biped".to_owned(),
        package_id: package_id_for("/Game/Tags/objects/copy-biped"),
        uasset_path: "Meteorite/Content/Tags/objects/copy-biped.uasset".to_owned(),
        ubulk_path: ubulk.to_owned(),
        display_path: "objects/copy.biped".to_owned(),
        group_tag: 0,
        source_display: "objects/original.biped".to_owned(),
        container_entry_count_before: 4,
        origin: CreatedTagOrigin::Authored,
        created_unix_secs: 1,
    }
}

#[test]
fn a_record_addresses_one_copy_in_one_container() {
    let ledger = {
        let mut ledger = CreatedTagLedger::default();
        ledger.record(record(
            "C:/Game/Paks/pakchunk240-WinGDK.utoc",
            "Meteorite/Content/Tags/objects/copy-biped.ubulk",
        ));
        ledger
    };

    assert!(
        ledger
            .find(
                Path::new("C:/Game/Paks/pakchunk240-WinGDK.utoc"),
                "Meteorite/Content/Tags/objects/COPY-biped.ubulk",
            )
            .is_some(),
        "container paths are matched case-insensitively, as the mount stores them"
    );
    assert!(
        ledger
            .find(
                Path::new("D:/Other/Paks/pakchunk240-WinGDK.utoc"),
                "Meteorite/Content/Tags/objects/copy-biped.ubulk",
            )
            .is_none(),
        "the same relative path in another install is a different tag"
    );
}

const UTOC: &str = "C:/Game/Paks/pakchunk240-WinGDK.utoc";

fn renamed_to(ubulk: &str) -> CreatedTagRecord {
    CreatedTagRecord {
        // Deliberately the wrong answer, to prove the caller is not the one
        // deciding: `record_rename` overwrites this from the ledger.
        origin: CreatedTagOrigin::Authored,
        ..record(UTOC, ubulk)
    }
}

/// A tag the game ships has no ledger record, so the rename is the first
/// thing the ledger ever hears about it — and it must not conclude from the
/// silence that Baboon authored it.
#[test]
fn renaming_a_tag_the_ledger_never_knew_marks_it_as_shipped() {
    let mut ledger = CreatedTagLedger::default();
    let new = "Meteorite/Content/Tags/objects/renamed-biped.ubulk";
    ledger.record_rename(
        "Meteorite/Content/Tags/objects/shipped-biped.ubulk",
        renamed_to(new),
    );

    let record = ledger.find(Path::new(UTOC), new).expect("recorded");
    assert_eq!(record.origin, CreatedTagOrigin::RenamedFromShipped);
}

/// And the converse: a copy Baboon made keeps its authorship across the
/// move, along with the provenance line the delete path checks.
#[test]
fn renaming_a_copy_carries_its_authorship_and_its_provenance_line() {
    let mut ledger = CreatedTagLedger::default();
    let old = "Meteorite/Content/Tags/objects/copy-biped.ubulk";
    let new = "Meteorite/Content/Tags/objects/moved-biped.ubulk";
    ledger.record(record(UTOC, old));
    ledger.record_rename(
        old,
        CreatedTagRecord {
            container_entry_count_before: 9999,
            ..renamed_to(new)
        },
    );

    assert!(ledger.find(Path::new(UTOC), old).is_none());
    let record = ledger.find(Path::new(UTOC), new).expect("recorded");
    assert_eq!(record.origin, CreatedTagOrigin::Authored);
    assert_eq!(
        record.container_entry_count_before, 4,
        "the line already recorded is the conservative one and is kept"
    );
    assert_eq!(ledger.tags.len(), 1);
}

/// Renaming twice must not launder a shipped tag into an authored one, and
/// a stale record sitting at the destination must not be inherited either —
/// that is the same laundering with an extra step.
#[test]
fn a_shipped_tag_stays_shipped_however_far_it_is_moved() {
    let mut ledger = CreatedTagLedger::default();
    let first = "Meteorite/Content/Tags/objects/once-biped.ubulk";
    let second = "Meteorite/Content/Tags/objects/twice-biped.ubulk";
    // A copy Baboon made once lived where the shipped tag is about to land.
    ledger.record(record(UTOC, second));
    ledger.record_rename(
        "Meteorite/Content/Tags/objects/shipped-biped.ubulk",
        renamed_to(first),
    );
    ledger.record_rename(first, renamed_to(second));

    let record = ledger.find(Path::new(UTOC), second).expect("recorded");
    assert_eq!(
        record.origin,
        CreatedTagOrigin::RenamedFromShipped,
        "the record already at the destination is dropped, not inherited"
    );
    assert_eq!(ledger.tags.len(), 1);
}

/// Every row written before this field existed was a duplicate, because a
/// duplicate was the only thing the ledger could hold. `Authored` is the
/// truth about those rows rather than a fallback — and it has to survive
/// reading a file that predates the field, or every existing copy silently
/// stops being deletable.
#[test]
fn a_ledger_file_with_no_origin_reads_as_authored() {
    let legacy = serde_json::json!({
        "version": 1,
        "tags": [{
            "utoc_path": UTOC,
            "chunk_label": "pakchunk240-WinGDK",
            "package_path": "/Game/Tags/objects/copy-biped",
            "package_id": 7,
            "uasset_path": "Meteorite/Content/Tags/objects/copy-biped.uasset",
            "ubulk_path": "Meteorite/Content/Tags/objects/copy-biped.ubulk",
            "display_path": "objects/copy.biped",
            "group_tag": 0,
            "source_display": "objects/original.biped",
            "container_entry_count_before": 4,
            "created_unix_secs": 1
        }]
    });
    let ledger = CreatedTagLedger::from_bytes(&serde_json::to_vec(&legacy).unwrap());
    assert!(ledger.load_error.is_none(), "a pre-origin ledger still reads");
    let record = ledger
        .find(
            Path::new(UTOC),
            "Meteorite/Content/Tags/objects/copy-biped.ubulk",
        )
        .expect("the row survived");
    assert_eq!(record.origin, CreatedTagOrigin::Authored);
}

#[test]
fn recording_the_same_path_twice_keeps_only_the_newer_copy() {
    let mut ledger = CreatedTagLedger::default();
    let utoc = "C:/Game/Paks/pakchunk240-WinGDK.utoc";
    let ubulk = "Meteorite/Content/Tags/objects/copy-biped.ubulk";
    ledger.record(record(utoc, ubulk));
    let mut newer = record(utoc, ubulk);
    newer.created_unix_secs = 99;
    ledger.record(newer);

    assert_eq!(ledger.tags.len(), 1);
    assert_eq!(
        ledger
            .find(Path::new(utoc), ubulk)
            .unwrap()
            .created_unix_secs,
        99
    );
    assert!(ledger.forget(Path::new(utoc), ubulk));
    assert!(!ledger.forget(Path::new(utoc), ubulk));
    assert!(ledger.is_empty());
}

fn write_test_toc(path: &Path, entry_count: u32) {
    let mut bytes = vec![0u8; 144];
    bytes[..16].copy_from_slice(TOC_MAGIC);
    bytes[24..28].copy_from_slice(&entry_count.to_le_bytes());
    fs::write(path, bytes).unwrap();
}

#[test]
fn backups_recover_the_chunk_count_a_container_started_with() {
    let root = std::env::temp_dir().join(format!(
        "baboon-backup-provenance-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let utoc = root.join("pakchunk0-Windows.utoc");
    write_test_toc(&utoc, 122_810);

    // No backup yet: nothing in the container can be claimed as Baboon's.
    assert_eq!(container_original_entry_count(&utoc), None);

    write_test_toc(
        &root.join("pakchunk0-Windows.utoc.baboon-duplicate-backup"),
        122_804,
    );
    write_test_toc(
        &root.join("pakchunk0-Windows.utoc.baboon-duplicate-backup-1"),
        122_806,
    );
    fs::write(
        root.join("pakchunk0-Windows.utoc.baboon-duplicate-backup.manifest.json"),
        b"{}",
    )
    .unwrap();
    // The earliest backup wins, so a later one cannot narrow the window and
    // strand a copy made before it. Manifests are not TOCs and are skipped.
    assert_eq!(container_original_entry_count(&utoc), Some(122_804));

    // A sibling that is not one of our backups is ignored entirely.
    write_test_toc(&root.join("pakchunk1-Windows.utoc"), 5);
    assert_eq!(container_original_entry_count(&utoc), Some(122_804));

    fs::write(
        root.join("pakchunk0-Windows.utoc.baboon-duplicate-backup-2"),
        b"not a toc",
    )
    .unwrap();
    assert_eq!(container_original_entry_count(&utoc), Some(122_804));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_unreadable_ledger_reads_as_empty_rather_than_failing_startup() {
    let ledger = CreatedTagLedger::from_bytes(b"{ not json");
    assert!(ledger.is_empty());
    assert!(ledger.load_error.is_some(), "and is remembered as unread");
}

fn scratch_ledger(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "baboon-ledger-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir.join(LEDGER_FILE)
}

/// A newer build may write an `origin` this one does not know. That used
/// to fail the whole file's parse, so the ledger loaded empty and the next
/// save (any duplicate, delete or rename) erased every record in it.
#[test]
fn an_unknown_origin_costs_nothing_and_survives_a_save() {
    let path = scratch_ledger("future-origin");
    let mut future = serde_json::to_value(record(UTOC, "future.ubulk")).unwrap();
    future["origin"] = serde_json::json!("ImportedFromMod");
    // A row whose shape this build cannot read at all is kept raw.
    let alien = serde_json::json!({ "utoc_path": 7, "shape": "from the future" });
    let file = serde_json::json!({
        "version": 2,
        "tags": [future, serde_json::to_value(record(UTOC, "known.ubulk")).unwrap(), alien],
    });
    fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();

    let mut ledger = CreatedTagLedger::load_from(&path);
    let found = ledger.find(Path::new(UTOC), "future.ubulk").cloned();
    ledger.record(record(UTOC, "new.ubulk"));
    ledger.save_to(&path).unwrap();
    let reloaded: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let _ = fs::remove_dir_all(path.parent().unwrap());

    assert_eq!(
        found.map(|record| record.origin),
        Some(CreatedTagOrigin::Unrecognized("ImportedFromMod".to_owned()))
    );
    let rows = reloaded["tags"].as_array().unwrap();
    assert_eq!(rows.len(), 4, "{rows:#?}");
    assert!(rows.iter().any(|row| row["origin"] == "ImportedFromMod"));
    assert!(rows.contains(&alien));
    assert!(rows.iter().any(|row| row["ubulk_path"] == "known.ubulk"));
}

/// A file that cannot be parsed at all is never written over.
#[test]
fn a_ledger_that_failed_to_load_is_not_saved_over() {
    let path = scratch_ledger("truncated");
    let original = b"{ \"version\": 1, \"tags\": [ { \"utoc_path\": \"C:/";
    fs::write(&path, original).unwrap();

    let mut ledger = CreatedTagLedger::load_from(&path);
    ledger.record(record(UTOC, "new.ubulk"));
    let saved = ledger.save_to(&path);
    let on_disk = fs::read(&path).unwrap();
    let _ = fs::remove_dir_all(path.parent().unwrap());

    assert!(saved.is_err(), "the save is refused and says why");
    assert_eq!(on_disk, original, "the file is left exactly as it was");
}
