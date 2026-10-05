use super::*;

/// A version-1 file predates multiple kits. It must still restore, as one
/// kit — silently dropping it would lose the user's open tags on upgrade.
#[test]
fn version_1_sessions_upgrade_to_a_single_kit() {
    let value = serde_json::json!({
        "version": 1,
        "source": { "kind": "loose_folder", "path": "/tags", "game": "halo3_mcc" },
        "tags": [{ "key": "file:/tags/a.weapon", "label": "a", "group_tag": 1, "path": null }],
    });
    let session = parse_last_session(&value).expect("v1 session parses");
    assert_eq!(session.kits.len(), 1);
    assert_eq!(session.kits[0].game.as_deref(), Some("halo3_mcc"));
    assert_eq!(session.kits[0].tags.len(), 1);
}

#[test]
fn version_3_sessions_restore_every_kit() {
    let value = serde_json::json!({
        "version": 3,
        "kits": [
            {
                "source": { "kind": "loose_folder", "path": "/h3", "game": "halo3_mcc" },
                "tags": [{ "key": "file:/h3/a.weapon", "label": "a", "group_tag": 1 }],
            },
            {
                "source": { "kind": "loose_folder", "path": "/reach", "game": "haloreach_mcc" },
                "tags": [{ "key": "file:/reach/b.weapon", "label": "b", "group_tag": 1 }],
            },
        ],
    });
    let session = parse_last_session(&value).expect("v3 session parses");
    assert_eq!(session.kits.len(), 2);
    assert_eq!(session.kits[1].game.as_deref(), Some("haloreach_mcc"));
    assert_eq!(session.kits[1].tags[0].key, "file:/reach/b.weapon");
}

#[test]
fn custom_profile_identity_survives_session_round_trip() {
    let mut custom = kit("/custom-reach", Some(BrowserMode::Folders));
    custom.game = Some("haloreach_mcc".to_owned());
    custom.profile_id = Some("11111111-1111-4111-8111-111111111111".to_owned());
    let restored = parse_last_session(&session_value(&LastSessionState { kits: vec![custom] }))
        .expect("custom profile session parses");
    assert_eq!(
        restored.kits[0].profile_id.as_deref(),
        Some("11111111-1111-4111-8111-111111111111")
    );
    assert_eq!(restored.kits[0].game.as_deref(), Some("haloreach_mcc"));
}

/// A source-only kit remains part of the session even without open tags,
/// while a session left with no kits at all is still no session.
#[test]
fn source_only_kits_are_retained_for_restore() {
    let value = serde_json::json!({
        "version": 3,
        "kits": [
            { "source": { "kind": "loose_folder", "path": "/h3" }, "tags": [] },
            {
                "source": { "kind": "loose_folder", "path": "/reach" },
                "tags": [{ "key": "file:/reach/b.weapon", "label": "b", "group_tag": 1 }],
            },
        ],
    });
    let session = parse_last_session(&value).expect("session parses");
    assert_eq!(session.kits.len(), 2);
    assert_eq!(session.kits[0].source_path, PathBuf::from("/h3"));
    assert_eq!(session.kits[0].tags.len(), 0);
    assert_eq!(session.kits[1].source_path, PathBuf::from("/reach"));

    let empty = serde_json::json!({ "version": 3, "kits": [] });
    assert!(parse_last_session(&empty).is_none());
}

#[test]
fn two_source_only_workspaces_keep_their_order_and_identity() {
    let mut campaign = kit("/campaign-evolved", None);
    campaign.tags.clear();
    let mut reach = kit("/reach", None);
    reach.tags.clear();

    let restored = parse_last_session(&session_value(&LastSessionState {
        kits: vec![campaign, reach],
    }))
    .expect("two source-only workspaces parse");

    assert_eq!(
        restored
            .kits
            .iter()
            .map(|kit| kit.source_path.clone())
            .collect::<Vec<_>>(),
        [PathBuf::from("/campaign-evolved"), PathBuf::from("/reach")]
    );
}

