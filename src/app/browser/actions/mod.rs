//! What the browser's rows and menus do: browser actions, copying names and
//! paths, opening folders in Explorer or another program, favourites, revealing
//! a tag, and refreshing the tree.

use super::*;
use crate::core::tag_key::same_entry_key;

impl Baboon {
    pub(in crate::app) fn favorite_kit_index(&self, root: &Path) -> Option<usize> {
        self.prefs
            .editing_kit_favorites
            .iter()
            .position(|kit| same_recent_path(&kit.tags_root, root))
    }

    /// Rebuild `kit`'s resolved favorite entries from the saved paths for its
    /// tags root. Kit-scoped because a finished background refactor refreshes
    /// the workspace it belonged to, which need not be the focused one.
    pub(in crate::app) fn refresh_favorite_entries_for(&mut self, kit: usize) {
        self.kits[kit].browser.active_favorite_entries.clear();
        self.kits[kit].browser.active_favorite_folders.clear();
        let Some(root) = self.loaded_tags_root_for(kit) else {
            return;
        };
        let Some(index) = self.favorite_kit_index(&root) else {
            return;
        };
        let names = self.kits[kit]
            .source
            .as_ref()
            .map(|source| source.names.clone())
            .unwrap_or_else(|| self.kits[kit].names.clone());
        let saved_paths = self.prefs.editing_kit_favorites[index].tags.clone();
        let saved_folders = self.prefs.editing_kit_favorites[index].folders.clone();
        let mut missing = Vec::new();
        for relative_path in saved_paths {
            let path = root.join(&relative_path);
            if !path.is_file() {
                missing.push(relative_path);
                continue;
            }
            if let Ok(Some(entry)) = loose_file_entry(&root, &path, &names) {
                self.kits[kit].browser.active_favorite_entries.push(entry);
            }
        }
        let mut missing_folders = Vec::new();
        for relative_path in saved_folders {
            if root.join(&relative_path).is_dir() {
                self.kits[kit].browser.active_favorite_folders.push(relative_path);
            } else {
                missing_folders.push(relative_path);
            }
        }
        if !missing.is_empty() || !missing_folders.is_empty() {
            let favorites = &mut self.prefs.editing_kit_favorites[index];
            favorites.tags.retain(|path| {
                !missing
                    .iter()
                    .any(|missing| same_recent_path(missing, path))
            });
            favorites.folders.retain(|path| {
                !missing_folders
                    .iter()
                    .any(|missing| same_recent_path(missing, path))
            });
            if favorites.tags.is_empty() && favorites.folders.is_empty() {
                self.prefs.editing_kit_favorites.remove(index);
            }
        }
    }

