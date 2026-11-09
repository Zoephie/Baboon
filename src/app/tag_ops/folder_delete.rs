//! Confirm and recoverably delete a loose folder, without rebuilding the tag set.
use super::*;
use super::folder_rename::outside_referrer_keys;

pub(in crate::app) struct DeleteFolderConfirm {
    kit: KitId,
    root: PathBuf,
    relative: PathBuf,
    entries: Vec<TagEntry>,
    outside_referrers: Option<Vec<String>>,
}

pub(in crate::app) struct DeletedFolder {
    relative: PathBuf,
    keys: HashSet<String>,
    destination: PathBuf,
    warning: Option<String>,
}

/// Resolve only a real descendant directory; never the tags root, a parent,
/// or a directory reached through a symlink/junction.
fn checked_folder(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.as_os_str().is_empty() || !relative.components()
        .all(|part| matches!(part, std::path::Component::Normal(_))) {
        return Err("Choose a folder inside the tags directory; the tags root cannot be deleted".into());
    }
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let mut folder = root.to_path_buf();
    for component in relative.components() {
        folder.push(component);
        if fs::symlink_metadata(&folder).map_err(|error| error.to_string())?.file_type().is_symlink() {
            return Err("Folders reached through links cannot be deleted".into());
        }
    }
    let resolved = folder.canonicalize().map_err(|error| error.to_string())?;
    if resolved == canonical_root || !resolved.starts_with(&canonical_root) || !folder.is_dir() {
        return Err("Choose a real folder inside the tags directory".into());
    }
    Ok(folder)
}

fn referrers(model: &Model, confirm: &DeleteFolderConfirm) -> Option<Vec<String>> {
    if confirm.entries.is_empty() { return Some(Vec::new()); }
    let kit = model.kit_index(confirm.kit)?;
    let source = model.kits[kit].source.as_ref()?;
    let keys = outside_referrer_keys(&confirm.entries, source.reverse_dependencies.as_ref()?, &source.names);
    let mut paths: Vec<_> = source.full_entry_set().iter().filter(|entry| keys.contains(&entry.key))
        .map(|entry| entry.display_path.clone()).collect();
    paths.sort_by_cached_key(|path| crate::core::source::natural_key(path));
    Some(paths)
}

impl Dialog for DeleteFolderConfirm {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let Some(index) = cx.model.kit_index(self.kit) else { return false; };
        if cx.model.loaded_tags_root_for(index).as_ref() != Some(&self.root) { return false; }
        // A modal blocks editing. Cache a completed preview instead of
        // repeatedly walking a large folder's references during every repaint.
        if self.outside_referrers.is_none() { self.outside_referrers = referrers(cx.model, self); }
        let outside = &self.outside_referrers;
        let unsaved = self.entries.iter().filter(|entry| cx.model.kits[index].parsed_tags.get(&entry.key)
            .is_some_and(|document| document.dirty.is_set())).count();
        let mut delete = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("delete_folder_confirm"))
            .frame(egui::Frame::window(&cx.egui.global_style()))
            .show(cx.egui, |ui| {
                ui.set_width(window_width(cx.egui, 520.0));
                crate::app::search::draw_icon_window_header_without_close(ui, "Delete Folder?", ButtonIcon::Garbage);
                ui.add_space(12.0);
                ui.label(RichText::new(tag_path::relative_tag_path(&self.relative)).monospace());
                ui.label(format!("This folder contains {} tag(s). The entire folder, including subfolders and other files, will be moved to Baboon's deleted-tags folder so it can be recovered by hand.", self.entries.len()));
                if unsaved > 0 {
                    ui.add_space(8.0);
                    ui.label(format!("{unsaved} tag(s) have unsaved edits that will be discarded."));
                }
                ui.add_space(8.0);
                match outside {
                    Some(paths) => {
                        ui.label(format!("{} tag(s) outside this folder reference its tags.", paths.len()));
                        ui.label(RichText::new("References from tags inside this folder are excluded.").color(subtle_dark()));
                        if !paths.is_empty() {
                            ui.label("These references will be left pointing at missing tags:");
                            crate::app::ScrollArea::vertical().id_salt("delete_folder_referrers")
                                .max_height(160.0).show(ui, |ui| {
                                    for path in paths.iter().take(100) { ui.label(RichText::new(path).monospace()); }
                                    if paths.len() > 100 { ui.label(format!("And {} more…", paths.len() - 100)); }
                                });
                        }
                    }
                    None if cx.model.kits[index].index_jobs.building_references || cx.model.kits[index].scanning_entries => {
                        ui.horizontal(|ui| { ui.spinner(); ui.label("Checking references before deletion…"); });
                    }
                    None => { ui.label("The reference count is unavailable. Build the reference index before deleting this folder."); }
                }
                ui.add_space(12.0);
                popup_footer(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        cancel = icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked();
                        delete = icon_text_button(ui, ButtonIcon::Garbage, "Delete Folder", outside.is_some()).clicked();
                    });
                });
            });
        if delete { cx.send(TagOpsCommand::DeleteFolder); }
        !cancel && !response.should_close()
    }
}

