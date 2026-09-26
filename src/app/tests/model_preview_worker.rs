//! The model preview parses on a worker, not the UI thread.
//!
//! What the app derives, frame by frame: the post-draw hook starts a worker
//! and leaves the state without data (the loading shells), the worker's
//! message installs the result, and a result for a request the state no
//! longer wants is dropped instead of installed. Both a modern (Halo 3) and a
//! classic (Halo 2) tag must load, from disk and from an edited document's
//! bytes: a classic tag's bytes carry no layout, so re-parsing them the
//! modern way fails where reading them the kit's way does not.
//!
//! Needs `BLAM_TEST_H3EK` / `BLAM_TEST_H2EK`; skips a kit that is not set.

use std::path::Path;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::app::state::ModelTagPanelTab;
use crate::app::{Baboon, LoadedSourceData, ModelPreviewState, TagDocument};
use crate::format::TagNameIndex;
use crate::source::{TagEntry, TagEntryLocation, TagSource, TagTree};

struct Fixture {
    app: Baboon,
    key: String,
    ctx: egui::Context,
}

fn fixture(tags: &Path, game: &str, rel: &str) -> Option<Fixture> {
    let path = tags.join(rel);
    if !path.is_file() {
        eprintln!("skipping: {} is not present", path.display());
        return None;
    }
    let definitions = crate::test_kits::definitions();
    let entry = TagEntry {
        key: format!("file:{}", path.display()),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"mode"),
        group_name: Some("render_model".to_owned()),
        location: TagEntryLocation::LooseFile(path.clone()),
    };
    let mut app = Baboon::for_test();
    app.install_loaded_source(LoadedSourceData {
        label: game.to_owned(),
        source: TagSource::LooseFolder {
            root: tags.to_path_buf(),
            game: Some(game.to_owned()),
            definitions_root: definitions.to_path_buf(),
        },
        names: TagNameIndex::load_from_definitions(definitions),
        game: Some(game.to_owned()),
        entries: vec![entry.clone()],
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: vec![entry.clone()],
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: true,
    });
    let preview = ModelPreviewState {
        active_tab: ModelTagPanelTab::ModelPreview,
        ..ModelPreviewState::default()
    };
    app.kits[0].model_previews.insert(entry.key.clone(), preview);
    Some(Fixture {
        app,
        key: entry.key,
        ctx: egui::Context::default(),
    })
}

impl Fixture {
    fn state(&self) -> &ModelPreviewState {
        &self.app.kits[0].model_previews[&self.key]
    }

    fn state_mut(&mut self) -> &mut ModelPreviewState {
        self.app.kits[0].model_previews.get_mut(&self.key).unwrap()
    }

    /// One frame's worth of preview work: drain replies, then the post-draw hook.
    fn frame(&mut self) {
        self.app.process_worker_messages(&self.ctx);
        let key = self.key.clone();
        self.app.maybe_request_model_preview(0, &key, &self.ctx);
    }

    /// Run frames until the state holds a preview it considers current.
    fn frames_until_loaded(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.state().needs_preview_load(&self.key) {
            assert!(Instant::now() < deadline, "the preview never landed");
            self.frame();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Open the tag as an edited document, so the worker parses its bytes.
    fn open_edited(&mut self) {
        let entry = self.app.kits[0].entry_for_key(&self.key).unwrap().clone();
        let source = self.app.kits[0].source.as_ref().unwrap().source.clone();
        let tag = crate::source::read_entry(&source, &entry).expect("read render_model");
        let mut document = TagDocument::clean(tag);
        document.dirty.touch();
        self.app.kits[0]
            .parsed_tags
            .insert(self.key.clone(), document);
    }
}

fn loads_on_a_worker(fixture: &mut Fixture) {
    fixture.frame();
    assert!(
        fixture.state().data.is_none() && fixture.state().preview_load_id.is_some(),
        "the first frame must hand the parse to a worker and show the shells"
    );
    fixture.frames_until_loaded();

    let data = fixture
        .state()
        .data
        .as_ref()
        .unwrap()
        .as_ref()
        .expect("preview loads");
    assert!(!data.preview.batches.is_empty(), "no draw batches");
}

fn check(tags: &Path, game: &str, rel: &str) {
    for edited in [false, true] {
        if let Some(mut fixture) = fixture(tags, game, rel) {
            if edited {
                fixture.open_edited();
            }
            loads_on_a_worker(&mut fixture);
        }
    }
}

#[test]
fn a_halo3_render_model_loads_on_a_worker() {
    check(
        &crate::test_kits::h3ek_tags(),
        "halo3_mcc",
        "objects/weapons/rifle/assault_rifle/assault_rifle.render_model",
    );
}

#[test]
fn a_classic_halo2_render_model_loads_on_a_worker() {
    check(
        &crate::test_kits::h2ek_tags(),
        "halo2_mcc",
        "objects/weapons/rifle/battle_rifle/battle_rifle.render_model",
    );
}

/// Wait for the worker's reply without handing it to the app.
fn wait_for_reply(fixture: &Fixture) -> crate::app::WorkerMessage {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(message) = fixture.app.rx.try_recv() {
            return message;
        }
        assert!(Instant::now() < deadline, "the worker never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A setting changed while the worker ran: its result answers a request the
/// state no longer makes, so it must be dropped and the load re-run — not
/// installed as current and left for a later frame to notice.
#[test]
fn a_result_for_a_superseded_request_is_dropped() {
    let tags = crate::test_kits::h3ek_tags();
    let rel = "objects/weapons/rifle/assault_rifle/assault_rifle.render_model";
    let Some(mut fixture) = fixture(&tags, "halo3_mcc", rel) else {
        return;
    };
    assert!(fixture.state().high_detail);
    fixture.frame();
    let first = fixture.state().preview_load_id.expect("a worker started");
    // Let the superseded worker finish, so dropping its result is a choice
    // the app makes rather than a race it happened to win.
    let reply = wait_for_reply(&fixture);

    fixture.state_mut().high_detail = false;
    let key = fixture.key.clone();
    fixture.app.maybe_request_model_preview(0, &key, &fixture.ctx);
    let second = fixture.state().preview_load_id.expect("a new worker started");
    assert_ne!(first, second);
    fixture.app.tx.send(reply).unwrap();
    fixture.app.process_worker_messages(&fixture.ctx);
    assert!(
        fixture.state().data.is_none(),
        "the superseded result was installed as the preview"
    );

    fixture.frames_until_loaded();
    assert!(!fixture.state().loaded_high_detail);
}

/// An edit invalidates the preview while a load is in flight: the worker is
/// parsing the bytes from before the edit, so its result must not land.
#[test]
fn invalidating_drops_the_load_in_flight() {
    let tags = crate::test_kits::h3ek_tags();
    let rel = "objects/weapons/rifle/assault_rifle/assault_rifle.render_model";
    let Some(mut fixture) = fixture(&tags, "halo3_mcc", rel) else {
        return;
    };
    fixture.frame();
    let reply = wait_for_reply(&fixture);
    fixture.state_mut().invalidate_load();
    fixture.app.tx.send(reply).unwrap();
    fixture.app.process_worker_messages(&fixture.ctx);
    assert!(
        fixture.state().data.is_none(),
        "the pre-edit load landed after the edit"
    );
}
