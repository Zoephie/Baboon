use super::*;

#[test]
fn recent_icon_classification_does_not_require_the_path_to_exist() {
    let missing_tag = std::path::Path::new("Z:/missing/network/path/example.scenario");
    let missing_folder = std::path::Path::new("Z:/missing/network/path/tags");

    assert_eq!(recent_tag_icon_group(missing_tag).as_deref(), Some("scnr"));
    assert_eq!(recent_tag_icon_group(missing_folder), None);
}
