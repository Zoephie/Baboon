use super::super::{
    KitStamp, ModelPreviewData, RenderModel, RenderModelPreviewRegion, read_entry, tag_path,
};
use super::*;
use crate::app::editor::{
    is_model_group, is_previewable_geometry_group, is_previewable_geometry_group_for_game,
};
use crate::app::model_preview::RenderModelPreview;
use crate::app::model_preview::harness::kit::kit_root;
use crate::app::model_preview::load::build_particle_model_preview;
use crate::app::model_preview::state::ModelTagPanelTab;
use crate::app::{Baboon, LoadedSourceData, ModelPreviewState, TagDocument};
use crate::core::format::TagNameIndex;
use crate::core::game::GameId;
use crate::core::source::{TagEntry, TagEntryLocation, TagSource, TagTree};
use crate::core::tag_key::file_entry_key;
use blam_tags::TagFile;
use eframe::egui;
use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, Instant};

// `particle_model` tags get a working Model Preview tab.
//
// blam-tags owns the decode (splitting the merged triangle strip at the
// `m_gpu_data/m_variants` boundaries, decompressing through the
// compression bounds) and is tested there. What this asserts is
// Baboon's half:
//
// - the tab pair and viewport actually appear for `pmdf` / `PRTM`,
// - **without** widening [`is_model_group`], whose other job is deciding
//   whether a `tag_reference` is an object's model link — a `particle`
//   tag's `Model` → `pmdf` field would be misread as one,
// - each JMI object becomes its own preview region, so the region list
//   doubles as an object toggle,
// - the geometry the viewport uploads is right way round — batches
//   index inside the vertex buffer and face normals agree with the
//   stored vertex normals.
//
// Skips silently when the corresponding tag set is absent.

/// Root of an extracted MCC tag set, via `BLAM_TEST_<KIT>_TAGS` or the
/// conventional local layout.
/// A kit's `tags` folder by its short name, through the shared harness
/// helper.
fn kit_tags(kit: &str) -> Option<PathBuf> {
    let game = match kit {
        "halo2" => GameId::Halo2,
        "halo3" => GameId::Halo3,
        "haloreach" => GameId::HaloReach,
        "halo4" => GameId::Halo4,
        other => panic!("no kit named {other}"),
    };
    crate::app::model_preview::harness::kit::kit_root(game)
}

fn bundled_names() -> crate::core::format::TagNameIndex {
    let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    crate::core::format::TagNameIndex::load_from_definitions(&defs)
}

/// Read a tag, routing Halo 2's classic format through its JSON
/// definition (classic tags carry no embedded `blay`).
fn read(path: &std::path::Path, game: &str) -> TagFile {
    let bytes = std::fs::read(path).expect("read tag bytes");
    if blam_tags::classic::ClassicHeader::parse(&bytes).is_some() {
        let def = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("definitions")
            .join(game)
            .join("particle_model.json");
        let layout = blam_tags::layout::TagLayout::from_json(&def).expect("load classic layout");
        return blam_tags::classic::read_classic_tag_file(&bytes, layout).expect("decode classic");
    }
    TagFile::read(path).expect("read tag")
}

/// Mean dot(face normal, averaged vertex normal) over the preview's
/// triangles. A correct upload lands near +1; flipped winding lands near
/// -1, and a mis-split strip near 0.
fn face_normal_agreement(preview: &RenderModelPreview) -> Option<f32> {
    let mut total = 0.0f64;
    let mut n = 0usize;
    for tri in preview.indices.chunks_exact(3) {
        let v: Vec<_> = tri
            .iter()
            .filter_map(|&i| preview.vertices.get(i as usize))
            .collect();
        if v.len() != 3 {
            continue;
        }
        let (pa, pb, pc) = (v[0].position, v[1].position, v[2].position);
        let u = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let w = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let f = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        let fl = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
        if fl < 1e-12 {
            continue;
        }
        let vn = [
            (v[0].normal[0] + v[1].normal[0] + v[2].normal[0]) / 3.0,
            (v[0].normal[1] + v[1].normal[1] + v[2].normal[1]) / 3.0,
            (v[0].normal[2] + v[1].normal[2] + v[2].normal[2]) / 3.0,
        ];
        let vl = (vn[0] * vn[0] + vn[1] * vn[1] + vn[2] * vn[2]).sqrt();
        if vl < 1e-12 {
            continue;
        }
        total += (0..3).map(|k| (f[k] / fl) * (vn[k] / vl)).sum::<f32>() as f64;
        n += 1;
    }
    (n > 0).then(|| (total / n as f64) as f32)
}

