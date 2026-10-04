
use super::*;

/// A mod without `_P` mounts at the same priority as the game's own
/// containers and loses, so it builds correctly and does nothing. Renaming
/// the default to something meaningful is exactly how it gets dropped --
/// which is how one was reported.
/// A session written before the chosen folder was recorded holds the
/// resolved `Paks` directory. Restoring from it put that directory back
/// into the recents list on every launch, which is how "Paks" kept
/// reappearing however often it was removed.
#[test]
fn a_paks_directory_walks_back_up_to_the_opened_folder() {
    let root = std::env::temp_dir().join(format!("baboon-paks-{}", std::process::id()));
    let paks = root.join("Meteorite").join("Content").join("Paks");
    std::fs::create_dir_all(&paks).unwrap();
    // `find_paks_dir` needs a container present to recognise the folder.
    std::fs::write(paks.join("pakchunk0-WinGDK.utoc"), []).unwrap();

    assert_eq!(super::install_root_for_paks(&paks), root);
    // Already the opened folder: nothing to strip.
    assert_eq!(super::install_root_for_paks(&root), root);
    // The shorter layout the resolver also accepts.
    assert_eq!(
        super::install_root_for_paks(&root.join("Content").join("Paks")),
        root
    );
    // An unfamiliar layout is left exactly as it is rather than guessed at.
    let odd = root.join("somewhere").join("Paks");
    assert_eq!(super::install_root_for_paks(&odd), odd);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn normalize_container_tag_rel_cleans_path() {
    // Lowercases, normalizes separators, trims slashes, drops a leaf extension.
    assert_eq!(
        normalize_container_tag_rel("Objects\\Characters/Foo/Bar"),
        "objects/characters/foo/bar"
    );
    assert_eq!(
        normalize_container_tag_rel("/objects//foo/bar.biped/"),
        "objects/foo/bar"
    );
    assert_eq!(normalize_container_tag_rel("  Foo.Weapon  "), "foo");
    assert_eq!(normalize_container_tag_rel(""), "");
    assert_eq!(normalize_container_tag_rel("///"), "");
}
