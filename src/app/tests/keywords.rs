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
