use super::*;

#[test]
fn mounted_path_prefixes_the_wwise_mount() {
    let m = CeSoundMedia {
        event_name: "Play_X".into(),
        language: "SFX".into(),
        media_id: 43030714,
        location: CeMediaLocation::Loose("Media/43/43030714.wem".into()),
        source_name: String::new(),
    };
    assert_eq!(
        m.mounted_path(),
        "Meteorite/Content/WwiseAudio/Media/43/43030714.wem"
    );
    // With no source name, fall back to the id rather than an empty label.
    assert_eq!(m.display_name(), "43030714");
}

#[test]
fn display_name_uses_the_authoring_wav_stem() {
    let m = CeSoundMedia {
        event_name: "Play_X".into(),
        language: "SFX".into(),
        media_id: 1,
        location: CeMediaLocation::Loose("Media/1/1.wem".into()),
        source_name: r"000_Olympus\006_Character\Foo_AR_ReloadMagHit_02.wav".into(),
    };
    assert_eq!(m.display_name(), "Foo_AR_ReloadMagHit_02");
}

#[test]
fn localized_lookup_falls_back_to_sfx() {
    let sfx = CeSoundMedia {
        event_name: "Play_X".into(),
        language: "SFX".into(),
        media_id: 1,
        location: CeMediaLocation::Loose("Media/1/1.wem".into()),
        source_name: String::new(),
    };
    let binding = CeSoundBinding {
        events: Vec::new(),
        media: vec![sfx],
    };
    // A non-localized event has no English(US) entry; asking for one must
    // still play rather than silently returning nothing.
    assert_eq!(binding.media_for_language("English(US)").len(), 1);
    assert_eq!(binding.languages(), vec!["SFX".to_string()]);
}

fn media(language: &str, id: u32) -> CeSoundMedia {
    CeSoundMedia {
        event_name: "Play_X".into(),
        language: language.into(),
        media_id: id,
        location: CeMediaLocation::Loose(format!("Media/{language}/{id}.wem")),
        source_name: String::new(),
    }
}

/// Localized voice carries no `SFX` entry at all. Showing `SFX` by default
/// rendered an empty player for every line of dialogue in the game.
#[test]
fn localized_only_binding_never_shows_an_empty_player() {
    let binding = CeSoundBinding {
        events: Vec::new(),
        media: vec![
            media("Chinese(PRC)", 1),
            media("English(US)", 2),
            media("German", 3),
        ],
    };

    // Nothing selected: prefer English rather than the alphabetical first.
    let shown = binding.language_to_show(None);
    assert_eq!(shown, "English(US)");
    assert_eq!(binding.media_for_language(&shown).len(), 1);

    // A selection this tag does carry is honoured.
    assert_eq!(binding.language_to_show(Some("German")), "German");

    // A selection it does not carry must still show something.
    let shown = binding.language_to_show(Some("Korean"));
    assert!(
        binding.languages().contains(&shown),
        "fell back to an absent language"
    );
    assert!(!binding.media_for_language(&shown).is_empty());
}

/// The mirror case: a non-localized tag while the shared selector holds a
/// language, which must not blank it either.
#[test]
fn sfx_only_binding_ignores_a_localized_selection() {
    let binding = CeSoundBinding {
        events: Vec::new(),
        media: vec![media("SFX", 1)],
    };
    assert_eq!(binding.language_to_show(Some("German")), "SFX");
    assert_eq!(binding.media_for_language("German").len(), 1);
}

/// Every root the audio graph reaches through has to stay walkable. Events
/// themselves are found by class, but the walk still needs to *get* to them.
#[test]
fn every_audio_root_is_walkable() {
    assert!(is_audio_package(
        "/game/audio/characters/elite/shield_pop_elite"
    ));
    assert!(is_audio_package(
        "/game/audio/audio_fi/character/elites/shield/play_x"
    ));
    assert!(is_audio_package("/game/wwise/events/play_foo"));
    assert!(is_audio_package(
        "/game/wwiseaudio/events/systemic/vo/play_bar"
    ));
    assert!(!is_audio_package("/game/tags/sound/x-sound"));
}

