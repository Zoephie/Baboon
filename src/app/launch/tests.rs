use super::*;

/// A tag named on the command line gets the key the folder scan gives it,
/// even when the root is not in canonical form. On macOS the temp folder is
/// `/var/...`, which canonicalizes to `/private/var/...`; on Windows every
/// canonical path gains `\\?\`. Either way the launched tag used to be a
/// second entry, under a key the browser never uses.
#[test]
fn a_launched_tag_is_keyed_like_the_folder_scan() {
    let root = std::env::temp_dir().join(format!(
        "baboon-launch-key-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("objects")).unwrap();
    let mut header = [0u8; 64];
    header[48..52].copy_from_slice(&u32::from_be_bytes(*b"hlmt").to_le_bytes());
    header[60..64].copy_from_slice(b"MALB");
    std::fs::write(root.join("objects/a.model"), header).unwrap();
    let names = TagNameIndex::default();

    let launched =
        resolve_launch_tag_entries(&root, &[PathBuf::from("objects/a.model")], &names).unwrap();
    let scanned =
        crate::core::source::scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();

    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(launched.entries.len(), 1, "{:?}", launched.errors);
    assert_eq!(launched.entries[0].key, scanned[0].key);
    assert_eq!(launched.entries[0].display_path, scanned[0].display_path);
}
use std::time::{SystemTime, UNIX_EPOCH};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn unique_test_dir(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "baboon-command-line-{label}-{}-{stamp}",
        std::process::id()
    ))
}

fn write_classic_tag(path: &Path, group: &[u8; 4]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = [0u8; 64];
    bytes[36..40].copy_from_slice(group);
    bytes[60..64].copy_from_slice(b"blam");
    fs::write(path, bytes).unwrap();
}

#[test]
fn no_arguments_preserve_normal_startup() {
    let parsed = parse_startup_arguments(Vec::<OsString>::new());
    assert_eq!(parsed, StartupArguments::Normal);
    assert!(!parsed.suppresses_startup_popups());
}

#[test]
fn every_builtin_flag_and_alias_is_case_insensitive() {
    for (flag, game) in [
        ("-HCEEK", "haloce_mcc"),
        ("-h1ek", "haloce_mcc"),
        ("-H2EK", "halo2_mcc"),
        ("-h3ek", "halo3_mcc"),
        ("-H3ODSTEK", "halo3odst_mcc"),
        ("-hrek", "haloreach_mcc"),
        ("-H4EK", "halo4_mcc"),
        ("-H2AMPEK", "halo2amp_mcc"),
        ("-h2aek", "halo2amp_mcc"),
    ] {
        let StartupArguments::Launch(launch) =
            parse_startup_arguments(args(&[flag, "objects/example.weapon"]))
        else {
            panic!("{flag} should parse");
        };
        assert_eq!(launch.game.as_str(), game);
    }
}

#[test]
fn parser_preserves_multiple_unicode_paths() {
    let StartupArguments::Launch(launch) = parse_startup_arguments(args(&[
        "-HREK",
        "objects/éclair.weapon",
        "objects/二.model",
    ])) else {
        panic!("launch should parse");
    };
    assert_eq!(
        launch.tag_paths,
        [
            PathBuf::from("objects/éclair.weapon"),
            PathBuf::from("objects/二.model")
        ]
    );
    assert!(StartupArguments::Launch(launch).suppresses_startup_popups());
}

#[test]
fn malformed_command_lines_suppress_startup_popups() {
    for parsed in [
        parse_startup_arguments(args(&["-UNKNOWN", "objects/example.weapon"])),
        parse_startup_arguments(args(&["-HREK"])),
    ] {
        assert!(matches!(parsed, StartupArguments::Invalid(_)));
        assert!(parsed.suppresses_startup_popups());
    }
}

#[test]
fn paths_resolve_under_root_deduplicate_and_reject_outside_files() {
    let base = unique_test_dir("paths");
    let tags = base.join("tags");
    let outside = base.join("outside.weapon");
    let relative = PathBuf::from("objects")
        .join("path with spaces")
        .join("éclair.weapon");
    let inside = tags.join(&relative);
    fs::create_dir_all(inside.parent().unwrap()).unwrap();
    fs::write(&inside, b"tag").unwrap();
    fs::write(&outside, b"tag").unwrap();

    let resolved = resolve_launch_tag_paths(
        &tags,
        &[
            relative.clone(),
            inside.clone(),
            outside.clone(),
            PathBuf::from("../outside.weapon"),
            PathBuf::from("missing.weapon"),
        ],
    )
    .unwrap();

    assert_eq!(resolved.paths, [fs::canonicalize(&inside).unwrap()]);
    assert_eq!(resolved.errors.len(), 3);
    assert!(
        resolved
            .errors
            .iter()
            .any(|error| error.contains("outside"))
    );
    assert!(
        resolved
            .errors
            .iter()
            .any(|error| error.contains("missing.weapon"))
    );
    let _ = fs::remove_dir_all(base);
}

#[test]
fn forward_slash_relative_paths_resolve() {
    let base = unique_test_dir("slashes");
    let tags = base.join("tags");
    let inside = tags.join("objects").join("weapon.weapon");
    fs::create_dir_all(inside.parent().unwrap()).unwrap();
    fs::write(&inside, b"tag").unwrap();

    let resolved =
        resolve_launch_tag_paths(&tags, &[PathBuf::from("objects/weapon.weapon")]).unwrap();

    assert_eq!(resolved.paths, [fs::canonicalize(&inside).unwrap()]);
    let _ = fs::remove_dir_all(base);
}

#[test]
fn supported_entries_keep_argument_order_and_invalid_files_do_not_block_them() {
    let base = unique_test_dir("entries");
    let tags = base.join("tags");
    let first = tags.join("objects").join("first.weapon");
    let second = tags.join("objects").join("second.weapon");
    let unsupported = tags.join("objects").join("notes.txt");
    write_classic_tag(&first, b"weap");
    write_classic_tag(&second, b"weap");
    fs::write(&unsupported, b"not a tag").unwrap();

    let resolved = resolve_launch_tag_entries(
        &tags,
        &[
            PathBuf::from("objects/first.weapon"),
            PathBuf::from("objects/notes.txt"),
            first.clone(),
            PathBuf::from("objects/second.weapon"),
        ],
        &TagNameIndex::default(),
    )
    .unwrap();

    let entry_paths = resolved
        .entries
        .iter()
        .map(|entry| match &entry.location {
            TagEntryLocation::LooseFile(path) => path.clone(),
            _ => panic!("expected loose file"),
        })
        .collect::<Vec<_>>();
    // Spelled on the root the kit holds, as the folder scan spells them.
    // This used to expect canonical paths, which is what gave launched
    // tags keys the browser never uses.
    assert_eq!(entry_paths, [first.clone(), second.clone()]);
    assert_eq!(resolved.errors.len(), 1);
    assert!(resolved.errors[0].contains("not a supported tag"));
    let _ = fs::remove_dir_all(base);
}
