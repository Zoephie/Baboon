use super::*;

#[test]
fn tutorial_tab_renders_at_minimum_size_in_both_themes() {
    let ctx = egui::Context::default();
    let tutorials = TutorialsState::load(&ctx);

    for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
        ctx.set_visuals(visuals);
        for category in TUTORIAL_CATEGORIES {
            let mut selected_game = "haloce_evolved".to_owned();
            let mut selected_category = category;
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(520.0, 360.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        draw_tutorials_tab(
                            ui,
                            &tutorials,
                            &mut selected_game,
                            &mut selected_category,
                        );
                    });
                },
            );
            assert!(!output.shapes.is_empty());
        }
    }
}

#[test]
fn tutorial_url_requests_a_new_browser_tab() {
    let ctx = egui::Context::default();
    let url = "https://www.youtube.com/watch?v=2xL2AiuaFwE";
    let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |_| {
        open_tutorial_url(&ctx, url);
    });
    let request = output
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::OpenUrl(open) => Some(open.clone()),
            _ => None,
        })
        .expect("tutorial action should request an external URL");
    assert_eq!(request.url, url);
    assert!(request.new_tab);
}
