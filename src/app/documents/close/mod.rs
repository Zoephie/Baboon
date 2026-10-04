//! Closing tabs and the app: which documents are dirty, the save-changes
//! prompt, and discarding changes.

use super::*;

impl Baboon {
    /// Throw away a tag's unsaved edits and put it back the way its source has
    /// it: drop the parsed document and everything derived from it, forget any
    /// stashed Campaign Evolved overlay, then reload the tag if it is open.
    ///
    /// Forgetting the overlay is the load-bearing half for a container source.
    /// The project autosaves every dirty tag within a second of the edit, so
    /// clearing the dirty flag alone leaves the edited bytes stashed and
    /// reopening the tag restores them — the edit would be unremovable.
    pub(in crate::app) fn discard_tag_changes(&mut self, kit: usize, key: &str, ctx: &egui::Context) {
        // Reloading below goes through the active-kit path, and discarding is a
        // user action on this kit either way.
        self.model.active = kit;
        let was_dirty = self.model.kits[kit]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set());
        let had_overlay = self.forget_campaign_overlay(kit, key);
        if !was_dirty && !had_overlay {
            self.model.status = "That tag has no unsaved changes".to_owned();
            return;
        }
        let label = self.tag_path_label(key);
        let kit_state = &mut self.model.kits[kit];
        let view = &mut self.views[kit_state.id];
        kit_state.parsed_tags.remove(key);
        kit_state.loading_tags.remove(key);
        view.caches.bitmap_previews.remove(key);
        view.caches.model_previews.remove(key);
        view.find_filter_applied.remove(key);
        view.edit_buffers.forget_tag(key);
        // Persist the removal. The document is gone by now, so the capture
        // below cannot put the overlay straight back.
        if had_overlay {
            let now = ctx.input(|input| input.time);
            if let Err(error) = self.checkpoint_campaign_project(kit, now) {
                self.model.status = format!("Could not update the Campaign Evolved project: {error}");
                return;
            }
        }
        // A brand-new tag has no source to reload from — the document that was
        // just dropped WAS the tag. Take its browser entry with it instead of
        // leaving a row that errors on every reopen.
        if self.forget_new_container_entry(kit, key) {
            self.model.status = format!("Discarded the unsaved new tag {label}");
            return;
        }
        // Still open: reload it as the source has it, rather than leaving an
        // empty pane behind.
        if self.model.kits[kit].open_tabs.iter().any(|open| open == key) {
            self.select_entry(key.to_owned(), ctx.clone());
        }
        self.model.status = format!("Discarded unsaved changes to {label}");
    }

    /// Drop a brand-new (never-saved) container tag's browser entry, closing its
    /// pane and dropping everything derived from it. No-op — returning `false` —
    /// for any other kind of entry.
    ///
    /// Load-bearing for every path that discards a new tag's document: the
    /// document is the tag's ONLY copy (there is no `.ubulk` behind it), so an
    /// entry that outlives it is a row whose every reopen fails in `read_entry`
    /// with "unsaved new tag is no longer loaded".
    pub(in crate::app) fn forget_new_container_entry(&mut self, kit: usize, key: &str) -> bool {
        if !matches!(
            self.entry_for_key_in(kit, key).map(|entry| &entry.location),
            Some(TagEntryLocation::NewContainer { .. })
        ) {
            return false;
        }
        self.kit_and_view(kit).close_tag_pane(key);
        let folder_seeds = self.model.kits[kit].folder_seeds();
        let kit_state = &mut self.model.kits[kit];
        let view = &mut self.views[kit_state.id];
        kit_state.parsed_tags.remove(key);
        kit_state.loading_tags.remove(key);
        view.caches.bitmap_previews.remove(key);
        view.caches.model_previews.remove(key);
        view.find_filter_applied.remove(key);
        view.edit_buffers.forget_tag(key);
        if kit_state.selected_key.as_deref() == Some(key) {
            kit_state.selected_key = None;
        }
        if let Some(source) = kit_state.source.as_mut() {
            source.remove_entry(key, &folder_seeds);
        }
        kit_state.set_tag_references(key, None);
        kit_state.generation = kit_state.generation.wrapping_add(1);
        true
    }

    /// Whether `key` has anything to discard — unsaved edits, or bytes stashed
    /// in this kit's project from an earlier session.
    pub(in crate::app) fn tag_has_discardable_changes(&self, kit: usize, key: &str) -> bool {
        self.model.kits[kit]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set())
            || self.tag_has_stashed_overlay(kit, key)
    }

    pub(in crate::app) fn close_tab(&mut self, key: &str) {
        // `close_tag_pane` re-derives the open set and moves the selection off
        // a removed tag, so there is nothing to fix up afterwards.
        self.kit_and_view(self.model.active).close_tag_pane(key);
        self.unload_tag(key);
        self.editor.color_popup = None;
        self.editor.function_popup = None;
    }

    pub(in crate::app) fn request_close_action(&mut self, action: PendingCloseAction, ctx: &egui::Context) {
        // Chimp's recovery checkpoints wait for edits to pause; one still
        // waiting when the app or a workspace closes would be lost.
        self.flush_all_chimp_checkpoints();
        if self.documents.save_changes_prompt.visible
            || self.chimp.chimp_discard_prompt.is_some()
            || self.has_chimp_save_dialog()
        {
            return;
        }
        // A Chimp save is writing containers on a worker. The close waits
        // for it and runs from its completion, like a close the save dialog
        // was opened for.
        let writing = match &action {
            PendingCloseAction::CloseApp => self.chimp.chimp_writes.keys().next().copied(),
            PendingCloseAction::CloseKit(id) => self.chimp.chimp_writes.contains_key(id).then_some(*id),
            _ => None,
        };
        if let Some(kit) = writing {
            self.chimp.chimp_writes.insert(kit, Some(action));
            self.model.status = "Closing once the Chimp save finishes…".to_owned();
            return;
        }
        // The save prompt and every save path below it address documents by
        // tag key against the active kit. Point `active` at the kit the prompt
        // will be about first, so all of that — including the project check
        // just below — resolves against the right kit.
        match &action {
            PendingCloseAction::CloseKit(id) => {
                if let Some(index) = self.kit_index(*id) {
                    self.model.active = index;
                }
            }
            PendingCloseAction::CloseApp => {
                if let Some(index) = self.first_dirty_kit() {
                    self.model.active = index;
                }
            }
            _ => {}
        }
        // Upstream skipped the unsaved-changes prompt entirely for a container
        // source, checkpointing the project instead on the grounds that the
        // project retains the edits. It does — but silently, and there was no
        // way to say no: overlays were only ever inserted, so an edit could not
        // be taken back once stashed. The prompt is raised for these sources
        // too, and offers stashing as a third, named choice.
        let can_stash = self.current_source_is_campaign_project_capable(self.model.active);
        let dirty_tags = self.dirty_tags_for_close_action(&action);
        if !dirty_tags.is_empty() {
            // What discarding would cost, resolved here rather than described in the
            // abstract: these edits were stashed into the workspace's project within
            // a second of being typed, so declining to save deletes them from a file
            // that outlives the session.
            let stashed = dirty_tags
                .iter()
                .filter(|entry| self.tag_has_stashed_overlay(self.model.active, &entry.tag_id))
                .count();
            let stash_file = self.model.kits[self.model.active]
                .project.active
                .as_ref()
                .map(|project| project.recovery_path.clone());
            self.documents.save_changes_prompt = SaveChangesPrompt {
                visible: true,
                can_stash,
                dirty_tags,
                pending_action: action,
                error: None,
                allow_app_close_once: self.documents.save_changes_prompt.allow_app_close_once,
                stash_file,
                stashed,
                confirm_discard: false,
            };
            return;
        }

        let chimp_packages = self.dirty_chimp_for_close_action(&action);
        if !chimp_packages.is_empty() {
            self.open_chimp_discard_prompt(self.model.active, chimp_packages, Some(action), None);
            return;
        }

        self.execute_close_action(action, ctx);
    }

    /// Native app close is a two-step flow in eframe 0.29: when the OS close
    /// request arrives, Baboon sends `CancelClose` to veto it, shows the shared
    /// save prompt, then re-issues `ViewportCommand::Close` only after the user
    /// chooses Save or Don't Save. `allow_app_close_once` lets that confirmed
    /// second close request pass without opening the prompt again.
    pub(in crate::app) fn handle_app_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        if self.documents.save_changes_prompt.allow_app_close_once {
            self.documents.save_changes_prompt.allow_app_close_once = false;
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        // Quitting would kill the worker partway through rewriting references,
        // leaving some tags pointing at a path that no longer exists.
        if self.tag_ops.folder_refactor.is_some() {
            self.model.status = "Wait for the folder move/rename to finish before closing".to_owned();
            return;
        }
        if self.documents.save_changes_prompt.visible
            || self.chimp.chimp_discard_prompt.is_some()
            || self.has_chimp_save_dialog()
        {
            return;
        }
        self.defer_file_action(DeferredFileAction::Close(PendingCloseAction::CloseApp), ctx);
    }

    pub(in crate::app) fn dirty_tags_for_close_action(&self, action: &PendingCloseAction) -> Vec<DirtyTagEntry> {
        self.close_action_tag_keys(action)
            .into_iter()
            .filter_map(|key| {
                let doc = self.model.kits[self.model.active].parsed_tags.get(&key)?;
                if !doc.dirty.is_set() {
                    return None;
                }
                // Edits to a tag that has no writer (a monolithic build, a
                // big-endian tag) are session-scratch by construction. Listing
                // them here would offer a Save that always fails, and — for
                // CloseApp, which re-checks for dirty work after the prompt —
                // a close that never terminates.
                if !document_edits_are_saveable(&self.model.kits[self.model.active], &key, doc) {
                    return None;
                }
                Some(DirtyTagEntry {
                    path: self.tag_path_label(&key),
                    tag_id: key,
                    checked: true,
                })
            })
            .collect()
    }

    pub(in crate::app) fn dirty_chimp_for_close_action(&self, action: &PendingCloseAction) -> Vec<String> {
        if close_action_includes_chimp(action) {
            self.chimp_dirty_packages(self.model.active)
        } else {
            Vec::new()
        }
    }

    pub(in crate::app) fn close_action_tag_keys(&self, action: &PendingCloseAction) -> Vec<String> {
        match action {
            PendingCloseAction::CloseApp | PendingCloseAction::CloseAllTabs => {
                ordered_unique_keys(self.model.kits[self.model.active].open_tabs.iter())
            }
            PendingCloseAction::CloseTab(key) => vec![key.clone()],
            PendingCloseAction::CloseAllButThis(kept_key) => ordered_unique_keys(
                self.model.kits[self.model.active]
                    .open_tabs
                    .iter()
                    .filter(|key| *key != kept_key),
            ),
            // `request_close_action` has already made this kit active, so the
            // active-kit lookups above address the right documents.
            PendingCloseAction::CloseKit(_) => {
                ordered_unique_keys(self.model.kits[self.model.active].open_tabs.iter())
            }
        }
    }

    pub(in crate::app) fn tag_path_label(&self, key: &str) -> String {
        let Some(entry) = self.entry_for_key(key) else {
            return key.to_owned();
        };
        match &entry.location {
            TagEntryLocation::LooseFile(path) => path.display().to_string(),
            TagEntryLocation::Monolithic { .. }
            | TagEntryLocation::Container { .. }
            | TagEntryLocation::NewContainer { .. } => entry.display_path.clone(),
        }
    }

    /// Whether the loaded document for `key` still has unsaved edits. Save
    /// paths that report through `status` (container writes) use this to tell
    /// success from failure.
    pub(in crate::app) fn tag_is_dirty(&self, key: &str) -> bool {
        self.model.kits[self.model.active]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set())
    }

    pub(in crate::app) fn execute_close_action(&mut self, action: PendingCloseAction, ctx: &egui::Context) {
        match action {
            PendingCloseAction::CloseApp => {
                // `request_close_action` is the close coordinator and only
                // calls this once every dirty workspace has been resolved. Do
                // not call it recursively here: a dirty Chimp document used
                // to be counted by this check but omitted from the tag prompt,
                // creating an infinite CloseApp -> request_close_action loop.
                if self.any_kit_dirty() {
                    self.model.status = "Could not close while unsaved workspace data remains".to_owned();
                    return;
                }
                if let Some(session) = self.current_session_state() {
                    if let Err(error) = save_last_session(&session) {
                        self.model.status = error;
                        return;
                    }
                } else {
                    clear_last_session();
                }
                self.documents.save_changes_prompt.allow_app_close_once = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            PendingCloseAction::CloseTab(key) => self.close_tab(&key),
            PendingCloseAction::CloseAllTabs => self.close_all_tabs(),
            PendingCloseAction::CloseAllButThis(key) => self.close_all_tabs_but(&key),
            PendingCloseAction::CloseKit(id) => {
                self.remove_kit(id);
                self.editor.color_popup = None;
                self.editor.function_popup = None;
                self.model.status = "Closed kit".to_owned();
            }
        }
    }

    pub(in crate::app) fn close_all_tabs(&mut self) {
        let id = self.model.kits[self.model.active].id;
        self.views[self.model.kits[self.model.active].id].tag_tree = egui_tiles::Tree::empty(tag_tree_id(id));
        self.model.kits[self.model.active].open_tabs.clear();
        self.kit_and_view(self.model.active).drop_documents_except(None);
        self.model.kits[self.model.active].selected_key = None;
        self.editor.color_popup = None;
        self.editor.function_popup = None;
    }

    pub(in crate::app) fn close_all_tabs_but(&mut self, key: &str) {
        for open in self.views[self.model.kits[self.model.active].id].tabs_from_tree() {
            if open != key {
                self.kit_and_view(self.model.active).close_tag_pane(&open);
            }
        }
        self.kit_and_view(self.model.active).drop_documents_except(Some(key));
        self.model.kits[self.model.active].selected_key = (!is_folder_pane_key(key)).then(|| key.to_owned());
        self.editor.color_popup = None;
        self.editor.function_popup = None;
    }

    pub(in crate::app) fn handle_save_changes_prompt(&mut self, ctx: &egui::Context) {
        let action = render_save_changes_prompt(ctx, &mut self.documents.save_changes_prompt);
        match action {
            SaveChangesPromptAction::None => {}
            SaveChangesPromptAction::Cancel => {
                self.documents.save_changes_prompt.visible = false;
                self.documents.save_changes_prompt.dirty_tags.clear();
                self.documents.save_changes_prompt.error = None;
                self.documents.save_changes_prompt.confirm_discard = false;
            }
            // Arming, not acting: the click that deletes is the next one.
            SaveChangesPromptAction::ConfirmDiscard => {
                self.documents.save_changes_prompt.confirm_discard = true;
            }
            SaveChangesPromptAction::StashForMod => {
                let action = self.documents.save_changes_prompt.pending_action.clone();
                let now = ctx.input(|input| input.time);
                match self.checkpoint_campaign_project(self.model.active, now) {
                    Ok(_) => {
                        // The project holds these bytes now, so they are no
                        // longer unsaved work: leaving them dirty would prompt
                        // again on the next close and, for a CloseApp walking
                        // several kits, would never terminate.
                        for entry in &self.documents.save_changes_prompt.dirty_tags {
                            if let Some(document) =
                                self.model.kits[self.model.active].parsed_tags.get_mut(&entry.tag_id)
                            {
                                document.dirty.clear();
                            }
                        }
                        self.documents.save_changes_prompt.visible = false;
                        self.documents.save_changes_prompt.dirty_tags.clear();
                        self.documents.save_changes_prompt.error = None;
                        self.documents.save_changes_prompt.confirm_discard = false;
                        self.model.status = match self.model.kits[self.model.active]
                            .project.active
                            .as_ref()
                            .and_then(|project| project.project_path.clone())
                        {
                            // Named, because the file the user thinks of as their
                            // project is not the one this wrote.
                            Some(path) => format!(
                                "Stashed for Export Mod. {} is unchanged until you save it",
                                path.display()
                            ),
                            None => "Stashed for Export Mod".to_owned(),
                        };
                        self.request_close_action(action, ctx);
                    }
                    Err(error) => {
                        self.documents.save_changes_prompt.error =
                            Some(format!("Could not stash into the project: {error}"));
                    }
                }
            }
            SaveChangesPromptAction::DontSave => {
                let action = self.documents.save_changes_prompt.pending_action.clone();
                // Discarding is explicit, so drop the dirty flags the prompt
                // listed. Without this, a CloseApp that spans several kits
                // would see the same unsaved work again and re-prompt forever.
                //
                // On a stashing workspace this also deletes the stashed copies —
                // which is why the button is named Discard there and takes a
                // second, confirming click. What it deletes from is the
                // workspace's own recovery file; a `.baboon` the user opened or
                // saved is never written by a close.
                let kit = self.model.active;
                let tag_ids: Vec<String> = self
                    .documents.save_changes_prompt
                    .dirty_tags
                    .iter()
                    .map(|entry| entry.tag_id.clone())
                    .collect();
                for tag_id in &tag_ids {
                    if let Some(doc) = self.model.kits[kit].parsed_tags.get_mut(tag_id) {
                        doc.dirty.clear();
                    }
                    // And forget anything the project stashed for it. Autosave
                    // captures a dirty tag within a second of the edit, so
                    // without this "Don't Save" cleared a flag while the edited
                    // bytes stayed behind and came back on reopen.
                    self.forget_campaign_overlay(kit, tag_id);
                    self.views[self.model.kits[kit].id].edit_buffers.forget_tag(tag_id);
                    // Declining to save a brand-new tag discards the tag, not
                    // just its edits: nothing backs it but the document the
                    // close is about to drop. Its browser entry goes with it.
                    self.forget_new_container_entry(kit, tag_id);
                }
                let now = ctx.input(|input| input.time);
                if let Err(error) = self.checkpoint_campaign_project(kit, now) {
                    self.model.status = format!("Could not update the Campaign Evolved project: {error}");
                }
                self.documents.save_changes_prompt.visible = false;
                self.documents.save_changes_prompt.dirty_tags.clear();
                self.documents.save_changes_prompt.error = None;
                self.documents.save_changes_prompt.confirm_discard = false;
                self.request_close_action(action, ctx);
            }
            SaveChangesPromptAction::Save(tag_ids) => {
                let mut saved = Vec::new();
                let mut errors = Vec::new();
                for tag_id in tag_ids {
                    // Container tags have no loose file to write. A brand-new
                    // one saves via a file dialog (new override container); an
                    // existing one is overwritten inside the game's pak, with
                    // this prompt's Save button standing in for the separate
                    // overwrite confirmation. Both report through `status`
                    // instead of returning a path, so success is read back off
                    // the document's dirty flag.
                    match close_prompt_save_route(
                        self.entry_for_key(&tag_id).map(|entry| &entry.location),
                    ) {
                        ClosePromptSave::NewContainer => {
                            self.save_new_container_tag(&tag_id);
                            if self.tag_is_dirty(&tag_id) {
                                let label = self.tag_path_label(&tag_id);
                                errors.push(format!("{label}: not saved"));
                            } else {
                                saved.push(tag_id.clone());
                            }
                            continue;
                        }
                        ClosePromptSave::ContainerInPlace => {
                            self.overwrite_current_tag_in_place(&tag_id);
                            if self.tag_is_dirty(&tag_id) {
                                // The overwrite failure reason is in `status`.
                                let label = self.tag_path_label(&tag_id);
                                let detail = self.model.status.clone();
                                errors.push(format!("{label}: {detail}"));
                            } else {
                                saved.push(tag_id.clone());
                            }
                            continue;
                        }
                        ClosePromptSave::File => {}
                    }
                    match self.save_tag_by_key(&tag_id) {
                        Ok(path) => saved.push(path.display().to_string()),
                        Err(error) => {
                            let label = self.tag_path_label(&tag_id);
                            errors.push(format!("{label}: {error}"));
                        }
                    }
                }
                if errors.is_empty() {
                    let action = self.documents.save_changes_prompt.pending_action.clone();
                    self.documents.save_changes_prompt.visible = false;
                    self.documents.save_changes_prompt.dirty_tags.clear();
                    self.documents.save_changes_prompt.error = None;
                    self.model.status = if saved.is_empty() {
                        "No files selected to save".to_owned()
                    } else {
                        format!("Saved {} file(s)", saved.len())
                    };
                    self.request_close_action(action, ctx);
                } else {
                    let message = format!("Save failed: {}", errors.join("; "));
                    let pending_action = self.documents.save_changes_prompt.pending_action.clone();
                    self.documents.save_changes_prompt.dirty_tags =
                        self.dirty_tags_for_close_action(&pending_action);
                    // A failed save leaves the prompt up, and an armed discard
                    // has no business surviving into it.
                    self.documents.save_changes_prompt.confirm_discard = false;
                    self.model.status = message.clone();
                    self.documents.save_changes_prompt.error = Some(message);
                }
            }
        }
    }
}

