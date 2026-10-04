//! Characterization of the loose-kit refactors -- tag rename and move,
//! folder rename, move and copy, duplicate, delete -- run to completion on a
//! temporary tree whose tags reference each other.
//!
//! Each job runs as it does in the app: the dialog state is set the way the
//! dialog leaves it, the apply entry point starts the worker, and the worker's
//! messages are applied until it settles. The assertions read the results
//! back off disk (files moved, references rewritten in the referrers' bytes)
//! and out of the app (tabs, favorites and keys remapped, failures reported).

use super::loose_fixture::*;
use super::*;

const RENDER: &str = "objects/props/crate.render_model";
const MODEL: &str = "objects/props/crate.model";
/// Outside the folder, referencing the render model inside it.
const USER: &str = "levels/test/crate_user.model";
const BARREL: &str = "objects/props/barrel.model";

/// `objects/props` holds a render model, a model referencing it and an
/// unrelated model; `levels/test` holds a second model referencing it from
/// outside the folder.
fn kit(name: &str) -> LooseKit {
    let kit = LooseKit::new(name, "halo3_mcc");
    let mode = group_tag("halo3_mcc", "render_model");
    kit.write_mcc("objects/props/crate", "render_model", |_| {});
    for rel in ["objects/props/crate", "levels/test/crate_user"] {
        kit.write_mcc(rel, "model", |tag| {
            set_reference(tag, "render model", mode, "objects\\props\\crate");
        });
    }
    kit.write_mcc("objects/props/barrel", "model", |_| {});
    kit
}

fn render_reference(kit: &LooseKit, rel: &str) -> Option<String> {
    let tag = TagFile::read(kit.root.join(rel)).expect("the referrer reads");
    reference_of(&tag, "render model")
}

/// The terminal's text, one line each.
fn terminal(app: &Baboon) -> Vec<String> {
    app.terminal.lines.iter().map(|line| line.text.clone()).collect()
}

fn settle(app: &mut Baboon, what: &str) {
    pump_until(app, what, |app| app.folder_refactor.is_none());
}

