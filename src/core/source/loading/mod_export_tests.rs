use super::*;

static PAKS: std::sync::LazyLock<&'static str> =
    std::sync::LazyLock::new(|| crate::test_kits::leak(crate::test_kits::ce_paks()));

/// End-to-end check of what Export Mod actually writes, short of the game
/// loading it: take a real container tag, change a byte, write an override
/// container, then read the tag back out of that container.
///
/// Reported as "it creates the pak but ingame nothing happens", and the
/// in-game load has never been verified on this machine — so this pins down
/// whether the artifact carries the edit at all.
#[test]
fn exported_mod_container_carries_the_edited_bytes() {
    if !Path::new(*PAKS).exists() {
        eprintln!("skipping: {} not present", *PAKS);
        return;
    }
    let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = TagNameIndex::load_from_definitions(&defs);
    let loaded =
        load_iostore_container_set(PathBuf::from(*PAKS), &names, &defs).expect("mount");
    let TagSource::IoStoreContainerSet { ref containers, .. } = loaded.source else {
        panic!("expected a container set");
    };

    // A tag whose bytes we can perturb without changing its length, so the
    // export takes the common same-size path.
    let (container, rel_path, original) = loaded
        .entries
        .iter()
        .find_map(|entry| match &entry.location {
            TagEntryLocation::Container {
                container,
                rel_path,
            } => {
                let archive = &containers.get(*container)?.archive;
                let bytes = archive.read(rel_path).ok()?;
                (bytes.len() > 64).then(|| (*container, rel_path.clone(), bytes))
            }
            _ => None,
        })
        .expect("a readable container tag");

    let mut edited = original.clone();
    let last = edited.len() - 1;
    edited[last] ^= 0xFF;

    let archive = containers[container].archive.clone();
    let dir = std::env::temp_dir().join(format!("baboon-modexport-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let out = dir.join("mymod-WinGDK_P.utoc");
    blam_tags::iostore::writer::write_mod_container_ex(
        &[(archive.as_ref(), rel_path.as_str(), edited.as_slice())],
        &[],
        &out,
    )
    .expect("write override container");

    // The game discovers containers by scanning `Paks/*.pak`, so all three
    // files have to be there — a missing stub is a mod that never loads.
    for ext in ["utoc", "ucas", "pak"] {
        let path = out.with_extension(ext);
        assert!(path.is_file(), "{} was not written", path.display());
        eprintln!(
            "{}: {} bytes",
            path.file_name().unwrap().to_string_lossy(),
            std::fs::metadata(&path).unwrap().len()
        );
    }

    // An override container carries chunks by id with no directory index --
    // the game resolves packages through the global store, not this TOC --
    // so the payload is checked by chunk rather than by path. The chunk id
    // itself is taken from the base archive when the override is built, so
    // it matches the tag it is overriding by construction.
    let reopened =
        blam_tags::iostore::IoStoreArchive::open(&out).expect("reopen exported container");
    // Two chunks even for a same-size edit: the tag and its paired
    // `.uasset`. The `.uasset` rides along so the mod stays editable — in-place
    // surgery can only repoint chunks the container already has, and without
    // it a later size-changing edit could never rewrite the declared length.
    assert_eq!(
        reopened.chunk_count(),
        2,
        "an override should carry the tag and its .uasset"
    );
    let ub_id = archive
        .chunk_id_for(&rel_path)
        .expect("the base names the tag's chunk");
    let ub_chunk = reopened
        .find_chunk(&ub_id)
        .expect("the override reuses the base chunk id");
    let served = reopened
        .read_chunk(ub_chunk)
        .expect("read the override chunk");
    assert_eq!(
        served, edited,
        "the exported container did not carry the edited bytes for {rel_path}"
    );
    assert_ne!(
        served, original,
        "the exported container carried the base bytes"
    );
    eprintln!("override verified for {rel_path} ({} bytes)", served.len());
    let _ = std::fs::remove_dir_all(&dir);
}

/// An exported mod opens EMPTY in the browser: an override container is
/// addressed by chunk id and ships with no directory index, so the file list
/// it advertises is nothing at all. Assert on what the tag browser actually
/// derives — the mounted entries — not on the raw chunks, which were already
/// fine while the UI showed an empty tree.
#[test]
fn a_mod_container_lists_its_tags_when_opened_on_its_own() {
    if !Path::new(*PAKS).exists() {
        eprintln!("skipping: {} not present", *PAKS);
        return;
    }
    let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = TagNameIndex::load_from_definitions(&defs);
    let loaded =
        load_iostore_container_set(PathBuf::from(*PAKS), &names, &defs).expect("mount");
    let TagSource::IoStoreContainerSet { ref containers, .. } = loaded.source else {
        panic!("expected a container set");
    };
    let (container, rel_path, original) = loaded
        .entries
        .iter()
        .find_map(|entry| match &entry.location {
            TagEntryLocation::Container {
                container,
                rel_path,
            } => {
                let archive = &containers.get(*container)?.archive;
                let bytes = archive.read(rel_path).ok()?;
                (bytes.len() > 64).then(|| (*container, rel_path.clone(), bytes))
            }
            _ => None,
        })
        .expect("a readable container tag");

    let mut edited = original.clone();
    let last = edited.len() - 1;
    edited[last] ^= 0xFF;

    // Write the mod into the game's own Paks folder, which is where mods
    // live and therefore where they get opened from.
    let out =
        PathBuf::from(*PAKS).join(format!("baboon-listing-test-{}_P.utoc", std::process::id()));
    let archive = containers[container].archive.clone();
    blam_tags::iostore::writer::write_mod_container_ex(
        &[(archive.as_ref(), rel_path.as_str(), edited.as_slice())],
        &[],
        &out,
    )
    .expect("write override container");

    // No install root known: the container has only the folder it is in.
    let opened = load_iostore_container(out.clone(), None, &names, &defs);
    for ext in ["utoc", "ucas", "pak"] {
        let _ = std::fs::remove_file(out.with_extension(ext));
    }
    let opened = opened.expect("mount the exported mod on its own");

    assert!(
        !opened.entries.is_empty(),
        "opening a mod container listed no tags at all"
    );
    let listed = opened.entries.iter().any(|entry| match &entry.location {
        TagEntryLocation::Container { rel_path: p, .. } => *p == rel_path,
        _ => false,
    });
    assert!(listed, "the overridden tag {rel_path} was not listed");
    eprintln!(
        "mod container listed {} tag(s); found {rel_path}",
        opened.entries.len()
    );
}

/// Mods are commonly installed in a folder under `Paks` — `~mods`, `~.mods`
/// — not loose beside the game's own paks. The game finds them there because
/// UE scans the pak folder recursively; a flat scan mounted the base game
/// and reported no sign of the mod at all.
///
/// Both ways in are covered: mounting the install, and opening the mod's own
/// `.utoc` against the install root the app already knows.
#[test]
fn a_mod_installed_below_the_paks_folder_is_mounted() {
    if !Path::new(*PAKS).exists() {
        eprintln!("skipping: {} not present", *PAKS);
        return;
    }
    let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = TagNameIndex::load_from_definitions(&defs);
    let loaded =
        load_iostore_container_set(PathBuf::from(*PAKS), &names, &defs).expect("mount");
    let TagSource::IoStoreContainerSet { ref containers, .. } = loaded.source else {
        panic!("expected a container set");
    };
    let (container, rel_path, original) = loaded
        .entries
        .iter()
        .find_map(|entry| match &entry.location {
            TagEntryLocation::Container {
                container,
                rel_path,
            } => {
                let archive = &containers.get(*container)?.archive;
                let bytes = archive.read(rel_path).ok()?;
                (bytes.len() > 64).then(|| (*container, rel_path.clone(), bytes))
            }
            _ => None,
        })
        .expect("a readable container tag");

    let mut edited = original.clone();
    let last = edited.len() - 1;
    edited[last] ^= 0xFF;

    let mods_dir = PathBuf::from(*PAKS).join("~mods");
    std::fs::create_dir_all(&mods_dir).expect("create the mod folder");
    let out = mods_dir.join(format!("baboon-submod-test-{}_P.utoc", std::process::id()));
    blam_tags::iostore::writer::write_mod_container_ex(
        &[(
            containers[container].archive.as_ref(),
            rel_path.as_str(),
            edited.as_slice(),
        )],
        &[],
        &out,
    )
    .expect("write override container");

    // Everything from here has to clean up after itself: the mod is sitting
    // in the user's install and must not outlive the test.
    let result = std::panic::catch_unwind(|| {
        // Mounting the install must serve the tag out of the mod, not the
        // base pak it overrides.
        let with_mod = load_iostore_container_set(PathBuf::from(*PAKS), &names, &defs)
            .expect("remount the install");
        let TagSource::IoStoreContainerSet {
            containers: ref mounted,
            ..
        } = with_mod.source
        else {
            panic!("expected a container set");
        };
        let served = with_mod
            .entries
            .iter()
            .find_map(|entry| match &entry.location {
                TagEntryLocation::Container {
                    container,
                    rel_path: p,
                } if *p == rel_path => {
                    let mounted = mounted.get(*container)?;
                    Some((mounted.utoc_path.clone(), mounted.archive.read(p).ok()?))
                }
                _ => None,
            })
            .expect("the overridden tag is in the mounted set");
        assert_eq!(
            served.0,
            out,
            "{rel_path} is still served by {}, not the mod under ~mods",
            served.0.display()
        );
        assert_eq!(served.1, edited, "the mod's bytes were not the ones served");

        // Opening the mod alone, against the install root the app knows.
        let opened =
            load_iostore_container(out.clone(), Some(PathBuf::from(*PAKS)), &names, &defs)
                .expect("mount the mod against the install");
        let listed = opened.entries.iter().any(|entry| match &entry.location {
            TagEntryLocation::Container { rel_path: p, .. } => *p == rel_path,
            _ => false,
        });
        assert!(
            listed,
            "the mod listed {} tag(s), none of them {rel_path}",
            opened.entries.len()
        );
    });

    for ext in ["utoc", "ucas", "pak"] {
        let _ = std::fs::remove_file(out.with_extension(ext));
    }
    let _ = std::fs::remove_dir(&mods_dir);
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
    eprintln!("a mod under ~mods served {rel_path}");
}

/// Save a tag that is being served by an already-exported mod: edit, export,
/// reload the folder, edit again, Save. The mod outranks the base pak, so
/// the save writes into the MOD — which ships no directory index, and
/// resolving the write through a freshly opened handle failed with
/// `path not found in container`.
#[test]
fn a_tag_served_by_an_exported_mod_can_be_saved_into_it_again() {
    if !Path::new(*PAKS).exists() {
        eprintln!("skipping: {} not present", *PAKS);
        return;
    }
    let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = TagNameIndex::load_from_definitions(&defs);
    let loaded =
        load_iostore_container_set(PathBuf::from(*PAKS), &names, &defs).expect("mount");
    let TagSource::IoStoreContainerSet { ref containers, .. } = loaded.source else {
        panic!("expected a container set");
    };
    let (container, rel_path, original) = loaded
        .entries
        .iter()
        .find_map(|entry| match &entry.location {
            TagEntryLocation::Container {
                container,
                rel_path,
            } => {
                let archive = &containers.get(*container)?.archive;
                let bytes = archive.read(rel_path).ok()?;
                (bytes.len() > 64).then(|| (*container, rel_path.clone(), bytes))
            }
            _ => None,
        })
        .expect("a readable container tag");

    let mut exported = original.clone();
    let last = exported.len() - 1;
    exported[last] ^= 0xFF;

    // Export the mod into the game's own Paks folder, where mods live.
    let out =
        PathBuf::from(*PAKS).join(format!("baboon-resave-test-{}_P.utoc", std::process::id()));
    blam_tags::iostore::writer::write_mod_container_ex(
        &[(
            containers[container].archive.as_ref(),
            rel_path.as_str(),
            exported.as_slice(),
        )],
        &[],
        &out,
    )
    .expect("write override container");

    // Everything from here has to clean up after itself: the mod is sitting
    // in the user's install and must not outlive the test.
    let result = std::panic::catch_unwind(|| {
        let opened = load_iostore_container(out.clone(), None, &names, &defs)
            .expect("mount the exported mod");
        let TagSource::IoStoreContainerSet {
            ref root,
            ref containers,
            ..
        } = opened.source
        else {
            panic!("expected a container set");
        };
        let (index, rel) = opened
            .entries
            .iter()
            .find_map(|entry| match &entry.location {
                TagEntryLocation::Container {
                    container,
                    rel_path: p,
                } if *p == rel_path => Some((*container, p.clone())),
                _ => None,
            })
            .expect("the mod serves the overridden tag");

        let mut resaved = exported.clone();
        resaved[0..4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let mounted = &containers[index];
        blam_tags::iostore::writer::overwrite_tag_in_place_with(
            &mounted.archive,
            &mounted.utoc_path,
            &rel,
            &resaved,
        )
        .expect("save into the mod container");

        // What the app does next: reopen the pak it just wrote. A plain
        // reopen loses the recovered file list, and the tag becomes
        // unreadable and unsaveable for the rest of the session.
        let reopened = reopen_container_archive(root, containers, index)
            .expect("reopen the written container");
        assert_eq!(
            reopened.read(&rel).expect("read the tag back"),
            resaved,
            "the mod did not serve the re-saved bytes"
        );
    });

    for ext in ["utoc", "ucas", "pak"] {
        let _ = std::fs::remove_file(out.with_extension(ext));
    }
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
    eprintln!("re-saved {rel_path} into its own exported mod");
}
