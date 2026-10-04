use super::*;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

/// Which file errors a scan passes over. Anything else still fails it.
#[test]
fn a_scan_skips_missing_unreadable_and_held_files_only() {
    use std::io::{Error, ErrorKind};
    assert!(skippable_io_error(&Error::from(ErrorKind::NotFound)));
    assert!(skippable_io_error(&Error::from(ErrorKind::PermissionDenied)));
    assert!(!skippable_io_error(&Error::from(ErrorKind::InvalidData)));
    assert!(!skippable_io_error(&Error::from(ErrorKind::UnexpectedEof)));
    #[cfg(windows)]
    {
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION, ERROR_ACCESS_DENIED.
        assert!(skippable_io_error(&Error::from_raw_os_error(32)));
        assert!(skippable_io_error(&Error::from_raw_os_error(33)));
        assert!(skippable_io_error(&Error::from_raw_os_error(5)));
        // ERROR_INVALID_HANDLE is not a reason to skip.
        assert!(!skippable_io_error(&Error::from_raw_os_error(6)));
    }
}

/// A tag another program holds open without sharing (a kit tool saving
/// it) is left out of a folder scan, which finishes with the rest.
#[cfg(windows)]
#[test]
fn a_tag_held_open_without_sharing_is_skipped_by_a_scan() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = std::env::temp_dir().join(format!(
        "baboon-locked-scan-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let mut header = [0u8; 64];
    header[48..52].copy_from_slice(b"weap");
    header[60..64].copy_from_slice(b"BLAM");
    fs::write(root.join("free.weapon"), header).unwrap();
    fs::write(root.join("held.weapon"), header).unwrap();
    let scan = || {
        scan_folder_subtree_entries(&root, Path::new(""), &TagNameIndex::default())
            .map(|entries| entries.len())
    };
    assert_eq!(scan().unwrap(), 2);

    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(root.join("held.weapon"))
        .unwrap();
    let blocked = fs::File::open(root.join("held.weapon")).unwrap_err();
    assert_eq!(blocked.raw_os_error(), Some(32), "{blocked}");
    assert_eq!(scan().expect("the scan finishes"), 1);

    drop(held);
    assert_eq!(scan().unwrap(), 2);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn detect_game_from_game_id_folder_name() {
    // A folder named after the game (not the EK) must still resolve, so the
    // definitions/per-game features (incl. the doc overlay) work.
    assert_eq!(
        detect_ek_game(Path::new("/Users/x/Halo/halo3_mcc/tags/objects")),
        Some(GameId::Halo3)
    );
    assert_eq!(
        detect_ek_game(Path::new("/data/haloreach_mcc/tags")),
        Some(GameId::HaloReach)
    );
    // EK-style names still work.
    assert_eq!(detect_ek_game(Path::new("/x/H3EK/tags")), Some(GameId::Halo3));
}

fn temp_dir(name: &str) -> PathBuf {
    crate::test_kits::unique_temp_path(name)
}

fn write_fake_tag(path: &Path, group: &[u8; 4]) {
    let mut bytes = [0u8; 64];
    let group_tag = u32::from_be_bytes(*group);
    bytes[48..52].copy_from_slice(&group_tag.to_le_bytes());
    bytes[60..64].copy_from_slice(b"MALB");
    fs::write(path, bytes).unwrap();
}

#[test]
fn normalizes_blob_index_to_parent_cache_root() {
    let root = PathBuf::from(r"C:\tags\tag_cache");
    let blob = root.join("blob_index.dat");
    assert_eq!(normalize_blob_index_path(&blob).unwrap(), root);
    assert!(normalize_blob_index_path(&root.join("tag_blob.dat")).is_err());
}

#[test]
fn scans_loose_folder_with_header_probe() {
    let root = temp_dir("scan");
    fs::create_dir_all(root.join("objects/characters")).unwrap();
    write_fake_tag(&root.join("objects/characters/test.biped"), b"bipd");
    fs::write(root.join("not_a_tag.txt"), b"hello").unwrap();

    let index = TagNameIndex::default();
    let entries = scan_folder_entries(&root, &index).unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].display_path, "objects/characters/test.biped");
    assert_eq!(format_group_tag(entries[0].group_tag), "bipd");
}

#[test]
fn loose_file_entry_matches_scanned_folder_entry_metadata() {
    let root = temp_dir("drop_entry");
    fs::create_dir_all(root.join("objects/characters/brute")).unwrap();
    let path = root
        .join("objects")
        .join("characters")
        .join("brute")
        .join("brute.shader");
    write_fake_tag(&path, b"shdr");

    let names = TagNameIndex::default();
    let entry = loose_file_entry(&root, &path, &names)
        .unwrap()
        .expect("fake tag should probe as a tag");
    let scanned = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(scanned.len(), 1);
    assert_eq!(entry.key, scanned[0].key);
    assert_eq!(entry.display_path, "objects/characters/brute/brute.shader");
    assert_eq!(entry.display_path, scanned[0].display_path);
    assert_eq!(entry.group_tag, scanned[0].group_tag);
}

fn unique_game(name: &str) -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{name}_{stamp}")
}

fn remove_test_index(game: &str) {
    let _ = fs::remove_file(index_path(game));
    let _ = fs::remove_file(reverse_dependency_index_path(game));
    if let Ok(conn) = open_index_db() {
        let _ = conn.execute("DELETE FROM sources WHERE game = ?1", params![game]);
    }
}

fn write_fake_tag_with_padding(path: &Path, group: &[u8; 4], padding: usize) {
    let mut bytes = vec![0u8; 64 + padding];
    let group_tag = u32::from_be_bytes(*group);
    bytes[48..52].copy_from_slice(&group_tag.to_le_bytes());
    bytes[60..64].copy_from_slice(b"MALB");
    fs::write(path, bytes).unwrap();
}

