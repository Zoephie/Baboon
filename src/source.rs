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
use serde_json;
use walkdir::WalkDir;

use crate::format::TagNameIndex;

#[derive(Clone, Copy, Debug, Default)]
pub struct EntryIndexScanProgress {
    pub processed: usize,
    pub total: usize,
    pub matched: usize,
}

#[derive(Clone)]
pub struct TagEntry {
    pub key: String,
    pub display_path: String,
    pub group_tag: u32,
    pub group_name: Option<String>,
    pub location: TagEntryLocation,
}

#[derive(Clone)]
pub enum TagEntryLocation {
    LooseFile(PathBuf),
    Monolithic { name: String, group_tag: u32 },
}

#[derive(Clone)]
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
pub struct ReverseDependencyIndex {
    by_tag: BTreeMap<String, Vec<DependencyRef>>,
    by_dependency: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
pub struct TagTree {
    pub children: Vec<TagTreeNode>,
    pub entries: Vec<usize>,
}

#[derive(Default)]
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

pub fn build_tree(entries: &[TagEntry]) -> TagTree {
    let mut root = TreeBuildNode::default();
    for (index, entry) in entries.iter().enumerate() {
        let parts = split_display_path(&entry.display_path);
        if parts.len() <= 1 {
            root.entries.push(index);
            continue;
        }

        let mut node = &mut root;
        for part in &parts[..parts.len() - 1] {
            node = node.children.entry(part.clone()).or_default();
        }
        node.entries.push(index);
    }
    TagTree {
        children: root
            .children
            .into_iter()
            .map(|(label, node)| finish_node(label, node))
            .collect(),
        entries: root.entries,
    }
}

pub fn build_group_tree(entries: &[TagEntry]) -> TagTree {
    let mut root = TreeBuildNode::default();
    for (index, entry) in entries.iter().enumerate() {
        let fourcc = format_group_tag(entry.group_tag);
        let group = friendly_group_name(entry.group_tag, entry.group_name.as_deref(), &fourcc);
        let label = if group == fourcc {
            fourcc
        } else {
            format!("{group} {fourcc}")
        };
        root.children.entry(label).or_default().entries.push(index);
    }
    TagTree {
        children: root
            .children
            .into_iter()
            .map(|(label, node)| finish_node(label, node))
            .collect(),
        entries: root.entries,
    }
}

fn friendly_group_name(group_tag: u32, indexed_name: Option<&str>, fourcc: &str) -> String {
    if let Some(name) = indexed_name {
        if !name.eq_ignore_ascii_case(fourcc) {
            return name.to_owned();
        }
    }
    fallback_group_name(group_tag)
        .map(str::to_owned)
        .unwrap_or_else(|| fourcc.to_owned())
}

fn fallback_group_name(group_tag: u32) -> Option<&'static str> {
    group_tag_to_extension(group_tag).or_else(|| {
        let fourcc = group_tag.to_be_bytes();
        Some(match &fourcc {
            b"achi" => "achievements",
            b"adlg" => "ai_dialogue_globals",
            b"aigl" => "ai_globals",
            b"mdlg" => "ai_mission_dialogue",
            b"airs" => "airstrike",
            b"ant!" => "antenna",
            b"sefc" => "area_screen_effect",
            b"armg" => "armormod_globals",
            b"fogg" => "atmosphere_fog",
            b"atgf" => "atmosphere_globals",
            b"aulp" => "authored_light_probe",
            b"avat" => "avatar_awards",
            b"bink" => "bink",
            b"bsdt" => "breakable_surface",
            b"zone" => "cache_file_resource_gestalt",
            b"play" => "cache_file_resource_layout_table",
            b"$#!+" => "cache_file_sound",
            b"csdt" => "camera_shake",
            b"trak" => "camera_track",
            b"cmoe" => "camo",
            b"chdg" => "challenge_globals_definition",
            b"char" => "character",
            b"cine" => "cinematic",
            b"cisd" => "cinematic_scene_data",
            b"cisc" => "cinematic_scene",
            b"clwd" => "cloth",
            b"cddf" => "collision_damage",
            b"colo" => "color_table",
            b"cntl" => "contrail_system",
            b"bloc" => "crate",
            b"jpt!" => "damage_effect",
            b"drdf" => "damage_response_definition",
            b"decs" => "decal_system",
            b"dctr" => "decorator_set",
            b"ctrl" => "device_control",
            b"mach" => "device_machine",
            b"term" => "device_terminal",
            b"udlg" => "dialogue",
            b"effe" => "effect",
            b"efsc" => "effect_scenery",
            b"eqip" => "equipment",
            b"forg" => "forge_globals",
            b"fpch" => "fragment_program_control",
            b"glps" => "global_pixel_shader",
            b"matg" => "globals",
            b"grup" => "gui_group_widget_definition",
            b"gint" => "giant",
            b"goof" => "gui_datasource_definition",
            b"txt3" => "gui_text_widget_definition",
            b"wigl" => "user_interface_globals_definition",
            b"ugh!" => "sound_cache_file_gestalt",
            b"ligh" => "light",
            b"ltvl" => "light_volume_system",
            b"unic" => "multilingual_unicode_string_list",
            b"pman" => "particle_model",
            b"pmov" => "particle_physics",
            b"phmo" => "physics_model",
            b"proj" => "projectile",
            b"rasg" => "rasterizer_globals",
            b"rm  " => "render_method",
            b"rmb " => "shader_beam",
            b"rmcs" => "shader_custom",
            b"rmct" => "shader_cortana",
            b"rmd " => "shader_decal",
            b"rmfl" => "shader_foliage",
            b"rmhg" => "shader_halogram",
            b"rmp " => "shader_particle",
            b"rmsk" => "shader_skin",
            b"rmtr" => "shader_terrain",
            b"rmw " => "shader_water",
            b"rmsh" => "shader",
            b"scnr" => "scenario",
            b"sbsp" => "scenario_structure_bsp",
            b"scen" => "scenery",
            b"ssce" => "sound_scenery",
            b"snd!" => "sound",
            b"snde" => "sound_effect_template",
            b"lsnd" => "sound_looping",
            b"spk!" => "sound_mix",
            b"stli" => "scenario_structure_lighting_info",
            b"styl" => "style",
            b"trac" => "tracer_system",
            b"unit" => "unit",
            b"vehi" => "vehicle",
            b"weap" => "weapon",
            b"wind" => "wind",
            _ => return None,
        })
    })
}

