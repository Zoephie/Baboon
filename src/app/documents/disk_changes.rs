//! Noticing that an open tag's file changed on disk outside Baboon: Explorer,
//! Tool's bitmap monitoring, another editor. Each open document remembers its
//! file as it was read or saved ([`DiskStamp`]); when Baboon's window regains
//! focus the open loose tags are checked against it off the UI thread. A tag
//! with no unsaved edits is read again; one with edits gets a banner asking
//! whether to reload it or keep the edits. A tag whose file is gone stays
//! open, marked missing, until it is saved back or closed.

use super::*;

/// One open tag's file, checked against the stamp it had when the check began.
pub(in crate::app) struct OpenTagCheck {
    pub(in crate::app) key: String,
    pub(in crate::app) checked: DiskStamp,
    pub(in crate::app) result: DiskCheck,
}

/// Why an open tag is being read again from its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum DiskReload {
    /// It had no unsaved edits and the file changed.
    Unedited,
    /// The user chose Reload from Disk over their unsaved edits.
    DiscardEdits,
}

/// A question the tag's banner is asking about its file.
#[derive(Clone)]
pub(in crate::app) enum DiskNotice {
    /// The file changed while the tag had unsaved edits. Holds the file as
    /// it is now, which Keep My Edits accepts as the version it replaces.
    Changed(DiskStamp),
    /// Nothing is at the tag's path any more. Holds the tag's browser entry:
    /// the kit's next index refresh drops it, and the tab, its banner and
    /// Save still need it.
    Missing(TagEntry),
}

/// The banner's answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum DiskChoice {
    ReloadFromDisk,
    KeepMyEdits,
    /// Close a missing tag's tab, through the usual prompt if it has edits.
    CloseTab,
}

/// One kit's open tags whose files changed: reads in flight and banners
/// waiting for an answer, by key.
#[derive(Default)]
pub(in crate::app) struct DiskWatch {
    pub(in crate::app) reloads: HashMap<String, DiskReload>,
    pub(in crate::app) notices: HashMap<String, DiskNotice>,
}

impl DiskWatch {
    /// A renamed tag keeps its read in flight and its banner.
    pub(in crate::app) fn rekey(&mut self, old: &str, new: &str) {
        if let Some(reload) = self.reloads.remove(old) {
            self.reloads.insert(new.to_owned(), reload);
        }
        if let Some(notice) = self.notices.remove(old) {
            self.notices.insert(new.to_owned(), notice);
        }
    }

    /// The entry of an open tag whose file is missing.
    pub(in crate::app) fn missing_entry(&self, key: &str) -> Option<&TagEntry> {
        match self.notices.get(key)? {
            DiskNotice::Missing(entry) => Some(entry),
            DiskNotice::Changed(_) => None,
        }
    }

    /// The tag's document went or now matches its file: nothing to ask.
    pub(in crate::app) fn forget(&mut self, key: &str) {
        self.reloads.remove(key);
        self.notices.remove(key);
    }
}

