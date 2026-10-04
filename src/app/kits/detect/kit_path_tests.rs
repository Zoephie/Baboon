use std::path::{Path, PathBuf};

use super::*;

/// Path and key equality per OS. On Windows the comparisons are of the
/// text, ignoring ASCII case; elsewhere paths compare by component and
/// keys exactly. A typed path or key has to answer every row the same.
#[test]
fn path_and_key_equality_follows_the_platform() {
    use crate::core::tag_key::same_entry_key;
    use crate::app::kits::detect::same_path_text;
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

#[test]
fn detect_editing_kit_paths_finds_all_known_common_folder_names() {
    let common = crate::test_kits::unique_temp_path("ek-detect-all");
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
    let common = crate::test_kits::unique_temp_path("ek-detect-campaign-evolved-containers-required");
    std::fs::create_dir_all(common.join("Halo Campaign Evolved")).unwrap();

    let detected = detect_editing_kit_paths_in_common_roots(vec![common.clone()]);

    assert!(!detected.contains_key("haloce_evolved"));
    let _ = std::fs::remove_dir_all(common);
}

#[test]
fn detect_editing_kit_paths_ignores_folder_without_tags_child() {
    let common = crate::test_kits::unique_temp_path("ek-detect-tags-required");
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
