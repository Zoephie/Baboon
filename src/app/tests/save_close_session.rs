//! Characterization of the save, close, discard, undo and session-restore
//! flows on synthetic loose tags.
//!
//! These pin what the app does today, so moving the code that does it can be
//! shown to have changed nothing. Each flow runs through the entry point the
//! UI calls -- the close prompt is clicked, not short-circuited -- and each
//! assertion reads back state: files on disk, documents, tabs, the prompt.
//!
//! A Campaign Evolved source would route the prompt's Save into the pak
//! (`overwrite_current_tag_in_place` / `save_new_container_tag`), which needs a
//! mounted install; those branches are not reached here. The stash half of the
//! prompt is covered in `campaign_project_round_trips.rs`.

use super::loose_fixture::*;
use super::*;

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
    real_of(&app.kits[app.active].parsed_tags[key].tag, DISTANCE).expect("a real")
}

fn is_dirty(app: &Baboon, key: &str) -> bool {
    app.kits[app.active]
        .parsed_tags
        .get(key)
        .is_some_and(|document| document.dirty.is_set())
}

fn tab_open(app: &Baboon, key: &str) -> bool {
    app.kits[app.active].open_tabs.iter().any(|open| open == key)
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

    assert!(!app.save_changes_prompt.visible);
    assert!(!tab_open(&app, &key));
    assert!(!app.kits[0].parsed_tags.contains_key(&key), "the document is dropped");
}

#[test]
fn closing_a_dirty_tab_raises_the_prompt_and_leaves_the_tab_open() {
    let (_kit, mut app, key, other) = edited("close-dirty");

    app.request_close_action(PendingCloseAction::CloseTab(key.clone()), &ctx());

    let prompt = &app.save_changes_prompt;
    assert!(prompt.visible);
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
        &app.save_changes_prompt.pending_action,
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
    assert!(!app.save_changes_prompt.visible);
    assert!(app.save_changes_prompt.dirty_tags.is_empty());
    assert_eq!(app.status, "Saved 1 file(s)");
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
    assert!(!app.save_changes_prompt.visible);
    assert!(!tab_open(&app, &key));
    assert!(!app.kits[0].parsed_tags.contains_key(&key));
}

#[test]
fn the_prompt_s_cancel_keeps_the_tab_and_its_edit() {
    let (kit, mut app, key, _other) = edited("close-cancel");
    app.request_close_action(PendingCloseAction::CloseTab(key.clone()), &ctx());
    let mut driver = PromptDriver::new();

    driver.click(&mut app, "Cancel");

    assert!(!app.save_changes_prompt.visible);
    assert!(app.save_changes_prompt.dirty_tags.is_empty());
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
    app.save_changes_prompt.dirty_tags[0].checked = false;
    let mut driver = PromptDriver::new();

    driver.click(&mut app, "Save");

    assert_eq!(distance_on_disk(&kit, MODEL), 0.0);
    assert_eq!(app.status, "No files selected to save");
    assert!(app.save_changes_prompt.visible, "prompted again");
    assert_eq!(app.save_changes_prompt.dirty_tags.len(), 1);
    assert!(app.save_changes_prompt.dirty_tags[0].checked, "re-listed checked");
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

    let prompt = &app.save_changes_prompt;
    assert!(prompt.visible);
    let error = prompt.error.as_deref().expect("an error is shown");
    assert!(error.starts_with("Save failed: "), "{error}");
    assert!(error.contains("crate.model"), "{error}");
    assert_eq!(app.status, error);
    assert_eq!(prompt.dirty_tags.len(), 1, "the tag is listed again");
    assert!(tab_open(&app, &key));
    assert!(is_dirty(&app, &key));
}

#[test]
fn close_all_prompts_for_the_dirty_tags_only_then_closes_everything() {
    let (_kit, mut app, key, other) = edited("close-all");

    app.request_close_action(PendingCloseAction::CloseAllTabs, &ctx());
    assert_eq!(
        app.save_changes_prompt
            .dirty_tags
            .iter()
            .map(|entry| entry.tag_id.clone())
            .collect::<Vec<_>>(),
        vec![key.clone()],
        "the clean tab is not listed"
    );
    PromptDriver::new().click(&mut app, "Don't Save");

    assert!(app.kits[0].open_tabs.is_empty());
    assert!(app.kits[0].parsed_tags.is_empty());
    assert_eq!(app.kits[0].selected_key, None);
    assert!(!tab_open(&app, &other));
}