pub fn load_folder_node_entries(
    root: &Path,
    node: &mut TagTreeNode,
    entries: &mut Vec<TagEntry>,
    names: &TagNameIndex,
) -> Result<()> {
    if !node.children_loaded {
        node.children = list_direct_child_nodes(root, &node.rel_path)?;
        node.children_loaded = true;
    }
    if node.entries_loaded {
        return Ok(());
    }
    let folder = root.join(&node.rel_path);
    let mut new_entries = scan_folder_direct_entries(root, &folder, names)?;
    new_entries.sort_by(|a, b| natural_key(&a.display_path).cmp(&natural_key(&b.display_path)));
    let start = entries.len();
    node.entries.extend(start..start + new_entries.len());
    entries.extend(new_entries);
    node.entries_loaded = true;
    Ok(())
}

pub fn scan_folder_subtree_entries(
    root: &Path,
    rel_path: &Path,
    names: &TagNameIndex,
) -> Result<Vec<TagEntry>> {
    scan_folder_subtree_entries_with_progress(root, rel_path, names, |_| {})
}

pub fn scan_folder_subtree_entries_with_progress<F>(
    root: &Path,
    rel_path: &Path,
    names: &TagNameIndex,
    progress: F,
) -> Result<Vec<TagEntry>>
where
    F: Fn(EntryIndexScanProgress) + Sync,
{
    let folder = root.join(rel_path);
    let mut paths = Vec::new();
    for item in WalkDir::new(&folder).follow_links(false) {
        let item = item?;
        if !item.file_type().is_file() {
            continue;
        }
        paths.push(item.into_path());
    }

    let total = paths.len();
    progress(EntryIndexScanProgress {
        processed: 0,
        total,
        matched: 0,
    });

    let worker_count = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(1, paths.len().max(1));
    let chunk_size = paths.len().div_ceil(worker_count).max(1);
    let mut probed = Vec::new();
    let processed = AtomicUsize::new(0);
    let matched = AtomicUsize::new(0);

    std::thread::scope(|scope| -> Result<()> {
        let mut handles = Vec::new();
        for chunk in paths.chunks(chunk_size) {
            let progress = &progress;
            let processed = &processed;
            let matched = &matched;
            handles.push(scope.spawn(move || -> Result<Vec<(PathBuf, u32)>> {
                let mut chunk_entries = Vec::new();
                for path in chunk {
                    if let Some(group_tag) = probe_tag_group(path)? {
                        matched.fetch_add(1, Ordering::Relaxed);
                        chunk_entries.push((path.clone(), group_tag));
                    }
                    let processed_now = processed.fetch_add(1, Ordering::Relaxed) + 1;
                    if processed_now == total || processed_now % 256 == 0 {
                        progress(EntryIndexScanProgress {
                            processed: processed_now,
                            total,
                            matched: matched.load(Ordering::Relaxed),
                        });
                    }
                }
                Ok(chunk_entries)
            }));
        }

        for handle in handles {
            let chunk_entries = handle
                .join()
                .map_err(|_| anyhow!("tag index worker panicked"))??;
            probed.extend(chunk_entries);
        }
        Ok(())
    })?;
    progress(EntryIndexScanProgress {
        processed: total,
        total,
        matched: matched.load(Ordering::Relaxed),
    });

    let mut entries = Vec::with_capacity(probed.len());
    for (path, group_tag) in probed {
        let rel = path.strip_prefix(root).unwrap_or(path.as_path());
        let group_name = names.name_for(group_tag).map(str::to_owned);
        let display_path = display_path_with_friendly_extension(rel, group_tag, names);
        entries.push(TagEntry {
            key: format!("file:{}", path.display()),
            display_path,
            group_tag,
            group_name,
            location: TagEntryLocation::LooseFile(path),
        });
    }
    entries.sort_by(|a, b| natural_key(&a.display_path).cmp(&natural_key(&b.display_path)));
    Ok(entries)
}

pub fn loose_file_entry(
    root: &Path,
    path: &Path,
    names: &TagNameIndex,
) -> Result<Option<TagEntry>> {
    let Some(group_tag) = probe_tag_group(path)? else {
        return Ok(None);
    };
    let rel = path.strip_prefix(root).unwrap_or(path);
    let group_name = names.name_for(group_tag).map(str::to_owned);
    let display_path = display_path_with_friendly_extension(rel, group_tag, names);
    Ok(Some(TagEntry {
        key: format!("file:{}", path.display()),
        display_path,
        group_tag,
        group_name,
        location: TagEntryLocation::LooseFile(path.to_path_buf()),
    }))
}

// ── Index persistence ─────────────────────────────────────────────────────────

/// Derive a stable index filename from the game name stored in `FolderRootInfo`.
/// e.g. "halo3_mcc" → `halo3_mcc_index.json`.
pub fn index_path(game: &str) -> PathBuf {
    app_cache_path(&format!("{game}_index.json"), "Baboon", "baboon")
}

pub fn reverse_dependency_index_path(game: &str) -> PathBuf {
    app_cache_path(
        &format!("{game}_reverse_dependencies.json"),
        "Baboon",
        "baboon",
    )
}

/// Sidecar file storing user keywords for a game's tags (kept outside the tag
/// binaries). Keyed by tag entry key → sorted unique keyword list.
pub fn keywords_path(game: &str) -> PathBuf {
    app_cache_path(&format!("{game}_keywords.json"), "Baboon", "baboon")
}

fn legacy_index_path(game: &str) -> PathBuf {
    app_cache_path(&format!("{game}_index.json"), "Genesis", "genesis")
}

fn cache_root_key(root: &Path) -> String {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let key = canonical.display().to_string();
    #[cfg(windows)]
    {
        key.replace('/', "\\").to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        key
    }
}

fn cache_root_str_key(root: &str) -> String {
    cache_root_key(Path::new(root))
}

fn app_cache_path(filename: &str, windows_folder: &str, unix_folder: &str) -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join(windows_folder).join(filename);
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(home)
            .join(".config")
            .join(unix_folder)
            .join(filename);
    }
    PathBuf::from(filename)
}

