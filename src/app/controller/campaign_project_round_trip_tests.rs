//! Characterization of Campaign Evolved project persistence and mod-export
//! review, without an install.
//!
//! The source is a container set with no containers mounted: its browser
//! entries are container tags, and their documents are built from the
//! Campaign Evolved definitions. That reaches everything that works off the
//! entries and documents -- capture, the recovery file, autosave, the stash,
//! the close prompt's stash and discard, the export review -- and stops where
//! a real `.utoc` would be read or written: building a mod container, and
//! telling a stashed tag apart from the shipped one, need an install
//! (`mod_override_tests.rs` covers those against `BLAM_TEST_CE`).

use super::loose_fixture::*;
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

const GROUP: &str = "point_physics";
const FRICTION: &str = "air friction";

/// A stand-in `Paks` directory, and the recovery file Baboon keys off it.
struct CeKit {
    root: PathBuf,
}

impl CeKit {
    fn new(name: &str) -> Self {
        let root = fs::canonicalize(crate::test_kits::unique_temp_dir(name)).unwrap();
        Self { root }
    }

    fn recovery(&self) -> PathBuf {
        campaign_recovery_path(Some(&self.root))
    }

    fn entry(&self, path: &str) -> TagEntry {
        TagEntry {
            key: format!("ublock:pakchunk0:{path}"),
            display_path: format!("{path}.{GROUP}"),
            group_tag: group_tag("haloce_evolved", GROUP),
            group_name: Some(GROUP.to_owned()),
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: format!("Tags/{path}-{GROUP}.ubulk"),
            },
        }
    }

    /// A container tag's project identity: its group, then its path without
    /// the extension.
    fn identity(&self, path: &str) -> String {
        format!("{:08x}:{path}", group_tag("haloce_evolved", GROUP))
    }

    /// An app with this source installed and `paths` open, the first one
    /// selected.
    fn app(&self, paths: &[&str]) -> Baboon {
        let entries: Vec<TagEntry> = paths.iter().map(|path| self.entry(path)).collect();
        let mut app = Baboon::for_test();
        app.install_loaded_source(LoadedSourceData {
            label: "Campaign Evolved".to_owned(),
            source: TagSource::IoStoreContainerSet {
                root: self.root.clone(),
                containers: Vec::new(),
                index: Default::default(),
                packages: Default::default(),
                shipped: Default::default(),
            },
            names: TagNameIndex::load_game(&locate_definitions_root(), GameId::CampaignEvolved)
                .unwrap(),
            game: Some(GameId::CampaignEvolved),
            entries: entries.clone(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: entries.clone(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        });
        for entry in entries.iter().rev() {
            let tag = TagFile::new(definition("haloce_evolved", GROUP)).unwrap();
            app.kits[0]
                .parsed_tags
                .insert(entry.key.clone(), TagDocument::clean(tag));
            app.kits[0].open_tag_pane(&entry.key);
        }
        app.kits[0].selected_key = Some(entries[0].key.clone());
        app
    }
}

impl Drop for CeKit {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.recovery());
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn key(path: &str) -> String {
    format!("ublock:pakchunk0:{path}")
}

fn friction_in(bytes: &[u8]) -> Option<f32> {
    real_of(&TagFile::read_from_bytes(bytes).ok()?, FRICTION)
}

/// One frame at `time` running the autosave.
fn autosave_at(app: &mut Baboon, ctx: &egui::Context, time: f64) {
    let _ = crate::app::run_ui_test(&ctx, screen(Vec::new(), time), |ui| {
        app.maybe_autosave_campaign_projects(ui.ctx())
    });
}

fn project(app: &Baboon) -> &ActiveCampaignProject {
    app.kits[0].campaign_project.as_ref().expect("a project")
}

fn step(id: u64, label: &str, bytes: &[u8]) -> HistoryStep {
    HistoryStep {
        id,
        label: label.to_owned(),
        bytes: Arc::new(bytes.to_vec()),
    }
}

fn overlay(identity: &str, kind: CampaignProjectTagKind, bytes: &[u8]) -> CampaignProjectOverlay {
    let (_, logical_path) = identity.split_once(':').unwrap();
    CampaignProjectOverlay {
        identity: identity.to_owned(),
        group_tag: 0x7070_6879,
        logical_path: logical_path.to_owned(),
        kind,
        package: (kind == CampaignProjectTagKind::New)
            .then(|| format!("/Game/Tags/{logical_path}")),
        digest: overlay_digest(bytes),
        bytes: Arc::new(bytes.to_vec()),
    }
}

/// `snapshot` as it reads back: history revisions are not stored.
fn without_revisions(snapshot: &CampaignProjectSnapshot) -> CampaignProjectSnapshot {
    let mut snapshot = snapshot.clone();
    for history in snapshot.history.values_mut() {
        history.revision = 0;
    }
    snapshot
}

/// Everything a project holds, written and read back.
#[test]
fn a_saved_project_loads_back_field_for_field() {
    let kit = CeKit::new("project-round-trip");
    let existing = "70706879:objects/rock.point_physics";
    let new = "70706879:objects/mine/pebble.point_physics";
    let binary: Vec<u8> = (0..=255).chain([0, 0, 255]).collect();
    let snapshot = CampaignProjectSnapshot {
        game: "haloce_evolved".to_owned(),
        source_path: kit.root.clone(),
        selected_identity: Some(new.to_owned()),
        tabs: vec![
            CampaignProjectTab {
                identity: new.to_owned(),
                label: "objects/mine/pebble.point_physics".to_owned(),
                group_tag: 0x7070_6879,
                logical_path: "objects/mine/pebble.point_physics".to_owned(),
                kind: CampaignProjectTagKind::New,
                package: Some("/Game/Tags/objects/mine/pebble-point_physics".to_owned()),
                floating: false,
            },
            CampaignProjectTab {
                identity: existing.to_owned(),
                label: "objects/rock.point_physics".to_owned(),
                group_tag: 0x7070_6879,
                logical_path: "objects/rock.point_physics".to_owned(),
                kind: CampaignProjectTagKind::Existing,
                package: None,
                floating: false,
            },
        ],
        overlays: HashMap::from([
            (
                existing.to_owned(),
                overlay(existing, CampaignProjectTagKind::Existing, &binary),
            ),
            (
                new.to_owned(),
                overlay(new, CampaignProjectTagKind::New, b"new tag bytes"),
            ),
        ]),
        history: BTreeMap::from([(
            existing.to_owned(),
            TagHistory {
                undo: vec![step(1, "Edit", b"first"), step(2, "Edit", b"second")],
                redo: vec![step(3, "Block edit", b"undone")],
                revision: 7,
            },
        )]),
        folders: BTreeSet::from(["objects/mine".to_owned(), "levels/new".to_owned()]),
    };
    let path = kit.root.join("project.baboon");

    save_campaign_project(&path, &snapshot, None, ProjectScope::Session).unwrap();
    let loaded = load_campaign_project(&path).unwrap();

    assert_eq!(loaded.game, "haloce_evolved");
    assert_eq!(loaded.source_path, kit.root);
    assert_eq!(loaded.selected_identity.as_deref(), Some(new));
    let tabs = |snapshot: &CampaignProjectSnapshot| {
        snapshot
            .tabs
            .iter()
            .map(|tab| {
                (
                    tab.identity.clone(),
                    tab.label.clone(),
                    tab.group_tag,
                    tab.logical_path.clone(),
                    tab.kind,
                    tab.package.clone(),
                    tab.floating,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(tabs(&loaded), tabs(&snapshot), "tabs, in order");
    assert_eq!(loaded.overlays.len(), 2);
    for (identity, expected) in &snapshot.overlays {
        let got = &loaded.overlays[identity];
        assert_eq!(*got.bytes, *expected.bytes, "{identity}");
        assert_eq!(got.digest, expected.digest);
        assert_eq!(got.kind, expected.kind);
        assert_eq!(got.package, expected.package);
        assert_eq!(got.logical_path, expected.logical_path);
        assert_eq!(got.group_tag, expected.group_tag);
    }
    let history = &loaded.history[existing];
    let steps = |steps: &[HistoryStep]| {
        steps
            .iter()
            .map(|step| (step.id, step.label.clone(), step.bytes.to_vec()))
            .collect::<Vec<_>>()
    };
    assert_eq!(steps(&history.undo), steps(&snapshot.history[existing].undo));
    assert_eq!(steps(&history.redo), steps(&snapshot.history[existing].redo));
    // QUIRK: the journal revision is not stored; it reads back as zero.
    assert_eq!(history.revision, 0);
    assert_eq!(loaded.folders, snapshot.folders);
    // The fingerprint covers the revision too, so it is the one difference.
    assert_eq!(loaded.fingerprint(), without_revisions(&snapshot).fingerprint());
    assert_ne!(loaded.fingerprint(), snapshot.fingerprint());

    // A mod's sidecar project carries no history.
    let sidecar = kit.root.join("sidecar.baboon");
    save_campaign_project(&sidecar, &snapshot, None, ProjectScope::ModSidecar).unwrap();
    let loaded = load_campaign_project(&sidecar).unwrap();
    assert!(loaded.history.is_empty());
    assert_eq!(loaded.overlays.len(), 2);
}

/// What the app captures: an overlay per dirty document, the open tabs, the
/// selection, each document's undo trail and the folders made this session;
/// a checkpoint writes exactly that to the recovery file.
#[test]
fn a_capture_holds_the_workspace_and_a_checkpoint_writes_it() {
    let _session = session_file_lock();
    let kit = CeKit::new("project-capture");
    let mut app = kit.app(&["objects/rock", "objects/stone"]);
    edit_field(&mut app, &key("objects/rock"), FRICTION, "0.25");
    edit_field(&mut app, &key("objects/rock"), FRICTION, "0.5");
    app.kits[0]
        .pending_container_folders
        .insert("objects/mine".to_owned());

    let snapshot = app.capture_campaign_project(0, 1.0).unwrap().expect("a snapshot");

    assert_eq!(snapshot.source_path, kit.root);
    assert_eq!(
        snapshot.overlays.keys().cloned().collect::<Vec<_>>(),
        vec![kit.identity("objects/rock")],
        "only the dirty document"
    );
    let rock = &snapshot.overlays[&kit.identity("objects/rock")];
    assert_eq!(rock.kind, CampaignProjectTagKind::Existing);
    assert_eq!(rock.package, None);
    assert_eq!(friction_in(&rock.bytes), Some(0.5));
    assert_eq!(
        *rock.bytes,
        app.kits[0].parsed_tags[&key("objects/rock")].tag.write_to_bytes().unwrap()
    );
    let open: Vec<String> = app.kits[0]
        .open_tabs
        .iter()
        .map(|key| kit.identity(&key["ublock:pakchunk0:".len()..]))
        .collect();
    assert_eq!(
        snapshot.tabs.iter().map(|tab| tab.identity.clone()).collect::<Vec<_>>(),
        open,
        "in open-tab order"
    );
    assert_eq!(
        snapshot.selected_identity.as_deref(),
        Some(kit.identity("objects/rock").as_str())
    );
    assert_eq!(snapshot.history.len(), 1, "only the edited document has history");
    let history = &snapshot.history[&kit.identity("objects/rock")];
    assert_eq!(history.undo.len(), 2);
    assert!(history.redo.is_empty());
    assert_eq!(friction_in(&history.undo[1].bytes), Some(0.25), "the state before the step");
    assert_eq!(snapshot.folders, BTreeSet::from(["objects/mine".to_owned()]));
    assert!(app.tag_has_stashed_overlay(0, &key("objects/rock")));
    assert!(!app.tag_has_stashed_overlay(0, &key("objects/stone")));
    assert_eq!(app.stashed_campaign_tags(0), vec!["objects/rock".to_owned()]);

    assert_eq!(app.checkpoint_campaign_project(0, 1.0), Ok(true));
    let written = load_campaign_project(&kit.recovery()).unwrap();
    assert_eq!(written.fingerprint(), without_revisions(&snapshot).fingerprint());
    assert_eq!(
        app.checkpoint_campaign_project(0, 2.0),
        Ok(false),
        "nothing changed, nothing written"
    );
    assert_eq!(project(&app).next_autosave_at, 2.0 + CAMPAIGN_PROJECT_AUTOSAVE_SECS);

    // A loose kit has no project to capture.
    let loose = LooseKit::new("project-capture-loose", "halo3_mcc");
    let mut app = Baboon::for_test();
    loose.install(&mut app);
    assert!(matches!(app.capture_campaign_project(0, 1.0), Ok(None)));
    assert!(app.kits[0].campaign_project.is_none());
}

/// When the autosave writes: not before it is due, once something changed,
/// never twice for the same state, and not while a write is in flight.
#[test]
fn autosave_writes_only_when_due_and_changed() {
    let _session = session_file_lock();
    let kit = CeKit::new("project-autosave");
    let mut app = kit.app(&["objects/rock"]);
    let ctx = egui::Context::default();

    autosave_at(&mut app, &ctx, 10.0);
    assert_eq!(project(&app).next_autosave_at, 10.0 + CAMPAIGN_PROJECT_AUTOSAVE_SECS);
    assert_eq!(project(&app).revision, 0, "not due yet");
    assert!(project(&app).project_path.is_none(), "autosave never names a project");

    // Due, and never written: even an empty workspace is written once.
    autosave_at(&mut app, &ctx, 11.0);
    assert_eq!(project(&app).revision, 1);
    assert_eq!(project(&app).save_in_flight, Some(1));
    pump_until(&mut app, "the autosave", |app| {
        project(app).save_in_flight.is_none()
    });
    assert!(project(&app).saved_digests.is_some());
    assert!(load_campaign_project(&kit.recovery()).unwrap().overlays.is_empty());

    // Due again, nothing changed: no write.
    autosave_at(&mut app, &ctx, 12.0);
    assert_eq!(project(&app).revision, 1);
    assert_eq!(project(&app).save_in_flight, None);
    assert_eq!(project(&app).next_autosave_at, 12.0 + CAMPAIGN_PROJECT_AUTOSAVE_SECS);

    // An edit is written at the next due tick, and not before.
    edit_field(&mut app, &key("objects/rock"), FRICTION, "0.75");
    autosave_at(&mut app, &ctx, 12.5);
    assert_eq!(project(&app).revision, 1, "not due");
    autosave_at(&mut app, &ctx, 13.0);
    assert_eq!(project(&app).revision, 2);
    pump_until(&mut app, "the second autosave", |app| {
        project(app).save_in_flight.is_none()
    });
    let written = load_campaign_project(&kit.recovery()).unwrap();
    assert_eq!(
        friction_in(&written.overlays[&kit.identity("objects/rock")].bytes),
        Some(0.75)
    );

    // While a write is in flight a due tick only pushes the next one back.
    edit_field(&mut app, &key("objects/rock"), FRICTION, "1");
    app.kits[0].campaign_project.as_mut().unwrap().save_in_flight = Some(99);
    autosave_at(&mut app, &ctx, 14.0);
    assert_eq!(project(&app).revision, 2);
    assert_eq!(project(&app).next_autosave_at, 14.0 + CAMPAIGN_PROJECT_AUTOSAVE_SECS);
    assert_eq!(
        friction_in(&project(&app).overlays[&kit.identity("objects/rock")].bytes),
        Some(0.75),
        "not even captured"
    );
    assert!(app.rx.recv_timeout(Duration::from_millis(100)).is_err());

    // A loose kit never gets a project.
    let loose = LooseKit::new("project-autosave-loose", "halo3_mcc");
    let mut app = Baboon::for_test();
    loose.install(&mut app);
    autosave_at(&mut app, &ctx, 20.0);
    assert!(app.kits[0].campaign_project.is_none());
}

/// A recovery file left by an earlier session is adopted, not overwritten:
/// the next session starts with its stash.
#[test]
fn a_new_session_adopts_the_recovery_file() {
    let _session = session_file_lock();
    let kit = CeKit::new("project-adopt");
    let mut app = kit.app(&["objects/rock"]);
    edit_field(&mut app, &key("objects/rock"), FRICTION, "0.5");
    app.checkpoint_campaign_project(0, 1.0).unwrap();

    let mut next = kit.app(&["objects/rock"]);
    next.kits[0].parsed_tags.clear();
    autosave_at(&mut next, &egui::Context::default(), 1.0);

    assert!(next.tag_has_stashed_overlay(0, &key("objects/rock")));
    assert_eq!(
        next.status,
        "Restored 1 stashed modification(s) from this workspace's last session"
    );
    assert!(project(&next).saved_digests.is_some(), "believed, so not rewritten");
    // Opening the tag serves it from the stash.
    assert!(next.load_campaign_overlay_for_key(0, &key("objects/rock")));
    assert_eq!(
        real_of(&next.kits[0].parsed_tags[&key("objects/rock")].tag, FRICTION),
        Some(0.5)
    );
}

#[test]
fn clearing_the_stash_forgets_every_overlay_and_document() {
    let _session = session_file_lock();
    let kit = CeKit::new("project-clear-stash");
    let mut app = kit.app(&["objects/rock", "objects/stone"]);
    edit_field(&mut app, &key("objects/rock"), FRICTION, "0.5");
    app.checkpoint_campaign_project(0, 1.0).unwrap();
    assert_eq!(load_campaign_project(&kit.recovery()).unwrap().overlays.len(), 1);

    app.clear_campaign_stash(0, &ctx());

    assert_eq!(app.status, "Cleared 1 stashed modification");
    assert!(project(&app).overlays.is_empty());
    assert!(app.kits[0].parsed_tags.is_empty(), "documents dropped, dirty or not");
    assert!(load_campaign_project(&kit.recovery()).unwrap().overlays.is_empty());
    // The open tabs are asked for again, from the (absent) containers.
    for path in ["objects/rock", "objects/stone"] {
        assert!(app.kits[0].loading_tags.contains(&key(path)), "{path}");
    }
    drain_messages(&mut app, Duration::from_millis(200));

    app.clear_campaign_stash(0, &ctx());
    assert_eq!(app.status, "Cleared this workspace's unsaved modifications");
}

/// Write the user's own project, named, through the app's Save Project.
fn save_user_project(app: &mut Baboon, path: &Path) {
    // The project exists from the first autosave on.
    app.capture_campaign_project(0, 1.0).unwrap();
    app.kits[0].campaign_project.as_mut().unwrap().project_path = Some(path.to_path_buf());
    app.save_campaign_project_file(0, 2.0);
    assert_eq!(
        app.status,
        format!("Saved 1 modified tag(s) to {}", path.display())
    );
}

/// "Don't Save" on a stashing workspace deletes the stashed copy from the
/// recovery file, and never touches the `.baboon` the user saved.
#[test]
fn discarding_never_writes_the_user_s_project() {
    let _session = session_file_lock();
    let kit = CeKit::new("project-discard");
    let mut app = kit.app(&["objects/rock"]);
    let rock = key("objects/rock");
    edit_field(&mut app, &rock, FRICTION, "0.5");
    app.checkpoint_campaign_project(0, 1.0).unwrap();
    let user = kit.root.join("mine.baboon");
    save_user_project(&mut app, &user);
    let user_bytes = fs::read(&user).unwrap();

    app.request_close_action(PendingCloseAction::CloseTab(rock.clone()), &ctx());
    let prompt = &app.save_changes_prompt;
    assert!(prompt.visible && prompt.can_stash);
    assert_eq!(prompt.stashed, 1);
    assert_eq!(prompt.stash_file.as_deref(), Some(kit.recovery().as_path()));

    let mut driver = PromptDriver::new();
    driver.click(&mut app, "Discard...");
    assert!(app.save_changes_prompt.confirm_discard, "the first click arms");
    assert!(app.save_changes_prompt.visible);
    assert!(app.kits[0].open_tabs.contains(&rock));
    driver.click(&mut app, "Delete Stashed Edits");

    assert!(!app.save_changes_prompt.visible);
    assert!(!app.kits[0].open_tabs.contains(&rock));
    assert!(!app.tag_has_stashed_overlay(0, &rock));
    assert!(load_campaign_project(&kit.recovery()).unwrap().overlays.is_empty());
    assert_eq!(fs::read(&user).unwrap(), user_bytes, "the user's project is untouched");
    assert_eq!(load_campaign_project(&user).unwrap().overlays.len(), 1);
}

/// "Stash for Mod" keeps the edit in the recovery file and closes; the
/// user's project keeps what was last saved into it.
#[test]
fn stashing_for_mod_keeps_the_edit_out_of_the_user_s_project() {
    let _session = session_file_lock();
    let kit = CeKit::new("project-stash");
    let mut app = kit.app(&["objects/rock"]);
    let rock = key("objects/rock");
    edit_field(&mut app, &rock, FRICTION, "0.5");
    let user = kit.root.join("mine.baboon");
    save_user_project(&mut app, &user);
    let user_bytes = fs::read(&user).unwrap();
    edit_field(&mut app, &rock, FRICTION, "0.75");

    app.request_close_action(PendingCloseAction::CloseTab(rock.clone()), &ctx());
    PromptDriver::new().click(&mut app, "Stash for Mod");

    assert_eq!(
        app.status,
        format!(
            "Stashed for Export Mod. {} is unchanged until you save it",
            user.display()
        )
    );
    assert!(!app.save_changes_prompt.visible);
    assert!(!app.kits[0].open_tabs.contains(&rock), "the close went ahead");
    let stashed = load_campaign_project(&kit.recovery()).unwrap();
    assert_eq!(
        friction_in(&stashed.overlays[&kit.identity("objects/rock")].bytes),
        Some(0.75)
    );
    assert_eq!(fs::read(&user).unwrap(), user_bytes);
    assert_eq!(
        friction_in(
            &load_campaign_project(&user).unwrap().overlays[&kit.identity("objects/rock")].bytes
        ),
        Some(0.5)
    );
}

/// The prompt's Save routes a container tag to the in-place overwrite rather
/// than the loose-file writer. With its container not mounted the overwrite
/// refuses before taking a lease, and the prompt stays up saying why.
#[test]
fn the_prompt_s_save_routes_a_container_tag_into_its_pak() {
    let _session = session_file_lock();
    let kit = CeKit::new("project-prompt-save");
    let mut app = kit.app(&["objects/rock"]);
    let rock = key("objects/rock");
    edit_field(&mut app, &rock, FRICTION, "0.5");
    app.request_close_action(PendingCloseAction::CloseTab(rock.clone()), &ctx());

    PromptDriver::new().click(&mut app, "Save");

    let prompt = &app.save_changes_prompt;
    assert!(prompt.visible, "the save failed, so the prompt stays");
    assert_eq!(
        prompt.error.as_deref(),
        Some("Save failed: objects/rock.point_physics: Container provenance is stale")
    );
    assert!(app.container_write_leases.is_empty(), "no lease was taken");
    assert!(app.kits[0].open_tabs.contains(&rock));
    assert!(app.kits[0].parsed_tags[&rock].dirty.is_set());
    assert!(fs::read_dir(&kit.root).unwrap().next().is_none(), "nothing written");
}

/// The export review lists every stashed tag, and what the writer does with
/// a selection it cannot build.
#[test]
fn the_export_review_lists_the_stash_and_refuses_what_it_cannot_write() {
    let _session = session_file_lock();
    let kit = CeKit::new("project-export");
    let mut app = kit.app(&["objects/rock"]);
    edit_field(&mut app, &key("objects/rock"), FRICTION, "0.5");
    app.capture_campaign_project(0, 1.0).unwrap();
    let orphan = "70706879:objects/gone";
    app.kits[0]
        .campaign_project
        .as_mut()
        .unwrap()
        .overlays
        .insert(
            orphan.to_owned(),
            overlay(orphan, CampaignProjectTagKind::Existing, b"orphaned"),
        );

    app.review_changes();
    let review = app.mod_export.as_ref().expect("the review opened");
    assert!(review.review_only);
    assert!(!kit.root.join("~mods").exists(), "a review creates nothing");

    app.export_mod();
    let dialog = app.mod_export.as_ref().expect("the export opened");
    assert!(!dialog.review_only);
    assert_eq!(dialog.name, "mymod");
    assert_eq!(dialog.folder, kit.root.join("~mods"));
    assert!(dialog.folder.is_dir(), "the default destination is made");
    let mut rows: Vec<_> = dialog.rows.iter().collect();
    rows.sort_by(|a, b| a.identity.cmp(&b.identity));
    let rock = rows
        .iter()
        .find(|row| row.identity == kit.identity("objects/rock"))
        .expect("the edited tag");
    assert!(matches!(rock.kind, ModExportChange::Modified));
    assert!(rock.include);
    assert_eq!(rock.reason, None);
    assert_eq!(rock.display_path, "objects/rock");
    let gone = rows.iter().find(|row| row.identity == orphan).expect("the orphan");
    assert!(matches!(gone.kind, ModExportChange::Unresolved));
    assert!(!gone.include, "an unresolved tag is not offered");
    assert_eq!(gone.reason.as_deref(), Some("not in this source"));
    assert_eq!(dialog.rows.len(), 2);

    let snapshot = app.mod_export.as_ref().unwrap().snapshot.clone();
    let output = kit.root.join("~mods/mymod_P.utoc");
    let write = |app: &mut Baboon, included: &[&str], output: &Path| {
        let included = included.iter().map(|id| (*id).to_owned()).collect();
        app.write_reviewed_mod(&snapshot, &included, output.to_path_buf(), &ctx());
        app.status.clone()
    };
    assert_eq!(write(&mut app, &[], &output), "Nothing selected to export");
    assert_eq!(write(&mut app, &[orphan], &output), "Nothing selected to export");
    // An existing tag whose container is not mounted is passed over the same
    // way (QUIRK: reported as nothing selected rather than as unwritable).
    let rock_identity = kit.identity("objects/rock");
    assert_eq!(
        write(&mut app, &[rock_identity.as_str()], &output),
        "Nothing selected to export"
    );
    assert!(!output.exists());
    // A destination whose folder cannot be made is refused first.
    fs::write(kit.root.join("blocker"), b"").unwrap();
    let status = write(&mut app, &[rock_identity.as_str()], &kit.root.join("blocker/x.utoc"));
    assert!(status.starts_with("Could not create "), "{status}");

    // And a loose kit is not a Campaign Evolved source at all.
    let loose = LooseKit::new("project-export-loose", "halo3_mcc");
    let mut app = Baboon::for_test();
    loose.install(&mut app);
    assert_eq!(
        write(&mut app, &[rock_identity.as_str()], &loose.base.join("out/x.utoc")),
        "Export Mod is only for Campaign Evolved containers"
    );
    app.export_mod();
    assert!(app.mod_export.is_none());
    assert_eq!(app.status, "Export Mod is only for Campaign Evolved containers");
}
