//! Tag source loading and source-aware tag reads.
//! It owns source identity, discovery, indexing, and source-aware reads; editor presentation and application workflow state belong elsewhere.

use super::*;

/// Loads one self-describing non-classic tag and seeds the document cache with it.
/// Classic tags intentionally require folder loading so their game layout is known.
pub fn load_single_file(path: PathBuf, names: &TagNameIndex) -> Result<LoadedSourceData> {
    let tag = read_non_classic_tag(&path)
        .with_context(|| format!("failed to load {}", path.display()))?;
    let group_tag = tag.group().tag;
    let group_name = names.name_for(group_tag).map(str::to_owned);
    let file_name = path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("loaded tag"));
    let display_path = display_path_with_friendly_extension(&file_name, group_tag, names);
    let key = format!("file:{}", path.display());
    let entry = TagEntry {
        key: key.clone(),
        display_path: display_path.clone(),
        group_tag,
        group_name,
        location: TagEntryLocation::LooseFile(path.clone()),
    };
    let entries = vec![entry];
    Ok(LoadedSourceData {
        label: display_path,
        source: TagSource::SingleFile { path },
        names: names.clone(),
        game: None,
        tree: build_tree(&entries),
        group_tree: build_group_tree(&entries),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        entries,
        initial_tag: Some((key, tag)),
    })
}

/// Resolves an editing-kit tags root and prepares lazy folder browsing.
/// A saved full index may populate `all_entries`, but `entries` remains lazy and
/// is filled only as browser folders are expanded.
pub fn load_folder(
    selected_root: PathBuf,
    fallback_names: &TagNameIndex,
    definitions_root: &Path,
    aliases: &[EkFolderAlias],
) -> Result<LoadedSourceData> {
    let info = resolve_folder_root(&selected_root, aliases)?;
    load_resolved_folder(
        info.scan_root,
        info.label,
        info.game.map(str::to_owned),
        fallback_names,
        definitions_root,
    )
}

/// Loads an explicitly typed editing-kit layout. Custom profile roots cannot be
/// identified reliably from their folder names, so the Settings-selected engine
/// is authoritative while the ordinary Open Folder path keeps auto-detection.
pub fn load_editing_kit_layout(
    tags_root: PathBuf,
    label: String,
    game: String,
    fallback_names: &TagNameIndex,
    definitions_root: &Path,
) -> Result<LoadedSourceData> {
    load_resolved_folder(
        tags_root,
        label,
        Some(game),
        fallback_names,
        definitions_root,
    )
}

fn load_resolved_folder(
    scan_root: PathBuf,
    label: String,
    game: Option<String>,
    fallback_names: &TagNameIndex,
    definitions_root: &Path,
) -> Result<LoadedSourceData> {
    let names = game
        .as_deref()
        .and_then(|g| TagNameIndex::load_game(definitions_root, g).ok())
        .unwrap_or_else(|| fallback_names.clone());
    let entries = Vec::new();
    let tree = build_folder_directory_tree(&scan_root)
        .with_context(|| format!("failed to list folders in {}", scan_root.display()))?;
    // Pre-load a saved index so Groups and search work immediately.
    let all_entries = game
        .as_deref()
        .and_then(|g| load_entry_index(g, &scan_root))
        .unwrap_or_default();
    let reverse_dependencies = game
        .as_deref()
        .and_then(|g| load_reverse_dependency_index(g, &scan_root));
    let group_tree = build_group_tree(&all_entries);
    Ok(LoadedSourceData {
        label,
        source: TagSource::LooseFolder {
            root: scan_root,
            game: game.clone(),
            definitions_root: definitions_root.to_path_buf(),
        },
        names,
        game,
        entries,
        tree,
        group_tree,
        all_entries,
        reverse_dependencies,
        initial_tag: None,
    })
}

/// Opens a monolithic cache and creates stable name/group-backed entries.
/// The returned cache is shared through [`TagSource::MonolithicCache`] so later
/// reads do not reopen or duplicate the cache.
pub fn load_monolithic_blob_index(
    blob_index: PathBuf,
    names: &TagNameIndex,
) -> Result<LoadedSourceData> {
    let root = normalize_blob_index_path(&blob_index)?;
    let cache = Arc::new(
        MonolithicCache::open(&root)
            .with_context(|| format!("failed to open monolithic cache {}", root.display()))?,
    );
    let mut entries = Vec::with_capacity(cache.len());
    for entry in cache.iter_tags() {
        if entry.name.is_empty() {
            continue;
        }
        let group_name = names.name_for(entry.group_tag).map(str::to_owned);
        let display_path = display_str_with_friendly_extension(
            &entry.name.replace('\\', "/"),
            entry.group_tag,
            names,
        );
        entries.push(TagEntry {
            key: format!("cache:{}:{}", format_group_tag(entry.group_tag), entry.name),
            display_path,
            group_tag: entry.group_tag,
            group_name,
            location: TagEntryLocation::Monolithic {
                name: entry.name.clone(),
                group_tag: entry.group_tag,
            },
        });
    }
    entries.sort_by(|a, b| natural_key(&a.display_path).cmp(&natural_key(&b.display_path)));
    let label = root
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| root.display().to_string());
    let tree = build_tree(&entries);
    let group_tree = build_group_tree(&entries);
    Ok(LoadedSourceData {
        label,
        source: TagSource::MonolithicCache { root, cache },
        names: names.clone(),
        game: None,
        all_entries: Vec::new(),
        entries,
        tree,
        group_tree,
        initial_tag: None,
        reverse_dependencies: None,
    })
}

const CAMPAIGN_EVOLVED_GAME: &str = "haloce_evolved";

/// Mounts every IoStore container in a `Paks` directory as one merged read-only
/// source of Reach tags (Halo: Campaign Evolved). Shared tags live in
/// `pakchunk0`; each level chunk carries that mission's scenario + BSPs, so all
/// packs must be mounted to see the whole tag tree.
pub fn load_iostore_container_set(
    paks_dir: PathBuf,
    fallback_names: &TagNameIndex,
    definitions_root: &Path,
) -> Result<LoadedSourceData> {
    if !paks_dir.is_dir() {
        anyhow::bail!("failed to read {}", paks_dir.display());
    }
    let mut utocs = utocs_under(&paks_dir);
    // Mount base chunk first, then level chunks by number, so higher/patch
    // chunks win on any collision (mirrors UE's FIoDispatcher last-wins). A mod
    // is not named `pakchunkN`, so it sorts last and overrides what it patches.
    utocs.sort_by_key(|p| (chunk_number(p), p.clone()));
    build_container_set(paks_dir, utocs, fallback_names, definitions_root)
}

/// Mounts a single IoStore container (`.utoc`) — the "open one chunk" path. The
/// resulting source is still a set (of one).
///
/// `pak_root` is the install's `Paks` directory when the caller knows it (it is
/// configured in Settings), and it is what the container is mounted against. A
/// mod ships no directory index, so naming its chunks means reading the base
/// containers it overrides — and those are up in `Paks` when the mod itself is
/// installed in `Paks/~mods`. Rooting at the folder holding the `.utoc` left the
/// mod naming its own chunks off their package headers, under paths that
/// disagree with the base game's; that is the fallback when no root is known.
pub fn load_iostore_container(
    utoc: PathBuf,
    pak_root: Option<PathBuf>,
    fallback_names: &TagNameIndex,
    definitions_root: &Path,
) -> Result<LoadedSourceData> {
    let root = pak_root.unwrap_or_else(|| {
        utoc.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| utoc.clone())
    });
    build_container_set(root, vec![utoc], fallback_names, definitions_root)
}

/// Every mountable `.utoc` at or beneath `dir`.
///
/// Recursive because UE's own discovery is: `FPakPlatformFile::FindPakFilesInDirectory`
/// walks the pak folder with `IterateDirectoryRecursively`, which is what makes
/// the `~mods` convention work in game. A flat scan mounted the base game and
/// silently ignored every mod installed one directory down — the mod's tags were
/// simply absent from the browser.
fn utocs_under(dir: &Path) -> Vec<PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("utoc"))
        })
        // `global.utoc` has no directory index; it would fail to open anyway.
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("global.utoc"))
        })
        .filter(|p| !is_container_backup(p))
        .collect()
}