/// The panel gate opens for particle models — and `is_model_group` stays
/// closed, so `find_model_reference` does not start treating a
/// `particle`'s `Model` field as an object's model link.
#[test]
fn particle_model_is_previewable_without_becoming_a_model() {
    let names = bundled_names();
    for group in [b"pmdf", b"PRTM"] {
        let tag = u32::from_be_bytes(*group);
        let label = String::from_utf8_lossy(group).into_owned();
        assert!(
            is_previewable_geometry_group(tag, &names),
            "`{label}` must open the Model Preview tab",
        );
        assert!(
            !is_model_group(tag, &names),
            "`{label}` must NOT count as a model group — that predicate also \
         decides whether a tag_reference is an object's model link",
        );
    }
    // The predicate must still admit everything it used to.
    for group in [b"hlmt", b"mod2"] {
        let tag = u32::from_be_bytes(*group);
        assert!(is_previewable_geometry_group(tag, &names));
    }
}

/// The panel gate opens for a bare `render_model` (mode) — the preview loader
/// draws the tag itself, so the viewport belongs inside the tag too.
#[test]
fn render_model_is_previewable() {
    let names = bundled_names();
    let tag = u32::from_be_bytes(*b"mode");
    assert!(
        is_previewable_geometry_group(tag, &names),
        "`mode` must open the Model Preview tab",
    );
}

#[test]
fn an_object_previews_in_any_kit_and_needs_one() {
    let names = bundled_names();
    for group in [b"bipd", b"vehi", b"weap", b"eqip", b"scen"] {
        let tag = u32::from_be_bytes(*group);
        assert!(is_previewable_geometry_group_for_game(
            tag,
            &names,
            Some(GameId::HaloCe)
        ));
        assert!(is_previewable_geometry_group_for_game(
            tag,
            &names,
            Some(GameId::Halo3)
        ));
        assert!(!is_previewable_geometry_group_for_game(tag, &names, None));
    }
}

/// A multi-object gen3 tag: one region per JMI object, batches wired to
/// those regions, and geometry the right way round.
#[test]
fn gen3_objects_become_regions_with_valid_geometry() {
    let Some(tags) = kit_tags("haloreach") else {
        return;
    };
    let path = tags.join("fx/particles/models/debris/generic_shards/generic_shards.particle_model");
    if !path.is_file() {
        return;
    }
    let tag = read(&path, "haloreach_mcc");
    let preview = build_particle_model_preview(&tag, "generic_shards").expect("build preview");

    assert_eq!(preview.regions.len(), 8, "generic_shards ships 8 objects");
    assert_eq!(
        preview.batches.len(),
        preview.regions.len(),
        "every object needs a draw batch or it renders invisible",
    );
    for (region, batch) in preview.regions.iter().zip(&preview.batches) {
        assert_eq!(
            region.name, batch.region_name,
            "batch must target its region"
        );
        assert!(
            region.permutations.contains(&batch.permutation_name),
            "batch permutation `{}` is not selectable in region `{}`",
            batch.permutation_name,
            region.name,
        );
        assert!(
            batch.index_count > 0,
            "region `{}` has an empty batch",
            region.name
        );
    }

    // Every batch must address inside the shared buffers, or the
    // renderer reads past the end.
    for batch in &preview.batches {
        let end = (batch.index_start + batch.index_count) as usize;
        assert!(
            end <= preview.indices.len(),
            "batch range past the index buffer"
        );
        for &i in &preview.indices[batch.index_start as usize..end] {
            assert!(
                (i as usize) < preview.vertices.len(),
                "index past the vertex buffer"
            );
        }
    }

    assert!(
        preview.bounds_min.iter().all(|v| v.is_finite())
            && preview.bounds_max.iter().all(|v| v.is_finite()),
        "bounds must be finite or the camera cannot frame the model",
    );

    let score = face_normal_agreement(&preview).expect("measurable");
    assert!(
        score > 0.6,
        "preview geometry scored {score:.3} — a mis-split strip scores ~0 and \
     flipped winding ~-1",
    );
}

