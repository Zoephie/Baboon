//! Applying the edits the UI collected to an open tag: the read-only
//! refusal, the undo step, draft bookkeeping and the status line around
//! `core::document::apply`.

use super::*;

impl Baboon {
    /// Apply edits the UI collected to one open tag: the one entry every
    /// UI-originated edit goes through, so each gets the same undo step, the
    /// same read-only refusal, the same draft bookkeeping and the same
    /// status line.
    ///
    /// `None` when nothing was applied: the tag is not open in `kit_index`,
    /// or the kit is read-only (which says so on the status line when there
    /// was something to refuse).
    /// Commit the changed drafts `which` names, in every kit, as their rows
    /// would have. Returns whether any did.
    pub(in crate::app) fn commit_drafts(&mut self, which: DraftFlush) -> bool {
        let mut committed = false;
        for kit_index in 0..self.model.kits.len() {
            let kit = self.model.kits[kit_index].id;
            for (tag_key, ops) in self.views[kit].edit_buffers.take_uncommitted(which) {
                committed = true;
                match ops {
                    Ok(ops) => {
                        self.apply_doc_ops(kit_index, &tag_key, "Edit", ops, UndoStep::Coalesce);
                    }
                    Err(error) => self.model.status = error,
                }
            }
        }
        committed
    }

    pub(in crate::app) fn apply_doc_ops(
        &mut self,
        kit_index: usize,
        tag_key: &str,
        label: &str,
        ops: DeferredOps,
        step: UndoStep,
    ) -> Option<AppliedDeferredOps> {
        if self.model.editing_kit_is_read_only(kit_index) {
            if !ops.is_empty() {
                self.refuse_read_only_edit(kit_index);
            }
            if let Some(doc) = self.model.kits[kit_index].parsed_tags.get_mut(tag_key) {
                doc.journal.end_edit_window();
            }
            return None;
        }
        let kit = &mut self.model.kits[kit_index];
        let view = &mut self.views[kit.id];
        let doc = kit.parsed_tags.get_mut(tag_key)?;
        let applied = apply_deferred_ops(doc, ops, label);
        if step == UndoStep::Own {
            doc.journal.end_edit_window();
        }
        // Per-edit outcomes: a draft whose value applied cleanly is marked
        // clean, while one the parser rejected keeps the text the user typed
        // instead of snapping back to the old value.
        view.edit_buffers
            .accept_successful_edits(tag_key, &applied.outcomes);
        if applied.model_variants_changed
            && let Some(preview) = view.caches.model_previews.get_mut(tag_key)
        {
            preview.invalidate_load();
        }
        if let Some(status) = &applied.status {
            self.model.status = status.clone();
        }
        Some(applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "file:test.render_model";
    const FIELD: &str = "node list checksum";

    fn app_with_open_tag() -> Baboon {
        let mut app = Baboon::for_test();
        let schema = locate_definitions_root().join("halo3_mcc/render_model.json");
        app.model.kits[0].parsed_tags.insert(
            KEY.to_owned(),
            TagDocument::clean(TagFile::new(schema).unwrap()),
        );
        app
    }

    fn set(value: &str) -> DeferredOps {
        DeferredOps {
            pending: vec![PendingFieldEdit {
                path: FIELD.to_owned(),
                input: value.to_owned(),
            }],
            ..DeferredOps::default()
        }
    }

    fn value(app: &Baboon) -> String {
        let doc = &app.model.kits[0].parsed_tags[KEY];
        doc.tag
            .root()
            .field_path(FIELD)
            .and_then(|field| field.value())
            .map(|value| match value {
                blam_tags::TagFieldData::LongInteger(value) => value.to_string(),
                other => panic!("{FIELD} is a long integer, got {other:?}"),
            })
            .unwrap_or_default()
    }

    fn undo_steps(app: &mut Baboon) -> usize {
        let doc = app.model.kits[0].parsed_tags.get_mut(KEY).unwrap();
        let mut steps = 0;
        while doc.journal.undo(&doc.tag).is_some() {
            steps += 1;
        }
        steps
    }

    /// A popup's confirmed edit is its own undo step; a pane's per-frame
    /// edits join the one still open.
    #[test]
    fn own_edits_are_separate_steps_and_coalesced_edits_merge() {
        let mut app = app_with_open_tag();
        app.apply_doc_ops(0, KEY, "Edit color", set("7"), UndoStep::Own);
        app.apply_doc_ops(0, KEY, "Edit color", set("8"), UndoStep::Own);
        assert_eq!(value(&app), "8");
        assert_eq!(undo_steps(&mut app), 2, "two confirmed popups, two steps");

        let mut app = app_with_open_tag();
        app.apply_doc_ops(0, KEY, "Edit", set("7"), UndoStep::Coalesce);
        app.apply_doc_ops(0, KEY, "Edit", set("8"), UndoStep::Coalesce);
        assert_eq!(undo_steps(&mut app), 1, "one typing session, one step");
    }

    /// A read-only kit refuses the edit wherever it came from. The popups and
    /// the reference picker applied theirs regardless, because only the pane
    /// checked.
    #[test]
    fn a_read_only_kit_refuses_every_ui_edit() {
        let mut app = app_with_open_tag();
        let profile = CustomEditingKitProfile {
            read_only: true,
            git_tracked: false,
            id: "protected".to_owned(),
            name: "Protected".to_owned(),
            game: "halo3_mcc".to_owned(),
            root: PathBuf::from("/nowhere"),
            icon: None,
            tags_folder: None,
            data_folder: None,
        };
        app.model.kits[0].profile = Some(EditingKitProfileIdentity {
            id: profile.id.clone(),
            name: profile.name.clone(),
        });
        app.model.prefs.custom_editing_kit_profiles = vec![profile];
        let before = value(&app);

        let applied = app.apply_doc_ops(0, KEY, "Edit color", set("7"), UndoStep::Own);

        assert!(applied.is_none());
        assert_eq!(value(&app), before, "the tag is unchanged");
        assert!(app.model.status.contains("read-only"), "status: {}", app.model.status);
        assert_eq!(undo_steps(&mut app), 0);
    }

    /// A draft whose value was applied is marked clean, whichever path
    /// applied it. Typed as `07`, which the field shows as `7`: only the
    /// accept step can tell that draft was applied rather than abandoned.
    #[test]
    fn an_applied_edit_marks_its_draft_clean() {
        let mut app = app_with_open_tag();
        let draft_key = format!("{KEY}|{FIELD}");
        let shown = value(&app);
        let draft = app.views[app.model.kits[0].id]
            .edit_buffers
            .draft_mut(&draft_key, &shown);
        draft.text = "07".to_owned();
        draft.changed = true;

        app.apply_doc_ops(0, KEY, "Paste TSV", set("07"), UndoStep::Own);

        let shown = value(&app);
        assert_eq!(shown, "7");
        let draft = app.views[app.model.kits[0].id].edit_buffers.take(&draft_key, &shown);
        assert!(!draft.changed, "the applied draft still reads as unsaved");
        assert_eq!(draft.text, "7");
    }
}
