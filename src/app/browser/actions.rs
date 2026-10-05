//! What the browser's rows and menus do: browser actions, copying names and
//! paths, opening folders in Explorer or another program, favourites, revealing
//! a tag, and refreshing the tree.

use super::*;
use crate::core::tag_key::same_entry_key;

impl Baboon {

    /// Rebuild `kit`'s resolved favorite entries from the saved paths for its
    /// tags root. Kit-scoped because a finished background refactor refreshes
    /// the workspace it belonged to, which need not be the focused one.
    pub(in crate::app) fn refresh_favorite_entries_for(&mut self, kit: usize) {
        self.model.kits[kit].active_favorite_entries.clear();
        self.model.kits[kit].active_favorite_folders.clear();
        let Some(root) = self.model.loaded_tags_root_for(kit) else {
            return;
        };
        let Some(index) = self.model.favorite_kit_index(&root) else {
            return;
        };
        let names = self.model.kits[kit]
            .source
            .as_ref()
            .map(|source| source.names.clone())
            .unwrap_or_else(|| self.model.kits[kit].names.clone());
        let saved_paths = self.model.prefs.editing_kit_favorites[index].tags.clone();
        let saved_folders = self.model.prefs.editing_kit_favorites[index].folders.clone();
        let mut missing = Vec::new();
        for relative_path in saved_paths {
            let path = root.join(&relative_path);
            if !path.is_file() {
                missing.push(relative_path);
                continue;
            }
            if let Ok(Some(entry)) = loose_file_entry(&root, &path, &names) {
                self.model.kits[kit].active_favorite_entries.push(entry);
            }
        }
        let mut missing_folders = Vec::new();
        for relative_path in saved_folders {
            if root.join(&relative_path).is_dir() {
                self.model.kits[kit].active_favorite_folders.push(relative_path);
            } else {
                missing_folders.push(relative_path);
            }
        }
        if !missing.is_empty() || !missing_folders.is_empty() {
            let favorites = &mut self.model.prefs.editing_kit_favorites[index];
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
                self.model.prefs.editing_kit_favorites.remove(index);
            }
        }
    }

    pub(in crate::app) fn toggle_favorite(&mut self, key: &str) {
        let Some(root) = self.model.loaded_tags_root() else {
            self.model.status = "Favorites are only available for editing-kit tag folders".to_owned();
            return;
        };
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            self.model.status = "Tag is no longer available".to_owned();
            return;
        };
        let TagEntryLocation::LooseFile(path) = &entry.location else {
            self.model.status = "Only loose tags can be favorited".to_owned();
            return;
        };
        let Some(relative_path) = path
            .strip_prefix(&root)
            .ok()
            .map(Path::to_path_buf)
            .and_then(clean_favorite_relative_path)
        else {
            self.model.status = "Could not resolve tag relative to the loaded tags folder".to_owned();
            return;
        };
        let index = self.model.favorite_kit_index(&root).unwrap_or_else(|| {
            self.model.prefs.editing_kit_favorites.push(EditingKitFavorites {
                tags_root: clean_recent_path(root.clone()),
                tags: Vec::new(),
                folders: Vec::new(),
            });
            self.model.prefs.editing_kit_favorites.len() - 1
        });
        let kit = &mut self.model.prefs.editing_kit_favorites[index];
        if let Some(position) = kit
            .tags
            .iter()
            .position(|current| same_recent_path(current, &relative_path))
        {
            kit.tags.remove(position);
            self.model.kits[self.model.active].active_favorite_entries
                .retain(|favorite| favorite.key != entry.key);
            if kit.tags.is_empty() && kit.folders.is_empty() {
                self.model.prefs.editing_kit_favorites.remove(index);
            }
            self.model.status = format!("Removed {} from Favorites", entry.display_path);
        } else {
            kit.tags.push(relative_path);
            self.model.kits[self.model.active].active_favorite_entries
                .push(entry.clone());
            self.model.status = format!("Added {} to Favorites", entry.display_path);
        }
    }

    pub(in crate::app) fn toggle_folder_favorite(&mut self, rel_path: &Path) {
        let Some(root) = self.model.loaded_tags_root() else {
            self.model.status = "Only loose editing-kit folders can be favorited".to_owned();
            return;
        };
        let Some(relative_path) = clean_favorite_relative_path(rel_path.to_path_buf()) else {
            self.model.status = "Could not resolve the folder inside the loaded tags folder".to_owned();
            return;
        };
        if !root.join(&relative_path).is_dir() {
            self.model.status = format!("Folder no longer exists: {}", relative_path.display());
            return;
        }
        let index = self.model.favorite_kit_index(&root).unwrap_or_else(|| {
            self.model.prefs.editing_kit_favorites.push(EditingKitFavorites {
                tags_root: clean_recent_path(root.clone()),
                tags: Vec::new(),
                folders: Vec::new(),
            });
            self.model.prefs.editing_kit_favorites.len() - 1
        });
        let favorites = &mut self.model.prefs.editing_kit_favorites[index];
        if let Some(position) = favorites
            .folders
            .iter()
            .position(|current| same_recent_path(current, &relative_path))
        {
            favorites.folders.remove(position);
            self.model.kits[self.model.active].active_favorite_folders
                .retain(|current| !same_recent_path(current, &relative_path));
            if favorites.tags.is_empty() && favorites.folders.is_empty() {
                self.model.prefs.editing_kit_favorites.remove(index);
            }
            self.model.status = format!("Removed {} from Favorites", relative_path.display());
        } else {
            favorites.folders.push(relative_path.clone());
            self.model.kits[self.model.active].active_favorite_folders
                .push(relative_path.clone());
            self.model.status = format!("Added {} to Favorites", relative_path.display());
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
        let Some(root) = self.model.loaded_tags_root_for(kit) else {
            return;
        };
        let Some(index) = self.model.favorite_kit_index(&root) else {
            return;
        };
        remap_favorite_paths(
            &root,
            &mut self.model.prefs.editing_kit_favorites[index].tags,
            old_to_new_keys,
        );
        if let Some((from, to)) = moved_folder {
            remap_favorite_folders(&mut self.model.prefs.editing_kit_favorites[index].folders, from, to);
            let mut unique_folders: Vec<PathBuf> = Vec::new();
            self.model.prefs.editing_kit_favorites[index].folders.retain(|path| {
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
        self.model.prefs.editing_kit_favorites[index].tags.retain(|path| {
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
                self.model.kits[self.model.active].generation =
                    self.model.kits[self.model.active].generation.wrapping_add(1);
                self.model.status = "Tag browser refreshed; checking index...".to_owned();
                self.begin_refresh_entry_index(ctx);
            }
            Some(Err(error)) => self.model.status = format!("Tag browser refresh failed: {error}"),
            None => self.model.status = "No loose tag folder is loaded".to_owned(),
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
                    let panes = &self.views[self.model.kits[self.model.active].id].browser.folder_browsers;
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
                    if !self.views[self.model.kits[self.model.active].id].browser.folder_browsers.contains_key(&base) {
                        return base;
                    }
                    (2..)
                        .map(|suffix| format!("{base}#{suffix}"))
                        .find(|candidate| {
                            !self.views[self.model.kits[self.model.active].id]
                                .browser.folder_browsers
                                .contains_key(candidate)
                        })
                        .expect("folder pane suffix space is unbounded")
                });
                self.views[self.model.kits[self.model.active].id]
                    .browser.folder_browsers
                    .entry(key.clone())
                    .or_insert_with(|| FolderBrowserState {
                        rel_path,
                        label,
                        filter: String::new(),
                        focus_search: false,
                        mode: BrowserMode::Folders,
                        sort: self.model.prefs.browser_sort,
                        cached_generation: u64::MAX,
                        cached_source_len: usize::MAX,
                        tree: TagTree::default(),
                        group_tree: TagTree::default(),
                        group_tree_for: None,
                        filter_cache: FilterCache::default(),
                        date_cache: FolderDateCache::default(),
                        table_layout: FolderTableLayout::default(),
                        search_scope: self.model.prefs.browser_search_scope,
                        assets_view: false,
                        asset_bitmaps: true,
                        asset_models: true,
                        asset_cell_size: DEFAULT_CELL,
                    });
                let selected = self.model.kits[self.model.active].selected_key.clone();
                self.kit_and_view(self.model.active).open_tag_pane(&key);
                self.model.kits[self.model.active].selected_key = selected;
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
        let Some(entry) = self.model.entry_for_key(key) else {
            self.model.status = "Tag is no longer in the browser".to_owned();
            return;
        };
        let copied_path = crate::core::format::to_native_path_string(&entry.display_path);
        ctx.copy_text(copied_path.clone());
        self.model.status = format!("Copied {copied_path}");
    }

    pub(in crate::app) fn copy_folder_path(&mut self, path: &Path, ctx: &egui::Context) {
        let copied_path = crate::core::format::to_native_path_string(&path.to_string_lossy());
        ctx.copy_text(copied_path.clone());
        self.model.status = format!("Copied {copied_path}");
    }

    pub(in crate::app) fn open_entry_in_explorer(&mut self, key: &str) {
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            self.model.status = "Tag is no longer in the browser".to_owned();
            return;
        };
        let Some(source) = self.model.source().map(|source| &source.source) else {
            self.model.status = "No source loaded".to_owned();
            return;
        };
        let path = match (&entry.location, source) {
            (TagEntryLocation::LooseFile(path), _) => path.clone(),
            (_, TagSource::SingleFile { path }) => path.clone(),
            (TagEntryLocation::Monolithic { .. }, TagSource::MonolithicCache { root, .. }) => {
                root.join("blob_index.dat")
            }
            (TagEntryLocation::Monolithic { .. }, _) => {
                self.model.status = "Monolithic tag has no loose file to show".to_owned();
                return;
            }
            (TagEntryLocation::Container { .. }, _) => {
                self.model.status = "Container tag has no loose file to show".to_owned();
                return;
            }
            (TagEntryLocation::NewContainer { .. }, _) => {
                self.model.status = "New tag has not been saved yet".to_owned();
                return;
            }
        };
        #[cfg(windows)]
        {
            if !path.is_file() {
                self.model.status = format!(
                    "Could not open File Explorer: file no longer exists at {}",
                    path.display()
                );
                return;
            }
            match Command::new("explorer.exe")
                .args(explorer_select_args(&path))
                .spawn()
            {
                Ok(_) => self.model.status = format!("Opened {}", path.display()),
                Err(error) => self.model.status = format!("Could not open File Explorer: {error}"),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            self.model.status = "Open with File Explorer is only available on Windows".to_owned();
        }
    }

    pub(in crate::app) fn open_loaded_tags_folder(&mut self) {
        let Some(path) = self.model.loaded_tags_root() else {
            self.model.status = "Open Tags Folder requires a loaded editing-kit tags folder".to_owned();
            return;
        };
        self.open_folder_in_explorer(path, "tags");
    }

    pub(in crate::app) fn open_loaded_data_folder(&mut self) {
        let Some(path) = self.model.loaded_data_root() else {
            self.model.status = "Open Data Folder requires a loaded editing-kit tags folder".to_owned();
            return;
        };
        self.open_folder_in_explorer(path, "data");
    }

    /// Show a browser folder in File Explorer.
    ///
    /// `rel_path` is the browser's own path for the node, which for a loose kit
    /// is the directory's path under the tags root — so the only work is joining
    /// the two. A kit that is not a loose folder has no directory to open, and
    /// says so rather than opening the wrong thing.
    pub(in crate::app) fn open_loose_folder_in_explorer(&mut self, rel_path: &Path) {
        let Some(root) = self.model.loaded_tags_root() else {
            self.model.status = "This workspace has no tags folder on disk".to_owned();
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
            self.model.status = format!("{label} folder not found: {}", path.display());
            return;
        }
        self.model.status = match spawn(folder_opener(&path)) {
            Ok(()) => format!("Opened {} folder: {}", label, path.display()),
            Err(error) => format!("Could not open the {label} folder: {error}"),
        };
    }

    /// Locate a tag in the browser tree: switch to Folders mode, clear the
    /// filter, select it, and request a one-shot force-open + scroll.
    pub(in crate::app) fn reveal_in_browser(&mut self, key: &str) {
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            return;
        };
        self.views[self.model.kits[self.model.active].id].browser.filter.clear();
        self.views[self.model.kits[self.model.active].id].browser.mode = BrowserMode::Folders;
        self.model.kits[self.model.active].selected_key = Some(entry.key.clone());
        self.browser.reveal_target = Some(RevealRequest {
            kit: self.model.active_kit_id(),
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

impl Model {
    pub(in crate::app) fn favorite_kit_index(&self, root: &Path) -> Option<usize> {
        self.prefs
            .editing_kit_favorites
            .iter()
            .position(|kit| same_recent_path(&kit.tags_root, root))
    }

    pub(in crate::app) fn loaded_data_root(&self) -> Option<PathBuf> {
        Some(self.kit_layout_for(self.active)?.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::import::{CacheImportDialog, TagImportDialog};
    use crate::app::loose_fixture::*;
    use crate::app::search::QueryResultsWindow;
    use crate::app::shell::FolderRefactorUiState;
    use crate::app::tag_ops::{DeleteConfirm, DeleteKind, NewTagDialog};
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use super::remap_favorite_folders;

    // Every `BrowserAction` dispatched through `handle_browser_action` on a
    // synthetic loose Halo 3 kit, one case per variant, each asserting the state
    // its arm leaves behind.
    //
    // Many arms end in a native file or folder dialog, which a test cannot
    // answer. For those the case pins everything before the dialog: the guard
    // that refuses, with its message, or -- where the guard returns silently --
    // that nothing at all changed. What runs after a dialog is covered by the
    // tests of the jobs themselves.
    //
    // `variant_index` matches every variant without a wildcard, so adding one
    // fails to compile here until it is given an index, and the table test then
    // fails until it is given a case.

    const MODEL: &str = "objects/props/crate.model";
    const RENDER: &str = "objects/props/crate.render_model";
    const BARREL: &str = "objects/props/barrel.model";
    const FOLDER: &str = "objects/props";
    const UNKNOWN: &str = "file:/no/such/tag.model";

    /// How many variants `BrowserAction` has. Bump it with `variant_index`.
    const VARIANTS: usize = 49;

    fn variant_index(action: &BrowserAction) -> usize {
        use BrowserAction as A;
        match action {
            A::OpenFolderBrowser { .. } => 0,
            A::ToggleFolderFavorite(_) => 1,
            A::Select(_) => 2,
            A::ToggleFavorite(_) => 3,
            A::CopyTagName(_) => 4,
            A::CopyFolderPath(_) => 5,
            A::DumpJson(_) => 6,
            A::OpenInExplorer(_) => 7,
            A::DumpLoadedFolderJson(_) => 8,
            A::DumpLooseFolderJson { .. } => 9,
            A::RenameLooseFolder { .. } => 10,
            A::MoveLooseFolder { .. } => 11,
            A::CopyLooseFolder { .. } => 12,
            A::ImportTagsIntoLooseFolder { .. } => 13,
            A::OpenLooseFolderInExplorer { .. } => 14,
            A::ImportCacheFolderIntoKit { .. } => 15,
            A::ImportCacheTagIntoKit { .. } => 16,
            A::ExtractRaw(_) => 17,
            A::ExtractBitmap(_) => 18,
            A::ExtractBitmapFolder(_) => 19,
            A::ExtractBitmapSource(_) => 20,
            A::ExtractBitmapSourceFolder(_) => 21,
            A::ExtractSound { .. } => 22,
            A::LoadFolderExtractables { .. } => 23,
            A::ExtractGeometry(_) => 24,
            A::ExtractImportInfo(_) => 25,
            A::ExtractAnimation(_) => 26,
            A::ExtractMaterialShaderSources(_) => 27,
            A::ExtractMaterialShaderSourceFolder(_) => 28,
            A::ExtractHlslIncludeSource(_) => 29,
            A::ExtractHlslIncludeFolder(_) => 30,
            A::ReimportGeometry(_) => 31,
            A::ExtractContainerFolderTags { .. } => 32,
            A::ExtractScenarioScripts(_) => 33,
            A::ImportScenarioScripts(_) => 34,
            A::FindReferences(_) => 35,
            A::ExploreReferences(_) => 36,
            A::DumpReferences(_) => 37,
            A::LaunchScenarioInSapien(_) => 38,
            A::LaunchScenarioInTagTest(_) => 39,
            A::RenameTag(_) => 40,
            A::DuplicateTag(_) => 41,
            A::DeleteTag(_) => 42,
            A::MoveTag(_) => 43,
            A::ImportTagInFolder { .. } => 44,
            A::NewTagInFolder { .. } => 45,
            A::NewContainerFolder { .. } => 46,
            A::RenameContainerFolder { .. } => 47,
            A::DeleteContainerFolder { .. } => 48,
        }
    }

    /// An H3 kit: a model referencing its render model, and a second model.
    fn fixture() -> LooseKit {
        let kit = LooseKit::new("browser-actions", "halo3_mcc");
        let mode = group_tag("halo3_mcc", "render_model");
        kit.write_mcc("objects/props/crate", "render_model", |_| {});
        kit.write_mcc("objects/props/crate", "model", |tag| {
            set_reference(tag, "render model", mode, "objects\\props\\crate");
        });
        kit.write_mcc("objects/props/barrel", "model", |_| {});
        kit
    }

    /// What one dispatch left behind beyond the app itself.
    struct Outcome {
        copied_text: String,
        /// Whether a worker answered within a moment of the dispatch.
        worker_answered: bool,
    }

    type Setup = fn(&mut Baboon, &LooseKit);
    type Check = fn(&Baboon, &LooseKit, &Outcome) -> Result<(), String>;

    struct Case {
        action: fn(&LooseKit) -> BrowserAction,
        setup: Setup,
        check: Check,
    }

    fn no_setup(_: &mut Baboon, _: &LooseKit) {}

    /// Leave the model open with an unsaved edit.
    fn dirty_model(app: &mut Baboon, kit: &LooseKit) {
        let key = kit.open(app, MODEL);
        edit_field(app, &key, "disappear distance", "3");
    }

    fn ensure(condition: bool, what: impl Into<String>) -> Result<(), String> {
        condition.then_some(()).ok_or_else(|| what.into())
    }

    fn status_is(app: &Baboon, expected: &str) -> Result<(), String> {
        ensure(
            app.model.status == expected,
            format!("status {:?}, expected {expected:?}", app.model.status),
        )
    }

    /// Nothing visible happened: the status is untouched, no worker was
    /// started, and no dialog or prompt opened. For arms whose guard returns
    /// silently ahead of a native dialog.
    fn nothing_happened(app: &Baboon, _: &LooseKit, outcome: &Outcome) -> Result<(), String> {
        status_is(app, "Ready")?;
        ensure(!outcome.worker_answered, "a worker was started")?;
        ensure(outcome.copied_text.is_empty(), "something was copied")?;
        ensure(
            app.dialogs.get::<RenameTagState>().is_none()
                && app.dialogs.get::<DeleteConfirm>().is_none()
                && app.dialogs.get::<ExtractTargetPrompt>().is_none()
                && app.dialogs.get::<QueryResultsWindow>().is_none()
                && app.dialogs.get::<ContentExplorer>().is_none()
                && app.tag_ops.folder_refactor.is_none()
                && app.dialogs.get::<LooseFolderRenameState>().is_none()
                && app.dialogs.get::<TagImportDialog>().is_none()
                && app.dialogs.get::<CacheImportDialog>().is_none()
                && app.dialogs.get::<ContainerFolderDialog>().is_none()
                && app.kit_tools.pending_tool_import.is_none()
                && app.export.pending_sound_extract.is_none(),
            "a dialog opened",
        )
    }

    fn cases() -> Vec<Case> {
        use BrowserAction as A;
        vec![
            // 0
            Case {
                action: |_| A::OpenFolderBrowser {
                    rel_path: PathBuf::from(FOLDER),
                    label: "props".to_owned(),
                    open_in_new_tab: false,
                },
                setup: |app, kit| {
                    kit.open(app, BARREL);
                },
                check: |app, kit, _| {
                    let pane = folder_pane_key(Path::new(FOLDER));
                    let state = app.views[app.model.kits[0].id].browser.folder_browsers.get(&pane).ok_or("no pane")?;
                    ensure(state.label == "props", "label")?;
                    ensure(state.rel_path == Path::new(FOLDER), "rel path")?;
                    ensure(app.model.kits[0].open_tabs.contains(&pane), "not a tab")?;
                    ensure(
                        app.model.kits[0].selected_key.as_deref() == Some(kit.key(BARREL).as_str()),
                        "a folder pane must not take the tag selection",
                    )
                },
            },
            // 1
            Case {
                action: |_| A::ToggleFolderFavorite(PathBuf::from(FOLDER)),
                setup: no_setup,
                check: |app, kit, _| {
                    status_is(app, "Added objects/props to Favorites")?;
                    let favorites = &app.model.prefs.editing_kit_favorites;
                    ensure(favorites.len() == 1, "one kit's favorites")?;
                    ensure(same_recent_path(&favorites[0].tags_root, &kit.root), "tags root")?;
                    ensure(favorites[0].folders == vec![PathBuf::from(FOLDER)], "folders")?;
                    ensure(
                        app.model.kits[0].active_favorite_folders == vec![PathBuf::from(FOLDER)],
                        "kit favorites",
                    )
                },
            },
            // 2
            Case {
                action: |kit| A::Select(kit.key(MODEL)),
                setup: no_setup,
                check: |app, kit, outcome| {
                    let key = kit.key(MODEL);
                    ensure(app.model.kits[0].selected_key.as_deref() == Some(key.as_str()), "selected")?;
                    ensure(app.model.kits[0].open_tabs.contains(&key), "opened as a tab")?;
                    ensure(outcome.worker_answered, "a load was started")?;
                    status_is(app, &format!("Loading {MODEL}"))
                },
            },
            // 3
            Case {
                action: |kit| A::ToggleFavorite(kit.key(MODEL)),
                setup: no_setup,
                check: |app, kit, _| {
                    status_is(app, &format!("Added {MODEL} to Favorites"))?;
                    ensure(
                        app.model.prefs.editing_kit_favorites[0].tags == vec![PathBuf::from(MODEL)],
                        "favorite tags",
                    )?;
                    ensure(
                        app.model.kits[0].active_favorite_entries.len() == 1
                            && app.model.kits[0].active_favorite_entries[0].key == kit.key(MODEL),
                        "kit favorite entries",
                    )
                },
            },
            // 4
            Case {
                action: |kit| A::CopyTagName(kit.key(MODEL)),
                setup: no_setup,
                check: |app, _, outcome| {
                    let native = crate::core::format::to_native_path_string(MODEL);
                    ensure(outcome.copied_text == native, format!("copied {:?}", outcome.copied_text))?;
                    status_is(app, &format!("Copied {native}"))
                },
            },
            // 5
            Case {
                action: |_| A::CopyFolderPath(PathBuf::from(FOLDER)),
                setup: no_setup,
                check: |app, _, outcome| {
                    let native = crate::core::format::to_native_path_string(FOLDER);
                    ensure(outcome.copied_text == native, format!("copied {:?}", outcome.copied_text))?;
                    status_is(app, &format!("Copied {native}"))
                },
            },
            // 6: a known key reaches the save dialog.
            Case {
                action: |_| A::DumpJson(UNKNOWN.to_owned()),
                setup: no_setup,
                check: nothing_happened,
            },
            // 7: elsewhere than Windows, the arm says it cannot; on Windows a
            // known key would launch Explorer, so the unknown-key guard is used.
            Case {
                action: |kit| {
                    if cfg!(windows) {
                        A::OpenInExplorer(UNKNOWN.to_owned())
                    } else {
                        A::OpenInExplorer(kit.key(MODEL))
                    }
                },
                setup: no_setup,
                check: |app, _, _| {
                    if cfg!(windows) {
                        status_is(app, "Tag is no longer in the browser")
                    } else {
                        status_is(app, "Open with File Explorer is only available on Windows")
                    }
                },
            },
            // 8
            Case {
                action: |_| A::DumpLoadedFolderJson(vec![UNKNOWN.to_owned()]),
                setup: no_setup,
                check: |app, _, _| status_is(app, "No loaded tags found in folder"),
            },
            // 9: a loose kit reaches the folder dialog; without a source the arm
            // returns before it.
            Case {
                action: |_| A::DumpLooseFolderJson {
                    rel_path: PathBuf::from(FOLDER),
                    label: "props".to_owned(),
                },
                setup: |app, _| app.model.kits[0].source = None,
                check: nothing_happened,
            },
            // 10
            Case {
                action: |_| A::RenameLooseFolder {
                    rel_path: PathBuf::from(FOLDER),
                    label: "props".to_owned(),
                },
                setup: |app, kit| {
                    app.model.kits[0].source.as_mut().unwrap().reverse_dependencies = Some(kit.index());
                },
                check: |app, _, _| {
                    let state = app
                        .dialogs
                        .get::<LooseFolderRenameState>()
                        .ok_or("no dialog")?;
                    ensure(state.kit == app.model.kits[0].id, "kit")?;
                    ensure(state.rel_path == Path::new(FOLDER), "rel path")?;
                    ensure(state.old_name == "props" && state.name_input == "props", "name")?;
                    ensure(state.parent_display == "objects", "parent")?;
                    ensure(state.tag_count == 3, format!("{} tags", state.tag_count))?;
                    ensure(
                        state.outside_referrers.as_deref() == Some(&[][..]),
                        "the only referrer is inside the folder",
                    )
                },
            },
            // 11
            Case {
                action: |_| A::MoveLooseFolder {
                    rel_path: PathBuf::from(FOLDER),
                    label: "props".to_owned(),
                },
                setup: dirty_model,
                check: |app, _, _| status_is(app, "Save or close dirty tags before moving/copying folders"),
            },
            // 12
            Case {
                action: |_| A::CopyLooseFolder {
                    rel_path: PathBuf::from(FOLDER),
                    label: "props".to_owned(),
                },
                setup: |app, _| {
                    app.tag_ops.folder_refactor = Some(FolderRefactorUiState {
                        label: "Moving".to_owned(),
                        phase: "Preparing".to_owned(),
                        progress: None,
                    })
                },
                check: |app, _, _| status_is(app, "A folder move/copy is already running"),
            },
            // 13
            Case {
                action: |_| A::ImportTagsIntoLooseFolder {
                    rel_path: PathBuf::from(FOLDER),
                },
                setup: no_setup,
                check: |app, kit, _| {
                    let dialog = app.dialogs.get::<TagImportDialog>().ok_or("no dialog")?;
                    ensure(dialog.target_game == "halo3_mcc", "target game")?;
                    ensure(dialog.target_tags_root == kit.root, "tags root")?;
                    ensure(dialog.destination_rel == FOLDER, format!("{:?}", dialog.destination_rel))?;
                    ensure(dialog.destination_base == FOLDER, "base")?;
                    ensure(!dialog.source_game.is_empty(), "a source game is offered")
                },
            },
            // 14: an existing folder would be opened in the file manager.
            Case {
                action: |_| A::OpenLooseFolderInExplorer {
                    rel_path: PathBuf::from("objects/missing"),
                },
                setup: no_setup,
                check: |app, kit, _| {
                    status_is(
                        app,
                        &format!(
                            "Tag folder not found: {}",
                            loose_folder_explorer_path(&kit.root, Path::new("objects/missing"))
                                .display()
                        ),
                    )
                },
            },
            // 15
            Case {
                action: |_| A::ImportCacheFolderIntoKit {
                    prefix: "objects".to_owned(),
                },
                setup: no_setup,
                check: |app, _, _| status_is(app, "This is not a monolithic cache workspace"),
            },
            // 16
            Case {
                action: |kit| A::ImportCacheTagIntoKit { key: kit.key(MODEL) },
                setup: no_setup,
                check: |app, _, _| status_is(app, "This is not a monolithic cache workspace"),
            },
            // 17: a known key reaches the save dialog.
            Case {
                action: |_| A::ExtractRaw(UNKNOWN.to_owned()),
                setup: no_setup,
                check: nothing_happened,
            },
            // 18: a known key reaches the folder dialog.
            Case {
                action: |_| A::ExtractBitmap(UNKNOWN.to_owned()),
                setup: no_setup,
                check: nothing_happened,
            },
            // 19
            Case {
                action: |_| A::ExtractBitmapFolder(vec![UNKNOWN.to_owned()]),
                setup: no_setup,
                check: |app, _, _| status_is(app, "No bitmap tags found in folder"),
            },
            // 20
            Case {
                action: |_| A::ExtractBitmapSource(UNKNOWN.to_owned()),
                setup: no_setup,
                check: |app, _, _| status_is(app, "No bitmap tags found"),
            },
            // 21
            Case {
                action: |_| A::ExtractBitmapSourceFolder(Vec::new()),
                setup: no_setup,
                check: |app, _, _| status_is(app, "No bitmap tags found"),
            },
            // 22
            Case {
                action: |kit| A::ExtractSound {
                    keys: vec![kit.key(MODEL)],
                    all_languages: true,
                },
                setup: no_setup,
                check: |app, _, _| {
                    status_is(app, "No loaded sound tags found")?;
                    ensure(app.export.pending_sound_extract.is_none(), "an extraction was queued")
                },
            },
            // 23: the whole scan is in memory, so the folder loads at once.
            Case {
                action: |_| A::LoadFolderExtractables {
                    rel_path: PathBuf::from(FOLDER),
                    label: "props".to_owned(),
                },
                setup: |app, _| app.model.kits[0].source.as_mut().unwrap().entries.clear(),
                check: |app, _, outcome| {
                    status_is(app, "Loaded the entire props folder for extraction")?;
                    ensure(!outcome.worker_answered, "no scan was needed")?;
                    let entries = &app.model.kits[0].source.as_ref().unwrap().entries;
                    ensure(entries.len() == 3, format!("{} entries loaded", entries.len()))
                },
            },
            // 24
            Case {
                action: |kit| A::ExtractGeometry(kit.key(MODEL)),
                setup: no_setup,
                check: |app, kit, _| {
                    let prompt = app
                        .dialogs
                        .get::<ExtractTargetPrompt>()
                        .ok_or("no prompt")?;
                    ensure(prompt.key == kit.key(MODEL), "key")?;
                    ensure(prompt.display_path == MODEL, "display path")?;
                    ensure(matches!(prompt.kind, ExtractKind::Geometry), "kind")?;
                    ensure(
                        prompt.source == blam_tags::game::Game::Halo3
                            && prompt.target == blam_tags::game::Game::Halo3,
                        "the kit's own game, preselected",
                    )
                },
            },
            // 25: a known key reaches the folder dialog.
            Case {
                action: |_| A::ExtractImportInfo(UNKNOWN.to_owned()),
                setup: no_setup,
                check: nothing_happened,
            },
            // 26
            Case {
                action: |kit| A::ExtractAnimation(kit.key(MODEL)),
                setup: no_setup,
                check: |app, _, _| {
                    let prompt = app
                        .dialogs
                        .get::<ExtractTargetPrompt>()
                        .ok_or("no prompt")?;
                    ensure(matches!(prompt.kind, ExtractKind::Animation), "kind")
                },
            },
            // 27: a known key reaches the folder dialog.
            Case {
                action: |_| A::ExtractMaterialShaderSources(UNKNOWN.to_owned()),
                setup: no_setup,
                check: nothing_happened,
            },
            // 28
            Case {
                action: |_| A::ExtractMaterialShaderSourceFolder(vec![UNKNOWN.to_owned()]),
                setup: no_setup,
                check: |app, _, _| status_is(app, "No material shaders found in folder"),
            },
            // 29: a known key reaches the folder dialog.
            Case {
                action: |_| A::ExtractHlslIncludeSource(UNKNOWN.to_owned()),
                setup: no_setup,
                check: nothing_happened,
            },
            // 30
            Case {
                action: |_| A::ExtractHlslIncludeFolder(vec![UNKNOWN.to_owned()]),
                setup: no_setup,
                check: |app, _, _| status_is(app, "No HLSL includes found in folder"),
            },
            // 31
            Case {
                action: |kit| A::ReimportGeometry(kit.key(RENDER)),
                setup: no_setup,
                check: |app, _, _| {
                    let request = app.kit_tools.pending_tool_import.as_ref().ok_or_else(|| {
                        format!("no tool import queued; status {:?}", app.model.status)
                    })?;
                    ensure(request.verb == "render", format!("verb {:?}", request.verb))?;
                    ensure(
                        request.source_dir == "objects\\props\\crate"
                            || request.source_dir == "objects/props/crate",
                        format!("source dir {:?}", request.source_dir),
                    )
                },
            },
            // 32
            Case {
                action: |kit| A::ExtractContainerFolderTags {
                    label: "props".to_owned(),
                    keys: vec![kit.key(MODEL)],
                },
                setup: no_setup,
                check: |app, _, _| status_is(app, "Extracting tags needs a Campaign Evolved container"),
            },
            // 33
            Case {
                action: |kit| A::ExtractScenarioScripts(kit.key(MODEL)),
                setup: no_setup,
                check: |app, _, _| {
                    status_is(app, "Script extraction is only available for Campaign Evolved")
                },
            },
            // 34
            Case {
                action: |kit| A::ImportScenarioScripts(kit.key(MODEL)),
                setup: no_setup,
                check: |app, _, _| status_is(app, "Script import is only available for Campaign Evolved"),
            },
            // 35
            Case {
                action: |kit| A::FindReferences(kit.key(RENDER)),
                setup: |app, kit| {
                    app.model.kits[0].source.as_mut().unwrap().reverse_dependencies = Some(kit.index());
                },
                check: |app, kit, _| {
                    let results = &app.dialogs.get::<QueryResultsWindow>().ok_or("no results")?.results;
                    ensure(results.title == format!("References to {RENDER}"), results.title.clone())?;
                    ensure(
                        results.entries.iter().map(|entry| entry.key.clone()).collect::<Vec<_>>()
                            == vec![kit.key(MODEL)],
                        "the model references it",
                    )?;
                    ensure(results.note.is_none(), "no note")?;
                    ensure(
                        results.ref_target
                            == Some((group_tag("halo3_mcc", "render_model"), "objects\\props\\crate".to_owned())),
                        format!("ref target {:?}", results.ref_target),
                    )
                },
            },
            // 36: without an index, both directions are unavailable.
            Case {
                action: |kit| A::ExploreReferences(kit.key(MODEL)),
                setup: no_setup,
                check: |app, kit, _| {
                    let explorer = app.dialogs.get::<ContentExplorer>().ok_or("no explorer")?;
                    ensure(explorer.focus.key == kit.key(MODEL), "focus")?;
                    ensure(explorer.parents.is_empty() && explorer.children.is_empty(), "empty")?;
                    ensure(explorer.index_unavailable, "index unavailable")
                },
            },
            // 37: with an index, the report goes to a save dialog.
            Case {
                action: |kit| A::DumpReferences(kit.key(MODEL)),
                setup: no_setup,
                check: |app, _, _| {
                    status_is(
                        app,
                        "Build the reference index first — Tools ▸ Build/Rebuild Reference Index",
                    )
                },
            },
            // 38
            Case {
                action: |kit| A::LaunchScenarioInSapien(kit.key(MODEL)),
                setup: no_setup,
                check: |app, _, _| status_is(app, "Only scenario tags can be launched"),
            },
            // 39
            Case {
                action: |kit| A::LaunchScenarioInTagTest(kit.key(MODEL)),
                setup: no_setup,
                check: |app, _, _| status_is(app, "Only scenario tags can be launched"),
            },
            // 40
            Case {
                action: |kit| A::RenameTag(kit.key(RENDER)),
                setup: |app, kit| {
                    app.model.kits[0].source.as_mut().unwrap().reverse_dependencies = Some(kit.index());
                },
                check: |app, kit, _| {
                    let state = app.dialogs.get::<RenameTagState>().ok_or("no dialog")?;
                    ensure(state.key == kit.key(RENDER), "key")?;
                    ensure(state.operation == TagNameOperation::Rename, "operation")?;
                    ensure(state.old_display == RENDER, "old display")?;
                    ensure(state.extension == "render_model", "extension")?;
                    ensure(state.fixed_parent == FOLDER, format!("parent {:?}", state.fixed_parent))?;
                    ensure(state.new_path_input == "crate", format!("input {:?}", state.new_path_input))?;
                    ensure(state.referrers == vec![MODEL.to_owned()], format!("{:?}", state.referrers))?;
                    ensure(!state.referrers_unavailable, "index available")?;
                    ensure(!state.is_container && !state.whole_path_editable, "loose")
                },
            },
            // 41
            Case {
                action: |kit| A::DuplicateTag(kit.key(MODEL)),
                setup: no_setup,
                check: |app, _, _| {
                    let state = app.dialogs.get::<RenameTagState>().ok_or("no dialog")?;
                    ensure(state.operation == TagNameOperation::Duplicate, "operation")?;
                    ensure(state.focus_input, "the name field takes focus")?;
                    ensure(state.referrers_unavailable, "no index")
                },
            },
            // 42
            Case {
                action: |kit| A::DeleteTag(kit.key(MODEL)),
                setup: dirty_model,
                check: |app, kit, _| {
                    let confirm = app
                        .dialogs
                        .get::<DeleteConfirm>()
                        .ok_or("no confirmation")?;
                    ensure(confirm.key == kit.key(MODEL), "key")?;
                    ensure(confirm.display_path == MODEL, "display path")?;
                    ensure(matches!(confirm.kind, DeleteKind::Loose), "loose")?;
                    ensure(confirm.has_unsaved_edits, "unsaved edits flagged")?;
                    ensure(confirm.referrers_unavailable && confirm.referrers.is_empty(), "no index")
                },
            },
            // 43
            Case {
                action: |kit| A::MoveTag(kit.key(BARREL)),
                setup: dirty_model,
                check: |app, _, _| status_is(app, "Save or close dirty tags before moving"),
            },
            // 44
            Case {
                action: |_| A::ImportTagInFolder {
                    folder_rel: Some(FOLDER.to_owned()),
                },
                setup: no_setup,
                check: |app, _, _| status_is(app, "Import tag is only for Campaign Evolved containers"),
            },
            // 45
            Case {
                action: |_| A::NewTagInFolder {
                    folder_rel: Some(format!("{FOLDER}/")),
                },
                setup: no_setup,
                check: |app, _, _| {
                    let Some(dialog) = app.dialogs.get::<NewTagDialog>() else {
                        return ensure(false, "the dialog opened");
                    };
                    ensure(
                        dialog.rel_path == format!("{FOLDER}/"),
                        format!("path {:?}", dialog.rel_path),
                    )
                },
            },
            // 46: offered only on container kits; the arm itself does not check.
            Case {
                action: |_| A::NewContainerFolder {
                    parent_rel: Some("objects\\props".to_owned()),
                },
                setup: no_setup,
                check: |app, _, _| {
                    let dialog = app
                        .dialogs
                        .get::<ContainerFolderDialog>()
                        .ok_or("no dialog")?;
                    ensure(dialog.kit == app.model.kits[0].id, "kit")?;
                    ensure(
                        dialog.parent_rel.as_deref() == Some(FOLDER),
                        format!("parent {:?}", dialog.parent_rel),
                    )?;
                    ensure(dialog.renaming.is_none() && dialog.name_input.is_empty(), "new")
                },
            },
            // 47
            Case {
                action: |_| A::RenameContainerFolder {
                    rel: "objects/props".to_owned(),
                },
                setup: no_setup,
                check: |app, _, _| {
                    let dialog = app
                        .dialogs
                        .get::<ContainerFolderDialog>()
                        .ok_or("no dialog")?;
                    ensure(dialog.parent_rel.as_deref() == Some("objects"), "parent")?;
                    ensure(dialog.renaming.as_deref() == Some(FOLDER), "renaming")?;
                    ensure(dialog.name_input == "props", "prefilled leaf")
                },
            },
            // 48: on a loose kit there is no pending folder to remove, and the
            // arm reports a removal anyway (QUIRK; the menu never offers it here).
            Case {
                action: |_| A::DeleteContainerFolder {
                    rel: "objects/props".to_owned(),
                },
                setup: no_setup,
                check: |app, kit, _| {
                    status_is(app, "Removed folder objects/props")?;
                    ensure(app.model.kits[0].pending_container_folders.is_empty(), "nothing pending")?;
                    ensure(kit.root.join(FOLDER).is_dir(), "the folder on disk is untouched")
                },
            },
        ]
    }

    /// Run one case on a fresh app over the shared kit.
    fn run(case: &Case, kit: &LooseKit) -> Result<(), String> {
        let mut app = Baboon::for_test();
        kit.install(&mut app);
        (case.setup)(&mut app, kit);
        // Whatever the setup left in flight is settled before the dispatch, so
        // a worker answering afterwards is the arm's own.
        drain_messages(&mut app, Duration::from_millis(50));
        app.model.status = "Ready".to_owned();
        let mut action = Some((case.action)(kit));
        let ctx = egui::Context::default();
        let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            if let Some(action) = action.take() {
                app.handle_browser_action(action, ui.ctx().clone())
            }
        });
        let worker_answered = app.rx.recv_timeout(Duration::from_millis(150)).is_ok();
        let outcome = Outcome {
            copied_text: crate::app::copied_text(&output.platform_output),
            worker_answered,
        };
        (case.check)(&app, kit, &outcome)
    }

    #[test]
    fn every_browser_action_variant_has_a_case() {
        let kit = fixture();
        let covered: BTreeSet<usize> = cases()
            .iter()
            .map(|case| variant_index(&(case.action)(&kit)))
            .collect();
        let missing: Vec<usize> = (0..VARIANTS).filter(|index| !covered.contains(index)).collect();
        assert!(missing.is_empty(), "variants without a case: {missing:?}");
        assert_eq!(cases().len(), VARIANTS, "one case per variant");
    }

    #[test]
    fn every_browser_action_leaves_its_state() {
        let kit = fixture();
        let failures: Vec<String> = cases()
            .iter()
            .enumerate()
            .filter_map(|(index, case)| {
                let action = (case.action)(&kit);
                let variant = variant_index(&action);
                run(case, &kit)
                    .err()
                    .map(|error| format!("case {index} (variant {variant}): {error}"))
            })
            .collect();
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    /// A second open of the same folder reuses its pane; "open in new tab"
    /// makes another beside it.
    #[test]
    fn opening_a_folder_twice_reuses_its_pane_unless_asked_for_a_new_tab() {
        let kit = fixture();
        let mut app = Baboon::for_test();
        kit.install(&mut app);
        let open = |app: &mut Baboon, new_tab: bool| {
            app.handle_browser_action(
                BrowserAction::OpenFolderBrowser {
                    rel_path: PathBuf::from(FOLDER),
                    label: "props".to_owned(),
                    open_in_new_tab: new_tab,
                },
                ctx(),
            )
        };
        open(&mut app, false);
        open(&mut app, false);
        assert_eq!(app.views[app.model.kits[0].id].browser.folder_browsers.len(), 1);
        open(&mut app, true);
        let base = folder_pane_key(Path::new(FOLDER));
        let mut keys: Vec<_> = app.views[app.model.kits[0].id].browser.folder_browsers.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, vec![base.clone(), format!("{base}#2")]);
    }

    /// Toggling a favorite twice takes it back out, and an emptied kit drops
    /// its favorites entry altogether.
    #[test]
    fn a_favorite_toggled_twice_is_gone() {
        let kit = fixture();
        let mut app = Baboon::for_test();
        kit.install(&mut app);
        for _ in 0..2 {
            app.handle_browser_action(BrowserAction::ToggleFavorite(kit.key(MODEL)), ctx());
        }
        assert_eq!(app.model.status, format!("Removed {MODEL} from Favorites"));
        assert!(app.model.prefs.editing_kit_favorites.is_empty());
        assert!(app.model.kits[0].active_favorite_entries.is_empty());
    }

    #[test]
    fn browser_refresh_discards_lazy_entries_and_relists_folders() {
        let root = std::env::temp_dir().join(format!(
            "baboon-browser-refresh-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("objects/old")).unwrap();
        let mut tree = crate::core::source::build_folder_directory_tree(&root).unwrap();
        let mut entries = vec![TagEntry {
            key: "stale".to_owned(),
            display_path: "objects/old/stale.weapon".to_owned(),
            group_tag: u32::from_be_bytes(*b"weap"),
            group_name: None,
            location: TagEntryLocation::LooseFile(root.join("objects/old/stale.weapon")),
        }];

        std::fs::create_dir_all(root.join("new_folder")).unwrap();
        reset_lazy_folder_browser(&root, &mut tree, &mut entries).unwrap();

        assert!(entries.is_empty());
        assert!(tree.children.iter().any(|node| node.label == "new_folder"));
        assert!(tree.children.iter().all(|node| !node.entries_loaded));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn favorite_folders_at_or_under_a_moved_folder_follow_it() {
        let mut folders = vec![
            PathBuf::from("objects/props"),
            PathBuf::from("Objects/Props/Barrels"),
            PathBuf::from("objects/propsheet"),
            PathBuf::from("levels/test"),
        ];
        remap_favorite_folders(&mut folders, Path::new("objects/props"), Path::new("levels/crates"));
        assert_eq!(
            folders,
            vec![
                PathBuf::from("levels/crates"),
                PathBuf::from("levels/crates/Barrels"),
                PathBuf::from("objects/propsheet"),
                PathBuf::from("levels/test"),
            ]
        );
    }

    /// Open Folder did nothing but say "only available on Windows" on macOS
    /// and Linux. Every platform now launches its file manager on the folder.
    #[test]
    fn open_folder_launches_the_platform_file_manager() {
        let folder = std::env::temp_dir();
        let mut app = Baboon::for_test();
        let mut launched = None;
        app.open_folder_with(folder.clone(), "Tag", |command| {
            launched = Some((
                command.get_program().to_owned(),
                command.get_args().map(ToOwned::to_owned).collect::<Vec<_>>(),
            ));
            Ok(())
        });
        let expected = if cfg!(windows) {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        assert_eq!(
            launched,
            Some((expected.into(), vec![folder.clone().into_os_string()]))
        );
        assert!(app.model.status.starts_with("Opened Tag folder"), "{}", app.model.status);

        // A folder that is not there launches nothing.
        let mut launched = false;
        app.open_folder_with(folder.join("baboon-no-such-folder"), "Tag", |_| {
            launched = true;
            Ok(())
        });
        assert!(!launched);
        assert!(app.model.status.contains("not found"), "{}", app.model.status);
    }

    #[test]
    fn explorer_select_arguments_keep_switch_separate_from_path_with_spaces() {
        let path = Path::new(r"C:\Program Files\H2EK\tags\objects\example.weapon");

        assert_eq!(
            explorer_select_args(path),
            [
                std::ffi::OsString::from("/select,"),
                path.as_os_str().to_owned(),
            ]
        );
    }

    #[test]
    fn favorite_folder_explorer_path_stays_bound_to_its_rendered_tags_root() {
        let windows_rendered = Path::new(r"D:\HREK\tags\objects\characters");
        assert_eq!(
            loose_folder_explorer_path(Path::new(r"C:\OtherKit\tags"), windows_rendered),
            windows_rendered
        );

        let native_tags_root = std::env::temp_dir().join("baboon-hrek").join("tags");
        let native_relative = Path::new("objects").join("characters");
        let native_rendered = native_tags_root.join(&native_relative);
        assert_eq!(
            loose_folder_explorer_path(&native_tags_root, &native_relative),
            native_rendered
        );
    }

    #[test]
    fn moved_tags_remap_favorite_relative_paths() {
        let root = PathBuf::from("C:/Games/H2EK/tags");
        let old_relative = PathBuf::from("objects/old/brute.model");
        let new_relative = PathBuf::from("objects/characters/brute/brute.model");
        let mut favorites = vec![old_relative.clone(), PathBuf::from("sound/brute.sound")];
        let mut remap = HashMap::new();
        remap.insert(
            file_entry_key(&root.join(&old_relative)),
            file_entry_key(&root.join(&new_relative)),
        );

        remap_favorite_paths(&root, &mut favorites, &remap);

        assert_eq!(favorites[0], new_relative);
        assert_eq!(favorites[1], PathBuf::from("sound/brute.sound"));
    }
}