#[test]
fn entry_index_refresh_reuses_unchanged_metadata() {
    let root = temp_dir("index_refresh_unchanged");
    let game = unique_game("index_refresh_unchanged");
    fs::create_dir_all(root.join("objects")).unwrap();
    write_fake_tag(&root.join("objects/a.model"), b"hlmt");
    write_fake_tag(&root.join("objects/b.shader"), b"shdr");
    let names = TagNameIndex::default();
    let entries = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    save_entry_index(&game, &root, &entries).unwrap();

    let refresh = refresh_entry_index(&game, &root, &names).unwrap();

    remove_test_index(&game);
    fs::remove_dir_all(&root).unwrap();

    assert!(!refresh.changed);
    assert_eq!(refresh.added, 0);
    assert_eq!(refresh.updated, 0);
    assert_eq!(refresh.removed, 0);
    assert_eq!(refresh.entries.len(), 2);
}

/// The refresh fans its per-file work out across every core, so its result
/// must not depend on how the files happened to be split between workers.
///
/// Enough files to guarantee more than one chunk on any machine, with a
/// mix of tags, non-tags, and a changed file, so the merged counts and the
/// final ordering are exercised rather than a single worker's happy path.
#[test]
fn entry_index_refresh_is_the_same_however_it_is_split_across_workers() {
    let root = temp_dir("index_refresh_parallel");
    let game = unique_game("index_refresh_parallel");
    fs::create_dir_all(root.join("objects")).unwrap();
    for i in 0..400 {
        write_fake_tag(&root.join(format!("objects/tag{i:03}.model")), b"hlmt");
    }
    // Not tags: they must be walked, counted as seen, and left out.
    for i in 0..40 {
        fs::write(root.join(format!("objects/note{i:02}.txt")), b"not a tag").unwrap();
    }
    let names = TagNameIndex::default();
    let entries = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    save_entry_index(&game, &root, &entries).unwrap();

    let unchanged = refresh_entry_index(&game, &root, &names).unwrap();
    assert!(!unchanged.changed, "nothing on disk moved");
    assert_eq!(unchanged.added, 0);
    assert_eq!(unchanged.updated, 0);
    assert_eq!(unchanged.removed, 0);
    assert_eq!(unchanged.entries.len(), 400);
    // Sorted, and sorted the same way a full scan sorts — the browser draws
    // folders in entry-vector order.
    assert_eq!(
        unchanged
            .entries
            .iter()
            .map(|entry| entry.display_path.clone())
            .collect::<Vec<_>>(),
        entries
            .iter()
            .map(|entry| entry.display_path.clone())
            .collect::<Vec<_>>()
    );

    // One file changes and one appears; the counts have to survive being
    // tallied by whichever worker happened to see them.
    write_fake_tag(&root.join("objects/tag007.model"), b"bipd");
    write_fake_tag(&root.join("objects/brand_new.weapon"), b"weap");
    let changed = refresh_entry_index(&game, &root, &names).unwrap();

    remove_test_index(&game);
    fs::remove_dir_all(&root).unwrap();

    assert!(changed.changed);
    assert_eq!(changed.added, 1, "one file appeared");
    assert_eq!(changed.updated, 1, "one file's group changed");
    assert_eq!(changed.removed, 0);
    assert_eq!(changed.entries.len(), 401);
}

#[test]
fn entry_index_refresh_reprobes_changed_files_and_drops_deleted_files() {
    let root = temp_dir("index_refresh_changed");
    let game = unique_game("index_refresh_changed");
    fs::create_dir_all(root.join("objects")).unwrap();
    let changed = root.join("objects/a.model");
    let removed = root.join("objects/b.shader");
    let added = root.join("objects/c.biped");
    write_fake_tag(&changed, b"hlmt");
    write_fake_tag(&removed, b"shdr");
    let names = TagNameIndex::default();
    let entries = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    save_entry_index(&game, &root, &entries).unwrap();

    write_fake_tag_with_padding(&changed, b"bipd", 1);
    fs::remove_file(&removed).unwrap();
    write_fake_tag(&added, b"bipd");
    let refresh = refresh_entry_index(&game, &root, &names).unwrap();

    remove_test_index(&game);
    fs::remove_dir_all(&root).unwrap();

    assert!(refresh.changed);
    assert_eq!(refresh.added, 1);
    assert_eq!(refresh.updated, 1);
    assert_eq!(refresh.removed, 1);
    let paths = refresh
        .entries
        .iter()
        .map(|entry| entry.display_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["objects/a.biped", "objects/c.biped"]);
}

#[test]
fn load_entry_index_accepts_legacy_cache_without_metadata() {
    let root = temp_dir("legacy_index");
    let game = unique_game("legacy_index");
    fs::create_dir_all(root.join("objects")).unwrap();
    let path = root.join("objects/a.model");
    write_fake_tag(&path, b"hlmt");
    let text = serde_json::to_string(&serde_json::json!({
        "root": root.display().to_string(),
        "entries": [{
            "key": format!("file:{}", path.display()),
            "display_path": "objects/a.model",
            "group_tag": u32::from_be_bytes(*b"hlmt"),
            "group_name": null
        }]
    }))
    .unwrap();
    if let Some(parent) = index_path(&game).parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(index_path(&game), text).unwrap();

    let entries = load_entry_index(&game, &root).unwrap();
    let refresh = refresh_entry_index(&game, &root, &TagNameIndex::default()).unwrap();

    remove_test_index(&game);
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(entries.len(), 1);
    assert!(!refresh.changed);
    assert_eq!(refresh.updated, 0);
    assert_eq!(refresh.entries.len(), 1);
}

