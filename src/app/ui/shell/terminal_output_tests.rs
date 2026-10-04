use super::*;

thread_local! {
    /// Output lines laid out. egui skips painting offscreen labels by
    /// itself, so the painted text alone cannot show that the pane lays
    /// out only what is in view.
    pub(in crate::app) static LINES_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn lines(count: usize) -> Vec<TerminalLineEntry> {
    (0..count)
        .map(|index| {
            TerminalLineEntry::new(format!(
                "{index}: tool.exe: importing C:\\Halo\\tags\\objects\\weapons\\rifle_{index}\\\
                     render\\rifle_{index}.render_model from data\\objects\\weapons ... done"
            ))
        })
        .collect()
}

fn frame(
    ctx: &egui::Context,
    lines: &[TerminalLineEntry],
    bottom: bool,
) -> std::time::Duration {
    let started = std::time::Instant::now();
    let _ = crate::app::run_ui_test(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 300.0),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                draw_terminal_output(ui, lines, bottom);
            });
        },
    );
    started.elapsed()
}

/// The text of every line painted in a frame.
fn painted(ctx: &egui::Context, lines: &[TerminalLineEntry], bottom: bool) -> Vec<String> {
    LINES_BUILT.with(|built| built.set(0));
    let output = crate::app::run_ui_test(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 300.0),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                draw_terminal_output(ui, lines, bottom);
            });
        },
    );
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

/// Only the lines in view are drawn, and they are the right ones: the
/// top of the output when opened, the end of it after scrolling there.
#[test]
fn the_terminal_draws_the_lines_in_view() {
    let lines = lines(20_000);
    let starts =
        |painted: &[String], prefix: &str| painted.iter().any(|text| text.starts_with(prefix));

    let top = painted(&egui::Context::default(), &lines, false);
    assert!(starts(&top, "0: ") && !starts(&top, "19999: "));
    let built = LINES_BUILT.with(std::cell::Cell::get);
    assert!(built < 100, "laid out {built} of 20,000 lines");

    let ctx = egui::Context::default();
    // Scrolling animates over frames; land in one.
    ctx.global_style_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
    painted(&ctx, &lines, true);
    let bottom = (0..3).map(|_| painted(&ctx, &lines, false)).last().unwrap();
    assert!(starts(&bottom, "19999: "), "the last line is in view");
    assert!(!starts(&bottom, "0: "));
    let built = LINES_BUILT.with(std::cell::Cell::get);
    assert!(built < 100, "laid out {built} of 20,000 lines");
}

/// Frame time with a full terminal. Run with `--release --ignored
/// --nocapture`.
#[test]
#[ignore]
fn bench_terminal_frame() {
    let lines = lines(20_000);
    let ctx = egui::Context::default();
    for index in 0..6 {
        eprintln!("frame {index}: {:?}", frame(&ctx, &lines, index == 0));
    }
}
