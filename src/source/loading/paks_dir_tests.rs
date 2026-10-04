use super::*;

fn temp_dir(name: &str) -> PathBuf {
    crate::test_kits::unique_temp_path(&format!("paks-{name}"))
}

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("has a parent")).expect("create dirs");
    std::fs::write(path, b"").expect("write file");
}

/// Baboon's own transactional artefacts are not containers. A duplicate
/// leaves an immutable copy of the `.utoc` beside the container it is about
/// to mutate, and an export builds its replacement in a hidden folder
/// inside the destination — mounting either would show the user a mod made
/// of a half-finished write, and shipping either would put it in a mod.
#[test]
fn baboons_own_backups_are_never_mounted() {
    let root = temp_dir("backups");
    touch(&root.join("pakchunk0-WinGDK.utoc"));
    touch(&root.join("pakchunk0-WinGDK.utoc.baboon-duplicate-backup"));
    touch(&root.join("pakchunk0-WinGDK.utoc.baboon-duplicate-backup-3"));
    touch(&root.join("~mods/mymod_P.utoc"));
    touch(&root.join("~mods/mymod_P.utoc.previous"));
    touch(&root.join("~mods/.baboon-export-1234-9/mymod_P.utoc"));

    let mut mounted: Vec<String> = utocs_under(&root)
        .iter()
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    mounted.sort();

    assert_eq!(
        mounted,
        vec![
            "pakchunk0-WinGDK.utoc".to_owned(),
            "~mods/mymod_P.utoc".to_owned()
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

fn container_entry(
    key: &str,
    display_path: &str,
    container: usize,
    rel_path: &str,
) -> TagEntry {
    TagEntry {
        key: key.to_owned(),
        display_path: display_path.to_owned(),
        group_tag: 0x62697064,
        group_name: Some("biped".to_owned()),
        location: TagEntryLocation::Container {
            container,
            rel_path: rel_path.to_owned(),
        },
    }
}

/// Mounting a mod over a tag must not change what that tag *is*.
///
/// The key is what open tabs, parsed documents and the undo journal are
/// filed under. Replacing the entry wholesale gave the tag a new identity
/// derived from the mod's container label, so every tab of a just-exported
/// tag showed its raw key and "This tag is no longer in the source", with
/// the user's edits stranded behind it.
#[test]
fn a_mod_layered_over_a_tag_leaves_its_identity_alone() {
    let mut entries = vec![container_entry(
        "ublock:pakchunk0-Windows:Meteorite/Content/Tags/objects/brute-biped.ubulk",
        "objects/brute.biped",
        0,
        "Meteorite/Content/Tags/objects/brute-biped.ubulk",
    )];

    layer_entry(
        &mut entries,
        &container_entry(
            "ublock:mymod_P:Meteorite/Content/Tags/objects/brute-biped.ubulk",
            "objects/brute.biped",
            4,
            "Meteorite/Content/Tags/objects/brute-biped.ubulk",
        ),
    );

    assert_eq!(entries.len(), 1, "the mod replaces rather than duplicates");
    assert_eq!(
        entries[0].key,
        "ublock:pakchunk0-Windows:Meteorite/Content/Tags/objects/brute-biped.ubulk",
        "the tag keeps the identity its open tab is filed under"
    );
    // What did change is where it is read from.
    assert!(matches!(
        &entries[0].location,
        TagEntryLocation::Container { container: 4, .. }
    ));
}

#[test]
fn a_tag_only_the_mod_carries_is_added_in_sorted_position() {
    let mut entries = vec![
        container_entry("ublock:pak:a.ubulk", "objects/a.biped", 0, "a.ubulk"),
        container_entry("ublock:pak:z.ubulk", "objects/z.biped", 0, "z.ubulk"),
    ];

    layer_entry(
        &mut entries,
        &container_entry("ublock:mymod_P:m.ubulk", "objects/m.biped", 4, "m.ubulk"),
    );

    let order: Vec<&str> = entries
        .iter()
        .map(|entry| entry.display_path.as_str())
        .collect();
    assert_eq!(
        order,
        ["objects/a.biped", "objects/m.biped", "objects/z.biped"],
        "a new tag lands beside its neighbours, not at the bottom of the list"
    );
}

#[test]
fn a_backup_is_recognised_wherever_it_sits() {
    for path in [
        "D:/Paks/pakchunk0.utoc.baboon-duplicate-backup",
        "D:/Paks/pakchunk0.utoc.baboon-duplicate-backup-7",
        "D:/Paks/pakchunk0.utoc.baboon-duplicate-backup.manifest.json",
        "D:/Paks/~mods/mymod_P.utoc.previous",
        "D:/Paks/~mods/.baboon-export-900-1/mymod_P.utoc",
    ] {
        assert!(is_container_backup(Path::new(path)), "{path}");
    }
    for path in [
        "D:/Paks/pakchunk0.utoc",
        "D:/Paks/~mods/mymod_P.utoc",
        "D:/Paks/~mods/baboon-export/mymod_P.utoc",
    ] {
        assert!(!is_container_backup(Path::new(path)), "{path}");
    }
}

/// The game root of an install that has had a mod exported into it: a
/// stray `.utoc` sits beside the executable. The real containers must win.
#[test]
fn the_install_layout_beats_a_stray_container_in_the_root() {
    let root = temp_dir("stray");
    let paks = root.join("Meteorite").join("Content").join("Paks");
    touch(&root.join("mymod-WinGDK_P.utoc"));
    touch(&paks.join("pakchunk0-WinGDK.utoc"));

    assert_eq!(find_paks_dir(&root), Some(paks));
    let _ = std::fs::remove_dir_all(&root);
}

/// Picking the `Paks` directory itself still resolves to itself.
#[test]
fn picking_the_paks_directory_resolves_to_itself() {
    let root = temp_dir("direct");
    let paks = root.join("Meteorite").join("Content").join("Paks");
    touch(&paks.join("pakchunk0-WinGDK.utoc"));

    assert_eq!(find_paks_dir(&paks), Some(paks.clone()));
    let _ = std::fs::remove_dir_all(&root);
}

/// A bare directory of containers not named `Paks` is still accepted, as
/// the last resort rather than the first check.
#[test]
fn a_bare_container_directory_is_still_accepted() {
    let root = temp_dir("bare");
    touch(&root.join("pakchunk0-WinGDK.utoc"));

    assert_eq!(find_paks_dir(&root), Some(root.clone()));
    let _ = std::fs::remove_dir_all(&root);
}

/// The real install, which is what surfaced this: its root holds an
/// exported `mymod-WinGDK_P.utoc`. Skipped when the game is not present.
#[test]
fn the_real_install_root_resolves_to_its_paks_directory() {
    static ROOT: std::sync::LazyLock<&'static str> =
        std::sync::LazyLock::new(|| crate::test_kits::leak(crate::test_kits::ce_install()));
    if !Path::new(*ROOT).is_dir() {
        return;
    }
    assert_eq!(
        find_paks_dir(Path::new(*ROOT)),
        Some(
            PathBuf::from(*ROOT)
                .join("Meteorite")
                .join("Content")
                .join("Paks")
        )
    );
}

#[test]
fn a_folder_with_no_containers_is_not_an_install() {
    let root = temp_dir("none");
    touch(&root.join("readme.txt"));

    assert_eq!(find_paks_dir(&root), None);
    let _ = std::fs::remove_dir_all(&root);
}