/// Serialize `entries` to `{game}_index.json`. Called from the background
/// worker after a full scan completes so it never blocks the UI thread.
/// `root` is recorded so a stale index from a different folder for the same
/// game is rejected on load.
pub fn save_entry_index(game: &str, root: &Path, entries: &[TagEntry]) -> Result<()> {
    let path = index_path(game);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create config dir")?;
    }
    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            let rel_path = entry_relative_path(root, e)
                .map(|path| path_to_display(&path))
                .unwrap_or_default();
            let fingerprint =
                entry_file_path(e).and_then(|path| file_fingerprint(path).ok().flatten());
            serde_json::json!({
                "rel_path": rel_path,
                "key": &e.key,
                "display_path": &e.display_path,
                "group_tag": e.group_tag,
                "group_name": e.group_name,
                "size": fingerprint.map(|fp| fp.size),
                "modified_secs": fingerprint.map(|fp| fp.modified_secs),
                "modified_nanos": fingerprint.map(|fp| fp.modified_nanos),
            })
        })
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::json!({
        "version": 2,
        "root": cache_root_key(root),
        "entries": items,
    }))
    .context("serialize index")?;
    std::fs::write(&path, text).context("write index file")?;
    Ok(())
}

/// Load a previously saved index for `game`. Returns `None` if no file exists,
/// it can't be parsed, or it was saved for a different `root` folder (the keys
/// are absolute paths, so the index is only valid for its original root).
pub fn load_entry_index(game: &str, root: &Path) -> Option<Vec<TagEntry>> {
    let value = load_entry_index_value(game)?;
    let (entries, _) = parse_entry_index(root, &value)?;
    (!entries.is_empty()).then_some(entries)
}

pub fn refresh_entry_index(
    game: &str,
    root: &Path,
    names: &TagNameIndex,
) -> Result<EntryIndexRefresh> {
    let value = load_entry_index_value(game);
    let (cached_entries, cached_fingerprints) = value
        .as_ref()
        .and_then(|value| parse_entry_index(root, value))
        .unwrap_or_default();
    refresh_entry_index_from_cache(root, names, &cached_entries, &cached_fingerprints)
}

fn load_entry_index_value(game: &str) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(index_path(game))
        .or_else(|_| std::fs::read_to_string(legacy_index_path(game)))
        .ok()?;
    serde_json::from_str(&text).ok()
}

fn parse_entry_index(
    root: &Path,
    value: &serde_json::Value,
) -> Option<(Vec<TagEntry>, HashMap<PathBuf, EntryFingerprint>)> {
    let saved_root = value.get("root").and_then(|v| v.as_str())?;
    if cache_root_str_key(saved_root) != cache_root_key(root) {
        return None;
    }

    let items = value.get("entries")?.as_array()?;
    let mut entries = Vec::with_capacity(items.len());
    let mut fingerprints = HashMap::new();
    for item in items {
        let entry = entry_from_index_item(item)?;
        if let Some(rel_path) = index_item_relative_path(root, item, &entry)
            && let Some(fingerprint) = fingerprint_from_index_item(item)
        {
            fingerprints.insert(normalize_rel_path(&rel_path), fingerprint);
        }
        entries.push(entry);
    }
    Some((entries, fingerprints))
}

fn entry_from_index_item(item: &serde_json::Value) -> Option<TagEntry> {
    let key = item.get("key")?.as_str()?.to_owned();
    let display_path = item.get("display_path")?.as_str()?.to_owned();
    let group_tag = item.get("group_tag")?.as_u64()? as u32;
    let group_name = item
        .get("group_name")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let location = if let Some(abs) = key.strip_prefix("file:") {
        TagEntryLocation::LooseFile(PathBuf::from(abs))
    } else {
        return None;
    };
    Some(TagEntry {
        key,
        display_path,
        group_tag,
        group_name,
        location,
    })
}

fn fingerprint_from_index_item(item: &serde_json::Value) -> Option<EntryFingerprint> {
    Some(EntryFingerprint {
        size: item.get("size")?.as_u64()?,
        modified_secs: item.get("modified_secs")?.as_u64()?,
        modified_nanos: item.get("modified_nanos")?.as_u64()? as u32,
    })
}

fn index_item_relative_path(
    root: &Path,
    item: &serde_json::Value,
    entry: &TagEntry,
) -> Option<PathBuf> {
    item.get("rel_path")
        .and_then(|value| value.as_str())
        .filter(|rel| !rel.is_empty())
        .map(PathBuf::from)
        .or_else(|| entry_relative_path(root, entry))
}

fn refresh_entry_index_from_cache(
    root: &Path,
    names: &TagNameIndex,
    cached_entries: &[TagEntry],
    cached_fingerprints: &HashMap<PathBuf, EntryFingerprint>,
) -> Result<EntryIndexRefresh> {
    let cached_by_rel = cached_entries
        .iter()
        .filter_map(|entry| {
            Some((
                normalize_rel_path(&entry_relative_path(root, entry)?),
                entry.clone(),
            ))
        })
        .collect::<HashMap<_, _>>();

    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    let mut added = 0;
    let mut updated = 0;
    for item in WalkDir::new(root).follow_links(false) {
        let item = item?;
        if !item.file_type().is_file() {
            continue;
        }
        let path = item.into_path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path.as_path())
            .to_path_buf();
        let rel_key = normalize_rel_path(&rel);
        seen.insert(rel_key.clone());
        let fingerprint = file_fingerprint(&path)?;

        if let (Some(cached), Some(current)) = (cached_by_rel.get(&rel_key), fingerprint.as_ref())
            && cached_fingerprints
                .get(&rel_key)
                .is_some_and(|cached_fp| cached_fp == current)
        {
            entries.push(cached.clone());
            continue;
        }

        if let Some(entry) = loose_file_entry(root, &path, names)? {
            if cached_by_rel.contains_key(&rel_key) {
                updated += 1;
            } else {
                added += 1;
            }
            entries.push(entry);
        } else if cached_by_rel.contains_key(&rel_key) {
            updated += 1;
        }
    }

    let removed = cached_by_rel
        .keys()
        .filter(|rel_path| !seen.contains(*rel_path))
        .count();
    entries.sort_by(|a, b| natural_key(&a.display_path).cmp(&natural_key(&b.display_path)));
    let changed = added > 0
        || updated > 0
        || removed > 0
        || entries.len() != cached_entries.len()
        || entries
            .iter()
            .zip(cached_entries.iter())
            .any(|(a, b)| a.key != b.key || a.group_tag != b.group_tag);

    Ok(EntryIndexRefresh {
        entries,
        changed,
        added,
        updated,
        removed,
    })
}

fn entry_file_path(entry: &TagEntry) -> Option<&Path> {
    match &entry.location {
        TagEntryLocation::LooseFile(path) => Some(path.as_path()),
        TagEntryLocation::Monolithic { .. } => None,
    }
}

