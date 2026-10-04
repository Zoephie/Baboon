use super::*;

/// Draw the open dialogs, as the frame does, without applying what they
/// send: the export would run.
fn draw_prompt(app: &mut Baboon) -> impl FnMut(&mut egui::Ui) + '_ {
    move |ui| {
        let ctx = ui.ctx().clone();
        app.dialogs.draw(&cx!(app, &ctx));
    }
}

fn app_with_prompt() -> Baboon {
    let mut app = Baboon::for_test();
    let kit = app.model.kits[0].id;
    app.dialogs
        .open(ChimpMeshTexturePrompt::for_test(kit, "/Game/Test/SM_Thing"));
    app
}

/// Choosing what to export asks for the export, leaving the prompt for it to
/// take; the prompt never starts it itself. Cancel closes it and asks for
/// nothing.
#[test]
fn a_choice_sends_the_export_and_cancel_sends_nothing() {
    let mut app = app_with_prompt();
    let mut frames = Frames::new();
    frames.click("Mesh only", &mut draw_prompt(&mut app));
    assert!(app.dialogs.get::<ChimpMeshTexturePrompt>().is_some());
    assert_eq!(app.commands.len(), 1);

    let mut app = app_with_prompt();
    let mut frames = Frames::new();
    frames.click("Cancel", &mut draw_prompt(&mut app));
    assert!(app.dialogs.get::<ChimpMeshTexturePrompt>().is_none());
    assert_eq!(app.commands.len(), 0);
}
