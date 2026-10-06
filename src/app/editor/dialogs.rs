//! The editor's windows over a tag: the tag reference picker, and the color
//! and function popups a field opens. Each is a dialog that remembers the kit
//! it was opened from; what they change in a document is an [`EditorCommand`].

use super::*;

/// The movable tag reference picker over a reference field, and the kit it
/// was opened from: its catalog comes from that kit, the same kit the pick is
/// applied to, or it would offer another game's tags.
pub(in crate::app) struct TagReferencePickerWindow {
    pub(in crate::app) state: TagReferencePickerState,
    pub(in crate::app) kit: KitId,
}

impl Dialog for TagReferencePickerWindow {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let ctx = cx.egui;
        let expert_mode = cx.model.prefs.expert_mode;
        let Some(picker_kit) = cx.model.resolve_kit(self.kit) else {
            return false;
        };
        let Some(catalog) = cx.model.kits[picker_kit]
            .source
            .as_ref()
            .and_then(|source| tag_reference_catalog_for_source(source, expert_mode))
        else {
            return false;
        };

        let mut open = true;
        let mut picked = None;
        let picker = &mut self.state;
        egui::Window::new("Select Tag Reference")
            .constrain_to(window_work_area(ctx))
            .id(egui::Id::new(
                "campaign_evolved_tag_reference_picker_window",
            ))
            .open(&mut open)
            .movable(true)
            .resizable(true)
            .collapsible(false)
            .default_size(window_size(ctx, Vec2::new(620.0, 420.0), true))
            .min_size(window_size(ctx, Vec2::new(420.0, 220.0), true))
            .show(ctx, |ui| {
                picked = draw_tag_reference_catalog_picker_contents(
                    ui,
                    egui::Id::new("campaign_evolved_tag_reference_picker_contents"),
                    catalog,
                    &picker.allowed_groups,
                    &mut picker.search,
                );
            });

        if let Some(input) = picked {
            cx.send(EditorCommand::PickTagReference {
                kit: self.kit,
                tag_key: self.state.tag_key.clone(),
                field_path: self.state.field_path.clone(),
                input,
            });
            return false;
        }
        open
    }
}

/// The color picker popup over a field, and the kit it was opened from: its
/// edit is addressed by tag key, which is only unique within a kit. The
/// palette it keeps (custom swatches and the folder palettes load from) is a
/// preference: the popup edits copies, and a change goes to the live
/// preferences as a command.
pub(in crate::app) struct ColorPopupWindow {
    pub(in crate::app) popup: Option<MaterialColorPopup>,
    pub(in crate::app) kit: KitId,
    /// The tag's [`TagDocument::layout_stamp`] when the popup opened, if it
    /// writes to one: OK is refused once it moves, since the path the popup
    /// holds may point at another element by then.
    pub(in crate::app) opened_at: Option<(u64, u64)>,
}

