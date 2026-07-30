//! Separating the game's own packs from mods installed into the same tree.
//!
//! Baboon mounts the pak folder recursively, exactly as the game does, so a mod
//! in `Paks/~mods` -- or loose in `Paks` -- is mounted and wins every collision.
//! That is correct for reading, and it silently redefined "as the game ships it"
//! to mean "as this install currently loads it": every comparison against a
//! modded tag came back empty, including a mod compared against an earlier export
//! of itself.

use super::*;

const PAKS: &str = "/Users/camden/Halo/halo-campaign-evolved_pc/Meteorite/Content/Paks";

fn mount() -> Option<crate::source::LoadedSourceData> {
    if !std::path::Path::new(PAKS).exists() {
        eprintln!("skipping: Campaign Evolved not present");
        return None;
    }
    let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = crate::format::TagNameIndex::load_from_definitions(&defs);
    Some(
        crate::source::load_iostore_container_set(std::path::PathBuf::from(PAKS), &names, &defs)
            .expect("mount container set"),
    )
}

/// The game's own packs must not be mistaken for mods, or every tag would be
/// compared against nothing and the whole shipped layer would be empty.
#[test]
fn the_games_own_packs_are_not_mods() {
    let Some(loaded) = mount() else { return };
    let TagSource::IoStoreContainerSet {
        containers,
        shipped,
        ..
    } = &loaded.source
    else {
        panic!("not a container set");
    };
    let pakchunks = containers
        .iter()
        .filter(|container| container.chunk_label.starts_with("pakchunk"))
        .collect::<Vec<_>>();
    assert!(!pakchunks.is_empty(), "the install has pakchunk containers");
    for container in pakchunks {
        assert!(
            !container.is_mod,
            "{} is one of the game's own packs",
            container.chunk_label
        );
    }
    assert!(
        !shipped.is_empty(),
        "the shipped layer holds the game's own payloads"
    );
}

