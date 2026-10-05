//! Synthetic loose editing kits for the characterization tests.
//!
//! Real tags, built from this repository's own definitions and written into a
//! temporary `<EK>/tags` folder, then installed as a loose source the way a
//! folder load installs one. The save, close, browser and refactor flows then
//! run against files a test can read back, without any editing kit installed.

use super::*;
use crate::app::references::build_reverse_dependency_index;
use crate::core::document::apply::{DeferredOps, UndoStep};
use blam_tags::fields::{TagFieldData, TagReferenceData};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Held by every test that writes or reads the per-process last-session file
/// or the shared prefs, which all tests in one run share.
pub(in crate::app) static SESSION_FILE: Mutex<()> = Mutex::new(());

pub(in crate::app) fn session_file_lock() -> std::sync::MutexGuard<'static, ()> {
    SESSION_FILE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(in crate::app) fn definition(game: &str, group: &str) -> PathBuf {
    locate_definitions_root()
        .join(game)
        .join(format!("{group}.json"))
}

/// A group's tag, read from its definition rather than spelled out, so a
/// fixture cannot disagree with the schema it was built from.
pub(in crate::app) fn group_tag(game: &str, group: &str) -> u32 {
    let text = fs::read_to_string(definition(game, group)).expect("read the group's definition");
    let value: Value = serde_json::from_str(&text).expect("parse the group's definition");
    let tag = value
        .get("tag")
        .and_then(Value::as_str)
        .expect("the definition names its group tag");
    let mut bytes = [b' '; 4];
    bytes[..tag.len()].copy_from_slice(tag.as_bytes());
    u32::from_be_bytes(bytes)
}

fn ek_folder(game: &str) -> &'static str {
    match game {
        "haloce_mcc" => "HCEEK",
        "halo2_mcc" => "H2EK",
        "halo3_mcc" => "H3EK",
        "haloreach_mcc" => "HREK",
        other => panic!("no fixture editing kit for {other}"),
    }
}

/// Set a root-level tag reference.
pub(in crate::app) fn set_reference(tag: &mut TagFile, field: &str, group: u32, path: &str) {
    tag.root_mut()
        .field_mut(field)
        .unwrap_or_else(|| panic!("no `{field}` field"))
        .set(TagFieldData::TagReference(TagReferenceData {
            group_tag_and_name: Some((group, path.to_owned())),
        }))
        .unwrap_or_else(|error| panic!("set `{field}`: {error:?}"));
}

/// A root-level tag reference's path, as the tag holds it.
pub(in crate::app) fn reference_of(tag: &TagFile, field: &str) -> Option<String> {
    match tag.root().field_path(field)?.value()? {
        TagFieldData::TagReference(reference) => {
            reference.group_tag_and_name.map(|(_, path)| path)
        }
        _ => None,
    }
}

/// A root-level real's value.
pub(in crate::app) fn real_of(tag: &TagFile, field: &str) -> Option<f32> {
    match tag.root().field_path(field)?.value()? {
        TagFieldData::Real(value) => Some(value),
        _ => None,
    }
}

/// A classic Halo CE tag of `group` with every body field zeroed: the 64-byte
/// header and a body exactly as long as the group's root struct reads.
pub(in crate::app) fn classic_ce_bytes(group: &str) -> Vec<u8> {
    let tag = group_tag("haloce_mcc", group);
    let mut bytes = vec![0u8; 64];
    bytes[36..40].copy_from_slice(&tag.to_be_bytes());
    bytes[56..58].copy_from_slice(&1u16.to_be_bytes());
    bytes[60..64].copy_from_slice(b"blam");
    let layout = || {
        blam_tags::layout::TagLayout::from_json(definition("haloce_mcc", group))
            .expect("read the CE layout")
    };
    // Sized by the reader itself: a body longer than the layout walks reports
    // how much it consumed.
    let mut probe = bytes.clone();
    probe.extend(std::iter::repeat_n(0u8, 4096));
    let consumed = match blam_tags::classic::read_classic_tag_file(&probe, layout()) {
        Err(blam_tags::classic::ClassicError::TrailingBytes { consumed, .. }) => consumed,
        other => panic!("sizing a zeroed {group} body: {:?}", other.err()),
    };
    bytes.extend(std::iter::repeat_n(0u8, consumed));
    blam_tags::classic::read_classic_tag_file(&bytes, layout()).expect("the zeroed tag reads");
    bytes
}

/// A fresh context, for calls that only need one to hand.
pub(in crate::app) fn ctx() -> egui::Context {
    egui::Context::default()
}

