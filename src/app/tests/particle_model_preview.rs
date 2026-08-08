//! `particle_model` tags get a working Render Model tab.
//!
//! blam-tags owns the decode (splitting the merged triangle strip at the
//! `m_gpu_data/m_variants` boundaries, decompressing through the
//! compression bounds) and is tested there. What this asserts is
//! Baboon's half:
//!
//! - the tab pair and viewport actually appear for `pmdf` / `PRTM`,
//! - **without** widening [`is_model_group`], whose other job is deciding
//!   whether a `tag_reference` is an object's model link — a `particle`
//!   tag's `Model` → `pmdf` field would be misread as one,
//! - each JMI object becomes its own preview region, so the region list
//!   doubles as an object toggle,
//! - the geometry the viewport uploads is right way round — batches
//!   index inside the vertex buffer and face normals agree with the
//!   stored vertex normals.
//!
//! Skips silently when the corresponding tag set is absent.

use std::path::PathBuf;

use blam_tags::TagFile;

use crate::app::editor::{is_model_group, is_previewable_geometry_group};
use crate::app::model_preview::loading::build_particle_model_preview;
use crate::app::model_preview::RenderModelPreview;

/// Root of an extracted MCC tag set, via `BLAM_TEST_<KIT>_TAGS` or the
/// conventional local layout.
fn kit_tags(kit: &str) -> Option<PathBuf> {
    let var = format!("BLAM_TEST_{}_TAGS", kit.to_uppercase());
    if let Ok(p) = std::env::var(&var) {
        let p = PathBuf::from(p);
        return p.is_dir().then_some(p);
    }
    let home = std::env::var("HOME").ok()?;
    let p = PathBuf::from(home)
        .join("Halo")
        .join(format!("{kit}_mcc"))
        .join("tags");
    p.is_dir().then_some(p)
}

fn names() -> crate::format::TagNameIndex {
    let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    crate::format::TagNameIndex::load_from_definitions(&defs)
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
    let names = names();
    for group in [b"pmdf", b"PRTM"] {
        let tag = u32::from_be_bytes(*group);
        let label = String::from_utf8_lossy(group).into_owned();
        assert!(
            is_previewable_geometry_group(tag, &names),
            "`{label}` must open the Render Model tab",
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

/// A multi-object gen3 tag: one region per JMI object, batches wired to
/// those regions, and geometry the right way round.
#[test]
fn gen3_objects_become_regions_with_valid_geometry() {
    let Some(tags) = kit_tags("haloreach") else {
        return;
    };
    let path =
        tags.join("fx/particles/models/debris/generic_shards/generic_shards.particle_model");
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
        assert_eq!(region.name, batch.region_name, "batch must target its region");
        assert!(
            region.permutations.contains(&batch.permutation_name),
            "batch permutation `{}` is not selectable in region `{}`",
            batch.permutation_name,
            region.name,
        );
        assert!(batch.index_count > 0, "region `{}` has an empty batch", region.name);
    }

    // Every batch must address inside the shared buffers, or the
    // renderer reads past the end.
    for batch in &preview.batches {
        let end = (batch.index_start + batch.index_count) as usize;
        assert!(end <= preview.indices.len(), "batch range past the index buffer");
        for &i in &preview.indices[batch.index_start as usize..end] {
            assert!((i as usize) < preview.vertices.len(), "index past the vertex buffer");
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
            "can_1", "can_2", "can_3", "can_4", "can_5", "paper_1", "paper_2", "paper_3",
            "butt_1", "butt_2",
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
    assert!(!preview.indices.is_empty(), "single-object preview must have geometry");
}

/// Drive the real UI entry point, not the builder underneath it.
///
/// `load_model_preview` is what the panel calls, and it derives the
/// object-naming stem from `entry.display_path` rather than being handed
/// one. Asserting through it is what catches a stem derivation that
/// silently yields `""` (every object would become `_1`, `_2`, …) or
/// keeps the `.particle_model` extension.
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
    let names = names();
    let entry = crate::source::TagEntry {
        key: format!("file:{}", path.display()),
        display_path: "fx/particles/models/debris/falling_leaves/falling_leaves.particle_model"
            .to_owned(),
        group_tag: tag.header.group_tag,
        group_name: Some("particle_model".to_owned()),
        location: crate::source::TagEntryLocation::LooseFile(path.clone()),
    };

    let data = crate::app::model_preview::loading::load_model_preview(
        &tag, &entry, &names, None, false,
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
    let names = names();
    let mut checked = 0usize;
    let mut objects = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for (kit, game) in kits {
        let Some(root) = kit_tags(kit) else { continue };
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
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
                let display = rel.to_string_lossy().replace('\\', "/");
                let tag_entry = crate::source::TagEntry {
                    key: format!("file:{}", path.display()),
                    display_path: display.clone(),
                    group_tag: tag.header.group_tag,
                    group_name: Some("particle_model".to_owned()),
                    location: crate::source::TagEntryLocation::LooseFile(path.clone()),
                };
                checked += 1;
                match crate::app::model_preview::loading::load_model_preview(
                    &tag, &tag_entry, &names, None, false,
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
