use super::remap_favorite_folders;
use std::path::{Path, PathBuf};

#[test]
fn favorite_folders_at_or_under_a_moved_folder_follow_it() {
    let mut folders = vec![
        PathBuf::from("objects/props"),
        PathBuf::from("Objects/Props/Barrels"),
        PathBuf::from("objects/propsheet"),
        PathBuf::from("levels/test"),
    ];
    remap_favorite_folders(&mut folders, Path::new("objects/props"), Path::new("levels/crates"));
    assert_eq!(
        folders,
        vec![
            PathBuf::from("levels/crates"),
            PathBuf::from("levels/crates/Barrels"),
            PathBuf::from("objects/propsheet"),
            PathBuf::from("levels/test"),
        ]
    );
}
