use super::*;
use anyhow::Context as _;

const TERMINAL_VISIBLE_LINE_LIMIT: usize = 20_000;
const TERMINAL_VISIBLE_LINE_TRIM_TARGET: usize = 18_000;

impl Baboon {
    fn push_terminal_line(&mut self, line: String) {
        self.terminal.lines.push(TerminalLineEntry::new(line));
        trim_terminal_lines(&mut self.terminal.lines);
        self.terminal.scroll_to_bottom = true;
    }

    pub(super) fn process_worker_messages(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                WorkerMessage::TerminalLine(line) => {
                    self.push_terminal_line(line);
                }
                WorkerMessage::TerminalLogError(error) => {
                    self.status = error;
                }
                WorkerMessage::TerminalDone { run_id } => {
                    if self.terminal.running_id != Some(run_id) {
                        continue;
                    }
                    self.terminal.running = false;
                    self.terminal.running_id = None;
                    self.terminal.running_command = None;
                    self.terminal.process = None;
                    self.terminal.scroll_to_bottom = true;
                    self.terminal.refocus_input = true;
                }
                WorkerMessage::UpdateCheckFinished(result) => {
                    self.status = match result {
                        Ok(result) => update_check_status(&result),
                        Err(error) if error == NO_PUBLIC_RELEASE_MESSAGE => error,
                        Err(error) => format!("Update check failed: {error}"),
                    };
                }
                WorkerMessage::FieldValueSearchFinished {
                    generation,
                    query,
                    result,
                } => {
                    self.field_value_searching = false;
                    // Discard results from a source that has since been reloaded.
                    if generation != self.source_generation {
                        continue;
                    }
                    match result {
                        Ok(matches) => {
                            let entries: Vec<TagEntry> =
                                matches.iter().map(|m| m.entry.clone()).collect();
                            let annotations: Vec<String> =
                                matches.iter().map(|m| m.label.clone()).collect();
                            let note = entries
                                .is_empty()
                                .then(|| format!("No tag field values contain \"{query}\"."));
                            self.status = format!(
                                "Field search for \"{query}\": {} match(es)",
                                entries.len()
                            );
                            self.query_results = Some(TagQueryResults {
                                title: format!("Field value '{query}' ({})", entries.len()),
                                entries,
                                annotations,
                                note,
                            });
                        }
                        Err(error) => {
                            self.status = format!("Field search failed: {error}");
                        }
                    }
                }
                WorkerMessage::FieldIndexBuilt { generation, blobs } => {
                    if generation == self.source_generation {
                        self.field_index.install(generation, blobs);
                    }
                }
                WorkerMessage::ReverseDependenciesBuilt { generation, index } => {
                    self.building_reverse_dependencies = false;
                    // Discard a build that finished against a now-stale source.
                    if generation != self.source_generation {
                        continue;
                    }
                    if let Some(source) = self.source.as_mut() {
                        let n = index.len();
                        // Persist so the next launch skips the rebuild entirely.
                        if let (Some(game), TagSource::LooseFolder { root, .. }) =
                            (source.game.clone(), &source.source)
                        {
                            let root = root.clone();
                            let to_save = index.clone();
                            thread::spawn(move || {
                                if let Err(e) = crate::source::save_reverse_dependency_index(
                                    &game, &root, &to_save,
                                ) {
                                    eprintln!("reverse-dependency index save failed: {e}");
                                }
                            });
                        }
                        source.reverse_dependencies = Some(index);
                        self.status = format!("Reference index complete: {n} tags");
                    }
                }
                WorkerMessage::SourceLoaded {
                    result: Ok(mut loaded),
                    recent_path,
                } => {
                    if let Some(path) = recent_path {
                        self.remember_recent_folder(path);
                    }
                    // Set terminal work dir to the game kit root (parent of tags/).
                    self.terminal_work_dir =
                        if let TagSource::LooseFolder { root, .. } = &loaded.source {
                            root.parent().map(|p| p.to_owned())
                        } else {
                            None
                        };
                    // Restore the per-kit terminal-open preference for this game.
                    self.terminal_open = loaded
                        .game
                        .as_deref()
                        .map(|g| self.terminal_open_games.contains(g))
                        .unwrap_or(false);
                    self.names = loaded.names.clone();
                    // Backfill any groups the source's game index lacks (and
                    // cover the case where definitions failed to load on disk).
                    self.names.merge_missing(self.default_names.clone());
                    self.parsed_tags.clear();
                    self.tag_cache_order.clear();
                    self.loading_tags.clear();
                    self.bitmap_previews.clear();
                    self.rmdf_cache.clear();
                    self.rmop_cache.clear();
                    self.color_popup = None;
                    self.function_popup = None;
                    self.selected_key = None;
                    self.open_tabs.clear();
                    self.floating_tabs.clear();
                    if let Some((key, tag)) = loaded.initial_tag.take() {
                        self.selected_key = Some(key.clone());
                        self.open_tabs.push(key.clone());
                        self.remember_tag_use(&key);
                        self.parsed_tags.insert(key, TagDocument::clean(tag));
                    }
                    self.status = loaded_source_status(&loaded);
                    // Load this game's keyword sidecar (external to the tags).
                    self.keywords.load_for_game(loaded.game.as_deref());
                    // The field-value index is bound to the old entry set.
                    self.field_index.invalidate();
                    self.source = Some(loaded);
                    // New entry universe — invalidate any cached search results.
                    self.source_generation = self.source_generation.wrapping_add(1);
                    self.finish_pending_session_restore(ctx.clone());
                }
                WorkerMessage::SourceLoaded {
                    result: Err(error), ..
                } => {
                    self.pending_session_restore = None;
                    self.status = error;
                }
                WorkerMessage::TagLoaded { key, result } => {
                    self.loading_tags.remove(&key);
                    if !self.open_tabs.iter().any(|tab| tab == &key) {
                        continue;
                    }
                    match result {
                        Ok(tag) => {
                            self.status = "Tag loaded".to_owned();
                            self.remember_tag_use(&key);
                            self.parsed_tags.insert(key, TagDocument::clean(tag));
                            self.trim_tag_memory();
                        }
                        Err(error) => {
                            self.terminal.lines.push(TerminalLineEntry::new(format!(
                                "Folder refactor failed: {error}"
                            )));
                            trim_terminal_lines(&mut self.terminal.lines);
                            self.terminal.scroll_to_bottom = true;
                            self.status = error;
                        }
                    }
                }
                WorkerMessage::BitmapReimportFinished { key, result } => {
                    self.terminal.running = false;
                    self.terminal.running_id = None;
                    self.terminal.running_command = None;
                    self.terminal.process = None;
                    self.terminal.scroll_to_bottom = true;
                    self.terminal.refocus_input = true;
                    match result {
                        Ok(tag) => {
                            if self.open_tabs.iter().any(|tab| tab == &key) {
                                self.parsed_tags
                                    .insert(key.clone(), TagDocument::clean(tag));
                                self.bitmap_previews.remove(&key);
                                self.remember_tag_use(&key);
                                self.trim_tag_memory();
                            }
                            self.status = "Bitmap reimported and reloaded".to_owned();
                        }
                        Err(error) => {
                            self.status = format!("Bitmap reimport failed: {error}");
                        }
                    }
                }
                WorkerMessage::ExportFinished(result) => {
                    self.status = match result {
                        Ok(message) => message,
                        Err(error) => error,
                    };
                }
                WorkerMessage::FolderRefactorProgress(progress) => {
                    self.folder_refactor = Some(FolderRefactorUiState {
                        label: progress.label.clone(),
                        phase: progress.phase.clone(),
                        progress: progress.progress,
                    });
                    self.status = format!("{}: {}", progress.label, progress.phase);
                }
                WorkerMessage::FolderRefactorFinished(result) => {
                    self.folder_refactor = None;
                    match result {
                        Ok(done) => {
                            if let Some(source) = self.source.as_mut() {
                                source.entries.clear();
                                source.all_entries = done.all_entries;
                                source.tree = done.tree;
                                source.group_tree =
                                    crate::source::build_group_tree(&source.all_entries);
                                source.reverse_dependencies = done.reverse_dependencies;
                                if let TagSource::LooseFolder { root, .. } = &source.source {
                                    if !source.all_entries.is_empty()
                                        && let Some(game) = source.game.as_deref()
                                    {
                                        let _ = crate::source::save_entry_index(
                                            game,
                                            root,
                                            &source.all_entries,
                                        );
                                    }
                                    if let (Some(game), Some(reverse_dependencies)) = (
                                        source.game.as_deref(),
                                        source.reverse_dependencies.as_ref(),
                                    ) {
                                        let _ = crate::source::save_reverse_dependency_index(
                                            game,
                                            root,
                                            reverse_dependencies,
                                        );
                                    }
                                }
                            }
                            if done.moved {
                                remap_open_tag_keys(&mut self.open_tabs, &done.old_to_new_keys);
                                remap_hashset_keys(&mut self.floating_tabs, &done.old_to_new_keys);
                                if let Some(selected) = self.selected_key.clone()
                                    && let Some(new_key) = done.old_to_new_keys.get(&selected)
                                {
                                    self.selected_key = Some(new_key.clone());
                                }
                            }
                            self.parsed_tags.clear();
                            self.loading_tags.clear();
                            self.tag_cache_order.clear();
                            self.bitmap_previews.clear();
                            self.model_previews.clear();
                            self.edit_buffers.clear();
                            self.field_search.clear();
                            self.field_search_applied.clear();
                            self.source_generation = self.source_generation.wrapping_add(1);
                            self.terminal
                                .lines
                                .extend(done.lines.into_iter().map(TerminalLineEntry::new));
                            trim_terminal_lines(&mut self.terminal.lines);
                            self.terminal.scroll_to_bottom = true;
                            self.status = done.status;
                        }
                        Err(error) => {
                            self.status = error;
                        }
                    }
                }
                WorkerMessage::AllEntriesScanned(result) => {
                    self.scanning_entries = false;
                    match result {
                        Ok(scanned) => {
                            if let Some(source) = self.source.as_mut() {
                                let n = scanned.len();
                                source.group_tree = crate::source::build_group_tree(&scanned);
                                source.all_entries = scanned;
                                // The full index just landed — cached search
                                // results were built against the partial set.
                                self.source_generation = self.source_generation.wrapping_add(1);
                                self.status = format!("Index complete: {n} tags");
                                // Persist the index in the background so the
                                // next launch can skip the scan entirely.
                                if let (Some(game), TagSource::LooseFolder { root, .. }) =
                                    (source.game.clone(), &source.source)
                                {
                                    let root = root.clone();
                                    let entries = source.all_entries.clone();
                                    thread::spawn(move || {
                                        if let Err(e) =
                                            crate::source::save_entry_index(&game, &root, &entries)
                                        {
                                            eprintln!("index save failed: {e}");
                                        }
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            self.status = format!("Scan failed: {e}");
                        }
                    }
                }
            }
        }
    }

    pub(super) fn begin_load_single(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new().set_title("Load Tag").pick_file() else {
            return;
        };
        self.begin_load_single_path(path, ctx);
    }

    pub(super) fn begin_load_single_path(&mut self, path: PathBuf, ctx: egui::Context) {
        let tx = self.tx.clone();
        let names = self.default_names.clone();
        self.status = format!("Loading {}", path.display());
        thread::spawn(move || {
            let result = load_single_file(path, &names).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::SourceLoaded {
                result,
                recent_path: None,
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_load_folder(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Load Folder")
            .pick_folder()
        else {
            return;
        };
        self.begin_load_folder_path(path, ctx);
    }

    pub(super) fn begin_load_folder_path(&mut self, path: PathBuf, ctx: egui::Context) {
        let tx = self.tx.clone();
        let names = self.default_names.clone();
        let definitions_root = locate_definitions_root();
        let ek_folder_aliases = self.ek_folder_aliases.clone();
        let folder_info = match resolve_folder_root(&path, &ek_folder_aliases) {
            Ok(info) => info,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        self.status = match folder_info.game {
            Some(game) => format!("Indexing {} as {game}", folder_info.scan_root.display()),
            None => format!("Indexing {}", folder_info.scan_root.display()),
        };
        let recent_path = clean_recent_path(path.clone());
        thread::spawn(move || {
            let result = load_folder(path, &names, &definitions_root, &ek_folder_aliases)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::SourceLoaded {
                result,
                recent_path: Some(recent_path),
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_load_monolithic(&mut self, ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Load Monolithic blob_index.dat")
            .add_filter("blob index", &["dat"])
            .pick_file()
        else {
            return;
        };
        self.begin_load_monolithic_path(path, ctx);
    }

    pub(super) fn begin_load_monolithic_path(&mut self, path: PathBuf, ctx: egui::Context) {
        let tx = self.tx.clone();
        let names = self.default_names.clone();
        self.status = format!("Opening {}", path.display());
        let recent_path = clean_recent_path(path.clone());
        thread::spawn(move || {
            let result = load_monolithic_blob_index(path, &names).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::SourceLoaded {
                result,
                recent_path: Some(recent_path),
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn load_recent_folder(&mut self, path: PathBuf, ctx: egui::Context) {
        if !path.exists() {
            self.status = format!("Folder not found: {}", path.display());
            self.remove_recent_folder(&path);
            return;
        }
        if path.is_dir() {
            self.begin_load_folder_path(path, ctx);
        } else {
            self.begin_load_monolithic_path(path, ctx);
        }
    }

    pub(super) fn remember_recent_folder(&mut self, path: PathBuf) {
        let path = clean_recent_path(path);
        self.recent_folders
            .retain(|existing| !same_recent_path(existing, &path));
        self.recent_folders.insert(0, path);
        self.recent_folders.truncate(MAX_RECENT_FOLDERS);
    }

    pub(super) fn remove_recent_folder(&mut self, path: &Path) {
        self.recent_folders
            .retain(|existing| !same_recent_path(existing, path));
    }

    pub(super) fn open_new_tag_dialog(&mut self) {
        let default_game = self
            .source
            .as_ref()
            .and_then(|source| source.game.as_deref())
            .unwrap_or("halo3_mcc")
            .to_owned();
        self.new_tag_dialog = NewTagDialog {
            game: default_game,
            rel_path: String::new(),
            output_path: None,
            groups: Vec::new(),
            selected_group: 0,
            error: None,
        };
        self.refresh_new_tag_groups();
        self.new_tag_open = true;
    }

    pub(super) fn refresh_new_tag_groups(&mut self) {
        match load_new_tag_groups(&self.new_tag_dialog.game) {
            Ok(groups) if groups.is_empty() => {
                self.new_tag_dialog.groups = groups;
                self.new_tag_dialog.selected_group = 0;
                self.new_tag_dialog.error = Some(format!(
                    "No tag schemas found for {}",
                    self.new_tag_dialog.game
                ));
            }
            Ok(groups) => {
                self.new_tag_dialog.groups = groups;
                self.new_tag_dialog.selected_group = self
                    .new_tag_dialog
                    .selected_group
                    .min(self.new_tag_dialog.groups.len() - 1);
                self.new_tag_dialog.rel_path.clear();
                self.new_tag_dialog.output_path = None;
                self.new_tag_dialog.error = None;
            }
            Err(error) => {
                self.new_tag_dialog.groups.clear();
                self.new_tag_dialog.selected_group = 0;
                self.new_tag_dialog.rel_path.clear();
                self.new_tag_dialog.output_path = None;
                self.new_tag_dialog.error = Some(error);
            }
        }
    }

    pub(super) fn choose_new_tag_output_path(&mut self) {
        let Some(root) = self.loaded_tags_root() else {
            self.new_tag_dialog.error =
                Some("Load a loose editing-kit tags folder before creating a tag".to_owned());
            return;
        };
        let Some(group) = self
            .new_tag_dialog
            .groups
            .get(self.new_tag_dialog.selected_group)
            .cloned()
        else {
            self.new_tag_dialog.error = Some("Choose a tag group".to_owned());
            return;
        };

        let mut dialog = rfd::FileDialog::new()
            .set_title(format!("Create New {}", group.name))
            .set_directory(&root)
            .set_file_name(format!("new_tag.{}", group.extension))
            .add_filter(
                format!("{} tag", group.extension),
                &[group.extension.as_str()],
            );
        if let Some(output) = self.new_tag_dialog.output_path.as_ref()
            && let Some(parent) = output.parent()
        {
            dialog = dialog.set_directory(parent);
        }
        let Some(picked) = dialog.save_file() else {
            return;
        };
        match new_tag_output_path_from_dialog(&root, &picked, &group.extension) {
            Ok((output, rel_path)) => {
                self.new_tag_dialog.output_path = Some(output);
                self.new_tag_dialog.rel_path = rel_path;
                self.new_tag_dialog.error = None;
            }
            Err(error) => {
                self.new_tag_dialog.output_path = None;
                self.new_tag_dialog.rel_path.clear();
                self.new_tag_dialog.error = Some(error);
            }
        }
    }

    pub(super) fn create_new_tag(&mut self) {
        let Some(root) = self.loaded_tags_root() else {
            self.new_tag_dialog.error =
                Some("Load a loose editing-kit tags folder before creating a tag".to_owned());
            return;
        };
        let Some(group) = self
            .new_tag_dialog
            .groups
            .get(self.new_tag_dialog.selected_group)
            .cloned()
        else {
            self.new_tag_dialog.error = Some("Choose a tag group".to_owned());
            return;
        };
        let Some(output) = self.new_tag_dialog.output_path.clone() else {
            self.new_tag_dialog.error = Some("Choose a tag name and location".to_owned());
            return;
        };
        let output = match new_tag_output_path_from_dialog(&root, &output, &group.extension) {
            Ok((output, rel_path)) => {
                self.new_tag_dialog.rel_path = rel_path;
                output
            }
            Err(error) => {
                self.new_tag_dialog.error = Some(error);
                return;
            }
        };
        if output.exists() {
            self.new_tag_dialog.error = Some(format!("{} already exists", output.display()));
            return;
        }
        let tag = match TagFile::new(&group.schema_path) {
            Ok(tag) => tag,
            Err(error) => {
                self.new_tag_dialog.error = Some(format!("Could not create tag: {error}"));
                return;
            }
        };
        if let Some(parent) = output.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            self.new_tag_dialog.error =
                Some(format!("Could not create {}: {error}", parent.display()));
            return;
        }
        if let Err(error) = tag.write_atomic(&output) {
            self.new_tag_dialog.error =
                Some(format!("Could not write {}: {error}", output.display()));
            return;
        }

        let display_path = output
            .strip_prefix(&root)
            .unwrap_or(output.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        let key = display_path.clone();
        let entry = TagEntry {
            key: key.clone(),
            display_path,
            group_tag: group.group_tag,
            group_name: Some(group.name.clone()),
            location: TagEntryLocation::LooseFile(output.clone()),
        };
        self.register_created_tag(entry, tag);
        self.new_tag_open = false;
        self.status = format!("Created {}", output.display());
    }

    pub(super) fn loaded_tags_root(&self) -> Option<PathBuf> {
        let TagSource::LooseFolder { root, .. } = &self.source.as_ref()?.source else {
            return None;
        };
        Some(root.clone())
    }

    pub(super) fn open_dropped_files(&mut self, paths: Vec<PathBuf>, ctx: egui::Context) {
        if paths.is_empty() {
            return;
        }

        let count = paths.len();
        for path in paths {
            match self.open_dropped_file(path, ctx.clone()) {
                Ok(true) => return,
                Ok(false) => {}
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        }

        self.status = if count == 1 {
            "Dropped file is not a supported tag".to_owned()
        } else {
            "No supported tag files were dropped".to_owned()
        };
    }

    fn open_dropped_file(&mut self, path: PathBuf, ctx: egui::Context) -> Result<bool, String> {
        if !path.is_file() {
            return Ok(false);
        }

        let Some(source) = self.source.as_ref() else {
            return Err("Load an editing-kit tags folder before dropping tag files".to_owned());
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return Err("Drop-to-open requires a loaded loose tags folder".to_owned());
        };

        let root = fs::canonicalize(root)
            .map_err(|error| format!("Could not resolve loaded tags folder: {error}"))?;
        let path = fs::canonicalize(&path)
            .map_err(|error| format!("Could not resolve dropped file: {error}"))?;
        if !path.starts_with(&root) {
            return Err(format!(
                "Dropped tag must be inside the loaded tags folder: {}",
                root.display()
            ));
        }

        if let Some(key) = self.key_for_loose_path(&path) {
            self.select_entry(key, ctx);
            return Ok(true);
        }

        let Some(entry) = loose_file_entry(&root, &path, &source.names)
            .map_err(|error| format!("Could not inspect dropped tag: {error:#}"))?
        else {
            return Ok(false);
        };

        let key = entry.key.clone();
        if let Some(source) = self.source.as_mut() {
            source.entries.retain(|existing| existing.key != key);
            source.entries.push(entry.clone());

            if !source.all_entries.is_empty() {
                source.all_entries.retain(|existing| existing.key != key);
                source.all_entries.push(entry.clone());
                source
                    .all_entries
                    .sort_by(|a, b| a.display_path.cmp(&b.display_path));
                source.group_tree = crate::source::build_group_tree(&source.all_entries);
                if let (Some(game), TagSource::LooseFolder { root, .. }) =
                    (source.game.as_deref(), &source.source)
                {
                    let _ = crate::source::save_entry_index(game, root, &source.all_entries);
                }
            }
        }
        self.source_generation = self.source_generation.wrapping_add(1);
        self.select_entry(key, ctx);
        Ok(true)
    }

    fn key_for_loose_path(&self, path: &Path) -> Option<String> {
        let source = self.source.as_ref()?;
        source
            .entries
            .iter()
            .chain(source.all_entries.iter())
            .find_map(|entry| {
                let TagEntryLocation::LooseFile(existing) = &entry.location else {
                    return None;
                };
                if existing == path || fs::canonicalize(existing).ok().as_deref() == Some(path) {
                    Some(entry.key.clone())
                } else {
                    None
                }
            })
    }

    pub(super) fn register_created_tag(&mut self, entry: TagEntry, tag: TagFile) {
        let key = entry.key.clone();
        if let Some(source) = self.source.as_mut() {
            source.entries.retain(|existing| existing.key != key);
            source.entries.push(entry.clone());
            source.all_entries.retain(|existing| existing.key != key);
            source.all_entries.push(entry.clone());
            if let TagSource::LooseFolder { root, .. } = &source.source {
                if let Ok(tree) = crate::source::build_folder_directory_tree(root) {
                    source.tree = tree;
                }
                source.group_tree = crate::source::build_group_tree(&source.all_entries);
                if let Some(game) = source.game.as_deref() {
                    let _ = crate::source::save_entry_index(game, root, &source.all_entries);
                }
            }
        }
        self.source_generation = self.source_generation.wrapping_add(1);
        self.parsed_tags
            .insert(key.clone(), TagDocument::clean(tag));
        if !self.open_tabs.iter().any(|tab| tab == &key) {
            self.open_tabs.push(key.clone());
        }
        self.selected_key = Some(key.clone());
        self.remember_tag_use(&key);
        self.trim_open_tabs();
    }

    /// Trigger a background full recursive scan of a LooseFolder source so
    /// that Groups mode and search work without needing to expand every tree
    /// node first. No-op if already scanning or source is not a LooseFolder.
    pub(super) fn begin_scan_all_entries(&mut self, ctx: egui::Context) {
        if self.scanning_entries {
            return;
        }
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return; // monolithic/single-file already have all entries
        };
        let root = root.clone();
        let names = source.names.clone();
        let tx = self.tx.clone();
        self.scanning_entries = true;
        self.status = "Indexing tags…".to_owned();
        thread::spawn(move || {
            let result = scan_folder_subtree_entries(&root, std::path::Path::new(""), &names)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::AllEntriesScanned(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_check_for_updates(&mut self, ctx: egui::Context) {
        self.status = "Checking for updates...".to_owned();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = fetch_latest_release();
            let _ = tx.send(WorkerMessage::UpdateCheckFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_terminal_command(&mut self, ctx: egui::Context) {
        let command = self.terminal.input.trim().to_owned();
        if command.is_empty() {
            return;
        }
        self.submit_terminal_command(command, ctx);
    }

    pub(super) fn submit_terminal_command(&mut self, command: String, ctx: egui::Context) {
        if self.terminal.history.last() != Some(&command) {
            self.terminal.history.push(command.clone());
        }
        self.terminal.history_cursor = None;
        self.terminal.input.clear();
        self.terminal.refocus_input = true;
        self.spawn_terminal_command(command, ctx);
    }

    pub(super) fn recall_terminal_history(&mut self, delta: i32) {
        let len = self.terminal.history.len();
        if len == 0 {
            return;
        }

        let next = match self.terminal.history_cursor {
            Some(index) => index as i32 + delta,
            None if delta < 0 => len as i32 - 1,
            None => return,
        };

        if next < 0 {
            self.terminal.history_cursor = Some(0);
            self.terminal.input = self.terminal.history[0].clone();
        } else if next >= len as i32 {
            self.terminal.history_cursor = None;
            self.terminal.input.clear();
        } else {
            let next = next as usize;
            self.terminal.history_cursor = Some(next);
            self.terminal.input = self.terminal.history[next].clone();
        }
    }

    /// Run `command` in the editing-kit root, streaming output to the terminal
    /// panel. Shared by the terminal input and the geometry Import button.
    pub(super) fn spawn_terminal_command(&mut self, command: String, ctx: egui::Context) {
        if self.terminal.running {
            self.status = "A command is already running".to_owned();
            return;
        }
        let Some(work_dir) = self.terminal_work_dir.clone() else {
            self.status = "Run requires a loaded editing-kit folder".to_owned();
            return;
        };
        self.terminal_open = true;
        self.terminal
            .lines
            .push(TerminalLineEntry::new(format!("> {command}")));
        trim_terminal_lines(&mut self.terminal.lines);
        self.terminal.scroll_to_bottom = true;
        self.terminal.refocus_input = true;
        self.terminal.running = true;
        let run_id = self.terminal.next_run_id;
        self.terminal.next_run_id = self.terminal.next_run_id.wrapping_add(1).max(1);
        self.terminal.running_id = Some(run_id);
        self.terminal.running_command = Some(command.clone());
        let mut log_file = match create_terminal_log_file(run_id, &command) {
            Ok((path, file)) => {
                self.terminal.last_log_path = Some(path);
                Some(file)
            }
            Err(error) => {
                self.status = format!("Terminal full log unavailable: {error}");
                self.terminal.last_log_path = None;
                None
            }
        };
        let tx = self.tx.clone();
        let child_slot: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));
        let stop_requested = Arc::new(AtomicBool::new(false));
        self.terminal.process = Some(TerminalProcess {
            child: Arc::clone(&child_slot),
            stop_requested: Arc::clone(&stop_requested),
        });
        thread::spawn(move || {
            let mut log_error_reported = false;
            #[cfg(target_os = "windows")]
            let mut cmd = {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                let mut c = std::process::Command::new("cmd");
                c.creation_flags(CREATE_NO_WINDOW);
                c.args(["/C", &format!("{command} 2>&1")]);
                c
            };
            #[cfg(not(target_os = "windows"))]
            let mut cmd = {
                #[cfg(unix)]
                use std::os::unix::process::CommandExt;
                let mut c = std::process::Command::new("sh");
                c.args(["-c", &command]);
                #[cfg(unix)]
                c.process_group(0);
                c
            };
            cmd.current_dir(&work_dir)
                .stdout(std::process::Stdio::piped())
                .stdin(std::process::Stdio::null());
            match cmd.spawn() {
                Err(e) => {
                    send_terminal_line(
                        &tx,
                        &ctx,
                        &mut log_file,
                        &mut log_error_reported,
                        format!("[error] {e}"),
                    );
                    let _ = tx.send(WorkerMessage::TerminalDone { run_id });
                    ctx.request_repaint();
                }
                Ok(child) => {
                    let stdout = match child_slot.lock() {
                        Ok(mut slot) => {
                            *slot = Some(child);
                            slot.as_mut().and_then(|child| child.stdout.take())
                        }
                        Err(_) => {
                            send_terminal_line(
                                &tx,
                                &ctx,
                                &mut log_file,
                                &mut log_error_reported,
                                "[error] terminal process lock was poisoned".to_owned(),
                            );
                            let _ = tx.send(WorkerMessage::TerminalDone { run_id });
                            ctx.request_repaint();
                            return;
                        }
                    };
                    if let Some(stdout) = stdout {
                        let _ = stream_terminal_output(
                            stdout,
                            &tx,
                            &ctx,
                            &mut log_file,
                            &mut log_error_reported,
                        );
                    }
                    let exit = match child_slot.lock() {
                        Ok(mut slot) => {
                            if let Some(mut child) = slot.take() {
                                child.wait().ok()
                            } else {
                                None
                            }
                        }
                        Err(_) => {
                            send_terminal_line(
                                &tx,
                                &ctx,
                                &mut log_file,
                                &mut log_error_reported,
                                "[error] terminal process lock was poisoned".to_owned(),
                            );
                            None
                        }
                    };
                    if let Some(code) = exit.and_then(|status| status.code())
                        && !stop_requested.load(Ordering::SeqCst)
                    {
                        send_terminal_line(
                            &tx,
                            &ctx,
                            &mut log_file,
                            &mut log_error_reported,
                            format!("[exit {code}]"),
                        );
                    }
                    let _ = tx.send(WorkerMessage::TerminalDone { run_id });
                    ctx.request_repaint();
                }
            }
        });
    }

    pub(super) fn stop_terminal_command(&mut self) {
        if !self.terminal.running {
            self.status = "No terminal command is running".to_owned();
            return;
        }
        let Some(process) = self.terminal.process.as_ref() else {
            self.status = "No tracked terminal process to stop".to_owned();
            return;
        };

        process.stop_requested.store(true, Ordering::SeqCst);
        let command = self
            .terminal
            .running_command
            .clone()
            .unwrap_or_else(|| "command".to_owned());
        match stop_terminal_process(process) {
            Ok(TerminalStopResult::Stopped) => {
                let line = format!("[stopped] {command} stopped by user");
                let mut log_status = None;
                if let Some(path) = self.terminal.last_log_path.as_ref()
                    && let Err(error) = append_terminal_log_path(path, &line)
                {
                    log_status = Some(error);
                }
                self.terminal.lines.push(TerminalLineEntry::new(line));
                trim_terminal_lines(&mut self.terminal.lines);
                self.finish_stopped_terminal_command();
                self.status = log_status.unwrap_or_else(|| "Terminal command stopped".to_owned());
            }
            Ok(TerminalStopResult::AlreadyExited) => {
                self.finish_stopped_terminal_command();
                self.status = "Terminal command had already exited".to_owned();
            }
            Err(error) => {
                let line = format!("[error] could not stop terminal command: {error}");
                let mut log_status = None;
                if let Some(path) = self.terminal.last_log_path.as_ref()
                    && let Err(log_error) = append_terminal_log_path(path, &line)
                {
                    log_status = Some(log_error);
                }
                self.terminal.lines.push(TerminalLineEntry::new(line));
                trim_terminal_lines(&mut self.terminal.lines);
                self.terminal.scroll_to_bottom = true;
                self.status = log_status
                    .unwrap_or_else(|| format!("Could not stop terminal command: {error}"));
            }
        }
    }

    fn finish_stopped_terminal_command(&mut self) {
        self.terminal.running = false;
        self.terminal.running_id = None;
        self.terminal.running_command = None;
        self.terminal.process = None;
        self.terminal.scroll_to_bottom = true;
        self.terminal.refocus_input = true;
    }

    pub(super) fn select_entry(&mut self, key: String, ctx: egui::Context) {
        if !self.open_tabs.iter().any(|tab| tab == &key) {
            self.open_tabs.push(key.clone());
        }
        self.selected_key = Some(key.clone());
        self.remember_tag_use(&key);
        self.trim_open_tabs();
        self.ensure_tag_loading(key, ctx);
    }

    pub(super) fn ensure_tag_loading(&mut self, key: String, ctx: egui::Context) {
        if self.parsed_tags.contains_key(&key) || self.loading_tags.contains(&key) {
            self.remember_tag_use(&key);
            return;
        }
        let Some(source) = self.source.as_ref() else {
            return;
        };
        // Check both the lazily-loaded entries and the full scan set (all_entries).
        // Flat search results reference all_entries, which may not overlap with entries.
        let Some(entry) = source
            .entries
            .iter()
            .chain(source.all_entries.iter())
            .find(|e| e.key == key)
            .cloned()
        else {
            return;
        };
        let source_kind = source.source.clone();
        let tx = self.tx.clone();
        self.loading_tags.insert(key.clone());
        self.status = format!("Loading {}", entry.display_path);
        thread::spawn(move || {
            let result = read_entry(&source_kind, &entry).map_err(|error| format!("{error:#}"));
            let _ = tx.send(WorkerMessage::TagLoaded { key, result });
            ctx.request_repaint();
        });
    }

    pub(super) fn selected_entry(&self) -> Option<&TagEntry> {
        let key = self.selected_key.as_ref()?;
        self.entry_for_key(key)
    }

    pub(super) fn entry_for_key(&self, key: &str) -> Option<&TagEntry> {
        let source = self.source.as_ref()?;
        source
            .entries
            .iter()
            .chain(source.all_entries.iter())
            .find(|entry| entry.key == key)
    }

    pub(super) fn close_tab(&mut self, key: &str) {
        let removed_index = self.open_tabs.iter().position(|tab| tab == key);
        self.open_tabs.retain(|tab| tab != key);
        self.floating_tabs.remove(key);
        self.unload_tag(key);
        if self.selected_key.as_deref() == Some(key) {
            self.selected_key = selected_tab_after_removal(&self.open_tabs, removed_index);
        }
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn request_close_action(&mut self, action: PendingCloseAction, ctx: &egui::Context) {
        if self.save_changes_prompt.visible {
            return;
        }
        let dirty_tags = self.dirty_tags_for_close_action(&action);
        if dirty_tags.is_empty() {
            self.execute_close_action(action, ctx);
            return;
        }
        self.save_changes_prompt = SaveChangesPrompt {
            visible: true,
            dirty_tags,
            pending_action: action,
            error: None,
            allow_app_close_once: self.save_changes_prompt.allow_app_close_once,
        };
    }

    /// Native app close is a two-step flow in eframe 0.29: when the OS close
    /// request arrives, Baboon sends `CancelClose` to veto it, shows the shared
    /// save prompt, then re-issues `ViewportCommand::Close` only after the user
    /// chooses Save or Don't Save. `allow_app_close_once` lets that confirmed
    /// second close request pass without opening the prompt again.
    pub(super) fn handle_app_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        if self.save_changes_prompt.allow_app_close_once {
            self.save_changes_prompt.allow_app_close_once = false;
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        if self.save_changes_prompt.visible {
            return;
        }
        self.request_close_action(PendingCloseAction::CloseApp, ctx);
    }

    fn dirty_tags_for_close_action(&self, action: &PendingCloseAction) -> Vec<DirtyTagEntry> {
        self.close_action_tag_keys(action)
            .into_iter()
            .filter_map(|key| {
                let doc = self.parsed_tags.get(&key)?;
                if !doc.dirty {
                    return None;
                }
                Some(DirtyTagEntry {
                    path: self.tag_path_label(&key),
                    tag_id: key,
                    checked: true,
                })
            })
            .collect()
    }

    fn close_action_tag_keys(&self, action: &PendingCloseAction) -> Vec<String> {
        match action {
            PendingCloseAction::CloseApp | PendingCloseAction::CloseAllTabs => {
                ordered_unique_keys(self.open_tabs.iter().chain(self.floating_tabs.iter()))
            }
            PendingCloseAction::CloseTab(key) => vec![key.clone()],
            PendingCloseAction::CloseAllButThis(kept_key) => ordered_unique_keys(
                self.open_tabs
                    .iter()
                    .chain(self.floating_tabs.iter())
                    .filter(|key| *key != kept_key),
            ),
        }
    }

    fn tag_path_label(&self, key: &str) -> String {
        let Some(entry) = self.entry_for_key(key) else {
            return key.to_owned();
        };
        match &entry.location {
            TagEntryLocation::LooseFile(path) => path.display().to_string(),
            TagEntryLocation::Monolithic { .. } => entry.display_path.clone(),
        }
    }

    fn execute_close_action(&mut self, action: PendingCloseAction, ctx: &egui::Context) {
        match action {
            PendingCloseAction::CloseApp => {
                if let Some(session) = self.current_session_state() {
                    let _ = save_last_session(&session);
                } else {
                    clear_last_session();
                }
                self.save_changes_prompt.allow_app_close_once = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            PendingCloseAction::CloseTab(key) => self.close_tab(&key),
            PendingCloseAction::CloseAllTabs => self.close_all_tabs(),
            PendingCloseAction::CloseAllButThis(key) => self.close_all_tabs_but(&key),
        }
    }

    fn current_session_state(&self) -> Option<LastSessionState> {
        let source = self.source.as_ref()?;
        let (source_kind, source_path) = match &source.source {
            TagSource::SingleFile { path } => (LastSessionSourceKind::SingleFile, path.clone()),
            TagSource::LooseFolder { root, .. } => {
                (LastSessionSourceKind::LooseFolder, root.clone())
            }
            TagSource::MonolithicCache { root, .. } => {
                (LastSessionSourceKind::MonolithicCache, root.clone())
            }
        };
        let mut tags = Vec::new();
        for key in ordered_unique_keys(self.open_tabs.iter().chain(self.floating_tabs.iter())) {
            let Some(entry) = self.entry_for_key(&key) else {
                continue;
            };
            let path = match &entry.location {
                TagEntryLocation::LooseFile(path) => Some(path.clone()),
                TagEntryLocation::Monolithic { .. } => None,
            };
            tags.push(LastSessionTag {
                key: entry.key.clone(),
                label: format!(
                    "{} - {}",
                    entry.display_path,
                    group_label(&self.names, entry.group_tag)
                ),
                group_tag: entry.group_tag,
                path,
            });
        }
        if tags.is_empty() {
            return None;
        }
        Some(LastSessionState {
            source_kind,
            source_path,
            game: source.game.clone(),
            tags,
        })
    }

    pub(super) fn begin_last_session_restore(
        &mut self,
        source_kind: LastSessionSourceKind,
        source_path: PathBuf,
        tags: Vec<LastSessionTag>,
        ctx: egui::Context,
    ) {
        if tags.is_empty() {
            return;
        }
        self.pending_session_restore = Some(PendingSessionRestore { tags });
        match source_kind {
            LastSessionSourceKind::SingleFile => self.begin_load_single_path(source_path, ctx),
            LastSessionSourceKind::LooseFolder => self.begin_load_folder_path(source_path, ctx),
            LastSessionSourceKind::MonolithicCache => {
                let blob_index = if source_path.is_dir() {
                    source_path.join("blob_index.dat")
                } else {
                    source_path
                };
                self.begin_load_monolithic_path(blob_index, ctx);
            }
        }
    }

    fn finish_pending_session_restore(&mut self, ctx: egui::Context) {
        let Some(restore) = self.pending_session_restore.take() else {
            return;
        };
        let mut opened = 0usize;
        let mut missing = 0usize;
        for tag in restore.tags {
            if self.ensure_restored_tag_entry(&tag) {
                self.select_entry(tag.key, ctx.clone());
                opened += 1;
            } else {
                missing += 1;
            }
        }
        if opened > 0 {
            self.status = if missing > 0 {
                format!("Restored {opened} window(s); skipped {missing} missing item(s)")
            } else {
                format!("Restored {opened} window(s)")
            };
        } else if missing > 0 {
            self.status = "No saved windows could be restored".to_owned();
        }
    }

    fn ensure_restored_tag_entry(&mut self, tag: &LastSessionTag) -> bool {
        if self.entry_for_key(&tag.key).is_some() {
            return true;
        }
        let Some(path) = tag.path.as_ref() else {
            return false;
        };
        if !path.is_file() {
            return false;
        }
        let Some(source) = self.source.as_ref() else {
            return false;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return false;
        };
        let Ok(root) = fs::canonicalize(root) else {
            return false;
        };
        let Ok(path) = fs::canonicalize(path) else {
            return false;
        };
        if !path.starts_with(&root) {
            return false;
        }
        let Ok(entry) = loose_file_entry(&root, &path, &source.names) else {
            return false;
        };
        let Some(entry) = entry else {
            return false;
        };
        if let Some(source) = self.source.as_mut() {
            source.entries.retain(|existing| existing.key != tag.key);
            source.entries.push(entry.clone());
            if !source.all_entries.is_empty() {
                source
                    .all_entries
                    .retain(|existing| existing.key != tag.key);
                source.all_entries.push(entry);
                source
                    .all_entries
                    .sort_by(|a, b| a.display_path.cmp(&b.display_path));
                source.group_tree = crate::source::build_group_tree(&source.all_entries);
            }
        }
        self.source_generation = self.source_generation.wrapping_add(1);
        true
    }

    pub(super) fn pop_tab(&mut self, key: &str) {
        let removed_index = self.open_tabs.iter().position(|tab| tab == key);
        self.open_tabs.retain(|tab| tab != key);
        self.floating_tabs.insert(key.to_owned());
        if self.selected_key.as_deref() == Some(key) {
            self.selected_key = selected_tab_after_removal(&self.open_tabs, removed_index);
        }
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn dock_tab(&mut self, key: &str) {
        self.floating_tabs.remove(key);
        if !self.open_tabs.iter().any(|tab| tab == key) {
            self.open_tabs.push(key.to_owned());
        }
        self.selected_key = Some(key.to_owned());
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn handle_floating_tab_drop(&mut self, ctx: &egui::Context) {
        let Some(key) = self.dragging_floating_tab.clone() else {
            return;
        };
        let Some(rack_rect) = self.tab_rack_rect else {
            return;
        };
        let release = ctx.input(|input| {
            input
                .pointer
                .interact_pos()
                .map(|pos| (input.pointer.any_released(), pos))
        });
        let Some((released, pos)) = release else {
            return;
        };
        if !released {
            return;
        }
        if rack_rect.contains(pos) {
            self.dock_tab(&key);
        }
        self.dragging_floating_tab = None;
    }

    pub(super) fn close_all_tabs(&mut self) {
        self.open_tabs.clear();
        self.floating_tabs.clear();
        self.parsed_tags.clear();
        self.loading_tags.clear();
        self.tag_cache_order.clear();
        self.bitmap_previews.clear();
        self.edit_buffers.clear();
        self.selected_key = None;
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn close_all_tabs_but(&mut self, key: &str) {
        self.open_tabs.retain(|tab| tab == key);
        self.floating_tabs.retain(|tab| tab == key);
        self.parsed_tags.retain(|tab, _| tab == key);
        self.loading_tags.retain(|tab| tab == key);
        self.tag_cache_order.retain(|tab| tab == key);
        self.bitmap_previews.retain(|tab, _| tab == key);
        let edit_prefix = format!("{key}|");
        self.edit_buffers
            .retain(|buffer_key, _| buffer_key.starts_with(&edit_prefix));
        self.selected_key = Some(key.to_owned());
        self.color_popup = None;
        self.function_popup = None;
    }

    pub(super) fn unload_tag(&mut self, key: &str) {
        self.parsed_tags.remove(key);
        self.loading_tags.remove(key);
        self.bitmap_previews.remove(key);
        let edit_prefix = format!("{key}|");
        self.edit_buffers
            .retain(|buffer_key, _| !buffer_key.starts_with(&edit_prefix));
        self.tag_cache_order.retain(|tab| tab != key);
    }

    pub(super) fn remember_tag_use(&mut self, key: &str) {
        self.tag_cache_order.retain(|tab| tab != key);
        self.tag_cache_order.push_back(key.to_owned());
    }

    pub(super) fn trim_open_tabs(&mut self) {
        while self.open_tabs.len() > MAX_OPEN_TABS {
            let removable = self.open_tabs.iter().position(|tab| {
                Some(tab.as_str()) != self.selected_key.as_deref()
                    && !self
                        .parsed_tags
                        .get(tab)
                        .map(|doc| doc.dirty)
                        .unwrap_or(false)
            });
            let Some(removable) = removable else {
                break;
            };
            let key = self.open_tabs.remove(removable);
            self.floating_tabs.remove(&key);
            self.unload_tag(&key);
        }
    }

    pub(super) fn trim_tag_memory(&mut self) {
        let open_tabs = self.open_tabs.iter().cloned().collect::<HashSet<_>>();
        self.bitmap_previews
            .retain(|key, _| open_tabs.contains(key));

        let mut attempts = self.tag_cache_order.len();
        while self.parsed_tags.len() > MAX_PARSED_TAGS && attempts > 0 {
            attempts -= 1;
            let Some(key) = self.tag_cache_order.pop_front() else {
                break;
            };
            if Some(key.as_str()) == self.selected_key.as_deref() {
                self.tag_cache_order.push_back(key);
                continue;
            }
            self.parsed_tags.remove(&key);
            self.bitmap_previews.remove(&key);
        }
    }

    pub(super) fn handle_browser_action(&mut self, action: BrowserAction, ctx: egui::Context) {
        match action {
            BrowserAction::Select(key) => self.select_entry(key, ctx),
            BrowserAction::CopyTagName(key) => self.copy_tag_name(&key, &ctx),
            BrowserAction::DumpJson(key) => self.begin_export_json(key, ctx),
            BrowserAction::OpenInExplorer(key) => self.open_entry_in_explorer(&key),
            BrowserAction::DumpLoadedFolderJson(keys) => {
                self.begin_export_loaded_folder_json(keys, ctx)
            }
            BrowserAction::DumpLooseFolderJson { rel_path, label } => {
                self.begin_export_loose_folder_json(rel_path, label, ctx)
            }
            BrowserAction::MoveLooseFolder { rel_path, label } => {
                self.begin_refactor_loose_folder(rel_path, label, true)
            }
            BrowserAction::CopyLooseFolder { rel_path, label } => {
                self.begin_refactor_loose_folder(rel_path, label, false)
            }
            BrowserAction::ExtractRaw(key) => self.begin_extract_raw(key, ctx),
            BrowserAction::ExtractBitmap(key) => self.begin_extract_bitmap(key, ctx),
            BrowserAction::ExtractBitmapFolder(keys) => self.begin_extract_bitmap_folder(keys, ctx),
            BrowserAction::ExtractGeometry(key) => self.begin_extract_geometry(key, ctx),
            BrowserAction::ExtractImportInfo(key) => self.begin_extract_import_info(key, ctx),
            BrowserAction::ExtractAnimation(key) => self.begin_extract_animation(key, ctx),
            BrowserAction::ExtractMaterialShaderSources(key) => {
                self.begin_extract_material_shader_sources(key, ctx)
            }
            BrowserAction::ExtractMaterialShaderSourceFolder(keys) => {
                self.begin_extract_material_shader_source_folder(keys, ctx)
            }
            BrowserAction::ExtractHlslIncludeSource(key) => {
                self.begin_extract_hlsl_include_source(key, ctx)
            }
            BrowserAction::ExtractHlslIncludeFolder(keys) => {
                self.begin_extract_hlsl_include_folder(keys, ctx)
            }
            BrowserAction::RenameTag(key) => self.open_rename_tag(&key),
            BrowserAction::FindReferences(key) => self.show_references_for(&key),
            BrowserAction::ExploreReferences(key) => self.open_content_explorer(&key),
        }
    }

    pub(super) fn copy_tag_name(&mut self, key: &str, ctx: &egui::Context) {
        let Some(entry) = self.entry_for_key(key) else {
            self.status = "Tag is no longer in the browser".to_owned();
            return;
        };
        ctx.output_mut(|output| output.copied_text = entry.display_path.clone());
        self.status = format!("Copied {}", entry.display_path);
    }

    pub(super) fn open_entry_in_explorer(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "Tag is no longer in the browser".to_owned();
            return;
        };
        let Some(source) = self.source.as_ref().map(|source| &source.source) else {
            self.status = "No source loaded".to_owned();
            return;
        };
        let path = match (&entry.location, source) {
            (TagEntryLocation::LooseFile(path), _) => path.clone(),
            (_, TagSource::SingleFile { path }) => path.clone(),
            (TagEntryLocation::Monolithic { .. }, TagSource::MonolithicCache { root, .. }) => {
                root.join("blob_index.dat")
            }
            (TagEntryLocation::Monolithic { .. }, _) => {
                self.status = "Monolithic tag has no loose file to show".to_owned();
                return;
            }
        };
        #[cfg(windows)]
        {
            let arg = format!("/select,{}", path.display());
            match Command::new("explorer.exe").arg(arg).spawn() {
                Ok(_) => self.status = format!("Opened {}", path.display()),
                Err(error) => self.status = format!("Could not open File Explorer: {error}"),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            self.status = "Open with File Explorer is only available on Windows".to_owned();
        }
    }

    pub(super) fn begin_export_json(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let default_name = format!("{}.json", tag_file_stem(&entry));
        let Some(output) = rfd::FileDialog::new()
            .set_title("Dump Tag JSON")
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        self.status = format!("Dumping JSON for {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = export_tag_json(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_export_loaded_folder_json(
        &mut self,
        keys: Vec<String>,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.source.as_ref() else {
            return;
        };
        let entries = keys
            .iter()
            .filter_map(|key| source_data.entries.iter().find(|entry| entry.key == *key))
            .cloned()
            .collect::<Vec<_>>();
        if entries.is_empty() {
            self.status = "No loaded tags found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Dump Folder JSON")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Dumping {} loaded tag(s) to JSON", entries.len());
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                export_tag_json_entries(&source, &entries, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_export_loose_folder_json(
        &mut self,
        rel_path: PathBuf,
        label: String,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.source.as_ref() else {
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source_data.source else {
            return;
        };
        let root = root.clone();
        let names = source_data.names.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Dump Folder JSON")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Dumping JSON for folder {label}");
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = export_loose_folder_json(&root, &rel_path, &names, &output)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_refactor_loose_folder(
        &mut self,
        rel_path: PathBuf,
        label: String,
        move_folder: bool,
    ) {
        if self.folder_refactor.is_some() {
            self.status = "A folder move/copy is already running".to_owned();
            return;
        }
        if self.parsed_tags.values().any(|doc| doc.dirty) {
            self.status = "Save or close dirty tags before moving/copying folders".to_owned();
            return;
        }
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Folder move/copy requires a loaded tags folder".to_owned();
            return;
        };
        let title = if move_folder {
            format!("Move {label} To")
        } else {
            format!("Copy {label} To")
        };
        let Some(destination_parent) = rfd::FileDialog::new()
            .set_title(title)
            .set_directory(&root)
            .pick_folder()
        else {
            return;
        };
        let names = self.names.clone();
        let existing_all_entries = self
            .source
            .as_ref()
            .map(|source| source.all_entries.clone())
            .unwrap_or_default();
        let existing_reverse_dependencies = self
            .source
            .as_ref()
            .and_then(|source| source.reverse_dependencies.clone());
        let game = self.source.as_ref().and_then(|source| source.game.clone());
        let tx = self.tx.clone();
        let job_label = if move_folder {
            format!("Moving {label}")
        } else {
            format!("Copying {label}")
        };
        self.folder_refactor = Some(FolderRefactorUiState {
            label: job_label.clone(),
            phase: "Preparing".to_owned(),
            progress: None,
        });
        self.status = format!("{job_label}: Preparing");
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_folder_refactor_job(
                    root,
                    rel_path,
                    destination_parent,
                    move_folder,
                    job_label,
                    names,
                    game,
                    existing_all_entries,
                    existing_reverse_dependencies,
                    &tx,
                )
            }))
            .unwrap_or_else(|_| Err("Folder move/copy worker crashed".to_owned()));
            let _ = tx.send(WorkerMessage::FolderRefactorFinished(result));
        });
    }

    pub(super) fn begin_extract_raw(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Raw Tag")
            .set_file_name(tag_file_name(&entry).as_str())
            .save_file()
        else {
            return;
        };
        self.status = format!("Extracting raw tag {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = extract_raw_tag(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_extract_bitmap(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Bitmap Images")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting bitmap {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = extract_bitmap_images(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_extract_bitmap_folder(&mut self, keys: Vec<String>, ctx: egui::Context) {
        let Some(source_data) = self.source.as_ref() else {
            return;
        };
        let entries = keys
            .iter()
            .filter_map(|key| source_data.entries.iter().find(|entry| entry.key == *key))
            .cloned()
            .collect::<Vec<_>>();
        if entries.is_empty() {
            self.status = "No bitmap tags found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract All Bitmaps")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting {} bitmap tag(s)", entries.len());
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                extract_bitmap_entries(&source, &entries, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_extract_geometry(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Geometry")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting geometry from {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                extract_geometry_for_entry(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_extract_import_info(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Import Info")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting import info from {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                extract_import_info_for_entry(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_extract_animation(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Animations")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting animations from {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = extract_animations_for_entry(&source, &entry, &output)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_extract_material_shader_sources(
        &mut self,
        key: String,
        ctx: egui::Context,
    ) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Source Shaders")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting source shaders from {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = extract_material_shader_sources(&source, &entry, &output)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_extract_material_shader_source_folder(
        &mut self,
        keys: Vec<String>,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.source.as_ref() else {
            return;
        };
        let entries = entries_for_keys(source_data, &keys);
        if entries.is_empty() {
            self.status = "No material shaders found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Material Shader Sources")
            .pick_folder()
        else {
            return;
        };
        self.status = format!(
            "Extracting source shaders from {} material shader(s)",
            entries.len()
        );
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = extract_material_shader_source_entries(&source, &entries, &output)
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_extract_hlsl_include_source(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract HLSL Include")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting HLSL include from {}", entry.display_path);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                extract_hlsl_include_source(&source, &entry, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn begin_extract_hlsl_include_folder(
        &mut self,
        keys: Vec<String>,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.source.as_ref() else {
            return;
        };
        let entries = entries_for_keys(source_data, &keys);
        if entries.is_empty() {
            self.status = "No HLSL includes found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract HLSL Includes")
            .pick_folder()
        else {
            return;
        };
        self.status = format!("Extracting {} HLSL include(s)", entries.len());
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                extract_hlsl_include_entries(&source, &entries, &output).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn export_context(&self, key: &str) -> Option<(TagSource, TagEntry)> {
        let source = self.source.as_ref()?.source.clone();
        let entry = self.entry_for_key(key)?.clone();
        Some((source, entry))
    }

    pub(super) fn save_current_tag(&mut self) {
        let Some(key) = self.selected_key.clone() else {
            self.status = "No tag selected".to_owned();
            return;
        };
        match self.save_tag_by_key(&key) {
            Ok(path) => self.status = format!("Saved {}", path.display()),
            Err(error) => self.status = format!("Save failed: {error}"),
        }
    }

    pub(super) fn save_tag_by_key(&mut self, key: &str) -> Result<PathBuf, String> {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return Err("Selected tag is no longer in the source".to_owned());
        };
        let Some(doc) = self.parsed_tags.get(key) else {
            return Err("Load the selected tag before saving".to_owned());
        };
        if doc.tag.classic_engine().is_none() && doc.tag.endian != Endian::Le {
            return Err("Only little-endian MCC tags can be saved".to_owned());
        }
        let TagEntryLocation::LooseFile(path) = &entry.location else {
            return Err("Monolithic cache tags are read-only".to_owned());
        };
        let output = path.clone();
        doc.tag
            .write_atomic(&output)
            .map_err(|error| error.to_string())?;
        if let Some(doc) = self.parsed_tags.get_mut(key) {
            doc.dirty = false;
        }
        Ok(output)
    }

    pub(super) fn save_current_tag_as(&mut self) {
        let Some(key) = self.selected_key.clone() else {
            self.status = "No tag selected".to_owned();
            return;
        };
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.status = "Selected tag is no longer in the source".to_owned();
            return;
        };
        let Some(doc) = self.parsed_tags.get(&key) else {
            self.status = "Load the selected tag before saving".to_owned();
            return;
        };
        if doc.tag.classic_engine().is_none() && doc.tag.endian != Endian::Le {
            self.status = "Only little-endian MCC tags can be saved".to_owned();
            return;
        }

        let extension = save_as_extension(self, &entry);
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save Current Tag As")
            .set_file_name(save_as_file_name(&entry, extension.as_deref()));
        if let Some(parent) = save_as_start_dir(&entry) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(extension) = extension.as_deref() {
            dialog = dialog.add_filter("Tag file", &[extension]);
        }
        let Some(mut output) = dialog.save_file() else {
            return;
        };
        if output.extension().is_none() {
            if let Some(extension) = extension.as_deref() {
                output.set_extension(extension);
            }
        }

        match doc.tag.write_atomic(&output) {
            Ok(()) => {
                self.status = format!("Saved copy to {}", output.display());
            }
            Err(error) => self.status = format!("Save As failed: {error}"),
        }
    }

    pub(super) fn fix_current_tag_dependencies(&mut self) {
        let Some(key) = self.selected_key.clone() else {
            self.status = "No tag selected".to_owned();
            return;
        };
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.status = "Selected tag is no longer in the source".to_owned();
            return;
        };
        let TagEntryLocation::LooseFile(_) = entry.location else {
            self.status = "Fix Tag Dependencies requires a loose-folder tag".to_owned();
            return;
        };
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Fix Tag Dependencies requires a loaded tags folder".to_owned();
            return;
        };

        let entries = match self.dependency_database_entries(&root) {
            Ok(entries) => entries,
            Err(error) => {
                self.status = format!("Could not build dependency database: {error}");
                return;
            }
        };
        let names = self.names.clone();
        let index = build_dependency_candidate_index(&entries, &names);
        let Some(doc) = self.parsed_tags.get_mut(&key) else {
            self.status = "Load the selected tag before fixing dependencies".to_owned();
            return;
        };
        if doc.tag.endian != Endian::Le {
            self.status = "Only little-endian loose tags can be edited".to_owned();
            return;
        }

        let report = fix_tag_dependencies_in_tag(&mut doc.tag, &root, &names, &index);
        if report.fixed > 0 {
            doc.dirty = true;
        }
        let status = report.status();
        self.terminal
            .lines
            .extend(report.lines.into_iter().map(TerminalLineEntry::new));
        trim_terminal_lines(&mut self.terminal.lines);
        self.terminal.scroll_to_bottom = true;
        self.status = status;
    }

    fn dependency_database_entries(&mut self, root: &Path) -> Result<Vec<TagEntry>, String> {
        let Some(source) = self.source.as_mut() else {
            return Err("no tag source is loaded".to_owned());
        };
        if !matches!(source.source, TagSource::LooseFolder { .. }) {
            return Err("load a loose editing-kit tags folder first".to_owned());
        }
        let entries = scan_folder_subtree_entries(root, Path::new(""), &self.names)
            .map_err(|error| error.to_string())?;
        source.all_entries = entries;
        source.group_tree = crate::source::build_group_tree(&source.all_entries);
        if let Some(game) = source.game.as_deref() {
            let _ = crate::source::save_entry_index(game, root, &source.all_entries);
        }
        Ok(source.all_entries.clone())
    }

    /// Tags that reference `entry` (its "parents"), via the reverse-dependency
    /// index. `None` when no index is available (non-folder source or not yet
    /// scanned).
    /// Open the rename/move dialog for a tag, pre-listing the tags that
    /// reference it (which will be rewritten on apply).
    pub(super) fn open_rename_tag(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return;
        };
        if !matches!(entry.location, TagEntryLocation::LooseFile(_)) {
            self.status = "Only loose-folder tags can be renamed/moved".to_owned();
            return;
        }
        let display = entry.display_path.replace('\\', "/");
        let (stem, extension) = match display.rsplit_once('.') {
            Some((stem, ext)) => (stem.to_owned(), ext.to_owned()),
            None => (display.clone(), String::new()),
        };
        let (referrers, referrers_unavailable) = match self.references_to_entry(&entry) {
            Some(list) => (
                list.iter()
                    .map(|e| e.display_path.replace('\\', "/"))
                    .collect(),
                false,
            ),
            None => (Vec::new(), true),
        };
        self.rename_tag = Some(RenameTagState {
            key: entry.key.clone(),
            group_tag: entry.group_tag,
            old_display: display,
            extension,
            new_path_input: stem,
            referrers,
            referrers_unavailable,
        });
    }

    /// Apply the rename/move: move the file on disk and rewrite every
    /// referencing tag, in the background (reuses the folder-refactor pipeline).
    pub(super) fn begin_rename_tag(&mut self) {
        let Some(state) = self.rename_tag.as_ref() else {
            return;
        };
        if self.folder_refactor.is_some() {
            self.status = "A move/rename is already running".to_owned();
            return;
        }
        if self.parsed_tags.values().any(|doc| doc.dirty) {
            self.status = "Save or close dirty tags before renaming".to_owned();
            return;
        }
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Rename requires a loaded tags folder".to_owned();
            return;
        };
        let new_rel = state
            .new_path_input
            .trim()
            .trim_matches(['/', '\\'])
            .to_owned();
        if new_rel.is_empty() {
            self.status = "Enter a destination path".to_owned();
            return;
        }
        let Some(entry) = self.entry_for_key(&state.key).cloned() else {
            self.status = "Tag no longer exists".to_owned();
            return;
        };
        let names = self.names.clone();
        let game = self.source.as_ref().and_then(|source| source.game.clone());
        let all_entries = self
            .source
            .as_ref()
            .map(|source| source.all_entries.clone())
            .unwrap_or_default();
        let reverse_dependencies = self
            .source
            .as_ref()
            .and_then(|source| source.reverse_dependencies.clone());
        let tx = self.tx.clone();
        self.folder_refactor = Some(FolderRefactorUiState {
            label: "Renaming tag".to_owned(),
            phase: "Preparing".to_owned(),
            progress: None,
        });
        self.status = "Renaming tag: Preparing".to_owned();
        self.rename_tag = None;
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_tag_rename_job(
                    root,
                    entry,
                    new_rel,
                    names,
                    game,
                    all_entries,
                    reverse_dependencies,
                    &tx,
                )
            }))
            .unwrap_or_else(|_| Err("Rename worker crashed".to_owned()));
            let _ = tx.send(WorkerMessage::FolderRefactorFinished(result));
        });
    }

    pub(super) fn references_to_entry(&self, entry: &TagEntry) -> Option<Vec<TagEntry>> {
        let source = self.source.as_ref()?;
        let index = source.reverse_dependencies.as_ref()?;
        let rel = dependency_entry_reference_path(entry, &self.names)?;
        let referrer_keys = index.dependents_for(entry.group_tag, &rel);
        let mut out: Vec<TagEntry> = referrer_keys
            .iter()
            .filter_map(|key| source.all_entries.iter().find(|e| &e.key == key).cloned())
            .collect();
        out.sort_by(|a, b| natural_entry_order(a).cmp(&natural_entry_order(b)));
        Some(out)
    }

    /// All tags that nothing references (orphans / roots). `None` when no index
    /// is available.
    pub(super) fn unreferenced_entries(&self) -> Option<Vec<TagEntry>> {
        let source = self.source.as_ref()?;
        let index = source.reverse_dependencies.as_ref()?;
        let mut out: Vec<TagEntry> = source
            .all_entries
            .iter()
            .filter(|entry| {
                dependency_entry_reference_path(entry, &self.names)
                    .map(|rel| index.dependents_for(entry.group_tag, &rel).is_empty())
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| natural_entry_order(a).cmp(&natural_entry_order(b)));
        Some(out)
    }

    /// Resolve the dependencies a tag declares (children) into browseable
    /// entries, via a one-shot dependency-key → entry lookup over all entries.
    fn children_of_entry(&self, key: &str) -> (Vec<TagEntry>, bool) {
        let Some(source) = self.source.as_ref() else {
            return (Vec::new(), true);
        };
        let Some(index) = source.reverse_dependencies.as_ref() else {
            return (Vec::new(), true);
        };
        let deps = index.dependencies_of(key);
        let mut by_key: HashMap<String, &TagEntry> = HashMap::new();
        for entry in &source.all_entries {
            if let Some(rel) = dependency_entry_reference_path(entry, &self.names) {
                by_key
                    .entry(crate::source::dependency_key(entry.group_tag, &rel))
                    .or_insert(entry);
            }
        }
        let mut children: Vec<TagEntry> = deps
            .iter()
            .filter_map(|dep| {
                by_key
                    .get(&crate::source::dependency_key(dep.group_tag, &dep.rel_path))
                    .map(|entry| (*entry).clone())
            })
            .collect();
        children.sort_by(|a, b| natural_entry_order(a).cmp(&natural_entry_order(b)));
        children.dedup_by(|a, b| a.key == b.key);
        (children, false)
    }

    /// Open the Content Explorer centered on `key`.
    pub(super) fn open_content_explorer(&mut self, key: &str) {
        let Some(focus) = self.entry_for_key(key).cloned() else {
            return;
        };
        let (parents, parents_unavailable) = match self.references_to_entry(&focus) {
            Some(parents) => (parents, false),
            None => (Vec::new(), true),
        };
        let (children, children_unavailable) = self.children_of_entry(key);
        self.content_explorer = Some(ContentExplorer {
            focus,
            parents,
            children,
            filter: String::new(),
            index_unavailable: parents_unavailable && children_unavailable,
            back: Vec::new(),
            forward: Vec::new(),
        });
    }

    /// Re-center the open Content Explorer on `entry`, recording history.
    pub(super) fn content_explorer_navigate(&mut self, entry: TagEntry) {
        let key = entry.key.clone();
        let (parents, parents_unavailable) = match self.references_to_entry(&entry) {
            Some(parents) => (parents, false),
            None => (Vec::new(), true),
        };
        let (children, children_unavailable) = self.children_of_entry(&key);
        if let Some(explorer) = self.content_explorer.as_mut() {
            explorer.back.push(explorer.focus.clone());
            explorer.forward.clear();
            explorer.focus = entry;
            explorer.parents = parents;
            explorer.children = children;
            explorer.index_unavailable = parents_unavailable && children_unavailable;
        }
    }

    pub(super) fn content_explorer_back(&mut self) {
        let Some(prev) = self
            .content_explorer
            .as_mut()
            .and_then(|explorer| explorer.back.pop())
        else {
            return;
        };
        self.recenter_explorer(prev, true);
    }

    pub(super) fn content_explorer_forward(&mut self) {
        let Some(next) = self
            .content_explorer
            .as_mut()
            .and_then(|explorer| explorer.forward.pop())
        else {
            return;
        };
        self.recenter_explorer(next, false);
    }

    /// Re-center without clearing history; pushes the current focus onto the
    /// opposite stack (used by back/forward).
    fn recenter_explorer(&mut self, entry: TagEntry, going_back: bool) {
        let key = entry.key.clone();
        let (parents, parents_unavailable) = match self.references_to_entry(&entry) {
            Some(parents) => (parents, false),
            None => (Vec::new(), true),
        };
        let (children, children_unavailable) = self.children_of_entry(&key);
        if let Some(explorer) = self.content_explorer.as_mut() {
            let current = std::mem::replace(&mut explorer.focus, entry);
            if going_back {
                explorer.forward.push(current);
            } else {
                explorer.back.push(current);
            }
            explorer.parents = parents;
            explorer.children = children;
            explorer.index_unavailable = parents_unavailable && children_unavailable;
        }
    }

    pub(super) fn show_references_for(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return;
        };
        let title = format!("References to {}", entry.display_path.replace('\\', "/"));
        match self.references_to_entry(&entry) {
            Some(entries) => {
                let note = entries
                    .is_empty()
                    .then(|| "No other tags reference this tag.".to_owned());
                self.query_results = Some(TagQueryResults {
                    title,
                    entries,
                    annotations: Vec::new(),
                    note,
                });
            }
            None => {
                self.query_results = Some(TagQueryResults {
                    title,
                    entries: Vec::new(),
                    annotations: Vec::new(),
                    note: Some(self.reference_index_unavailable_note()),
                });
            }
        }
    }

    /// Explain why a reference lookup found no index, tailored to whether one is
    /// currently building (auto after the full scan, or via Tools → Build
    /// Reference Index).
    fn reference_index_unavailable_note(&self) -> String {
        if self.building_reverse_dependencies || self.scanning_entries {
            "Reference index is building — try again in a moment.".to_owned()
        } else {
            "Reference index unavailable — it builds automatically for loose-folder sources, or \
             run Tools → Build Reference Index."
                .to_owned()
        }
    }

    pub(super) fn show_unreferenced_tags(&mut self) {
        match self.unreferenced_entries() {
            Some(entries) => {
                let note = entries
                    .is_empty()
                    .then(|| "Every tag is referenced by at least one other tag.".to_owned());
                self.query_results = Some(TagQueryResults {
                    title: format!("Unreferenced tags ({})", entries.len()),
                    entries,
                    annotations: Vec::new(),
                    note,
                });
            }
            None => {
                self.query_results = Some(TagQueryResults {
                    title: "Unreferenced tags".to_owned(),
                    entries: Vec::new(),
                    annotations: Vec::new(),
                    note: Some(self.reference_index_unavailable_note()),
                });
            }
        }
    }

    /// Scan every scenario (`scnr`) tag and list its map id (+ map name where
    /// present). Reads `map id` at the scenario root, which covers the modern
    /// engines (H2A/H3/ODST/Reach/H4); classic Halo 2 stores it elsewhere.
    pub(super) fn show_map_ids(&mut self) {
        let Some(source) = self.source.as_ref() else {
            self.query_results = Some(TagQueryResults {
                title: "Scenario map IDs".to_owned(),
                entries: Vec::new(),
                annotations: Vec::new(),
                note: Some("No source loaded.".to_owned()),
            });
            return;
        };
        let mut entries = Vec::new();
        let mut annotations = Vec::new();
        for entry in &source.all_entries {
            if &entry.group_tag.to_be_bytes() != b"scnr" {
                continue;
            }
            let Ok(tag) = crate::source::read_entry(&source.source, entry) else {
                continue;
            };
            let root = tag.root();
            if let Some(id) = root.read_int_any("map id") {
                // `map name` carries a `#tooltip` suffix in Reach/H4, so resolve
                // it via the cleaned-name lookup rather than an exact match.
                let name = find_full_field_name(&root, "map name")
                    .and_then(|full| root.read_string_id(full))
                    .unwrap_or_default();
                annotations.push(if name.is_empty() {
                    format!("map id {id}")
                } else {
                    format!("map id {id}  ({name})")
                });
                entries.push(entry.clone());
            }
        }
        let note = entries.is_empty().then(|| {
            "No scenario map IDs found (scnr tags only; classic Halo 2 stores them elsewhere)."
                .to_owned()
        });
        self.query_results = Some(TagQueryResults {
            title: format!("Scenario map IDs ({})", entries.len()),
            entries,
            annotations,
            note,
        });
    }

    /// Scan every `snd!` tag once, reading its `sound class` + `compression`
    /// enum names. Shared by the class-listing and uncompressed-listing tools.
    /// Returns `(class, compression, entry)` triples, or `None` if no source.
    fn scan_sound_tags(&self) -> Option<Vec<(String, String, TagEntry)>> {
        let source = self.source.as_ref()?;
        let mut rows = Vec::new();
        for entry in &source.all_entries {
            if &entry.group_tag.to_be_bytes() != b"snd!" {
                continue;
            }
            let Ok(tag) = crate::source::read_entry(&source.source, entry) else {
                continue;
            };
            let root = tag.root();
            let class = find_full_field_name(&root, "sound class")
                .and_then(|full| root.read_enum_name(full))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "(none)".to_owned());
            let compression = find_full_field_name(&root, "compression")
                .and_then(|full| root.read_enum_name(full))
                .unwrap_or_default();
            rows.push((class, compression, entry.clone()));
        }
        Some(rows)
    }

    /// List every `snd!` tag annotated with its sound class + compression, with a
    /// per-class count summary (mirrors `count-class-sounds` /
    /// `count-all-class-sounds`).
    pub(super) fn show_sounds_by_class(&mut self) {
        let title = "Sounds by class";
        let Some(mut rows) = self.scan_sound_tags() else {
            self.query_results = Some(TagQueryResults {
                title: title.to_owned(),
                entries: Vec::new(),
                annotations: Vec::new(),
                note: Some("No source loaded.".to_owned()),
            });
            return;
        };
        rows.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.2.display_path.cmp(&b.2.display_path))
        });
        let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for (class, _, _) in &rows {
            *counts.entry(class.as_str()).or_default() += 1;
        }
        let entries: Vec<TagEntry> = rows.iter().map(|(_, _, e)| e.clone()).collect();
        let annotations: Vec<String> = rows
            .iter()
            .map(|(class, comp, _)| {
                if comp.is_empty() {
                    format!("[{class}]")
                } else {
                    format!("[{class}] {comp}")
                }
            })
            .collect();
        let note = if entries.is_empty() {
            Some("No sound tags found.".to_owned())
        } else {
            let summary = counts
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!("{} class(es) \u{2014} {summary}", counts.len()))
        };
        self.query_results = Some(TagQueryResults {
            title: format!("{title} ({})", entries.len()),
            entries,
            annotations,
            note,
        });
    }

    /// List `snd!` tags stored uncompressed (compression name contains "none"),
    /// mirroring `dump-uncompressed-sounds`.
    pub(super) fn show_uncompressed_sounds(&mut self) {
        let title = "Uncompressed sounds";
        let Some(rows) = self.scan_sound_tags() else {
            self.query_results = Some(TagQueryResults {
                title: title.to_owned(),
                entries: Vec::new(),
                annotations: Vec::new(),
                note: Some("No source loaded.".to_owned()),
            });
            return;
        };
        let mut hits: Vec<(String, String, TagEntry)> = rows
            .into_iter()
            .filter(|(_, comp, _)| comp.to_ascii_lowercase().contains("none"))
            .collect();
        hits.sort_by(|a, b| a.2.display_path.cmp(&b.2.display_path));
        let entries: Vec<TagEntry> = hits.iter().map(|(_, _, e)| e.clone()).collect();
        let annotations: Vec<String> = hits
            .iter()
            .map(|(class, comp, _)| format!("{comp}  [{class}]"))
            .collect();
        let note = entries
            .is_empty()
            .then(|| "No uncompressed sound tags found.".to_owned());
        self.query_results = Some(TagQueryResults {
            title: format!("{title} ({})", entries.len()),
            entries,
            annotations,
            note,
        });
    }

    /// Locate a tag in the browser tree: switch to Folders mode, clear the
    /// filter, select it, and request a one-shot force-open + scroll.
    pub(super) fn reveal_in_browser(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return;
        };
        self.filter.clear();
        self.browser_mode = BrowserMode::Folders;
        self.selected_key = Some(entry.key.clone());
        self.reveal_target = Some(RevealRequest {
            key: entry.key.clone(),
            ancestors: browser::ancestor_labels(&entry.display_path),
        });
    }

    /// Run a field-value search for the current query. If the in-memory index is
    /// ready it answers instantly from cache; otherwise it kicks off a live
    /// background scan (correct, slower) and builds the index for next time.
    pub(super) fn begin_field_value_search(&mut self, ctx: egui::Context) {
        let display = self.field_value_query.trim().to_owned();
        if display.is_empty() {
            return;
        }
        let query_lower = display.to_ascii_lowercase();
        let group_filter = self.field_value_group.trim().to_ascii_lowercase();
        let generation = self.source_generation;

        // Fast path: answer from the cached index.
        if self.field_index.is_ready_for(generation) {
            // Over-fetch when group-filtering so the cap applies post-filter.
            let raw_cap = if group_filter.is_empty() { 1000 } else { 8000 };
            let hits = self.field_index.query(&query_lower, raw_cap);
            let mut entries = Vec::new();
            let mut annotations = Vec::new();
            for (key, snippet) in hits {
                if let Some(entry) = self.entry_for_key(&key).cloned() {
                    if !group_filter.is_empty()
                        && !self.group_label_matches(entry.group_tag, &group_filter)
                    {
                        continue;
                    }
                    entries.push(entry);
                    annotations.push(snippet);
                    if entries.len() >= 1000 {
                        break;
                    }
                }
            }
            let note = entries
                .is_empty()
                .then(|| format!("No tag field values contain \"{display}\"."));
            self.status = format!(
                "Field search for \"{display}\": {} match(es) (indexed)",
                entries.len()
            );
            self.query_results = Some(TagQueryResults {
                title: format!("Field value '{display}' ({})", entries.len()),
                entries,
                annotations,
                note,
            });
            return;
        }

        if self.source.is_none() {
            return;
        }
        let base_entries: Vec<TagEntry> = {
            let source = self.source.as_ref().expect("checked");
            if source.all_entries.is_empty() {
                source.entries.clone()
            } else {
                source.all_entries.clone()
            }
        };
        let entries: Vec<TagEntry> = if group_filter.is_empty() {
            base_entries
        } else {
            base_entries
                .into_iter()
                .filter(|entry| self.group_label_matches(entry.group_tag, &group_filter))
                .collect()
        };
        let tag_source = self.source.as_ref().expect("checked").source.clone();
        let tx = self.tx.clone();
        self.field_value_searching = true;
        self.status = format!("Searching field values for \"{display}\"…");
        let search_ctx = ctx.clone();
        thread::spawn(move || {
            let result = run_field_value_search(&tag_source, &entries, &query_lower);
            let _ = tx.send(WorkerMessage::FieldValueSearchFinished {
                generation,
                query: display,
                result,
            });
            search_ctx.request_repaint();
        });
        // Build the index in the background so the next search is instant.
        self.begin_build_field_index(ctx);
    }

    /// Whether a group matches a (lowercased) group filter — by four-CC or by a
    /// substring of the group's name/extension (e.g. "weap" or "weapon").
    fn group_label_matches(&self, group_tag: u32, filter_lower: &str) -> bool {
        if format_group_tag(group_tag).to_ascii_lowercase() == filter_lower {
            return true;
        }
        self.names
            .name_for(group_tag)
            .or_else(|| group_tag_to_extension(group_tag))
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains(filter_lower)
    }

    /// Build the in-memory searchable-text index in the background (idempotent —
    /// skips if already ready for this generation or already building).
    pub(super) fn begin_build_field_index(&mut self, ctx: egui::Context) {
        let generation = self.source_generation;
        if self.field_index.is_ready_for(generation) || self.field_index.is_building() {
            return;
        }
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let entries: Vec<TagEntry> = if source.all_entries.is_empty() {
            source.entries.clone()
        } else {
            source.all_entries.clone()
        };
        let tag_source = source.source.clone();
        let tx = self.tx.clone();
        self.field_index.mark_building();
        thread::spawn(move || {
            let blobs = build_field_value_index(&tag_source, &entries);
            let _ = tx.send(WorkerMessage::FieldIndexBuilt { generation, blobs });
            ctx.request_repaint();
        });
    }

    /// Build the reverse-dependency index in the background so the
    /// find-references / unreferenced / Content Explorer features work without
    /// first running a move/rename. Idempotent: skips while a build is running,
    /// and skips an already-present index unless `force` is set (Tools →
    /// Rebuild). Loose-folder sources only; the result is persisted to disk so
    /// future launches load it instantly.
    pub(super) fn begin_build_reverse_dependencies(&mut self, ctx: egui::Context, force: bool) {
        if self.building_reverse_dependencies {
            return;
        }
        let Some(source) = self.source.as_ref() else {
            return;
        };
        if !matches!(source.source, TagSource::LooseFolder { .. }) {
            return;
        }
        if source.reverse_dependencies.is_some() && !force {
            return;
        }
        let entries = source.all_entries.clone();
        if entries.is_empty() {
            // The full entry set isn't ready yet. A reverse-dep index built from
            // the partially-loaded folder would be wrong (it would flag tags as
            // unreferenced just because their referrers weren't scanned), so
            // kick the full scan first. `begin_scan_all_entries` is idempotent
            // (guards on `scanning_entries`); the update loop re-enters here and
            // builds the index once the scan lands.
            if !self.scanning_entries {
                self.status = "Indexing tags, then building reference index…".to_owned();
            }
            self.begin_scan_all_entries(ctx);
            return;
        }
        let tag_source = source.source.clone();
        let generation = self.source_generation;
        let tx = self.tx.clone();
        self.building_reverse_dependencies = true;
        self.status = "Building reference index…".to_owned();
        thread::spawn(move || {
            let mut index = ReverseDependencyIndex::default();
            for entry in &entries {
                if let Ok(deps) = read_entry_dependencies(&tag_source, entry) {
                    index.set_tag_dependencies(entry.key.clone(), deps);
                }
            }
            let _ = tx.send(WorkerMessage::ReverseDependenciesBuilt { generation, index });
            ctx.request_repaint();
        });
    }

    /// Apply pasted TSV (header row = field names, one data row per element) to
    /// the target block's EXISTING elements, cell-by-cell via `apply_field_edit`.
    /// Rows beyond the current element count are reported and ignored (no
    /// structural changes — fully covered by undo). Returns a status summary.
    pub(super) fn apply_tsv_paste(&mut self) {
        let Some(paste) = self.tsv_paste.as_ref() else {
            return;
        };
        let tag_key = paste.tag_key.clone();
        let block_path = paste.block_path.clone();
        let text = paste.text.clone();

        let Some(doc) = self.parsed_tags.get_mut(&tag_key) else {
            self.set_tsv_paste_status("Tag is no longer open.");
            return;
        };
        let Some(block) = doc
            .tag
            .root()
            .field_path(&block_path)
            .and_then(|field| field.as_block())
        else {
            self.set_tsv_paste_status("Block no longer resolves in this tag.");
            return;
        };
        let element_count = block.len();
        let columns = block_leaf_columns(&block); // (clean, full) per leaf field

        let mut lines = text.lines();
        let Some(header_line) = lines.next() else {
            self.set_tsv_paste_status("Nothing to paste.");
            return;
        };
        // Map each pasted column index → the full field name to write.
        let header_to_full = map_tsv_header_to_fields(header_line, &columns);
        if header_to_full.iter().all(Option::is_none) {
            self.set_tsv_paste_status("No pasted column headers matched this block's fields.");
            return;
        }

        let mut edits = Vec::new();
        let mut data_rows = 0usize;
        let mut skipped_rows = 0usize;
        for (row_index, line) in lines.enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            data_rows += 1;
            if row_index >= element_count {
                skipped_rows += 1;
                continue;
            }
            for (col_index, cell) in line.split('\t').enumerate() {
                if let Some(Some(full)) = header_to_full.get(col_index) {
                    edits.push(PendingFieldEdit {
                        path: format!("{block_path}[{row_index}]/{full}"),
                        input: cell.trim().to_owned(),
                    });
                }
            }
        }

        if edits.is_empty() {
            self.set_tsv_paste_status("No editable cells matched.");
            return;
        }
        let edit_count = edits.len();
        let applied_rows = data_rows.saturating_sub(skipped_rows);
        doc.journal.begin_edit(&doc.tag, "Paste TSV");
        let _ = apply_pending_edits(&mut doc.tag, edits, &mut doc.dirty);
        doc.journal.end_edit_window();
        self.invalidate_tag_caches(&tag_key);

        let mut summary = format!("Pasted {edit_count} cell(s) across {applied_rows} row(s)");
        if skipped_rows > 0 {
            summary.push_str(&format!(
                " — {skipped_rows} extra row(s) ignored (block has {element_count} elements; add more first)"
            ));
        }
        self.status = summary.clone();
        self.set_tsv_paste_status(&summary);
    }

    fn set_tsv_paste_status(&mut self, message: &str) {
        if let Some(paste) = self.tsv_paste.as_mut() {
            paste.status = Some(message.to_owned());
        }
    }

    /// The documentation overlay (help/units + explanations) for a group,
    /// parsed once from its definition JSON and cached. `None` when the
    /// definitions can't be located (e.g. non-loose sources).
    pub(super) fn def_docs_for_entry(&mut self, entry: &TagEntry) -> Option<Rc<DefDocs>> {
        let root = match self.source.as_ref().map(|source| &source.source) {
            Some(TagSource::LooseFolder {
                definitions_root, ..
            }) => definitions_root.clone(),
            _ => return None,
        };
        let game = self
            .source
            .as_ref()
            .and_then(|source| source.game.clone())?;
        let group = self
            .names
            .name_for(entry.group_tag)
            .or_else(|| group_tag_to_extension(entry.group_tag))?
            .to_owned();
        // Cache key is the group's own JSON path; the docs themselves merge the
        // whole `parent_tag` inheritance chain (object-family fields live in
        // parent files).
        let path = root.join(&game).join(format!("{group}.json"));
        if let Some(docs) = self.def_docs_cache.get(&path) {
            return Some(docs.clone());
        }
        let docs = Rc::new(build_def_docs(&root, &game, &group));
        self.def_docs_cache.insert(path, docs.clone());
        Some(docs)
    }

    pub(super) fn show_tags_with_keyword(&mut self, keyword: &str) {
        let keys = self.keywords.tags_with(keyword);
        let entries: Vec<TagEntry> = keys
            .iter()
            .filter_map(|key| self.entry_for_key(key).cloned())
            .collect();
        let note = entries
            .is_empty()
            .then(|| "No tags with this keyword are in the current source.".to_owned());
        self.query_results = Some(TagQueryResults {
            title: format!("Tags tagged '{keyword}' ({})", entries.len()),
            entries,
            annotations: Vec::new(),
            note,
        });
    }

    /// Drop cached previews derived from a tag's contents so they rebuild from
    /// the (newly restored) tag bytes after an undo/redo.
    pub(super) fn invalidate_tag_caches(&mut self, key: &str) {
        if let Some(preview) = self.model_previews.get_mut(key) {
            preview.loaded_key = None;
            preview.data = None;
        }
        if let Some(bitmap) = self.bitmap_previews.get_mut(key) {
            bitmap.decoded = None;
            bitmap.texture = None;
            bitmap.texture_dirty = true;
        }
        // rmdf/rmop caches are keyed by external render-method paths, not by this
        // tag's contents, and the shader grid rebuilds from the tag each frame —
        // so nothing to clear there.
    }

    pub(super) fn undo_current_tag(&mut self) {
        let Some(key) = self.selected_key.clone() else {
            self.status = "Nothing to undo".to_owned();
            return;
        };
        let restored = self
            .parsed_tags
            .get_mut(&key)
            .and_then(|doc| doc.journal.undo(&doc.tag));
        self.restore_snapshot(&key, restored, "Undo");
    }

    pub(super) fn redo_current_tag(&mut self) {
        let Some(key) = self.selected_key.clone() else {
            self.status = "Nothing to redo".to_owned();
            return;
        };
        let restored = self
            .parsed_tags
            .get_mut(&key)
            .and_then(|doc| doc.journal.redo(&doc.tag));
        self.restore_snapshot(&key, restored, "Redo");
    }

    /// Apply a snapshot returned by the journal: re-parse the bytes into the
    /// document and invalidate derived caches.
    fn restore_snapshot(&mut self, key: &str, restored: Option<(Vec<u8>, String)>, verb: &str) {
        // Classic (Halo CE / Halo 2) snapshots are serialized in classic format,
        // which `read_from_bytes` can't parse — re-parse with the JSON layout.
        let group_tag = self.parsed_tags.get(key).map(|doc| doc.tag.group().tag);
        let game = self.source_game().map(str::to_owned);
        let definitions_root = self.source_definitions_root().map(Path::to_owned);
        match restored {
            Some((bytes, label)) => {
                match group_tag
                    .context("no open tag to restore")
                    .and_then(|group_tag| {
                        crate::source::read_tag_from_bytes(
                            &bytes,
                            game.as_deref(),
                            definitions_root.as_deref(),
                            group_tag,
                        )
                    }) {
                    Ok(tag) => {
                        if let Some(doc) = self.parsed_tags.get_mut(key) {
                            doc.tag = tag;
                            doc.dirty = true;
                        }
                        self.invalidate_tag_caches(key);
                        self.status = format!("{verb}: {label}");
                    }
                    Err(error) => {
                        self.status = format!("{verb} failed: {error}");
                    }
                }
            }
            None => {
                self.status = format!("Nothing to {}", verb.to_ascii_lowercase());
            }
        }
    }

    pub(super) fn can_undo_current(&self) -> bool {
        self.selected_key
            .as_ref()
            .and_then(|key| self.parsed_tags.get(key))
            .is_some_and(|doc| doc.journal.can_undo())
    }

    pub(super) fn can_redo_current(&self) -> bool {
        self.selected_key
            .as_ref()
            .and_then(|key| self.parsed_tags.get(key))
            .is_some_and(|doc| doc.journal.can_redo())
    }

    pub(super) fn current_prefs(&self) -> GuiPrefs {
        GuiPrefs {
            browser_mode: self.browser_mode,
            browser_sort: self.browser_sort,
            show_browser_prefixes: self.show_browser_prefixes,
            folders_before_tags: self.folders_before_tags,
            double_click_to_open_tags: self.double_click_to_open_tags,
            auto_restore_last_session: self.auto_restore_last_session,
            show_block_sizes: self.show_block_sizes,
            scroll_to_cycle_dropdowns: self.scroll_to_cycle_dropdowns,
            expert_mode: self.expert_mode,
            field_search_passive: self.field_search_passive,
            dark_mode: self.dark_mode,
            ui_scale: self.ui_scale,
            model_preview_size: self.model_preview_size,
            blender_path: self.blender_path.clone(),
            ek_folder_aliases: self.ek_folder_aliases.clone(),
            tool_commands_window_pos: self.tool_commands_window_pos,
            tool_commands_window_size: Some(self.tool_commands_window_size),
            tool_commands_left_width: self.tool_commands_left_width,
            tool_commands_collapsed_categories: self.tool_commands_collapsed_categories.clone(),
            recent_folders: self.recent_folders.clone(),
            custom_color_swatches: self.custom_color_swatches.clone(),
            palette_last_dir: self.palette_last_dir.clone(),
        }
    }

    pub(super) fn reapply_current_folder_profile(&mut self) {
        let Some(source) = self.source.as_mut() else {
            return;
        };
        let TagSource::LooseFolder {
            root,
            game,
            definitions_root,
        } = &mut source.source
        else {
            return;
        };
        let Ok(info) = resolve_folder_root(root, &self.ek_folder_aliases) else {
            return;
        };
        let new_game = info.game.map(str::to_owned);
        if source.game == new_game && *game == new_game {
            return;
        }

        source.label = info.label;
        source.game = new_game.clone();
        *game = new_game.clone();
        self.names = new_game
            .as_deref()
            .and_then(|game| TagNameIndex::load_game(definitions_root, game).ok())
            .unwrap_or_else(|| self.default_names.clone());
        source.names = self.names.clone();
        source.group_tree = crate::source::build_group_tree(&source.all_entries);
        self.source_generation = self.source_generation.wrapping_add(1);
        self.status = match new_game {
            Some(game) => format!("Current folder now uses {game} definitions"),
            None => "Current folder no longer has a detected game profile".to_owned(),
        };
    }

    pub(super) fn editing_kit_root(&self) -> Option<PathBuf> {
        let TagSource::LooseFolder { root, .. } = &self.source.as_ref()?.source else {
            return None;
        };
        if root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("tags"))
        {
            return root.parent().map(Path::to_path_buf);
        }
        Some(root.clone())
    }

    pub(super) fn kit_tool_path(&self, executable_name: &str) -> Option<PathBuf> {
        Some(self.editing_kit_root()?.join(executable_name))
    }

    pub(super) fn launch_sapien(&mut self) {
        self.launch_kit_tool("Sapien", "sapien.exe");
    }

    /// The tag_test executable name for the loaded game. Each editing kit ships
    /// its own renamed build (e.g. H3EK is `halo3_tag_test.exe`); fall back to
    /// the generic name when the game is unknown.
    pub(super) fn tag_test_executable(&self) -> &'static str {
        match self.source.as_ref().and_then(|s| s.game.as_deref()) {
            Some("haloce_mcc") => "halo_tag_test.exe",
            Some("halo2_mcc") => "halo2_tag_test.exe",
            Some("halo3_mcc") => "halo3_tag_test.exe",
            Some("halo3odst_mcc") => "atlas_tag_test.exe",
            Some("haloreach_mcc") => "reach_tag_test.exe",
            Some("halo4_mcc") => "halo4_tag_test.exe",
            _ => "tag_test.exe",
        }
    }

    pub(super) fn launch_tag_test(&mut self) {
        self.launch_kit_tool("tag_test", self.tag_test_executable());
    }

    pub(super) fn launch_blender(&mut self) {
        let Some(path) = self.blender_path.clone() else {
            self.settings_open = true;
            self.status = "Set the Blender path in File > Settings first".to_owned();
            return;
        };
        if !path.is_file() {
            self.status = format!("Blender executable not found: {}", path.display());
            self.settings_open = true;
            return;
        }
        self.spawn_tool("Blender", &path, path.parent().map(Path::to_path_buf));
    }

    pub(super) fn choose_blender_path(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Select Blender Executable");
        if let Some(path) = self.blender_path.as_ref().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(path);
        }
        #[cfg(target_os = "windows")]
        {
            dialog = dialog.add_filter("Executable", &["exe"]);
        }
        if let Some(path) = dialog.pick_file() {
            self.blender_path = Some(path.clone());
            self.blender_path_input = path.display().to_string();
            self.status = format!("Blender path set to {}", path.display());
        }
    }

    fn launch_kit_tool(&mut self, label: &str, executable_name: &str) {
        let Some(path) = self.kit_tool_path(executable_name) else {
            self.status = format!("{label} requires a loaded editing-kit folder");
            return;
        };
        if !path.is_file() {
            self.status = format!("{label} executable not found: {}", path.display());
            return;
        }
        self.spawn_tool(label, &path, self.editing_kit_root());
    }

    fn spawn_tool(&mut self, label: &str, path: &Path, work_dir: Option<PathBuf>) {
        let mut command = Command::new(path);
        if let Some(work_dir) = work_dir {
            command.current_dir(work_dir);
        }
        match command.spawn() {
            Ok(_) => self.status = format!("Launched {label}"),
            Err(error) => self.status = format!("Could not launch {label}: {error}"),
        }
    }

    /// Record the current terminal-open state against the loaded game so it
    /// is restored next time that editing kit is opened.
    pub(super) fn remember_terminal_open_for_game(&mut self) {
        let Some(game) = self.source.as_ref().and_then(|s| s.game.clone()) else {
            return;
        };
        if self.terminal_open {
            self.terminal_open_games.insert(game);
        } else {
            self.terminal_open_games.remove(&game);
        }
    }

    /// Resolve a pending "Open referenced tag" request against the loose-folder
    /// tags root and open it in a new tab (creating a transient entry if the
    /// target isn't in the current index).
    pub(super) fn process_pending_open(&mut self, ctx: &egui::Context) {
        let Some(req) = self.pending_open.take() else {
            return;
        };
        let root = match self.source.as_ref().map(|s| &s.source) {
            Some(TagSource::LooseFolder { root, .. }) => root.clone(),
            _ => {
                self.status = "Open requires a loose-folder source".to_owned();
                return;
            }
        };
        // Resolve the file extension from the definitions name index first
        // (covers every group, e.g. collision_model/physics_model), falling
        // back to the built-in table.
        let ext = self
            .names
            .name_for(req.group_tag)
            .or_else(|| blam_tags::paths::group_tag_to_extension(req.group_tag))
            .unwrap_or("");
        // Normalize: tolerate forward slashes and a path that already carries
        // its extension (e.g. a shader bitmap ref), so we don't double-append.
        let mut rel = req.rel_path.replace('/', "\\");
        if !ext.is_empty() {
            if let Some(stripped) = rel
                .strip_suffix(&format!(".{ext}"))
                .or_else(|| rel.strip_suffix(&format!(".{}", ext.to_ascii_uppercase())))
            {
                rel = stripped.to_owned();
            }
        }
        let abs = blam_tags::paths::resolve_tag_path(&root, &rel, ext);
        if !abs.exists() {
            self.status = format!(
                "Referenced tag not found: {} (group {})",
                abs.display(),
                blam_tags::format_group_tag(req.group_tag)
            );
            return;
        }
        let key = format!("file:{}", abs.display());
        // Ensure an entry exists so ensure_tag_loading can resolve it.
        if self.entry_for_key(&key).is_none() {
            let group_name = self.names.name_for(req.group_tag).map(str::to_owned);
            let display_path = if ext.is_empty() {
                req.rel_path.replace('\\', "/")
            } else {
                format!("{}.{ext}", req.rel_path.replace('\\', "/"))
            };
            let entry = TagEntry {
                key: key.clone(),
                display_path,
                group_tag: req.group_tag,
                group_name,
                location: TagEntryLocation::LooseFile(abs),
            };
            if let Some(source) = self.source.as_mut() {
                source.entries.push(entry);
            }
        }
        self.select_entry(key.clone(), ctx.clone());
        // Alt-click requested a floating window: tear the just-opened tab off.
        if req.float {
            self.pop_tab(&key);
        }
    }

    /// Run a geometry Import request (`tool render/collision/physics/...`)
    /// streamed to the terminal panel.
    pub(super) fn process_pending_tool_import(&mut self, ctx: &egui::Context) {
        let Some(req) = self.pending_tool_import.take() else {
            return;
        };
        if self.editing_kit_root().is_none() {
            self.status = "Import requires a loaded editing-kit folder".to_owned();
            return;
        }
        let command = format!("tool {} \"{}\"", req.verb, req.source_dir);
        self.spawn_terminal_command(command, ctx.clone());
    }

    pub(super) fn begin_reimport_bitmap(&mut self, key: String, ctx: egui::Context) {
        if self.terminal.running {
            self.status = "A command is already running".to_owned();
            return;
        }
        let Some(source) = self.source.as_ref().map(|source| source.source.clone()) else {
            self.status = "Reimport requires a loaded editing-kit folder".to_owned();
            return;
        };
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.status = "Bitmap tag is no longer in the source".to_owned();
            return;
        };
        let Some(tags_root) = (match &source {
            TagSource::LooseFolder { root, .. } => Some(root.as_path()),
            _ => None,
        }) else {
            self.status = "Bitmap reimport requires a loose tags folder".to_owned();
            return;
        };
        let Some(work_dir) = tags_root.parent().map(Path::to_path_buf) else {
            self.status = "Could not resolve editing-kit root".to_owned();
            return;
        };
        let Some(data_path) = bitmap_reimport_data_path(&entry, Some(tags_root)) else {
            self.status = "Could not resolve bitmap data path".to_owned();
            return;
        };
        let command = format!("tool bitmaps \"{data_path}\"");
        self.terminal_open = true;
        self.terminal
            .lines
            .push(TerminalLineEntry::new(format!("> {command}")));
        trim_terminal_lines(&mut self.terminal.lines);
        self.terminal.scroll_to_bottom = true;
        self.terminal.refocus_input = true;
        self.terminal.running = true;
        self.status = format!("Reimporting bitmap {}", entry.display_path);
        let run_id = self.terminal.next_run_id;
        self.terminal.next_run_id = self.terminal.next_run_id.wrapping_add(1).max(1);
        let log_file = match create_terminal_log_file(run_id, &command) {
            Ok((path, file)) => {
                self.terminal.last_log_path = Some(path);
                Some(file)
            }
            Err(error) => {
                self.status = format!("Terminal full log unavailable: {error}");
                self.terminal.last_log_path = None;
                None
            }
        };

        let tx = self.tx.clone();
        thread::spawn(move || {
            let result =
                run_terminal_command_for_reimport(&command, &work_dir, &tx, &ctx, log_file)
                    .and_then(|_| read_entry(&source, &entry).map_err(|error| error.to_string()));
            let _ = tx.send(WorkerMessage::BitmapReimportFinished { key, result });
            ctx.request_repaint();
        });
    }

    /// Render the block delete/delete-all confirmation modal (if pending) and
    /// apply the op on confirm.
    pub(super) fn handle_block_confirm(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.block_confirm.as_ref() else {
            return;
        };
        let message = confirm.message.clone();
        let confirm_label = confirm.confirm_label.clone();
        let mut do_apply = false;
        let mut do_cancel = false;
        egui::Window::new("Confirm")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(RichText::new(message).color(text_dark()));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(&confirm_label)
                                    .color(Color32::from_rgb(230, 230, 228)),
                            )
                            .fill(Color32::from_rgb(150, 48, 40))
                            .min_size(Vec2::new(80.0, 24.0)),
                        )
                        .clicked()
                    {
                        do_apply = true;
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(Vec2::new(80.0, 24.0)))
                        .clicked()
                    {
                        do_cancel = true;
                    }
                });
            });
        if do_apply {
            if let Some(confirm) = self.block_confirm.take() {
                if let Some(doc) = self.parsed_tags.get_mut(&confirm.tag_key) {
                    let op = BlockOp {
                        path: confirm.path,
                        kind: confirm.kind,
                    };
                    doc.journal.begin_edit(&doc.tag, "Block edit");
                    if let Some(status) = apply_block_ops(&mut doc.tag, vec![op], &mut doc.dirty) {
                        self.status = status;
                    }
                    doc.journal.end_edit_window();
                }
            }
        } else if do_cancel {
            self.block_confirm = None;
        }
    }

    pub(super) fn handle_save_changes_prompt(&mut self, ctx: &egui::Context) {
        let action = render_save_changes_prompt(ctx, &mut self.save_changes_prompt);
        match action {
            SaveChangesPromptAction::None => {}
            SaveChangesPromptAction::Cancel => {
                self.save_changes_prompt.visible = false;
                self.save_changes_prompt.dirty_tags.clear();
                self.save_changes_prompt.error = None;
            }
            SaveChangesPromptAction::DontSave => {
                let action = self.save_changes_prompt.pending_action.clone();
                self.save_changes_prompt.visible = false;
                self.save_changes_prompt.dirty_tags.clear();
                self.save_changes_prompt.error = None;
                self.execute_close_action(action, ctx);
            }
            SaveChangesPromptAction::Save(tag_ids) => {
                let mut saved = Vec::new();
                let mut errors = Vec::new();
                for tag_id in tag_ids {
                    match self.save_tag_by_key(&tag_id) {
                        Ok(path) => saved.push(path.display().to_string()),
                        Err(error) => {
                            let label = self.tag_path_label(&tag_id);
                            errors.push(format!("{label}: {error}"));
                        }
                    }
                }
                if errors.is_empty() {
                    let action = self.save_changes_prompt.pending_action.clone();
                    self.save_changes_prompt.visible = false;
                    self.save_changes_prompt.dirty_tags.clear();
                    self.save_changes_prompt.error = None;
                    self.status = if saved.is_empty() {
                        "No files selected to save".to_owned()
                    } else {
                        format!("Saved {} file(s)", saved.len())
                    };
                    self.execute_close_action(action, ctx);
                } else {
                    let message = format!("Save failed: {}", errors.join("; "));
                    let pending_action = self.save_changes_prompt.pending_action.clone();
                    self.save_changes_prompt.dirty_tags =
                        self.dirty_tags_for_close_action(&pending_action);
                    self.status = message.clone();
                    self.save_changes_prompt.error = Some(message);
                }
            }
        }
    }

    pub(super) fn handle_last_opened_windows_prompt(&mut self, ctx: &egui::Context) {
        let action = render_last_opened_windows_prompt(ctx, self.last_opened_windows.as_mut());
        match action {
            LastOpenedWindowsAction::None => {}
            LastOpenedWindowsAction::OpenSettings => {
                self.last_opened_windows = None;
                self.settings_open = true;
            }
            LastOpenedWindowsAction::Cancel => {
                self.last_opened_windows = None;
            }
            LastOpenedWindowsAction::Restore {
                source_kind,
                source_path,
                tags,
            } => {
                self.last_opened_windows = None;
                self.begin_last_session_restore(source_kind, source_path, tags, ctx.clone());
            }
        }
    }

    pub(super) fn persist_prefs_if_changed(&mut self) {
        let prefs = self.current_prefs();
        if prefs == self.saved_prefs && self.terminal_open_games == self.saved_terminal_open_games {
            return;
        }
        match save_gui_prefs(&prefs, &self.terminal_open_games) {
            Ok(()) => {
                self.saved_prefs = prefs;
                self.saved_terminal_open_games = self.terminal_open_games.clone();
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn draw_floating_tabs(&mut self, ctx: &egui::Context) {
        let keys = self.floating_tabs.iter().cloned().collect::<Vec<_>>();
        let mut closed = Vec::new();
        for key in keys {
            let Some(entry) = self.entry_for_key(&key).cloned() else {
                closed.push(key);
                continue;
            };
            let Some(mut doc) = self.parsed_tags.remove(&key) else {
                continue;
            };
            let mut open = true;
            let mut dock_requested = false;
            let mut edit_status = None;
            let mut bitmap_reimport_request = None;
            let def_docs = self.def_docs_for_entry(&entry);
            let window_response = egui::Window::new(tag_tab_label(&entry))
                .id(egui::Id::new(("floating_tag", key.clone())))
                .resizable(true)
                .default_width(860.0)
                .default_height(640.0)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let dock = ui
                            .add(
                                egui::Label::new(RichText::new("dock").color(text_dark()).strong())
                                    .sense(Sense::click_and_drag()),
                            )
                            .on_hover_text("Click to dock, or drag onto the tab rack");
                        if dock.clicked() {
                            dock_requested = true;
                        }
                        if dock.drag_started() || dock.dragged() {
                            self.dragging_floating_tab = Some(key.clone());
                        }
                        ui.label(
                            RichText::new("drag to tab rack")
                                .color(subtle_dark())
                                .small(),
                        );
                    });
                    ui.separator();
                    draw_entry_header(ui, &entry, &self.names);
                    let supports_field_search = supports_field_search(&entry);
                    if supports_field_search {
                        self.draw_field_search_bar(ui, &key);
                    }
                    let mut pending = Vec::new();
                    let mut block_ops = Vec::new();
                    let mut shader_ops = Vec::new();
                    let mut shader_param_ops = Vec::new();
                    let mut h2_shader_param_ops = Vec::new();
                    let mut function_data_ops = Vec::new();
                    let mut model_variant_ops = Vec::new();
                    let mut bitmap_reimport = None;
                    let field_filter = compute_pending_field_filter(
                        &doc.tag,
                        supports_field_search,
                        self.field_search_passive,
                        &key,
                        &self.field_search,
                        &mut self.field_search_applied,
                    );
                    let mut color_request = None;
                    let mut function_request = None;
                    let mut block_clip_request = None;
                    let mut tsv_paste_request = None;
                    let sound_volume = self.audio.volume();
                    let mut edit_context = FieldEditContext {
                        view_scope: "floating",
                        tag_key: &key,
                        names: Some(&self.names),
                        group_tag: entry.group_tag,
                        root: Some(doc.tag.root()),
                        game: self
                            .source
                            .as_ref()
                            .and_then(|source| source.game.as_deref()),
                        definitions_root: self.source.as_ref().and_then(|source| {
                            match &source.source {
                                TagSource::LooseFolder {
                                    definitions_root, ..
                                } => Some(definitions_root.as_path()),
                                _ => None,
                            }
                        }),
                        definition_group_name: self
                            .names
                            .name_for(entry.group_tag)
                            .or_else(|| group_tag_to_extension(entry.group_tag)),
                        tags_root: self
                            .source
                            .as_ref()
                            .and_then(|source| match &source.source {
                                TagSource::LooseFolder { root, .. } => Some(root.as_path()),
                                _ => None,
                            }),
                        status: Some(&mut self.status),
                        editable: is_editable_tag(&entry, &doc.tag),
                        show_block_sizes: self.show_block_sizes,
                        buffers: &mut self.edit_buffers,
                        pending: &mut pending,
                        block_ops: &mut block_ops,
                        block_confirm: &mut self.block_confirm,
                        open_request: &mut self.pending_open,
                        sound_play_request: &mut self.audio.pending,
                        sound_status: self.audio.status.as_deref(),
                        sound_volume,
                        sound_extract_request: &mut self.pending_sound_extract,
                        sound_language: self.audio.language.as_deref(),
                        tool_import: &mut self.pending_tool_import,
                        bitmap_reimport: &mut bitmap_reimport,
                        shader_ops: &mut shader_ops,
                        shader_param_ops: &mut shader_param_ops,
                        h2_shader_param_ops: &mut h2_shader_param_ops,
                        function_data_ops: &mut function_data_ops,
                        model_variant_ops: &mut model_variant_ops,
                        color_request: &mut color_request,
                        function_request: &mut function_request,
                        docs: def_docs.as_deref(),
                        tsv_paste_request: &mut tsv_paste_request,
                        block_clipboard: self.block_clipboard.as_ref(),
                        block_clip_request: &mut block_clip_request,
                        field_filter: field_filter.as_ref(),
                    };
                    if is_bitmap_tag(&entry) {
                        let preview = self.bitmap_previews.entry(key.clone()).or_default();
                        draw_bitmap_tag(
                            ui,
                            ctx,
                            &doc.tag,
                            &entry,
                            &self.names,
                            &mut self.color_popup,
                            preview,
                            self.expert_mode,
                            &mut edit_context,
                        );
                    } else {
                        let mut local_model_preview;
                        let model_preview = if is_model_group(entry.group_tag, &self.names) {
                            self.model_previews.entry(key.clone()).or_default()
                        } else {
                            local_model_preview = ModelPreviewState::default();
                            &mut local_model_preview
                        };
                        draw_tag(
                            ui,
                            &doc.tag,
                            &entry,
                            &self.names,
                            self.source.as_ref().map(|source| &source.source),
                            &mut self.rmdf_cache,
                            &mut self.rmop_cache,
                            &mut self.color_popup,
                            &mut self.function_popup,
                            model_preview,
                            &mut self.model_preview_size,
                            self.expert_mode,
                            &mut edit_context,
                        );
                    }
                    // Undo snapshot before this floating tab's edit batch.
                    if !pending.is_empty()
                        || !block_ops.is_empty()
                        || !shader_ops.is_empty()
                        || !shader_param_ops.is_empty()
                        || !model_variant_ops.is_empty()
                    {
                        doc.journal.begin_edit(&doc.tag, "Edit");
                    } else {
                        doc.journal.end_edit_window();
                    }
                    edit_status = apply_pending_edits(&mut doc.tag, pending, &mut doc.dirty);
                    if let Some(status) = apply_block_ops(&mut doc.tag, block_ops, &mut doc.dirty) {
                        edit_status = Some(status);
                    }
                    if let Some(status) = apply_shader_ops(&mut doc.tag, shader_ops, &mut doc.dirty)
                    {
                        edit_status = Some(status);
                    }
                    if let Some(status) =
                        apply_shader_param_ops(&mut doc.tag, shader_param_ops, &mut doc.dirty)
                    {
                        edit_status = Some(status);
                    }
                    if let Some(status) =
                        apply_h2_shader_param_ops(&mut doc.tag, h2_shader_param_ops, &mut doc.dirty)
                    {
                        edit_status = Some(status);
                    }
                    if let Some(status) =
                        apply_function_data_ops(&mut doc.tag, function_data_ops, &mut doc.dirty)
                    {
                        edit_status = Some(status);
                    }
                    if let Some(status) =
                        apply_model_variant_ops(&mut doc.tag, model_variant_ops, &mut doc.dirty)
                    {
                        edit_status = Some(status);
                        if let Some(preview) = self.model_previews.get_mut(&key) {
                            preview.loaded_key = None;
                            preview.data = None;
                        }
                    }
                    // A color swatch was clicked: open the shared picker.
                    if let Some(popup) = color_request {
                        self.color_popup = Some(popup);
                    }
                    if let Some(popup) = function_request {
                        self.function_popup = Some(popup);
                    }
                    // Element(s) were copied: stash them on the clipboard.
                    if let Some(clip) = block_clip_request {
                        edit_status = Some(format!(
                            "Copied {} '{}' element(s)",
                            clip.elements.len(),
                            clip.label
                        ));
                        self.block_clipboard = Some(clip);
                    }
                    // "Paste TSV…" was chosen: open the import window.
                    if let Some(req) = tsv_paste_request {
                        self.tsv_paste = Some(TsvPasteState {
                            tag_key: key.clone(),
                            block_path: req.block_path,
                            block_label: req.block_label,
                            element_count: req.element_count,
                            text: String::new(),
                            status: None,
                        });
                    }
                    bitmap_reimport_request = bitmap_reimport;
                });
            if let Some(inner) = &window_response {
                let pointer_down_over_window = ctx.input(|input| {
                    input.pointer.primary_down()
                        && input
                            .pointer
                            .interact_pos()
                            .is_some_and(|pos| inner.response.rect.contains(pos))
                });
                if inner.response.drag_started()
                    || inner.response.dragged()
                    || pointer_down_over_window
                {
                    self.dragging_floating_tab = Some(key.clone());
                }
            }
            if !open {
                closed.push(key.clone());
            }
            self.parsed_tags.insert(key.clone(), doc);
            if open && dock_requested {
                self.dock_tab(&key);
                self.dragging_floating_tab = None;
            }
            if let Some(status) = edit_status {
                self.status = status;
            }
            if let Some(key) = bitmap_reimport_request {
                self.begin_reimport_bitmap(key, ctx.clone());
            }
        }
        for key in closed {
            self.request_close_action(PendingCloseAction::CloseTab(key), ctx);
        }
    }
}

fn ordered_unique_keys<'a>(keys: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for key in keys {
        if seen.insert(key.clone()) {
            ordered.push(key.clone());
        }
    }
    ordered
}

