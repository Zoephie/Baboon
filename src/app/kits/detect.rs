//! Editing-kit discovery and Steam library path parsing.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

const CAMPAIGN_EVOLVED_INSTALL_FOLDER: &str = "Halo Campaign Evolved";

/// Converts legacy game-keyed entries and discovered installs to ordinary profiles.
/// Missing installs are retained during migration; validation is a separate concern.
pub(in crate::app) fn add_standard_editing_kit_profiles(
    profiles: &mut Vec<CustomEditingKitProfile>,
    paths: &HashMap<String, PathBuf>,
) -> usize {
    let mut added = 0;
    for (index, shortcut) in EDITING_KIT_SHORTCUTS.into_iter().enumerate() {
        let Some(path) = paths
            .get(shortcut.game.as_str())
            .filter(|path| !path.as_os_str().is_empty())
        else {
            continue;
        };
        let layout = validate_builtin_editing_kit(shortcut, Some(path));
        let root = layout
            .layout()
            .map(|layout| layout.root.clone())
            .unwrap_or_else(|| canonical_or_clean(path));
        // Kits may share a root (a CE or H2 kit choosing another tags folder
        // beside the stock one), so the stock kit is already there only when
        // a profile uses its tags folder.
        let tags = layout
            .layout()
            .map(|layout| layout.tags.clone())
            .unwrap_or_else(|| root.join("tags"));
        if profiles
            .iter()
            .any(|profile| same_recent_path(&profile_tags_folder(profile), &tags))
        {
            continue;
        }
        // Stable IDs prevent identity churn if legacy preferences are read again.
        let mut id =
            uuid::Uuid::from_u128(0xbab00000000040008000000000000000 + index as u128).to_string();
        if profiles.iter().any(|profile| profile.id == id) {
            id = uuid::Uuid::new_v4().to_string();
        }
        profiles.push(CustomEditingKitProfile {
            read_only: false,
            git_tracked: false,
            id,
            name: shortcut.label.to_owned(),
            game: shortcut.game.as_str().to_owned(),
            root,
            icon: None,
            tags_folder: None,
            data_folder: None,
        });
        added += 1;
    }
    added
}

pub(in crate::app) fn detect_editing_kit_paths() -> HashMap<String, PathBuf> {
    detect_editing_kit_paths_in_common_roots(steam_common_roots())
}

pub(in crate::app) fn detect_editing_kit_paths_in_common_roots<I>(
    common_roots: I,
) -> HashMap<String, PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut detected = HashMap::new();
    for common_root in common_roots {
        let campaign_evolved = common_root.join(CAMPAIGN_EVOLVED_INSTALL_FOLDER);
        if !detected.contains_key(GameId::CampaignEvolved.as_str())
            && crate::core::source::find_paks_dir(&campaign_evolved).is_some()
        {
            detected.insert(GameId::CampaignEvolved.as_str().to_owned(), campaign_evolved);
        }

        for shortcut in EDITING_KIT_SHORTCUTS {
            if shortcut.game.is_campaign_evolved() {
                continue;
            }
            if detected.contains_key(shortcut.game.as_str()) {
                continue;
            }
            let candidate = common_root.join(shortcut.label);
            if candidate.is_dir() && candidate.join("tags").is_dir() {
                detected.insert(shortcut.game.as_str().to_owned(), candidate);
            }
        }
    }
    detected
}

#[cfg(test)]
pub(in crate::app) fn apply_detected_editing_kit_paths(
    editing_kit_paths: &mut HashMap<String, PathBuf>,
    editing_kit_path_inputs: &mut HashMap<String, String>,
    editing_kit_path_attention: &mut Option<String>,
    detected: &HashMap<String, PathBuf>,
) -> usize {
    let mut added = 0;
    for shortcut in EDITING_KIT_SHORTCUTS {
        let has_existing = editing_kit_paths
            .get(shortcut.game.as_str())
            .is_some_and(|path| !path.as_os_str().is_empty());
        if has_existing {
            continue;
        }
        let Some(path) = detected.get(shortcut.game.as_str()) else {
            continue;
        };
        editing_kit_paths.insert(shortcut.game.as_str().to_owned(), path.clone());
        editing_kit_path_inputs.insert(shortcut.game.as_str().to_owned(), path.display().to_string());
        if editing_kit_path_attention.as_deref() == Some(shortcut.game.as_str()) {
            *editing_kit_path_attention = None;
        }
        added += 1;
    }
    added
}

fn steam_common_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for steam_root in default_steam_roots() {
        push_unique_path(&mut roots, steam_root.join("steamapps").join("common"));
        let library_file = steam_root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(library_file) {
            for library_root in parse_steam_library_paths(&text) {
                push_unique_path(&mut roots, library_root.join("steamapps").join("common"));
            }
        }
    }
    roots
}

fn default_steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(root) = std::env::var_os(var) {
            push_unique_path(&mut roots, PathBuf::from(root).join("Steam"));
        }
    }
    push_unique_path(&mut roots, PathBuf::from(r"C:\Program Files (x86)\Steam"));
    roots
}

pub(in crate::app) fn parse_steam_library_paths(text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for line in text.lines() {
        let tokens = quoted_vdf_tokens(line);
        if tokens.len() >= 2 && tokens[0].eq_ignore_ascii_case("path") {
            push_unique_path(&mut paths, PathBuf::from(&tokens[1]));
        }
    }
    paths
}

fn quoted_vdf_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut in_quote = false;
    let mut escape = false;
    for ch in line.chars() {
        if !in_quote {
            if ch == '"' {
                in_quote = true;
                token.clear();
            }
            continue;
        }
        if escape {
            token.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '"' => {
                tokens.push(token.clone());
                token.clear();
                in_quote = false;
            }
            _ => token.push(ch),
        }
    }
    tokens
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| same_path_text(existing, &path)) {
        paths.push(path);
    }
}

pub(in crate::app) fn same_path_text(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

#[cfg(test)]
mod kit_path_tests {
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
        let common = crate::core::test_kits::unique_temp_path("ek-detect-all");
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
        let common = crate::core::test_kits::unique_temp_path("ek-detect-campaign-evolved-containers-required");
        std::fs::create_dir_all(common.join("Halo Campaign Evolved")).unwrap();

        let detected = detect_editing_kit_paths_in_common_roots(vec![common.clone()]);

        assert!(!detected.contains_key("haloce_evolved"));
        let _ = std::fs::remove_dir_all(common);
    }

    #[test]
    fn detect_editing_kit_paths_ignores_folder_without_tags_child() {
        let common = crate::core::test_kits::unique_temp_path("ek-detect-tags-required");
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
}