/// Whether a path is one of Baboon's own transactional artefacts rather than a
/// container anyone should mount or ship.
///
/// A duplicate writes an immutable `<name>.utoc.baboon-duplicate-backup[-N]`
/// beside the container before it mutates it, and an export builds its
/// replacement in a `.baboon-export-…` folder. Neither ends in `.utoc`, so
/// nothing mounts them today — but that is a consequence of how they happen to
/// be named, and it is exactly the kind of thing a later rename would undo
/// silently. Stated here, and asserted in the tests.
pub fn is_container_backup(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.contains(".baboon-duplicate-backup") || name.ends_with(".previous") {
        return true;
    }
    path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .starts_with(".baboon-export-")
    })
}

/// Other `.utoc`s in the pak tree at `root` that aren't already mounted, opened
/// purely so an index-less container's chunk ids can be resolved back to paths.
/// Only ones that carry a directory index are any use as a reference.
fn sibling_reference_archives(root: &Path, mounted: &[PathBuf]) -> Vec<IoStoreArchive> {
    utocs_under(root)
        .into_iter()
        .filter(|p| !mounted.contains(p))
        .filter_map(|p| IoStoreArchive::open(&p).ok())
        .filter(|archive| !archive.entries().is_empty())
        .collect()
}

/// Reopen one mounted container from disk after it was written to.
///
/// Not just `IoStoreArchive::open`: an override/mod container ships no
/// directory index, so a plain reopen comes back knowing none of its paths and
/// every later read or save of a tag in it fails. Rebuild the file list exactly
/// as the mount did — from the other mounted containers plus the siblings in
/// `root` — so the recovered paths agree with the `rel_path`s already recorded
/// in the tag entries.
pub fn reopen_container_archive(
    root: &Path,
    containers: &[MountedContainer],
    index: usize,
) -> Result<IoStoreArchive> {
    let target = containers
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("container provenance is stale"))?;
    let mut archive = IoStoreArchive::open(&target.utoc_path)?;
    if archive.entries().is_empty() {
        let mounted: Vec<PathBuf> = containers.iter().map(|c| c.utoc_path.clone()).collect();
        let references = sibling_reference_archives(root, &mounted);
        let bases: Vec<&IoStoreArchive> = containers
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != index)
            .map(|(_, c)| c.archive.as_ref())
            .chain(references.iter())
            .collect();
        archive.recover_entries(&bases, None);
    }
    Ok(archive)
}

/// Mount one more container into an already-loaded container set.
///
/// The alternative — reloading the whole source — rebuilds the workspace from
/// scratch, which costs every open tab, every unsaved document and the Mod
/// Stash. A mod that was just exported into the game's own `Paks` is one file
/// and a handful of tags, so it is folded into the mount that is already there.
///
/// The layering rules are the mount's: an override container ships no directory
/// index, so its paths are recovered against the containers already mounted;
/// it is a mod, so it wins collisions and contributes nothing to the shipped
/// index; and its entries land in sorted position rather than at the end, which
/// is the order the browser draws.
///
/// Returns the number of tags it contributed. Mounting a container that carries
/// none is not an error — it just is not kept.
pub fn mount_additional_container(
    source: &mut LoadedSourceData,
    utoc: &Path,
    pending_folders: &[String],
) -> Result<usize> {
    let names = source.names.clone();
    let (root, mounted_paths, container_index) = {
        let TagSource::IoStoreContainerSet {
            root, containers, ..
        } = &source.source
        else {
            anyhow::bail!("not a Campaign Evolved container source");
        };
        if containers
            .iter()
            .any(|mounted| paths_are_same_file(&mounted.utoc_path, utoc))
        {
            return Ok(0);
        }
        (
            root.clone(),
            containers
                .iter()
                .map(|mounted| mounted.utoc_path.clone())
                .collect::<Vec<_>>(),
            containers.len(),
        )
    };
    let mut archive = IoStoreArchive::open(utoc)?;
    // Decided before recovery fills in the missing names, exactly as the mount
    // does: shipping no directory index at all is the strongest signal there
    // is, and `recover_entries` erases it.
    let is_mod = archive.entries().is_empty() || !is_shipped_container_path(&root, utoc);
    if archive.entries().is_empty() {
        let TagSource::IoStoreContainerSet { containers, .. } = &source.source else {
            anyhow::bail!("not a Campaign Evolved container source");
        };
        let mut mounted = mounted_paths;
        mounted.push(utoc.to_path_buf());
        let references = sibling_reference_archives(&root, &mounted);
        let bases: Vec<&IoStoreArchive> = containers
            .iter()
            .map(|c| c.archive.as_ref())
            .chain(references.iter())
            .collect();
        archive.recover_entries(&bases, None);
    }
    let chunk_label = utoc
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("container")
        .to_string();
    let mut fresh: Vec<TagEntry> = Vec::new();
    let mut pending_packages: Vec<(String, String)> = Vec::new();
    let mut pending_index: Vec<(String, String)> = Vec::new();
    for entry in archive.entries() {
        if let Some(package) = container_package_name(&entry.path) {
            pending_packages.push((package, entry.path.clone()));
        }
    }
    for e in archive.ublock_entries() {
        let Some((tag_name, group_longname)) = parse_ublock_stem(&e.path) else {
            continue;
        };
        let Some(group_tag) = names.group_tag_for(group_longname) else {
            continue;
        };
        let after = strip_tags_root(&e.path);
        let dir = after.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let logical = if dir.is_empty() {
            tag_name.to_ascii_lowercase()
        } else {
            format!(
                "{}/{}",
                dir.to_ascii_lowercase(),
                tag_name.to_ascii_lowercase()
            )
        };
        let display_path = display_str_with_friendly_extension(&logical, group_tag, &names);
        pending_index.push((format!("{group_tag:08x}:{logical}"), e.path.clone()));
        fresh.push(TagEntry {
            key: format!("ublock:{chunk_label}:{}", e.path),
            display_path,
            group_tag,
            group_name: names.name_for(group_tag).map(str::to_owned),
            location: TagEntryLocation::Container {
                container: container_index,
                rel_path: e.path.clone(),
            },
        });
    }
    if fresh.is_empty() {
        return Ok(0);
    }
    {
        let TagSource::IoStoreContainerSet {
            containers,
            index,
            packages,
            ..
        } = &mut source.source
        else {
            anyhow::bail!("not a Campaign Evolved container source");
        };
        for (key, rel_path) in pending_index {
            Arc::make_mut(index).insert(key, container_index, rel_path);
        }
        for (package, rel_path) in pending_packages {
            Arc::make_mut(packages).insert(package, container_index, rel_path);
        }
        // Not added to the shipped index: a mod's copy of a tag is never what
        // the game ships, and that distinction is what "what does this mod
        // change?" is answered from.
        containers.push(MountedContainer {
            utoc_path: utoc.to_path_buf(),
            chunk_label,
            is_mod,
            archive: Arc::new(archive),
        });
    }
    let contributed = fresh.len();
    for entry in fresh {
        // Last-wins, exactly as the mount layers packs: a mod taking over a tag
        // replaces the entry rather than adding a second one beside it.
        //
        // The superseded entry's **key is kept**. A key is this application's
        // identity for a tag — open tabs, parsed documents, the undo journal
        // and every cache are filed under it — and a mod's key differs from the
        // pak's only in the container label it happens to be carried by. Minting
        // a new one here left every tab of an exported tag pointing at an entry
        // that no longer existed, showing its raw key and "This tag is no longer
        // in the source", with the user's edits stranded behind it. What changed
        // is where the tag is read from, which is `location`, not what it is.
        layer_entry(&mut source.entries, &entry);
        // The CE mount leaves `all_entries` empty and works off `entries`; an
        // empty one stays empty rather than becoming a second, partial list.
        if !source.all_entries.is_empty() {
            layer_entry(&mut source.all_entries, &entry);
        }
    }
    crate::source::rebuild_folder_tree(source, pending_folders);
    source.group_tree = build_group_tree(if source.all_entries.is_empty() {
        &source.entries
    } else {
        &source.all_entries
    });
    Ok(contributed)
}

