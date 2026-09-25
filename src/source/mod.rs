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
use blam_tags::iostore::{IoStoreArchive, parse_ublock_stem};
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
    Monolithic {
        name: String,
        group_tag: u32,
    },
    /// A tag inside a mounted UE5 IoStore container (Halo: Campaign Evolved).
    /// `container` indexes the owning [`TagSource::IoStoreContainerSet`]'s
    /// `containers` (its provenance — which pak the tag came from, needed by the
    /// write/repack path). `rel_path` is the container-relative path of the
    /// `.ubulk` payload in its **original case** (the IoStore directory index is
    /// case-sensitive), whose bytes are a self-describing Reach MCC tag.
    Container {
        container: usize,
        rel_path: String,
    },
    /// A brand-new Campaign Evolved tag that does not exist in any mounted pak
    /// yet — it lives only in memory (the open [`TagDocument`]) until the user
    /// Saves (writes a new `_P` override container) or Exports a Mod. There is
    /// no backing `.ubulk` to read, so the document is authoritative.
    ///
    /// `template` says where the package wrapper comes from. `package` is the
    /// target UE package path `"/Game/Tags/<rel>-<group>"`.
    NewContainer {
        template: NewContainerTemplate,
        package: String,
        group_tag: u32,
    },
}

/// Where a brand-new Campaign Evolved tag's Unreal `.uasset` wrapper comes from.
///
/// Cloning a same-group tag was the only option for a long time, which meant a
/// group the game ships no tag of could not be authored at all — 26 of the 140
/// defined groups, `cinematic_scene` among them. The wrapper is derivable for
/// exactly those groups, so the source of it is a choice now rather than a
/// precondition.
#[derive(Clone, Debug)]
pub enum NewContainerTemplate {
    /// An existing same-group tag's `.uasset` in a mounted container, cloned
    /// and given the new tag's identity. `container` indexes the owning
    /// [`TagSource::IoStoreContainerSet`]'s `containers`.
    Donor { container: usize, rel_path: String },
    /// No tag of this group ships, so the wrapper is derived from the group's
    /// own rules by `blam_tags::iostore::asset::tag_package`. Only possible for
    /// a group whose class adds nothing over `BlamTagDataAssetBase`; the group
    /// long name is carried because deriving needs it and `group_tag` alone
    /// cannot produce it.
    Derived { group: String },
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
    /// One or more mounted UE5 IoStore containers (`.utoc`/`.ucas`) presented as
    /// a single read-only virtual filesystem of Reach tags. `root` is the `Paks`
    /// directory; `containers` are the individual mounted packs (base + level
    /// chunks), each shared behind an `Arc` so reads don't reopen or re-mmap.
    IoStoreContainerSet {
        root: PathBuf,
        containers: Vec<MountedContainer>,
        /// Tag-reference → container payload lookup, built at mount time so
        /// resolving a reference agrees with the browser tree by construction.
        index: Arc<ContainerTagIndex>,
        /// Cooked package-name lookup over the same containers, for following
        /// UE package imports (Campaign Evolved's audio binding).
        packages: Arc<ContainerPackageIndex>,
        /// Where the *game's own* copy of each mounted tag lives, ignoring any
        /// mod mounted over it. The mount layers mods last-wins, exactly as the
        /// game does, so without this the only reachable copy of a modded tag is
        /// the mod's — and every "what does this change about the game?"
        /// question answered itself with "nothing".
        shipped: Arc<ShippedTagIndex>,
    },
}

/// One mounted IoStore pack within a [`TagSource::IoStoreContainerSet`]. Carries
/// provenance (`utoc_path`, `chunk_label`) so the write/repack path knows
/// exactly which container/chunk a tag belongs to.
#[derive(Clone)]
pub struct MountedContainer {
    pub utoc_path: PathBuf,
    /// e.g. `pakchunk240-WinGDK` — the pak this tag was read from.
    pub chunk_label: String,
    /// Whether this is a mod rather than one of the game's own packs. Mods mount
    /// last and win every collision, so this is what separates "the game ships
    /// it this way" from "something installed here changed it".
    pub is_mod: bool,
    pub archive: Arc<IoStoreArchive>,
}