pub(in crate::app) fn close_action_includes_chimp(action: &PendingCloseAction) -> bool {
    matches!(
        action,
        PendingCloseAction::CloseApp | PendingCloseAction::CloseKit(_)
    )
}

pub(in crate::app) enum SaveChangesPromptAction {
    None,
    Save(Vec<String>),
    /// Keep the edits in this workspace's Baboon project, ready for Export
    /// Mod, without writing anything into the game's own files.
    StashForMod,
    /// Arm the discard. Only raised on a stashing workspace, where discarding
    /// deletes stashed bytes rather than just dropping an in-memory edit.
    ConfirmDiscard,
    DontSave,
    Cancel,
}

pub(in crate::app) struct DiscardButton {
    label: &'static str,
    width: f32,
    /// Whether this click only arms the discard. False means it deletes.
    arming: bool,
}

/// What the prompt's discard button says and does.
///
/// On a workspace that stashes, this button deletes bytes that persist across
/// sessions — an edit is stashed within a second of being typed, and exporting a
/// mod does not clear it — so it is named for what it does and takes a second,
/// confirming click. A loose kit has no stash to lose and keeps the one-click
/// "Don't Save" every editor has.
pub(in crate::app) fn discard_button(can_stash: bool, confirmed: bool) -> DiscardButton {
    match (can_stash, confirmed) {
        (false, _) => DiscardButton {
            label: "Don't Save",
            width: 78.0,
            arming: false,
        },
        (true, false) => DiscardButton {
            label: "Discard...",
            width: 96.0,
            arming: true,
        },
        (true, true) => DiscardButton {
            label: "Delete Stashed Edits",
            width: 150.0,
            arming: false,
        },
    }
}

