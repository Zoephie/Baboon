//! Finding a kit's folders and tools, refusing edits to a read-only kit, and
//! launching Sapien, Guerilla, tag_test, Blender and tool imports.

use super::*;

impl Baboon {
    pub(in crate::app) fn editing_kit_root(&self) -> Option<PathBuf> {
        self.editing_kit_root_for(self.model.active)
    }

    pub(in crate::app) fn editing_kit_is_read_only(&self, kit_index: usize) -> bool {
        let Some(kit) = self.model.kits.get(kit_index) else {
            return false;
        };
        // The tags folder rather than the kit root: every folder above the
        // root is above the tags folder too, so this matches at least what the
        // root did and never makes a read-only kit writable.
        let root = self
            .kit_layout_for(kit_index)
            .map(|layout| layout.tags)
            .or_else(|| match &kit.source.as_ref()?.source {
                TagSource::SingleFile { path } => Some(path.clone()),
                _ => None,
            });
        self.model.prefs
            .custom_editing_kit_profiles
            .iter()
            .any(|profile| profile.is_read_only_for(kit.profile.as_ref(), root.as_deref()))
    }

    pub(in crate::app) fn refuse_read_only_edit(&mut self, kit_index: usize) -> bool {
        if self.editing_kit_is_read_only(kit_index) {
            self.model.status =
                "This editing kit is read-only. Change its Editing Kit settings to enable editing."
                    .to_owned();
            true
        } else {
            false
        }
    }

    pub(in crate::app) fn editing_kit_root_for(&self, kit_index: usize) -> Option<PathBuf> {
        Some(self.kit_layout_for(kit_index)?.root)
    }

    /// The loaded kit's root, tags and data folders. See [`KitLayout`].
    pub(in crate::app) fn kit_layout_for(&self, kit_index: usize) -> Option<KitLayout> {
        self.model.kits.get(kit_index)?.source.as_ref()?.kit_layout()
    }

    pub(in crate::app) fn kit_tool_path(&self, executable_name: &str) -> Option<PathBuf> {
        Some(self.editing_kit_root()?.join(executable_name))
    }

    pub(in crate::app) fn launch_sapien(&mut self) {
        self.launch_kit_tool("Sapien", "sapien.exe");
    }