    pub(in crate::app) fn toggle_favorite(&mut self, key: &str) {
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Favorites are only available for editing-kit tag folders".to_owned();
            return;
        };
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "Tag is no longer available".to_owned();
            return;
        };
        let TagEntryLocation::LooseFile(path) = &entry.location else {
            self.status = "Only loose tags can be favorited".to_owned();
            return;
        };
        let Some(relative_path) = path
            .strip_prefix(&root)
            .ok()
            .map(Path::to_path_buf)
            .and_then(clean_favorite_relative_path)
        else {
            self.status = "Could not resolve tag relative to the loaded tags folder".to_owned();
            return;
        };
        let index = self.favorite_kit_index(&root).unwrap_or_else(|| {
            self.prefs.editing_kit_favorites.push(EditingKitFavorites {
                tags_root: clean_recent_path(root.clone()),
                tags: Vec::new(),
                folders: Vec::new(),
            });
            self.prefs.editing_kit_favorites.len() - 1
        });
        let kit = &mut self.prefs.editing_kit_favorites[index];
        if let Some(position) = kit
            .tags
            .iter()
            .position(|current| same_recent_path(current, &relative_path))
        {
            kit.tags.remove(position);
            self.kits[self.active]
                .browser.active_favorite_entries
                .retain(|favorite| favorite.key != entry.key);
            if kit.tags.is_empty() && kit.folders.is_empty() {
                self.prefs.editing_kit_favorites.remove(index);
            }
            self.status = format!("Removed {} from Favorites", entry.display_path);
        } else {
            kit.tags.push(relative_path);
            self.kits[self.active]
                .browser.active_favorite_entries
                .push(entry.clone());
            self.status = format!("Added {} to Favorites", entry.display_path);
        }
    }

    pub(in crate::app) fn toggle_folder_favorite(&mut self, rel_path: &Path) {
        let Some(root) = self.loaded_tags_root() else {
            self.status = "Only loose editing-kit folders can be favorited".to_owned();
            return;
        };
        let Some(relative_path) = clean_favorite_relative_path(rel_path.to_path_buf()) else {
            self.status = "Could not resolve the folder inside the loaded tags folder".to_owned();
            return;
        };
        if !root.join(&relative_path).is_dir() {
            self.status = format!("Folder no longer exists: {}", relative_path.display());
            return;
        }
        let index = self.favorite_kit_index(&root).unwrap_or_else(|| {
            self.prefs.editing_kit_favorites.push(EditingKitFavorites {
                tags_root: clean_recent_path(root.clone()),
                tags: Vec::new(),
                folders: Vec::new(),
            });
            self.prefs.editing_kit_favorites.len() - 1
        });
        let favorites = &mut self.prefs.editing_kit_favorites[index];
        if let Some(position) = favorites
            .folders
            .iter()
            .position(|current| same_recent_path(current, &relative_path))
        {
            favorites.folders.remove(position);
            self.kits[self.active]
                .browser.active_favorite_folders
                .retain(|current| !same_recent_path(current, &relative_path));
            if favorites.tags.is_empty() && favorites.folders.is_empty() {
                self.prefs.editing_kit_favorites.remove(index);
            }
            self.status = format!("Removed {} from Favorites", relative_path.display());
        } else {
            favorites.folders.push(relative_path.clone());
            self.kits[self.active]
                .browser.active_favorite_folders
                .push(relative_path.clone());
            self.status = format!("Added {} to Favorites", relative_path.display());
        }
    }

    /// Rewrite `kit`'s favorites after a move or rename changed its tag paths.
    ///
    /// Takes the kit rather than reading the active one: this runs from a
    /// finished background refactor, which may well land while the user is in
    /// another workspace — and then it resolved the wrong root and remapped the
    /// wrong workspace's favorites with this one's rename map.
    pub(in crate::app) fn remap_favorites_for_kit(
        &mut self,
        kit: usize,
        old_to_new_keys: &HashMap<String, String>,
        moved_folder: Option<(&Path, &Path)>,
    ) {
        let Some(root) = self.loaded_tags_root_for(kit) else {
            return;
        };
        let Some(index) = self.favorite_kit_index(&root) else {
            return;
        };
        remap_favorite_paths(
            &root,
            &mut self.prefs.editing_kit_favorites[index].tags,
            old_to_new_keys,
        );
        if let Some((from, to)) = moved_folder {
            remap_favorite_folders(&mut self.prefs.editing_kit_favorites[index].folders, from, to);
            let mut unique_folders: Vec<PathBuf> = Vec::new();
            self.prefs.editing_kit_favorites[index].folders.retain(|path| {
                if unique_folders
                    .iter()
                    .any(|existing| same_recent_path(existing, path))
                {
                    false
                } else {
                    unique_folders.push(path.clone());
                    true
                }
            });
        }
        let mut unique: Vec<PathBuf> = Vec::new();
        self.prefs.editing_kit_favorites[index].tags.retain(|path| {
            if unique
                .iter()
                .any(|existing| same_recent_path(existing, path))
            {
                false
            } else {
                unique.push(path.clone());
                true
            }
        });
        self.refresh_favorite_entries_for(kit);
    }

    pub(in crate::app) fn refresh_tag_browser(&mut self, ctx: egui::Context) {
        let reset_result = self.source_mut().and_then(|source| {
            let TagSource::LooseFolder { root, .. } = &source.source else {
                return None;
            };
            Some(reset_lazy_folder_browser(
                root,
                &mut source.tree,
                &mut source.entries,
            ))
        });
        match reset_result {
            Some(Ok(())) => {
                self.kits[self.active].generation =
                    self.kits[self.active].generation.wrapping_add(1);
                self.status = "Tag browser refreshed; checking index...".to_owned();
                self.begin_refresh_entry_index(ctx);
            }
            Some(Err(error)) => self.status = format!("Tag browser refresh failed: {error}"),
            None => self.status = "No loose tag folder is loaded".to_owned(),
        }
    }

    pub(in crate::app) fn handle_browser_action(&mut self, action: BrowserAction, ctx: egui::Context) {
        match action {
            BrowserAction::OpenFolderBrowser {
                rel_path,
                label,
                open_in_new_tab,
            } => {
                let matching_key = if open_in_new_tab {
                    None
                } else {
                    let normalized = rel_path.to_string_lossy().replace('\\', "/");
                    let base = folder_pane_key(&rel_path);
                    let panes = &self.kits[self.active].browser.folder_browsers;
                    panes
                        .get(&base)
                        .filter(|pane| {
                            pane.rel_path
                                .to_string_lossy()
                                .replace('\\', "/")
                                .eq_ignore_ascii_case(&normalized)
                        })
                        .map(|_| base)
                        .or_else(|| {
                            let mut matches = panes
                                .iter()
                                .filter(|(_, pane)| {
                                    pane.rel_path
                                        .to_string_lossy()
                                        .replace('\\', "/")
                                        .eq_ignore_ascii_case(&normalized)
                                })
                                .map(|(key, _)| key.clone())
                                .collect::<Vec<_>>();
                            matches.sort();
                            matches.into_iter().next()
                        })
                };
                let key = matching_key.unwrap_or_else(|| {
                    let base = folder_pane_key(&rel_path);
                    if !self.kits[self.active].browser.folder_browsers.contains_key(&base) {
                        return base;
                    }
                    (2..)
                        .map(|suffix| format!("{base}#{suffix}"))
                        .find(|candidate| {
                            !self.kits[self.active]
                                .browser.folder_browsers
                                .contains_key(candidate)
                        })
                        .expect("folder pane suffix space is unbounded")
                });
                self.kits[self.active]
                    .browser.folder_browsers
                    .entry(key.clone())
                    .or_insert_with(|| FolderBrowserState {
                        rel_path,
                        label,
                        filter: String::new(),
                        focus_search: false,
                        mode: BrowserMode::Folders,
                        sort: self.prefs.browser_sort,
                        cached_generation: u64::MAX,
                        cached_source_len: usize::MAX,
                        tree: TagTree::default(),
                        group_tree: TagTree::default(),
                        group_tree_for: None,
                        filter_cache: FilterCache::default(),
                    });
                let selected = self.kits[self.active].selected_key.clone();
                self.kits[self.active].open_tag_pane(&key);
                self.kits[self.active].selected_key = selected;
            }
            BrowserAction::ToggleFolderFavorite(rel_path) => self.toggle_folder_favorite(&rel_path),
            BrowserAction::Select(key) => self.select_entry(key, ctx),
            BrowserAction::ToggleFavorite(key) => self.toggle_favorite(&key),
            BrowserAction::CopyTagName(key) => self.copy_tag_name(&key, &ctx),
            BrowserAction::CopyFolderPath(path) => self.copy_folder_path(&path, &ctx),
            BrowserAction::DumpJson(key) => self.begin_export_json(key, ctx),
            BrowserAction::OpenInExplorer(key) => self.open_entry_in_explorer(&key),
            BrowserAction::DumpLoadedFolderJson(keys) => {
                self.begin_export_loaded_folder_json(keys, ctx)
            }
            BrowserAction::DumpLooseFolderJson { rel_path, label } => {
                self.begin_export_loose_folder_json(rel_path, label, ctx)
            }
            BrowserAction::RenameLooseFolder { rel_path, label } => {
                self.open_loose_folder_rename(rel_path, label)
            }
            BrowserAction::MoveLooseFolder { rel_path, label } => {
                self.begin_refactor_loose_folder(rel_path, label, true)
            }
            BrowserAction::CopyLooseFolder { rel_path, label } => {
                self.begin_refactor_loose_folder(rel_path, label, false)
            }
            BrowserAction::ImportTagsIntoLooseFolder { rel_path } => {
                self.open_tag_import_dialog(Some(rel_path.to_string_lossy().into_owned()))
            }
            BrowserAction::OpenLooseFolderInExplorer { rel_path } => {
                self.open_loose_folder_in_explorer(&rel_path)
            }
            BrowserAction::ImportCacheFolderIntoKit { prefix } => {
                self.open_cache_import_dialog(prefix)
            }
            BrowserAction::ImportCacheTagIntoKit { key } => {
                self.open_cache_import_dialog_for_tag(key)
            }
            BrowserAction::ExtractRaw(key) => self.begin_extract_raw(key, ctx),
            BrowserAction::ExtractBitmap(key) => self.begin_extract_bitmap(key, ctx),
            BrowserAction::ExtractBitmapFolder(keys) => self.begin_extract_bitmap_folder(keys, ctx),
            BrowserAction::ExtractBitmapSource(key) => {
                self.begin_extract_bitmap_sources(vec![key], false, ctx)
            }
            BrowserAction::ExtractBitmapSourceFolder(keys) => {
                self.begin_extract_bitmap_sources(keys, true, ctx)
            }
            BrowserAction::ExtractSound {
                keys,
                all_languages,
            } => self.begin_extract_sounds(keys, all_languages),
            BrowserAction::LoadFolderExtractables { rel_path, label } => {
                self.begin_load_folder_extractables(rel_path, label, ctx)
            }
            BrowserAction::ExtractGeometry(key) => {
                self.prompt_extract_target(key, ExtractKind::Geometry)
            }
            BrowserAction::ExtractImportInfo(key) => self.begin_extract_import_info(key, ctx),
            BrowserAction::ExtractAnimation(key) => {
                self.prompt_extract_target(key, ExtractKind::Animation)
            }
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
            BrowserAction::ReimportGeometry(key) => self.begin_reimport_geometry(&key),
            BrowserAction::ExtractContainerFolderTags { label, keys } => {
                self.begin_extract_container_folder_tags(label, keys)
            }
            BrowserAction::ExtractScenarioScripts(key) => {
                self.begin_extract_scenario_scripts(key, ctx)
            }
            BrowserAction::ImportScenarioScripts(key) => self.import_scenario_scripts(&key),
            BrowserAction::RenameTag(key) => self.open_rename_tag(&key),
            BrowserAction::DuplicateTag(key) => self.open_duplicate_tag(&key),
            BrowserAction::DeleteTag(key) => self.open_delete_tag(&key),
            BrowserAction::FindReferences(key) => self.show_references_for(&key),
            BrowserAction::ExploreReferences(key) => self.open_content_explorer(&key),
            BrowserAction::DumpReferences(key) => self.begin_dump_tag_references(&key, ctx),
            BrowserAction::LaunchScenarioInSapien(key) => self.launch_scenario_in_sapien(&key),
            BrowserAction::LaunchScenarioInTagTest(key) => self.launch_scenario_in_tag_test(&key),
            BrowserAction::MoveTag(key) => self.begin_move_tag(&key),
            BrowserAction::ImportTagInFolder { folder_rel } => self.begin_import_tag(folder_rel),
            BrowserAction::NewTagInFolder { folder_rel } => {
                self.open_new_tag_dialog_in_folder(folder_rel)
            }
            BrowserAction::NewContainerFolder { parent_rel } => {
                self.open_new_container_folder(parent_rel)
            }
            BrowserAction::RenameContainerFolder { rel } => self.open_rename_container_folder(rel),
            BrowserAction::DeleteContainerFolder { rel } => self.delete_container_folder(rel),
        }
    }

    pub(in crate::app) fn copy_tag_name(&mut self, key: &str, ctx: &egui::Context) {
        let Some(entry) = self.entry_for_key(key) else {
            self.status = "Tag is no longer in the browser".to_owned();
            return;
        };
        let copied_path = crate::core::format::to_native_path_string(&entry.display_path);
        ctx.copy_text(copied_path.clone());
        self.status = format!("Copied {copied_path}");
    }

    pub(in crate::app) fn copy_folder_path(&mut self, path: &Path, ctx: &egui::Context) {
        let copied_path = crate::core::format::to_native_path_string(&path.to_string_lossy());
        ctx.copy_text(copied_path.clone());
        self.status = format!("Copied {copied_path}");
    }

    pub(in crate::app) fn open_entry_in_explorer(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "Tag is no longer in the browser".to_owned();
            return;
        };
        let Some(source) = self.source().map(|source| &source.source) else {
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
            (TagEntryLocation::Container { .. }, _) => {
                self.status = "Container tag has no loose file to show".to_owned();
                return;
            }
            (TagEntryLocation::NewContainer { .. }, _) => {
                self.status = "New tag has not been saved yet".to_owned();
                return;
            }
        };
        #[cfg(windows)]
        {
            if !path.is_file() {
                self.status = format!(
                    "Could not open File Explorer: file no longer exists at {}",
                    path.display()
                );
                return;
            }
            match Command::new("explorer.exe")
                .args(explorer_select_args(&path))
                .spawn()
            {
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

    pub(in crate::app) fn open_loaded_tags_folder(&mut self) {
        let Some(path) = self.loaded_tags_root() else {
            self.status = "Open Tags Folder requires a loaded editing-kit tags folder".to_owned();
            return;
        };
        self.open_folder_in_explorer(path, "tags");
    }

    pub(in crate::app) fn open_loaded_data_folder(&mut self) {
        let Some(path) = self.loaded_data_root() else {
            self.status = "Open Data Folder requires a loaded editing-kit tags folder".to_owned();
            return;
        };
        self.open_folder_in_explorer(path, "data");
    }

    pub(in crate::app) fn loaded_data_root(&self) -> Option<PathBuf> {
        Some(self.kit_layout_for(self.active)?.data)
    }

    /// Show a browser folder in File Explorer.
    ///
    /// `rel_path` is the browser's own path for the node, which for a loose kit
    /// is the directory's path under the tags root — so the only work is joining
    /// the two. A kit that is not a loose folder has no directory to open, and
    /// says so rather than opening the wrong thing.
    pub(in crate::app) fn open_loose_folder_in_explorer(&mut self, rel_path: &Path) {
        let Some(root) = self.loaded_tags_root() else {
            self.status = "This workspace has no tags folder on disk".to_owned();
            return;
        };
        let path = loose_folder_explorer_path(&root, rel_path);
        self.open_folder_in_explorer(path, "Tag");
    }

    /// Show `path` in the system's file manager: File Explorer, Finder, or
    /// whatever `xdg-open` picks.
    ///
    /// Only Windows used to do anything here; everywhere else the user was
    /// told the action was Windows-only, though the git review panel already
    /// opened folders on macOS and Linux its own way.
    pub(in crate::app) fn open_folder_in_explorer(&mut self, path: PathBuf, label: &str) {
        self.open_folder_with(path, label, |mut command| command.spawn().map(drop));
    }

    /// [`Self::open_folder_in_explorer`] with the launch passed in, so the
    /// command can be checked without opening a window.
    pub(in crate::app) fn open_folder_with(
        &mut self,
        path: PathBuf,
        label: &str,
        spawn: impl FnOnce(Command) -> std::io::Result<()>,
    ) {
        if !path.is_dir() {
            self.status = format!("{label} folder not found: {}", path.display());
            return;
        }
        self.status = match spawn(folder_opener(&path)) {
            Ok(()) => format!("Opened {} folder: {}", label, path.display()),
            Err(error) => format!("Could not open the {label} folder: {error}"),
        };
    }

    /// Locate a tag in the browser tree: switch to Folders mode, clear the
    /// filter, select it, and request a one-shot force-open + scroll.
    pub(in crate::app) fn reveal_in_browser(&mut self, key: &str) {
        let Some(entry) = self.entry_for_key(key).cloned() else {
            return;
        };
        self.kits[self.active].browser.filter.clear();
        self.kits[self.active].browser.mode = BrowserMode::Folders;
        self.kits[self.active].selected_key = Some(entry.key.clone());
        self.browser.reveal_target = Some(RevealRequest {
            kit: self.active_kit_id(),
            key: entry.key.clone(),
            ancestors: browser::ancestor_labels(&entry.display_path),
        });
    }
}

#[cfg(any(windows, test))]
pub(in crate::app) fn explorer_select_args(path: &Path) -> [std::ffi::OsString; 2] {
    [
        std::ffi::OsString::from("/select,"),
        path.as_os_str().to_owned(),
    ]
}

pub(in crate::app) fn loose_folder_explorer_path(tags_root: &Path, requested: &Path) -> PathBuf {
    if requested.is_absolute() || looks_like_absolute_windows_path(requested) {
        requested.to_path_buf()
    } else {
        tags_root.join(requested)
    }
}

/// `Path::is_absolute` follows the host platform, but favorite-folder actions
/// can carry an Explorer path while this pure helper is exercised by Unix CI.
/// Recognize the Windows forms explicitly so an already-rooted favorite is
/// never appended to whichever kit happens to be active.
pub(in crate::app) fn looks_like_absolute_windows_path(path: &Path) -> bool {
    let text = path.as_os_str().to_string_lossy();
    let bytes = text.as_bytes();
    let drive_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let network_or_device_absolute =
        bytes.len() >= 2 && matches!(bytes[0], b'\\' | b'/') && matches!(bytes[1], b'\\' | b'/');
    drive_absolute || network_or_device_absolute
}

/// The command that opens `folder` in the platform's file manager.
pub(in crate::app) fn folder_opener(folder: &Path) -> Command {
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(any(windows, target_os = "macos")))]
    let program = "xdg-open";
    let mut command = Command::new(program);
    command.arg(folder);
    command
}