#[test]
fn unknown_versions_are_ignored() {
    let value = serde_json::json!({ "version": 99, "kits": [] });
    assert!(parse_last_session(&value).is_none());
}

fn kit(path: &str, mode: Option<BrowserMode>) -> LastSessionKit {
    LastSessionKit {
        source_kind: LastSessionSourceKind::LooseFolder,
        source_path: PathBuf::from(path),
        game: None,
        profile_id: None,
        project_path: None,
        has_project: false,
        browser_mode: mode,
        browser_sort: Some(BrowserSort::Name),
        tags: vec![LastSessionTag {
            key: format!("file:{path}/a.weapon"),
            label: "a".to_owned(),
            group_tag: 1,
            path: None,
        }],
        folders: Vec::new(),
        chimp_packages: Vec::new(),
        active_chimp_package: None,
        bitmap_library_open: false,
        model_library_open: false,
        was_active: false,
    }
}

#[test]
fn unchecked_workspaces_are_excluded_without_losing_their_pane_choices() {
    let root = std::env::temp_dir().display().to_string();
    let mut prompt = LastOpenedWindowsPrompt::from_session(
        LastSessionState {
            kits: vec![kit(&root, None), kit(&root, None)],
        },
        &[],
    )
    .unwrap();
    prompt.kits[0].bitmap_library_open = true;
    prompt.kits[0].checked = false;
    assert_eq!(prompt.checked_kits().len(), 1);
    assert!(prompt.kits[0].entries[0].checked);
    prompt.kits[1].checked = false;
    assert!(prompt.checked_kits().is_empty());
    assert!(!prompt.has_reopenable_kits());
    prompt.kits[0].checked = true;
    assert_eq!(prompt.checked_kits()[0].tags.len(), 1);
    assert!(prompt.checked_kits()[0].bitmap_library_open);
    // A selected workspace can still reopen with all its panes unchecked.
    prompt.kits[0].entries[0].checked = false;
    assert_eq!(prompt.checked_kits().len(), 1);
    assert!(prompt.checked_kits()[0].tags.is_empty());
    prompt.kits[0].source_available = false;
    assert!(prompt.checked_kits().is_empty());
}

#[test]
fn folder_windows_round_trip_and_can_be_unchecked_for_restore() {
    let source_path = std::env::temp_dir();
    let mut saved = kit(
        &source_path.display().to_string(),
        Some(BrowserMode::Folders),
    );
    saved.tags.clear();
    saved.folders.push(LastSessionFolder {
        rel_path: PathBuf::from(r"objects\characters\brute"),
        label: "brute".to_owned(),
    });

    let value = session_value(&LastSessionState { kits: vec![saved] });
    assert_eq!(value["version"], 6);
    assert_eq!(
        value["kits"][0]["folders"][0]["path"],
        "objects/characters/brute"
    );

    let restored = parse_last_session(&value).expect("folder session parses");
    assert_eq!(restored.kits[0].folders.len(), 1);
    assert_eq!(restored.kits[0].folders[0].label, "brute");

    let mut prompt =
        LastOpenedWindowsPrompt::from_session(restored, &[]).expect("restore prompt exists");
    assert!(prompt.kits[0].folder_entries[0].checked);
    assert_eq!(prompt.checked_kits()[0].folders.len(), 1);
    prompt.kits[0].folder_entries[0].checked = false;
    assert!(prompt.checked_kits()[0].folders.is_empty());
}

