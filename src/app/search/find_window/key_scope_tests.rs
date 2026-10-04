use super::*;

fn occurrence(field_path: &str) -> FindOccurrence {
    FindOccurrence {
        tag_key: "file:missing".to_owned(),
        field_path: field_path.to_owned(),
        kind: FindTargetKind::Value,
        text: "match".to_owned(),
        range: 0..5,
    }
}

/// One frame: a text box standing in for a field editor, and Find.
fn frame(
    app: &mut Baboon,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    focus: Option<egui::Id>,
) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::Vec2::new(1200.0, 800.0),
        )),
        events,
        ..Default::default()
    };
    let _ = crate::app::run_ui_test(&ctx, input, |ui| {
        if let Some(id) = focus {
            ctx.memory_mut(|memory| memory.request_focus(id));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            let mut text = String::from("12");
            ui.add(egui::TextEdit::singleline(&mut text).id(egui::Id::new("a_field")));
        });
        app.draw_find_window(ctx);
    });
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }
}

/// Enter and Escape are Find's only while its query box has focus.
/// Committing a field with Enter used to step to the next match, and
/// Escape out of anything closed Find.
#[test]
fn find_answers_enter_and_escape_only_in_its_query_box() {
    let ctx = egui::Context::default();
    let mut app = Baboon::for_test();
    app.search.find.open = true;
    app.search.find.occurrences = vec![occurrence("a"), occurrence("b")];
    app.search.find.active = Some(0);
    let field = Some(egui::Id::new("a_field"));
    let query = Some(egui::Id::new("find_query"));

    // Lay both out once, then put focus in the field.
    frame(&mut app, &ctx, Vec::new(), None);
    frame(&mut app, &ctx, Vec::new(), field);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)], None);
    assert_eq!(
        app.search.find.active,
        Some(0),
        "Enter in a field must not step Find"
    );
    frame(&mut app, &ctx, Vec::new(), field);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)], None);
    assert!(app.search.find.open, "Escape in a field must not close Find");

    // The same keys in the query box.
    frame(&mut app, &ctx, Vec::new(), query);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)], None);
    assert_eq!(app.search.find.active, Some(1), "Enter in the query box steps");
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)], None);
    assert_eq!(app.search.find.active, Some(0), "and keeps stepping");
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)], None);
    assert!(!app.search.find.open, "Escape in the query box closes Find");
}
