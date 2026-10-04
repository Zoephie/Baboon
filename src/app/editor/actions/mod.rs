//! What the tag editor asks of the app between frames: Campaign Evolved sound
//! bindings, model textures, TSV paste, field docs, and confirming destructive
//! block edits.

use super::*;

impl Baboon {
    /// Resolve (and cache) the Wwise media a Campaign Evolved `sound` tag binds
    /// to. Returns `None` for every other game and source kind.
    ///
    /// Resolution walks package imports and parses several cooked packages, so
    /// the result is memoized per tag key — the sound panel asks for it every
    /// frame. An unbound tag caches an empty binding, so stubs are not re-walked.
    pub(in crate::app) fn ce_sound_binding(
        &mut self,
        kit_index: usize,
        tag_key: &str,
        entry: &TagEntry,
    ) -> Option<std::sync::Arc<crate::core::source::ce_audio::CeSoundBinding>> {
        use crate::core::source::ce_audio;

        if !crate::app::editor::is_sound_group(entry.group_tag) {
            return None;
        }
        if let Some(hit) = self.kits[kit_index].caches.ce_sound_bindings.get(tag_key) {
            return Some(hit.clone());
        }

        let TagEntryLocation::Container { rel_path, .. } = &entry.location else {
            return None;
        };
        let package = ce_audio::tag_package_for_rel_path(rel_path)?;
        self.ce_binding_for_package(kit_index, tag_key, &package)
    }

    /// The Wwise binding of a `sound` tag reached by *reference* rather than by
    /// browser entry — a `sound_looping` track's component, a dialogue
    /// vocalization. Resolves the reference against the mounted containers and
    /// then follows the same walk, memoized under the same per-kit cache.
    pub(in crate::app) fn ce_sound_binding_for_ref(
        &mut self,
        kit_index: usize,
        group_tag: u32,
        reference: &str,
    ) -> Option<std::sync::Arc<crate::core::source::ce_audio::CeSoundBinding>> {
        use crate::core::source::ce_audio;

        let Some(TagSource::IoStoreContainerSet { index, .. }) =
            self.kits[kit_index].source.as_ref().map(|s| &s.source)
        else {
            return None;
        };
        let (_, rel_path) = index.lookup(group_tag, reference)?;
        let package = ce_audio::tag_package_for_rel_path(rel_path)?;
        let cache_key = format!("ref:{package}");
        self.ce_binding_for_package(kit_index, &cache_key, &package)
    }

    /// Walk one cooked package out to its Wwise media, memoized under
    /// `cache_key` in this kit. Shared by both entry points above.
    pub(in crate::app) fn ce_binding_for_package(
        &mut self,
        kit_index: usize,
        cache_key: &str,
        package: &str,
    ) -> Option<std::sync::Arc<crate::core::source::ce_audio::CeSoundBinding>> {
        use crate::core::source::ce_audio;

        if let Some(hit) = self.kits[kit_index].caches.ce_sound_bindings.get(cache_key) {
            return Some(hit.clone());
        }

        // Checked before the usmap is parsed, as it always has been: a non-CE
        // source must bail out without paying for the bundled reflection data.
        // The `matches!` ends its borrow immediately, which the destructure
        // below could not do across the `ce_usmap` assignment.
        if !matches!(
            self.kits[kit_index].source.as_ref().map(|s| &s.source),
            Some(TagSource::IoStoreContainerSet { .. })
        ) {
            return None;
        }
        if self.ce_usmap.is_none() {
            match blam_tags::iostore::usmap::Usmap::meteorite() {
                Ok(u) => self.ce_usmap = Some(std::sync::Arc::new(u)),
                Err(err) => {
                    eprintln!("campaign evolved: could not parse bundled usmap: {err}");
                    return None;
                }
            }
        }
        let usmap = self.ce_usmap.clone()?;

        let Some(TagSource::IoStoreContainerSet {
            root,
            containers,
            packages,
            ..
        }) = self.kits[kit_index].source.as_ref().map(|s| &s.source)
        else {
            return None;
        };

        // The pak set is needed for events whose media is cooked inside a
        // SoundBank. A decode worker holds the lock only while it reads a
        // file's bytes.
        let store = self.audio.ce_media.clone();
        let mut store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let binding = std::sync::Arc::new(ce_audio::resolve_sound_binding(
            containers,
            packages,
            &usmap,
            package,
            Some((root.as_path(), &mut store)),
        ));
        self.kits[kit_index]
            .caches.ce_sound_bindings
            .insert(cache_key.to_owned(), binding.clone());
        Some(binding)
    }