/// A kit's chosen folders survive a save and load. A kit on its root's own
/// folders saves no folder keys at all, exactly as before they existed,
/// and an engine whose tools can't use them reads them as unset.
#[test]
fn chosen_kit_folders_round_trip_and_stay_out_of_other_kits() {
    let profile = |id: &str, game: &str, tags: Option<&str>, data: Option<&str>| {
        CustomEditingKitProfile {
            read_only: false,
            git_tracked: false,
            id: id.to_owned(),
            name: id.to_owned(),
            game: game.to_owned(),
            root: PathBuf::from("/kits/H2EK"),
            icon: None,
            tags_folder: tags.map(PathBuf::from),
            data_folder: data.map(PathBuf::from),
        }
    };
    let moda = profile(
        "00000000-0000-4000-8000-00000000000a",
        "halo2_mcc",
        Some("tags_moda"),
        Some("/elsewhere/data_moda"),
    );
    let stock = profile(
        "00000000-0000-4000-8000-00000000000b",
        "halo2_mcc",
        None,
        None,
    );
    let saved = custom_editing_kit_profiles_value(&[moda.clone(), stock.clone()]);
    assert!(saved[1].get("tags_folder").is_none() && saved[1].get("data_folder").is_none());
    let loaded = load_custom_editing_kit_profiles(&json!({ "editing_kit_profiles": saved }));
    assert_eq!(loaded, vec![moda, stock]);

    let mut halo3 = custom_editing_kit_profiles_value(&[profile(
        "00000000-0000-4000-8000-00000000000c",
        "halo3_mcc",
        None,
        None,
    )]);
    halo3[0]["tags_folder"] = json!("tags_moda");
    let loaded = load_custom_editing_kit_profiles(&json!({ "editing_kit_profiles": halo3 }));
    assert_eq!(loaded[0].tags_folder, None);
}

#[test]
fn restore_prompt_resolves_the_current_custom_project_name_and_root() {
    let root = std::env::temp_dir();
    let mut saved = kit(&root.display().to_string(), Some(BrowserMode::Folders));
    saved.profile_id = Some("custom-h2-project".to_owned());
    let profile = CustomEditingKitProfile {
        read_only: false,
        git_tracked: false,
        id: "custom-h2-project".to_owned(),
        name: "Halo 2 Rebalance".to_owned(),
        game: "halo2_mcc".to_owned(),
        root: root.clone(),
        icon: None,
        tags_folder: None,
        data_folder: None,
    };

    let prompt = LastOpenedWindowsPrompt::from_session(
        LastSessionState { kits: vec![saved] },
        &[profile],
    )
    .expect("restore prompt exists");

    assert_eq!(
        prompt.kits[0].profile_name.as_deref(),
        Some("Halo 2 Rebalance")
    );
    assert_eq!(prompt.kits[0].profile_root.as_deref(), Some(root.as_path()));
}

#[test]
fn sessions_from_before_folder_windows_restore_with_none() {
    let value = serde_json::json!({
        "version": 5,
        "kits": [{
            "source": { "kind": "loose_folder", "path": "/h3" },
            "tags": [],
        }],
    });
    let restored = parse_last_session(&value).expect("version 5 session parses");
    assert!(restored.kits[0].folders.is_empty());
}

#[test]
fn chimp_packages_round_trip_and_keep_an_otherwise_empty_kit() {
    let session = LastSessionState {
        kits: vec![LastSessionKit {
            source_kind: LastSessionSourceKind::IoStoreContainerSet,
            source_path: PathBuf::from("C:/game"),
            game: Some("campaignevolved".to_owned()),
            profile_id: None,
            project_path: None,
            has_project: false,
            browser_mode: Some(BrowserMode::Folders),
            browser_sort: Some(BrowserSort::Name),
            tags: Vec::new(),
            folders: Vec::new(),
            chimp_packages: vec!["/Game/Vehicles/Warthog".to_owned()],
            active_chimp_package: Some("/Game/Vehicles/Warthog".to_owned()),
            bitmap_library_open: false,
            model_library_open: false,
            was_active: true,
        }],
    };
    let value = session_value(&session);
    assert_eq!(value["version"], 6);
    let restored = parse_last_session(&value).expect("session parses");
    assert_eq!(restored.kits.len(), 1);
    assert_eq!(restored.kits[0].chimp_packages, ["/Game/Vehicles/Warthog"]);
    assert_eq!(
        restored.kits[0].active_chimp_package.as_deref(),
        Some("/Game/Vehicles/Warthog")
    );
}

