//! Synthetic loose editing kits for the characterization tests.
//!
//! Real tags, built from this repository's own definitions and written into a
//! temporary `<EK>/tags` folder, then installed as a loose source the way a
//! folder load installs one. The save, close, browser and refactor flows then
//! run against files a test can read back, without any editing kit installed.

use super::*;
use blam_tags::fields::{TagFieldData, TagReferenceData};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Held by every test that writes or reads the per-process last-session file
/// or the shared prefs, which all tests in one run share.
pub(super) static SESSION_FILE: Mutex<()> = Mutex::new(());

pub(super) fn session_file_lock() -> std::sync::MutexGuard<'static, ()> {
    SESSION_FILE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) fn definition(game: &str, group: &str) -> PathBuf {
    locate_definitions_root()
        .join(game)
        .join(format!("{group}.json"))
}

/// A group's tag, read from its definition rather than spelled out, so a
/// fixture cannot disagree with the schema it was built from.
pub(super) fn group_tag(game: &str, group: &str) -> u32 {
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
pub(super) fn set_reference(tag: &mut TagFile, field: &str, group: u32, path: &str) {
    tag.root_mut()
        .field_mut(field)
        .unwrap_or_else(|| panic!("no `{field}` field"))
        .set(TagFieldData::TagReference(TagReferenceData {
            group_tag_and_name: Some((group, path.to_owned())),
        }))
        .unwrap_or_else(|error| panic!("set `{field}`: {error:?}"));
}

/// A root-level tag reference's path, as the tag holds it.
pub(super) fn reference_of(tag: &TagFile, field: &str) -> Option<String> {
    match tag.root().field_path(field)?.value()? {
        TagFieldData::TagReference(reference) => {
            reference.group_tag_and_name.map(|(_, path)| path)
        }
        _ => None,
    }
}

/// A root-level real's value.
pub(super) fn real_of(tag: &TagFile, field: &str) -> Option<f32> {
    match tag.root().field_path(field)?.value()? {
        TagFieldData::Real(value) => Some(value),
        _ => None,
    }
}

/// A classic Halo CE tag of `group` with every body field zeroed: the 64-byte
/// header and a body exactly as long as the group's root struct reads.
pub(super) fn classic_ce_bytes(group: &str) -> Vec<u8> {
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

/// Make the next pump's frames see a later clock than the last.
pub(super) fn ctx() -> egui::Context {
    egui::Context::default()
}

/// Apply worker messages as frames would, until `done` holds.
pub(super) fn pump_until(app: &mut Baboon, what: &str, mut done: impl FnMut(&Baboon) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let ctx = ctx();
    while !done(app) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; status: {}",
            app.status
        );
        if let Ok(message) = app.rx.recv_timeout(Duration::from_millis(50)) {
            app.tx.send(message).unwrap();
            app.process_worker_messages(&ctx);
        }
    }
}

/// Apply whatever worker messages arrive within `quiet` of each other.
pub(super) fn drain_messages(app: &mut Baboon, quiet: Duration) {
    let ctx = ctx();
    while let Ok(message) = app.rx.recv_timeout(quiet) {
        app.tx.send(message).unwrap();
        app.process_worker_messages(&ctx);
    }
}

/// A temporary `<EK>/tags` folder for one game, removed on drop.
pub(super) struct LooseKit {
    pub(super) base: PathBuf,
    pub(super) root: PathBuf,
    pub(super) game: &'static str,
}

impl LooseKit {
    pub(super) fn new(name: &str, game: &'static str) -> Self {
        // Canonical, so a key made from this path agrees with one made from
        // its canonical form (the temp dir is `/var` -> `/private/var` on
        // macOS); a real kit's root has no such alias.
        let base = fs::canonicalize(crate::test_kits::unique_temp_dir(name)).unwrap();
        let root = base.join(ek_folder(game)).join("tags");
        fs::create_dir_all(&root).unwrap();
        Self { base, root, game }
    }

    pub(super) fn names(&self) -> TagNameIndex {
        TagNameIndex::load_game(&locate_definitions_root(), self.game).expect("load group names")
    }

    /// Write an MCC tag of `group` at `rel` (no extension), shaped by `edit`.
    pub(super) fn write_mcc(
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
    pub(super) fn write_classic_ce(&self, rel: &str, group: &str) -> PathBuf {
        assert_eq!(self.game, "haloce_mcc");
        let path = self.root.join(format!("{rel}.{group}"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, classic_ce_bytes(group)).unwrap();
        path
    }

    pub(super) fn entries(&self) -> Vec<TagEntry> {
        crate::source::scan_folder_subtree_entries(&self.root, Path::new(""), &self.names())
            .expect("scan the fixture")
    }

    /// The key the folder scan gives the tag at `rel_with_extension`.
    pub(super) fn key(&self, rel_with_extension: &str) -> String {
        loose_file_entry(&self.root, &self.root.join(rel_with_extension), &self.names())
            .unwrap()
            .unwrap_or_else(|| panic!("{rel_with_extension} does not probe as a tag"))
            .key
    }

    /// The source a completed folder load of this kit would hand over.
    pub(super) fn source(&self) -> LoadedSourceData {
        let entries = self.entries();
        LoadedSourceData {
            label: "fixture".to_owned(),
            source: TagSource::LooseFolder {
                root: self.root.clone(),
                game: Some(self.game.to_owned()),
                definitions_root: locate_definitions_root(),
            },
            names: self.names(),
            game: Some(self.game.to_owned()),
            entries: entries.clone(),
            tree: crate::source::build_folder_directory_tree(&self.root).unwrap(),
            group_tree: crate::source::build_group_tree(&entries),
            all_entries: entries,
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: true,
            chosen_kit_layout: None,
        }
    }

    pub(super) fn install(&self, app: &mut Baboon) {
        app.install_loaded_source(self.source());
    }

    /// Open the tag at `rel_with_extension` and wait for its document.
    pub(super) fn open(&self, app: &mut Baboon, rel_with_extension: &str) -> String {
        let key = self.key(rel_with_extension);
        app.select_entry(key.clone(), ctx());
        pump_until(app, &format!("{rel_with_extension} to load"), |app| {
            app.kits[app.active].parsed_tags.contains_key(&key)
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
pub(super) fn edit_field(app: &mut Baboon, key: &str, path: &str, input: &str) {
    let ops = DeferredOps {
        pending: vec![PendingFieldEdit {
            path: path.to_owned(),
            input: input.to_owned(),
        }],
        ..DeferredOps::default()
    };
    let active = app.active;
    let applied = app
        .apply_doc_ops(active, key, "Edit", ops, UndoStep::Own)
        .expect("the document is open");
    for outcome in &applied.outcomes {
        assert!(outcome.result.is_ok(), "edit {path}: {:?}", outcome.result);
    }
}

pub(super) fn app() -> Baboon {
    Baboon::for_test()
}
