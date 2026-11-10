//! Document-aware clipboard commands and linked history for cross-tag moves.
use super::*;

#[derive(Clone)]
pub(in crate::app) struct RowCommand {
    pub document: RowDocument,
    pub revision: Option<u64>,
    pub request: RowRequest,
    pub confirmed: bool,
    pub cut_token: Option<String>,
}

struct LanguageWarning {
    command: RowCommand,
    unsupported: Vec<String>,
    missing: Vec<String>,
}

impl Dialog for LanguageWarning {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let mut open = true;
        let mut proceed = false;
        let mut cancel = false;
        egui::Window::new("Move String Rows Between Languages").id(egui::Id::unique("string row language warning"))
            .open(&mut open).collapsible(false).resizable(false).default_width(440.0).show(cx.egui, |ui| {
                if !self.unsupported.is_empty() {
                    ui.label("These source languages cannot be retained in the destination:");
                    ui.colored_label(ui.visuals().warn_fg_color, self.unsupported.join(", "));
                }
                if !self.missing.is_empty() {
                    ui.label("These destination languages are absent from the source:");
                    ui.label(self.missing.join(", "));
                }
                ui.label("Existing translations in languages absent from the source are preserved. New rows have no text in those languages.");
                ui.horizontal(|ui| {
                    proceed = icon_text_button(ui, ButtonIcon::Paste, "Move Rows", true).clicked();
                    cancel = icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked();
                });
            });
        if proceed {
            self.command.confirmed = true;
            cx.send(EditorCommand::StringRows(Box::new(self.command.clone())));
        }
        open && !proceed && !cancel
    }
}

impl RowDocument {
    fn resolve(&self, model: &Model) -> Option<usize> {
        let index = model.kit_index(self.kit)?;
        model.kits[index].parsed_tags.get(&self.key).filter(|doc| doc.id == self.id)?;
        Some(index)
    }
}

impl Baboon {
    pub(in crate::app) fn apply_string_row_command(&mut self, command: RowCommand, ctx: &egui::Context) {
        if let Err(error) = self.try_string_row_command(command, ctx) { self.model.status = error; }
        ctx.request_repaint();
    }

