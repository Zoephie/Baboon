//! The saved spelling of every entry key and index root key, checked against
//! fixed strings and the synthetic samples in `testdata/compat`.
//!
//! These strings are a file format: sessions, keyword sidecars, favourites, the
//! duplicate ledger and the index database all store them. A typed key has to
//! write exactly these bytes, or every one of those files stops finding its
//! tags. Each golden is a literal, so a change to how a key is built fails here.

use super::*;
use std::path::MAIN_SEPARATOR;

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/compat/samples")
}

fn sample_json(rel: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(samples().join(rel))
        .unwrap_or_else(|error| panic!("{rel}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{rel}: {error}"))
}

fn group(fourcc: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*fourcc)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "baboon-compat-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Every key kind in `tag_keys.json`, rebuilt by the function the loaders use.
#[test]
fn compat_entry_keys_are_spelled_as_saved() {
    let keys = sample_json("tag_keys.json");
    let key = |kind: &str| keys[kind].as_str().unwrap().to_owned();

    // Paths are written as displayed, whatever their shape, on every OS.
    for (kind, path) in [
        (
            "file_windows",
            r"C:\Program Files (x86)\Steam\steamapps\common\H3EK\tags\objects\weapons\rifle\assault_rifle\assault_rifle.weapon",
        ),
        (
            "file_unc",
            r"\\fileserver\share\H2EK\tags\objects\characters\masterchief\masterchief.biped",
        ),
        (
            "file_verbatim",
            r"\\?\C:\Kits\H3ODSTEK\tags\objects\vehicles\warthog\warthog.vehicle",
        ),
    ] {
        assert_eq!(file_entry_key(Path::new(path)), key(kind), "{kind}");
    }
    // Never split on `:`: the drive letter's colon is part of the path.
    assert!(key("file_windows").starts_with(r"file:C:\Program Files"));

    // A root joined with what the walk found: the separator added is the
    // platform's, so the saved spelling of the same tag differs by OS.
    let mixed = file_entry_key(
        &Path::new("C:/Kits/HREK/tags").join(r"objects\characters\elite\elite.biped"),
    );
    let posix = file_entry_key(
        &Path::new("/Users/me/Kits/H4EK/tags")
            .join("objects/weapons/rifle/storm_rifle/storm_rifle.weapon"),
    );
    #[cfg(windows)]
    {
        assert_eq!(mixed, key("file_mixed"));
        assert_eq!(
            posix,
            r"file:/Users/me/Kits/H4EK/tags\objects/weapons/rifle/storm_rifle/storm_rifle.weapon"
        );
    }
    #[cfg(not(windows))]
    {
        assert_eq!(
            mixed,
            r"file:C:/Kits/HREK/tags/objects\characters\elite\elite.biped"
        );
        assert_eq!(posix, key("file_posix"));
    }

    // The group with its trailing spaces trimmed, the name as the cache has it.
    assert_eq!(
        cache_entry_key(
            group(b"bitm"),
            r"objects\weapons\rifle\assault_rifle\bitmaps\assault_rifle_diffuse"
        ),
        key("cache_bitm")
    );
    assert_eq!(
        cache_entry_key(group(b"rm  "), r"shaders\default"),
        key("cache_rm")
    );
    assert_eq!(
        cache_entry_key(group(b"snd!"), r"sound\weapons\assault_rifle\fire"),
        key("cache_snd")
    );

    // The container's file stem, then the payload path in its original case.
    for (kind, utoc, payload) in [
        (
            "ublock_base",
            "/Paks/pakchunk0-WinGDK.utoc",
            "Meteorite/Content/Tags/objects/characters/marine/marine-biped.ubulk",
        ),
        (
            "ublock_level",
            "/Paks/pakchunk240-WinGDK.utoc",
            "Meteorite/Content/Tags/levels/halo1/solo/a30/_Generated_/a30-scenario.ubulk",
        ),
        (
            "ublock_mod",
            "/Paks/~mods/mymod_P.utoc",
            "Meteorite/Content/Tags/objects/characters/marine/marine_copy-biped.ubulk",
        ),
    ] {
        let label = container_chunk_label(Path::new(utoc));
        assert_eq!(container_entry_key(&label, payload), key(kind), "{kind}");
    }
    assert_eq!(
        container_chunk_label(Path::new("/Paks/PakChunk7-WinGDK.UTOC")),
        "PakChunk7-WinGDK",
        "the stem keeps its case"
    );
    assert_eq!(
        container_chunk_label(Path::new("/Paks/~mods/my.mod_P.utoc")),
        "my.mod_P",
        "only the extension is dropped"
    );

    // A bare key from before `file:` existed names no location at all.
    assert!(!key("legacy_bare").contains(':'));
}