/// Apply worker messages as frames would, until `done` holds.
pub(in crate::app) fn pump_until(app: &mut Baboon, what: &str, mut done: impl FnMut(&Baboon) -> bool) {
    // Generous, because the wait is for a real worker job: on a loaded machine
    // or a shared CI runner the full suite runs several times slower, and a
    // 30-second budget timed out under load while passing on its own.
    let deadline = Instant::now() + Duration::from_secs(180);
    let ctx = ctx();
    while !done(app) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; status: {}",
            app.model.status
        );
        if let Ok(message) = app.rx.recv_timeout(Duration::from_millis(50)) {
            app.apply_worker_message(message, &ctx);
            app.process_worker_messages(&ctx);
        }
    }
}

/// Apply whatever worker messages arrive within `quiet` of each other.
pub(in crate::app) fn drain_messages(app: &mut Baboon, quiet: Duration) {
    let ctx = ctx();
    while let Ok(message) = app.rx.recv_timeout(quiet) {
        app.apply_worker_message(message, &ctx);
        app.process_worker_messages(&ctx);
    }
}

/// A temporary `<EK>/tags` folder for one game, removed on drop.
pub(in crate::app) struct LooseKit {
    pub(in crate::app) base: PathBuf,
    pub(in crate::app) root: PathBuf,
    pub(in crate::app) game: &'static str,
}

impl LooseKit {
    pub(in crate::app) fn new(name: &str, game: &'static str) -> Self {
        // Canonical, so a key made from this path agrees with one made from
        // its canonical form (the temp dir is `/var` -> `/private/var` on
        // macOS); a real kit's root has no such alias.
        let base = fs::canonicalize(crate::core::test_kits::unique_temp_dir(name)).unwrap();
        let root = base.join(ek_folder(game)).join("tags");
        fs::create_dir_all(&root).unwrap();
        Self { base, root, game }
    }

    pub(in crate::app) fn names(&self) -> TagNameIndex {
        TagNameIndex::load_game(&locate_definitions_root(), GameId::from_id(self.game).unwrap()).expect("load group names")
    }

    /// Write an MCC tag of `group` at `rel` (no extension), shaped by `edit`.
    pub(in crate::app) fn write_mcc(
        &self,
        rel: &str,
        group: &str,
        edit: impl FnOnce(&mut TagFile),
    ) -> PathBuf {
        let mut tag = TagFile::new(definition(self.game, group))
            .unwrap_or_else(|error| panic!("build a {group}: {error}"));
        edit(&mut tag);
        let path = self.root.join(format!("{rel}.{group}"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        tag.write(&path).unwrap();
        path
    }

    /// Write a zeroed classic Halo CE tag of `group` at `rel`.
    pub(in crate::app) fn write_classic_ce(&self, rel: &str, group: &str) -> PathBuf {
        assert_eq!(self.game, "haloce_mcc");
        let path = self.root.join(format!("{rel}.{group}"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, classic_ce_bytes(group)).unwrap();
        path
    }

    pub(in crate::app) fn entries(&self) -> Vec<TagEntry> {
        crate::core::source::scan_folder_subtree_entries(&self.root, Path::new(""), &self.names())
            .expect("scan the fixture")
    }

    /// The key the folder scan gives the tag at `rel_with_extension`.
    pub(in crate::app) fn key(&self, rel_with_extension: &str) -> String {
        loose_file_entry(&self.root, &self.root.join(rel_with_extension), &self.names())
            .unwrap()
            .unwrap_or_else(|| panic!("{rel_with_extension} does not probe as a tag"))
            .key
    }

    /// The source a completed folder load of this kit would hand over.
    pub(in crate::app) fn source(&self) -> LoadedSourceData {
        let entries = self.entries();
        LoadedSourceData {
            label: "fixture".to_owned(),
            source: TagSource::LooseFolder {
                root: self.root.clone(),
                game: GameId::from_id(self.game),
                definitions_root: locate_definitions_root(),
            },
            names: self.names(),
            game: GameId::from_id(self.game),
            entries: entries.clone(),
            tree: crate::core::source::build_folder_directory_tree(&self.root).unwrap(),
            group_tree: crate::core::source::build_group_tree(&entries),
            all_entries: entries,
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: true,
            chosen_kit_layout: None,
        }
    }

    pub(in crate::app) fn install(&self, app: &mut Baboon) {
        app.install_loaded_source(self.source());
    }

    /// Open the tag at `rel_with_extension` and wait for its document.
    pub(in crate::app) fn open(&self, app: &mut Baboon, rel_with_extension: &str) -> String {
        let key = self.key(rel_with_extension);
        app.select_entry(key.clone(), ctx());
        pump_until(app, &format!("{rel_with_extension} to load"), |app| {
            app.model.kits[app.model.active].parsed_tags.contains_key(&key)
        });
        key
    }
}

impl Drop for LooseKit {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// Edit one field of an open document through the editor's own entry point,
/// as one undo step.
pub(in crate::app) fn edit_field(app: &mut Baboon, key: &str, path: &str, input: &str) {
    let ops = DeferredOps {
        pending: vec![PendingFieldEdit {
            path: path.to_owned(),
            input: input.to_owned(),
        }],
        ..DeferredOps::default()
    };
    let active = app.model.active;
    let applied = app
        .apply_doc_ops(active, key, "Edit", ops, UndoStep::Own)
        .expect("the document is open");
    for outcome in &applied.outcomes {
        assert!(outcome.result.is_ok(), "edit {path}: {:?}", outcome.result);
    }
}

pub(in crate::app) fn app() -> Baboon {
    Baboon::for_test()
}

impl LooseKit {
    /// The reference index a full build over this kit makes.
    pub(in crate::app) fn index(&self) -> ReverseDependencyIndex {
        let (tx, _rx) = std::sync::mpsc::channel();
        let source = TagSource::LooseFolder {
            root: self.root.clone(),
            game: GameId::from_id(self.game),
            definitions_root: locate_definitions_root(),
        };
        build_reverse_dependency_index(&self.root, &source, &self.entries(), "Indexing", &tx)
    }

    /// [`Self::install`], with the reference index already built.
    pub(in crate::app) fn install_indexed(&self, app: &mut Baboon) {
        let mut source = self.source();
        source.reverse_dependencies = Some(self.index());
        app.install_loaded_source(source);
    }
}

/// Input for one frame on a 1000x800 screen at `time`.
pub(in crate::app) fn screen(events: Vec<egui::Event>, time: f64) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 800.0),
        )),
        time: Some(time),
        events,
        ..Default::default()
    }
}

