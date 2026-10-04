use super::*;

const VIEWPORT: egui::Rect = egui::Rect {
    min: egui::Pos2::new(0.0, 150.0),
    max: egui::Pos2::new(400.0, 350.0),
};
const OVER_PANEL: egui::Pos2 = egui::Pos2::new(200.0, 50.0);
const OVER_VIEWPORT: egui::Pos2 = egui::Pos2::new(200.0, 250.0);

/// One frame of a scrolling pane with a viewport in it: optionally a
/// wheel event at `pointer`. Returns the viewport's zoom travel and the
/// pane's scroll offset after the frame.
fn frame(ctx: &egui::Context, wheel: bool, pointer: egui::Pos2) -> (Option<f32>, f32) {
    let mut events = vec![egui::Event::PointerMoved(pointer)];
    if wheel {
        // Trackpad-sized, so egui applies it this frame rather than
        // smoothing it over the next ones.
        events.push(egui::Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            unit: egui::MouseWheelUnit::Point,
            delta: egui::Vec2::new(0.0, -4.0),
            modifiers: Default::default(),
        });
    }
    let mut zoom = None;
    let mut offset = 0.0;
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
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let output = egui::ScrollArea::vertical().animated(false).show(ui, |ui| {
                        ui.allocate_exact_size(
                            egui::Vec2::new(400.0, 150.0),
                            egui::Sense::hover(),
                        );
                        let (_, response) = ui.allocate_exact_size(
                            VIEWPORT.size(),
                            egui::Sense::click_and_drag(),
                        );
                        zoom = viewport_wheel_zoom(ui, &response);
                        ui.allocate_exact_size(
                            egui::Vec2::new(400.0, 4000.0),
                            egui::Sense::hover(),
                        );
                    });
                    offset = output.state.offset.y;
                });
            end_wheel_gesture(ctx);
        },
    );
    (zoom, offset)
}

/// A pane that has been on screen a frame, as any the user scrolls has:
/// egui hit-tests against the previous frame's widgets, so nothing is
/// hovered on the first.
fn ctx(pointer: egui::Pos2) -> egui::Context {
    let ctx = egui::Context::default();
    frame(&ctx, false, pointer);
    ctx
}

/// The reported defect: scrolling the pane, the cursor passes over the
/// viewport, which took the wheel and zoomed instead of letting the pane
/// keep scrolling.
#[test]
fn a_viewport_scrolled_past_mid_gesture_does_not_zoom() {
    let ctx = ctx(OVER_PANEL);
    let (zoom, mut offset) = frame(&ctx, true, OVER_PANEL);
    assert_eq!(zoom, None);
    assert!(offset > 0.0, "the pane scrolled");
    for _ in 0..5 {
        let (zoom, next) = frame(&ctx, true, OVER_VIEWPORT);
        assert_eq!(zoom, None, "the viewport stole a panel scroll");
        assert!(
            next > offset,
            "the pane stopped scrolling under the viewport"
        );
        offset = next;
    }
}

/// Deliberate use still zooms, and the pane under the viewport holds
/// still while it does.
#[test]
fn a_viewport_pointed_at_first_zooms_and_the_pane_holds() {
    let ctx = ctx(OVER_VIEWPORT);
    for _ in 0..3 {
        let (zoom, offset) = frame(&ctx, true, OVER_VIEWPORT);
        assert_eq!(zoom, Some(-4.0));
        assert_eq!(offset, 0.0, "the pane scrolled under a zoom");
    }
}

#[test]
fn zoom_speed_scales_the_zoom() {
    let ctx = ctx(OVER_VIEWPORT);
    set_zoom_speed(&ctx, 2.5);
    assert_eq!(frame(&ctx, true, OVER_VIEWPORT).0, Some(-10.0));
}
