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
        let label = self.model.tag_path_label(key);
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
            self.model.entry_for_key_in(kit, key).map(|entry| &entry.location),
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

    pub(in crate::app) fn close_tab(&mut self, key: &str) {
        // `close_tag_pane` re-derives the open set and moves the selection off
        // a removed tag, so there is nothing to fix up afterwards.
        self.kit_and_view(self.model.active).close_tag_pane(key);
        self.unload_tag(key);
        let kit = self.model.kits[self.model.active].id;
        self.close_tag_popups(kit, |tag| tag == key);
    }

    /// Close the color and function popups editing those of `kit`'s tags
    /// that `closing` names. A popup for any other tag stays, with its
    /// unconfirmed edits: closing one tab used to throw away the popup open
    /// on another. A function editor's own color picker goes with it.
    pub(in crate::app) fn close_tag_popups(&mut self, kit: KitId, closing: impl Fn(&str) -> bool) {
        let function_closes = self.dialogs.get::<FunctionPopupWindow>().is_some_and(|window| {
            window.kit == kit && window.popup.as_ref().is_none_or(|popup| closing(popup.tag_key()))
        });
        if function_closes {
            self.dialogs.close::<FunctionPopupWindow>();
        }
        let color_closes = self.dialogs.get::<ColorPopupWindow>().is_some_and(|window| {
            window.popup.as_ref().is_none_or(|popup| {
                if popup.edits_function_draft() {
                    function_closes
                } else {
                    window.kit == kit && closing(popup.tag_key())
                }
            })
        });
        if color_closes {
            self.dialogs.close::<ColorPopupWindow>();
        }
    }

    pub(in crate::app) fn request_close_action(&mut self, action: PendingCloseAction, ctx: &egui::Context) {
        // Chimp's recovery checkpoints wait for edits to pause; one still
        // waiting when the app or a workspace closes would be lost.
        self.flush_all_chimp_checkpoints();
        if self.dialogs.get::<SaveChangesPrompt>().is_some()
            || self.dialogs.get::<ChimpDiscardPrompt>().is_some()
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
                if let Some(index) = self.model.kit_index(*id) {
                    self.model.active = index;
                }
            }
            PendingCloseAction::CloseApp => {
                if let Some(index) = self.model.first_dirty_kit() {
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
        let can_stash = self.model.current_source_is_campaign_project_capable(self.model.active);
        let dirty_tags = self.model.dirty_tags_for_close_action(&action);
        if !dirty_tags.is_empty() {
            // What discarding would cost, resolved here rather than described in the
            // abstract: these edits were stashed into the workspace's project within
            // a second of being typed, so declining to save deletes them from a file
            // that outlives the session.
            let stashed = dirty_tags
                .iter()
                .filter(|entry| self.model.tag_has_stashed_overlay(self.model.active, &entry.tag_id))
                .count();
            let stash_file = self.model.kits[self.model.active]
                .project.active
                .as_ref()
                .map(|project| project.recovery_path.clone());
            self.dialogs.open(SaveChangesPrompt {
                can_stash,
                dirty_tags,
                pending_action: action,
                error: None,
                stash_file,
                stashed,
                confirm_discard: false,
            });
            return;
        }

        let chimp_packages = self.model.dirty_chimp_for_close_action(&action);
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
        if self.documents.allow_app_close_once {
            self.documents.allow_app_close_once = false;
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        // Quitting would kill the worker partway through rewriting references,
        // leaving some tags pointing at a path that no longer exists.
        if self.tag_ops.folder_refactor.is_some() {
            self.model.status = "Wait for the folder move/rename to finish before closing".to_owned();
            return;
        }
        if self.dialogs.get::<SaveChangesPrompt>().is_some()
            || self.dialogs.get::<ChimpDiscardPrompt>().is_some()
            || self.has_chimp_save_dialog()
        {
            return;
        }
        self.defer_file_action(DeferredFileAction::Close(PendingCloseAction::CloseApp), ctx);
    }

    pub(in crate::app) fn execute_close_action(&mut self, action: PendingCloseAction, ctx: &egui::Context) {
        match action {
            PendingCloseAction::CloseApp => {
                // `request_close_action` is the close coordinator and only
                // calls this once every dirty workspace has been resolved. Do
                // not call it recursively here: a dirty Chimp document used
                // to be counted by this check but omitted from the tag prompt,
                // creating an infinite CloseApp -> request_close_action loop.
                if self.model.any_kit_dirty() {
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
                self.documents.allow_app_close_once = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            PendingCloseAction::CloseTab(key) => self.close_tab(&key),
            PendingCloseAction::CloseAllTabs => self.close_all_tabs(),
            PendingCloseAction::CloseAllButThis(key) => self.close_all_tabs_but(&key),
            PendingCloseAction::CloseKit(id) => {
                self.remove_kit(id);
                self.close_tag_popups(id, |_| true);
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
        self.close_tag_popups(id, |_| true);
    }

    pub(in crate::app) fn close_all_tabs_but(&mut self, key: &str) {
        for open in self.views[self.model.kits[self.model.active].id].tabs_from_tree() {
            if open != key {
                self.kit_and_view(self.model.active).close_tag_pane(&open);
            }
        }
        self.kit_and_view(self.model.active).drop_documents_except(Some(key));
        self.model.kits[self.model.active].selected_key = (!is_folder_pane_key(key)).then(|| key.to_owned());
        let kit = self.model.kits[self.model.active].id;
        self.close_tag_popups(kit, |tag| tag != key);
    }

    /// Carry out the save-changes prompt's answer.
    ///
    /// The prompt is taken out of the host while it is, and put back when it
    /// stays up: an armed discard, or a save or stash that failed.
    pub(in crate::app) fn apply_save_changes_prompt_action(&mut self, action: SaveChangesPromptAction, ctx: &egui::Context) {
        let Some(mut prompt) = self.dialogs.close::<SaveChangesPrompt>() else {
            return;
        };
        match action {
            SaveChangesPromptAction::None => self.dialogs.open(prompt),
            SaveChangesPromptAction::Cancel => {}
            // Arming, not acting: the click that deletes is the next one.
            SaveChangesPromptAction::ConfirmDiscard => {
                prompt.confirm_discard = true;
                self.dialogs.open(prompt);
            }
            SaveChangesPromptAction::StashForMod => {
                let action = prompt.pending_action.clone();
                let now = ctx.input(|input| input.time);
                match self.checkpoint_campaign_project(self.model.active, now) {
                    Ok(_) => {
                        // The project holds these bytes now, so they are no
                        // longer unsaved work: leaving them dirty would prompt
                        // again on the next close and, for a CloseApp walking
                        // several kits, would never terminate.
                        for entry in &prompt.dirty_tags {
                            if let Some(document) =
                                self.model.kits[self.model.active].parsed_tags.get_mut(&entry.tag_id)
                            {
                                document.dirty.clear();
                            }
                        }
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
                        prompt.error = Some(format!("Could not stash into the project: {error}"));
                        self.dialogs.open(prompt);
                    }
                }
            }
            SaveChangesPromptAction::DontSave => {
                let action = prompt.pending_action.clone();
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
                let tag_ids: Vec<String> = prompt
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
                        self.model.entry_for_key(&tag_id).map(|entry| &entry.location),
                    ) {
                        ClosePromptSave::NewContainer => {
                            self.save_new_container_tag(&tag_id);
                            if self.model.tag_is_dirty(&tag_id) {
                                let label = self.model.tag_path_label(&tag_id);
                                errors.push(format!("{label}: not saved"));
                            } else {
                                saved.push(tag_id.clone());
                            }
                            continue;
                        }
                        ClosePromptSave::ContainerInPlace => {
                            self.overwrite_current_tag_in_place(&tag_id);
                            if self.model.tag_is_dirty(&tag_id) {
                                // The overwrite failure reason is in `status`.
                                let label = self.model.tag_path_label(&tag_id);
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
                            let label = self.model.tag_path_label(&tag_id);
                            errors.push(format!("{label}: {error}"));
                        }
                    }
                }
                if errors.is_empty() {
                    let action = prompt.pending_action.clone();
                    self.model.status = if saved.is_empty() {
                        "No files selected to save".to_owned()
                    } else {
                        format!("Saved {} file(s)", saved.len())
                    };
                    self.request_close_action(action, ctx);
                } else {
                    let message = format!("Save failed: {}", errors.join("; "));
                    prompt.dirty_tags = self
                        .model
                        .dirty_tags_for_close_action(&prompt.pending_action);
                    // A failed save leaves the prompt up, and an armed discard
                    // has no business surviving into it.
                    prompt.confirm_discard = false;
                    self.model.status = message.clone();
                    prompt.error = Some(message);
                    self.dialogs.open(prompt);
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

/// The save-changes prompt. It stays up until its answer is carried out once
/// drawing is over; that takes it back from the host, and puts it back if
/// the answer leaves it up.
impl Dialog for SaveChangesPrompt {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let action = render_save_changes_prompt(cx.egui, self);
        if !matches!(action, SaveChangesPromptAction::None) {
            cx.send(DocumentsCommand::SaveChangesPrompt(action));
        }
        true
    }
}

pub(in crate::app) fn render_save_changes_prompt(
    ctx: &egui::Context,
    prompt: &mut SaveChangesPrompt,
) -> SaveChangesPromptAction {
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

impl Model {
    /// Whether `key` has anything to discard — unsaved edits, or bytes stashed
    /// in this kit's project from an earlier session.
    pub(in crate::app) fn tag_has_discardable_changes(&self, kit: usize, key: &str) -> bool {
        self.kits[kit]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set())
            || self.tag_has_stashed_overlay(kit, key)
    }

    pub(in crate::app) fn dirty_tags_for_close_action(&self, action: &PendingCloseAction) -> Vec<DirtyTagEntry> {
        self.close_action_tag_keys(action)
            .into_iter()
            .filter_map(|key| {
                let doc = self.kits[self.active].parsed_tags.get(&key)?;
                if !doc.dirty.is_set() {
                    return None;
                }
                // Edits to a tag that has no writer (a monolithic build, a
                // big-endian tag) are session-scratch by construction. Listing
                // them here would offer a Save that always fails, and — for
                // CloseApp, which re-checks for dirty work after the prompt —
                // a close that never terminates.
                if !document_edits_are_saveable(&self.kits[self.active], &key, doc) {
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
            self.chimp_dirty_packages(self.active)
        } else {
            Vec::new()
        }
    }

    pub(in crate::app) fn close_action_tag_keys(&self, action: &PendingCloseAction) -> Vec<String> {
        match action {
            PendingCloseAction::CloseApp | PendingCloseAction::CloseAllTabs => {
                ordered_unique_keys(self.kits[self.active].open_tabs.iter())
            }
            PendingCloseAction::CloseTab(key) => vec![key.clone()],
            PendingCloseAction::CloseAllButThis(kept_key) => ordered_unique_keys(
                self.kits[self.active]
                    .open_tabs
                    .iter()
                    .filter(|key| *key != kept_key),
            ),
            // `request_close_action` has already made this kit active, so the
            // active-kit lookups above address the right documents.
            PendingCloseAction::CloseKit(_) => {
                ordered_unique_keys(self.kits[self.active].open_tabs.iter())
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
        self.kits[self.active]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::browser::{BrowserAction, BrowserMode, BrowserSort};
    use crate::app::loose_fixture::*;
    use crate::app::shell::FolderRefactorUiState;
    use crate::app::shell::session::LastSessionSourceKind;
    use std::path::PathBuf;

    // The close prompt's discard button, which used to be one click from deleting
    // a workspace's stashed edits.

    /// No single click may delete a stash. The button that did was labelled "Don't
    /// Save" on a quit dialog, and because Export Mod leaves its tags dirty, that is
    /// the dialog an exporter sees on every exit -- so "no thanks, I already
    /// exported" deleted the very edits the mod was made from.
    #[test]
    fn no_single_click_deletes_a_stash() {
        let fresh = discard_button(true, false);
        assert!(fresh.arming, "the first click on a stashing workspace arms");
        assert_ne!(
            fresh.label, "Don't Save",
            "it is named for what it does, not for what it declines"
        );
        let armed = discard_button(true, true);
        assert!(!armed.arming, "the second click acts");
        assert!(
            armed.label.contains("Stashed"),
            "and says what it is deleting: {}",
            armed.label
        );
    }

    /// A workspace with nowhere to stash to loses nothing beyond the open document,
    /// so it keeps the one-click discard every editor has.
    #[test]
    fn a_workspace_without_a_stash_keeps_one_click_discard() {
        let button = discard_button(false, false);
        assert_eq!(button.label, "Don't Save");
        assert!(!button.arming);
        // Even a stale confirmation flag cannot turn this into two clicks.
        assert_eq!(discard_button(false, true).label, "Don't Save");
    }

    // Characterization of the save, close, discard, undo and session-restore
    // flows on synthetic loose tags.
    //
    // These pin what the app does today, so moving the code that does it can be
    // shown to have changed nothing. Each flow runs through the entry point the
    // UI calls -- the close prompt is clicked, not short-circuited -- and each
    // assertion reads back state: files on disk, documents, tabs, the prompt.
    //
    // A Campaign Evolved source would route the prompt's Save into the pak
    // (`overwrite_current_tag_in_place` / `save_new_container_tag`), which needs a
    // mounted install; those branches are not reached here. The stash half of the
    // prompt is covered by the tests in `mods/project.rs`.

    const MODEL: &str = "objects/props/crate.model";
    const OTHER: &str = "objects/props/barrel.model";
    const DISTANCE: &str = "disappear distance";

    /// An H3 kit with two models, one referencing a render model.
    fn kit(name: &str) -> LooseKit {
        let kit = LooseKit::new(name, "halo3_mcc");
        let mode = group_tag("halo3_mcc", "render_model");
        kit.write_mcc("objects/props/crate", "render_model", |_| {});
        kit.write_mcc("objects/props/crate", "model", |tag| {
            set_reference(tag, "render model", mode, "objects\\props\\crate");
        });
        kit.write_mcc("objects/props/barrel", "model", |_| {});
        kit
    }

    fn distance_on_disk(kit: &LooseKit, rel: &str) -> f32 {
        let tag = TagFile::read(kit.root.join(rel)).expect("the saved tag reads");
        real_of(&tag, DISTANCE).expect("a real")
    }

    fn distance_in_document(app: &Baboon, key: &str) -> f32 {
        real_of(&app.model.kits[app.model.active].parsed_tags[key].tag, DISTANCE).expect("a real")
    }

    fn is_dirty(app: &Baboon, key: &str) -> bool {
        app.model.kits[app.model.active]
            .parsed_tags
            .get(key)
            .is_some_and(|document| document.dirty.is_set())
    }

    fn tab_open(app: &Baboon, key: &str) -> bool {
        app.model.kits[app.model.active].open_tabs.iter().any(|open| open == key)
    }

    /// A kit installed with `MODEL` open and edited, and `OTHER` open and clean.
    fn edited(name: &str) -> (LooseKit, Baboon, String, String) {
        let kit = kit(name);
        let mut app = app();
        kit.install(&mut app);
        let other = kit.open(&mut app, OTHER);
        let key = kit.open(&mut app, MODEL);
        edit_field(&mut app, &key, DISTANCE, "12.5");
        assert!(is_dirty(&app, &key));
        (kit, app, key, other)
    }

    #[test]
    fn closing_a_clean_tab_closes_it_without_a_prompt() {
        let kit = kit("close-clean");
        let mut app = app();
        kit.install(&mut app);
        let key = kit.open(&mut app, MODEL);

        app.request_close_action(PendingCloseAction::CloseTab(key.clone()), &ctx());

        assert!(app.dialogs.get::<SaveChangesPrompt>().is_none());
        assert!(!tab_open(&app, &key));
        assert!(!app.model.kits[0].parsed_tags.contains_key(&key), "the document is dropped");
    }

    #[test]
    fn closing_a_dirty_tab_raises_the_prompt_and_leaves_the_tab_open() {
        let (_kit, mut app, key, other) = edited("close-dirty");

        app.request_close_action(PendingCloseAction::CloseTab(key.clone()), &ctx());

        let prompt = app
            .dialogs
            .get::<SaveChangesPrompt>()
            .expect("the prompt is up");
        assert!(!prompt.can_stash, "a loose kit has nowhere to stash");
        assert_eq!(
            prompt
                .dirty_tags
                .iter()
                .map(|entry| entry.tag_id.as_str())
                .collect::<Vec<_>>(),
            vec![key.as_str()]
        );
        assert!(prompt.dirty_tags[0].checked, "listed tags start checked");
        assert!(
            prompt.dirty_tags[0].path.ends_with("crate.model"),
            "labelled by its file path: {}",
            prompt.dirty_tags[0].path
        );
        assert!(matches!(&prompt.pending_action, PendingCloseAction::CloseTab(k) if *k == key));
        assert_eq!(prompt.stashed, 0);
        assert!(tab_open(&app, &key) && tab_open(&app, &other));
        assert!(is_dirty(&app, &key));

        // A second close while the prompt is up is ignored rather than replacing it.
        app.request_close_action(PendingCloseAction::CloseAllTabs, &ctx());
        assert!(matches!(
            &app.dialogs
                .get::<SaveChangesPrompt>()
                .unwrap()
                .pending_action,
            PendingCloseAction::CloseTab(_)
        ));
    }

    #[test]
    fn the_prompt_s_save_writes_the_tag_then_closes_it() {
        let (kit, mut app, key, other) = edited("close-save");
        app.request_close_action(PendingCloseAction::CloseTab(key.clone()), &ctx());
        let mut driver = PromptDriver::new();

        driver.click(&mut app, "Save");

        assert_eq!(distance_on_disk(&kit, MODEL), 12.5);
        assert!(app.dialogs.get::<SaveChangesPrompt>().is_none());
        assert!(
            app.dialogs
                .get::<SaveChangesPrompt>()
                .is_none_or(|prompt| prompt.dirty_tags.is_empty())
        );
        assert_eq!(app.model.status, "Saved 1 file(s)");
        assert!(!tab_open(&app, &key), "the close went ahead");
        assert!(tab_open(&app, &other));
        // The reference the tag already held is written back untouched.
        let saved = TagFile::read(kit.root.join(MODEL)).unwrap();
        assert_eq!(
            reference_of(&saved, "render model").as_deref(),
            Some("objects\\props\\crate")
        );
    }

    #[test]
    fn the_prompt_s_dont_save_closes_without_writing() {
        let (kit, mut app, key, _other) = edited("close-dont-save");
        app.request_close_action(PendingCloseAction::CloseTab(key.clone()), &ctx());
        let before = fs::read(kit.root.join(MODEL)).unwrap();
        let mut driver = PromptDriver::new();

        driver.click(&mut app, "Don't Save");

        assert_eq!(fs::read(kit.root.join(MODEL)).unwrap(), before, "nothing written");
        assert!(app.dialogs.get::<SaveChangesPrompt>().is_none());
        assert!(!tab_open(&app, &key));
        assert!(!app.model.kits[0].parsed_tags.contains_key(&key));
    }

    #[test]
    fn the_prompt_s_cancel_keeps_the_tab_and_its_edit() {
        let (kit, mut app, key, _other) = edited("close-cancel");
        app.request_close_action(PendingCloseAction::CloseTab(key.clone()), &ctx());
        let mut driver = PromptDriver::new();

        driver.click(&mut app, "Cancel");

        assert!(app.dialogs.get::<SaveChangesPrompt>().is_none());
        assert!(
            app.dialogs
                .get::<SaveChangesPrompt>()
                .is_none_or(|prompt| prompt.dirty_tags.is_empty())
        );
        assert!(tab_open(&app, &key));
        assert!(is_dirty(&app, &key));
        assert_eq!(distance_in_document(&app, &key), 12.5);
        assert_eq!(distance_on_disk(&kit, MODEL), 0.0);
    }

    /// An unchecked tag is not written. The Save itself "succeeds" ("No files
    /// selected to save"), and the close is then retried -- which finds the tag
    /// still dirty and raises the prompt again. So unchecking is not a way to
    /// close without saving; only Don't Save is.
    // QUIRK: most editors treat an unchecked row as "close without saving it".
    #[test]
    fn an_unchecked_tag_is_not_saved_and_the_prompt_comes_back() {
        let (kit, mut app, key, _other) = edited("close-unchecked");
        app.request_close_action(PendingCloseAction::CloseTab(key.clone()), &ctx());
        app.dialogs
            .get_mut::<SaveChangesPrompt>()
            .unwrap()
            .dirty_tags[0]
            .checked = false;
        let mut driver = PromptDriver::new();

        driver.click(&mut app, "Save");

        assert_eq!(distance_on_disk(&kit, MODEL), 0.0);
        assert_eq!(app.model.status, "No files selected to save");
        assert!(
            app.dialogs.get::<SaveChangesPrompt>().is_some(),
            "prompted again"
        );
        assert_eq!(
            app.dialogs
                .get::<SaveChangesPrompt>()
                .unwrap()
                .dirty_tags
                .len(),
            1
        );
        assert!(
            app.dialogs.get::<SaveChangesPrompt>().unwrap().dirty_tags[0].checked,
            "re-listed checked"
        );
        assert!(tab_open(&app, &key));
        assert!(is_dirty(&app, &key));
    }

    /// A save that fails keeps the prompt up with the reason, lists the tags again
    /// and does not close anything.
    #[test]
    fn a_failed_save_keeps_the_prompt_up_with_the_reason() {
        let (kit, mut app, key, _other) = edited("close-save-fails");
        app.request_close_action(PendingCloseAction::CloseTab(key.clone()), &ctx());
        // The tag's file is replaced by a directory, so the atomic write cannot
        // land.
        fs::remove_file(kit.root.join(MODEL)).unwrap();
        fs::create_dir_all(kit.root.join(MODEL)).unwrap();
        let mut driver = PromptDriver::new();

        driver.click(&mut app, "Save");

        let prompt = app
            .dialogs
            .get::<SaveChangesPrompt>()
            .expect("the prompt is up");
        let error = prompt.error.as_deref().expect("an error is shown");
        assert!(error.starts_with("Save failed: "), "{error}");
        assert!(error.contains("crate.model"), "{error}");
        assert_eq!(app.model.status, error);
        assert_eq!(prompt.dirty_tags.len(), 1, "the tag is listed again");
        assert!(tab_open(&app, &key));
        assert!(is_dirty(&app, &key));
    }

    #[test]
    fn close_all_prompts_for_the_dirty_tags_only_then_closes_everything() {
        let (_kit, mut app, key, other) = edited("close-all");

        app.request_close_action(PendingCloseAction::CloseAllTabs, &ctx());
        assert_eq!(
            app.dialogs
                .get::<SaveChangesPrompt>()
                .unwrap()
                .dirty_tags
                .iter()
                .map(|entry| entry.tag_id.clone())
                .collect::<Vec<_>>(),
            vec![key.clone()],
            "the clean tab is not listed"
        );
        PromptDriver::new().click(&mut app, "Don't Save");

        assert!(app.model.kits[0].open_tabs.is_empty());
        assert!(app.model.kits[0].parsed_tags.is_empty());
        assert_eq!(app.model.kits[0].selected_key, None);
        assert!(!tab_open(&app, &other));
    }

    #[test]
    fn close_all_but_this_keeps_the_named_tab_and_its_unsaved_edit() {
        let (_kit, mut app, key, other) = edited("close-all-but");

        // The dirty tag is the one kept, so nothing needs saving.
        app.request_close_action(PendingCloseAction::CloseAllButThis(key.clone()), &ctx());

        assert!(app.dialogs.get::<SaveChangesPrompt>().is_none());
        assert_eq!(app.model.kits[0].open_tabs, vec![key.clone()]);
        assert!(!app.model.kits[0].parsed_tags.contains_key(&other));
        assert!(is_dirty(&app, &key));
        assert_eq!(app.model.kits[0].selected_key.as_deref(), Some(key.as_str()));

        // Keeping the clean one instead lists the dirty one.
        let (_kit, mut app, key, other) = edited("close-all-but-other");
        app.request_close_action(PendingCloseAction::CloseAllButThis(other.clone()), &ctx());
        assert!(app.dialogs.get::<SaveChangesPrompt>().is_some());
        assert_eq!(
            app.dialogs.get::<SaveChangesPrompt>().unwrap().dirty_tags[0].tag_id,
            key
        );
    }

    fn close_requested_input(time: f64) -> egui::RawInput {
        let mut input = screen(Vec::new(), time);
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        input
    }

    /// Closing the window while it is minimized (from the taskbar, say) runs no
    /// UI pass, only `App::logic`. The close must still be vetoed there, and the
    /// next logic tick must still raise the prompt for the dirty tag.
    #[test]
    fn a_close_while_the_window_is_hidden_is_still_vetoed_and_prompted_for() {
        let (_kit, mut app, key, _other) = edited("close-hidden");
        let ctx = egui::Context::default();
        let _ = crate::app::run_ui_test(&ctx, screen(Vec::new(), 1.0), |_| {});
        let hidden = |mut input: egui::RawInput| {
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .minimized = Some(true);
            input
        };

        let output = ctx.run_logic(&hidden(close_requested_input(2.0)), |ctx| app.run_logic(ctx));
        let commands = output
            .viewport_commands
            .get(&egui::ViewportId::ROOT)
            .cloned()
            .unwrap_or_default();
        assert!(commands.contains(&egui::ViewportCommand::CancelClose), "{commands:?}");
        assert!(matches!(
            app.editor.deferred_file_action,
            Some(DeferredFileAction::Close(PendingCloseAction::CloseApp))
        ));
        assert!(
            app.dialogs.get::<SaveChangesPrompt>().is_none(),
            "the close waits a frame"
        );

        let _ = ctx.run_logic(&hidden(screen(Vec::new(), 2.1)), |ctx| app.run_logic(ctx));
        assert!(app.editor.deferred_file_action.is_none());
        assert!(
            app.dialogs.get::<SaveChangesPrompt>().is_some(),
            "dirty work is prompted for"
        );
        assert_eq!(
            app.dialogs.get::<SaveChangesPrompt>().unwrap().dirty_tags[0].tag_id,
            key
        );
    }

    /// The native close is vetoed, prompted for, and only then re-issued; the
    /// re-issued close is let through once.
    #[test]
    fn the_app_close_is_two_step_and_writes_the_session() {
        let _session = session_file_lock();
        let (kit, mut app, key, _other) = edited("close-app");
        let ctx = egui::Context::default();

        let output = crate::app::run_ui_test(&ctx, close_requested_input(1.0), |ui| {
            app.handle_app_close_request(ui.ctx())
        });
        assert!(root_commands(&output).contains(&egui::ViewportCommand::CancelClose));
        assert!(matches!(
            app.editor.deferred_file_action,
            Some(DeferredFileAction::Close(PendingCloseAction::CloseApp))
        ));
        // What the next frame's `run_deferred_file_action` does with it.
        let Some(DeferredFileAction::Close(action)) = app.editor.deferred_file_action.take() else {
            unreachable!()
        };
        let _ = crate::app::run_ui_test(&ctx, screen(Vec::new(), 1.1), |ui| {
            app.request_close_action(action.clone(), ui.ctx())
        });
        assert!(
            app.dialogs.get::<SaveChangesPrompt>().is_some(),
            "dirty work is prompted for"
        );
        assert!(matches!(
            app.dialogs
                .get::<SaveChangesPrompt>()
                .unwrap()
                .pending_action,
            PendingCloseAction::CloseApp
        ));

        // Don't Save: the edit is dropped and the close re-issued.
        let mut driver = PromptDriver::on(ctx.clone(), 2.0);
        driver.click(&mut app, "Don't Save");
        assert!(driver.commands.contains(&egui::ViewportCommand::Close));
        assert!(app.documents.allow_app_close_once);
        assert!(!is_dirty(&app, &key));
        assert_eq!(distance_on_disk(&kit, MODEL), 0.0);
        // The quit recorded the session, naming the open tags.
        let session = load_last_session().expect("the session was written");
        assert_eq!(session.kits.len(), 1);
        assert!(session.kits[0].tags.iter().any(|tag| tag.key == key));

        // The close it re-issued comes back as a request and passes, once.
        let output = crate::app::run_ui_test(&ctx, close_requested_input(driver.time + 1.0), |ui| {
            app.handle_app_close_request(ui.ctx())
        });
        assert!(!root_commands(&output).contains(&egui::ViewportCommand::CancelClose));
        assert!(!app.documents.allow_app_close_once);
        assert!(app.editor.deferred_file_action.is_none());
    }

    #[test]
    fn the_app_does_not_close_while_a_folder_refactor_runs() {
        let mut app = app();
        app.tag_ops.folder_refactor = Some(FolderRefactorUiState {
            label: "Renaming".to_owned(),
            phase: "Moving files".to_owned(),
            progress: None,
        });
        let ctx = egui::Context::default();

        let output = crate::app::run_ui_test(&ctx, close_requested_input(1.0), |ui| {
            app.handle_app_close_request(ui.ctx())
        });

        assert!(root_commands(&output).contains(&egui::ViewportCommand::CancelClose));
        assert!(app.editor.deferred_file_action.is_none());
        assert_eq!(
            app.model.status,
            "Wait for the folder move/rename to finish before closing"
        );
    }

    /// With nothing dirty the app closes straight away, without a prompt.
    #[test]
    fn a_clean_app_close_closes_at_once() {
        let _session = session_file_lock();
        let kit = kit("close-app-clean");
        let mut app = app();
        kit.install(&mut app);
        kit.open(&mut app, MODEL);
        let ctx = egui::Context::default();

        let output = crate::app::run_ui_test(&ctx, screen(Vec::new(), 1.0), |ui| {
            app.request_close_action(PendingCloseAction::CloseApp, ui.ctx())
        });

        assert!(app.dialogs.get::<SaveChangesPrompt>().is_none());
        assert!(root_commands(&output).contains(&egui::ViewportCommand::Close));
        assert!(app.documents.allow_app_close_once);
    }

    #[test]
    fn save_current_tag_writes_the_selected_tag() {
        let (kit, mut app, key, _other) = edited("save-current");

        app.save_current_tag(&ctx());

        assert_eq!(distance_on_disk(&kit, MODEL), 12.5);
        assert!(!is_dirty(&app, &key));
        assert_eq!(
            app.model.status,
            format!("Saved {}", kit.root.join(MODEL).display())
        );
        // The save records what the tag now references.
        let source = app.model.kits[0].source.as_ref().unwrap();
        assert!(source.reverse_dependencies.is_none(), "no index was built to update");
    }

    #[test]
    fn save_current_tag_needs_a_selection() {
        let mut app = app();
        app.save_current_tag(&ctx());
        assert_eq!(app.model.status, "No tag selected");
    }

    #[test]
    fn save_tag_by_key_refuses_what_it_cannot_write() {
        let kit = kit("save-by-key");
        let mut app = app();
        kit.install(&mut app);

        assert_eq!(
            app.save_tag_by_key("file:/nowhere.model"),
            Err("Selected tag is no longer in the source".to_owned())
        );
        let key = kit.key(MODEL);
        assert_eq!(
            app.save_tag_by_key(&key),
            Err("Load the selected tag before saving".to_owned())
        );

        // A big-endian document has no writer.
        kit.open(&mut app, MODEL);
        app.model.kits[0].parsed_tags.get_mut(&key).unwrap().tag.endian = blam_tags::Endian::Be;
        let refused = app.save_tag_by_key(&key).unwrap_err();
        let entry = app.model.entry_for_key(&key).cloned().unwrap();
        assert_eq!(
            Some(refused),
            unsaveable_reason(&entry, &app.model.kits[0].parsed_tags[&key].tag)
                .map(|reason| reason.to_string())
        );

        // A clean, loaded tag saves even with nothing to save.
        app.model.kits[0].parsed_tags.get_mut(&key).unwrap().tag.endian = blam_tags::Endian::Le;
        assert_eq!(app.save_tag_by_key(&key), Ok(kit.root.join(MODEL)));
    }

    /// Everything before the file dialog: Save As refuses without a selection, an
    /// entry, or a loaded document.
    #[test]
    fn save_as_refuses_before_its_dialog() {
        let kit = kit("save-as-guards");
        let mut app = app();
        app.save_current_tag_as();
        assert_eq!(app.model.status, "No tag selected");

        kit.install(&mut app);
        app.model.kits[0].selected_key = Some("file:/nowhere.model".to_owned());
        app.save_current_tag_as();
        assert_eq!(app.model.status, "Selected tag is no longer in the source");

        app.model.kits[0].selected_key = Some(kit.key(MODEL));
        app.save_current_tag_as();
        assert_eq!(app.model.status, "Load the selected tag before saving");
    }

    /// What Save As does after its dialog: a copy written inside the loaded tags
    /// folder is registered in the browser, keyed the way a scan keys it.
    #[test]
    fn a_save_as_copy_inside_the_tags_folder_joins_the_browser() {
        let kit = kit("save-as-register");
        let mut app = app();
        kit.install(&mut app);
        let generation = app.model.kits[0].generation;
        let copy = kit.write_mcc("objects/copies/crate_copy", "model", |_| {});

        let key = kit.key("objects/copies/crate_copy.model");
        let registered = app.register_saved_copy_if_in_loaded_folder(&copy);
        assert_eq!(registered.map(|entry| entry.map(|entry| entry.key)), Ok(Some(key.clone())));

        assert!(app.model.entry_for_key(&key).is_some());
        assert_ne!(app.model.kits[0].generation, generation);

        // Outside the tags folder there is nothing to register.
        let outside = kit.base.join("elsewhere.model");
        fs::copy(&copy, &outside).unwrap();
        assert!(matches!(app.register_saved_copy_if_in_loaded_folder(&outside), Ok(None)));
    }

    /// After Save As the editor is on the copy: the document, its tab and its
    /// undo history move to the new file, clean, and the original is left as
    /// it was on disk.
    #[test]
    fn save_as_switches_the_editor_to_the_copy() {
        let (kit, mut app, key, _other) = edited("save-as-switch");
        const COPY: &str = "objects/copies/crate_copy.model";
        std::fs::create_dir_all(kit.root.join("objects/copies")).unwrap();

        app.save_tag_as_to(&key, &kit.root.join(COPY));

        let copy = kit.key(COPY);
        assert!(app.model.status.starts_with("Saved as"), "{}", app.model.status);
        assert_eq!(app.model.kits[0].selected_key.as_deref(), Some(copy.as_str()));
        assert!(tab_open(&app, &copy) && !tab_open(&app, &key));
        assert!(!app.model.kits[0].parsed_tags.contains_key(&key));
        assert!(!is_dirty(&app, &copy), "it holds exactly what was written");
        assert_eq!(distance_on_disk(&kit, COPY), 12.5);
        assert_eq!(distance_on_disk(&kit, MODEL), 0.0, "the original is untouched");

        app.undo_current_tag();
        assert_eq!(distance_in_document(&app, &copy), 0.0, "the undo history came along");
    }

    /// Saving over a tag that is open with unsaved edits would close its tab
    /// and lose them, so it is refused before anything is written.
    #[test]
    fn save_as_over_an_open_modified_tag_is_refused() {
        let (kit, mut app, key, other) = edited("save-as-over-modified");
        edit_field(&mut app, &other, DISTANCE, "3");

        app.save_tag_as_to(&key, &kit.root.join(OTHER));

        assert!(app.model.status.contains("unsaved changes"), "{}", app.model.status);
        assert_eq!(distance_on_disk(&kit, OTHER), 0.0, "nothing was written");
        assert_eq!(distance_in_document(&app, &other), 3.0);
        assert!(tab_open(&app, &key) && is_dirty(&app, &key));
    }

    /// Saving over an open tag with nothing unsaved replaces it: one tab for
    /// that file, now holding the saved document.
    #[test]
    fn save_as_over_an_open_clean_tag_takes_its_place() {
        let (kit, mut app, key, other) = edited("save-as-over-clean");

        app.save_tag_as_to(&key, &kit.root.join(OTHER));

        assert_eq!(distance_on_disk(&kit, OTHER), 12.5);
        assert_eq!(distance_in_document(&app, &other), 12.5);
        let tabs = &app.model.kits[0].open_tabs;
        assert_eq!(tabs.iter().filter(|tab| **tab == other).count(), 1);
        assert!(!tab_open(&app, &key));
        assert!(!is_dirty(&app, &other));
    }

    /// A copy saved outside the loaded tags folder cannot be opened from it,
    /// so the editor stays on the original, still unsaved.
    #[test]
    fn save_as_outside_the_tags_folder_stays_on_the_original() {
        let (kit, mut app, key, _other) = edited("save-as-outside");
        let outside = kit.base.join("elsewhere.model");

        app.save_tag_as_to(&key, &outside);

        assert!(outside.is_file());
        assert!(app.model.status.contains("outside the loaded tags folder"), "{}", app.model.status);
        assert!(tab_open(&app, &key) && is_dirty(&app, &key));
    }

    #[test]
    fn discarding_reloads_the_tag_from_disk() {
        let (_kit, mut app, key, _other) = edited("discard");
        let label = app.model.tag_path_label(&key);

        app.discard_tag_changes(0, &key, &ctx());
        assert_eq!(app.model.status, format!("Discarded unsaved changes to {label}"));
        assert!(!app.model.kits[0].parsed_tags.contains_key(&key), "the document is dropped");
        assert!(app.model.kits[0].loading_tags.contains(&key), "and read again");
        pump_until(&mut app, "the reload", |app| {
            app.model.kits[0].parsed_tags.contains_key(&key)
        });

        assert!(!is_dirty(&app, &key));
        assert_eq!(distance_in_document(&app, &key), 0.0);
        assert!(tab_open(&app, &key));
        assert!(label.ends_with("crate.model"));

        app.discard_tag_changes(0, &key, &ctx());
        assert_eq!(app.model.status, "That tag has no unsaved changes");
    }

    #[test]
    fn discarding_a_closed_tag_drops_its_document_without_reloading() {
        let (_kit, mut app, key, _other) = edited("discard-closed");
        app.kit_and_view(0).close_tag_pane(&key);
        let label = app.model.tag_path_label(&key);

        app.discard_tag_changes(0, &key, &ctx());

        assert_eq!(app.model.status, format!("Discarded unsaved changes to {label}"));
        assert!(!app.model.kits[0].parsed_tags.contains_key(&key));
        assert!(!app.model.kits[0].loading_tags.contains(&key));
    }

    #[test]
    fn undo_and_redo_walk_the_selected_tag_s_edits() {
        let (_kit, mut app, key, _other) = edited("undo-redo");
        edit_field(&mut app, &key, DISTANCE, "20");
        assert_eq!(distance_in_document(&app, &key), 20.0);

        app.undo_current_tag();
        assert_eq!(distance_in_document(&app, &key), 12.5);
        assert!(app.model.status.starts_with("Undo"), "{}", app.model.status);
        app.undo_current_tag();
        assert_eq!(distance_in_document(&app, &key), 0.0);
        app.redo_current_tag();
        app.redo_current_tag();
        assert_eq!(distance_in_document(&app, &key), 20.0);
        assert!(app.model.status.starts_with("Redo"), "{}", app.model.status);
        assert!(is_dirty(&app, &key));

        app.model.kits[0].selected_key = None;
        app.undo_current_tag();
        assert_eq!(app.model.status, "Nothing to undo");
        app.redo_current_tag();
        assert_eq!(app.model.status, "Nothing to redo");
    }

    /// A classic Halo CE tag's undo snapshots are classic bytes, and re-parse
    /// through the classic reader -- the MCC reader cannot read them. The saved
    /// file is classic too.
    #[test]
    fn a_classic_tag_round_trips_through_undo_and_save() {
        let kit = LooseKit::new("undo-classic", "haloce_mcc");
        kit.write_classic_ce("physics/pebble", "point_physics");
        let mut app = app();
        kit.install(&mut app);
        let key = kit.open(&mut app, "physics/pebble.point_physics");
        let friction = |app: &Baboon| real_of(&app.model.kits[0].parsed_tags[&key].tag, "air friction");
        assert_eq!(
            app.model.kits[0].parsed_tags[&key].tag.classic_engine(),
            Some(blam_tags::classic::ClassicEngine::HaloCe)
        );

        edit_field(&mut app, &key, "air friction", "0.25");
        edit_field(&mut app, &key, "air friction", "0.5");
        app.undo_current_tag();
        assert_eq!(friction(&app), Some(0.25));
        assert_eq!(
            app.model.kits[0].parsed_tags[&key].tag.classic_engine(),
            Some(blam_tags::classic::ClassicEngine::HaloCe),
            "the snapshot came back classic"
        );
        app.redo_current_tag();
        assert_eq!(friction(&app), Some(0.5));

        app.save_current_tag(&ctx());
        let bytes = fs::read(kit.root.join("physics/pebble.point_physics")).unwrap();
        assert!(blam_tags::classic::ClassicHeader::parse(&bytes).is_some());
        assert!(TagFile::read_from_bytes(&bytes).is_err(), "not an MCC tag");
        let saved = crate::core::source::read_tag_from_bytes(
            &bytes,
            Some(GameId::HaloCe),
            Some(&locate_definitions_root()),
            group_tag("haloce_mcc", "point_physics"),
        )
        .expect("re-parses through the classic reader");
        assert_eq!(real_of(&saved, "air friction"), Some(0.5));
    }

    /// Quit, start again, restore: the same tabs, folder pane and browser view
    /// come back.
    #[test]
    fn a_session_written_on_exit_restores_its_workspace() {
        let _session = session_file_lock();
        let kit = kit("session");
        let mut app = app();
        kit.install(&mut app);
        app.views[app.model.kits[0].id].browser.mode = BrowserMode::Groups;
        app.views[app.model.kits[0].id].browser.sort = BrowserSort::Type;
        let other = kit.open(&mut app, OTHER);
        let key = kit.open(&mut app, MODEL);
        app.handle_browser_action(
            BrowserAction::OpenFolderBrowser {
                rel_path: PathBuf::from("objects/props"),
                label: "props".to_owned(),
                open_in_new_tab: true,
            },
            ctx(),
        );

        app.persist_session_on_exit();
        let session = load_last_session().expect("written");
        assert_eq!(session.kits.len(), 1);
        let saved = &session.kits[0];
        assert!(matches!(saved.source_kind, LastSessionSourceKind::LooseFolder));
        assert_eq!(saved.source_path, kit.root);
        assert_eq!(saved.game.as_deref(), Some("halo3_mcc"));
        // In `open_tabs` order, which is read off the tile tree and is not
        // stable from run to run (QUIRK: not the order the tabs show in).
        assert_eq!(
            saved.tags.iter().map(|tag| tag.key.clone()).collect::<Vec<_>>(),
            app.model.kits[0]
                .open_tabs
                .iter()
                .filter(|tab| !is_folder_pane_key(tab))
                .cloned()
                .collect::<Vec<_>>()
        );
        assert_eq!(saved.tags.len(), 2);
        let crate_tag = saved.tags.iter().find(|tag| tag.key == key).expect("crate saved");
        assert!(saved.tags.iter().any(|tag| tag.key == other));
        assert_eq!(crate_tag.label, format!("{MODEL} - hlmt (model)"));
        assert_eq!(crate_tag.path.as_deref(), Some(kit.root.join(MODEL).as_path()));
        assert_eq!(crate_tag.group_tag, group_tag("halo3_mcc", "model"));
        assert_eq!(saved.folders.len(), 1);
        assert_eq!(saved.folders[0].rel_path, PathBuf::from("objects/props"));
        assert_eq!(saved.browser_mode, Some(BrowserMode::Groups));
        assert_eq!(saved.browser_sort, Some(BrowserSort::Type));
        assert!(saved.was_active);
        assert!(!saved.has_project);

        let last_saved = saved.tags.last().unwrap().key.clone();

        // A new app, as the next launch builds one, restoring what was written.
        let prompt = LastOpenedWindowsPrompt::from_session(session, &[]).expect("a prompt");
        let mut next = Baboon::for_test();
        next.begin_last_session_restore(prompt.checked_kits(), ctx());
        pump_until(&mut next, "the restore", |app| {
            app.model.kits[app.model.active].open_tabs.len() >= 3
                && app.model.kits[app.model.active].parsed_tags.len() >= 2
        });

        let restored = &next.model.kits[next.model.active];
        let restored_view = &next.views[restored.id];
        assert!(restored.open_tabs.contains(&key) && restored.open_tabs.contains(&other));
        assert!(
            restored_view
                .browser.folder_browsers
                .values()
                .any(|folder| folder.rel_path == Path::new("objects/props"))
        );
        assert_eq!(restored_view.browser.mode, BrowserMode::Groups);
        assert_eq!(restored_view.browser.sort, BrowserSort::Type);
        // The session does not record which tab was selected: each restored tag
        // is selected in turn, so the last one saved ends up selected.
        // QUIRK: with the saved order unstable, so is the restored selection.
        assert_eq!(restored.selected_key.as_ref(), Some(&last_saved));

        // With nothing loaded there is no session, and exit clears the file.
        Baboon::for_test().persist_session_on_exit();
        assert!(load_last_session().is_none());
    }

    #[test]
    fn only_workspace_close_actions_wait_for_chimp_documents() {
        assert!(crate::app::documents::close::close_action_includes_chimp(
            &super::PendingCloseAction::CloseApp
        ));
        assert!(crate::app::documents::close::close_action_includes_chimp(
            &super::PendingCloseAction::CloseKit(super::KitId(1))
        ));
        assert!(!crate::app::documents::close::close_action_includes_chimp(
            &super::PendingCloseAction::CloseAllTabs
        ));
        assert!(!crate::app::documents::close::close_action_includes_chimp(
            &super::PendingCloseAction::CloseTab("tag".to_owned())
        ));
    }

    /// The close prompt's Save sends container tags to the container writers
    /// and only file tags to the file save.
    #[test]
    fn the_close_prompt_saves_container_tags_through_the_containers() {
        use super::TagEntryLocation;
        use crate::app::documents::close::{ClosePromptSave, close_prompt_save_route};
        let route = |location: TagEntryLocation| close_prompt_save_route(Some(&location));
        assert_eq!(
            route(TagEntryLocation::NewContainer {
                template: crate::core::source::NewContainerTemplate::Derived {
                    group: "camera_track".to_owned(),
                },
                package: "/Game/Tags/objects/foo/bar-camera_track".to_owned(),
                group_tag: u32::from_be_bytes(*b"trak"),
            }),
            ClosePromptSave::NewContainer
        );
        assert_eq!(
            route(TagEntryLocation::Container {
                container: 0,
                rel_path: "Meteorite/Content/Tags/objects/a-biped.ubulk".to_owned(),
            }),
            ClosePromptSave::ContainerInPlace
        );
        assert_eq!(
            route(TagEntryLocation::LooseFile(PathBuf::from("/kit/tags/a.biped"))),
            ClosePromptSave::File
        );
        assert_eq!(
            route(TagEntryLocation::Monolithic {
                name: r"objects\a".to_owned(),
                group_tag: u32::from_be_bytes(*b"bipd"),
            }),
            ClosePromptSave::File
        );
        assert_eq!(close_prompt_save_route(None), ClosePromptSave::File);
    }

    /// Closing one tab leaves a color popup open on another tag, with its
    /// unconfirmed color; closing the popup's own tag closes it.
    #[test]
    fn closing_a_tab_keeps_another_tags_popup() {
        use crate::app::editor::{ColorPopupWindow, MaterialColorPopup};
        let kit = LooseKit::new("close-popups", "haloce_mcc");
        kit.write_classic_ce("weapons/a", "weapon");
        kit.write_classic_ce("weapons/b", "weapon");
        let mut app = app();
        kit.install(&mut app);
        let a = kit.open(&mut app, "weapons/a.weapon");
        let b = kit.open(&mut app, "weapons/b.weapon");
        let kit_id = app.model.kits[0].id;
        let popup = MaterialColorPopup::new("tint", 1.0, 0.5, 0.0, 1.0).with_write(&a, "tint");
        app.dialogs.open(ColorPopupWindow { popup: Some(popup), kit: kit_id, opened_at: None });

        app.close_tab(&b);
        assert!(app.dialogs.get::<ColorPopupWindow>().is_some(), "a's popup outlives b's tab");
        app.close_tab(&a);
        assert!(app.dialogs.get::<ColorPopupWindow>().is_none(), "and goes with a's");
    }

    /// Re-entering the value a field already holds changes nothing: no
    /// modified mark, no undo step. And undoing back to the state a tag was
    /// saved in leaves it unmodified; a redo away from it marks it again.
    #[test]
    fn undo_back_to_the_saved_state_is_unmodified() {
        let kit = LooseKit::new("undo-saved", "haloce_mcc");
        kit.write_classic_ce("physics/pebble", "point_physics");
        let mut app = app();
        kit.install(&mut app);
        let key = kit.open(&mut app, "physics/pebble.point_physics");
        fn doc<'a>(app: &'a Baboon, key: &str) -> &'a crate::core::document::TagDocument {
            &app.model.kits[0].parsed_tags[key]
        }
        let friction = |app: &Baboon| real_of(&doc(app, &key).tag, "air friction");

        edit_field(&mut app, &key, "air friction", "0");
        assert!(!doc(&app, &key).dirty.is_set(), "the same value is no edit");
        assert!(!doc(&app, &key).journal.can_undo(), "and takes no undo step");

        edit_field(&mut app, &key, "air friction", "0.25");
        app.model.kits[0].parsed_tags.get_mut(&key).unwrap().mark_saved();
        edit_field(&mut app, &key, "air friction", "0.5");
        assert!(doc(&app, &key).dirty.is_set());

        app.undo_current_tag();
        assert_eq!(friction(&app), Some(0.25));
        assert!(!doc(&app, &key).dirty.is_set(), "back to the saved state");
        app.redo_current_tag();
        assert_eq!(friction(&app), Some(0.5));
        assert!(doc(&app, &key).dirty.is_set(), "away from it again");
        app.undo_current_tag();
        assert!(!doc(&app, &key).dirty.is_set());
        app.undo_current_tag();
        assert_eq!(friction(&app), Some(0.0));
        assert!(doc(&app, &key).dirty.is_set(), "before the save is unsaved");
    }
}