    /// Drain a queued referenced-sound click from a container source: resolve
    /// the reference's own Wwise binding, then queue the same playback or
    /// extraction the primary sound player would.
    pub(in crate::app) fn process_ce_sound_ref(&mut self) {
        let Some((kit_id, tab_key, request)) = self.editor.pending_ce_sound_ref.take() else {
            return;
        };
        let Some(kit_index) = self.kit_index(kit_id) else {
            return;
        };
        let paks_root = match self.kits[kit_index].source.as_ref().map(|s| &s.source) {
            Some(TagSource::IoStoreContainerSet { root, .. }) => root.clone(),
            _ => return,
        };
        let Some(binding) =
            self.ce_sound_binding_for_ref(kit_index, request.group_tag, &request.reference)
        else {
            self.status = format!("{} not found in mounted containers", request.label);
            return;
        };
        if binding.is_empty() {
            self.status = format!("{} has no audio bound", request.label);
            return;
        }

        let language = binding.language_to_show(self.audio.language.as_deref());
        let media: Vec<crate::core::source::ce_audio::CeSoundMedia> = binding
            .media_for_language(&language)
            .into_iter()
            .cloned()
            .collect();
        if !request.extract {
            // Play the first permutation, matching the loose-folder player.
            let Some(first) = media.into_iter().next() else {
                return;
            };
            self.audio.pending.push_back(crate::app::audio::SoundRequest {
                owner: Some(crate::app::audio::SoundOwner {
                    kit: kit_id,
                    key: tab_key,
                }),
                clip: request.clip.clone(),
                preview: request.preview,
                action: crate::app::audio::SoundAction::PlayCeMedia {
                    paks_root,
                    label: format!("{} \u{00B7} {}", request.label, first.display_name()),
                    media: Box::new(first),
                },
            });
            return;
        }
        let Some(base) = rfd::FileDialog::new()
            .set_title(format!("Extract {}", request.label))
            .pick_folder()
        else {
            return;
        };
        let items = media
            .into_iter()
            .map(|m| crate::app::export::sound_extract::ExtractItem {
                out_path: base.join(format!(
                    "{}.wav",
                    crate::app::export::sound_extract::sanitize_component(&m.display_name())
                )),
                source: crate::app::export::sound_extract::ExtractSource::CeMedia {
                    paks_root: paks_root.clone(),
                    media: Box::new(m),
                },
            })
            .collect();
        self.export.pending_sound_extract = Some(crate::app::export::sound_extract::ExtractRequest {
            items,
            tags_root: None,
            label: request.label,
        });
    }

