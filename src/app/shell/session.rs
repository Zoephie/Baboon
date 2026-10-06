//! The session: what is open is saved on exit and offered back on the next
//! start through the Last Opened Windows prompt.

use super::*;
use crate::app::prefs::{
    browser_mode_from_str, browser_mode_str, browser_sort_from_str, browser_sort_str,
    write_text_atomic,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum LastSessionSourceKind {
    SingleFile,
    LooseFolder,
    MonolithicCache,
    IoStoreContainerSet,
}

impl LastSessionSourceKind {
    pub(in crate::app) fn as_str(self) -> &'static str {
        match self {
            LastSessionSourceKind::SingleFile => "single_file",
            LastSessionSourceKind::LooseFolder => "loose_folder",
            LastSessionSourceKind::MonolithicCache => "monolithic_cache",
            LastSessionSourceKind::IoStoreContainerSet => "iostore_container_set",
        }
    }

    pub(in crate::app) fn from_str(value: &str) -> Option<Self> {
        match value {
            "single_file" => Some(Self::SingleFile),
            "loose_folder" => Some(Self::LooseFolder),
            "monolithic_cache" => Some(Self::MonolithicCache),
            "iostore_container_set" => Some(Self::IoStoreContainerSet),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct LastSessionTag {
    pub(in crate::app) key: String,
    pub(in crate::app) label: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) path: Option<PathBuf>,
}

/// One kit's worth of saved session: its source and the tag/folder panes it had open.
#[derive(Clone, Debug)]
pub(in crate::app) struct LastSessionKit {
    pub(in crate::app) source_kind: LastSessionSourceKind,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) game: Option<String>,
    pub(in crate::app) profile_id: Option<String>,
    /// The `.baboon` this kit had open, to reattach as its save target. `None`
    /// for a workspace whose edits only ever lived in its recovery file.
    pub(in crate::app) project_path: Option<PathBuf>,
    /// Whether this kit carried a Baboon project at all. A workspace with a
    /// stash but no named project file is still worth reopening — the project
    /// *is* the session — and that no longer follows from `project_path`.
    pub(in crate::app) has_project: bool,
    /// The browser view this kit was in, or `None` when the session predates
    /// per-kit views — the restored kit then falls back to the saved default.
    pub(in crate::app) browser_mode: Option<BrowserMode>,
    pub(in crate::app) browser_sort: Option<BrowserSort>,
    pub(in crate::app) tags: Vec<LastSessionTag>,
    pub(in crate::app) folders: Vec<LastSessionFolder>,
    pub(in crate::app) chimp_packages: Vec<String>,
    pub(in crate::app) active_chimp_package: Option<String>,
    /// Whether this workspace had the Bitmap Library tab open.
    ///
    /// Carried as a flag rather than as a `tags` entry: it is not a tag, has no
    /// document behind it, and nothing in the source resolves its pane key — the
    /// tag loop would drop it on the way out and again on the way back in.
    pub(in crate::app) bitmap_library_open: bool,
    /// Whether this workspace had the Model Library tab open, carried the same
    /// way for the same reason.
    pub(in crate::app) model_library_open: bool,
    /// Whether this was the kit the user was looking at. Carried per kit rather
    /// than as an index into the list so that dropping a kit — which the
    /// restore prompt lets the user do — cannot silently re-point it at
    /// whichever workspace slid into that slot.
    pub(in crate::app) was_active: bool,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct LastSessionFolder {
    pub(in crate::app) rel_path: PathBuf,
    pub(in crate::app) label: String,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct LastSessionState {
    pub(in crate::app) kits: Vec<LastSessionKit>,
}

/// One kit to reopen during a session restore.
pub(in crate::app) struct RestoreKit {
    pub(in crate::app) source_kind: LastSessionSourceKind,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) profile_id: Option<String>,
    pub(in crate::app) project_path: Option<PathBuf>,
    /// The browser view this kit was in, or `None` when the session predates
    /// per-kit views — the restored kit then falls back to the saved default.
    pub(in crate::app) browser_mode: Option<BrowserMode>,
    pub(in crate::app) browser_sort: Option<BrowserSort>,
    pub(in crate::app) tags: Vec<LastSessionTag>,
    pub(in crate::app) folders: Vec<LastSessionFolder>,
    pub(in crate::app) chimp_packages: Vec<String>,
    pub(in crate::app) active_chimp_package: Option<String>,
    /// Whether this workspace had the Bitmap Library tab open.
    ///
    /// Carried as a flag rather than as a `tags` entry: it is not a tag, has no
    /// document behind it, and nothing in the source resolves its pane key — the
    /// tag loop would drop it on the way out and again on the way back in.
    pub(in crate::app) bitmap_library_open: bool,
    /// Whether this workspace had the Model Library tab open, carried the same
    /// way for the same reason.
    pub(in crate::app) model_library_open: bool,
    /// Whether this kit was the focused one when the session was saved.
    pub(in crate::app) was_active: bool,
}

pub(in crate::app) struct LastOpenedWindowEntry {
    pub(in crate::app) tag: LastSessionTag,
    pub(in crate::app) checked: bool,
    pub(in crate::app) available: bool,
}

pub(in crate::app) struct LastOpenedChimpEntry {
    pub(in crate::app) package: String,
    pub(in crate::app) checked: bool,
    pub(in crate::app) available: bool,
}

pub(in crate::app) struct LastOpenedFolderEntry {
    pub(in crate::app) folder: LastSessionFolder,
    pub(in crate::app) checked: bool,
    pub(in crate::app) available: bool,
}

/// One kit's section of the restore prompt.
pub(in crate::app) struct LastOpenedWindowsKit {
    /// Reopen this workspace, independently of its selected panes.
    pub(in crate::app) checked: bool,
    pub(in crate::app) source_kind: LastSessionSourceKind,
    pub(in crate::app) source_path: PathBuf,
    pub(in crate::app) game: Option<String>,
    pub(in crate::app) profile_id: Option<String>,
    /// Current display identity resolved from Editing Kits settings. Kept out
    /// of the session file because the stable profile ID lets renames appear
    /// here immediately without rewriting the saved session.
    pub(in crate::app) profile_name: Option<String>,
    pub(in crate::app) profile_root: Option<PathBuf>,
    pub(in crate::app) source_available: bool,
    pub(in crate::app) project_path: Option<PathBuf>,
    pub(in crate::app) has_project: bool,
    /// The browser view this kit was in, or `None` when the session predates
    /// per-kit views — the restored kit then falls back to the saved default.
    pub(in crate::app) browser_mode: Option<BrowserMode>,
    pub(in crate::app) browser_sort: Option<BrowserSort>,
    pub(in crate::app) entries: Vec<LastOpenedWindowEntry>,
    pub(in crate::app) folder_entries: Vec<LastOpenedFolderEntry>,
    pub(in crate::app) chimp_entries: Vec<LastOpenedChimpEntry>,
    /// Whether this workspace had the Bitmap Library tab open.
    ///
    /// Carried as a flag rather than as a `tags` entry: it is not a tag, has no
    /// document behind it, and nothing in the source resolves its pane key — the
    /// tag loop would drop it on the way out and again on the way back in.
    pub(in crate::app) bitmap_library_open: bool,
    /// Whether this workspace had the Model Library tab open, carried the same
    /// way for the same reason.
    pub(in crate::app) model_library_open: bool,
    pub(in crate::app) active_chimp_package: Option<String>,
    /// Whether this kit was the focused one when the session was saved.
    pub(in crate::app) was_active: bool,
}

/// Launch-time restore prompt backed by `last_session.json`. OK reloads each
/// saved kit's source; as each async load completes, that kit's queued tag and
/// folder panes are reopened through their normal paths. Restores are independent,
/// so the kits can finish loading in any order.
pub(in crate::app) struct LastOpenedWindowsPrompt {
    pub(in crate::app) kits: Vec<LastOpenedWindowsKit>,
    /// "Don't ask again": on OK, remember as Always; on Cancel, as Never.
    pub(in crate::app) dont_ask_again: bool,
}

impl LastOpenedWindowsKit {
    fn from_saved(
        saved: LastSessionKit,
        profile: Option<&CustomEditingKitProfile>,
    ) -> Option<Self> {
        let availability_path = profile
            .map(|profile| profile.root.as_path())
            .unwrap_or(&saved.source_path);
        let source_available = match saved.source_kind {
            LastSessionSourceKind::SingleFile => availability_path.is_file(),
            LastSessionSourceKind::LooseFolder => availability_path.is_dir(),
            LastSessionSourceKind::MonolithicCache => {
                if availability_path.is_dir() {
                    availability_path.join("blob_index.dat").is_file()
                } else {
                    availability_path.is_file()
                        && availability_path
                            .file_name()
                            .is_some_and(|name| name.eq_ignore_ascii_case("blob_index.dat"))
                }
            }
            LastSessionSourceKind::IoStoreContainerSet => {
                crate::core::source::find_paks_dir(availability_path).is_some()
            }
        };
        let entries = saved
            .tags
            .into_iter()
            .map(|tag| {
                let tag_available = tag.path.as_ref().map(|path| path.exists()).unwrap_or(true);
                let available = source_available && tag_available;
                LastOpenedWindowEntry {
                    tag,
                    checked: available,
                    available,
                }
            })
            .collect::<Vec<_>>();
        let chimp_entries = saved
            .chimp_packages
            .into_iter()
            .map(|package| LastOpenedChimpEntry {
                package,
                checked: source_available,
                available: source_available,
            })
            .collect::<Vec<_>>();
        let folder_entries = saved
            .folders
            .into_iter()
            .map(|folder| LastOpenedFolderEntry {
                folder,
                checked: source_available,
                available: source_available,
            })
            .collect::<Vec<_>>();
        Some(Self {
            checked: source_available,
            source_kind: saved.source_kind,
            source_path: saved.source_path,
            game: saved.game,
            profile_id: saved.profile_id,
            profile_name: profile.map(|profile| profile.name.clone()),
            profile_root: profile.map(|profile| profile.root.clone()),
            source_available,
            project_path: saved.project_path,
            has_project: saved.has_project,
            browser_mode: saved.browser_mode,
            browser_sort: saved.browser_sort,
            entries,
            folder_entries,
            chimp_entries,
            active_chimp_package: saved.active_chimp_package,
            // Restored with the workspace rather than offered as a checkbox,
            // like the browser view beside it: the library is a view onto the
            // kit, not a document that could be missing or unsaved.
            bitmap_library_open: saved.bitmap_library_open && source_available,
            model_library_open: saved.model_library_open && source_available,
            was_active: saved.was_active,
        })
    }

    pub(in crate::app) fn checked_tags(&self) -> Vec<LastSessionTag> {
        self.entries
            .iter()
            .filter(|entry| entry.available && entry.checked)
            .map(|entry| entry.tag.clone())
            .collect()
    }

    pub(in crate::app) fn checked_chimp_packages(&self) -> Vec<String> {
        self.chimp_entries
            .iter()
            .filter(|entry| entry.available && entry.checked)
            .map(|entry| entry.package.clone())
            .collect()
    }

    pub(in crate::app) fn checked_folders(&self) -> Vec<LastSessionFolder> {
        self.folder_entries
            .iter()
            .filter(|entry| entry.available && entry.checked)
            .map(|entry| entry.folder.clone())
            .collect()
    }
}

impl LastOpenedWindowsPrompt {
    pub(in crate::app) fn from_session(
        session: LastSessionState,
        profiles: &[CustomEditingKitProfile],
    ) -> Option<Self> {
        let kits = session
            .kits
            .into_iter()
            .filter_map(|saved| {
                let profile = saved
                    .profile_id
                    .as_deref()
                    .and_then(|id| profiles.iter().find(|profile| profile.id == id));
                LastOpenedWindowsKit::from_saved(saved, profile)
            })
            .collect::<Vec<_>>();
        if kits.is_empty() {
            return None;
        }
        Some(Self {
            kits,
            dont_ask_again: false,
        })
    }

    /// Selected, available workspaces, paired with their selected panes.
    pub(in crate::app) fn checked_kits(&self) -> Vec<RestoreKit> {
        self.kits
            .iter()
            .filter(|kit| kit.checked && kit.source_available)
            .map(|kit| {
                let tags = kit.checked_tags();
                let chimp_packages = kit.checked_chimp_packages();
                let folders = kit.checked_folders();
                let active_chimp_package = kit
                    .active_chimp_package
                    .clone()
                    .filter(|active| chimp_packages.contains(active));
                RestoreKit {
                    source_kind: kit.source_kind,
                    source_path: kit.source_path.clone(),
                    profile_id: kit.profile_id.clone(),
                    project_path: kit.project_path.clone(),
                    browser_mode: kit.browser_mode,
                    browser_sort: kit.browser_sort,
                    tags,
                    folders,
                    chimp_packages,
                    active_chimp_package,
                    bitmap_library_open: kit.bitmap_library_open,
                    model_library_open: kit.model_library_open,
                    was_active: kit.was_active,
                }
            })
            .collect()
    }

    pub(in crate::app) fn has_reopenable_kits(&self) -> bool {
        self.kits
            .iter()
            .any(|kit| kit.checked && kit.source_available)
    }
}
use crate::app::kits::loading::loose_entry_key_for_canonical_path;

impl Baboon {
    /// Snapshot every kit's source and open tag/folder panes for the restore prompt.
    pub(in crate::app) fn current_session_state(&self) -> Option<LastSessionState> {
        let kits = (0..self.model.kits.len())
            .filter_map(|index| self.session_kit_state(index))
            .collect::<Vec<_>>();
        (!kits.is_empty()).then_some(LastSessionState { kits })
    }

    pub(in crate::app) fn session_kit_state(&self, kit_index: usize) -> Option<LastSessionKit> {
        let kit = &self.model.kits[kit_index];
        let view = &self.views[kit.id];
        let was_active = kit_index == self.model.active;
        let source = kit.source.as_ref()?;
        let (source_kind, source_path) = match &source.source {
            TagSource::SingleFile { path } => (LastSessionSourceKind::SingleFile, path.clone()),
            TagSource::LooseFolder { root, .. } => {
                (LastSessionSourceKind::LooseFolder, root.clone())
            }
            TagSource::MonolithicCache { root, .. } => {
                (LastSessionSourceKind::MonolithicCache, root.clone())
            }
            TagSource::IoStoreContainerSet { root, .. } => {
                (LastSessionSourceKind::IoStoreContainerSet, root.clone())
            }
        };
        // Record the folder the user actually chose, not the directory the
        // source ended up reading from. They differ for exactly the sources
        // whose root is resolved inwards: a container set mounts from
        // `<install>/Meteorite/Content/Paks`, and a loose kit from
        // `<kit>/tags`. Storing the resolved one meant every session restore
        // reloaded that inner path and remembered *it* as a recent folder, so
        // "Paks" reappeared in the recents list after each restart however
        // often it was removed.
        let source_path = kit.requested_path.clone().unwrap_or(source_path);
        let mut tags = Vec::new();
        for key in ordered_unique_keys(kit.open_tabs.iter()) {
            let Some(entry) = source.entry_for_key(&key) else {
                continue;
            };
            let path = match &entry.location {
                TagEntryLocation::LooseFile(path) => {
                    Some(fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
                }
                TagEntryLocation::Monolithic { .. }
                | TagEntryLocation::Container { .. }
                | TagEntryLocation::NewContainer { .. } => None,
            };
            tags.push(LastSessionTag {
                key: entry.key.clone(),
                label: format!(
                    "{} - {}",
                    entry.display_path,
                    group_label(&kit.names, entry.group_tag)
                ),
                group_tag: entry.group_tag,
                path,
            });
        }
        let folders = ordered_unique_keys(kit.open_tabs.iter())
            .into_iter()
            .filter_map(|key| view.browser.folder_browsers.get(&key))
            .map(|folder| LastSessionFolder {
                rel_path: folder.rel_path.clone(),
                label: folder.label.clone(),
            })
            .collect();
        let chimp_packages = ordered_unique_keys(kit.chimp.open_packages.iter());
        let active_chimp_package = kit
            .chimp
            .selected_package
            .clone()
            .filter(|active| chimp_packages.contains(active));
        // The source itself is part of the workspace session, even when the
        // user has no tag, Chimp package, or project open in it. Otherwise a
        // second loaded editing kit disappears from the next-session prompt
        // simply because its tags were not selected yet.
        Some(LastSessionKit {
            source_kind,
            source_path,
            game: source.game.map(|game| game.as_str().to_owned()),
            profile_id: kit.profile.as_ref().map(|profile| profile.id.clone()),
            // The `.baboon` this workspace has open, if any — not its recovery
            // file, which the next session finds from the source root anyway.
            project_path: kit
                .project.active
                .as_ref()
                .and_then(|project| project.project_path.clone()),
            has_project: kit.project.active.is_some(),
            browser_mode: Some(view.browser.mode),
            browser_sort: Some(view.browser.sort),
            tags,
            folders,
            chimp_packages,
            active_chimp_package,
            // Read off the open tabs rather than the tag list: the libraries'
            // pane keys resolve to no entry, so the loop above skipped them.
            bitmap_library_open: kit.open_tabs.iter().any(|key| key == BITMAP_LIBRARY_KEY),
            model_library_open: kit.open_tabs.iter().any(|key| key == MODEL_LIBRARY_KEY),
            was_active,
        })
    }

    /// Reopen each saved kit. Every kit gets its own load, and its panes are
    /// staged on the kit itself rather than in one shared slot, so the loads
    /// can finish in any order without stealing each other's restore state.
    pub(in crate::app) fn begin_last_session_restore(&mut self, kits: Vec<RestoreKit>, ctx: egui::Context) {
        for RestoreKit {
            source_kind,
            source_path,
            profile_id,
            project_path,
            browser_mode,
            browser_sort,
            tags,
            folders,
            chimp_packages,
            active_chimp_package,
            bitmap_library_open,
            model_library_open,
            was_active,
        } in kits
        {
            match source_kind {
                LastSessionSourceKind::SingleFile => {
                    self.begin_load_single_path(source_path, ctx.clone())
                }
                LastSessionSourceKind::LooseFolder => {
                    let started = if let Some(profile) = profile_id
                        .as_deref()
                        .and_then(|id| {
                            self.model.prefs
                                .custom_editing_kit_profiles
                                .iter()
                                .find(|profile| profile.id == id)
                        })
                        .cloned()
                    {
                        self.load_custom_editing_kit_profile(profile, ctx.clone())
                    } else {
                        self.begin_load_folder_path(source_path, ctx.clone());
                        true
                    };
                    if !started {
                        continue;
                    }
                }
                LastSessionSourceKind::MonolithicCache => {
                    let blob_index = if source_path.is_dir() {
                        source_path.join("blob_index.dat")
                    } else {
                        source_path
                    };
                    self.begin_load_monolithic_path(blob_index, ctx.clone());
                }
                // Upstream added container sources to the session format, so a
                // Campaign Evolved install now comes back with the rest.
                LastSessionSourceKind::IoStoreContainerSet => {
                    self.begin_load_folder_path(install_root_for_paks(&source_path), ctx.clone())
                }
            }
            // The loaders route to a kit and leave it active, so this stages
            // the tags on the kit the load will land in.
            //
            // Each load also finishes by making its own kit active, so the
            // focused workspace would otherwise be whichever one happened to
            // load last. Remember the kit the session named and every kit still
            // to land, so the focus can be set once they all have.
            let restoring = self.model.kits[self.model.active].id;
            self.shell.restoring_kits.insert(restoring);
            if was_active {
                self.shell.restored_active_kit = Some(restoring);
            }
            self.model.kits[self.model.active].restore.pending_restore_tags = tags;
            self.model.kits[self.model.active].restore.pending_restore_folders = folders;
            self.model.kits[self.model.active].restore.pending_restore_chimp_packages = chimp_packages;
            self.model.kits[self.model.active].restore.pending_restore_bitmap_library = bitmap_library_open;
            self.model.kits[self.model.active].restore.pending_restore_model_library = model_library_open;
            self.model.kits[self.model.active].restore.pending_restore_active_chimp_package = active_chimp_package;
            // Its browser view is staged the same way: `install_loaded_source`
            // carries it across the load rather than resetting it, so each
            // workspace comes back in the view it was left in.
            if let Some(mode) = browser_mode {
                self.views[self.model.kits[self.model.active].id].browser.mode = mode;
            }
            if let Some(sort) = browser_sort {
                self.views[self.model.kits[self.model.active].id].browser.sort = sort;
            }
            // The project file it had open is queued the same way, and is
            // attached as this workspace's save target once the source has
            // mounted. The edits themselves come back from the recovery file.
            if let Some(project_path) = project_path {
                let restoring = self.model.active;
                self.queue_campaign_project_target(restoring, project_path);
            }
        }
    }

    /// Record the session as the event loop tears down.
    ///
    /// Baboon's whole shutdown chain hangs off a window close request:
    /// `handle_app_close_request` only acts on `close_requested()`, and it is
    /// what eventually reaches [`Self::execute_close_action`] and saves the
    /// session. macOS never sends one for Cmd+Q — AppKit posts
    /// `applicationWillTerminate:`, which closes each window directly rather
    /// than asking it to close, so no `CloseRequested` is ever emitted and none
    /// of that runs. The session file was then left holding whatever last wrote
    /// it, which for a Campaign Evolved workspace is its project autosave: quit
    /// with a Halo 3 kit open and the next launch restored Campaign Evolved,
    /// because that was the last session anything had recorded.
    ///
    /// This runs on every shutdown, including the ordinary one that already
    /// saved a moment earlier — the write is the same document either way. It
    /// cannot prompt: the loop is already exiting and `LoopExiting` cannot be
    /// vetoed, so unsaved tag edits still go unremarked on a Cmd+Q.
    pub(in crate::app) fn persist_session_on_exit(&mut self) {
        match self.current_session_state() {
            Some(session) => {
                let _ = save_last_session(&session);
            }
            None => clear_last_session(),
        }
    }

    /// Mark one restored kit's load as settled, whatever became of it, and once
    /// none are left hand the focus to the kit the session named.
    ///
    /// Every completed load makes its own kit active, so during a restore the
    /// focused workspace is otherwise decided by which source finishes first —
    /// a loose folder against a container set is not a race with a stable
    /// winner. The saved kit is only honoured while it is still open and it
    /// still loaded; a kit the user unchecked in the restore prompt, or whose
    /// source has since moved, leaves the focus wherever the loads put it.
    pub(in crate::app) fn settle_restored_kit(&mut self, kit: KitId) {
        let Some(active) =
            focus_after_restore(&mut self.shell.restoring_kits, &mut self.shell.restored_active_kit, kit)
        else {
            return;
        };
        if let Some(index) = self.model.kit_index(active) {
            self.model.active = index;
        }
    }

    /// Reopen the panes staged for the kit that just finished loading.
    pub(in crate::app) fn finish_pending_session_restore(&mut self, ctx: egui::Context) {
        // Ahead of the early return below: a workspace whose only open tab was
        // the Bitmap Library has no tags staged, and would otherwise come back
        // without it.
        if std::mem::take(&mut self.model.kits[self.model.active].restore.pending_restore_bitmap_library) {
            self.open_bitmap_library();
        }
        if std::mem::take(&mut self.model.kits[self.model.active].restore.pending_restore_model_library) {
            self.open_model_library();
        }
        let restore_folders = std::mem::take(&mut self.model.kits[self.model.active].restore.pending_restore_folders);
        for folder in &restore_folders {
            self.handle_browser_action(
                BrowserAction::OpenFolderBrowser {
                    rel_path: folder.rel_path.clone(),
                    label: folder.label.clone(),
                    open_in_new_tab: true,
                },
                ctx.clone(),
            );
        }
        let restore = std::mem::take(&mut self.model.kits[self.model.active].restore.pending_restore_tags);
        if restore.is_empty() && restore_folders.is_empty() {
            return;
        }
        let mut opened = restore_folders.len();
        let mut missing = 0usize;
        for tag in restore {
            if let Some(current_key) = self.restored_tag_entry_key(&tag) {
                self.select_entry(current_key, ctx.clone());
                opened += 1;
            } else {
                missing += 1;
            }
        }
        if opened > 0 {
            self.model.status = if missing > 0 {
                format!("Restored {opened} window(s); skipped {missing} missing item(s)")
            } else {
                format!("Restored {opened} window(s)")
            };
        } else if missing > 0 {
            self.model.status = "No saved windows could be restored".to_owned();
        }
    }

    /// Resolve a saved pane to the key used by the freshly mounted source.
    ///
    /// Loose-file keys include a displayed filesystem path. Windows accepts
    /// both separators, and older sessions could therefore persist a mixed
    /// `file:C:\.../objects\...` spelling that no longer compared equal to the
    /// newly scanned entry. Rediscovering the file was not enough: restore then
    /// opened the stale saved key and reported that the tag had disappeared.
    /// Return the source's current key so existing sessions recover in place.
    pub(in crate::app) fn restored_tag_entry_key(&mut self, tag: &LastSessionTag) -> Option<String> {
        if let Some(entry) = self.model.entry_for_key(&tag.key) {
            return Some(entry.key.clone());
        }
        let path = tag.path.as_ref()?;
        if !path.is_file() {
            return None;
        }
        let source = self.model.source()?;
        let TagSource::LooseFolder { root, .. } = &source.source else {
            return None;
        };
        let root = fs::canonicalize(root).ok()?;
        let path = fs::canonicalize(path).ok()?;
        if !path.starts_with(&root) {
            return None;
        }
        if let Some(current_key) = loose_entry_key_for_canonical_path(
            source.entries.iter().chain(source.all_entries.iter()),
            &path,
        ) {
            return Some(current_key);
        }
        let entry = loose_file_entry(&root, &path, &source.names).ok()??;
        let current_key = entry.key.clone();
        let folder_seeds = self.model.kits[self.model.active].folder_seeds();
        if let Some(source) = self.source_mut() {
            if tag.key != current_key {
                source.remove_entry(&tag.key, &folder_seeds);
            }
            source.upsert_entry(entry, &folder_seeds);
        }
        self.model.kits[self.model.active].generation = self.model.kits[self.model.active].generation.wrapping_add(1);
        Some(current_key)
    }

}

pub(in crate::app) enum LastOpenedWindowsAction {
    None,
    Restore {
        /// Each kit to reopen, with the tags checked for it.
        kits: Vec<RestoreKit>,
        /// "Don't ask again" was ticked — remember this as `Always`.
        remember: bool,
    },
    Cancel {
        /// "Don't ask again" was ticked — remember this as `Never`.
        remember: bool,
    },
}

pub(in crate::app) fn last_opened_workspace_heading(
    profile: Option<(&str, &Path)>,
    game: Option<&str>,
    source_path: &Path,
    project_path: Option<&Path>,
) -> (String, Option<String>) {
    if let Some((name, root)) = profile {
        return (name.to_owned(), Some(root.display().to_string()));
    }
    if let Some(project_path) = project_path {
        let name = project_path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .or_else(|| project_path.file_name().and_then(|name| name.to_str()))
            .map(str::to_owned)
            .unwrap_or_else(|| project_path.display().to_string());
        return (name, Some(project_path.display().to_string()));
    }

    let heading = match game {
        Some(game) => game_display_name(game).to_owned(),
        None => source_path.display().to_string(),
    };
    (heading, None)
}

/// A fixed-height, full-width restore row, sharing Git Review's path styling.
fn restore_path_row(
    ui: &mut Ui,
    checked: &mut bool,
    available: bool,
    path: &str,
    group_tag: Option<u32>,
    game: Option<GameId>,
    folder: bool,
) {
    ui.add_enabled_ui(available, |ui| {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(20.0, 0.0))),
            |ui| {
                ui.horizontal_centered(|ui| {
                    ui.checkbox(checked, "");
                    let (icon_rect, _) =
                        ui.allocate_exact_size(Vec2::splat(BUTTON_ICON_SIZE), Sense::hover());
                    if folder {
                        paint_button_icon_at(ui, ButtonIcon::FolderOpen, icon_rect, text_dark());
                    } else {
                        paint_tag_icon_at(ui, group_tag, game, icon_rect);
                    }
                    let (text_rect, response) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
                    crate::app::compare::tag_compare::paint_path_label(ui, path, text_rect);
                    response.on_hover_text(if available {
                        path.to_owned()
                    } else {
                        format!("{path} (unavailable)")
                    });
                });
            },
        );
        ui.painter().hline(
            rect.x_range(),
            rect.bottom(),
            Stroke::new(1.0_f32, grid_line()),
        );
    });
}

