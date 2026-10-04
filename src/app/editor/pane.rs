//! The single tag-editor pane: header, per-tag bars, the schema-driven editor, and edit application.
//! It owns one document's presentation and deferred-op application; layout of panes and source I/O belong elsewhere.

use super::*;
use crate::app::shell::frame::PANE_HEADER_ACTION_GAP;
use crate::app::shell::frame::pane_header_breadcrumbs;
use crate::app::shell::frame::PANE_HEADER_ICON_TEXT_GAP;
use crate::app::shell::frame::pane_header_path_parts;
use crate::app::shell::frame::PANE_HEADER_ICON_SIZE;
use crate::app::shell::frame::pane_header_inline_left_width;
use crate::app::shell::frame::PANE_HEADER_COMMON_ACTIONS_WIDTH;
use crate::app::shell::frame::PANE_HEADER_SECTION_GAP;

const TAG_HEADER_KEYWORDS_INLINE_BREAKPOINT: f32 = 1160.0;
const TAG_HEADER_ACTIONS_SINGLE_ROW_BREAKPOINT: f32 = 1180.0;
const TAG_HEADER_DYNAMIC_ACTIONS_WIDTH: f32 = 285.0;
const BITMAP_HEADER_ACTIONS_WIDTH: f32 = 105.0;

/// What a tag pane reads beyond the model and its kit's view: Find's state,
/// for the filter it may apply; the field a reference jump is heading for;
/// the sound player; and the docs and Wwise binding the caller resolved for
/// this tag, which fill caches and so are resolved before the draw.
pub(in crate::app) struct PaneInputs<'a> {
    pub(in crate::app) find: &'a FindDialogState,
    pub(in crate::app) field_nav: Option<&'a FieldNav>,
    pub(in crate::app) audio: &'a audio::AudioState,
    pub(in crate::app) def_docs: Option<Rc<DefDocs>>,
    pub(in crate::app) ce_sound: Option<Arc<crate::core::source::ce_audio::CeSoundBinding>>,
}

/// What a tag pane hands on once it has drawn: its edits, a jump Find's
/// filter asked for, and the bitmap thumbnails its hovers want.
pub(in crate::app) struct PaneDrawn {
    kit: KitId,
    key: String,
    /// Whether the ops will change the document: the kit is writable and
    /// there are some.
    mutated: bool,
    ops: DeferredOps,
    find_filter_block_jump: Option<String>,
    bitmap_hover_requests: Arc<std::sync::Mutex<Vec<TagEntry>>>,
}

impl Baboon {
    /// Apply what a tag pane collected while it drew.
    pub(in crate::app) fn apply_pane_drawn(&mut self, drawn: PaneDrawn, ctx: &egui::Context) {
        let PaneDrawn {
            kit,
            key,
            mutated,
            ops,
            find_filter_block_jump,
            bitmap_hover_requests,
        } = drawn;
        let Some(kit_index) = self.model.kit_index(kit) else {
            return;
        };
        // Applying them opens the undo window, or closes it on a frame with
        // none, which is why a pane sends this every frame.
        self.apply_doc_ops(kit_index, &key, "Edit", ops, UndoStep::Coalesce);
        if find_filter_block_jump.is_some() {
            // Preserve find_filter_applied until the next render so disabling
            // the filter produces the normal one-shot restore-defaults pass.
            self.search.find.filter_results = false;
        }
        let kit_id = self.model.kits[kit_index].id;
        queue_bitmap_hover_thumbnails(
            &cx!(self, ctx),
            kit_index,
            &mut self.views[kit_id].bitmap_browser,
            &bitmap_hover_requests,
        );
        // These ops are applied *after* the pane has been drawn, so the frame
        // on screen still shows the tag as it was before the edit. egui only
        // redraws when new input arrives, so nothing here is guaranteed to be
        // visible until something else happens to wake the UI -- an added
        // block element missing from that block's own instance selector, for
        // one.
        if mutated {
            ctx.request_repaint();
        }
        if let Some(block_path) = find_filter_block_jump {
            self.navigate_to_field(ctx, &key, &block_path);
            ctx.data_mut(|data| data.insert_temp(jump_target_id(), block_path));
        }
        // Model preview work starts only after the pane has drawn its shell,
        // so switching tabs can reach the screen before a complex geometry
        // parse begins. Follow-up texture/overlay/animation workers use the
        // same post-draw hook once the base preview lands.
        self.maybe_request_model_preview(kit_index, &key, ctx);
        self.maybe_request_model_textures(kit_index, &key, ctx);
        self.maybe_request_model_overlays(kit_index, &key, ctx);
        self.maybe_request_model_animations(kit_index, &key, ctx);
        self.maybe_request_model_animation_decode(kit_index, &key, ctx);
    }
}

