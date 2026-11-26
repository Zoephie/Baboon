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
            .id(egui::Id::unique(
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
                    egui::Id::unique("campaign_evolved_tag_reference_picker_contents"),
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

/// A picker opened in a Model Preview's Material Editor: what it is
/// picking shows in that preview as it changes, before OK writes it.
#[derive(Clone)]
pub(in crate::app) struct LivePreviewTarget {
    /// The previewed tag whose editor opened it.
    pub(in crate::app) preview: String,
    /// This picker's own, so the preview can tell its draft from another's.
    pub(in crate::app) draft_id: u64,
}

impl LivePreviewTarget {
    pub(in crate::app) fn new(preview: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            preview: preview.to_owned(),
            draft_id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }
}

/// What a live picker last sent of its draft, so it sends again only when
/// that changes.
#[derive(Clone, Copy, PartialEq)]
enum SentDraft {
    Nothing,
    Draft(u64),
}

/// A hash of `value`'s `Debug`, to tell one draft from the next.
fn draft_signature(value: &impl std::fmt::Debug) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{value:?}").hash(&mut hasher);
    hasher.finish()
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
    /// Set when the Material Editor opened it: the preview shows its draft.
    pub(in crate::app) live: Option<LivePreviewTarget>,
}

impl ColorPopupWindow {
    /// Send the live preview what OK would write now, when it differs from
    /// what was last sent; nothing once it closes.
    fn send_draft(&self, cx: &Ctx) {
        let Some(live) = &self.live else { return };
        let id = egui::Id::unique(("live_color_draft", live.draft_id));
        let last = cx.egui.data(|data| data.get_temp::<SentDraft>(id)).unwrap_or(SentDraft::Nothing);
        let draft = self
            .popup
            .as_ref()
            .filter(|popup| popup.picked_another())
            .and_then(|popup| Some((popup.draft_result()?, popup.channels())));
        let now = draft.as_ref().map_or(SentDraft::Nothing, |(_, channels)| SentDraft::Draft(draft_signature(channels)));
        if now == last {
            return;
        }
        cx.egui.data_mut(|data| data.insert_temp(id, now));
        match draft {
            // A function editor's own colour: its draft, not the tag's.
            Some((ColorPopupResult::FunctionDraftColor { target, argb }, _)) => {
                cx.send(EditorCommand::FunctionDraftColorPreview { target, argb: Some(argb) });
            }
            Some((result, _)) => {
                let Some((tag_key, _, ops)) = color_result_ops(result) else { return };
                cx.send(EditorCommand::DraftPreview {
                    kit: self.kit,
                    preview: live.preview.clone(),
                    draft_id: live.draft_id,
                    tag_key,
                    opened_at: self.opened_at,
                    ops: Some(ops),
                });
            }
            // Closed, or showing nothing: whichever it was showing goes. Only
            // one colour picker is open at a time, so a function editor's
            // tentative colour can only have been this one's.
            None => {
                cx.send(EditorCommand::FunctionDraftColorPreview { target: FunctionDraftColorTarget::Logical(0), argb: None });
                cx.send(EditorCommand::DraftPreview {
                    kit: self.kit,
                    preview: live.preview.clone(),
                    draft_id: live.draft_id,
                    tag_key: String::new(),
                    opened_at: self.opened_at,
                    ops: None,
                });
            }
        }
    }
}

/// The tag, undo label and edits a colour picker's result writes, when it
/// writes the tag: what OK applies, and what the live preview draws.
fn color_result_ops(result: ColorPopupResult) -> Option<(String, &'static str, DeferredOps)> {
    Some(match result {
        ColorPopupResult::FieldEdit { tag_key, edit } => {
            (tag_key, "Edit color", DeferredOps { pending: vec![edit], ..DeferredOps::default() })
        }
        ColorPopupResult::ShaderOp { tag_key, op } => {
            (tag_key, "Shader edit", DeferredOps { shader_ops: vec![op], ..DeferredOps::default() })
        }
        ColorPopupResult::ShaderParamOp { tag_key, op } => {
            (tag_key, "Shader parameter", DeferredOps { shader_param_ops: vec![op], ..DeferredOps::default() })
        }
        ColorPopupResult::H2ShaderParamOp { tag_key, op } => {
            (tag_key, "Shader parameter", DeferredOps { h2_shader_param_ops: vec![op], ..DeferredOps::default() })
        }
        ColorPopupResult::StringListOp { tag_key, op } => {
            (tag_key, "Edit color", DeferredOps { string_list_ops: vec![op], ..DeferredOps::default() })
        }
        ColorPopupResult::FunctionDraftColor { .. } => return None,
    })
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
            if let ColorPopupResult::FunctionDraftColor { target, argb } = result {
                cx.send(EditorCommand::FunctionDraftColor { target, argb });
                self.send_draft(cx);
                return self.popup.is_some();
            }
            let Some((tag_key, label, ops)) = color_result_ops(result) else {
                return self.popup.is_some();
            };
            cx.send(EditorCommand::ApplyPopupOps {
                opened_from: Some(self.kit),
                opened_at: self.opened_at,
                tag_key,
                label,
                ops,
            });
        }
        self.send_draft(cx);
        self.popup.is_some()
    }
}

