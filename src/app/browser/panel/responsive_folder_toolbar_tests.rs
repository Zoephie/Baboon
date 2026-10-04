use super::*;
use crate::app::shell::frame::sidebar_wrappable_path_label;

#[test]
fn search_stacks_before_the_toolbar_reaches_600_points() {
    assert!(!folder_browser_search_stacks(600.0));
    assert!(folder_browser_search_stacks(599.0));
}

#[test]
fn search_hint_names_the_current_folder() {
    assert_eq!(folder_browser_search_hint("brute"), "search brute folder");
}

#[test]
fn sidebar_paths_can_wrap_after_each_separator() {
    assert_eq!(
        sidebar_wrappable_path_label(r"C:\tags\objects"),
        "C:\\\u{200b}tags\\\u{200b}objects"
    );
}
