//! Opening an editing kit from a built-in shortcut, a custom profile or the
//! command line, and finding or asking for the kit's folder.

use super::*;

impl Baboon {
    pub(in crate::app) fn load_editing_kit_shortcut(
        &mut self,
        shortcut: EditingKitShortcut,
        ctx: egui::Context,
    ) {
        let Some(path) = self.model.prefs.editing_kit_paths.get(shortcut.game.as_str()).cloned() else {
            if let Some(profile) = self
                .model.prefs
                .custom_editing_kit_profiles
                .iter()
                .find(|profile| profile.game == shortcut.game.as_str())
                .cloned()
            {
                self.load_custom_editing_kit_profile(profile, ctx);
                return;
            }
            self.prompt_for_editing_kit_path(
                shortcut,
                format!("Set the {} path in Settings first", shortcut.label),
            );
            return;
        };
        let status = self
            .kit_tools.editing_kit_validation
            .refresh_builtin(shortcut, Some(&path));
        let Some(layout) = status.layout().cloned() else {
            self.prompt_for_editing_kit_path(shortcut, status.message());
            return;
        };
        if shortcut.game.is_campaign_evolved() {
            self.begin_load_folder_path(path, ctx);
        } else {
            self.begin_load_editing_kit_layout(
                layout,
                shortcut.game.as_str().to_owned(),
                shortcut.game.display_name().to_owned(),
                None,
                false,
                ctx,
            );
        }
    }

    pub(in crate::app) fn begin_command_line_launch(
        &mut self,
        launch: CommandLineLaunch,
        ctx: egui::Context,
    ) {
        let Some(shortcut) = EDITING_KIT_SHORTCUTS
            .iter()
            .copied()
            .find(|shortcut| shortcut.game == launch.game)
        else {
            self.model.status = format!(
                "Command line: {} is not a supported MCC editing kit",
                launch.kit_label
            );
            return;
        };
        // Several kits of one game can share a root, each with its own tags
        // folder; the one holding the first tag named is the one meant.
        let profiles = &self.model.prefs.custom_editing_kit_profiles;
        let first_absolute = launch.tag_paths.iter().find(|path| path.is_absolute());
        let profile = first_absolute
            .and_then(|tag| {
                let tag = canonical_or_clean(tag);
                profiles.iter().find(|profile| {
                    profile.game == shortcut.game.as_str() && tag.starts_with(profile_tags_folder(profile))
                })
            })
            .or_else(|| {
                profiles
                    .iter()
                    .find(|profile| profile.game == shortcut.game.as_str())
            })
            .cloned();
        if let Some(profile) = profile
            .as_ref()
            .filter(|profile| profile.has_chosen_folders())
            .cloned()
        {
            self.model.kits[self.model.active].restore.pending_launch_tags = Some(launch.tag_paths);
            if !self.load_custom_editing_kit_profile(profile, ctx) {
                self.model.kits[self.model.active].restore.pending_launch_tags = None;
                self.model.status = format!("Command line: {}", self.model.status);
            }
            return;
        }
        let Some(path) = profile
            .map(|profile| profile.root)
            .or_else(|| self.model.prefs.editing_kit_paths.get(shortcut.game.as_str()).cloned())
        else {
            self.model.status = format!(
                "Command line: set the {} path in Settings before launching tags",
                launch.kit_label
            );
            return;
        };
        let status = self
            .kit_tools.editing_kit_validation
            .refresh_builtin(shortcut, Some(&path));
        let Some(layout) = status.layout().cloned() else {
            self.model.status = format!("Command line: {}", status.message());
            return;
        };
        self.model.kits[self.model.active].restore.pending_launch_tags = Some(launch.tag_paths);
        self.begin_load_editing_kit_layout(
            layout,
            shortcut.game.as_str().to_owned(),
            shortcut.game.display_name().to_owned(),
            None,
            false,
            ctx,
        );
    }