pub(in crate::app) fn render_save_changes_prompt(
    ctx: &egui::Context,
    prompt: &mut SaveChangesPrompt,
) -> SaveChangesPromptAction {
    if !prompt.visible {
        return SaveChangesPromptAction::None;
    }

    let mut action = SaveChangesPromptAction::None;
    egui::Window::new("Baboon - Save Changes?")
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .default_width(window_width(ctx, 520.0))
        .default_height(window_height(ctx, 260.0, true))
        .show(ctx, |ui| {
            ui.label(
                RichText::new("The following files have been modified. Select the files to save.")
                    .color(text_dark()),
            );
            if prompt.can_stash {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Save overwrites the game's own pak files in place. Stash for Mod keeps \
                         the edits in this workspace's project instead, ready for Export Mod. \
                         Discard throws them away, including the copy the project is holding.",
                    )
                    .small()
                    .color(subtle_dark()),
                );
            }
            ui.add_space(8.0);
            if let Some(error) = prompt.error.as_deref() {
                ui.label(RichText::new(error).color(Color32::from_rgb(180, 48, 40)));
                ui.add_space(6.0);
            }
            // Spelling out what the destructive button costs, and where from.
            // Exporting a mod does not clear these edits — the mod is a copy —
            // so this is the prompt an exporter sees on every exit, and it used
            // to delete the stash on one unlabelled click.
            if prompt.confirm_discard {
                ui.label(
                    RichText::new(match (prompt.stashed, prompt.stash_file.as_deref()) {
                        (0, _) => "Discard these edits? They are not stashed anywhere, so they \
                                   cannot be recovered."
                            .to_owned(),
                        (count, Some(file)) => format!(
                            "Discard deletes the stashed copy of {count} tag(s) from {}. Other \
                             stashed tags in this workspace, and mods you have already exported, \
                             are not affected.",
                            file.display()
                        ),
                        (count, None) => format!(
                            "Discard deletes the stashed copy of {count} tag(s). Mods you have \
                             already exported are not affected."
                        ),
                    })
                    .color(Color32::from_rgb(210, 120, 90)),
                );
                ui.add_space(6.0);
            }
            ScrollArea::both().max_height(150.0).show(ui, |ui| {
                for dirty in &mut prompt.dirty_tags {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut dirty.checked, "");
                        ui.label(RichText::new(&dirty.path).color(text_dark()));
                    });
                }
            });
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("Cancel").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    action = SaveChangesPromptAction::Cancel;
                }
                let DiscardButton {
                    label,
                    width,
                    arming,
                } = discard_button(prompt.can_stash, prompt.confirm_discard);
                if ui
                    .add(egui::Button::new(label).min_size(Vec2::new(width, 24.0)))
                    .clicked()
                {
                    action = if arming {
                        SaveChangesPromptAction::ConfirmDiscard
                    } else {
                        SaveChangesPromptAction::DontSave
                    };
                }
                if prompt.can_stash
                    && ui
                        .add(egui::Button::new("Stash for Mod").min_size(Vec2::new(110.0, 24.0)))
                        .on_hover_text(
                            "Keep these edits in this workspace's Baboon project, ready for \
                             Export Mod. The game's own files are left untouched.",
                        )
                        .clicked()
                {
                    action = SaveChangesPromptAction::StashForMod;
                }
                if ui
                    .add(egui::Button::new("Save").min_size(Vec2::new(78.0, 24.0)))
                    .clicked()
                {
                    let tag_ids = prompt
                        .dirty_tags
                        .iter()
                        .filter(|dirty| dirty.checked)
                        .map(|dirty| dirty.tag_id.clone())
                        .collect();
                    action = SaveChangesPromptAction::Save(tag_ids);
                }
            });
        });
    action
}

/// How the close prompt's Save writes one tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ClosePromptSave {
    /// A brand-new container tag: a new override container, via a dialog.
    NewContainer,
    /// A mounted container tag: overwritten inside its own pak.
    ContainerInPlace,
    /// Everything else is a file on disk, or has none and is refused there.
    File,
}

/// A container tag has no file to write, so sending one down the file path
/// would fail, or worse, write the payload somewhere it does not belong.
pub(in crate::app) fn close_prompt_save_route(location: Option<&TagEntryLocation>) -> ClosePromptSave {
    match location {
        Some(TagEntryLocation::NewContainer { .. }) => ClosePromptSave::NewContainer,
        Some(TagEntryLocation::Container { .. }) => ClosePromptSave::ContainerInPlace,
        Some(TagEntryLocation::LooseFile(_) | TagEntryLocation::Monolithic { .. }) | None => {
            ClosePromptSave::File
        }
    }
}

#[cfg(test)]
mod save_changes_prompt_tests;

#[cfg(test)]
mod save_close_session_tests;

#[cfg(test)]
mod close_tests;