/// Lay one newly mounted entry over an existing list, last-wins.
///
/// A superseded entry keeps its own `key`. The key is this application's
/// identity for a tag — open tabs, parsed documents, the undo journal and every
/// per-tag cache are filed under it — and it differs between a pak's copy and a
/// mod's only in the container label baked into it. What a new layer changes is
/// where the tag is *read from*, which is `location`, not what it is.
fn layer_entry(entries: &mut Vec<TagEntry>, entry: &TagEntry) {
    let superseded = entries.iter().position(|existing| {
        existing.group_tag == entry.group_tag && existing.display_path == entry.display_path
    });
    match superseded {
        Some(position) => {
            entries[position].location = entry.location.clone();
            entries[position].group_name = entry.group_name.clone();
        }
        None => {
            insert_entry_sorted(entries, entry.clone());
        }
    }
}

fn build_container_set(
    root: PathBuf,
    utocs: Vec<PathBuf>,
    fallback_names: &TagNameIndex,
    definitions_root: &Path,
) -> Result<LoadedSourceData> {
    let names = TagNameIndex::load_game(definitions_root, CAMPAIGN_EVOLVED_GAME)
        .unwrap_or_else(|_| fallback_names.clone());

    let mut containers: Vec<MountedContainer> = Vec::new();
    let mut entries: Vec<TagEntry> = Vec::new();
    // Dedup/layer by lowercase logical key; later packs (higher chunk) win.
    let mut seen: HashMap<String, usize> = HashMap::new();
    // Same key → the payload the mount resolved it to, for reference lookups.
    let mut index = ContainerTagIndex::default();
    // Cooked package names over the same containers, for following UE imports.
    let mut packages = ContainerPackageIndex::default();
    // The same payloads as the game's own packs have them, mods excluded.
    let mut shipped = ShippedTagIndex::default();
    let mut opened_any = false;

    // Open every container up front. A mod/override container addresses its
    // chunks by id and so ships with no directory index at all; naming its
    // contents needs the containers it overrides as reference, which may not
    // have been opened yet at its turn in the list.
    let mut opened: Vec<(PathBuf, Option<IoStoreArchive>)> = Vec::new();
    for utoc in utocs {
        // Skip containers we can't parse (e.g. index-less globals).
        if let Ok(archive) = IoStoreArchive::open(&utoc) {
            opened.push((utoc, Some(archive)));
        }
    }
    let needs_recovery = |slot: &Option<IoStoreArchive>| matches!(slot, Some(archive) if archive.entries().is_empty());
    // Which of these are mods, decided before recovery fills in the missing
    // names: shipping no directory index at all is the strongest signal there
    // is, and it is gone the moment `recover_entries` runs.
    let is_mod: Vec<bool> = opened
        .iter()
        .map(|(utoc, archive)| needs_recovery(archive) || !is_shipped_container_path(&root, utoc))
        .collect();
    // Naming an index-less container's chunks means resolving their ids against
    // the containers it overrides. On the "open one chunk" path — a mod sitting
    // in the game's own Paks folder — those aren't in the set, so pull the
    // siblings in as references. They name chunks and contribute no tags.
    let references: Vec<IoStoreArchive> = if opened.iter().any(|(_, a)| needs_recovery(a)) {
        let mounted: Vec<PathBuf> = opened.iter().map(|(p, _)| p.clone()).collect();
        sibling_reference_archives(&root, &mounted)
    } else {
        Vec::new()
    };
    for i in 0..opened.len() {
        if !needs_recovery(&opened[i].1) {
            continue;
        }
        // Lift the target out so the rest can be borrowed as its references.
        let Some(mut archive) = opened[i].1.take() else {
            continue;
        };
        let bases: Vec<&IoStoreArchive> = opened
            .iter()
            .filter_map(|(_, a)| a.as_ref())
            .chain(references.iter())
            .collect();
        archive.recover_entries(&bases, None);
        opened[i].1 = Some(archive);
    }

    for (position, (utoc, archive)) in opened.into_iter().enumerate() {
        let Some(archive) = archive else { continue };
        let is_mod = is_mod.get(position).copied().unwrap_or(false);
        opened_any = true;
        let chunk_label = utoc
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("container")
            .to_string();
        let container_index = containers.len();
        let archive = Arc::new(archive);
        let mut contributed = false;
        // Packages are only committed once we know the container is kept —
        // `container_index` is provisional until then, because packs that
        // contribute no tags are dropped and the next pack reuses the slot.
        let mut pending_packages: Vec<(String, String)> = Vec::new();
        for e in archive.entries() {
            if let Some(pkg) = container_package_name(&e.path) {
                pending_packages.push((pkg, e.path.clone()));
            }
        }

        for e in archive.ublock_entries() {
            let Some((tag_name, group_longname)) = parse_ublock_stem(&e.path) else {
                continue;
            };
            // A known group long-name yields the FOURCC and also filters out
            // non-tag `.ubulk` bulk data whose fake "group" isn't a real group.
            let Some(group_tag) = names.group_tag_for(group_longname) else {
                continue;
            };

            // Strip the `Tags/` root (that folder IS the Halo tags root, so the
            // remainder is tag-reference-relative), then lowercase folders/name
            // for consistency with lowercase tag references. `rel_path` keeps
            // ORIGINAL case for the case-sensitive container read.
            let after = strip_tags_root(&e.path);
            let dir = after.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            let logical = if dir.is_empty() {
                tag_name.to_ascii_lowercase()
            } else {
                format!(
                    "{}/{}",
                    dir.to_ascii_lowercase(),
                    tag_name.to_ascii_lowercase()
                )
            };
            let display_path = display_str_with_friendly_extension(&logical, group_tag, &names);

            let entry = TagEntry {
                key: format!("ublock:{chunk_label}:{}", e.path),
                display_path,
                group_tag,
                group_name: names.name_for(group_tag).map(str::to_owned),
                location: TagEntryLocation::Container {
                    container: container_index,
                    rel_path: e.path.clone(),
                },
            };
            contributed = true;

            let dedup_key = format!("{group_tag:08x}:{logical}");
            // Later packs win, matching the `entries[existing] = entry` override.
            index.insert(dedup_key.clone(), container_index, e.path.clone());
            // The shipped layer records only the game's own packs, so a mod
            // taking over a tag leaves the base copy reachable.
            if !is_mod {
                shipped.insert(&e.path, container_index);
            }
            match seen.get(&dedup_key) {
                Some(&existing) => {
                    // Overlap should be near-zero; note it rather than hide it.
                    eprintln!(
                        "container tag collision on {}: {} overrides earlier pack",
                        entry.display_path, chunk_label
                    );
                    entries[existing] = entry;
                }
                None => {
                    seen.insert(dedup_key, entries.len());
                    entries.push(entry);
                }
            }
        }

        // Only keep packs that actually contributed tags (drops empty stubs).
        if contributed {
            for (pkg, rel) in pending_packages {
                packages.insert(pkg, container_index, rel);
            }
            containers.push(MountedContainer {
                utoc_path: utoc,
                chunk_label,
                is_mod,
                archive,
            });
        }
    }

    if !opened_any {
        anyhow::bail!("no readable IoStore containers found in {}", root.display());
    }

    entries.sort_by(|a, b| natural_key(&a.display_path).cmp(&natural_key(&b.display_path)));
    let mods = containers.iter().filter(|c| c.is_mod).count();
    let label = format!(
        "{} ({} packs{})",
        root.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Campaign Evolved"),
        containers.len(),
        match mods {
            0 => String::new(),
            1 => ", 1 mod".to_owned(),
            n => format!(", {n} mods"),
        }
    );
    let tree = build_tree(&entries);
    let group_tree = build_group_tree(&entries);
    Ok(LoadedSourceData {
        label,
        source: TagSource::IoStoreContainerSet {
            root,
            containers,
            index: Arc::new(index),
            packages: Arc::new(packages),
            shipped: Arc::new(shipped),
        },
        names,
        game: Some(CAMPAIGN_EVOLVED_GAME.to_string()),
        all_entries: Vec::new(),
        entries,
        tree,
        group_tree,
        initial_tag: None,
        reverse_dependencies: None,
    })
}