    pub(in crate::app) fn finish_pending_command_line_launch(&mut self, ctx: egui::Context) {
        let Some(requested) = self.model.kits[self.model.active].restore.pending_launch_tags.take() else {
            return;
        };
        // Command-line startup deliberately remains popup-free. Indexing still
        // runs in the background and remains visible in the status bar.
        self.kit_tools.show_entry_index_wait_notice = false;
        let Some(source) = self.model.source() else {
            self.model.status = "Command line: the editing-kit source did not load".to_owned();
            return;
        };
        let TagSource::LooseFolder { root, .. } = &source.source else {
            self.model.status = "Command line: the selected source is not a loose editing kit".to_owned();
            return;
        };
        let root = root.clone();
        let names = source.names.clone();
        let resolved = match resolve_launch_tag_entries(&root, &requested, &names) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.model.status = format!("Command line: {error}");
                return;
            }
        };
        let errors = resolved.errors;
        let entries = resolved.entries;
        let folder_seeds = self.model.kits[self.model.active].folder_seeds();
        if let Some(source) = self.source_mut() {
            for entry in &entries {
                if source.entry_for_key(&entry.key).is_none() {
                    source.upsert_entry(entry.clone(), &folder_seeds);
                }
            }
        }
        for entry in &entries {
            self.select_entry(entry.key.clone(), ctx.clone());
        }
        self.model.status = match (entries.len(), errors.len()) {
            (opened, 0) => format!("Opened {opened} command-line tag(s)"),
            (opened, skipped) => format!(
                "Opened {opened} command-line tag(s); skipped {skipped}: {}",
                errors.join("; ")
            ),
        };
    }

    pub(in crate::app) fn load_custom_editing_kit_profile(
        &mut self,
        profile: CustomEditingKitProfile,
        ctx: egui::Context,
    ) -> bool {
        let layout = match self.kit_tools.editing_kit_validation.refresh_custom(&profile) {
            Ok(layout) => layout,
            Err(error) => {
                self.model.status = format!("{} is unavailable: {error}", profile.name);
                return false;
            }
        };
        if profile.is_campaign_evolved() {
            self.begin_load_folder_path(profile.root.clone(), ctx);
            self.model.kits[self.model.active].profile = Some(EditingKitProfileIdentity {
                id: profile.id,
                name: profile.name,
            });
            return true;
        }
        let chosen_folders = profile.has_chosen_folders();
        self.begin_load_editing_kit_layout(
            layout,
            profile.game.clone(),
            profile.name.clone(),
            Some(EditingKitProfileIdentity {
                id: profile.id,
                name: profile.name,
            }),
            chosen_folders,
            ctx,
        );
        true
    }

    /// Load a kit from its validated layout. `chosen_folders` is a profile
    /// that names its own tags or data folder: the loaded source then carries
    /// exactly this layout, rather than working out its root and data folder
    /// from its tags folder, and the kit is identified by its tags folder,
    /// since its root may be shared with other kits.
    pub(in crate::app) fn begin_load_editing_kit_layout(
        &mut self,
        layout: EditingKitLayout,
        game: String,
        label: String,
        profile: Option<EditingKitProfileIdentity>,
        chosen_folders: bool,
        ctx: egui::Context,
    ) {
        // Profiles keep the game id they were saved with; one this build does
        // not know has no definitions to load against.
        let Some(game) = GameId::from_id(&game) else {
            self.model.status = format!("{label} is for a game this version of Baboon does not know ({game})");
            return;
        };
        let chosen_layout = chosen_folders.then(|| KitLayout {
            root: layout.root.clone(),
            tags: layout.tags.clone(),
            data: layout
                .data
                .clone()
                .unwrap_or_else(|| layout.root.join("data")),
        });
        // What the kit is remembered and matched by: its root, unless other
        // kits may share that root, when it is its tags folder.
        let identity_path = if chosen_folders {
            layout.tags.clone()
        } else {
            layout.root.clone()
        };
        if let Some(profile_identity) = profile.as_ref() {
            if let Some(index) = self.model.kits.iter().position(|kit| {
                kit.profile.as_ref().map(|open| open.id.as_str())
                    == Some(profile_identity.id.as_str())
            }) {
                self.model.active = index;
                self.model.status = format!("Switched to {}", label);
                return;
            }
            // A kit already open on this profile's tags folder (opened as a
            // folder) becomes this profile's. Matched on the tags folder, not
            // the root: kits sharing a root are different kits.
            if !chosen_folders
                && let Some(index) = self.model.kits.iter().position(|kit| {
                    kit.requested_path
                        .as_deref()
                        .is_some_and(|open| same_recent_path(open, &layout.root))
                        && kit
                            .source
                            .as_ref()
                            .and_then(LoadedSourceData::kit_layout)
                            .is_some_and(|open| same_recent_path(&open.tags, &layout.tags))
                        && kit
                            .source
                            .as_ref()
                            .and_then(|source| source.game)
                            == Some(game)
                })
            {
                self.model.active = index;
                self.model.kits[index].profile = Some(profile_identity.clone());
                self.model.status = format!("Switched to {}", label);
                return;
            }
            if !self.model.kits[self.model.active].can_accept_source_load() {
                self.add_kit();
            }
            self.model.kits[self.model.active].requested_path = Some(identity_path.clone());
        } else if self.open_kit_for(&layout.root) {
            self.model.status = format!("Switched to {}", label);
            return;
        }
        self.model.kits[self.model.active].profile = profile;
        let tx = self.tx.clone();
        let kit = self.model.active_kit_id();
        let names = self.model.default_names.clone();
        let definitions_root = locate_definitions_root();
        let tags_root = layout.tags;
        let recent_path = identity_path;
        self.model.status = format!("Indexing {} as {game}", tags_root.display());
        // Through `spawn_worker`: a loader that panicked used to send nothing,
        // leaving the kit reserved for this load ("starting up") for good.
        spawn_worker(
            &tx,
            &ctx,
            move || {
                let result = load_editing_kit_layout(tags_root, label, game, &names, &definitions_root)
                    .map(|mut source| {
                        source.chosen_kit_layout = chosen_layout;
                        source
                    })
                    .map_err(|error| error.to_string());
                WorkerMessage::SourceLoaded {
                    kit,
                    result,
                    recent_path: Some(recent_path),
                }
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(format!("Loading failed: {error}")),
                recent_path: None,
            },
        );
    }

    pub(in crate::app) fn choose_editing_kit_path(&mut self, shortcut: EditingKitShortcut) {
        let title = if shortcut.game.is_campaign_evolved() {
            "Select Campaign Evolved Install or Paks Folder".to_owned()
        } else {
            format!("Select {} Editing Kit Folder", shortcut.label)
        };
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(path) = self.model.prefs.editing_kit_paths.get(shortcut.game.as_str()) {
            if path.is_dir() {
                dialog = dialog.set_directory(path);
            } else if let Some(parent) = path.parent().filter(|parent| parent.is_dir()) {
                dialog = dialog.set_directory(parent);
            }
        }
        if let Some(path) = dialog.pick_folder() {
            self.model.prefs
                .editing_kit_paths
                .insert(shortcut.game.as_str().to_owned(), path.clone());
            self.kit_tools.editing_kit_path_inputs
                .insert(shortcut.game.as_str().to_owned(), path.display().to_string());
            if self.kit_tools.editing_kit_path_attention.as_deref() == Some(shortcut.game.as_str()) {
                self.kit_tools.editing_kit_path_attention = None;
            }
            self.model.status = format!("{} path set to {}", shortcut.label, path.display());
            self.refresh_builtin_editing_kit_validation(shortcut);
        }
    }

    pub(in crate::app) fn auto_detect_editing_kit_paths(&mut self) {
        let detected = detect_editing_kit_paths();
        let previous = self.model.prefs.custom_editing_kit_profiles.clone();
        let added = add_standard_editing_kit_profiles(
            &mut self.model.prefs.custom_editing_kit_profiles,
            &detected,
        );
        if added > 0 {
            let prefs = self.current_prefs();
            if let Err(error) = save_gui_prefs(
                &prefs,
                &self.kit_tools.terminal_open_games,
                self.shell.first_run_wizard.is_none(),
            ) {
                self.model.prefs.custom_editing_kit_profiles = previous;
                self.model.status = error;
                return;
            }
            self.saved_prefs = prefs;
            self.kit_tools.saved_terminal_open_games = self.kit_tools.terminal_open_games.clone();
        }
        self.refresh_editing_kit_validation();
        self.model.status = if added == 0 {
            "No new editing kit paths detected".to_owned()
        } else {
            format!("Detected {added} editing kit path(s)")
        };
    }

    pub(in crate::app) fn prompt_for_editing_kit_path(&mut self, shortcut: EditingKitShortcut, status: String) {
        self.shell.settings_open = true;
        self.shell.settings_tab = SettingsTab::EditingKits;
        self.kit_tools.editing_kit_path_attention = Some(shortcut.game.as_str().to_owned());
        self.kit_tools.editing_kit_path_inputs
            .entry(shortcut.game.as_str().to_owned())
            .or_default();
        self.model.status = status;
    }
}