/// Every tag the browser serves from a mod must still resolve to the game's own
/// copy, or "what does this change about the game?" answers itself with
/// "nothing" -- which is exactly what a user reported after installing an
/// earlier export of their own mod.
#[test]
fn a_modded_tag_still_resolves_to_the_shipped_copy() {
    let Some(loaded) = mount() else { return };
    let TagSource::IoStoreContainerSet {
        containers,
        shipped,
        ..
    } = &loaded.source
    else {
        panic!("not a container set");
    };
    let mods = containers
        .iter()
        .enumerate()
        .filter(|(_, container)| container.is_mod)
        .map(|(index, container)| (index, container.chunk_label.clone()))
        .collect::<Vec<_>>();
    if mods.is_empty() {
        eprintln!("skipping: no mod is installed in this Paks folder");
        return;
    }
    eprintln!(
        "{} mod container(s): {}",
        mods.len(),
        mods.iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mod_indices = mods.iter().map(|(index, _)| *index).collect::<Vec<_>>();
    let mut checked = 0usize;
    for entry in &loaded.entries {
        let TagEntryLocation::Container {
            container,
            rel_path,
        } = &entry.location
        else {
            continue;
        };
        if !mod_indices.contains(container) {
            continue;
        }
        checked += 1;
        // The mod is what the mount resolved the tag to -- that part is correct,
        // it is what the game loads.
        let mounted = crate::source::read_entry(&loaded.source, entry)
            .unwrap_or_else(|error| panic!("read {} as mounted: {error}", entry.display_path));
        // ...and the game's own copy has to remain reachable beside it.
        let base = crate::source::read_shipped_entry(&loaded.source, entry)
            .unwrap_or_else(|error| panic!("read {} as shipped: {error}", entry.display_path));
        let Some(base) = base else {
            eprintln!(
                "{} exists only in the mod, nothing shipped to compare",
                entry.display_path
            );
            continue;
        };
        let shipped_container = shipped
            .container_for(rel_path)
            .expect("a shipped copy was just read");
        assert!(
            !mod_indices.contains(&shipped_container),
            "{} resolved its shipped copy to a mod",
            entry.display_path
        );
        assert_ne!(
            shipped_container, *container,
            "{} must read its baseline from a different container than the mod \
             serving it, or the comparison is the mod against itself",
            entry.display_path
        );
        // Whether the mod actually changed anything is up to the mod; what
        // matters is that both sides are now readable and distinct.
        let (base_bytes, mounted_bytes) = (
            base.write_to_bytes().expect("serialize shipped"),
            mounted.write_to_bytes().expect("serialize mounted"),
        );
        eprintln!(
            "{}: shipped {} bytes from container {shipped_container}, mounted {} bytes from \
             container {container} ({})",
            entry.display_path,
            base_bytes.len(),
            mounted_bytes.len(),
            if base_bytes == mounted_bytes {
                "identical"
            } else {
                "differs"
            }
        );
    }
    assert!(checked > 0, "at least one tag is served from a mod");
}

/// Exporting a mod over an installed copy of itself: the whole sequence the
/// export performs, on a copy of a real mod container so nothing in the install
/// is touched.
///
/// The mount holds the only reference to each archive — which is what lets the
/// mapping be released at all — and the container has to come back readable
/// afterwards, since the browser reads through it.
#[test]
fn a_mounted_container_can_be_released_replaced_and_remounted() {
    if !std::path::Path::new(PAKS).exists() {
        eprintln!("skipping: Campaign Evolved not present");
        return;
    }
    // A real mod container, copied out of the install. Any non-pakchunk container
    // will do; without one there is nothing index-less to exercise.
    let Some(donor) = std::fs::read_dir(PAKS)
        .expect("read Paks")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|path| {
            path.extension().is_some_and(|ext| ext == "utoc")
                && !path
                    .file_stem()
                    .is_some_and(|stem| stem.to_string_lossy().starts_with("pakchunk"))
                && path.file_name().is_some_and(|name| name != "global.utoc")
        })
    else {
        eprintln!("skipping: no mod container in this Paks folder to copy");
        return;
    };
    let scratch = std::env::temp_dir().join(format!("baboon-remount-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("scratch dir");
    let target = scratch.join("copied_mod_P.utoc");
    for extension in ["utoc", "ucas", "pak"] {
        let from = donor.with_extension(extension);
        if from.exists() {
            std::fs::copy(&from, target.with_extension(extension)).expect("copy container");
        }
    }

    let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = crate::format::TagNameIndex::load_from_definitions(&defs);
    // Mounted against the real Paks, as the app does: an override container ships
    // no directory index, so only the containers it overrides can name its chunks.
    let mut loaded = crate::source::load_iostore_container(
        target.clone(),
        Some(std::path::PathBuf::from(PAKS)),
        &names,
        &defs,
    )
    .expect("mount the copied mod");

    assert_eq!(
        crate::source::mounted_containers_at(&loaded.source, &target),
        vec![0],
        "the export target is recognised as a mounted container"
    );
    let entry = loaded.entries.first().cloned().expect("the mod carries a tag");
    crate::source::read_entry(&loaded.source, &entry).expect("read while mapped");

    let root = match &loaded.source {
        TagSource::IoStoreContainerSet { root, .. } => root.clone(),
        _ => panic!("not a container set"),
    };
    {
        let TagSource::IoStoreContainerSet { containers, .. } = &mut loaded.source else {
            panic!("not a container set");
        };
        assert_eq!(
            std::sync::Arc::strong_count(&containers[0].archive),
            1,
            "the mount holds the only reference, so the mapping can be released"
        );
        std::sync::Arc::get_mut(&mut containers[0].archive)
            .expect("uniquely held archive")
            .release_partition();
        assert!(!containers[0].archive.is_partition_mapped());
    }
    assert!(
        crate::source::read_entry(&loaded.source, &entry).is_err(),
        "a read through a released partition is refused rather than served stale"
    );

    // What the release is for: the container is rewritten where it stands.
    let mut writer = blam_tags::iostore::writer::OverrideContainerWriter::new("../../../");
    let mut id = [0u8; 12];
    id[..8].copy_from_slice(&0x0bad_f00d_dead_beefu64.to_le_bytes());
    id[11] = blam_tags::iostore::CHUNK_TYPE_BULK_DATA;
    writer.add_chunk(blam_tags::iostore::FIoChunkId(id), vec![7u8; 2048]);
    writer.write(&target).expect("replace the mounted container");

    let containers_snapshot = match &loaded.source {
        TagSource::IoStoreContainerSet { containers, .. } => containers.clone(),
        _ => panic!("not a container set"),
    };
    let reopened = crate::source::reopen_container_archive(&root, &containers_snapshot, 0)
        .expect("reopen the replaced container");
    assert!(
        reopened.is_partition_mapped(),
        "the remounted container is readable again"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

/// The export target has to be recognised as a mounted container before the
/// writer runs, since that is what decides whether a mapping must be released.
#[test]
fn an_export_over_a_mounted_container_is_detected() {
    let Some(loaded) = mount() else { return };
    let TagSource::IoStoreContainerSet { containers, .. } = &loaded.source else {
        panic!("not a container set");
    };
    let target = containers
        .first()
        .expect("the install mounts containers")
        .utoc_path
        .clone();
    assert_eq!(
        crate::source::mounted_containers_at(&loaded.source, &target),
        vec![0],
        "exporting over a mounted container is detected"
    );
    let free = target.with_file_name("a-name-nothing-has-taken_P.utoc");
    assert!(
        crate::source::mounted_containers_at(&loaded.source, &free).is_empty(),
        "an unused name is free to write"
    );
}
