//! `load_model_preview` over synthetic tags: which group goes down which
//! path, what each produces, and what each refuses with.
//!
//! Characterization, with no kit. The geometry is a Halo CE gbxmodel built
//! from the definitions — the one render format whose vertices and triangles
//! are plain tag blocks — and the references between tags resolve against a
//! loose folder written to a temporary directory.

use super::*;
use crate::app::{Baboon, LoadedSourceData, ModelPreviewState, TagDocument};
use crate::app::model_preview::state::ModelTagPanelTab;
use crate::core::source::TagTree;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn new_tag_for(game: &str, group: &str) -> TagFile {
    TagFile::new(
        crate::core::bundled::locate_definitions_root()
            .join(game)
            .join(format!("{group}.json")),
    )
    .unwrap_or_else(|error| panic!("{game}/{group}.json: {error:?}"))
}

/// A fresh Halo CE tag of `group` in its classic container.
///
/// `TagFile::new` only builds MCC containers, and a CE tag in one is not
/// read as Halo CE by anything downstream. So this assembles the smallest
/// classic file there is — the 64-byte header and an all-zero root struct —
/// and reads it back the way a loose CE kit reads its tags.
fn classic_ce_tag(group: &str) -> TagFile {
    let definitions = crate::core::bundled::locate_definitions_root();
    let path = definitions.join("haloce_mcc").join(format!("{group}.json"));
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read the definition")).unwrap();
    let root_block = json["block"].as_str().unwrap();
    let root_struct = json["blocks"][root_block]["struct"].as_str().unwrap();
    let size = json["structs"][root_struct]["size"].as_u64().unwrap() as usize;
    let group_tag: [u8; 4] = json["tag"].as_str().unwrap().as_bytes().try_into().unwrap();
    let version = json["version"].as_u64().unwrap() as u16;
    let mut bytes = vec![0u8; 64];
    bytes[36..40].copy_from_slice(&group_tag);
    bytes[40..44].copy_from_slice(&u32::MAX.to_be_bytes());
    bytes[56..58].copy_from_slice(&version.to_be_bytes());
    bytes[60..64].copy_from_slice(b"blam");
    bytes.resize(64 + size, 0);
    let tag = crate::core::source::read_tag_from_bytes(
        &bytes,
        Some(GameId::HaloCe),
        Some(&definitions),
        u32::from_be_bytes(group_tag),
    )
    .unwrap_or_else(|error| panic!("a fresh classic {group}: {error:#}"));
    assert_eq!(
        blam_tags::game::Game::of(&tag),
        blam_tags::game::Game::Halo1
    );
    tag
}

fn set(tag: &mut TagFile, path: &str, input: &str) {
    crate::app::apply_field_edit(tag, path, input)
        .unwrap_or_else(|error| panic!("{path} = {input}: {error}"));
}

fn reference(tag: &mut TagFile, path: &str, group: &[u8; 4], name: &str) {
    let mut root = tag.root_mut();
    root.field_path_mut(path)
        .unwrap_or_else(|| panic!("{path} resolves"))
        .set(blam_tags::TagFieldData::TagReference(
            blam_tags::TagReferenceData {
                group_tag_and_name: Some((u32::from_be_bytes(*group), name.to_owned())),
            },
        ))
        .unwrap_or_else(|error| panic!("{path}: {error:?}"));
}

fn add(tag: &mut TagFile, path: &str) {
    let mut root = tag.root_mut();
    let mut field = root
        .field_path_mut(path)
        .unwrap_or_else(|| panic!("{path} resolves"));
    field
        .as_block_mut()
        .unwrap_or_else(|| panic!("{path} is a block"))
        .add_element();
}