fn save_as_extension(app: &Baboon, entry: &TagEntry) -> Option<String> {
    app.names
        .name_for(entry.group_tag)
        .or_else(|| group_tag_to_extension(entry.group_tag))
        .map(|extension| extension.trim().to_owned())
        .filter(|extension| !extension.is_empty())
}

enum SaveChangesPromptAction {
    None,
    Save(Vec<String>),
    DontSave,
    Cancel,
}

fn render_save_changes_prompt(
    ctx: &egui::Context,
    prompt: &mut SaveChangesPrompt,
) -> SaveChangesPromptAction {
    if !prompt.visible {
        return SaveChangesPromptAction::None;
    }

    let mut action = SaveChangesPromptAction::None;
    egui::Window::new("Baboon - Save Changes?")
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(520.0)
        .default_height(260.0)
        .show(ctx, |ui| {
            ui.label(
                RichText::new("The following files have been modified. Select the files to save.")
                    .color(text_dark()),
            );
            ui.add_space(8.0);
            if let Some(error) = prompt.error.as_deref() {
                ui.label(RichText::new(error).color(Color32::from_rgb(180, 48, 40)));
                ui.add_space(6.0);
            }
            ScrollArea::both().max_height(150.0).show(ui, |ui| {
                for dirty in &mut prompt.dirty_tags {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut dirty.checked, "");
                        ui.label(RichText::new(&dirty.path).color(text_dark()));
                    });
                }
            });
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("Cancel").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    action = SaveChangesPromptAction::Cancel;
                }
                if ui
                    .add(egui::Button::new("Don't Save").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    action = SaveChangesPromptAction::DontSave;
                }
                if ui
                    .add(egui::Button::new("Save").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    let tag_ids = prompt
                        .dirty_tags
                        .iter()
                        .filter(|dirty| dirty.checked)
                        .map(|dirty| dirty.tag_id.clone())
                        .collect();
                    action = SaveChangesPromptAction::Save(tag_ids);
                }
            });
        });
    action
}

