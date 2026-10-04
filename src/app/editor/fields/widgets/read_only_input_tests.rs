use super::*;

#[test]
fn read_only_inputs_allow_selection_and_copy_but_reject_mutations() {
    let ctx = egui::Context::default();
    ctx.set_global_style(foundation_style());
    let value = "A long editing kit value that must be copied in full";
    let mut id = egui::Id::NULL;
    let mut frame = |events| {
        crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(120.0, 24.0), Sense::hover());
                    let response =
                        foundation_read_only_text_cell(ui, rect, value, text_dark(), 5.0);
                    id = response.id;
                    response.request_focus();
                    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(value.chars().count()),
                        )));
                    state.store(&ctx, id);
                });
            },
        )
    };
    let _ = frame(vec![]);
    let copy = frame(vec![egui::Event::Copy]);
    assert_eq!(crate::app::copied_text(&copy.platform_output), value);
    let _ = frame(vec![
        egui::Event::Text("modified".to_owned()),
        egui::Event::Paste("pasted".to_owned()),
        egui::Event::Cut,
        egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    let copy = frame(vec![egui::Event::Copy]);
    assert_eq!(crate::app::copied_text(&copy.platform_output), value);
}
