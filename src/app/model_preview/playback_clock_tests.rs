use super::*;

/// Two panes on the same tag draw the same playback state in one pass; the
/// clock moves once. Each pane used to move it, so playback ran at 2x.
#[test]
fn two_panes_advance_the_clock_once_per_pass() {
    let mut playback = PreviewAnimationPlayback {
        playing: true,
        ..Default::default()
    };
    advance_playback_clock(&mut playback, 7, 0.1, 10.0, 300.0);
    advance_playback_clock(&mut playback, 7, 0.1, 10.0, 300.0);
    assert!((playback.time - 0.1).abs() < 1e-6, "{}", playback.time);
    advance_playback_clock(&mut playback, 8, 0.1, 10.0, 300.0);
    assert!((playback.time - 0.2).abs() < 1e-6, "{}", playback.time);
}
