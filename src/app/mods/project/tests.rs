use super::*;

/// A container tag with a dot in its name has a new identity now that its
/// display path keeps the dot; a project written before still names it
/// by the old one, and that still finds it, unless it is ambiguous.
#[test]
fn a_dotted_container_tag_answers_to_its_old_identity_too() {
    let bitm = u32::from_be_bytes(*b"bitm");
    let entry = |logical: &str| TagEntry {
        key: format!("ublock:pakchunk0:Meteorite/Content/Tags/{logical}-bitmap.ubulk"),
        // What the container loader now displays it as.
        display_path: format!("{logical}.bitmap"),
        group_tag: bitm,
        group_name: None,
        location: TagEntryLocation::Container {
            container: 0,
            rel_path: format!("Meteorite/Content/Tags/{logical}-bitmap.ubulk"),
        },
    };
    let mut app = Baboon::for_test();
    let source = |entries: Vec<TagEntry>| LoadedSourceData {
        label: "ce".to_owned(),
        source: TagSource::LooseFolder {
            root: PathBuf::from("/ce"),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names: TagNameIndex::default(),
        game: None,
        entries,
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    };
    app.install_loaded_source(source(vec![entry("levels/v1.2/bitmaps/rock")]));
    let found = |app: &Baboon, identity: &str| {
        app.campaign_entry_for_identity(0, identity)
            .map(|entry| entry.display_path)
    };
    assert_eq!(
        found(&app, "6269746d:levels/v1.2/bitmaps/rock").as_deref(),
        Some("levels/v1.2/bitmaps/rock.bitmap")
    );
    assert_eq!(
        found(&app, "6269746d:levels/v1").as_deref(),
        Some("levels/v1.2/bitmaps/rock.bitmap"),
        "the identity an older build recorded"
    );

    // Two tags that both had that old identity: neither is guessed.
    app.install_loaded_source(source(vec![
        entry("levels/v1.2/bitmaps/rock"),
        entry("levels/v1.3/bitmaps/rock"),
    ]));
    assert_eq!(found(&app, "6269746d:levels/v1"), None);
}

/// The revision has to keep climbing across a save, or a cache that saw
/// revision 1, watched the document be saved and then edited again, would
/// see revision 1 once more and skip work it needed to do.
#[test]
fn a_dirty_revision_never_repeats() {
    let mut dirty = Dirty::default();
    assert!(!dirty.is_set());
    dirty.touch();
    let first = dirty.revision();
    dirty.clear();
    assert!(!dirty.is_set());
    dirty.touch();
    assert!(dirty.is_set());
    assert_ne!(dirty.revision(), first);
}

fn overlay(identity: &str, bytes: &[u8]) -> CampaignProjectOverlay {
    CampaignProjectOverlay {
        identity: identity.to_owned(),
        group_tag: 0x1234_5678,
        logical_path: identity.to_owned(),
        kind: CampaignProjectTagKind::Existing,
        package: None,
        digest: overlay_digest(bytes),
        bytes: Arc::new(bytes.to_vec()),
    }
}

fn history_of(sizes: &[usize]) -> TagHistory {
    TagHistory {
        undo: sizes
            .iter()
            .enumerate()
            .map(|(index, size)| HistoryStep {
                id: index as u64 + 1,
                label: format!("edit {index}"),
                bytes: Arc::new(vec![0; *size]),
            })
            .collect(),
        redo: Vec::new(),
        revision: 1,
    }
}

#[test]
fn history_is_trimmed_to_the_newest_steps() {
    let mut history = BTreeMap::from([("a".to_owned(), history_of(&[1; 20]))]);

    trim_history_for_disk(&mut history, 16, 1024);

    let kept: Vec<&str> = history["a"]
        .undo
        .iter()
        .map(|step| step.label.as_str())
        .collect();
    assert_eq!(kept.len(), 16);
    assert_eq!(
        kept.first().copied(),
        Some("edit 4"),
        "the oldest four steps are the ones dropped"
    );
    assert_eq!(kept.last().copied(), Some("edit 19"));
}

#[test]
fn the_byte_budget_is_shared_across_every_open_tag() {
    // The budget bounds the recovery file, so it cannot be per tag — ten
    // tags each holding "only" their own allowance is ten times the file.
    let mut history = BTreeMap::from([
        ("a".to_owned(), history_of(&[400, 400, 400])),
        ("b".to_owned(), history_of(&[400, 400, 400])),
    ]);

    trim_history_for_disk(&mut history, 16, 1000);

    let total: usize = history
        .values()
        .flat_map(|entry| entry.undo.iter().chain(entry.redo.iter()))
        .map(|step| step.bytes.len())
        .sum();
    assert!(total <= 1000, "kept {total} bytes against a 1000 budget");
    // What survives is the newest of each tag, not one tag's whole stack.
    for identity in ["a", "b"] {
        assert_eq!(
            history[identity]
                .undo
                .last()
                .map(|step| step.label.as_str()),
            Some("edit 2"),
            "{identity} kept its most recent step"
        );
    }
}

/// A save writes only the history steps it has not written before: an
/// unchanged history not at all, and a stack that moved only its new step.
/// Every step is a whole tag, and saves run twice a second while editing.
#[test]
fn a_save_writes_only_new_history_steps() {
    let path = temp_project("history-skip");
    let step = |id: u64, label: &str| HistoryStep {
        id,
        label: label.to_owned(),
        bytes: Arc::new(vec![id as u8; 3]),
    };
    let mut snapshot = snapshot_of(vec![overlay("a", b"one")]);
    snapshot.history = BTreeMap::from([(
        "a".to_owned(),
        TagHistory {
            undo: vec![step(1, "Edit color")],
            redo: Vec::new(),
            revision: 7,
        },
    )]);
    save_campaign_project(&path, &snapshot, None, ProjectScope::Session).unwrap();
    let saved = snapshot.digests();

    // Mark the row on disk, so a rewrite of it is detectable.
    let mark = || {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute("UPDATE history_steps SET label = 'sentinel'", [])
            .unwrap();
    };
    mark();

    // The tag was edited again, but the journal did not move.
    let mut later = snapshot_of(vec![overlay("a", b"two")]);
    later.history = snapshot.history.clone();
    save_campaign_project(&path, &later, Some(&saved), ProjectScope::Session).unwrap();
    let loaded = load_campaign_project(&path).unwrap();
    assert_eq!(loaded.history["a"].undo[0].label, "sentinel", "rewritten");
    assert_eq!(*loaded.overlays["a"].bytes, b"two".to_vec());

    // A new edit: the old step moves below it and is not written again.
    let mut moved = later.clone();
    let entry = moved.history.get_mut("a").unwrap();
    entry.undo.push(step(2, "Edit name"));
    entry.undo.push(step(5, "Edit size"));
    entry.revision = 8;
    save_campaign_project(&path, &moved, Some(&later.digests()), ProjectScope::Session)
        .unwrap();
    let loaded = load_campaign_project(&path).unwrap();
    let labels: Vec<&str> = loaded.history["a"]
        .undo
        .iter()
        .map(|step| step.label.as_str())
        .collect();
    assert_eq!(
        labels,
        ["sentinel", "Edit name", "Edit size"],
        "only the new steps are written"
    );

    // A new edit pushes the two oldest past the budget and one is then
    // undone: step 5 moves from the top to the bottom, beneath a new
    // step, and the file must read back in the snapshot's order.
    let mut shifted = moved.clone();
    let entry = shifted.history.get_mut("a").unwrap();
    entry.undo = vec![step(5, "Edit size"), step(7, "Edit scale")];
    entry.redo = vec![step(8, "Edit scale")];
    entry.revision = 9;
    save_campaign_project(
        &path,
        &shifted,
        Some(&moved.digests()),
        ProjectScope::Session,
    )
    .unwrap();
    let loaded = load_campaign_project(&path).unwrap();
    let ids = |steps: &[HistoryStep]| steps.iter().map(|step| step.id).collect::<Vec<_>>();
    assert_eq!(ids(&loaded.history["a"].undo), [5, 7]);
    assert_eq!(ids(&loaded.history["a"].redo), [8]);

    let _ = fs::remove_file(&path);
}

/// A recovery file from before steps had ids keeps its history in the old
/// table. It still opens with its history, and the next save moves it
/// across rather than trusting a row map that does not describe it.
#[test]
fn history_in_the_old_table_is_read_and_moved_across() {
    let path = temp_project("history-legacy");
    let snapshot = snapshot_of(vec![overlay("a", b"one")]);
    save_campaign_project(&path, &snapshot, None, ProjectScope::Session).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS history (
                     identity TEXT NOT NULL, stack TEXT NOT NULL, position INTEGER NOT NULL,
                     label TEXT NOT NULL, bytes BLOB NOT NULL,
                     PRIMARY KEY (identity, stack, position));
                 INSERT INTO history VALUES ('a', 'undo', 0, 'Old edit', x'0102');",
        )
        .unwrap();
    drop(connection);

    let loaded = load_campaign_project(&path).unwrap();
    assert_eq!(loaded.history["a"].undo[0].label, "Old edit");
    // What the app believes is on disk after adopting the file.
    let adopted = loaded.digests();
    let mut next = loaded.clone();
    next.history.get_mut("a").unwrap().revision += 1;
    save_campaign_project(&path, &next, Some(&adopted), ProjectScope::Session).unwrap();

    let connection = Connection::open(&path).unwrap();
    let legacy: i64 = connection
        .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))
        .unwrap();
    assert_eq!(legacy, 0, "the old table is cleared");
    drop(connection);
    let reloaded = load_campaign_project(&path).unwrap();
    assert_eq!(
        reloaded.history["a"].undo[0].label, "Old edit",
        "and nothing lost"
    );
    let _ = fs::remove_file(&path);
}