    /// The tag_test executable name for the loaded game. Each editing kit ships
    /// its own renamed build (e.g. H3EK is `halo3_tag_test.exe`); fall back to
    /// the generic name when the game is unknown.
    pub(in crate::app) fn tag_test_executable(&self) -> &'static str {
        tag_test_executable_for_game(self.source().and_then(|s| s.game))
    }

    pub(in crate::app) fn launch_tag_test(&mut self) {
        self.launch_kit_tool_clearing_startup("tag_test", self.tag_test_executable(), "init.txt");
    }

    pub(in crate::app) fn launch_blender(&mut self) {
        let Some(path) = self.model.prefs.blender_path.clone() else {
            self.shell.settings_open = true;
            self.model.status = "Set the Blender path in File > Settings first".to_owned();
            return;
        };
        if !path.is_file() {
            self.model.status = format!("Blender executable not found: {}", path.display());
            self.shell.settings_open = true;
            return;
        }
        self.spawn_tool("Blender", &path, path.parent().map(Path::to_path_buf), &[]);
    }

    pub(in crate::app) fn choose_blender_path(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Select Blender Executable");
        if let Some(path) = self
            .model.prefs
            .blender_path
            .as_ref()
            .and_then(|path| path.parent())
        {
            dialog = dialog.set_directory(path);
        }
        #[cfg(target_os = "windows")]
        {
            dialog = dialog.add_filter("Executable", &["exe"]);
        }
        if let Some(path) = dialog.pick_file() {
            self.model.prefs.blender_path = Some(path.clone());
            self.kit_tools.blender_path_input = path.display().to_string();
            self.model.status = format!("Blender path set to {}", path.display());
        }
    }

    pub(in crate::app) fn launch_kit_tool(&mut self, label: &str, executable_name: &str) {
        let Some(path) = self.kit_tool_path(executable_name) else {
            self.model.status = format!("{label} requires a loaded editing-kit folder");
            return;
        };
        if !path.is_file() {
            self.model.status = format!("{label} executable not found: {}", path.display());
            return;
        }
        let options = self.active_kit_tool_folder_options();
        self.spawn_tool(label, &path, self.editing_kit_root(), &options);
    }

    pub(in crate::app) fn launch_kit_tool_clearing_startup(
        &mut self,
        label: &str,
        executable_name: &str,
        startup_file_name: &str,
    ) {
        let Some(path) = self.kit_tool_path(executable_name) else {
            self.model.status = format!("{label} requires a loaded editing-kit folder");
            return;
        };
        if !path.is_file() {
            self.model.status = format!("{label} executable not found: {}", path.display());
            return;
        }
        let Some(root) = self.editing_kit_root() else {
            self.model.status = format!("{label} requires a loaded editing-kit folder");
            return;
        };
        let startup_file = root.join(startup_file_name);
        if let Err(error) = clear_scenario_startup_commands(&startup_file) {
            self.model.status = error;
            return;
        }
        let options = self.active_kit_tool_folder_options();
        self.spawn_tool(label, &path, Some(root), &options);
    }

    pub(in crate::app) fn spawn_tool(
        &mut self,
        label: &str,
        path: &Path,
        work_dir: Option<PathBuf>,
        options: &[(&'static str, PathBuf)],
    ) {
        let mut command = Command::new(path);
        for (option, folder) in options {
            command.arg(option).arg(folder);
        }
        if let Some(work_dir) = work_dir {
            command.current_dir(work_dir);
        }
        match command.spawn() {
            Ok(_) => self.model.status = format!("Launched {label}"),
            Err(error) => self.model.status = format!("Could not launch {label}: {error}"),
        }
    }

    /// Run a geometry Import request (`tool render/collision/physics/...`)
    /// streamed to the terminal panel.
    pub(in crate::app) fn process_pending_tool_import(&mut self, ctx: &egui::Context) {
        if self.editing_kit_is_read_only(self.model.active) {
            self.kit_tools.pending_tool_import = None;
            self.refuse_read_only_edit(self.model.active);
            return;
        }
        let Some(req) = self.kit_tools.pending_tool_import.take() else {
            return;
        };
        if self.editing_kit_root().is_none() {
            self.model.status = "Import requires a loaded editing-kit folder".to_owned();
            return;
        }
        let command = format!("tool {} \"{}\"", req.verb, req.source_dir);
        self.spawn_terminal_command(command, ctx.clone());
    }

    /// Queue the same editing-kit geometry import that a compatible tag
    /// reference offers, deriving the tool source folder from the clicked tag.
    pub(in crate::app) fn begin_reimport_geometry(&mut self, key: &str) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.model.status = "The tag is no longer in the browser".to_owned();
            return;
        };
        if !matches!(entry.location, TagEntryLocation::LooseFile(_)) {
            self.model.status = "Reimport requires a loose editing-kit tag".to_owned();
            return;
        }
        let Some(verb) = geometry_import_verb(self.names(), entry.group_tag) else {
            self.model.status = "This tag type does not support reimport".to_owned();
            return;
        };
        self.kit_tools.pending_tool_import = Some(ToolImportRequest {
            verb,
            source_dir: model_source_dir(&entry_rel_path(&entry)),
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_reimport_bitmap(&mut self, key: String, ctx: egui::Context) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        if self.kit_tools.terminal.running {
            self.model.status = "A command is already running".to_owned();
            return;
        }
        let Some(source) = self.source().map(|source| source.source.clone()) else {
            self.model.status = "Reimport requires a loaded editing-kit folder".to_owned();
            return;
        };
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.model.status = "Bitmap tag is no longer in the source".to_owned();
            return;
        };
        let Some(tags_root) = (match &source {
            TagSource::LooseFolder { root, .. } => Some(root.as_path()),
            _ => None,
        }) else {
            self.model.status = "Bitmap reimport requires a loose tags folder".to_owned();
            return;
        };
        let Some(work_dir) = self.kit_layout_for(self.model.active).map(|layout| layout.root) else {
            self.model.status = "Could not resolve editing-kit root".to_owned();
            return;
        };
        let Some(data_path) = bitmap_reimport_data_path(&entry, Some(tags_root)) else {
            self.model.status = "Could not resolve bitmap data path".to_owned();
            return;
        };
        let command = with_tool_folder_options(
            &format!("tool bitmaps \"{data_path}\""),
            &self.active_kit_tool_folder_options(),
        );
        self.views[self.model.kits[self.model.active].id].terminal.open = true;
        self.kit_tools.terminal
            .lines
            .push(TerminalLineEntry::new(format!("> {command}")));
        trim_terminal_lines(&mut self.kit_tools.terminal.lines);
        self.kit_tools.terminal.scroll_to_bottom = true;
        self.kit_tools.terminal.refocus_input = true;
        self.kit_tools.terminal.running = true;
        self.model.status = format!("Reimporting bitmap {}", entry.display_path);
        let run_id = self.kit_tools.terminal.next_run_id;
        self.kit_tools.terminal.next_run_id = self.kit_tools.terminal.next_run_id.wrapping_add(1).max(1);
        let log_file = match create_terminal_log_file(run_id, &command) {
            Ok((path, file)) => {
                self.kit_tools.terminal.last_log_path = Some(path);
                Some(file)
            }
            Err(error) => {
                self.model.status = format!("Terminal full log unavailable: {error}");
                self.kit_tools.terminal.last_log_path = None;
                None
            }
        };

        let tx = self.tx.clone();
        let kit = self.active_kit_id();
        let panic_key = key.clone();
        let worker_ctx = ctx.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || {
                let result =
                    run_terminal_command_for_reimport(&command, &work_dir, &tx, &worker_ctx, log_file)
                        .and_then(|_| read_entry(&source, &entry).map_err(|error| error.to_string()));
                WorkerMessage::BitmapReimportFinished { kit, key, result }
            },
            move |error| WorkerMessage::BitmapReimportFinished {
                kit,
                key: panic_key,
                result: Err(format!("The reimport crashed: {error}")),
            },
        );
    }
}

impl Baboon {
    pub(in crate::app) fn active_game_is_campaign_evolved(&self) -> bool {
        self.source_game().is_some_and(GameId::is_campaign_evolved)
    }
}
