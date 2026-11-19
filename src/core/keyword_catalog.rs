//! The keyword catalog: every keyword the user has, in their order, with how
//! each is spelled for display and its colour. One catalog for every game,
//! in its own file beside the per-game sidecars
//! ([`crate::core::keywords`]), which say which tags carry which keyword.
//!
//! Sidecars keep keywords lowercase and match without letter case; the
//! catalog is keyed the same way but keeps the spelling the user typed, so
//! "To Be Deleted" shows as typed. Colours are names here: the app maps each
//! to its light and dark theme colours.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A keyword's colour: one of a fixed set, each with a light and a dark
/// version in the theme. No colour draws as the neutral pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KeywordColour {
    Red,
    Orange,
    Yellow,
    Green,
    Teal,
    Blue,
    Purple,
    Pink,
    Grey,
}

impl KeywordColour {
    pub(crate) const ALL: [KeywordColour; 9] = [
        KeywordColour::Red,
        KeywordColour::Orange,
        KeywordColour::Yellow,
        KeywordColour::Green,
        KeywordColour::Teal,
        KeywordColour::Blue,
        KeywordColour::Purple,
        KeywordColour::Pink,
        KeywordColour::Grey,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            KeywordColour::Red => "Red",
            KeywordColour::Orange => "Orange",
            KeywordColour::Yellow => "Yellow",
            KeywordColour::Green => "Green",
            KeywordColour::Teal => "Teal",
            KeywordColour::Blue => "Blue",
            KeywordColour::Purple => "Purple",
            KeywordColour::Pink => "Pink",
            KeywordColour::Grey => "Grey",
        }
    }
}

/// One keyword: its display spelling and colour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KeywordInfo {
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) colour: Option<KeywordColour>,
}

impl KeywordInfo {
    /// The lowercase form the sidecars store and searches match.
    pub(crate) fn key(&self) -> String {
        keyword_key(&self.name)
    }
}

/// The lowercase form of a keyword, as the sidecars store it.
pub(crate) fn keyword_key(keyword: &str) -> String {
    keyword.trim().to_ascii_lowercase()
}

#[derive(Default, Serialize, Deserialize)]
struct CatalogFile {
    #[serde(default)]
    keywords: Vec<KeywordInfo>,
}

/// Every keyword, in the user's order; see the module docs. Cloned for a
/// frame's tables, which draw outside the app model's borrow.
#[derive(Clone, Default)]
pub(crate) struct KeywordCatalog {
    /// The catalog's file; `None` keeps it in memory only.
    path: Option<PathBuf>,
    entries: Vec<KeywordInfo>,
    /// Changed since it was last written.
    dirty: bool,
    /// Keys removed or renamed away since the last write, so a write doesn't
    /// bring them back from the file.
    removed: Vec<String>,
    /// A problem with the file the user has not been told about yet.
    notice: Option<String>,
}

impl KeywordCatalog {
    /// The catalog's file in Baboon's data folder.
    pub(crate) fn default_path() -> PathBuf {
        crate::core::storage::data_path("keywords.json")
    }

    /// Load the catalog at `path`; a missing file is an empty catalog. One
    /// that can't be read or parsed is left alone, and the catalog starts
    /// empty in memory without writing over it.
    pub(crate) fn load_at(&mut self, path: Option<PathBuf>) {
        self.entries.clear();
        self.removed.clear();
        self.dirty = false;
        let mut writable = true;
        if let Some(path) = path.as_deref() {
            match read_catalog(path) {
                Ok(entries) => self.entries = entries,
                Err(error) => {
                    self.notice = Some(error);
                    writable = false;
                }
            }
        }
        self.path = if writable { path } else { None };
    }

    /// The latest file problem, once; the shell shows it in the status line.
    pub(crate) fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    pub(crate) fn entries(&self) -> &[KeywordInfo] {
        &self.entries
    }

    pub(crate) fn position(&self, keyword: &str) -> Option<usize> {
        let key = keyword_key(keyword);
        self.entries.iter().position(|entry| entry.key() == key)
    }

    pub(crate) fn get(&self, keyword: &str) -> Option<&KeywordInfo> {
        self.position(keyword).map(|index| &self.entries[index])
    }

    /// How `keyword` is shown: its catalog spelling, or as stored.
    pub(crate) fn display<'a>(&'a self, keyword: &'a str) -> &'a str {
        self.get(keyword).map_or(keyword, |entry| entry.name.as_str())
    }