/// The game's own copy of each mounted tag payload: container-relative path
/// (lowercased) → the highest-priority **non-mod** container carrying it.
///
/// Keyed by path rather than by tag identity because a mod's entries are
/// recovered from the very containers it overrides, so both sides name the same
/// payload by the same path. A path absent here is a tag that only a mod
/// provides — there is nothing shipped to compare it against.
#[derive(Clone, Default)]
pub struct ShippedTagIndex {
    by_path: HashMap<String, usize>,
}

impl ShippedTagIndex {
    /// Record a payload carried by one of the game's own packs. Later inserts
    /// win, matching the mount loop's later-pack override.
    pub fn insert(&mut self, rel_path: &str, container: usize) {
        self.by_path
            .insert(rel_path.to_ascii_lowercase(), container);
    }

    pub fn container_for(&self, rel_path: &str) -> Option<usize> {
        self.by_path.get(&rel_path.to_ascii_lowercase()).copied()
    }

    /// Forget a payload that is no longer in any pack. The mount's later-pack
    /// layering is not reconstructed: a path that another pack still provides is
    /// re-learned on the next mount, and until then it reads as mod-only, which
    /// is the safe direction — it claims nothing about what the game ships.
    pub fn remove(&mut self, rel_path: &str) -> bool {
        self.by_path
            .remove(&rel_path.to_ascii_lowercase())
            .is_some()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }
}

/// Maps a tag reference (group + Halo-relative path) to the container payload
/// the mount resolved it to. Keyed identically to `build_container_set`'s
/// dedup key, so lookups agree with the browser tree by construction (and
/// inherit its later-pack-wins layering for free).
#[derive(Clone, Default)]
pub struct ContainerTagIndex {
    by_key: HashMap<String, (usize, String)>, // key -> (container index, original-case rel_path)
}

/// Normalize a tag reference into the key `build_container_set` dedups on:
/// strip NULs (references can carry them), trim, `\` → `/`, lowercase, and
/// prefix the group FOURCC. Mirrors the `logical` normalization at mount time.
pub fn container_ref_key(group_tag: u32, reference: &str) -> String {
    let normalized = reference
        .replace('\u{0}', "")
        .trim()
        .replace('\\', "/")
        .to_ascii_lowercase();
    format!("{group_tag:08x}:{normalized}")
}

impl ContainerTagIndex {
    /// Record a mounted tag payload, keyed by group + logical path. Later
    /// inserts win, matching the mount loop's later-pack override.
    pub fn insert(&mut self, key: String, container: usize, rel_path: String) {
        self.by_key.insert(key, (container, rel_path));
    }

    pub fn lookup(&self, group_tag: u32, reference: &str) -> Option<(usize, &str)> {
        let key = container_ref_key(group_tag, reference);
        if let Some((c, p)) = self.by_key.get(&key) {
            return Some((*c, p.as_str()));
        }
        // Campaign Evolved's cook moves a level's generated tags — the
        // scenario, its structure BSPs, their lighting info — into a
        // `_Generated_` folder, but the references baked into the tag data
        // still carry the pre-cook path: `c10.scenario` points its
        // `structure bsps[]` at `levels\halo1\solo\c10\level_a` while the
        // payload mounts as `levels/halo1/solo/c10/_generated_/level_a`.
        // Retry through that folder when the literal path misses.
        //
        // A fallback rather than an alias inserted at mount: the exact key
        // still wins, so a real tag sitting at the un-generated path is never
        // shadowed by its `_Generated_` neighbour.
        let (prefix, leaf) = key.rsplit_once('/')?;
        self.by_key
            .get(&format!("{prefix}/_generated_/{leaf}"))
            .map(|(c, p)| (*c, p.as_str()))
    }

