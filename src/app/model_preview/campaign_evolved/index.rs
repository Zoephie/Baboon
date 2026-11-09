//! Finding a character's meshes: the mesh-sync and path indexes over the game's packages.

use super::*;

/// Heuristic: is this a first-person hand/body model (`spartans_fp`,
/// `.../fp_body`)? Those are `FirstPersonHands`/`FirstPersonBody`
/// representations sourced outside the world mesh-sync path.
pub(super) fn is_first_person_model(model_key: &str) -> bool {
    let k = model_key.to_ascii_lowercase();
    k.contains("_fp/") || k.contains("/fp_") || k.contains("fp_body") || k.ends_with("_fp")
}

/// Find a character's UE asset root by locating the `*MeshSynchronization`
/// data asset that imports this model's package, returning a
/// `.../characters/<name>` path prefix.
pub(super) fn ce_find_character_root(containers: &[MountedContainer], model_key: &str) -> Option<String> {
    let index = ce_path_index(containers);
    for (norm, container, entry) in &index.mesh_sync {
        let c = &containers[*container];
        let e = &c.archive.entries()[*entry];
        let Ok(bytes) = c.archive.read(&e.path) else {
            continue;
        };
        let Ok(hdr) =
            FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None)
        else {
            continue;
        };
        let hit = hdr.imported_package_names.iter().any(|p| {
            p.to_ascii_lowercase()
                .replace('\\', "/")
                .ends_with(model_key)
        });
        if !hit {
            continue;
        }
        // Char folder = the dir holding this DA's `Common/` subfolder
        // (or the DA's own dir when it isn't under a `Common/`).
        return norm
            .rsplit_once("/common/")
            .map(|(root, _)| root.to_string())
            .or_else(|| norm.rsplit_once('/').map(|(root, _)| root.to_string()));
    }
    None
}

/// Native UClass object paths, identified by class (not filename) so resolution
/// is robust to the game's inconsistent asset-naming. A native class shows up in
/// an export's `class_index` as a `ScriptImport` whose value is the CityHash64
/// of the lowercased object path — so we compare against a precomputed
/// `FPackageObjectIndex` in O(1).
const CE_MESHSYNC_DA_CLASS: &str = "/Script/BlamSynchronization.BlamMeshSynchronizationDataAsset";

const CE_MESHSYNC_COMP_CLASS: &str = "/Script/BlamSynchronization.BlamMeshSynchronizationComponent";

const CE_MESHSYNC_COMP_BASE_CLASS: &str =
    "/Script/BlamSynchronization.BlamMeshSynchronizationComponentBase";

/// Precomputed class indices `(mesh-sync DA, component, component base)`.
pub(super) static CE_CLASSES: LazyLock<(
    FPackageObjectIndex,
    FPackageObjectIndex,
    FPackageObjectIndex,
)> = LazyLock::new(|| {
    (
        FPackageObjectIndex::create_script_import(CE_MESHSYNC_DA_CLASS),
        FPackageObjectIndex::create_script_import(CE_MESHSYNC_COMP_CLASS),
        FPackageObjectIndex::create_script_import(CE_MESHSYNC_COMP_BASE_CLASS),
    )
});

/// One actor Blueprint that owns a mesh-sync component (identified by class).
#[derive(Clone)]
pub(super) struct ActorRef {
    pub(super) container: usize,
    pub(super) path: String,
}

/// The mesh-sync binding graph for a mounted container set, built once and
/// cached: which actor Blueprints render which model. Built by a single
/// header-only pass (via [`IoStoreArchive::read_prefix`]) that identifies every
/// `BlamMeshSynchronizationDataAsset` and every actor with a
/// `BlamMeshSynchronizationComponent` **by class**, then joins them through the
/// import graph (actor → imports DA → DA's `ModelTag` is the model). No filename
/// heuristics: the class check catches `DA_*`, `BP_*`, and tokenless device
/// assets alike, and the import join is immune to codename aliases
/// (`tuning_fork`↔`Spirit`, `monitor`↔`GuiltySpark`, …).
pub(super) struct CeMeshSyncIndex {
    /// `(imported model package name, lowercased) → actor Blueprints that render
    /// it`. Matched against a previewed model by suffix, preserving the exact
    /// `ends_with` semantics the resolver has always used.
    pub(super) by_model: Vec<(String, Vec<ActorRef>)>,
}