/// Lay out both lines as one block so the entire heading is vertically centered.
fn restore_workspace_row(ui: &mut Ui, kit: &mut LastOpenedWindowsKit) -> (egui::Rect, egui::Rect) {
    let (heading, root) = last_opened_workspace_heading(
        kit.profile_name.as_deref().zip(kit.profile_root.as_deref()),
        kit.game.as_deref(),
        &kit.source_path,
        kit.project_path.as_deref(),
    );
    let root = root.unwrap_or_else(|| kit.source_path.display().to_string());
    let text_width = (ui.available_width() - 40.0).max(0.0);
    let heading = egui::WidgetText::from(RichText::new(heading).strong()).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        text_width,
        TextStyle::Body,
    );
    let path = egui::WidgetText::from(RichText::new(&root).color(subtle_dark())).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        text_width,
        TextStyle::Body,
    );
    let text_height = heading.size().y + 2.0 + path.size().y;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), (text_height + 12.0).max(44.0)),
        Sense::hover(),
    );
    let checkbox_size = ui.spacing().interact_size.y;
    let checkbox_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 8.0 + checkbox_size * 0.5, rect.center().y),
        Vec2::splat(checkbox_size),
    );
    // This row is already allocated. A child UI keeps the checkbox from
    // advancing the parent cursor and adding spacing below the row.
    let mut checkbox_ui = ui.new_child(egui::UiBuilder::new().max_rect(checkbox_rect));
    if !kit.source_available {
        checkbox_ui.disable();
    }
    checkbox_ui.put(
        checkbox_rect,
        egui::Checkbox::without_text(&mut kit.checked),
    );
    let text_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 32.0, rect.center().y - text_height * 0.5),
        Vec2::new(text_width, text_height),
    );
    let painter = ui.painter().with_clip_rect(rect);
    painter.galley(text_rect.min, heading.clone(), text_dark());
    painter.galley(
        text_rect.min + egui::vec2(0.0, heading.size().y + 2.0),
        path,
        subtle_dark(),
    );
    painter.hline(
        rect.x_range(),
        rect.bottom(),
        Stroke::new(1.0_f32, grid_line()),
    );
    response.on_hover_text(&root);
    if !kit.source_available {
        ui.label(
            RichText::new(format!("Missing source: {root}")).color(Color32::from_rgb(180, 48, 40)),
        );
    }
    (rect, text_rect)
}

