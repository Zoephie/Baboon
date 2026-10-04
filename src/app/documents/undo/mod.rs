//! Undo and redo for the open tag, through its journal, and for Chimp's surface
//! when it is the one in front.

use super::*;
use anyhow::Context as _;

impl Baboon {
    pub(in crate::app) fn undo_current_tag(&mut self) {
        if self.chimp_surface_is_active() {
            self.model.status = "Chimp has no undo yet; undo applies to tags.".to_owned();
            return;
        }
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        let Some(key) = self.model.kits[self.model.active].selected_key.clone() else {
            self.model.status = "Nothing to undo".to_owned();
            return;
        };
        let restored = self.model.kits[self.model.active]
            .parsed_tags
            .get_mut(&key)
            .and_then(|doc| doc.journal.undo(&doc.tag));
        self.restore_snapshot(&key, restored, "Undo");
    }

    pub(in crate::app) fn redo_current_tag(&mut self) {
        if self.chimp_surface_is_active() {
            self.model.status = "Chimp has no redo yet; redo applies to tags.".to_owned();
            return;
        }
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        let Some(key) = self.model.kits[self.model.active].selected_key.clone() else {
            self.model.status = "Nothing to redo".to_owned();
            return;
        };
        let restored = self.model.kits[self.model.active]
            .parsed_tags
            .get_mut(&key)
            .and_then(|doc| doc.journal.redo(&doc.tag));
        self.restore_snapshot(&key, restored, "Redo");
    }

    /// Apply a snapshot returned by the journal: re-parse the bytes into the
    /// document and invalidate derived caches.
    pub(in crate::app) fn restore_snapshot(
        &mut self,
        key: &str,
        restored: Option<(Arc<Vec<u8>>, String)>,
        verb: &str,
    ) {
        // Classic (Halo CE / Halo 2) snapshots are serialized in classic format,
        // which `read_from_bytes` can't parse — re-parse with the JSON layout.
        let group_tag = self.model.kits[self.model.active]
            .parsed_tags
            .get(key)
            .map(|doc| doc.tag.group().tag);
        let game = self.source_game();
        let definitions_root = self.source_definitions_root().map(Path::to_owned);
        match restored {
            Some((bytes, label)) => {
                match group_tag
                    .context("no open tag to restore")
                    .and_then(|group_tag| {
                        crate::core::source::read_tag_from_bytes(
                            &bytes,
                            game,
                            definitions_root.as_deref(),
                            group_tag,
                        )
                    }) {
                    Ok(tag) => {
                        if let Some(doc) = self.model.kits[self.model.active].parsed_tags.get_mut(key) {
                            doc.tag = tag;
                            doc.dirty.touch();
                        }
                        let active = self.model.active;
                        self.invalidate_tag_caches_in(active, key);
                        self.model.status = format!("{verb}: {label}");
                    }
                    Err(error) => {
                        self.model.status = format!("{verb} failed: {error}");
                    }
                }
            }
            None => {
                self.model.status = format!("Nothing to {}", verb.to_ascii_lowercase());
            }
        }
    }

    pub(in crate::app) fn can_undo_current(&self) -> bool {
        if self.chimp_surface_is_active() || self.editing_kit_is_read_only(self.model.active) {
            return false;
        }
        self.model.kits[self.model.active]
            .selected_key
            .as_ref()
            .and_then(|key| self.model.kits[self.model.active].parsed_tags.get(key))
            .is_some_and(|doc| doc.journal.can_undo())
    }

    pub(in crate::app) fn can_redo_current(&self) -> bool {
        if self.chimp_surface_is_active() || self.editing_kit_is_read_only(self.model.active) {
            return false;
        }
        self.model.kits[self.model.active]
            .selected_key
            .as_ref()
            .and_then(|key| self.model.kits[self.model.active].parsed_tags.get(key))
            .is_some_and(|doc| doc.journal.can_redo())
    }
}

#[cfg(test)]
mod chimp_surface_undo_tests;
