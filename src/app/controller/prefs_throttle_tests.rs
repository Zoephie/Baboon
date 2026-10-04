use super::*;

/// The per-frame prefs check runs at most once a second. It ran every
/// frame, and wrote prefs.json every frame while a slider was dragged.
#[test]
fn the_per_frame_prefs_check_runs_once_a_second() {
    // Unchanged prefs, so nothing is written: this only watches the clock.
    let mut app = Baboon::for_test();
    app.persist_prefs_throttled(10.0);
    assert_eq!(app.prefs_next_check_at, 11.0);
    app.persist_prefs_throttled(10.5);
    assert_eq!(app.prefs_next_check_at, 11.0, "inside the second: skipped");
    app.persist_prefs_throttled(11.2);
    assert_eq!(app.prefs_next_check_at, 12.2);
}