/// How long egui may wait before the next frame, once `before_frame` has
/// been applied just ahead of one drawn by `draw`. Frames run on an advancing
/// clock first, so egui's own start-up repaints and the scrollbars fading in
/// have settled.
pub(in crate::app) fn repaint_delay_after(
    app: &mut Baboon,
    draw: impl Fn(&mut Baboon, &mut egui::Ui),
    before_frame: impl FnOnce(&mut Baboon),
) -> Duration {
    let ctx = ctx();
    Baboon::configure_context(&ctx);
    let frame = |app: &mut Baboon, number: u32| {
        let input = screen(Vec::new(), f64::from(number) / 60.0);
        let output = crate::app::run_ui_test(&ctx, input, |ui| draw(app, ui));
        output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
    };
    for number in 0..30 {
        frame(app, number);
    }
    before_frame(app);
    frame(app, 30)
}

/// Draws the save prompt frame after frame, with an advancing clock.
pub(in crate::app) struct PromptDriver {
    pub(in crate::app) ctx: egui::Context,
    pub(in crate::app) time: f64,
    /// Every command the frames sent the root viewport.
    pub(in crate::app) commands: Vec<egui::ViewportCommand>,
}

impl PromptDriver {
    pub(in crate::app) fn new() -> Self {
        Self::on(egui::Context::default(), 1.0)
    }

    pub(in crate::app) fn on(ctx: egui::Context, time: f64) -> Self {
        Self {
            ctx,
            time,
            commands: Vec::new(),
        }
    }

    pub(in crate::app) fn frame(
        &mut self,
        app: &mut Baboon,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Rect)> {
        self.time += 0.1;
        let output = crate::app::run_ui_test(&self.ctx, screen(events, self.time), |ui| {
            app.dialogs.draw(&cx!(app, ui.ctx()), &app_reads!(app));
            // What the prompt answered runs once drawing is over, inside the
            // same pass as in a frame, so a close it re-issues is in this
            // pass's output.
            app.apply_commands(ui.ctx());
        });
        self.commands.extend(root_commands(&output));
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }

    /// Where the button labelled exactly `label` is drawn now.
    pub(in crate::app) fn find(
        &mut self,
        app: &mut Baboon,
        label: &str,
        events: Vec<egui::Event>,
    ) -> egui::Pos2 {
        let labels = self.frame(app, events);
        labels
            .iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("no `{label}` button drawn; drew {labels:?}"))
            .1
            .center()
    }

    /// Click the prompt button labelled exactly `label`: slide onto it over a
    /// few frames, press, release.
    pub(in crate::app) fn click(&mut self, app: &mut Baboon, label: &str) {
        // A window lays itself out unseen on its first frame.
        self.frame(app, Vec::new());
        let mut pos = self.find(app, label, Vec::new());
        for step in [3.0, 2.0, 1.0, 0.0] {
            let at = pos + egui::vec2(step, 0.0);
            pos = self.find(app, label, vec![egui::Event::PointerMoved(at)]);
        }
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(app, vec![button(true)]);
        self.frame(app, vec![button(false)]);
    }
}

/// The commands a frame sent the root viewport.
pub(in crate::app) fn root_commands(output: &egui::FullOutput) -> Vec<egui::ViewportCommand> {
    output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|viewport| viewport.commands.clone())
        .unwrap_or_default()
}