/// Halo 2's `PRTM` is a different tag and a different decode, and it
/// names its own objects — the region list should show those names.
#[test]
fn halo2_regions_carry_the_shipped_object_names() {
    let Some(tags) = kit_tags("halo2") else {
        return;
    };
    let path = tags.join("effects/particle_models/urban_debris/urban_debris.particle_model");
    if !path.is_file() {
        return;
    }
    let tag = read(&path, "halo2_mcc");
    let preview = build_particle_model_preview(&tag, "urban_debris").expect("build preview");

    let region_names: Vec<&str> = preview.regions.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        region_names,
        vec![
            "can_1", "can_2", "can_3", "can_4", "can_5", "paper_1", "paper_2", "paper_3", "butt_1",
            "butt_2",
        ],
        "Halo 2 stores `models[].model name` — the region list must show them",
    );

    let score = face_normal_agreement(&preview).expect("measurable");
    assert!(score > 0.6, "preview geometry scored {score:.3}");
}

/// A single-object tag still produces one selectable region rather than
/// an empty list, so the viewport is not blank.
#[test]
fn single_object_tag_still_yields_one_region() {
    let Some(tags) = kit_tags("haloreach") else {
        return;
    };
    let path = tags.join("fx/particles/models/weapons/brute_spike/brute_spike.particle_model");
    if !path.is_file() {
        return;
    }
    let tag = read(&path, "haloreach_mcc");
    let preview = build_particle_model_preview(&tag, "brute_spike").expect("build preview");

    assert_eq!(preview.regions.len(), 1);
    assert_eq!(preview.regions[0].name, "brute_spike");
    assert!(
        !preview.indices.is_empty(),
        "single-object preview must have geometry"
    );
}

/// Drive the real UI entry point, not the builder underneath it.
///
/// `load_model_preview` is what the panel calls, and it derives the
/// object-naming stem from `entry.display_path` rather than being handed
/// one. Asserting through it is what catches a stem derivation that
/// silently yields `""` (every object would become `_1`, `_2`, …) or
/// keeps the `.particle_model` extension.
#[test]
fn reach_lights_quadratic_meshes_by_sh_and_clamps_its_ambient_transfer() {
    let Some(tags) = kit_tags("haloreach") else {
        return;
    };
    let read = |path: &str| {
        let tag = TagFile::read(tags.join(path)).unwrap();
        let model = RenderModel::from_tag(&tag).unwrap();
        let meshes = RenderModel::derive_render_meshes(&tag).unwrap();
        super::super::gen3::prt::read_gen3_prt(&tag, &model, &meshes).unwrap()
    };
    // Its meshes bake quadratic transfer, which Reach's tool no longer lights by.
    let quadratic = read(
        "objects/test/test_fbx_characters/test_fbx_civilian_female_custom_animation/test_fbx_civilian_female_custom_animation.render_model",
    );
    assert!(
        (0..quadratic.vertices.len() as u32)
            .all(|vertex| quadratic.kind_at(vertex) == super::super::gen3::prt::Gen3PrtKind::None)
    );
    let ambient = read("objects/characters/spartans/spartans.render_model");
    assert!(
        (0..ambient.vertices.len() as u32)
            .any(|vertex| ambient.kind_at(vertex) == super::super::gen3::prt::Gen3PrtKind::Ambient)
    );
    // The client's clamp to 0..1 is kept, but no object in the kit has an
    // ambient value outside it (all 1,549 render models scanned), so
    // nothing here can show it.
}