pub(in crate::app) fn render_last_opened_windows_prompt(
    ctx: &egui::Context,
    prompt: Option<&mut LastOpenedWindowsPrompt>,
) -> LastOpenedWindowsAction {
    let Some(prompt) = prompt else {
        return LastOpenedWindowsAction::None;
    };

    let mut action = LastOpenedWindowsAction::None;
    egui::Window::new("Restore Last Opened Windows")
        .id(egui::Id::new("last_opened_windows"))
        .collapsible(false)
        .resizable([true, false])
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(520.0)
        .min_width(420.0)
        .show(ctx, |ui| {
            Frame::NONE
                .inner_margin(egui::Margin { left: 22, right: 8, top: 4, bottom: 8 })
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label("These windows were open the last time you used Baboon.");
                    ui.label("Which of these would you like to reopen?");
                });
            // The list's height budget is independent of the window's previous
            // size. Short sessions hug their contents; long sessions scroll.
            let list_height = (ctx.content_rect().height() - 160.0).clamp(64.0, 560.0);
            let list_rect = egui::Rect::from_min_size(
                ui.cursor().min, Vec2::new(ui.available_width(), list_height),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, true])
                .max_height(list_height)
                .min_scrolled_height(32.0)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.set_min_width(ui.available_width());
                    for (index, kit) in prompt.kits.iter_mut().enumerate() {
                        ui.push_id(index, |ui| {
                            restore_workspace_row(ui, kit);
                            let game = kit.game.as_deref().and_then(GameId::from_id);
                            if kit.entries.is_empty() && kit.has_project {
                                ui.label(RichText::new("Unsaved changes stashed in this workspace").color(subtle_dark()).small());
                            }
                            ui.add_enabled_ui(kit.checked, |ui| {
                            for entry in &mut kit.folder_entries {
                                restore_path_row(ui, &mut entry.checked, entry.available,
                                    &entry.folder.rel_path.display().to_string(), None, None, true);
                            }
                            for entry in &mut kit.entries {
                                // Session labels include " - group name" for the old
                                // plain-text list; the icon now communicates the group.
                                let path = entry.tag.label.rsplit_once(" - ").map_or(entry.tag.label.as_str(), |(path, _)| path);
                                restore_path_row(ui, &mut entry.checked, entry.available,
                                    path, Some(entry.tag.group_tag), game, false);
                            }
                            for entry in &mut kit.chimp_entries {
                                restore_path_row(ui, &mut entry.checked, entry.available,
                                    &entry.package, None, None, false);
                            }
                            });
                        });
                    }
                });
            });
            // Paint behind the controls, extending through the window margins
            // without changing their layout or the dialog's content height.
            let window_margin = ui.spacing().window_margin;
            let mut footer_painter = ui.painter().clone();
            // with_clip_rect intersects the existing content clip, so it cannot
            // expose the window margins. Replace the clip on this painter only.
            footer_painter.set_clip_rect(
                ui.clip_rect().expand(window_margin.sum().max_elem()).intersect(ctx.content_rect()),
            );
            let footer_background = footer_painter.add(egui::Shape::Noop);
            let footer = Frame::NONE
                // The fill includes the window's bottom margin. Balance that
                // extra space above the controls to center them in the fill.
                .inner_margin(egui::Margin {
                    left: 8, right: 8,
                    top: 6 + window_margin.bottom, bottom: 6,
                })
                .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 24.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                ui.checkbox(&mut prompt.dont_ask_again, "Don't ask me again")
                    .on_hover_text("Remember this choice: Restore Selected always reopens the last session; Close All never does. Change it later in File > Settings.");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("Close All").min_size(Vec2::new(78.0, 24.0))).clicked() {
                        action = LastOpenedWindowsAction::Cancel { remember: prompt.dont_ask_again };
                    }
                    if ui.add_enabled(prompt.has_reopenable_kits(),
                        egui::Button::new("Restore Selected").min_size(Vec2::new(110.0, 24.0))).clicked() {
                        action = LastOpenedWindowsAction::Restore {
                            kits: prompt.checked_kits(), remember: prompt.dont_ask_again,
                        };
                    }
                });
                });
            });
            let mut footer_rect = footer.response.rect;
            // Out to the window's edge: egui 0.36 counts its stroke as part
            // of the frame's margin, beside the padding.
            let edge = egui::Frame::window(ui.style()).total_margin();
            footer_rect.min.x -= edge.left;
            footer_rect.max.x += edge.right;
            footer_rect.max.y += edge.bottom;
            let mut rounding = ui.visuals().window_corner_radius;
            rounding.nw = 0;
            rounding.ne = 0;
            let header_fill = if ui.visuals().window_highlight_topmost
                && Some(ui.layer_id()) == ctx.top_layer_id()
            {
                ui.visuals().widgets.open.weak_bg_fill
            } else {
                ui.visuals().window_fill()
            };
            footer_painter.set(footer_background, egui::epaint::RectShape::filled(
                footer_rect, rounding, header_fill,
            ));
            footer_painter.hline(
                footer_rect.x_range(), footer_rect.top(),
                Stroke::new(1.0_f32, grid_line()),
            );
        });
    action
}

