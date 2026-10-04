//! Every saved format Baboon reads, fed through the real readers from the
//! synthetic samples in `testdata/compat` (see its README; regenerate with
//! `gen_samples.py`). Old files must keep loading, files a newer build wrote
//! must not be destroyed by this one, and the cases a reader refuses are
//! pinned beside the ones it accepts, so a reader that accepted everything
//! would fail here too.

use super::*;
use std::path::PathBuf;

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/compat/samples")
}

fn json(rel: &str) -> Value {
    let text = std::fs::read_to_string(samples().join(rel))
        .unwrap_or_else(|error| panic!("{rel}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{rel}: {error}"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "baboon-compat-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn compat_last_session_versions() {
    let v1 = parse_last_session(&json("last_session/v1.json")).expect("v1");
    assert_eq!(v1.kits.len(), 1);
    assert!(v1.kits[0].tags[0].key.starts_with("file:C:\\"));
    assert!(v1.kits[0].tags[0].path.is_some());

    let v2 = parse_last_session(&json("last_session/v2_single_source.json")).expect("v2 single");
    assert_eq!(
        v2.kits[0].source_kind,
        LastSessionSourceKind::IoStoreContainerSet
    );
    assert!(
        v2.kits[0].has_project,
        "has_project defaults to project_path.is_some()"
    );

    // The v2 shape 8f30d04 wrote ({version: 2, kits: [...]}) is not accepted.
    assert!(parse_last_session(&json("last_session/v2_kits_8f30d04.json")).is_none());

    let v3 = parse_last_session(&json("last_session/v3.json")).expect("v3");
    assert_eq!(v3.kits.len(), 3);
    assert_eq!(v3.kits[1].tags[1].key, "cache:rm:shaders\\default");
    assert_eq!(v3.kits[1].tags[1].group_tag, u32::from_be_bytes(*b"rm  "));
    assert!(v3.kits[2].has_project);

    let v4 = parse_last_session(&json("last_session/v4.json")).expect("v4");
    assert!(v4.kits[0].was_active);
    assert_eq!(v4.kits[0].chimp_packages.len(), 2);
    assert_eq!(
        v4.kits[0].active_chimp_package.as_deref(),
        Some("/Game/Maps/a30/a30_Persistent")
    );

    let v5 = parse_last_session(&json("last_session/v5.json")).expect("v5");
    assert!(v5.kits[0].bitmap_library_open && !v5.kits[0].model_library_open);

    let v6 = parse_last_session(&json("last_session/v6.json")).expect("v6");
    assert_eq!(v6.kits.len(), 11);
    let keys: Vec<&str> = v6
        .kits
        .iter()
        .flat_map(|kit| kit.tags.iter().map(|tag| tag.key.as_str()))
        .collect();
    let tag_keys = json("tag_keys.json");
    for (kind, expected) in tag_keys.as_object().unwrap() {
        assert!(
            keys.contains(&expected.as_str().unwrap()),
            "{kind} kept verbatim"
        );
    }
    assert_eq!(
        v6.kits[10].game.as_deref(),
        Some("halo5_mcc"),
        "an unknown game id passes through unvalidated"
    );
    assert_eq!(
        v6.kits[0].folders[1].rel_path,
        PathBuf::from("levels/solo/010_jungle")
    );

    // Written again and read back, every key survives and the version is current.
    let again = parse_last_session(&session_value(&v6)).expect("round trip");
    let keys_again: Vec<String> = again
        .kits
        .iter()
        .flat_map(|kit| kit.tags.iter().map(|tag| tag.key.clone()))
        .collect();
    assert_eq!(keys, keys_again);
    assert_eq!(session_value(&again)["version"], 6);

    assert!(parse_last_session(&json("last_session/v99_unknown_version.json")).is_none());
}

#[test]
fn compat_prefs() {
    let prefs = prefs_from_value(&json("prefs/prefs.current.json"));
    assert_eq!(prefs.custom_editing_kit_profiles.len(), 10);
    let games: Vec<&str> = prefs
        .custom_editing_kit_profiles
        .iter()
        .map(|profile| profile.game.as_str())
        .collect();
    for game in [
        "haloce_mcc",
        "halo2_mcc",
        "halo2amp_mcc",
        "halo3_mcc",
        "halo3odst_mcc",
        "haloreach_mcc",
        "halo4_mcc",
        "haloce_evolved",
    ] {
        assert!(games.contains(&game), "{game}");
    }
    let moda = prefs
        .custom_editing_kit_profiles
        .iter()
        .find(|profile| profile.name == "H2 (moda tags)")
        .unwrap();
    assert!(moda.read_only && moda.git_tracked);
    // A backslash icon path is one component on Unix, so it is not under the
    // icon folder there and is dropped; Windows keeps it.
    #[cfg(not(windows))]
    assert!(moda.icon.is_none());
    assert!(moda.tags_folder.is_some());
    let reach = prefs
        .custom_editing_kit_profiles
        .iter()
        .find(|profile| profile.name == "Reach ignored folder")
        .unwrap();
    assert!(
        reach.tags_folder.is_none(),
        "tags_folder is ignored for a game whose folders are not choosable"
    );
    assert_eq!(prefs.ek_folder_aliases.len(), 2);
    assert_eq!(prefs.editing_kit_favorites.len(), 2);

    let encoded = prefs_to_value(&prefs, &HashSet::from(["halo3_mcc".to_owned()]), true);
    let decoded = prefs_from_value(&encoded);
    let ids = |prefs: &GuiPrefs| {
        prefs
            .custom_editing_kit_profiles
            .iter()
            .map(|profile| (profile.id.clone(), profile.game.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&prefs), ids(&decoded));

    // Kits for games this build does not support are not offered, but they
    // are written back as they were read (63b257d). Before that, the next
    // save of any preference deleted them.
    let stored = json("prefs/prefs.unknown_game.json");
    let unknown = prefs_from_value(&stored);
    assert_eq!(
        ids(&unknown),
        [(
            "bab00000-0000-4000-8000-000000000002".to_owned(),
            "halo3_mcc".to_owned()
        )]
    );
    assert!(unknown.ek_folder_aliases.is_empty());
    let resaved = prefs_to_value(&unknown, &HashSet::new(), true);
    let profiles = resaved["editing_kit_profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 3, "{profiles:#?}");
    for kept in [
        &stored["editing_kit_profiles"][0],
        &stored["editing_kit_profiles"][2],
    ] {
        assert!(profiles.contains(kept), "{kept} kept field for field");
    }
    assert_eq!(
        resaved["ek_folder_aliases"],
        stored["ek_folder_aliases"],
        "the alias for an unknown game is kept"
    );
    assert!(prefs_from_value(&resaved) == unknown, "stable once written");

    // Before unified profiles: `editing_kit_paths`, one standard kit per game.
    let legacy = prefs_from_value(&json("prefs/prefs.legacy_editing_kit_paths.json"));
    assert_eq!(legacy.custom_editing_kit_profiles.len(), 8);
    assert!(
        legacy
            .custom_editing_kit_profiles
            .iter()
            .all(|profile| profile.id.starts_with("bab00000-0000-4000-8000-00000000000"))
    );
    assert_eq!(legacy.session_restore, SessionRestore::Always);
    assert_eq!(
        legacy.custom_color_swatches.len(),
        CUSTOM_COLOR_SWATCH_COUNT
    );

    // The older `custom_editing_kit_profiles` key, a mixed-case game id.
    let old = prefs_from_value(&json("prefs/prefs.legacy_custom_profiles.json"));
    assert!(
        old.custom_editing_kit_profiles
            .iter()
            .any(|profile| profile.game == "halo3_mcc" && profile.name == "Old custom")
    );
    assert!(
        old.custom_editing_kit_profiles
            .iter()
            .any(|profile| profile.game == "haloreach_mcc")
    );

    // A file cut short is not a first run.
    let text = std::fs::read_to_string(samples().join("prefs/prefs.malformed.json")).unwrap();
    assert!(first_run_complete_from_text(Some(&text)));
    assert!(!first_run_complete_from_text(None));
}

#[test]
fn compat_projects() {
    use crate::app::project::{
        ProjectScope, is_campaign_recovery_file, load_campaign_project, save_campaign_project,
    };
    let project = samples().join("project");
    let recovery = project.join("campaign_evolved_recovery-46ec1ffb674b.baboon");
    assert!(is_campaign_recovery_file(&recovery));
    assert!(!is_campaign_recovery_file(
        &project.join("user_project.history_table_only.baboon")
    ));
    let snap = load_campaign_project(&recovery).expect("recovery");
    assert_eq!(snap.game, "haloce_evolved");
    assert_eq!(snap.tabs.len(), 4);
    assert_eq!(snap.overlays.len(), 2);
    assert_eq!(snap.folders.len(), 2);
    let marine = "62697064:objects/characters/marine/marine";
    assert_eq!(snap.selected_identity.as_deref(), Some(marine));
    let history = &snap.history[marine];
    assert_eq!(
        (history.undo.len(), history.redo.len()),
        (2, 1),
        "a stack name this build does not know is skipped"
    );
    // The recovery file is named by sha256(source_path); the loader compares
    // source_path exactly, so renormalizing the root orphans the file.
    let expected = {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(snap.source_path.to_string_lossy().as_bytes());
        digest[..6]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    assert_eq!(expected, "46ec1ffb674b");

    let scratch = scratch("project");
    let out = scratch.join("roundtrip.baboon");
    save_campaign_project(&out, &snap, None, ProjectScope::Session).unwrap();
    let back = load_campaign_project(&out).unwrap();
    let identities =
        |snap: &CampaignProjectSnapshot| snap.tabs.iter().map(|tab| tab.identity.clone()).collect::<Vec<_>>();
    assert_eq!(identities(&back), identities(&snap));
    assert_eq!(back.folders, snap.folders);
    let _ = std::fs::remove_dir_all(&scratch);

    let legacy = load_campaign_project(&project.join("user_project.history_table_only.baboon"))
        .expect("history table only");
    assert_eq!(legacy.history[marine].undo.len(), 1);
    let original = load_campaign_project(&project.join("user_project.original_v1_schema.baboon"))
        .expect("original schema");
    assert!(original.history.is_empty() && original.folders.is_empty());
    for rejected in [
        "rejected.version2.baboon",
        "rejected.game_halo3_mcc.baboon",
        "rejected.unknown_kind.baboon",
    ] {
        assert!(
            load_campaign_project(&project.join(rejected)).is_err(),
            "{rejected} must be refused"
        );
    }
}

/// A Campaign Evolved tag's project identity is `{group:08x}:{logical path}`,
/// the display path lowered without its extension. Dotted names keep their
/// dots (bb6315b); an identity written before that, cut at the last dot, still
/// finds its tag when only one tag had it.
#[test]
fn compat_campaign_identities() {
    let container = |logical: &str, group: &[u8; 4], extension: &str| TagEntry {
        key: crate::core::source::container_entry_key(
            "pakchunk0-WinGDK",
            &format!("Meteorite/Content/Tags/{logical}-{extension}.ubulk"),
        ),
        display_path: format!("{logical}.{extension}"),
        group_tag: u32::from_be_bytes(*group),
        group_name: Some(extension.to_owned()),
        location: TagEntryLocation::Container {
            container: 0,
            rel_path: format!("Meteorite/Content/Tags/{logical}-{extension}.ubulk"),
        },
    };
    let identity = |entry: &TagEntry| campaign_entry_project_parts(entry).unwrap().0;
    assert_eq!(
        identity(&container("objects/characters/marine/marine", b"bipd", "biped")),
        "62697064:objects/characters/marine/marine"
    );
    assert_eq!(
        identity(&container("Levels/V1.2/Bitmaps/Rock", b"bitm", "bitmap")),
        "6269746d:levels/v1.2/bitmaps/rock"
    );
    assert_eq!(
        identity(&container("sound/machines/piston_close2.l", b"snd!", "sound")),
        "736e6421:sound/machines/piston_close2.l"
    );
    let package = "/Game/Tags/objects/foo/bar-camera_track";
    assert_eq!(
        crate::core::tag_key::new_tag_entry_key(package),
        json("tag_keys.json")["newtag_ce"].as_str().unwrap()
    );
    let authored = TagEntry {
        key: crate::core::tag_key::new_tag_entry_key(package),
        display_path: "objects/foo/bar.camera_track".to_owned(),
        group_tag: u32::from_be_bytes(*b"trak"),
        group_name: Some("camera_track".to_owned()),
        location: TagEntryLocation::NewContainer {
            template: crate::core::source::NewContainerTemplate::Derived {
                group: "camera_track".to_owned(),
            },
            package: package.to_owned(),
            group_tag: u32::from_be_bytes(*b"trak"),
        },
    };
    let (new_identity, _, kind, new_package) = campaign_entry_project_parts(&authored).unwrap();
    assert_eq!(new_identity, "7472616b:objects/foo/bar");
    assert_eq!(kind, CampaignProjectTagKind::New);
    assert_eq!(new_package.as_deref(), Some(package));

    // The project file holding both spellings, against a mounted source with
    // one dotted tag in it.
    let snap = crate::app::project::load_campaign_project(
        &samples().join("project/user_project.dotted_identities.baboon"),
    )
    .expect("dotted identities");
    let tabs: Vec<&str> = snap.tabs.iter().map(|tab| tab.identity.as_str()).collect();
    assert_eq!(
        tabs,
        [
            "6269746d:levels/v1.2/bitmaps/rock",
            "6269746d:levels/v1",
            "736e6421:sound/machines/piston_close2.l",
        ]
    );
    let mut app = Baboon::for_test();
    let mounted = |entries: Vec<TagEntry>| LoadedSourceData {
        label: "ce".to_owned(),
        source: TagSource::LooseFolder {
            root: PathBuf::from("/ce"),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names: TagNameIndex::default(),
        game: None,
        entries,
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    };
    let rock = container("levels/v1.2/bitmaps/rock", b"bitm", "bitmap");
    app.install_loaded_source(mounted(vec![rock.clone()]));
    for tab in &tabs[..2] {
        assert_eq!(
            app.campaign_entry_for_identity(0, tab).map(|entry| entry.key),
            Some(rock.key.clone()),
            "{tab}"
        );
    }
    assert!(app.campaign_entry_for_identity(0, tabs[2]).is_none());
    // Two tags that had the same old identity: neither is guessed.
    app.install_loaded_source(mounted(vec![
        rock.clone(),
        container("levels/v1.3/bitmaps/rock", b"bitm", "bitmap"),
    ]));
    assert!(app.campaign_entry_for_identity(0, tabs[1]).is_none());
    assert!(app.campaign_entry_for_identity(0, tabs[0]).is_some());
}

#[test]
fn compat_duplicate_ledger() {
    use crate::app::controller::{CreatedTagLedger, CreatedTagOrigin};
    let ledger_dir = samples().join("ledger");
    let utoc = Path::new(
        r"D:\XboxGames\Halo Campaign Evolved\Content\Meteorite\Content\Paks\~mods\mymod_P.utoc",
    );
    let copy = "Meteorite/Content/Tags/objects/characters/marine/marine_copy-biped.ubulk";

    let current = CreatedTagLedger::load_from(&ledger_dir.join("campaign_duplicates.json"));
    assert_eq!(
        current.find(utoc, &copy.to_ascii_uppercase()).map(|record| &record.origin),
        Some(&CreatedTagOrigin::Authored),
        "payload paths compare without case"
    );
    let v0 = CreatedTagLedger::load_from(
        &ledger_dir.join("campaign_duplicates.v0_no_version_no_origin.json"),
    );
    assert_eq!(
        v0.find(utoc, copy).map(|record| &record.origin),
        Some(&CreatedTagOrigin::Authored),
        "a row from before `origin` existed was a duplicate"
    );

    // A newer build's ledger: the unknown origin is kept and is not deletable
    // as Authored; the row of an unknown shape is kept raw. Saving writes all
    // three back (8c6da07). Before that, the next save erased the file.
    let future_path = ledger_dir.join("campaign_duplicates.future_origin.json");
    let future = CreatedTagLedger::load_from(&future_path);
    assert_eq!(
        future.find(utoc, copy).map(|record| &record.origin),
        Some(&CreatedTagOrigin::Unrecognized("ImportedFromMod".to_owned()))
    );
    assert_eq!(
        future.find(utoc, "x.ubulk").map(|record| &record.origin),
        Some(&CreatedTagOrigin::Authored)
    );
    let scratch = scratch("ledger");
    let saved = scratch.join("campaign_duplicates.json");
    future.save_to(&saved).expect("save");
    let written: Value = serde_json::from_slice(&std::fs::read(&saved).unwrap()).unwrap();
    let stored = json("ledger/campaign_duplicates.future_origin.json");
    let rows = written["tags"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for row in stored["tags"].as_array().unwrap() {
        assert!(rows.contains(row), "{row} written back unchanged");
    }

    // Not a ledger at all: it loads empty, and saving leaves it as it is.
    let truncated = std::fs::read(ledger_dir.join("campaign_duplicates.truncated.json")).unwrap();
    let damaged = scratch.join("damaged.json");
    std::fs::write(&damaged, &truncated).unwrap();
    let empty = CreatedTagLedger::load_from(&damaged);
    assert!(empty.is_empty());
    assert!(empty.save_to(&damaged).is_err());
    assert_eq!(std::fs::read(&damaged).unwrap(), truncated);
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn compat_keyword_sidecars() {
    let mut store = crate::core::keywords::KeywordStore::default();
    store.load_at(Some(samples().join("keywords/haloce_evolved_keywords.json")));
    let keys = json("tag_keys.json");
    let key = |kind: &str| keys[kind].as_str().unwrap().to_owned();
    assert_eq!(store.keywords(&key("newtag_ce")), ["cinematic"]);
    assert_eq!(store.keywords(&key("ublock_mod")), ["copy"]);
    store.load_at(Some(samples().join("keywords/halo3_mcc_keywords.json")));
    assert_eq!(store.keywords(&key("legacy_bare")), ["wip"], "bare keys are read");
    assert_eq!(store.keywords(&key("file_windows")), ["favorite", "rifle"]);

    // A sidecar cut short mid-write reads as empty and says so; the next save
    // moves it aside byte for byte before starting a new one (41bd542).
    let corrupt = std::fs::read(samples().join("keywords/halo4_mcc_keywords.corrupt.json")).unwrap();
    let scratch = scratch("keywords");
    let sidecar = scratch.join("halo4_mcc_keywords.json");
    std::fs::write(&sidecar, &corrupt).unwrap();
    store.load_at(Some(sidecar.clone()));
    assert!(store.all_keywords().is_empty());
    assert!(store.take_notice().is_some());
    store.add(&key("file_posix"), "Storm");
    store.save_if_dirty();
    assert_eq!(
        std::fs::read(scratch.join("halo4_mcc_keywords.json.unreadable")).unwrap(),
        corrupt
    );
    let written: Value = serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
    assert_eq!(written, json!({ key("file_posix"): ["storm"] }));
    let _ = std::fs::remove_dir_all(&scratch);
}