/// The Bitmap Library comes back open, and a workspace that had *only* the
/// library open still survives the round trip.
///
/// It cannot ride in `tags`: its pane key resolves to no entry, so the tag
/// loop drops it on the way out. A kit with no tags and no Chimp packages is
/// the case that would silently lose it.
#[test]
fn the_bitmap_library_round_trips_on_a_kit_with_nothing_else_open() {
    let mut only_library = kit("C:/halo3", Some(BrowserMode::Folders));
    only_library.tags = Vec::new();
    only_library.bitmap_library_open = true;
    let session = LastSessionState {
        kits: vec![only_library, kit("C:/reach", Some(BrowserMode::Groups))],
    };

    let restored = parse_last_session(&session_value(&session)).expect("session parses");

    assert_eq!(
        restored.kits.len(),
        2,
        "an empty workspace is still a workspace"
    );
    assert!(restored.kits[0].bitmap_library_open);
    assert!(
        !restored.kits[1].bitmap_library_open,
        "the flag belongs to its own kit, not to the session"
    );
}

/// The Model Library rides the same flag mechanism as the Bitmap Library,
/// and each library's flag comes back independently.
#[test]
fn the_model_library_round_trips_independently_of_the_bitmap_library() {
    let mut only_models = kit("C:/halo3", Some(BrowserMode::Folders));
    only_models.tags = Vec::new();
    only_models.model_library_open = true;
    let session = LastSessionState {
        kits: vec![only_models],
    };

    let restored = parse_last_session(&session_value(&session)).expect("session parses");

    assert!(restored.kits[0].model_library_open);
    assert!(
        !restored.kits[0].bitmap_library_open,
        "one library's flag must not drag the other's along"
    );
}

/// Sessions written before the Bitmap Library existed carry no flag, and
/// must read back as "it was not open" rather than failing to parse.
#[test]
fn a_session_without_the_bitmap_library_flag_still_loads() {
    let mut value = session_value(&LastSessionState {
        kits: vec![kit("C:/halo3", Some(BrowserMode::Folders))],
    });
    // Exactly what a version-4 file looks like: the field never written.
    value["version"] = serde_json::json!(4);
    value["kits"][0]
        .as_object_mut()
        .unwrap()
        .remove("bitmap_library");

    let restored = parse_last_session(&value).expect("an older session still parses");
    assert!(!restored.kits[0].bitmap_library_open);
    assert_eq!(restored.kits[0].tags.len(), 1, "its tags still come back");
}

/// Which workspace the user was looking at survives the round trip, and is
/// carried on the kit rather than as an index beside the list — the restore
/// prompt can drop kits, and an index would then name whichever one moved
/// into that slot.
#[test]
fn the_focused_kit_is_remembered_and_travels_with_its_own_kit() {
    let mut halo3 = kit("C:/halo3", Some(BrowserMode::Folders));
    let mut evolved = kit("C:/evolved", Some(BrowserMode::Groups));
    evolved.source_kind = LastSessionSourceKind::IoStoreContainerSet;
    halo3.was_active = true;
    evolved.was_active = false;
    let session = LastSessionState {
        kits: vec![evolved, halo3],
    };

    let value = session_value(&session);
    let restored = parse_last_session(&value).expect("session parses");
    assert_eq!(restored.kits.len(), 2);
    assert!(
        !restored.kits[0].was_active,
        "the container kit was not focused"
    );
    assert!(restored.kits[1].was_active, "the Halo 3 kit was focused");
    // It is the kit that is marked, not a position: the flag follows its
    // own workspace when the list is filtered.
    let kept = restored
        .kits
        .into_iter()
        .filter(|kit| kit.source_kind == LastSessionSourceKind::LooseFolder)
        .collect::<Vec<_>>();
    assert_eq!(kept.len(), 1);
    assert!(kept[0].was_active);
}

