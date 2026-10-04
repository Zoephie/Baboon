use super::*;

fn failure(holders: Vec<ContainerHolder>, unattributed: usize) -> ContainerWriteFailure {
    ContainerWriteFailure {
        phase: LeasePhase::Unmap,
        file: PathBuf::from("D:/Game/Paks/~mods/mymod_P.ucas"),
        label: "mymod_P".to_owned(),
        holders,
        unattributed,
        io: None,
    }
}

#[test]
fn a_named_holder_is_named_and_nothing_is_invented() {
    let text = failure(
        vec![ContainerHolder::ChimpWorld {
            workspace: "Paks (12 packs)".to_owned(),
            doing: "indexing package types".to_owned(),
        }],
        0,
    )
    .to_string();
    assert!(text.contains("mymod_P.ucas"), "{text}");
    assert!(text.contains("releasing the container"), "{text}");
    assert!(text.contains("indexing package types"), "{text}");
    assert!(!text.contains("cannot say"), "{text}");
}

#[test]
fn an_unattributed_hold_says_so_rather_than_guessing() {
    // The one thing this message must never do is send the user to close
    // something Baboon has no evidence about.
    let text = failure(Vec::new(), 2).to_string();
    assert!(text.contains("2 background reader(s)"), "{text}");
    assert!(text.contains("cannot say which"), "{text}");
    assert!(!text.contains("try again in a moment"), "{text}");
}

#[test]
fn a_partial_attribution_reports_both_halves() {
    let text = failure(
        vec![ContainerHolder::Job {
            workspace: "Paks (12 packs)".to_owned(),
            job: "a bulk tag extraction",
        }],
        2,
    )
    .to_string();
    assert!(text.contains("a bulk tag extraction is running"), "{text}");
    assert!(text.contains("2 further reader(s)"), "{text}");
}

#[test]
fn a_concurrent_write_is_refused_by_name() {
    let text = ContainerWriteFailure {
        phase: LeasePhase::Acquire,
        file: PathBuf::from("D:/Game/Paks/~mods/mymod_P.utoc"),
        label: "mymod_P".to_owned(),
        holders: vec![ContainerHolder::ConcurrentWrite],
        unattributed: 0,
        io: None,
    }
    .to_string();
    assert!(text.contains("another write to these files"), "{text}");
}

#[test]
fn staging_keeps_the_stem_so_the_container_id_is_the_real_one() {
    // The writer hashes the file stem into the TOC's container id, so a
    // staging file named anything else ships a container declaring itself
    // under a name nothing will ever look for.
    let output = PathBuf::from("D:/Game/Paks/~mods/mymod_P.utoc");
    let staging = staging_utoc_for(&output);
    assert_eq!(staging.file_name(), output.file_name());
    // One level down, inside the output's own folder, so the swap is a
    // same-volume rename.
    assert_eq!(staging.parent().and_then(Path::parent), output.parent());
    let directory = staging
        .parent()
        .and_then(Path::file_name)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(directory.starts_with(".baboon-export-"), "{directory}");
}

