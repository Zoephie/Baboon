//! Unit tests for embedded button icons.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;

#[test]
fn button_icon_lookup_uses_expected_assets() {
    assert!(button_icon_svg(ButtonIcon::Open).contains("<svg"));
    assert!(button_icon_svg(ButtonIcon::Import).contains("<svg"));
    assert!(button_icon_svg(ButtonIcon::Export).contains("<svg"));
    assert!(button_icon_svg(ButtonIcon::Clear).contains("<svg"));
    assert!(button_icon_svg(ButtonIcon::Search).contains("<svg"));
    assert!(button_icon_svg(ButtonIcon::Function).contains("<svg"));
    assert!(button_icon_svg(ButtonIcon::Group).contains("<svg"));
    assert!(button_icon_svg(ButtonIcon::FolderClosed).contains("<svg"));
    assert!(button_icon_svg(ButtonIcon::FolderOpen).contains("<svg"));
}

#[test]
fn colorized_icon_replaces_current_color() {
    let svg = colorized_icon_svg(ButtonIcon::Clear, Color32::from_rgb(1, 2, 3));
    assert!(svg.contains("#010203"));
    assert!(!svg.contains("currentColor"));
}

#[test]
fn button_icon_uri_changes_with_pixels_per_point() {
    let low = button_icon_uri_for_pixels_per_point(ButtonIcon::Open, Color32::WHITE, 1.0);
    let high = button_icon_uri_for_pixels_per_point(ButtonIcon::Open, Color32::WHITE, 2.0);
    assert_ne!(low, high);
    assert!(low.contains("dpi100"));
    assert!(high.contains("dpi200"));
}
