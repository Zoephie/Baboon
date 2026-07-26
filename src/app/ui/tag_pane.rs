//! The single tag-editor pane: header, per-tag bars, the schema-driven editor, and edit application.
//! It owns one document's presentation and deferred-op application; layout of panes and source I/O belong elsewhere.

use super::*;

impl Baboon {
    /// Renders one open tag as a self-contained pane.
    ///
    /// This is the only place a tag document is rendered. Every layout that
    /// shows a tag — the docked editor, a popped-out window, and (later) each
    /// tile in a split — calls this with a distinct `scope`, which salts every
    /// widget id underneath (see [`FieldEditContext::widget_id`]) so the same
    /// tag shown twice keeps independent scroll/focus/collapse state while
    /// sharing one underlying [`TagDocument`].
    ///
    /// The document is taken out of `parsed_tags` for the duration and put back
    /// before returning: owning it outright is what lets the rest of the body
    /// borrow `self`'s other fields freely, which the deferred-op plumbing
    /// needs. A caller that renders a chrome row of its own (a dock button, a
    /// tile tab bar) draws it before calling.
    pub(in crate::app) fn draw_tag_pane(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
        entry: &TagEntry,
        scope: &str,
        show_keyword_bar: bool,
    ) {
        let key = entry.key.clone();
        draw_entry_header(ui, entry, &self.kits[kit_index].names);
        self.draw_scenario_launcher_buttons(ui, kit_index, entry);
        if show_keyword_bar {
            self.draw_keyword_bar(ui, kit_index, &key);
        }

        // "Search fields" collapses the editor to matching blocks.
        // Not offered for shader/sound tags (their own surfaces).
        let supports_field_search = supports_field_search(entry);
        if supports_field_search {
            self.draw_field_search_bar(ui, kit_index, &key);
        }

        // Documentation overlay and the Campaign Evolved Wwise binding are both
        // resolved through `&mut self` methods, so they must be taken before any
        // long-lived field borrows below.
        let picker_was_open = self.tag_reference_picker.is_some();
        let def_docs = self.def_docs_for_entry(kit_index, entry);
        let ce_sound = self.ce_sound_binding(kit_index, &key, entry);

        let Some(mut doc) = self.kits[kit_index].parsed_tags.remove(&key) else {
            if self.kits[kit_index].loading_tags.contains(&key) {
                ui.label("Loading tag data...");
            } else {
                ui.label("Select the tag again to load it.");
            }
            return;
        };

        let kit = &mut self.kits[kit_index];
        let kit_id = kit.id;
        let source = kit.source.as_ref();
        let names = &kit.names;

        let mut pending = Vec::new();
        let mut block_ops = Vec::new();
        let mut shader_ops = Vec::new();
        let mut shader_param_ops = Vec::new();
        let mut h2_shader_param_ops = Vec::new();
        let mut function_data_ops = Vec::new();
        let mut model_variant_ops = Vec::new();
        let mut color_request = None;
        let mut function_request = None;
        let mut block_clip_request = None;
        let mut bitmap_reimport = None;
        let mut tsv_paste_request = None;

        // Taken rather than read: it is a one-shot, and egui remembers the
        // state each container lands in, so forcing it for a single frame is
        // what makes it stick.
        let expand_all = kit.pending_expand.remove(&key);
        let field_filter = compute_pending_field_filter(
            &doc.tag,
            supports_field_search,
            &key,
            &kit.field_search,
            &mut kit.field_search_applied,
        );
        let sound_volume = self.audio.volume();
        let expert_mode = self.expert_mode;
        // Borrow the kit's source as a plain field rather than through
        // `source()`: a method borrows all of `self`, and the context below
        // needs `&mut` on a dozen sibling fields. Going through `self.kits[i]`
        // directly is what lets the borrow checker see them as disjoint.
        let ce_paks_root = source.and_then(|s| match &s.source {
            TagSource::IoStoreContainerSet { root, .. } => Some(root.as_path()),
            _ => None,
        });
        let mut edit_context = FieldEditContext {
            view_scope: scope,
            tag_key: &key,
            group_tag: entry.group_tag,
            root: Some(doc.tag.root()),
            game: source.and_then(|source| source.game.as_deref()),
            definitions_root: source.and_then(|source| match &source.source {
                TagSource::LooseFolder {
                    definitions_root, ..
                } => Some(definitions_root.as_path()),
                _ => None,
            }),
            names: Some(names),
            tags_root: source.and_then(|source| match &source.source {
                TagSource::LooseFolder { root, .. } => Some(root.as_path()),
                _ => None,
            }),
            tag_reference_catalog: source
                .and_then(|source| tag_reference_catalog_for_source(source, expert_mode)),
            tag_reference_picker: &mut self.tag_reference_picker,
            status: Some(&mut self.status),
            editable: is_editable_tag(entry, &doc.tag),
            show_block_sizes: self.show_block_sizes,
            buffers: &mut kit.edit_buffers,
            pending: &mut pending,
            block_ops: &mut block_ops,
            block_confirm: &mut self.block_confirm,
            open_request: &mut self.pending_open,
            sound_play_request: &mut self.audio.pending,
            sound_status: self.audio.status.as_deref(),
            sound_volume,
            sound_extract_request: &mut self.pending_sound_extract,
            sound_language: self.audio.language.as_deref(),
            ce_sound: ce_sound.as_deref(),
            ce_paks_root,
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
            // Only the pane being navigated to sees the request. The scroll
            // gate downstream matches on the field path alone, so an unfiltered
            // nav scrolled every pane whose tag happened to have a field at the
            // same path — which, between two tags of the same group, is most of
            // them. Splitting a tag view is what exposed this.
            field_nav: self
                .field_nav
                .as_ref()
                .filter(|nav| nav.kit == kit_id && nav.tag_key == key),
            expand_all,
        };

        if is_bitmap_tag(entry) {
            let preview = kit.bitmap_previews.entry(key.clone()).or_default();
            draw_bitmap_tag(
                ui,
                ctx,
                &doc.tag,
                entry,
                names,
                &mut self.color_popup,
                preview,
                self.expert_mode,
                &mut edit_context,
            );
        } else {
            let mut local_model_preview;
            let model_preview = if is_model_group(entry.group_tag, names) {
                kit.model_previews.entry(key.clone()).or_default()
            } else {
                local_model_preview = ModelPreviewState::default();
                &mut local_model_preview
            };
            draw_tag(
                ui,
                &doc.tag,
                entry,
                names,
                source.map(|source| &source.source),
                &mut kit.rmdf_cache,
                &mut kit.rmop_cache,
                &mut self.color_popup,
                &mut self.function_popup,
                model_preview,
                &mut self.model_preview_size,
                self.expert_mode,
                &mut edit_context,
            );
        }

        // Snapshot for undo before a mutating batch. Coalesces continuous edits
        // into one entry; closes the window on frames with no edits.
        // Every deferred op this pane collected, including the kinds the undo
        // window below deliberately ignores. Used only to decide whether the
        // frame needs redrawing.
        let mutated = !pending.is_empty()
            || !block_ops.is_empty()
            || !shader_ops.is_empty()
            || !shader_param_ops.is_empty()
            || !h2_shader_param_ops.is_empty()
            || !function_data_ops.is_empty()
            || !model_variant_ops.is_empty();
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
        // Per-edit outcomes, from upstream: a draft whose value applied cleanly
        // is marked clean, while one the parser rejected keeps the text the
        // user typed instead of snapping back to the old value.
        let applied = apply_pending_edits(&mut doc.tag, pending, &mut doc.dirty);
        kit.edit_buffers
            .accept_successful_edits(&key, &applied.outcomes);
        if let Some(status) = applied.status {
            self.status = status;
        }
        if let Some(status) = apply_block_ops(&mut doc.tag, block_ops, &mut doc.dirty) {
            self.status = status;
        }
        if let Some(status) = apply_shader_ops(&mut doc.tag, shader_ops, &mut doc.dirty) {
            self.status = status;
        }
        if let Some(status) = apply_shader_param_ops(&mut doc.tag, shader_param_ops, &mut doc.dirty)
        {
            self.status = status;
        }
        if let Some(status) =
            apply_h2_shader_param_ops(&mut doc.tag, h2_shader_param_ops, &mut doc.dirty)
        {
            self.status = status;
        }
        if let Some(status) = apply_function_data_ops(&mut doc.tag, function_data_ops, &mut doc.dirty)
        {
            self.status = status;
        }
        if let Some(status) = apply_model_variant_ops(&mut doc.tag, model_variant_ops, &mut doc.dirty)
        {
            self.status = status;
            if let Some(preview) = kit.model_previews.get_mut(&key) {
                preview.loaded_key = None;
                preview.data = None;
            }
        }
        // A color swatch was clicked: open the shared picker. Each popup
        // records the kit it was opened from, so confirming it later edits
        // this document rather than whichever kit is active by then.
        if let Some(popup) = color_request {
            self.color_popup = Some(popup);
            self.color_popup_kit = Some(kit_id);
        }
        if let Some(popup) = function_request {
            self.function_popup = Some(popup);
            self.function_popup_kit = Some(kit_id);
        }
        // The reference picker is opened from inside the field renderer rather
        // than hoisted here, so it is stamped by noticing it appear.
        if !picker_was_open && self.tag_reference_picker.is_some() {
            self.tag_reference_picker_kit = Some(kit_id);
        }
        // And a block confirmation, raised the same way. Stamping only an
        // unstamped one leaves a confirmation another pane raised alone.
        if let Some(confirm) = self.block_confirm.as_mut() {
            confirm.kit.get_or_insert(kit_id);
        }
        // Element(s) were copied: stash them on the clipboard.
        if let Some(clip) = block_clip_request {
            self.status = format!(
                "Copied {} '{}' element(s)",
                clip.elements.len(),
                clip.label
            );
            self.block_clipboard = Some(clip);
        }
        // "Paste TSV…" was chosen: open the import window.
        if let Some(req) = tsv_paste_request {
            self.tsv_paste = Some(TsvPasteState {
                kit: kit_id,
                tag_key: key.clone(),
                block_path: req.block_path,
                block_label: req.block_label,
                element_count: req.element_count,
                text: String::new(),
                status: None,
            });
        }

        kit.parsed_tags.insert(key, doc);
        // These ops are applied *after* the pane has been drawn, so the frame
        // on screen still shows the tag as it was before the edit. egui only
        // redraws when new input arrives, so nothing here is guaranteed to be
        // visible until something else happens to wake the UI -- an added block
        // element missing from that block's own instance selector, for one.
        if mutated {
            ctx.request_repaint();
        }

        if let Some(key) = bitmap_reimport {
            // Resolves its source and entry against the active kit, and runs an
            // external tool that rewrites the bitmap on disk. This pane's kit is
            // the one being asked, so make it active first rather than trusting
            // press-activation to have already landed this frame.
            self.active = kit_index;
            self.begin_reimport_bitmap(key, ctx.clone());
        }
    }
}
