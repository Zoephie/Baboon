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
    let game = info.game.map(str::to_owned);
    let names = game
        .as_deref()
        .and_then(|g| TagNameIndex::load_game(definitions_root, g).ok())
        .unwrap_or_else(|| fallback_names.clone());
    let entries = Vec::new();
    let tree = build_folder_directory_tree(&info.scan_root)
        .with_context(|| format!("failed to list folders in {}", info.scan_root.display()))?;
    // Pre-load a saved index so Groups and search work immediately.
    let all_entries = game
        .as_deref()
        .and_then(|g| load_entry_index(g, &info.scan_root))
        .unwrap_or_default();
    let reverse_dependencies = game
        .as_deref()
        .and_then(|g| load_reverse_dependency_index(g, &info.scan_root));
    let group_tree = build_group_tree(&all_entries);
    Ok(LoadedSourceData {
        label: info.label,
        source: TagSource::LooseFolder {
            root: info.scan_root,
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
    let mut utocs: Vec<PathBuf> = std::fs::read_dir(&paks_dir)
        .with_context(|| format!("failed to read {}", paks_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("utoc")))
        // `global.utoc` has no directory index; it would fail to open anyway.
        .filter(|p| !p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("global.utoc")))
        .collect();
    // Mount base chunk first, then level chunks by number, so higher/patch
    // chunks win on any collision (mirrors UE's FIoDispatcher last-wins).
    utocs.sort_by_key(|p| (chunk_number(p), p.clone()));
    build_container_set(paks_dir, utocs, fallback_names, definitions_root)
}

/// Mounts a single IoStore container (`.utoc`) — the "open one chunk" path. The
/// resulting source is still a set (of one).
pub fn load_iostore_container(
    utoc: PathBuf,
    fallback_names: &TagNameIndex,
    definitions_root: &Path,
) -> Result<LoadedSourceData> {
    let root = utoc.parent().map(Path::to_path_buf).unwrap_or_else(|| utoc.clone());
    build_container_set(root, vec![utoc], fallback_names, definitions_root)
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
    let mut opened_any = false;

    for utoc in utocs {
        let archive = match IoStoreArchive::open(&utoc) {
            Ok(a) => a,
            // Skip containers we can't parse (e.g. index-less globals).
            Err(_) => continue,
        };
        opened_any = true;
        let chunk_label = utoc
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("container")
            .to_string();
        let container_index = containers.len();
        let archive = Arc::new(archive);
        let mut contributed = false;

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
                format!("{}/{}", dir.to_ascii_lowercase(), tag_name.to_ascii_lowercase())
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
            containers.push(MountedContainer {
                utoc_path: utoc,
                chunk_label,
                archive,
            });
        }
    }

    if !opened_any {
        anyhow::bail!("no readable IoStore containers found in {}", root.display());
    }

    entries.sort_by(|a, b| natural_key(&a.display_path).cmp(&natural_key(&b.display_path)));
    let label = format!(
        "{} ({} packs)",
        root.file_name().and_then(|s| s.to_str()).unwrap_or("Campaign Evolved"),
        containers.len()
    );
    let tree = build_tree(&entries);
    let group_tree = build_group_tree(&entries);
    Ok(LoadedSourceData {
        label,
        source: TagSource::IoStoreContainerSet { root, containers },
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
pub fn find_paks_dir(root: &Path) -> Option<PathBuf> {
    if dir_has_utoc(root) {
        return Some(root.to_path_buf());
    }
    for cand in [
        root.join("Meteorite").join("Content").join("Paks"),
        root.join("Content").join("Paks"),
    ] {
        if dir_has_utoc(&cand) {
            return Some(cand);
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
    None
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
        assert!(containers.len() > 10, "expected base + level chunks, got {}", containers.len());

        // Scenarios live only in level chunks — proves multi-container merge.
        let scnr = u32::from_be_bytes(*b"scnr");
        let scenarios: Vec<&TagEntry> =
            loaded.entries.iter().filter(|e| e.group_tag == scnr).collect();
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
                tag.group().tag, entry.group_tag,
                "group mismatch for {}", entry.display_path
            );
        }
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
