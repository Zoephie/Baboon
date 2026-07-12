//! Unit tests for top-level UI helpers.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;

#[test]
fn terminal_line_visuals_are_distinct_by_severity() {
    let normal = terminal_line_color(TerminalLineSeverity::Normal);
    assert_eq!(terminal_line_color(TerminalLineSeverity::Summary), normal);
    assert_ne!(terminal_line_color(TerminalLineSeverity::Warning), normal);
    assert_ne!(terminal_line_color(TerminalLineSeverity::Error), normal);
    assert_ne!(terminal_line_color(TerminalLineSeverity::Success), normal);
    assert!(terminal_line_is_strong(TerminalLineSeverity::Summary));
    assert!(terminal_line_is_strong(TerminalLineSeverity::Error));
    assert!(!terminal_line_is_strong(TerminalLineSeverity::Warning));
    assert!(!terminal_line_is_strong(TerminalLineSeverity::Success));
    assert!(!terminal_line_is_strong(TerminalLineSeverity::Normal));
}

#[test]
fn monitor_commands_are_game_specific() {
    assert_eq!(
        monitor_commands_for_game(Some("halo2_mcc")),
        &[
            "monitor-bitmaps",
            "monitor-bitmaps-data-and-tags",
            "monitor-models",
            "monitor-structures",
        ]
    );
    assert_eq!(
        monitor_commands_for_game(Some("halo4_mcc")),
        &["monitor-bitmaps", "monitor-strings"]
    );
    assert!(monitor_commands_for_game(Some("haloce_mcc")).is_empty());
    assert!(monitor_commands_for_game(None).is_empty());
}
