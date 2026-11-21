//! Noticing that an open tag's file changed on disk outside Baboon: Explorer,
//! Tool's bitmap monitoring, another editor. Each open document remembers its
//! file as it was read or saved ([`DiskStamp`]); when Baboon's window regains
//! focus the open loose tags are checked against it off the UI thread, and a
//! tag with no unsaved edits is read again.

use super::*;

/// One open tag's file, checked against the stamp it had when the check began.
pub(in crate::app) struct OpenTagCheck {
    pub(in crate::app) key: String,
    pub(in crate::app) checked: DiskStamp,
    pub(in crate::app) result: DiskCheck,
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
                    let TagEntryLocation::LooseFile(path) = &source.entry_for_key(key)?.location else {
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
            let Some(doc) = kit.parsed_tags.get_mut(&key) else {
                continue;
            };
            // Saved, reloaded or closed and reopened while the check ran:
            // the answer is about a file the document no longer stands for.
            if doc.disk != Some(checked) {
                continue;
            }
            match result {
                DiskCheck::Same | DiskCheck::Unreadable(_) => {}
                DiskCheck::Restamped(stamp) => doc.disk = Some(stamp),
                DiskCheck::Changed if !doc.dirty.is_set() => {
                    if kit.loading_tags.contains(&key) {
                        continue;
                    }
                    kit.disk_reloads.insert(key.clone());
                    self.load_tag_in(index, key, ctx.clone(), true);
                }
                // A tag with unsaved edits, or whose file is gone, is left
                // as it is for now: asking the user about it comes next.
                DiskCheck::Changed | DiskCheck::Missing => {}
            }
        }
        false
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
            kit.loading_tags.is_empty() && kit.disk_reloads.is_empty()
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

    /// Unsaved edits are never replaced silently; the banner that asks about
    /// them is the next step.
    #[test]
    fn a_tag_with_unsaved_edits_is_not_reloaded() {
        let fixture = LooseKit::new("disk-dirty", "halo3_mcc");
        let path = fixture.write_mcc("warthog", "model", |_| {});
        let mut app = app();
        fixture.install(&mut app);
        let key = fixture.open(&mut app, "warthog.model");
        let kit = app.model.kits[0].id;
        edit_field(&mut app, &key, DISTANCE, "3");

        fixture.write_mcc("warthog", "model", |tag| {
            set_distance(tag, 7.5);
        });
        bump_modified(&path);
        check_and_wait(&mut app, kit);

        let doc = &app.model.kits[0].parsed_tags[&key];
        assert!(doc.dirty.is_set());
        assert_eq!(real_of(&doc.tag, DISTANCE), Some(3.0));
    }
}