/// A one-row upsert must leave the index exactly as a full rewrite of the
/// new entry set would, fingerprint included, so the next refresh finds
/// nothing to do.
#[test]
fn entry_index_upsert_matches_a_full_rewrite() {
    let root = temp_dir("index_upsert");
    let game = unique_game("index_upsert");
    let rewrite_game = unique_game("index_upsert_rewrite");
    fs::create_dir_all(root.join("objects")).unwrap();
    fs::create_dir_all(root.join("saved")).unwrap();
    write_fake_tag(&root.join("objects/a.model"), b"hlmt");
    let names = TagNameIndex::default();
    let before = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    save_entry_index(&game, &root, &before).unwrap();

    // A new file, and an existing file rewritten under the same key.
    write_fake_tag(&root.join("saved/b.shader"), b"shdr");
    write_fake_tag_with_padding(&root.join("objects/a.model"), b"hlmt", 16);
    let after = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    for entry in &after {
        assert!(upsert_entry_index_row(&game, &root, entry).unwrap());
    }
    save_entry_index(&rewrite_game, &root, &after).unwrap();

    let upserted = load_entry_index(&game, &root).unwrap();
    let rewritten = load_entry_index(&rewrite_game, &root).unwrap();
    let refresh = refresh_entry_index(&game, &root, &names).unwrap();

    remove_test_index(&game);
    remove_test_index(&rewrite_game);
    fs::remove_dir_all(&root).unwrap();

    let paths = |entries: &[TagEntry]| {
        entries
            .iter()
            .map(|entry| {
                (
                    entry.key.clone(),
                    entry.display_path.clone(),
                    entry.group_tag,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(upserted.len(), 2);
    assert_eq!(paths(&upserted), paths(&rewritten));
    assert!(!refresh.changed);
    assert_eq!((refresh.added, refresh.updated, refresh.removed), (0, 0, 0));
}

fn loose_source(
    root: &Path,
    game: Option<GameId>,
    entries: Vec<TagEntry>,
    all: Vec<TagEntry>,
) -> LoadedSourceData {
    LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: root.to_path_buf(),
            game,
            definitions_root: PathBuf::new(),
        },
        names: TagNameIndex::default(),
        game,
        entries,
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: all,
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    }
}

/// One tag in, one tag out, on an indexed loose folder: the full list stays
/// in `natural_key` order, a replaced entry keeps its lazy slot (trees hold
/// positions in that list), and the on-disk index matches a full rewrite.
#[test]
fn upserting_and_removing_an_entry_keeps_lists_and_index_consistent() {
    let root = temp_dir("upsert_entry");
    // The source's game names its index rows. A real game's rows are shared
    // with the user's own folders, so this test's are told apart by its
    // folder and removed by it.
    let game = GameId::Halo3.as_str();
    let check = unique_game("upsert_entry_check");
    fs::create_dir_all(root.join("objects")).unwrap();
    for name in ["a.model", "c.model"] {
        write_fake_tag(&root.join("objects").join(name), b"hlmt");
    }
    let names = TagNameIndex::default();
    let scanned = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    save_entry_index(game, &root, &scanned).unwrap();
    let mut source = loose_source(&root, Some(GameId::Halo3), scanned.clone(), scanned.clone());
    let lazy_c = source
        .entries
        .iter()
        .position(|e| e.display_path.ends_with("c.model"))
        .unwrap();

    // A new tag (sorted into the middle of the full list), a rewritten one
    // (replaced in place), and a deleted one.
    write_fake_tag(&root.join("objects/B.model"), b"hlmt");
    write_fake_tag_with_padding(&root.join("objects/c.model"), b"hlmt", 8);
    fs::remove_file(root.join("objects/a.model")).unwrap();
    let entry = |name: &str| {
        loose_file_entry(&root, &root.join("objects").join(name), &names)
            .unwrap()
            .unwrap()
    };
    source.upsert_entry(entry("B.model"), &[]);
    source.upsert_entry(entry("c.model"), &[]);
    let replaced_in_place = source.entries[lazy_c].display_path.ends_with("c.model")
        && source
            .entries
            .iter()
            .filter(|e| e.display_path.ends_with("c.model"))
            .count()
            == 1;
    // A removal is allowed to shift the lazy list: it re-reads the tree.
    source.remove_entry(&scanned[0].key, &[]);

    let after = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    save_entry_index(&check, &root, &after).unwrap();
    let indexed = load_entry_index(game, &root).unwrap();
    let rewritten = load_entry_index(&check, &root).unwrap();
    let refresh = refresh_entry_index(game, &root, &names).unwrap();

    crate::core::source::index::remove_test_index_source(game, &root);
    remove_test_index(&check);
    fs::remove_dir_all(&root).unwrap();

    let order: Vec<&str> = source
        .all_entries
        .iter()
        .map(|e| e.display_path.as_str())
        .collect();
    assert_eq!(
        order,
        ["objects/B.model", "objects/c.model"],
        "natural (case-insensitive) order"
    );
    assert!(replaced_in_place, "a replaced entry keeps its slot, once");
    let keys = |entries: &[TagEntry]| entries.iter().map(|e| e.key.clone()).collect::<Vec<_>>();
    assert_eq!(keys(&indexed), keys(&rewritten));
    assert!(!refresh.changed, "the index already matches the disk");
}

/// Expanding a folder again after its tree was rebuilt reuses the entries
/// the lazy list already has, instead of appending the same keys again.
#[test]
fn re_expanding_a_folder_does_not_duplicate_its_entries() {
    let root = temp_dir("reexpand");
    fs::create_dir_all(root.join("objects")).unwrap();
    write_fake_tag(&root.join("objects/a.model"), b"hlmt");
    write_fake_tag(&root.join("objects/b.model"), b"hlmt");
    let names = TagNameIndex::default();
    let mut entries = Vec::new();
    for _ in 0..2 {
        let mut tree = build_folder_directory_tree(&root).unwrap();
        let node = tree
            .children
            .iter_mut()
            .find(|node| node.label == "objects")
            .unwrap();
        load_folder_node_entries(&root, node, &mut entries, &names).unwrap();
        assert_eq!(node.entries.len(), 2);
    }
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(entries.len(), 2, "the second expansion added nothing");
}

/// A container's list is the full list and stays in `natural_key` order.
/// Several insert paths used to re-sort by a case-sensitive comparison,
/// which put "B" before "a" and broke `insert_entry_sorted` afterwards.
#[test]
fn a_containers_entries_stay_in_natural_order() {
    let entry = |name: &str| TagEntry {
        key: format!("ublock:{name}"),
        display_path: name.to_owned(),
        group_tag: 0,
        group_name: None,
        location: TagEntryLocation::LooseFile(PathBuf::from(name)),
    };
    let mut source = LoadedSourceData {
        source: TagSource::SingleFile {
            path: PathBuf::from("x"),
        },
        ..loose_source(
            Path::new("/unused"),
            None,
            vec![entry("a"), entry("c")],
            Vec::new(),
        )
    };
    source.upsert_entry(entry("B"), &[]);
    source.upsert_entry(entry("b2"), &[]);
    let order: Vec<&str> = source
        .entries
        .iter()
        .map(|e| e.display_path.as_str())
        .collect();
    assert_eq!(order, ["a", "B", "b2", "c"]);
}

/// A tag that cannot be opened (locked by another program, or unreadable)
/// is left out of a scan and a refresh; it used to fail both outright.
#[cfg(unix)]
#[test]
fn an_unreadable_tag_is_skipped_not_fatal() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_dir("unreadable_tag");
    let game = unique_game("unreadable_tag");
    fs::create_dir_all(root.join("objects")).unwrap();
    write_fake_tag(&root.join("objects/a.model"), b"hlmt");
    write_fake_tag(&root.join("objects/b.model"), b"hlmt");
    let names = TagNameIndex::default();
    let before = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    save_entry_index(&game, &root, &before).unwrap();
    let locked = root.join("objects/b.model");
    // Rewritten, so the refresh has to read it, then made unreadable.
    write_fake_tag_with_padding(&locked, b"hlmt", 4);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

    let scanned = scan_folder_subtree_entries(&root, Path::new(""), &names);
    let refreshed = refresh_entry_index(&game, &root, &names);

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
    remove_test_index(&game);
    fs::remove_dir_all(&root).unwrap();
    let scanned = scanned.expect("the scan survives one unreadable tag");
    assert_eq!(scanned.len(), 1);
    let refreshed = refreshed.expect("so does the refresh");
    assert_eq!(refreshed.entries.len(), 1);
}

/// A refresh names what changed, so only those tags are rewritten: the
/// rows it writes leave the index exactly as a full rewrite would, and
/// references are saved per tag only where a reference index exists.
#[test]
fn a_refresh_reports_and_persists_only_what_changed() {
    let root = temp_dir("refresh_changes");
    let game = unique_game("refresh_changes");
    let check = unique_game("refresh_changes_check");
    fs::create_dir_all(root.join("objects")).unwrap();
    for name in ["a.model", "b.model", "c.model"] {
        write_fake_tag(&root.join("objects").join(name), b"hlmt");
    }
    let names = TagNameIndex::default();
    let before = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    save_entry_index(&game, &root, &before).unwrap();
    let key = |name: &str| {
        before
            .iter()
            .find(|e| e.display_path.ends_with(name))
            .unwrap()
            .key
            .clone()
    };

    write_fake_tag_with_padding(&root.join("objects/b.model"), b"hlmt", 8);
    fs::remove_file(root.join("objects/c.model")).unwrap();
    write_fake_tag(&root.join("objects/d.model"), b"hlmt");
    let refresh = refresh_entry_index(&game, &root, &names).unwrap();
    let mut touched: Vec<String> = refresh
        .touched
        .iter()
        .map(|e| e.display_path.clone())
        .collect();
    touched.sort();

    // Row by row, as the refresh worker writes them.
    for gone in &refresh.removed_keys {
        delete_entry_index_row(&game, &root, gone).unwrap();
    }
    for entry in &refresh.touched {
        upsert_entry_index_row(&game, &root, entry).unwrap();
    }
    let after = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    save_entry_index(&check, &root, &after).unwrap();
    let patched = load_entry_index(&game, &root).unwrap();
    let rewritten = load_entry_index(&check, &root).unwrap();
    let no_reference_index =
        save_tag_dependencies(&game, &root, &key("a.model"), Some(&[])).unwrap();

    remove_test_index(&game);
    remove_test_index(&check);
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(touched, ["objects/b.model", "objects/d.model"]);
    assert_eq!(refresh.removed_keys, [key("c.model")]);
    let keys = |entries: &[TagEntry]| entries.iter().map(|e| e.key.clone()).collect::<Vec<_>>();
    assert_eq!(keys(&patched), keys(&rewritten));
    assert!(
        !no_reference_index,
        "no reference index here, so none is started"
    );
}

/// One row that names no file must not cost the whole index: the loader
/// used to return nothing at all, so every other tag was re-probed and the
/// reference index was lost with it.
#[test]
fn an_index_row_without_a_file_key_is_skipped_not_fatal() {
    let root = temp_dir("index_bad_row");
    let game = unique_game("index_bad_row");
    fs::create_dir_all(root.join("objects")).unwrap();
    write_fake_tag(&root.join("objects/a.model"), b"hlmt");
    let names = TagNameIndex::default();
    let entries = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
    save_entry_index(&game, &root, &entries).unwrap();
    // What New Tag and Blam Import used to register.
    let mut bare = entries[0].clone();
    bare.key = "objects/b.model".to_owned();
    bare.display_path = "objects/b.model".to_owned();
    assert!(upsert_entry_index_row(&game, &root, &bare).unwrap());

    let loaded = load_entry_index(&game, &root);

    remove_test_index(&game);
    fs::remove_dir_all(&root).unwrap();
    let loaded = loaded.expect("the good rows must still load");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].key, entries[0].key);
}