/// End-to-end against a real Campaign Evolved install: mount the
/// containers, walk a known sound tag's imports, and decode the media it
/// resolves to. Ignored by default — it needs the shipped game.
///
/// Run with:
///   CE_PAKS=/path/to/Meteorite/Content/Paks cargo test ce_audio -- --ignored --nocapture
#[test]
#[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
fn resolves_and_decodes_real_sound_tags() {
    use blam_tags::iostore::IoStoreArchive;
    use std::path::PathBuf;
    use std::sync::Arc;

    let root = PathBuf::from(
        std::env::var("CE_PAKS").expect("set CE_PAKS to the game's Content/Paks"),
    );

    let mut utocs: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("read paks dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("utoc"))
        })
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("global.utoc"))
        })
        .collect();
    utocs.sort();

    let mut containers = Vec::new();
    let mut packages = ContainerPackageIndex::default();
    for utoc in utocs {
        let Ok(archive) = IoStoreArchive::open(&utoc) else {
            continue;
        };
        let idx = containers.len();
        for e in archive.entries() {
            if let Some(pkg) = super::super::container_package_name(&e.path) {
                packages.insert(pkg, idx, e.path.clone());
            }
        }
        containers.push(MountedContainer {
            utoc_path: utoc.clone(),
            chunk_label: utoc.file_stem().unwrap().to_string_lossy().into_owned(),
            // Nothing on this path layers shipped against modded — it mounts
            // everything to resolve one lookup.
            is_mod: false,
            archive: Arc::new(archive),
        });
    }
    assert!(!packages.is_empty(), "no cooked packages indexed");
    println!(
        "indexed {} packages across {} containers",
        packages.len(),
        containers.len()
    );

    // The app reaches a tag by its browser entry, which points at the
    // `.ubulk` payload — so the `.ubulk` → package mapping must land on a
    // package that actually exists. Check it over every mounted sound tag.
    let mut sound_tags = 0usize;
    let mut resolved = 0usize;
    for c in &containers {
        for e in c.archive.ublock_entries() {
            if !e.path.to_ascii_lowercase().ends_with("-sound.ubulk") {
                continue;
            }
            sound_tags += 1;
            if tag_package_for_rel_path(&e.path)
                .is_some_and(|pkg| packages.lookup(&pkg).is_some())
            {
                resolved += 1;
            }
        }
    }
    println!("sound tags: {sound_tags}, package mapping resolved: {resolved}");
    assert!(sound_tags > 0, "no sound tags mounted");
    assert_eq!(
        sound_tags, resolved,
        "some sound tags had no cooked package"
    );

    let usmap = Usmap::meteorite().expect("bundled usmap");
    let mut store = CeMediaStore::default();

    // One tag per shape the resolution has to handle. Each was silent at
    // some point because the walk assumed the previous one's shape.
    let cases = [
        "/Game/Tags/sound/005_sandbox/006_character/006_character_movement/\
             006_chm_ge_weaanim/006_chm_ge_weaanim_ar_reloadmaghit-sound",
        "/Game/Tags/sound/dialog/combat/bisenti/default/01_contact/ambush-sound",
        // Scripted mission dialogue — the shape that rendered an empty
        // player because it carries no SFX entry.
        "/Game/Tags/sound/scripted/vo_scr_m02halo/m02_00040_cortana-sound",
        // Its event sits under `/Game/Audio/Audio_FI/…` rather than either
        // `Events` root, so a path-prefix test never finds it.
        "/Game/Tags/sound/characters/elite/shield_pop_elite-sound",
        // Media cooked inside a SoundBank: the event names banks and no
        // media, so the permutations come from the bank's own event graph.
        "/Game/Tags/sound/characters/bodyfalls/brute_bodyfalls/brute_bodyfall_dirt-sound",
    ];

    for tag in cases {
        let tag = tag.replace(char::is_whitespace, "");
        let binding = resolve_sound_binding(
            &containers,
            &packages,
            &usmap,
            &tag,
            Some((root.as_path(), &mut store)),
        );
        assert!(!binding.is_empty(), "no media resolved for {tag}");
        println!(
            "{tag}\n  events={} media={} languages={:?}",
            binding.events.len(),
            binding.media.len(),
            binding.languages()
        );

        // Every event's id must equal the Wwise hash of its own name — a
        // misread of the cooked struct cannot satisfy that.
        for ev in &binding.events {
            assert_eq!(
                blam_tags::audio::wwise::hash_name(&ev.event_name),
                ev.event_id,
                "event id/name mismatch for {}",
                ev.event_name
            );
        }

        // What the player would actually show with nothing selected must
        // never be empty for a tag that resolved media.
        let shown = binding.language_to_show(None);
        assert!(
            !binding.media_for_language(&shown).is_empty(),
            "{tag} shows no rows for default language {shown}"
        );
        println!("  default language shown: {shown}");

        // And the media has to actually decode to non-silent audio.
        for m in binding.media_for_language(&shown) {
            let bytes = store.fetch(&root, m).expect("fetch media");
            let pcm = decode_media_bytes(m, &bytes).expect("decode media");
            assert!(
                !pcm.samples.is_empty(),
                "{} decoded to nothing",
                m.location_label()
            );
            let peak = pcm
                .samples
                .iter()
                .map(|s| s.unsigned_abs())
                .max()
                .unwrap_or(0);
            assert!(peak > 0, "{} decoded to silence", m.location_label());
            println!(
                "  {} [{}] {} ch {} Hz peak {peak}  ({})",
                m.display_name(),
                m.language,
                pcm.channels,
                pcm.sample_rate,
                m.location_label()
            );
        }
    }
}