    /// Forget a payload that no longer exists. Takes the already-normalized
    /// dedup key, as [`ContainerTagIndex::insert`] does, so an add and a remove
    /// address the same row.
    pub fn remove(&mut self, key: &str) -> bool {
        self.by_key.remove(key).is_some()
    }
}

/// Maps a cooked UE package name (`/Game/...`, lowercased) to the container
/// payload holding it.
///
/// [`ContainerTagIndex`] only covers `.ubulk` tag payloads, which is all the
/// tag editor needs. Campaign Evolved's audio, though, is reached by following
/// *package imports* out of a tag's `.uasset` wrapper — tag → `BlamAudioSound`
/// → `AkAudioEvent` — and those intermediates are ordinary cooked packages with
/// no tag payload at all. This is the sibling index that makes those hops
/// resolvable.
#[derive(Clone, Default)]
pub struct ContainerPackageIndex {
    /// Every container providing each package, in container (mount) order.
    /// The last is the one that is read, as it is for tags: a mod mounts after
    /// the game and overrides what it ships.
    by_package: HashMap<String, Vec<(usize, String)>>,
}

/// Cooked container path → UE package name, e.g.
/// `Meteorite/Content/Tags/sound/x-sound.uasset` → `/game/tags/sound/x-sound`.
/// Returns `None` for anything that isn't a `.uasset` under a `Content/` root.
pub fn container_package_name(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    let stem = lower.strip_suffix(".uasset")?;
    let rest = stem.split_once("/content/").map(|(_, r)| r)?;
    Some(format!("/game/{rest}"))
}

impl ContainerPackageIndex {
    /// Record that `container` provides a cooked package, replacing what that
    /// container provided before.
    ///
    /// The latest-mounted container wins, as it does for tags. This used to
    /// keep the first insert, which is the base game: a mod's copy of a
    /// package was ignored, so a modded tag read its `.ubulk` from the mod and
    /// its `.uasset` wrapper from the game.
    pub fn insert(&mut self, package: String, container: usize, rel_path: String) {
        let layers = self.by_package.entry(package).or_default();
        layers.retain(|(existing, _)| *existing != container);
        let at = layers.partition_point(|(existing, _)| *existing < container);
        layers.insert(at, (container, rel_path));
    }

    /// Resolve a `/Game/...` package name (any case) to its container payload.
    pub fn lookup(&self, package: &str) -> Option<(usize, &str)> {
        self.by_package
            .get(&package.to_ascii_lowercase())
            .and_then(|layers| layers.last())
            .map(|(c, p)| (*c, p.as_str()))
    }

    /// Forget that `container` provides a package. Another container that
    /// also provides it — the game under a mod's deleted override — is read
    /// from then on.
    pub fn remove(&mut self, package: &str, container: usize) -> bool {
        let key = package.to_ascii_lowercase();
        let Some(layers) = self.by_package.get_mut(&key) else {
            return false;
        };
        let before = layers.len();
        layers.retain(|(existing, _)| *existing != container);
        let removed = layers.len() != before;
        if layers.is_empty() {
            self.by_package.remove(&key);
        }
        removed
    }

    /// Number of indexed packages. Part of the type's surface and asserted on
    /// by the container integration tests; the app itself only ever looks up.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.by_package.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.by_package.is_empty()
    }
}

impl TagSource {
    pub fn origin_label(&self) -> String {
        match self {
            TagSource::SingleFile { path } => format!("File: {}", path.display()),
            TagSource::LooseFolder { root, .. } => format!("Folder: {}", root.display()),
            TagSource::MonolithicCache { root, .. } => {
                format!("Monolithic cache: {}", root.display())
            }
            TagSource::IoStoreContainerSet {
                root, containers, ..
            } => {
                format!("Containers ({}): {}", containers.len(), root.display())
            }
        }
    }

