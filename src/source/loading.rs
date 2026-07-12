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
