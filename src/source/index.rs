//! Persistent entry and reverse-dependency indexes.
//! It owns source identity, discovery, indexing, and source-aware reads; editor presentation and application workflow state belong elsewhere.

use super::*;

/// Returns the legacy per-game JSON entry-index path used for migration.
pub fn index_path(game: &str) -> PathBuf {
    app_cache_path(&format!("{game}_index.json"), "Baboon", "baboon")
}

/// Legacy JSON reverse-dependency path. New saves use [`index_db_path`].
pub fn reverse_dependency_index_path(game: &str) -> PathBuf {
    app_cache_path(
        &format!("{game}_reverse_dependencies.json"),
        "Baboon",
        "baboon",
    )
}

/// Returns the shared SQLite index path used by current entry/dependency caches.
pub fn index_db_path() -> PathBuf {
    app_cache_path("indexes.sqlite3", "Baboon", "baboon")
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
    if windows_folder == "Baboon" && unix_folder == "baboon" {
        return crate::storage::data_path(filename);
    }
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

/// Persist `entries` to the shared SQLite index DB. Called from the background
/// worker after a full scan completes so it never blocks the UI thread.
/// `root` is recorded so a stale index from a different folder for the same
/// game is rejected on load.
pub fn save_entry_index(game: &str, root: &Path, entries: &[TagEntry]) -> Result<()> {
    save_entry_index_to_db(game, root, entries)
}

/// Load a previously saved index for `game`. Returns `None` if no file exists,
/// it can't be parsed, or it was saved for a different `root` folder (the keys
/// are absolute paths, so the index is only valid for its original root).
pub fn load_entry_index(game: &str, root: &Path) -> Option<Vec<TagEntry>> {
    if let Some((entries, _)) = load_entry_index_from_db(game, root) {
        return (!entries.is_empty()).then_some(entries);
    }
    let value = load_legacy_entry_index_value(game)?;
    let (entries, _) = parse_entry_index(root, &value)?;
    let _ = save_entry_index_to_db(game, root, &entries);
    (!entries.is_empty()).then_some(entries)
}

/// Reconciles cached fingerprints with disk without parsing unchanged tags.
/// The returned entries remain sorted and keyed exactly like a full scan.
pub fn refresh_entry_index(
    game: &str,
    root: &Path,
    names: &TagNameIndex,
) -> Result<EntryIndexRefresh> {
    let (cached_entries, cached_fingerprints) = load_entry_index_from_db(game, root)
        .or_else(|| {
            let value = load_legacy_entry_index_value(game)?;
            let parsed = parse_entry_index(root, &value)?;
            let _ = save_entry_index_to_db(game, root, &parsed.0);
            Some(parsed)
        })
        .unwrap_or_default();
    refresh_entry_index_from_cache(root, names, &cached_entries, &cached_fingerprints)
}

fn load_legacy_entry_index_value(game: &str) -> Option<serde_json::Value> {
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
        TagEntryLocation::Monolithic { .. } | TagEntryLocation::Container { .. } => None,
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

/// Persists both directions of a source-scoped dependency index atomically.
pub fn save_reverse_dependency_index(
    game: &str,
    root: &Path,
    index: &ReverseDependencyIndex,
) -> Result<()> {
    save_reverse_dependency_index_to_db(game, root, index)
}

/// Loads a dependency index only when its canonical source root still matches.
pub fn load_reverse_dependency_index(game: &str, root: &Path) -> Option<ReverseDependencyIndex> {
    if let Some(index) = load_reverse_dependency_index_from_db(game, root) {
        return Some(index);
    }
    let index = load_reverse_dependency_index_from_json(game, root)?;
    let _ = save_reverse_dependency_index_to_db(game, root, &index);
    Some(index)
}

fn load_reverse_dependency_index_from_json(
    game: &str,
    root: &Path,
) -> Option<ReverseDependencyIndex> {
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

pub(super) fn open_index_db() -> Result<Connection> {
    let path = index_db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create index db dir")?;
    }
    let conn =
        Connection::open(&path).with_context(|| format!("open index db {}", path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .context("enable index db WAL mode")?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .context("set index db synchronous mode")?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .context("enable index db foreign keys")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS sources (
            id INTEGER PRIMARY KEY,
            game TEXT NOT NULL,
            root_key TEXT NOT NULL,
            schema_version INTEGER NOT NULL DEFAULT 1,
            updated_at INTEGER NOT NULL DEFAULT 0,
            UNIQUE(game, root_key)
        );
        CREATE TABLE IF NOT EXISTS entries (
            source_id INTEGER NOT NULL,
            key TEXT NOT NULL,
            rel_path TEXT NOT NULL,
            display_path TEXT NOT NULL,
            group_tag INTEGER NOT NULL,
            group_name TEXT,
            size INTEGER,
            modified_secs INTEGER,
            modified_nanos INTEGER,
            PRIMARY KEY(source_id, key),
            FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS dependencies (
            source_id INTEGER NOT NULL,
            tag_key TEXT NOT NULL,
            dep_group_tag INTEGER NOT NULL,
            dep_rel_path TEXT NOT NULL,
            PRIMARY KEY(source_id, tag_key, dep_group_tag, dep_rel_path),
            FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS indexed_tags (
            source_id INTEGER NOT NULL,
            tag_key TEXT NOT NULL,
            PRIMARY KEY(source_id, tag_key),
            FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS entries_by_source_rel ON entries(source_id, rel_path);
        CREATE INDEX IF NOT EXISTS deps_by_target ON dependencies(source_id, dep_group_tag, dep_rel_path);
        CREATE INDEX IF NOT EXISTS deps_by_tag ON dependencies(source_id, tag_key);
        "#,
    )
    .context("initialize index db schema")?;
    Ok(conn)
}

fn source_id(conn: &Connection, game: &str, root: &Path) -> Result<Option<i64>> {
    let root_key = cache_root_key(root);
    conn.query_row(
        "SELECT id FROM sources WHERE game = ?1 AND root_key = ?2",
        params![game, root_key],
        |row| row.get(0),
    )
    .optional()
    .context("query index source")
}

fn ensure_source_id(conn: &Connection, game: &str, root: &Path) -> Result<i64> {
    let root_key = cache_root_key(root);
    conn.execute(
        "INSERT INTO sources (game, root_key, schema_version, updated_at)
         VALUES (?1, ?2, 1, unixepoch())
         ON CONFLICT(game, root_key)
         DO UPDATE SET schema_version = 1, updated_at = unixepoch()",
        params![game, root_key],
    )
    .context("upsert index source")?;
    source_id(conn, game, root)?.context("index source missing after upsert")
}

fn save_entry_index_to_db(game: &str, root: &Path, entries: &[TagEntry]) -> Result<()> {
    let mut conn = open_index_db()?;
    let tx = conn
        .transaction()
        .context("begin entry index transaction")?;
    let source_id = ensure_source_id(&tx, game, root)?;
    tx.execute(
        "DELETE FROM entries WHERE source_id = ?1",
        params![source_id],
    )
    .context("clear entry index rows")?;
    {
        let mut insert = tx
            .prepare(
                "INSERT INTO entries (
                    source_id, key, rel_path, display_path, group_tag, group_name,
                    size, modified_secs, modified_nanos
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .context("prepare entry index insert")?;
        for entry in entries {
            let rel_path = entry_relative_path(root, entry)
                .map(|path| path_to_display(&path))
                .unwrap_or_default();
            let fingerprint =
                entry_file_path(entry).and_then(|path| file_fingerprint(path).ok().flatten());
            insert
                .execute(params![
                    source_id,
                    &entry.key,
                    rel_path,
                    &entry.display_path,
                    i64::from(entry.group_tag),
                    entry.group_name.as_deref(),
                    fingerprint.map(|fp| fp.size as i64),
                    fingerprint.map(|fp| fp.modified_secs as i64),
                    fingerprint.map(|fp| i64::from(fp.modified_nanos)),
                ])
                .context("insert entry index row")?;
        }
    }
    tx.commit().context("commit entry index transaction")?;
    Ok(())
}

fn load_entry_index_from_db(
    game: &str,
    root: &Path,
) -> Option<(Vec<TagEntry>, HashMap<PathBuf, EntryFingerprint>)> {
    let conn = open_index_db().ok()?;
    let source_id = source_id(&conn, game, root).ok().flatten()?;
    let mut stmt = conn
        .prepare(
            "SELECT key, rel_path, display_path, group_tag, group_name,
                    size, modified_secs, modified_nanos
             FROM entries
             WHERE source_id = ?1
             ORDER BY display_path COLLATE NOCASE, key COLLATE NOCASE",
        )
        .ok()?;
    let rows = stmt
        .query_map(params![source_id], |row| {
            let key: String = row.get(0)?;
            let rel_path: String = row.get(1)?;
            let display_path: String = row.get(2)?;
            let group_tag: i64 = row.get(3)?;
            let group_name: Option<String> = row.get(4)?;
            let size: Option<i64> = row.get(5)?;
            let modified_secs: Option<i64> = row.get(6)?;
            let modified_nanos: Option<i64> = row.get(7)?;
            let location = key
                .strip_prefix("file:")
                .map(|abs| TagEntryLocation::LooseFile(PathBuf::from(abs)));
            let fingerprint = match (size, modified_secs, modified_nanos) {
                (Some(size), Some(modified_secs), Some(modified_nanos))
                    if size >= 0 && modified_secs >= 0 && modified_nanos >= 0 =>
                {
                    Some(EntryFingerprint {
                        size: size as u64,
                        modified_secs: modified_secs as u64,
                        modified_nanos: modified_nanos as u32,
                    })
                }
                _ => None,
            };
            Ok((
                location.map(|location| TagEntry {
                    key,
                    display_path,
                    group_tag: group_tag as u32,
                    group_name,
                    location,
                }),
                rel_path,
                fingerprint,
            ))
        })
        .ok()?;
    let mut entries = Vec::new();
    let mut fingerprints = HashMap::new();
    for row in rows {
        let (entry, rel_path, fingerprint) = row.ok()?;
        let entry = entry?;
        if !rel_path.is_empty()
            && let Some(fingerprint) = fingerprint
        {
            fingerprints.insert(normalize_rel_path(Path::new(&rel_path)), fingerprint);
        }
        entries.push(entry);
    }
    Some((entries, fingerprints))
}

fn save_reverse_dependency_index_to_db(
    game: &str,
    root: &Path,
    index: &ReverseDependencyIndex,
) -> Result<()> {
    let mut conn = open_index_db()?;
    let tx = conn
        .transaction()
        .context("begin reverse dependency index transaction")?;
    let source_id = ensure_source_id(&tx, game, root)?;
    tx.execute(
        "DELETE FROM dependencies WHERE source_id = ?1",
        params![source_id],
    )
    .context("clear reverse dependency rows")?;
    tx.execute(
        "DELETE FROM indexed_tags WHERE source_id = ?1",
        params![source_id],
    )
    .context("clear indexed tag rows")?;
    {
        let mut insert_tag = tx
            .prepare(
                "INSERT OR IGNORE INTO indexed_tags (source_id, tag_key)
                 VALUES (?1, ?2)",
            )
            .context("prepare indexed tag insert")?;
        let mut insert = tx
            .prepare(
                "INSERT OR IGNORE INTO dependencies (
                    source_id, tag_key, dep_group_tag, dep_rel_path
                 ) VALUES (?1, ?2, ?3, ?4)",
            )
            .context("prepare reverse dependency insert")?;
        for (tag_key, deps) in &index.by_tag {
            insert_tag
                .execute(params![source_id, tag_key])
                .context("insert indexed tag row")?;
            for dep in deps {
                insert
                    .execute(params![
                        source_id,
                        tag_key,
                        i64::from(dep.group_tag),
                        &dep.rel_path,
                    ])
                    .context("insert reverse dependency row")?;
            }
        }
    }
    tx.commit()
        .context("commit reverse dependency index transaction")?;
    Ok(())
}

fn load_reverse_dependency_index_from_db(
    game: &str,
    root: &Path,
) -> Option<ReverseDependencyIndex> {
    let conn = open_index_db().ok()?;
    let source_id = source_id(&conn, game, root).ok().flatten()?;
    let mut tags_stmt = conn
        .prepare(
            "SELECT tag_key
             FROM indexed_tags
             WHERE source_id = ?1
             ORDER BY tag_key COLLATE NOCASE",
        )
        .ok()?;
    let tag_rows = tags_stmt
        .query_map(params![source_id], |row| row.get::<_, String>(0))
        .ok()?;
    let mut grouped: BTreeMap<String, Vec<DependencyRef>> = BTreeMap::new();
    for row in tag_rows {
        grouped.entry(row.ok()?).or_default();
    }
    let mut stmt = conn
        .prepare(
            "SELECT tag_key, dep_group_tag, dep_rel_path
             FROM dependencies
             WHERE source_id = ?1
             ORDER BY tag_key COLLATE NOCASE, dep_group_tag, dep_rel_path COLLATE NOCASE",
        )
        .ok()?;
    let rows = stmt
        .query_map(params![source_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                DependencyRef {
                    group_tag: row.get::<_, i64>(1)? as u32,
                    rel_path: row.get(2)?,
                },
            ))
        })
        .ok()?;
    for row in rows {
        let (tag_key, dep) = row.ok()?;
        grouped.entry(tag_key).or_default().push(dep);
    }
    if grouped.is_empty() {
        return None;
    }
    let mut index = ReverseDependencyIndex::default();
    for (tag_key, deps) in grouped {
        index.set_tag_dependencies(tag_key, deps);
    }
    Some(index)
}

pub(crate) fn dependency_key(group_tag: u32, rel_path: &str) -> String {
    format!("{group_tag:08x}\t{}", normalize_dependency_path(rel_path))
}

pub(crate) fn normalize_dependency_path(rel_path: &str) -> String {
    rel_path.replace('/', "\\").to_ascii_lowercase()
}

#[cfg(test)]
/// Produces bounded, stable field summaries for search indexing and reports.
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
