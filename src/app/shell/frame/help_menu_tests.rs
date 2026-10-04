//! The Help menu opens Help on the tab it names. The menu sends a command and
//! the window draws from Help's own state, so this crosses the whole path: a
//! click, the queue, the frame applying it, and the next frame's window.

use super::perf_baseline_tests::Harness;
use super::*;

fn idle(h: &mut Harness) {
    for _ in 0..4 {
        h.frame(Vec::new());
    }
}

#[test]
fn a_help_menu_item_opens_help_on_its_tab() {
    let mut h = Harness::new();
    idle(&mut h);
    assert!(h.app.dialogs.get::<HelpWindow>().is_none());
    assert!(!h.painted.iter().any(|text| text == "Baboon Help"));

    h.click("Help", 0);
    idle(&mut h);
    h.click("Map Names...", 0);
    idle(&mut h);
    let help = h.app.dialogs.get::<HelpWindow>().expect("Help opened");
    assert!(help.tab == HelpPanelTab::MapNames);
    assert!(h.painted.iter().any(|text| text == "Baboon Help"), "the window draws");
}