    /// Start resolving a loaded model's materials to textures, if it needs it.
    ///
    /// Idempotent and cheap to call every frame: it only spawns when a model is
    /// loaded, a textured mode is on, its textures are still absent, and no job for it
    /// is already running.
    pub(in crate::app) fn maybe_request_model_textures(
        &mut self,
        kit_index: usize,
        key: &str,
        ctx: &egui::Context,
    ) {
        let Some(state) = self.kits[kit_index].caches.model_previews.get(key) else {
            return;
        };
        if !state.render_mode.uses_textures() || state.textures_pending {
            return;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return;
        };
        if data.textures.is_some() || data.preview.materials.is_empty() {
            return;
        }
        let Some(source) = self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.source.clone())
        else {
            return;
        };
        let materials = data.preview.materials.clone();
        let textures_id = data.textures_id;
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
        };
        if let Some(state) = self.kits[kit_index].caches.model_previews.get_mut(key) {
            state.textures_pending = true;
        }

        let (key, panic_key) = (key.to_owned(), key.to_owned());
        spawn_worker(
            &self.tx,
            ctx,
            move || WorkerMessage::ModelTexturesResolved {
                stamp,
                key,
                textures_id,
                textures: resolve_model_textures(&source, &materials),
            },
            // No textures: the preview draws untextured, and stops waiting.
            move |_| WorkerMessage::ModelTexturesResolved {
                stamp,
                key: panic_key,
                textures_id,
                textures: Vec::new(),
            },
        );
    }

    pub(in crate::app) fn handle_model_textures_resolved(
        &mut self,
        stamp: KitStamp,
        key: String,
        textures_id: u64,
        textures: Vec<MaterialTextures>,
    ) -> bool {
        let Some(kit_index) = self.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.resolve_stamp(stamp).is_none();
        let Some(state) = self.kits[kit_index].caches.model_previews.get_mut(&key) else {
            return true;
        };
        // The in-flight marker is cleared before the staleness check: a result
        // dropped for a generation bump used to leave it set, and the preview
        // then waited for it for good ("Loading shaders…", repainting every
        // frame).
        state.textures_pending = false;
        if stale {
            return true;
        }
        let Some(Ok(data)) = state.data.as_mut() else {
            return true;
        };
        // The model was reloaded while this ran, and these textures are
        // indexed against the materials of the old one.
        if data.textures_id != textures_id {
            return true;
        }
        data.textures = Some(std::sync::Arc::new(textures));
        false
    }

    /// Apply pasted TSV (header row = field names, one data row per element) to
    /// the target block's EXISTING elements, cell-by-cell via `apply_field_edit`.
    /// Rows beyond the current element count are reported and ignored (no
    /// structural changes — fully covered by undo). Returns a status summary.
    pub(in crate::app) fn apply_tsv_paste(&mut self) {
        // The document is looked up in the active kit below, and two workspaces
        // of the same game share a key space, so a paste answered after a
        // switch could land in the wrong game's tag rather than simply missing.
        let Some(kit) = self.editor.tsv_paste.as_ref().map(|paste| paste.kit) else {
            return;
        };
        if !self.focus_navigation_kit(kit) {
            self.set_tsv_paste_status("The workspace this paste came from is closed.");
            return;
        }
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        let Some(paste) = self.editor.tsv_paste.as_ref() else {
            return;
        };
        let tag_key = paste.tag_key.clone();
        let block_path = paste.block_path.clone();
        let text = paste.text.clone();

        let Some(doc) = self.kits[self.active].parsed_tags.get_mut(&tag_key) else {
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
        let applied_rows = data_rows.saturating_sub(skipped_rows);
        let active = self.active;
        let ops = DeferredOps {
            pending: edits,
            ..DeferredOps::default()
        };
        let Some(applied) = self.apply_doc_ops(active, &tag_key, "Paste TSV", ops, UndoStep::Own)
        else {
            return;
        };
        self.invalidate_tag_caches_in(active, &tag_key);

        let summary =
            tsv_paste_summary(&applied.outcomes, applied_rows, skipped_rows, element_count);
        self.status = summary.clone();
        self.set_tsv_paste_status(&summary);
    }

    pub(in crate::app) fn set_tsv_paste_status(&mut self, message: &str) {
        if let Some(paste) = self.editor.tsv_paste.as_mut() {
            paste.status = Some(message.to_owned());
        }
    }

    /// The documentation overlay (help/units + explanations) for a group,
    /// parsed once from its definition JSON and cached. `None` when the
    /// definitions can't be located (e.g. non-loose sources).
    /// Documentation overlay for `entry`'s group, resolved against `kit_index`
    /// rather than the active kit — in a split the two panes can be different
    /// games, whose definitions and group naming differ.
    pub(in crate::app) fn def_docs_for_entry(
        &mut self,
        kit_index: usize,
        entry: &TagEntry,
    ) -> Option<Rc<DefDocs>> {
        let source = self.kits[kit_index].source.as_ref()?;
        let root = match &source.source {
            TagSource::LooseFolder {
                definitions_root, ..
            } => definitions_root.clone(),
            _ => return None,
        };
        let game = source.game.clone()?;
        let group = self.kits[kit_index]
            .names
            .name_for(entry.group_tag)
            .or_else(|| group_tag_to_extension(entry.group_tag))?
            .to_owned();
        // Cache key is the group's own JSON path; the docs themselves merge the
        // whole `parent_tag` inheritance chain (object-family fields live in
        // parent files).
        let path = root.join(game.as_str()).join(format!("{group}.json"));
        if let Some(docs) = self.help.def_docs_cache.get(&path) {
            return Some(docs.clone());
        }
        let docs = Rc::new(build_def_docs(&root, game, &group));
        self.help.def_docs_cache.insert(path, docs.clone());
        Some(docs)
    }

    /// Render the block delete/delete-all confirmation modal (if pending) and
    /// apply the op on confirm.
    pub(in crate::app) fn handle_block_confirm(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.editor.block_confirm.as_ref() else {
            return;
        };
        // The op is applied to the active kit's document, and two workspaces of
        // the same game share a key space, so a confirmation answered after a
        // switch could delete from the wrong game's tag.
        let confirm_kit = confirm.kit;
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
            let routed = confirm_kit.is_some_and(|kit| self.focus_navigation_kit(kit));
            if routed && self.refuse_read_only_edit(self.active) {
                self.editor.block_confirm = None;
                return;
            }
            if let Some(confirm) = self.editor.block_confirm.take()
                && routed
            {
                let deletes_model_variant = confirm.path == "variants"
                    && matches!(confirm.kind, BlockOpKind::Delete(_))
                    && self.kits[self.active]
                        .parsed_tags
                        .get(&confirm.tag_key)
                        .is_some_and(|doc| doc.tag.header.group_tag.to_be_bytes() == *b"hlmt");
                let ops = DeferredOps {
                    block_ops: vec![BlockOp {
                        path: confirm.path,
                        kind: confirm.kind,
                    }],
                    ..DeferredOps::default()
                };
                let active = self.active;
                let applied =
                    self.apply_doc_ops(active, &confirm.tag_key, "Block edit", ops, UndoStep::Own);
                let refresh_model_preview = deletes_model_variant
                    && applied.is_some_and(|applied| applied.status.is_some());
                if refresh_model_preview
                    && let Some(preview) = self.kits[self.active]
                        .caches.model_previews
                        .get_mut(&confirm.tag_key)
                {
                    preview.selected_variant = None;
                    preview.invalidate_load();
                }
            }
        } else if do_cancel {
            self.editor.block_confirm = None;
        }
    }
}