/// The function editor popup over a field, and the kit it was opened from.
pub(in crate::app) struct FunctionPopupWindow {
    pub(in crate::app) popup: Option<FunctionPopup>,
    pub(in crate::app) kit: KitId,
    /// As [`ColorPopupWindow::opened_at`].
    pub(in crate::app) opened_at: Option<(u64, u64)>,
    /// As [`ColorPopupWindow::live`].
    pub(in crate::app) live: Option<LivePreviewTarget>,
}

impl FunctionPopupWindow {
    /// As [`ColorPopupWindow::send_draft`], with its colour picker's colour
    /// not yet picked.
    fn send_draft(&self, cx: &Ctx) {
        let Some(live) = &self.live else { return };
        let id = egui::Id::unique(("live_function_draft", live.draft_id));
        let last = cx.egui.data(|data| data.get_temp::<SentDraft>(id)).unwrap_or(SentDraft::Nothing);
        let batch = self.popup.as_ref().and_then(|popup| popup.batch(true));
        let now = batch.as_ref().map_or(SentDraft::Nothing, |batch| {
            SentDraft::Draft(draft_signature(&(
                batch.edits.iter().map(|edit| (&edit.path, &edit.input)).collect::<Vec<_>>(),
                batch.data_ops.iter().map(|op| (&op.block_path, &op.data)).collect::<Vec<_>>(),
            )))
        });
        if now == last {
            return;
        }
        cx.egui.data_mut(|data| data.insert_temp(id, now));
        cx.send(EditorCommand::DraftPreview {
            kit: self.kit,
            preview: live.preview.clone(),
            draft_id: live.draft_id,
            tag_key: batch.as_ref().map(|batch| batch.tag_key.clone()).unwrap_or_default(),
            opened_at: self.opened_at,
            ops: batch.map(|batch| DeferredOps {
                pending: batch.edits,
                function_data_ops: batch.data_ops,
                ..DeferredOps::default()
            }),
        });
    }
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
            // Its color goes to this editor's draft, not to the tag; shown
            // live as this editor's is.
            cx.open_dialog(ColorPopupWindow {
                popup: Some(popup),
                kit: self.kit,
                opened_at: None,
                live: self.live.as_ref().map(|live| LivePreviewTarget::new(&live.preview)),
            });
        }
        self.send_draft(cx);
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
    /// The color picker a live function popup opened shows `argb` for
    /// `target`, not yet picked (`None`: shows nothing now).
    FunctionDraftColorPreview {
        target: FunctionDraftColorTarget,
        argb: Option<u32>,
    },
    /// What a picker the Material Editor opened would write now, for its
    /// preview to draw (`None`: nothing, as it closed or matches the tag).
    DraftPreview {
        kit: KitId,
        preview: String,
        draft_id: u64,
        tag_key: String,
        opened_at: Option<(u64, u64)>,
        ops: Option<DeferredOps>,
    },
    /// Apply what a tag pane collected while it drew.
    PaneDrawn(Box<PaneDrawn>),
    /// What a Model Preview's Material Editor asked for.
    MaterialEditor(MaterialEditorCommand),
    StringRows(Box<super::string_row_clipboard::RowCommand>),
    /// The pane showing `key` in `kit` took focus: it is the tag the file
    /// actions act on.
    FocusTab { kit: KitId, key: String },
    /// Bring a tag's tab to the front, in its kit, from anywhere: the mini
    /// player's way back to the sound it plays.
    RevealTab { kit: KitId, key: String },
    /// Bring `kit`'s open tabs in line with the panes its tile tree holds,
    /// which a drag, a split or a close can have changed.
    SyncOpenTabs { kit: KitId },
    /// Run tool bitmaps for the bitmap at `key` in `kit`, then reload it.
    ReimportBitmap { kit: KitId, key: String },
    /// Proceed with the folder-wide Halo 2 bitmap reimport held by its dialog.
    ConfirmBitmapReimport,
    /// Read the bitmap at `key` in `kit` from disk again: Refresh Bitmap.
    RefreshBitmap { kit: KitId, key: String },
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
            EditorCommand::StringRows(command) => self.apply_string_row_command(*command, ctx),
            EditorCommand::PaneDrawn(drawn) => return self.apply_pane_drawn(*drawn, ctx),
            EditorCommand::MaterialEditor(command) => self.apply_material_editor_command(command, ctx),
            EditorCommand::SyncOpenTabs { kit } => {
                return self
                    .model
                    .kit_index(kit)
                    .is_some_and(|index| self.kit_and_view(index).sync_open_tabs());
            }
            EditorCommand::FocusTab { kit, key } => {
                if let Some(index) = self.model.kit_index(kit) {
                    // A folder pane is not a tag: focusing one leaves no tag for
                    // Save, Undo or Ctrl+W to act on, rather than a hidden one.
                    self.kit_and_view(index).focus_pane(key);
                }
            }
            EditorCommand::RevealTab { kit, key } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.focus_kit(index);
                    let mut kit_view = self.kit_and_view(index);
                    kit_view.open_tag_pane(&key);
                    kit_view.focus_pane(key);
                }
            }
            EditorCommand::ReimportBitmap { kit, key } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.focus_kit(index);
                    self.begin_reimport_bitmap(key, ctx.clone());
                }
            }
            EditorCommand::ConfirmBitmapReimport => self.confirm_reimport_bitmap(ctx),
            EditorCommand::RefreshBitmap { kit, key } => {
                if let Some(index) = self.model.kit_index(kit) {
                    // Unsaved edits would be dropped, so the button is greyed
                    // out for them; a reload landing on one is ignored too.
                    if self.model.kits[index].parsed_tags.get(&key).is_some_and(|doc| doc.dirty.is_set()) {
                        self.model.status = "Save or undo the bitmap's edits before refreshing it".to_owned();
                    } else {
                        self.load_tag_in(index, key, ctx.clone(), true);
                        // Said now: the read landing leaves the status alone,
                        // and one that fails says so instead.
                        self.model.status = "Bitmap refreshed from disk".to_owned();
                    }
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
            EditorCommand::FunctionDraftColorPreview { target, argb } => {
                if let Some(popup) = self
                    .dialogs
                    .get_mut::<FunctionPopupWindow>()
                    .and_then(|window| window.popup.as_mut())
                {
                    popup.set_tentative_color(argb.map(|argb| (target, argb)));
                }
            }
            EditorCommand::DraftPreview { kit, preview, draft_id, tag_key, opened_at, ops } => {
                self.apply_draft_preview(kit, preview, draft_id, tag_key, opened_at, ops);
            }
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
