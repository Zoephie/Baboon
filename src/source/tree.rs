//! Tag trees, folder scanning, and entry discovery.
//! It owns source identity, discovery, indexing, and source-aware reads; editor presentation and application workflow state belong elsewhere.

use super::*;

/// Builds a path hierarchy whose stored indices address `entries` exactly.
pub fn build_tree(entries: &[TagEntry]) -> TagTree {
    build_tree_with_folders(entries, &[])
}

/// Build a folder tree rooted beneath `folder` while keeping indices pointed
/// at the original, full-path entries. Node paths remain source-relative so
/// opening a nested folder or invoking a folder action needs no rebasing.
pub fn build_tree_beneath(entries: &[TagEntry], folder: &Path) -> TagTree {
    let prefix = split_display_path(&path_to_display(folder));
    let mut root = TreeBuildNode::default();
    for (index, entry) in entries.iter().enumerate() {
        let parts = split_display_path(&entry.display_path);
        if !display_path_is_beneath(&parts, &prefix) {
            continue;
        }
        let beneath = &parts[prefix.len()..];
        if beneath.len() == 1 {
            root.entries.push(index);
            continue;
        }
        let mut node = &mut root;
        for part in &beneath[..beneath.len() - 1] {
            node = node.children.entry(part.clone()).or_default();
        }
        node.entries.push(index);
    }
    let parent = prefix.join("/");
    TagTree {
        children: root
            .children
            .into_iter()
            .map(|(label, node)| finish_node(label, node, &parent))
            .collect(),
        entries: root.entries,
    }
}

/// Build the initially visible portion of a loose folder tab without scanning
/// its descendants. Direct tags are indexed now; each child remains lazy and
/// is materialized only when the user expands it.
pub fn build_lazy_folder_tree_beneath(
    root: &Path,
    folder: &Path,
    entries: &mut Vec<TagEntry>,
    names: &TagNameIndex,
) -> Result<TagTree> {
    let children = list_direct_child_nodes(root, folder)?;
    let mut direct_entries = scan_folder_direct_entries(root, &root.join(folder), names)?;
    direct_entries.sort_by(|a, b| natural_key(&a.display_path).cmp(&natural_key(&b.display_path)));

    let mut indices = Vec::with_capacity(direct_entries.len());
    for entry in direct_entries {
        if let Some(index) = entries.iter().position(|known| known.key == entry.key) {
            indices.push(index);
        } else {
            indices.push(entries.len());
            entries.push(entry);
        }
    }
    Ok(TagTree {
        children,
        entries: indices,
    })
}

/// Build a group tree containing only entries beneath `folder`, while keeping
/// every stored index pointed at the original `entries` slice.
pub fn build_group_tree_beneath(entries: &[TagEntry], folder: &Path) -> TagTree {
    let prefix = split_display_path(&path_to_display(folder));
    build_group_tree_from_indices(
        entries,
        entries.iter().enumerate().filter_map(|(index, entry)| {
            display_path_is_beneath(&split_display_path(&entry.display_path), &prefix)
                .then_some(index)
        }),
    )
}

/// Whether an entry lives below `folder` rather than being the folder itself.
/// Paths are source-relative and compared case-insensitively to match Windows
/// editing-kit behavior.
pub fn entry_is_beneath_folder(entry: &TagEntry, folder: &Path) -> bool {
    let prefix = split_display_path(&path_to_display(folder));
    display_path_is_beneath(&split_display_path(&entry.display_path), &prefix)
}

fn display_path_is_beneath(parts: &[String], prefix: &[String]) -> bool {
    parts.len() > prefix.len()
        && parts[..prefix.len()]
            .iter()
            .zip(prefix)
            .all(|(part, expected)| part.eq_ignore_ascii_case(expected))
}

