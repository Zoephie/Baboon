//! Kit tests: Halo CE's detail object collections read, laid out and built.

use super::super::super::harness::kit::{kit_preview, kit_root, kit_source};
use super::super::super::load::TagReader;
use super::placement::LATTICE;
use crate::core::game::GameId;

/// With `BLAM_TEST_HCEEK`: the grass field reads its types and plate, stands
/// every instance on Sapien's lattice at the collection's z offset, its frame
/// one of its sequence's, and builds a batch per type.
#[test]
fn a_halo_ce_collection_stands_its_sprites_on_the_lattice() {
    let Some(root) = kit_root(GameId::HaloCe) else {
        return;
    };
    let data = kit_preview(
        &root,
        GameId::HaloCe,
        "effects\\detail_objects\\grass_field\\grass_field",
        "detail_object_collection",
        b"dobc",
    );
    let set = data
        .preview
        .decorators
        .as_ref()
        .expect("the collection it was drawn from");
    let collection = set.halo_ce.as_ref().expect("a Halo CE collection");
    assert!(!collection.types.is_empty() && !collection.instances.is_empty());
    for instance in &collection.instances {
        for axis in 0..2 {
            let steps = instance.position[axis] / LATTICE;
            assert!((steps - steps.round()).abs() < 1e-3, "{steps}");
        }
        assert_eq!(instance.position[2], collection.z_offset);
        let sprites =
            collection.plate.sequences[collection.types[instance.type_index].sequence].len();
        assert!(instance.frame < sprites.max(1));
    }
    assert!(!data.preview.batches.is_empty());
    let placed: usize = (0..set.types.len()).map(|index| set.placed(index)).sum();
    assert_eq!(placed, collection.instances.len());
}

/// With `BLAM_TEST_HCEEK`: every kit collection reads and builds without a
/// panic.
#[test]
#[ignore = "reads every Halo CE detail object collection; run by hand"]
fn every_halo_ce_collection_reads() {
    let Some(source) = kit_source(GameId::HaloCe) else {
        return;
    };
    let crate::core::source::TagSource::LooseFolder { root, .. } = &source else {
        unreachable!()
    };
    let mut failures = Vec::new();
    let mut count = 0;
    let mut folders = vec![root.clone()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            if path
                .extension()
                .is_none_or(|extension| extension != "detail_object_collection")
            {
                continue;
            }
            count += 1;
            let reference = path
                .strip_prefix(root)
                .unwrap()
                .with_extension("")
                .to_string_lossy()
                .replace('/', "\\");
            let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let reader = TagReader::with_game_shaders(&source, false);
                let tag = reader
                    .load(&reference, "detail_object_collection", b"dobc")
                    .map_err(|error| error.to_string())?;
                let mut collection = super::read_halo_ce_collection(&tag, Some(&reader))?;
                collection.instances = super::scatter_halo_ce(&collection, Default::default());
                Ok::<_, String>(collection.instances.len())
            }));
            match read {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => failures.push(format!("{reference}: {error}")),
                Err(_) => failures.push(format!("{reference}: panicked")),
            }
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
