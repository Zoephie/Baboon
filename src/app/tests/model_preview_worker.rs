//! The model preview parses on a worker, not the UI thread.
//!
//! What the panel derives, frame by frame: the first frame starts a worker and
//! leaves the state without data (the spinner), a later frame installs the
//! worker's result, and a result for a request the state no longer wants is
//! dropped instead of installed. Both a modern (Halo 3) and a classic (Halo 2)
//! tag must make the serialize/re-parse round trip on the worker; a classic tag
//! that silently fell back to the UI thread would still preview, so the test
//! counts the loads the UI thread ran.
//!
//! Needs `BLAM_TEST_H3EK` / `BLAM_TEST_H2EK`; skips a kit that is not set.

use std::path::Path;
use std::time::{Duration, Instant};

use eframe::egui;

use super::{UI_THREAD_LOADS, ensure_model_preview_loaded};
use crate::app::ModelPreviewState;
use crate::source::{TagEntry, TagEntryLocation, TagSource};

struct Fixture {
    tag: blam_tags::TagFile,
    entry: TagEntry,
    source: TagSource,
    names: crate::format::TagNameIndex,
}

fn fixture(tags: &Path, game: &str, rel: &str) -> Option<Fixture> {
    let path = tags.join(rel);
    if !path.is_file() {
        eprintln!("skipping: {} is not present", path.display());
        return None;
    }
    let definitions = crate::test_kits::definitions();
    let group_tag = u32::from_be_bytes(*b"mode");
    let tag = crate::source::read_tag_at_path(&path, Some(game), Some(definitions), group_tag)
        .expect("read render_model");
    Some(Fixture {
        entry: TagEntry {
            key: format!("file:{}", path.display()),
            display_path: rel.to_owned(),
            group_tag,
            group_name: Some("render_model".to_owned()),
            location: TagEntryLocation::LooseFile(path.clone()),
        },
        source: TagSource::LooseFolder {
            root: tags.to_path_buf(),
            game: Some(game.to_owned()),
            definitions_root: definitions.to_path_buf(),
        },
        names: crate::format::TagNameIndex::load_from_definitions(definitions),
        tag,
    })
}

impl Fixture {
    fn frame(&self, state: &mut ModelPreviewState, ctx: &egui::Context) {
        ensure_model_preview_loaded(
            &self.tag,
            &self.entry,
            &self.names,
            Some(&self.source),
            state,
            ctx,
        );
    }

    /// Run frames until the state holds a preview it considers current.
    fn frames_until_loaded(&self, state: &mut ModelPreviewState, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while state.needs_preview_load(&self.entry.key) {
            assert!(Instant::now() < deadline, "the preview never landed");
            self.frame(state, ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn loads_on_a_worker(fixture: &Fixture) {
    let ctx = egui::Context::default();
    let mut state = ModelPreviewState::default();
    UI_THREAD_LOADS.with(|count| count.set(0));

    fixture.frame(&mut state, &ctx);
    assert!(
        state.data.is_none() && state.loading.is_some(),
        "the first frame must hand the parse to a worker and show the spinner"
    );
    fixture.frames_until_loaded(&mut state, &ctx);

    let data = state
        .data
        .as_ref()
        .unwrap()
        .as_ref()
        .expect("preview loads");
    assert!(!data.preview.batches.is_empty(), "no draw batches");
    assert_eq!(
        UI_THREAD_LOADS.with(|count| count.get()),
        0,
        "the worker could not re-parse the tag, so the UI thread parsed it"
    );
}

#[test]
fn a_halo3_render_model_loads_on_a_worker() {
    let tags = crate::test_kits::h3ek_tags();
    let rel = "objects/weapons/rifle/assault_rifle/assault_rifle.render_model";
    if let Some(fixture) = fixture(&tags, "halo3_mcc", rel) {
        loads_on_a_worker(&fixture);
    }
}

#[test]
fn a_classic_halo2_render_model_loads_on_a_worker() {
    let tags = crate::test_kits::h2ek_tags();
    let rel = "objects/weapons/rifle/battle_rifle/battle_rifle.render_model";
    if let Some(fixture) = fixture(&tags, "halo2_mcc", rel) {
        loads_on_a_worker(&fixture);
    }
}

/// A setting changed while the worker ran: its result answers a request the
/// state no longer makes, so it must be dropped and the load re-run — not
/// installed as current and left for a later frame to notice.
#[test]
fn a_result_for_a_superseded_request_is_dropped() {
    let tags = crate::test_kits::h3ek_tags();
    let rel = "objects/weapons/rifle/assault_rifle/assault_rifle.render_model";
    let Some(fixture) = fixture(&tags, "halo3_mcc", rel) else {
        return;
    };
    let ctx = egui::Context::default();
    let mut state = ModelPreviewState::default();
    assert!(state.high_detail);
    fixture.frame(&mut state, &ctx);
    let first = state.loading.as_ref().expect("a worker started");
    // Let the superseded worker finish, so dropping its result is a choice
    // the loader makes rather than a race it happened to win.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match first.receiver.try_recv() {
            Ok(outcome) => {
                let (sender, receiver) = std::sync::mpsc::channel();
                sender.send(outcome).unwrap();
                state.loading.as_mut().unwrap().receiver = receiver;
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "the worker never finished");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("worker hung up: {error}"),
        }
    }

    state.high_detail = false;
    fixture.frame(&mut state, &ctx);
    assert!(
        state.data.is_none(),
        "the superseded result was installed as the preview"
    );
    let pending = state.loading.as_ref().expect("a new worker started");
    assert!(!pending.high_detail, "the new worker runs the new request");

    fixture.frames_until_loaded(&mut state, &ctx);
    assert!(!state.loaded_high_detail);
}

/// An edit invalidates the preview while a load is in flight: the worker is
/// parsing the bytes from before the edit, so its result must not land.
#[test]
fn invalidating_drops_the_load_in_flight() {
    let tags = crate::test_kits::h3ek_tags();
    let rel = "objects/weapons/rifle/assault_rifle/assault_rifle.render_model";
    let Some(fixture) = fixture(&tags, "halo3_mcc", rel) else {
        return;
    };
    let ctx = egui::Context::default();
    let mut state = ModelPreviewState::default();
    fixture.frame(&mut state, &ctx);
    assert!(state.loading.is_some());
    state.invalidate_load();
    assert!(
        state.loading.is_none(),
        "the pre-edit load is still pending"
    );
}