/// [`build_tree`], plus folders that exist only because the user asked for them.
///
/// A container folder has no independent existence on either side: this tree is
/// derived entirely from `display_path`, and a pak's directory index can only
/// encode a directory that has a file beneath it. So a folder made to organise
/// work into is carried here until the first tag lands in it, at which point the
/// container's own index starts expressing it and the seed becomes redundant
/// (re-seeding it is a no-op, so the row is kept rather than pruned).
///
/// `extra_folders` are `/`-separated paths matching `display_path` casing.
pub fn build_tree_with_folders(entries: &[TagEntry], extra_folders: &[String]) -> TagTree {
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

    // Seeded after the entries so `pending` marks only the nodes no tag reached.
    // A folder that already exists keeps its derived identity untouched.
    for folder in extra_folders {
        let mut node = &mut root;
        for part in split_display_path(folder) {
            let fresh = !node.children.contains_key(&part);
            node = node.children.entry(part).or_default();
            node.pending |= fresh;
        }
    }

    TagTree {
        children: root
            .children
            .into_iter()
            .map(|(label, node)| finish_node(label, node, ""))
            .collect(),
        entries: root.entries,
    }
}

/// Rebuilds a mounted source's folder tree, re-applying the workspace's pending
/// folders.
///
/// Every site that reassigns `source.tree` for a *live* kit must go through
/// this. A bare [`build_tree`] there is not wrong so much as forgetful: it
/// silently drops every folder the user made and has not filled yet, and it does
/// so on unrelated events like a delete or a duplicate.
pub fn rebuild_folder_tree(source: &mut LoadedSourceData, pending_folders: &[String]) {
    source.tree = build_tree_with_folders(&source.entries, pending_folders);
}

/// Groups entries by friendly tag group while preserving entry-vector indices.
pub fn build_group_tree(entries: &[TagEntry]) -> TagTree {
    build_group_tree_from_indices(entries, 0..entries.len())
}

fn build_group_tree_from_indices(
    entries: &[TagEntry],
    indices: impl IntoIterator<Item = usize>,
) -> TagTree {
    let mut root = TreeBuildNode::default();
    for index in indices {
        let entry = &entries[index];
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
            .map(|(label, node)| finish_node(label, node, ""))
            .collect(),
        entries: root.entries,
    }
}

fn friendly_group_name(group_tag: u32, indexed_name: Option<&str>, fourcc: &str) -> String {
    match indexed_name {
        Some(name) if !name.eq_ignore_ascii_case(fourcc) => return name.to_owned(),
        _ => {}
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

/// Materializes one lazy folder node exactly once and appends its direct tags.
/// Existing entry indices remain valid because new entries are append-only.
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
    let mut found = scan_folder_direct_entries(root, &folder, names)?;
    found.sort_by_cached_key(|entry| natural_key(&entry.display_path));
    // A tag already in the list (loaded before the tree was rebuilt, or added
    // by a save or a new tag) keeps its slot. Appending it again, as this
    // used to, left the same key in the list twice after every Save As or New
    // Tag followed by re-expanding its folder.
    let known: HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.key.as_str(), index))
        .collect();
    let mut indices = Vec::with_capacity(found.len());
    let mut appended = Vec::new();
    for entry in found {
        match known.get(entry.key.as_str()) {
            Some(&index) => indices.push(index),
            None => {
                indices.push(entries.len() + appended.len());
                appended.push(entry);
            }
        }
    }
    drop(known);
    entries.extend(appended);
    node.entries.extend(indices);
    node.entries_loaded = true;
    Ok(())
}

/// Whether a per-file error means the file went away or cannot be opened,
/// which a scan skips, rather than something wrong with the scan itself.
///
/// A temp file deleted between the walk and the read, or a tag locked by
/// tool.exe, used to fail the whole scan (or the 30-second refresh) over one
/// file.
pub(crate) fn skippable_file_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| skippable_io_error(io.kind()))
    })
}

fn skippable_io_error(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
    )
}

