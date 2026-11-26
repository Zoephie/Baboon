//! The Model Preview's Material Editor: the sidebar's Materials card, which
//! lists the shaders the model draws with, lights each one's parts while the
//! pointer is over it, and edits the one picked with the same editor its
//! own tab has.
//!
//! The card's room is left by the preview's sidebar (`MaterialSlot`), and
//! filled by the tag pane once its own drawing is done: the editor edits a
//! document and draws it with [`draw_tag_body`], so the shader is drawn,
//! edited, undone and has its popups stamped exactly as in its tab, under a
//! scope of the card's own.
//! The shader is the same document its tab would show: opened without a tab
//! when it has none, and held for the preview (`Kit::held_documents`), so it
//! is checked against its file and asked about when the model closes.
//!
//! What it reads of the preview (the list, and what hovering lights) is
//! `model_preview::material_editor`'s.

use super::*;
use crate::app::editor::dialogs::ShaderSwitchTarget;
use crate::app::model_preview::material_editor::{EditorMaterial, MaterialKey, editor_materials};
use crate::app::shell::AppAction;

/// What the Material Editor asks the app for.
pub(in crate::app) enum MaterialEditorCommand {
    /// Open the shader tag `material` names for the preview of `preview`,
    /// holding its document there, and let go of the clean ones it held
    /// before.
    Hold {
        kit: KitId,
        preview: String,
        material: MaterialKey,
        group: u32,
        path: String,
    },
    /// The preview's editor closed: let go of its documents that have no
    /// unsaved edits.
    Release { kit: KitId, preview: String },
    /// Open the render model `(group, path)` for the preview of `preview`,
    /// holding its document there, for the editor's shader reference to
    /// switch its slots' shaders.
    HoldRenderModel { kit: KitId, preview: String, group: u32, path: String },
    /// Set the shader reference at each of `paths` in the render model at
    /// `render_model` to `input`, as one edit, and show the material in
    /// `slot` once the preview has loaded again.
    SwitchShader { kit: KitId, preview: String, render_model: String, paths: Vec<String>, input: String, slot: u16 },
    /// Save the shader documents at `keys`, each where its tag saves.
    Save { kit: KitId, keys: Vec<String> },
    /// Throw away the unsaved edits of the shader documents at `keys`.
    Reset { kit: KitId, keys: Vec<String> },
}