/// With no index for the folder yet, a lone row would load back as a
/// complete one-tag index, so the upsert must not create one.
#[test]
fn entry_index_upsert_never_creates_an_index() {
    let root = temp_dir("index_upsert_absent");
    let game = unique_game("index_upsert_absent");
    fs::create_dir_all(root.join("objects")).unwrap();
    write_fake_tag(&root.join("objects/a.model"), b"hlmt");
    let names = TagNameIndex::default();
    let entries = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();

    let written = upsert_entry_index_row(&game, &root, &entries[0]).unwrap();
    let loaded = load_entry_index(&game, &root);

    remove_test_index(&game);
    fs::remove_dir_all(&root).unwrap();

    assert!(!written);
    assert!(loaded.is_none());
}

#[test]

fn sqlite_entry_index_keeps_multiple_roots_for_same_game() {
    let root_a = temp_dir("sqlite_multi_root_a");
    let root_b = temp_dir("sqlite_multi_root_b");
    let game = unique_game("sqlite_multi_root");
    fs::create_dir_all(root_a.join("objects")).unwrap();
    fs::create_dir_all(root_b.join("levels")).unwrap();
    write_fake_tag(&root_a.join("objects/a.model"), b"hlmt");
    write_fake_tag(&root_b.join("levels/b.scenario"), b"scnr");
    let names = TagNameIndex::default();
    let entries_a = scan_folder_subtree_entries(&root_a, Path::new(""), &names).unwrap();
    let entries_b = scan_folder_subtree_entries(&root_b, Path::new(""), &names).unwrap();

    save_entry_index(&game, &root_a, &entries_a).unwrap();
    save_entry_index(&game, &root_b, &entries_b).unwrap();
    let loaded_a = load_entry_index(&game, &root_a).unwrap();
    let loaded_b = load_entry_index(&game, &root_b).unwrap();

    remove_test_index(&game);
    fs::remove_dir_all(&root_a).unwrap();
    fs::remove_dir_all(&root_b).unwrap();

    assert_eq!(loaded_a.len(), 1);
    assert_eq!(loaded_a[0].display_path, "objects/a.model");
    assert_eq!(loaded_b.len(), 1);
    assert_eq!(loaded_b[0].display_path, "levels/b.scenario");
}

