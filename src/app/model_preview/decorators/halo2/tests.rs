//! Kit tests: Halo 2's decorator sets read, laid out and built.

use super::super::super::harness::kit::{kit_preview, kit_root, kit_source};
use super::super::super::load::TagReader;
use super::ClassKind;
use crate::core::game::GameId;

/// With `BLAM_TEST_H2EK`: Delta Halo's ground cover reads its models and
/// sprite permutations, lays its patch, and builds a batch per permutation
/// that has any, each drawn with its own shader.
#[test]
fn a_halo_2_set_builds_its_models_and_sprites() {
    let Some(root) = kit_root(GameId::Halo2) else {
        return;
    };
    let data = kit_preview(
        &root,
        GameId::Halo2,
        "scenarios\\decorators\\dcr_ground\\dcr_ground",
        "decorator_set",
        b"DECR",
    );
    assert!(data.notes.is_empty(), "{:?}", data.notes);
    let set = data
        .preview
        .decorators
        .as_ref()
        .expect("the set it was drawn from");
    let halo2 = set.halo2.as_ref().expect("a Halo 2 set");
    assert_eq!(halo2.models.len(), 15);
    assert!(
        halo2
            .classes
            .iter()
            .any(|class| class.kind == Some(ClassKind::Model))
    );
    assert_eq!(set.types.len(), halo2.types().count());
    assert!(!halo2.records.is_empty());
    let placed: usize = (0..set.types.len()).map(|index| set.placed(index)).sum();
    assert_eq!(placed, halo2.records.len());
    assert!(!data.preview.batches.is_empty());
    for batch in &data.preview.batches {
        assert!((batch.material_index as usize) < halo2.shaders.len());
    }
    assert!(
        data.preview
            .materials
            .iter()
            .all(|material| material.shader_group == u32::from_be_bytes(*b"shad"))
    );
}

/// With `BLAM_TEST_H2EK`: every kit set reads and lays out without a panic,
/// and knows every class's type by name.
#[test]
#[ignore = "reads every Halo 2 decorator set; run by hand"]
fn every_halo_2_set_reads() {
    let Some(source) = kit_source(GameId::Halo2) else {
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
                .is_none_or(|extension| extension != "decorator_set")
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
                    .load(&reference, "decorator_set", b"DECR")
                    .map_err(|error| error.to_string())?;
                let mut notes = Vec::new();
                let mut set = super::read_halo_2_set(&tag, Some(&reader), &mut notes)?;
                set.records = super::scatter_halo_2(&set, Default::default());
                let built = super::bake_halo_2(&set);
                Ok::<_, String>((notes, built.batches.len()))
            }));
            match read {
                Ok(Ok((notes, _))) if !notes.is_empty() => {
                    failures.push(format!("{reference}: {notes:?}"))
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("{reference}: {error}"),
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