enum LastOpenedWindowsAction {
    None,
    OpenSettings,
    Restore {
        source_kind: LastSessionSourceKind,
        source_path: PathBuf,
        tags: Vec<LastSessionTag>,
    },
    Cancel,
}

fn render_last_opened_windows_prompt(
    ctx: &egui::Context,
    prompt: Option<&mut LastOpenedWindowsPrompt>,
) -> LastOpenedWindowsAction {
    let Some(prompt) = prompt else {
        return LastOpenedWindowsAction::None;
    };
    if !prompt.visible {
        return LastOpenedWindowsAction::None;
    }

    let mut action = LastOpenedWindowsAction::None;
    egui::Window::new("Last Opened Windows")
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(520.0)
        .default_height(300.0)
        .show(ctx, |ui| {
            ui.label(
                RichText::new("These windows were opened the last time you used Baboon.")
                    .color(text_dark()),
            );
            ui.label(RichText::new("Which of these would you like to reopen?").color(text_dark()));
            if !prompt.source_available {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("Missing source: {}", prompt.source_path.display()))
                        .color(Color32::from_rgb(180, 48, 40)),
                );
            }
            ui.add_space(8.0);
            ScrollArea::both().max_height(170.0).show(ui, |ui| {
                for entry in &mut prompt.entries {
                    ui.add_enabled_ui(entry.available, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut entry.checked, "");
                            let label = if entry.available {
                                entry.tag.label.clone()
                            } else {
                                format!("{} (missing)", entry.tag.label)
                            };
                            ui.label(RichText::new(label).color(text_dark()));
                        });
                    });
                }
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Options for this window available in")
                        .color(subtle_dark())
                        .small(),
                );
                if ui.link("File > Settings").clicked() {
                    action = LastOpenedWindowsAction::OpenSettings;
                }
            });
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("Cancel").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    action = LastOpenedWindowsAction::Cancel;
                }
                if ui
                    .add(egui::Button::new("OK").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    let tags = prompt.checked_tags();
                    action = LastOpenedWindowsAction::Restore {
                        source_kind: prompt.source_kind,
                        source_path: prompt.source_path.clone(),
                        tags,
                    };
                }
            });
        });
    action
}

