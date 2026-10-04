use super::*;

/// Two panes showing the same tag keep their own keyword drafts. The draft
/// used to be one field on the app, shared by every pane in every kit.
#[test]
fn each_pane_keeps_its_own_keyword_draft() {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::app::foundation_fonts());
    let mut app = Baboon::for_test();
    let mut draft_ids = Vec::new();
    let frame = |app: &mut Baboon, draft_ids: &mut Vec<egui::Id>| {
        let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                draft_ids.clear();
                for pane in ["pane a", "pane b"] {
                    ui.push_id(pane, |ui| {
                        let egui = ui.ctx().clone();
                        draw_keyword_bar(&cx!(app, &egui), ui, 0, "file:crate.model");
                        draft_ids
                            .push(ui.make_persistent_id(("keyword_input", "file:crate.model")));
                    });
                }
            });
        });
    };

    frame(&mut app, &mut draft_ids);
    ctx.data_mut(|data| data.insert_temp(draft_ids[0], "rocket".to_owned()));
    frame(&mut app, &mut draft_ids);

    let draft = |id: egui::Id| {
        ctx.data_mut(|data| data.get_temp::<String>(id))
            .unwrap_or_default()
    };
    assert_eq!(draft(draft_ids[0]), "rocket");
    assert_eq!(draft(draft_ids[1]), "", "the other pane's box is untouched");
}
