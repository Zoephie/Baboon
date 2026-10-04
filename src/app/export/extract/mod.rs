//! Starting each extraction and export as a background job: raw tags, bitmaps
//! and their source images, sounds, geometry, animations, import info, shader
//! sources, scripts, JSON, reference dumps, and whole containers or folders of
//! them.

use super::*;
use crate::app::documents::saving::entries_for_keys;

impl Baboon {
    /// Write the whole tree of tags this one pulls in to a text file.
    ///
    /// Runs on the UI thread, unlike the JSON dump beside it: that one re-parses
    /// the tag from disk, while this reads an index already in memory and a walk
    /// over it costs microseconds. Cloning the index onto a worker would be the
    /// expensive half of the job.
    pub(in crate::app) fn begin_dump_tag_references(&mut self, key: &str, _ctx: egui::Context) {
        let Some(source) = self.model.source() else {
            self.model.status = "No tag source is loaded".to_owned();
            return;
        };
        let Some(index) = source.reverse_dependencies.as_ref() else {
            self.model.status = "Build the reference index first — Tools ▸ Build/Rebuild Reference Index"
                .to_owned();
            return;
        };
        let Some(root) = self.model.entry_for_key(key).cloned() else {
            self.model.status = "That tag is no longer in the source".to_owned();
            return;
        };
        // Built once over the whole entry set. `children_of_entry` rebuilds this
        // per call, which is fine for one hop and quadratic inside a recursion.
        let mut by_dependency_key: HashMap<String, TagEntry> = HashMap::new();
        for entry in source.full_entry_set() {
            if let Some(rel) = dependency_entry_reference_path(entry, self.model.names()) {
                by_dependency_key
                    .entry(crate::core::source::dependency_key(entry.group_tag, &rel))
                    .or_insert_with(|| entry.clone());
            }
        }
        let report = tag_reference_tree_text(index, &by_dependency_key, &root);

        let default_name = format!("{}-references.txt", tag_file_stem(&root));
        let Some(output) = rfd::FileDialog::new()
            .set_title("Dump Tag References")
            .add_filter("Text file", &["txt"])
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        self.model.status = match fs::write(&output, report) {
            Ok(()) => format!("Wrote {}", output.display()),
            Err(error) => format!("Could not write {}: {error}", output.display()),
        };
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_export_json(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.model.export_context(&key) else {
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
        self.model.status = format!("Dumping JSON for {}", entry.display_path);
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            export_tag_json(&source, &entry, &output).map_err(|e| e.to_string())
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_export_loaded_folder_json(
        &mut self,
        keys: Vec<String>,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.model.source() else {
            return;
        };
        let entries = keys
            .iter()
            .filter_map(|key| source_data.entries.iter().find(|entry| entry.key == *key))
            .cloned()
            .collect::<Vec<_>>();
        if entries.is_empty() {
            self.model.status = "No loaded tags found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Dump Folder JSON")
            .pick_folder()
        else {
            return;
        };
        self.model.status = format!("Dumping {} loaded tag(s) to JSON", entries.len());
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            export_tag_json_entries(&source, &entries, &output).map_err(|e| e.to_string())
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_export_loose_folder_json(
        &mut self,
        rel_path: PathBuf,
        label: String,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.model.source() else {
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
        self.model.status = format!("Dumping JSON for folder {label}");
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            export_loose_folder_json(&root, &rel_path, &names, &output)
                .map_err(|e| e.to_string())
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_extract_raw(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.model.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Raw Tag")
            .set_file_name(tag_file_name(&entry).as_str())
            .save_file()
        else {
            return;
        };
        self.model.status = format!("Extracting raw tag {}", entry.display_path);
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            extract_raw_tag(&source, &entry, &output).map_err(|e| e.to_string())
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_extract_bitmap(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.model.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Bitmap Images")
            .pick_folder()
        else {
            return;
        };
        self.model.status = format!("Extracting bitmap {}", entry.display_path);
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            extract_bitmap_images(&source, &entry, &output).map_err(|e| e.to_string())
        });
    }

    /// Recovers the color plates of `keys` into a folder the user picks,
    /// starting from the active kit's data folder. A `folder` extract keeps
    /// each tag's folder under it; a single tag lands in it directly.
    pub(in crate::app) fn begin_extract_bitmap_sources(
        &mut self,
        keys: Vec<String>,
        folder: bool,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.model.source() else {
            return;
        };
        let entries = keys
            .iter()
            .filter_map(|key| source_data.entries.iter().find(|entry| entry.key == *key))
            .cloned()
            .collect::<Vec<_>>();
        if entries.is_empty() {
            self.model.status = "No bitmap tags found".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let mut dialog = rfd::FileDialog::new().set_title("Extract Bitmap Source");
        if let Some(layout) = self.model.kit_layout_for(self.model.active) {
            dialog = dialog.set_directory(layout.data);
        }
        let Some(output) = dialog.pick_folder() else {
            return;
        };
        self.model.status = match entries.as_slice() {
            [entry] if !folder => format!("Extracting bitmap source for {}", entry.display_path),
            entries => format!("Extracting {} bitmap source(s)", entries.len()),
        };
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            match entries.as_slice() {
                [entry] if !folder => extract_bitmap_source(&source, entry, &output),
                entries => extract_bitmap_sources(&source, entries, &output),
            }
            .map_err(|e| e.to_string())
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_extract_bitmap_folder(&mut self, keys: Vec<String>, ctx: egui::Context) {
        let Some(source_data) = self.model.source() else {
            return;
        };
        let entries = keys
            .iter()
            .filter_map(|key| source_data.entries.iter().find(|entry| entry.key == *key))
            .cloned()
            .collect::<Vec<_>>();
        if entries.is_empty() {
            self.model.status = "No bitmap tags found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract All Bitmaps")
            .pick_folder()
        else {
            return;
        };
        self.model.status = format!("Extracting {} bitmap tag(s)", entries.len());
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            extract_bitmap_entries(&source, &entries, &output).map_err(|e| e.to_string())
        });
    }

    /// Extract one or more browser-selected `.sound` tags without opening
    /// them. Loose kits retain the sound pane's automatic `data[_lang]` layout;
    /// container sounds have no editing-kit data root and therefore retain
    /// their existing folder picker behavior.
    pub(in crate::app) fn begin_extract_sounds(&mut self, keys: Vec<String>, all_languages: bool) {
        let entries: Vec<TagEntry> = keys
            .iter()
            .filter_map(|key| self.model.entry_for_key(key).cloned())
            .filter(|entry| crate::app::editor::is_sound_group(entry.group_tag))
            .collect();
        if entries.is_empty() {
            self.model.status = "No loaded sound tags found".to_owned();
            return;
        }

        let source_kind = self.model.source().map(|source| source.source.clone());
        match source_kind {
            Some(TagSource::LooseFolder { root, .. }) => {
                let game = self.model.source().and_then(|source| source.game.clone());
                let selected_language = self.audio.language.clone();
                let shared_fmod_banks = matches!(
                    game,
                    Some(GameId::Halo3 | GameId::Halo3Odst | GameId::HaloReach)
                )
                .then(|| {
                    blam_tags::audio::SoundBanks::open_pc_language(
                        &root,
                        Some("__baboon_shared_bank_only__"),
                    )
                    .ok()
                })
                .flatten();
                let Some(layout) = self.model.kit_layout_for(self.model.active) else {
                    self.model.status = "Could not resolve the editing kit's data folder".to_owned();
                    return;
                };
                let mut items = Vec::new();
                let mut read_errors = 0usize;
                for entry in &entries {
                    let abs = match &entry.location {
                        TagEntryLocation::LooseFile(path) => path.clone(),
                        _ => continue,
                    };
                    match crate::core::source::read_entry(
                        self.model.source()
                            .map(|source| &source.source)
                            .expect("source exists"),
                        entry,
                    ) {
                        Ok(tag) => items.extend(crate::app::editor::browser_sound_extract_items(
                            &tag,
                            &abs,
                            &layout,
                            game,
                            selected_language.as_deref(),
                            all_languages,
                            shared_fmod_banks.as_ref(),
                        )),
                        Err(_) => read_errors += 1,
                    }
                }
                if items.is_empty() {
                    self.model.status = if read_errors > 0 {
                        format!("Could not read {read_errors} sound tag(s)")
                    } else {
                        "The selected sound tags contain no extractable audio".to_owned()
                    };
                    return;
                }
                let count = entries.len();
                self.export.pending_sound_extract = Some(crate::app::export::sound_extract::ExtractRequest {
                    items,
                    tags_root: Some(root),
                    label: if all_languages {
                        format!("{count} sound tag(s), all available languages")
                    } else {
                        format!("{count} sound tag(s)")
                    },
                });
            }
            Some(TagSource::IoStoreContainerSet { root, .. }) => {
                let Some(base) = rfd::FileDialog::new()
                    .set_title(if all_languages {
                        "Extract Sound Tags (All Available Languages)"
                    } else {
                        "Extract Sound Tags"
                    })
                    .pick_folder()
                else {
                    return;
                };
                let multiple = entries.len() > 1;
                let selected_language = self.audio.language.clone();
                let mut items = Vec::new();
                for entry in &entries {
                    let Some(binding) = self.ce_sound_binding(self.model.active, &entry.key, entry)
                    else {
                        continue;
                    };
                    let languages = if all_languages {
                        binding.languages()
                    } else {
                        vec![binding.language_to_show(selected_language.as_deref())]
                    };
                    let tag_base = if multiple {
                        base.join(std::path::Path::new(&entry.display_path).with_extension(""))
                    } else {
                        base.clone()
                    };
                    for language in languages {
                        let language_base = if all_languages {
                            tag_base.join(crate::app::export::sound_extract::sanitize_component(&language))
                        } else {
                            tag_base.clone()
                        };
                        items.extend(binding.media_for_language(&language).into_iter().map(
                            |media| crate::app::export::sound_extract::ExtractItem {
                                out_path: language_base.join(format!(
                                    "{}.wav",
                                    crate::app::export::sound_extract::sanitize_component(
                                        &media.display_name()
                                    )
                                )),
                                source: crate::app::export::sound_extract::ExtractSource::CeMedia {
                                    paks_root: root.clone(),
                                    media: Box::new(media.clone()),
                                },
                            },
                        ));
                    }
                }
                if items.is_empty() {
                    self.model.status = "The selected sound tags have no audio bound".to_owned();
                    return;
                }
                self.export.pending_sound_extract = Some(crate::app::export::sound_extract::ExtractRequest {
                    items,
                    tags_root: None,
                    label: format!("{} sound tag(s)", entries.len()),
                });
            }
            _ => {
                self.model.status =
                    "Sound extraction requires an editing-kit or container source".to_owned();
            }
        }
    }

    /// Asks where to put every shipped tag in the mounted containers, then
    /// raises the confirmation that says what that costs.
    ///
    /// Expert-only, and re-checked here rather than trusting the menu: this is
    /// the one action in the application that writes tens of thousands of files
    /// in one go, and it should not be reachable by a stale request. The folder-
    /// scoped twin below carries no such gate, because it is bounded and aimed.
    pub(in crate::app) fn begin_extract_all_container_tags(&mut self, _ctx: egui::Context) {
        if !self.model.prefs.expert_mode {
            self.model.status = "Extracting all tags requires Expert mode".to_owned();
            return;
        }
        self.raise_container_dump_confirm(ContainerDumpScope::AllShipped, "Extract All Tags");
    }

    /// Asks where to put the shipped tags beneath one browser folder.
    ///
    /// The keys are the ones collected when the menu was drawn, so what runs is
    /// what the count in the menu promised.
    pub(in crate::app) fn begin_extract_container_folder_tags(&mut self, label: String, keys: Vec<String>) {
        self.raise_container_dump_confirm(
            ContainerDumpScope::Folder { label, keys },
            "Extract Folder Tags",
        );
    }

    /// The shared front half of both extractions: refuse to stack a second run,
    /// require a container mount, count what the scope actually covers, pick a
    /// destination, and keep that destination out of the game's own Paks folder.
    pub(in crate::app) fn raise_container_dump_confirm(&mut self, scope: ContainerDumpScope, dialog_title: &str) {
        if self.export.container_dump_job.is_some() {
            self.model.status = "An extraction is already running".to_owned();
            return;
        }
        let Some(source_data) = self.model.source() else {
            return;
        };
        let TagSource::IoStoreContainerSet { root, .. } = &source_data.source else {
            self.model.status = "Extracting tags needs a Campaign Evolved container".to_owned();
            return;
        };
        let root = root.clone();
        // A container mount enumerates every tag up front, so this is the whole
        // set — there is no background scan to wait on first.
        let total = container_dump_entries(&source_data.entries, &scope).len();
        if total == 0 {
            self.model.status = match &scope {
                ContainerDumpScope::AllShipped => {
                    "This workspace has no container tags to extract".to_owned()
                }
                ContainerDumpScope::Folder { label, .. } => {
                    format!("{label} has no shipped tags to extract")
                }
            };
            return;
        }
        let Some(output) = rfd::FileDialog::new().set_title(dialog_title).pick_folder() else {
            return;
        };
        // Files landing in the game's own Paks folder would be found by the next
        // mount and are a nuisance to unpick by hand.
        if output.starts_with(&root) {
            self.model.status = format!(
                "Choose a folder outside {} — extracting into the game's own Paks folder would \
                 leave the extracted tags beside its containers",
                root.display()
            );
            return;
        }
        self.dialogs.open(ContainerDumpConfirm {
            kit: self.model.active_kit_id(),
            output,
            total,
            scope,
        });
    }

    /// Runs the confirmed extraction on a worker thread.
    pub(in crate::app) fn start_container_dump(
        &mut self,
        kit: KitId,
        output: PathBuf,
        scope: ContainerDumpScope,
        ctx: egui::Context,
    ) {
        if self.export.container_dump_job.is_some() {
            self.model.status = "An extraction is already running".to_owned();
            return;
        }
        let Some(index) = self.model.kit_index(kit) else {
            return;
        };
        let Some(source_data) = self.model.kits[index].source.as_ref() else {
            return;
        };
        // Cloning the source is cheap: the mounted archives are behind `Arc`, so
        // this shares them rather than re-mapping the containers.
        let source = source_data.source.clone();
        // Re-resolved rather than carried over from the confirmation: the user
        // can edit the workspace while a modeless confirm is up, so this is the
        // set as it stands at the moment the run actually starts.
        let entries: Vec<TagEntry> = container_dump_entries(&source_data.entries, &scope)
            .into_iter()
            .cloned()
            .collect();
        let total = entries.len();
        if total == 0 {
            self.model.status = match &scope {
                ContainerDumpScope::AllShipped => {
                    "This workspace has no container tags to extract".to_owned()
                }
                ContainerDumpScope::Folder { label, .. } => {
                    format!("{label} has no shipped tags to extract")
                }
            };
            return;
        }
        let stamp = KitStamp {
            kit,
            generation: self.model.kits[index].generation,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.export.container_dump_job = Some(ContainerDumpJob {
            kit,
            output: output.clone(),
            done: 0,
            total,
            started: std::time::Instant::now(),
            cancel: cancel.clone(),
        });
        self.model.status = format!("Extracting {total} tag(s) to {}", output.display());
        let tx = self.tx.clone();
        let worker_ctx = ctx.clone();
        spawn_worker(&self.tx, &ctx, move || {
            let progress_tx = tx.clone();
            let progress_ctx = worker_ctx;
            let progress = move |done: usize, total: usize| {
                let _ =
                    progress_tx.send(WorkerMessage::ContainerDumpProgress { stamp, done, total });
                progress_ctx.request_repaint();
            };
            // This reads memory-mapped `.ucas` partitions across several
            // threads; a panic in there ends the job, not the application.
            let result = dump_shipped_container_tags(&source, &entries, &output, &cancel, &progress)
                .map_err(|error| error.to_string());
            WorkerMessage::ContainerDumpFinished { stamp, result }
        }, move |_| WorkerMessage::ContainerDumpFinished {
            stamp,
            result: Err("Tag extraction worker crashed".to_owned()),
        });
    }

    /// Open the window that asks which game's tools a geometry or animation
    /// extraction is for, defaulting to the active kit's game.
    pub(in crate::app) fn prompt_extract_target(&mut self, key: String, kind: ExtractKind) {
        let Some(entry) = self.model.entry_for_key(&key) else {
            return;
        };
        let display_path = entry.display_path.clone();
        let source = self.model.source_game().map_or(blam_tags::game::Game::Halo3, GameId::generation);
        self.dialogs.open(ExtractTargetPrompt {
            key,
            display_path,
            kind,
            source,
            target: source,
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_extract_geometry(
        &mut self,
        key: String,
        target: blam_tags::game::Game,
        ctx: egui::Context,
    ) {
        let Some((source, entry)) = self.model.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Geometry")
            .pick_folder()
        else {
            return;
        };
        self.model.status = format!("Extracting geometry from {}", entry.display_path);
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            extract_geometry_for_entry(&source, &entry, &output, target)
                .map_err(|e| e.to_string())
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_extract_import_info(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.model.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Import Info")
            .pick_folder()
        else {
            return;
        };
        self.model.status = format!("Extracting import info from {}", entry.display_path);
        let tx = self.tx.clone();
        let is_model = entry.group_tag == u32::from_be_bytes(*b"hlmt");
        spawn_export(&tx, &ctx, move || {
            if is_model {
                extract_import_info_for_model_entry(&source, &entry, &output)
            } else {
                extract_import_info_for_entry(&source, &entry, &output)
            }
            .map_err(|e| e.to_string())
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_extract_animation(
        &mut self,
        key: String,
        target: blam_tags::game::Game,
        ctx: egui::Context,
    ) {
        let Some((source, entry)) = self.model.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Animations")
            .pick_folder()
        else {
            return;
        };
        self.model.status = format!("Extracting animations from {}", entry.display_path);
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            extract_animations_for_entry(&source, &entry, &output, target)
                .map_err(|e| e.to_string())
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_extract_material_shader_sources(
        &mut self,
        key: String,
        ctx: egui::Context,
    ) {
        let Some((source, entry)) = self.model.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Source Shaders")
            .pick_folder()
        else {
            return;
        };
        self.model.status = format!("Extracting source shaders from {}", entry.display_path);
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            extract_material_shader_sources(&source, &entry, &output)
                .map_err(|e| e.to_string())
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_extract_material_shader_source_folder(
        &mut self,
        keys: Vec<String>,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.model.source() else {
            return;
        };
        let entries = entries_for_keys(source_data, &keys);
        if entries.is_empty() {
            self.model.status = "No material shaders found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Material Shader Sources")
            .pick_folder()
        else {
            return;
        };
        self.model.status = format!(
            "Extracting source shaders from {} material shader(s)",
            entries.len()
        );
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            extract_material_shader_source_entries(&source, &entries, &output)
                .map_err(|e| e.to_string())
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_extract_scenario_scripts(&mut self, key: String, ctx: egui::Context) {
        if !self.model.active_game_is_campaign_evolved() {
            self.model.status = "Script extraction is only available for Campaign Evolved".to_owned();
            return;
        }
        let Some((source, entry)) = self.model.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract Scripts")
            .pick_folder()
        else {
            return;
        };
        self.model.status = format!("Extracting scripts from {}", entry.display_path);
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            extract_scenario_scripts(&source, &entry, &output).map_err(|e| e.to_string())
        });
    }

    /// Replace a scenario's `source files` block from a folder of `.hsc` files.
    ///
    /// Runs on the UI thread because it edits the loaded document: the tag is
    /// left **modified, not saved**, so the change goes through the same review
    /// and save path as any other edit. If the tag is not open yet it is read
    /// first — synchronously, since the result has to be mutated in the same
    /// step rather than handed to a worker.
    pub(in crate::app) fn import_scenario_scripts(&mut self, key: &str) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        if !self.model.active_game_is_campaign_evolved() {
            self.model.status = "Script import is only available for Campaign Evolved".to_owned();
            return;
        }
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            self.model.status = "Tag is no longer in the browser".to_owned();
            return;
        };
        if !is_scenario_group(entry.group_tag) {
            self.model.status = "Script import is only available for scenario tags".to_owned();
            return;
        }
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Import Scripts")
            .pick_folder()
        else {
            return;
        };

        if !self.model.kits[self.model.active].parsed_tags.contains_key(key) {
            let Some(source) = self.model.source().map(|source| source.source.clone()) else {
                self.model.status = "No tag source is loaded".to_owned();
                return;
            };
            self.model.status = format!("Loading {}", entry.display_path);
            match read_entry(&source, &entry) {
                Ok(tag) => {
                    self.model.kits[self.model.active]
                        .parsed_tags
                        .insert(key.to_owned(), TagDocument::clean(tag));
                }
                Err(error) => {
                    self.model.status = format!("Could not load {}: {error:#}", entry.display_path);
                    return;
                }
            }
        }

        let Some(document) = self.model.kits[self.model.active].parsed_tags.get_mut(key) else {
            self.model.status = "Load the tag before importing scripts".to_owned();
            return;
        };
        match replace_scenario_scripts(&mut document.tag, &folder) {
            Ok(message) => {
                document.dirty.touch();
                self.kit_and_view(self.model.active).open_tag_pane(key);
                self.model.kits[self.model.active].selected_key = Some(key.to_owned());
                self.model.status = format!("{message} (unsaved)");
            }
            // A failed read leaves the block untouched — `replace_scenario_scripts`
            // reads the whole folder before it clears anything.
            Err(error) => self.model.status = format!("Could not import scripts: {error:#}"),
        }
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_extract_hlsl_include_source(&mut self, key: String, ctx: egui::Context) {
        let Some((source, entry)) = self.model.export_context(&key) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract HLSL Include")
            .pick_folder()
        else {
            return;
        };
        self.model.status = format!("Extracting HLSL include from {}", entry.display_path);
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            extract_hlsl_include_source(&source, &entry, &output).map_err(|e| e.to_string())
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_extract_hlsl_include_folder(
        &mut self,
        keys: Vec<String>,
        ctx: egui::Context,
    ) {
        let Some(source_data) = self.model.source() else {
            return;
        };
        let entries = entries_for_keys(source_data, &keys);
        if entries.is_empty() {
            self.model.status = "No HLSL includes found in folder".to_owned();
            return;
        }
        let source = source_data.source.clone();
        let Some(output) = rfd::FileDialog::new()
            .set_title("Extract HLSL Includes")
            .pick_folder()
        else {
            return;
        };
        self.model.status = format!("Extracting {} HLSL include(s)", entries.len());
        let tx = self.tx.clone();
        spawn_export(&tx, &ctx, move || {
            extract_hlsl_include_entries(&source, &entries, &output).map_err(|e| e.to_string())
        });
    }


}

/// Create the directory a mod's files are about to be written into.
///
/// This is what creates the game's `~mods` on a first export: the default
/// destination is inside it, and nothing else in the app makes it.
pub(in crate::app) fn ensure_export_directory(output: &Path) -> Result<(), String> {
    let Some(directory) = output.parent() else {
        return Ok(());
    };
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))
}

/// The entries an extraction covers, resolved by one function so the count shown
/// in the confirmation and the set handed to the worker cannot disagree.
///
/// `Container` is the only location with a shipped payload to read, so both
/// scopes filter on it — a folder holding nothing but tags authored this session
/// resolves to empty here rather than starting a run that writes no files.
///
/// Borrows rather than clones: the confirmation only needs to count, and cloning
/// twelve thousand entries to call `.len()` on them is a waste the user pays for
/// in the gap between picking a folder and seeing the dialog.
pub(in crate::app) fn container_dump_entries<'a>(
    entries: &'a [TagEntry],
    scope: &ContainerDumpScope,
) -> Vec<&'a TagEntry> {
    let wanted = match scope {
        ContainerDumpScope::AllShipped => None,
        ContainerDumpScope::Folder { keys, .. } => {
            Some(keys.iter().map(String::as_str).collect::<HashSet<_>>())
        }
    };
    entries
        .iter()
        .filter(|entry| matches!(entry.location, TagEntryLocation::Container { .. }))
        .filter(|entry| {
            wanted
                .as_ref()
                .is_none_or(|keys| keys.contains(entry.key.as_str()))
        })
        .collect()
}

#[cfg(test)]
mod container_folder_extract_tests;

impl Model {
    pub(in crate::app) fn export_context(&self, key: &str) -> Option<(TagSource, TagEntry)> {
        let source = self.source()?.source.clone();
        let entry = self.entry_for_key(key)?.clone();
        Some((source, entry))
    }
}