#[test]
fn sqlite_reverse_dependency_index_round_trips_empty_and_targeted_dependencies() {
    let root = temp_dir("sqlite_reverse");
    let game = unique_game("sqlite_reverse");
    fs::create_dir_all(root.join("objects")).unwrap();
    let tag_key = format!("file:{}", root.join("objects/a.model").display());
    let empty_key = format!("file:{}", root.join("objects/empty.model").display());
    let mut index = ReverseDependencyIndex::default();
    index.set_tag_dependencies(
        tag_key.clone(),
        vec![DependencyRef {
            group_tag: u32::from_be_bytes(*b"bitm"),
            rel_path: "objects\\tex".to_owned(),
        }],
    );
    index.set_tag_dependencies(empty_key.clone(), Vec::new());

    save_reverse_dependency_index(&game, &root, &index).unwrap();
    let loaded = load_reverse_dependency_index(&game, &root).unwrap();

    remove_test_index(&game);
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded.dependencies_of(&empty_key), &[]);
    assert_eq!(
        loaded.dependents_for(u32::from_be_bytes(*b"bitm"), "objects\\tex"),
        &[tag_key]
    );
}

#[test]
fn probes_classic_h2_group_from_reversed_header() {
    let root = temp_dir("classic_h2_probe");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("brute.mode");
    let mut bytes = [0u8; 64];
    bytes[36..40].copy_from_slice(b"edom");
    bytes[60..64].copy_from_slice(b"!MLB");
    fs::write(&path, bytes).unwrap();

    assert_eq!(
        probe_tag_group(&path).unwrap(),
        Some(u32::from_be_bytes(*b"mode"))
    );

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn probes_classic_ce_group_from_big_endian_header() {
    let root = temp_dir("classic_ce_probe");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("cyborg.gbxmodel");
    let mut bytes = [0u8; 64];
    bytes[36..40].copy_from_slice(b"mod2");
    bytes[60..64].copy_from_slice(b"blam");
    fs::write(&path, bytes).unwrap();

    assert_eq!(
        probe_tag_group(&path).unwrap(),
        Some(u32::from_be_bytes(*b"mod2"))
    );

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn load_folder_descends_into_ek_tags_root() {
    let root = temp_dir("h4ek");
    let ek_root = root.join("H4EK");
    fs::create_dir_all(ek_root.join("tags/objects/vehicles")).unwrap();
    fs::create_dir_all(ek_root.join("data/objects/vehicles")).unwrap();
    write_fake_tag(
        &ek_root.join("tags/objects/vehicles/warthog.model"),
        b"hlmt",
    );
    write_fake_tag(
        &ek_root.join("data/objects/vehicles/not_in_tags.model"),
        b"hlmt",
    );

    let loaded = load_folder(
        ek_root.clone(),
        &TagNameIndex::default(),
        &root.join("definitions"),
        &[],
    )
    .unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(loaded.label, "H4EK/tags (halo4_mcc)");
    assert!(loaded.entries.is_empty());
    assert_eq!(loaded.tree.children[0].label, "objects");
    assert_eq!(loaded.tree.children[0].rel_path, PathBuf::from("objects"));
    assert!(loaded.tree.children[0].children.is_empty());
    assert!(!loaded.tree.children[0].children_loaded);
    assert!(!loaded.tree.children[0].entries_loaded);
    match loaded.source {
        TagSource::LooseFolder { root, .. } => assert!(root.ends_with("tags")),
        _ => panic!("expected loose folder source"),
    }
}

#[test]
fn lazy_folder_node_loads_only_direct_tag_files() {
    let root = temp_dir("lazy_node");
    fs::create_dir_all(root.join("objects/vehicles/child")).unwrap();
    write_fake_tag(&root.join("objects/vehicles/warthog.model"), b"hlmt");
    write_fake_tag(&root.join("objects/vehicles/child/child.model"), b"hlmt");

    let mut tree = build_folder_directory_tree(&root).unwrap();
    let mut entries = Vec::new();
    let objects = &mut tree.children[0];
    load_folder_node_entries(&root, objects, &mut entries, &TagNameIndex::default()).unwrap();
    let vehicles = &mut objects.children[0];
    load_folder_node_entries(&root, vehicles, &mut entries, &TagNameIndex::default()).unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].display_path, "objects/vehicles/warthog.model");
    assert!(vehicles.entries_loaded);
}