/// Locate a UE5 `Paks` directory at or beneath `root` (any folder containing a
/// `.utoc`), so "Load Folder" can auto-detect a Campaign Evolved install. Checks
/// the folder itself, the common UE layout, then a shallow walk.
/// Locate the container directory for a Campaign Evolved install, given the
/// folder the user picked.
///
/// The canonical install layouts are tried first, and only then whether the
/// picked folder is itself full of containers. That order matters: an exported
/// mod leaves a `.utoc` in the game root, and checking the root first would
/// mount that one stray file and report an install with no tags in it. A
/// directory named `Paks` holding containers is a far stronger signal than a
/// loose `.utoc` beside the executable.
///
/// Picking the `Paks` directory itself still works — the recursive pass
/// includes the starting directory.
pub fn find_paks_dir(root: &Path) -> Option<PathBuf> {
    for candidate in [
        root.join("Meteorite").join("Content").join("Paks"),
        root.join("Content").join("Paks"),
    ] {
        if dir_has_utoc(&candidate) {
            return Some(candidate);
        }
    }
    for entry in WalkDir::new(root)
        .max_depth(4)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_dir()
            && entry.file_name().eq_ignore_ascii_case("Paks")
            && dir_has_utoc(entry.path())
        {
            return Some(entry.path().to_path_buf());
        }
    }
    // Last resort: a directory of containers that is not named `Paks`.
    dir_has_utoc(root).then(|| root.to_path_buf())
}

fn dir_has_utoc(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .any(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("utoc"))
        })
}

/// Strip the leading `Tags/` root (optionally under `Meteorite/Content/`),
/// case-insensitively. The remainder is relative to the Halo tags root.
fn strip_tags_root(path: &str) -> &str {
    for prefix in ["Meteorite/Content/Tags/", "Tags/"] {
        if path.len() >= prefix.len() && path[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return &path[prefix.len()..];
        }
    }
    let mc = "Meteorite/Content/";
    if path.len() >= mc.len() && path[..mc.len()].eq_ignore_ascii_case(mc) {
        return &path[mc.len()..];
    }
    path
}

/// Parse the chunk id from a `pakchunk<N>-...utoc` filename (u32::MAX if none),
/// so `pakchunk0` sorts first as the base.
/// Whether `utoc` looks like one of the game's own containers rather than a mod
/// installed into the same tree.
///
/// Two independent signals, both of which a mod fails: the game's packs are named
/// `pakchunk<N>[-platform]`, and they sit directly in `Paks`. Mods are named
/// whatever their author chose — and the convention the game itself relies on is
/// to drop them in a subfolder like `~mods`, which is why they are found at all.
/// A third and stronger signal (shipping no directory index) is checked at the
/// mount, where it is still observable.
fn is_shipped_container_path(paks_root: &Path, utoc: &Path) -> bool {
    chunk_number(utoc) != u32::MAX
        && utoc
            .parent()
            .is_some_and(|parent| paths_are_same_dir(parent, paks_root))
}

/// Directory comparison that survives the two forms the same folder arrives in:
/// the walked path under a mounted root, and the root the caller configured.
fn paths_are_same_dir(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Whether two paths name the same file, including when it does not exist yet —
/// which is the case that matters for an export about to create one.
pub fn paths_are_same_file(left: &Path, right: &Path) -> bool {
    // Names first, because they decide it without touching the disk: this runs
    // once per mounted container on every frame the export review is open, and
    // an install can mount ninety of them.
    match (left.file_name(), right.file_name()) {
        (Some(left_name), Some(right_name)) if left_name.eq_ignore_ascii_case(right_name) => {}
        _ => return false,
    }
    if left == right {
        return true;
    }
    if let (Ok(left), Ok(right)) = (left.canonicalize(), right.canonicalize()) {
        return left == right;
    }
    // The target does not exist yet, so compare the directories instead. Names
    // are compared case-insensitively throughout: Windows, the only platform
    // where writing over a mapped container fails at all, folds case, and
    // erring towards "same file" refuses an export rather than breaking one.
    match (left.parent(), right.parent()) {
        (Some(left_dir), Some(right_dir)) => paths_are_same_dir(left_dir, right_dir),
        _ => false,
    }
}

/// Read a container tag as the **game's own packs** have it, ignoring any mod
/// mounted over it.
///
/// `Ok(None)` for a tag no shipped pack carries: a mod added it, so there is
/// nothing to compare against rather than nothing changed. Every "what does this
/// change about the game?" answer has to come from here, because [`read_entry`]
/// deliberately reads what the *game* would load, mods included.
/// The shipped payload bytes for `entry`, without parsing them.
///
/// Parsing a 7 MB scenario to answer "is this byte-identical to what ships?" is
/// most of the cost of the question. Used by the export review, which asks it
/// for every stashed tag.
pub fn read_shipped_entry_bytes(source: &TagSource, entry: &TagEntry) -> Result<Option<Vec<u8>>> {
    let (
        TagEntryLocation::Container { rel_path, .. },
        TagSource::IoStoreContainerSet {
            containers,
            shipped,
            ..
        },
    ) = (&entry.location, source)
    else {
        return Ok(None);
    };
    let Some(index) = shipped.container_for(rel_path) else {
        return Ok(None);
    };
    let mounted = containers
        .get(index)
        .context("shipped container index out of range")?;
    mounted.archive.read(rel_path).map(Some).map_err(|e| {
        anyhow!(
            "failed to read {rel_path} from {}: {e}",
            mounted.chunk_label
        )
    })
}

pub fn read_shipped_entry(source: &TagSource, entry: &TagEntry) -> Result<Option<TagFile>> {
    let (
        TagEntryLocation::Container { rel_path, .. },
        TagSource::IoStoreContainerSet {
            containers,
            shipped,
            ..
        },
    ) = (&entry.location, source)
    else {
        return Ok(None);
    };
    let Some(index) = shipped.container_for(rel_path) else {
        return Ok(None);
    };
    let mounted = containers
        .get(index)
        .context("shipped container index out of range")?;
    let bytes = mounted.archive.read(rel_path).map_err(|e| {
        anyhow!(
            "failed to read {rel_path} from {}: {e}",
            mounted.chunk_label
        )
    })?;
    TagFile::read_from_bytes(&bytes)
        .map(Some)
        .map_err(|e| anyhow!("failed to parse {}: {e}", entry.display_path))
}

/// The mounted containers an export to `out_utoc` would overwrite.
///
/// Mounting maps a container's `.ucas` into memory, and the export replaces that
/// file wholesale; on Windows, truncating a mapped file fails outright
/// (`ERROR_USER_MAPPED_FILE`, os error 1224). Since mods installed under `Paks`
/// are mounted, re-exporting a mod over itself is exactly that collision.
pub fn mounted_containers_at(source: &TagSource, out_utoc: &Path) -> Vec<usize> {
    let TagSource::IoStoreContainerSet { containers, .. } = source else {
        return Vec::new();
    };
    containers
        .iter()
        .enumerate()
        .filter(|(_, container)| paths_are_same_file(&container.utoc_path, out_utoc))
        .map(|(index, _)| index)
        .collect()
}

fn chunk_number(utoc: &Path) -> u32 {
    utoc.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("pakchunk"))
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(u32::MAX)
}

