use super::*;

/// What one frame with `key` pressed under `modifiers` asks for.
fn pressed(modifiers: egui::Modifiers, key: egui::Key) -> Vec<AppAction> {
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        events: vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }],
        ..Default::default()
    };
    let mut actions = Vec::new();
    let _ = crate::app::run_ui_test(&ctx, input, |ui| actions = pressed_shortcuts(ui.ctx()));
    actions
}

/// Ctrl+Shift+Z redoes. egui matches a Ctrl+Z pattern against it too, and
/// when undo was checked first that is what it did.
#[test]
fn ctrl_shift_z_redoes_and_ctrl_z_undoes() {
    let ctrl = egui::Modifiers::CTRL;
    let redo = pressed(ctrl.plus(egui::Modifiers::SHIFT), egui::Key::Z);
    assert!(matches!(redo.as_slice(), [AppAction::Redo]));
    assert!(matches!(
        pressed(ctrl, egui::Key::Z).as_slice(),
        [AppAction::Undo]
    ));
    assert!(matches!(
        pressed(ctrl, egui::Key::Y).as_slice(),
        [AppAction::Redo]
    ));
}

/// A key the table does not list asks for nothing.
#[test]
fn an_unlisted_key_asks_for_nothing() {
    assert!(pressed(egui::Modifiers::CTRL, egui::Key::Q).is_empty());
}