impl Baboon {
    pub(in crate::app) fn apply_material_editor_command(&mut self, command: MaterialEditorCommand, ctx: &egui::Context) {
        match command {
            MaterialEditorCommand::Hold { kit, preview, material, group, path } => {
                let Some(index) = self.model.kit_index(kit) else {
                    return;
                };
                let result = self.reference_entry_key(index, group, &path);
                if let Ok(key) = &result {
                    let render_model = self.views[kit]
                    .caches
                    .model_previews
                    .get(&preview)
                    .and_then(|state| state.material_live.render_model.as_ref()?.key.clone().ok());
                let keep: Vec<&str> = std::iter::once(key.as_str()).chain(render_model.as_deref()).collect();
                self.kit_and_view(index).release_clean_documents_held_by(&preview, &keep);
                    self.model.kits[index].hold_document(key, &preview);
                    self.load_tag_in(index, key.clone(), ctx.clone(), false);
                }
                let file = result.as_ref().ok().and_then(|key| match &self.model.entry_for_key_in(index, key)?.location {
                    TagEntryLocation::LooseFile(path) => Some(path.clone()),
                    _ => None,
                });
                if let Some(state) = self.views[kit].caches.model_previews.get_mut(&preview) {
                    // The preview keeps it up to date itself from now on:
                    // saving it, or editing it, needs no load.
                    if let Some(file) = &file {
                        state.reads.mark_live(file);
                    }
                    state.material_live.documents.insert(material, result);
                }
            }
            MaterialEditorCommand::HoldRenderModel { kit, preview, group, path } => {
                let Some(index) = self.model.kit_index(kit) else { return };
                let key = self.reference_entry_key(index, group, &path);
                let mut allowed = Vec::new();
                if let Ok(key) = &key {
                    // The previewed tag itself is its tab's.
                    if *key != preview {
                        self.model.kits[index].hold_document(key, &preview);
                        if !self.model.kits[index].parsed_tags.contains_key(key) {
                            self.load_tag_in(index, key.clone(), ctx.clone(), false);
                        }
                    }
                    let field = if group == u32::from_be_bytes(*b"mod2") { "shader" } else { "render method" };
                    if let Some(entry) = self.model.entry_for_key_in(index, key).cloned()
                        && let Some(docs) = self.def_docs_for_entry(index, &entry)
                    {
                        let field_allowed = |name: &str| {
                            docs.all_entries().find_map(|entry| match entry {
                                DefEntry::Field { clean_name, tag_reference_allowed, .. } if clean_name == name => {
                                    Some(tag_reference_allowed.clone())
                                }
                                _ => None,
                            })
                        };
                        // Halo 2's render model names its shader `shader`.
                        allowed = field_allowed(field).or_else(|| field_allowed("shader")).unwrap_or_default();
                    }
                }
                if let Some(state) = self.views[kit].caches.model_previews.get_mut(&preview) {
                    state.material_live.render_model = Some(crate::app::model_preview::material_editor::EditorRenderModel {
                        path,
                        key,
                        allowed,
                        drawn: None,
                    });
                }
            }
            MaterialEditorCommand::SwitchShader { kit, preview, render_model, paths, input, slot } => {
                let Some(index) = self.model.kit_index(kit) else { return };
                if !self.model.kits[index].parsed_tags.contains_key(&render_model) {
                    self.model.status = "The render model is no longer open; nothing was written.".to_owned();
                    return;
                }
                let ops = DeferredOps {
                    pending: paths.iter().map(|path| PendingFieldEdit { path: path.clone(), input: input.clone() }).collect(),
                    ..DeferredOps::default()
                };
                // The load drawn now: editing the previewed tag itself starts
                // the next one as the edit lands.
                let geometry = self.views[kit].caches.model_previews.get(&preview).and_then(|state| match &state.data {
                    Some(Ok(data)) => Some(data.geometry_id),
                    _ => None,
                });
                if self.apply_doc_ops(index, &render_model, "Change shader", ops, UndoStep::Own).is_some() {
                    self.invalidate_tag_caches_in(index, &render_model);
                    if let Some(state) = self.views[kit].caches.model_previews.get_mut(&preview)
                        && let (Some(editor), Some(geometry)) = (state.material_editor.as_mut(), geometry)
                    {
                        editor.follow_slot = Some((slot, geometry));
                    }
                }
            }
            MaterialEditorCommand::Save { kit, keys } => {
                let Some(index) = self.model.kit_index(kit) else { return };
                // The save paths act on the active kit.
                self.focus_kit(index);
                let (mut saved, mut errors) = (0, Vec::new());
                for key in keys {
                    match self.save_tag_routed(&key) {
                        Ok(_) => saved += 1,
                        // One refused stops the rest, as a disk conflict
                        // opens its window and waits on it.
                        Err(error) => {
                            errors.push(error);
                            break;
                        }
                    }
                }
                self.model.status = if errors.is_empty() {
                    format!("Saved {saved} material(s)")
                } else {
                    format!("Save failed: {}", errors.join("; "))
                };
            }
            MaterialEditorCommand::Reset { kit, keys } => {
                let Some(index) = self.model.kit_index(kit) else { return };
                for key in &keys {
                    self.discard_tag_changes(index, key, ctx);
                }
                if keys.len() > 1 {
                    self.model.status = format!("Discarded the unsaved changes to {} materials", keys.len());
                }
            }
            MaterialEditorCommand::Release { kit, preview } => {
                let Some(index) = self.model.kit_index(kit) else {
                    return;
                };
                let held: Vec<String> = self.model.kits[index]
                    .held_documents
                    .iter()
                    .filter(|(_, holders)| holders.contains(&preview))
                    .map(|(key, _)| key.clone())
                    .collect();
                self.kit_and_view(index).release_clean_documents_held_by(&preview, &[]);
                // What it let go of, the preview watches as files again.
                let kit_state = &self.model.kits[index];
                let released: Vec<PathBuf> = held
                    .iter()
                    .filter(|key| !kit_state.held_documents.get(*key).is_some_and(|holders| holders.contains(&preview)))
                    .filter_map(|key| match &kit_state.entry_for_key(key)?.location {
                        TagEntryLocation::LooseFile(path) => Some(path.clone()),
                        _ => None,
                    })
                    .collect();
                let render_model_held = |key: &str| {
                    self.model.kits[index].held_documents.get(key).is_some_and(|holders| holders.contains(&preview))
                };
                if let Some(state) = self.views[kit].caches.model_previews.get_mut(&preview) {
                    for path in &released {
                        state.reads.unmark_live(path);
                    }
                    // A render model left with a switched shader stays held,
                    // and its changes still load the preview again.
                    if state.material_live.render_model.as_ref().is_some_and(|model| model.key.as_deref().is_ok_and(|key| !render_model_held(key))) {
                        state.material_live.render_model = None;
                    }
                }
            }
        }
    }
}