/// A Halo CE gbxmodel: one region `body` whose permutation `base` uses
/// geometry 0, one part of one triangle spanning (0,0,0)-(1,2,3).
fn gbxmodel() -> TagFile {
    let mut tag = classic_ce_tag("gbxmodel");
    add(&mut tag, "regions");
    set(&mut tag, "regions[0]/name", "body");
    add(&mut tag, "regions[0]/permutations");
    set(&mut tag, "regions[0]/permutations[0]/name", "base");
    set(&mut tag, "regions[0]/permutations[0]/super high", "0");
    add(&mut tag, "geometries");
    add(&mut tag, "geometries[0]/parts");
    for (index, position) in ["0, 0, 0", "1, 0, 0", "0, 2, 3"].into_iter().enumerate() {
        add(&mut tag, "geometries[0]/parts[0]/uncompressed vertices");
        set(
            &mut tag,
            &format!("geometries[0]/parts[0]/uncompressed vertices[{index}]/position"),
            position,
        );
        set(
            &mut tag,
            &format!("geometries[0]/parts[0]/uncompressed vertices[{index}]/normal"),
            "0, 0, 1",
        );
    }
    add(&mut tag, "geometries[0]/parts[0]/triangles");
    for (field, index) in [("vertex0 index", "0"), ("vertex1 index", "1"), ("vertex2 index", "2")]
    {
        set(
            &mut tag,
            &format!("geometries[0]/parts[0]/triangles[0]/{field}"),
            index,
        );
    }
    tag
}

fn entry(display_path: &str, tag: &TagFile, location: TagEntryLocation) -> TagEntry {
    TagEntry {
        key: format!("file:{display_path}"),
        display_path: display_path.to_owned(),
        group_tag: tag.header.group_tag,
        group_name: display_path
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_owned()),
        location,
    }
}

fn names() -> TagNameIndex {
    TagNameIndex::load_from_definitions(&crate::core::bundled::locate_definitions_root())
}

fn load(tag: &TagFile, display_path: &str, source: Option<&TagSource>) -> Result<ModelPreviewData, String> {
    load_model_preview(
        tag,
        &entry(display_path, tag, TagEntryLocation::LooseFile(display_path.into())),
        &names(),
        source,
        &PreviewLoadSettings::default(),
    )
}

/// A fresh temporary tags folder, removed when dropped.
struct LooseKit {
    root: PathBuf,
    game: &'static str,
}

impl LooseKit {
    fn new(name: &str, game: &'static str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "baboon-preview-{name}-{}-{}",
            std::process::id(),
            NEXT_KIT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self { root, game }
    }

    fn write(&self, relative: &str, tag: &TagFile) -> PathBuf {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, tag.write_to_bytes().expect("serialize the tag")).unwrap();
        path
    }

    fn source(&self) -> TagSource {
        TagSource::LooseFolder {
            root: self.root.clone(),
            game: GameId::from_id(self.game),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        }
    }
}

impl Drop for LooseKit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

static NEXT_KIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn assert_one_triangle(preview: &RenderModelPreview) {
    assert_eq!(preview.regions.len(), 1);
    assert_eq!(preview.regions[0].name, "body");
    assert_eq!(preview.regions[0].permutations, ["base"]);
    assert_eq!(preview.vertices.len(), 3);
    assert_eq!(preview.indices.len(), 3);
    assert_eq!(preview.batches.len(), 1);
    assert_eq!(preview.bounds_min, [0.0, 0.0, 0.0]);
    assert_eq!(preview.bounds_max, [1.0, 2.0, 3.0]);
}

/// A gbxmodel is its own render geometry: no wrapper, no source needed.
#[test]
fn a_gbxmodel_previews_itself() {
    let tag = gbxmodel();
    let data = load(&tag, "objects/thing/thing.gbxmodel", None).expect("the gbxmodel previews");
    assert_eq!(data.source_key, "file:objects/thing/thing.gbxmodel");
    assert_eq!(data.render_model_path, "objects/thing/thing.gbxmodel");
    assert!(data.variants.is_empty());
    assert!(data.scenario_bsps.is_empty());
    assert_one_triangle(&data.preview);
}

/// Geometry with nothing to draw is refused rather than shown empty.
#[test]
fn an_empty_render_tag_is_refused() {
    let tag = classic_ce_tag("gbxmodel");
    assert_eq!(
        load(&tag, "objects/thing/empty.gbxmodel", None).err().as_deref(),
        Some("This render tag has no previewable draw batches.")
    );
}