    fn try_string_row_command(&mut self, mut command: RowCommand, ctx: &egui::Context) -> Result<(), String> {
        refresh_cut(&self.model, ctx);
        let index = command.document.resolve(&self.model).ok_or("The string-list document is no longer open.")?;
        let document = &self.model.kits[index].parsed_tags[&command.document.key];
        if command.revision.is_some_and(|revision| revision != document.dirty.revision()) {
            return Err("The destination changed while the move warning was open. Paste again to review the current rows.".to_owned());
        }
        if self.views[command.document.kit].edit_buffers.has_changed_prefix(&format!("{}|strings|", command.document.key)) {
            return Err("Finish or cancel the current string edit before copying or pasting rows.".to_owned());
        }
        let list = strings::read_string_list(&document.tag).ok_or("This tag has no readable string list.")?;
        let game = self.model.kits[index].source.as_ref().and_then(|source| source.game);
        let syntax = list.storage.text_syntax(self.row_syntax(index));
        match command.request.action.clone() {
            RowAction::Copy { cut: cutting } => {
                if cutting && list.storage == Storage::HudMessages && self.hud_message_usage(index, &command.document.key).locked(self.model.prefs.expert_mode) { return Err(super::super::hud_messages::EXPERT_REQUIRED.to_owned()); }
                if cutting && self.model.editing_kit_is_read_only(index) { return Err("This tag is read-only.".to_owned()); }
                let indices: BTreeSet<_> = command.request.selected.iter().copied().filter(|&index| index < list.entries.len()).collect();
                if indices.is_empty() { return Ok(()); }
                let data = RowData {
                    hud: list.storage == Storage::HudMessages,
                    token: uuid::Uuid::new_v4().to_string(), classic: list.storage == Storage::Classic,
                    languages: list.languages.iter().map(|language| language.name.clone()).collect(),
                    language: command.request.language.min(list.languages.len() - 1),
                    rows: indices.iter().map(|&index| Row { id: list.entries[index].id.clone(), texts: list.entries[index].texts.clone() }).collect(),
                };
                write(ctx, &data, &syntax)?;
                let active = ActiveRows { cutting, source: command.document, revision: document.dirty.revision(), indices, token: data.token.clone(), data };
                set_active(ctx, active);
                self.model.status = if cutting { "Cut rows ready to move. Escape cancels the move." } else { "Copied string rows, including all languages. Escape clears copy mode." }.to_owned();
            }
            RowAction::Paste { mode, data } => {
                if self.model.editing_kit_is_read_only(index) { return Err("This tag is read-only.".to_owned()); }
                if cut(ctx).is_some_and(|active| self.views[active.source.kit].edit_buffers.has_changed_prefix(&format!("{}|strings|", active.source.key))) {
                    cancel_cut(ctx);
                }
                let active = cut(ctx).filter(|active| active.token == data.token && active.data == data);
                if command.confirmed && command.cut_token.is_some() && active.is_none() {
                    return Err("The cut source changed. The move was cancelled; the clipboard can still be pasted as a copy.".to_owned());
                }
                let mut source = None;
                let mut local_cut = BTreeSet::new();
                if let Some(active) = &active {
                    let source_index = active.source.resolve(&self.model).ok_or("The cut source is no longer open.")?;
                    if self.model.editing_kit_is_read_only(source_index) { return Err("The cut source is read-only.".to_owned()); }
                    let source_doc = &self.model.kits[source_index].parsed_tags[&active.source.key];
                    let source_list = strings::read_string_list(&source_doc.tag).ok_or("The cut source is not a readable string list.")?;
                    if source_list.storage != list.storage { return Err("Cut cannot move between CE and multilingual lists. Copy the rows instead.".to_owned()); }
                    if active.source == command.document { local_cut = active.indices.clone(); }
                    else { source = Some((active.source.clone(), source_index, source_list)); }
                }
                let (unsupported, missing) = language_difference(&data, &list);
                if active.is_some() && !command.confirmed && (!unsupported.is_empty() || !missing.is_empty()) {
                    command.revision = Some(document.dirty.revision());
                    command.cut_token = active.as_ref().map(|active| active.token.clone());
                    self.dialogs.open(LanguageWarning { command, unsupported, missing });
                    return Ok(());
                }
                let plan = plan(&list, &data, command.request.language.min(list.languages.len() - 1), &command.request.selected, mode, &local_cut)?;
                let mut candidates = Vec::new();
                if plan.rows != list.entries || source.is_some() {
                    candidates.push((command.document.clone(), index, self.row_candidate(index, &command.document.key, &list, plan.rows, game)?));
                }
                if let Some((source_doc, source_index, source_list)) = source {
                    let indices = &active.as_ref().unwrap().indices;
                    let rows = source_list.entries.iter().enumerate().filter(|(index, _)| !indices.contains(index)).map(|(_, row)| row.clone()).collect();
                    let source_game = self.model.kits[source_index].source.as_ref().and_then(|source| source.game);
                    let candidate = self.row_candidate(source_index, &source_doc.key, &source_list, rows, source_game)?;
                    candidates.push((source_doc, source_index, candidate));
                }
                // Both candidates have been parsed and fully rebuilt before either
                // journal or document changes. This is one all-or-nothing move.
                let mut linked = Vec::new();
                for (address, kit_index, candidate) in candidates {
                    let doc = self.model.kits[kit_index].parsed_tags.get_mut(&address.key).unwrap();
                    doc.journal.end_edit_window();
                    doc.journal.begin_edit(&doc.tag, if active.is_some() { "Move String Rows" } else { "Paste String Rows" });
                    doc.journal.end_edit_window();
                    doc.tag = candidate;
                    doc.dirty.touch();
                    doc.note_layout_change();
                    linked.push((address.clone(), doc.journal.top_id().unwrap()));
                    self.views[address.kit].edit_buffers.forget_tag(&address.key);
                    self.invalidate_tag_caches_in(kit_index, &address.key);
                }
                if linked.len() == 2 {
                    self.editor.string_row_links.push(LinkedMove { documents: [linked[0].0.clone(), linked[1].0.clone()], steps: [linked[0].1, linked[1].1], redo: false });
                }
                let clipboard_error = if active.is_some() { consume_cut(ctx, &data).err() } else { None };
                super::super::string_list::select_pasted_rows(ctx, &command.request.scope, &command.document.key, plan.selected);
                if !self.search.find.strings.as_ref().is_some_and(|options| options.selected_rows)
                    && self.search.find.string_context.as_ref().is_some_and(|context| context.kit == command.document.kit && context.tag_key == command.document.key) { self.search.find.filter_results = false; }
                self.model.status = if unsupported.is_empty() && missing.is_empty() { "Pasted string rows.".to_owned() }
                    else { format!("Pasted string rows. Unsupported source languages: {}. Destination languages absent from source: {}.", unsupported.join(", "), missing.join(", ")) };
                if let Some(error) = clipboard_error { self.model.status.push_str(&format!(" {error}")); }
            }
        }
        Ok(())
    }