impl Baboon {
    /// Check the open tags when the window comes back into focus: the user
    /// was most likely away changing files. A window whose focus is not
    /// reported (headless tests) never "gains" it.
    pub(in crate::app) fn check_open_tags_on_focus(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|input| input.viewport().focused).unwrap_or(true);
        let gained = focused && !self.documents.window_focused;
        self.documents.window_focused = focused;
        if gained {
            self.check_open_tags_on_disk(ctx);
        }
    }

    /// Start a check of every open loose tag, in every kit, against its file.
    pub(in crate::app) fn check_open_tags_on_disk(&mut self, ctx: &egui::Context) {
        for kit in &self.model.kits {
            let Some(source) = kit.source.as_ref() else {
                continue;
            };
            if !matches!(source.source, TagSource::LooseFolder { .. }) {
                continue;
            }
            let files: Vec<(String, PathBuf, DiskStamp)> = kit
                .open_tabs
                .iter()
                .filter_map(|key| {
                    let stamp = kit.parsed_tags.get(key)?.disk?;
                    let TagEntryLocation::LooseFile(path) = &kit.entry_for_key(key)?.location else {
                        return None;
                    };
                    Some((key.clone(), path.clone(), stamp))
                })
                .collect();
            if files.is_empty() {
                continue;
            }
            let id = kit.id;
            spawn_worker(
                &self.tx,
                ctx,
                move || WorkerMessage::OpenTagsChecked {
                    kit: id,
                    checks: files
                        .into_iter()
                        .map(|(key, path, checked)| OpenTagCheck { result: checked.check(&path), key, checked })
                        .collect(),
                },
                move |_| WorkerMessage::OpenTagsChecked { kit: id, checks: Vec::new() },
            );
        }
    }

    /// Applies `WorkerMessage::OpenTagsChecked`.
    pub(in crate::app) fn handle_open_tags_checked(
        &mut self,
        kit: KitId,
        checks: Vec<OpenTagCheck>,
        ctx: &egui::Context,
    ) -> bool {
        let Some(index) = self.model.resolve_kit(kit) else {
            return true;
        };
        for OpenTagCheck { key, checked, result } in checks {
            let kit = &mut self.model.kits[index];
            let entry = kit.entry_for_key(&key).cloned();
            let was_missing = kit.disk.missing_entry(&key).is_some();
            let Some(doc) = kit.parsed_tags.get_mut(&key) else {
                continue;
            };
            // Saved, reloaded or closed and reopened while the check ran:
            // the answer is about a file the document no longer stands for.
            if doc.disk != Some(checked) {
                continue;
            }
            // The file is back: whatever it holds is answered below.
            if was_missing && !matches!(result, DiskCheck::Missing | DiskCheck::Unreadable(_)) {
                kit.disk.notices.remove(&key);
            }
            match result {
                DiskCheck::Same | DiskCheck::Unreadable(_) => {}
                DiskCheck::Restamped(stamp) => doc.disk = Some(stamp),
                DiskCheck::Changed(_) if !doc.dirty.is_set() => {
                    if kit.loading_tags.contains(&key) {
                        continue;
                    }
                    kit.disk.reloads.insert(key.clone(), DiskReload::Unedited);
                    self.load_tag_in(index, key, ctx.clone(), true);
                }
                // Unsaved edits are never replaced without asking. A banner
                // already up takes the newer file, so Keep accepts that one.
                DiskCheck::Changed(now) => {
                    kit.disk.notices.insert(key, DiskNotice::Changed(now));
                }
                // The tab stays open with what it holds; saving puts the
                // file back.
                DiskCheck::Missing => {
                    if let (Some(entry), false) = (entry, was_missing) {
                        kit.disk.notices.insert(key, DiskNotice::Missing(entry));
                    }
                }
            }
        }
        false
    }

    /// Carry out the answer to a tag's changed-on-disk banner.
    pub(in crate::app) fn apply_disk_choice(&mut self, kit: KitId, key: String, choice: DiskChoice, ctx: &egui::Context) {
        let Some(index) = self.model.resolve_kit(kit) else {
            return;
        };
        if choice == DiskChoice::CloseTab {
            // The banner stays until the tab is gone: the save-changes
            // prompt this may raise can be cancelled.
            self.focus_kit(index);
            self.request_close_action(PendingCloseAction::CloseTab(key), ctx);
            return;
        }
        let kit = &mut self.model.kits[index];
        let Some(notice) = kit.disk.notices.remove(&key) else {
            return;
        };
        match (choice, notice) {
            // Trusted from here on: the document now stands for the file as
            // it is, so the next Save writes over it without asking again.
            (DiskChoice::KeepMyEdits, DiskNotice::Changed(now)) => {
                if let Some(doc) = kit.parsed_tags.get_mut(&key) {
                    doc.disk = Some(now);
                }
            }
            (DiskChoice::ReloadFromDisk, DiskNotice::Changed(_)) => {
                kit.disk.reloads.insert(key.clone(), DiskReload::DiscardEdits);
                self.load_tag_in(index, key, ctx.clone(), true);
            }
            // Not offered for a missing file; it keeps its banner.
            (_, notice) => {
                kit.disk.notices.insert(key, notice);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::{LooseKit, app, ctx, edit_field, pump_until, real_of};
    use std::time::{Duration, SystemTime};

    const DISTANCE: &str = "disappear distance";

    fn set_distance(tag: &mut TagFile, value: f32) {
        tag.root_mut().field_mut(DISTANCE).unwrap().set(TagFieldData::Real(value)).unwrap();
    }

    /// The same file under a different time, so the change is seen however
    /// coarse the file system's clock is.
    fn bump_modified(path: &Path) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(120))
            .unwrap();
    }

    /// Run a check, apply its answer, then wait for any read it started.
    fn check_and_wait(app: &mut Baboon, kit: KitId) {
        let ctx = ctx();
        app.check_open_tags_on_disk(&ctx);
        loop {
            let message = app.rx.recv_timeout(Duration::from_secs(60)).expect("the check answers");
            let answer = matches!(message, WorkerMessage::OpenTagsChecked { .. });
            app.apply_worker_message(message, &ctx);
            if answer {
                break;
            }
        }
        pump_until(app, "the reload", |app| {
            let kit = &app.model.kits[app.model.resolve_kit(kit).unwrap()];
            kit.loading_tags.is_empty() && kit.disk.reloads.is_empty()
        });
    }

    #[test]
    fn an_unedited_tag_changed_on_disk_is_reloaded_with_a_notice() {
        let fixture = LooseKit::new("disk-reload", "halo3_mcc");
        let path = fixture.write_mcc("warthog", "model", |_| {});
        let mut app = app();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "warthog.model");
        let kit = app.model.kits[0].id;
        let first = app.model.kits[0].parsed_tags[&key].id;

        fixture.write_mcc("warthog", "model", |tag| {
            set_distance(tag, 7.5);
        });
        bump_modified(&path);
        check_and_wait(&mut app, kit);

        let doc = &app.model.kits[0].parsed_tags[&key];
        assert_ne!(doc.id, first, "the tag was read again");
        assert_eq!(real_of(&doc.tag, DISTANCE), Some(7.5));
        assert!(doc.journal.top_id().is_none(), "undo cannot bring back the old version");
        assert_eq!(app.model.status, "Reloaded warthog.model: changed on disk");
    }

    /// Switching back to Baboon is what starts a check: a frame whose window
    /// has just regained focus, as `App::logic` runs it.
    #[test]
    fn regaining_window_focus_reloads_a_changed_tag() {
        let fixture = LooseKit::new("disk-focus", "halo3_mcc");
        let path = fixture.write_mcc("warthog", "model", |_| {});
        let mut app = app();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "warthog.model");
        let ctx = ctx();
        let frame = |app: &mut Baboon, focused: bool, time: f64| {
            let mut input = crate::app::loose_fixture::screen(Vec::new(), time);
            input.viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(focused);
            let _ = ctx.run_logic(&input, |ctx| app.run_logic(ctx));
        };
        frame(&mut app, false, 1.0);
        fixture.write_mcc("warthog", "model", |tag| set_distance(tag, 7.5));
        bump_modified(&path);
        assert!(app.model.kits[0].loading_tags.is_empty());

        frame(&mut app, true, 2.0);
        pump_until(&mut app, "the reload", |app| {
            real_of(&app.model.kits[0].parsed_tags[&key].tag, DISTANCE) == Some(7.5)
        });
    }

    #[test]
    fn baboons_own_save_is_not_taken_for_a_change() {
        let fixture = LooseKit::new("disk-own-save", "halo3_mcc");
        fixture.write_mcc("warthog", "model", |_| {});
        let mut app = app();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "warthog.model");
        let kit = app.model.kits[0].id;
        edit_field(&mut app, &key, DISTANCE, "3");
        app.save_tag_by_key(&key).expect("save");
        let saved = app.model.kits[0].parsed_tags[&key].id;

        check_and_wait(&mut app, kit);

        let doc = &app.model.kits[0].parsed_tags[&key];
        assert_eq!(doc.id, saved, "the saved document stays");
        assert!(doc.journal.top_id().is_some(), "and keeps its undo history");
    }

    #[test]
    fn a_touched_file_with_the_same_bytes_is_not_reloaded() {
        let fixture = LooseKit::new("disk-touch", "halo3_mcc");
        let path = fixture.write_mcc("warthog", "model", |_| {});
        let mut app = app();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "warthog.model");
        let kit = app.model.kits[0].id;
        let first = app.model.kits[0].parsed_tags[&key].id;

        bump_modified(&path);
        check_and_wait(&mut app, kit);

        assert_eq!(app.model.kits[0].parsed_tags[&key].id, first);
    }

    /// A tag edited to 3 whose file was then rewritten with 7.5, checked: the
    /// state a banner is up in.
    fn edited_then_changed_on_disk(name: &str) -> (LooseKit, Baboon, String, PathBuf) {
        let fixture = LooseKit::new(name, "halo3_mcc");
        let path = fixture.write_mcc("warthog", "model", |_| {});
        let mut app = app();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "warthog.model");
        let kit = app.model.kits[0].id;
        edit_field(&mut app, &key, DISTANCE, "3");
        fixture.write_mcc("warthog", "model", |tag| set_distance(tag, 7.5));
        bump_modified(&path);
        check_and_wait(&mut app, kit);
        (fixture, app, key, path)
    }

    fn choose(app: &mut Baboon, key: &str, choice: DiskChoice) {
        let kit = app.model.kits[0].id;
        app.apply_disk_choice(kit, key.to_owned(), choice, &ctx());
        pump_until(app, "the choice to settle", |app| {
            app.model.kits[0].loading_tags.is_empty() && app.model.kits[0].disk.reloads.is_empty()
        });
    }

    /// Unsaved edits are never replaced silently: the tag keeps them and its
    /// banner asks.
    #[test]
    fn a_tag_with_unsaved_edits_gets_a_banner_instead_of_a_reload() {
        let (_fixture, app, key, _) = edited_then_changed_on_disk("disk-dirty");

        let doc = &app.model.kits[0].parsed_tags[&key];
        assert!(doc.dirty.is_set());
        assert_eq!(real_of(&doc.tag, DISTANCE), Some(3.0));
        assert!(matches!(app.model.kits[0].disk.notices.get(&key), Some(DiskNotice::Changed(_))));
    }

    #[test]
    fn reload_from_disk_discards_the_edits_for_the_file() {
        let (_fixture, mut app, key, _) = edited_then_changed_on_disk("disk-choose-reload");

        choose(&mut app, &key, DiskChoice::ReloadFromDisk);

        let doc = &app.model.kits[0].parsed_tags[&key];
        assert!(!doc.dirty.is_set());
        assert_eq!(real_of(&doc.tag, DISTANCE), Some(7.5));
        assert!(app.model.kits[0].disk.notices.is_empty());
        assert_eq!(app.model.status, "Reloaded warthog.model from disk; unsaved edits discarded");
    }

    /// Keep is trusted: the banner goes, the edits stay, and the file as it
    /// was when the user chose is not asked about again.
    #[test]
    fn keep_my_edits_keeps_them_and_stops_asking() {
        let (_fixture, mut app, key, _) = edited_then_changed_on_disk("disk-choose-keep");
        let kit = app.model.kits[0].id;

        choose(&mut app, &key, DiskChoice::KeepMyEdits);
        assert!(app.model.kits[0].disk.notices.is_empty());
        check_and_wait(&mut app, kit);

        let doc = &app.model.kits[0].parsed_tags[&key];
        assert!(doc.dirty.is_set());
        assert_eq!(real_of(&doc.tag, DISTANCE), Some(3.0));
        assert!(app.model.kits[0].disk.notices.is_empty(), "the same file is not asked about twice");
    }

    /// Kept edits whose file then changes again: that is a new question.
    #[test]
    fn a_later_change_after_keep_asks_again() {
        let (fixture, mut app, key, path) = edited_then_changed_on_disk("disk-keep-again");
        let kit = app.model.kits[0].id;
        choose(&mut app, &key, DiskChoice::KeepMyEdits);

        fixture.write_mcc("warthog", "model", |tag| set_distance(tag, 9.0));
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(600))
            .unwrap();
        check_and_wait(&mut app, kit);

        assert!(app.model.kits[0].disk.notices.contains_key(&key));
    }

    /// Saving answers the banner too: the edits are now the file.
    #[test]
    fn saving_over_the_changed_file_clears_the_banner() {
        let (_fixture, mut app, key, _) = edited_then_changed_on_disk("disk-save-clears");

        app.save_tag_by_key(&key).expect("save");

        assert!(app.model.kits[0].disk.notices.is_empty());
    }

    /// A kit with an indexed tag in its own folder, open, then deleted along
    /// with that folder and checked.
    fn opened_then_deleted(name: &str) -> (LooseKit, Baboon, String, PathBuf) {
        let fixture = LooseKit::new(name, "halo3_mcc");
        let path = fixture.write_mcc("vehicles\\warthog", "model", |tag| set_distance(tag, 4.0));
        // A second tag, as any real kit has: an empty listing reads as one
        // not scanned yet.
        fixture.write_mcc("scenery\\rock", "model", |_| {});
        let mut app = app();
        fixture.install_indexed(&mut app);
        let key = fixture.open(&mut app, "vehicles\\warthog.model");
        let kit = app.model.kits[0].id;
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
        check_and_wait(&mut app, kit);
        (fixture, app, key, path)
    }

    /// A full refresh, as the periodic one is once its cached index sees a
    /// file go. The fixture kit has no cached index to compare against.
    fn refresh_index(app: &mut Baboon) {
        app.model.kits[0].index_jobs.refresh_browser = true;
        app.begin_refresh_entry_index(ctx());
        pump_until(app, "the index refresh", |app| !app.model.kits[0].index_jobs.refreshing);
    }

    #[test]
    fn a_deleted_tag_stays_open_marked_missing() {
        let (_fixture, app, key, _) = opened_then_deleted("disk-missing");

        let kit = &app.model.kits[0];
        assert!(kit.open_tabs.contains(&key));
        assert_eq!(real_of(&kit.parsed_tags[&key].tag, DISTANCE), Some(4.0));
        assert!(kit.disk.missing_entry(&key).is_some());
    }

    /// The index refresh drops the deleted tag from the browser, but the open
    /// tab keeps its entry, and a tag deleted before any focus check is
    /// marked missing by the refresh itself.
    #[test]
    fn the_index_refresh_marks_a_deleted_open_tag_missing_and_keeps_its_entry() {
        let fixture = LooseKit::new("disk-missing-refresh", "halo3_mcc");
        let path = fixture.write_mcc("vehicles\\warthog", "model", |_| {});
        let mut app = app();
        fixture.install_indexed(&mut app);
        let key = fixture.open(&mut app, "vehicles\\warthog.model");
        fs::remove_file(&path).unwrap();

        refresh_index(&mut app);

        let kit = &app.model.kits[0];
        let listed = |key: &str| kit.source.as_ref().unwrap().all_entries.iter().any(|entry| entry.key == key);
        assert!(!listed(&key), "gone from the browser; status {}", app.model.status);
        assert!(kit.disk.missing_entry(&key).is_some());
        assert!(app.model.entry_for_key(&key).is_some(), "the tab still finds its tag");
    }

    /// Saving puts the file back, folder and all, and the tag back in the
    /// browser, after the refresh had dropped it.
    #[test]
    fn saving_a_missing_tag_recreates_its_file() {
        let (_fixture, mut app, key, path) = opened_then_deleted("disk-missing-save");
        refresh_index(&mut app);

        app.save_tag_by_key(&key).expect("save");

        assert!(path.is_file());
        let kit = &app.model.kits[0];
        assert!(kit.disk.notices.is_empty());
        assert!(
            kit.source.as_ref().unwrap().all_entries.iter().any(|entry| entry.key == key),
            "back in the browser"
        );
    }

    /// The file put back outside Baboon (an undo in Explorer, a Git
    /// restore): the tag is no longer missing.
    #[test]
    fn a_missing_file_put_back_clears_the_banner() {
        let (fixture, mut app, key, _) = opened_then_deleted("disk-missing-back");
        let kit = app.model.kits[0].id;

        fixture.write_mcc("vehicles\\warthog", "model", |tag| set_distance(tag, 4.0));
        check_and_wait(&mut app, kit);

        assert!(app.model.kits[0].disk.notices.is_empty());
        assert_eq!(real_of(&app.model.kits[0].parsed_tags[&key].tag, DISTANCE), Some(4.0));
    }

    #[test]
    fn close_tab_closes_a_missing_tag() {
        let (_fixture, mut app, key, _) = opened_then_deleted("disk-missing-close");

        choose(&mut app, &key, DiskChoice::CloseTab);

        let kit = &app.model.kits[0];
        assert!(!kit.open_tabs.contains(&key));
        assert!(kit.disk.notices.is_empty());
    }

    /// Unsaved edits on a missing tag are the only copy left: closing asks
    /// first, and the banner stays while it does.
    #[test]
    fn close_tab_on_an_edited_missing_tag_asks_first() {
        let (_fixture, mut app, key, _) = opened_then_deleted("disk-missing-close-edited");
        edit_field(&mut app, &key, DISTANCE, "3");

        choose(&mut app, &key, DiskChoice::CloseTab);

        assert!(app.dialogs.get::<SaveChangesPrompt>().is_some());
        assert!(app.model.kits[0].disk.missing_entry(&key).is_some());
    }
}