/// Retire one restored kit's load and report the kit that should take the focus
/// — `None` while any restore is still outstanding, or when the session named
/// no kit and there is nothing to honour.
///
/// Split out from [`Baboon::settle_restored_kit`] because it is the whole
/// decision: the app half only turns the answer into an index.
pub(in crate::app) fn focus_after_restore(
    restoring: &mut HashSet<KitId>,
    restored_active: &mut Option<KitId>,
    settled: KitId,
) -> Option<KitId> {
    // A load that was not part of the restore settles nothing, and neither does
    // one that still leaves others in flight.
    if !restoring.remove(&settled) || !restoring.is_empty() {
        return None;
    }
    restored_active.take()
}

// --- The session file: where it lives, and reading and writing it. ---

pub(in crate::app) fn last_session_path() -> PathBuf {
    crate::core::storage::data_path("last_session.json")
}

pub(in crate::app) fn load_last_session() -> Option<LastSessionState> {
    let text = fs::read_to_string(last_session_path()).ok()?;
    let value = serde_json::from_str::<Value>(&text).ok()?;
    parse_last_session(&value)
}

/// Pure parse of a session document, split out from the file read so every
/// format version is covered by tests.
fn parse_last_session(value: &Value) -> Option<LastSessionState> {
    let kits = match value.get("version").and_then(Value::as_u64)? {
        // Versions 1 and 2 each describe a single source, so they load as one
        // kit. They are both accepted because they were both written: v1 by
        // released Baboon, v2 by the build that added `.baboon` projects.
        1 | 2 => vec![parse_session_kit(value)?],
        // Versions 3 to 6 are that same per-kit object, once per open kit.
        // Version 4 adds optional Chimp package tabs, version 5 the Bitmap
        // Library flag, and version 6 folder panes. Each field is optional on
        // the way in, so older files still load and only lack what they never
        // recorded.
        3 | 4 | 5 | 6 => value
            .get("kits")?
            .as_array()?
            .iter()
            .filter_map(parse_session_kit)
            .collect(),
        _ => return None,
    };
    if kits.is_empty() {
        return None;
    }
    Some(LastSessionState { kits })
}

