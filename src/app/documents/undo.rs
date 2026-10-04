//! Undo and redo for the open tag, through its journal, or for the selected
//! Chimp package when Chimp's surface is the one in front.

use super::*;
use anyhow::Context as _;

impl Baboon {
    pub(in crate::app) fn undo_current_tag(&mut self) {
        if self.chimp_surface_is_active() {
            self.step_current_chimp_journal(false);
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
            self.step_current_chimp_journal(true);
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
        let game = self.model.source_game();
        let definitions_root = self.model.source_definitions_root().map(Path::to_owned);
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
        if self.chimp_surface_is_active() {
            return self.can_step_chimp_journal(false);
        }
        if self.model.editing_kit_is_read_only(self.model.active) {
            return false;
        }
        self.model.kits[self.model.active]
            .selected_key
            .as_ref()
            .and_then(|key| self.model.kits[self.model.active].parsed_tags.get(key))
            .is_some_and(|doc| doc.journal.can_undo())
    }

    pub(in crate::app) fn can_redo_current(&self) -> bool {
        if self.chimp_surface_is_active() {
            return self.can_step_chimp_journal(true);
        }
        if self.model.editing_kit_is_read_only(self.model.active) {
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
mod chimp_surface_undo_tests {
    //! Undo and redo on the Chimp surface.
    //!
    //! Ctrl+Z, Ctrl+Y and the Edit menu act on the selected tag. On the Chimp
    //! surface that tag is hidden, so they used to change it without the user
    //! seeing anything happen. There they act on the selected package instead
    //! (see `chimp::edit`), and with none selected they do nothing.

    use super::*;
    use crate::app::chimp::KitSurface;

    const KEY: &str = "ublock:pakchunk0:objects/vehicles/warthog";

    fn app_with_undoable_tag() -> Baboon {
        let definition = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("definitions")
            .join("haloce_evolved")
            .join("cinematic_scene.json");
        let tag = TagFile::new(definition).expect("build a tag from the CE schema");
        let mut document = TagDocument::modified(tag);
        document.journal.begin_edit(&document.tag, "Edit");
        document.journal.end_edit_window();
        let mut app = Baboon::for_test();
        app.model.prefs.enable_chimp = true;
        app.model.kits[0].parsed_tags.insert(KEY.to_owned(), document);
        app.model.kits[0].selected_key = Some(KEY.to_owned());
        app
    }

    fn can_undo_tag(app: &Baboon) -> bool {
        app.model.kits[0].parsed_tags[KEY].journal.can_undo()
    }

    #[test]
    fn undo_on_the_chimp_surface_leaves_the_hidden_tag_alone() {
        let mut app = app_with_undoable_tag();
        app.views[app.model.kits[0].id].surface = KitSurface::Chimp;

        assert!(!app.can_undo_current(), "the Edit menu's Undo is disabled");
        app.undo_current_tag();
        assert!(can_undo_tag(&app), "the hidden tag's history is untouched");
        app.redo_current_tag();
        assert!(!app.model.kits[0].parsed_tags[KEY].journal.can_redo());

        // Back on the tag surface, the same undo acts on the tag.
        app.views[app.model.kits[0].id].surface = KitSurface::Tags;
        assert!(app.can_undo_current());
        app.undo_current_tag();
        assert!(!can_undo_tag(&app));
        assert!(app.model.kits[0].parsed_tags[KEY].journal.can_redo());
    }
}