#[test]
fn subtree_scan_finds_nested_tag_files_for_folder_export() {
    let root = temp_dir("subtree_export");
    fs::create_dir_all(root.join("objects/vehicles/child")).unwrap();
    fs::create_dir_all(root.join("objects/characters")).unwrap();
    write_fake_tag(&root.join("objects/vehicles/warthog.model"), b"hlmt");
    write_fake_tag(&root.join("objects/vehicles/child/child.model"), b"hlmt");
    write_fake_tag(&root.join("objects/characters/spartan.model"), b"hlmt");

    let entries = scan_folder_subtree_entries(
        &root,
        Path::new("objects/vehicles"),
        &TagNameIndex::default(),
    )
    .unwrap();
    fs::remove_dir_all(&root).unwrap();

    let display_paths = entries
        .into_iter()
        .map(|entry| entry.display_path)
        .collect::<Vec<_>>();
    assert_eq!(
        display_paths,
        vec![
            "objects/vehicles/child/child.model",
            "objects/vehicles/warthog.model",
        ]
    );
}

#[test]
fn ek_root_without_tags_folder_errors_without_deep_search() {
    let root = temp_dir("missing_tags");
    let ek_root = root.join("HREK");
    fs::create_dir_all(ek_root.join("data/tags")).unwrap();

    let error = resolve_folder_root(&ek_root, &[]).unwrap_err().to_string();
    fs::remove_dir_all(&root).unwrap();

    assert!(error.contains("expected tags folder was missing"));
    assert!(error.contains("HREK"));
}

/// A kit keeping a second tags folder beside the stock one opens that
/// folder when it (or a folder in it) is picked. Picking the root, the
/// stock tags folder, or the data folder still opens the stock `tags`.
#[test]
fn another_tags_folder_under_an_ek_root_opens_itself() {
    let root = temp_dir("other_tags_folder");
    let ek_root = root.join("H2EK");
    for folder in ["tags", "tags_moda/objects", "data", "data_moda"] {
        fs::create_dir_all(ek_root.join(folder)).unwrap();
    }
    let scan = |selected: &Path| resolve_folder_root(selected, &[]).unwrap().scan_root;

    let other = scan(&ek_root.join("tags_moda"));
    let inside = scan(&ek_root.join("tags_moda/objects"));
    let stock = scan(&ek_root);
    let from_data = scan(&ek_root.join("data_moda"));
    let label = resolve_folder_root(&ek_root, &[]).unwrap().label;
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(other, ek_root.join("tags_moda"));
    assert_eq!(inside, ek_root.join("tags_moda"));
    assert_eq!(stock, ek_root.join("tags"));
    assert_eq!(from_data, ek_root.join("tags"));
    assert_eq!(label, "H2EK/tags (halo2_mcc)");
}

#[test]
fn detects_supported_ek_games_from_root_or_tags_folder() {
    assert_eq!(detect_ek_game(&PathBuf::from("HCEEK")), Some(GameId::HaloCe));
    assert_eq!(
        detect_ek_game(&PathBuf::from("H1EK").join("tags")),
        Some(GameId::HaloCe)
    );
    assert_eq!(
        detect_ek_game(&PathBuf::from("H2EK").join("tags")),
        Some(GameId::Halo2)
    );
    assert_eq!(
        detect_ek_game(&PathBuf::from("HREK")),
        Some(GameId::HaloReach)
    );
    assert_eq!(
        detect_ek_game(&PathBuf::from("H4EK").join("tags")),
        Some(GameId::Halo4)
    );
    assert_eq!(
        detect_ek_game(&PathBuf::from("H3ODSTEK").join("tags")),
        Some(GameId::Halo3Odst)
    );
    assert_eq!(
        detect_ek_game(&PathBuf::from("H3EK").join("tags")),
        Some(GameId::Halo3)
    );
}

#[test]
fn custom_ek_alias_detects_root_folder() {
    let root = temp_dir("custom_ek_alias_root");
    let ek_root = root.join("h2rek");
    fs::create_dir_all(ek_root.join("tags/objects")).unwrap();
    let aliases = vec![EkFolderAlias {
        folder_name: "h2rek".to_owned(),
        game: "halo2_mcc".to_owned(),
    }];

    let info = resolve_folder_root(&ek_root, &aliases).unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(info.game, Some(GameId::Halo2));
    assert!(info.scan_root.ends_with("tags"));
    assert_eq!(info.label, "h2rek/tags (halo2_mcc)");
}