/// A Halo CE object names its gbxmodel directly, and previews it from the
/// loaded source — refusing without one, or without a reference.
#[test]
fn a_halo_ce_object_previews_the_gbxmodel_it_names() {
    let kit = LooseKit::new("ce-object", "haloce_mcc");
    kit.write("objects/thing/thing.gbxmodel", &gbxmodel());
    let source = kit.source();

    let mut scenery = classic_ce_tag("scenery");
    assert_eq!(
        load(&scenery, "objects/thing/thing.scenery", Some(&source))
            .err()
            .as_deref(),
        Some("This object references no gbxmodel.")
    );
    reference(&mut scenery, "object/model", b"mod2", "objects\\thing\\thing");
    assert_eq!(
        load(&scenery, "objects/thing/thing.scenery", None).err().as_deref(),
        Some("Halo CE object preview requires a loaded source.")
    );
    let data = load(&scenery, "objects/thing/thing.scenery", Some(&source))
        .expect("the referenced gbxmodel previews");
    assert_eq!(data.source_key, "file:objects/thing/thing.scenery");
    assert_eq!(data.render_model_path, "objects\\thing\\thing");
    assert_one_triangle(&data.preview);

    reference(&mut scenery, "object/model", b"mod2", "objects\\thing\\missing");
    let error = load(&scenery, "objects/thing/thing.scenery", Some(&source)).err().expect("refused");
    assert!(
        error.starts_with("Could not load objects\\thing\\missing.gbxmodel:"),
        "{error}"
    );
}

/// A `.model` resolves its render model against the loose folder, and says
/// which of the steps on the way failed.
#[test]
fn a_model_resolves_its_render_model_in_the_loose_folder() {
    let kit = LooseKit::new("h3-model", "halo3_mcc");
    let source = kit.source();
    let mut model = new_tag_for("halo3_mcc", "model");
    assert_eq!(
        load(&model, "objects/thing/thing.model", Some(&source))
            .err()
            .as_deref(),
        Some("This model tag has no render model reference.")
    );
    set(&mut model, "render model", "objects\\thing\\thing.render_model");
    assert_eq!(
        load(&model, "objects/thing/thing.model", None).err().as_deref(),
        Some("Render model preview requires a loaded loose-folder editing kit.")
    );
    let error = load(&model, "objects/thing/thing.model", Some(&source)).err().expect("refused");
    assert!(
        error.starts_with("Referenced render_model was not found:"),
        "{error}"
    );
    assert!(error.ends_with("thing.render_model"), "{error}");

    // Present, but a fresh render_model has nothing to draw.
    kit.write(
        "objects/thing/thing.render_model",
        &new_tag_for("halo3_mcc", "render_model"),
    );
    assert_eq!(
        load(&model, "objects/thing/thing.model", Some(&source))
            .err()
            .as_deref(),
        Some("Referenced render_model has no previewable draw batches.")
    );
}

/// A scenario lists its BSPs and loads none of them until one is chosen.
#[test]
fn a_scenario_lists_its_bsps_without_loading_them() {
    let mut scenario = new_tag_for("halo3_mcc", "scenario");
    assert_eq!(
        load(&scenario, "levels/test/test.scenario", None)
            .err()
            .as_deref(),
        Some("This scenario lists no structure BSPs.")
    );
    add(&mut scenario, "structure bsps");
    add(&mut scenario, "structure bsps");
    set(
        &mut scenario,
        "structure bsps[0]/structure bsp",
        "levels\\test\\test_a.scenario_structure_bsp",
    );
    let data = load(&scenario, "levels/test/test.scenario", None).expect("the scenario lists");
    assert_eq!(
        data.scenario_bsps,
        [Some("levels\\test\\test_a".to_owned()), None]
    );
    assert!(data.preview.batches.is_empty(), "nothing loads unasked");
    assert_eq!(data.preview.bounds_min, [0.0; 3]);

    // Choosing one needs a source to load it from.
    let settings = PreviewLoadSettings {
        high_detail: true,
        scenario_selection: [0].into_iter().collect(),
    };
    let error = load_model_preview(
        &scenario,
        &entry(
            "levels/test/test.scenario",
            &scenario,
            TagEntryLocation::LooseFile("levels/test/test.scenario".into()),
        ),
        &names(),
        None,
        &settings,
    )
    .err().expect("refused");
    assert_eq!(error, "Scenario preview requires a loaded source.");
}