impl Baboon {
    /// Keep (or, with no `ops`, drop) what a picker the Material Editor of
    /// `preview` opened would write now: its preview draws it. A draft for a
    /// document whose layout has moved since the picker opened is dropped,
    /// as OK would refuse it.
    pub(in crate::app) fn apply_draft_preview(
        &mut self,
        kit: KitId,
        preview: String,
        draft_id: u64,
        tag_key: String,
        opened_at: Option<(u64, u64)>,
        ops: Option<DeferredOps>,
    ) {
        let Some(index) = self.model.kit_index(kit) else { return };
        let layout = self.model.kits[index].parsed_tags.get(&tag_key).map(TagDocument::layout_stamp);
        let Some(state) = self.views[kit].caches.model_previews.get_mut(&preview) else { return };
        let live = &mut state.material_live;
        let seq = live.draft.as_ref().map_or(0, |draft| draft.seq) + 1;
        live.draft = match ops {
            Some(ops) if opened_at.is_none_or(|stamp| Some(stamp) == layout) => Some(
                crate::app::model_preview::material_editor::DraftPreview {
                    draft_id,
                    document: tag_key,
                    ops,
                    layout_stamp: opened_at,
                    seq,
                },
            ),
            // Gone, or another picker's: only its own picker clears a draft.
            None if live.draft.as_ref().is_some_and(|draft| draft.draft_id != draft_id) => live.draft.take(),
            _ => None,
        };
    }
}

/// The scope the window draws its shader under: the pane's, so two panes
/// of one model keep windows of their own, and apart from the shader's tab.
fn editor_scope(scope: &str) -> String {
    format!("{scope}\u{1f}material_editor")
}

/// The height the Material Editor lays out in: more than any shader's
/// fields take, finite so nothing sized to what is left grows forever.
const UNBOUNDED_HEIGHT: f32 = 100_000.0;