#[test]
fn close_all_but_this_keeps_the_named_tab_and_its_unsaved_edit() {
    let (_kit, mut app, key, other) = edited("close-all-but");

    // The dirty tag is the one kept, so nothing needs saving.
    app.request_close_action(PendingCloseAction::CloseAllButThis(key.clone()), &ctx());

    assert!(!app.save_changes_prompt.visible);
    assert_eq!(app.kits[0].open_tabs, vec![key.clone()]);
    assert!(!app.kits[0].parsed_tags.contains_key(&other));
    assert!(is_dirty(&app, &key));
    assert_eq!(app.kits[0].selected_key.as_deref(), Some(key.as_str()));

    // Keeping the clean one instead lists the dirty one.
    let (_kit, mut app, key, other) = edited("close-all-but-other");
    app.request_close_action(PendingCloseAction::CloseAllButThis(other.clone()), &ctx());
    assert!(app.save_changes_prompt.visible);
    assert_eq!(app.save_changes_prompt.dirty_tags[0].tag_id, key);
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
        app.deferred_file_action,
        Some(DeferredFileAction::Close(PendingCloseAction::CloseApp))
    ));
    // What the next frame's `run_deferred_file_action` does with it.
    let Some(DeferredFileAction::Close(action)) = app.deferred_file_action.take() else {
        unreachable!()
    };
    let _ = crate::app::run_ui_test(&ctx, screen(Vec::new(), 1.1), |ui| {
        app.request_close_action(action.clone(), ui.ctx())
    });
    assert!(app.save_changes_prompt.visible, "dirty work is prompted for");
    assert!(matches!(
        app.save_changes_prompt.pending_action,
        PendingCloseAction::CloseApp
    ));

    // Don't Save: the edit is dropped and the close re-issued.
    let mut driver = PromptDriver::on(ctx.clone(), 2.0);
    driver.click(&mut app, "Don't Save");
    assert!(driver.commands.contains(&egui::ViewportCommand::Close));
    assert!(app.save_changes_prompt.allow_app_close_once);
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
    assert!(!app.save_changes_prompt.allow_app_close_once);
    assert!(app.deferred_file_action.is_none());
}

