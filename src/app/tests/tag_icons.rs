//! Unit tests for tag-group icon selection.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;

#[test]
fn tag_icon_lookup_uses_expected_group_mappings() {
    assert!(get_icon_svg("bipd").contains("<svg"));
    assert!(get_icon_svg("actr").contains("<svg"));
    assert!(get_icon_svg("actv").contains("<svg"));
    assert!(get_icon_svg("antr").contains("<svg"));
    assert!(get_icon_svg("mod2").contains("<svg"));
    assert!(get_icon_svg("shad").contains("<svg"));
    assert!(get_icon_svg("rmsh").contains("<svg"));
    assert!(get_icon_svg("char").contains("<svg"));
    assert!(get_icon_svg("jpt!").contains("<svg"));
    assert!(get_icon_svg("lens").contains("<svg"));
    assert!(get_icon_svg("ligh").contains("<svg"));
    assert!(get_icon_svg("matg").contains("<svg"));
    assert!(get_icon_svg("styl").contains("<svg"));
    assert!(get_icon_svg("unknown").contains("<svg"));
}

#[test]
fn tag_icon_uri_changes_with_pixels_per_point() {
    let low = tag_icon_uri_for_pixels_per_point("bipd", 1.0);
    let high = tag_icon_uri_for_pixels_per_point("bipd", 2.0);
    assert_ne!(low, high);
    assert!(low.contains("dpi100"));
    assert!(high.contains("dpi200"));
}