#[test]
fn load_model_preview_derives_object_names_from_the_entry() {
    let Some(tags) = kit_tags("haloreach") else {
        return;
    };
    let path = tags.join("fx/particles/models/debris/falling_leaves/falling_leaves.particle_model");
    if !path.is_file() {
        return;
    }
    let tag = read(&path, "haloreach_mcc");
    let names = bundled_names();
    let entry = crate::core::source::TagEntry {
        key: file_entry_key(&path),
        display_path: r"fx\particles\models\debris\falling_leaves\falling_leaves.particle_model"
            .to_owned(),
        group_tag: tag.header.group_tag,
        group_name: Some("particle_model".to_owned()),
        location: crate::core::source::TagEntryLocation::LooseFile(path.clone()),
    };

    let data = crate::app::model_preview::load::load_model_preview(
        &tag,
        &entry,
        &names,
        None,
        &crate::app::model_preview::load::PreviewLoadSettings::default(),
    )
    .expect("preview loads without a loose-folder source — geometry is inline");

    assert!(!data.preview.regions.is_empty(), "no objects were exposed");
    for region in &data.preview.regions {
        assert!(
            region.name.starts_with("falling_leaves"),
            "object `{}` was not named from the tag stem — the stem derivation \
         is dropping or mangling `entry.display_path`",
            region.name,
        );
        assert!(
            !region.name.contains(".particle_model"),
            "object `{}` kept the tag extension",
            region.name,
        );
    }
    // A particle_model has no model variants. The Variant combo still
    // renders (showing only `<None>`), same as a bare `render_model`
    // preview — but nothing must invent entries for it, or the combo
    // would offer selections that change nothing.
    assert!(data.variants.is_empty(), "a particle_model has no variants");
}

/// A shipped Halo 3 `render_model` opened on its own must produce a preview
/// with geometry: `load_model_preview` draws the tag itself, no `hlmt`
/// wrapper involved, which is what the Model Preview tab inside the tag shows.
#[test]
fn a_shipped_render_model_previews_on_its_own() {
    let Some(tags) = kit_tags("halo3") else {
        eprintln!("skipping: no halo3 tag set");
        return;
    };
    let rel = "objects/weapons/rifle/assault_rifle/assault_rifle.render_model";
    let path = tags.join(rel);
    if !path.is_file() {
        eprintln!("skipping: no {rel} in this kit");
        return;
    }
    let tag = read(&path, "halo3_mcc");
    let names = bundled_names();
    assert!(
        is_previewable_geometry_group(tag.header.group_tag, &names),
        "`mode` must open the Model Preview tab",
    );
    let entry = crate::core::source::TagEntry {
        key: file_entry_key(&path),
        display_path: rel.to_owned(),
        group_tag: tag.header.group_tag,
        group_name: Some("render_model".to_owned()),
        location: crate::core::source::TagEntryLocation::LooseFile(path.clone()),
    };
    let data = crate::app::model_preview::load::load_model_preview(
        &tag,
        &entry,
        &names,
        None,
        &crate::app::model_preview::load::PreviewLoadSettings::default(),
    )
    .expect("a shipped render_model must preview");
    assert!(
        !data.preview.batches.is_empty(),
        "the preview came back with no draw batches"
    );
}

/// Every shipped particle_model in every present kit must produce a
/// preview with geometry — no panic, no empty viewport.
///
/// The panel wraps the load in `catch_unwind` and shows the message, so
/// a regression here degrades to a red label rather than a crash; this
/// keeps it from degrading silently.
#[test]
fn every_shipped_particle_model_previews() {
    let kits = [
        ("halo2", "halo2_mcc"),
        ("halo3", "halo3_mcc"),
        ("haloreach", "haloreach_mcc"),
        ("halo4", "halo4_mcc"),
    ];
    let names = bundled_names();
    let mut checked = 0usize;
    let mut objects = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for (kit, game) in kits {
        let Some(root) = kit_tags(kit) else { continue };
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("particle_model") {
                    continue;
                }
                let tag = read(&path, game);
                let rel = path.strip_prefix(&root).unwrap_or(&path);
                let display = tag_path::relative_tag_path(rel);
                let tag_entry = crate::core::source::TagEntry {
                    key: file_entry_key(&path),
                    display_path: display.clone(),
                    group_tag: tag.header.group_tag,
                    group_name: Some("particle_model".to_owned()),
                    location: crate::core::source::TagEntryLocation::LooseFile(path.clone()),
                };
                checked += 1;
                match crate::app::model_preview::load::load_model_preview(
                    &tag,
                    &tag_entry,
                    &names,
                    None,
                    &crate::app::model_preview::load::PreviewLoadSettings::default(),
                ) {
                    Ok(data) => {
                        if data.preview.indices.is_empty() || data.preview.regions.is_empty() {
                            failures.push(format!("{display}: empty preview"));
                        }
                        objects += data.preview.regions.len();
                    }
                    Err(e) => failures.push(format!("{display}: {e}")),
                }
            }
        }
    }

    if checked == 0 {
        return; // no kits present
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} particle_models failed to preview:\n  {}",
        failures.len(),
        failures.join("\n  "),
    );
    eprintln!("[particle_model preview] {checked} tags, {objects} objects");
}

