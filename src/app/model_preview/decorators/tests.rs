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
    every_set_reads(GameId::Halo3);
}

/// With `BLAM_TEST_HREK`: as [`every_halo_3_set_reads`], Reach's sets, whose
/// render shader options Reach renamed.
#[test]
#[ignore = "reads every Reach decorator set; run by hand"]
fn every_reach_set_reads() {
    every_set_reads(GameId::HaloReach);
}

/// With `BLAM_TEST_H4EK`: as [`every_halo_3_set_reads`], Halo 4's sets.
#[test]
#[ignore = "reads every Halo 4 decorator set; run by hand"]
fn every_halo_4_set_reads() {
    every_set_reads(GameId::Halo4);
}

fn every_set_reads(game: GameId) {
    let Some(source) = kit_source(game) else {
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
            Ok(Ok((set, notes))) => {
                if let Some(problem) = strips_disagree(&source, &reference, &set) {
                    failures.push(format!("{reference}: {problem}"));
                }
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

/// Where `set`'s strips disagree with the engine's own reading of its
/// models' decorator meshes (`extract_decorator_subparts`): a type's
/// triangles from its strip counted against the engine's for the same
/// placement, and any strip restart, which the instanced draw cannot keep.
fn strips_disagree(
    source: &crate::core::source::TagSource,
    reference: &str,
    set: &super::DecoratorSet,
) -> Option<String> {
    let reader = TagReader::with_game_shaders(source, false);
    let tag = reader.load(reference, "decorator_set", b"dctr").ok()?;
    let root = tag.root();
    let names = ["render model", "Base", "Lod2", "Lod3", "Lod4"];
    let paths: Vec<String> = names
        .iter()
        .filter_map(|name| root.read_tag_ref_path(name))
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
        .collect();
    let mut model = 0;
    // A further level draws the nearest's subparts, whatever it names them.
    let mut subparts: Option<Vec<Option<usize>>> = None;
    for path in paths {
        let Ok(tag) = reader.load(&path, "render_model", b"mode") else {
            continue;
        };
        let Some(geometry) = blam_tags::render_model::extract_decorator_subparts(&tag) else {
            continue;
        };
        let placements: Vec<String> = super::super::common::fields::elements(tag.root(), "instance placements")
            .iter()
            .map(|element| element.read_string_id("name").unwrap_or_default())
            .collect();
        let meshes = set.level_meshes(model)?;
        model += 1;
        let found = subparts.get_or_insert_with(|| {
            set.types
                .iter()
                .map(|kind| placements.iter().position(|name| name.eq_ignore_ascii_case(&kind.mesh_name)))
                .collect()
        });
        for ((kind, mesh), subpart) in set.types.iter().zip(meshes).zip(found.iter()) {
            if mesh.strip.contains(&u32::MAX) {
                return Some(format!("{path}: {} restarts its strip", kind.mesh_name));
            }
            let Some(engine) = subpart.and_then(|index| geometry.subpart_indices.get(index)) else {
                continue;
            };
            if engine.len() != mesh.indices.len() {
                return Some(format!(
                    "{path}: {} has {} triangles from its strip, the engine {}",
                    kind.mesh_name,
                    mesh.indices.len() / 3,
                    engine.len() / 3
                ));
            }
        }
    }
    None
}

/// With `BLAM_TEST_HREK`: Reach's arid grass names three levels and
/// decimates the nearest. Its later two ranges are empty in the tag, so
/// the postprocess gives them their own: 8 past the end before, starting a
/// quarter of the way from it. Its pattern is the 2 × 2 dither, its
/// throttle the default one-player 1.
#[test]
fn reach_levels_read_as_the_postprocess_leaves_them() {
    let Some(source) = kit_source(GameId::HaloReach) else {
        return;
    };
    let reader = TagReader::with_game_shaders(&source, false);
    let tag = reader
        .load("levels\\shared\\decorators\\grass_arid\\grass_arid", "decorator_set", b"dctr")
        .unwrap();
    let mut notes = Vec::new();
    let set = super::read_decorator_set(&tag, Some(&reader), &mut notes).unwrap();
    assert_eq!(set.level_count(), 3, "{notes:?}");
    let detail = set.detail.as_ref().expect("Reach's levels");
    assert_eq!(detail.last, 2);
    let ranges: Vec<[f32; 2]> = detail.slots[..3].iter().map(|slot| [slot.start, slot.end]).collect();
    assert_eq!(ranges, [[6.0, 10.0], [16.0, 18.0], [24.0, 26.0]]);
    assert!(detail.decimation.on && detail.block_size == 3.0 && detail.throttle == 1.0);
    for model in 0..3 {
        assert!(set.level_meshes(model).unwrap().iter().all(|mesh| mesh.strip.len() > 2), "level {model}");
    }
    let stipple = set.stipple.as_ref().expect("default bitmaps[9]");
    assert_eq!(stipple.size, [8, 8]);
    assert_eq!(&stipple.values[..2], [0, 3]);
    assert_eq!(&stipple.values[8..10], [2, 1]);
}

/// With `BLAM_TEST_H4EK`: Halo 4's juniper names all four levels, each
/// range as the tag has it, and its pattern is the 4 × 4 Bayer dither.
#[test]
fn halo_4_levels_read_as_the_tag_has_them() {
    let Some(source) = kit_source(GameId::Halo4) else {
        return;
    };
    let reader = TagReader::with_game_shaders(&source, false);
    let tag = reader
        .load(
            "levels\\multi\\ca_gore_valley\\decorators\\ca_longbow_juniper_bush\\ca_longbow_juniper_bush",
            "decorator_set",
            b"dctr",
        )
        .unwrap();
    let mut notes = Vec::new();
    let set = super::read_decorator_set(&tag, Some(&reader), &mut notes).unwrap();
    assert_eq!(set.level_count(), 4, "{notes:?}");
    let detail = set.detail.as_ref().expect("Halo 4's levels");
    assert_eq!(detail.rule, super::lod::LodRule::Halo4);
    let ranges: Vec<[f32; 2]> = detail.slots.iter().map(|slot| [slot.start, slot.end]).collect();
    assert_eq!(ranges, [[4.0, 5.0], [8.0, 9.0], [12.0, 13.0], [15.0, 16.0]]);
    assert!(detail.decimation.on);
    let stipple = set.stipple.as_ref().expect("default bitmaps[9]");
    let mut values = stipple.values.clone();
    values.sort_unstable();
    values.dedup();
    assert_eq!(values, (0..16).collect::<Vec<u8>>());
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
