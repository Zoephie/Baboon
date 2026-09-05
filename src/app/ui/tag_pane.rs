//! The single tag-editor pane: header, per-tag bars, the schema-driven editor, and edit application.
//! It owns one document's presentation and deferred-op application; layout of panes and source I/O belong elsewhere.

use super::*;

const TAG_HEADER_ICON_SIZE: f32 = 32.0;
const TAG_HEADER_WIDE_BREAKPOINT: f32 = 600.0;
const TAG_HEADER_KEYWORDS_INLINE_BREAKPOINT: f32 = 1160.0;
const TAG_HEADER_ACTIONS_SINGLE_ROW_BREAKPOINT: f32 = 1180.0;
const TAG_HEADER_COMMON_ACTIONS_WIDTH: f32 = 205.0;
const TAG_HEADER_DYNAMIC_ACTIONS_WIDTH: f32 = 285.0;

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
        self.draw_responsive_tag_header(ui, ctx, kit_index, entry, show_keyword_bar);

        // "Search fields" collapses the editor to matching blocks.
        // Not offered for shader/sound tags (their own surfaces).
        let supports_field_search = supports_field_search(entry);
        if supports_field_search {
            let jump_match_count = self.kits[kit_index]
                .field_search
                .get(&key)
                .and_then(|query| {
                    self.kits[kit_index]
                        .parsed_tags
                        .get(&key)
                        .map(|doc| field_label_matches(&doc.tag, query).len())
                })
                .unwrap_or(0);
            if self.draw_field_search_bar(ui, kit_index, &key, jump_match_count)
            {
                let query = self.kits[kit_index]
                    .field_search
                    .get(&key)
                    .cloned()
                    .unwrap_or_default();
                let matches = self.kits[kit_index]
                    .parsed_tags
                    .get(&key)
                    .map(|doc| field_label_matches(&doc.tag, &query))
                    .unwrap_or_default();
                let cursor_id = egui::Id::new((
                    "field_search_cursor",
                    self.kits[kit_index].id.0,
                    key.as_str(),
                ));
                let selected = ctx.data_mut(|data| {
                    let mut cursor = data
                        .get_temp::<FieldSearchCursor>(cursor_id)
                        .unwrap_or_default();
                    let selected = cursor.advance(&query, matches.len());
                    data.insert_temp(cursor_id, cursor);
                    selected
                });
                if let Some(field_path) = selected.and_then(|index| matches.get(index)).cloned() {
                    self.navigate_to_field(ctx, &key, &field_path);
                    // Search matches labels, including container labels. The block
                    // header path is consumed by container rendering; scalar rows
                    // use the exact field target installed by navigate_to_field.
                    ctx.data_mut(|data| data.insert_temp(jump_target_id(), field_path));
                }
            }
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
        let mut ce_sound_ref_request = None;

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
            ce_sound_ref_request: &mut ce_sound_ref_request,
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
            nested_default: self.nested_default,
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
            let model_preview = if is_previewable_geometry_group(entry.group_tag, names) {
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

        let field_search_block_jump = ctx.data_mut(|data| {
            let id = field_search_block_jump_id(scope, &key);
            let request = data.get_temp::<String>(id);
            data.remove::<String>(id);
            request
        });

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
        // A referenced sound was played/extracted from a container source. It
        // is stamped with this kit because resolving it needs that kit's
        // containers, not whichever one happens to be active by the drain.
        if let Some(request) = ce_sound_ref_request {
            self.pending_ce_sound_ref = Some((kit_id, request));
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

        if field_search_block_jump.is_some() {
            // Preserve field_search_applied until the next render so clearing
            // the query produces the normal one-shot restore-defaults pass.
            kit.field_search.entry(key.clone()).or_default().clear();
        }
        kit.parsed_tags.insert(key.clone(), doc);
        // These ops are applied *after* the pane has been drawn, so the frame
        // on screen still shows the tag as it was before the edit. egui only
        // redraws when new input arrives, so nothing here is guaranteed to be
        // visible until something else happens to wake the UI -- an added block
        // element missing from that block's own instance selector, for one.
        if mutated {
            ctx.request_repaint();
        }

        if let Some(block_path) = field_search_block_jump {
            self.navigate_to_field(ctx, &key, &block_path);
            ctx.data_mut(|data| data.insert_temp(jump_target_id(), block_path));
        }

        if let Some(key) = bitmap_reimport {
            // Resolves its source and entry against the active kit, and runs an
            // external tool that rewrites the bitmap on disk. This pane's kit is
            // the one being asked, so make it active first rather than trusting
            // press-activation to have already landed this frame.
            self.active = kit_index;
            self.begin_reimport_bitmap(key, ctx.clone());
        }
        // The model preview panel draws without `&mut Baboon`, so it cannot
        // start its own texture job. Started here instead, once the panel has
        // had its frame to load the geometry the textures belong to. The
        // collision/physics overlay build rides the same hook for the same
        // reason.
        self.maybe_request_model_textures(kit_index, &key, ctx);
        self.maybe_request_model_overlays(kit_index, &key, ctx);
        self.maybe_request_model_animations(kit_index, &key, ctx);
        self.maybe_request_model_animation_decode(kit_index, &key, ctx);
    }

    fn draw_responsive_tag_header(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
        entry: &TagEntry,
        show_keyword_bar: bool,
    ) {
        let available = ui.available_width();
        let wide = available >= TAG_HEADER_WIDE_BREAKPOINT;
        let keywords_inline = available >= TAG_HEADER_KEYWORDS_INLINE_BREAKPOINT;
        let actions_single_row = available >= TAG_HEADER_ACTIONS_SINGLE_ROW_BREAKPOINT;
        let has_dynamic_actions = entry.group_tag == u32::from_be_bytes(*b"scnr");
        let actions_stacked = has_dynamic_actions && !actions_single_row;
        let action_width = if has_dynamic_actions {
            if actions_stacked {
                TAG_HEADER_DYNAMIC_ACTIONS_WIDTH
            } else {
                TAG_HEADER_DYNAMIC_ACTIONS_WIDTH + 20.0 + TAG_HEADER_COMMON_ACTIONS_WIDTH
            }
        } else {
            TAG_HEADER_COMMON_ACTIONS_WIDTH
        };
        let left_width = if wide {
            (available - action_width - 20.0).max(200.0)
        } else {
            available
        };
        let title_height = if self.expert_mode { 48.0 } else { TAG_HEADER_ICON_SIZE };
        let left_height = if !keywords_inline && show_keyword_bar {
            title_height + 10.0 + BUTTON_HEIGHT
        } else {
            title_height.max(BUTTON_HEIGHT)
        };
        let key = entry.key.clone();
        let (breadcrumbs, title) = tag_header_path_parts(&entry.display_path);

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            ui.allocate_ui_with_layout(
                Vec2::new(left_width, left_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 20.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let (icon_rect, _) = ui.allocate_exact_size(
                            Vec2::splat(TAG_HEADER_ICON_SIZE),
                            Sense::hover(),
                        );
                        paint_tag_icon_at(ui, Some(entry.group_tag), icon_rect);

                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(
                                RichText::new(breadcrumbs)
                                    .size(12.0)
                                    .color(subtle_dark()),
                            );
                            ui.label(
                                RichText::new(title).size(16.0).strong().color(text_dark()),
                            );
                            if self.expert_mode {
                                ui.label(
                                    RichText::new(group_label(
                                        &self.kits[kit_index].names,
                                        entry.group_tag,
                                    ))
                                    .size(11.0)
                                    .color(subtle_dark()),
                                );
                            }
                        });
                    });

                    if keywords_inline && show_keyword_bar {
                        self.draw_keyword_bar(ui, kit_index, &key);
                    }
                });
                if !keywords_inline && show_keyword_bar {
                    self.draw_keyword_bar(ui, kit_index, &key);
                }
                },
            );

            if wide {
                let action_height = if actions_stacked {
                    BUTTON_HEIGHT * 2.0 + 8.0
                } else {
                    BUTTON_HEIGHT
                };
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), action_height),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.set_height(action_height);
                        if actions_stacked {
                            ui.vertical(|ui| {
                                ui.set_height(action_height);
                                ui.spacing_mut().item_spacing.y = 8.0;
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| self.draw_scenario_launcher_buttons(ui, kit_index, entry),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        self.draw_tag_header_common_actions(
                                            ui, ctx, kit_index, entry,
                                        );
                                    },
                                );
                            });
                        } else {
                            self.draw_tag_header_common_actions(ui, ctx, kit_index, entry);
                            if has_dynamic_actions {
                                self.draw_scenario_launcher_buttons(ui, kit_index, entry);
                            }
                        }
                    },
                );
            }
        });

        if !wide {
            if entry.group_tag == u32::from_be_bytes(*b"scnr") {
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), BUTTON_HEIGHT),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| self.draw_scenario_launcher_buttons(ui, kit_index, entry),
                );
                ui.add_space(8.0);
            }
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), BUTTON_HEIGHT),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| self.draw_tag_header_common_actions(ui, ctx, kit_index, entry),
            );
        }
        ui.add_space(20.0);
        ui.separator();
    }

    fn draw_tag_header_common_actions(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
        entry: &TagEntry,
    ) {
        let key = entry.key.clone();
        let is_favorite = self.kits[kit_index]
            .active_favorite_entries
            .iter()
            .any(|favorite| favorite.key == key);
        let favorite_enabled = matches!(entry.location, TagEntryLocation::LooseFile(_));
        let mut action = None;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let menu = ui.menu_button("   ", |ui| {
                style_tag_context_menu(ui);
                if matches!(&entry.location, TagEntryLocation::Monolithic { .. })
                    && context_menu_button(ui, "Import into editing kit...").clicked()
                {
                    action = Some(BrowserAction::ImportCacheTagIntoKit { key: key.clone() });
                    ui.close_menu();
                }
                let can_rename = supports_rename_menu(entry);
                if supports_rename_menu(entry)
                    && context_menu_button(ui, "Rename").clicked()
                {
                    action = Some(BrowserAction::RenameTag(key.clone()));
                    ui.close_menu();
                }
                if supports_duplicate_menu(entry)
                    && context_menu_button(ui, "Duplicate").clicked()
                {
                    action = Some(BrowserAction::DuplicateTag(key.clone()));
                    ui.close_menu();
                }
                let delete_enabled = supports_delete_menu(
                    entry,
                    browser_deletable_keys(ui).as_deref(),
                );
                if ui
                    .add_enabled_ui(delete_enabled, |ui| context_menu_button(ui, "Delete"))
                    .inner
                    .clicked()
                {
                    action = Some(BrowserAction::DeleteTag(key.clone()));
                    ui.close_menu();
                }
                if ui
                    .add_enabled_ui(can_rename, |ui| context_menu_button(ui, "Move"))
                    .inner
                    .clicked()
                {
                    action = Some(BrowserAction::MoveTag(key.clone()));
                    ui.close_menu();
                }
                if supports_tag_extract_menu(entry.group_tag) {
                    ui.menu_button("Extract", |ui| {
                        if supports_tag_geometry_extraction(entry.group_tag)
                            && context_menu_button(ui, "Extract model geometry").clicked()
                        {
                            action = Some(BrowserAction::ExtractGeometry(key.clone()));
                            ui.close_menu();
                        }
                        if supports_bsp_geometry_extraction(entry.group_tag)
                            && context_menu_button(ui, "Extract BSP geometry").clicked()
                        {
                            action = Some(BrowserAction::ExtractGeometry(key.clone()));
                            ui.close_menu();
                        }
                        if supports_scenario_geometry_extraction(entry.group_tag)
                            && context_menu_button(
                                ui,
                                "Extract level geometry (one file per BSP)",
                            )
                            .clicked()
                        {
                            action = Some(BrowserAction::ExtractGeometry(key.clone()));
                            ui.close_menu();
                        }
                        if supports_particle_geometry_extraction(entry.group_tag)
                            && context_menu_button(
                                ui,
                                "Extract particle geometry (JMI + one JMS per object)",
                            )
                            .clicked()
                        {
                            action = Some(BrowserAction::ExtractGeometry(key.clone()));
                            ui.close_menu();
                        }
                        if supports_animation_extraction(entry.group_tag)
                            && context_menu_button(ui, "Extract animations").clicked()
                        {
                            action = Some(BrowserAction::ExtractAnimation(key.clone()));
                            ui.close_menu();
                        }
                        if supports_tag_import_info_extraction(entry.group_tag)
                            && context_menu_button(ui, "Extract import-info").clicked()
                        {
                            action = Some(BrowserAction::ExtractImportInfo(key.clone()));
                            ui.close_menu();
                        }
                        if is_bitmap_group(entry.group_tag)
                            && context_menu_button(ui, "Extract bitmap images...").clicked()
                        {
                            action = Some(BrowserAction::ExtractBitmap(key.clone()));
                            ui.close_menu();
                        }
                        if is_material_shader_group(entry.group_tag)
                            && context_menu_button(ui, "Extract source shaders...").clicked()
                        {
                            action = Some(BrowserAction::ExtractMaterialShaderSources(key.clone()));
                            ui.close_menu();
                        }
                        if is_hlsl_include_group(entry.group_tag)
                            && context_menu_button(ui, "Extract HLSL include...").clicked()
                        {
                            action = Some(BrowserAction::ExtractHlslIncludeSource(key.clone()));
                            ui.close_menu();
                        }
                    });
                }
                let campaign_evolved = self.kits[kit_index]
                    .source
                    .as_ref()
                    .and_then(|source| source.game.as_deref())
                    == Some(CAMPAIGN_EVOLVED_GAME);
                let scenario_scripts = is_scenario_group(entry.group_tag) && campaign_evolved;
                if is_embedded_tag_entry(entry) || scenario_scripts {
                    context_menu_separator(ui);
                    if is_embedded_tag_entry(entry)
                        && context_menu_button(ui, "Extract raw tag...").clicked()
                    {
                        action = Some(BrowserAction::ExtractRaw(key.clone()));
                        ui.close_menu();
                    }
                    if scenario_scripts
                        && context_menu_button(ui, "Extract scripts...").clicked()
                    {
                        action = Some(BrowserAction::ExtractScenarioScripts(key.clone()));
                        ui.close_menu();
                    }
                    if scenario_scripts
                        && context_menu_button(ui, "Import scripts...").clicked()
                    {
                        action = Some(BrowserAction::ImportScenarioScripts(key.clone()));
                        ui.close_menu();
                    }
                }
                context_menu_separator(ui);
                if context_menu_button(ui, "Open with File Explorer").clicked() {
                    action = Some(BrowserAction::OpenInExplorer(key.clone()));
                    ui.close_menu();
                }
                if context_menu_button(ui, "Copy Tag Path").clicked() {
                    action = Some(BrowserAction::CopyTagName(key.clone()));
                    ui.close_menu();
                }
                if context_menu_button(ui, "Find Tag References...").clicked() {
                    action = Some(BrowserAction::FindReferences(key.clone()));
                    ui.close_menu();
                }
                if context_menu_button(ui, "Explore references...").clicked() {
                    action = Some(BrowserAction::ExploreReferences(key.clone()));
                    ui.close_menu();
                }
                context_menu_separator(ui);
                if context_menu_button(ui, "Dump Tag to JSON...").clicked() {
                    action = Some(BrowserAction::DumpJson(key.clone()));
                    ui.close_menu();
                }
                if context_menu_button(ui, "Dump Tag References...").clicked() {
                    action = Some(BrowserAction::DumpReferences(key.clone()));
                    ui.close_menu();
                }
            });
            let icon_rect = egui::Rect::from_center_size(
                menu.response.rect.center(),
                Vec2::splat(BUTTON_ICON_SIZE),
            );
            paint_button_icon_at(ui, ButtonIcon::Other, icon_rect, text_dark());
            menu.response.on_hover_text("Other tag actions");

            let favorite_label = if is_favorite { "Favorited" } else { "Favorite" };
            let favorite_icon = if is_favorite {
                ButtonIcon::FavouriteFilled
            } else {
                ButtonIcon::Favourite
            };
            if icon_text_button(ui, favorite_icon, favorite_label, favorite_enabled)
                .on_disabled_hover_text("Only loose editing-kit tags can be favorited")
                .clicked()
            {
                action = Some(BrowserAction::ToggleFavorite(key.clone()));
            }
            if icon_text_button(ui, ButtonIcon::Find, "Find", true).clicked() {
                self.active = kit_index;
                self.kits[kit_index].selected_key = Some(key.clone());
                self.find.within = FindWithin::CurrentTag;
                self.find.open = true;
                self.find.focus_query = true;
            }
        });

        if let Some(action) = action {
            self.active = kit_index;
            self.handle_browser_action(action, ctx.clone());
        }
    }

}

fn tag_header_path_parts(display_path: &str) -> (String, String) {
    let normalized = display_path.replace('\\', "/");
    match normalized.rsplit_once('/') {
        Some((parent, title)) => (parent.replace('/', "  ›  "), title.to_owned()),
        None => (String::new(), normalized),
    }
}
