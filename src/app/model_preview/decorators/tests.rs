//! Kit tests: real decorator sets read and laid out.

use super::super::harness::kit::{kit_preview, kit_root, kit_source};
use super::super::load::TagReader;
use crate::core::game::GameId;

/// With `BLAM_TEST_H3EK`: the Isolation dead grass reads by its option
/// names, its four types each find their mesh, and the patch it lays stands
/// every decorator in its type's ranges, its grass two-sided.
#[test]
fn a_halo_3_set_lays_its_types_on_the_ground() {
    let Some(root) = kit_root(GameId::Halo3) else {
        return;
    };
    let data = kit_preview(
        &root,
        GameId::Halo3,
        "levels\\shared\\decorators\\outskirts_tall_grass\\isolation_deadgrass",
        "decorator_set",
        b"dctr",
    );
    assert!(data.notes.is_empty(), "{:?}", data.notes);
    let set = data
        .preview
        .decorators
        .as_ref()
        .expect("the set it was drawn from");
    assert!(set.two_sided);
    assert_eq!(set.types.len(), 4);
    assert!(
        set.meshes.iter().all(|mesh| !mesh.indices.is_empty()),
        "every type finds its mesh"
    );
    assert!(!set.placements.is_empty());
    for placement in &set.placements {
        let kind = &set.types[placement.type_index];
        let within = |value: f32, [low, high]: [f32; 2]| {
            value >= low.min(high) - 1e-5 && value <= low.max(high) + 1e-5
        };
        assert!(within(placement.scale, kind.scale) && within(placement.motion, kind.wind));
        assert!(within(placement.position[2], kind.hover));
    }
    // A batch per type, its region the type's.
    assert_eq!(data.preview.regions.len(), 4);
    assert!(!data.preview.batches.is_empty() && data.preview.batches.len() <= 4);
}

/// With `BLAM_TEST_H3EK`: a new layout lays the patch out again in place:
/// other placements and new geometry, the same textures, and nothing done
/// for the layout it already has.
#[test]
fn a_new_layout_lays_the_patch_out_again_in_place() {
    let Some(root) = kit_root(GameId::Halo3) else {
        return;
    };
    let mut data = kit_preview(
        &root,
        GameId::Halo3,
        "levels\\shared\\decorators\\outskirts_tall_grass\\isolation_deadgrass",
        "decorator_set",
        b"dctr",
    );
    let (mesh, textures) = (data.mesh_id, data.textures_id);
    super::relayout(&mut data, super::Layout::default());
    assert_eq!(data.mesh_id, mesh, "the layout it has changes nothing");
    let layout = super::Layout {
        count: 40,
        seed: 9,
        ..Default::default()
    };
    super::relayout(&mut data, layout);
    let set = data.preview.decorators.as_ref().unwrap();
    assert_eq!((set.layout, set.placements.len()), (layout, 40));
    assert_ne!(data.mesh_id, mesh);
    assert_eq!(data.textures_id, textures);
    assert_eq!(data.preview.game, Some(GameId::Halo3));
}

/// With `BLAM_TEST_H3EK`: every kit set reads without a panic and knows its
/// render shader by name; one that cannot draw says why. The kit has sets
/// whose types name meshes their model has lost, and one model
/// (`ivy_clump`) whose `tgbl` chunk is damaged.
#[test]
#[ignore = "reads every Halo 3 decorator set; run by hand"]
fn every_halo_3_set_reads() {
    let Some(source) = kit_source(GameId::Halo3) else {
        return;
    };
    let crate::core::source::TagSource::LooseFolder { root, .. } = &source else {
        unreachable!()
    };
    let mut failures = Vec::new();
    let mut count = 0;
    for path in walk(root, "decorator_set") {
        count += 1;
        let relative = path.strip_prefix(root).unwrap().with_extension("");
        let reference = relative.to_string_lossy().replace('/', "\\");
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let reader = TagReader::with_game_shaders(&source, false);
            let tag = reader
                .load(&reference, "decorator_set", b"dctr")
                .map_err(|error| error.to_string())?;
            let mut notes = Vec::new();
            super::read_decorator_set(&tag, Some(&reader), &mut notes).map(|set| (set, notes))
        }));
        match read {
            Ok(Ok((_, notes))) => {
                if notes.iter().any(|note| note.contains("render shader")) {
                    failures.push(format!("{reference}: {notes:?}"));
                } else if !notes.is_empty() {
                    eprintln!("{reference}: {notes:?}");
                }
            }
            Ok(Err(error)) => eprintln!("{reference}: {error}"),
            Err(_) => failures.push(format!("{reference}: panicked")),
        }
    }
    assert!(count > 0);
    assert!(
        failures.is_empty(),
        "{} of {count}:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every file under `root` with the extension `extension`.
fn walk(root: &std::path::Path, extension: &str) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|found| found == extension) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}