/// Coverage over every mounted sound tag. Most of the game's audio was
/// invisible to three separate assumptions in turn (one event root, one
/// package layout, one bank version), each of which failed silently — so
/// the guard is a floor on how much of the game resolves, not a spot check.
///
/// Run with:
///   CE_PAKS=/path/to/Meteorite/Content/Paks cargo test ce_audio -- --ignored --nocapture
#[test]
#[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
fn most_sound_tags_resolve_to_media() {
    use blam_tags::iostore::IoStoreArchive;
    use std::path::PathBuf;
    use std::sync::Arc;

    let root = PathBuf::from(
        std::env::var("CE_PAKS").expect("set CE_PAKS to the game's Content/Paks"),
    );
    let mut utocs: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("read paks dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("utoc"))
        })
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("global.utoc"))
        })
        .collect();
    utocs.sort();

    let mut containers = Vec::new();
    let mut packages = ContainerPackageIndex::default();
    let mut tag_packages: Vec<String> = Vec::new();
    for utoc in utocs {
        let Ok(archive) = IoStoreArchive::open(&utoc) else {
            continue;
        };
        let idx = containers.len();
        for e in archive.entries() {
            if let Some(pkg) = super::super::container_package_name(&e.path) {
                packages.insert(pkg, idx, e.path.clone());
            }
        }
        for e in archive.ublock_entries() {
            if e.path.to_ascii_lowercase().ends_with("-sound.ubulk")
                && let Some(pkg) = tag_package_for_rel_path(&e.path)
            {
                tag_packages.push(pkg);
            }
        }
        containers.push(MountedContainer {
            utoc_path: utoc.clone(),
            chunk_label: utoc.file_stem().unwrap().to_string_lossy().into_owned(),
            // Nothing on this path layers shipped against modded — it mounts
            // everything to resolve one lookup.
            is_mod: false,
            archive: Arc::new(archive),
        });
    }
    tag_packages.sort();
    tag_packages.dedup();

    let usmap = Usmap::meteorite().expect("bundled usmap");
    let mut store = CeMediaStore::default();
    let mut bound = 0usize;
    let mut from_banks = 0usize;
    for pkg in &tag_packages {
        let binding = resolve_sound_binding(
            &containers,
            &packages,
            &usmap,
            pkg,
            Some((root.as_path(), &mut store)),
        );
        if binding.is_empty() {
            continue;
        }
        bound += 1;
        if binding
            .media
            .iter()
            .any(|m| matches!(m.location, CeMediaLocation::Bank(_)))
        {
            from_banks += 1;
        }
    }
    let total = tag_packages.len();
    println!("{bound}/{total} sound tags resolved media ({from_banks} out of SoundBanks)");
    // Measured 5332/5895 on the 2026.06.26 build; the rest are stubs with
    // no audio asset behind them at all.
    assert!(
        bound * 100 / total >= 88,
        "only {bound}/{total} sound tags resolved"
    );
    assert!(
        from_banks > 400,
        "bank-embedded media stopped resolving ({from_banks})"
    );
}