/// A model tag of no previewable kind falls through to the render-model
/// lookup, and a physics model with no shapes is refused by its builder.
#[test]
fn other_groups_take_the_paths_their_group_names() {
    let biped = new_tag_for("halo3_mcc", "biped");
    assert_eq!(
        load(&biped, "objects/thing/thing.biped", None).err().as_deref(),
        Some("This model tag has no render model reference.")
    );
    let physics = new_tag_for("halo3_mcc", "physics_model");
    assert!(
        load(&physics, "objects/thing/thing.physics_model", None).is_err(),
        "an empty physics model has nothing to show"
    );
}

/// The worker round trip: the post-draw hook hands the parse to a thread,
/// shows the loading shells, and the reply installs the preview with its
/// selection reset — from disk, and from an edited document's bytes.
#[test]
fn a_gbxmodel_preview_loads_on_a_worker() {
    for edited in [false, true] {
        let kit = LooseKit::new("worker", "haloce_mcc");
        let relative = "objects/thing/thing.gbxmodel";
        let path = kit.write(relative, &gbxmodel());
        let tag = gbxmodel();
        let entry = entry(relative, &tag, TagEntryLocation::LooseFile(path));
        let mut app = Baboon::for_test();
        app.install_loaded_source(LoadedSourceData {
            label: "synthetic".to_owned(),
            source: kit.source(),
            names: names(),
            game: Some(GameId::HaloCe),
            entries: vec![entry.clone()],
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: vec![entry.clone()],
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: true,
            chosen_kit_layout: None,
        });
        if edited {
            // Moved the triangle's apex: the worker must parse these bytes,
            // not the file.
            let mut document = TagDocument::clean(gbxmodel());
            set(
                &mut document.tag,
                "geometries[0]/parts[0]/uncompressed vertices[2]/position",
                "0, 4, 5",
            );
            document.dirty.touch();
            app.kits[0].parsed_tags.insert(entry.key.clone(), document);
        }
        let ctx = egui::Context::default();
        // Not the preview tab: nothing is asked for.
        app.kits[0]
            .caches.model_previews
            .insert(entry.key.clone(), ModelPreviewState::default());
        app.maybe_request_model_preview(0, &entry.key, &ctx);
        assert!(app.kits[0].caches.model_previews[&entry.key].preview_load_id.is_none());

        app.kits[0]
            .caches.model_previews
            .get_mut(&entry.key)
            .unwrap()
            .active_tab = ModelTagPanelTab::ModelPreview;
        app.maybe_request_model_preview(0, &entry.key, &ctx);
        let state = &app.kits[0].caches.model_previews[&entry.key];
        let first = state.preview_load_id.expect("a worker started");
        assert!(state.data.is_none(), "the shells show while it parses");
        assert_eq!(state.loaded_key.as_deref(), Some(entry.key.as_str()));
        // Asking again while it runs starts nothing new.
        app.maybe_request_model_preview(0, &entry.key, &ctx);
        assert_eq!(
            app.kits[0].caches.model_previews[&entry.key].preview_load_id,
            Some(first)
        );

        let deadline = Instant::now() + Duration::from_secs(60);
        while app.kits[0].caches.model_previews[&entry.key].data.is_none() {
            assert!(Instant::now() < deadline, "the preview never landed");
            app.process_worker_messages(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        let state = &app.kits[0].caches.model_previews[&entry.key];
        assert!(state.preview_load_id.is_none(), "the request is answered");
        assert_eq!(state.render_model_path.as_deref(), Some(relative));
        let data = state.data.as_ref().unwrap().as_ref().expect("it loads");
        assert_eq!(data.preview.vertices.len(), 3);
        let apex = if edited { [1.0, 4.0, 5.0] } else { [1.0, 2.0, 3.0] };
        assert_eq!(data.preview.bounds_max, apex, "edited: {edited}");
    }
}