fn entry_relative_path(root: &Path, entry: &TagEntry) -> Option<PathBuf> {
    entry_file_path(entry)?
        .strip_prefix(root)
        .ok()
        .map(Path::to_path_buf)
}

fn normalize_rel_path(path: &Path) -> PathBuf {
    PathBuf::from(path_to_display(path).to_ascii_lowercase())
}

fn file_fingerprint(path: &Path) -> Result<Option<EntryFingerprint>> {
    let metadata = std::fs::metadata(path)?;
    let Ok(modified) = metadata.modified() else {
        return Ok(None);
    };
    let Ok(duration) = modified.duration_since(UNIX_EPOCH) else {
        return Ok(None);
    };
    Ok(Some(EntryFingerprint {
        size: metadata.len(),
        modified_secs: duration.as_secs(),
        modified_nanos: duration.subsec_nanos(),
    }))
}

pub fn save_reverse_dependency_index(
    game: &str,
    root: &Path,
    index: &ReverseDependencyIndex,
) -> Result<()> {
    let path = reverse_dependency_index_path(game);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create config dir")?;
    }
    let tags = index
        .by_tag
        .iter()
        .map(|(key, deps)| {
            let deps = deps
                .iter()
                .map(|dep| {
                    serde_json::json!({
                        "group_tag": dep.group_tag,
                        "rel_path": dep.rel_path,
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "key": key,
                "dependencies": deps,
            })
        })
        .collect::<Vec<_>>();
    let text = serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "root": cache_root_key(root),
        "tags": tags,
    }))
    .context("serialize reverse dependency index")?;
    std::fs::write(&path, text).context("write reverse dependency index")?;
    Ok(())
}

pub fn load_reverse_dependency_index(game: &str, root: &Path) -> Option<ReverseDependencyIndex> {
    let text = std::fs::read_to_string(reverse_dependency_index_path(game)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    if value.get("version").and_then(|v| v.as_u64())? != 1 {
        return None;
    }
    let saved_root = value.get("root").and_then(|v| v.as_str())?;
    if cache_root_str_key(saved_root) != cache_root_key(root) {
        return None;
    }
    let mut index = ReverseDependencyIndex::default();
    for item in value.get("tags")?.as_array()? {
        let key = item.get("key")?.as_str()?.to_owned();
        let deps = item
            .get("dependencies")?
            .as_array()?
            .iter()
            .filter_map(|dep| {
                Some(DependencyRef {
                    group_tag: dep.get("group_tag")?.as_u64()? as u32,
                    rel_path: dep.get("rel_path")?.as_str()?.to_owned(),
                })
            })
            .collect::<Vec<_>>();
        index.set_tag_dependencies(key, deps);
    }
    Some(index)
}

pub(crate) fn dependency_key(group_tag: u32, rel_path: &str) -> String {
    format!(
        "{group_tag:08x}\t{}",
        rel_path.replace('/', "\\").to_ascii_lowercase()
    )
}

#[cfg(test)]
pub fn field_row_summaries(tag: &TagFile, names: &TagNameIndex, limit: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for field in tag.root().fields().take(limit) {
        let kind = if let Some(value) = field.value() {
            crate::format::format_value(names, &value, false)
        } else if let Some(block) = field.as_block() {
            format!("block [{} elements]", block.len())
        } else if let Some(array) = field.as_array() {
            format!("array [{} elements]", array.len())
        } else if field.as_struct().is_some() {
            "struct".to_owned()
        } else if let Some(resource) = field.as_resource() {
            format!("resource {:?}", resource.kind())
        } else {
            "container".to_owned()
        };
        rows.push(format!("{}:{}={kind}", field.name(), field.type_name()));
    }
    rows
}

pub(crate) fn resolve_folder_root(
    selected_root: &Path,
    aliases: &[EkFolderAlias],
) -> Result<FolderRootInfo> {
    let ek_root = detect_ek_root_with_aliases(selected_root, aliases);
    let game = ek_root.as_ref().map(|(_, game)| *game);
    let scan_root = if is_tags_folder(selected_root) {
        selected_root.to_path_buf()
    } else if let Some((ek_root, _)) = ek_root {
        let tags = ek_root.join("tags");
        if !tags.is_dir() {
            anyhow::bail!(
                "recognized {} as an EK root, but expected tags folder was missing: {}",
                ek_root.display(),
                tags.display()
            );
        }
        tags
    } else {
        find_tags_folder(selected_root).unwrap_or_else(|| selected_root.to_path_buf())
    };
    let label = folder_source_label(selected_root, &scan_root, game);
    Ok(FolderRootInfo {
        scan_root,
        label,
        game,
    })
}

fn find_tags_folder(selected_root: &Path) -> Option<PathBuf> {
    if is_tags_folder(selected_root) {
        return Some(selected_root.to_path_buf());
    }

    let direct = selected_root.join("tags");
    if direct.is_dir() {
        return Some(direct);
    }

    WalkDir::new(selected_root)
        .min_depth(1)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| {
            entry.file_type().is_dir()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case("tags"))
        })
        .map(|entry| entry.into_path())
}

fn is_tags_folder(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("tags"))
}

#[cfg(test)]
fn detect_ek_game(path: &Path) -> Option<&'static str> {
    detect_ek_root_with_aliases(path, &[]).map(|(_, game)| game)
}

fn detect_ek_root_with_aliases(
    path: &Path,
    aliases: &[EkFolderAlias],
) -> Option<(PathBuf, &'static str)> {
    let built_in = path
        .ancestors()
        .filter_map(|ancestor| {
            ancestor
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| ek_folder_game(name).map(|game| (ancestor.to_path_buf(), game)))
        })
        .next();
    if built_in.is_some() {
        return built_in;
    }

    path.ancestors()
        .filter_map(|ancestor| {
            let name = ancestor.file_name().and_then(|name| name.to_str())?;
            let game = alias_folder_game(name, aliases)?;
            Some((ancestor.to_path_buf(), game))
        })
        .next()
}

fn ek_folder_game(name: &str) -> Option<&'static str> {
    // Recognize both the editing-kit folder names (e.g. `H3EK`) and the
    // canonical game-id folder names (e.g. `halo3_mcc`) — users often keep tags
    // under a folder named after the game, not the EK.
    match name.to_ascii_uppercase().as_str() {
        "HCEEK" | "H1EK" | "HALOCEEK" | "HALOCE_MCC" => Some("haloce_mcc"),
        "H2EK" | "HALO2EK" | "HALO2_MCC" => Some("halo2_mcc"),
        "HREK" | "HALOREACH_MCC" => Some("haloreach_mcc"),
        "H4EK" | "HALO4_MCC" => Some("halo4_mcc"),
        "H3ODSTEK" | "HALO3ODST_MCC" => Some("halo3odst_mcc"),
        "H3EK" | "HALO3_MCC" => Some("halo3_mcc"),
        "H2AMPEK" | "H2AEK" | "HALO2AMP_MCC" => Some("halo2amp_mcc"),
        _ => None,
    }
}