#[test]
fn custom_ek_alias_detects_tags_folder() {
    let root = temp_dir("custom_ek_alias_tags");
    let tags_root = root.join("h2rek").join("tags");
    fs::create_dir_all(tags_root.join("objects")).unwrap();
    let aliases = vec![EkFolderAlias {
        folder_name: "h2rek".to_owned(),
        game: "halo2_mcc".to_owned(),
    }];

    let info = resolve_folder_root(&tags_root, &aliases).unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(info.game, Some(GameId::Halo2));
    assert!(info.scan_root.ends_with("tags"));
    assert_eq!(info.label, "tags (halo2_mcc)");
}

#[test]
fn built_in_ek_name_takes_precedence_over_alias() {
    let path = PathBuf::from("H2EK").join("tags");
    let aliases = vec![EkFolderAlias {
        folder_name: "H2EK".to_owned(),
        game: "halo3_mcc".to_owned(),
    }];

    let detected = detect_ek_root_with_aliases(&path, &aliases).map(|(_, game)| game);

    assert_eq!(detected, Some(GameId::Halo2));
}

/// Monolithic cache names and container logical paths carry no extension,
/// so a dot in them is part of the name. Real tags with dots exist in
/// every game (sixteen in Halo 3 alone).
#[test]
fn a_dot_in_an_extensionless_name_is_kept() {
    let names = TagNameIndex::default();
    let snd = u32::from_be_bytes(*b"snd!");
    let bitm = u32::from_be_bytes(*b"bitm");
    assert_eq!(
        display_str_with_friendly_extension(
            "sound/levels/dlc/descent/scarab_factory/piston_close2.l",
            snd,
            &names,
        ),
        "sound/levels/dlc/descent/scarab_factory/piston_close2.l.sound"
    );
    assert_eq!(
        display_str_with_friendly_extension("levels/v1.2/bitmaps/rock", bitm, &names),
        "levels/v1.2/bitmaps/rock.bitmap"
    );
    // A name that already ends in this group's extension keeps it once.
    assert_eq!(
        display_str_with_friendly_extension("a/b/piston_close2.l.sound", snd, &names),
        "a/b/piston_close2.l.sound"
    );
}

/// A tag file's own extension is replaced, but a dot in a folder is not one.
#[test]
fn only_a_files_own_extension_is_replaced() {
    let names = TagNameIndex::default();
    let bitm = u32::from_be_bytes(*b"bitm");
    let snd = u32::from_be_bytes(*b"snd!");
    assert_eq!(
        display_path_with_friendly_extension(
            Path::new("levels/v1.2/bitmaps/rock.bitmap"),
            bitm,
            &names
        ),
        "levels/v1.2/bitmaps/rock.bitmap"
    );
    assert_eq!(
        display_path_with_friendly_extension(Path::new("a/piston_close2.l.sound"), snd, &names),
        "a/piston_close2.l.sound"
    );
}

/// Before definitions load, `bloc` used to display as `device_control`;
/// it is `crate` in every game that has it, and the engine's table says so.
#[test]
fn a_crate_displays_as_a_crate_without_definitions() {
    let names = TagNameIndex::default();
    let bloc = u32::from_be_bytes(*b"bloc");
    let ctrl = u32::from_be_bytes(*b"ctrl");
    assert_eq!(
        display_str_with_friendly_extension("objects/x/crate1", bloc, &names),
        "objects/x/crate1.crate"
    );
    assert_eq!(
        display_str_with_friendly_extension("objects/x/switch", ctrl, &names),
        "objects/x/switch.device_control"
    );
}

#[test]
fn rewrites_short_cache_suffixes_to_foundation_names() {
    let names = TagNameIndex::default();
    let cases = [
        (
            b"bipd",
            "objects/characters/spartans/spartans.bipd",
            "objects/characters/spartans/spartans.biped",
        ),
        (
            b"coll",
            "objects/characters/spartans/spartans.coll",
            "objects/characters/spartans/spartans.collision_model",
        ),
        (
            b"phmo",
            "objects/characters/spartans/spartans.phmo",
            "objects/characters/spartans/spartans.physics_model",
        ),
        (
            b"jmad",
            "objects/characters/spartans/spartans.jmad",
            "objects/characters/spartans/spartans.model_animation_graph",
        ),
        (
            b"impo",
            "objects/characters/spartans/spartans.impo",
            "objects/characters/spartans/spartans.imposter_model",
        ),
        (
            b"frms",
            "objects/characters/spartans/spartans.frms",
            "objects/characters/spartans/spartans.frame_event_list",
        ),
    ];

    for (group, input, expected) in cases {
        let group_tag = u32::from_be_bytes(*group);
        assert_eq!(
            display_str_with_friendly_extension(input, group_tag, &names),
            expected
        );
    }
}

#[test]
fn builds_hierarchical_tree_from_display_paths() {
    let entries = vec![
        TagEntry {
            key: "a".into(),
            display_path: "objects/test/a.biped".into(),
            group_tag: u32::from_be_bytes(*b"bipd"),
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from("a")),
        },
        TagEntry {
            key: "b".into(),
            display_path: "objects/test/b.model".into(),
            group_tag: u32::from_be_bytes(*b"hlmt"),
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from("b")),
        },
    ];
    let tree = build_tree(&entries);
    assert_eq!(tree.children[0].label, "objects");
    assert_eq!(tree.children[0].children[0].label, "test");
    assert_eq!(tree.children[0].children[0].entries, vec![0, 1]);
}

#[test]
fn folder_tree_starts_beneath_the_opened_folder_and_keeps_full_paths() {
    let entries = vec![
        folder_entry("direct", "objects/characters/brute/brute.biped"),
        folder_entry("nested", "objects/characters/brute/bitmaps/brute.bitmap"),
        folder_entry("outside", "objects/characters/elite/elite.biped"),
    ];

    let tree = build_tree_beneath(&entries, Path::new("objects/characters/brute"));

    assert_eq!(tree.entries, vec![0]);
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].label, "bitmaps");
    assert_eq!(
        tree.children[0].rel_path,
        PathBuf::from("objects/characters/brute/bitmaps")
    );
    assert_eq!(tree.children[0].entries, vec![1]);
}