#[test]
fn a_tag_trimmed_to_nothing_leaves_no_row_behind() {
    let mut history = BTreeMap::from([("a".to_owned(), TagHistory::default())]);
    trim_history_for_disk(&mut history, 16, 1000);
    assert!(history.is_empty());
}

fn snapshot_of(overlays: Vec<CampaignProjectOverlay>) -> CampaignProjectSnapshot {
    CampaignProjectSnapshot {
        game: "haloce_evolved".to_owned(),
        source_path: PathBuf::from("Paks"),
        selected_identity: None,
        tabs: Vec::new(),
        overlays: overlays
            .into_iter()
            .map(|overlay| (overlay.identity.clone(), overlay))
            .collect(),
        history: BTreeMap::new(),
        folders: Default::default(),
    }
}

/// A save must rewrite only the overlays whose bytes changed. Stashing a
/// 105 MiB animation graph alongside a 4 MiB scenario meant every edit to
/// the scenario rewrote both.
#[test]
fn a_save_rewrites_only_the_overlays_whose_bytes_changed() {
    let path = std::env::temp_dir().join(format!(
        "baboon-project-diff-{}-{}.baboon",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let first = snapshot_of(vec![overlay("a", b"one"), overlay("b", b"two")]);
    save_campaign_project(&path, &first, None, ProjectScope::Session).unwrap();

    // "a" is unchanged, "b" is gone, "c" is new. The claim that "a" is
    // already on disk is honoured by digest, so passing different bytes
    // under its old digest proves the row really was skipped.
    let mut stale = overlay("a", b"REWRITTEN");
    stale.digest = overlay_digest(b"one");
    let second = snapshot_of(vec![stale, overlay("c", b"three")]);
    save_campaign_project(
        &path,
        &second,
        Some(&first.digests()),
        ProjectScope::Session,
    )
    .unwrap();

    let loaded = load_campaign_project(&path).unwrap();
    let mut identities: Vec<&String> = loaded.overlays.keys().collect();
    identities.sort();
    assert_eq!(identities, vec!["a", "c"], "b was dropped, c was added");
    assert_eq!(
        loaded.overlays["a"].bytes.as_slice(),
        b"one",
        "a's row was left alone"
    );
    assert_eq!(loaded.overlays["c"].bytes.as_slice(), b"three");
    let _ = std::fs::remove_file(&path);
}

/// The fingerprint is what autosave compares to decide whether to write, so
/// it has to notice a changed overlay while never touching its bytes.
#[test]
fn the_fingerprint_follows_the_digest() {
    let before = snapshot_of(vec![overlay("a", b"one")]);
    let same = snapshot_of(vec![overlay("a", b"one")]);
    let changed = snapshot_of(vec![overlay("a", b"other")]);
    assert_eq!(before.fingerprint(), same.fingerprint());
    assert_ne!(before.fingerprint(), changed.fingerprint());
}

fn temp_project(name: &str) -> PathBuf {
    crate::test_kits::unique_temp_path(name).with_extension("baboon")
}

fn identities_in(path: &Path) -> Vec<String> {
    let mut identities: Vec<String> = load_campaign_project(path)
        .unwrap()
        .overlays
        .into_keys()
        .collect();
    identities.sort();
    identities
}

/// A `.baboon` the user opened is not what the workspace writes to. It was,
/// and so an exported mod's sidecar became a live file the moment it was
/// opened: autosave rewrote it, and declining to save at exit deleted the
/// stashed rows straight out of it — which is how an exported mod's project
/// came back empty two sessions later.
#[test]
fn an_opened_project_is_never_the_autosave_target() {
    let recovery = temp_project("recovery");
    let opened = temp_project("opened");
    let snapshot = snapshot_of(vec![overlay("a", b"one")]);
    let project =
        ActiveCampaignProject::imported(recovery.clone(), opened.clone(), &snapshot, 0.0);
    assert_eq!(project.recovery_path, recovery);
    assert_eq!(project.project_path, Some(opened.clone()));
    assert_eq!(
        project.label(),
        opened.file_name().unwrap().to_string_lossy(),
        "the workspace is labelled with the project the user opened"
    );
}

/// An imported project's overlays have never been in the recovery file, so
/// neither the digests nor the fingerprint may claim they have. Either one
/// would let the first autosave decide there was nothing to write — leaving
/// the workspace's live copy as whatever the last session left there.
#[test]
fn an_imported_project_replaces_a_stale_recovery_file() {
    let recovery = temp_project("stale-recovery");
    // What an earlier session of this workspace left behind.
    save_campaign_project(
        &recovery,
        &snapshot_of(vec![overlay("old", b"x")]),
        None,
        ProjectScope::Session,
    )
    .unwrap();

    let imported = snapshot_of(vec![overlay("new", b"y")]);
    let project = ActiveCampaignProject::imported(
        recovery.clone(),
        temp_project("opened"),
        &imported,
        0.0,
    );
    assert!(
        project.saved_digests.is_none(),
        "nothing is known about the recovery file, so it must be replaced whole"
    );
    assert_ne!(
        project.last_saved_fingerprint,
        imported.fingerprint(),
        "the recovery file does not hold these bytes yet, so a write is due"
    );
    // Exactly what `checkpoint_campaign_project` then does with them.
    save_campaign_project(
        &recovery,
        &imported,
        project.saved_digests.as_ref(),
        ProjectScope::Session,
    )
    .unwrap();
    assert_eq!(
        identities_in(&recovery),
        vec!["new"],
        "the stale row was replaced, not merged into"
    );
    let _ = fs::remove_file(&recovery);
}

fn autosave_of(project: &ActiveCampaignProject, revision: u64) -> CampaignProjectWrite {
    let snapshot = snapshot_of(vec![overlay("a", b"one")]);
    CampaignProjectWrite {
        revision,
        path: project.recovery_path.clone(),
        fingerprint: snapshot.fingerprint(),
        snapshot,
        on_disk: None,
        write_lock: project.write_lock.clone(),
        latest_write_revision: project.latest_write_revision.clone(),
    }
}

/// An autosave that panicked sent nothing, so its save stayed in flight and
/// every later autosave was skipped for the session.
#[test]
fn an_autosave_that_panics_is_no_longer_in_flight() {
    let mut app = Baboon::for_test();
    let recovery = temp_project("panicking-autosave");
    let mut project = ActiveCampaignProject::adopted(recovery.clone(), &snapshot_of(Vec::new()), 0.0);
    project.save_in_flight = Some(3);
    project.latest_write_revision.store(3, Ordering::SeqCst);
    let write = autosave_of(&project, 3);
    app.model.kits[0].project.active = Some(project);

    let ctx = egui::Context::default();
    crate::app::with_panicking_workers(|| {
        write_campaign_project_in_background(&app.tx, &ctx, write)
    });
    assert!(crate::app::apply_next_worker_message(&mut app), "the autosave answered");
    let project = app.model.kits[0].project.active.as_ref().unwrap();
    assert_eq!(project.save_in_flight, None);
    assert!(app.model.status.contains("crashed"), "{}", app.model.status);
    let _ = fs::remove_file(&recovery);
}

/// The writer lock guards only the order of writes. A writer that panicked
/// while holding it poisoned it, and every later checkpoint then failed.
#[test]
fn a_poisoned_writer_lock_does_not_stop_later_saves() {
    let recovery = temp_project("poisoned-lock");
    let project = ActiveCampaignProject::adopted(recovery.clone(), &snapshot_of(Vec::new()), 0.0);
    project.latest_write_revision.store(1, Ordering::SeqCst);
    let lock = project.write_lock.clone();
    let _ = std::thread::spawn(move || {
        let _guard = lock.lock().unwrap();
        panic!("a writer fell over while holding the lock");
    })
    .join();
    assert!(project.write_lock.is_poisoned());

    let (tx, rx) = std::sync::mpsc::channel();
    write_campaign_project_in_background(&tx, &egui::Context::default(), autosave_of(&project, 1));
    let Ok(WorkerMessage::CampaignProjectSaved { result, .. }) =
        rx.recv_timeout(std::time::Duration::from_secs(10))
    else {
        panic!("the autosave did not answer");
    };
    let identities = load_campaign_project(&recovery).map(|loaded| loaded.overlays.len());
    let _ = fs::remove_file(&recovery);
    assert_eq!(result, Ok(()));
    assert_eq!(identities.ok(), Some(1), "the write reached the file");
}

/// The recovery file the workspace autosaves to is picked back up as-is, so
/// it needs neither a rewrite nor a full replace until something changes.
#[test]
fn an_adopted_recovery_file_is_believed() {
    let recovery = temp_project("adopted");
    let snapshot = snapshot_of(vec![overlay("a", b"one")]);
    let project = ActiveCampaignProject::adopted(recovery, &snapshot, 0.0);
    assert_eq!(project.saved_digests, Some(snapshot.digests()));
    assert_eq!(project.last_saved_fingerprint, snapshot.fingerprint());
    assert_eq!(project.project_path, None);
    assert_eq!(
        project.label(),
        "unsaved",
        "a recovery file is not a project the user named"
    );
}

/// Baboon's own recovery files are not save targets, and a session that
/// recorded one — every session written while the two were the same file —
/// must not come back reading as though the user had a project open.
#[test]
fn recovery_files_are_recognized_as_baboons_own() {
    assert!(is_campaign_recovery_file(&campaign_recovery_path(Some(
        Path::new("/games/evolved/Paks")
    ))));
    assert!(is_campaign_recovery_file(&campaign_recovery_path(None)));
    assert!(!is_campaign_recovery_file(Path::new(
        "/games/evolved/Paks/~mods/mymod_P.baboon"
    )));
}

/// Making a folder changes no overlay, no tab and no history, so if the
/// autosave fingerprint ignored it the write would be skipped and the
/// folder would be gone at the next launch — after looking, all session,
/// exactly as though it had been saved.
#[test]
fn the_autosave_fingerprint_notices_a_folder_only_change() {
    let base = snapshot_of(Vec::new());
    let mut with_folder = base.clone();
    with_folder.folders = ["objects/vehicles".to_owned()].into_iter().collect();
    assert_ne!(base.fingerprint(), with_folder.fingerprint());

    // And a different folder is a different session.
    let mut other = base.clone();
    other.folders = ["objects/characters".to_owned()].into_iter().collect();
    assert_ne!(with_folder.fingerprint(), other.fingerprint());

    // Same set, same fingerprint — otherwise every tick would rewrite.
    let mut repeat = base.clone();
    repeat.folders = ["objects/vehicles".to_owned()].into_iter().collect();
    assert_eq!(with_folder.fingerprint(), repeat.fingerprint());
}

/// A `.baboon` written before folders existed must still open.
///
/// This is why `CAMPAIGN_PROJECT_VERSION` is not bumped for the new table:
/// the version check is a strict equality, so raising it would reject every
/// project already on disk rather than migrate it. The table is purely
/// additive, so an older build ignores it and a newer one reads its absence
/// as "no folders".
#[test]
fn a_project_written_before_the_folders_table_still_opens() {
    let path = std::env::temp_dir().join(format!(
        "baboon-project-v1-{}-{}.baboon",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // The v1 schema, written literally — no `folders` table.
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE project (
                     id INTEGER PRIMARY KEY CHECK (id = 1),
                     version INTEGER NOT NULL,
                     game TEXT NOT NULL,
                     source_path TEXT NOT NULL,
                     selected_identity TEXT
                 );
                 CREATE TABLE tabs (
                     position INTEGER PRIMARY KEY,
                     identity TEXT NOT NULL UNIQUE,
                     label TEXT NOT NULL,
                     group_tag INTEGER NOT NULL,
                     logical_path TEXT NOT NULL,
                     kind TEXT NOT NULL,
                     package TEXT,
                     floating INTEGER NOT NULL
                 );
                 CREATE TABLE overlays (
                     identity TEXT PRIMARY KEY,
                     group_tag INTEGER NOT NULL,
                     logical_path TEXT NOT NULL,
                     kind TEXT NOT NULL,
                     package TEXT,
                     bytes BLOB NOT NULL
                 );
                 CREATE TABLE history (
                     identity TEXT NOT NULL,
                     stack TEXT NOT NULL,
                     position INTEGER NOT NULL,
                     label TEXT NOT NULL,
                     bytes BLOB NOT NULL,
                     PRIMARY KEY (identity, stack, position)
                 );
                 INSERT INTO project (id, version, game, source_path, selected_identity)
                 VALUES (1, 1, 'haloce_evolved', 'Paks', NULL);",
        )
        .unwrap();
    drop(connection);

    let loaded = load_campaign_project(&path).expect("a v1 project still opens");
    assert!(loaded.folders.is_empty());

    // And saving it forward adds the table without disturbing anything.
    let mut forward = loaded.clone();
    forward.folders = ["objects/vehicles".to_owned()].into_iter().collect();
    save_campaign_project(&path, &forward, None, ProjectScope::Session).unwrap();
    let reloaded = load_campaign_project(&path).unwrap();
    assert_eq!(reloaded.folders, forward.folders);
    let _ = fs::remove_file(path);
}

#[test]
fn campaign_project_round_trips_binary_overlays_and_tab_order() {
    let path = std::env::temp_dir().join(format!(
        "baboon-project-{}-{}.baboon",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let overlay = CampaignProjectOverlay {
        identity: "12345678:objects/test".to_owned(),
        group_tag: 0x1234_5678,
        logical_path: "objects/test".to_owned(),
        kind: CampaignProjectTagKind::Existing,
        package: None,
        digest: overlay_digest(&[0, 1, 2, 0xff]),
        bytes: Arc::new(vec![0, 1, 2, 0xff]),
    };
    let snapshot = CampaignProjectSnapshot {
        game: "haloce_evolved".to_owned(),
        source_path: PathBuf::from("Paks"),
        selected_identity: Some(overlay.identity.clone()),
        tabs: vec![CampaignProjectTab {
            identity: overlay.identity.clone(),
            label: "objects/test.weapon".to_owned(),
            group_tag: overlay.group_tag,
            logical_path: overlay.logical_path.clone(),
            kind: overlay.kind,
            package: None,
            floating: false,
        }],
        overlays: HashMap::from([(overlay.identity.clone(), overlay.clone())]),
        history: BTreeMap::from([(
            overlay.identity.clone(),
            TagHistory {
                undo: vec![HistoryStep {
                    id: 1,
                    label: "Edit color".to_owned(),
                    bytes: Arc::new(vec![7, 7, 7]),
                }],
                redo: vec![HistoryStep {
                    id: 2,
                    label: "Block edit".to_owned(),
                    bytes: Arc::new(vec![9]),
                }],
                revision: 3,
            },
        )]),
        folders: ["objects/vehicles".to_owned(), "sound/new".to_owned()]
            .into_iter()
            .collect(),
    };
    save_campaign_project(&path, &snapshot, None, ProjectScope::Session).unwrap();
    let loaded = load_campaign_project(&path).unwrap();
    assert_eq!(loaded.tabs.len(), 1);
    assert_eq!(loaded.tabs[0].identity, overlay.identity);
    assert_eq!(
        loaded.overlays[&loaded.tabs[0].identity].bytes,
        overlay.bytes
    );
    // The session, not just the files: which tags were open, what was
    // edited, and the steps that got there.
    let restored = &loaded.history[&overlay.identity];
    assert_eq!(restored.undo.len(), 1);
    assert_eq!(restored.undo[0].label, "Edit color");
    assert_eq!(*restored.undo[0].bytes, vec![7, 7, 7]);
    assert_eq!(restored.redo.len(), 1);
    assert_eq!(restored.redo[0].label, "Block edit");
    assert_eq!(*restored.redo[0].bytes, vec![9]);
    // A folder no tag has landed in exists nowhere but the workspace, so
    // without this it is gone on the next launch.
    assert_eq!(loaded.folders, snapshot.folders);

    // The same snapshot written as a mod's sidecar carries the tags and
    // nothing about how they were arrived at: that file is downloaded by
    // whoever installs the mod.
    let sidecar = path.with_extension("sidecar.baboon");
    save_campaign_project(&sidecar, &snapshot, None, ProjectScope::ModSidecar).unwrap();
    let published = load_campaign_project(&sidecar).unwrap();
    assert!(
        published.history.is_empty(),
        "an exported mod must not ship the author's undo history"
    );
    assert!(
        published.folders.is_empty(),
        "an exported mod must not ship the author's workspace folders"
    );
    assert_eq!(published.overlays.len(), snapshot.overlays.len());
    let _ = fs::remove_file(&sidecar);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute("UPDATE project SET version = 99 WHERE id = 1", [])
        .unwrap();
    drop(connection);
    assert!(
        load_campaign_project(&path)
            .unwrap_err()
            .contains("Unsupported Baboon project version")
    );
    let _ = fs::remove_file(path);
}

/// Export resolves each stashed overlay back to a tag by identity string,
/// taking the first entry that produces a match. Two tags sharing an
/// identity would therefore send one tag's edited bytes to the other's
/// path in the container -- a mod that builds and does the wrong thing, or
/// nothing.
#[test]
fn container_tag_identities_are_unique() {
    static PAKS: std::sync::LazyLock<&'static str> =
        std::sync::LazyLock::new(|| crate::test_kits::leak(crate::test_kits::ce_paks()));
    if !std::path::Path::new(*PAKS).exists() {
        eprintln!("skipping: Campaign Evolved not present");
        return;
    }
    let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = crate::core::format::TagNameIndex::load_from_definitions(&defs);
    let loaded = crate::core::source::load_iostore_container_set(
        std::path::PathBuf::from(*PAKS),
        &names,
        &defs,
    )
    .expect("mount container set");

    let mut seen: HashMap<String, String> = HashMap::new();
    let mut collisions = Vec::new();
    let mut identified = 0usize;
    for entry in loaded.entries.iter().chain(loaded.all_entries.iter()) {
        let Some((identity, ..)) = campaign_entry_project_parts(entry) else {
            continue;
        };
        identified += 1;
        let location = match &entry.location {
            TagEntryLocation::Container {
                container,
                rel_path,
            } => format!("container {container}: {rel_path}"),
            TagEntryLocation::NewContainer { package, .. } => format!("new: {package}"),
            _ => "other".to_owned(),
        };
        match seen.get(&identity) {
            Some(existing) if *existing != location => {
                collisions.push(format!("{identity}: {existing} vs {location}"));
            }
            Some(_) => {}
            None => {
                seen.insert(identity, location);
            }
        }
    }
    eprintln!("{identified} identified tag(s), {} distinct", seen.len());
    assert!(
        collisions.is_empty(),
        "{} identity collision(s):\n{}",
        collisions.len(),
        collisions
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