fn alias_folder_game(name: &str, aliases: &[EkFolderAlias]) -> Option<&'static str> {
    aliases.iter().rev().find_map(|alias| {
        let folder_name = alias.folder_name.trim();
        if folder_name.is_empty() || !folder_name.eq_ignore_ascii_case(name) {
            return None;
        }
        supported_ek_game_id(&alias.game)
    })
}

pub(crate) fn supported_ek_game_id(game: &str) -> Option<&'static str> {
    SUPPORTED_EK_GAMES
        .iter()
        .find_map(|(_, id)| id.eq_ignore_ascii_case(game).then_some(*id))
}

fn folder_source_label(
    selected_root: &Path,
    scan_root: &Path,
    game: Option<&'static str>,
) -> String {
    let selected_label = selected_root
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| selected_root.display().to_string());
    let mut label = if scan_root != selected_root {
        format!("{selected_label}/tags")
    } else {
        selected_label
    };
    if let Some(game) = game {
        label.push_str(&format!(" ({game})"));
    }
    label
}

#[cfg(test)]
fn scan_folder_entries(root: &Path, names: &TagNameIndex) -> Result<Vec<TagEntry>> {
    let mut entries = Vec::new();
    for item in WalkDir::new(root).follow_links(false) {
        let item = item?;
        if !item.file_type().is_file() {
            continue;
        }
        let path = item.into_path();
        let Some(group_tag) = probe_tag_group(&path)? else {
            continue;
        };
        let rel = path.strip_prefix(root).unwrap_or(path.as_path());
        let group_name = names.name_for(group_tag).map(str::to_owned);
        let display_path = display_path_with_friendly_extension(rel, group_tag, names);
        entries.push(TagEntry {
            key: format!("file:{}", path.display()),
            display_path,
            group_tag,
            group_name,
            location: TagEntryLocation::LooseFile(path),
        });
    }
    Ok(entries)
}

pub(crate) fn build_folder_directory_tree(root: &Path) -> Result<TagTree> {
    let mut tree = TagTree::default();
    tree.entries = Vec::new();
    tree.children = list_direct_child_nodes(root, Path::new(""))?;
    Ok(tree)
}

fn list_direct_child_nodes(root: &Path, rel_path: &Path) -> Result<Vec<TagTreeNode>> {
    let folder = root.join(&rel_path);
    let mut children = Vec::new();
    for item in std::fs::read_dir(&folder)
        .with_context(|| format!("failed to read {}", folder.display()))?
    {
        let item = item?;
        let file_type = item.file_type()?;
        if !file_type.is_dir() {
            continue;
        }
        let label = item.file_name().to_string_lossy().into_owned();
        children.push(build_folder_node(rel_path.join(label)));
    }
    children.sort_by(|a, b| natural_key(&a.label).cmp(&natural_key(&b.label)));
    Ok(children)
}

fn build_folder_node(rel_path: PathBuf) -> TagTreeNode {
    let label = rel_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    TagTreeNode {
        label,
        rel_path,
        children: Vec::new(),
        children_loaded: false,
        entries: Vec::new(),
        entries_loaded: false,
    }
}

fn scan_folder_direct_entries(
    root: &Path,
    folder: &Path,
    names: &TagNameIndex,
) -> Result<Vec<TagEntry>> {
    let mut entries = Vec::new();
    for item in
        std::fs::read_dir(folder).with_context(|| format!("failed to read {}", folder.display()))?
    {
        let item = item?;
        if !item.file_type()?.is_file() {
            continue;
        }
        let path = item.path();
        let Some(group_tag) = probe_tag_group(&path)? else {
            continue;
        };
        let rel = path.strip_prefix(root).unwrap_or(path.as_path());
        let group_name = names.name_for(group_tag).map(str::to_owned);
        let display_path = display_path_with_friendly_extension(rel, group_tag, names);
        entries.push(TagEntry {
            key: format!("file:{}", path.display()),
            display_path,
            group_tag,
            group_name,
            location: TagEntryLocation::LooseFile(path),
        });
    }
    Ok(entries)
}

fn probe_tag_group(path: &Path) -> Result<Option<u32>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len < 64 {
        return Ok(None);
    }

    let mut header = [0u8; 64];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut header)?;
    if let Some((classic, _)) = ClassicHeader::parse(&header) {
        return Ok(Some(u32::from_be_bytes(classic.group_tag)));
    }
    match &header[60..64] {
        b"MALB" => Ok(Some(u32::from_le_bytes([
            header[48], header[49], header[50], header[51],
        ]))),
        b"BLAM" => Ok(Some(u32::from_be_bytes([
            header[48], header[49], header[50], header[51],
        ]))),
        _ => Ok(None),
    }
}

fn finish_node(label: String, node: TreeBuildNode) -> TagTreeNode {
    TagTreeNode {
        label,
        children: node
            .children
            .into_iter()
            .map(|(label, node)| finish_node(label, node))
            .collect(),
        entries: node.entries,
        ..Default::default()
    }
}

