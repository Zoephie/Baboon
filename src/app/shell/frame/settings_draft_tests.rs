//! Settings edits a draft of the preferences and sends it once drawn. A tick
//! has to reach the live preferences that way, and only the setting ticked
//! may change.

use super::perf_baseline_tests::Harness;
use super::*;

#[test]
fn a_settings_checkbox_changes_the_live_preference() {
    let mut h = Harness::new();
    h.app.open_settings(Some(SettingsTab::Browser));
    for _ in 0..4 {
        h.frame(Vec::new());
    }
    let before = h.app.model.prefs.clone();
    h.click("Double-click to open tags", 0);
    for _ in 0..2 {
        h.frame(Vec::new());
    }
    let after = &h.app.model.prefs;
    assert_eq!(after.double_click_to_open_tags, !before.double_click_to_open_tags);
    let mut expected = before.clone();
    expected.double_click_to_open_tags = after.double_click_to_open_tags;
    assert!(*after == expected, "nothing else changed");
}

/// A draft the commit refuses comes back as its dialog, with the reason.
#[test]
fn a_refused_editing_kit_draft_reopens_with_its_reason() {
    let mut app = Baboon::for_test();
    app.commands.send(SettingsCommand::CommitEditingKitDraft(
        CustomEditingKitDraft::new(),
    ));
    app.apply_commands(&egui::Context::default());
    let draft = app
        .dialogs
        .get::<CustomEditingKitDraft>()
        .expect("reopened");
    assert_eq!(draft.error.as_deref(), Some("Enter an editing kit name"));
}