/// Reads an entry using the storage and parsing rules of its owning source.
/// Mismatched source/location pairs are rejected rather than guessed.
pub fn read_entry(source: &TagSource, entry: &TagEntry) -> Result<TagFile> {
    match (&entry.location, source) {
        (
            TagEntryLocation::LooseFile(path),
            TagSource::LooseFolder {
                game,
                definitions_root,
                ..
            },
        ) => read_loose_tag(path, entry, game.as_deref(), definitions_root)
            .with_context(|| format!("failed to load {}", path.display())),
        (TagEntryLocation::LooseFile(path), _) => {
            read_non_classic_tag(path).with_context(|| format!("failed to load {}", path.display()))
        }
        (
            TagEntryLocation::Monolithic { name, group_tag },
            TagSource::MonolithicCache { cache, .. },
        ) => cache.read_tag_by_name(*group_tag, name).with_context(|| {
            format!(
                "failed to load {} from monolithic cache",
                entry.display_path
            )
        }),
        (TagEntryLocation::Monolithic { .. }, _) => {
            anyhow::bail!("monolithic entry selected outside a monolithic source")
        }
        (
            TagEntryLocation::Container {
                container,
                rel_path,
            },
            TagSource::IoStoreContainerSet { containers, .. },
        ) => {
            let mounted = containers
                .get(*container)
                .context("container index out of range")?;
            // The `.ubulk` payload is a byte-complete self-describing Reach MCC
            // tag — no external layout needed.
            let bytes = mounted
                .archive
                .read(rel_path)
                .map_err(|e| anyhow!("failed to read {rel_path} from container: {e}"))?;
            TagFile::read_from_bytes(&bytes)
                .map_err(|e| anyhow!("failed to parse {}: {e}", entry.display_path))
        }
        (TagEntryLocation::Container { .. }, _) => {
            anyhow::bail!("container entry selected outside a container source")
        }
        (TagEntryLocation::NewContainer { .. }, _) => {
            // A brand-new tag has no backing payload; its bytes live only in the
            // in-memory document. This is only reached if the document was
            // unloaded (e.g. the tab was closed), by which point the new tag is
            // gone.
            anyhow::bail!("unsaved new tag is no longer loaded")
        }
    }
}

/// Read a tag at `path` for preview/decoding (e.g. a referenced bitmap), handling
/// classic Halo CE / Halo 2 tags that need a JSON layout + `read_classic_tag_file`
/// rather than the plain `TagFile::read`. `group_tag` selects the classic layout.
pub fn read_tag_at_path(
    path: &Path,
    game: Option<&str>,
    definitions_root: Option<&Path>,
    group_tag: u32,
) -> Result<TagFile> {
    let bytes = std::fs::read(path)?;
    if ClassicHeader::parse(&bytes).is_some() {
        let game = game.context("classic tag requires a detected game profile")?;
        let definitions_root =
            definitions_root.context("classic tag requires a definitions root")?;
        let group_name = blam_tags::paths::group_tag_to_extension(group_tag)
            .context("unknown group for classic tag layout")?;
        let def_path = definitions_root
            .join(game)
            .join(format!("{group_name}.json"));
        let layout = TagLayout::from_json(&def_path)
            .with_context(|| format!("failed to load classic layout {}", def_path.display()))?;
        return read_classic_tag_file(&bytes, layout)
            .map_err(|error| anyhow::anyhow!("failed to decode classic tag: {error}"));
    }
    TagFile::read(path).map_err(Into::into)
}

/// Re-parse in-memory tag bytes, honoring classic (Halo CE / Halo 2) format.
///
/// Classic tags serialize with reversed signatures (`!MLB`/`BMAL`, no `BLAM`
/// at 0x3C) and are not self-describing, so `TagFile::read_from_bytes` fails on
/// them — the JSON layout for `group_tag` must be supplied out of band. Used by
/// the undo/redo journal, whose snapshots come straight from
/// `TagFile::write_to_bytes` (which writes classic format for classic tags).
pub fn read_tag_from_bytes(
    bytes: &[u8],
    game: Option<&str>,
    definitions_root: Option<&Path>,
    group_tag: u32,
) -> Result<TagFile> {
    if ClassicHeader::parse(bytes).is_some() {
        let game = game.context("classic tag requires a detected game profile")?;
        let definitions_root =
            definitions_root.context("classic tag requires a definitions root")?;
        let group_name =
            group_tag_to_extension(group_tag).context("unknown group for classic tag layout")?;
        let def_path = definitions_root
            .join(game)
            .join(format!("{group_name}.json"));
        let layout = TagLayout::from_json(&def_path)
            .with_context(|| format!("failed to load classic layout {}", def_path.display()))?;
        return read_classic_tag_file(bytes, layout)
            .map_err(|error| anyhow::anyhow!("failed to decode classic tag: {error}"));
    }
    TagFile::read_from_bytes(bytes).map_err(Into::into)
}

fn read_loose_tag(
    path: &Path,
    entry: &TagEntry,
    game: Option<&str>,
    definitions_root: &Path,
) -> Result<TagFile> {
    let bytes = std::fs::read(path)?;
    if ClassicHeader::parse(&bytes).is_some() {
        let game = game.context(
            "classic Halo CE / Halo 2 tags require a detected game profile to locate definitions",
        )?;
        let group_name = entry.group_name.as_deref().with_context(|| {
            format!(
                "no group definition for {} in definitions/{game}/",
                format_group_tag(entry.group_tag)
            )
        })?;
        let def_path = definitions_root
            .join(game)
            .join(format!("{group_name}.json"));
        if !def_path.is_file() {
            if !definitions_root.is_dir() {
                anyhow::bail!(
                    "{}",
                    crate::app::definitions_missing_message(definitions_root)
                );
            }
            anyhow::bail!(
                "no group definition for {} at {}",
                format_group_tag(entry.group_tag),
                def_path.display()
            );
        }
        let layout = TagLayout::from_json(&def_path)
            .with_context(|| format!("failed to load classic layout {}", def_path.display()))?;
        return read_classic_tag_file(&bytes, layout)
            .map_err(|error| anyhow::anyhow!("failed to decode classic tag: {error}"));
    }
    TagFile::read(path).map_err(Into::into)
}

fn read_non_classic_tag(path: &Path) -> Result<TagFile> {
    let mut header = [0u8; 64];
    if let Ok(mut file) = File::open(path) {
        let read = file.read(&mut header)?;
        if read >= 64 && ClassicHeader::parse(&header).is_some() {
            anyhow::bail!(
                "classic Halo CE / Halo 2 tags require opening an editing-kit tags folder so Baboon can detect the game profile"
            );
        }
    }
    TagFile::read(path).map_err(Into::into)
}

#[cfg(test)]
mod container_tests {
    use super::*;

    const PAKS: &str = "/Users/camden/Halo/halo-campaign-evolved_pc/Meteorite/Content/Paks";

    #[test]
    fn container_ref_key_normalizes() {
        let skel = u32::from_be_bytes(*b"skel");
        // Backslashes → forward, uppercase → lower, trailing NUL stripped,
        // group FOURCC hex-prefixed — matching `build_container_set`'s logical key.
        assert_eq!(
            container_ref_key(skel, "Objects\\Characters\\Elite_AI\\Elite_AI\u{0}"),
            "736b656c:objects/characters/elite_ai/elite_ai"
        );
        // Idempotent on an already-normalized reference.
        assert_eq!(
            container_ref_key(skel, "objects/characters/elite_ai/elite_ai"),
            "736b656c:objects/characters/elite_ai/elite_ai"
        );
    }