#[test]
fn the_app_does_not_close_while_a_folder_refactor_runs() {
    let mut app = app();
    app.folder_refactor = Some(FolderRefactorUiState {
        label: "Renaming".to_owned(),
        phase: "Moving files".to_owned(),
        progress: None,
    });
    let ctx = egui::Context::default();

    let output = crate::app::run_ui_test(&ctx, close_requested_input(1.0), |ui| {
        app.handle_app_close_request(ui.ctx())
    });

    assert!(root_commands(&output).contains(&egui::ViewportCommand::CancelClose));
    assert!(app.deferred_file_action.is_none());
    assert_eq!(
        app.status,
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

    assert!(!app.save_changes_prompt.visible);
    assert!(root_commands(&output).contains(&egui::ViewportCommand::Close));
    assert!(app.save_changes_prompt.allow_app_close_once);
}

#[test]
fn save_current_tag_writes_the_selected_tag() {
    let (kit, mut app, key, _other) = edited("save-current");

    app.save_current_tag(&ctx());

    assert_eq!(distance_on_disk(&kit, MODEL), 12.5);
    assert!(!is_dirty(&app, &key));
    assert_eq!(
        app.status,
        format!("Saved {}", kit.root.join(MODEL).display())
    );
    // The save records what the tag now references.
    let source = app.kits[0].source.as_ref().unwrap();
    assert!(source.reverse_dependencies.is_none(), "no index was built to update");
}

#[test]
fn save_current_tag_needs_a_selection() {
    let mut app = app();
    app.save_current_tag(&ctx());
    assert_eq!(app.status, "No tag selected");
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
    app.kits[0].parsed_tags.get_mut(&key).unwrap().tag.endian = blam_tags::Endian::Be;
    let refused = app.save_tag_by_key(&key).unwrap_err();
    let entry = app.entry_for_key(&key).cloned().unwrap();
    assert_eq!(
        Some(refused),
        unsaveable_reason(&entry, &app.kits[0].parsed_tags[&key].tag)
            .map(|reason| reason.to_string())
    );

    // A clean, loaded tag saves even with nothing to save.
    app.kits[0].parsed_tags.get_mut(&key).unwrap().tag.endian = blam_tags::Endian::Le;
    assert_eq!(app.save_tag_by_key(&key), Ok(kit.root.join(MODEL)));
}

/// Everything before the file dialog: Save As refuses without a selection, an
/// entry, or a loaded document.
#[test]
fn save_as_refuses_before_its_dialog() {
    let kit = kit("save-as-guards");
    let mut app = app();
    app.save_current_tag_as();
    assert_eq!(app.status, "No tag selected");

    kit.install(&mut app);
    app.kits[0].selected_key = Some("file:/nowhere.model".to_owned());
    app.save_current_tag_as();
    assert_eq!(app.status, "Selected tag is no longer in the source");

    app.kits[0].selected_key = Some(kit.key(MODEL));
    app.save_current_tag_as();
    assert_eq!(app.status, "Load the selected tag before saving");
}

/// What Save As does after its dialog: a copy written inside the loaded tags
/// folder is registered in the browser, keyed the way a scan keys it.
#[test]
fn a_save_as_copy_inside_the_tags_folder_joins_the_browser() {
    let kit = kit("save-as-register");
    let mut app = app();
    kit.install(&mut app);
    let generation = app.kits[0].generation;
    let copy = kit.write_mcc("objects/copies/crate_copy", "model", |_| {});

    assert_eq!(app.register_saved_copy_if_in_loaded_folder(&copy), Ok(true));

    let key = kit.key("objects/copies/crate_copy.model");
    assert!(app.entry_for_key(&key).is_some());
    assert_ne!(app.kits[0].generation, generation);

    // Outside the tags folder there is nothing to register.
    let outside = kit.base.join("elsewhere.model");
    fs::copy(&copy, &outside).unwrap();
    assert_eq!(app.register_saved_copy_if_in_loaded_folder(&outside), Ok(false));
}

#[test]
fn discarding_reloads_the_tag_from_disk() {
    let (_kit, mut app, key, _other) = edited("discard");
    let label = app.tag_path_label(&key);

    app.discard_tag_changes(0, &key, &ctx());
    assert_eq!(app.status, format!("Discarded unsaved changes to {label}"));
    assert!(!app.kits[0].parsed_tags.contains_key(&key), "the document is dropped");
    assert!(app.kits[0].loading_tags.contains(&key), "and read again");
    pump_until(&mut app, "the reload", |app| {
        app.kits[0].parsed_tags.contains_key(&key)
    });

    assert!(!is_dirty(&app, &key));
    assert_eq!(distance_in_document(&app, &key), 0.0);
    assert!(tab_open(&app, &key));
    assert!(label.ends_with("crate.model"));

    app.discard_tag_changes(0, &key, &ctx());
    assert_eq!(app.status, "That tag has no unsaved changes");
}

#[test]
fn discarding_a_closed_tag_drops_its_document_without_reloading() {
    let (_kit, mut app, key, _other) = edited("discard-closed");
    app.kits[0].close_tag_pane(&key);
    let label = app.tag_path_label(&key);

    app.discard_tag_changes(0, &key, &ctx());

    assert_eq!(app.status, format!("Discarded unsaved changes to {label}"));
    assert!(!app.kits[0].parsed_tags.contains_key(&key));
    assert!(!app.kits[0].loading_tags.contains(&key));
}

#[test]
fn undo_and_redo_walk_the_selected_tag_s_edits() {
    let (_kit, mut app, key, _other) = edited("undo-redo");
    edit_field(&mut app, &key, DISTANCE, "20");
    assert_eq!(distance_in_document(&app, &key), 20.0);

    app.undo_current_tag();
    assert_eq!(distance_in_document(&app, &key), 12.5);
    assert!(app.status.starts_with("Undo"), "{}", app.status);
    app.undo_current_tag();
    assert_eq!(distance_in_document(&app, &key), 0.0);
    app.redo_current_tag();
    app.redo_current_tag();
    assert_eq!(distance_in_document(&app, &key), 20.0);
    assert!(app.status.starts_with("Redo"), "{}", app.status);
    assert!(is_dirty(&app, &key));

    app.kits[0].selected_key = None;
    app.undo_current_tag();
    assert_eq!(app.status, "Nothing to undo");
    app.redo_current_tag();
    assert_eq!(app.status, "Nothing to redo");
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
    let friction = |app: &Baboon| real_of(&app.kits[0].parsed_tags[&key].tag, "air friction");
    assert_eq!(
        app.kits[0].parsed_tags[&key].tag.classic_engine(),
        Some(blam_tags::classic::ClassicEngine::HaloCe)
    );

    edit_field(&mut app, &key, "air friction", "0.25");
    edit_field(&mut app, &key, "air friction", "0.5");
    app.undo_current_tag();
    assert_eq!(friction(&app), Some(0.25));
    assert_eq!(
        app.kits[0].parsed_tags[&key].tag.classic_engine(),
        Some(blam_tags::classic::ClassicEngine::HaloCe),
        "the snapshot came back classic"
    );
    app.redo_current_tag();
    assert_eq!(friction(&app), Some(0.5));

    app.save_current_tag(&ctx());
    let bytes = fs::read(kit.root.join("physics/pebble.point_physics")).unwrap();
    assert!(blam_tags::classic::ClassicHeader::parse(&bytes).is_some());
    assert!(TagFile::read_from_bytes(&bytes).is_err(), "not an MCC tag");
    let saved = crate::source::read_tag_from_bytes(
        &bytes,
        Some("haloce_mcc"),
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
    app.kits[0].browser_mode = BrowserMode::Groups;
    app.kits[0].browser_sort = BrowserSort::Type;
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
        app.kits[0]
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
        app.kits[app.active].open_tabs.len() >= 3
            && app.kits[app.active].parsed_tags.len() >= 2
    });

    let restored = &next.kits[next.active];
    assert!(restored.open_tabs.contains(&key) && restored.open_tabs.contains(&other));
    assert!(
        restored
            .folder_browsers
            .values()
            .any(|folder| folder.rel_path == Path::new("objects/props"))
    );
    assert_eq!(restored.browser_mode, BrowserMode::Groups);
    assert_eq!(restored.browser_sort, BrowserSort::Type);
    // The session does not record which tab was selected: each restored tag
    // is selected in turn, so the last one saved ends up selected.
    // QUIRK: with the saved order unstable, so is the restored selection.
    assert_eq!(restored.selected_key.as_ref(), Some(&last_saved));

    // With nothing loaded there is no session, and exit clears the file.
    Baboon::for_test().persist_session_on_exit();
    assert!(load_last_session().is_none());
}
