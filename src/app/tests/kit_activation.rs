//! Which workspace is active decides where Ctrl+S, the save prompt and open
//! pickers land, so it changes on a press inside a workspace and never because
//! the cursor passed over one. Two workspaces side by side, driven headless.

use super::*;

fn input(time: f64, events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        time: Some(time),
        events,
        ..Default::default()
    }
}

#[test]
fn a_workspace_activates_on_a_press_not_on_hover() {
    let mut app = Baboon::for_test();
    app.add_kit();
    assert_eq!(app.kits.len(), 2);
    let (left, right) = (app.kits[0].id, app.kits[1].id);
    app.kit_tree = egui_tiles::Tree::new_horizontal("kit_activation_test", vec![left, right]);
    app.active = 1;

    let ctx = egui::Context::default();
    let mut time = 0.0;
    let mut frame = |app: &mut Baboon, events: Vec<egui::Event>| {
        time += 0.1;
        let _ = ctx.run(input(time, events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw_kit_tiles(ui, ctx));
        });
    };
    let over_left = egui::pos2(200.0, 300.0);
    let over_right = egui::pos2(600.0, 300.0);
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };

    frame(&mut app, Vec::new());
    for _ in 0..3 {
        frame(&mut app, vec![egui::Event::PointerMoved(over_left)]);
    }
    assert_eq!(app.active, 1, "hovering the left workspace changes nothing");

    frame(&mut app, vec![press(over_left, true)]);
    frame(&mut app, vec![press(over_left, false)]);
    assert_eq!(app.active, 0, "a press inside the left workspace activates it");

    for _ in 0..3 {
        frame(&mut app, vec![egui::Event::PointerMoved(over_right)]);
    }
    assert_eq!(app.active, 0, "hovering the right workspace changes nothing");

    frame(&mut app, vec![press(over_right, true)]);
    frame(&mut app, vec![press(over_right, false)]);
    assert_eq!(app.active, 1);
}