    /// Mount the whole `Paks` directory through Baboon's set loader and read a
    /// sample of tags via `read_entry`. Asserts scenarios (only in level chunks)
    /// show up alongside pak0's shared tags. Skipped when the game isn't present.
    #[test]
    fn mount_container_set_and_read_tags() {
        if !Path::new(PAKS).exists() {
            eprintln!("skipping: {PAKS} not present");
            return;
        }
        let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let names = TagNameIndex::load_from_definitions(&defs);
        let loaded = load_iostore_container_set(PathBuf::from(PAKS), &names, &defs)
            .expect("mount container set");

        assert!(
            loaded.entries.len() > 5000,
            "expected thousands of tags, got {}",
            loaded.entries.len()
        );
        assert_eq!(loaded.game.as_deref(), Some("haloce_evolved"));
        let TagSource::IoStoreContainerSet { ref containers, .. } = loaded.source else {
            panic!("expected a container set");
        };
        assert!(
            containers.len() > 10,
            "expected base + level chunks, got {}",
            containers.len()
        );

        // Scenarios live only in level chunks — proves multi-container merge.
        let scnr = u32::from_be_bytes(*b"scnr");
        let scenarios: Vec<&TagEntry> = loaded
            .entries
            .iter()
            .filter(|e| e.group_tag == scnr)
            .collect();
        eprintln!(
            "mounted {} packs, {} tags, {} scenarios",
            containers.len(),
            loaded.entries.len(),
            scenarios.len()
        );
        for s in scenarios.iter().take(3) {
            eprintln!("  scenario: {}", s.display_path);
        }
        assert!(
            scenarios.len() >= 10,
            "expected ~13 scenarios across level chunks, got {}",
            scenarios.len()
        );
        // Display paths are lowercased and Tags/-stripped.
        for s in &scenarios {
            assert!(
                s.display_path == s.display_path.to_ascii_lowercase(),
                "display path not lowercased: {}",
                s.display_path
            );
            assert!(!s.display_path.to_ascii_lowercase().contains("/tags/"));
        }

        // Read a sample (including every scenario) via the source-aware path.
        let mut sample: Vec<&TagEntry> = scenarios.clone();
        sample.extend(loaded.entries.iter().take(300));
        for entry in sample {
            let tag = read_entry(&loaded.source, entry)
                .unwrap_or_else(|e| panic!("read_entry failed for {}: {e}", entry.display_path));
            assert_eq!(
                tag.group().tag,
                entry.group_tag,
                "group mismatch for {}",
                entry.display_path
            );
        }

        // Reference resolution: a CE `.model` (hlmt) resolves its `animation`
        // (jmad) and `skeleton model` (skel) refs through the container index —
        // the payload the browser tree already mounts, by construction.
        let hlmt = u32::from_be_bytes(*b"hlmt");
        let mut checked = false;
        for entry in loaded.entries.iter().filter(|e| e.group_tag == hlmt) {
            let Ok(model) = read_entry(&loaded.source, entry) else {
                continue;
            };
            let root = model.root();
            let (Some((_, jmad_ref)), Some((_, skel_ref))) = (
                root.read_tag_ref_with_group("animation"),
                root.read_tag_ref_with_group("skeleton model"),
            ) else {
                continue;
            };
            if jmad_ref.trim().is_empty() || skel_ref.trim().is_empty() {
                continue;
            }
            let jmad = loaded
                .source
                .read_container_tag_by_ref(u32::from_be_bytes(*b"jmad"), &jmad_ref)
                .unwrap_or_else(|e| panic!("resolve jmad {jmad_ref}: {e}"));
            assert_eq!(jmad.group().tag, u32::from_be_bytes(*b"jmad"));
            let skel = loaded
                .source
                .read_container_tag_by_ref(u32::from_be_bytes(*b"skel"), &skel_ref)
                .unwrap_or_else(|e| panic!("resolve skel {skel_ref}: {e}"));
            assert_eq!(skel.group().tag, u32::from_be_bytes(*b"skel"));
            eprintln!(
                "resolved refs for {}: {jmad_ref} + {skel_ref}",
                entry.display_path
            );
            checked = true;
            break;
        }
        assert!(
            checked,
            "expected an hlmt carrying both animation and skeleton model refs"
        );
    }
}

/// Validates a selected `blob_index.dat` and returns its cache directory.
pub fn normalize_blob_index_path(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if !file_name.eq_ignore_ascii_case("blob_index.dat") {
        anyhow::bail!("expected blob_index.dat, got {}", path.display());
    }
    path.parent()
        .map(Path::to_path_buf)
        .with_context(|| format!("{} has no parent directory", path.display()))
}

