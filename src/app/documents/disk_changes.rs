//! Noticing that an open tag's file changed on disk outside Baboon: Explorer,
//! Tool's bitmap monitoring, another editor. Each open document remembers its
//! file as it was read or saved ([`DiskStamp`]); when Baboon's window regains
//! focus the open loose tags are checked against it off the UI thread. A tag
//! with no unsaved edits is read again; one with edits gets a banner asking
//! whether to reload it or keep the edits. A tag whose file is gone stays
//! open, marked missing, until it is saved back or closed.
//!
//! A loose kit also watches its folder ([`FolderWatch`]), so a change is
//! noticed moments after it lands rather than at the next focus change; the
//! focus check stays as the fallback for a watch that could not start.
//! Windows can stop a watch after an error without saying so, so a watch is
//! only trusted while it is heard from ([`TRUSTED_WATCH`]), and one that
//! missed a change the timed browser refresh found is started again.

use super::*;
use crate::core::source::reads::file_key;

/// How long a folder watch is trusted after it last reported anything. A
/// quieter one may have stopped, so the browser goes back to the 30-second
/// refresh and a focus change rechecks the previews too.
pub(in crate::app) const TRUSTED_WATCH: std::time::Duration = std::time::Duration::from_secs(300);

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
    /// Keep My Edits, then the Save that found the file changed.
    KeepAndSave,
}

/// One kit's open tags whose files changed: reads in flight and banners
/// waiting for an answer, by key.
#[derive(Default)]
pub(in crate::app) struct DiskWatch {
    pub(in crate::app) reloads: HashMap<String, DiskReload>,
    pub(in crate::app) notices: HashMap<String, DiskNotice>,
    /// The OS watch on a loose kit's folder, if it started.
    pub(in crate::app) folder: Option<FolderWatch>,
    /// A check of the open tags was put off while Baboon moved files: run
    /// it once the move is done, or changes made meanwhile go unseen.
    pub(in crate::app) check_due: bool,
    /// Files Baboon itself just wrote, by [`file_key`], as it wrote them.
    /// The burst they cause is not a change: the save already updated the
    /// browser, the tag and its previews.
    pub(in crate::app) own_writes: HashMap<String, DiskStamp>,
}

impl DiskWatch {
    /// Whether the folder watch has been heard from lately, so it can be
    /// relied on to report changes.
    pub(in crate::app) fn trusted(&self) -> bool {
        self.folder.as_ref().is_some_and(|folder| folder.heard_within(TRUSTED_WATCH))
    }

    /// Baboon just wrote `path`: the watch's report of it is not a change.
    pub(in crate::app) fn note_own_write(&mut self, path: &Path) {
        if self.folder.is_some()
            && let Ok(stamp) = DiskStamp::of_file(path)
        {
            self.own_writes.insert(file_key(path), stamp);
        }
    }

