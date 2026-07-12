//! Tag-source discovery, loading, indexing, and browser-tree construction.
//! It owns source identity, discovery, indexing, and source-aware reads; editor presentation and application workflow state belong elsewhere.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, anyhow};
use blam_tags::classic::{ClassicHeader, read_classic_tag_file};
use blam_tags::monolithic::MonolithicCache;
use blam_tags::paths::group_tag_to_extension;
use blam_tags::{TagFile, TagLayout, format_group_tag};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json;
use walkdir::WalkDir;

use crate::format::TagNameIndex;

#[derive(Clone, Copy, Debug, Default)]
/// Snapshot reported while a background loose-folder index scan is running.
/// Counts are advisory UI progress and never define the resulting entry set.
pub struct EntryIndexScanProgress {
    pub processed: usize,
    pub total: usize,
    pub matched: usize,
}

#[derive(Clone)]
/// A stable browser entry identifying one tag within a [`TagSource`].
///
/// `key` is the application identity used by tabs and caches; `display_path`
/// is user-facing and is not necessarily an absolute filesystem path.
/// Callers must preserve `key` across tree rebuilds so open documents remain
/// associated with the same on-disk or monolithic tag.
pub struct TagEntry {
    pub key: String,
    pub display_path: String,
    pub group_tag: u32,
    pub group_name: Option<String>,
    pub location: TagEntryLocation,
}

#[derive(Clone)]
/// Physical storage backing a [`TagEntry`].
/// The location is interpreted only together with the owning [`TagSource`]; a
/// loose path does not by itself carry the game definitions needed to parse it.
pub enum TagEntryLocation {
    LooseFile(PathBuf),
    Monolithic { name: String, group_tag: u32 },
}

#[derive(Clone)]
/// The source-aware context required to read tags correctly.
///
/// Classic loose tags must retain the `LooseFolder` game and definitions root;
/// they cannot be treated as self-describing MCC tag bytes.
/// Clones deliberately share monolithic cache storage while keeping loose-file
/// parsing context explicit.
pub enum TagSource {
    SingleFile {
        path: PathBuf,
    },
    LooseFolder {
        root: PathBuf,
        game: Option<String>,
        definitions_root: PathBuf,
    },
    MonolithicCache {
        root: PathBuf,
        cache: Arc<MonolithicCache>,
    },
}

impl TagSource {
    pub fn origin_label(&self) -> String {
        match self {
            TagSource::SingleFile { path } => format!("File: {}", path.display()),
            TagSource::LooseFolder { root, .. } => format!("Folder: {}", root.display()),
            TagSource::MonolithicCache { root, .. } => {
                format!("Monolithic cache: {}", root.display())
            }
        }
    }
}

/// Browser and index state associated with the currently loaded source.
///
/// `entries` is the lazily materialized browser subset, while `all_entries` is
/// the authoritative completed scan used by global filtering and group views.
/// Code must not interpret an empty `all_entries` as an empty loose folder while
/// a scan is pending.
pub struct LoadedSourceData {
    pub label: String,
    pub source: TagSource,
    pub names: TagNameIndex,
    /// Game identifier (e.g. "halo3_mcc"), used for the index cache filename.
    /// None for single-file and monolithic sources.
    pub game: Option<String>,
    /// Lazily-expanded entries for the folder tree (LooseFolder) or all
    /// entries for Monolithic / SingleFile sources.
    pub entries: Vec<TagEntry>,
    pub tree: TagTree,
    /// Built from `all_entries` once a background scan completes, or from
    /// `entries` for non-lazy sources (Monolithic / SingleFile).
    pub group_tree: TagTree,
    /// Full entry set from a completed background scan (or a loaded cache).
    /// Empty until populated. Groups mode and filtered search read from this.
    pub all_entries: Vec<TagEntry>,
    /// Reverse dependency cache for loose-folder sources. Built lazily by
    /// folder moves so future refactors can touch only dependent tags.
    pub reverse_dependencies: Option<ReverseDependencyIndex>,
    pub initial_tag: Option<(String, TagFile)>,
}

/// Result of reconciling a cached folder index with current files on disk.
/// `changed` describes index membership or fingerprints, not tag contents.
pub struct EntryIndexRefresh {
    pub entries: Vec<TagEntry>,
    pub changed: bool,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EntryFingerprint {
    size: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Clone, Debug, Default)]