fn save_as_file_name(entry: &TagEntry, extension: Option<&str>) -> String {
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

fn save_as_start_dir(entry: &TagEntry) -> Option<PathBuf> {
    match &entry.location {
        TagEntryLocation::LooseFile(path) => path.parent().map(Path::to_path_buf),
        TagEntryLocation::Monolithic { .. } => None,
    }
}

fn entries_for_keys(source: &LoadedSourceData, keys: &[String]) -> Vec<TagEntry> {
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

fn run_terminal_command_for_reimport(
    command: &str,
    work_dir: &Path,
    tx: &Sender<WorkerMessage>,
    ctx: &egui::Context,
    mut log_file: Option<std::fs::File>,
) -> Result<(), String> {
    let mut log_error_reported = false;
    #[cfg(target_os = "windows")]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut c = std::process::Command::new("cmd");
        c.creation_flags(CREATE_NO_WINDOW);
        c.args(["/C", &format!("{command} 2>&1")]);
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(work_dir)
        .stdout(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|error| {
        let message = format!("[error] {error}");
        send_terminal_line(
            tx,
            ctx,
            &mut log_file,
            &mut log_error_reported,
            message.clone(),
        );
        message
    })?;
    if let Some(stdout) = child.stdout.take() {
        if let Err(error) =
            stream_terminal_output(stdout, tx, ctx, &mut log_file, &mut log_error_reported)
        {
            let message = format!("Could not read tool output: {error}");
            send_terminal_line(
                tx,
                ctx,
                &mut log_file,
                &mut log_error_reported,
                format!("[error] {message}"),
            );
            return Err(message);
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("Could not wait for tool: {error}"))?;
    if let Some(code) = status.code() {
        send_terminal_line(
            tx,
            ctx,
            &mut log_file,
            &mut log_error_reported,
            format!("[exit {code}]"),
        );
    }
    if status.success() {
        Ok(())
    } else {
        Err(status
            .code()
            .map(|code| format!("tool exited with code {code}"))
            .unwrap_or_else(|| "tool exited without a status code".to_owned()))
    }
}

fn stream_terminal_output<R: std::io::Read>(
    mut reader: R,
    tx: &Sender<WorkerMessage>,
    ctx: &egui::Context,
    log_file: &mut Option<std::fs::File>,
    log_error_reported: &mut bool,
) -> std::io::Result<()> {
    let mut buffer = [0_u8; 4096];
    let mut line = Vec::new();
    let mut pending_cr = false;

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }

        for &byte in &buffer[..read] {
            if pending_cr {
                if byte == b'\n' {
                    emit_terminal_output_line(&line, tx, ctx, log_file, log_error_reported);
                    line.clear();
                    pending_cr = false;
                    continue;
                }

                emit_terminal_output_line(&line, tx, ctx, log_file, log_error_reported);
                line.clear();
                pending_cr = false;
            }

            match byte {
                b'\r' => pending_cr = true,
                b'\n' => {
                    emit_terminal_output_line(&line, tx, ctx, log_file, log_error_reported);
                    line.clear();
                }
                _ => line.push(byte),
            }
        }
    }

    if pending_cr {
        emit_terminal_output_line(&line, tx, ctx, log_file, log_error_reported);
        line.clear();
    }

    if !line.is_empty() {
        emit_terminal_output_line(&line, tx, ctx, log_file, log_error_reported);
    }

    Ok(())
}

fn emit_terminal_output_line(
    line: &[u8],
    tx: &Sender<WorkerMessage>,
    ctx: &egui::Context,
    log_file: &mut Option<std::fs::File>,
    log_error_reported: &mut bool,
) {
    let line = String::from_utf8_lossy(line).into_owned();
    write_terminal_log_line(log_file, tx, line.as_str(), log_error_reported);
    let _ = tx.send(WorkerMessage::TerminalLine(line));
    ctx.request_repaint();
}

fn send_terminal_line(
    tx: &Sender<WorkerMessage>,
    ctx: &egui::Context,
    log_file: &mut Option<std::fs::File>,
    log_error_reported: &mut bool,
    line: String,
) {
    write_terminal_log_line(log_file, tx, &line, log_error_reported);
    let _ = tx.send(WorkerMessage::TerminalLine(line));
    ctx.request_repaint();
}

fn trim_terminal_lines(lines: &mut Vec<TerminalLineEntry>) {
    if lines.len() > TERMINAL_VISIBLE_LINE_LIMIT {
        let remove = lines.len() - TERMINAL_VISIBLE_LINE_TRIM_TARGET;
        lines.drain(..remove);
    }
}

fn create_terminal_log_file(
    run_id: u64,
    command: &str,
) -> Result<(PathBuf, std::fs::File), String> {
    use std::io::Write as _;

    let dir = terminal_logs_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    let path = dir.join(format!(
        "terminal-run-{}-{run_id}.log",
        terminal_log_timestamp()
    ));
    let mut file = std::fs::File::create(&path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    writeln!(file, "> {command}")
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok((path, file))
}

fn write_terminal_log_line(
    log_file: &mut Option<std::fs::File>,
    tx: &Sender<WorkerMessage>,
    line: &str,
    log_error_reported: &mut bool,
) {
    use std::io::Write as _;

    let Some(file) = log_file.as_mut() else {
        return;
    };
    if let Err(error) = writeln!(file, "{line}") {
        *log_file = None;
        if !*log_error_reported {
            *log_error_reported = true;
            let _ = tx.send(WorkerMessage::TerminalLogError(format!(
                "Terminal full log disabled: {error}"
            )));
        }
    }
}

fn append_terminal_log_path(path: &Path, line: &str) -> Result<(), String> {
    use std::io::Write as _;

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| format!("Could not open terminal log {}: {error}", path.display()))?;
    writeln!(file, "{line}")
        .map_err(|error| format!("Could not write terminal log {}: {error}", path.display()))
}

fn terminal_log_timestamp() -> String {
    let seconds = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_) => 0,
    };
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u32, d as u32)
}

