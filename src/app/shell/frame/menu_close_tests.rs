//! Menus close when an item closes them, or on a click outside — not on every
//! click inside, which is egui 0.36's default and would shut the View menu
//! each time one of its checkboxes was ticked.

use super::perf_baseline_tests::Harness;
use super::*;

fn idle(h: &mut Harness) {
    for _ in 0..4 {
        h.frame(Vec::new());
    }
}

/// Whether the View menu is showing, told by one of its items.
fn view_menu_open(h: &Harness) -> bool {
    h.painted.iter().any(|text| text == "Expert mode")
}

#[test]
fn view_menu_toggles_keep_it_open_and_an_action_closes_it() {
    let mut h = Harness::new();
    idle(&mut h);
    assert!(!view_menu_open(&h));
    h.click("View", 0);
    idle(&mut h);
    assert!(view_menu_open(&h), "the View menu opened");

    let block_sizes = h.app.prefs.show_block_sizes;
    h.click("Show block sizes", 0);
    idle(&mut h);
    assert_eq!(h.app.prefs.show_block_sizes, !block_sizes, "the checkbox toggled");
    assert!(view_menu_open(&h), "a toggle leaves the menu open");

    let expert = h.app.prefs.expert_mode;
    h.click("Expert mode", 0);
    idle(&mut h);
    assert_eq!(h.app.prefs.expert_mode, !expert);
    assert!(view_menu_open(&h), "so does a second one");

    h.click("Tag Groups", 0);
    idle(&mut h);
    assert_eq!(h.app.kits[h.app.active].browser_mode, BrowserMode::Groups);
    assert!(!view_menu_open(&h), "an action closes it");
}

#[test]
fn a_click_outside_a_menu_closes_it() {
    let mut h = Harness::new();
    idle(&mut h);
    h.click("View", 0);
    idle(&mut h);
    assert!(view_menu_open(&h));
    h.click("Recent", 0);
    idle(&mut h);
    assert!(!view_menu_open(&h));
}

/// Outside a menu `close_menu` does nothing, as egui 0.29's `Ui::close_menu`
/// did. egui 0.36's `Ui::close`, which the menus' former `close_menu` calls
/// would otherwise map to, collapses the header around it instead.
#[test]
fn close_menu_outside_a_menu_leaves_its_container_alone() {
    let openness_after = |close: fn(&Ui)| {
        let ctx = egui::Context::default();
        let mut openness = 0.0;
        for frame in 0..4 {
            let input = egui::RawInput {
                time: Some(f64::from(frame)),
                ..Default::default()
            };
            let _ = crate::app::run_ui_test(&ctx, input, |ui| {
                openness = egui::CollapsingHeader::new("Header")
                    .default_open(true)
                    .show(ui, |ui| {
                        if frame == 1 {
                            close(ui);
                        }
                    })
                    .openness;
            });
        }
        openness
    };
    assert_eq!(openness_after(close_menu), 1.0);
    assert_eq!(openness_after(|ui| ui.close()), 0.0, "egui's close collapses it");
}