/// Renders one open tag as a self-contained pane.
///
/// This is the only place a tag document is rendered. Every layout that
/// shows a tag — the docked editor, a popped-out window, and (later) each
/// tile in a split — calls this with a distinct `scope`, which salts every
/// widget id underneath (see [`FieldEditContext::widget_id`]) so the same
/// tag shown twice keeps independent scroll/focus/collapse state while
/// sharing one underlying [`TagDocument`].
///
/// The document is read where it lies in the model: the editor collects
/// what the user does as deferred ops rather than editing it in place, and
/// those, with everything else the pane collected, go out as one
/// [`EditorCommand::PaneDrawn`] every frame the tag is loaded — an empty
/// one too, since applying no ops is what closes the undo coalescing
/// window. A caller that renders a chrome row of its own (a dock button, a
/// tile tab bar) draws it before calling.
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn draw_tag_pane(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    entry: &TagEntry,
    scope: &str,
    show_keyword_bar: bool,
    inputs: PaneInputs,
    view: &mut KitView,
    editor: &mut EditorFeature,
) {
    let ctx = cx.egui;
    let key = entry.key.clone();
    let kit_read_only = cx.model.editing_kit_is_read_only(kit_index);
    draw_responsive_tag_header(cx, ui, kit_index, entry, show_keyword_bar);
    if kit_read_only {
        ui.label(
            RichText::new("Read-Only Editing Kit")
                .small()
                .color(subtle_dark()),
        );
    }

    let supports_field_search = supports_field_search(entry);

    let picker_was_open = editor.tag_reference_picker.is_some();
    let PaneInputs {
        find,
        field_nav,
        audio,
        def_docs,
        ce_sound,
    } = inputs;
    let bitmap_preview_view = cx.model.prefs.bitmap_preview_view;
    let model_preview_perspective = cx.model.prefs.model_preview_perspective;

    let Some(doc) = cx.model.kits[kit_index].parsed_tags.get(&key) else {
        if cx.model.kits[kit_index].loading_tags.contains(&key) {
            ui.label("Loading tag data...");
        } else {
            ui.label("Select the tag again to load it.");
        }
        return;
    };

    let filter_in_scope = match find.within {
        FindWithin::CurrentTag => {
            cx.model.kits[kit_index].selected_key.as_deref() == Some(key.as_str())
        }
        FindWithin::OpenTags | FindWithin::AllTags => {
            cx.model.kits[kit_index].open_tabs.contains(&key)
        }
    };
    let apply_find_filter = supports_field_search
        && find.open
        && find.filter_results
        && !find.query.is_empty()
        && !find.look_in.is_empty()
        && filter_in_scope;
    let field_filter = if apply_find_filter {
        let signature = format!(
            "{}|{:?}|{}|{}|{:?}",
            find.query,
            find.look_in,
            find.match_case,
            find.whole_word,
            doc.content_stamp(),
        );
        let cached = view
            .find_filter_applied
            .get(&key)
            .filter(|applied| applied.signature == signature)
            .map(|applied| applied.filter.clone());
        let filter = cached.unwrap_or_else(|| {
            let filter = std::sync::Arc::new(compute_find_field_filter(
                &doc.tag,
                cx.model.names(),
                def_docs.as_deref(),
                &find.query,
                find.look_in,
                find.match_case,
                find.whole_word,
            ));
            view.find_filter_applied.insert(
                key.clone(),
                AppliedFindFilter {
                    signature,
                    filter: filter.clone(),
                },
            );
            filter
        });
        Some(FieldFilterAction::Apply(filter))
    } else {
        view
            .find_filter_applied
            .remove(&key)
            .map(|_| FieldFilterAction::RestoreDefaults)
    };

    // Where the sound player's keyboard shortcuts act: the focused tab.
    let sound_has_focus = cx.model.active == kit_index
        && cx.model.kits[kit_index].selected_key.as_deref() == Some(key.as_str());
    let kit = &cx.model.kits[kit_index];
    let kit_id = kit.id;
    let bitmap_hover_requests =
        begin_bitmap_hovers(ui, Arc::clone(&view.bitmap_browser.thumbnails));
    let source = kit.source.as_ref();
    let names = &kit.names;

    let mut ops = DeferredOps::default();
    let mut color_request = None;
    let mut function_request = None;
    // What the shader and material grids open. They write a popup
    // directly rather than through `edit_context`, so they get their own
    // sinks here instead of the shared popups: written straight into
    // those, nothing recorded which kit they came from, and confirming
    // one applied to whichever kit had last opened a popup.
    let mut grid_color_popup = None;
    let mut grid_function_popup = None;
    let mut block_clip_request = None;
    let mut tsv_paste_request = None;
    let mut ce_sound_ref_request = None;
    // What the fields ask of other features, collected here and sent once
    // the pane has drawn.
    let mut status = cx.model.status.clone();
    let mut open_request = None;
    let mut sound_queue = std::collections::VecDeque::new();
    let mut sound_extract_request = None;
    let mut tool_import = None;
    let mut model_preview_size = cx.model.prefs.model_preview_size;

    // Taken rather than read: it is a one-shot, and egui remembers the
    // state each container lands in, so forcing it for a single frame is
    // what makes it stick.
    let expand_all = view.pending_expand.remove(&key);
    let sound_volume = audio.volume();
    let sound_speed = audio.speed();
    let sound_owner = crate::app::audio::SoundOwner {
        kit: kit_id,
        key: key.clone(),
    };
    let sound_playback = audio.playback(Some(&sound_owner));
    // A status line from another tab's sound is that tab's business.
    let sound_status_shown = audio.status_is_for(&sound_owner);
    let sound_looping = audio.looping();
    let sound_preview = audio.preview_for(&sound_owner).cloned();
    let expert_mode = cx.model.prefs.expert_mode;
    let ce_paks_root = source.and_then(|s| match &s.source {
        TagSource::IoStoreContainerSet { root, .. } => Some(root.as_path()),
        _ => None,
    });
    let kit_layout = source.and_then(LoadedSourceData::kit_layout);
    let mut edit_context = FieldEditContext {
        view_scope: scope,
        tag_key: &key,
        group_tag: entry.group_tag,
        root: Some(doc.tag.root()),
        game: source.and_then(|source| source.game),
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
        kit_layout: kit_layout.as_ref(),
        bitmap_hover_entries: source.map(LoadedSourceData::full_entry_set),
        tag_reference_catalog: source
            .and_then(|source| tag_reference_catalog_for_source(source, expert_mode)),
        tag_reference_picker: &mut editor.tag_reference_picker,
        status: Some(&mut status),
        editable: !kit_read_only && is_editable_tag(entry, &doc.tag),
        show_block_sizes: cx.model.prefs.show_block_sizes,
        buffers: &mut view.edit_buffers,
        pending: &mut ops.pending,
        block_ops: &mut ops.block_ops,
        block_confirm: &mut editor.block_confirm,
        open_request: &mut open_request,
        sound_play_request: crate::app::audio::SoundRequests::new(
            &mut sound_queue,
            Some(sound_owner),
        ),
        sound_status: audio.status.as_deref().filter(|_| sound_status_shown),
        sound_volume,
        sound_speed,
        sound_playback,
        sound_looping,
        sound_preview,
        sound_has_focus,
        sound_extract_request: &mut sound_extract_request,
        sound_language: audio.language.as_deref(),
        ce_sound: ce_sound.as_deref(),
        ce_sound_ref_request: &mut ce_sound_ref_request,
        ce_paks_root,
        tool_import: &mut tool_import,
        shader_ops: &mut ops.shader_ops,
        shader_param_ops: &mut ops.shader_param_ops,
        h2_shader_param_ops: &mut ops.h2_shader_param_ops,
        model_variant_ops: &mut ops.model_variant_ops,
        color_request: &mut color_request,
        function_request: &mut function_request,
        docs: def_docs.as_deref(),
        tsv_paste_request: &mut tsv_paste_request,
        block_clipboard: editor.block_clipboard.as_ref(),
        block_clip_request: &mut block_clip_request,
        field_filter: field_filter.as_ref(),
        // Only the pane being navigated to sees the request. The scroll
        // gate downstream matches on the field path alone, so an unfiltered
        // nav scrolled every pane whose tag happened to have a field at the
        // same path — which, between two tags of the same group, is most of
        // them. Splitting a tag view is what exposed this.
        field_nav: field_nav.filter(|nav| nav.kit == kit_id && nav.tag_key == key),
        expand_all,
        nested_default: cx.model.prefs.nested_default,
    };

    if is_bitmap_tag(entry) {
        let preview = view.caches.bitmap_previews.entry(key.clone()).or_default();
        preview.apply_view_settings(bitmap_preview_view);
        draw_bitmap_tag(
            ui,
            ctx,
            &doc.tag,
            entry,
            names,
            &mut grid_color_popup,
            preview,
            cx.model.prefs.expert_mode,
            &mut edit_context,
        );
        if preview.view_settings() != bitmap_preview_view {
            let settings = preview.view_settings();
            cx.edit_prefs(move |prefs| prefs.bitmap_preview_view = settings);
        }
    } else {
        let mut local_model_preview;
        let model_preview = if is_previewable_geometry_group_for_game(
            entry.group_tag,
            names,
            source.and_then(|source| source.game),
        ) {
            view.caches.model_previews.entry(key.clone()).or_default()
        } else {
            local_model_preview = ModelPreviewState::default();
            &mut local_model_preview
        };
        // One projection for every pane, as the bitmap view settings are:
        // applied going in, and a toggle in this pane written back.
        model_preview.perspective = model_preview_perspective;
        let document_revision = (
            doc.id,
            doc.dirty.revision(),
            kit.generation,
            view.caches.render_method_epoch,
        );
        draw_tag(
            ui,
            &doc.tag,
            document_revision,
            entry,
            names,
            source.map(|source| &source.source),
            source.and_then(|source| source.game),
            &mut view.caches.rmdf_cache,
            &mut view.caches.rmop_cache,
            &mut view.caches.h2_templates,
            &mut grid_color_popup,
            &mut grid_function_popup,
            model_preview,
            &mut model_preview_size,
            cx.model.prefs.expert_mode,
            &mut edit_context,
        );
        if model_preview.perspective != model_preview_perspective {
            let perspective = model_preview.perspective;
            cx.edit_prefs(move |prefs| prefs.model_preview_perspective = perspective);
        }
    }

    let find_filter_block_jump = ctx.data_mut(|data| {
        let id = find_filter_block_jump_id(scope, &key);
        let request = data.get_temp::<String>(id);
        data.remove::<String>(id);
        request
    });

    if model_preview_size != cx.model.prefs.model_preview_size {
        cx.edit_prefs(move |prefs| prefs.model_preview_size = model_preview_size);
    }
    if status != cx.model.status {
        cx.set_status(status);
    }
    if let Some(request) = open_request {
        cx.send(ReferencesCommand::Open(request));
    }
    if !sound_queue.is_empty() {
        cx.send(AudioCommand::Queue(sound_queue));
    }
    if let Some(request) = sound_extract_request {
        cx.send(ExportCommand::QueueSoundExtract(request));
    }
    if let Some(request) = tool_import {
        cx.send(KitsCommand::QueueToolImport(request));
    }

    // A color swatch was clicked: open the shared picker. Each popup
    // records the kit it was opened from, so confirming it later edits
    // this document rather than whichever kit is active by then.
    editor.adopt_opened_popups(
        kit_id,
        grid_color_popup.or(color_request),
        grid_function_popup.or(function_request),
    );
    // A referenced sound was played/extracted from a container source. It
    // is stamped with this kit because resolving it needs that kit's
    // containers, not whichever one happens to be active by the drain.
    if let Some(request) = ce_sound_ref_request {
        editor.pending_ce_sound_ref = Some((kit_id, key.clone(), request));
    }
    // The reference picker is opened from inside the field renderer rather
    // than hoisted here, so it is stamped by noticing it appear.
    if !picker_was_open && editor.tag_reference_picker.is_some() {
        editor.tag_reference_picker_kit = Some(kit_id);
    }
    // And a block confirmation, raised the same way. Stamping only an
    // unstamped one leaves a confirmation another pane raised alone.
    if let Some(confirm) = editor.block_confirm.as_mut() {
        confirm.kit.get_or_insert(kit_id);
    }
    // Element(s) were copied: stash them on the clipboard.
    if let Some(clip) = block_clip_request {
        cx.set_status(format!("Copied {} '{}' element(s)", clip.elements.len(), clip.label));
        editor.block_clipboard = Some(clip);
    }
    // "Paste TSV…" was chosen: open the import window.
    if let Some(req) = tsv_paste_request {
        editor.tsv_paste = Some(TsvPasteState {
            kit: kit_id,
            tag_key: key.clone(),
            block_path: req.block_path,
            block_label: req.block_label,
            element_count: req.element_count,
            text: String::new(),
            status: None,
        });
    }
    cx.send(EditorCommand::PaneDrawn(Box::new(PaneDrawn {
        kit: kit_id,
        key,
        mutated: !kit_read_only && !ops.is_empty(),
        ops,
        find_filter_block_jump,
        bitmap_hover_requests,
    })));
}

