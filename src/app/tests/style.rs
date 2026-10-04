//! Unit tests for shared visual-style helpers.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;

#[test]
fn material_text_for_bg_chooses_contrasting_foreground() {
    assert_eq!(
        material_text_for_bg(Color32::from_rgb(42, 43, 41)),
        Color32::from_gray(232)
    );
    assert_eq!(
        material_text_for_bg(Color32::from_rgb(232, 191, 171)),
        Color32::from_gray(20)
    );
}

#[test]
fn dark_active_tab_is_lighter_than_every_tab_rack_background() {
    let active = active_tab_for(true);
    assert_eq!(active, Color32::from_rgb(72, 72, 72));
    assert!(active.r() > 55, "document/Chimp inactive gray");
    assert!(active.r() > 31, "workspace inactive gray");
}

#[test]
fn light_active_tab_keeps_the_existing_menu_bar_color() {
    assert_eq!(active_tab_for(false), Color32::from_rgb(161, 161, 157));
}

#[test]
fn filtered_block_jump_uses_the_cyan_navigation_accent() {
    assert_eq!(foundation_jump_cyan(), Color32::from_rgb(77, 208, 225));
}

/// A window sized through [`window_size`] gets the content area it asks for,
/// with a title bar and without, resizable or not, at two heading sizes. The
/// sizes in Baboon were picked for the content, which is what egui 0.29
/// sized; egui 0.36 sizes the whole window.
#[test]
fn a_window_sized_for_its_content_gets_that_content_area() {
    let content = Vec2::new(400.0, 300.0);
    for heading in [17.0, 24.0] {
        let ctx = egui::Context::default();
        crate::app::Baboon::configure_context(&ctx);
        ctx.global_style_mut(|style| {
            style
                .text_styles
                .insert(TextStyle::Heading, FontId::proportional(heading));
        });
        let mut seen = Vec::new();
        for frame in 0..3 {
            seen.clear();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 1000.0),
                )),
                time: Some(f64::from(frame)),
                ..Default::default()
            };
            let _ = crate::app::run_ui_test(&ctx, input, |ui| {
                let ctx = ui.ctx().clone();
                for (name, title_bar, resizable) in
                    [("a", true, true), ("b", true, false), ("c", false, true)]
                {
                    let shown = egui::Window::new(name)
                        .title_bar(title_bar)
                        .resizable(resizable)
                        .default_size(window_size(&ctx, content, title_bar))
                        .default_pos(egui::pos2(100.0, 100.0))
                        .show(&ctx, |ui| {
                            let available = ui.available_size();
                            ui.allocate_space(available);
                            available
                        });
                    seen.push((name, shown.and_then(|shown| shown.inner)));
                }
            });
        }
        for (name, available) in &seen {
            let available = available.expect("the window drew");
            assert!(
                (available - content).abs().max_elem() < 1.0,
                "heading {heading}, window {name}: content {available:?}, asked for {content:?}"
            );
        }
    }
}

/// A window with no position of its own opens inside the work area the root
/// frame records — below the menu bar — as egui 0.29 placed it.
#[test]
fn a_window_without_a_position_opens_inside_the_work_area() {
    let ctx = egui::Context::default();
    crate::app::Baboon::configure_context(&ctx);
    let work_area = egui::Rect::from_min_max(egui::pos2(0.0, 29.0), egui::pos2(1600.0, 970.0));
    let mut rect = egui::Rect::NOTHING;
    for frame in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1000.0),
            )),
            time: Some(f64::from(frame)),
            ..Default::default()
        };
        let _ = crate::app::run_ui_test(&ctx, input, |ui| {
            set_window_work_area(ui.ctx(), work_area);
            let ctx = ui.ctx().clone();
            rect = egui::Window::new("Unplaced")
                .constrain_to(window_work_area(&ctx))
                .show(&ctx, |ui| ui.label("text"))
                .expect("the window drew")
                .response
                .rect;
        });
    }
    assert!(work_area.contains_rect(rect), "{rect:?} is outside {work_area:?}");
    assert!(rect.top() > work_area.top(), "{rect:?}");
}