fn split_display_path(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn path_to_display(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn display_path_with_friendly_extension(
    path: &Path,
    group_tag: u32,
    names: &TagNameIndex,
) -> String {
    let display = path_to_display(path);
    display_str_with_friendly_extension(&display, group_tag, names)
}

fn display_str_with_friendly_extension(
    display: &str,
    group_tag: u32,
    names: &TagNameIndex,
) -> String {
    let extension = friendly_extension(group_tag, names);
    match display.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => format!("{stem}.{extension}"),
        _ => format!("{display}.{extension}"),
    }
}

fn friendly_extension(group_tag: u32, names: &TagNameIndex) -> String {
    names
        .name_for(group_tag)
        .or_else(|| gui_group_tag_to_extension(group_tag))
        .or_else(|| group_tag_to_extension(group_tag))
        .map(str::to_owned)
        .unwrap_or_else(|| format_group_tag(group_tag))
}

fn gui_group_tag_to_extension(group_tag: u32) -> Option<&'static str> {
    Some(match format_group_tag(group_tag).trim_end() {
        "mat" => "material",
        "mats" => "material_shader",
        "mtsb" => "material_shader_bank",
        "hlmt" => "model",
        "mode" => "render_model",
        "coll" => "collision_model",
        "phmo" => "physics_model",
        "jmad" => "model_animation_graph",
        "bipd" => "biped",
        "vehi" => "vehicle",
        "weap" => "weapon",
        "scen" => "scenery",
        "crat" => "crate",
        "mach" => "device_machine",
        "bloc" => "device_control",
        "bitm" => "bitmap",
        "sbsp" => "scenario_structure_bsp",
        "scnr" => "scenario",
        "impo" => "imposter_model",
        "frms" => "frame_event_list",
        "effe" => "effect",
        "snd!" => "sound",
        "rmsh" => "shader",
        "rmtr" => "shader_terrain",
        "rmw" => "shader_water",
        "rmfl" => "shader_foliage",
        "rmd" => "shader_decal",
        "rmhg" => "shader_halogram",
        "rmsk" => "shader_skin",
        "rmct" => "shader_cortana",
        "rmcs" => "shader_custom",
        _ => return None,
    })
}