impl Dialog for ColorPopupWindow {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let mut swatches = cx.model.prefs.custom_color_swatches.clone();
        let mut palette_dir = cx.model.prefs.palette_last_dir.clone();
        let result = draw_color_popup(cx.egui, &mut self.popup, &mut swatches, &mut palette_dir);
        if swatches != cx.model.prefs.custom_color_swatches
            || palette_dir != cx.model.prefs.palette_last_dir
        {
            cx.edit_prefs(move |prefs| {
                prefs.custom_color_swatches = swatches;
                prefs.palette_last_dir = palette_dir;
            });
        }
        if let Some(result) = result {
            let (tag_key, label, ops) = match result {
                ColorPopupResult::FieldEdit { tag_key, edit } => {
                    let ops = DeferredOps {
                        pending: vec![edit],
                        ..DeferredOps::default()
                    };
                    (tag_key, "Edit color", ops)
                }
                ColorPopupResult::ShaderOp { tag_key, op } => {
                    let ops = DeferredOps {
                        shader_ops: vec![op],
                        ..DeferredOps::default()
                    };
                    (tag_key, "Shader edit", ops)
                }
                ColorPopupResult::ShaderParamOp { tag_key, op } => {
                    let ops = DeferredOps {
                        shader_param_ops: vec![op],
                        ..DeferredOps::default()
                    };
                    (tag_key, "Shader parameter", ops)
                }
                ColorPopupResult::H2ShaderParamOp { tag_key, op } => {
                    let ops = DeferredOps {
                        h2_shader_param_ops: vec![op],
                        ..DeferredOps::default()
                    };
                    (tag_key, "Shader parameter", ops)
                }
                ColorPopupResult::FunctionDraftColor { target, argb } => {
                    cx.send(EditorCommand::FunctionDraftColor { target, argb });
                    return self.popup.is_some();
                }
            };
            cx.send(EditorCommand::ApplyPopupOps {
                opened_from: Some(self.kit),
                opened_at: self.opened_at,
                tag_key,
                label,
                ops,
            });
        }
        self.popup.is_some()
    }
}

/// The function editor popup over a field, and the kit it was opened from.
pub(in crate::app) struct FunctionPopupWindow {
    pub(in crate::app) popup: Option<FunctionPopup>,
    pub(in crate::app) kit: KitId,
    /// As [`ColorPopupWindow::opened_at`].
    pub(in crate::app) opened_at: Option<(u64, u64)>,
}

impl Dialog for FunctionPopupWindow {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        // The function editor opens a color picker for a draft color; the
        // picker is a dialog of its own, and its choice comes back as
        // `EditorCommand::FunctionDraftColor`.
        let mut color = None;
        if let Some(batch) = draw_function_popup(cx.egui, &mut self.popup, &mut color) {
            let ops = DeferredOps {
                pending: batch.edits,
                function_data_ops: batch.data_ops,
                ..DeferredOps::default()
            };
            cx.send(EditorCommand::ApplyPopupOps {
                opened_from: Some(self.kit),
                opened_at: self.opened_at,
                tag_key: batch.tag_key,
                label: "Edit function",
                ops,
            });
        }
        if let Some(popup) = color {
            // Its color goes to this editor's draft, not to the tag.
            cx.open_dialog(ColorPopupWindow {
                popup: Some(popup),
                kit: self.kit,
                opened_at: None,
            });
        }
        self.popup.is_some()
    }
}

/// What the editor's windows can be asked to do.
pub(in crate::app) enum EditorCommand {
    /// Paste the TSV paste window's rows into its block.
    ApplyTsvPaste,
    /// Apply the confirmed block delete or delete-all.
    ApplyBlockConfirm,
    /// Commit the block table's staged changes to its tag.
    SaveBlockTable,
    /// Apply a popup's edits to the tag at `tag_key` in the kit it was opened
    /// from, or the active kit when it recorded none.
    ApplyPopupOps {
        opened_from: Option<KitId>,
        /// The tag's layout stamp when the popup opened.
        opened_at: Option<(u64, u64)>,
        tag_key: String,
        label: &'static str,
        ops: DeferredOps,
    },
    /// The color picker a function popup opened chose `argb` for `target`:
    /// set it in that popup's draft.
    FunctionDraftColor {
        target: FunctionDraftColorTarget,
        argb: u32,
    },
    /// Apply what a tag pane collected while it drew.
    PaneDrawn(Box<PaneDrawn>),
    /// The pane showing `key` in `kit` took focus: it is the tag the file
    /// actions act on.
    FocusTab { kit: KitId, key: String },
    /// Bring `kit`'s open tabs in line with the panes its tile tree holds,
    /// which a drag, a split or a close can have changed.
    SyncOpenTabs { kit: KitId },
    /// Run tool bitmaps for the bitmap at `key` in `kit`, then reload it.
    ReimportBitmap { kit: KitId, key: String },
    /// Set the reference field at `field_path` of the tag at `tag_key` in
    /// `kit` to the picked `input`.
    PickTagReference {
        kit: KitId,
        tag_key: String,
        field_path: String,
        input: String,
    },
}

