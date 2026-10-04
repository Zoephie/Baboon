//! While a folder move or rename rewrites tags on disk, nothing under the lock
//! takes a click and no shortcut runs. Each check has an unlocked control that
//! must disagree, so a lock that blocked nothing would fail here.

use super::*;

fn app(locked: bool) -> Baboon {
    let mut app = Baboon::assemble(
        &egui::Context::default(),
        crate::window_state::WindowStateTracker::for_test(),
        GuiPrefs::default(),
        HashSet::new(),
        None,
        TagNameIndex::default(),
        None,
    );
    if locked {
        app.tag_ops.folder_refactor = Some(FolderRefactorUiState {
            label: "Renaming creep to shadow".to_owned(),
            phase: "Moving files".to_owned(),
            progress: Some(0.5),
        });
    }
    app
}

fn input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        events,
        ..Default::default()
    }
}

/// Click a button drawn beneath the lock layer; whether it saw the click.
fn button_sees_click(locked: bool) -> bool {
    let mut app = app(locked);
    let ctx = egui::Context::default();
    let rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let clicked = std::cell::Cell::new(false);
    let frame = |events: Vec<egui::Event>, app: &mut Baboon| {
        let _ = crate::app::run_ui_test(&ctx, input(events), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let response = ui.button("Save");
                rect.set(response.rect);
                clicked.set(clicked.get() | response.clicked());
            });
            draw_folder_refactor_lock(&ctx, app.tag_ops.folder_refactor.as_ref());
        });
    };
    frame(Vec::new(), &mut app);
    let pos = rect.get().center();
    let press = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(vec![egui::Event::PointerMoved(pos)], &mut app);
    frame(vec![press(true)], &mut app);
    frame(vec![press(false)], &mut app);
    frame(Vec::new(), &mut app);
    clicked.get()
}

#[test]
fn the_lock_swallows_clicks_meant_for_the_app_beneath() {
    assert!(
        button_sees_click(false),
        "control: the click lands unlocked"
    );
    assert!(!button_sees_click(true), "the lock must take the click");
}

/// Press Ctrl+S; whether a save was queued.
fn ctrl_s_queues_save(locked: bool) -> bool {
    let mut app = app(locked);
    let ctx = egui::Context::default();
    let _ = crate::app::run_ui_test(
        &ctx,
        input(vec![egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        }]),
        |_| {
            app.prepare_root_frame(&ctx);
            // A shortcut's action is applied with the frame's commands.
            app.apply_commands(&ctx);
        },
    );
    app.editor.deferred_file_action.is_some()
}

#[test]
fn shortcuts_do_not_run_while_locked() {
    assert!(
        ctrl_s_queues_save(false),
        "control: Ctrl+S queues a save unlocked"
    );
    assert!(
        !ctrl_s_queues_save(true),
        "Ctrl+S must not run under the lock"
    );
}