impl Baboon {
    pub(in crate::app) fn open_delete_folder(&mut self, relative: PathBuf, ctx: egui::Context) {
        if self.refuse_read_only_edit(self.model.active) { return; }
        let result = (|| {
            if self.tag_ops.folder_refactor.is_some() { return Err("A folder operation is already running".to_owned()); }
            let root = self.model.loaded_tags_root().ok_or("Deletion requires a loose tags folder")?;
            let folder = checked_folder(&root, &relative)?;
            let source = self.model.source().ok_or("The source is no longer loaded")?;
            let entries = if source.complete_scan {
                source.full_entry_set().iter().filter(|entry| matches!(&entry.location,
                    TagEntryLocation::LooseFile(path) if path.starts_with(&folder))).cloned().collect()
            } else {
                scan_folder_subtree_entries(&root, &relative, &source.names).map_err(|error| error.to_string())?
            };
            Ok(DeleteFolderConfirm { kit: self.model.active_kit_id(), root, relative, entries, outside_referrers: None })
        })();
        match result {
            Ok(confirm) => {
                let needs_index = referrers(&self.model, &confirm).is_none();
                self.dialogs.open(confirm);
                if needs_index { self.begin_build_reverse_dependencies(ctx, false); }
            }
            Err(error) => self.model.status = error,
        }
    }

    pub(in crate::app) fn begin_delete_folder(&mut self, ctx: egui::Context) {
        let Some(confirm) = self.dialogs.close::<DeleteFolderConfirm>() else { return; };
        if !self.focus_navigation_kit(confirm.kit) || self.refuse_read_only_edit(self.model.active) { return; }
        if self.model.loaded_tags_root().as_ref() != Some(&confirm.root) {
            self.model.status = "The folder's source has changed; deletion was cancelled".into();
            return;
        }
        if self.tag_ops.folder_refactor.is_some() || referrers(&self.model, &confirm).is_none() {
            self.model.status = "Wait for the folder operation or reference check to finish before deleting".into();
            return;
        }
        let stamp = self.model.kit_stamp();
        let game = self.model.source().and_then(|source| source.game);
        let names = self.model.names().clone();
        let tx = self.tx.clone();
        self.tag_ops.folder_refactor = Some(FolderRefactorUiState {
            label: format!("Deleting {}", tag_path::relative_tag_path(&confirm.relative)),
            phase: "Moving folder to deleted-tags".into(), progress: None,
        });
        self.spawn_job(move || WorkerMessage::FolderDeleteFinished { stamp,
            result: delete_folder(confirm, game, names, &tx),
        }, move |_| WorkerMessage::FolderDeleteFinished { stamp, result: Err("Folder deletion worker crashed".into()) });
        ctx.request_repaint();
    }