/// The invariant the whole lease exists for, stated against the OS.
///
/// Needs no game files: `OverrideContainerWriter` produces a valid
/// `.utoc`/`.ucas`/`.pak` triplet from nothing. Windows-only because it is
/// a Windows rule — a file with a mapped section refuses to be truncated
/// (`ERROR_USER_MAPPED_FILE`, os error 1224), which is exactly what an
/// export over an installed mod does, and what every other platform
/// permits. There is no portable way to observe it.
#[cfg(windows)]
#[test]
fn a_mapped_partition_refuses_truncation_until_it_is_released() {
    let scratch = std::env::temp_dir().join(format!(
        "baboon-lease-mapping-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&scratch).expect("scratch dir");
    let utoc = scratch.join("leased_P.utoc");
    let mut writer = blam_tags::iostore::writer::OverrideContainerWriter::new("../../../");
    let mut id = [0u8; 12];
    id[..8].copy_from_slice(&0x0bad_f00d_dead_beefu64.to_le_bytes());
    id[11] = blam_tags::iostore::CHUNK_TYPE_BULK_DATA;
    writer.add_chunk(blam_tags::iostore::FIoChunkId(id), vec![7u8; 2048]);
    writer.write(&utoc).expect("write a container to lease");

    let mut archive = blam_tags::iostore::IoStoreArchive::open(&utoc).expect("open");
    assert!(archive.is_partition_mapped());
    let ucas = utoc.with_extension("ucas");
    let truncate = || {
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&ucas)
    };
    assert!(
        truncate().is_err(),
        "a mapped .ucas cannot be replaced — this is os error 1224, the whole reason the \
             lease exists"
    );

    archive.release_partition();

    assert!(!archive.is_partition_mapped());
    assert!(
        truncate().is_ok(),
        "once the mapping is released the file can be replaced"
    );
    drop(archive);
    let _ = fs::remove_dir_all(&scratch);
}

/// The other half of the mapping story, and the one every in-place write
/// actually depends on.
///
/// `AppendInPlace` leaves every Chimp `World` mounted and its `.ucas`
/// mapped, then appends to that file and renames a new `.utoc` over the old
/// one. Truncation is refused under a mapping — the test above is that —
/// but append and rename are different operations, and the whole lease
/// design rests on them being permitted. Nothing pinned that.
#[cfg(windows)]
#[test]
fn a_mapped_partition_still_allows_append_and_a_utoc_rename() {
    let scratch = std::env::temp_dir().join(format!(
        "baboon-lease-append-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&scratch).expect("scratch dir");
    let utoc = scratch.join("leased_P.utoc");
    let mut writer = blam_tags::iostore::writer::OverrideContainerWriter::new("../../../");
    let mut id = [0u8; 12];
    id[..8].copy_from_slice(&0x0bad_f00d_dead_beefu64.to_le_bytes());
    id[11] = blam_tags::iostore::CHUNK_TYPE_BULK_DATA;
    writer.add_chunk(blam_tags::iostore::FIoChunkId(id), vec![7u8; 2048]);
    writer.write(&utoc).expect("write a container to lease");

    let archive = blam_tags::iostore::IoStoreArchive::open(&utoc).expect("open");
    assert!(archive.is_partition_mapped());
    let ucas = utoc.with_extension("ucas");
    let before = fs::metadata(&ucas).expect("ucas exists").len();

    // Appending while mapped: what `append_ucas_items` does.
    {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&ucas)
            .expect("a mapped .ucas opens for append");
        file.write_all(&[0xAB; 64])
            .expect("a mapped .ucas accepts appended bytes");
    }
    assert_eq!(
        fs::metadata(&ucas).expect("ucas exists").len(),
        before + 64,
        "the appended bytes landed"
    );

    // Renaming a fresh .utoc over the open one: what `atomic_replace_file`
    // does. The .utoc is read into an owned buffer at open and never held.
    let staged = scratch.join("staged.utoc");
    fs::copy(&utoc, &staged).expect("stage a replacement");
    fs::rename(&staged, &utoc).expect("a .utoc can be replaced while its .ucas is mapped");

    drop(archive);
    let _ = fs::remove_dir_all(&scratch);
}

/// Releasing a mounted container's mapping works only while the mount
/// holds the only reference to its archive. A clone held anywhere else (a
/// job that snapshots the source) keeps the `.ucas` mapped whatever the
/// mount does, so the unmap refuses and says what holds it: a running
/// job it can see by name, otherwise a count. Synthetic container; runs
/// on every OS (only Windows refuses the write itself, see above).
#[test]
fn a_second_holder_of_a_mounted_archive_refuses_the_unmap() {
    let scratch = std::env::temp_dir().join(format!(
        "baboon-lease-holder-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&scratch).expect("scratch dir");
    let utoc = scratch.join("held_P.utoc");
    let mut writer = blam_tags::iostore::writer::OverrideContainerWriter::new("../../../");
    let mut id = [0u8; 12];
    id[..8].copy_from_slice(&0x0bad_f00d_dead_beefu64.to_le_bytes());
    id[11] = blam_tags::iostore::CHUNK_TYPE_BULK_DATA;
    writer.add_chunk(blam_tags::iostore::FIoChunkId(id), vec![7u8; 2048]);
    writer.write(&utoc).expect("write a container to mount");

    // Mounted the way the loader mounts it; the container carries no tags,
    // so the set is assembled here rather than discovered.
    let archive = blam_tags::iostore::IoStoreArchive::open(&utoc).expect("open");
    let mut app = Baboon::for_test();
    app.install_loaded_source(crate::core::source::LoadedSourceData {
        label: "Paks".to_owned(),
        source: TagSource::IoStoreContainerSet {
            root: scratch.clone(),
            containers: vec![crate::core::source::MountedContainer {
                utoc_path: utoc.clone(),
                chunk_label: crate::core::source::container_chunk_label(&utoc),
                is_mod: true,
                archive: std::sync::Arc::new(archive),
            }],
            index: Default::default(),
            packages: Default::default(),
            shipped: Default::default(),
        },
        names: TagNameIndex::default(),
        game: Some(GameId::CampaignEvolved),
        entries: Vec::new(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    });
    let archive = |app: &Baboon| match &app.kits[0].source.as_ref().unwrap().source {
        TagSource::IoStoreContainerSet { containers, .. } => {
            std::sync::Arc::clone(&containers[0].archive)
        }
        _ => panic!("not a container set"),
    };
    let unmap = |app: &mut Baboon| {
        let mut lease = app
            .acquire_container_write_lease(&utoc, ContainerWriteMode::Replace)
            .expect("lease");
        let result = app.unmap_leased_containers(&mut lease);
        let mapped_meanwhile = archive(app).is_partition_mapped();
        let _ = app.release_container_write_lease_inner(lease, ContainerWriteOutcome::Unchanged);
        (result, mapped_meanwhile)
    };

    // A clone held while a job that snapshots the source is running.
    let held = archive(&app);
    app.poke_direct_running = true;
    let (result, mapped) = unmap(&mut app);
    let failure = result.expect_err("a held archive cannot be released");
    assert_eq!(failure.phase, LeasePhase::Unmap);
    assert!(
        matches!(
            failure.holders.as_slice(),
            [ContainerHolder::Job {
                job: "a runtime poke",
                ..
            }]
        ),
        "{failure}"
    );
    assert_eq!(failure.unattributed, 0);
    assert!(mapped, "nothing was released");

    // The same clone with no job to name: counted, not guessed at.
    app.poke_direct_running = false;
    let (result, _) = unmap(&mut app);
    let failure = result.expect_err("still held");
    assert!(failure.holders.is_empty(), "{failure}");
    assert_eq!(failure.unattributed, 1);

    // The only holder is the mount: released, then reopened by the lease.
    drop(held);
    let (result, mapped) = unmap(&mut app);
    assert!(result.is_ok());
    assert!(!mapped, "released while the lease held it");
    assert!(archive(&app).is_partition_mapped(), "reopened on release");
    drop(app);
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn the_triplet_is_the_three_files_the_engine_loads() {
    let files = container_triplet(Path::new("D:/Game/Paks/~mods/mymod_P.utoc"));
    let names: Vec<String> = files
        .iter()
        .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["mymod_P.utoc", "mymod_P.ucas", "mymod_P.pak"]);
}