    pub(crate) fn colour(&self, keyword: &str) -> Option<KeywordColour> {
        self.get(keyword).and_then(|entry| entry.colour)
    }

    /// Add `keyword` at the end, spelled as given, unless it is already
    /// there. True when it was added.
    pub(crate) fn ensure(&mut self, keyword: &str) -> bool {
        let name = keyword.trim();
        if name.is_empty() || self.position(name).is_some() {
            return false;
        }
        let key = keyword_key(name);
        self.removed.retain(|removed| *removed != key);
        self.entries.push(KeywordInfo { name: name.to_owned(), colour: None });
        self.dirty = true;
        true
    }

    /// [`Self::ensure`] each of `keywords`, in alphabetical order so keywords
    /// found in a sidecar join the end in a stable order.
    pub(crate) fn ensure_all<'a>(&mut self, keywords: impl IntoIterator<Item = &'a str>) {
        let mut keywords: Vec<&str> = keywords.into_iter().collect();
        keywords.sort_unstable();
        for keyword in keywords {
            self.ensure(keyword);
        }
    }

    pub(crate) fn set_colour(&mut self, keyword: &str, colour: Option<KeywordColour>) {
        if let Some(index) = self.position(keyword)
            && self.entries[index].colour != colour
        {
            self.entries[index].colour = colour;
            self.dirty = true;
        }
    }

    /// Move the keyword at `from` so it lands at `to` in the new order.
    pub(crate) fn move_to(&mut self, from: usize, to: usize) {
        if from >= self.entries.len() || from == to {
            return;
        }
        let entry = self.entries.remove(from);
        let to = to.min(self.entries.len());
        self.entries.insert(to, entry);
        self.dirty = true;
    }

    /// Rename `old` to `new`. When `new` is another keyword already in the
    /// catalog, `old` folds into it, keeping that one's spelling, colour and
    /// place; true then. A rename that only changes letter case respells it.
    pub(crate) fn rename(&mut self, old: &str, new: &str) -> bool {
        let new = new.trim();
        let Some(from) = self.position(old) else {
            self.ensure(new);
            return false;
        };
        if new.is_empty() {
            return false;
        }
        match self.position(new) {
            Some(into) if into != from => {
                let gone = self.entries.remove(from);
                self.removed.push(gone.key());
                self.dirty = true;
                true
            }
            _ => {
                if self.entries[from].name != new {
                    let old_key = self.entries[from].key();
                    if old_key != keyword_key(new) {
                        self.removed.push(old_key);
                    }
                    self.entries[from].name = new.to_owned();
                    self.dirty = true;
                }
                false
            }
        }
    }

    pub(crate) fn remove(&mut self, keyword: &str) {
        if let Some(index) = self.position(keyword) {
            let gone = self.entries.remove(index);
            self.removed.push(gone.key());
            self.dirty = true;
        }
    }

    fn forgotten(&self, entry: &KeywordInfo) -> bool {
        self.removed.contains(&entry.key())
    }

    /// Where `keyword` sorts: its place in the catalog, or after all of them.
    pub(crate) fn rank(&self, keyword: &str) -> usize {
        self.position(keyword).unwrap_or(usize::MAX)
    }

    /// `keywords` in catalog order; any the catalog lacks follow, A–Z.
    pub(crate) fn ordered(&self, keywords: &[String]) -> Vec<String> {
        let mut keywords = keywords.to_vec();
        keywords.sort_by(|a, b| self.rank(a).cmp(&self.rank(b)).then_with(|| a.cmp(b)));
        keywords
    }

    /// Write the catalog if it changed. Keywords another Baboon wrote to the
    /// file since this one loaded it are kept, at the end.
    pub(crate) fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.dirty = false;
            return;
        };
        if let Ok(on_disk) = read_catalog(&path) {
            for entry in on_disk {
                if self.position(&entry.name).is_none() && !self.forgotten(&entry) {
                    self.entries.push(entry);
                }
            }
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = CatalogFile { keywords: self.entries.clone() };
        let result = serde_json::to_string_pretty(&file)
            .map_err(|error| error.to_string())
            .and_then(|text| crate::core::keywords::write_atomic(&path, &text));
        match result {
            Ok(()) => {
                self.dirty = false;
                self.removed.clear();
            }
            Err(error) => {
                self.notice = Some(format!("Keywords not saved to {}: {error}", path.display()));
                self.dirty = false;
            }
        }
    }
}

