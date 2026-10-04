//! The close prompt's discard button, which used to be one click from deleting
//! a workspace's stashed edits.

use super::*;

/// No single click may delete a stash. The button that did was labelled "Don't
/// Save" on a quit dialog, and because Export Mod leaves its tags dirty, that is
/// the dialog an exporter sees on every exit -- so "no thanks, I already
/// exported" deleted the very edits the mod was made from.
#[test]
fn no_single_click_deletes_a_stash() {
    let fresh = discard_button(true, false);
    assert!(fresh.arming, "the first click on a stashing workspace arms");
    assert_ne!(
        fresh.label, "Don't Save",
        "it is named for what it does, not for what it declines"
    );
    let armed = discard_button(true, true);
    assert!(!armed.arming, "the second click acts");
    assert!(
        armed.label.contains("Stashed"),
        "and says what it is deleting: {}",
        armed.label
    );
}

/// A workspace with nowhere to stash to loses nothing beyond the open document,
/// so it keeps the one-click discard every editor has.
#[test]
fn a_workspace_without_a_stash_keeps_one_click_discard() {
    let button = discard_button(false, false);
    assert_eq!(button.label, "Don't Save");
    assert!(!button.arming);
    // Even a stale confirmation flag cannot turn this into two clicks.
    assert_eq!(discard_button(false, true).label, "Don't Save");
}