pub(super) fn open_terminal_log(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.arg("/C").arg("start").arg("").arg(path);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(path);
        command
    };
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path);
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open terminal log {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_terminal_output(input: &[u8]) -> Vec<(&'static str, String)> {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = egui::Context::default();
        let mut log_file = None;
        let mut log_error_reported = false;
        let result = stream_terminal_output(
            std::io::Cursor::new(input),
            &tx,
            &ctx,
            &mut log_file,
            &mut log_error_reported,
        );
        assert!(result.is_ok());
        drop(tx);

        rx.try_iter()
            .filter_map(|message| match message {
                WorkerMessage::TerminalLine(line) => Some(("line", line)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn terminal_output_appends_carriage_return_progress() {
        let output = collect_terminal_output(
            b"building bsp3d children... 10%\rbuilding bsp3d children... 70%\rbuilding bsp3d children... 100%\nnext\n",
        );

        assert_eq!(
            output,
            vec![
                ("line", "building bsp3d children... 10%".to_owned()),
                ("line", "building bsp3d children... 70%".to_owned()),
                ("line", "building bsp3d children... 100%".to_owned()),
                ("line", "next".to_owned()),
            ]
        );
    }

    #[test]
    fn terminal_output_treats_crlf_as_newline() {
        let output = collect_terminal_output(b"done\r\nnext");

        assert_eq!(
            output,
            vec![("line", "done".to_owned()), ("line", "next".to_owned()),]
        );
    }

    #[test]
    fn terminal_output_full_log_keeps_carriage_return_progress() {
        let path = std::env::temp_dir().join(format!(
            "baboon-terminal-test-{}.log",
            terminal_log_timestamp()
        ));
        let file = std::fs::File::create(&path);
        assert!(file.is_ok());

        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = egui::Context::default();
        let mut log_file = file.ok();
        let mut log_error_reported = false;
        let result = stream_terminal_output(
            std::io::Cursor::new(b"building... 10%\rbuilding... 60%\rbuilding... 100%\n"),
            &tx,
            &ctx,
            &mut log_file,
            &mut log_error_reported,
        );
        assert!(result.is_ok());
        drop(log_file);
        drop(tx);

        let output: Vec<_> = rx
            .try_iter()
            .filter_map(|message| match message {
                WorkerMessage::TerminalLine(line) => Some(("line", line)),
                _ => None,
            })
            .collect();
        assert_eq!(
            output,
            vec![
                ("line", "building... 10%".to_owned()),
                ("line", "building... 60%".to_owned()),
                ("line", "building... 100%".to_owned()),
            ]
        );

        let text = std::fs::read_to_string(&path);
        assert!(text.is_ok());
        if let Ok(text) = text {
            assert!(text.contains("building... 10%\n"));
            assert!(text.contains("building... 60%\n"));
            assert!(text.contains("building... 100%\n"));
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn terminal_output_handles_heavy_carriage_return_progress() {
        let mut input = Vec::new();
        for index in 0..25_000 {
            input.extend_from_slice(format!("building bsp3d children... {index}%\r").as_bytes());
        }
        input.extend_from_slice(b"done\n");

        let output = collect_terminal_output(&input);

        assert_eq!(output.len(), 25_001);
        assert_eq!(
            output.first(),
            Some(&("line", "building bsp3d children... 0%".to_owned()))
        );
        assert_eq!(output.last(), Some(&("line", "done".to_owned())));
    }

    #[test]
    fn terminal_line_severity_classifies_tool_markers() {
        assert!(matches!(
            TerminalLineEntry::new("-ERROR- bad connection".to_owned()).severity,
            TerminalLineSeverity::Error
        ));
        assert!(matches!(
            TerminalLineEntry::new("WARNING overlapping surfaces".to_owned()).severity,
            TerminalLineSeverity::Warning
        ));
        assert!(matches!(
            TerminalLineEntry::new("[exit 0]".to_owned()).severity,
            TerminalLineSeverity::Success
        ));
        assert!(matches!(
            TerminalLineEntry::new("[exit 2]".to_owned()).severity,
            TerminalLineSeverity::Error
        ));
        assert!(matches!(
            TerminalLineEntry::new("=== summary".to_owned()).severity,
            TerminalLineSeverity::Summary
        ));
    }

    #[test]
    fn terminal_visible_lines_are_trimmed_to_limit() {
        let mut lines = Vec::new();
        for index in 0..(TERMINAL_VISIBLE_LINE_LIMIT + 10) {
            lines.push(TerminalLineEntry::new(format!("line {index}")));
        }

        trim_terminal_lines(&mut lines);

        assert_eq!(lines.len(), TERMINAL_VISIBLE_LINE_TRIM_TARGET);
        assert_eq!(
            lines.first().map(|line| line.text.as_str()),
            Some("line 2010")
        );
    }
}

enum TerminalStopResult {
    Stopped,
    AlreadyExited,
}

fn stop_terminal_process(process: &TerminalProcess) -> Result<TerminalStopResult, String> {
    #[cfg(target_os = "windows")]
    {
        stop_terminal_process_windows(process)
    }
    #[cfg(not(target_os = "windows"))]
    {
        stop_terminal_process_unix(process)
    }
}

#[cfg(target_os = "windows")]
fn stop_terminal_process_windows(process: &TerminalProcess) -> Result<TerminalStopResult, String> {
    let pid = {
        let mut slot = process
            .child
            .lock()
            .map_err(|_| "terminal process lock was poisoned".to_owned())?;
        let Some(child) = slot.as_mut() else {
            return Ok(TerminalStopResult::AlreadyExited);
        };
        match child
            .try_wait()
            .map_err(|error| format!("could not query terminal process: {error}"))?
        {
            Some(_) => {
                *slot = None;
                return Ok(TerminalStopResult::AlreadyExited);
            }
            None => child.id(),
        }
    };

    let output = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .output()
        .map_err(|error| format!("could not launch taskkill: {error}"))?;
    if output.status.success() {
        return Ok(TerminalStopResult::Stopped);
    }

    if terminal_process_already_exited(process)? {
        return Ok(TerminalStopResult::AlreadyExited);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("taskkill exited with {}", output.status)
    };
    Err(detail)
}

#[cfg(not(target_os = "windows"))]
fn stop_terminal_process_unix(process: &TerminalProcess) -> Result<TerminalStopResult, String> {
    let mut slot = process
        .child
        .lock()
        .map_err(|_| "terminal process lock was poisoned".to_owned())?;
    let Some(child) = slot.as_mut() else {
        return Ok(TerminalStopResult::AlreadyExited);
    };
    match child
        .try_wait()
        .map_err(|error| format!("could not query terminal process: {error}"))?
    {
        Some(_) => {
            *slot = None;
            Ok(TerminalStopResult::AlreadyExited)
        }
        None => {
            let pid = i32::try_from(child.id())
                .map_err(|_| format!("terminal process id {} is out of range", child.id()))?;
            let process_group = -pid;
            // The shell was spawned with `process_group(0)`, so its children
            // inherit the same process group. A negative pid targets the group.
            let result = unsafe { libc::kill(process_group, libc::SIGKILL) };
            if result == 0 {
                return Ok(TerminalStopResult::Stopped);
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                if child.try_wait().ok().flatten().is_some() {
                    *slot = None;
                }
                return Ok(TerminalStopResult::AlreadyExited);
            }
            Err(format!(
                "could not kill terminal process group {pid}: {error}"
            ))
        }
    }
}

#[cfg(target_os = "windows")]
fn terminal_process_already_exited(process: &TerminalProcess) -> Result<bool, String> {
    let mut slot = process
        .child
        .lock()
        .map_err(|_| "terminal process lock was poisoned".to_owned())?;
    let Some(child) = slot.as_mut() else {
        return Ok(true);
    };
    match child
        .try_wait()
        .map_err(|error| format!("could not query terminal process: {error}"))?
    {
        Some(_) => {
            *slot = None;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn fetch_latest_release() -> Result<UpdateCheckResult, String> {
    #[cfg(target_os = "windows")]
    {
        fetch_latest_release_powershell()
    }
    #[cfg(not(target_os = "windows"))]
    {
        fetch_latest_release_curl()
    }
}

const NO_PUBLIC_RELEASE_MESSAGE: &str = "No public Baboon releases found yet";

#[cfg(target_os = "windows")]
fn fetch_latest_release_powershell() -> Result<UpdateCheckResult, String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         $headers = @{{ 'User-Agent' = 'Baboon' }}; \
         try {{ \
             $release = Invoke-RestMethod -UseBasicParsing -Headers $headers -Uri '{}'; \
             [Console]::Out.WriteLine($release.tag_name); \
             [Console]::Out.WriteLine($release.html_url); \
         }} catch {{ \
             $statusCode = $null; \
             if ($_.Exception.Response -ne $null) {{ \
                 $statusCode = [int]$_.Exception.Response.StatusCode; \
             }} \
             if ($statusCode -eq 404) {{ \
                 [Console]::Out.WriteLine('__BABOON_NO_PUBLIC_RELEASE__'); \
                 exit 0; \
             }} \
             [Console]::Error.WriteLine($_.Exception.Message); \
             exit 1; \
         }}",
        BABOON_LATEST_RELEASE_API
    );
    let output = Command::new("powershell.exe")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .output()
        .map_err(|error| format!("Could not run PowerShell: {error}"))?;
    parse_latest_release_lines(&output.stdout, &output.stderr, output.status.success())
}

#[cfg(not(target_os = "windows"))]
fn fetch_latest_release_curl() -> Result<UpdateCheckResult, String> {
    let output = Command::new("curl")
        .args([
            "-sSL",
            "-w",
            "\n%{http_code}",
            "-H",
            "User-Agent: Baboon",
            BABOON_LATEST_RELEASE_API,
        ])
        .output()
        .map_err(|error| format!("Could not run curl: {error}"))?;
    if !output.status.success() {
        return Err(command_error(&output.stderr));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some((body, status_code)) = text.rsplit_once('\n') else {
        return Err("GitHub response did not include an HTTP status".to_owned());
    };
    if status_code.trim() == "404" {
        return Err(NO_PUBLIC_RELEASE_MESSAGE.to_owned());
    }
    if status_code.trim() != "200" {
        return Err(format!("GitHub returned HTTP {}", status_code.trim()));
    }
    let value: Value = serde_json::from_str(body)
        .map_err(|error| format!("GitHub returned invalid JSON: {error}"))?;
    let latest_tag = value
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    if latest_tag.is_empty() {
        return Err("GitHub response did not include a release tag".to_owned());
    }
    let release_url = value
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty())
        .unwrap_or(BABOON_RELEASES_URL)
        .to_owned();
    Ok(UpdateCheckResult {
        latest_tag,
        release_url,
    })
}

#[cfg(target_os = "windows")]
fn parse_latest_release_lines(
    stdout: &[u8],
    stderr: &[u8],
    success: bool,
) -> Result<UpdateCheckResult, String> {
    if !success {
        return Err(command_error(stderr));
    }
    let text = String::from_utf8_lossy(stdout);
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let latest_tag = lines.next().unwrap_or_default().to_owned();
    if latest_tag == "__BABOON_NO_PUBLIC_RELEASE__" {
        return Err(NO_PUBLIC_RELEASE_MESSAGE.to_owned());
    }
    if latest_tag.is_empty() {
        return Err("GitHub response did not include a release tag".to_owned());
    }
    let release_url = lines
        .next()
        .filter(|url| !url.is_empty())
        .unwrap_or(BABOON_RELEASES_URL)
        .to_owned();
    Ok(UpdateCheckResult {
        latest_tag,
        release_url,
    })
}

fn command_error(stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        "command exited without an error message".to_owned()
    } else {
        message
    }
}

fn loaded_source_status(source: &LoadedSourceData) -> String {
    match &source.source {
        TagSource::LooseFolder { .. } if source.all_entries.is_empty() => {
            format!("Browsing tags from {}", source.label)
        }
        TagSource::LooseFolder { .. } => {
            format!(
                "Found {} tag(s) in {}",
                source.all_entries.len(),
                source.label
            )
        }
        _ => format!(
            "Loaded {} tag(s) from {}",
            source.entries.len(),
            source.label
        ),
    }
}

fn update_check_status(result: &UpdateCheckResult) -> String {
    let current = env!("CARGO_PKG_VERSION");
    if is_newer_release(&result.latest_tag, current) {
        format!(
            "Update available: {} (current {}). {}",
            result.latest_tag, current, result.release_url
        )
    } else {
        format!("Baboon is up to date ({current})")
    }
}

fn is_newer_release(latest: &str, current: &str) -> bool {
    let latest = version_numbers(latest);
    let current = version_numbers(current);
    let max_len = latest.len().max(current.len());
    for index in 0..max_len {
        let latest_part = latest.get(index).copied().unwrap_or(0);
        let current_part = current.get(index).copied().unwrap_or(0);
        if latest_part != current_part {
            return latest_part > current_part;
        }
    }
    false
}

fn version_numbers(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

fn selected_tab_after_removal(
    open_tabs: &[String],
    removed_index: Option<usize>,
) -> Option<String> {
    let removed_index = removed_index?;
    if open_tabs.is_empty() {
        None
    } else {
        open_tabs
            .get(removed_index)
            .or_else(|| {
                removed_index
                    .checked_sub(1)
                    .and_then(|index| open_tabs.get(index))
            })
            .cloned()
    }
}

pub(super) fn available_definition_games() -> Vec<String> {
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

pub(super) fn load_new_tag_groups(game: &str) -> Result<Vec<NewTagGroup>, String> {
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

pub(super) fn new_tag_output_path(
    tags_root: &Path,
    rel_path: &str,
    extension: &str,
) -> Result<PathBuf, String> {
    let cleaned = rel_path.trim().replace('\\', "/");
    let cleaned = cleaned.trim_matches('/');
    if cleaned.is_empty() {
        return Err("Enter a relative tag path".to_owned());
    }
    // Tag paths never contain a colon; reject drive prefixes (e.g. `C:/…`)
    // explicitly since `Component::Prefix` is only produced on Windows.
    if cleaned.contains(':') {
        return Err("Tag path cannot contain a drive prefix or ':'".to_owned());
    }
    let rel = Path::new(cleaned);
    if rel.is_absolute() {
        return Err("Tag path must be relative to the tags folder".to_owned());
    }
    if rel.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::Prefix(_)
        )
    }) {
        return Err("Tag path cannot contain .. or a drive prefix".to_owned());
    }
    let mut output = tags_root.join(rel);
    output.set_extension(extension.trim_start_matches('.'));
    Ok(output)
}

pub(super) fn new_tag_output_path_from_dialog(
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

fn lexical_normalize_path(path: &Path) -> PathBuf {
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

type DependencyCandidateIndex = HashMap<(u32, String), Vec<String>>;

#[derive(Default)]
struct DependencyFixReport {
    scanned: usize,
    fixed: usize,
    already_ok: usize,
    unresolved: usize,
    ambiguous: usize,
    skipped: usize,
    lines: Vec<String>,
}

impl DependencyFixReport {
    fn status(&self) -> String {
        if self.fixed > 0 {
            format!(
                "Fixed {} dependenc{} ({} unresolved, {} ambiguous)",
                self.fixed,
                if self.fixed == 1 { "y" } else { "ies" },
                self.unresolved,
                self.ambiguous
            )
        } else if self.unresolved == 0 && self.ambiguous == 0 {
            format!(
                "No broken dependencies found across {} reference(s)",
                self.scanned
            )
        } else {
            format!(
                "No dependencies auto-fixed ({} unresolved, {} ambiguous)",
                self.unresolved, self.ambiguous
            )
        }
    }
}

#[derive(Clone, Debug)]
struct TagReferenceUse {
    field_path: String,
    group_tag: u32,
    rel_path: String,
}

fn fix_tag_dependencies_in_tag(
    tag: &mut TagFile,
    tags_root: &Path,
    names: &TagNameIndex,
    index: &DependencyCandidateIndex,
) -> DependencyFixReport {
    let mut refs = Vec::new();
    collect_tag_references(tag.root(), "", &mut refs);

    let mut report = DependencyFixReport {
        scanned: refs.len(),
        lines: vec![format!(
            "Fix Tag Dependencies: scanned {} reference(s)",
            refs.len()
        )],
        ..Default::default()
    };
    let mut fixes = Vec::new();
    for reference in refs {
        let Some(extension) = names
            .name_for(reference.group_tag)
            .or_else(|| group_tag_to_extension(reference.group_tag))
        else {
            report.skipped += 1;
            report.lines.push(format!(
                "Skipped {}: unknown group {}",
                reference.field_path,
                format_group_tag(reference.group_tag)
            ));
            continue;
        };
        if dependency_target_exists(tags_root, &reference.rel_path, extension) {
            report.already_ok += 1;
            continue;
        }

        let leaf = dependency_leaf_key(&reference.rel_path);
        let key = (reference.group_tag, leaf.clone());
        let candidates = index.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        match candidates {
            [candidate] if !candidate.eq_ignore_ascii_case(&reference.rel_path) => {
                fixes.push((reference.clone(), candidate.clone()));
            }
            [] => {
                report.unresolved += 1;
                report.lines.push(format!(
                    "Unresolved {}: {}",
                    reference.field_path,
                    format_reference_path(names, reference.group_tag, &reference.rel_path)
                ));
            }
            _ => {
                report.ambiguous += 1;
                report.lines.push(format!(
                    "Ambiguous {}: {} candidate(s) named {}.{}",
                    reference.field_path,
                    candidates.len(),
                    leaf,
                    extension
                ));
            }
        }
    }

    for (reference, fixed_path) in fixes {
        let mut root = tag.root_mut();
        let Some(mut field) = root.field_path_mut(&reference.field_path) else {
            report.unresolved += 1;
            report.lines.push(format!(
                "Skipped {}: field path no longer resolves",
                reference.field_path
            ));
            continue;
        };
        let result = field.set(TagFieldData::TagReference(TagReferenceData {
            group_tag_and_name: Some((reference.group_tag, fixed_path.clone())),
        }));
        match result {
            Ok(()) => {
                report.fixed += 1;
                report.lines.push(format!(
                    "Fixed {}: {} -> {}",
                    reference.field_path,
                    format_reference_path(names, reference.group_tag, &reference.rel_path),
                    format_reference_path(names, reference.group_tag, &fixed_path)
                ));
            }
            Err(error) => {
                report.unresolved += 1;
                report.lines.push(format!(
                    "Skipped {}: could not write dependency ({error:?})",
                    reference.field_path
                ));
            }
        }
    }

    report.lines.push(report.status());
    report
}

fn collect_tag_references(
    tag_struct: TagStruct<'_>,
    path_prefix: &str,
    refs: &mut Vec<TagReferenceUse>,
) {
    for field in tag_struct.fields() {
        let field_path = append_field_path(path_prefix, field.name());
        if let Some(value) = field.value() {
            if let TagFieldData::TagReference(reference) = value
                && let Some((group_tag, rel_path)) = reference.group_tag_and_name
            {
                let rel_path = sanitize_ref_path(&rel_path).replace('/', "\\");
                if !rel_path.is_empty() && !rel_path.eq_ignore_ascii_case("none") {
                    refs.push(TagReferenceUse {
                        field_path,
                        group_tag,
                        rel_path,
                    });
                }
            }
            continue;
        }
        if let Some(nested) = field.as_struct() {
            collect_tag_references(nested, &field_path, refs);
        } else if let Some(block) = field.as_block() {
            for (index, element) in block.iter().enumerate() {
                let element_path = format!("{field_path}[{index}]");
                collect_tag_references(element, &element_path, refs);
            }
        } else if let Some(array) = field.as_array() {
            for (index, element) in array.iter().enumerate() {
                let element_path = format!("{field_path}[{index}]");
                collect_tag_references(element, &element_path, refs);
            }
        }
    }
}

fn build_dependency_candidate_index(
    entries: &[TagEntry],
    names: &TagNameIndex,
) -> DependencyCandidateIndex {
    let mut index: DependencyCandidateIndex = HashMap::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let Some(rel_path) = dependency_entry_reference_path(entry, names) else {
            continue;
        };
        if !seen.insert((entry.group_tag, rel_path.to_ascii_lowercase())) {
            continue;
        }
        let leaf = dependency_leaf_key(&rel_path);
        index
            .entry((entry.group_tag, leaf))
            .or_default()
            .push(rel_path);
    }
    for candidates in index.values_mut() {
        candidates.sort();
    }
    index
}

/// Read each entry's tag and record the first field whose value contains
/// `query_lower`. Capped to keep result sets (and runtime) bounded.
fn run_field_value_search(
    source: &TagSource,
    entries: &[TagEntry],
    query_lower: &str,
) -> Result<Vec<FieldValueMatch>, String> {
    const MATCH_CAP: usize = 1000;
    let mut matches = Vec::new();
    for entry in entries {
        if matches.len() >= MATCH_CAP {
            break;
        }
        let Ok(tag) = crate::source::read_entry(source, entry) else {
            continue; // unreadable / unparseable tag — skip
        };
        if let Some((field_path, value)) = first_field_value_match(&tag.root(), query_lower, "") {
            matches.push(FieldValueMatch {
                entry: entry.clone(),
                label: format!("{field_path} = {}", truncate_field_value(&value)),
            });
        }
    }
    Ok(matches)
}

/// Map each pasted TSV header column → the full stored field name to write
/// (matched case-insensitively against the block's cleaned leaf-column names),
/// or `None` for columns that don't correspond to a writable field.
fn map_tsv_header_to_fields(
    header_line: &str,
    columns: &[(String, String)],
) -> Vec<Option<String>> {
    header_line
        .split('\t')
        .map(|raw| {
            let clean = raw.trim();
            columns
                .iter()
                .find(|(col_clean, _)| col_clean.eq_ignore_ascii_case(clean))
                .map(|(_, full)| full.clone())
        })
        .collect()
}

/// Build the searchable-text index: for each tag, a lowercased blob of all its
/// string / string_id / reference / enum values. Tags with no searchable text
/// are omitted.
fn build_field_value_index(source: &TagSource, entries: &[TagEntry]) -> Vec<(String, String)> {
    let mut blobs = Vec::new();
    for entry in entries {
        let Ok(tag) = crate::source::read_entry(source, entry) else {
            continue;
        };
        let mut blob = String::new();
        collect_searchable_text(&tag.root(), &mut blob, 0);
        if !blob.is_empty() {
            blobs.push((entry.key.clone(), blob));
        }
    }
    blobs
}

/// Append every leaf field's lowercased searchable text into `blob`, separated
/// by " · ", bounded in size and recursion depth.
fn collect_searchable_text(element: &TagStruct, blob: &mut String, depth: usize) {
    const CAP: usize = 4000;
    if blob.len() >= CAP || depth > 32 {
        return;
    }
    for field in element.fields() {
        if blob.len() >= CAP {
            return;
        }
        if let Some(block) = field.as_block() {
            for index in 0..block.len() {
                if let Some(child) = block.element(index) {
                    collect_searchable_text(&child, blob, depth + 1);
                }
                if blob.len() >= CAP {
                    return;
                }
            }
            continue;
        }
        if let Some(nested) = field.as_struct() {
            collect_searchable_text(&nested, blob, depth + 1);
            continue;
        }
        if let Some(text) = field_searchable_text(field.value()) {
            if !text.is_empty() {
                if !blob.is_empty() {
                    blob.push_str(" · ");
                }
                blob.push_str(&text.to_ascii_lowercase());
            }
        }
    }
}

/// Depth-first search for the first field whose searchable text contains
/// `query_lower`. Returns the cleaned field path and the matched value.
fn first_field_value_match(
    element: &TagStruct,
    query_lower: &str,
    path: &str,
) -> Option<(String, String)> {
    for field in element.fields() {
        let clean = clean_field_name(field.name());
        let field_path = if path.is_empty() {
            clean.clone()
        } else {
            format!("{path}/{clean}")
        };
        if let Some(block) = field.as_block() {
            for index in 0..block.len() {
                if let Some(child) = block.element(index) {
                    if let Some(hit) = first_field_value_match(
                        &child,
                        query_lower,
                        &format!("{field_path}[{index}]"),
                    ) {
                        return Some(hit);
                    }
                }
            }
            continue;
        }
        if let Some(nested) = field.as_struct() {
            if let Some(hit) = first_field_value_match(&nested, query_lower, &field_path) {
                return Some(hit);
            }
            continue;
        }
        if let Some(text) = field_searchable_text(field.value()) {
            if !text.is_empty() && text.to_ascii_lowercase().contains(query_lower) {
                return Some((field_path, text));
            }
        }
    }
    None
}

/// The user-visible text of a leaf field for searching, or `None` for fields
/// with no meaningful text (raw numbers, padding, data blobs, …).
fn field_searchable_text(value: Option<TagFieldData>) -> Option<String> {
    match value? {
        TagFieldData::String(s) | TagFieldData::LongString(s) => Some(s),
        TagFieldData::StringId(d) | TagFieldData::OldStringId(d) => Some(d.string),
        TagFieldData::TagReference(r) => r.group_tag_and_name.map(|(_, path)| path),
        TagFieldData::CharEnum { name, .. }
        | TagFieldData::ShortEnum { name, .. }
        | TagFieldData::LongEnum { name, .. } => name,
        _ => None,
    }
}

fn truncate_field_value(value: &str) -> String {
    const MAX: usize = 80;
    if value.chars().count() > MAX {
        let head: String = value.chars().take(MAX).collect();
        format!("{head}…")
    } else {
        value.to_owned()
    }
}

fn dependency_entry_reference_path(entry: &TagEntry, names: &TagNameIndex) -> Option<String> {
    let extension = names
        .name_for(entry.group_tag)
        .or_else(|| group_tag_to_extension(entry.group_tag));
    let mut path = entry.display_path.replace('/', "\\");
    if let Some(extension) = extension {
        let suffix = format!(".{extension}");
        if path
            .to_ascii_lowercase()
            .ends_with(&suffix.to_ascii_lowercase())
        {
            let keep = path.len().saturating_sub(suffix.len());
            path.truncate(keep);
            return Some(path);
        }
    }
    Path::new(&path)
        .with_extension("")
        .to_str()
        .map(|path| path.replace('/', "\\"))
}

fn dependency_leaf_key(rel_path: &str) -> String {
    rel_path
        .replace('/', "\\")
        .rsplit('\\')
        .next()
        .unwrap_or(rel_path)
        .to_ascii_lowercase()
}

fn dependency_target_exists(tags_root: &Path, rel_path: &str, extension: &str) -> bool {
    resolve_tag_path(tags_root, rel_path, extension).is_file()
}

#[derive(Default)]
struct ReferenceRewriteResult {
    references_changed: usize,
    tags_changed: usize,
    changed_keys: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
/// Rename/move a SINGLE tag and rewrite every reference to it, mirroring
/// [`run_folder_refactor_job`] for one file with an explicit new relative path
/// (no extension). Reuses the same reference-rewrite + key-remap machinery and
/// returns a [`FolderRefactorFinished`] so the existing finish handler applies
/// the in-memory update.
#[allow(clippy::too_many_arguments)]
fn run_tag_rename_job(
    root: PathBuf,
    entry: TagEntry,
    new_rel: String,
    names: TagNameIndex,
    game: Option<String>,
    all_entries_before: Vec<TagEntry>,
    existing_reverse_dependencies: Option<ReverseDependencyIndex>,
    tx: &Sender<WorkerMessage>,
) -> Result<FolderRefactorFinished, String> {
    let label = "Renaming tag".to_owned();
    send_folder_refactor_progress(tx, &label, "Preparing", None);
    let root = lexical_normalize_path(&root);

    let TagEntryLocation::LooseFile(old_path) = &entry.location else {
        return Err("Only loose-folder tags can be renamed".to_owned());
    };
    let old_path = lexical_normalize_path(old_path);
    if !old_path.is_file() {
        return Err(format!("Source tag not found: {}", old_path.display()));
    }
    let extension = old_path
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| "Tag file has no extension".to_owned())?
        .to_owned();

    // Compute the destination absolute path from the (extension-less) new rel.
    let new_rel_norm = new_rel.replace('\\', "/");
    if new_rel_norm
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err("Destination path is not a valid relative path".to_owned());
    }
    let new_path = lexical_normalize_path(&root.join(format!("{new_rel_norm}.{extension}")));
    if !new_path.starts_with(&root) {
        return Err("Destination escapes the tags folder".to_owned());
    }
    if new_path == old_path {
        return Err("New path is the same as the current one".to_owned());
    }
    if new_path.exists() {
        return Err(format!(
            "A tag already exists at the destination: {}",
            new_path.display()
        ));
    }

    // The one rewrite: old reference path → new reference path (same group).
    let old_ref = reference_path_from_abs_file(&root, &old_path, entry.group_tag, &names)
        .ok_or_else(|| "Could not resolve the tag's reference path".to_owned())?;
    let new_ref = reference_path_from_abs_file(&root, &new_path, entry.group_tag, &names)
        .ok_or_else(|| "Could not resolve the destination reference path".to_owned())?;
    let mut rewrites = HashMap::new();
    rewrites.insert((entry.group_tag, old_ref.to_ascii_lowercase()), new_ref);

    // Ensure a reverse-dependency index so we only rewrite actual referrers.
    let mut reverse_dependencies = existing_reverse_dependencies.or_else(|| {
        game.as_deref()
            .and_then(|game| crate::source::load_reverse_dependency_index(game, &root))
    });
    if let Some(index) = reverse_dependencies.as_ref()
        && index.len() != all_entries_before.len()
    {
        reverse_dependencies = None; // stale → rebuild below
    }
    let dependency_source = TagSource::LooseFolder {
        root: root.clone(),
        game: game.clone(),
        definitions_root: locate_definitions_root(),
    };
    if reverse_dependencies.is_none() {
        reverse_dependencies = Some(build_reverse_dependency_index(
            &root,
            &dependency_source,
            &all_entries_before,
            &label,
            tx,
        ));
    }
    let dependency_schema_path = game
        .as_deref()
        .map(|game| {
            locate_definitions_root()
                .join(game)
                .join("tag_dependency_list.json")
        })
        .filter(|path| path.is_file());

    // The moved entry, post-rename.
    let new_display = new_path
        .strip_prefix(&root)
        .unwrap_or(&new_path)
        .to_string_lossy()
        .replace('\\', "/");
    let new_entry = TagEntry {
        key: format!("file:{}", new_path.display()),
        display_path: new_display,
        group_tag: entry.group_tag,
        group_name: entry.group_name.clone(),
        location: TagEntryLocation::LooseFile(new_path.clone()),
    };
    let old_entries = vec![entry.clone()];
    let new_entries = vec![new_entry.clone()];

    // Move the file on disk.
    if let Some(parent) = new_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    fs::rename(&old_path, &new_path).map_err(|error| {
        format!(
            "Could not move {} to {}: {error}",
            old_path.display(),
            new_path.display()
        )
    })?;

    // Rewrite references in the affected (referring) tags.
    let rewrite_entries = affected_move_rewrite_entries(
        &all_entries_before,
        &old_entries,
        &new_entries,
        &rewrites,
        reverse_dependencies.as_ref(),
    );
    send_folder_refactor_progress(
        tx,
        &label,
        &format!("Rewriting {} affected tag(s)", rewrite_entries.len()),
        None,
    );
    let rewrite_result = rewrite_references_in_entries(
        &dependency_source,
        &rewrite_entries,
        &rewrites,
        &label,
        tx,
        dependency_schema_path.as_deref(),
    )?;
    let references_changed = rewrite_result.references_changed;
    let tags_changed = rewrite_result.tags_changed;

    // Rebuild browser tree + entry set + key map.
    send_folder_refactor_progress(tx, &label, "Refreshing browser", None);
    let tree = crate::source::build_folder_directory_tree(&root).map_err(|e| e.to_string())?;
    let all_entries =
        merge_refactored_entries(all_entries_before, &old_entries, &new_entries, true);
    let mut old_to_new_keys = HashMap::new();
    old_to_new_keys.insert(entry.key.clone(), new_entry.key.clone());
    if let Some(index) = reverse_dependencies.as_mut() {
        refresh_reverse_dependency_index_after_refactor(
            index,
            &dependency_source,
            true,
            &old_entries,
            &new_entries,
            &rewrite_result.changed_keys,
            &all_entries,
        );
    }

    let status =
        format!("Renamed tag, updated {references_changed} reference(s) in {tags_changed} tag(s)");
    let lines = vec![
        format!(
            "Renamed: {} -> {}",
            entry.display_path, new_entry.display_path
        ),
        format!("Updated {references_changed} reference(s) in {tags_changed} tag(s)"),
    ];
    Ok(FolderRefactorFinished {
        status,
        lines,
        tree,
        all_entries,
        reverse_dependencies,
        old_to_new_keys,
        moved: true,
    })
}

fn run_folder_refactor_job(
    root: PathBuf,
    rel_path: PathBuf,
    destination_parent: PathBuf,
    move_folder: bool,
    label: String,
    names: TagNameIndex,
    game: Option<String>,
    existing_all_entries: Vec<TagEntry>,
    existing_reverse_dependencies: Option<ReverseDependencyIndex>,
    tx: &Sender<WorkerMessage>,
) -> Result<FolderRefactorFinished, String> {
    send_folder_refactor_progress(tx, &label, "Preparing", None);
    let root = lexical_normalize_path(&root);
    let source_rel = validate_relative_folder_path(&rel_path)?;
    let source = lexical_normalize_path(&root.join(&source_rel));
    if !source.is_dir() {
        return Err(format!("Folder not found: {}", source.display()));
    }
    let destination_parent = lexical_normalize_path(&destination_parent);
    if !destination_parent.starts_with(&root) {
        return Err("Choose a destination inside the loaded tags folder".to_owned());
    }
    let folder_name = source
        .file_name()
        .ok_or_else(|| "Cannot move/copy the tags root itself".to_owned())?;
    let destination = lexical_normalize_path(&destination_parent.join(folder_name));
    if destination == source {
        return Err("Source and destination are the same folder".to_owned());
    }
    if destination.starts_with(&source) {
        return Err("Cannot move/copy a folder into itself".to_owned());
    }
    if destination.exists() {
        return Err(format!(
            "Destination already exists: {}",
            destination.display()
        ));
    }

    send_folder_refactor_progress(tx, &label, "Scanning selected folder", None);
    let old_entries =
        scan_folder_subtree_entries(&root, &source_rel, &names).map_err(|e| e.to_string())?;
    if old_entries.is_empty() {
        return Err("No tags found in that folder".to_owned());
    }
    let rewrites =
        build_folder_reference_rewrites(&root, &source, &destination, &old_entries, &names);
    let all_entries_before = if move_folder && existing_all_entries.is_empty() {
        send_folder_refactor_progress(tx, &label, "Building tag database", None);
        scan_folder_subtree_entries(&root, Path::new(""), &names).map_err(|e| e.to_string())?
    } else {
        existing_all_entries.clone()
    };
    let mut reverse_dependencies = existing_reverse_dependencies.or_else(|| {
        game.as_deref()
            .and_then(|game| crate::source::load_reverse_dependency_index(game, &root))
    });
    if move_folder
        && let Some(index) = reverse_dependencies.as_ref()
        && index.len() != all_entries_before.len()
    {
        let _ = tx.send(WorkerMessage::TerminalLine(format!(
            "Dependency index is stale ({} indexed tag(s), {} current tag(s)); rebuilding",
            index.len(),
            all_entries_before.len()
        )));
        reverse_dependencies = None;
    }
    if move_folder && reverse_dependencies.is_none() {
        let dependency_source = TagSource::LooseFolder {
            root: root.clone(),
            game: game.clone(),
            definitions_root: locate_definitions_root(),
        };
        reverse_dependencies = Some(build_reverse_dependency_index(
            &root,
            &dependency_source,
            &all_entries_before,
            &label,
            tx,
        ));
    }
    let dependency_schema_path = game
        .as_deref()
        .map(|game| {
            locate_definitions_root()
                .join(game)
                .join("tag_dependency_list.json")
        })
        .filter(|path| path.is_file());
    let rewrite_source = TagSource::LooseFolder {
        root: root.clone(),
        game: game.clone(),
        definitions_root: locate_definitions_root(),
    };

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    if move_folder {
        send_folder_refactor_progress(tx, &label, "Moving files", Some(0.15));
        fs::rename(&source, &destination).map_err(|error| {
            format!(
                "Could not move {} to {}: {error}",
                source.display(),
                destination.display()
            )
        })?;
    } else {
        copy_folder_recursive_progress(&source, &destination, &label, tx)?;
    }

    let new_entries = transform_folder_entries(&root, &source, &old_entries, &destination);
    let rewrite_result = if move_folder {
        let rewrite_entries = affected_move_rewrite_entries(
            &all_entries_before,
            &old_entries,
            &new_entries,
            &rewrites,
            reverse_dependencies.as_ref(),
        );
        send_folder_refactor_progress(
            tx,
            &label,
            &format!("Rewriting {} affected tag(s)", rewrite_entries.len()),
            None,
        );
        rewrite_references_in_entries(
            &rewrite_source,
            &rewrite_entries,
            &rewrites,
            &label,
            tx,
            dependency_schema_path.as_deref(),
        )?
    } else {
        send_folder_refactor_progress(tx, &label, "Rewriting copied references", None);
        rewrite_references_in_entries(
            &rewrite_source,
            &new_entries,
            &rewrites,
            &label,
            tx,
            dependency_schema_path.as_deref(),
        )?
    };
    let references_changed = rewrite_result.references_changed;
    let tags_changed = rewrite_result.tags_changed;

    send_folder_refactor_progress(tx, &label, "Refreshing browser", None);
    let tree = crate::source::build_folder_directory_tree(&root).map_err(|e| e.to_string())?;
    let all_entries = if move_folder {
        merge_refactored_entries(all_entries_before, &old_entries, &new_entries, true)
    } else if existing_all_entries.is_empty() {
        Vec::new()
    } else {
        merge_refactored_entries(
            existing_all_entries,
            &old_entries,
            &new_entries,
            move_folder,
        )
    };
    let old_to_new_keys = if move_folder {
        moved_key_map(&root, &source, &old_entries, &destination)
    } else {
        HashMap::new()
    };
    if let Some(index) = reverse_dependencies.as_mut() {
        let dependency_source = TagSource::LooseFolder {
            root: root.clone(),
            game: game.clone(),
            definitions_root: locate_definitions_root(),
        };
        refresh_reverse_dependency_index_after_refactor(
            index,
            &dependency_source,
            move_folder,
            &old_entries,
            &new_entries,
            &rewrite_result.changed_keys,
            &all_entries,
        );
    }

    let action = if move_folder { "Moved" } else { "Copied" };
    let status = format!(
        "{action} {} tag(s), updated {} reference(s) in {} tag(s)",
        old_entries.len(),
        references_changed,
        tags_changed
    );
    let mut lines = vec![format!(
        "{action} folder: {} -> {}",
        source.strip_prefix(&root).unwrap_or(&source).display(),
        destination
            .strip_prefix(&root)
            .unwrap_or(&destination)
            .display()
    )];
    lines.push(format!(
        "Updated {references_changed} reference(s) in {tags_changed} tag(s)"
    ));

    Ok(FolderRefactorFinished {
        status,
        lines,
        tree,
        all_entries,
        reverse_dependencies,
        old_to_new_keys,
        moved: move_folder,
    })
}

fn send_folder_refactor_progress(
    tx: &Sender<WorkerMessage>,
    label: &str,
    phase: &str,
    progress: Option<f32>,
) {
    let _ = tx.send(WorkerMessage::FolderRefactorProgress(
        FolderRefactorProgress {
            label: label.to_owned(),
            phase: phase.to_owned(),
            progress,
        },
    ));
}

fn validate_relative_folder_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err("Choose a folder inside the loaded tags folder".to_owned());
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::Prefix(_)
        )
    }) {
        return Err("Folder path cannot contain .. or a drive prefix".to_owned());
    }
    Ok(path.to_path_buf())
}

fn copy_folder_recursive_progress(
    source: &Path,
    destination: &Path,
    label: &str,
    tx: &Sender<WorkerMessage>,
) -> Result<(), String> {
    let items = walkdir::WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let file_total = items
        .iter()
        .filter(|item| item.file_type().is_file())
        .count();
    let mut copied = 0usize;
    for item in items {
        let rel = item
            .path()
            .strip_prefix(source)
            .map_err(|error| error.to_string())?;
        let target = destination.join(rel);
        if item.file_type().is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| format!("Could not create {}: {error}", target.display()))?;
        } else if item.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
            }
            fs::copy(item.path(), &target).map_err(|error| {
                format!(
                    "Could not copy {} to {}: {error}",
                    item.path().display(),
                    target.display()
                )
            })?;
            copied += 1;
            if copied == 1 || copied % 25 == 0 || copied == file_total {
                let progress = if file_total == 0 {
                    None
                } else {
                    Some(copied as f32 / file_total as f32)
                };
                send_folder_refactor_progress(
                    tx,
                    label,
                    &format!("Copying files {copied}/{file_total}"),
                    progress,
                );
            }
        }
    }
    Ok(())
}