// The model preview parses on a worker, not the UI thread.
//
// What the app derives, frame by frame: the post-draw hook starts a worker
// and leaves the state without data (the loading shells), the worker's
// message installs the result, and a result for a request the state no
// longer wants is dropped instead of installed. Both a modern (Halo 3) and a
// classic (Halo 2) tag must load, from disk and from an edited document's
// bytes: a classic tag's bytes carry no layout, so re-parsing them the
// modern way fails where reading them the kit's way does not.
//
// Needs `BLAM_TEST_H3EK` / `BLAM_TEST_H2EK`; skips a kit that is not set.

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
    let definitions = crate::core::test_kits::definitions();
    let entry = TagEntry {
        key: file_entry_key(&path),
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
            game: GameId::from_id(game),
            definitions_root: definitions.to_path_buf(),
        },
        names: TagNameIndex::load_from_definitions(definitions),
        game: GameId::from_id(game),
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
    let preview = ModelPreviewState {
        active_tab: ModelTagPanelTab::ModelPreview,
        ..ModelPreviewState::default()
    };
    app.views[app.model.kits[0].id]
        .caches
        .model_previews
        .insert(entry.key.clone(), preview);
    Some(Fixture {
        app,
        key: entry.key,
        ctx: egui::Context::default(),
    })
}

impl Fixture {
    fn state(&self) -> &ModelPreviewState {
        &self.app.views[self.app.model.kits[0].id]
            .caches
            .model_previews[&self.key]
    }

