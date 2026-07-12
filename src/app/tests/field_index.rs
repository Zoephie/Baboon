//! Unit tests for field-index generation and query limits.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;

#[test]
fn install_query_and_generation_invalidation() {
    let mut index = FieldValueIndex::default();
    assert!(!index.is_ready_for(1));
    index.install(
        1,
        vec![
            ("a".to_owned(), "weapons · objects\\rifle".to_owned()),
            ("b".to_owned(), "bipeds · masterchief".to_owned()),
        ],
    );
    assert!(index.is_ready_for(1));
    // Wrong generation → treated as not ready.
    assert!(!index.is_ready_for(2));
    let hits = index.query("rifle", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "a");
    assert!(hits[0].1.contains("rifle"));
    assert_eq!(index.query("chief", 10).len(), 1);
    assert!(index.query("nonexistent", 10).is_empty());
    index.invalidate();
    assert!(!index.is_ready_for(1));
    assert!(index.query("rifle", 10).is_empty());
}

#[test]
fn query_respects_cap() {
    let mut index = FieldValueIndex::default();
    index.install(
        1,
        (0..50)
            .map(|i| (format!("k{i}"), "shared token".to_owned()))
            .collect(),
    );
    assert_eq!(index.query("token", 10).len(), 10);
}