/// The catalog at `path`; a missing file is empty, not an error.
fn read_catalog(path: &Path) -> Result<Vec<KeywordInfo>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!("Could not read the keyword list {}: {error}", path.display()));
        }
    };
    serde_json::from_str::<CatalogFile>(&text)
        .map(|file| file.keywords)
        .map_err(|error| format!("Could not parse the keyword list {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    //! Unit tests for the keyword catalog's order, spelling, colours and file.

    use super::*;
    use crate::core::test_kits::unique_temp_dir;

    fn names(catalog: &KeywordCatalog) -> Vec<&str> {
        catalog.entries().iter().map(|entry| entry.name.as_str()).collect()
    }

    /// Keywords keep their spelling, sort in the user's order, and a
    /// keyword the list lacks follows them.
    #[test]
    fn keywords_show_as_spelled_and_sort_in_the_users_order() {
        let mut catalog = KeywordCatalog::default();
        assert!(catalog.ensure("To Be Deleted"));
        assert!(catalog.ensure("WIP"));
        assert!(!catalog.ensure("wip"), "letter case aside, it's there");
        catalog.ensure_all(["zeta", "alpha"]);
        assert_eq!(names(&catalog), ["To Be Deleted", "WIP", "alpha", "zeta"]);
        assert_eq!(catalog.display("to be deleted"), "To Be Deleted");
        assert_eq!(catalog.display("unknown"), "unknown");
        catalog.move_to(1, 0);
        let keywords = ["zeta", "unknown", "to be deleted", "wip"].map(str::to_owned);
        assert_eq!(catalog.ordered(&keywords), ["wip", "to be deleted", "zeta", "unknown"]);
        catalog.move_to(0, 9);
        assert_eq!(names(&catalog), ["To Be Deleted", "alpha", "zeta", "WIP"]);
    }

    /// A rename to another keyword folds into it; one that only changes
    /// letter case respells it; colours stay with their keyword.
    #[test]
    fn renames_respell_or_merge_and_colours_follow() {
        let mut catalog = KeywordCatalog::default();
        catalog.ensure_all(["brute", "brutes"]);
        catalog.set_colour("brute", Some(KeywordColour::Red));
        assert!(!catalog.rename("brute", "Brute"));
        assert_eq!(catalog.colour("brute"), Some(KeywordColour::Red));
        assert!(catalog.rename("brutes", "BRUTE"), "merged");
        assert_eq!(names(&catalog), ["Brute"]);
        catalog.remove("brute");
        assert!(catalog.entries().is_empty());
    }

    /// The file keeps order, spelling and colour; a write keeps keywords
    /// another Baboon added since, but not ones removed here.
    #[test]
    fn the_file_round_trips_and_keeps_another_baboons_keywords() {
        let dir = unique_temp_dir("keyword_catalog");
        let path = dir.join("keywords.json");
        let mut catalog = KeywordCatalog::default();
        catalog.load_at(Some(path.clone()));
        catalog.ensure_all(["wip", "old"]);
        catalog.set_colour("wip", Some(KeywordColour::Orange));
        catalog.save_if_dirty();

        let mut other = KeywordCatalog::default();
        other.load_at(Some(path.clone()));
        assert_eq!(names(&other), ["old", "wip"]);
        assert_eq!(other.colour("WIP"), Some(KeywordColour::Orange));
        other.ensure("Ready");
        other.save_if_dirty();

        catalog.remove("old");
        catalog.save_if_dirty();
        let mut reread = KeywordCatalog::default();
        reread.load_at(Some(path));
        assert_eq!(names(&reread), ["wip", "Ready"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A file that can't be parsed is left as it is, not written over.
    #[test]
    fn an_unparseable_file_is_not_written_over() {
        let dir = unique_temp_dir("keyword_catalog_bad");
        let path = dir.join("keywords.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "not json").unwrap();
        let mut catalog = KeywordCatalog::default();
        catalog.load_at(Some(path.clone()));
        assert!(catalog.take_notice().is_some());
        catalog.ensure("wip");
        catalog.save_if_dirty();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
        let _ = std::fs::remove_dir_all(dir);
    }
}