/// Cache of the mesh-sync index, keyed by the mounted container set's identity
/// ([`ce_container_set_key`]). Building it scans every package header once
/// (~5s), so we keep it for the life of the mount rather than rebuilding per
/// preview.
static CE_INDEX_CACHE: LazyLock<Mutex<HashMap<String, Arc<CeMeshSyncIndex>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Every `.uasset` in a mounted container set, found by name without a scan.
pub(super) struct CePathIndex {
    /// Lowercased file name (`sk_foo.uasset`) → `(container, entry)`, in the
    /// order a scan of containers then entries meets them, so the first match
    /// is the one a linear scan would have found.
    pub(super) by_file_name: HashMap<String, Vec<(usize, usize)>>,
    /// The mesh-sync assets (lowercased, `/`-separated paths), in scan order.
    pub(super) mesh_sync: Vec<(String, usize, usize)>,
}

/// Cache of [`CePathIndex`], keyed like [`CE_INDEX_CACHE`].
static CE_PATH_INDEX_CACHE: LazyLock<Mutex<HashMap<String, Arc<CePathIndex>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// A container set's identity: its `.utoc` paths, with each one's size and
/// modified time, so a game update that rewrites them makes a new set rather
/// than reusing indices of the old.
pub(super) fn ce_container_set_key(containers: &[MountedContainer]) -> String {
    files_key(containers.iter().map(|c| c.utoc_path.as_path()))
}

/// Files by path, size and modified time.
pub(super) fn files_key<'a>(paths: impl Iterator<Item = &'a std::path::Path>) -> String {
    let stamp = |path: &std::path::Path| {
        let metadata = std::fs::metadata(path).ok();
        let modified = metadata.as_ref().and_then(|metadata| metadata.modified().ok());
        let since = modified.and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok());
        (metadata.map_or(0, |metadata| metadata.len()), since.map_or(0, |since| since.as_nanos()))
    };
    paths
        .map(|path| {
            let (len, modified) = stamp(path);
            format!("{}@{len}@{modified}", path.display())
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn build_ce_path_index(containers: &[MountedContainer]) -> CePathIndex {
    index_ce_paths(containers.iter().enumerate().flat_map(|(container, c)| {
        c.archive
            .entries()
            .iter()
            .enumerate()
            .map(move |(entry, e)| (container, entry, e.path.as_str()))
    }))
}

/// Index `(container, entry, path)` in the order given.
pub(super) fn index_ce_paths<'p>(paths: impl Iterator<Item = (usize, usize, &'p str)>) -> CePathIndex {
    let mut by_file_name: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    let mut mesh_sync = Vec::new();
    for (container, entry, path) in paths {
        let norm = path.to_ascii_lowercase().replace('\\', "/");
        if !norm.ends_with(".uasset") {
            continue;
        }
        let file_name = norm.rsplit('/').next().unwrap_or(&norm).to_owned();
        by_file_name
            .entry(file_name)
            .or_default()
            .push((container, entry));
        if norm.contains("meshsync") {
            mesh_sync.push((norm, container, entry));
        }
    }
    CePathIndex {
        by_file_name,
        mesh_sync,
    }
}

/// The cached path index for this container set (built on first use).
pub(super) fn ce_path_index(containers: &[MountedContainer]) -> Arc<CePathIndex> {
    let key = ce_container_set_key(containers);
    let mut cache = CE_PATH_INDEX_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry(key)
        .or_insert_with(|| Arc::new(build_ce_path_index(containers)))
        .clone()
}

/// Parse a package's Zen header cheaply — decode only the header prefix (name
/// map + import/export tables live at the front, before the bulky export data),
/// falling back to a full read for the rare header that exceeds the window.
pub(super) fn ce_read_header(archive: &IoStoreArchive, path: &str) -> Option<FZenPackageHeader> {
    if let Ok(bytes) = archive.read_prefix(path, 192 * 1024)
        && let Ok(hdr) = FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None)
    {
        return Some(hdr);
    }
    let bytes = archive.read(path).ok()?;
    FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None).ok()
}