    /// Whether `path` in a burst is only Baboon's own write: the file as
    /// Baboon left it, the temporary file a save writes first, or a folder
    /// holding one (Windows reports the folder as modified too).
    fn own_write(&self, path: &Path) -> bool {
        let saving = format!(".blam-save-{}-", std::process::id());
        if path.file_name().is_some_and(|name| name.to_string_lossy().contains(&saving)) {
            return true;
        }
        let key = file_key(path);
        if let Some(stamp) = self.own_writes.get(&key) {
            return stamp.check(path) == DiskCheck::Same;
        }
        path.is_dir() && self.own_writes.keys().any(|written| written.starts_with(&format!("{key}{}", std::path::MAIN_SEPARATOR)))
    }

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
            // A commit, pull or checkout made in another app changes Git's
            // records without touching the tags the watch sees.
            for index in 0..self.model.kits.len() {
                self.git_review_refresh_due(index);
                // Without a watch it trusts, nothing named what changed
                // meanwhile.
                if !self.model.kits[index].disk.trusted() {
                    self.reload_previews_reading(index, &Changed::Anything, ctx);
                }
            }
        }
    }

    /// Once a burst of changes under a kit's folder has gone quiet, check its
    /// open tags and refresh its browser.
    pub(in crate::app) fn check_open_tags_on_watch(&mut self, ctx: &egui::Context) {
        for index in 0..self.model.kits.len() {
            let Some(burst) = self.model.kits[index]
                .disk
                .folder
                .as_ref()
                .and_then(|folder| folder.settled(WATCH_QUIET))
            else {
                if self.model.kits[index].disk.check_due && self.tag_ops.folder_refactor.is_none() {
                    self.check_open_tags_in(index, ctx);
                }
                continue;
            };
            // A commit's own records aside, Git sees a save as a change.
            self.git_review_refresh_due(index);
            let disk = &mut self.model.kits[index].disk;
            let paths: Vec<&Path> =
                burst.paths.iter().map(PathBuf::as_path).filter(|path| !disk.own_write(path)).collect();
            disk.own_writes.clear();
            if paths.is_empty() && !burst.incomplete {
                continue;
            }
            self.check_open_tags_in(index, ctx);
            self.refresh_entry_index_soon(index, ctx);
            let changed = if burst.incomplete { Changed::Anything } else { Changed::Files(&paths) };
            self.reload_previews_reading(index, &changed, ctx);
        }
    }

    /// The timed browser refresh found a change on disk that the folder
    /// watch never reported, so the watch has stopped (Windows stops one
    /// after an error without saying so). Start it again, and check what it
    /// may have missed.
    pub(in crate::app) fn restart_silent_folder_watch(&mut self, index: usize, ctx: &egui::Context) {
        self.start_folder_watch(index);
        self.check_open_tags_in(index, ctx);
        self.git_review_refresh_due(index);
        self.reload_previews_reading(index, &Changed::Anything, ctx);
    }

    /// Start watching a newly loaded loose kit's folder. Without a watch
    /// (the OS refused one) the focus check still catches changes.
    pub(in crate::app) fn start_folder_watch(&mut self, index: usize) {
        let kit = &mut self.model.kits[index];
        let Some(TagSource::LooseFolder { root, .. }) = kit.source.as_ref().map(|source| &source.source) else {
            return;
        };
        let ctx = self.egui_ctx.clone();
        kit.disk.folder = FolderWatch::start(root, move |after| ctx.request_repaint_after(after)).ok();
    }

    /// Start a check of every open loose tag, in every kit, against its file.
    pub(in crate::app) fn check_open_tags_on_disk(&mut self, ctx: &egui::Context) {
        for index in 0..self.model.kits.len() {
            self.check_open_tags_in(index, ctx);
        }
    }

    /// Start a check of one kit's open loose tags (tabs and held documents)
    /// against their files.
    fn check_open_tags_in(&mut self, index: usize, ctx: &egui::Context) {
        // Baboon's own move is renaming files under open tabs; it reloads
        // them itself when it ends, and the check runs after.
        self.model.kits[index].disk.check_due = self.tag_ops.folder_refactor.is_some();
        if self.model.kits[index].disk.check_due {
            return;
        }
        let kit = &self.model.kits[index];
        let Some(source) = kit.source.as_ref() else {
            return;
        };
        if !matches!(source.source, TagSource::LooseFolder { .. }) {
            return;
        }
        // Held documents (a Material Editor's shaders) are checked as tabs are.
        let files: Vec<(String, PathBuf, DiskStamp)> = kit
            .open_tabs
            .iter()
            .chain(kit.held_documents.keys().filter(|key| !kit.open_tabs.contains(key)))
            .filter_map(|key| {
                let stamp = kit.parsed_tags.get(key)?.disk?;
                let TagEntryLocation::LooseFile(path) = &kit.entry_for_key(key)?.location else {
                    return None;
                };
                Some((key.clone(), path.clone(), stamp))
            })
            .collect();
        if files.is_empty() {
            return;
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
        // A move that started while the check ran: a file it already moved
        // is not missing, and its tab is about to follow it. The check runs
        // again once it ends.
        if self.tag_ops.folder_refactor.is_some() {
            self.model.kits[index].disk.check_due = true;
            return false;
        }
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
        if choice == DiskChoice::KeepAndSave {
            self.apply_disk_choice(kit, key.clone(), DiskChoice::KeepMyEdits, ctx);
            // Save addresses the active kit's documents.
            self.focus_kit(index);
            match self.save_tag_by_key(&key) {
                Ok(path) => {
                    self.model.status = format!("Saved {}", path.display());
                    // The save the save-changes prompt (closing a tab,
                    // quitting) stopped on: carry on with it.
                    self.resume_save_changes_prompt(ctx);
                }
                Err(error) => self.model.status = format!("Save failed: {error}"),
            }
            return;
        }
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
    /// The folder watch notices a change with no focus change and no
    /// explicit check: only the frame's settled-burst poll runs.
    #[test]
    fn a_watched_change_reloads_the_tag_without_a_focus_change() {
        let fixture = LooseKit::new("disk-watch", "halo3_mcc");
        let path = fixture.write_mcc("warthog", "model", |_| {});
        let mut app = app();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "warthog.model");
        assert!(app.model.kits[0].disk.folder.is_some(), "a loose kit watches its folder");

        fixture.write_mcc("warthog", "model", |tag| set_distance(tag, 7.5));
        bump_modified(&path);
        let ctx = ctx();
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            app.check_open_tags_on_watch(&ctx);
            if let Ok(message) = app.rx.recv_timeout(Duration::from_millis(50)) {
                app.apply_worker_message(message, &ctx);
                app.process_worker_messages(&ctx);
            }
            let doc = &app.model.kits[0].parsed_tags[&key];
            if real_of(&doc.tag, DISTANCE) == Some(7.5) {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "timed out; status: {}", app.model.status);
        }
        assert_eq!(app.model.status, "Reloaded warthog.model: changed on disk");
    }

    /// A check that came while Baboon was moving files waits for the move,
    /// then runs: a change made meanwhile is still noticed.
    #[test]
    fn a_check_put_off_by_a_move_runs_once_the_move_ends() {
        let fixture = LooseKit::new("disk-after-move", "halo3_mcc");
        let path = fixture.write_mcc("warthog", "model", |_| {});
        let mut app = app();
        fixture.install(&mut app);
        // No watch: only the put-off check can notice.
        app.model.kits[0].disk.folder = None;
        let key = fixture.open(&mut app, "warthog.model");
        app.tag_ops.folder_refactor = Some(crate::app::shell::worker::FolderRefactorUiState {
            label: "Moving".to_owned(),
            phase: "Moving files".to_owned(),
            progress: None,
        });
        fixture.write_mcc("warthog", "model", |tag| set_distance(tag, 7.5));
        bump_modified(&path);
        let ctx = ctx();
        app.check_open_tags_on_disk(&ctx);
        app.check_open_tags_on_watch(&ctx);
        assert!(app.rx.recv_timeout(Duration::from_millis(200)).is_err(), "no check during the move");
        assert!(app.model.kits[0].disk.check_due);

        app.tag_ops.folder_refactor = None;
        app.check_open_tags_on_watch(&ctx);
        assert!(!app.model.kits[0].disk.check_due);
        pump_until(&mut app, "the reload", |app| {
            real_of(&app.model.kits[0].parsed_tags[&key].tag, DISTANCE) == Some(7.5)
        });
    }

    /// A tag made outside Baboon reaches the browser through the folder
    /// watch, with the timed refresh never due.
    #[test]
    fn a_watched_new_tag_appears_in_the_browser_without_the_timed_refresh() {
        let fixture = LooseKit::new("disk-watch-index", "halo3_mcc");
        fixture.write_mcc("vehicles\\warthog", "model", |_| {});
        let mut app = app();
        fixture.install_indexed(&mut app);
        app.model.kits[0].index_jobs.next_refresh_at = f64::INFINITY;

        fixture.write_mcc("vehicles\\ghost", "model", |_| {});
        let key = fixture.key("vehicles\\ghost.model");
        let ctx = ctx();
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let listed = |app: &Baboon| {
            app.model.kits[0].source.as_ref().unwrap().all_entries.iter().any(|entry| entry.key == key)
        };
        while !listed(&app) {
            assert!(std::time::Instant::now() < deadline, "timed out; status: {}", app.model.status);
            app.check_open_tags_on_watch(&ctx);
            app.maybe_refresh_entry_index(ctx.clone());
            if let Ok(message) = app.rx.recv_timeout(Duration::from_millis(50)) {
                app.apply_worker_message(message, &ctx);
                app.process_worker_messages(&ctx);
            }
        }
        // A later burst may already have asked for another refresh; the
        // timed one, scheduled now, is the slow backstop.
        app.model.kits[0].index_jobs.refresh_again = false;
        app.schedule_next_entry_index_refresh(0, &ctx);
        let next = app.model.kits[0].index_jobs.next_refresh_at;
        assert!(next >= 300.0, "the timed refresh becomes a slow backstop, due at {next}");
    }

    /// A change the watch reports refreshes the browser at once, unless a
    /// refresh only just ended: then it waits for the gap.
    #[test]
    fn watched_refreshes_leave_a_gap_after_the_last_one() {
        let fixture = LooseKit::new("disk-watch-gap", "halo3_mcc");
        fixture.write_mcc("warthog", "model", |_| {});
        let mut app = app();
        fixture.install(&mut app);
        let ctx = ctx();
        let now = ctx.input(|input| input.time);

        app.model.kits[0].index_jobs.last_refresh_ended = None;
        app.model.kits[0].index_jobs.next_refresh_at = f64::INFINITY;
        app.refresh_entry_index_soon(0, &ctx);
        assert_eq!(app.model.kits[0].index_jobs.next_refresh_at, now, "nothing ran lately: at once");

        app.model.kits[0].index_jobs.last_refresh_ended = Some(std::time::Instant::now());
        app.model.kits[0].index_jobs.next_refresh_at = f64::INFINITY;
        app.refresh_entry_index_soon(0, &ctx);
        let wait = app.model.kits[0].index_jobs.next_refresh_at - now;
        assert!((4.0..=5.0).contains(&wait), "waits out the gap, {wait} s");
    }

    /// The watch reports Baboon's own save like any write, but the save has
    /// already updated the browser and the tag: no rescan, no check.
    #[test]
    fn baboons_own_save_does_not_rescan_the_browser() {
        let fixture = LooseKit::new("disk-watch-own-save", "halo3_mcc");
        fixture.write_mcc("vehicles\\warthog", "model", |_| {});
        let mut app = app();
        fixture.install_indexed(&mut app);
        let key = fixture.open(&mut app, "vehicles\\warthog.model");
        edit_field(&mut app, &key, DISTANCE, "3");
        app.model.kits[0].index_jobs.next_refresh_at = f64::INFINITY;
        app.save_tag_by_key(&key).expect("save");
        assert!(!app.model.kits[0].disk.own_writes.is_empty(), "the save is noted");

        let ctx = ctx();
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !app.model.kits[0].disk.own_writes.is_empty() {
            assert!(std::time::Instant::now() < deadline, "timed out waiting for the save's burst");
            app.check_open_tags_on_watch(&ctx);
            std::thread::sleep(Duration::from_millis(20));
        }

        assert_eq!(app.model.kits[0].index_jobs.next_refresh_at, f64::INFINITY, "no rescan");
        assert!(app.rx.try_recv().is_err(), "and no check of the open tags");
        assert!(app.model.kits[0].disk.trusted(), "the save shows the watch is running");
    }

    /// A render model open with its Model Preview shown, which (as a real
    /// load's material jobs would) read the shader `objects/rock/shaders/rock`.
    /// Beside them, an unrelated shader. The real loads' reads are covered by
    /// `model_preview_reads_note_a_real_models_shaders_and_bitmaps`.
    fn previewed_rock(name: &str) -> (LooseKit, Baboon, String, PathBuf) {
        use crate::app::model_preview::state::{ModelPreviewState, ModelTagPanelTab};
        let fixture = LooseKit::new(name, "halo3_mcc");
        fixture.write_mcc("objects/rock/rock", "render_model", |_| {});
        let shader = fixture.write_mcc("objects/rock/shaders/rock", "shader", |_| {});
        fixture.write_mcc("objects/tree/shaders/bark", "shader", |_| {});
        let mut app = app();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "objects/rock/rock.render_model");
        let state = ModelPreviewState {
            active_tab: ModelTagPanelTab::ModelPreview,
            ..ModelPreviewState::loaded_empty_for_test(&key)
        };
        {
            let _recording = crate::core::source::reads::record_reads(&state.reads);
            let source = app.model.kits[0].source.as_ref().unwrap().source.clone();
            let entry = app.model.entry_for_key(&fixture.key("objects/rock/shaders/rock.shader")).unwrap().clone();
            crate::core::source::read_entry(&source, &entry).expect("read the shader");
        }
        let id = app.model.kits[0].id;
        app.views[id].caches.model_previews.insert(key.clone(), state);
        (fixture, app, key, shader)
    }

    fn preview<'a>(app: &'a Baboon, key: &str) -> &'a crate::app::model_preview::state::ModelPreviewState {
        &app.views[app.model.kits[0].id].caches.model_previews[key]
    }

    fn save_edited(app: &mut Baboon, fixture: &LooseKit, rel: &str) {
        let key = fixture.open(app, rel);
        app.model.kits[0].parsed_tags.get_mut(&key).unwrap().dirty.touch();
        app.save_tag_by_key(&key).expect("save");
    }

    /// Saving a shader in Baboon marks an open preview drawn with it out of
    /// date, which loads it again; saving a shader it never read does not.
    #[test]
    fn saving_a_shader_reloads_the_model_preview_using_it() {
        let (fixture, mut app, key, shader) = previewed_rock("preview-shader-save");
        assert!(preview(&app, &key).reads.contains(&shader));
        assert!(!preview(&app, &key).needs_preview_load(&key));

        save_edited(&mut app, &fixture, "objects/tree/shaders/bark.shader");
        assert!(!preview(&app, &key).reads_changed, "the bark shader is not in the preview");

        save_edited(&mut app, &fixture, "objects/rock/shaders/rock.shader");
        let state = preview(&app, &key);
        assert!(state.reads_changed, "the preview is out of date");
        assert!(state.needs_preview_load(&key), "so the pane loads it again");
        assert!(state.data.is_some(), "keeping the page up meanwhile");
    }

    /// Saving the previewed tag itself is not a reason to reload: the preview
    /// already draws its document, edits and all.
    #[test]
    fn saving_the_previewed_tag_itself_does_not_reload_it() {
        let (fixture, mut app, key, _) = previewed_rock("preview-own-save");
        {
            let state = preview(&app, &key);
            let _recording = crate::core::source::reads::record_reads(&state.reads);
            let source = app.model.kits[0].source.as_ref().unwrap().source.clone();
            crate::core::source::read_entry(&source, app.model.entry_for_key(&key).unwrap()).unwrap();
        }
        save_edited(&mut app, &fixture, "objects/rock/rock.render_model");
        assert!(!preview(&app, &key).reads_changed);
    }

    /// With Show Unsaved Edits on, editing a shader the preview drew from
    /// its file loads the preview again once the edits pause; each edit
    /// before then starts the pause over.
    #[test]
    fn with_unsaved_edits_shown_an_edited_shader_reloads_once_edits_pause() {
        let (fixture, mut app, key, _) = previewed_rock("preview-unsaved-edit");
        let id = app.model.kits[0].id;
        app.views[id].caches.model_previews.get_mut(&key).unwrap().show_unsaved_edits = true;
        let rock = fixture.open(&mut app, "objects/rock/shaders/rock.shader");
        let ctx = ctx();
        app.check_previews_for_unsaved_edits_at(10.0, &ctx);
        assert!(preview(&app, &key).edits_seen.is_none(), "nothing edited yet");
        assert!(preview(&app, &key).edits_checked.is_some(), "not compared again until something changes");

        app.model.kits[0].parsed_tags.get_mut(&rock).unwrap().dirty.touch();
        app.check_previews_for_unsaved_edits_at(10.0, &ctx);
        app.check_previews_for_unsaved_edits_at(10.2, &ctx);
        app.model.kits[0].parsed_tags.get_mut(&rock).unwrap().dirty.touch();
        app.check_previews_for_unsaved_edits_at(10.25, &ctx);
        app.check_previews_for_unsaved_edits_at(10.5, &ctx);
        assert!(!preview(&app, &key).reads_changed, "still editing");
        app.check_previews_for_unsaved_edits_at(10.6, &ctx);
        assert!(preview(&app, &key).reads_changed, "the edits paused: load again");
    }

    /// With the option off, the preview keeps drawing the saved shader.
    #[test]
    fn with_unsaved_edits_hidden_an_edited_shader_changes_nothing() {
        let (fixture, mut app, key, _) = previewed_rock("preview-unsaved-off");
        let rock = fixture.open(&mut app, "objects/rock/shaders/rock.shader");
        app.model.kits[0].parsed_tags.get_mut(&rock).unwrap().dirty.touch();
        let ctx = ctx();
        app.check_previews_for_unsaved_edits_at(10.0, &ctx);
        app.check_previews_for_unsaved_edits_at(20.0, &ctx);
        assert!(!preview(&app, &key).reads_changed);
    }

    /// A preview drawn from a shader's unsaved edits loads again from the
    /// file when those edits are discarded by closing the shader, but not
    /// when they are saved (the file then holds what it drew).
    #[test]
    fn discarding_edits_a_preview_shows_reloads_it_from_the_file() {
        use crate::core::source::reads::{Edited, ReadFiles, file_key, record_reads};
        let (fixture, mut app, key, shader) = previewed_rock("preview-unsaved-discard");
        let rock = fixture.open(&mut app, "objects/rock/shaders/rock.shader");
        app.model.kits[0].parsed_tags.get_mut(&rock).unwrap().dirty.touch();
        // Drawn from the shader's edits, as a load with the option on reads it.
        let stamp = app.model.kits[0].parsed_tags[&rock].content_stamp();
        let bytes = std::fs::read(&shader).unwrap();
        let reads = ReadFiles::with_edits(HashMap::from([(file_key(&shader), Edited { stamp, bytes: Some(bytes) })]));
        {
            let _recording = record_reads(&reads);
            let source = app.model.kits[0].source.as_ref().unwrap().source.clone();
            crate::core::source::read_entry(&source, app.model.entry_for_key(&rock).unwrap()).unwrap();
        }
        let id = app.model.kits[0].id;
        let state = app.views[id].caches.model_previews.get_mut(&key).unwrap();
        state.reads = reads;
        state.show_unsaved_edits = true;
        let ctx = ctx();

        app.save_tag_by_key(&rock).expect("save the shader");
        app.check_previews_for_unsaved_edits_at(10.0, &ctx);
        app.check_previews_for_unsaved_edits_at(20.0, &ctx);
        assert!(!preview(&app, &key).reads_changed, "saved: the file holds what it drew");

        app.model.kits[0].parsed_tags.get_mut(&rock).unwrap().dirty.touch();
        let stamp = app.model.kits[0].parsed_tags[&rock].content_stamp();
        let reads = ReadFiles::with_edits(HashMap::from([(file_key(&shader), Edited { stamp, bytes: Some(Vec::new()) })]));
        {
            let _recording = record_reads(&reads);
            crate::core::source::reads::note_read(&shader);
        }
        app.views[id].caches.model_previews.get_mut(&key).unwrap().reads = reads;
        app.kit_and_view(0).drop_document(&rock);
        app.check_previews_for_unsaved_edits_at(30.0, &ctx);
        app.check_previews_for_unsaved_edits_at(30.5, &ctx);
        assert!(preview(&app, &key).reads_changed, "discarded: draw the file again");
    }

    /// A shader rewritten outside Baboon reaches the preview through the
    /// folder watch.
    #[test]
    fn a_shader_changed_outside_baboon_reloads_the_model_preview() {
        let (fixture, mut app, key, shader) = previewed_rock("preview-shader-watch");
        fixture.write_mcc("objects/rock/shaders/rock", "shader", |_| {});
        bump_modified(&shader);
        let ctx = ctx();
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !preview(&app, &key).reads_changed {
            assert!(std::time::Instant::now() < deadline, "the watch never reached the preview");
            app.check_open_tags_on_watch(&ctx);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

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

    /// Saving with the banner up does not answer it: the file is not
    /// written over, and a window asks the user to choose.
    #[test]
    fn saving_with_the_banner_up_asks_first() {
        let (_fixture, mut app, key, path) = edited_then_changed_on_disk("disk-save-banner");

        let refused = app.save_tag_by_key(&key).expect_err("the save waits for an answer");

        assert!(refused.contains("changed on disk since it was opened"), "{refused}");
        assert!(app.model.kits[0].disk.notices.contains_key(&key));
        assert!(app.model.kits[0].parsed_tags[&key].dirty.is_set());
        let on_disk = TagFile::read(&path).unwrap();
        assert_eq!(real_of(&on_disk, DISTANCE), Some(7.5), "the file was left alone");
        let window = app.dialogs.get::<DiskSaveConfirm>().expect("the window asks");
        assert_eq!(window.key, key);
    }

    /// The window's Keep My Edits and Save: the save the user asked for
    /// goes through, over the changed file.
    #[test]
    fn keep_my_edits_and_save_writes_over_the_file() {
        let (_fixture, mut app, key, path) = edited_then_changed_on_disk("disk-keep-and-save");
        assert!(app.save_tag_by_key(&key).is_err());

        choose(&mut app, &key, DiskChoice::KeepAndSave);

        assert_eq!(real_of(&TagFile::read(&path).unwrap(), DISTANCE), Some(3.0));
        let kit = &app.model.kits[0];
        assert!(kit.disk.notices.is_empty());
        assert!(!kit.parsed_tags[&key].dirty.is_set());
        assert!(app.model.status.starts_with("Saved "), "{}", app.model.status);
    }

    /// The save backstop: a file changed while Baboon kept the focus, so no
    /// check saw it, still stops the save and raises the banner.
    #[test]
    fn save_notices_a_change_no_check_saw() {
        let fixture = LooseKit::new("disk-save-backstop", "halo3_mcc");
        let path = fixture.write_mcc("warthog", "model", |_| {});
        let mut app = app();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "warthog.model");
        edit_field(&mut app, &key, DISTANCE, "3");
        fixture.write_mcc("warthog", "model", |tag| set_distance(tag, 7.5));
        bump_modified(&path);

        assert!(app.save_tag_by_key(&key).is_err());

        assert!(matches!(app.model.kits[0].disk.notices.get(&key), Some(DiskNotice::Changed(_))));
        assert_eq!(real_of(&TagFile::read(&path).unwrap(), DISTANCE), Some(7.5));
    }

    /// Keep My Edits is trusted: the next Save writes over the file.
    #[test]
    fn save_after_keep_my_edits_writes_over_the_file() {
        let (_fixture, mut app, key, path) = edited_then_changed_on_disk("disk-save-kept");
        choose(&mut app, &key, DiskChoice::KeepMyEdits);

        app.save_tag_by_key(&key).expect("save");

        assert_eq!(real_of(&TagFile::read(&path).unwrap(), DISTANCE), Some(3.0));
        assert!(app.model.kits[0].disk.notices.is_empty());
        assert!(!app.model.kits[0].parsed_tags[&key].dirty.is_set());
    }

    /// A file deleted while Baboon kept the focus: Save puts it back, folder
    /// and all, without a check having marked it missing first.
    #[test]
    fn save_recreates_a_file_no_check_saw_go() {
        let fixture = LooseKit::new("disk-save-gone", "halo3_mcc");
        let path = fixture.write_mcc("vehicles\\warthog", "model", |_| {});
        fixture.write_mcc("scenery\\rock", "model", |_| {});
        let mut app = app();
        fixture.install_indexed(&mut app);
        let key = fixture.open(&mut app, "vehicles\\warthog.model");
        fs::remove_dir_all(path.parent().unwrap()).unwrap();

        app.save_tag_by_key(&key).expect("save");

        assert!(path.is_file());
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