    /// The path this source was mounted from: a file for a single tag, a
    /// directory for everything else. Identifies the source across runs, which
    /// is what per-source sidecars key on.
    pub fn root_path(&self) -> &Path {
        match self {
            TagSource::SingleFile { path } => path,
            TagSource::LooseFolder { root, .. } => root,
            TagSource::MonolithicCache { root, .. } => root,
            TagSource::IoStoreContainerSet { root, .. } => root,
        }
    }

    /// Resolve a tag reference against mounted containers, returning the parsed
    /// tag. Errors for non-container sources or an unresolved reference. This is
    /// the same read the browser performs for a `TagEntryLocation::Container`.
    pub fn read_container_tag_by_ref(&self, group_tag: u32, reference: &str) -> Result<TagFile> {
        let TagSource::IoStoreContainerSet {
            containers, index, ..
        } = self
        else {
            anyhow::bail!("tag-reference resolution requires a container source");
        };
        let (container, rel_path) = index.lookup(group_tag, reference).ok_or_else(|| {
            anyhow!(
                "{}.{:08x} not found in mounted containers",
                reference.replace('\\', "/"),
                group_tag
            )
        })?;
        let mounted = containers
            .get(container)
            .context("container index out of range")?;
        let bytes = mounted
            .archive
            .read(rel_path)
            .map_err(|e| anyhow!("failed to read {rel_path} from container: {e}"))?;
        TagFile::read_from_bytes(&bytes).map_err(|e| anyhow!("failed to parse {rel_path}: {e}"))
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
    /// Where each looked-up key was last found. See [`Self::entry_for_key`].
    pub key_hints: EntryKeyHints,
    /// A full scan (or refresh) has installed `all_entries`. An empty
    /// `all_entries` alone cannot say this: it is also what an empty folder
    /// scans to, and reading it as "not scanned" made an empty folder rescan
    /// forever.
    pub complete_scan: bool,
}

#[cfg(test)]
thread_local! {
    /// Fallback scans this thread has made, for tests that bound them.
    static KEY_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A hint cache from tag key to its position in `entries` or `all_entries`.
///
/// Key lookups were a linear scan of both lists, made every frame (tab labels,
/// tag panes), per search hit, and per exported key, over tens of thousands of
/// entries. The lists are mutated in many places, including the browser's lazy
/// loader, so a map every mutation had to maintain would go stale the first
/// time one forgot. A hint is instead checked against the list before it is
/// trusted, and a miss or a stale hint falls back to the scan and records what
/// it found: a lookup is never wrong, and is only ever as slow as it used to be.
#[derive(Default)]
pub struct EntryKeyHints(std::sync::Mutex<HashMap<String, (EntryList, usize)>>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryList {
    /// `entries`, the browser's lazily loaded subset (every entry, for
    /// container and single-file sources).
    Lazy,
    /// `all_entries`, a loose folder's completed scan.
    All,
}

impl LoadedSourceData {
    /// Add `entry`, or replace the entry that has its key, and keep both lists,
    /// both trees and the on-disk entry index in step with it.
    ///
    /// This is the one way to put a single tag into a loaded source. There
    /// used to be about eight hand-written versions, and they disagreed: some
    /// inserted in `natural_key` order, some pushed and then re-sorted by a
    /// case-sensitive `display_path` comparison (which breaks the order
    /// `insert_entry_sorted` relies on), some did not sort at all, and most
    /// rewrote the whole entry index, a stat per tag, on the UI thread.
    ///
    /// For a loose folder, `entries` is the browser's lazy list and trees hold
    /// positions in it, so an entry is replaced where it is or appended, never
    /// inserted in the middle. Everywhere else `entries` is the full list and
    /// stays sorted.
    pub fn upsert_entry(&mut self, entry: TagEntry, pending_folders: &[String]) {
        let key = entry.key.clone();
        if let TagSource::LooseFolder { root, .. } = &self.source {
            match self.entries.iter_mut().find(|existing| existing.key == key) {
                Some(slot) => *slot = entry.clone(),
                None => self.entries.push(entry.clone()),
            }
            // An empty `all_entries` is a folder not scanned yet, not an empty
            // one: the scan will find this tag, and there is no index to add it
            // to (`upsert_entry_index_row` will not create one).
            if !self.all_entries.is_empty() {
                self.all_entries.retain(|existing| existing.key != key);
                insert_entry_sorted(&mut self.all_entries, entry.clone());
                if let Some(game) = self.game.as_deref() {
                    let _ = upsert_entry_index_row(game, root, &entry);
                }
            }
        } else {
            self.entries.retain(|existing| existing.key != key);
            insert_entry_sorted(&mut self.entries, entry.clone());
            if !self.all_entries.is_empty() {
                self.all_entries.retain(|existing| existing.key != key);
                insert_entry_sorted(&mut self.all_entries, entry);
            }
        }
        self.rebuild_trees(pending_folders);
    }

    /// Remove the entry with `key` from both lists, both trees and the on-disk
    /// entry index. Whether there was one.
    pub fn remove_entry(&mut self, key: &str, pending_folders: &[String]) -> bool {
        let before = self.entries.len() + self.all_entries.len();
        self.entries.retain(|entry| entry.key != key);
        self.all_entries.retain(|entry| entry.key != key);
        let removed = self.entries.len() + self.all_entries.len() != before;
        if let (TagSource::LooseFolder { root, .. }, Some(game)) = (&self.source, self.game.as_deref())
        {
            let _ = delete_entry_index_row(game, root, key);
        }
        self.rebuild_trees(pending_folders);
        removed
    }

    /// Rebuild the folder and group trees after a change to the lists. A loose
    /// folder's tree is re-read from disk (it is lazy, and positions in the
    /// lazy list may have moved); other sources rebuild theirs from `entries`.
    fn rebuild_trees(&mut self, pending_folders: &[String]) {
        if let TagSource::LooseFolder { root, .. } = &self.source {
            if let Ok(tree) = build_folder_directory_tree(root) {
                self.tree = tree;
            }
        } else {
            rebuild_folder_tree(self, pending_folders);
        }
        self.group_tree = build_group_tree(if self.all_entries.is_empty() {
            &self.entries
        } else {
            &self.all_entries
        });
    }

    /// The entry with `key`, from `entries` first and then `all_entries`.
    pub fn entry_for_key(&self, key: &str) -> Option<&TagEntry> {
        let list = |which: EntryList| match which {
            EntryList::Lazy => &self.entries,
            EntryList::All => &self.all_entries,
        };
        let hint = self
            .key_hints
            .0
            .lock()
            .ok()
            .and_then(|hints| hints.get(key).copied());
        if let Some((which, index)) = hint
            && let Some(entry) = list(which).get(index)
            && entry.key == key
        {
            return Some(entry);
        }
        #[cfg(test)]
        KEY_SCANS.with(|scans| scans.set(scans.get() + 1));
        let found = self
            .entries
            .iter()
            .position(|entry| entry.key == key)
            .map(|index| (EntryList::Lazy, index))
            .or_else(|| {
                self.all_entries
                    .iter()
                    .position(|entry| entry.key == key)
                    .map(|index| (EntryList::All, index))
            });
        if let Ok(mut hints) = self.key_hints.0.lock() {
            match found {
                Some(location) => {
                    hints.insert(key.to_owned(), location);
                }
                None => {
                    hints.remove(key);
                }
            }
        }
        found.map(|(which, index)| &list(which)[index])
    }

    /// The complete entry set, for callers that must see every tag rather than
    /// the browser's lazy subset. A loose folder fills `all_entries` from its
    /// background scan; a container mount enumerates every tag into `entries`
    /// up front and leaves `all_entries` empty. Only meaningful once the source
    /// is fully enumerated — a loose folder mid-scan returns the partial
    /// `entries`, so gate on the scan (or on an index built from it) first.
    pub fn full_entry_set(&self) -> &[TagEntry] {
        if self.all_entries.is_empty() {
            &self.entries
        } else {
            &self.all_entries
        }
    }
}

/// Result of reconciling a cached folder index with current files on disk.
/// `changed` describes index membership or fingerprints, not tag contents.
pub struct EntryIndexRefresh {
    pub entries: Vec<TagEntry>,
    pub changed: bool,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    /// Entries added or changed since the cached index: the only ones whose
    /// index rows and references need rewriting.
    pub touched: Vec<TagEntry>,
    /// Keys the cached index had that are gone, or no longer tags.
    pub removed_keys: Vec<String>,
    /// References of each touched tag, read by whoever applies the refresh.
    /// Empty from [`crate::source::refresh_entry_index`] itself.
    pub touched_dependencies: Vec<(String, Vec<DependencyRef>)>,
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
    ("Halo: Campaign Evolved", "haloce_evolved"),
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
    /// Seeded from the workspace's pending-folder set rather than derived from
    /// any entry's `display_path`. A pending node that is also childless and
    /// entryless is one nothing has landed in yet, and is the only kind the
    /// browser offers to delete.
    pub pending: bool,
}

#[derive(Default)]
struct TreeBuildNode {
    entries: Vec<usize>,
    children: BTreeMap<String, TreeBuildNode>,
    pending: bool,
}

/// Loads one self-describing tag file as an isolated source.
pub mod ce_audio;
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
    display_path_with_friendly_extension, display_str_with_friendly_extension, path_to_display,
};

#[cfg(test)]
mod container_ref_tests {
    use super::*;

    /// Campaign Evolved's cook puts a level's generated tags under
    /// `_Generated_`, but the references baked into the tag data still use the
    /// pre-cook path — `c10.scenario` points at `levels\halo1\solo\c10\level_a`
    /// while the payload mounts at `.../c10/_generated_/level_a`. Without the
    /// fallback, exporting a CE scenario's geometry resolved none of its \\Ps.
    #[test]
    fn container_lookup_falls_back_through_generated_folder() {
        let sbsp = u32::from_be_bytes(*b"sbsp");
        let mut index = ContainerTagIndex::default();
        index.insert(
            container_ref_key(sbsp, "levels/halo1/solo/c10/_generated_/level_a"),
            3,
            "Meteorite/Content/Tags/Levels/Halo1/Solo/C10/_Generated_/level_a-scenario_structure_bsp.ubulk"
                .to_owned(),
        );

        // The reference as the scenario stores it: backslashes, no `_Generated_`.
        let (container, rel) = index
            .lookup(sbsp, "levels\\halo1\\solo\\c10\\level_a")
            .expect("reference should resolve through the _Generated_ folder");
        assert_eq!(container, 3);
        assert!(rel.ends_with("level_a-scenario_structure_bsp.ubulk"));

        // The literal mounted path still resolves.
        assert!(
            index
                .lookup(sbsp, "levels\\halo1\\solo\\c10\\_generated_\\level_a")
                .is_some()
        );
        // Wrong group and unknown paths still miss.
        assert!(
            index
                .lookup(
                    u32::from_be_bytes(*b"scnr"),
                    "levels\\halo1\\solo\\c10\\level_a"
                )
                .is_none()
        );
        assert!(
            index
                .lookup(sbsp, "levels\\halo1\\solo\\c99\\nope")
                .is_none()
        );
    }

    /// An exact hit must never be shadowed by a `_Generated_` neighbour.
    #[test]
    fn exact_container_match_wins_over_generated_fallback() {
        let sbsp = u32::from_be_bytes(*b"sbsp");
        let mut index = ContainerTagIndex::default();
        index.insert(
            container_ref_key(sbsp, "levels/x/bsp"),
            1,
            "exact.ubulk".to_owned(),
        );
        index.insert(
            container_ref_key(sbsp, "levels/x/_generated_/bsp"),
            2,
            "generated.ubulk".to_owned(),
        );
        assert_eq!(
            index.lookup(sbsp, "levels\\x\\bsp"),
            Some((1, "exact.ubulk"))
        );
    }
}

#[cfg(test)]
mod entry_key_hint_tests {
    use super::*;

    fn entry(key: &str) -> TagEntry {
        TagEntry {
            key: key.to_owned(),
            display_path: key.to_owned(),
            group_tag: 0,
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(key)),
        }
    }

    fn source(entries: Vec<TagEntry>, all_entries: Vec<TagEntry>) -> LoadedSourceData {
        LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::SingleFile {
                path: PathBuf::from("a"),
            },
            names: TagNameIndex::default(),
            game: None,
            entries,
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries,
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
        }
    }