/// A walk entry, or `None` for one below the root that vanished or cannot be
/// read. An error on the root itself still fails: an unreadable tags folder is
/// not an empty one.
pub(crate) fn walk_item(
    item: std::result::Result<walkdir::DirEntry, walkdir::Error>,
) -> Result<Option<walkdir::DirEntry>> {
    match item {
        Ok(item) => Ok(Some(item)),
        Err(error)
            if error.depth() > 0
                && error
                    .io_error()
                    .is_some_and(|io| skippable_io_error(io.kind())) =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

/// Recursively scans one source-relative subtree without progress reporting.
pub fn scan_folder_subtree_entries(
    root: &Path,
    rel_path: &Path,
    names: &TagNameIndex,
) -> Result<Vec<TagEntry>> {
    scan_folder_subtree_entries_with_progress(root, rel_path, names, |_| {})
}

/// Recursively scans one source-relative subtree and reports monotonic counts.
/// Symlinks are not followed, preventing scans from escaping or cycling beneath
/// the selected tags root.
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
        let Some(item) = walk_item(item)? else {
            continue;
        };
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
                    let probed = match probe_tag_group(path) {
                        Ok(probed) => probed,
                        Err(error) if skippable_file_error(&error) => None,
                        Err(error) => return Err(error),
                    };
                    if let Some(group_tag) = probed {
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

/// Probes one loose file and returns its stable source entry when it is a tag.
/// Group detection is source-aware and must not be replaced by extension alone.
/// `path` spelled on `root` as the source holds it, or `None` when it is not
/// under `root`.
///
/// A loose entry's key is `file:` plus its path, and the folder scan builds
/// those paths by joining names onto the root it was given. Canonicalizing is
/// the right way to decide whether a path is inside the root, but not a way to
/// spell it: on Windows it adds `\\?\`, on macOS it resolves `/var` to
/// `/private/var`, and an entry built from that form gets a key the scan never
/// produces, so the same tag is listed twice under two keys.
pub fn path_on_root(root: &Path, path: &Path) -> std::io::Result<Option<PathBuf>> {
    let canonical_root = std::fs::canonicalize(root)?;
    let canonical = std::fs::canonicalize(path)?;
    Ok(canonical
        .strip_prefix(&canonical_root)
        .ok()
        .map(|relative| root.join(relative)))
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

/// Legacy JSON index path. New saves use [`index_db_path`], but this remains
/// readable so existing AppData caches can be migrated.

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
        // Loose folders are real directories on disk, so none of them is pending.
        pending: false,
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

fn finish_node(label: String, node: TreeBuildNode, parent: &str) -> TagTreeNode {
    let rel_path = if parent.is_empty() {
        label.clone()
    } else {
        format!("{parent}/{label}")
    };
    let children = node
        .children
        .into_iter()
        .map(|(child_label, child)| finish_node(child_label, child, &rel_path))
        .collect();
    TagTreeNode {
        label,
        rel_path: PathBuf::from(&rel_path),
        children,
        entries: node.entries,
        pending: node.pending,
        ..Default::default()
    }
}

fn split_display_path(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn path_to_display(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub(super) fn display_path_with_friendly_extension(
    path: &Path,
    group_tag: u32,
    names: &TagNameIndex,
) -> String {
    let display = path_to_display(path);
    display_str_with_friendly_extension(&display, group_tag, names)
}

pub(super) fn display_str_with_friendly_extension(
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

pub fn natural_key(value: &str) -> String {
    value.to_ascii_lowercase().replace('\\', "/")
}

/// Insert `entry` into a list already ordered by [`natural_key`], returning the
/// position it landed at.
///
/// Every source sorts its entries this way once, at mount, and `build_tree`
/// stores positional indices — so a folder is drawn in entry-vector order under
/// `BrowserSort::Natural`. An entry that is pushed rather than inserted lands at
/// the bottom of its folder instead of beside its neighbours, which reads as
/// "the new tag never appeared". A list that is not sorted still gets a valid
/// insertion; it simply has no ordering to preserve.
pub fn insert_entry_sorted(entries: &mut Vec<TagEntry>, entry: TagEntry) -> usize {
    let key = natural_key(&entry.display_path);
    let position = entries.partition_point(|existing| natural_key(&existing.display_path) <= key);
    entries.insert(position, entry);
    position
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
        if let Ok(conn) = open_index_db() {
            let _ = conn.execute("DELETE FROM sources WHERE game = ?1", params![game]);
        }
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

    /// The refresh fans its per-file work out across every core, so its result
    /// must not depend on how the files happened to be split between workers.
    ///
    /// Enough files to guarantee more than one chunk on any machine, with a
    /// mix of tags, non-tags, and a changed file, so the merged counts and the
    /// final ordering are exercised rather than a single worker's happy path.
    #[test]
    fn entry_index_refresh_is_the_same_however_it_is_split_across_workers() {
        let root = temp_dir("index_refresh_parallel");
        let game = unique_game("index_refresh_parallel");
        fs::create_dir_all(root.join("objects")).unwrap();
        for i in 0..400 {
            write_fake_tag(&root.join(format!("objects/tag{i:03}.model")), b"hlmt");
        }
        // Not tags: they must be walked, counted as seen, and left out.
        for i in 0..40 {
            fs::write(root.join(format!("objects/note{i:02}.txt")), b"not a tag").unwrap();
        }
        let names = TagNameIndex::default();
        let entries = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        save_entry_index(&game, &root, &entries).unwrap();

        let unchanged = refresh_entry_index(&game, &root, &names).unwrap();
        assert!(!unchanged.changed, "nothing on disk moved");
        assert_eq!(unchanged.added, 0);
        assert_eq!(unchanged.updated, 0);
        assert_eq!(unchanged.removed, 0);
        assert_eq!(unchanged.entries.len(), 400);
        // Sorted, and sorted the same way a full scan sorts — the browser draws
        // folders in entry-vector order.
        assert_eq!(
            unchanged
                .entries
                .iter()
                .map(|entry| entry.display_path.clone())
                .collect::<Vec<_>>(),
            entries
                .iter()
                .map(|entry| entry.display_path.clone())
                .collect::<Vec<_>>()
        );

        // One file changes and one appears; the counts have to survive being
        // tallied by whichever worker happened to see them.
        write_fake_tag(&root.join("objects/tag007.model"), b"bipd");
        write_fake_tag(&root.join("objects/brand_new.weapon"), b"weap");
        let changed = refresh_entry_index(&game, &root, &names).unwrap();

        remove_test_index(&game);
        fs::remove_dir_all(&root).unwrap();

        assert!(changed.changed);
        assert_eq!(changed.added, 1, "one file appeared");
        assert_eq!(changed.updated, 1, "one file's group changed");
        assert_eq!(changed.removed, 0);
        assert_eq!(changed.entries.len(), 401);
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
        assert!(!refresh.changed);
        assert_eq!(refresh.updated, 0);
        assert_eq!(refresh.entries.len(), 1);
    }

    /// A one-row upsert must leave the index exactly as a full rewrite of the
    /// new entry set would, fingerprint included, so the next refresh finds
    /// nothing to do.
    #[test]
    fn entry_index_upsert_matches_a_full_rewrite() {
        let root = temp_dir("index_upsert");
        let game = unique_game("index_upsert");
        let rewrite_game = unique_game("index_upsert_rewrite");
        fs::create_dir_all(root.join("objects")).unwrap();
        fs::create_dir_all(root.join("saved")).unwrap();
        write_fake_tag(&root.join("objects/a.model"), b"hlmt");
        let names = TagNameIndex::default();
        let before = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        save_entry_index(&game, &root, &before).unwrap();

        // A new file, and an existing file rewritten under the same key.
        write_fake_tag(&root.join("saved/b.shader"), b"shdr");
        write_fake_tag_with_padding(&root.join("objects/a.model"), b"hlmt", 16);
        let after = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        for entry in &after {
            assert!(upsert_entry_index_row(&game, &root, entry).unwrap());
        }
        save_entry_index(&rewrite_game, &root, &after).unwrap();

        let upserted = load_entry_index(&game, &root).unwrap();
        let rewritten = load_entry_index(&rewrite_game, &root).unwrap();
        let refresh = refresh_entry_index(&game, &root, &names).unwrap();

        remove_test_index(&game);
        remove_test_index(&rewrite_game);
        fs::remove_dir_all(&root).unwrap();

        let paths = |entries: &[TagEntry]| {
            entries
                .iter()
                .map(|entry| {
                    (
                        entry.key.clone(),
                        entry.display_path.clone(),
                        entry.group_tag,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(upserted.len(), 2);
        assert_eq!(paths(&upserted), paths(&rewritten));
        assert!(!refresh.changed);
        assert_eq!((refresh.added, refresh.updated, refresh.removed), (0, 0, 0));
    }

    fn loose_source(root: &Path, game: &str, entries: Vec<TagEntry>, all: Vec<TagEntry>) -> LoadedSourceData {
        LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root: root.to_path_buf(),
                game: Some(game.to_owned()),
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: Some(game.to_owned()),
            entries,
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: all,
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
        }
    }

    /// One tag in, one tag out, on an indexed loose folder: the full list stays
    /// in `natural_key` order, a replaced entry keeps its lazy slot (trees hold
    /// positions in that list), and the on-disk index matches a full rewrite.
    #[test]
    fn upserting_and_removing_an_entry_keeps_lists_and_index_consistent() {
        let root = temp_dir("upsert_entry");
        let game = unique_game("upsert_entry");
        let check = unique_game("upsert_entry_check");
        fs::create_dir_all(root.join("objects")).unwrap();
        for name in ["a.model", "c.model"] {
            write_fake_tag(&root.join("objects").join(name), b"hlmt");
        }
        let names = TagNameIndex::default();
        let scanned = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        save_entry_index(&game, &root, &scanned).unwrap();
        let mut source = loose_source(&root, &game, scanned.clone(), scanned.clone());
        let lazy_c = source.entries.iter().position(|e| e.display_path.ends_with("c.model")).unwrap();

        // A new tag (sorted into the middle of the full list), a rewritten one
        // (replaced in place), and a deleted one.
        write_fake_tag(&root.join("objects/B.model"), b"hlmt");
        write_fake_tag_with_padding(&root.join("objects/c.model"), b"hlmt", 8);
        fs::remove_file(root.join("objects/a.model")).unwrap();
        let entry = |name: &str| loose_file_entry(&root, &root.join("objects").join(name), &names).unwrap().unwrap();
        source.upsert_entry(entry("B.model"), &[]);
        source.upsert_entry(entry("c.model"), &[]);
        let replaced_in_place = source.entries[lazy_c].display_path.ends_with("c.model")
            && source.entries.iter().filter(|e| e.display_path.ends_with("c.model")).count() == 1;
        // A removal is allowed to shift the lazy list: it re-reads the tree.
        source.remove_entry(&scanned[0].key, &[]);

        let after = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        save_entry_index(&check, &root, &after).unwrap();
        let indexed = load_entry_index(&game, &root).unwrap();
        let rewritten = load_entry_index(&check, &root).unwrap();
        let refresh = refresh_entry_index(&game, &root, &names).unwrap();

        remove_test_index(&game);
        remove_test_index(&check);
        fs::remove_dir_all(&root).unwrap();

        let order: Vec<&str> = source.all_entries.iter().map(|e| e.display_path.as_str()).collect();
        assert_eq!(order, ["objects/B.model", "objects/c.model"], "natural (case-insensitive) order");
        assert!(replaced_in_place, "a replaced entry keeps its slot, once");
        let keys = |entries: &[TagEntry]| entries.iter().map(|e| e.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&indexed), keys(&rewritten));
        assert!(!refresh.changed, "the index already matches the disk");
    }

    /// Expanding a folder again after its tree was rebuilt reuses the entries
    /// the lazy list already has, instead of appending the same keys again.
    #[test]
    fn re_expanding_a_folder_does_not_duplicate_its_entries() {
        let root = temp_dir("reexpand");
        fs::create_dir_all(root.join("objects")).unwrap();
        write_fake_tag(&root.join("objects/a.model"), b"hlmt");
        write_fake_tag(&root.join("objects/b.model"), b"hlmt");
        let names = TagNameIndex::default();
        let mut entries = Vec::new();
        for _ in 0..2 {
            let mut tree = build_folder_directory_tree(&root).unwrap();
            let node = tree.children.iter_mut().find(|node| node.label == "objects").unwrap();
            load_folder_node_entries(&root, node, &mut entries, &names).unwrap();
            assert_eq!(node.entries.len(), 2);
        }
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(entries.len(), 2, "the second expansion added nothing");
    }

    /// A container's list is the full list and stays in `natural_key` order.
    /// Several insert paths used to re-sort by a case-sensitive comparison,
    /// which put "B" before "a" and broke `insert_entry_sorted` afterwards.
    #[test]
    fn a_containers_entries_stay_in_natural_order() {
        let entry = |name: &str| TagEntry {
            key: format!("ublock:{name}"),
            display_path: name.to_owned(),
            group_tag: 0,
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(name)),
        };
        let mut source = LoadedSourceData {
            source: TagSource::SingleFile { path: PathBuf::from("x") },
            ..loose_source(Path::new("/unused"), "none", vec![entry("a"), entry("c")], Vec::new())
        };
        source.upsert_entry(entry("B"), &[]);
        source.upsert_entry(entry("b2"), &[]);
        let order: Vec<&str> = source.entries.iter().map(|e| e.display_path.as_str()).collect();
        assert_eq!(order, ["a", "B", "b2", "c"]);
    }

    /// A tag that cannot be opened (locked by another program, or unreadable)
    /// is left out of a scan and a refresh; it used to fail both outright.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_tag_is_skipped_not_fatal() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp_dir("unreadable_tag");
        let game = unique_game("unreadable_tag");
        fs::create_dir_all(root.join("objects")).unwrap();
        write_fake_tag(&root.join("objects/a.model"), b"hlmt");
        write_fake_tag(&root.join("objects/b.model"), b"hlmt");
        let names = TagNameIndex::default();
        let before = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        save_entry_index(&game, &root, &before).unwrap();
        let locked = root.join("objects/b.model");
        // Rewritten, so the refresh has to read it, then made unreadable.
        write_fake_tag_with_padding(&locked, b"hlmt", 4);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        let scanned = scan_folder_subtree_entries(&root, Path::new(""), &names);
        let refreshed = refresh_entry_index(&game, &root, &names);

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
        remove_test_index(&game);
        fs::remove_dir_all(&root).unwrap();
        let scanned = scanned.expect("the scan survives one unreadable tag");
        assert_eq!(scanned.len(), 1);
        let refreshed = refreshed.expect("so does the refresh");
        assert_eq!(refreshed.entries.len(), 1);
    }

    /// A refresh names what changed, so only those tags are rewritten: the
    /// rows it writes leave the index exactly as a full rewrite would, and
    /// references are saved per tag only where a reference index exists.
    #[test]
    fn a_refresh_reports_and_persists_only_what_changed() {
        let root = temp_dir("refresh_changes");
        let game = unique_game("refresh_changes");
        let check = unique_game("refresh_changes_check");
        fs::create_dir_all(root.join("objects")).unwrap();
        for name in ["a.model", "b.model", "c.model"] {
            write_fake_tag(&root.join("objects").join(name), b"hlmt");
        }
        let names = TagNameIndex::default();
        let before = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        save_entry_index(&game, &root, &before).unwrap();
        let key = |name: &str| before.iter().find(|e| e.display_path.ends_with(name)).unwrap().key.clone();

        write_fake_tag_with_padding(&root.join("objects/b.model"), b"hlmt", 8);
        fs::remove_file(root.join("objects/c.model")).unwrap();
        write_fake_tag(&root.join("objects/d.model"), b"hlmt");
        let refresh = refresh_entry_index(&game, &root, &names).unwrap();
        let mut touched: Vec<String> = refresh.touched.iter().map(|e| e.display_path.clone()).collect();
        touched.sort();

        // Row by row, as the refresh worker writes them.
        for gone in &refresh.removed_keys {
            delete_entry_index_row(&game, &root, gone).unwrap();
        }
        for entry in &refresh.touched {
            upsert_entry_index_row(&game, &root, entry).unwrap();
        }
        let after = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        save_entry_index(&check, &root, &after).unwrap();
        let patched = load_entry_index(&game, &root).unwrap();
        let rewritten = load_entry_index(&check, &root).unwrap();
        let no_reference_index =
            save_tag_dependencies(&game, &root, &key("a.model"), Some(&[])).unwrap();

        remove_test_index(&game);
        remove_test_index(&check);
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(touched, ["objects/b.model", "objects/d.model"]);
        assert_eq!(refresh.removed_keys, [key("c.model")]);
        let keys = |entries: &[TagEntry]| entries.iter().map(|e| e.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&patched), keys(&rewritten));
        assert!(!no_reference_index, "no reference index here, so none is started");
    }

    /// One row that names no file must not cost the whole index: the loader
    /// used to return nothing at all, so every other tag was re-probed and the
    /// reference index was lost with it.
    #[test]
    fn an_index_row_without_a_file_key_is_skipped_not_fatal() {
        let root = temp_dir("index_bad_row");
        let game = unique_game("index_bad_row");
        fs::create_dir_all(root.join("objects")).unwrap();
        write_fake_tag(&root.join("objects/a.model"), b"hlmt");
        let names = TagNameIndex::default();
        let entries = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();
        save_entry_index(&game, &root, &entries).unwrap();
        // What New Tag and Blam Import used to register.
        let mut bare = entries[0].clone();
        bare.key = "objects/b.model".to_owned();
        bare.display_path = "objects/b.model".to_owned();
        assert!(upsert_entry_index_row(&game, &root, &bare).unwrap());

        let loaded = load_entry_index(&game, &root);

        remove_test_index(&game);
        fs::remove_dir_all(&root).unwrap();
        let loaded = loaded.expect("the good rows must still load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].key, entries[0].key);
    }

    /// With no index for the folder yet, a lone row would load back as a
    /// complete one-tag index, so the upsert must not create one.
    #[test]
    fn entry_index_upsert_never_creates_an_index() {
        let root = temp_dir("index_upsert_absent");
        let game = unique_game("index_upsert_absent");
        fs::create_dir_all(root.join("objects")).unwrap();
        write_fake_tag(&root.join("objects/a.model"), b"hlmt");
        let names = TagNameIndex::default();
        let entries = scan_folder_subtree_entries(&root, Path::new(""), &names).unwrap();

        let written = upsert_entry_index_row(&game, &root, &entries[0]).unwrap();
        let loaded = load_entry_index(&game, &root);

        remove_test_index(&game);
        fs::remove_dir_all(&root).unwrap();

        assert!(!written);
        assert!(loaded.is_none());
    }

    #[test]

    fn sqlite_entry_index_keeps_multiple_roots_for_same_game() {
        let root_a = temp_dir("sqlite_multi_root_a");
        let root_b = temp_dir("sqlite_multi_root_b");
        let game = unique_game("sqlite_multi_root");
        fs::create_dir_all(root_a.join("objects")).unwrap();
        fs::create_dir_all(root_b.join("levels")).unwrap();
        write_fake_tag(&root_a.join("objects/a.model"), b"hlmt");
        write_fake_tag(&root_b.join("levels/b.scenario"), b"scnr");
        let names = TagNameIndex::default();
        let entries_a = scan_folder_subtree_entries(&root_a, Path::new(""), &names).unwrap();
        let entries_b = scan_folder_subtree_entries(&root_b, Path::new(""), &names).unwrap();

        save_entry_index(&game, &root_a, &entries_a).unwrap();
        save_entry_index(&game, &root_b, &entries_b).unwrap();
        let loaded_a = load_entry_index(&game, &root_a).unwrap();
        let loaded_b = load_entry_index(&game, &root_b).unwrap();

        remove_test_index(&game);
        fs::remove_dir_all(&root_a).unwrap();
        fs::remove_dir_all(&root_b).unwrap();

        assert_eq!(loaded_a.len(), 1);
        assert_eq!(loaded_a[0].display_path, "objects/a.model");
        assert_eq!(loaded_b.len(), 1);
        assert_eq!(loaded_b[0].display_path, "levels/b.scenario");
    }

    #[test]
    fn sqlite_reverse_dependency_index_round_trips_empty_and_targeted_dependencies() {
        let root = temp_dir("sqlite_reverse");
        let game = unique_game("sqlite_reverse");
        fs::create_dir_all(root.join("objects")).unwrap();
        let tag_key = format!("file:{}", root.join("objects/a.model").display());
        let empty_key = format!("file:{}", root.join("objects/empty.model").display());
        let mut index = ReverseDependencyIndex::default();
        index.set_tag_dependencies(
            tag_key.clone(),
            vec![DependencyRef {
                group_tag: u32::from_be_bytes(*b"bitm"),
                rel_path: "objects\\tex".to_owned(),
            }],
        );
        index.set_tag_dependencies(empty_key.clone(), Vec::new());

        save_reverse_dependency_index(&game, &root, &index).unwrap();
        let loaded = load_reverse_dependency_index(&game, &root).unwrap();

        remove_test_index(&game);
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.dependencies_of(&empty_key), &[]);
        assert_eq!(
            loaded.dependents_for(u32::from_be_bytes(*b"bitm"), "objects\\tex"),
            &[tag_key]
        );
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
    fn folder_tree_starts_beneath_the_opened_folder_and_keeps_full_paths() {
        let entries = vec![
            folder_entry("direct", "objects/characters/brute/brute.biped"),
            folder_entry("nested", "objects/characters/brute/bitmaps/brute.bitmap"),
            folder_entry("outside", "objects/characters/elite/elite.biped"),
        ];

        let tree = build_tree_beneath(&entries, Path::new("objects/characters/brute"));

        assert_eq!(tree.entries, vec![0]);
        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].label, "bitmaps");
        assert_eq!(
            tree.children[0].rel_path,
            PathBuf::from("objects/characters/brute/bitmaps")
        );
        assert_eq!(tree.children[0].entries, vec![1]);
    }

    #[test]
    fn folder_group_tree_keeps_indices_into_the_shared_entry_set() {
        let entries = vec![
            folder_entry("outside", "objects/characters/elite/elite.model"),
            folder_entry("inside", "objects/characters/brute/brute.model"),
        ];

        let tree = build_group_tree_beneath(&entries, Path::new("objects/characters/brute"));

        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].entries, vec![1]);
        assert!(entry_is_beneath_folder(
            &entries[1],
            Path::new("OBJECTS/CHARACTERS/BRUTE")
        ));
        assert!(!entry_is_beneath_folder(
            &entries[0],
            Path::new("objects/characters/brute")
        ));
    }

    fn folder_entry(key: &str, display_path: &str) -> TagEntry {
        TagEntry {
            key: key.into(),
            display_path: display_path.into(),
            group_tag: u32::from_be_bytes(*b"hlmt"),
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(key)),
        }
    }

    fn child<'a>(node: &'a TagTreeNode, label: &str) -> &'a TagTreeNode {
        node.children
            .iter()
            .find(|child| child.label == label)
            .unwrap_or_else(|| panic!("no child {label:?}"))
    }

    fn root_child<'a>(tree: &'a TagTree, label: &str) -> &'a TagTreeNode {
        tree.children
            .iter()
            .find(|child| child.label == label)
            .unwrap_or_else(|| panic!("no root child {label:?}"))
    }

    /// A pak's directory index cannot encode a directory with no file beneath
    /// it, so a folder the user made has to be carried by the tree until a tag
    /// lands in it. Without this it vanishes the moment it is created.
    #[test]
    fn a_seeded_folder_appears_even_though_no_entry_reaches_it() {
        let entries = vec![folder_entry("a", "objects/characters/a.model")];
        let tree = build_tree_with_folders(&entries, &["objects/vehicles/warthog".to_owned()]);

        let objects = root_child(&tree, "objects");
        let vehicles = child(objects, "vehicles");
        let warthog = child(vehicles, "warthog");
        assert!(warthog.entries.is_empty());
        assert!(warthog.children.is_empty());
        // The deletable-folder rule the browser draws with.
        assert!(warthog.pending);
        // `objects` already existed, so seeding through it must not claim it.
        assert!(!objects.pending);
        // An intermediate the seed did create is pending, but it has a child, so
        // it is not offered for deletion until the leaf goes first.
        assert!(vehicles.pending);
        assert!(!vehicles.children.is_empty());
    }

    /// Re-seeding a folder a tag has since landed in must not produce a second
    /// node beside the real one, and must not relabel the real one as pending.
    #[test]
    fn seeding_a_folder_that_now_holds_a_tag_is_a_no_op() {
        let entries = vec![folder_entry("a", "objects/vehicles/warthog/a.model")];
        let tree = build_tree_with_folders(&entries, &["objects/vehicles/warthog".to_owned()]);

        let vehicles = child(root_child(&tree, "objects"), "vehicles");
        assert_eq!(vehicles.children.len(), 1, "no duplicate warthog node");
        let warthog = child(vehicles, "warthog");
        assert_eq!(warthog.entries, vec![0]);
        assert!(!warthog.pending, "a folder a tag reached is not pending");
    }

    /// `build_tree` is the same code path with an empty seed list, so nothing
    /// that does not opt in can acquire a pending node.
    #[test]
    fn build_tree_seeds_nothing_and_marks_nothing_pending() {
        let entries = vec![folder_entry("a", "objects/characters/a.model")];
        let tree = build_tree(&entries);
        let objects = root_child(&tree, "objects");
        assert!(!objects.pending);
        assert!(!child(objects, "characters").pending);
        assert_eq!(objects.children.len(), 1);
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