/// What a folder scan saves: `file:`, the root exactly as it was given, then
/// the walked names joined with the platform's separator. Not canonicalized,
/// so a root under a symlink (macOS's `/var`) keeps its spelling.
#[test]
fn compat_a_folder_scan_keys_tags_on_the_root_as_given() {
    let root = scratch("scan");
    let folder = root.join("objects").join("a");
    std::fs::create_dir_all(&folder).unwrap();
    let mut header = [0u8; 64];
    header[48..52].copy_from_slice(b"weap");
    header[60..64].copy_from_slice(b"BLAM");
    std::fs::write(folder.join("b.weapon"), header).unwrap();

    let scan = |root: &Path| {
        let entries = scan_folder_subtree_entries(root, Path::new(""), &TagNameIndex::default())
            .expect("scan");
        assert_eq!(entries.len(), 1);
        entries[0].key.clone()
    };
    let s = MAIN_SEPARATOR;
    assert_eq!(
        scan(&root),
        format!("file:{}{s}objects{s}a{s}b.weapon", root.display())
    );
    // macOS's temp dir is under `/var`, a symlink, unless TMPDIR points
    // somewhere else; where the two spellings differ, the key keeps the one
    // it was given.
    let canonical = std::fs::canonicalize(&root).unwrap();
    if canonical != root {
        assert!(
            !scan(&root).starts_with(&format!("file:{}", canonical.display())),
            "the root is reached through a symlink; the key must not resolve it"
        );
    }
    // A root typed with forward slashes on Windows keeps them; what the walk
    // adds uses backslashes. Both halves are saved as they are.
    #[cfg(windows)]
    {
        let typed = root.display().to_string().replace('\\', "/");
        assert_eq!(
            scan(Path::new(&typed)),
            format!(r"file:{typed}\objects\a\b.weapon")
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// The index database finds a source by `(game, cache_root_key(root))`. The
/// key is the root as given (canonical when it exists); on Windows it is also
/// lowered and given backslashes, so any spelling of the same folder matches.
#[test]
fn compat_cache_root_keys() {
    let existing = scratch("root-key");
    let canonical = std::fs::canonicalize(&existing).unwrap();
    #[cfg(not(windows))]
    {
        for missing in [
            "/nonexistent-baboon-compat/Kits/H4EK/tags",
            "C:/Kits/HREK/tags",
            r"C:\Kits\HREK\Tags",
        ] {
            assert_eq!(cache_root_key(Path::new(missing)), missing, "kept as given");
        }
        assert_eq!(
            cache_root_key(&existing),
            canonical.display().to_string(),
            "an existing root is canonicalized"
        );
        // Through a symlink (macOS's `/var`, unless TMPDIR says otherwise)
        // the canonical spelling differs from the one given.
        if canonical != existing {
            assert_ne!(cache_root_key(&existing), existing.display().to_string());
        }
    }
    #[cfg(windows)]
    {
        assert_eq!(
            cache_root_key(Path::new("Q:/Nonexistent-Baboon-Compat/HREK/Tags")),
            r"q:\nonexistent-baboon-compat\hrek\tags"
        );
        assert_eq!(
            cache_root_key(Path::new(r"\\?\Q:\Nonexistent-Baboon-Compat\Tags")),
            r"\\?\q:\nonexistent-baboon-compat\tags"
        );
        let key = cache_root_key(&existing);
        assert!(key.starts_with(r"\\?\"), "{key}");
        assert_eq!(key, canonical.display().to_string().to_ascii_lowercase());
        let shouted = existing.display().to_string().to_ascii_uppercase();
        assert_eq!(cache_root_key(Path::new(&shouted)), key, "case does not matter");
    }
    let _ = std::fs::remove_dir_all(&existing);
}

/// The sample database through the real row decoder. A row whose key names
/// no file (a bare key from before `file:`) is skipped, not fatal.
#[test]
fn compat_index_database_rows() {
    let conn = Connection::open_with_flags(
        samples().join("index/indexes.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open the sample index");
    let keys = |game: &str, root: &str| {
        load_entry_index_from_conn(&conn, game, Path::new(root)).map(|(entries, fingerprints)| {
            (
                entries.into_iter().map(|entry| entry.key).collect::<Vec<_>>(),
                fingerprints.len(),
            )
        })
    };
    let keys_file = sample_json("tag_keys.json");
    let key = |kind: &str| keys_file[kind].as_str().unwrap().to_owned();

    // Saved on a Unix host: the root key is the path, case and all.
    let h4 = keys("halo4_mcc", "/Users/me/Kits/H4EK/tags");
    // Saved on Windows for a root that did not canonicalize.
    let hr = keys("haloreach_mcc", "C:/Kits/HREK/tags");
    let hr_shouted = keys("haloreach_mcc", r"C:\KITS\hrek\TAGS");
    #[cfg(not(windows))]
    {
        assert_eq!(h4, Some((vec![key("file_posix")], 1)), "bare key skipped");
        assert_eq!(hr, None, "a Windows root key is not this host's spelling");
        assert_eq!(hr_shouted, None);
        assert_eq!(keys("halo4_mcc", "/users/me/kits/h4ek/tags"), None, "case matters here");
    }
    #[cfg(windows)]
    {
        assert_eq!(h4, None, "a Unix root key is not this host's spelling");
        assert_eq!(hr, Some((vec![key("file_mixed")], 1)));
        assert_eq!(hr_shouted, hr, "any case of the same root");
    }
    assert_eq!(keys("halo3_mcc", "/nowhere"), None);
}

/// The JSON indexes Genesis and early Baboon wrote, still read for migration.
#[test]
fn compat_legacy_json_index() {
    let h3_root = Path::new(
        r"C:\Program Files (x86)\Steam\steamapps\common\H3EK\tags",
    );
    let (entries, fingerprints) =
        parse_entry_index(h3_root, &sample_json("legacy_index/halo3_mcc_index.json"))
            .expect("halo3 index");
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].key,
        sample_json("tag_keys.json")["file_windows"].as_str().unwrap()
    );
    assert_eq!(fingerprints.len(), 1, "only the item with size and time");
    assert!(
        parse_entry_index(
            Path::new("/somewhere/else"),
            &sample_json("legacy_index/halo3_mcc_index.json")
        )
        .is_none(),
        "an index saved for another root is not this one's"
    );
    // One item without `file:` discards the whole legacy index. A known
    // hazard, pinned so a change to it is a decision rather than an accident.
    let reach_root = Path::new("C:/Kits/HREK/tags");
    assert!(
        parse_entry_index(
            reach_root,
            &sample_json("legacy_index/haloreach_mcc_index.json")
        )
        .is_none()
    );
}