fn build_folder_reference_rewrites(
    tags_root: &Path,
    source: &Path,
    destination: &Path,
    old_entries: &[TagEntry],
    names: &TagNameIndex,
) -> HashMap<(u32, String), String> {
    let mut rewrites = HashMap::new();
    for entry in old_entries {
        let TagEntryLocation::LooseFile(old_path) = &entry.location else {
            continue;
        };
        let Some(old_ref) =
            reference_path_from_abs_file(tags_root, old_path, entry.group_tag, names)
        else {
            continue;
        };
        let Ok(inner_rel) = old_path.strip_prefix(source) else {
            continue;
        };
        let new_path = destination.join(inner_rel);
        let Some(new_ref) =
            reference_path_from_abs_file(tags_root, &new_path, entry.group_tag, names)
        else {
            continue;
        };
        rewrites.insert((entry.group_tag, old_ref.to_ascii_lowercase()), new_ref);
    }
    rewrites
}

fn transform_folder_entries(
    tags_root: &Path,
    source: &Path,
    old_entries: &[TagEntry],
    destination: &Path,
) -> Vec<TagEntry> {
    old_entries
        .iter()
        .filter_map(|entry| {
            let TagEntryLocation::LooseFile(old_path) = &entry.location else {
                return None;
            };
            let inner_rel = old_path.strip_prefix(source).ok()?;
            let new_path = destination.join(inner_rel);
            let display_path = new_path
                .strip_prefix(tags_root)
                .unwrap_or(&new_path)
                .to_string_lossy()
                .replace('\\', "/");
            Some(TagEntry {
                key: format!("file:{}", new_path.display()),
                display_path,
                group_tag: entry.group_tag,
                group_name: entry.group_name.clone(),
                location: TagEntryLocation::LooseFile(new_path),
            })
        })
        .collect()
}

