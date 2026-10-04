use super::*;

#[test]
fn mirrors_the_display_path() {
    assert_eq!(
        safe_relative_path("levels/solo/c10/c10.scenario"),
        Some(
            PathBuf::from("levels")
                .join("solo")
                .join("c10")
                .join("c10.scenario")
        )
    );
}

#[test]
fn trims_a_padded_fourcc_extension() {
    // `format_group_tag` pads a three-letter group to four, and a file name
    // ending in a space cannot be created on Windows.
    assert_eq!(
        safe_relative_path("objects/foo.mat "),
        Some(PathBuf::from("objects").join("foo.mat"))
    );
}

#[test]
fn refuses_to_escape_the_output_root() {
    assert_eq!(
        safe_relative_path("../../windows/system32/foo.scenario"),
        Some(
            PathBuf::from("windows")
                .join("system32")
                .join("foo.scenario")
        )
    );
    assert_eq!(
        safe_relative_path("c:/absolute/foo.scenario"),
        Some(PathBuf::from("absolute").join("foo.scenario"))
    );
}

#[test]
fn rejects_a_path_with_nothing_left() {
    assert_eq!(safe_relative_path("../.."), None);
    assert_eq!(safe_relative_path(""), None);
}
