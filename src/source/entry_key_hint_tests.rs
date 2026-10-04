use super::*;

fn entry(key: &str) -> TagEntry {
    TagEntry {
        key: key.to_owned(),
        display_path: key.to_owned(),
        group_tag: 0,
        group_name: None,
        location: TagEntryLocation::LooseFile(PathBuf::from(key)),
    }
}

fn source(entries: Vec<TagEntry>, all_entries: Vec<TagEntry>) -> LoadedSourceData {
    LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::SingleFile {
            path: PathBuf::from("a"),
        },
        names: TagNameIndex::default(),
        game: None,
        entries,
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries,
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    }
}

fn found<'a>(source: &'a LoadedSourceData, key: &str) -> Option<&'a str> {
    source
        .entry_for_key(key)
        .map(|entry| entry.display_path.as_str())
}

/// The lists are mutated in many places behind the hints' back. Whatever
/// happens to them, a lookup answers exactly what a scan would.
#[test]
fn key_lookups_stay_right_as_the_lists_change_under_them() {
    let mut source = source(vec![entry("a"), entry("b")], vec![entry("c")]);
    assert_eq!(found(&source, "b"), Some("b"));
    assert_eq!(found(&source, "c"), Some("c"), "found in the full scan");
    assert_eq!(found(&source, "b"), Some("b"), "and again from the hint");

    // Inserting ahead of a remembered key moves it: the stale hint is
    // caught, not trusted.
    source.entries.insert(0, entry("z"));
    assert_eq!(found(&source, "b"), Some("b"));

    // The browser's lazy loader appends; a key nobody asked about before
    // is found by the fallback.
    source.entries.push(entry("d"));
    assert_eq!(found(&source, "d"), Some("d"));

    // A removed key is gone, even though its hint pointed at a real slot.
    source.entries.retain(|entry| entry.key != "b");
    assert_eq!(found(&source, "b"), None);
    source.all_entries.clear();
    assert_eq!(found(&source, "c"), None);
    assert_eq!(found(&source, "missing"), None);
}

/// A key found once is found again without scanning, which is the point.
#[test]
fn a_repeated_key_lookup_does_not_scan_again() {
    let entries: Vec<TagEntry> = (0..1000).map(|index| entry(&format!("k{index}"))).collect();
    let source = source(entries, Vec::new());
    assert_eq!(found(&source, "k999"), Some("k999"));
    let before = KEY_SCANS.with(std::cell::Cell::get);
    for _ in 0..100 {
        assert_eq!(found(&source, "k999"), Some("k999"));
    }
    assert_eq!(KEY_SCANS.with(std::cell::Cell::get), before);
}

/// Packages layer as tags do: the last-mounted container is read, and
/// removing one container's copy leaves the others.
#[test]
fn a_mods_package_overrides_the_games_until_it_is_deleted() {
    const PACKAGE: &str = "/game/tags/sound/x-sound";
    let mut packages = ContainerPackageIndex::default();
    packages.insert(PACKAGE.to_owned(), 0, "Game/x-sound.uasset".to_owned());
    packages.insert(PACKAGE.to_owned(), 5, "Mod/x-sound.uasset".to_owned());
    assert_eq!(
        packages.lookup("/Game/Tags/Sound/X-Sound"),
        Some((5, "Mod/x-sound.uasset"))
    );

    // A rename inside the game's container rewrites its copy, beneath the
    // mod's.
    packages.insert(PACKAGE.to_owned(), 0, "Game/renamed.uasset".to_owned());
    assert_eq!(packages.lookup(PACKAGE), Some((5, "Mod/x-sound.uasset")));

    assert!(packages.remove(PACKAGE, 5), "the mod's copy is deleted");
    assert_eq!(packages.lookup(PACKAGE), Some((0, "Game/renamed.uasset")));
    assert!(!packages.remove(PACKAGE, 5));
    assert!(packages.remove(PACKAGE, 0));
    assert_eq!(packages.lookup(PACKAGE), None);
    assert!(packages.is_empty());
}