/// The lowercased basename stem (no dir, no extension) of a UE path.
fn ce_stem(path: &str) -> String {
    let n = path.to_ascii_lowercase().replace('\\', "/");
    let base = n.rsplit('/').next().unwrap_or(&n);
    base.strip_suffix(".uasset").unwrap_or(base).to_string()
}

/// Build the mesh-sync binding index for a container set (see [`CeMeshSyncIndex`]).
fn build_ce_mesh_sync_index(containers: &[MountedContainer]) -> CeMeshSyncIndex {
    let (da_class, comp_class, comp_base_class) = *CE_CLASSES;

    // Single header-only pass over every `.uasset`.
    // `da_to_models`: DA stem → the `-model` package names it imports (its ModelTag).
    // `actors`: actor refs that own a mesh-sync component, with the DA stems they import.
    let mut da_to_models: HashMap<String, Vec<String>> = HashMap::new();
    let mut actors: Vec<(ActorRef, Vec<String>)> = Vec::new();
    for (ci, c) in containers.iter().enumerate() {
        for e in c.archive.entries() {
            if !e.path.to_ascii_lowercase().ends_with(".uasset") {
                continue;
            }
            let Some(hdr) = ce_read_header(&c.archive, &e.path) else {
                continue;
            };
            let is_da = hdr.exports_class(da_class);
            let has_comp = hdr.exports_class(comp_class) || hdr.exports_class(comp_base_class);
            if !is_da && !has_comp {
                continue;
            }
            if is_da {
                let models: Vec<String> = hdr
                    .imported_package_names
                    .iter()
                    .map(|p| p.to_ascii_lowercase().replace('\\', "/"))
                    .filter(|p| p.ends_with("-model"))
                    .collect();
                if !models.is_empty() {
                    da_to_models
                        .entry(ce_stem(&e.path))
                        .or_default()
                        .extend(models);
                }
            }
            if has_comp {
                let imported_stems: Vec<String> = hdr
                    .imported_package_names
                    .iter()
                    .map(|p| ce_stem(p))
                    .collect();
                actors.push((
                    ActorRef {
                        container: ci,
                        path: e.path.clone(),
                    },
                    imported_stems,
                ));
            }
        }
    }

    // Join: an actor renders model M if it imports a DA whose ModelTag is M.
    let mut model_to_actors: HashMap<String, Vec<ActorRef>> = HashMap::new();
    for (actor, imported_stems) in &actors {
        let mut models: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for stem in imported_stems {
            if let Some(m) = da_to_models.get(stem) {
                models.extend(m.iter().cloned());
            }
        }
        for m in models {
            let list = model_to_actors.entry(m).or_default();
            if !list
                .iter()
                .any(|a| a.container == actor.container && a.path == actor.path)
            {
                list.push(actor.clone());
            }
        }
    }

    let by_model: Vec<(String, Vec<ActorRef>)> = model_to_actors.into_iter().collect();
    CeMeshSyncIndex { by_model }
}

/// The cached mesh-sync index for this container set (built on first use).
pub(super) fn ce_mesh_sync_index(containers: &[MountedContainer]) -> Arc<CeMeshSyncIndex> {
    let key = ce_container_set_key(containers);
    // Held for the whole build, so a preview that asks while the prewarm is
    // running waits for it instead of scanning every header a second time. A
    // build that panicked poisons the lock; the map it guards is still sound.
    let mut cache = CE_INDEX_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(idx) = cache.get(&key) {
        return idx.clone();
    }
    let idx = Arc::new(build_ce_mesh_sync_index(containers));
    cache.insert(key, idx.clone());
    idx
}

/// Build a Campaign Evolved container set's mesh-sync index in the background
/// as soon as it is mounted.
///
/// Building it scans every package header in the install, about five
/// seconds. Built here, the first preview finds it ready (or waits on this
/// build rather than starting its own).
pub(in crate::app) fn prewarm_ce_mesh_sync_index(containers: Vec<MountedContainer>) {
    spawn_background("Campaign Evolved mesh index prewarm", move || {
        ce_path_index(&containers);
        ce_mesh_sync_index(&containers);
    });
}
