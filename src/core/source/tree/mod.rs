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
        let label = group_tree_label(&entries[index]);
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

/// The Groups-view node an entry is filed under, e.g. `bitmap bitm`.
pub fn group_tree_label(entry: &TagEntry) -> String {
    let fourcc = format_group_tag(entry.group_tag);
    let group = friendly_group_name(entry.group_tag, entry.group_name.as_deref(), &fourcc);
    if group == fourcc {
        fourcc
    } else {
        format!("{group} {fourcc}")
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

/// A group's name when the tag's own index has none: the engine's table,
/// generated from every MCC title's definitions.
///
/// A hand-kept table used to follow it here, but every group in it is also
/// in the engine's, so it could never answer; six of its names were wrong
/// besides (`pman` as `particle_model`, `snde` as `sound_effect_template`).
fn fallback_group_name(group_tag: u32) -> Option<&'static str> {
    group_tag_to_extension(group_tag)
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
            .is_some_and(skippable_io_error)
    })
}

/// A file a scan passes over rather than failing on: gone, not ours to read,
/// or (on Windows) held open by another program that shares nothing, such as
/// an editing-kit tool saving it. A sharing or lock violation has no
/// `ErrorKind` of its own, so it is matched by code.
fn skippable_io_error(error: &std::io::Error) -> bool {
    /// `ERROR_SHARING_VIOLATION` and `ERROR_LOCK_VIOLATION`.
    #[cfg(windows)]
    const HELD_BY_ANOTHER_PROGRAM: [i32; 2] = [32, 33];
    #[cfg(windows)]
    if error
        .raw_os_error()
        .is_some_and(|code| HELD_BY_ANOTHER_PROGRAM.contains(&code))
    {
        return true;
    }
    matches!(
        error.kind(),
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
                    .is_some_and(skippable_io_error) =>
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
            key: file_entry_key(&path),
            display_path,
            group_tag,
            group_name,
            location: TagEntryLocation::LooseFile(path),
        });
    }
    entries.sort_by(|a, b| natural_key(&a.display_path).cmp(&natural_key(&b.display_path)));
    Ok(entries)
}

// Entry keys are built in `crate::core::tag_key`.
pub(crate) use crate::core::tag_key::{cache_entry_key, container_entry_key, file_entry_key};

/// The label a container's tags are keyed under: its `.utoc` file stem, so a
/// renamed mod or a renumbered chunk changes every key in it.
pub(crate) fn container_chunk_label(utoc: &Path) -> String {
    utoc.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("container")
        .to_string()
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
    // The folder scan's path is the root as given plus what it walked, joined
    // with the platform's separator. A caller's path may have been joined from
    // a `/`-separated relative path instead, which on Windows leaves both
    // separators in it and a key the scan never produces. Rebuild the
    // relative part from its components so the two agree.
    let path = match path.strip_prefix(root) {
        Ok(rel) => root.join(rel.components().collect::<PathBuf>()),
        Err(_) => path.to_path_buf(),
    };
    let group_name = names.name_for(group_tag).map(str::to_owned);
    let display_path = display_path_with_friendly_extension(rel, group_tag, names);
    Ok(Some(TagEntry {
        key: file_entry_key(&path),
        display_path,
        group_tag,
        group_name,
        location: TagEntryLocation::LooseFile(path),
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
            key: file_entry_key(&path),
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
            key: file_entry_key(&path),
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

/// A tag file's display path: its path with the file's own extension
/// replaced by the group's friendly name. Only the file name's extension is
/// replaced; a dot in a folder name is part of the folder.
pub(super) fn display_path_with_friendly_extension(
    path: &Path,
    group_tag: u32,
    names: &TagNameIndex,
) -> String {
    let display = path_to_display(path);
    let extension = friendly_extension(group_tag, names);
    match leaf_extension_dot(&display) {
        Some(dot) => format!("{}.{extension}", &display[..dot]),
        None => format!("{display}.{extension}"),
    }
}

/// A display path for a tag *name*, such as a monolithic cache's tag name or a
/// container's logical path: names that carry no file extension.
///
/// A dot in such a name is part of the name (`piston_close2.l`, a folder
/// called `v1.2`), so the friendly extension is appended. This used to cut at
/// the last dot anywhere in the path and replace what followed, turning
/// `piston_close2.l` into `piston_close2.sound` and `levels/v1.2/bitmaps/rock`
/// into `levels/v1.bitmap`. Only a final suffix that already names this
/// group (`.bipd`, `.biped`) is replaced instead of doubled.
pub(super) fn display_str_with_friendly_extension(
    display: &str,
    group_tag: u32,
    names: &TagNameIndex,
) -> String {
    let extension = friendly_extension(group_tag, names);
    if let Some(dot) = leaf_extension_dot(display)
        && suffix_names_group(&display[dot + 1..], group_tag, names)
    {
        return format!("{}.{extension}", &display[..dot]);
    }
    format!("{display}.{extension}")
}

/// Where the last dot of the final path component is, when it is not the
/// first character of the path.
fn leaf_extension_dot(display: &str) -> Option<usize> {
    let leaf_start = display.rfind('/').map_or(0, |slash| slash + 1);
    let dot = leaf_start + display[leaf_start..].rfind('.')?;
    (dot > 0).then_some(dot)
}

/// Whether `suffix` is one of the names this group goes by: its friendly
/// name, its four-character code, or the engine table's name for it.
fn suffix_names_group(suffix: &str, group_tag: u32, names: &TagNameIndex) -> bool {
    let fourcc = format_group_tag(group_tag);
    [
        names.name_for(group_tag),
        group_tag_to_extension(group_tag),
        Some(fourcc.trim_end()),
    ]
    .into_iter()
    .flatten()
    .any(|name| name.eq_ignore_ascii_case(suffix))
}

/// The extension a group's tags display with: the game's own definitions,
/// then the engine's cross-game table, which only runs before definitions
/// have loaded.
///
/// A hand-kept table used to sit ahead of the engine's. It agreed with it
/// everywhere but two rows: `bloc` as `device_control` (it is `crate` in every
/// game that has it) and a `crat` group that no game has.
fn friendly_extension(group_tag: u32, names: &TagNameIndex) -> String {
    names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag))
        .map(str::to_owned)
        .unwrap_or_else(|| format_group_tag(group_tag))
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
mod tests;
