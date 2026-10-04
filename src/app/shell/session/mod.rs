//! The session: what is open is saved on exit and offered back on the next
//! start through the Last Opened Windows prompt.

use super::*;

pub(in crate::app) mod state;
pub(in crate::app) use state::*;
use crate::app::kits::loading::loose_entry_key_for_canonical_path;

impl Baboon {
    /// Snapshot every kit's source and open tag/folder panes for the restore prompt.
    pub(in crate::app) fn current_session_state(&self) -> Option<LastSessionState> {
        let kits = (0..self.kits.len())
            .filter_map(|index| self.session_kit_state(index))
            .collect::<Vec<_>>();
        (!kits.is_empty()).then_some(LastSessionState { kits })
    }

    pub(in crate::app) fn session_kit_state(&self, kit_index: usize) -> Option<LastSessionKit> {
        let kit = &self.kits[kit_index];
        let was_active = kit_index == self.active;
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
            .filter_map(|key| kit.folder_browsers.get(&key))
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
            browser_mode: Some(kit.browser_mode),
            browser_sort: Some(kit.browser_sort),
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
                            self.prefs
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
            let restoring = self.kits[self.active].id;
            self.shell.restoring_kits.insert(restoring);
            if was_active {
                self.shell.restored_active_kit = Some(restoring);
            }
            self.kits[self.active].restore.pending_restore_tags = tags;
            self.kits[self.active].restore.pending_restore_folders = folders;
            self.kits[self.active].restore.pending_restore_chimp_packages = chimp_packages;
            self.kits[self.active].restore.pending_restore_bitmap_library = bitmap_library_open;
            self.kits[self.active].restore.pending_restore_model_library = model_library_open;
            self.kits[self.active].restore.pending_restore_active_chimp_package = active_chimp_package;
            // Its browser view is staged the same way: `install_loaded_source`
            // carries it across the load rather than resetting it, so each
            // workspace comes back in the view it was left in.
            if let Some(mode) = browser_mode {
                self.kits[self.active].browser_mode = mode;
            }
            if let Some(sort) = browser_sort {
                self.kits[self.active].browser_sort = sort;
            }
            // The project file it had open is queued the same way, and is
            // attached as this workspace's save target once the source has
            // mounted. The edits themselves come back from the recovery file.
            if let Some(project_path) = project_path {
                let restoring = self.active;
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
        if let Some(index) = self.kit_index(active) {
            self.active = index;
        }
    }

    /// Reopen the panes staged for the kit that just finished loading.
    pub(in crate::app) fn finish_pending_session_restore(&mut self, ctx: egui::Context) {
        // Ahead of the early return below: a workspace whose only open tab was
        // the Bitmap Library has no tags staged, and would otherwise come back
        // without it.
        if std::mem::take(&mut self.kits[self.active].restore.pending_restore_bitmap_library) {
            self.open_bitmap_library();
        }
        if std::mem::take(&mut self.kits[self.active].restore.pending_restore_model_library) {
            self.open_model_library();
        }
        let restore_folders = std::mem::take(&mut self.kits[self.active].restore.pending_restore_folders);
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
        let restore = std::mem::take(&mut self.kits[self.active].restore.pending_restore_tags);
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
            self.status = if missing > 0 {
                format!("Restored {opened} window(s); skipped {missing} missing item(s)")
            } else {
                format!("Restored {opened} window(s)")
            };
        } else if missing > 0 {
            self.status = "No saved windows could be restored".to_owned();
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
        if let Some(entry) = self.entry_for_key(&tag.key) {
            return Some(entry.key.clone());
        }
        let path = tag.path.as_ref()?;
        if !path.is_file() {
            return None;
        }
        let source = self.source()?;
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
        let folder_seeds = self.kits[self.active].folder_seeds();
        if let Some(source) = self.source_mut() {
            if tag.key != current_key {
                source.remove_entry(&tag.key, &folder_seeds);
            }
            source.upsert_entry(entry, &folder_seeds);
        }
        self.kits[self.active].generation = self.kits[self.active].generation.wrapping_add(1);
        Some(current_key)
    }

    pub(in crate::app) fn handle_last_opened_windows_prompt(&mut self, ctx: &egui::Context) {
        let action = render_last_opened_windows_prompt(ctx, self.shell.last_opened_windows.as_mut());
        match action {
            LastOpenedWindowsAction::None => {}
            LastOpenedWindowsAction::OpenSettings => {
                self.shell.last_opened_windows = None;
                self.shell.settings_open = true;
            }
            LastOpenedWindowsAction::Cancel { remember } => {
                if remember {
                    self.prefs.session_restore = SessionRestore::Never;
                }
                self.shell.last_opened_windows = None;
            }
            LastOpenedWindowsAction::Restore { kits, remember } => {
                if remember {
                    self.prefs.session_restore = SessionRestore::Always;
                }
                self.shell.last_opened_windows = None;
                self.begin_last_session_restore(kits, ctx.clone());
            }
        }
    }
}

pub(in crate::app) enum LastOpenedWindowsAction {
    None,
    OpenSettings,
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

pub(in crate::app) fn render_last_opened_windows_prompt(
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
        .default_width(window_width(ctx, 520.0))
        .default_height(window_height(ctx, 300.0, true))
        .show(ctx, |ui| {
            ui.label(
                RichText::new("These windows were opened the last time you used Baboon.")
                    .color(text_dark()),
            );
            ui.label(RichText::new("Which of these would you like to reopen?").color(text_dark()));
            ui.add_space(8.0);
            ScrollArea::both().max_height(260.0).show(ui, |ui| {
                for (index, kit) in prompt.kits.iter_mut().enumerate() {
                    if index > 0 {
                        ui.add_space(10.0);
                    }
                    let displayed_source_path =
                        kit.profile_root.as_deref().unwrap_or(&kit.source_path);
                    let (heading, project_path) = last_opened_workspace_heading(
                        kit.profile_name.as_deref().zip(kit.profile_root.as_deref()),
                        kit.game.as_deref(),
                        &kit.source_path,
                        kit.project_path.as_deref(),
                    );
                    let heading_response =
                        ui.label(RichText::new(heading).color(text_dark()).strong());
                    if let Some(project_path) = project_path {
                        heading_response.on_hover_text(&project_path);
                        ui.label(RichText::new(project_path).color(subtle_dark()).small());
                    } else {
                        heading_response.on_hover_text(kit.source_path.display().to_string());
                    }
                    if !kit.source_available {
                        ui.label(
                            RichText::new(format!(
                                "Missing source: {}",
                                displayed_source_path.display()
                            ))
                            .color(Color32::from_rgb(180, 48, 40)),
                        );
                    }
                    // Why a workspace is listed with nothing under it: its
                    // session is its stash, which comes back from its own
                    // recovery file rather than from a list of tabs.
                    if kit.entries.is_empty() && kit.has_project {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("Unsaved changes stashed in this workspace")
                                    .color(subtle_dark())
                                    .small(),
                            );
                        });
                    }
                    if !kit.entries.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Tags").color(subtle_dark()).strong());
                        });
                    }
                    for entry in &mut kit.entries {
                        ui.add_enabled_ui(entry.available, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
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
                    if !kit.folder_entries.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Folders").color(subtle_dark()).strong());
                        });
                    }
                    for entry in &mut kit.folder_entries {
                        ui.add_enabled_ui(entry.available, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.checkbox(&mut entry.checked, "");
                                let label = if entry.available {
                                    entry.folder.rel_path.display().to_string()
                                } else {
                                    format!("{} (missing source)", entry.folder.rel_path.display())
                                };
                                ui.label(RichText::new(label).color(text_dark()));
                            });
                        });
                    }
                    if !kit.chimp_entries.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Chimp").color(subtle_dark()).strong());
                        });
                    }
                    for entry in &mut kit.chimp_entries {
                        ui.add_enabled_ui(entry.available, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.checkbox(&mut entry.checked, "");
                                let label = if entry.available {
                                    entry.package.clone()
                                } else {
                                    format!("{} (missing source)", entry.package)
                                };
                                ui.label(RichText::new(label).color(text_dark()));
                            });
                        });
                    }
                }
            });
            ui.add_space(6.0);
            ui.checkbox(&mut prompt.dont_ask_again, "Don't ask again")
                .on_hover_text(
                    "Remember this choice: OK always reopens the last session, \
                     Cancel never does. Change it later in File > Settings.",
                );
            ui.add_space(4.0);
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
                    action = LastOpenedWindowsAction::Cancel {
                        remember: prompt.dont_ask_again,
                    };
                }
                if ui
                    .add(egui::Button::new("OK").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    action = LastOpenedWindowsAction::Restore {
                        kits: prompt.checked_kits(),
                        remember: prompt.dont_ask_again,
                    };
                }
            });
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

#[cfg(test)]
mod restore_focus_tests;

#[cfg(test)]
mod session_tests;

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