/// Draw the Material Editor of the preview of `model`, the tag the pane
/// shows, in the room the sidebar's Materials card left for it, while the
/// card is open. Called once the pane's own body has drawn, so the view is
/// free for the shader's.
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn draw_material_editor_section(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    model: &TagEntry,
    scope: &str,
    inputs: PaneInputs,
    view: &mut KitView,
    editor: &mut EditorFeature,
) {
    let ctx = cx.egui;
    let kit = &cx.model.kits[kit_index];
    let kit_id = kit.id;
    let Some(state) = view.caches.model_previews.get_mut(&model.key) else {
        return;
    };
    let slot = state.material_slot.take();
    let documents = state.material_live.documents.clone();
    let Some(mut editor_state) = state.material_editor.clone() else {
        // Closed: what it held and left clean goes.
        if kit.held_documents.values().any(|holders| holders.contains(&model.key)) {
            cx.send(EditorCommand::MaterialEditor(MaterialEditorCommand::Release {
                kit: kit_id,
                preview: model.key.clone(),
            }));
        }
        return;
    };
    // Not on the page this pass (the Tag Fields tab is up, or the model is
    // loading): open still, but not drawn.
    let Some(slot) = slot else {
        return;
    };
    // The model as drawn: while it loads again, the page before it.
    let shown = match (&state.data, &state.reloading) {
        (Some(Ok(data)), _) => Some(data),
        (_, Some(reloading)) => Some(&reloading.data),
        _ => None,
    };
    let Some(shown) = shown else {
        return;
    };
    let materials = editor_materials(&shown.preview);
    let shows_textures = state.render_mode.uses_textures();
    follow_switched_slot(&mut editor_state, shown, &materials);
    let render_model = state.material_live.render_model.clone();
    let render_model_path = shown.render_model_path.clone();
    // "None" until one is picked, and once the picked one is gone.
    if editor_state.selected.as_ref().is_some_and(|key| !materials.iter().any(|material| &material.key == key)) {
        editor_state.selected = None;
    }
    let was_hovered = editor_state.hovered.take();

    let section_id = egui::Id::unique(("material_editor", scope, &model.key));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(section_id)
            .layer_id(slot.layer)
            // Laid out at the card's width and as tall as it needs, which is
            // measured for the room it leaves next pass; shown within the
            // room it has now.
            .max_rect(egui::Rect::from_min_size(slot.rect.min, egui::vec2(slot.rect.width(), UNBOUNDED_HEIGHT)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(slot.clip.intersect(slot.rect));
    if !slot.enabled {
        child.visuals_mut().disabled_alpha = 1.0;
        child.disable();
    }
    let contents = |ui: &mut Ui| {
        let picked = draw_material_header(ui, section_id, &materials, &mut editor_state);
        if let Some(material) = &picked {
            draw_shader_reference_row(cx, ui, kit_index, model, scope, view, material, &render_model_path, render_model.as_ref());
        }
        // "None": the dropdown alone, and the shaders it held and left
        // clean let go.
        let Some(material) = picked else {
            if kit.held_documents.values().any(|holders| holders.contains(&model.key)) {
                cx.send(EditorCommand::MaterialEditor(MaterialEditorCommand::Release {
                    kit: kit_id,
                    preview: model.key.clone(),
                }));
            }
            return;
        };
        if !shows_textures {
            ui.label(
                RichText::new("Edits show in the preview under Textured or Game Shaders shading.")
                    .small()
                    .color(subtle_dark()),
            );
        }
        ui.separator();
        let modified = modified_materials(cx, kit_index, &materials, &documents);
        draw_material_body(cx, ui, kit_index, model, scope, inputs, view, editor, &documents, &material);
        draw_modified_list(cx, ui, kit_id, &modified, &mut editor_state);
    };
    // At the card's width, never scrolling sideways: the sidebar widens for
    // a shader's grid while one is picked, as far as the page allows.
    contents(&mut child);
    // The card is as tall as what it drew, which the sidebar scrolls: the
    // room it leaves next pass.
    let drawn = child.min_rect().height().ceil();

    // Pressing in the card makes its shader the one Undo steps, as pressing
    // in the shader's own pane does.
    let active_id = section_id.with("active");
    if ctx.input(|input| input.pointer.any_pressed()) {
        let pos = ctx.input(|input| input.pointer.interact_pos());
        let inside = pos.is_some_and(|pos| slot.clip.intersect(slot.rect).contains(pos) && ctx.layer_id_at(pos) == Some(slot.layer));
        ctx.data_mut(|data| data.insert_temp(active_id, inside));
        if inside {
            cx.send(AppAction::FocusKit(kit_id));
        }
    }
    let active = ctx.data(|data| data.get_temp::<bool>(active_id)).unwrap_or(false);
    if active
        && let Some(Ok(key)) = editor_state.selected.as_ref().and_then(|selected| documents.get(selected))
    {
        crate::app::documents::claim_history_target(ctx, kit_id, key);
    }

    if editor_state.hovered != was_hovered {
        // The viewport drew before the card: draw again, lit as it is now.
        ctx.request_repaint();
    }
    if let Some(state) = view.caches.model_previews.get_mut(&model.key) {
        if state.material_editor.is_some() {
            state.material_editor = Some(editor_state);
        }
        // Its room was another height: drawn again at once in the right
        // one, so the card never shows cut short or with a gap for a frame.
        if slot.enabled && state.material_content_height != Some(drawn) {
            state.material_content_height = Some(drawn);
            ctx.request_discard("the Materials card changed height");
        }
    }
}

/// The dropdown of materials, "None" first, with arrows stepping through
/// them. Returns the picked material; `None` for "None", which shows the
/// dropdown alone.
fn draw_material_header(
    ui: &mut Ui,
    section_id: egui::Id,
    materials: &[EditorMaterial],
    state: &mut crate::app::model_preview::material_editor::MaterialEditorState,
) -> Option<EditorMaterial> {
    // Positions in the list: 0 is "None", then each material.
    let current = state
        .selected
        .as_ref()
        .and_then(|key| materials.iter().position(|material| &material.key == key))
        .map_or(0, |at| at + 1);
    let count = materials.len() + 1;
    let step = |at: usize| at.checked_sub(1).and_then(|at| materials.get(at)).map(|material| material.key.clone());
    ui.horizontal(|ui| {
        if icon_button(ui, ButtonIcon::ListDropdownLeft, "Previous material", current > 0, text_dark()).clicked() {
            state.selected = step(current - 1);
        }
        let width = (ui.available_width() - BUTTON_HEIGHT - 8.0).max(160.0);
        let popup_id = section_id.with("material_picker");
        let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
        let label = step(current).map_or_else(|| "None".to_owned(), |_| materials[current - 1].display_name());
        let response = picker_button(ui, popup_id, &label, Some(materials.len()), width, text_dark(), true);
        let just_opened = response.clicked() && !open;
        if response.clicked() {
            egui::Popup::toggle_id(ui.ctx(), popup_id);
        }
        let popup_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
        egui::Popup::from_response(&response)
            .id(popup_id)
            .open_memory(None)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                picker_popup_width(ui, &response);
                let filter_id = popup_id.with("search");
                let mut filter = ui.data(|data| data.get_temp::<String>(filter_id)).unwrap_or_default();
                let changed = picker_search_field(ui, &mut filter, "search materials…", just_opened);
                ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
                ui.separator();
                let query = filter.trim().to_lowercase();
                picker_results(ui, 360.0, changed, |ui| {
                    if query.is_empty() {
                        let row = ui.selectable_label(current == 0, "None");
                        if just_opened && current == 0 {
                            row.scroll_to_me(Some(egui::Align::Center));
                        }
                        if row.clicked() {
                            state.selected = None;
                            egui::Popup::close_id(ui.ctx(), popup_id);
                        }
                    }
                    let mut shown = 0;
                    for (at, material) in materials.iter().enumerate() {
                        let name = material.display_name();
                        if !query.is_empty()
                            && !name.to_lowercase().contains(&query)
                            && !material.path.to_lowercase().contains(&query)
                        {
                            continue;
                        }
                        shown += 1;
                        let row = ui.selectable_label(at + 1 == current, &name).on_hover_text(&material.path);
                        if just_opened && query.is_empty() && at + 1 == current {
                            row.scroll_to_me(Some(egui::Align::Center));
                        }
                        if row.hovered() {
                            state.hovered = Some(material.key.clone());
                        }
                        if row.clicked() {
                            state.selected = Some(material.key.clone());
                            egui::Popup::close_id(ui.ctx(), popup_id);
                        }
                    }
                    if shown == 0 && !query.is_empty() {
                        ui.label(RichText::new("No materials match.").color(subtle_dark()));
                    }
                });
            });
        if let Some(delta) = dropdown_wheel_delta(ui, &response, popup_open)
            && let Some(next) = combo_scroll_next_index(current, count, delta)
        {
            state.selected = step(next);
        }
        if icon_button(ui, ButtonIcon::ListDropdownRight, "Next material", current + 1 < count, text_dark()).clicked() {
            state.selected = step(current + 1);
        }
    });
    let picked = state
        .selected
        .as_ref()
        .and_then(|key| materials.iter().find(|material| &material.key == key))
        .cloned()?;
    Some(picked)
}