    pub(in crate::app) fn handle_folder_delete_finished(&mut self, stamp: KitStamp,
        result: Result<DeletedFolder, String>, ctx: &egui::Context) {
        self.tag_ops.folder_refactor = None;
        let done = match result {
            Ok(done) => done,
            Err(error) => { self.model.status = error; return; }
        };
        if let Some(index) = self.model.resolve_stamp(stamp) {
            let tabs: HashSet<_> = self.model.kits[index].parsed_tags.keys().chain(self.model.kits[index].open_tabs.iter())
                .filter(|key| done.keys.contains(*key)).cloned()
                .chain(self.views[stamp.kit].browser.folder_browsers.iter()
                    .filter(|(_, pane)| pane.rel_path.starts_with(&done.relative)).map(|(key, _)| key.clone())).collect();
            for key in tabs {
                let mut both = self.kit_and_view(index);
                both.close_tag_pane(&key);
                both.drop_document(&key);
            }
            for key in &done.keys { self.forget_campaign_overlay(index, key); }
            let kit = &mut self.model.kits[index];
            if kit.selected_key.as_ref().is_some_and(|key| done.keys.contains(key)) { kit.selected_key = None; }
            for key in &done.keys {
                kit.set_tag_references(key, None);
                kit.keywords.forget_tag(key);
            }
            kit.keywords.save_if_dirty();
            kit.pending_container_folders.retain(|folder| !tag_path::to_relative(folder).starts_with(&done.relative));
            if let Some(source) = kit.source.as_mut() {
                crate::core::source::retain_loaded_folder_entries(&mut source.tree, &mut source.entries, |entry| !done.keys.contains(&entry.key));
                prune_folder(&mut source.tree.children, &done.relative);
                source.all_entries.retain(|entry| !done.keys.contains(&entry.key));
                source.group_tree = crate::core::source::build_group_tree(source.full_entry_set());
            }
            kit.generation = kit.generation.wrapping_add(1);
            kit.field_index.invalidate();
            self.refresh_favorite_entries_for(index);
            if self.browser.reveal_target.as_ref().is_some_and(|target| done.keys.contains(&target.key)) { self.browser.reveal_target = None; }
        }
        self.model.status = format!("Deleted {} ({} tags) — moved to {}{}", tag_path::relative_tag_path(&done.relative),
            done.keys.len(), done.destination.display(), done.warning.map(|warning| format!("; {warning}")).unwrap_or_default());
        ctx.request_repaint();
    }
}

fn prune_folder(children: &mut Vec<crate::core::source::TagTreeNode>, relative: &Path) {
    children.retain(|node| !node.rel_path.starts_with(relative));
    for node in children { prune_folder(&mut node.children, relative); }
}

