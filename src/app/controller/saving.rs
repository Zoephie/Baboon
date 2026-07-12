//! Definition discovery and validated new-tag output paths.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::ExportFinished` to the application status.
    pub(super) fn handle_export_finished(&mut self, result: Result<String, String>) -> bool {
        self.status = match result {
            Ok(message) => message,
            Err(error) => error,
        };
        false
    }

    /// Applies `WorkerMessage::EntryIndexSaved`, rejecting stale source generations.
    pub(super) fn handle_entry_index_saved(
        &mut self,
        generation: u64,
        path: PathBuf,
        result: Result<(), String>,
    ) -> bool {
        if generation != self.source_generation {
            return true;
        }
        match result {
            Ok(()) => {
                if !self.building_reference_for_entry_index {
                    self.status = format!("Index saved: {}", path.display());
                }
            }
            Err(error) => {
                self.status = format!("Index save failed: {} ({error})", path.display());
            }
        }
        false
    }
}

pub(super) fn ordered_unique_keys<'a>(keys: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for key in keys {
        if seen.insert(key.clone()) {
            ordered.push(key.clone());
        }
    }
    ordered
}

pub(super) fn save_as_extension(app: &Baboon, entry: &TagEntry) -> Option<String> {
    app.names
        .name_for(entry.group_tag)
        .or_else(|| group_tag_to_extension(entry.group_tag))
        .map(|extension| extension.trim().to_owned())
        .filter(|extension| !extension.is_empty())
}

pub(super) fn register_saved_copy_in_loaded_source(
    source: &mut LoadedSourceData,
    path: &Path,
) -> Result<bool, String> {
    let TagSource::LooseFolder { root, .. } = &source.source else {
        return Ok(false);
    };
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| format!("Could not resolve loaded tags folder: {error}"))?;
    let canonical_path = fs::canonicalize(path)
        .map_err(|error| format!("Could not resolve saved tag path: {error}"))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Ok(false);
    }
    let Some(entry) = loose_file_entry(&canonical_root, &canonical_path, &source.names)
        .map_err(|error| format!("Could not inspect saved tag: {error:#}"))?
    else {
        return Ok(false);
    };
    let key = entry.key.clone();
    source.entries.retain(|existing| existing.key != key);
    source.entries.push(entry.clone());
    source
        .entries
        .sort_by(|a, b| a.display_path.cmp(&b.display_path));
    if !source.all_entries.is_empty() {
        source.all_entries.retain(|existing| existing.key != key);
        source.all_entries.push(entry);
        source
            .all_entries
            .sort_by(|a, b| a.display_path.cmp(&b.display_path));
        source.group_tree = crate::source::build_group_tree(&source.all_entries);
        if let (Some(game), TagSource::LooseFolder { root, .. }) =
            (source.game.as_deref(), &source.source)
        {
            let _ = crate::source::save_entry_index(game, root, &source.all_entries);
        }
    } else {
        source.group_tree = crate::source::build_group_tree(&source.entries);
    }
    if let TagSource::LooseFolder { root, .. } = &source.source
        && let Ok(tree) = crate::source::build_folder_directory_tree(root)
    {
        source.tree = tree;
    }
    Ok(true)
}

pub(super) fn save_as_file_name(entry: &TagEntry, extension: Option<&str>) -> String {
    let path = match &entry.location {
        TagEntryLocation::LooseFile(path) => path,
        TagEntryLocation::Monolithic { .. } => Path::new(&entry.display_path),
    };
    let mut file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .or_else(|| {
            Path::new(&entry.display_path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| clean_file_name(&entry.display_path));
    if Path::new(&file_name).extension().is_none() {
        if let Some(extension) = extension {
            file_name.push('.');
            file_name.push_str(extension);
        }
    }
    file_name
}

pub(super) fn save_as_start_dir(entry: &TagEntry) -> Option<PathBuf> {
    match &entry.location {
        TagEntryLocation::LooseFile(path) => path.parent().map(Path::to_path_buf),
        TagEntryLocation::Monolithic { .. } => None,
    }
}

pub(super) fn entries_for_keys(source: &LoadedSourceData, keys: &[String]) -> Vec<TagEntry> {
    let key_set = keys.iter().map(String::as_str).collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    source
        .entries
        .iter()
        .chain(source.all_entries.iter())
        .filter(|entry| key_set.contains(entry.key.as_str()))
        .filter(|entry| seen.insert(entry.key.as_str()))
        .cloned()
        .collect()
}

fn clean_file_name(value: &str) -> String {
    let mut name = value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("tag")
        .trim()
        .to_owned();
    name.retain(|ch| !matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'));
    if name.is_empty() {
        "tag".to_owned()
    } else {
        name
    }
}

pub(in crate::app) fn available_definition_games() -> Vec<String> {
    let root = locate_definitions_root();
    let mut games = fs::read_dir(root)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            let path = entry.path();
            path.join("_meta.json")
                .is_file()
                .then(|| entry.file_name().to_string_lossy().trim().to_owned())
        })
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    games.sort();
    games.dedup();
    if games.is_empty() {
        games.push("halo3_mcc".to_owned());
    }
    games
}

pub(in crate::app) fn load_new_tag_groups(game: &str) -> Result<Vec<NewTagGroup>, String> {
    let game_dir = locate_definitions_root().join(game);
    if !game_dir.parent().is_some_and(|root| root.is_dir()) {
        return Err(definitions_missing_message(&locate_definitions_root()));
    }
    let meta_path = game_dir.join("_meta.json");
    let bytes = fs::read(&meta_path).map_err(|error| {
        if !locate_definitions_root().is_dir() {
            definitions_missing_message(&locate_definitions_root())
        } else {
            format!("Could not read {}: {error}", meta_path.display())
        }
    })?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse {}: {error}", meta_path.display()))?;
    let Some(tag_index) = value.get("tag_index").and_then(Value::as_object) else {
        return Err(format!("{} is missing tag_index", meta_path.display()));
    };
    let mut groups = Vec::new();
    for (fourcc, name_value) in tag_index {
        let Some(name) = name_value.as_str() else {
            continue;
        };
        let Some(group_tag) = parse_group_tag(fourcc) else {
            continue;
        };
        let disk_schema_path = game_dir.join(format!("{name}.json"));
        if !disk_schema_path.is_file() {
            continue;
        }
        groups.push(NewTagGroup {
            group_tag,
            name: name.to_owned(),
            schema_path: disk_schema_path,
            extension: group_tag_to_extension(group_tag)
                .unwrap_or(name)
                .trim()
                .to_owned(),
        });
    }
    groups.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.group_tag.cmp(&b.group_tag))
    });
    Ok(groups)
}

pub(in crate::app) fn new_tag_output_path_from_dialog(
    tags_root: &Path,
    picked_path: &Path,
    extension: &str,
) -> Result<(PathBuf, String), String> {
    let extension = extension.trim_start_matches('.');
    let mut output = picked_path.to_path_buf();
    output.set_extension(extension);
    let root = lexical_normalize_path(tags_root);
    let output = lexical_normalize_path(&output);
    if !output.starts_with(&root) {
        return Err("Choose a location inside the loaded tags folder".to_owned());
    }
    let rel = output
        .strip_prefix(&root)
        .map_err(|_| "Choose a location inside the loaded tags folder".to_owned())?;
    if rel.as_os_str().is_empty()
        || rel.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return Err("Choose a tag name inside the loaded tags folder".to_owned());
    }
    let display = rel.to_string_lossy().replace('\\', "/");
    Ok((output, display))
}

pub(super) fn lexical_normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}