/// The paths of every tag in the kit, relative and with `/`.
fn tree(kit: &LooseKit) -> Vec<String> {
    let mut paths: Vec<String> = walkdir::WalkDir::new(&kit.root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|item| item.file_type().is_file())
        .map(|item| {
            item.path()
                .strip_prefix(&kit.root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    paths.sort();
    paths
}

#[test]
fn renaming_a_tag_moves_it_and_rewrites_every_referrer() {
    let kit = kit("refactor-rename-tag");
    let mut app = app();
    kit.install_indexed(&mut app);
    let old_key = kit.key(RENDER);
    let model_key = kit.key(MODEL);
    kit.open(&mut app, RENDER);
    kit.open(&mut app, MODEL);
    app.handle_browser_action(BrowserAction::ToggleFavorite(old_key.clone()), ctx());
    let generation = app.kits[0].generation;

    app.handle_browser_action(BrowserAction::RenameTag(old_key.clone()), ctx());
    {
        let state = app.rename_tag.as_mut().expect("the dialog opened");
        assert_eq!(state.referrers, vec![USER.to_owned(), MODEL.to_owned()]);
        state.new_path_input = "crate_renamed".to_owned();
    }
    app.begin_rename_tag(&ctx());
    assert!(app.rename_tag.is_none(), "the dialog closed");
    assert!(app.folder_refactor.is_some(), "the app is locked while it runs");
    settle(&mut app, "the rename");

    let new_rel = "objects/props/crate_renamed.render_model";
    let new_key = kit.key(new_rel);
    assert_eq!(
        app.status,
        "Renamed tag, updated 2 reference(s) in 2 tag(s)"
    );
    assert!(!kit.root.join(RENDER).exists());
    assert!(kit.root.join(new_rel).is_file());
    for referrer in [MODEL, USER] {
        assert_eq!(
            render_reference(&kit, referrer).as_deref(),
            Some("objects\\props\\crate_renamed"),
            "{referrer}"
        );
    }
    // Open state follows the tag to its new key.
    let tabs = &app.kits[0].open_tabs;
    assert!(tabs.contains(&new_key) && !tabs.contains(&old_key), "{tabs:?}");
    assert!(tabs.contains(&model_key));
    assert_eq!(app.kits[0].selected_key.as_deref(), Some(model_key.as_str()));
    // Every document is dropped, edited or not, to be read again.
    assert!(app.kits[0].parsed_tags.is_empty());
    assert_ne!(app.kits[0].generation, generation);
    // Favorites follow it too.
    assert_eq!(
        app.prefs.editing_kit_favorites[0].tags,
        vec![PathBuf::from(new_rel)]
    );
    // The browser and the reference index know the tag by its new path.
    let source = app.kits[0].source.as_ref().unwrap();
    assert!(source.all_entries.iter().any(|entry| entry.key == new_key));
    assert!(!source.all_entries.iter().any(|entry| entry.key == old_key));
    let index = source.reverse_dependencies.as_ref().expect("the index survives");
    let mut dependents = index
        .dependents_for(group_tag("halo3_mcc", "render_model"), "objects\\props\\crate_renamed")
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    dependents.sort();
    let mut expected = vec![kit.key(MODEL), kit.key(USER)];
    expected.sort();
    assert_eq!(dependents, expected);
    assert!(terminal(&app).contains(&format!("Renamed: {RENDER} -> {new_rel}")));
    assert!(terminal(&app).contains(&"Updated 2 reference(s) in 2 tag(s)".to_owned()));
}

#[test]
fn a_rename_waits_for_unsaved_edits_and_for_a_running_refactor() {
    let kit = kit("refactor-rename-refused");
    let mut app = app();
    kit.install(&mut app);
    let key = kit.open(&mut app, MODEL);
    edit_field(&mut app, &key, "disappear distance", "4");

    app.handle_browser_action(BrowserAction::RenameTag(kit.key(BARREL)), ctx());
    app.rename_tag.as_mut().unwrap().new_path_input = "keg".to_owned();
    app.begin_rename_tag(&ctx());
    assert_eq!(app.status, "Save or close dirty tags before renaming");
    assert!(app.rename_tag.is_some(), "the dialog stays open");
    assert!(kit.root.join(BARREL).is_file());

    app.kits[0].parsed_tags.get_mut(&key).unwrap().dirty.clear();
    app.folder_refactor = Some(FolderRefactorUiState {
        label: "Moving".to_owned(),
        phase: "Preparing".to_owned(),
        progress: None,
    });
    app.begin_rename_tag(&ctx());
    assert_eq!(app.status, "A move/rename is already running");

    // The name itself is checked before either.
    app.folder_refactor = None;
    for (input, refusal) in [
        ("", "Enter a new tag name"),
        ("sub/keg", "Enter a name only; use Move to choose a folder"),
        ("keg.model", "Enter a name without an extension"),
    ] {
        app.rename_tag.as_mut().unwrap().new_path_input = input.to_owned();
        app.begin_rename_tag(&ctx());
        assert_eq!(app.status, refusal, "{input:?}");
    }
}

/// Moving a tag is the rename job with a new folder: the file lands there and
/// the referrers follow.
#[test]
fn moving_a_tag_into_another_folder_rewrites_its_referrers() {
    let kit = kit("refactor-move-tag");
    let entries = kit.entries();
    let entry = entries
        .iter()
        .find(|entry| entry.display_path == RENDER)
        .unwrap()
        .clone();
    let (tx, _rx) = std::sync::mpsc::channel();

    let done = run_tag_rename_job(
        kit.root.clone(),
        entry,
        "objects/moved/crate".to_owned(),
        "Moving tag".to_owned(),
        kit.names(),
        Some(GameId::Halo3),
        entries.clone(),
        Some(kit.index()),
        &tx,
    )
    .expect("the move runs");

    assert_eq!(done.status, "Moved tag, updated 2 reference(s) in 2 tag(s)");
    assert!(done.moved);
    assert!(kit.root.join("objects/moved/crate.render_model").is_file());
    assert!(!kit.root.join(RENDER).exists());
    assert_eq!(
        render_reference(&kit, USER).as_deref(),
        Some("objects\\moved\\crate")
    );
    assert_eq!(done.old_to_new_keys.len(), 1);
    assert_eq!(done.all_entries.len(), entries.len());
    assert!(
        done.tree
            .children
            .iter()
            .any(|node| node.label == "objects"),
        "the browser tree is rebuilt from disk"
    );
}

/// Tag paths ignore case, so a rename that only changes case is no rename at
/// all: it is refused by name on every file system. It used to be refused only
/// where the file system ignores case (the destination "existed": it was the
/// tag itself), and on a case-sensitive one the job went ahead and rewrote
/// every reference to the new spelling.
#[test]
fn a_case_only_tag_rename_is_refused_on_every_file_system() {
    let kit = kit("refactor-case-only");
    let entries = kit.entries();
    let entry = entries
        .iter()
        .find(|entry| entry.display_path == RENDER)
        .unwrap()
        .clone();
    let (tx, _rx) = std::sync::mpsc::channel();

    let result = run_tag_rename_job(
        kit.root.clone(),
        entry,
        "objects/props/Crate".to_owned(),
        "Renaming tag".to_owned(),
        kit.names(),
        Some(GameId::Halo3),
        entries,
        Some(kit.index()),
        &tx,
    );

    let error = result.err().expect("refused");
    assert!(
        error.starts_with("Tag paths ignore case"),
        "refused by name, not by the file system: {error}"
    );
    assert_eq!(
        render_reference(&kit, USER).as_deref(),
        Some("objects\\props\\crate"),
        "nothing rewritten"
    );
    assert!(kit.root.join(RENDER).is_file(), "the tag was not moved");

    // Renaming a folder to a case variant is refused by name, everywhere.
    assert_eq!(
        validate_loose_folder_rename_for_test("Props", "props"),
        Err("Tag paths ignore case, so changing only the case would not change any reference"
            .to_owned())
    );
}

fn validate_loose_folder_rename_for_test(raw: &str, old: &str) -> Result<String, String> {
    super::folder_rename::validate_loose_folder_rename(raw, old, &[old.to_owned()])
}

/// A referrer that cannot be read does not stop the rename: the others are
/// rewritten, and the one left behind is named in the status and terminal.
#[test]
fn a_referrer_that_cannot_be_rewritten_is_reported_and_passed_over() {
    let kit = kit("refactor-failure");
    let mode = group_tag("halo3_mcc", "render_model");
    kit.write_mcc("levels/test/broken", "model", |tag| {
        set_reference(tag, "render model", mode, "objects\\props\\crate");
    });
    let entries = kit.entries();
    // Indexed while it still read, then corrupted: trailing bytes fail the
    // read, and the reference it holds is still there to be found.
    let index = kit.index();
    let broken = kit.root.join("levels/test/broken.model");
    let mut bytes = fs::read(&broken).unwrap();
    bytes.extend_from_slice(b"not part of the tag");
    fs::write(&broken, &bytes).unwrap();
    let entry = entries
        .iter()
        .find(|entry| entry.display_path == RENDER)
        .unwrap()
        .clone();
    let (tx, _rx) = std::sync::mpsc::channel();

    let done = run_tag_rename_job(
        kit.root.clone(),
        entry,
        "objects/props/crate2".to_owned(),
        "Renaming tag".to_owned(),
        kit.names(),
        Some(GameId::Halo3),
        entries,
        Some(index),
        &tx,
    )
    .expect("the rename itself succeeds");

    assert_eq!(
        done.status,
        "Renamed tag, updated 2 reference(s) in 2 tag(s); 1 tag(s) could NOT be updated and \
         may still reference the old path (see terminal)"
    );
    assert!(kit.root.join("objects/props/crate2.render_model").is_file());
    assert_eq!(
        render_reference(&kit, USER).as_deref(),
        Some("objects\\props\\crate2")
    );
    assert_eq!(fs::read(&broken).unwrap(), bytes, "the broken tag is untouched");
    assert!(
        done.lines
            .iter()
            .any(|line| line.starts_with("Warning: not updated: levels/test/broken.model (")),
        "{:?}",
        done.lines
    );
    assert!(done.lines.contains(
        &"Warning: 1 tag(s) could not be updated and may still reference the old path:".to_owned()
    ));
}

#[test]
fn renaming_a_folder_moves_its_tags_and_rewrites_referrers_outside_it() {
    let kit = kit("refactor-rename-folder");
    let mut app = app();
    kit.install_indexed(&mut app);
    let old_key = kit.key(RENDER);
    kit.open(&mut app, RENDER);
    app.handle_browser_action(BrowserAction::ToggleFavorite(old_key.clone()), ctx());
    app.handle_browser_action(
        BrowserAction::ToggleFolderFavorite(PathBuf::from("objects/props")),
        ctx(),
    );

    app.handle_browser_action(
        BrowserAction::RenameLooseFolder {
            rel_path: PathBuf::from("objects/props"),
            label: "props".to_owned(),
        },
        ctx(),
    );
    {
        let state = app.loose_folder_rename.as_mut().expect("the dialog opened");
        assert_eq!(state.tag_count, 3);
        assert_eq!(state.outside_referrers.as_deref(), Some(&[USER.to_owned()][..]));
        state.name_input = "Props".to_owned();
    }
    assert!(!app.apply_loose_folder_rename(), "a case-only rename keeps the dialog open");
    assert_eq!(
        app.loose_folder_rename.as_ref().unwrap().error.as_deref(),
        Some("Tag paths ignore case, so changing only the case would not change any reference")
    );
    app.loose_folder_rename.as_mut().unwrap().name_input = "crates".to_owned();
    assert!(app.apply_loose_folder_rename(), "accepted");
    assert!(app.folder_refactor.is_some());
    settle(&mut app, "the folder rename");

    assert!(app.status.starts_with("Renamed"), "{}", app.status);
    assert!(!app.status.contains("NOT"), "{}", app.status);
    assert_eq!(
        tree(&kit),
        vec![
            "levels/test/crate_user.model",
            "objects/crates/barrel.model",
            "objects/crates/crate.model",
            "objects/crates/crate.render_model",
        ]
    );
    for referrer in [USER, "objects/crates/crate.model"] {
        assert_eq!(
            render_reference(&kit, referrer).as_deref(),
            Some("objects\\crates\\crate"),
            "{referrer}"
        );
    }
    let new_key = kit.key("objects/crates/crate.render_model");
    assert!(app.kits[0].open_tabs.contains(&new_key));
    assert!(!app.kits[0].open_tabs.contains(&old_key));
    assert_eq!(
        app.prefs.editing_kit_favorites[0].tags,
        vec![PathBuf::from("objects/crates/crate.render_model")]
    );
    // A favorited folder follows the rename, as its tags do. It used to be
    // left at the old path and then pruned as missing, losing the favorite.
    assert_eq!(
        app.prefs.editing_kit_favorites[0].folders,
        vec![PathBuf::from("objects/crates")]
    );
    assert_eq!(app.kits[0].active_favorite_folders.len(), 1);
}

#[test]
fn moving_a_folder_carries_its_tags_and_their_references() {
    let kit = kit("refactor-move-folder");
    let (tx, _rx) = std::sync::mpsc::channel();

    let done = run_folder_refactor_job(
        kit.root.clone(),
        PathBuf::from("objects/props"),
        kit.root.join("levels"),
        None,
        true,
        "Moving props".to_owned(),
        kit.names(),
        Some(GameId::Halo3),
        kit.entries(),
        Some(kit.index()),
        &tx,
    )
    .expect("the move runs");

    assert!(done.moved);
    assert!(!done.status.contains("NOT"), "{}", done.status);
    assert_eq!(
        tree(&kit),
        vec![
            "levels/props/barrel.model",
            "levels/props/crate.model",
            "levels/props/crate.render_model",
            "levels/test/crate_user.model",
        ]
    );
    assert_eq!(
        render_reference(&kit, USER).as_deref(),
        Some("levels\\props\\crate")
    );
    assert_eq!(
        render_reference(&kit, "levels/props/crate.model").as_deref(),
        Some("levels\\props\\crate")
    );
    assert_eq!(done.old_to_new_keys.len(), 3);
    assert_eq!(
        done.old_to_new_keys.get(&kit.key(USER)),
        None,
        "a referrer outside the folder keeps its key"
    );
}

/// A copy leaves the original and its referrers alone; the copied tags
/// reference the copies, not the originals.
#[test]
fn copying_a_folder_leaves_the_original_and_points_the_copy_at_itself() {
    let kit = kit("refactor-copy-folder");
    let (tx, _rx) = std::sync::mpsc::channel();

    let done = run_folder_refactor_job(
        kit.root.clone(),
        PathBuf::from("objects/props"),
        kit.root.join("levels"),
        None,
        false,
        "Copying props".to_owned(),
        kit.names(),
        Some(GameId::Halo3),
        kit.entries(),
        Some(kit.index()),
        &tx,
    )
    .expect("the copy runs");

    assert!(!done.moved);
    assert!(done.old_to_new_keys.is_empty() || !done.moved);
    assert_eq!(
        tree(&kit),
        vec![
            "levels/props/barrel.model",
            "levels/props/crate.model",
            "levels/props/crate.render_model",
            "levels/test/crate_user.model",
            "objects/props/barrel.model",
            "objects/props/crate.model",
            "objects/props/crate.render_model",
        ]
    );
    assert_eq!(
        render_reference(&kit, USER).as_deref(),
        Some("objects\\props\\crate"),
        "the outside referrer keeps the original"
    );
    assert_eq!(
        render_reference(&kit, MODEL).as_deref(),
        Some("objects\\props\\crate")
    );
    assert_eq!(
        render_reference(&kit, "levels/props/crate.model").as_deref(),
        Some("levels\\props\\crate")
    );
    assert_eq!(done.all_entries.len(), 7);
}

#[test]
fn duplicating_a_tag_copies_its_bytes_beside_it() {
    let kit = kit("refactor-duplicate");
    let mut app = app();
    kit.install(&mut app);
    let key = kit.key(MODEL);

    app.handle_browser_action(BrowserAction::DuplicateTag(key.clone()), ctx());
    app.rename_tag.as_mut().unwrap().new_path_input = "crate_copy".to_owned();
    app.begin_rename_tag(&ctx());

    let copy = kit.root.join("objects/props/crate_copy.model");
    assert_eq!(fs::read(&copy).unwrap(), fs::read(kit.root.join(MODEL)).unwrap());
    assert_eq!(
        app.status,
        format!("Duplicated {MODEL} → {}", copy.display())
    );
    let copy_key = kit.key("objects/props/crate_copy.model");
    assert!(app.entry_for_key(&copy_key).is_some(), "registered in the browser");
    assert!(app.kits[0].parsed_tags.contains_key(&copy_key), "and opened clean");
    assert!(!app.kits[0].parsed_tags[&copy_key].dirty.is_set());
    assert!(app.rename_tag.is_none());

    // A name already taken is refused, and nothing is written.
    let barrel = fs::read(kit.root.join(BARREL)).unwrap();
    app.handle_browser_action(BrowserAction::DuplicateTag(key.clone()), ctx());
    app.rename_tag.as_mut().unwrap().new_path_input = "barrel".to_owned();
    app.begin_rename_tag(&ctx());
    assert!(app.rename_tag.is_some(), "the dialog stays open");
    assert_eq!(fs::read(kit.root.join(BARREL)).unwrap(), barrel);
    assert_eq!(app.status, "A tag with that name already exists in this source");
}

/// A dirty tag is duplicated with its edit; the original file is not saved.
#[test]
fn duplicating_an_edited_tag_copies_the_edit_not_the_file() {
    let kit = kit("refactor-duplicate-dirty");
    let mut app = app();
    kit.install(&mut app);
    let key = kit.open(&mut app, MODEL);
    edit_field(&mut app, &key, "disappear distance", "9");
    let original = fs::read(kit.root.join(MODEL)).unwrap();

    app.handle_browser_action(BrowserAction::DuplicateTag(key.clone()), ctx());
    app.rename_tag.as_mut().unwrap().new_path_input = "crate_edited".to_owned();
    app.begin_rename_tag(&ctx());

    let copy = TagFile::read(kit.root.join("objects/props/crate_edited.model")).unwrap();
    assert_eq!(real_of(&copy, "disappear distance"), Some(9.0));
    assert_eq!(fs::read(kit.root.join(MODEL)).unwrap(), original);
    assert!(app.kits[0].parsed_tags[&key].dirty.is_set(), "the original stays dirty");
}

#[test]
fn deleting_a_tag_moves_it_to_the_trash_and_forgets_it() {
    let kit = kit("refactor-delete");
    // A leaf no other test deletes: the trash is shared by the test process
    // and keyed by the second.
    let doomed = "objects/props/doomed_by_delete_test.model";
    kit.write_mcc("objects/props/doomed_by_delete_test", "model", |_| {});
    let bytes = fs::read(kit.root.join(doomed)).unwrap();
    let mut app = app();
    kit.install(&mut app);
    let key = kit.open(&mut app, doomed);
    let generation = app.kits[0].generation;

    app.handle_browser_action(BrowserAction::DeleteTag(key.clone()), ctx());
    assert!(app.delete_confirm.is_some());
    app.begin_delete_tag(ctx());

    assert!(!kit.root.join(doomed).exists());
    let destination = app
        .status
        .strip_prefix(&format!("Deleted {doomed} — moved to "))
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{}", app.status));
    assert_eq!(fs::read(&destination).unwrap(), bytes, "moved, not destroyed");
    assert!(destination.ends_with(doomed));
    assert!(
        destination
            .components()
            .any(|part| part.as_os_str() == "halo3_mcc"),
        "{}",
        destination.display()
    );
    assert!(app.entry_for_key(&key).is_none());
    assert!(!app.kits[0].parsed_tags.contains_key(&key));
    assert_eq!(app.kits[0].selected_key, None);
    assert_ne!(app.kits[0].generation, generation);
    assert!(app.delete_confirm.is_none());
    let _ = fs::remove_file(destination);
}
