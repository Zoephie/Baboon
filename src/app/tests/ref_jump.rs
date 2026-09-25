//! The "References to X" popup's per-row occurrence walk.

use super::*;
use std::time::Duration;

fn scratch_root(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("baboon-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(root.join("objects")).unwrap();
    root
}

/// A 64-byte header the folder probe recognises as a tag, and nothing after it.
fn write_header_only_tag(path: &Path, group: &[u8; 4]) {
    let mut bytes = [0u8; 64];
    bytes[48..52].copy_from_slice(&u32::from_be_bytes(*group).to_le_bytes());
    bytes[60..64].copy_from_slice(b"MALB");
    std::fs::write(path, bytes).unwrap();
}

/// An expanded row for a referrer that is not open must be read once. It used
/// to go through the tab loader, which drops results for tags without a tab,
/// so the row asked again as soon as each load finished — for as long as the
/// popup stayed open.
#[test]
fn an_unopened_referrer_is_read_once_not_reloaded_forever() {
    let root = scratch_root("ref-jump");
    let path = root.join("objects/referrer.model");
    write_header_only_tag(&path, b"hlmt");
    let names = TagNameIndex::default();
    let entry = loose_file_entry(&root, &path, &names).unwrap().unwrap();

    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: root.clone(),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names,
        game: None,
        entries: vec![entry.clone()],
        tree: crate::source::build_folder_directory_tree(&root).unwrap(),
        group_tree: crate::source::build_group_tree(std::slice::from_ref(&entry)),
        all_entries: vec![entry.clone()],
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
    });
    app.query_results = Some(TagQueryResults {
        kit: app.active_kit_id(),
        title: "References to bitmaps/target".to_owned(),
        entries: vec![entry],
        annotations: Vec::new(),
        note: None,
        ref_target: Some((u32::from_be_bytes(*b"bitm"), "bitmaps/target".to_owned())),
    });
    app.ref_jump_expanded.insert(0);
    let ctx = egui::Context::default();

    // Three frames of the popup. Each one delivers whatever the last one
    // started through the app's own message pump. Installing the source
    // starts unrelated background work too, so only reads of the referrer —
    // through either path — are counted.
    let mut loads = 0;
    for frame in 0..3 {
        app.refresh_ref_jump_occurrences(&ctx);
        let mut wait = if frame == 0 {
            Duration::from_secs(10)
        } else {
            Duration::from_millis(300)
        };
        let mut received = Vec::new();
        while let Ok(message) = app.rx.recv_timeout(wait) {
            if matches!(
                message,
                WorkerMessage::RefJumpOccurrences { .. } | WorkerMessage::TagLoaded { .. }
            ) {
                loads += 1;
            }
            received.push(message);
            wait = Duration::from_millis(300);
        }
        for message in received {
            app.tx.send(message).unwrap();
        }
        app.process_worker_messages(&ctx);
    }

    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(loads, 1, "the referrer must be read exactly once");
    assert!(
        app.ref_jump_occurrences.contains_key(&0),
        "the row must settle (here with no occurrences: the tag has no body)"
    );
}
