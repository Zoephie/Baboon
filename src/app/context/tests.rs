use super::*;

/// A draw's request changes nothing until the frame applies it — which is
/// what lets every draw in a frame see the same model.
#[test]
fn a_sent_command_changes_nothing_until_applied() {
    let mut app = Baboon::for_test();
    let egui = egui::Context::default();
    let before = app.model.status.clone();
    cx!(app, &egui).set_status("from a draw");
    assert_eq!(app.model.status, before);
    app.apply_commands(&egui);
    assert_eq!(app.model.status, "from a draw");
}

#[test]
fn commands_apply_in_the_order_sent() {
    let mut app = Baboon::for_test();
    let egui = egui::Context::default();
    app.commands.send(HelpCommand::Open(HelpPanelTab::Doc));
    app.commands.send(HelpCommand::Open(HelpPanelTab::MapNames));
    app.apply_commands(&egui);
    assert!(app.help.about_open);
    assert!(app.help.help_panel_tab == HelpPanelTab::MapNames);
}

/// The queue is drained, so a command runs once however many frames follow.
#[test]
fn an_applied_command_does_not_run_again() {
    let mut app = Baboon::for_test();
    let egui = egui::Context::default();
    app.commands.send(HelpCommand::Open(HelpPanelTab::Doc));
    app.apply_commands(&egui);
    app.help.about_open = false;
    app.apply_commands(&egui);
    assert!(!app.help.about_open);
}

/// Two draws changing different preferences in one frame both land: each
/// sends a change, not a whole copy of the preferences that would overwrite
/// the other's.
#[test]
fn prefs_edits_from_one_frame_all_land() {
    let mut app = Baboon::for_test();
    let egui = egui::Context::default();
    let expert = app.model.prefs.expert_mode;
    let sizes = app.model.prefs.show_block_sizes;
    let cx = cx!(app, &egui);
    cx.edit_prefs(move |prefs| prefs.expert_mode = !expert);
    cx.edit_prefs(move |prefs| prefs.show_block_sizes = !sizes);
    app.apply_commands(&egui);
    assert_eq!(app.model.prefs.expert_mode, !expert);
    assert_eq!(app.model.prefs.show_block_sizes, !sizes);
}
