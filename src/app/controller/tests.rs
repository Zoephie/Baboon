use super::ensure_priority_suffix;
use std::path::{Path, PathBuf};

#[test]
fn restored_loose_tag_uses_the_current_sources_key() {
    let root = std::env::temp_dir().join(format!("baboon-session-key-{}", std::process::id()));
    let path = root.join("objects").join("characters").join("brute.model");
    std::fs::create_dir_all(path.parent().expect("tag has parent")).expect("create tag path");
    std::fs::write(&path, b"tag").expect("create tag");
    let canonical = std::fs::canonicalize(&path).expect("canonical tag path");
    let entry = crate::core::source::TagEntry {
        key: format!("file:{}", canonical.display()),
        display_path: "objects/characters/brute.model".to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: Some("model".to_owned()),
        location: crate::core::source::TagEntryLocation::LooseFile(canonical.clone()),
    };

    assert_eq!(
        super::loose_entry_key_for_canonical_path(std::iter::once(&entry), &canonical),
        Some(entry.key.clone())
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn last_opened_workspace_heading_prefers_the_named_project() {
    let source = PathBuf::from("Games").join("Halo Infinite");
    let project = PathBuf::from("Baboon Projects").join("Campaign Overhaul.baboon");

    assert_eq!(
        super::last_opened_workspace_heading(
            None,
            Some("halo_infinite"),
            &source,
            Some(&project)
        ),
        (
            "Campaign Overhaul".to_owned(),
            Some(project.display().to_string())
        )
    );
}

#[test]
fn last_opened_workspace_heading_keeps_the_source_fallback() {
    let source = Path::new(r"C:\Editing Kits\Custom Kit");

    assert_eq!(
        super::last_opened_workspace_heading(None, None, source, None),
        (source.display().to_string(), None)
    );
}

#[test]
fn last_opened_workspace_heading_prefers_the_custom_editing_kit_profile() {
    let source = Path::new(r"C:\Editing Kits\H2EK");

    assert_eq!(
        super::last_opened_workspace_heading(
            Some(("Halo 2 Rebalance", source)),
            Some("halo2_mcc"),
            source,
            None
        ),
        (
            "Halo 2 Rebalance".to_owned(),
            Some(source.display().to_string())
        )
    );
}

#[test]
fn only_workspace_close_actions_wait_for_chimp_documents() {
    assert!(super::close_action_includes_chimp(
        &super::PendingCloseAction::CloseApp
    ));
    assert!(super::close_action_includes_chimp(
        &super::PendingCloseAction::CloseKit(super::KitId(1))
    ));
    assert!(!super::close_action_includes_chimp(
        &super::PendingCloseAction::CloseAllTabs
    ));
    assert!(!super::close_action_includes_chimp(
        &super::PendingCloseAction::CloseTab("tag".to_owned())
    ));
}

/// Path and key equality per OS. On Windows the comparisons are of the
/// text, ignoring ASCII case; elsewhere paths compare by component and
/// keys exactly. A typed path or key has to answer every row the same.
#[test]
fn path_and_key_equality_follows_the_platform() {
    use super::same_entry_key;
    use super::tools::same_path_text;
    use crate::app::prefs::same_recent_path;
    // (a, b, equal on Windows, equal elsewhere)
    let paths = [
        (r"C:\Kits\H3EK\tags", r"C:\Kits\H3EK\tags", true, true),
        (r"C:\Kits\H3EK\tags", r"c:\kits\h3ek\TAGS", true, false),
        ("/kits/h3ek/tags", "/kits/H3EK/tags", true, false),
        (r"C:\Kits\H3EK", "C:/Kits/H3EK", false, false),
        ("/kits/h3ek/", "/kits/h3ek", false, true),
        ("/kits/./h3ek", "/kits/h3ek", false, true),
        (r"\\?\C:\Kits\H3EK", r"C:\Kits\H3EK", false, false),
        (r"\\Server\Share\H2EK", r"\\server\share\h2ek", true, false),
    ];
    for (a, b, windows, elsewhere) in paths {
        let expected = if cfg!(windows) { windows } else { elsewhere };
        let (a, b) = (Path::new(a), Path::new(b));
        assert_eq!(same_recent_path(a, b), expected, "same_recent_path {a:?} {b:?}");
        assert_eq!(same_path_text(a, b), expected, "same_path_text {a:?} {b:?}");
    }
    let keys = [
        (r"file:C:\Kits\tags\a.weapon", r"file:C:\Kits\tags\a.weapon", true, true),
        (r"file:C:\Kits\tags\a.weapon", r"file:c:\kits\TAGS\A.weapon", true, false),
        (r"file:C:\Kits\tags\a.weapon", "file:C:/Kits/tags/a.weapon", false, false),
        ("file:/kits/tags/a.weapon", "file:/kits/tags//a.weapon", false, false),
        ("cache:rm:shaders\\default", "CACHE:RM:SHADERS\\DEFAULT", true, false),
    ];
    for (a, b, windows, elsewhere) in keys {
        let expected = if cfg!(windows) { windows } else { elsewhere };
        assert_eq!(same_entry_key(a, b), expected, "same_entry_key {a:?} {b:?}");
    }
}

/// The close prompt's Save sends container tags to the container writers
/// and only file tags to the file save.
#[test]
fn the_close_prompt_saves_container_tags_through_the_containers() {
    use super::{ClosePromptSave, TagEntryLocation, close_prompt_save_route};
    let route = |location: TagEntryLocation| close_prompt_save_route(Some(&location));
    assert_eq!(
        route(TagEntryLocation::NewContainer {
            template: crate::core::source::NewContainerTemplate::Derived {
                group: "camera_track".to_owned(),
            },
            package: "/Game/Tags/objects/foo/bar-camera_track".to_owned(),
            group_tag: u32::from_be_bytes(*b"trak"),
        }),
        ClosePromptSave::NewContainer
    );
    assert_eq!(
        route(TagEntryLocation::Container {
            container: 0,
            rel_path: "Meteorite/Content/Tags/objects/a-biped.ubulk".to_owned(),
        }),
        ClosePromptSave::ContainerInPlace
    );
    assert_eq!(
        route(TagEntryLocation::LooseFile(PathBuf::from("/kit/tags/a.biped"))),
        ClosePromptSave::File
    );
    assert_eq!(
        route(TagEntryLocation::Monolithic {
            name: r"objects\a".to_owned(),
            group_tag: u32::from_be_bytes(*b"bipd"),
        }),
        ClosePromptSave::File
    );
    assert_eq!(close_prompt_save_route(None), ClosePromptSave::File);
}

/// A mod without `_P` mounts at the same priority as the game's own
/// containers and loses, so it builds correctly and does nothing. Renaming
/// the default to something meaningful is exactly how it gets dropped --
/// which is how one was reported.
/// A session written before the chosen folder was recorded holds the
/// resolved `Paks` directory. Restoring from it put that directory back
/// into the recents list on every launch, which is how "Paks" kept
/// reappearing however often it was removed.
#[test]
fn a_paks_directory_walks_back_up_to_the_opened_folder() {
    let root = std::env::temp_dir().join(format!("baboon-paks-{}", std::process::id()));
    let paks = root.join("Meteorite").join("Content").join("Paks");
    std::fs::create_dir_all(&paks).unwrap();
    // `find_paks_dir` needs a container present to recognise the folder.
    std::fs::write(paks.join("pakchunk0-WinGDK.utoc"), []).unwrap();

    assert_eq!(super::install_root_for_paks(&paks), root);
    // Already the opened folder: nothing to strip.
    assert_eq!(super::install_root_for_paks(&root), root);
    // The shorter layout the resolver also accepts.
    assert_eq!(
        super::install_root_for_paks(&root.join("Content").join("Paks")),
        root
    );
    // An unfamiliar layout is left exactly as it is rather than guessed at.
    let odd = root.join("somewhere").join("Paks");
    assert_eq!(super::install_root_for_paks(&odd), odd);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_mod_always_gets_the_priority_suffix() {
    assert_eq!(
        ensure_priority_suffix(PathBuf::from("/mods/h2a_magnum.utoc")),
        PathBuf::from("/mods/h2a_magnum_P.utoc")
    );
    // Already correct, including the platform suffix the game itself uses.
    assert_eq!(
        ensure_priority_suffix(PathBuf::from("/mods/mymod-WinGDK_P.utoc")),
        PathBuf::from("/mods/mymod-WinGDK_P.utoc")
    );
    // The loader folds case before comparing, so a lowercase suffix
    // already has priority and must not collect a second one.
    assert_eq!(
        ensure_priority_suffix(PathBuf::from("/mods/thing_p.utoc")),
        PathBuf::from("/mods/thing_p.utoc")
    );
    // A version before the suffix raises priority further; it is still a
    // suffixed name and must be left alone.
    assert_eq!(
        ensure_priority_suffix(PathBuf::from("/mods/thing_2_P.utoc")),
        PathBuf::from("/mods/thing_2_P.utoc")
    );
}

use super::*;

#[test]
fn normalize_container_tag_rel_cleans_path() {
    // Lowercases, normalizes separators, trims slashes, drops a leaf extension.
    assert_eq!(
        normalize_container_tag_rel("Objects\\Characters/Foo/Bar"),
        "objects/characters/foo/bar"
    );
    assert_eq!(
        normalize_container_tag_rel("/objects//foo/bar.biped/"),
        "objects/foo/bar"
    );
    assert_eq!(normalize_container_tag_rel("  Foo.Weapon  "), "foo");
    assert_eq!(normalize_container_tag_rel(""), "");
    assert_eq!(normalize_container_tag_rel("///"), "");
}

#[test]
fn explorer_select_arguments_keep_switch_separate_from_path_with_spaces() {
    let path = Path::new(r"C:\Program Files\H2EK\tags\objects\example.weapon");

    assert_eq!(
        explorer_select_args(path),
        [
            std::ffi::OsString::from("/select,"),
            path.as_os_str().to_owned(),
        ]
    );
}

#[test]
fn favorite_folder_explorer_path_stays_bound_to_its_rendered_tags_root() {
    let windows_rendered = Path::new(r"D:\HREK\tags\objects\characters");
    assert_eq!(
        loose_folder_explorer_path(Path::new(r"C:\OtherKit\tags"), windows_rendered),
        windows_rendered
    );

    let native_tags_root = std::env::temp_dir().join("baboon-hrek").join("tags");
    let native_relative = Path::new("objects").join("characters");
    let native_rendered = native_tags_root.join(&native_relative);
    assert_eq!(
        loose_folder_explorer_path(&native_tags_root, &native_relative),
        native_rendered
    );
}

fn unique_test_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("baboon-{name}-{}-{nanos}", std::process::id()))
}

fn write_classic_ce_tag(path: &Path, group: &[u8; 4]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut bytes = [0u8; 64];
    bytes[36..40].copy_from_slice(group);
    bytes[60..64].copy_from_slice(b"blam");
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn detect_editing_kit_paths_finds_all_known_common_folder_names() {
    let common = unique_test_dir("ek-detect-all");
    for shortcut in EDITING_KIT_SHORTCUTS {
        if shortcut.game.is_campaign_evolved() {
            continue;
        }
        std::fs::create_dir_all(common.join(shortcut.label).join("tags")).unwrap();
    }
    let campaign_evolved_paks = common
        .join("Halo Campaign Evolved")
        .join("Meteorite")
        .join("Content")
        .join("Paks");
    std::fs::create_dir_all(&campaign_evolved_paks).unwrap();
    std::fs::write(campaign_evolved_paks.join("pakchunk0-WinGDK.utoc"), []).unwrap();

    let detected = detect_editing_kit_paths_in_common_roots(vec![common.clone()]);

    for shortcut in EDITING_KIT_SHORTCUTS {
        let expected = if shortcut.game.is_campaign_evolved() {
            common.join("Halo Campaign Evolved")
        } else {
            common.join(shortcut.label)
        };
        assert_eq!(detected.get(shortcut.game.as_str()), Some(&expected));
    }
    let _ = std::fs::remove_dir_all(common);
}

#[test]
fn detect_editing_kit_paths_ignores_campaign_evolved_without_containers() {
    let common = unique_test_dir("ek-detect-campaign-evolved-containers-required");
    std::fs::create_dir_all(common.join("Halo Campaign Evolved")).unwrap();

    let detected = detect_editing_kit_paths_in_common_roots(vec![common.clone()]);

    assert!(!detected.contains_key("haloce_evolved"));
    let _ = std::fs::remove_dir_all(common);
}

#[test]
fn detect_editing_kit_paths_ignores_folder_without_tags_child() {
    let common = unique_test_dir("ek-detect-tags-required");
    std::fs::create_dir_all(common.join("H3EK")).unwrap();
    std::fs::create_dir_all(common.join("H4EK").join("tags")).unwrap();

    let detected = detect_editing_kit_paths_in_common_roots(vec![common.clone()]);

    assert!(!detected.contains_key("halo3_mcc"));
    assert_eq!(detected.get("halo4_mcc"), Some(&common.join("H4EK")));
    let _ = std::fs::remove_dir_all(common);
}

#[test]
fn apply_detected_editing_kit_paths_fills_blanks_only() {
    let mut paths = HashMap::from([
        ("halo3_mcc".to_owned(), PathBuf::from("C:/Custom/H3EK")),
        (
            "haloce_evolved".to_owned(),
            PathBuf::from("D:/Custom/Halo Campaign Evolved"),
        ),
    ]);
    let mut inputs = HashMap::from([
        ("halo3_mcc".to_owned(), "C:/Custom/H3EK".to_owned()),
        (
            "haloce_evolved".to_owned(),
            "D:/Custom/Halo Campaign Evolved".to_owned(),
        ),
    ]);
    let mut attention = Some("halo4_mcc".to_owned());
    let detected = HashMap::from([
        ("halo3_mcc".to_owned(), PathBuf::from("C:/Steam/H3EK")),
        ("halo4_mcc".to_owned(), PathBuf::from("C:/Steam/H4EK")),
        (
            "haloce_evolved".to_owned(),
            PathBuf::from("C:/Steam/Halo Campaign Evolved"),
        ),
    ]);

    let added =
        apply_detected_editing_kit_paths(&mut paths, &mut inputs, &mut attention, &detected);

    assert_eq!(added, 1);
    assert_eq!(
        paths.get("halo3_mcc"),
        Some(&PathBuf::from("C:/Custom/H3EK"))
    );
    assert_eq!(
        paths.get("halo4_mcc"),
        Some(&PathBuf::from("C:/Steam/H4EK"))
    );
    assert_eq!(
        paths.get("haloce_evolved"),
        Some(&PathBuf::from("D:/Custom/Halo Campaign Evolved"))
    );
    assert_eq!(
        inputs.get("halo4_mcc").map(String::as_str),
        Some("C:/Steam/H4EK")
    );
    assert_eq!(attention, None);
}

#[test]
fn save_as_registers_classic_ce_copy_in_loaded_folder() {
    let root = unique_test_dir("save-as-register-ce");
    let old_path = root.join("objects").join("old").join("old.gbxmodel");
    write_classic_ce_tag(&old_path, b"mod2");
    std::fs::create_dir_all(root.join("objects")).unwrap();

    let names = TagNameIndex::default();
    let old_entry = loose_file_entry(&root, &old_path, &names)
        .unwrap()
        .expect("old CE tag should probe");
    let entries = vec![old_entry.clone()];
    let mut source = LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: root.clone(),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names,
        game: None,
        entries: entries.clone(),
        tree: crate::core::source::build_folder_directory_tree(&root).unwrap(),
        group_tree: crate::core::source::build_group_tree(&entries),
        all_entries: entries,
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    };

    let saved_path = root.join("saved").join("cyborg.gbxmodel");
    write_classic_ce_tag(&saved_path, b"mod2");

    let registered = register_saved_copy_in_loaded_source(&mut source, &saved_path).unwrap();
    // The key the folder scan gives the copy. The root is a temp folder,
    // which is not canonical on macOS (/var is /private/var), and keying the
    // copy off canonical paths gave it a key the scan never makes.
    let scanned_key = loose_file_entry(&root, &saved_path, &TagNameIndex::default())
        .unwrap()
        .unwrap()
        .key;

    let _ = std::fs::remove_dir_all(&root);
    assert!(registered);
    assert!(
        source.entries.iter().any(|entry| entry.key == scanned_key),
        "the copy is keyed like the folder scan"
    );
    assert!(
        source
            .tree
            .children
            .iter()
            .any(|node| node.label == "saved")
    );
    assert!(source.entries.iter().any(|entry| {
        entry.display_path == "saved/cyborg.gbxmodel"
            && entry.group_tag == u32::from_be_bytes(*b"mod2")
    }));
    assert!(source.all_entries.iter().any(|entry| {
        entry.display_path == "saved/cyborg.gbxmodel"
            && entry.group_tag == u32::from_be_bytes(*b"mod2")
    }));
    assert!(source.group_tree.children.iter().any(|node| {
        node.entries
            .iter()
            .any(|&index| source.all_entries[index].display_path == "saved/cyborg.gbxmodel")
    }));
}

#[test]
fn moved_tags_remap_favorite_relative_paths() {
    let root = PathBuf::from("C:/Games/H2EK/tags");
    let old_relative = PathBuf::from("objects/old/brute.model");
    let new_relative = PathBuf::from("objects/characters/brute/brute.model");
    let mut favorites = vec![old_relative.clone(), PathBuf::from("sound/brute.sound")];
    let mut remap = HashMap::new();
    remap.insert(
        format!("file:{}", root.join(&old_relative).display()),
        format!("file:{}", root.join(&new_relative).display()),
    );

    remap_favorite_paths(&root, &mut favorites, &remap);

    assert_eq!(favorites[0], new_relative);
    assert_eq!(favorites[1], PathBuf::from("sound/brute.sound"));
}

#[test]
fn parse_steam_library_paths_reads_libraryfolders_vdf_paths() {
    let text = r#"
            "libraryfolders"
            {
                "0"
                {
                    "path"      "C:\\Program Files (x86)\\Steam"
                }
                "1"
                {
                    "path"      "D:\\SteamLibrary"
                }
            }
        "#;

    let paths = parse_steam_library_paths(text);

    assert!(paths.contains(&PathBuf::from(r"C:\Program Files (x86)\Steam")));
    assert!(paths.contains(&PathBuf::from(r"D:\SteamLibrary")));
}

#[test]
fn ancestor_block_indices_splits_indexed_path() {
    // Nested blocks: each pair's path is the drawn `path_prefix` (parent
    // indices kept, own index dropped).
    assert_eq!(
        ancestor_block_indices("custom references[3]/sounds[1]/melee sound"),
        vec![
            ("custom references".to_owned(), 3),
            ("custom references[3]/sounds".to_owned(), 1),
        ],
    );
    // A plain struct segment between blocks carries no selection.
    assert_eq!(
        ancestor_block_indices("weapon[2]/melee/damage sound"),
        vec![("weapon".to_owned(), 2)],
    );
    // A top-level (unindexed) reference field has no ancestor blocks.
    assert_eq!(
        ancestor_block_indices("havok cleanup resources"),
        Vec::<(String, usize)>::new(),
    );
    assert_eq!(
        ancestor_block_indices("custom references#5[3]/sounds#2[1]/melee sound#4"),
        vec![
            ("custom references#5".to_owned(), 3),
            ("custom references#5[3]/sounds#2".to_owned(), 1),
        ],
    );
    // Foundation renders inherited wrappers without ordinals, so selector
    // IDs beneath Unit/Object must preserve those plain wrapper segments.
    assert_eq!(
        ancestor_block_indices("unit/object/functions#25[2]/import name#3"),
        vec![("unit/object/functions#25".to_owned(), 2)],
    );
    // Reference-jump paths may retain schema ordinals on inherited wrappers;
    // normalize those to the same selector ID as canonical Find paths.
    assert_eq!(
        ancestor_block_indices("unit#0/object#0/functions#25[2]/import name#3"),
        vec![("unit/object/functions#25".to_owned(), 2)],
    );
}

#[test]
fn occurrence_label_keeps_indices_and_cleans_names() {
    assert_eq!(
        occurrence_label("custom references[3]/melee sound"),
        "custom references[3] › melee sound",
    );
    assert_eq!(
        occurrence_label("havok cleanup resources"),
        "havok cleanup resources"
    );
    assert_eq!(
        occurrence_label("custom references#5[3]/melee sound#4"),
        "custom references[3] › melee sound",
    );
}

#[test]
fn normalize_ref_matches_dependency_key_form() {
    assert_eq!(
        normalize_ref("Sound/Materials/Hard/Human_Weap_Melee"),
        normalize_ref("sound\\materials\\hard\\human_weap_melee"),
    );
}

fn collect_terminal_output(input: &[u8]) -> Vec<(&'static str, String)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = egui::Context::default();
    let mut log_file = None;
    let mut log_error_reported = false;
    let result = stream_terminal_output(
        std::io::Cursor::new(input),
        &tx,
        &ctx,
        &mut log_file,
        &mut log_error_reported,
    );
    assert!(result.is_ok());
    drop(tx);

    rx.try_iter()
        .filter_map(|message| match message {
            WorkerMessage::TerminalLine(line) => Some(("line", line)),
            _ => None,
        })
        .collect()
}

#[test]
fn terminal_output_appends_carriage_return_progress() {
    let output = collect_terminal_output(
        b"building bsp3d children... 10%\rbuilding bsp3d children... 70%\rbuilding bsp3d children... 100%\nnext\n",
    );

    assert_eq!(
        output,
        vec![
            ("line", "building bsp3d children... 10%".to_owned()),
            ("line", "building bsp3d children... 70%".to_owned()),
            ("line", "building bsp3d children... 100%".to_owned()),
            ("line", "next".to_owned()),
        ]
    );
}

#[test]
fn terminal_output_treats_crlf_as_newline() {
    let output = collect_terminal_output(b"done\r\nnext");

    assert_eq!(
        output,
        vec![("line", "done".to_owned()), ("line", "next".to_owned()),]
    );
}

#[test]
fn terminal_output_full_log_keeps_carriage_return_progress() {
    let path = std::env::temp_dir().join(format!(
        "baboon-terminal-test-{}.log",
        terminal_log_timestamp()
    ));
    let file = std::fs::File::create(&path);
    assert!(file.is_ok());

    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = egui::Context::default();
    let mut log_file = file.ok();
    let mut log_error_reported = false;
    let result = stream_terminal_output(
        std::io::Cursor::new(b"building... 10%\rbuilding... 60%\rbuilding... 100%\n"),
        &tx,
        &ctx,
        &mut log_file,
        &mut log_error_reported,
    );
    assert!(result.is_ok());
    drop(log_file);
    drop(tx);

    let output: Vec<_> = rx
        .try_iter()
        .filter_map(|message| match message {
            WorkerMessage::TerminalLine(line) => Some(("line", line)),
            _ => None,
        })
        .collect();
    assert_eq!(
        output,
        vec![
            ("line", "building... 10%".to_owned()),
            ("line", "building... 60%".to_owned()),
            ("line", "building... 100%".to_owned()),
        ]
    );

    let text = std::fs::read_to_string(&path);
    assert!(text.is_ok());
    if let Ok(text) = text {
        assert!(text.contains("building... 10%\n"));
        assert!(text.contains("building... 60%\n"));
        assert!(text.contains("building... 100%\n"));
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn terminal_output_handles_heavy_carriage_return_progress() {
    let mut input = Vec::new();
    for index in 0..25_000 {
        input.extend_from_slice(format!("building bsp3d children... {index}%\r").as_bytes());
    }
    input.extend_from_slice(b"done\n");

    let output = collect_terminal_output(&input);

    assert_eq!(output.len(), 25_001);
    assert_eq!(
        output.first(),
        Some(&("line", "building bsp3d children... 0%".to_owned()))
    );
    assert_eq!(output.last(), Some(&("line", "done".to_owned())));
}

#[test]
fn terminal_line_severity_classifies_tool_markers() {
    assert!(matches!(
        TerminalLineEntry::new("-ERROR- bad connection".to_owned()).severity,
        TerminalLineSeverity::Error
    ));
    assert!(matches!(
        TerminalLineEntry::new("WARNING overlapping surfaces".to_owned()).severity,
        TerminalLineSeverity::Warning
    ));
    assert!(matches!(
        TerminalLineEntry::new("[exit 0]".to_owned()).severity,
        TerminalLineSeverity::Success
    ));
    assert!(matches!(
        TerminalLineEntry::new("[exit 2]".to_owned()).severity,
        TerminalLineSeverity::Error
    ));
    assert!(matches!(
        TerminalLineEntry::new("=== summary".to_owned()).severity,
        TerminalLineSeverity::Summary
    ));
}

#[test]
fn terminal_visible_lines_are_trimmed_to_limit() {
    let mut lines = Vec::new();
    for index in 0..(TERMINAL_VISIBLE_LINE_LIMIT + 10) {
        lines.push(TerminalLineEntry::new(format!("line {index}")));
    }

    trim_terminal_lines(&mut lines);

    assert_eq!(lines.len(), TERMINAL_VISIBLE_LINE_TRIM_TARGET);
    assert_eq!(
        lines.first().map(|line| line.text.as_str()),
        Some("line 2010")
    );
}
