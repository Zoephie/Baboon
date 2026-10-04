use super::*;

/// Draw the mesh texture prompt, as the frame does.
fn draw_prompt(app: &mut Baboon) -> impl FnMut(&mut egui::Ui) + '_ {
    move |ui| {
        let ctx = ui.ctx().clone();
        draw_chimp_mesh_texture_prompt(&cx!(app, &ctx), &mut app.chimp);
    }
}

fn app_with_prompt() -> Baboon {
    let mut app = Baboon::for_test();
    let kit = app.model.kits[0].id;
    app.chimp.chimp_mesh_texture_prompt =
        Some(ChimpMeshTexturePrompt::for_test(kit, "/Game/Test/SM_Thing"));
    app
}

/// Choosing what to export closes the prompt and asks for the export; the
/// prompt never starts it itself.
#[test]
fn a_choice_sends_the_export_and_cancel_sends_nothing() {
    let mut app = app_with_prompt();
    let mut frames = Frames::new();
    frames.click("Mesh only", &mut draw_prompt(&mut app));
    assert!(app.chimp.chimp_mesh_texture_prompt.is_none());
    assert_eq!(app.commands.len(), 1);

    let mut app = app_with_prompt();
    let mut frames = Frames::new();
    frames.click("Cancel", &mut draw_prompt(&mut app));
    assert!(app.chimp.chimp_mesh_texture_prompt.is_none());
    assert_eq!(app.commands.len(), 0);
}