/// What a TSV paste did, counted from the per-cell outcomes. It used to count
/// the cells it tried, so a paste whose cells all failed to parse still said
/// every one of them was pasted.
pub(in crate::app) fn tsv_paste_summary(
    outcomes: &[FieldEditOutcome],
    applied_rows: usize,
    skipped_rows: usize,
    element_count: usize,
) -> String {
    let failed: Vec<&FieldEditOutcome> = outcomes
        .iter()
        .filter(|outcome| outcome.result.is_err())
        .collect();
    let pasted = outcomes.len() - failed.len();
    let mut summary = if failed.is_empty() {
        format!("Pasted {pasted} cell(s) across {applied_rows} row(s)")
    } else {
        format!(
            "Pasted {pasted} of {} cell(s) across {applied_rows} row(s)",
            outcomes.len()
        )
    };
    if let Some(first) = failed.first() {
        let error = first
            .result
            .as_ref()
            .err()
            .map(String::as_str)
            .unwrap_or("");
        summary.push_str(&format!(
            " — {} failed; first: {} = \"{}\": {error}",
            failed.len(),
            first.path,
            first.input
        ));
    }
    if skipped_rows > 0 {
        summary.push_str(&format!(
            " — {skipped_rows} extra row(s) ignored (block has {element_count} elements; add more first)"
        ));
    }
    summary
}

#[cfg(test)]
mod tsv_paste_tests;

#[cfg(test)]
mod tsv_paste_summary_tests;
