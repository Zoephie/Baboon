//! The Model Preview's Material Editor: a window over a model's preview
//! that lists the shaders it draws with, lights each one's parts while the
//! pointer is over it, and edits the one picked with the same editor its
//! own tab has.
//!
//! The window is the tag pane's, as the Layout Differences window is, and
//! not a `Dialog`: it edits a document, which a dialog can't, and draws it
//! with [`draw_tag_body`], so the shader is drawn, edited, undone and has its
//! popups stamped exactly as in its tab, under a scope of the window's own.
//! The shader is the same document its tab would show: opened without a tab
//! when it has none, and held for the preview (`Kit::held_documents`), so it
//! is checked against its file and asked about when the model closes.
//!
//! What it reads of the preview (the list, and what hovering lights) is
//! `model_preview::material_editor`'s.

use super::*;
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
                    self.kit_and_view(index).release_clean_documents_held_by(&preview, Some(key));
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
                self.kit_and_view(index).release_clean_documents_held_by(&preview, None);
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
                if let Some(state) = self.views[kit].caches.model_previews.get_mut(&preview) {
                    for path in &released {
                        state.reads.unmark_live(path);
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

/// Draw the Material Editor of the preview of `model`, the tag the pane
/// shows, when it is open. Called once the pane's own body has drawn, so
/// the view is free for the shader's.
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn draw_material_editor_window(
    cx: &Ctx,
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
    if editor_state.selected.as_ref().is_none_or(|key| !materials.iter().any(|material| &material.key == key)) {
        editor_state.selected = materials.first().map(|material| material.key.clone());
    }
    let was_hovered = editor_state.hovered.take();

    let title = format!("Material Editor — {}", tag_file_name(model));
    let window_id = egui::Id::unique(("material_editor", scope, &model.key));
    let mut open = true;
    let response = egui::Window::new(title)
        .id(window_id)
        .open(&mut open)
        .default_size([560.0, 640.0])
        .min_width(SHADER_GRID_MIN_WIDTH + 40.0)
        .min_height(320.0)
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            let picked = draw_material_header(ui, window_id, &materials, &mut editor_state);
            let Some(material) = picked else {
                ui.label(RichText::new("The model names no shaders.").color(subtle_dark()));
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
            let list_height = modified_list_height(&modified, editor_state.confirm_reset.is_some());
            let body = ui.available_size();
            ui.allocate_ui(egui::vec2(body.x, (body.y - list_height).max(240.0)), |ui| {
                draw_material_body(cx, ui, kit_index, model, scope, inputs, view, editor, &documents, &material);
            });
            draw_modified_list(cx, ui, kit_id, &modified, &mut editor_state);
        });

    // Pressing in the window makes its kit the one actions land in, and its
    // shader the one Undo steps, as pressing in a pane does.
    let active_id = window_id.with("active");
    if let Some(response) = &response {
        let pressed = ctx.input(|input| input.pointer.any_pressed());
        if pressed {
            let inside = ctx.input(|input| input.pointer.interact_pos()).is_some_and(|pos| response.response.rect.contains(pos))
                && ctx.layer_id_at(ctx.input(|input| input.pointer.interact_pos()).unwrap_or_default())
                    == Some(response.response.layer_id);
            ctx.data_mut(|data| data.insert_temp(active_id, inside));
            if inside {
                cx.send(AppAction::FocusKit(kit_id));
            }
        }
    }
    let active = ctx.data(|data| data.get_temp::<bool>(active_id)).unwrap_or(false);
    if active
        && let Some(Ok(key)) = editor_state.selected.as_ref().and_then(|selected| documents.get(selected))
    {
        crate::app::documents::claim_history_target(ctx, kit_id, key);
    }

    if editor_state.hovered != was_hovered {
        // The viewport drew before the window: draw again, lit as it is now.
        ctx.request_repaint();
    }
    if let Some(state) = view.caches.model_previews.get_mut(&model.key) {
        state.material_editor = open.then_some(editor_state);
    }
}

/// The dropdown of materials, with arrows stepping through them, and the
/// picked one's path under it. Returns the picked material.
fn draw_material_header(
    ui: &mut Ui,
    window_id: egui::Id,
    materials: &[EditorMaterial],
    state: &mut crate::app::model_preview::material_editor::MaterialEditorState,
) -> Option<EditorMaterial> {
    let current = state
        .selected
        .as_ref()
        .and_then(|key| materials.iter().position(|material| &material.key == key));
    ui.horizontal(|ui| {
        let step = |at: usize| materials.get(at).map(|material| material.key.clone());
        if icon_button(ui, ButtonIcon::ListDropdownLeft, "Previous material", current.is_some_and(|at| at > 0), text_dark()).clicked()
            && let Some(at) = current
        {
            state.selected = step(at - 1);
        }
        let width = (ui.available_width() - BUTTON_HEIGHT - 8.0).max(160.0);
        let popup_id = window_id.with("material_picker");
        let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
        let label = current.map_or_else(|| "No material".to_owned(), |at| materials[at].display_name());
        let response = picker_button(ui, popup_id, &label, Some(materials.len()), width, text_dark(), !materials.is_empty());
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
                        let row = ui.selectable_label(Some(at) == current, &name).on_hover_text(&material.path);
                        if just_opened && query.is_empty() && Some(at) == current {
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
                    if shown == 0 {
                        ui.label(RichText::new("No materials match.").color(subtle_dark()));
                    }
                });
            });
        if let Some(delta) = dropdown_wheel_delta(ui, &response, popup_open)
            && let Some(next) = combo_scroll_next_index(current.unwrap_or(0), materials.len(), delta)
        {
            state.selected = step(next);
        }
        if icon_button(ui, ButtonIcon::ListDropdownRight, "Next material", current.is_some_and(|at| at + 1 < materials.len()), text_dark())
            .clicked()
            && let Some(at) = current
        {
            state.selected = step(at + 1);
        }
    });
    let picked = state
        .selected
        .as_ref()
        .and_then(|key| materials.iter().find(|material| &material.key == key))
        .cloned()?;
    ui.label(RichText::new(&picked.path).small().color(subtle_dark()));
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
    ui.horizontal(|ui| {
        if let Some(reason) = unsaveable_reason(&entry, &document.tag) {
            ui.label(RichText::new(format!("Cannot be saved: {reason}")).small().color(subtle_dark()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_text_button(ui, ButtonIcon::Open, "Open in Tab", true)
                .on_hover_text("Open the shader in a tab of its own. Its edits go with it.")
                .clicked()
            {
                cx.send(ReferencesCommand::Open(OpenTagRequest {
                    group_tag: material.key.group,
                    rel_path: material.path.clone(),
                    float: false,
                }));
            }
        });
    });
    disk_banner::draw_disk_banner(cx, ui, kit_index, &entry);
    draw_tag_body(cx, ui, kit_index, &entry, &editor_scope(scope), PaneInputs { field_nav: None, live_preview: Some(model.key.clone()), ..inputs }, view, editor);
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

/// The height the modified list takes, kept clear of the editor above it.
fn modified_list_height(modified: &[ModifiedMaterial], confirming: bool) -> f32 {
    if modified.is_empty() {
        return 0.0;
    }
    let row = BUTTON_HEIGHT + 4.0;
    12.0 + row * (modified.len() as f32 + 1.0) + if confirming { row * 2.0 } else { 0.0 }
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