/// A session written before the focused workspace was recorded has no
/// `active` on any kit, and must still load rather than being rejected.
#[test]
fn a_session_without_a_focused_kit_still_loads() {
    let value = serde_json::json!({
        "version": 4,
        "kits": [{
            "source": { "kind": "loose_folder", "path": "C:/halo3" },
            "tags": [],
        }],
    });
    let restored = parse_last_session(&value).expect("session parses");
    assert_eq!(restored.kits.len(), 1);
    assert!(!restored.kits[0].was_active);
}

/// Each workspace keeps its own browser view, so a session holding a kit
/// in Folders and one in Groups must bring both back as they were — not
/// collapse them onto one setting.
#[test]
fn each_kit_restores_its_own_browser_view() {
    let session = LastSessionState {
        kits: vec![
            kit("/evolved", Some(BrowserMode::Folders)),
            kit("/reach", Some(BrowserMode::Groups)),
        ],
    };
    let restored = parse_last_session(&session_value(&session)).expect("round trip");
    assert_eq!(restored.kits[0].browser_mode, Some(BrowserMode::Folders));
    assert_eq!(restored.kits[1].browser_mode, Some(BrowserMode::Groups));
    assert_eq!(restored.kits[0].browser_sort, Some(BrowserSort::Name));
}

/// Sessions written before the view was saved carry none, which has to
/// stay distinguishable from a saved Folders so the restore can fall back
/// to the user's default instead of overriding it.
#[test]
fn sessions_without_a_saved_view_restore_none() {
    let value = serde_json::json!({
        "version": 3,
        "kits": [{
            "source": { "kind": "loose_folder", "path": "/h3" },
            "tags": [{ "key": "file:/h3/a.weapon", "label": "a", "group_tag": 1 }],
        }],
    });
    let session = parse_last_session(&value).expect("session parses");
    assert_eq!(session.kits[0].browser_mode, None);
    assert_eq!(session.kits[0].browser_sort, None);
}

/// A kit whose session is its project has no tags to save, so dropping the
/// project path on write left nothing to restore it from.
#[test]
fn a_projects_path_survives_the_round_trip() {
    let mut project_kit = kit("/evolved", None);
    project_kit.project_path = Some(PathBuf::from("/evolved/work.baboon"));
    project_kit.has_project = true;
    project_kit.tags.clear();
    let session = LastSessionState {
        kits: vec![project_kit],
    };
    let restored = parse_last_session(&session_value(&session)).expect("round trip");
    assert_eq!(
        restored.kits[0].project_path,
        Some(PathBuf::from("/evolved/work.baboon"))
    );
    assert!(restored.kits[0].has_project);
}

/// A workspace whose only content is its stash records no project path — its
/// edits live in the recovery file, which is found from the source root — so
/// "carries a project" cannot be inferred from that path any more. Reading it
/// that way would drop such a kit from the session entirely and lose the
/// stash with it.
#[test]
fn a_stash_only_kit_survives_the_round_trip() {
    let mut stash_kit = kit("/evolved", None);
    stash_kit.has_project = true;
    stash_kit.tags.clear();
    let session = LastSessionState {
        kits: vec![stash_kit],
    };
    let restored = parse_last_session(&session_value(&session)).expect("round trip");
    assert_eq!(restored.kits[0].project_path, None);
    assert!(restored.kits[0].has_project);
}

/// Sessions written before the recovery file and the project file were
/// separate recorded the recovery path as `project_path`, and set it for every
/// workspace that had a project at all. That is what `has_project` now means,
/// so its absence reads straight off the old field.
#[test]
fn a_legacy_sessions_recovery_path_still_restores_the_workspace() {
    let value = serde_json::json!({
        "version": 3,
        "kits": [{
            "source": {
                "kind": "iostore_container_set",
                "path": "/evolved",
                "project_path": "/data/campaign_evolved_recovery-abc123.baboon",
            },
            "tags": [],
        }],
    });
    let session = parse_last_session(&value).expect("session parses");
    assert!(session.kits[0].has_project, "the kit is still restored");
}
