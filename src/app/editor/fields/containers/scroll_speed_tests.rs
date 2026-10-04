use super::*;

/// One frame with one trackpad-sized scroll (small enough that egui
/// applies it unsmoothed), returning `(smooth, raw)` as panes see them.
fn scrolled(scroll_speed: f32) -> (f32, f32) {
    let ctx = egui::Context::default();
    let mut seen = (0.0, 0.0);
    let _ = crate::app::run_ui_test(
        &ctx,
        egui::RawInput {
            events: vec![egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::Vec2::new(0.0, -4.0),
                modifiers: Default::default(),
            }],
            ..Default::default()
        },
        |_| {
            apply_scroll_speed(&ctx, scroll_speed);
            let options = input_options(&ctx);
            seen = ctx.input(|input| {
                (input.smooth_scroll_delta.y, raw_wheel_delta(input, &options).y)
            });
        },
    );
    seen
}

#[test]
fn scroll_speed_scales_scrolling_but_not_viewport_zoom() {
    assert_eq!(scrolled(1.0), (-4.0, -4.0), "100% is egui's own speed");
    assert_eq!(scrolled(2.5), (-10.0, -4.0));
    assert_eq!(scrolled(0.5), (-2.0, -4.0));
}

/// A Windows wheel notch arrives in lines, which egui smooths over many
/// frames; the distance travelled over the whole notch scales too.
#[test]
fn scroll_speed_scales_a_whole_smoothed_wheel_notch() {
    let travelled = |scroll_speed: f32| {
        let ctx = egui::Context::default();
        let mut total = 0.0;
        for frame in 0..240 {
            let events = if frame == 0 {
                vec![egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::Vec2::new(0.0, -1.0),
                    modifiers: Default::default(),
                }]
            } else {
                Vec::new()
            };
            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |_| {
                    apply_scroll_speed(&ctx, scroll_speed);
                    total += ctx.input(|input| input.smooth_scroll_delta.y);
                },
            );
        }
        total
    };
    let base = travelled(1.0);
    assert!((base + 40.0).abs() < 0.01, "one line is 40 points: {base}");
    assert!((travelled(3.0) - base * 3.0).abs() < 0.01);
}