fn draw_responsive_tag_header(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    entry: &TagEntry,
    show_keyword_bar: bool,
) {
    let available = ui.available_width();
    let keywords_inline = available >= TAG_HEADER_KEYWORDS_INLINE_BREAKPOINT;
    let actions_single_row = available >= TAG_HEADER_ACTIONS_SINGLE_ROW_BREAKPOINT;
    let dynamic_actions_width = match &entry.group_tag.to_be_bytes() {
        b"scnr" => TAG_HEADER_DYNAMIC_ACTIONS_WIDTH,
        b"bitm" => BITMAP_HEADER_ACTIONS_WIDTH,
        _ => 0.0,
    };
    let has_dynamic_actions = dynamic_actions_width > 0.0;
    let actions_stacked = has_dynamic_actions && !actions_single_row;
    let action_width = if has_dynamic_actions {
        if actions_stacked {
            dynamic_actions_width
        } else {
            dynamic_actions_width + PANE_HEADER_SECTION_GAP + PANE_HEADER_COMMON_ACTIONS_WIDTH
        }
    } else {
        PANE_HEADER_COMMON_ACTIONS_WIDTH
    };
    let inline_left_width = pane_header_inline_left_width(available, action_width);
    let wide = inline_left_width.is_some();
    let left_width = inline_left_width.unwrap_or(available);
    let title_height = if cx.model.prefs.expert_mode {
        48.0
    } else {
        PANE_HEADER_ICON_SIZE
    };
    let left_height = if !keywords_inline && show_keyword_bar {
        title_height + 10.0 + BUTTON_HEIGHT
    } else {
        title_height.max(BUTTON_HEIGHT)
    };
    let key = entry.key.clone();
    let (breadcrumbs, title) = pane_header_path_parts(&entry.display_path);
    let mut breadcrumb_navigation = None;

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = PANE_HEADER_SECTION_GAP;
        ui.allocate_ui_with_layout(
            Vec2::new(left_width, left_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = PANE_HEADER_SECTION_GAP;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = PANE_HEADER_ICON_TEXT_GAP;
                        let (icon_rect, _) = ui.allocate_exact_size(
                            Vec2::splat(PANE_HEADER_ICON_SIZE),
                            Sense::hover(),
                        );
                        paint_tag_icon_at(ui, Some(entry.group_tag), icon_rect);

                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            breadcrumb_navigation = pane_header_breadcrumbs(ui, &breadcrumbs);
                            ui.label(
                                RichText::new(title).size(15.0).strong().color(text_dark()),
                            );
                            if cx.model.prefs.expert_mode {
                                ui.label(
                                    RichText::new(group_label(
                                        &cx.model.kits[kit_index].names,
                                        entry.group_tag,
                                    ))
                                    .size(11.0)
                                    .color(subtle_dark()),
                                );
                            }
                        });
                    });

                    if keywords_inline && show_keyword_bar {
                        draw_keyword_bar(cx, ui, kit_index, &key);
                    }
                });
                if !keywords_inline && show_keyword_bar {
                    draw_keyword_bar(cx, ui, kit_index, &key);
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
                                |ui| {
                                    draw_tag_header_specific_actions(
                                        cx, ui, kit_index, entry,
                                    )
                                },
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    draw_tag_header_common_actions(
                                        cx, ui, kit_index, entry,
                                    );
                                },
                            );
                        });
                    } else {
                        draw_tag_header_common_actions(cx, ui, kit_index, entry);
                        if has_dynamic_actions {
                            draw_tag_header_specific_actions(cx, ui, kit_index, entry);
                        }
                    }
                },
            );
        }
    });

    if !wide {
        if has_dynamic_actions {
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), BUTTON_HEIGHT),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| draw_tag_header_specific_actions(cx, ui, kit_index, entry),
            );
            ui.add_space(8.0);
        }
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), BUTTON_HEIGHT),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| draw_tag_header_common_actions(cx, ui, kit_index, entry),
        );
    }
    ui.add_space(20.0);
    ui.separator();
    if let Some((rel_path, label)) = breadcrumb_navigation {
        cx.send(BrowserCommand::Action {
            kit: cx.model.kits[kit_index].id,
            action: BrowserAction::OpenFolderBrowser {
                rel_path,
                label,
                open_in_new_tab: true,
            },
        });
    }
}