    fn state_mut(&mut self) -> &mut ModelPreviewState {
        self.app.views[self.app.model.kits[0].id]
            .caches
            .model_previews
            .get_mut(&self.key)
            .unwrap()
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
        let entry = self.app.model.kits[0]
            .entry_for_key(&self.key)
            .unwrap()
            .clone();
        let source = self.app.model.kits[0]
            .source
            .as_ref()
            .unwrap()
            .source
            .clone();
        let tag = crate::core::source::read_entry(&source, &entry).expect("read render_model");
        let mut document = TagDocument::clean(tag);
        document.dirty.touch();
        self.app.model.kits[0]
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
    let Some(tags) = crate::app::model_preview::harness::kit::kit_root(GameId::Halo3) else {
        return;
    };
    check(
        &tags,
        "halo3_mcc",
        "objects/weapons/rifle/assault_rifle/assault_rifle.render_model",
    );
}

#[test]
fn a_classic_halo2_render_model_loads_on_a_worker() {
    let Some(tags) = crate::app::model_preview::harness::kit::kit_root(GameId::Halo2) else {
        return;
    };
    check(
        &tags,
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
    let Some(tags) = kit_root(GameId::Halo3) else {
        return;
    };
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
    fixture
        .app
        .maybe_request_model_preview(0, &key, &fixture.ctx);
    let second = fixture
        .state()
        .preview_load_id
        .expect("a new worker started");
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

/// Refresh, an edit or a saved variant reloads the preview: the variant
/// stays chosen by name, and the animation plays on as it was set.
#[test]
fn a_reload_keeps_the_variant_and_how_the_animation_plays() {
    let Some(tags) = kit_root(GameId::Halo3) else {
        return;
    };
    let rel = "objects/vehicles/warthog/warthog.model";
    let Some(mut fixture) = fixture(&tags, "halo3_mcc", rel) else {
        return;
    };
    fixture.frames_until_loaded();
    let variants: Vec<String> = match &fixture.state().data {
        Some(Ok(data)) => data
            .variants
            .iter()
            .map(|variant| variant.name.clone())
            .collect(),
        _ => panic!("the warthog loads"),
    };
    let chosen = variants
        .iter()
        .position(|name| name != "default")
        .expect("a variant past default");
    let state = fixture.state_mut();
    state.selected_variant = Some(chosen);
    state.animation.speed = 0.5;
    state.animation.looped = false;
    state.invalidate_load();
    fixture.frames_until_loaded();
    let state = fixture.state();
    assert_eq!(
        state.selected_variant.map(|index| variants[index].as_str()),
        Some(variants[chosen].as_str())
    );
    assert_eq!(
        (state.animation.speed, state.animation.looped),
        (0.5, false)
    );
}

/// The overlays landing seed their collision-only regions as the chosen
/// variant does, so the variant still reads as chosen, not "(custom)".
#[test]
fn overlays_landing_keep_the_variant_chosen() {
    let Some(tags) = kit_root(GameId::Halo3) else {
        return;
    };
    let rel = "objects/vehicles/warthog/warthog.model";
    let Some(mut fixture) = fixture(&tags, "halo3_mcc", rel) else {
        return;
    };
    fixture.frames_until_loaded();
    let kit = &fixture.app.model.kits[0];
    let stamp = KitStamp {
        kit: kit.id,
        generation: kit.generation,
    };
    let (geometry_id, chosen) = match &fixture.state().data {
        Some(Ok(data)) => (
            data.geometry_id,
            data.variants
                .iter()
                .position(|variant| variant.name != "default"),
        ),
        _ => panic!("the warthog loads"),
    };
    fixture.state_mut().selected_variant = chosen;
    let state = fixture.state_mut();
    if let Some(Ok(data)) = &state.data {
        state.region_selections = super::super::selection::compute_variant_selection(data, chosen);
    }
    // Its first permutation is not the one a variant gives it.
    let collision = RenderModelPreview {
        regions: vec![RenderModelPreviewRegion {
            name: "collision only".to_owned(),
            permutations: vec!["~broken".to_owned(), "whole".to_owned()],
        }],
        ..Default::default()
    };
    let key = fixture.key.clone();
    fixture.app.handle_model_overlays_built(
        stamp,
        key,
        geometry_id,
        Some(collision),
        None,
        Vec::new(),
    );
    let state = fixture.state();
    let Some(Ok(data)) = &state.data else {
        panic!("still loaded")
    };
    assert_eq!(
        super::super::selection::detect_active_variant(data, state),
        Some(chosen)
    );
}

/// A reader that panics costs its feature, with a note, and the preview it
/// was adding to comes back as it was.
#[test]
fn a_crashing_feature_is_noted_and_its_half_merge_undone() {
    let mut notes = Vec::new();
    let lights: Vec<u32> = feature(&mut notes, "its lights", || panic!("bad light tag"));
    assert!(lights.is_empty());
    assert_eq!(notes, ["its lights: the reader crashed (bad light tag)"]);
    let mut preview = RenderModelPreview::default();
    feature_into(
        &mut notes,
        "a held weapon",
        &mut preview,
        |preview| -> Option<()> {
            preview.indices.push(7);
            panic!("half merged");
        },
    );
    assert!(preview.indices.is_empty(), "the half merge was left in");
    assert_eq!(notes.len(), 2);
    let kept = feature(&mut notes, "its functions", || Some(3));
    assert_eq!(
        (kept, notes.len()),
        (Some(3), 2),
        "a reader that works is untouched"
    );
}

/// A scenario whose chosen BSP will not load still previews, saying why,
/// rather than failing as a whole.
#[test]
fn a_scenario_with_an_unloadable_bsp_previews_with_a_note() {
    let Some(tags) = kit_root(GameId::Halo3) else {
        return;
    };
    let rel = "levels/multi/guardian/guardian.scenario";
    if !tags.join(rel).is_file() {
        eprintln!("skipping: BLAM_TEST_H3EK has no {rel}");
        return;
    }
    // The scenario alone, so its BSP reference has nothing to find.
    let root = std::env::temp_dir().join(format!("baboon-lone-scenario-{}", std::process::id()));
    std::fs::create_dir_all(root.join("levels/multi/guardian")).unwrap();
    std::fs::copy(tags.join(rel), root.join(rel)).unwrap();
    let source = TagSource::LooseFolder {
        root: root.clone(),
        game: Some(GameId::Halo3),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let TagSource::LooseFolder {
        definitions_root, ..
    } = &source
    else {
        unreachable!()
    };
    let names = TagNameIndex::load_game(definitions_root, GameId::Halo3).unwrap();
    let entry = TagEntry {
        key: rel.to_owned(),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"scnr"),
        group_name: Some("scenario".to_owned()),
        location: TagEntryLocation::LooseFile(root.join(rel)),
    };
    let tag = read_entry(&source, &entry).unwrap();
    let settings = PreviewLoadSettings {
        scenario_selection: [0].into(),
        ..Default::default()
    };
    let loaded = load_model_preview(&tag, &entry, &names, Some(&source), &settings);
    std::fs::remove_dir_all(&root).ok();
    let data = loaded.expect("the scenario previews without its BSP");
    assert_eq!(data.notes.len(), 1, "{:?}", data.notes);
    assert!(
        data.notes[0].starts_with("Could not load"),
        "{:?}",
        data.notes
    );
}

/// A clip list made for geometry a reload has replaced is dropped; one made
/// for the geometry loaded now lands.
#[test]
fn a_clip_list_for_replaced_geometry_is_dropped() {
    let Some(tags) = kit_root(GameId::Halo3) else {
        return;
    };
    let rel = "objects/vehicles/warthog/warthog.model";
    let Some(mut fixture) = fixture(&tags, "halo3_mcc", rel) else {
        return;
    };
    fixture.frames_until_loaded();
    let kit = &fixture.app.model.kits[0];
    let stamp = KitStamp {
        kit: kit.id,
        generation: kit.generation,
    };
    let current = match &fixture.state().data {
        Some(Ok(data)) => data.geometry_id,
        _ => panic!("the warthog loads"),
    };
    let listed = |geometry_id| crate::app::WorkerMessage::ModelAnimationsListed {
        stamp,
        key: fixture.key.clone(),
        geometry_id,
        result: Ok(Vec::new()),
    };
    let animations = |fixture: &Fixture| match &fixture.state().data {
        Some(Ok(data)) => data.animations.is_some(),
        _ => false,
    };
    fixture.app.tx.send(listed(current + 1)).unwrap();
    fixture.app.process_worker_messages(&fixture.ctx);
    assert!(!animations(&fixture), "the old geometry's list landed");
    fixture.app.tx.send(listed(current)).unwrap();
    fixture.app.process_worker_messages(&fixture.ctx);
    assert!(
        animations(&fixture),
        "the current geometry's list was dropped"
    );
}

/// An edit invalidates the preview while a load is in flight: the worker is
/// parsing the bytes from before the edit, so its result must not land.
#[test]
fn invalidating_drops_the_load_in_flight() {
    let Some(tags) = kit_root(GameId::Halo3) else {
        return;
    };
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

/// The kit's generation moved while the worker ran — a background scan
/// started, or a tag was created. The reply is stale and is dropped, but the
/// preview must ask again rather than wait on a request nobody will answer.
#[test]
fn a_result_dropped_for_a_generation_bump_is_requested_again() {
    let Some(tags) = kit_root(GameId::Halo3) else {
        return;
    };
    let rel = "objects/weapons/rifle/assault_rifle/assault_rifle.render_model";
    let Some(mut fixture) = fixture(&tags, "halo3_mcc", rel) else {
        return;
    };
    fixture.frame();
    let first = fixture.state().preview_load_id.expect("a worker started");
    let reply = wait_for_reply(&fixture);
    fixture.app.model.kits[0].generation = fixture.app.model.kits[0].generation.wrapping_add(1);
    fixture.app.tx.send(reply).unwrap();
    fixture.app.process_worker_messages(&fixture.ctx);
    assert!(
        fixture.state().data.is_none(),
        "a stale result was installed"
    );

    let key = fixture.key.clone();
    fixture
        .app
        .maybe_request_model_preview(0, &key, &fixture.ctx);
    let second = fixture.state().preview_load_id;
    assert!(
        second.is_some_and(|second| second != first),
        "the dropped request was never re-made: the preview waits for good"
    );
    fixture.frames_until_loaded();
}

// `load_model_preview` over synthetic tags: which group goes down which
// path, what each produces, and what each refuses with.
//
// Characterization, with no kit. The geometry is a Halo CE gbxmodel built
// from the definitions — the one render format whose vertices and triangles
// are plain tag blocks — and the references between tags resolve against a
// loose folder written to a temporary directory.

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
    crate::core::document::apply::apply_field_edit(tag, path, input)
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
    for (field, index) in [
        ("vertex0 index", "0"),
        ("vertex1 index", "1"),
        ("vertex2 index", "2"),
    ] {
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

fn load(
    tag: &TagFile,
    display_path: &str,
    source: Option<&TagSource>,
) -> Result<ModelPreviewData, String> {
    load_model_preview(
        tag,
        &entry(
            display_path,
            tag,
            TagEntryLocation::LooseFile(display_path.into()),
        ),
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
        load(&tag, "objects/thing/empty.gbxmodel", None)
            .err()
            .as_deref(),
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
    reference(
        &mut scenery,
        "object/model",
        b"mod2",
        "objects\\thing\\thing",
    );
    assert_eq!(
        load(&scenery, "objects/thing/thing.scenery", None)
            .err()
            .as_deref(),
        Some("Halo CE object preview requires a loaded source.")
    );
    let data = load(&scenery, "objects/thing/thing.scenery", Some(&source))
        .expect("the referenced gbxmodel previews");
    assert_eq!(data.source_key, "file:objects/thing/thing.scenery");
    assert_eq!(data.render_model_path, "objects\\thing\\thing");
    assert_one_triangle(&data.preview);

    reference(
        &mut scenery,
        "object/model",
        b"mod2",
        "objects\\thing\\missing",
    );
    let error = load(&scenery, "objects/thing/thing.scenery", Some(&source))
        .err()
        .expect("refused");
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
    set(
        &mut model,
        "render model",
        "objects\\thing\\thing.render_model",
    );
    assert_eq!(
        load(&model, "objects/thing/thing.model", None)
            .err()
            .as_deref(),
        Some("Render model preview requires a loaded loose-folder editing kit.")
    );
    let error = load(&model, "objects/thing/thing.model", Some(&source))
        .err()
        .expect("refused");
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
        ..Default::default()
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
    .err()
    .expect("refused");
    assert_eq!(error, "Scenario preview requires a loaded source.");
}

/// A model tag of no previewable kind falls through to the render-model
/// lookup, and a physics model with no shapes is refused by its builder.
#[test]
fn other_groups_take_the_paths_their_group_names() {
    let biped = new_tag_for("halo3_mcc", "biped");
    assert_eq!(
        load(&biped, "objects/thing/thing.biped", None)
            .err()
            .as_deref(),
        Some("Object preview requires a loaded source.")
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
            app.model.kits[0]
                .parsed_tags
                .insert(entry.key.clone(), document);
        }
        let ctx = egui::Context::default();
        // Not the preview tab: nothing is asked for.
        app.views[app.model.kits[0].id]
            .caches
            .model_previews
            .insert(entry.key.clone(), ModelPreviewState::default());
        app.maybe_request_model_preview(0, &entry.key, &ctx);
        assert!(
            app.views[app.model.kits[0].id].caches.model_previews[&entry.key]
                .preview_load_id
                .is_none()
        );

        app.views[app.model.kits[0].id]
            .caches
            .model_previews
            .get_mut(&entry.key)
            .unwrap()
            .active_tab = ModelTagPanelTab::ModelPreview;
        app.maybe_request_model_preview(0, &entry.key, &ctx);
        let state = &app.views[app.model.kits[0].id].caches.model_previews[&entry.key];
        let first = state.preview_load_id.expect("a worker started");
        assert!(state.data.is_none(), "the shells show while it parses");
        assert_eq!(state.loaded_key.as_deref(), Some(entry.key.as_str()));
        // Asking again while it runs starts nothing new.
        app.maybe_request_model_preview(0, &entry.key, &ctx);
        assert_eq!(
            app.views[app.model.kits[0].id].caches.model_previews[&entry.key].preview_load_id,
            Some(first)
        );

        let deadline = Instant::now() + Duration::from_secs(60);
        while app.views[app.model.kits[0].id].caches.model_previews[&entry.key]
            .data
            .is_none()
        {
            assert!(Instant::now() < deadline, "the preview never landed");
            app.process_worker_messages(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        let state = &app.views[app.model.kits[0].id].caches.model_previews[&entry.key];
        assert!(state.preview_load_id.is_none(), "the request is answered");
        assert_eq!(state.render_model_path.as_deref(), Some(relative));
        let data = state.data.as_ref().unwrap().as_ref().expect("it loads");
        assert_eq!(data.preview.vertices.len(), 3);
        let apex = if edited {
            [1.0, 4.0, 5.0]
        } else {
            [1.0, 2.0, 3.0]
        };
        assert_eq!(data.preview.bounds_max, apex, "edited: {edited}");
    }
}