/// The picked material's shader: its notices, then its editor, once its
/// document is open.
#[allow(clippy::too_many_arguments)]
fn draw_material_body(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    model: &TagEntry,
    scope: &str,
    inputs: PaneInputs,
    view: &mut KitView,
    editor: &mut EditorFeature,
    documents: &std::collections::BTreeMap<MaterialKey, Result<String, String>>,
    material: &EditorMaterial,
) {
    let kit = &cx.model.kits[kit_index];
    let hold = || {
        cx.send(EditorCommand::MaterialEditor(MaterialEditorCommand::Hold {
            kit: kit.id,
            preview: model.key.clone(),
            material: material.key.clone(),
            group: material.key.group,
            path: material.path.clone(),
        }))
    };
    let key = match documents.get(&material.key) {
        None => {
            hold();
            ui.label(RichText::new("Opening the shader…").color(subtle_dark()));
            return;
        }
        Some(Err(error)) => {
            ui.colored_label(crate::app::ui_kit::panel_error_text(), error);
            return;
        }
        Some(Ok(key)) => key.clone(),
    };
    let Some(entry) = cx.model.entry_for_key_in(kit_index, &key).cloned() else {
        ui.colored_label(crate::app::ui_kit::panel_error_text(), "The shader is no longer in this kit.");
        return;
    };
    let Some(document) = kit.parsed_tags.get(&key) else {
        // Not open yet, or dropped (its edits discarded): open it again.
        if !kit.loading_tags.contains(&key) || !kit.held_documents.contains_key(&key) {
            hold();
        }
        ui.label(RichText::new("Loading the shader…").color(subtle_dark()));
        return;
    };
    if let Some(reason) = unsaveable_reason(&entry, &document.tag) {
        ui.label(RichText::new(format!("Cannot be saved: {reason}")).small().color(subtle_dark()));
    }
    disk_banner::draw_disk_banner(cx, ui, kit_index, &entry);
    draw_tag_body(cx, ui, kit_index, &entry, &editor_scope(scope), PaneInputs { field_nav: None, live_preview: Some(model.key.clone()), ..inputs }, view, editor);
}

