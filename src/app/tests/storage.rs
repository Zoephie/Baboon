use super::*;

fn unique_root(label: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("baboon_storage_{label}_{stamp}"))
}

#[test]
fn portable_preferences_take_precedence() {
    let root = unique_root("precedence");
    let portable = root.join("portable");
    let installed = root.join("installed");
    let legacy = root.join("legacy");
    std::fs::create_dir_all(&portable).unwrap();
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::write(portable.join("prefs.json"), "{}").unwrap();
    std::fs::write(installed.join("prefs.json"), "{}").unwrap();

    let found = detect_at(&portable, &installed, &legacy);

    assert_eq!(found.mode, Some(StorageMode::Portable));
    assert_eq!(found.prefs_path, Some(portable.join("prefs.json")));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_and_legacy_installed_preferences_are_detected() {
    let root = unique_root("installed");
    let portable = root.join("portable");
    let installed = root.join("installed");
    let legacy = root.join("legacy");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("prefs.json"), "{}").unwrap();

    let found = detect_at(&portable, &installed, &legacy);

    assert_eq!(found.mode, Some(StorageMode::Installed));
    assert!(found.used_legacy_prefs);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn fresh_install_leaves_storage_undecided() {
    let root = unique_root("fresh");
    let found = detect_at(
        &root.join("portable"),
        &root.join("installed"),
        &root.join("legacy"),
    );

    assert_eq!(found.mode, None);
    assert!(found.prefs_path.is_none());
}

#[test]
fn portable_redirects_every_automatic_state_path() {
    let portable = PathBuf::from("portable");
    let installed = PathBuf::from("installed");
    for name in [
        "prefs.json",
        "last_session.json",
        "indexes.sqlite3",
        "halo3_mcc_index.json",
        "halo3_mcc_keywords.json",
        "terminal-logs",
    ] {
        assert_eq!(
            path_at(StorageMode::Portable, &portable, &installed, name),
            portable.join(name)
        );
    }
}

#[test]
fn installed_state_paths_remain_under_app_data() {
    let portable = PathBuf::from("portable");
    let installed = PathBuf::from("installed");
    assert_eq!(
        path_at(StorageMode::Installed, &portable, &installed, "prefs.json"),
        installed.join("prefs.json")
    );
}