fn natural_key(value: &str) -> String {
    value.to_ascii_lowercase().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn detect_game_from_game_id_folder_name() {
        // A folder named after the game (not the EK) must still resolve, so the
        // definitions/per-game features (incl. the doc overlay) work.
        assert_eq!(
            detect_ek_game(Path::new("/Users/x/Halo/halo3_mcc/tags/objects")),
            Some("halo3_mcc")
        );
        assert_eq!(
            detect_ek_game(Path::new("/data/haloreach_mcc/tags")),
            Some("haloreach_mcc")
        );
        // EK-style names still work.
        assert_eq!(detect_ek_game(Path::new("/x/H3EK/tags")), Some("halo3_mcc"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("blam_tag_gui_{name}_{stamp}"))
    }

    fn write_fake_tag(path: &Path, group: &[u8; 4]) {
        let mut bytes = [0u8; 64];
        let group_tag = u32::from_be_bytes(*group);
        bytes[48..52].copy_from_slice(&group_tag.to_le_bytes());
        bytes[60..64].copy_from_slice(b"MALB");
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn normalizes_blob_index_to_parent_cache_root() {
        let root = PathBuf::from(r"C:\tags\tag_cache");
        let blob = root.join("blob_index.dat");
        assert_eq!(normalize_blob_index_path(&blob).unwrap(), root);
        assert!(normalize_blob_index_path(&root.join("tag_blob.dat")).is_err());
    }

    #[test]
    fn scans_loose_folder_with_header_probe() {
        let root = temp_dir("scan");
        fs::create_dir_all(root.join("objects/characters")).unwrap();
        write_fake_tag(&root.join("objects/characters/test.biped"), b"bipd");
        fs::write(root.join("not_a_tag.txt"), b"hello").unwrap();

        let index = TagNameIndex::default();
        let entries = scan_folder_entries(&root, &index).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].display_path, "objects/characters/test.biped");
        assert_eq!(format_group_tag(entries[0].group_tag), "bipd");
    }

    #[test]
    fn loose_file_entry_matches_scanned_folder_entry_metadata() {
        let root = temp_dir("drop_entry");
        fs::create_dir_all(root.join("objects/characters/brute")).unwrap();
        let path = root
            .join("objects")
            .join("characters")
            .join("brute")
            .join("brute.shader");
        write_fake_tag(&path, b"shdr");

        let names = TagNameIndex::default();
        let entry = loose_file_entry(&root, &path, &names)
            .unwrap()
            .expect("fake tag should probe as a tag");
        let scanned = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(scanned.len(), 1);
        assert_eq!(entry.key, scanned[0].key);
        assert_eq!(entry.display_path, "objects/characters/brute/brute.shader");
        assert_eq!(entry.display_path, scanned[0].display_path);
        assert_eq!(entry.group_tag, scanned[0].group_tag);
    }

    fn unique_game(name: &str) -> String {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{name}_{stamp}")
    }

    fn remove_test_index(game: &str) {
        let _ = fs::remove_file(index_path(game));
        let _ = fs::remove_file(reverse_dependency_index_path(game));
    }

    fn write_fake_tag_with_padding(path: &Path, group: &[u8; 4], padding: usize) {
        let mut bytes = vec![0u8; 64 + padding];
        let group_tag = u32::from_be_bytes(*group);
        bytes[48..52].copy_from_slice(&group_tag.to_le_bytes());
        bytes[60..64].copy_from_slice(b"MALB");
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn entry_index_refresh_reuses_unchanged_metadata() {
        let root = temp_dir("index_refresh_unchanged");
        let game = unique_game("index_refresh_unchanged");
        fs::create_dir_all(root.join("objects")).unwrap();
        write_fake_tag(&root.join("objects/a.model"), b"hlmt");
        write_fake_tag(&root.join("objects/b.shader"), b"shdr");
        let names = TagNameIndex::default();
        let entries = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        save_entry_index(&game, &root, &entries).unwrap();

        let refresh = refresh_entry_index(&game, &root, &names).unwrap();

        remove_test_index(&game);
        fs::remove_dir_all(&root).unwrap();

        assert!(!refresh.changed);
        assert_eq!(refresh.added, 0);
        assert_eq!(refresh.updated, 0);
        assert_eq!(refresh.removed, 0);
        assert_eq!(refresh.entries.len(), 2);
    }

    #[test]
    fn entry_index_refresh_reprobes_changed_files_and_drops_deleted_files() {
        let root = temp_dir("index_refresh_changed");
        let game = unique_game("index_refresh_changed");
        fs::create_dir_all(root.join("objects")).unwrap();
        let changed = root.join("objects/a.model");
        let removed = root.join("objects/b.shader");
        let added = root.join("objects/c.biped");
        write_fake_tag(&changed, b"hlmt");
        write_fake_tag(&removed, b"shdr");
        let names = TagNameIndex::default();
        let entries = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        save_entry_index(&game, &root, &entries).unwrap();

        write_fake_tag_with_padding(&changed, b"bipd", 1);
        fs::remove_file(&removed).unwrap();
        write_fake_tag(&added, b"bipd");
        let refresh = refresh_entry_index(&game, &root, &names).unwrap();

        remove_test_index(&game);
        fs::remove_dir_all(&root).unwrap();

        assert!(refresh.changed);
        assert_eq!(refresh.added, 1);
        assert_eq!(refresh.updated, 1);
        assert_eq!(refresh.removed, 1);
        let paths = refresh
            .entries
            .iter()
            .map(|entry| entry.display_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["objects/a.biped", "objects/c.biped"]);
    }

    #[test]
    fn load_entry_index_accepts_legacy_cache_without_metadata() {
        let root = temp_dir("legacy_index");
        let game = unique_game("legacy_index");
        fs::create_dir_all(root.join("objects")).unwrap();
        let path = root.join("objects/a.model");
        write_fake_tag(&path, b"hlmt");
        let text = serde_json::to_string(&serde_json::json!({
            "root": root.display().to_string(),
            "entries": [{
                "key": format!("file:{}", path.display()),
                "display_path": "objects/a.model",
                "group_tag": u32::from_be_bytes(*b"hlmt"),
                "group_name": null
            }]
        }))
        .unwrap();
        if let Some(parent) = index_path(&game).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(index_path(&game), text).unwrap();

        let entries = load_entry_index(&game, &root).unwrap();
        let refresh = refresh_entry_index(&game, &root, &TagNameIndex::default()).unwrap();

        remove_test_index(&game);
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(entries.len(), 1);
        assert!(refresh.changed);
        assert_eq!(refresh.updated, 1);
        assert_eq!(refresh.entries.len(), 1);
    }

    #[test]
    fn probes_classic_h2_group_from_reversed_header() {
        let root = temp_dir("classic_h2_probe");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("brute.mode");
        let mut bytes = [0u8; 64];
        bytes[36..40].copy_from_slice(b"edom");
        bytes[60..64].copy_from_slice(b"!MLB");
        fs::write(&path, bytes).unwrap();

        assert_eq!(
            probe_tag_group(&path).unwrap(),
            Some(u32::from_be_bytes(*b"mode"))
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn probes_classic_ce_group_from_big_endian_header() {
        let root = temp_dir("classic_ce_probe");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("cyborg.gbxmodel");
        let mut bytes = [0u8; 64];
        bytes[36..40].copy_from_slice(b"mod2");
        bytes[60..64].copy_from_slice(b"blam");
        fs::write(&path, bytes).unwrap();

        assert_eq!(
            probe_tag_group(&path).unwrap(),
            Some(u32::from_be_bytes(*b"mod2"))
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn load_folder_descends_into_ek_tags_root() {
        let root = temp_dir("h4ek");
        let ek_root = root.join("H4EK");
        fs::create_dir_all(ek_root.join("tags/objects/vehicles")).unwrap();
        fs::create_dir_all(ek_root.join("data/objects/vehicles")).unwrap();
        write_fake_tag(
            &ek_root.join("tags/objects/vehicles/warthog.model"),
            b"hlmt",
        );
        write_fake_tag(
            &ek_root.join("data/objects/vehicles/not_in_tags.model"),
            b"hlmt",
        );

        let loaded = load_folder(
            ek_root.clone(),
            &TagNameIndex::default(),
            &root.join("definitions"),
            &[],
        )
        .unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(loaded.label, "H4EK/tags (halo4_mcc)");
        assert!(loaded.entries.is_empty());
        assert_eq!(loaded.tree.children[0].label, "objects");
        assert_eq!(loaded.tree.children[0].rel_path, PathBuf::from("objects"));
        assert!(loaded.tree.children[0].children.is_empty());
        assert!(!loaded.tree.children[0].children_loaded);
        assert!(!loaded.tree.children[0].entries_loaded);
        match loaded.source {
            TagSource::LooseFolder { root, .. } => assert!(root.ends_with("tags")),
            _ => panic!("expected loose folder source"),
        }
    }

    #[test]
    fn lazy_folder_node_loads_only_direct_tag_files() {
        let root = temp_dir("lazy_node");
        fs::create_dir_all(root.join("objects/vehicles/child")).unwrap();
        write_fake_tag(&root.join("objects/vehicles/warthog.model"), b"hlmt");
        write_fake_tag(&root.join("objects/vehicles/child/child.model"), b"hlmt");

        let mut tree = build_folder_directory_tree(&root).unwrap();
        let mut entries = Vec::new();
        let objects = &mut tree.children[0];
        load_folder_node_entries(&root, objects, &mut entries, &TagNameIndex::default()).unwrap();
        let vehicles = &mut objects.children[0];
        load_folder_node_entries(&root, vehicles, &mut entries, &TagNameIndex::default()).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].display_path, "objects/vehicles/warthog.model");
        assert!(vehicles.entries_loaded);
    }

    #[test]
    fn subtree_scan_finds_nested_tag_files_for_folder_export() {
        let root = temp_dir("subtree_export");
        fs::create_dir_all(root.join("objects/vehicles/child")).unwrap();
        fs::create_dir_all(root.join("objects/characters")).unwrap();
        write_fake_tag(&root.join("objects/vehicles/warthog.model"), b"hlmt");
        write_fake_tag(&root.join("objects/vehicles/child/child.model"), b"hlmt");
        write_fake_tag(&root.join("objects/characters/spartan.model"), b"hlmt");

        let entries = scan_folder_subtree_entries(
            &root,
            Path::new("objects/vehicles"),
            &TagNameIndex::default(),
        )
        .unwrap();
        fs::remove_dir_all(&root).unwrap();

        let display_paths = entries
            .into_iter()
            .map(|entry| entry.display_path)
            .collect::<Vec<_>>();
        assert_eq!(
            display_paths,
            vec![
                "objects/vehicles/child/child.model",
                "objects/vehicles/warthog.model",
            ]
        );
    }

    #[test]
    fn ek_root_without_tags_folder_errors_without_deep_search() {
        let root = temp_dir("missing_tags");
        let ek_root = root.join("HREK");
        fs::create_dir_all(ek_root.join("data/tags")).unwrap();

        let error = resolve_folder_root(&ek_root, &[]).unwrap_err().to_string();
        fs::remove_dir_all(&root).unwrap();

        assert!(error.contains("expected tags folder was missing"));
        assert!(error.contains("HREK"));
    }

    #[test]
    fn detects_supported_ek_games_from_root_or_tags_folder() {
        assert_eq!(detect_ek_game(&PathBuf::from("HCEEK")), Some("haloce_mcc"));
        assert_eq!(
            detect_ek_game(&PathBuf::from("H1EK").join("tags")),
            Some("haloce_mcc")
        );
        assert_eq!(
            detect_ek_game(&PathBuf::from("H2EK").join("tags")),
            Some("halo2_mcc")
        );
        assert_eq!(
            detect_ek_game(&PathBuf::from("HREK")),
            Some("haloreach_mcc")
        );
        assert_eq!(
            detect_ek_game(&PathBuf::from("H4EK").join("tags")),
            Some("halo4_mcc")
        );
        assert_eq!(
            detect_ek_game(&PathBuf::from("H3ODSTEK").join("tags")),
            Some("halo3odst_mcc")
        );
        assert_eq!(
            detect_ek_game(&PathBuf::from("H3EK").join("tags")),
            Some("halo3_mcc")
        );
    }

    #[test]
    fn custom_ek_alias_detects_root_folder() {
        let root = temp_dir("custom_ek_alias_root");
        let ek_root = root.join("h2rek");
        fs::create_dir_all(ek_root.join("tags/objects")).unwrap();
        let aliases = vec![EkFolderAlias {
            folder_name: "h2rek".to_owned(),
            game: "halo2_mcc".to_owned(),
        }];

        let info = resolve_folder_root(&ek_root, &aliases).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(info.game, Some("halo2_mcc"));
        assert!(info.scan_root.ends_with("tags"));
        assert_eq!(info.label, "h2rek/tags (halo2_mcc)");
    }

    #[test]
    fn custom_ek_alias_detects_tags_folder() {
        let root = temp_dir("custom_ek_alias_tags");
        let tags_root = root.join("h2rek").join("tags");
        fs::create_dir_all(tags_root.join("objects")).unwrap();
        let aliases = vec![EkFolderAlias {
            folder_name: "h2rek".to_owned(),
            game: "halo2_mcc".to_owned(),
        }];

        let info = resolve_folder_root(&tags_root, &aliases).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(info.game, Some("halo2_mcc"));
        assert!(info.scan_root.ends_with("tags"));
        assert_eq!(info.label, "tags (halo2_mcc)");
    }

    #[test]
    fn built_in_ek_name_takes_precedence_over_alias() {
        let path = PathBuf::from("H2EK").join("tags");
        let aliases = vec![EkFolderAlias {
            folder_name: "H2EK".to_owned(),
            game: "halo3_mcc".to_owned(),
        }];

        let detected = detect_ek_root_with_aliases(&path, &aliases).map(|(_, game)| game);

        assert_eq!(detected, Some("halo2_mcc"));
    }

    #[test]
    fn rewrites_short_cache_suffixes_to_foundation_names() {
        let names = TagNameIndex::default();
        let cases = [
            (
                b"bipd",
                "objects/characters/spartans/spartans.bipd",
                "objects/characters/spartans/spartans.biped",
            ),
            (
                b"coll",
                "objects/characters/spartans/spartans.coll",
                "objects/characters/spartans/spartans.collision_model",
            ),
            (
                b"phmo",
                "objects/characters/spartans/spartans.phmo",
                "objects/characters/spartans/spartans.physics_model",
            ),
            (
                b"jmad",
                "objects/characters/spartans/spartans.jmad",
                "objects/characters/spartans/spartans.model_animation_graph",
            ),
            (
                b"impo",
                "objects/characters/spartans/spartans.impo",
                "objects/characters/spartans/spartans.imposter_model",
            ),
            (
                b"frms",
                "objects/characters/spartans/spartans.frms",
                "objects/characters/spartans/spartans.frame_event_list",
            ),
        ];

        for (group, input, expected) in cases {
            let group_tag = u32::from_be_bytes(*group);
            assert_eq!(
                display_str_with_friendly_extension(input, group_tag, &names),
                expected
            );
        }
    }

    #[test]
    fn builds_hierarchical_tree_from_display_paths() {
        let entries = vec![
            TagEntry {
                key: "a".into(),
                display_path: "objects/test/a.biped".into(),
                group_tag: u32::from_be_bytes(*b"bipd"),
                group_name: None,
                location: TagEntryLocation::LooseFile(PathBuf::from("a")),
            },
            TagEntry {
                key: "b".into(),
                display_path: "objects/test/b.model".into(),
                group_tag: u32::from_be_bytes(*b"hlmt"),
                group_name: None,
                location: TagEntryLocation::LooseFile(PathBuf::from("b")),
            },
        ];
        let tree = build_tree(&entries);
        assert_eq!(tree.children[0].label, "objects");
        assert_eq!(tree.children[0].children[0].label, "test");
        assert_eq!(tree.children[0].children[0].entries, vec![0, 1]);
    }

    #[test]
    fn builds_group_tree_from_entries() {
        let entries = vec![
            TagEntry {
                key: "a".into(),
                display_path: "objects/test/a.biped".into(),
                group_tag: u32::from_be_bytes(*b"bipd"),
                group_name: Some("biped".into()),
                location: TagEntryLocation::LooseFile(PathBuf::from("a")),
            },
            TagEntry {
                key: "b".into(),
                display_path: "objects/test/b2.biped".into(),
                group_tag: u32::from_be_bytes(*b"bipd"),
                group_name: Some("biped".into()),
                location: TagEntryLocation::LooseFile(PathBuf::from("b")),
            },
            TagEntry {
                key: "c".into(),
                display_path: "objects/test/c.render_model".into(),
                group_tag: u32::from_be_bytes(*b"mode"),
                group_name: Some("render_model".into()),
                location: TagEntryLocation::LooseFile(PathBuf::from("c")),
            },
        ];
        let tree = build_group_tree(&entries);
        assert_eq!(tree.children.len(), 2);
        assert_eq!(tree.children[0].label, "biped bipd");
        assert_eq!(tree.children[0].entries, vec![0, 1]);
    }

    #[test]
    fn group_tree_uses_friendly_fallback_when_name_is_fourcc() {
        let entries = vec![TagEntry {
            key: "a".into(),
            display_path: "objects/test/a.weapon".into(),
            group_tag: u32::from_be_bytes(*b"weap"),
            group_name: Some("weap".into()),
            location: TagEntryLocation::LooseFile(PathBuf::from("a")),
        }];

        let tree = build_group_tree(&entries);

        assert_eq!(tree.children[0].label, "weapon weap");
    }

    #[test]
    fn builds_field_summaries_from_fixture_when_present() {
        let fixture = PathBuf::from("dump/storm_knight/storm_knight.biped");
        if !fixture.exists() {
            return;
        }
        let tag = TagFile::read(&fixture).unwrap();
        let rows = field_row_summaries(&tag, &TagNameIndex::default(), 24);
        assert!(!rows.is_empty());
        assert!(
            rows.iter()
                .any(|r| r.contains("block") || r.contains("struct"))
        );
    }
}