/// Once the preview has loaded again after a shader switch, show the
/// material now in the switched slot.
fn follow_switched_slot(
    state: &mut crate::app::model_preview::material_editor::MaterialEditorState,
    shown: &crate::app::model_preview::ModelPreviewData,
    materials: &[EditorMaterial],
) {
    let Some((slot, geometry)) = state.follow_slot else { return };
    if shown.geometry_id == geometry {
        return;
    }
    state.follow_slot = None;
    if let Some(material) = materials.iter().find(|material| material.indices.contains(&slot)) {
        state.selected = Some(material.key.clone());
    }
}

/// The render model a preview names, as `(group, path)`: Halo CE's names
/// its `gbxmodel` with the extension, the later games' their `render_model`
/// without one.
fn render_model_reference(render_model_path: &str) -> Option<(u32, String)> {
    let path = render_model_path.replace('/', "\\");
    if path.is_empty() {
        return None;
    }
    Some(match path.rsplit_once('.') {
        Some((stem, "gbxmodel")) => (u32::from_be_bytes(*b"mod2"), stem.to_owned()),
        Some((stem, "render_model")) => (u32::from_be_bytes(*b"mode"), stem.to_owned()),
        _ => (u32::from_be_bytes(*b"mode"), path),
    })
}

/// The picked material's shader reference, as the render model's own field
/// with no label, whose Open opens the shader in a tab. Switching it rewrites every slot of the
/// opened model's render model naming the shader, as switching each in the
/// render model's tab would, and the preview loads again. A shader only an
/// LOD model or a held object names shows its path, not to be switched here.
#[allow(clippy::too_many_arguments)]
fn draw_shader_reference_row(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    model: &TagEntry,
    scope: &str,
    view: &mut KitView,
    material: &EditorMaterial,
    render_model_path: &str,
    render_model: Option<&crate::app::model_preview::material_editor::EditorRenderModel>,
) {
    let kit = &cx.model.kits[kit_index];
    if let Some((group, path)) = render_model_reference(render_model_path)
        && render_model.is_none_or(|held| held.path != path)
    {
        cx.send(EditorCommand::MaterialEditor(MaterialEditorCommand::HoldRenderModel {
            kit: kit.id,
            preview: model.key.clone(),
            group,
            path,
        }));
    }
    let document = render_model
        .and_then(|held| held.key.as_ref().ok())
        .and_then(|key| Some((key.as_str(), kit.parsed_tags.get(key)?)));
    let slots = document
        .map(|(_, document)| crate::app::model_preview::material_editor::shader_slots(&document.tag, &material.key));
    ui.horizontal(|ui| {
        match (document, slots) {
            (Some((key, _)), Some((slots, reference))) if !slots.is_empty() => {
                let source = kit.source.as_ref();
                let mut sinks = EditSinks::default();
                let row_scope = format!("{}\u{1f}shader_reference", editor_scope(scope));
                let target = ShaderSwitchTarget {
                    preview: model.key.clone(),
                    render_model: key.to_owned(),
                    paths: slots.iter().map(|(_, path)| path.clone()).collect(),
                    slot: slots[0].0,
                };
                let (pending, picker, open) = {
                    let mut edit = FieldEditContext::read_only(&mut sinks, &row_scope, key);
                    edit.buffers = &mut view.edit_buffers;
                    edit.group_tag = kit.entry_for_key(key).map_or(0, |entry| entry.group_tag);
                    edit.game = source.and_then(|source| source.game);
                    edit.definitions_root = source.and_then(|source| match &source.source {
                        TagSource::LooseFolder { definitions_root, .. } => Some(definitions_root.as_path()),
                        _ => None,
                    });
                    edit.tags_root = source.and_then(|source| match &source.source {
                        TagSource::LooseFolder { root, .. } => Some(root.as_path()),
                        _ => None,
                    });
                    edit.names = Some(&kit.names);
                    edit.tag_reference_catalog = source
                        .and_then(|source| tag_reference_catalog_for_source(source, cx.model.prefs.expert_mode));
                    edit.bitmap_hover_entries = source.map(LoadedSourceData::full_entry_set);
                    edit.editable = !cx.model.editing_kit_is_read_only(kit_index) && !kit.loading_tags.contains(key);
                    let mut meta = field_display_meta("");
                    meta.label = String::new();
                    meta.tag_reference_allowed = render_model.map(|held| held.allowed.clone()).unwrap_or_default();
                    let value = reference.as_ref().map_or_else(
                        || "NONE".to_owned(),
                        |(group, path)| {
                            let extension = kit.names.name_for(*group).or_else(|| blam_tags::paths::group_tag_to_extension(*group));
                            extension.map_or_else(|| path.clone(), |extension| format!("{path}.{extension}"))
                        },
                    );
                    // Unlabelled, the row sizes its box to what it has.
                    draw_foundation_tag_reference_row(ui, &meta, &value, reference.clone(), None, 0, &target.paths[0], &mut edit, 0.0);
                    (std::mem::take(edit.pending), edit.tag_reference_picker.take(), edit.open_request.take())
                };
                // The row's last edit stands for every slot naming the shader.
                if let Some(edit) = pending.into_iter().last() {
                    cx.send(EditorCommand::MaterialEditor(MaterialEditorCommand::SwitchShader {
                        kit: kit.id,
                        preview: target.preview.clone(),
                        render_model: target.render_model.clone(),
                        paths: target.paths.clone(),
                        input: edit.input,
                        slot: target.slot,
                    }));
                }
                if let Some(state) = picker {
                    cx.open_dialog(TagReferencePickerWindow { state, kit: kit.id, shader_switch: Some(target) });
                }
                if let Some(request) = open {
                    cx.send(ReferencesCommand::Open(request));
                }
            }
            (_, slots) => {
                let why = match (slots, render_model.map(|held| &held.key)) {
                    (Some(_), _) => "Named by an LOD model or a held object, not the opened model's own render model: switch it in that tag.".to_owned(),
                    (None, Some(Err(error))) => format!("Its render model can't be opened, so it can't be switched here: {error}"),
                    (None, _) => "The render model is opening.".to_owned(),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_text_button(ui, ButtonIcon::Open, "Open", true)
                        .on_hover_text("Open the shader in a tab of its own. Its edits go with it.")
                        .clicked()
                    {
                        cx.send(ReferencesCommand::Open(OpenTagRequest {
                            group_tag: material.key.group,
                            rel_path: material.path.clone(),
                            float: false,
                        }));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(&material.path).color(subtle_dark())).truncate()).on_hover_text(why);
                    });
                });
            }
        }
    });
}

