use std::path::{Path, PathBuf};

use super::*;

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

#[test]
fn moved_tags_remap_favorite_relative_paths() {
    let root = PathBuf::from("C:/Games/H2EK/tags");
    let old_relative = PathBuf::from("objects/old/brute.model");
    let new_relative = PathBuf::from("objects/characters/brute/brute.model");
    let mut favorites = vec![old_relative.clone(), PathBuf::from("sound/brute.sound")];
    let mut remap = HashMap::new();
    remap.insert(
        file_entry_key(&root.join(&old_relative)),
        file_entry_key(&root.join(&new_relative)),
    );

    remap_favorite_paths(&root, &mut favorites, &remap);

    assert_eq!(favorites[0], new_relative);
    assert_eq!(favorites[1], PathBuf::from("sound/brute.sound"));
}