fn delete_folder(confirm: DeleteFolderConfirm, game: Option<GameId>, names: TagNameIndex,
    tx: &Sender<WorkerMessage>) -> Result<DeletedFolder, String> {
    checked_folder(&confirm.root, &confirm.relative)?;
    let current = scan_folder_subtree_entries(&confirm.root, &confirm.relative, &names).map_err(|error| error.to_string())?;
    let keys: HashSet<_> = current.into_iter().map(|entry| entry.key).collect();
    if keys != confirm.entries.iter().map(|entry| entry.key.clone()).collect() {
        return Err("Folder contents changed. Open Delete Folder again to review the new tag count.".into());
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let destination = super::delete::loose_trash_destination(game.map(GameId::as_str),
        &tag_path::relative_tag_path(&confirm.relative), now.as_secs())?;
    move_folder_to_trash(&confirm.root, &confirm.relative, &destination, tx)?;
    // A failed cache write must not hide the completed deletion from the UI.
    let warning = game.and_then(|game| (|| -> anyhow::Result<()> {
        if let Some(mut writer) = crate::core::source::EntryIndexWriter::open(game.as_str(), &confirm.root)? {
            for key in &keys { writer.delete_with_dependencies(key)?; }
        }
        Ok(())
    })().err().map(|error| format!("Could not update the saved index: {error}")));
    Ok(DeletedFolder { relative: confirm.relative, keys, destination, warning })
}

fn move_folder_to_trash(root: &Path, relative: &Path, destination: &Path,
    tx: &Sender<WorkerMessage>) -> Result<(), String> {
    let folder = checked_folder(root, relative)?;
    if destination.exists() { return Err(format!("{} already exists; no files were moved", destination.display())); }
    // Preflight every descendant before moving anything. In particular a
    // cross-volume copy must never follow a junction outside this folder.
    let canonical = folder.canonicalize().map_err(|error| error.to_string())?;
    for item in walkdir::WalkDir::new(&folder).follow_links(false) {
        let item = item.map_err(|error| error.to_string())?;
        if item.file_type().is_symlink() || !item.path().canonicalize().map_err(|error| error.to_string())?.starts_with(&canonical) {
            return Err("Folders containing links cannot be deleted; no files were moved".into());
        }
    }
    let parent = destination.parent().ok_or("The recovery folder has no parent")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    if parent.canonicalize().map_err(|error| error.to_string())?.starts_with(&canonical) {
        return Err("The recovery folder cannot be inside the folder being deleted".into());
    }
    match fs::rename(&folder, destination) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {},
        Err(error) => return Err(format!("Could not move {} to recovery; no files were removed: {error}", folder.display())),
    }
    // Reserve the destination, then copy everything before removing any source
    // bytes. A failed copy leaves the original intact and the recovery copy.
    fs::create_dir(destination).map_err(|error| error.to_string())?;
    copy_folder_recursive_progress(&folder, destination, "Deleting folder", tx)?;
    checked_folder(root, relative)?;
    fs::remove_dir_all(&folder).map_err(|error| format!(
        "Recovery copy saved at {}, but the original folder could not be fully removed: {error}", destination.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::{app, ctx, LooseKit, group_tag, set_reference};

    fn fixture() -> LooseKit {
        let kit = LooseKit::new("delete-folder", "halo3_mcc");
        let render = group_tag("halo3_mcc", "render_model");
        let collision = group_tag("halo3_mcc", "collision_model");
        kit.write_mcc("doomed/render", "render_model", |_| {});
        kit.write_mcc("doomed/sub/collision", "collision_model", |_| {});
        for path in ["doomed/internal", "outside/user"] {
            kit.write_mcc(path, "model", |tag| {
                set_reference(tag, "render model", render, "doomed\\render");
                set_reference(tag, "collision model", collision, "doomed\\sub\\collision");
            });
        }
        kit
    }

    #[test]
    fn confirmation_counts_distinct_outside_tags_and_cancel_keeps_everything() {
        let kit = fixture();
        let mut app = app();
        kit.install_indexed(&mut app);
        app.open_delete_folder("doomed".into(), ctx());
        let confirm = app.dialogs.get::<DeleteFolderConfirm>().unwrap();
        assert_eq!(confirm.entries.len(), 3);
        assert_eq!(referrers(&app.model, confirm).unwrap(), ["outside\\user.model"]);
        app.dialogs.close::<DeleteFolderConfirm>();
        assert!(kit.path("doomed/render.render_model").exists());
        assert!(kit.path("doomed/internal.model").exists());
        for invalid in ["", ".", "..", "doomed/../outside"] {
            app.open_delete_folder(invalid.into(), ctx());
            assert!(app.dialogs.get::<DeleteFolderConfirm>().is_none(), "{invalid}");
        }
    }

    #[test]
    fn folder_moves_as_a_whole_and_finish_removes_only_its_documents_and_index_rows() {
        let kit = fixture();
        fs::write(kit.root.join("doomed/readme.txt"), b"keep these bytes").unwrap();
        let mut app = app();
        kit.install_indexed(&mut app);
        let internal = kit.open(&mut app, "doomed/internal.model");
        let outside = kit.open(&mut app, "outside/user.model");
        let outside_bytes = fs::read(kit.path("outside/user.model")).unwrap();
        app.open_delete_folder("doomed".into(), ctx());
        let confirm = app.dialogs.close::<DeleteFolderConfirm>().unwrap();
        let keys = confirm.entries.iter().map(|entry| entry.key.clone()).collect();
        let stamp = app.model.kit_stamp();
        let destination = kit.root.parent().unwrap().join("recovery/doomed");
        let (tx, _rx) = mpsc::channel();
        move_folder_to_trash(&kit.root, Path::new("doomed"), &destination, &tx).unwrap();
        assert_eq!(fs::read(destination.join("readme.txt")).unwrap(), b"keep these bytes");
        assert!(destination.join("sub/collision.collision_model").exists());
        assert!(!kit.root.join("doomed").exists());
        app.handle_folder_delete_finished(stamp, Ok(DeletedFolder {
            relative: "doomed".into(), keys, destination, warning: None,
        }), &ctx());
        assert!(!app.model.kits[0].parsed_tags.contains_key(&internal));
        assert!(app.model.kits[0].parsed_tags.contains_key(&outside));
        assert_eq!(fs::read(kit.path("outside/user.model")).unwrap(), outside_bytes);
        let source = app.model.source().unwrap();
        assert_eq!(source.full_entry_set().len(), 1);
        assert!(!source.tree.children.iter().any(|node| node.rel_path == Path::new("doomed")));
        assert!(!source.reverse_dependencies.as_ref().unwrap().contains_tag(&internal));
        assert!(source.reverse_dependencies.as_ref().unwrap().contains_tag(&outside));
    }

    #[test]
    fn a_changed_tag_set_or_occupied_recovery_destination_is_refused() {
        let kit = fixture();
        let mut app = app();
        kit.install_indexed(&mut app);
        app.open_delete_folder("doomed".into(), ctx());
        let confirm = app.dialogs.close::<DeleteFolderConfirm>().unwrap();
        kit.write_mcc("doomed/new", "model", |_| {});
        let (tx, _rx) = mpsc::channel();
        assert!(delete_folder(confirm, Some(GameId::from_id("halo3_mcc").unwrap()), kit.names(), &tx).is_err());
        assert!(kit.root.join("doomed").exists());
        let occupied = kit.root.parent().unwrap().join("occupied");
        fs::create_dir_all(&occupied).unwrap();
        fs::write(occupied.join("important.txt"), b"existing recovery").unwrap();
        assert!(move_folder_to_trash(&kit.root, Path::new("doomed"), &occupied, &tx).is_err());
        assert_eq!(fs::read(occupied.join("important.txt")).unwrap(), b"existing recovery");
        assert!(kit.root.join("doomed").exists());
    }

    #[cfg(windows)]
    #[test]
    fn locked_folder_delete_leaves_every_original_file_intact() {
        use std::os::windows::fs::OpenOptionsExt;
        let kit = fixture();
        let free = kit.root.join("doomed/aaa_free.txt");
        let locked = kit.root.join("doomed/zzz_locked.txt");
        fs::write(&free, b"unlocked").unwrap();
        fs::write(&locked, b"locked").unwrap();
        let _lock = fs::OpenOptions::new().read(true).share_mode(3).open(&locked).unwrap();
        let destination = kit.root.parent().unwrap().join("locked-recovery/doomed");
        let (tx, _rx) = mpsc::channel();
        assert!(move_folder_to_trash(&kit.root, Path::new("doomed"), &destination, &tx).is_err());
        assert_eq!(fs::read(&free).unwrap(), b"unlocked", "failed deletion must leave unlocked siblings intact too");
        assert!(kit.path("doomed/render.render_model").exists());
        assert!(!destination.exists(), "a sharing violation must not start a fallback copy");
    }

    #[test]
    fn confirmed_folder_deletion_runs_the_worker_and_keeps_a_recovery_copy() {
        let kit = fixture();
        let mut app = app();
        kit.install_indexed(&mut app);
        app.open_delete_folder("doomed".into(), ctx());
        app.begin_delete_folder(ctx());
        crate::app::loose_fixture::pump_until(&mut app, "folder delete", |app| app.tag_ops.folder_refactor.is_none());
        assert!(!kit.root.join("doomed").exists(), "{}", app.model.status);
        assert_eq!(app.model.source().unwrap().full_entry_set().len(), 1);
        let destination = PathBuf::from(app.model.status.split(" — moved to ").nth(1).expect("recovery location"));
        assert!(destination.starts_with(crate::core::storage::data_path("deleted-tags")));
        assert!(destination.join("render.render_model").is_file());
        fs::remove_dir_all(destination).unwrap();
    }

    #[test]
    fn unavailable_reference_count_does_not_authorize_deletion() {
        let kit = fixture();
        let mut app = app();
        kit.install(&mut app);
        app.open_delete_folder("doomed".into(), ctx());
        assert!(referrers(&app.model, app.dialogs.get::<DeleteFolderConfirm>().unwrap()).is_none());
        app.begin_delete_folder(ctx());
        assert!(app.tag_ops.folder_refactor.is_none());
        assert!(kit.root.join("doomed").is_dir());
        crate::app::loose_fixture::pump_until(&mut app, "reference check", |app| !app.model.kits[0].index_jobs.building_references);
    }
}
