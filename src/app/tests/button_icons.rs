//! Unit tests for embedded button icons.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;

#[test]
fn button_icon_lookup_uses_expected_assets() {
    let icons = [
        ButtonIcon::Add,
        ButtonIcon::Browse,
        ButtonIcon::Clear,
        ButtonIcon::Closed,
        ButtonIcon::CopyPath,
        ButtonIcon::Copy,
        ButtonIcon::Doc,
        ButtonIcon::Duplicate,
        ButtonIcon::Export,
        ButtonIcon::Favourite,
        ButtonIcon::FileExplorer,
        ButtonIcon::Filter,
        ButtonIcon::Find,
        ButtonIcon::FolderClosed,
        ButtonIcon::FolderOpen,
        ButtonIcon::Function,
        ButtonIcon::Garbage,
        ButtonIcon::Group,
        ButtonIcon::Import,
        ButtonIcon::InsertRow,
        ButtonIcon::Json,
        ButtonIcon::JumpTo,
        ButtonIcon::JumpUp,
        ButtonIcon::Left,
        ButtonIcon::Move,
        ButtonIcon::Open,
        ButtonIcon::Opened,
        ButtonIcon::Other,
        ButtonIcon::Remove,
        ButtonIcon::Rename,
        ButtonIcon::Right,
        ButtonIcon::Save,
        ButtonIcon::SearchBar,
        ButtonIcon::Search,
        ButtonIcon::Settings,
        ButtonIcon::Sort,
        ButtonIcon::WindowMode,
    ];
    for icon in icons {
        assert!(button_icon_svg(icon).contains("<svg"), "missing {icon:?}");
    }
}

#[test]
fn colorized_icon_replaces_current_color() {
    let svg = colorized_icon_svg(ButtonIcon::Open, Color32::from_rgb(1, 2, 3));
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
