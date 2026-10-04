//! Document and tab-selection helpers.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::TagLoaded`, discarding results for tabs closed while loading.
    pub(in crate::app) fn handle_tag_loaded(
        &mut self,
        kit: KitId,
        key: String,
        result: Result<TagFile, String>,
    ) -> bool {
        // Routed by kit id rather than generation: a parsed document stays
        // valid across a source reload, and must land in the kit that asked
        // for it even if the user has since switched to another.
        let Some(index) = self.model.resolve_kit(kit) else {
            return true;
        };
        self.model.kits[index].loading_tags.remove(&key);
        if !self.model.kits[index].open_tabs.iter().any(|tab| tab == &key) {
            return true;
        }
        match result {
            Ok(tag) => {
                self.model.status = "Tag loaded".to_owned();
                self.model.kits[index]
                    .parsed_tags
                    .insert(key.clone(), TagDocument::clean(tag));
                // A restored session stages this tag's undo history before the
                // read that produces the document, so it is waiting here.
                self.apply_pending_history(index, &key);
            }
            Err(error) => {
                let name = self
                    .model.entry_for_key_in(index, &key)
                    .map(|entry| entry.display_path.clone())
                    .unwrap_or_else(|| key.clone());
                let message = format!("Could not load {name}: {error}");
                self.kit_tools.terminal
                    .lines
                    .push(TerminalLineEntry::new(message.clone()));
                trim_terminal_lines(&mut self.kit_tools.terminal.lines);
                self.kit_tools.terminal.scroll_to_bottom = true;
                self.model.status = message;
            }
        }
        false
    }

    /// Applies `WorkerMessage::BitmapReimportFinished` and reloads an open bitmap document.
    pub(in crate::app) fn handle_bitmap_reimport_finished(
        &mut self,
        kit: KitId,
        key: String,
        result: Result<TagFile, String>,
    ) -> bool {
        // The terminal reset is global and must run even when the owning kit
        // has closed, so it happens before the routing check.
        self.kit_tools.terminal.running = false;
        self.kit_tools.terminal.running_id = None;
        self.kit_tools.terminal.running_command = None;
        self.kit_tools.terminal.process = None;
        self.kit_tools.terminal.scroll_to_bottom = true;
        self.kit_tools.terminal.refocus_input = true;
        let Some(index) = self.model.resolve_kit(kit) else {
            return true;
        };
        match result {
            Ok(tag) => {
                if self.model.kits[index].open_tabs.iter().any(|tab| tab == &key) {
                    self.model.kits[index]
                        .parsed_tags
                        .insert(key.clone(), TagDocument::clean(tag));
                    self.views[self.model.kits[index].id].caches.bitmap_previews.remove(&key);
                }
                self.model.status = "Bitmap reimported and reloaded".to_owned();
            }
            Err(error) => self.model.status = format!("Bitmap reimport failed: {error}"),
        }
        false
    }
}

#[cfg(test)]
mod tag_load_failure_tests;

impl Baboon {




    pub(in crate::app) fn select_entry(&mut self, key: String, ctx: egui::Context) {
        self.kit_and_view(self.model.active).open_tag_pane(&key);
        self.model.kits[self.model.active].selected_key = Some(key.clone());
        // A tag the project has an overlay for opens from the project, not from
        // disk — otherwise reopening it would silently discard its edits.
        if !self.load_campaign_overlay_for_key(self.model.active, &key) {
            self.ensure_tag_loading(key, ctx);
        }
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn ensure_tag_loading(&mut self, key: String, ctx: egui::Context) {
        if self.model.kits[self.model.active].parsed_tags.contains_key(&key)
            || self.model.kits[self.model.active].loading_tags.contains(&key)
        {
            return;
        }
        let Some(source) = self.model.source() else {
            return;
        };
        // Check both the lazily-loaded entries and the full scan set (all_entries).
        // Flat search results reference all_entries, which may not overlap with entries.
        let Some(entry) = source
            .entry_for_key(&key)
            .or_else(|| {
                self.model.kits[self.model.active].active_favorite_entries
                    .iter()
                    .find(|e| e.key == key)
            })
            .cloned()
        else {
            return;
        };
        // A new tag reaching here has lost its document and its project overlay
        // (`select_entry` tries the overlay first), so there is nothing left to
        // read — `read_entry` would only fail on the worker thread. Retire the
        // entry here instead of leaving a row that fails forever.
        if matches!(entry.location, TagEntryLocation::NewContainer { .. }) {
            let kit = self.model.active;
            self.forget_new_container_entry(kit, &key);
            self.model.status = format!("The unsaved new tag {} was discarded", entry.display_path);
            return;
        }
        let source_kind = source.source.clone();
        let kit = self.model.active_kit_id();
        self.model.kits[self.model.active].loading_tags.insert(key.clone());
        self.model.status = format!("Loading {}", entry.display_path);
        let panic_key = key.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || {
                let result = read_entry(&source_kind, &entry).map_err(|error| format!("{error:#}"));
                WorkerMessage::TagLoaded { kit, key, result }
            },
            move |error| WorkerMessage::TagLoaded {
                kit,
                key: panic_key,
                result: Err(error),
            },
        );
    }







    pub(in crate::app) fn unload_tag(&mut self, key: &str) {
        self.kit_and_view(self.model.active).drop_document(key);
    }

    /// Drop cached previews derived from a tag's contents so they rebuild from
    /// the (newly restored) tag bytes after an undo/redo.
    /// Drop derived previews for `key` in `kit`, after its document changed.
    pub(in crate::app) fn invalidate_tag_caches_in(&mut self, kit: usize, key: &str) {
        if let Some(preview) = self.views[self.model.kits[kit].id].caches.model_previews.get_mut(key) {
            preview.invalidate_load();
        }
        if let Some(bitmap) = self.views[self.model.kits[kit].id].caches.bitmap_previews.get_mut(key) {
            bitmap.decoded = None;
            bitmap.decoding = None;
            bitmap.texture = None;
            bitmap.texture_dirty = true;
        }
        // rmdf/rmop caches are keyed by external render-method paths, not by this
        // tag's contents, and the shader grid's model is keyed by the document's
        // dirty revision, which the change has already moved — so nothing to
        // clear there.
    }
}

impl Model {
    pub(in crate::app) fn loaded_tags_root(&self) -> Option<PathBuf> {
        self.loaded_tags_root_for(self.active)
    }

    /// A specific kit's loose tags root. Background work has to name its kit:
    /// the one it started in may no longer be the focused one when it lands.
    pub(in crate::app) fn loaded_tags_root_for(&self, kit: usize) -> Option<PathBuf> {
        let TagSource::LooseFolder { root, .. } = &self.kits.get(kit)?.source.as_ref()?.source
        else {
            return None;
        };
        Some(root.clone())
    }

    /// Kept for save/export paths that address "the current tag".
    #[allow(dead_code)]
    pub(in crate::app) fn selected_entry(&self) -> Option<&TagEntry> {
        let key = self.kits[self.active].selected_key.as_ref()?;
        self.entry_for_key(key)
    }

    pub(in crate::app) fn entry_for_key(&self, key: &str) -> Option<&TagEntry> {
        self.entry_for_key_in(self.active, key)
    }

    /// Resolve a tag key against a specific kit. Anything that runs for a kit
    /// other than the focused one has to use this: a key only means something
    /// inside its own source, so resolving it against the active kit silently
    /// finds nothing and the caller skips the tag.
    pub(in crate::app) fn entry_for_key_in(&self, kit: usize, key: &str) -> Option<&TagEntry> {
        self.kits.get(kit)?.entry_for_key(key)
    }
}
