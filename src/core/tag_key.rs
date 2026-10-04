//! Entry keys: the strings that name a tag in the browser, the document set,
//! sessions, keyword sidecars, favourites and the index database.
//!
//! A key's spelling is a file format, so every kind is built here and nowhere
//! else, and read back here too; `compat_keys_tests` pins every spelling.

use std::path::Path;

use blam_tags::format_group_tag;

/// A loose tag file's key: `file:` and the path exactly as displayed. Never
/// normalized: the path is the root as the user gave it joined with what the
/// walk found, so it can mix separators, and splitting it on `:` breaks drive
/// letters.
pub(crate) fn file_entry_key(path: &Path) -> String {
    format!("file:{}", path.display())
}

/// A monolithic cache tag's key: its group with trailing spaces trimmed (`rm`
/// for `rm  `) and its name as the cache stores it, backslashes included.
pub(crate) fn cache_entry_key(group_tag: u32, name: &str) -> String {
    format!("cache:{}:{name}", format_group_tag(group_tag))
}

/// A Campaign Evolved container tag's key: the container's label and the
/// payload path in its original case.
pub(crate) fn container_entry_key(chunk_label: &str, rel_path: &str) -> String {
    format!("ublock:{chunk_label}:{rel_path}")
}

/// A new Campaign Evolved tag's key, before it has a container: `newtag:` and
/// its package path. Prefixed so it cannot collide with a mounted container
/// tag's key.
pub(crate) fn new_tag_entry_key(package: &str) -> String {
    format!("newtag:{package}")
}

/// The path a loose tag's key names, or `None` for any other kind of key.
pub(crate) fn file_key_path(key: &str) -> Option<&Path> {
    key.strip_prefix("file:").map(Path::new)
}

/// How a key reads to a person: a loose tag's path, or any other key as it is.
pub(crate) fn key_label(key: &str) -> &str {
    key.strip_prefix("file:").unwrap_or(key)
}

/// Whether two keys name the same tag: `file:` keys hold paths, which
/// Windows compares without case.
pub(crate) fn same_entry_key(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a.eq_ignore_ascii_case(b)
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