#[test]
fn folder_group_tree_keeps_indices_into_the_shared_entry_set() {
    let entries = vec![
        folder_entry("outside", "objects/characters/elite/elite.model"),
        folder_entry("inside", "objects/characters/brute/brute.model"),
    ];

    let tree = build_group_tree_beneath(&entries, Path::new("objects/characters/brute"));

    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].entries, vec![1]);
    assert!(entry_is_beneath_folder(
        &entries[1],
        Path::new("OBJECTS/CHARACTERS/BRUTE")
    ));
    assert!(!entry_is_beneath_folder(
        &entries[0],
        Path::new("objects/characters/brute")
    ));
}

fn folder_entry(key: &str, display_path: &str) -> TagEntry {
    TagEntry {
        key: key.into(),
        display_path: display_path.into(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: None,
        location: TagEntryLocation::LooseFile(PathBuf::from(key)),
    }
}

fn child<'a>(node: &'a TagTreeNode, label: &str) -> &'a TagTreeNode {
    node.children
        .iter()
        .find(|child| child.label == label)
        .unwrap_or_else(|| panic!("no child {label:?}"))
}

fn root_child<'a>(tree: &'a TagTree, label: &str) -> &'a TagTreeNode {
    tree.children
        .iter()
        .find(|child| child.label == label)
        .unwrap_or_else(|| panic!("no root child {label:?}"))
}

/// A pak's directory index cannot encode a directory with no file beneath
/// it, so a folder the user made has to be carried by the tree until a tag
/// lands in it. Without this it vanishes the moment it is created.
#[test]
fn a_seeded_folder_appears_even_though_no_entry_reaches_it() {
    let entries = vec![folder_entry("a", "objects/characters/a.model")];
    let tree = build_tree_with_folders(&entries, &["objects/vehicles/warthog".to_owned()]);

    let objects = root_child(&tree, "objects");
    let vehicles = child(objects, "vehicles");
    let warthog = child(vehicles, "warthog");
    assert!(warthog.entries.is_empty());
    assert!(warthog.children.is_empty());
    // The deletable-folder rule the browser draws with.
    assert!(warthog.pending);
    // `objects` already existed, so seeding through it must not claim it.
    assert!(!objects.pending);
    // An intermediate the seed did create is pending, but it has a child, so
    // it is not offered for deletion until the leaf goes first.
    assert!(vehicles.pending);
    assert!(!vehicles.children.is_empty());
}

/// Re-seeding a folder a tag has since landed in must not produce a second
/// node beside the real one, and must not relabel the real one as pending.
#[test]
fn seeding_a_folder_that_now_holds_a_tag_is_a_no_op() {
    let entries = vec![folder_entry("a", "objects/vehicles/warthog/a.model")];
    let tree = build_tree_with_folders(&entries, &["objects/vehicles/warthog".to_owned()]);

    let vehicles = child(root_child(&tree, "objects"), "vehicles");
    assert_eq!(vehicles.children.len(), 1, "no duplicate warthog node");
    let warthog = child(vehicles, "warthog");
    assert_eq!(warthog.entries, vec![0]);
    assert!(!warthog.pending, "a folder a tag reached is not pending");
}

/// `build_tree` is the same code path with an empty seed list, so nothing
/// that does not opt in can acquire a pending node.
#[test]
fn build_tree_seeds_nothing_and_marks_nothing_pending() {
    let entries = vec![folder_entry("a", "objects/characters/a.model")];
    let tree = build_tree(&entries);
    let objects = root_child(&tree, "objects");
    assert!(!objects.pending);
    assert!(!child(objects, "characters").pending);
    assert_eq!(objects.children.len(), 1);
}

#[test]
fn builds_group_tree_from_entries() {
    let entries = vec![
        TagEntry {
            key: "a".into(),
            display_path: "objects/test/a.biped".into(),
            group_tag: u32::from_be_bytes(*b"bipd"),
            group_name: Some("biped".into()),
            location: TagEntryLocation::LooseFile(PathBuf::from("a")),
        },
        TagEntry {
            key: "b".into(),
            display_path: "objects/test/b2.biped".into(),
            group_tag: u32::from_be_bytes(*b"bipd"),
            group_name: Some("biped".into()),
            location: TagEntryLocation::LooseFile(PathBuf::from("b")),
        },
        TagEntry {
            key: "c".into(),
            display_path: "objects/test/c.render_model".into(),
            group_tag: u32::from_be_bytes(*b"mode"),
            group_name: Some("render_model".into()),
            location: TagEntryLocation::LooseFile(PathBuf::from("c")),
        },
    ];
    let tree = build_group_tree(&entries);
    assert_eq!(tree.children.len(), 2);
    assert_eq!(tree.children[0].label, "biped bipd");
    assert_eq!(tree.children[0].entries, vec![0, 1]);
}

#[test]
fn group_tree_uses_friendly_fallback_when_name_is_fourcc() {
    let entries = vec![TagEntry {
        key: "a".into(),
        display_path: "objects/test/a.weapon".into(),
        group_tag: u32::from_be_bytes(*b"weap"),
        group_name: Some("weap".into()),
        location: TagEntryLocation::LooseFile(PathBuf::from("a")),
    }];

    let tree = build_group_tree(&entries);

    assert_eq!(tree.children[0].label, "weapon weap");
}

#[test]
fn builds_field_summaries_from_fixture_when_present() {
    let fixture = PathBuf::from("dump/storm_knight/storm_knight.biped");
    if !fixture.exists() {
        return;
    }
    let tag = TagFile::read(&fixture).unwrap();
    let rows = field_row_summaries(&tag, &TagNameIndex::default(), 24);
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .any(|r| r.contains("block") || r.contains("struct"))
    );
}