fn merge_refactored_entries(
    mut all_entries: Vec<TagEntry>,
    old_entries: &[TagEntry],
    new_entries: &[TagEntry],
    moved: bool,
) -> Vec<TagEntry> {
    let old_keys = old_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<HashSet<_>>();
    if moved {
        all_entries.retain(|entry| !old_keys.contains(&entry.key));
    }
    let existing = all_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<HashSet<_>>();
    all_entries.extend(
        new_entries
            .iter()
            .filter(|entry| !existing.contains(&entry.key))
            .cloned(),
    );
    all_entries.sort_by(|a, b| a.display_path.cmp(&b.display_path));
    all_entries
}

fn affected_move_rewrite_entries(
    all_entries: &[TagEntry],
    old_entries: &[TagEntry],
    new_entries: &[TagEntry],
    rewrites: &HashMap<(u32, String), String>,
    reverse_dependencies: Option<&ReverseDependencyIndex>,
) -> Vec<TagEntry> {
    let old_keys = old_entries
        .iter()
        .map(|entry| entry.key.as_str())
        .collect::<HashSet<_>>();
    let mut entries_by_key = all_entries
        .iter()
        .map(|entry| (entry.key.clone(), entry.clone()))
        .collect::<HashMap<_, _>>();
    for entry in new_entries {
        entries_by_key.insert(entry.key.clone(), entry.clone());
    }

    let mut affected = new_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<HashSet<_>>();
    if let Some(index) = reverse_dependencies {
        for ((group_tag, old_ref), _) in rewrites {
            for dependent_key in index.dependents_for(*group_tag, old_ref) {
                if !old_keys.contains(dependent_key.as_str()) {
                    affected.insert(dependent_key.clone());
                }
            }
        }
    } else {
        affected.extend(all_entries.iter().map(|entry| entry.key.clone()));
    }

    let mut entries = affected
        .into_iter()
        .filter_map(|key| entries_by_key.get(&key).cloned())
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| natural_entry_order(a).cmp(&natural_entry_order(b)));
    entries
}