pub(in crate::app) fn reset_lazy_folder_browser(
    root: &Path,
    tree: &mut TagTree,
    entries: &mut Vec<TagEntry>,
) -> Result<(), String> {
    *tree = crate::core::source::build_folder_directory_tree(root).map_err(|error| error.to_string())?;
    entries.clear();
    Ok(())
}

/// Favorite folders at or under `from` follow it to `to` (both relative to the
/// tags root). Folders are compared component by component ignoring case, the
/// way tag paths are, and keep the case of the part below the moved folder.
pub(in crate::app) fn remap_favorite_folders(folders: &mut [PathBuf], from: &Path, to: &Path) {
    let from: Vec<String> = from
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    for folder in folders {
        let parts: Vec<_> = folder.components().collect();
        if parts.len() < from.len()
            || !parts
                .iter()
                .zip(&from)
                .all(|(part, wanted)| part.as_os_str().to_string_lossy().to_ascii_lowercase() == *wanted)
        {
            continue;
        }
        let mut moved = to.to_path_buf();
        for part in &parts[from.len()..] {
            moved.push(part);
        }
        *folder = moved;
    }
}

pub(in crate::app) fn remap_favorite_paths(
    root: &Path,
    relative_paths: &mut [PathBuf],
    old_to_new_keys: &HashMap<String, String>,
) {
    for relative_path in relative_paths {
        let old_key = file_entry_key(&root.join(&*relative_path));
        let Some(new_key) = old_to_new_keys
            .iter()
            .find_map(|(old, new)| same_entry_key(old, &old_key).then_some(new))
        else {
            continue;
        };
        let Some(new_path) = file_key_path(new_key).map(Path::to_path_buf) else {
            continue;
        };
        if let Some(new_relative) = new_path
            .strip_prefix(root)
            .ok()
            .map(Path::to_path_buf)
            .and_then(clean_favorite_relative_path)
        {
            *relative_path = new_relative;
        }
    }
}

#[cfg(test)]
mod browser_action_table_tests;

#[cfg(test)]
mod browser_refresh_tests;

#[cfg(test)]
mod favorite_folder_tests;

#[cfg(test)]
mod folder_opener_tests;

#[cfg(test)]
mod explorer_path_tests;
