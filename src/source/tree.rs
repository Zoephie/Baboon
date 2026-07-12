//! Tag trees, folder scanning, and entry discovery.
//! It owns source identity, discovery, indexing, and source-aware reads; editor presentation and application workflow state belong elsewhere.

use super::*;

/// Builds a path hierarchy whose stored indices address `entries` exactly.
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

/// Groups entries by friendly tag group while preserving entry-vector indices.
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
    let mut new_entries = scan_folder_direct_entries(root, &folder, names)?;
    new_entries.sort_by(|a, b| natural_key(&a.display_path).cmp(&natural_key(&b.display_path)));
    let start = entries.len();
    node.entries.extend(start..start + new_entries.len());
    entries.extend(new_entries);
    node.entries_loaded = true;
    Ok(())
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

/// Probes one loose file and returns its stable source entry when it is a tag.
/// Group detection is source-aware and must not be replaced by extension alone.
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

pub(super) fn natural_key(value: &str) -> String {
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
