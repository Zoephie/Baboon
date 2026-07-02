//! User keyword tags, stored in a per-game sidecar JSON (outside the tag
//! binaries). Keyed by tag entry key → sorted, unique, lowercased keywords.

use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct KeywordStore {
    game: Option<String>,
    by_tag: BTreeMap<String, Vec<String>>,
    dirty: bool,
}

impl KeywordStore {
    /// Load the sidecar for `game` (clears state for `None` / non-folder sources).
    pub(super) fn load_for_game(&mut self, game: Option<&str>) {
        self.by_tag.clear();
        self.dirty = false;
        self.game = game.map(str::to_owned);
        if let Some(game) = game {
            if let Ok(text) = std::fs::read_to_string(crate::source::keywords_path(game)) {
                if let Ok(map) = serde_json::from_str::<BTreeMap<String, Vec<String>>>(&text) {
                    self.by_tag = map;
                }
            }
        }
    }

    pub(super) fn keywords(&self, tag_key: &str) -> &[String] {
        self.by_tag.get(tag_key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(super) fn add(&mut self, tag_key: &str, keyword: &str) {
        let keyword = keyword.trim().to_ascii_lowercase();
        if keyword.is_empty() {
            return;
        }
        let list = self.by_tag.entry(tag_key.to_owned()).or_default();
        if !list.iter().any(|existing| existing == &keyword) {
            list.push(keyword);
            list.sort();
            self.dirty = true;
        }
    }

    pub(super) fn remove(&mut self, tag_key: &str, keyword: &str) {
        if let Some(list) = self.by_tag.get_mut(tag_key) {
            let before = list.len();
            list.retain(|existing| existing != keyword);
            let changed = list.len() != before;
            if list.is_empty() {
                self.by_tag.remove(tag_key);
            }
            if changed {
                self.dirty = true;
            }
        }
    }

    /// All keywords with how many tags carry each, sorted by name.
    pub(super) fn all_keywords(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for keywords in self.by_tag.values() {
            for keyword in keywords {
                *counts.entry(keyword.clone()).or_default() += 1;
            }
        }
        counts.into_iter().collect()
    }

    /// Tag keys carrying `keyword`.
    pub(super) fn tags_with(&self, keyword: &str) -> Vec<String> {
        self.by_tag
            .iter()
            .filter(|(_, kws)| kws.iter().any(|k| k == keyword))
            .map(|(key, _)| key.clone())
            .collect()
    }

    pub(super) fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let Some(game) = self.game.as_deref() else {
            return;
        };
        let path = crate::source::keywords_path(game);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.by_tag) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
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
}