fn natural_entry_order(entry: &TagEntry) -> String {
    entry.display_path.to_ascii_lowercase().replace('\\', "/")
}

fn rewrite_references_in_entries(
    source: &TagSource,
    entries: &[TagEntry],
    rewrites: &HashMap<(u32, String), String>,
    label: &str,
    tx: &Sender<WorkerMessage>,
    dependency_schema_path: Option<&Path>,
) -> Result<ReferenceRewriteResult, String> {
    let mut result = ReferenceRewriteResult::default();
    let needles = rewrite_reference_needles(rewrites);
    if needles.is_empty() {
        return Ok(result);
    }
    let total = entries.len();
    for (index, entry) in entries.iter().enumerate() {
        let TagEntryLocation::LooseFile(path) = &entry.location else {
            continue;
        };
        if index == 0 || (index + 1) % 25 == 0 || index + 1 == total {
            let progress = if total == 0 {
                None
            } else {
                Some((index + 1) as f32 / total as f32)
            };
            send_folder_refactor_progress(
                tx,
                label,
                &format!("Rewriting affected references {}/{}", index + 1, total),
                progress,
            );
        }
        let bytes = fs::read(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        if !bytes_contain_any_ascii_case_insensitive(&bytes, &needles) {
            continue;
        }
        send_folder_refactor_progress(
            tx,
            label,
            &format!(
                "Rewriting {}",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("tag")
            ),
            None,
        );
        let mut tag =
            read_entry(source, entry).map_err(|error| format!("Could not parse tag: {error}"))?;
        let changed = rewrite_references_in_tag(&mut tag, rewrites);
        if changed == 0 {
            continue;
        }
        if tag.classic_engine().is_none()
            && let Some(schema_path) = dependency_schema_path
            && let Err(error) = tag.rebuild_dependency_list(schema_path)
        {
            let _ = tx.send(WorkerMessage::TerminalLine(format!(
                "Warning: could not rebuild dependency list for {}: {error}",
                entry.display_path
            )));
        }
        tag.write_atomic(&path)
            .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
        result.references_changed += changed;
        result.tags_changed += 1;
        result.changed_keys.push(entry.key.clone());
    }
    Ok(result)
}

fn build_reverse_dependency_index(
    root: &Path,
    source: &TagSource,
    entries: &[TagEntry],
    label: &str,
    tx: &Sender<WorkerMessage>,
) -> ReverseDependencyIndex {
    let mut index = ReverseDependencyIndex::default();
    let total = entries.len();
    for (entry_index, entry) in entries.iter().enumerate() {
        if entry_index == 0 || (entry_index + 1) % 50 == 0 || entry_index + 1 == total {
            let progress = if total == 0 {
                None
            } else {
                Some((entry_index + 1) as f32 / total as f32)
            };
            send_folder_refactor_progress(
                tx,
                label,
                &format!("Building dependency index {}/{}", entry_index + 1, total),
                progress,
            );
        }
        let deps = match read_entry_dependencies(source, entry) {
            Ok(deps) => deps,
            Err(error) => {
                let _ = tx.send(WorkerMessage::TerminalLine(format!(
                    "Warning: skipped dependency index for {}: {error}",
                    entry.display_path
                )));
                continue;
            }
        };
        index.set_tag_dependencies(entry.key.clone(), deps);
    }
    let _ = tx.send(WorkerMessage::TerminalLine(format!(
        "Built dependency index for {} tag(s) under {}",
        index.len(),
        root.display()
    )));
    index
}

fn refresh_reverse_dependency_index_after_refactor(
    index: &mut ReverseDependencyIndex,
    source: &TagSource,
    moved: bool,
    old_entries: &[TagEntry],
    new_entries: &[TagEntry],
    changed_keys: &[String],
    all_entries: &[TagEntry],
) {
    if moved {
        for entry in old_entries {
            index.clear_tag(&entry.key);
        }
    }
    let entries_by_key = all_entries
        .iter()
        .map(|entry| (entry.key.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let mut refresh_keys = new_entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<HashSet<_>>();
    refresh_keys.extend(changed_keys.iter().cloned());
    for key in refresh_keys {
        let Some(entry) = entries_by_key.get(key.as_str()) else {
            continue;
        };
        if let Ok(deps) = read_entry_dependencies(source, entry) {
            index.set_tag_dependencies(entry.key.clone(), deps);
        }
    }
}

fn read_entry_dependencies(
    source: &TagSource,
    entry: &TagEntry,
) -> Result<Vec<DependencyRef>, String> {
    let TagEntryLocation::LooseFile(path) = &entry.location else {
        return Ok(Vec::new());
    };
    if let Some(refs) = TagFile::read_dependency_references(path)
        .map_err(|error| format!("Could not read dependency list: {error}"))?
    {
        return Ok(refs
            .into_iter()
            .map(|(group_tag, rel_path)| DependencyRef {
                group_tag,
                rel_path: sanitize_ref_path(&rel_path).replace('/', "\\"),
            })
            .collect());
    }
    let tag = read_entry(source, entry).map_err(|error| format!("Could not parse tag: {error}"))?;
    let mut refs = Vec::new();
    collect_tag_references(tag.root(), "", &mut refs);
    Ok(refs
        .into_iter()
        .map(|reference| DependencyRef {
            group_tag: reference.group_tag,
            rel_path: reference.rel_path,
        })
        .collect())
}

fn rewrite_reference_needles(rewrites: &HashMap<(u32, String), String>) -> Vec<Vec<u8>> {
    let mut seen = HashSet::new();
    rewrites
        .keys()
        .filter_map(|(_, old_ref)| {
            let lowered = old_ref.replace('/', "\\").to_ascii_lowercase().into_bytes();
            (!lowered.is_empty() && seen.insert(lowered.clone())).then_some(lowered)
        })
        .collect()
}

fn bytes_contain_any_ascii_case_insensitive(bytes: &[u8], needles: &[Vec<u8>]) -> bool {
    if needles.is_empty() || bytes.is_empty() {
        return false;
    }
    let lowered = bytes.to_ascii_lowercase();
    needles
        .iter()
        .any(|needle| contains_subslice(&lowered, needle.as_slice()))
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && needle.len() <= haystack.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn rewrite_references_in_tag(
    tag: &mut TagFile,
    rewrites: &HashMap<(u32, String), String>,
) -> usize {
    let mut refs = Vec::new();
    collect_tag_references(tag.root(), "", &mut refs);
    let mut changed = 0usize;
    for reference in refs {
        let key = (reference.group_tag, reference.rel_path.to_ascii_lowercase());
        let Some(new_path) = rewrites.get(&key) else {
            continue;
        };
        if new_path.eq_ignore_ascii_case(&reference.rel_path) {
            continue;
        }
        let mut root = tag.root_mut();
        let Some(mut field) = root.field_path_mut(&reference.field_path) else {
            continue;
        };
        if field
            .set(TagFieldData::TagReference(TagReferenceData {
                group_tag_and_name: Some((reference.group_tag, new_path.clone())),
            }))
            .is_ok()
        {
            changed += 1;
        }
    }
    changed
}

fn moved_key_map(
    tags_root: &Path,
    source: &Path,
    old_entries: &[TagEntry],
    destination: &Path,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for entry in old_entries {
        let TagEntryLocation::LooseFile(old_path) = &entry.location else {
            continue;
        };
        let Ok(inner_rel) = old_path.strip_prefix(source) else {
            continue;
        };
        let new_path = destination.join(inner_rel);
        if new_path.starts_with(tags_root) {
            map.insert(entry.key.clone(), format!("file:{}", new_path.display()));
        }
    }
    map
}

fn remap_open_tag_keys(keys: &mut Vec<String>, map: &HashMap<String, String>) {
    for key in keys.iter_mut() {
        if let Some(new_key) = map.get(key) {
            *key = new_key.clone();
        }
    }
    let mut seen = HashSet::new();
    keys.retain(|key| seen.insert(key.clone()));
}

fn remap_hashset_keys(keys: &mut HashSet<String>, map: &HashMap<String, String>) {
    if keys.is_empty() {
        return;
    }
    *keys = keys
        .drain()
        .map(|key| map.get(&key).cloned().unwrap_or(key))
        .collect();
}

fn reference_path_from_abs_file(
    tags_root: &Path,
    path: &Path,
    group_tag: u32,
    names: &TagNameIndex,
) -> Option<String> {
    let rel = path.strip_prefix(tags_root).ok()?;
    reference_path_from_rel_file(rel, group_tag, names)
}

fn reference_path_from_rel_file(
    rel_file: &Path,
    group_tag: u32,
    names: &TagNameIndex,
) -> Option<String> {
    let mut rel = rel_file.to_string_lossy().replace('/', "\\");
    if let Some(extension) = names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag))
    {
        let suffix = format!(".{extension}");
        if rel
            .to_ascii_lowercase()
            .ends_with(&suffix.to_ascii_lowercase())
        {
            rel.truncate(rel.len().saturating_sub(suffix.len()));
            return Some(rel);
        }
    }
    Path::new(&rel)
        .with_extension("")
        .to_str()
        .map(|path| path.replace('/', "\\"))
}

#[cfg(test)]
mod tsv_paste_tests {
    use super::*;

    #[test]
    fn header_maps_case_insensitively_reordered_and_ignores_unknown() {
        let columns = vec![
            ("material name".to_owned(), "material name^".to_owned()),
            ("sweetener mode".to_owned(), "sweetener mode".to_owned()),
        ];
        // Reordered, mixed case, plus an unknown column.
        let mapped = map_tsv_header_to_fields("Sweetener Mode\tbogus\tmaterial name", &columns);
        assert_eq!(
            mapped,
            vec![
                Some("sweetener mode".to_owned()),
                None,
                Some("material name^".to_owned()),
            ]
        );
    }
}

#[cfg(test)]
mod field_search_tests {
    use super::*;

    #[test]
    fn searchable_text_extracts_text_kinds_only() {
        assert_eq!(
            field_searchable_text(Some(TagFieldData::String("Hello".to_owned()))).as_deref(),
            Some("Hello")
        );
        assert_eq!(
            field_searchable_text(Some(TagFieldData::CharEnum {
                value: 1,
                name: Some("alert".to_owned()),
            }))
            .as_deref(),
            Some("alert")
        );
        // Numbers / padding carry no searchable text.
        assert_eq!(
            field_searchable_text(Some(TagFieldData::LongInteger(42))),
            None
        );
        assert_eq!(field_searchable_text(None), None);
    }

    #[test]
    fn first_match_finds_a_string_id_value_and_path() {
        let mut tag = TagFile::new("definitions/halo2_mcc/model.json").unwrap();
        let mut dirty = false;
        apply_model_variant_ops(
            &mut tag,
            vec![ModelVariantOp::Create {
                name: "myhero".to_owned(),
                regions: Vec::new(),
            }],
            &mut dirty,
        );
        let hit = first_field_value_match(&tag.root(), "hero", "");
        let (path, value) = hit.expect("variant name should match 'hero'");
        assert!(value.to_ascii_lowercase().contains("hero"));
        assert!(path.to_ascii_lowercase().contains("variant"));
        assert!(
            first_field_value_match(&tag.root(), "zzz-not-present", "").is_none(),
            "absent text should not match"
        );
    }
}

#[cfg(test)]
mod dependency_tests {
    use super::*;

    fn entry(display_path: &str, group_tag: u32) -> TagEntry {
        TagEntry {
            key: format!("file:{display_path}"),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
        }
    }

    fn abs_entry(root: &Path, display_path: &str, group_tag: u32) -> TagEntry {
        TagEntry {
            key: format!("file:{}", root.join(display_path).display()),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: None,
            location: TagEntryLocation::LooseFile(root.join(display_path)),
        }
    }

    fn loose_source_with_counts(label: &str, entries: Vec<TagEntry>) -> LoadedSourceData {
        LoadedSourceData {
            label: label.to_owned(),
            source: TagSource::LooseFolder {
                root: PathBuf::from("C:/kit/tags"),
                game: Some("halo3_mcc".to_owned()),
                definitions_root: PathBuf::from("C:/kit/definitions"),
            },
            names: TagNameIndex::default(),
            game: Some("halo3_mcc".to_owned()),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: entries,
            reverse_dependencies: None,
            initial_tag: None,
        }
    }

    #[test]
    fn loose_folder_status_does_not_report_zero_loaded_tags_before_scan() {
        let source = loose_source_with_counts("H3EK/tags (halo3_mcc)", Vec::new());

        assert_eq!(
            loaded_source_status(&source),
            "Browsing tags from H3EK/tags (halo3_mcc)"
        );
    }

    #[test]
    fn loose_folder_status_uses_recursive_index_when_available() {
        let shader = parse_group_tag("rmsh").unwrap();
        let source = loose_source_with_counts(
            "H3EK/tags (halo3_mcc)",
            vec![
                entry("objects/a.shader", shader),
                entry("objects/b.shader", shader),
            ],
        );

        assert_eq!(
            loaded_source_status(&source),
            "Found 2 tag(s) in H3EK/tags (halo3_mcc)"
        );
    }

    #[test]
    fn dependency_entry_reference_path_strips_only_group_extension() {
        let names = TagNameIndex::default();
        let bitmap = parse_group_tag("bitm").unwrap();
        let entry = entry("objects/weapons/decal_road_1.bitmap.bitmap", bitmap);

        assert_eq!(
            dependency_entry_reference_path(&entry, &names).unwrap(),
            "objects\\weapons\\decal_road_1.bitmap"
        );
    }

    #[test]
    fn dependency_candidate_index_matches_by_group_and_leaf_name() {
        let names = TagNameIndex::default();
        let bitmap = parse_group_tag("bitm").unwrap();
        let shader = parse_group_tag("rmsh").unwrap();
        let entries = vec![
            entry("objects/new/run.bitmap", bitmap),
            entry("objects/new/run.shader", shader),
        ];

        let index = build_dependency_candidate_index(&entries, &names);

        assert_eq!(
            index
                .get(&(bitmap, "run".to_owned()))
                .cloned()
                .unwrap_or_default(),
            vec!["objects\\new\\run".to_owned()]
        );
        assert_eq!(
            index
                .get(&(shader, "run".to_owned()))
                .cloned()
                .unwrap_or_default(),
            vec!["objects\\new\\run".to_owned()]
        );
    }

    #[test]
    fn folder_reference_rewrites_point_moved_tags_at_new_folder() {
        let names = TagNameIndex::default();
        let bitmap = parse_group_tag("bitm").unwrap();
        let root = Path::new("C:/kit/tags");
        let source = root.join("objects/old");
        let destination = root.join("objects/new/old");
        let entries = vec![abs_entry(
            root,
            "objects/old/decal_road_1.bitmap.bitmap",
            bitmap,
        )];

        let rewrites =
            build_folder_reference_rewrites(root, &source, &destination, &entries, &names);

        assert_eq!(
            rewrites
                .get(&(bitmap, "objects\\old\\decal_road_1.bitmap".to_owned()))
                .cloned(),
            Some("objects\\new\\old\\decal_road_1.bitmap".to_owned())
        );
    }

    #[test]
    fn rewrite_reference_prefilter_matches_ascii_case_insensitively() {
        let shader = parse_group_tag("rmsh").unwrap();
        let mut rewrites = HashMap::new();
        rewrites.insert(
            (shader, "objects\\characters\\bugger\\bugger".to_owned()),
            "zoeph_test\\bugger\\bugger".to_owned(),
        );
        let needles = rewrite_reference_needles(&rewrites);

        assert!(bytes_contain_any_ascii_case_insensitive(
            b"xx OBJECTS\\CHARACTERS\\BUGGER\\BUGGER yy",
            &needles
        ));
        assert!(!bytes_contain_any_ascii_case_insensitive(
            b"objects\\characters\\dervish\\dervish",
            &needles
        ));
    }

    #[test]
    fn affected_move_entries_include_moved_tags_and_external_dependents() {
        let shader = parse_group_tag("rmsh").unwrap();
        let model = parse_group_tag("hlmt").unwrap();
        let old_shader = entry("objects/characters/jackal/jackal.shader", shader);
        let new_shader = entry("zoeph_test/jackal/jackal.shader", shader);
        let outside_model = entry("objects/characters/shared/shared.model", model);
        let unrelated = entry("objects/characters/brute/brute.model", model);
        let all_entries = vec![old_shader.clone(), outside_model.clone(), unrelated];
        let old_entries = vec![old_shader.clone()];
        let new_entries = vec![new_shader.clone()];
        let mut rewrites = HashMap::new();
        rewrites.insert(
            (shader, "objects\\characters\\jackal\\jackal".to_owned()),
            "zoeph_test\\jackal\\jackal".to_owned(),
        );
        let mut reverse = ReverseDependencyIndex::default();
        reverse.set_tag_dependencies(
            outside_model.key.clone(),
            vec![DependencyRef {
                group_tag: shader,
                rel_path: "objects\\characters\\jackal\\jackal".to_owned(),
            }],
        );
        reverse.set_tag_dependencies(
            old_shader.key.clone(),
            vec![DependencyRef {
                group_tag: shader,
                rel_path: "objects\\characters\\jackal\\jackal".to_owned(),
            }],
        );

        let affected = affected_move_rewrite_entries(
            &all_entries,
            &old_entries,
            &new_entries,
            &rewrites,
            Some(&reverse),
        );
        let affected_keys = affected
            .into_iter()
            .map(|entry| entry.key)
            .collect::<HashSet<_>>();

        assert_eq!(affected_keys.len(), 2);
        assert!(affected_keys.contains(&new_shader.key));
        assert!(affected_keys.contains(&outside_model.key));
    }
}