#[cfg(test)]
mod paks_dir_tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!("baboon_paks_{name}_{stamp}"))
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("create dirs");
        std::fs::write(path, b"").expect("write file");
    }

    /// Baboon's own transactional artefacts are not containers. A duplicate
    /// leaves an immutable copy of the `.utoc` beside the container it is about
    /// to mutate, and an export builds its replacement in a hidden folder
    /// inside the destination — mounting either would show the user a mod made
    /// of a half-finished write, and shipping either would put it in a mod.
    #[test]
    fn baboons_own_backups_are_never_mounted() {
        let root = temp_dir("backups");
        touch(&root.join("pakchunk0-WinGDK.utoc"));
        touch(&root.join("pakchunk0-WinGDK.utoc.baboon-duplicate-backup"));
        touch(&root.join("pakchunk0-WinGDK.utoc.baboon-duplicate-backup-3"));
        touch(&root.join("~mods/mymod_P.utoc"));
        touch(&root.join("~mods/mymod_P.utoc.previous"));
        touch(&root.join("~mods/.baboon-export-1234-9/mymod_P.utoc"));

        let mut mounted: Vec<String> = utocs_under(&root)
            .iter()
            .map(|path| {
                path.strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        mounted.sort();

        assert_eq!(
            mounted,
            vec![
                "pakchunk0-WinGDK.utoc".to_owned(),
                "~mods/mymod_P.utoc".to_owned()
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn container_entry(key: &str, display_path: &str, container: usize, rel_path: &str) -> TagEntry {
        TagEntry {
            key: key.to_owned(),
            display_path: display_path.to_owned(),
            group_tag: 0x62697064,
            group_name: Some("biped".to_owned()),
            location: TagEntryLocation::Container {
                container,
                rel_path: rel_path.to_owned(),
            },
        }
    }

    /// Mounting a mod over a tag must not change what that tag *is*.
    ///
    /// The key is what open tabs, parsed documents and the undo journal are
    /// filed under. Replacing the entry wholesale gave the tag a new identity
    /// derived from the mod's container label, so every tab of a just-exported
    /// tag showed its raw key and "This tag is no longer in the source", with
    /// the user's edits stranded behind it.
    #[test]
    fn a_mod_layered_over_a_tag_leaves_its_identity_alone() {
        let mut entries = vec![container_entry(
            "ublock:pakchunk0-Windows:Meteorite/Content/Tags/objects/brute-biped.ubulk",
            "objects/brute.biped",
            0,
            "Meteorite/Content/Tags/objects/brute-biped.ubulk",
        )];

        layer_entry(
            &mut entries,
            &container_entry(
                "ublock:mymod_P:Meteorite/Content/Tags/objects/brute-biped.ubulk",
                "objects/brute.biped",
                4,
                "Meteorite/Content/Tags/objects/brute-biped.ubulk",
            ),
        );

        assert_eq!(entries.len(), 1, "the mod replaces rather than duplicates");
        assert_eq!(
            entries[0].key, "ublock:pakchunk0-Windows:Meteorite/Content/Tags/objects/brute-biped.ubulk",
            "the tag keeps the identity its open tab is filed under"
        );
        // What did change is where it is read from.
        assert!(matches!(
            &entries[0].location,
            TagEntryLocation::Container { container: 4, .. }
        ));
    }

    #[test]
    fn a_tag_only_the_mod_carries_is_added_in_sorted_position() {
        let mut entries = vec![
            container_entry("ublock:pak:a.ubulk", "objects/a.biped", 0, "a.ubulk"),
            container_entry("ublock:pak:z.ubulk", "objects/z.biped", 0, "z.ubulk"),
        ];

        layer_entry(
            &mut entries,
            &container_entry("ublock:mymod_P:m.ubulk", "objects/m.biped", 4, "m.ubulk"),
        );

        let order: Vec<&str> = entries
            .iter()
            .map(|entry| entry.display_path.as_str())
            .collect();
        assert_eq!(
            order,
            ["objects/a.biped", "objects/m.biped", "objects/z.biped"],
            "a new tag lands beside its neighbours, not at the bottom of the list"
        );
    }

    #[test]
    fn a_backup_is_recognised_wherever_it_sits() {
        for path in [
            "D:/Paks/pakchunk0.utoc.baboon-duplicate-backup",
            "D:/Paks/pakchunk0.utoc.baboon-duplicate-backup-7",
            "D:/Paks/pakchunk0.utoc.baboon-duplicate-backup.manifest.json",
            "D:/Paks/~mods/mymod_P.utoc.previous",
            "D:/Paks/~mods/.baboon-export-900-1/mymod_P.utoc",
        ] {
            assert!(is_container_backup(Path::new(path)), "{path}");
        }
        for path in [
            "D:/Paks/pakchunk0.utoc",
            "D:/Paks/~mods/mymod_P.utoc",
            "D:/Paks/~mods/baboon-export/mymod_P.utoc",
        ] {
            assert!(!is_container_backup(Path::new(path)), "{path}");
        }
    }

    /// The game root of an install that has had a mod exported into it: a
    /// stray `.utoc` sits beside the executable. The real containers must win.
    #[test]
    fn the_install_layout_beats_a_stray_container_in_the_root() {
        let root = temp_dir("stray");
        let paks = root.join("Meteorite").join("Content").join("Paks");
        touch(&root.join("mymod-WinGDK_P.utoc"));
        touch(&paks.join("pakchunk0-WinGDK.utoc"));

        assert_eq!(find_paks_dir(&root), Some(paks));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Picking the `Paks` directory itself still resolves to itself.
    #[test]
    fn picking_the_paks_directory_resolves_to_itself() {
        let root = temp_dir("direct");
        let paks = root.join("Meteorite").join("Content").join("Paks");
        touch(&paks.join("pakchunk0-WinGDK.utoc"));

        assert_eq!(find_paks_dir(&paks), Some(paks.clone()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A bare directory of containers not named `Paks` is still accepted, as
    /// the last resort rather than the first check.
    #[test]
    fn a_bare_container_directory_is_still_accepted() {
        let root = temp_dir("bare");
        touch(&root.join("pakchunk0-WinGDK.utoc"));

        assert_eq!(find_paks_dir(&root), Some(root.clone()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The real install, which is what surfaced this: its root holds an
    /// exported `mymod-WinGDK_P.utoc`. Skipped when the game is not present.
    #[test]
    fn the_real_install_root_resolves_to_its_paks_directory() {
        const ROOT: &str = "/Users/camden/Halo/halo-campaign-evolved_pc";
        if !Path::new(ROOT).is_dir() {
            return;
        }
        assert_eq!(
            find_paks_dir(Path::new(ROOT)),
            Some(
                PathBuf::from(ROOT)
                    .join("Meteorite")
                    .join("Content")
                    .join("Paks")
            )
        );
    }

    #[test]
    fn a_folder_with_no_containers_is_not_an_install() {
        let root = temp_dir("none");
        touch(&root.join("readme.txt"));

        assert_eq!(find_paks_dir(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod mod_export_tests {
    use super::*;

    const PAKS: &str = "/Users/camden/Halo/halo-campaign-evolved_pc/Meteorite/Content/Paks";

    /// End-to-end check of what Export Mod actually writes, short of the game
    /// loading it: take a real container tag, change a byte, write an override
    /// container, then read the tag back out of that container.
    ///
    /// Reported as "it creates the pak but ingame nothing happens", and the
    /// in-game load has never been verified on this machine — so this pins down
    /// whether the artifact carries the edit at all.
    #[test]
    fn exported_mod_container_carries_the_edited_bytes() {
        if !Path::new(PAKS).exists() {
            eprintln!("skipping: {PAKS} not present");
            return;
        }
        let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let names = TagNameIndex::load_from_definitions(&defs);
        let loaded = load_iostore_container_set(PathBuf::from(PAKS), &names, &defs).expect("mount");
        let TagSource::IoStoreContainerSet { ref containers, .. } = loaded.source else {
            panic!("expected a container set");
        };

        // A tag whose bytes we can perturb without changing its length, so the
        // export takes the common same-size path.
        let (container, rel_path, original) = loaded
            .entries
            .iter()
            .find_map(|entry| match &entry.location {
                TagEntryLocation::Container {
                    container,
                    rel_path,
                } => {
                    let archive = &containers.get(*container)?.archive;
                    let bytes = archive.read(rel_path).ok()?;
                    (bytes.len() > 64).then(|| (*container, rel_path.clone(), bytes))
                }
                _ => None,
            })
            .expect("a readable container tag");

        let mut edited = original.clone();
        let last = edited.len() - 1;
        edited[last] ^= 0xFF;

        let archive = containers[container].archive.clone();
        let dir = std::env::temp_dir().join(format!("baboon-modexport-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let out = dir.join("mymod-WinGDK_P.utoc");
        blam_tags::iostore::writer::write_mod_container_ex(
            &[(archive.as_ref(), rel_path.as_str(), edited.as_slice())],
            &[],
            &out,
        )
        .expect("write override container");

        // The game discovers containers by scanning `Paks/*.pak`, so all three
        // files have to be there — a missing stub is a mod that never loads.
        for ext in ["utoc", "ucas", "pak"] {
            let path = out.with_extension(ext);
            assert!(path.is_file(), "{} was not written", path.display());
            eprintln!(
                "{}: {} bytes",
                path.file_name().unwrap().to_string_lossy(),
                std::fs::metadata(&path).unwrap().len()
            );
        }

        // An override container carries chunks by id with no directory index --
        // the game resolves packages through the global store, not this TOC --
        // so the payload is checked by chunk rather than by path. The chunk id
        // itself is taken from the base archive when the override is built, so
        // it matches the tag it is overriding by construction.
        let reopened =
            blam_tags::iostore::IoStoreArchive::open(&out).expect("reopen exported container");
        // Two chunks even for a same-size edit: the tag and its paired
        // `.uasset`. The `.uasset` rides along so the mod stays editable — in-place
        // surgery can only repoint chunks the container already has, and without
        // it a later size-changing edit could never rewrite the declared length.
        assert_eq!(
            reopened.chunk_count(),
            2,
            "an override should carry the tag and its .uasset"
        );
        let ub_id = archive
            .chunk_id_for(&rel_path)
            .expect("the base names the tag's chunk");
        let ub_chunk = reopened
            .find_chunk(&ub_id)
            .expect("the override reuses the base chunk id");
        let served = reopened
            .read_chunk(ub_chunk)
            .expect("read the override chunk");
        assert_eq!(
            served, edited,
            "the exported container did not carry the edited bytes for {rel_path}"
        );
        assert_ne!(
            served, original,
            "the exported container carried the base bytes"
        );
        eprintln!("override verified for {rel_path} ({} bytes)", served.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An exported mod opens EMPTY in the browser: an override container is
    /// addressed by chunk id and ships with no directory index, so the file list
    /// it advertises is nothing at all. Assert on what the tag browser actually
    /// derives — the mounted entries — not on the raw chunks, which were already
    /// fine while the UI showed an empty tree.
    #[test]
    fn a_mod_container_lists_its_tags_when_opened_on_its_own() {
        if !Path::new(PAKS).exists() {
            eprintln!("skipping: {PAKS} not present");
            return;
        }
        let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let names = TagNameIndex::load_from_definitions(&defs);
        let loaded = load_iostore_container_set(PathBuf::from(PAKS), &names, &defs).expect("mount");
        let TagSource::IoStoreContainerSet { ref containers, .. } = loaded.source else {
            panic!("expected a container set");
        };
        let (container, rel_path, original) = loaded
            .entries
            .iter()
            .find_map(|entry| match &entry.location {
                TagEntryLocation::Container {
                    container,
                    rel_path,
                } => {
                    let archive = &containers.get(*container)?.archive;
                    let bytes = archive.read(rel_path).ok()?;
                    (bytes.len() > 64).then(|| (*container, rel_path.clone(), bytes))
                }
                _ => None,
            })
            .expect("a readable container tag");

        let mut edited = original.clone();
        let last = edited.len() - 1;
        edited[last] ^= 0xFF;

        // Write the mod into the game's own Paks folder, which is where mods
        // live and therefore where they get opened from.
        let out =
            PathBuf::from(PAKS).join(format!("baboon-listing-test-{}_P.utoc", std::process::id()));
        let archive = containers[container].archive.clone();
        blam_tags::iostore::writer::write_mod_container_ex(
            &[(archive.as_ref(), rel_path.as_str(), edited.as_slice())],
            &[],
            &out,
        )
        .expect("write override container");

        // No install root known: the container has only the folder it is in.
        let opened = load_iostore_container(out.clone(), None, &names, &defs);
        for ext in ["utoc", "ucas", "pak"] {
            let _ = std::fs::remove_file(out.with_extension(ext));
        }
        let opened = opened.expect("mount the exported mod on its own");

        assert!(
            !opened.entries.is_empty(),
            "opening a mod container listed no tags at all"
        );
        let listed = opened.entries.iter().any(|entry| match &entry.location {
            TagEntryLocation::Container { rel_path: p, .. } => *p == rel_path,
            _ => false,
        });
        assert!(listed, "the overridden tag {rel_path} was not listed");
        eprintln!(
            "mod container listed {} tag(s); found {rel_path}",
            opened.entries.len()
        );
    }

    /// Mods are commonly installed in a folder under `Paks` — `~mods`, `~.mods`
    /// — not loose beside the game's own paks. The game finds them there because
    /// UE scans the pak folder recursively; a flat scan mounted the base game
    /// and reported no sign of the mod at all.
    ///
    /// Both ways in are covered: mounting the install, and opening the mod's own
    /// `.utoc` against the install root the app already knows.
    #[test]
    fn a_mod_installed_below_the_paks_folder_is_mounted() {
        if !Path::new(PAKS).exists() {
            eprintln!("skipping: {PAKS} not present");
            return;
        }
        let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let names = TagNameIndex::load_from_definitions(&defs);
        let loaded = load_iostore_container_set(PathBuf::from(PAKS), &names, &defs).expect("mount");
        let TagSource::IoStoreContainerSet { ref containers, .. } = loaded.source else {
            panic!("expected a container set");
        };
        let (container, rel_path, original) = loaded
            .entries
            .iter()
            .find_map(|entry| match &entry.location {
                TagEntryLocation::Container {
                    container,
                    rel_path,
                } => {
                    let archive = &containers.get(*container)?.archive;
                    let bytes = archive.read(rel_path).ok()?;
                    (bytes.len() > 64).then(|| (*container, rel_path.clone(), bytes))
                }
                _ => None,
            })
            .expect("a readable container tag");

        let mut edited = original.clone();
        let last = edited.len() - 1;
        edited[last] ^= 0xFF;

        let mods_dir = PathBuf::from(PAKS).join("~mods");
        std::fs::create_dir_all(&mods_dir).expect("create the mod folder");
        let out = mods_dir.join(format!("baboon-submod-test-{}_P.utoc", std::process::id()));
        blam_tags::iostore::writer::write_mod_container_ex(
            &[(
                containers[container].archive.as_ref(),
                rel_path.as_str(),
                edited.as_slice(),
            )],
            &[],
            &out,
        )
        .expect("write override container");

        // Everything from here has to clean up after itself: the mod is sitting
        // in the user's install and must not outlive the test.
        let result = std::panic::catch_unwind(|| {
            // Mounting the install must serve the tag out of the mod, not the
            // base pak it overrides.
            let with_mod = load_iostore_container_set(PathBuf::from(PAKS), &names, &defs)
                .expect("remount the install");
            let TagSource::IoStoreContainerSet {
                containers: ref mounted,
                ..
            } = with_mod.source
            else {
                panic!("expected a container set");
            };
            let served = with_mod
                .entries
                .iter()
                .find_map(|entry| match &entry.location {
                    TagEntryLocation::Container {
                        container,
                        rel_path: p,
                    } if *p == rel_path => {
                        let mounted = mounted.get(*container)?;
                        Some((mounted.utoc_path.clone(), mounted.archive.read(p).ok()?))
                    }
                    _ => None,
                })
                .expect("the overridden tag is in the mounted set");
            assert_eq!(
                served.0,
                out,
                "{rel_path} is still served by {}, not the mod under ~mods",
                served.0.display()
            );
            assert_eq!(served.1, edited, "the mod's bytes were not the ones served");

            // Opening the mod alone, against the install root the app knows.
            let opened =
                load_iostore_container(out.clone(), Some(PathBuf::from(PAKS)), &names, &defs)
                    .expect("mount the mod against the install");
            let listed = opened.entries.iter().any(|entry| match &entry.location {
                TagEntryLocation::Container { rel_path: p, .. } => *p == rel_path,
                _ => false,
            });
            assert!(
                listed,
                "the mod listed {} tag(s), none of them {rel_path}",
                opened.entries.len()
            );
        });

        for ext in ["utoc", "ucas", "pak"] {
            let _ = std::fs::remove_file(out.with_extension(ext));
        }
        let _ = std::fs::remove_dir(&mods_dir);
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
        eprintln!("a mod under ~mods served {rel_path}");
    }

    /// Save a tag that is being served by an already-exported mod: edit, export,
    /// reload the folder, edit again, Save. The mod outranks the base pak, so
    /// the save writes into the MOD — which ships no directory index, and
    /// resolving the write through a freshly opened handle failed with
    /// `path not found in container`.
    #[test]
    fn a_tag_served_by_an_exported_mod_can_be_saved_into_it_again() {
        if !Path::new(PAKS).exists() {
            eprintln!("skipping: {PAKS} not present");
            return;
        }
        let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let names = TagNameIndex::load_from_definitions(&defs);
        let loaded = load_iostore_container_set(PathBuf::from(PAKS), &names, &defs).expect("mount");
        let TagSource::IoStoreContainerSet { ref containers, .. } = loaded.source else {
            panic!("expected a container set");
        };
        let (container, rel_path, original) = loaded
            .entries
            .iter()
            .find_map(|entry| match &entry.location {
                TagEntryLocation::Container {
                    container,
                    rel_path,
                } => {
                    let archive = &containers.get(*container)?.archive;
                    let bytes = archive.read(rel_path).ok()?;
                    (bytes.len() > 64).then(|| (*container, rel_path.clone(), bytes))
                }
                _ => None,
            })
            .expect("a readable container tag");

        let mut exported = original.clone();
        let last = exported.len() - 1;
        exported[last] ^= 0xFF;

        // Export the mod into the game's own Paks folder, where mods live.
        let out =
            PathBuf::from(PAKS).join(format!("baboon-resave-test-{}_P.utoc", std::process::id()));
        blam_tags::iostore::writer::write_mod_container_ex(
            &[(
                containers[container].archive.as_ref(),
                rel_path.as_str(),
                exported.as_slice(),
            )],
            &[],
            &out,
        )
        .expect("write override container");

        // Everything from here has to clean up after itself: the mod is sitting
        // in the user's install and must not outlive the test.
        let result = std::panic::catch_unwind(|| {
            let opened = load_iostore_container(out.clone(), None, &names, &defs)
                .expect("mount the exported mod");
            let TagSource::IoStoreContainerSet {
                ref root,
                ref containers,
                ..
            } = opened.source
            else {
                panic!("expected a container set");
            };
            let (index, rel) = opened
                .entries
                .iter()
                .find_map(|entry| match &entry.location {
                    TagEntryLocation::Container {
                        container,
                        rel_path: p,
                    } if *p == rel_path => Some((*container, p.clone())),
                    _ => None,
                })
                .expect("the mod serves the overridden tag");

            let mut resaved = exported.clone();
            resaved[0..4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
            let mounted = &containers[index];
            blam_tags::iostore::writer::overwrite_tag_in_place_with(
                &mounted.archive,
                &mounted.utoc_path,
                &rel,
                &resaved,
            )
            .expect("save into the mod container");

            // What the app does next: reopen the pak it just wrote. A plain
            // reopen loses the recovered file list, and the tag becomes
            // unreadable and unsaveable for the rest of the session.
            let reopened = reopen_container_archive(root, containers, index)
                .expect("reopen the written container");
            assert_eq!(
                reopened.read(&rel).expect("read the tag back"),
                resaved,
                "the mod did not serve the re-saved bytes"
            );
        });

        for ext in ["utoc", "ucas", "pak"] {
            let _ = std::fs::remove_file(out.with_extension(ext));
        }
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
        eprintln!("re-saved {rel_path} into its own exported mod");
    }
}