    fn row_syntax(&self, index: usize) -> TextSyntax {
        let source = self.model.kits[index].source.as_ref();
        let definitions = source.and_then(|source| match &source.source { TagSource::LooseFolder { definitions_root, .. } => Some(definitions_root.as_path()), _ => None });
        super::super::string_list::text_syntax(definitions, source.and_then(|source| source.game))
    }

    fn row_parse(&self, index: usize, key: &str, bytes: &[u8]) -> Result<TagFile, String> {
        let kit = &self.model.kits[index];
        let source = kit.source.as_ref();
        let definitions = source.and_then(|source| match &source.source { TagSource::LooseFolder { definitions_root, .. } => Some(definitions_root.as_path()), _ => None });
        crate::core::source::read_tag_from_bytes(bytes, source.and_then(|source| source.game), definitions, kit.parsed_tags[key].tag.group().tag).map_err(|error| error.to_string())
    }

    fn row_candidate(&self, index: usize, key: &str, list: &StringList, rows: Vec<StringEntry>, game: Option<GameId>) -> Result<TagFile, String> {
        if list.storage == Storage::HudMessages { self.check_hud_rows(index, key, &list.entries, &rows)?; }
        let bytes = self.model.kits[index].parsed_tags[key].tag.write_to_bytes().map_err(|error| error.to_string())?;
        let mut candidate = self.row_parse(index, key, &bytes)?;
        let edits = replace_ops(list, rows, game).string_list_ops.into_iter().map(|op| op.edit).collect::<Vec<_>>();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| strings::apply_string_list_edits(&mut candidate, game, &edits)))
            .map_err(|_| "The engine could not rebuild the pasted string rows.".to_owned())??;
        // Also verify the edited representation can be serialized before commit.
        candidate.write_to_bytes().map_err(|error| error.to_string())?;
        Ok(candidate)
    }

    /// Return true when a linked step handled (or deliberately blocked) undo.
    /// A closed/reloaded peer dissolves the link; the surviving local history
    /// remains a normal undo/redo history, as requested.
    pub(in crate::app) fn step_linked_string_rows(&mut self, redo: bool) -> bool {
        let index = self.model.active;
        let Some(key) = self.model.kits[index].selected_key.as_ref() else { return false; };
        let Some(doc) = self.model.kits[index].parsed_tags.get(key) else { return false; };
        let stack = if redo { doc.journal.stacks().1 } else { doc.journal.stacks().0 };
        let Some(step) = stack.last().map(|step| step.id) else { return false; };
        let address = RowDocument { kit: self.model.kits[index].id, key: key.clone(), id: doc.id };
        let Some(link_index) = self.editor.string_row_links.iter().position(|link| link.redo == redo && (0..2).any(|i| link.documents[i] == address && link.steps[i] == step)) else { return false; };
        let link = self.editor.string_row_links[link_index].clone();
        let Some(indices) = link.documents.iter().map(|doc| doc.resolve(&self.model)).collect::<Option<Vec<_>>>() else {
            self.editor.string_row_links.remove(link_index);
            return false;
        };
        let mut tags = Vec::new();
        for i in 0..2 {
            let doc = &self.model.kits[indices[i]].parsed_tags[&link.documents[i].key];
            let stack = if redo { doc.journal.stacks().1 } else { doc.journal.stacks().0 };
            if !stack.iter().any(|snapshot| snapshot.id == link.steps[i]) {
                self.model.status = "The other tag's history changed. This linked move cannot be stepped while both tags are open.".to_owned();
                return true;
            }
            if stack.last().is_none_or(|snapshot| snapshot.id != link.steps[i]) {
                self.model.status = if redo { "Redo the earlier steps in the other tag before redoing this linked move." } else { "Undo the newer edits in the other tag before undoing this linked move." }.to_owned();
                return true;
            }
            if self.model.editing_kit_is_read_only(indices[i]) { self.model.status = "Both tags must be writable to undo this linked move.".to_owned(); return true; }
            if doc.tag.write_to_bytes().is_err() {
                self.model.status = "Finish correcting the tag before undoing this linked move.".to_owned();
                return true;
            }
            match self.row_parse(indices[i], &link.documents[i].key, &stack.last().unwrap().bytes) {
                Ok(tag) => tags.push(tag),
                Err(error) => { self.model.status = format!("Could not restore the linked move: {error}"); return true; }
            }
        }
        let mut steps = [0; 2];
        for (i, tag) in tags.into_iter().enumerate() {
            let address = &link.documents[i];
            let doc = self.model.kits[indices[i]].parsed_tags.get_mut(&address.key).unwrap();
            if redo { doc.journal.redo(&doc.tag); } else { doc.journal.undo(&doc.tag); }
            doc.tag = tag;
            doc.dirty.touch();
            doc.note_layout_change();
            doc.settle_after_step();
            super::string_list::clear_selection_after_history(&self.egui_ctx,doc.id);
            let opposite = if redo { doc.journal.stacks().0 } else { doc.journal.stacks().1 };
            steps[i] = opposite.last().unwrap().id;
            self.views[address.kit].edit_buffers.forget_tag(&address.key);
            self.invalidate_tag_caches_in(indices[i], &address.key);
        }
        self.editor.string_row_links[link_index].steps = steps;
        self.editor.string_row_links[link_index].redo = !redo;
        self.model.status = if redo { "Redid linked string-row move." } else { "Undid linked string-row move." }.to_owned();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::LooseKit;

    fn fixture() -> (Baboon, LooseKit, egui::Context, String, String) {
        let kit = LooseKit::new("row-clipboard", "halo3_mcc");
        for (path, names) in [("ui\\a", ["A", "B", "C"]), ("ui\\b", ["D", "E", "F"])] {
            kit.write_mcc(path, "multilingual_unicode_string_list", |tag| {
                let list = strings::read_string_list(tag).unwrap();
                let entries = names.iter().map(|name| {
                    let mut entry = list.new_entry();
                    entry.id = Some((*name).to_owned());
                    entry.texts[0] = Some((*name).to_owned());
                    entry.texts[1] = Some(format!("{name} 日本語"));
                    entry
                }).collect();
                strings::apply_string_list_edits(tag, Some(GameId::Halo3), &[StringListEdit::Insert { at: 0, entries }]).unwrap();
            });
        }
        let mut app = Baboon::for_test();
        kit.install(&mut app);
        let a = kit.open(&mut app, "ui\\a.multilingual_unicode_string_list");
        let b = kit.open(&mut app, "ui\\b.multilingual_unicode_string_list");
        (app, kit, egui::Context::default(), a, b)
    }

    fn list(app: &Baboon, key: &str) -> StringList { strings::read_string_list(&app.model.kits[0].parsed_tags[key].tag).unwrap() }

    fn command(app: &Baboon, key: &str, selected: &[usize], action: RowAction) -> RowCommand {
        RowCommand { document: RowDocument { kit: app.model.kits[0].id, key: key.to_owned(), id: app.model.kits[0].parsed_tags[key].id },
            revision: None, request: RowRequest { selected: selected.iter().copied().collect(), action, language: 0, scope: "row-test".to_owned() }, confirmed: false, cut_token: None }
    }

    fn copy(app: &mut Baboon, ctx: &egui::Context, key: &str, selected: &[usize], cut: bool) {
        let command = command(app, key, selected, RowAction::Copy { cut });
        app.apply_string_row_command(command, ctx);
    }

    fn paste(app: &mut Baboon, ctx: &egui::Context, key: &str, selected: &[usize], mode: PasteMode) {
        let data = clipboard(ctx, &list(app, key), 0, &app.row_syntax(0)).unwrap();
        let command = command(app, key, selected, RowAction::Paste { mode, data });
        app.apply_string_row_command(command, ctx);
    }

    #[test]
    fn hud_cross_tag_cut_preserves_tokens_names_and_linked_undo() {
        let kit=crate::app::loose_fixture::LooseKit::new("hud-cut", "haloce_mcc");
        for (file,name,text) in [("a","pickup","Hold %{18} %{32}"),("b","exit","%{29}")] {
            let mut tag=TagFile::new_classic(crate::core::bundled::locate_definitions_root().join("haloce_mcc/hud_message_text.json"),blam_tags::classic::ClassicEngine::HaloCe).unwrap();
            strings::apply_string_list_edits(&mut tag,Some(GameId::HaloCe),&[StringListEdit::Insert {at:0,entries:vec![StringEntry {id:Some(name.into()),texts:vec![Some(text.into())]}]}]).unwrap();
            std::fs::write(kit.path(&format!("{file}.hud_message_text")),tag.write_to_bytes().unwrap()).unwrap();
        }
        let mut app=Baboon::for_test(); kit.install(&mut app);
        let a=kit.open(&mut app,"a.hud_message_text"); let b=kit.open(&mut app,"b.hud_message_text"); let ctx=egui::Context::default();
        let before_a=app.model.kits[0].parsed_tags[&a].tag.write_to_bytes().unwrap(); let before_b=app.model.kits[0].parsed_tags[&b].tag.write_to_bytes().unwrap();
        copy(&mut app,&ctx,&a,&[0],true); paste(&mut app,&ctx,&b,&[0],PasteMode::Below);
        assert!(list(&app,&a).entries.is_empty()); assert_eq!(list(&app,&b).entries[1].texts[0].as_deref(),Some("Hold %{18} %{32}"));
        assert_eq!(list(&app,&b).entries[1].id.as_deref(),Some("pickup"));
        app.undo_current_tag();
        assert_eq!(app.model.kits[0].parsed_tags[&a].tag.write_to_bytes().unwrap(),before_a); assert_eq!(app.model.kits[0].parsed_tags[&b].tag.write_to_bytes().unwrap(),before_b);
    }

    #[test]
    fn escape_clears_copy_or_cut_mode_without_reactivating_it_from_the_clipboard() {
        for cutting in [false, true] {
            let (mut app, _kit, ctx, a, _) = fixture();
            copy(&mut app, &ctx, &a, &[0, 2], cutting);
            let active = active(&ctx).unwrap();
            assert_eq!(active.cutting, cutting);
            assert_eq!(active.indices, BTreeSet::from([0, 2]));
            let source = list(&app, &a);
            let syntax = app.row_syntax(0);
            assert!(clipboard(&ctx, &source, 0, &syntax).is_some());
            assert!(insert_active(&ctx));
            dismiss(&ctx);
            assert!(super::active(&ctx).is_none());
            for _ in 0..3 {
                assert!(clipboard(&ctx, &source, 0, &syntax).is_some(), "Escape leaves the system clipboard intact");
                assert!(!insert_active(&ctx), "reopening Insert must not reactivate dismissed rows");
            }
            copy(&mut app, &ctx, &a, &[1], false);
            assert!(clipboard(&ctx, &source, 0, &syntax).is_some());
            assert!(insert_active(&ctx), "a new Copy reactivates row insertion");
            assert!(!super::active(&ctx).unwrap().cutting);
        }
    }

    #[test]
    fn successful_cut_pastes_consume_the_clipboard_without_turning_into_copies() {
        for mode in [PasteMode::Overwrite, PasteMode::Above, PasteMode::Below] {
            let (mut app, _kit, ctx, a, b) = fixture();
            copy(&mut app, &ctx, &a, &[0], true);
            let snapshot = ctx.data(|memory| memory.get_temp::<(String, String)>(egui::Id::unique(CLIPBOARD_ID))).unwrap();
            paste(&mut app, &ctx, &b, &[1], mode);
            assert!(active(&ctx).is_none(), "a completed move must not become Copy mode");
            assert!(clipboard(&ctx, &list(&app, &b), 0, &app.row_syntax(0)).is_none());
            // A busy OS clipboard must not make the consumed cut reusable.
            ctx.data_mut(|memory| memory.insert_temp(egui::Id::unique(CLIPBOARD_ID), snapshot));
            assert!(clipboard(&ctx, &list(&app, &b), 0, &app.row_syntax(0)).is_none());
            copy(&mut app, &ctx, &b, &[0], false);
            assert!(clipboard(&ctx, &list(&app, &b), 0, &app.row_syntax(0)).is_some(), "an explicit new Copy works normally");
        }
    }

    #[test]
    fn row_copy_overwrites_ids_and_all_languages_and_undo_restores_exact_bytes() {
        let (mut app, _kit, ctx, a, _) = fixture();
        let bytes = app.model.kits[0].parsed_tags[&a].tag.write_to_bytes().unwrap();
        copy(&mut app, &ctx, &a, &[0, 2], false);
        paste(&mut app, &ctx, &a, &[1], PasteMode::Overwrite);
        let result = list(&app, &a);
        assert_eq!(result.entries.iter().map(|row| row.id.as_deref().unwrap()).collect::<Vec<_>>(), ["A", "A (1)", "C"]);
        assert_eq!(result.entries[1].texts[1].as_deref(), Some("A 日本語"));
        assert_eq!(result.damaged_offsets, 0);
        app.model.kits[0].selected_key = Some(a.clone());
        app.undo_current_tag();
        assert_eq!(app.model.kits[0].parsed_tags[&a].tag.write_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn cross_tag_cut_undo_redo_can_be_started_from_either_tag() {
        let (mut app, _kit, ctx, a, b) = fixture();
        let before_a = list(&app, &a);
        let before_b = list(&app, &b);
        copy(&mut app, &ctx, &a, &[0, 2], true);
        assert!(cut(&ctx).is_some());
        paste(&mut app, &ctx, &b, &[1], PasteMode::Overwrite);
        assert!(cut(&ctx).is_none());
        assert_eq!(list(&app, &a).entries.len(), 1);
        assert_eq!(list(&app, &b).entries[1].id.as_deref(), Some("A"));
        assert_eq!(app.editor.string_row_links.len(), 1);
        app.model.kits[0].selected_key = Some(a.clone());
        app.undo_current_tag();
        assert_eq!(list(&app, &a), before_a);
        assert_eq!(list(&app, &b), before_b);
        app.model.kits[0].selected_key = Some(b.clone());
        app.redo_current_tag();
        assert_eq!(list(&app, &a).entries.len(), 1);
        assert_eq!(list(&app, &b).entries[1].id.as_deref(), Some("A"));
        app.undo_current_tag();
        assert_eq!(list(&app, &a), before_a);
        assert_eq!(list(&app, &b), before_b);
    }

    #[test]
    fn linked_undo_preserves_newer_edits_and_survives_peer_closure_locally() {
        let (mut app, _kit, ctx, a, b) = fixture();
        copy(&mut app, &ctx, &a, &[0], true);
        paste(&mut app, &ctx, &b, &[0], PasteMode::Above);
        app.apply_doc_ops(0, &b, "Later edit", DeferredOps { string_list_ops: vec![crate::core::document::ops::StringListOp {
            game: Some(GameId::Halo3), edit: StringListEdit::SetText { index: 0, language: 0, text: "Later".to_owned() }
        }], ..Default::default() }, UndoStep::Own);
        app.model.kits[0].selected_key = Some(a.clone());
        app.undo_current_tag();
        assert!(app.model.status.contains("newer edits"));
        assert_eq!(list(&app, &a).entries.len(), 2);
        assert_eq!(list(&app, &b).entries[0].texts[0].as_deref(), Some("Later"));
        app.model.kits[0].selected_key = Some(b.clone());
        app.undo_current_tag();
        app.model.kits[0].selected_key = Some(a.clone());
        app.undo_current_tag();
        assert_eq!(list(&app, &a).entries.len(), 3);
        app.redo_current_tag();
        app.model.kits[0].parsed_tags.remove(&b);
        app.undo_current_tag();
        assert_eq!(list(&app, &a).entries.len(), 3, "closing a peer leaves this tag's undo usable");
    }

    #[test]
    fn branching_a_peer_history_never_redoes_only_half_of_a_move() {
        let (mut app, _kit, ctx, a, b) = fixture();
        copy(&mut app, &ctx, &a, &[0], true);
        paste(&mut app, &ctx, &b, &[0], PasteMode::Above);
        app.model.kits[0].selected_key = Some(b.clone());
        app.undo_current_tag();
        let target_before = list(&app, &b);
        app.apply_doc_ops(0, &a, "New branch", DeferredOps { string_list_ops: vec![crate::core::document::ops::StringListOp {
            game: Some(GameId::Halo3), edit: StringListEdit::SetText { index: 0, language: 0, text: "Changed branch".to_owned() }
        }], ..Default::default() }, UndoStep::Own);
        app.redo_current_tag();
        assert!(app.model.status.contains("history changed"));
        assert_eq!(list(&app, &b), target_before);
        assert_eq!(list(&app, &a).entries[0].texts[0].as_deref(), Some("Changed branch"));
    }

    #[test]
    fn source_edits_demote_cut_to_copy_and_repeated_paste_does_not_move_twice() {
        let (mut app, _kit, ctx, a, b) = fixture();
        copy(&mut app, &ctx, &a, &[0], true);
        app.apply_doc_ops(0, &a, "Edit source", DeferredOps { string_list_ops: vec![crate::core::document::ops::StringListOp {
            game: Some(GameId::Halo3), edit: StringListEdit::SetText { index: 1, language: 0, text: "Changed".to_owned() }
        }], ..Default::default() }, UndoStep::Own);
        paste(&mut app, &ctx, &b, &[0], PasteMode::Above);
        assert!(cut(&ctx).is_none());
        assert_eq!(list(&app, &a).entries.len(), 3);
        assert_eq!(list(&app, &b).entries[0].id.as_deref(), Some("A"));
        paste(&mut app, &ctx, &b, &[1], PasteMode::Above);
        assert_eq!(list(&app, &b).entries[1].id.as_deref(), Some("A (1)"));
    }

    #[test]
    fn cross_engine_cut_warns_then_moves_atomically_and_undoes_across_kits() {
        let (mut app, _kit, ctx, a, _) = fixture();
        copy(&mut app, &ctx, &a, &[0], true);
        let source_before = list(&app, &a);
        let other = LooseKit::new("row-clipboard-h2", "halo2_mcc");
        other.write_mcc("ui\\target", "multilingual_unicode_string_list", |_| {});
        let target_kit = app.add_kit();
        other.install(&mut app);
        let b = other.open(&mut app, "ui\\target.multilingual_unicode_string_list");
        let target = strings::read_string_list(&app.model.kits[1].parsed_tags[&b].tag).unwrap();
        let data = clipboard(&ctx, &target, 0, &app.row_syntax(1)).unwrap();
        let command = RowCommand { document: RowDocument { kit: target_kit, key: b.clone(), id: app.model.kits[1].parsed_tags[&b].id }, revision: None,
            request: RowRequest { selected: BTreeSet::new(), language: 0, scope: "row-h2".to_owned(), action: RowAction::Paste { mode: PasteMode::Above, data } }, confirmed: false, cut_token: None };
        app.apply_string_row_command(command, &ctx);
        assert_eq!(list(&app, &a), source_before, "no source mutation before warning confirmation");
        let warning = app.dialogs.get::<LanguageWarning>().unwrap();
        assert!(!warning.unsupported.is_empty());
        let mut command = warning.command.clone();
        command.confirmed = true;
        app.apply_string_row_command(command, &ctx);
        assert_eq!(list(&app, &a).entries.len(), 2);
        assert_eq!(strings::read_string_list(&app.model.kits[1].parsed_tags[&b].tag).unwrap().entries.len(), 1, "{}", app.model.status);
        app.model.kits[1].selected_key = Some(b.clone());
        app.undo_current_tag();
        assert_eq!(list(&app, &a), source_before);
        assert!(strings::read_string_list(&app.model.kits[1].parsed_tags[&b].tag).unwrap().entries.is_empty());
        app.model.active = 0;
        app.model.kits[0].selected_key = Some(a.clone());
        app.redo_current_tag();
        assert_eq!(list(&app, &a).entries.len(), 2);
    }

    #[test]
    fn copying_to_a_classic_ce_tag_preflights_and_undoes_its_native_format() {
        let (mut app, _kit, ctx, a, _) = fixture();
        copy(&mut app, &ctx, &a, &[0], false);
        let other = LooseKit::new("row-clipboard-ce", "haloce_mcc");
        other.write_classic_ce("ui\\target", "unicode_string_list");
        let target_kit = app.add_kit();
        other.install(&mut app);
        let b = other.open(&mut app, "ui\\target.unicode_string_list");
        let target = strings::read_string_list(&app.model.kits[1].parsed_tags[&b].tag).unwrap();
        let data = clipboard(&ctx, &target, 0, &app.row_syntax(1)).unwrap();
        let command = RowCommand { document: RowDocument { kit: target_kit, key: b.clone(), id: app.model.kits[1].parsed_tags[&b].id }, revision: None,
            request: RowRequest { selected: BTreeSet::new(), language: 0, scope: "row-ce".to_owned(), action: RowAction::Paste { mode: PasteMode::Overwrite, data } }, confirmed: false, cut_token: None };
        app.apply_string_row_command(command, &ctx);
        assert_eq!(strings::read_string_list(&app.model.kits[1].parsed_tags[&b].tag).unwrap().entries[0].texts[0].as_deref(), Some("A"), "{}", app.model.status);
        app.model.kits[1].selected_key = Some(b.clone());
        app.undo_current_tag();
        assert!(strings::read_string_list(&app.model.kits[1].parsed_tags[&b].tag).unwrap().entries.is_empty());
    }

    #[test]
    fn invalid_destination_preflight_leaves_cut_source_and_both_histories_untouched() {
        let (mut app, _kit, ctx, a, b) = fixture();
        copy(&mut app, &ctx, &a, &[0], true);
        let source = list(&app, &a);
        let target = list(&app, &b);
        let mut data = clipboard(&ctx, &target, 0, &app.row_syntax(0)).unwrap();
        data.rows[0].texts[0] = Some("x".repeat(target.max_text_units() + 1));
        let request = command(&app, &b, &[0], RowAction::Paste { mode: PasteMode::Overwrite, data });
        app.apply_string_row_command(request, &ctx);
        assert_eq!(list(&app, &a), source);
        assert_eq!(list(&app, &b), target);
        assert!(!app.model.kits[0].parsed_tags[&a].journal.can_undo());
        assert!(!app.model.kits[0].parsed_tags[&b].journal.can_undo());
    }
}
