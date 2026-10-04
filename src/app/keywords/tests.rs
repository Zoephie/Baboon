//! Unit tests for keyword-store normalization and lookup.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;

#[test]
fn add_dedupes_and_remove_clears() {
    let mut store = KeywordStore::default();
    store.add("file:a", "Hero");
    store.add("file:a", "hero"); // case-insensitive dedupe
    store.add("file:a", "wip");
    assert_eq!(store.keywords("file:a"), &["hero", "wip"]);
    assert_eq!(
        store.all_keywords(),
        vec![("hero".to_owned(), 1), ("wip".to_owned(), 1)]
    );
    assert_eq!(store.tags_with("wip"), vec!["file:a".to_owned()]);
    store.remove("file:a", "hero");
    assert_eq!(store.keywords("file:a"), &["wip"]);
    store.remove("file:a", "wip");
    assert!(store.keywords("file:a").is_empty());
}

/// Two kits of one game share its sidecar. Each used to write back the whole
/// map it loaded, so whichever saved second erased the first one's keywords.
#[test]
fn two_kits_of_one_game_keep_each_others_keywords() {
    let dir = std::env::temp_dir().join(format!("baboon-keywords-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("halo2_mcc_keywords.json");
    let mut first = KeywordStore::default();
    let mut second = KeywordStore::default();
    first.load_at(Some(path.clone()));
    second.load_at(Some(path.clone()));

    first.add("file:a", "hero");
    first.save_if_dirty();
    second.add("file:b", "wip");
    second.save_if_dirty();
    // A removal is a change too, and must not bring back what was removed.
    first.remove("file:a", "hero");
    first.save_if_dirty();

    let mut reloaded = KeywordStore::default();
    reloaded.load_at(Some(path));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(reloaded.keywords("file:a").is_empty());
    assert_eq!(reloaded.keywords("file:b"), &["wip"]);
    assert_eq!(second.keywords("file:b"), &["wip"]);
    assert_eq!(
        first.keywords("file:b"),
        &["wip"],
        "the first kit sees the second's on its next save"
    );
}

/// A sidecar that cannot be parsed used to read as an empty map; the next save
/// then laid this kit's one change over that and wrote it back, erasing every
/// other keyword for the game.
#[test]
fn an_unparseable_sidecar_is_kept_not_overwritten() {
    let dir = std::env::temp_dir().join(format!("baboon-keywords-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("halo3_mcc_keywords.json");
    // A sidecar from a crash mid-write: valid keywords, cut short.
    let original = "{\n  \"file:a\": [\n    \"hero\"\n  ],\n  \"file:b\": [\n    \"wi";
    std::fs::write(&path, original).unwrap();

    let mut store = KeywordStore::default();
    store.load_at(Some(path.clone()));
    assert!(store.take_notice().is_some(), "the user is told the file was unreadable");
    store.add("file:c", "new");
    store.save_if_dirty();
    let notice = store.take_notice();

    let kept = dir.join("halo3_mcc_keywords.json.unreadable");
    let kept_text = std::fs::read_to_string(&kept).ok();
    let mut reloaded = KeywordStore::default();
    reloaded.load_at(Some(path));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        kept_text.as_deref(),
        Some(original),
        "the unparseable sidecar survives, byte for byte"
    );
    assert!(notice.is_some_and(|notice| notice.contains("unreadable")));
    assert_eq!(reloaded.keywords("file:c"), &["new"]);
}

/// A sidecar that exists but cannot be read is never written over: the
/// change stays pending until the file can be read again.
#[cfg(unix)]
#[test]
fn an_unreadable_sidecar_keeps_the_change_pending() {
    let dir = std::env::temp_dir().join(format!("baboon-keywords-unreadable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A directory where the file should be reads as an error other than
    // "not found" on every platform user, root included.
    let path = dir.join("halo3_mcc_keywords.json");
    std::fs::create_dir_all(&path).unwrap();

    let mut store = KeywordStore::default();
    store.path = Some(path.clone());
    store.add("file:c", "new");
    store.save_if_dirty();
    assert!(store.take_notice().is_some_and(|notice| notice.contains("could not read")));
    assert!(path.is_dir(), "nothing replaced what is at the sidecar path");

    std::fs::remove_dir_all(&path).unwrap();
    std::fs::write(&path, "{\"file:a\": [\"hero\"]}").unwrap();
    store.save_if_dirty();
    let mut reloaded = KeywordStore::default();
    reloaded.load_at(Some(path));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(reloaded.keywords("file:a"), &["hero"]);
    assert_eq!(reloaded.keywords("file:c"), &["new"], "the pending change lands once readable");
}
