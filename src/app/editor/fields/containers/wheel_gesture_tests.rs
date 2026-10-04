use super::*;

/// One frame: optionally a wheel event, and a dropdown at `rect` asking
/// whether it may cycle. Returns whether it was allowed to.
fn frame(ctx: &egui::Context, wheel: bool, pointer: egui::Pos2, rect: egui::Rect) -> bool {
    let mut events = vec![egui::Event::PointerMoved(pointer)];
    if wheel {
        events.push(egui::Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            unit: egui::MouseWheelUnit::Line,
            delta: egui::Vec2::new(0.0, -1.0),
            modifiers: Default::default(),
        });
    }
    let mut claimed = false;
    let _ = crate::app::run_ui_test(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(400.0, 400.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            begin_wheel_gesture(ctx);
            egui::CentralPanel::default().show(ui, |ui| {
                let response = ui.allocate_rect(rect, egui::Sense::hover());
                claimed = dropdown_wheel_delta(ui, &response, false).is_some();
            });
            end_wheel_gesture(ctx);
        },
    );
    claimed
}

fn ctx() -> egui::Context {
    let ctx = egui::Context::default();
    set_combo_scroll_cycle_enabled(&ctx, true);
    ctx
}

const BOX_RECT: egui::Rect = egui::Rect {
    min: egui::Pos2::new(10.0, 100.0),
    max: egui::Pos2::new(200.0, 120.0),
};
const OVER_BOX: egui::Pos2 = egui::Pos2::new(100.0, 110.0);
const ABOVE_BOX: egui::Pos2 = egui::Pos2::new(100.0, 20.0);

/// The reported defect: scrolling a tag pane, the cursor passes over a
/// dropdown, and the dropdown silently changes value. The gesture started as
/// panel scrolling and must stay panel scrolling.
#[test]
fn a_dropdown_scrolled_past_mid_gesture_does_not_change() {
    let ctx = ctx();
    // The gesture begins away from any dropdown.
    assert!(!frame(&ctx, true, ABOVE_BOX, BOX_RECT));
    // Now the cursor slides over one while the wheel is still turning.
    for _ in 0..5 {
        assert!(
            !frame(&ctx, true, OVER_BOX, BOX_RECT),
            "a dropdown claimed a wheel gesture that began as panel scrolling"
        );
    }
}

/// Deliberate use still works: point at a dropdown, then scroll.
#[test]
fn a_dropdown_pointed_at_first_still_cycles() {
    let ctx = ctx();
    assert!(
        frame(&ctx, true, OVER_BOX, BOX_RECT),
        "a gesture starting on a dropdown should be the dropdown's"
    );
    // ...and keeps it for the rest of the gesture.
    assert!(frame(&ctx, true, OVER_BOX, BOX_RECT));
}

/// After the wheel goes quiet the next turn is a fresh gesture, so stopping
/// and pointing at a dropdown works without moving the mouse away first.
#[test]
fn a_pause_starts_a_new_gesture() {
    let ctx = ctx();
    assert!(!frame(&ctx, true, ABOVE_BOX, BOX_RECT));
    assert!(!frame(&ctx, true, OVER_BOX, BOX_RECT));
    // Frames with no wheel event: the gesture goes stale.
    for _ in 0..40 {
        frame(&ctx, false, OVER_BOX, BOX_RECT);
    }
    assert!(
        frame(&ctx, true, OVER_BOX, BOX_RECT),
        "a new gesture over a dropdown should be claimable"
    );
}

/// The preference still wins outright.
#[test]
fn the_preference_disables_it_entirely() {
    let ctx = egui::Context::default();
    set_combo_scroll_cycle_enabled(&ctx, false);
    assert!(!frame(&ctx, true, OVER_BOX, BOX_RECT));
}
