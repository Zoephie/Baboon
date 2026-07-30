//! The welcome screen's UI scale slider, driven with real pointer input.
//!
//! The slider sets the zoom factor of the window it is drawn in. Applying the
//! value while the drag was live rescaled the slider under the pointer, so the
//! handle slid away from the cursor and the scale could not be aimed at all.

use super::*;

/// One frame with `events` delivered to it, standing in for the wizard: the
/// slider edits `pending`, and the rule decides when that reaches `live`.
fn frame(
    ctx: &egui::Context,
    pending: &mut f32,
    live: &mut f32,
    events: Vec<egui::Event>,
    pointer: Option<egui::Pos2>,
) -> egui::Rect {
    let mut rect = egui::Rect::NOTHING;
    let mut events = events;
    if let Some(pos) = pointer {
        events.insert(0, egui::Event::PointerMoved(pos));
    }
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(600.0, 200.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.add(
                    egui::Slider::new(pending, MIN_UI_SCALE..=MAX_UI_SCALE).show_value(false),
                );
                rect = response.rect;
                if crate::app::ui::first_run::commit_ui_scale_now(&response, *pending, *live) {
                    *live = *pending;
                }
            });
        },
    );
    rect
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// The reported interaction: grab the handle, drag it, let go. The window must
/// not be rescaled until the release, however many frames the drag spans.
#[test]
fn the_scale_is_applied_when_the_drag_ends_not_while_it_lasts() {
    let ctx = egui::Context::default();
    let (mut pending, mut live) = (DEFAULT_UI_SCALE, DEFAULT_UI_SCALE);

    // Lay out once to learn where the slider is.
    let rect = frame(&ctx, &mut pending, &mut live, Vec::new(), None);
    assert!(rect.width() > 20.0, "the slider was laid out");

    let start = egui::pos2(rect.center().x, rect.center().y);
    frame(
        &ctx,
        &mut pending,
        &mut live,
        vec![button(start, true)],
        Some(start),
    );

    // Drag in steps, as a pointer does. Every frame that moves the value is a
    // frame that must not rescale the window.
    let mut moved = false;
    for step in 1..=4 {
        let at = egui::pos2(start.x - step as f32 * 12.0, start.y);
        let before = pending;
        frame(&ctx, &mut pending, &mut live, Vec::new(), Some(at));
        moved |= pending != before;
        assert_eq!(
            live, DEFAULT_UI_SCALE,
            "frame {step} of the drag rescaled the window mid-drag (slider {before} -> {pending})"
        );
    }
    assert!(
        moved,
        "the drag actually moved the slider — otherwise this proves nothing"
    );

    let end = egui::pos2(start.x - 48.0, start.y);
    frame(
        &ctx,
        &mut pending,
        &mut live,
        vec![button(end, false)],
        Some(end),
    );
    assert_eq!(
        live, pending,
        "releasing applies exactly the scale the user aimed at"
    );
    assert_ne!(live, DEFAULT_UI_SCALE, "and that is not where it started");
}

/// A click that never crosses the drag threshold still has to arrive. Waiting for
/// a drag-stopped event that never comes would leave the slider showing a scale
/// the window never took.
/// Press and release are separate frames: a click lasts tens of milliseconds and
/// the app redraws throughout. (egui moves a slider on the frames *after* the
/// press, so a press and release collapsed into one frame moves nothing at all —
/// that is the synthetic input being unrealistic, not the widget.)
#[test]
fn a_click_on_the_track_is_not_swallowed() {
    let ctx = egui::Context::default();
    let (mut pending, mut live) = (DEFAULT_UI_SCALE, DEFAULT_UI_SCALE);
    let rect = frame(&ctx, &mut pending, &mut live, Vec::new(), None);
    let at = egui::pos2(rect.left() + rect.width() * 0.85, rect.center().y);

    frame(&ctx, &mut pending, &mut live, vec![button(at, true)], Some(at));
    frame(&ctx, &mut pending, &mut live, vec![button(at, false)], Some(at));

    assert_ne!(pending, DEFAULT_UI_SCALE, "the click moved the slider");
    assert_eq!(live, pending, "and the window followed it");
}
