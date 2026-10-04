//! The editor's windows over a tag: the tag reference picker, and the color
//! and function popups a field opens. They draw from the editor's own state;
//! what they change in a document is an [`EditorCommand`].

use super::*;

/// The tag reference picker, while one is open. Its catalog comes from
/// the kit the picker was opened from, the same kit the pick is applied
/// to, or it would offer another game's tags.
pub(in crate::app) fn draw_tag_reference_picker_window(cx: &Ctx, editor: &mut EditorFeature) {
    if editor.tag_reference_picker.is_none() {
        return;
    }
    let ctx = cx.egui;
    let expert_mode = cx.model.prefs.expert_mode;
    // The catalog has to come from the kit the picker was opened from, the
    // same kit its selection is applied to — otherwise it would offer
    // another game's tags to pick from.
    let picker_kit = editor
        .tag_reference_picker_kit
        .and_then(|kit| cx.model.resolve_kit(kit))
        .unwrap_or(cx.model.active);
    let Some(catalog) = cx.model.kits[picker_kit]
        .source
        .as_ref()
        .and_then(|source| tag_reference_catalog_for_source(source, expert_mode))
    else {
        editor.tag_reference_picker = None;
        return;
    };

    let mut open = true;
    let mut picked = None;
    {
        let picker = editor
            .tag_reference_picker
            .as_mut()
            .expect("picker presence checked above");
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
                    picker.current_group,
                    &mut picker.search,
                );
            });
    }

    if let Some(input) = picked {
        let picker = editor
            .tag_reference_picker
            .take()
            .expect("picker remains open while processing selection");
        cx.send(EditorCommand::PickTagReference {
            kit: cx.model.kits[picker_kit].id,
            tag_key: picker.tag_key,
            field_path: picker.field_path,
            input,
        });
    } else if !open {
        editor.tag_reference_picker = None;
    }
}

/// The color picker popup, while one is open. The palette it keeps (custom
/// swatches and the folder palettes load from) is a preference: the popup
/// edits copies, and a change goes to the live preferences as a command.
pub(in crate::app) fn draw_color_popup_window(cx: &Ctx, editor: &mut EditorFeature) {
    if editor.color_popup.is_none() {
        editor.color_popup_kit = None;
        return;
    }
    let mut swatches = cx.model.prefs.custom_color_swatches.clone();
    let mut palette_dir = cx.model.prefs.palette_last_dir.clone();
    let result = draw_color_popup(cx.egui, &mut editor.color_popup, &mut swatches, &mut palette_dir);
    if swatches != cx.model.prefs.custom_color_swatches || palette_dir != cx.model.prefs.palette_last_dir {
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
                if let Some(popup) = editor.function_popup.as_mut() {
                    popup.apply_draft_color(target, argb);
                }
                return;
            }
        };
        cx.send(EditorCommand::ApplyPopupOps {
            opened_from: editor.color_popup_kit,
            tag_key,
            label,
            ops,
        });
    }
    if editor.color_popup.is_none() {
        editor.color_popup_kit = None;
    }
}

/// The function editor popup, while one is open.
pub(in crate::app) fn draw_function_popup_window(cx: &Ctx, editor: &mut EditorFeature) {
    if let Some(batch) = draw_function_popup(cx.egui, &mut editor.function_popup, &mut editor.color_popup) {
        let ops = DeferredOps {
            pending: batch.edits,
            function_data_ops: batch.data_ops,
            ..DeferredOps::default()
        };
        cx.send(EditorCommand::ApplyPopupOps {
            opened_from: editor.function_popup_kit,
            tag_key: batch.tag_key,
            label: "Edit function",
            ops,
        });
    }
    if editor.function_popup.is_none() {
        editor.function_popup_kit = None;
    }
}

/// What the editor's windows can be asked to do.
pub(in crate::app) enum EditorCommand {
    /// Paste the TSV paste window's rows into its block.
    ApplyTsvPaste,
    /// Apply the confirmed block delete or delete-all.
    ApplyBlockConfirm,
    /// Apply a popup's edits to the tag at `tag_key` in the kit it was opened
    /// from, or the active kit when it recorded none.
    ApplyPopupOps {
        opened_from: Option<KitId>,
        tag_key: String,
        label: &'static str,
        ops: DeferredOps,
    },
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
    pub(in crate::app) fn apply_editor_command(&mut self, command: EditorCommand) {
        match command {
            EditorCommand::ApplyTsvPaste => self.apply_tsv_paste(),
            EditorCommand::ApplyBlockConfirm => self.apply_block_confirm(),
            EditorCommand::ApplyPopupOps {
                opened_from,
                tag_key,
                label,
                ops,
            } => {
                if let Some(kit) = self.popup_target_kit(opened_from) {
                    self.apply_doc_ops(kit, &tag_key, label, ops, UndoStep::Own);
                }
            }
            EditorCommand::PickTagReference {
                kit,
                tag_key,
                field_path,
                input,
            } => self.apply_picked_tag_reference(kit, &tag_key, &field_path, input),
        }
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