/// Bidirectional lookup between tags and the tag references they contain.
/// Paths are normalized into stable dependency keys; both directions must be
/// updated together when a tag is replaced or removed.
pub struct ReverseDependencyIndex {
    by_tag: BTreeMap<String, Vec<DependencyRef>>,
    by_dependency: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// One normalized outbound tag reference recorded in the dependency index.
pub struct DependencyRef {
    pub group_tag: u32,
    pub rel_path: String,
}

impl ReverseDependencyIndex {
    pub fn set_tag_dependencies<I>(&mut self, tag_key: String, deps: I)
    where
        I: IntoIterator<Item = DependencyRef>,
    {
        self.clear_tag(&tag_key);
        let mut deps = deps.into_iter().collect::<Vec<_>>();
        deps.sort_by(|a, b| {
            dependency_key(a.group_tag, &a.rel_path).cmp(&dependency_key(b.group_tag, &b.rel_path))
        });
        deps.dedup_by(|a, b| {
            a.group_tag == b.group_tag && a.rel_path.eq_ignore_ascii_case(&b.rel_path)
        });
        for dep in &deps {
            let key = dependency_key(dep.group_tag, &dep.rel_path);
            let tags = self.by_dependency.entry(key).or_default();
            if !tags.iter().any(|existing| existing == &tag_key) {
                tags.push(tag_key.clone());
                tags.sort();
            }
        }
        self.by_tag.insert(tag_key, deps);
    }

    pub fn clear_tag(&mut self, tag_key: &str) {
        let Some(deps) = self.by_tag.remove(tag_key) else {
            return;
        };
        for dep in deps {
            let key = dependency_key(dep.group_tag, &dep.rel_path);
            if let Some(tags) = self.by_dependency.get_mut(&key) {
                tags.retain(|existing| existing != tag_key);
                if tags.is_empty() {
                    self.by_dependency.remove(&key);
                }
            }
        }
    }

    pub fn dependents_for(&self, group_tag: u32, rel_path: &str) -> &[String] {
        self.by_dependency
            .get(&dependency_key(group_tag, rel_path))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The dependencies a tag declares (what it references).
    pub fn dependencies_of(&self, tag_key: &str) -> &[DependencyRef] {
        self.by_tag.get(tag_key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn len(&self) -> usize {
        self.by_tag.len()
    }
}

#[derive(Debug)]
pub(crate) struct FolderRootInfo {
    pub(crate) scan_root: PathBuf,
    pub(crate) label: String,
    pub(crate) game: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EkFolderAlias {
    pub(crate) folder_name: String,
    pub(crate) game: String,
}

pub(crate) const SUPPORTED_EK_GAMES: &[(&str, &str)] = &[
    ("Halo CE", "haloce_mcc"),
    ("Halo 2", "halo2_mcc"),
    ("Halo 2 Anniversary Multiplayer", "halo2amp_mcc"),
    ("Halo 3", "halo3_mcc"),
    ("Halo 3 ODST", "halo3odst_mcc"),
    ("Halo Reach", "haloreach_mcc"),
    ("Halo 4", "halo4_mcc"),
];

#[derive(Default)]
/// Root of a browser tree whose entry indices address the owning entry vector.
/// Reusing a tree with a different entry vector is invalid even when paths look
/// identical, because nodes store positional indices rather than tag keys.
pub struct TagTree {
    pub children: Vec<TagTreeNode>,
    pub entries: Vec<usize>,
}

#[derive(Default)]
/// One lazily populated folder or group node in a [`TagTree`].
/// The `*_loaded` flags distinguish an unexpanded node from a known-empty node.
pub struct TagTreeNode {
    pub label: String,
    pub rel_path: PathBuf,
    pub children: Vec<TagTreeNode>,
    pub children_loaded: bool,
    pub entries: Vec<usize>,
    pub entries_loaded: bool,
}

#[derive(Default)]
struct TreeBuildNode {
    entries: Vec<usize>,
    children: BTreeMap<String, TreeBuildNode>,
}

/// Loads one self-describing tag file as an isolated source.
mod editing_kit;
mod index;
mod loading;
mod tree;

pub(crate) use editing_kit::*;
#[cfg(test)]
use editing_kit::{detect_ek_game, detect_ek_root_with_aliases};
#[cfg(test)]
use index::open_index_db;
pub(crate) use index::*;
pub use loading::*;
pub use tree::*;
use tree::{
    display_path_with_friendly_extension, display_str_with_friendly_extension, natural_key,
    path_to_display,
};