    fn found<'a>(source: &'a LoadedSourceData, key: &str) -> Option<&'a str> {
        source.entry_for_key(key).map(|entry| entry.display_path.as_str())
    }

    /// The lists are mutated in many places behind the hints' back. Whatever
    /// happens to them, a lookup answers exactly what a scan would.
    #[test]
    fn key_lookups_stay_right_as_the_lists_change_under_them() {
        let mut source = source(vec![entry("a"), entry("b")], vec![entry("c")]);
        assert_eq!(found(&source, "b"), Some("b"));
        assert_eq!(found(&source, "c"), Some("c"), "found in the full scan");
        assert_eq!(found(&source, "b"), Some("b"), "and again from the hint");

        // Inserting ahead of a remembered key moves it: the stale hint is
        // caught, not trusted.
        source.entries.insert(0, entry("z"));
        assert_eq!(found(&source, "b"), Some("b"));

        // The browser's lazy loader appends; a key nobody asked about before
        // is found by the fallback.
        source.entries.push(entry("d"));
        assert_eq!(found(&source, "d"), Some("d"));

        // A removed key is gone, even though its hint pointed at a real slot.
        source.entries.retain(|entry| entry.key != "b");
        assert_eq!(found(&source, "b"), None);
        source.all_entries.clear();
        assert_eq!(found(&source, "c"), None);
        assert_eq!(found(&source, "missing"), None);
    }

    /// A key found once is found again without scanning, which is the point.
    #[test]
    fn a_repeated_key_lookup_does_not_scan_again() {
        let entries: Vec<TagEntry> = (0..1000).map(|index| entry(&format!("k{index}"))).collect();
        let source = source(entries, Vec::new());
        assert_eq!(found(&source, "k999"), Some("k999"));
        let before = KEY_SCANS.with(std::cell::Cell::get);
        for _ in 0..100 {
            assert_eq!(found(&source, "k999"), Some("k999"));
        }
        assert_eq!(KEY_SCANS.with(std::cell::Cell::get), before);
    }

    /// Packages layer as tags do: the last-mounted container is read, and
    /// removing one container's copy leaves the others.
    #[test]
    fn a_mods_package_overrides_the_games_until_it_is_deleted() {
        const PACKAGE: &str = "/game/tags/sound/x-sound";
        let mut packages = ContainerPackageIndex::default();
        packages.insert(PACKAGE.to_owned(), 0, "Game/x-sound.uasset".to_owned());
        packages.insert(PACKAGE.to_owned(), 5, "Mod/x-sound.uasset".to_owned());
        assert_eq!(
            packages.lookup("/Game/Tags/Sound/X-Sound"),
            Some((5, "Mod/x-sound.uasset"))
        );

        // A rename inside the game's container rewrites its copy, beneath the
        // mod's.
        packages.insert(PACKAGE.to_owned(), 0, "Game/renamed.uasset".to_owned());
        assert_eq!(packages.lookup(PACKAGE), Some((5, "Mod/x-sound.uasset")));

        assert!(packages.remove(PACKAGE, 5), "the mod's copy is deleted");
        assert_eq!(packages.lookup(PACKAGE), Some((0, "Game/renamed.uasset")));
        assert!(!packages.remove(PACKAGE, 5));
        assert!(packages.remove(PACKAGE, 0));
        assert_eq!(packages.lookup(PACKAGE), None);
        assert!(packages.is_empty());
    }
}