/// Parse one kit's `{source, tags}` object. Both format versions use the same
/// shape for this part, which is what makes the v1 upgrade a one-liner.
fn parse_session_kit(value: &Value) -> Option<LastSessionKit> {
    let source = value.get("source")?;
    let source_kind = LastSessionSourceKind::from_str(source.get("kind")?.as_str()?.trim())?;
    let source_path = source
        .get("path")?
        .as_str()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)?;
    let game = source
        .get("game")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|game| !game.is_empty())
        .map(str::to_owned);
    let profile_id = source
        .get("profile_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    let project_path = source
        .get("project_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    // Absent in sessions written while `project_path` still meant "the file this
    // workspace autosaves to", which was set for every workspace that had a
    // project at all — so its presence is exactly what this flag now records.
    let has_project = source
        .get("has_project")
        .and_then(Value::as_bool)
        .unwrap_or(project_path.is_some());
    // Absent in every session written before the focused workspace was
    // recorded, which reads back as "no kit was active" and leaves the restore
    // picking whichever kit it used to.
    let was_active = value
        .get("active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // Absent in sessions written before the browser view became per-kit, and
    // in every version-1 and version-2 file. `None` means "use the default".
    let browser_mode = browser_mode_from_str(value.get("browser_mode").and_then(Value::as_str));
    let browser_sort = browser_sort_from_str(value.get("browser_sort").and_then(Value::as_str));
    let mut tags = Vec::new();
    for item in value
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(key) = item
            .get("key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|key| !key.is_empty())
        else {
            continue;
        };
        let label = item
            .get("label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .unwrap_or(key)
            .to_owned();
        let group_tag = item.get("group_tag").and_then(Value::as_u64).unwrap_or(0) as u32;
        let path = item
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        tags.push(LastSessionTag {
            key: key.to_owned(),
            label,
            group_tag,
            path,
        });
    }
    let folders = value
        .get("folders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let rel_path = item
                .get("path")?
                .as_str()
                .map(str::trim)
                .filter(|path| !path.is_empty())?;
            let label = item
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .or_else(|| rel_path.rsplit(['/', '\\']).next())?;
            Some(LastSessionFolder {
                rel_path: PathBuf::from(rel_path),
                label: label.to_owned(),
            })
        })
        .collect::<Vec<_>>();
    let chimp_packages = value
        .get("chimp_packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|package| !package.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let active_chimp_package = value
        .get("active_chimp_package")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|package| chimp_packages.iter().any(|open| open == package))
        .map(str::to_owned);
    // Absent in every session written before the Bitmap Library existed, which
    // reads back as "it was not open" — the right answer for those files.
    let bitmap_library_open = value
        .get("bitmap_library")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let model_library_open = value
        .get("model_library")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // Keep source-only workspaces. The source path is meaningful session state
    // even when no tag window or project was open in that workspace.
    Some(LastSessionKit {
        source_kind,
        source_path,
        game,
        profile_id,
        project_path,
        has_project,
        browser_mode,
        browser_sort,
        tags,
        folders,
        chimp_packages,
        active_chimp_package,
        bitmap_library_open,
        model_library_open,
        was_active,
    })
}

/// Persist every kit's source and open tag/folder panes for the launch-time restore
/// prompt, along with which of them was focused. Written from the confirmed
/// app-exit path and again as the event loop tears down, so a quit that never
/// asks the window to close — macOS Cmd+Q — still records the session; a crash
/// leaves the previous one intact.
pub(in crate::app) fn save_last_session(session: &LastSessionState) -> Result<(), String> {
    let path = last_session_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create session folder: {error}"))?;
    }
    let text = serde_json::to_string_pretty(&session_value(session))
        .map_err(|error| format!("Could not encode session: {error}"))?;
    // Atomic, as the doc above promises: this runs after every autosave, so a
    // plain write left a window for a crash to truncate the session to nothing.
    write_text_atomic(&path, &text, "session")
}

/// Pure encode of a session document, split out from the file write so the
/// round trip through [`parse_last_session`] is covered by tests.
fn session_value(session: &LastSessionState) -> Value {
    let kits = session
        .kits
        .iter()
        .map(|kit| {
            let tags = kit
                .tags
                .iter()
                .map(|tag| {
                    json!({
                        "key": tag.key,
                        "label": tag.label,
                        "group_tag": tag.group_tag,
                        "path": tag.path.as_ref().map(|path| path.display().to_string()),
                    })
                })
                .collect::<Vec<_>>();
            let folders = kit
                .folders
                .iter()
                .map(|folder| {
                    json!({
                        "path": folder.rel_path.to_string_lossy().replace('\\', "/"),
                        "label": folder.label,
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "source": {
                    "kind": kit.source_kind.as_str(),
                    "path": kit.source_path.display().to_string(),
                    "game": kit.game,
                    "profile_id": kit.profile_id,
                    "project_path": kit.project_path.as_ref().map(|path| path.display().to_string()),
                    "has_project": kit.has_project,
                },
                "browser_mode": kit.browser_mode.map(browser_mode_str),
                "browser_sort": kit.browser_sort.map(browser_sort_str),
                "tags": tags,
                "folders": folders,
                "chimp_packages": kit.chimp_packages,
                "active_chimp_package": kit.active_chimp_package,
                "bitmap_library": kit.bitmap_library_open,
                "model_library": kit.model_library_open,
                // Which workspace the user was looking at. Written as a flag on
                // the kit rather than an index beside the list: the restore
                // prompt can drop kits, and an index would then point at
                // whichever one moved into that slot. Absent in sessions
                // written before this, which read back as "no kit was active"
                // and leave the restore picking as it used to.
                "active": kit.was_active,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "version": 6,
        "kits": kits,
    })
}

pub(in crate::app) fn clear_last_session() {
    let _ = fs::remove_file(last_session_path());
}

/// What a restored session still has to put back once the kit's source loads:
/// tags and folders, undo histories, Chimp packages, the libraries, and tags
/// named on the command line.
#[derive(Default)]
pub(in crate::app) struct RestorePlan {
    /// Tags staged by a session restore, drained once this kit's source
    /// finishes loading. Held per kit rather than in one shared slot so
    /// several kits can restore concurrently and finish in any order.
    pub(in crate::app) pending_restore_tags: Vec<LastSessionTag>,
    pub(in crate::app) pending_restore_folders: Vec<LastSessionFolder>,
    /// Undo/redo stacks a restored project brought back, by document key, held
    /// until the document they belong to exists. A restored tab is loaded
    /// asynchronously, so the history almost always arrives before the tag it
    /// applies to.
    pub(in crate::app) pending_history: HashMap<String, TagHistory>,
    /// Chimp packages staged by session restore until the Unreal container
    /// world has mounted. Kept separate from tag restoration so Tags remains
    /// the initial surface.
    pub(in crate::app) pending_restore_chimp_packages: Vec<String>,
    pub(in crate::app) pending_restore_active_chimp_package: Option<String>,
    /// Whether session restore should reopen the Bitmap Library here, staged
    /// the same way and for the same reason as the Chimp packages: the tab can
    /// only be opened once this kit's source has finished loading.
    pub(in crate::app) pending_restore_bitmap_library: bool,
    /// Whether session restore should reopen the Model Library here, likewise.
    pub(in crate::app) pending_restore_model_library: bool,
    /// Loose tag paths requested on the command line, drained after this kit's
    /// editing-kit source finishes loading.
    pub(in crate::app) pending_launch_tags: Option<Vec<PathBuf>>,
}

/// The prompt asking which of the last session's windows to reopen.
/// Remembering the answer changes the preferences; reopening is
/// [`AppAction::RestoreSession`].
impl Dialog for LastOpenedWindowsPrompt {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        match render_last_opened_windows_prompt(cx.egui, Some(self)) {
            LastOpenedWindowsAction::None => true,
            LastOpenedWindowsAction::Cancel { remember } => {
                if remember {
                    cx.edit_prefs(|prefs| prefs.session_restore = SessionRestore::Never);
                }
                false
            }
            LastOpenedWindowsAction::Restore { kits, remember } => {
                if remember {
                    cx.edit_prefs(|prefs| prefs.session_restore = SessionRestore::Always);
                }
                cx.send(AppAction::RestoreSession(kits));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_kits::compat_json;
    use std::path::PathBuf;
    use std::path::Path;

    /// Every completed load makes its own kit active, so a restore's focus can
    /// only be honoured once none are left in flight — otherwise whichever
    /// source finished last would win, and a loose folder racing a container
    /// set has no stable winner. Quitting with Halo 3 focused came back to
    /// Campaign Evolved this way.
    #[test]
    fn the_focus_waits_for_every_restored_kit_to_land() {
        let (halo3, evolved) = (KitId(1), KitId(2));
        let mut restoring = HashSet::from([halo3, evolved]);
        let mut active = Some(halo3);

        assert_eq!(
            focus_after_restore(&mut restoring, &mut active, evolved),
            None,
            "one kit is still loading, so the focus is not settled yet"
        );
        assert_eq!(
            focus_after_restore(&mut restoring, &mut active, halo3),
            Some(halo3),
            "the last landing hands the focus to the kit the session named"
        );
        assert_eq!(active, None, "and it is honoured only once");
    }

    /// Load order must not change the answer.
    #[test]
    fn the_focused_kit_wins_whichever_lands_first() {
        let (halo3, evolved) = (KitId(1), KitId(2));
        for order in [[halo3, evolved], [evolved, halo3]] {
            let mut restoring = HashSet::from([halo3, evolved]);
            let mut active = Some(halo3);
            let settled: Vec<_> = order
                .into_iter()
                .filter_map(|kit| focus_after_restore(&mut restoring, &mut active, kit))
                .collect();
            assert_eq!(
                settled,
                [halo3],
                "landing order {order:?} changed the focus"
            );
        }
    }

    /// A session written before the focused kit was recorded names none, and a
    /// load that was never part of a restore must not disturb anything.
    #[test]
    fn nothing_is_claimed_without_a_named_kit_or_a_restore() {
        let halo3 = KitId(1);
        let mut restoring = HashSet::from([halo3]);
        let mut active = None;
        assert_eq!(
            focus_after_restore(&mut restoring, &mut active, halo3),
            None
        );

        let mut restoring = HashSet::new();
        let mut active = Some(halo3);
        assert_eq!(
            focus_after_restore(&mut restoring, &mut active, KitId(9)),
            None,
            "an ordinary load is not a restore landing"
        );
        assert_eq!(active, Some(halo3), "and leaves the pending focus alone");
    }

    #[test]
    fn restored_loose_tag_uses_the_current_sources_key() {
        let root = std::env::temp_dir().join(format!("baboon-session-key-{}", std::process::id()));
        let path = root.join("objects").join("characters").join("brute.model");
        std::fs::create_dir_all(path.parent().expect("tag has parent")).expect("create tag path");
        std::fs::write(&path, b"tag").expect("create tag");
        let canonical = std::fs::canonicalize(&path).expect("canonical tag path");
        let entry = crate::core::source::TagEntry {
            key: file_entry_key(&canonical),
            display_path: "objects/characters/brute.model".to_owned(),
            group_tag: u32::from_be_bytes(*b"hlmt"),
            group_name: Some("model".to_owned()),
            location: crate::core::source::TagEntryLocation::LooseFile(canonical.clone()),
        };

        assert_eq!(
            super::loose_entry_key_for_canonical_path(std::iter::once(&entry), &canonical),
            Some(entry.key.clone())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn last_opened_workspace_heading_prefers_the_named_project() {
        let source = PathBuf::from("Games").join("Halo Infinite");
        let project = PathBuf::from("Baboon Projects").join("Campaign Overhaul.baboon");

        assert_eq!(
            super::last_opened_workspace_heading(
                None,
                Some("halo_infinite"),
                &source,
                Some(&project)
            ),
            (
                "Campaign Overhaul".to_owned(),
                Some(project.display().to_string())
            )
        );
    }

    #[test]
    fn last_opened_workspace_heading_keeps_the_source_fallback() {
        let source = Path::new(r"C:\Editing Kits\Custom Kit");

        assert_eq!(
            super::last_opened_workspace_heading(None, None, source, None),
            (source.display().to_string(), None)
        );
    }

    #[test]
    fn last_opened_workspace_heading_prefers_the_custom_editing_kit_profile() {
        let source = Path::new(r"C:\Editing Kits\H2EK");

        assert_eq!(
            super::last_opened_workspace_heading(
                Some(("Halo 2 Rebalance", source)),
                Some("halo2_mcc"),
                source,
                None
            ),
            (
                "Halo 2 Rebalance".to_owned(),
                Some(source.display().to_string())
            )
        );
    }

    fn restore_test_prompt(folder_count: usize) -> LastOpenedWindowsPrompt {
        LastOpenedWindowsPrompt::from_session(
            LastSessionState {
                kits: vec![LastSessionKit {
                    source_kind: LastSessionSourceKind::LooseFolder,
                    source_path: std::env::temp_dir(),
                    game: None,
                    profile_id: None,
                    project_path: None,
                    has_project: false,
                    browser_mode: None,
                    browser_sort: None,
                    tags: Vec::new(),
                    folders: (0..folder_count)
                        .map(|index| LastSessionFolder {
                            rel_path: PathBuf::from(format!(
                                "objects/characters/brute/folder{index}"
                            )),
                            label: format!("folder{index}"),
                        })
                        .collect(),
                    chimp_packages: Vec::new(),
                    active_chimp_package: None,
                    bitmap_library_open: false,
                    model_library_open: false,
                    was_active: false,
                }],
            },
            &[],
        )
        .unwrap()
    }

    #[test]
    fn restore_dialog_hugs_contents_and_stays_stable_during_width_resizing() {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        ctx.set_global_style(foundation_style());
        let mut prompt = restore_test_prompt(20);
        let frame = |prompt: &mut LastOpenedWindowsPrompt, events| {
            let _ = crate::app::run_ui_test(&ctx, 
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1200.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    super::render_last_opened_windows_prompt(ui.ctx(), Some(prompt));
                },
            );
            ctx.memory(|memory| {
                memory
                    .area_rect(egui::Id::new("last_opened_windows"))
                    .unwrap()
            })
        };
        for _ in 0..5 {
            frame(&mut prompt, Vec::new());
        }
        let initial = frame(&mut prompt, Vec::new());
        assert!(
            initial.height() < 700.0,
            "long lists must be capped and scroll"
        );
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        for direction in [1.0, -1.0] {
            let before = frame(&mut prompt, Vec::new());
            let edge = egui::pos2(before.right() - 1.0, before.center().y);
            frame(
                &mut prompt,
                vec![egui::Event::PointerMoved(edge), pointer(edge, true)],
            );
            for step in 1..=40 {
                let pos = edge + egui::vec2(direction * step as f32 * 2.0, 0.0);
                let during = frame(&mut prompt, vec![egui::Event::PointerMoved(pos)]);
                assert!(
                    (during.height() - initial.height()).abs() < 1.0,
                    "width drag changed height from {} to {}",
                    initial.height(),
                    during.height()
                );
            }
            let pos = edge + egui::vec2(direction * 80.0, 0.0);
            frame(&mut prompt, vec![pointer(pos, false)]);
            let after = frame(&mut prompt, Vec::new());
            assert!(
                (after.width() - before.width()).abs() > 20.0,
                "the test must actually change the width"
            );
            for _ in 0..10 {
                assert!((frame(&mut prompt, Vec::new()).height() - initial.height()).abs() < 1.0);
            }
        }
        prompt.kits[0].folder_entries.truncate(2);
        for _ in 0..5 {
            frame(&mut prompt, Vec::new());
        }
        let short = frame(&mut prompt, Vec::new());
        assert!(
            short.height() < 270.0,
            "short lists must hug all rows: {short:?}"
        );
        assert!(initial.height() - short.height() > 300.0);
    }

    #[test]
    fn restore_footer_fill_reaches_window_edges_without_content_clipping() {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        ctx.set_global_style(foundation_style());
        ctx.set_visuals(foundation_visuals());
        let mut prompt = restore_test_prompt(2);
        let mut output = None;
        for _ in 0..6 {
            output = Some(crate::app::run_ui_test(&ctx, egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO, Vec2::new(1200.0, 800.0),
                )),
                ..Default::default()
            }, |ui| {
                super::render_last_opened_windows_prompt(ui.ctx(), Some(&mut prompt));
            }));
        }
        let window = ctx.memory(|memory| {
            memory.area_rect(egui::Id::new("last_opened_windows")).unwrap()
        });
        let output = output.unwrap();
        let (clip, footer) = output.shapes.iter().find_map(|clipped| {
            match &clipped.shape {
                egui::Shape::Rect(rect) if rect.corner_radius.nw == 0
                    && rect.corner_radius.ne == 0 && rect.corner_radius.sw > 0
                    && rect.corner_radius.se > 0 => Some((clipped.clip_rect, rect)),
                _ => None,
            }
        }).expect("footer background with rounded bottom corners");
        assert!((footer.rect.left() - window.left()).abs() < 1.0);
        assert!((footer.rect.right() - window.right()).abs() < 1.0);
        assert!((footer.rect.bottom() - window.bottom()).abs() < 1.0);
        assert!(clip.contains_rect(footer.rect), "the content clip must not inset the footer fill");
    }

    #[test]
    fn restore_workspace_heading_centers_both_lines_with_padding() {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        ctx.set_global_style(foundation_style());
        let mut prompt = restore_test_prompt(0);
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.scope(|ui| {
                    let (row, text) = super::restore_workspace_row(ui, &mut prompt.kits[0]);
                    assert!((row.center().y - text.center().y).abs() < 0.01);
                    assert!(text.top() - row.top() >= 6.0);
                    assert!(row.bottom() - text.bottom() >= 6.0);
                    assert!(
                        ui.min_rect().bottom() <= row.bottom() + 1.0,
                        "heading widgets must not extend the allocated row"
                    );
                });
            });
        });
    }

    /// A version-1 file predates multiple kits. It must still restore, as one
    /// kit — silently dropping it would lose the user's open tags on upgrade.
    #[test]
    fn version_1_sessions_upgrade_to_a_single_kit() {
        let value = serde_json::json!({
            "version": 1,
            "source": { "kind": "loose_folder", "path": "/tags", "game": "halo3_mcc" },
            "tags": [{ "key": "file:/tags/a.weapon", "label": "a", "group_tag": 1, "path": null }],
        });
        let session = parse_last_session(&value).expect("v1 session parses");
        assert_eq!(session.kits.len(), 1);
        assert_eq!(session.kits[0].game.as_deref(), Some("halo3_mcc"));
        assert_eq!(session.kits[0].tags.len(), 1);
    }

    #[test]
    fn version_3_sessions_restore_every_kit() {
        let value = serde_json::json!({
            "version": 3,
            "kits": [
                {
                    "source": { "kind": "loose_folder", "path": "/h3", "game": "halo3_mcc" },
                    "tags": [{ "key": "file:/h3/a.weapon", "label": "a", "group_tag": 1 }],
                },
                {
                    "source": { "kind": "loose_folder", "path": "/reach", "game": "haloreach_mcc" },
                    "tags": [{ "key": "file:/reach/b.weapon", "label": "b", "group_tag": 1 }],
                },
            ],
        });
        let session = parse_last_session(&value).expect("v3 session parses");
        assert_eq!(session.kits.len(), 2);
        assert_eq!(session.kits[1].game.as_deref(), Some("haloreach_mcc"));
        assert_eq!(session.kits[1].tags[0].key, "file:/reach/b.weapon");
    }

    #[test]
    fn custom_profile_identity_survives_session_round_trip() {
        let mut custom = kit("/custom-reach", Some(BrowserMode::Folders));
        custom.game = Some("haloreach_mcc".to_owned());
        custom.profile_id = Some("11111111-1111-4111-8111-111111111111".to_owned());
        let restored = parse_last_session(&session_value(&LastSessionState { kits: vec![custom] }))
            .expect("custom profile session parses");
        assert_eq!(
            restored.kits[0].profile_id.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(restored.kits[0].game.as_deref(), Some("haloreach_mcc"));
    }

    /// A source-only kit remains part of the session even without open tags,
    /// while a session left with no kits at all is still no session.
    #[test]
    fn source_only_kits_are_retained_for_restore() {
        let value = serde_json::json!({
            "version": 3,
            "kits": [
                { "source": { "kind": "loose_folder", "path": "/h3" }, "tags": [] },
                {
                    "source": { "kind": "loose_folder", "path": "/reach" },
                    "tags": [{ "key": "file:/reach/b.weapon", "label": "b", "group_tag": 1 }],
                },
            ],
        });
        let session = parse_last_session(&value).expect("session parses");
        assert_eq!(session.kits.len(), 2);
        assert_eq!(session.kits[0].source_path, PathBuf::from("/h3"));
        assert_eq!(session.kits[0].tags.len(), 0);
        assert_eq!(session.kits[1].source_path, PathBuf::from("/reach"));

        let empty = serde_json::json!({ "version": 3, "kits": [] });
        assert!(parse_last_session(&empty).is_none());
    }

    #[test]
    fn two_source_only_workspaces_keep_their_order_and_identity() {
        let mut campaign = kit("/campaign-evolved", None);
        campaign.tags.clear();
        let mut reach = kit("/reach", None);
        reach.tags.clear();

        let restored = parse_last_session(&session_value(&LastSessionState {
            kits: vec![campaign, reach],
        }))
        .expect("two source-only workspaces parse");

        assert_eq!(
            restored
                .kits
                .iter()
                .map(|kit| kit.source_path.clone())
                .collect::<Vec<_>>(),
            [PathBuf::from("/campaign-evolved"), PathBuf::from("/reach")]
        );
    }

    #[test]
    fn unknown_versions_are_ignored() {
        let value = serde_json::json!({ "version": 99, "kits": [] });
        assert!(parse_last_session(&value).is_none());
    }

    fn kit(path: &str, mode: Option<BrowserMode>) -> LastSessionKit {
        LastSessionKit {
            source_kind: LastSessionSourceKind::LooseFolder,
            source_path: PathBuf::from(path),
            game: None,
            profile_id: None,
            project_path: None,
            has_project: false,
            browser_mode: mode,
            browser_sort: Some(BrowserSort::Name),
            tags: vec![LastSessionTag {
                key: format!("file:{path}/a.weapon"),
                label: "a".to_owned(),
                group_tag: 1,
                path: None,
            }],
            folders: Vec::new(),
            chimp_packages: Vec::new(),
            active_chimp_package: None,
            bitmap_library_open: false,
            model_library_open: false,
            was_active: false,
        }
    }

    #[test]
    fn unchecked_workspaces_are_excluded_without_losing_their_pane_choices() {
        let root = std::env::temp_dir().display().to_string();
        let mut prompt = LastOpenedWindowsPrompt::from_session(
            LastSessionState {
                kits: vec![kit(&root, None), kit(&root, None)],
            },
            &[],
        )
        .unwrap();
        prompt.kits[0].bitmap_library_open = true;
        prompt.kits[0].checked = false;
        assert_eq!(prompt.checked_kits().len(), 1);
        assert!(prompt.kits[0].entries[0].checked);
        prompt.kits[1].checked = false;
        assert!(prompt.checked_kits().is_empty());
        assert!(!prompt.has_reopenable_kits());
        prompt.kits[0].checked = true;
        assert_eq!(prompt.checked_kits()[0].tags.len(), 1);
        assert!(prompt.checked_kits()[0].bitmap_library_open);
        // A selected workspace can still reopen with all its panes unchecked.
        prompt.kits[0].entries[0].checked = false;
        assert_eq!(prompt.checked_kits().len(), 1);
        assert!(prompt.checked_kits()[0].tags.is_empty());
        prompt.kits[0].source_available = false;
        assert!(prompt.checked_kits().is_empty());
    }

    #[test]
    fn folder_windows_round_trip_and_can_be_unchecked_for_restore() {
        let source_path = std::env::temp_dir();
        let mut saved = kit(
            &source_path.display().to_string(),
            Some(BrowserMode::Folders),
        );
        saved.tags.clear();
        saved.folders.push(LastSessionFolder {
            rel_path: PathBuf::from(r"objects\characters\brute"),
            label: "brute".to_owned(),
        });

        let value = session_value(&LastSessionState { kits: vec![saved] });
        assert_eq!(value["version"], 6);
        assert_eq!(
            value["kits"][0]["folders"][0]["path"],
            "objects/characters/brute"
        );

        let restored = parse_last_session(&value).expect("folder session parses");
        assert_eq!(restored.kits[0].folders.len(), 1);
        assert_eq!(restored.kits[0].folders[0].label, "brute");

        let mut prompt =
            LastOpenedWindowsPrompt::from_session(restored, &[]).expect("restore prompt exists");
        assert!(prompt.kits[0].folder_entries[0].checked);
        assert_eq!(prompt.checked_kits()[0].folders.len(), 1);
        prompt.kits[0].folder_entries[0].checked = false;
        assert!(prompt.checked_kits()[0].folders.is_empty());
    }

    #[test]
    fn restore_prompt_resolves_the_current_custom_project_name_and_root() {
        let root = std::env::temp_dir();
        let mut saved = kit(&root.display().to_string(), Some(BrowserMode::Folders));
        saved.profile_id = Some("custom-h2-project".to_owned());
        let profile = CustomEditingKitProfile {
            read_only: false,
            git_tracked: false,
            id: "custom-h2-project".to_owned(),
            name: "Halo 2 Rebalance".to_owned(),
            game: "halo2_mcc".to_owned(),
            root: root.clone(),
            icon: None,
            tags_folder: None,
            data_folder: None,
        };

        let prompt = LastOpenedWindowsPrompt::from_session(
            LastSessionState { kits: vec![saved] },
            &[profile],
        )
        .expect("restore prompt exists");

        assert_eq!(
            prompt.kits[0].profile_name.as_deref(),
            Some("Halo 2 Rebalance")
        );
        assert_eq!(prompt.kits[0].profile_root.as_deref(), Some(root.as_path()));
    }

    #[test]
    fn sessions_from_before_folder_windows_restore_with_none() {
        let value = serde_json::json!({
            "version": 5,
            "kits": [{
                "source": { "kind": "loose_folder", "path": "/h3" },
                "tags": [],
            }],
        });
        let restored = parse_last_session(&value).expect("version 5 session parses");
        assert!(restored.kits[0].folders.is_empty());
    }

    #[test]
    fn chimp_packages_round_trip_and_keep_an_otherwise_empty_kit() {
        let session = LastSessionState {
            kits: vec![LastSessionKit {
                source_kind: LastSessionSourceKind::IoStoreContainerSet,
                source_path: PathBuf::from("C:/game"),
                game: Some("campaignevolved".to_owned()),
                profile_id: None,
                project_path: None,
                has_project: false,
                browser_mode: Some(BrowserMode::Folders),
                browser_sort: Some(BrowserSort::Name),
                tags: Vec::new(),
                folders: Vec::new(),
                chimp_packages: vec!["/Game/Vehicles/Warthog".to_owned()],
                active_chimp_package: Some("/Game/Vehicles/Warthog".to_owned()),
                bitmap_library_open: false,
                model_library_open: false,
                was_active: true,
            }],
        };
        let value = session_value(&session);
        assert_eq!(value["version"], 6);
        let restored = parse_last_session(&value).expect("session parses");
        assert_eq!(restored.kits.len(), 1);
        assert_eq!(restored.kits[0].chimp_packages, ["/Game/Vehicles/Warthog"]);
        assert_eq!(
            restored.kits[0].active_chimp_package.as_deref(),
            Some("/Game/Vehicles/Warthog")
        );
    }

    /// The Bitmap Library comes back open, and a workspace that had *only* the
    /// library open still survives the round trip.
    ///
    /// It cannot ride in `tags`: its pane key resolves to no entry, so the tag
    /// loop drops it on the way out. A kit with no tags and no Chimp packages is
    /// the case that would silently lose it.
    #[test]
    fn the_bitmap_library_round_trips_on_a_kit_with_nothing_else_open() {
        let mut only_library = kit("C:/halo3", Some(BrowserMode::Folders));
        only_library.tags = Vec::new();
        only_library.bitmap_library_open = true;
        let session = LastSessionState {
            kits: vec![only_library, kit("C:/reach", Some(BrowserMode::Groups))],
        };

        let restored = parse_last_session(&session_value(&session)).expect("session parses");

        assert_eq!(
            restored.kits.len(),
            2,
            "an empty workspace is still a workspace"
        );
        assert!(restored.kits[0].bitmap_library_open);
        assert!(
            !restored.kits[1].bitmap_library_open,
            "the flag belongs to its own kit, not to the session"
        );
    }

    /// The Model Library rides the same flag mechanism as the Bitmap Library,
    /// and each library's flag comes back independently.
    #[test]
    fn the_model_library_round_trips_independently_of_the_bitmap_library() {
        let mut only_models = kit("C:/halo3", Some(BrowserMode::Folders));
        only_models.tags = Vec::new();
        only_models.model_library_open = true;
        let session = LastSessionState {
            kits: vec![only_models],
        };

        let restored = parse_last_session(&session_value(&session)).expect("session parses");

        assert!(restored.kits[0].model_library_open);
        assert!(
            !restored.kits[0].bitmap_library_open,
            "one library's flag must not drag the other's along"
        );
    }

    /// Sessions written before the Bitmap Library existed carry no flag, and
    /// must read back as "it was not open" rather than failing to parse.
    #[test]
    fn a_session_without_the_bitmap_library_flag_still_loads() {
        let mut value = session_value(&LastSessionState {
            kits: vec![kit("C:/halo3", Some(BrowserMode::Folders))],
        });
        // Exactly what a version-4 file looks like: the field never written.
        value["version"] = serde_json::json!(4);
        value["kits"][0]
            .as_object_mut()
            .unwrap()
            .remove("bitmap_library");

        let restored = parse_last_session(&value).expect("an older session still parses");
        assert!(!restored.kits[0].bitmap_library_open);
        assert_eq!(restored.kits[0].tags.len(), 1, "its tags still come back");
    }

    /// Which workspace the user was looking at survives the round trip, and is
    /// carried on the kit rather than as an index beside the list — the restore
    /// prompt can drop kits, and an index would then name whichever one moved
    /// into that slot.
    #[test]
    fn the_focused_kit_is_remembered_and_travels_with_its_own_kit() {
        let mut halo3 = kit("C:/halo3", Some(BrowserMode::Folders));
        let mut evolved = kit("C:/evolved", Some(BrowserMode::Groups));
        evolved.source_kind = LastSessionSourceKind::IoStoreContainerSet;
        halo3.was_active = true;
        evolved.was_active = false;
        let session = LastSessionState {
            kits: vec![evolved, halo3],
        };

        let value = session_value(&session);
        let restored = parse_last_session(&value).expect("session parses");
        assert_eq!(restored.kits.len(), 2);
        assert!(
            !restored.kits[0].was_active,
            "the container kit was not focused"
        );
        assert!(restored.kits[1].was_active, "the Halo 3 kit was focused");
        // It is the kit that is marked, not a position: the flag follows its
        // own workspace when the list is filtered.
        let kept = restored
            .kits
            .into_iter()
            .filter(|kit| kit.source_kind == LastSessionSourceKind::LooseFolder)
            .collect::<Vec<_>>();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].was_active);
    }

    /// A session written before the focused workspace was recorded has no
    /// `active` on any kit, and must still load rather than being rejected.
    #[test]
    fn a_session_without_a_focused_kit_still_loads() {
        let value = serde_json::json!({
            "version": 4,
            "kits": [{
                "source": { "kind": "loose_folder", "path": "C:/halo3" },
                "tags": [],
            }],
        });
        let restored = parse_last_session(&value).expect("session parses");
        assert_eq!(restored.kits.len(), 1);
        assert!(!restored.kits[0].was_active);
    }

    /// Each workspace keeps its own browser view, so a session holding a kit
    /// in Folders and one in Groups must bring both back as they were — not
    /// collapse them onto one setting.
    #[test]
    fn each_kit_restores_its_own_browser_view() {
        let session = LastSessionState {
            kits: vec![
                kit("/evolved", Some(BrowserMode::Folders)),
                kit("/reach", Some(BrowserMode::Groups)),
            ],
        };
        let restored = parse_last_session(&session_value(&session)).expect("round trip");
        assert_eq!(restored.kits[0].browser_mode, Some(BrowserMode::Folders));
        assert_eq!(restored.kits[1].browser_mode, Some(BrowserMode::Groups));
        assert_eq!(restored.kits[0].browser_sort, Some(BrowserSort::Name));
    }

    /// Sessions written before the view was saved carry none, which has to
    /// stay distinguishable from a saved Folders so the restore can fall back
    /// to the user's default instead of overriding it.
    #[test]
    fn sessions_without_a_saved_view_restore_none() {
        let value = serde_json::json!({
            "version": 3,
            "kits": [{
                "source": { "kind": "loose_folder", "path": "/h3" },
                "tags": [{ "key": "file:/h3/a.weapon", "label": "a", "group_tag": 1 }],
            }],
        });
        let session = parse_last_session(&value).expect("session parses");
        assert_eq!(session.kits[0].browser_mode, None);
        assert_eq!(session.kits[0].browser_sort, None);
    }

    /// A kit whose session is its project has no tags to save, so dropping the
    /// project path on write left nothing to restore it from.
    #[test]
    fn a_projects_path_survives_the_round_trip() {
        let mut project_kit = kit("/evolved", None);
        project_kit.project_path = Some(PathBuf::from("/evolved/work.baboon"));
        project_kit.has_project = true;
        project_kit.tags.clear();
        let session = LastSessionState {
            kits: vec![project_kit],
        };
        let restored = parse_last_session(&session_value(&session)).expect("round trip");
        assert_eq!(
            restored.kits[0].project_path,
            Some(PathBuf::from("/evolved/work.baboon"))
        );
        assert!(restored.kits[0].has_project);
    }

    /// A workspace whose only content is its stash records no project path — its
    /// edits live in the recovery file, which is found from the source root — so
    /// "carries a project" cannot be inferred from that path any more. Reading it
    /// that way would drop such a kit from the session entirely and lose the
    /// stash with it.
    #[test]
    fn a_stash_only_kit_survives_the_round_trip() {
        let mut stash_kit = kit("/evolved", None);
        stash_kit.has_project = true;
        stash_kit.tags.clear();
        let session = LastSessionState {
            kits: vec![stash_kit],
        };
        let restored = parse_last_session(&session_value(&session)).expect("round trip");
        assert_eq!(restored.kits[0].project_path, None);
        assert!(restored.kits[0].has_project);
    }

    /// Sessions written before the recovery file and the project file were
    /// separate recorded the recovery path as `project_path`, and set it for every
    /// workspace that had a project at all. That is what `has_project` now means,
    /// so its absence reads straight off the old field.
    #[test]
    fn a_legacy_sessions_recovery_path_still_restores_the_workspace() {
        let value = serde_json::json!({
            "version": 3,
            "kits": [{
                "source": {
                    "kind": "iostore_container_set",
                    "path": "/evolved",
                    "project_path": "/data/campaign_evolved_recovery-abc123.baboon",
                },
                "tags": [],
            }],
        });
        let session = parse_last_session(&value).expect("session parses");
        assert!(session.kits[0].has_project, "the kit is still restored");
    }

    #[test]
    fn last_session_v1_remains_compatible() {
        let session = parse_last_session(
            &serde_json::from_str::<Value>(
                r#"{
                "version": 1,
                "source": {
                    "kind": "loose_folder",
                    "path": "C:/tags",
                    "game": "haloreach"
                },
                "tags": [{
                    "key": "objects/test.weapon",
                    "label": "test",
                    "group_tag": 2003132784
                }]
            }"#,
            )
            .expect("valid json"),
        )
        .expect("version 1 session");
        // A single-source file loads as one kit.
        assert_eq!(session.kits.len(), 1);
        assert_eq!(
            session.kits[0].source_kind,
            LastSessionSourceKind::LooseFolder
        );
        assert_eq!(session.kits[0].project_path, None);
        assert_eq!(session.kits[0].tags.len(), 1);
    }

    #[test]
    fn project_pointer_survives_with_no_open_tabs() {
        let session = parse_last_session(
            &serde_json::from_str::<Value>(
                r#"{
                "version": 2,
                "source": {
                    "kind": "iostore_container_set",
                    "path": "C:/CampaignEvolved/Paks",
                    "game": "haloce_evolved",
                    "project_path": "C:/mods/recovery.baboon"
                },
                "tags": []
            }"#,
            )
            .expect("valid json"),
        )
        .expect("project-only session");
        assert_eq!(session.kits.len(), 1);
        assert_eq!(
            session.kits[0].source_kind,
            LastSessionSourceKind::IoStoreContainerSet
        );
        assert_eq!(
            session.kits[0].project_path,
            Some(PathBuf::from("C:/mods/recovery.baboon"))
        );
        assert!(session.kits[0].tags.is_empty());
    }

    // Every saved format Baboon reads, fed through the real readers from the
    // synthetic samples in `testdata/compat` (see its README; regenerate with
    // `gen_samples.py`). Old files must keep loading, files a newer build wrote
    // must not be destroyed by this one, and the cases a reader refuses are
    // pinned beside the ones it accepts, so a reader that accepted everything
    // would fail here too.

    #[test]
    fn compat_last_session_versions() {
        let v1 = parse_last_session(&compat_json("last_session/v1.json")).expect("v1");
        assert_eq!(v1.kits.len(), 1);
        assert!(v1.kits[0].tags[0].key.starts_with("file:C:\\"));
        assert!(v1.kits[0].tags[0].path.is_some());

        let v2 = parse_last_session(&compat_json("last_session/v2_single_source.json")).expect("v2 single");
        assert_eq!(
            v2.kits[0].source_kind,
            LastSessionSourceKind::IoStoreContainerSet
        );
        assert!(
            v2.kits[0].has_project,
            "has_project defaults to project_path.is_some()"
        );

        // The v2 shape 8f30d04 wrote ({version: 2, kits: [...]}) is not accepted.
        assert!(parse_last_session(&compat_json("last_session/v2_kits_8f30d04.json")).is_none());

        let v3 = parse_last_session(&compat_json("last_session/v3.json")).expect("v3");
        assert_eq!(v3.kits.len(), 3);
        assert_eq!(v3.kits[1].tags[1].key, "cache:rm:shaders\\default");
        assert_eq!(v3.kits[1].tags[1].group_tag, u32::from_be_bytes(*b"rm  "));
        assert!(v3.kits[2].has_project);

        let v4 = parse_last_session(&compat_json("last_session/v4.json")).expect("v4");
        assert!(v4.kits[0].was_active);
        assert_eq!(v4.kits[0].chimp_packages.len(), 2);
        assert_eq!(
            v4.kits[0].active_chimp_package.as_deref(),
            Some("/Game/Maps/a30/a30_Persistent")
        );

        let v5 = parse_last_session(&compat_json("last_session/v5.json")).expect("v5");
        assert!(v5.kits[0].bitmap_library_open && !v5.kits[0].model_library_open);

        let v6 = parse_last_session(&compat_json("last_session/v6.json")).expect("v6");
        assert_eq!(v6.kits.len(), 11);
        let keys: Vec<&str> = v6
            .kits
            .iter()
            .flat_map(|kit| kit.tags.iter().map(|tag| tag.key.as_str()))
            .collect();
        let tag_keys = compat_json("tag_keys.json");
        for (kind, expected) in tag_keys.as_object().unwrap() {
            assert!(
                keys.contains(&expected.as_str().unwrap()),
                "{kind} kept verbatim"
            );
        }
        assert_eq!(
            v6.kits[10].game.as_deref(),
            Some("halo5_mcc"),
            "an unknown game id passes through unvalidated"
        );
        assert_eq!(
            v6.kits[0].folders[1].rel_path,
            PathBuf::from("levels/solo/010_jungle")
        );

        // Written again and read back, every key survives and the version is current.
        let again = parse_last_session(&session_value(&v6)).expect("round trip");
        let keys_again: Vec<String> = again
            .kits
            .iter()
            .flat_map(|kit| kit.tags.iter().map(|tag| tag.key.clone()))
            .collect();
        assert_eq!(keys, keys_again);
        assert_eq!(session_value(&again)["version"], 6);

        assert!(parse_last_session(&compat_json("last_session/v99_unknown_version.json")).is_none());
    }
}
