//! User keyword tags, stored in a per-game sidecar JSON (outside the tag
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.
//! binaries). Keyed by tag entry key → sorted, unique, lowercased keywords.

use std::collections::{BTreeMap, BTreeSet};

/// One kit's view of its game's keyword sidecar.
///
/// Every kit of a game shares the one sidecar, and several can be open at
/// once (kits sharing a root, or two copies of a kit). So a save writes only
/// the tags this kit changed over what is on disk now, rather than the whole
/// map as this kit loaded it, which dropped every other kit's changes made
/// since.
#[derive(Default)]
pub(super) struct KeywordStore {
    /// The game's sidecar file; `None` for a source with no game.
    path: Option<std::path::PathBuf>,
    by_tag: BTreeMap<String, Vec<String>>,
    /// Tag keys whose keywords this kit changed since it last saved.
    touched: BTreeSet<String>,
    /// A problem with the sidecar the user has not been told about yet.
    notice: Option<String>,
}

impl KeywordStore {
    /// Load the sidecar for `game` (clears state for `None` / non-folder sources).
    pub(super) fn load_for_game(&mut self, game: Option<&str>) {
        self.load_at(game.map(crate::source::keywords_path));
    }

    /// Load the sidecar at `path`; `None` leaves the store empty.
    pub(super) fn load_at(&mut self, path: Option<std::path::PathBuf>) {
        self.touched.clear();
        self.by_tag = match path.as_deref().map(read_sidecar) {
            Some(Ok(by_tag)) => by_tag,
            Some(Err(error)) => {
                self.notice = Some(error.message());
                BTreeMap::new()
            }
            None => BTreeMap::new(),
        };
        self.path = path;
    }

    /// The latest sidecar problem, once; the shell shows it in the status line.
    pub(super) fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
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
            self.touched.insert(tag_key.to_owned());
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
                self.touched.insert(tag_key.to_owned());
            }
        }
    }

    /// Drop every keyword attached to a tag that no longer exists, so a deleted
    /// tag stops appearing in keyword browsing and its rows leave the sidecar.
    pub(super) fn forget_tag(&mut self, tag_key: &str) {
        if self.by_tag.remove(tag_key).is_some() {
            self.touched.insert(tag_key.to_owned());
        }
    }

    /// Carry a tag's keywords to its new key after a rename.
    ///
    /// The sidecar outlives the session, so a rename that skipped this would
    /// leave the keywords filed under a key nothing resolves any more — the tag
    /// would silently lose them, and keyword browsing would list a path that no
    /// longer exists. Any keywords already at `new_key` are merged rather than
    /// replaced, because the destination may be a path that was in use before.
    pub(super) fn rekey_tag(&mut self, old_key: &str, new_key: &str) {
        if old_key == new_key {
            return;
        }
        let Some(moved) = self.by_tag.remove(old_key) else {
            return;
        };
        let list = self.by_tag.entry(new_key.to_owned()).or_default();
        for keyword in moved {
            if !list.iter().any(|existing| existing == &keyword) {
                list.push(keyword);
            }
        }
        list.sort();
        self.touched.insert(old_key.to_owned());
        self.touched.insert(new_key.to_owned());
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

    /// Lay this kit's changes over the sidecar as it is on disk now and write
    /// it back.
    ///
    /// The merge base must be what is really on disk. An unreadable or
    /// unparseable sidecar used to read as empty, so the save that followed
    /// wrote back only this kit's few changes and erased every other keyword
    /// for the game. Now a sidecar that cannot be read leaves the changes
    /// pending, and one that cannot be parsed is moved aside, kept, before a
    /// fresh one is written.
    pub(super) fn save_if_dirty(&mut self) {
        if self.touched.is_empty() {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.touched.clear();
            return;
        };
        // What another kit of this game saved since this one loaded, with
        // this kit's changes laid over it; this kit then sees theirs too.
        let mut merged = match read_sidecar(&path) {
            Ok(merged) => merged,
            Err(SidecarError::Unreadable(error)) => {
                // Possibly transient (a sharing violation, a permission
                // change): keep the changes and try again on a later frame,
                // telling the user once.
                let message = format!(
                    "Keywords not saved: could not read {}: {error}",
                    path.display()
                );
                if self.notice.as_deref() != Some(message.as_str()) {
                    self.notice = Some(message);
                }
                return;
            }
            Err(SidecarError::Unparseable(error)) => match set_aside(&path) {
                Ok(kept) => {
                    self.notice = Some(format!(
                        "The keyword file {} could not be parsed ({error}); it was kept as {} and a new one started.",
                        path.display(),
                        kept.display()
                    ));
                    BTreeMap::new()
                }
                Err(move_error) => {
                    self.notice = Some(format!(
                        "Keywords not saved: {} could not be parsed ({error}) or moved aside ({move_error}).",
                        path.display()
                    ));
                    return;
                }
            },
        };
        let touched = std::mem::take(&mut self.touched);
        for key in touched {
            match self.by_tag.get(&key) {
                Some(keywords) => {
                    merged.insert(key, keywords.clone());
                }
                None => {
                    merged.remove(&key);
                }
            }
        }
        self.by_tag = merged;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let result = serde_json::to_string_pretty(&self.by_tag)
            .map_err(|error| error.to_string())
            .and_then(|text| write_atomic(&path, &text));
        if let Err(error) = result {
            self.notice = Some(format!(
                "Keywords not saved to {}: {error}",
                path.display()
            ));
        }
    }
}

/// Why a sidecar on disk could not be used as a merge base.
#[derive(Debug)]
enum SidecarError {
    /// The file exists but could not be read.
    Unreadable(String),
    /// The file was read but is not a keyword map.
    Unparseable(String),
}

impl SidecarError {
    fn message(&self) -> String {
        match self {
            SidecarError::Unreadable(error) => format!("Could not read the keyword file: {error}"),
            SidecarError::Unparseable(error) => {
                format!("Could not parse the keyword file: {error}")
            }
        }
    }
}

/// The sidecar at `path`; a missing file is an empty map, not an error.
fn read_sidecar(path: &std::path::Path) -> Result<BTreeMap<String, Vec<String>>, SidecarError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(SidecarError::Unreadable(error.to_string())),
    };
    serde_json::from_str(&text).map_err(|error| SidecarError::Unparseable(error.to_string()))
}

/// Rename an unparseable sidecar to a free `.unreadable` name beside it.
fn set_aside(path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    for attempt in 0u32.. {
        let suffix = if attempt == 0 {
            ".unreadable".to_owned()
        } else {
            format!(".unreadable.{attempt}")
        };
        let kept = path.with_file_name(format!("{file_name}{suffix}"));
        if kept.exists() {
            continue;
        }
        std::fs::rename(path, &kept)?;
        return Ok(kept);
    }
    unreachable!("an unbounded range always yields")
}

/// Replace `path` with `text` so a crash leaves either the old file or the
/// new one, never a truncated one.
fn write_atomic(path: &std::path::Path, text: &str) -> Result<(), String> {
    use std::io::Write as _;
    let mut file =
        atomic_write_file::AtomicWriteFile::open(path).map_err(|error| error.to_string())?;
    file.write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    file.commit().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