impl Baboon {
    /// Apply `command`, returning whether it may have changed what the next
    /// frame draws. Only the two sent every frame a tag is open can say no.
    pub(in crate::app) fn apply_editor_command(&mut self, command: EditorCommand, ctx: &egui::Context) -> bool {
        match command {
            EditorCommand::PaneDrawn(drawn) => return self.apply_pane_drawn(*drawn, ctx),
            EditorCommand::SyncOpenTabs { kit } => {
                return self
                    .model
                    .kit_index(kit)
                    .is_some_and(|index| self.kit_and_view(index).sync_open_tabs());
            }
            EditorCommand::FocusTab { kit, key } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.model.kits[index].selected_key = Some(key);
                }
            }
            EditorCommand::ReimportBitmap { kit, key } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.focus_kit(index);
                    self.begin_reimport_bitmap(key, ctx.clone());
                }
            }
            EditorCommand::ApplyTsvPaste => self.apply_tsv_paste(),
            EditorCommand::ApplyBlockConfirm => self.apply_block_confirm(),
            EditorCommand::SaveBlockTable => self.save_block_table(ctx),
            EditorCommand::ApplyPopupOps {
                opened_from,
                opened_at,
                tag_key,
                label,
                ops,
            } => {
                let Some(kit) = self.popup_target_kit(opened_from) else {
                    return true;
                };
                let Some(doc) = self.model.kits[kit].parsed_tags.get(&tag_key) else {
                    self.model.status = format!("{label}: the tag is no longer open; nothing was written.");
                    return true;
                };
                // The popup's path has element indices in it. Once the tag's
                // blocks change shape it can point at a different element: a
                // color for the first parameter, written after that parameter
                // was deleted, landed in the next one and reported success.
                if opened_at.is_some_and(|stamp| stamp != doc.layout_stamp()) {
                    self.model.status = format!(
                        "{label}: the tag's blocks changed while the editor was open, so nothing was written. Open it again."
                    );
                    return true;
                }
                self.apply_doc_ops(kit, &tag_key, label, ops, UndoStep::Own);
            }
            EditorCommand::PickTagReference {
                kit,
                tag_key,
                field_path,
                input,
            } => self.apply_picked_tag_reference(kit, &tag_key, &field_path, input),
            EditorCommand::FunctionDraftColor { target, argb } => {
                if let Some(popup) = self
                    .dialogs
                    .get_mut::<FunctionPopupWindow>()
                    .and_then(|window| window.popup.as_mut())
                {
                    popup.apply_draft_color(target, argb);
                }
            }
        }
        true
    }

    fn apply_picked_tag_reference(&mut self, kit: KitId, tag_key: &str, field_path: &str, input: String) {
        let Some(kit) = self.model.kit_index(kit) else {
            self.model.status = "The tag being edited is no longer open".to_owned();
            return;
        };
        if !self.model.kits[kit].parsed_tags.contains_key(tag_key) {
            self.model.status = "The tag being edited is no longer open".to_owned();
            return;
        }
        let ops = DeferredOps {
            pending: vec![PendingFieldEdit {
                path: field_path.to_owned(),
                input: input.clone(),
            }],
            ..DeferredOps::default()
        };
        let applied = self.apply_doc_ops(kit, tag_key, "Change tag reference", ops, UndoStep::Own);
        if applied.is_some() {
            // `insert_clean` from upstream: the picked reference is now the
            // document's value, so the draft starts unmodified rather than
            // looking like an uncommitted edit.
            self.views[self.model.kits[kit].id]
                .edit_buffers
                .insert_clean(format!("{tag_key}|{field_path}"), input);
            self.invalidate_tag_caches_in(kit, tag_key);
        }
    }
}
