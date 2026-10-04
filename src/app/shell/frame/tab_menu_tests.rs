//! A kit's tag tabs: pressing in a pane focuses its tag, a middle-click closes
//! a tab, and the tab menu's closes reach the right tabs. Each goes through a
//! command sent while the tiles draw, so these drive whole frames.

use super::perf_baseline_tests::{Harness, fixture};
use super::*;

const PATHS: [&str; 3] = [
    "folder_00/sub_00/tag_000.biped",
    "folder_00/sub_00/tag_001.biped",
    "folder_00/sub_00/tag_002.biped",
];

/// A kit with the three tags open in one tab group, `tag_002` showing.
fn three_tabs() -> (Harness, Vec<String>) {
    let mut h = Harness::new();
    fixture::install_kit(&mut h.app, fixture::synthetic_entries(1, 1, 3));
    let keys = PATHS
        .iter()
        .map(|path| fixture::open_document(&mut h.app, path, fixture::new_tag("biped")))
        .collect();
    idle(&mut h);
    (h, keys)
}

fn idle(h: &mut Harness) {
    for _ in 0..4 {
        h.frame(Vec::new());
    }
}

/// Slide onto the first painting of `text` and press and release `button`
/// on it, as [`Harness::click`] does with the primary button.
fn press(h: &mut Harness, text: &str, button: egui::PointerButton) {
    let target = h
        .painted_rects
        .iter()
        .find(|(painted, _)| painted == text)
        .map(|(_, rect)| rect.center())
        .unwrap_or_else(|| panic!("{text:?} is not painted: {:?}", h.painted));
    let from = target - egui::vec2(30.0, 30.0);
    for step in 1..=3 {
        h.frame(vec![egui::Event::PointerMoved(
            from + (target - from) * step as f32 / 3.0,
        )]);
    }
    for pressed in [true, false] {
        h.frame(vec![egui::Event::PointerButton {
            pos: target,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
    idle(h);
}

/// The kit's open tabs, sorted: they are read off the tile tree, which does
/// not keep them in any order.
fn open_tabs(h: &Harness) -> Vec<String> {
    let mut tabs = h.app.model.kits[h.app.model.active].open_tabs.clone();
    tabs.sort();
    tabs
}

/// A press inside a pane makes its tag the one the file actions act on.
#[test]
fn a_press_in_a_pane_focuses_its_tag() {
    let (mut h, keys) = three_tabs();
    let active = h.app.model.active;
    h.app.model.kits[active].selected_key = Some(keys[0].clone());
    press(&mut h, "runtime object type", egui::PointerButton::Primary);
    assert_eq!(
        h.app.model.kits[active].selected_key.as_ref(),
        Some(&keys[2])
    );
}

/// A middle-click on a tab closes it, and only it.
#[test]
fn a_middle_click_closes_a_tab() {
    let (mut h, keys) = three_tabs();
    assert_eq!(open_tabs(&h), keys);
    press(&mut h, "tag_000.biped", egui::PointerButton::Middle);
    assert_eq!(open_tabs(&h), keys[1..]);
}

/// The tab menu's "Close all but this" keeps the tab it was opened on.
#[test]
fn close_all_but_this_keeps_that_tab() {
    let (mut h, keys) = three_tabs();
    press(&mut h, "tag_001.biped", egui::PointerButton::Secondary);
    h.click("Close all but this", 0);
    idle(&mut h);
    assert_eq!(open_tabs(&h), [keys[1].clone()]);
}