fn draw_tag_header_specific_actions(cx: &Ctx, ui: &mut Ui, kit_index: usize, entry: &TagEntry) {
    match &entry.group_tag.to_be_bytes() {
        b"scnr" => draw_scenario_launcher_buttons(cx, ui, kit_index, entry),
        b"bitm" => {
            let tags_root = cx.model.loaded_tags_root_for(kit_index);
            let can_reimport = bitmap_reimport_data_path(entry, tags_root.as_deref()).is_some();
            if icon_text_button(ui, ButtonIcon::Import, "Reimport", can_reimport)
                .on_disabled_hover_text("Reimport requires a loose editing-kit bitmap tag")
                .on_hover_text(
                    "Run tool bitmaps for this bitmap source path, then reload the tag",
                )
                .clicked()
            {
                cx.send(EditorCommand::ReimportBitmap {
                    kit: cx.model.kits[kit_index].id,
                    key: entry.key.clone(),
                });
            }
        }
        _ => {}
    }
}

fn draw_tag_header_common_actions(cx: &Ctx, ui: &mut Ui, kit_index: usize, entry: &TagEntry) {
    let key = entry.key.clone();
    let kit = cx.model.kits[kit_index].id;
    let is_favorite = cx.model.kits[kit_index].active_favorite_entries
        .iter()
        .any(|favorite| favorite.key == key);
    let favorite_enabled = matches!(entry.location, TagEntryLocation::LooseFile(_));
    let mut action = None;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = PANE_HEADER_ACTION_GAP;
        right_aligned_icon_menu_button(
            ui,
            ButtonIcon::Other,
            "Other tag actions",
            CONTEXT_MENU_WIDTH,
            |ui| {
                if let Some(menu_action) = draw_tag_context_menu_contents(ui, entry, None, true)
                {
                    action = Some(menu_action);
                }
            },
        );

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
            cx.send(SearchCommand::FindInTag {
                kit,
                key: key.clone(),
            });
        }
    });

    if let Some(action) = action {
        cx.send(BrowserCommand::Action { kit, action });
    }
}