/// A material whose shader has unsaved edits, as the modified list shows it.
struct ModifiedMaterial {
    material: EditorMaterial,
    /// Its shader document's key.
    document: String,
    /// The shader is open in a tab too: Reset discards that tab's edits.
    in_tab: bool,
    /// Why Save is refused, when it is.
    unsaveable: Option<&'static str>,
}

/// The listed materials whose shader documents have unsaved edits.
fn modified_materials(
    cx: &Ctx,
    kit_index: usize,
    materials: &[EditorMaterial],
    documents: &std::collections::BTreeMap<MaterialKey, Result<String, String>>,
) -> Vec<ModifiedMaterial> {
    let kit = &cx.model.kits[kit_index];
    materials
        .iter()
        .filter_map(|material| {
            let Some(Ok(document)) = documents.get(&material.key) else { return None };
            let tag = kit.parsed_tags.get(document).filter(|tag| tag.dirty.is_set())?;
            let entry = kit.entry_for_key(document)?;
            Some(ModifiedMaterial {
                material: material.clone(),
                document: document.clone(),
                in_tab: kit.open_tabs.contains(document),
                unsaveable: unsaveable_reason(entry, &tag.tag),
            })
        })
        .collect()
}

/// What has unsaved edits, each with Save and Reset, and both for all.
/// Reset asks a second click first: it can't be undone.
fn draw_modified_list(
    cx: &Ctx,
    ui: &mut Ui,
    kit: KitId,
    modified: &[ModifiedMaterial],
    state: &mut crate::app::model_preview::material_editor::MaterialEditorState,
) {
    if modified.is_empty() {
        state.confirm_reset = None;
        return;
    }
    ui.separator();
    let saveable: Vec<String> =
        modified.iter().filter(|item| item.unsaveable.is_none()).map(|item| item.document.clone()).collect();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Modified ({})", modified.len())).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_text_button(ui, ButtonIcon::Clear, "Reset All", true)
                .on_hover_text("Discard the unsaved changes to every material listed.")
                .clicked()
            {
                state.confirm_reset = Some(modified.iter().map(|item| item.document.clone()).collect());
            }
            if icon_text_button(ui, ButtonIcon::Save, "Save All", !saveable.is_empty())
                .on_hover_text("Save every material listed.")
                .clicked()
            {
                cx.send(EditorCommand::MaterialEditor(MaterialEditorCommand::Save { kit, keys: saveable.clone() }));
            }
        });
    });
    for item in modified {
        ui.horizontal(|ui| {
            let name = ui.label(format!("● {}", item.material.display_name())).on_hover_text(&item.material.path);
            if name.hovered() {
                state.hovered = Some(item.material.key.clone());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_text_button(ui, ButtonIcon::Clear, "Reset", true)
                    .on_hover_text("Discard this material's unsaved changes.")
                    .clicked()
                {
                    state.confirm_reset = Some(vec![item.document.clone()]);
                }
                let save = icon_text_button(ui, ButtonIcon::Save, "Save", item.unsaveable.is_none());
                let save = match item.unsaveable {
                    Some(reason) => save.on_disabled_hover_text(format!("Cannot be saved: {reason}")),
                    None => save.on_hover_text("Save this material's shader."),
                };
                if save.clicked() {
                    cx.send(EditorCommand::MaterialEditor(MaterialEditorCommand::Save {
                        kit,
                        keys: vec![item.document.clone()],
                    }));
                }
            });
        });
    }
    let Some(keys) = state.confirm_reset.clone() else { return };
    let names: Vec<String> = modified
        .iter()
        .filter(|item| keys.contains(&item.document))
        .map(|item| item.material.label.clone())
        .collect();
    let in_tab = modified.iter().any(|item| keys.contains(&item.document) && item.in_tab);
    let mut text = format!("Discard the unsaved changes to {}? This can't be undone.", names.join(", "));
    if in_tab {
        text.push_str(" This also discards the changes made in its tab.");
    }
    ui.label(RichText::new(text).color(save_error_text()));
    ui.horizontal(|ui| {
        if icon_text_button(ui, ButtonIcon::Clear, "Discard", true).clicked() {
            cx.send(EditorCommand::MaterialEditor(MaterialEditorCommand::Reset { kit, keys: keys.clone() }));
            state.confirm_reset = None;
        }
        if ui.button("Keep").clicked() {
            state.confirm_reset = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// After a switch, the editor shows the material in the switched slot
    /// once the preview has loaded again, and not before.
    #[test]
    fn a_switched_slots_material_is_shown_once_loaded_again() {
        let state = crate::app::model_preview::state::ModelPreviewState::loaded_with_shaders_for_test(
            "model",
            &[(b"rmsh", "shaders\\a"), (b"rmsh", "shaders\\b")],
        );
        let Some(Ok(shown)) = state.data.as_ref() else { unreachable!() };
        let materials = editor_materials(&shown.preview);
        let mut editor = crate::app::model_preview::material_editor::MaterialEditorState {
            selected: Some(materials[0].key.clone()),
            follow_slot: Some((1, shown.geometry_id)),
            ..Default::default()
        };
        follow_switched_slot(&mut editor, shown, &materials);
        assert_eq!(editor.selected, Some(materials[0].key.clone()), "still the page it switched on");
        editor.follow_slot = Some((1, shown.geometry_id + 1000));
        follow_switched_slot(&mut editor, shown, &materials);
        assert_eq!(editor.selected, Some(materials[1].key.clone()), "slot 1's, once loaded again");
        assert_eq!(editor.follow_slot, None);
    }

    #[test]
    fn a_preview_names_its_render_model_by_group_and_path() {
        assert_eq!(render_model_reference("objects/thing/thing.gbxmodel"), Some((u32::from_be_bytes(*b"mod2"), "objects\\thing\\thing".to_owned())));
        assert_eq!(render_model_reference("objects\\thing\\thing"), Some((u32::from_be_bytes(*b"mode"), "objects\\thing\\thing".to_owned())));
        assert_eq!(render_model_reference(""), None);
    }
}
